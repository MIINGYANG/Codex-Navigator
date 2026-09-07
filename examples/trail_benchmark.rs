//! 合成 100 MiB / 1000 问题的端到端 API 基准；不接触真实 Codex 数据。
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Read, Write},
    net::TcpStream,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn get(address: &str, token: &str, path: &str) -> Result<Value> {
    let mut stream = TcpStream::connect(address)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    write!(stream,"GET {path} HTTP/1.1\r\nHost: {address}\r\nX-Codex-Nav-Token: {token}\r\nConnection: close\r\n\r\n")?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    let split = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("HTTP headers")?;
    anyhow::ensure!(response.starts_with(b"HTTP/1.1 200"), "HTTP API failed");
    Ok(serde_json::from_slice(&response[split + 4..])?)
}

fn main() -> Result<()> {
    let root = tempfile::tempdir()?;
    let sessions = root.path().join("sessions/2026/09/08");
    std::fs::create_dir_all(&sessions)?;
    let path = sessions.join("rollout-trail-benchmark.jsonl");
    let mut writer = BufWriter::new(File::create(&path)?);
    writeln!(
        writer,
        "{}",
        json!({"type":"session_meta","payload":{"id":"trail-benchmark","source":"cli","cwd":"/synthetic/project"}})
    )?;
    let output = "Synthetic output.".repeat(6554);
    for index in 0..1000 {
        writeln!(
            writer,
            "{}",
            json!({"type":"turn_context","payload":{"turn_id":format!("t{index}")}})
        )?;
        writeln!(
            writer,
            "{}",
            json!({"type":"event_msg","payload":{"type":"user_message","message":format!("Synthetic question {index}: verify local question navigation")}})
        )?;
        writeln!(
            writer,
            "{}",
            json!({"type":"response_item","payload":{"type":"function_call_output","call_id":format!("c{index}"),"output":output}})
        )?;
        writeln!(
            writer,
            "{}",
            json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":format!("t{index}")}})
        )?;
    }
    writer.flush()?;
    let bytes = writer.get_ref().metadata()?.len();
    drop(writer);
    let binary = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "target/release/codex-trail".into());
    let started = Instant::now();
    let mut child = ChildGuard(
        Command::new(binary)
            .args(["--port", "0", "--no-open", "--codex-home"])
            .arg(root.path())
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?,
    );
    let mut reader = BufReader::new(child.0.stdout.take().context("stdout")?);
    let mut line = String::new();
    let url = loop {
        line.clear();
        anyhow::ensure!(reader.read_line(&mut line)? > 0, "server exited before URL");
        if let Some(pos) = line.find("http://127.0.0.1:") {
            break line[pos..].trim().to_owned();
        }
    };
    let (address, token) = url
        .strip_prefix("http://")
        .context("local URL")?
        .split_once("/trail/#token=")
        .context("token URL")?;
    let deadline = Instant::now() + Duration::from_secs(60);
    let key = loop {
        let data = get(address, token, "/api/sessions?all=1")?;
        if data["loading"] == false && data["sessions"].as_array().is_some_and(|s| !s.is_empty()) {
            break data["sessions"][0]["key"]
                .as_str()
                .context("session key")?
                .to_owned();
        }
        anyhow::ensure!(Instant::now() < deadline, "discovery timeout");
        thread::sleep(Duration::from_millis(10));
    };
    let discovery_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut first = None;
    let mut first_count = 0;
    loop {
        let graph = get(address, token, &format!("/api/trail/session/{key}"))?;
        let count = graph["nodes"].as_array().context("nodes")?.len();
        if count > 0 && first.is_none() {
            first = Some(started.elapsed().as_secs_f64() * 1000.0);
            first_count = count;
        }
        if graph["loading"] == false && count == 1000 {
            break;
        }
        anyhow::ensure!(Instant::now() < deadline, "graph timeout");
        thread::sleep(Duration::from_millis(10));
    }
    println!("bytes={bytes} mib={:.2} questions=1000 discovery_ms={discovery_ms:.2} first_graph_ms={:.2} first_graph_questions={first_count} complete_graph_ms={:.2}",bytes as f64/1048576.0,first.context("first graph")?,started.elapsed().as_secs_f64()*1000.0);
    Ok(())
}
