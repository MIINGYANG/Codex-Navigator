//! Navigator-owned, bounded favorites. Session files are never written.
use crate::domain::{Turn, TurnStatus};
use anyhow::{bail, Context, Result};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::{BTreeSet, HashMap},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_ENTRIES: usize = 20_000;
const MAX_KEY: usize = 32_768;

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Favorites {
    version: u8,
    entries: BTreeSet<String>,
}
impl Favorites {
    pub fn questions(&self) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .filter(|key| key.starts_with("q:"))
            .map(String::as_str)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains(key)
    }
}

pub(crate) fn path() -> Result<PathBuf> {
    Ok(BaseDirs::new()
        .context("无法定位 Navigator 数据目录")?
        .data_dir()
        .join("codex-nav/favorites.json"))
}

pub(crate) fn session_key(path: &Path, id: &str) -> Result<String> {
    let path = path
        .to_str()
        .context("会话路径不是有效 UTF-8，无法保存收藏")?;
    let key = format!("s:{}", serde_json::to_string(&(path, id))?);
    validate_key(&key)?;
    Ok(key)
}

fn question_identity(turn: &Turn) -> Cow<'_, str> {
    match turn.id.as_deref().filter(|id| !id.is_empty()) {
        Some(id) => Cow::Borrowed(id),
        None => {
            // Fixed FNV-1a rather than DefaultHasher, whose algorithm is not stable.
            let hash = turn
                .prompt
                .preview
                .as_bytes()
                .iter()
                .fold(0xcbf29ce484222325u64, |h, b| {
                    (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
                });
            Cow::Owned(format!(
                "fallback:{}:{}:{hash:016x}",
                turn.ordinal,
                turn.started_at.map(|v| v.to_rfc3339()).unwrap_or_default()
            ))
        }
    }
}

pub(crate) fn question_key(session: &str, turn: &Turn) -> Result<String> {
    let identity = question_identity(turn);
    let identity = if matches!(identity, Cow::Borrowed(_)) {
        format!("id:{identity}")
    } else {
        identity.into_owned()
    };
    let key = format!("q:{}", serde_json::to_string(&(session, identity))?);
    validate_key(&key)?;
    Ok(key)
}

/// Produce identities lazily so a long session path is not duplicated for every
/// question before the bounded search projection can enforce its memory budget.
pub(crate) fn question_keys<'a>(
    session: &'a str,
    turns: &'a [Turn],
) -> impl DoubleEndedIterator<Item = Option<String>> + ExactSizeIterator + 'a {
    let mut counts = HashMap::new();
    for turn in turns {
        let identity = question_identity(turn);
        *counts
            .entry((matches!(identity, Cow::Borrowed(_)), identity))
            .or_insert(0usize) += 1;
    }
    turns.iter().map(move |turn| {
        let identity = question_identity(turn);
        if turn.status != TurnStatus::RolledBack
            && counts.get(&(matches!(identity, Cow::Borrowed(_)), identity)) == Some(&1)
        {
            question_key(session, turn).ok()
        } else {
            None
        }
    })
}

fn validate_key(key: &str) -> Result<()> {
    if key.is_empty() || key.len() > MAX_KEY || !(key.starts_with("s:") || key.starts_with("q:")) {
        bail!("收藏身份无效或过长");
    }
    Ok(())
}
fn regular(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_file() => Ok(true),
        Ok(_) => bail!("收藏存储不是普通文件，拒绝读取或覆盖"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => bail!("无法检查收藏存储"),
    }
}
pub(crate) fn load(path: &Path) -> Result<Favorites> {
    if !regular(path)? {
        return Ok(Favorites {
            version: 1,
            ..Favorites::default()
        });
    }
    let mut bytes = Vec::new();
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x20000); // O_NOFOLLOW: do not follow a replaced symlink.
    }
    options
        .open(path)
        .context("无法读取收藏存储")?
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .context("无法读取收藏存储")?;
    if bytes.len() > MAX_BYTES {
        bail!("收藏存储超过 4 MiB，未修改原文件");
    }
    let value: Favorites = serde_json::from_slice(&bytes).map_err(|_| {
        anyhow::anyhow!("收藏存储损坏，未修改原文件；请检查 Navigator favorites.json")
    })?;
    if value.version != 1
        || value.entries.len() > MAX_ENTRIES
        || value.entries.iter().any(|key| validate_key(key).is_err())
    {
        bail!("收藏存储版本或内容无效，未修改原文件");
    }
    Ok(value)
}

#[cfg(unix)]
fn private_file(path: &Path, new: bool) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = OpenOptions::new();
    options.read(true).write(true).mode(0o600);
    #[cfg(target_os = "linux")]
    options.custom_flags(0x20000); // O_NOFOLLOW
    if new {
        options.create_new(true);
    } else {
        options.create(true).truncate(false);
    }
    options.open(path).context("无法打开 Navigator 收藏文件")
}
#[cfg(not(unix))]
fn private_file(path: &Path, new: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    if new {
        options.create_new(true);
    } else {
        options.create(true).truncate(false);
    }
    options.open(path).context("无法打开 Navigator 收藏文件")
}

// A persistent lock inode coordinates different Navigator ports/processes.
// Explicitly unlock: a concurrently forked child may briefly retain the same
// open-file description even after this process closes its CLOEXEC descriptor.
struct FileLock(File);
#[cfg(unix)]
fn flock(file: &File, operation: std::os::raw::c_int) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    unsafe extern "C" {
        #[link_name = "flock"]
        fn system_flock(
            fd: std::os::raw::c_int,
            operation: std::os::raw::c_int,
        ) -> std::os::raw::c_int;
    }
    // The borrowed File owns a valid descriptor for the entire call.
    if unsafe { system_flock(file.as_raw_fd(), operation) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}
impl Drop for FileLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        let _ = flock(&self.0, 8); // LOCK_UN releases inherited duplicates too.
    }
}
#[cfg(unix)]
fn lock(file: File) -> Result<FileLock> {
    // Nonblocking: a stalled writer never stalls the HTTP state worker.
    flock(&file, 2 | 4).context("收藏正由另一个 Navigator 保存，请稍后重试")?;
    Ok(FileLock(file))
}
#[cfg(not(unix))]
fn lock(_file: File) -> Result<FileLock> {
    bail!("当前平台暂不支持安全的跨进程收藏保存");
}

pub(crate) fn set(path: &Path, key: &str, favorite: bool) -> Result<()> {
    validate_key(key)?;
    let parent = path.parent().context("收藏目录无效")?;
    fs::create_dir_all(parent).context("无法创建 Navigator 收藏目录")?;
    if fs::symlink_metadata(parent)?.file_type().is_symlink() {
        bail!("收藏目录不能是符号链接");
    }
    let lock_path = parent.join("favorites.lock");
    regular(&lock_path)?;
    let _guard = lock(private_file(&lock_path, false)?)?;
    let mut data = load(path)?;
    if favorite {
        data.entries.insert(key.to_owned());
    } else {
        data.entries.remove(key);
    }
    if data.entries.len() > MAX_ENTRIES {
        bail!("收藏已达到 20,000 项上限");
    }
    let bytes = serde_json::to_vec(&data)?;
    if bytes.len() > MAX_BYTES {
        bail!("收藏已达到 4 MiB 上限");
    }
    let mut random = [0u8; 12];
    getrandom::fill(&mut random).map_err(|_| anyhow::anyhow!("无法生成收藏临时文件名"))?;
    let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let temporary = parent.join(format!(".favorites-{suffix}.tmp"));
    let mut file = private_file(&temporary, true)?;
    file.write_all(&bytes).context("无法保存收藏")?;
    file.sync_all().context("无法同步收藏")?;
    fs::rename(&temporary, path).context("无法原子保存收藏")?;
    File::open(parent)?.sync_all().context("无法同步收藏目录")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persistent_and_stable_identity() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("favorites.json");
        let a = session_key(Path::new("/synthetic/a.jsonl"), "same").unwrap();
        let b = session_key(Path::new("/synthetic/b.jsonl"), "same").unwrap();
        set(&path, &a, true).unwrap();
        set(&path, &b, true).unwrap();
        set(&path, &a, false).unwrap();
        let data = load(&path).unwrap();
        assert!(!data.contains(&a));
        assert!(data.contains(&b));
        let mut turn = Turn {
            ordinal: 1,
            ..Turn::default()
        };
        turn.prompt.preview = "first".into();
        let key = question_key(&b, &turn).unwrap();
        turn.prompt.preview = "different".into();
        assert_ne!(key, question_key(&b, &turn).unwrap());
        turn.id = Some("stable-id".into());
        let key = question_key(&b, &turn).unwrap();
        turn.ordinal = 5;
        assert_eq!(key, question_key(&b, &turn).unwrap());
    }
    #[test]
    fn ambiguous_and_rolled_back_questions_cannot_be_resolved() {
        let session = session_key(Path::new("/synthetic/a.jsonl"), "same").unwrap();
        let mut turns = vec![
            Turn {
                id: Some("a".into()),
                ..Turn::default()
            },
            Turn {
                id: Some("b".into()),
                ..Turn::default()
            },
        ];
        assert!(question_keys(&session, &turns).all(|key| key.is_some()));
        turns[1].id = Some("a".into());
        assert!(question_keys(&session, &turns).all(|key| key.is_none()));
        turns[1].id = Some("b".into());
        turns[1].status = TurnStatus::RolledBack;
        let keys: Vec<_> = question_keys(&session, &turns).collect();
        assert!(keys[0].is_some());
        assert!(keys[1].is_none());
    }

    #[test]
    fn corrupt_store_is_not_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("favorites.json");
        fs::write(&path, b"{broken").unwrap();
        assert!(load(&path).is_err());
        assert!(set(&path, "s:valid", true).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{broken");
    }
    #[test]
    fn refuses_oversized_or_unknown_version_store() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("favorites.json");
        let oversized = vec![b' '; MAX_BYTES + 1];
        fs::write(&path, &oversized).unwrap();
        assert!(load(&path).is_err());
        assert!(set(&path, "s:valid", true).is_err());
        assert_eq!(fs::metadata(&path).unwrap().len(), oversized.len() as u64);
        fs::write(&path, br#"{"version":2,"entries":[]}"#).unwrap();
        assert!(set(&path, "s:valid", true).is_err());
        assert!(session_key(Path::new("/a"), &"x".repeat(MAX_KEY)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn releases_lock_even_if_a_fork_or_duplicate_keeps_description_open() {
        let root = tempfile::tempdir().unwrap();
        let lock_path = root.path().join("favorites.lock");
        let held = lock(private_file(&lock_path, false).unwrap()).unwrap();
        // A dup shares exactly the lock lifetime that a forked child's fd does.
        let inherited = held.0.try_clone().unwrap();
        assert!(lock(private_file(&lock_path, false).unwrap()).is_err());
        drop(held);
        set(&root.path().join("favorites.json"), "s:valid", true).unwrap();
        assert!(inherited.metadata().is_ok());
        assert!(load(&root.path().join("favorites.json"))
            .unwrap()
            .contains("s:valid"));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinks_and_concurrent_writer() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("source");
        fs::write(&target, b"unchanged").unwrap();
        let path = root.path().join("favorites.json");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(set(&path, "s:valid", true).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"unchanged");
        let other = root.path().join("other");
        fs::create_dir(&other).unwrap();
        let _held = lock(private_file(&other.join("favorites.lock"), false).unwrap()).unwrap();
        assert!(set(&other.join("favorites.json"), "s:valid", true).is_err());
    }
}
