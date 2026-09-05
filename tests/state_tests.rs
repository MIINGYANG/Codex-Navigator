use codex_navigator::{
    app::{Action, App, Focus},
    domain::{Session, SessionSummary, Turn, TurnItem, TurnStatus, UserPrompt},
    index::SearchIndex,
    ui, util,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};

fn session(prompts: &[&str]) -> Session {
    Session {
        turns: prompts
            .iter()
            .enumerate()
            .map(|(i, text)| Turn {
                ordinal: i + 1,
                revision: i as u64,
                prompt: UserPrompt {
                    text: text.to_string(),
                    preview: text.to_string(),
                    ..UserPrompt::default()
                },
                ..Turn::default()
            })
            .collect(),
        ..Session::default()
    }
}
fn key(app: &mut App, code: KeyCode) -> Action {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn source_identity_change_resets_search_scroll_and_selection() {
    let mut app = App::new(true);
    let mut old = session(&["old first", "old second"]);
    old.meta.id = "old-source".into();
    app.open_session(old);
    key(&mut app, KeyCode::Char('g'));
    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Char('o'));
    app.viewer_scroll = 99;
    let mut new = session(&["new first", "new last"]);
    new.meta.id = "new-source".into();
    app.apply_update(
        new.meta,
        new.parse_stats,
        1,
        new.turns.into_iter().enumerate().collect(),
        true,
    );
    assert_eq!(app.selected, Some(1));
    assert_eq!(app.viewer_scroll, 0);
    assert!(!app.searching);
    assert!(app.query.is_empty());
}

#[test]
fn state_history_is_not_stolen_by_new_turns() {
    let mut app = App::new(true);
    app.open_session(session(&["one", "two"]));
    key(&mut app, KeyCode::Char('g'));
    app.update_session(session(&["one", "two", "three"]));
    assert_eq!(app.selected, Some(0));
    assert_eq!(app.new_turns, 1);
    key(&mut app, KeyCode::Char('G'));
    assert_eq!(app.selected, Some(2));
    assert_eq!(app.new_turns, 0);
}

#[test]
fn state_latest_follows_and_skips_rolled_back_turns() {
    let mut app = App::new(true);
    app.open_session(session(&["one"]));
    app.update_session(session(&["one", "two"]));
    assert_eq!(app.selected, Some(1));
    let mut rolled = session(&["one", "two"]);
    rolled.turns[1].status = TurnStatus::RolledBack;
    app.update_session(rolled);
    assert_eq!(app.selected, Some(0));
}

#[test]
fn state_search_enter_selects_original_turn_index() {
    let mut app = App::new(false);
    app.open_session(session(&["401 auth failure", "hello", "auth token"]));
    key(&mut app, KeyCode::Char('/'));
    for c in "401".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    assert_eq!(app.results, vec![0]);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.selected, Some(0));
    assert_eq!(app.focus, Focus::Viewer);
    assert!(!app.searching);
    assert_eq!(app.session.unwrap().turns.len(), 3);
}

#[test]
fn state_search_escape_preserves_history_and_live_search_does_not_steal_focus() {
    let mut app = App::new(true);
    app.open_session(session(&["one", "two"]));
    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Char('o'));
    app.update_session(session(&["one", "two", "three"]));
    assert_eq!(app.selected, Some(1));
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.selected, Some(1));
    assert!(app.query.is_empty());
}

#[test]
fn state_session_switch_and_selection_reset_scroll() {
    let mut app = App::new(false);
    app.open_session(session(&["one", "two"]));
    app.viewer_scroll = 100;
    key(&mut app, KeyCode::Char('['));
    assert_eq!(app.viewer_scroll, 0);
    app.viewer_scroll = 20;
    app.open_session(session(&["new"]));
    assert_eq!(app.viewer_scroll, 0);
}

#[test]
fn state_delta_updates_append_search_and_preserve_history() {
    let mut app = App::new(true);
    app.open_session(session(&["one", "two"]));
    key(&mut app, KeyCode::Char('g'));
    let next = session(&["one", "two", "delta"]);
    app.apply_update(
        next.meta,
        next.parse_stats,
        1,
        vec![(2, next.turns[2].clone())],
        false,
    );
    assert_eq!(app.selected, Some(0));
    assert_eq!(app.new_turns, 1);
    key(&mut app, KeyCode::Char('/'));
    for c in "delta".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    assert_eq!(app.results, vec![2]);
}

#[test]
fn search_substring_unicode_fuzzy_ties_and_incremental_update() {
    let mut index = SearchIndex::default();
    let mut s = session(&["Fix AUTH", "authentication", "中文错误", "Fix AUTH"]);
    index.sync(&s.turns);
    assert_eq!(index.search("AUTH"), vec![1, 3, 0]);
    assert_eq!(index.search("文错"), vec![2]);
    assert!(index.search("fxath").contains(&0));
    s.turns[0].prompt.text = "changed".into();
    s.turns[0].revision += 1;
    index.sync(&s.turns);
    assert_eq!(index.search("changed"), vec![0]);
}

#[test]
fn unicode_preview_never_breaks_graphemes_and_strips_escape_commands() {
    assert_eq!(util::preview(" \n 中\t文 \n hello", 8), "中 文 h…");
    assert_eq!(util::preview("e\u{301}xx", 2), "e\u{301}…");
    assert_eq!(util::preview("👩‍💻abc", 3), "👩‍💻…");
    assert_eq!(util::preview("中文", 0), "");
    assert_eq!(
        util::sanitize(
            "safe\x1b]52;c;bad\x07\x1b[31mred\x1b[0m\x1b]8;;url\x1b\\link\x1b]8;;\x1b\\"
        ),
        "saferedlink"
    );
    assert_eq!(util::sanitize("a\u{9d}secret\u{9c}b\u{202e}c"), "abc");
}

#[test]
fn state_clipboard_uses_only_visible_sanitized_normalized_text() {
    let mut s = session(&["hello\x1b[2J"]);
    s.turns[0].items.push(TurnItem::AgentMessage {
        text: "visible".into(),
        phase: None,
    });
    let mut app = App::new(false);
    app.open_session(s);
    assert_eq!(
        key(&mut app, KeyCode::Char('c')),
        Action::Copy("hello".into())
    );
    let Action::Copy(full) = key(&mut app, KeyCode::Char('C')) else {
        panic!("copy expected")
    };
    assert!(full.contains("visible"));
    assert!(!full.contains('\x1b'));
    assert_eq!(
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        Action::Quit
    );
}

#[test]
fn ui_wide_narrow_tiny_and_empty_terminals_do_not_panic() {
    for (width, height) in [
        (140, 40),
        (100, 24),
        (99, 24),
        (40, 10),
        (12, 4),
        (1, 1),
        (0, 0),
    ] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut app = App::new(true);
        app.open_session(session(&["中文 👩‍💻 prompt", "two"]));
        for focus in [Focus::Timeline, Focus::Viewer] {
            app.focus = focus;
            terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
        }
        app.help = true;
        terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
        app.help = false;
        app.show_picker(
            Vec::new(),
            Some("No recent session matched this directory.".into()),
        );
        terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
    }
    assert!(ui::is_wide(100));
    assert!(!ui::is_wide(99));
}

#[test]
fn viewer_cache_updates_revisions_and_scroll_clamps() {
    let mut app = App::new(false);
    app.open_session(session(&["one"]));
    assert!(app.viewer_lines(30, 3).join("\n").contains("one"));
    let mut next = session(&["changed"]);
    next.turns[0].revision += 1;
    app.update_session(next);
    assert!(app.viewer_lines(30, 3).join("\n").contains("changed"));
    app.viewer_scroll = usize::MAX;
    assert!(!app.viewer_lines(30, 3).is_empty());
    assert!(app.viewer_scroll < usize::MAX);
}

#[test]
fn picker_searches_metadata_and_opens_selected_path() {
    let mut app = App::new(false);
    app.show_picker(
        vec![
            SessionSummary {
                id: "first".into(),
                path: "/fixtures/one.jsonl".into(),
                title: Some("Authentication".into()),
                cwd: Some("/project/api".into()),
                ..SessionSummary::default()
            },
            SessionSummary {
                id: "second".into(),
                path: "/fixtures/two.jsonl".into(),
                title: Some("Build".into()),
                cwd: Some("/project/ui".into()),
                ..SessionSummary::default()
            },
        ],
        None,
    );
    key(&mut app, KeyCode::Char('/'));
    for c in "/project/ui".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    assert_eq!(app.results, vec![1]);
    assert_eq!(
        key(&mut app, KeyCode::Enter),
        Action::OpenSession("/fixtures/two.jsonl".into())
    );
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.results, vec![0, 1]);
    assert_eq!(key(&mut app, KeyCode::Char('q')), Action::Quit);
}

#[test]
fn responsive_layout_shows_focused_pane_and_visible_turn_text() {
    let mut app = App::new(false);
    app.open_session(session(&["Find authentication failure"]));
    let mut wide = Terminal::new(TestBackend::new(120, 20)).unwrap();
    wide.draw(|frame| ui::draw(frame, &mut app)).unwrap();
    let rendered = format!("{:?}", wide.backend().buffer());
    assert!(rendered.contains("TIMELINE"));
    assert!(rendered.contains("TURN 01"));
    assert!(rendered.contains("USER"));
    let mut narrow = Terminal::new(TestBackend::new(60, 20)).unwrap();
    narrow.draw(|frame| ui::draw(frame, &mut app)).unwrap();
    let rendered = format!("{:?}", narrow.backend().buffer());
    assert!(rendered.contains("TIMELINE"));
    assert!(!rendered.contains("TURN 01"));
    key(&mut app, KeyCode::Tab);
    narrow.draw(|frame| ui::draw(frame, &mut app)).unwrap();
    let rendered = format!("{:?}", narrow.backend().buffer());
    assert!(rendered.contains("TURN 01"));
    assert!(!rendered.contains("TIMELINE"));
}

#[test]
fn reset_update_rebuilds_index_without_stale_search_or_viewer_text() {
    let mut app = App::new(true);
    app.open_session(session(&["old text", "historical"]));
    key(&mut app, KeyCode::Char('g'));
    assert!(app.viewer_lines(80, 10).join("\n").contains("old text"));
    let replacement = session(&["replacement"]);
    app.apply_update(
        replacement.meta,
        replacement.parse_stats,
        9,
        replacement.turns.into_iter().enumerate().collect(),
        true,
    );
    assert_eq!(app.selected, Some(0));
    assert!(app.viewer_lines(80, 10).join("\n").contains("replacement"));
    key(&mut app, KeyCode::Char('/'));
    for c in "old text".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    assert!(app.results.is_empty());
}

#[test]
fn search_backspace_removes_a_whole_grapheme() {
    let mut app = App::new(false);
    app.open_session(session(&["emoji"]));
    key(&mut app, KeyCode::Char('/'));
    for c in "a👩‍💻".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Backspace);
    assert_eq!(app.query, "a");
}

#[test]
fn search_cancel_keeps_a_valid_selection_after_truncation_or_first_prompt() {
    let mut app = App::new(true);
    app.open_session(session(&["one", "two", "three"]));
    key(&mut app, KeyCode::Char('/'));
    let replacement = session(&["one"]);
    app.apply_update(
        replacement.meta,
        replacement.parse_stats,
        9,
        replacement.turns.into_iter().enumerate().collect(),
        true,
    );
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.selected, Some(0));
    assert_eq!(app.selected_turn().unwrap().prompt.text, "one");

    app.open_session(Session::default());
    key(&mut app, KeyCode::Char('/'));
    app.update_session(session(&["arrived during search"]));
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.selected, Some(0));
}

#[test]
fn live_matching_turn_does_not_change_the_search_result_chosen_by_keyboard() {
    let mut app = App::new(true);
    app.open_session(session(&["auth first", "unrelated", "auth second"]));
    key(&mut app, KeyCode::Char('/'));
    for c in "auth".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Down);
    assert_eq!(app.results[app.search_cursor], 0);
    let next = session(&["auth first", "unrelated", "auth second", "auth newest"]);
    app.apply_update(
        next.meta,
        next.parse_stats,
        7,
        vec![(3, next.turns[3].clone())],
        false,
    );
    assert_eq!(app.results[app.search_cursor], 0);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.selected, Some(0));
}

#[test]
fn all_rolled_back_history_has_no_fake_active_selection_and_remains_browsable() {
    let mut rolled = session(&["first rolled back", "second rolled back"]);
    for turn in &mut rolled.turns {
        turn.status = TurnStatus::RolledBack;
    }
    let mut app = App::new(false);
    app.open_session(rolled);
    assert_eq!(app.selected, None);
    assert!(app
        .viewer_lines(100, 8)
        .join("\n")
        .contains("No active turn"));
    let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
    assert!(!format!("{:?}", terminal.backend().buffer()).contains('▸'));
    key(&mut app, KeyCode::Char('j'));
    assert_eq!(app.selected, Some(0));
    assert!(app
        .viewer_lines(100, 8)
        .join("\n")
        .contains("first rolled back"));
    key(&mut app, KeyCode::Char('G'));
    key(&mut app, KeyCode::Char('k'));
    assert_eq!(app.selected, Some(1));
}
