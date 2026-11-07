//! The accept gate of an evaluation report (contract §7).
//!
//! The gate is a pure function: it compares the report with the pins of the
//! run and returns a typed error. It never trusts an unregistered report and
//! never trusts a hash that the report declares itself.

use crate::error::EvidenceError;
use crate::registry::EvaluationRegistry;
use crate::report::{ClaimStatus, EvaluationReport, TargetKind, TriState, ViewPin};

/// The kind of the author geometric conditions.
pub const AUTHOR_CONDITION_KIND: &str = "author_geometric_condition";

/// The owner-fixed per-view regression limit (IoU). A report may tighten it but
/// the gate never lets a report widen it.
pub const PER_VIEW_REGRESSION_MAX: f64 = 0.01;

/// The state of the run the gate compares the report with.
pub struct GateContext<'a> {
    /// The identifier of the run.
    pub run_id: &'a str,
    /// The kind of the geometry that is about to be accepted.
    pub target_kind: TargetKind,
    /// The identifier of the geometry that is about to be accepted.
    pub target_id: &'a str,
    /// The aggregate digest of the geometry that is about to be accepted.
    pub target_aggregate_sha256: &'a str,
    /// The scene revision the geometry was built on.
    pub base_scene_revision: Option<&'a str>,
    /// The digest of the copy of the input package of the run.
    pub current_package_sha256: Option<&'a str>,
    /// The views the run pins right now (the source views).
    pub current_source_views: &'a [ViewPin],
    /// The identifiers of the required author geometric conditions (the
    /// declared measurements of the package).
    pub required_condition_ids: &'a [String],
    /// The digest of the evaluation that the decision of the candidate
    /// recorded (None when the candidate carries no such decision).
    pub decision_evaluation_sha256: Option<&'a str>,
}

/// Verify an evaluation report against the pins of the run.
///
/// The checks are the ones of the contract §7:
///
/// 1. the evaluation is registered in the registry of the run,
/// 2. the re-computed digest of the report equals the registered digest and
///    the digest that the decision of the candidate recorded,
/// 3. the target of the report is the geometry that is about to be accepted
///    (the kind, the identifier, the aggregate digest and the base revision),
/// 4. the pinned inputs of the report are the inputs of the run right now,
/// 5. the projection conformance passes,
/// 6. every required author geometric condition passes,
/// 7. no source view regresses against the base revision,
/// 8. no claim that needs a ground truth is left unverified.
pub fn verify_report(
    registry: &EvaluationRegistry,
    report: &EvaluationReport,
    report_sha256: &str,
    context: &GateContext<'_>,
) -> Result<(), EvidenceError> {
    // 1. The report must be registered.
    let entry = match registry.entry(&report.evaluation_id) {
        Some(entry) => entry,
        None => {
            return Err(EvidenceError::Unregistered {
                message: format!(
                    "the evaluation '{}' is not registered in the registry of the run",
                    report.evaluation_id
                ),
            })
        }
    };

    // 2. The digest of the file must equal the registered digest.
    if entry.sha256 != report_sha256 {
        return Err(EvidenceError::Stale {
            message: format!(
                "the evaluation '{}' carries the digest {report_sha256} but the registry pins {}",
                report.evaluation_id, entry.sha256
            ),
        });
    }
    if let Some(decision_digest) = context.decision_evaluation_sha256 {
        if decision_digest != report_sha256 {
            return Err(EvidenceError::Stale {
                message: format!(
                    "the evaluation '{}' carries the digest {report_sha256} but the decision of the candidate recorded {decision_digest}",
                    report.evaluation_id
                ),
            });
        }
    }

    // 3. The target of the report must be the geometry that is accepted.
    if report.target.kind != context.target_kind || report.target.id != context.target_id {
        return Err(EvidenceError::Stale {
            message: format!(
                "the evaluation '{}' measured the {} '{}' but the acceptance is about the {} '{}'",
                report.evaluation_id,
                report.target.kind.as_str(),
                report.target.id,
                context.target_kind.as_str(),
                context.target_id
            ),
        });
    }
    if report.target.aggregate_sha256 != context.target_aggregate_sha256 {
        return Err(EvidenceError::Stale {
            message: format!(
                "the evaluation '{}' measured the geometry {} but the acceptance is about {}",
                report.evaluation_id,
                report.target.aggregate_sha256,
                context.target_aggregate_sha256
            ),
        });
    }
    if report.run_id != context.run_id {
        return Err(EvidenceError::Stale {
            message: format!(
                "the evaluation '{}' belongs to the run '{}' but the acceptance is about the run '{}'",
                report.evaluation_id, report.run_id, context.run_id
            ),
        });
    }
    let report_base = report.target.base_scene_revision.as_deref();
    if report_base != context.base_scene_revision {
        return Err(EvidenceError::Stale {
            message: format!(
                "the evaluation '{}' was made on the revision '{}' but the acceptance is about the revision '{}'",
                report.evaluation_id,
                report_base.unwrap_or("-"),
                context.base_scene_revision.unwrap_or("-")
            ),
        });
    }

    // 4. The pinned inputs must be the inputs of the run right now.
    if report.inputs.package_sha256.as_deref() != context.current_package_sha256 {
        return Err(EvidenceError::InputChanged {
            message: format!(
                "the evaluation '{}' pinned the package {} but the run carries {}",
                report.evaluation_id,
                report.inputs.package_sha256.as_deref().unwrap_or("-"),
                context.current_package_sha256.unwrap_or("-")
            ),
        });
    }
    let pinned: Vec<&ViewPin> = report
        .inputs
        .views
        .iter()
        .filter(|view| !view.held_out)
        .collect();
    if pinned.len() != context.current_source_views.len() {
        return Err(EvidenceError::InputChanged {
            message: format!(
                "the evaluation '{}' pinned {} source views but the run pins {}",
                report.evaluation_id,
                pinned.len(),
                context.current_source_views.len()
            ),
        });
    }
    let mut pinned_sorted: Vec<&ViewPin> = pinned;
    pinned_sorted.sort_by(|left, right| left.view_id.cmp(&right.view_id));
    let mut current_sorted: Vec<&ViewPin> = context.current_source_views.iter().collect();
    current_sorted.sort_by(|left, right| left.view_id.cmp(&right.view_id));
    for (pinned_view, current_view) in pinned_sorted.iter().zip(current_sorted.iter()) {
        if pinned_view.view_id != current_view.view_id
            || pinned_view.camera_digest != current_view.camera_digest
            || pinned_view.mask_sha256 != current_view.mask_sha256
            || pinned_view.resolution != current_view.resolution
        {
            return Err(EvidenceError::InputChanged {
                message: format!(
                    "the evaluation '{}' pinned the view '{}' with the camera digest {} but the run pins {}",
                    report.evaluation_id,
                    pinned_view.view_id,
                    pinned_view.camera_digest,
                    current_view.camera_digest
                ),
            });
        }
    }

    // 5. The projection conformance must pass.
    if report.results.projection_conformance != TriState::Pass {
        return Err(EvidenceError::VisibleFailed {
            message: format!(
                "the projection conformance of the evaluation '{}' is {}",
                report.evaluation_id,
                report.results.projection_conformance.as_str()
            ),
        });
    }

    // 6. Every required author geometric condition must pass.
    for id in context.required_condition_ids {
        let check = report
            .checks
            .iter()
            .find(|check| check.kind == AUTHOR_CONDITION_KIND && check.id == *id);
        match check {
            None => {
                return Err(EvidenceError::AuthorConditionFailed {
                    message: format!(
                        "the evaluation '{}' does not report the required author condition '{id}'",
                        report.evaluation_id
                    ),
                })
            }
            Some(check) if !check.required => {
                return Err(EvidenceError::AuthorConditionFailed {
                    message: format!(
                    "the author condition '{id}' of the evaluation '{}' is not marked as required",
                    report.evaluation_id
                ),
                })
            }
            Some(check) if check.status != TriState::Pass => {
                return Err(EvidenceError::AuthorConditionFailed {
                    message: format!(
                        "the required author condition '{id}' of the evaluation '{}' is {}: {}",
                        report.evaluation_id,
                        check.status.as_str(),
                        check.detail
                    ),
                })
            }
            Some(_) => {}
        }
    }
    for check in report.checks.iter() {
        if check.kind == AUTHOR_CONDITION_KIND && check.required && check.status != TriState::Pass {
            return Err(EvidenceError::AuthorConditionFailed {
                message: format!(
                    "the required author condition '{}' of the evaluation '{}' is {}: {}",
                    check.id,
                    report.evaluation_id,
                    check.status.as_str(),
                    check.detail
                ),
            });
        }
    }

    // 7. No source view may regress against the base revision. The owner fixed
    // the per-view limit at 0.01 IoU: a report may tighten it but never widen it.
    let limit = report
        .thresholds
        .per_view_regression_max
        .min(PER_VIEW_REGRESSION_MAX);
    for view in &report.visible.views {
        let regression = match view.regression {
            Some(regression) => regression,
            None => {
                return Err(EvidenceError::Regression {
                    message: format!(
                    "the view '{}' of the evaluation '{}' has no measurement of the base revision",
                    view.view_id, report.evaluation_id
                ),
                })
            }
        };
        if regression > limit {
            return Err(EvidenceError::Regression {
                message: format!(
                    "the view '{}' of the evaluation '{}' drops the IoU by {regression:.4} against the base revision (the limit is {limit:.4})",
                    view.view_id, report.evaluation_id
                ),
            });
        }
    }

    // 8. A claim that needs a ground truth must be verified.
    for claim in &report.hidden.claims {
        if claim.requires_ground_truth && claim.status != ClaimStatus::Verified {
            return Err(EvidenceError::NoGroundTruth {
                message: format!(
                    "the claim '{}' of the evaluation '{}' needs an independent ground truth that is not available",
                    claim.id, report.evaluation_id
                ),
            });
        }
    }

    Ok(())
}
