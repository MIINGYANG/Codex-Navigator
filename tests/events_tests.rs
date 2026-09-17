use codex_navigator::{
    domain::Session,
    parser::{jsonl_reader::Record, Parser},
};
use serde_json::{json, Value};

fn feed(parser: &mut Parser, value: Value) {
    parser.consume(Record::Line(value.to_string().as_bytes()));
}
fn parse(records: Vec<Value>) -> Session {
    let mut parser = Parser::new(100);
    feed(
        &mut parser,
        json!({"type":"session_meta","payload":{"id":"synthetic","cwd":"/synthetic/project"}}),
    );
    feed(
        &mut parser,
        json!({"type":"event_msg","payload":{"type":"user_message","message":"Implement change"}}),
    );
    for record in records {
        feed(&mut parser, record);
    }
    parser.session
}
fn call(id: &str, cmd: &str) -> Value {
    json!({"type":"response_item","payload":{"type":"function_call","name":"exec_command","call_id":id,"arguments":json!({"cmd":cmd,"workdir":"/synthetic/repo"}).to_string()}})
}
fn out(id: &str, code: i32, output: &str) -> Value {
    json!({"timestamp":"2026-09-17T10:01:00Z","type":"response_item","payload":{"type":"function_call_output","call_id":id,"output":{"exit_code":code,"output":output}}})
}
#[test]
fn commits_require_success_output_and_matching_command_not_prose() {
    let session = parse(vec![
        json!({"type":"event_msg","payload":{"type":"agent_message","message":"Committed [main abc1234] done"}}),
        out("missing", 0, "[main abc1234] fake"),
        call("echo", "echo '[main abc1234] fake'"),
        out("echo", 0, "[main abc1234] fake"),
        call("failed", "git commit -m failed"),
        out("failed", 1, "error: nothing committed"),
        call("ok", "git add . && git commit -m 'Implement feature'"),
        out(
            "ok",
            0,
            "[feature/ux abc1234] Implement feature\n 2 files changed",
        ),
        out("ok", 0, "[feature/ux abc1234] Implement feature"),
    ]);
    assert_eq!(session.events.len(), 1);
    let event = &session.events[0];
    assert_eq!(event.kind, "commit");
    assert_eq!(event.turn_index, Some(0));
    assert_eq!(event.hash.as_deref(), Some("abc1234"));
    assert_eq!(event.repository.as_deref(), Some("/synthetic/repo"));
    assert_eq!(event.branch.as_deref(), Some("feature/ux"));
    assert!(event.version.is_none());
    assert_eq!(session.turns.len(), 1);
}
#[test]
fn successful_commit_is_kept_when_later_push_fails() {
    let session = parse(vec![
        call("a", "git commit -m change && git push"),
        out("a", 1, "[main abc1234] change\nfatal: network unavailable"),
    ]);
    assert_eq!(session.event_summary().commit_count, 1);
}
#[test]
fn root_commit_detached_head_and_explicit_tag_metadata() {
    let session = parse(vec![
        call("a", "git -C /synthetic/other commit -m initial"),
        out("a", 0, "[main (root-commit) 012abcd] initial"),
        call("b", "git -C /synthetic/other tag v1.2.0 012abcd"),
        out("b", 0, ""),
        call("c", "git commit -m detached"),
        out("c", 0, "[detached HEAD abc1234] detached"),
    ]);
    assert_eq!(session.events.len(), 2);
    assert_eq!(
        session.events[0].repository.as_deref(),
        Some("/synthetic/other")
    );
    assert_eq!(session.events[0].version.as_deref(), Some("v1.2.0"));
    assert!(session.events[1].branch.is_none());
}
#[test]
fn async_command_output_remembers_original_turn() {
    let session = parse(vec![
        call("a", "git commit -m async"),
        json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"a","output":{"session_id":123,"output":"running"}}}),
        json!({"type":"event_msg","payload":{"type":"task_complete"}}),
        json!({"type":"event_msg","payload":{"type":"user_message","message":"next question"}}),
        json!({"type":"response_item","payload":{"type":"function_call","name":"write_stdin","call_id":"wait","arguments":{"session_id":123}}}),
        out("wait", 0, "[main abc1234] async"),
    ]);
    assert_eq!(session.events[0].turn_index, Some(0));
}
#[test]
fn nested_exec_literal_call_and_multilayer_output() {
    let script = "const r = await tools.exec_command({\"cmd\":\"git commit -m nested\",\"workdir\":\"/synthetic/nested\"}); text(r);";
    let session = parse(vec![
        json!({"type":"response_item","payload":{"type":"custom_tool_call","name":"functions.exec","call_id":"nested","input":script}}),
        json!({"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"nested","output":{"content":[{"type":"text","text":json!({"exit_code":0,"output":"[main abc1234] nested"}).to_string()}]}}}),
    ]);
    assert_eq!(session.events.len(), 1);
    assert_eq!(
        session.events[0].repository.as_deref(),
        Some("/synthetic/nested")
    );
}
#[test]
fn legacy_output_and_local_completed_command() {
    let session = parse(vec![
        call("a", "git commit -m legacy"),
        json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"a","output":"Chunk ID: x\nProcess exited with code 0\nOutput:\n[main abc1234] legacy"}}),
        json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"CommandExecution","id":"b","command":"git commit -m local","cwd":"/synthetic/local","exit_code":0,"aggregated_output":"[main def5678] local"}}}),
    ]);
    assert_eq!(session.events.len(), 2);
}
#[test]
fn compaction_records_do_not_create_questions_or_expose_context() {
    let session = parse(vec![
        json!({"timestamp":"2026-09-17T11:00:00Z","type":"compacted","payload":{"message":"PRIVATE SUMMARY","replacement_history":[{"role":"user","text":"PRIVATE CONTEXT"}]}}),
        json!({"timestamp":"2026-09-17T11:00:00Z","type":"event_msg","payload":{"type":"context_compacted"}}),
        json!({"timestamp":"2026-09-17T11:01:00Z","type":"compacted","payload":{"trigger":"auto"}}),
        json!({"timestamp":"2026-09-17T11:02:00Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"ContextCompaction","id":"compact-manual","trigger":"manual"}}}),
        json!({"timestamp":"2026-09-17T11:02:00Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"ContextCompaction","id":"compact-manual","trigger":"manual"}}}),
    ]);
    assert_eq!(session.events.len(), 3);
    assert_eq!(session.turns.len(), 1);
    let triggers: Vec<_> = session
        .events
        .iter()
        .map(|e| e.trigger.as_deref())
        .collect();
    assert_eq!(
        triggers,
        vec![Some("unknown"), Some("auto"), Some("manual")]
    );
    assert!(!format!("{:?}", session.events).contains("PRIVATE"));
}
#[test]
fn compaction_trigger_alone_is_not_completed_compaction() {
    let session = parse(vec![
        json!({"type":"response_item","payload":{"type":"compaction_trigger"}}),
    ]);
    assert!(session.events.is_empty());
}
#[test]
fn rolled_back_events_are_excluded_from_projection() {
    let session = parse(vec![
        call("a", "git commit -m change"),
        out("a", 0, "[main abc1234] change"),
        json!({"type":"compacted","payload":{}}),
        json!({"type":"event_msg","payload":{"type":"thread_rollback","num_turns":1}}),
    ]);
    assert_eq!(session.events.len(), 2);
    assert_eq!(session.visible_events().count(), 0);
    assert_eq!(session.event_summary().commit_count, 0);
    assert_eq!(session.event_summary().compaction_count, 0);
}
#[test]
fn tail_replacement_resets_events_and_pending_commands() {
    use std::fs;
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("session.jsonl");
    fs::write(
        &file,
        format!(
            "{}\n{}\n{}\n",
            json!({"type":"event_msg","payload":{"type":"user_message","message":"first"}}),
            call("a", "git commit -m first"),
            out("a", 0, "[main abc1234] first")
        ),
    )
    .unwrap();
    let mut tail = codex_navigator::parser::Tail::open(&file, 1024 * 1024, 100).unwrap();
    tail.poll(1024 * 1024).unwrap();
    assert_eq!(tail.session().events.len(), 1);
    fs::write(
        &file,
        format!(
            "{}\n",
            json!({"type":"event_msg","payload":{"type":"user_message","message":"replacement"}})
        ),
    )
    .unwrap();
    assert!(tail.poll(1024 * 1024).unwrap().1);
    assert!(tail.session().events.is_empty());
}
#[test]
fn retained_event_count_is_bounded() {
    let mut parser = Parser::new(100);
    for i in 0..10_100 {
        feed(
            &mut parser,
            json!({"type":"compacted","payload":{"id":format!("event-{i}")}}),
        );
    }
    assert_eq!(parser.session.events.len(), 10_000);
    assert!(parser.session.parse_stats.omitted_text_bytes > 0);
    feed(
        &mut parser,
        json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"ContextCompaction","id":"event-9999","trigger":"manual"}}}),
    );
    assert_eq!(parser.session.events.len(), 10_000);
    assert_eq!(
        parser.session.events[9999].trigger.as_deref(),
        Some("manual")
    );
}

#[test]
fn ambiguous_shell_programs_and_unexecuted_examples_never_become_commit_evidence() {
    for command in [
        "echo '[main abc1234] fake'",
        "git log --oneline",
        "cat <<'EOF'\ngit commit -m fake\nEOF",
        "if false; then\ngit commit -m fake\nfi",
        "function unused() {\ngit commit -m fake\n}",
        "git commit -m change && git -C /other log",
        "git -C /one commit -m first && git -C /two commit -m second",
        "cd /missing || git commit -m fake",
        "git commit -m fake | cat",
    ] {
        let session = parse(vec![
            call("test", command),
            out("test", 0, "[main abc1234] fake"),
        ]);
        assert!(session.events.is_empty(), "{command}");
    }
}
#[test]
fn shell_wrapper_array_and_context_cwd_are_supported() {
    let session = parse(vec![
        json!({"type":"turn_context","payload":{"cwd":"/synthetic/context"}}),
        json!({"type":"event_msg","payload":{"type":"exec_command_end","command":["bash","-lc","git commit -m wrapped"],"exit_code":0,"aggregated_output":"[main abc1234] wrapped"}}),
    ]);
    assert_eq!(session.events.len(), 1);
    assert_eq!(
        session.events[0].repository.as_deref(),
        Some("/synthetic/context")
    );
}
#[test]
fn explicit_unknown_turn_compaction_has_no_invented_anchor() {
    let session = parse(vec![
        json!({"type":"event_msg","payload":{"type":"context_compacted","turn_id":"missing"}}),
    ]);
    assert!(session.events[0].turn_index.is_none());
}
#[test]
fn unsuccessful_or_unbound_tags_do_not_infer_version_from_head() {
    let session = parse(vec![
        call("a", "git commit -m change"),
        out("a", 0, "[main abc1234] change"),
        call("b", "git tag v1.0.0"),
        out("b", 0, ""),
        call("c", "git tag v1.1.0 abc1234"),
        out("c", 1, "failed"),
        call("d", "git tag v1.2.0 def5678"),
        out("d", 0, ""),
    ]);
    assert_eq!(session.events.len(), 1);
    assert!(session.events[0].version.is_none());
}
#[test]
fn mixed_nested_batch_does_not_guess_output_to_repository_mapping() {
    let script="await Promise.all([tools.exec_command({\"cmd\":\"git commit -m a\",\"workdir\":\"/one\"}),tools.exec_command({\"cmd\":\"git commit -m b\",\"workdir\":\"/two\"})]);";
    let session = parse(vec![
        json!({"type":"response_item","payload":{"type":"custom_tool_call","name":"functions.exec","call_id":"n","input":script}}),
        out("n", 0, "[main abc1234] mixed"),
    ]);
    assert!(session.events.is_empty());
}

#[test]
fn javascript_examples_comments_and_unexecuted_branches_are_not_commands() {
    for script in [
        "// tools.exec_command({\"cmd\":\"git commit -m fake\"})\ntext('fake');",
        "text('await tools.exec_command({\"cmd\":\"git commit -m fake\"})');",
        "if(false) { await tools.exec_command({\"cmd\":\"git commit -m fake\"}); }",
        "async function unused() { await tools.exec_command({\"cmd\":\"git commit -m fake\"}); }",
        "false && await tools.exec_command({\"cmd\":\"git commit -m fake\"});",
    ] {
        let session = parse(vec![
            json!({"type":"response_item","payload":{"type":"custom_tool_call","name":"functions.exec","call_id":"n","input":script}}),
            out("n", 0, "[main abc1234] fake"),
        ]);
        assert!(session.events.is_empty(), "{script}");
    }
}

#[test]
fn mirrored_compaction_enriches_explicit_trigger_in_either_order() {
    for reverse in [false, true] {
        let mut records = vec![
            json!({"timestamp":"2026-09-17T11:00:00Z","type":"event_msg","payload":{"type":"context_compacted"}}),
            json!({"timestamp":"2026-09-17T11:00:00Z","type":"compacted","payload":{"trigger":"manual"}}),
        ];
        if reverse {
            records.reverse();
        }
        let session = parse(records);
        assert_eq!(session.events.len(), 1);
        assert_eq!(session.events[0].trigger.as_deref(), Some("manual"));
    }
}
#[test]
fn explicit_distinct_compaction_ids_are_not_coalesced() {
    let session = parse(vec![
        json!({"timestamp":"2026-09-17T11:00:00Z","type":"compacted","payload":{"id":"one"}}),
        json!({"timestamp":"2026-09-17T11:00:00Z","type":"event_msg","payload":{"type":"context_compacted","id":"two"}}),
    ]);
    assert_eq!(session.events.len(), 2);
}
#[test]
fn compound_tag_success_cannot_mask_tag_failure() {
    let session = parse(vec![
        call("a", "git commit -m change"),
        out("a", 0, "[main abc1234] change"),
        call("b", "git tag v1.0.0 abc1234; git status"),
        out("b", 0, "fatal: tag already exists\nOn branch main"),
    ]);
    assert!(session.events[0].version.is_none());
}

fn identified_parser() -> Parser {
    let mut parser = Parser::new(100);
    feed(
        &mut parser,
        json!({"type":"event_msg","payload":{"type":"user_message","turn_id":"synthetic-turn","message":"Synthetic question"}}),
    );
    parser
}

fn compact_record(source: &str, timestamp: &str, id: Option<&str>, trigger: &str) -> Value {
    let mut item = json!({"trigger":trigger});
    if let Some(id) = id {
        item["id"] = json!(id);
    }
    match source {
        "compacted" => json!({"timestamp":timestamp,"type":"compacted","payload":item}),
        "item_completed" => {
            item["type"] = json!("ContextCompaction");
            json!({"timestamp":timestamp,"type":"event_msg","payload":{"type":"item_completed","item":item}})
        }
        _ => {
            item["type"] = json!(source);
            json!({"timestamp":timestamp,"type":"event_msg","payload":item})
        }
    }
}

#[test]
fn persisted_compaction_and_completed_item_merge_across_metadata_and_19ms() {
    let mut parser = identified_parser();
    feed(
        &mut parser,
        compact_record("compacted", "2026-09-17T16:27:36.661Z", None, "unknown"),
    );
    let stable_id = parser.session.events[0].id.clone();
    for record in [
        json!({"timestamp":"2026-09-17T16:27:36.665Z","type":"world_state","payload":{}}),
        json!({"timestamp":"2026-09-17T16:27:36.665Z","type":"turn_context","payload":{"turn_id":"synthetic-turn"}}),
        json!({"timestamp":"2026-09-17T16:27:36.666Z","type":"event_msg","payload":{"type":"thread_settings_applied"}}),
        json!({"timestamp":"2026-09-17T16:27:36.678Z","type":"event_msg","payload":{"type":"token_count"}}),
        json!({"type":"token_usage_record","payload":{}}),
        json!({"timestamp":"2026-09-17T16:27:36.680Z","type":"event_msg","payload":{"type":"item_completed","turn_id":"synthetic-turn","item":{"type":"ContextCompaction","id":"synthetic-c1"}}}),
    ] {
        feed(&mut parser, record);
    }
    assert_eq!(parser.session.events.len(), 1);
    assert_eq!(parser.session.events[0].id, stable_id);
    assert_eq!(parser.session.event_summary().compaction_count, 1);
    assert_eq!(parser.session.turns.len(), 1);
    assert_eq!(parser.session.events[0].turn_index, Some(0));
    // The completed-item alias remains usable after the original pair was merged.
    feed(
        &mut parser,
        json!({"type":"event_msg","payload":{"type":"agent_message","message":"later"}}),
    );
    feed(
        &mut parser,
        compact_record(
            "item_completed",
            "2026-09-17T16:28:00Z",
            Some("synthetic-c1"),
            "manual",
        ),
    );
    assert_eq!(parser.session.events.len(), 1);
    assert_eq!(parser.session.events[0].trigger.as_deref(), Some("manual"));
    assert_eq!(parser.session.events[0].id, stable_id);
}

#[test]
fn compaction_complementary_sources_merge_in_both_orders_without_sliding_window() {
    for sources in [
        ["compacted", "context_compacted", "item_completed"],
        ["item_completed", "context_compacted", "compacted"],
    ] {
        let session = parse(
            sources
                .into_iter()
                .map(|source| compact_record(source, "2026-09-17T11:00:00Z", None, "unknown"))
                .collect(),
        );
        assert_eq!(session.events.len(), 1);
    }
    let session = parse(vec![
        compact_record("compacted", "2026-09-17T11:00:00Z", None, "unknown"),
        compact_record(
            "context_compacted",
            "2026-09-17T11:00:00.900Z",
            None,
            "unknown",
        ),
        compact_record(
            "item_completed",
            "2026-09-17T11:00:01.001Z",
            None,
            "unknown",
        ),
    ]);
    assert_eq!(session.events.len(), 2);
    let equivalent_timezones = parse(vec![
        compact_record("compacted", "2026-09-17T11:00:00Z", None, "unknown"),
        compact_record(
            "item_completed",
            "2026-09-17T19:00:00+08:00",
            None,
            "unknown",
        ),
    ]);
    assert_eq!(equivalent_timezones.events.len(), 1);
}

#[test]
fn compaction_repeated_source_and_distinct_ids_preserve_separate_operations() {
    let first = compact_record("compacted", "2026-09-17T11:00:00Z", None, "unknown");
    let second = compact_record(
        "context_compacted",
        "2026-09-17T11:00:00Z",
        Some("first"),
        "unknown",
    );
    for records in [
        vec![first.clone(), first.clone()],
        vec![first.clone(), second.clone(), first.clone()],
        vec![
            first.clone(),
            second.clone(),
            compact_record(
                "item_completed",
                "2026-09-17T11:00:00Z",
                Some("second"),
                "unknown",
            ),
        ],
        vec![
            second,
            compact_record("ContextCompacted", "2026-09-17T11:00:00Z", None, "unknown"),
        ],
        vec![
            compact_record("compacted", "2026-09-17T11:00:00Z", None, "auto"),
            compact_record("item_completed", "2026-09-17T11:00:00Z", None, "manual"),
        ],
    ] {
        assert_eq!(parse(records).events.len(), 2);
    }
}

#[test]
fn compaction_mirrors_require_known_turn_and_valid_forward_time() {
    for timestamp in [
        "",
        "invalid",
        "2026-09-17T10:59:59.999Z",
        "2026-09-17T11:00:01.001Z",
    ] {
        let session = parse(vec![
            compact_record("compacted", "2026-09-17T11:00:00Z", None, "unknown"),
            compact_record("item_completed", timestamp, None, "unknown"),
        ]);
        assert_eq!(session.events.len(), 2, "{timestamp}");
    }
    let mut parser = Parser::new(100);
    for source in ["compacted", "item_completed"] {
        feed(
            &mut parser,
            compact_record(source, "2026-09-17T11:00:00Z", None, "unknown"),
        );
    }
    assert_eq!(parser.session.events.len(), 2);
    let mut parser = identified_parser();
    feed(
        &mut parser,
        compact_record("compacted", "2026-09-17T11:00:00Z", None, "unknown"),
    );
    let mut second = compact_record("item_completed", "2026-09-17T11:00:00Z", None, "unknown");
    second["payload"]["turn_id"] = json!("unrecognized-turn");
    feed(&mut parser, second);
    assert_eq!(parser.session.events.len(), 2);
    assert!(parser.session.events[1].turn_index.is_none());
}

#[test]
fn compaction_mirror_candidates_end_on_activity_or_other_turn_metadata() {
    for boundary in [
        json!({"type":"event_msg","payload":{"type":"user_message","message":"next question"}}),
        json!({"type":"event_msg","payload":{"type":"agent_message","message":"continued work"}}),
        call("boundary", "echo boundary"),
        out("boundary", 0, "boundary"),
        json!({"type":"event_msg","payload":{"type":"task_complete"}}),
        json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"synthetic-turn"}}),
        json!({"type":"event_msg","payload":{"type":"token_count","turn_id":"another-turn"}}),
        json!({"type":"turn_context","payload":{"turn_id":"another-turn"}}),
        json!({"type":"unknown_record","payload":{}}),
    ] {
        let mut parser = identified_parser();
        feed(
            &mut parser,
            compact_record("compacted", "2026-09-17T11:00:00Z", None, "unknown"),
        );
        feed(&mut parser, boundary.clone());
        feed(
            &mut parser,
            compact_record(
                "item_completed",
                "2026-09-17T11:00:00.019Z",
                None,
                "unknown",
            ),
        );
        assert_eq!(parser.session.events.len(), 2, "{boundary}");
    }
}

#[test]
fn compaction_mirrors_do_not_cross_unobserved_records() {
    for boundary in [
        Record::Line(b"invalid json"),
        Record::Oversized,
        Record::Line(b" "),
    ] {
        let mut parser = identified_parser();
        feed(
            &mut parser,
            compact_record("compacted", "2026-09-17T11:00:00Z", None, "unknown"),
        );
        parser.consume(boundary);
        feed(
            &mut parser,
            compact_record("item_completed", "2026-09-17T11:00:00Z", None, "unknown"),
        );
        assert_eq!(parser.session.events.len(), 2);
    }
}

#[test]
fn explicit_compaction_identity_enriches_unknown_trigger_without_replacing_known_trigger() {
    let session = parse(vec![
        compact_record("item_completed", "", Some("known"), "unknown"),
        compact_record("item_completed", "", Some("known"), "manual"),
        compact_record("item_completed", "", Some("known"), "auto"),
    ]);
    assert_eq!(session.events.len(), 1);
    assert_eq!(session.events[0].trigger.as_deref(), Some("manual"));
}
