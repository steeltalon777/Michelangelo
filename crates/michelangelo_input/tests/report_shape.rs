//! Report shape, determinism and resource reporting.

mod common;

use common::{copy_fixture, fixture_path, has_code, package_dir, read_manifest, write_manifest};
use michelangelo_input::{validate_package, ReportStatus, ValidateOptions};

#[test]
fn the_report_has_exactly_the_contract_keys() {
    let dir = fixture_path("valid_full");
    let report = validate_package(&dir, &ValidateOptions::default());
    let value = serde_json::to_value(&report).expect("serialize report");

    let object = value.as_object().expect("report object");
    let mut keys: Vec<&str> = object.keys().map(|key| key.as_str()).collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "completeness",
            "diagnostics",
            "levels",
            "package_id",
            "resources",
            "revision",
            "schema",
            "status",
        ]
    );
    assert_eq!(
        object["schema"],
        serde_json::json!("michelangelo.input-package-report/0")
    );
    assert_eq!(
        object["package_id"],
        serde_json::json!("stage-a-fixture-full")
    );
    assert_eq!(object["revision"], serde_json::json!(1));
    assert_eq!(object["status"], serde_json::json!("valid_complete"));
}

#[test]
fn the_levels_and_completeness_blocks_have_the_contract_shape() {
    let dir = fixture_path("valid_full");
    let report = validate_package(&dir, &ValidateOptions::default());
    let value = serde_json::to_value(&report).expect("serialize report");

    let levels = value["levels"].as_object().expect("levels object");
    let mut level_keys: Vec<&str> = levels.keys().map(|key| key.as_str()).collect();
    level_keys.sort();
    assert_eq!(level_keys, vec!["format_safety", "sufficiency"]);

    let format_safety = levels["format_safety"].as_object().expect("format_safety");
    let mut summary_keys: Vec<&str> = format_safety.keys().map(|key| key.as_str()).collect();
    summary_keys.sort();
    assert_eq!(summary_keys, vec!["errors", "ok", "warnings"]);

    let completeness = value["completeness"].as_object().expect("completeness");
    let mut completeness_keys: Vec<&str> = completeness.keys().map(|key| key.as_str()).collect();
    completeness_keys.sort();
    assert_eq!(
        completeness_keys,
        vec![
            "axes_covered",
            "calibrated_views",
            "distinct_directions",
            "missing",
            "status",
        ]
    );
}

#[test]
fn a_diagnostic_has_the_contract_shape() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["revision"] = serde_json::json!(0);
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    let value = serde_json::to_value(&report).expect("serialize report");
    let diagnostic = &value["diagnostics"][0];
    let object = diagnostic.as_object().expect("diagnostic object");
    let mut keys: Vec<&str> = object.keys().map(|key| key.as_str()).collect();
    keys.sort();
    assert_eq!(
        keys,
        vec!["address", "code", "level", "message", "severity"]
    );
    assert_eq!(diagnostic["level"], serde_json::json!(1));
    assert_eq!(diagnostic["code"], serde_json::json!("SCHEMA_RANGE"));
    assert_eq!(diagnostic["severity"], serde_json::json!("error"));
    assert_eq!(diagnostic["address"], serde_json::json!("revision"));
    assert!(diagnostic["message"].as_str().is_some());
}

#[test]
fn a_resource_entry_has_the_contract_shape() {
    let dir = fixture_path("valid_full");
    let report = validate_package(&dir, &ValidateOptions::default());
    let value = serde_json::to_value(&report).expect("serialize report");
    let resource = &value["resources"][0];
    let object = resource.as_object().expect("resource object");
    let mut keys: Vec<&str> = object.keys().map(|key| key.as_str()).collect();
    keys.sort();
    assert_eq!(keys, vec!["address", "bytes", "path", "sha256", "verified"]);

    assert_eq!(report.resources.len(), 4);
    assert!(report.resources.iter().all(|resource| resource.verified));
    assert!(report
        .resources
        .iter()
        .all(|resource| resource.sha256.len() == 64));
    assert!(report.resources.iter().all(|resource| resource.bytes > 0));
}

#[test]
fn diagnostics_are_sorted_by_level_address_and_code() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["revision"] = serde_json::json!(0);
    manifest["package_id"] = serde_json::json!("Bad Id");
    manifest["views"][0]["image"]["path"] = serde_json::json!("images\\front.png");
    manifest["views"][1]["mask"]["sha256"] = serde_json::json!("00");
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    let keys: Vec<(u8, &str, &str)> = report
        .diagnostics
        .iter()
        .map(|diagnostic| {
            (
                diagnostic.level,
                diagnostic.address.as_str(),
                diagnostic.code.as_str(),
            )
        })
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
    assert!(report.diagnostics.len() >= 4);
}

#[test]
fn the_report_is_deterministic() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["revision"] = serde_json::json!(0);
    manifest["views"][1]["image"]["sha256"] = serde_json::json!("00");
    write_manifest(&dir, &manifest);

    let first = validate_package(&dir, &ValidateOptions::default());
    let second = validate_package(&dir, &ValidateOptions::default());
    assert_eq!(first, second);
}

#[test]
fn an_invalid_package_still_reports_its_resources() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["views"][0]["image"]["sha256"] = serde_json::json!("0".repeat(64));
    write_manifest(&dir, &manifest);

    let report = validate_package(&dir, &ValidateOptions::default());
    assert_eq!(report.status, ReportStatus::Invalid);
    assert!(has_code(&report, "HASH_MISMATCH"));
    assert_eq!(report.resources.len(), 4);
}

#[test]
fn the_report_can_be_deserialized_again() {
    let dir = fixture_path("valid_partial");
    let report = validate_package(&dir, &ValidateOptions::default());
    let text = serde_json::to_string(&report).expect("serialize");
    let decoded: michelangelo_input::InputPackageReport =
        serde_json::from_str(&text).expect("deserialize");
    assert_eq!(decoded, report);
    assert_eq!(decoded.status, ReportStatus::ValidPartial);
}
