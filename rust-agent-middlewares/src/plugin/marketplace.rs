use crate::plugin::config::{load_claude_settings, marketplaces_cache_dir};
use crate::plugin::types::{
    KnownMarketplace, MarketplaceManifest, MarketplacePlugin, MarketplaceSource, PluginAuthor,
};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{debug, warn};

#[derive(Debug, Error)]
pub enum MarketplaceError {
    #[error("Git 操作失败: {0}")]
    GitFailed(String),
    #[error("HTTP 请求失败: {0}")]
    HttpFailed(String),
    #[error("JSON 解析失败: {0}")]
    ParseFailed(String),
    #[error("marketplace.json 未找到: {path}")]
    ManifestNotFound { path: String },
    #[error("NPM 操作失败: {0}")]
    NpmFailed(String),
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq)]
pub enum MarketplaceStatus {
    Cached,
    Fetching,
    Fresh,
    Stale(String),
    NotFetched,
}

pub struct MarketplaceEntry {
    pub name: String,
    pub source: MarketplaceSource,
    pub manifest: Option<MarketplaceManifest>,
    pub status: MarketplaceStatus,
    pub last_updated: Option<DateTime<Utc>>,
    pub auto_update: bool,
}

#[derive(Debug, Clone)]
pub struct AvailablePlugin {
    pub name: String,
    pub description: String,
    pub version: String,
    pub marketplace: String,
    pub source: serde_json::Value,
    pub author: Option<PluginAuthor>,
    pub category: Option<String>,
    pub homepage: Option<String>,
}

#[derive(Debug, Clone)]
pub enum MarketplaceRefreshEvent {
    Updated {
        index: usize,
        name: String,
    },
    Failed {
        index: usize,
        name: String,
        error: String,
    },
}

pub fn find_marketplace_json(dir: &Path) -> Option<PathBuf> {
    let root = dir.join("marketplace.json");
    if root.exists() {
        return Some(root);
    }
    let subdir = dir.join(".claude-plugin").join("marketplace.json");
    if subdir.exists() {
        return Some(subdir);
    }
    None
}

pub fn read_manifest_from_path(path: &Path) -> Result<MarketplaceManifest, MarketplaceError> {
    let content = std::fs::read_to_string(path)?;
    serde_json::from_str(&content).map_err(|e| MarketplaceError::ParseFailed(e.to_string()))
}

async fn fetch_github(
    name: &str,
    repo: &str,
    cache_base: &Path,
    auto_update: bool,
) -> Result<MarketplaceManifest, MarketplaceError> {
    let cache_dir = cache_base.join(name);

    if !cache_dir.exists() {
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            tokio::process::Command::new("git")
                .args([
                    "clone",
                    "--depth",
                    "1",
                    &format!("https://github.com/{repo}.git"),
                    &cache_dir.display().to_string(),
                ])
                .output(),
        )
        .await
        .map_err(|e| MarketplaceError::GitFailed(format!("clone 超时: {e}")))?
        .map_err(|e| MarketplaceError::GitFailed(format!("clone 执行失败: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(MarketplaceError::GitFailed(format!("clone 失败: {stderr}")));
        }
    } else if auto_update {
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            tokio::process::Command::new("git")
                .args(["-C", &cache_dir.display().to_string(), "pull", "--ff-only"])
                .output(),
        )
        .await
        .map_err(|e| MarketplaceError::GitFailed(format!("pull 超时: {e}")))?
        .map_err(|e| MarketplaceError::GitFailed(format!("pull 执行失败: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("git pull 失败 '{}', 回退到缓存: {stderr}", repo);
            // fall through to read cache
        }
    }

    let manifest_path =
        find_marketplace_json(&cache_dir).ok_or_else(|| MarketplaceError::ManifestNotFound {
            path: cache_dir.display().to_string(),
        })?;
    read_manifest_from_path(&manifest_path)
}

async fn fetch_url(
    name: &str,
    url: &str,
    cache_base: &Path,
) -> Result<MarketplaceManifest, MarketplaceError> {
    let cache_file = cache_base.join(format!("{name}.json"));

    let last_modified = std::fs::metadata(&cache_file)
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|t| {
            let dt: DateTime<Utc> = t.into();
            dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string()
        });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| MarketplaceError::HttpFailed(e.to_string()))?;

    let mut req = client.get(url);
    if let Some(ref lm) = last_modified {
        req = req.header("If-Modified-Since", lm);
    }

    let result = req.send().await;

    match result {
        Ok(response) => match response.status().as_u16() {
            304 => read_manifest_from_path(&cache_file),
            200 => {
                let body = response
                    .text()
                    .await
                    .map_err(|e| MarketplaceError::HttpFailed(e.to_string()))?;
                if let Some(parent) = cache_file.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&cache_file, &body)?;
                serde_json::from_str(&body)
                    .map_err(|e| MarketplaceError::ParseFailed(e.to_string()))
            }
            status => Err(MarketplaceError::HttpFailed(format!("HTTP {status}"))),
        },
        Err(e) => {
            if cache_file.exists() {
                warn!("URL 拉取失败 '{}', 回退到缓存: {}", url, e);
                read_manifest_from_path(&cache_file)
            } else {
                Err(MarketplaceError::HttpFailed(e.to_string()))
            }
        }
    }
}

fn read_file(path: &Path) -> Result<MarketplaceManifest, MarketplaceError> {
    read_manifest_from_path(path)
}

fn read_directory(path: &Path) -> Result<MarketplaceManifest, MarketplaceError> {
    let manifest_path =
        find_marketplace_json(path).ok_or_else(|| MarketplaceError::ManifestNotFound {
            path: path.display().to_string(),
        })?;
    read_manifest_from_path(&manifest_path)
}

async fn fetch_npm(
    name: &str,
    package: &str,
    cache_base: &Path,
) -> Result<MarketplaceManifest, MarketplaceError> {
    let cache_dir = cache_base.join(name);

    if let Some(manifest_path) = find_marketplace_json(&cache_dir) {
        return read_manifest_from_path(&manifest_path);
    }

    let tmp_dir = std::env::temp_dir().join(format!("npm-pack-{package}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp_dir)?;
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        tokio::process::Command::new("npm")
            .args([
                "pack",
                package,
                "--pack-destination",
                &tmp_dir.display().to_string(),
            ])
            .output(),
    )
    .await
    .map_err(|e| MarketplaceError::NpmFailed(format!("npm pack 超时: {e}")))?
    .map_err(|e| MarketplaceError::NpmFailed(format!("npm pack 执行失败: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(MarketplaceError::NpmFailed(format!(
            "npm pack 失败: {stderr}"
        )));
    }

    let tgz_path = std::fs::read_dir(&tmp_dir)?
        .find_map(|e| {
            e.ok().and_then(|f| {
                if f.path()
                    .extension()
                    .map(|ext| ext == "tgz")
                    .unwrap_or(false)
                {
                    Some(f.path())
                } else {
                    None
                }
            })
        })
        .ok_or_else(|| MarketplaceError::NpmFailed("未找到 .tgz 文件".into()))?;

    let file = std::fs::File::open(&tgz_path)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    std::fs::create_dir_all(&cache_dir)?;
    archive.unpack(&cache_dir)?;

    let manifest_path =
        find_marketplace_json(&cache_dir).ok_or_else(|| MarketplaceError::ManifestNotFound {
            path: cache_dir.display().to_string(),
        })?;
    read_manifest_from_path(&manifest_path)
}

pub struct MarketplaceManager {
    entries: Vec<MarketplaceEntry>,
    override_dir: Option<PathBuf>,
}

impl MarketplaceManager {
    pub fn new(override_dir: Option<PathBuf>) -> Self {
        MarketplaceManager {
            entries: Vec::new(),
            override_dir,
        }
    }

    fn cache_base(&self) -> PathBuf {
        if let Some(ref dir) = self.override_dir {
            dir.join("marketplaces")
        } else {
            marketplaces_cache_dir()
        }
    }

    fn known_marketplaces_path(&self) -> PathBuf {
        if let Some(ref dir) = self.override_dir {
            dir.join("known_marketplaces.json")
        } else {
            crate::plugin::config::known_marketplaces_path()
        }
    }

    fn claude_settings_path(&self) -> PathBuf {
        if let Some(ref dir) = self.override_dir {
            dir.join("settings.json")
        } else {
            crate::plugin::config::claude_settings_path()
        }
    }

    fn try_load_cache(
        &self,
        source: &MarketplaceSource,
        name: &str,
    ) -> Option<MarketplaceManifest> {
        let cache_base = self.cache_base();
        let path: Option<PathBuf> = match source {
            MarketplaceSource::GitHub { .. } => find_marketplace_json(&cache_base.join(name)),
            MarketplaceSource::Url { .. } => {
                let p = cache_base.join(format!("{name}.json"));
                if p.exists() {
                    Some(p)
                } else {
                    None
                }
            }
            MarketplaceSource::File { path } => {
                let p = PathBuf::from(path);
                if p.exists() {
                    Some(p)
                } else {
                    None
                }
            }
            MarketplaceSource::Directory { path } => find_marketplace_json(Path::new(path)),
            MarketplaceSource::Npm { .. } => find_marketplace_json(&cache_base.join(name)),
        };
        path.and_then(|p| {
            read_manifest_from_path(&p)
                .map_err(|e| {
                    debug!("缓存 manifest 读取失败 {:?}: {}", p, e);
                    e
                })
                .ok()
        })
    }

    fn extract_name(source: &MarketplaceSource) -> String {
        match source {
            MarketplaceSource::GitHub { repo } => {
                repo.split('/').next_back().unwrap_or(repo).to_string()
            }
            MarketplaceSource::Url { url } => url::Url::parse(url)
                .ok()
                .and_then(|u| {
                    u.path_segments()
                        .and_then(|segs| segs.last().map(|s| s.to_string()))
                })
                .map(|s| s.trim_end_matches(".json").to_string())
                .unwrap_or_else(|| "url-marketplace".into()),
            MarketplaceSource::File { path } => PathBuf::from(path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "file-marketplace".into()),
            MarketplaceSource::Directory { path } => PathBuf::from(path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "dir-marketplace".into()),
            MarketplaceSource::Npm { package } => package.clone(),
        }
    }

    pub async fn init(
        &mut self,
        tx: mpsc::Sender<MarketplaceRefreshEvent>,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        use crate::plugin::config::{load_known_marketplaces, save_known_marketplaces};

        let km_path = self.known_marketplaces_path();
        let settings_path = self.claude_settings_path();
        let mut known = load_known_marketplaces(Some(&km_path)).unwrap_or_default();
        let settings = load_claude_settings(Some(&settings_path)).unwrap_or_default();

        // Merge extra known marketplaces from settings.json
        for extra in &settings.extra_known_marketplaces {
            let extra_json = serde_json::to_string(&extra.source).unwrap_or_default();
            let already_exists = known
                .iter()
                .any(|km| serde_json::to_string(&km.source).unwrap_or_default() == extra_json);
            if !already_exists {
                known.push(extra.clone());
            }
        }

        // Auto-register official marketplace
        let has_official = known.iter().any(|km| match &km.source {
            MarketplaceSource::GitHub { repo } => repo == "anthropics/claude-plugins-official",
            _ => false,
        });
        if !has_official {
            let official = KnownMarketplace {
                source: MarketplaceSource::GitHub {
                    repo: "anthropics/claude-plugins-official".into(),
                },
                install_location: None,
                auto_update: true,
                last_updated: None,
            };
            known.push(official);
            let _ = save_known_marketplaces(&known, Some(&km_path));
        }

        // Build entries
        self.entries.clear();
        for km in &known {
            let name = Self::extract_name(&km.source);
            let cached_manifest = self.try_load_cache(&km.source, &name);
            let status = if cached_manifest.is_some() {
                MarketplaceStatus::Cached
            } else {
                MarketplaceStatus::NotFetched
            };
            let last_updated = km.last_updated.as_ref().and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            });
            self.entries.push(MarketplaceEntry {
                name,
                source: km.source.clone(),
                manifest: cached_manifest,
                status,
                last_updated,
                auto_update: km.auto_update,
            });
        }

        // Spawn background refresh tasks
        let mut handles = Vec::new();
        for i in 0..self.entries.len() {
            handles.push(self.spawn_refresh(i, tx.clone()));
        }
        handles
    }

    pub fn spawn_refresh(
        &self,
        index: usize,
        tx: mpsc::Sender<MarketplaceRefreshEvent>,
    ) -> tokio::task::JoinHandle<()> {
        let name = self.entries[index].name.clone();
        let source = self.entries[index].source.clone();
        let auto_update = self.entries[index].auto_update;
        let cache_base = self.cache_base();

        tokio::spawn(async move {
            let result = match &source {
                MarketplaceSource::GitHub { repo } => {
                    fetch_github(&name, repo, &cache_base, auto_update).await
                }
                MarketplaceSource::Url { url } => fetch_url(&name, url, &cache_base).await,
                MarketplaceSource::File { path } => {
                    let p = path.clone();
                    tokio::task::spawn_blocking(move || read_file(Path::new(&p)))
                        .await
                        .expect("spawn_blocking panicked")
                }
                MarketplaceSource::Directory { path } => {
                    let p = path.clone();
                    tokio::task::spawn_blocking(move || read_directory(Path::new(&p)))
                        .await
                        .expect("spawn_blocking panicked")
                }
                MarketplaceSource::Npm { package } => fetch_npm(&name, package, &cache_base).await,
            };

            match result {
                Ok(manifest) => {
                    let _ = tx
                        .send(MarketplaceRefreshEvent::Updated {
                            index,
                            name: name.clone(),
                        })
                        .await;
                    // Update entry is handled by caller via update_entry
                    let _ = manifest; // caller uses event to trigger update
                }
                Err(e) => {
                    warn!("Marketplace '{}' 刷新失败: {}", name, e);
                    let _ = tx
                        .send(MarketplaceRefreshEvent::Failed {
                            index,
                            name: name.clone(),
                            error: e.to_string(),
                        })
                        .await;
                }
            }
        })
    }

    pub fn entries(&self) -> &[MarketplaceEntry] {
        &self.entries
    }

    /// 公共包装：从缓存加载 manifest
    pub fn try_load_cache_wrapper(
        &self,
        source: &MarketplaceSource,
        name: &str,
    ) -> Option<MarketplaceManifest> {
        self.try_load_cache(source, name)
    }

    /// 公共包装：从 source 提取 marketplace 名称
    pub fn extract_name_wrapper(source: &MarketplaceSource) -> String {
        Self::extract_name(source)
    }

    pub fn update_entry(
        &mut self,
        index: usize,
        manifest: MarketplaceManifest,
        status: MarketplaceStatus,
    ) {
        if let Some(entry) = self.entries.get_mut(index) {
            entry.manifest = Some(manifest);
            entry.status = status;
            entry.last_updated = Some(Utc::now());
        }
    }

    pub fn find_plugin(&self, plugin_name: &str) -> Option<(&MarketplacePlugin, &str)> {
        for entry in &self.entries {
            if entry.status != MarketplaceStatus::Cached && entry.status != MarketplaceStatus::Fresh
            {
                continue;
            }
            if let Some(ref manifest) = entry.manifest {
                for plugin in &manifest.plugins {
                    if plugin.name == plugin_name {
                        return Some((plugin, &entry.name));
                    }
                }
            }
        }
        None
    }

    pub fn available_plugins(&self) -> Vec<AvailablePlugin> {
        let mut result = Vec::new();
        for entry in &self.entries {
            if entry.status != MarketplaceStatus::Cached && entry.status != MarketplaceStatus::Fresh
            {
                continue;
            }
            if let Some(ref manifest) = entry.manifest {
                for p in &manifest.plugins {
                    result.push(AvailablePlugin {
                        name: p.name.clone(),
                        description: p.description.clone(),
                        version: p.version.clone(),
                        marketplace: entry.name.clone(),
                        source: p.source.clone(),
                        author: p.author.clone(),
                        category: p.category.clone(),
                        homepage: p.homepage.clone(),
                    });
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_find_marketplace_json_root() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("marketplace.json"), "{}").unwrap();
        let result = find_marketplace_json(dir.path());
        assert!(result.is_some());
        assert_eq!(result.unwrap().file_name().unwrap(), "marketplace.json");
    }

    #[test]
    fn test_find_marketplace_json_subdir() {
        let dir = tempdir().unwrap();
        let subdir = dir.path().join(".claude-plugin");
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::write(subdir.join("marketplace.json"), "{}").unwrap();
        let result = find_marketplace_json(dir.path());
        assert!(result.is_some());
    }

    #[test]
    fn test_find_marketplace_json_not_found() {
        let dir = tempdir().unwrap();
        let result = find_marketplace_json(dir.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_find_marketplace_json_priority() {
        let dir = tempdir().unwrap();
        let subdir = dir.path().join(".claude-plugin");
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::write(dir.path().join("marketplace.json"), "root").unwrap();
        std::fs::write(subdir.join("marketplace.json"), "sub").unwrap();
        let result = find_marketplace_json(dir.path()).unwrap();
        let content = std::fs::read_to_string(result).unwrap();
        assert_eq!(content, "root");
    }

    #[test]
    fn test_read_manifest_from_path_success() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("marketplace.json");
        let json = r#"{"name":"test","plugins":[]}"#;
        std::fs::write(&path, json).unwrap();
        let manifest = read_manifest_from_path(&path).unwrap();
        assert_eq!(manifest.name, "test");
        assert!(manifest.plugins.is_empty());
    }

    #[test]
    fn test_read_manifest_from_path_invalid_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("marketplace.json");
        std::fs::write(&path, "not json").unwrap();
        let result = read_manifest_from_path(&path);
        assert!(result.is_err());
        match result.unwrap_err() {
            MarketplaceError::ParseFailed(_) => {}
            _ => panic!("expected ParseFailed"),
        }
    }

    #[test]
    fn test_read_manifest_from_path_not_found() {
        let result = read_manifest_from_path(Path::new("/nonexistent/path.json"));
        assert!(result.is_err());
    }

    #[test]
    fn test_fetch_github_cache_hit() {
        let dir = tempdir().unwrap();
        let cache_base = dir.path().join("marketplaces");
        let cache_dir = cache_base.join("test-repo");
        let plugin_dir = cache_dir.join(".claude-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        let json = r#"{"name":"cached-marketplace","plugins":[{"name":"p1","description":"d","source":"s","version":"1.0.0"}]}"#;
        std::fs::write(plugin_dir.join("marketplace.json"), json).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let manifest = rt
            .block_on(fetch_github("test-repo", "some/repo", &cache_base, false))
            .unwrap();
        assert_eq!(manifest.name, "cached-marketplace");
        assert_eq!(manifest.plugins.len(), 1);
    }

    #[test]
    fn test_read_file_success() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("marketplace.json");
        let json = r#"{"name":"file-test","plugins":[]}"#;
        std::fs::write(&path, json).unwrap();
        let manifest = read_file(&path).unwrap();
        assert_eq!(manifest.name, "file-test");
    }

    #[test]
    fn test_read_file_not_found() {
        let result = read_file(Path::new("/nonexistent/file.json"));
        assert!(result.is_err());
    }

    #[test]
    fn test_read_directory_root() {
        let dir = tempdir().unwrap();
        let json = r#"{"name":"dir-test","plugins":[]}"#;
        std::fs::write(dir.path().join("marketplace.json"), json).unwrap();
        let manifest = read_directory(dir.path()).unwrap();
        assert_eq!(manifest.name, "dir-test");
    }

    #[test]
    fn test_read_directory_subdir() {
        let dir = tempdir().unwrap();
        let subdir = dir.path().join(".claude-plugin");
        std::fs::create_dir_all(&subdir).unwrap();
        let json = r#"{"name":"subdir-test","plugins":[]}"#;
        std::fs::write(subdir.join("marketplace.json"), json).unwrap();
        let manifest = read_directory(dir.path()).unwrap();
        assert_eq!(manifest.name, "subdir-test");
    }

    #[test]
    fn test_read_directory_not_found() {
        let dir = tempdir().unwrap();
        let result = read_directory(dir.path());
        assert!(result.is_err());
        match result.unwrap_err() {
            MarketplaceError::ManifestNotFound { .. } => {}
            _ => panic!("expected ManifestNotFound"),
        }
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "integration"), ignore)]
    async fn test_fetch_url_cache_fallback() {
        let dir = tempdir().unwrap();
        let cache_base = dir.path().join("marketplaces");
        std::fs::create_dir_all(&cache_base).unwrap();
        let json = r#"{"name":"cached-url","plugins":[]}"#;
        std::fs::write(cache_base.join("test.json"), json).unwrap();
        let manifest = fetch_url("test", "http://127.0.0.1:1/nonexistent.json", &cache_base)
            .await
            .unwrap();
        assert_eq!(manifest.name, "cached-url");
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "integration"), ignore)]
    async fn test_fetch_url_no_cache_no_server() {
        let dir = tempdir().unwrap();
        let cache_base = dir.path().join("marketplaces");
        std::fs::create_dir_all(&cache_base).unwrap();
        let result = fetch_url("test", "http://127.0.0.1:1/nonexistent.json", &cache_base).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_manager_auto_register_official() {
        let dir = tempdir().unwrap();
        let (tx, _rx) = mpsc::channel(16);
        let mut manager = MarketplaceManager::new(Some(dir.path().to_path_buf()));
        let handles = manager.init(tx).await;

        // Check that official marketplace was registered
        let km_path = dir.path().join("known_marketplaces.json");
        assert!(km_path.exists());
        let known: Vec<KnownMarketplace> =
            serde_json::from_str(&std::fs::read_to_string(&km_path).unwrap()).unwrap();
        assert!(known.iter().any(|km| match &km.source {
            MarketplaceSource::GitHub { repo } => repo == "anthropics/claude-plugins-official",
            _ => false,
        }));

        for h in handles {
            h.abort();
        }
    }

    #[tokio::test]
    async fn test_manager_merge_extra_known_marketplaces() {
        let dir = tempdir().unwrap();
        let settings_path = dir.path().join("settings.json");
        let settings = r#"{
            "extraKnownMarketplaces": [
                {"source": {"source":"file","path":"/test/marketplace.json"}}
            ]
        }"#;
        std::fs::write(&settings_path, settings).unwrap();

        let (tx, _rx) = mpsc::channel(16);
        let mut manager = MarketplaceManager::new(Some(dir.path().to_path_buf()));
        let handles = manager.init(tx).await;

        assert!(manager.entries().iter().any(|e| match &e.source {
            MarketplaceSource::File { path } => path == "/test/marketplace.json",
            _ => false,
        }));

        for h in handles {
            h.abort();
        }
    }

    #[tokio::test]
    async fn test_manager_cache_loading() {
        let dir = tempdir().unwrap();
        let marketplaces_dir = dir.path().join("marketplaces");
        std::fs::create_dir_all(&marketplaces_dir).unwrap();
        let json = r#"{"name":"cached-test","plugins":[{"name":"p1","description":"Plugin 1","source":"s","version":"1.0.0"}]}"#;
        std::fs::write(marketplaces_dir.join("test.json"), json).unwrap();

        let km_path = dir.path().join("known_marketplaces.json");
        let known = r#"[{"source":{"source":"url","url":"https://example.com/test.json"}}]"#;
        std::fs::write(&km_path, known).unwrap();

        let (tx, _rx) = mpsc::channel(16);
        let mut manager = MarketplaceManager::new(Some(dir.path().to_path_buf()));
        let handles = manager.init(tx).await;

        let cached_entry = manager.entries().iter().find(|e| e.name == "test");
        assert!(cached_entry.is_some());
        let entry = cached_entry.unwrap();
        assert_eq!(entry.status, MarketplaceStatus::Cached);
        assert!(entry.manifest.is_some());

        for h in handles {
            h.abort();
        }
    }

    #[test]
    fn test_manager_find_plugin() {
        let mut manager = MarketplaceManager::new(None);
        let manifest = MarketplaceManifest {
            name: "test-mkt".into(),
            plugins: vec![MarketplacePlugin {
                name: "target-plugin".into(),
                description: "desc".into(),
                source: serde_json::json!("src"),
                version: "1.0.0".into(),
                sha: None,
                author: None,
                category: None,
                homepage: None,
                tags: None,
            }],
            allow_cross_marketplace: None,
        };
        manager.entries.push(MarketplaceEntry {
            name: "test-mkt".into(),
            source: MarketplaceSource::Directory {
                path: "/tmp/test".into(),
            },
            manifest: Some(manifest),
            status: MarketplaceStatus::Cached,
            last_updated: None,
            auto_update: false,
        });
        let result = manager.find_plugin("target-plugin");
        assert!(result.is_some());
        assert_eq!(result.unwrap().0.name, "target-plugin");
    }

    #[test]
    fn test_manager_find_plugin_not_found() {
        let mut manager = MarketplaceManager::new(None);
        let manifest = MarketplaceManifest {
            name: "test-mkt".into(),
            plugins: vec![],
            allow_cross_marketplace: None,
        };
        manager.entries.push(MarketplaceEntry {
            name: "test-mkt".into(),
            source: MarketplaceSource::Directory {
                path: "/tmp/test".into(),
            },
            manifest: Some(manifest),
            status: MarketplaceStatus::Cached,
            last_updated: None,
            auto_update: false,
        });
        assert!(manager.find_plugin("nonexistent").is_none());
    }

    #[test]
    fn test_manager_available_plugins() {
        let mut manager = MarketplaceManager::new(None);
        let manifest1 = MarketplaceManifest {
            name: "mkt1".into(),
            plugins: vec![
                MarketplacePlugin {
                    name: "p1".into(),
                    description: "d1".into(),
                    source: serde_json::json!("s1"),
                    version: "1.0.0".into(),
                    sha: None,
                    author: None,
                    category: None,
                    homepage: None,
                    tags: None,
                },
                MarketplacePlugin {
                    name: "p2".into(),
                    description: "d2".into(),
                    source: serde_json::json!("s2"),
                    version: "2.0.0".into(),
                    sha: None,
                    author: None,
                    category: None,
                    homepage: None,
                    tags: None,
                },
            ],
            allow_cross_marketplace: None,
        };
        manager.entries.push(MarketplaceEntry {
            name: "mkt1".into(),
            source: MarketplaceSource::Directory { path: "/t".into() },
            manifest: Some(manifest1),
            status: MarketplaceStatus::Fresh,
            last_updated: None,
            auto_update: false,
        });
        // NotFetched entry should be skipped
        manager.entries.push(MarketplaceEntry {
            name: "mkt2".into(),
            source: MarketplaceSource::Directory { path: "/t2".into() },
            manifest: None,
            status: MarketplaceStatus::NotFetched,
            last_updated: None,
            auto_update: false,
        });

        let available = manager.available_plugins();
        assert_eq!(available.len(), 2);
        assert_eq!(available[0].name, "p1");
        assert_eq!(available[1].name, "p2");
    }

    #[test]
    fn test_manager_update_entry() {
        let mut manager = MarketplaceManager::new(None);
        manager.entries.push(MarketplaceEntry {
            name: "test".into(),
            source: MarketplaceSource::Directory { path: "/t".into() },
            manifest: None,
            status: MarketplaceStatus::NotFetched,
            last_updated: None,
            auto_update: false,
        });
        let manifest = MarketplaceManifest {
            name: "updated".into(),
            plugins: vec![],
            allow_cross_marketplace: None,
        };
        manager.update_entry(0, manifest, MarketplaceStatus::Fresh);
        assert_eq!(manager.entries[0].status, MarketplaceStatus::Fresh);
        assert!(manager.entries[0].manifest.is_some());
        assert!(manager.entries[0].last_updated.is_some());
    }
}
