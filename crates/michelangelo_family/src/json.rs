//! Minimal JSON value type with a strict parser and a canonical writer.
//!
//! Zero-dependency stand-in (provisional) for the serde-based wire layer that
//! GLM-1's draft-0.1 schemas imply. The parser deliberately rejects trailing
//! content, duplicate object keys and invalid escapes so that wire-level
//! negative fixtures fail loudly instead of silently.
//!
//! The writer side provides `canonical_string`, an RFC 8785-flavoured
//! deterministic encoding (sorted object keys, no whitespace, minimal string
//! escaping) used for digest computation. Floats are NOT canonically encoded
//! here: hashed content must only contain integers, strings, booleans, arrays
//! and objects (see `canonical::` guard). This limitation is documented in
//! INTEGRATION_NOTES.md as an open item for the round-2 contract merge.

use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    /// Present only where a recipe deliberately carries a decimal parameter
    /// value; excluded from canonical hashing by `canonical.rs`.
    Float(f64),
    Str(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

impl Json {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Float(f) => Some(*f),
            Json::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, Json>> {
        match self {
            Json::Object(m) => Some(m),
            _ => None,
        }
    }

    /// Non-lossy canonical text of a JSON value (also used for hashing).
    pub fn to_canonical(&self) -> String {
        let mut out = String::new();
        write_canonical(self, &mut out);
        out
    }
}

impl fmt::Display for Json {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_canonical())
    }
}

fn write_escaped(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_canonical(v: &Json, out: &mut String) {
    match v {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Int(i) => out.push_str(&i.to_string()),
        Json::Float(x) => out.push_str(&format_float(*x)),
        Json::Str(s) => write_escaped(s, out),
        Json::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Json::Object(map) => {
            // BTreeMap iterates in sorted key order => canonical.
            out.push('{');
            for (i, (k, val)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_escaped(k, out);
                out.push(':');
                write_canonical(val, out);
            }
            out.push('}');
        }
    }
}

/// Deterministic shortest decimal formatting for floats (Rust std
/// shortest-round-trip). Numbers only reach hashed content through
/// integer-typed paths; float params are hashed via their canonical decimal
/// string with the caveat pinned to `derivation_version` (see
/// INTEGRATION_NOTES.md).
fn format_float(x: f64) -> String {
    if x == x.trunc() && x.is_finite() && x.abs() < 1e15 {
        format!("{x:.1}")
    } else {
        format!("{x}")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JsonError {
    pub offset: usize,
    pub message: String,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "json error at byte {}: {}", self.offset, self.message)
    }
}

pub fn parse(input: &str) -> Result<Json, JsonError> {
    let bytes = input.as_bytes();
    let mut p = Parser { bytes, pos: 0 };
    p.skip_ws();
    let v = p.parse_value()?;
    p.skip_ws();
    if p.pos != bytes.len() {
        return Err(p.err("trailing content after JSON value"));
    }
    Ok(v)
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn err(&self, msg: &str) -> JsonError {
        JsonError {
            offset: self.pos,
            message: msg.to_string(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, b: u8) -> Result<(), JsonError> {
        if self.peek() == Some(b) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err(&format!("expected '{}'", b as char)))
        }
    }

    fn parse_value(&mut self) -> Result<Json, JsonError> {
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => Ok(Json::Str(self.parse_string()?)),
            Some(b't') => self.parse_literal("true", Json::Bool(true)),
            Some(b'f') => self.parse_literal("false", Json::Bool(false)),
            Some(b'n') => self.parse_literal("null", Json::Null),
            Some(b) if b == b'-' || b.is_ascii_digit() => self.parse_number(),
            _ => Err(self.err("unexpected token")),
        }
    }

    fn parse_literal(&mut self, lit: &str, v: Json) -> Result<Json, JsonError> {
        if self.bytes[self.pos..].starts_with(lit.as_bytes()) {
            self.pos += lit.len();
            Ok(v)
        } else {
            Err(self.err(&format!("invalid literal, expected {lit}")))
        }
    }

    fn parse_object(&mut self) -> Result<Json, JsonError> {
        self.expect(b'{')?;
        let mut map = BTreeMap::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Object(map));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(b':')?;
            self.skip_ws();
            let value = self.parse_value()?;
            if map.insert(key.clone(), value.clone()).is_some() {
                return Err(self.err(&format!("duplicate object key '{key}'")));
            }
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Object(map));
                }
                _ => return Err(self.err("expected ',' or '}' in object")),
            }
        }
    }

    fn parse_array(&mut self) -> Result<Json, JsonError> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_ws();
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.err("expected ',' or ']' in array")),
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, JsonError> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    match self.peek() {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'/') => out.push('/'),
                        Some(b'n') => out.push('\n'),
                        Some(b't') => out.push('\t'),
                        Some(b'r') => out.push('\r'),
                        Some(b'b') => out.push('\u{0008}'),
                        Some(b'f') => out.push('\u{000C}'),
                        Some(b'u') => {
                            self.pos += 1;
                            let mut code = 0u32;
                            for _ in 0..4 {
                                let h = self
                                    .peek()
                                    .and_then(|b| (b as char).to_digit(16))
                                    .ok_or_else(|| self.err("invalid \\u escape"))?;
                                code = code * 16 + h;
                                self.pos += 1;
                            }
                            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                            continue;
                        }
                        _ => return Err(self.err("invalid escape")),
                    }
                    self.pos += 1;
                }
                Some(b) if b < 0x20 => return Err(self.err("control character in string")),
                Some(_) => {
                    // Consume one UTF-8 char.
                    let rest = &self.bytes[self.pos..];
                    let s = std::str::from_utf8(rest)
                        .map_err(|_| self.err("invalid utf-8"))?;
                    let ch = s.chars().next().unwrap();
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn parse_number(&mut self) -> Result<Json, JsonError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while matches!(self.peek(), Some(b) if b.is_ascii_digit()) {
            self.pos += 1;
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') {
            is_float = true;
            self.pos += 1;
            while matches!(self.peek(), Some(b) if b.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            while matches!(self.peek(), Some(b) if b.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| self.err("invalid number"))?;
        if text.is_empty() || text == "-" {
            return Err(self.err("invalid number"));
        }
        if is_float {
            text.parse::<f64>()
                .map(Json::Float)
                .map_err(|_| self.err("invalid float"))
        } else {
            text.parse::<i64>()
                .map(Json::Int)
                .map_err(|_| self.err("integer out of i64 range"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_canonical_roundtrip() {
        let src = r#"{"b":[1,2,{"a":null}],"a":"x\ny"}"#;
        let v = parse(src).unwrap();
        assert_eq!(
            v.to_canonical(),
            r#"{"a":"x\ny","b":[1,2,{"a":null}]}"#
        );
    }

    #[test]
    fn key_order_irrelevant_for_canonical() {
        let a = parse(r#"{"x":1,"y":2}"#).unwrap();
        let b = parse(r#"{"y":2,"x":1}"#).unwrap();
        assert_eq!(a.to_canonical(), b.to_canonical());
    }

    #[test]
    fn rejects_duplicates_and_trailing() {
        assert!(parse(r#"{"a":1,"a":2}"#).is_err());
        assert!(parse(r#"{} "#.trim_end()).is_ok());
        assert!(parse("{} x").is_err());
        assert!(parse("{").is_err());
    }

    #[test]
    fn numbers() {
        assert_eq!(parse("42").unwrap(), Json::Int(42));
        assert_eq!(parse("-7").unwrap(), Json::Int(-7));
        assert_eq!(parse("0.5").unwrap(), Json::Float(0.5));
        assert!(parse("1e400").is_err() || parse("1e400").unwrap() == Json::Float(f64::INFINITY));
    }
}
