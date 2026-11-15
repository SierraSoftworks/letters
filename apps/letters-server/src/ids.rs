//! Randomness and dates, which the core crates leave to their shell.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

pub fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).expect("the system has a source of randomness");
    bytes
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(2 * bytes.len()), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// A secret nobody can guess: 256 random bits, as text that goes into a header or an address as it is.
pub fn secret() -> String {
    base64url(&random::<32>())
}

/// What the store keeps of a secret. Whoever reads the store cannot use what they find there.
pub fn hashed(secret: &str) -> String {
    hex(&Sha256::digest(secret.as_bytes()))
}

/// Base64 in the alphabet for addresses, without padding: what PKCE and JWTs write.
pub fn base64url(bytes: &[u8]) -> String {
    encoded(bytes, b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_", false)
}

/// Base64 as a header writes it, padded.
pub fn base64(bytes: &[u8]) -> String {
    encoded(bytes, b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/", true)
}

fn encoded(bytes: &[u8], alphabet: &[u8; 64], padded: bool) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let bits = chunk.iter().fold(0u32, |bits, byte| bits << 8 | u32::from(*byte)) << (8 * (3 - chunk.len()));
        for i in 0..=chunk.len() {
            out.push(char::from(alphabet[(bits >> (18 - 6 * i) & 63) as usize]));
        }
        if padded {
            out.extend(std::iter::repeat_n('=', 3 - chunk.len()));
        }
    }
    out
}

/// A block id, as the editor makes them.
pub fn block_id() -> String {
    letters_editor::block_id(random())
}

/// A document id: a random (version 4) UUID, as the desktop app makes them.
pub fn document_id() -> String {
    let mut b = random::<16>();
    b[6] = b[6] & 0x0f | 0x40;
    b[8] = b[8] & 0x3f | 0x80;
    let hex = hex(&b);
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

/// `seconds` since 1970 as RFC 3339, in UTC.
pub fn rfc3339(seconds: u64) -> String {
    let time = i64::try_from(seconds).ok().and_then(|seconds| jiff::Timestamp::from_second(seconds).ok());
    time.unwrap_or(jiff::Timestamp::UNIX_EPOCH).to_string()
}
