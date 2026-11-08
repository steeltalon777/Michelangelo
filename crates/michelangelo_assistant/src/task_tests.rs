//! The tests of the bounded assistant task.

#![cfg(test)]

use crate::policy::AssistantPolicy;
use crate::task::{generate_task, load_task, next_task_id, task_path, write_task, TaskRequest};
use crate::testutil::Fixture;
use crate::AssistantError;

use michelangelo_edit::candidate::Region;

/// Build a fixture with an initialised scene store.
fn fixture() -> Result<Fixture, Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    fixture.init_scene(&["base"])?;
    Ok(fixture)
}

/// Generate a task of the fixture.
fn task_of(
    fixture: &Fixture,
    request: &TaskRequest,
) -> Result<crate::AssistantTask, AssistantError> {
    let policy = AssistantPolicy::default();
    generate_task(
        &fixture.run_dir,
        "asst-0001",
        request,
        &policy,
        &fixture.clock,
    )
}

#[test]
fn the_task_carries_the_schema_and_the_identity_of_the_run(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let task = task_of(&fixture, &TaskRequest::new())?;
    assert_eq!(task.schema, crate::TASK_SCHEMA);
    assert_eq!(task.task_id, "asst-0001");
    assert_eq!(task.base.package_id, crate::testutil::PACKAGE_ID);
    assert_eq!(task.base.package_revision, 1);
    assert_eq!(task.base.run_id, crate::testutil::RUN_ID);
    assert_eq!(task.base.scene_revision, "s0001");
    assert_eq!(task.created_at, crate::testutil::CLOCK_TIMESTAMP);
    Ok(())
}

#[test]
fn the_generation_is_deterministic() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let first = task_of(&fixture, &TaskRequest::new())?;
    let second = task_of(&fixture, &TaskRequest::new())?;
    assert_eq!(first, second);
    Ok(())
}

#[test]
fn the_allowed_components_are_sorted_and_the_protected_ones_are_the_rest(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let task = task_of(&fixture, &TaskRequest::new())?;
    assert_eq!(task.allowed_components, vec!["lid".to_string()]);
    assert_eq!(task.protected, vec!["base".to_string()]);
    for name in &task.allowed_components {
        assert!(!task.protected.contains(name));
    }
    Ok(())
}

#[test]
fn an_explicit_set_of_components_is_honoured() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let task = task_of(&fixture, &TaskRequest::with_allowed(&["lid".to_string()]))?;
    assert_eq!(task.allowed_components, vec!["lid".to_string()]);
    assert_eq!(task.protected, vec!["base".to_string()]);
    Ok(())
}

#[test]
fn an_unknown_component_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let error = task_of(&fixture, &TaskRequest::with_allowed(&["ghost".to_string()]))
        .expect_err("the unknown component is refused");
    assert!(matches!(error, AssistantError::TaskInvalid { .. }));
    Ok(())
}

#[test]
fn a_protected_component_cannot_be_allowed() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let error = task_of(&fixture, &TaskRequest::with_allowed(&["base".to_string()]))
        .expect_err("the protected component is refused");
    assert!(matches!(error, AssistantError::TaskInvalid { .. }));
    Ok(())
}

#[test]
fn an_empty_set_of_components_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let error =
        task_of(&fixture, &TaskRequest::with_allowed(&[])).expect_err("the empty set is refused");
    assert!(matches!(error, AssistantError::TaskInvalid { .. }));
    Ok(())
}

#[test]
fn the_default_region_is_the_bounding_box_with_a_margin() -> Result<(), Box<dyn std::error::Error>>
{
    let fixture = fixture()?;
    let task = task_of(&fixture, &TaskRequest::new())?;
    // The lid spans -0.04..0.04, 0.05..0.09 and -0.04..0.04; the margin is 5 %.
    assert_eq!(task.allowed_region.min, [-0.044, 0.048, -0.044]);
    assert_eq!(task.allowed_region.max, [0.044, 0.092, 0.044]);
    Ok(())
}

/// Write a binding document into the store of a fixture.
fn bind(
    fixture: &Fixture,
    unknown_id: &str,
    components: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = fixture.run_dir.join("verification/bindings");
    std::fs::create_dir_all(&directory)?;
    let bindings: Vec<serde_json::Value> = components
        .iter()
        .map(|component| {
            serde_json::json!({
                "unknown_id": unknown_id,
                "components": [component],
                "region": {"min": [-0.05, 0.04, -0.05], "max": [0.05, 0.10, 0.05]},
                "by": "alice",
                "claim": "the inside of the lid is a plain box"
            })
        })
        .collect();
    let document = serde_json::json!({
        "schema": "michelangelo.unknown-bindings/0",
        "version": "b0001",
        "created_at": "2026-01-02T03:04:05Z",
        "bindings": bindings
    });
    std::fs::write(directory.join("b0001.json"), document.to_string())?;
    Ok(())
}

/// The region of the binding of the tests.
fn binding_region() -> Region {
    Region {
        min: [-0.05, 0.04, -0.05],
        max: [0.05, 0.10, 0.05],
    }
}

#[test]
fn a_task_of_an_unknown_is_narrowed_to_the_binding() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    bind(&fixture, "u_lid_inside", &["lid"])?;
    let task = task_of(&fixture, &TaskRequest::with_unknown("u_lid_inside"))?;
    assert_eq!(task.unknown_id.as_deref(), Some("u_lid_inside"));
    assert_eq!(task.allowed_components, vec!["lid".to_string()]);
    assert_eq!(task.protected, vec!["base".to_string()]);
    assert_eq!(task.allowed_region, binding_region());
    let criteria = task
        .unknown_criteria
        .clone()
        .expect("the criteria of the unknown are present");
    assert_eq!(criteria.unknown_id, "u_lid_inside");
    assert_eq!(criteria.components, vec!["lid".to_string()]);
    assert_eq!(criteria.region, binding_region());
    assert_eq!(
        criteria.claim.as_deref(),
        Some("the inside of the lid is a plain box")
    );
    assert!(criteria.requires_confirmation);
    assert!(criteria.requires_evaluation);
    assert!(!criteria.conditions.is_empty());
    // The references are filtered: only the unknown of the binding is listed.
    assert_eq!(task.unknowns.len(), 1);
    assert_eq!(task.unknowns[0].id, "u_lid_inside");
    Ok(())
}

#[test]
fn a_narrowed_task_may_not_widen_the_components_of_the_binding(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    bind(&fixture, "u_lid_inside", &["lid"])?;
    let request = TaskRequest {
        allowed: Some(vec!["base".to_string()]),
        region: None,
        unknown: Some("u_lid_inside".to_string()),
    };
    let error = task_of(&fixture, &request).expect_err("the widening is refused");
    match error {
        AssistantError::TaskInvalid { message } => {
            assert!(message.contains("SCOPE_DENIED"), "{message}");
            assert!(message.contains("base"), "{message}");
        }
        other => panic!("unexpected error: {other}"),
    }
    Ok(())
}

#[test]
fn a_narrowed_task_may_narrow_the_components_of_the_binding(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    bind(&fixture, "u_lid_inside", &["lid", "base"])?;
    let request = TaskRequest {
        allowed: Some(vec!["lid".to_string()]),
        region: None,
        unknown: Some("u_lid_inside".to_string()),
    };
    let task = task_of(&fixture, &request)?;
    assert_eq!(task.allowed_components, vec!["lid".to_string()]);
    assert_eq!(task.protected, vec!["base".to_string()]);
    let criteria = task
        .unknown_criteria
        .clone()
        .expect("the criteria of the unknown are present");
    assert_eq!(
        criteria.components,
        vec!["lid".to_string()],
        "the criteria carry the narrowed scope"
    );
    Ok(())
}

#[test]
fn a_narrowed_task_may_not_widen_the_region_of_the_binding(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    bind(&fixture, "u_lid_inside", &["lid"])?;
    let request = TaskRequest {
        allowed: None,
        region: Some(Region {
            min: [-0.2, -0.2, -0.2],
            max: [0.2, 0.2, 0.2],
        }),
        unknown: Some("u_lid_inside".to_string()),
    };
    let error = task_of(&fixture, &request).expect_err("the widening is refused");
    match error {
        AssistantError::TaskInvalid { message } => {
            assert!(message.contains("SCOPE_DENIED"), "{message}");
        }
        other => panic!("unexpected error: {other}"),
    }
    Ok(())
}

#[test]
fn a_narrowed_task_may_narrow_the_region_of_the_binding() -> Result<(), Box<dyn std::error::Error>>
{
    let fixture = fixture()?;
    bind(&fixture, "u_lid_inside", &["lid"])?;
    let region = Region {
        min: [-0.04, 0.05, -0.04],
        max: [0.04, 0.09, 0.04],
    };
    let request = TaskRequest {
        allowed: None,
        region: Some(region),
        unknown: Some("u_lid_inside".to_string()),
    };
    let task = task_of(&fixture, &request)?;
    assert_eq!(task.allowed_region, region);
    let criteria = task
        .unknown_criteria
        .clone()
        .expect("the criteria of the unknown are present");
    assert_eq!(
        criteria.region, region,
        "the criteria carry the narrowed region"
    );
    Ok(())
}

#[test]
fn a_task_of_an_unknown_without_a_binding_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let error = task_of(&fixture, &TaskRequest::with_unknown("u_lid_inside"))
        .expect_err("the task is refused");
    assert!(matches!(error, AssistantError::NotFound { .. }));
    Ok(())
}

#[test]
fn a_task_of_an_unknown_that_is_not_bound_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    bind(&fixture, "u_lid_inside", &["lid"])?;
    let error =
        task_of(&fixture, &TaskRequest::with_unknown("u_ghost")).expect_err("the task is refused");
    assert!(matches!(error, AssistantError::NotFound { .. }));
    Ok(())
}

#[test]
fn an_explicit_region_is_used() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let region = Region {
        min: [-0.05, 0.04, -0.05],
        max: [0.05, 0.10, 0.05],
    };
    let request = TaskRequest {
        allowed: Some(vec!["lid".to_string()]),
        region: Some(region),
        unknown: None,
    };
    let task = task_of(&fixture, &request)?;
    assert_eq!(task.allowed_region, region);
    Ok(())
}

#[test]
fn an_inverted_region_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let request = TaskRequest {
        allowed: Some(vec!["lid".to_string()]),
        region: Some(Region {
            min: [0.1, 0.1, 0.1],
            max: [-0.1, -0.1, -0.1],
        }),
        unknown: None,
    };
    let error = task_of(&fixture, &request).expect_err("the inverted region is refused");
    assert!(matches!(error, AssistantError::TaskInvalid { .. }));
    Ok(())
}

#[test]
fn a_region_that_does_not_intersect_the_component_is_refused(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let request = TaskRequest {
        allowed: Some(vec!["lid".to_string()]),
        region: Some(Region {
            min: [1.0, 1.0, 1.0],
            max: [2.0, 2.0, 2.0],
        }),
        unknown: None,
    };
    let error = task_of(&fixture, &request).expect_err("the empty region is refused");
    match error {
        AssistantError::TaskInvalid { message } => assert!(message.contains("TASK_EMPTY")),
        other => panic!("unexpected error: {other}"),
    }
    Ok(())
}

#[test]
fn the_references_only_carry_the_calibrated_views() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let task = task_of(&fixture, &TaskRequest::new())?;
    assert_eq!(task.references.views.len(), 1);
    assert_eq!(task.references.views[0].view_id, "front");
    assert_eq!(
        task.references.views[0].mask,
        Some("package/masks/front.png".to_string())
    );
    assert_eq!(
        task.references.views[0].image,
        Some("package/images/front.png".to_string())
    );
    Ok(())
}

#[test]
fn the_references_carry_the_renders_and_the_reports() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let task = task_of(&fixture, &TaskRequest::new())?;
    assert_eq!(task.references.renders.len(), 1);
    assert_eq!(task.references.renders[0].path, "renders/front.png");
    assert_eq!(
        task.references.quality_report,
        Some("quality_report.json".to_string())
    );
    assert_eq!(task.references.build_report, None);
    Ok(())
}

#[test]
fn the_references_never_carry_the_evaluation_data() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let task = task_of(&fixture, &TaskRequest::new())?;
    let references = task.to_json()?;
    assert!(!references.contains("evaluation_renders"));
    assert!(!references.contains("ground_truth"));
    for view in &task.references.views {
        let mask = view.mask.clone().unwrap_or_default();
        let image = view.image.clone().unwrap_or_default();
        assert!(!mask.contains("evaluation"));
        assert!(!image.contains("evaluation"));
    }
    for render in &task.references.renders {
        assert!(!render.path.contains("evaluation"));
    }
    Ok(())
}

#[test]
fn the_task_carries_no_absolute_path() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let task = task_of(&fixture, &TaskRequest::new())?;
    let text = task.to_json()?;
    let rendered = fixture.workspace.display().to_string();
    assert!(!text.contains(&rendered));
    assert!(!text.contains("C:\\"));
    Ok(())
}

#[test]
fn the_acceptance_criteria_come_from_the_thresholds() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let task = task_of(&fixture, &TaskRequest::new())?;
    assert_eq!(task.acceptance_criteria.min_source_iou, 0.85);
    assert_eq!(task.acceptance_criteria.min_evaluation_iou, 0.7);
    assert_eq!(
        task.acceptance_criteria.measurement_relative_error_max,
        0.05
    );
    assert!(task.acceptance_criteria.reject_if_protected_changed);
    Ok(())
}

#[test]
fn the_budget_comes_from_the_policy() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let policy = AssistantPolicy {
        max_calls_per_run: 2,
        max_candidate_bytes: 4096,
        ..AssistantPolicy::default()
    };
    let task = generate_task(
        &fixture.run_dir,
        "asst-0001",
        &TaskRequest::new(),
        &policy,
        &fixture.clock(),
    )?;
    assert_eq!(task.budget.max_calls, 2);
    assert_eq!(task.budget.max_candidate_bytes, 4096);
    assert_eq!(task.budget.max_seconds, 600);
    assert_eq!(task.budget.max_cost_usd, 0.0);
    Ok(())
}

#[test]
fn the_data_policy_denies_the_network() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let task = task_of(&fixture, &TaskRequest::new())?;
    assert!(!task.data_policy.allow_network);
    assert!(!task.data_policy.allow_external_service);
    assert_eq!(
        task.data_policy.allowed_reference_kinds,
        vec![
            "mask".to_string(),
            "silhouette_render".to_string(),
            "report".to_string()
        ]
    );
    Ok(())
}

#[test]
fn the_unknowns_and_the_supplements_are_summarised() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let task = task_of(&fixture, &TaskRequest::new())?;
    assert_eq!(task.unknowns.len(), 1);
    assert_eq!(task.unknowns[0].id, "u_lid_inside");
    assert_eq!(task.unknowns[0].policy, "author_required");
    assert_eq!(task.supplements.len(), 1);
    assert_eq!(task.supplements[0].supplement_id, "lid_thickness");
    assert_eq!(task.supplements[0].kind, "thickness");
    Ok(())
}

#[test]
fn the_provenance_names_the_core_and_the_run() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let task = task_of(&fixture, &TaskRequest::new())?;
    assert_eq!(task.provenance.origin, "core");
    assert!(task.provenance.detail.contains(crate::testutil::RUN_ID));
    Ok(())
}

#[test]
fn the_task_is_written_and_read_back() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let task = task_of(&fixture, &TaskRequest::new())?;
    let path = write_task(&fixture.run_dir, &task)?;
    assert!(path.is_file());
    assert_eq!(path, task_path(&fixture.run_dir, "asst-0001"));
    let read = load_task(&path)?;
    assert_eq!(read, task);
    Ok(())
}

#[test]
fn the_next_task_id_increases() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    assert_eq!(next_task_id(&fixture.run_dir)?, "asst-0001");
    let task = task_of(&fixture, &TaskRequest::new())?;
    write_task(&fixture.run_dir, &task)?;
    assert_eq!(next_task_id(&fixture.run_dir)?, "asst-0002");
    Ok(())
}

#[test]
fn a_task_with_an_unknown_field_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let task = task_of(&fixture, &TaskRequest::new())?;
    let mut value: serde_json::Value = serde_json::from_str(&task.to_json()?)?;
    value
        .as_object_mut()
        .ok_or("the task is an object")?
        .insert("secret".to_string(), serde_json::Value::Bool(true));
    let error =
        crate::AssistantTask::parse(&value.to_string()).expect_err("the unknown field is refused");
    assert!(matches!(error, AssistantError::TaskInvalid { .. }));
    Ok(())
}

#[test]
fn a_task_with_another_schema_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    let task = task_of(&fixture, &TaskRequest::new())?;
    let mut value: serde_json::Value = serde_json::from_str(&task.to_json()?)?;
    value
        .as_object_mut()
        .ok_or("the task is an object")?
        .insert(
            "schema".to_string(),
            serde_json::Value::String("other/0".to_string()),
        );
    let error = crate::AssistantTask::parse(&value.to_string()).expect_err("the schema is refused");
    assert!(matches!(error, AssistantError::TaskInvalid { .. }));
    Ok(())
}

// ---------------------------------------------------------------------------
// IT-8E: the task reads the binding of the bundle of the current revision
// ---------------------------------------------------------------------------

/// Write a binding document of a version into the store of a fixture.
fn bind_version(
    fixture: &Fixture,
    version: &str,
    unknown_id: &str,
    components: &[&str],
    by: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = fixture.run_dir.join("verification/bindings");
    std::fs::create_dir_all(&directory)?;
    let bindings: Vec<serde_json::Value> = components
        .iter()
        .map(|component| {
            serde_json::json!({
                "unknown_id": unknown_id,
                "components": [component],
                "region": {"min": [-0.05, 0.04, -0.05], "max": [0.05, 0.10, 0.05]},
                "by": by,
                "claim": "the inside of the lid is a plain box"
            })
        })
        .collect();
    let document = serde_json::json!({
        "schema": "michelangelo.unknown-bindings/0",
        "version": version,
        "created_at": "2026-01-02T03:04:05Z",
        "bindings": bindings
    });
    std::fs::write(
        directory.join(format!("{version}.json")),
        document.to_string(),
    )?;
    Ok(())
}

/// Write a candidate directory on a base revision.
fn write_candidate_on(
    fixture: &Fixture,
    name: &str,
    candidate_id: &str,
    base_revision: &str,
    lid: &str,
) -> std::io::Result<std::path::PathBuf> {
    let directory = fixture.workspace.join(name);
    std::fs::create_dir_all(directory.join("geometry"))?;
    let mut document = crate::testutil::candidate_document(candidate_id, "fix-lid");
    document["base"]["scene_revision"] = serde_json::json!(base_revision);
    std::fs::write(
        directory.join("candidate.json"),
        serde_json::to_string_pretty(&document).map_err(std::io::Error::other)?,
    )?;
    std::fs::write(directory.join("geometry/lid.obj"), lid)?;
    Ok(directory)
}

/// Register an evaluation of a candidate of the fixture.
fn register_evaluation(fixture: &Fixture, candidate_id: &str) -> (std::path::PathBuf, String) {
    let registry = michelangelo_evidence::registry::load_registry(&fixture.run_dir)
        .expect("the registry is read");
    let evaluation_id = registry.next_evaluation_id();
    let report = michelangelo_evidence::testing::synthetic_evaluation(
        &fixture.run_dir,
        candidate_id,
        &evaluation_id,
        "2026-01-03T04:05:06Z",
    )
    .expect("the synthetic evaluation is built");
    let entry = michelangelo_evidence::registry::write_and_register(&fixture.run_dir, &report)
        .expect("the evaluation is registered");
    (fixture.run_dir.join(entry.report_path), evaluation_id)
}

#[test]
fn owner_review_task_after_rollback_uses_selected_bundle_binding(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture()?;
    bind(&fixture, "u_lid_inside", &["lid"])?;
    let store = michelangelo_edit::scene::SceneStore::new(&fixture.run_dir);

    // The first candidate is accepted with the resolution of the unknown: the
    // bundle of the revision references the binding of the lid.
    let first = write_candidate_on(
        &fixture,
        "cand-0001",
        "cand-0001",
        "s0001",
        crate::testutil::LID_OBJ,
    )?;
    michelangelo_edit::apply::submit_candidate(&fixture.run_dir, &first, &fixture.clock)?;
    let (report_path, evaluation_id) = register_evaluation(&fixture, "cand-0001");
    let evaluation = michelangelo_edit::gate::resolve_evaluation_by_id(&store, &evaluation_id)?;
    michelangelo_edit::unknowns::confirm_unknown(
        &store,
        "u_lid_inside",
        "alice",
        &evaluation,
        None,
        None,
        &fixture.clock,
    )?;
    let accepted = michelangelo_edit::apply::accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0001",
        "alice",
        &fixture.clock,
        michelangelo_edit::gate::AcceptOptions {
            evaluation: Some(report_path),
            resolve: Some("u_lid_inside".to_string()),
        },
    )?;
    assert_eq!(accepted.resulting_scene_revision.as_deref(), Some("s0002"));

    // The author rebinds the unknown with another scope.
    bind_version(&fixture, "b0002", "u_lid_inside", &["base"], "bob")?;

    // A later revision is published with the new binding.
    let second = write_candidate_on(
        &fixture,
        "cand-0002",
        "cand-0002",
        "s0002",
        &crate::testutil::LID_OBJ.replace("0.09", "0.08"),
    )?;
    michelangelo_edit::apply::submit_candidate(&fixture.run_dir, &second, &fixture.clock)?;
    let (report_path, _evaluation_id) = register_evaluation(&fixture, "cand-0002");
    let accepted = michelangelo_edit::apply::accept_candidate_with_options(
        &fixture.run_dir,
        "cand-0002",
        "alice",
        &fixture.clock,
        michelangelo_edit::gate::AcceptOptions {
            evaluation: Some(report_path),
            resolve: None,
        },
    )?;
    assert_eq!(accepted.resulting_scene_revision.as_deref(), Some("s0003"));

    // The rollback selects the first revision again: its bundle carries the
    // binding of the lid.
    michelangelo_edit::apply::rollback_scene(&fixture.run_dir, "s0002", "alice", &fixture.clock)?;

    // The view and the task read the same binding: the rolled back one.
    let view = michelangelo_edit::unknowns::show_unknowns(&store, None)?;
    let selected = view
        .unknowns
        .iter()
        .find(|entry| entry.unknown_id == "u_lid_inside")
        .expect("the unknown is listed");
    let task = task_of(&fixture, &TaskRequest::with_unknown("u_lid_inside"))?;
    assert_eq!(
        task.allowed_components,
        selected
            .binding
            .as_ref()
            .expect("the binding is shown")
            .components,
        "the assistant task must not silently reactivate a rolled back binding"
    );
    assert_eq!(task.allowed_components, vec!["lid".to_string()]);
    Ok(())
}
