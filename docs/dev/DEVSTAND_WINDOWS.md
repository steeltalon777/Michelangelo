# Windows Dev Stand

How to build, test, and run Michelangelo on Windows (10/11, x64). The workspace itself is
platform-agnostic; this page covers the Windows specifics (toolchain, fake Blender test doubles,
path normalization, the batch-wrapper launch path, and the acceptance smoke driver).

## Prerequisites

| Tool | Why | Install |
|---|---|---|
| Rust stable (MSVC toolchain) | builds the workspace | `rustup default stable-msvc` |
| Visual Studio Build Tools 2022, "Desktop development with C++" | `rusqlite` with the `bundled` feature compiles SQLite C sources and needs `link.exe` + Windows SDK | https://visualstudio.microsoft.com/visual-cpp-build-tools/ |
| Python 3.10+ (3.13 used here) | `blender/worker.py` validation, `scripts/*.py` smoke drivers (stdlib only) | `winget install Python.Python.3.12` or the current 3.13 installer |
| Blender 5.2.3 LTS (optional but required for real jobs/acceptance) | real Blender batch jobs, real acceptance smoke | `winget install BlenderFoundation.Blender` (pin 5.2 LTS) |
| Git for Windows (optional) | repository + Bash used by some `justfile` recipes | `winget install Git.Git` |
| `just` (optional, **not installed on this stand**) | runs the POSIX recipes in `justfile` | `cargo install just` |

The current Windows stand has Rust 1.96.0, Python 3.13, Blender 5.2.3 LTS on `PATH`, and Git
Bash. `just` is **not** installed: use the direct commands below (they are what the recipes wrap).
`justfile` recipes are POSIX shell scripts; they require `sh` and only run reliably from Git
Bash/WSL.

## Quick start (PowerShell)

```powershell
cargo build -p michelangelo_cli

# Health check
'{"id":"1","method":"system.ping","params":{}}' |
    .\target\debug\michelangelo.exe core stdio

# Create a project and read the snapshot
$tmp = Join-Path $env:TEMP "michelangelo-demo"
New-Item -ItemType Directory -Path $tmp -Force | Out-Null
$lines = @(
    (@{ id = "c1"; method = "project.create"; params = @{ path = $tmp; name = "demo" } } | ConvertTo-Json -Compress),
    '{"id":"s1","method":"project.get_snapshot","params":{}}'
)
$lines | .\target\debug\michelangelo.exe core stdio
```

Build JSON requests with `ConvertTo-Json` (or any real JSON serializer), not by string
interpolation: Windows paths contain backslashes, which are invalid JSON escapes when pasted into
a raw JSON literal.

## Verification

```powershell
# Rust — 271 tests; real-Blender tests auto-skip when Blender is not installed
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Python worker checks (no bpy) — 30/30
python -m py_compile blender/worker.py scripts/test_worker_validation.py scripts/smoke_blender_scene.py
python scripts/test_worker_validation.py

# Cross-platform acceptance E2E
python scripts/smoke_blender_scene.py --fake   # deterministic, no Blender required
python scripts/smoke_blender_scene.py          # real Blender; prints SKIP + exit 0 if absent
```

Equivalent `just` recipes (POSIX, need `just` + `sh`): `just verify`, `just test-worker-python`,
`just smoke-blender-scene`, `just smoke-blender-scene-fake`, `just smoke-blender-scene-required`,
plus the legacy smokes. Without `just`, run the exact commands above.

## Acceptance smoke driver

`scripts/smoke_blender_scene.py` is the canonical E2E check; it works identically on Windows and
Linux and does not need `just` or Bash:

```powershell
python scripts/smoke_blender_scene.py [--fake] [--blender-bin PATH] [--cli PATH] [--keep]
                                      [--timeout-ms N] [--require-blender]
```

- real mode resolves Blender as `--blender-bin` > `BLENDER_BIN` > `blender` on `PATH`, runs
  `--version`, prints a clear `SKIP` and exits `0` when Blender is missing (`--require-blender`
  makes that exit `2`);
- `--fake` generates `fake_blender.cmd` + a shared `fake_impl.py` in a temp directory and sets
  `BLENDER_BIN` for the CLI — the **production** adapter launches `.cmd` through `cmd /C call`, so
  the fake exercises the real Windows spawn path;
- it runs the composite scene from `examples/scene_ops/composite_scene_spec.json` through the CLI,
  validates JSONL events/identity, artifact metadata and headers, the calibrated silhouette alpha
  (real mode), then reopens the project in a second process;
- the temporary workspace and fake directory are removed unless `--keep` is passed; exit codes:
  `0` pass/SKIP, `1` validation failure, `2` setup error.

## Tests on Windows

The test suite is cross-platform. Windows-aware details:

- **Fake Blender test double.** `michelangelo_blender::create_fake_blender()` writes
  `fake_blender.cmd` on Windows (with a PowerShell implementation) and `fake_blender.sh` on Unix.
  `Command::new` cannot start `.cmd`/`.bat` files directly (`CreateProcess` fails with error 193),
  so **all** adapter launch paths (`--version`, capabilities probe, real and fake job runs) route
  batch programs through `cmd /C call`. Passing a `.cmd`/`.bat` path via `BLENDER_BIN` is therefore
  supported in production code paths, not just in tests.
- **Path normalization.** `create_project` stores the canonicalized path, which on Windows looks
  like `\\?\C:\Users\Name\...`. `open_project` first looks up the caller-provided spelling in
  SQLite and then retries with the canonical form, so 8.3 short names, trailing dots and other
  non-canonical spellings still hit the SQLite record. `blender/worker.py` strips the `\\?\`
  prefix before Blender file operations (the bundled glTF exporter's `os.makedirs` rejects
  verbatim paths).
- **Path forms.** All validators reject Windows drive/UNC forms (`C:\...`, `C:foo`,
  `\\server\share\...`, `//server/share/...`) on every platform, matching the worker.
- Absolute-path tests use the host syntax: `/etc/passwd` on Unix and `C:/etc/passwd` on Windows.

## Real Blender jobs

The adapter uses `BLENDER_BIN` (default `blender` on `PATH`) and invokes:

```text
blender --background --factory-startup --python blender/worker.py -- <job.json> <result.json>
```

Set the path explicitly when Blender is not on `PATH` (standard install location shown):

```powershell
$env:BLENDER_BIN = "C:\Program Files\Blender Foundation\Blender 5.2\blender.exe"
.\target\debug\michelangelo.exe core stdio
```

- Tested target: **Blender 5.2.3 LTS** (headless, Windows). Bundled Python 3.13.13.
- Only Workbench rendering is supported by the worker contract; EEVEE/Cycles determinism is out of
  scope this iteration.
- `blender --version`, the capabilities probe (30 s) and each job are bounded; timeouts kill the
  process tree (`taskkill /T /F`) and are reported as job failures.
- Real Blender acceptance E2E is local/manual — GitHub-hosted CI runners do not ship Blender 5.2.

## Known limitations

1. `justfile` recipes are POSIX shell scripts (`#!/bin/bash` or shell lines). On Windows use Git
   Bash/WSL, or run the equivalent `cargo`/`python` commands directly; `just` is not installed on
   this stand.
2. CI runs Rust/Python checks and the fake acceptance smoke on `windows-latest`, but **no real
   Blender** — real-Blender tests auto-skip there. Real Blender acceptance remains a local gate.
3. `Path::is_absolute()` on Windows requires a drive prefix: a rooted path without a drive (for
   example `/etc/passwd`) is not treated as absolute by spec/artifact validators. Windows drive,
   drive-relative and UNC forms are rejected by explicit string checks instead, so cross-platform
   behavior matches the worker.
4. Canonical `root_path` values may carry the `\\?\` prefix and the long (non-8.3) form. This is
   expected and survives across process boundaries; the worker normalizes it before file I/O.
5. Containment checks resolve existing symlinks before writes and repeat the check after parent
   creation, but a hostile concurrent process could still swap a path between check and write
   (classic local TOCTOU). The adapter's post-run artifact validation remains the final backstop.

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `os error 193` when a Blender job runs | a `.cmd`/`.sh` path reached `CreateProcess` directly; production launch paths wrap `.cmd`/`.bat` with `cmd /C call` — check custom launchers for the old behavior |
| `invalid escape at line 1 column N` in a JSONL request | request built by string interpolation of a Windows path; serialize with `serde_json` / `ConvertTo-Json` instead |
| `just: command not found` | `just` is not installed; use the direct `cargo`/`python` commands above (or `cargo install just` and run from Git Bash) |
| SQLite/linker errors during `cargo test` | install the MSVC toolchain and VS Build Tools C++ workload |
| `python` not found | install Python 3.10+ and make sure the installer added it to `PATH` |
| Real acceptance smoke exits 0 after printing `SKIP` | Blender is not on `PATH`/`BLENDER_BIN`; set `BLENDER_BIN` or pass `--blender-bin`, or use `--fake` |
| `whitelist ...` / capabilities probe error | Blender binary found but the worker capabilities probe failed; run the capabilities command from `docs/specs/BLENDER_WORKER.md` manually |
