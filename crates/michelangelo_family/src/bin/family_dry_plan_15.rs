//! family_dry_plan_15 — deterministic dry-plan demo (pure planning, no
//! Blender, no execution).
//!
//! Modes:
//!   family_dry_plan_15 --regen <fixtures_dir>   write wire-shaped fixture
//!                                               documents (source_set.json,
//!                                               family_recipe.json,
//!                                               recipe_lock.json) with real
//!                                               computed digests
//!   family_dry_plan_15 <fixtures_dir>           load fixtures, validate, plan
//!                                               15 distinct variant requests
//!                                               and print the canonical JSON
//!                                               dry plan to stdout

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use michelangelo_family::errors::FamilyResult;
use michelangelo_family::fixture;
use michelangelo_family::json::{parse, Json};
use michelangelo_family::plan::{sample_requests, FamilyPlanner};
use michelangelo_family::wire::{parse_lock, parse_recipe, parse_source_set};
use michelangelo_family::{BatchExecutorPort, FamilyRecipe, RecipeLock, SourceSet};

const REQUEST_COUNT: usize = 15;
const BATCH_ID: &str = "urban-house-dry-15";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(|s| s.as_str()) {
        Some("--regen") => regen(args.get(1).expect("usage: --regen <fixtures_dir>")),
        Some(dir) => plan_from_fixtures(dir),
        None => {
            eprintln!("usage: family_dry_plan_15 [--regen] <fixtures_dir>");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("family_dry_plan_15 failed: {e}");
        std::process::exit(1);
    }
}

fn regen(dir: &str) -> FamilyResult<()> {
    let (sources, recipe, lock) = fixture::test_house();
    let path = Path::new(dir);
    fs::create_dir_all(path).expect("create fixtures dir");

    let docs: BTreeMap<&str, Json> = BTreeMap::from([
        ("source_set.json", sources.to_json()),
        ("family_recipe.json", recipe.to_json()),
        ("recipe_lock.json", lock.to_json()),
    ]);

    for (name, doc) in &docs {
        let file = path.join(name);
        fs::write(&file, format!("{}\n", doc.to_canonical())).expect("write fixture");
        eprintln!("wrote {}", file.display());
    }
    eprintln!("fixtures regenerated; run without --regen to produce the dry plan");
    Ok(())
}

fn plan_from_fixtures(dir: &str) -> FamilyResult<()> {
    let path = Path::new(dir);
    let sources: SourceSet = parse_source_set(&read_json(&path.join("source_set.json"))?)?;
    let recipe: FamilyRecipe = parse_recipe(&read_json(&path.join("family_recipe.json"))?)?;
    let lock: RecipeLock = parse_lock(&read_json(&path.join("recipe_lock.json"))?)?;

    let requests = sample_requests(&recipe, "house", REQUEST_COUNT)?;
    let plan = FamilyPlanner::plan(BATCH_ID, &recipe, &sources, &lock, &requests)?;

    // Pure-planning boundary visible on purpose: the plan cannot be executed
    // inside this stream.
    let null_executor = michelangelo_family::NullBatchExecutor;
    let port_result = null_executor.accept_plan(&plan);

    println!("{}", plan.to_canonical_json());
    eprintln!(
        "dry plan ok: {} items, recipe {}@{}, lock digest {}",
        plan.items.len(),
        plan.recipe_id,
        plan.recipe_revision,
        &plan.recipe_lock_digest[..16]
    );
    match port_result {
        Ok(_) => eprintln!("unexpected executor ack"),
        Err(e) => eprintln!("executor port refused (expected): {e}"),
    }
    Ok(())
}

fn read_json(path: &Path) -> FamilyResult<Json> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        michelangelo_family::FamilyError::new(
            michelangelo_family::FamilyErrorCode::SchemaInvalid,
            format!("cannot read {}: {e}", path.display()),
        )
    })?;
    parse(&text).map_err(|e| {
        michelangelo_family::FamilyError::new(
            michelangelo_family::FamilyErrorCode::SchemaInvalid,
            format!("{}: {e}", path.display()),
        )
    })
}
