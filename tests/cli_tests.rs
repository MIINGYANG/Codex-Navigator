use std::{fs, process::Command};
use tempfile::TempDir;

fn command(root: &TempDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_codex-nav"));
    command
        .env("CODEX_HOME", root.path().join("codex"))
        .env("XDG_CONFIG_HOME", root.path().join("config"));
    command
}

#[test]
fn cli_help_version_and_noninteractive_message() {
    let root = TempDir::new().unwrap();
    let help = command(&root).arg("--help").output().unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    for option in [
        "--session",
        "--codex-home",
        "--cwd",
        "--all",
        "--no-watch",
        "--web",
        "--port",
        "--no-open",
        "doctor",
    ] {
        assert!(help.contains(option));
    }
    let version = command(&root).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(String::from_utf8(version.stdout)
        .unwrap()
        .contains(env!("CARGO_PKG_VERSION")));
    let tui = command(&root).output().unwrap();
    assert!(!tui.status.success());
    assert!(String::from_utf8(tui.stderr)
        .unwrap()
        .contains("interactive terminal"));
}

#[test]
fn cli_codex_home_is_global_and_rejects_empty_path() {
    use clap::Parser;
    use codex_navigator::cli::Cli;

    for args in [
        vec!["codex-nav", "--codex-home", "/synthetic/home", "doctor"],
        vec!["codex-nav", "doctor", "--codex-home", "/synthetic/home"],
        vec!["codex-nav", "--web", "--codex-home", "/synthetic/home"],
    ] {
        let cli = Cli::try_parse_from(args).unwrap();
        assert_eq!(
            cli.codex_home.unwrap(),
            std::path::PathBuf::from("/synthetic/home")
        );
    }
    for args in [
        vec!["codex-nav", "--codex-home", "", "doctor"],
        vec!["codex-nav", "doctor", "--codex-home", ""],
    ] {
        assert!(Cli::try_parse_from(args).is_err());
    }
}

#[test]
fn web_options_require_web_and_validate_port() {
    let root = TempDir::new().unwrap();
    for args in [
        vec!["--port", "1234"],
        vec!["--no-open"],
        vec!["--web", "--port", "65536"],
        vec!["--web", "--port", "-1"],
        vec!["--web", "doctor"],
    ] {
        let output = command(&root).args(&args).output().unwrap();
        assert!(!output.status.success(), "accepted {args:?}");
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn cli_doctor_only_reports_environment_and_never_changes_source() {
    let root = TempDir::new().unwrap();
    let dir = root.path().join("codex/sessions");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("rollout-synthetic.jsonl");
    let bytes = b"{\"type\":\"session_meta\",\"payload\":{\"id\":\"synthetic\"}}\n{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"sensitive-test-marker\"}}\n";
    fs::write(&path, bytes).unwrap();
    let output = command(&root)
        .arg("doctor")
        .env("OPENAI_API_KEY", "secret-environment-marker")
        .output()
        .unwrap();
    assert!(output.status.success());
    let out = String::from_utf8(output.stdout).unwrap();
    for label in [
        "Codex home",
        "Sessions dir",
        "Session index",
        "Recent rollouts",
        "Readable",
        "Watcher",
        "Clipboard",
    ] {
        assert!(out.contains(label));
    }
    assert!(!out.contains("sensitive-test-marker"));
    assert!(!out.contains("secret-environment-marker"));
    assert_eq!(fs::read(path).unwrap(), bytes);
    assert!(!root.path().join("config").exists());
}
