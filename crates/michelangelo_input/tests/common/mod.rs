#![allow(dead_code)]

//! Shared helpers for the integration tests of the michelangelo_input crate.

use std::fs;
use std::path::{Path, PathBuf};

use michelangelo_input::{Diagnostic, InputPackageReport};
use tempfile::TempDir;

/// Root of the repository (the crate lives in <root>/crates/michelangelo_input).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repository root")
}

/// Path of a committed fixture package.
pub fn fixture_path(name: &str) -> PathBuf {
    repo_root()
        .join("examples")
        .join("input_packages")
        .join(name)
}

/// Copy a committed fixture into a temporary directory.
///
/// The package is placed in <temp>/pkg so that sibling files can be used to
/// test path traversal.
pub fn copy_fixture(name: &str) -> TempDir {
    let temp = TempDir::new().expect("temp dir");
    let target = package_dir(&temp);
    copy_tree(&fixture_path(name), &target);
    temp
}

/// Path of the package inside a temporary directory.
pub fn package_dir(temp: &TempDir) -> PathBuf {
    temp.path().join("pkg")
}

/// Recursively copy a directory tree.
pub fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("create directory");
    for entry in fs::read_dir(source).expect("read directory") {
        let entry = entry.expect("directory entry");
        let destination = target.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            fs::copy(entry.path(), &destination).expect("copy file");
        }
    }
}

/// Read the manifest of a package as JSON.
pub fn read_manifest(dir: &Path) -> serde_json::Value {
    let text = fs::read_to_string(dir.join("package.json")).expect("read manifest");
    serde_json::from_str(&text).expect("parse manifest")
}

/// Write a manifest from JSON.
pub fn write_manifest(dir: &Path, value: &serde_json::Value) {
    let text = serde_json::to_string_pretty(value).expect("serialize manifest");
    fs::write(dir.join("package.json"), text).expect("write manifest");
}

/// SHA-256 of a file as lowercase hexadecimal.
pub fn sha256_of(path: &Path) -> String {
    michelangelo_input::sha256::sha256_hex(&fs::read(path).expect("read file"))
}

/// All diagnostic codes of a report.
pub fn codes(report: &InputPackageReport) -> Vec<String> {
    report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.clone())
        .collect()
}

/// True when the report contains a diagnostic with the given code.
pub fn has_code(report: &InputPackageReport, code: &str) -> bool {
    report.diagnostics.iter().any(|d| d.code == code)
}

/// All diagnostics with the given code.
pub fn diagnostics_with_code<'a>(
    report: &'a InputPackageReport,
    code: &'a str,
) -> Vec<&'a Diagnostic> {
    report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == code)
        .collect()
}

/// CRC-32 (IEEE) of a byte slice.
pub fn crc32_of(bytes: &[u8]) -> u32 {
    michelangelo_input::crc32::crc32(bytes)
}

/// Adler-32 checksum, used by the zlib container of PNG files.
pub fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// Build a PNG chunk with a correct CRC.
pub fn png_chunk(chunk_type: &[u8], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    let mut crc_input = chunk_type.to_vec();
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32_of(&crc_input).to_be_bytes());
    out
}

/// Wrap data in a zlib stream that uses uncompressed deflate blocks.
pub fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    if data.is_empty() {
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xff, 0xff]);
    } else {
        let mut offset = 0;
        while offset < data.len() {
            let length = std::cmp::min(65_535, data.len() - offset);
            let final_block = offset + length >= data.len();
            out.push(u8::from(final_block));
            out.extend_from_slice(&(length as u16).to_le_bytes());
            out.extend_from_slice(&(!(length as u16)).to_le_bytes());
            out.extend_from_slice(&data[offset..offset + length]);
            offset += length;
        }
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// Build a PNG image with the given header fields.
pub fn build_png(width: u32, height: u32, bit_depth: u8, color_type: u8, interlace: u8) -> Vec<u8> {
    let mut out = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(bit_depth);
    ihdr.push(color_type);
    ihdr.push(0);
    ihdr.push(0);
    ihdr.push(interlace);
    out.extend_from_slice(&png_chunk(b"IHDR", &ihdr));

    let channels = match color_type {
        0 | 3 => 1usize,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => 1,
    };
    let mut raw = Vec::new();
    for _ in 0..height {
        raw.push(0);
        raw.extend(std::iter::repeat_n(0u8, width as usize * channels));
    }
    out.extend_from_slice(&png_chunk(b"IDAT", &zlib_stored(&raw)));
    out.extend_from_slice(&png_chunk(b"IEND", &[]));
    out
}
