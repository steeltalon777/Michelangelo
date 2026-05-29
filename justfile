# Michelangelo Headless Core — verification commands

_default:
    just --list

# Check formatting without modifying files
fmt-check:
    cargo fmt --all -- --check

# Apply formatting to all workspace files
fmt:
    cargo fmt --all

# Run clippy with warnings as errors
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Run all workspace tests
test:
    cargo test --workspace

# Full verification: formatting + clippy + tests
verify: fmt-check clippy test

# Run JSONL stdio smoke tests
smoke-jsonl:
    #!/bin/bash
    set -euo pipefail

    echo "=== JSONL smoke: system.ping ==="
    printf '{"id":"1","method":"system.ping","params":{}}\n' \
        | cargo run -p michelangelo_cli -- core stdio 2>/dev/null

    echo ""
    echo "=== JSONL smoke: project.create + get_snapshot ==="
    tmpdir=$(mktemp -d)
    printf '{"id":"c1","method":"project.create","params":{"path":"%s","name":"smoke"}}\n{"id":"c2","method":"project.get_snapshot","params":{}}\n' "$tmpdir" \
        | cargo run -p michelangelo_cli -- core stdio 2>/dev/null

    echo ""
    echo "=== JSONL smoke: project.open + get_snapshot ==="
    tmpdir=$(mktemp -d)
    printf '{"id":"o1","method":"project.create","params":{"path":"%s","name":"open-smoke"}}\n{"id":"o2","method":"project.open","params":{"path":"%s"}}\n{"id":"o3","method":"project.get_snapshot","params":{}}\n' "$tmpdir" "$tmpdir" \
        | cargo run -p michelangelo_cli -- core stdio 2>/dev/null

    echo ""
    echo "=== JSONL smoke: project.get_snapshot with no project (error) ==="
    printf '{"id":"e1","method":"project.get_snapshot","params":{}}\n' \
        | cargo run -p michelangelo_cli -- core stdio 2>/dev/null

    echo ""
    echo "=== smoke-jsonl complete ==="

# Smoke test: reopen across process boundaries
smoke-reopen:
    #!/bin/bash
    set -euo pipefail

    echo "=== smoke-reopen: create and reopen ==="
    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' EXIT

    echo "--- CLI invocation 1: create ---"
    printf '{"id":"c1","method":"project.create","params":{"path":"%s","name":"reopen-test"}}\n' "$tmpdir" \
        | cargo run -p michelangelo_cli -- core stdio 2>/dev/null

    echo ""
    echo "--- CLI invocation 2: open + get_snapshot ---"
    printf '{"id":"o1","method":"project.open","params":{"path":"%s"}}\n{"id":"s1","method":"project.get_snapshot","params":{}}\n' "$tmpdir" \
        | cargo run -p michelangelo_cli -- core stdio 2>/dev/null

    echo ""
    echo "=== smoke-reopen complete ==="

# End-to-end headless scenario with JSON validation (requires python3)
smoke-e2e:
    #!/bin/bash
    set -euo pipefail

    echo "=== smoke-e2e: headless end-to-end scenario ==="
    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' EXIT

    RUNNER="${CARGO_TARGET_DIR:-target}/debug/michelangelo"
    cargo build -p michelangelo_cli -q 2>/dev/null

    py() {
        echo "$2" | python3 -c "import sys,json; data=json.load(sys.stdin); $1" && echo "  ✓ $3" || { echo "  ✗ $3 — response: $2"; exit 1; }
    }

    echo "--- 1. ping ---"
    out=$(printf '{"id":"1","method":"system.ping","params":{}}\n' | "$RUNNER" core stdio 2>/dev/null)
    py 'assert data["result"]["pong"] == True' "$out" 'ping returns pong=true'

    echo "--- 2. project.create + get_snapshot ---"
    out=$(printf '{"id":"c1","method":"project.create","params":{"path":"%s","name":"e2e-test"}}\n{"id":"s1","method":"project.get_snapshot","params":{}}\n' "$tmpdir" | "$RUNNER" core stdio 2>/dev/null)
    create_resp=$(echo "$out" | sed -n '1p')
    snap_resp=$(echo "$out" | sed -n '2p')
    py 'assert data["result"]["name"] == "e2e-test"' "$create_resp" 'create returns project with name'
    py 'assert data["result"]["workspace_status"] == "active"' "$snap_resp" 'snapshot has active status'
    py 'assert data["result"]["project"]["name"] == "e2e-test"' "$snap_resp" 'snapshot has project name'
    py 'assert data["result"]["assets"]["index_status"] == "empty"' "$snap_resp" 'snapshot has asset summary'

    echo "--- 3. project.open across process boundary ---"
    out=$(printf '{"id":"o1","method":"project.open","params":{"path":"%s"}}\n{"id":"o2","method":"project.get_snapshot","params":{}}\n' "$tmpdir" | "$RUNNER" core stdio 2>/dev/null)
    open_resp=$(echo "$out" | sed -n '1p')
    snap_resp2=$(echo "$out" | sed -n '2p')
    py 'assert "result" in data' "$open_resp" 'reopen succeeds'
    py 'assert data["result"]["project"]["name"] == "e2e-test"' "$snap_resp2" 'snapshot after reopen has project'

    echo "--- 4. error: invalid request preserves id with whitespace ---"
    out=$(printf '{"id": "inv2", "params": {}}\n' | "$RUNNER" core stdio 2>/dev/null)
    py 'assert data["id"] == "inv2"' "$out" 'invalid request id preserved with whitespace'
    py 'assert data["error"]["code"] == -32600' "$out" 'invalid request error code -32600'

    echo "--- 5. error: get_snapshot with no project ---"
    out=$(printf '{"id":"e1","method":"project.get_snapshot","params":{}}\n' | "$RUNNER" core stdio 2>/dev/null)
    py 'assert data["error"]["code"] == -32000' "$out" 'error response has code -32000'

    echo ""
    echo "=== smoke-e2e PASSED ==="

# Build the CLI binary
build:
    cargo build -p michelangelo_cli
