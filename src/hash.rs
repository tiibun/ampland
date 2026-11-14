use std::io::{self, Read, Write};

use sha2::{Digest, Sha256};

/// Streams `reader` into `writer`, returning the number of bytes copied and
/// the lowercase hex SHA-256 of the data.
pub fn copy_and_sha256(mut reader: impl Read, mut writer: impl Write) -> io::Result<(u64, String)> {
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut size = 0u64;

    loop {
        let count = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        };
        hasher.update(&buf[..count]);
        writer.write_all(&buf[..count])?;
        size += count as u64;
    }
    writer.flush()?;

    Ok((size, to_hex(&hasher.finalize())))
}

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
    use super::{copy_and_sha256, to_hex};
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

    #[test]
    fn copy_and_sha256_copies_and_hashes() {
        let data = vec![7u8; 200 * 1024]; // spans several buffer fills
        let mut out = Vec::new();
        let (size, hex) = copy_and_sha256(&data[..], &mut out).expect("copy");
        assert_eq!(size, data.len() as u64);
        assert_eq!(out, data);
        assert_eq!(hex, to_hex(&Sha256::digest(&data)));
    }

    #[test]
    fn copy_and_sha256_handles_empty_input() {
        let mut out = Vec::new();
        let (size, hex) = copy_and_sha256(&b""[..], &mut out).expect("copy");
        assert_eq!(size, 0);
        assert!(out.is_empty());
        assert_eq!(
            hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
