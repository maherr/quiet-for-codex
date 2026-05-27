//! Compact render-time grouping for completed tool calls in codex-quiet.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use codex_protocol::parse_command::ParsedCommand;
use ratatui::prelude::*;
use ratatui::style::Stylize;

use crate::exec_cell::ExecCell;
use crate::history_cell::HistoryCell;
use crate::history_cell::HistoryRenderMode;
use crate::history_cell::McpToolCallCell;
use crate::history_cell::WebSearchCell;
use crate::render::line_utils::line_to_static;
use crate::wrapping::RtOptions;
use crate::wrapping::adaptive_wrap_line;

pub(super) struct CompactToolGroup {
    pub(super) lines: Vec<Line<'static>>,
    pub(super) consumed_cells: usize,
}

#[derive(Default)]
struct ToolGroupItem {
    summary: ToolGroupSummary,
    collapse_single: bool,
}

#[derive(Default)]
struct ToolGroupSummary {
    read_labels: BTreeSet<String>,
    list_count: usize,
    search_count: usize,
    run_count: usize,
    test_count: usize,
    install_count: usize,
    web_count: usize,
    mcp_counts: BTreeMap<String, usize>,
    failures: usize,
    actions: usize,
}

impl ToolGroupSummary {
    fn merge(&mut self, other: ToolGroupSummary) {
        self.read_labels.extend(other.read_labels);
        self.list_count += other.list_count;
        self.search_count += other.search_count;
        self.run_count += other.run_count;
        self.test_count += other.test_count;
        self.install_count += other.install_count;
        self.web_count += other.web_count;
        self.failures += other.failures;
        self.actions += other.actions;
        for (label, count) in other.mcp_counts {
            *self.mcp_counts.entry(label).or_default() += count;
        }
    }

    fn parts(&self) -> Vec<String> {
        let mut parts = Vec::new();
        if !self.read_labels.is_empty() {
            parts.push(read_summary(&self.read_labels));
        }
        push_count_part(&mut parts, self.list_count, "listed dir", "listed dirs");
        push_count_part(&mut parts, self.search_count, "searched", "searched");
        push_count_part(&mut parts, self.test_count, "ran tests", "ran tests");
        push_count_part(
            &mut parts,
            self.install_count,
            "installed deps",
            "installed deps",
        );
        push_count_part(&mut parts, self.run_count, "ran command", "ran commands");
        push_count_part(&mut parts, self.web_count, "searched web", "searched web");
        if !self.mcp_counts.is_empty() {
            parts.push(mcp_summary(&self.mcp_counts));
        }
        if self.failures > 0 {
            push_count_part(&mut parts, self.failures, "1 failed", "failed");
        }
        parts
    }
}

pub(super) fn compact_tool_group_at(
    cells: &[Arc<dyn HistoryCell>],
    start: usize,
    width: u16,
) -> Option<CompactToolGroup> {
    let mut summary = ToolGroupSummary::default();
    let mut consumed_cells = 0usize;
    let mut collapse_single = false;

    for cell in cells.iter().skip(start) {
        let Some(item) = tool_group_item(cell.as_ref()) else {
            break;
        };
        summary.merge(item.summary);
        collapse_single |= item.collapse_single;
        consumed_cells += 1;
    }

    if consumed_cells == 0 || (consumed_cells == 1 && !collapse_single) {
        return None;
    }

    let parts = summary.parts();
    if parts.is_empty() {
        return None;
    }

    let detail = parts.join(", ");
    let line = Line::from(vec![
        "▸ ".dim(),
        "Tools".bold(),
        ": ".dim(),
        detail.into(),
        " · ".dim(),
        "Alt+O expands".dim(),
    ]);
    let wrapped = adaptive_wrap_line(
        &line,
        RtOptions::new(usize::from(width.max(1)))
            .initial_indent(Line::from(""))
            .subsequent_indent(Line::from("  ")),
    );

    Some(CompactToolGroup {
        lines: wrapped
            .into_iter()
            .map(|line| line_to_static(&line))
            .collect(),
        consumed_cells,
    })
}

pub(super) fn appended_cell_touches_compact_group(
    cells: &[Arc<dyn HistoryCell>],
    width: u16,
) -> bool {
    let len = cells.len();
    if len == 0 {
        return false;
    }

    (0..len).rev().any(|start| {
        compact_tool_group_at(cells, start, width)
            .is_some_and(|group| start.saturating_add(group.consumed_cells) == len)
    })
}

pub(super) fn render_transcript_lines(
    cells: &[Arc<dyn HistoryCell>],
    width: u16,
    render_mode: HistoryRenderMode,
    compact_tool_groups: bool,
    row_cap: Option<usize>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut has_emitted_cell = false;
    let mut i = 0usize;

    while i < cells.len() {
        let (display, is_stream_continuation, consumed_cells) =
            if compact_tool_groups && let Some(group) = compact_tool_group_at(cells, i, width) {
                (group.lines, false, group.consumed_cells)
            } else {
                let cell = cells[i].clone();
                (
                    cell.display_lines_for_mode(width, render_mode),
                    cell.is_stream_continuation(),
                    1,
                )
            };

        if !display.is_empty() {
            if !is_stream_continuation {
                if has_emitted_cell {
                    lines.push(Line::from(""));
                } else {
                    has_emitted_cell = true;
                }
            }
            lines.extend(display);
        }
        i += consumed_cells.max(1);
    }

    if let Some(max_rows) = row_cap
        && lines.len() > max_rows
    {
        let trimmed_line_count = lines.len() - max_rows;
        lines = lines.split_off(trimmed_line_count);
    }

    lines
}

fn tool_group_item(cell: &dyn HistoryCell) -> Option<ToolGroupItem> {
    if let Some(exec) = cell.as_any().downcast_ref::<ExecCell>() {
        return exec_tool_group_item(exec);
    }
    if let Some(mcp) = cell.as_any().downcast_ref::<McpToolCallCell>() {
        return mcp_tool_group_item(mcp);
    }
    if let Some(web) = cell.as_any().downcast_ref::<WebSearchCell>() {
        return web_tool_group_item(web);
    }
    None
}

#[allow(clippy::redundant_closure_for_method_calls)]
fn exec_tool_group_item(exec: &ExecCell) -> Option<ToolGroupItem> {
    if exec.is_active() || exec.iter_calls().any(|call| call.is_user_shell_command()) {
        return None;
    }

    let mut summary = ToolGroupSummary::default();
    let mut call_count = 0usize;
    for call in exec.iter_calls() {
        call_count += 1;
        let output = call.output.as_ref()?;
        if output.exit_code != 0 {
            summary.failures += 1;
        }

        if call.parsed.is_empty() {
            add_command(&mut summary, &call.command.join(" "));
            continue;
        }

        for parsed in &call.parsed {
            add_parsed_command(&mut summary, parsed);
        }
    }

    Some(ToolGroupItem {
        collapse_single: exec.is_exploring_cell() && summary.actions > 1 || call_count > 1,
        summary,
    })
}

fn mcp_tool_group_item(mcp: &McpToolCallCell) -> Option<ToolGroupItem> {
    let (invocation, success) = mcp.completed_invocation()?;
    let mut summary = ToolGroupSummary::default();
    let label = format!("{}.{}", invocation.server, invocation.tool);
    summary.mcp_counts.insert(label, 1);
    summary.actions = 1;
    if !success {
        summary.failures = 1;
    }
    Some(ToolGroupItem {
        summary,
        collapse_single: false,
    })
}

fn web_tool_group_item(web: &WebSearchCell) -> Option<ToolGroupItem> {
    if !web.is_completed() {
        return None;
    }
    let summary = ToolGroupSummary {
        web_count: 1,
        actions: 1,
        ..Default::default()
    };
    Some(ToolGroupItem {
        summary,
        collapse_single: false,
    })
}

fn add_parsed_command(summary: &mut ToolGroupSummary, parsed: &ParsedCommand) {
    match parsed {
        ParsedCommand::Read { name, path, .. } => {
            summary.read_labels.insert(read_label(name, path));
            summary.actions += 1;
        }
        ParsedCommand::ListFiles { .. } => {
            summary.list_count += 1;
            summary.actions += 1;
        }
        ParsedCommand::Search { .. } => {
            summary.search_count += 1;
            summary.actions += 1;
        }
        ParsedCommand::Unknown { cmd } => add_command(summary, cmd),
    }
}

fn add_command(summary: &mut ToolGroupSummary, command: &str) {
    if is_test_command(command) {
        summary.test_count += 1;
    } else if is_dependency_install_command(command) {
        summary.install_count += 1;
    } else {
        summary.run_count += 1;
    }
    summary.actions += 1;
}

fn read_label(name: &str, path: &Path) -> String {
    if !name.trim().is_empty() {
        return name.to_string();
    }
    path.file_name()
        .and_then(|file_name| file_name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.display().to_string())
}

fn read_summary(labels: &BTreeSet<String>) -> String {
    match labels.len() {
        0 => String::new(),
        1 => labels
            .first()
            .map(|label| format!("read {}", truncate_label(label)))
            .unwrap_or_default(),
        count => format!("read {count} files"),
    }
}

fn mcp_summary(counts: &BTreeMap<String, usize>) -> String {
    if counts.len() == 1 {
        let Some((label, count)) = counts.iter().next() else {
            return "called 0 MCP tools".to_string();
        };
        if *count == 1 {
            format!("called {label}")
        } else {
            format!("called {label} x{count}")
        }
    } else {
        format!("called {} MCP tools", counts.values().sum::<usize>())
    }
}

fn push_count_part(parts: &mut Vec<String>, count: usize, singular: &str, plural: &str) {
    match count {
        0 => {}
        1 if singular.starts_with('1') => parts.push(singular.to_string()),
        1 => parts.push(singular.to_string()),
        _ => parts.push(format!("{count} {plural}")),
    }
}

fn truncate_label(label: &str) -> String {
    const MAX_CHARS: usize = 32;
    if label.chars().count() <= MAX_CHARS {
        return label.to_string();
    }
    let mut out = label.chars().take(MAX_CHARS).collect::<String>();
    out.push('…');
    out
}

fn is_test_command(command: &str) -> bool {
    [
        "cargo test",
        "cargo nextest",
        "npm test",
        "npm run test",
        "pnpm test",
        "pnpm run test",
        "yarn test",
        "yarn run test",
        "bun test",
        "bun run test",
        "pytest",
        "go test",
        "swift test",
        "zig build test",
        "just test",
    ]
    .iter()
    .any(|prefix| command_has_prefix(command.trim_start(), prefix))
}

fn is_dependency_install_command(command: &str) -> bool {
    [
        "npm install",
        "npm ci",
        "pnpm install",
        "yarn install",
        "bun install",
        "cargo fetch",
    ]
    .iter()
    .any(|prefix| command_has_prefix(command.trim_start(), prefix))
}

fn command_has_prefix(command: &str, prefix: &str) -> bool {
    command
        .strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use codex_app_server_protocol::CommandExecutionSource as ExecCommandSource;
    use codex_protocol::mcp::CallToolResult;
    use codex_protocol::parse_command::ParsedCommand;
    use serde_json::json;

    use super::*;
    use crate::exec_cell::CommandOutput;
    use crate::exec_cell::new_active_exec_command;
    use crate::history_cell::McpInvocation;
    use crate::history_cell::new_active_mcp_tool_call;
    use crate::history_cell::new_web_search_call;

    fn render_line_text(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    fn render_group_text(group: CompactToolGroup) -> String {
        group
            .lines
            .iter()
            .map(render_line_text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn completed_read_exec(call_id: &str, name: &str) -> Arc<dyn HistoryCell> {
        let command = vec!["cat".to_string(), name.to_string()];
        let parsed = vec![ParsedCommand::Read {
            cmd: command.join(" "),
            name: name.to_string(),
            path: name.into(),
        }];
        let mut cell = new_active_exec_command(
            call_id.to_string(),
            command,
            parsed,
            ExecCommandSource::Agent,
            /*interaction_input*/ None,
            /*animations_enabled*/ false,
        );
        assert!(cell.complete_call(
            call_id,
            CommandOutput {
                exit_code: 0,
                aggregated_output: String::new(),
                formatted_output: String::new(),
            },
            Duration::from_millis(10),
        ));
        Arc::new(cell)
    }

    fn completed_mcp(call_id: &str, server: &str, tool: &str) -> Arc<dyn HistoryCell> {
        let mut cell = new_active_mcp_tool_call(
            call_id.to_string(),
            McpInvocation {
                server: server.to_string(),
                tool: tool.to_string(),
                arguments: Some(json!({ "q": "needle" })),
            },
            /*animations_enabled*/ false,
        );
        assert!(
            cell.complete(
                Duration::from_millis(10),
                Ok(CallToolResult {
                    content: Vec::new(),
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
            )
            .is_none()
        );
        Arc::new(cell)
    }

    #[test]
    fn compact_group_summarizes_adjacent_tool_cells() {
        let cells = vec![
            completed_read_exec("read-1", "app.rs"),
            completed_read_exec("read-2", "lib.rs"),
            completed_mcp("mcp-1", "gmail", "read_thread"),
        ];

        let group = compact_tool_group_at(&cells, 0, 80).expect("compact group");

        assert_eq!(group.consumed_cells, 3);
        assert_eq!(
            render_group_text(group),
            "▸ Tools: read 2 files, called gmail.read_thread · Alt+O expands"
        );
    }

    #[test]
    fn single_multi_action_exploring_cell_can_collapse() {
        let command = vec!["rg".to_string(), "needle".to_string()];
        let parsed = vec![
            ParsedCommand::Search {
                cmd: "rg needle".to_string(),
                query: Some("needle".to_string()),
                path: None,
            },
            ParsedCommand::Read {
                cmd: "sed -n 1,20p app.rs".to_string(),
                name: "app.rs".to_string(),
                path: "app.rs".into(),
            },
        ];
        let mut cell = new_active_exec_command(
            "search-read".to_string(),
            command,
            parsed,
            ExecCommandSource::Agent,
            /*interaction_input*/ None,
            /*animations_enabled*/ false,
        );
        assert!(cell.complete_call(
            "search-read",
            CommandOutput {
                exit_code: 0,
                aggregated_output: String::new(),
                formatted_output: String::new(),
            },
            Duration::from_millis(10),
        ));
        let cells: Vec<Arc<dyn HistoryCell>> = vec![Arc::new(cell)];

        let group = compact_tool_group_at(&cells, 0, 80).expect("compact group");

        assert_eq!(group.consumed_cells, 1);
        assert_eq!(
            render_group_text(group),
            "▸ Tools: read app.rs, searched · Alt+O expands"
        );
    }

    #[test]
    fn web_searches_compact_when_adjacent() {
        let cells: Vec<Arc<dyn HistoryCell>> = vec![
            Arc::new(new_web_search_call(
                "web-1".to_string(),
                "coffee montreal".to_string(),
                codex_app_server_protocol::WebSearchAction::Other,
            )),
            Arc::new(new_web_search_call(
                "web-2".to_string(),
                "cafe pista".to_string(),
                codex_app_server_protocol::WebSearchAction::Other,
            )),
        ];

        let group = compact_tool_group_at(&cells, 0, 80).expect("compact group");

        assert_eq!(group.consumed_cells, 2);
        assert_eq!(
            render_group_text(group),
            "▸ Tools: 2 searched web · Alt+O expands"
        );
    }
}
