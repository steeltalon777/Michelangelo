# Linux Dev Stand

How to build, test, and run Michelangelo on Linux. The current iteration was developed and
verified on a **Windows** stand with Blender 5.2.3 LTS; this page records the Linux workflow and
what CI actually covers.

## Prerequisites

| Tool | Why | Install |
|---|---|---|
| Rust stable (1.96.0 used) | builds the workspace | `rustup default stable` |
| C compiler + linker | `rusqlite` (bundled SQLite) | `sudo apt install build-essential` |
| Python 3.10+ (3.13 used) | worker validation and smoke drivers (stdlib only) | `sudo apt install python3` |
| Blender 5.2.3 LTS (optional, real jobs only) | real Blender batch jobs / acceptance E2E | distribution package or the official tarball |
| `just` (optional) | runs the POSIX `justfile` recipes | `cargo install just` |

No Linux Blender stand was available locally during the reliability/scene-ops iteration, so **real
Blender behavior is verified on the Windows stand only**. CI on `ubuntu-latest` covers the Rust
workspace and the Python/fake-Blender checks; it does not ship Blender.

## Build and verify

```bash
cargo build -p michelangelo_cli

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace        # real-Blender tests auto-skip when `blender` is not on PATH

python3 -m py_compile blender/worker.py scripts/test_worker_validation.py scripts/smoke_blender_scene.py
python3 scripts/test_worker_validation.py
```

`just` is optional; the recipes are POSIX shell scripts and work on Linux. The Python recipes
default to `python3` (Linux/macOS convention); override with `just python_bin=python ...` on hosts
where only `python` exists:
`just verify`, `just test-worker-python`, `just smoke-blender-scene-fake`,
`just smoke-blender-scene`, `just smoke-blender-scene-required`, plus the legacy smokes.

## Acceptance smoke

```bash
# Deterministic: generated fake Blender, no Blender installation required
python3 scripts/smoke_blender_scene.py --fake

# Real Blender: resolves --blender-bin > BLENDER_BIN > `blender` on PATH.
# Prints SKIP and exits 0 when Blender is unavailable; --require-blender makes that exit 2.
python3 scripts/smoke_blender_scene.py
python3 scripts/smoke_blender_scene.py --require-blender
```

Both modes run the composite scene from `examples/scene_ops/composite_scene_spec.json` through the
CLI (5 artifacts), validate JSONL event ordering/identity, invalid-spec rejection, artifact
metadata and headers, and reopen the project in a second process. Real mode additionally asserts
the calibrated front silhouette alpha and the framed-scene silhouette coverage/bbox/margin
(`cam_scene` starts at `ortho_scale=100`, so a broken `frame_camera` fails). Temporary directories
are removed unless `--keep` is passed; the driver resolves `<repo>/target/debug/michelangelo` or
`--cli PATH` and does not read `CARGO_TARGET_DIR`.

Fake mode uses `fake_blender.sh` (shebang + executable bit) plus the shared `fake_impl.py`, so it
also exercises the Unix direct-exec launch path. On Windows the same driver generates
`fake_blender.cmd`; the production adapter routes batch files through `cmd /C call`.

## CI coverage (ubuntu-latest)

`.github/workflows/ci.yml` runs on Ubuntu and Windows:

- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, `cargo build -p michelangelo_cli`;
- Python worker syntax + `scripts/test_worker_validation.py` (no `bpy`);
- `python3 scripts/smoke_blender_scene.py --fake`;
- CLI project/snapshot smokes.

**Blocker (documented):** GitHub-hosted runners have no Blender 5.2 installation, so the real
Blender acceptance smoke (calibrated silhouette) is not part of CI. Real-Blender tests
auto-skip there. Run the real smoke locally on a stand with Blender 5.2.3 (`BLENDER_BIN`) before
claiming real-Blender verification.

## Known limitations on Linux

1. No local Linux Blender stand was available for this iteration; real Blender behavior was
   verified on Windows with Blender 5.2.3. Use `BLENDER_BIN` to point at a Linux Blender and run
   `python3 scripts/smoke_blender_scene.py --require-blender` to validate a stand.
2. Rendering is Workbench-only; EEVEE/Cycles determinism is out of scope.
3. Worker import axis behavior comes from Blender's default importers (no axis overrides), same as
   on Windows.
4. Blender 4.2 is API-plausible but untested; only 5.2.3 is verified in this iteration.
