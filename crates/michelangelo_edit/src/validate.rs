//! The deterministic validator and the decision report.
//!
//! The validator checks a candidate against the current scene revision of a
//! run and produces the ordered list of checks and the diagnostics of the
//! decision report (`michelangelo.edit-decision/0`).
//!
//! The checks always run in the same order, so two runs over the same inputs
//! produce byte identical reports:
//!
//! | Check | Diagnostic codes |
//! |---|---|
//! | `candidate_schema` | `INTERNAL_ERROR` |
//! | `identity` | `IDENTITY_MISMATCH` |
//! | `base_revision` | `STALE_BASE` |
//! | `task_scope` | `NOT_ALLOWED` |
//! | `budget` | `BUDGET_INVALID` |
//! | `components` | `COMPONENT_UNKNOWN`, `NOT_ALLOWED`, `UNSUPPORTED_ACTION` |
//! | `protected` | `PROTECTED_MODIFIED` |
//! | `geometry` | `OBJ_PARSE`, `GEOMETRY_INVALID` |
//! | `region` | `REGION_VIOLATION` |
//! | `acceptance` | `ACCEPTANCE_REQUIRED` |
//! | `policy` | `POLICY_DENIED` |

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::candidate::{CandidateAction, EditCandidate};
use crate::obj::{parse_obj_file, Bbox};
use crate::policy::{EditPolicy, PolicyDecision};
use crate::scene::Scene;
use crate::DECISION_SCHEMA;

/// The decision of a validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// The candidate was applied and produced a scene revision.
    Accepted,
    /// The candidate was refused.
    Rejected,
    /// The candidate is valid and waits for an explicit acceptance.
    Proposed,
    /// The candidate could not be interpreted.
    Error,
}

impl Decision {
    /// The decision as the text used in the document.
    pub fn as_str(&self) -> &'static str {
        match self {
            Decision::Accepted => "accepted",
            Decision::Rejected => "rejected",
            Decision::Proposed => "proposed",
            Decision::Error => "error",
        }
    }

    /// The CLI exit code of the decision.
    pub fn exit_code(&self) -> u8 {
        match self {
            Decision::Accepted | Decision::Proposed => 0,
            Decision::Rejected => 1,
            Decision::Error => 2,
        }
    }
}

/// The status of a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// The check succeeded.
    Pass,
    /// The check failed.
    Fail,
    /// The check did not apply.
    Skipped,
}

impl CheckStatus {
    /// The status as the text used in the document.
    pub fn as_str(&self) -> &'static str {
        match self {
            CheckStatus::Pass => "pass",
            CheckStatus::Fail => "fail",
            CheckStatus::Skipped => "skipped",
        }
    }
}

/// The severity of a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// The diagnostic refuses the candidate.
    Error,
    /// The diagnostic informs the author.
    Warning,
    /// The diagnostic informs about the state of the candidate.
    Info,
}

impl Severity {
    /// The severity as the text used in the document.
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
}

/// One check of a decision report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    /// The name of the check.
    pub check: String,
    /// The status of the check.
    pub status: CheckStatus,
    /// The human readable detail of the check.
    pub detail: String,
}

/// One diagnostic of a decision report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// The stable diagnostic code.
    pub code: String,
    /// The severity of the diagnostic.
    pub severity: Severity,
    /// The human readable message.
    pub message: String,
    /// The address of the offending element (None when it has none).
    pub address: Option<String>,
}

/// The decision report of a candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionReport {
    /// The schema identifier.
    pub schema: String,
    /// The identifier of the candidate.
    pub candidate_id: String,
    /// The decision.
    pub decision: Decision,
    /// The scene revision the candidate was built on.
    pub base_scene_revision: Option<String>,
    /// The scene revision the decision produced.
    pub resulting_scene_revision: Option<String>,
    /// The ordered checks.
    pub checks: Vec<Check>,
    /// The diagnostics.
    pub diagnostics: Vec<Diagnostic>,
    /// The time of the decision (RFC 3339, UTC).
    pub decided_at: String,
    /// The author that decided the candidate.
    pub decided_by: Option<String>,
    /// The identifier of the snapshot of the candidate (None for a decision
    /// that was taken before the verification contract).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    /// The aggregate digest of the geometry of the snapshot of the candidate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate_sha256: Option<String>,
}

impl DecisionReport {
    /// A report without any check.
    pub fn empty(
        candidate_id: impl Into<String>,
        decision: Decision,
        decided_at: impl Into<String>,
    ) -> Self {
        DecisionReport {
            schema: DECISION_SCHEMA.to_string(),
            candidate_id: candidate_id.into(),
            decision,
            base_scene_revision: None,
            resulting_scene_revision: None,
            checks: Vec::new(),
            diagnostics: Vec::new(),
            decided_at: decided_at.into(),
            decided_by: None,
            snapshot_id: None,
            aggregate_sha256: None,
        }
    }

    /// The JSON representation of the report (pretty, deterministic).
    pub fn to_json(&self) -> Result<String, crate::EditError> {
        serde_json::to_string_pretty(self).map_err(|err| crate::EditError::Internal {
            message: format!("the decision report cannot be serialized: {err}"),
        })
    }

    /// Read a decision report from a file.
    pub fn load(path: &Path) -> Result<Self, crate::EditError> {
        let text = std::fs::read_to_string(path).map_err(|err| {
            crate::io_error(path, format!("the decision report cannot be read: {err}"))
        })?;
        serde_json::from_str(&text).map_err(|err| crate::EditError::Scene {
            message: format!(
                "the decision report '{}' is not usable: {err}",
                path.display()
            ),
        })
    }

    /// True when the report carries an error diagnostic.
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }
}

/// The mode the validator runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationMode {
    /// The candidate is submitted and waits for an acceptance.
    Submit,
    /// The candidate is accepted by an explicit author.
    Accept,
}

/// The context of a validation.
#[derive(Debug)]
pub struct ValidationContext<'a> {
    /// The candidate that is validated.
    pub candidate: &'a EditCandidate,
    /// The directory of the candidate (the geometry is relative to it).
    pub candidate_dir: &'a Path,
    /// The scene revision the candidate is validated against.
    pub base_scene: &'a Scene,
    /// The revision the store currently points to.
    pub current_scene_revision: &'a str,
    /// The mode the validator runs in.
    pub mode: ValidationMode,
    /// The author that decides the candidate (None for a submission).
    pub decided_by: Option<&'a str>,
}

/// One change whose geometry was read and validated.
#[derive(Debug, Clone)]
pub struct PreparedChange {
    /// The component the change applies to.
    pub component: String,
    /// The action of the change.
    pub action: CandidateAction,
    /// The OBJ of the change inside the candidate directory.
    pub source: PathBuf,
    /// The SHA-256 of the bytes of the OBJ.
    pub sha256: String,
    /// The bounding box of the geometry.
    pub bbox: Bbox,
    /// The number of vertices of the geometry.
    pub vertex_count: usize,
}

/// The outcome of a validation.
#[derive(Debug, Clone)]
pub struct ValidationOutcome {
    /// The ordered checks.
    pub checks: Vec<Check>,
    /// The diagnostics.
    pub diagnostics: Vec<Diagnostic>,
    /// True when every blocking check succeeded.
    pub valid: bool,
    /// True when the candidate may be applied without an explicit author
    /// acceptance.
    pub auto_accept: bool,
    /// The changes whose geometry was read.
    pub prepared: Vec<PreparedChange>,
}

impl ValidationOutcome {
    /// The codes of the error diagnostics.
    pub fn error_codes(&self) -> Vec<String> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .map(|diagnostic| diagnostic.code.clone())
            .collect()
    }
}

/// Validate a candidate against the current scene revision.
pub fn validate(context: &ValidationContext<'_>, policy: Option<&EditPolicy>) -> ValidationOutcome {
    let candidate = context.candidate;
    let mut checks: Vec<Check> = Vec::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut prepared: Vec<PreparedChange> = Vec::new();
    let mut blocking_failure = false;

    // 1. The schema of the candidate.
    checks.push(Check {
        check: "candidate_schema".to_string(),
        status: CheckStatus::Pass,
        detail: format!("the candidate follows '{}'", candidate.schema),
    });

    // 2. The identity of the base.
    let identity_ok = candidate.base.package_id == context.base_scene.source.package_id
        && candidate.base.package_revision == context.base_scene.source.package_revision
        && candidate.base.run_id == context.base_scene.run_id;
    if identity_ok {
        checks.push(Check {
            check: "identity".to_string(),
            status: CheckStatus::Pass,
            detail: format!(
                "the base package {} (revision {}) of the run '{}' matches the scene",
                candidate.base.package_id, candidate.base.package_revision, candidate.base.run_id
            ),
        });
    } else {
        blocking_failure = true;
        checks.push(Check {
            check: "identity".to_string(),
            status: CheckStatus::Fail,
            detail: format!(
                "the candidate was built on the package {} (revision {}) of the run '{}' but the scene belongs to the package {} (revision {}) of the run '{}'",
                candidate.base.package_id,
                candidate.base.package_revision,
                candidate.base.run_id,
                context.base_scene.source.package_id,
                context.base_scene.source.package_revision,
                context.base_scene.run_id
            ),
        });
        diagnostics.push(Diagnostic {
            code: "IDENTITY_MISMATCH".to_string(),
            severity: Severity::Error,
            message: "the base of the candidate does not match the scene".to_string(),
            address: Some("base".to_string()),
        });
    }

    // 3. The base revision.
    if candidate.base.scene_revision == context.current_scene_revision {
        checks.push(Check {
            check: "base_revision".to_string(),
            status: CheckStatus::Pass,
            detail: format!(
                "the candidate was built on the current revision '{}'",
                candidate.base.scene_revision
            ),
        });
    } else {
        blocking_failure = true;
        checks.push(Check {
            check: "base_revision".to_string(),
            status: CheckStatus::Fail,
            detail: format!(
                "the candidate was built on the revision '{}' but the current revision is '{}'",
                candidate.base.scene_revision, context.current_scene_revision
            ),
        });
        diagnostics.push(Diagnostic {
            code: "STALE_BASE".to_string(),
            severity: Severity::Error,
            message: format!(
                "the candidate was built on the revision '{}' but the current revision is '{}'",
                candidate.base.scene_revision, context.current_scene_revision
            ),
            address: Some("base.scene_revision".to_string()),
        });
    }

    // 4. The scope of the task.
    let mut scope_detail = format!(
        "the task '{}' allows the components [{}] and protects [{}]",
        candidate.task.task_id,
        candidate.task.allowed_components.join(", "),
        candidate.task.protected.join(", ")
    );
    let mut scope_failed = false;
    if candidate.task.allowed_components.is_empty() {
        scope_failed = true;
        scope_detail = "the task allows no component".to_string();
        diagnostics.push(Diagnostic {
            code: "NOT_ALLOWED".to_string(),
            severity: Severity::Error,
            message: "the allowed component set of the task is empty".to_string(),
            address: Some("task.allowed_components".to_string()),
        });
    }
    for name in &candidate.task.protected {
        if candidate.task.allowed_components.contains(name) {
            scope_failed = true;
            scope_detail =
                format!("the component '{name}' is allowed and protected at the same time");
            diagnostics.push(Diagnostic {
                code: "NOT_ALLOWED".to_string(),
                severity: Severity::Error,
                message: format!(
                    "the component '{name}' is allowed and protected at the same time"
                ),
                address: Some(format!("task.protected.{name}")),
            });
            break;
        }
    }
    checks.push(Check {
        check: "task_scope".to_string(),
        status: if scope_failed {
            CheckStatus::Fail
        } else {
            CheckStatus::Pass
        },
        detail: scope_detail,
    });
    blocking_failure = blocking_failure || scope_failed;

    // 5. The actions and the components of the changes.
    let mut component_failed = false;
    let mut component_detail = String::new();
    for change in &candidate.changes {
        if change.action == CandidateAction::Delete {
            component_failed = true;
            component_detail = format!(
                "the action 'delete' of the component '{}' is not supported in v0",
                change.component
            );
            diagnostics.push(Diagnostic {
                code: "UNSUPPORTED_ACTION".to_string(),
                severity: Severity::Error,
                message: component_detail.clone(),
                address: Some(format!("changes.{}.action", change.component)),
            });
            continue;
        }
        if !candidate
            .task
            .allowed_components
            .contains(&change.component)
        {
            component_failed = true;
            component_detail = format!(
                "the component '{}' is not part of the allowed component set",
                change.component
            );
            diagnostics.push(Diagnostic {
                code: "NOT_ALLOWED".to_string(),
                severity: Severity::Error,
                message: component_detail.clone(),
                address: Some(format!("changes.{}", change.component)),
            });
            continue;
        }
        let exists = context.base_scene.component(&change.component).is_some();
        match change.action {
            CandidateAction::Replace => {
                if !exists {
                    component_failed = true;
                    component_detail = format!(
                        "the component '{}' is not part of the base scene",
                        change.component
                    );
                    diagnostics.push(Diagnostic {
                        code: "COMPONENT_UNKNOWN".to_string(),
                        severity: Severity::Error,
                        message: component_detail.clone(),
                        address: Some(format!("changes.{}", change.component)),
                    });
                }
            }
            CandidateAction::Add => {
                if exists {
                    component_failed = true;
                    component_detail = format!(
                        "the component '{}' already exists and cannot be added",
                        change.component
                    );
                    diagnostics.push(Diagnostic {
                        code: "NOT_ALLOWED".to_string(),
                        severity: Severity::Error,
                        message: component_detail.clone(),
                        address: Some(format!("changes.{}", change.component)),
                    });
                }
            }
            CandidateAction::Delete => {}
        }
    }
    if !component_failed {
        component_detail = format!(
            "the candidate changes the components [{}]",
            candidate.changed_components().join(", ")
        );
    }
    checks.push(Check {
        check: "components".to_string(),
        status: if component_failed {
            CheckStatus::Fail
        } else {
            CheckStatus::Pass
        },
        detail: component_detail,
    });
    blocking_failure = blocking_failure || component_failed;

    // 6. The protected components.
    let mut protected_failed = false;
    let mut protected_detail = String::new();
    for change in &candidate.changes {
        let listed = candidate.task.protected.contains(&change.component);
        let flagged = context
            .base_scene
            .component(&change.component)
            .map(|component| component.protected)
            .unwrap_or(false);
        if listed || flagged {
            protected_failed = true;
            protected_detail = format!(
                "the component '{}' is protected and must not change",
                change.component
            );
            diagnostics.push(Diagnostic {
                code: "PROTECTED_MODIFIED".to_string(),
                severity: Severity::Error,
                message: protected_detail.clone(),
                address: Some(format!("changes.{}", change.component)),
            });
        }
    }
    if !protected_failed {
        protected_detail = format!(
            "the protected components [{}] stay untouched",
            candidate.task.protected.join(", ")
        );
    }
    checks.push(Check {
        check: "protected".to_string(),
        status: if protected_failed {
            CheckStatus::Fail
        } else {
            CheckStatus::Pass
        },
        detail: protected_detail,
    });
    blocking_failure = blocking_failure || protected_failed;

    // 7. The geometry of the changes.
    let mut geometry_failed = false;
    let mut geometry_detail = String::new();
    let mut changed_vertices: u64 = 0;
    for change in &candidate.changes {
        let source = context.candidate_dir.join(&change.obj);
        match parse_obj_file(&source) {
            Ok(file) => {
                changed_vertices += file.mesh.vertex_count() as u64;
                geometry_detail = format!(
                    "the geometry of the component '{}' carries {} vertices",
                    change.component,
                    file.mesh.vertex_count()
                );
                prepared.push(PreparedChange {
                    component: change.component.clone(),
                    action: change.action,
                    source,
                    sha256: file.sha256,
                    bbox: file.mesh.bbox,
                    vertex_count: file.mesh.vertex_count(),
                });
            }
            Err(error) => {
                geometry_failed = true;
                let code = error.code();
                geometry_detail = format!(
                    "the geometry of the component '{}' is not usable: {error}",
                    change.component
                );
                diagnostics.push(Diagnostic {
                    code: code.to_string(),
                    severity: Severity::Error,
                    message: format!(
                        "the geometry of the component '{}' is not usable: {error}",
                        change.component
                    ),
                    address: Some(format!("changes.{}.obj", change.component)),
                });
            }
        }
    }
    checks.push(Check {
        check: "geometry".to_string(),
        status: if geometry_failed {
            CheckStatus::Fail
        } else {
            CheckStatus::Pass
        },
        detail: geometry_detail,
    });
    blocking_failure = blocking_failure || geometry_failed;

    // 8. The region of the task.
    let mut region_failed = false;
    let mut region_detail = String::new();
    for change in &prepared {
        if !candidate.task.allowed_region.contains(&change.bbox) {
            region_failed = true;
            region_detail = format!(
                "the geometry of the component '{}' leaves the allowed region",
                change.component
            );
            diagnostics.push(Diagnostic {
                code: "REGION_VIOLATION".to_string(),
                severity: Severity::Error,
                message: region_detail.clone(),
                address: Some(format!("changes.{}.obj", change.component)),
            });
        }
    }
    if !region_failed {
        region_detail = format!(
            "the geometry stays inside the region [{}] .. [{}]",
            format_vector(&candidate.task.allowed_region.min),
            format_vector(&candidate.task.allowed_region.max)
        );
    }
    checks.push(Check {
        check: "region".to_string(),
        status: if region_failed {
            CheckStatus::Fail
        } else {
            CheckStatus::Pass
        },
        detail: region_detail,
    });
    blocking_failure = blocking_failure || region_failed;

    // 9. The budget of the task.
    let budget_ok = changed_vertices <= candidate.task.max_changed_vertices;
    checks.push(Check {
        check: "budget".to_string(),
        status: if budget_ok {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        },
        detail: format!(
            "the change touches {changed_vertices} vertices, the task allows {}",
            candidate.task.max_changed_vertices
        ),
    });
    if !budget_ok {
        blocking_failure = true;
        diagnostics.push(Diagnostic {
            code: "BUDGET_INVALID".to_string(),
            severity: Severity::Error,
            message: format!(
                "the change touches {changed_vertices} vertices but the task allows {}",
                candidate.task.max_changed_vertices
            ),
            address: Some("task.max_changed_vertices".to_string()),
        });
    }

    // 10. The acceptance.
    let (acceptance_check, acceptance_diagnostic) = match context.mode {
        ValidationMode::Submit => (
            Check {
                check: "acceptance".to_string(),
                status: CheckStatus::Skipped,
                detail: "the acceptance is deferred to an explicit author acceptance".to_string(),
            },
            Some(Diagnostic {
                code: "ACCEPTANCE_REQUIRED".to_string(),
                severity: Severity::Info,
                message: "the candidate waits for an explicit author acceptance".to_string(),
                address: None,
            }),
        ),
        ValidationMode::Accept => (
            Check {
                check: "acceptance".to_string(),
                status: CheckStatus::Pass,
                detail: format!(
                    "the candidate was accepted by '{}'",
                    context.decided_by.unwrap_or("unknown")
                ),
            },
            None,
        ),
    };
    checks.push(acceptance_check);
    if let Some(diagnostic) = acceptance_diagnostic {
        diagnostics.push(diagnostic);
    }

    // 11. The policy of the workspace.
    let decision = match context.mode {
        ValidationMode::Submit => policy_decision(candidate, policy),
        ValidationMode::Accept => PolicyDecision::Allow,
    };
    let (policy_check, policy_diagnostic, auto_accept) = match decision {
        PolicyDecision::Allow => (
            Check {
                check: "policy".to_string(),
                status: CheckStatus::Pass,
                detail: "the edit policy allows an automatic acceptance".to_string(),
            },
            None,
            true,
        ),
        PolicyDecision::NoPolicy => (
            Check {
                check: "policy".to_string(),
                status: CheckStatus::Skipped,
                detail: "the workspace carries no edit policy: an explicit author acceptance is required"
                    .to_string(),
            },
            None,
            false,
        ),
        PolicyDecision::Deferred { reason } => (
            Check {
                check: "policy".to_string(),
                status: CheckStatus::Skipped,
                detail: reason.clone(),
            },
            None,
            false,
        ),
        PolicyDecision::Denied { reason } => (
            Check {
                check: "policy".to_string(),
                status: CheckStatus::Skipped,
                detail: reason.clone(),
            },
            Some(Diagnostic {
                code: "POLICY_DENIED".to_string(),
                severity: Severity::Warning,
                message: reason,
                address: None,
            }),
            false,
        ),
    };
    checks.push(policy_check);
    if let Some(diagnostic) = policy_diagnostic {
        diagnostics.push(diagnostic);
    }

    let auto_accept = auto_accept && !blocking_failure && context.mode == ValidationMode::Submit;

    ValidationOutcome {
        checks,
        diagnostics,
        valid: !blocking_failure,
        auto_accept,
        prepared,
    }
}

/// The policy decision of a candidate.
fn policy_decision(candidate: &EditCandidate, policy: Option<&EditPolicy>) -> PolicyDecision {
    if candidate.task.require_author_acceptance {
        return PolicyDecision::Deferred {
            reason: "the task requires an explicit author acceptance".to_string(),
        };
    }
    match crate::policy::evaluate(policy, candidate) {
        PolicyDecision::NoPolicy => PolicyDecision::Deferred {
            reason:
                "the workspace carries no edit policy: an explicit author acceptance is required"
                    .to_string(),
        },
        other => other,
    }
}

/// Format a vector for a human readable message.
fn format_vector(vector: &[f64; 3]) -> String {
    format!("[{:.6}, {:.6}, {:.6}]", vector[0], vector[1], vector[2])
}
