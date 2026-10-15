//! The key ceremony's side of the chip (docs/CEREMONY.md): the commands that write, lock and make a
//! key, and the steps P1 to K4 built from them. Only the ceremony image uses this file; the signer
//! keeps `atecc.rs`, which only reads.
//!
//! Every step works on the chip through `atecc::Link`, so `cargo test` runs them on a virtual chip
//! (`core/tests/ceremony.rs`). Each step says what it did on `log`, one line at a time, for the page.
//! A step that finds something it doesn't expect stops (`Stop`) and says why.
use core::fmt::Write;

use crate::atecc::{self, Cmd, Error, Link, OP_GENKEY, OP_INFO, OP_READ};
use crate::hex;
use crate::safe_tx::{self, SafeTx};
use crate::webauthn;

// ----------------------------------------------------------------------------- the table
/// The rules, diykey v1: the config zone the MicroPython signer locked on 2026-10-07, byte for byte.
/// Bytes 0-15 and 84-87 are the chip's own and never written (0-15 here are a placeholder, as in
/// docs/CEREMONY.md, whose SHA-256 covers these 128 bytes).
pub const TABLE: [u8; 128] = [
    0x01, 0x23, 0x00, 0x00, 0x00, 0x00, 0x60, 0x00, 0x00, 0x00, 0x00, 0x00, 0xee, 0x00, 0x00, 0x00,
    0xc0, 0x00, 0x00, 0x00, 0xa1, 0x2f, 0x9f, 0x8f, 0x9f, 0x8f, 0x9f, 0x8f, 0x9f, 0x8f, 0x9f, 0x8f,
    0x9f, 0x8f, 0x9f, 0x8f, 0x1f, 0x8f, 0x9f, 0x8f, 0x9f, 0x8f, 0x9f, 0x8f, 0x9f, 0x8f, 0x9f, 0x8f,
    0x9f, 0x8f, 0x9f, 0x8f, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x55, 0x55, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x33, 0x00, 0x1c, 0x00, 0x1c, 0x00, 0x1c, 0x00, 0x1c, 0x00, 0x1c, 0x00, 0x1c, 0x00, 0x1c, 0x00,
    0x1c, 0x00, 0x1c, 0x00, 0x1c, 0x00, 0x1c, 0x00, 0x1c, 0x00, 0x1c, 0x00, 0x1c, 0x00, 0x1c, 0x00,
];

/// The only revision this table has run on: a 608A.
pub const REVISION_608A: [u8; 4] = [0x00, 0x00, 0x60, 0x02];
/// What Random answers while the config zone is open.
const FIXED_RANDOM: [u8; 4] = [0xff, 0xff, 0x00, 0x00];
const UNLOCKED: u8 = 0x55;
const LOCKED: u8 = 0x00;
/// Config bytes: the data zone's lock, the config zone's, and SlotLocked (one bit a slot, 0 = sealed).
const LOCK_DATA: usize = 86;
const LOCK_CONFIG: usize = 87;
const SLOT_LOCKED: usize = 88;

/// Bytes in each slot: 0-7 hold 36, 8 holds 416, 9-15 hold 72.
pub const fn slot_len(slot: u16) -> usize {
    match slot {
        0..=7 => 36,
        8 => 416,
        _ => 72,
    }
}

// ----------------------------------------------------------------------------- commands
pub const OP_WRITE: u8 = 0x12;
pub const OP_LOCK: u8 = 0x17;
pub const OP_NONCE: u8 = 0x16;
pub const OP_SIGN: u8 = 0x41;
pub const OP_RANDOM: u8 = 0x1B;
pub const OP_SELFTEST: u8 = 0x77;

/// 4 bytes into the config zone at byte `at` (a multiple of 4).
pub const fn write_config_word(at: usize) -> Cmd {
    Cmd { opcode: OP_WRITE, p1: 0x00, p2: (at / 4) as u16, resp_len: 1, wait_ms: 45 }
}

/// 32 bytes into config block `block`.
pub const fn write_config_block(block: u8) -> Cmd {
    Cmd { opcode: OP_WRITE, p1: 0x80, p2: (block as u16) << 3, resp_len: 1, wait_ms: 45 }
}

/// 32 bytes into `slot`, block `block`.
pub const fn write_slot_block(slot: u16, block: u16) -> Cmd {
    Cmd { opcode: OP_WRITE, p1: 0x82, p2: slot << 3 | block << 8, resp_len: 1, wait_ms: 45 }
}

/// 32 bytes into OTP block `block` (0 or 1).
pub const fn write_otp_block(block: u16) -> Cmd {
    Cmd { opcode: OP_WRITE, p1: 0x81, p2: block << 3, resp_len: 1, wait_ms: 45 }
}

pub const fn read_slot_block(slot: u16, block: u16) -> Cmd {
    Cmd { opcode: OP_READ, p1: 0x82, p2: slot << 3 | block << 8, resp_len: 32, wait_ms: 5 }
}

pub const fn read_otp_block(block: u16) -> Cmd {
    Cmd { opcode: OP_READ, p1: 0x81, p2: block << 3, resp_len: 32, wait_ms: 5 }
}

/// Lock the config zone, if what it holds has this CRC. A wrong CRC locks nothing.
pub const fn lock_config(crc: [u8; 2]) -> Cmd {
    Cmd { opcode: OP_LOCK, p1: 0x00, p2: u16::from_le_bytes(crc), resp_len: 1, wait_ms: 35 }
}

/// Lock the data and OTP zones, if what they hold has this CRC.
pub const fn lock_data(crc: [u8; 2]) -> Cmd {
    Cmd { opcode: OP_LOCK, p1: 0x01, p2: u16::from_le_bytes(crc), resp_len: 1, wait_ms: 35 }
}

/// Seal slot 0 alone: no CRC, as there's no knowing a private key's bytes.
pub const LOCK_SLOT_0: Cmd = Cmd { opcode: OP_LOCK, p1: 0x82, p2: 0, resp_len: 1, wait_ms: 35 };

/// GenKey mode 0x04: a new private key in `slot`, made by the chip; answers its public key.
pub const fn new_key(slot: u16) -> Cmd {
    Cmd { opcode: OP_GENKEY, p1: 0x04, p2: slot, resp_len: 64, wait_ms: 115 }
}

/// Nonce pass-through: 32 bytes into TempKey, for Sign. TempKey is gone after a sleep.
pub const NONCE_LOAD: Cmd = Cmd { opcode: OP_NONCE, p1: 0x03, p2: 0, resp_len: 1, wait_ms: 20 };

/// Sign the 32 bytes in TempKey with the key in `slot` (external message): r ‖ s.
pub const fn sign(slot: u16) -> Cmd {
    Cmd { opcode: OP_SIGN, p1: 0x80, p2: slot, resp_len: 64, wait_ms: 115 }
}

pub const RANDOM: Cmd = Cmd { opcode: OP_RANDOM, p1: 0x00, p2: 0, resp_len: 32, wait_ms: 23 };

/// SelfTest of ECDSA sign and verify (0x04) and SHA (0x20), Microchip's bits for them. Answers 0
/// when they pass.
pub const SELFTEST: Cmd = Cmd { opcode: OP_SELFTEST, p1: 0x24, p2: 0, resp_len: 1, wait_ms: 250 };

/// Info KeyValid: 1 if `slot` holds a valid ECC key. Not a use of the key.
pub const fn key_valid(slot: u16) -> Cmd {
    Cmd { opcode: OP_INFO, p1: 0x01, p2: slot, resp_len: 4, wait_ms: 5 }
}

// ----------------------------------------------------------------------------- what's written
/// The config zone the rules lock: the chip's own bytes 0-15, then the table. P1 has checked that
/// 84-87 hold the table's 00 00 55 55.
pub fn config_image(chip: &[u8]) -> [u8; 128] {
    let mut c = TABLE;
    c[..16].copy_from_slice(&chip[..16]);
    c
}

/// The CRC that P3 locks with, worked out from the table, never from a readback.
pub fn config_crc(chip: &[u8]) -> [u8; 2] {
    atecc::crc16(&config_image(chip))
}

/// The CRC as the 16-bit number the Lock command takes, the way the screen shows it.
pub fn crc_value(crc: [u8; 2]) -> u16 {
    u16::from_le_bytes(crc)
}

/// The record: 64 bytes in OTP, written at P4 and frozen by P5 (docs/CEREMONY.md, "The record").
/// Prepared by its owner, no brand.
pub fn record(serial: &[u8; 9], revision: &[u8; 4], config_crc: [u8; 2], commit: &[u8; 4]) -> [u8; 64] {
    let mut r = [0u8; 64];
    r[..4].copy_from_slice(&[b'D', b'K', 1, 1]);
    r[4..13].copy_from_slice(serial);
    r[13..17].copy_from_slice(revision);
    r[17..19].copy_from_slice(&crc_value(config_crc).to_be_bytes());
    r[19..23].copy_from_slice(commit);
    r
}

/// The CRC that P5 locks with: every slot that isn't a key (1-15, in order, all zeros), then OTP.
pub fn data_crc(record: &[u8; 64]) -> [u8; 2] {
    let mut crc = 0;
    for slot in 1..16 {
        let mut left = slot_len(slot);
        while left > 0 {
            let n = left.min(32);
            crc = atecc::crc16_update(crc, &[0u8; 32][..n]);
            left -= n;
        }
    }
    atecc::crc16_update(crc, record).to_le_bytes()
}

/// Whether a locked config zone holds these rules: everything the table sets, but the counters
/// (they count) and the locks.
pub fn holds_the_table(config: &[u8; 128]) -> bool {
    config[16..52] == TABLE[16..52]
        && config[68..86] == TABLE[68..86]
        && config[90..] == TABLE[90..]
        && matches!(config[SLOT_LOCKED..SLOT_LOCKED + 2], [0xff, 0xff] | [0xfe, 0xff])
}

/// The serial's last group of four, as the screen groups it ("0123 4a7b 9c2d 5e6f ee"): what's
/// typed from the sticker before a permanent step.
pub fn last_four(serial: &[u8; 9]) -> [u8; 4] {
    let mut h = [0u8; 18];
    hex::encode(serial, &mut h);
    [h[12], h[13], h[14], h[15]]
}

// ----------------------------------------------------------------------------- the test transactions
/// The Safe the test transactions are for: address zero, which no Safe can be, so nothing they sign
/// can approve anything. Base Sepolia.
pub const TEST_CHAIN: u64 = 84532;
pub const TEST_SAFE: safe_tx::Address = safe_tx::ZERO;
pub const TEST_DATA: &[u8] = b"diykey key ceremony: a test signature";

/// Test transaction `n` (1, 2 or 3): nothing to address zero, nonce n, with TEST_DATA. Its Safe hash,
/// worked out here.
pub fn test_hash(n: u8) -> [u8; 32] {
    let tx = SafeTx {
        to: safe_tx::ZERO,
        value: [0; 32],
        data: TEST_DATA,
        operation: safe_tx::CALL,
        safe_tx_gas: [0; 32],
        base_gas: [0; 32],
        gas_price: [0; 32],
        gas_token: safe_tx::ZERO,
        refund_receiver: safe_tx::ZERO,
        nonce: safe_tx::u256_from_u64(n as u64),
    };
    safe_tx::safe_tx_hash(TEST_CHAIN, &TEST_SAFE, &tx).unwrap_or([0; 32])
}

// ----------------------------------------------------------------------------- the chip, read
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stop {
    /// The chip didn't answer, or answered with an error, while doing this.
    Chip(&'static str, Error),
    /// Not a chip this ceremony prepares, and why. Nothing was written.
    Refused(&'static str),
    /// A check after a step didn't hold.
    Check(&'static str),
}

/// What the chip holds, as read at the start: from it the ceremony knows where it is.
#[derive(Clone, Copy)]
pub struct Chip {
    pub config: [u8; 128],
    pub revision: [u8; 4],
    /// The record in OTP, read once the data zone is locked.
    pub otp: Option<[u8; 64]>,
}

/// Where the ceremony is, worked out from the chip alone (decision 8): nothing is stored.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stage {
    /// Both zones open: P1.
    Fresh,
    /// The rules locked, the data zone open: P4 next.
    RulesLocked,
    /// Both locked, slot 0 not sealed: P6, then a new key.
    Prepared,
    /// Slot 0 sealed: the ceremony is done.
    Sealed,
}

impl Chip {
    pub fn serial(&self) -> [u8; 9] {
        atecc::serial(&self.config[..32])
    }
    pub fn config_locked(&self) -> bool {
        self.config[LOCK_CONFIG] == LOCKED
    }
    pub fn data_locked(&self) -> bool {
        self.config[LOCK_DATA] == LOCKED
    }
    pub fn sealed(&self) -> bool {
        self.config[SLOT_LOCKED] & 1 == 0
    }

    pub fn stage(&self) -> Result<Stage, Stop> {
        if self.revision != REVISION_608A {
            return Err(Stop::Refused(if self.revision[2..] == [0x60, 0x03] {
                "A 608B. This table has run on 608A chips only. Try a spare 608B first."
            } else {
                "Not an ATECC608A: its revision is one this ceremony doesn't know."
            }));
        }
        let lock_byte = |b: u8| b == LOCKED || b == UNLOCKED;
        if !lock_byte(self.config[LOCK_CONFIG]) || !lock_byte(self.config[LOCK_DATA]) {
            return Err(Stop::Refused("Its lock bytes are neither locked nor open."));
        }
        if !self.config_locked() {
            if self.data_locked() {
                return Err(Stop::Refused("Its data zone is locked and its rules aren't. Not a fresh chip."));
            }
            return Ok(Stage::Fresh);
        }
        if !holds_the_table(&self.config) {
            return Err(Stop::Refused("Its rules were locked before, and they aren't these. Nothing was written. Use a fresh chip."));
        }
        if !self.data_locked() {
            return Ok(Stage::RulesLocked);
        }
        match &self.otp {
            Some(r) if r[..4] == [b'D', b'K', 1, 1] && r[4..13] == self.serial() => {}
            _ => return Err(Stop::Refused("Its data was locked without a diykey record. Not a chip this ceremony prepared.")),
        }
        Ok(if self.sealed() { Stage::Sealed } else { Stage::Prepared })
    }
}

// ----------------------------------------------------------------------------- talking to it
/// `f` with the chip awake, then to sleep whatever happened. Each `awake` must fit inside the
/// chip's watchdog, 1.3 s.
fn awake<T>(l: &mut dyn Link, f: impl FnOnce(&mut dyn Link) -> Result<T, Stop>) -> Result<T, Stop> {
    l.wake().map_err(|e| Stop::Chip("waking the chip", e))?;
    let r = f(l);
    l.sleep();
    r
}

fn cmd(l: &mut dyn Link, what: &'static str, c: &Cmd, data: &[u8], out: &mut [u8]) -> Result<usize, Stop> {
    atecc::run(l, c, data, out).map_err(|e| Stop::Chip(what, e))
}

/// A command that answers only "done" (Write, Lock, Nonce).
fn done(l: &mut dyn Link, what: &'static str, c: &Cmd, data: &[u8]) -> Result<(), Stop> {
    let mut ok = [0u8; 1];
    match cmd(l, what, c, data, &mut ok)? {
        1 if ok[0] == 0 => Ok(()),
        _ => Err(Stop::Chip(what, Error::Length(1))),
    }
}

/// A command the chip must refuse, as a check: Ok when it does, and changes nothing.
fn refused(l: &mut dyn Link, c: &Cmd, data: &[u8], what: &'static str) -> Result<(), Stop> {
    let mut out = [0u8; 64];
    match atecc::run(l, c, data, &mut out) {
        Err(Error::Status(_)) => Ok(()),
        Err(e) => Err(Stop::Chip(what, e)),
        Ok(_) => Err(Stop::Check(what)),
    }
}

fn read_config(l: &mut dyn Link) -> Result<[u8; 128], Stop> {
    let mut c = [0u8; 128];
    for b in 0..4 {
        let n = cmd(l, "reading the config zone", &atecc::read_config(b), &[], &mut c[b as usize * 32..])?;
        if n != 32 {
            return Err(Stop::Chip("reading the config zone", Error::Length(n as u8)));
        }
    }
    Ok(c)
}

fn read_otp(l: &mut dyn Link) -> Result<[u8; 64], Stop> {
    let mut r = [0u8; 64];
    for b in 0..2 {
        cmd(l, "reading OTP", &read_otp_block(b), &[], &mut r[b as usize * 32..])?;
    }
    Ok(r)
}

fn counter0(l: &mut dyn Link) -> Result<u32, Stop> {
    let mut c = [0u8; 4];
    cmd(l, "reading counter 0", &atecc::read_counter(0), &[], &mut c)?;
    Ok(u32::from_le_bytes(c))
}

fn public_key(l: &mut dyn Link) -> Result<[u8; 64], Stop> {
    let mut k = [0u8; 64];
    match cmd(l, "reading slot 0's public key", &atecc::public_key(0), &[], &mut k)? {
        64 => Ok(k),
        n => Err(Stop::Chip("reading slot 0's public key", Error::Length(n as u8))),
    }
}

/// The WebAuthn digest of test transaction `n`'s hash, signed by `slot`, in one wake (TempKey).
fn sign_test(l: &mut dyn Link, n: u8, slot: u16) -> Result<[u8; 64], Stop> {
    let digest = webauthn::digest(&test_hash(n));
    done(l, "loading the digest (Nonce)", &NONCE_LOAD, &digest)?;
    let mut sig = [0u8; 64];
    match cmd(l, "signing", &sign(slot), &[], &mut sig)? {
        64 => Ok(sig),
        k => Err(Stop::Chip("signing", Error::Length(k as u8))),
    }
}

fn hexline(log: &mut dyn Write, b: &[u8]) {
    for x in b {
        let _ = write!(log, "{x:02x}");
    }
}

fn say_sig(log: &mut dyn Write, step: &str, n: u8, sig: &[u8; 64]) {
    let _ = write!(log, "{step}: sig {n} hash 0x");
    hexline(log, &test_hash(n));
    let _ = write!(log, " r 0x");
    hexline(log, &sig[..32]);
    let _ = write!(log, " s 0x");
    hexline(log, &sig[32..]);
    let _ = writeln!(log);
}

/// The chip as it is now, with the line the signer prints for it, so the page reads both alike.
pub fn read_chip(l: &mut dyn Link, log: &mut dyn Write) -> Result<Chip, Stop> {
    let chip = awake(l, |l| {
        let config = read_config(l)?;
        let mut revision = [0u8; 4];
        cmd(l, "reading the revision", &atecc::REVISION, &[], &mut revision)?;
        let otp = if config[LOCK_DATA] == LOCKED { Some(read_otp(l)?) } else { None };
        Ok(Chip { config, revision, otp })
    })?;
    let c = &chip;
    let _ = writeln!(log, "chip: serial {:02x?} revision {:02x?} config locked {} data locked {} sealed {}",
                     c.serial(), c.revision, c.config_locked(), c.data_locked(), c.sealed());
    Ok(chip)
}

// ----------------------------------------------------------------------------- part 1
pub struct P1 {
    pub config: [u8; 128],
    pub revision: [u8; 4],
}

/// P1, read the chip: a fresh 608A, both zones open, nothing set it shouldn't have, Random's fixed
/// pattern, SelfTest. Writes nothing. The factory config goes to the log.
pub fn p1(l: &mut dyn Link, log: &mut dyn Write) -> Result<P1, Stop> {
    let (config, revision, random, selftest) = awake(l, |l| {
        let config = read_config(l)?;
        let mut revision = [0u8; 4];
        cmd(l, "reading the revision", &atecc::REVISION, &[], &mut revision)?;
        let mut random = [0u8; 32];
        cmd(l, "Random", &RANDOM, &[], &mut random)?;
        let mut st = [0u8; 1];
        let selftest = atecc::run(l, &SELFTEST, &[], &mut st);
        Ok((config, revision, random, selftest))
    })?;
    let _ = write!(log, "p1: factory config 0x");
    hexline(log, &config);
    let _ = writeln!(log);
    let chip = Chip { config, revision, otp: None };
    if chip.stage()? != Stage::Fresh {
        return Err(Stop::Refused("Its zones aren't both open. Not a fresh chip."));
    }
    if config[84..88] != TABLE[84..88] {
        return Err(Stop::Refused("Its UserExtra or Selector byte was set. Not a fresh chip."));
    }
    if config[SLOT_LOCKED..SLOT_LOCKED + 2] != [0xff, 0xff] {
        return Err(Stop::Refused("A slot of it is sealed already. Not a fresh chip."));
    }
    if random.chunks(4).any(|c| c != FIXED_RANDOM) {
        return Err(Stop::Refused("Random didn't answer an open chip's fixed pattern."));
    }
    match selftest {
        Ok(1) => {}
        Ok(_) => return Err(Stop::Refused("SelfTest answered something odd.")),
        Err(Error::Status(_)) => return Err(Stop::Refused("SelfTest failed.")),
        Err(e) => return Err(Stop::Chip("SelfTest", e)),
    }
    let _ = writeln!(log, "p1: ok, a fresh 608A; selftest passed");
    Ok(P1 { config, revision })
}

/// P2, write the rules: the table into the config zone, but the chip's own bytes (0-15 and 84-87),
/// then read back. Returns the config CRC, worked out from the table. Can be run again.
pub fn p2(l: &mut dyn Link, chip: &[u8; 128], log: &mut dyn Write) -> Result<[u8; 2], Stop> {
    awake(l, |l| {
        let what = "writing the rules";
        // bytes 0-15 can't be written, so block 0 goes in 4 bytes at a time; block 2 too, around 84-87
        for at in (16..32).chain(64..84).chain(88..96).step_by(4) {
            done(l, what, &write_config_word(at), &TABLE[at..at + 4])?;
        }
        for b in [1u8, 3] {
            let at = b as usize * 32;
            done(l, what, &write_config_block(b), &TABLE[at..at + 32])?;
        }
        Ok(())
    })?;
    let back = awake(l, read_config)?;
    let image = config_image(chip);
    if back[16..84] != image[16..84] || back[88..] != image[88..] {
        return Err(Stop::Check("The rules read back differently from the table."));
    }
    let crc = config_crc(chip);
    let _ = writeln!(log, "p2: rules written and read back; config crc {:04x}", crc_value(crc));
    Ok(crc)
}

/// P3, lock the rules. PERMANENT. The chip checks `crc` first and locks nothing if one byte differs.
pub fn p3(l: &mut dyn Link, crc: [u8; 2], log: &mut dyn Write) -> Result<(), Stop> {
    awake(l, |l| done(l, "locking the rules", &lock_config(crc), &[]))?;
    let _ = writeln!(log, "p3: rules locked with crc {:04x}", crc_value(crc));
    let (config, random) = awake(l, |l| {
        let mut random = [0u8; 32];
        let config = read_config(l)?;
        cmd(l, "Random", &RANDOM, &[], &mut random)?;
        Ok((config, random))
    })?;
    if config[LOCK_CONFIG] != LOCKED || !holds_the_table(&config) {
        return Err(Stop::Check("After the lock, the config zone isn't the table, locked."));
    }
    if random.chunks(4).all(|c| c == FIXED_RANDOM) {
        return Err(Stop::Check("After the lock, Random still answers the fixed pattern."));
    }
    let _ = writeln!(log, "p3: ok, locked and unchanged; random live");
    Ok(())
}

/// P4, zeros and the record: zeros into slots 1-15 (32-byte writes: before the data lock the chip
/// refuses 4-byte writes into secret slots), the record into OTP. Returns the data CRC. Can be run
/// again until P5.
pub fn p4(l: &mut dyn Link, record: &[u8; 64], log: &mut dyn Write) -> Result<[u8; 2], Stop> {
    for slot in 1..16 {
        awake(l, |l| {
            for b in 0..slot_len(slot).div_ceil(32) as u16 {
                done(l, "writing zeros", &write_slot_block(slot, b), &[0u8; 32])?;
            }
            Ok(())
        })?;
    }
    awake(l, |l| {
        for b in 0..2u16 {
            let at = b as usize * 32;
            done(l, "writing the record", &write_otp_block(b), &record[at..at + 32])?;
        }
        Ok(())
    })?;
    let crc = data_crc(record);
    let _ = write!(log, "p4: zeros in slots 1-15; record 0x");
    hexline(log, record);
    let _ = writeln!(log, "; data crc {:04x}", crc_value(crc));
    Ok(crc)
}

/// P5, lock the data. PERMANENT. Then: locked, the record and slot 8 read in the clear, slot 1
/// can't be read.
pub fn p5(l: &mut dyn Link, crc: [u8; 2], record: &[u8; 64], log: &mut dyn Write) -> Result<(), Stop> {
    awake(l, |l| done(l, "locking the data", &lock_data(crc), &[]))?;
    let _ = writeln!(log, "p5: data locked with crc {:04x}", crc_value(crc));
    awake(l, |l| {
        let config = read_config(l)?;
        if config[LOCK_DATA] != LOCKED {
            return Err(Stop::Check("After the lock, the data zone reads open."));
        }
        if read_otp(l)? != *record {
            return Err(Stop::Check("After the lock, OTP isn't the record."));
        }
        let mut s8 = [0u8; 32];
        cmd(l, "reading slot 8", &read_slot_block(8, 0), &[], &mut s8)?;
        if s8 != [0; 32] {
            return Err(Stop::Check("After the lock, slot 8 isn't zeros."));
        }
        refused(l, &read_slot_block(1, 0), &[], "Slot 1 could be read: it should be secret.")
    })?;
    let _ = writeln!(log, "p5: ok, record and slot 8 read back; slot 1 refuses a read");
    Ok(())
}

pub struct P6 {
    /// Info says slot 0 holds a valid key already (None: Info didn't say). K1 replaces it either way.
    pub key_in_slot0: Option<bool>,
    pub counter0: u32,
}

/// P6, check: both zones locked, the table, the record, slot 0 not sealed, counter 0. Info looks at
/// slot 0 without counting as a use.
pub fn p6(l: &mut dyn Link, log: &mut dyn Write) -> Result<P6, Stop> {
    let (config, otp, valid, counter0) = awake(l, |l| {
        let config = read_config(l)?;
        let otp = read_otp(l)?;
        // only shown, so an answer it doesn't expect doesn't stop the ceremony
        let mut v = [0u8; 4];
        let valid = atecc::run(l, &key_valid(0), &[], &mut v).ok().map(|_| v[0] == 1);
        Ok((config, otp, valid, counter0(l)?))
    })?;
    let chip = Chip { config, revision: REVISION_608A, otp: Some(otp) };
    if chip.stage()? != Stage::Prepared {
        return Err(Stop::Check("The chip isn't prepared: both zones locked, slot 0 not sealed."));
    }
    if otp[17..19] != crc_value(config_crc(&config)).to_be_bytes() {
        return Err(Stop::Check("The record's config CRC isn't this chip's."));
    }
    let _ = writeln!(log, "p6: ok, both zones locked with the table and the record; key in slot 0 {}; counter0 {counter0}",
                     match valid { Some(true) => "true", Some(false) => "false", None => "unknown" });
    Ok(P6 { key_in_slot0: valid, counter0 })
}

// ----------------------------------------------------------------------------- part 2
/// K1, make your key: GenKey mode 0x04 on slot 0, whatever is there. Then its public key reads
/// back, and GenKey on slot 1 is refused.
pub fn k1(l: &mut dyn Link, log: &mut dyn Write) -> Result<[u8; 64], Stop> {
    let key = awake(l, |l| {
        let mut key = [0u8; 64];
        if cmd(l, "making the key (GenKey)", &new_key(0), &[], &mut key)? != 64 {
            return Err(Stop::Chip("making the key (GenKey)", Error::Length(1)));
        }
        if public_key(l)? != key {
            return Err(Stop::Check("Slot 0's public key read back differently."));
        }
        refused(l, &new_key(1), &[], "Slot 1 took GenKey: it should refuse.")?;
        Ok(key)
    })?;
    let _ = write!(log, "k1: pubkey 0x");
    hexline(log, &key);
    let _ = writeln!(log);
    Ok(key)
}

pub struct K2 {
    pub sigs: [[u8; 64]; 2],
    pub counter_before: u32,
    pub counter_after: u32,
}

/// K2, test it: the key signs test transactions 1 and 2, each the WebAuthn digest of a Safe hash
/// worked out here, never a digest from the computer. Slot 1 refuses to sign. The page checks the
/// signatures; the device has no P-256 code to.
pub fn k2(l: &mut dyn Link, log: &mut dyn Write) -> Result<K2, Stop> {
    let counter_before = awake(l, counter0)?;
    let mut sigs = [[0u8; 64]; 2];
    for (i, s) in sigs.iter_mut().enumerate() {
        *s = awake(l, |l| sign_test(l, i as u8 + 1, 0))?;
        say_sig(log, "k2", i as u8 + 1, s);
    }
    let counter_after = awake(l, |l| {
        let digest = webauthn::digest(&test_hash(1));
        done(l, "loading the digest (Nonce)", &NONCE_LOAD, &digest)?;
        refused(l, &sign(1), &[], "Slot 1 signed: it should refuse.")?;
        counter0(l)
    })?;
    let _ = writeln!(log, "k2: slot 1 refuses to sign; counter0 {counter_before} -> {counter_after}");
    Ok(K2 { sigs, counter_before, counter_after })
}

/// K3, seal it. PERMANENT. First the key in slot 0 must still be `key`, the one K2 tested. Then
/// SlotLocked reads FE FF, the key is unchanged, and it signs test transaction 3.
pub fn k3(l: &mut dyn Link, key: &[u8; 64], log: &mut dyn Write) -> Result<[u8; 64], Stop> {
    awake(l, |l| {
        if public_key(l)? != *key {
            return Err(Stop::Check("Slot 0 holds a different key from the one tested. Nothing sealed."));
        }
        done(l, "sealing slot 0", &LOCK_SLOT_0, &[])
    })?;
    let _ = writeln!(log, "k3: slot 0 sealed");
    let sig = awake(l, |l| {
        let config = read_config(l)?;
        if config[SLOT_LOCKED..SLOT_LOCKED + 2] != [0xfe, 0xff] {
            return Err(Stop::Check("After the seal, SlotLocked doesn't read FE FF."));
        }
        if public_key(l)? != *key {
            return Err(Stop::Check("After the seal, slot 0's key is a different one."));
        }
        sign_test(l, 3, 0)
    })?;
    say_sig(log, "k3", 3, &sig);
    Ok(sig)
}

/// K4, its address: the Safe signer for the key, worked out here. The page works it out too.
pub fn k4(key: &[u8; 64], log: &mut dyn Write) -> safe_tx::Address {
    let a = crate::safe_signer::address(key);
    let mut s = [0u8; 42];
    let _ = writeln!(log, "k4: address {}", crate::safe_signer::checksum(&a, &mut s));
    a
}

/// A sealed chip's key, to show its address again: GenKey mode 0, which counts as a use.
pub fn sealed_key(l: &mut dyn Link, log: &mut dyn Write) -> Result<[u8; 64], Stop> {
    let key = awake(l, public_key)?;
    let _ = write!(log, "k4: pubkey 0x");
    hexline(log, &key);
    let _ = writeln!(log);
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hexs(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    // The table in docs/CEREMONY.md, and its SHA-256 as written there.
    #[test]
    fn the_table() {
        assert_eq!(hexs(&crate::sha256::sha256(&[&TABLE])), "36da855657a3ee7e098f27cd17b2a636c9e51adec02d252ee0adbe905b8113e1");
        let doc = include_str!("../../docs/CEREMONY.md");
        for (i, row) in TABLE.chunks(16).enumerate() {
            let want = format!("{:>3}: {}", i * 16, row.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "));
            assert!(doc.contains(&want), "row {i}: {want}");
        }
        // slot 0: P-256, private, external sign only, GenKey and never PrivWrite or Write, lockable
        assert_eq!(TABLE[20..22], [0xa1, 0x2f]);
        assert_eq!(TABLE[96..98], [0x33, 0x00]);
        assert_eq!(TABLE[84..88], [0x00, 0x00, 0x55, 0x55]);
        assert_eq!(TABLE[88..90], [0xff, 0xff]);
    }

    // The bytes each command puts on the wire, each worked out by hand from the datasheet's layout.
    #[test]
    fn packets() {
        let mut out = [0u8; 48];
        let mut wire = |c: Cmd, data: &[u8]| {
            let n = atecc::packet(c.opcode, c.p1, c.p2, data, &mut out);
            hexs(&out[1..6]) + "/" + &(n - 8).to_string()
        };
        assert_eq!(wire(write_config_word(16), &[0; 4]), "0b12000400/4");
        assert_eq!(wire(write_config_word(88), &[0; 4]), "0b12001600/4");
        assert_eq!(wire(write_config_block(3), &[0; 32]), "2712801800/32");
        assert_eq!(wire(write_slot_block(8, 12), &[0; 32]), "271282400c/32");
        assert_eq!(wire(write_otp_block(1), &[0; 32]), "2712810800/32");
        assert_eq!(wire(read_slot_block(1, 0), &[]), "0702820800/0");
        assert_eq!(wire(lock_config([0x34, 0x12]), &[]), "0717003412/0");
        assert_eq!(wire(lock_data([0xcd, 0xab]), &[]), "071701cdab/0");
        assert_eq!(wire(LOCK_SLOT_0, &[]), "0717820000/0");
        assert_eq!(wire(new_key(0), &[]), "0740040000/0");
        assert_eq!(wire(NONCE_LOAD, &[0; 32]), "2716030000/32");
        assert_eq!(wire(sign(0), &[]), "0741800000/0");
        assert_eq!(wire(key_valid(0), &[]), "0730010000/0");
    }

    #[test]
    fn crcs_and_the_record() {
        let serial = [0x01, 0x23, 0x4a, 0x7b, 0x9c, 0x2d, 0x5e, 0x6f, 0xee];
        let mut chip = [0u8; 16];
        chip[..4].copy_from_slice(&serial[..4]);
        chip[4..8].copy_from_slice(&REVISION_608A);
        chip[8..13].copy_from_slice(&serial[4..]);
        chip[14] = 0x01;
        let image = config_image(&chip);
        assert_eq!(image[..16], chip);
        assert_eq!(image[16..], TABLE[16..]);
        let crc = config_crc(&chip);
        assert_eq!(crc, atecc::crc16(&image));
        let r = record(&serial, &REVISION_608A, crc, &[0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(&r[..4], b"DK\x01\x01");
        assert_eq!(r[17..19], crc_value(crc).to_be_bytes());
        assert_eq!(r[23..], [0; 41]);
        let mut all = vec![0u8; 7 * 36 + 416 + 7 * 72];
        all.extend_from_slice(&r);
        assert_eq!(data_crc(&r), atecc::crc16(&all));
        assert_eq!(&last_four(&serial), b"5e6f");
    }

    #[test]
    fn test_transactions_differ() {
        assert_ne!(test_hash(1), test_hash(2));
        assert_ne!(test_hash(2), test_hash(3));
        assert_ne!(test_hash(1), [0; 32]);
    }
}
