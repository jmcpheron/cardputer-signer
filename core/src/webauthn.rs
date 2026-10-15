//! The message the chip signs for a Safe hash: the one a passkey signs (docs/PASSKEY.md), so a chip
//! and a passkey own a Safe the same way, through Safe's passkey signer. Built here from the hash
//! this device worked out:
//!
//!   authenticatorData = sha256(rp id) ‖ flags ‖ sign count (4 bytes)
//!   clientDataJSON    = {"type":"webauthn.get","challenge":"<base64url of the hash>","origin":"…"}
//!   digest            = sha256(authenticatorData ‖ sha256(clientDataJSON))
//!
//! The chip signs `digest`. Checked against `vectors/webauthn.json`, signatures Safe's contract
//! accepted on Base Sepolia.
use core::fmt::Write;

use crate::buf::Buf;
use crate::sha256::sha256;

/// What the device says it is: the site's name and origin, as a passkey on diykey.app says.
pub const RP_ID: &str = "diykey.app";
pub const ORIGIN: &str = "https://diykey.app";
/// User present and user verified: someone held the key down, on the device that showed the hash.
pub const FLAGS: u8 = 0x05;

pub type ClientData = Buf<128>;

const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// 32 bytes as base64url without padding: 43 characters.
pub fn b64url(b: &[u8; 32], out: &mut [u8; 43]) {
    let mut k = 0;
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..c.len() + 1 {
            out[k] = B64URL[(n >> (18 - 6 * i) & 63) as usize];
            k += 1;
        }
    }
}

pub fn authenticator_data(rp_id: &str) -> [u8; 37] {
    let mut a = [0u8; 37];
    a[..32].copy_from_slice(&sha256(&[rp_id.as_bytes()]));
    a[32] = FLAGS;
    a
}

pub fn client_data(hash: &[u8; 32], origin: &str) -> ClientData {
    let mut ch = [0u8; 43];
    b64url(hash, &mut ch);
    let mut c = Buf::new();
    let _ = write!(c, r#"{{"type":"webauthn.get","challenge":"{}","origin":"{}"}}"#,
                   core::str::from_utf8(&ch).unwrap_or(""), origin);
    c
}

/// The digest the chip signs for `hash`.
pub fn digest_for(hash: &[u8; 32], rp_id: &str, origin: &str) -> [u8; 32] {
    let client = client_data(hash, origin);
    sha256(&[&authenticator_data(rp_id), &sha256(&[client.as_str().as_bytes()])])
}

/// The digest the chip signs for `hash`, as this device says it.
pub fn digest(hash: &[u8; 32]) -> [u8; 32] {
    digest_for(hash, RP_ID, ORIGIN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hex;

    fn bytes<const N: usize>(s: &str) -> [u8; N] {
        let mut b = [0u8; N];
        hex::decode(s, &mut b).unwrap();
        b
    }

    // The MicroPython signer's site was "picoquorum": with it, the same bytes as the vectors.
    #[test]
    fn the_vectors_messages() {
        let v: serde_json::Value = serde_json::from_str(include_str!("../../vectors/webauthn.json")).unwrap();
        for t in v["vectors"].as_array().unwrap() {
            let s = |k: &str| t[k].as_str().unwrap();
            let hash: [u8; 32] = bytes(s("safeTxHash"));
            let auth: [u8; 37] = bytes(s("authenticatorData"));
            assert_eq!(authenticator_data("picoquorum"), auth);
            assert_eq!(client_data(&hash, "picoquorum").as_str(), s("clientDataJSON"));
            assert_eq!(digest_for(&hash, "picoquorum", "picoquorum"), bytes::<32>(s("digest")), "{}", s("name"));
        }
    }

    #[test]
    fn diykey_s_message() {
        let c = client_data(&[0xfb; 32], ORIGIN);
        assert_eq!(c.as_str(), r#"{"type":"webauthn.get","challenge":"-_v7-_v7-_v7-_v7-_v7-_v7-_v7-_v7-_v7-_v7-_s","origin":"https://diykey.app"}"#);
        assert_eq!(authenticator_data(RP_ID)[32..], [5, 0, 0, 0, 0]);
    }
}
