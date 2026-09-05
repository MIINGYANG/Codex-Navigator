//! 只读检查真实 rollout；只输出统计，不输出内容、路径、cwd 或 session ID。
use clap::Parser;
use codex_navigator::{domain::TurnItem, parser::Tail};
use std::{
    path::PathBuf,
    process::ExitCode,
    time::{Duration, Instant},
};

const BUDGET: usize = 4 * 1024 * 1024;

#[derive(Parser)]
#[command(about = "Read-only rollout audit; prints aggregate counts without session content")]
struct Args {
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u64).range(0..=86400))]
    follow_seconds: u64,
    #[arg(required = true, value_name = "ROLLOUT")]
    paths: Vec<PathBuf>,
}

struct Observed {
    index: usize,
    tail: Tail,
    appended_bytes: usize,
    added_turns: usize,
    resets: usize,
}

fn report(index: usize, tail: &Tail, elapsed: Duration) {
    let session = tail.session();
    let mut agents = 0;
    let mut calls = 0;
    let mut outputs = 0;
    let mut files = 0;
    let mut notices = 0;
    for item in session.turns.iter().flat_map(|turn| &turn.items) {
        match item {
            TurnItem::AgentMessage { .. } => agents += 1,
            TurnItem::ToolCall { .. } => calls += 1,
            TurnItem::ToolOutput { .. } => outputs += 1,
            TurnItem::FileActivity { .. } => files += 1,
            TurnItem::Notice { .. } => notices += 1,
        }
    }
    let stats = &session.parse_stats;
    println!(
        "file={index} bytes={} turns={} agents={agents} tool_calls={calls} tool_outputs={outputs} file_items={files} notices={notices} records={} malformed={} oversized={} omitted_text_bytes={} pending_bytes={} elapsed_ms={:.2}",
        tail.reader.byte_offset,
        session.turns.len(),
        stats.records,
        stats.malformed_records,
        stats.skipped_oversize_records,
        stats.omitted_text_bytes,
        tail.reader.pending_len(),
        elapsed.as_secs_f64() * 1000.0,
    );
}

fn main() -> ExitCode {
    let args = Args::parse();
    let mut observed = Vec::new();
    let mut failed = false;
    for (index, path) in args.paths.iter().enumerate() {
        let started = Instant::now();
        let loaded = (|| -> std::io::Result<Tail> {
            let mut tail = Tail::open(path, BUDGET, 36)?;
            while tail.poll(BUDGET)?.0 != 0 {}
            Ok(tail)
        })();
        match loaded {
            Ok(tail) => {
                report(index + 1, &tail, started.elapsed());
                observed.push(Observed {
                    index: index + 1,
                    tail,
                    appended_bytes: 0,
                    added_turns: 0,
                    resets: 0,
                });
            }
            Err(error) => {
                eprintln!("file={} read_failed={:?}", index + 1, error.kind());
                failed = true;
            }
        }
    }
    if args.follow_seconds > 0 && !observed.is_empty() {
        let started = Instant::now();
        let duration = Duration::from_secs(args.follow_seconds);
        while started.elapsed() < duration {
            for item in &mut observed {
                let old_turns = item.tail.session().turns.len();
                match item.tail.poll(BUDGET) {
                    Ok((bytes, reset)) => {
                        if reset {
                            item.resets += 1;
                        } else {
                            item.appended_bytes += bytes;
                            item.added_turns +=
                                item.tail.session().turns.len().saturating_sub(old_turns);
                        }
                    }
                    Err(error) => {
                        eprintln!("file={} follow_failed={:?}", item.index, error.kind());
                        failed = true;
                        break;
                    }
                }
            }
            if failed {
                break;
            }
            std::thread::sleep(
                Duration::from_millis(100).min(duration.saturating_sub(started.elapsed())),
            );
        }
        for item in &observed {
            println!(
                "file={} follow_elapsed_ms={:.2} appended_bytes={} added_turns={} resets={}",
                item.index,
                started.elapsed().as_secs_f64() * 1000.0,
                item.appended_bytes,
                item.added_turns,
                item.resets
            );
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
