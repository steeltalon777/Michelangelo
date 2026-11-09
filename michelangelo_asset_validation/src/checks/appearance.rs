//! Appearance and preservation checks: material_presence, texture_integrity,
//! uv_preservation, format_loss_declared. Measurements come from the bytes
//! actually read (OBJ/MTL parsing at fixture level).

use std::collections::{BTreeMap, BTreeSet};

use crate::canonical::sha256_hex;
use crate::check::{CheckId, CheckRecord, CheckStatus, DocRef};
use crate::checks::integrity::{reference_keys, resolve_pinned, PinnedRef};
use crate::checks::parsing::{parse_mtl, parse_obj};
use crate::context::ValidationContext;
use crate::error::ValidationError;
use crate::profile::ValidationProfile;

fn record(
    check: CheckId,
    result: CheckStatus,
    value: Option<f64>,
    detail: String,
    evidence: Vec<DocRef>,
) -> CheckRecord {
    CheckRecord {
        check_id: check,
        result,
        value,
        detail: Some(detail),
        applicability_ref: None,
        evidence,
    }
}

/// Collect the OBJ material references of the bundle and verify that every
/// used material is defined in a dependency MTL inside the closure. Also
/// verifies manifest-declared material artifacts exist (their bytes are
/// already checked by payload_integrity).
pub fn check_material_presence(
    context: &mut ValidationContext,
) -> Result<CheckRecord, ValidationError> {
    let manifest = context.manifest().clone();
    let mut defined: BTreeSet<String> = BTreeSet::new();
    let mut used: BTreeSet<String> = BTreeSet::new();
    let mut parsed_any = false;
    for dep in &manifest.dependencies {
        if !dep.path.ends_with(".mtl") {
            continue;
        }
        let bytes = context.read_bytes(&dep.path)?;
        let data = parse_mtl(&bytes)?;
        defined.extend(data.materials);
        parsed_any = true;
    }
    for artifact in &manifest.artifacts {
        if artifact.content_kind == "material" {
            parsed_any = true;
        }
        if artifact.content_kind == "geometry" && artifact.path.ends_with(".obj") {
            let bytes = context.read_bytes(&artifact.path)?;
            let data = parse_obj(&bytes)?;
            used.extend(data.used_materials);
            parsed_any = true;
        }
    }
    if !parsed_any {
        return Ok(record(
            CheckId::MaterialPresence,
            CheckStatus::Unavailable,
            None,
            "no material data this module can measure (no OBJ/MTL artifacts); a material \
             backend integration is pending"
                .to_string(),
            Vec::new(),
        ));
    }
    let unresolved: Vec<&String> = used.difference(&defined).collect();
    if unresolved.is_empty() {
        Ok(record(
            CheckId::MaterialPresence,
            CheckStatus::Pass,
            Some(defined.len() as f64),
            format!(
                "{} defined materials; every used material resolves inside the closure",
                defined.len()
            ),
            Vec::new(),
        ))
    } else {
        let names: Vec<String> = unresolved.iter().map(|name| name.to_string()).collect();
        Ok(record(
            CheckId::MaterialPresence,
            CheckStatus::Fail,
            None,
            format!(
                "MISSING_MATERIAL: the used materials {:?} are not defined in any dependency",
                names
            ),
            Vec::new(),
        ))
    }
}

/// Every texture referenced by a dependency MTL (`map_*`) and every manifest
/// texture artifact must exist in the closure with the declared digest.
pub fn check_texture_integrity(
    context: &mut ValidationContext,
) -> Result<CheckRecord, ValidationError> {
    let manifest = context.manifest().clone();
    let declared_paths: BTreeSet<&str> = manifest
        .artifacts
        .iter()
        .map(|a| a.path.as_str())
        .chain(manifest.dependencies.iter().map(|d| d.path.as_str()))
        .collect();
    let mut referenced: BTreeSet<String> = BTreeSet::new();
    let mut parsed_any = false;
    for dep in &manifest.dependencies {
        if !dep.path.ends_with(".mtl") {
            continue;
        }
        let bytes = context.read_bytes(&dep.path)?;
        let data = parse_mtl(&bytes)?;
        referenced.extend(data.textures);
        parsed_any = true;
    }
    for artifact in &manifest.artifacts {
        if artifact.content_kind == "texture" {
            parsed_any = true;
        }
    }
    if !parsed_any {
        return Ok(record(
            CheckId::TextureIntegrity,
            CheckStatus::Unavailable,
            None,
            "no texture data this module can measure (no MTL/texture artifacts); a texture \
             backend integration is pending"
                .to_string(),
            Vec::new(),
        ));
    }
    // Textures referenced by MTL must be present in the closure; the bytes of
    // closure members are verified by payload/dependency checks.
    let missing: Vec<&String> = referenced
        .iter()
        .filter(|path| !declared_paths.contains(path.as_str()))
        .collect();
    if missing.is_empty() {
        Ok(record(
            CheckId::TextureIntegrity,
            CheckStatus::Pass,
            Some(referenced.len() as f64),
            format!(
                "{} referenced textures are inside the declared closure",
                referenced.len()
            ),
            Vec::new(),
        ))
    } else {
        let names: Vec<String> = missing.iter().map(|path| path.to_string()).collect();
        Ok(record(
            CheckId::TextureIntegrity,
            CheckStatus::Fail,
            None,
            format!(
                "DEPENDENCY_MISSING: the referenced textures {:?} are outside the declared \
                 dependency closure",
                names
            ),
            Vec::new(),
        ))
    }
}

/// UV preservation: the vt counts of the actually read OBJ artifacts are
/// compared with the pinned `expected_uv_counts` reference (when present) and
/// with the `min_uv_count` threshold (when declared). No OBJ data -> not
/// applicable. Without a pin AND without a threshold the presence of vt lines
/// proves nothing about preservation, so the honest result is `unavailable`
/// (M4) — never `pass` by mere presence.
pub fn check_uv_preservation(
    context: &mut ValidationContext,
    profile: &ValidationProfile,
) -> Result<CheckRecord, ValidationError> {
    let manifest = context.manifest().clone();
    let mut measured: BTreeMap<String, usize> = BTreeMap::new();
    for artifact in &manifest.artifacts {
        if artifact.content_kind == "geometry" && artifact.path.ends_with(".obj") {
            let bytes = context.read_bytes(&artifact.path)?;
            let data = parse_obj(&bytes)?;
            measured.insert(artifact.path.clone(), data.uvs);
        }
    }
    if measured.is_empty() {
        return Ok(record(
            CheckId::UvPreservation,
            CheckStatus::Unavailable,
            None,
            "no OBJ geometry with UV data this module can measure; a UV backend integration \
             is pending"
                .to_string(),
            Vec::new(),
        ));
    }
    let mut failures = Vec::new();
    let mut pinned_total = 0_usize;
    let mut has_pin = false;
    match resolve_pinned(context, profile, reference_keys::EXPECTED_UV_COUNTS)? {
        PinnedRef::Present(value, _) => {
            has_pin = true;
            match value.get("counts").and_then(|v| v.as_array()) {
                Some(entries) => {
                    for entry in entries {
                        let path = entry
                            .get("path")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        let expected =
                            entry.get("vt").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        pinned_total += expected;
                        match measured.get(path) {
                            Some(actual) if *actual != expected => {
                                failures.push(format!(
                                    "UV_CHANGED: the artifact '{}' has {} vt lines, the pinned \
                                     expectation is {}",
                                    path, actual, expected
                                ));
                            }
                            Some(_) => {}
                            None => {
                                failures.push(format!(
                                    "UV_CHANGED: the pinned UV expectation refers to '{}' which \
                                     is not a measurable OBJ artifact",
                                    path
                                ));
                            }
                        }
                    }
                }
                None => failures.push(
                    "UV_PIN_INVALID: the expected_uv_counts pin carries no 'counts' array"
                        .to_string(),
                ),
            }
        }
        PinnedRef::Changed(message) => {
            has_pin = true;
            failures.push(format!("UV_CHANGED: {message}"));
        }
        PinnedRef::Missing => {
            // No pin: the threshold below is the only preservation evidence.
        }
    }
    let total: usize = measured.values().sum();
    let threshold = profile.threshold(CheckId::UvPreservation, "min_uv_count");
    if let Some(threshold) = threshold {
        if !threshold.comparison.holds(total as f64, threshold.value) {
            failures.push(format!(
                "UV_MISSING: the bundle has {} vt lines in total, below the threshold {} {}",
                total,
                threshold.comparison.as_str(),
                threshold.value
            ));
        }
    }
    if failures.is_empty() && !has_pin && threshold.is_none() {
        return Ok(record(
            CheckId::UvPreservation,
            CheckStatus::Unavailable,
            Some(total as f64),
            "no expected_uv_counts pin and no min_uv_count threshold; the presence of vt lines \
             is not preservation evidence"
                .to_string(),
            Vec::new(),
        ));
    }
    if failures.is_empty() {
        Ok(record(
            CheckId::UvPreservation,
            CheckStatus::Pass,
            Some(total as f64),
            format!(
                "{} vt lines measured; pinned expectation total {}",
                total, pinned_total
            ),
            Vec::new(),
        ))
    } else {
        Ok(record(
            CheckId::UvPreservation,
            CheckStatus::Fail,
            Some(total as f64),
            failures.join("; "),
            Vec::new(),
        ))
    }
}

/// Format loss declaration (passthrough): every declared conversion must
/// carry declared losses; the count of losses is recorded as the value.
pub fn check_format_loss_declared(
    context: &mut ValidationContext,
) -> Result<CheckRecord, ValidationError> {
    let manifest = context.manifest().clone();
    let conversions = manifest.conversions.len();
    let losses = manifest.declared_losses.len();
    if conversions == 0 {
        return Ok(record(
            CheckId::FormatLossDeclared,
            CheckStatus::Pass,
            Some(0.0),
            "no conversions declared; no format losses to declare".to_string(),
            Vec::new(),
        ));
    }
    if losses == 0 {
        return Ok(record(
            CheckId::FormatLossDeclared,
            CheckStatus::Fail,
            None,
            "LOSS_UNDECLARED: the bundle declares conversions but no declared_losses; \
             undeclared loss is not allowed (ADR 0005 §8.3)"
                .to_string(),
            Vec::new(),
        ));
    }
    let kinds: Vec<String> = manifest
        .declared_losses
        .iter()
        .map(|loss| loss.kind.clone())
        .collect();
    Ok(record(
        CheckId::FormatLossDeclared,
        CheckStatus::Pass,
        Some(losses as f64),
        format!("{conversions} conversions with declared losses {:?}", kinds),
        Vec::new(),
    ))
}

/// Hash helper retained for future evidence attachments.
#[allow(dead_code)]
fn _unused(bytes: &[u8]) -> String {
    sha256_hex(bytes)
}
