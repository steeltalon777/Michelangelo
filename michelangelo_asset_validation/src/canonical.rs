//! Canonical encoding and aggregate digests (contract gate §3, draft-0.1).
//!
//! Canonical JSON: UTF-8, sorted keys (byte-wise), compact separators,
//! shortest round-trip numbers (serde_json `Number` display). This follows
//! the RFC 8785 (JCS) direction chosen by the contract gate; the exact JCS
//! number formatting for exotic values (negative zero, huge exponents) is
//! pending owner decision OP-3, so the encoder is marked provisional. All
//! fixtures in this crate use plain decimal values where both encoders agree.

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::ValidationError;

/// SHA-256 of the bytes, lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(64);
    for byte in digest {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

/// Canonical JSON text of a value: recursively sorted keys, compact.
pub fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => out.push_str(&number.to_string()),
        Value::String(text) => {
            let encoded = serde_json::to_string(text).unwrap_or_default();
            out.push_str(&encoded);
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                let encoded = serde_json::to_string(key).unwrap_or_default();
                out.push_str(&encoded);
                out.push(':');
                write_canonical(&map[*key], out);
            }
            out.push('}');
        }
    }
}

/// SHA-256 over the canonical JSON of the value.
pub fn digest_value(value: &Value) -> String {
    sha256_hex(canonical_json(value).as_bytes())
}

/// SHA-256 over the canonical JSON of a serializable document.
pub fn digest_document<T: serde::Serialize>(document: &T) -> Result<String, ValidationError> {
    let value = serde_json::to_value(document).map_err(|err| {
        ValidationError::rule(
            crate::error::DOCUMENT_INVALID,
            format!("the document cannot be canonicalized: {err}"),
        )
    })?;
    Ok(digest_value(&value))
}

/// The aggregate `asset_digest` (contract gate §3.4): SHA-256 over the UTF-8
/// lines `<artifact_id>\u0000<path>\u0000<content_kind>\u0000<sha256>\n`,
/// sorted byte-wise by `(artifact_id, path)`.
pub fn asset_digest(entries: &[(String, String, String, String)]) -> String {
    let mut sorted: Vec<&(String, String, String, String)> = entries.iter().collect();
    sorted.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    let mut hasher = Sha256::new();
    for (artifact_id, path, kind, sha) in sorted {
        let line = format!("{artifact_id}\u{0}{path}\u{0}{kind}\u{0}{sha}\n");
        hasher.update(line.as_bytes());
    }
    let digest = hasher.finalize();
    let mut text = String::with_capacity(64);
    for byte in digest {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

/// The aggregate `dependency_digest` (contract gate §3.6): SHA-256 over the
/// UTF-8 lines `<dep_id>\u0000<revision>\u0000<sha256>\n`, sorted by `dep_id`.
pub fn dependency_digest(entries: &[(String, String, String)]) -> String {
    let mut sorted: Vec<&(String, String, String)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = Sha256::new();
    for (dep_id, revision, sha) in sorted {
        let line = format!("{dep_id}\u{0}{revision}\u{0}{sha}\n");
        hasher.update(line.as_bytes());
    }
    let digest = hasher.finalize();
    let mut text = String::with_capacity(64);
    for byte in digest {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

/// Parse bytes as JSON.
pub fn parse_json(bytes: &[u8], what: &str) -> Result<Value, ValidationError> {
    serde_json::from_slice(bytes).map_err(|err| {
        ValidationError::rule(
            crate::error::DOCUMENT_INVALID,
            format!("{what} is not usable JSON: {err}"),
        )
    })
}

/// Serialize a document as pretty JSON with a trailing newline.
pub fn to_pretty_json<T: serde::Serialize>(document: &T) -> Result<Vec<u8>, ValidationError> {
    let text = serde_json::to_string_pretty(document).map_err(|err| {
        ValidationError::rule(
            crate::error::DOCUMENT_INVALID,
            format!("the document cannot be serialized: {err}"),
        )
    })?;
    Ok(format!("{text}\n").into_bytes())
}
