//! Background tail with bounded delivery and a filesystem hint plus polling fallback.
use crate::{
    config::Config,
    domain::{ParseStats, SessionMeta, Turn},
    parser::Tail,
};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub enum Update {
    Batch {
        meta: SessionMeta,
        stats: ParseStats,
        revision: u64,
        turns: Vec<(usize, Turn)>,
        reset: bool,
        offset: u64,
        total: u64,
    },
    Error(String),
}
enum Command {
    Refresh,
    Stop,
}
pub struct SessionWorker {
    pub updates: Receiver<Update>,
    commands: mpsc::Sender<Command>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl SessionWorker {
    pub fn start(path: PathBuf, config: Config) -> Self {
        let (tx, updates) = mpsc::sync_channel(2);
        let (commands, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let handle = thread::spawn(move || run(path, config, tx, rx, worker_stop));
        Self {
            updates,
            commands,
            stop,
            handle: Some(handle),
        }
    }
    pub fn refresh(&self) {
        let _ = self.commands.send(Command::Refresh);
    }
}
impl Drop for SessionWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.commands.send(Command::Stop);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub fn watcher_available(path: &std::path::Path) -> bool {
    let Ok(mut watcher) = notify::recommended_watcher(|_: notify::Result<notify::Event>| {}) else {
        return false;
    };
    watcher.watch(path, RecursiveMode::NonRecursive).is_ok()
}

fn run(
    path: PathBuf,
    config: Config,
    tx: SyncSender<Update>,
    rx: Receiver<Command>,
    stop: Arc<AtomicBool>,
) {
    let mut tail = match Tail::open(&path, config.max_record_bytes, config.preview_width) {
        Ok(tail) => tail,
        Err(e) => {
            let _ = tx.try_send(Update::Error(format!(
                "Cannot open session ({:?}). Check permissions or select another session.",
                e.kind()
            )));
            return;
        }
    };
    let hint = Arc::new(AtomicBool::new(false));
    let event_hint = hint.clone();
    let mut watcher: Option<RecommendedWatcher> = if config.watch {
        notify::recommended_watcher(move |_: notify::Result<notify::Event>| {
            event_hint.store(true, Ordering::Relaxed);
        })
        .ok()
    } else {
        None
    };
    if let Some(w) = watcher.as_mut() {
        if w.watch(path.parent().unwrap_or(&path), RecursiveMode::NonRecursive)
            .is_err()
        {
            watcher = None;
        }
    }
    let _watcher = watcher;
    let mut loading = true;
    let mut force = true;
    let mut reset = true;
    let mut sent_revision = u64::MAX;
    let mut sent_offset = u64::MAX;
    let mut last_sent = Instant::now() - Duration::from_secs(1);
    let mut last_error = None;
    while !stop.load(Ordering::Relaxed) {
        if !loading && !force {
            let debounce_ms = if hint.swap(false, Ordering::Relaxed) {
                50
            } else {
                100
            };
            match rx.recv_timeout(Duration::from_millis(debounce_ms)) {
                Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(Command::Refresh) => force = true,
                Err(mpsc::RecvTimeoutError::Timeout) => (),
            }
        }
        while let Ok(command) = rx.try_recv() {
            match command {
                Command::Stop => return,
                Command::Refresh => force = true,
            }
        }
        if !config.watch
            && !force
            && !loading
            && sent_revision == tail.session().revision
            && sent_offset == tail.reader.byte_offset
            && !reset
        {
            continue;
        }
        match tail.poll(4 * 1024 * 1024) {
            Ok((_, was_reset)) => {
                reset |= was_reset;
                last_error = None;
            }
            Err(e) => {
                if last_error != Some(e.kind()) && tx.try_send(Update::Error(format!("Session temporarily unreadable ({:?}); retrying. Use s to select another session.",e.kind()))).is_ok() {
                    last_error = Some(e.kind());
                }
                loading = false;
                force = false;
                continue;
            }
        }
        let total = tail.reader.len().unwrap_or(tail.reader.byte_offset);
        loading = tail.reader.byte_offset < total;
        force = false;
        if (sent_revision != tail.session().revision
            || sent_offset != tail.reader.byte_offset
            || reset)
            && (!loading || last_sent.elapsed() >= Duration::from_millis(100))
        {
            tail.parser.session.meta.updated_at = std::fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .ok()
                .map(Into::into)
                .or(tail.session().meta.updated_at);
            let update = Update::Batch {
                meta: tail.session().meta.clone(),
                stats: tail.session().parse_stats.clone(),
                revision: tail.session().revision,
                turns: tail
                    .parser
                    .dirty
                    .iter()
                    .map(|&i| (i, tail.session().turns[i].clone()))
                    .collect(),
                reset,
                offset: tail.reader.byte_offset,
                total,
            };
            match tx.try_send(update) {
                Ok(()) => {
                    sent_revision = tail.session().revision;
                    sent_offset = tail.reader.byte_offset;
                    tail.parser.dirty.clear();
                    reset = false;
                    last_sent = Instant::now();
                }
                Err(TrySendError::Disconnected(_)) => break,
                Err(TrySendError::Full(_)) => (),
            }
        }
    }
}
