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
use crate::history_cell::PatchHistoryCell;
use crate::history_cell::SelectionContribution;
use crate::history_cell::WebSearchCell;
use crate::history_cell::selection_contribution_from_display_lines;
use crate::history_cell::tool_result_requires_user_action;
use crate::render::line_utils::line_to_static;
use crate::wrapping::RtOptions;
use crate::wrapping::adaptive_wrap_line;

pub(super) struct CompactToolGroup {
    pub(super) lines: Vec<Line<'static>>,
    pub(super) consumed_cells: usize,
}

/// A retained-view projection of adjacent completed tool cells.
///
/// The source cells stay intact for raw mode, transcript overlays, replay, and selection. Rich
/// owned-screen rendering asks the group to recompute its one-line summary at the current width,
/// so terminal resize does not require flattening the transcript into cached rows.
#[derive(Debug)]
struct CompactToolGroupCell {
    source_cells: Vec<Arc<dyn HistoryCell>>,
}

impl CompactToolGroupCell {
    fn new(source_cells: Vec<Arc<dyn HistoryCell>>) -> Self {
        Self { source_cells }
    }

    fn expanded_lines(&self, width: u16, mode: HistoryRenderMode) -> Vec<Line<'static>> {
        render_transcript_lines(
            &self.source_cells,
            width,
            mode,
            /*compact_tool_groups*/ false,
            /*row_cap*/ None,
        )
    }
}

impl HistoryCell for CompactToolGroupCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        compact_tool_group_at(&self.source_cells, /*start*/ 0, width)
            .map(|group| group.lines)
            .unwrap_or_else(|| self.expanded_lines(width, HistoryRenderMode::Rich))
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        self.expanded_lines(u16::MAX, HistoryRenderMode::Raw)
    }

    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.expanded_lines(width, HistoryRenderMode::Rich)
    }

    fn selection_contribution(&self, width: u16, mode: HistoryRenderMode) -> SelectionContribution {
        selection_contribution_from_display_lines(self.display_lines_for_mode(width, mode), width)
    }
}

/// Builds the retained presentation cells used by the application-owned viewport.
///
/// This runs only when committed source changes or the user toggles compact/raw presentation. The
/// viewport continues rendering retained cells frame-to-frame and never rebuilds the full history
/// during an ordinary draw.
pub(super) fn project_owned_cells(
    cells: &[Arc<dyn HistoryCell>],
    compact_tool_groups: bool,
) -> Vec<Arc<dyn HistoryCell>> {
    if !compact_tool_groups {
        return cells.to_vec();
    }

    let mut projected = Vec::with_capacity(cells.len());
    let mut index = 0usize;
    while index < cells.len() {
        if let Some(group) = compact_tool_group_at(cells, index, /*width*/ u16::MAX) {
            let end = index.saturating_add(group.consumed_cells).min(cells.len());
            projected.push(
                Arc::new(CompactToolGroupCell::new(cells[index..end].to_vec()))
                    as Arc<dyn HistoryCell>,
            );
            index = end;
        } else {
            projected.push(cells[index].clone());
            index += 1;
        }
    }
    projected
}

pub(super) fn trailing_compact_tool_run_start(cells: &[Arc<dyn HistoryCell>]) -> usize {
    let mut start = cells.len();
    while start > 0 && tool_group_item(cells[start - 1].as_ref()).is_some() {
        start -= 1;
    }
    start
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
    edit_file_count: usize,
    run_count: usize,
    test_count: usize,
    build_count: usize,
    check_count: usize,
    install_count: usize,
    web_search_count: usize,
    web_open_count: usize,
    web_find_count: usize,
    mcp_counts: BTreeMap<String, usize>,
    actions: usize,
}

impl ToolGroupSummary {
    fn merge(&mut self, other: ToolGroupSummary) {
        self.read_labels.extend(other.read_labels);
        self.list_count += other.list_count;
        self.search_count += other.search_count;
        self.edit_file_count += other.edit_file_count;
        self.run_count += other.run_count;
        self.test_count += other.test_count;
        self.build_count += other.build_count;
        self.check_count += other.check_count;
        self.install_count += other.install_count;
        self.web_search_count += other.web_search_count;
        self.web_open_count += other.web_open_count;
        self.web_find_count += other.web_find_count;
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
        push_count_part(
            &mut parts,
            self.edit_file_count,
            "edited 1 file",
            "files edited",
        );
        push_count_part(
            &mut parts,
            self.test_count,
            "tests passed",
            "test runs passed",
        );
        push_count_part(
            &mut parts,
            self.build_count,
            "build passed",
            "builds passed",
        );
        push_count_part(
            &mut parts,
            self.check_count,
            "check passed",
            "checks passed",
        );
        push_count_part(
            &mut parts,
            self.install_count,
            "installed deps",
            "installed deps",
        );
        push_count_part(&mut parts, self.run_count, "ran command", "ran commands");
        push_count_part(
            &mut parts,
            self.web_search_count,
            "searched web",
            "web searches",
        );
        push_count_part(
            &mut parts,
            self.web_open_count,
            "opened web page",
            "web pages opened",
        );
        push_count_part(
            &mut parts,
            self.web_find_count,
            "found on page",
            "in-page finds",
        );
        if !self.mcp_counts.is_empty() {
            parts.push(mcp_summary(&self.mcp_counts));
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

    let detail = parts.join(" · ");
    let line = Line::from(vec![
        "▸ ".dim(),
        "Work".bold(),
        ": ".dim(),
        detail.into(),
        " · ".dim(),
        "Alt+I inspect · Alt+O all".dim(),
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

pub(super) fn latest_compact_tool_group_cells(
    cells: &[Arc<dyn HistoryCell>],
    width: u16,
) -> Option<Vec<Arc<dyn HistoryCell>>> {
    let mut latest = None;
    let mut start = 0usize;

    while start < cells.len() {
        if let Some(group) = compact_tool_group_at(cells, start, width) {
            let end = start.saturating_add(group.consumed_cells).min(cells.len());
            latest = Some(cells[start..end].to_vec());
            start = end;
        } else {
            start += 1;
        }
    }

    latest
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
    if let Some(patch) = cell.as_any().downcast_ref::<PatchHistoryCell>() {
        return patch_tool_group_item(patch);
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
        if output.exit_code != 0
            || output
                .transcript_lines()
                .any(|line| tool_result_requires_user_action(line.as_ref()))
        {
            return None;
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
    if !success || !mcp.is_safe_to_compact() {
        return None;
    }
    let mut summary = ToolGroupSummary::default();
    let label = format!("{}.{}", invocation.server, invocation.tool);
    summary.mcp_counts.insert(label, 1);
    summary.actions = 1;
    Some(ToolGroupItem {
        summary,
        collapse_single: false,
    })
}

fn web_tool_group_item(web: &WebSearchCell) -> Option<ToolGroupItem> {
    let action = web.completed_action()?;
    let mut summary = ToolGroupSummary {
        actions: 1,
        ..Default::default()
    };
    match action {
        codex_app_server_protocol::WebSearchAction::Search { .. }
        | codex_app_server_protocol::WebSearchAction::Other => summary.web_search_count = 1,
        codex_app_server_protocol::WebSearchAction::OpenPage { .. } => summary.web_open_count = 1,
        codex_app_server_protocol::WebSearchAction::FindInPage { .. } => summary.web_find_count = 1,
    }
    Some(ToolGroupItem {
        summary,
        collapse_single: false,
    })
}

fn patch_tool_group_item(patch: &PatchHistoryCell) -> Option<ToolGroupItem> {
    let edit_file_count = patch.changed_file_count();
    if edit_file_count == 0 {
        return None;
    }
    Some(ToolGroupItem {
        summary: ToolGroupSummary {
            edit_file_count,
            actions: 1,
            ..Default::default()
        },
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
    } else if is_build_command(command) {
        summary.build_count += 1;
    } else if is_check_command(command) {
        summary.check_count += 1;
    } else if is_dependency_install_command(command) {
        summary.install_count += 1;
    } else {
        summary.run_count += 1;
    }
    summary.actions += 1;
}

fn is_build_command(command: &str) -> bool {
    [
        "cargo build",
        "npm run build",
        "pnpm run build",
        "yarn build",
        "yarn run build",
        "bun run build",
        "go build",
        "swift build",
        "zig build",
        "cmake --build",
        "make",
        "just build",
    ]
    .iter()
    .any(|prefix| command_has_prefix(command.trim_start(), prefix))
}

fn is_check_command(command: &str) -> bool {
    [
        "cargo check",
        "cargo clippy",
        "npm run typecheck",
        "pnpm run typecheck",
        "yarn typecheck",
        "bun run typecheck",
        "tsc",
        "eslint",
        "ruff check",
        "just lint",
        "just check",
    ]
    .iter()
    .any(|prefix| command_has_prefix(command.trim_start(), prefix))
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
    use std::collections::HashMap;
    use std::time::Duration;

    use codex_app_server_protocol::CommandExecutionSource as ExecCommandSource;
    use codex_protocol::mcp::CallToolResult;
    use codex_protocol::parse_command::ParsedCommand;
    use serde_json::json;

    use super::*;
    use crate::diff_model::FileChange;
    use crate::exec_cell::CommandOutput;
    use crate::exec_cell::new_active_exec_command;
    use crate::history_cell::McpInvocation;
    use crate::history_cell::new_active_mcp_tool_call;
    use crate::history_cell::new_patch_event;
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

    fn render_lines_text(lines: &[Line<'static>]) -> String {
        lines
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
            CommandOutput::new(/*exit_code*/ 0, String::new()),
            Duration::from_millis(10),
        ));
        Arc::new(cell)
    }

    fn completed_mcp(call_id: &str, server: &str, tool: &str) -> Arc<dyn HistoryCell> {
        completed_mcp_result(
            call_id,
            server,
            tool,
            Ok(CallToolResult {
                content: Vec::new(),
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        )
    }

    fn completed_mcp_result(
        call_id: &str,
        server: &str,
        tool: &str,
        result: Result<CallToolResult, String>,
    ) -> Arc<dyn HistoryCell> {
        let mut cell = new_active_mcp_tool_call(
            call_id.to_string(),
            McpInvocation {
                server: server.to_string(),
                tool: tool.to_string(),
                arguments: Some(json!({ "q": "needle" })),
            },
            /*animations_enabled*/ false,
        );
        assert!(cell.complete(Duration::from_millis(10), result).is_none());
        Arc::new(cell)
    }

    fn completed_command(call_id: &str, command: &str, exit_code: i32) -> Arc<dyn HistoryCell> {
        completed_command_with_output(
            call_id,
            command,
            exit_code,
            if exit_code == 0 {
                String::new()
            } else {
                "permission denied".to_string()
            },
        )
    }

    fn completed_command_with_output(
        call_id: &str,
        command: &str,
        exit_code: i32,
        output: String,
    ) -> Arc<dyn HistoryCell> {
        let mut cell = new_active_exec_command(
            call_id.to_string(),
            command.split_whitespace().map(str::to_string).collect(),
            Vec::new(),
            ExecCommandSource::Agent,
            /*interaction_input*/ None,
            /*animations_enabled*/ false,
        );
        assert!(cell.complete_call(
            call_id,
            CommandOutput::new(exit_code, output),
            Duration::from_millis(10),
        ));
        Arc::new(cell)
    }

    fn completed_patch(paths: &[&str]) -> Arc<dyn HistoryCell> {
        let changes = paths
            .iter()
            .map(|path| {
                (
                    Path::new(path).to_path_buf(),
                    FileChange::Add {
                        content: String::new(),
                    },
                )
            })
            .collect::<HashMap<_, _>>();
        Arc::new(new_patch_event(changes, Path::new("/repo")))
    }

    #[test]
    fn compact_group_summarizes_adjacent_tool_cells() {
        let cells = vec![
            completed_read_exec("read-1", "app.rs"),
            completed_read_exec("read-2", "lib.rs"),
            completed_mcp("mcp-1", "gmail", "read_thread"),
        ];

        let group = compact_tool_group_at(&cells, 0, 120).expect("compact group");

        assert_eq!(group.consumed_cells, 3);
        assert_eq!(
            render_group_text(group),
            "▸ Work: read 2 files · called gmail.read_thread · Alt+I inspect · Alt+O all"
        );
    }

    #[test]
    fn owned_projection_compacts_rich_view_and_preserves_raw_sources() {
        let cells = vec![
            completed_read_exec("read-1", "app.rs"),
            completed_read_exec("read-2", "lib.rs"),
        ];

        let projected = project_owned_cells(&cells, /*compact_tool_groups*/ true);
        assert_eq!(projected.len(), 1);
        let rich = projected[0]
            .display_lines(/*width*/ 80)
            .iter()
            .map(render_line_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(rich, "▸ Work: read 2 files · Alt+I inspect · Alt+O all");

        let raw = projected[0]
            .raw_lines()
            .iter()
            .map(render_line_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            raw.contains("app.rs"),
            "raw projection lost first source: {raw}"
        );
        assert!(
            raw.contains("lib.rs"),
            "raw projection lost second source: {raw}"
        );

        let expanded = project_owned_cells(&cells, /*compact_tool_groups*/ false);
        assert_eq!(expanded.len(), 2);
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
            CommandOutput::new(/*exit_code*/ 0, String::new()),
            Duration::from_millis(10),
        ));
        let cells: Vec<Arc<dyn HistoryCell>> = vec![Arc::new(cell)];

        let group = compact_tool_group_at(&cells, 0, 80).expect("compact group");

        assert_eq!(group.consumed_cells, 1);
        assert_eq!(
            render_group_text(group),
            "▸ Work: read app.rs · searched · Alt+I inspect · Alt+O all"
        );
    }

    #[test]
    fn web_searches_compact_when_adjacent() {
        let cells: Vec<Arc<dyn HistoryCell>> = vec![
            Arc::new(new_web_search_call(
                "web-1".to_string(),
                "coffee montreal".to_string(),
                codex_app_server_protocol::WebSearchAction::Search {
                    query: Some("coffee montreal".to_string()),
                    queries: None,
                },
            )),
            Arc::new(new_web_search_call(
                "web-2".to_string(),
                String::new(),
                codex_app_server_protocol::WebSearchAction::OpenPage {
                    url: Some("https://example.com".to_string()),
                },
            )),
            Arc::new(new_web_search_call(
                "web-3".to_string(),
                String::new(),
                codex_app_server_protocol::WebSearchAction::FindInPage {
                    url: Some("https://example.com".to_string()),
                    pattern: Some("needle".to_string()),
                },
            )),
        ];

        let group = compact_tool_group_at(&cells, 0, 120).expect("compact group");

        assert_eq!(group.consumed_cells, 3);
        assert_eq!(
            render_group_text(group),
            "▸ Work: searched web · opened web page · found on page · Alt+I inspect · Alt+O all"
        );
    }

    #[test]
    fn outcome_first_work_bundle_golden() {
        let cells = vec![
            completed_read_exec("read-1", "app.rs"),
            completed_read_exec("read-2", "lib.rs"),
            completed_patch(&["src/app.rs", "src/lib.rs"]),
            completed_command("test-1", "just test -p codex-tui", 0),
            completed_command("build-1", "cargo build", 0),
            completed_command("check-1", "cargo clippy", 0),
            completed_mcp("mcp-1", "gmail", "read_thread"),
        ];

        let rendered = render_group_text(compact_tool_group_at(&cells, 0, 120).unwrap());

        insta::assert_snapshot!("outcome_first_work_bundle", rendered);
    }

    #[test]
    fn failures_break_groups_and_render_their_error_tail() {
        let cells = vec![
            completed_read_exec("read-1", "app.rs"),
            completed_read_exec("read-2", "lib.rs"),
            completed_command("failed", "cargo build", 7),
            completed_read_exec("read-3", "main.rs"),
            completed_read_exec("read-4", "mod.rs"),
        ];

        assert_eq!(
            compact_tool_group_at(&cells, 0, 100)
                .unwrap()
                .consumed_cells,
            2
        );
        assert!(compact_tool_group_at(&cells, 2, 100).is_none());
        assert_eq!(
            compact_tool_group_at(&cells, 3, 100)
                .unwrap()
                .consumed_cells,
            2
        );

        let rendered = render_lines_text(&render_transcript_lines(
            &cells,
            100,
            HistoryRenderMode::Rich,
            true,
            None,
        ));
        assert!(rendered.contains("permission denied"));
        assert!(rendered.contains("failed (exit 7)"));
        insta::assert_snapshot!("failed_exec_between_work_bundles", rendered);
    }

    #[test]
    fn failed_and_action_required_mcp_calls_never_compact() {
        let failed = completed_mcp_result(
            "mcp-failed",
            "calendar",
            "create",
            Err("permission denied".to_string()),
        );
        let action_required = completed_mcp_result(
            "mcp-auth",
            "drive",
            "read",
            Ok(CallToolResult {
                content: vec![json!({
                    "type": "text",
                    "text": "Log in at https://example.com/device"
                })],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        );
        let cells = vec![
            completed_read_exec("read-1", "app.rs"),
            completed_read_exec("read-2", "lib.rs"),
            failed,
            action_required,
            completed_read_exec("read-3", "main.rs"),
            completed_read_exec("read-4", "mod.rs"),
        ];

        assert!(compact_tool_group_at(&cells, 2, 100).is_none());
        assert!(compact_tool_group_at(&cells, 3, 100).is_none());
        let rendered = render_lines_text(&render_transcript_lines(
            &cells,
            100,
            HistoryRenderMode::Rich,
            true,
            None,
        ));
        assert!(rendered.contains("Error: permission denied"));
        assert!(rendered.contains("https://example.com/device"));
    }

    #[test]
    fn action_required_exec_output_never_compacts() {
        let action_required = completed_command_with_output(
            "login",
            "service login",
            0,
            "Open the following URL to sign in: https://example.com/device".to_string(),
        );
        let cells = vec![
            completed_read_exec("read-1", "app.rs"),
            completed_read_exec("read-2", "lib.rs"),
            action_required,
            completed_read_exec("read-3", "main.rs"),
            completed_read_exec("read-4", "mod.rs"),
        ];

        assert!(compact_tool_group_at(&cells, 2, 100).is_none());
        let rendered = render_lines_text(&render_transcript_lines(
            &cells,
            100,
            HistoryRenderMode::Rich,
            true,
            None,
        ));
        assert!(rendered.contains("https://example.com/device"));
    }

    #[test]
    fn latest_group_inspector_returns_only_the_latest_bundle_source_cells() {
        let cells = vec![
            completed_read_exec("read-1", "app.rs"),
            completed_read_exec("read-2", "lib.rs"),
            completed_command("failed", "false", 1),
            completed_read_exec("read-3", "main.rs"),
            completed_read_exec("read-4", "mod.rs"),
        ];

        let inspected = latest_compact_tool_group_cells(&cells, 100).unwrap();
        let rendered = render_lines_text(&render_transcript_lines(
            &inspected,
            100,
            HistoryRenderMode::Raw,
            false,
            None,
        ));

        assert_eq!(inspected.len(), 2);
        assert!(rendered.contains("$ cat main.rs"));
        assert!(rendered.contains("$ cat mod.rs"));
        assert!(!rendered.contains("$ cat app.rs"));
        assert!(!rendered.contains("permission denied"));
    }

    #[test]
    fn compact_rows_are_bounded_and_raw_source_remains_recoverable() {
        let cells = (0..8)
            .map(|index| completed_read_exec(&format!("read-{index}"), &format!("file-{index}.rs")))
            .collect::<Vec<_>>();

        let compact = render_transcript_lines(&cells, 120, HistoryRenderMode::Rich, true, Some(2));
        let raw = render_transcript_lines(&cells, 120, HistoryRenderMode::Raw, false, None);
        let raw_text = render_lines_text(&raw);

        assert!(compact.len() <= 2);
        assert!(raw.len() > compact.len());
        for index in 0..8 {
            assert!(raw_text.contains(&format!("$ cat file-{index}.rs")));
        }
    }
}
