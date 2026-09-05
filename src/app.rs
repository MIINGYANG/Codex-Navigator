use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::{
    domain::{ParseStats, Session, SessionMeta, SessionSummary, Turn, TurnItem},
    index::{score, SearchIndex},
    util::{sanitize, wrap},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Focus {
    #[default]
    Timeline,
    Viewer,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    Refresh,
    ShowPicker,
    OpenSession(PathBuf),
    Copy(String),
}

#[derive(Default)]
pub struct App {
    pub session: Option<Session>,
    pub selected: Option<usize>,
    pub focus: Focus,
    pub viewer_scroll: usize,
    pub viewer_height: usize,
    pub new_turns: usize,
    pub live: bool,
    pub loading: bool,
    pub progress: Option<(u64, u64)>,
    pub toast: Option<String>,
    pub help: bool,
    pub picker: bool,
    pub summaries: Vec<SessionSummary>,
    pub picker_notice: Option<String>,
    pub picker_selected: usize,
    pub searching: bool,
    pub query: String,
    pub results: Vec<usize>,
    pub search_cursor: usize,
    pub timeline_offset: usize,
    pub picker_offset: usize,
    index: SearchIndex,
    search_original: Option<usize>,
    viewer_cache: Option<(usize, u64, usize, Vec<String>)>,
}

impl App {
    pub fn new(live: bool) -> Self {
        Self {
            live,
            ..Self::default()
        }
    }

    pub fn open_session(&mut self, session: Session) {
        self.selected = session.latest_active();
        self.index = SearchIndex::default();
        self.index.sync(&session.turns);
        self.session = Some(session);
        self.picker = false;
        self.searching = false;
        self.query.clear();
        self.viewer_scroll = 0;
        self.viewer_cache = None;
        self.new_turns = 0;
        self.timeline_offset = 0;
        self.focus = Focus::Timeline;
        self.refresh_results();
    }

    pub fn update_session(&mut self, session: Session) {
        let old_count = self.session.as_ref().map_or(0, |s| s.turns.len());
        let old_latest = self.session.as_ref().and_then(Session::latest_active);
        let follow = !self.searching && self.selected == old_latest;
        if follow {
            if self.selected != session.latest_active() {
                self.viewer_scroll = 0;
            }
            self.selected = session.latest_active();
            self.new_turns = 0;
        } else {
            self.new_turns += session.turns.len().saturating_sub(old_count);
            if self.selected.is_some_and(|i| i >= session.turns.len()) {
                self.selected = session.latest_active();
                self.viewer_scroll = 0;
            }
        }
        self.index.sync(&session.turns);
        self.session = Some(session);
        self.refresh_results_preserving_turn();
    }

    pub fn apply_update(
        &mut self,
        meta: SessionMeta,
        stats: ParseStats,
        revision: u64,
        changed: Vec<(usize, Turn)>,
        reset: bool,
    ) {
        if reset {
            let source_changed = self.session.as_ref().is_some_and(|session| {
                !session.meta.id.is_empty() && !meta.id.is_empty() && session.meta.id != meta.id
            });
            let mut session = Session {
                meta,
                parse_stats: stats,
                revision,
                ..Session::default()
            };
            for (index, turn) in changed {
                if session.turns.len() <= index {
                    session.turns.resize_with(index + 1, Turn::default);
                }
                session.turns[index] = turn;
            }
            self.index = SearchIndex::default();
            self.viewer_cache = None;
            if source_changed {
                self.open_session(session);
                return;
            }
            self.update_session(session);
            return;
        }
        let session = self.session.get_or_insert_with(Session::default);
        let old_count = session.turns.len();
        let follow = !self.searching && self.selected == session.latest_active();
        session.meta = meta;
        session.parse_stats = stats;
        session.revision = revision;
        for (index, turn) in changed {
            self.index.update(index, &turn);
            if session.turns.len() <= index {
                session.turns.resize_with(index + 1, Turn::default);
            }
            session.turns[index] = turn;
        }
        if follow {
            if self.selected != session.latest_active() {
                self.viewer_scroll = 0;
            }
            self.selected = session.latest_active();
            self.new_turns = 0;
        } else {
            self.new_turns += session.turns.len().saturating_sub(old_count);
        }
        self.refresh_results_preserving_turn();
    }

    pub fn show_picker(&mut self, summaries: Vec<SessionSummary>, notice: Option<String>) {
        self.summaries = summaries;
        self.picker_notice = notice;
        self.picker = true;
        self.picker_selected = 0;
        self.picker_offset = 0;
        self.searching = false;
        self.query.clear();
        self.refresh_results();
    }

    pub fn selected_turn(&self) -> Option<&Turn> {
        self.session.as_ref()?.turns.get(self.selected?)
    }

    pub fn refresh_results(&mut self) {
        self.results = if self.picker {
            let query = self.query.trim().to_lowercase();
            let mut ranked: Vec<_> = self
                .summaries
                .iter()
                .enumerate()
                .filter_map(|(i, s)| {
                    let haystack = format!(
                        "{} {} {} {}",
                        s.title.as_deref().unwrap_or_default(),
                        s.first_prompt.as_deref().unwrap_or_default(),
                        s.id,
                        s.cwd
                            .as_ref()
                            .map(|p| p.to_string_lossy())
                            .unwrap_or_default()
                    )
                    .to_lowercase();
                    if query.is_empty() {
                        Some((i, 0))
                    } else {
                        score(&haystack, &query).map(|s| (i, s))
                    }
                })
                .collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            ranked.into_iter().map(|(i, _)| i).collect()
        } else {
            self.index.search(&self.query)
        };
        self.search_cursor = self.search_cursor.min(self.results.len().saturating_sub(1));
        self.picker_selected = self
            .picker_selected
            .min(self.results.len().saturating_sub(1));
    }

    fn refresh_results_preserving_turn(&mut self) {
        let focused = if self.searching && !self.picker {
            self.results.get(self.search_cursor).copied()
        } else {
            None
        };
        self.refresh_results();
        if let Some(position) =
            focused.and_then(|index| self.results.iter().position(|&i| i == index))
        {
            self.search_cursor = position;
        }
    }

    fn select(&mut self, selected: Option<usize>) {
        let selected =
            selected.filter(|&index| self.session.as_ref().is_some_and(|s| index < s.turns.len()));
        if self.selected != selected {
            self.viewer_scroll = 0;
        }
        self.selected = selected;
        if self.selected == self.session.as_ref().and_then(Session::latest_active) {
            self.new_turns = 0;
        }
    }

    fn move_turn(&mut self, down: bool) {
        let count = self.session.as_ref().map_or(0, |s| s.turns.len());
        if count == 0 {
            return;
        }
        let next = match self.selected {
            Some(current) if down => (current + 1).min(count - 1),
            Some(current) => current.saturating_sub(1),
            None if down => 0,
            None => count - 1,
        };
        self.select(Some(next));
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.kind == KeyEventKind::Release {
            return Action::None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }
        if self.help {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('q' | '?') | KeyCode::Enter
            ) {
                self.help = false;
            }
            return Action::None;
        }
        if self.searching {
            match key.code {
                KeyCode::Esc => {
                    self.searching = false;
                    self.query.clear();
                    if !self.picker {
                        let original = self.search_original.filter(|&index| {
                            self.session
                                .as_ref()
                                .is_some_and(|session| index < session.turns.len())
                        });
                        self.select(
                            original
                                .or(self.selected)
                                .or_else(|| self.session.as_ref().and_then(Session::latest_active)),
                        );
                    }
                    self.refresh_results();
                }
                KeyCode::Enter => {
                    if self.picker {
                        if let Some(&i) = self.results.get(self.picker_selected) {
                            return Action::OpenSession(self.summaries[i].path.clone());
                        }
                    } else if let Some(&i) = self.results.get(self.search_cursor) {
                        self.select(Some(i));
                        self.searching = false;
                        self.query.clear();
                        self.refresh_results();
                        self.focus = Focus::Viewer;
                    }
                }
                KeyCode::Up => self.move_result(false),
                KeyCode::Down => self.move_result(true),
                KeyCode::Backspace => {
                    use unicode_segmentation::UnicodeSegmentation;
                    if let Some((index, _)) = self.query.grapheme_indices(true).next_back() {
                        self.query.truncate(index);
                    }
                    self.search_cursor = 0;
                    self.picker_selected = 0;
                    self.refresh_results();
                }
                KeyCode::Char(ch)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    if self.query.len() < 4096 {
                        self.query.push(ch);
                    }
                    self.search_cursor = 0;
                    self.picker_selected = 0;
                    self.refresh_results();
                }
                _ => {}
            }
            return Action::None;
        }
        if self.picker {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Action::Quit,
                KeyCode::Char('j') | KeyCode::Down => self.move_result(true),
                KeyCode::Char('k') | KeyCode::Up => self.move_result(false),
                KeyCode::Enter => {
                    if let Some(&i) = self.results.get(self.picker_selected) {
                        return Action::OpenSession(self.summaries[i].path.clone());
                    }
                }
                KeyCode::Char('/') => self.start_search(),
                KeyCode::Char('r') => return Action::ShowPicker,
                KeyCode::Char('?') => self.help = true,
                _ => {}
            }
            return Action::None;
        }
        match key.code {
            KeyCode::Char('q') => return Action::Quit,
            KeyCode::Char('r') => return Action::Refresh,
            KeyCode::Char('s') => return Action::ShowPicker,
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('/') => self.start_search(),
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = if self.focus == Focus::Timeline {
                    Focus::Viewer
                } else {
                    Focus::Timeline
                }
            }
            KeyCode::Enter => self.focus = Focus::Viewer,
            KeyCode::Esc => self.focus = Focus::Timeline,
            KeyCode::Char('[') => self.move_turn(false),
            KeyCode::Char(']') => self.move_turn(true),
            KeyCode::Char('j') | KeyCode::Down => {
                if self.focus == Focus::Viewer {
                    self.viewer_scroll = self.viewer_scroll.saturating_add(1);
                } else {
                    self.move_turn(true);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.focus == Focus::Viewer {
                    self.viewer_scroll = self.viewer_scroll.saturating_sub(1);
                } else {
                    self.move_turn(false);
                }
            }
            KeyCode::PageDown => {
                self.viewer_scroll = self
                    .viewer_scroll
                    .saturating_add(self.viewer_height.saturating_sub(1).max(1))
            }
            KeyCode::PageUp => {
                self.viewer_scroll = self
                    .viewer_scroll
                    .saturating_sub(self.viewer_height.saturating_sub(1).max(1))
            }
            KeyCode::Home => self.viewer_scroll = 0,
            KeyCode::End => self.viewer_scroll = usize::MAX,
            KeyCode::Char('g') => {
                if self.session.as_ref().is_some_and(|s| !s.turns.is_empty()) {
                    self.select(Some(0));
                }
            }
            KeyCode::Char('G') => {
                self.select(self.session.as_ref().and_then(Session::latest_active))
            }
            KeyCode::Char('c') => {
                if let Some(turn) = self.selected_turn() {
                    return Action::Copy(sanitize(&turn.prompt.text));
                }
            }
            KeyCode::Char('C') => {
                if let Some(turn) = self.selected_turn() {
                    return Action::Copy(turn_text(turn));
                }
            }
            _ => {}
        }
        Action::None
    }

    fn move_result(&mut self, down: bool) {
        let cursor = if self.picker {
            &mut self.picker_selected
        } else {
            &mut self.search_cursor
        };
        *cursor = if down {
            cursor
                .saturating_add(1)
                .min(self.results.len().saturating_sub(1))
        } else {
            cursor.saturating_sub(1)
        };
    }

    fn start_search(&mut self) {
        self.searching = true;
        self.query.clear();
        self.search_original = self.selected;
        self.search_cursor = 0;
        self.picker_selected = 0;
        self.focus = Focus::Timeline;
        self.refresh_results();
    }

    pub fn viewer_lines(&mut self, width: usize, height: usize) -> Vec<String> {
        let Some(index) = self.selected else {
            if self
                .session
                .as_ref()
                .is_some_and(|session| !session.turns.is_empty())
            {
                return vec![
                    "No active turn. Use j/k in the timeline to inspect rolled-back history."
                        .into(),
                ];
            }
            return vec!["No turns yet. Waiting for a prompt…".into()];
        };
        let Some(turn) = self.selected_turn() else {
            return Vec::new();
        };
        let key = (index, turn.revision, width);
        if self
            .viewer_cache
            .as_ref()
            .is_none_or(|(i, r, w, _)| (*i, *r, *w) != key)
        {
            self.viewer_cache = Some((index, turn.revision, width, wrap(&turn_text(turn), width)));
        }
        self.viewer_height = height;
        let lines = &self.viewer_cache.as_ref().expect("cache populated").3;
        self.viewer_scroll = self.viewer_scroll.min(lines.len().saturating_sub(height));
        lines
            .iter()
            .skip(self.viewer_scroll)
            .take(height)
            .cloned()
            .collect()
    }
}

pub fn turn_text(turn: &Turn) -> String {
    let mut out = format!("USER\n{}\n", sanitize(&turn.prompt.text));
    if turn.prompt.images_count > 0 {
        out.push_str(&format!("[{} image(s)]\n", turn.prompt.images_count));
    }
    for item in &turn.items {
        let (heading, text) = match item {
            TurnItem::AgentMessage { text, .. } => ("AGENT".to_owned(), text.as_str()),
            TurnItem::ToolCall { name, summary } => {
                (format!("ACTIVITY · {}", sanitize(name)), summary.as_str())
            }
            TurnItem::ToolOutput { summary, is_error } => (
                if *is_error { "ERROR" } else { "OUTPUT" }.to_owned(),
                summary.as_str(),
            ),
            TurnItem::FileActivity { path, kind } => {
                (format!("FILE · {}", sanitize(kind)), path.as_str())
            }
            TurnItem::Notice { text } => ("NOTICE".to_owned(), text.as_str()),
        };
        out.push_str(&format!("\n{heading}\n{}\n", sanitize(text)));
    }
    out.push_str(&format!(
        "\nRESULT · {}\n{} commands · {} tools · {} files read · {} files changed · {} errors",
        turn.status.label(),
        turn.activity.commands,
        turn.activity.tool_calls,
        turn.activity.files_read,
        turn.activity.files_changed,
        turn.activity.errors
    ));
    if let Some(start) = turn.started_at {
        out.push_str(&format!(
            "\nStarted {}",
            start.format("%Y-%m-%d %H:%M:%S UTC")
        ));
    }
    if let Some(end) = turn.completed_at {
        out.push_str(&format!(
            "\nCompleted {}",
            end.format("%Y-%m-%d %H:%M:%S UTC")
        ));
    }
    out
}
