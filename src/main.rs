use anyhow::{Context, Result};
use clap::Parser;
use codex_navigator::{
    app::{Action, App},
    cli::{Cli, Command},
    config::Config,
    discovery,
    domain::{Session, SessionSummary},
    ui,
    util::sanitize,
    watch::{SessionWorker, Update},
};
use crossterm::{
    cursor::Show,
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

fn main() {
    if let Err(error) = run() {
        eprintln!("codex-nav: {}", sanitize(&format!("{error:#}")));
        std::process::exit(1);
    }
}

struct TerminalGuard;
impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("Cannot enable terminal raw mode")?;
        let guard = Self;
        execute!(io::stdout(), EnterAlternateScreen).context("Cannot enter terminal screen")?;
        Ok(guard)
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
}

enum Found {
    Sessions(Vec<SessionSummary>),
    Path(PathBuf),
}
fn scan(
    home: PathBuf,
    cwd: PathBuf,
    all: bool,
    config: Config,
    session: Option<String>,
) -> Receiver<Result<Found>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let found = if let Some(id) = session {
            discovery::resolve_session(&home, &id, &config).map(Found::Path)
        } else {
            discovery::discover(&home, &cwd, all, &config)
                .map(discovery::main_sessions)
                .map(Found::Sessions)
        };
        let _ = tx.send(found);
    });
    rx
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    anyhow::ensure!(
        !(cli.web && cli.command.is_some()),
        "--web cannot be combined with doctor"
    );
    let mut config = Config::load()?;
    config.watch &= !cli.no_watch;
    let home = match cli.codex_home {
        Some(path) => path,
        None => discovery::resolve_codex_home()?,
    };
    let cwd = cli
        .cwd
        .unwrap_or(std::env::current_dir().context("Cannot resolve current directory")?);
    if matches!(cli.command, Some(Command::Doctor)) {
        return doctor(&home, &cwd, &config);
    }
    if cli.web {
        return codex_navigator::web::run(
            home,
            cwd,
            config,
            codex_navigator::web::WebOptions {
                port: cli.port,
                no_open: cli.no_open,
                all: cli.all,
                session: cli.session,
            },
        );
    }
    anyhow::ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "An interactive terminal is required. Try codex-nav --web, doctor or --help."
    );
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        previous_hook(info);
    }));
    let _guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut app = App::new(config.watch);
    app.show_picker(Vec::new(), Some("Discovering local sessions…".into()));
    app.loading = true;
    let mut discovery_rx = Some(scan(
        home.clone(),
        cwd.clone(),
        cli.all,
        config.clone(),
        cli.session,
    ));
    let mut worker: Option<SessionWorker> = None;
    let mut current_path: Option<PathBuf> = None;
    let mut summaries: Vec<SessionSummary> = Vec::new();
    let mut counter: Option<(usize, usize, SessionWorker)> = None;
    let mut count_cursor = 0;
    let mut clipboard: Option<arboard::Clipboard> = None;
    let mut toast_until = Instant::now();
    let mut redraw = true;
    loop {
        if let Some(rx) = &discovery_rx {
            if let Ok(result) = rx.try_recv() {
                discovery_rx = None;
                app.loading = false;
                redraw = true;
                let path = match result {
                    Ok(Found::Path(path)) => Some(path),
                    Ok(Found::Sessions(found)) => {
                        counter = None;
                        count_cursor = 0;
                        summaries = found;
                        let exact = summaries.iter().any(|s| {
                            s.cwd.as_ref().is_some_and(|p| {
                                p == &cwd
                                    || p.canonicalize()
                                        .ok()
                                        .zip(cwd.canonicalize().ok())
                                        .is_some_and(|(a, b)| a == b)
                            })
                        });
                        let notice = if summaries.is_empty() {
                            "Only main sessions are listed. Try --all for older sessions, or --session ID/PATH to open another source."
                        } else if !exact {
                            "No recent main session matched this directory. Select from recent main sessions · / search"
                        } else {
                            "Select a main session · Enter open · / search"
                        };
                        app.show_picker(summaries.clone(), Some(notice.into()));
                        None
                    }
                    Err(e) => {
                        app.toast = Some(sanitize(&format!("{e:#}")));
                        toast_until = Instant::now() + Duration::from_secs(8);
                        None
                    }
                };
                if let Some(path) = path {
                    open(&mut app, &mut worker, &mut current_path, path, &config);
                }
            }
        }
        // Complete large-file picker counts lazily; opening a session cancels this work.
        if app.picker && discovery_rx.is_none() {
            let mut finished = false;
            if let Some((index, count, counter_worker)) = &mut counter {
                while let Ok(update) = counter_worker.updates.try_recv() {
                    match update {
                        Update::Batch {
                            turns,
                            reset,
                            offset,
                            total,
                            ..
                        } => {
                            if reset {
                                *count = 0;
                            }
                            *count = turns.last().map_or(*count, |(i, _)| (*count).max(i + 1));
                            if offset >= total {
                                summaries[*index].turn_count = Some(*count);
                                if let Some(summary) = app.summaries.get_mut(*index) {
                                    summary.turn_count = Some(*count);
                                }
                                finished = true;
                                redraw = true;
                            }
                        }
                        Update::Error(_) => finished = true,
                    }
                }
            }
            if finished {
                counter = None;
            }
            if counter.is_none() {
                while count_cursor < summaries.len() && summaries[count_cursor].turn_count.is_some()
                {
                    count_cursor += 1;
                }
                if count_cursor < summaries.len() {
                    let mut count_config = config.clone();
                    count_config.watch = false;
                    counter = Some((
                        count_cursor,
                        0,
                        SessionWorker::start(summaries[count_cursor].path.clone(), count_config),
                    ));
                    count_cursor += 1;
                }
            }
        } else {
            counter = None;
        }
        if let Some(w) = &worker {
            while let Ok(update) = w.updates.try_recv() {
                redraw = true;
                match update {
                    Update::Batch {
                        meta,
                        stats,
                        revision,
                        turns,
                        events,
                        reset,
                        offset,
                        total,
                    } => {
                        app.apply_update(meta, stats, revision, turns, reset);
                        app.update_events(events);
                        app.loading = offset < total;
                        app.progress = if app.loading {
                            Some((offset, total))
                        } else {
                            None
                        };
                        if let Some(path) = &current_path {
                            if let Some(summary) = summaries.iter_mut().find(|s| &s.path == path) {
                                summary.turn_count = app.session.as_ref().map(|s| s.turns.len());
                            }
                        }
                    }
                    Update::Error(message) => {
                        app.loading = false;
                        app.toast = Some(message);
                        toast_until = Instant::now() + Duration::from_secs(8);
                    }
                }
            }
        }
        if app.toast.is_some() && Instant::now() >= toast_until {
            app.toast = None;
            redraw = true;
        }
        if redraw {
            terminal.draw(|frame| ui::draw(frame, &mut app))?;
            redraw = false;
        }
        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        match event::read()? {
            Event::Resize(_, _) => redraw = true,
            Event::Key(key) => {
                redraw = true;
                let previous_toast = app.toast.clone();
                let action = app.handle_key(key);
                if app.toast.is_some() && app.toast != previous_toast {
                    toast_until = Instant::now() + Duration::from_secs(3);
                }
                match action {
                    Action::Quit => break,
                    Action::None => (),
                    Action::OpenSession(path) => {
                        counter = None;
                        open(&mut app, &mut worker, &mut current_path, path, &config)
                    }
                    Action::Refresh => {
                        if let Some(worker) = &worker {
                            worker.refresh();
                        }
                    }
                    Action::ShowPicker => {
                        counter = None;
                        worker = None;
                        app.show_picker(
                            summaries.clone(),
                            Some("Refreshing local sessions…".into()),
                        );
                        app.loading = true;
                        if discovery_rx.is_none() {
                            discovery_rx = Some(scan(
                                home.clone(),
                                cwd.clone(),
                                cli.all,
                                config.clone(),
                                None,
                            ));
                        }
                    }
                    Action::Copy(text) => {
                        if clipboard.is_none() {
                            clipboard = arboard::Clipboard::new().ok();
                        }
                        let ok = clipboard
                            .as_mut()
                            .is_some_and(|clipboard| clipboard.set_text(text).is_ok());
                        app.toast = Some(
                            if ok {
                                "Copied to clipboard"
                            } else {
                                "Clipboard unavailable in this terminal session"
                            }
                            .into(),
                        );
                        toast_until = Instant::now() + Duration::from_secs(3);
                    }
                }
            }
            _ => (),
        }
    }
    drop(worker);
    Ok(())
}

fn open(
    app: &mut App,
    worker: &mut Option<SessionWorker>,
    current: &mut Option<PathBuf>,
    path: PathBuf,
    config: &Config,
) {
    *worker = None;
    app.open_session(Session::default());
    app.loading = true;
    *worker = Some(SessionWorker::start(path.clone(), config.clone()));
    *current = Some(path);
}

fn doctor(home: &std::path::Path, cwd: &std::path::Path, config: &Config) -> Result<()> {
    let sessions = home.join("sessions");
    println!(
        "Codex home        {}",
        sanitize(&home.display().to_string())
    );
    println!(
        "Sessions dir      {}",
        if sessions.is_dir() {
            "found"
        } else {
            "missing"
        }
    );
    println!(
        "Session index     {}",
        if home.join("session_index.jsonl").is_file() {
            "found"
        } else {
            "missing"
        }
    );
    match discovery::discover(home, cwd, false, config) {
        Ok(found) => {
            println!("Recent rollouts   {}", found.len());
            println!(
                "Readable          {}",
                if found.is_empty() {
                    "no readable recent rollouts"
                } else {
                    "yes"
                }
            );
        }
        Err(_) => println!("Readable          no (check filesystem permissions)"),
    }
    println!(
        "Watcher           {}",
        if codex_navigator::watch::watcher_available(&sessions) {
            "supported (100 ms polling fallback)"
        } else {
            "polling fallback (100 ms)"
        }
    );
    println!(
        "Clipboard         {}",
        if arboard::Clipboard::new().is_ok() {
            "supported"
        } else {
            "unavailable"
        }
    );
    println!("Session access    read-only; no network, login or API key required");
    Ok(())
}
