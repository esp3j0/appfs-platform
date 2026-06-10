use anyhow::{Context, Result};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use super::action_dispatcher;
use super::errors::ERR_INVALID_PAYLOAD;
use super::shared::{
    action_template_matches, boundary_probe_from_bytes, classify_multiline_json_payload,
    collect_files_with_suffix, decode_jsonl_line, extract_client_token, has_odd_unescaped_quotes,
    is_safe_action_rel_path, is_transient_action_sink_busy, template_specificity,
    write_pretty_json_file, MultilineRecoveryOutcome,
};
use super::{ActionCursorDoc, ActionCursorState, ActionSpec, InputMode, ProcessOutcome};

pub(super) struct ActionConsumerConfig {
    pub(super) app_id: String,
    pub(super) session_id: String,
    pub(super) app_dir: PathBuf,
    pub(super) action_cursors_path: PathBuf,
    pub(super) action_specs: Vec<ActionSpec>,
    pub(super) actionline_strict: bool,
    pub(super) cursor_label: &'static str,
    pub(super) log_label: &'static str,
    pub(super) fixed_action_order: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub(super) struct ActionInvocation {
    pub(super) rel_path: String,
    pub(super) normalized_path: String,
    pub(super) request_id: String,
    pub(super) payload_json: String,
    pub(super) client_token: Option<String>,
}

pub(super) struct InvalidActionPayload {
    pub(super) rel_path: String,
    pub(super) normalized_path: String,
    pub(super) request_id: String,
    pub(super) code: &'static str,
    pub(super) message: String,
    pub(super) client_token: Option<String>,
}

pub(super) trait ActionInvocationHandler {
    fn handle_action_invocation(
        &mut self,
        invocation: ActionInvocation,
        spec: &ActionSpec,
    ) -> Result<ProcessOutcome>;

    fn handle_invalid_action_payload(
        &mut self,
        invalid: InvalidActionPayload,
    ) -> Result<ProcessOutcome> {
        eprintln!(
            "AppFS action consumer rejected action payload for {}: validation={} reason={}",
            invalid.normalized_path, invalid.code, invalid.message
        );
        Ok(ProcessOutcome::Consumed)
    }
}

pub(super) struct ActionDrainResult {
    pub(super) action_cursors: HashMap<String, ActionCursorState>,
    pub(super) cursor_dirty: bool,
}

pub(super) fn drain_action_sinks(
    config: ActionConsumerConfig,
    action_cursors: &mut HashMap<String, ActionCursorState>,
    handler: &mut impl ActionInvocationHandler,
) -> Result<()> {
    let result = drain_action_sinks_uncommitted(&config, action_cursors, handler)?;
    if result.cursor_dirty {
        commit_action_cursors(&config, action_cursors, result.action_cursors)?;
    }
    Ok(())
}

pub(super) fn drain_action_sinks_uncommitted(
    config: &ActionConsumerConfig,
    action_cursors: &HashMap<String, ActionCursorState>,
    handler: &mut impl ActionInvocationHandler,
) -> Result<ActionDrainResult> {
    let mut next_action_cursors = action_cursors.clone();
    let mut actions = collect_action_files_for_config(&config)?;
    if config.fixed_action_order.is_none() {
        actions.sort();
    }

    let mut cursor_dirty = false;
    for action_path in actions {
        cursor_dirty |=
            process_action_sink(&config, &mut next_action_cursors, &action_path, handler)?;
    }

    Ok(ActionDrainResult {
        action_cursors: next_action_cursors,
        cursor_dirty,
    })
}

pub(super) fn commit_action_cursors(
    config: &ActionConsumerConfig,
    action_cursors: &mut HashMap<String, ActionCursorState>,
    next_action_cursors: HashMap<String, ActionCursorState>,
) -> Result<()> {
    let doc = ActionCursorDoc {
        actions: next_action_cursors.clone(),
    };
    write_pretty_json_file(&config.action_cursors_path, &doc, config.cursor_label)?;
    *action_cursors = next_action_cursors;
    Ok(())
}

pub(super) fn collect_action_files(app_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    collect_files_with_suffix(app_dir, ".act", &mut out)?;
    Ok(out)
}

pub(super) fn load_action_cursors(path: &Path) -> Result<HashMap<String, ActionCursorState>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let content =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
    let doc: ActionCursorDoc = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    Ok(doc.actions)
}

pub(super) fn stable_action_line_client_token(
    app_id: &str,
    session_id: &str,
    rel: &str,
    offset: u64,
    payload: &str,
) -> String {
    // Action retries must keep a stable idempotency key even when the caller
    // intentionally keeps the JSONL action payload terse.
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let offset = offset.to_string();
    for part in [
        app_id.as_bytes(),
        session_id.as_bytes(),
        rel.as_bytes(),
        offset.as_bytes(),
        payload.as_bytes(),
    ] {
        for byte in part {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= u64::from(b'|');
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("appfs-act-{hash:016x}")
}

pub(super) fn new_request_id() -> String {
    let uuid = Uuid::new_v4().simple().to_string();
    format!("req-{}", &uuid[..8])
}

fn collect_action_files_for_config(config: &ActionConsumerConfig) -> Result<Vec<PathBuf>> {
    if let Some(order) = config.fixed_action_order.as_ref() {
        return Ok(order
            .iter()
            .map(|rel| config.app_dir.join(rel))
            .filter(|path| path.exists())
            .collect());
    }

    collect_action_files(&config.app_dir)
}

fn process_action_sink(
    config: &ActionConsumerConfig,
    action_cursors: &mut HashMap<String, ActionCursorState>,
    action_path: &Path,
    handler: &mut impl ActionInvocationHandler,
) -> Result<bool> {
    let rel = rel_path_for_log(&config.app_dir, action_path);
    if !is_safe_action_rel_path(&rel) {
        eprintln!("{} rejected unsafe action path: {rel}", config.log_label);
        return Ok(false);
    }

    let Some(spec) = find_action_spec(&config.action_specs, &rel).cloned() else {
        eprintln!("{} ignored undeclared action path: {rel}", config.log_label);
        return Ok(false);
    };

    let mut cursor = action_cursors.get(&rel).cloned().unwrap_or_default();
    let original_cursor = cursor.clone();
    let file_len = match fs::metadata(action_path) {
        Ok(meta) => meta.len(),
        Err(err) => {
            if is_transient_action_sink_busy(&err) {
                return Ok(false);
            }
            eprintln!(
                "{} rejected action payload for {rel}: validation={ERR_INVALID_PAYLOAD} reason={}",
                config.log_label, err
            );
            return Ok(false);
        }
    };

    if cursor.offset > file_len {
        eprintln!(
            "{} HIGH: illegal action sink truncation detected for {rel}: offset={} file_len={file_len}; skipping rewritten content and waiting for future append",
            config.log_label, cursor.offset
        );
        cursor.offset = file_len;
        cursor.boundary_probe = None;
        cursor.pending_multiline_eof_len = None;
    } else if cursor.offset == file_len {
        return Ok(false);
    }

    let bytes = match fs::read(action_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            if is_transient_action_sink_busy(&err) {
                return Ok(false);
            }
            eprintln!(
                "{} rejected action payload for {rel}: validation={ERR_INVALID_PAYLOAD} reason={}",
                config.log_label, err
            );
            return Ok(false);
        }
    };
    let file_len = bytes.len() as u64;

    if cursor.offset > file_len {
        eprintln!(
            "{} HIGH: illegal action sink truncation detected for {rel}: offset={} file_len={file_len}; skipping rewritten content and waiting for future append",
            config.log_label, cursor.offset
        );
        cursor.offset = file_len;
        cursor.boundary_probe = None;
        cursor.pending_multiline_eof_len = None;
    } else if cursor.offset > 0 && cursor.boundary_probe.is_some() {
        let expected = cursor.boundary_probe.as_deref().unwrap_or_default();
        let current = boundary_probe_from_bytes(&bytes, cursor.offset);
        if current.as_deref() != Some(expected) {
            eprintln!(
                "{} HIGH: illegal action sink overwrite detected for {rel}: offset={} (probe mismatch); skipping rewritten content and waiting for future append",
                config.log_label, cursor.offset
            );
            cursor.offset = file_len;
            cursor.boundary_probe = boundary_probe_from_bytes(&bytes, cursor.offset);
            cursor.pending_multiline_eof_len = None;
        }
    }

    let mut position = cursor.offset as usize;
    while position < bytes.len() {
        while position < bytes.len() && bytes[position] == 0 {
            // PowerShell 5 `>>` (Out-File) may leave a trailing UTF-16 newline NUL byte
            // after our `\n` delimiter split. Consume it so the cursor can progress.
            position += 1;
            cursor.offset = position as u64;
            cursor.boundary_probe = boundary_probe_from_bytes(&bytes, cursor.offset);
            cursor.pending_multiline_eof_len = None;
        }
        if position >= bytes.len() {
            break;
        }

        let Some(rel_idx) = bytes[position..].iter().position(|b| *b == b'\n') else {
            break;
        };
        let line_end = position + rel_idx + 1;
        let line_bytes = &bytes[position..line_end];
        let request_id = new_request_id();
        let mut payload = match decode_jsonl_line(line_bytes, position == 0) {
            Ok(Some(line)) => line,
            Ok(None) => {
                cursor.offset = line_end as u64;
                cursor.boundary_probe = boundary_probe_from_bytes(&bytes, cursor.offset);
                cursor.pending_multiline_eof_len = None;
                position = line_end;
                continue;
            }
            Err(reason) => {
                let invalid = InvalidActionPayload {
                    rel_path: rel.clone(),
                    normalized_path: normalized_action_path(&rel),
                    request_id,
                    code: ERR_INVALID_PAYLOAD,
                    message: reason,
                    client_token: None,
                };
                match handler.handle_invalid_action_payload(invalid)? {
                    ProcessOutcome::Consumed => {
                        cursor.offset = line_end as u64;
                        cursor.boundary_probe = boundary_probe_from_bytes(&bytes, cursor.offset);
                        cursor.pending_multiline_eof_len = None;
                        position = line_end;
                    }
                    ProcessOutcome::RetryPending => {
                        eprintln!(
                            "{} deferred invalid action retry for {rel} at offset={}",
                            config.log_label, cursor.offset
                        );
                        break;
                    }
                }
                continue;
            }
        };
        let mut payload_line_end = line_end;
        let mut client_token_override = None;

        if matches!(spec.input_mode, InputMode::Json)
            && serde_json::from_str::<JsonValue>(&payload).is_err()
            && has_odd_unescaped_quotes(&payload)
        {
            match classify_multiline_json_payload(&bytes, &payload, line_end, &spec) {
                Some(MultilineRecoveryOutcome::Recovered {
                    merged_payload,
                    merged_line_end,
                    consumed_lines,
                }) => {
                    eprintln!(
                        "{} normalized shell-expanded newline for {rel}: consumed_lines={consumed_lines}",
                        config.log_label
                    );
                    payload = merged_payload;
                    payload_line_end = merged_line_end;
                    cursor.pending_multiline_eof_len = None;
                }
                Some(MultilineRecoveryOutcome::PendingAtEof) => {
                    let pending_len = bytes.len() as u64;
                    if cursor.pending_multiline_eof_len == Some(pending_len) {
                        cursor.pending_multiline_eof_len = None;
                    } else {
                        eprintln!(
                            "{} deferred incomplete multiline payload for {rel} at offset={}",
                            config.log_label, cursor.offset
                        );
                        cursor.pending_multiline_eof_len = Some(pending_len);
                        break;
                    }
                }
                None => {
                    cursor.pending_multiline_eof_len = None;
                }
            }
        } else {
            cursor.pending_multiline_eof_len = None;
        }

        match action_dispatcher::normalize_actionline_payload(&payload, config.actionline_strict) {
            Ok(Some(parsed)) => {
                client_token_override = Some(parsed.client_token);
                payload = parsed.payload_json;
            }
            Ok(None) => {}
            Err(validation) => {
                let invalid = InvalidActionPayload {
                    rel_path: rel.clone(),
                    normalized_path: normalized_action_path(&rel),
                    request_id,
                    code: validation.code,
                    message: validation.reason.to_string(),
                    client_token: None,
                };
                match handler.handle_invalid_action_payload(invalid)? {
                    ProcessOutcome::Consumed => {
                        cursor.offset = payload_line_end as u64;
                        cursor.boundary_probe = boundary_probe_from_bytes(&bytes, cursor.offset);
                        cursor.pending_multiline_eof_len = None;
                        position = payload_line_end;
                    }
                    ProcessOutcome::RetryPending => {
                        eprintln!(
                            "{} deferred invalid action retry for {rel} at offset={}",
                            config.log_label, cursor.offset
                        );
                        break;
                    }
                }
                continue;
            }
        }

        let client_token = client_token_override
            .or_else(|| extract_client_token(&payload))
            .or_else(|| {
                Some(stable_action_line_client_token(
                    &config.app_id,
                    &config.session_id,
                    &rel,
                    cursor.offset,
                    &payload,
                ))
            });
        let invocation = ActionInvocation {
            rel_path: rel.clone(),
            normalized_path: normalized_action_path(&rel),
            request_id,
            payload_json: payload,
            client_token,
        };

        match handler.handle_action_invocation(invocation, &spec)? {
            ProcessOutcome::Consumed => {
                cursor.offset = payload_line_end as u64;
                cursor.boundary_probe = boundary_probe_from_bytes(&bytes, cursor.offset);
                cursor.pending_multiline_eof_len = None;
                position = payload_line_end;
            }
            ProcessOutcome::RetryPending => {
                eprintln!(
                    "{} deferred action retry for {rel} at offset={}",
                    config.log_label, cursor.offset
                );
                break;
            }
        }
    }

    let changed = cursor != original_cursor;
    if changed {
        action_cursors.insert(rel, cursor);
    }
    Ok(changed)
}

fn find_action_spec<'a>(action_specs: &'a [ActionSpec], rel_path: &str) -> Option<&'a ActionSpec> {
    action_specs
        .iter()
        .filter(|spec| action_template_matches(&spec.template, rel_path))
        .max_by_key(|spec| template_specificity(&spec.template))
}

fn rel_path_for_log(app_dir: &Path, action_path: &Path) -> String {
    action_path
        .strip_prefix(app_dir)
        .unwrap_or(action_path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn normalized_action_path(rel_path: &str) -> String {
    format!("/{rel_path}")
}
