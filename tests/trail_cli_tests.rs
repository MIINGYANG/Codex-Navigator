use std::{fs, process::Command};
use tempfile::TempDir;

fn command(root: &TempDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_codex-trail"));
    command
        .env("CODEX_HOME", root.path().join("codex"))
        .env("XDG_CONFIG_HOME", root.path().join("config"));
    command
}

#[test]
fn trail_help_and_version_describe_compatible_entry() {
    let root = TempDir::new().unwrap();
    let output = command(&root).arg("--help").output().unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    for text in [
        "--no-open",
        "--port",
        "47321",
        "--codex-home",
        "--session",
        "--no-watch",
        "doctor",
        "codex-nav",
        "No model calls",
    ] {
        assert!(help.contains(text), "missing {text}");
    }
    let version = command(&root).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap(),
        format!("codex-trail {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn trail_rejects_invalid_arguments_without_starting_server() {
    let root = TempDir::new().unwrap();
    for args in [
        vec!["--port", "65536"],
        vec!["--port", "-1"],
        vec!["--codex-home", "", "doctor"],
        vec!["--web"],
    ] {
        let result = command(&root).args(&args).output().unwrap();
        assert!(!result.status.success(), "accepted {args:?}");
        assert!(!result.stderr.is_empty());
    }
}

#[test]
fn trail_doctor_reports_missing_home_without_creating_it() {
    let root = TempDir::new().unwrap();
    let result = command(&root).arg("doctor").output().unwrap();
    assert!(result.status.success());
    assert!(String::from_utf8(result.stdout)
        .unwrap()
        .contains("Codex home missing"));
    assert!(!root.path().join("codex").exists());
    assert!(!root.path().join("config").exists());
}

#[test]
fn trail_home_flag_overrides_environment_and_doctor_keeps_data_private() {
    let root = TempDir::new().unwrap();
    let home = root.path().join("explicit");
    let sessions = home.join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    let path = sessions.join("rollout-synthetic.jsonl");
    let bytes = b"{\"type\":\"session_meta\",\"payload\":{\"id\":\"synthetic\",\"source\":\"cli\"}}\n{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"private-prompt-marker\"}}\n";
    fs::write(&path, bytes).unwrap();
    for args in [
        vec!["--codex-home", home.to_str().unwrap(), "doctor"],
        vec!["doctor", "--codex-home", home.to_str().unwrap()],
    ] {
        let result = command(&root)
            .args(args)
            .env("OPENAI_API_KEY", "private-key-marker")
            .output()
            .unwrap();
        assert!(result.status.success());
        let out = String::from_utf8(result.stdout).unwrap();
        for label in [
            "Discovered rollouts: 1",
            "Readable: 1",
            "Main sessions: 1",
            "read-only",
            "Watcher",
        ] {
            assert!(out.contains(label), "missing {label}: {out}");
        }
        assert!(!out.contains("private-prompt-marker"));
        assert!(!out.contains("private-key-marker"));
    }
    assert_eq!(fs::read(path).unwrap(), bytes);
    assert!(!root.path().join("codex").exists());
    assert!(!root.path().join("config").exists());
}

#[test]
fn trail_doctor_rejects_non_directory_home() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("codex"), "not a directory").unwrap();
    let result = command(&root).arg("doctor").output().unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8(result.stderr)
        .unwrap()
        .contains("not a directory"));
}
