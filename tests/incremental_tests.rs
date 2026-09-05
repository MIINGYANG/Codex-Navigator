use codex_navigator::parser::Tail;
use serde_json::json;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};
use tempfile::TempDir;

fn prompt(n: usize) -> String {
    format!(
        "{}\n",
        json!({"type":"event_msg","payload":{"type":"user_message","message":format!("prompt {n}")}})
    )
}

fn append(path: &Path, bytes: &[u8]) {
    OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(bytes)
        .unwrap();
}

fn drain(tail: &mut Tail, budget: usize) {
    while tail.poll(budget).unwrap().0 != 0 {}
}

#[test]
fn incremental_append_preserves_offsets_and_never_replays_records() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("rollout.jsonl");
    let first = (0..20).map(prompt).collect::<String>();
    fs::write(&path, &first).unwrap();
    let mut tail = Tail::open(&path, 4096, 36).unwrap();
    drain(&mut tail, 97);
    assert_eq!(tail.reader.byte_offset, first.len() as u64);
    assert_eq!(tail.session().turns.len(), 20);
    assert_eq!(tail.session().parse_stats.records, 20);
    assert_eq!(tail.poll(4096).unwrap(), (0, false));
    let second = (20..30).map(prompt).collect::<String>();
    append(&path, second.as_bytes());
    drain(&mut tail, 113);
    assert_eq!(tail.reader.byte_offset, (first.len() + second.len()) as u64);
    assert_eq!(tail.session().turns.len(), 30);
    assert_eq!(tail.session().parse_stats.records, 30);
    for (n, turn) in tail.session().turns.iter().enumerate() {
        assert_eq!(turn.prompt.text, format!("prompt {n}"));
    }
}

#[test]
fn incremental_partial_json_and_split_utf8_wait_for_newline() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("partial.jsonl");
    let record = format!(
        "{}\n",
        json!({"type":"event_msg","payload":{"type":"user_message","message":"中文"}})
    );
    let split = record.find('中').unwrap() + 1;
    fs::write(&path, &record.as_bytes()[..split]).unwrap();
    let mut tail = Tail::open(&path, 1024, 36).unwrap();
    drain(&mut tail, 3);
    assert!(tail.session().turns.is_empty());
    assert_eq!(tail.session().parse_stats.malformed_records, 0);
    assert_eq!(tail.reader.pending_len(), split);
    append(&path, &record.as_bytes()[split..record.len() - 1]);
    drain(&mut tail, 11);
    assert!(tail.session().turns.is_empty());
    append(&path, b"\n");
    drain(&mut tail, 100);
    assert_eq!(tail.session().turns[0].prompt.text, "中文");
    assert_eq!(tail.reader.pending_len(), 0);
    assert_eq!(tail.reader.byte_offset, record.len() as u64);
}

#[test]
fn incremental_oversized_media_is_bounded_across_appends_and_counted_once() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("image.jsonl");
    fs::write(&path, b"{\"image\":\"").unwrap();
    let mut tail = Tail::open(&path, 1024, 36).unwrap();
    let chunk = [b'A'; 64 * 1024];
    for _ in 0..64 {
        append(&path, &chunk);
        drain(&mut tail, 701);
        assert!(tail.reader.pending_len() <= 1024);
        assert_eq!(tail.session().parse_stats.skipped_oversize_records, 0);
    }
    append(&path, b"\"}\n");
    append(&path, prompt(1).as_bytes());
    drain(&mut tail, 4096);
    assert_eq!(tail.session().parse_stats.skipped_oversize_records, 1);
    assert_eq!(tail.session().parse_stats.malformed_records, 0);
    assert_eq!(tail.session().turns.len(), 1);
    assert_eq!(tail.reader.pending_len(), 0);
}

#[test]
fn incremental_oversize_boundary_accepts_exact_limit() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("boundary.jsonl");
    let first = prompt(1);
    let limit = first.len() - 1;
    fs::write(&path, &first).unwrap();
    let mut tail = Tail::open(&path, limit, 36).unwrap();
    drain(&mut tail, 7);
    assert_eq!(tail.session().turns.len(), 1);
    assert_eq!(tail.session().parse_stats.skipped_oversize_records, 0);
    append(&path, format!("{} \n", first.trim_end()).as_bytes());
    drain(&mut tail, 7);
    assert_eq!(tail.session().parse_stats.skipped_oversize_records, 1);
}

#[test]
fn incremental_truncated_path_safely_reloads_and_discards_pending() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("truncate.jsonl");
    fs::write(&path, format!("{}{}{{\"partial\":", prompt(1), prompt(2))).unwrap();
    let mut tail = Tail::open(&path, 4096, 36).unwrap();
    drain(&mut tail, 4096);
    assert!(tail.reader.pending_len() > 0);
    fs::write(&path, prompt(3)).unwrap();
    assert!(tail.poll(4096).unwrap().1);
    assert_eq!(tail.session().turns.len(), 1);
    assert_eq!(tail.session().turns[0].prompt.text, "prompt 3");
    assert_eq!(tail.reader.pending_len(), 0);
    assert_eq!(tail.reader.byte_offset, prompt(3).len() as u64);
}

#[test]
fn incremental_replaced_path_reloads_even_if_new_file_is_larger() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("replace.jsonl");
    fs::write(&path, prompt(1)).unwrap();
    let mut tail = Tail::open(&path, 4096, 36).unwrap();
    drain(&mut tail, 4096);
    // 保留旧文件，模拟原子替换，不删除测试以外的任何文件。
    fs::rename(&path, temp.path().join("retained-original.jsonl")).unwrap();
    fs::write(&path, format!("{}{}", prompt(2), prompt(3))).unwrap();
    assert!(tail.poll(4096).unwrap().1);
    assert_eq!(tail.session().turns.len(), 2);
    assert_eq!(tail.session().turns[0].prompt.text, "prompt 2");
}

#[test]
fn incremental_same_size_rewrite_is_detected() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("rewrite.jsonl");
    fs::write(&path, prompt(1)).unwrap();
    let mut tail = Tail::open(&path, 4096, 36).unwrap();
    drain(&mut tail, 4096);
    fs::write(&path, prompt(2)).unwrap();
    // 明确改变 mtime，避免低精度文件系统让测试依赖执行速度。
    let file = OpenOptions::new().write(true).open(&path).unwrap();
    file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(2))
        .unwrap();
    assert!(tail.poll(4096).unwrap().1);
    assert_eq!(tail.session().turns[0].prompt.text, "prompt 2");
}

#[test]
fn incremental_malformed_commit_counted_only_after_newline_and_next_record_survives() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("malformed.jsonl");
    fs::write(&path, b"broken").unwrap();
    let mut tail = Tail::open(&path, 4096, 36).unwrap();
    drain(&mut tail, 4096);
    assert_eq!(tail.session().parse_stats.malformed_records, 0);
    append(&path, b"\n");
    append(&path, prompt(1).as_bytes());
    drain(&mut tail, 4096);
    assert_eq!(tail.session().parse_stats.malformed_records, 1);
    assert_eq!(tail.session().turns.len(), 1);
}

#[test]
fn incremental_reader_never_changes_input_and_respects_poll_budget() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("readonly.jsonl");
    let input = format!("\u{feff}{}bad\n{}", prompt(1), prompt(2));
    fs::write(&path, &input).unwrap();
    let mut tail = Tail::open(&path, 4096, 36).unwrap();
    assert_eq!(tail.poll(0).unwrap(), (0, false));
    loop {
        let (bytes, reset) = tail.poll(5).unwrap();
        assert!(bytes <= 5);
        assert!(!reset);
        if bytes == 0 {
            break;
        }
    }
    assert_eq!(fs::read(&path).unwrap(), input.as_bytes());
    assert_eq!(tail.session().turns.len(), 2);
    assert_eq!(tail.session().parse_stats.malformed_records, 1);
}

#[test]
fn incremental_stored_partial_and_oversize_fixtures_recover() {
    let temp = TempDir::new().unwrap();
    let partial_path = temp.path().join("partial-fixture.jsonl");
    // 文本 fixture 保留 Git 惯用末尾换行；模拟写入者尚未提交此换行。
    let partial = include_str!("fixtures/truncated-last-line.jsonl").trim_end_matches('\n');
    fs::write(&partial_path, partial).unwrap();
    let mut tail = Tail::open(&partial_path, 4096, 36).unwrap();
    drain(&mut tail, 4096);
    assert_eq!(tail.session().turns.len(), 1);
    assert_eq!(tail.session().parse_stats.malformed_records, 0);
    append(&partial_path, b" completed\"}}\n");
    drain(&mut tail, 4096);
    assert_eq!(tail.session().turns[1].prompt.text, "partial completed");

    let oversized_path = temp.path().join("oversized-fixture.jsonl");
    fs::write(
        &oversized_path,
        include_str!("fixtures/huge-record-marker.jsonl"),
    )
    .unwrap();
    let mut tail = Tail::open(&oversized_path, 256, 36).unwrap();
    drain(&mut tail, 73);
    assert_eq!(tail.session().parse_stats.skipped_oversize_records, 1);
    assert_eq!(
        tail.session().turns[0].prompt.text,
        "after oversized record"
    );
}
