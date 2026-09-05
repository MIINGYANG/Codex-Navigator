use codex_navigator::{
    config::Config,
    discovery::{discover, main_sessions, resolve_session},
    domain::{SessionIdentity, SessionKind, SessionSummary},
    parser::{identity::parse_identity, jsonl_reader::Record, Parser},
};
use serde_json::{json, Value};
use std::{fs, path::Path};
use tempfile::TempDir;
use unicode_width::UnicodeWidthStr;

#[test]
fn parser_updated_timestamp_ignores_invalid_and_older_inherited_records() {
    let mut parser = Parser::new(36);
    for timestamp in ["2026-09-06T10:00:00Z", "2026-09-05T10:00:00Z", "invalid"] {
        let record = json!({"type":"event_msg", "timestamp":timestamp,
            "payload":{"type":"token_count"}})
        .to_string();
        parser.consume(Record::Line(record.as_bytes()));
    }
    assert_eq!(
        parser.session.meta.updated_at.unwrap().to_rfc3339(),
        "2026-09-06T10:00:00+00:00"
    );
}

#[test]
fn identity_recognizes_known_main_string_and_object_sources() {
    for source in [
        json!("cli"),
        json!("vscode"),
        json!("exec"),
        json!("appServer"),
        json!({"cli":{}}),
        json!({"type":"cli"}),
        json!({"kind":"app_server"}),
    ] {
        assert_eq!(
            parse_identity(&json!({"source": source})).kind,
            SessionKind::Main
        );
    }
}

#[test]
fn identity_unknown_sources_and_forks_do_not_invent_provenance() {
    for payload in [
        json!({}),
        json!({"source": null}),
        json!({"source":42}),
        json!({"source":"future"}),
        json!({"source":{"future":{}}}),
        json!({"forked_from_id":"previous", "agent_path":"/root"}),
        json!({"source":{"subagent":null}}),
    ] {
        assert_eq!(parse_identity(&payload).kind, SessionKind::Unknown);
    }
    assert_eq!(
        parse_identity(&json!({"source":"cli","forked_from_id":"previous"})).kind,
        SessionKind::Main
    );
}

#[test]
fn identity_recognizes_spawn_variants_and_parent_metadata() {
    for source in [
        json!("subagent"),
        json!("sub_agent"),
        json!("subAgent"),
        json!({"subagent":"review"}),
        json!({"type":"subagent"}),
        json!({"subAgent":{"threadSpawn":{"parentThreadId":"parent", "agentNickname":"Reviewer"}}}),
    ] {
        assert_eq!(
            parse_identity(&json!({"source":source})).kind,
            SessionKind::Subagent
        );
    }
    let identity = parse_identity(
        &json!({"source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent", "depth":1, "agent_path":"/root/review", "agent_nickname":"Reviewer", "agent_role":"reviewer"}}}}),
    );
    assert_eq!(identity.parent_id.as_deref(), Some("parent"));
    assert_eq!(identity.agent_label.as_deref(), Some("Reviewer"));
    let identity = parse_identity(
        &json!({"source":"future", "parent_thread_id":"parent", "agent_path":"/root/review"}),
    );
    assert_eq!(identity.kind, SessionKind::Subagent);
    assert_eq!(identity.agent_label.as_deref(), Some("/root/review"));
}

#[test]
fn identity_prefers_own_labels_and_falls_back_past_malformed_fields() {
    let identity = parse_identity(
        &json!({"parent_thread_id":false,"agent_nickname":" ","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent","agent_nickname":"Nested","agent_path":"/root/worker"}}}}),
    );
    assert_eq!(identity.parent_id.as_deref(), Some("parent"));
    assert_eq!(identity.agent_label.as_deref(), Some("Nested"));
    let identity = parse_identity(
        &json!({"agent_nickname":"Own", "source":{"subagent":{"thread_spawn":{"agent_nickname":"Nested"}}}}),
    );
    assert_eq!(identity.agent_label.as_deref(), Some("Own"));
}

#[test]
fn identity_sanitizes_and_bounds_untrusted_display_fields() {
    let identity = parse_identity(
        &json!({"parent_thread_id":"pa\u{1b}]52;c;hidden\u{7}rent\u{202e}", "agent_nickname":"\u{1b}[31mReviewer\n\t安全\u{1b}[0m"}),
    );
    assert_eq!(identity.parent_id.as_deref(), Some("parent"));
    assert_eq!(identity.agent_label.as_deref(), Some("Reviewer 安全"));
    for text in ["界".repeat(2000), "\u{301}".repeat(10000)] {
        let identity = parse_identity(&json!({"parent_thread_id":text,"agent_nickname":text}));
        for text in [identity.parent_id.unwrap(), identity.agent_label.unwrap()] {
            assert!(text.len() <= 1024);
            assert!(UnicodeWidthStr::width(text.as_str()) <= 256);
        }
    }
}

fn summary(kind: SessionKind, cwd: &str) -> SessionSummary {
    SessionSummary {
        cwd: Some(cwd.into()),
        identity: SessionIdentity {
            kind,
            ..SessionIdentity::default()
        },
        ..SessionSummary::default()
    }
}

#[test]
fn main_picker_filters_non_main_without_changing_discovery_order() {
    let sessions = main_sessions(vec![
        summary(SessionKind::Subagent, "/synthetic/project"),
        summary(SessionKind::Main, "/synthetic/project"),
        summary(SessionKind::Unknown, "/synthetic/project"),
        summary(SessionKind::Main, "/synthetic"),
    ]);
    assert_eq!(sessions.len(), 2);
    assert_eq!(
        sessions[0].cwd.as_deref(),
        Some(Path::new("/synthetic/project"))
    );
    assert_eq!(sessions[1].cwd.as_deref(), Some(Path::new("/synthetic")));
    assert!(sessions
        .iter()
        .all(|s| s.identity.kind == SessionKind::Main));
}

#[test]
fn main_picker_is_empty_for_unknown_and_subagent_only() {
    assert!(main_sessions(Vec::new()).is_empty());
    assert!(main_sessions(vec![
        summary(SessionKind::Unknown, "/synthetic/project"),
        summary(SessionKind::Subagent, "/synthetic/project"),
    ])
    .is_empty());
    assert_eq!(
        main_sessions(vec![summary(SessionKind::Main, "/synthetic/project")]).len(),
        1
    );
}

#[test]
fn explicit_non_main_resolution_is_not_filtered_or_redirected_to_parent() {
    let temp = TempDir::new().unwrap();
    for (id, source) in [
        ("main", json!("cli")),
        (
            "agent",
            json!({"subagent":{"thread_spawn":{"parent_thread_id":"main"}}}),
        ),
        ("unknown", Value::Null),
    ] {
        write_rollout(
            temp.path(),
            id,
            "/synthetic/project",
            source,
            "2026-09-06T00:00:00Z",
        );
        let path = temp
            .path()
            .join("sessions")
            .join(format!("rollout-{id}.jsonl"));
        assert_eq!(
            resolve_session(temp.path(), id, &Config::default()).unwrap(),
            path
        );
        assert_eq!(
            resolve_session(temp.path(), path.to_str().unwrap(), &Config::default()).unwrap(),
            path
        );
    }
    let discovered = discover(
        temp.path(),
        Path::new("/synthetic/project"),
        true,
        &Config::default(),
    )
    .unwrap();
    assert_eq!(discovered.len(), 3);
    assert_eq!(
        main_sessions(discovered)
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        vec!["main"]
    );
}

fn write_rollout(home: &Path, id: &str, cwd: &str, source: Value, updated_at: &str) {
    let directory = home.join("sessions");
    fs::create_dir_all(&directory).unwrap();
    let meta = json!({"type":"session_meta", "timestamp":updated_at, "payload":{"id":id,"cwd":cwd,"source":source}});
    fs::write(
        directory.join(format!("rollout-{id}.jsonl")),
        format!("{meta}\n"),
    )
    .unwrap();
}

#[test]
fn discovery_propagates_identity_and_ranks_relevance_before_kind_before_time() {
    let temp = TempDir::new().unwrap();
    for (id, cwd, source, time) in [
        (
            "agent",
            "/synthetic/project",
            json!({"subagent":{"thread_spawn":{"parent_thread_id":"main", "agent_nickname":"Worker"}}}),
            "2099-01-05T00:00:00Z",
        ),
        (
            "unknown",
            "/synthetic/project",
            Value::Null,
            "2099-01-04T00:00:00Z",
        ),
        (
            "main-old",
            "/synthetic/project",
            json!("cli"),
            "2099-01-01T00:00:00Z",
        ),
        (
            "main-new",
            "/synthetic/project",
            json!("cli"),
            "2099-01-02T00:00:00Z",
        ),
        (
            "related",
            "/synthetic",
            json!("cli"),
            "2099-01-06T00:00:00Z",
        ),
    ] {
        write_rollout(temp.path(), id, cwd, source, time);
    }
    fs::write(
        temp.path().join("session_index.jsonl"),
        [
            json!({"id":"main-old","updated_at":"2099-01-01T00:00:00Z"}),
            json!({"id":"main-new","updated_at":"2099-01-02T00:00:00Z"}),
            json!({"id":"unknown","updated_at":"2099-01-04T00:00:00Z"}),
            json!({"id":"agent","updated_at":"2099-01-05T00:00:00Z"}),
        ]
        .iter()
        .map(|value| format!("{value}\n"))
        .collect::<String>(),
    )
    .unwrap();
    let sessions = discover(
        temp.path(),
        Path::new("/synthetic/project"),
        true,
        &Config::default(),
    )
    .unwrap();
    assert_eq!(
        sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        ["main-new", "main-old", "unknown", "agent", "related"]
    );
    assert_eq!(sessions[3].identity.agent_label.as_deref(), Some("Worker"));
    assert_eq!(sessions[3].identity.parent_id.as_deref(), Some("main"));
    assert_eq!(
        resolve_session(temp.path(), "agent", &Config::default()).unwrap(),
        sessions[3].path
    );
}

#[test]
fn parser_preserves_first_metadata_identity_in_inherited_histories() {
    for include_id in [true, false] {
        let mut parser = Parser::new(36);
        let mut first = json!({"type":"session_meta","payload":{"source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent", "agent_nickname":"Worker"}}},"cwd":"/synthetic/child"}});
        if include_id {
            first["payload"]["id"] = json!("child");
        }
        let inherited = json!({"type":"session_meta","payload":{"id":"parent","source":"cli","cwd":"/synthetic/parent"}});
        for record in [first, inherited] {
            parser.consume(Record::Line(record.to_string().as_bytes()));
        }
        assert_eq!(parser.session.meta.identity.kind, SessionKind::Subagent);
        assert_eq!(
            parser.session.meta.identity.parent_id.as_deref(),
            Some("parent")
        );
        assert_eq!(
            parser.session.meta.cwd.as_deref(),
            Some(Path::new("/synthetic/child"))
        );
        assert_ne!(parser.session.meta.id, "parent");
    }
}
