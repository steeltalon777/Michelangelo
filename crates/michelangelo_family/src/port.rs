//! Provisional handoff port to the future Batch Executor (GLM-3 stream).
//!
//! The Family Builder ends at an immutable `ResolvedBatchPlan`. It never
//! executes anything itself: the handoff is a pure data handoff of the plan
//! document. The default implementation refuses execution so that the pure
//! planning boundary is visible at the type level.

use crate::errors::{FamilyError, FamilyErrorCode, FamilyResult};
use crate::plan::ResolvedBatchPlan;

/// PROVISIONAL port (round-2 subject, GLM-3 owns `batch.execute/resume/cancel`).
pub trait BatchExecutorPort {
    /// Hands an immutable dry plan to the executor for scheduling.
    /// Implementations must re-verify plan/lock digests before use.
    fn accept_plan(&self, plan: &ResolvedBatchPlan) -> FamilyResult<PlanAck>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlanAck {
    pub batch_id: String,
    pub item_count: usize,
    /// Always "planning_only" in this stream: no execution happened.
    pub mode: &'static str,
}

/// Refusing default: the pure planner has no executor behind it. Every accept
/// is an explicit `NOT_PLANNABLE` refusal instead of a silent no-op.
pub struct NullBatchExecutor;

impl BatchExecutorPort for NullBatchExecutor {
    fn accept_plan(&self, plan: &ResolvedBatchPlan) -> FamilyResult<PlanAck> {
        Err(FamilyError::new(
            FamilyErrorCode::NotPlannable,
            format!(
                "plan '{}' ({}) reaches the executor port, but this stream is pure planning: no execution, no Blender, no publication",
                plan.batch_id,
                plan.items.len()
            ),
        ))
    }
}
