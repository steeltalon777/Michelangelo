//! A local PoC harness: builds the family-static fixture workspace in a
//! disposable OS temp directory, evaluates under a pinned profile,
//! registers the report in the no-clobber registry, verifies it, and prints
//! the artifact locations. Used for the local smoke and for cross-validating
//! the emitted report against the GLM-1 draft JSON Schema.
//!
//! Run: cargo run --offline --example dump_validation_report

use std::path::PathBuf;

use michelangelo_asset_validation::registry::{
    verify_registered_report, write_and_register, Reservation,
};
use michelangelo_asset_validation::testing::{
    build_family_workspace, evaluate_fixture, TIMESTAMP,
};

fn main() {
    // One disposable validation workspace per candidate; the report registry
    // lives inside the same root (the input digests are relative to it).
    let root = std::env::temp_dir().join("michelangelo-asset-validation-smoke");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp workspace");

    let (workspace, profile) = build_family_workspace(&root, "cand-house-smoke");

    println!("workspace: {}", workspace.root.display());
    println!("manifest sha256: {}", workspace.manifest_sha256);
    println!("asset_digest: {}", workspace.asset_digest);

    let reservation =
        Reservation::reserve(&root, "vr-smoke-0001", &workspace.manifest_sha256, TIMESTAMP)
            .expect("reservation");
    let (report, report_digest) =
        evaluate_fixture(&workspace, &profile, "vr-smoke-0001").expect("evaluation");
    println!("report verdict: {}", report.verdict.as_str());
    println!("report digest: {report_digest}");
    for record in &report.checks {
        println!(
            "  check {:<26} {:<12} {}",
            record.check_id.as_str(),
            match record.result {
                michelangelo_asset_validation::check::CheckStatus::Pass => "pass",
                michelangelo_asset_validation::check::CheckStatus::Fail => "fail",
                michelangelo_asset_validation::check::CheckStatus::Unavailable => "unavailable",
                michelangelo_asset_validation::check::CheckStatus::NotRequired => "not_required",
            },
            record.detail.as_deref().unwrap_or_default()
        );
    }

    let entry = write_and_register(&root, &reservation, &report).expect("registration");
    println!("registered: {} ({})", entry.report_id, entry.report_path);
    println!(
        "registry: {}",
        michelangelo_asset_validation::registry::registry_path(&root).display()
    );

    let outcome = verify_registered_report(&root, "vr-smoke-0001", Some(&profile)).expect("verify");
    println!(
        "verified: digest {} verdict {}",
        outcome.report_sha256,
        outcome.verdict.as_str()
    );

    // A second evaluation (e.g. after a profile revision) never overwrites
    // the first registered report.
    let second_reservation =
        Reservation::reserve(&root, "vr-smoke-0002", &workspace.manifest_sha256, TIMESTAMP)
            .expect("reservation 2");
    let mut profile_v2 = profile.clone();
    profile_v2.revision = 2;
    let (report_v2, _) =
        evaluate_fixture(&workspace, &profile_v2, "vr-smoke-0002").expect("evaluation 2");
    let _ = write_and_register(&root, &second_reservation, &report_v2).expect("registration 2");
    println!("second report registered alongside the first (no-clobber)");

    let report_file = root.join(&entry.report_path);
    println!("report file: {}", report_file.display());

    // Dump the pinned profile for schema cross-validation.
    let profile_file = root.join("validation/pinned-profile.json");
    std::fs::create_dir_all(profile_file.parent().expect("parent")).expect("dir");
    std::fs::write(
        &profile_file,
        serde_json::to_vec_pretty(&profile).expect("profile json"),
    )
    .expect("profile write");
    println!("profile file: {}", profile_file.display());
    println!("SMOKE_OK");
}
