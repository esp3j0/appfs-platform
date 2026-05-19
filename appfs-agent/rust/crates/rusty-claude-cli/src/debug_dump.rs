//! Optional debug-dump module (gated by `debug-dump` feature).
//! Writes the actual MessageRequest sent to the LLM API to a companion
//! `.debug.jsonl` file for the dashboard.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

use api::MessageRequest;
use runtime::ConversationMessage;
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
    let _ = std::fs::create_dir_all(dir);
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
        let _ = writeln!(
            file,
            "{}",
            serde_json::to_string_pretty(&meta).unwrap_or_default()
        );
    }
}

/// Append the actual MessageRequest sent to the LLM to a companion `.debug.jsonl`.
///
/// The session JSONL is periodically rewritten by `save_to_path()` which
/// would erase any appended records it doesn't know about.  Instead, we
/// write to `<session-stem>.debug.jsonl` — a separate append-only file
/// that survives session snapshots.
///
/// Because this is called *after* `MessageRequest` construction, all fields
/// (model, max_tokens, tools, messages, system_prompt) reflect the actual
/// data sent to the API.
pub fn write_message_request(
    session_jsonl_path: &Path,
    request_index: usize,
    message_request: &MessageRequest,
) {
    // Derive companion path: session-xxx.jsonl → session-xxx.debug.jsonl
    let debug_path = session_jsonl_path.with_extension("debug.jsonl");

    // Serialise the MessageRequest directly — it already has the exact data
    // sent to the API (model, max_tokens, tools, messages, system, etc.).
    let mut record = match serde_json::to_value(message_request) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[debug-dump] serialise error: {e}");
            return;
        }
    };

    // Wrap with metadata
    let map = record.as_object_mut();
    if let Some(m) = map {
        m.insert("type".to_string(), json!("message_request"));
        m.insert(
            "timestamp_ms".to_string(),
            json!(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64),
        );
        m.insert("request_index".to_string(), json!(request_index));
    }

    match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&debug_path)
    {
        Ok(mut file) => {
            if let Err(e) = writeln!(
                file,
                "{}",
                serde_json::to_string(&record).unwrap_or_default()
            ) {
                eprintln!("[debug-dump] write error: {e}");
            }
        }
        Err(e) => {
            eprintln!("[debug-dump] open error for {}: {e}", debug_path.display());
        }
    }
}

/// Archive messages that are about to be removed by compaction.
///
/// Called *before* `build_compaction_result()` replaces `session.messages`.
/// The removed messages are written to the companion `.debug.jsonl` file so
/// the dashboard can reconstruct the full timeline including pre-compaction
/// history.
///
/// Each archived message is written as a separate JSONL record with
/// `type: "compaction_archive"`, enabling the dashboard to distinguish them
/// from regular session messages and debug-dump records.
pub fn write_compaction_archive(
    session_jsonl_path: &Path,
    removed_messages: &[ConversationMessage],
    compaction_count: u32,
) {
    if removed_messages.is_empty() {
        return;
    }

    let debug_path = session_jsonl_path.with_extension("debug.jsonl");
    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&debug_path)
    {
        Ok(mut file) => {
            // Write a header record marking the compaction event
            let header = json!({
                "type": "compaction_boundary",
                "timestamp_ms": timestamp_ms,
                "compaction_count": compaction_count,
                "archived_message_count": removed_messages.len(),
            });
            if let Err(e) = writeln!(
                file,
                "{}",
                serde_json::to_string(&header).unwrap_or_default()
            ) {
                eprintln!("[debug-dump] compaction boundary write error: {e}");
                return;
            }

            // Write each removed message as an archive record
            for msg in removed_messages {
                // Use the session's native JSON format (same format as session JSONL)
                // so the dashboard can reuse its existing message parsing.
                let msg_json = msg.to_json().render();
                let record = json!({
                    "type": "compaction_archive",
                    "timestamp_ms": timestamp_ms,
                    "message": serde_json::from_str::<serde_json::Value>(&msg_json)
                        .unwrap_or_else(|_| json!(msg_json)),
                });
                if let Err(e) = writeln!(
                    file,
                    "{}",
                    serde_json::to_string(&record).unwrap_or_default()
                ) {
                    eprintln!("[debug-dump] compaction archive write error: {e}");
                }
            }
        }
        Err(e) => {
            eprintln!(
                "[debug-dump] compaction archive open error for {}: {e}",
                debug_path.display()
            );
        }
    }
}
