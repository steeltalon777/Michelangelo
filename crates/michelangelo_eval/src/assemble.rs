//! The assembly of the evaluation report.

use std::collections::BTreeMap;

use michelangelo_evidence::gate::AUTHOR_CONDITION_KIND;
use michelangelo_evidence::report::{
    AuthorDecision, ClaimStatus, EvaluationCheck, EvaluationInputs, EvaluationReport,
    EvaluationResults, EvaluationTarget, EvaluationThresholds, GroundTruthState, HiddenClaim,
    HiddenSection, PackageView, ReportIntegrity, TargetKind, TriState, ViewPin, VisibleSection,
    VisibleSet, VisibleView, EVALUATION_REPORT_SCHEMA,
};
use serde_json::Value;

use crate::compare::CompareOutcome;

/// The inputs of the assembly of a report.
pub struct Assembly<'a> {
    /// The identifier of the evaluation.
    pub evaluation_id: &'a str,
    /// The creation time of the report.
    pub created_at: String,
    /// The identifier of the run.
    pub run_id: &'a str,
    /// The kind of the geometry that was measured.
    pub target_kind: TargetKind,
    /// The identifier of the geometry that was measured.
    pub target_id: &'a str,
    /// The aggregate digest of the geometry that was measured.
    pub target_aggregate_sha256: &'a str,
    /// The scene revision the geometry was built on.
    pub base_scene_revision: Option<&'a str>,
    /// The digest of the copy of the input package of the run.
    pub package_sha256: Option<String>,
    /// The pinned source views.
    pub source_views: &'a [PackageView],
    /// The pinned held-out views.
    pub held_out_views: Option<&'a [PackageView]>,
    /// The digest of the held-out package.
    pub evaluation_package_sha256: Option<String>,
    /// The diagnostic of a held-out set that could not be used.
    pub held_out_diagnostic: Option<String>,
    /// The thresholds the evaluation applied.
    pub thresholds: EvaluationThresholds,
    /// The comparison of the target.
    pub target_outcome: &'a CompareOutcome,
    /// The comparison of the base revision.
    pub base_outcome: &'a CompareOutcome,
    /// The copy of the input package of the run.
    pub package: &'a Value,
    /// The immutable report of the run.
    pub run_report: Option<&'a Value>,
    /// The versions of the tools.
    pub tool_versions: BTreeMap<String, String>,
}

/// Assemble the immutable report of an evaluation.
pub fn assemble(input: &Assembly<'_>) -> EvaluationReport {
    let pins = pinned_views(input);
    let held_out = held_out_section(input);
    let visible = visible_section(input, held_out.clone());
    let (checks, conditions) = author_conditions(input);
    let projection = projection_conformance(input);

    let mut acceptance_basis: Vec<String> = Vec::new();
    acceptance_basis.push(format!("projection_conformance={}", projection.as_str()));
    acceptance_basis.push(format!(
        "author_geometric_conditions={}",
        conditions.as_str()
    ));
    acceptance_basis.push(format!(
        "per_view_regression_max={}",
        input.thresholds.per_view_regression_max
    ));
    acceptance_basis.push(String::from(
        "the evaluation is registered by the Core: the registry holds the digest of the report",
    ));
    acceptance_basis.push(String::from(
        "the hidden claims stay unverified: the acceptance of a geometry does not close an unknown",
    ));

    EvaluationReport {
        schema: EVALUATION_REPORT_SCHEMA.to_string(),
        evaluation_id: input.evaluation_id.to_string(),
        created_at: input.created_at.clone(),
        run_id: input.run_id.to_string(),
        target: EvaluationTarget {
            kind: input.target_kind,
            id: input.target_id.to_string(),
            aggregate_sha256: input.target_aggregate_sha256.to_string(),
            base_scene_revision: input.base_scene_revision.map(|value| value.to_string()),
        },
        inputs: EvaluationInputs {
            package_sha256: input.package_sha256.clone(),
            views: pins,
            evaluation_package_sha256: input.evaluation_package_sha256.clone(),
        },
        thresholds: input.thresholds.clone(),
        results: EvaluationResults {
            projection_conformance: projection,
            author_geometric_conditions: conditions,
            independent_ground_truth: TriState::Unavailable,
            author_hidden_decision: AuthorDecision::Pending,
        },
        visible,
        hidden: hidden_section(input),
        checks,
        acceptance_basis,
        tool_versions: input.tool_versions.clone(),
        integrity: ReportIntegrity::default(),
    }
}

/// The pinned views of the evaluation.
fn pinned_views(input: &Assembly<'_>) -> Vec<ViewPin> {
    let mut pins: Vec<ViewPin> = input
        .source_views
        .iter()
        .map(|view| view.pin(false))
        .collect();
    if let Some(held_out) = input.held_out_views {
        for view in held_out {
            pins.push(view.pin(true));
        }
    }
    pins.sort_by(|left, right| left.view_id.cmp(&right.view_id));
    pins
}

/// The visible part of the report.
fn visible_section(input: &Assembly<'_>, held_out: Option<VisibleSet>) -> VisibleSection {
    let mut views: Vec<VisibleView> = Vec::new();
    for view in &input.target_outcome.source.views {
        let base_iou = input
            .base_outcome
            .source
            .views
            .iter()
            .find(|base| base.view_id == view.view_id)
            .map(|base| base.iou);
        let regression = base_iou.map(|base| base - view.iou);
        views.push(VisibleView {
            view_id: view.view_id.clone(),
            iou: view.iou,
            bbox_error_px: view.bbox_error_px,
            base_iou,
            regression,
        });
    }
    views.sort_by(|left, right| left.view_id.cmp(&right.view_id));

    let mut verdict = TriState::Pass;
    if input.target_outcome.source.views.is_empty() {
        verdict = TriState::Unavailable;
    } else {
        if input.target_outcome.source.mean_iou < input.thresholds.source_mean_iou
            || input.target_outcome.source.min_iou < input.thresholds.source_min_iou
        {
            verdict = TriState::Fail;
        }
        let limit = input.thresholds.per_view_regression_max;
        for view in &views {
            match view.regression {
                Some(regression) if regression > limit => verdict = TriState::Fail,
                Some(_) => {}
                None => {
                    if verdict == TriState::Pass {
                        verdict = TriState::Unavailable;
                    }
                }
            }
        }
    }

    VisibleSection {
        views,
        mean_iou: input.target_outcome.source.mean_iou,
        min_iou: input.target_outcome.source.min_iou,
        base_mean_iou: Some(input.base_outcome.source.mean_iou),
        verdict,
        held_out,
    }
}

/// The measured held-out views.
fn held_out_section(input: &Assembly<'_>) -> Option<VisibleSet> {
    let evaluation = input.target_outcome.evaluation.as_ref()?;
    let mut views: Vec<VisibleView> = Vec::new();
    for view in &evaluation.views {
        views.push(VisibleView {
            view_id: view.view_id.clone(),
            iou: view.iou,
            bbox_error_px: view.bbox_error_px,
            base_iou: None,
            regression: None,
        });
    }
    views.sort_by(|left, right| left.view_id.cmp(&right.view_id));
    Some(VisibleSet {
        views,
        mean_iou: evaluation.mean_iou,
        min_iou: evaluation.min_iou,
    })
}

/// The conformance of the projections.
fn projection_conformance(input: &Assembly<'_>) -> TriState {
    if input.target_outcome.source.views.is_empty() {
        return TriState::Unavailable;
    }
    if input.target_outcome.source.mean_iou < input.thresholds.source_mean_iou
        || input.target_outcome.source.min_iou < input.thresholds.source_min_iou
    {
        return TriState::Fail;
    }
    if let Some(evaluation) = &input.target_outcome.evaluation {
        if evaluation.views.is_empty() {
            return TriState::Fail;
        }
        if evaluation.mean_iou < input.thresholds.evaluation_mean_iou
            || evaluation.min_iou < input.thresholds.evaluation_min_iou
        {
            return TriState::Fail;
        }
    }
    TriState::Pass
}

/// The checks of the report and the state of the author conditions.
fn author_conditions(input: &Assembly<'_>) -> (Vec<EvaluationCheck>, TriState) {
    let mut checks: Vec<EvaluationCheck> = Vec::new();
    let measurement_ids = michelangelo_evidence::report::package_measurement_ids(input.package);
    let mut state = TriState::Pass;
    for id in &measurement_ids {
        let measurement = input
            .target_outcome
            .measurements
            .iter()
            .find(|measurement| measurement.id == *id);
        let (status, detail) = match measurement {
            Some(measurement) => match measurement.within_tolerance {
                Some(true) => (
                    TriState::Pass,
                    format!(
                        "the measured value {} m is inside the declared tolerance of the measurement '{}'",
                        format_optional(measurement.measured),
                        measurement.id
                    ),
                ),
                Some(false) => (
                    TriState::Fail,
                    format!(
                        "the measured value {} m is outside the declared tolerance of the measurement '{}'",
                        format_optional(measurement.measured),
                        measurement.id
                    ),
                ),
                None => (
                    TriState::Unavailable,
                    format!(
                        "the measurement '{}' could not be checked against the geometry of the target",
                        measurement.id
                    ),
                ),
            },
            None => (
                TriState::Unavailable,
                format!(
                    "the comparator did not report the declared measurement '{id}'"
                ),
            ),
        };
        if status == TriState::Fail {
            state = TriState::Fail;
        } else if status == TriState::Unavailable && state == TriState::Pass {
            state = TriState::Unavailable;
        }
        checks.push(EvaluationCheck {
            id: id.clone(),
            kind: AUTHOR_CONDITION_KIND.to_string(),
            required: true,
            status,
            detail,
        });
    }
    if measurement_ids.is_empty() {
        checks.push(EvaluationCheck {
            id: String::from("author_conditions"),
            kind: AUTHOR_CONDITION_KIND.to_string(),
            required: false,
            status: TriState::Pass,
            detail: String::from(
                "the package declares no measurable author condition: nothing has to be verified",
            ),
        });
    }

    // The held-out set.
    let held_out_status = match (&input.held_out_diagnostic, &input.target_outcome.evaluation) {
        (Some(_), _) => TriState::Unavailable,
        (None, None) => TriState::Unavailable,
        (None, Some(evaluation)) => {
            if evaluation.views.is_empty() {
                TriState::Unavailable
            } else {
                TriState::Pass
            }
        }
    };
    let held_out_detail = match &input.held_out_diagnostic {
        Some(diagnostic) => diagnostic.clone(),
        None => match &input.target_outcome.evaluation {
            Some(evaluation) => format!(
                "the held-out views were measured (the mean IoU is {:.4}, the smallest IoU is {:.4})",
                evaluation.mean_iou, evaluation.min_iou
            ),
            None => String::from(
                "no held-out package was given: the held-out result is unavailable",
            ),
        },
    };
    checks.push(EvaluationCheck {
        id: String::from("held_out"),
        kind: String::from("held_out"),
        required: false,
        status: held_out_status,
        detail: held_out_detail,
    });

    // The independent ground truth.
    checks.push(EvaluationCheck {
        id: String::from("ground_truth"),
        kind: String::from("ground_truth"),
        required: false,
        status: TriState::Unavailable,
        detail: String::from(
            "the comparison against an independent ground truth is not implemented in v0: the absence of a ground truth is never a pass",
        ),
    });

    // The hidden claims.
    checks.push(EvaluationCheck {
        id: String::from("hidden_claims"),
        kind: String::from("hidden"),
        required: false,
        status: TriState::Unavailable,
        detail: String::from(
            "the hidden claims have no resolvable evidence yet: they wait for the author decision of IT-8C",
        ),
    });

    checks.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.id.cmp(&right.id))
    });
    (checks, state)
}

/// The hidden part of the report.
fn hidden_section(input: &Assembly<'_>) -> HiddenSection {
    let mut claims: Vec<HiddenClaim> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut push = |id: String, kind: &str| {
        if id.is_empty() || seen.contains(&id) {
            return;
        }
        seen.push(id.clone());
        claims.push(HiddenClaim {
            id,
            kind: kind.to_string(),
            status: ClaimStatus::Unverified,
            evidence_refs: Vec::new(),
            requires_ground_truth: false,
        });
    };
    for id in michelangelo_evidence::report::package_unknown_ids(input.package) {
        push(id, "unknown");
    }
    if let Some(supplements) = input
        .package
        .get("supplements")
        .and_then(|value| value.as_array())
    {
        for supplement in supplements {
            let id = supplement
                .get("supplement_id")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            push(id, "supplement");
        }
    }
    if let Some(report) = input.run_report {
        if let Some(unknowns) = report
            .get("build")
            .and_then(|build| build.get("unknowns"))
            .and_then(|value| value.as_array())
        {
            for unknown in unknowns {
                let id = unknown
                    .get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string();
                push(id, "unknown");
            }
        }
    }
    claims.sort_by(|left, right| left.id.cmp(&right.id));
    HiddenSection {
        ground_truth: GroundTruthState::Absent,
        claims,
        verdict: AuthorDecision::Pending,
    }
}

/// The text of an optional number.
fn format_optional(value: Option<f64>) -> String {
    match value {
        Some(value) => format!("{value:.6}"),
        None => String::from("unknown"),
    }
}
