# michelangelo_asset_validation (GLM-6, ADR 0005)

Standalone validation layer PoC for ADR 0005 §§5.3/8/11 on the IT-8D base
(`dev@b861547`). Not a member of the root workspace (empty `[workspace]`
table in `Cargo.toml`); build from this directory:

```bash
cargo test --offline          # 9 positive + 17 adversarial tests
cargo clippy --offline --all-targets -- -D warnings   # clean
cargo fmt --check             # clean
cargo run --offline --example dump_validation_report   # local smoke
```

Boundary notes:

- All DTOs are private provisional drafts aligned with the GLM-1 contract
  gate (`draft-0.1`); not an approved wire protocol.
- `CheckStatus::NotRequired` exists only in the NEW report model — it never
  enters the IT-8 evaluation enum and never converts `unavailable` into
  `pass` (OP-7).
- Reconstruction profiles are refused (`UNSUPPORTED_PROFILE_KIND`): the
  existing IT-8D reconstruction evaluator stays authoritative.
- The acceptance side is a seam only (`CanonicalAcceptSeam`); the default
  publication path refuses — there is no second publisher.
- Fixture-level formats only (OBJ/MTL + provisional metadata JSON);
  everything the module cannot measure honestly returns `unavailable`.

Stream documents: `docs/adr5-streams/validation/` (SCOPE.md, TEST_EVIDENCE.md,
INTEGRATION_NOTES.md).
