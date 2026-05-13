# 问题索引

按关键词索引已归档 issue，遇到相似问题时快速定位历史经验。

## 关键词索引

### HashMap 顺序
- [多处 HashMap 非确定性顺序导致 Anthropic Prompt Cache 前缀不稳定](domains/message-pipeline.md#issue_2026-05-12-deferred-tool-list-nondeterministic-order) — message-pipeline

### Prompt Cache
- [多处 HashMap 非确定性顺序导致 Anthropic Prompt Cache 前缀不稳定](domains/message-pipeline.md#issue_2026-05-12-deferred-tool-list-nondeterministic-order) — message-pipeline
- [Skill Preload 注入消息到历史最前面导致首轮 Prompt Cache 失效](domains/message-pipeline.md#issue_2026-05-12-skill-preload-invalidates-prompt-cache) — message-pipeline

### 缓存前缀
- [多处 HashMap 非确定性顺序导致 Anthropic Prompt Cache 前缀不稳定](domains/message-pipeline.md#issue_2026-05-12-deferred-tool-list-nondeterministic-order) — message-pipeline

### ToolSearchIndex
- [多处 HashMap 非确定性顺序导致 Anthropic Prompt Cache 前缀不稳定](domains/message-pipeline.md#issue_2026-05-12-deferred-tool-list-nondeterministic-order) — message-pipeline

### prepend_message
- [Skill Preload 注入消息到历史最前面导致首轮 Prompt Cache 失效](domains/message-pipeline.md#issue_2026-05-12-skill-preload-invalidates-prompt-cache) — message-pipeline

### add_message
- [Skill Preload 注入消息到历史最前面导致首轮 Prompt Cache 失效](domains/message-pipeline.md#issue_2026-05-12-skill-preload-invalidates-prompt-cache) — message-pipeline

### cache_control
- [Skill Preload 注入消息到历史最前面导致首轮 Prompt Cache 失效](domains/message-pipeline.md#issue_2026-05-12-skill-preload-invalidates-prompt-cache) — message-pipeline

### SystemNote
- [SystemNote 在 RebuildAll 后堆积到消息列表末尾](domains/message-pipeline.md#issue_2026-05-12-systemnote-position-drift-on-rebuild) — message-pipeline

### RebuildAll
- [SystemNote 在 RebuildAll 后堆积到消息列表末尾](domains/message-pipeline.md#issue_2026-05-12-systemnote-position-drift-on-rebuild) — message-pipeline
- [CacheWarning 消息被 RebuildAll 立即丢弃，用户无法看到](domains/message-pipeline.md#issue_2026-05-12-cache-warning-discarded-by-rebuild) — message-pipeline
- [Compact 完成后残留 "正在压缩上下文…" 系统通知](domains/message-pipeline.md#issue_2026-05-12-compact-ephemeral-notes-not-cleared) — message-pipeline

### ephemeral_notes
- [SystemNote 在 RebuildAll 后堆积到消息列表末尾](domains/message-pipeline.md#issue_2026-05-12-systemnote-position-drift-on-rebuild) — message-pipeline
- [Compact 完成后残留 "正在压缩上下文…" 系统通知](domains/message-pipeline.md#issue_2026-05-12-compact-ephemeral-notes-not-cleared) — message-pipeline

### 锚点机制
- [SystemNote 在 RebuildAll 后堆积到消息列表末尾](domains/message-pipeline.md#issue_2026-05-12-systemnote-position-drift-on-rebuild) — message-pipeline

### CacheWarning
- [CacheWarning 消息被 RebuildAll 立即丢弃，用户无法看到](domains/message-pipeline.md#issue_2026-05-12-cache-warning-discarded-by-rebuild) — message-pipeline

### AiReasoning
- [流式过程中 AI 文本不可见（工具调用场景）](domains/agent.md#issue_2026-05-11-streaming-text-invisible-with-tools) — agent

### TextChunk
- [流式过程中 AI 文本不可见（工具调用场景）](domains/agent.md#issue_2026-05-11-streaming-text-invisible-with-tools) — agent

### 事件类型语义
- [流式过程中 AI 文本不可见（工具调用场景）](domains/agent.md#issue_2026-05-11-streaming-text-invisible-with-tools) — agent

### frozen_subagent_vms
- [Background Agent 三个 Bug：显示消失、subagent_type 限制、continuation 不触发](domains/agent.md#issue_2026-05-12-background-agent-display-and-continuation-bugs) — agent

### continuation 竞态
- [Background Agent 三个 Bug：显示消失、subagent_type 限制、continuation 不触发](domains/agent.md#issue_2026-05-12-background-agent-display-and-continuation-bugs) — agent

### fork+background
- [Background Agent 三个 Bug：显示消失、subagent_type 限制、continuation 不触发](domains/agent.md#issue_2026-05-12-background-agent-display-and-continuation-bugs) — agent

### SubAgent
- [Background Agent 工具继承缺失——子 agent 仅能使用 TodoWrite](domains/agent.md#issue_2026-05-11-background-agent-missing-tools) — agent

### 工具继承
- [Background Agent 工具继承缺失——子 agent 仅能使用 TodoWrite](domains/agent.md#issue_2026-05-11-background-agent-missing-tools) — agent

### register_tool
- [Background Agent 工具继承缺失——子 agent 仅能使用 TodoWrite](domains/agent.md#issue_2026-05-11-background-agent-missing-tools) — agent

### reasoning
- [GLM 模型 reasoning 字段未被解析，thinking 内容跨轮次丢失](domains/agent.md#issue_2026-05-12-glm-reasoning-field-not-parsed) — agent

### reasoning_content
- [GLM 模型 reasoning 字段未被解析，thinking 内容跨轮次丢失](domains/agent.md#issue_2026-05-12-glm-reasoning-field-not-parsed) — agent

### GLM
- [GLM 模型 reasoning 字段未被解析，thinking 内容跨轮次丢失](domains/agent.md#issue_2026-05-12-glm-reasoning-field-not-parsed) — agent

### context_window
- [OpenAI 兼容第三方 Provider 上下文用量计算不准确](domains/token-tracking.md#issue_2026-05-11-context-usage-miscalculation-openai-compatible) — token-tracking

### 缓存命中率
- [OpenAI 兼容第三方 Provider 上下文用量计算不准确](domains/token-tracking.md#issue_2026-05-11-context-usage-miscalculation-openai-compatible) — token-tracking
- [状态栏缓存百分比在对话停止后消失](domains/token-tracking.md#issue_2026-05-12-cache-percentage-disappears-after-done) — token-tracking

### last_user_input
- [Auto Compact 后 Agent 未自动 Resubmit 继续执行](domains/compact.md#issue_2026-05-11-auto-compact-no-resubmit) — compact

### auto-compact
- [Auto Compact 后 Agent 未自动 Resubmit 继续执行](domains/compact.md#issue_2026-05-11-auto-compact-no-resubmit) — compact

### CJK 宽度
- [输入框鼠标点击光标定位不准](domains/tui.md#issue_2026-05-12-textarea-mouse-click-cursor-misposition-cjk) — tui

### unicode-width
- [输入框鼠标点击光标定位不准](domains/tui.md#issue_2026-05-12-textarea-mouse-click-cursor-misposition-cjk) — tui

### 鼠标定位
- [输入框鼠标点击光标定位不准](domains/tui.md#issue_2026-05-12-textarea-mouse-click-cursor-misposition-cjk) — tui

### on_error 回调
- [LSP transport 层错误处理缺陷（进程退出未更新状态 + 崩溃后无自动重连）](domains/lsp.md#issue_2026-05-12-lsp-transport-no-fast-fail-on-process-exit) — lsp

### LSP 重连
- [LSP transport 层错误处理缺陷（进程退出未更新状态 + 崩溃后无自动重连）](domains/lsp.md#issue_2026-05-12-lsp-transport-no-fast-fail-on-process-exit) — lsp

### parking_lot::MutexGuard !Send
- [LSP transport 层错误处理缺陷（进程退出未更新状态 + 崩溃后无自动重连）](domains/lsp.md#issue_2026-05-12-lsp-transport-no-fast-fail-on-process-exit) — lsp

### transport 断开
- [LSP transport 层错误处理缺陷（进程退出未更新状态 + 崩溃后无自动重连）](domains/lsp.md#issue_2026-05-12-lsp-transport-no-fast-fail-on-process-exit) — lsp

## 更新记录

- 2026-05-13: 首次创建，归档 22 个 issue，提取 14 条领域认知
