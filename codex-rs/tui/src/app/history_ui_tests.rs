use super::*;
use crate::history_cell;
use crate::history_cell::HistoryCell;
use crate::history_cell::PlainHistoryCell;
use codex_app_server_protocol::CommandExecutionSource;
use ratatui::text::Line;
use std::sync::Arc;
use std::time::Duration;

#[test]
fn desktop_thread_opened_history_snapshot() {
    let cell = history_cell::new_info_event(
        DESKTOP_THREAD_OPENED_MESSAGE.to_string(),
        /*hint*/ None,
    );

    insta::assert_snapshot!("desktop_thread_opened_history", render_cell(&cell));
}

#[test]
fn desktop_thread_open_error_history_snapshot() {
    let cell = history_cell::new_error_event(desktop_thread_open_error_message("launch failed"));

    insta::assert_snapshot!("desktop_thread_open_error_history", render_cell(&cell));
}

fn render_cell(cell: &impl HistoryCell) -> String {
    let lines = cell.display_lines(/*width*/ 80);
    lines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn background_terminal_promotion_replaces_causal_exec_across_interleaved_cells() {
    let mut exec = crate::exec_cell::ExecCell::new(
        crate::exec_cell::ExecCall {
            call_id: "call-background".to_string(),
            command: vec!["cargo test".to_string()],
            parsed: Vec::new(),
            output: None,
            source: CommandExecutionSource::UnifiedExecStartup,
            start_time: None,
            duration: None,
            interaction_input: None,
        },
        /*animations_enabled*/ false,
    );
    assert!(exec.complete_call(
        "call-background",
        crate::exec_cell::CommandOutput {
            exit_code: 0,
            aggregated_output: String::new(),
            formatted_output: String::new(),
        },
        Duration::from_secs(1),
    ));
    let unrelated: Arc<dyn HistoryCell> = Arc::new(PlainHistoryCell::new(vec![Line::from(
        "unrelated agent message",
    )]));
    let mut cells: Vec<Arc<dyn HistoryCell>> = vec![Arc::new(exec), unrelated.clone()];

    let lifecycle = crate::history_cell::BackgroundTerminalLifecycleCell::new(
        "call-background".to_string(),
        "proc-1".to_string(),
        "cargo test".to_string(),
    );
    lifecycle.activate();
    lifecycle.record_interaction(String::new());
    let lifecycle: Arc<dyn HistoryCell> = Arc::new(lifecycle);
    promote_background_terminal_cell(&mut cells, "call-background", lifecycle);

    assert_eq!(cells.len(), 2, "unrelated cells should not be consumed");
    assert!(
        cells[0]
            .as_any()
            .is::<crate::history_cell::BackgroundTerminalLifecycleCell>()
    );
    assert!(Arc::ptr_eq(&cells[1], &unrelated));
}
