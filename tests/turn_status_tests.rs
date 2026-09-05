use codex_navigator::{
    app::App,
    domain::{TurnItem, TurnStatus},
    parser::{jsonl_reader::Record, Parser},
};
use serde_json::{json, Value};

fn consume(parser: &mut Parser, value: Value) {
    parser.consume(Record::Line(value.to_string().as_bytes()));
}

fn started() -> Parser {
    let mut parser = Parser::new(36);
    consume(
        &mut parser,
        json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"a"}}),
    );
    consume(
        &mut parser,
        json!({"type":"event_msg","payload":{"type":"user_message","message":"检查结果"}}),
    );
    parser
}

fn output(id: &str, exit_code: i64) -> Value {
    json!({"type":"response_item","payload":{"type":"function_call_output","call_id":id,
        "output":{"exit_code":exit_code,"output":"synthetic tool outcome"}}})
}

#[test]
fn tool_failure_then_successful_retry_and_completion_keeps_only_activity_warning() {
    let mut parser = started();
    consume(&mut parser, output("failed", 1));
    assert_eq!(parser.session.turns[0].status, TurnStatus::InProgress);
    consume(&mut parser, output("retry", 0));
    consume(
        &mut parser,
        json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"a",
        "last_agent_message":"请根据此回复检查结果"}}),
    );
    let turn = &parser.session.turns[0];
    assert_eq!(turn.status, TurnStatus::Completed);
    assert_eq!(turn.activity.errors, 1);
    assert!(turn
        .items
        .iter()
        .any(|item| matches!(item, TurnItem::AgentMessage {phase,..}
        if phase.as_deref()==Some("final_answer"))));
}

#[test]
fn explicit_execution_errors_and_abort_are_distinct_and_not_activity_errors() {
    for (kind, expected) in [
        ("turn_error", TurnStatus::Failed),
        ("error", TurnStatus::Failed),
        ("turn_aborted", TurnStatus::Interrupted),
    ] {
        for wrapped in [true, false] {
            let mut parser = started();
            let payload =
                json!({"type":kind,"turn_id":"a","last_agent_message":"not a completion"});
            let record = if wrapped {
                json!({"type":"event_msg","timestamp":"2026-09-06T12:00:00Z","payload":payload})
            } else {
                let mut payload = payload;
                payload["timestamp"] = json!("2026-09-06T12:00:00Z");
                payload
            };
            consume(&mut parser, record);
            let turn = &parser.session.turns[0];
            assert_eq!(turn.status, expected, "{kind}, wrapped={wrapped}");
            assert_eq!(turn.activity.errors, 0);
            assert!(turn.completed_at.is_some());
            assert!(
                turn.items.is_empty(),
                "error/abort must not invent a final answer"
            );
        }
    }
}

#[test]
fn error_on_completion_is_execution_failure_independent_of_tool_counts() {
    for outcome in [
        json!({"error":{"message":"execution failed"}}),
        json!({"status":"failed"}),
        json!({"exit_code":1}),
    ] {
        let mut parser = started();
        let mut payload = outcome;
        payload["type"] = json!("task_complete");
        payload["turn_id"] = json!("a");
        consume(&mut parser, json!({"type":"event_msg","payload":payload}));
        assert_eq!(parser.session.turns[0].status, TurnStatus::Failed);
        assert_eq!(parser.session.turns[0].activity.errors, 0);
    }
}

#[test]
fn late_tool_failure_updates_activity_without_overwriting_lifecycle_or_history_selection() {
    for (terminal_event, expected) in [
        ("task_complete", TurnStatus::Completed),
        ("turn_aborted", TurnStatus::Interrupted),
        ("turn_error", TurnStatus::Failed),
    ] {
        let mut parser = started();
        consume(
            &mut parser,
            json!({"type":"event_msg","payload":{"type":terminal_event,"turn_id":"a"}}),
        );
        consume(
            &mut parser,
            json!({"type":"event_msg","payload":{"type":"user_message","message":"new turn"}}),
        );
        let mut app = App::new(true);
        app.open_session(parser.session.clone());
        parser.dirty.clear();
        consume(
            &mut parser,
            json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"a",
            "item":{"type":"CommandExecution","id":"late","command":"synthetic", "exit_code":1}}}),
        );
        assert!(parser.dirty.contains(&0));
        let changed = parser
            .dirty
            .iter()
            .map(|&i| (i, parser.session.turns[i].clone()))
            .collect();
        app.apply_update(
            parser.session.meta.clone(),
            parser.session.parse_stats.clone(),
            parser.session.revision,
            changed,
            false,
        );
        assert_eq!(app.selected, Some(1));
        assert_eq!(app.session.as_ref().unwrap().turns[0].status, expected);
        assert_eq!(app.session.as_ref().unwrap().turns[0].activity.errors, 1);
        assert_eq!(parser.session.turns[1].status, TurnStatus::InProgress);
    }
}

#[test]
fn rollback_survives_late_lifecycle_and_activity_records() {
    let mut parser = started();
    consume(
        &mut parser,
        json!({"type":"event_msg","payload":{"type":"thread_rollback","turn_ids":["a"]}}),
    );
    for kind in ["task_complete", "turn_aborted", "turn_error"] {
        consume(
            &mut parser,
            json!({"type":"event_msg","payload":{"type":kind,"turn_id":"a"}}),
        );
    }
    consume(
        &mut parser,
        json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"a",
        "item":{"type":"CommandExecution","id":"late","command":"synthetic", "exit_code":1}}}),
    );
    assert_eq!(parser.session.turns[0].status, TurnStatus::RolledBack);
    assert_eq!(parser.session.turns[0].activity.errors, 1);
}

#[test]
fn final_text_alone_does_not_claim_completion_or_correctness() {
    let mut parser = started();
    consume(&mut parser, output("failed", 1));
    consume(
        &mut parser,
        json!({"type":"event_msg","payload":{"type":"agent_message",
        "phase":"final_answer","message":"Everything is correct"}}),
    );
    assert_eq!(parser.session.turns[0].status, TurnStatus::InProgress);
    assert_eq!(parser.session.turns[0].activity.errors, 1);
}
