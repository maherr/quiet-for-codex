//! TUI-only restoration of compact tool-call rows from legacy local rollouts.
//!
//! App-server intentionally limits legacy history materialization for scalability. Older rollouts
//! still contain response tool calls, but not the richer command lifecycle events used by current
//! history replay. This projector streams the local JSONL and restores only bounded call metadata;
//! output bodies are skipped by serde and are never decoded or retained.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use codex_app_server_protocol::Thread;
use codex_app_server_protocol::ThreadHistoryBuilder;
use codex_app_server_protocol::ThreadHistoryMode;
use codex_app_server_protocol::ThreadItem;
use codex_app_server_protocol::Turn;
use codex_app_server_protocol::TurnStatus;
use codex_protocol::ThreadId;
use codex_protocol::items::DynamicToolCallItem;
use codex_protocol::items::DynamicToolCallStatus;
use codex_protocol::items::TurnItem;
use codex_protocol::protocol::CompactedItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ItemCompletedEvent;
use codex_protocol::protocol::ItemStartedEvent;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::WorldStateItem;
use codex_rollout::open_rollout_line_reader;
use codex_utils_home_dir::find_codex_home;
use serde::Deserialize;
use serde::Serialize;
use serde::de::IgnoredAny;
use serde_json::Map;
use serde_json::Value;

const RESTORE_COUNTERS_FILENAME: &str = "quiet-legacy-tool-history-counters.json";
const RESTORE_COUNTERS_LOCK_FILENAME: &str = ".quiet-legacy-tool-history-counters.lock";
const RESTORE_COUNTER_LOCK_RETRIES: usize = 50;
const RESTORE_COUNTER_LOCK_DELAY: Duration = Duration::from_millis(5);
static RESTORE_COUNTER_PROCESS_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug)]
enum LegacyToolRestoreOutcome {
    RestoreSuccess,
    InvalidThread,
    RestoreReadFailure,
    TurnAssignmentConflict,
    BaselineMismatch,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyToolRestoreCounters {
    attempts: u64,
    restore_success: u64,
    invalid_thread: u64,
    restore_read_failure: u64,
    turn_assignment_conflict: u64,
    baseline_mismatch: u64,
}

impl LegacyToolRestoreCounters {
    fn validate(&self) -> io::Result<()> {
        let outcomes = [
            self.restore_success,
            self.invalid_thread,
            self.restore_read_failure,
            self.turn_assignment_conflict,
            self.baseline_mismatch,
        ]
        .into_iter()
        .try_fold(0_u64, |sum, value| {
            sum.checked_add(value).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "legacy tool history counter outcome total overflowed",
                )
            })
        })?;
        if outcomes > self.attempts {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "legacy tool history counter outcomes exceed attempts",
            ));
        }
        Ok(())
    }
}

fn increment_counter(value: &mut u64) -> io::Result<()> {
    *value = value.checked_add(1).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "legacy tool history counter overflowed",
        )
    })?;
    Ok(())
}

#[derive(Debug)]
pub(crate) struct LegacyToolReplay {
    turns: Vec<Turn>,
    inserted_call_ids: HashSet<String>,
    turn_assignment_conflict: bool,
}

#[derive(Deserialize)]
struct RecordHeader {
    #[serde(rename = "type")]
    record_type: String,
}

#[derive(Deserialize)]
struct RecordPayload<T> {
    payload: T,
}

#[derive(Clone, Deserialize)]
struct TurnMetadata {
    #[serde(default)]
    turn_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SlimResponseItem {
    FunctionCall {
        name: String,
        #[serde(default)]
        namespace: Option<String>,
        arguments: String,
        call_id: String,
        #[serde(default)]
        internal_chat_message_metadata_passthrough: Option<TurnMetadata>,
    },
    FunctionCallOutput {
        call_id: String,
        #[serde(rename = "output")]
        _output: IgnoredAny,
        #[serde(default)]
        #[serde(rename = "internal_chat_message_metadata_passthrough")]
        metadata: Option<TurnMetadata>,
    },
    CustomToolCall {
        call_id: String,
        name: String,
        #[serde(default)]
        namespace: Option<String>,
        #[serde(rename = "input")]
        _input: IgnoredAny,
        #[serde(default)]
        internal_chat_message_metadata_passthrough: Option<TurnMetadata>,
    },
    CustomToolCallOutput {
        call_id: String,
        #[serde(rename = "output")]
        _output: IgnoredAny,
        #[serde(default)]
        #[serde(rename = "internal_chat_message_metadata_passthrough")]
        metadata: Option<TurnMetadata>,
    },
    ToolSearchCall {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        call_id: Option<String>,
        #[serde(default)]
        status: Option<String>,
        execution: String,
        #[serde(rename = "arguments")]
        _arguments: IgnoredAny,
        #[serde(default)]
        internal_chat_message_metadata_passthrough: Option<TurnMetadata>,
    },
    ToolSearchOutput {
        #[serde(default)]
        call_id: Option<String>,
        #[serde(rename = "tools")]
        _tools: IgnoredAny,
        #[serde(default)]
        #[serde(rename = "internal_chat_message_metadata_passthrough")]
        metadata: Option<TurnMetadata>,
    },
    #[serde(other)]
    Other,
}

struct PendingResponseToolCall {
    turn_id: String,
    namespace: Option<String>,
    tool: String,
    arguments: Value,
}

pub(crate) async fn maybe_restore_local_legacy_tool_history(
    thread: &mut Thread,
    embedded: bool,
) -> bool {
    if !embedded
        || thread.history_mode != ThreadHistoryMode::Legacy
        || thread
            .turns
            .iter()
            .any(|turn| turn.status == TurnStatus::InProgress)
    {
        return false;
    }
    let Some(path) = thread.path.as_deref() else {
        return false;
    };
    let Ok(thread_id) = ThreadId::from_string(&thread.id) else {
        record_restore_attempt(Some(LegacyToolRestoreOutcome::InvalidThread)).await;
        tracing::warn!(
            thread_id = %thread.id,
            "skipping legacy tool history restoration for invalid thread id"
        );
        return false;
    };

    let mut replay = match rebuild_legacy_turns_with_tool_calls(path, thread_id).await {
        Ok(replay) => replay,
        Err(err) => {
            record_restore_attempt(Some(LegacyToolRestoreOutcome::RestoreReadFailure)).await;
            tracing::warn!(
                rollout_path = %path.display(),
                %err,
                "failed to restore legacy tool history; keeping app-server history"
            );
            return false;
        }
    };
    if replay.turn_assignment_conflict {
        record_restore_attempt(Some(LegacyToolRestoreOutcome::TurnAssignmentConflict)).await;
        tracing::warn!(
            rollout_path = %path.display(),
            "legacy tool history turn assignment conflict; keeping app-server history"
        );
        return false;
    }
    if replay.inserted_call_ids.is_empty() {
        record_restore_attempt(/*outcome*/ None).await;
        return false;
    }

    merge_authoritative_hook_prompts(&mut replay.turns, &thread.turns);
    merge_authoritative_turn_state(&mut replay.turns, &thread.turns);
    let expected = baseline_turns(&thread.turns, &replay.inserted_call_ids);
    let rebuilt = baseline_turns(&replay.turns, &replay.inserted_call_ids);
    if rebuilt != expected {
        record_restore_attempt(Some(LegacyToolRestoreOutcome::BaselineMismatch)).await;
        tracing::warn!(
            rollout_path = %path.display(),
            app_server_turns = expected.len(),
            rebuilt_turns = rebuilt.len(),
            "legacy tool history baseline mismatch; keeping app-server history"
        );
        return false;
    }

    let restored_call_count = replay.inserted_call_ids.len();
    thread.turns = replay.turns;
    record_restore_attempt(Some(LegacyToolRestoreOutcome::RestoreSuccess)).await;
    tracing::info!(
        rollout_path = %path.display(),
        restored_call_count,
        "restored compact legacy tool history"
    );
    true
}

async fn record_restore_attempt(outcome: Option<LegacyToolRestoreOutcome>) {
    let Ok(codex_home) = find_codex_home() else {
        tracing::warn!("failed to resolve Codex home for legacy tool history counters");
        return;
    };
    let codex_home = codex_home.to_path_buf();
    let result =
        tokio::task::spawn_blocking(move || update_restore_counters(codex_home.as_path(), outcome))
            .await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(err)) => tracing::warn!(
            %err,
            "failed to persist legacy tool history counters"
        ),
        Err(err) => tracing::warn!(
            %err,
            "legacy tool history counter task failed"
        ),
    }
}

fn update_restore_counters(
    codex_home: &Path,
    outcome: Option<LegacyToolRestoreOutcome>,
) -> io::Result<()> {
    // Serialize writers in this process before entering the bounded cross-process lock loop.
    let _process_guard = RESTORE_COUNTER_PROCESS_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::fs::create_dir_all(codex_home)?;
    let counters_path = codex_home.join(RESTORE_COUNTERS_FILENAME);
    let lock_path = codex_home.join(RESTORE_COUNTERS_LOCK_FILENAME);
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock_file = options.open(lock_path)?;
    let mut locked = false;
    for _ in 0..RESTORE_COUNTER_LOCK_RETRIES {
        match lock_file.try_lock() {
            Ok(()) => {
                locked = true;
                break;
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                std::thread::sleep(RESTORE_COUNTER_LOCK_DELAY);
            }
            Err(err) => return Err(err.into()),
        }
    }
    if !locked {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "legacy tool history counter lock remained busy",
        ));
    }

    let mut counters = match std::fs::read_to_string(&counters_path) {
        Ok(contents) => serde_json::from_str::<LegacyToolRestoreCounters>(&contents)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
        Err(err) if err.kind() == io::ErrorKind::NotFound => LegacyToolRestoreCounters::default(),
        Err(err) => return Err(err),
    };
    counters.validate()?;
    increment_counter(&mut counters.attempts)?;
    match outcome {
        Some(LegacyToolRestoreOutcome::RestoreSuccess) => {
            increment_counter(&mut counters.restore_success)?;
        }
        Some(LegacyToolRestoreOutcome::InvalidThread) => {
            increment_counter(&mut counters.invalid_thread)?;
        }
        Some(LegacyToolRestoreOutcome::RestoreReadFailure) => {
            increment_counter(&mut counters.restore_read_failure)?;
        }
        Some(LegacyToolRestoreOutcome::TurnAssignmentConflict) => {
            increment_counter(&mut counters.turn_assignment_conflict)?;
        }
        Some(LegacyToolRestoreOutcome::BaselineMismatch) => {
            increment_counter(&mut counters.baseline_mismatch)?;
        }
        None => {}
    }
    counters.validate()?;
    let mut contents = serde_json::to_string(&counters)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    contents.push('\n');
    codex_utils_path::write_atomically(&counters_path, &contents)
}

pub(crate) async fn rebuild_legacy_turns_with_tool_calls(
    path: &Path,
    thread_id: ThreadId,
) -> io::Result<LegacyToolReplay> {
    let mut reader = open_rollout_line_reader(path).await?;
    let mut builder = ThreadHistoryBuilder::new();
    let mut pending_calls = HashMap::<String, PendingResponseToolCall>::new();
    let mut inserted_call_ids = HashSet::new();
    let mut richer_item_ids = HashSet::new();
    let mut turn_assignment_conflict = false;
    let mut line_number = 0usize;

    while let Some(line) = reader.next_line().await? {
        line_number += 1;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let header: RecordHeader = serde_json::from_str(trimmed)
            .map_err(|err| rollout_parse_error(path, line_number, err))?;
        match header.record_type.as_str() {
            "event_msg" => {
                let event_header: RecordPayload<RecordHeader> = serde_json::from_str(trimmed)
                    .map_err(|err| rollout_parse_error(path, line_number, err))?;
                if materializes_legacy_history(&event_header.payload.record_type) {
                    let record: RecordPayload<EventMsg> = serde_json::from_str(trimmed)
                        .map_err(|err| rollout_parse_error(path, line_number, err))?;
                    handle_materialized_event(
                        &mut builder,
                        record.payload,
                        &mut richer_item_ids,
                        &mut inserted_call_ids,
                    );
                } else {
                    handle_noop(&mut builder);
                }
            }
            "compacted" => {
                builder.handle_rollout_item(&RolloutItem::Compacted(empty_compacted_item()));
            }
            "response_item" => {
                let record: RecordPayload<SlimResponseItem> = serde_json::from_str(trimmed)
                    .map_err(|err| rollout_parse_error(path, line_number, err))?;
                handle_response_item(
                    &mut builder,
                    &mut pending_calls,
                    &mut inserted_call_ids,
                    &richer_item_ids,
                    &mut turn_assignment_conflict,
                    thread_id,
                    record.payload,
                );
            }
            _ => handle_noop(&mut builder),
        }
    }

    Ok(LegacyToolReplay {
        turns: builder.finish(),
        inserted_call_ids,
        turn_assignment_conflict,
    })
}

fn handle_response_item(
    builder: &mut ThreadHistoryBuilder,
    pending_calls: &mut HashMap<String, PendingResponseToolCall>,
    inserted_call_ids: &mut HashSet<String>,
    richer_item_ids: &HashSet<String>,
    turn_assignment_conflict: &mut bool,
    thread_id: ThreadId,
    item: SlimResponseItem,
) {
    match item {
        SlimResponseItem::FunctionCall {
            name,
            namespace,
            arguments,
            call_id,
            internal_chat_message_metadata_passthrough,
        } => {
            let compact_arguments = compact_function_arguments(&name, &arguments);
            start_tool_call(
                builder,
                pending_calls,
                inserted_call_ids,
                thread_id,
                call_id,
                namespace,
                name,
                compact_arguments,
                internal_chat_message_metadata_passthrough,
            );
        }
        SlimResponseItem::CustomToolCall {
            call_id,
            name,
            namespace,
            _input: _,
            internal_chat_message_metadata_passthrough,
        } => {
            start_tool_call(
                builder,
                pending_calls,
                inserted_call_ids,
                thread_id,
                call_id,
                namespace,
                name,
                Value::Null,
                internal_chat_message_metadata_passthrough,
            );
        }
        SlimResponseItem::FunctionCallOutput {
            call_id,
            _output: _,
            metadata,
        }
        | SlimResponseItem::CustomToolCallOutput {
            call_id,
            _output: _,
            metadata,
        } => {
            complete_tool_call(
                builder,
                pending_calls,
                inserted_call_ids,
                richer_item_ids,
                turn_assignment_conflict,
                thread_id,
                &call_id,
                metadata,
            );
        }
        SlimResponseItem::ToolSearchCall {
            id,
            call_id,
            status,
            execution,
            _arguments: _,
            internal_chat_message_metadata_passthrough,
        } => {
            if execution == "server" && call_id.is_none() {
                let Some(id) = id.filter(|_| status.as_deref() == Some("completed")) else {
                    handle_noop(builder);
                    return;
                };
                start_tool_call(
                    builder,
                    pending_calls,
                    inserted_call_ids,
                    thread_id,
                    id.clone(),
                    Some(execution),
                    "tool_search".to_string(),
                    Value::Null,
                    internal_chat_message_metadata_passthrough,
                );
                complete_tool_call(
                    builder,
                    pending_calls,
                    inserted_call_ids,
                    richer_item_ids,
                    turn_assignment_conflict,
                    thread_id,
                    &id,
                    None,
                );
                return;
            }
            let Some(call_id) = call_id.or(id) else {
                handle_noop(builder);
                return;
            };
            start_tool_call(
                builder,
                pending_calls,
                inserted_call_ids,
                thread_id,
                call_id,
                (!execution.is_empty()).then_some(execution),
                "tool_search".to_string(),
                Value::Null,
                internal_chat_message_metadata_passthrough,
            );
        }
        SlimResponseItem::ToolSearchOutput {
            call_id,
            _tools: _,
            metadata,
        } => {
            let Some(call_id) = call_id else {
                handle_noop(builder);
                return;
            };
            complete_tool_call(
                builder,
                pending_calls,
                inserted_call_ids,
                richer_item_ids,
                turn_assignment_conflict,
                thread_id,
                &call_id,
                metadata,
            );
        }
        SlimResponseItem::Other => handle_noop(builder),
    }
}

#[allow(clippy::too_many_arguments)]
fn start_tool_call(
    builder: &mut ThreadHistoryBuilder,
    pending_calls: &mut HashMap<String, PendingResponseToolCall>,
    inserted_call_ids: &mut HashSet<String>,
    thread_id: ThreadId,
    call_id: String,
    namespace: Option<String>,
    tool: String,
    arguments: Value,
    metadata: Option<TurnMetadata>,
) {
    // Response items replayed at the beginning of a turn can retain the prior turn's metadata even
    // though their matching output and lifecycle events belong to the active turn. Prefer a real
    // active boundary, including older implicit turns, then fall back to item metadata.
    let active_turn_id = if builder.has_active_turn() {
        builder.active_turn_id().map(str::to_string)
    } else {
        None
    };
    let turn_id = active_turn_id
        .or_else(|| {
            metadata
                .and_then(|metadata| metadata.turn_id)
                .filter(|turn_id| !turn_id.is_empty())
        })
        .or_else(|| builder.active_turn_id().map(str::to_string));
    let Some(turn_id) = turn_id else {
        handle_noop(builder);
        return;
    };

    let item = dynamic_tool_item(
        &call_id,
        namespace.clone(),
        &tool,
        arguments.clone(),
        DynamicToolCallStatus::InProgress,
    );
    builder.handle_rollout_item(&RolloutItem::EventMsg(EventMsg::ItemStarted(
        ItemStartedEvent {
            thread_id,
            turn_id: turn_id.clone(),
            item,
            started_at_ms: 0,
        },
    )));
    pending_calls.insert(
        call_id.clone(),
        PendingResponseToolCall {
            turn_id,
            namespace,
            tool,
            arguments,
        },
    );
    inserted_call_ids.insert(call_id);
}

fn complete_tool_call(
    builder: &mut ThreadHistoryBuilder,
    pending_calls: &mut HashMap<String, PendingResponseToolCall>,
    inserted_call_ids: &mut HashSet<String>,
    richer_item_ids: &HashSet<String>,
    turn_assignment_conflict: &mut bool,
    thread_id: ThreadId,
    call_id: &str,
    output_metadata: Option<TurnMetadata>,
) {
    let Some(call) = pending_calls.remove(call_id) else {
        handle_noop(builder);
        return;
    };
    let turn_id = call.turn_id;
    if output_metadata
        .and_then(|metadata| metadata.turn_id)
        .filter(|output_turn_id| !output_turn_id.is_empty())
        .is_some_and(|output_turn_id| output_turn_id != turn_id)
    {
        *turn_assignment_conflict = true;
        handle_noop(builder);
        return;
    }
    if richer_item_ids.contains(call_id) {
        inserted_call_ids.remove(call_id);
        handle_noop(builder);
        return;
    }
    let item = dynamic_tool_item(
        call_id,
        call.namespace,
        &call.tool,
        call.arguments,
        DynamicToolCallStatus::Completed,
    );
    builder.handle_rollout_item(&RolloutItem::EventMsg(EventMsg::ItemCompleted(
        ItemCompletedEvent {
            thread_id,
            turn_id,
            item,
            started_at_ms: None,
            completed_at_ms: 0,
        },
    )));
}

fn handle_materialized_event(
    builder: &mut ThreadHistoryBuilder,
    event: EventMsg,
    richer_item_ids: &mut HashSet<String>,
    inserted_call_ids: &mut HashSet<String>,
) {
    let changes = builder.handle_rollout_item_with_changes(&RolloutItem::EventMsg(event));
    for change in changes.changed_items {
        let id = change.item.id().to_string();
        richer_item_ids.insert(id.clone());
        inserted_call_ids.remove(&id);
    }
}

fn dynamic_tool_item(
    call_id: &str,
    namespace: Option<String>,
    tool: &str,
    arguments: Value,
    status: DynamicToolCallStatus,
) -> TurnItem {
    TurnItem::DynamicToolCall(DynamicToolCallItem {
        id: call_id.to_string(),
        namespace,
        tool: tool.to_string(),
        arguments,
        status,
        content_items: None,
        success: None,
        error: None,
        duration: None,
    })
}

fn compact_function_arguments(tool: &str, arguments: &str) -> Value {
    if tool != "exec_command" {
        return Value::Null;
    }
    let Ok(Value::Object(arguments)) = serde_json::from_str::<Value>(arguments) else {
        return Value::Null;
    };
    let mut compact = Map::new();
    for key in ["cmd", "workdir"] {
        if let Some(value @ Value::String(_)) = arguments.get(key) {
            compact.insert(key.to_string(), value.clone());
        }
    }
    Value::Object(compact)
}

fn materializes_legacy_history(event_type: &str) -> bool {
    matches!(
        event_type,
        "user_message"
            | "agent_message"
            | "agent_reasoning"
            | "agent_reasoning_raw_content"
            | "web_search_end"
            | "patch_apply_end"
            | "mcp_tool_call_end"
            | "image_generation_end"
            | "sub_agent_activity"
            | "context_compacted"
            | "entered_review_mode"
            | "exited_review_mode"
            | "item_completed"
            | "thread_rolled_back"
            | "turn_aborted"
            | "task_started"
            | "turn_started"
            | "task_complete"
            | "turn_complete"
    )
}

fn baseline_turns(turns: &[Turn], inserted_call_ids: &HashSet<String>) -> Vec<Turn> {
    let mut turns = turns.to_vec();
    for turn in &mut turns {
        turn.items.retain(|item| match item {
            ThreadItem::DynamicToolCall {
                id,
                content_items,
                success,
                ..
            } => !(inserted_call_ids.contains(id) && content_items.is_none() && success.is_none()),
            _ => true,
        });
    }
    turns
}

fn merge_authoritative_hook_prompts(rebuilt: &mut [Turn], authoritative: &[Turn]) {
    for authoritative_turn in authoritative {
        let Some(rebuilt_turn) = rebuilt
            .iter_mut()
            .find(|turn| turn.id == authoritative_turn.id)
        else {
            continue;
        };
        for (authoritative_index, item) in authoritative_turn.items.iter().enumerate() {
            if !matches!(item, ThreadItem::HookPrompt { .. })
                || rebuilt_turn
                    .items
                    .iter()
                    .any(|rebuilt_item| rebuilt_item.id() == item.id())
            {
                continue;
            }

            let predecessor = authoritative_turn.items[..authoritative_index]
                .iter()
                .rev()
                .find_map(|candidate| {
                    rebuilt_turn
                        .items
                        .iter()
                        .position(|rebuilt_item| rebuilt_item.id() == candidate.id())
                });
            let successor = authoritative_turn.items[authoritative_index + 1..]
                .iter()
                .find_map(|candidate| {
                    rebuilt_turn
                        .items
                        .iter()
                        .position(|rebuilt_item| rebuilt_item.id() == candidate.id())
                });
            let insert_at = predecessor
                .map(|index| index + 1)
                .or(successor)
                .unwrap_or(rebuilt_turn.items.len());
            rebuilt_turn.items.insert(insert_at, item.clone());
        }
    }
}

fn merge_authoritative_turn_state(rebuilt: &mut [Turn], authoritative: &[Turn]) {
    for rebuilt_turn in rebuilt {
        let Some(authoritative_turn) = authoritative.iter().find(|turn| turn.id == rebuilt_turn.id)
        else {
            continue;
        };
        rebuilt_turn.items_view = authoritative_turn.items_view;
        rebuilt_turn.error = authoritative_turn.error.clone();
        rebuilt_turn.status = authoritative_turn.status.clone();
        rebuilt_turn.started_at = authoritative_turn.started_at;
        rebuilt_turn.completed_at = authoritative_turn.completed_at;
        rebuilt_turn.duration_ms = authoritative_turn.duration_ms;
    }
}

fn handle_noop(builder: &mut ThreadHistoryBuilder) {
    builder.handle_rollout_item(&RolloutItem::WorldState(WorldStateItem::full(Value::Null)));
}

fn empty_compacted_item() -> CompactedItem {
    CompactedItem {
        message: String::new(),
        replacement_history: None,
        window_number: None,
        first_window_id: None,
        previous_window_id: None,
        window_id: None,
    }
}

fn rollout_parse_error(path: &Path, line_number: usize, error: serde_json::Error) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "invalid rollout record at {}:{line_number}: {error}",
            path.display()
        ),
    )
}

#[cfg(test)]
#[path = "legacy_tool_history_tests.rs"]
mod tests;
