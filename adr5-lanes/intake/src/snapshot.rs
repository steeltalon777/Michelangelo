//! Immutable source snapshot: plan → execute.
//!
//! Two-phase contract aligned with the IT-8D invariants (docs/tz/
//! TZ_IT8_ACCEPTANCE_INTEGRITY.md §12):
//!
//! * `plan_intake` reads the source, discovers the dependency closure, and
//!   pins the SHA-256 of every byte it *actually read* (never manifest
//!   declarations);
//! * `execute_plan` re-reads every pinned file and refuses with
//!   `InputChanged` when a dependency was substituted after planning — a
//!   changed input is never a silent success;
//! * the snapshot directory is reserved with create/no-overwrite semantics
//!   keyed by content identity, so re-running on identical content is
//!   idempotent (verified reuse) while any difference creates a distinct
//!   snapshot and never overwrites an existing one;
//! * the original bytes are copied unmodified; this module performs no
//!   transformation of any kind.
//!
//! The resulting manifest is a **provisional private DTO** (schema
//! `michelangelo.asset-source-snapshot/draft-0`); it is not the approved
//! GLM-1 wire contract. See `crate::dto` for the adapter seam.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::IntakeError;
use crate::format::{detect_source_format, SourceFormat};
use crate::gltf::{GltfRef, RefRole};
use crate::hash::{identity_digest, sha256_hex};
use crate::license::{license_decision, LicenseDecision, ProvenanceInfo};
use crate::limits::IntakeLimits;
use crate::obj;
use crate::pathsafe::{flatten_for_storage, CollisionTracker, RefPathError};
use crate::report::{report_status, Diagnostic, DiagnosticCode, PlanRejection, ReportStatus};

/// Manifest schema tag. `draft-0` + `provisional: true` state clearly that
/// this is NOT the GLM-1 contract.
pub const SNAPSHOT_SCHEMA: &str = "michelangelo.asset-source-snapshot/draft-0";

/// Crate identity recorded in manifests.
pub const INTAKE_TOOL: &str = concat!(
    "michelangelo_asset_intake/",
    env!("CARGO_PKG_VERSION"),
    " (ADR 0005 stream A PoC, provisional)"
);

/// Intake request: one local source file plus supplied provenance.
#[derive(Debug, Clone)]
pub struct IntakeRequest {
    /// Local filesystem path of the source. Remote URIs are not accepted:
    /// any non-local path fails with the usual filesystem error and is never
    /// fetched.
    pub source: PathBuf,
    pub provenance: ProvenanceInfo,
}

impl IntakeRequest {
    pub fn with_unknown_provenance(source: impl Into<PathBuf>) -> Self {
        IntakeRequest {
            source: source.into(),
            provenance: ProvenanceInfo::unknown(),
        }
    }
}

/// A pinned file (source or dependency): what was read and its hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedFile {
    /// Name used inside the snapshot directory.
    pub stored_name: String,
    pub sha256: String,
    pub size_bytes: u64,
}

/// Unified dependency role across formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Buffer,
    Image,
    MaterialLibrary,
    TextureMap,
}

impl RefKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            RefKind::Buffer => "buffer",
            RefKind::Image => "image",
            RefKind::MaterialLibrary => "material_library",
            RefKind::TextureMap => "texture_map",
        }
    }
}

impl From<RefRole> for RefKind {
    fn from(r: RefRole) -> Self {
        match r {
            RefRole::Buffer => RefKind::Buffer,
            RefRole::Image => RefKind::Image,
        }
    }
}

/// How a dependency's bytes are obtained. All offsets are absolute into the
/// verified bytes, so execution needs no re-parsing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DependencyKind {
    /// A real file on disk next to (or below) the source.
    ExternalFile,
    /// A `data:` URI payload; re-decoded from `source_uri` at execution.
    EmbeddedDataUri,
    /// The GLB BIN chunk of the source itself.
    GlbBinChunk { byte_offset: u64, byte_length: u64 },
    /// Bytes inside the source GLB BIN chunk (e.g. a texture image).
    GlbBufferView { byte_offset: u64, byte_length: u64 },
    /// A byte range inside another dependency (an external buffer file).
    ExternalBufferRange {
        declared_buffer_ref: String,
        byte_offset: u64,
        byte_length: u64,
    },
}

/// One pinned dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedDependency {
    /// The reference exactly as declared in the asset file (or a synthetic
    /// label like `buffers[0]` for embedded payloads).
    pub declared_ref: String,
    pub role: String,
    pub kind: DependencyKind,
    /// Name inside `dependencies/`; `None` when the bytes live inside another
    /// stored file (e.g. an external buffer range).
    pub stored_name: Option<String>,
    pub sha256: String,
    pub size_bytes: u64,
    /// The full raw `data:` URI for `EmbeddedDataUri` dependencies, so the
    /// payload can be re-derived deterministically from pinned information.
    pub source_uri: Option<String>,
}

/// Result of a successful `plan_intake`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntakePlan {
    pub schema: &'static str,
    /// Content-derived operation identity (SHA-256 hex).
    pub identity: String,
    pub format: SourceFormat,
    pub source: PlannedFile,
    /// Absolute path of the source, for reference only — not part of the
    /// identity and never trusted at execution time (inputs are re-verified
    /// by content).
    pub source_path: PathBuf,
    pub dependencies: Vec<PlannedDependency>,
    pub diagnostics: Vec<Diagnostic>,
    pub status: ReportStatus,
    pub limits: IntakeLimits,
    pub provenance: ProvenanceInfo,
    pub license_decision: LicenseDecision,
}

impl IntakePlan {
    /// Canonical identity string: format + source hash + sorted dependency
    /// tuples. Deliberately excludes absolute paths, timestamps and tool
    /// versions so identical content has identical identity.
    fn canonical_identity(
        format: &SourceFormat,
        source_sha: &str,
        deps: &[PlannedDependency],
    ) -> String {
        let mut s = String::new();
        s.push_str("michelangelo-asset-intake/identity/0\n");
        s.push_str(&format!("format={}\n", format.as_str()));
        s.push_str(&format!("source_sha256={source_sha}\n"));
        let mut rows: Vec<String> = deps
            .iter()
            .map(|d| {
                let kind_str = serde_json::to_string(&d.kind).unwrap_or_default();
                format!(
                    "dep\t{}\t{}\t{}\t{}\n",
                    d.declared_ref, d.role, kind_str, d.sha256
                )
            })
            .collect();
        rows.sort();
        for row in rows {
            s.push_str(&row);
        }
        identity_digest(&s)
    }
}

/// Internal reference discovered in the source (and, for OBJ, in its MTL
/// files) before filesystem resolution.
#[derive(Debug, Clone)]
enum RawRef {
    /// File on disk, declared relative to the source directory. For glTF
    /// buffer-role refs this carries the document buffer index.
    External {
        uri: String,
        kind: RefKind,
        buffer_index: Option<u32>,
    },
    /// MTL file referenced by `mtllib`: pinned like a file, and its texture
    /// map references are appended as further raw refs.
    MtlFile { uri: String },
    /// Inline `data:` URI payload (already decoded; raw URI carried).
    DataUri {
        raw_uri: String,
        decoded: Vec<u8>,
        kind: RefKind,
    },
    /// The GLB BIN chunk.
    GlbBin,
    /// A byte range inside the GLB BIN chunk (absolute offsets).
    EmbeddedView { abs_offset: u64, byte_length: u64 },
    /// A byte range inside an external buffer file.
    ViewIntoExternal {
        buffer_index: u32,
        byte_offset: u64,
        byte_length: u64,
        kind: RefKind,
    },
}

/// Build a plan. Reads the source and every resolvable dependency once and
/// pins their actual bytes. Any error-severity diagnostic rejects the plan
/// with `IntakeError::PlanRejected`.
pub fn plan_intake(req: &IntakeRequest, limits: &IntakeLimits) -> Result<IntakePlan, IntakeError> {
    let source_path = req.source.clone();
    let metadata = std::fs::metadata(&source_path).map_err(|source| IntakeError::Io {
        path: source_path.clone(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(IntakeError::Io {
            path: source_path.clone(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "source is not a regular file (remote URIs are not supported)",
            ),
        });
    }
    let size = metadata.len();
    if size > limits.max_source_bytes {
        return Err(IntakeError::SourceTooLarge {
            path: source_path.clone(),
            size,
            limit: limits.max_source_bytes,
        });
    }

    let format = detect_source_format(&source_path)?;
    let source_bytes = std::fs::read(&source_path).map_err(|source| IntakeError::Io {
        path: source_path.clone(),
        source,
    })?;
    let source_sha = sha256_hex(&source_bytes);

    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut raw_refs: Vec<RawRef> = Vec::new();
    let mut bin_range: Option<crate::gltf::GlbBinRange> = None;

    match format {
        SourceFormat::Glb | SourceFormat::Gltf => {
            let scan = if format == SourceFormat::Glb {
                crate::gltf::scan_glb(&source_bytes)
            } else {
                crate::gltf::scan_gltf_json(&source_bytes)
            }
            .map_err(|e| match e {
                IntakeError::MalformedGlb { code, reason } => IntakeError::PlanRejected {
                    rejection: PlanRejection {
                        identity: None,
                        format: Some(format),
                        diagnostics: vec![Diagnostic::error(
                            code,
                            Some(source_path.display().to_string()),
                            reason,
                        )],
                        status: ReportStatus::Rejected,
                    },
                },
                other => other,
            })?;
            diagnostics.extend(scan.diagnostics.iter().cloned());
            bin_range = scan.bin_range;
            for gref in scan.refs {
                match gref {
                    GltfRef::External {
                        uri,
                        role,
                        buffer_index,
                    } => {
                        raw_refs.push(RawRef::External {
                            uri,
                            kind: kind_of(role),
                            buffer_index,
                        });
                    }
                    GltfRef::DataUri {
                        decoded,
                        role,
                        raw_uri,
                    } => {
                        raw_refs.push(RawRef::DataUri {
                            raw_uri,
                            decoded,
                            kind: kind_of(role),
                        });
                    }
                    GltfRef::GlbBin => raw_refs.push(RawRef::GlbBin),
                    GltfRef::BufferView {
                        buffer_index,
                        byte_offset,
                        byte_length,
                        role,
                    } => {
                        if scan.embedded_buffer_indices.contains(&buffer_index) {
                            let Some(bin) = bin_range else { continue };
                            raw_refs.push(RawRef::EmbeddedView {
                                abs_offset: bin.byte_offset + byte_offset,
                                byte_length,
                            });
                        } else {
                            raw_refs.push(RawRef::ViewIntoExternal {
                                buffer_index,
                                byte_offset,
                                byte_length,
                                kind: kind_of(role),
                            });
                        }
                    }
                }
            }
        }
        SourceFormat::Obj => {
            let scan = obj::scan_obj(&source_bytes, &source_path.display().to_string());
            diagnostics.extend(scan.diagnostics.iter().cloned());
            for r in scan.refs {
                if r.role == obj::ObjRefRole::MaterialLibrary {
                    raw_refs.push(RawRef::MtlFile { uri: r.declared });
                } else {
                    raw_refs.push(RawRef::External {
                        uri: r.declared,
                        kind: RefKind::TextureMap,
                        buffer_index: None,
                    });
                }
            }
        }
    }

    // Resolve and pin every reference.
    let base_dir = source_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let mut dependencies: Vec<PlannedDependency> = Vec::new();
    let mut collision = CollisionTracker::new();
    let mut total_dep_bytes: u64 = 0;
    // buffer_index -> (declared_ref, bytes) for external buffer files.
    let mut external_buffers: BTreeMap<u32, (String, Vec<u8>)> = BTreeMap::new();

    let mut index = 0usize;
    while index < raw_refs.len() {
        let rref = raw_refs[index].clone();
        index += 1;
        match rref {
            RawRef::External {
                uri,
                kind,
                buffer_index,
            } => {
                pin_file_ref(
                    &uri,
                    kind,
                    buffer_index,
                    &base_dir,
                    limits,
                    &mut dependencies,
                    &mut total_dep_bytes,
                    &mut collision,
                    &mut diagnostics,
                    &mut external_buffers,
                );
            }
            RawRef::MtlFile { uri } => {
                let pinned = pin_file_ref(
                    &uri,
                    RefKind::MaterialLibrary,
                    None,
                    &base_dir,
                    limits,
                    &mut dependencies,
                    &mut total_dep_bytes,
                    &mut collision,
                    &mut diagnostics,
                    &mut external_buffers,
                );
                if let Some(dep) = pinned {
                    if let Some(bytes) = dep_bytes_for(&dep, &base_dir, &dependencies) {
                        let subject = format!("{uri} (mtl)");
                        let mtl = obj::scan_mtl(&bytes, &subject);
                        diagnostics.extend(mtl.diagnostics.iter().cloned());
                        for r in mtl.refs {
                            raw_refs.push(RawRef::External {
                                uri: r.declared,
                                kind: RefKind::TextureMap,
                                buffer_index: None,
                            });
                        }
                    }
                }
            }
            RawRef::DataUri {
                raw_uri,
                decoded,
                kind,
            } => {
                let declared_ref = format!("data:uri:{}[{}]", kind.as_str(), dependencies.len());
                if !check_dep_size(
                    decoded.len() as u64,
                    &declared_ref,
                    limits,
                    &mut total_dep_bytes,
                    &mut diagnostics,
                ) {
                    continue;
                }
                let dep_index = dependencies.len();
                dependencies.push(PlannedDependency {
                    declared_ref,
                    role: kind.as_str().to_string(),
                    kind: DependencyKind::EmbeddedDataUri,
                    stored_name: Some(format!("embedded_{:04}.bin", dep_index)),
                    sha256: sha256_hex(&decoded),
                    size_bytes: decoded.len() as u64,
                    source_uri: Some(raw_uri),
                });
            }
            RawRef::GlbBin => {
                let Some(bin) = bin_range else { continue };
                let Ok(slice) = slice_checked(
                    &source_bytes,
                    bin.byte_offset,
                    bin.byte_length,
                    "GLB BIN chunk",
                ) else {
                    continue;
                };
                let dep_index = dependencies.len();
                dependencies.push(PlannedDependency {
                    declared_ref: "buffers[0] (GLB BIN chunk)".to_string(),
                    role: RefKind::Buffer.as_str().to_string(),
                    kind: DependencyKind::GlbBinChunk {
                        byte_offset: bin.byte_offset,
                        byte_length: bin.byte_length,
                    },
                    stored_name: Some("glb.bin".to_string()),
                    sha256: sha256_hex(slice),
                    size_bytes: slice.len() as u64,
                    source_uri: None,
                });
                let _ = dep_index;
            }
            RawRef::EmbeddedView {
                abs_offset,
                byte_length,
            } => {
                let Ok(slice) =
                    slice_checked(&source_bytes, abs_offset, byte_length, "GLB bufferView")
                else {
                    diagnostics.push(Diagnostic::error(
                        DiagnosticCode::TruncatedGlb,
                        Some("bufferView into GLB BIN chunk".to_string()),
                        format!(
                            "view range {abs_offset}..{} exceeds source size {}",
                            abs_offset + byte_length,
                            source_bytes.len()
                        ),
                    ));
                    continue;
                };
                let dep_index = dependencies.len();
                dependencies.push(PlannedDependency {
                    declared_ref: format!(
                        "bufferView in GLB BIN chunk at {abs_offset}+{byte_length}"
                    ),
                    role: RefKind::Image.as_str().to_string(),
                    kind: DependencyKind::GlbBufferView {
                        byte_offset: abs_offset,
                        byte_length,
                    },
                    stored_name: Some(format!("glb_view_{:04}.bin", dep_index)),
                    sha256: sha256_hex(slice),
                    size_bytes: slice.len() as u64,
                    source_uri: None,
                });
            }
            RawRef::ViewIntoExternal {
                buffer_index,
                byte_offset,
                byte_length,
                kind,
            } => {
                let Some((declared_buffer_ref, buffer_bytes)) = external_buffers.get(&buffer_index)
                else {
                    diagnostics.push(Diagnostic::error(
                        DiagnosticCode::GlbChunkMismatch,
                        Some(format!("bufferView into buffer {buffer_index}")),
                        "bufferView references a buffer that was not pinned",
                    ));
                    continue;
                };
                if byte_offset + byte_length > buffer_bytes.len() as u64 {
                    diagnostics.push(Diagnostic::error(
                        DiagnosticCode::TruncatedGlb,
                        Some(declared_buffer_ref.clone()),
                        format!(
                            "bufferView range {byte_offset}..{} exceeds buffer size {}",
                            byte_offset + byte_length,
                            buffer_bytes.len()
                        ),
                    ));
                    continue;
                }
                let slice =
                    &buffer_bytes[byte_offset as usize..(byte_offset + byte_length) as usize];
                dependencies.push(PlannedDependency {
                    declared_ref: format!(
                        "bufferView into buffer {buffer_index} ({declared_buffer_ref}) at {byte_offset}+{byte_length}"
                    ),
                    role: kind.as_str().to_string(),
                    kind: DependencyKind::ExternalBufferRange {
                        declared_buffer_ref: declared_buffer_ref.clone(),
                        byte_offset,
                        byte_length,
                    },
                    stored_name: None,
                    sha256: sha256_hex(slice),
                    size_bytes: byte_length,
                    source_uri: None,
                });
            }
        }
    }

    // Limit checks over the final dependency set.
    let file_dep_count = dependencies
        .iter()
        .filter(|d| matches!(d.kind, DependencyKind::ExternalFile))
        .count();
    if file_dep_count > limits.max_dependency_count {
        diagnostics.push(Diagnostic::error(
            DiagnosticCode::TooManyDependencies,
            None,
            format!(
                "{file_dep_count} external dependencies exceed the limit of {}",
                limits.max_dependency_count
            ),
        ));
    }
    if total_dep_bytes > limits.max_total_dependency_bytes {
        diagnostics.push(Diagnostic::error(
            DiagnosticCode::TotalSizeExceeded,
            None,
            format!(
                "combined dependency size {total_dep_bytes} exceeds the limit of {}",
                limits.max_total_dependency_bytes
            ),
        ));
    }

    // License rule: unknown license is recorded, never treated as permission.
    if req.provenance.license == crate::license::LicenseStatus::Unknown {
        diagnostics.push(Diagnostic::warning(
            DiagnosticCode::UnknownLicense,
            None,
            "no license information was supplied; unknown provenance is NOT a usage permission",
        ));
    }

    let status = report_status(&diagnostics);
    let identity = IntakePlan::canonical_identity(&format, &source_sha, &dependencies);
    if status == ReportStatus::Rejected {
        return Err(IntakeError::PlanRejected {
            rejection: PlanRejection {
                identity: Some(identity),
                format: Some(format),
                diagnostics,
                status,
            },
        });
    }

    let stored_name = flatten_for_storage(
        &source_path
            .file_name()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("source")),
    );
    Ok(IntakePlan {
        schema: SNAPSHOT_SCHEMA,
        identity,
        format,
        source: PlannedFile {
            stored_name,
            sha256: source_sha,
            size_bytes: source_bytes.len() as u64,
        },
        source_path,
        dependencies,
        diagnostics,
        status,
        limits: limits.clone(),
        provenance: req.provenance.clone(),
        license_decision: license_decision(&req.provenance.license),
    })
}

fn kind_of(role: RefRole) -> RefKind {
    RefKind::from(role)
}

fn slice_checked<'a>(
    bytes: &'a [u8],
    offset: u64,
    len: u64,
    subject: &str,
) -> Result<&'a [u8], IntakeError> {
    let end = offset.checked_add(len).ok_or_else(|| IntakeError::Io {
        path: PathBuf::from(subject),
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, "byte range overflows"),
    })?;
    if end > bytes.len() as u64 {
        return Err(IntakeError::Io {
            path: PathBuf::from(subject),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("byte range {offset}..{end} exceeds {} bytes", bytes.len()),
            ),
        });
    }
    Ok(&bytes[offset as usize..end as usize])
}

/// Resolve, size-check, read and hash one external file reference. Returns
/// the pinned dependency on success; on failure records a diagnostic and
/// returns `None`.
#[allow(clippy::too_many_arguments)]
fn pin_file_ref(
    uri: &str,
    kind: RefKind,
    buffer_index: Option<u32>,
    base_dir: &Path,
    limits: &IntakeLimits,
    dependencies: &mut Vec<PlannedDependency>,
    total_dep_bytes: &mut u64,
    collision: &mut CollisionTracker,
    diagnostics: &mut Vec<Diagnostic>,
    external_buffers: &mut BTreeMap<u32, (String, Vec<u8>)>,
) -> Option<PlannedDependency> {
    let dep_index = dependencies.len();
    if uri.len() > limits.max_reference_len {
        diagnostics.push(Diagnostic::error(
            DiagnosticCode::ReferenceTooLong,
            Some(uri.to_string()),
            format!(
                "reference longer than {} characters",
                limits.max_reference_len
            ),
        ));
        return None;
    }
    let resolved = match crate::pathsafe::resolve_reference(base_dir, uri) {
        Ok(r) => r,
        Err(e) => {
            diagnostics.push(ref_path_diagnostic(uri, &e));
            return None;
        }
    };
    if let Some(previous) = collision.register(&resolved.canonical, uri) {
        diagnostics.push(Diagnostic::warning(
            DiagnosticCode::PathCollision,
            Some(uri.to_string()),
            format!("distinct declared reference resolves to the same file as '{previous}'"),
        ));
    }
    let metadata = match std::fs::metadata(&resolved.canonical) {
        Ok(m) => m,
        Err(source) => {
            diagnostics.push(Diagnostic::error(
                DiagnosticCode::DependencyReadError,
                Some(uri.to_string()),
                format!("dependency exists but metadata failed: {source}"),
            ));
            return None;
        }
    };
    if !check_dep_size(metadata.len(), uri, limits, total_dep_bytes, diagnostics) {
        return None;
    }
    let bytes = match std::fs::read(&resolved.canonical) {
        Ok(b) => b,
        Err(source) => {
            diagnostics.push(Diagnostic::error(
                DiagnosticCode::DependencyReadError,
                Some(uri.to_string()),
                format!("dependency could not be read: {source}"),
            ));
            return None;
        }
    };
    let dep = PlannedDependency {
        declared_ref: uri.to_string(),
        role: kind.as_str().to_string(),
        kind: DependencyKind::ExternalFile,
        stored_name: Some(format!(
            "{:04}__{}",
            dep_index,
            flatten_for_storage(&resolved.relative)
        )),
        sha256: sha256_hex(&bytes),
        size_bytes: bytes.len() as u64,
        source_uri: None,
    };
    if kind == RefKind::Buffer {
        // Bookkeeping for bufferView range checks. The document buffer index
        // is carried by the reference itself; refs without an index (OBJ,
        // malformed documents) are never looked up by views.
        if let Some(index) = buffer_index {
            external_buffers.insert(index, (uri.to_string(), bytes));
        }
    }
    dependencies.push(dep.clone());
    Some(dep)
}

fn dep_bytes_for(
    dep: &PlannedDependency,
    base_dir: &Path,
    _deps: &[PlannedDependency],
) -> Option<Vec<u8>> {
    if dep.kind != DependencyKind::ExternalFile {
        return None;
    }
    std::fs::read(base_dir.join(&dep.declared_ref)).ok()
}

/// Map a path-resolution failure to the correct error diagnostic.
fn ref_path_diagnostic(uri: &str, err: &RefPathError) -> Diagnostic {
    let (code, message) = match err {
        RefPathError::Malformed => (
            DiagnosticCode::GlbChunkMismatch,
            "empty or malformed reference",
        ),
        RefPathError::PercentEncoded => (
            DiagnosticCode::PercentEncodedReference,
            "reference contains percent-encoding; conservatively rejected by this PoC",
        ),
        RefPathError::Absolute => (
            DiagnosticCode::AbsoluteReference,
            "reference is an absolute filesystem path",
        ),
        RefPathError::Network => (
            DiagnosticCode::NetworkReference,
            "reference uses a network or URI scheme",
        ),
        RefPathError::Traversal => (
            DiagnosticCode::PathTraversal,
            "reference escapes the source directory via '..'",
        ),
        RefPathError::BaseUnresolved => (
            DiagnosticCode::DependencyReadError,
            "source directory could not be canonicalized",
        ),
        RefPathError::NotFound => (
            DiagnosticCode::MissingDependency,
            "referenced file not found",
        ),
        RefPathError::SymlinkEscape => (
            DiagnosticCode::SymlinkEscape,
            "resolved target escapes the source directory via a symlink or junction",
        ),
        RefPathError::Unresolvable => (
            DiagnosticCode::DependencyReadError,
            "target could not be canonicalized",
        ),
    };
    Diagnostic::error(code, Some(uri.to_string()), message)
}

fn check_dep_size(
    size: u64,
    subject: &str,
    limits: &IntakeLimits,
    total_dep_bytes: &mut u64,
    diagnostics: &mut Vec<Diagnostic>,
) -> bool {
    if size > limits.max_dependency_bytes {
        diagnostics.push(Diagnostic::error(
            DiagnosticCode::DependencyTooLarge,
            Some(subject.to_string()),
            format!(
                "dependency is {size} bytes, above the limit of {}",
                limits.max_dependency_bytes
            ),
        ));
        return false;
    }
    *total_dep_bytes += size;
    if *total_dep_bytes > limits.max_total_dependency_bytes {
        diagnostics.push(Diagnostic::error(
            DiagnosticCode::TotalSizeExceeded,
            Some(subject.to_string()),
            format!(
                "combined dependency size {} exceeds the limit of {}",
                total_dep_bytes, limits.max_total_dependency_bytes
            ),
        ));
        return false;
    }
    true
}

/// The immutable snapshot written to the disposable workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotManifest {
    pub schema: String,
    pub provisional: bool,
    pub snapshot_id: String,
    pub tool: String,
    pub format: SourceFormat,
    pub source: PlannedFile,
    pub original_stored_name: String,
    pub provenance: ProvenanceInfo,
    pub license_decision: LicenseDecision,
    pub dependencies: Vec<PlannedDependency>,
    pub diagnostics: Vec<Diagnostic>,
    pub status: ReportStatus,
    pub limits: IntakeLimits,
}

/// Handle to a materialized snapshot on disk.
#[derive(Debug, Clone)]
pub struct StoredSnapshot {
    pub dir: PathBuf,
    pub manifest_path: PathBuf,
    pub manifest: SnapshotManifest,
    /// True when an existing snapshot with the same identity was verified
    /// and reused instead of created.
    pub reused: bool,
}

impl StoredSnapshot {
    pub fn source_copy(&self) -> PathBuf {
        self.dir
            .join("original")
            .join(&self.manifest.original_stored_name)
    }

    pub fn dependency_copy(&self, stored_name: &str) -> PathBuf {
        self.dir.join("dependencies").join(stored_name)
    }
}

/// Execute a plan into `workspace_root`: re-verify all pinned inputs, then
/// materialize the immutable snapshot with no-overwrite semantics.
///
/// Snapshot layout (all inside the disposable workspace):
/// `workspace_root/snapshots/<identity>/{manifest.json, original/, dependencies/}`
pub fn execute_plan(
    plan: &IntakePlan,
    workspace_root: &Path,
) -> Result<StoredSnapshot, IntakeError> {
    // 1. Re-read the source and verify the pin.
    let source_bytes = std::fs::read(&plan.source_path).map_err(|source| IntakeError::Io {
        path: plan.source_path.clone(),
        source,
    })?;
    let actual = sha256_hex(&source_bytes);
    if actual != plan.source.sha256 {
        return Err(IntakeError::InputChanged {
            path: plan.source_path.clone(),
            expected_sha256: plan.source.sha256.clone(),
            actual_sha256: actual,
        });
    }

    // 2. Re-read every external dependency and verify its pin. A dependency
    //    substituted after planning is INPUT_CHANGED, never a success.
    let base_dir = plan
        .source_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let mut external_bytes: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for dep in &plan.dependencies {
        if dep.kind != DependencyKind::ExternalFile {
            continue;
        }
        let dep_path = base_dir.join(&dep.declared_ref);
        let bytes = std::fs::read(&dep_path).map_err(|source| IntakeError::Io {
            path: dep_path.clone(),
            source,
        })?;
        let actual = sha256_hex(&bytes);
        if actual != dep.sha256 {
            return Err(IntakeError::InputChanged {
                path: dep_path,
                expected_sha256: dep.sha256.clone(),
                actual_sha256: actual,
            });
        }
        external_bytes.insert(dep.declared_ref.clone(), bytes);
    }

    // 3. Reserve the snapshot directory atomically (no overwrite).
    let snapshots_dir = workspace_root.join("snapshots");
    std::fs::create_dir_all(&snapshots_dir).map_err(|source| IntakeError::Io {
        path: snapshots_dir.clone(),
        source,
    })?;
    let snapshot_dir = snapshots_dir.join(&plan.identity);
    match std::fs::create_dir(&snapshot_dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return reuse_existing(&snapshot_dir, plan);
        }
        Err(source) => {
            return Err(IntakeError::Io {
                path: snapshot_dir.clone(),
                source,
            })
        }
    }

    // 4. Materialize verified copies.
    let original_dir = snapshot_dir.join("original");
    let deps_dir = snapshot_dir.join("dependencies");
    std::fs::create_dir_all(&original_dir).map_err(|source| IntakeError::Io {
        path: original_dir.clone(),
        source,
    })?;
    std::fs::create_dir_all(&deps_dir).map_err(|source| IntakeError::Io {
        path: deps_dir.clone(),
        source,
    })?;

    write_verified(&original_dir.join(&plan.source.stored_name), &source_bytes)?;
    for dep in &plan.dependencies {
        let Some(stored_name) = &dep.stored_name else {
            continue;
        };
        let bytes: Vec<u8> = match &dep.kind {
            DependencyKind::ExternalFile => external_bytes
                .get(&dep.declared_ref)
                .cloned()
                .ok_or_else(|| IntakeError::SnapshotCorrupt {
                    dir: snapshot_dir.clone(),
                    reason: format!(
                        "internal: bytes for '{}' were not collected",
                        dep.declared_ref
                    ),
                })?,
            DependencyKind::EmbeddedDataUri => {
                let Some(uri) = &dep.source_uri else {
                    return Err(IntakeError::SnapshotCorrupt {
                        dir: snapshot_dir.clone(),
                        reason: format!("internal: data URI for '{}' is missing", dep.declared_ref),
                    });
                };
                let payload = uri.split_once(',').map(|(_, p)| p).unwrap_or(uri);
                let decoded =
                    crate::base64::decode(payload).map_err(|e| IntakeError::SnapshotCorrupt {
                        dir: snapshot_dir.clone(),
                        reason: format!(
                            "internal: data URI '{}' failed to decode: {e:?}",
                            dep.declared_ref
                        ),
                    })?;
                if sha256_hex(&decoded) != dep.sha256 {
                    return Err(IntakeError::SnapshotCorrupt {
                        dir: snapshot_dir.clone(),
                        reason: format!(
                            "internal: decoded data URI '{}' does not match its pinned hash",
                            dep.declared_ref
                        ),
                    });
                }
                decoded
            }
            DependencyKind::GlbBinChunk {
                byte_offset,
                byte_length,
            } => {
                let slice =
                    slice_checked(&source_bytes, *byte_offset, *byte_length, "GLB BIN chunk")?;
                slice.to_vec()
            }
            DependencyKind::GlbBufferView {
                byte_offset,
                byte_length,
            } => {
                let slice =
                    slice_checked(&source_bytes, *byte_offset, *byte_length, "GLB bufferView")?;
                slice.to_vec()
            }
            DependencyKind::ExternalBufferRange {
                declared_buffer_ref,
                byte_offset,
                byte_length,
            } => {
                let buffer_bytes = external_bytes.get(declared_buffer_ref).ok_or_else(|| {
                    IntakeError::SnapshotCorrupt {
                        dir: snapshot_dir.clone(),
                        reason: format!(
                            "internal: buffer '{declared_buffer_ref}' was not collected"
                        ),
                    }
                })?;
                let slice = slice_checked(
                    buffer_bytes,
                    *byte_offset,
                    *byte_length,
                    "external buffer range",
                )?;
                slice.to_vec()
            }
        };
        write_verified(&deps_dir.join(stored_name), &bytes)?;
    }

    // 5. Build and write the manifest.
    let manifest = SnapshotManifest {
        schema: SNAPSHOT_SCHEMA.to_string(),
        provisional: true,
        snapshot_id: plan.identity.clone(),
        tool: INTAKE_TOOL.to_string(),
        format: plan.format,
        source: plan.source.clone(),
        original_stored_name: plan.source.stored_name.clone(),
        provenance: plan.provenance.clone(),
        license_decision: plan.license_decision.clone(),
        dependencies: plan.dependencies.clone(),
        diagnostics: plan.diagnostics.clone(),
        status: plan.status,
        limits: plan.limits.clone(),
    };
    let manifest_path = snapshot_dir.join("manifest.json");
    let json = serde_json::to_vec_pretty(&manifest).expect("manifest serialization cannot fail");
    write_verified(&manifest_path, &json)?;

    Ok(StoredSnapshot {
        dir: snapshot_dir,
        manifest_path,
        manifest,
        reused: false,
    })
}

/// Verify an existing snapshot directory for the same identity and reuse it.
/// Any mismatch is a conflict; nothing is overwritten.
fn reuse_existing(snapshot_dir: &Path, plan: &IntakePlan) -> Result<StoredSnapshot, IntakeError> {
    let manifest_path = snapshot_dir.join("manifest.json");
    let raw = std::fs::read(&manifest_path).map_err(|source| IntakeError::SnapshotDirConflict {
        dir: snapshot_dir.to_path_buf(),
        reason: format!("existing directory has no readable manifest: {source}"),
    })?;
    // Two-step parse: `Value` first, then `from_value`, because the manifest
    // contains `&'static str` schema tags whose Deserialize impl requires a
    // 'static deserializer lifetime.
    let value: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|source| IntakeError::SnapshotDirConflict {
            dir: snapshot_dir.to_path_buf(),
            reason: format!("existing manifest is not a valid snapshot manifest: {source}"),
        })?;
    let manifest: SnapshotManifest =
        serde_json::from_value(value).map_err(|source| IntakeError::SnapshotDirConflict {
            dir: snapshot_dir.to_path_buf(),
            reason: format!("existing manifest is not a valid snapshot manifest: {source}"),
        })?;
    if manifest.snapshot_id != plan.identity {
        return Err(IntakeError::SnapshotDirConflict {
            dir: snapshot_dir.to_path_buf(),
            reason: format!(
                "existing manifest id {} does not match plan identity {}",
                manifest.snapshot_id, plan.identity
            ),
        });
    }
    if let Err(e) = verify_snapshot_integrity(snapshot_dir, &manifest) {
        return Err(match e {
            IntakeError::SnapshotCorrupt { .. } => e,
            other => IntakeError::SnapshotCorrupt {
                dir: snapshot_dir.to_path_buf(),
                reason: other.to_string(),
            },
        });
    }
    Ok(StoredSnapshot {
        dir: snapshot_dir.to_path_buf(),
        manifest_path,
        manifest,
        reused: true,
    })
}

/// Re-verify every stored copy of a snapshot against its manifest.
pub fn verify_snapshot_integrity(
    snapshot_dir: &Path,
    manifest: &SnapshotManifest,
) -> Result<(), IntakeError> {
    let source_copy = snapshot_dir
        .join("original")
        .join(&manifest.original_stored_name);
    verify_file(&source_copy, &manifest.source.sha256)?;
    for dep in &manifest.dependencies {
        let Some(stored_name) = &dep.stored_name else {
            continue;
        };
        let copy = snapshot_dir.join("dependencies").join(stored_name);
        verify_file(&copy, &dep.sha256)?;
    }
    Ok(())
}

fn verify_file(path: &Path, expected: &str) -> Result<(), IntakeError> {
    let bytes = std::fs::read(path).map_err(|source| IntakeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let actual = sha256_hex(&bytes);
    if actual != expected {
        return Err(IntakeError::CopyVerificationFailed {
            path: path.to_path_buf(),
            expected_sha256: expected.to_string(),
            actual_sha256: actual,
        });
    }
    Ok(())
}

/// Write bytes, then read the file back and verify the hash (defense in
/// depth against silent bit rot / partial writes).
fn write_verified(path: &Path, bytes: &[u8]) -> Result<(), IntakeError> {
    std::fs::write(path, bytes).map_err(|source| IntakeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let expected = sha256_hex(bytes);
    verify_file(path, &expected)
}
