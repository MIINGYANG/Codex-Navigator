use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use codex_navigator::{config::Config, discovery, util::sanitize, web};
use std::{fs, path::PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "codex-trail",
    version = "1.0.0",
    about = "Question Trail: explore your local Codex questions, not hidden reasoning",
    after_help = "Read-only and local-only. No AI/API calls or conversation uploads.\nUse codex-nav for the terminal navigator. Press Ctrl+C to stop the web server."
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
    // Trail is a live question viewer, independent of the reader's watch preference.
    config.watch = true;
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
            session: None,
        },
    )
}

fn doctor(home: &std::path::Path, cwd: &std::path::Path, config: &Config) -> Result<()> {
    println!("Codex Question Trail 1.0.0");
    println!("Codex home: {}", sanitize(&home.display().to_string()));
    println!("Privacy: read-only · localhost only · no AI/API calls · no uploads");
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
    println!("Watcher: enabled, with incremental reads and polling fallback");
    if readable == 0 {
        println!("No readable sessions found. Send a question in Codex, then rescan.");
    }
    println!("Start: codex-trail (or --no-open to print the local URL)");
    Ok(())
}
