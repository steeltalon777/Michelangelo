# Task Tracking

## Completed — Stage 1 (Foundation)

- [x] T-0001 Rust workspace skeleton (4 crates)
- [x] T-0002 Protocol DTOs (envelopes, errors, snapshot)
- [x] T-0003 CLI bootstrap (clap, --help, --version, core stdio)
- [x] T-0004 JSONL stdin/stdout loop (system.ping, error handling)

## Completed — Stage 2 (Workspace & Verification)

- [x] T-0005 Workspace create/open (directory layout, project.json)
- [x] T-0006 project.get_snapshot (state recovery DTO)
- [x] T-0007 justfile verification commands
- [x] T-0008 Minimal GitHub Actions CI

## Completed — Stage 3 (Hardening)

- [x] T-0009 Protocol error contract hardening (numeric error codes, golden tests)
- [x] T-0010 Core command router (registration-based dispatch, standalone handlers)
- [x] T-0011 Workspace filesystem invariants (validate_layout, Corrupted error)
- [x] T-0012 SQLite project metadata MVP (Storage, metadata.db, fallback to JSON)
- [x] T-0013 Project list/reopen smoke (process-boundary survival test)
- [x] T-0014 Headless job DTOs (JobStatus, JobDto)
- [x] T-0015 Asset graph placeholder DTOs (AssetDto, AssetSummaryDto, IndexStatus)
- [x] T-0016 Headless end-to-end scenario (smoke-e2e with Python validation)
- [x] T-0017 Documentation sync

## Not yet started

- Blender job execution
- PNG/asset ingest
- LLM integration
- GUI/TUI
- Network transport
