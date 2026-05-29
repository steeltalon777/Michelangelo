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
    @echo "=== JSONL smoke: system.ping ==="
    printf '{"id":"1","method":"system.ping","params":{}}\n' \
        | cargo run -p michelangelo_cli -- core stdio 2>/dev/null
    @echo ""
    @echo "=== JSONL smoke: project.create + get_snapshot ==="
    tmpdir=`mktemp -d` && \
    printf "{\"id\":\"c1\",\"method\":\"project.create\",\"params\":{\"path\":\"$$tmpdir\",\"name\":\"smoke\"}}\n{\"id\":\"c2\",\"method\":\"project.get_snapshot\",\"params\":{}}\n" \
        | cargo run -p michelangelo_cli -- core stdio 2>/dev/null
    @echo ""
    @echo "=== JSONL smoke: project.open + get_snapshot ==="
    tmpdir=`mktemp -d` && \
    printf "{\"id\":\"o1\",\"method\":\"project.create\",\"params\":{\"path\":\"$$tmpdir\",\"name\":\"open-smoke\"}}\n{\"id\":\"o2\",\"method\":\"project.open\",\"params\":{\"path\":\"$$tmpdir\"}}\n{\"id\":\"o3\",\"method\":\"project.get_snapshot\",\"params\":{}}\n" \
        | cargo run -p michelangelo_cli -- core stdio 2>/dev/null
    @echo ""
    @echo "=== JSONL smoke: project.get_snapshot with no project (error) ==="
    printf '{"id":"e1","method":"project.get_snapshot","params":{}}\n' \
        | cargo run -p michelangelo_cli -- core stdio 2>/dev/null

# Build the CLI binary
build:
    cargo build -p michelangelo_cli
