//! Safe resolution of declared dependency references against a base
//! directory.
//!
//! Security posture: a declared reference from an untrusted asset file is
//! data, not a path. It may only point to a real file *inside* the source
//! directory tree. Absolute paths, network schemes, percent-encoding,
//! `..` traversal and symlink/junction escapes are all rejected with distinct
//! diagnostics so the operator sees exactly why a reference was refused.

use std::path::{Path, PathBuf};

/// Failure to safely resolve a declared reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefPathError {
    /// Empty or malformed reference.
    Malformed,
    /// Contains percent-encoding (conservatively rejected).
    PercentEncoded,
    /// Absolute filesystem path (POSIX `/...`, Windows `C:\...`, UNC `\\...`).
    Absolute,
    /// Network or URI scheme (`http:`, `https:`, `ftp:`, `file:`, ...).
    Network,
    /// `..` escapes the base directory.
    Traversal,
    /// The base directory itself could not be canonicalized.
    BaseUnresolved,
    /// The referenced file does not exist.
    NotFound,
    /// The canonical target is outside the canonical base directory, i.e. it
    /// escaped via a symlink or junction.
    SymlinkEscape,
    /// Canonicalization of the target failed with an IO error.
    Unresolvable,
}

/// A safely resolved reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRef {
    /// Canonicalized absolute path of the target (Windows `\\?\` prefixes
    /// stripped for comparability).
    pub canonical: PathBuf,
    /// Normalized relative path as declared (no `.` / `..` components),
    /// used for storage naming and manifest display.
    pub relative: PathBuf,
}

fn is_absolute_like(declared: &str) -> bool {
    let b = declared.as_bytes();
    if b.is_empty() {
        return false;
    }
    // POSIX absolute or UNC.
    if b[0] == b'/' || b[0] == b'\\' {
        return true;
    }
    // Windows drive letter with any separator style.
    b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic()
}

fn looks_like_uri_scheme(declared: &str) -> bool {
    // `scheme:` where scheme is ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ).
    // Drive letters were already excluded by `is_absolute_like` before this
    // is consulted.
    let Some(idx) = declared.find(':') else {
        return false;
    };
    let scheme = &declared[..idx];
    if scheme.is_empty() {
        return false;
    }
    let mut chars = scheme.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
}

fn normalize_components(declared: &str) -> Result<Vec<String>, RefPathError> {
    let mut stack: Vec<String> = Vec::new();
    for comp in declared.split(['/', '\\']) {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp == ".." {
            if stack.pop().is_none() {
                return Err(RefPathError::Traversal);
            }
            continue;
        }
        stack.push(comp.to_string());
    }
    Ok(stack)
}

/// Strip Windows extended-length prefixes so that string prefix comparison
/// works uniformly.
fn normalize_canonical(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy().into_owned();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path
}

fn path_key(path: &Path) -> String {
    let s = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        s.to_ascii_lowercase()
    } else {
        s
    }
}

fn is_within(canonical: &Path, base_canonical: &Path) -> bool {
    let target = path_key(canonical);
    let base = path_key(base_canonical);
    let base = base.trim_end_matches(['/', '\\']);
    if !target.starts_with(base) {
        return false;
    }
    let rest = &target[base.len()..];
    rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\')
}

/// Resolve a declared reference against `base_dir`.
///
/// On success the target is guaranteed to exist and to canonicalize inside
/// the canonical base directory.
pub fn resolve_reference(base_dir: &Path, declared: &str) -> Result<ResolvedRef, RefPathError> {
    if declared.is_empty() {
        return Err(RefPathError::Malformed);
    }
    if declared.contains('%') {
        return Err(RefPathError::PercentEncoded);
    }
    if declared.contains('\0') {
        return Err(RefPathError::Malformed);
    }
    if is_absolute_like(declared) {
        return Err(RefPathError::Absolute);
    }
    if looks_like_uri_scheme(declared) {
        return Err(RefPathError::Network);
    }
    let components = normalize_components(declared)?;
    if components.is_empty() {
        return Err(RefPathError::Malformed);
    }
    let relative: PathBuf = components.iter().collect();
    let base_canonical = base_dir
        .canonicalize()
        .map(normalize_canonical)
        .map_err(|_| RefPathError::BaseUnresolved)?;
    let joined = base_dir.join(&relative);
    let metadata = std::fs::metadata(&joined).map_err(|_| RefPathError::NotFound)?;
    if !metadata.is_file() {
        return Err(RefPathError::NotFound);
    }
    let canonical = joined
        .canonicalize()
        .map(normalize_canonical)
        .map_err(|_| RefPathError::Unresolvable)?;
    if !is_within(&canonical, &base_canonical) {
        return Err(RefPathError::SymlinkEscape);
    }
    Ok(ResolvedRef {
        canonical,
        relative,
    })
}

/// Map declared references to their canonical targets to detect path
/// collisions (two distinct declared strings pointing at one file).
pub struct CollisionTracker {
    seen: std::collections::BTreeMap<String, String>,
}

impl CollisionTracker {
    pub fn new() -> Self {
        CollisionTracker {
            seen: std::collections::BTreeMap::new(),
        }
    }

    /// Returns the previously declared reference if `declared` resolves to a
    /// canonical target that was already seen.
    pub fn register(&mut self, canonical: &Path, declared: &str) -> Option<String> {
        let key = path_key(canonical);
        self.seen.insert(key, declared.to_string())
    }
}

impl Default for CollisionTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Flatten a normalized relative path into a single safe filename component
/// used inside the snapshot's `dependencies/` directory.
pub fn flatten_for_storage(relative: &Path) -> String {
    let joined = relative.to_string_lossy().replace(['/', '\\', ':'], "__");
    let cleaned: String = joined
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned == "." {
        "unnamed".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_rejects_escape() {
        assert_eq!(normalize_components("a/../b").unwrap(), vec!["b"]);
        assert_eq!(
            normalize_components("..").unwrap_err(),
            RefPathError::Traversal
        );
        assert_eq!(
            normalize_components("a/../../b").unwrap_err(),
            RefPathError::Traversal
        );
    }

    #[test]
    fn absolute_and_network_forms() {
        assert!(is_absolute_like("/etc/passwd"));
        assert!(is_absolute_like(r"C:\tmp\x.bin"));
        assert!(is_absolute_like(r"\\server\share\x"));
        assert!(!is_absolute_like("textures/a.png"));
        assert!(looks_like_uri_scheme("https://example.com/x.png"));
        assert!(looks_like_uri_scheme("file:///x"));
        assert!(!looks_like_uri_scheme("textures/a.png"));
        // Drive letter is caught by is_absolute_like first.
        assert_eq!(
            resolve_reference(Path::new("."), "C:/x.bin").unwrap_err(),
            RefPathError::Absolute
        );
    }

    #[test]
    fn flatten_is_stable() {
        assert_eq!(
            flatten_for_storage(Path::new("textures/albedo.png")),
            "textures__albedo.png"
        );
    }
}
