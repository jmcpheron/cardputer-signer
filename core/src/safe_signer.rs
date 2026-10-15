//! A P-256 key's Safe signer address: the SafeWebAuthnSignerProxy that Safe's passkey factory
//! (safe-modules passkey v0.2.1) deploys for it, at a CREATE2 address fixed by the key. A Safe lists
//! this address as an owner. It only checks the key's signatures: it isn't a wallet. The same as
//! web/lib/signer.js, and checked against `vectors/webauthn.json`, whose addresses came from the
//! factory on Base Sepolia (`getSigner`).
use crate::hex;
use crate::safe_tx::{keccak256, Address};
use sha3::{Digest, Keccak256};

const FACTORY: &str = "1d31f259ee307358a26dfb23eb365939e8641195";
const SINGLETON: &str = "4e27b51350e6c2083ee19011120f50dafec5ca50";
/// The P-256 precompile at 0x100 first, then the Daimo verifier, as one uint176.
const VERIFIERS: &str = "0100c2b78104907f722dabac4c69f826a522b2754de4";
/// SafeWebAuthnSignerProxy's creation code, as the factory deploys it (437 bytes).
const PROXY_CODE: [&str; 10] = [
    "610100346100ad57601f6101b538819003918201601f19168301916001600160401b038311848410176100b257808492",
    "6080946040528339810103126100ad578051906001600160a01b03821682036100ad5760208101516040820151606090",
    "920151926001600160b01b03841684036100ad5760805260a05260c05260e05260405160ec90816100c9823960805181",
    "6082015260a05181604d015260c051816027015260e0518160010152f35b600080fd5b634e487b7160e01b6000526041",
    "60045260246000fdfe7f000000000000000000000000000000000000000000000000000000000000000060b63601527f",
    "000000000000000000000000000000000000000000000000000000000000000060a03601527f00000000000000000000",
    "0000000000000000000000000000000000000000000036608001523660006080376000806056360160807f0000000000",
    "0000000000000000000000000000000000000000000000000000005af43d600060803e60b1573d6080fd5b3d6080f3fe",
    "a26469706673582212201660515548d15702d720bbc046b457ca85e941a4559ab9f9518488e4c82e5ee964736f6c6343",
    "00081a0033",
];

fn word(h: &str) -> [u8; 32] {
    let mut w = [0u8; 32];
    let n = h.len() / 2;
    hex::decode(h, &mut w[32 - n..]).unwrap();
    w
}

/// The signer's address for the public key x ‖ y: CREATE2 by the factory, salt 0, over the proxy's
/// creation code and its arguments (singleton, x, y, verifiers).
pub fn address(pubkey: &[u8; 64]) -> Address {
    let mut init = Keccak256::new();
    for line in PROXY_CODE {
        let mut b = [0u8; 48];
        let n = line.len() / 2;
        hex::decode(line, &mut b[..n]).unwrap();
        init.update(&b[..n]);
    }
    init.update(word(SINGLETON));
    init.update(&pubkey[..32]);
    init.update(&pubkey[32..]);
    init.update(word(VERIFIERS));
    let init: [u8; 32] = init.finalize().into();
    let mut factory = [0u8; 20];
    hex::decode(FACTORY, &mut factory).unwrap();
    let h = keccak256(&[&[0xff], &factory, &[0u8; 32], &init]);
    let mut a = [0u8; 20];
    a.copy_from_slice(&h[12..]);
    a
}

/// The address as EIP-55 writes it: 0x, then its letters cased by its own keccak.
pub fn checksum<'a>(a: &Address, out: &'a mut [u8; 42]) -> &'a str {
    out[..2].copy_from_slice(b"0x");
    hex::encode(a, &mut out[2..]);
    let h = keccak256(&[&out[2..]]);
    for i in 0..40 {
        let nib = if i % 2 == 0 { h[i / 2] >> 4 } else { h[i / 2] & 15 };
        if nib >= 8 {
            out[2 + i] = out[2 + i].to_ascii_uppercase();
        }
    }
    core::str::from_utf8(out).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_factory_s_addresses() {
        let v: serde_json::Value = serde_json::from_str(include_str!("../../vectors/webauthn.json")).unwrap();
        let s = &v["signer"];
        assert_eq!(s["factory"].as_str().unwrap(), format!("0x{FACTORY}"));
        assert_eq!(s["singleton"].as_str().unwrap(), format!("0x{SINGLETON}"));
        assert_eq!(s["verifiers"].as_str().unwrap(), format!("0x{VERIFIERS}"));
        assert_eq!(PROXY_CODE.concat().len(), 437 * 2);
        for t in v["vectors"].as_array().unwrap() {
            let mut k = [0u8; 64];
            hex::decode(t["qx"].as_str().unwrap(), &mut k[..32]).unwrap();
            hex::decode(t["qy"].as_str().unwrap(), &mut k[32..]).unwrap();
            let mut out = [0u8; 42];
            let a = checksum(&address(&k), &mut out).to_string();
            assert_eq!(a.to_lowercase(), t["owner"].as_str().unwrap(), "{}", t["name"]);
        }
    }

    // EIP-55's own examples.
    #[test]
    fn eip55() {
        for want in ["0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed", "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359",
                     "0xdbF03B407c01E7cD3CBea99509d93f8DDDC8C6FB", "0xD1220A0cf47c7B9Be7A2E6BA89F429762e7b9aDb"] {
            let mut a = [0u8; 20];
            hex::decode(want, &mut a).unwrap();
            let mut out = [0u8; 42];
            assert_eq!(checksum(&a, &mut out), want);
        }
    }
}
