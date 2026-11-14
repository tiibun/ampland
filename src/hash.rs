/// Encodes bytes (e.g. a SHA-256 digest) as a lowercase hex string.
pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::to_hex;
    use sha2::{Digest, Sha256};

    #[test]
    fn to_hex_encodes_lowercase() {
        assert_eq!(to_hex(&[]), "");
        assert_eq!(to_hex(&[0x00, 0x0f, 0xab, 0xff]), "000fabff");
    }

    #[test]
    fn to_hex_matches_known_sha256() {
        let digest = Sha256::digest(b"abc");
        assert_eq!(
            to_hex(&digest),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
