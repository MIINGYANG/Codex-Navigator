use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "codex-nav",
    version,
    about = "Read-only local navigator for Codex sessions"
)]
pub struct Cli {
    /// Open a session by exact ID, unique ID prefix, or rollout file path.
    #[arg(long, value_name = "SESSION_ID_OR_PATH")]
    pub session: Option<String>,
    /// Prefer sessions belonging to this directory (defaults to the current directory).
    #[arg(long, value_name = "PATH")]
    pub cwd: Option<PathBuf>,
    /// Include all dates in the main-session picker (subagents and unknown sources stay hidden).
    #[arg(long)]
    pub all: bool,
    /// Disable automatic updates; use r to refresh manually.
    #[arg(long)]
    pub no_watch: bool,
    /// Open the local read-only web reader instead of the terminal interface.
    #[arg(long)]
    pub web: bool,
    /// Loopback web port; use 0 to choose an available port automatically.
    #[arg(long, requires = "web", default_value_t = 8765)]
    pub port: u16,
    /// Print the local web URL without opening a browser.
    #[arg(long, requires = "web")]
    pub no_open: bool,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Diagnose local paths, session readability, watcher and clipboard availability.
    Doctor,
}
