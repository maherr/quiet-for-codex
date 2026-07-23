//! Compact render-time grouping for completed tool calls in codex-quiet.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use codex_protocol::parse_command::ParsedCommand;
use ratatui::prelude::*;
use ratatui::style::Stylize;
use serde_json::Value;

use crate::chatwidget::ActiveCellDisplaySnapshot;
use crate::exec_cell::CommandOutput;
use crate::exec_cell::ExecCell;
use crate::exec_cell::compact_command_for_viewport;
use crate::exec_command::strip_bash_lc_and_escape;
use crate::history_cell::HistoryCell;
use crate::history_cell::HistoryRenderMode;
use crate::history_cell::MAX_TOOL_RESULT_SCAN_BYTES;
use crate::history_cell::McpToolCallCell;
use crate::history_cell::PatchHistoryCell;
use crate::history_cell::ReasoningSummaryCell;
use crate::history_cell::SelectionContribution;
use crate::history_cell::WebSearchCell;
use crate::history_cell::contains_ascii_case_insensitive;
use crate::history_cell::selection_contribution_from_display_lines;
use crate::history_cell::tool_result_requires_user_action;
use crate::line_truncation::truncate_line_with_ellipsis_if_overflow;
use crate::render::line_utils::line_to_static;
use crate::terminal_hyperlinks::plain_hyperlink_lines;
use crate::wrapping::RtOptions;
use crate::wrapping::adaptive_wrap_line;

/// Large successful outputs stay expanded instead of being copied and parsed solely to decide
/// whether they can collapse into a one-line Work summary.
const MAX_COMPACT_OUTPUT_SCAN_BYTES: usize = MAX_TOOL_RESULT_SCAN_BYTES;

pub(super) struct CompactToolGroup {
    pub(super) lines: Vec<Line<'static>>,
    pub(super) consumed_cells: usize,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct CompactToolGroupId(usize);

/// A retained-view projection of adjacent completed tool cells.
///
/// The source cells stay intact for raw mode, transcript overlays, replay, and selection. Rich
/// owned-screen rendering asks the group to recompute its one-line summary at the current width,
/// so terminal resize does not require flattening the transcript into cached rows.
#[derive(Debug)]
struct CompactToolGroupCell {
    id: CompactToolGroupId,
    source_cells: Vec<Arc<dyn HistoryCell>>,
    expanded: bool,
}

impl CompactToolGroupCell {
    fn new(
        id: CompactToolGroupId,
        source_cells: Vec<Arc<dyn HistoryCell>>,
        expanded: bool,
    ) -> Self {
        Self {
            id,
            source_cells,
            expanded,
        }
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
        compact_tool_group_at_with_state(&self.source_cells, /*start*/ 0, width, self.expanded)
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
#[cfg(test)]
pub(super) fn project_owned_cells(
    cells: &[Arc<dyn HistoryCell>],
    compact_tool_groups: bool,
) -> Vec<Arc<dyn HistoryCell>> {
    project_owned_cells_with_expanded(cells, compact_tool_groups, &HashSet::new())
}

pub(super) fn project_owned_cells_with_expanded(
    cells: &[Arc<dyn HistoryCell>],
    compact_tool_groups: bool,
    expanded_groups: &HashSet<CompactToolGroupId>,
) -> Vec<Arc<dyn HistoryCell>> {
    if !compact_tool_groups {
        return cells.to_vec();
    }

    let mut projected = Vec::with_capacity(cells.len());
    let mut index = 0usize;
    while index < cells.len() {
        if let Some(group) = compact_tool_group_at(cells, index, /*width*/ u16::MAX) {
            let end = index.saturating_add(group.consumed_cells).min(cells.len());
            let id = compact_tool_group_id(&cells[index]);
            let expanded = expanded_groups.contains(&id);
            projected.push(Arc::new(CompactToolGroupCell::new(
                id,
                cells[index..end].to_vec(),
                expanded,
            )) as Arc<dyn HistoryCell>);
            if expanded {
                projected.extend(cells[index..end].iter().cloned());
            }
            index = end;
        } else {
            projected.push(cells[index].clone());
            index += 1;
        }
    }
    projected
}

pub(super) fn compact_tool_group_ids(
    cells: &[Arc<dyn HistoryCell>],
) -> HashSet<CompactToolGroupId> {
    let mut ids = HashSet::new();
    let mut index = 0usize;
    while index < cells.len() {
        if let Some(group) = compact_tool_group_at(cells, index, /*width*/ u16::MAX) {
            ids.insert(compact_tool_group_id(&cells[index]));
            index = index.saturating_add(group.consumed_cells);
        } else {
            index += 1;
        }
    }
    ids
}

pub(super) fn compact_tool_group_header_id(
    cell: &dyn HistoryCell,
    width: u16,
    row_within_cell: usize,
) -> Option<CompactToolGroupId> {
    let cell = cell.as_any().downcast_ref::<CompactToolGroupCell>()?;
    (row_within_cell < cell.display_lines(width).len()).then_some(cell.id)
}

/// Builds a compact retained-view projection for a multi-call exploring cell while a turn is
/// still running. Completed safe actions become the Work summary and the current command remains
/// visible on a second row. Any failure, action prompt, user shell call, or streamed output falls
/// back to the source cell so live information is never hidden behind an optimistic summary.
#[allow(clippy::redundant_closure_for_method_calls)]
pub(super) fn compact_active_exec_snapshot(
    cell: &dyn HistoryCell,
    width: u16,
) -> Option<ActiveCellDisplaySnapshot> {
    let exec = cell.as_any().downcast_ref::<ExecCell>()?;
    if !exec.is_exploring_cell() || exec.iter_calls().any(|call| call.is_user_shell_command()) {
        return None;
    }

    let mut summary = ToolGroupSummary::default();
    let mut running = Vec::new();
    for call in exec.iter_calls() {
        if call.duration.is_none() {
            if call
                .output
                .as_ref()
                .is_some_and(|output| output.transcript_lines().next().is_some())
            {
                return None;
            }
            running.push(call);
            continue;
        }

        let output = call.output.as_ref()?;
        if output.exit_code != 0 {
            return None;
        }
        let transcript = bounded_transcript_for_compaction(output)?;
        if successful_output_requires_user_action(&transcript, &call.command) {
            return None;
        }
        if call.parsed.is_empty() {
            add_command(&mut summary, &call.command.join(" "));
        } else {
            for parsed in &call.parsed {
                add_parsed_command(&mut summary, parsed);
            }
        }
    }

    let running_actions = running
        .iter()
        .map(|call| call.parsed.len().max(1))
        .sum::<usize>();
    if summary.actions == 0 || summary.actions.saturating_add(running_actions) < 2 {
        return None;
    }
    if running.len() > 1 {
        return None;
    }

    let detail = summary.parts().join(" · ");
    if detail.is_empty() {
        return None;
    }
    let header = Line::from(vec![
        "• ".cyan().bold(),
        "Work".bold(),
        ": ".dim(),
        detail.into(),
    ]);
    let mut lines = adaptive_wrap_line(
        &header,
        RtOptions::new(usize::from(width.max(1)))
            .initial_indent(Line::from(""))
            .subsequent_indent(Line::from("  ")),
    )
    .into_iter()
    .map(|line| line_to_static(&line))
    .collect::<Vec<_>>();

    if !running.is_empty() {
        let running_line = if let [call] = running.as_slice() {
            let command = compact_command_for_viewport(&strip_bash_lc_and_escape(&call.command));
            Line::from(vec!["  └ ".dim(), "Running: ".bold(), command.into()])
        } else {
            Line::from(vec![
                "  └ ".dim(),
                format!("Running {} commands", running.len()).bold(),
            ])
        };
        lines.push(truncate_line_with_ellipsis_if_overflow(
            running_line,
            usize::from(width.max(1)),
        ));
    }

    let selection_projection =
        selection_contribution_from_display_lines(lines.clone(), width).into_projection();
    Some(ActiveCellDisplaySnapshot {
        lines: plain_hyperlink_lines(lines),
        selection_projection,
        is_stream_continuation: cell.is_stream_continuation(),
    })
}

fn compact_tool_group_id(cell: &Arc<dyn HistoryCell>) -> CompactToolGroupId {
    CompactToolGroupId(Arc::as_ptr(cell) as *const () as usize)
}

pub(super) fn trailing_compact_tool_run_start(cells: &[Arc<dyn HistoryCell>]) -> usize {
    let mut start = cells.len();
    while start > 0 {
        let cell = cells[start - 1].as_ref();
        if tool_group_item(cell).is_some() || is_transparent_tool_group_cell(cell) {
            start -= 1;
        } else {
            break;
        }
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
    compact_tool_group_at_with_state(cells, start, width, /*expanded*/ false)
}

fn compact_tool_group_at_with_state(
    cells: &[Arc<dyn HistoryCell>],
    start: usize,
    width: u16,
    expanded: bool,
) -> Option<CompactToolGroup> {
    let mut summary = ToolGroupSummary::default();
    let mut consumed_cells = 0usize;
    let mut tool_cells = 0usize;
    let mut collapse_single = false;

    for cell in cells.iter().skip(start) {
        if let Some(item) = tool_group_item(cell.as_ref()) {
            summary.merge(item.summary);
            collapse_single |= item.collapse_single;
            consumed_cells += 1;
            tool_cells += 1;
            continue;
        }
        if tool_cells > 0 && is_transparent_tool_group_cell(cell.as_ref()) {
            consumed_cells += 1;
            continue;
        }
        break;
    }

    if tool_cells == 0 || (tool_cells == 1 && !collapse_single) {
        return None;
    }

    let parts = summary.parts();
    if parts.is_empty() {
        return None;
    }

    let detail = parts.join(" · ");
    let line = Line::from(vec![
        (if expanded { "▾ " } else { "▸ " }).dim(),
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

    let run_start = trailing_compact_tool_run_start(cells);
    (run_start..len).rev().any(|start| {
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

/// Hidden reasoning summaries are transcript-only source records, not visible conversation
/// boundaries. Keep them inside a surrounding Work group so they remain available to inspectors
/// and transcript recovery without splitting two otherwise adjacent completed tool calls.
fn is_transparent_tool_group_cell(cell: &dyn HistoryCell) -> bool {
    cell.as_any().is::<ReasoningSummaryCell>()
        && cell.display_lines(u16::MAX).is_empty()
        && cell.raw_lines().is_empty()
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
            return None;
        }
        let transcript = bounded_transcript_for_compaction(output)?;
        if successful_output_requires_user_action(&transcript, &call.command) {
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

fn bounded_transcript_for_compaction(output: &CommandOutput) -> Option<String> {
    let mut transcript = String::new();
    let mut has_line = false;
    for line in output.transcript_lines() {
        let separator_bytes = usize::from(has_line);
        let next_len = transcript
            .len()
            .checked_add(separator_bytes)?
            .checked_add(line.len())?;
        if next_len > MAX_COMPACT_OUTPUT_SCAN_BYTES {
            return None;
        }
        if has_line {
            transcript.push('\n');
        }
        transcript.push_str(&line);
        has_line = true;
    }
    Some(transcript)
}

/// Machine-readable command output often contains URL-valued metadata. It is safe to hide those
/// assignment values when the surrounding output has no action marker, but a bare or prose URL is
/// still user-facing by default (OAuth/device links are frequently printed without explanation).
fn successful_output_requires_user_action(output: &str, command: &[String]) -> bool {
    if !contains_web_url(output) {
        return tool_result_requires_user_action(output);
    }
    if web_urls_have_action_markers(output)
        || !(all_web_urls_are_machine_metadata(output)
            || yt_dlp_tsv_urls_are_metadata(output, command))
    {
        return tool_result_requires_user_action(output);
    }
    tool_result_requires_user_action(&strip_web_urls(output))
}

fn yt_dlp_tsv_urls_are_metadata(output: &str, command: &[String]) -> bool {
    let command = strip_bash_lc_and_escape(command);
    if has_unquoted_shell_control(&command) {
        return false;
    }
    let Some(tokens) = shlex::split(&command) else {
        return false;
    };
    let first_command = tokens
        .iter()
        .take_while(|token| !matches!(token.as_str(), "|" | "||" | "&&" | ";" | "#"))
        .collect::<Vec<_>>();
    let Some(executable) = first_command.first() else {
        return false;
    };
    let is_yt_dlp = Path::new(executable)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("yt-dlp"));
    let has_metadata_flag = first_command.iter().skip(1).any(|argument| {
        matches!(
            argument.as_str(),
            "--dump-json" | "--dump-single-json" | "--flat-playlist" | "--print"
        ) || argument.starts_with("--print=")
    });
    if !is_yt_dlp || !has_metadata_flag {
        return false;
    }

    let mut found_url = false;
    let mut found_line = false;
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        found_line = true;
        if !line.contains('\t') {
            return false;
        }
        for field in line.split('\t').filter(|field| contains_web_url(field)) {
            found_url = true;
            if !is_plain_web_url(field) {
                return false;
            }
        }
    }
    found_line && found_url
}

fn has_unquoted_shell_control(command: &str) -> bool {
    #[derive(Clone, Copy)]
    enum Quote {
        Unquoted,
        Single,
        Double,
    }

    let mut quote = Quote::Unquoted;
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        match quote {
            Quote::Unquoted => match ch {
                '\\' => {
                    chars.next();
                }
                '\'' => quote = Quote::Single,
                '"' => quote = Quote::Double,
                '\n' | '\r' | ';' | '|' | '&' | '#' | '`' | '<' | '>' => return true,
                '$' if chars.peek() == Some(&'(') => return true,
                _ => {}
            },
            Quote::Single => {
                if ch == '\'' {
                    quote = Quote::Unquoted;
                }
            }
            Quote::Double => match ch {
                '"' => quote = Quote::Unquoted,
                '`' => return true,
                '$' if chars.peek() == Some(&'(') => return true,
                '\\' => {
                    if chars
                        .peek()
                        .is_some_and(|next| matches!(next, '$' | '`' | '"' | '\\' | '\n'))
                    {
                        chars.next();
                    }
                }
                _ => {}
            },
        }
    }

    false
}

fn all_web_urls_are_machine_metadata(input: &str) -> bool {
    all_web_urls_are_assignment_values(input) || json_output_urls_are_metadata(input)
}

fn all_web_urls_are_assignment_values(input: &str) -> bool {
    let mut cursor = 0usize;
    let mut found_url = false;
    while cursor < input.len() {
        let Some((relative_start, scheme_len)) = next_web_url(&input[cursor..]) else {
            break;
        };
        found_url = true;
        let url_start = cursor.saturating_add(relative_start);
        let line_start = input[..url_start]
            .rfind('\n')
            .map_or(0, |index| index.saturating_add(1));
        let prefix = input[line_start..url_start].trim_end();
        let Some(key_prefix) = prefix.strip_suffix('=') else {
            return false;
        };
        let key = key_prefix.trim().to_ascii_lowercase();
        if !key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'))
            || !safe_metadata_url_key(&key)
        {
            return false;
        }
        cursor = url_start.saturating_add(scheme_len);
    }
    found_url
}

fn json_output_urls_are_metadata(input: &str) -> bool {
    if let Ok(value) = serde_json::from_str::<Value>(input) {
        return json_value_urls_are_metadata(&value).is_some_and(|found| found);
    }

    let mut found_url = false;
    let mut found_value = false;
    for line in input.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return false;
        };
        found_value = true;
        let Some(value_found_url) = json_value_urls_are_metadata(&value) else {
            return false;
        };
        found_url |= value_found_url;
    }
    found_value && found_url
}

/// Returns None when any URL-bearing JSON string is not proven metadata, otherwise whether a URL
/// was found. Generic `url` fields are accepted only inside yt-dlp-style media collections.
fn json_value_urls_are_metadata(value: &Value) -> Option<bool> {
    fn visit(value: &Value, ancestors: &mut Vec<String>, found_url: &mut bool) -> bool {
        match value {
            Value::Object(object) => object.iter().all(|(key, value)| {
                ancestors.push(key.to_ascii_lowercase());
                let safe = visit(value, ancestors, found_url);
                ancestors.pop();
                safe
            }),
            Value::Array(values) => values
                .iter()
                .all(|value| visit(value, ancestors, found_url)),
            Value::String(value) if contains_web_url(value) => {
                *found_url = true;
                let key = ancestors.last().map(String::as_str).unwrap_or_default();
                let nested_media_url = key == "url"
                    && ancestors.iter().any(|ancestor| {
                        matches!(
                            ancestor.as_str(),
                            "subtitles"
                                | "automatic_captions"
                                | "thumbnails"
                                | "formats"
                                | "requested_formats"
                                | "requested_subtitles"
                        )
                    });
                is_plain_web_url(value) && (safe_metadata_url_key(key) || nested_media_url)
            }
            _ => true,
        }
    }

    let mut found_url = false;
    let mut ancestors = Vec::new();
    visit(value, &mut ancestors, &mut found_url).then_some(found_url)
}

fn safe_metadata_url_key(key: &str) -> bool {
    matches!(
        key,
        "webpage_url"
            | "original_url"
            | "channel_url"
            | "uploader_url"
            | "thumbnail"
            | "thumbnail_url"
            | "manifest"
            | "manifest_url"
    )
}

fn contains_web_url(input: &str) -> bool {
    next_web_url(input).is_some()
}

fn is_plain_web_url(input: &str) -> bool {
    let input = input.trim();
    input.split_whitespace().count() == 1
        && (input
            .get(.."http://".len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
            || input
                .get(.."https://".len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://")))
}

fn web_urls_have_action_markers(input: &str) -> bool {
    let mut cursor = 0usize;
    while cursor < input.len() {
        let Some((relative_start, _)) = next_web_url(&input[cursor..]) else {
            return false;
        };
        let url_start = cursor.saturating_add(relative_start);
        let url_end = input[url_start..]
            .char_indices()
            .find_map(|(offset, ch)| {
                (offset > 0 && is_url_terminator(ch)).then_some(url_start.saturating_add(offset))
            })
            .unwrap_or(input.len());
        let url = &input[url_start..url_end];
        if [
            "login",
            "sign-in",
            "signin",
            "oauth",
            "authorize",
            "approval",
            "verification",
            "verify",
            "activate",
            "activation",
            "authenticate",
            "authentication",
            "authorization",
            "consent",
            "/device",
        ]
        .iter()
        .any(|marker| contains_ascii_case_insensitive(url, marker))
        {
            return true;
        }
        cursor = url_end;
    }
    false
}

fn strip_web_urls(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0usize;

    while cursor < input.len() {
        let Some((relative_start, _)) = next_web_url(&input[cursor..]) else {
            output.push_str(&input[cursor..]);
            break;
        };
        let url_start = cursor.saturating_add(relative_start);
        output.push_str(&input[cursor..url_start]);

        let url_end = input[url_start..]
            .char_indices()
            .find_map(|(offset, ch)| {
                (offset > 0 && is_url_terminator(ch)).then_some(url_start.saturating_add(offset))
            })
            .unwrap_or(input.len());
        output.push_str("[url]");
        cursor = url_end;
    }

    output
}

fn next_web_url(input: &str) -> Option<(usize, usize)> {
    let http = find_ascii_case_insensitive(input, "http://").map(|index| (index, "http://".len()));
    let https =
        find_ascii_case_insensitive(input, "https://").map(|index| (index, "https://".len()));
    match (http, https) {
        (Some(http), Some(https)) => Some(if http.0 <= https.0 { http } else { https }),
        (Some(http), None) => Some(http),
        (None, Some(https)) => Some(https),
        (None, None) => None,
    }
}

fn find_ascii_case_insensitive(input: &str, needle: &str) -> Option<usize> {
    let needle = needle.as_bytes();
    (!needle.is_empty()).then_some(())?;
    input
        .as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
}

fn is_url_terminator(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '\"' | '\'' | '<' | '>' | ')' | ']' | '}')
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

    fn active_exploring_exec() -> ExecCell {
        let first_command = vec!["cat".to_string(), "app.rs".to_string()];
        let first_parsed = vec![ParsedCommand::Read {
            cmd: first_command.join(" "),
            name: "app.rs".to_string(),
            path: "app.rs".into(),
        }];
        let mut cell = new_active_exec_command(
            "read-1".to_string(),
            first_command,
            first_parsed,
            ExecCommandSource::Agent,
            /*interaction_input*/ None,
            /*animations_enabled*/ false,
        );
        assert!(cell.complete_call(
            "read-1",
            CommandOutput::new(/*exit_code*/ 0, String::new()),
            Duration::from_millis(10),
        ));

        let second_command = vec!["cat".to_string(), "lib.rs".to_string()];
        let second_parsed = vec![ParsedCommand::Read {
            cmd: second_command.join(" "),
            name: "lib.rs".to_string(),
            path: "lib.rs".into(),
        }];
        assert!(cell.add_call(
            "read-2".to_string(),
            second_command,
            second_parsed,
            ExecCommandSource::Agent,
            /*interaction_input*/ None,
        ));
        cell
    }

    fn reasoning_summary(content: &str, transcript_only: bool) -> Arc<dyn HistoryCell> {
        Arc::new(ReasoningSummaryCell::new(
            "reasoning".to_string(),
            content.to_string(),
            Path::new("/repo"),
            transcript_only,
        ))
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
    fn large_exec_output_stays_out_of_compact_work_groups() {
        let at_limit = completed_command_with_output(
            "bounded",
            "generate report",
            0,
            "x".repeat(MAX_COMPACT_OUTPUT_SCAN_BYTES),
        );
        let bounded_cells = vec![at_limit, completed_read_exec("read-1", "report.txt")];
        assert!(
            compact_tool_group_at(&bounded_cells, 0, 100).is_some(),
            "the byte limit itself should remain eligible"
        );

        let over_limit = completed_command_with_output(
            "oversized",
            "generate report",
            0,
            "x".repeat(MAX_COMPACT_OUTPUT_SCAN_BYTES + 1),
        );
        let oversized_cells = vec![over_limit, completed_read_exec("read-2", "report.txt")];
        assert!(
            compact_tool_group_at(&oversized_cells, 0, 100).is_none(),
            "oversized output should remain expanded without full classification"
        );

        let blank_lines = completed_command_with_output(
            "blank-lines",
            "generate report",
            0,
            "\n".repeat(MAX_COMPACT_OUTPUT_SCAN_BYTES + 2),
        );
        let blank_line_cells = vec![blank_lines, completed_read_exec("read-3", "report.txt")];
        assert!(
            compact_tool_group_at(&blank_line_cells, 0, 100).is_none(),
            "oversized blank-line output must count its separators toward the limit"
        );
    }

    #[test]
    fn bounded_exec_transcript_preserves_leading_empty_lines() {
        let output = CommandOutput::new(/*exit_code*/ 0, "\n\nx".to_string());
        assert_eq!(
            bounded_transcript_for_compaction(&output).as_deref(),
            Some("\n\nx")
        );
    }

    #[test]
    fn action_markers_remain_ascii_case_insensitive_without_lowercase_copies() {
        assert!(tool_result_requires_user_action(
            "Please SIGN IN to continue"
        ));
        assert!(contains_web_url("Open HTTPS://example.com/device"));
        assert!(web_urls_have_action_markers(
            "Open HTTPS://example.com/OAuth/device"
        ));

        let metadata = completed_command_with_output(
            "metadata",
            "fetch artifact",
            0,
            "manifest_url=HTTPS://example.com/data".to_string(),
        );
        assert!(
            compact_tool_group_at(
                &[metadata, completed_read_exec("read-meta", "artifact.json")],
                0,
                100
            )
            .is_some(),
            "mixed-case metadata URLs should remain compactable"
        );

        let action = completed_command_with_output(
            "action",
            "service login",
            0,
            "OPEN HTTPS://example.com/OAuth/device".to_string(),
        );
        assert!(
            compact_tool_group_at(
                &[action, completed_read_exec("read-action", "artifact.json")],
                0,
                100
            )
            .is_none(),
            "mixed-case action URLs must remain visible"
        );
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
    fn live_exploring_cell_compacts_completed_work_and_keeps_current_command_visible() {
        let cell = active_exploring_exec();

        let snapshot = compact_active_exec_snapshot(&cell, /*width*/ 80)
            .expect("compact active exploring cell");
        let rendered = snapshot
            .lines
            .iter()
            .map(|line| render_line_text(&line.line))
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(rendered, "• Work: read app.rs\n  └ Running: cat lib.rs");
        assert!(snapshot.selection_projection.is_some());
    }

    #[test]
    fn live_summary_uses_the_canonical_shell_command_display() {
        let first_command = vec!["cat".to_string(), "app.rs".to_string()];
        let mut cell = new_active_exec_command(
            "read-1".to_string(),
            first_command.clone(),
            vec![ParsedCommand::Read {
                cmd: first_command.join(" "),
                name: "app.rs".to_string(),
                path: "app.rs".into(),
            }],
            ExecCommandSource::Agent,
            /*interaction_input*/ None,
            /*animations_enabled*/ false,
        );
        assert!(cell.complete_call(
            "read-1",
            CommandOutput::new(/*exit_code*/ 0, String::new()),
            Duration::from_millis(10),
        ));
        let wrapped = vec![
            "bash".to_string(),
            "-lc".to_string(),
            "cat 'lib file.rs'".to_string(),
        ];
        assert!(cell.add_call(
            "read-2".to_string(),
            wrapped,
            vec![ParsedCommand::Read {
                cmd: "cat 'lib file.rs'".to_string(),
                name: "lib file.rs".to_string(),
                path: "lib file.rs".into(),
            }],
            ExecCommandSource::Agent,
            /*interaction_input*/ None,
        ));

        let snapshot = compact_active_exec_snapshot(&cell, /*width*/ 80)
            .expect("compact active exploring cell");
        let rendered = snapshot
            .lines
            .iter()
            .map(|line| render_line_text(&line.line))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Running: cat 'lib file.rs'"));
        assert!(!rendered.contains("bash -lc"));
    }

    #[test]
    fn live_summary_falls_back_for_multiple_running_commands() {
        let mut cell = active_exploring_exec();
        let third_command = vec!["cat".to_string(), "main.rs".to_string()];
        assert!(cell.add_call(
            "read-3".to_string(),
            third_command.clone(),
            vec![ParsedCommand::Read {
                cmd: third_command.join(" "),
                name: "main.rs".to_string(),
                path: "main.rs".into(),
            }],
            ExecCommandSource::Agent,
            /*interaction_input*/ None,
        ));

        assert!(compact_active_exec_snapshot(&cell, /*width*/ 80).is_none());
    }

    #[test]
    fn live_exploring_cell_falls_back_when_output_or_failure_needs_full_visibility() {
        let mut streamed = active_exploring_exec();
        assert!(streamed.append_output("read-2", "waiting for confirmation\n"));
        assert!(compact_active_exec_snapshot(&streamed, 80).is_none());

        let mut failed = active_exploring_exec();
        assert!(failed.complete_call(
            "read-2",
            CommandOutput::new(/*exit_code*/ 7, "permission denied".to_string()),
            Duration::from_millis(10),
        ));
        assert!(compact_active_exec_snapshot(&failed, 80).is_none());

        let single = new_active_exec_command(
            "single".to_string(),
            vec!["cat".to_string(), "only.rs".to_string()],
            vec![ParsedCommand::Read {
                cmd: "cat only.rs".to_string(),
                name: "only.rs".to_string(),
                path: "only.rs".into(),
            }],
            ExecCommandSource::Agent,
            /*interaction_input*/ None,
            /*animations_enabled*/ false,
        );
        assert!(compact_active_exec_snapshot(&single, 80).is_none());
    }

    #[test]
    fn transcript_only_reasoning_bridges_tools_and_preserves_source_order() {
        let hidden_reasoning = reasoning_summary("hidden reasoning detail", true);
        assert!(hidden_reasoning.display_lines(/*width*/ 80).is_empty());
        assert!(hidden_reasoning.raw_lines().is_empty());
        assert!(
            render_lines_text(&hidden_reasoning.transcript_lines(/*width*/ 80))
                .contains("hidden reasoning detail")
        );
        let cells = vec![
            completed_read_exec("read-1", "app.rs"),
            hidden_reasoning,
            completed_read_exec("read-2", "lib.rs"),
        ];

        let group = compact_tool_group_at(&cells, 0, 120).expect("compact group");
        assert_eq!(group.consumed_cells, 3);
        assert_eq!(
            render_group_text(group),
            "▸ Work: read 2 files · Alt+I inspect · Alt+O all"
        );
        assert_eq!(trailing_compact_tool_run_start(&cells[..2]), 0);
        assert!(appended_cell_touches_compact_group(&cells, 120));

        let projected = project_owned_cells(&cells, /*compact_tool_groups*/ true);
        assert_eq!(projected.len(), 1);
        let raw = render_lines_text(&projected[0].raw_lines());
        let app_position = raw.find("$ cat app.rs").expect("first raw source");
        let lib_position = raw.find("$ cat lib.rs").expect("second raw source");
        assert!(
            app_position < lib_position,
            "raw source order changed: {raw}"
        );

        let inspected = latest_compact_tool_group_cells(&cells, 120).expect("latest group");
        assert_eq!(inspected.len(), 3);
        assert!(
            render_lines_text(&inspected[1].transcript_lines(/*width*/ 80))
                .contains("hidden reasoning detail")
        );
    }

    #[test]
    fn visible_reasoning_remains_a_hard_group_boundary() {
        let cells = vec![
            completed_read_exec("read-1", "app.rs"),
            completed_read_exec("read-2", "lib.rs"),
            reasoning_summary("Visible reasoning", false),
            completed_read_exec("read-3", "main.rs"),
            completed_read_exec("read-4", "mod.rs"),
        ];

        assert_eq!(
            compact_tool_group_at(&cells, 0, 100)
                .expect("first compact group")
                .consumed_cells,
            2
        );
        assert!(compact_tool_group_at(&cells, 2, 100).is_none());
        assert!(!appended_cell_touches_compact_group(&cells[..3], 100));
        assert_eq!(
            compact_tool_group_at(&cells, 3, 100)
                .expect("second compact group")
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
        assert!(rendered.contains("Visible reasoning"));
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
    fn oversized_mcp_results_never_compact() {
        let oversized_text = completed_mcp_result(
            "mcp-large-text",
            "files",
            "read",
            Ok(CallToolResult {
                content: vec![json!({
                    "type": "text",
                    "text": "x".repeat(MAX_TOOL_RESULT_SCAN_BYTES)
                })],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        );
        let oversized_structured = completed_mcp_result(
            "mcp-large-structured",
            "files",
            "read",
            Ok(CallToolResult {
                content: Vec::new(),
                structured_content: Some(json!({
                    "payload": "x".repeat(MAX_TOOL_RESULT_SCAN_BYTES)
                })),
                is_error: Some(false),
                meta: None,
            }),
        );

        for (index, result) in [oversized_text, oversized_structured]
            .into_iter()
            .enumerate()
        {
            let cells = vec![
                result,
                completed_read_exec(&format!("read-large-{index}"), "result.json"),
            ];
            assert!(
                compact_tool_group_at(&cells, 0, 100).is_none(),
                "oversized MCP result {index} should remain expanded"
            );
        }
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
    fn real_yt_dlp_tsv_and_json_metadata_can_join_work_groups() {
        let cases = [
            (
                "tsv",
                "yt-dlp --flat-playlist --dump-json ytsearch10:Ziak Grabba",
                concat!(
                    "pos7SwbEXoI\tZiak - Grabba\t140\thttps://www.youtube.com/watch?v=pos7SwbEXoI\n",
                    "abc123\tZiak - Akimbo\t161\thttps://www.youtube.com/watch?v=abc123"
                )
                .to_string(),
            ),
            (
                "json",
                "yt-dlp --dump-single-json --skip-download https://youtu.be/pos7SwbEXoI",
                json!({
                    "webpage_url": "https://www.youtube.com/watch?v=pos7SwbEXoI",
                    "thumbnail": "https://i.ytimg.com/vi/pos7SwbEXoI/maxresdefault.jpg",
                    "subtitles": {
                        "fr": [{ "url": "https://www.youtube.com/api/timedtext?v=pos7SwbEXoI" }]
                    }
                })
                .to_string(),
            ),
            (
                "quoted-playlist-url",
                "yt-dlp --print '%(id)s\\t%(webpage_url)s' 'https://www.youtube.com/playlist?list=PL123&si=abc'",
                "pos7SwbEXoI\thttps://www.youtube.com/watch?v=pos7SwbEXoI".to_string(),
            ),
        ];

        for (call_id, command, output) in cases {
            let url_output = completed_command_with_output(call_id, command, 0, output);
            let cells = vec![url_output, completed_read_exec("read", "metadata.json")];
            let group = compact_tool_group_at(&cells, 0, 120)
                .unwrap_or_else(|| panic!("real {call_id} metadata did not compact"));

            assert_eq!(group.consumed_cells, 2);
            assert_eq!(
                render_group_text(group),
                "▸ Work: read metadata.json · ran command · Alt+I inspect · Alt+O all"
            );
        }
    }

    #[test]
    fn explicit_approval_output_with_a_url_remains_a_hard_boundary() {
        let approval_required = completed_command_with_output(
            "approval",
            "deploy preview",
            0,
            "Approval required: confirm in https://example.com/deploy/123".to_string(),
        );
        let cells = vec![
            completed_read_exec("read-1", "app.rs"),
            completed_read_exec("read-2", "lib.rs"),
            approval_required,
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
        assert!(rendered.contains("Approval required"));
        assert!(rendered.contains("https://example.com/deploy/123"));
    }

    #[test]
    fn bare_and_prose_urls_remain_hard_boundaries() {
        for (call_id, output) in [
            ("bare", "https://github.com/login/device"),
            ("prose", "Please open https://example.com/device"),
            (
                "prefixed-assignment",
                "Open webpage_url=https://example.com/watch?v=123",
            ),
            (
                "auth-assignment",
                "webpage_url=https://github.com/login/device",
            ),
            (
                "activation-assignment",
                "webpage_url=https://www.youtube.com/activate",
            ),
            (
                "yt-warning",
                "WARNING: No supported JavaScript runtime was found. See https://github.com/yt-dlp/yt-dlp/wiki/EJS",
            ),
            ("spoofed-ytdlp", "Confirm\thttps://example.com/continue"),
        ] {
            let command = match call_id {
                "yt-warning" => "yt-dlp --dump-single-json example",
                "spoofed-ytdlp" => "printf Confirm # yt-dlp --print",
                _ => "service status",
            };
            let action_required =
                completed_command_with_output(call_id, command, 0, output.to_string());
            let cells = vec![action_required, completed_read_exec("read", "status.json")];

            assert!(
                compact_tool_group_at(&cells, 0, 100).is_none(),
                "user-facing URL was compacted: {output}"
            );
        }
    }

    #[test]
    fn chained_yt_dlp_commands_do_not_gain_the_metadata_exception() {
        for (call_id, script) in [
            (
                "semicolon-chain",
                "yt-dlp --print '%(id)s\\t%(webpage_url)s' URL;printf 'Confirm\\thttps://example.com/continue'",
            ),
            (
                "newline-chain",
                "yt-dlp --print '%(id)s\\t%(webpage_url)s' URL\nprintf 'Confirm\\thttps://example.com/continue'",
            ),
            (
                "pipe-chain",
                "yt-dlp --print '%(id)s\\t%(webpage_url)s' URL | printf 'Confirm\\thttps://example.com/continue'",
            ),
            (
                "attached-pipe-chain",
                "yt-dlp --print '%(id)s\\t%(webpage_url)s' URL|printf 'Confirm\\thttps://example.com/continue'",
            ),
            (
                "background-chain",
                "yt-dlp --print '%(id)s\\t%(webpage_url)s' URL & printf 'Confirm\\thttps://example.com/continue'",
            ),
        ] {
            let command = vec!["bash".to_string(), "-lc".to_string(), script.to_string()];
            let mut cell = new_active_exec_command(
                call_id.to_string(),
                command,
                Vec::new(),
                ExecCommandSource::Agent,
                /*interaction_input*/ None,
                /*animations_enabled*/ false,
            );
            assert!(cell.complete_call(
                call_id,
                CommandOutput::new(
                    /*exit_code*/ 0,
                    "Confirm\thttps://example.com/continue".to_string(),
                ),
                Duration::from_millis(10),
            ));
            let cells: Vec<Arc<dyn HistoryCell>> =
                vec![Arc::new(cell), completed_read_exec("read", "metadata.json")];

            assert!(
                compact_tool_group_at(&cells, 0, 100).is_none(),
                "chained shell output was compacted: {script:?}"
            );
        }
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
