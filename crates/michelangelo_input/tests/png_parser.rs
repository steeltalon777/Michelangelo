//! PNG container parser tests.

mod common;

use std::fs;

use common::{build_png, crc32_of, png_chunk, zlib_stored};
use michelangelo_input::png::{self, PngError};

#[test]
fn parses_a_valid_rgb8_png() {
    let bytes = build_png(8, 8, 8, 2, 0);
    let header = png::parse_png_header(&bytes).expect("valid png");
    assert_eq!(header.width, 8);
    assert_eq!(header.height, 8);
    assert_eq!(header.bit_depth, 8);
    assert_eq!(header.color_type, 2);
    assert_eq!(header.interlace, 0);
}

#[test]
fn parses_a_valid_rgba8_png() {
    let bytes = build_png(16, 4, 8, 6, 0);
    let header = png::parse_png_header(&bytes).expect("valid png");
    assert_eq!(header.width, 16);
    assert_eq!(header.height, 4);
    assert_eq!(header.color_type, 6);
}

#[test]
fn parses_a_valid_gray8_png() {
    let bytes = build_png(4, 4, 8, 0, 0);
    let header = png::parse_png_header(&bytes).expect("valid png");
    assert_eq!(header.color_type, 0);
    assert_eq!(header.bit_depth, 8);
}

#[test]
fn rejects_a_jpeg_file() {
    let bytes = vec![0xFFu8, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46];
    match png::parse_png_header(&bytes) {
        Err(PngError::Signature) => {}
        Err(other) => panic!("unexpected error: {other}"),
        Ok(_) => panic!("a JPEG file must not be accepted"),
    }
}

#[test]
fn rejects_a_wrong_bit_depth() {
    let bytes = build_png(8, 8, 16, 2, 0);
    match png::parse_png_header(&bytes) {
        Err(PngError::BitDepth(16)) => {}
        Err(other) => panic!("unexpected error: {other}"),
        Ok(_) => panic!("a 16 bit PNG must not be accepted"),
    }
}

#[test]
fn rejects_a_truncated_file() {
    let mut bytes = build_png(8, 8, 8, 2, 0);
    bytes.truncate(bytes.len() - 6);
    match png::parse_png_header(&bytes) {
        Err(PngError::Truncated) => {}
        Err(other) => panic!("unexpected error: {other}"),
        Ok(_) => panic!("a truncated PNG must not be accepted"),
    }
}

#[test]
fn rejects_a_file_without_iend() {
    let mut bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&8u32.to_be_bytes());
    ihdr.extend_from_slice(&8u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    bytes.extend_from_slice(&png_chunk(b"IHDR", &ihdr));
    bytes.extend_from_slice(&png_chunk(b"IDAT", &zlib_stored(&[0u8; 16])));
    match png::parse_png_header(&bytes) {
        Err(PngError::MissingIend) => {}
        Err(other) => panic!("unexpected error: {other}"),
        Ok(_) => panic!("a PNG without IEND must not be accepted"),
    }
}

#[test]
fn rejects_a_bad_ihdr_length() {
    let mut bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&png_chunk(b"IHDR", &[0u8; 12]));
    match png::parse_png_header(&bytes) {
        Err(PngError::IhdrInvalid(_)) => {}
        Err(other) => panic!("unexpected error: {other}"),
        Ok(_) => panic!("a short IHDR must not be accepted"),
    }
}

#[test]
fn rejects_a_zero_dimension() {
    let bytes = build_png(0, 8, 8, 2, 0);
    match png::parse_png_header(&bytes) {
        Err(PngError::IhdrInvalid(_)) => {}
        Err(other) => panic!("unexpected error: {other}"),
        Ok(_) => panic!("a zero sized PNG must not be accepted"),
    }
}

#[test]
fn rejects_an_unknown_critical_chunk() {
    let mut bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&8u32.to_be_bytes());
    ihdr.extend_from_slice(&8u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    bytes.extend_from_slice(&png_chunk(b"IHDR", &ihdr));
    bytes.extend_from_slice(&png_chunk(b"ABCD", &[1, 2, 3]));
    match png::parse_png_header(&bytes) {
        Err(PngError::UnknownCriticalChunk(name)) => assert_eq!(name, "ABCD"),
        Err(other) => panic!("unexpected error: {other}"),
        Ok(_) => panic!("an unknown critical chunk must not be accepted"),
    }
}

#[test]
fn allows_an_unknown_ancillary_chunk() {
    let mut bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&8u32.to_be_bytes());
    ihdr.extend_from_slice(&8u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    bytes.extend_from_slice(&png_chunk(b"IHDR", &ihdr));
    bytes.extend_from_slice(&png_chunk(b"abCd", &[1, 2, 3]));
    bytes.extend_from_slice(&png_chunk(b"IDAT", &zlib_stored(&[0u8; 16])));
    bytes.extend_from_slice(&png_chunk(b"IEND", &[]));
    let header = png::parse_png_header(&bytes).expect("ancillary chunks are allowed");
    assert_eq!(header.width, 8);
}

#[test]
fn detects_a_broken_chunk_checksum() {
    let mut bytes = build_png(8, 8, 8, 2, 0);
    // The checksum of the IHDR chunk follows the 13 byte header data.
    bytes[29] ^= 0xFF;
    match png::parse_png_header(&bytes) {
        Err(PngError::CrcMismatch(_)) => {}
        Err(other) => panic!("unexpected error: {other}"),
        Ok(_) => panic!("a broken checksum must not be accepted"),
    }
}

#[test]
fn reads_a_png_header_from_disk() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let path = temp.path().join("image.png");
    fs::write(&path, build_png(12, 6, 8, 2, 0)).expect("write png");
    let header = png::read_png_header(&path).expect("header");
    assert_eq!(header.width, 12);
    assert_eq!(header.height, 6);
}

#[test]
fn reports_a_missing_file_as_an_io_error() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let path = temp.path().join("missing.png");
    match png::read_png_header(&path) {
        Err(PngError::Io(_)) => {}
        Err(other) => panic!("unexpected error: {other}"),
        Ok(_) => panic!("a missing file must not be accepted"),
    }
}

#[test]
fn color_type_helpers_follow_the_contract() {
    assert!(png::is_image_color_type(2));
    assert!(png::is_image_color_type(6));
    assert!(!png::is_image_color_type(0));
    assert!(!png::is_image_color_type(4));

    assert!(png::is_mask_color_type(0));
    assert!(png::is_mask_color_type(6));
    assert!(!png::is_mask_color_type(2));
    assert!(!png::is_mask_color_type(3));
}

#[test]
fn crc32_of_png_chunks_matches_the_reference_implementation() {
    // The CRC of the chunk type and data is stored in the chunk trailer.
    let chunk = png_chunk(b"IEND", &[]);
    let stored = u32::from_be_bytes([chunk[8], chunk[9], chunk[10], chunk[11]]);
    assert_eq!(stored, crc32_of(b"IEND"));
}
