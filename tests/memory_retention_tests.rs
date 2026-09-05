use codex_navigator::{
    app::App,
    config::Config,
    domain::{Session, TurnItem, TurnStatus},
    index::SearchIndex,
    parser::{jsonl_reader::Record, Parser},
    watch::{SessionWorker, Update},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{json, Value};
use std::{
    fs::OpenOptions,
    io::Write,
    time::{Duration, Instant},
};
use tempfile::TempDir;

const MIB: usize = 1024 * 1024;
const PROMPT_LIMIT: usize = 256 * 1024;

fn consume(parser: &mut Parser, value: Value) {
    parser.consume(Record::Line(value.to_string().as_bytes()));
}

fn user(text: &str) -> Value {
    json!({"type":"event_msg","payload":{"type":"user_message","message":text}})
}

fn sized_prompt(n: usize) -> String {
    let prefix = format!("previewtoken{n:03} ");
    let suffix = format!(" bodytoken{n:03}");
    format!(
        "{prefix}{}{suffix}",
        "x".repeat(PROMPT_LIMIT - prefix.len() - suffix.len())
    )
}

fn retained_bodies(session: &Session) -> usize {
    session
        .turns
        .iter()
        .map(|turn| {
            turn.prompt.text.len()
                + turn
                    .items
                    .iter()
                    .map(|item| match item {
                        TurnItem::AgentMessage { text, phase } => {
                            text.len() + phase.as_ref().map_or(0, String::len)
                        }
                        TurnItem::ToolCall { name, summary } => name.len() + summary.len(),
                        TurnItem::ToolOutput { summary, .. } => summary.len(),
                        TurnItem::FileActivity { path, kind } => path.len() + kind.len(),
                        TurnItem::Notice { text } => text.len(),
                        TurnItem::Omitted => 0,
                    })
                    .sum::<usize>()
        })
        .sum()
}

fn deliver(parser: &mut Parser, app: &mut App) {
    let changed = std::mem::take(&mut parser.dirty)
        .into_iter()
        .map(|i| (i, parser.session.turns[i].clone()))
        .collect();
    app.apply_update(
        parser.session.meta.clone(),
        parser.session.parse_stats.clone(),
        parser.session.revision,
        changed,
        false,
    );
}

fn search(app: &mut App, query: &str) {
    app.query = query.to_owned();
    app.refresh_results();
}

#[test]
fn historical_tool_pressure_preserves_latest_multiline_prompt_and_final_answer() {
    let mut parser = Parser::new(36);
    consume(&mut parser, user("historical prompt"));
    consume(
        &mut parser,
        json!({"type":"response_item","payload":{
        "type":"function_call","name":"exec_command","call_id":"command",
        "arguments":{"cmd":"synthetic check"}}}),
    );
    consume(
        &mut parser,
        json!({"type":"response_item","payload":{
        "type":"function_call_output","call_id":"error","output":{"exit_code":1,"text":"failed"}}}),
    );
    // 49 MiB 原始工具正文逐条生成，既超过活动预算，也避免一次构造整个会话。
    let output = "o".repeat(64 * 1024);
    for n in 0..784 {
        consume(
            &mut parser,
            json!({"type":"response_item","payload":{
            "type":"function_call_output","call_id":format!("output-{n}"),"output":output}}),
        );
    }
    let prompt = "请保留最新问题\n第二行包含 searchablelatest 输入与中文标点。";
    consume(&mut parser, user(prompt));
    let answer = "最后结论：所有检查已完成。";
    consume(
        &mut parser,
        json!({"type":"event_msg","payload":{
        "type":"task_complete","last_agent_message":answer}}),
    );

    assert_eq!(parser.session.turns[1].prompt.text, prompt);
    assert_eq!(parser.session.turns[1].prompt.omitted_bytes, 0);
    assert_eq!(parser.session.turns[1].status, TurnStatus::Completed);
    assert!(parser.session.turns[1].items.iter().any(|item| matches!(item,
        TurnItem::AgentMessage { text, phase } if text == answer && phase.as_deref() == Some("final_answer"))));
    assert!(parser.session.turns[0]
        .items
        .iter()
        .any(|item| matches!(item, TurnItem::Omitted)));
    assert_eq!(parser.session.turns[0].activity.tool_calls, 1);
    assert_eq!(parser.session.turns[0].activity.errors, 1);
    assert!(parser.session.parse_stats.omitted_text_bytes > 0);
    assert!(retained_bodies(&parser.session) <= 64 * MIB);
    let prompt_bytes = parser
        .session
        .turns
        .iter()
        .map(|turn| turn.prompt.text.len())
        .sum::<usize>();
    assert!(retained_bodies(&parser.session) - prompt_bytes <= 48 * MIB);
    let mut index = SearchIndex::default();
    index.sync(&parser.session.turns);
    assert_eq!(index.search("searchablelatest"), vec![1]);
    assert_eq!(index.search("第二行"), vec![1]);
}

#[test]
fn prompt_eviction_updates_search_and_preserves_history_selection_and_preview() {
    let mut parser = Parser::new(36);
    let mut first = user(&sized_prompt(0));
    first["payload"]["images"] = json!(["synthetic-image"]);
    consume(&mut parser, first);
    consume(&mut parser, user(&sized_prompt(1)));
    let mut app = App::new(true);
    app.open_session(parser.session.clone());
    parser.dirty.clear();
    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.selected, Some(0));
    search(&mut app, "bodytoken000");
    assert_eq!(app.results, vec![0]);

    // 17.5 MiB Prompt 累计超过独立预算；只通过 dirty 增量交付已淘汰的旧轮。
    for n in 2..70 {
        consume(&mut parser, user(&sized_prompt(n)));
        if n == 69 {
            assert!(parser.dirty.iter().any(|&i| i < 6));
        }
        if n % 8 == 1 {
            deliver(&mut parser, &mut app);
        }
    }
    deliver(&mut parser, &mut app);
    assert_eq!(app.selected, Some(0));
    assert_eq!(app.new_turns, 68);
    assert!(
        app.results.is_empty(),
        "evicted full-body search terms must disappear"
    );
    search(&mut app, "previewtoken000");
    assert_eq!(app.results.first(), Some(&0));
    search(&mut app, "bodytoken069");
    assert_eq!(app.results, vec![69]);
    let session = app.session.as_ref().unwrap();
    assert!(session.turns[0].prompt.text.is_empty());
    assert!(session.turns[0].prompt.omitted_bytes >= PROMPT_LIMIT);
    assert!(session.turns[0].prompt.preview.contains("previewtoken000"));
    assert_eq!(session.turns[0].prompt.images_count, 1);
    assert_eq!(session.turns[69].prompt.text, sized_prompt(69));
    assert_eq!(session.turns[69].prompt.omitted_bytes, 0);
    assert!(session
        .turns
        .iter()
        .all(|turn| !turn.prompt.preview.contains("[0 image(s)]")));
    assert!(session.turns[1..]
        .iter()
        .all(|turn| turn.prompt.images_count == 0));
    assert!(
        session
            .turns
            .iter()
            .map(|t| t.prompt.text.len())
            .sum::<usize>()
            <= 16 * MIB
    );
    assert!(retained_bodies(session) <= 64 * MIB);
}

#[test]
fn oversized_unicode_prompt_remains_valid_and_accounts_for_omitted_bytes() {
    let mut parser = Parser::new(36);
    let prompt = "中文测试🙂".repeat(32 * 1024);
    consume(&mut parser, user(&prompt));
    let retained = &parser.session.turns[0].prompt;
    assert!(!retained.text.is_empty());
    assert!(retained.text.len() <= PROMPT_LIMIT);
    assert!(retained.omitted_bytes > 0);
    assert!(retained.preview.starts_with("中文测试"));
    consume(&mut parser, user("后续完整输入"));
    assert_eq!(parser.session.turns[1].prompt.text, "后续完整输入");
}

#[test]
fn completion_promotes_existing_unphased_answer_without_duplicate() {
    let mut parser = Parser::new(36);
    consume(&mut parser, user("问题"));
    consume(
        &mut parser,
        json!({"type":"event_msg","payload":{
        "type":"agent_message","message":"已完成"}}),
    );
    parser.dirty.clear();
    consume(
        &mut parser,
        json!({"type":"event_msg","payload":{
        "type":"task_complete","last_agent_message":"已完成"}}),
    );
    assert_eq!(parser.session.turns[0].items.len(), 1);
    assert!(
        matches!(&parser.session.turns[0].items[0], TurnItem::AgentMessage {
        text, phase } if text == "已完成" && phase.as_deref() == Some("final_answer"))
    );
    assert!(parser.dirty.contains(&0));
}

#[test]
fn mirrored_final_phase_promotes_unphased_answer_in_either_order() {
    let event = json!({"type":"event_msg","payload":{
        "type":"agent_message","message":"结论"}});
    let response = json!({"type":"response_item","payload":{
        "type":"message","role":"assistant","phase":"final_answer",
        "content":[{"type":"output_text","text":"结论"}]}});
    for mirrors in [[event.clone(), response.clone()], [response, event]] {
        let mut parser = Parser::new(36);
        consume(&mut parser, user("问题"));
        for mirror in mirrors {
            consume(&mut parser, mirror);
        }
        assert_eq!(parser.session.turns[0].items.len(), 1);
        assert!(
            matches!(&parser.session.turns[0].items[0], TurnItem::AgentMessage {
            phase, .. } if phase.as_deref() == Some("final_answer"))
        );
    }
}

#[test]
fn item_id_mirror_promotes_phase_without_duplicating_answer() {
    let mut parser = Parser::new(36);
    consume(&mut parser, user("问题"));
    for phase in [None, Some("final_answer")] {
        consume(
            &mut parser,
            json!({"type":"event_msg","payload":{
            "type":"item_completed","item":{"type":"AgentMessage","id":"same-item",
            "phase":phase,"content":[{"type":"text","text":"完成"}]}}}),
        );
    }
    assert_eq!(parser.session.turns[0].items.len(), 1);
    assert!(
        matches!(&parser.session.turns[0].items[0], TurnItem::AgentMessage {
        phase, .. } if phase.as_deref() == Some("final_answer"))
    );
}

#[test]
fn completion_promotes_truncated_unicode_answer_and_repeated_mirrors_do_not_duplicate() {
    let mut parser = Parser::new(36);
    consume(&mut parser, user("长回复问题"));
    let answer = "完整结论包含中文🙂\n".repeat(16 * 1024);
    consume(
        &mut parser,
        json!({"type":"event_msg","payload":{
        "type":"agent_message","message":answer}}),
    );
    let retained = match &parser.session.turns[0].items[0] {
        TurnItem::AgentMessage { text, phase } => {
            assert_eq!(phase, &None);
            assert!(text.contains("[display text truncated]"));
            assert!(text.len() < answer.len());
            text.clone()
        }
        _ => panic!("expected assistant message"),
    };
    for _ in 0..2 {
        consume(
            &mut parser,
            json!({"type":"event_msg","payload":{
            "type":"task_complete","last_agent_message":answer}}),
        );
    }
    consume(
        &mut parser,
        json!({"type":"response_item","payload":{
        "type":"message","role":"assistant","phase":"final_answer",
        "content":[{"type":"output_text","text":answer}]}}),
    );
    assert_eq!(parser.session.turns[0].items.len(), 1);
    assert!(
        matches!(&parser.session.turns[0].items[0], TurnItem::AgentMessage {
        text, phase } if text == &retained && phase.as_deref() == Some("final_answer"))
    );
}

#[test]
fn final_promotion_at_exact_activity_budget_evicts_other_item_and_preserves_target() {
    let mut parser = Parser::new(36);
    consume(&mut parser, user("budget boundary"));
    let answer = "a".repeat(64 * 1024);
    consume(
        &mut parser,
        json!({"type":"event_msg","payload":{
        "type":"agent_message","message":answer}}),
    );
    let output = "o".repeat(64 * 1024);
    for n in 0..767 {
        consume(
            &mut parser,
            json!({"type":"response_item","payload":{
            "type":"function_call_output","call_id":format!("boundary-{n}"),"output":output}}),
        );
    }
    let prompt_bytes = parser.session.turns[0].prompt.text.len();
    assert_eq!(retained_bodies(&parser.session) - prompt_bytes, 48 * MIB);
    assert_eq!(parser.session.parse_stats.omitted_text_bytes, 0);
    consume(
        &mut parser,
        json!({"type":"event_msg","payload":{
        "type":"task_complete","last_agent_message":answer}}),
    );
    assert!(
        matches!(&parser.session.turns[0].items[0], TurnItem::AgentMessage {
        text, phase } if text == &answer && phase.as_deref() == Some("final_answer"))
    );
    assert!(matches!(
        parser.session.turns[0].items[1],
        TurnItem::Omitted
    ));
    assert_eq!(parser.session.turns[0].items.len(), 768);
    assert!(retained_bodies(&parser.session) - prompt_bytes <= 48 * MIB);
    assert_eq!(parser.session.parse_stats.omitted_text_bytes, 64 * 1024);
}

fn receive_until(worker: &SessionWorker, app: &mut App, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let update = worker
            .updates
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("worker should deliver completed synthetic input within deadline");
        match update {
            Update::Batch {
                meta,
                stats,
                revision,
                turns,
                reset,
                offset,
                total,
            } => {
                app.apply_update(meta, stats, revision, turns, reset);
                if offset == total && app.session.as_ref().is_some_and(|s| s.turns.len() == count) {
                    return;
                }
            }
            Update::Error(message) => panic!("worker error: {message}"),
        }
    }
}

#[test]
fn worker_delivers_prompt_evictions_and_latest_input_without_stealing_history() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("synthetic-pressure.jsonl");
    let mut file = OpenOptions::new()
        .create_new(true)
        .append(true)
        .open(&path)
        .unwrap();
    for n in 0..2 {
        writeln!(file, "{}", user(&sized_prompt(n))).unwrap();
    }
    file.flush().unwrap();
    let worker = SessionWorker::start(
        path.clone(),
        Config {
            watch: false,
            ..Config::default()
        },
    );
    let mut app = App::new(true);
    app.open_session(Session::default());
    receive_until(&worker, &mut app, 2);
    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    for n in 2..66 {
        writeln!(file, "{}", user(&sized_prompt(n))).unwrap();
    }
    let latest = "最后一个中文问题\nworkerlatest 完整保留";
    writeln!(file, "{}", user(latest)).unwrap();
    file.flush().unwrap();
    worker.refresh();
    receive_until(&worker, &mut app, 67);
    assert_eq!(app.selected, Some(0));
    assert_eq!(app.new_turns, 65);
    let session = app.session.as_ref().unwrap();
    assert!(session.turns[0].prompt.text.is_empty());
    assert!(session.turns[0].prompt.omitted_bytes > 0);
    assert_eq!(session.turns[66].prompt.text, latest);
    assert!(retained_bodies(session) <= 64 * MIB);
    search(&mut app, "bodytoken000");
    assert!(app.results.is_empty());
    search(&mut app, "previewtoken000");
    assert_eq!(app.results.first(), Some(&0));
    search(&mut app, "workerlatest");
    assert_eq!(app.results, vec![66]);
}
