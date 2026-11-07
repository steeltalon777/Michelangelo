//! Level 1 diagnostics: resource safety, container format, hashes and
//! manifest semantics.

mod common;

use std::fs::{self, File};

use common::{build_png, copy_fixture, has_code, package_dir, read_manifest, write_manifest};
use michelangelo_input::{validate_package, ReportStatus, ValidateOptions};

fn report_for(dir: &std::path::Path) -> michelangelo_input::InputPackageReport {
    validate_package(dir, &ValidateOptions::default())
}

fn sparse_file(path: &std::path::Path, len: u64) {
    let file = File::create(path).expect("create file");
    file.set_len(len).expect("set length");
}

#[test]
fn a_backslash_in_a_path_is_rejected() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["views"][0]["image"]["path"] = serde_json::json!("images\\front.png");
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "PATH_INVALID"));
    assert_eq!(report.status, ReportStatus::Invalid);
}

#[test]
fn a_drive_letter_in_a_path_is_rejected() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["views"][0]["image"]["path"] = serde_json::json!("C:/images/front.png");
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "PATH_INVALID"));
}

#[test]
fn a_nul_byte_in_a_path_is_rejected() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    let mut path = String::from("images/front");
    path.push(char::from(0u8));
    path.push_str(".png");
    manifest["views"][0]["image"]["path"] = serde_json::json!(path);
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "PATH_INVALID"));
}

#[test]
fn an_absolute_path_is_rejected() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["views"][0]["image"]["path"] = serde_json::json!("/etc/hosts");
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "PATH_OUTSIDE_PACKAGE"));
}

#[test]
fn a_parent_traversal_is_rejected_and_the_file_is_never_read() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);

    // A file outside of the package that would fail the PNG check if it was
    // ever opened: the diagnostic must be the traversal, not a PNG error.
    let outside = temp.path().join("outside.png");
    fs::write(&outside, [0xFFu8, 0xD8, 0xFF, 0xE0, 0x00, 0x10]).expect("write outside file");

    let mut manifest = read_manifest(&dir);
    manifest["views"][0]["image"]["path"] = serde_json::json!("../outside.png");
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "PATH_OUTSIDE_PACKAGE"));
    assert!(
        !has_code(&report, "PNG_SIGNATURE"),
        "the file outside of the package must never be read: {:?}",
        common::codes(&report)
    );
    let resource = report
        .resources
        .iter()
        .find(|resource| resource.address == "views[0].image")
        .expect("resource entry");
    assert_eq!(resource.bytes, 0);
    assert!(!resource.verified);
}

#[test]
fn a_missing_resource_is_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    fs::remove_file(dir.join("images/front.png")).expect("remove");

    let report = report_for(&dir);
    assert!(has_code(&report, "RESOURCE_MISSING"));
}

#[test]
fn a_directory_is_not_a_regular_file() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    fs::remove_file(dir.join("images/front.png")).expect("remove");
    fs::create_dir(dir.join("images/front.png")).expect("create dir");

    let report = report_for(&dir);
    assert!(has_code(&report, "RESOURCE_NOT_REGULAR"));
}

#[test]
fn a_resource_above_sixty_four_mebibytes_is_rejected() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    sparse_file(&dir.join("images/front.png"), 64 * 1024 * 1024 + 1);

    let report = report_for(&dir);
    assert!(has_code(&report, "RESOURCE_TOO_LARGE"));
}

#[test]
fn referenced_bytes_above_five_hundred_twelve_mebibytes_are_rejected() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let size = 64 * 1024 * 1024u64;
    for index in 0..9 {
        sparse_file(&dir.join(format!("images/big{index}.png")), size);
    }

    let mut manifest = read_manifest(&dir);
    manifest["views"][0]["image"]["path"] = serde_json::json!("images/big0.png");
    manifest["views"][0]["mask"]["path"] = serde_json::json!("images/big1.png");
    manifest["views"][1]["image"]["path"] = serde_json::json!("images/big2.png");
    manifest["views"][1]["mask"]["path"] = serde_json::json!("images/big3.png");
    let template = manifest["views"][0].clone();
    let mut views = vec![manifest["views"][0].clone(), manifest["views"][1].clone()];
    for index in 0..5 {
        let mut view = template.clone();
        view["view_id"] = serde_json::json!(format!("extra_{index}"));
        view["image"]["path"] = serde_json::json!(format!("images/big{}.png", index + 4));
        view["mask"]["path"] = serde_json::json!(format!("images/big{}.png", index + 4));
        views.push(view);
    }
    manifest["views"] = serde_json::Value::Array(views);
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "TOTAL_BYTES_EXCEEDED"));
}

#[test]
fn a_jpeg_resource_is_reported_as_a_signature_error() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    fs::write(
        dir.join("images/front.png"),
        [0xFFu8, 0xD8, 0xFF, 0xE0, 0x00, 0x10],
    )
    .expect("write");

    let report = report_for(&dir);
    assert!(has_code(&report, "PNG_SIGNATURE"));
}

#[test]
fn a_gray_png_is_not_a_valid_image_resource() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    fs::write(dir.join("images/front.png"), build_png(8, 8, 8, 0, 0)).expect("write");

    let report = report_for(&dir);
    assert!(has_code(&report, "PNG_COLOR_TYPE"));
}

#[test]
fn a_rgb_png_is_not_a_valid_mask_resource() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    fs::write(dir.join("masks/front.png"), build_png(8, 8, 8, 2, 0)).expect("write");

    let report = report_for(&dir);
    assert!(has_code(&report, "PNG_COLOR_TYPE"));
}

#[test]
fn a_truncated_png_is_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let bytes = fs::read(dir.join("images/front.png")).expect("read");
    fs::write(dir.join("images/front.png"), &bytes[..bytes.len() - 6]).expect("write");

    let report = report_for(&dir);
    assert!(has_code(&report, "PNG_TRUNCATED"));
}

#[test]
fn a_wrong_bit_depth_is_reported_as_an_invalid_ihdr() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    fs::write(dir.join("images/front.png"), build_png(8, 8, 16, 2, 0)).expect("write");

    let report = report_for(&dir);
    assert!(has_code(&report, "PNG_IHDR_INVALID"));
}

#[test]
fn dimensions_that_differ_from_the_ihdr_are_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["views"][0]["image"]["width_px"] = serde_json::json!(16);
    manifest["views"][0]["image"]["height_px"] = serde_json::json!(16);
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "PNG_DIMENSION_MISMATCH"));
}

#[test]
fn a_mask_with_other_dimensions_than_the_image_is_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    fs::write(dir.join("masks/front.png"), build_png(4, 4, 8, 0, 0)).expect("write");

    let report = report_for(&dir);
    assert!(has_code(&report, "PNG_DIMENSION_MISMATCH"));
}

#[test]
fn a_tampered_resource_is_reported_as_a_hash_mismatch() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut bytes = fs::read(dir.join("masks/front.png")).expect("read");
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    fs::write(dir.join("masks/front.png"), &bytes).expect("write");

    let report = report_for(&dir);
    assert!(has_code(&report, "HASH_MISMATCH"));
    assert_eq!(report.status, ReportStatus::Invalid);
    let diagnostic = common::diagnostics_with_code(&report, "HASH_MISMATCH")
        .into_iter()
        .next()
        .expect("diagnostic");
    assert_eq!(diagnostic.address, "views[0].mask.sha256");
    assert_eq!(diagnostic.level, 1);
}

#[test]
fn a_hash_mismatch_can_be_downgraded_to_a_warning() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut bytes = fs::read(dir.join("masks/front.png")).expect("read");
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    fs::write(dir.join("masks/front.png"), &bytes).expect("write");

    let options = ValidateOptions {
        require_complete: false,
        verify_hashes: false,
    };
    let report = validate_package(&dir, &options);
    assert!(has_code(&report, "HASH_MISMATCH"));
    let diagnostic = common::diagnostics_with_code(&report, "HASH_MISMATCH")
        .into_iter()
        .next()
        .expect("diagnostic");
    assert_eq!(diagnostic.severity, michelangelo_input::Severity::Warning);
    assert_eq!(diagnostic.level, 1);
    assert_eq!(report.status, ReportStatus::ValidComplete);
    assert!(report.levels.format_safety.ok);
}

#[test]
fn an_uppercase_hash_is_invalid() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["views"][0]["image"]["sha256"] = serde_json::json!("A".repeat(64));
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "HASH_INVALID"));
}

#[test]
fn a_parent_reference_to_an_unknown_part_is_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["parts"][0]["parent"] = serde_json::json!("ghost");
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "REFERENCE_UNKNOWN"));
}

#[test]
fn a_parent_cycle_is_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["parts"] = serde_json::json!([
        {"part_id": "a", "parent": "b"},
        {"part_id": "b", "parent": "a"}
    ]);
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "PARENT_CYCLE"));
}

#[test]
fn a_supplement_of_an_unknown_part_is_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["supplements"] = serde_json::json!([
        {
            "supplement_id": "wall",
            "part_id": "ghost",
            "kind": "thickness",
            "geometry": {"value_m": 0.01},
            "provenance": {"origin": "human"}
        }
    ]);
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "SUPPLEMENT_PART_UNKNOWN"));
}

#[test]
fn an_empty_supplement_geometry_is_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["supplements"] = serde_json::json!([
        {
            "supplement_id": "wall",
            "part_id": "body",
            "kind": "thickness",
            "geometry": {},
            "provenance": {"origin": "human"}
        }
    ]);
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "GEOMETRY_INVALID"));
}

#[test]
fn a_supplement_geometry_without_the_required_number_is_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["supplements"] = serde_json::json!([
        {
            "supplement_id": "wall",
            "part_id": "body",
            "kind": "thickness",
            "geometry": {"material": "steel"},
            "provenance": {"origin": "human"}
        }
    ]);
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "SUPPLEMENT_GEOMETRY_INVALID"));
}

#[test]
fn a_deeply_nested_geometry_is_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    let mut geometry = serde_json::json!({"value_m": 0.01});
    for _ in 0..10 {
        geometry = serde_json::json!({ "nested": geometry });
    }
    manifest["supplements"] = serde_json::json!([
        {
            "supplement_id": "wall",
            "part_id": "body",
            "kind": "thickness",
            "geometry": geometry,
            "provenance": {"origin": "human"}
        }
    ]);
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "GEOMETRY_INVALID"));
}

#[test]
fn a_measurement_without_a_positive_value_is_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["measurements"][0]["value_m"] = serde_json::json!(0.0);
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "MEASUREMENT_INVALID"));
}

#[test]
fn an_acceptance_of_an_unknown_target_is_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["acceptance"] = serde_json::json!([
        {
            "target": "parts:ghost",
            "status": "accepted",
            "by": "owner",
            "at": "2026-11-07T12:00:00Z"
        }
    ]);
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "ACCEPTANCE_TARGET_UNKNOWN"));
}

#[test]
fn a_calibrated_view_without_a_projection_is_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    if let Some(view) = manifest["views"][0].as_object_mut() {
        view.remove("projection");
    }
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "SCHEMA_TYPE"));
}

#[test]
fn an_ortho_scale_of_zero_is_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["views"][0]["projection"]["ortho_scale_m"] = serde_json::json!(0.0);
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "SCHEMA_RANGE"));
}

#[test]
fn an_up_vector_that_is_not_normalized_is_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["views"][0]["projection"]["up_world"] = serde_json::json!([0.0, 0.0, 2.0]);
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "SCHEMA_RANGE"));
}

#[test]
fn a_view_direction_parallel_to_the_up_vector_is_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["views"][0]["projection"]["location_m"] = serde_json::json!([0.0, 0.0, 4.0]);
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "SCHEMA_RANGE"));
}

#[test]
fn a_clip_range_that_is_not_increasing_is_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["views"][0]["projection"]["clip_end_m"] = serde_json::json!(0.01);
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "SCHEMA_RANGE"));
}

#[test]
fn a_lens_shift_out_of_range_is_reported() {
    let temp = copy_fixture("valid_full");
    let dir = package_dir(&temp);
    let mut manifest = read_manifest(&dir);
    manifest["views"][0]["projection"]["shift_x"] = serde_json::json!(11.0);
    write_manifest(&dir, &manifest);

    let report = report_for(&dir);
    assert!(has_code(&report, "SCHEMA_RANGE"));
}
