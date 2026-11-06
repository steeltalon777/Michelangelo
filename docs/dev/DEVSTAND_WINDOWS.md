# Windows Dev Stand

How to build, test, and run Michelangelo on Windows (10/11, x64).

The workspace itself is platform-agnostic; the notes below cover the Windows
specifics (toolchain, fake Blender test double, path normalization, and the
Bash-only parts of the tooling).

## Prerequisites

| Tool | Why | Install |
|---|---|---|
| Rust stable (MSVC toolchain) | builds the workspace | `rustup default stable-msvc` |
| Visual Studio Build Tools 2022, "Desktop development with C++" | `rusqlite` with the `bundled` feature compiles SQLite C sources and needs `link.exe` + Windows SDK | https://visualstudio.microsoft.com/visual-cpp-build-tools/ |
| Git for Windows | repository + Bash used by some `justfile` recipes | `winget install Git.Git` |
| Python 3.x | only the smoke validation script `scripts/validate_blender_smoke.py` | `winget install Python.Python.3.12` |
| Blender 4.x/5.x (optional) | real Blender jobs and smoke tests | `winget install BlenderFoundation.Blender` |
| `just` (optional) | runs the recipes in `justfile` | `cargo install just` |

`justfile` recipes are POSIX shell scripts. Run them from Git Bash or WSL, or
use the equivalent `cargo` commands shown below.

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

Build the protocol requests with `ConvertTo-Json` (or any real JSON
serializer), not by string interpolation: Windows paths contain backslashes,
which are invalid JSON escapes when pasted into a raw JSON literal.

## Verification

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

This is the same set of checks the Linux CI runs, with two exceptions that are
Bash-only: `just smoke-*` recipes and the `just verify` wrapper.

## Tests on Windows

The test suite is cross-platform. Two details are Windows-aware:

- **Fake Blender test double.** `michelangelo_blender::create_fake_blender()`
  writes `fake_blender.sh` on Unix and `fake_blender.cmd` on Windows.
  `Command::new` cannot start a `.cmd`/`.bat` directly on Windows
  (`CreateProcess` fails with error 193), so the adapter launches batch test
  doubles through `cmd /C call`. The Windows `success` variant reads
  `job.json` with the built-in Windows PowerShell (`powershell.exe`) to embed
  the real `job_id` in `result.json`; no Bash, WSL, or Python is required.
- **Path normalization.** `create_project` stores the canonicalized path,
  which on Windows looks like `\\?\C:\Users\Name\...`. `open_project` first
  looks up the caller-provided spelling in SQLite and then retries with the
  canonical form, so 8.3 short names (`C:\Users\NAME~1\...`), trailing `.`,
  and other non-canonical spellings still hit the SQLite record instead of
  falling back to `project.json`. Tests compare canonical path forms rather
  than substrings for the same reason.

Absolute-path tests use the host syntax: `/etc/passwd` on Unix and
`C:/etc/passwd` on Windows.

## Real Blender jobs

The adapter uses `BLENDER_BIN` (default `blender`) and invokes:

```bash
blender --background --python blender/worker.py -- <job.json> <result.json>
```

Set the path explicitly when Blender is not on `PATH`:

```powershell
$env:BLENDER_BIN = "C:\Program Files\Blender Foundation\Blender 5.2\blender.exe"
.\target\debug\michelangelo.exe core stdio
```

The `just smoke-blender-real` / `just smoke-blender-real-glb` recipes show the
exact JSONL scenarios; run them from Git Bash/WSL or copy their command lines.

## Known limitations

1. `justfile` recipes are Bash scripts (`#!/bin/bash`). On Windows use Git
   Bash/WSL, or run the equivalent `cargo` commands directly.
2. `scripts/create_fake_blender.sh` and `just smoke-blender-fake` are
   Unix-only. The production adapter spawns `BLENDER_BIN` directly (it does
   not wrap batch files), so a fake Blender passed through `BLENDER_BIN` is
   not supported on Windows. Use `cargo test -p michelangelo_blender` for the
   fake-executable coverage and a real Blender for end-to-end smoke.
3. `Path::is_absolute()` on Windows requires a drive prefix. A rooted path
   without a drive (for example `/etc/passwd`) is therefore not treated as
   absolute by the spec/artifact validators on Windows; tests use
   platform-appropriate absolute paths. This is existing validation behavior,
   unchanged by the cross-platform test work.
4. Canonical `root_path` values may carry the `\\?\` prefix and the long
   (non-8.3) form. This is expected and survives across process boundaries.
5. CI (`.github/workflows/ci.yml`) runs on `ubuntu-latest` only. Windows
   support is validated locally; there is no Windows CI job yet.

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `os error 193` when a Blender job runs | a `.cmd`/`.sh` test double was launched directly; fake test doubles must go through the adapter's batch-aware launch path |
| `invalid escape at line 1 column N` in a JSONL request | request built by string interpolation of a Windows path; serialize with `serde_json` / `ConvertTo-Json` instead |
| `just: command not found` | `cargo install just`, or use the `cargo` commands above |
| SQLite/linker errors during `cargo test` | install the MSVC toolchain and VS Build Tools C++ workload |
| Paths like `C:\Users\NAME~1\...` in `%TEMP%` | 8.3 short names; the workspace canonicalizes before SQLite lookup, so no action needed |
