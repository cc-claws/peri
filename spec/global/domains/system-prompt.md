# 系统提示词 领域

## 领域综述

系统提示词领域负责 Agent 系统提示词的架构设计，将单体提示词拆分为独立段落文件，支持基于功能的条件注入。

核心职责：
- 12 个 .md 段落文件按编号排序，8 个静态 + 4 个 feature-gated
- include_str! 编译时嵌入，零运行时开销
- PromptFeatures 从环境变量推断功能开关
- 动态覆盖块从 AgentOverrides 生成

## 核心流程

### 提示词合成流程

```
build_system_prompt(overrides, cwd, features)
  → 静态段落（01-08）: 始终 include_str!
  → Feature-gated 段落（10-13）: PromptFeatures 条件判断
  → 环境变量替换: {{cwd}}, {{is_git_repo}}, {{platform}}, {{os_version}}, {{date}}
  → AgentOverrides 覆盖块: persona/tone/proactiveness 注入到最前面
```

## 技术方案总结

| 维度 | 选型 |
|------|------|
| 段落文件 | prompts/sections/ 目录，12 个 .md 文件按编号排序 |
| 静态段落 | 01_intro, 02_system, 03_doing_tasks, 04_actions, 05_using_tools, 06_tone_style, 07_communicating, 08_env |
| Feature-gated | 10_hitl, 11_subagent, 12_cron, 13_skills |
| 编译嵌入 | include_str! 宏，零运行时开销 |
| 条件注入 | PromptFeatures::detect() 从环境变量推断 |
| 环境变量 | PromptEnv::detect() 运行时环境检测 |

## Feature 附录

### feature_20260430_F001_system-prompt-restructure
**摘要:** 系统提示词拆分为独立段落文件并支持 Feature 条件注入
**关键决策:**
- 提示词从单体文件拆分为 sections/ 子目录下 12 个按编号排序的 .md 文件
- 8 个静态段落始终包含，4 个 Feature-gated 段落通过 PromptFeatures 条件注入
- 使用 include_str! 编译时嵌入，零运行时开销
- PromptFeatures::detect() 从环境变量推断，长期改为从中间件注册列表推断
- 同步 claude-code 工具 description 详细版本
- 工具名从 PascalCase 映射为 snake_case
**归档:** [链接](../../archive/feature_20260430_F001_system-prompt-restructure/)
**归档日期:** 2026-04-30

---

## Issue 经验附录

### issue_2026-05-13-system-prompt-dynamic-parts-duplicated-in-consecutive-calls

**摘要:** prepend_message 的 insert(0) 右移导致 StateSnapshot 包含 System 消息，下一轮动态内容重复注入
**状态:** Fixed
**归档日期:** 2026-05-14
**关键词:** prepend_message 右移, insert(0), StateSnapshot 泄露, System 消息泄露, agent_state_messages
**问题本质:** prepend_message 使用 insert(0) 将所有已有元素右移，导致 state.messages()[last_message_count..] 的快照范围包含被右移到该范围内的 System 消息。这些泄露的 System 消息通过 agent_state_messages 进入下一轮 history，与新的 prepend 消息合并产生重复。
**通用模式:** prepend_message 的 insert(0) 有隐式的范围扩大副作用——快照计算时必须考虑右移效应。StateSnapshot 应始终过滤 System 消息（.filter(|m| !m.is_system())），因为 System 消息由 middleware 每轮重新注入，不应持久化到历史中。
**架构影响:** 确认了 agent_state_messages 不应包含 BaseMessage::System 变体的设计原则。compact 路径也有独立的 System 消息泄露问题（直接将 compact 结果写入 agent_state_messages）。
**涉及文件:** peri-agent/src/agent/executor/final_answer.rs, peri-agent/src/agent/executor/mod.rs, peri-agent/src/agent/state.rs, peri-agent/src/llm/openai.rs, peri-tui/src/app/agent_ops.rs, peri-tui/src/app/agent_submit.rs, peri-tui/src/app/agent_compact.rs
**CLAUDE.md 链接:** true

### issue_2026-05-13-system-prompt-dynamic-cache-invalidation

**摘要:** System prompt 动态内容（date/cwd/middleware 注入）导致 Anthropic prompt cache 频繁失效，修复方案为边界标记拆分
**状态:** Fixed
**归档日期:** 2026-05-14
**关键词:** Prompt Cache, 边界标记, __SYSTEM_PROMPT_DYNAMIC_BOUNDARY__, split_system_blocks, 动态占位符
**问题本质:** 整个 system prompt 作为单一缓存段，动态占位符（{{date}} 每日变化、{{cwd}} 跨项目变化）导致缓存前缀完全失效。边界标记方案将静态核心（01-06，~80% token）与动态内容（07_env + feature-gated + middleware，~20%）分离为独立缓存块。
**通用模式:** 缓存前缀稳定性原则——所有参与缓存前缀的数据必须保证跨请求稳定。动态内容应通过边界标记或独立块隔离到缓存段之外。此方案仿照 Claude Code 的 splitSysPromptPrefix() 模式。
**架构影响:** 引入 __SYSTEM_PROMPT_DYNAMIC_BOUNDARY__ 标记和 split_system_blocks() 方法，但不改变 ReactLLM trait 签名。后续可进一步拆分 feature-gated 静态段落为独立缓存块（需变更 trait 签名）。
**涉及文件:** peri-tui/src/prompt.rs, peri-agent/src/llm/anthropic.rs
**CLAUDE.md 链接:** true

### issue_2026-05-13-missing-skillpreload-in-main-agent

**摘要:** 主 Agent 中间件链缺少 SkillPreloadMiddleware，/skill-name 预加载功能完全不可用
**状态:** Fixed
**归档日期:** 2026-05-14
**关键词:** SkillPreloadMiddleware, 中间件链缺失, /skill-name, 预加载
**问题本质:** agent.rs 和 agent_assembler.rs 两个构建路径均未注册 SkillPreloadMiddleware，导致系统提示声称支持 skill 预加载但实际不生效。子 Agent 路径通过 tool.rs 正确注册。
**通用模式:** 中间件链的完整性需要同时检查所有构建路径。功能声明（系统提示）与实际实现（中间件注册）必须保持一致。空参数的 SkillPreloadMiddleware（无 skill 列表）会 early return 无性能损耗，可安全始终注册。
**涉及文件:** peri-tui/src/app/agent.rs, peri-tui/src/app/agent_assembler.rs, peri-middlewares/src/subagent/skill_preload.rs, peri-middlewares/src/subagent/tool.rs
**CLAUDE.md 链接:** false

### issue_2026-05-13-askuserquestion-cache-hit-rate-drop

**摘要:** AskUserQuestion 导致缓存命中率极速下降
**状态:** Fixed
**归档日期:** 2026-05-14
**关键词:** AskUserQuestion, 缓存下降, system prompt 动态占位符
**问题本质:** 此 issue 是 system-prompt-dynamic-cache-invalidation 的子集表现。AskUserQuestion 调用时缓存下降的根因是 system prompt 动态占位符导致整段缓存失效，而非 AskUserQuestion 本身的问题。
**通用模式:** 表面症状（特定工具调用导致缓存下降）的根因可能在更底层（system prompt 架构）。修复底层问题后上层症状自动消除。
**涉及文件:** peri-middlewares/src/tool_search/middleware.rs, peri-agent/src/llm/anthropic.rs
**CLAUDE.md 链接:** false

### issue_2026-05-20-rapid-context-expansion
**摘要:** System Prompt 每轮重复注入导致上下文膨胀
**状态:** Fixed
**归档日期:** 2026-05-20
**关键词:** Frozen Session Data, system prompt 膨胀, prepend_message, StateSnapshot, 上下文爆炸
**问题本质:** 每轮 Agent 执行时重新构建 system prompt（含 MCP 工具注册），prepend_message 向消息头部插入导致 StateSnapshot 快照范围扩大，system prompt 内容跨轮次累积形成倍数膨胀
**通用模式:** 会话内不可变数据（system prompt、CLAUDE.md、skill summary、frozen_date）必须在 session/new 时构建一次并冻结，后续所有轮次直接使用已冻结值，禁止跨轮次重建
**架构影响:** FrozenSessionData 模式统一了所有会话内不可变数据的管理，成为系统提示词稳定性的基础设施
**技术决策:** session/new → 冻结全部不可变数据 → 后续轮次直接注入，消除重建开销和累积风险
**涉及文件:** peri-acp/src/session/executor.rs, peri-tui/src/acp_server/prompt.rs, peri-tui/src/acp_server/requests.rs
**CLAUDE.md 链接:** true

---

## 相关 Feature
- → [agent.md](./agent.md) — ReActAgent.with_system_prompt() 注入
- → [tui.md](./tui.md) — TUI 层 build_system_prompt() 调用
