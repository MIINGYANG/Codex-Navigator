use crate::util::{preview, sanitize};
use serde_json::Value;

pub fn text(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return sanitize(s);
    }
    if let Some(parts) = v.as_array() {
        return parts
            .iter()
            .filter_map(|p| {
                let kind = p.get("type").and_then(Value::as_str).unwrap_or("");
                if matches!(kind, "input_text" | "output_text" | "text" | "Text") {
                    p.get("text").and_then(Value::as_str).map(sanitize)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    v.get("text")
        .and_then(Value::as_str)
        .map(sanitize)
        .unwrap_or_default()
}

pub fn error(v: &Value) -> bool {
    v.get("exit_code")
        .or_else(|| v.get("exitCode"))
        .and_then(Value::as_i64)
        .is_some_and(|n| n != 0)
        || v.get("is_error")
            .or_else(|| v.get("isError"))
            .and_then(Value::as_bool)
            == Some(true)
        || v.get("error").is_some_and(|e| !e.is_null() && e != false)
        || matches!(
            v.get("status").and_then(Value::as_str),
            Some("failed" | "error" | "Failed")
        )
}

pub fn output(v: &Value) -> (String, bool) {
    let parsed = v
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let obj = parsed.as_ref().unwrap_or(v);
    let failed = error(obj) || obj.get("metadata").is_some_and(error);
    let out = if let Some(s) = obj.as_str() {
        sanitize(s)
    } else if obj.is_array() {
        text(obj)
    } else {
        [
            "output",
            "stdout",
            "stderr",
            "content",
            "text",
            "aggregated_output",
        ]
        .iter()
        .filter_map(|k| obj.get(k))
        .map(text)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
    };
    // Codex's legacy shell envelope, anchored at complete lines, not arbitrary “error” text.
    let structured = has_outcome(obj) || obj.get("metadata").is_some_and(has_outcome);
    let failed = failed
        || (!structured
            && out.lines().any(|line| {
                line.strip_prefix("Process exited with code ")
                    .or_else(|| line.strip_prefix("Process exited with exit code "))
                    .and_then(|n| n.trim().parse::<i64>().ok())
                    .is_some_and(|n| n != 0)
            }));
    (out, failed)
}

fn has_outcome(v: &Value) -> bool {
    [
        "exit_code",
        "exitCode",
        "is_error",
        "isError",
        "error",
        "status",
    ]
    .iter()
    .any(|key| v.get(key).is_some_and(|v| !v.is_null()))
}

pub fn call_summary(name: &str, args: &Value) -> String {
    let parsed = args
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let a = parsed.as_ref().unwrap_or(args);
    if let Some(cmd) = a.get("cmd").or_else(|| a.get("command")) {
        let command = if let Some(v) = cmd.as_array() {
            v.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            text(cmd)
        };
        if !command.is_empty() {
            return format!("$ {}", sanitize(&command));
        }
    }
    if name.contains("apply_patch") {
        let patch = args.as_str().unwrap_or("");
        let files = patch
            .lines()
            .filter_map(|l| {
                ["*** Add File: ", "*** Update File: ", "*** Delete File: "]
                    .iter()
                    .find_map(|p| l.strip_prefix(p))
            })
            .collect::<Vec<_>>();
        return format!("patch {}", preview(&files.join(", "), 512));
    }
    if let Some(path) = a
        .get("path")
        .or_else(|| a.get("file_path"))
        .and_then(Value::as_str)
    {
        return format!("{} {}", sanitize(name), preview(path, 512));
    }
    format!("{} · called", sanitize(name))
}
