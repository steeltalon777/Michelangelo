//! OBJ / MTL reference scanning.
//!
//! This module extracts *references only*: `mtllib` directives from the OBJ
//! file and texture-map directives from referenced MTL files. OBJ geometry
//! itself is not parsed or validated by intake (honest scope note recorded
//! as an `ObjParseNote` info diagnostic).
//!
//! Encoding: OBJ files in the wild are frequently not valid UTF-8. The file
//! is decoded lossily with a `NonUtf8Text` warning; if a *filename token*
//! contains replacement characters the reference is marked
//! `AmbiguousEncoding` (error) instead of being silently misread.

use crate::report::{Diagnostic, DiagnosticCode};

/// A reference extracted from OBJ/MTL text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjRef {
    pub declared: String,
    pub role: ObjRefRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjRefRole {
    MaterialLibrary,
    TextureMap,
}

/// Texture map keywords whose argument is a filename.
const MAP_KEYWORDS: &[&str] = &[
    "map_Kd", "map_Ks", "map_Ka", "map_Ke", "map_Ns", "map_d", "map_bump", "bump", "disp", "decal",
    "refl",
];

/// MTL options that consume N value tokens after the flag.
const OPTION_ARITY: &[(&str, usize)] = &[
    ("-s", 3),
    ("-o", 3),
    ("-t", 3),
    ("-mm", 2),
    ("-bm", 1),
    ("-boost", 1),
    ("-texres", 1),
    ("-clamp", 1),
    ("-blendu", 1),
    ("-blendv", 1),
    ("-cc", 1),
    ("-imfchan", 1),
    ("-type", 1),
    ("-sharpness", 1),
];

pub struct ObjScan {
    pub refs: Vec<ObjRef>,
    pub diagnostics: Vec<Diagnostic>,
}

fn decode_text(bytes: &[u8], subject: &str, diagnostics: &mut Vec<Diagnostic>) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            diagnostics.push(Diagnostic::warning(
                DiagnosticCode::NonUtf8Text,
                Some(subject.to_string()),
                "text asset is not valid UTF-8; parsed lossily",
            ));
            String::from_utf8_lossy(bytes).into_owned()
        }
    }
}

fn filename_from_tokens<'a>(tokens: &[&'a str]) -> Option<&'a str> {
    let mut i = 0usize;
    while i < tokens.len() {
        let tok = tokens[i];
        if let Some(rest) = tok.strip_prefix('-') {
            let key = format!("-{}", rest.split('-').next().unwrap_or(rest));
            let arity = OPTION_ARITY
                .iter()
                .find(|(k, _)| tok.eq_ignore_ascii_case(k) || key.eq_ignore_ascii_case(k))
                .map(|(_, a)| *a)
                .unwrap_or(1);
            i += 1 + arity;
            continue;
        }
        return Some(tok);
    }
    None
}

fn push_ref(
    declared: &str,
    role: ObjRefRole,
    refs: &mut Vec<ObjRef>,
    diagnostics: &mut Vec<Diagnostic>,
    subject: &str,
) {
    if declared.contains('\u{FFFD}') {
        diagnostics.push(Diagnostic::error(
            DiagnosticCode::AmbiguousEncoding,
            Some(declared.to_string()),
            "reference contains characters lost to lossy decoding; filename is ambiguous",
        ));
        return;
    }
    if declared.is_empty() {
        diagnostics.push(Diagnostic::info(
            DiagnosticCode::ObjParseNote,
            Some(subject.to_string()),
            "empty filename token ignored",
        ));
        return;
    }
    refs.push(ObjRef {
        declared: declared.to_string(),
        role,
    });
}

/// Scan OBJ file bytes for `mtllib` references.
pub fn scan_obj(bytes: &[u8], subject: &str) -> ObjScan {
    let mut diagnostics = Vec::new();
    let text = decode_text(bytes, subject, &mut diagnostics);
    let mut refs = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut tokens = line.split_whitespace();
        let keyword = tokens.next().unwrap_or("");
        if keyword.eq_ignore_ascii_case("mtllib") {
            for name in tokens {
                push_ref(
                    name,
                    ObjRefRole::MaterialLibrary,
                    &mut refs,
                    &mut diagnostics,
                    subject,
                );
            }
        }
    }
    diagnostics.push(Diagnostic::info(
        DiagnosticCode::ObjParseNote,
        Some(subject.to_string()),
        "OBJ geometry data was not parsed or validated by intake; only file references were read",
    ));
    ObjScan { refs, diagnostics }
}

/// Scan MTL file bytes for texture-map references.
pub fn scan_mtl(bytes: &[u8], subject: &str) -> ObjScan {
    let mut diagnostics = Vec::new();
    let text = decode_text(bytes, subject, &mut diagnostics);
    let mut refs = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut tokens = line.split_whitespace();
        let keyword = tokens.next().unwrap_or("");
        if MAP_KEYWORDS.iter().any(|k| keyword.eq_ignore_ascii_case(k)) {
            let rest: Vec<&str> = tokens.collect();
            if let Some(name) = filename_from_tokens(&rest) {
                push_ref(
                    name,
                    ObjRefRole::TextureMap,
                    &mut refs,
                    &mut diagnostics,
                    subject,
                );
            }
        }
    }
    ObjScan { refs, diagnostics }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_mtllib_and_maps() {
        let obj = scan_obj(
            b"# comment\nv 0 0 0\nmtllib materials.mtl\nf 1 2 3\n",
            "m.obj",
        );
        assert_eq!(obj.refs.len(), 1);
        assert_eq!(obj.refs[0].declared, "materials.mtl");

        let mtl = scan_mtl(
            b"newmtl body\nmap_Kd -clamp on textures/albedo.png\nmap_Ks spec.png\nbump -bm 0.5 bump.png\n",
            "m.mtl",
        );
        let names: Vec<&str> = mtl.refs.iter().map(|r| r.declared.as_str()).collect();
        assert_eq!(names, vec!["textures/albedo.png", "spec.png", "bump.png"]);
    }

    #[test]
    fn lossy_encoding_flags_ambiguous_filename() {
        let bad = b"mtllib \xFFmat.mtl\n";
        let scan = scan_obj(bad, "m.obj");
        assert!(scan
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::NonUtf8Text));
        assert!(scan.refs.is_empty());
        assert!(scan
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::AmbiguousEncoding));
    }
}
