# TZ: Docs Sync after ADR 0004 Acceptance

Status: draft
Branch: `dev` (no push, no merge; commits only after verified checks)
Owner: TechLead (orchestrator); implementation by worker subagents; docs-only iteration.

## 0. Context and Baseline

Baseline at start (2026-11-07, `dev` = `4e3329c`):

- ADR 0004 (`docs/adr/0004-validated-visual-spec-and-hybrid-reconstruction.md`) is approved by
  the project owner on 2026-11-07; its current status line still says "Proposed".
- ADR §10 requires a separate sync of the product specification and tasks after acceptance.
- `docs/research/Michelangelo исследование стека реконструкции 3D-ассетов и сопутствующей
AI-инфраструктуры.md` is a local untracked material and must NOT be committed; ADR §14
currently contains a markdown link to it which would be broken in the committed repo.
- ROADMAP Phase 3/4 predate ADR 0004 and do not reflect its staged plan (ADR §13).
- MVP_SPEC.md and AI_CONTEXT.md do not reference the accepted ADR.

## 1. Goal

Synchronize repository documentation with accepted ADR 0004: flip the ADR status to Accepted,
remove the broken link to the untracked research document, restate ROADMAP Phase 3/4 per the
ADR §13 staged plan (A: minimal input package + validation; B: vertical reconstruction PoC
without API; C: local-edit boundary; D: optional assistant), and add cross-references from
MVP_SPEC.md and AI_CONTEXT.md.

## 2. Out of Scope

- Any code, wire-protocol, DTO, SQLite schema or worker-operation changes.
- Renumbering or rewording ROADMAP Phases 1/2/2.5 (Done) and Phase 5 (GUI/TUI).
- Committing `docs/research/` or any part of the untracked research material into Git.
- Changes to `docs/specs/CORE_PROTOCOL.md` and `docs/specs/BLENDER_WORKER.md` (they remain the
  normative wire/runtime description per ADR §10).
- CHANGELOG/README updates beyond what is strictly required by the files listed in section 4.

## 3. Preserved Conventions

- Russian language for ADR/MVP_SPEC prose, English for ROADMAP items and TZ text, matching the
  existing repo style.
- Relative markdown links only, pointing at files that exist in the committed tree.
- No phase renumbering; existing still-relevant Phase 3/4 items (manifest/PNG ingest,
  Asset Graph, MCP adapter) are integrated into the new wording, not dropped.

## 4. Iteration Plan (single iteration IT-1, docs-only)

Write scopes (exhaustive list of files allowed to change):

- `docs/adr/0004-validated-visual-spec-and-hybrid-reconstruction.md` — status line →
  `Accepted — утверждён владельцем проекта 2026-11-07.`; §14 research entry → plain text
  mention "локальный untracked-материал, в репозиторий не входит", no hyperlink.
- `ROADMAP.md` — full replacement of the Phase 3 and Phase 4 sections per the staged plan
  ADR §13 (draft text attached to this TZ); Phases 1/2/2.5/5 untouched.
- `docs/specs/MVP_SPEC.md` — one blockquote cross-reference to accepted ADR 0004 inserted
  after the Phase 2.5 status note in §3.1; nothing else.
- `docs/ai/AI_CONTEXT.md` — new `## Product Direction` section before `## Out of Scope`;
  `## Out of Scope` reworded so Visual Hull / Parametric Solver / LLM-MCP are marked as
  deferred per ADR 0004, not rejected; Version Pins, Verification, Environment Variables
  unchanged.
- `docs/tz/TZ_DOCS_SYNC_ADR_0004.md` — this TZ (checklist/evidence).

## 5. Acceptance Criteria

- ADR 0004 status line reads `Accepted — утверждён владельцем проекта 2026-11-07.`; the ADR
  date line still reads `2026-11-07`.
- No markdown link in any changed file targets
  `docs/research/Michelangelo исследование стека реконструкции 3D-ассетов и сопутствующей
AI-инфраструктуры.md`:
  `git grep -n "research/Michelangelo" -- docs README.md ROADMAP.md` (run from repo root)
  returns no markdown-link occurrences referencing the research document path.
- `git status --porcelain docs/research/` shows the research directory as untracked (not
  staged); `git ls-files docs/research/` is empty.
- ROADMAP still contains exactly one Phase 1, Phase 2, Phase 2.5 (all marked Done), Phase 3,
  Phase 4 and Phase 5 heading; no phase renumbering:
  `git grep -nE "^## .*(Phase [0-9.]+)" -- ROADMAP.md` shows the unchanged phase set and
  order.
- ROADMAP Phase 3/4 mention ADR 0004 staged plan stages A–D and keep manifest/PNG ingest,
  Asset Graph indexing, MCP adapter items.
- MVP_SPEC §3.1 contains a relative link `../adr/0004-validated-visual-spec-and-hybrid-reconstruction.md`
  that resolves: the target file exists in the working tree.
- All relative markdown links added by this iteration resolve to existing files
  (manual check of each added link target; no absolute paths, no URLs to local files).
- `git diff --check` clean (no trailing whitespace); changed Markdown renders correctly
  (headings, blockquotes, lists).
- `cargo`/Python checks are not required (no code touched); if run for sanity, results must
  match the pre-change baseline.
- Commit on `dev` only, explicit pathspecs, staged diff inspected before commit; no push.

## 6. Evidence

Iteration executed 2026-11-07 on `dev` (base `4e3329c`), single docs-only change set:

- Applied edits: ADR 0004 status line → Accepted; ADR §14 research entry de-linked;
  ROADMAP Phase 3/4 replaced with the ADR §13 staged plan (status `Status: draft` above is
  superseded — this TZ is completed); MVP_SPEC §3.1 cross-reference blockquote; AI_CONTEXT
  `## Product Direction` + reworded `## Out of Scope`; this TZ file.
- `git status --porcelain`: exactly ROADMAP.md, docs/ai/AI_CONTEXT.md, docs/specs/MVP_SPEC.md
  modified; docs/adr/0004-*.md and this TZ untracked; `docs/research/` untracked, never staged
  (`git ls-files docs/research/` empty).
- `git diff --check`: clean (no trailing whitespace).
- `git grep -n "research/Michelangelo" -- docs ROADMAP.md README.md`: no matches (no links to
  the untracked research material).
- `git grep -nE "^## .*(Phase [0-9.]+)" -- ROADMAP.md`: 6 phase headings, order
  1, 2, 2.5, 3, 4, 5; Phases 1/2/2.5 unchanged (Done).
- Independent reviewer verdict: **GREEN** (all 10 acceptance checks passed; two minor notes —
  a missing trailing period in ROADMAP Phase 3 and this Evidence placeholder — both fixed in
  the closing edit).
- Commit: this change set itself (docs-only, explicit pathspecs, `dev`, no push).
