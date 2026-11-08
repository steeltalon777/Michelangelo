# INTEGRATION NOTES — GLM-6 Asset Validation

- **Дата:** 2026-11-08. **База:** `dev@b8615477b735c1f60c93518b59f8fe32613a7556`.
- Аудитория: интегратор (раунд 2/3), GLM-1 (расхождения контрактов), DeepSeek
  (независимая negative acceptance).

## 1. Adapter seam к canonical accept Core (не второй publisher)

Валидационный слой никогда не публикует. Единственная точка интеграции —
trait `CanonicalAcceptSeam` (`src/acceptance.rs`):

- `check_prerequisites(record) -> PrerequisiteReport` — реализовано
  `RegistryBackedSeam`: F14 (отчёт зарегистрирован, bytes digest сходится),
  F2-внутри-seam (inputs перечитываются), F5 (`bundle_binding_matches`),
  F19 (`profile_binding_matches`), F16-гигиена (verdict_sufficient только
  при pass; rejected-решение может ссылаться на любой отчёт).
- `submit_for_publication(record)` — **дефолтная реализация отказывает**
  (`ACCEPT_SEAM_NOT_AVAILABLE`). Публикацию выполняет будущий canonical
  accept Core (CAS `expected_current_revision`, no-clobber identity, один
  commit point на identity — ADR §5.3, gate §4). Никакой второй publisher
  в этом модуле не создан.
- `LocalReplayIndex` — append-only test device для детекции replay record id
  → другой bundle digest (`ACCEPTANCE_REPLAY`). Это не указатель и не
  publisher; настоящее хранилище решений остаётся за Core.

Для интегратора: canonical Core может переиспользовать `check_prerequisites`
как pre-publication gate и `registry::verify_registered_report` как
пересчитываемую основу; отчёты и реестр уже соответствуют правилам
D1.2/D1.3-аналогов IT-8D.

## 2. Предложения и расхождения для GLM-1 (раунд 2)

1. **Digest отчёта — bytes digest.** Реестр хранит sha256 точных записанных
   bytes отчёта (прецедент `michelangelo.evidence::registry`), и acceptance
   ссылается на тот же digest. Предложение: зафиксировать в контракте, что
   `validation_report digest = sha256(persisted bytes)`, а canonical JSON
   (RFC 8785) остаётся кодировкой документов, не байтов документов.
2. **Digest профиля — canonical JSON digest** (gate §3.2). Расхождение (1)
   vs (2) сознательное, но требует явной фиксации, иначе интегратор запутается.
3. **OP-3:** мой canonical encoder — serde_json с сортировкой ключей; полный
   JCS number-formatting не реализован (OP-3 не решён). Фикстуры используют
   простые десятичные числа, где форматы совпадают. До резолюции OP-3 digest
   профиля нельзя считать стабильным между реализациями.
4. **Условные required-проверки.** VALIDATION_MATRIX «R (при …)» выражена
   condition-токенами профиля (`constraints:connector`, `textures_declared`,
   `uv_preserved`, `export:glb`, `clips_pinned`, `materials_preserved`,
   `textures_preserved`). Предложение: стандартизировать словарь condition-
   токенов в схеме профиля, чтобы applicability был машинно проверяемым.
5. **not_required только для optional.** В моей реализации required-проверка
   не может получить `not_required` (только pass/fail/unavailable). Это
   stricter, чем текст CONTRACT_GATE §5 — предложение зафиксировать как
   правило (F16-безопасно).
6. **Registration-time structural guard.** Регистрация отказывает отчётам,
   нарушающим свёртку/not_required-инварианты до записи на диск — предлагаю
   включить в контракт реестра (сейчас это поведенческая гарантия потоков).
7. **Реестр per-workspace.** Реестр отчётов лежит внутри validation workspace
   (`validation/reports/index.json`); input digests относительны корню
   workspace. Для batch из 30 items каждый item — свой workspace/registry
   (соответствует «один item = отдельные artifacts/evidence»). Интегратору
   нужен единый каталог реестров, но не единый изменяемый файл.

## 3. Что требует интеграционных тестов (не доказано этим модулем)

| Аспект IT-8D / ADR | Почему нельзя доказать изолированно | Где тестировать |
|---|---|---|
| D1.5 сериализация решений под lock (accept/reject/publish) | seam не публикует; конкурентные интерливинги accept-vs-reject нужны на canonical Core | интеграция: временный workspace + SQLite + real publisher |
| D1.4/CAS `expected_current_revision` | CAS/no-clobber публикации — publisher-логика (GLM-3/publication) | интеграция: два sibling-кандидата на одной identity |
| D1.1 полный diff snapshot'а против подмены | intake строит snapshot; мой модуль проверяет digests бандла, но не строит SourceSnapshot | интеграция: intake (GLM-2) → validation цепочка |
| Crash/retry/fencing вокруг регистрации (F20) | мой реестр атомарен на файл, но crash runner-а и осиротевшие процессы — слой executor'а | интеграция: batch executor |
| Blender roundtrip export_roundtrip | нет worker op; backend-trait готов, реальная проверка требует Blender | stand smoke: pinned Blender headless |
| Проверка клипов в движении (animation playback) | нет плеера; digest-проверка не доказывает совместимость | stand smoke + отдельный animation gate |
| UE import/playback совместимость | вне scope этого потока; Blender-проверка ≠ UE-ready | отдельная UE integration |
| PBR appearance (renderer) | Workbench не проверяет PBR (ADR §2); material/texture checks здесь — presence/closure | stand smoke с EEVEE/Cycles по решению владельца |
| Стиль/художественное review | human procedure, автоматизация запрещена (ADR §4.1) | per-variant author review workflow |
| 15 sibling-вариантов: индивидуальность и свёртка batch-статусов | нужны generator (GLM-4) + executor (GLM-3) | интеграция раунда 3 |
| Перенос evidence/unknown references между revision (D2.7/D2.8) | мой модуль не владеет unknown/evidence store | интеграция с michelangelo_unknowns/evidence |

## 4. Проверки для DeepSeek (независимый reviewer)

1. **Анти-маскинг F1:** достаточно ли pinned `expected_asset_digest` (пин до
   генерации) + per-artifact byte digests, чтобы «отремонтированный манифест»
   никогда не легитимизировался новой evaluation? Контрпример: смена
   `content_kind` артефакта меняет asset_digest, но не байты — покрыто ли
   проверкой структуры манифеста? (Сейчас: меняет digest → отказ через пин.)
2. **Effective-transform проверка F9:** покрывает ли composition parent-chain
   + epsilon 1e-9 все случаи (scale-in-parent, negative scale, rotation)? Мой
   тест фиксирует translation; degenerate-матрицы и non-affine — вне PoC.
3. **Свёртка и not_required:** не маскирует ли `not_required` будущие
   `unavailable`? Мой guard: optional-only + applicability_ref + probe по
   контенту бандла; проверьте, что probe-семантика не позволяет «спрятать»
   недоступную проверку под not_required.
4. **Seam-граница:** убедиться, что `submit_for_publication` невозможен
   обходом (дефолт отказывает; других точек публикации нет), а
   `LocalReplayIndex` не превращается в конкурентный указатель.
5. **Draft-schemas соответствие:** emitted документы проходят
   `validation_report.schema.json` и `validation_profile.schema.json`
   (см. TEST_EVIDENCE §1.4) — подтвердить, что этого достаточно для раунда 2,
   или указать поля-расхождения.
6. **Digest-двойственность (§2.1/2.2):** bytes digest для отчёта, canonical
   digest для профиля — принять/отклонить как контрактное правило.
