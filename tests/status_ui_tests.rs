use codex_navigator::{
    app::{turn_text, App, Focus},
    domain::{ActivitySummary, Session, Turn, TurnStatus, UserPrompt},
    ui,
};
use ratatui::{backend::TestBackend, Terminal};

fn turn(status: TurnStatus, errors: usize) -> Turn {
    Turn {
        ordinal: 1,
        status,
        prompt: UserPrompt {
            text: "Check the answer".into(),
            preview: "Check the answer".into(),
            ..UserPrompt::default()
        },
        activity: ActivitySummary {
            errors,
            ..ActivitySummary::default()
        },
        ..Turn::default()
    }
}

fn app(status: TurnStatus, errors: usize) -> App {
    let mut app = App::new(false);
    app.open_session(Session {
        turns: vec![turn(status, errors)],
        ..Session::default()
    });
    // Rolled-back turns remain inspectable even without an active turn.
    app.selected = Some(0);
    app
}

fn render(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn timeline_shows_lifecycle_and_activity_errors_independently_across_resize() {
    for (status, symbol) in [
        (TurnStatus::Completed, "✓"),
        (TurnStatus::InProgress, "…"),
        (TurnStatus::Failed, "✕"),
        (TurnStatus::Interrupted, "⊘"),
        (TurnStatus::RolledBack, "↶"),
        (TurnStatus::Unknown, "?"),
    ] {
        for errors in [0, 1, 12] {
            let mut app = app(status, errors);
            for width in [140, 60, 24, 120] {
                let screen = render(&mut app, width, 24);
                let expected = if errors == 0 {
                    format!("01 {symbol} ")
                } else {
                    format!("01 {symbol} !{errors} ")
                };
                assert!(screen.contains(&expected), "{width}: {screen}");
                if errors == 0 {
                    assert!(!screen.contains("!0"));
                }
                assert_eq!(app.selected, Some(0));
                assert_eq!(app.focus, Focus::Timeline);
            }
        }
    }
}

#[test]
fn turn_summary_reports_execution_state_without_judging_answer_correctness() {
    for (status, label) in [
        (TurnStatus::Completed, "completed"),
        (TurnStatus::InProgress, "incomplete"),
        (TurnStatus::Failed, "execution error"),
        (TurnStatus::Interrupted, "interrupted"),
        (TurnStatus::RolledBack, "rolled back"),
        (TurnStatus::Unknown, "unknown"),
    ] {
        let text = turn_text(&turn(status, 2));
        assert!(text.contains(&format!("TURN STATUS · {label}")));
        assert!(text.contains("2 activity errors"));
        assert!(text.contains("Completion is not a correctness verdict."));
        assert!(text.contains("Press f to review the final answer."));
        assert!(!text.contains("RESULT ·"));
    }
}

#[test]
fn viewer_keeps_status_warning_and_disclaimer_readable_in_wide_and_narrow_layouts() {
    let mut app = app(TurnStatus::Completed, 1);
    app.focus = Focus::Viewer;
    for width in [140, 60, 120] {
        let screen = render(&mut app, width, 30);
        assert!(screen.contains("TURN STATUS · completed"));
        assert!(screen.contains("1 activity errors"));
        // Wrapping is allowed, but all words and the review shortcut remain visible.
        assert!(screen.contains("correctness verdict."));
        assert!(screen.contains("Press f to review"));
        assert_eq!(app.selected, Some(0));
        assert_eq!(app.focus, Focus::Viewer);
    }
}

#[test]
fn help_explains_separate_lifecycle_and_warning_signals() {
    let mut app = app(TurnStatus::Completed, 1);
    app.help = true;
    for width in [140, 60] {
        let screen = render(&mut app, width, 24);
        for explanation in [
            "✓ completed",
            "✕ execution error",
            "⊘ interrupted",
            "↶ rolled back",
            "!N activity errors: independent of turn status",
            "Status does not judge correctness.",
            "? / Esc / Enter closes help.",
        ] {
            assert!(screen.contains(explanation), "{screen}");
        }
    }
}
