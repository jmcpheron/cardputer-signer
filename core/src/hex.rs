//! Hex in and out, without a heap.

const DIGITS: &[u8; 16] = b"0123456789abcdef";

/// `bytes` as lowercase hex into `out` (twice as long). Returns the written part as a str.
pub fn encode<'a>(bytes: &[u8], out: &'a mut [u8]) -> &'a str {
    let n = bytes.len() * 2;
    for (i, b) in bytes.iter().enumerate() {
        out[2 * i] = DIGITS[(b >> 4) as usize];
        out[2 * i + 1] = DIGITS[(b & 15) as usize];
    }
    core::str::from_utf8(&out[..n]).unwrap_or("")
}

fn nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Hex (with or without 0x) into exactly `out.len()` bytes. None on a bad digit or length.
pub fn decode(s: &str, out: &mut [u8]) -> Option<()> {
    let s = s.strip_prefix("0x").unwrap_or(s).as_bytes();
    if s.len() != out.len() * 2 {
        return None;
    }
    for (i, o) in out.iter_mut().enumerate() {
        *o = nibble(s[2 * i])? << 4 | nibble(s[2 * i + 1])?;
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut b = [0u8; 4];
        decode("0xDEADbeef", &mut b).unwrap();
        assert_eq!(b, [0xde, 0xad, 0xbe, 0xef]);
        let mut s = [0u8; 8];
        assert_eq!(encode(&b, &mut s), "deadbeef");
        assert!(decode("0xdead", &mut b).is_none());
        assert!(decode("0xdeadbeeg", &mut b).is_none());
    }
}
