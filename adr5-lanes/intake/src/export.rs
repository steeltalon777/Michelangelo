//! Passthrough export and the explicit conversion boundary.
//!
//! Passthrough means: byte-for-byte copies of the snapshot's verified
//! original and dependency copies, re-verified after writing, with an
//! explicit (empty) loss report. No conversion of any kind exists in this
//! PoC — `convert` refuses explicitly — so there is no hidden remesh,
//! retopo or decimation anywhere, and the source is never modified.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::IntakeError;
use crate::hash::sha256_hex;
use crate::snapshot::{verify_snapshot_integrity, StoredSnapshot};

/// Export manifest schema tag (provisional, same caveat as the snapshot).
pub const EXPORT_SCHEMA: &str = "michelangelo.asset-passthrough-export/draft-0";

/// Loss report of an export. Empty for a byte-identical passthrough; any
/// future conversion MUST populate `conversions` with an honest entry per
/// data loss. This PoC never populates it because it never converts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LossReport {
    pub conversions: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedFile {
    /// Role in the snapshot: "original" or "dependency".
    pub role: String,
    pub stored_name: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PassthroughExport {
    pub schema: &'static str,
    pub snapshot_id: String,
    /// SHA-256 of the pinned original, re-verified before the copy.
    pub source_sha256: String,
    /// True when every copied file matched its pinned hash on read-back.
    pub byte_identical: bool,
    pub loss_report: LossReport,
    pub files: Vec<ExportedFile>,
}

#[derive(Debug, Clone)]
pub struct StoredExport {
    pub dir: PathBuf,
    pub report: PassthroughExport,
}

/// Copy the snapshot contents into a fresh, no-clobber output directory and
/// verify every copy. The source snapshot is never modified.
pub fn export_passthrough(
    snapshot: &StoredSnapshot,
    out_root: &Path,
) -> Result<StoredExport, IntakeError> {
    // Verify the snapshot on disk before copying anything.
    verify_snapshot_integrity(&snapshot.dir, &snapshot.manifest).map_err(|e| match e {
        IntakeError::SnapshotCorrupt { .. } => e,
        other => IntakeError::SnapshotCorrupt {
            dir: snapshot.dir.clone(),
            reason: other.to_string(),
        },
    })?;

    let export_root = out_root
        .join("passthrough")
        .join(&snapshot.manifest.snapshot_id);
    let export_parent = export_root.parent().unwrap_or(out_root).to_path_buf();
    std::fs::create_dir_all(&export_parent).map_err(|source| IntakeError::Io {
        path: export_parent.clone(),
        source,
    })?;
    match std::fs::create_dir(&export_root) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(IntakeError::OutputExists { path: export_root });
        }
        Err(source) => {
            return Err(IntakeError::Io {
                path: export_root.clone(),
                source,
            })
        }
    }

    let mut files: Vec<ExportedFile> = Vec::new();
    let mut byte_identical = true;

    let source_copy = snapshot.source_copy();
    files.push(copy_verified(
        &source_copy,
        &export_root.join(&snapshot.manifest.original_stored_name),
        "original",
        &snapshot.manifest.original_stored_name,
        &mut byte_identical,
    )?);

    for dep in &snapshot.manifest.dependencies {
        let Some(stored_name) = &dep.stored_name else {
            continue;
        };
        let from = snapshot.dependency_copy(stored_name);
        files.push(copy_verified(
            &from,
            &export_root.join(stored_name),
            "dependency",
            stored_name,
            &mut byte_identical,
        )?);
    }

    let report = PassthroughExport {
        schema: EXPORT_SCHEMA,
        snapshot_id: snapshot.manifest.snapshot_id.clone(),
        source_sha256: snapshot.manifest.source.sha256.clone(),
        byte_identical,
        loss_report: LossReport {
            conversions: Vec::new(),
            notes: vec![
                "byte-for-byte passthrough of the verified snapshot; no conversion was performed"
                    .to_string(),
            ],
        },
        files,
    };
    let manifest_path = export_root.join("export.json");
    let json = serde_json::to_vec_pretty(&report).expect("export report serialization cannot fail");
    write_verified(&manifest_path, &json)?;

    Ok(StoredExport {
        dir: export_root,
        report,
    })
}

fn copy_verified(
    from: &Path,
    to: &Path,
    role: &str,
    stored_name: &str,
    byte_identical: &mut bool,
) -> Result<ExportedFile, IntakeError> {
    let bytes = std::fs::read(from).map_err(|source| IntakeError::Io {
        path: from.to_path_buf(),
        source,
    })?;
    let expected = sha256_hex(&bytes);
    write_verified(to, &bytes)?;
    let actual = sha256_hex(&std::fs::read(to).map_err(|source| IntakeError::Io {
        path: to.to_path_buf(),
        source,
    })?);
    if actual != expected {
        *byte_identical = false;
    }
    Ok(ExportedFile {
        role: role.to_string(),
        stored_name: stored_name.to_string(),
        sha256: expected,
        size_bytes: bytes.len() as u64,
    })
}

fn write_verified(path: &Path, bytes: &[u8]) -> Result<(), IntakeError> {
    std::fs::write(path, bytes).map_err(|source| IntakeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let actual = sha256_hex(&std::fs::read(path).map_err(|source| IntakeError::Io {
        path: path.to_path_buf(),
        source,
    })?);
    if actual != sha256_hex(bytes) {
        return Err(IntakeError::CopyVerificationFailed {
            path: path.to_path_buf(),
            expected_sha256: sha256_hex(bytes),
            actual_sha256: actual,
        });
    }
    Ok(())
}

/// Explicit conversion boundary. This PoC implements NO format conversion:
/// no remesh, no retopo, no decimation, no re-save. Any future conversion
/// must produce an explicit loss report and a new derived revision; the
/// original snapshot is never touched.
pub fn convert(_snapshot: &StoredSnapshot, _target: &str) -> Result<StoredExport, IntakeError> {
    Err(IntakeError::ConversionNotImplemented)
}
