# DSH independent review — ADR 0005, все шесть потоков

**Дата:** 2026-11-09 (Asia/Chita). **Reviewer:** DSH TechLead session.
**Режим:** verification-review, read-only по worktree; merge/push/commit не выполнялись.
**База всех веток:** `b861547` (IT-8D). **Текущий dev tip:** `68a2612` (IT-8E) — merged-tip регрессия ещё не выполнялась.
**Пакет:** `C:\Users\Макс\Downloads\ADR5_STREAM_REVIEWS_2026-11-08\` (самоотчёты + этот отчёт).
**Ревьюеры:** 4 независимых прохода на штатном маршруте `xiaomi/mimo-v2.6-pro`, effort high:
GLM-1/2/4 (один), GLM-3, GLM-5, GLM-6. `review_subagent` в сессии не зарегистрирован (`unknown tool`) — использован generic `subagent` с тем же пином; это конфигурационный дефект бандла, не роутинга.

## 1. Итог

| Поток | Коммит | Lead-прогон | Вердикт ревьюера | Блокеры |
|---|---|---|---|---|
| GLM-1 Contracts | `4597c43` | схемы: 13/13 + 13/13 + 10/10, exit 0 | **REWORK** | A held-out, B `not_required`, C фикстуры |
| GLM-2 Intake | `065acdf` | 35/35, clippy/fmt green | **PASS** | — (только LOW/INFO) |
| GLM-3 Batch | `a0b2817` | 28/28; fmt/clippy red | **REWORK** | F1 crash claim→insert, F2 hash подмена, F4 gate |
| GLM-4 Family | `a1df9a1` | 54/54, dry plan byte-identical; fmt red | **REWORK** | K fmt, L коллизия тегов |
| GLM-5 Blender | `e89704f` | smoke 6/7, worker.py не тронут, 6/6 bundles valid; worktree dirty | **REWORK** | HIGH-1 evidence rmtree, MEDIUM-2 CRLF-пины, MEDIUM-3 цифры |
| GLM-6 Validation | `592da2f` | 18/18; fmt/clippy red | **REWORK** | H1 gate, H2 `not_required` на required |

**Общий вывод: 6/6 потоков сданы, но `wave-verified` = НЕТ (YELLOW).** 1 PASS (GLM-2), 5 REWORK с точечными фиксами. Архитектурных переделок не требуется; все изменения — новые директории, IT-8D не ослаблен. **Контракт `draft-0.1` замораживать нельзя** до фиксов A/B (GLM-1) и H2 (GLM-6); интеграцию не начинать до P0-фиксов.

## 2. Как проверялось

- **Lead:** git log/status/stat/name-status по каждому worktree; `git diff --diff-filter=MD` (ни одного изменённого/удалённого существующего файла); прогоны: `check_schemas.py`, `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, dry plan ×2 + byte-compare, валидация 6 GLM-5 bundle'ов против схемы GLM-1, probe-контрпримеры.
- **Ревьюеры:** независимые read-only проходы; свои прогоны/пересчёты (seed derivation 15/15, GLB hashes, schema validation, cross-validation GLM-6, повтор gate).
- **Не делалось:** merge, push, rebase, правки worktree/evidence, повторный Blender smoke (по запрету; GLM-5 worktree сохранён dirty как есть).
- Логи Lead: `%TEMP%\dsh-adr5-review\{contracts-schemas,intake-cargo,family-cargo,family-clippy,batch-cargo,validation-cargo}.log`, `dry_raw.json`, probe-скрипты.

## 3. GLM-1 Contracts — REWORK

**Скоуп:** `4597c43`, 46 файлов, +2971; только `docs/adr5-contracts/`, `docs/adr5-streams/contracts/`, `schemas/adr5/`; дерево чистое. `check_schemas.py` → exit 0 (13 meta, 13 positive, 10 negative).

**Findings:**
- **A [HIGH]** `source_set.schema.json:78-106`: `allOf/if/then` на `contains` — «хотя бы один», а не «каждый». Проходят обе запрещённые формы: `IVR+build_input` рядом с `IVR+held_out`; `[base_candidate, IVR] + held_out`. Контрпримеры воспроизведены; описание `:104` заявляет неверное. **Фикс:** per-item `if/then` в `sources.items.allOf` (форма проверена: оба контрпримера отклоняются, positive проходит) + negative-фикстуры на оба направления.
- **B [MEDIUM]** `validation_report.schema.json:73-96`: тот же `contains` для «каждый `not_required` обязан иметь `applicability_ref`». Контрпример: два `not_required`, у одного нет ref → valid. **Фикс:** per-check `if/then` в `checks.items`.
- **C [MEDIUM]** 4/10 negative-фикстур содержат 66-символьные digest'ы; 2 из них (`report_not_required_verdict.json:7`, `source_snapshot_fbx.json:10`) падают на pattern раньше заявленного правила; `check_schemas.py:79-93` не пинит ожидаемый путь. **Фикс:** 64 hex + таблица `fixture → expected path`.
- **D [LOW]** `tool_versions.blender="latest"` проходит `recipe_lock.schema.json:39-44` при `floating_latest_allowed:false`. **E [LOW]** `candidate_bundle` без `expected_current_revision` валиден (`:8,18-22`). **F [INFO]** `cycle_check.acyclic` — самоутверждение. **G [INFO]** `LibraryEntry` не имеет draft-схемы.

**Вердикт:** содержание сильное, но до contract freeze — A/B обязательны, C — желательно.

## 4. GLM-2 Intake — PASS

**Скоуп:** `065acdf`, 22 файла, +4766; только `adr5-lanes/intake/` + `docs/adr5-streams/intake/`; дерево чистое. `cargo test` 35/35 (8+27), clippy 0, fmt clean.

**Подтверждено по исходникам:** хеши по фактически прочитанным байтам; immutable snapshot no-overwrite; запись только в workspace; сети нет; `convert()` всегда отказывает; unknown license ≠ permission; `#![forbid(unsafe_code)]`.

**LOW/INFO:**
- H: узкий TOCTOU при скане MTL (`snapshot.rs:743-752`) — fail-safe через `InputChanged`.
- I: Windows-вариант `negative_symlink_escape` — зелёный no-op (`tests/intake.rs:492-505`), честно помечен SKIP; «27 passed» = 26 реальных + 1 skip.
- J: crash между `create_dir` и записью `manifest.json` навсегда «отравляет» identity-каталог (`snapshot.rs:928-940,1038-1073`); нужен reconcile/cleanup.

**Открытые контрактные вопросы:** identity-нормализация (см. §9), межпроцессная сериализация (делегирована GLM-3).

## 5. GLM-3 Batch — REWORK

**Скоуп:** `a0b2817`, 19 файлов, +5487; только `michelangelo_batch/` + `docs/adr5-streams/batch/`; дерево чистое. `cargo test` 28/28 (13+4+7+4). `cargo fmt --check` exit 1; `cargo clippy --all-targets -- -D warnings` exit 101 (6 линтов).

**Findings:**
- **F1 [HIGH]** окно `claim item→running` (`executor.rs:328`) → `insert_attempt` (`:402`): при смерти runner в этом окне item остаётся `running` без attempt-строки; reconcile смотрит только `list_live_attempts` (`:762`), resume/retry клеймят `[pending, execution_failed, cancelled, blocked]` (`:311-327`), cancel — `pending/blocked` (`:1125-1134`) → пачка навсегда висит. Существующий инъекционный краш стоит после insert (`:416-421`). **Фикс:** свип `running` без живых attempt → `execution_failed`/перехватываемое состояние + тест.
- **F2 [HIGH]** `plan.rs:398-399` безусловно перезаписывает `input.sha256`; fingerprint считается до `pin_inputs` (`executor.rs:146` vs `:167`) → заданный hash не проверяется, immutable identity не покрывает фактические входы. **Фикс:** `Some(h) != actual` → `InputHashMismatch`; fingerprint от пинованной формы; negative-тест.
- **F3 [MEDIUM]** fresh-run не вызывает reconcile (`executor.rs:234-238`) и не сверяет drift в claim-loop (`:246+`) → исполнение по изменённым байтам при старом hash. **Фикс:** verify во всех режимах.
- **F4 [MEDIUM, gate]** fmt/clippy красные; в TEST_EVIDENCE не заявлены и не SKIPPED.
- **F5 [MEDIUM]** `valid_item_id` (`plan.rs:426-432`) пропускает `.`/`..`; `attempt_dir` (`:435-437`) уводит за runs root. **Фикс:** запретить `.`/`..`, первый символ `[A-Za-z0-9]`; negative-тест.
- **F6–F9 [LOW]** slot-busy оставляет `reserved` и помечается «owner died»; cancel пропускает worker в окне до `attempt.lock`; `attempts_total` = max, не сумма; таймаут лока после kill прерывает cancel без CAS.

**Запрошены тесты:** инъекция claim→insert; negative declared-hash; fmt/clippy exit 0; `item_id=".."`.

## 6. GLM-4 Family — REWORK

**Скоуп:** `a1df9a1`, 22 файла, +4601; только `crates/michelangelo_family/` + `docs/adr5-streams/family/`; дерево чистое. `cargo test` 54/54 (7+37+10); clippy green; `cargo fmt --check` exit 1; dry plan дважды byte-identical и равен committed примеру (sha256 `C44227FD…`).

**Findings:**
- **K [MEDIUM, gate]** fmt exit 1 (~130 мест: `src/bin/family_dry_plan_15.rs`, `fixture.rs`, `model.rs`, `plan.rs`, `wire.rs`, `tests/*`). **Фикс:** `cargo fmt` автором, повтор gate.
- **L [MEDIUM, новое]** документы несут теги GLM-1 `draft-0.1`, но схемам GLM-1 не соответствуют. Проверено: `source_set` — 6 ошибок, `recipe_lock` — 22, `family_recipe` — 23, `dry_plan_15` — 20. Потребитель, диспетчеризующий по тегу, упадёт. **Фикс:** временно собственные provisional-теги (как GLM-2: `draft-0` + `provisional:true`) либо контрактный маппинг раунда 2.
- **M [LOW]** role/usage coherence (`model.rs:200-230`) строже ADR §4.4: multi-role `base_candidate+style_reference` отвергается — безопасно, но решение раунда 2.

**Подтверждено:** seed derivation 15/15 пересчитаны ревьюером; lock digest, fixture pins, fingerprints, canonical JSON; hold-out правило строже GLM-1 и по ADR §4.4/§5.2 верно; cap/`INSUFFICIENT_UNIQUE`; пин доноров.

## 7. GLM-5 Blender — REWORK

**Скоуп:** `e89704f`, 55 файлов, +6492; только `adr5_blender_poc/` + `docs/adr5-streams/blender/`; `worker.py` не изменён (`git diff b861547 -- blender/worker.py` = 0). Smoke: 6 positives `executed`, 7/7 negatives, `source_hashes_unchanged=true`; 44 pure-теста OK; 6/6 candidate bundle валидны против схемы GLM-1 (проверено Lead).

**Состояние:** worktree **dirty** — 13 файлов в `adr5_blender_poc/evidence/` изменены повторным smoke оркестратора после коммита. README/отчёт «чисто» — устарело; DSH не откатывал и не перегенерировал.

**Findings:**
- **HIGH-1** `run_smoke.py:460-462` делает `shutil.rmtree(EVIDENCE_DIR)` + `mkdir` при каждом запуске; `TEST_EVIDENCE.md:129-130` заявляет обратное. Committed evidence уничтожается штатной командой `python -m unittest discover`. **Фикс:** писать в `evidence/run-<ts>/` с no-clobber, tracked `evidence/` — замороженный снимок; исправить ложное предложение.
- **MEDIUM-2** пины шаблонов/worker — по CRLF-байтам worktree: `v0_palette_master` blob `6d080a13…` vs pin `50e69342…`; `v1_roof_assembly` blob `4a1c21c2…` vs pin `a47aa9ea…`; `worker.py` blob `28dfc5c9…` vs `9c7c24f3…`; `v0_textured_probe` совпадает. На LF-checkout registry даст `TEMPLATE_HASH_MISMATCH` на 2/3. **Решение:** нормализованные LF-байты или `.gitattributes eol=crlf` + контрактное правило digest.
- **MEDIUM-3** цифры в `TEST_EVIDENCE`/`CAPABILITY_MATRIX` — из другого прогона: blend `a09e3e0f…`/`31629600…` vs HEAD `9a4da439…`/`9bf005d0…`; bundle `7eb06c65…` vs HEAD `908bd7f5…`; RAM/GPU `174.5–206.9 / 1397–1470` vs HEAD `160.4–206.6 / 1443–1521`. **Фикс:** перегенерировать §2/§3 из одного снимка с явным `generated_at`.
- **LOW-4..7:** материалы сравниваются только соседние пары; `PARAMETER_SCOPE_PROTECTED` — мёртвый код; `tail_operations`/`slot.field` не структурно валидируются (остаётся guard worker); `tests/helpers.py:14-19` хардкодит contracts-worktree.

## 8. GLM-6 Validation — REWORK

**Скоуп:** `592da2f`, 29 файлов, +7529; только `michelangelo_asset_validation/` + `docs/adr5-streams/validation/`; дерево чистое. `cargo test --offline` 18/18 (9 adversarial + 9 positive); `cargo fmt --check` exit 1; `cargo clippy --offline --all-targets -- -D warnings` exit 101 (unused imports: `examples/dump_validation_report.rs:9`, `tests/adversarial.rs:14`).

**Findings:**
- **H1 [HIGH, gate]** fmt/clippy красные. TEST_EVIDENCE §1.2 заявляет `cargo clippy --offline -> 0 warnings` — это lib-target, не gate `--all-targets -D warnings`; fmt не запускался. **Фикс:** удалить 2 импорта, `cargo fmt`, повторить gate.
- **H2 [HIGH]** «`not_required` только для optional» не обеспечен: `report.rs:100-114` проверяет только непустой `applicability_ref` (профиль не передан); `check.rs:176` fold нейтрален к `not_required`; `registry.rs:437-448` coverage проверяет лишь наличие записи; seam вызывает `verify` с `None` (`acceptance.rs:211`). Контрпример: `payload_integrity: not_required` → assemble OK → register OK → verify OK → seam `verdict_sufficient=true` для accept. **Фикс:** в `verify` с текущим профилем требовать `result ∈ {pass,fail,unavailable}` для required-проверок; тот же guard при регистрации; тест.
- **M1 [MEDIUM]** «live reservation» не проверяется по диску: `registry.rs:228-236` сравнивает только `reservation.report_id` переданной структуры; `Reservation::load` не вызывается, поля публичные → фабрика in-memory проходит. **Фикс:** `Reservation::load(root, id)` в начале `write_and_register`.
- **M2 [MEDIUM]** probe гасит условные проверки: optional `RequiredWithCondition("constraints:clearance")` без порога → `not_required` (`evaluate.rs:236-312`), токен валидируется только для required (`profile.rs:312-341`). **Фикс:** конфликт applicability или не-exempt при присутствии токена.
- **M3 [MEDIUM]** комментарий «strict round-trip» неверен: `deny_unknown_fields` только у `AcceptanceRecord`; `ValidationReport/Profile/CheckRecord` молча отбрасывают ghost-поля; digest профиля — по typed-структуре. **Фикс:** `deny_unknown_fields` или правка комментария + контрактное правило.
- **M4 [MEDIUM]** `uv_preservation` без пина/порога → pass по одному наличию vt-строк (`appearance.rs:233-247`), хотя для `family_rig_compatible` это required. **Фикс:** нет пина/порога → `unavailable`.
- **L1–L5 [LOW]:** unknown parent в `world_transforms` тихо = root (`parsing.rs:304-308`); mixed path/digest в rig-evidence (`rig.rs:162-165`); третья digest-конвенция (acceptance canonical) не описана; гонка append-only index (`registry.rs:242,322-324`); несоответствия комментарий/код (`profile.rs:274-279`, `ids.rs:138-150`, мёртвый `VALIDATION_VERDICT_NOT_PASS`, `detail` >1000).

**Подтверждено:** 18/18 тестов; закрытый реестр 18 check_id; applicability-матрица/конфликты; fold fail>unavailable>pass; `Verdict` без `not_required`; F1 anti-masking держится на обязательности пина (без пина → unavailable, не pass); F2/F5/F8/F9/F13/F14/F16/F19; no-clobber запись/повтор отчёта; seam не публикует; cross-validation emitted profile/report против GLM-1 — **REPORT_OK, PROFILE_OK** (независимый повтор).

## 9. Перекрёстные решения (раунд 2 / owner)

1. **Hold-out.** Правило GLM-4 верное (per-item: IVR ⇒ held_out; held_out ⇒ только IVR). Схему GLM-1 чинить per-item `if/then` (A). Иначе контракт фиксирует неработающий инвариант.
2. **Draft-теги.** GLM-4 использует теги GLM-1 на невалидных документах (L) — до раунда 2 перейти на собственные provisional-теги (как GLM-2) или согласовать маппинг. GLM-6 emitted-документы, наоборот, **проходят** схемы GLM-1 (проверено).
3. **Identity GLM-2.** `SHA-256("…/identity/0\n" + format + … + sorted deps)` (`snapshot.rs:165-193`); в контракте GLM-1 правила нет. Решить: принять это как контракт (и зафиксировать исключение limits/tool-версий) или маппить в `snapshot_sha256`.
4. **Межпроцессная сериализация intake.** GLM-2 даёт atomic no-clobber, но не межпроцессный lock; GLM-3 admission/file-locks есть. Перенос семантики на intake — раунд 2; до этого intake single-process-safe.
5. **Retry GLM-3.** «Бюджет на цикл» (реализовано) vs «на item навсегда» — owner-decision; зафиксировать в контракте.
6. **Evidence GLM-5.** Smoke обязан писать в disposable workspace/no-clobber; tracked evidence — immutable snapshot. Плюс политика hash: LF-нормализация или `.gitattributes eol=crlf`.
7. **Digest-конвенции.** Отчёт — bytes digest; профиль — canonical digest; acceptance — canonical (L3). Нужна явная таблица «всё persisted evidence = sha256(bytes), кроме …» либо наоборот; OP-3/JCS остаётся открытым.
8. **`not_required`.** GLM-1 схема (B) и GLM-6 реализация (H2) должны сойтись на одном правиле: `not_required` — только optional, и required-проверка не может дать `not_required` ни в assemble, ни в verify. Это анти-маскинг `unavailable`, не косметика.
9. **База интеграции.** Ветки от `b861547` (IT-8D), dev уже `68a2612` (IT-8E). Merge/rebase и полная merged-tip регрессия — отдельный шаг; сейчас не выполнены.

## 10. Регрессионный gate (per-crate; standalone не входят в корневой workspace)

| Поток | Test | fmt | clippy `-D warnings` |
|---|---|---|---|
| GLM-1 | schema check exit 0 | n/a | n/a |
| GLM-2 | 35/35 | clean | clean |
| GLM-3 | 28/28 | **FAIL** | **FAIL (6)** |
| GLM-4 | 54/54 | **FAIL** | clean |
| GLM-5 | 6 positives/7 negatives + 44 pure | n/a | n/a |
| GLM-6 | 18/18 | **FAIL** | **FAIL (2)** |

`cargo fmt --all` в корне не покрывает standalone-крейты, пока они не добавлены в workspace; после интеграции gate нужно прогнать на merged tip.

## 11. Приоритеты фиксов

**P0 — до contract freeze / интеграции:**
1. GLM-1 A (held-out) + B (`not_required`) + C (фикстуры).
2. GLM-3 F1 (crash claim→insert), F2 (declared hash подмена).
3. GLM-6 H2 (`not_required` на required).
4. GLM-5 HIGH-1 (evidence rmtree/immutability).
5. GLM-4 L (tag collision).

**P1 — gate/medium:**
6. fmt/clippy: GLM-3, GLM-4, GLM-6; TEST_EVIDENCE — добавить результаты.
7. GLM-5 MEDIUM-2 (CRLF-пины) и MEDIUM-3 (цифры).
8. GLM-6 M1 (reservation по диску), M2 (probe/conditions).
9. GLM-3 F3 (fresh-run drift), F5 (`item_id=".."`).

**P2 — low/info:** F6–F9, D/E/F/G, H/I/J, LOW-4..7, L1–L5, M3/M4.

После фиксов — один delta-review (не новый full pass), затем merged-tip regression и решение owner о contract freeze.

## 12. Что не проверено / остаточные риски

- Нет merged-tip регрессии, нет merge/push (по контракту).
- Нет реального Blender/UE smoke на интегрированной ветке; GLM-5 smoke — PoC-level, один Blender-процесс, evidence dirty.
- Нет реальных backend'ов validation (export roundtrip, clip playback, PBR, UE) — честно BLOCKED.
- Конкурентность/crash сверх тестов GLM-3 не проверялась (F20/D1.4/D1.5 — зона Core/executor).
- GLM-5 worktree остаётся dirty (13 evidence-файлов) — это ожидаемое состояние для ревьюера; перед интеграцией нужно решение: откат к HEAD или коммит новой генерации после фикса HIGH-1.
- `review_subagent` не зарегистрирован в сессии — reviewer-пресет не поднялся; использован generic `subagent` с пином `xiaomi/mimo-v2.6-pro`. Для следующих итераций — перезапуск/починка композиции бандла.

## 13. Итоговая рекомендация

Не мержить и не замораживать контракт. Пакет даёт честные, воспроизводимые PoC, но `wave-verified` нет: 1 PASS, 5 REWORK. Все блокеры — точечные (схемные кванторы, crash-окно, hash-проверка, evidence-запись, colliding tags, not_required guard, fmt/clippy). После P0+P1 и одного delta-review пакет можно переводить в `PASS с условиями` (PoC-level, backend'ы BLOCKED, merged-tip regression отдельно).
