//! Hook run lifecycle handling for `ChatWidget`.
//!
//! This module keeps active hook cells, hook timers, and hook completion output
//! together.

use super::*;

impl ChatWidget {
    /// Drop transient live hook status without flushing it into history.
    pub(super) fn clear_active_hook_cell(&mut self) {
        if self.active_hook_cell.take().is_some() {
            self.bottom_pane.set_hook_status(None);
            self.request_pending_usage_output_insertion();
        }
    }

    pub(super) fn on_hook_started(&mut self, run: codex_app_server_protocol::HookRunSummary) {
        self.flush_answer_stream_with_separator();
        self.flush_completed_hook_output();
        match self.active_hook_cell.as_mut() {
            Some(cell) => {
                cell.start_run(run);
            }
            None => {
                self.active_hook_cell = Some(history_cell::new_active_hook_cell(
                    run,
                    self.config.animations,
                ));
            }
        }
        self.sync_active_hook_footer();
        self.request_redraw();
    }

    pub(super) fn on_hook_completed(
        &mut self,
        completed: codex_app_server_protocol::HookRunSummary,
    ) {
        let completed_existing_run = self
            .active_hook_cell
            .as_mut()
            .map(|cell| cell.complete_run(completed.clone()))
            .unwrap_or(false);
        if !completed_existing_run {
            match self.active_hook_cell.as_mut() {
                Some(cell) => {
                    cell.add_completed_run(completed);
                }
                None => {
                    let cell =
                        history_cell::new_completed_hook_cell(completed, self.config.animations);
                    if !cell.is_empty() {
                        self.active_hook_cell = Some(cell);
                    }
                }
            }
        }
        self.flush_completed_hook_output();
        self.finish_active_hook_cell_if_idle();
        self.sync_active_hook_footer();
        self.request_redraw();
    }

    pub(super) fn flush_completed_hook_output(&mut self) {
        let Some(completed_cell) = self
            .active_hook_cell
            .as_mut()
            .and_then(HookCell::take_completed_persistent_runs)
        else {
            return;
        };
        let active_cell_is_empty = self
            .active_hook_cell
            .as_ref()
            .is_some_and(HookCell::is_empty);
        if active_cell_is_empty {
            self.active_hook_cell = None;
        }
        self.transcript.needs_final_message_separator = true;
        self.queue_pending_history_commit(Box::new(completed_cell), /*retained_stream*/ false);
        self.request_pending_usage_output_insertion();
    }

    pub(super) fn finish_active_hook_cell_if_idle(&mut self) {
        let Some(cell) = self.active_hook_cell.as_ref() else {
            return;
        };
        if cell.is_empty() {
            self.active_hook_cell = None;
            self.request_pending_usage_output_insertion();
            return;
        }
        if cell.should_flush()
            && let Some(cell) = self.active_hook_cell.take()
        {
            self.transcript.needs_final_message_separator = true;
            self.queue_pending_history_commit(Box::new(cell), /*retained_stream*/ false);
            self.request_pending_usage_output_insertion();
        }
    }

    pub(super) fn update_due_hook_visibility(&mut self) {
        let Some(cell) = self.active_hook_cell.as_mut() else {
            return;
        };
        let now = Instant::now();
        cell.advance_time(now);
        self.finish_active_hook_cell_if_idle();
        self.sync_active_hook_footer();
    }

    pub(super) fn schedule_hook_timer_if_needed(&self) {
        let Some(deadline) = self
            .active_hook_cell
            .as_ref()
            .and_then(HookCell::next_timer_deadline)
        else {
            return;
        };
        let delay = deadline.saturating_duration_since(Instant::now());
        self.frame_requester.schedule_frame_in(delay);
    }

    fn sync_active_hook_footer(&mut self) {
        let status = self
            .active_hook_cell
            .as_ref()
            .and_then(HookCell::visible_running_status);
        self.bottom_pane.set_hook_status(status);
    }
}
