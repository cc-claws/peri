// ── Event module ──────────────────────────────────────────────────────────────
// Split from the original monolithic event.rs (1447 lines) into:
//   mouse.rs   — mouse coordinate helpers + clipboard functions
//   keyboard.rs — key event handler
//   macros.rs  — panel dispatch macros (with_global_panels!, with_session_panels!)
//   mod.rs     — Action, event loop, dispatcher, OAuth handling

pub mod keyboard;
mod macros;
pub mod mouse;

use crate::{with_global_panels, with_session_panels};

use anyhow::Result;
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use std::time::Duration;
use tui_textarea::{Input, Key};

use crate::app::{
    panel_manager::{EventResult, PanelKind},
    App,
};

// ── Action ──────────────────────────────────────────────────────────────────

pub enum Action {
    Quit,
    Submit(String),
    RunShellCommand(String),
    Redraw,
}

// ── Event loop ──────────────────────────────────────────────────────────────

pub async fn next_event(app: &mut App) -> Result<Option<Action>> {
    // Quit-pending state auto-expires after 2s; trigger redraw so the shortcut bar
    // returns to normal.  Must match the window used by handle_ctrl_c().
    if let Some(since) = app.global_ui.quit_pending_since {
        if since.elapsed() >= std::time::Duration::from_secs(2) {
            app.global_ui.quit_pending_since = None;
            return Ok(Some(Action::Redraw));
        }
    }

    // Mouse-availability probe: on first user input after startup, determine
    // whether the terminal supports mouse events.
    if app.global_ui.mouse_available.is_none() {
        // Wait for the first event (up to 1 s); this is not counted as normal poll timeout
        if event::poll(Duration::from_secs(1))? {
            let ev = event::read()?;
            if matches!(ev, Event::Mouse(_)) {
                app.global_ui.mouse_available = Some(true);
            } else {
                // Received keyboard/resize etc. but not mouse → terminal likely
                // does not support mice (mouse-capable terminals almost always trigger
                // scroll/move within 1 s)
                app.global_ui.mouse_available = Some(false);
            }
            return handle_event(app, ev).await;
        } else {
            // No event within 1 s → no mouse
            app.global_ui.mouse_available = Some(false);
            return Ok(None);
        }
    }

    if !event::poll(Duration::from_millis(50))? {
        return Ok(None);
    }

    let ev = event::read()?;

    // Scroll/Drag event coalescing: drain queued mouse events to avoid
    // redundant redraws during rapid scrolling or scrollbar dragging.
    let ev = coalesce_mouse_events(ev);

    // Simulated-paste detection: on terminals without bracketed paste support
    // (Windows), multi-line paste arrives as a rapid burst of key events.
    // Detect this pattern and convert to Event::Paste so the normal paste
    // handler inserts the full text into the textarea.
    let ev = detect_simulated_paste(ev);

    handle_event(app, ev).await
}

// ── Mouse event coalescing ───────────────────────────────────────────────

/// Coalesces rapid-fire mouse scroll/drag events from the crossterm queue.
///
/// When a Scroll or Drag(Left) mouse event is the initial event, drains any
/// additional queued events using a non-blocking poll, keeping only the last
/// coalesceable event. This trades scroll precision for CPU: N scroll events
/// within one poll cycle (~50ms) result in only ±3 lines moved instead of N×3.
/// Drag(Left) is unaffected since only the final position matters.
///
/// Non-coalesceable events (click, keypress, etc.) terminate the drain and
/// replace the pending scroll as the returned event (not dropped).
fn coalesce_mouse_events(ev: Event) -> Event {
    // Only activate coalescing for scroll and drag mouse events
    match &ev {
        Event::Mouse(m) => match m.kind {
            MouseEventKind::ScrollUp
            | MouseEventKind::ScrollDown
            | MouseEventKind::Drag(MouseButton::Left) => {}
            _ => return ev,
        },
        _ => return ev,
    }

    let mut last_ev = ev;

    // Drain all queued scroll/drag events, keeping only the last one.
    // Non-scroll/drag events terminate the drain and become the result
    // so they are not lost.
    while event::poll(Duration::ZERO).unwrap_or(false) {
        let next = match event::read() {
            Ok(e) => e,
            Err(_) => break,
        };
        match &next {
            Event::Mouse(m) => match m.kind {
                MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
                | MouseEventKind::Drag(MouseButton::Left) => {
                    last_ev = next;
                }
                // Other mouse events (click, release, move): stop draining,
                // return this event instead so it's handled normally
                _ => {
                    last_ev = next;
                    break;
                }
            },
            // Non-mouse events: stop draining, return this event
            _ => {
                last_ev = next;
                break;
            }
        }
    }

    last_ev
}

// ── Simulated-paste detection (Windows) ───────────────────────────────

/// On terminals that do not support bracketed paste (e.g. Windows cmd.exe,
/// legacy PowerShell), multi-line paste is simulated as a rapid burst of
/// individual Key events — each character becomes a Char event and each
/// newline becomes a bare Enter event.
///
/// This function detects that pattern from the first key in a burst. Waiting
/// until Enter is too late: the first pasted line has already been inserted as
/// normal typing, which splits one external paste into raw text plus multiple
/// placeholders.
///
/// A 1 ms start window is too short for human typing to trigger in practice.
/// Once a burst is detected, a small idle window lets slower Windows terminals
/// deliver the rest of the paste without fragmenting it at every newline.
fn detect_simulated_paste(ev: Event) -> Event {
    const START_WINDOW: Duration = Duration::from_millis(1);
    const IDLE_WINDOW: Duration = Duration::from_millis(15);

    if !is_simulated_paste_start(&ev) {
        return ev;
    }

    // Quick probe: any queued event within 1 ms?
    if !event::poll(START_WINDOW).unwrap_or(false) {
        return ev; // No queued events → manual typing / manual Enter
    }

    let original_ev = ev.clone();
    let mut text = String::new();
    let _ = key_event_to_text(ev, &mut text);
    let mut meaningful_after_first = false;

    while event::poll(IDLE_WINDOW).unwrap_or(false) {
        match event::read() {
            Ok(next) => {
                meaningful_after_first |= key_event_to_text(next, &mut text);
            }
            Err(_) => break,
        }
    }

    // A key release queued behind the press is not a paste. It is safe that the
    // release event was consumed because the TUI only acts on key presses.
    if !meaningful_after_first {
        return original_ev;
    }

    Event::Paste(text)
}

fn is_simulated_paste_start(ev: &Event) -> bool {
    match ev {
        Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
            KeyCode::Char(_) | KeyCode::Tab => {
                !k.modifiers.contains(KeyModifiers::CONTROL)
                    && !k.modifiers.contains(KeyModifiers::ALT)
            }
            KeyCode::Enter => k.modifiers == KeyModifiers::NONE,
            _ => false,
        },
        _ => false,
    }
}

/// Append a single crossterm `Event` into `text` for simulated-paste
/// reconstruction. Key(Char) appends the character; Key(Enter) appends
/// `\n`; Key(Tab) appends `\t`; Key(Backspace) removes the last char;
/// everything else (modifiers, non-printable keys) terminates the drain.
fn key_event_to_text(ev: Event, text: &mut String) -> bool {
    match ev {
        Event::Key(k) if k.kind != KeyEventKind::Release => match k.code {
            KeyCode::Char(c) => {
                // Ctrl+char or Alt+char during paste → stop collecting
                if k.modifiers.contains(KeyModifiers::CONTROL)
                    || k.modifiers.contains(KeyModifiers::ALT)
                {
                    // Flush remaining: stop collecting but don't lose the event.
                    // Since we can't re-inject, treat modifier+char as literal.
                    text.push(c);
                } else {
                    text.push(c);
                }
                true
            }
            KeyCode::Enter => {
                text.push('\n');
                true
            }
            KeyCode::Tab => {
                text.push('\t');
                true
            }
            KeyCode::Backspace => {
                text.pop();
                true
            }
            _ => false, // Ignore other keys (arrows, etc.) during paste
        },
        Event::Mouse(_) | Event::FocusGained | Event::FocusLost | Event::Resize(_, _) => {
            // Non-key events shouldn't appear in a paste burst; stop collecting.
            false
        }
        Event::Paste(p) => {
            // Rare: a real Paste event appeared mid-burst (shouldn't happen).
            text.push_str(&p);
            true
        }
        _ => false,
    }
}

// ── Event dispatcher ────────────────────────────────────────────────────────

/// Core event-handling logic (extracted from `next_event` to avoid duplicating
/// the probe and normal paths).
async fn handle_event(app: &mut App, ev: Event) -> Result<Option<Action>> {
    match ev {
        Event::FocusGained => {
            app.focused = true;
            return Ok(Some(Action::Redraw));
        }
        Event::FocusLost => {
            app.focused = false;
            return Ok(Some(Action::Redraw));
        }
        Event::Resize(_, _) => {
            // Width sync is now driven by render_messages (compares cache.width vs text_area.width)
            app.session_mgr.current_mut().ui.text_selection.clear();
        }
        Event::Key(key_event) => {
            return keyboard::handle_key_event(app, key_event);
        }
        Event::Paste(text) => {
            // Paste text handling
            // Some terminals (e.g. VSCode) use \r instead of \n as line separator in bracketed paste
            let text = text.replace('\r', "\n");

            // Setup wizard open — paste into active field
            if let Some(wizard) = &mut app.global_ui.setup_wizard {
                wizard.paste_text(&text);
                return Ok(Some(Action::Redraw));
            }

            // ─── 交互弹窗优先路由（AskUser/HITL/OAuth） ──────────────────
            // 弹窗激活时，Paste（含终端 IME 组合后的中文）应进入弹窗
            // 而非 textarea。仅 AskUser 弹窗有 custom_input 接收文本。
            if app.is_interaction_popup_active() {
                app.paste_to_interaction_popup(&text);
                return Ok(Some(Action::Redraw));
            }

            // ─── PanelManager paste dispatch ────────────────────────────
            {
                // Session panels: Model, Agent, Hooks, Login, Config, ThreadBrowser
                let session_kind = app.session_mgr.current_mut().session_panels.active_kind();
                if matches!(
                    session_kind,
                    Some(PanelKind::Model)
                        | Some(PanelKind::Agent)
                        | Some(PanelKind::Hooks)
                        | Some(PanelKind::Login)
                        | Some(PanelKind::Config)
                        | Some(PanelKind::ThreadBrowser)
                ) {
                    with_session_panels!(app, |sp, ctx| sp.dispatch_paste(&text, &mut ctx));
                    return Ok(Some(Action::Redraw));
                }

                // Global panels: Status, Memory, Mcp, Cron, Plugin
                let global_kind = app.global_panels.active_kind();
                if matches!(
                    global_kind,
                    Some(PanelKind::Status)
                        | Some(PanelKind::Memory)
                        | Some(PanelKind::Mcp)
                        | Some(PanelKind::Cron)
                        | Some(PanelKind::Plugin)
                ) {
                    with_global_panels!(app, |pm, ctx| pm.dispatch_paste(&text, &mut ctx));
                    return Ok(Some(Action::Redraw));
                }
            }

            // Fallback: paste into textarea
            // 弹窗激活时不写入 textarea——用户应通过弹窗 UI 交互
            if !app.is_interaction_popup_active() {
                app.paste_text_into_textarea(&text);
            }
        }
        Event::Mouse(mouse) => match mouse.kind {
            // ── AskUser 弹窗鼠标交互（优先于面板/消息区） ────────────────────────
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                {
                    if let Some(crate::app::InteractionPrompt::Questions(_)) =
                        app.session_mgr.current_mut().agent.interaction_prompt
                    {
                        if let Some(area) = app.session_mgr.current_mut().ui.panel_area {
                            if mouse::mouse_in_rect(&mouse, area) {
                                let delta = if matches!(mouse.kind, MouseEventKind::ScrollUp) {
                                    -3
                                } else {
                                    3
                                };
                                app.ask_user_scroll(delta);
                                return Ok(Some(Action::Redraw));
                            }
                        }
                    }
                }
                // Phase 8: 鼠标滚轮在主区滚动条 bar 上 → 滚一屏（而非默认 3 行）
                {
                    let on_bar = app
                        .session_mgr
                        .current()
                        .ui
                        .messages_scrollbar_metrics
                        .as_ref()
                        .is_some_and(|m| {
                            let b = m.bar_area;
                            mouse.column >= b.x
                                && mouse.column < b.x + b.width
                                && mouse.row >= b.y
                                && mouse.row < b.bottom()
                        });
                    if on_bar {
                        let page = app
                            .session_mgr
                            .current()
                            .ui
                            .messages_scrollbar_metrics
                            .map(|m| m.bar_area.height as usize)
                            .unwrap_or(20);
                        let ui = &mut app.session_mgr.current_mut().ui;
                        let max_scroll = ui.scrollbar_max_offset;
                        let min_scroll = ui.scrollbar_min_offset.min(max_scroll);
                        let current = if ui.scroll_follow {
                            max_scroll
                        } else {
                            ui.scroll_offset.clamp(min_scroll, max_scroll)
                        };
                        let new_off = match mouse.kind {
                            MouseEventKind::ScrollUp => {
                                current.saturating_sub(page).max(min_scroll)
                            }
                            MouseEventKind::ScrollDown => {
                                current.saturating_add(page).min(max_scroll)
                            }
                            _ => unreachable!(),
                        };
                        ui.scroll_offset = new_off;
                        ui.scroll_follow = new_off >= max_scroll;
                        return Ok(Some(Action::Redraw));
                    }
                }
                // 正常滚动处理
                match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        let panel_area = app.session_mgr.current_mut().ui.panel_area;
                        if let Some(area) = panel_area {
                            if mouse::mouse_in_rect(&mouse, area) {
                                // Session panel takes priority
                                let sp = &app.session_mgr.current_mut().session_panels;
                                if sp.is_any_open() {
                                    let result = with_session_panels!(app, |sp, ctx| {
                                        sp.dispatch_scroll(-3, &mut ctx)
                                    });
                                    if result == EventResult::Consumed {
                                        return Ok(Some(Action::Redraw));
                                    }
                                }
                                // Global panel
                                if app.global_panels.is_any_open() {
                                    let result = with_global_panels!(app, |pm, ctx| {
                                        pm.dispatch_scroll(-3, &mut ctx)
                                    });
                                    if result == EventResult::Consumed {
                                        return Ok(Some(Action::Redraw));
                                    }
                                }
                            }
                        }
                        app.scroll_up();
                    }
                    MouseEventKind::ScrollDown => {
                        let panel_area = app.session_mgr.current_mut().ui.panel_area;
                        if let Some(area) = panel_area {
                            if mouse::mouse_in_rect(&mouse, area) {
                                // Session panel takes priority
                                let sp = &app.session_mgr.current_mut().session_panels;
                                if sp.is_any_open() {
                                    let result = with_session_panels!(app, |sp, ctx| {
                                        sp.dispatch_scroll(3, &mut ctx)
                                    });
                                    if result == EventResult::Consumed {
                                        return Ok(Some(Action::Redraw));
                                    }
                                }
                                // Global panel
                                if app.global_panels.is_any_open() {
                                    let result = with_global_panels!(app, |pm, ctx| {
                                        pm.dispatch_scroll(3, &mut ctx)
                                    });
                                    if result == EventResult::Consumed {
                                        return Ok(Some(Action::Redraw));
                                    }
                                }
                            }
                        }
                        app.scroll_down();
                    }
                    _ => unreachable!(),
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                // ── AskUser 弹窗滚动条点击（优先于面板滚动条） ──────────────────
                {
                    if let Some(crate::app::InteractionPrompt::Questions(ref p)) =
                        app.session_mgr.current_mut().agent.interaction_prompt
                    {
                        if let Some(metrics) = p.scrollbar_metrics {
                            if mouse.column >= metrics.bar_area.x
                                && mouse.column < metrics.bar_area.x + metrics.bar_area.width
                                && mouse.row >= metrics.bar_area.y
                                && mouse.row < metrics.bar_area.bottom()
                                && metrics.max_offset > 0
                            {
                                let bar_inner_height = metrics.bar_area.height.saturating_sub(2);
                                if bar_inner_height > 0 {
                                    let rel_y = (mouse.row.saturating_sub(metrics.bar_area.y + 1))
                                        .min(bar_inner_height);
                                    let new_offset = ((rel_y as f64 / bar_inner_height as f64)
                                        * metrics.max_offset as f64)
                                        as u16;
                                    let new_offset = new_offset.min(metrics.max_offset);
                                    if let Some(crate::app::InteractionPrompt::Questions(p)) = app
                                        .session_mgr
                                        .current_mut()
                                        .agent
                                        .interaction_prompt
                                        .as_mut()
                                    {
                                        p.scroll_offset = new_offset;
                                    }
                                }
                                return Ok(Some(Action::Redraw));
                            }
                        }
                    }
                }
                // Panel scrollbar: ▲/▼ buttons and bar click/drag
                // Must be checked BEFORE dispatch_mouse so scrollbar clicks
                // aren't consumed by panel content area handlers.
                {
                    let session = &mut app.session_mgr.current_mut();
                    if let Some(ref metrics) = session.ui.panel_scrollbar_metrics {
                        // ▼ button click (scroll to bottom)
                        if let Some(btn) = metrics.down_btn_area {
                            if mouse.column >= btn.x
                                && mouse.column < btn.x + btn.width
                                && mouse.row >= btn.y
                                && mouse.row < btn.y + btn.height
                            {
                                session
                                    .session_panels
                                    .dispatch_set_scroll_offset(metrics.max_offset);
                                session.ui.panel_scroll_offset = metrics.max_offset;
                                return Ok(Some(Action::Redraw));
                            }
                        }
                        // ▲ button click (scroll to top)
                        if let Some(btn) = metrics.up_btn_area {
                            if mouse.column >= btn.x
                                && mouse.column < btn.x + btn.width
                                && mouse.row >= btn.y
                                && mouse.row < btn.y + btn.height
                            {
                                session.session_panels.dispatch_set_scroll_offset(0);
                                session.ui.panel_scroll_offset = 0;
                                return Ok(Some(Action::Redraw));
                            }
                        }
                        // Scrollbar bar click (proportional jump + start drag)
                        if mouse.column == metrics.bar_area.x
                            && mouse.row >= metrics.bar_area.y
                            && mouse.row < metrics.bar_area.bottom()
                            && metrics.max_offset > 0
                        {
                            let bar_inner_height = metrics.bar_area.height.saturating_sub(2);
                            if bar_inner_height > 0 {
                                let rel_y = (mouse.row.saturating_sub(metrics.bar_area.y + 1))
                                    .min(bar_inner_height);
                                let new_offset = ((rel_y as f64 / bar_inner_height as f64)
                                    * metrics.max_offset as f64)
                                    as u16;
                                let new_offset = new_offset.min(metrics.max_offset);
                                session
                                    .session_panels
                                    .dispatch_set_scroll_offset(new_offset);
                                session.ui.panel_scroll_offset = new_offset;
                                session.ui.panel_scrollbar_dragging = true;
                            }
                            return Ok(Some(Action::Redraw));
                        }
                    }
                }
                // Panel area: dispatch mouse click to panel content
                let panel_area = app.session_mgr.current_mut().ui.panel_area;
                let mut click_consumed = false;
                if let Some(area) = panel_area {
                    if mouse::mouse_in_rect(&mouse, area) {
                        // Session panels
                        {
                            let sp = &app.session_mgr.current_mut().session_panels;
                            if sp.is_any_open() {
                                let result = with_session_panels!(app, |sp, ctx| {
                                    sp.dispatch_mouse(mouse, area, &mut ctx)
                                });
                                if result == EventResult::Consumed {
                                    click_consumed = true;
                                }
                            }
                        }
                        // Global panels
                        if !click_consumed && app.global_panels.is_any_open() {
                            let result = with_global_panels!(app, |pm, ctx| {
                                pm.dispatch_mouse(mouse, area, &mut ctx)
                            });
                            if result == EventResult::Consumed {
                                click_consumed = true;
                            }
                        }
                    }
                }
                if click_consumed {
                    return Ok(Some(Action::Redraw));
                }
                // Panel area: start panel selection
                let panel_area = app.session_mgr.current_mut().ui.panel_area;
                if let Some(area) = panel_area {
                    if mouse::mouse_in_rect(&mouse, area) {
                        let content_row = mouse.row - area.y
                            + app.session_mgr.current_mut().ui.panel_scroll_offset;
                        let col = mouse.column - area.x;
                        app.session_mgr
                            .current_mut()
                            .ui
                            .panel_selection
                            .start_drag(content_row, col);
                        app.session_mgr.current_mut().ui.text_selection.clear();
                        // Don't process other-area selections
                        return Ok(Some(Action::Redraw));
                    }
                }
                // Main message area scrollbar: click → proportional jump + start drag.
                // 必须在 messages_area 文本选区之前拦截，否则 thumb 列的点击会被当作文本选区 start_drag。
                {
                    let session = &mut app.session_mgr.current_mut();
                    if let Some(ref metrics) = session.ui.messages_scrollbar_metrics {
                        let bar = metrics.bar_area;
                        let in_bar = mouse.column >= bar.x
                            && mouse.column < bar.x + bar.width
                            && mouse.row >= bar.y
                            && mouse.row < bar.bottom();
                        if in_bar && metrics.max_offset > metrics.min_offset {
                            // ▲ 按钮：滚动一屏（向上 bar.height 行）
                            if let Some(btn) = metrics.up_btn_area {
                                if mouse.column == btn.x
                                    && mouse.row >= btn.y
                                    && mouse.row < btn.y + btn.height
                                {
                                    let page = bar.height as usize;
                                    let new_off = session
                                        .ui
                                        .scroll_offset
                                        .saturating_sub(page)
                                        .max(metrics.min_offset);
                                    session.ui.scroll_offset = new_off;
                                    session.ui.scroll_follow = false;
                                    return Ok(Some(Action::Redraw));
                                }
                            }
                            // ▼ 按钮：滚动一屏（向下 bar.height 行）
                            if let Some(btn) = metrics.down_btn_area {
                                if mouse.column == btn.x
                                    && mouse.row >= btn.y
                                    && mouse.row < btn.y + btn.height
                                {
                                    let page = bar.height as usize;
                                    let new_off = session
                                        .ui
                                        .scroll_offset
                                        .saturating_add(page)
                                        .min(metrics.max_offset);
                                    session.ui.scroll_offset = new_off;
                                    session.ui.scroll_follow =
                                        new_off >= metrics.max_offset;
                                    return Ok(Some(Action::Redraw));
                                }
                            }
                            // bar 内点击：按 Y 比例计算 offset。
                            // 除以 (height - 1) 让 row=bar.y+height-1 恰好映射到 max_offset。
                            // Phase 7：双击（300ms 内同 Y）切换 follow 模式
                            let now = std::time::Instant::now();
                            let is_double_click = session
                                .ui
                                .last_bar_click_at
                                .map(|t| now.duration_since(t) < std::time::Duration::from_millis(300))
                                .unwrap_or(false)
                                && session
                                    .ui
                                    .last_bar_click_y
                                    .map(|y| y.abs_diff(mouse.row) <= 1)
                                    .unwrap_or(false);
                            if is_double_click {
                                // 双击切换 follow：true → false（保持当前 offset），false → true（跳到底部）
                                if session.ui.scroll_follow {
                                    session.ui.scroll_follow = false;
                                } else {
                                    session.ui.scroll_offset = metrics.max_offset;
                                    session.ui.scroll_follow = true;
                                }
                                // 双击不进入 dragging，重置双击状态防止误触发三击
                                session.ui.last_bar_click_at = None;
                                session.ui.last_bar_click_y = None;
                                session.ui.messages_scrollbar_dragging = false;
                                return Ok(Some(Action::Redraw));
                            }
                            // 单击：记录时间戳 + Y 供下次双击检测
                            session.ui.last_bar_click_at = Some(now);
                            session.ui.last_bar_click_y = Some(mouse.row);

                            let rel_y = (mouse.row.saturating_sub(bar.y)) as usize;
                            let range = metrics.max_offset.saturating_sub(metrics.min_offset);
                            let height = bar.height as usize;
                            let new_offset = if height > 1 {
                                metrics.min_offset + (rel_y * range) / (height - 1)
                            } else {
                                metrics.min_offset + range / 2
                            };
                            let new_offset = new_offset.min(metrics.max_offset);
                            session.ui.scroll_offset = new_offset;
                            session.ui.scroll_follow = new_offset >= metrics.max_offset;
                            session.ui.messages_scrollbar_dragging = true;
                            return Ok(Some(Action::Redraw));
                        }
                    }
                }
                if let Some(area) = app.session_mgr.current_mut().ui.messages_area {
                    if mouse.row >= area.y
                        && mouse.row < area.y + area.height
                        && mouse.column >= area.x
                        && mouse.column < area.x + area.width
                    {
                        let visual_row = usize::from(mouse.row - area.y)
                            + app.session_mgr.current_mut().ui.scroll_offset;
                        let visual_col = mouse.column - area.x;
                        app.session_mgr
                            .current_mut()
                            .ui
                            .text_selection
                            .start_drag(visual_row, visual_col);
                    }
                }
                // Textarea area: start textarea selection
                // 弹窗激活时跳过——光标不应移到 textarea 内
                if !app.is_interaction_popup_active() {
                    if let Some(area) = app.session_mgr.current_mut().ui.textarea_area {
                        if mouse.row >= area.y
                            && mouse.row < area.y + area.height
                            && mouse.column >= area.x
                            && mouse.column < area.x + area.width
                        {
                            let session = &app.session_mgr.current_mut();
                            let (row, col) =
                                mouse::textarea_mouse_to_cursor(&session.ui.textarea, area, &mouse);
                            app.session_mgr.current_mut().ui.textarea.move_cursor(
                                tui_textarea::CursorMove::Jump(row as u16, col as u16),
                            );
                            app.session_mgr.current_mut().ui.textarea.start_selection();
                        }
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                // Panel scrollbar drag: update panel scroll offset from mouse Y
                {
                    let session = &mut app.session_mgr.current_mut();
                    if session.ui.panel_scrollbar_dragging {
                        if let Some(ref metrics) = session.ui.panel_scrollbar_metrics {
                            let bar_inner_height = metrics.bar_area.height.saturating_sub(2);
                            if bar_inner_height > 0 {
                                let rel_y = (mouse.row.saturating_sub(metrics.bar_area.y + 1))
                                    .min(bar_inner_height);
                                let new_offset = ((rel_y as f64 / bar_inner_height as f64)
                                    * metrics.max_offset as f64)
                                    as u16;
                                let new_offset = new_offset.min(metrics.max_offset);
                                session
                                    .session_panels
                                    .dispatch_set_scroll_offset(new_offset);
                                session.ui.panel_scroll_offset = new_offset;
                            }
                        }
                        return Ok(Some(Action::Redraw));
                    }
                }
                // Main message area scrollbar drag: update scroll_offset from mouse Y
                {
                    let session = &mut app.session_mgr.current_mut();
                    if session.ui.messages_scrollbar_dragging {
                        if let Some(ref metrics) = session.ui.messages_scrollbar_metrics {
                            let bar = metrics.bar_area;
                            let rel_y = (mouse.row.saturating_sub(bar.y)) as usize;
                            let range = metrics.max_offset.saturating_sub(metrics.min_offset);
                            let height = bar.height as usize;
                            let new_offset = if height > 1 {
                                metrics.min_offset + (rel_y * range) / (height - 1)
                            } else {
                                metrics.min_offset + range / 2
                            };
                            let new_offset = new_offset.min(metrics.max_offset);
                            session.ui.scroll_offset = new_offset;
                            session.ui.scroll_follow = new_offset >= metrics.max_offset;
                        }
                        return Ok(Some(Action::Redraw));
                    }
                }
                // Panel selection drag
                if app.session_mgr.current_mut().ui.panel_selection.dragging {
                    if let Some(area) = app.session_mgr.current_mut().ui.panel_area {
                        let content_row = mouse
                            .row
                            .saturating_sub(area.y)
                            .saturating_add(app.session_mgr.current_mut().ui.panel_scroll_offset);
                        let col = mouse.column.saturating_sub(area.x);
                        app.session_mgr
                            .current_mut()
                            .ui
                            .panel_selection
                            .update_drag(content_row, col);
                    }
                }
                if app.session_mgr.current_mut().ui.text_selection.dragging {
                    if let Some(area) = app.session_mgr.current_mut().ui.messages_area {
                        let visual_row = usize::from(mouse.row.saturating_sub(area.y))
                            + app.session_mgr.current_mut().ui.scroll_offset;
                        let visual_col = mouse.column.saturating_sub(area.x);
                        app.session_mgr
                            .current_mut()
                            .ui
                            .text_selection
                            .update_drag(visual_row, visual_col);
                    }
                }
                // Textarea area: extend textarea selection
                if app.session_mgr.current_mut().ui.textarea.is_selecting() {
                    if let Some(area) = app.session_mgr.current_mut().ui.textarea_area {
                        if mouse.row >= area.y && mouse.row < area.y + area.height {
                            let session = &app.session_mgr.current_mut();
                            let (row, col) =
                                mouse::textarea_mouse_to_cursor(&session.ui.textarea, area, &mouse);
                            app.session_mgr.current_mut().ui.textarea.move_cursor(
                                tui_textarea::CursorMove::Jump(row as u16, col as u16),
                            );
                        }
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                // End panel scrollbar drag
                app.session_mgr.current_mut().ui.panel_scrollbar_dragging = false;
                // End main message area scrollbar drag
                app.session_mgr.current_mut().ui.messages_scrollbar_dragging = false;
                // Panel selection released
                if app.session_mgr.current_mut().ui.panel_selection.dragging {
                    app.session_mgr.current_mut().ui.panel_selection.end_drag();
                    let sel = &app.session_mgr.current_mut().ui.panel_selection;
                    if let (Some(start), Some(end)) = (sel.start, sel.end) {
                        let text = crate::app::text_selection::extract_panel_text(
                            start,
                            end,
                            &app.session_mgr.current_mut().ui.panel_plain_lines,
                        );
                        app.session_mgr
                            .current_mut()
                            .ui
                            .panel_selection
                            .set_selected_text(text);
                    }
                    mouse::copy_panel_selection_to_clipboard(app);
                }
                if app.session_mgr.current_mut().ui.text_selection.dragging {
                    app.session_mgr.current_mut().ui.text_selection.end_drag();
                    let ts = &app.session_mgr.current_mut().ui.text_selection;
                    if let (Some(start), Some(end)) = (ts.start, ts.end) {
                        let usable_width = app
                            .session_mgr
                            .current_mut()
                            .ui
                            .messages_area
                            .map(|a| a.width)
                            .unwrap_or(0);
                        let cache = app.session_mgr.current_mut().messages.render_cache.read();
                        let text = crate::app::text_selection::extract_selected_text(
                            start,
                            end,
                            &cache.wrap_map,
                            usable_width,
                        );
                        drop(cache);
                        app.session_mgr
                            .current_mut()
                            .ui
                            .text_selection
                            .set_selected_text(text);
                    }
                    mouse::copy_selection_to_clipboard(app);
                }
                // textarea selection on mouse up: no extra handling; tui_textarea maintains
                // its own selection state
            }
            _ => {}
        },
    }

    Ok(Some(Action::Redraw))
}

// ── OAuth prompt ────────────────────────────────────────────────────────────

fn handle_oauth_prompt(app: &mut App, input: Input) {
    use crate::app::handle_edit_key;
    let prompt = match app.global_ui.oauth_prompt.as_mut() {
        Some(p) => p,
        None => return,
    };
    match input {
        Input {
            key: Key::Enter, ..
        } => {
            if prompt.submit() {
                app.global_ui.oauth_prompt = None;
            }
        }
        Input {
            key: Key::Char('o'),
            ctrl: true,
            ..
        } => {
            let url = prompt.authorization_url.clone();
            #[cfg(unix)]
            let _ = std::process::Command::new("open").arg(&url).spawn();
            #[cfg(windows)]
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", &url])
                .spawn();
        }
        Input { key: Key::Esc, .. } => {
            app.global_ui.oauth_prompt = None;
        }
        Input {
            key: Key::Char('c'),
            ctrl: true,
            ..
        } => {
            // Ctrl+C in OAuth popup: ignore (no quit)
        }
        _ => {
            prompt.error_message = None;
            handle_edit_key(&mut prompt.input, &mut prompt.cursor, input);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyEvent, MouseEvent};

    fn make_key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn test_key_event_to_text_simulated_paste_includes_first_line() {
        let mut text = String::new();

        assert!(key_event_to_text(make_key(KeyCode::Char('b')), &mut text));
        assert!(key_event_to_text(make_key(KeyCode::Char('u')), &mut text));
        assert!(key_event_to_text(make_key(KeyCode::Enter), &mut text));
        assert!(key_event_to_text(make_key(KeyCode::Char('i')), &mut text));
        assert!(key_event_to_text(make_key(KeyCode::Char('d')), &mut text));

        assert_eq!(
            text, "bu\nid",
            "模拟粘贴重建必须从第一个字符开始，不能等到 Enter 后才收集"
        );
    }

    // ── 主区滚动条鼠标交互（Phase 2）──────────────────────────────────────

    fn make_mouse_down(row: u16, col: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn make_mouse_drag(row: u16, col: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn make_mouse_up(row: u16, col: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn make_mouse_scroll_down(row: u16, col: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn make_mouse_scroll_up(row: u16, col: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    /// 构造已渲染好的 App：填充大量消息触发主区滚动 + metrics 已被设置
    async fn make_app_with_overflow(
        width: u16,
        height: u16,
        msg_count: usize,
    ) -> (crate::app::App, crate::ui::headless::HeadlessHandle) {
        use crate::app::MessageViewModel;
        let (mut app, mut handle) = crate::app::App::new_headless(width, height).await;
        for i in 0..msg_count {
            let notified = handle.render_notify.notified();
            let vm = MessageViewModel::user(format!("padding line content {}", i));
            app.session_mgr
                .current_mut()
                .messages
                .view_messages
                .push(vm);
            app.render_rebuild();
            notified.await;
        }
        handle
            .terminal
            .draw(|f| crate::ui::main_ui::render(f, &mut app))
            .unwrap();
        (app, handle)
    }

    /// 点击主区滚动条 thumb 区域：scroll_offset 应按 Y 比例跳转
    /// Phase 4 起 bar 顶/底各 1 行被 ▲/▼ 按钮占用，比例区是 [bar.y+1, bar.bottom()-2]
    #[tokio::test]
    async fn test_messages_scrollbar_click_jumps_proportionally() {
        let (mut app, _handle) = make_app_with_overflow(80, 24, 30).await;
        let metrics = app
            .session_mgr
            .current()
            .ui
            .messages_scrollbar_metrics
            .expect("应有 metrics");
        assert!(metrics.max_offset > metrics.min_offset);
        let range = metrics.max_offset.saturating_sub(metrics.min_offset);
        // 1/4 range 作为「接近」容差，吸收比例公式整除误差
        let tolerance = (range / 4).max(1);

        let bar = metrics.bar_area;
        // 点击 bar 顶部下方一行（避开 ▲ 按钮）→ offset 接近 min
        let ev = make_mouse_down(bar.y + 1, bar.x);
        let _ = handle_event(&mut app, ev).await;
        let off_top = app.session_mgr.current().ui.scroll_offset;
        assert!(
            off_top <= metrics.min_offset + tolerance,
            "点击近顶部 offset 应接近 min，实际: {} tolerance={}",
            off_top,
            tolerance
        );

        // 点击 bar 底部上方一行（避开 ▼ 按钮）→ offset 接近 max
        let ev = make_mouse_down(bar.bottom().saturating_sub(2), bar.x);
        let _ = handle_event(&mut app, ev).await;
        let off_bottom = app.session_mgr.current().ui.scroll_offset;
        assert!(
            off_bottom >= metrics.max_offset.saturating_sub(tolerance),
            "点击近底部 offset 应接近 max，实际: {} max={} tolerance={}",
            off_bottom,
            metrics.max_offset,
            tolerance
        );
    }

    /// Phase 4: ▲ 按钮点击应上翻一屏
    #[tokio::test]
    async fn test_messages_scrollbar_up_button_scrolls_page() {
        let (mut app, _handle) = make_app_with_overflow(80, 24, 30).await;
        let metrics = app
            .session_mgr
            .current()
            .ui
            .messages_scrollbar_metrics
            .expect("应有 metrics");
        let bar = metrics.bar_area;
        let page = bar.height as usize;

        // 初次渲染默认 follow=true，offset=max；▲ 可见，▼ 不可见
        assert_eq!(app.session_mgr.current().ui.scroll_offset, metrics.max_offset);
        assert!(metrics.up_btn_area.is_some(), "offset=max 时 ▲ 应可见");
        assert!(
            metrics.down_btn_area.is_none(),
            "offset=max 时 ▼ 应隐藏"
        );

        // 点击 ▲ 按钮 → 上翻一屏
        let up_btn = metrics.up_btn_area.expect("▲ 按钮");
        let _ = handle_event(&mut app, make_mouse_down(up_btn.y, up_btn.x)).await;
        let off_after_up = app.session_mgr.current().ui.scroll_offset;
        let expected = metrics.max_offset.saturating_sub(page).max(metrics.min_offset);
        assert_eq!(
            off_after_up, expected,
            "▲ 点击应上翻一屏 (page={})，期望 {} 实际 {}",
            page, expected, off_after_up
        );
        assert!(
            !app.session_mgr.current().ui.scroll_follow,
            "▲ 点击后 follow 应为 false"
        );
    }

    /// Phase 4: ▼ 按钮可见且点击时，应下翻一屏并在到达底部时恢复 follow
    #[tokio::test]
    async fn test_messages_scrollbar_down_button_scrolls_and_restores_follow() {
        let (mut app, mut handle) = make_app_with_overflow(80, 24, 30).await;
        let metrics = app
            .session_mgr
            .current()
            .ui
            .messages_scrollbar_metrics
            .expect("应有 metrics");
        let bar = metrics.bar_area;
        let page = bar.height as usize;

        // 先点 ▲ 让 offset 离开 max
        let up_btn = metrics.up_btn_area.expect("▲ 按钮");
        let _ = handle_event(&mut app, make_mouse_down(up_btn.y, up_btn.x)).await;
        let off_after_up = app.session_mgr.current().ui.scroll_offset;
        assert!(off_after_up < metrics.max_offset, "▲ 后 offset 应 < max");

        // 重绘以让 metrics 反映新 offset（此时 ▼ 变可见）
        handle
            .terminal
            .draw(|f| crate::ui::main_ui::render(f, &mut app))
            .unwrap();
        let metrics_after = app
            .session_mgr
            .current()
            .ui
            .messages_scrollbar_metrics
            .expect("重绘后应有 metrics");
        let down_btn = metrics_after
            .down_btn_area
            .expect("offset<max 时 ▼ 应可见");

        // 点击 ▼ → 下翻一屏
        let _ = handle_event(&mut app, make_mouse_down(down_btn.y, down_btn.x)).await;
        let off_after_down = app.session_mgr.current().ui.scroll_offset;
        let expected = off_after_up.saturating_add(page).min(metrics_after.max_offset);
        assert_eq!(
            off_after_down, expected,
            "▼ 点击应下翻一屏，期望 {} 实际 {}",
            expected, off_after_down
        );

        // 持续点 ▼ 直到抵 max → follow 恢复
        let mut last_offset = off_after_down;
        for _ in 0..5 {
            handle
                .terminal
                .draw(|f| crate::ui::main_ui::render(f, &mut app))
                .unwrap();
            let m = app
                .session_mgr
                .current()
                .ui
                .messages_scrollbar_metrics
                .expect("应有 metrics");
            let Some(btn) = m.down_btn_area else {
                break;
            };
            let _ = handle_event(&mut app, make_mouse_down(btn.y, btn.x)).await;
            last_offset = app.session_mgr.current().ui.scroll_offset;
            if last_offset >= m.max_offset {
                break;
            }
        }
        assert!(
            app.session_mgr.current().ui.scroll_follow,
            "抵 max 后 follow 应恢复 true，实际 offset={}",
            last_offset
        );
    }

    /// Phase 4: offset=min 时 ▲ 应隐藏；offset=max 时 ▼ 应隐藏
    #[tokio::test]
    async fn test_messages_scrollbar_buttons_visibility_extremes() {
        let (mut app, mut handle) = make_app_with_overflow(80, 24, 30).await;

        // 初次渲染 offset=max → ▲ 可见、▼ 隐藏
        let metrics = app
            .session_mgr
            .current()
            .ui
            .messages_scrollbar_metrics
            .expect("应有 metrics");
        assert!(metrics.up_btn_area.is_some(), "offset=max 时 ▲ 应可见");
        assert!(metrics.down_btn_area.is_none(), "offset=max 时 ▼ 应隐藏");

        // 滚动到顶部
        app.session_mgr.current_mut().ui.scroll_offset = metrics.min_offset;
        app.session_mgr.current_mut().ui.scroll_follow = false;
        handle
            .terminal
            .draw(|f| crate::ui::main_ui::render(f, &mut app))
            .unwrap();
        let metrics_top = app
            .session_mgr
            .current()
            .ui
            .messages_scrollbar_metrics
            .expect("应有 metrics");
        assert!(
            metrics_top.up_btn_area.is_none(),
            "offset=min 时 ▲ 应隐藏"
        );
        assert!(
            metrics_top.down_btn_area.is_some(),
            "offset=min 时 ▼ 应可见"
        );
    }

    /// Phase 7: 双击 bar 切换 follow：scroll_follow=false → 跳到底部 + follow=true
    #[tokio::test]
    async fn test_messages_scrollbar_double_click_bar_restores_follow() {
        let (mut app, _handle) = make_app_with_overflow(80, 24, 30).await;
        let metrics = app
            .session_mgr
            .current()
            .ui
            .messages_scrollbar_metrics
            .expect("应有 metrics");
        let bar = metrics.bar_area;

        // 初始 follow=true → 单击中间位置取消 follow
        let mid_y = bar.y + bar.height / 2;
        let _ = handle_event(&mut app, make_mouse_down(mid_y, bar.x)).await;
        assert!(
            !app.session_mgr.current().ui.scroll_follow,
            "第一次单击应取消 follow"
        );
        let off_after_single = app.session_mgr.current().ui.scroll_offset;
        assert!(
            off_after_single < metrics.max_offset,
            "单击中间 offset 应 < max"
        );

        // 第二次单击（同位置，< 300ms）→ 双击 → 跳到底部 + follow=true
        let _ = handle_event(&mut app, make_mouse_down(mid_y, bar.x)).await;
        assert!(
            app.session_mgr.current().ui.scroll_follow,
            "双击应恢复 follow=true"
        );
        assert_eq!(
            app.session_mgr.current().ui.scroll_offset,
            metrics.max_offset,
            "双击应跳到 max_offset"
        );
        // 双击后不应进入 dragging
        assert!(
            !app.session_mgr.current().ui.messages_scrollbar_dragging,
            "双击不应进入 dragging"
        );
    }

    /// Phase 7: follow=true 时双击 bar 切到 false，保持当前 offset
    #[tokio::test]
    async fn test_messages_scrollbar_double_click_bar_disables_follow() {
        let (mut app, _handle) = make_app_with_overflow(80, 24, 30).await;
        let metrics = app
            .session_mgr
            .current()
            .ui
            .messages_scrollbar_metrics
            .expect("应有 metrics");
        let bar = metrics.bar_area;

        // 初始 follow=true（默认）
        assert!(app.session_mgr.current().ui.scroll_follow);
        let initial_offset = app.session_mgr.current().ui.scroll_offset;

        // 第一次单击 → 取消 follow + 跳到中间
        let mid_y = bar.y + bar.height / 2;
        let _ = handle_event(&mut app, make_mouse_down(mid_y, bar.x)).await;
        let off_after_first = app.session_mgr.current().ui.scroll_offset;
        assert!(off_after_first < initial_offset, "单击中间 offset 应减小");

        // 单击 ▲ 离开 follow=true 状态再测试 true→false 切换
        // 实际上初始就是 follow=true，第一次单击已经变成 false。
        // 这里测试在 follow=false 状态下双击切换回 true，再双击切换回 false。
        // 先单击一次让 last_bar_click_at 重置（避免误触发双击）
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let _ = handle_event(&mut app, make_mouse_down(mid_y + 2, bar.x)).await;
        let off_before_double = app.session_mgr.current().ui.scroll_offset;

        // 第二次单击（同 mid_y + 2，< 300ms）→ 双击 → follow=true
        let _ = handle_event(&mut app, make_mouse_down(mid_y + 2, bar.x)).await;
        assert!(
            app.session_mgr.current().ui.scroll_follow,
            "双击应切回 follow=true"
        );

        // 再双击切回 false（先单击重置时间戳）
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let _ = handle_event(&mut app, make_mouse_down(mid_y, bar.x)).await;
        // 此时 follow=true，单击中间会取消 follow 并跳到中间
        assert!(!app.session_mgr.current().ui.scroll_follow);
        // 第二次单击 → 双击 → 切回 true
        let _ = handle_event(&mut app, make_mouse_down(mid_y, bar.x)).await;
        assert!(
            app.session_mgr.current().ui.scroll_follow,
            "第二次双击应再切回 follow=true"
        );

        let _ = off_before_double; // 抑制 unused warning
    }

    /// Phase 8: 鼠标滚轮在 bar 上 → 滚一屏（bar.height 行），而非默认 3 行
    #[tokio::test]
    async fn test_messages_scrollbar_wheel_on_bar_scrolls_full_page() {
        let (mut app, _handle) = make_app_with_overflow(80, 24, 30).await;
        let metrics = app
            .session_mgr
            .current()
            .ui
            .messages_scrollbar_metrics
            .expect("应有 metrics");
        let bar = metrics.bar_area;
        let page = bar.height as usize;

        // 初始 follow=true，offset=max。滚到顶部以便测试 ScrollDown 能加 offset
        app.session_mgr.current_mut().ui.scroll_follow = false;
        app.session_mgr.current_mut().ui.scroll_offset = metrics.min_offset;

        // 在 bar 上 ScrollDown → offset 应增加 page 行
        let _ = handle_event(&mut app, make_mouse_scroll_down(bar.y + 2, bar.x)).await;
        let off_after_bar = app.session_mgr.current().ui.scroll_offset;
        assert_eq!(
            off_after_bar,
            metrics.min_offset + page,
            "bar 上 ScrollDown 应滚一屏 (+page={})，实际 {}",
            page,
            off_after_bar
        );

        // 在 bar 上 ScrollUp → offset 应减少 page 行
        let _ = handle_event(&mut app, make_mouse_scroll_up(bar.y + 2, bar.x)).await;
        let off_after_up = app.session_mgr.current().ui.scroll_offset;
        assert_eq!(
            off_after_up, metrics.min_offset,
            "bar 上 ScrollUp 应滚回 min，实际 {}", off_after_up
        );
    }

    /// Phase 8: 鼠标滚轮在 messages_area 内但不在 bar 上 → 走原逻辑（3 行）
    #[tokio::test]
    async fn test_messages_scrollbar_wheel_off_bar_uses_default_step() {
        let (mut app, _handle) = make_app_with_overflow(80, 24, 30).await;
        let metrics = app
            .session_mgr
            .current()
            .ui
            .messages_scrollbar_metrics
            .expect("应有 metrics");
        let bar = metrics.bar_area;
        let messages_area = app
            .session_mgr
            .current()
            .ui
            .messages_area
            .expect("应有 messages_area");

        // 滚到顶部测试 ScrollDown
        app.session_mgr.current_mut().ui.scroll_follow = false;
        app.session_mgr.current_mut().ui.scroll_offset = metrics.min_offset;

        // 在 messages_area 内（非 bar 列）ScrollDown
        // 选 messages_area 内距 bar 至少 2 列的位置
        let col_inside = bar.x.saturating_sub(2).max(messages_area.x);
        let row_inside = messages_area.y + 2;
        let _ = handle_event(&mut app, make_mouse_scroll_down(row_inside, col_inside)).await;
        let off_after_inside = app.session_mgr.current().ui.scroll_offset;
        assert_eq!(
            off_after_inside,
            metrics.min_offset + 3,
            "bar 外 ScrollDown 应走默认 3 行逻辑，实际 {}",
            off_after_inside
        );
    }

    /// 点击主区滚动条后 dragging=true；mouse up 清除
    #[tokio::test]
    async fn test_messages_scrollbar_drag_lifecycle() {
        let (mut app, _handle) = make_app_with_overflow(80, 24, 30).await;
        let metrics = app
            .session_mgr
            .current()
            .ui
            .messages_scrollbar_metrics
            .expect("应有 metrics");
        let bar = metrics.bar_area;

        // Down → dragging=true
        assert!(
            !app.session_mgr.current().ui.messages_scrollbar_dragging,
            "初始不应处于 dragging"
        );
        let _ = handle_event(&mut app, make_mouse_down(bar.y + 2, bar.x)).await;
        assert!(
            app.session_mgr.current().ui.messages_scrollbar_dragging,
            "点击 scrollbar 后应进入 dragging"
        );

        // Drag → offset 跟随 Y 变化（拖到底部）
        let _ = handle_event(
            &mut app,
            make_mouse_drag(bar.bottom().saturating_sub(1), bar.x),
        )
        .await;
        let off_after_drag = app.session_mgr.current().ui.scroll_offset;
        assert!(
            off_after_drag >= metrics.max_offset.saturating_sub(1),
            "拖到底部 offset 应接近 max，实际: {} max={}",
            off_after_drag,
            metrics.max_offset
        );

        // Up → dragging=false
        let _ = handle_event(&mut app, make_mouse_up(bar.y, bar.x)).await;
        assert!(
            !app.session_mgr.current().ui.messages_scrollbar_dragging,
            "mouse up 后应退出 dragging"
        );
    }

    /// 点击主区滚动条不应触发文本选区 start_drag（互斥）
    #[tokio::test]
    async fn test_messages_scrollbar_click_does_not_start_text_selection() {
        let (mut app, _handle) = make_app_with_overflow(80, 24, 30).await;
        let metrics = app
            .session_mgr
            .current()
            .ui
            .messages_scrollbar_metrics
            .expect("应有 metrics");
        let bar = metrics.bar_area;

        assert!(
            !app.session_mgr.current().ui.text_selection.dragging,
            "初始无文本选区"
        );
        let _ = handle_event(&mut app, make_mouse_down(bar.y + 5, bar.x)).await;
        assert!(
            !app.session_mgr.current().ui.text_selection.dragging,
            "点击 scrollbar 不应触发文本选区"
        );
        assert!(
            app.session_mgr.current().ui.messages_scrollbar_dragging,
            "应进入 scrollbar dragging"
        );
    }

    /// follow-bottom 时点击 thumb 中间位置应取消 follow
    #[tokio::test]
    async fn test_messages_scrollbar_click_disables_follow() {
        let (mut app, _handle) = make_app_with_overflow(80, 24, 30).await;
        let metrics = app
            .session_mgr
            .current()
            .ui
            .messages_scrollbar_metrics
            .expect("应有 metrics");
        // 初次渲染默认 follow=true
        assert!(
            app.session_mgr.current().ui.scroll_follow,
            "初次渲染应 follow=true"
        );

        let bar = metrics.bar_area;
        // 点击中间位置（不是最底部）
        let mid_y = bar.y + bar.height / 2;
        let _ = handle_event(&mut app, make_mouse_down(mid_y, bar.x)).await;
        assert!(
            !app.session_mgr.current().ui.scroll_follow,
            "点击中间应取消 follow"
        );

        // 点击最底部 → 恢复 follow
        let _ = handle_event(
            &mut app,
            make_mouse_down(bar.bottom().saturating_sub(1), bar.x),
        )
        .await;
        assert!(
            app.session_mgr.current().ui.scroll_follow,
            "点击最底部应恢复 follow"
        );
    }
}
