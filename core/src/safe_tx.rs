//! The Safe transaction hash, rebuilt from the fields the screen shows. The signer never signs a
//! hash it was handed: it hashes these fields itself. Safe >= 1.3.0: the EIP-712 domain is
//! {chainId, verifyingContract}, no name or version. Checked against `vectors/safe_tx.json`.
use sha3::{Digest, Keccak256};

pub type Address = [u8; 20];
/// A uint256, big-endian.
pub type U256 = [u8; 32];

pub const CALL: u8 = 0;
pub const DELEGATECALL: u8 = 1;
pub const ZERO: Address = [0; 20];

const DOMAIN_TYPE: &[u8] = b"EIP712Domain(uint256 chainId,address verifyingContract)";
const SAFE_TX_TYPE: &[u8] = b"SafeTx(address to,uint256 value,bytes data,uint8 operation,uint256 safeTxGas,\
uint256 baseGas,uint256 gasPrice,address gasToken,address refundReceiver,uint256 nonce)";

pub struct SafeTx<'a> {
    pub to: Address,
    pub value: U256,
    pub data: &'a [u8],
    pub operation: u8,
    pub safe_tx_gas: U256,
    pub base_gas: U256,
    pub gas_price: U256,
    pub gas_token: Address,
    pub refund_receiver: Address,
    pub nonce: U256,
}

#[derive(Debug, PartialEq)]
pub enum Error {
    /// operation is neither CALL (0) nor DELEGATECALL (1)
    Operation(u8),
}

pub fn keccak256(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Keccak256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

fn addr_word(a: &Address) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a);
    w
}

pub fn u256_from_u64(n: u64) -> U256 {
    let mut w = [0u8; 32];
    w[24..].copy_from_slice(&n.to_be_bytes());
    w
}

pub fn domain_separator(chain_id: u64, safe: &Address) -> [u8; 32] {
    keccak256(&[&keccak256(&[DOMAIN_TYPE]), &u256_from_u64(chain_id), &addr_word(safe)])
}

pub fn safe_tx_hash(chain_id: u64, safe: &Address, tx: &SafeTx) -> Result<[u8; 32], Error> {
    if tx.operation > DELEGATECALL {
        return Err(Error::Operation(tx.operation));
    }
    let s = keccak256(&[
        &keccak256(&[SAFE_TX_TYPE]),
        &addr_word(&tx.to),
        &tx.value,
        &keccak256(&[tx.data]),
        &u256_from_u64(tx.operation as u64),
        &tx.safe_tx_gas,
        &tx.base_gas,
        &tx.gas_price,
        &addr_word(&tx.gas_token),
        &addr_word(&tx.refund_receiver),
        &tx.nonce,
    ]);
    Ok(keccak256(&[b"\x19\x01", &domain_separator(chain_id, safe), &s]))
}

/// The last 8 hex characters of the hash, lowercase, as the Safe app shows them.
pub fn verify_code(h: &[u8; 32]) -> [u8; 8] {
    let mut out = [0u8; 8];
    crate::hex::encode(&h[28..], &mut out);
    out
}

/// A decimal string as a uint256. None if it isn't all digits or doesn't fit.
pub fn u256_from_dec(s: &str) -> Option<U256> {
    if s.is_empty() {
        return None;
    }
    let mut w = [0u8; 32];
    for c in s.bytes() {
        if !c.is_ascii_digit() {
            return None;
        }
        let mut carry = (c - b'0') as u32;
        for b in w.iter_mut().rev() {
            let v = *b as u32 * 10 + carry;
            *b = v as u8;
            carry = v >> 8;
        }
        if carry != 0 {
            return None;
        }
    }
    Some(w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals() {
        assert_eq!(u256_from_dec("0"), Some([0; 32]));
        assert_eq!(u256_from_dec("258").unwrap()[30..], [1, 2]);
        let max = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
        assert_eq!(u256_from_dec(max), Some([0xff; 32]));
        assert_eq!(u256_from_dec("115792089237316195423570985008687907853269984665640564039457584007913129639936"), None);
        assert_eq!(u256_from_dec("12a"), None);
        assert_eq!(u256_from_dec(""), None);
    }

    #[test]
    fn bad_operation() {
        let tx = SafeTx { to: ZERO, value: [0; 32], data: &[], operation: 2, safe_tx_gas: [0; 32], base_gas: [0; 32],
                          gas_price: [0; 32], gas_token: ZERO, refund_receiver: ZERO, nonce: [0; 32] };
        assert_eq!(safe_tx_hash(1, &ZERO, &tx), Err(Error::Operation(2)));
    }
}
