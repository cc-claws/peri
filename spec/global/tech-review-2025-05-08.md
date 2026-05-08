# Perihelion 项目技术评估报告

**日期**：2025-05-08
**评估范围**：全 workspace 6 个 crate
**评估维度**：模块耦合度、跨 crate 依赖、代码坏味道、错误处理与健壮性

---

## 总体评分

| 维度 | 评分 | 评级 |
|------|------|------|
| 核心框架耦合度 | 7.5/10 | B+ |
| 中间件层耦合度 | 6.5/10 | B- |
| TUI 层耦合度 | 6.0/10 | B- |
| 跨 crate 依赖关系 | 8.0/10 | A- |
| 代码坏味道 | 6.5/10 | B- |
| 错误处理与健壮性 | 7.0/10 | B |
| **综合** | **6.9/10** | **B** |

> 架构设计合理，依赖方向清晰（无循环依赖），但 TUI 层和中间件层存在较重的架构债务需要关注。

---

## 高优先级问题 Top 10

### 1. event.rs 巨型文件 — 2486 行，52 个 match 块

- `next_event()` 函数 907 行，15 层面板优先级检查
- 27 处 `login_panel.as_mut().unwrap()` 密集使用
- 影响：新增面板需改 3 处，维护成本极高
- 建议：拆分为独立 handler 模块 + PanelRegistry 统一路由

### 2. App God Object — 34 个字段，11 个 Option<Panel>

- AppCore 额外 26 个字段，混合全局/会话/面板/配置状态
- 影响：测试困难，状态追踪复杂
- 建议：提取 PanelManager，分层解耦全局/会话/UI 状态

### 3. 非测试代码 unwrap/expect — 141 处 ✅ 部分修复

- event.rs 独占 ~30 处 → 已消灭最危险的 2 处（`a42ba74`）
- OpenAI adapter 中 expect("missing tool_calls") 在非标准 LLM 响应时崩溃
- 建议：改为 if let / match 防御，或用 ? 传播错误

### 4. 错误处理不一致 — anyhow 泄漏到库 crate

- AgentError::Other(#[from] anyhow::Error) 破坏了结构化错误链
- OpenAI/Anthropic adapter 全用 anyhow::Result
- 建议：库 crate 统一用 thiserror，移除 Other 变体

### 5. MCP 配置合并函数过长 — 124 行 ✅ 已修复

- load_merged_config_full() 承担 6 种职责
- 插件 sources 旁路表是 LoadedPlugin 缺 marketplace 字段的 workaround
- **修复**：给 LoadedPlugin 添加 marketplace 字段，消除旁路表（-15 行冗余逻辑）

### 6. SubAgent 硬编码中间件链

- 子 agent 在 subagent/tool.rs 手动组装 5 个中间件
- 与父 agent 配置链不同步，新增中间件需手动维护
- 建议：传递 Vec<Box<dyn Middleware>> 或用 MiddlewareRegistry

### 7. CJK 不安全字节切片 — 2 处高风险 ✅ 已修复

- hooks/output_parser.rs:45 — &trimmed[..200]
- hooks/executor.rs:306 — &body[..body.len().min(200)]
- 修复：已加 floor_char_boundary 保护

### 8. 面板互斥逻辑重复 10 次

- 每个 open_*_panel() 手动关闭其他 10 个面板
- 建议：统一由 PanelManager 管理互斥

### 9. TUI 层访问核心框架内部类型

- 直接调用 micro_compact_enhanced、full_compact 等内部函数
- 访问 llm::types::TokenUsage、telemetry::init_tracing 等内部模块
- 建议：在核心框架创建外观层（Facade），限制 pub 范围

### 10. 资源泄漏风险 — MCP 连接池、错误静默吞掉 ✅ 部分修复

- 连接失败后部分资源可能未清理
- `let _ =` 静默吞掉错误 → **已加 warn! 日志**（langfuse-client ×2、agent 事件通道、cron trigger、subagent 通知、batcher ack）
- 建议：实现 RAII 清理，继续覆盖其余 `let _ =` 点

---

## 中优先级问题汇总

| # | 问题 | 位置 | 建议 |
|---|------|------|------|
| 11 | AgentEvent 17 变体过多 | agent/events.rs | 按 LLM/Tool/Lifecycle/System 拆分 |
| 12 | State trait 12 个方法职责混杂 | agent/state.rs | 拆为 MessageStorage + ContextStorage + ExecutionTracker |
| 13 | 面板 Mode 枚举重复（Browse/Edit/ConfirmDelete） | 各面板文件 | 抽象通用 Panel trait |
| 14 | Frontmatter 解析逻辑重复 | claude_agent_parser.rs / skills/loader.rs | 提取通用 parse_frontmatter<T>() |
| 15 | HITL decide_by_mode 圈复杂度 ~15 | hitl/mod.rs | 用策略模式拆分 |
| 16 | Channel 容量硬编码（32/64/8/无界） | 分散 10+ 文件 | 统一为命名常量 |
| 17 | thiserror v1 vs v2 共存 | 间接依赖 | 监控上游升级 |
| 18 | 超长函数 29 个（>150行） | 多文件 | 优先拆 event.rs 的 3 个函数 |

---

## 架构亮点

1. **零循环依赖** — workspace 依赖方向清晰（下→上），cargo tree 验证通过
2. **零 unsafe 代码** — 全代码库无 unsafe 块
3. **langfuse-client / acpx-g 完全独立** — 可作为独立子项目发布
4. **中间件 trait 设计清晰** — 12 个中间件正确实现 collect_tools/before_agent/before_tool
5. **渲染线程解耦** — 独立渲染线程 + RenderCache，不阻塞主循环
6. **LLM 重试机制完善** — RetryableLLM 装饰器实现指数退避
7. **TODO 债务极低** — 仅 3 处 TODO，无 FIXME/HACK

---

## 量化指标总览

| 指标 | 当前值 | 上次 | 健康值 | 状态 |
|------|--------|------|--------|------|
| App 字段数量 | 34 | 34 | < 20 | 🔴 高 |
| event.rs 行数 | 2,485 | 2,486 | < 1,000 | 🔴 高 |
| 非测试 unwrap/expect | 141 | 141 | < 30 | 🔴 高 |
| 超长函数 (>150行) | 29 | 29 | < 10 | 🔴 高 |
| 面板互斥逻辑重复 | 10 次 | 10 | 0 | 🔴 高 |
| CJK 不安全切片 | **0** | 2 | 0 | 🟢 健康 |
| event.rs 无保护 unwrap | **0** | 2 | 0 | 🟢 健康 |
| `let _ =` 静默吞关键错误 | **-6** | 258 | < 10 | 🟡 改善 |
| AgentEvent 变体数 | 17 | 17 | < 10 | 🟡 中 |
| MCP config 函数行数 | **~109** | 124 | < 50 | 🟡 改善 |
| TODO/FIXME 标记 | 3 | 3 | < 10 | 🟢 健康 |
| 循环依赖 | 0 | 0 | 0 | 🟢 健康 |
| unsafe 块 | 0 | 0 | 0 | 🟢 健康 |

---

## 改进路线图建议

### Phase 1 — 灭火（1-2 周）

1. ✅ 修复 2 处 CJK 字节切片 panic → 加 floor_char_boundary
2. ✅ 消灭 event.rs 中最危险的 unwrap → 改为 if let 防御
3. ✅ 给 LoadedPlugin 加 marketplace 字段，消除旁路表

> **Phase 1 全部完成** ✅

### Phase 2 — 重构（2-4 周）

4. 拆分 event.rs → 独立 handler 模块 + PanelRegistry
2. 提取 PanelManager 统一面板生命周期和互斥
3. 统一库 crate 错误处理，移除 AgentError::Other(anyhow)

### Phase 3 — 优化（1-2 月）

7. 拆分 AgentEvent 为分类子枚举
2. 创建核心框架 Facade 层，限制 pub 范围
3. SubAgent 中间件链参数化
4. 统一 channel 容量常量管理

---

## 修复进度

> 2025-05-08 开始逐项修复，每修复一个提交一个 git commit。

| # | 问题 | 状态 | Commit |
|---|------|------|--------|
| 1 | CJK 字节切片 panic（#7）| ✅ 已修复（评估前完成）| — |
| 2 | event.rs 最危险 unwrap（#3）| ✅ 已修复 | `a42ba74` |
| 3 | `let _ =` 静默吞错 — OTLP body（#10）| ✅ 已修复 | `53686ae` |
| 4 | `let _ =` 静默吞错 — agent 事件通道（#10）| ✅ 已修复 | `fd719b1` |
| 5 | `let _ =` 静默吞错 — cron trigger（#10）| ✅ 已修复 | `a009d60` |
| 6 | `let _ =` 静默吞错 — subagent 通知（#10）| ✅ 已修复 | `60c1df3` |
| 7 | `let _ =` 静默吞错 — batcher ack（#10）| ✅ 已修复 | `9adaf97` |
| 8 | LoadedPlugin 加 marketplace 字段（#5）| ✅ 已修复 | `515bbd1` |

---

## 各 Crate 详细评估

### rust-create-agent（核心框架）— 7.5/10

**优势**：清晰的 trait 抽象（Middleware/BaseTool/ReactLLM）、零 unsafe、良好测试覆盖、合理生命周期管理

**问题**：

- anyhow 混入库 crate（error.rs:37, thread/store.rs, messages/adapters/）
- BaseTool::invoke 返回 Box<dyn Error>，类型信息丢失（tools/mod.rs:38）
- Middleware::before_tool 强制 clone ToolCall（middleware/trait.rs:38）
- AgentEvent 17 变体过多（agent/events.rs:16-83）
- State trait 12 方法职责混杂（agent/state.rs:11-28）
- BaseModelReactLLM 职责外溢（llm/react_adapter.rs:11-27）

### rust-agent-middlewares（中间件层）— 6.5/10

**优势**：中间件 trait 实现规范、MCP 模块文件职责划分清晰、Arc/RwLock 并发处理得当

**问题**：

- MCP load_merged_config_full 124 行 6 职责（mcp/config.rs:280-403）
- SubAgent 硬编码 5 个中间件（subagent/tool.rs:322-338）
- 后台 agent 模式复杂度高（subagent/tool.rs 50 个 Arc clone）
- 插件 sources 旁路表 workaround（mcp/config.rs:335-344）
- Frontmatter 解析重复（claude_agent_parser.rs / skills/loader.rs）
- HITL decide_by_mode 圈复杂度 ~15（hitl/mod.rs:150-268）
- scan_agents / scan_agents_with_extra_dirs 80% 重复

### rust-agent-tui（TUI 应用）— 6.0/10

**优势**：MessagePipeline 设计优秀、会话隔离良好、渲染线程解耦、插件系统扩展性强

**问题**：

- App 34 字段 God Object（app/mod.rs:87-134）
- event.rs 2486 行、next_event() 907 行（event.rs）
- 15 层面板优先级 if-链（event.rs:222-303）
- 面板互斥逻辑重复 10 次（panel_ops.rs）
- 渲染逻辑直接访问修改 App 状态（main_ui.rs:19-60）
- 面板 Mode 枚举重复（login_panel/config_panel 等）
- 系统提示词构建无缓存（prompt.rs:91-99）
- Channel 容量硬编码不一致（mpsc(32/64/8)）

### 跨 crate 依赖 — 8.0/10

**优势**：零循环依赖、workspace 版本统一、feature gate 设计良好

**问题**：

- TUI 访问核心框架 6 处内部类型（compact/llm::types/telemetry）
- regex 未纳入 workspace 依赖
- thiserror v1/v2 共存（oauth2/sqlx 间接依赖）
- agent-client-protocol 增加 TUI 编译负担

### 错误处理与健壮性 — 7.0/10

**优势**：RetryableLLM 指数退避、McpPoolError 结构化、Langfuse Batcher Drop 清理

**问题**：

- 错误静默吞掉（let _ =）— MCP 通知、Langfuse 响应
- MCP 连接失败后资源可能未清理
- 文件操作非原子（thread/filesystem.rs:92-100）
- JoinError 未处理（subagent/tool.rs:528-580）
- 超时配置硬编码（langfuse client 5s/30s）
- parking_lot::RwLock 无超时机制

---

*报告由 6 位评估员并行分析生成，覆盖核心框架、中间件层、TUI 层、跨 crate 依赖、代码坏味道、错误处理与健壮性六个维度。*
