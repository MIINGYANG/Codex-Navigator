//! Conservative session provenance from metadata, independent of message content.
use crate::{
    domain::{SessionIdentity, SessionKind},
    util::preview,
};
use serde_json::Value;

/// Unknown or malformed sources stay unknown. A fork alone is not an agent spawn.
pub fn parse_identity(payload: &Value) -> SessionIdentity {
    let source = payload.get("source").unwrap_or(&Value::Null);
    let subagent = ["subagent", "sub_agent", "subAgent"]
        .iter()
        .find_map(|key| {
            source
                .get(key)
                .filter(|value| value.is_object() || value.is_string())
        });
    let spawn = subagent
        .and_then(|value| {
            value
                .get("thread_spawn")
                .or_else(|| value.get("threadSpawn"))
        })
        .or_else(|| source.get("thread_spawn"))
        .filter(|value| value.is_object());
    let sources = [Some(payload), spawn, subagent, Some(source)];
    let parent_id = metadata_text(&sources, &["parent_thread_id", "parentThreadId"]);
    let agent_label = metadata_text(&sources, &["agent_nickname", "agentNickname"])
        .or_else(|| metadata_text(&sources, &["agent_path", "agentPath"]))
        .or_else(|| metadata_text(&sources, &["agent_role", "agentRole"]));
    let source_name = source.as_str().or_else(|| {
        source
            .get("type")
            .or_else(|| source.get("kind"))
            .and_then(Value::as_str)
    });
    let kind = if subagent.is_some()
        || spawn.is_some()
        || parent_id.is_some()
        || source_name.is_some_and(is_subagent)
    {
        SessionKind::Subagent
    } else if source_name.is_some_and(is_main)
        || source.as_object().is_some_and(|object| {
            object
                .iter()
                .any(|(key, value)| is_main(key) && (value.is_object() || value.is_string()))
        })
    {
        SessionKind::Main
    } else {
        SessionKind::Unknown
    };
    SessionIdentity {
        kind,
        parent_id,
        agent_label,
    }
}

fn is_subagent(source: &str) -> bool {
    matches!(source, "subagent" | "sub_agent" | "subAgent")
}

fn is_main(source: &str) -> bool {
    matches!(
        source,
        "cli" | "vscode" | "exec" | "appServer" | "app_server"
    )
}

fn metadata_text(sources: &[Option<&Value>], keys: &[&str]) -> Option<String> {
    sources.iter().flatten().find_map(|source| {
        keys.iter().find_map(|key| {
            let text = source.get(key)?.as_str()?;
            // Limit bytes as well as display width: zero-width combining sequences
            // could otherwise retain an arbitrarily large identity label.
            let mut end = text.len().min(1024);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            let text = preview(&text[..end], 256);
            (!text.is_empty()).then_some(text)
        })
    })
}
