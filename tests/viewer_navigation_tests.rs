use codex_navigator::{
    app::{App, Focus},
    domain::{Session, Turn, TurnItem, UserPrompt},
    ui,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};

fn session(count: usize) -> Session {
    Session {
        turns: (0..count)
            .map(|index| Turn {
                ordinal: index + 1,
                completed_at: Some("2026-09-06T12:34:56Z".parse().unwrap()),
                prompt: UserPrompt {
                    text: format!("Question {index}"),
                    preview: format!("Question {index}"),
                    ..UserPrompt::default()
                },
                items: vec![TurnItem::AgentMessage {
                    text: (0..80)
                        .map(|line| {
                            format!("Line {line}: enough text to wrap differently across terminal widths.\n")
                        })
                        .collect(),
                    phase: None,
                }],
                ..Turn::default()
            })
            .collect(),
        ..Session::default()
    }
}

fn press(app: &mut App, code: KeyCode) {
    let modifiers = if matches!(code, KeyCode::Char('G')) {
        KeyModifiers::SHIFT
    } else {
        KeyModifiers::NONE
    };
    app.handle_key(KeyEvent::new(code, modifiers));
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

fn history_app() -> App {
    let mut app = App::new(true);
    app.open_session(session(3));
    press(&mut app, KeyCode::Char('['));
    app.update_session(session(4));
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.selected, Some(1));
    assert_eq!(app.new_turns, 1);
    assert_eq!(app.focus, Focus::Viewer);
    app
}

#[test]
fn viewer_g_returns_to_top_without_selecting_first_turn() {
    let mut app = history_app();
    render(&mut app, 140, 24);
    press(&mut app, KeyCode::PageDown);
    render(&mut app, 140, 24);
    assert!(app.viewer_scroll > 0);

    press(&mut app, KeyCode::Char('g'));
    let screen = render(&mut app, 140, 24);
    assert_eq!(app.selected, Some(1));
    assert_eq!(app.new_turns, 1);
    assert_eq!(app.viewer_scroll, 0);
    assert!(screen.contains("USER"));
    assert!(screen.contains("Question 1"));
}

#[test]
fn viewer_g_and_shift_g_work_after_wide_narrow_wide_resizes() {
    let mut app = history_app();
    for (width, height) in [(140, 24), (60, 16), (120, 30)] {
        render(&mut app, width, height);
        press(&mut app, KeyCode::Char('G'));
        let bottom = render(&mut app, width, height);
        assert_eq!(app.focus, Focus::Viewer);
        assert_eq!(app.selected, Some(1));
        assert_eq!(app.new_turns, 1);
        assert!(app.viewer_scroll > 0);
        assert!(bottom.contains("Ended 2026-09-06 12:34:56 UTC"));
        assert!(!bottom.contains("USER"));

        press(&mut app, KeyCode::Char('g'));
        let top = render(&mut app, width, height);
        assert_eq!(app.viewer_scroll, 0);
        assert_eq!(app.selected, Some(1));
        assert_eq!(app.new_turns, 1);
        assert!(top.contains("USER"));
        assert!(top.contains("Question 1"));
    }

    press(&mut app, KeyCode::Tab);
    assert_eq!(app.focus, Focus::Timeline);
    press(&mut app, KeyCode::Char('g'));
    assert_eq!(app.selected, Some(0));
    assert_eq!(app.new_turns, 1);
    press(&mut app, KeyCode::Char('G'));
    assert_eq!(app.selected, Some(3));
    assert_eq!(app.new_turns, 0);
}

#[test]
fn viewer_boundary_keys_handle_empty_short_and_tiny_views() {
    for mut source in [Session::default(), session(1)] {
        for turn in &mut source.turns {
            turn.items.clear();
        }
        let mut app = App::new(false);
        app.open_session(source);
        press(&mut app, KeyCode::Tab);
        let selected = app.selected;
        for (width, height) in [(140, 24), (60, 24), (12, 4), (1, 1), (140, 24)] {
            render(&mut app, width, height);
            press(&mut app, KeyCode::Char('G'));
            render(&mut app, width, height);
            assert_eq!(app.selected, selected);
            press(&mut app, KeyCode::Char('g'));
            render(&mut app, width, height);
            assert_eq!(app.viewer_scroll, 0);
            assert_eq!(app.selected, selected);
        }
    }
}

#[test]
fn g_and_shift_g_remain_text_in_search() {
    let mut app = history_app();
    for picker in [false, true] {
        if picker {
            app.show_picker(Vec::new(), None);
        }
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('G'));
        assert!(app.searching);
        assert_eq!(app.query, "gG");
        assert_eq!(app.selected, Some(1));
        assert_eq!(app.new_turns, 1);
        press(&mut app, KeyCode::Esc);
    }
}
