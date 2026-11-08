//! Identifier and path token rules of the contract gate (draft-0.1).
//!
//! The rules mirror `schemas/adr5/*.schema.json` (`idToken`, `token32`,
//! `relPath`). They are enforced before any filesystem access so that a
//! hostile path never reaches `Path::join`.

use crate::error::ValidationError;

/// Validate an identifier token: `^[a-z0-9]([a-z0-9._-]{0,62}[a-z0-9])?$`, max 64.
pub fn validate_id(token: &str, field: &str) -> Result<(), ValidationError> {
    let invalid = || {
        ValidationError::rule(
            crate::error::DOCUMENT_INVALID,
            format!("the {field} '{}' is not a valid identifier token", token),
        )
    };
    if token.is_empty() || token.len() > 64 {
        return Err(invalid());
    }
    let bytes = token.as_bytes();
    if !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit() {
        return Err(invalid());
    }
    if bytes.len() >= 2 {
        let last = bytes[bytes.len() - 1];
        if !last.is_ascii_lowercase() && !last.is_ascii_digit() {
            return Err(invalid());
        }
        for &b in &bytes[1..bytes.len() - 1] {
            if !(b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || b == b'.'
                || b == b'_'
                || b == b'-')
            {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

/// Validate a short reference key: `^[a-z][a-z0-9_-]{0,31}$`, max 32.
pub fn validate_token32(token: &str, field: &str) -> Result<(), ValidationError> {
    let invalid = || {
        ValidationError::rule(
            crate::error::DOCUMENT_INVALID,
            format!("the {field} '{}' is not a valid reference key", token),
        )
    };
    if token.is_empty() || token.len() > 32 {
        return Err(invalid());
    }
    for (index, b) in token.bytes().enumerate() {
        let ok = if index == 0 {
            b.is_ascii_lowercase()
        } else {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'
        };
        if !ok {
            return Err(invalid());
        }
    }
    Ok(())
}

/// Validate a lowercase hex SHA-256 digest.
pub fn validate_sha256(token: &str, field: &str) -> Result<(), ValidationError> {
    let ok = token.len() == 64
        && token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b.is_ascii_lowercase() && b.is_ascii_hexdigit()));
    if ok {
        Ok(())
    } else {
        Err(ValidationError::rule(
            crate::error::DOCUMENT_INVALID,
            format!("the {field} is not a lowercase hex sha256 digest"),
        ))
    }
}

/// Validate a workspace-relative path (the `relPath` rule of draft-0.1):
/// '/'-separated, no leading '/', no `..` segment, no backslash, no drive
/// letter, no NUL, 1..=1024 bytes.
pub fn validate_rel_path(path: &str) -> Result<(), ValidationError> {
    let invalid = || {
        ValidationError::rule(
            crate::error::PATH_ESCAPE,
            format!("the path '{}' is not a valid workspace-relative path", path),
        )
    };
    if path.is_empty() || path.len() > 1024 {
        return Err(invalid());
    }
    if path.starts_with('/') || path.contains('\\') || path.contains(':') || path.contains('\0') {
        return Err(invalid());
    }
    for segment in path.split('/') {
        if segment.is_empty() && !path.contains("//") {
            // only possible for the whole-string case which cannot happen
            return Err(invalid());
        }
        if segment == ".." || segment == "." {
            return Err(invalid());
        }
    }
    Ok(())
}

/// Resolve a validated workspace-relative path under a root, re-checking
/// containment on the joined result.
pub fn resolve_under(root: &std::path::Path, rel: &str) -> Result<std::path::PathBuf, ValidationError> {
    validate_rel_path(rel)?;
    let joined = root.join(rel);
    let canonical_base = root
        .canonicalize()
        .map_err(|err| ValidationError::io(root, format!("the workspace root is not usable: {err}")))?;
    // The target may not exist yet; containment is checked on the parent.
    let parent = joined
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| joined.clone());
    let anchor = if parent.exists() { parent } else { joined.clone() };
    if let Ok(resolved) = anchor.canonicalize() {
        if !resolved.starts_with(&canonical_base) {
            return Err(ValidationError::rule(
                crate::error::PATH_ESCAPE,
                format!("the path '{}' leaves the workspace container", rel),
            ));
        }
    }
    Ok(joined)
}

/// RFC 3339 timestamp guard (length + rough shape; full format validation is
/// the responsibility of the schema layer).
pub fn validate_rfc3339(value: &str) -> Result<(), ValidationError> {
    let ok = (20..=64).contains(&value.len())
        && value.as_bytes().get(4) == Some(&b'-')
        && value.ends_with('Z');
    if ok {
        Ok(())
    } else {
        Err(ValidationError::rule(
            crate::error::DOCUMENT_INVALID,
            format!("the timestamp '{}' is not an RFC 3339 UTC instant", value),
        ))
    }
}
