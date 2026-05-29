use serde::{Deserialize, Serialize};

use crate::asset::AssetSummaryDto;
use crate::JobDto;

/// Project descriptor returned by `project.create` and `project.open`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectDto {
    pub id: String,
    pub name: String,
    pub root_path: String,
}

impl ProjectDto {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        root_path: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            root_path: root_path.into(),
        }
    }
}

/// Minimal project snapshot returned by `project.get_snapshot`.
///
/// Provides enough state for a UI shell to recover after connect/reconnect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSnapshotDto {
    pub project: Option<ProjectDto>,
    pub assets: AssetSummaryDto,
    pub jobs: Vec<JobDto>,
    pub workspace_status: String,
    pub protocol_version: String,
    pub core_version: String,
}

impl ProjectSnapshotDto {
    /// Create a snapshot for a workspace with no open project.
    pub fn empty() -> Self {
        Self {
            project: None,
            assets: AssetSummaryDto::empty(),
            jobs: vec![],
            workspace_status: "no_project".into(),
            protocol_version: crate::PROTOCOL_VERSION.into(),
            core_version: crate::PROTOCOL_VERSION.into(),
        }
    }

    /// Create a snapshot for an active project.
    pub fn with_project(project: ProjectDto) -> Self {
        Self {
            project: Some(project),
            assets: AssetSummaryDto::empty(),
            jobs: vec![],
            workspace_status: "active".into(),
            protocol_version: crate::PROTOCOL_VERSION.into(),
            core_version: crate::PROTOCOL_VERSION.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_dto_roundtrip() {
        let p = ProjectDto::new("p1", "demo", "/tmp/demo");
        let json = serde_json::to_string(&p).unwrap();
        let decoded: ProjectDto = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.id, "p1");
        assert_eq!(decoded.name, "demo");
        assert_eq!(decoded.root_path, "/tmp/demo");
    }

    #[test]
    fn test_empty_snapshot_serializes() {
        let snap = ProjectSnapshotDto::empty();
        let json = serde_json::to_string(&snap).unwrap();
        assert!(json.contains(r#""workspace_status":"no_project""#));
        assert!(json.contains(r#""project":null"#));
        assert!(json.contains(r#""index_status":"empty""#));
    }

    #[test]
    fn test_with_project_snapshot() {
        let project = ProjectDto::new("x", "test", "/p");
        let snap = ProjectSnapshotDto::with_project(project);
        let json = serde_json::to_string(&snap).unwrap();
        assert!(json.contains(r#""workspace_status":"active""#));
        assert!(json.contains(r#""id":"x""#));
        assert!(json.contains(r#""name":"test""#));
    }

    #[test]
    fn test_snapshot_empty_assets_and_jobs() {
        let snap = ProjectSnapshotDto::empty();
        assert!(snap.assets.assets.is_empty());
        assert_eq!(snap.assets.total_count, 0);
        assert!(snap.jobs.is_empty());
    }
}
