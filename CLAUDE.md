# CLAUDE.md

## 项目概述

Rust Agent 框架，6 个 Workspace Crate + 1 个独立 Node.js CLI（`peri-cli/`）。

| Crate | 职责 |
|-------|------|
| `rust-create-agent` | 核心：ReAct 循环、Middleware trait、LLM 适配器、工具系统、持久化（SQLite）、遥测 |
| `rust-agent-middlewares` | 中间件：文件系统、终端、HITL、SubAgent、Skills、Todo、Cron、MCP、Hooks、Plugin、LSP |
| `perihelion-widgets` | Widget 组件库（11 组件），仅依赖 ratatui + pulldown-cmark |
| `rust-agent-tui` | TUI 应用，依赖 widgets + middlewares |
| `langfuse-client` | Langfuse 遥测客户端（独立） |
| `perihelion-lsp` | LSP 客户端库（独立，被 middlewares 使用） |

`rmcp` crate（v1.6.0）通过 `[patch.crates-io]` 指向本地 `rust-mcp-patch/`，上游修复后删除补丁目录即可。

## 依赖关系

```
rust-create-agent → rust-agent-middlewares → rust-agent-tui
                   ↗ perihelion-lsp       ↗ perihelion-widgets
langfuse-client（独立）  peri-cli（独立，Node.js）
```

## 开发命令

```bash
cargo build                          # 构建所有 crate
cargo build -p <crate>               # 构建指定 crate
cargo run -p rust-agent-tui          # 运行 TUI
cargo run -p rust-agent-tui -- -a    # HITL 审批模式
cargo test                           # 全量测试
cargo test -p <crate> --lib -- <test_name>  # 单个测试
lefthook install                     # 安装 git hooks
lefthook run pre-commit              # pre-commit（fmt/check/clippy）
```

## 架构要点

**ReAct 循环**（`rust-create-agent`）：AgentInput → collect_tools → before_agent → loop(50) { LLM → [工具调用] before_tool → 并发执行 → after_tool → emit | [回答] → emit TextChunk + StateSnapshot → after_agent }。

**消息类型**：`BaseMessage`（Human/Ai/System/Tool），`ContentBlock`（Text/Image/Document/ToolUse/ToolResult/Reasoning/Unknown）。

**LLM 适配层**：`BaseModel` trait（OpenAI/Anthropic）→ `BaseModelReactLLM` → `ReactLLM`。`RetryableLLM<L>` 指数退避重试。

**TUI 消息渲染**（`rust-agent-tui`）：所有消息更新通过统一 `RebuildAll` 路径触发（无增量更新）。`MessagePipeline`（`message_pipeline.rs`）维护规范状态，`build_tail_vms()` 构建尾部 VMs，`messages_to_view_models()` 是唯一转换入口。流式文本通过 100ms 节流触发 RebuildAll，非流式事件立即触发。独立 `RenderThread` 处理渲染，通过 `RenderCache(RwLock)` 与 UI 线程同步。

**系统提示词**：`build_system_prompt(overrides, cwd, features)` 合成，段落文件位于 `rust-agent-tui/prompts/sections/`。`PromptFeatures` 控制条件段落注入。

## Thinking/推理模式

`ThinkingConfig` 控制推理参数。Anthropic 用 `thinking + output_config.effort`，OpenAI 用 `reasoning_effort`。`budget_tokens` 最小 1024，`max_tokens` 必须 > `budget_tokens`。

**OpenAI Reasoning 回传**（`openai.rs`）：
- `reasoning_content` 顶层字段：所有模型无条件回传
- content 数组 `thinking` 类型：仅 deepseek-v4-pro（`supports_thinking_content` 标志控制）

**[TRAP]** DeepSeek `unknown variant 'thinking'`：不要把 `Reasoning` block 序列化为 `{"type":"thinking"}` 发给不支持的 provider。**[TRAP]** `reasoning_content must be passed back`：过滤 `Reasoning` 时必须同时作为顶层字段回传。两个陷阱互相关联。（详见 spec/global/domains/agent.md#issue_2026-05-12-glm-reasoning-field-not-parsed）

## Tool Search 延迟加载

非核心工具通过 `SearchExtraTools` 按需发现、`ExecuteExtraTool` 代理执行。核心工具（12 个）：Read/Write/Edit/Glob/Grep/folder_operations/Bash/WebFetch/WebSearch/Agent/AskUserQuestion/TodoWrite。

**[TRAP]** `Box<dyn BaseTool>` 不能直接转 `Arc<dyn BaseTool>`，用 `box_to_arc()` 通过 `ToolWrapper(ManuallyDrop<Box>)` 透传。**绝不能用 `Box::into_raw` + `Arc::from_raw`**——布局不同导致 UB。

**[TRAP]** Prompt Cache 前缀稳定性——两个已踩坑的违反模式：（1）HashMap 迭代顺序不确定导致序列化内容跨进程变化（详见 spec/global/domains/message-pipeline.md#issue_2026-05-12-deferred-tool-list-nondeterministic-order）；（2）`prepend_message` 向消息头部插入内容改变了 `cache_control` 标记的第一条 user 消息位置（详见 spec/global/domains/message-pipeline.md#issue_2026-05-12-skill-preload-invalidates-prompt-cache）。**通用原则**：所有参与缓存前缀的数据（system prompt、tools 数组、消息顺序）必须保证跨请求稳定，优先用 `add_message`（尾部追加）而非 `prepend_message`（头部插入）。

## 中间件链执行顺序

```
1. AgentDefineMiddleware    ← agent 定义，model/maxTurns 覆盖
2. AgentsMdMiddleware       ← CLAUDE.md/AGENTS.md 注入
3. SkillsMiddleware         ← Skills 摘要注入
4. SkillPreloadMiddleware   ← #skill-name 全文注入
5. PluginMiddleware         ← 插件命令注入
6. HookMiddleware           ← hooks 事件拦截
7. FilesystemMiddleware     ← 6 个文件系统工具
8. TerminalMiddleware       ← Bash
9. WebMiddleware            ← WebFetch/WebSearch
10. TodoMiddleware          ← after_tool 解析 TodoWrite
11. CronMiddleware          ← Cron 调度
12. LspMiddleware           ← LSP 工具 + after_tool 文件变更同步
13. HumanInTheLoopMiddleware← before_tool 拦截
14. SubAgentMiddleware      ← Agent 工具
15. McpMiddleware           ← MCP 工具和资源（pool 成功时注册）
[ReActAgent.with_system_prompt()] ← prepend
```

`register_tool` 优先级最高，覆盖同名中间件工具。

## HITL 审批

默认需审批：`Bash`、`folder_operations`、`Agent`、`Write`、`Edit`、`delete_*`、`rm_*`、`mcp__*`、`WebFetch`、`WebSearch`。

## 上下文压缩

Token 达到上下文窗口 85% 时自动触发：Micro-compact（清除可压缩工具结果）→ Full Compact（LLM 生成 9 段摘要）→ Re-inject（最近文件 + Skills）。

## MCP 中间件

`McpMiddleware` 基于 `rmcp` crate。配置合并：全局 `~/.peri/settings.json` + 项目 `{cwd}/.mcp.json`（同名覆盖）。工具命名 `mcp__{server_name}__{tool_name}`。插件 MCP 使用 `{plugin_name}__{server_name}` 前缀命名空间。

**[TRAP]** `ClaudeSettings` 的 `extraKnownMarketplaces` 和 `enabledPlugins` 需同时支持对象和数组格式。**`enabledPlugins` 写入必须用对象格式** `{"id": true}`。

**Plugin Sources 旁路表**：`load_merged_config_full` 返回 `(McpConfigFile, HashMap<String, String>)`，key 格式 `"plugin:{name}:{server}"`，value `"name@marketplace"`。`load_installed_plugins` 路径需从 `claude_home` 参数推导。

## 插件系统

兼容 Claude Code 插件生态。配置：`~/.peri/settings.json`（全局）+ `~/.claude/plugins/cache/`（插件 manifest）。

**Hooks**（`rust-agent-middlewares/src/hooks/`）：4 种执行类型（Command/Prompt/Http/Agent），14 种事件。exit code 控制流程：0=Allow，1=Warn，2=Block。SSRF 防护阻止内网地址（`ssrf_guard.rs`），回环地址允许。

**Frontmatter 解析**：skill 和插件命令用 `gray_matter` crate（YAML engine），必须复用 `Matter::<YAML>::new()` 模式。

**Skills**：搜索顺序 `~/.claude/skills/` → `skillsDir` → `./.claude/skills/` → 插件 skills。`SkillsMiddleware.with_extra_dirs()` 是插件扩展点。

## SubAgents

`.claude/agents/{agent_id}/agent.md` 定义。`tools` 为空继承父工具（排除 Agent 防递归），有值仅保留允许列表，`disallowedTools` 额外排除。插件 agent 通过 `scan_agents_with_extra_dirs` 追加搜索路径。

## LSP 中间件

`LspMiddleware` + `LspTool` + `perihelion-lsp` 客户端库。10 种操作（goToDefinition/findReferences/hover 等），`after_tool` 自动同步文件变更（`didChange` + `didSave`）。

## 环境变量

| 变量 | 说明 |
|------|------|
| `ANTHROPIC_API_KEY` | Anthropic API Key |
| `OPENAI_API_KEY` | OpenAI 兼容 API Key |
| `OPENAI_BASE_URL` | API Base URL |
| `OPENAI_MODEL` | 模型名称 |
| `YOLO_MODE=true/false` | 跳过/启用 HITL 审批 |
| `RUST_LOG` | 日志级别（默认 info） |
| `RUST_LOG_FILE` | 日志文件路径 |
| `LANGFUSE_*` | Langfuse 追踪 |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | OTLP 导出端点 |

配置通过 `~/.peri/settings.json` 的 `env` 字段注入。

## CLI 参数

`-a` 启用 HITL 审批。运行时 `Shift+Tab` 切换权限模式，`Alt+M` 切换模型。支持多 session 分屏。

## 编码规范

- Rust 2021 edition，tokio async/await + async-trait
- 库用 `thiserror`，应用层用 `anyhow::Result`
- 日志用 `tracing`，禁止 `println!`/`eprintln!`
- 测试与源码分离为同目录 `_test.rs` 文件（≥30 行必须分离）
- bin crate 集成测试在 `src/` 内（不支持 `tests/` 目录）
- 每模块一目录，`mod.rs` 入口；Workspace resolver = "2"，禁止下层依赖上层
- 禁止 `ℹ`（U+2139）符号和 `[i]` 前缀
- **字符串截断必须用字符级操作**：`s.chars().take(N).collect()` 或 `s.char_indices().nth(N)`，`&s[..N]` 对 CJK 会 panic
- 终端列宽用 `unicode-width` crate（CJK 占 2 列）

## 测试编写风格

- 注释、断言消息用中文；命名 `test_<被测对象>_<场景>`
- Arrange-Act-Assert，无空行分隔
- 断言优先 `assert_eq!`/`assert!`，`.unwrap()` 仅用于构造测试数据
- Mock 命名 `make_`/`mock_` 前缀，不跨文件共享
- 最小依赖：`assert!`/`assert_eq!`/`matches!` + `tempfile` + `tokio-test`

## 开发注意事项

- **BaseMessage vs MessageViewModel 维度混淆 [TRAP]**：`completed_len_at_round_start` 是 BaseMessage 长度，`prefix_len` 是 VM 索引，两者非 1:1。`prefix_len` 必须用 `round_start_vm_idx`，`drain` 必须钳位。**禁止 Pipeline 内部返回 `RebuildAll`**——Pipeline 不拥有 `round_start_vm_idx`。
- **`Interrupted`/`Error` + `Done` 互斥 [TRAP]**：`Interrupted`/`Error` 先 `request_rebuild()` + 添加通知，设 `reconcile_already_done=true`，后续 `Done` 跳过 `request_rebuild()` 防止覆盖通知。
- **快捷键设计**：禁止 `Shift+字母`（编辑态等同大写输入）。全局用 `Ctrl+字母`，面板用方向键/Space/Enter/Esc。
- **面板系统**：`PanelManager` + `PanelComponent` trait（`panel_manager.rs`/`panel_component.rs`），新增面板只需定义变体 + 实现 trait。面板内禁止渲染提示行，由 `status_bar_hints()` 统一描述。
- **`Event::Paste`**：独立于 key event 链，必须单独拦截。
- **测试隔离**：禁止写入全局配置。用 `App::save_config(cfg, self.config_path_override.as_deref())`。
- **`std::sync::RwLockReadGuard` 不是 `Send`**，async 中不能跨 `.await` 持有，用 `parking_lot::RwLock`。（详见 spec/global/domains/lsp.md#issue_2026-05-12-lsp-transport-no-fast-fail-on-process-exit）
- **`CommandRegistry::dispatch` 借用限制 [TRAP]**：`&self` + `&mut App` 冲突，当前用 `std::mem::take` + put-back 解决。
