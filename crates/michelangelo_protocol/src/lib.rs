//! Protocol DTOs for Michelangelo headless core.
//!
//! Defines the stable command/response/event types used by all
//! Michelangelo Core clients and shells. Transport is JSONL over stdio.

pub mod asset;
pub mod blender;
pub mod command;
pub mod error;
pub mod event;
pub mod job;
pub mod method;
pub mod response;
pub mod snapshot;

pub use asset::{AssetDto, AssetSummaryDto, IndexStatus};
pub use blender::{
    BlenderArtifactDto, BlenderCapabilitiesDto, BlenderJobResult, BlenderJobSpec, BlenderOperation,
    BlenderSpecBody, SAFE_WORKER_OPS,
};
pub use command::CommandEnvelope;
pub use error::{ErrorCode, ProtocolError};
pub use event::{
    EventEnvelope, JOB_CANCELLED, JOB_COMPLETED, JOB_EVENTS, JOB_FAILED, JOB_PROGRESS, JOB_QUEUED,
    JOB_STARTED,
};
pub use job::{ArtifactDto, JobDto, JobStatus};
pub use method::{
    BLENDER_GET_CAPABILITIES, BLENDER_RUN_JOB, JOB_CANCEL, JOB_GET, JOB_LIST, PHASE_2_METHODS,
};
pub use response::ResponseEnvelope;
pub use snapshot::{ProjectDto, ProjectSnapshotDto};

/// The current protocol version carried in snapshots and metadata.
pub const PROTOCOL_VERSION: &str = env!("CARGO_PKG_VERSION");
