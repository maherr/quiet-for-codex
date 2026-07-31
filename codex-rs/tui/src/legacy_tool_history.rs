//! TUI-only restoration of compact tool-call rows from legacy local rollouts.
//!
//! App-server intentionally limits legacy history materialization for scalability. Older rollouts
//! still contain response tool calls, but not the richer command lifecycle events used by current
//! history replay. This projector streams the local JSONL and restores only bounded call metadata;
//! output bodies are skipped by serde and are never decoded or retained.

use std::collections::HashMap;
use std::collections::HashSet;
use std::io;
use std::path::Path;

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
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Map;
use serde_json::Value;

#[derive(Debug)]
pub(crate) struct LegacyToolReplay {
    turns: Vec<Turn>,
    inserted_call_ids: HashSet<String>,
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
        _metadata: Option<TurnMetadata>,
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
        _metadata: Option<TurnMetadata>,
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
        _metadata: Option<TurnMetadata>,
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
        tracing::warn!(
            thread_id = %thread.id,
            "skipping legacy tool history restoration for invalid thread id"
        );
        return false;
    };

    let mut replay = match rebuild_legacy_turns_with_tool_calls(path, thread_id).await {
        Ok(replay) => replay,
        Err(err) => {
            tracing::warn!(
                rollout_path = %path.display(),
                %err,
                "failed to restore legacy tool history; keeping app-server history"
            );
            return false;
        }
    };
    if replay.inserted_call_ids.is_empty() {
        return false;
    }

    merge_authoritative_hook_prompts(&mut replay.turns, &thread.turns);
    let expected = baseline_turns(&thread.turns, &replay.inserted_call_ids);
    let rebuilt = baseline_turns(&replay.turns, &replay.inserted_call_ids);
    if rebuilt != expected {
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
    tracing::info!(
        rollout_path = %path.display(),
        restored_call_count,
        "restored compact legacy tool history"
    );
    true
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
    })
}

fn handle_response_item(
    builder: &mut ThreadHistoryBuilder,
    pending_calls: &mut HashMap<String, PendingResponseToolCall>,
    inserted_call_ids: &mut HashSet<String>,
    richer_item_ids: &HashSet<String>,
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
            _metadata: _,
        }
        | SlimResponseItem::CustomToolCallOutput {
            call_id,
            _output: _,
            _metadata: _,
        } => {
            complete_tool_call(
                builder,
                pending_calls,
                inserted_call_ids,
                richer_item_ids,
                thread_id,
                &call_id,
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
                    thread_id,
                    &id,
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
            _metadata: _,
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
                thread_id,
                &call_id,
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
    let turn_id = metadata
        .and_then(|metadata| metadata.turn_id)
        .filter(|turn_id| !turn_id.is_empty())
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
    thread_id: ThreadId,
    call_id: &str,
) {
    let Some(call) = pending_calls.remove(call_id) else {
        handle_noop(builder);
        return;
    };
    let turn_id = call.turn_id;
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
