use crate::domain::Turn;

#[derive(Default)]
pub struct SearchIndex {
    entries: Vec<(u64, String)>,
}

impl SearchIndex {
    pub fn update(&mut self, index: usize, turn: &Turn) {
        if index >= self.entries.len() {
            self.entries.resize(index + 1, (u64::MAX, String::new()));
        }
        self.entries[index] = (turn.revision, prompt_text(turn).to_lowercase());
    }

    pub fn sync(&mut self, turns: &[Turn]) {
        self.entries.truncate(turns.len());
        for (i, turn) in turns.iter().enumerate() {
            if i == self.entries.len() {
                self.entries
                    .push((turn.revision, prompt_text(turn).to_lowercase()));
            } else if self.entries[i].0 != turn.revision {
                self.entries[i] = (turn.revision, prompt_text(turn).to_lowercase());
            }
        }
    }

    pub fn search(&self, query: &str) -> Vec<usize> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return (0..self.entries.len()).collect();
        }
        let mut matches: Vec<_> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, (_, text))| score(text, &query).map(|score| (index, score)))
            .collect();
        matches.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
        matches.into_iter().map(|(index, _)| index).collect()
    }
}

pub fn prompt_text(turn: &Turn) -> &str {
    if turn.prompt.text.is_empty() && turn.prompt.omitted_bytes > 0 {
        &turn.prompt.preview
    } else {
        &turn.prompt.text
    }
}

/// Substrings rank ahead of a character-subsequence fuzzy match. Fuzzy scores
/// prefer adjacent characters; equal scores choose newer turns at the caller.
pub fn score(text: &str, query: &str) -> Option<i64> {
    if text.contains(query) {
        return Some(1_000_000 + i64::from(text.starts_with(query)));
    }
    let mut wanted = query.chars();
    let mut next = wanted.next()?;
    let mut first = None;
    let mut matched = 0_i64;
    for (position, ch) in text.chars().enumerate() {
        if ch == next {
            first.get_or_insert(position);
            matched += 1;
            match wanted.next() {
                Some(ch) => next = ch,
                None => return Some(matched * 20 - (position - first.unwrap_or(position)) as i64),
            }
        }
    }
    None
}
