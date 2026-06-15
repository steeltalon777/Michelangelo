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

# Smoke test: blender.run_job with a fake Blender executable
# Validates JSONL structurally (events, status, artifacts).
smoke-blender-fake:
    #!/bin/bash
    set -euo pipefail

    echo "=== smoke-blender-fake: blender.run_job with fake Blender ==="

    fake_dir=$(mktemp -d)
    trap 'rm -rf "$fake_dir"' EXIT
    fake_blender="$fake_dir/fake_blender.sh"
    bash scripts/create_fake_blender.sh "$fake_blender"

    workspace=$(mktemp -d)
    trap 'rm -rf "$fake_dir" "$workspace"' EXIT

    echo "--- Create project and run fake Blender job ---"
    out=$(BLENDER_BIN="$fake_blender" printf '{"id":"c1","method":"project.create","params":{"path":"%s","name":"blender-fake-test"}}\n{"id":"j1","method":"blender.run_job","params":{"job_type":"blender_smoke_scene","wait":true,"spec":{"operations":[{"op":"create_box","name":"body","size":[2.0,1.0,0.5],"location":[0.0,0.0,0.0]},{"op":"save_blend","path":"blender/scenes/smoke.blend"}]}}}\n' "$workspace" | BLENDER_BIN="$fake_blender" cargo run -p michelangelo_cli -- core stdio 2>/dev/null)

    echo "$out"
    echo ""

    echo "--- Validating JSONL output (structural) ---"
    echo "$out" | python3 scripts/validate_blender_smoke.py --workspace "$workspace"
    echo ""
    echo "=== smoke-blender-fake PASSED ==="

# Real Blender smoke test — required path: create_box + save_blend only.
# Validates JSONL structurally: terminal "completed" status, .blend artifact exists.
# Skips gracefully if Blender is unavailable.
smoke-blender-real:
    #!/bin/bash
    set -euo pipefail

    echo "=== smoke-blender-real: blender.run_job with real Blender ==="

    if ! command -v "${BLENDER_BIN:-blender}" &>/dev/null; then
        echo "[skip] Blender not found at '${BLENDER_BIN:-blender}'. Install Blender or set BLENDER_BIN"
        exit 0
    fi

    echo "Blender: $("${BLENDER_BIN:-blender}" --version | head -1)"

    workspace=$(mktemp -d)
    trap 'rm -rf "$workspace"' EXIT

    echo "--- Create project and run real Blender job (create_box + save_blend) ---"
    out=$(printf '{"id":"c1","method":"project.create","params":{"path":"%s","name":"blender-real-test"}}\n{"id":"j1","method":"blender.run_job","params":{"job_type":"blender_smoke_scene","wait":true,"timeout_ms":120000,"spec":{"operations":[{"op":"create_box","name":"body","size":[2.0,1.0,0.5],"location":[0.0,0.0,0.0]},{"op":"save_blend","path":"blender/scenes/smoke_real.blend"}]}}}\n' "$workspace" | cargo run -p michelangelo_cli -- core stdio 2>/dev/null)

    echo "$out"
    echo ""

    echo "--- Validating JSONL output (structural) ---"
    echo "$out" | python3 scripts/validate_blender_smoke.py --workspace "$workspace" --artifact "blender/scenes/smoke_real.blend"
    echo ""
    echo "=== smoke-blender-real PASSED ==="

# Optional GLB export smoke test — runs only if Blender/glTF prerequisites are available.
# Never prints PASSED on failed job status.
smoke-blender-real-glb:
    #!/bin/bash
    set -euo pipefail

    echo "=== smoke-blender-real-glb: optional GLB export smoke ==="

    if ! command -v "${BLENDER_BIN:-blender}" &>/dev/null; then
        echo "[skip] Blender not found at '${BLENDER_BIN:-blender}'. Install Blender or set BLENDER_BIN"
        exit 0
    fi

    echo "Blender: $("${BLENDER_BIN:-blender}" --version | head -1)"

    # Quick check: can Blender Python enable the glTF exporter?
    gltf_ok=$("${BLENDER_BIN:-blender}" --background --python-expr "import addon_utils; addon_utils.enable('io_scene_gltf2'); print('MICHELANGELO_GLTF_OK')" 2>/dev/null | grep -c "MICHELANGELO_GLTF_OK" || true)
    if [ "$gltf_ok" -eq 0 ]; then
        echo "[skip] glTF/GLB export prerequisites unavailable (e.g. missing numpy)."
        echo "  Install numpy into Blender's Python or use a bundled Blender build."
        exit 0
    fi

    workspace=$(mktemp -d)
    trap 'rm -rf "$workspace"' EXIT

    echo "--- Create project and run Blender job with GLB export ---"
    out=$(printf '{"id":"c1","method":"project.create","params":{"path":"%s","name":"blender-glb-test"}}\n{"id":"j1","method":"blender.run_job","params":{"job_type":"blender_smoke_scene","wait":true,"timeout_ms":120000,"spec":{"operations":[{"op":"create_box","name":"body","size":[2.0,1.0,0.5],"location":[0.0,0.0,0.0]},{"op":"save_blend","path":"blender/scenes/smoke_glb.blend"},{"op":"export_glb","path":"blender/exports/smoke_glb.glb"}]}}}\n' "$workspace" | cargo run -p michelangelo_cli -- core stdio 2>/dev/null)

    echo "$out"
    echo ""

    echo "--- Validating JSONL output (structural) ---"
    echo "$out" | python3 scripts/validate_blender_smoke.py --workspace "$workspace" --artifact "blender/scenes/smoke_glb.blend"

    # Check GLB specifically
    glb_path="$workspace/blender/exports/smoke_glb.glb"
    if [ -f "$glb_path" ] && [ -s "$glb_path" ]; then
        echo "  PASS: GLB artifact exists ($(stat -c%s "$glb_path") bytes)"
    else
        echo "  WARN: GLB artifact missing or empty"
    fi
    echo ""
    echo "=== smoke-blender-real-glb PASSED ==="

# Build the CLI binary
build:
    cargo build -p michelangelo_cli
