//! Bounded evidence extraction. No repository reads and no assistant-prose inference.
use crate::{
    domain::{Session, SessionEvent},
    util::sanitize,
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
};

const MAX_EVENTS: usize = 10_000;
const MAX_PENDING: usize = 256;
const MAX_COMMAND: usize = 16 * 1024;

#[derive(Clone)]
struct Command {
    turn: Option<usize>,
    repository: Option<String>,
    commit: bool,
    followup: bool,
    tag: Option<(String, String)>,
}

struct CompactionMirror {
    source: String,
    turn: Option<usize>,
    timestamp: Option<String>,
    sequence: usize,
    id: Option<String>,
}

#[derive(Default)]
pub(super) struct Events {
    calls: HashMap<String, Vec<Command>>,
    processes: HashMap<String, Command>,
    order: VecDeque<String>,
    seen: HashSet<String>,
    last_compaction: Option<CompactionMirror>,
    cwd: Option<String>,
}

impl Events {
    pub fn observe(&mut self, v: &Value, active: Option<usize>, seq: usize, session: &mut Session) {
        let outer = string(v, "type");
        let p = v.get("payload").unwrap_or(v);
        let kind = string(p, "type");
        let time = v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.to_rfc3339());
        let explicit_turn = p
            .get("turn_id")
            .or_else(|| p.get("turnId"))
            .and_then(Value::as_str);
        let turn = explicit_turn
            .map(|id| {
                session
                    .turns
                    .iter()
                    .position(|t| t.id.as_deref() == Some(id))
            })
            .unwrap_or(active);
        if outer == "turn_context" {
            if let Some(cwd) = field(p, "cwd", 4096) {
                self.cwd = Some(cwd);
            }
        }
        let cwd = self
            .cwd
            .as_deref()
            .or_else(|| session.meta.cwd.as_ref().and_then(|p| p.to_str()));
        let compact = if outer == "compacted" {
            Some((p, "compacted"))
        } else if matches!(
            kind,
            "context_compacted" | "ContextCompacted" | "context_compaction"
        ) {
            Some((p, kind))
        } else if kind == "item_completed"
            && matches!(
                string(&p["item"], "type"),
                "ContextCompaction" | "contextCompaction" | "context_compaction"
            )
        {
            Some((&p["item"], "item_completed"))
        } else {
            None
        };
        if let Some((item, source)) = compact {
            let turn = if explicit_turn.is_some() {
                turn
            } else {
                turn.or_else(|| session.latest_active())
            };
            let id = field(item, "id", 256).map(|id| format!("compaction:{id}"));
            if id.as_ref().is_some_and(|id| self.seen.contains(id)) {
                return;
            }
            let trigger = match item
                .get("trigger")
                .or_else(|| item.get("source"))
                .and_then(Value::as_str)
            {
                Some("auto" | "automatic") => "auto",
                Some("manual") => "manual",
                _ => "unknown",
            };
            // Persisted compacted and context_compacted may mirror one operation.
            if self.last_compaction.as_ref().is_some_and(|previous| {
                previous.source != source
                    && previous.turn == turn
                    && time.is_some()
                    && previous.timestamp == time
                    && (id.is_none() || previous.id.is_none())
                    && seq.saturating_sub(previous.sequence) <= 3
            }) {
                if let Some(previous) = &self.last_compaction {
                    let key = previous
                        .id
                        .clone()
                        .unwrap_or_else(|| format!("compaction:{}", previous.sequence));
                    if let Some(event) = session.events.iter_mut().find(|event| event.id == key) {
                        if event.trigger.as_deref() == Some("unknown") && trigger != "unknown" {
                            event.trigger = Some(trigger.into());
                            event.source = source.into();
                        }
                    }
                }
                if let Some(id) = id {
                    if self.seen.len() < 2 * MAX_EVENTS {
                        self.seen.insert(id);
                    }
                }
                return;
            }
            self.last_compaction = Some(CompactionMirror {
                source: source.to_owned(),
                turn,
                timestamp: time.clone(),
                sequence: seq,
                id: id.clone(),
            });
            let id = id.unwrap_or_else(|| format!("compaction:{seq}"));
            self.push(
                session,
                SessionEvent {
                    id,
                    kind: "compaction".into(),
                    turn_index: turn,
                    timestamp: time,
                    source: source.into(),
                    trigger: Some(trigger.into()),
                    ..SessionEvent::default()
                },
            );
            return;
        }
        if outer == "response_item" && matches!(kind, "function_call" | "custom_tool_call") {
            let id = string(p, "call_id");
            if id.is_empty() || id.len() > 256 {
                return;
            }
            let name = string(p, "name").rsplit('.').next().unwrap_or("");
            let args = p
                .get("arguments")
                .or_else(|| p.get("input"))
                .unwrap_or(&Value::Null);
            let parsed = args
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok());
            let args = parsed.as_ref().unwrap_or(args);
            let commands = match name {
                "exec_command" | "shell_command" => command(args, turn, cwd).into_iter().collect(),
                "write_stdin" => process_id(&args["session_id"])
                    .and_then(|id| self.processes.get(&id).cloned())
                    .into_iter()
                    .collect(),
                // Parse only literal JSON tool arguments. Dynamic JS is deliberately unknown.
                "exec" => args
                    .as_str()
                    .filter(|s| s.len() <= MAX_COMMAND)
                    .map(|script| nested_commands(script, turn, cwd, &self.processes))
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            if !commands.is_empty() {
                if !self.calls.contains_key(id) {
                    self.order.push_back(id.to_owned());
                }
                self.calls.insert(id.to_owned(), commands);
                while self.order.len() > MAX_PENDING {
                    if let Some(id) = self.order.pop_front() {
                        self.calls.remove(&id);
                    }
                }
            }
        } else if outer == "response_item"
            && matches!(kind, "function_call_output" | "custom_tool_call_output")
        {
            let Some(commands) = self.calls.get(string(p, "call_id")).cloned() else {
                return;
            };
            let outputs = outputs(&p["output"], 0);
            // Multiple nested calls cannot safely be paired by emission order (Promise.all).
            if commands.len() == 1 {
                for output in outputs {
                    self.result(&commands[0], &output, time.clone(), session);
                }
            }
        } else if kind == "exec_command_end" {
            if let Some(command) = command(p, turn, cwd) {
                self.result(&command, p, time, session);
            }
        } else if kind == "item_completed" {
            let item = &p["item"];
            if matches!(
                string(item, "type"),
                "CommandExecution" | "commandExecution" | "command_execution"
            ) {
                if let Some(command) = command(item, turn, cwd) {
                    self.result(&command, item, time, session);
                }
            }
        }
    }

    fn push(&mut self, session: &mut Session, event: SessionEvent) {
        if session.events.len() >= MAX_EVENTS {
            session.parse_stats.omitted_text_bytes += 1;
        } else if self.seen.insert(event.id.clone()) {
            session.events.push(event);
        }
    }

    fn result(
        &mut self,
        command: &Command,
        out: &Value,
        time: Option<String>,
        session: &mut Session,
    ) {
        if let Some(id) = process_id(&out["session_id"]) {
            if self.processes.len() >= MAX_PENDING && !self.processes.contains_key(&id) {
                if let Some(old) = self.processes.keys().next().cloned() {
                    self.processes.remove(&old);
                }
            }
            self.processes.insert(id, command.clone());
        }
        let exit = out
            .get("exit_code")
            .or_else(|| out.get("exitCode"))
            .or_else(|| out.pointer("/metadata/exit_code"))
            .and_then(Value::as_i64);
        let text = [
            "output",
            "stdout",
            "aggregated_output",
            "aggregatedOutput",
            "formatted_output",
        ]
        .iter()
        .find_map(|key| out.get(key).and_then(Value::as_str))
        .or_else(|| out.as_str())
        .unwrap_or("");
        let legacy_exit = text.lines().find_map(|line| {
            line.strip_prefix("Process exited with code ")
                .and_then(|n| n.trim().parse::<i64>().ok())
        });
        let exit = exit.or(legacy_exit);
        if exit.is_none() || (exit != Some(0) && !command.followup) {
            return;
        }
        if command.commit {
            for line in text.lines() {
                let Some((branch, hash, summary)) = commit_line(line) else {
                    continue;
                };
                let id = format!(
                    "commit:{}:{hash}",
                    command.repository.as_deref().unwrap_or("?")
                );
                self.push(
                    session,
                    SessionEvent {
                        id,
                        kind: "commit".into(),
                        turn_index: command.turn,
                        timestamp: time.clone(),
                        source: "git_commit_output".into(),
                        hash: Some(hash),
                        repository: command.repository.clone(),
                        branch,
                        summary: Some(summary),
                        ..SessionEvent::default()
                    },
                );
            }
        }
        if exit == Some(0) {
            if let Some((tag, hash)) = &command.tag {
                for event in &mut session.events {
                    if event.kind == "commit"
                        && event.repository == command.repository
                        && event
                            .hash
                            .as_ref()
                            .is_some_and(|h| h.starts_with(hash) || hash.starts_with(h))
                    {
                        event.version = Some(tag.clone());
                    }
                }
            }
        }
    }
}

fn string<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
fn field(v: &Value, key: &str, limit: usize) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= limit)
        .map(sanitize)
}
fn process_id(v: &Value) -> Option<String> {
    v.as_u64()
        .map(|n| n.to_string())
        .or_else(|| v.as_str().filter(|s| s.len() <= 128).map(str::to_owned))
}
fn hash(s: &str) -> bool {
    (7..=64).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_hexdigit())
}
fn commit_line(line: &str) -> Option<(Option<String>, String, String)> {
    let line = line.trim();
    let (header, summary) = line.strip_prefix('[')?.split_once("] ")?;
    let (branch, sha) = header.rsplit_once(' ')?;
    if !hash(sha) {
        return None;
    }
    let branch = branch.strip_suffix(" (root-commit)").unwrap_or(branch);
    if branch.is_empty() || branch.len() > 256 || summary.len() > 1024 {
        return None;
    }
    Some((
        if branch == "detached HEAD" {
            None
        } else {
            Some(sanitize(branch))
        },
        sha.to_ascii_lowercase(),
        sanitize(summary),
    ))
}

/// Minimal literal shell lexer: no expansion, substitutions, redirection or pipelines.
fn words(command: &str) -> Option<Vec<String>> {
    let mut result = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escape = false;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        if escape {
            word.push(c);
            escape = false;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escape = true;
            continue;
        }
        if matches!(c, '$' | '`') && quote != Some('\'') {
            return None;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                word.push(c);
            }
        } else if matches!(c, '\'' | '"') {
            quote = Some(c);
        } else if c == '\n' {
            if !word.is_empty() {
                result.push(std::mem::take(&mut word));
            }
            result.push(";".into());
        } else if c.is_whitespace() {
            if !word.is_empty() {
                result.push(std::mem::take(&mut word));
            }
        } else if matches!(c, '&' | ';') {
            if !word.is_empty() {
                result.push(std::mem::take(&mut word));
            }
            if c == '&' {
                if chars.next() != Some('&') {
                    return None;
                }
                result.push("&&".into());
            } else {
                result.push(";".into());
            }
        } else if matches!(c, '<' | '>' | '(' | ')' | '|' | '{' | '}') {
            return None;
        } else {
            word.push(c);
        }
    }
    if quote.is_some() || escape {
        return None;
    }
    if !word.is_empty() {
        result.push(word);
    }
    Some(result)
}
fn resolve(base: Option<&str>, path: &str) -> Option<String> {
    let path = Path::new(path);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        PathBuf::from(base?).join(path)
    };
    if !absolute.is_absolute() {
        return None;
    }
    Some(absolute.to_string_lossy().into_owned())
}
fn command(args: &Value, turn: Option<usize>, cwd: Option<&str>) -> Option<Command> {
    let raw = args.get("cmd").or_else(|| args.get("command"))?;
    let mut tokens = if let Some(raw) = raw.as_str().filter(|raw| raw.len() <= MAX_COMMAND) {
        words(raw)?
    } else {
        let args = raw.as_array().filter(|args| args.len() <= 128)?;
        args.iter()
            .map(|arg| {
                arg.as_str()
                    .filter(|s| s.len() <= MAX_COMMAND)
                    .map(str::to_owned)
            })
            .collect::<Option<Vec<_>>>()?
    };
    if tokens.len() == 3
        && matches!(
            Path::new(&tokens[0]).file_name().and_then(|s| s.to_str()),
            Some("sh" | "bash" | "zsh")
        )
        && matches!(tokens[1].as_str(), "-c" | "-lc")
    {
        tokens = words(&tokens[2])?;
    }
    let mut repository = args
        .get("workdir")
        .or_else(|| args.get("cwd"))
        .and_then(Value::as_str)
        .or(cwd)
        .filter(|s| s.len() <= 4096)
        .and_then(|path| resolve(cwd, path));
    let mut result = None;
    let separators: Vec<_> = tokens
        .iter()
        .filter(|s| matches!(s.as_str(), ";" | "&&"))
        .collect();
    let segments: Vec<_> = tokens
        .split(|s| matches!(s.as_str(), ";" | "&&"))
        .filter(|s| !s.is_empty())
        .collect();
    for (i, segment) in segments.iter().enumerate() {
        if segment[0] == "cd" && segment.len() == 2 {
            if separators.get(i).map(|s| s.as_str()) != Some("&&") {
                return None;
            }
            repository = resolve(repository.as_deref(), &segment[1]);
            continue;
        }
        if segment[0] != "git" {
            return None;
        }
        let mut offset = 1;
        let mut repo = repository.clone();
        if segment.get(offset).is_some_and(|s| s == "-C") {
            repo = resolve(repo.as_deref(), segment.get(offset + 1)?);
            offset += 2;
        }
        match segment.get(offset).map(String::as_str) {
            Some("commit")
                if !segment
                    .iter()
                    .any(|s| matches!(s.as_str(), "--dry-run" | "--help" | "-h")) =>
            {
                // More than one Git commit in one command is ambiguous across repositories.
                if result.is_some() {
                    return None;
                }
                result = Some(Command {
                    turn,
                    repository: repo,
                    commit: true,
                    followup: i + 1 < segments.len(),
                    tag: None,
                });
            }
            Some("tag")
                if segments.len() == 1
                    && segment.len() == offset + 3
                    && !segment[offset + 1].starts_with('-')
                    && segment[offset + 1].len() <= 256
                    && hash(&segment[offset + 2]) =>
            {
                if result.is_none() {
                    result = Some(Command {
                        turn,
                        repository: repo,
                        commit: false,
                        followup: false,
                        tag: Some((sanitize(&segment[offset + 1]), segment[offset + 2].clone())),
                    });
                }
            }
            Some("add" | "status" | "push") => {}
            _ => return None,
        }
    }
    result
}
fn nested_commands(
    script: &str,
    turn: Option<usize>,
    cwd: Option<&str>,
    processes: &HashMap<String, Command>,
) -> Vec<Command> {
    let mut result = Vec::new();
    let bytes = script.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i..].starts_with(b"/*") {
            let Some(end) = script[i + 2..].find("*/") else {
                return Vec::new();
            };
            i += end + 4;
            continue;
        }
        if matches!(bytes[i], b'\'' | b'"' | b'`') {
            let quote = bytes[i];
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                } else if bytes[i] == quote {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        // Only straight-line awaited literal calls can be associated safely.
        if bytes[i] == b'?'
            || bytes[i..].starts_with(b"=>")
            || bytes[i..].starts_with(b"&&")
            || bytes[i..].starts_with(b"||")
        {
            return Vec::new();
        }
        if bytes[i..].starts_with(b"tools.") {
            if !script[..i].trim_end().ends_with("await") {
                return Vec::new();
            }
            let suffix = &script[i + 6..];
            let (suffix, stdin) = if let Some(suffix) = suffix.strip_prefix("exec_command(") {
                (suffix, false)
            } else if let Some(suffix) = suffix.strip_prefix("write_stdin(") {
                (suffix, true)
            } else {
                return Vec::new();
            };
            let start = script.len() - suffix.len();
            let mut stream = serde_json::Deserializer::from_str(suffix).into_iter::<Value>();
            let Some(Ok(args)) = stream.next() else {
                return Vec::new();
            };
            let command = if stdin {
                process_id(&args["session_id"]).and_then(|id| processes.get(&id).cloned())
            } else {
                command(&args, turn, cwd)
            };
            let Some(command) = command else {
                return Vec::new();
            };
            result.push(command);
            i = start + stream.byte_offset();
            continue;
        }
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            if matches!(
                &script[start..i],
                "if" | "else" | "function" | "for" | "while" | "switch" | "try" | "catch"
            ) {
                return Vec::new();
            }
        } else {
            i += 1;
        }
    }
    result
}

fn outputs(value: &Value, depth: usize) -> Vec<Value> {
    if depth > 5 {
        return Vec::new();
    }
    if let Some(text) = value.as_str() {
        let text = text.trim();
        let values: Vec<_> = serde_json::Deserializer::from_str(text)
            .into_iter::<Value>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_default();
        if !values.is_empty() {
            return values.iter().flat_map(|v| outputs(v, depth + 1)).collect();
        }
        // A wrapper's text may begin with execution metadata, then a JSON result.
        if let Some((_, body)) = text.split_once("\nOutput:\n") {
            if body.trim_start().starts_with('{') {
                return outputs(&Value::String(body.to_owned()), depth + 1);
            }
        }
        return vec![value.clone()];
    }
    if let Some(content) = value.get("content").and_then(Value::as_array) {
        return content
            .iter()
            .filter(|c| string(c, "type") == "text")
            .flat_map(|c| outputs(&c["text"], depth + 1))
            .collect();
    }
    vec![value.clone()]
}
