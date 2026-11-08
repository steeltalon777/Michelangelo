//! Geometry checks: budget_geometry, protected_roles (effective transforms),
//! connector_compatibility (module/socket fit), minimum_clearance,
//! maximum_dimension. All measurements come from the bytes actually read.

use std::collections::BTreeMap;

use crate::check::{CheckId, CheckRecord, CheckStatus, DocRef};
use crate::checks::parsing::{
    parse_layout, parse_obj, world_transforms, BBox, Transform,
};
use crate::checks::integrity::{resolve_pinned, PinnedRef, reference_keys};
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

/// Sum of evaluated triangles over all parseable geometry artifacts compared
/// with the `max_evaluated_triangles` threshold. A geometry artifact this
/// module cannot count (no OBJ parser) makes the check unavailable — never a
/// silent pass.
pub fn check_budget_geometry(
    context: &mut ValidationContext,
    profile: &ValidationProfile,
) -> Result<CheckRecord, ValidationError> {
    let manifest = context.manifest().clone();
    let mut total = 0_usize;
    let mut uncounted = Vec::new();
    for artifact in &manifest.artifacts {
        if artifact.content_kind != "geometry" {
            continue;
        }
        if artifact.path.ends_with(".obj") {
            let bytes = context.read_bytes(&artifact.path)?;
            let data = parse_obj(&bytes)?;
            total += data.triangles;
        } else {
            uncounted.push(artifact.path.clone());
        }
    }
    if !uncounted.is_empty() {
        return Ok(record(
            CheckId::BudgetGeometry,
            CheckStatus::Unavailable,
            None,
            format!(
                "the geometry artifacts {:?} cannot be counted by this module; a counting \
                 backend integration is pending",
                uncounted
            ),
            Vec::new(),
        ));
    }
    let threshold = profile.threshold(CheckId::BudgetGeometry, "max_evaluated_triangles");
    let Some(threshold) = threshold else {
        return Ok(record(
            CheckId::BudgetGeometry,
            CheckStatus::Unavailable,
            Some(total as f64),
            "the profile declares no max_evaluated_triangles threshold; the budget cannot \
             be evaluated"
                .to_string(),
            Vec::new(),
        ));
    };
    let holds = threshold.comparison.holds(total as f64, threshold.value);
    let detail = format!(
        "measured {total} triangles; limit {} {}",
        threshold.comparison.as_str(),
        threshold.value
    );
    if holds {
        Ok(record(
            CheckId::BudgetGeometry,
            CheckStatus::Pass,
            Some(total as f64),
            detail,
            Vec::new(),
        ))
    } else {
        Ok(record(
            CheckId::BudgetGeometry,
            CheckStatus::Fail,
            Some(total as f64),
            format!("BUDGET_EXCEEDED: {detail}"),
            Vec::new(),
        ))
    }
}

/// Protected roles: for every protected component the mesh bytes digest and
/// the EFFECTIVE world transform must match the pinned expectations. An
/// allowed parent transform that indirectly moves a protected child fails
/// even though the child mesh bytes are unchanged (ADR §11.4, fixture F9).
pub fn check_protected_roles(
    context: &mut ValidationContext,
    profile: &ValidationProfile,
) -> Result<CheckRecord, ValidationError> {
    let manifest = context.manifest().clone();
    let layout_artifacts = manifest
        .artifacts
        .iter()
        .filter(|a| a.content_kind == "scene_layout")
        .cloned()
        .collect::<Vec<_>>();
    if layout_artifacts.len() != 1 {
        return Ok(record(
            CheckId::ProtectedRoles,
            CheckStatus::Unavailable,
            None,
            format!(
                "expected exactly one scene_layout artifact, found {}",
                layout_artifacts.len()
            ),
            Vec::new(),
        ));
    }
    let layout_path = layout_artifacts[0].path.clone();
    let bytes = context.read_bytes(&layout_path)?;
    let layout = match parse_layout(&bytes) {
        Ok(layout) => layout,
        Err(err) => {
            return Ok(record(
                CheckId::ProtectedRoles,
                CheckStatus::Unavailable,
                None,
                format!("the scene_layout cannot be parsed: {}", err.message()),
                Vec::new(),
            ));
        }
    };
    let world = match world_transforms(&layout) {
        Ok(world) => world,
        Err(err) => {
            return Ok(record(
                CheckId::ProtectedRoles,
                CheckStatus::Unavailable,
                None,
                format!("the scene_layout is invalid: {}", err.message()),
                Vec::new(),
            ));
        }
    };
    let artifact_by_id: BTreeMap<&str, &crate::context::ArtifactEntry> = manifest
        .artifacts
        .iter()
        .map(|a| (a.artifact_id.as_str(), a))
        .collect();

    let pinned = match resolve_pinned(context, profile, reference_keys::PROTECTED_SCENE_LAYOUT)? {
        PinnedRef::Present(value, _) => value,
        PinnedRef::Changed(message) => {
            return Ok(record(
                CheckId::ProtectedRoles,
                CheckStatus::Fail,
                None,
                format!("PROTECTED_CHANGED: {message}"),
                Vec::new(),
            ));
        }
        PinnedRef::Missing => {
            return Ok(record(
                CheckId::ProtectedRoles,
                CheckStatus::Unavailable,
                None,
                "the profile does not pin the protected_scene_layout reference; protected \
                 components cannot be verified"
                    .to_string(),
                Vec::new(),
            ));
        }
    };

    let mut failures = Vec::new();
    let mut checked = 0_usize;
    let mut evidence = Vec::new();
    for component in &layout.components {
        if !component.protected {
            continue;
        }
        let entry = pinned
            .get("components")
            .and_then(|list| list.as_array())
            .and_then(|list| {
                list.iter().find(|entry| {
                    entry.get("component_id").and_then(|v| v.as_str()) == Some(&component.component_id)
                })
            });
        let Some(entry) = entry else {
            failures.push(format!(
                "UNPROTECTED: the protected component '{}' has no pinned expectation; \
                 protection cannot be confirmed",
                component.component_id
            ));
            continue;
        };
        checked += 1;

        // Mesh bytes: the protected artifact must hash to the pinned value.
        let pinned_sha = entry.get("artifact_sha256").and_then(|v| v.as_str()).unwrap_or_default();
        if let Some(artifact) = artifact_by_id.get(component.artifact.as_str()) {
            let inputs = context.input_digests();
            match inputs.get(&artifact.path) {
                Some(actual) => {
                    if actual != pinned_sha {
                        failures.push(format!(
                            "PROTECTED_CHANGED: the mesh bytes of the protected component '{}' \
                             changed (pinned {}, actual {})",
                            component.component_id, pinned_sha, actual
                        ));
                    }
                }
                None => {
                    failures.push(format!(
                        "UNAVAILABLE: the artifact '{}' of the protected component '{}' was \
                         not read",
                        artifact.path, component.component_id
                    ));
                }
            }
        } else {
            failures.push(format!(
                "PROTECTED_CHANGED: the artifact '{}' of the protected component '{}' is not \
                 in the manifest",
                component.artifact, component.component_id
            ));
        }

        // Effective world transform: pinned expectation must match the
        // composed transform (parent chain included).
        let expected: Option<Transform> = entry
            .get("world_transform")
            .and_then(|value| serde_json::from_value(value.clone()).ok());
        let actual = world.get(&component.component_id);
        match (expected, actual) {
            (Some(expected), Some(actual)) => {
                if !crate::checks::parsing::transforms_equal(&expected, actual) {
                    failures.push(format!(
                        "PROTECTED_CHANGED: the effective world transform of the protected \
                         component '{}' differs from the pinned expectation; an allowed \
                         dependency (parent transform) moved protected data",
                        component.component_id
                    ));
                }
            }
            (None, _) => {
                failures.push(format!(
                    "UNPROTECTED: the pinned expectation of '{}' has no world_transform",
                    component.component_id
                ));
            }
            (_, None) => {
                failures.push(format!(
                    "UNAVAILABLE: the world transform of '{}' could not be composed",
                    component.component_id
                ));
            }
        }
        evidence.push(DocRef {
            path: layout_path.clone(),
            sha256: context
                .input_digests()
                .get(&layout_path)
                .cloned()
                .unwrap_or_default(),
        });
    }

    if failures.is_empty() {
        Ok(record(
            CheckId::ProtectedRoles,
            CheckStatus::Pass,
            Some(checked as f64),
            format!(
                "{checked} protected components match the pinned bytes and effective \
                 transforms"
            ),
            evidence,
        ))
    } else {
        Ok(record(
            CheckId::ProtectedRoles,
            CheckStatus::Fail,
            None,
            failures.join("; "),
            evidence,
        ))
    }
}

/// Connector compatibility (V1 module/socket fit): each pinned pair must
/// exist in the sockets artifact with the pinned kinds, and the world
/// distance between the socket positions must satisfy the
/// `connector_tolerance_m` threshold.
pub fn check_connector_compatibility(
    context: &mut ValidationContext,
    profile: &ValidationProfile,
) -> Result<CheckRecord, ValidationError> {
    let connector_profile = match resolve_pinned(context, profile, reference_keys::CONNECTOR_PROFILE)? {
        PinnedRef::Present(value, _) => value,
        PinnedRef::Changed(message) => {
            return Ok(record(
                CheckId::ConnectorCompatibility,
                CheckStatus::Fail,
                None,
                format!("CONNECTOR_PROFILE_CHANGED: {message}"),
                Vec::new(),
            ));
        }
        PinnedRef::Missing => {
            return Ok(record(
                CheckId::ConnectorCompatibility,
                CheckStatus::Unavailable,
                None,
                "the profile does not pin the connector_profile reference"
                    .to_string(),
                Vec::new(),
            ));
        }
    };
    let parsed: crate::checks::parsing::ConnectorProfile =
        match serde_json::from_value(connector_profile) {
            Ok(value) => value,
            Err(err) => {
                return Ok(record(
                    CheckId::ConnectorCompatibility,
                    CheckStatus::Unavailable,
                    None,
                    format!("the connector profile reference is not usable: {err}"),
                    Vec::new(),
                ));
            }
        };

    let manifest = context.manifest().clone();
    let sockets_artifacts = manifest
        .artifacts
        .iter()
        .filter(|a| a.content_kind == "metadata" && a.path.ends_with("sockets.json"))
        .cloned()
        .collect::<Vec<_>>();
    if sockets_artifacts.len() != 1 {
        return Ok(record(
            CheckId::ConnectorCompatibility,
            CheckStatus::Unavailable,
            None,
            format!(
                "expected exactly one sockets metadata artifact, found {}",
                sockets_artifacts.len()
            ),
            Vec::new(),
        ));
    }
    let sockets_path = sockets_artifacts[0].path.clone();
    let sockets_bytes = context.read_bytes(&sockets_path)?;
    let sockets: crate::checks::parsing::SocketsDoc = match serde_json::from_slice(&sockets_bytes) {
        Ok(value) => value,
        Err(err) => {
            return Ok(record(
                CheckId::ConnectorCompatibility,
                CheckStatus::Unavailable,
                None,
                format!("the sockets artifact is not usable: {err}"),
                Vec::new(),
            ));
        }
    };

    // World transforms of components (for socket positions).
    let layout_artifact = manifest
        .artifacts
        .iter()
        .find(|a| a.content_kind == "scene_layout")
        .cloned();
    let world: BTreeMap<String, Transform> = match layout_artifact {
        Some(layout_artifact) => {
            let bytes = context.read_bytes(&layout_artifact.path)?;
            match parse_layout(&bytes).and_then(|layout| world_transforms(&layout)) {
                Ok(world) => world,
                Err(err) => {
                    return Ok(record(
                        CheckId::ConnectorCompatibility,
                        CheckStatus::Unavailable,
                        None,
                        format!("the scene_layout is unusable for socket positions: {}", err.message()),
                        Vec::new(),
                    ));
                }
            }
        }
        None => {
            return Ok(record(
                CheckId::ConnectorCompatibility,
                CheckStatus::Unavailable,
                None,
                "the bundle has no scene_layout artifact; socket positions cannot be \
                 resolved"
                    .to_string(),
                Vec::new(),
            ));
        }
    };

    let by_id: BTreeMap<&str, &crate::checks::parsing::SocketDef> = sockets
        .sockets
        .iter()
        .map(|socket| (socket.socket_id.as_str(), socket))
        .collect();
    let tolerance = profile.threshold(CheckId::ConnectorCompatibility, "connector_tolerance_m");
    let Some(tolerance) = tolerance else {
        return Ok(record(
            CheckId::ConnectorCompatibility,
            CheckStatus::Unavailable,
            None,
            "the profile declares no connector_tolerance_m threshold".to_string(),
            Vec::new(),
        ));
    };

    let mut failures = Vec::new();
    for pair in &parsed.pairs {
        let socket_a = by_id.get(pair.a.as_str());
        let socket_b = by_id.get(pair.b.as_str());
        let (Some(socket_a), Some(socket_b)) = (socket_a, socket_b) else {
            failures.push(format!(
                "CONNECTOR_MISSING: the pinned pair '{} <-> {}' has no socket in the bundle",
                pair.a, pair.b
            ));
            continue;
        };
        if socket_a.kind != pair.a_kind {
            failures.push(format!(
                "CONNECTOR_INCOMPATIBLE: the socket '{}' is kind '{}' but the profile pins '{}'",
                pair.a, socket_a.kind, pair.a_kind
            ));
        }
        if socket_b.kind != pair.b_kind {
            failures.push(format!(
                "CONNECTOR_INCOMPATIBLE: the socket '{}' is kind '{}' but the profile pins '{}'",
                pair.b, socket_b.kind, pair.b_kind
            ));
        }
        let position_a = match world.get(&socket_a.component) {
            Some(transform) => crate::checks::parsing::apply(transform, socket_a.position),
            None => {
                failures.push(format!(
                    "CONNECTOR_MISSING: the socket '{}' refers to unknown component '{}'",
                    pair.a, socket_a.component
                ));
                continue;
            }
        };
        let position_b = match world.get(&socket_b.component) {
            Some(transform) => crate::checks::parsing::apply(transform, socket_b.position),
            None => {
                failures.push(format!(
                    "CONNECTOR_MISSING: the socket '{}' refers to unknown component '{}'",
                    pair.b, socket_b.component
                ));
                continue;
            }
        };
        let gap = crate::checks::parsing::distance(position_a, position_b);
        let holds = tolerance.comparison.holds(gap, tolerance.value);
        if !holds {
            failures.push(format!(
                "CONNECTOR_INCOMPATIBLE: the pinned pair '{} <-> {}' has world distance {:.6} \
                 which violates the tolerance {} {}",
                pair.a,
                pair.b,
                gap,
                tolerance.comparison.as_str(),
                tolerance.value
            ));
        }
    }
    if failures.is_empty() {
        Ok(record(
            CheckId::ConnectorCompatibility,
            CheckStatus::Pass,
            Some(parsed.pairs.len() as f64),
            format!("{} pinned connector pairs fit within tolerance", parsed.pairs.len()),
            Vec::new(),
        ))
    } else {
        Ok(record(
            CheckId::ConnectorCompatibility,
            CheckStatus::Fail,
            None,
            failures.join("; "),
            Vec::new(),
        ))
    }
}

/// The whole-assembly world bounding box extents per axis compared with the
/// `max_extent_<axis>_m` thresholds.
pub fn check_maximum_dimension(
    context: &mut ValidationContext,
    profile: &ValidationProfile,
) -> Result<CheckRecord, ValidationError> {
    let manifest = context.manifest().clone();
    let layout_artifact = match manifest
        .artifacts
        .iter()
        .find(|a| a.content_kind == "scene_layout")
        .cloned()
    {
        Some(artifact) => artifact,
        None => {
            return Ok(record(
                CheckId::MaximumDimension,
                CheckStatus::Unavailable,
                None,
                "the bundle has no scene_layout artifact; dimensions cannot be measured"
                    .to_string(),
                Vec::new(),
            ));
        }
    };
    let layout_bytes = context.read_bytes(&layout_artifact.path)?;
    let layout = match parse_layout(&layout_bytes) {
        Ok(layout) => layout,
        Err(err) => {
            return Ok(record(
                CheckId::MaximumDimension,
                CheckStatus::Unavailable,
                None,
                format!("the scene_layout cannot be parsed: {}", err.message()),
                Vec::new(),
            ));
        }
    };
    let world = match world_transforms(&layout) {
        Ok(world) => world,
        Err(err) => {
            return Ok(record(
                CheckId::MaximumDimension,
                CheckStatus::Unavailable,
                None,
                format!("the scene_layout is invalid: {}", err.message()),
                Vec::new(),
            ));
        }
    };
    let artifact_by_id: BTreeMap<&str, &crate::context::ArtifactEntry> = manifest
        .artifacts
        .iter()
        .map(|a| (a.artifact_id.as_str(), a))
        .collect();
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    let mut uncounted = Vec::new();
    let mut measured_any = false;
    for component in &layout.components {
        let Some(artifact) = artifact_by_id.get(component.artifact.as_str()) else {
            uncounted.push(component.artifact.clone());
            continue;
        };
        if !artifact.path.ends_with(".obj") {
            uncounted.push(artifact.path.clone());
            continue;
        }
        let bytes = context.read_bytes(&artifact.path)?;
        let data = parse_obj(&bytes)?;
        if let Some(bbox) = BBox::of_transformed(world.get(&component.component_id).unwrap_or(&crate::checks::parsing::IDENTITY), &data.vertices) {
            for axis in 0..3 {
                min[axis] = min[axis].min(bbox.min[axis]);
                max[axis] = max[axis].max(bbox.max[axis]);
            }
            measured_any = true;
        }
    }
    if !uncounted.is_empty() || !measured_any {
        return Ok(record(
            CheckId::MaximumDimension,
            CheckStatus::Unavailable,
            None,
            format!(
                "the geometry of {:?} cannot be measured by this module; a measuring backend \
                 integration is pending",
                uncounted
            ),
            Vec::new(),
        ));
    }
    let mut failures = Vec::new();
    let mut measured = 0_usize;
    for (axis, metric) in [
        (0_usize, "max_extent_x_m"),
        (1, "max_extent_y_m"),
        (2, "max_extent_z_m"),
    ] {
        if let Some(threshold) = profile.threshold(CheckId::MaximumDimension, metric) {
            let extent = max[axis] - min[axis];
            measured += 1;
            if !threshold.comparison.holds(extent, threshold.value) {
                failures.push(format!(
                    "DIMENSION_EXCEEDED: the {metric} extent is {:.6}, limit {} {}",
                    extent,
                    threshold.comparison.as_str(),
                    threshold.value
                ));
            }
        }
    }
    if measured == 0 {
        return Ok(record(
            CheckId::MaximumDimension,
            CheckStatus::Unavailable,
            None,
            "the profile declares no max_extent_*_m thresholds".to_string(),
            Vec::new(),
        ));
    }
    if failures.is_empty() {
        Ok(record(
            CheckId::MaximumDimension,
            CheckStatus::Pass,
            Some(max[2] - min[2]),
            format!("{measured} axis extents measured within thresholds"),
            Vec::new(),
        ))
    } else {
        Ok(record(
            CheckId::MaximumDimension,
            CheckStatus::Fail,
            None,
            failures.join("; "),
            Vec::new(),
        ))
    }
}

/// The minimum separation between the entrance-role components and every
/// other component (provisional bbox metric) compared with the
/// `min_clearance_m` threshold (ge).
pub fn check_minimum_clearance(
    context: &mut ValidationContext,
    profile: &ValidationProfile,
) -> Result<CheckRecord, ValidationError> {
    let Some(threshold) = profile.threshold(CheckId::MinimumClearance, "min_clearance_m") else {
        return Ok(record(
            CheckId::MinimumClearance,
            CheckStatus::Unavailable,
            None,
            "the profile declares no min_clearance_m threshold".to_string(),
            Vec::new(),
        ));
    };
    let manifest = context.manifest().clone();
    let layout_artifact = match manifest
        .artifacts
        .iter()
        .find(|a| a.content_kind == "scene_layout")
        .cloned()
    {
        Some(artifact) => artifact,
        None => {
            return Ok(record(
                CheckId::MinimumClearance,
                CheckStatus::Unavailable,
                None,
                "the bundle has no scene_layout artifact; clearance cannot be measured"
                    .to_string(),
                Vec::new(),
            ));
        }
    };
    let layout_bytes = context.read_bytes(&layout_artifact.path)?;
    let layout = match parse_layout(&layout_bytes) {
        Ok(layout) => layout,
        Err(err) => {
            return Ok(record(
                CheckId::MinimumClearance,
                CheckStatus::Unavailable,
                None,
                format!("the scene_layout cannot be parsed: {}", err.message()),
                Vec::new(),
            ));
        }
    };
    let world = match world_transforms(&layout) {
        Ok(world) => world,
        Err(err) => {
            return Ok(record(
                CheckId::MinimumClearance,
                CheckStatus::Unavailable,
                None,
                format!("the scene_layout is invalid: {}", err.message()),
                Vec::new(),
            ));
        }
    };
    let artifact_by_id: BTreeMap<&str, &crate::context::ArtifactEntry> = manifest
        .artifacts
        .iter()
        .map(|a| (a.artifact_id.as_str(), a))
        .collect();

    // Measure every component bbox.
    let mut bboxes: BTreeMap<String, BBox> = BTreeMap::new();
    let mut unmeasured = Vec::new();
    for component in &layout.components {
        let Some(artifact) = artifact_by_id.get(component.artifact.as_str()) else {
            unmeasured.push(component.component_id.clone());
            continue;
        };
        if !artifact.path.ends_with(".obj") {
            unmeasured.push(component.component_id.clone());
            continue;
        }
        let bytes = context.read_bytes(&artifact.path)?;
        let data = parse_obj(&bytes)?;
        if let Some(bbox) =
            BBox::of_transformed(world.get(&component.component_id).unwrap_or(&crate::checks::parsing::IDENTITY), &data.vertices)
        {
            bboxes.insert(component.component_id.clone(), bbox);
        } else {
            unmeasured.push(component.component_id.clone());
        }
    }
    if !unmeasured.is_empty() {
        return Ok(record(
            CheckId::MinimumClearance,
            CheckStatus::Unavailable,
            None,
            format!(
                "the components {:?} cannot be measured by this module",
                unmeasured
            ),
            Vec::new(),
        ));
    }

    let entrance_roles = layout
        .components
        .iter()
        .filter(|component| component.role == "entrance")
        .map(|component| component.component_id.clone())
        .collect::<Vec<_>>();
    if entrance_roles.is_empty() {
        return Ok(record(
            CheckId::MinimumClearance,
            CheckStatus::Unavailable,
            None,
            "no component carries the 'entrance' role; the clearance target is undefined"
                .to_string(),
            Vec::new(),
        ));
    }
    let mut minimum = f64::INFINITY;
    for entrance in &entrance_roles {
        let Some(entrance_bbox) = bboxes.get(entrance) else {
            continue;
        };
        for (other_id, other_bbox) in &bboxes {
            if other_id == entrance || entrance_roles.contains(other_id) {
                continue;
            }
            let gap = crate::checks::parsing::bbox_separation(entrance_bbox, other_bbox);
            minimum = minimum.min(gap);
        }
    }
    let holds = threshold.comparison.holds(minimum, threshold.value);
    if holds {
        Ok(record(
            CheckId::MinimumClearance,
            CheckStatus::Pass,
            Some(minimum),
            format!(
                "the minimum entrance separation {:.6} satisfies {} {}",
                minimum,
                threshold.comparison.as_str(),
                threshold.value
            ),
            Vec::new(),
        ))
    } else {
        Ok(record(
            CheckId::MinimumClearance,
            CheckStatus::Fail,
            Some(minimum),
            format!(
                "CLEARANCE_VIOLATED: the minimum entrance separation {:.6} violates {} {}",
                minimum,
                threshold.comparison.as_str(),
                threshold.value
            ),
            Vec::new(),
        ))
    }
}
