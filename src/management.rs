//! Explicit session management through Codex's protocol and the system trash.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(8);
const MAX_LINE: u64 = 1024 * 1024;
const MAX_MESSAGES: usize = 128;

/// Rename without loading/resuming a thread or starting a model turn.
pub fn rename(home: &Path, path: &Path, id: &str, name: &str) -> Result<String> {
    let mut command = Command::new("codex");
    command.args(["app-server", "--stdio", "-c", "analytics.enabled=false"]);
    rename_with_command(home, path, id, name, command, TIMEOUT)
}

fn validated_name(name: &str) -> Result<&str> {
    if name.chars().any(char::is_control) {
        bail!("会话名称不能包含控制字符");
    }
    let name = name.trim();
    if !(1..=100).contains(&name.chars().count()) {
        bail!("会话名称需为 1–100 个字符");
    }
    Ok(name)
}

fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

fn target(home: &Path, path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("会话路径必须为绝对路径");
    }
    let canonical = path.canonicalize().context("无法确认会话文件位置")?;
    let root = home
        .join("sessions")
        .canonicalize()
        .context("无法确认会话目录")?;
    if canonical != path || !canonical.starts_with(root) || !canonical.is_file() {
        bail!("只能管理会话目录内未改变目标的普通文件");
    }
    Ok(canonical)
}

struct ManagedChild(Child);

impl Drop for ManagedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Protocol {
    _child: ManagedChild,
    input: ChildStdin,
    replies: Receiver<Result<Value, ()>>,
    deadline: Instant,
    messages: usize,
}

impl Protocol {
    fn start(mut command: Command, home: &Path, timeout: Duration) -> Result<Self> {
        let deadline = Instant::now() + timeout;
        command
            .current_dir(home)
            .env("CODEX_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = ManagedChild(command.spawn().context("无法启动 Codex 会话管理服务")?);
        let input = child
            .0
            .stdin
            .take()
            .context("无法连接 Codex 会话管理服务")?;
        let output = child
            .0
            .stdout
            .take()
            .context("无法读取 Codex 会话管理服务")?;
        let (sender, replies) = mpsc::sync_channel(8);
        // A bounded reader also handles a peer that never sends a newline. No payload
        // or stderr is copied into user-visible errors. Killing the owned child closes
        // its stdout; the reader never blocks waiting for queue space during cleanup.
        thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                let mut line = Vec::new();
                let read = reader
                    .by_ref()
                    .take(MAX_LINE + 1)
                    .read_until(b'\n', &mut line);
                let value = match read {
                    Ok(0) => return,
                    Ok(_) if line.len() as u64 <= MAX_LINE && line.ends_with(b"\n") => {
                        serde_json::from_slice(&line).map_err(|_| ())
                    }
                    _ => Err(()),
                };
                let failed = value.is_err();
                if sender.try_send(value).is_err() || failed {
                    return;
                }
            }
        });
        Ok(Self {
            _child: child,
            input,
            replies,
            deadline,
            messages: 0,
        })
    }

    fn send(&mut self, value: Value) -> Result<()> {
        // Only the five fixed metadata messages below use this pipe. Their combined
        // size is below 2 KiB, including a validated 100-character name, so a peer
        // that stops reading cannot fill the child stdin pipe with these writes.
        self.remaining()?;
        let mut message = serde_json::to_vec(&value).context("无法创建会话管理请求")?;
        message.push(b'\n');
        self.input
            .write_all(&message)
            .context("无法发送会话管理请求")?;
        self.input.flush().context("无法发送会话管理请求")
    }

    fn remaining(&self) -> Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .context("会话管理操作超时，请刷新后确认结果")
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Result<Value> {
        self.send(json!({"id":id,"method":method,"params":params}))?;
        loop {
            let value = self
                .replies
                .recv_timeout(self.remaining()?)
                .map_err(|_| anyhow::anyhow!("会话管理服务无响应，请刷新后确认结果"))?
                .map_err(|_| anyhow::anyhow!("会话管理服务返回了无效数据"))?;
            self.messages += 1;
            if self.messages > MAX_MESSAGES {
                bail!("会话管理服务消息过多，已停止操作");
            }
            if value.get("id") == Some(&json!(id)) {
                if value.get("error").is_some() {
                    bail!("Codex 无法完成会话管理请求，请检查会话状态后重试");
                }
                return value
                    .get("result")
                    .cloned()
                    .context("会话管理服务返回了无效结果");
            }
            // Only notifications are expected during this metadata-only exchange.
            if value.get("id").is_some() || value.get("method").and_then(Value::as_str).is_none() {
                bail!("会话管理服务返回了意外消息");
            }
        }
    }
}

fn checked_thread<'a>(value: &'a Value, path: &Path, id: &str) -> Result<&'a Value> {
    let thread = value.get("thread").context("无法确认 Codex 会话身份")?;
    let actual_path = thread["path"]
        .as_str()
        .map(Path::new)
        .filter(|p| p.is_absolute())
        .and_then(|p| p.canonicalize().ok());
    if thread["id"].as_str() != Some(id) || actual_path.as_deref() != Some(path) {
        bail!("Codex 会话身份或文件路径不匹配，已停止操作");
    }
    // This is instance-local protocol state, not a lock on other Codex processes.
    if thread["status"]["type"].as_str() != Some("notLoaded") {
        bail!("会话管理服务已加载该会话，请稍后重试");
    }
    Ok(thread)
}

fn rename_with_command(
    home: &Path,
    path: &Path,
    id: &str,
    name: &str,
    command: Command,
    timeout: Duration,
) -> Result<String> {
    let name = validated_name(name)?;
    if !valid_id(id) {
        bail!("会话 ID 不是有效 UUID");
    }
    let home = home.canonicalize().context("无法确认 Codex 数据目录")?;
    let path = target(&home, path)?;
    let mut protocol = Protocol::start(command, &home, timeout)?;
    protocol.request(
        1,
        "initialize",
        json!({"clientInfo":{"name":"codex_navigator","version":env!("CARGO_PKG_VERSION")}}),
    )?;
    protocol.send(json!({"method":"initialized"}))?;
    let before = protocol.request(
        2,
        "thread/read",
        json!({"threadId":id,"includeTurns":false}),
    )?;
    checked_thread(&before, &path, id)?;
    target(&home, &path)?;
    protocol.request(3, "thread/name/set", json!({"threadId":id,"name":name}))?;
    let after = protocol.request(
        4,
        "thread/read",
        json!({"threadId":id,"includeTurns":false}),
    )?;
    let thread = checked_thread(&after, &path, id)?;
    if thread["name"].as_str() != Some(name) {
        bail!("Codex 未确认新的会话名称，请刷新后确认结果");
    }
    Ok(name.to_owned())
}

/// Use only the platform trash; there is intentionally no permanent-delete fallback.
pub fn trash(home: &Path, path: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let mut command = Command::new("gio");
        command.arg("trash").arg("--").arg(path);
        trash_with_command(home, path, command, TIMEOUT)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (home, path);
        bail!("当前系统尚不支持会话移入回收站");
    }
}

#[cfg(any(target_os = "linux", test))]
fn trash_with_command(
    home: &Path,
    path: &Path,
    mut command: Command,
    timeout: Duration,
) -> Result<()> {
    target(home, path)?;
    let deadline = Instant::now() + timeout;
    let mut child = ManagedChild(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("无法启动系统回收站，请检查 gio 是否可用")?,
    );
    loop {
        if let Some(status) = child.0.try_wait().context("无法确认回收站操作状态")? {
            if !status.success() {
                bail!("无法将会话移入系统回收站，源文件未确认移除");
            }
            return match path.symlink_metadata() {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                _ => anyhow::bail!("回收站操作未确认成功，请刷新后检查会话文件"),
            };
        }
        if Instant::now() >= deadline {
            bail!("回收站操作超时，请刷新后确认结果");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    const ID: &str = "01992a54-1234-7000-8000-111111111111";

    fn fixture() -> (TempDir, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("sessions")).unwrap();
        let path = root.path().join("sessions/rollout.jsonl");
        fs::write(&path, "synthetic\n").unwrap();
        (root, path)
    }

    fn fake(root: &Path, path: &Path, mode: &str) -> Command {
        let script = root.join("fake.py");
        fs::write(&script, r#"
import json, os, pathlib, sys, time
mode, path, tid = sys.argv[1:]
assert pathlib.Path(os.environ["CODEX_HOME"]).is_absolute()
assert pathlib.Path(os.environ["CODEX_HOME"]) == pathlib.Path.cwd()
pathlib.Path("child-pid").write_text(str(os.getpid()))
name = None
for line in sys.stdin:
    request = json.loads(line)
    method = request['method']
    if 'id' not in request: continue
    if mode == 'hang': time.sleep(30)
    if mode == 'oversize': print('x' * 1048577, flush=True); continue
    if mode == 'error':
        print(json.dumps({'id':request['id'], 'error':{'message':'PRIVATE_PAYLOAD'}}),flush=True)
        continue
    result = {}
    if method == 'thread/read':
        result = {'thread':{'id':tid,'path':path if mode != 'mismatch' and not (mode == 'postmismatch' and name is not None) else str(pathlib.Path(path).parent.parent / 'other.jsonl'),'name':name,'status':{'type':'idle' if mode == 'loaded' else 'notLoaded'}}}
    if method == 'thread/name/set':
        pathlib.Path('name-set').write_text(request['params']['name'])
        name = request['params']['name'] if mode != 'wrongname' else 'wrong'
    print(json.dumps({'id':request['id'],'result':result}),flush=True)
"#).unwrap();
        fs::write(root.join("other.jsonl"), "other\n").unwrap();
        let mut command = Command::new("python3");
        command.arg(script).arg(mode).arg(path).arg(ID);
        command
    }

    fn assert_reaped(root: &Path) {
        #[cfg(target_os = "linux")]
        {
            if let Ok(pid) = fs::read_to_string(root.join("child-pid")) {
                assert!(
                    !Path::new("/proc").join(pid).exists(),
                    "child must be killed and reaped"
                );
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = root;
    }

    #[test]
    fn rename_validates_names_and_ids() {
        assert_eq!(validated_name("  中文名称  ").unwrap(), "中文名称");
        for value in ["", "   ", "bad\nname", "bad\u{7f}name"] {
            assert!(validated_name(value).is_err());
        }
        assert!(validated_name(&"中".repeat(100)).is_ok());
        assert!(validated_name(&"中".repeat(101)).is_err());
        assert!(valid_id(ID));
        assert!(!valid_id("x"));
        assert!(!valid_id("01992a54-1234-7000-8000-11111111111z"));
    }

    #[test]
    fn rename_uses_protocol_and_preserves_rollout() {
        let (root, path) = fixture();
        let command = fake(root.path(), &path, "ok");
        assert_eq!(
            rename_with_command(root.path(), &path, ID, " 新名称 ", command, TIMEOUT).unwrap(),
            "新名称"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("name-set")).unwrap(),
            "新名称"
        );
        assert_eq!(fs::read_to_string(path).unwrap(), "synthetic\n");
        assert_reaped(root.path());
    }

    #[test]
    fn rename_resolves_relative_home_before_starting_child() {
        let cwd = std::env::current_dir().unwrap();
        let root = tempfile::tempdir_in(&cwd).unwrap();
        fs::create_dir(root.path().join("sessions")).unwrap();
        let path = root.path().join("sessions/rollout.jsonl");
        fs::write(&path, "synthetic\n").unwrap();
        let path = path.canonicalize().unwrap();
        let relative_home = root.path().strip_prefix(&cwd).unwrap();
        assert!(!relative_home.is_absolute());
        let command = fake(root.path(), &path, "ok");
        assert_eq!(
            rename_with_command(relative_home, &path, ID, "名称", command, TIMEOUT).unwrap(),
            "名称"
        );
        assert_reaped(root.path());
    }

    #[test]
    fn rename_rejects_wrong_target_loaded_thread_and_protocol_failure_before_write() {
        for mode in ["mismatch", "loaded", "error", "oversize"] {
            let (root, path) = fixture();
            let command = fake(root.path(), &path, mode);
            let error =
                rename_with_command(root.path(), &path, ID, "名称", command, TIMEOUT).unwrap_err();
            assert!(!root.path().join("name-set").exists(), "{mode}");
            assert!(!format!("{error:#}").contains("PRIVATE_PAYLOAD"));
            assert_reaped(root.path());
        }
    }

    #[test]
    fn rename_requires_readback_confirmation_and_times_out() {
        let (root, path) = fixture();
        let command = fake(root.path(), &path, "postmismatch");
        assert!(rename_with_command(root.path(), &path, ID, "名称", command, TIMEOUT).is_err());
        assert_reaped(root.path());
        let command = fake(root.path(), &path, "wrongname");
        assert!(rename_with_command(root.path(), &path, ID, "名称", command, TIMEOUT).is_err());
        let command = fake(root.path(), &path, "hang");
        let start = Instant::now();
        assert!(rename_with_command(
            root.path(),
            &path,
            ID,
            "名称",
            command,
            Duration::from_millis(100)
        )
        .is_err());
        assert!(start.elapsed() < Duration::from_secs(3));
        assert_reaped(root.path());
    }

    #[test]
    fn trash_requires_success_and_missing_original_and_has_timeout() {
        let (root, path) = fixture();
        let mut false_success = Command::new("python3");
        false_success.args(["-c", "pass"]);
        assert!(trash_with_command(root.path(), &path, false_success, TIMEOUT).is_err());
        let mut failure = Command::new("python3");
        failure.args(["-c", "raise SystemExit(1)"]);
        assert!(trash_with_command(root.path(), &path, failure, TIMEOUT).is_err());
        let mut hang = Command::new("python3");
        hang.args(["-c", "import time; time.sleep(30)"]);
        let start = Instant::now();
        assert!(trash_with_command(root.path(), &path, hang, Duration::from_millis(100)).is_err());
        assert!(start.elapsed() < Duration::from_secs(3));
        let saved = root.path().join("saved.jsonl");
        let mut move_file = Command::new("python3");
        move_file
            .args(["-c", "import os,sys; os.rename(sys.argv[1],sys.argv[2])"])
            .arg(&path)
            .arg(&saved);
        trash_with_command(root.path(), &path, move_file, TIMEOUT).unwrap();
        assert_eq!(fs::read_to_string(saved).unwrap(), "synthetic\n");
    }

    #[test]
    fn targets_must_be_regular_canonical_files_in_sessions() {
        let (root, path) = fixture();
        assert_eq!(target(root.path(), &path).unwrap(), path);
        assert!(target(root.path(), Path::new("relative.jsonl")).is_err());
        assert!(target(root.path(), &root.path().join("sessions")).is_err());
        let outside = root.path().join("outside.jsonl");
        fs::write(&outside, "outside").unwrap();
        assert!(target(root.path(), &outside).is_err());
        #[cfg(unix)]
        {
            let link = root.path().join("sessions/link.jsonl");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(target(root.path(), &link).is_err());
        }
    }
}
