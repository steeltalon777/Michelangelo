//! PROVISIONAL adapter seam for the future GLM-1 contracts.
//!
//! Everything in this module is a **private internal view**, explicitly not
//! the approved wire protocol of ADR 0005 §5.2. The GLM-1 contracts stream
//! owns the real DTOs; when they land, the mapping functions below are the
//! single place to adapt (rename fields, split documents, change schema
//! tags). Nothing outside this crate should depend on these shapes.
//!
//! Design intent for the future contract:
//! * `SourceSnapshotView` maps toward the ADR `SourceSnapshot` document
//!   (ID, format, payload files with actual SHA-256, dependencies,
//!   provenance, license status, ambiguities/diagnostics);
//! * `IntakeReportView` maps toward the ADR `IntakeReport` document;
//! * schema tags remain `draft-0` / `provisional` until GLM-1 ratifies the
//!   real ones.

use serde::{Deserialize, Serialize};

use crate::report::{PlanRejection, ReportStatus};
use crate::snapshot::SnapshotManifest;

/// Internal view toward the future `SourceSnapshot` contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceSnapshotView {
    /// Always `provisional/intake-source-snapshot/0` until GLM-1 lands.
    pub schema: &'static str,
    pub snapshot_id: String,
    pub format: String,
    pub payload: PayloadView,
    pub dependencies: Vec<DependencyView>,
    pub provenance: ProvenanceView,
    pub unresolved_issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayloadView {
    pub stored_name: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyView {
    pub declared_ref: String,
    pub role: String,
    pub acquisition: String,
    pub stored_name: Option<String>,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceView {
    pub supplier: Option<String>,
    pub external_id: Option<String>,
    pub license_status: String,
    pub license_decision: String,
    pub notes: Option<String>,
}

/// Map a snapshot manifest to the provisional view. Single adaptation point.
pub fn source_snapshot_view(manifest: &SnapshotManifest) -> SourceSnapshotView {
    SourceSnapshotView {
        schema: "provisional/intake-source-snapshot/0",
        snapshot_id: manifest.snapshot_id.clone(),
        format: manifest.format.as_str().to_string(),
        payload: PayloadView {
            stored_name: manifest.original_stored_name.clone(),
            sha256: manifest.source.sha256.clone(),
            size_bytes: manifest.source.size_bytes,
        },
        dependencies: manifest
            .dependencies
            .iter()
            .map(|d| DependencyView {
                declared_ref: d.declared_ref.clone(),
                role: d.role.clone(),
                acquisition: serde_json::to_string(&d.kind).unwrap_or_default(),
                stored_name: d.stored_name.clone(),
                sha256: d.sha256.clone(),
                size_bytes: d.size_bytes,
            })
            .collect(),
        provenance: ProvenanceView {
            supplier: manifest.provenance.supplier.clone(),
            external_id: manifest.provenance.external_id.clone(),
            license_status: serde_json::to_string(&manifest.provenance.license).unwrap_or_default(),
            license_decision: serde_json::to_string(&manifest.license_decision).unwrap_or_default(),
            notes: manifest.provenance.notes.clone(),
        },
        unresolved_issues: manifest
            .diagnostics
            .iter()
            .map(|d| format!("{:?} ({}): {}", d.code, d.severity_str(), d.message))
            .collect(),
    }
}

/// Internal view toward the future `IntakeReport` contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntakeReportView {
    /// Always `provisional/intake-report/0` until GLM-1 lands.
    pub schema: &'static str,
    pub status: ReportStatus,
    pub identity: Option<String>,
    pub diagnostics: Vec<serde_json::Value>,
}

pub fn intake_report_view_from_rejection(rejection: &PlanRejection) -> IntakeReportView {
    IntakeReportView {
        schema: "provisional/intake-report/0",
        status: rejection.status,
        identity: rejection.identity.clone(),
        diagnostics: rejection
            .diagnostics
            .iter()
            .map(|d| serde_json::to_value(d).unwrap_or_default())
            .collect(),
    }
}
