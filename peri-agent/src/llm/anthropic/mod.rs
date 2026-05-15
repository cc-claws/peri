use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};

use super::BaseModel;
use crate::agent::events::AgentEvent;
use crate::agent::react::{ReactLLM, Reasoning, ToolCall};
use crate::error::{AgentError, AgentResult};
use crate::llm::sse::SseParser;
use crate::llm::types::{LlmRequest, LlmResponse, StopReason, StreamingContext};
use crate::messages::{BaseMessage, ContentBlock, ImageSource, MessageContent, ToolCallRequest};
use crate::tools::BaseTool;

/// system prompt 边界标记：之前的内容可被 Anthropic prompt cache 命中，
/// 之后的内容变化不会破坏前缀缓存。
const SYSTEM_PROMPT_DYNAMIC_BOUNDARY: &str = "__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__";

/// system prompt 的独立缓存块
struct SystemPromptBlock {
    text: String,
    cache_control: bool,
}

/// ChatAnthropic - Anthropic Messages API 实现
pub struct ChatAnthropic {
    pub api_key: String,
    pub model: String,
    pub extended_thinking: bool,
    pub thinking_budget: u32,
    /// 思考强度 "low" / "medium" / "high"（output_config.effort）
    pub thinking_effort: String,
    /// 是否开启 Prompt Caching（anthropic-beta: prompt-caching-2024-07-31），默认开启
    pub enable_cache: bool,
    /// 自定义 base URL（代理场景），不含末尾 /
    pub base_url: Option<String>,
    client: reqwest::Client,
}

impl ChatAnthropic {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            extended_thinking: false,
            thinking_budget: 10000,
            thinking_effort: "medium".to_string(),
            enable_cache: true,
            base_url: None,
            client: reqwest::Client::new(),
        }
    }

    /// 设置自定义 base URL（用于代理或兼容 API）
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        let url = base_url.into();
        self.base_url = if url.is_empty() { None } else { Some(url) };
        self
    }

    /// 开启 Extended Thinking（claude-3-7-sonnet 及以上）
    pub fn with_extended_thinking(mut self, budget_tokens: u32, effort: impl Into<String>) -> Self {
        self.extended_thinking = true;
        self.thinking_budget = budget_tokens;
        self.thinking_effort = effort.into();
        self
    }

    /// 关闭 Prompt Caching
    pub fn without_cache(mut self) -> Self {
        self.enable_cache = false;
        self
    }

    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY").ok()?;
        let model = std::env::var("ANTHROPIC_MODEL")
            .ok()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| "claude-sonnet-4-6".to_string());
        let mut s = Self::new(api_key, model);
        if let Ok(url) = std::env::var("ANTHROPIC_BASE_URL") {
            s = s.with_base_url(url);
        }
        Some(s)
    }

    // ─── ContentBlock → Anthropic content part ────────────────────────────────

    fn block_to_anthropic(block: &ContentBlock) -> Option<Value> {
        match block {
            ContentBlock::Text { text } => Some(json!({ "type": "text", "text": text })),
            ContentBlock::Image { source } => match source {
                ImageSource::Base64 { media_type, data } => Some(json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": media_type,
                        "data": data
                    }
                })),
                ImageSource::Url { url } => Some(json!({
                    "type": "image",
                    "source": { "type": "url", "url": url }
                })),
            },
            ContentBlock::Document { source, title } => {
                let src = serde_json::to_value(source).unwrap_or_default();
                let mut obj = json!({ "type": "document", "source": src });
                if let Some(t) = title {
                    obj["title"] = json!(t);
                }
                Some(obj)
            }
            ContentBlock::ToolUse { id, name, input } => Some(json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input
            })),
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                let content_val: Vec<Value> = content
                    .iter()
                    .filter_map(Self::block_to_anthropic)
                    .collect();
                Some(json!({
                    "type": "tool_result",
                    "tool_use_id": tool_use_id,
                    "content": content_val,
                    "is_error": is_error
                }))
            }
            // thinking block 在 assistant 消息中由 Anthropic 生成，发送时透传
            ContentBlock::Reasoning { text, signature } => {
                let mut obj = json!({ "type": "thinking", "thinking": text });
                if let Some(sig) = signature {
                    obj["signature"] = json!(sig);
                }
                Some(obj)
            }
            ContentBlock::Unknown(v) => Some(v.clone()),
        }
    }

    fn content_to_anthropic(content: &MessageContent) -> Value {
        match content {
            MessageContent::Text(s) => json!([{"type": "text", "text": s}]),
            MessageContent::Blocks(blocks) => {
                let parts: Vec<Value> =
                    blocks.iter().filter_map(Self::block_to_anthropic).collect();
                Value::Array(parts)
            }
            MessageContent::Raw(values) => Value::Array(values.clone()),
        }
    }

    /// 将 system prompt 文本按边界标记拆分为缓存块。
    ///
    /// 边界标记 `__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__` 之前的内容标记为可缓存，
    /// 之后的内容不标记缓存（动态内容变化不会破坏前缀缓存）。
    fn split_system_blocks(text: &str) -> Vec<SystemPromptBlock> {
        if text.is_empty() {
            return Vec::new();
        }
        if let Some(idx) = text.find(SYSTEM_PROMPT_DYNAMIC_BOUNDARY) {
            let mut blocks = Vec::new();
            let static_text = text[..idx].trim().to_string();
            let dynamic_text = text[idx + SYSTEM_PROMPT_DYNAMIC_BOUNDARY.len()..]
                .trim()
                .to_string();
            if !static_text.is_empty() {
                blocks.push(SystemPromptBlock {
                    text: static_text,
                    cache_control: true,
                });
            }
            if !dynamic_text.is_empty() {
                blocks.push(SystemPromptBlock {
                    text: dynamic_text,
                    cache_control: false,
                });
            }
            blocks
        } else {
            // 无边界标记 → 单块，不缓存
            vec![SystemPromptBlock {
                text: text.to_string(),
                cache_control: false,
            }]
        }
    }

    /// 将 BaseMessage 列表转为 Anthropic messages 格式
    ///
    /// - System 消息提取到顶层 system 字段
    /// - Tool 消息合并为 user content blocks
    ///
    /// **缓存前缀稳定性**：含边界标记的 System 消息（来自 build_system_prompt）
    /// 排在最前面，不含边界标记的 middleware 注入内容排在边界之后，
    /// 确保 middleware 内容变化不会破坏 Anthropic prompt cache 前缀。
    fn messages_to_anthropic(messages: &[BaseMessage]) -> (Vec<Value>, Vec<SystemPromptBlock>) {
        let mut system_parts_with_boundary: Vec<String> = Vec::new();
        let mut system_parts_no_boundary: Vec<String> = Vec::new();
        let mut result: Vec<Value> = Vec::new();

        for msg in messages {
            match msg {
                BaseMessage::System { content, .. } => {
                    let text = content.text_content();
                    if !text.trim().is_empty() {
                        // 含边界标记的 system prompt 排在前面（可缓存前缀），
                        // middleware 注入的内容排在后面（动态段，不影响缓存）
                        if text.contains(SYSTEM_PROMPT_DYNAMIC_BOUNDARY) {
                            system_parts_with_boundary.push(text);
                        } else {
                            system_parts_no_boundary.push(text);
                        }
                    }
                }
                BaseMessage::Human { content, .. } => {
                    result.push(json!({
                        "role": "user",
                        "content": Self::content_to_anthropic(content)
                    }));
                }
                BaseMessage::Ai {
                    content,
                    tool_calls,
                    ..
                } => {
                    if tool_calls.is_empty() {
                        result.push(json!({
                            "role": "assistant",
                            "content": Self::content_to_anthropic(content)
                        }));
                    } else {
                        // 若 content 已经是 Blocks（含 ToolUse），直接序列化
                        // 否则构造 text + tool_use blocks
                        let content_val = match content {
                            MessageContent::Blocks(_) | MessageContent::Raw(_) => {
                                Self::content_to_anthropic(content)
                            }
                            MessageContent::Text(t) => {
                                let mut blocks: Vec<Value> = Vec::new();
                                if !t.is_empty() {
                                    blocks.push(json!({ "type": "text", "text": t }));
                                }
                                for tc in tool_calls {
                                    blocks.push(json!({
                                        "type": "tool_use",
                                        "id": tc.id,
                                        "name": tc.name,
                                        "input": tc.arguments
                                    }));
                                }
                                Value::Array(blocks)
                            }
                        };
                        result.push(json!({ "role": "assistant", "content": content_val }));
                    }
                }
                BaseMessage::Tool {
                    tool_call_id,
                    content,
                    is_error,
                    ..
                } => {
                    let tool_result_block = json!({
                        "type": "tool_result",
                        "tool_use_id": tool_call_id,
                        "content": Self::content_to_anthropic(content),
                        "is_error": is_error
                    });

                    // Anthropic 要求 tool_result blocks 必须在 user content 数组开头
                    let appended = if let Some(last) = result.last_mut() {
                        if last["role"] == "user" {
                            if let Some(arr) = last["content"].as_array_mut() {
                                arr.insert(0, tool_result_block.clone());
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if !appended {
                        result.push(json!({
                            "role": "user",
                            "content": [tool_result_block]
                        }));
                    }
                }
            }
        }

        // 拼接：含边界的 system prompt 在前，middleware 注入内容追加到边界之后
        let mut system_text = system_parts_with_boundary.join("\n\n");
        if !system_parts_no_boundary.is_empty() {
            let middleware_text = system_parts_no_boundary.join("\n\n");
            if system_text.contains(SYSTEM_PROMPT_DYNAMIC_BOUNDARY) {
                // 在边界标记位置之后插入 middleware 内容
                // split_system_blocks 会在边界处拆分，middleware 内容归入动态段
                system_text = system_text.replacen(
                    SYSTEM_PROMPT_DYNAMIC_BOUNDARY,
                    &format!("{}\n\n{}", SYSTEM_PROMPT_DYNAMIC_BOUNDARY, middleware_text),
                    1,
                );
            } else {
                // 无边界标记：全部作为动态段
                system_text = format!("{system_text}\n\n{middleware_text}");
            }
        }
        let system_blocks = Self::split_system_blocks(&system_text);
        (result, system_blocks)
    }

    /// 对 messages 列表中的 user 消息追加 cache_control 断点
    ///
    /// Anthropic Prompt Caching 要求在需要缓存的边界位置加 `cache_control: { type: "ephemeral" }`。
    /// 最多允许 4 个断点（system 占 1-2 个，messages 中占剩余名额）。
    ///
    /// **缓存策略**（最多 3 断点）：
    /// 1. **第一条 user 消息**：system + 首条 user 构成稳定缓存段，后续轮次不会失效。
    /// 2. **倒数第二条 user 消息**：多轮对话中，上一轮的 user+assistant+tool 整段可被缓存。
    ///    当目标消息仅含 tool_result 无 text block 时，沿 user_indices 向前回退搜索。
    /// 3. **最后一条 user 消息**：当前轮次的完整前缀可被缓存（同一轮内多次工具调用间复用）。
    ///    同样支持回退搜索。
    ///
    /// 当 user 消息不足 3 条时，按实际数量设置断点（不会重复）。
    fn apply_cache_to_messages(messages: &mut [Value]) {
        let user_indices: Vec<usize> = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m["role"] == "user")
            .map(|(i, _)| i)
            .collect();

        if user_indices.is_empty() {
            return;
        }

        // 检查 user 消息是否包含可附加 cache_control 的 text block
        let has_text_block = |msg: &Value| -> bool {
            match msg.get("content") {
                Some(Value::Array(blocks)) => blocks.iter().any(|b| {
                    b["type"].as_str() == Some("text")
                        && !b["text"]
                            .as_str()
                            .map(|t| t.trim().is_empty())
                            .unwrap_or(true)
                }),
                Some(Value::String(s)) => !s.trim().is_empty(),
                _ => false,
            }
        };

        // 确定要加断点的位置：第一条 + 倒数第二条 + 最后一条（去重）
        let mut target_indices: Vec<usize> = Vec::new();
        target_indices.push(user_indices[0]);
        if let Some(&last) = user_indices.last() {
            if last != user_indices[0] {
                target_indices.push(last);
            }
        }
        if user_indices.len() >= 3 {
            let second_to_last = user_indices[user_indices.len() - 2];
            if !target_indices.contains(&second_to_last) {
                if second_to_last < target_indices[0] {
                    target_indices.insert(0, second_to_last);
                } else if second_to_last > target_indices[target_indices.len() - 1] {
                    target_indices.push(second_to_last);
                } else {
                    target_indices.insert(1, second_to_last);
                }
            }
        }

        for idx in &target_indices {
            // 如果目标消息无 text block，沿 user_indices 向前回退搜索
            let effective_idx = if has_text_block(&messages[*idx]) {
                Some(*idx)
            } else {
                user_indices
                    .iter()
                    .rev()
                    .find(|&&ui| {
                        ui < *idx && has_text_block(&messages[ui]) && !target_indices.contains(&ui)
                    })
                    .copied()
            };

            if let Some(ei) = effective_idx {
                let msg = &mut messages[ei];
                if let Some(content) = msg.get_mut("content") {
                    match content {
                        Value::Array(blocks) => {
                            let target = blocks.iter_mut().rfind(|b| {
                                let btype = b["type"].as_str().unwrap_or("");
                                btype == "text"
                                    && !b["text"]
                                        .as_str()
                                        .map(|t| t.trim().is_empty())
                                        .unwrap_or(true)
                            });
                            if let Some(block) = target {
                                block["cache_control"] = json!({ "type": "ephemeral" });
                            }
                        }
                        Value::String(s) if !s.trim().is_empty() => {
                            let text = s.clone();
                            *content = json!([{
                                "type": "text",
                                "text": text,
                                "cache_control": { "type": "ephemeral" }
                            }]);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// 为不含 thinking 的 assistant 消息注入占位 thinking block。
    ///
    /// DeepSeek Anthropic 兼容端口在 thinking 模式下要求所有 assistant 消息都包含 thinking block。
    /// 中间件（如 SkillPreloadMiddleware）注入的伪 assistant 消息不含 thinking，会导致 400 错误。
    /// 注入带占位文本和空 signature 的 thinking block 以通过 API 验证。
    fn ensure_thinking_blocks(messages: &mut [Value]) {
        for msg in messages.iter_mut() {
            if msg["role"] != "assistant" {
                continue;
            }
            let has_thinking = match msg.get("content") {
                Some(Value::Array(blocks)) => blocks.iter().any(|b| {
                    let btype = b["type"].as_str().unwrap_or("");
                    btype == "thinking" || btype == "redacted_thinking"
                }),
                _ => false,
            };
            if !has_thinking {
                let placeholder = json!({
                    "type": "thinking",
                    "thinking": "",
                    "signature": ""
                });
                match msg.get_mut("content") {
                    Some(Value::Array(blocks)) => {
                        blocks.insert(0, placeholder);
                    }
                    Some(content) => {
                        let old = content.clone();
                        *content = Value::Array(vec![placeholder, old]);
                    }
                    None => {
                        msg["content"] = Value::Array(vec![placeholder]);
                    }
                }
            }
        }
    }

    // ─── 响应 content blocks → BaseMessage ───────────────────────────────────

    fn parse_content_blocks(raw_blocks: &[Value]) -> (Vec<ContentBlock>, Vec<ToolCallRequest>) {
        let mut blocks: Vec<ContentBlock> = Vec::new();
        let mut tool_calls: Vec<ToolCallRequest> = Vec::new();

        for b in raw_blocks {
            match b["type"].as_str() {
                Some("text") => {
                    if let Some(text) = b["text"].as_str() {
                        blocks.push(ContentBlock::text(text));
                    }
                }
                Some("thinking") => {
                    let text = b["thinking"].as_str().unwrap_or("").to_string();
                    let signature = b["signature"].as_str().map(|s| s.to_string());
                    if let Some(sig) = signature {
                        blocks.push(ContentBlock::reasoning_with_signature(text, sig));
                    } else {
                        blocks.push(ContentBlock::reasoning(text));
                    }
                }
                Some("tool_use") => {
                    if let (Some(id), Some(name)) = (b["id"].as_str(), b["name"].as_str()) {
                        let input = b["input"].clone();
                        blocks.push(ContentBlock::tool_use(id, name, input.clone()));
                        tool_calls.push(ToolCallRequest::new(id, name, input));
                    }
                }
                // Anthropic extended thinking 可能返回 redacted_thinking block，
                // 必须保留原始数据以便在后续请求中回传，否则 API 会拒绝
                Some("redacted_thinking") => {
                    blocks.push(ContentBlock::Unknown(b.clone()));
                }
                _ => {
                    blocks.push(ContentBlock::Unknown(b.clone()));
                }
            }
        }

        (blocks, tool_calls)
    }
}

#[async_trait]
impl BaseModel for ChatAnthropic {
    async fn invoke(&self, request: LlmRequest) -> AgentResult<LlmResponse> {
        let msg_count = request.messages.len();
        let start = std::time::Instant::now();

        let chat_url = match &self.base_url {
            Some(base) => format!("{}/v1/messages", base.trim_end_matches('/')),
            None => "https://api.anthropic.com/v1/messages".to_string(),
        };

        let tools_json: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters
                })
            })
            .collect();

        // tools 不再单独设 cache_control 断点。
        // tools 已被 msg[first] 断点的缓存前缀覆盖（system + tools + first user），
        // 省下的断点名额用于 system[last]，使整个 system prompt 可缓存。

        let (mut messages, system_from_msgs) = Self::messages_to_anthropic(&request.messages);

        // Extended Thinking 模式下，为缺少 thinking 的 assistant 消息注入 redacted_thinking 占位。
        // DeepSeek Anthropic 兼容端口要求所有 assistant 消息都包含 thinking block，
        // 但中间件（如 SkillPreloadMiddleware）注入的伪 assistant 消息不含 thinking。
        // Anthropic 原生 API 不要求凭空注入（已有 thinking 通过 Reasoning 序列化回传），
        // 但兼容端口需要 redacted_thinking 占位以通过 API 验证。
        if self.extended_thinking {
            Self::ensure_thinking_blocks(&mut messages);
        }

        // 合并 system blocks：消息列表中的 System（中间件注入）+ request.system
        let mut system_blocks = system_from_msgs;
        if let Some(ref base) = request.system {
            if !base.is_empty() {
                system_blocks.push(SystemPromptBlock {
                    text: base.clone(),
                    cache_control: false,
                });
            }
        }
        let max_tokens = request.max_tokens.unwrap_or(4096);

        // 开启缓存时：对最后一条消息的最后一个 block 加 cache_control
        if self.enable_cache {
            Self::apply_cache_to_messages(&mut messages);
        }

        let mut body = json!({
            "model": self.model,
            "max_tokens": max_tokens,
            "messages": messages
        });

        if self.enable_cache {
            // system 多块格式：静态块 + 最后一块标记 cache_control
            // 静态块（split_system_blocks 产出）已有 cache_control=true，
            // 最后一块（通常含 middleware 内容）额外标记以缓存整个 system prompt。
            if !system_blocks.is_empty() {
                let last_idx = system_blocks.len() - 1;
                let blocks_json: Vec<Value> = system_blocks
                    .iter()
                    .enumerate()
                    .map(|(i, b)| {
                        let mut block = json!({"type": "text", "text": &b.text});
                        if b.cache_control || i == last_idx {
                            block["cache_control"] = json!({"type": "ephemeral"});
                        }
                        block
                    })
                    .collect();
                body["system"] = Value::Array(blocks_json);
            }
        } else if !system_blocks.is_empty() {
            // 不启用缓存：合并为单个字符串，移除边界标记
            let text = system_blocks
                .iter()
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n")
                .replace(SYSTEM_PROMPT_DYNAMIC_BOUNDARY, "");
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                body["system"] = json!(trimmed);
            }
        }

        if !tools_json.is_empty() {
            body["tools"] = Value::Array(tools_json);
        }

        if let Some(temperature) = request.temperature {
            body["temperature"] = json!(temperature);
        }

        // Extended Thinking 配置
        if self.extended_thinking {
            body["thinking"] = json!({
                "type": "enabled",
                "budget_tokens": self.thinking_budget
            });
            body["output_config"] = json!({ "effort": self.thinking_effort });
        }

        let mut req = self
            .client
            .post(chat_url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json");

        // Prompt Caching 需要 beta header
        if self.enable_cache {
            req = req.header("anthropic-beta", "prompt-caching-2024-07-31");
        }

        // LiteLLM session tracking：通过 header 按 session 聚合多次请求
        if let Some(ref sid) = request.session_id {
            req = req.header("x-session-id", sid.as_str());
        }

        tracing::debug!(
            provider = "anthropic",
            model = %self.model,
            messages_count = body["messages"].as_array().map(|a| a.len()).unwrap_or(0),
            "LLM 请求发送"
        );

        let resp = req.json(&body).send().await.map_err(|e| {
            tracing::error!(
                provider = "anthropic",
                model = %self.model,
                elapsed_ms = start.elapsed().as_millis() as u64,
                error = %e,
                "LLM 网络请求失败"
            );
            AgentError::LlmError(e.to_string())
        })?;

        let status = resp.status();
        // 先保存 header 中的 request_id，解析 body 后尝试用 body id 兜底
        let header_request_id = resp
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let resp_text = resp.text().await.map_err(|e| {
            tracing::error!(
                provider = "anthropic",
                model = %self.model,
                status = %status,
                elapsed_ms = start.elapsed().as_millis() as u64,
                error = %e,
                "LLM 读取响应体失败"
            );
            AgentError::LlmError(format!("读取响应体失败: {e}"))
        })?;
        let resp_json: Value = serde_json::from_str(&resp_text).map_err(|e| {
            tracing::error!(
                provider = "anthropic",
                model = %self.model,
                status = %status,
                elapsed_ms = start.elapsed().as_millis() as u64,
                error = %e,
                "LLM 响应解析失败"
            );
            AgentError::LlmError(format!(
                "解析响应失败: {e}\n原始响应({status}): {resp_text}"
            ))
        })?;

        // request_id 优先用 x-request-id header，无则回退到 body 中的 id 字段
        let request_id =
            header_request_id.or_else(|| resp_json["id"].as_str().map(|s| s.to_string()));

        if !status.is_success() {
            let msg = resp_json["error"]["message"]
                .as_str()
                .unwrap_or("未知错误")
                .to_string();
            let error_type = resp_json["error"]["type"].as_str().unwrap_or("unknown");

            // 500 错误记录完整请求体以便排查服务端 bug
            if status.as_u16() == 500 {
                tracing::error!(
                    provider = "anthropic",
                    model = %self.model,
                    status = %status,
                    error_type,
                    error_message = %msg,
                    elapsed_ms = start.elapsed().as_millis() as u64,
                    request_messages = %serde_json::to_string(&body["messages"]).unwrap_or_else(|_| "serialize failed".into()),
                    "LLM API 500 错误（服务端 bug），已记录请求体"
                );
            } else {
                tracing::error!(
                    provider = "anthropic",
                    model = %self.model,
                    status = %status,
                    error_type,
                    error_message = %msg,
                    elapsed_ms = start.elapsed().as_millis() as u64,
                    msg_count,
                    "LLM API 错误"
                );
            }
            return Err(AgentError::LlmHttpError {
                status: status.as_u16(),
                message: format!("API 错误 {status}: {msg}"),
            });
        }

        tracing::info!(
            provider = "anthropic",
            model = %self.model,
            status = %status,
            elapsed_ms = start.elapsed().as_millis() as u64,
            msg_count,
            input_tokens = resp_json["usage"]["input_tokens"].as_u64().unwrap_or(0),
            output_tokens = resp_json["usage"]["output_tokens"].as_u64().unwrap_or(0),
            cache_read = resp_json["usage"]["cache_read_input_tokens"].as_u64().unwrap_or(0),
            cache_creation = resp_json["usage"]["cache_creation_input_tokens"].as_u64().unwrap_or(0),
            "LLM invoke completed"
        );

        let stop_reason =
            StopReason::from_anthropic(resp_json["stop_reason"].as_str().unwrap_or("end_turn"));

        let raw_blocks = resp_json["content"]
            .as_array()
            .ok_or_else(|| AgentError::LlmError("响应缺少 content 字段".to_string()))?;

        let (blocks, tool_calls) = Self::parse_content_blocks(raw_blocks);

        // 决定 content 形式
        // - 只有单个纯文本且无工具调用 → 简单 Text（向后兼容）
        // - 含 thinking / tool_use / 多 block → Blocks
        let message = if !tool_calls.is_empty() {
            let content = if let [single] = blocks.as_slice() {
                if let Some(text) = single.as_text() {
                    MessageContent::text(text)
                } else {
                    MessageContent::Blocks(blocks)
                }
            } else {
                MessageContent::Blocks(blocks)
            };
            BaseMessage::ai_with_tool_calls(content, tool_calls)
        } else if let [single] = blocks.as_slice() {
            if let Some(text) = single.as_text() {
                BaseMessage::ai(text)
            } else {
                BaseMessage::ai(MessageContent::Blocks(blocks))
            }
        } else if blocks.is_empty() {
            BaseMessage::ai("")
        } else {
            // 含 thinking block 或多 block
            BaseMessage::ai(MessageContent::Blocks(blocks))
        };

        let usage = {
            let raw_input = resp_json["usage"]["input_tokens"]
                .as_u64()
                .map(|v| v as u32)
                .unwrap_or(0);
            let output = resp_json["usage"]["output_tokens"]
                .as_u64()
                .map(|v| v as u32);
            // Anthropic API 缓存字段始终存在，但值可能为 null（无缓存活动时）。
            // null 等价于 0，用 unwrap_or(0) 统一处理。
            let cache_creation = resp_json["usage"]["cache_creation_input_tokens"]
                .as_u64()
                .map(|v| v as u32)
                .unwrap_or(0);
            let cache_read = resp_json["usage"]["cache_read_input_tokens"]
                .as_u64()
                .map(|v| v as u32)
                .unwrap_or(0);
            match (resp_json["usage"]["input_tokens"].as_u64(), output) {
                (Some(_), Some(o)) => Some(crate::llm::types::TokenUsage {
                    // 规范化：Anthropic 的 input_tokens 不含缓存 token，
                    // 加上 cache_creation + cache_read 使其与 OpenAI 语义一致（总输入）。
                    // 这样 estimated_context_tokens() 和 cache_hit_rate() 可用单一公式。
                    input_tokens: raw_input + cache_creation + cache_read,
                    output_tokens: o,
                    cache_creation_input_tokens: Some(cache_creation),
                    cache_read_input_tokens: Some(cache_read),
                    request_id: request_id.clone(),
                }),
                _ => None,
            }
        };
        Ok(LlmResponse {
            message,
            stop_reason,
            usage,
            request_id,
        })
    }

    fn provider_name(&self) -> &str {
        "anthropic"
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    fn context_window(&self) -> u32 {
        200_000
    }

    async fn invoke_streaming(
        &self,
        request: LlmRequest,
        ctx: StreamingContext,
    ) -> AgentResult<LlmResponse> {
        let msg_count = request.messages.len();
        let start = std::time::Instant::now();

        let chat_url = match &self.base_url {
            Some(base) => format!("{}/v1/messages", base.trim_end_matches('/')),
            None => "https://api.anthropic.com/v1/messages".to_string(),
        };

        let tools_json: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters
                })
            })
            .collect();

        let (mut messages, system_from_msgs) = Self::messages_to_anthropic(&request.messages);

        if self.extended_thinking {
            Self::ensure_thinking_blocks(&mut messages);
        }

        let mut system_blocks = system_from_msgs;
        if let Some(ref base) = request.system {
            if !base.is_empty() {
                system_blocks.push(SystemPromptBlock {
                    text: base.clone(),
                    cache_control: false,
                });
            }
        }
        let max_tokens = request.max_tokens.unwrap_or(4096);

        if self.enable_cache {
            Self::apply_cache_to_messages(&mut messages);
        }

        let mut body = json!({
            "model": self.model,
            "max_tokens": max_tokens,
            "messages": messages,
            "stream": true
        });

        if self.enable_cache {
            if !system_blocks.is_empty() {
                let last_idx = system_blocks.len() - 1;
                let blocks_json: Vec<Value> = system_blocks
                    .iter()
                    .enumerate()
                    .map(|(i, b)| {
                        let mut block = json!({"type": "text", "text": &b.text});
                        if b.cache_control || i == last_idx {
                            block["cache_control"] = json!({"type": "ephemeral"});
                        }
                        block
                    })
                    .collect();
                body["system"] = Value::Array(blocks_json);
            }
        } else if !system_blocks.is_empty() {
            let text = system_blocks
                .iter()
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n")
                .replace(SYSTEM_PROMPT_DYNAMIC_BOUNDARY, "");
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                body["system"] = json!(trimmed);
            }
        }

        if !tools_json.is_empty() {
            body["tools"] = Value::Array(tools_json);
        }

        if let Some(temperature) = request.temperature {
            body["temperature"] = json!(temperature);
        }

        if self.extended_thinking {
            body["thinking"] = json!({
                "type": "enabled",
                "budget_tokens": self.thinking_budget
            });
            body["output_config"] = json!({ "effort": self.thinking_effort });
        }

        let mut req = self
            .client
            .post(chat_url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json");

        if self.enable_cache {
            req = req.header("anthropic-beta", "prompt-caching-2024-07-31");
        }

        if let Some(ref sid) = request.session_id {
            req = req.header("x-session-id", sid.as_str());
        }

        let resp = req.json(&body).send().await.map_err(|e| {
            tracing::error!(
                provider = "anthropic", model = %self.model,
                elapsed_ms = start.elapsed().as_millis() as u64, error = %e,
                "LLM 流式网络请求失败"
            );
            AgentError::LlmError(e.to_string())
        })?;

        let status = resp.status();
        if !status.is_success() {
            let resp_text = resp.text().await.unwrap_or_default();
            let error_msg = serde_json::from_str::<Value>(&resp_text)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "未知错误".to_string());
            tracing::error!(
                provider = "anthropic", model = %self.model, status = %status,
                error_message = %error_msg,
                elapsed_ms = start.elapsed().as_millis() as u64, msg_count,
                "LLM 流式 API 错误"
            );
            return Err(AgentError::LlmHttpError {
                status: status.as_u16(),
                message: format!("API 错误 {status}: {error_msg}"),
            });
        }

        let mut stream = resp.bytes_stream();
        let mut parser = SseParser::new();

        // Accumulators
        let mut text_content = String::new();
        let mut reasoning_content = String::new();
        let mut thinking_signature: Option<String> = None;
        let mut tool_use_id: Option<String> = None;
        let mut tool_use_name: Option<String> = None;
        let mut tool_input_fragments: String = String::new();
        let mut accumulated_blocks: Vec<Value> = Vec::new(); // for final parse
        let mut current_block_type: Option<String> = None;

        let mut input_tokens: u32 = 0;
        let mut cache_creation_input_tokens: u32 = 0;
        let mut cache_read_input_tokens: u32 = 0;
        let mut output_tokens: u32 = 0;
        let mut stop_reason_str: String = "end_turn".to_string();
        let mut stream_request_id: Option<String> = None;

        while let Some(chunk_result) = stream.next().await {
            let chunk =
                chunk_result.map_err(|e| AgentError::LlmError(format!("流式读取失败: {e}")))?;

            for (event_type, data) in parser.push(&chunk) {
                let event = event_type.as_deref().unwrap_or("");
                let parsed: Value = match serde_json::from_str(&data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                match event {
                    "message_start" => {
                        stream_request_id = parsed["message"]["id"].as_str().map(|s| s.to_string());
                        // 先尝试标准路径 message.usage（Anthropic 规范），
                        // 部分代理把 usage 放在顶层
                        let usage_obj = if parsed["message"]["usage"].is_object() {
                            &parsed["message"]["usage"]
                        } else if parsed["usage"].is_object() {
                            &parsed["usage"]
                        } else {
                            &serde_json::Value::Null
                        };
                        input_tokens = usage_obj["input_tokens"].as_u64().unwrap_or(0) as u32;
                        cache_creation_input_tokens = usage_obj["cache_creation_input_tokens"]
                            .as_u64()
                            .unwrap_or(0)
                            as u32;
                        cache_read_input_tokens =
                            usage_obj["cache_read_input_tokens"].as_u64().unwrap_or(0) as u32;
                    }
                    "content_block_start" => {
                        let cb = &parsed["content_block"];
                        let cb_type = cb["type"].as_str().unwrap_or("");
                        current_block_type = Some(cb_type.to_string());

                        match cb_type {
                            "thinking" => {
                                // Signature is in content_block_start (NOT content_block_stop)
                                if let Some(sig) = cb["signature"].as_str() {
                                    thinking_signature = Some(sig.to_string());
                                }
                            }
                            "tool_use" => {
                                tool_use_id = cb["id"].as_str().map(|s| s.to_string());
                                tool_use_name = cb["name"].as_str().map(|s| s.to_string());
                                tool_input_fragments.clear();
                            }
                            _ => {}
                        }
                    }
                    "content_block_delta" => {
                        let delta = &parsed["delta"];
                        match delta["type"].as_str().unwrap_or("") {
                            "thinking_delta" => {
                                if let Some(t) = delta["thinking"].as_str() {
                                    if !t.is_empty() {
                                        ctx.event_handler
                                            .on_event(AgentEvent::AiReasoning(t.to_string()));
                                        reasoning_content.push_str(t);
                                    }
                                }
                            }
                            "text_delta" => {
                                if let Some(t) = delta["text"].as_str() {
                                    if !t.is_empty() {
                                        ctx.event_handler.on_event(AgentEvent::TextChunk {
                                            message_id: ctx.message_id,
                                            chunk: t.to_string(),
                                        });
                                        text_content.push_str(t);
                                    }
                                }
                            }
                            "input_json_delta" => {
                                if let Some(json_part) = delta["partial_json"].as_str() {
                                    tool_input_fragments.push_str(json_part);
                                }
                            }
                            _ => {}
                        }
                    }
                    "content_block_stop" => {
                        // Finalize current block
                        match current_block_type.as_deref() {
                            Some("thinking") => {
                                let mut block = json!({
                                    "type": "thinking",
                                    "thinking": &reasoning_content
                                });
                                if let Some(ref sig) = thinking_signature {
                                    block["signature"] = json!(sig);
                                }
                                accumulated_blocks.push(block);
                            }
                            Some("text") => {
                                accumulated_blocks.push(json!({
                                    "type": "text",
                                    "text": &text_content
                                }));
                            }
                            Some("tool_use") => {
                                let input: Value = serde_json::from_str(&tool_input_fragments)
                                    .unwrap_or(Value::Null);
                                accumulated_blocks.push(json!({
                                    "type": "tool_use",
                                    "id": tool_use_id,
                                    "name": tool_use_name,
                                    "input": input
                                }));
                            }
                            _ => {}
                        }
                        current_block_type = None;
                    }
                    "message_delta" => {
                        stop_reason_str = parsed["delta"]["stop_reason"]
                            .as_str()
                            .unwrap_or("end_turn")
                            .to_string();
                        output_tokens =
                            parsed["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;
                        // 部分代理不发送 message_start 事件，将 input_tokens 放在此处
                        if input_tokens == 0 {
                            input_tokens =
                                parsed["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
                            cache_creation_input_tokens =
                                parsed["usage"]["cache_creation_input_tokens"]
                                    .as_u64()
                                    .unwrap_or(0) as u32;
                            cache_read_input_tokens = parsed["usage"]["cache_read_input_tokens"]
                                .as_u64()
                                .unwrap_or(0)
                                as u32;
                        }
                    }
                    "message_stop" if input_tokens == 0 => {
                        // stream naturally ends
                        // 最后兜底：部分代理仅在 message_stop 返回 input_tokens
                        input_tokens = parsed["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
                        cache_creation_input_tokens = parsed["usage"]["cache_creation_input_tokens"]
                            .as_u64()
                            .unwrap_or(0)
                            as u32;
                        cache_read_input_tokens = parsed["usage"]["cache_read_input_tokens"]
                            .as_u64()
                            .unwrap_or(0) as u32;
                    }
                    _ => {}
                }
            }

            if parser.is_done() {
                break;
            }
        }

        // Build final response using existing parse_content_blocks
        let stop_reason = StopReason::from_anthropic(&stop_reason_str);

        let (blocks, tool_calls) = Self::parse_content_blocks(&accumulated_blocks);

        let message = if !tool_calls.is_empty() {
            let content = if let [single] = blocks.as_slice() {
                if let Some(text) = single.as_text() {
                    MessageContent::text(text)
                } else {
                    MessageContent::Blocks(blocks)
                }
            } else {
                MessageContent::Blocks(blocks)
            };
            BaseMessage::ai_with_tool_calls(content, tool_calls)
        } else if let [single] = blocks.as_slice() {
            if let Some(text) = single.as_text() {
                BaseMessage::ai(text)
            } else {
                BaseMessage::ai(MessageContent::Blocks(blocks))
            }
        } else if blocks.is_empty() {
            BaseMessage::ai("")
        } else {
            BaseMessage::ai(MessageContent::Blocks(blocks))
        };

        // 规范化 input_tokens：Anthropic 的 input_tokens 不含缓存 token，
        // 加上 cache_creation + cache_read 使其与 OpenAI 语义一致（总输入）。
        // 这样 estimated_context_tokens() 和 cache_hit_rate() 可用单一公式。
        let normalized_input = input_tokens + cache_creation_input_tokens + cache_read_input_tokens;

        let usage = Some(crate::llm::types::TokenUsage {
            input_tokens: normalized_input,
            output_tokens,
            cache_creation_input_tokens: Some(cache_creation_input_tokens),
            cache_read_input_tokens: Some(cache_read_input_tokens),
            request_id: stream_request_id.clone(),
        });

        tracing::info!(
            provider = "anthropic",
            model = %self.model,
            elapsed_ms = start.elapsed().as_millis() as u64,
            msg_count,
            input_tokens = normalized_input,
            output_tokens,
            cache_read = cache_read_input_tokens,
            cache_creation = cache_creation_input_tokens,
            "LLM streaming completed"
        );

        Ok(LlmResponse {
            message,
            stop_reason,
            usage,
            request_id: stream_request_id,
        })
    }
}

#[async_trait]
impl ReactLLM for ChatAnthropic {
    async fn generate_reasoning(
        &self,
        messages: &[BaseMessage],
        tools: &[&dyn BaseTool],
        _streaming: Option<StreamingContext>,
    ) -> AgentResult<Reasoning> {
        let tool_defs = tools.iter().map(|t| t.definition()).collect();
        let request = LlmRequest::new(messages.to_vec()).with_tools(tool_defs);

        // system 消息由 messages_to_anthropic 从消息列表提取，无需单独处理

        let response = self.invoke(request).await?;
        let usage = response.usage.clone();
        let model_name = self.model.clone();

        if response.stop_reason == StopReason::ToolUse {
            let blocks = response.message.content_blocks();
            let thought = blocks
                .iter()
                .filter_map(|b| b.as_text())
                .collect::<Vec<_>>()
                .join("");

            let calls: Vec<ToolCall> = blocks
                .iter()
                .filter_map(|b| {
                    if let ContentBlock::ToolUse { id, name, input } = b {
                        Some(ToolCall::new(id.clone(), name.clone(), input.clone()))
                    } else {
                        None
                    }
                })
                .collect();

            if !calls.is_empty() {
                let mut r = Reasoning::with_tools(thought, calls);
                r.source_message = Some(response.message);
                r.usage = usage;
                r.model = model_name;
                return Ok(r);
            }

            let calls: Vec<ToolCall> = response
                .message
                .tool_calls()
                .iter()
                .map(|tc| ToolCall::new(tc.id.clone(), tc.name.clone(), tc.arguments.clone()))
                .collect();
            let mut r = Reasoning::with_tools(thought, calls);
            r.source_message = Some(response.message);
            r.usage = usage;
            r.model = model_name;
            Ok(r)
        } else {
            let text = response.message.content();
            let mut r = Reasoning::with_answer("", text);
            r.source_message = Some(response.message);
            r.usage = usage;
            r.model = model_name;
            Ok(r)
        }
    }

    fn model_name(&self) -> String {
        self.model.clone()
    }
}

#[cfg(test)]
#[path = "../anthropic_test.rs"]
mod tests;
