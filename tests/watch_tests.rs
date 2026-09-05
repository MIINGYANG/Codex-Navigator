use codex_navigator::{
    app::App,
    config::Config,
    domain::Session,
    watch::{SessionWorker, Update},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    sync::mpsc::RecvTimeoutError,
    time::{Duration, Instant},
};
use tempfile::TempDir;

const UPDATE_DEADLINE: Duration = Duration::from_secs(2);

fn prompt(text: &str) -> String {
    format!(
        "{}\n",
        json!({"type":"event_msg","payload":{"type":"user_message","message":text}})
    )
}

fn append(path: &Path, bytes: &[u8]) {
    OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(bytes)
        .unwrap();
}

fn apply(app: &mut App, update: Update) -> bool {
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
            assert!(offset <= total);
            app.apply_update(meta, stats, revision, turns, reset);
            app.loading = offset < total;
            app.progress = Some((offset, total));
            reset
        }
        Update::Error(message) => panic!("worker unexpectedly failed: {message}"),
    }
}

fn wait_until(worker: &SessionWorker, app: &mut App, ready: impl Fn(&App) -> bool) -> bool {
    let deadline = Instant::now() + UPDATE_DEADLINE;
    let mut saw_reset = false;
    loop {
        if ready(app) {
            return saw_reset;
        }
        let timeout = deadline.saturating_duration_since(Instant::now());
        let update = worker.updates.recv_timeout(timeout).unwrap_or_else(|error| {
            panic!(
                "worker failed to reach expected state within {UPDATE_DEADLINE:?}: {error}; turns={}, selected={:?}, progress={:?}",
                turn_count(app), app.selected, app.progress
            )
        });
        saw_reset |= apply(app, update);
    }
}

fn turn_count(app: &App) -> usize {
    app.session
        .as_ref()
        .map_or(0, |session| session.turns.len())
}

fn start(path: &Path, config: Config) -> (SessionWorker, App) {
    let mut app = App::new(config.watch);
    app.open_session(Session::default());
    app.loading = true;
    let worker = SessionWorker::start(path.to_path_buf(), config);
    wait_until(&worker, &mut app, |app| !app.loading);
    (worker, app)
}

#[test]
fn watch_automatic_append_follows_latest_but_never_steals_history() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("live.jsonl");
    fs::write(&path, prompt("first")).unwrap();
    let (worker, mut app) = start(&path, Config::default());
    assert_eq!(app.selected, Some(0));

    append(&path, prompt("second").as_bytes());
    wait_until(&worker, &mut app, |app| turn_count(app) == 2);
    assert_eq!(app.selected, Some(1));
    assert_eq!(app.new_turns, 0);

    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    append(&path, prompt("中文第三轮").as_bytes());
    wait_until(&worker, &mut app, |app| turn_count(app) == 3);
    assert_eq!(app.selected, Some(0));
    assert_eq!(app.new_turns, 1);
    assert_eq!(
        app.session.as_ref().unwrap().turns[2].prompt.text,
        "中文第三轮"
    );
    app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE));
    append(&path, prompt("fourth").as_bytes());
    wait_until(&worker, &mut app, |app| turn_count(app) == 4);
    assert_eq!(app.selected, Some(3));
    assert_eq!(app.new_turns, 0);
    assert_eq!(app.session.as_ref().unwrap().parse_stats.records, 4);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        ["first", "second", "中文第三轮", "fourth"]
            .into_iter()
            .map(prompt)
            .collect::<String>()
    );
}

#[test]
fn watch_partial_utf8_record_waits_for_completion_without_malformed_warning() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("partial.jsonl");
    fs::write(&path, prompt("first")).unwrap();
    let (worker, mut app) = start(&path, Config::default());
    let record = prompt("中文 prompt");
    let split = record.find('中').unwrap() + 1;
    append(&path, &record.as_bytes()[..split]);

    // 部分行可能产生纯进度通知，但不能创建 Turn 或报坏行。
    let deadline = Instant::now() + Duration::from_millis(250);
    loop {
        match worker
            .updates
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(update) => {
                apply(&mut app, update);
                assert_eq!(turn_count(&app), 1);
                assert_eq!(
                    app.session.as_ref().unwrap().parse_stats.malformed_records,
                    0
                );
            }
            Err(RecvTimeoutError::Timeout) => break,
            Err(RecvTimeoutError::Disconnected) => {
                panic!("worker stopped while waiting for partial line")
            }
        }
    }
    append(&path, &record.as_bytes()[split..]);
    wait_until(&worker, &mut app, |app| turn_count(app) == 2);
    assert_eq!(
        app.session.as_ref().unwrap().turns[1].prompt.text,
        "中文 prompt"
    );
    assert_eq!(
        app.session.as_ref().unwrap().parse_stats.malformed_records,
        0
    );
}

#[test]
fn watch_disabled_requires_manual_refresh_after_initial_load() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("static.jsonl");
    fs::write(&path, prompt("first")).unwrap();
    let (worker, mut app) = start(
        &path,
        Config {
            watch: false,
            ..Config::default()
        },
    );
    append(&path, prompt("second").as_bytes());
    assert!(matches!(
        worker.updates.recv_timeout(Duration::from_millis(350)),
        Err(RecvTimeoutError::Timeout)
    ));
    assert_eq!(turn_count(&app), 1);
    worker.refresh();
    wait_until(&worker, &mut app, |app| turn_count(app) == 2);
    assert_eq!(app.selected, Some(1));
    assert_eq!(app.session.as_ref().unwrap().parse_stats.records, 2);
}

#[test]
fn watch_file_replacement_and_truncation_rebuild_model_instead_of_replaying() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("replace.jsonl");
    fs::write(&path, prompt("original")).unwrap();
    let (worker, mut app) = start(&path, Config::default());
    fs::copy(&path, temp.path().join("retained-original.jsonl")).unwrap();
    let replacement = temp.path().join("replacement.jsonl");
    fs::write(
        &replacement,
        format!("{}{}", prompt("replacement one"), prompt("replacement two")),
    )
    .unwrap();
    fs::rename(&replacement, &path).unwrap();
    let reset = wait_until(&worker, &mut app, |app| {
        app.session.as_ref().is_some_and(|s| {
            s.turns
                .first()
                .is_some_and(|t| t.prompt.text == "replacement one")
        })
    });
    assert!(reset);
    assert_eq!(turn_count(&app), 2);
    assert_eq!(app.selected, Some(1));

    fs::write(&path, prompt("short")).unwrap();
    let reset = wait_until(&worker, &mut app, |app| {
        app.session
            .as_ref()
            .is_some_and(|s| s.turns.first().is_some_and(|t| t.prompt.text == "short"))
    });
    assert!(reset);
    assert_eq!(turn_count(&app), 1);
    assert_eq!(app.selected, Some(0));
    assert_eq!(app.session.as_ref().unwrap().parse_stats.records, 1);
}

#[test]
fn watch_oversized_unterminated_tail_finishes_loading_and_resumes_on_newline() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("oversized.jsonl");
    let mut file = fs::File::create(&path).unwrap();
    file.write_all(prompt("before image").as_bytes()).unwrap();
    file.write_all(b"{\"image\":\"").unwrap();
    let chunk = [b'A'; 64 * 1024];
    for _ in 0..160 {
        file.write_all(&chunk).unwrap();
    }
    drop(file);
    let total = fs::metadata(&path).unwrap().len();
    let (worker, mut app) = start(
        &path,
        Config {
            max_record_bytes: 1024,
            ..Config::default()
        },
    );
    assert_eq!(app.progress, Some((total, total)));
    assert_eq!(turn_count(&app), 1);
    assert_eq!(app.selected, Some(0));
    assert_eq!(
        app.session
            .as_ref()
            .unwrap()
            .parse_stats
            .skipped_oversize_records,
        0
    );
    assert_eq!(
        app.session.as_ref().unwrap().parse_stats.malformed_records,
        0
    );

    append(&path, b"\"}\n");
    append(&path, prompt("after image").as_bytes());
    wait_until(&worker, &mut app, |app| turn_count(app) == 2);
    assert_eq!(
        app.session
            .as_ref()
            .unwrap()
            .parse_stats
            .skipped_oversize_records,
        1
    );
    assert_eq!(
        app.session.as_ref().unwrap().parse_stats.malformed_records,
        0
    );
    assert_eq!(app.selected, Some(1));
}

#[test]
fn watch_initial_multibatch_load_follows_last_turn_without_new_turn_badge() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("multibatch.jsonl");
    let mut file = fs::File::create(&path).unwrap();
    file.write_all(prompt("first historical prompt").as_bytes())
        .unwrap();
    let chunk = [b'A'; 64 * 1024];
    for _ in 0..80 {
        file.write_all(&chunk).unwrap();
    }
    file.write_all(b"\n").unwrap();
    file.write_all(prompt("latest historical prompt").as_bytes())
        .unwrap();
    drop(file);

    let (_worker, app) = start(
        &path,
        Config {
            max_record_bytes: 1024,
            ..Config::default()
        },
    );
    assert_eq!(turn_count(&app), 2);
    assert_eq!(app.selected, Some(1));
    assert_eq!(app.new_turns, 0);
    assert!(!app.loading);
    assert_eq!(
        app.session
            .as_ref()
            .unwrap()
            .parse_stats
            .skipped_oversize_records,
        1
    );
}

#[test]
fn watch_worker_reports_missing_source_and_shuts_down_without_blocking() {
    let temp = TempDir::new().unwrap();
    let worker = SessionWorker::start(temp.path().join("missing.jsonl"), Config::default());
    let update = worker.updates.recv_timeout(UPDATE_DEADLINE).unwrap();
    let Update::Error(message) = update else {
        panic!("missing source should report an error");
    };
    assert!(message.contains("Cannot open session"));
    let start = Instant::now();
    drop(worker);
    assert!(start.elapsed() < Duration::from_secs(1));
}
