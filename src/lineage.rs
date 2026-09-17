//! Conservative, bounded dependency checks before moving a rollout to the trash.
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::{
    collections::{BTreeSet, HashMap},
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const MAX_HEADER: u64 = 1024 * 1024;
const MAX_BYTES: usize = 128 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;
const MAX_DEPTH: usize = 64;
const TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Deserialize)]
struct Header {
    #[serde(rename = "type")]
    kind: String,
    payload: Metadata,
}

#[derive(Deserialize)]
struct Metadata {
    id: String,
    history_mode: Option<String>,
    history_base: Option<HistoryBase>,
}

#[derive(Deserialize)]
struct HistoryBase {
    thread_id: String,
}

struct Scan {
    deadline: Instant,
    entries_left: usize,
    bytes_left: usize,
    records: HashMap<String, (PathBuf, BTreeSet<String>)>,
}

impl Scan {
    fn check_time(&self) -> Result<()> {
        if Instant::now() >= self.deadline {
            bail!("会话依赖检查超时，已停止删除；请稍后重试");
        }
        Ok(())
    }

    fn visit(&mut self, path: &Path, depth: usize) -> Result<()> {
        self.check_time()?;
        if depth > MAX_DEPTH || self.entries_left == 0 {
            bail!("会话依赖检查超过目录或文件数量上限，已停止删除");
        }
        self.entries_left -= 1;
        let metadata = fs::symlink_metadata(path).context("无法检查会话目录项，已停止删除")?;
        // A symlink may hide a live dependency, so skipping it is not sufficient.
        if metadata.file_type().is_symlink() {
            bail!("会话目录包含符号链接，无法完整检查依赖，已停止删除");
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(path).context("无法读取会话目录，已停止删除")? {
                let entry = entry.context("无法读取会话目录项，已停止删除")?;
                self.visit(&entry.path(), depth + 1)?;
            }
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            if !metadata.is_file() {
                bail!("会话记录不是普通文件，无法完整检查依赖，已停止删除");
            }
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
            }
            let file = options.open(path).context("无法读取会话头部，已停止删除")?;
            let opened = file.metadata().context("无法确认会话文件，已停止删除")?;
            if !opened.is_file() {
                bail!("会话文件在检查时发生变化，已停止删除");
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if opened.dev() != metadata.dev() || opened.ino() != metadata.ino() {
                    bail!("会话文件在检查时发生变化，已停止删除");
                }
            }
            let mut line = Vec::new();
            BufReader::new(file)
                .take(MAX_HEADER + 1)
                .read_until(b'\n', &mut line)
                .context("无法读取会话头部，已停止删除")?;
            self.check_time()?;
            if line.len() as u64 > MAX_HEADER || !line.ends_with(b"\n") {
                bail!("会话头部不完整或超过读取上限，已停止删除");
            }
            self.bytes_left = self
                .bytes_left
                .checked_sub(line.len())
                .context("会话依赖检查超过读取预算，已停止删除")?;
            // Typed deserialization also rejects duplicate identity/lineage fields.
            let header: Header = serde_json::from_slice(&line)
                .map_err(|_| anyhow::anyhow!("会话头部格式无效，无法完整检查依赖，已停止删除"))?;
            if header.kind != "session_meta" || !crate::management::valid_id(&header.payload.id) {
                bail!("会话缺少有效的 session_meta 身份，已停止删除");
            }
            let mut meta = header.payload;
            meta.id.make_ascii_lowercase();
            if meta
                .history_mode
                .as_deref()
                .is_some_and(|mode| !matches!(mode, "paginated" | "legacy"))
            {
                bail!("会话使用未知历史模式，无法完整检查依赖，已停止删除");
            }
            let mut sources = BTreeSet::new();
            // forked_from_id records ancestry only. Codex can resume a copied
            // legacy fork without its parent; history_base is the live reference.
            if let Some(mut source) = meta.history_base.map(|base| base.thread_id) {
                source.make_ascii_lowercase();
                if !crate::management::valid_id(&source) || source == meta.id {
                    bail!("会话来源身份无效，无法完整检查依赖，已停止删除");
                }
                sources.insert(source);
            }
            if self
                .records
                .insert(meta.id, (path.to_owned(), sources))
                .is_some()
            {
                bail!("存在重复会话 ID，无法完整检查依赖，已停止删除");
            }
        }
        Ok(())
    }
}

/// Includes archived and hidden/subagent rollouts, independently of UI discovery.
/// This is a preflight check, not a lock against another Codex process creating a fork.
pub(crate) fn ensure_unreferenced(home: &Path, target: &Path) -> Result<()> {
    check_with_limits(home, target, MAX_ENTRIES, MAX_BYTES, TIMEOUT)
}

fn check_with_limits(
    home: &Path,
    target: &Path,
    entries: usize,
    bytes: usize,
    timeout: Duration,
) -> Result<()> {
    let mut scan = Scan {
        deadline: Instant::now() + timeout,
        entries_left: entries,
        bytes_left: bytes,
        records: HashMap::new(),
    };
    for directory in ["sessions", "archived_sessions"] {
        let path = home.join(directory);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                scan.visit(&path, 0)?;
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && directory == "archived_sessions" => {}
            _ => bail!("无法完整检查会话目录 {directory}，已停止删除"),
        }
    }
    scan.check_time()?;
    let id = scan
        .records
        .iter()
        .find_map(|(id, (path, _))| (path == target).then_some(id))
        .context("未找到待删除会话的有效身份，已停止删除")?;
    let dependents: BTreeSet<_> = scan
        .records
        .iter()
        .filter_map(|(child, (_, sources))| sources.contains(id).then_some(child.as_str()))
        .collect();
    if !dependents.is_empty() {
        let ids = dependents
            .iter()
            .take(3)
            .copied()
            .collect::<Vec<_>>()
            .join("、");
        bail!(
            "无法删除：仍有 {} 个会话引用此会话的历史（{}{}）。移动来源文件会使分支无法恢复；请保留该会话。",
            dependents.len(), ids, if dependents.len() > 3 { " 等" } else { "" }
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use tempfile::TempDir;

    const SOURCE: &str = "01992a54-1234-7000-8000-111111111111";
    const CHILD: &str = "01992a54-1234-7000-8000-222222222222";
    const GRANDCHILD: &str = "01992a54-1234-7000-8000-333333333333";

    fn write(root: &Path, relative: &str, payload: Value) -> PathBuf {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                "{}\nPRIVATE BODY IS NOT JSON",
                json!({"type":"session_meta","payload":payload})
            ),
        )
        .unwrap();
        path
    }

    fn fixture() -> (TempDir, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let path = write(root.path(), "sessions/source.jsonl", json!({"id":SOURCE}));
        (root, path)
    }

    #[test]
    fn dependencies_protect_each_link_but_allow_leaf() {
        for mode in ["legacy", "paginated"] {
            let (root, source) = fixture();
            let child = json!({"id":CHILD,"history_mode":mode,"history_base":{"thread_id":SOURCE,"end_ordinal_exclusive":7,"end_byte_offset":34923}});
            let child = write(root.path(), "sessions/2000/01/01/child.jsonl", child);
            let leaf = write(
                root.path(),
                "sessions/deep/leaf.jsonl",
                json!({"id":GRANDCHILD,"history_mode":"paginated","history_base":{"thread_id":CHILD}}),
            );
            let original = fs::read(&source).unwrap();
            let error = ensure_unreferenced(root.path(), &source)
                .unwrap_err()
                .to_string();
            assert!(error.contains(CHILD));
            assert!(error.contains("1 个会话"));
            assert!(ensure_unreferenced(root.path(), &child)
                .unwrap_err()
                .to_string()
                .contains(GRANDCHILD));
            ensure_unreferenced(root.path(), &leaf).unwrap();
            assert_eq!(fs::read(source).unwrap(), original);
        }
    }

    #[test]
    fn archived_and_hidden_subagent_dependencies_are_included() {
        let (root, source) = fixture();
        write(
            root.path(),
            "archived_sessions/archived.jsonl",
            json!({"id":CHILD,"history_base":{"thread_id":SOURCE}}),
        );
        write(
            root.path(),
            "sessions/.hidden/subagent.jsonl",
            json!({"id":GRANDCHILD,"source":{"subagent":{"thread_spawn":{"parent_thread_id":SOURCE}}},"forked_from_id":SOURCE,"history_base":{"thread_id":SOURCE}}),
        );
        let error = ensure_unreferenced(root.path(), &source)
            .unwrap_err()
            .to_string();
        assert!(error.contains("2 个会话"));
        assert!(error.contains(CHILD) && error.contains(GRANDCHILD));
    }

    #[test]
    fn malformed_ambiguous_and_unknown_headers_fail_closed() {
        for payload in [
            json!({"id":SOURCE}), // duplicate identity
            json!({"id":CHILD,"history_mode":"future-format"}),
            json!({"id":CHILD,"history_base":{}}),
            json!({"id":CHILD,"history_base":{"thread_id":42}}),
            json!({"id":CHILD,"history_base":{"thread_id":"invalid"}}),
            json!({"id":CHILD,"history_base":{"thread_id":CHILD}}),
            json!({"id":"invalid"}),
            json!({"history_mode":"paginated"}),
        ] {
            let (root, source) = fixture();
            let original = fs::read(&source).unwrap();
            write(root.path(), "sessions/other.jsonl", payload);
            assert!(ensure_unreferenced(root.path(), &source).is_err());
            assert_eq!(fs::read(source).unwrap(), original);
        }
        for bytes in [
            String::new(),
            "{broken PRIVATE CONTENT}\n".to_owned(),
            format!("{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{CHILD}\",\"id\":\"{SOURCE}\"}}}}\n"),
            format!("{}\n", json!({"type":"event_msg","payload":{"id":CHILD}})),
            json!({"type":"session_meta","payload":{"id":CHILD}}).to_string(),
            format!("{}\n", "x".repeat(MAX_HEADER as usize + 1)),
        ] {
            let (root, source) = fixture();
            fs::write(root.path().join("sessions/broken.jsonl"), bytes).unwrap();
            let error = ensure_unreferenced(root.path(), &source).unwrap_err().to_string();
            assert!(!error.contains("PRIVATE CONTENT"));
            assert!(source.exists());
        }
    }

    #[test]
    fn budgets_and_unreadable_directory_roots_fail_closed() {
        let (root, source) = fixture();
        assert!(check_with_limits(root.path(), &source, 1, MAX_BYTES, TIMEOUT).is_err());
        assert!(check_with_limits(root.path(), &source, MAX_ENTRIES, 1, TIMEOUT).is_err());
        assert!(
            check_with_limits(root.path(), &source, MAX_ENTRIES, MAX_BYTES, Duration::ZERO)
                .is_err()
        );
        fs::write(root.path().join("archived_sessions"), "not a directory").unwrap();
        assert!(ensure_unreferenced(root.path(), &source).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_files_directories_and_nonregular_records_fail_closed() {
        use std::os::unix::fs::symlink;
        for directory in [false, true] {
            let (root, source) = fixture();
            symlink(
                if directory { root.path() } else { &source },
                root.path().join("sessions/linked.jsonl"),
            )
            .unwrap();
            assert!(ensure_unreferenced(root.path(), &source).is_err());
        }
        let (root, source) = fixture();
        symlink(
            root.path().join("missing"),
            root.path().join("archived_sessions"),
        )
        .unwrap();
        assert!(ensure_unreferenced(root.path(), &source).is_err());
        let (root, source) = fixture();
        let socket =
            std::os::unix::net::UnixListener::bind(root.path().join("sessions/socket.jsonl"))
                .unwrap();
        assert!(ensure_unreferenced(root.path(), &source).is_err());
        drop(socket);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_metadata_fails_closed() {
        use std::os::unix::fs::PermissionsExt;
        let (root, source) = fixture();
        let unreadable = write(
            root.path(),
            "sessions/unreadable.jsonl",
            json!({"id":CHILD}),
        );
        let permissions = fs::metadata(&unreadable).unwrap().permissions();
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();
        // A privileged test process can still read mode-000 files.
        let actually_unreadable = fs::File::open(&unreadable).is_err();
        let result = ensure_unreferenced(root.path(), &source);
        fs::set_permissions(unreadable, permissions).unwrap();
        if actually_unreadable {
            assert!(result.is_err());
        }
    }

    #[test]
    fn copied_legacy_forks_do_not_require_source_rollouts() {
        for mode in [Value::Null, json!("legacy"), json!("paginated")] {
            let (root, source) = fixture();
            write(
                root.path(),
                "sessions/copied.jsonl",
                json!({"id":CHILD,"history_mode":mode,"forked_from_id":SOURCE,"history_base":null}),
            );
            ensure_unreferenced(root.path(), &source).unwrap();
        }
    }

    #[test]
    fn only_metadata_is_read_and_target_identity_never_comes_from_filename() {
        let (root, source) = fixture();
        let misleading = root.path().join(format!("sessions/rollout-{CHILD}.jsonl"));
        fs::rename(&source, &misleading).unwrap();
        write(
            root.path(),
            "sessions/child.jsonl",
            json!({"id":CHILD,"history_base":{"thread_id":SOURCE}}),
        );
        assert!(ensure_unreferenced(root.path(), &misleading)
            .unwrap_err()
            .to_string()
            .contains(CHILD));
        let (root, source) = fixture();
        ensure_unreferenced(root.path(), &source).unwrap(); // non-JSON body is never parsed
    }
}
