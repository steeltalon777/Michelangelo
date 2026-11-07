//! Minimal PNG container parser.
//!
//! Only the container is inspected: signature, chunk structure, IHDR and IEND
//! presence. Pixel data is never decoded (that is Stage B / solver work).
//!
//! The parser implements the input-package profile: bit depth 8 only, and the
//! color types are checked per resource kind by the caller
//! (see [is_image_color_type] and [is_mask_color_type]).

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::crc32::crc32;

/// The 8 byte PNG file signature.
pub const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// The PNG chunks that are critical (uppercase first letter) in this profile.
const KNOWN_CRITICAL_CHUNKS: [&str; 4] = ["IHDR", "PLTE", "IDAT", "IEND"];

/// Header information of a PNG image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PngHeader {
    /// Image width in pixels (IHDR).
    pub width: u32,
    /// Image height in pixels (IHDR).
    pub height: u32,
    /// Bits per sample (IHDR); always 8 in this profile.
    pub bit_depth: u8,
    /// PNG color type (IHDR).
    pub color_type: u8,
    /// Interlace method (IHDR); 0 (none) or 1 (Adam7).
    pub interlace: u8,
}

/// Errors produced by the PNG container parser.
#[derive(Debug)]
pub enum PngError {
    /// The file could not be read.
    Io(io::Error),
    /// The file does not start with the PNG signature.
    Signature,
    /// The file ends in the middle of a chunk.
    Truncated,
    /// The IHDR chunk is missing or malformed.
    IhdrInvalid(String),
    /// The bit depth is not 8.
    BitDepth(u8),
    /// The color type is not a valid PNG color type.
    ColorType(u8),
    /// The file has no IEND chunk.
    MissingIend,
    /// The file contains a critical chunk that is not known to this profile.
    UnknownCriticalChunk(String),
    /// The CRC of a chunk does not match its content.
    CrcMismatch(String),
}

impl fmt::Display for PngError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PngError::Io(err) => write!(f, "cannot read PNG: {err}"),
            PngError::Signature => write!(f, "not a PNG file (bad signature)"),
            PngError::Truncated => write!(f, "truncated PNG file"),
            PngError::IhdrInvalid(msg) => write!(f, "invalid PNG IHDR chunk: {msg}"),
            PngError::BitDepth(depth) => {
                write!(f, "unsupported PNG bit depth {depth}, expected 8")
            }
            PngError::ColorType(color_type) => {
                write!(f, "unsupported PNG color type {color_type}")
            }
            PngError::MissingIend => write!(f, "PNG file has no IEND chunk"),
            PngError::UnknownCriticalChunk(name) => {
                write!(f, "unknown critical PNG chunk '{name}'")
            }
            PngError::CrcMismatch(name) => {
                write!(f, "CRC mismatch in PNG chunk '{name}'")
            }
        }
    }
}

impl std::error::Error for PngError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PngError::Io(err) => Some(err),
            _ => None,
        }
    }
}

/// True when the color type is accepted for package images (RGB or RGBA).
pub fn is_image_color_type(color_type: u8) -> bool {
    color_type == 2 || color_type == 6
}

/// True when the color type is accepted for package masks (grayscale or RGBA).
pub fn is_mask_color_type(color_type: u8) -> bool {
    color_type == 0 || color_type == 6
}

/// Parse the header of a PNG image held in memory.
pub fn parse_png_header(bytes: &[u8]) -> Result<PngHeader, PngError> {
    if bytes.len() < PNG_SIGNATURE.len() {
        let common = bytes.len();
        if bytes != &PNG_SIGNATURE[..common] {
            return Err(PngError::Signature);
        }
        return Err(PngError::Truncated);
    }
    if bytes[..PNG_SIGNATURE.len()] != PNG_SIGNATURE {
        return Err(PngError::Signature);
    }

    let mut pos = PNG_SIGNATURE.len();
    let mut header: Option<PngHeader> = None;
    let mut seen_iend = false;

    while pos < bytes.len() {
        if pos + 8 > bytes.len() {
            return Err(PngError::Truncated);
        }
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
            as usize;
        let name_bytes = &bytes[pos + 4..pos + 8];
        let name = String::from_utf8_lossy(name_bytes).to_string();

        let data_start = pos + 8;
        let data_end = match data_start.checked_add(len) {
            Some(end) => end,
            None => return Err(PngError::Truncated),
        };
        if data_end + 4 > bytes.len() {
            return Err(PngError::Truncated);
        }

        let is_critical = name_bytes
            .first()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false);
        if is_critical && !KNOWN_CRITICAL_CHUNKS.contains(&name.as_str()) {
            return Err(PngError::UnknownCriticalChunk(name));
        }

        if header.is_none() {
            if name != "IHDR" {
                return Err(PngError::IhdrInvalid(format!(
                    "first chunk is '{name}', expected 'IHDR'"
                )));
            }
            // The checksum of the header chunk is verified; the content of the
            // remaining chunks is covered by the SHA-256 of the manifest.
            let expected_crc = u32::from_be_bytes([
                bytes[data_end],
                bytes[data_end + 1],
                bytes[data_end + 2],
                bytes[data_end + 3],
            ]);
            let actual_crc = crc32(&bytes[pos + 4..data_end]);
            if expected_crc != actual_crc {
                return Err(PngError::CrcMismatch(name));
            }
            if len != 13 {
                return Err(PngError::IhdrInvalid(format!(
                    "IHDR length is {len}, expected 13"
                )));
            }
            let data = &bytes[data_start..data_end];
            let width = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
            let height = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
            let bit_depth = data[8];
            let color_type = data[9];
            let compression = data[10];
            let filter = data[11];
            let interlace = data[12];

            if width == 0 || height == 0 {
                return Err(PngError::IhdrInvalid(
                    "image dimensions must be non-zero".to_string(),
                ));
            }
            if compression != 0 {
                return Err(PngError::IhdrInvalid(format!(
                    "compression method {compression} is not defined"
                )));
            }
            if filter != 0 {
                return Err(PngError::IhdrInvalid(format!(
                    "filter method {filter} is not defined"
                )));
            }
            if interlace > 1 {
                return Err(PngError::IhdrInvalid(format!(
                    "interlace method {interlace} is not defined"
                )));
            }
            if !matches!(color_type, 0 | 2 | 3 | 4 | 6) {
                return Err(PngError::ColorType(color_type));
            }
            if bit_depth != 8 {
                return Err(PngError::BitDepth(bit_depth));
            }

            header = Some(PngHeader {
                width,
                height,
                bit_depth,
                color_type,
                interlace,
            });
        }

        if name == "IEND" {
            seen_iend = true;
            break;
        }
        pos = data_end + 4;
    }

    let header = header.ok_or_else(|| PngError::IhdrInvalid("missing IHDR chunk".to_string()))?;
    if !seen_iend {
        return Err(PngError::MissingIend);
    }
    Ok(header)
}

/// Read the header of a PNG file from disk.
pub fn read_png_header(path: &Path) -> Result<PngHeader, PngError> {
    let bytes = fs::read(path).map_err(PngError::Io)?;
    parse_png_header(&bytes)
}
