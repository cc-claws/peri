//! Workflow schema types — the in-memory representation of a DAG workflow.
//!
//! **Core types** (`Workflow`, `NodeDef`, etc.) are always available.
//!
//! **YAML parsing** (`parse_workflow`) is gated behind `#[cfg(feature = "yaml")]`.

use std::collections::HashMap;

use crate::core::error::{CoreError, ErrorKind};

// ─── Top-Level Workflow ───────────────────────────────────────────

/// Complete workflow definition.
#[derive(Debug, Clone)]
pub struct Workflow {
    pub name: String,
    pub description: Option<String>,
    pub version: String,

    pub defaults: NodeDefaults,

    /// External input parameter declarations.
    pub inputs: HashMap<String, InputDef>,

    /// Global environment variables.
    pub env: HashMap<String, String>,

    /// Referenced sub-workflow alias → path/URL.
    pub references: HashMap<String, String>,

    /// Workflow-level timeout in seconds. `None` = unlimited.
    pub timeout: Option<u64>,

    /// Node list.
    pub nodes: Vec<NodeDef>,

    /// Parameters passed when referencing this workflow via `with`.
    /// Core stores this as `HashMap<String, String>` (no serde_yaml::Value).
    pub with: HashMap<String, String>,

    /// Runtime-only: maps reference node ID prefix to bound input values.
    /// Populated by the loader during reference expansion.
    pub reference_inputs: HashMap<String, HashMap<String, String>>,

    /// Runtime-only: maps reference node ID to its exit node IDs.
    /// Used to forward exit node outputs back to the reference node ID
    /// so downstream templates can reference `needs.<ref_id>.outputs.*`.
    pub output_forward: HashMap<String, Vec<String>>,
}

// ─── Input Definition ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct InputDef {
    pub input_type: InputType,
    pub default: Option<String>,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputType {
    String,
    Number,
    Boolean,
}

// ─── Node Defaults ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct NodeDefaults {
    pub retry: u32,
    pub timeout: u64,
    pub shell: String,
}

impl Default for NodeDefaults {
    fn default() -> Self {
        Self {
            retry: 0,
            timeout: 300,
            shell: "bash -c".into(),
        }
    }
}

// ─── Node Definition ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum NodeDef {
    Shell(ShellNode),
    Agent(AgentNode),
    Reference(ReferenceNode),
}

// ─── Shell Node ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ShellNode {
    pub id: String,
    /// Script source: inline | file | platform-specific.
    pub run: ScriptSource,
    pub depends: Vec<String>,
    pub outputs: HashMap<String, String>,
    pub env: HashMap<String, String>,
    pub continue_on_error: bool,
    pub exec: ExecConfig,
}

// ─── Agent Node ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AgentNode {
    pub id: String,
    pub prompt: PromptSource,
    /// Agent sub-command name (peri / claude / codex etc.), default "peri".
    pub agent: Option<String>,
    pub model: Option<String>,
    pub cwd: Option<String>,
    pub depends: Vec<String>,
    pub outputs: HashMap<String, String>,
    pub env: HashMap<String, String>,
    pub continue_on_error: bool,
    pub exec: ExecConfig,
}

// ─── Reference Node ───────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ReferenceNode {
    pub id: String,
    /// Alias in the top-level `references` map.
    pub r#ref: String,
    /// Parameters to pass to the sub-workflow.
    pub with: HashMap<String, String>,
    pub depends: Vec<String>,
    pub outputs: HashMap<String, String>,
    pub continue_on_error: bool,
    pub exec: ExecConfig,
}

// ─── Script / Prompt Source ───────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ScriptSource {
    Inline(String),
    File(FileSource),
    Platform(PlatformFiles),
}

#[derive(Debug, Clone)]
pub struct FileSource {
    pub file: String,
}

#[derive(Debug, Clone)]
pub enum PromptSource {
    Inline(String),
    File(FileSource),
    Platform(PlatformFiles),
}

#[derive(Debug, Clone)]
pub struct PlatformFiles {
    pub linux: Option<String>,
    pub macos: Option<String>,
    pub windows: Option<String>,
    pub default: Option<String>,
}

// ─── Execution Config ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ExecConfig {
    pub timeout: Option<u64>,
    pub retry: Option<u32>,
    pub shell: Option<String>,
    /// Conditional expression. Evaluated as false → skip node.
    pub r#if: Option<String>,
}

// ─── Platform Resolution ──────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    MacOs,
    Windows,
}

impl Platform {
    pub fn detect() -> Self {
        if cfg!(target_os = "linux") {
            Platform::Linux
        } else if cfg!(target_os = "macos") {
            Platform::MacOs
        } else if cfg!(target_os = "windows") {
            Platform::Windows
        } else {
            match std::env::consts::OS {
                "linux" => Platform::Linux,
                "macos" => Platform::MacOs,
                "windows" => Platform::Windows,
                _ => Platform::Linux,
            }
        }
    }
}

impl ScriptSource {
    pub fn resolve(&self, platform: Platform) -> Result<ResolvedScript, CoreError> {
        match self {
            ScriptSource::Inline(s) => Ok(ResolvedScript::Inline(s.clone())),
            ScriptSource::File(f) => Ok(ResolvedScript::File(f.file.clone())),
            ScriptSource::Platform(pf) => {
                let path = pf.resolve(platform)?;
                Ok(ResolvedScript::File(path))
            }
        }
    }
}

impl PromptSource {
    pub fn resolve(&self, platform: Platform) -> Result<ResolvedPrompt, CoreError> {
        match self {
            PromptSource::Inline(s) => Ok(ResolvedPrompt::Inline(s.clone())),
            PromptSource::File(f) => Ok(ResolvedPrompt::File(f.file.clone())),
            PromptSource::Platform(pf) => {
                let path = pf.resolve(platform)?;
                Ok(ResolvedPrompt::File(path))
            }
        }
    }
}

impl PlatformFiles {
    pub fn resolve(&self, platform: Platform) -> Result<String, CoreError> {
        let key = match platform {
            Platform::Linux => &self.linux,
            Platform::MacOs => &self.macos,
            Platform::Windows => &self.windows,
        };

        if let Some(path) = key {
            return Ok(path.clone());
        }
        if let Some(path) = &self.default {
            return Ok(path.clone());
        }
        Err(CoreError::new(
            ErrorKind::Validation,
            format!(
                "no script defined for platform {:?} and no default fallback",
                platform
            ),
        ))
    }
}

#[derive(Debug, Clone)]
pub enum ResolvedScript {
    Inline(String),
    File(String),
}

#[derive(Debug, Clone)]
pub enum ResolvedPrompt {
    Inline(String),
    File(String),
}

// ─── Node Accessors ───────────────────────────────────────────────

pub fn node_id(node: &NodeDef) -> &str {
    match node {
        NodeDef::Shell(n) => &n.id,
        NodeDef::Agent(n) => &n.id,
        NodeDef::Reference(n) => &n.id,
    }
}

pub fn node_depends(node: &NodeDef) -> &[String] {
    match node {
        NodeDef::Shell(n) => &n.depends,
        NodeDef::Agent(n) => &n.depends,
        NodeDef::Reference(n) => &n.depends,
    }
}

pub fn node_type_name(node: &NodeDef) -> &str {
    match node {
        NodeDef::Shell(_) => "shell",
        NodeDef::Agent(_) => "agent",
        NodeDef::Reference(_) => "reference",
    }
}

pub fn node_continue_on_error(node: &NodeDef) -> bool {
    match node {
        NodeDef::Shell(n) => n.continue_on_error,
        NodeDef::Agent(n) => n.continue_on_error,
        NodeDef::Reference(n) => n.continue_on_error,
    }
}

pub fn node_if_condition(node: &NodeDef) -> Option<&str> {
    match node {
        NodeDef::Shell(n) => n.exec.r#if.as_deref(),
        NodeDef::Agent(n) => n.exec.r#if.as_deref(),
        NodeDef::Reference(n) => n.exec.r#if.as_deref(),
    }
}

pub fn get_node_outputs(node: &NodeDef) -> &HashMap<String, String> {
    match node {
        NodeDef::Shell(n) => &n.outputs,
        NodeDef::Agent(n) => &n.outputs,
        NodeDef::Reference(n) => &n.outputs,
    }
}

pub fn get_node_env(node: &NodeDef) -> &HashMap<String, String> {
    match node {
        NodeDef::Shell(n) => &n.env,
        NodeDef::Agent(n) => &n.env,
        NodeDef::Reference(_) => &EMPTY_MAP,
    }
}

static EMPTY_MAP: std::sync::LazyLock<HashMap<String, String>> =
    std::sync::LazyLock::new(HashMap::new);

// ─── Validation (always available) ────────────────────────────────

/// Validate a parsed workflow has required fields and consistent structure.
pub fn validate_workflow(wf: &Workflow) -> Result<(), CoreError> {
    if wf.name.trim().is_empty() {
        return Err(CoreError::validation("workflow name must not be empty"));
    }
    if wf.version.trim().is_empty() {
        return Err(CoreError::validation("workflow version must not be empty"));
    }
    if wf.nodes.is_empty() {
        return Err(CoreError::validation(
            "workflow must have at least one node",
        ));
    }

    let mut seen_ids = std::collections::HashSet::new();
    for node in &wf.nodes {
        let id = node_id(node);
        if id.trim().is_empty() {
            return Err(CoreError::validation("node id must not be empty"));
        }
        if !id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/')
        {
            return Err(CoreError::validation(format!(
                "node id '{}' contains invalid characters (allowed: alphanumeric, '-', '_', '.', '/')",
                id
            )));
        }
        if !seen_ids.insert(id.to_string()) {
            return Err(CoreError::validation(format!("duplicate node id '{}'", id)));
        }
        let depends = node_depends(node);
        if depends.iter().any(|d| d == id) {
            return Err(CoreError::validation(format!(
                "node '{}' cannot depend on itself",
                id
            )));
        }
        for dep in depends.iter() {
            if !dep
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/')
            {
                return Err(CoreError::validation(format!(
                    "dependency '{}' in node '{}' contains invalid characters",
                    dep, id
                )));
            }
        }
    }

    for node in &wf.nodes {
        let id = node_id(node);
        for dep in node_depends(node) {
            if !seen_ids.contains(dep) {
                return Err(CoreError::validation(format!(
                    "node '{}' depends on '{}', which does not exist. Available nodes: {}",
                    id,
                    dep,
                    seen_ids.iter().cloned().collect::<Vec<_>>().join(", ")
                )));
            }
        }
    }

    for node in &wf.nodes {
        if let NodeDef::Reference(ref_node) = node {
            if !wf.references.contains_key(&ref_node.r#ref) {
                return Err(CoreError::validation(format!(
                    "reference node '{}' references '{}' which is not defined in the references section",
                    ref_node.id,
                    ref_node.r#ref
                )));
            }
        }
    }

    Ok(())
}

// ─── YAML Parsing (feature-gated) ────────────────────────────────

#[cfg(feature = "yaml")]
mod yaml_impl;

#[cfg(feature = "yaml")]
pub use yaml_impl::parse_workflow;
