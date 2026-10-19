//! The ATECC608's I2C packets, ported from the MicroPython driver `atecc.py`, which started in
//! picowallet by Austin Griffith (MIT, github.com/austintgriffith/picowallet). Only commands that
//! read: nothing here can write, lock, or make a key. GenKey appears only in its mode 0, which
//! works out the public key of the private key already in a slot and changes nothing. The firmware sends what `packet` builds and
//! checks the answer with `response`.
//!
//! A command: word address 0x03, then count, opcode, p1, p2 (little-endian), data, CRC-16.
//! An answer: count, payload, CRC-16. A 1-byte payload that isn't 0 is a status code.

pub const ADDR: u8 = 0x60;
/// What a chip says right after the wake token.
pub const WAKE_OK: [u8; 4] = [0x04, 0x11, 0x33, 0x43];

pub const OP_READ: u8 = 0x02;
pub const OP_INFO: u8 = 0x30;
pub const OP_COUNTER: u8 = 0x24;
pub const OP_GENKEY: u8 = 0x40;

/// The chip's CRC-16 (poly 0x8005, bits taken low first), little-endian.
pub fn crc16(data: &[u8]) -> [u8; 2] {
    crc16_update(0, data).to_le_bytes()
}

/// The CRC-16 carried on over more data: for a zone too big to hold at once.
pub fn crc16_update(crc: u16, data: &[u8]) -> u16 {
    let mut crc = crc;
    for &b in data {
        for shift in 0..8 {
            let data_bit = (b >> shift) & 1;
            let crc_bit = (crc >> 15) as u8 & 1;
            crc <<= 1;
            if data_bit != crc_bit {
                crc ^= 0x8005;
            }
        }
    }
    crc
}

/// One command packet into `out`; returns its length (8 + data.len()).
pub fn packet(opcode: u8, p1: u8, p2: u16, data: &[u8], out: &mut [u8]) -> usize {
    let n = 7 + data.len();
    out[0] = 0x03;
    out[1] = n as u8;
    out[2] = opcode;
    out[3] = p1;
    out[4..6].copy_from_slice(&p2.to_le_bytes());
    out[6..6 + data.len()].copy_from_slice(data);
    let crc = crc16(&out[1..n - 1]);
    out[n - 1..n + 1].copy_from_slice(&crc);
    n + 1
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Error {
    /// the count byte is impossible for what was read
    Length(u8),
    /// the answer's CRC doesn't match
    Crc,
    /// the chip answered with a status code (`explain` says what it means)
    Status(u8),
    /// no answer to the wake token, or the bus failed
    NoChip,
}

/// The payload of an answer, checked. A nonzero 1-byte payload is a status code: `Err(Status)`.
pub fn response(resp: &[u8]) -> Result<&[u8], Error> {
    let n = *resp.first().ok_or(Error::Length(0))? as usize;
    if n < 4 || n > resp.len() {
        return Err(Error::Length(n as u8));
    }
    if crc16(&resp[..n - 2]) != resp[n - 2..n] {
        return Err(Error::Crc);
    }
    let payload = &resp[1..n - 2];
    if payload.len() == 1 && payload[0] != 0 {
        return Err(Error::Status(payload[0]));
    }
    Ok(payload)
}

pub fn explain(status: u8) -> &'static str {
    match status {
        0x01 => "checkmac or verify failed",
        0x03 => "parse error",
        0x05 => "ecc fault",
        0x07 => "self test error",
        0x08 => "health test error",
        0x0F => "execution error",
        0x11 => "after wake",
        0xEE => "watchdog about to expire",
        0xFF => "crc or communication error",
        _ => "unknown status",
    }
}

/// The commands the signer sends, each with the answer length and how long to wait before reading.
pub struct Cmd {
    pub opcode: u8,
    pub p1: u8,
    pub p2: u16,
    pub resp_len: usize,
    pub wait_ms: u32,
}

/// 32 bytes of the config zone, block 0..3.
pub const fn read_config(block: u8) -> Cmd {
    Cmd { opcode: OP_READ, p1: 0x80, p2: (block as u16) << 3, resp_len: 32, wait_ms: 2 }
}

/// Info mode 0: the revision, 4 bytes. 00006002 is a 608A, 00006003 a 608B, 00006005 a 608C.
pub const REVISION: Cmd = Cmd { opcode: OP_INFO, p1: 0x00, p2: 0, resp_len: 4, wait_ms: 2 };

/// Counter `idx` (0 or 1), read only (mode 0): 4 bytes, little-endian.
pub const fn read_counter(idx: u16) -> Cmd {
    Cmd { opcode: OP_COUNTER, p1: 0x00, p2: idx, resp_len: 4, wait_ms: 20 }
}

/// The public key (x ‖ y, 64 bytes) of the private key in `slot`: GenKey mode 0, which only
/// computes it. Mode 0x04 would make a new key and destroy the old one; the mode here is fixed at
/// 0, and `public_key_is_mode_0` holds it there. Fails on an empty slot, or one whose PubInfo is clear.
pub const fn public_key(slot: u16) -> Cmd {
    Cmd { opcode: OP_GENKEY, p1: 0x00, p2: slot, resp_len: 64, wait_ms: 120 }
}

/// The wire to the chip: the firmware's I2C, or a virtual chip in the tests.
pub trait Link {
    /// The wake token, and the chip's answer to it. The chip's watchdog then gives it 1.3 s.
    fn wake(&mut self) -> Result<(), Error>;
    /// To sleep: the chip forgets TempKey.
    fn sleep(&mut self);
    /// A command packet to the chip, then after `wait_ms` its answer into `resp` (count byte first).
    fn exchange(&mut self, pkt: &[u8], wait_ms: u32, resp: &mut [u8]) -> Result<(), Error>;
}

/// One command, with `data` (at most 32 bytes), over `link`; its payload into `out`.
pub fn run(link: &mut dyn Link, cmd: &Cmd, data: &[u8], out: &mut [u8]) -> Result<usize, Error> {
    let mut pkt = [0u8; 8 + 32];
    let n = packet(cmd.opcode, cmd.p1, cmd.p2, data, &mut pkt);
    let len = cmd.resp_len + 3;
    let mut resp = [0u8; 64 + 3];
    link.exchange(&pkt[..n], cmd.wait_ms, &mut resp[..len])?;
    let p = response(&resp[..len])?;
    out[..p.len()].copy_from_slice(p);
    Ok(p.len())
}

/// The 9-byte serial number out of config block 0.
pub fn serial(block0: &[u8]) -> [u8; 9] {
    let mut s = [0u8; 9];
    s[..4].copy_from_slice(&block0[0..4]);
    s[4..].copy_from_slice(&block0[8..13]);
    s
}

/// (config zone locked, data zone locked) from config block 2 (bytes 64..95).
pub fn locks(block2: &[u8]) -> (bool, bool) {
    (block2[87 - 64] == 0x00, block2[86 - 64] == 0x00)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_of_the_wake_answer() {
        assert_eq!(crc16(&WAKE_OK[..2]), [0x33, 0x43]);
    }

    // The bytes the MicroPython atecc.py sends for the same commands.
    #[test]
    fn packets_match_the_python_driver() {
        let mut out = [0u8; 16];
        for (cmd, want) in [
            (read_config(0), "03070280000009ad"),
            (REVISION, "030730000000035d"),
            (read_counter(0), "0307240000000cfd"),
            (public_key(0), "0307400000000005"),
        ] {
            let n = packet(cmd.opcode, cmd.p1, cmd.p2, &[], &mut out);
            let mut h = [0u8; 32];
            assert_eq!(crate::hex::encode(&out[..n], &mut h), want);
        }
    }

    #[test]
    fn public_key_is_mode_0() {
        for slot in 0..16 {
            let c = public_key(slot);
            assert_eq!((c.opcode, c.p1, c.p2), (OP_GENKEY, 0x00, slot)); // never 0x04: a new key
        }
    }

    #[test]
    fn answers() {
        assert_eq!(response(&WAKE_OK), Err(Error::Status(0x11)));
        let ok = [0x04, 0x00, 0x03, 0x40];
        assert_eq!(response(&ok), Ok(&[0x00][..]));
        assert_eq!(response(&[0x04, 0x11, 0x33, 0x44]), Err(Error::Crc));
        assert_eq!(response(&[0x09, 0x00]), Err(Error::Length(9)));
    }
}
