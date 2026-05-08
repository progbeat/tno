use crate::*;

pub(crate) fn full_scope() -> Vec<String> {
    vec![".".to_string()]
}

pub(crate) fn expectation_id(prompt: &str, expected: &str) -> String {
    hash_120(format!("q\0{}\0a\0{}", prompt, expected).as_bytes())
}

pub(crate) fn hash_120(input: &[u8]) -> String {
    let first = fnv64_with_seed(FNV_OFFSET, input);
    let second = fnv64_with_seed(FNV_OFFSET ^ 0x9e37_79b9_7f4a_7c15, input);
    let mut bytes = [0u8; 15];
    bytes[..8].copy_from_slice(&first.to_be_bytes());
    bytes[8..].copy_from_slice(&second.to_be_bytes()[..7]);
    encode_base64url_no_pad(&bytes)
}

pub(crate) fn fnv64_with_seed(seed: u64, input: &[u8]) -> u64 {
    let mut hash = seed;
    for byte in input {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

pub(crate) fn encode_base64url_no_pad(bytes: &[u8]) -> String {
    let mut out = String::with_capacity((bytes.len() * 4 + 2) / 3);
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        let value = ((a as u32) << 16) | ((b as u32) << 8) | c as u32;
        out.push(B64_URL[((value >> 18) & 0x3f) as usize] as char);
        out.push(B64_URL[((value >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_URL[((value >> 6) & 0x3f) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(B64_URL[(value & 0x3f) as usize] as char);
        }
    }
    out
}
