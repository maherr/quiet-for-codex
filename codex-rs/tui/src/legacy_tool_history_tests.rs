use super::*;
use serde_json::json;
use tempfile::TempDir;

fn record(item_type: &str, payload: Value) -> Value {
    json!({
        "timestamp": "2026-01-01T00:00:00Z",
        "type": item_type,
        "payload": payload,
    })
}

fn write_records(path: &Path, records: &[Value]) -> io::Result<()> {
    let mut text = String::new();
    for record in records {
        text.push_str(&serde_json::to_string(record).expect("serialize fixture"));
        text.push('\n');
    }
    std::fs::write(path, text)
}

fn task_started(turn_id: &str) -> Value {
    record(
        "event_msg",
        json!({
            "type": "task_started",
            "turn_id": turn_id,
            "started_at": 1,
            "model_context_window": 1000,
            "collaboration_mode_kind": "default",
        }),
    )
}

fn user_message(message: &str) -> Value {
    record(
        "event_msg",
        json!({
            "type": "user_message",
            "message": message,
            "images": [],
            "image_details": [],
            "local_images": [],
            "local_image_details": [],
            "audio": [],
            "local_audio": [],
            "text_elements": [],
        }),
    )
}

fn agent_message(message: &str) -> Value {
    record(
        "event_msg",
        json!({
            "type": "agent_message",
            "message": message,
            "phase": "commentary",
            "memory_citation": null,
        }),
    )
}

fn task_complete(turn_id: &str) -> Value {
    record(
        "event_msg",
        json!({
            "type": "task_complete",
            "turn_id": turn_id,
            "last_agent_message": "done",
            "started_at": 1,
            "completed_at": 2,
            "duration_ms": 1000,
            "time_to_first_token_ms": 1,
        }),
    )
}

fn function_call(turn_id: &str, call_id: &str, name: &str, arguments: &str) -> Value {
    record(
        "response_item",
        json!({
            "type": "function_call",
            "id": format!("fc-{call_id}"),
            "name": name,
            "arguments": arguments,
            "call_id": call_id,
            "internal_chat_message_metadata_passthrough": {"turn_id": turn_id},
        }),
    )
}

fn function_output(turn_id: &str, call_id: &str, output: &str) -> Value {
    record(
        "response_item",
        json!({
            "type": "function_call_output",
            "id": format!("fco-{call_id}"),
            "call_id": call_id,
            "output": output,
            "internal_chat_message_metadata_passthrough": {"turn_id": turn_id},
        }),
    )
}

fn tool_search_call(turn_id: &str, call_id: &str) -> Value {
    record(
        "response_item",
        json!({
            "type": "tool_search_call",
            "id": format!("tsc-{call_id}"),
            "call_id": call_id,
            "status": "completed",
            "execution": "client",
            "arguments": {"queries": [{"query": "calendar"}]},
            "internal_chat_message_metadata_passthrough": {"turn_id": turn_id},
        }),
    )
}

fn tool_search_output(turn_id: &str, call_id: &str, tool_name: &str) -> Value {
    record(
        "response_item",
        json!({
            "type": "tool_search_output",
            "id": format!("tso-{call_id}"),
            "call_id": call_id,
            "status": "completed",
            "execution": "client",
            "tools": [{"name": tool_name, "description": "must not survive"}],
            "internal_chat_message_metadata_passthrough": {"turn_id": turn_id},
        }),
    )
}

#[tokio::test]
async fn legacy_tool_history_restores_calls_without_output_bodies() -> io::Result<()> {
    let temp_dir = TempDir::new()?;
    let path = temp_dir.path().join("rollout.jsonl");
    let thread_id = ThreadId::new();
    let turn_id = "turn-1";
    let output_sentinel = "OUTPUT_BODY_MUST_NOT_SURVIVE".repeat(32_768);
    write_records(
        &path,
        &[
            task_started(turn_id),
            user_message("hello"),
            function_call(
                turn_id,
                "exec-1",
                "exec_command",
                r#"{"cmd":"printf hello","workdir":"/tmp","max_output_tokens":99999}"#,
            ),
            function_output(turn_id, "exec-1", &output_sentinel),
            function_call(
                turn_id,
                "goal-1",
                "get_goal",
                r#"{"unused":"large input is not retained"}"#,
            ),
            function_output(turn_id, "goal-1", "ignored"),
            agent_message("done"),
            task_complete(turn_id),
        ],
    )?;

    let replay = rebuild_legacy_turns_with_tool_calls(&path, thread_id).await?;

    assert_eq!(replay.turns.len(), 1);
    let items = &replay.turns[0].items;
    assert!(matches!(items[0], ThreadItem::UserMessage { .. }));
    let ThreadItem::DynamicToolCall {
        id,
        tool,
        arguments,
        status,
        content_items,
        success,
        ..
    } = &items[1]
    else {
        panic!("expected restored exec tool call");
    };
    assert_eq!(id, "exec-1");
    assert_eq!(tool, "exec_command");
    assert_eq!(
        status,
        &codex_app_server_protocol::DynamicToolCallStatus::Completed
    );
    assert_eq!(arguments["cmd"], "printf hello");
    assert_eq!(arguments["workdir"], "/tmp");
    assert!(arguments.get("max_output_tokens").is_none());
    assert_eq!(content_items, &None);
    assert_eq!(success, &None);

    let ThreadItem::DynamicToolCall {
        id,
        tool,
        arguments,
        ..
    } = &items[2]
    else {
        panic!("expected restored generic tool call");
    };
    assert_eq!(id, "goal-1");
    assert_eq!(tool, "get_goal");
    assert_eq!(arguments, &Value::Null);
    assert!(matches!(items[3], ThreadItem::AgentMessage { .. }));
    assert_eq!(
        replay.inserted_call_ids,
        HashSet::from(["exec-1".to_string(), "goal-1".to_string()])
    );
    let rebuilt_json = serde_json::to_string(&replay.turns).expect("serialize rebuilt turns");
    assert!(!rebuilt_json.contains("OUTPUT_BODY_MUST_NOT_SURVIVE"));
    Ok(())
}

#[tokio::test]
async fn legacy_tool_history_preserves_richer_items_and_restores_other_calls() -> io::Result<()> {
    let temp_dir = TempDir::new()?;
    let path = temp_dir.path().join("rollout.jsonl");
    let thread_id = ThreadId::new();
    let turn_id = "turn-1";
    write_records(
        &path,
        &[
            task_started(turn_id),
            user_message("hello"),
            function_call(turn_id, "patch-1", "apply_patch", "{}"),
            record(
                "event_msg",
                json!({
                    "type": "patch_apply_end",
                    "call_id": "patch-1",
                    "turn_id": turn_id,
                    "stdout": "",
                    "stderr": "",
                    "success": true,
                    "status": "completed",
                    "changes": {},
                }),
            ),
            function_output(turn_id, "patch-1", "done"),
            function_call(turn_id, "plan-1", "update_plan", "{}"),
            function_output(turn_id, "plan-1", "done"),
            function_call(turn_id, "mcp-1", "mcp__server__tool", "{}"),
            function_output(turn_id, "mcp-1", "done"),
            agent_message("done"),
            task_complete(turn_id),
        ],
    )?;

    let replay = rebuild_legacy_turns_with_tool_calls(&path, thread_id).await?;

    assert_eq!(
        replay.inserted_call_ids,
        HashSet::from(["plan-1".to_string(), "mcp-1".to_string()])
    );
    assert!(
        replay.turns[0]
            .items
            .iter()
            .any(|item| matches!(item, ThreadItem::FileChange { id, .. } if id == "patch-1"))
    );
    assert!(
        replay.turns[0]
            .items
            .iter()
            .any(|item| matches!(item, ThreadItem::DynamicToolCall { id, .. } if id == "plan-1"))
    );
    assert!(
        replay.turns[0]
            .items
            .iter()
            .any(|item| matches!(item, ThreadItem::DynamicToolCall { id, .. } if id == "mcp-1"))
    );
    Ok(())
}

#[tokio::test]
async fn legacy_tool_history_restores_tool_search_without_discovered_tools() -> io::Result<()> {
    let temp_dir = TempDir::new()?;
    let path = temp_dir.path().join("rollout.jsonl");
    let thread_id = ThreadId::new();
    let turn_id = "turn-1";
    write_records(
        &path,
        &[
            task_started(turn_id),
            user_message("hello"),
            tool_search_call(turn_id, "search-1"),
            tool_search_output(turn_id, "search-1", "SECRET_DISCOVERED_TOOL"),
            task_complete(turn_id),
        ],
    )?;

    let replay = rebuild_legacy_turns_with_tool_calls(&path, thread_id).await?;

    assert!(matches!(
        &replay.turns[0].items[1],
        ThreadItem::DynamicToolCall {
            id,
            namespace: Some(namespace),
            tool,
            arguments: Value::Null,
            status: codex_app_server_protocol::DynamicToolCallStatus::Completed,
            ..
        } if id == "search-1" && namespace == "client" && tool == "tool_search"
    ));
    let rebuilt_json = serde_json::to_string(&replay.turns).expect("serialize rebuilt turns");
    assert!(!rebuilt_json.contains("SECRET_DISCOVERED_TOOL"));
    Ok(())
}

#[tokio::test]
async fn completed_server_tool_searches_without_call_ids_do_not_remain_unfinished() -> io::Result<()>
{
    let temp_dir = TempDir::new()?;
    let path = temp_dir.path().join("rollout.jsonl");
    let thread_id = ThreadId::new();
    let turn_id = "turn-1";
    write_records(
        &path,
        &[
            task_started(turn_id),
            user_message("hello"),
            record(
                "response_item",
                json!({
                    "type": "tool_search_call",
                    "execution": "server",
                    "call_id": null,
                    "status": "completed",
                    "arguments": {"paths": ["crm"]},
                    "internal_chat_message_metadata_passthrough": {"turn_id": turn_id},
                }),
            ),
            record(
                "response_item",
                json!({
                    "type": "tool_search_output",
                    "execution": "server",
                    "call_id": null,
                    "status": "completed",
                    "tools": [],
                    "internal_chat_message_metadata_passthrough": {"turn_id": turn_id},
                }),
            ),
            record(
                "response_item",
                json!({
                    "type": "tool_search_call",
                    "id": "tsc-1",
                    "execution": "server",
                    "call_id": null,
                    "status": "completed",
                    "arguments": {"paths": ["calendar"]},
                    "internal_chat_message_metadata_passthrough": {"turn_id": turn_id},
                }),
            ),
            task_complete(turn_id),
        ],
    )?;

    let replay = rebuild_legacy_turns_with_tool_calls(&path, thread_id).await?;

    assert!(replay.turns[0].items.iter().any(|item| matches!(
        item,
        ThreadItem::DynamicToolCall {
            id,
            namespace: Some(namespace),
            tool,
            status: codex_app_server_protocol::DynamicToolCallStatus::Completed,
            ..
        } if id == "tsc-1" && namespace == "server" && tool == "tool_search"
    )));
    assert!(!replay.turns[0].items.iter().any(|item| matches!(
        item,
        ThreadItem::DynamicToolCall {
            status: codex_app_server_protocol::DynamicToolCallStatus::InProgress,
            ..
        }
    )));
    Ok(())
}

#[tokio::test]
async fn legacy_tool_history_uses_active_turn_when_call_metadata_is_missing() -> io::Result<()> {
    let temp_dir = TempDir::new()?;
    let path = temp_dir.path().join("rollout.jsonl");
    let thread_id = ThreadId::new();
    let turn_id = "turn-1";
    let call = record(
        "response_item",
        json!({
            "type": "custom_tool_call",
            "id": "ctc-1",
            "call_id": "custom-1",
            "name": "exec",
            "namespace": "functions",
            "input": "must be ignored",
        }),
    );
    let output = record(
        "response_item",
        json!({
            "type": "custom_tool_call_output",
            "id": "ctco-1",
            "call_id": "custom-1",
            "name": "exec",
            "output": {"intentionally": "structurally invalid for the full response model"},
        }),
    );
    write_records(
        &path,
        &[
            task_started(turn_id),
            user_message("hello"),
            call,
            output,
            task_complete(turn_id),
        ],
    )?;

    let replay = rebuild_legacy_turns_with_tool_calls(&path, thread_id).await?;

    assert!(matches!(
        &replay.turns[0].items[1],
        ThreadItem::DynamicToolCall {
            namespace: Some(namespace),
            tool,
            status: codex_app_server_protocol::DynamicToolCallStatus::Completed,
            ..
        } if namespace == "functions" && tool == "exec"
    ));
    Ok(())
}

#[tokio::test]
async fn legacy_tool_history_reports_malformed_record_without_contents() -> io::Result<()> {
    let temp_dir = TempDir::new()?;
    let path = temp_dir.path().join("rollout.jsonl");
    std::fs::write(
        &path,
        "{\"type\":\"world_state\",\"payload\":null}\nSECRET_MALFORMED_BODY\n",
    )?;

    let error = rebuild_legacy_turns_with_tool_calls(&path, ThreadId::new())
        .await
        .expect_err("malformed record should fail");
    let message = error.to_string();
    assert!(message.contains(":2:"));
    assert!(!message.contains("SECRET_MALFORMED_BODY"));
    Ok(())
}

#[tokio::test]
async fn legacy_tool_history_keeps_calls_without_outputs_as_unfinished() -> io::Result<()> {
    let temp_dir = TempDir::new()?;
    let path = temp_dir.path().join("rollout.jsonl");
    let thread_id = ThreadId::new();
    let turn_id = "turn-1";
    write_records(
        &path,
        &[
            task_started(turn_id),
            user_message("hello"),
            function_call(
                turn_id,
                "unfinished-1",
                "exec_command",
                r#"{"cmd":"sleep 1"}"#,
            ),
            agent_message("interrupted"),
            task_complete(turn_id),
        ],
    )?;

    let replay = rebuild_legacy_turns_with_tool_calls(&path, thread_id).await?;

    assert_eq!(
        replay.inserted_call_ids,
        HashSet::from(["unfinished-1".to_string()])
    );
    assert!(replay.turns[0].items.iter().any(|item| matches!(
        item,
        ThreadItem::DynamicToolCall {
            id,
            status: codex_app_server_protocol::DynamicToolCallStatus::InProgress,
            ..
        } if id == "unfinished-1"
    )));
    assert!(
        replay.turns[0]
            .items
            .iter()
            .any(|item| matches!(item, ThreadItem::AgentMessage { .. }))
    );
    Ok(())
}

#[tokio::test]
async fn authoritative_hook_prompts_are_merged_at_their_original_anchor() -> io::Result<()> {
    let temp_dir = TempDir::new()?;
    let path = temp_dir.path().join("rollout.jsonl");
    let thread_id = ThreadId::new();
    let turn_id = "turn-1";
    write_records(
        &path,
        &[
            task_started(turn_id),
            user_message("hello"),
            function_call(turn_id, "goal-1", "get_goal", "{}"),
            function_output(turn_id, "goal-1", "ignored"),
            agent_message("done"),
            task_complete(turn_id),
        ],
    )?;
    let replay = rebuild_legacy_turns_with_tool_calls(&path, thread_id).await?;
    let mut authoritative = baseline_turns(&replay.turns, &replay.inserted_call_ids);
    authoritative[0].items.insert(
        1,
        ThreadItem::HookPrompt {
            id: "hook-1".to_string(),
            fragments: vec![codex_app_server_protocol::HookPromptFragment {
                text: "continue".to_string(),
                hook_run_id: "run-1".to_string(),
            }],
        },
    );
    let mut rebuilt = replay.turns;

    merge_authoritative_hook_prompts(&mut rebuilt, &authoritative);

    assert!(matches!(
        &rebuilt[0].items[1],
        ThreadItem::HookPrompt { id, .. } if id == "hook-1"
    ));
    assert_eq!(
        baseline_turns(&rebuilt, &replay.inserted_call_ids),
        authoritative
    );
    Ok(())
}

#[tokio::test]
async fn legacy_tool_history_skips_incompatible_telemetry_records() -> io::Result<()> {
    let temp_dir = TempDir::new()?;
    let path = temp_dir.path().join("rollout.jsonl");
    let thread_id = ThreadId::new();
    let turn_id = "turn-1";
    write_records(
        &path,
        &[
            task_started(turn_id),
            user_message("hello"),
            record(
                "event_msg",
                json!({
                    "type": "token_count",
                    "info": null,
                    "rate_limits": {
                        "primary": {"legacy_shape": "not current protocol"}
                    }
                }),
            ),
            function_call(turn_id, "goal-1", "get_goal", "{}"),
            function_output(turn_id, "goal-1", "ignored"),
            task_complete(turn_id),
        ],
    )?;

    let replay = rebuild_legacy_turns_with_tool_calls(&path, thread_id).await?;

    assert_eq!(
        replay.inserted_call_ids,
        HashSet::from(["goal-1".to_string()])
    );
    Ok(())
}
