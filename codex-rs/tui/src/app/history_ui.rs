//! Terminal history and clear-screen UI helpers for the TUI app.
//!
//! This module owns rendering the fresh session header, clearing inline or alternate-screen UI
//! state, and resetting transcript-related app state after `/clear` or Ctrl-L.

use super::*;

impl App {
    pub(super) fn insert_history_cell(&mut self, tui: &mut tui::Tui, cell: Box<dyn HistoryCell>) {
        let cell: Arc<dyn HistoryCell> = cell.into();
        if let Some(Overlay::Transcript(t)) = &mut self.overlay {
            t.insert_cell(cell.clone());
            tui.frame_requester().schedule_frame();
        }
        self.chat_widget.transcript_cells.push(cell.clone());
        self.owned_screen_push_cell(cell.clone());
        if self.has_owned_screen() {
            self.chat_widget.request_pending_usage_output_insertion();
            if !self.owned_screen_replay_in_progress() {
                tui.frame_requester().schedule_frame();
            }
            return;
        }
        let width = self
            .chat_widget
            .history_wrap_width(tui.terminal.last_known_screen_size.width);
        let appended_cell_touches_compact_group =
            self.appended_cell_touches_compact_tool_group(width);
        if self
            .chat_widget
            .initial_history_replay_buffer
            .as_ref()
            .is_some()
        {
            self.insert_history_cell_lines_with_initial_replay_buffer(tui, cell.as_ref(), width);
        } else if appended_cell_touches_compact_group && self.overlay.is_none() {
            if let Err(err) = self.reflow_transcript_now(tui) {
                tracing::warn!(
                    error = %err,
                    "failed to reflow transcript after compact tool group append"
                );
            }
        } else {
            self.insert_history_cell_lines(tui, cell.as_ref(), width);
        }
        // A committed cell can unblock a settled /usage card that was waiting
        // behind a transient active cell or a provisional stream tail.
        self.chat_widget.request_pending_usage_output_insertion();
    }

    pub(super) fn promote_background_terminal_lifecycle(
        &mut self,
        tui: &mut tui::Tui,
        call_id: &str,
        cell: Box<dyn HistoryCell>,
    ) -> Result<()> {
        let cell: Arc<dyn HistoryCell> = cell.into();
        promote_background_terminal_cell(&mut self.chat_widget.transcript_cells, call_id, cell);
        if let Some(Overlay::Transcript(overlay)) = &mut self.overlay {
            overlay.replace_cells(self.chat_widget.transcript_cells.clone());
        }
        if self.has_owned_screen() {
            self.sync_owned_screen_cells();
        }
        self.refresh_lifecycle_history(tui)
    }

    pub(super) fn refresh_lifecycle_history(&mut self, tui: &mut tui::Tui) -> Result<()> {
        if self.has_owned_screen() {
            tui.frame_requester().schedule_frame();
            return Ok(());
        }
        if self.defer_lifecycle_refresh_during_replay() {
            return Ok(());
        }
        if self.overlay.is_some() {
            self.schedule_lifecycle_history_reflow(tui);
            return Ok(());
        }
        self.reflow_transcript_now(tui)?;
        tui.frame_requester().schedule_frame();
        Ok(())
    }

    pub(super) fn pending_usage_output_insertion_blocked(&self) -> bool {
        self.chat_widget.usage_history_insertion_blocked()
            || self
                .chat_widget
                .transcript_cells
                .last()
                .is_some_and(|cell| cell.as_any().is::<history_cell::AgentMessageCell>())
    }

    fn insert_pending_usage_output(&mut self, tui: &mut tui::Tui) {
        if let Some(cell) = self.chat_widget.take_completed_token_activity_output() {
            self.insert_history_cell(tui, Box::new(cell));
        }
        if let Some(cell) = self.chat_widget.take_pending_rate_limit_reset_hint() {
            self.insert_history_cell(tui, Box::new(cell));
        }
    }

    pub(super) fn insert_pending_usage_output_if_ready(&mut self, tui: &mut tui::Tui) {
        if self.pending_usage_output_insertion_blocked() {
            return;
        }
        self.insert_pending_usage_output(tui);
    }

    pub(super) fn insert_pending_usage_output_after_stream_shutdown(&mut self, tui: &mut tui::Tui) {
        if self.chat_widget.usage_history_insertion_blocked() {
            return;
        }
        self.insert_pending_usage_output(tui);
    }

    pub(super) fn open_url_in_browser(&mut self, url: String) {
        if let Err(err) = webbrowser::open(&url) {
            self.chat_widget
                .add_error_message(format!("Failed to open browser for {url}: {err}"));
            return;
        }

        self.chat_widget
            .add_info_message(format!("Opened {url} in your browser."), /*hint*/ None);
    }

    pub(super) fn clear_ui_header_lines_with_version(
        &self,
        width: u16,
        version: &'static str,
    ) -> Vec<Line<'static>> {
        self.clear_ui_header_cell(version).display_lines(width)
    }

    fn clear_ui_header_cell(
        &self,
        version: &'static str,
    ) -> history_cell::SessionHeaderHistoryCell {
        history_cell::SessionHeaderHistoryCell::new(
            self.chat_widget.current_model().to_string(),
            self.chat_widget.current_reasoning_effort(),
            self.chat_widget.should_show_fast_status(
                self.chat_widget.current_model(),
                self.chat_widget.current_service_tier(),
            ),
            self.config.cwd.to_path_buf(),
            version,
        )
        .with_yolo_mode(history_cell::is_yolo_mode(&self.config))
    }

    pub(super) fn clear_ui_header_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.clear_ui_header_lines_with_version(width, crate::version::CODEX_CLI_DISPLAY_VERSION)
    }

    pub(super) fn queue_clear_ui_header(&mut self, tui: &mut tui::Tui) {
        if self.has_owned_screen() {
            let cell: Arc<dyn HistoryCell> =
                Arc::new(self.clear_ui_header_cell(crate::version::CODEX_CLI_DISPLAY_VERSION));
            self.chat_widget.transcript_cells.push(cell.clone());
            self.owned_screen_push_cell(cell);
            tui.frame_requester().schedule_frame();
            return;
        }
        let width = self
            .chat_widget
            .history_wrap_width(tui.terminal.last_known_screen_size.width);
        let header_lines = self.clear_ui_header_lines(width);
        if !header_lines.is_empty() {
            tui.insert_history_lines(header_lines);
            self.has_emitted_history_lines = true;
        }
    }

    pub(super) fn clear_terminal_ui(
        &mut self,
        tui: &mut tui::Tui,
        redraw_header: bool,
    ) -> Result<()> {
        let is_alt_screen_active = tui.is_alt_screen_active();

        // Drop queued history insertions so stale transcript lines cannot be flushed after /clear.
        tui.clear_pending_history_lines();

        if is_alt_screen_active {
            tui.terminal.clear_visible_screen()?;
        } else {
            // Some terminals (Terminal.app, Warp) do not reliably drop scrollback when purge and
            // clear are emitted as separate backend commands. Prefer a single ANSI sequence.
            tui.terminal.clear_scrollback_and_visible_screen_ansi()?;
        }

        let mut area = tui.terminal.viewport_area;
        if area.y > 0 {
            // After a full clear, anchor the inline viewport at the top and redraw a fresh header
            // box. `insert_history_lines()` will shift the viewport down by the rendered height.
            area.y = 0;
            tui.terminal.set_viewport_area(area);
        }
        self.has_emitted_history_lines = false;

        if redraw_header {
            self.queue_clear_ui_header(tui);
        }
        Ok(())
    }

    pub(super) fn reset_app_ui_state_after_clear(&mut self, tui: &mut tui::Tui) {
        if self.overlay.is_some()
            && let Err(err) = tui.leave_alt_screen()
        {
            tracing::warn!(error = %err, "failed to release transcript overlay while clearing UI");
        }
        self.reset_transcript_state_after_clear();
    }

    pub(super) fn reset_transcript_state_after_clear(&mut self) {
        self.overlay = None;
        self.chat_widget.transcript_cells.clear();
        self.sync_owned_screen_cells();
        self.finish_owned_screen_replay();
        self.deferred_history_lines.clear();
        self.has_emitted_history_lines = false;
        self.chat_widget.transcript_reflow.clear();
        self.chat_widget.clear_pending_token_activity_refreshes();
        self.chat_widget.clear_pending_rate_limit_reset_hint();
        self.chat_widget.initial_history_replay_buffer = None;
        self.backtrack = BacktrackState::default();
        self.backtrack_render_pending = false;
        self.skill_load_warnings.clear();
    }
}

fn promote_background_terminal_cell(
    transcript_cells: &mut Vec<Arc<dyn HistoryCell>>,
    call_id: &str,
    cell: Arc<dyn HistoryCell>,
) {
    if let Some(index) = transcript_cells.iter().position(|candidate| {
        candidate
            .as_any()
            .downcast_ref::<crate::exec_cell::ExecCell>()
            .is_some_and(|exec| exec.is_single_call(call_id))
    }) {
        transcript_cells[index] = cell;
    } else {
        transcript_cells.push(cell);
    }
}

#[cfg(test)]
#[path = "history_ui_tests.rs"]
mod tests;
