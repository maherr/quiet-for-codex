use super::advancing_cursor;
use super::merge_item_into_loaded_turns;
use super::thread_turns_page_params;
use codex_app_server_protocol::SortDirection;
use codex_app_server_protocol::ThreadItem;
use codex_app_server_protocol::Turn;
use codex_app_server_protocol::TurnItemsView;
use codex_app_server_protocol::TurnStatus;
use codex_protocol::ThreadId;
use pretty_assertions::assert_eq;
use std::collections::HashSet;

#[test]
fn advancing_cursor_rejects_repeated_cursors() {
    let mut seen_cursors = HashSet::new();
    assert_eq!(
        advancing_cursor(
            /*current*/ None,
            Some("first".to_string()),
            &mut seen_cursors,
        ),
        Some("first".to_string())
    );
    assert_eq!(
        advancing_cursor(Some("first"), Some("second".to_string()), &mut seen_cursors,),
        Some("second".to_string())
    );
    assert_eq!(
        advancing_cursor(Some("second"), Some("first".to_string()), &mut seen_cursors,),
        None
    );
    assert_eq!(
        advancing_cursor(Some("second"), /*next*/ None, &mut seen_cursors),
        None
    );
}

#[test]
fn bounded_turn_page_requests_durable_summary_fences() {
    let thread_id = ThreadId::new();
    let params = thread_turns_page_params(thread_id, Some("older".to_string()));

    assert_eq!(params.thread_id, thread_id.to_string());
    assert_eq!(params.cursor.as_deref(), Some("older"));
    assert_eq!(params.limit, Some(super::INITIAL_HISTORY_TURN_LIMIT));
    assert_eq!(params.sort_direction, Some(SortDirection::Desc));
    assert_eq!(params.items_view, Some(TurnItemsView::Summary));
}

#[test]
fn summary_final_reply_is_not_duplicated_when_item_pages_overlap() {
    let user = ThreadItem::UserMessage {
        id: "user-1".to_string(),
        content: vec![codex_app_server_protocol::UserInput::Text {
            text: "request".to_string(),
            text_elements: Vec::new(),
        }],
        client_id: None,
    };
    let final_reply = ThreadItem::AgentMessage {
        id: "final-1".to_string(),
        text: "durable final reply".to_string(),
        phase: None,
        memory_citation: None,
    };
    let mut turns = vec![Turn {
        id: "completed-turn".to_string(),
        items_view: TurnItemsView::Summary,
        items: vec![user, final_reply.clone()],
        status: TurnStatus::Completed,
        error: None,
        started_at: Some(1),
        completed_at: Some(2),
        duration_ms: Some(1),
    }];

    assert_eq!(
        merge_item_into_loaded_turns(&mut turns, "completed-turn", final_reply),
        None
    );
    for id in ["newest-detail", "older-detail"] {
        assert!(
            merge_item_into_loaded_turns(
                &mut turns,
                "completed-turn",
                ThreadItem::Reasoning {
                    id: id.to_string(),
                    summary: Vec::new(),
                    content: Vec::new(),
                },
            )
            .is_some()
        );
    }
    assert_eq!(
        turns[0]
            .items
            .iter()
            .filter(|item| item.id() == "final-1")
            .count(),
        1
    );
    assert_eq!(
        turns[0]
            .items
            .iter()
            .map(ThreadItem::id)
            .collect::<Vec<_>>(),
        vec!["user-1", "older-detail", "newest-detail", "final-1"]
    );
}
