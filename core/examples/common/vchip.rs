//! A virtual ATECC608A for the tests and the rehearsal: a factory chip, as an Adafruit 4314 comes,
//! that answers the commands the ceremony sends the way the datasheet and the bench say. It refuses
//! what the silicon refused (docs/CEREMONY.md, "What has run on silicon"): 4-byte writes into
//! secret slots before the data lock, a lock with the wrong CRC, GenKey and Sign on slot 1, any
//! write after the locks, a read of a secret slot. Its watchdog sends it to sleep 1.3 s after a
//! wake, and sleep forgets TempKey. Its keys are P-256 (the p256 crate), made from a counter, so a
//! run is the same every time.
#![allow(dead_code)]
use p256::ecdsa::signature::hazmat::PrehashSigner;
use p256::ecdsa::{Signature, SigningKey};
use signer_core::atecc::{self, Error, Link};
use signer_core::sha256::sha256;

pub const SERIAL: [u8; 9] = [0x01, 0x23, 0x4a, 0x7b, 0x9c, 0x2d, 0x5e, 0x6f, 0xee];
const WATCHDOG_MS: u32 = 1300;

pub struct VChip {
    pub config: [u8; 128],
    pub slots: Vec<Vec<u8>>,
    pub otp: [u8; 64],
    pub keys: Vec<Option<SigningKey>>,
    pub counter0: u32,
    pub revision: [u8; 4],
    awake: bool,
    ms: u32,
    tempkey: Option<[u8; 32]>,
    made: u32,
    /// Every command it was sent: (opcode, p1, p2).
    pub sent: Vec<(u8, u8, u16)>,
    /// The cable comes out after this many more commands.
    pub cut_after: Option<usize>,
    /// What the watchdog said, if it fired.
    pub watchdog_fired: bool,
}

/// What each command takes at most, from Microchip's table for the 608 (cryptoauthlib).
fn exec_ms(op: u8) -> u32 {
    match op {
        0x02 | 0x30 => 5,
        0x12 => 45,
        0x17 => 35,
        0x40 | 0x41 => 115,
        0x16 => 20,
        0x1B => 23,
        0x24 => 25,
        0x77 => 250,
        _ => 50,
    }
}

impl VChip {
    /// A factory chip: its own config (not the ceremony's), both zones open, and something other
    /// than zeros in every slot and in OTP.
    pub fn factory() -> Self {
        let mut c = [0u8; 128];
        c[..4].copy_from_slice(&SERIAL[..4]);
        c[4..8].copy_from_slice(&[0x00, 0x00, 0x60, 0x02]);
        c[8..13].copy_from_slice(&SERIAL[4..]);
        c[14] = 0x01;
        c[16] = 0xc0;
        for s in 0..16 {
            c[20 + 2 * s..22 + 2 * s].copy_from_slice(&[0x83, 0x20]);
            c[96 + 2 * s..98 + 2 * s].copy_from_slice(&[0x33, 0x00]);
        }
        c[52..60].copy_from_slice(&[0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0]);
        c[60..68].copy_from_slice(&[0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0]);
        c[86] = 0x55;
        c[87] = 0x55;
        c[88] = 0xff;
        c[89] = 0xff;
        let slots = (0..16).map(|s| vec![0xa5u8; signer_core::ceremony::slot_len(s)]).collect();
        VChip {
            config: c,
            slots,
            otp: [0xa5; 64],
            keys: (0..16).map(|_| None).collect(),
            counter0: 0,
            revision: [0x00, 0x00, 0x60, 0x02],
            awake: false,
            ms: 0,
            tempkey: None,
            made: 0,
            sent: Vec::new(),
            cut_after: None,
            watchdog_fired: false,
        }
    }

    /// Unplugged and plugged in again: awake no more, TempKey gone, EEPROM kept.
    pub fn power_cycle(&mut self) {
        self.awake = false;
        self.tempkey = None;
        self.cut_after = None;
    }

    pub fn config_locked(&self) -> bool {
        self.config[87] == 0
    }
    pub fn data_locked(&self) -> bool {
        self.config[86] == 0
    }
    fn slot_config(&self, s: usize) -> u16 {
        u16::from_le_bytes([self.config[20 + 2 * s], self.config[21 + 2 * s]])
    }
    fn key_config(&self, s: usize) -> u16 {
        u16::from_le_bytes([self.config[96 + 2 * s], self.config[97 + 2 * s]])
    }
    fn secret(&self, s: usize) -> bool {
        self.slot_config(s) & 0x80 != 0
    }
    fn private(&self, s: usize) -> bool {
        self.key_config(s) & 1 == 1 && (self.key_config(s) >> 2) & 7 == 4
    }
    fn sealed(&self, s: usize) -> bool {
        let bits = u16::from_le_bytes([self.config[88], self.config[89]]);
        bits >> s & 1 == 0
    }

    /// The data zone's CRC as the chip works it out for Lock: every slot that isn't a private key,
    /// in order, then OTP.
    fn data_crc(&self) -> u16 {
        let mut crc = 0;
        for s in 0..16 {
            if !self.private(s) {
                crc = atecc::crc16_update(crc, &self.slots[s]);
            }
        }
        atecc::crc16_update(crc, &self.otp)
    }

    fn new_key(&mut self) -> SigningKey {
        self.made += 1;
        let seed = sha256(&[b"vchip key", &self.made.to_le_bytes()]);
        SigningKey::from_slice(&seed).unwrap()
    }

    fn uses_key(&mut self, s: usize) {
        // LimitedUse (SlotConfig bit 5): every use of the key counts, once the data zone is locked
        if self.data_locked() && self.slot_config(s) & 0x20 != 0 {
            self.counter0 += 1;
        }
    }

    /// One command. Ok(payload) or Err(status byte).
    fn execute(&mut self, op: u8, p1: u8, p2: u16, data: &[u8]) -> Result<Vec<u8>, u8> {
        const EXEC: u8 = 0x0F;
        const PARSE: u8 = 0x03;
        let ok = || Ok(vec![0u8]);
        match op {
            0x02 => {
                let n = if p1 & 0x80 != 0 { 32 } else { 4 };
                match p1 & 3 {
                    0 => {
                        let at = (p2 as usize) * 4;
                        Ok(self.config.get(at..at + n).ok_or(PARSE)?.to_vec())
                    }
                    1 => {
                        if !self.data_locked() {
                            return Err(EXEC);
                        }
                        let at = (p2 as usize) * 4;
                        Ok(self.otp.get(at..at + n).ok_or(PARSE)?.to_vec())
                    }
                    2 => {
                        let (s, b) = ((p2 >> 3 & 15) as usize, (p2 >> 8) as usize);
                        if !self.data_locked() || self.secret(s) {
                            return Err(EXEC);
                        }
                        let mut out = self.slots[s].get(b * 32..).ok_or(PARSE)?.to_vec();
                        out.resize(n, 0);
                        Ok(out)
                    }
                    _ => Err(PARSE),
                }
            }
            0x12 => {
                let n = if p1 & 0x80 != 0 { 32 } else { 4 };
                if data.len() != n {
                    return Err(PARSE);
                }
                match p1 & 3 {
                    0 => {
                        let at = if n == 32 { (p2 as usize >> 3) * 32 } else { p2 as usize * 4 };
                        if self.config_locked() || at < 16 || (at < 88 && at + n > 84) || at + n > 128 {
                            return Err(EXEC);
                        }
                        self.config[at..at + n].copy_from_slice(data);
                        ok()
                    }
                    1 => {
                        if !self.config_locked() || self.data_locked() || n != 32 {
                            return Err(EXEC);
                        }
                        let at = (p2 as usize >> 3) * 32;
                        self.otp.get_mut(at..at + 32).ok_or(PARSE)?.copy_from_slice(data);
                        ok()
                    }
                    2 => {
                        let (s, b) = ((p2 >> 3 & 15) as usize, (p2 >> 8) as usize);
                        if !self.config_locked() || self.data_locked() {
                            // after the data lock every slot's WriteConfig here says never
                            return Err(EXEC);
                        }
                        if n == 4 && self.secret(s) {
                            return Err(EXEC); // seen on silicon, 2026-09-26
                        }
                        let at = b * 32 + if n == 4 { (p2 as usize & 7) * 4 } else { 0 };
                        let len = self.slots[s].len();
                        if at >= len {
                            return Err(PARSE);
                        }
                        let k = n.min(len - at);
                        self.slots[s][at..at + k].copy_from_slice(&data[..k]);
                        ok()
                    }
                    _ => Err(PARSE),
                }
            }
            0x17 => match p1 {
                0x00 => {
                    if self.config_locked() || atecc::crc16(&self.config) != p2.to_le_bytes() {
                        return Err(EXEC);
                    }
                    self.config[87] = 0;
                    ok()
                }
                0x01 => {
                    if !self.config_locked() || self.data_locked() || self.data_crc() != p2 {
                        return Err(EXEC);
                    }
                    self.config[86] = 0;
                    ok()
                }
                0x82 => {
                    if !self.data_locked() || self.key_config(0) & 0x20 == 0 || self.sealed(0) {
                        return Err(EXEC);
                    }
                    self.config[88] &= !1;
                    ok()
                }
                _ => Err(PARSE), // no lock without its CRC, but a slot's
            },
            0x40 => {
                let s = p2 as usize;
                match p1 {
                    0x04 => {
                        // GenKey on a slot that's a P-256 private key; after the data lock, only if
                        // its WriteConfig allows GenKey (bit 13) and it isn't sealed
                        if !self.config_locked() || s > 15 || !self.private(s) {
                            return Err(EXEC);
                        }
                        if self.data_locked() && (self.slot_config(s) & 0x2000 == 0 || self.sealed(s)) {
                            return Err(EXEC);
                        }
                        let k = self.new_key();
                        let pubkey = k.verifying_key().to_encoded_point(false).as_bytes()[1..].to_vec();
                        self.keys[s] = Some(k);
                        Ok(pubkey)
                    }
                    0x00 => {
                        let Some(k) = self.keys.get(s).cloned().flatten() else { return Err(EXEC) };
                        if self.data_locked() && self.key_config(s) & 2 == 0 {
                            return Err(EXEC);
                        }
                        self.uses_key(s); // seen on silicon: a public-key read counts, after the data lock
                        Ok(k.verifying_key().to_encoded_point(false).as_bytes()[1..].to_vec())
                    }
                    _ => Err(PARSE),
                }
            }
            0x16 => {
                if p1 != 0x03 || data.len() != 32 {
                    return Err(PARSE);
                }
                self.tempkey = Some(data.try_into().unwrap());
                ok()
            }
            0x41 => {
                let s = p2 as usize;
                if p1 != 0x80 {
                    return Err(PARSE);
                }
                let Some(msg) = self.tempkey.take() else { return Err(EXEC) };
                if !self.data_locked() || s > 15 || !self.private(s) || self.slot_config(s) & 1 == 0 {
                    return Err(EXEC);
                }
                let Some(k) = self.keys[s].clone() else { return Err(EXEC) };
                self.uses_key(s);
                let sig: Signature = k.sign_prehash(&msg).unwrap();
                Ok(sig.to_bytes().to_vec())
            }
            0x30 => match p1 {
                0x00 => Ok(self.revision.to_vec()),
                0x01 => {
                    let s = p2 as usize;
                    if s > 15 || (self.key_config(s) >> 2) & 7 != 4 {
                        return Err(EXEC);
                    }
                    Ok(vec![self.keys[s].is_some() as u8, 0, 0, 0])
                }
                _ => Err(PARSE),
            },
            0x24 => {
                if p1 != 0 || p2 > 1 {
                    return Err(PARSE);
                }
                Ok(if p2 == 0 { self.counter0 } else { 0 }.to_le_bytes().to_vec())
            }
            0x1B => {
                if !self.config_locked() {
                    return Ok([0xff, 0xff, 0, 0].repeat(8));
                }
                Ok(sha256(&[b"vchip random", &(self.sent.len() as u32).to_le_bytes()]).to_vec())
            }
            0x77 => Ok(vec![0]),
            _ => Err(PARSE),
        }
    }
}

fn answer(payload: &[u8], resp: &mut [u8]) {
    let n = payload.len() + 3;
    let mut a = vec![n as u8];
    a.extend_from_slice(payload);
    a.extend_from_slice(&atecc::crc16(&a));
    let k = a.len().min(resp.len());
    resp[..k].copy_from_slice(&a[..k]);
}

impl Link for VChip {
    fn wake(&mut self) -> Result<(), Error> {
        if self.cut_after == Some(0) {
            return Err(Error::NoChip);
        }
        self.awake = true;
        self.ms = 0;
        Ok(())
    }

    fn sleep(&mut self) {
        self.awake = false;
        self.tempkey = None;
    }

    fn exchange(&mut self, pkt: &[u8], wait_ms: u32, resp: &mut [u8]) -> Result<(), Error> {
        if let Some(n) = self.cut_after.as_mut() {
            if *n == 0 {
                return Err(Error::NoChip);
            }
            *n -= 1;
        }
        if !self.awake {
            return Err(Error::NoChip);
        }
        assert_eq!(pkt[0], 0x03, "word address");
        let n = pkt[1] as usize;
        assert_eq!(n + 1, pkt.len(), "count");
        if atecc::crc16(&pkt[1..n - 1]) != pkt[n - 1..n + 1] {
            answer(&[0xff], resp);
            return Ok(());
        }
        let (op, p1, p2) = (pkt[2], pkt[3], u16::from_le_bytes([pkt[4], pkt[5]]));
        self.sent.push((op, p1, p2));
        // the driver polls until the chip answers, so the chip is busy for its own time
        self.ms += wait_ms.max(exec_ms(op)) + 2;
        if self.ms > WATCHDOG_MS {
            self.watchdog_fired = true;
            self.sleep();
            return Err(Error::NoChip);
        }
        match self.execute(op, p1, p2, &pkt[6..n - 1]) {
            Ok(p) => answer(&p, resp),
            Err(status) => answer(&[status], resp),
        }
        Ok(())
    }
}
