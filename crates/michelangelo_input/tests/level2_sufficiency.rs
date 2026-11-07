//! Level 2: sufficiency, completeness and the level 2 rules.

mod common;

use common::{
    copy_fixture, copy_tree, fixture_path, has_code, package_dir, read_manifest, write_manifest,
};
use michelangelo_input::{
    validate_package, CompletenessStatus, ReportStatus, Severity, ValidateOptions,
};

#[test]
fn the_full_fixture_is_complete() {
    let dir = fixture_path("valid_full");
    let report = validate_package(&dir, &ValidateOptions::default());

    assert_eq!(report.status, ReportStatus::ValidComplete);
    assert_eq!(report.completeness.status, CompletenessStatus::Complete);
    assert_eq!(report.completeness.calibrated_views, 2);
    assert_eq!(report.completeness.distinct_directions, 2);
    assert_eq!(report.completeness.axes_covered, vec!["x", "y"]);
    assert!(report.completeness.missing.is_empty());
    assert!(report.levels.format_safety.ok);
    assert!(report.levels.sufficiency.ok);
    // No error is raised; a warning about the writable package directory may
    // appear, depending on the modification times of the checked out files.
    assert!(
        report
            .diagnostics
            .iter()
            .all(|d| d.severity != Severity::Error),
        "{:?}",
        report.diagnostics
    );
    assert!(report.resources.iter().all(|resource| resource.verified));
}

#[test]
fn the_partial_fixture_is_valid_but_partial() {
    let dir = fixture_path("valid_partial");
    let report = validate_package(&dir, &ValidateOptions::default());

    assert_eq!(report.status, ReportStatus::ValidPartial);
    assert_eq!(report.completeness.status, CompletenessStatus::Partial);
    assert_eq!(report.completeness.calibrated_views, 1);
    assert_eq!(report.completeness.distinct_directions, 1);
    assert!(report
        .completeness
        .missing
        .contains(&"INSUFFICIENT_VIEW_DIRECTIONS".to_string()));
    assert!(report.levels.format_safety.ok);
}

#[test]
fn a_partial_package_fails_when_a_complete_package_is_required() {
    let dir = fixture_path("valid_partial");
    let options = ValidateOptions {
        require_complete: true,
        verify_hashes: true,
    };
    let report = validate_package(&dir, &options);

    assert_eq!(report.status, ReportStatus::Invalid);
    assert!(!report.levels.sufficiency.ok);
    assert!(report.levels.sufficiency.errors >= 1);
    let diagnostic = report
        .diagnostics
        .iter()
        .find(|d| d.code == "INSUFFICIENT_VIEW_DIRECTIONS" && d.level == 2)
        .expect("level 2 diagnostic");
    assert_eq!(diagnostic.severity, Severity::Error);
}

#[test]
fn a_package_without_any_calibrated_view_is_not_calibrated() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    for view in manifest["views"].as_array_mut().expect("views") {
        if let Some(object) = view.as_object_mut() {
            object.insert("kind".to_string(), serde_json::json!("reference"));
        }
    }
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(report
        .completeness
        .missing
        .contains(&"INSUFFICIENT_CALIBRATION".to_string()));
}

#[test]
fn two_views_with_the_same_direction_are_a_duplicate_warning() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    let mut duplicate = manifest["views"][0].clone();
    duplicate["view_id"] = serde_json::json!("front_copy");
    duplicate["image"]["path"] = serde_json::json!("images/right.png");
    duplicate["image"]["sha256"] =
        serde_json::json!(common::sha256_of(&dir.join("images/right.png")));
    duplicate["mask"]["path"] = serde_json::json!("masks/right.png");
    duplicate["mask"]["sha256"] =
        serde_json::json!(common::sha256_of(&dir.join("masks/right.png")));
    let mut views = manifest["views"].as_array().expect("views").clone();
    views.push(duplicate);
    manifest["views"] = serde_json::Value::Array(views);
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "DUPLICATE_VIEW_DIRECTION"));
    let diagnostic = report
        .diagnostics
        .iter()
        .find(|d| d.code == "DUPLICATE_VIEW_DIRECTION")
        .expect("diagnostic");
    assert_eq!(diagnostic.level, 2);
    assert_eq!(diagnostic.severity, Severity::Warning);
}

#[test]
fn three_views_that_span_three_axes_are_complete() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    let mut top = manifest["views"][0].clone();
    top["view_id"] = serde_json::json!("top");
    top["image"]["path"] = serde_json::json!("images/right.png");
    top["image"]["sha256"] = serde_json::json!(common::sha256_of(&dir.join("images/right.png")));
    top["mask"]["path"] = serde_json::json!("masks/right.png");
    top["mask"]["sha256"] = serde_json::json!(common::sha256_of(&dir.join("masks/right.png")));
    top["projection"]["location_m"] = serde_json::json!([0.0, 0.0, 4.0]);
    top["projection"]["up_world"] = serde_json::json!([0.0, 1.0, 0.0]);
    let mut views = manifest["views"].as_array().expect("views").clone();
    views.push(top);
    manifest["views"] = serde_json::Value::Array(views);
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert_eq!(report.completeness.calibrated_views, 3);
    assert_eq!(report.completeness.distinct_directions, 3);
    assert_eq!(report.completeness.axes_covered, vec!["x", "y", "z"]);
    assert_eq!(report.status, ReportStatus::ValidComplete);
}

#[test]
fn the_projection_of_a_reference_view_is_ignored() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    let mut reference = manifest["views"][0].clone();
    reference["view_id"] = serde_json::json!("reference_view");
    reference["kind"] = serde_json::json!("reference");
    let mut views = manifest["views"].as_array().expect("views").clone();
    views.push(reference);
    manifest["views"] = serde_json::Value::Array(views);
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "REFERENCE_PROJECTION_IGNORED"));
    let diagnostic = report
        .diagnostics
        .iter()
        .find(|d| d.code == "REFERENCE_PROJECTION_IGNORED")
        .expect("diagnostic");
    assert_eq!(diagnostic.level, 2);
    assert_eq!(diagnostic.severity, Severity::Warning);
}

#[test]
fn a_superseded_acceptance_is_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["acceptance"] = serde_json::json!([
        {
            "target": "parts:body",
            "status": "accepted",
            "by": "owner",
            "at": "2026-11-07T12:00:00Z"
        },
        {
            "target": "parts:body",
            "status": "needs_revision",
            "by": "owner",
            "at": "2026-11-08T12:00:00Z"
        }
    ]);
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "ACCEPTANCE_SUPERSEDED"));
}

#[test]
fn an_ai_authored_supplement_without_acceptance_is_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["supplements"] = serde_json::json!([
        {
            "supplement_id": "wall",
            "part_id": "body",
            "kind": "thickness",
            "geometry": {"value_m": 0.01},
            "provenance": {"origin": "ai_author"}
        }
    ]);
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "SUPPLEMENT_NOT_ACCEPTED"));
}

#[test]
fn an_accepted_ai_authored_supplement_is_not_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["supplements"] = serde_json::json!([
        {
            "supplement_id": "wall",
            "part_id": "body",
            "kind": "thickness",
            "geometry": {"value_m": 0.01},
            "provenance": {"origin": "ai_author"}
        }
    ]);
    manifest["acceptance"] = serde_json::json!([
        {
            "target": "supplements:wall",
            "status": "accepted",
            "by": "owner",
            "at": "2026-11-07T12:00:00Z"
        }
    ]);
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(!has_code(&report, "SUPPLEMENT_NOT_ACCEPTED"));
}

#[test]
fn an_unknown_without_a_policy_is_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["unknowns"] = serde_json::json!([
        {
            "id": "u_interior",
            "description": "hidden interior"
        }
    ]);
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "UNKNOWN_WITHOUT_POLICY"));
}

#[test]
fn a_revision_directory_that_does_not_match_the_revision_is_reported() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let dir = temp.path().join("r0002");
    copy_tree(&fixture_path("valid_full"), &dir);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "REVISION_DIR_MISMATCH"));
    let diagnostic = report
        .diagnostics
        .iter()
        .find(|d| d.code == "REVISION_DIR_MISMATCH")
        .expect("diagnostic");
    assert_eq!(diagnostic.level, 2);
}

#[test]
fn a_matching_revision_directory_is_not_reported() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let dir = temp.path().join("r0001");
    copy_tree(&fixture_path("valid_full"), &dir);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(!has_code(&report, "REVISION_DIR_MISMATCH"));
    assert_eq!(report.status, ReportStatus::ValidComplete);
}
