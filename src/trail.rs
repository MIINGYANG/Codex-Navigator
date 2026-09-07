//! Deterministic question structure and bounded, prompt-only global search.
use crate::{config::Config, domain::Session, parser::Tail, util::preview};
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf, sync::mpsc, thread};

pub(super) fn node_id(index: usize) -> String {
    format!("q{}", index + 1)
}

pub(super) fn title(text: &str) -> String {
    let first = text
        .lines()
        .map(str::trim)
        .skip_while(|line| line.is_empty())
        .take_while(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    preview(&first, 100)
}

pub(super) fn graph(session: &Session) -> (Vec<Value>, Vec<Value>, Vec<String>) {
    let mut ids = HashMap::new();
    for (index, turn) in session.turns.iter().enumerate() {
        if let Some(id) = &turn.id {
            // Ambiguous duplicate IDs cannot establish relationships.
            ids.entry(id.as_str())
                .and_modify(|v| *v = None)
                .or_insert(Some(index));
        }
    }
    let mut edges = Vec::new();
    let mut nodes = Vec::new();
    let mut notices = Vec::new();
    let latest = session.latest_active();
    if session.meta.identity.has_fork_lineage {
        notices.push(
            "此会话记录了跨会话 fork 来源，但没有可验证的父问题锚点；不会推测跨会话连线。".into(),
        );
    }
    for (index, turn) in session.turns.iter().enumerate() {
        let explicit = turn.parent_turn_id.as_deref();
        // Backward-only links form a DAG and exclude self, cycles and forward references.
        let parent = explicit
            .and_then(|id| ids.get(id).copied().flatten())
            .filter(|&p| p < index);
        if explicit.is_some() && parent.is_none() {
            notices.push(format!(
                "Q{} 的 parent_turn_id 无法唯一对应之前的问题，已退化为顺序关系。",
                index + 1
            ));
        }
        let parent = parent.or_else(|| index.checked_sub(1));
        let id = node_id(index);
        let prompt = if turn.prompt.text.is_empty() {
            &turn.prompt.preview
        } else {
            &turn.prompt.text
        };
        nodes.push(json!({"id":id,"turnIndex":index,"ordinal":turn.ordinal,"title":title(prompt),"promptPreview":preview(prompt,320),"timestamp":turn.started_at.map(|t|t.to_rfc3339()),"parentId":parent.map(node_id),"isLatest":Some(index)==latest}));
        if let Some(parent) = parent {
            edges.push(json!({"id":format!("{}-{id}",node_id(parent)),"source":node_id(parent),"target":id,"type":if parent + 1 == index {"sequence"}else{"branch"}}));
        }
    }
    if session.parse_stats.omitted_text_bytes > 0 {
        notices
            .push("部分超大或较早的正文超出内存保留预算；节点预览仍可用，详情会标明省略。".into());
    }
    (nodes, edges, notices)
}

pub(super) struct SearchRow {
    pub key: String,
    pub session_title: String,
    pub index: usize,
    pub title: String,
    pub preview: String,
    pub searchable: String,
}

#[derive(Default)]
pub(super) struct SearchSnapshot {
    pub rows: Vec<SearchRow>,
    pub counts: HashMap<String, usize>,
    pub truncated: bool,
}

pub(super) fn start_index(
    paths: Vec<(String, PathBuf)>,
    config: Config,
    mut retained: usize,
    mut rows: usize,
) -> mpsc::Receiver<(SearchSnapshot, bool)> {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        for (key, path) in paths {
            let mut snapshot = SearchSnapshot::default();
            snapshot.counts.insert(key.clone(), 0);
            if !path.canonicalize().is_ok_and(|current| current == path) {
                snapshot.truncated = true;
                if sender.send((snapshot, false)).is_err() {
                    return;
                }
                continue;
            }
            let Ok(mut tail) = Tail::open(&path, config.max_record_bytes, 100) else {
                snapshot.truncated = true;
                if sender.send((snapshot, false)).is_err() {
                    return;
                }
                continue;
            };
            // Freeze the initial length: a busy writer cannot keep this scan alive forever.
            let size = tail.reader.len().unwrap_or(0);
            let limit = size.min(1024 * 1024 * 1024);
            snapshot.truncated |= size > limit;
            while tail.reader.byte_offset < limit {
                match tail.poll(512 * 1024) {
                    Ok((0, _)) | Err(_) => break,
                    _ => {}
                }
            }
            let session = tail.session();
            let projected = project(session, &key, &mut retained, &mut rows);
            snapshot.rows = projected.rows;
            snapshot.counts = projected.counts;
            snapshot.truncated |= projected.truncated;
            let stop = projected.truncated || rows >= 100_000;
            snapshot.truncated |= stop;
            if sender.send((snapshot, false)).is_err() {
                return;
            }
            if stop {
                break;
            }
        }
        let _ = sender.send((SearchSnapshot::default(), true));
    });
    receiver
}

/// Projection can reuse an already loaded session without any further filesystem reads.
pub(super) fn project(
    session: &Session,
    key: &str,
    retained: &mut usize,
    rows: &mut usize,
) -> SearchSnapshot {
    let mut snapshot = SearchSnapshot::default();
    snapshot.counts.insert(key.to_owned(), session.turns.len());
    let session_title = session
        .turns
        .iter()
        .map(|turn| {
            if turn.prompt.text.is_empty() {
                &turn.prompt.preview
            } else {
                &turn.prompt.text
            }
        })
        .find(|text| !text.trim().is_empty())
        .map(|text| title(text))
        .unwrap_or_default();
    for (index, turn) in session.turns.iter().enumerate().rev() {
        let text = if turn.prompt.text.is_empty() {
            &turn.prompt.preview
        } else {
            &turn.prompt.text
        };
        let searchable = format!("{} {}", session_title, text).to_lowercase();
        if retained.saturating_add(searchable.len()) > 32 * 1024 * 1024 || *rows >= 100_000 {
            snapshot.truncated = true;
            break;
        }
        *retained += searchable.len();
        *rows += 1;
        snapshot.rows.push(SearchRow {
            key: key.to_owned(),
            session_title: session_title.clone(),
            index,
            title: title(text),
            preview: preview(text, 200),
            searchable,
        });
    }
    snapshot
}

pub(super) fn search(snapshot: &SearchSnapshot, query: &str) -> Vec<Value> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    snapshot.rows.iter().filter(|row| {
        query.split_whitespace().all(|term| {
            if row.searchable.contains(term) { return true; }
            // Conservative subsequence fallback, bounded to a short preview.
            let mut chars = term.chars();
            let mut wanted = chars.next();
            for c in row.title.to_lowercase().chars() {
                if wanted == Some(c) { wanted = chars.next(); }
                if wanted.is_none() { return true; }
            }
            false
        })
    }).take(100).map(|row| json!({"sessionKey":row.key,"sessionTitle":row.session_title,"nodeId":node_id(row.index),"turnIndex":row.index,"ordinal":row.index+1,"title":row.title,"preview":row.preview})).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::{Turn, UserPrompt},
        parser::{jsonl_reader::Record, Parser},
    };

    fn session(parents: &[Option<&str>]) -> Session {
        Session {
            turns: parents
                .iter()
                .enumerate()
                .map(|(i, parent)| Turn {
                    id: Some(format!("t{i}")),
                    ordinal: i + 1,
                    parent_turn_id: parent.map(str::to_owned),
                    prompt: UserPrompt {
                        text: format!("Question {}", i + 1),
                        ..UserPrompt::default()
                    },
                    ..Turn::default()
                })
                .collect(),
            ..Session::default()
        }
    }
    #[test]
    fn graph_linear_stable_ids_ordinals_and_latest() {
        let (nodes, edges, notices) = graph(&session(&[None, None, None]));
        assert!(notices.is_empty());
        assert_eq!(nodes[0]["id"], "q1");
        assert_eq!(nodes[2]["ordinal"], 3);
        assert_eq!(nodes[2]["isLatest"], true);
        assert_eq!(nodes[0]["isLatest"], false);
        assert_eq!(edges[1]["source"], "q2");
        assert_eq!(edges[1]["type"], "sequence");
    }
    #[test]
    fn graph_explicit_branch_excludes_invented_sequence_link() {
        let (_, edges, notices) = graph(&session(&[None, None, Some("t0")]));
        assert!(notices.is_empty());
        assert_eq!(edges.len(), 2);
        assert_eq!(edges[1]["source"], "q1");
        assert_eq!(edges[1]["target"], "q3");
        assert_eq!(edges[1]["type"], "branch");
    }
    #[test]
    fn graph_rejects_cycles_forward_missing_and_duplicate_parent_ids() {
        let mut s = session(&[Some("t2"), Some("t1"), Some("missing")]);
        let (_, edges, notices) = graph(&s);
        assert_eq!(notices.len(), 3);
        assert_eq!(edges[1]["source"], "q2");
        s.turns[1].id = Some("t0".into());
        s.turns[2].parent_turn_id = Some("t0".into());
        assert_eq!(graph(&s).1[1]["source"], "q2");
    }
    #[test]
    fn branch_fixture_retains_explicit_metadata_and_skips_reasoning() {
        let mut p = Parser::new(100);
        for line in include_str!("../tests/fixtures/trail_branch.jsonl").lines() {
            p.consume(Record::Line(line.as_bytes()));
        }
        let (nodes, edges, notices) = graph(&p.session);
        assert_eq!(nodes.len(), 4);
        assert_eq!(edges[2]["source"], "q1");
        assert_eq!(edges[2]["type"], "branch");
        assert!(notices.is_empty());
        assert!(!serde_json::to_string(&nodes)
            .unwrap()
            .contains("PRIVATE_REASONING"));
        assert_eq!(p.session.turns[3].parent_turn_id.as_deref(), Some("start"));
    }
    #[test]
    fn deterministic_title_preserves_words_and_first_paragraph() {
        assert_eq!(
            title("  Why   this?\ncontinued\n\nSecond paragraph"),
            "Why this? continued"
        );
        assert_eq!(
            title("\r\nFirst paragraph\r\n  \r\nSecond paragraph"),
            "First paragraph"
        );
    }
}
