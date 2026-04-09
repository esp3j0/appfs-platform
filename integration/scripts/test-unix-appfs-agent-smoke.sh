#!/usr/bin/env bash
# AppFS + appfs-agent Unix smoke test
# Linux uses FUSE; macOS uses NFS over localhost.

set -euo pipefail

AGENT_ID="appfs-agent-smoke-unix"
WORKSPACE_NAME="workspace"
TIMEOUT_SEC=90
RUN_PROMPT=0
KEEP_LOGS=0
SKIP_BUILD=0
BACKEND=""
MOUNTPOINT=""

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
APPFS_CLI_DIR="${REPO_ROOT}/appfs/cli"
APPFS_AGENT_RUST_DIR="${REPO_ROOT}/appfs-agent/rust"

TMP_ROOT="${TMPDIR:-/tmp}"
LOG_DIR="$(mktemp -d "${TMP_ROOT%/}/appfs-agent-unix-smoke-XXXXXX")"
CARGO_CACHE_ROOT="${TMP_ROOT%/}/appfs-platform-cargo-targets"
APPFS_TARGET_DIR="${CARGO_CACHE_ROOT}/appfs-cli-unix"
CLAW_TARGET_DIR="${CARGO_CACHE_ROOT}/appfs-agent-rust-unix"
APPFS_BIN="${APPFS_TARGET_DIR}/debug/agentfs"
CLAW_BIN="${CLAW_TARGET_DIR}/debug/claw"
APPFS_PID=""
HAD_FAILURE=0

usage() {
    cat <<'EOF'
Usage: test-unix-appfs-agent-smoke.sh [options]

Options:
  --agent-id <id>         Agent/database id to initialize
  --mountpoint <path>     Mountpoint directory
  --workspace <name>      Workspace directory name inside the mount
  --backend <fuse|nfs>    Override backend (default: fuse on Linux, nfs on macOS)
  --timeout-sec <sec>     Mount bootstrap timeout in seconds
  --run-prompt            Also run one real appfs-agent prompt
  --skip-build            Reuse existing cargo build outputs
  --keep-logs             Preserve temp logs on success
  --help                  Show this message
EOF
}

section() { printf '\n==== %s ====\n' "$*"; }
ok() { printf '[ok] %s\n' "$*"; }
warn() { printf '[warn] %s\n' "$*"; }

dump_logs() {
    if [[ ! -d "${LOG_DIR}" ]]; then
        return
    fi
    printf '\nLogs preserved at %s\n' "${LOG_DIR}" >&2
    local path
    for path in \
        "${LOG_DIR}/appfs-build.log" \
        "${LOG_DIR}/claw-build.log" \
        "${LOG_DIR}/appfs-up.stdout.log" \
        "${LOG_DIR}/appfs-up.stderr.log" \
        "${LOG_DIR}/claw-status.log" \
        "${LOG_DIR}/claw-prompt.log"; do
        if [[ -f "${path}" ]]; then
            printf '\n--- tail: %s ---\n' "${path}" >&2
            tail -n 40 "${path}" >&2 || true
        fi
    done
}

fail() {
    HAD_FAILURE=1
    printf '[fail] %s\n' "$*" >&2
    dump_logs
    exit 1
}

require_cmd() {
    local cmd="$1"
    command -v "${cmd}" >/dev/null 2>&1 || fail "Required command not found: ${cmd}"
}

cleanup_path() {
    local path="$1"
    if [[ -e "${path}" ]]; then
        rm -rf "${path}" 2>/dev/null || true
    fi
}

process_alive() {
    local pid="$1"
    kill -0 "${pid}" >/dev/null 2>&1
}

is_mounted() {
    local path="$1"
    if command -v mountpoint >/dev/null 2>&1; then
        mountpoint -q "${path}"
        return
    fi
    mount | grep -F " on ${path} " >/dev/null 2>&1
}

attempt_unmount() {
    local path="$1"

    case "${BACKEND}" in
        fuse)
            if is_mounted "${path}"; then
                if command -v fusermount3 >/dev/null 2>&1; then
                    fusermount3 -u "${path}" >/dev/null 2>&1 || fusermount3 -uz "${path}" >/dev/null 2>&1 || true
                elif command -v fusermount >/dev/null 2>&1; then
                    fusermount -u "${path}" >/dev/null 2>&1 || fusermount -uz "${path}" >/dev/null 2>&1 || true
                fi
            fi
            ;;
        nfs)
            if is_mounted "${path}"; then
                umount "${path}" >/dev/null 2>&1 || /sbin/umount "${path}" >/dev/null 2>&1 || true
            fi
            ;;
    esac
}

cleanup() {
    if [[ -n "${APPFS_PID}" ]] && process_alive "${APPFS_PID}"; then
        kill -INT "${APPFS_PID}" >/dev/null 2>&1 || true
        for _ in $(seq 1 20); do
            if ! process_alive "${APPFS_PID}"; then
                break
            fi
            sleep 0.25
        done
        if process_alive "${APPFS_PID}"; then
            kill -TERM "${APPFS_PID}" >/dev/null 2>&1 || true
        fi
        wait "${APPFS_PID}" >/dev/null 2>&1 || true
    fi

    if [[ -n "${MOUNTPOINT}" ]]; then
        attempt_unmount "${MOUNTPOINT}"
        cleanup_path "${MOUNTPOINT}"
    fi

    cleanup_path "${APPFS_CLI_DIR}/.agentfs/${AGENT_ID}.db"
    cleanup_path "${APPFS_CLI_DIR}/.agentfs/${AGENT_ID}.db-shm"
    cleanup_path "${APPFS_CLI_DIR}/.agentfs/${AGENT_ID}.db-wal"

    if [[ "${KEEP_LOGS}" -eq 0 && "${HAD_FAILURE}" -eq 0 ]]; then
        cleanup_path "${LOG_DIR}"
    fi
}

wait_until() {
    local description="$1"
    local timeout_sec="$2"
    shift 2

    local deadline=$((SECONDS + timeout_sec))
    while (( SECONDS < deadline )); do
        if "$@"; then
            return 0
        fi
        sleep 0.25
    done

    fail "Timed out waiting for ${description}"
}

assert_file_contains() {
    local path="$1"
    local regex="$2"
    local description="$3"
    if grep -Eq "${regex}" "${path}"; then
        ok "${description}"
    else
        fail "${description}"
    fi
}

assert_text_contains() {
    local haystack="$1"
    local needle="$2"
    local description="$3"
    if [[ "${haystack}" == *"${needle}"* ]]; then
        ok "${description}"
    else
        fail "${description}"
    fi
}

detect_defaults() {
    local uname_s
    uname_s="$(uname -s)"
    case "${uname_s}" in
        Linux)
            : "${BACKEND:=fuse}"
            : "${MOUNTPOINT:=/tmp/appfs-agent-smoke-linux}"
            ;;
        Darwin)
            : "${BACKEND:=nfs}"
            : "${MOUNTPOINT:=/tmp/appfs-agent-smoke-macos}"
            ;;
        *)
            fail "Unsupported Unix platform: ${uname_s}"
            ;;
    esac

    case "${uname_s}:${BACKEND}" in
        Linux:fuse|Darwin:nfs)
            ;;
        Linux:*)
            fail "Linux smoke test only supports --backend fuse"
            ;;
        Darwin:*)
            fail "macOS smoke test only supports --backend nfs"
            ;;
    esac
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --agent-id)
                AGENT_ID="$2"
                shift 2
                ;;
            --mountpoint)
                MOUNTPOINT="$2"
                shift 2
                ;;
            --workspace)
                WORKSPACE_NAME="$2"
                shift 2
                ;;
            --backend)
                BACKEND="$2"
                shift 2
                ;;
            --timeout-sec)
                TIMEOUT_SEC="$2"
                shift 2
                ;;
            --run-prompt)
                RUN_PROMPT=1
                shift
                ;;
            --skip-build)
                SKIP_BUILD=1
                shift
                ;;
            --keep-logs)
                KEEP_LOGS=1
                shift
                ;;
            --help|-h)
                usage
                exit 0
                ;;
            *)
                fail "Unknown argument: $1"
                ;;
        esac
    done
}

build_binaries() {
    if [[ "${SKIP_BUILD}" -eq 1 ]]; then
        ok "Skipping cargo build and reusing existing binaries"
        [[ -x "${APPFS_BIN}" ]] || fail "Missing AppFS binary at ${APPFS_BIN}"
        [[ -x "${CLAW_BIN}" ]] || fail "Missing claw binary at ${CLAW_BIN}"
        return
    fi

    mkdir -p "${CARGO_CACHE_ROOT}"

    section "Build Test Binaries"
    (
        cd "${APPFS_CLI_DIR}"
        cargo build --target-dir "${APPFS_TARGET_DIR}" --bin agentfs
    ) >"${LOG_DIR}/appfs-build.log" 2>&1 || fail "Failed to build AppFS CLI"
    [[ -x "${APPFS_BIN}" ]] || fail "Built AppFS binary not found: ${APPFS_BIN}"
    ok "Built AppFS CLI binary ${APPFS_BIN}"

    (
        cd "${APPFS_AGENT_RUST_DIR}"
        cargo build \
            --target-dir "${CLAW_TARGET_DIR}" \
            --manifest-path "${APPFS_AGENT_RUST_DIR}/Cargo.toml" \
            -p rusty-claude-cli
    ) >"${LOG_DIR}/claw-build.log" 2>&1 || fail "Failed to build appfs-agent CLI"
    [[ -x "${CLAW_BIN}" ]] || fail "Built claw binary not found: ${CLAW_BIN}"
    ok "Built appfs-agent CLI binary ${CLAW_BIN}"
}

preflight_checks() {
    require_cmd cargo
    require_cmd sed
    require_cmd grep
    require_cmd mktemp

    case "${BACKEND}" in
        fuse)
            [[ -e /dev/fuse ]] || fail "/dev/fuse is required for Linux FUSE smoke tests"
            if ! command -v fusermount3 >/dev/null 2>&1 && ! command -v fusermount >/dev/null 2>&1; then
                fail "fusermount3 or fusermount is required for Linux FUSE smoke tests"
            fi
            ;;
        nfs)
            [[ -x /sbin/mount_nfs || -x /usr/sbin/mount_nfs ]] || fail "mount_nfs is required for macOS NFS smoke tests"
            ;;
    esac
}

main() {
    trap cleanup EXIT
    parse_args "$@"
    detect_defaults
    preflight_checks
    build_binaries

    local db_path="${APPFS_CLI_DIR}/.agentfs/${AGENT_ID}.db"
    local control_dir="${MOUNTPOINT}/_appfs"
    local manifest_path="${MOUNTPOINT}/.well-known/appfs/runtime.json"
    local workspace_dir="${MOUNTPOINT}/${WORKSPACE_NAME}"
    local hello_path="${workspace_dir}/hello.txt"
    local runtime_session_id=""

    attempt_unmount "${MOUNTPOINT}"
    cleanup_path "${MOUNTPOINT}"
    mkdir -p "${MOUNTPOINT}"
    cleanup_path "${db_path}"
    cleanup_path "${db_path}-shm"
    cleanup_path "${db_path}-wal"

    section "Init AppFS"
    (
        cd "${APPFS_CLI_DIR}"
        "${APPFS_BIN}" init "${AGENT_ID}" --force
    ) >"${LOG_DIR}/appfs-init.log" 2>&1 || fail "Failed to initialize AppFS database"
    [[ -f "${db_path}" ]] || fail "Created database not found: ${db_path}"
    ok "Created database ${db_path}"

    section "Start AppFS"
    (
        cd "${APPFS_CLI_DIR}"
        exec "${APPFS_BIN}" appfs up "${db_path}" "${MOUNTPOINT}" --backend "${BACKEND}" --auto-unmount
    ) >"${LOG_DIR}/appfs-up.stdout.log" 2>"${LOG_DIR}/appfs-up.stderr.log" &
    APPFS_PID=$!

    wait_until "AppFS mount bootstrap" "${TIMEOUT_SEC}" bash -c "
        kill -0 ${APPFS_PID} 2>/dev/null &&
        test -e '${control_dir}/register_app.act' &&
        test -e '${control_dir}/list_apps.act'
    "
    ok "AppFS mount is ready"

    [[ -f "${manifest_path}" ]] || fail "AppFS runtime manifest missing: ${manifest_path}"
    ok "AppFS runtime manifest exists at ${manifest_path}"
    assert_file_contains "${manifest_path}" '"schema_version"[[:space:]]*:[[:space:]]*1' "AppFS runtime manifest schema_version is 1"
    assert_file_contains "${manifest_path}" '"runtime_kind"[[:space:]]*:[[:space:]]*"appfs"' "AppFS runtime manifest runtime_kind is appfs"
    assert_file_contains "${manifest_path}" '"multi_agent_mode"[[:space:]]*:[[:space:]]*"shared_mount_distinct_attach"' "AppFS runtime manifest multi_agent_mode is shared_mount_distinct_attach"
    assert_file_contains "${manifest_path}" '"multi_agent_attach"[[:space:]]*:[[:space:]]*true' "AppFS runtime manifest advertises multi_agent_attach capability"

    runtime_session_id="$(
        tr -d '\n' <"${manifest_path}" |
            sed -n 's/.*"runtime_session_id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p'
    )"
    [[ -n "${runtime_session_id}" ]] || fail "AppFS runtime manifest runtime_session_id is not populated"
    ok "AppFS runtime manifest runtime_session_id is populated"

    section "Prepare Workspace"
    mkdir -p "${workspace_dir}"
    printf 'hello from appfs mount\n' >"${hello_path}"
    [[ -f "${hello_path}" ]] || fail "Created mounted workspace file ${hello_path}"
    ok "Created mounted workspace file ${hello_path}"
    assert_file_contains "${hello_path}" 'hello from appfs mount' "Mounted hello.txt is readable from the Unix shell"

    section "Run appfs-agent Status"
    local status_output
    status_output="$(
        cd "${workspace_dir}" &&
            "${CLAW_BIN}" status 2>&1 | tee "${LOG_DIR}/claw-status.log"
    )" || fail "claw status failed"
    assert_text_contains "${status_output}" "Workspace" "claw status rendered a workspace snapshot inside the mount"
    assert_text_contains "${status_output}" "${workspace_dir}" "claw status reported the mounted workspace path"
    assert_text_contains "${status_output}" "Attach source     manifest" "claw status attached through the AppFS runtime manifest"
    assert_text_contains "${status_output}" "Runtime session   ${runtime_session_id}" "claw status reported the shared AppFS runtime session"
    assert_text_contains "${status_output}" "Multi-agent mode  shared_mount_distinct_attach" "claw status reported the shared multi-agent attach mode"

    if [[ "${RUN_PROMPT}" -eq 1 ]]; then
        section "Run appfs-agent Prompt"
        [[ -n "${ANTHROPIC_API_KEY:-}" ]] || fail "--run-prompt requires ANTHROPIC_API_KEY"
        local prompt_output
        prompt_output="$(
            cd "${workspace_dir}" &&
                "${CLAW_BIN}" --dangerously-skip-permissions prompt \
                    "Confirm the current working directory, list files in the current directory, and print the exact contents of hello.txt. Do not modify any files." \
                    2>&1 | tee "${LOG_DIR}/claw-prompt.log"
        )" || fail "claw prompt failed"
        assert_text_contains "${prompt_output}" "hello from appfs mount" "claw prompt surfaced hello.txt content from the mounted workspace"
    fi

    printf '\nAppFS + appfs-agent Unix smoke test passed.\n'
}

main "$@"
