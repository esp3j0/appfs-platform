//! Optional debug-dump module (gated by `debug-dump` feature).
//! Writes raw ApiRequest payloads to a JSONL file for the dashboard.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

use runtime::{ApiRequest, ConversationMessage, MessageRole};
use serde_json::json;

/// Write agent metadata file on startup.
pub fn write_agent_meta(
    dir: &str,
    agent_name: &str,
    principal_id: &str,
    session_id: &str,
    model: &str,
    pid: u32,
    session_jsonl_path: &str,
) {
    let dir = Path::new(dir);
    let _ = fs::create_dir_all(dir);
    let meta = json!({
        "agent_name": agent_name,
        "principal_id": principal_id,
        "session_id": session_id,
        "model": model,
        "pid": pid,
        "started_at_ms": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        "session_jsonl_path": session_jsonl_path,
    });
    let path = dir.join(format!("agent-meta-{agent_name}.json"));
    if let Ok(mut file) = File::create(&path) {
        let _ = writeln!(file, "{}", serde_json::to_string_pretty(&meta).unwrap_or_default());
    }
}

/// Append a single ApiRequest dump to the agent's debug JSONL file.
///
/// Uses `ConversationMessage::to_json().render()` to serialise each message
/// using the runtime crate's own `JsonValue` type, then parses back into
/// `serde_json::Value` for the final output record.
pub fn write_request(dir: &str, session_id: &str, request: &ApiRequest) {
    let dir = Path::new(dir);
    let _ = fs::create_dir_all(dir);
    let path = dir.join(format!("{session_id}.jsonl"));

    let system_prompt = request.system_prompt.join("\n\n");
    let messages: Vec<serde_json::Value> = request
        .messages
        .iter()
        .map(serialize_message)
        .collect();

    let record = json!({
        "type": "message_request",
        "timestamp_ms": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        "request_index": 0,
        "model": request.model_override.clone().unwrap_or_default(),
        "max_tokens": 0,
        "system_prompt": system_prompt,
        "system_prompt_length": system_prompt.len(),
        "message_count": messages.len(),
        "messages": messages,
        "tools_count": 0,
        "tools": [],
        "stream": true,
        "reasoning_effort": request.reasoning_effort,
    });

    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(
            file,
            "{}",
            serde_json::to_string(&record).unwrap_or_default()
        );
    }
}

/// Serialise a single `ConversationMessage` to `serde_json::Value`.
///
/// Uses the runtime's own `to_json()` which returns a private `JsonValue`,
/// then `.render()` produces a valid JSON string we parse into serde.
fn serialize_message(msg: &ConversationMessage) -> serde_json::Value {
    let rendered = msg.to_json().render();
    serde_json::from_str(&rendered).unwrap_or_else(|_| {
        // Fallback: manual serialisation using role string
        let role_str = match msg.role {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Tool => "tool",
        };
        json!({
            "role": role_str,
            "uuid": "",
            "blocks": [],
        })
    })
}
