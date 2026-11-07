//! The typed edit candidate (`michelangelo.edit-candidate/0`).
//!
//! A candidate is an explicit, versioned description of a change of a scene
//! revision: the base it was built on, the bounded task it may carry out, the
//! replacement geometry, the provenance of the change and the budget it
//! declares.
//!
//! The document is strict: a field that is not part of the schema, a value of
//! the wrong type or an unusable name makes the document *not a candidate*.
//! Such a document is reported with the diagnostic code `INTERNAL_ERROR`
//! and the decision `error`; it never reaches the scene store.

use serde::{Deserialize, Serialize};

use crate::CANDIDATE_SCHEMA;

/// The maximum number of characters of a component name.
pub const COMPONENT_NAME_MAX_LEN: usize = 64;

/// The origins a candidate may declare.
pub const CANDIDATE_ORIGINS: [&str; 3] = ["human", "ai_author", "test_backend"];

/// A failure of the candidate parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateError {
    /// The diagnostic code of the failure.
    pub code: &'static str,
    /// The human readable reason.
    pub message: String,
}

impl CandidateError {
    /// A failure of the schema of the document.
    pub fn schema(message: impl Into<String>) -> Self {
        CandidateError {
            code: "INTERNAL_ERROR",
            message: message.into(),
        }
    }

    /// A failure of the declared budget.
    pub fn budget(message: impl Into<String>) -> Self {
        CandidateError {
            code: "BUDGET_INVALID",
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CandidateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for CandidateError {}

/// The origin of a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateOrigin {
    /// A human author wrote the candidate.
    Human,
    /// An assistant wrote the candidate.
    AiAuthor,
    /// A local test backend wrote the candidate.
    TestBackend,
}

impl CandidateOrigin {
    /// The origin as the text used in the document.
    pub fn as_str(&self) -> &'static str {
        match self {
            CandidateOrigin::Human => "human",
            CandidateOrigin::AiAuthor => "ai_author",
            CandidateOrigin::TestBackend => "test_backend",
        }
    }

    /// The origin of a text (None when the text is not an origin).
    pub fn from_text(text: &str) -> Option<Self> {
        match text {
            "human" => Some(CandidateOrigin::Human),
            "ai_author" => Some(CandidateOrigin::AiAuthor),
            "test_backend" => Some(CandidateOrigin::TestBackend),
            _ => None,
        }
    }
}

/// The action of a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateAction {
    /// Replace an existing component by new geometry.
    Replace,
    /// Add a new component.
    Add,
    /// Remove a component (not supported in v0).
    Delete,
}

impl CandidateAction {
    /// The action as the text used in the document.
    pub fn as_str(&self) -> &'static str {
        match self {
            CandidateAction::Replace => "replace",
            CandidateAction::Add => "add",
            CandidateAction::Delete => "delete",
        }
    }

    /// The action of a text (None when the text is not an action).
    pub fn from_text(text: &str) -> Option<Self> {
        match text {
            "replace" => Some(CandidateAction::Replace),
            "add" => Some(CandidateAction::Add),
            "delete" => Some(CandidateAction::Delete),
            _ => None,
        }
    }
}

/// The base a candidate was built on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateBase {
    /// The identifier of the input package.
    pub package_id: String,
    /// The revision of the input package.
    pub package_revision: u32,
    /// The scene revision the candidate changes.
    pub scene_revision: String,
    /// The run the scene revision belongs to.
    pub run_id: String,
}

/// The spatial region a change may touch.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Region {
    /// The minimum corner of the region.
    pub min: [f64; 3],
    /// The maximum corner of the region.
    pub max: [f64; 3],
}

impl Region {
    /// True when the region is usable (the minimum is below the maximum on
    /// every axis).
    pub fn is_valid(&self) -> bool {
        (0..3).all(|axis| self.min[axis] < self.max[axis])
    }

    /// True when the region contains the given box.
    pub fn contains(&self, bbox: &crate::obj::Bbox) -> bool {
        for axis in 0..3 {
            if bbox.min[axis] < self.min[axis] || bbox.max[axis] > self.max[axis] {
                return false;
            }
        }
        true
    }
}

/// The bounded task of a candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateTask {
    /// The identifier of the task.
    pub task_id: String,
    /// The components the task may change.
    pub allowed_components: Vec<String>,
    /// The spatial region the task may touch.
    pub allowed_region: Region,
    /// The components the task must not touch.
    pub protected: Vec<String>,
    /// The intersections the task is allowed to create.
    pub allowed_intersections: Vec<String>,
    /// Whether an explicit author acceptance is required.
    pub require_author_acceptance: bool,
    /// The maximum number of vertices the task may change.
    pub max_changed_vertices: u64,
}

/// One change of a candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateChange {
    /// The component the change applies to.
    pub component: String,
    /// The action of the change.
    pub action: CandidateAction,
    /// The OBJ of the change, relative to the candidate directory.
    pub obj: String,
}

/// The provenance of a candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// The origin of the candidate.
    pub origin: CandidateOrigin,
    /// A human readable detail.
    pub detail: String,
}

/// The budget a candidate declares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Budget {
    /// The maximum number of tool calls.
    pub max_calls: u64,
    /// The maximum number of seconds.
    pub max_seconds: u64,
    /// The maximum cost in US dollars.
    pub max_cost_usd: f64,
}

/// An edit candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditCandidate {
    /// The schema identifier.
    pub schema: String,
    /// The identifier of the candidate.
    pub candidate_id: String,
    /// The base the candidate was built on.
    pub base: CandidateBase,
    /// The bounded task of the candidate.
    pub task: CandidateTask,
    /// The changes of the candidate.
    pub changes: Vec<CandidateChange>,
    /// The provenance of the candidate.
    pub provenance: Provenance,
    /// The declared budget.
    pub budget: Budget,
}

impl EditCandidate {
    /// The component names the candidate changes.
    pub fn changed_components(&self) -> Vec<String> {
        self.changes
            .iter()
            .map(|change| change.component.clone())
            .collect()
    }
}

/// The raw document of a candidate (the strict shape of the schema).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCandidate {
    schema: String,
    candidate_id: String,
    base: RawBase,
    task: RawTask,
    changes: Vec<RawChange>,
    provenance: RawProvenance,
    budget: RawBudget,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBase {
    package_id: String,
    package_revision: u32,
    scene_revision: String,
    run_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTask {
    task_id: String,
    allowed_components: Vec<String>,
    allowed_region: RawRegion,
    protected: Vec<String>,
    #[serde(default)]
    allowed_intersections: Vec<String>,
    require_author_acceptance: bool,
    max_changed_vertices: u64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRegion {
    min: [f64; 3],
    max: [f64; 3],
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawChange {
    component: String,
    action: String,
    obj: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProvenance {
    origin: String,
    #[serde(default)]
    detail: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBudget {
    max_calls: f64,
    max_seconds: f64,
    max_cost_usd: f64,
}

/// Parse a candidate document.
pub fn parse_candidate(text: &str) -> Result<EditCandidate, CandidateError> {
    let raw: RawCandidate = serde_json::from_str(text).map_err(|err| {
        CandidateError::schema(format!(
            "the candidate does not follow the schema '{CANDIDATE_SCHEMA}': {err}"
        ))
    })?;

    if raw.schema != CANDIDATE_SCHEMA {
        return Err(CandidateError::schema(format!(
            "the schema '{}' is not supported",
            raw.schema
        )));
    }
    if !is_valid_component_name(&raw.candidate_id) {
        return Err(CandidateError::schema(format!(
            "the candidate id '{}' is not usable",
            raw.candidate_id
        )));
    }
    if raw.base.package_id.trim().is_empty() {
        return Err(CandidateError::schema(
            "the base package id must not be empty".to_string(),
        ));
    }
    if raw.base.run_id.trim().is_empty() {
        return Err(CandidateError::schema(
            "the base run id must not be empty".to_string(),
        ));
    }
    if !crate::scene::is_valid_revision_name(&raw.base.scene_revision) {
        return Err(CandidateError::schema(format!(
            "the base scene revision '{}' is not usable",
            raw.base.scene_revision
        )));
    }
    if raw.task.task_id.trim().is_empty() {
        return Err(CandidateError::schema(
            "the task id must not be empty".to_string(),
        ));
    }
    for name in raw
        .task
        .allowed_components
        .iter()
        .chain(raw.task.protected.iter())
    {
        if !is_valid_component_name(name) {
            return Err(CandidateError::schema(format!(
                "the component name '{name}' is not usable"
            )));
        }
    }

    let region = Region {
        min: raw.task.allowed_region.min,
        max: raw.task.allowed_region.max,
    };
    if !region.is_valid() {
        return Err(CandidateError::schema(
            "the allowed region is empty or inverted".to_string(),
        ));
    }

    if raw.changes.is_empty() {
        return Err(CandidateError::schema(
            "the candidate carries no change".to_string(),
        ));
    }
    let mut seen: Vec<String> = Vec::new();
    let mut changes = Vec::with_capacity(raw.changes.len());
    for change in &raw.changes {
        if !is_valid_component_name(&change.component) {
            return Err(CandidateError::schema(format!(
                "the component name '{}' is not usable",
                change.component
            )));
        }
        if seen.contains(&change.component) {
            return Err(CandidateError::schema(format!(
                "the component '{}' is changed twice",
                change.component
            )));
        }
        seen.push(change.component.clone());
        let action = CandidateAction::from_text(&change.action).ok_or_else(|| {
            CandidateError::schema(format!(
                "the action '{}' is not one of replace, add, delete",
                change.action
            ))
        })?;
        if !is_safe_relative_obj_path(&change.obj) {
            return Err(CandidateError::schema(format!(
                "the OBJ path '{}' is not a relative path inside the candidate",
                change.obj
            )));
        }
        changes.push(CandidateChange {
            component: change.component.clone(),
            action,
            obj: change.obj.clone(),
        });
    }

    let origin = CandidateOrigin::from_text(&raw.provenance.origin).ok_or_else(|| {
        CandidateError::schema(format!(
            "the origin '{}' is not one of human, ai_author, test_backend",
            raw.provenance.origin
        ))
    })?;

    let budget = Budget {
        max_calls: budget_u64(&raw.budget, "max_calls", true)?,
        max_seconds: budget_u64(&raw.budget, "max_seconds", false)?,
        max_cost_usd: budget_cost(&raw.budget)?,
    };

    Ok(EditCandidate {
        schema: raw.schema,
        candidate_id: raw.candidate_id,
        base: CandidateBase {
            package_id: raw.base.package_id,
            package_revision: raw.base.package_revision,
            scene_revision: raw.base.scene_revision,
            run_id: raw.base.run_id,
        },
        task: CandidateTask {
            task_id: raw.task.task_id,
            allowed_components: raw.task.allowed_components,
            allowed_region: region,
            protected: raw.task.protected,
            allowed_intersections: raw.task.allowed_intersections,
            require_author_acceptance: raw.task.require_author_acceptance,
            max_changed_vertices: raw.task.max_changed_vertices,
        },
        changes,
        provenance: Provenance {
            origin,
            detail: raw.provenance.detail,
        },
        budget,
    })
}

/// Convert a budget field into an unsigned integer.
fn budget_u64(budget: &RawBudget, field: &str, minimum_one: bool) -> Result<u64, CandidateError> {
    let value = match field {
        "max_calls" => budget.max_calls,
        "max_seconds" => budget.max_seconds,
        _ => {
            return Err(CandidateError::budget(format!(
                "the budget field '{field}' is unknown"
            )))
        }
    };
    if !value.is_finite() {
        return Err(CandidateError::budget(format!(
            "the budget field '{field}' is not finite"
        )));
    }
    if value < 0.0 {
        return Err(CandidateError::budget(format!(
            "the budget field '{field}' is negative"
        )));
    }
    if value.fract() != 0.0 {
        return Err(CandidateError::budget(format!(
            "the budget field '{field}' is not an integer"
        )));
    }
    if minimum_one && value < 1.0 {
        return Err(CandidateError::budget(
            "the budget field 'max_calls' must be at least 1".to_string(),
        ));
    }
    Ok(value as u64)
}

/// Validate the cost of a budget.
fn budget_cost(budget: &RawBudget) -> Result<f64, CandidateError> {
    if !budget.max_cost_usd.is_finite() {
        return Err(CandidateError::budget(
            "the budget field 'max_cost_usd' is not finite".to_string(),
        ));
    }
    if budget.max_cost_usd < 0.0 {
        return Err(CandidateError::budget(
            "the budget field 'max_cost_usd' is negative".to_string(),
        ));
    }
    Ok(budget.max_cost_usd)
}

/// True when the name is a usable component (or candidate) name.
///
/// A name starts with a letter or a digit and carries at most 63 further
/// characters of `[A-Za-z0-9_.-]`.
pub fn is_valid_component_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > COMPONENT_NAME_MAX_LEN {
        return false;
    }
    if !bytes[0].is_ascii_alphanumeric() {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'.' || *byte == b'-')
}

/// True when the path is a relative OBJ path that stays inside its directory.
pub fn is_safe_relative_obj_path(path: &str) -> bool {
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') {
        return false;
    }
    if path.contains(':') {
        return false;
    }
    if !path.to_ascii_lowercase().ends_with(".obj") {
        return false;
    }
    let parts: Vec<&str> = path.split(['/', '\\']).collect();
    for part in parts {
        if part.is_empty() || part == "." || part == ".." {
            return false;
        }
    }
    true
}
