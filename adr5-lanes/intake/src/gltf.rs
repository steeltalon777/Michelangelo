//! glTF 2.0 / GLB reference scanning.
//!
//! The scanner extracts dependency *references* from the JSON part:
//! external buffer/image URIs, `data:` URI payloads, GLB BIN-chunk
//! embeddings and bufferView ranges. Mesh/animation content is not
//! interpreted: intake reads and pins bytes, it does not validate geometry.
//!
//! GLB container rules checked here: magic, version, chunk type order,
//! chunk lengths vs declared total length (truncation), and buffer/view
//! ranges against the BIN chunk size.

use serde::Deserialize;

use crate::base64;
use crate::error::IntakeError;
use crate::format::GLB_MAGIC;
use crate::report::{Diagnostic, DiagnosticCode};

/// Minimal serde model of the glTF JSON we care about. All other fields are
/// ignored (glTF documents are extensible; unknown optional fields do not
/// affect the dependency set we pin).
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GltfDocument {
    #[serde(default)]
    pub asset: GltfAsset,
    #[serde(default)]
    pub extensions_used: Vec<String>,
    #[serde(default)]
    pub extensions_required: Vec<String>,
    #[serde(default)]
    pub buffers: Vec<GltfBuffer>,
    #[serde(default)]
    pub buffer_views: Vec<GltfBufferView>,
    #[serde(default)]
    pub images: Vec<GltfImage>,
}

#[derive(Debug, Deserialize, Default)]
pub struct GltfAsset {
    #[serde(default)]
    pub version: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GltfBuffer {
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub byte_length: u64,
    #[serde(default)]
    pub extensions: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GltfBufferView {
    #[serde(default)]
    pub buffer: u32,
    #[serde(default)]
    pub byte_offset: u64,
    #[serde(default)]
    pub byte_length: u64,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GltfImage {
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub buffer_view: Option<u64>,
    #[serde(default)]
    pub extensions: Option<serde_json::Map<String, serde_json::Value>>,
}

/// A decoded GLB BIN chunk range within the original file bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlbBinRange {
    pub byte_offset: u64,
    pub byte_length: u64,
}

/// One dependency reference discovered in the glTF JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GltfRef {
    /// External file reference (buffer or image URI). `buffer_index` is set
    /// for buffer-role references so bufferViews can be range-checked
    /// against the pinned buffer.
    External {
        uri: String,
        role: RefRole,
        buffer_index: Option<u32>,
    },
    /// Inline `data:<...>;base64,<payload>` URI, already decoded; the raw
    /// URI is carried so the payload can be re-derived deterministically.
    DataUri {
        decoded: Vec<u8>,
        role: RefRole,
        raw_uri: String,
    },
    /// The single GLB-embedded buffer (BIN chunk).
    GlbBin,
    /// A view into a buffer (embedded or external — resolved by the caller
    /// using `embedded_buffer_indices`).
    BufferView {
        buffer_index: u32,
        byte_offset: u64,
        byte_length: u64,
        role: RefRole,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefRole {
    Buffer,
    Image,
}

impl RefRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            RefRole::Buffer => "buffer",
            RefRole::Image => "image",
        }
    }
}

pub struct GltfScan {
    pub refs: Vec<GltfRef>,
    pub bin_range: Option<GlbBinRange>,
    pub diagnostics: Vec<Diagnostic>,
    /// Buffer indices that are embedded (no uri). Empty for plain `.gltf`.
    pub embedded_buffer_indices: Vec<u32>,
}

/// Parse and scan a plain `.gltf` JSON document.
pub fn scan_gltf_json(bytes: &[u8]) -> Result<GltfScan, IntakeError> {
    let doc: GltfDocument =
        serde_json::from_slice(bytes).map_err(|e| IntakeError::MalformedGlb {
            code: DiagnosticCode::GlbChunkMismatch,
            reason: format!(".gltf document is not valid JSON: {e}"),
        })?;
    let mut diagnostics = Vec::new();
    check_version_and_extensions(&doc, &mut diagnostics);
    let mut refs = Vec::new();

    for (index, buffer) in doc.buffers.iter().enumerate() {
        if buffer_has_extension(buffer) {
            push_extension_error("buffers", index, &mut diagnostics);
            continue;
        }
        match &buffer.uri {
            None => {
                diagnostics.push(Diagnostic::error(
                    DiagnosticCode::GlbChunkMismatch,
                    Some(format!("buffers[{index}]")),
                    "buffer without uri in a plain .gltf document; only GLB containers may embed buffers",
                ));
            }
            Some(uri) => push_uri_ref_indexed(
                uri,
                RefRole::Buffer,
                Some(index as u32),
                &mut refs,
                &mut diagnostics,
            ),
        }
    }
    collect_image_refs(&doc, &mut refs, &mut diagnostics);
    Ok(GltfScan {
        refs,
        bin_range: None,
        diagnostics,
        embedded_buffer_indices: Vec::new(),
    })
}

/// Parse and scan a GLB container. Structural problems are returned as
/// `IntakeError::MalformedGlb` carrying the diagnostic code to record.
pub fn scan_glb(bytes: &[u8]) -> Result<GltfScan, IntakeError> {
    // 12-byte header + 8-byte first chunk header.
    if bytes.len() < 20 {
        return Err(malformed(
            DiagnosticCode::TruncatedGlb,
            "file shorter than the GLB header",
        ));
    }
    if bytes[0..4] != GLB_MAGIC {
        return Err(malformed(
            DiagnosticCode::FormatMismatch,
            "missing glTF magic",
        ));
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    if version != 2 {
        return Err(malformed(
            DiagnosticCode::UnsupportedVersion,
            "GLB version is not 2",
        ));
    }
    let declared_total = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as u64;
    let actual_total = bytes.len() as u64;
    if declared_total > actual_total {
        return Err(malformed(
            DiagnosticCode::TruncatedGlb,
            format!("declared total length {declared_total} exceeds file size {actual_total}"),
        ));
    }
    if declared_total < actual_total {
        return Err(malformed(
            DiagnosticCode::GlbChunkMismatch,
            format!("file has {actual_total} bytes but header declares {declared_total}"),
        ));
    }

    let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as u64;
    let json_type = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
    const JSON_CHUNK: u32 = 0x4E4F_534A; // "JSON"
    const BIN_CHUNK: u32 = 0x004E_4942; // "BIN\0"
    if json_type != JSON_CHUNK {
        return Err(malformed(
            DiagnosticCode::GlbChunkMismatch,
            "first chunk is not JSON",
        ));
    }
    let json_end = 20u64.checked_add(json_len).ok_or_else(|| {
        malformed(
            DiagnosticCode::GlbChunkMismatch,
            "JSON chunk length overflows",
        )
    })?;
    if json_end > actual_total {
        return Err(malformed(
            DiagnosticCode::TruncatedGlb,
            format!("JSON chunk extends to {json_end} but file has {actual_total} bytes"),
        ));
    }
    let json_bytes = &bytes[20..json_end as usize];

    let mut offset = json_end;
    let mut bin_range: Option<GlbBinRange> = None;
    while offset < actual_total {
        if offset + 8 > actual_total {
            return Err(malformed(
                DiagnosticCode::TruncatedGlb,
                "truncated chunk header at end of file",
            ));
        }
        let chunk_len = u32::from_le_bytes(
            bytes[offset as usize..(offset + 4) as usize]
                .try_into()
                .unwrap(),
        ) as u64;
        let chunk_type = u32::from_le_bytes(
            bytes[(offset + 4) as usize..(offset + 8) as usize]
                .try_into()
                .unwrap(),
        );
        let data_start = offset + 8;
        let data_end = data_start
            .checked_add(chunk_len)
            .ok_or_else(|| malformed(DiagnosticCode::GlbChunkMismatch, "chunk length overflows"))?;
        if data_end > actual_total {
            return Err(malformed(
                DiagnosticCode::TruncatedGlb,
                format!("chunk extends to {data_end} but file has {actual_total} bytes"),
            ));
        }
        match chunk_type {
            BIN_CHUNK => {
                if bin_range.is_some() {
                    return Err(malformed(
                        DiagnosticCode::GlbChunkMismatch,
                        "multiple BIN chunks",
                    ));
                }
                bin_range = Some(GlbBinRange {
                    byte_offset: data_start,
                    byte_length: chunk_len,
                });
            }
            _ => {
                return Err(malformed(
                    DiagnosticCode::GlbChunkMismatch,
                    format!("unexpected chunk type 0x{chunk_type:08X}"),
                ));
            }
        }
        offset = data_end;
    }

    let doc: GltfDocument = serde_json::from_slice(json_bytes).map_err(|e| {
        malformed(
            DiagnosticCode::GlbChunkMismatch,
            format!("JSON chunk is not valid glTF JSON: {e}"),
        )
    })?;
    let mut diagnostics = Vec::new();
    check_version_and_extensions(&doc, &mut diagnostics);

    let mut refs = Vec::new();
    let mut embedded_buffer_indices = Vec::new();
    for (index, buffer) in doc.buffers.iter().enumerate() {
        if buffer_has_extension(buffer) {
            push_extension_error("buffers", index, &mut diagnostics);
            continue;
        }
        match &buffer.uri {
            None => {
                let Some(bin) = bin_range else {
                    diagnostics.push(Diagnostic::error(
                        DiagnosticCode::GlbChunkMismatch,
                        Some(format!("buffers[{index}]")),
                        "buffer has no uri but the GLB has no BIN chunk",
                    ));
                    continue;
                };
                if buffer.byte_length > bin.byte_length {
                    diagnostics.push(Diagnostic::error(
                        DiagnosticCode::TruncatedGlb,
                        Some(format!("buffers[{index}]")),
                        format!(
                            "buffer byteLength {} exceeds BIN chunk size {}",
                            buffer.byte_length, bin.byte_length
                        ),
                    ));
                    continue;
                }
                if !embedded_buffer_indices.is_empty() {
                    diagnostics.push(Diagnostic::error(
                        DiagnosticCode::GlbChunkMismatch,
                        Some(format!("buffers[{index}]")),
                        "more than one buffer without uri; GLB may embed only the first buffer",
                    ));
                    continue;
                }
                embedded_buffer_indices.push(index as u32);
                refs.push(GltfRef::GlbBin);
            }
            Some(uri) => push_uri_ref_indexed(
                uri,
                RefRole::Buffer,
                Some(index as u32),
                &mut refs,
                &mut diagnostics,
            ),
        }
    }
    collect_image_refs(&doc, &mut refs, &mut diagnostics);
    Ok(GltfScan {
        refs,
        bin_range,
        diagnostics,
        embedded_buffer_indices,
    })
}

fn buffer_has_extension(buffer: &GltfBuffer) -> bool {
    !buffer.extensions.as_ref().is_none_or(|m| m.is_empty())
}

fn push_extension_error(kind: &str, index: usize, diagnostics: &mut Vec<Diagnostic>) {
    diagnostics.push(Diagnostic::error(
        DiagnosticCode::UnsupportedBufferExtension,
        Some(format!("{kind}[{index}]")),
        "payload encoding is modified by a glTF extension; intake cannot pin the effective bytes",
    ));
}

fn check_version_and_extensions(doc: &GltfDocument, diagnostics: &mut Vec<Diagnostic>) {
    if !doc.asset.version.starts_with('2') {
        diagnostics.push(Diagnostic::error(
            DiagnosticCode::UnsupportedVersion,
            None,
            format!("glTF asset version '{}' is not 2.x", doc.asset.version),
        ));
    }
    for ext in &doc.extensions_required {
        diagnostics.push(Diagnostic::error(
            DiagnosticCode::UnsupportedRequiredExtension,
            Some(ext.clone()),
            "required glTF extension is not supported by this intake PoC",
        ));
    }
}

fn push_uri_ref(
    uri: &str,
    role: RefRole,
    refs: &mut Vec<GltfRef>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    push_uri_ref_indexed(uri, role, None, refs, diagnostics);
}

fn push_uri_ref_indexed(
    uri: &str,
    role: RefRole,
    buffer_index: Option<u32>,
    refs: &mut Vec<GltfRef>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(payload) = try_data_uri(uri, role, diagnostics) {
        refs.push(payload);
    } else {
        refs.push(GltfRef::External {
            uri: uri.to_string(),
            role,
            buffer_index,
        });
    }
}

fn collect_image_refs(
    doc: &GltfDocument,
    refs: &mut Vec<GltfRef>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (index, image) in doc.images.iter().enumerate() {
        if !image.extensions.as_ref().is_none_or(|m| m.is_empty()) {
            push_extension_error("images", index, diagnostics);
            continue;
        }
        if let Some(uri) = &image.uri {
            push_uri_ref(uri, RefRole::Image, refs, diagnostics);
            continue;
        }
        let Some(view_index) = image.buffer_view else {
            diagnostics.push(Diagnostic::error(
                DiagnosticCode::GlbChunkMismatch,
                Some(format!("images[{index}]")),
                "image has neither uri nor bufferView",
            ));
            continue;
        };
        let Some(view) = doc.buffer_views.get(view_index as usize) else {
            diagnostics.push(Diagnostic::error(
                DiagnosticCode::GlbChunkMismatch,
                Some(format!("images[{index}]")),
                format!("image references bufferView {view_index} which does not exist"),
            ));
            continue;
        };
        let Some(buffer) = doc.buffers.get(view.buffer as usize) else {
            diagnostics.push(Diagnostic::error(
                DiagnosticCode::GlbChunkMismatch,
                Some(format!("images[{index}]")),
                format!(
                    "bufferView {} references buffer {} which does not exist",
                    view_index, view.buffer
                ),
            ));
            continue;
        };
        if buffer.uri.is_some() {
            refs.push(GltfRef::BufferView {
                buffer_index: view.buffer,
                byte_offset: view.byte_offset,
                byte_length: view.byte_length,
                role: RefRole::Image,
            });
        } else {
            // Embedded buffer: resolved by the caller against the GLB BIN
            // chunk via `embedded_buffer_indices` + `bin_range`. In a plain
            // `.gltf` this shape was already diagnosed as an error by the
            // buffer loop above.
            refs.push(GltfRef::BufferView {
                buffer_index: view.buffer,
                byte_offset: view.byte_offset,
                byte_length: view.byte_length,
                role: RefRole::Image,
            });
        }
    }
}

fn try_data_uri(uri: &str, role: RefRole, diagnostics: &mut Vec<Diagnostic>) -> Option<GltfRef> {
    let rest = uri.strip_prefix("data:")?;
    let payload = rest.split_once(',').map(|(_, p)| p).unwrap_or(rest);
    match base64::decode(payload) {
        Ok(decoded) => Some(GltfRef::DataUri {
            decoded,
            role,
            raw_uri: uri.to_string(),
        }),
        Err(e) => {
            diagnostics.push(Diagnostic::error(
                DiagnosticCode::DataUriError,
                Some(truncate_subject(uri)),
                format!("data: URI payload could not be base64-decoded: {e:?}"),
            ));
            None
        }
    }
}

fn truncate_subject(s: &str) -> String {
    if s.len() > 80 {
        format!("{}...", &s[..80])
    } else {
        s.to_string()
    }
}

fn malformed(code: DiagnosticCode, reason: impl Into<String>) -> IntakeError {
    IntakeError::MalformedGlb {
        code,
        reason: reason.into(),
    }
}
