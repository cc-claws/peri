//! SearchExtraTools 元工具 — 搜索并发现延迟加载的工具

use std::sync::Arc;

use async_trait::async_trait;
use rust_create_agent::tools::BaseTool;
use serde_json::{json, Value};

use super::core_tools::SEARCH_EXTRA_TOOLS_NAME;
use super::tool_index::ToolSearchIndex;

/// 搜索延迟加载工具的元工具
///
/// LLM 通过此工具发现不在直接工具列表中的 deferred tools，
/// 获取完整 schema 后通过 ExecuteExtraTool 调用。
pub struct SearchExtraTools {
    index: Arc<ToolSearchIndex>,
}

impl SearchExtraTools {
    pub fn new(index: Arc<ToolSearchIndex>) -> Self {
        Self { index }
    }
}

#[async_trait]
impl BaseTool for SearchExtraTools {
    fn name(&self) -> &str {
        SEARCH_EXTRA_TOOLS_NAME
    }

    fn description(&self) -> &str {
        "Search for deferred tools by name or keyword. LOW PRIORITY — only use this tool when no core tool can accomplish the task. Core tools (Read, Edit, Write, Bash, Glob, Grep, Agent, WebFetch, WebSearch, AskUserQuestion, TodoWrite) are always available and should be used directly. This tool is for discovering additional capabilities like MCP tools, cron scheduling, etc.\n\nReturns matching tools with their full JSON schemas.\n\nIMPORTANT: ExecuteExtraTool is always available in your tool list. After this search returns tool names, you MUST call ExecuteExtraTool with {\"tool_name\": \"<returned_name>\", \"params\": {...}} to invoke the deferred tool. This is the ONLY way to execute deferred tools — do not read source code or analyze whether the tool is callable, just use ExecuteExtraTool directly.\n\nQuery forms:\n- \"select:CronCreate,Snip\" — fetch these exact tools by name\n- \"slack send\" — keyword search, best matches returned"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Query to find deferred tools. Use \"select:<tool_name>\" for direct selection, or keywords to search."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 5)"
                }
            },
            "required": ["query"]
        })
    }

    async fn invoke(
        &self,
        input: Value,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or("SearchExtraTools: missing required 'query' parameter")?;

        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;

        let results = self.index.search(query, max_results);
        let total = self.index.total_count();
        let output = json!({
            "results": results,
            "total_available": total
        });

        Ok(serde_json::to_string(&output)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_search::tool_index::ToolSearchIndex;

    struct MockTool {
        name_str: String,
        desc_str: String,
    }

    impl MockTool {
        fn new(name: &str, desc: &str) -> Self {
            Self {
                name_str: name.to_string(),
                desc_str: desc.to_string(),
            }
        }
    }

    #[async_trait]
    impl BaseTool for MockTool {
        fn name(&self) -> &str {
            &self.name_str
        }
        fn description(&self) -> &str {
            &self.desc_str
        }
        fn parameters(&self) -> Value {
            json!({"type": "object", "properties": {}})
        }
        async fn invoke(
            &self,
            _input: Value,
        ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
            Ok("mock".to_string())
        }
    }

    fn build_test_index() -> Arc<ToolSearchIndex> {
        let index = Arc::new(ToolSearchIndex::new());
        index.build(vec![
            Arc::new(MockTool::new(
                "mcp__slack__send_message",
                "Send a message to Slack channel",
            )),
            Arc::new(MockTool::new(
                "mcp__slack__get_channel",
                "Get Slack channel info",
            )),
            Arc::new(MockTool::new(
                "CronRegister",
                "Register a cron scheduled task",
            )),
        ]);
        index
    }

    #[test]
    fn test_tool_name_is_search_extra_tools() {
        let index = build_test_index();
        let tool = SearchExtraTools::new(index);
        assert_eq!(tool.name(), "SearchExtraTools");
    }

    #[test]
    fn test_parameters_schema() {
        let index = build_test_index();
        let tool = SearchExtraTools::new(index);
        let params = tool.parameters();
        assert_eq!(params["type"], "object");
        assert!(params["properties"]["query"].is_object());
        let required = params["required"].as_array().unwrap();
        assert!(required.contains(&json!("query")));
    }

    #[tokio::test]
    async fn test_invoke_search_returns_results() {
        let index = build_test_index();
        let tool = SearchExtraTools::new(index);

        let result = tool
            .invoke(json!({"query": "slack message"}))
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert!(parsed["results"].is_array());
        assert!(parsed["total_available"].is_number());
        let results = parsed["results"].as_array().unwrap();
        assert!(!results.is_empty());
        assert!(results[0]["name"].as_str().unwrap().contains("slack"));
    }

    #[tokio::test]
    async fn test_invoke_empty_results() {
        let index = build_test_index();
        let tool = SearchExtraTools::new(index);

        let result = tool
            .invoke(json!({"query": "nonexistent_tool_xyz"}))
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();

        // TF-IDF may still return results, but total_available should be > 0
        assert!(parsed["total_available"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn test_invoke_missing_query() {
        let index = build_test_index();
        let tool = SearchExtraTools::new(index);

        let result = tool.invoke(json!({})).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("missing required 'query' parameter"));
    }
}
