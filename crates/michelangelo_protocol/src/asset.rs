use serde::{Deserialize, Serialize};

/// Status of the asset graph index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexStatus {
    #[serde(rename = "empty")]
    Empty,
    #[serde(rename = "indexing")]
    Indexing,
    #[serde(rename = "ready")]
    Ready,
    #[serde(rename = "error")]
    Error,
}

/// Minimal asset descriptor returned in project snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetDto {
    pub id: String,
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
}

impl AssetDto {
    pub fn new(id: impl Into<String>, name: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind: kind.into(),
            file_path: None,
        }
    }
}

/// Summary of the asset graph included in project snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetSummaryDto {
    pub assets: Vec<AssetDto>,
    pub index_status: IndexStatus,
    pub total_count: usize,
}

impl AssetSummaryDto {
    pub fn empty() -> Self {
        Self {
            assets: vec![],
            index_status: IndexStatus::Empty,
            total_count: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asset_dto_serializes() {
        let asset = AssetDto::new("a1", "model.glb", "model");
        let json = serde_json::to_string(&asset).unwrap();
        assert!(json.contains(r#""id":"a1""#));
        assert!(json.contains(r#""kind":"model""#));
        assert!(!json.contains(r#""file_path""#));
    }

    #[test]
    fn test_asset_dto_roundtrip() {
        let asset = AssetDto {
            id: "a2".into(),
            name: "texture.png".into(),
            kind: "texture".into(),
            file_path: Some("/workspace/texture.png".into()),
        };
        let json = serde_json::to_string(&asset).unwrap();
        let decoded: AssetDto = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.file_path, Some("/workspace/texture.png".into()));
    }

    #[test]
    fn test_index_status_serializes() {
        assert_eq!(
            serde_json::to_string(&IndexStatus::Empty).unwrap(),
            r#""empty""#
        );
        assert_eq!(
            serde_json::to_string(&IndexStatus::Ready).unwrap(),
            r#""ready""#
        );
    }

    #[test]
    fn test_summary_empty() {
        let summary = AssetSummaryDto::empty();
        assert!(summary.assets.is_empty());
        assert_eq!(summary.index_status, IndexStatus::Empty);
        assert_eq!(summary.total_count, 0);
    }

    #[test]
    fn test_summary_serializes() {
        let summary = AssetSummaryDto::empty();
        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains(r#""index_status":"empty""#));
    }
}
