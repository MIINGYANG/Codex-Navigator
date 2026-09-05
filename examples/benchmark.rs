//! 使用临时合成数据测量解析/搜索与超大行保护，不读取真实会话。
use codex_navigator::{domain::TurnItem, index::SearchIndex, parser::Tail};
use serde_json::json;
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    time::Instant,
};
use tempfile::TempDir;

const MIB: usize = 1024 * 1024;
const TURN_COUNT: usize = 4096;

fn parse(path: &Path) -> std::io::Result<Tail> {
    let mut tail = Tail::open(path, 4 * MIB, 36)?;
    while tail.poll(4 * MIB)?.0 != 0 {
        assert!(tail.reader.pending_len() <= 4 * MIB);
    }
    Ok(tail)
}

fn typical(path: &Path) -> anyhow::Result<()> {
    let mut writer = BufWriter::new(File::create(path)?);
    let assistant = json!({"type":"event_msg","payload":{"type":"agent_message","message":"Synthetic visible output. ".repeat(512)}}).to_string();
    for n in 0..TURN_COUNT {
        writeln!(
            writer,
            "{}",
            json!({"type":"event_msg","payload":{"type":"user_message","message":format!("Synthetic request {n}: 修复 authentication and run cargo tests")}})
        )?;
        writeln!(writer, "{assistant}")?;
        writeln!(
            writer,
            "{}",
            json!({"type":"response_item","payload":{"type":"function_call","name":"exec_command","call_id":format!("command-{n}"),"arguments":{"cmd":"cargo test"}}})
        )?;
        writeln!(
            writer,
            "{}",
            json!({"type":"response_item","payload":{"type":"function_call_output","call_id":format!("command-{n}"),"output":{"exit_code":0,"output":"Synthetic command succeeded"}}})
        )?;
        writeln!(
            writer,
            "{}",
            json!({"type":"event_msg","payload":{"type":"task_complete"}})
        )?;
    }
    writer.flush()?;
    let size = writer.get_ref().metadata()?.len();
    drop(writer);
    let started = Instant::now();
    let tail = parse(path)?;
    let elapsed = started.elapsed();
    assert_eq!(tail.session().turns.len(), TURN_COUNT);
    assert_eq!(tail.session().parse_stats.records, TURN_COUNT * 5);
    assert_eq!(tail.session().parse_stats.malformed_records, 0);
    assert_eq!(tail.session().parse_stats.skipped_oversize_records, 0);
    assert_eq!(tail.session().parse_stats.omitted_text_bytes, 0);
    assert_eq!(tail.reader.byte_offset, size);
    assert!(tail
        .session()
        .turns
        .iter()
        .all(|turn| turn.activity.commands == 1 && turn.activity.errors == 0));
    let mut index = SearchIndex::default();
    let index_started = Instant::now();
    index.sync(&tail.session().turns);
    let index_elapsed = index_started.elapsed();
    let mut maximum_search = std::time::Duration::ZERO;
    for query in ["authentication", "修复", "authtn", "unmatchable-query"] {
        let started = Instant::now();
        let results = index.search(query);
        maximum_search = maximum_search.max(started.elapsed());
        assert_eq!(
            results.len(),
            if query == "unmatchable-query" {
                0
            } else {
                TURN_COUNT
            }
        );
    }
    let retained: usize = tail
        .session()
        .turns
        .iter()
        .map(|turn| {
            turn.prompt.text.len()
                + turn
                    .items
                    .iter()
                    .map(|item| match item {
                        TurnItem::AgentMessage { text, .. } | TurnItem::Notice { text } => {
                            text.len()
                        }
                        TurnItem::ToolCall { name, summary } => name.len() + summary.len(),
                        TurnItem::ToolOutput { summary, .. } => summary.len(),
                        TurnItem::FileActivity { path, kind } => path.len() + kind.len(),
                    })
                    .sum::<usize>()
        })
        .sum();
    assert!(retained <= 65 * MIB);
    println!("typical bytes={size} turns={TURN_COUNT} parse_ms={:.2} throughput_mib_s={:.2} index_ms={:.2} max_search_ms={:.3} retained_text_bytes={retained}", elapsed.as_secs_f64() * 1000.0, size as f64 / MIB as f64 / elapsed.as_secs_f64(), index_elapsed.as_secs_f64() * 1000.0, maximum_search.as_secs_f64() * 1000.0);
    Ok(())
}

fn oversized(path: &Path) -> anyhow::Result<()> {
    let mut writer = BufWriter::new(File::create(path)?);
    writer.write_all(b"{\"type\":\"response_item\",\"payload\":{\"image\":\"")?;
    let chunk = [b'A'; 64 * 1024];
    for _ in 0..(256 * MIB / chunk.len()) {
        writer.write_all(&chunk)?;
    }
    writer.write_all(b"\"}}\n")?;
    writeln!(
        writer,
        "{}",
        json!({"type":"event_msg","payload":{"type":"user_message","message":"after huge synthetic media"}})
    )?;
    writer.flush()?;
    let size = writer.get_ref().metadata()?.len();
    drop(writer);
    let started = Instant::now();
    let tail = parse(path)?;
    let elapsed = started.elapsed();
    assert_eq!(tail.session().turns.len(), 1);
    assert_eq!(tail.session().parse_stats.skipped_oversize_records, 1);
    assert_eq!(tail.session().parse_stats.malformed_records, 0);
    assert_eq!(tail.reader.byte_offset, size);
    assert_eq!(tail.reader.pending_len(), 0);
    println!(
        "oversized bytes={size} turns=1 skipped=1 parse_ms={:.2} throughput_mib_s={:.2}",
        elapsed.as_secs_f64() * 1000.0,
        size as f64 / MIB as f64 / elapsed.as_secs_f64()
    );
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let temporary = TempDir::new()?;
    typical(&temporary.path().join("typical.jsonl"))?;
    oversized(&temporary.path().join("oversized.jsonl"))?;
    Ok(())
}
