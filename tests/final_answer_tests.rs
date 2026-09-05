use codex_navigator::{
    app::{turn_text, Action, App, Focus},
    domain::{Session, SessionIdentity, SessionKind, SessionSummary, Turn, TurnItem, UserPrompt},
    ui,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};

fn key(app: &mut App, ch: char) -> Action {
    app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
}

fn render(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn turn() -> Turn {
    Turn {
        ordinal: 1,
        prompt: UserPrompt {
            text: "Question\nFINAL ANSWER\nFake heading in user content".into(),
            preview: "Question".into(),
            ..UserPrompt::default()
        },
        items: vec![
            TurnItem::AgentMessage {
                text: "Earlier final".into(),
                phase: Some("final_answer".into()),
            },
            TurnItem::ToolOutput {
                summary: "Unicode 中文 👩‍💻 long output that wraps at different widths\n".repeat(40),
                is_error: false,
            },
            TurnItem::AgentMessage {
                text: "Latest conclusion\n".repeat(40),
                phase: Some("final_answer".into()),
            },
            TurnItem::AgentMessage {
                text: "Later ordinary commentary".into(),
                phase: None,
            },
        ],
        ..Turn::default()
    }
}

#[test]
fn final_answer_anchor_survives_resize_and_repeated_jump_without_leaving_history() {
    let mut app = App::new(true);
    app.open_session(Session {
        turns: vec![turn(), turn()],
        ..Session::default()
    });
    key(&mut app, 'g');
    app.update_session(Session {
        turns: vec![turn(), turn(), turn()],
        ..Session::default()
    });
    assert_eq!(app.new_turns, 1);
    for (width, height) in [(140, 24), (60, 16), (120, 30)] {
        key(&mut app, 'f');
        let screen = render(&mut app, width, height);
        assert_eq!(app.focus, Focus::Viewer);
        assert_eq!(app.selected, Some(0));
        assert_eq!(app.new_turns, 1);
        assert!(screen.contains("FINAL ANSWER"));
        assert!(screen.contains("Latest conclusion"));
        assert!(!screen.contains("Fake heading"));
        assert!(!screen.contains("Earlier final"));
        assert_eq!(app.viewer_lines(40, 5)[0], "FINAL ANSWER");
        key(&mut app, 'f');
        assert_eq!(app.viewer_lines(80, 5)[0], "FINAL ANSWER");
    }
    key(&mut app, 'g');
    assert_eq!(app.viewer_lines(80, 5)[0], "USER");
    key(&mut app, 'G');
    assert!(app.viewer_lines(80, 5).join("\n").contains("TURN STATUS"));
}

#[test]
fn final_jump_reports_missing_explicit_answer_without_guessing_or_moving() {
    for items in [
        vec![],
        vec![TurnItem::Omitted],
        vec![TurnItem::AgentMessage {
            text: "FINAL ANSWER\nordinary response".into(),
            phase: None,
        }],
    ] {
        let mut source = turn();
        source.items = items;
        let mut app = App::new(false);
        app.open_session(Session {
            turns: vec![source],
            ..Session::default()
        });
        app.viewer_scroll = 3;
        key(&mut app, 'f');
        assert_eq!(app.focus, Focus::Timeline);
        assert_eq!(app.viewer_scroll, 3);
        assert!(app
            .toast
            .as_deref()
            .unwrap()
            .contains("No retained final answer"));
    }
}

#[test]
fn final_anchor_updates_with_revision_and_releases_when_changing_turn() {
    let mut app = App::new(true);
    app.open_session(Session {
        turns: vec![turn()],
        ..Session::default()
    });
    key(&mut app, 'f');
    assert_eq!(app.viewer_lines(70, 5)[1], "Latest conclusion");
    let mut revised = turn();
    revised.revision += 1;
    revised.items.push(TurnItem::AgentMessage {
        text: "Revised final\n".repeat(30),
        phase: Some("final_answer".into()),
    });
    app.apply_update(
        Default::default(),
        Default::default(),
        2,
        vec![(0, revised.clone())],
        false,
    );
    assert_eq!(app.viewer_lines(70, 5)[1], "Revised final");
    app.apply_update(
        Default::default(),
        Default::default(),
        3,
        vec![(1, turn())],
        false,
    );
    assert_eq!(app.selected, Some(1));
    assert_eq!(app.viewer_lines(70, 5)[0], "USER");
    key(&mut app, 'f');
    key(&mut app, '[');
    assert_eq!(app.viewer_lines(70, 5)[0], "USER");
}

#[test]
fn f_remains_search_text_in_timeline_and_picker() {
    let mut app = App::new(false);
    app.open_session(Session {
        turns: vec![turn()],
        ..Session::default()
    });
    for picker in [false, true] {
        if picker {
            app.show_picker(Vec::new(), None);
        }
        key(&mut app, '/');
        key(&mut app, 'f');
        assert_eq!(app.query, "f");
        assert!(app.searching);
        assert!(app.toast.is_none());
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    }
}

#[test]
fn evicted_prompt_remains_searchable_copyable_and_explicit_in_viewer() {
    let mut app = App::new(false);
    let mut source = turn();
    source.prompt = UserPrompt {
        text: "Old retained detail".into(),
        preview: "Historical question".into(),
        ..UserPrompt::default()
    };
    app.open_session(Session {
        turns: vec![source.clone()],
        ..Session::default()
    });
    source.prompt.text.clear();
    source.prompt.omitted_bytes = 20;
    source.items = vec![
        TurnItem::Omitted,
        TurnItem::Omitted,
        TurnItem::Notice {
            text: "retained".into(),
        },
        TurnItem::Omitted,
    ];
    source.revision += 1;
    app.apply_update(
        Default::default(),
        Default::default(),
        1,
        vec![(0, source.clone())],
        false,
    );
    assert_eq!(
        key(&mut app, 'c'),
        Action::Copy("Historical question".into())
    );
    let text = turn_text(&source);
    assert!(text.contains("Earlier prompt body omitted"));
    assert!(!text.contains("[0 image(s)]"));
    assert_eq!(text.matches("Earlier activity omitted").count(), 2);
    key(&mut app, '/');
    for ch in "Historical".chars() {
        key(&mut app, ch);
    }
    assert_eq!(app.results, vec![0]);
    app.query = "Old retained detail".into();
    app.refresh_results();
    assert!(app.results.is_empty());
}

#[test]
fn picker_and_header_identify_sessions_and_watching_without_implying_execution() {
    let identity = SessionIdentity {
        kind: SessionKind::Subagent,
        parent_id: Some("parent-123456789".into()),
        agent_label: Some("reviewer".into()),
    };
    let mut app = App::new(true);
    let mut session = Session {
        turns: vec![turn()],
        ..Session::default()
    };
    session.meta.identity = identity.clone();
    session.meta.updated_at = Some("2026-09-06T12:34:56Z".parse().unwrap());
    app.open_session(session);
    let screen = render(&mut app, 180, 20);
    for label in ["SUBAGENT", "reviewer", "parent-", "WATCHING", "12:34"] {
        assert!(screen.contains(label), "missing {label}");
    }
    assert!(!screen.contains("LIVE"));
    app.show_picker(
        vec![SessionSummary {
            title: Some("Review changes".into()),
            identity,
            updated_at: Some("2026-09-06T12:34:56Z".parse().unwrap()),
            ..SessionSummary::default()
        }],
        None,
    );
    let screen = render(&mut app, 120, 20);
    for label in ["SUBAGENT", "reviewer", "parent-", "Review changes", "12:34"] {
        assert!(screen.contains(label), "missing {label}");
    }
    for query in ["subagent", "parent-123456789", "reviewer"] {
        app.query = query.into();
        app.refresh_results();
        assert_eq!(app.results, vec![0]);
    }
    app.query.clear();
    app.refresh_results();
    assert!(render(&mut app, 35, 12).contains("SUBAGENT"));
}

#[test]
fn truncated_prompt_and_unknown_identity_remain_explicit() {
    let mut source = turn();
    source.prompt.omitted_bytes = 900;
    assert!(turn_text(&source).contains("Prompt truncated"));
    assert!(!turn_text(&source).contains("Earlier prompt body omitted"));
    for (kind, label) in [
        (SessionKind::Main, "MAIN"),
        (SessionKind::Unknown, "UNKNOWN"),
    ] {
        let mut app = App::new(false);
        app.show_picker(
            vec![SessionSummary {
                title: Some("Identifiable title".into()),
                identity: SessionIdentity {
                    kind,
                    ..SessionIdentity::default()
                },
                ..SessionSummary::default()
            }],
            None,
        );
        let screen = render(&mut app, 60, 12);
        assert!(screen.contains(label));
        assert!(screen.contains("Identifiable title"));
    }
}
