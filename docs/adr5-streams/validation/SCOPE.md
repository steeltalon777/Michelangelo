# SCOPE — GLM-6 Asset Validation Profiles & Evidence (ADR 0005)

- **Дата:** 2026-11-08. **База:** `dev@b8615477b735c1f60c93518b59f8fe32613a7556` (IT-8D).
- **Ветка:** `feat/adr5-validation` (worktree `I:/prog/michelangelo-adr5-validation`).
- **Статус:** контрактный PoC уровня fixture. Не production-ready, не UE-ready,
  не утверждение wire-протокола. Все DTO — private provisional draft-0.1,
  выровненные по контрактному gate GLM-1 (`I:/prog/michelangelo-adr5-contracts`,
  commit `4597c43`), прочитанному read-only.

## 1. Фактический путь владения

Корневой `Cargo.toml` не изменялся. Новый crate — **standalone, вне членов
корневого workspace**:

| Путь | Содержимое |
|---|---|
| `michelangelo_asset_validation/` | standalone crate, свой `Cargo.toml` с пустой секцией `[workspace]` (cargo не присоединяет его к корневому workspace и не трогает корневой `Cargo.lock`); собственный `Cargo.lock` в своей директории |
| `michelangelo_asset_validation/src/` | profile, check (реестр check_id + новая модель результата), checks/* (детерминированные evaluators), report, registry (reservation + no-clobber + verify), acceptance (seam), canonical, ids, error, testing (fixture builders, test-only) |
| `michelangelo_asset_validation/tests/` | `positive_paths.rs` (9 тестов), `adversarial.rs` (14 негативов, включая H2/M1/M2/M3/M4) |
| `michelangelo_asset_validation/examples/dump_validation_report.rs` | локальный smoke-harness |
| `docs/adr5-streams/validation/` | этот SCOPE, TEST_EVIDENCE.md, INTEGRATION_NOTES.md |

Чужие файлы не изменялись: корневой `Cargo.toml`, `Cargo.lock`, существующие
crates, IT-8D edit accept, reconstruction evaluator, текущий IT-8 enum — без
изменений (подтверждено `git status`).

## 2. Вход → выход

- **Вход:** disposable validation workspace (manifest бандла + артефакты +
  dependencies + pinned `references/*.json`), pinned `ValidationProfile`
  (closed check_id registry, thresholds, evaluator versions, applicability
  conditions), verifying backends (опционально; без них — честный
  `unavailable`).
- **Выход:** immutable `ValidationReport` (per-check `pass/fail/unavailable/
  not_required`, verdict-свёртка fail > unavailable > pass, input digests
  фактически прочитанных bytes, profile digest) в no-clobber реестре;
  `verify()` для перечитывания и отказов F2/F3/F19; prerequisite-проверки
  acceptance-записей (F5/F14/F19) через seam к будущему canonical accept Core.
- **Не делает:** не публикует, не создаёт author approval, не переиспользует
  или переписывает IT-8D reconstruction evaluator, не вносит `not_required`
  в IT-8 enum.

## 3. Маппинг на handoff GLM-1 (docs/adr5-streams/contracts/HANDOFF.md §5)

| Пункт handoff | Реализация |
|---|---|
| check_id реестр (OP-4) | закрытый enum 18 id в `src/check.rs`, расширение — новая ревизия; unknown id отказывается с `VALIDATION_UNKNOWN_REQUIRED`/`VALIDATION_UNKNOWN_OPTIONAL` |
| Матрица применимости (VALIDATION_MATRIX §1) | `applicability(kind, check)` — Required / RequiredWithCondition(token) / Optional / Human / NotApplicable; конфликт required ∩ NotApplicable → `VALIDATION_APPLICABILITY_CONFLICT` |
| `not_required` вне IT-8 enum (OP-7) | `CheckStatus::NotRequired` — только в новой модели отчёта, только для optional-проверок и только с `applicability_ref`; `Verdict` не имеет `not_required`; F16-тесты |
| F2 input changed | `verify_registered_report` перечитывает все `input_digests` → `VALIDATION_INPUT_CHANGED` |
| F5 replay | запись привязана к одному bundle digest: `PrerequisiteReport.bundle_binding_matches=false` для чужого бандла + `LocalReplayIndex` (test device) отказывает `ACCEPTANCE_REPLAY`; реестр отчётов append-only (повтор с другим digest → `NO_CLOBBER_VIOLATION`) |
| F8 unknown required | schema: pre-parse JSON профиля отказывает до typed-парсинга; behavioral: отчёт без required-проверки → `MISSING_REQUIRED_CHECK` при регистрации и при verify с текущим профилем |
| F14 acceptance без отчёта | `RegistryBackedSeam::check_prerequisites` → `VALIDATION_REPORT_NOT_REGISTERED` |
| F16 unavailable→pass | свёртка в `Verdict::fold`, отказ при несовпадении (`INVALID_VERDICT_FOLD`), verdict enum без `not_required`, тест «unavailable вердикт не авторизует acceptance»; required-проверка с `not_required` блокируется при регистрации, verify и в seam (`INVALID_NOT_REQUIRED`, H2) |
| F19 stale approval | digest профиля в отчёте против текущего профиля → `VALIDATION_PROFILE_CHANGED`; `profile_binding_matches=false` в seam |
| F1 tamper / F9 effective protected change / F3 missing dep / F13 path escape | покрыты behavioral-тестами как смежные негативы (см. TEST_EVIDENCE.md) — основная ответственность GLM-2/GLM-4/GLM-5 |

## 4. Допущения

- RFC 8785-направление canonical JSON (gate §3.2): сортировка ключей, компактная
  кодировка, shortest round-trip чисел; отклонения от строгого JCS для экзотических
  чисел возможны — OP-3 не решён (см. INTEGRATION_NOTES.md §2).
- digest **отчёта** = sha256 точных записанных bytes (прецедент реестра
  evidence IT-8D); digest **профиля** = canonical JSON digest (gate §3.2).
  Фиксируется в INTEGRATION_NOTES как предложение для раунда 2.
- Бандл-манифест — private provisional DTO с именами полей
  `candidate_bundle.schema.json`; где реальный intake/GLM-5 DTO отличается,
  потребуется тонкий adapter на раунде 2 (никаких изменений в общих схемах).
- Geometry parsing — только OBJ/MTL на уровне fixtures; всё остальное —
  честный `unavailable`.

## 5. Блокеры (честно, не PASS)

| Блокер | Код/статус |
|---|---|
| Экспортный roundtrip-бэкенд (Blender worker op отсутствует, ADR §2) | `export_roundtrip` → `unavailable` (BLOCKED), backend-trait готов |
| Плеер клипов / анимационный backend | `animation_clips` → `unavailable` при верифицированных digest |
| Силуэт-компаратор (calibrated references) | принадлежит IT-8D reconstruction evaluator, здесь не реимплементируется (`UNSUPPORTED_PROFILE_KIND`) |
| Реальные форматы: glTF skins, .blend DNA, node groups, PBR-рендер | без парсера → `unavailable`, не pass |
| `.blend` intake (OP-2/B2) | вне зоны потока, не поддерживается |

## 6. Статус реализации

- Реализовано и протестировано: profile validation + applicability guard
  (включая optional-сторону, M2), 12 детерминированных проверок на
  fixture-форматах, assembly/свёртка, reservation/no-clobber реестр с
  live-reservation по диску (M1), verify (F2/F3/F19, H2), acceptance seam
  prerequisites (F5/F14/F19, H2), 9 позитивных + 14 adversarial тестов,
  smoke, кросс-валидация против GLM-1 схем.
- Не входит в этот PoC: реальный Blender worker, SQLite, CLI, миграции,
  canonical publisher (seam только), UE-проверки, style-review автоматизация
  (запрещена ADR §4.1).
