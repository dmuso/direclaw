use sha2::{Digest, Sha256};
use std::path::Path;

pub fn compute_ingest_idempotency_key(canonical_source_path: &Path, bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical_source_path.as_os_str().as_encoded_bytes());
    hasher.update([0]);
    hasher.update(bytes);
    let digest = hasher.finalize();
    to_hex(&digest)
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
