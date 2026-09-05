use codex_navigator::{
    domain::{Session, TurnItem, TurnStatus},
    parser::{jsonl_reader::Record, Parser},
};
use serde_json::{json, Value};
use std::path::Path;

fn parse(input: &str) -> Session {
    let mut parser = Parser::new(36);
    for line in input.lines() {
        parser.consume(Record::Line(line.as_bytes()));
    }
    parser.session
}

fn records(values: &[Value]) -> Session {
    parse(
        &values
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn user(text: &str) -> Value {
    json!({"type":"event_msg","payload":{"type":"user_message","message":text}})
}

fn response_user(text: &str) -> Value {
    json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}})
}

fn event(kind: &str, id: &str) -> Value {
    json!({"type":"event_msg","payload":{"type":kind,"turn_id":id}})
}

fn agents(session: &Session, turn: usize) -> Vec<&str> {
    session.turns[turn]
        .items
        .iter()
        .filter_map(|item| match item {
            TurnItem::AgentMessage { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn parser_local_image_wrappers_do_not_duplicate_prompt_or_leak_attachment_markup() {
    let session = records(&[
        event("task_started", "image-turn"),
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[
            {"type":"input_text","text":"<image name=[Image #1] path=\"/synthetic/attachment.png\">"},
            {"type":"input_image","image_url":"data:image/png;base64,AAAA"},
            {"type":"input_text","text":"</image>"},
            {"type":"input_text","text":"请查看此图"}
        ],"internal_chat_message_metadata_passthrough":{"turn_id":"image-turn","content_item_kinds":["user.text","user.image","user.text","user.text"]}}}),
        json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"image-turn","item":{"type":"UserMessage","content":[{"type":"local_image","path":"/synthetic/attachment.png"},{"type":"text","text":"请查看此图"}]}}}),
    ]);
    assert_eq!(session.turns.len(), 1);
    assert_eq!(session.turns[0].prompt.text, "请查看此图");
    assert_eq!(session.turns[0].prompt.images_count, 1);
    let authored = records(&[response_user(
        "<image name=example> This is authored markup",
    )]);
    assert!(authored.turns[0].prompt.text.contains("authored markup"));
}

#[test]
fn parser_unbounded_identifiers_do_not_enter_turn_identity_index() {
    let id = "x".repeat(1024 * 1024);
    let session = records(&[event("task_started", &id), user("first"), user("second")]);
    assert_eq!(session.turns.len(), 2);
    assert!(session.turns.iter().all(|turn| turn.id.is_none()));
}

#[test]
fn parser_minimal_session_and_metadata() {
    let session = parse(include_str!("fixtures/minimal.jsonl"));
    assert_eq!(session.meta.id, "synthetic-session");
    assert_eq!(
        session.meta.cwd.as_deref(),
        Some(Path::new("/synthetic/project"))
    );
    assert!(session.meta.created_at.is_some());
    assert_eq!(session.turns.len(), 1);
    assert_eq!(session.turns[0].prompt.text, "检查这个示例项目");
    assert_eq!(agents(&session, 0), ["示例项目结构清晰。"]);
    assert_eq!(session.turns[0].status, TurnStatus::Completed);
}

#[test]
fn parser_metadata_first_identity_wins_across_fork_history() {
    let session = records(&[
        json!({"type":"session_meta","payload":{"session_id":"child","cwd":"/child"}}),
        json!({"type":"session_meta","payload":{"id":"parent","cwd":"/parent"}}),
    ]);
    assert_eq!(session.meta.id, "child");
    assert_eq!(session.meta.cwd.as_deref(), Some(Path::new("/child")));
}

#[test]
fn parser_explicit_turn_fragments_and_completion() {
    let session = parse(include_str!("fixtures/explicit-turns.jsonl"));
    assert_eq!(session.turns.len(), 2);
    assert_eq!(session.turns[0].prompt.text, "第一段\n第二段");
    assert_eq!(session.turns[0].id.as_deref(), Some("turn-a"));
    assert_eq!(session.turns[0].status, TurnStatus::Completed);
    assert_eq!(
        (session.turns[0].completed_at.unwrap() - session.turns[0].started_at.unwrap())
            .num_seconds(),
        30
    );
    assert_eq!(session.turns[1].status, TurnStatus::InProgress);
}

#[test]
fn parser_user_and_assistant_mirrors_deduplicate() {
    let session = parse(include_str!("fixtures/duplicate-user-records.jsonl"));
    assert_eq!(session.turns.len(), 1);
    assert_eq!(session.turns[0].prompt.text, "检查测试");
    assert_eq!(agents(&session, 0), ["测试已完成。"]);
}

#[test]
fn parser_mirrors_deduplicate_in_reverse_order_with_whitespace() {
    let session = records(&[response_user("one  two"), user("one\ntwo")]);
    assert_eq!(session.turns.len(), 1);
    assert_eq!(session.turns[0].prompt.text, "one  two");
}

#[test]
fn parser_three_user_mirrors_deduplicate_in_any_order_and_allow_later_repeat() {
    let item = json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","id":"user-a","content":[{"type":"text","text":"repeat"}]}}});
    let mirrors = [user("repeat"), response_user("repeat"), item];
    for order in [[0, 1, 2], [2, 1, 0], [1, 0, 2], [1, 2, 0]] {
        let session = records(&[
            mirrors[order[0]].clone(),
            mirrors[order[1]].clone(),
            mirrors[order[2]].clone(),
            json!({"type":"event_msg","payload":{"type":"task_complete"}}),
            user("repeat"),
        ]);
        assert_eq!(session.turns.len(), 2, "{order:?}");
        assert_eq!(session.turns[0].prompt.text, "repeat");
        assert_eq!(session.turns[1].prompt.text, "repeat");
    }
}

#[test]
fn parser_three_assistant_mirrors_deduplicate() {
    let session = records(&[
        user("request"),
        json!({"type":"event_msg","payload":{"type":"agent_message","message":"done"}}),
        json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}}),
        json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"AgentMessage","id":"assistant-a","content":[{"type":"Text","text":"done"}]}}}),
    ]);
    assert_eq!(agents(&session, 0), ["done"]);
}

#[test]
fn parser_legacy_identical_request_after_assistant_is_a_new_turn() {
    let session = parse(include_str!("fixtures/legacy-no-turn-start.jsonl"));
    assert_eq!(session.turns.len(), 2);
    assert_eq!(agents(&session, 0), ["第一次回复"]);
    assert_eq!(agents(&session, 1), ["第二次回复"]);
}

#[test]
fn parser_completion_is_a_dedup_boundary_even_without_assistant_text() {
    let session = records(&[
        response_user("repeat"),
        json!({"type":"event_msg","payload":{"type":"task_complete"}}),
        user("repeat"),
    ]);
    assert_eq!(session.turns.len(), 2);
}

#[test]
fn parser_local_item_completed_and_metadata_context() {
    let session = parse(include_str!("fixtures/local-item-completed.jsonl"));
    assert_eq!(session.turns.len(), 1);
    let turn = &session.turns[0];
    assert_eq!(turn.prompt.text, "运行示例检查");
    assert_eq!(agents(&session, 0), ["开始检查。"]);
    assert_eq!(turn.activity.commands, 1);
    assert_eq!(turn.activity.files_read, 1);
    assert_eq!(turn.status, TurnStatus::Completed);
    assert!(!format!("{session:?}").contains("private injected setup"));
}

#[test]
fn parser_internal_context_and_reasoning_never_become_visible_text() {
    let session = records(&[
        response_user("<environment_context>private context</environment_context>"),
        user("visible"),
        json!({"type":"response_item","payload":{"type":"reasoning","summary":[{"type":"summary_text","text":"secret reasoning"}]}}),
        json!({"type":"response_item","payload":{"type":"message","role":"assistant","channel":"analysis","content":[{"type":"output_text","text":"private analysis"}]}}),
        json!({"type":"event_msg","payload":{"type":"agent_reasoning","text":"hidden"}}),
        json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"Reasoning","content":[{"type":"text","text":"hidden reasoning"}]}}}),
    ]);
    assert_eq!(session.turns.len(), 1);
    assert_eq!(session.turns[0].prompt.text, "visible");
    assert!(session.turns[0].items.is_empty());
}

#[test]
fn parser_user_metadata_keeps_authored_instruction_like_text() {
    let text = "<environment_context>please explain this example</environment_context>";
    let session = records(&[
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":text}],"internal_chat_message_metadata_passthrough":{"content_item_kinds":["user.text"]}}}),
    ]);
    assert_eq!(session.turns[0].prompt.text, text);
}

#[test]
fn parser_turn_context_groups_fragments_without_start_event() {
    let session = records(&[
        json!({"type":"turn_context","payload":{"turn_id":"a"}}),
        user("first"),
        user("second"),
        json!({"type":"turn_context","payload":{"turn_id":"b"}}),
        user("third"),
    ]);
    assert_eq!(session.turns.len(), 2);
    assert_eq!(session.turns[0].prompt.text, "first\nsecond");
}

#[test]
fn parser_completion_id_targets_the_correct_turn_and_unknown_id_is_ignored() {
    let session = records(&[
        event("turn_started", "a"),
        user("first"),
        event("turn_started", "b"),
        user("second"),
        event("turn_complete", "missing"),
        event("turn_complete", "a"),
    ]);
    assert_eq!(session.turns[0].status, TurnStatus::Completed);
    assert_eq!(session.turns[1].status, TurnStatus::InProgress);
}

#[test]
fn parser_late_item_completion_uses_explicit_turn_id_without_stealing_active_turn() {
    let session = records(&[
        event("turn_started", "a"),
        user("first"),
        event("turn_started", "b"),
        user("second"),
        json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"a","item":{"type":"AgentMessage","id":"late-a","content":[{"type":"Text","text":"first result"}]}}}),
        json!({"type":"event_msg","payload":{"type":"agent_message","message":"second result"}}),
    ]);
    assert_eq!(agents(&session, 0), ["first result"]);
    assert_eq!(agents(&session, 1), ["second result"]);
}

#[test]
fn parser_completion_final_message_is_not_repeated() {
    let session = records(&[
        user("request"),
        json!({"type":"event_msg","payload":{"type":"agent_message","message":"done"}}),
        json!({"type":"event_msg","payload":{"type":"task_complete","last_agent_message":"done"}}),
    ]);
    assert_eq!(agents(&session, 0), ["done"]);
}

#[test]
fn parser_rollback_retains_history_and_changes_active_latest() {
    let session = parse(include_str!("fixtures/rollback.jsonl"));
    assert_eq!(session.turns.len(), 2);
    assert_eq!(session.turns[1].status, TurnStatus::RolledBack);
    assert_eq!(session.latest_active(), Some(0));
}

#[test]
fn parser_rollback_explicit_ids_and_following_prompt() {
    let session = records(&[
        event("turn_started", "a"),
        user("first"),
        event("turn_started", "b"),
        user("second"),
        json!({"type":"event_msg","payload":{"type":"thread_rollback","turn_ids":["b"]}}),
        event("turn_complete", "b"),
        user("replacement"),
    ]);
    assert_eq!(session.turns.len(), 3);
    assert_eq!(session.turns[1].status, TurnStatus::RolledBack);
    assert_eq!(session.latest_active(), Some(2));
}

#[test]
fn parser_ambiguous_rollback_is_unknown_without_guessing() {
    let session = records(&[
        user("first"),
        json!({"type":"event_msg","payload":{"type":"rollback","num_turns":50}}),
    ]);
    assert_eq!(session.turns[0].status, TurnStatus::Unknown);
    assert!(matches!(
        &session.turns[0].items[0],
        TurnItem::Notice { .. }
    ));
}

#[test]
fn parser_unknown_and_malformed_records_do_not_block_later_prompts() {
    let session = parse(include_str!("fixtures/malformed-record.jsonl"));
    assert_eq!(session.parse_stats.malformed_records, 1);
    assert_eq!(session.parse_stats.unknown_records, 1);
    assert_eq!(session.turns.len(), 1);
    let session = parse("null\n[]\n17\n\n");
    assert_eq!(session.parse_stats.malformed_records, 3);
}

#[test]
fn parser_accepts_bom_and_crlf() {
    let session = parse(&format!("\u{feff}{}\r\n", user("中文 👨‍👩‍👧‍👦 café e\u{301}")));
    assert_eq!(session.turns.len(), 1);
    assert!(session.turns[0].prompt.preview.contains("中文"));
    assert!(unicode_width::UnicodeWidthStr::width(session.turns[0].prompt.preview.as_str()) <= 36);
}

#[test]
fn parser_image_only_prompt_and_duplicate_image_count() {
    let session = records(&[
        json!({"type":"event_msg","payload":{"type":"user_message","message":"","images":["synthetic"],"local_images":[]}}),
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_image","image_url":"data:synthetic"}]}}),
    ]);
    assert_eq!(session.turns.len(), 1);
    assert_eq!(session.turns[0].prompt.images_count, 1);
    assert_eq!(session.turns[0].prompt.preview, "[1 image(s)]");
}

#[test]
fn parser_oversized_marker_does_not_hide_following_prompt() {
    let mut parser = Parser::new(36);
    parser.consume(Record::Oversized);
    parser.consume(Record::Line(user("after image").to_string().as_bytes()));
    assert_eq!(parser.session.parse_stats.skipped_oversize_records, 1);
    assert_eq!(parser.session.turns[0].prompt.text, "after image");
}

#[test]
fn parser_tools_normalize_commands_unknown_tools_and_duplicate_ids() {
    let call = json!({"type":"response_item","payload":{"type":"function_call","name":"exec_command","call_id":"command","arguments":"{\"cmd\":\"cargo test\"}"}});
    let session = records(&[
        user("test"),
        call.clone(),
        call,
        json!({"type":"response_item","payload":{"type":"function_call","name":"future_tool","call_id":"future","arguments":{"unknown":true}}}),
    ]);
    assert_eq!(session.turns[0].activity.tool_calls, 2);
    assert_eq!(session.turns[0].activity.commands, 1);
    assert!(
        matches!(&session.turns[0].items[0], TurnItem::ToolCall {summary,..} if summary=="$ cargo test")
    );
    assert!(
        matches!(&session.turns[0].items[1], TurnItem::ToolCall {summary,..} if summary=="future_tool · called")
    );
}

#[test]
fn parser_failed_tool_structured_statuses_and_legacy_exit_envelope() {
    for output in [
        json!({"exit_code":2,"output":"failed"}),
        json!({"metadata":{"exit_code":1},"output":"failed"}),
        json!({"status":"failed","stdout":"failed"}),
        json!({"isError":true,"content":[{"type":"text","text":"failed"}]}),
        json!("{\"exit_code\":1,\"output\":\"failed\"}"),
        json!("Process exited with code 3\nOutput"),
    ] {
        let session = records(&[
            user("test"),
            json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"a","output":output}}),
            json!({"type":"event_msg","payload":{"type":"task_complete"}}),
        ]);
        assert_eq!(session.turns[0].status, TurnStatus::Completed, "{output}");
        assert_eq!(session.turns[0].activity.errors, 1);
    }
}

#[test]
fn parser_error_word_is_not_failure_and_explicit_success_beats_text_heuristic() {
    for output in [
        json!("0 errors; error handling tests completed"),
        json!({"exit_code":0,"stdout":"Process exited with code 1\nquoted example"}),
    ] {
        let session = records(&[
            user("test"),
            json!({"type":"response_item","payload":{"type":"function_call_output","output":output}}),
            json!({"type":"event_msg","payload":{"type":"task_complete"}}),
        ]);
        assert_eq!(session.turns[0].status, TurnStatus::Completed, "{output}");
    }
}

#[test]
fn parser_current_tool_items_and_files_report_structured_failure() {
    let session = records(&[
        user("run tools"),
        json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"FileChange","id":"file","status":"completed","changes":{"src/main.rs":{"type":"update"}}}}}),
        json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"McpToolCall","id":"mcp","tool":"lookup","status":"failed","error":{"message":"unavailable"}}}}),
    ]);
    assert_eq!(session.turns[0].activity.files_changed, 1);
    assert_eq!(session.turns[0].activity.tool_calls, 1);
    assert_eq!(session.turns[0].status, TurnStatus::InProgress);
    assert_eq!(session.turns[0].activity.errors, 1);
}

#[test]
fn parser_unsafe_terminal_controls_are_removed_from_visible_fields() {
    let dangerous = "safe\u{1b}[31m red\u{1b}[0m\u{1b}]52;c;secret\u{7}\u{0}\u{8}";
    let session = records(&[
        user(dangerous),
        json!({"type":"event_msg","payload":{"type":"agent_message","message":dangerous}}),
    ]);
    for text in [
        &session.turns[0].prompt.text,
        &session.turns[0].prompt.preview,
    ] {
        assert!(!text.contains(['\u{1b}', '\u{7}', '\u{0}', '\u{8}']));
        assert!(!text.contains("secret"));
    }
    assert_eq!(agents(&session, 0), ["safe red"]);
}

#[test]
fn parser_long_multibyte_prompt_and_item_are_bounded_without_panics() {
    let session = records(&[
        user(&"界".repeat(100_000)),
        json!({"type":"event_msg","payload":{"type":"agent_message","message":"界".repeat(100_000)}}),
    ]);
    assert!(session.turns[0].prompt.text.len() < 257 * 1024);
    assert!(agents(&session, 0)[0].len() < 65 * 1024);
    assert!(session.parse_stats.omitted_text_bytes > 0);
}

#[test]
fn parser_aggregate_text_retention_is_bounded_across_many_records() {
    let mut parser = Parser::new(36);
    parser.consume(Record::Line(user("request").to_string().as_bytes()));
    let line = json!({"type":"event_msg","payload":{"type":"agent_message","message":"x".repeat(64 * 1024)}}).to_string();
    for _ in 0..1100 {
        parser.consume(Record::Line(line.as_bytes()));
    }
    let retained: usize = agents(&parser.session, 0)
        .iter()
        .map(|text| text.len())
        .sum();
    assert!(retained <= 65 * 1024 * 1024);
    assert!(parser.session.parse_stats.omitted_text_bytes > 0);
}

#[test]
fn parser_item_count_is_bounded_even_when_each_item_is_small() {
    let mut parser = Parser::new(36);
    parser.consume(Record::Line(user("request").to_string().as_bytes()));
    let line = br#"{"type":"event_msg","payload":{"type":"agent_message","message":"visible"}}"#;
    for _ in 0..200_100 {
        parser.consume(Record::Line(line));
    }
    assert!(parser.session.turns[0].items.len() <= 200_000);
    assert!(parser.session.parse_stats.omitted_text_bytes > 0);
}
