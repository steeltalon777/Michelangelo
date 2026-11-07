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
