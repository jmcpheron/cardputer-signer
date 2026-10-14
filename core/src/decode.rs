//! What a Safe transaction does, in words, and how worried to be: ok, warn or red. The wording
//! and the rules follow the MicroPython decoder this was ported from, and `tests/vectors.rs` holds
//! this to its summaries for the same vectors (the `expect` in `vectors/safe_tx.json`). Anything this can't read is red: don't sign what you can't read.
use core::fmt::Write;

use crate::buf::Buf;
use crate::num::{is_zero, units};
use crate::safe_tx::{Address, SafeTx, DELEGATECALL, ZERO};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
    Ok,
    Warn,
    Red,
}

#[derive(Clone, Copy)]
pub struct Item {
    pub title: Buf<24>,
    pub level: Level,
    /// one line: "Send 1.5 ETH"
    pub what: Buf<160>,
    /// the amount alone, big on its page: "1.5 ETH"
    pub amount: Buf<120>,
    pub to: Option<Address>,
    /// what `to` is: "to", "spender", "contract", "paid to"
    pub label: &'static str,
    pub note: &'static str,
}

impl Item {
    fn new(title: &str, level: Level, what: &str) -> Self {
        Item { title: Buf::from(title), level, what: Buf::from(what), amount: Buf::new(), to: None, label: "to", note: "" }
    }
}

pub struct Review {
    pub items: [Option<Item>; 2],
    pub level: Level,
}

impl Review {
    pub fn summary(&self) -> &str {
        self.items[0].as_ref().map(|i| i.what.as_str()).unwrap_or("")
    }

    pub fn count(&self) -> usize {
        self.items.iter().filter(|i| i.is_some()).count()
    }
}

/// Circle's USDC on the chains this knows: (symbol, decimals).
pub fn token(chain: u64, a: &Address) -> Option<(&'static str, usize)> {
    const USDC: [(u64, [u8; 20]); 3] = [
        (1, hex20(b"a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48")),
        (8453, hex20(b"833589fcd6edb6e08f4c7c32d4f71b54bda02913")),
        (84532, hex20(b"036cbd53842c5426634e7929541ec2318f3dcf7e")),
    ];
    USDC.iter().find(|(c, t)| *c == chain && t == a).map(|_| ("USDC", 6))
}

const fn hex20(s: &[u8; 40]) -> [u8; 20] {
    const fn n(c: u8) -> u8 {
        if c <= b'9' { c - b'0' } else { c - b'a' + 10 }
    }
    let mut out = [0u8; 20];
    let mut i = 0;
    while i < 20 {
        out[i] = n(s[2 * i]) << 4 | n(s[2 * i + 1]);
        i += 1;
    }
    out
}

const DONT: &str = "This console can't read this call. Don't sign what you can't read.";

fn word(data: &[u8], i: usize) -> Option<[u8; 32]> {
    let s = data.get(4 + 32 * i..4 + 32 * (i + 1))?;
    let mut w = [0u8; 32];
    w.copy_from_slice(s);
    Some(w)
}

fn addr(data: &[u8], i: usize) -> Option<Address> {
    let w = word(data, i)?;
    if w[..12].iter().any(|&b| b != 0) {
        return None;
    }
    let mut a = [0u8; 20];
    a.copy_from_slice(&w[12..]);
    Some(a)
}

fn token_call(chain: u64, to: &Address, data: &[u8]) -> Option<Item> {
    let tok = token(chain, to);
    let (sym, dec) = tok.unwrap_or(("TOKEN", 0));
    let level = if tok.is_some() { Level::Ok } else { Level::Warn };
    let note = if tok.is_some() { "" } else { "This token isn't in the console's list, so the amount is in raw units." };
    let (verb, title, who, amt, label) = match data[..4] {
        [0xa9, 0x05, 0x9c, 0xbb] => ("Send", "SEND ", addr(data, 0)?, word(data, 1)?, "to"),
        [0x09, 0x5e, 0xa7, 0xb3] => ("Approve", "APPROVE ", addr(data, 0)?, word(data, 1)?, "spender"),
        [0x23, 0xb8, 0x72, 0xdd] => ("Move", "MOVE ", addr(data, 1)?, word(data, 2)?, "to"),
        _ => return None,
    };
    let mut it = Item::new("", level, "");
    let _ = write!(it.title, "{}{}", title, sym);
    it.to = Some(who);
    it.label = label;
    it.note = note;
    if verb == "Approve" && amt[0] >= 0x80 {
        // at or above 2**255: "forever" (max uint256 is the usual one)
        it.level = Level::Red;
        let _ = write!(it.what, "Approve UNLIMITED {}", sym);
        let _ = write!(it.amount, "UNLIMITED {}", sym);
        it.note = "The spender can take every one of these the Safe holds, now and later, until someone revokes it.";
        return Some(it);
    }
    let _ = units(&amt, dec, &mut it.amount);
    let _ = write!(it.amount, " {}", sym);
    let _ = write!(it.what, "{} {}", verb, it.amount);
    if verb == "Approve" && tok.is_some() {
        it.note = "The spender may take up to this much from the Safe.";
    }
    Some(it)
}

fn action(chain: u64, safe: &Address, tx: &SafeTx) -> Item {
    if tx.operation == DELEGATECALL {
        let mut it = Item::new("DELEGATECALL", Level::Red, "Delegatecall");
        it.to = Some(tx.to);
        it.label = "code from";
        it.note = "The Safe runs this contract's code as its own: it can change anything.";
        return it;
    }
    if tx.data.is_empty() {
        if tx.to == *safe && is_zero(&tx.value) {
            let mut it = Item::new("REJECTION", Level::Ok, "On-chain rejection");
            it.note = "Uses up this nonce, so the transaction it replaces can never run.";
            return it;
        }
        let mut it = Item::new("SEND ETH", Level::Ok, "");
        let _ = units(&tx.value, 18, &mut it.amount);
        let _ = write!(it.amount, " ETH");
        let _ = write!(it.what, "Send {}", it.amount);
        it.to = Some(tx.to);
        return it;
    }
    if tx.to == *safe {
        let mut it = Item::new("SAFE SETTINGS", Level::Red, "Changes this Safe");
        it.note = "Owners, threshold, modules or code: this Safe's own settings.";
        return it;
    }
    if tx.data.len() >= 4 && is_zero(&tx.value) {
        if let Some(it) = token_call(chain, &tx.to, tx.data) {
            return it;
        }
    }
    let mut it = Item::new("UNKNOWN CALL", Level::Red, "Unknown call");
    it.to = Some(tx.to);
    it.label = "contract";
    it.note = DONT;
    if !is_zero(&tx.value) {
        let _ = units(&tx.value, 18, &mut it.amount);
        let _ = write!(it.amount, " ETH");
    }
    it
}

fn gas(tx: &SafeTx) -> Option<Item> {
    if !is_zero(&tx.gas_price) {
        let mut it = Item::new("GAS REFUND", Level::Red, "Pays a gas refund");
        it.note = "The Safe pays whoever executes this for the gas, at a price set here.";
        if tx.refund_receiver != ZERO {
            it.to = Some(tx.refund_receiver);
            it.label = "paid to";
        }
        return Some(it);
    }
    if !is_zero(&tx.base_gas) || !is_zero(&tx.safe_tx_gas) || tx.gas_token != ZERO || tx.refund_receiver != ZERO {
        let mut it = Item::new("GAS FIELDS", Level::Warn, "Unusual gas fields");
        it.note = "No refund is paid, because gasPrice is 0.";
        return Some(it);
    }
    None
}

pub fn review(chain: u64, safe: &Address, tx: &SafeTx) -> Review {
    let a = action(chain, safe, tx);
    let g = gas(tx);
    let level = g.as_ref().map_or(a.level, |g| a.level.max(g.level));
    Review { items: [Some(a), g], level }
}
