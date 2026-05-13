# 消息管线 领域

## 领域综述

消息管线领域负责统一 Agent 事件到 TUI 视图模型的消息转换，处理流式增量更新和历史恢复两条路径，确保最终一致性。

核心职责：
- MessagePipeline 成为消息状态管理唯一入口
- PipelineAction 枚举统一描述所有 UI 变更操作
- 流式 AppendChunk 优化 + Done 时 reconcile 尾部重建确保一致性
- SubAgent 路由逻辑集中管理

## 核心流程

### 消息处理管线

```
AgentEvent → MessagePipeline.handle_event()
  → 转换为 Vec<PipelineAction>
  → apply_pipeline_action():
      AddMessage → 追加 view_model
      AppendChunk → 增量更新最后一条 assistant
      RebuildAll{prefix_len, tail_vms} → 替换尾部
      RemoveLast / RemoveLastN → 删除最近消息
      StreamingDone → 最终一致性重建
  → Done/Interrupted → reconcile_tail() 尾部重建
```

### 尾部重建流程

```
reconcile_tail(round_start_vm_idx)
  → 找到最后一条 Human 消息
  → 从该位置开始重建 view_models
  → 返回 (prefix_len, tail_vms)
  → RebuildAll 只替换尾部，保留前缀
```

## 技术方案总结

| 维度 | 选型 |
|------|------|
| 核心组件 | MessagePipeline 结构体，持有 view_messages 和 render_tx |
| 操作枚举 | PipelineAction: None/AddMessage/AppendChunk/UpdateLast/RemoveLast/RemoveLastN/RebuildAll |
| 事件拆分 | ToolCall 拆分为 ToolStart + ToolEnd 两个独立事件 |
| 流式优化 | AppendChunk 直接操作渲染层，finalize 边界 reconcile |
| 尾部重建 | reconcile_tail() + round_start_vm_idx 记录轮次起始位置 |
| SubAgent 路由 | 从 agent_ops 迁入 Pipeline，移除 subagent_group_idx |

## Feature 附录

### feature_20260428_F002_message-pipeline-unify
**摘要:** 统一流式与历史恢复的消息显示管线
**关键决策:**
- MessagePipeline 成为消息状态管理唯一入口，agent_ops 不再手动操作 view_messages
- PipelineAction 枚举统一描述所有 UI 变更操作
- AgentEvent::ToolCall 拆分为 ToolStart + ToolEnd 两个独立事件
- 流式 AppendChunk 优化保留，Done 时 reconcile 确保最终一致性
- SubAgent 路由逻辑从 agent_ops 迁入 Pipeline
**归档:** [链接](../../archive/feature_20260428_F002_message-pipeline-unify/)
**归档日期:** 2026-04-30

### feature_20260430_F002_reconcile-on-done-interrupted
**摘要:** Done/Interrupted 事件触发尾部重建确保流式与恢复路径一致
**关键决策:**
- RebuildAll 改为携带 prefix_len + tail_vms 的结构体形式，只替换尾部
- 新增 reconcile_tail() 方法，从最后一条 Human 消息开始重建 view_models
- 通过 round_start_vm_idx 记录本轮起始位置
- 移除 StreamingDone 变体，职责合并到 RebuildAll
- 保留全量 reconcile() 方法供 CompactDone 等其他场景使用
**归档:** [链接](../../archive/feature_20260430_F002_reconcile-on-done-interrupted/)
**归档日期:** 2026-04-30

## Issue 经验附录

### issue_2026-05-12-deferred-tool-list-nondeterministic-order
**摘要:** 多处 HashMap 非确定性顺序导致 Anthropic Prompt Cache 前缀不稳定
**状态:** Fixed + Verify
**归档日期:** 2026-05-13
**关键词:** HashMap 顺序, Prompt Cache, 缓存前缀, ToolSearchIndex
**问题本质:** HashMap 迭代顺序不确定（Rust 默认 RandomState），跨进程重启时 API 请求前缀变化，导致 Prompt Cache 失效。涉及 ToolSearchIndex 的 deferred tools 列表注入和 executor 的 tool_refs 两个独立位置
**通用模式:** 所有需要跨进程复用的序列化内容（system prompt、tools 数组）必须保证顺序稳定。任何参与缓存前缀的数据结构，其迭代顺序必须是确定性的
**架构影响:** ToolSearchIndex 从 submit 级局部变量提升到 session 级共享 Arc，减少重复构建的同时保证缓存一致性
**技术决策:** 工具列表按名称排序；ToolSearchIndex 会话级缓存；每轮注入缓存提示词而非重新构建
**涉及文件:** rust-agent-middlewares/src/tool_search/tool_index.rs, rust-create-agent/src/agent/executor/mod.rs, rust-agent-tui/src/app/agent_comm.rs, rust-agent-tui/src/app/agent.rs, rust-agent-tui/src/app/agent_submit.rs
**CLAUDE.md 链接:** true

### issue_2026-05-12-skill-preload-invalidates-prompt-cache
**摘要:** Skill Preload 注入消息到历史最前面导致首轮 Prompt Cache 失效
**状态:** Fixed + Verify
**归档日期:** 2026-05-13
**关键词:** Prompt Cache, prepend_message, add_message, cache_control
**问题本质:** SkillPreloadMiddleware 用 prepend_message 将合成消息插入 index 0，改变了第一条 user 消息的位置，Anthropic 的 cache_control 标记落在不稳定的合成消息上，导致首轮 cache miss
**通用模式:** 向消息数组头部插入内容会改变缓存边界，应优先使用尾部追加（add_message）。缓存控制标记（cache_control）的位置决定了缓存前缀的稳定性
**技术决策:** prepend_message 改为 add_message，使 preload 工具调用追加在用户消息之后
**涉及文件:** rust-agent-middlewares/src/subagent/skill_preload.rs, rust-create-agent/src/agent/compact/re_inject.rs, rust-create-agent/src/llm/anthropic.rs
**CLAUDE.md 链接:** true

### issue_2026-05-12-systemnote-position-drift-on-rebuild
**摘要:** SystemNote 在 RebuildAll 后堆积到消息列表末尾
**状态:** Fixed
**归档日期:** 2026-05-13
**关键词:** SystemNote, RebuildAll, ephemeral_notes, 锚点机制
**问题本质:** AddMessage 直接 push 到 view_messages 末尾，RebuildAll 的 saved_notes 机制保存后追加到末尾，导致 SystemNote 永远漂移到消息流最后
**通用模式:** 纯 UI 层的临时 VM（不在 BaseMessage 中）需要独立的锚点机制来维持位置。RebuildAll 的 drain+重建会破坏所有尾部追加内容的位置
**架构影响:** 引入 ephemeral_notes 字段记录 (锚点, VM) 对，RebuildAll 时按锚点位置重新插入。这是 message-pipeline 处理纯 UI VM 生命周期的通用模式
**技术决策:** VM 索引锚点方案——记录创建时 view_messages.len() 作为锚点，RebuildAll 时根据锚点与 prefix_len 的关系决定保留/丢弃/重插入
**涉及文件:** rust-agent-tui/src/app/agent_render.rs, rust-agent-tui/src/app/message_pipeline.rs
**CLAUDE.md 链接:** false

### issue_2026-05-12-cache-warning-discarded-by-rebuild
**摘要:** CacheWarning 消息被 RebuildAll 立即丢弃，用户无法看到
**状态:** Fixed + Verify
**归档日期:** 2026-05-13
**关键词:** CacheWarning, RebuildAll, saved_notes, ephemeral VM
**问题本质:** RebuildAll 的 saved_notes 过滤器只保留 SystemNote 变体，不保留 CacheWarning，导致缓存警告一闪而过
**通用模式:** 新增 ephemeral VM 变体时，必须同步更新 RebuildAll 的 saved_notes 过滤逻辑，否则会被 drain 丢弃
**涉及文件:** rust-agent-tui/src/app/agent_ops.rs, rust-agent-tui/src/app/agent_render.rs, rust-agent-tui/src/ui/message_view.rs
**CLAUDE.md 链接:** false

### issue_2026-05-12-compact-ephemeral-notes-not-cleared
**摘要:** Compact 完成后残留 "正在压缩上下文…" 系统通知
**状态:** Closed
**归档日期:** 2026-05-13
**关键词:** ephemeral_notes, compact, RebuildAll, prefix_len: 0
**问题本质:** Compact 完成后的 RebuildAll { prefix_len: 0 } 保留所有锚点 >= 0 的 ephemeral_notes，包括 compact 前的旧通知
**通用模式:** 全量重建（prefix_len: 0）时，应先清理过期的 ephemeral_notes，否则所有历史临时通知都会被保留
**涉及文件:** rust-agent-tui/src/app/agent_compact.rs, rust-agent-tui/src/app/thread_ops.rs, rust-agent-tui/src/app/agent_ops.rs, rust-agent-tui/src/app/agent_render.rs
**CLAUDE.md 链接:** false

---

## 相关 Feature
- → [tui.md](./tui.md) — TUI 渲染依赖 MessagePipeline 输出的 view_models
- → [agent.md](./agent.md) — AgentEvent 事件定义在核心层
