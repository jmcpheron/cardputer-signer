//! The line protocol over USB. One line of text each way, so a person can read it in a serial
//! monitor and the device needs no parser crate.
//!
//! From the computer:
//!   ping
//!   tx chain=84532 safe=0x… to=0x… value=0 data=0x op=0 nonce=12 [safetxgas=0 basegas=0
//!      gasprice=0 gastoken=0x… refundreceiver=0x…] [expect=0x…]
//! Amounts are decimal, addresses and data hex. `expect` is the hash the computer says this is:
//! the device only compares it with the one it works out, and never signs it.
//!
//! From the device, each reply starts with "= ": `= pong …`, `= hash 0x…`, `= mismatch`,
//! `= rejected`, `= busy`, `= error …`. Anything else is a log line.
use crate::hex;
use crate::safe_tx::{u256_from_dec, Address, SafeTx, U256, ZERO};

/// Every field of a Safe transaction but the calldata, which `parse` puts in the caller's buffer.
#[derive(Clone, Copy)]
pub struct TxFields {
    pub chain: u64,
    pub safe: Address,
    pub to: Address,
    pub value: U256,
    pub data_len: usize,
    pub operation: u8,
    pub safe_tx_gas: U256,
    pub base_gas: U256,
    pub gas_price: U256,
    pub gas_token: Address,
    pub refund_receiver: Address,
    pub nonce: U256,
    pub expect: Option<[u8; 32]>,
}

impl TxFields {
    pub fn tx<'a>(&self, data: &'a [u8]) -> SafeTx<'a> {
        SafeTx {
            to: self.to,
            value: self.value,
            data: &data[..self.data_len],
            operation: self.operation,
            safe_tx_gas: self.safe_tx_gas,
            base_gas: self.base_gas,
            gas_price: self.gas_price,
            gas_token: self.gas_token,
            refund_receiver: self.refund_receiver,
            nonce: self.nonce,
        }
    }
}

pub enum Cmd {
    Ping,
    Tx(TxFields),
}

#[derive(Debug, PartialEq)]
pub enum Error {
    Unknown,
    Missing(&'static str),
    Bad(&'static str),
    DataTooLong,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            Error::Unknown => write!(f, "unknown command"),
            Error::Missing(k) => write!(f, "missing {}", k),
            Error::Bad(k) => write!(f, "bad {}", k),
            Error::DataTooLong => write!(f, "data too long"),
        }
    }
}

fn address(v: &str, k: &'static str) -> Result<Address, Error> {
    let mut a = [0u8; 20];
    hex::decode(v, &mut a).ok_or(Error::Bad(k))?;
    Ok(a)
}

fn number(v: &str, k: &'static str) -> Result<U256, Error> {
    u256_from_dec(v).ok_or(Error::Bad(k))
}

pub fn parse(line: &str, data: &mut [u8]) -> Result<Cmd, Error> {
    let mut words = line.split_ascii_whitespace();
    match words.next() {
        Some("ping") => return Ok(Cmd::Ping),
        Some("tx") => {}
        _ => return Err(Error::Unknown),
    }
    let mut t = TxFields {
        chain: 0,
        safe: ZERO,
        to: ZERO,
        value: [0; 32],
        data_len: 0,
        operation: 0,
        safe_tx_gas: [0; 32],
        base_gas: [0; 32],
        gas_price: [0; 32],
        gas_token: ZERO,
        refund_receiver: ZERO,
        nonce: [0; 32],
        expect: None,
    };
    let mut seen = 0u8;
    for w in words {
        let (k, v) = w.split_once('=').ok_or(Error::Bad("field"))?;
        match k {
            "chain" => {
                t.chain = v.parse().map_err(|_| Error::Bad("chain"))?;
                seen |= 1;
            }
            "safe" => {
                t.safe = address(v, "safe")?;
                seen |= 2;
            }
            "to" => {
                t.to = address(v, "to")?;
                seen |= 4;
            }
            "value" => {
                t.value = number(v, "value")?;
                seen |= 8;
            }
            "data" => {
                let h = v.strip_prefix("0x").unwrap_or(v);
                let n = h.len() / 2;
                if n > data.len() {
                    return Err(Error::DataTooLong);
                }
                hex::decode(h, &mut data[..n]).ok_or(Error::Bad("data"))?;
                t.data_len = n;
                seen |= 16;
            }
            "op" => {
                t.operation = match v {
                    "0" => 0,
                    "1" => 1,
                    _ => return Err(Error::Bad("op")),
                };
                seen |= 32;
            }
            "nonce" => {
                t.nonce = number(v, "nonce")?;
                seen |= 64;
            }
            "safetxgas" => t.safe_tx_gas = number(v, "safetxgas")?,
            "basegas" => t.base_gas = number(v, "basegas")?,
            "gasprice" => t.gas_price = number(v, "gasprice")?,
            "gastoken" => t.gas_token = address(v, "gastoken")?,
            "refundreceiver" => t.refund_receiver = address(v, "refundreceiver")?,
            "expect" => {
                let mut h = [0u8; 32];
                hex::decode(v, &mut h).ok_or(Error::Bad("expect"))?;
                t.expect = Some(h);
            }
            _ => return Err(Error::Bad("field")),
        }
    }
    for (bit, k) in [(1, "chain"), (2, "safe"), (4, "to"), (8, "value"), (16, "data"), (32, "op"), (64, "nonce")] {
        if seen & bit == 0 {
            return Err(Error::Missing(k));
        }
    }
    Ok(Cmd::Tx(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = "tx chain=84532 safe=0x5afe9c2d71b04e83a6f1d2c7e9b0a4f3168d2e57 \
        to=0x12ab5e7f90c3d1a84b6e2f0d9c7a3b51e48f89cd value=10000000000000000 data=0x op=0 nonce=12";

    #[test]
    fn a_send() {
        let mut d = [0u8; 64];
        let Ok(Cmd::Tx(t)) = parse(LINE, &mut d) else { panic!() };
        assert_eq!(t.chain, 84532);
        assert_eq!(t.data_len, 0);
        assert_eq!(t.nonce[31], 12);
        assert!(t.expect.is_none());
    }

    #[test]
    fn refusals() {
        let mut d = [0u8; 4];
        assert!(matches!(parse("ping", &mut d), Ok(Cmd::Ping)));
        assert_eq!(parse("sign 0x00", &mut d).err(), Some(Error::Unknown));
        assert_eq!(parse("tx chain=1", &mut d).err(), Some(Error::Missing("safe")));
        assert_eq!(parse(&LINE.replace("op=0", "op=2"), &mut d).err(), Some(Error::Bad("op")));
        assert_eq!(parse(&LINE.replace("data=0x", "data=0x0102030405"), &mut d).err(), Some(Error::DataTooLong));
        assert_eq!(parse(&LINE.replace("value=", "value=-"), &mut d).err(), Some(Error::Bad("value")));
        assert_eq!(parse(&(LINE.to_owned() + " hash=0x00"), &mut d).err(), Some(Error::Bad("field")));
    }
}
