use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use codex_navigator::{config::Config, discovery, util::sanitize, web};
use std::{fs, path::PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "codex-trail",
    version,
    about = "Compatibility entry for codex-nav --web (Question Trail)",
    after_help = "Local-only browsing and explicit session management. No model calls or conversation uploads.\nUse codex-nav --web for the web reader, or codex-nav for the terminal navigator. Press Ctrl+C to stop the web server."
)]
struct Cli {
    /// Print the private local URL without opening a browser.
    #[arg(long)]
    no_open: bool,
    /// Loopback port; use 0 to choose an available port automatically.
    #[arg(long, default_value_t = 47321)]
    port: u16,
    /// Codex data directory (otherwise CODEX_HOME, then ~/.codex).
    #[arg(long, value_name = "PATH", global = true)]
    codex_home: Option<PathBuf>,
    /// Open a session by exact ID, unique ID prefix, or rollout file path.
    #[arg(long, value_name = "SESSION_ID_OR_PATH")]
    session: Option<String>,
    /// Disable automatic updates; use Refresh in the web UI.
    #[arg(long)]
    no_watch: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Diagnose local session discovery and readability without changing files.
    Doctor,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("codex-trail: {}", sanitize(&format!("{error:#}")));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let home = match cli.codex_home {
        Some(path) => {
            anyhow::ensure!(
                !path.as_os_str().is_empty(),
                "--codex-home must not be empty"
            );
            path
        }
        None => discovery::resolve_codex_home()?,
    };
    let cwd = std::env::current_dir().context("Cannot resolve current directory")?;
    let mut config = Config::load()?;
    if cli.no_watch {
        config.watch = false;
    }
    if matches!(cli.command, Some(Command::Doctor)) {
        return doctor(&home, &cwd, &config);
    }
    web::run_trail(
        home,
        cwd,
        config,
        web::WebOptions {
            port: cli.port,
            no_open: cli.no_open,
            all: true,
            session: cli.session,
        },
    )
}

fn doctor(home: &std::path::Path, cwd: &std::path::Path, config: &Config) -> Result<()> {
    println!(
        "Codex Navigator {} · Question Trail",
        env!("CARGO_PKG_VERSION")
    );
    println!("Codex home: {}", sanitize(&home.display().to_string()));
    println!("Doctor: read-only · Web: local browsing, explicit rename/trash · no model calls");
    println!("Discovery: --codex-home > CODEX_HOME > ~/.codex; all dates and projects");
    match fs::metadata(home) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("Codex home missing. Run Codex once, or pass --codex-home PATH.");
            return Ok(());
        }
        Err(error) => return Err(error).context("Cannot inspect Codex home"),
        Ok(metadata) => anyhow::ensure!(metadata.is_dir(), "Codex home is not a directory"),
    }
    let sessions_dir = home.join("sessions");
    println!(
        "Sessions dir: {}",
        sanitize(&sessions_dir.display().to_string())
    );
    match fs::read_dir(&sessions_dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("No sessions directory yet. Send a question in Codex, then rescan.");
            return Ok(());
        }
        Err(error) => return Err(error).context("Cannot read Codex sessions directory"),
        Ok(_) => (),
    }
    let sessions = discovery::discover(home, cwd, true, config)?;
    let readable = sessions
        .iter()
        .filter(|session| fs::File::open(&session.path).is_ok())
        .count();
    println!("Discovered rollouts: {}", sessions.len());
    println!("Readable: {readable}");
    println!(
        "Main sessions: {}",
        discovery::main_sessions(sessions).len()
    );
    println!(
        "Watcher: {}",
        if config.watch {
            "enabled, with incremental reads and polling fallback"
        } else {
            "disabled; refresh manually"
        }
    );
    if readable == 0 {
        println!("No readable sessions found. Send a question in Codex, then rescan.");
    }
    println!("Start: codex-nav --web (codex-trail remains a compatible entry)");
    Ok(())
}
