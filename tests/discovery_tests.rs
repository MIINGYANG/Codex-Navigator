use chrono::{Duration, Local};
use clap::Parser;
use codex_navigator::{
    cli::{Cli, Command},
    config::Config,
    discovery::{auto_select, codex_home_from, discover, resolve_session},
};
use serde_json::json;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn fixture(home: &Path, days_ago: i64, id: &str, cwd: &str) -> PathBuf {
    let day = Local::now().date_naive() - Duration::days(days_ago);
    let directory = home
        .join("sessions")
        .join(day.format("%Y/%m/%d").to_string());
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!("rollout-{id}.jsonl"));
    let meta =
        json!({"type":"session_meta", "payload":{"id":id,"cwd":cwd,"unknown_future_field":true}});
    let prompt = json!({"type":"event_msg","payload":{"type":"user_message","message":"修复 authentication"}});
    fs::write(&path, format!("{meta}\n{prompt}\n")).unwrap();
    path
}

#[test]
fn discovery_exact_cwd_has_priority_over_related_and_unrelated() {
    let temp = TempDir::new().unwrap();
    fixture(temp.path(), 0, "unrelated", "/other");
    fixture(temp.path(), 0, "related", "/project/subdir");
    fixture(temp.path(), 2, "exact", "/project");
    let sessions = discover(
        temp.path(),
        Path::new("/project"),
        false,
        &Config::default(),
    )
    .unwrap();
    assert_eq!(
        sessions.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        ["exact", "related", "unrelated"]
    );
    assert_eq!(auto_select(&sessions, Path::new("/project")), Some(0));
    assert_eq!(
        sessions[0].first_prompt.as_deref(),
        Some("修复 authentication")
    );
    assert_eq!(sessions[0].turn_count, Some(1));
}

#[test]
fn discovery_multiple_exact_sessions_require_picker() {
    let temp = TempDir::new().unwrap();
    fixture(temp.path(), 0, "first", "/project");
    fixture(temp.path(), 1, "second", "/project");
    let sessions = discover(
        temp.path(),
        Path::new("/project"),
        false,
        &Config::default(),
    )
    .unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(auto_select(&sessions, Path::new("/project")), None);
}

#[test]
fn discovery_missing_index_and_unmatched_cwd_show_recent_picker() {
    let temp = TempDir::new().unwrap();
    fixture(temp.path(), 0, "first", "/project");
    let sessions = discover(
        temp.path(),
        Path::new("/unrelated"),
        false,
        &Config::default(),
    )
    .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(auto_select(&sessions, Path::new("/unrelated")), None);
}

#[test]
fn discovery_stale_index_supplements_existing_rollouts_only() {
    let temp = TempDir::new().unwrap();
    fixture(temp.path(), 0, "first", "/project");
    let index = [
        json!({"id":"missing-rollout", "thread_name":"Deleted session"}),
        json!({"id":"first", "thread_name":"Old title", "updated_at":"2000-01-01T00:00:00Z"}),
        json!({"id":"first", "thread_name":"New title", "updated_at":"2000-01-01T00:00:00Z"}),
    ]
    .iter()
    .map(|value| format!("{value}\n"))
    .collect::<String>();
    fs::write(temp.path().join("session_index.jsonl"), index).unwrap();
    let sessions = discover(
        temp.path(),
        Path::new("/project"),
        false,
        &Config::default(),
    )
    .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].title.as_deref(), Some("New title"));
    assert!(sessions[0].updated_at.unwrap().timestamp() > 1_000_000_000);
}

#[test]
fn discovery_all_includes_old_date_directories() {
    let temp = TempDir::new().unwrap();
    fixture(temp.path(), 100, "old", "/project");
    fixture(temp.path(), 0, "today", "/project");
    assert_eq!(
        discover(
            temp.path(),
            Path::new("/project"),
            false,
            &Config::default()
        )
        .unwrap()
        .len(),
        1
    );
    assert_eq!(
        discover(temp.path(), Path::new("/project"), true, &Config::default())
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn discovery_inactive_installation_falls_back_to_newest_existing_dates() {
    let temp = TempDir::new().unwrap();
    fixture(temp.path(), 100, "newest-old", "/project");
    fixture(temp.path(), 101, "older", "/project");
    fixture(temp.path(), 102, "oldest", "/project");
    let config = Config {
        recent_days: 2,
        ..Config::default()
    };
    let sessions = discover(temp.path(), Path::new("/unrelated"), false, &config).unwrap();
    assert_eq!(sessions.len(), 2);
    assert!(sessions.iter().any(|session| session.id == "newest-old"));
    assert!(sessions.iter().any(|session| session.id == "older"));
    assert!(!sessions.iter().any(|session| session.id == "oldest"));
}

#[test]
fn discovery_fallback_ignores_non_date_directories_and_skips_empty_days() {
    let temp = TempDir::new().unwrap();
    fixture(temp.path(), 100, "old", "/project");
    let empty_day = Local::now().date_naive() - Duration::days(99);
    fs::create_dir_all(
        temp.path()
            .join("sessions")
            .join(empty_day.format("%Y/%m/%d").to_string()),
    )
    .unwrap();
    fs::create_dir_all(temp.path().join("sessions/random/99/99")).unwrap();
    fs::write(
        temp.path()
            .join("sessions/random/99/99/rollout-invalid.jsonl"),
        "{}\n",
    )
    .unwrap();
    let sessions = discover(
        temp.path(),
        Path::new("/project"),
        false,
        &Config::default(),
    )
    .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "old");
}

#[cfg(unix)]
#[test]
fn discovery_ignores_fifo_index_without_waiting_for_a_writer() {
    let temp = TempDir::new().unwrap();
    fixture(temp.path(), 0, "safe", "/project");
    let index = temp.path().join("session_index.jsonl");
    assert!(std::process::Command::new("mkfifo")
        .arg(&index)
        .status()
        .unwrap()
        .success());
    let sessions = discover(
        temp.path(),
        Path::new("/project"),
        false,
        &Config::default(),
    )
    .unwrap();
    assert_eq!(sessions.len(), 1);
    assert!(resolve_session(temp.path(), index.to_str().unwrap(), &Config::default()).is_err());
    assert!(Config::load_from(&index).is_err());
}

#[test]
fn discovery_respects_recent_days() {
    let temp = TempDir::new().unwrap();
    fixture(temp.path(), 0, "today", "/project");
    fixture(temp.path(), 2, "earlier", "/project");
    let config = Config {
        recent_days: 1,
        ..Config::default()
    };
    let sessions = discover(temp.path(), Path::new("/project"), false, &config).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "today");
}

#[test]
fn discovery_missing_sessions_is_empty() {
    let temp = TempDir::new().unwrap();
    assert!(discover(
        temp.path(),
        Path::new("/project"),
        false,
        &Config::default()
    )
    .unwrap()
    .is_empty());
}

#[test]
fn discovery_codex_home_override_and_platform_fallback() {
    assert_eq!(
        codex_home_from(Some(OsStr::new("/custom/codex")), Some(Path::new("/user"))).unwrap(),
        Path::new("/custom/codex")
    );
    assert_eq!(
        codex_home_from(None, Some(Path::new("/user"))).unwrap(),
        Path::new("/user/.codex")
    );
    assert_eq!(
        codex_home_from(Some(OsStr::new("")), Some(Path::new("/user"))).unwrap(),
        Path::new("/user/.codex")
    );
    assert!(codex_home_from(None, None).is_err());
}

#[test]
fn discovery_resolve_explicit_path_exact_id_prefix_and_ambiguity() {
    let temp = TempDir::new().unwrap();
    let first = fixture(temp.path(), 100, "abc-first", "/project");
    fixture(temp.path(), 0, "abc-second", "/project");
    let config = Config::default();
    assert_eq!(
        resolve_session(temp.path(), first.to_str().unwrap(), &config).unwrap(),
        first
    );
    assert_eq!(
        resolve_session(temp.path(), "abc-first", &config).unwrap(),
        first
    );
    assert_eq!(
        resolve_session(temp.path(), "abc-f", &config).unwrap(),
        first
    );
    assert!(resolve_session(temp.path(), "abc", &config)
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));
    assert!(resolve_session(temp.path(), "not-found", &config).is_err());
    assert!(resolve_session(temp.path(), "", &config).is_err());
    assert!(resolve_session(temp.path(), temp.path().to_str().unwrap(), &config).is_err());
}

#[test]
fn discovery_bom_malformed_oversize_and_partial_rows_are_tolerated_read_only() {
    let temp = TempDir::new().unwrap();
    let path = fixture(temp.path(), 0, "robust", "/project");
    let meta = json!({"type":"session_meta","payload":{"session_id":"robust","cwd":"/project"}});
    let prompt =
        json!({"type":"event_msg","payload":{"type":"user_message","text":"visible prompt"}});
    let data = format!(
        "\u{feff}{meta}\nnot json\n{}\n{prompt}\n{{\"partial\":",
        "x".repeat(2048)
    );
    fs::write(&path, &data).unwrap();
    let config = Config {
        max_record_bytes: 1024,
        ..Config::default()
    };
    let sessions = discover(temp.path(), Path::new("/project"), false, &config).unwrap();
    assert_eq!(sessions[0].id, "robust");
    assert_eq!(sessions[0].first_prompt.as_deref(), Some("visible prompt"));
    assert_eq!(fs::read(&path).unwrap(), data.as_bytes());
}

#[test]
fn discovery_bounds_header_read_and_does_not_read_huge_tail() {
    let temp = TempDir::new().unwrap();
    let path = fixture(temp.path(), 0, "huge", "/project");
    // Sparse tail exercises the cap without allocating a giant test fixture.
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(4 * 1024 * 1024 * 1024)
        .unwrap();
    let sessions = discover(
        temp.path(),
        Path::new("/project"),
        false,
        &Config::default(),
    )
    .unwrap();
    assert_eq!(sessions[0].id, "huge");
}

#[test]
fn discovery_related_paths_use_components_not_string_prefix() {
    let temp = TempDir::new().unwrap();
    fixture(temp.path(), 0, "different", "/project-two");
    fixture(temp.path(), 0, "child", "/project/subdirectory");
    let sessions = discover(
        temp.path(),
        Path::new("/project"),
        false,
        &Config::default(),
    )
    .unwrap();
    assert_eq!(sessions[0].id, "child");
    assert_eq!(auto_select(&sessions, Path::new("/project")), Some(0));
}

#[test]
fn discovery_uses_filename_uuid_when_header_is_unreadable() {
    let temp = TempDir::new().unwrap();
    let id = "12345678-abcd-1234-abcd-1234567890ab";
    let path = fixture(
        temp.path(),
        0,
        &format!("2026-09-06T10-00-00-{id}"),
        "/project",
    );
    fs::write(&path, "not-json\n").unwrap();
    let sessions = discover(
        temp.path(),
        Path::new("/project"),
        false,
        &Config::default(),
    )
    .unwrap();
    assert_eq!(sessions[0].id, id);
    assert_eq!(
        resolve_session(temp.path(), id, &Config::default()).unwrap(),
        path
    );
}

#[test]
fn discovery_index_is_bounded_and_corrupt_rows_do_not_hide_titles() {
    let temp = TempDir::new().unwrap();
    fixture(temp.path(), 0, "recent", "/project");
    let index_path = temp.path().join("session_index.jsonl");
    let index = format!(
        "{}\nnot-json\n{{\"id\":\"recent\",\"title\":\"A valid title\"}}\n{{\"partial\":",
        "x".repeat(2 * 1024 * 1024)
    );
    fs::write(&index_path, index).unwrap();
    let sessions = discover(
        temp.path(),
        Path::new("/project"),
        false,
        &Config::default(),
    )
    .unwrap();
    assert_eq!(sessions[0].title.as_deref(), Some("A valid title"));
}

#[test]
fn discovery_reuses_parser_for_typed_human_messages_and_deduplicated_count() {
    let temp = TempDir::new().unwrap();
    let path = fixture(temp.path(), 0, "typed", "/project");
    let records = [
        json!({"type":"session_meta","payload":{"id":"typed","cwd":"/project"}}),
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"injected instructions"},{"type":"input_text","text":"real human request"}],"internal_chat_message_metadata_passthrough":{"content_item_kinds":["environment_context","user.text"]}}}),
        json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","id":"message-1","content":[{"type":"Text","text":"real human request"}]}}}),
    ];
    fs::write(
        &path,
        records
            .iter()
            .map(|record| format!("{record}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let sessions = discover(
        temp.path(),
        Path::new("/project"),
        false,
        &Config::default(),
    )
    .unwrap();
    assert_eq!(
        sessions[0].first_prompt.as_deref(),
        Some("real human request")
    );
    assert_eq!(sessions[0].turn_count, Some(1));
}

#[test]
fn config_defaults_partial_file_validation_and_no_secret_error() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("config.toml");
    assert_eq!(Config::load_from(&path).unwrap().preview_width, 36);
    fs::write(&path, "watch = false\npreview_width = 48\n").unwrap();
    let config = Config::load_from(&path).unwrap();
    assert!(!config.watch);
    assert_eq!(config.preview_width, 48);
    assert_eq!(config.max_record_bytes, 4 * 1024 * 1024);
    for text in [
        "recent_days = 0",
        "max_record_bytes = 3",
        "preview_width = 0",
    ] {
        fs::write(&path, text).unwrap();
        assert!(Config::load_from(&path).is_err());
    }
    fs::write(&path, "watch = TOP_SECRET").unwrap();
    assert!(!Config::load_from(&path)
        .unwrap_err()
        .to_string()
        .contains("TOP_SECRET"));
    fs::write(&path, "x".repeat(65_537)).unwrap();
    assert!(Config::load_from(&path).is_err());
}

#[test]
fn cli_exposes_required_options_and_doctor() {
    let args = Cli::try_parse_from([
        "codex-nav",
        "--session",
        "abc",
        "--cwd",
        "/project",
        "--all",
        "--no-watch",
    ])
    .unwrap();
    assert_eq!(args.session.as_deref(), Some("abc"));
    assert_eq!(args.cwd, Some(PathBuf::from("/project")));
    assert!(args.all && args.no_watch);
    assert!(matches!(
        Cli::try_parse_from(["codex-nav", "doctor"])
            .unwrap()
            .command,
        Some(Command::Doctor)
    ));
    assert_eq!(
        Cli::try_parse_from(["codex-nav", "--version"])
            .unwrap_err()
            .kind(),
        clap::error::ErrorKind::DisplayVersion
    );
    assert_eq!(
        Cli::try_parse_from(["codex-nav", "--help"])
            .unwrap_err()
            .kind(),
        clap::error::ErrorKind::DisplayHelp
    );
}
