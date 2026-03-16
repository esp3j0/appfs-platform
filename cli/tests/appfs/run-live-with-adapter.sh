#!/bin/sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
CLI_DIR="$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd)"
REPO_DIR="$(CDPATH= cd -- "$CLI_DIR/.." && pwd)"

APPFS_FIXTURE_DIR="${APPFS_FIXTURE_DIR:-$REPO_DIR/examples/appfs}"
APPFS_LIVE_AGENT_ID="${APPFS_LIVE_AGENT_ID:-appfs-live-$$}"
APPFS_LIVE_MOUNTPOINT="${APPFS_LIVE_MOUNTPOINT:-/tmp/agentfs-appfs-live-$$}"
APPFS_APP_ID="${APPFS_APP_ID:-aiim}"
APPFS_ADAPTER_POLL_MS="${APPFS_ADAPTER_POLL_MS:-100}"
APPFS_TIMEOUT_SEC="${APPFS_TIMEOUT_SEC:-20}"
APPFS_MOUNT_WAIT_SEC="${APPFS_MOUNT_WAIT_SEC:-20}"
APPFS_MOUNT_LOG="${APPFS_MOUNT_LOG:-$CLI_DIR/appfs-mount-live.log}"
APPFS_ADAPTER_LOG="${APPFS_ADAPTER_LOG:-$CLI_DIR/appfs-adapter-live.log}"

MOUNT_PID=""
ADAPTER_PID=""

say() {
    printf '%s\n' "$*"
}

fail() {
    say "FAILED: $*"
    exit 1
}

cleanup() {
    set +e

    if [ -n "${ADAPTER_PID:-}" ] && kill -0 "$ADAPTER_PID" 2>/dev/null; then
        kill "$ADAPTER_PID" 2>/dev/null || true
        wait "$ADAPTER_PID" 2>/dev/null || true
    fi

    if mountpoint -q "$APPFS_LIVE_MOUNTPOINT" 2>/dev/null; then
        fusermount -u "$APPFS_LIVE_MOUNTPOINT" 2>/dev/null || true
    fi

    if [ -n "${MOUNT_PID:-}" ] && kill -0 "$MOUNT_PID" 2>/dev/null; then
        kill "$MOUNT_PID" 2>/dev/null || true
        wait "$MOUNT_PID" 2>/dev/null || true
    fi

    if mountpoint -q "$APPFS_LIVE_MOUNTPOINT" 2>/dev/null; then
        umount "$APPFS_LIVE_MOUNTPOINT" 2>/dev/null || true
    fi

    rmdir "$APPFS_LIVE_MOUNTPOINT" 2>/dev/null || true

    rm -f "$CLI_DIR/.agentfs/${APPFS_LIVE_AGENT_ID}.db"
    rm -f "$CLI_DIR/.agentfs/${APPFS_LIVE_AGENT_ID}.db-shm"
    rm -f "$CLI_DIR/.agentfs/${APPFS_LIVE_AGENT_ID}.db-wal"
}

trap cleanup EXIT INT TERM

command -v cargo >/dev/null 2>&1 || fail "missing command: cargo"
command -v mountpoint >/dev/null 2>&1 || fail "missing command: mountpoint"
command -v fusermount >/dev/null 2>&1 || fail "missing command: fusermount"

[ -d "$APPFS_FIXTURE_DIR" ] || fail "missing fixture directory: $APPFS_FIXTURE_DIR"

cd "$CLI_DIR"

say "Building agentfs binary..."
cargo build >/dev/null
AGENTFS_BIN="$CLI_DIR/target/debug/agentfs"
[ -x "$AGENTFS_BIN" ] || fail "agentfs binary not found: $AGENTFS_BIN"

say "Preparing live mountpoint: $APPFS_LIVE_MOUNTPOINT"
mkdir -p "$APPFS_LIVE_MOUNTPOINT"
if mountpoint -q "$APPFS_LIVE_MOUNTPOINT" 2>/dev/null; then
    fusermount -u "$APPFS_LIVE_MOUNTPOINT" 2>/dev/null || true
fi
find "$APPFS_LIVE_MOUNTPOINT" -mindepth 1 -maxdepth 1 -exec rm -rf {} + 2>/dev/null || true

say "Initializing test agent: $APPFS_LIVE_AGENT_ID"
"$AGENTFS_BIN" init "$APPFS_LIVE_AGENT_ID" --force >/dev/null

say "Starting foreground mount process..."
"$AGENTFS_BIN" mount "$APPFS_LIVE_AGENT_ID" "$APPFS_LIVE_MOUNTPOINT" --backend fuse --foreground >"$APPFS_MOUNT_LOG" 2>&1 &
MOUNT_PID=$!

waited=0
while [ "$waited" -lt "$APPFS_MOUNT_WAIT_SEC" ]; do
    if mountpoint -q "$APPFS_LIVE_MOUNTPOINT" 2>/dev/null; then
        break
    fi
    sleep 1
    waited=$((waited + 1))
done
if ! mountpoint -q "$APPFS_LIVE_MOUNTPOINT" 2>/dev/null; then
    tail -n 80 "$APPFS_MOUNT_LOG" 2>/dev/null || true
    fail "mount did not become ready within ${APPFS_MOUNT_WAIT_SEC}s"
fi

say "Copying AppFS fixture into mounted filesystem..."
cp -a "$APPFS_FIXTURE_DIR"/. "$APPFS_LIVE_MOUNTPOINT"/

say "Starting AppFS adapter runtime..."
"$AGENTFS_BIN" serve appfs --root "$APPFS_LIVE_MOUNTPOINT" --app-id "$APPFS_APP_ID" --poll-ms "$APPFS_ADAPTER_POLL_MS" >"$APPFS_ADAPTER_LOG" 2>&1 &
ADAPTER_PID=$!
sleep 1
if ! kill -0 "$ADAPTER_PID" 2>/dev/null; then
    tail -n 80 "$APPFS_ADAPTER_LOG" 2>/dev/null || true
    fail "adapter failed to start"
fi

say "Running AppFS contract tests against live adapter..."
if ! APPFS_CONTRACT_TESTS=1 APPFS_ROOT="$APPFS_LIVE_MOUNTPOINT" APPFS_APP_ID="$APPFS_APP_ID" APPFS_TIMEOUT_SEC="$APPFS_TIMEOUT_SEC" sh "$CLI_DIR/tests/test-appfs-contract.sh"; then
    say "---- mount log tail ----"
    tail -n 80 "$APPFS_MOUNT_LOG" 2>/dev/null || true
    say "---- adapter log tail ----"
    tail -n 80 "$APPFS_ADAPTER_LOG" 2>/dev/null || true
    fail "live AppFS contract tests failed"
fi

say "LIVE AppFS contract tests passed."
