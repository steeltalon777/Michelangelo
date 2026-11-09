//! Source format detection.
//!
//! The supported set is deliberately small: GLB, plain `.gltf` with relative
//! local dependencies, and OBJ with MTL/local textures. Anything else is
//! rejected with an explicit `UnsupportedFormat` error. In particular,
//! arbitrary `.blend` intake and remote URIs are NOT supported by this PoC
//! (ADR 0005 §2, §13.2: the safe `.blend` adapter is an open gate).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::IntakeError;

/// Supported source formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    Glb,
    Gltf,
    Obj,
}

impl SourceFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceFormat::Glb => "glb",
            SourceFormat::Gltf => "gltf",
            SourceFormat::Obj => "obj",
        }
    }

    /// Typical file extension used when naming embedded dependency copies.
    pub fn canonical_extension(&self) -> &'static str {
        self.as_str()
    }
}

/// GLB container magic ("glTF" followed by version 2 in the header).
pub const GLB_MAGIC: [u8; 4] = *b"glTF";

/// Detect the source format from the file extension plus content evidence.
///
/// Returns `IntakeError::UnsupportedFormat` for every extension outside the
/// supported set. `.glb` additionally requires the GLB magic bytes; a `.glb`
/// file without magic is a `FormatMismatch` diagnostic, not a silent pass.
pub fn detect_source_format(path: &Path) -> Result<SourceFormat, IntakeError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "glb" => {
            let head = read_head(path, 4)?;
            if head.as_slice() != GLB_MAGIC {
                return Err(IntakeError::UnsupportedFormat {
                    path: path.to_path_buf(),
                    reason: "file has .glb extension but does not carry the GLB magic bytes".into(),
                });
            }
            Ok(SourceFormat::Glb)
        }
        "gltf" => Ok(SourceFormat::Gltf),
        "obj" => Ok(SourceFormat::Obj),
        other => {
            let reason = match other {
                "blend" => "arbitrary .blend intake is not supported by this PoC; a safe \
                            .blend adapter is an open ADR 0005 implementation gate"
                    .to_string(),
                "" => "source has no recognizable format extension".to_string(),
                _ => format!(".{other} is not in the supported intake set (glb, gltf, obj)"),
            };
            Err(IntakeError::UnsupportedFormat {
                path: path.to_path_buf(),
                reason,
            })
        }
    }
}

fn read_head(path: &Path, n: usize) -> Result<Vec<u8>, IntakeError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|source| IntakeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut buf = vec![0u8; n];
    let mut read_total = 0;
    loop {
        let read = file
            .read(&mut buf[read_total..])
            .map_err(|source| IntakeError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        if read == 0 {
            break;
        }
        read_total += read;
        if read_total == n {
            break;
        }
    }
    buf.truncate(read_total);
    Ok(buf)
}
