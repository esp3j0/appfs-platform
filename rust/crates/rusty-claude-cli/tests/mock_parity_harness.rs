use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mock_anthropic_service::{MockAnthropicService, SCENARIO_PREFIX};
use serde_json::Value;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn clean_env_cli_reaches_mock_anthropic_service_across_scripted_parity_scenarios() {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime should build");
    let server = runtime
        .block_on(MockAnthropicService::spawn())
        .expect("mock service should start");
    let base_url = server.base_url();

    let cases = [
        ScenarioCase {
            name: "streaming_text",
            permission_mode: "read-only",
            allowed_tools: None,
            seed: seed_noop,
            assert: assert_streaming_text,
        },
        ScenarioCase {
            name: "read_file_roundtrip",
            permission_mode: "read-only",
            allowed_tools: Some("read_file"),
            seed: seed_read_fixture,
            assert: assert_read_file_roundtrip,
        },
        ScenarioCase {
            name: "grep_chunk_assembly",
            permission_mode: "read-only",
            allowed_tools: Some("grep_search"),
            seed: seed_grep_fixture,
            assert: assert_grep_chunk_assembly,
        },
        ScenarioCase {
            name: "write_file_allowed",
            permission_mode: "workspace-write",
            allowed_tools: Some("write_file"),
            seed: seed_noop,
            assert: assert_write_file_allowed,
        },
        ScenarioCase {
            name: "write_file_denied",
            permission_mode: "read-only",
            allowed_tools: Some("write_file"),
            seed: seed_noop,
            assert: assert_write_file_denied,
        },
    ];

    for case in cases {
        let workspace = unique_temp_dir(case.name);
        fs::create_dir_all(&workspace).expect("workspace should exist");
        (case.seed)(&workspace);
        let response = run_case(case, &workspace, &base_url);
        (case.assert)(&workspace, &response);
        fs::remove_dir_all(&workspace).expect("workspace cleanup should succeed");
    }

    let captured = runtime.block_on(server.captured_requests());
    assert_eq!(
        captured.len(),
        9,
        "five scenarios should produce nine requests"
    );
    assert!(captured
        .iter()
        .all(|request| request.path == "/v1/messages"));
    assert!(captured.iter().all(|request| request.stream));

    let scenarios = captured
        .iter()
        .map(|request| request.scenario.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        scenarios,
        vec![
            "streaming_text",
            "read_file_roundtrip",
            "read_file_roundtrip",
            "grep_chunk_assembly",
            "grep_chunk_assembly",
            "write_file_allowed",
            "write_file_allowed",
            "write_file_denied",
            "write_file_denied",
        ]
    );
}

#[derive(Clone, Copy)]
struct ScenarioCase {
    name: &'static str,
    permission_mode: &'static str,
    allowed_tools: Option<&'static str>,
    seed: fn(&Path),
    assert: fn(&Path, &Value),
}

fn run_case(case: ScenarioCase, workspace: &Path, base_url: &str) -> Value {
    let config_home = workspace.join("config-home");
    let home = workspace.join("home");
    fs::create_dir_all(config_home.join(".claw")).expect("config home should exist");
    fs::create_dir_all(&home).expect("home should exist");

    let mut command = Command::new(env!("CARGO_BIN_EXE_claw"));
    command
        .current_dir(workspace)
        .env_clear()
        .env("ANTHROPIC_API_KEY", "test-parity-key")
        .env("ANTHROPIC_BASE_URL", base_url)
        .env("CLAW_CONFIG_HOME", &config_home)
        .env("HOME", &home)
        .env("NO_COLOR", "1")
        .args([
            "--model",
            "sonnet",
            "--permission-mode",
            case.permission_mode,
            "--output-format=json",
        ]);

    if let Some(allowed_tools) = case.allowed_tools {
        command.args(["--allowedTools", allowed_tools]);
    }

    let prompt = format!("{SCENARIO_PREFIX}{}", case.name);
    let output = command.arg(prompt).output().expect("claw should launch");
    assert_success(&output);
    serde_json::from_slice(&output.stdout).expect("prompt output should be valid json")
}

fn seed_noop(_: &Path) {}

fn seed_read_fixture(workspace: &Path) {
    fs::write(workspace.join("fixture.txt"), "alpha parity line\n").expect("fixture should write");
}

fn seed_grep_fixture(workspace: &Path) {
    fs::write(
        workspace.join("fixture.txt"),
        "alpha parity line\nbeta line\ngamma parity line\n",
    )
    .expect("grep fixture should write");
}

fn assert_streaming_text(_: &Path, response: &Value) {
    assert_eq!(
        response["message"],
        Value::String("Mock streaming says hello from the parity harness.".to_string())
    );
    assert_eq!(response["iterations"], Value::from(1));
    assert_eq!(response["tool_uses"], Value::Array(Vec::new()));
    assert_eq!(response["tool_results"], Value::Array(Vec::new()));
}

fn assert_read_file_roundtrip(workspace: &Path, response: &Value) {
    assert_eq!(response["iterations"], Value::from(2));
    assert_eq!(
        response["tool_uses"][0]["name"],
        Value::String("read_file".to_string())
    );
    assert_eq!(
        response["tool_uses"][0]["input"],
        Value::String(r#"{"path":"fixture.txt"}"#.to_string())
    );
    assert!(response["message"]
        .as_str()
        .expect("message text")
        .contains("alpha parity line"));
    let output = response["tool_results"][0]["output"]
        .as_str()
        .expect("tool output");
    assert!(output.contains(&workspace.join("fixture.txt").display().to_string()));
    assert!(output.contains("alpha parity line"));
}

fn assert_grep_chunk_assembly(_: &Path, response: &Value) {
    assert_eq!(response["iterations"], Value::from(2));
    assert_eq!(
        response["tool_uses"][0]["name"],
        Value::String("grep_search".to_string())
    );
    assert_eq!(
        response["tool_uses"][0]["input"],
        Value::String(
            r#"{"pattern":"parity","path":"fixture.txt","output_mode":"count"}"#.to_string()
        )
    );
    assert!(response["message"]
        .as_str()
        .expect("message text")
        .contains("2 occurrences"));
    assert_eq!(response["tool_results"][0]["is_error"], Value::Bool(false));
}

fn assert_write_file_allowed(workspace: &Path, response: &Value) {
    assert_eq!(response["iterations"], Value::from(2));
    assert_eq!(
        response["tool_uses"][0]["name"],
        Value::String("write_file".to_string())
    );
    assert!(response["message"]
        .as_str()
        .expect("message text")
        .contains("generated/output.txt"));
    let generated = workspace.join("generated").join("output.txt");
    let contents = fs::read_to_string(&generated).expect("generated file should exist");
    assert_eq!(contents, "created by mock service\n");
    assert_eq!(response["tool_results"][0]["is_error"], Value::Bool(false));
}

fn assert_write_file_denied(workspace: &Path, response: &Value) {
    assert_eq!(response["iterations"], Value::from(2));
    assert_eq!(
        response["tool_uses"][0]["name"],
        Value::String("write_file".to_string())
    );
    let tool_output = response["tool_results"][0]["output"]
        .as_str()
        .expect("tool output");
    assert!(tool_output.contains("requires workspace-write permission"));
    assert_eq!(response["tool_results"][0]["is_error"], Value::Bool(true));
    assert!(response["message"]
        .as_str()
        .expect("message text")
        .contains("denied as expected"));
    assert!(!workspace.join("generated").join("denied.txt").exists());
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn unique_temp_dir(label: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_millis();
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "claw-mock-parity-{label}-{}-{millis}-{counter}",
        std::process::id()
    ))
}
