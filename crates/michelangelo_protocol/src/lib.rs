//! Protocol DTOs for Michelangelo headless core.
//!
//! Defines the stable command/response/event types used by all
//! Michelangelo Core clients and shells. Transport is JSONL over stdio.

pub mod asset;
pub mod command;
pub mod error;
pub mod event;
pub mod job;
pub mod method;
pub mod response;
pub mod snapshot;

pub use asset::{AssetDto, AssetSummaryDto, IndexStatus};
pub use command::CommandEnvelope;
pub use error::{ErrorCode, ProtocolError};
pub use event::EventEnvelope;
pub use job::{JobDto, JobStatus};
pub use response::ResponseEnvelope;
pub use snapshot::{ProjectDto, ProjectSnapshotDto};

/// The current protocol version carried in snapshots and metadata.
pub const PROTOCOL_VERSION: &str = env!("CARGO_PKG_VERSION");
