use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use api::{InputContentBlock, MessageRequest, MessageResponse, OutputContentBlock, Usage};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;

pub const SCENARIO_PREFIX: &str = "PARITY_SCENARIO:";
pub const DEFAULT_MODEL: &str = "claude-sonnet-4-6";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub scenario: String,
    pub stream: bool,
    pub raw_body: String,
}

pub struct MockAnthropicService {
    base_url: String,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    shutdown: Option<oneshot::Sender<()>>,
    join_handle: JoinHandle<()>,
}

impl MockAnthropicService {
    pub async fn spawn() -> io::Result<Self> {
        Self::spawn_on("127.0.0.1:0").await
    }

    pub async fn spawn_on(bind_addr: &str) -> io::Result<Self> {
        let listener = TcpListener::bind(bind_addr).await?;
        let address = listener.local_addr()?;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        let request_state = Arc::clone(&requests);

        let join_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => {
                        let Ok((socket, _)) = accepted else {
                            break;
                        };
                        let request_state = Arc::clone(&request_state);
                        tokio::spawn(async move {
                            let _ = handle_connection(socket, request_state).await;
                        });
                    }
                }
            }
        });

        Ok(Self {
            base_url: format!("http://{address}"),
            requests,
            shutdown: Some(shutdown_tx),
            join_handle,
        })
    }

    #[must_use]
    pub fn base_url(&self) -> String {
        self.base_url.clone()
    }

    pub async fn captured_requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().await.clone()
    }
}

impl Drop for MockAnthropicService {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.join_handle.abort();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scenario {
    StreamingText,
    ReadFileRoundtrip,
    GrepChunkAssembly,
    WriteFileAllowed,
    WriteFileDenied,
}

impl Scenario {
    fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "streaming_text" => Some(Self::StreamingText),
            "read_file_roundtrip" => Some(Self::ReadFileRoundtrip),
            "grep_chunk_assembly" => Some(Self::GrepChunkAssembly),
            "write_file_allowed" => Some(Self::WriteFileAllowed),
            "write_file_denied" => Some(Self::WriteFileDenied),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::StreamingText => "streaming_text",
            Self::ReadFileRoundtrip => "read_file_roundtrip",
            Self::GrepChunkAssembly => "grep_chunk_assembly",
            Self::WriteFileAllowed => "write_file_allowed",
            Self::WriteFileDenied => "write_file_denied",
        }
    }
}

async fn handle_connection(
    mut socket: tokio::net::TcpStream,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
) -> io::Result<()> {
    let (method, path, headers, raw_body) = read_http_request(&mut socket).await?;
    let request: MessageRequest = serde_json::from_str(&raw_body)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let scenario = detect_scenario(&request)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing parity scenario"))?;

    requests.lock().await.push(CapturedRequest {
        method,
        path,
        headers,
        scenario: scenario.name().to_string(),
        stream: request.stream,
        raw_body,
    });

    let response = build_http_response(&request, scenario);
    socket.write_all(response.as_bytes()).await?;
    Ok(())
}

async fn read_http_request(
    socket: &mut tokio::net::TcpStream,
) -> io::Result<(String, String, HashMap<String, String>, String)> {
    let mut buffer = Vec::new();
    let mut header_end = None;

    loop {
        let mut chunk = [0_u8; 1024];
        let read = socket.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = find_header_end(&buffer) {
            header_end = Some(position);
            break;
        }
    }

    let header_end = header_end
        .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "missing http headers"))?;
    let (header_bytes, remaining) = buffer.split_at(header_end);
    let header_text = String::from_utf8(header_bytes.to_vec())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request line"))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing method"))?
        .to_string();
    let path = request_parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing path"))?
        .to_string();

    let mut headers = HashMap::new();
    let mut content_length = 0_usize;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line.split_once(':').ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "malformed http header line")
        })?;
        let value = value.trim().to_string();
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.parse().map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid content-length: {error}"),
                )
            })?;
        }
        headers.insert(name.to_ascii_lowercase(), value);
    }

    let mut body = remaining[4..].to_vec();
    while body.len() < content_length {
        let mut chunk = vec![0_u8; content_length - body.len()];
        let read = socket.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }

    let body = String::from_utf8(body)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    Ok((method, path, headers, body))
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn detect_scenario(request: &MessageRequest) -> Option<Scenario> {
    request.messages.iter().rev().find_map(|message| {
        message.content.iter().rev().find_map(|block| match block {
            InputContentBlock::Text { text } => text
                .split_whitespace()
                .find_map(|token| token.strip_prefix(SCENARIO_PREFIX))
                .and_then(Scenario::parse),
            _ => None,
        })
    })
}

fn latest_tool_result(request: &MessageRequest) -> Option<(String, bool)> {
    request.messages.iter().rev().find_map(|message| {
        message.content.iter().rev().find_map(|block| match block {
            InputContentBlock::ToolResult {
                content, is_error, ..
            } => Some((flatten_tool_result_content(content), *is_error)),
            _ => None,
        })
    })
}

fn flatten_tool_result_content(content: &[api::ToolResultContentBlock]) -> String {
    content
        .iter()
        .map(|block| match block {
            api::ToolResultContentBlock::Text { text } => text.clone(),
            api::ToolResultContentBlock::Json { value } => value.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[allow(clippy::too_many_lines)]
fn build_http_response(request: &MessageRequest, scenario: Scenario) -> String {
    let response = if request.stream {
        let body = build_stream_body(request, scenario);
        return http_response(
            "200 OK",
            "text/event-stream",
            &body,
            &[("x-request-id", request_id_for(scenario))],
        );
    } else {
        build_message_response(request, scenario)
    };

    http_response(
        "200 OK",
        "application/json",
        &serde_json::to_string(&response).expect("message response should serialize"),
        &[("request-id", request_id_for(scenario))],
    )
}

fn build_stream_body(request: &MessageRequest, scenario: Scenario) -> String {
    match scenario {
        Scenario::StreamingText => streaming_text_sse(),
        Scenario::ReadFileRoundtrip => match latest_tool_result(request) {
            Some((tool_output, _)) => final_text_sse(&format!(
                "read_file roundtrip complete: {}",
                extract_read_content(&tool_output)
            )),
            None => tool_use_sse(
                "toolu_read_fixture",
                "read_file",
                &[r#"{"path":"fixture.txt"}"#],
            ),
        },
        Scenario::GrepChunkAssembly => match latest_tool_result(request) {
            Some((tool_output, _)) => final_text_sse(&format!(
                "grep_search matched {} occurrences",
                extract_num_matches(&tool_output)
            )),
            None => tool_use_sse(
                "toolu_grep_fixture",
                "grep_search",
                &[
                    "{\"pattern\":\"par",
                    "ity\",\"path\":\"fixture.txt\"",
                    ",\"output_mode\":\"count\"}",
                ],
            ),
        },
        Scenario::WriteFileAllowed => match latest_tool_result(request) {
            Some((tool_output, _)) => final_text_sse(&format!(
                "write_file succeeded: {}",
                extract_file_path(&tool_output)
            )),
            None => tool_use_sse(
                "toolu_write_allowed",
                "write_file",
                &[r#"{"path":"generated/output.txt","content":"created by mock service\n"}"#],
            ),
        },
        Scenario::WriteFileDenied => match latest_tool_result(request) {
            Some((tool_output, _)) => {
                final_text_sse(&format!("write_file denied as expected: {tool_output}"))
            }
            None => tool_use_sse(
                "toolu_write_denied",
                "write_file",
                &[r#"{"path":"generated/denied.txt","content":"should not exist\n"}"#],
            ),
        },
    }
}

fn build_message_response(request: &MessageRequest, scenario: Scenario) -> MessageResponse {
    match scenario {
        Scenario::StreamingText => text_message_response(
            "msg_streaming_text",
            "Mock streaming says hello from the parity harness.",
        ),
        Scenario::ReadFileRoundtrip => match latest_tool_result(request) {
            Some((tool_output, _)) => text_message_response(
                "msg_read_file_final",
                &format!(
                    "read_file roundtrip complete: {}",
                    extract_read_content(&tool_output)
                ),
            ),
            None => tool_message_response(
                "msg_read_file_tool",
                "toolu_read_fixture",
                "read_file",
                json!({"path": "fixture.txt"}),
            ),
        },
        Scenario::GrepChunkAssembly => match latest_tool_result(request) {
            Some((tool_output, _)) => text_message_response(
                "msg_grep_final",
                &format!(
                    "grep_search matched {} occurrences",
                    extract_num_matches(&tool_output)
                ),
            ),
            None => tool_message_response(
                "msg_grep_tool",
                "toolu_grep_fixture",
                "grep_search",
                json!({"pattern": "parity", "path": "fixture.txt", "output_mode": "count"}),
            ),
        },
        Scenario::WriteFileAllowed => match latest_tool_result(request) {
            Some((tool_output, _)) => text_message_response(
                "msg_write_allowed_final",
                &format!("write_file succeeded: {}", extract_file_path(&tool_output)),
            ),
            None => tool_message_response(
                "msg_write_allowed_tool",
                "toolu_write_allowed",
                "write_file",
                json!({"path": "generated/output.txt", "content": "created by mock service\n"}),
            ),
        },
        Scenario::WriteFileDenied => match latest_tool_result(request) {
            Some((tool_output, _)) => text_message_response(
                "msg_write_denied_final",
                &format!("write_file denied as expected: {tool_output}"),
            ),
            None => tool_message_response(
                "msg_write_denied_tool",
                "toolu_write_denied",
                "write_file",
                json!({"path": "generated/denied.txt", "content": "should not exist\n"}),
            ),
        },
    }
}

fn request_id_for(scenario: Scenario) -> &'static str {
    match scenario {
        Scenario::StreamingText => "req_streaming_text",
        Scenario::ReadFileRoundtrip => "req_read_file_roundtrip",
        Scenario::GrepChunkAssembly => "req_grep_chunk_assembly",
        Scenario::WriteFileAllowed => "req_write_file_allowed",
        Scenario::WriteFileDenied => "req_write_file_denied",
    }
}

fn http_response(status: &str, content_type: &str, body: &str, headers: &[(&str, &str)]) -> String {
    let mut extra_headers = String::new();
    for (name, value) in headers {
        use std::fmt::Write as _;
        write!(&mut extra_headers, "{name}: {value}\r\n").expect("header write should succeed");
    }
    format!(
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\n{extra_headers}content-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn text_message_response(id: &str, text: &str) -> MessageResponse {
    MessageResponse {
        id: id.to_string(),
        kind: "message".to_string(),
        role: "assistant".to_string(),
        content: vec![OutputContentBlock::Text {
            text: text.to_string(),
        }],
        model: DEFAULT_MODEL.to_string(),
        stop_reason: Some("end_turn".to_string()),
        stop_sequence: None,
        usage: Usage {
            input_tokens: 10,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            output_tokens: 6,
        },
        request_id: None,
    }
}

fn tool_message_response(
    id: &str,
    tool_id: &str,
    tool_name: &str,
    input: Value,
) -> MessageResponse {
    MessageResponse {
        id: id.to_string(),
        kind: "message".to_string(),
        role: "assistant".to_string(),
        content: vec![OutputContentBlock::ToolUse {
            id: tool_id.to_string(),
            name: tool_name.to_string(),
            input,
        }],
        model: DEFAULT_MODEL.to_string(),
        stop_reason: Some("tool_use".to_string()),
        stop_sequence: None,
        usage: Usage {
            input_tokens: 10,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            output_tokens: 3,
        },
        request_id: None,
    }
}

fn streaming_text_sse() -> String {
    let mut body = String::new();
    append_sse(
        &mut body,
        "message_start",
        json!({
            "type": "message_start",
            "message": {
                "id": "msg_streaming_text",
                "type": "message",
                "role": "assistant",
                "content": [],
                "model": DEFAULT_MODEL,
                "stop_reason": null,
                "stop_sequence": null,
                "usage": usage_json(11, 0)
            }
        }),
    );
    append_sse(
        &mut body,
        "content_block_start",
        json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {"type": "text", "text": ""}
        }),
    );
    append_sse(
        &mut body,
        "content_block_delta",
        json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "text_delta", "text": "Mock streaming "}
        }),
    );
    append_sse(
        &mut body,
        "content_block_delta",
        json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "text_delta", "text": "says hello from the parity harness."}
        }),
    );
    append_sse(
        &mut body,
        "content_block_stop",
        json!({
            "type": "content_block_stop",
            "index": 0
        }),
    );
    append_sse(
        &mut body,
        "message_delta",
        json!({
            "type": "message_delta",
            "delta": {"stop_reason": "end_turn", "stop_sequence": null},
            "usage": usage_json(11, 8)
        }),
    );
    append_sse(&mut body, "message_stop", json!({"type": "message_stop"}));
    body
}

fn tool_use_sse(tool_id: &str, tool_name: &str, partial_json_chunks: &[&str]) -> String {
    let mut body = String::new();
    append_sse(
        &mut body,
        "message_start",
        json!({
            "type": "message_start",
            "message": {
                "id": format!("msg_{tool_id}"),
                "type": "message",
                "role": "assistant",
                "content": [],
                "model": DEFAULT_MODEL,
                "stop_reason": null,
                "stop_sequence": null,
                "usage": usage_json(12, 0)
            }
        }),
    );
    append_sse(
        &mut body,
        "content_block_start",
        json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {
                "type": "tool_use",
                "id": tool_id,
                "name": tool_name,
                "input": {}
            }
        }),
    );
    for chunk in partial_json_chunks {
        append_sse(
            &mut body,
            "content_block_delta",
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "input_json_delta", "partial_json": chunk}
            }),
        );
    }
    append_sse(
        &mut body,
        "content_block_stop",
        json!({
            "type": "content_block_stop",
            "index": 0
        }),
    );
    append_sse(
        &mut body,
        "message_delta",
        json!({
            "type": "message_delta",
            "delta": {"stop_reason": "tool_use", "stop_sequence": null},
            "usage": usage_json(12, 4)
        }),
    );
    append_sse(&mut body, "message_stop", json!({"type": "message_stop"}));
    body
}

fn final_text_sse(text: &str) -> String {
    let mut body = String::new();
    append_sse(
        &mut body,
        "message_start",
        json!({
            "type": "message_start",
            "message": {
                "id": unique_message_id(),
                "type": "message",
                "role": "assistant",
                "content": [],
                "model": DEFAULT_MODEL,
                "stop_reason": null,
                "stop_sequence": null,
                "usage": usage_json(14, 0)
            }
        }),
    );
    append_sse(
        &mut body,
        "content_block_start",
        json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {"type": "text", "text": ""}
        }),
    );
    append_sse(
        &mut body,
        "content_block_delta",
        json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "text_delta", "text": text}
        }),
    );
    append_sse(
        &mut body,
        "content_block_stop",
        json!({
            "type": "content_block_stop",
            "index": 0
        }),
    );
    append_sse(
        &mut body,
        "message_delta",
        json!({
            "type": "message_delta",
            "delta": {"stop_reason": "end_turn", "stop_sequence": null},
            "usage": usage_json(14, 7)
        }),
    );
    append_sse(&mut body, "message_stop", json!({"type": "message_stop"}));
    body
}

#[allow(clippy::needless_pass_by_value)]
fn append_sse(buffer: &mut String, event: &str, payload: Value) {
    use std::fmt::Write as _;
    writeln!(buffer, "event: {event}").expect("event write should succeed");
    writeln!(buffer, "data: {payload}").expect("payload write should succeed");
    buffer.push('\n');
}

fn usage_json(input_tokens: u32, output_tokens: u32) -> Value {
    json!({
        "input_tokens": input_tokens,
        "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": 0,
        "output_tokens": output_tokens
    })
}

fn unique_message_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    format!("msg_{nanos}")
}

fn extract_read_content(tool_output: &str) -> String {
    serde_json::from_str::<Value>(tool_output)
        .ok()
        .and_then(|value| {
            value
                .get("file")
                .and_then(|file| file.get("content"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| tool_output.trim().to_string())
}

#[allow(clippy::cast_possible_truncation)]
fn extract_num_matches(tool_output: &str) -> usize {
    serde_json::from_str::<Value>(tool_output)
        .ok()
        .and_then(|value| value.get("numMatches").and_then(Value::as_u64))
        .unwrap_or(0) as usize
}

fn extract_file_path(tool_output: &str) -> String {
    serde_json::from_str::<Value>(tool_output)
        .ok()
        .and_then(|value| {
            value
                .get("filePath")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| tool_output.trim().to_string())
}
