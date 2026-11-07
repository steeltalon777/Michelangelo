//! CRC-32 (IEEE 802.3, reflected, polynomial 0xEDB88320).
//!
//! Used to verify PNG chunk checksums.

const POLY: u32 = 0xEDB8_8320;

/// Compute the CRC-32 (IEEE) checksum of the given bytes.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (POLY & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn crc32_of_empty_input() {
        assert_eq!(crc32(b""), 0);
    }
}
