//! Provisional private DTO for the resolved batch plan.
//!
//! This is NOT the GLM-1 wire contract. Field names and schema string are
//! provisional; the adapter seam for the future shared contracts lives in
//! [`BatchPlan::to_canonical_json`] and the CLI boundary only.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::timeutil;

/// Provisional schema marker. Deliberately not any approved contract name.
pub const PLAN_SCHEMA: &str = "michelangelo.batch-plan/provisional-0";

/// Upper bound for a fake artifact payload (keeps tests disposable).
pub const MAX_ARTIFACT_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatchPlan {
    pub schema: String,
    /// Optional caller-chosen stable plan id. If absent, derived from the
    /// plan fingerprint. Once registered, the id is immutable for a payload.
    #[serde(default)]
    pub plan_id: Option<String>,
    pub items: Vec<PlanItem>,
    #[serde(default)]
    pub limits: PlanLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanLimits {
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_backoff_base_ms")]
    pub backoff_base_ms: u64,
    #[serde(default = "default_backoff_cap_ms")]
    pub backoff_cap_ms: u64,
    #[serde(default = "default_attempt_timeout_ms")]
    pub attempt_timeout_ms: u64,
    #[serde(default = "default_heavy_workers")]
    pub heavy_workers: u32,
}

impl Default for PlanLimits {
    fn default() -> Self {
        PlanLimits {
            max_attempts: default_max_attempts(),
            backoff_base_ms: default_backoff_base_ms(),
            backoff_cap_ms: default_backoff_cap_ms(),
            attempt_timeout_ms: default_attempt_timeout_ms(),
            heavy_workers: default_heavy_workers(),
        }
    }
}

fn default_max_attempts() -> u32 {
    3
}
fn default_backoff_base_ms() -> u64 {
    30
}
fn default_backoff_cap_ms() -> u64 {
    200
}
fn default_attempt_timeout_ms() -> u64 {
    10_000
}
fn default_heavy_workers() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanItem {
    pub item_id: String,
    /// Real input files whose content is fingerprinted at registration and
    /// re-verified on resume. Paths are stored canonicalized (absolute).
    #[serde(default)]
    pub inputs: Vec<PlanInput>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub worker: WorkerSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanInput {
    pub path: PathBuf,
    /// SHA-256 hex of the actual file bytes; filled by registration if absent.
    #[serde(default)]
    pub sha256: Option<String>,
}

/// Fake worker behaviour spec (provisional). The future real worker adapter
/// replaces this entirely; the failure taxonomy is the stable part.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkerSpec {
    pub failure_mode: FailureMode,
    /// For [`FailureMode::TransientUntilAttempt`]: attempts numbered below
    /// this fail transiently; this attempt number and later succeed.
    #[serde(default)]
    pub transient_until_attempt: u32,
    #[serde(default)]
    pub delay_ms: u64,
    #[serde(default = "default_artifact_name")]
    pub artifact_name: String,
    #[serde(default = "default_artifact_bytes")]
    pub artifact_bytes: u64,
    /// When true, a successful build passes the fake validation and the item
    /// ends in `awaiting_review`; otherwise it ends in `built`.
    #[serde(default)]
    pub simulate_validation: bool,
    #[serde(default)]
    pub reason: String,
}

fn default_artifact_name() -> String {
    "artifact.bin".to_string()
}
fn default_artifact_bytes() -> u64 {
    64
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailureMode {
    /// Normal successful build.
    None,
    /// Fails transiently until a given attempt number (flaky infra model).
    TransientUntilAttempt,
    /// Permanent failure (invalid input). Never retried.
    Permanent,
    /// Fake validation failure. Never retried (quality verdict).
    ValidationFail,
    /// Writes the artifact, then exits abruptly without a result (crash
    /// injection around finalize).
    CrashBeforeFinalize,
    /// Never finishes; the attempt timeout must kill it.
    Hang,
    /// Simulates an IO/disk error while writing the artifact.
    FailIo,
}

#[derive(Debug)]
pub enum PlanError {
    Io(std::io::Error),
    Json(serde_json::Error),
    SchemaMismatch {
        found: String,
    },
    NoItems,
    /// The item id is not unique. On Windows this also covers ids that are
    /// distinct as strings but collide on the filesystem ("Build" and
    /// "build" address one and the same directory).
    DuplicateItem {
        item_id: String,
    },
    BadItemId {
        item_id: String,
    },
    UnknownDependency {
        item_id: String,
        dep: String,
    },
    SelfDependency {
        item_id: String,
    },
    Cycle {
        remains: Vec<String>,
    },
    InputMissing {
        path: PathBuf,
    },
    InputRead {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The plan declared a sha256 for an input, but the actual file bytes
    /// hash to something else. A declared hash is a contract: registration
    /// never silently overwrites it with the observed value.
    InputHashMismatch {
        path: PathBuf,
        declared: String,
        actual: String,
    },
    BadWorkerSpec {
        item_id: String,
        reason: String,
    },
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::Io(e) => write!(f, "plan io error: {e}"),
            PlanError::Json(e) => write!(f, "plan json error: {e}"),
            PlanError::SchemaMismatch { found } => {
                write!(f, "plan schema mismatch: expected {PLAN_SCHEMA:?}, found {found:?}")
            }
            PlanError::NoItems => write!(f, "plan has no items"),
            PlanError::DuplicateItem { item_id } => write!(f, "duplicate item id {item_id:?}"),
            PlanError::BadItemId { item_id } => write!(
                f,
                "item id {item_id:?} is invalid: allowed [a-zA-Z0-9._-], \
                 first character must be [a-zA-Z0-9], 1..=64 chars, \
                 and Windows device names ({} etc.) are reserved",
                WINDOWS_RESERVED_NAMES
                    .iter()
                    .take(4)
                    .copied()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            PlanError::UnknownDependency { item_id, dep } => {
                write!(f, "item {item_id:?} depends on unknown item {dep:?}")
            }
            PlanError::SelfDependency { item_id } => {
                write!(f, "item {item_id:?} depends on itself")
            }
            PlanError::Cycle { remains } => {
                write!(f, "dependency cycle detected among items: {}", remains.join(", "))
            }
            PlanError::InputMissing { path } => write!(f, "input file missing: {}", path.display()),
            PlanError::InputRead { path, source } => {
                write!(f, "input read failed for {}: {source}", path.display())
            }
            PlanError::InputHashMismatch {
                path,
                declared,
                actual,
            } => write!(
                f,
                "input hash mismatch for {}: plan declares {declared}, actual bytes hash to {actual}",
                path.display()
            ),
            PlanError::BadWorkerSpec { item_id, reason } => {
                write!(f, "invalid worker spec for item {item_id:?}: {reason}")
            }
        }
    }
}

impl std::error::Error for PlanError {}

impl From<std::io::Error> for PlanError {
    fn from(e: std::io::Error) -> Self {
        PlanError::Io(e)
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex(&hasher.finalize())
}

pub fn hex(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 2);
    for b in data {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

impl BatchPlan {
    pub fn load(path: &Path) -> Result<BatchPlan, PlanError> {
        let raw = std::fs::read(path)?;
        let mut plan: BatchPlan = serde_json::from_slice(&raw).map_err(PlanError::Json)?;
        plan.normalize()?;
        Ok(plan)
    }

    /// Apply defaults that must exist before validation/serialization.
    pub fn normalize(&mut self) -> Result<(), PlanError> {
        for item in &mut self.items {
            item.worker.artifact_name = if item.worker.artifact_name.is_empty() {
                default_artifact_name()
            } else {
                item.worker.artifact_name.clone()
            };
        }
        Ok(())
    }

    /// Canonical JSON used for fingerprints: serde struct order is the
    /// declaration order, so identical payloads produce identical bytes.
    pub fn to_canonical_json(&self) -> Result<String, PlanError> {
        serde_json::to_string(self).map_err(PlanError::Json)
    }

    pub fn fingerprint(&self) -> Result<String, PlanError> {
        Ok(sha256_hex(self.to_canonical_json()?.as_bytes()))
    }

    /// Stable plan id: caller-provided or derived from the fingerprint.
    pub fn resolved_plan_id(&self) -> Result<String, PlanError> {
        match &self.plan_id {
            Some(id) => Ok(id.clone()),
            None => {
                let fp = self.fingerprint()?;
                Ok(format!("bp-{}", &fp[..16]))
            }
        }
    }

    pub fn validate(&self) -> Result<(), PlanError> {
        if self.schema != PLAN_SCHEMA {
            return Err(PlanError::SchemaMismatch {
                found: self.schema.clone(),
            });
        }
        if self.items.is_empty() {
            return Err(PlanError::NoItems);
        }
        let mut seen = std::collections::BTreeSet::new();
        // Uniqueness is checked on the EFFECTIVE id: on Windows the
        // filesystem folds case and trailing dots/spaces, so "Build" and
        // "build" would address one and the same attempt directory.
        let mut seen_effective = std::collections::BTreeSet::new();
        for item in &self.items {
            if !valid_item_id(&item.item_id) {
                return Err(PlanError::BadItemId {
                    item_id: item.item_id.clone(),
                });
            }
            if !seen.insert(item.item_id.clone()) {
                return Err(PlanError::DuplicateItem {
                    item_id: item.item_id.clone(),
                });
            }
            if !seen_effective.insert(id_uniqueness_key(&item.item_id)) {
                return Err(PlanError::DuplicateItem {
                    item_id: item.item_id.clone(),
                });
            }
            if item.worker.transient_until_attempt == 0
                && item.worker.failure_mode == FailureMode::TransientUntilAttempt
            {
                return Err(PlanError::BadWorkerSpec {
                    item_id: item.item_id.clone(),
                    reason: "transient_until_attempt must be >= 1".into(),
                });
            }
            if item.worker.artifact_bytes > MAX_ARTIFACT_BYTES {
                return Err(PlanError::BadWorkerSpec {
                    item_id: item.item_id.clone(),
                    reason: format!(
                        "artifact_bytes {} exceeds PoC cap {MAX_ARTIFACT_BYTES}",
                        item.worker.artifact_bytes
                    ),
                });
            }
        }
        for item in &self.items {
            for dep in &item.depends_on {
                if dep == &item.item_id {
                    return Err(PlanError::SelfDependency {
                        item_id: item.item_id.clone(),
                    });
                }
                if !seen.contains(dep) {
                    return Err(PlanError::UnknownDependency {
                        item_id: item.item_id.clone(),
                        dep: dep.clone(),
                    });
                }
            }
        }
        // Acyclicity via Kahn's algorithm; also produces the execution order.
        self.topo_order()?;
        if self.limits.heavy_workers != 1 {
            // Honest boundary: PoC serializes heavy work only.
            return Err(PlanError::BadWorkerSpec {
                item_id: "*".into(),
                reason: "limits.heavy_workers must be 1 in this PoC".into(),
            });
        }
        if self.limits.max_attempts == 0 {
            return Err(PlanError::BadWorkerSpec {
                item_id: "*".into(),
                reason: "limits.max_attempts must be >= 1".into(),
            });
        }
        Ok(())
    }

    /// Deterministic topological order (ties broken by item id).
    pub fn topo_order(&self) -> Result<Vec<String>, PlanError> {
        let ids: Vec<String> = self.items.iter().map(|i| i.item_id.clone()).collect();
        let mut deps: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
        let mut indegree: std::collections::BTreeMap<&str, usize> =
            ids.iter().map(|s| (s.as_str(), 0usize)).collect();
        let by_id: std::collections::BTreeMap<&str, &PlanItem> =
            self.items.iter().map(|i| (i.item_id.as_str(), i)).collect();
        for item in &self.items {
            for dep in &item.depends_on {
                deps.entry(dep.as_str())
                    .or_default()
                    .push(item.item_id.as_str());
                *indegree.get_mut(item.item_id.as_str()).unwrap() += 1;
            }
        }
        let mut ready: Vec<&str> = indegree
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(k, _)| *k)
            .collect();
        ready.sort_unstable();
        let mut order = Vec::with_capacity(ids.len());
        while let Some(id) = ready.first().copied() {
            ready.remove(0);
            order.push(id.to_string());
            if let Some(children) = deps.get(id) {
                for child in children {
                    let e = indegree.get_mut(child).unwrap();
                    *e -= 1;
                    if *e == 0 {
                        ready.push(child);
                        ready.sort_unstable();
                    }
                }
            }
        }
        if order.len() != ids.len() {
            let done: std::collections::BTreeSet<&str> = order.iter().map(|s| s.as_str()).collect();
            let remains: Vec<String> = by_id
                .keys()
                .filter(|k| !done.contains(**k))
                .map(|s| s.to_string())
                .collect();
            return Err(PlanError::Cycle { remains });
        }
        Ok(order)
    }

    /// Canonicalize input paths and fill missing sha256 fingerprints from the
    /// actual file bytes. Called once at registration; the filled plan is what
    /// gets persisted and later re-verified by resume.
    ///
    /// A plan MAY declare the expected sha256 of an input. A declared hash is
    /// a contract, not a hint: if the actual file bytes hash to something
    /// else the plan is rejected with [`PlanError::InputHashMismatch`] and
    /// the declared value is never overwritten. Only an absent hash is filled
    /// in from the observed bytes.
    pub fn pin_inputs(&mut self) -> Result<(), PlanError> {
        for item in &mut self.items {
            for input in &mut item.inputs {
                let abs = std::fs::canonicalize(&input.path).map_err(|e| match e.kind() {
                    std::io::ErrorKind::NotFound => PlanError::InputMissing {
                        path: input.path.clone(),
                    },
                    _ => PlanError::InputRead {
                        path: input.path.clone(),
                        source: e,
                    },
                })?;
                let bytes = std::fs::read(&abs).map_err(|e| PlanError::InputRead {
                    path: abs.clone(),
                    source: e,
                })?;
                let actual = sha256_hex(&bytes);
                if let Some(declared) = input.sha256.as_deref() {
                    let declared = declared.trim().to_ascii_lowercase();
                    if declared != actual {
                        return Err(PlanError::InputHashMismatch {
                            path: abs,
                            declared,
                            actual,
                        });
                    }
                }
                input.path = abs;
                input.sha256 = Some(actual);
            }
        }
        Ok(())
    }

    /// Recompute fingerprints of the actual input files and compare with the
    /// pinned ones. Returns the list of drifted item ids.
    pub fn verify_input_fingerprints(&self) -> Result<Vec<String>, PlanError> {
        let mut drifted = Vec::new();
        for item in &self.items {
            for input in &item.inputs {
                let bytes = std::fs::read(&input.path).map_err(|e| PlanError::InputRead {
                    path: input.path.clone(),
                    source: e,
                })?;
                let actual = sha256_hex(&bytes);
                if Some(actual) != input.sha256 {
                    drifted.push(item.item_id.clone());
                    break;
                }
            }
        }
        Ok(drifted)
    }

    /// True when the actual bytes of THIS item's inputs no longer match the
    /// pinned fingerprints. Used for the per-item drift check right before a
    /// claim, where the up-front verification may already be stale.
    pub fn item_inputs_drifted(&self, item_id: &str) -> Result<bool, PlanError> {
        let Some(item) = self.items.iter().find(|i| i.item_id == item_id) else {
            return Ok(false);
        };
        for input in &item.inputs {
            let bytes = std::fs::read(&input.path).map_err(|e| PlanError::InputRead {
                path: input.path.clone(),
                source: e,
            })?;
            if Some(sha256_hex(&bytes)) != input.sha256 {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Windows device names that must never be used as a directory name.
///
/// On Windows these names are reserved by the OS: `CON`, `NUL` and friends
/// address a device, not a file, and the reservation survives an extension
/// (`CON.txt` is the console device as well). An item id becomes a directory
/// name under the runs root, so a reserved name is rejected on EVERY
/// platform: a plan accepted on Linux must not break when the same batch is
/// executed on Windows.
pub const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// True when the item id is (or starts with) a reserved Windows device name.
///
/// The check is applied to the stem, i.e. the part before the first dot,
/// because Windows reserves `NUL.txt` exactly like `NUL`. The comparison is
/// case-insensitive: `nul`, `Nul` and `NUL` are the same device.
pub fn is_windows_reserved_name(id: &str) -> bool {
    let stem = id.split('.').next().unwrap_or(id);
    WINDOWS_RESERVED_NAMES
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(stem))
}

/// Item ids are used as directory names under the runs root, so the grammar
/// is deliberately narrow: the first character must be alphanumeric (which
/// excludes "." and ".." and any path separator), the rest may additionally
/// contain ".", "_" and "-".
///
/// Windows device names are rejected as well (see
/// [`WINDOWS_RESERVED_NAMES`]): they are reserved on every platform so that
/// a plan is portable.
pub fn valid_item_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 64 {
        return false;
    }
    // "." and ".." would resolve to the runs root / its parent.
    if id == "." || id == ".." {
        return false;
    }
    // Reserved Windows device names would not be usable as a directory name.
    if is_windows_reserved_name(id) {
        return false;
    }
    let mut chars = id.chars();
    let first = chars.next().unwrap_or_default();
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

/// The Windows-normalized form of an item id.
///
/// NTFS compares names case-insensitively and ignores trailing dots and
/// spaces, so "Build", "build" and "build." all denote one and the same
/// directory. This helper produces the key under which such ids collide.
///
/// The ids themselves are never rewritten: only comparisons (the uniqueness
/// check, the attempt-directory ownership check) use this form.
pub fn windows_fold_id(id: &str) -> String {
    let trimmed = id.trim_end_matches(['.', ' ']);
    trimmed.to_lowercase()
}

/// The Windows-normalized comparison key of a path.
///
/// Every component is case-folded and stripped of trailing dots and spaces,
/// mirroring the NTFS name comparison rules. Two paths with the same key
/// address the same directory on Windows.
pub fn windows_fold_path(path: &Path) -> String {
    let raw = path.to_string_lossy().to_lowercase();
    raw.split(['/', '\\'])
        .map(|component| component.trim_end_matches(['.', ' ']))
        .collect::<Vec<_>>()
        .join("/")
}

/// The key under which two attempt directories are the same directory.
///
/// On Windows the filesystem folds case and trailing dots/spaces, so two
/// different spellings can be one physical directory: the key folds them
/// together. On case-sensitive filesystems the exact path is the key and
/// distinct directories stay distinct.
pub fn dir_key(path: &Path) -> String {
    if cfg!(windows) {
        windows_fold_path(path)
    } else {
        path.to_string_lossy().to_string()
    }
}

/// The key under which two item ids are the same id.
///
/// Same rule as [`dir_key`]: on Windows "Build" and "build" are one id
/// (one directory), elsewhere they are two different ids.
fn id_uniqueness_key(id: &str) -> String {
    if cfg!(windows) {
        windows_fold_id(id)
    } else {
        id.to_owned()
    }
}

/// Attempt directory layout under the runs root: `<runs>/<item_id>/attempt_<no:03>`.
///
/// `item_id` MUST satisfy [`valid_item_id`]: the first character is
/// alphanumeric and no separator or "." / ".." is allowed, so the joined path
/// always stays inside the runs root.
pub fn attempt_dir(runs_root: &Path, item_id: &str, attempt_no: u32) -> PathBuf {
    runs_root
        .join(item_id)
        .join(format!("attempt_{attempt_no:03}"))
}

/// Deterministic fake artifact bytes derived from the attempt identity.
pub fn fake_artifact_bytes(item_id: &str, attempt_no: u32, len: u64) -> Vec<u8> {
    let seed = sha256_hex(format!("mb-fake-artifact:{item_id}:{attempt_no}").as_bytes());
    let mut out = Vec::with_capacity(len as usize);
    let seed_bytes = seed.as_bytes();
    for i in 0..len as usize {
        out.push(seed_bytes[i % seed_bytes.len()]);
    }
    out
}

#[allow(dead_code)]
pub fn now() -> String {
    timeutil::now_iso8601()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_with(items: Vec<PlanItem>) -> BatchPlan {
        BatchPlan {
            schema: PLAN_SCHEMA.to_string(),
            plan_id: Some("bp-test".into()),
            items,
            limits: PlanLimits::default(),
        }
    }

    fn item(id: &str, deps: &[&str]) -> PlanItem {
        PlanItem {
            item_id: id.to_string(),
            inputs: vec![],
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            worker: WorkerSpec {
                failure_mode: FailureMode::None,
                transient_until_attempt: 0,
                delay_ms: 0,
                artifact_name: "artifact.bin".into(),
                artifact_bytes: 16,
                simulate_validation: true,
                reason: String::new(),
            },
        }
    }

    #[test]
    fn topo_order_respects_dependencies_and_is_stable() {
        let plan = plan_with(vec![
            item("c", &["a", "b"]),
            item("b", &["a"]),
            item("a", &[]),
        ]);
        let order = plan.topo_order().unwrap();
        assert_eq!(
            order,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn topo_order_detects_cycles() {
        let plan = plan_with(vec![item("a", &["b"]), item("b", &["a"])]);
        match plan.topo_order() {
            Err(PlanError::Cycle { remains }) => {
                assert_eq!(remains.len(), 2);
            }
            other => panic!("expected cycle, got {other:?}"),
        }
    }

    #[test]
    fn validation_rejects_unknown_dep_self_dep_and_bad_ids() {
        let plan = plan_with(vec![item("a", &["ghost"])]);
        assert!(matches!(
            plan.validate(),
            Err(PlanError::UnknownDependency { .. })
        ));

        let plan = plan_with(vec![item("a", &["a"])]);
        assert!(matches!(
            plan.validate(),
            Err(PlanError::SelfDependency { .. })
        ));

        let plan = plan_with(vec![item("bad id!", &[])]);
        assert!(matches!(plan.validate(), Err(PlanError::BadItemId { .. })));

        let plan = plan_with(vec![item("a", &[]), item("a", &[])]);
        assert!(matches!(
            plan.validate(),
            Err(PlanError::DuplicateItem { .. })
        ));

        let mut plan = plan_with(vec![item("a", &[])]);
        plan.schema = "wrong".into();
        assert!(matches!(
            plan.validate(),
            Err(PlanError::SchemaMismatch { .. })
        ));

        let mut plan = plan_with(vec![item("a", &[])]);
        plan.limits.heavy_workers = 4;
        assert!(matches!(
            plan.validate(),
            Err(PlanError::BadWorkerSpec { .. })
        ));
    }

    #[test]
    fn windows_fold_id_folds_case_and_trailing_dots_and_spaces() {
        assert_eq!(windows_fold_id("Build"), "build");
        assert_eq!(windows_fold_id("BUILD"), "build");
        assert_eq!(windows_fold_id("build."), "build");
        assert_eq!(windows_fold_id("build..."), "build");
        assert_eq!(windows_fold_id("build "), "build");
        assert_eq!(windows_fold_id("build. "), "build");
        // The id itself is never rewritten: only the comparison key is folded.
        assert_eq!(windows_fold_id("item-1"), "item-1");
        assert_ne!(windows_fold_id("item-1"), windows_fold_id("item_1"));
    }

    #[test]
    fn windows_fold_path_folds_every_component() {
        let a = Path::new("/runs/Build/attempt_001");
        let b = Path::new("/runs/build/attempt_001");
        assert_eq!(windows_fold_path(a), windows_fold_path(b));
        let c = Path::new("/runs/build/attempt_002");
        assert_ne!(windows_fold_path(a), windows_fold_path(c));
    }

    #[test]
    fn item_id_grammar_rejects_windows_device_names() {
        for reserved in [
            "CON", "con", "Con", "PRN", "prn", "AUX", "aux", "NUL", "nul", "COM1", "com9", "LPT1",
            "lpt9",
        ] {
            assert!(
                !valid_item_id(reserved),
                "reserved Windows device name {reserved:?} must be rejected"
            );
        }
        // The reservation survives an extension, exactly like on Windows.
        assert!(!valid_item_id("NUL.txt"));
        assert!(!valid_item_id("com1.out"));
        // Names that merely start with a device name are fine.
        for good in ["CONSOLE", "COM0", "LPT0", "NULLIFY", "auxiliary", "n"] {
            assert!(
                valid_item_id(good),
                "item id {good:?} must not be treated as a device name"
            );
        }
    }

    /// On Windows the filesystem folds case, so "a" and "A" would address one
    /// and the same attempt directory: the plan must be rejected instead of
    /// letting two items fight over one directory.
    #[cfg(windows)]
    #[test]
    fn validation_rejects_ids_that_collide_on_windows() {
        let plan = plan_with(vec![item("a", &[]), item("A", &[])]);
        assert!(
            matches!(plan.validate(), Err(PlanError::DuplicateItem { .. })),
            "case-colliding item ids must be rejected on Windows"
        );

        let plan = plan_with(vec![item("build", &[]), item("Build.", &[])]);
        assert!(
            matches!(plan.validate(), Err(PlanError::DuplicateItem { .. })),
            "trailing-dot-colliding item ids must be rejected on Windows"
        );
    }

    /// The same plan is legal on a case-sensitive filesystem: the ids are
    /// distinct there and address distinct directories.
    #[cfg(not(windows))]
    #[test]
    fn validation_allows_case_distinct_ids_on_case_sensitive_filesystems() {
        let plan = plan_with(vec![item("a", &[]), item("A", &[])]);
        assert!(
            plan.validate().is_ok(),
            "case-distinct ids are distinct directories here"
        );
    }

    #[test]
    fn fingerprint_is_stable_and_payload_sensitive() {
        let mut plan = plan_with(vec![item("a", &[])]);
        let f1 = plan.fingerprint().unwrap();
        let f2 = plan.fingerprint().unwrap();
        assert_eq!(f1, f2);
        plan.items[0].worker.delay_ms = 5;
        assert_ne!(f1, plan.fingerprint().unwrap());
    }

    #[test]
    fn derived_plan_id_uses_fingerprint_prefix() {
        let mut plan = plan_with(vec![item("a", &[])]);
        plan.plan_id = None;
        let id = plan.resolved_plan_id().unwrap();
        assert!(id.starts_with("bp-"));
        assert_eq!(id.len(), 3 + 16);
    }

    #[test]
    fn pin_and_verify_inputs_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.txt");
        std::fs::write(&a, b"hello").unwrap();
        let mut plan = plan_with(vec![PlanItem {
            item_id: "a".into(),
            inputs: vec![PlanInput {
                path: a.clone(),
                sha256: None,
            }],
            depends_on: vec![],
            worker: item("a", &[]).worker,
        }]);
        plan.pin_inputs().unwrap();
        assert!(plan.items[0].inputs[0].path.is_absolute());
        let good = plan.items[0].inputs[0].sha256.clone().unwrap();
        assert_eq!(good.len(), 64);
        assert!(plan.verify_input_fingerprints().unwrap().is_empty());

        std::fs::write(&a, b"changed").unwrap();
        let drifted = plan.verify_input_fingerprints().unwrap();
        assert_eq!(drifted, vec!["a".to_string()]);
    }

    #[test]
    fn sha256_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn item_id_grammar_rejects_path_traversal() {
        for hostile in [
            ".", "..", "...", ".hidden", "-lead", "_lead", "a/b", "a\\b", "", "a b", "a:b",
        ] {
            assert!(
                !valid_item_id(hostile),
                "item id {hostile:?} must be rejected"
            );
        }
        for good in [
            "a", "A", "0", "item-1", "item_1", "item.1", "a.b-c_d", "0abc",
        ] {
            assert!(valid_item_id(good), "item id {good:?} must be accepted");
        }
        // Too long.
        assert!(!valid_item_id(&"a".repeat(65)));
        assert!(valid_item_id(&"a".repeat(64)));
    }

    #[test]
    fn attempt_dir_cannot_escape_the_runs_root() {
        let root = Path::new("/runs");
        for hostile in [".", "..", "a/b", "../escape", ".hidden"] {
            assert!(
                !valid_item_id(hostile),
                "hostile id {hostile:?} must never reach attempt_dir"
            );
        }
        for good in ["a", "item-1", "item_1", "item.1"] {
            let dir = attempt_dir(root, good, 7);
            assert!(dir.starts_with(root), "{dir:?} escaped {root:?}");
            assert_eq!(dir, root.join(good).join("attempt_007"));
        }
    }

    #[test]
    fn pin_inputs_rejects_declared_hash_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.txt");
        std::fs::write(&a, b"hello").unwrap();

        let mut plan = plan_with(vec![PlanItem {
            item_id: "a".into(),
            inputs: vec![PlanInput {
                path: a.clone(),
                sha256: Some(sha256_hex(b"different bytes")),
            }],
            depends_on: vec![],
            worker: item("a", &[]).worker,
        }]);

        match plan.pin_inputs() {
            Err(PlanError::InputHashMismatch {
                path,
                declared,
                actual,
            }) => {
                assert_eq!(path, std::fs::canonicalize(&a).unwrap());
                assert_eq!(declared, sha256_hex(b"different bytes"));
                assert_eq!(actual, sha256_hex(b"hello"));
            }
            other => panic!("expected InputHashMismatch, got {other:?}"),
        }
    }

    #[test]
    fn pin_inputs_keeps_matching_declared_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.txt");
        std::fs::write(&a, b"hello").unwrap();

        let mut plan = plan_with(vec![PlanItem {
            item_id: "a".into(),
            inputs: vec![PlanInput {
                path: a.clone(),
                sha256: Some(sha256_hex(b"hello")),
            }],
            depends_on: vec![],
            worker: item("a", &[]).worker,
        }]);
        plan.pin_inputs().unwrap();
        assert_eq!(
            plan.items[0].inputs[0].sha256.as_deref(),
            Some(sha256_hex(b"hello").as_str())
        );
        assert!(!plan.item_inputs_drifted("a").unwrap());

        // Tampered bytes are detected per item as well.
        std::fs::write(&a, b"tampered").unwrap();
        assert!(plan.item_inputs_drifted("a").unwrap());
        assert_eq!(
            plan.verify_input_fingerprints().unwrap(),
            vec!["a".to_string()]
        );
    }
}
