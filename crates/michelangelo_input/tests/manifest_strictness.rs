//! Manifest strictness: unknown fields, wrong schema, bad enums, ranges.

mod common;

use common::{copy_fixture, has_code, package_dir, read_manifest, write_manifest};
use michelangelo_input::validate::load_manifest;
use michelangelo_input::{validate_package, ReportStatus, ValidateOptions};

#[test]
fn the_crate_local_fixture_manifest_loads() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("minimal_manifest.json");
    std::fs::copy(&fixture, temp.path().join("package.json")).expect("copy fixture");

    let manifest = load_manifest(temp.path()).expect("manifest loads");
    assert_eq!(manifest.package_id, "crate-local-fixture");
    assert_eq!(manifest.views.len(), 1);

    // The images of the crate local fixture are not committed, so the
    // resources are reported as missing.
    let report = validate_package(temp.path(), &ValidateOptions::default());
    assert!(has_code(&report, "RESOURCE_MISSING"));
}

#[test]
fn the_committed_fixture_manifest_loads() {
    let manifest = load_manifest(&common::fixture_path("valid_full")).expect("manifest loads");
    assert_eq!(manifest.package_id, "stage-a-fixture-full");
    assert_eq!(manifest.revision, 1);
    assert_eq!(manifest.views.len(), 2);
}

#[test]
fn load_manifest_rejects_an_unknown_field() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["unexpected_field"] = serde_json::json!(true);
    write_manifest(&dir, &manifest);

    let error = load_manifest(&dir).expect_err("unknown fields are rejected");
    assert!(
        error.to_string().contains("unexpected_field"),
        "unexpected message: {error}"
    );
}

#[test]
fn load_manifest_reports_a_missing_directory() {
    let error = load_manifest(std::path::Path::new("does/not/exist"))
        .expect_err("missing directories are reported");
    assert!(error.to_string().contains("package root"));
}

#[test]
fn an_unknown_field_is_a_level_one_error() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["unexpected_field"] = serde_json::json!(true);
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "SCHEMA_UNKNOWN_FIELD"));
    assert_eq!(report.status, ReportStatus::Invalid);
    assert!(!report.levels.format_safety.ok);
}

#[test]
fn a_wrong_schema_value_is_rejected() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["schema"] = serde_json::json!("michelangelo.input-package/1");
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "SCHEMA_ENUM"));
    let diagnostic = report
        .diagnostics
        .iter()
        .find(|d| d.code == "SCHEMA_ENUM")
        .expect("diagnostic");
    assert_eq!(diagnostic.address, "schema");
}

#[test]
fn a_bad_enum_value_is_rejected() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["views"][0]["kind"] = serde_json::json!("bogus");
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "SCHEMA_ENUM"));
}

#[test]
fn a_missing_required_field_is_rejected() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    if let Some(object) = manifest.as_object_mut() {
        object.remove("provenance");
    }
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "SCHEMA_TYPE"));
}

#[test]
fn invalid_json_is_reported_as_a_schema_type_error() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    std::fs::write(dir.join("package.json"), "{ this is not json").expect("write");

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "SCHEMA_TYPE"));
    assert_eq!(report.status, ReportStatus::Invalid);
}

#[test]
fn a_missing_manifest_produces_an_invalid_report() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    std::fs::remove_file(dir.join("package.json")).expect("remove");

    let report = validate_package(&dir, &ValidateOptions::default());
    assert_eq!(report.status, ReportStatus::Invalid);
    assert!(has_code(&report, "RESOURCE_MISSING"));
    assert_eq!(report.package_id, "");
    assert_eq!(report.revision, 0);
}

#[test]
fn a_missing_package_directory_produces_an_invalid_report() {
    let report = validate_package(
        std::path::Path::new("does/not/exist"),
        &ValidateOptions::default(),
    );
    assert_eq!(report.status, ReportStatus::Invalid);
    assert!(has_code(&report, "RESOURCE_MISSING"));
}

#[test]
fn a_manifest_above_one_mebibyte_is_rejected() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let filler = "x".repeat(1_048_577);
    std::fs::write(dir.join("package.json"), filler).expect("write");

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "MANIFEST_TOO_LARGE"));
    assert_eq!(report.status, ReportStatus::Invalid);
}

#[test]
fn duplicate_view_ids_are_rejected() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["views"][1]["view_id"] = serde_json::json!("front");
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "ID_DUPLICATE"));
}

#[test]
fn an_invalid_package_id_is_rejected() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["package_id"] = serde_json::json!("Not A Valid Id");
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "ID_INVALID"));
}

#[test]
fn an_invalid_view_id_is_rejected() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["views"][0]["view_id"] = serde_json::json!("Front");
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "ID_INVALID"));
}

#[test]
fn a_revision_out_of_range_is_rejected() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["revision"] = serde_json::json!(0);
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "SCHEMA_RANGE"));
}

#[test]
fn an_empty_view_list_is_rejected() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["views"] = serde_json::json!([]);
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "SCHEMA_RANGE"));
}

#[test]
fn more_than_sixty_four_views_are_rejected() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    let view = manifest["views"][0].clone();
    let mut views = Vec::new();
    for index in 0..65 {
        let mut view = view.clone();
        view["view_id"] = serde_json::json!(format!("view_{index:02}"));
        views.push(view);
    }
    manifest["views"] = serde_json::Value::Array(views);
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "SCHEMA_RANGE"));
}

#[test]
fn a_mask_format_and_foreground_mismatch_is_rejected() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["views"][0]["mask"]["foreground"] = serde_json::json!("alpha");
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "SCHEMA_ENUM"));
}

#[test]
fn an_unknown_completion_rule_is_rejected() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["uncertainty_policy"]["allowed_completion_rules"] = serde_json::json!(["magic"]);
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "SCHEMA_ENUM"));
}

#[test]
fn an_omitted_require_author_acceptance_defaults_to_true() {
    // Cross-language pin: the Rust default and the Python solver default must
    // agree for an `uncertainty_policy` object that omits the key.
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["uncertainty_policy"]
        .as_object_mut()
        .expect("the policy is an object")
        .remove("require_author_acceptance");
    write_manifest(&dir, &manifest);

    let package = load_manifest(&dir).expect("the manifest loads");
    let uncertainty = package
        .uncertainty_policy
        .as_ref()
        .expect("the policy is present");
    assert!(uncertainty.require_author_acceptance);
    let policy = michelangelo_input::PackagePolicy::from_manifest(&package);
    assert!(policy.policy_present);
    assert!(policy.require_author_acceptance);
}

#[test]
fn a_non_distinct_coordinate_system_is_rejected() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["coordinate_system"]["right"] = serde_json::json!("+Z");
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert!(has_code(&report, "SCHEMA_RANGE"));
}
