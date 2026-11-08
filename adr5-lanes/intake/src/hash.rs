//! Hashing helpers.
//!
//! Reuses the in-repo FIPS 180-4 SHA-256 implementation from
//! `michelangelo_input` (path dependency, read-only reuse; that crate is not
//! modified by this stream). All hashes in this crate are lowercase hex
//! SHA-256 over the bytes that were *actually read*, never over manifest
//! declarations (IT-8D invariant).

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    michelangelo_input::sha256::sha256(bytes)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    michelangelo_input::sha256::sha256_hex(bytes)
}

/// Deterministic operation identity: SHA-256 over a canonical textual
/// description of the intake operation. Absolute paths and timestamps are
/// deliberately excluded so that identical content yields identical identity
/// on any machine.
pub fn identity_digest(canonical: &str) -> String {
    sha256_hex(canonical.as_bytes())
}
