//! SHA-256 and CRC-32 test vectors.

mod common;

use common::crc32_of;

#[test]
fn sha256_matches_nist_empty_string_vector() {
    assert_eq!(
        michelangelo_input::sha256::sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn sha256_matches_nist_abc_vector() {
    assert_eq!(
        michelangelo_input::sha256::sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn sha256_matches_nist_one_million_a_vector() {
    let data = vec![b'a'; 1_000_000];
    assert_eq!(
        michelangelo_input::sha256::sha256_hex(&data),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn sha256_digest_and_hex_agree() {
    let digest = michelangelo_input::sha256::sha256(b"michelangelo");
    let hex = michelangelo_input::sha256::sha256_hex(b"michelangelo");
    let mut expected = String::new();
    for byte in digest {
        expected.push_str(&format!("{byte:02x}"));
    }
    assert_eq!(hex, expected);
    assert_eq!(hex.len(), 64);
    assert!(hex
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
}

#[test]
fn sha256_of_the_two_block_boundary_is_stable() {
    assert_eq!(
        michelangelo_input::sha256::sha256_hex(&[b'a'; 55]),
        "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318"
    );
    assert_eq!(
        michelangelo_input::sha256::sha256_hex(&[b'a'; 56]),
        "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a"
    );
}

#[test]
fn crc32_matches_ieee_check_value() {
    assert_eq!(crc32_of(b"123456789"), 0xCBF4_3926);
}

#[test]
fn crc32_of_empty_input_is_zero() {
    assert_eq!(crc32_of(b""), 0);
}
