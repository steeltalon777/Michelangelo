# TZ: IT-8 — Проверяемое закрытие неизвестностей и безопасная приёмка локального изменения

Status: draft (rev. 3 — после второго круга замечаний владельца от 2026-11-07)
Branch: `dev` (no push, no branch switch; commits only after verified checks and a GREEN review)
Owner: TechLead (orchestrator); implementation by worker subagents; independent review before commit.
Baseline: dev `36f1bac` (IT-7). Depends on ADR 0004 stages A–D2 (IT-4…IT-7).

## 0. Основание и честность reproduction

Аудит Astra (read-only, dev `36f1bac`) принят как основание итерации. Lead самостоятельно сверил
замечания по исходникам и воспроизвёл их (`target/techlead/IT-8/audit_reproduction.py`, отчёт
`target/techlead/IT-8/audit-reproduction.json`; проверочный скрипт требует подтверждения **всех**
сценариев и возвращает ненулевой код при любом расхождении).

| # | Замечание | Воспроизведение (фактическое) |
|---|---|---|
| A1/A3 | нет неизменяемого снимка проверяемой геометрии | подмена `candidates/<id>/geometry/lid.obj` внутри региона → `accept` публикует подменённые байты; decision/store не хранят hash проверяемой геометрии |
| A1 control | смена исходной папки кандидата после submit | на стор не влияет (копия при submit) — единственный позитивный инвариант |
| A2 | проверка базовой ревизии/сериализация публикации | `STALE_BASE` на accept есть, но **гонка воспроизведена**: 12 параллельных `accept` из одной базы → **3 успешных приёма и 3 новые ревизии**; lock/CAS отсутствует |
| A4 | идемпотентность accept | повторный `accept` перезаписывает `decision.json` принятого кандидата `accepted/s0002` → `rejected`, `index.status=rejected` |
| A5 | rollback геометрии и unknown-состояния | rollback двигает только `current.json`; unknown/evaluation/evidence-состояния в run нет |
| B | оценка кандидата/ревизии | после accept нет рендера/метрик новой геометрии; `quality_report` не пиннит камеры/входы |
| C1 | unknown по part_id | `u_glass_thickness` остаётся `resolved` при **несвязанном** supplement на том же part_id (`unrelated_curve_on_same_part`), а не только при чрезмерной толщине; `stub` (`u_flame_shape`) отсутствует в unresolved-списке |
| C2 | uncertainty_policy | пакет с `require_author_acceptance=true` + workspace `auto_accept` + `candidate.task.require_author_acceptance=false` → `submit` сам публикует ревизию; package-policy в edit/assistant не читается |
| A2 race (static) | отсутствие механизма сериализации | `Mutex/RwLock/flock/try_lock/.lock` в edit sources — 0 совпадений; подтверждено также гонкой выше |

Это reproduction «до», а не runtime-проверка после исправлений; regression-тесты после фикса
обязаны доказывать инварианты, а не просто отсутствие старых симптомов.

## 1. Вне scope

- Изменение входного формата `michelangelo.input-package/0` (v1): не требуется; новое состояние —
  в run-store. Если реализация упрётся в необходимость v1, работа останавливается и требуется
  отдельное согласование.
- Встроенный LLM-провайдер, сеть, автоматический push/merge, новые production-возможности вне IT-8.
- Изменения JSONL wire-контракта (CLI + библиотеки, как IT-4…IT-7).
- Переписывание исторических solver/quality reports (они immutable; новые отчёты — отдельно).

## 2. Stage 0 — обязательный контракт до кода

До IT-8A выполняется Stage 0 (Lead, docs + evidence; без изменения поведения):

1. `docs/specs/VERIFICATION_STATE.md` — единый контракт состояния и приёмки:
   - один авторитетный указатель и bundle-ревизия (§3), digests, схемы snapshot/evaluation/
     confirmation/resolution/bindings, протокол lock, порядок приоритетов политик;
   - поведение для старых run/candidate без новых evidence: **никаких молчаливых `verified`**;
     старый кандидат без snapshot → `SNAPSHOT_MISSING` (exit 1), старые принятые ревизии
     остаются читаемыми, но новое принятие требует нового evidence;
   - идемпотентный повтор `accept` **после rollback**: возвращает исторический результат
     принятия, но сцену обратно не переключает;
   - отсутствие контрольных данных (held-out камер/GT) → статус `unavailable`, не `pass`;
   - held-out камера должна добавлять независимое ограничение: вид в том же направлении, что
     source-вид (`dot(dir_i, dir_j) > 0.9`), не даёт ограничения → `unavailable` с диагностикой;
     противоположные ортографические виды (`dot ≈ -1`, зеркальный силуэт) допустимы — именно они
     служат held-out контролями proof-датасетов.
2. Уточнённый reproduction «до» (§0) с постусловиями по каждому сценарию и ненулевым кодом при
   расхождении — уже выполнено; скрипт/отчёт в `target/techlead/IT-8/`.
3. Матрица failure-тестов: инвариант → тест → ожидаемый код/состояние (A/B/C), включая
   crash-recovery, гонку двух процессов, подмену подтверждённого кандидата, частичный прогресс.
4. Замороженный API-набросок `michelangelo_evidence` (DTO/digest-функции) — чтобы IT-8A не
   реализовывал контракт B заново.
5. Решения владельца (2026-11-07, зафиксированы в контракте §11): повыводовая регрессия 0.01 IoU
   плюс абсолютные пороги (mean — сводка); `author_geometric_conditions=pass` только по фактической
   проверке геометрии/проверяемой гарантии (исполнение supplement само по себе не pass); IT-8 ничего
   не удаляет автоматически — принятые bundle и транзитивные зависимости сохраняются (GC — отдельно).

## 3. Единая авторитетная bundle-ревизия (без двух указателей)

**Инвариант:** принятая ревизия однозначно связывает геометрию, unknown-состояние, evaluation,
bindings и авторские подтверждения; одно атомарное переключение выбирает весь согласованный набор.

```text
<run>/current.json                 # ЕДИНСТВЕННЫЙ авторитетный указатель -> scene_revision
<run>/scene/sNNNN/
  scene.json                       # schema + components + verification bundle
  components/*.obj                 # полный набор геометрии ревизии
<run>/verification/
  snapshots/<candidate_id>/        # immutable: ПОЛНЫЙ результат изменения (база + изменения)
    geometry/*.obj                 # все компоненты: неизменённые копируются из базы
    manifest.json                  # michelangelo.geometry-snapshot/0
  evaluations/<eval-id>/report.json# immutable; digest регистрируется Core
  evaluations/index.json           # append-only реестр {evaluation_id,file_sha256,target_digest,...}
  bindings/<binding-id>.json       # immutable версии bindings
  confirmations/<conf-id>.json     # immutable авторские подтверждения
  resolutions/<resolution-id>.json # immutable предложения разрешения
  unknowns/revisions/sNNNN.json    # immutable снимок unknown-состояния ревизии
  unknowns/events.jsonl            # append-only аудит bind/propose/confirm/resolve/reject/rollback
  run.lock                         # OS advisory lock (см. §4)
candidates/<id>/
  decisions/NNNN-<kind>.json       # append-only история решений
  decision.json                    # latest-зеркало (совместимость)
```

`scene.json` содержит `verification`:
`{geometry_digest, bindings{version,path,sha256}, unknowns{revision,path,sha256},
evaluations[{evaluation_id,path,sha256}], confirmations[{id,path,sha256}],
resolutions[{id,path,sha256}], policies{package_sha256,workspace_policy_sha256|null},
tool_versions, created_at}`.

Правила:
1. Публикация = запись revision-каталога (temp+rename) + атомарная замена `current.json` **после**
   проверки всех referenced-документов и их хешей. Падение до замены указателя оставляет старый
   bundle авторитетным; частично видимых состояний нет.
2. `verification/unknowns/revisions/<rev>.json` — immutable и выбран ревизией; отдельного
   мутируемого указателя на unknown-состояние нет (кэш допустим только как проверяемая копия).
3. `unknown resolve` до `accept` создаёт **предложение разрешения** (`resolutions/<id>.json`),
   но не меняет текущую сцену. Unknown закрыт только в ревизии, чей bundle ссылается на resolution.
4. Bindings версионированы (`bindings/<binding-id>.json`); ревизия ссылается на версию. Откат
   геометрии не оставляет новые привязки активными: активны привязки, на которые ссылается target-ревизия.
5. Подтверждение автора (`confirmations/<conf-id>.json`) привязано к
   `(unknown_id, claim_digest, target_geometry_digest, evaluation_id, evidence_sha256)` и не может
   быть переиспользовано для другой геометрии/варианта.
6. Идемпотентный повтор `accept` после rollback: возвращает исторический accepted-результат и
   ревизию, сцену обратно **не** переключает.

## 4. Сериализация: OS advisory lock

- Lock: `<run>/verification/run.lock`, открывается и берётся `std::fs::File::try_lock()` (Rust 1.96,
  проверено probe `LOCK_OK`); OS освобождает lock при завершении процесса. Никакого TTL-отбора.
- Занят → `STORE_BUSY` (exit 2), без ожидания в CLI (короткая критическая секция).
- Под lock: submit-snapshot insert, accept publish, rollback, bind/confirm/resolution publish,
  регистрация evaluation. Evaluation/рендер выполняются **вне** lock; accept под lock проверяет
  неизменность базы и всех зависимостей непосредственно перед публикацией.
- Lockfile после освобождения остаётся (содержимое `{pid, operation, since}` — диагностика).
- Crash-recovery: kill процесса между rename ревизии и заменой указателя → old bundle авторитетен;
  повторная операция доигрывается; тест обязателен.
- Все конкурирующие изменения (accept, rollback, bind, confirm, resolution) соблюдают один lock.

## 5. IT-8A — Acceptance integrity

Write scope: `crates/michelangelo_edit/**`, `crates/michelangelo_cli/src/edit_cmd.rs`,
`crates/michelangelo_cli/tests/edit_cli.rs`.

A1. Snapshot полного результата: `submit` создаёт `verification/snapshots/<candidate_id>` с
полным набором геометрии (неизменённые компоненты копируются из базы) и `manifest.json`:
`base{scene_revision, geometry_digest}`, `files[{component,path,sha256,source: base|candidate}]`,
`aggregate_sha256`, `candidate_sha256`, `task_sha256`, `package_sha256`, `bindings_sha256`,
`package_policy_sha256`, `tool_versions`, `created_at`. Decision хранит `snapshot_id`,
`aggregate_sha256` (serde default для старых файлов). Публикация и evaluation используют
только снимок.
A2. `StoreLock` (§4): submit/accept/rollback/bind/confirm/resolution; accept после валидации
перечитывает `current.json`, требует `candidate.base == current`, затем publish + set current;
`STORE_BUSY` при занятом lock. Детерминированный regression: тестовый hook между validate и
publish меняет current → `STALE_BASE`, ревизия не создана; двухпроцессный тест lock (ребёнок
держит lock → BUSY; kill → успех).
A3. Защита от подмены: accept сверяет файлы снимка с manifest и `candidate`/snapshot digest;
несовпадение → `EVIDENCE_MISMATCH`. Подмена `candidates/<id>/geometry` после submit не влияет
на публикацию (публикуется снимок) и обнаруживается отдельной проверкой snapshot-manifest.
Evaluation digest берётся из Core-реестра `evaluations/index.json`, а не из self-report.
Отказ в accept (`SNAPSHOT_MISSING`, `EVIDENCE_MISMATCH`, `STALE_BASE`) дописывается в историю
как `accept-refused` с кодом; latest-зеркало кандидата не перезаписывается (это попытка, не
новое решение), поэтому подмена остаётся аудируемой.
A4. Append-only история: `decisions/NNNN-<kind>.json` (`submit|accept|accept-idempotent|reject|
reject-idempotent|reject-refused|accept-refused`); `decision.json` — latest-зеркало. Повторный
accept: exit 0, та же ревизия, запись `accept-idempotent`, `index.status` не портится.
Reject принятого: exit 1 `CANDIDATE_ALREADY_ACCEPTED`, история не переписывается. Повторный
reject идемпотентен. Ошибки store типизированы (`StoreBusy`→2, `EvidenceMismatch`/`SnapshotMissing`/
`StaleBase`→1); код печатается в stderr.
A5. Bundle/rollback: при init создаётся `unknowns/revisions/s0001.json` из immutable
`report.json` (статусы `open|stub_pending`; нет report → пусто); каждая публикация пишет
immutable снимок unknown-состояния, на который ссылается `scene.json`; rollback под lock
выбирает target-ревизию (геометрия + unknown-состояние + bindings + подтверждения),
пишет событие `rollback`; исторический report hash не меняется.

Failure-тесты A: подмена store-геометрии; подмена snapshot; lock занят → BUSY; lock освобождается
при kill; гонка двух процессов (не более одной публикации из базы); двойной accept; reject-after-
accept; двойной reject; rollback состояния; crash между publish и pointer; hash historical report;
accept старого кандидата без snapshot → `SNAPSHOT_MISSING`; отказ accept записывается в историю
(`accept-refused`) без изменения latest-зеркала.

## 6. IT-8B — Candidate evaluation

Write sub-iterations: `crates/michelangelo_evidence/**` (Stage 0 контракт → DTO/digests/verification,
без рендера), `crates/michelangelo_eval/**` (evaluator), затем `crates/michelangelo_edit/**` (gate)
и CLI `evaluate`.

- `michelangelo evaluate --run <run> (--candidate <id>|--revision sNNNN) [--evaluation <pkg>]
  [--blender <bin>] [--json]` — рендер фактической новой геометрии снимка/ревизии существующими
  worker-операциями; source-камеры из pinned копии пакета + held-out из eval-пакета (если дан;
  проверка несовпадения направлений); отчёт в `verification/evaluations/<id>/`, Core регистрирует
  digest в `evaluations/index.json`.
- Схема `michelangelo.evaluation-report/0`: target{kind,id,aggregate_sha256}, inputs{package_sha256,
  views[{view_id,camera_digest,mask_sha256,resolution,held_out:bool}], evaluation_package_sha256|null},
  thresholds, checks[], `visible{views[], mean_iou, min_iou, verdict}`, `hidden{claims[], verdict}`,
  `acceptance_basis`, `tool_versions`, `created_at`.
- Раздельные результаты (не смешивать):
  1. `projection_conformance` (соответствие исходным проекциям): `pass|fail|unavailable`;
  2. `author_geometric_conditions` (заданные автором измеримые условия, например толщина 2 мм по
     mesh без GT): `pass|fail|unavailable`;
  3. `independent_ground_truth` (независимый GT, если передан): `pass|fail|unavailable`;
  4. `author_hidden_decision` (решение о скрытой форме): `accepted|rejected|pending` — **отдельное
     основание принятия, не измерение**.
  Отсутствие GT не означает fail: `unavailable`; при этом отсутствие GT **не** даёт
  автоматического `verified` для скрытого. Уже принятые в пакете supplements не требуют
  повторного авторского подтверждения, **но** `author_geometric_conditions = pass` требует
  проверки фактической геометрии (или проверяемой гарантии построения, сохраняющей условие):
  исполнение правила само по себе не доказывает размер (например, заказанная оболочка 2 мм на
  voxel-сетке 6 мм может выполниться корректно и не попасть в допуск). `fail`/`unavailable`
  обязательного условия не считается пройденным; необязательное может остаться `unavailable`
  только с явной пометкой и диагностикой. Авторское подтверждение после evaluation хранится
  отдельно и не переписывает immutable evaluation-report.
- Accept-gate (B): `accept --evaluation <report>`; проверки: зарегистрированный digest,
  target aggregate, input digests, `projection_conformance=pass`, каждое **обязательное**
  `author_geometric_conditions` = `pass` (необязательное — `unavailable` допустимо только с
  явной пометкой), **повыводовая** регрессия: каждый source-вид не хуже baseline более чем на
  0.01 IoU (mean — только сводка), плюс абсолютные пороги Stage B; baseline и кандидат
  сравниваются при одинаковых входах, камерах и настройках;
  коды `EVALUATION_MISSING`, `EVALUATION_STALE`, `EVALUATION_UNREGISTERED`, `INPUT_CHANGED`,
  `VISIBLE_FAILED`, `AUTHOR_CONDITION_FAILED`, `REGRESSION` (повыводовая), `NO_GROUND_TRUTH`
  (только когда для конкретного claim требуется GT, а он недоступен — не общий запрет).
- Hidden-подтверждения не нужны для **принятия геометрии** (см. §7); они нужны для resolution.
- Исторические `report.json`/`quality_report.json` не перезаписываются.

Failure-тесты B: нет evaluation; незарегистрированный/подменённый evaluation; stale target;
inputs изменены; visible fail; author condition fail; GT отсутствует и требуется конкретным
claim; регрессия; успешный путь; held-out камеры совпадают с source → `UNAVAILABLE` диагностика.

## 7. IT-8C — Unknown resolution + исполнение политик

Write scope: `crates/michelangelo_edit/**` (unknowns/state machine), `crates/michelangelo_assistant/**`
(task linkage), CLI (`unknown …`, `assistant task --unknown`), `tools/solver/**` и
`crates/michelangelo_input/**` (узкое исполнение входных политик), тесты.

C1. Реестр и версии: `unknown sync --run` импортирует unknown'ы из immutable solver-report
(статусы `open|stub_pending`); bindings — явные и версионированные (`unknown bind --unknown <id>
--components <a,b,...> --region min=max --by <author>`); part_id сам по себе не закрывает unknown.
C2. Авторское подтверждение: `unknown confirm --run --unknown <id> --by <author> [--evidence <file>]
[--claim <text>]` создаёт immutable `confirmations/<id>.json`, привязанный к claim digest +
target geometry digest + evaluation id + evidence hash (автор — владелец/уполномоченный оператор;
агент не подтверждает свою работу от имени owner).
C3. Resolution: `unknown propose --run --unknown <id> --candidate <id> --evaluation <eval-id>` и
`unknown resolve --run --unknown <id>` (или `edit accept --resolve <id>`) создают immutable
`resolutions/<id>.json`; закрытие происходит **только** в ревизии, чей bundle ссылается на resolution.
Условия resolution: binding; snapshot aggregate == evaluation target; `projection_conformance=pass`;
каждое **обязательное** `author_geometric_conditions=pass` (необязательное — только `unavailable`
с явной пометкой); авторское подтверждение принято для этого claim и этой геометрии;
`uncertainty_policy` пакета исполнена (`require_author_acceptance`, `allowed_completion_rules`,
`stub_policy`); типы/правила допустимы; все проверки пишутся в отчёт.
C4. Два независимых результата:
  - `edit accept` принимает **изменение геометрии**, даже если связанный unknown остаётся открытым
    (промежуточная ревизия допустима); unknown не закрывается автоматически;
  - `unknown resolve`/`accept --resolve` закрывает **конкретный unknown** со своими основаниями.
  Нет универсального запрета на правку компонента с открытым unknown; нет force/обхода.
C4b. **Инвалидация закрытий (обязательно для каждого нового bundle, правило вводится уже в A):**
  - если ревизия меняет зависимую геометрию, binding или условия, прежний resolution не переносится
    как действующий; unknown становится `open` с причиной `evidence_stale`;
  - прежние evidence/confirmations остаются в истории, но не выдаются за действующие для нового
    варианта;
  - незатронутые решения переносятся только при совпадении всех dependency digests (target
    components, binding version, claim/conditions);
  - rollback восстанавливает resolutions, действовавшие в target-bundle.
C5. `unknown show --run [--revision]` — состояние выбранной ревизии: unknown'ы, bindings, статусы,
evidence refs, confirmations; stubs остаются в текущем unresolved-списке до resolution; solver-report
не переписывается.
C6. Input-policy enforcement в построении (узкая правка в IT-8):
  - solver не применяет непринятые AI-supplements, если пакет требует author acceptance; такие
    supplements дают диагностику `SUPPLEMENT_NOT_ACCEPTED_UNRESOLVED` и не закрывают unknown;
  - `stub_policy: forbidden` запрещает заглушки (unresolved/ошибка, не silent stub);
  - `allowed_completion_rules` ограничивает исполняемые supplement-виды;
  - приоритет (для авторского геометрического замысла): принятые в пакете author-решения >
    package uncertainty_policy > workspace policy > candidate/task flags; кандидат никогда не
    ослабляет пакетные ограничения;
  - **ограничения безопасности** (доступ к файлам/сети, бюджеты, protected scope, sandbox,
    секреты) действуют независимо и не могут быть ослаблены package/candidate/workspace policy;
  - write scope включает `tools/solver/**`, `crates/michelangelo_input/**`; regression-тесты:
    unaccepted AI supplement не применяется, accepted применяется, forbidden stub, allowed rules,
    приоритет политик, security-ограничения не ослабляются, e2e build.
C7. `assistant task --unknown <id>`: bounded task включает `unknown_id`, target components/region
из binding, отфильтрованные references, acceptance criteria; MCP `get_task` отдаёт это агенту;
MCP по-прежнему не применяет.

Failure-тесты C: закрытие по part_id; несвязанный supplement на том же part_id; stub в unresolved;
`stub_policy: forbidden`; `allowed_completion_rules`; package `require_author_acceptance` нельзя
обойти кандидатом/auto_accept; resolution без confirmation; confirmation на другой геометрии;
чужой/устаревший evaluation; resolution до evaluation; частичное принятие (геометрия принята,
unknown открыт); rollback состояния; старый run без evidence → `unavailable`, не `verified`.

## 8. Обязательная демонстрация

Внешний LLM-агент (DSH-субагент через CLI/MCP-stdio, без встроенного провайдера):
`get_task` → создаёт **осмысленную деталь** вместо `stub_u_flame_shape` (не куб: ≥ 50 вершин,
сужающийся/симметричный профиль, bbox внутри binding-региона, заполнение ≥ 60 % проекции заглушки)
→ `submit_candidate` → Michelangelo: snapshot → evaluation (projection + author conditions +
hidden basis) → владелец/оператор выполняет `unknown confirm` → `unknown resolve` → `edit accept
--evaluation` → ревизия с resolution в bundle; `unknown show` показывает `resolved` + evidence,
stub исчез из unresolved; исторический solver-report не изменён.
Контроль: куб (8 вершин) заменяет заглушку → unknown **не закрывается** автоматически; решение о
геометрии принимает автор (куб может быть сознательным дизайном); resolution без подтверждения
невозможен; `accept` без evaluation — невозможен.

## 9. Критерии приёмки

1. Stage 0: контракт `docs/specs/VERIFICATION_STATE.md` + матрица failure-тестов; старые run/
   candidate без evidence не дают молчаливого `verified`.
2. Все 9 аудит-дефектов воспроизведены до реализации (см. §0) и закрыты regression-тестами после;
   A2-гонка проверяется двумя процессами (не только статикой).
3. A/B/C: инварианты §3–§7 подтверждены failure-тестами; единый указатель/bundle; OS lock без
   TTL-отбора; snapshot полного результата; Core-зарегистрированный evaluation digest; раздельные
   результаты проверок; частичное принятие; resolution-цепочка; исполнение входных политик.
4. Демо выполнено, evidence в `target/techlead/IT-8/` (транскрипт MCP, отчёты, хеши, решения,
   контроль куба).
5. Ladder: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace`, `cargo build -p michelangelo_cli`; Python/solver/CLI/e2e проверки;
   объём тестов определяется инвариантами, а не числом.
6. Package format остаётся v0; wire-контракт JSONL не изменён; нет встроенного LLM/сети/push.
7. Docs: `docs/specs/VERIFICATION_STATE.md` (Stage 0), обновления `LOCAL_EDIT.md`,
   `docs/specs/EVALUATION.md`, `docs/specs/UNKNOWNS.md`, ROADMAP/TASKS/CHANGELOG/AI_CONTEXT, TZ +
   review log; каждый блок — независимый read-only reviewer GREEN (командные проверки Lead).
8. Регрессия оценивается **по каждому source-виду** (допуск 0.01 IoU относительно базы) при
   одинаковых входах/камерах/настройках; mean — только сводка. Обязательное author-условие с
   `fail|unavailable` не проходит; необязательное может остаться `unavailable` с пометкой.
   Исполнение supplement не доказывает размер — `pass` только по проверенной геометрии/гарантии.
9. Инвалидация: каждое новое bundle пересчитывает актуальность resolutions; изменение зависимой
   геометрии/binding/условий → unknown `open` с `reason: evidence_stale`, старое evidence остаётся
   только в истории.
10. IT-8 ничего не удаляет автоматически; принятые bundle и их транзитивные зависимости сохраняются
    (GC — отдельная будущая задача).

## 10. План работ

| Этап | Writer scope | Основные проверки |
|---|---|---|
| Stage 0 | Lead: `docs/specs/VERIFICATION_STATE.md`, TZ, reproduction | контракт, матрица failure-тестов, reproduction всех сценариев |
| IT-8A | `crates/michelangelo_edit/**`, `cli/src/edit_cmd.rs`, `cli/tests/edit_cli.rs` | ladder + failure-тесты A + crash/lock/race |
| IT-8B | `crates/michelangelo_evidence/**`, `crates/michelangelo_eval/**`, edit gate, CLI `evaluate` | ladder + failure-тесты B + реальный Blender evaluation |
| IT-8C | edit unknowns, assistant, CLI `unknown`, `tools/solver`, `michelangelo_input` | ladder + failure-тесты C + solver-policy + e2e |
| Demo | `tools/demo/**`, evidence | e2e с внешним субагентом + контроль куба |

Блоки последовательные (общий crate `michelangelo_edit`), внутри — один writer; Lead
самостоятельно воспроизводит PASS (ladder, failure-тесты, adversarial), затем независимый
reviewer; RED — фикс и повтор. Commit на `dev` после GREEN, явные pathspecs, без push.

## 11. Evidence

Stage 0 reproduction: `target/techlead/IT-8/audit-reproduction.json` (9/9, exit 0; A2 race — 3
принятия из одной базы на старом коде).

### IT-8A (acceptance integrity)

- Code: `michelangelo_edit` (`verification.rs` snapshot/lock/state, `apply.rs` publish/history,
  `scene.rs` bundle, typed `error.rs`), CLI exit-code mapping, 27 new crate failure tests
  (`tests/verification.rs`) + CLI tests (two-process lock, tamper, legacy, idempotence).
- Independent Lead reproduction: `target/techlead/IT-8/verify_a_fixed.py` → `verify-A.json`,
  **14/14 checks passed**: full-set hash-verified snapshot; source-candidate tamper ignored;
  store tamper and snapshot tamper refused `EVIDENCE_MISMATCH` with `accept-refused` history and
  no revision; idempotent accept (exit 0, same revision, accepted history intact, index accepted);
  reject-after-accept refused without corrupting the mirror; accept after rollback returns the
  historical result and does not switch the scene back; bundle/unknown-state refs hash-verified
  per revision with rollback state selection and unchanged historical report hash; legacy
  `SNAPSHOT_MISSING`; OS lock busy → exit 2 and recovery after holder death; 12-process
  concurrency → 1 accepted / 1 revision; crash-before-pointer and injected-race Rust tests pass.
- Pre-fix audit reproduction re-run on the fixed code: A4 no longer reproduced (second accept
  accepted, decision unchanged, index accepted); A1/A3 store tamper now refuses (no revision).
  The pre-fix evidence is preserved as `audit-reproduction-before-A.json`.
- Reviewer verdict: **GREEN** (child `f21acc57`, route `xiaomi/mimo-v2.6-pro`); three minor
  findings were fixed before the commit: (1) refused acceptances now write their history under
  the store lock (lock taken before evidence verification), (2) the dead `CANDIDATE_ALREADY_ACCEPTED`
  exit-code arm was removed (that code travels as a decision diagnostic, not an `EditError`),
  (3) the `kinds=['?']` line was a script display issue (kinds are pinned by the unit test).
  `bindings_sha256` is intentionally null until IT-8C. The reviewer could not re-execute commands
  (no shell in the preset) and verified code/JSON only; the Lead ladder and `verify-A.json`
  (14/14 after the fixes) are the command evidence.

### IT-8B (candidate evaluation and the accept gate)

- Code: `michelangelo_evidence` (report DTO/digests/registry/gate checks), `michelangelo_eval`
  (renderer/comparator/evaluation assembly, real and fake implementations), `michelangelo_edit`
  gate (`gate.rs`, typed gate errors, mandatory `--evaluation`), CLI `evaluate`; docs
  `docs/specs/EVALUATION.md`; 51 new tests (evidence 23 incl. two Lead hardening tests, eval 12,
  edit gate 10, CLI 6); A tests updated to the strict gate through registered synthetic evidence,
  not weakened.
- Lead real-Blender reproduction (`target/techlead/IT-8B/real_eval.py`, copy of a real IT-5
  harbor run, Blender 5.2.3 LTS): `edit init` → `edit submit` (proposed) → `evaluate --candidate
  --evaluation <held-out pkg>` exit 0, source mean IoU **0.9584**, held-out `control_left` IoU
  **0.9450**, `projection_conformance=pass`, `author_geometric_conditions=pass`,
  `independent_ground_truth=unavailable`, `author_hidden_decision=pending`; the report was
  registered (`ev-0001`) and `edit accept --evaluation <report>` published `s0002` with a
  hash-pinned evaluation reference in the bundle. Negative: tampering the registered report →
  accept exit 1 (`EVALUATION_STALE`, digest mismatch), no revision.
- Owner-level decisions applied: geometry acceptance requires a registered evaluation; hidden
  claims stay `unverified`/`pending` and do not block geometry acceptance; missing GT is
  `unavailable` (never a pass); required author geometric conditions are the package
  `measurements` and an unmeasurable required condition blocks acceptance; per-view regression
  limit 0.01 IoU (mean is summary only); no automatic deletion.
- Deviation recorded: `auto_accept` is deferred — `submit` always keeps the candidate `proposed`
  (`EVALUATION_REQUIRED` diagnostic) because the evaluation can only run after the snapshot
  exists; documented in `LOCAL_EDIT.md`. Schema extensions over the TZ sketch are documented in
  `EVALUATION.md`: `target.base_scene_revision`, `visible.held_out`, `checks[]`.
- Held-out direction rule corrected during Lead verification: a held-out view looking in the
  **same** direction (`dot > 0.9`) adds no constraint; opposite orthographic views (dot ≈ -1) are
  independent and are the held-out controls of the proof datasets. Code, contract, TZ and spec
  updated; a regression test (`opposite_orthographic_held_out_cameras_are_allowed`) added.
- Reviewer verdict: **GREEN** (child `3788f634`, route `xiaomi/mimo-v2.6-pro`). Minor findings
  fixed before the commit: (1) the gate now sorts the current view list before pin comparison
  (a valid acceptance is no longer refused because of package view order), (2) the per-view
  regression limit is pinned to the owner-fixed 0.01 IoU in the gate — a report may tighten it but
  never widen it; tests added for both. NITs left as documented v0 limits: the synthetic-evidence
  helper and fake renderer/comparator are public with `testing` precedent (no CLI path reaches
  them), and the binary-only `unreachable!()` is guarded by an exhaustive match.

### Review log

| блок | attempt | reviewer | route | verdict | findings |
|---|---|---|---|---|---|
| Stage 0 | – | – | – | pending | – |
| A | 1 | child `f21acc57` (read-only) | `xiaomi/mimo-v2.6-pro`, preset `reviewer` | GREEN | MINOR: refusal history written outside the lock (fixed: lock before evidence); dead exit-code arm (fixed); `kinds ?` (script display only). Lead reproduced 14/14 after fixes. |
| B | 1 | child `3788f634` (read-only) | `xiaomi/mimo-v2.6-pro`, preset `reviewer` | GREEN | MINOR: view-order-sensitive pin comparison (fixed + test); regression limit taken from the report (fixed: pinned to 0.01 + test). NITs: public testing helpers/fakes, guarded `unreachable!()` in the binary. Lead reproduced real-Blender evaluation/accept and tamper refusal. |
| C | – | – | – | pending | – |
