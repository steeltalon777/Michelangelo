//! Minimal strict base64 (RFC 4648) decoder for `data:` URI payloads.
//!
//! Implemented in-repo to keep the dependency set at `serde`/`serde_json`
//! plus the reused in-repo SHA-256, following the house convention of
//! `michelangelo_input`. Strict: no whitespace, no URL-safe alphabet,
//! canonical padding only.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Base64Error {
    InvalidLength,
    InvalidChar { pos: usize, ch: char },
}

fn value(ch: u8) -> Option<u8> {
    match ch {
        b'A'..=b'Z' => Some(ch - b'A'),
        b'a'..=b'z' => Some(ch - b'a' + 26),
        b'0'..=b'9' => Some(ch - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Decode a standard-alphabet base64 string. Padding with `=` is required to
/// be canonical (0, 1 or 2 trailing `=`).
pub fn decode(input: &str) -> Result<Vec<u8>, Base64Error> {
    let trimmed = input.trim_end_matches('=');
    let padding = input.len() - trimmed.len();
    if padding > 2 {
        return Err(Base64Error::InvalidLength);
    }
    if trimmed.len() % 4 == 1 {
        return Err(Base64Error::InvalidLength);
    }
    if input.ends_with('=') && padding != (4 - trimmed.len() % 4) % 4 {
        // Non-canonical padding length.
        return Err(Base64Error::InvalidLength);
    }
    let mut out = Vec::with_capacity(trimmed.len() / 4 * 3 + 3);
    let bytes = trimmed.as_bytes();
    let mut chunk = [0u8; 4];
    let mut chunk_len = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        let v = value(b).ok_or(Base64Error::InvalidChar {
            pos: i,
            ch: b as char,
        })?;
        chunk[chunk_len] = v;
        chunk_len += 1;
        if chunk_len == 4 {
            push_chunk(&chunk, 4, &mut out);
            chunk_len = 0;
        }
    }
    if chunk_len > 0 {
        // Remaining 2 or 3 symbols (1 was rejected above).
        let last = chunk[chunk_len - 1];
        if last & 0x0f != 0 && chunk_len == 2 {
            // Non-canonical trailing bits are rejected.
            return Err(Base64Error::InvalidLength);
        }
        if chunk_len == 3 && last & 0x03 != 0 {
            return Err(Base64Error::InvalidLength);
        }
        push_chunk(&chunk, chunk_len, &mut out);
    }
    Ok(out)
}

fn push_chunk(chunk: &[u8; 4], len: usize, out: &mut Vec<u8>) {
    let n = (chunk[0] as u32) << 18
        | (chunk[1] as u32) << 12
        | (chunk[2] as u32) << 6
        | chunk[3] as u32;
    out.push((n >> 16) as u8);
    if len >= 3 {
        out.push((n >> 8) as u8);
    }
    if len >= 4 {
        out.push(n as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4648_vectors() {
        assert_eq!(decode("").unwrap(), b"");
        assert_eq!(decode("Zg==").unwrap(), b"f");
        assert_eq!(decode("Zm8=").unwrap(), b"fo");
        assert_eq!(decode("Zm9v").unwrap(), b"foo");
        assert_eq!(decode("Zm9vYg==").unwrap(), b"foob");
        assert_eq!(decode("Zm9vYmE=").unwrap(), b"fooba");
        assert_eq!(decode("Zm9vYmFy").unwrap(), b"foobar");
    }

    #[test]
    fn rejects_invalid() {
        assert!(decode("Zm9vYg=").is_err()); // bad padding
        assert!(decode("Zm9v!g==").is_err());
        assert!(decode("Zg=").is_err());
        assert!(decode("Zm9v YmFy").is_err()); // whitespace is not allowed
        assert!(decode("Zh==").is_err()); // non-canonical trailing bits
    }
}
