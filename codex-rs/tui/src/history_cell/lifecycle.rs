//! Quiet lifecycle cards for background terminals and collaborator fleets.
//!
//! These cells keep their source state behind a shared lock. The chat widget owns a clone of the
//! handle while the app owns the history-cell clone, so later lifecycle events can update one
//! committed card without manufacturing a row for every poll, write, wait, or resume event.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;
use std::time::Instant;

use codex_app_server_protocol::CollabAgentState;
use codex_app_server_protocol::CollabAgentStatus;
use codex_app_server_protocol::CollabAgentTool;
use codex_app_server_protocol::CollabAgentToolCallStatus;
use codex_app_server_protocol::SubAgentActivityKind;
use codex_app_server_protocol::ThreadItem;
use codex_utils_elapsed::format_duration;
use ratatui::prelude::*;
use ratatui::style::Stylize;

use super::HistoryCell;
use super::HistoryRenderMode;
use super::SelectionContribution;
use super::raw_lines_from_source;
use super::selection_contribution_from_display_lines;
use crate::render::line_utils::line_to_static;
use crate::text_formatting::truncate_text;
use crate::wrapping::RtOptions;
use crate::wrapping::adaptive_wrap_line;

const COMMAND_PREVIEW_GRAPHEMES: usize = 120;
const AGENT_MESSAGE_PREVIEW_GRAPHEMES: usize = 180;
const ERROR_PREVIEW_LINES: usize = 3;
const RESULT_PREVIEW_ITEMS: usize = 3;

#[derive(Clone, Debug)]
pub(crate) struct BackgroundTerminalLifecycleCell {
    state: Arc<RwLock<BackgroundTerminalLifecycleState>>,
}

#[derive(Debug)]
struct BackgroundTerminalLifecycleState {
    call_id: String,
    process_id: String,
    command: String,
    started_at: Instant,
    active: bool,
    interactions: Vec<TerminalInteraction>,
    output: String,
    outcome: TerminalOutcome,
}

#[derive(Debug)]
enum TerminalInteraction {
    Poll,
    Write(String),
}

#[derive(Debug, Default)]
enum TerminalOutcome {
    #[default]
    Running,
    Completed {
        exit_code: i32,
        duration: Duration,
    },
}

impl BackgroundTerminalLifecycleCell {
    pub(crate) fn new(
        call_id: String,
        process_id: String,
        command: String,
    ) -> BackgroundTerminalLifecycleCell {
        Self {
            state: Arc::new(RwLock::new(BackgroundTerminalLifecycleState {
                call_id,
                process_id,
                command,
                started_at: Instant::now(),
                active: false,
                interactions: Vec::new(),
                output: String::new(),
                outcome: TerminalOutcome::Running,
            })),
        }
    }

    pub(crate) fn call_id(&self) -> String {
        self.with_state(|state| state.call_id.clone())
    }

    /// Marks the card visible and returns whether this is the first promotion into history.
    pub(crate) fn activate(&self) -> bool {
        self.with_state_mut(|state| {
            let first = !state.active;
            state.active = true;
            first
        })
    }

    pub(crate) fn is_active(&self) -> bool {
        self.with_state(|state| state.active)
    }

    pub(crate) fn record_interaction(&self, stdin: String) {
        self.with_state_mut(|state| {
            if stdin.is_empty() {
                state.interactions.push(TerminalInteraction::Poll);
            } else {
                state.interactions.push(TerminalInteraction::Write(stdin));
            }
        });
    }

    pub(crate) fn append_output(&self, chunk: &str) {
        if chunk.is_empty() {
            return;
        }
        self.with_state_mut(|state| state.output.push_str(chunk));
    }

    pub(crate) fn complete(
        &self,
        exit_code: i32,
        duration: Option<Duration>,
        aggregated_output: Option<String>,
    ) {
        self.with_state_mut(|state| {
            if let Some(output) = aggregated_output
                && !output.is_empty()
            {
                state.output = output;
            }
            state.outcome = TerminalOutcome::Completed {
                exit_code,
                duration: duration.unwrap_or_else(|| state.started_at.elapsed()),
            };
        });
    }

    fn with_state<T>(&self, f: impl FnOnce(&BackgroundTerminalLifecycleState) -> T) -> T {
        let state = self
            .state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&state)
    }

    fn with_state_mut<T>(&self, f: impl FnOnce(&mut BackgroundTerminalLifecycleState) -> T) -> T {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut state)
    }
}

impl HistoryCell for BackgroundTerminalLifecycleCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.with_state(|state| terminal_display_lines(state, width))
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        self.with_state(terminal_raw_lines)
    }

    fn transcript_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.raw_lines()
    }

    fn selection_contribution(&self, width: u16, mode: HistoryRenderMode) -> SelectionContribution {
        selection_contribution_from_display_lines(self.display_lines_for_mode(width, mode), width)
    }
}

fn terminal_display_lines(
    state: &BackgroundTerminalLifecycleState,
    width: u16,
) -> Vec<Line<'static>> {
    if !state.active || width == 0 {
        return Vec::new();
    }

    let command = collapse_whitespace(&state.command);
    let command = truncate_text(&command, COMMAND_PREVIEW_GRAPHEMES);
    let interaction_count = state.interactions.len();
    let interaction_label = match interaction_count {
        0 => None,
        1 => Some("1 interaction".to_string()),
        count => Some(format!("{count} interactions")),
    };

    let mut spans = vec!["▸ ".dim(), "Terminal".bold(), ": ".dim(), command.into()];
    if let Some(interaction_label) = interaction_label {
        spans.extend([" · ".dim(), interaction_label.into()]);
    }

    match state.outcome {
        TerminalOutcome::Running => {
            spans.extend([
                " · ".dim(),
                format!(
                    "running {}",
                    format_coarse_elapsed(state.started_at.elapsed())
                )
                .cyan(),
            ]);
        }
        TerminalOutcome::Completed {
            exit_code: 0,
            duration,
        } => {
            spans.extend([
                " · ".dim(),
                format!("completed in {}", format_duration(duration)).green(),
            ]);
        }
        TerminalOutcome::Completed {
            exit_code,
            duration,
        } => {
            spans.extend([
                " · ".dim(),
                format!("failed (exit {exit_code}) in {}", format_duration(duration)).red(),
            ]);
        }
    }
    spans.extend([" · ".dim(), "Ctrl+T details".dim()]);

    let mut lines = adaptive_wrap_line(
        &Line::from(spans),
        RtOptions::new(usize::from(width.max(1)))
            .initial_indent(Line::from(""))
            .subsequent_indent(Line::from("  ")),
    )
    .into_iter()
    .map(|line| line_to_static(&line))
    .collect::<Vec<_>>();

    if matches!(state.outcome, TerminalOutcome::Completed { exit_code, .. } if exit_code != 0) {
        let errors = last_nonempty_lines(&state.output, ERROR_PREVIEW_LINES);
        lines.extend(errors.into_iter().enumerate().map(|(index, error)| {
            let prefix = if index == 0 { "  └ " } else { "    " };
            vec![prefix.dim(), error.red()].into()
        }));
    }
    lines
}

fn terminal_raw_lines(state: &BackgroundTerminalLifecycleState) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(format!("Background terminal {}", state.process_id)),
        Line::from(format!("$ {}", state.command)),
    ];
    for interaction in &state.interactions {
        match interaction {
            TerminalInteraction::Poll => lines.push(Line::from("[poll]")),
            TerminalInteraction::Write(stdin) => {
                lines.push(Line::from("[stdin]"));
                lines.extend(raw_lines_from_source(stdin));
            }
        }
    }
    if !state.output.is_empty() {
        lines.push(Line::from("[output]"));
        lines.extend(raw_lines_from_source(&state.output));
    }
    match state.outcome {
        TerminalOutcome::Running => lines.push(Line::from(format!(
            "[running {}]",
            format_coarse_elapsed(state.started_at.elapsed())
        ))),
        TerminalOutcome::Completed {
            exit_code,
            duration,
        } => lines.push(Line::from(format!(
            "[exit {exit_code} after {}]",
            format_duration(duration)
        ))),
    }
    lines
}

#[derive(Clone, Debug)]
pub(crate) struct AgentFleetLifecycleCell {
    state: Arc<RwLock<AgentFleetLifecycleState>>,
}

#[derive(Debug)]
struct AgentFleetLifecycleState {
    started_at: Instant,
    agents: BTreeMap<String, FleetAgent>,
    interactions: usize,
    raw_events: Vec<String>,
    operation_errors: Vec<String>,
    pending_spawns: BTreeMap<String, PendingSpawn>,
}

#[derive(Clone, Debug, Default)]
struct PendingSpawn {
    task: Option<String>,
    configuration: Option<String>,
}

#[derive(Debug)]
struct FleetAgent {
    label: String,
    task: Option<String>,
    configuration: Option<String>,
    status: FleetAgentStatus,
    message: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FleetAgentStatus {
    Pending,
    Running,
    Interrupted,
    Completed,
    Failed,
    Shutdown,
}

impl AgentFleetLifecycleCell {
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(AgentFleetLifecycleState {
                started_at: Instant::now(),
                agents: BTreeMap::new(),
                interactions: 0,
                raw_events: Vec::new(),
                operation_errors: Vec::new(),
                pending_spawns: BTreeMap::new(),
            })),
        }
    }

    pub(crate) fn update_tool_call(&self, item: &ThreadItem, labels: &BTreeMap<String, String>) {
        let ThreadItem::CollabAgentToolCall {
            id,
            tool,
            status,
            receiver_thread_ids,
            prompt,
            model,
            reasoning_effort,
            agents_states,
            ..
        } = item
        else {
            return;
        };

        self.with_state_mut(|state| {
            let spawn_detail = if matches!(tool, CollabAgentTool::SpawnAgent) {
                let current = PendingSpawn {
                    task: prompt
                        .as_deref()
                        .filter(|prompt| !prompt.trim().is_empty())
                        .map(collapse_whitespace),
                    configuration: spawn_configuration(model.as_deref(), reasoning_effort.clone()),
                };
                if matches!(status, CollabAgentToolCallStatus::InProgress) {
                    state.pending_spawns.insert(id.clone(), current.clone());
                    Some(current)
                } else {
                    state.pending_spawns.remove(id).or(Some(current))
                }
            } else {
                None
            };
            state.interactions += usize::from(
                !matches!(tool, CollabAgentTool::SpawnAgent)
                    && !matches!(status, CollabAgentToolCallStatus::InProgress),
            );
            state.raw_events.push(collab_raw_event(
                id,
                tool,
                status,
                receiver_thread_ids,
                prompt.as_deref(),
            ));

            for thread_id in receiver_thread_ids {
                let label = labels
                    .get(thread_id)
                    .cloned()
                    .unwrap_or_else(|| short_agent_id(thread_id));
                let agent = state
                    .agents
                    .entry(thread_id.clone())
                    .or_insert_with(|| FleetAgent {
                        label,
                        task: None,
                        configuration: None,
                        status: FleetAgentStatus::Pending,
                        message: None,
                    });
                if matches!(tool, CollabAgentTool::SpawnAgent)
                    && let Some(spawn_detail) = &spawn_detail
                {
                    if spawn_detail.task.is_some() {
                        agent.task.clone_from(&spawn_detail.task);
                    }
                    if spawn_detail.configuration.is_some() {
                        agent.configuration.clone_from(&spawn_detail.configuration);
                    }
                }
                if matches!(tool, CollabAgentTool::SpawnAgent)
                    && matches!(status, CollabAgentToolCallStatus::Completed)
                    && matches!(agent.status, FleetAgentStatus::Pending)
                {
                    agent.status = FleetAgentStatus::Running;
                }
            }

            for (thread_id, agent_state) in agents_states {
                let label = labels
                    .get(thread_id)
                    .cloned()
                    .unwrap_or_else(|| short_agent_id(thread_id));
                let agent = state
                    .agents
                    .entry(thread_id.clone())
                    .or_insert_with(|| FleetAgent {
                        label,
                        task: None,
                        configuration: None,
                        status: FleetAgentStatus::Pending,
                        message: None,
                    });
                apply_agent_state(agent, agent_state);
            }

            if matches!(status, CollabAgentToolCallStatus::Failed) {
                let error = format!("{} failed", collab_tool_label(tool));
                state.operation_errors.push(error);
            }
        });
    }

    pub(crate) fn update_activity(&self, item: &ThreadItem) {
        let ThreadItem::SubAgentActivity {
            kind,
            agent_thread_id,
            agent_path,
            ..
        } = item
        else {
            return;
        };
        self.with_state_mut(|state| {
            state.raw_events.push(format!(
                "{} {agent_path}",
                match kind {
                    SubAgentActivityKind::Started => "started",
                    SubAgentActivityKind::Interacted => "interacted with",
                    SubAgentActivityKind::Interrupted => "interrupted",
                }
            ));
            let agent = state
                .agents
                .entry(agent_thread_id.clone())
                .or_insert_with(|| FleetAgent {
                    label: agent_path.clone(),
                    task: None,
                    configuration: None,
                    status: FleetAgentStatus::Pending,
                    message: None,
                });
            agent.label = agent_path.clone();
            agent.status = match kind {
                SubAgentActivityKind::Started | SubAgentActivityKind::Interacted => {
                    FleetAgentStatus::Running
                }
                SubAgentActivityKind::Interrupted => FleetAgentStatus::Interrupted,
            };
        });
    }

    fn with_state<T>(&self, f: impl FnOnce(&AgentFleetLifecycleState) -> T) -> T {
        let state = self
            .state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&state)
    }

    fn with_state_mut<T>(&self, f: impl FnOnce(&mut AgentFleetLifecycleState) -> T) -> T {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut state)
    }
}

impl HistoryCell for AgentFleetLifecycleCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.with_state(|state| agent_fleet_display_lines(state, width))
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        self.with_state(agent_fleet_raw_lines)
    }

    fn transcript_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.raw_lines()
    }

    fn selection_contribution(&self, width: u16, mode: HistoryRenderMode) -> SelectionContribution {
        selection_contribution_from_display_lines(self.display_lines_for_mode(width, mode), width)
    }
}

fn agent_fleet_display_lines(state: &AgentFleetLifecycleState, width: u16) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let counts = FleetCounts::from_state(state);
    let mut parts = Vec::new();
    if counts.running > 0 {
        parts.push(format!("{} running", counts.running));
    }
    if counts.completed > 0 {
        parts.push(format!("{} done", counts.completed));
    }
    if counts.failed > 0 {
        parts.push(format!("{} failed", counts.failed));
    }
    if counts.other > 0 {
        parts.push(format!("{} paused", counts.other));
    }
    if state.agents.is_empty() {
        parts.push("spawning".to_string());
    }

    let total = state.agents.len();
    let mut spans = vec![
        "▸ ".dim(),
        "Agents".bold(),
        ": ".dim(),
        total.to_string().into(),
        " · ".dim(),
        parts.join(" · ").into(),
    ];
    if state.interactions > 0 {
        let interaction_label = if state.interactions == 1 {
            "1 interaction".to_string()
        } else {
            format!("{} interactions", state.interactions)
        };
        spans.extend([" · ".dim(), interaction_label.dim()]);
    }
    spans.extend([
        " · ".dim(),
        format_coarse_elapsed(state.started_at.elapsed()).dim(),
    ]);
    spans.extend([" · ".dim(), "/agent details".dim()]);

    let mut lines = adaptive_wrap_line(
        &Line::from(spans),
        RtOptions::new(usize::from(width.max(1)))
            .initial_indent(Line::from(""))
            .subsequent_indent(Line::from("  ")),
    )
    .into_iter()
    .map(|line| line_to_static(&line))
    .collect::<Vec<_>>();

    let mut important = state
        .agents
        .values()
        .filter(|agent| {
            matches!(
                agent.status,
                FleetAgentStatus::Completed | FleetAgentStatus::Failed
            ) && agent
                .message
                .as_ref()
                .is_some_and(|message| !message.trim().is_empty())
        })
        .collect::<Vec<_>>();
    important.sort_by_key(|agent| !matches!(agent.status, FleetAgentStatus::Failed));
    let mut shown = 0usize;
    for agent in important.into_iter().take(RESULT_PREVIEW_ITEMS) {
        let message = collapse_whitespace(agent.message.as_deref().unwrap_or_default());
        let message = truncate_text(&message, AGENT_MESSAGE_PREVIEW_GRAPHEMES);
        let (marker, style) = if matches!(agent.status, FleetAgentStatus::Failed) {
            ("✗", Style::default().red())
        } else {
            ("✓", Style::default().green())
        };
        lines.push(Line::from(vec![
            "  └ ".dim(),
            Span::styled(marker, style),
            " ".into(),
            agent.label.clone().cyan(),
            ": ".dim(),
            message.into(),
        ]));
        shown += 1;
    }
    for agent in state
        .agents
        .values()
        .filter(|agent| {
            matches!(
                agent.status,
                FleetAgentStatus::Pending | FleetAgentStatus::Running
            ) && agent.task.is_some()
        })
        .take(RESULT_PREVIEW_ITEMS.saturating_sub(shown))
    {
        let task = truncate_text(
            agent.task.as_deref().unwrap_or_default(),
            AGENT_MESSAGE_PREVIEW_GRAPHEMES,
        );
        lines.push(Line::from(vec![
            "  └ ".dim(),
            "… ".cyan(),
            agent.label.clone().cyan(),
            agent
                .configuration
                .as_ref()
                .map(|configuration| format!(" ({configuration})"))
                .unwrap_or_default()
                .magenta(),
            ": ".dim(),
            task.into(),
        ]));
    }
    for error in &state.operation_errors {
        lines.push(vec!["  └ ✗ ".red(), error.clone().red()].into());
    }
    lines
}

fn agent_fleet_raw_lines(state: &AgentFleetLifecycleState) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(format!(
        "Agent fleet after {}",
        format_coarse_elapsed(state.started_at.elapsed())
    ))];
    lines.extend(
        state
            .raw_events
            .iter()
            .map(|event| Line::from(format!("[event] {event}"))),
    );
    for agent in state.agents.values() {
        let status = match agent.status {
            FleetAgentStatus::Pending => "pending",
            FleetAgentStatus::Running => "running",
            FleetAgentStatus::Interrupted => "interrupted",
            FleetAgentStatus::Completed => "completed",
            FleetAgentStatus::Failed => "failed",
            FleetAgentStatus::Shutdown => "shutdown",
        };
        lines.push(Line::from(format!("[agent] {}: {status}", agent.label)));
        if let Some(configuration) = &agent.configuration {
            lines.push(Line::from(format!("[configuration] {configuration}")));
        }
        if let Some(task) = &agent.task {
            lines.push(Line::from(format!("[task] {task}")));
        }
        if let Some(message) = &agent.message {
            lines.extend(raw_lines_from_source(message));
        }
    }
    for error in &state.operation_errors {
        lines.push(Line::from(format!("[error] {error}")));
    }
    lines
}

#[derive(Default)]
struct FleetCounts {
    running: usize,
    completed: usize,
    failed: usize,
    other: usize,
}

impl FleetCounts {
    fn from_state(state: &AgentFleetLifecycleState) -> Self {
        let mut counts = Self::default();
        for agent in state.agents.values() {
            match agent.status {
                FleetAgentStatus::Pending | FleetAgentStatus::Running => counts.running += 1,
                FleetAgentStatus::Completed | FleetAgentStatus::Shutdown => counts.completed += 1,
                FleetAgentStatus::Failed => counts.failed += 1,
                FleetAgentStatus::Interrupted => counts.other += 1,
            }
        }
        counts
    }
}

fn apply_agent_state(agent: &mut FleetAgent, state: &CollabAgentState) {
    agent.status = match state.status {
        CollabAgentStatus::PendingInit => FleetAgentStatus::Pending,
        CollabAgentStatus::Running => FleetAgentStatus::Running,
        CollabAgentStatus::Interrupted => FleetAgentStatus::Interrupted,
        CollabAgentStatus::Completed => FleetAgentStatus::Completed,
        CollabAgentStatus::Errored | CollabAgentStatus::NotFound => FleetAgentStatus::Failed,
        CollabAgentStatus::Shutdown => FleetAgentStatus::Shutdown,
    };
    if state.message.is_some() {
        agent.message.clone_from(&state.message);
    }
}

fn collab_raw_event(
    id: &str,
    tool: &CollabAgentTool,
    status: &CollabAgentToolCallStatus,
    receiver_thread_ids: &[String],
    prompt: Option<&str>,
) -> String {
    let status = match status {
        CollabAgentToolCallStatus::InProgress => "started",
        CollabAgentToolCallStatus::Completed => "completed",
        CollabAgentToolCallStatus::Failed => "failed",
    };
    let receivers = if receiver_thread_ids.is_empty() {
        String::new()
    } else {
        format!(" -> {}", receiver_thread_ids.join(", "))
    };
    let prompt = prompt
        .filter(|prompt| !prompt.trim().is_empty())
        .map(|prompt| format!(" | {}", collapse_whitespace(prompt)))
        .unwrap_or_default();
    format!(
        "{} {status} ({id}){receivers}{prompt}",
        collab_tool_label(tool)
    )
}

fn collab_tool_label(tool: &CollabAgentTool) -> &'static str {
    match tool {
        CollabAgentTool::SpawnAgent => "spawn",
        CollabAgentTool::SendInput => "send",
        CollabAgentTool::ResumeAgent => "resume",
        CollabAgentTool::Wait => "wait",
        CollabAgentTool::CloseAgent => "close",
    }
}

fn short_agent_id(id: &str) -> String {
    truncate_text(id, 12)
}

fn spawn_configuration(
    model: Option<&str>,
    reasoning_effort: Option<codex_protocol::openai_models::ReasoningEffort>,
) -> Option<String> {
    let model = model.map(str::trim).filter(|model| !model.is_empty());
    match (model, reasoning_effort) {
        (Some(model), Some(effort)) => Some(format!("{model} {effort}")),
        (Some(model), None) => Some(model.to_string()),
        (None, Some(effort)) => Some(effort.to_string()),
        (None, None) => None,
    }
}

fn format_coarse_elapsed(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    }
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn last_nonempty_lines(text: &str, limit: usize) -> Vec<String> {
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if lines.len() > limit {
        lines.drain(..lines.len() - limit);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_app_server_protocol::CollabAgentToolCallStatus;
    use codex_protocol::ThreadId;
    use pretty_assertions::assert_eq;

    fn rendered(cell: &dyn HistoryCell) -> String {
        cell.display_lines(100)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn background_terminal_lifecycle_keeps_interleaved_events_in_one_card() {
        let cell = BackgroundTerminalLifecycleCell::new(
            "call-1".to_string(),
            "proc-1".to_string(),
            "cargo test -p codex-tui".to_string(),
        );
        assert!(cell.activate());
        cell.record_interaction(String::new());
        cell.append_output("running suite\n");
        cell.record_interaction("y\n".to_string());
        cell.complete(0, Some(Duration::from_secs(42)), None);

        assert_eq!(
            rendered(&cell),
            "▸ Terminal: cargo test -p codex-tui · 2 interactions · completed in 42.00s · Ctrl+T details"
        );
        let raw = cell
            .raw_lines()
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(raw.contains("[poll]"));
        assert!(raw.contains("[stdin]\ny"));
        assert!(raw.contains("[output]\nrunning suite"));
    }

    #[test]
    fn background_terminal_failure_is_never_hidden() {
        let cell = BackgroundTerminalLifecycleCell::new(
            "call-2".to_string(),
            "proc-2".to_string(),
            "cargo test".to_string(),
        );
        cell.activate();
        cell.complete(
            101,
            Some(Duration::from_millis(1500)),
            Some("compile line\nerror: assertion failed\nFAILED".to_string()),
        );
        let output = rendered(&cell);
        assert!(output.contains("failed (exit 101)"));
        assert!(output.contains("error: assertion failed"));
        assert!(output.contains("FAILED"));
    }

    #[test]
    fn agent_fleet_card_preserves_results_and_failures() {
        let cell = AgentFleetLifecycleCell::new();
        let a = ThreadId::new().to_string();
        let b = ThreadId::new().to_string();
        let labels = BTreeMap::from([
            (a.clone(), "builder".to_string()),
            (b.clone(), "reviewer".to_string()),
        ]);
        cell.update_tool_call(
            &ThreadItem::CollabAgentToolCall {
                id: "spawn-a".to_string(),
                tool: CollabAgentTool::SpawnAgent,
                status: CollabAgentToolCallStatus::Completed,
                sender_thread_id: ThreadId::new().to_string(),
                receiver_thread_ids: vec![a.clone()],
                prompt: Some("implement it".to_string()),
                model: None,
                reasoning_effort: None,
                agents_states: BTreeMap::new().into_iter().collect(),
            },
            &labels,
        );
        cell.update_tool_call(
            &ThreadItem::CollabAgentToolCall {
                id: "wait".to_string(),
                tool: CollabAgentTool::Wait,
                status: CollabAgentToolCallStatus::Completed,
                sender_thread_id: ThreadId::new().to_string(),
                receiver_thread_ids: vec![a.clone(), b.clone()],
                prompt: None,
                model: None,
                reasoning_effort: None,
                agents_states: BTreeMap::from([
                    (
                        a,
                        CollabAgentState {
                            status: CollabAgentStatus::Completed,
                            message: Some("implemented and tested".to_string()),
                        },
                    ),
                    (
                        b,
                        CollabAgentState {
                            status: CollabAgentStatus::Errored,
                            message: Some("review timed out".to_string()),
                        },
                    ),
                ])
                .into_iter()
                .collect(),
            },
            &labels,
        );

        let output = rendered(&cell);
        assert!(output.contains("2 · 1 done · 1 failed"));
        assert!(output.contains("builder: implemented and tested"));
        assert!(output.contains("reviewer: review timed out"));
        assert!(
            cell.raw_lines().len() >= 6,
            "raw event trail should be retained"
        );
    }

    #[test]
    fn lifecycle_cells_replay_from_shared_source_state() {
        let cell = BackgroundTerminalLifecycleCell::new(
            "call-replay".to_string(),
            "proc-replay".to_string(),
            "make check".to_string(),
        );
        let replay = cell.clone();
        cell.activate();
        cell.record_interaction(String::new());
        cell.complete(0, Some(Duration::from_secs(3)), Some("ok\n".to_string()));

        assert_eq!(rendered(&cell), rendered(&replay));
        assert_eq!(cell.raw_lines(), replay.raw_lines());
    }
}
