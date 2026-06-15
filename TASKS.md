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

## Completed — Phase 2 (Blender Integration)

- [x] T-0201 Protocol contract (methods, events, Blender DTOs, crate skeletons)
- [x] T-0202 SQLite job persistence (`jobs` table, CRUD APIs)
- [x] T-0203 Job scheduler and event queue (in-process, max concurrency 1, mpsc events)
- [x] T-0204 Blender worker contract (`blender/worker.py`, 7 safe operations)
- [x] T-0205 Headless Blender adapter (`HeadlessBlenderAdapter`, fake executable support)
- [x] T-0206 Core command wiring (Phase 2 handlers in Router, CoreService integration)
- [x] T-0207 CLI JSONL event transport (event channel, stdout JSONL streaming)
- [x] T-0208 Snapshot includes jobs, process-boundary job persistence
- [x] T-0209 Failure/cancel/timeout hardening
- [x] T-0210 Verification recipes (`smoke-blender-fake`, `smoke-blender-real`)
- [x] T-0211 Real Blender stand smoke (validated with Blender 4.0.2)
- [x] T-0212 Documentation sync

## In Progress — Phase 2 Hardening

- [ ] T-0213 Acceptance hardening (smoke validation, contract sync, docs) — see `docs/tz/TZ_PHASE_2_ACCEPTANCE_HARDENING.md`

## Not yet started

- PNG/asset ingest (Phase 3)
- LLM integration (Phase 4)
- GUI/TUI (Phase 5)
- Network transport
