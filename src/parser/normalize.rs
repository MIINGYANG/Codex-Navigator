use super::{activity, jsonl_reader::Record};
use crate::{
    domain::*,
    util::{preview, sanitize},
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    hash::{DefaultHasher, Hash, Hasher},
};

const PROMPT_TEXT_BYTES: usize = 16 * 1024 * 1024;
const ACTIVITY_TEXT_BYTES: usize = 48 * 1024 * 1024;
const MAX_ITEM_BYTES: usize = 64 * 1024;
const MAX_PROMPT_BYTES: usize = 256 * 1024;
const MAX_TURNS: usize = 100_000;
const MAX_ITEMS: usize = 200_000;

pub struct Parser {
    pub session: Session,
    pub preview_width: usize,
    pub dirty: BTreeSet<usize>,
    active: Option<usize>,
    pending_id: Option<String>,
    pending_parent: Option<String>,
    pending_time: Option<DateTime<Utc>>,
    turn_ids: HashMap<String, usize>,
    seq: usize,
    generation: usize,
    user_seen: VecDeque<(String, &'static str, usize, usize, usize)>,
    agent_seen: VecDeque<(u64, &'static str, usize, usize, usize, String)>,
    item_seen: VecDeque<(String, String, usize)>,
    prompt_retained: usize,
    activity_retained: usize,
    prompt_order: VecDeque<usize>,
    activity_order: VecDeque<(usize, usize)>,
    saw_meta: bool,
    items: usize,
}

impl Parser {
    pub fn new(preview_width: usize) -> Self {
        Self {
            session: Session::default(),
            preview_width,
            dirty: BTreeSet::new(),
            active: None,
            pending_id: None,
            pending_parent: None,
            pending_time: None,
            turn_ids: HashMap::new(),
            seq: 0,
            generation: 0,
            user_seen: VecDeque::new(),
            agent_seen: VecDeque::new(),
            item_seen: VecDeque::new(),
            prompt_retained: 0,
            activity_retained: 0,
            prompt_order: VecDeque::new(),
            activity_order: VecDeque::new(),
            saw_meta: false,
            items: 0,
        }
    }
    pub fn consume(&mut self, record: Record<'_>) {
        self.seq += 1;
        self.session.parse_stats.records += 1;
        self.session.revision += 1;
        match record {
            Record::Oversized => self.session.parse_stats.skipped_oversize_records += 1,
            Record::Line(bytes) => {
                let bytes = if self.seq == 1 {
                    bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes)
                } else {
                    bytes
                };
                if bytes.iter().all(u8::is_ascii_whitespace) {
                    return;
                }
                match serde_json::from_slice::<Value>(bytes) {
                    Ok(v) if v.is_object() => self.record(&v),
                    _ => self.session.parse_stats.malformed_records += 1,
                }
            }
        }
    }
    fn touch(&mut self, i: usize) {
        self.session.turns[i].revision += 1;
        self.dirty.insert(i);
    }
    fn limited(&mut self, text: String, limit: usize) -> String {
        let mut n = text.len().min(limit);
        while !text.is_char_boundary(n) {
            n -= 1;
        }
        self.session.parse_stats.omitted_text_bytes += text.len() - n;
        if n < text.len() && n > 0 {
            format!("{}\n[display text truncated]", &text[..n])
        } else {
            text[..n].to_owned()
        }
    }
    fn reserve_activity(&mut self, bytes: usize, keep: Option<(usize, usize)>) {
        while self.activity_retained + bytes > ACTIVITY_TEXT_BYTES {
            let Some((i, j)) = self.activity_order.pop_front() else {
                break;
            };
            if keep == Some((i, j)) {
                self.activity_order.push_back((i, j));
                continue;
            }
            let old = std::mem::replace(&mut self.session.turns[i].items[j], TurnItem::Omitted);
            let removed = old.retained_bytes();
            self.activity_retained -= removed;
            self.session.parse_stats.omitted_text_bytes += removed;
            self.touch(i);
        }
    }
    fn promote_final(&mut self, i: usize, j: usize) -> bool {
        let TurnItem::AgentMessage { phase, .. } = &self.session.turns[i].items[j] else {
            return false;
        };
        let old = phase.as_ref().map_or(0, String::len);
        self.reserve_activity("final_answer".len().saturating_sub(old), Some((i, j)));
        if let TurnItem::AgentMessage { phase, .. } = &mut self.session.turns[i].items[j] {
            *phase = Some("final_answer".into());
        }
        self.activity_retained = self.activity_retained - old + "final_answer".len();
        self.touch(i);
        true
    }
    fn record(&mut self, v: &Value) {
        let kind = s(v, "type");
        let p = v.get("payload").unwrap_or(v);
        let timestamp = time(v.get("timestamp"));
        self.session.meta.updated_at = self.session.meta.updated_at.max(timestamp);
        match kind {
            "session_meta" => {
                // Fork histories may contain their parent's metadata: first identity wins.
                if !self.saw_meta {
                    self.saw_meta = true;
                    self.session.meta.identity = super::identity::parse_identity(p);
                    self.session.meta.id = s(p, "id").to_owned();
                    if self.session.meta.id.is_empty() {
                        self.session.meta.id = s(p, "session_id").to_owned();
                    }
                    self.session.meta.cwd = p.get("cwd").and_then(Value::as_str).map(Into::into);
                    self.session.meta.created_at = time(p.get("timestamp")).or(timestamp);
                }
            }
            "event_msg" => self.event(p, timestamp),
            "response_item" => self.response(p, timestamp),
            "turn_context" => {
                if let Some(id) = p.get("turn_id").and_then(Value::as_str) {
                    if self.pending_id.as_deref() != Some(id) {
                        self.start(Some(id), timestamp);
                    }
                }
            }
            "turn_started" | "task_started" | "turn_complete" | "task_complete"
            | "turn_completed" | "turn_aborted" | "turn_error" | "error" | "thread_rollback"
            | "thread_rolled_back" => self.event(v, timestamp),
            // Compaction replacement_history is context, never a new conversation.
            "compacted" | "world_state" | "token_usage_record" => (),
            _ => self.session.parse_stats.unknown_records += 1,
        }
        // Accept only an explicit relationship attached to a turn-bearing record.
        // root_turn_id and fork record ordinals deliberately do not enter this model.
        let turn_record = kind == "turn_context"
            || matches!(
                s(p, "type"),
                "task_started" | "turn_started" | "user_message"
            )
            || (kind == "response_item" && s(p, "role") == "user");
        if turn_record {
            if let Some(parent) = p
                .get("parent_turn_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty() && id.len() <= 256)
            {
                if matches!(kind, "turn_context" | "task_started" | "turn_started")
                    || matches!(s(p, "type"), "task_started" | "turn_started")
                {
                    self.pending_parent = Some(parent.to_owned());
                }
                if let Some(i) = self.active {
                    self.session.turns[i].parent_turn_id = Some(parent.to_owned());
                    self.touch(i);
                }
            }
        }
    }
    fn start(&mut self, id: Option<&str>, timestamp: Option<DateTime<Utc>>) {
        let id = id.filter(|id| !id.is_empty() && id.len() <= 256);
        if id.is_some() && self.pending_id.as_deref() == id {
            return;
        }
        self.pending_id = id.map(str::to_owned);
        self.pending_parent = None;
        self.pending_time = timestamp;
        self.active = id.and_then(|id| self.turn_ids.get(id).copied());
    }
    fn event(&mut self, p: &Value, timestamp: Option<DateTime<Utc>>) {
        match s(p, "type") {
            "task_started" | "turn_started" => self.start(
                p.get("turn_id").and_then(Value::as_str),
                time(p.get("started_at")).or(timestamp),
            ),
            "task_complete" | "turn_complete" | "turn_completed" => {
                self.complete(p, timestamp, TurnStatus::Completed)
            }
            "turn_aborted" => self.complete(p, timestamp, TurnStatus::Interrupted),
            "turn_error" | "error" => self.complete(p, timestamp, TurnStatus::Failed),
            "user_message" => {
                let text = activity::text(
                    p.get("message")
                        .or_else(|| p.get("text"))
                        .unwrap_or(&Value::Null),
                );
                let images = p
                    .get("images")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len)
                    + p.get("local_images")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len);
                self.user(
                    text,
                    images,
                    "event",
                    timestamp,
                    p.get("turn_id").and_then(Value::as_str),
                );
            }
            "agent_message" => self.agent(
                activity::text(
                    p.get("message")
                        .or_else(|| p.get("text"))
                        .unwrap_or(&Value::Null),
                ),
                p.get("phase").and_then(Value::as_str),
                "event",
                s(p, "id"),
            ),
            "item_completed" => {
                if let Some(item) = p.get("item") {
                    let id = p.get("turn_id").and_then(Value::as_str);
                    if matches!(s(item, "type"), "UserMessage" | "user_message") {
                        self.item(item, id, timestamp);
                    } else {
                        let previous = self.active;
                        if let Some(id) = id {
                            self.active = self.turn_ids.get(id).copied();
                        }
                        self.item(item, id, timestamp);
                        self.active = previous;
                    }
                }
            }
            "thread_rollback" | "thread_rolled_back" | "rollback" => self.rollback(p),
            "exec_command_end" => self.command(p),
            "token_count" | "agent_reasoning" | "agent_reasoning_raw_content" => (),
            _ => self.session.parse_stats.unknown_records += 1,
        }
    }
    fn response(&mut self, p: &Value, timestamp: Option<DateTime<Utc>>) {
        match s(p, "type") {
            "message" if s(p, "role") == "user" => {
                let meta = &p["internal_chat_message_metadata_passthrough"];
                let kinds = meta.get("content_item_kinds").and_then(Value::as_array);
                let mut parts = Vec::new();
                let mut images = 0;
                if let Some(content) = p.get("content").and_then(Value::as_array) {
                    for (i, c) in content.iter().enumerate() {
                        if let Some(kinds) = kinds {
                            if let Some(kind) = kinds.get(i).and_then(Value::as_str) {
                                if !kind.starts_with("user.") {
                                    continue;
                                }
                            }
                        }
                        match s(c, "type") {
                            "input_text" | "text" => {
                                let t = activity::text(c);
                                if image_wrapper(content, i, &t) {
                                    continue;
                                }
                                if kinds.is_some() || !internal_context(&t) {
                                    parts.push(t);
                                }
                            }
                            "input_image" | "image" => images += 1,
                            _ => (),
                        }
                    }
                } else if let Some(content) = p.get("content") {
                    let t = activity::text(content);
                    if !internal_context(&t) {
                        parts.push(t);
                    }
                }
                self.user(
                    parts.join("\n"),
                    images,
                    "response",
                    timestamp,
                    meta.get("turn_id")
                        .or_else(|| p.get("turn_id"))
                        .and_then(Value::as_str),
                );
            }
            "message" if s(p, "role") == "assistant" => {
                if matches!(s(p, "channel"), "analysis" | "reasoning")
                    || matches!(s(p, "phase"), "analysis" | "reasoning")
                {
                    return;
                }
                self.agent(
                    activity::text(&p["content"]),
                    p.get("phase").and_then(Value::as_str),
                    "response",
                    s(p, "id"),
                );
            }
            "function_call" | "custom_tool_call" => {
                let name = s(p, "name");
                let id = s(p, "call_id");
                if self.seen_item(id, "call") {
                    return;
                }
                let summary = activity::call_summary(
                    name,
                    p.get("arguments")
                        .or_else(|| p.get("input"))
                        .unwrap_or(&Value::Null),
                );
                self.add(TurnItem::ToolCall {
                    name: sanitize(name),
                    summary,
                });
            }
            "function_call_output" | "custom_tool_call_output" => {
                if self.seen_item(s(p, "call_id"), "output") {
                    return;
                }
                let (summary, is_error) = activity::output(&p["output"]);
                self.add(TurnItem::ToolOutput {
                    summary,
                    is_error: is_error || activity::error(p),
                });
            }
            "reasoning" | "message" => (),
            _ => self.session.parse_stats.unknown_records += 1,
        }
    }
    fn user(
        &mut self,
        text: String,
        images: usize,
        source: &'static str,
        timestamp: Option<DateTime<Utc>>,
        id: Option<&str>,
    ) {
        let text = text.trim().to_owned();
        if text.is_empty() && images == 0 {
            return;
        }
        if id.is_some() && id != self.pending_id.as_deref() {
            self.start(id, timestamp);
        }
        let key = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if let Some(i) = self.active {
            if self.user_seen.iter().any(|(k, src, seq, gen, idx)| {
                *idx == i
                    && k == &key
                    && *src != source
                    && self.seq.saturating_sub(*seq) <= 16
                    && (*gen == self.generation || self.pending_id.is_some())
            }) {
                self.session.turns[i].prompt.images_count =
                    self.session.turns[i].prompt.images_count.max(images);
                self.touch(i);
                return;
            }
        }
        let merge = self.pending_id.is_some()
            && self
                .active
                .is_some_and(|i| self.session.turns[i].status != TurnStatus::RolledBack);
        if !merge {
            if self.session.turns.len() >= MAX_TURNS {
                self.session.parse_stats.omitted_text_bytes += text.len();
                self.active = None;
                return;
            }
            let i = self.session.turns.len();
            self.session.turns.push(Turn {
                ordinal: i + 1,
                id: self.pending_id.clone(),
                parent_turn_id: self.pending_parent.take(),
                started_at: self.pending_time.or(timestamp),
                status: TurnStatus::InProgress,
                ..Turn::default()
            });
            self.active = Some(i);
            if let Some(id) = &self.pending_id {
                self.turn_ids.insert(id.clone(), i);
            }
        }
        let i = self.active.expect("created turn");
        let existing = self.session.turns[i].prompt.text.len();
        let separator = usize::from(existing > 0 && !text.is_empty());
        let mut n = text
            .len()
            .min(MAX_PROMPT_BYTES.saturating_sub(existing + separator));
        while !text.is_char_boundary(n) {
            n -= 1;
        }
        let added = n + if n > 0 { separator } else { 0 };
        while self.prompt_retained + added > PROMPT_TEXT_BYTES {
            let old = self
                .prompt_order
                .pop_front()
                .expect("prompt budget has retained bodies");
            if old == i {
                self.prompt_order.push_back(old);
                continue;
            }
            let prompt = &mut self.session.turns[old].prompt;
            let removed = prompt.text.len();
            prompt.text = String::new();
            prompt.omitted_bytes += removed;
            self.prompt_retained -= removed;
            self.session.parse_stats.omitted_text_bytes += removed;
            self.touch(old);
        }
        if existing == 0 && n > 0 {
            self.prompt_order.push_back(i);
        }
        self.prompt_retained += added;
        let omitted = text.len() - n;
        self.session.parse_stats.omitted_text_bytes += omitted;
        let retained = &text[..n];
        let turn = &mut self.session.turns[i];
        turn.prompt.omitted_bytes += omitted;
        if !turn.prompt.text.is_empty() && !retained.is_empty() {
            turn.prompt.text.push('\n');
        }
        turn.prompt.text.push_str(retained);
        turn.prompt.images_count += images;
        turn.prompt.preview = if turn.prompt.text.is_empty() && turn.prompt.omitted_bytes > 0 {
            turn.prompt.preview.clone()
        } else if turn.prompt.text.is_empty() {
            format!("[{} image(s)]", turn.prompt.images_count)
        } else {
            preview(&turn.prompt.text, self.preview_width)
        };
        self.user_seen
            .push_back((key, source, self.seq, self.generation, i));
        while self.user_seen.len() > 16 {
            self.user_seen.pop_front();
        }
        self.touch(i);
    }
    fn seen_item(&mut self, id: &str, kind: &str) -> bool {
        let i = self.active.unwrap_or(usize::MAX);
        if id.is_empty() || id.len() > 256 {
            return false;
        }
        if self
            .item_seen
            .iter()
            .any(|(a, b, c)| a == id && b == kind && *c == i)
        {
            return true;
        }
        self.item_seen
            .push_back((id.to_owned(), kind.to_owned(), i));
        while self.item_seen.len() > 1024 {
            self.item_seen.pop_front();
        }
        false
    }
    fn agent(&mut self, text: String, phase: Option<&str>, source: &'static str, id: &str) {
        if text.is_empty() || matches!(phase, Some("analysis" | "reasoning")) {
            return;
        }
        let i = self.active.unwrap_or(usize::MAX);
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let fingerprint = hasher.finish();
        let matching = self
            .agent_seen
            .iter()
            .rev()
            .find(|(t, src, seq, idx, _, old_id)| {
                *idx == i
                    && ((*t == fingerprint
                        && *src != source
                        && (source == "completion" || self.seq.saturating_sub(*seq) <= 16))
                        || (!id.is_empty() && old_id == id))
            })
            .map(|entry| entry.4);
        if let Some(j) = matching {
            if phase == Some("final_answer") {
                if self.promote_final(i, j) {
                    return;
                }
            } else if !matches!(self.session.turns[i].items[j], TurnItem::Omitted) {
                return;
            }
        }
        if self.seen_item(id, "agent") && matching.is_none() {
            return;
        }
        let Some(turn) = self.session.turns.get(i) else {
            return;
        };
        let j = turn.items.len();
        self.add(TurnItem::AgentMessage {
            text,
            phase: phase.map(|p| preview(p, 64)),
        });
        if self.session.turns[i].items.len() == j {
            return;
        }
        self.agent_seen.push_back((
            fingerprint,
            source,
            self.seq,
            i,
            j,
            if id.len() <= 256 {
                id.to_owned()
            } else {
                String::new()
            },
        ));
        while self.agent_seen.len() > 8 {
            self.agent_seen.pop_front();
        }
    }
    fn add(&mut self, item: TurnItem) {
        let Some(i) = self.active else {
            return;
        };
        if self.items >= MAX_ITEMS {
            self.session.parse_stats.omitted_text_bytes += 1;
            return;
        }
        self.generation += 1;
        let item = match item {
            TurnItem::AgentMessage { text, phase } => TurnItem::AgentMessage {
                text: self.limited(text, MAX_ITEM_BYTES),
                phase: phase.map(|phase| self.limited(phase, 64)),
            },
            TurnItem::ToolCall { name, summary } => {
                self.session.turns[i].activity.tool_calls += 1;
                if summary.starts_with("$ ") {
                    self.session.turns[i].activity.commands += 1;
                }
                TurnItem::ToolCall {
                    name: self.limited(name, 256),
                    summary: self.limited(summary, MAX_ITEM_BYTES),
                }
            }
            TurnItem::ToolOutput { summary, is_error } => {
                if is_error {
                    self.session.turns[i].activity.errors += 1;
                }
                TurnItem::ToolOutput {
                    summary: self.limited(summary, MAX_ITEM_BYTES),
                    is_error,
                }
            }
            TurnItem::FileActivity { path, kind } => {
                if kind == "read" {
                    self.session.turns[i].activity.files_read += 1;
                } else {
                    self.session.turns[i].activity.files_changed += 1;
                }
                TurnItem::FileActivity {
                    path: self.limited(path, 4096),
                    kind: self.limited(kind, 256),
                }
            }
            TurnItem::Notice { text } => TurnItem::Notice {
                text: self.limited(text, 4096),
            },
            TurnItem::Omitted => TurnItem::Omitted,
        };
        let bytes = item.retained_bytes();
        self.reserve_activity(bytes, None);
        self.activity_retained += bytes;
        self.activity_order
            .push_back((i, self.session.turns[i].items.len()));
        self.session.turns[i].items.push(item);
        self.items += 1;
        self.touch(i);
    }
    fn complete(&mut self, p: &Value, timestamp: Option<DateTime<Utc>>, outcome: TurnStatus) {
        let i = if let Some(id) = p.get("turn_id").and_then(Value::as_str) {
            self.turn_ids.get(id).copied()
        } else {
            self.active
        };
        let Some(i) = i else {
            return;
        };
        if self.session.turns[i].status == TurnStatus::RolledBack {
            return;
        }
        let old = self.active;
        self.active = Some(i);
        let outcome = if outcome == TurnStatus::Completed && activity::error(p) {
            TurnStatus::Failed
        } else {
            outcome
        };
        if let Some(text) = p
            .get("last_agent_message")
            .and_then(Value::as_str)
            .filter(|_| outcome == TurnStatus::Completed)
        {
            let text = sanitize(text);
            if let Some(j) = self.session.turns[i]
                .items
                .iter()
                .rposition(|item| matches!(item,TurnItem::AgentMessage {text:t,..} if t==&text))
            {
                self.promote_final(i, j);
            } else {
                self.agent(text, Some("final_answer"), "completion", "");
            }
        }
        let turn = &mut self.session.turns[i];
        turn.completed_at = time(p.get("completed_at")).or(timestamp);
        // Completion is a lifecycle fact, never a verdict on the answer's correctness.
        turn.status = outcome;
        // Completion is a submission boundary even if no assistant text was emitted.
        self.generation += 1;
        self.touch(i);
        self.active = old;
        if self.active == Some(i) {
            self.pending_id = None;
            self.pending_time = None;
        }
    }
    fn rollback(&mut self, p: &Value) {
        let ids = p.get("turn_ids").and_then(Value::as_array);
        let indices: Option<Vec<usize>> = if let Some(ids) = ids {
            ids.iter()
                .map(|id| id.as_str().and_then(|id| self.turn_ids.get(id).copied()))
                .collect()
        } else if let Some(n) = p
            .get("num_turns")
            .or_else(|| p.get("count"))
            .and_then(Value::as_u64)
        {
            let active: Vec<_> = self
                .session
                .turns
                .iter()
                .enumerate()
                .filter(|(_, t)| t.status != TurnStatus::RolledBack)
                .map(|(i, _)| i)
                .collect();
            if n <= active.len() as u64 {
                Some(active.into_iter().rev().take(n as usize).collect())
            } else {
                None
            }
        } else {
            None
        };
        if let Some(indices) = indices {
            for i in indices {
                self.session.turns[i].status = TurnStatus::RolledBack;
                self.touch(i);
            }
        } else {
            if let Some(i) = self.active {
                self.session.turns[i].status = TurnStatus::Unknown;
                self.touch(i);
            }
            self.add(TurnItem::Notice {
                text: "Rollback reported; affected turns could not be determined.".into(),
            });
        }
        self.active = None;
        self.pending_id = None;
        self.pending_time = None;
    }
    fn command(&mut self, p: &Value) {
        if self.seen_item(s(p, "id"), "command") {
            return;
        }
        self.add(TurnItem::ToolCall {
            name: "command".into(),
            summary: activity::call_summary("command", p),
        });
        if let Some(parsed) = p.get("parsed_cmd").and_then(Value::as_array) {
            for cmd in parsed {
                if s(cmd, "type") == "read" {
                    if let Some(path) = cmd.get("path").and_then(Value::as_str) {
                        self.add(TurnItem::FileActivity {
                            path: sanitize(path),
                            kind: "read".into(),
                        });
                    }
                }
            }
        }
        let out = p
            .get("aggregated_output")
            .or_else(|| p.get("formatted_output"));
        let summary = out.map(activity::text).unwrap_or_else(|| {
            [activity::text(&p["stdout"]), activity::text(&p["stderr"])].join("\n")
        });
        let summary = if let Some(code) = p.get("exit_code").and_then(Value::as_i64) {
            format!("exit code {code}\n{summary}")
        } else {
            summary
        };
        self.add(TurnItem::ToolOutput {
            summary,
            is_error: activity::error(p),
        });
    }
    fn item(&mut self, p: &Value, id: Option<&str>, timestamp: Option<DateTime<Utc>>) {
        match s(p, "type") {
            "UserMessage" | "user_message" => {
                let images = p.get("content").and_then(Value::as_array).map_or(0, |v| {
                    v.iter()
                        .filter(|c| {
                            matches!(
                                s(c, "type"),
                                "image" | "localImage" | "local_image" | "input_image"
                            )
                        })
                        .count()
                });
                self.user(activity::text(&p["content"]), images, "item", timestamp, id);
            }
            "AgentMessage" | "agent_message" => self.agent(
                activity::text(&p["content"]),
                p.get("phase").and_then(Value::as_str),
                "item",
                s(p, "id"),
            ),
            "CommandExecution" | "command_execution" => self.command(p),
            "FileChange" | "file_change" => {
                if self.seen_item(s(p, "id"), "files") {
                    return;
                }
                if let Some(changes) = p.get("changes").and_then(Value::as_object) {
                    for (path, change) in changes {
                        self.add(TurnItem::FileActivity {
                            path: sanitize(path),
                            kind: preview(s(change, "type"), 32),
                        });
                    }
                }
                if activity::error(p) {
                    self.add(TurnItem::ToolOutput {
                        summary: "File change failed".into(),
                        is_error: true,
                    });
                }
            }
            "ImageView" => {
                if let Some(path) = p.get("path").and_then(Value::as_str) {
                    self.add(TurnItem::FileActivity {
                        path: sanitize(path),
                        kind: "read".into(),
                    });
                }
            }
            "McpToolCall" | "DynamicToolCall" | "CollabAgentToolCall" => {
                if self.seen_item(s(p, "id"), "call") {
                    return;
                }
                let name = p
                    .get("tool")
                    .or_else(|| p.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("tool");
                self.add(TurnItem::ToolCall {
                    name: sanitize(name),
                    summary: format!("{} · {}", sanitize(name), sanitize(s(p, "status"))),
                });
                if activity::error(p) {
                    self.add(TurnItem::ToolOutput {
                        summary: "Tool reported failure".into(),
                        is_error: true,
                    });
                }
            }
            "Reasoning" | "reasoning" | "ContextCompaction" => (),
            _ => self.session.parse_stats.unknown_records += 1,
        }
    }
}

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
fn time(v: Option<&Value>) -> Option<DateTime<Utc>> {
    v.and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Utc))
}
fn internal_context(text: &str) -> bool {
    let t = text.trim();
    (t.starts_with("<environment_context>") && t.ends_with("</environment_context>"))
        || (t.starts_with("# AGENTS.md instructions")
            && t.contains("<INSTRUCTIONS>")
            && t.contains("</INSTRUCTIONS>"))
        || (t.starts_with("<permissions instructions>")
            && t.ends_with("</permissions instructions>"))
}

fn image_wrapper(content: &[Value], index: usize, text: &str) -> bool {
    let text = text.trim();
    let image_at = |i: usize| {
        content
            .get(i)
            .is_some_and(|v| s(v, "type") == "input_image")
    };
    ((text == "<image>"
        || (text.starts_with("<image ") && text.ends_with('>') && !text.contains('\n')))
        && image_at(index + 1))
        || (text == "</image>" && index > 0 && image_at(index - 1))
}
