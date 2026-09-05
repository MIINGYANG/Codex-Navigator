//! Read-only, bounded session discovery. Index entries supplement actual rollouts.
use crate::parser::{jsonl_reader::BoundedReader, Parser};
use crate::{
    config::Config,
    domain::{SessionKind, SessionSummary},
};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration, Local, Utc};
use directories::BaseDirs;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const HEADER_BYTES: u64 = 1024 * 1024;
const INDEX_BYTES: u64 = 1024 * 1024;

pub fn resolve_codex_home() -> Result<PathBuf> {
    let override_home = std::env::var_os("CODEX_HOME");
    let dirs = BaseDirs::new();
    codex_home_from(
        override_home.as_deref(),
        dirs.as_ref().map(BaseDirs::home_dir),
    )
}

/// Pure path resolution also lets callers test overrides without mutating process environment.
pub fn codex_home_from(override_home: Option<&OsStr>, user_home: Option<&Path>) -> Result<PathBuf> {
    if let Some(value) = override_home.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(value));
    }
    user_home
        .map(|home| home.join(".codex"))
        .context("Cannot locate Codex home; set CODEX_HOME explicitly")
}

pub fn discover(
    home: &Path,
    cwd: &Path,
    all: bool,
    config: &Config,
) -> Result<Vec<SessionSummary>> {
    let root = home.join("sessions");
    if !root.exists() {
        return Ok(Vec::new());
    }
    // Surface root permission errors, but tolerate individual files being rotated during discovery.
    fs::read_dir(&root).context("Cannot read Codex sessions directory")?;
    let paths = rollout_paths(&root, all, config.recent_days)?;
    let index = read_index(&home.join("session_index.jsonl"));
    let mut summaries = Vec::with_capacity(paths.len());
    for path in paths {
        if let Ok(mut summary) = read_summary(&path, config) {
            if let Some((title, indexed_time)) = index.get(&summary.id) {
                summary.title = title.clone();
                summary.updated_at = summary.updated_at.max(*indexed_time);
            }
            summaries.push(summary);
        }
    }
    let cwd = normalized_path(cwd);
    summaries.sort_by(|left, right| {
        relationship(right.cwd.as_deref(), &cwd)
            .cmp(&relationship(left.cwd.as_deref(), &cwd))
            .then_with(|| {
                identity_priority(left.identity.kind).cmp(&identity_priority(right.identity.kind))
            })
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(summaries)
}

/// Picker-only policy: explicit ID/path resolution still includes every source kind.
pub fn main_sessions(mut summaries: Vec<SessionSummary>) -> Vec<SessionSummary> {
    summaries.retain(|session| session.identity.kind == SessionKind::Main);
    summaries
}

fn identity_priority(kind: SessionKind) -> u8 {
    match kind {
        SessionKind::Main => 0,
        SessionKind::Unknown => 1,
        SessionKind::Subagent => 2,
    }
}

pub fn resolve_session(home: &Path, id_or_path: &str, config: &Config) -> Result<PathBuf> {
    if id_or_path.trim().is_empty() {
        bail!("Session ID or path must not be empty");
    }
    let path = Path::new(id_or_path);
    if path.exists() {
        if !path.metadata()?.is_file() {
            bail!("The selected session path is not a regular file");
        }
        File::open(path).context("Cannot read the selected session")?;
        return Ok(path.to_path_buf());
    }
    let sessions = discover(home, Path::new(""), true, config)?;
    let exact: Vec<_> = sessions
        .iter()
        .filter(|session| session.id == id_or_path)
        .collect();
    if exact.len() == 1 {
        return Ok(exact[0].path.clone());
    }
    let mut matches = sessions
        .iter()
        .filter(|session| session.id.starts_with(id_or_path));
    match (matches.next(), matches.next()) {
        (Some(session), None) => Ok(session.path.clone()),
        (None, _) => {
            bail!("Session not found; use --all to browse history or provide a rollout path")
        }
        _ => bail!("Session ID is ambiguous; provide the complete ID or rollout path"),
    }
}

fn normalized_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn relationship(session_cwd: Option<&Path>, cwd: &Path) -> u8 {
    let Some(session_cwd) = session_cwd else {
        return 0;
    };
    let session_cwd = normalized_path(session_cwd);
    if session_cwd == cwd {
        2
    } else if !cwd.as_os_str().is_empty()
        && !session_cwd.as_os_str().is_empty()
        && (session_cwd.starts_with(cwd) || cwd.starts_with(&session_cwd))
    {
        1
    } else {
        0
    }
}

fn rollout_paths(root: &Path, all: bool, recent_days: u32) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    collect_files(root, &mut paths)?;
    if all {
        collect_history(root, 3, &mut paths)?;
    } else {
        // Date directories are local-time dated. Include tomorrow to tolerate timezone changes.
        let today = Local::now().date_naive();
        for ago in -1..i64::from(recent_days) {
            let date = today - Duration::days(ago);
            let dir = root.join(date.format("%Y/%m/%d").to_string());
            if dir.is_dir() {
                collect_files(&dir, &mut paths)?;
            }
        }
        if paths.is_empty() {
            collect_recent_existing_dates(root, recent_days, &mut paths)?;
        }
    }
    let mut seen = HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));
    Ok(paths)
}

/// Fall back to existing date directories on inactive installations. The scan
/// inspects at most 366 days and opens rollouts in at most 31 populated days.
fn collect_recent_existing_dates(
    root: &Path,
    recent_days: u32,
    paths: &mut Vec<PathBuf>,
) -> Result<()> {
    let mut remaining_days = recent_days.min(31);
    let mut inspected_days = 0;
    for (_, year) in numbered_directories(root, 4, 9999)? {
        for (_, month) in numbered_directories(&year, 2, 12).unwrap_or_default() {
            for (_, day) in numbered_directories(&month, 2, 31).unwrap_or_default() {
                if remaining_days == 0 || inspected_days >= 366 {
                    return Ok(());
                }
                inspected_days += 1;
                let before = paths.len();
                let _ = collect_files(&day, paths);
                if paths.len() > before {
                    remaining_days -= 1;
                }
            }
        }
    }
    Ok(())
}

fn numbered_directories(
    root: &Path,
    digits: usize,
    maximum: u32,
) -> std::io::Result<Vec<(u32, PathBuf)>> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(root)?.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.len() != digits || !name.bytes().all(|character| character.is_ascii_digit()) {
            continue;
        }
        if let Ok(number) = name.parse::<u32>() {
            if (1..=maximum).contains(&number) {
                dirs.push((number, entry.path()));
            }
        }
    }
    dirs.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.0));
    Ok(dirs)
}

fn collect_files(dir: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        if name.starts_with("rollout-") && name.ends_with(".jsonl") && path.is_file() {
            paths.push(path);
        }
    }
    Ok(())
}

fn collect_history(dir: &Path, depth: usize, paths: &mut Vec<PathBuf>) -> Result<()> {
    if depth == 0 {
        return Ok(());
    }
    for entry in fs::read_dir(dir)?.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            let path = entry.path();
            // Unreadable individual date directories should not hide the rest of history.
            let _ = collect_files(&path, paths);
            let _ = collect_history(&path, depth - 1, paths);
        }
    }
    Ok(())
}

fn read_summary(path: &Path, config: &Config) -> Result<SessionSummary> {
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    let modified = metadata.modified().ok().map(DateTime::<Utc>::from);
    let stem = path
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("unknown");
    let fallback_id = stem
        .get(stem.len().saturating_sub(36)..)
        .filter(|id| {
            id.len() == 36
                && id.chars().enumerate().all(|(index, character)| {
                    if [8, 13, 18, 23].contains(&index) {
                        character == '-'
                    } else {
                        character.is_ascii_hexdigit()
                    }
                })
        })
        .unwrap_or_else(|| stem.strip_prefix("rollout-").unwrap_or(stem));
    let mut summary = SessionSummary {
        id: fallback_id.to_owned(),
        path: path.to_path_buf(),
        updated_at: modified,
        ..SessionSummary::default()
    };
    // Share human-input classification and dedup with the viewer: recent formats carry
    // user.text metadata and item_completed, and must not expose injected instructions.
    let mut reader = BoundedReader::open(path, config.max_record_bytes)?;
    let mut parser = Parser::new(config.preview_width);
    reader.read_batch(HEADER_BYTES as usize, |record| parser.consume(record))?;
    let session = parser.session;
    if !session.meta.id.is_empty() {
        summary.id = session.meta.id;
    }
    summary.cwd = session.meta.cwd;
    summary.identity = session.meta.identity;
    summary.first_prompt = session.turns.first().map(|turn| {
        if turn.prompt.text.is_empty() {
            turn.prompt.preview.clone()
        } else {
            turn.prompt.text.chars().take(2048).collect()
        }
    });
    if reader.byte_offset >= reader.len()? && reader.pending_len() == 0 {
        summary.turn_count = Some(session.turns.len());
    }
    Ok(summary)
}

/// Bounded one-shot JSONL read: oversized rows are discarded through their newline.
/// Incomplete trailing rows are ignored because discovery never owns a live parser cursor.
fn bounded_line(reader: &mut impl BufRead, limit: usize) -> std::io::Result<Option<Vec<u8>>> {
    let mut row = Vec::new();
    let mut oversize = false;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(None);
        }
        let newline = memchr::memchr(b'\n', buffer);
        let consumed = newline.map_or(buffer.len(), |index| index + 1);
        if !oversize {
            if row.len().saturating_add(consumed) <= limit {
                row.extend_from_slice(&buffer[..consumed]);
            } else {
                oversize = true;
                row.clear();
            }
        }
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(Some(row));
        }
    }
}

type IndexEntry = (Option<String>, Option<DateTime<Utc>>);

fn read_index(path: &Path) -> HashMap<String, IndexEntry> {
    let mut entries = HashMap::new();
    if !path.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return entries;
    }
    let Ok(mut file) = File::open(path) else {
        return entries;
    };
    let length = file.metadata().map_or(0, |meta| meta.len());
    let start = length.saturating_sub(INDEX_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return entries;
    }
    let mut reader = BufReader::new(file.take(INDEX_BYTES));
    if start > 0 {
        let _ = bounded_line(&mut reader, 64 * 1024);
    }
    while let Ok(Some(line)) = bounded_line(&mut reader, 64 * 1024) {
        let bytes = line.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&line);
        let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
            continue;
        };
        let Some(id) = value
            .get("id")
            .or_else(|| value.get("session_id"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let title = value
            .get("thread_name")
            .or_else(|| value.get("title"))
            .and_then(Value::as_str)
            .map(|text| text.chars().take(2048).collect());
        let updated = value
            .get("updated_at")
            .and_then(Value::as_str)
            .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
            .map(|time| time.with_timezone(&Utc));
        entries.insert(id.to_owned(), (title, updated));
    }
    entries
}
