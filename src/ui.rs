use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::{
    app::{App, Focus},
    domain::SessionIdentity,
    util::{preview, sanitize},
};

pub fn is_wide(width: u16) -> bool {
    width >= 100
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if area.width < 12 || area.height < 4 {
        frame.render_widget(Paragraph::new("Resize terminal · q quit"), area);
        return;
    }
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
    let header = if app.picker {
        format!(" Codex Navigator · {} sessions", app.summaries.len())
    } else if let Some(session) = &app.session {
        let cwd = session
            .meta
            .cwd
            .as_ref()
            .map(|p| p.to_string_lossy())
            .unwrap_or_default();
        format!(
            " Codex Navigator · {} · {} · {} · {} turns · updated {} · {}",
            if app.live { "WATCHING" } else { "STATIC" },
            identity_label(&session.meta.identity),
            preview(&session.meta.id, 12),
            session.turns.len(),
            session
                .meta
                .updated_at
                .map(|date| date.format("%Y-%m-%d %H:%M UTC").to_string())
                .unwrap_or_else(|| "unknown".into()),
            preview(&cwd, 40)
        )
    } else {
        " Codex Navigator · waiting for session".into()
    };
    frame.render_widget(
        Paragraph::new(header).style(Style::default().add_modifier(Modifier::BOLD)),
        sections[0],
    );
    if app.picker {
        picker(frame, app, sections[1]);
    } else if is_wide(area.width) {
        let panels = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(34), Constraint::Percentage(66)])
            .split(sections[1]);
        timeline(frame, app, panels[0]);
        viewer(frame, app, panels[1]);
    } else if app.focus == Focus::Timeline {
        timeline(frame, app, sections[1]);
    } else {
        viewer(frame, app, sections[1]);
    }

    let mut status = Vec::new();
    if let Some(toast) = &app.toast {
        status.push(sanitize(toast));
    }
    if app.loading {
        status.push(match app.progress {
            Some((read, total)) => format!("Loading {} / {} KiB…", read / 1024, total / 1024),
            None => "Loading…".into(),
        });
    }
    if app.new_turns > 0 {
        status.push(format!(
            "+{} new turn{} · {} latest",
            app.new_turns,
            if app.new_turns == 1 { "" } else { "s" },
            if app.focus == Focus::Viewer {
                "Tab G"
            } else {
                "G"
            }
        ));
    }
    if let Some(s) = &app.session {
        if s.parse_stats.malformed_records > 0 {
            status.push(format!(
                "{} malformed records skipped",
                s.parse_stats.malformed_records
            ));
        }
        if s.parse_stats.skipped_oversize_records > 0 {
            status.push(format!(
                "{} oversized records skipped",
                s.parse_stats.skipped_oversize_records
            ));
        }
        if s.parse_stats.omitted_text_bytes > 0 {
            status.push(format!(
                "{} text bytes omitted (memory limit)",
                s.parse_stats.omitted_text_bytes
            ));
        }
    }
    if status.is_empty() {
        status.push(
            if app.picker {
                "Read-only · local sessions"
            } else {
                "Read-only · Tab switches pane · j/k navigate or scroll"
            }
            .into(),
        );
    }
    frame.render_widget(
        Paragraph::new(format!(" {}", status.join(" · ")))
            .style(Style::default().fg(Color::DarkGray)),
        sections[2],
    );
    let footer = if app.searching {
        " ↑/↓ result  Enter open  Esc cancel"
    } else if app.picker {
        " / search  ↑/↓ choose  Enter open  r refresh  ? help  q quit"
    } else if app.focus == Focus::Viewer {
        " f final  g/G top/end  [/] turn  Tab timeline  / search  c copy  ? help  q quit"
    } else {
        " f final  / search  [/] turn  G latest  Tab focus  c copy  s sessions  ? help  q quit"
    };
    frame.render_widget(Paragraph::new(footer), sections[3]);
    if app.help {
        help(frame, sections[1]);
    }
}

fn panel(title: String, focused: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(if focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        })
}

fn viewport(offset: &mut usize, selected: usize, count: usize, height: usize) -> (usize, usize) {
    if height == 0 {
        return (0, 0);
    }
    if selected < *offset {
        *offset = selected;
    }
    if selected >= offset.saturating_add(height) {
        *offset = selected + 1 - height;
    }
    *offset = (*offset).min(count.saturating_sub(height));
    (*offset, offset.saturating_add(height).min(count))
}

fn timeline(frame: &mut Frame, app: &mut App, area: Rect) {
    let title = if app.searching {
        format!(
            " Search prompts: {}▏ · {} results ",
            sanitize(&app.query),
            app.results.len()
        )
    } else {
        " TIMELINE ".into()
    };
    let block = panel(title, app.focus == Focus::Timeline);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(session) = &app.session else {
        frame.render_widget(Paragraph::new("Opening session…"), inner);
        return;
    };
    if session.turns.is_empty() {
        frame.render_widget(
            Paragraph::new(if app.loading {
                "Loading prompts…"
            } else {
                "No prompts yet. Waiting for session updates…"
            })
            .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }
    let count = if app.searching {
        app.results.len()
    } else {
        session.turns.len()
    };
    if count == 0 {
        frame.render_widget(
            Paragraph::new("No matching prompts. Esc clears search."),
            inner,
        );
        return;
    }
    let selected = if app.searching {
        Some(app.search_cursor)
    } else {
        app.selected
    };
    let (start, end) = viewport(
        &mut app.timeline_offset,
        selected.unwrap_or(0),
        count,
        inner.height as usize,
    );
    let items: Vec<_> = (start..end)
        .map(|position| {
            let i = if app.searching {
                app.results[position]
            } else {
                position
            };
            let turn = &session.turns[i];
            let marker = if selected == Some(position) {
                "▸"
            } else {
                " "
            };
            let warning = if turn.activity.errors > 0 {
                format!(" !{}", turn.activity.errors)
            } else {
                String::new()
            };
            let prefix = format!(
                "{marker} {:02} {}{warning} ",
                turn.ordinal,
                turn.status.symbol()
            );
            let width = inner
                .width
                .saturating_sub(unicode_width::UnicodeWidthStr::width(prefix.as_str()) as u16)
                as usize;
            let text = format!("{prefix}{}", preview(&turn.prompt.preview, width));
            let style = if selected == Some(position) {
                Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
            } else if turn.status == crate::domain::TurnStatus::RolledBack {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };
            ListItem::new(text).style(style)
        })
        .collect();
    frame.render_widget(List::new(items), inner);
}

fn viewer(frame: &mut Frame, app: &mut App, area: Rect) {
    let title = app
        .selected_turn()
        .map(|turn| {
            let duration = turn
                .started_at
                .zip(turn.completed_at)
                .map(|(start, end)| format!(" · {}s", (end - start).num_seconds().max(0)))
                .unwrap_or_default();
            format!(
                " TURN {:02}{duration} · {} ",
                turn.ordinal,
                turn.status.label()
            )
        })
        .unwrap_or_else(|| " TURN VIEW ".into());
    let block = panel(title, app.focus == Focus::Viewer);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let text: Vec<Line<'static>> = app
        .viewer_lines(inner.width as usize, inner.height as usize)
        .into_iter()
        .map(|line| {
            if matches!(
                line.as_str(),
                "USER" | "AGENT" | "FINAL ANSWER" | "OUTPUT" | "NOTICE"
            ) || line.starts_with("ACTIVITY ·")
                || line.starts_with("FILE ·")
                || line.starts_with("TURN STATUS ·")
            {
                Line::from(Span::styled(
                    line,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ))
            } else if line == "ERROR" {
                Line::from(Span::styled(
                    line,
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(line)
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(text), inner);
}

fn picker(frame: &mut Frame, app: &mut App, area: Rect) {
    let title = if app.searching {
        format!(" Search sessions: {}▏ ", sanitize(&app.query))
    } else {
        " SESSIONS ".into()
    };
    let block = panel(title, true);
    let mut inner = block.inner(area);
    frame.render_widget(block, area);
    if let Some(notice) = &app.picker_notice {
        let notice_height = 2.min(inner.height);
        frame.render_widget(
            Paragraph::new(sanitize(notice)).wrap(Wrap { trim: true }),
            Rect {
                height: notice_height,
                ..inner
            },
        );
        inner.y += notice_height;
        inner.height -= notice_height;
    }
    if app.results.is_empty() {
        frame.render_widget(
            Paragraph::new(if app.searching {
                "No matching sessions. Esc clears search."
            } else {
                "No main sessions found. Start Codex in another terminal, then press r to refresh."
            })
            .wrap(Wrap { trim: true }),
            inner,
        );
        return;
    }
    let row_height = if inner.height >= 3 { 3 } else { 1 };
    let count = (inner.height as usize / row_height).max(1);
    let (start, end) = viewport(
        &mut app.picker_offset,
        app.picker_selected,
        app.results.len(),
        count,
    );
    let mut lines = Vec::new();
    for position in start..end {
        let s = &app.summaries[app.results[position]];
        let selected = position == app.picker_selected;
        let title = s
            .title
            .as_deref()
            .or(s.first_prompt.as_deref())
            .unwrap_or("Untitled session");
        let style = if selected {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::styled(
            format!(
                "{} {}",
                if selected { "▸" } else { " " },
                preview(
                    &format!("[{}] {title}", s.identity.kind.label()),
                    inner.width.saturating_sub(2) as usize
                )
            ),
            style,
        ));
        if row_height > 1 {
            let updated = s
                .updated_at
                .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
                .unwrap_or_else(|| "unknown time".into());
            let cwd = s
                .cwd
                .as_ref()
                .map(|p| p.to_string_lossy())
                .unwrap_or_default();
            let turns = s
                .turn_count
                .map(|n| n.to_string())
                .unwrap_or_else(|| "?".into());
            lines.push(
                Line::from(format!(
                    "  {updated} · {turns} turns · {}",
                    preview(&s.id, 12)
                ))
                .style(Style::default().fg(Color::DarkGray)),
            );
            lines.push(
                Line::from(format!(
                    "  {}",
                    preview(
                        &format!("{} · {}", identity_label(&s.identity), cwd),
                        inner.width.saturating_sub(2) as usize
                    )
                ))
                .style(Style::default().fg(Color::DarkGray)),
            );
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn help(frame: &mut Frame, area: Rect) {
    let width = area.width.min(76);
    let height = area.height.min(25);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    let lines = [
        "✓ completed · ✕ execution error · ⊘ interrupted",
        "… incomplete · ? unknown · ↶ rolled back",
        "!N activity errors: independent of turn status",
        "Status does not judge correctness. Review final answer (f).",
        "",
        "↑/k ↓/j     Timeline: select turn · Viewer: scroll",
        "[ / ]       Previous / next turn from either pane",
        "g / G       Timeline: first/latest · Viewer: top/end",
        "Tab         Switch timeline / viewer",
        "Enter       Focus viewer / open picker or search result",
        "PgUp/PgDn   Page · Home/End: viewer top/bottom",
        "f           Last retained final answer in current turn",
        "/           Search prompts / sessions",
        "Esc         Cancel search / focus timeline / leave picker",
        "c / C       Copy prompt/turn · r refresh · s sessions",
        "q / Ctrl+C  Exit Navigator safely",
        "New turns follow only while the latest turn is selected.",
        "? / Esc / Enter closes help. Read-only session access.",
    ]
    .join("\n");
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(panel(" KEYBOARD HELP ".into(), true)),
        popup,
    );
}

fn identity_label(identity: &SessionIdentity) -> String {
    let mut label = identity.kind.label().to_owned();
    if let Some(agent) = &identity.agent_label {
        label.push_str(&format!(" {}", preview(agent, 18)));
    }
    if let Some(parent) = &identity.parent_id {
        label.push_str(&format!(" · parent {}", preview(parent, 12)));
    }
    label
}
