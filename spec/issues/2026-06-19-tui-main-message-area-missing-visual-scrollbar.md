# 主消息区缺少可视滚动条反馈（终端渲染完整性缺口）

**状态**：Open
**优先级**：中
**创建日期**：2026-06-19

## 问题描述

主消息区（Messages 区域）在对话内容超出可视高度时，没有渲染任何可视滚动条指示器（thumb/gutter/箭头按钮/进度条），用户只能通过键盘（`Ctrl+U`/`Ctrl+D`/鼠标滚轮）滚动来浏览历史消息，但无法直观感知：

- 当前处于整个对话流的什么位置（顶部/中部/底部）
- 上方还有多少内容未读
- 下方还有多少内容未渲染
- 滚动操作是否生效（无视觉反馈）

用户期望像主流 TUI（如 codex）一样看到一根滚动条来辅助长对话导航。

## 症状详情

| 观察项 | 当前表现 | 期望表现 |
|--------|---------|---------|
| 长对话滚动时右侧滚动条 | 完全空白，无任何指示器 | 显示 thumb 表示当前位置和总长度比例 |
| 鼠标点击主消息区右侧 | 无视觉响应（仅 scrollbar_min/max_offset 用于事件坐标映射） | 点击/拖动 thumb 可快速跳转 |
| 顶部/底部到达感知 | 需手动滚动尝试，无指示 | thumb 贴顶/贴底可直观判断 |
| 与面板/弹窗滚动条一致性 | 面板/弹窗有 `ScrollableArea`（thumb `█`），主消息区无 | 主消息区也有同款滚动条 |

### 对比 codex TUI

参考 openai/codex 的 TUI 实现（`codex-rs/tui/src/chatwidget.rs`）：

- codex 同样未在主视图直接渲染 `Scrollbar` widget
- codex 使用 `BottomPane` + 内部 scroll state 管理，用户感知到的"滚动反馈"来自历史区块布局变化
- 但 peri 当前的主消息区是单一带 `Paragraph::scroll((offset, 0))` 的渲染区，缺乏任何位置指示

`peri-widgets/src/scrollable.rs` 已经实现了统一滚动条 `ScrollableArea`（thumb `█`，gutter `track_symbol(None)`，箭头按钮 `▲`/`▼`），但仅用于面板/弹窗/ThreadBrowser 等 13 个位置，主消息区未接入。

## 复现条件

- **复现频率**：必现
- **触发步骤**：
  1. 启动 TUI：`cargo run -p peri-tui`
  2. 进行多轮对话（≥ 5 轮）使消息累积超过一屏
  3. 观察右侧主消息区边缘——无任何滚动条/进度条/thumb
  4. 按 `Ctrl+U` 向上滚动——内容变化但无位置指示
- **环境**：所有 OS，所有终端

## 涉及文件

- `peri-tui/src/ui/main_ui/message_area.rs`（约 496 行）—— 主消息区渲染逻辑，目前使用 `Paragraph::scroll((offset, 0))`，未渲染任何 Scrollbar widget；`scrollbar_min_offset`/`scrollbar_max_offset` 字段仅用于鼠标点击坐标映射
- `peri-widgets/src/scrollable.rs`（约 223 行）—— 已有的统一 `ScrollableArea` 实现（thumb `█`，含上下箭头按钮和 `ScrollbarMetrics`），当前仅被 13 个面板/弹窗文件使用，未接入主消息区
- `peri-tui/src/ui/main_ui/sticky_header.rs`（约 120 行）—— 滚动时顶部固定显示最后一条 Human 消息摘要（独立机制，与滚动条不冲突但可协同）

## 状态变更记录

| 日期 | 从 | 到 | 操作人 | 说明 |
|------|-----|-----|--------|------|
| 2026-06-19 | — | Open | agent | 创建（对比 codex TUI 后发现完整性缺口） |

## 修复记录

（由 fix-issue 或 issue-verify skill 追加，创建时留空）
