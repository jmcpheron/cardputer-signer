//! The key ceremony, P1 to K4, on a virtual ATECC608A (`examples/common/vchip.rs`), driven through
//! the same pages and keys as the device (`ceremony_pages`): the whole way through, with a restart
//! after every step, Del on every page, letting go of Enter early, a wrong serial, the cable pulled
//! in the middle of steps, and the refusals.
use p256::ecdsa::signature::hazmat::PrehashVerifier;
use p256::ecdsa::{Signature, VerifyingKey};
use signer_core::atecc::Error;
use signer_core::ceremony::{self, Stop};
use signer_core::ceremony_pages::{Ceremony, Key, Page, HOLD_MS};
use signer_core::{hex, safe_signer, webauthn};

#[path = "../examples/common/vchip.rs"]
mod vchip;
use vchip::VChip;

const COMMIT: [u8; 4] = [0xc0, 0xff, 0xee, 0x01];

struct Bench {
    chip: VChip,
    c: Ceremony,
    log: String,
    t: u32,
}

impl Bench {
    fn new(chip: VChip) -> Self {
        let mut b = Bench { chip, c: Ceremony::new(COMMIT), log: String::new(), t: 0 };
        b.settle();
        b
    }

    /// The device restarts: its memory is gone, the chip keeps what it holds.
    fn restart(self) -> Self {
        let mut chip = self.chip;
        chip.power_cycle();
        let mut b = Bench::new(chip);
        b.log = self.log + &b.log;
        b
    }

    fn settle(&mut self) {
        while matches!(self.c.page, Page::Working(_)) {
            self.t += 20;
            self.c.input(None, self.t, &mut self.chip, &mut self.log);
        }
    }

    fn press(&mut self, k: Key) {
        self.t += 40;
        self.c.input(Some(k), self.t, &mut self.chip, &mut self.log);
        self.t += 40;
        self.c.input(None, self.t, &mut self.chip, &mut self.log);
        self.settle();
    }

    fn typed(&mut self, s: &str) {
        for ch in s.chars() {
            self.press(Key::Char(ch));
        }
    }

    /// Enter held for `ms`, then let go.
    fn hold(&mut self, ms: u32) {
        let end = self.t + ms;
        while self.t < end {
            self.c.input(Some(Key::Enter), self.t, &mut self.chip, &mut self.log);
            self.t += 50;
        }
        self.c.input(Some(Key::Enter), self.t, &mut self.chip, &mut self.log);
        self.settle();
        self.t += 40;
        self.c.input(None, self.t, &mut self.chip, &mut self.log);
        self.settle();
    }

    fn page(&self) -> Page {
        self.c.page
    }

    /// The chip's EEPROM: what must not change when nothing is supposed to be written.
    fn eeprom(&self) -> (Vec<u8>, Vec<Vec<u8>>, Vec<u8>, Vec<Option<Vec<u8>>>) {
        let keys = self.chip.keys.iter().map(|k| k.as_ref().map(|k| k.to_bytes().to_vec())).collect();
        (self.chip.config.to_vec(), self.chip.slots.clone(), self.chip.otp.to_vec(), keys)
    }
}

/// From a fresh chip to the gate of `stop` (or the end), the way a person would.
fn walk(b: &mut Bench, to: Page) {
    let steps: &[(Page, &dyn Fn(&mut Bench))] = &[
        (Page::P1, &|b| b.press(Key::Enter)),
        (Page::P2, &|b| b.press(Key::Enter)),
        (Page::P3, &|b| { b.typed("5e6f"); b.hold(HOLD_MS + 100) }),
        (Page::Locked, &|b| b.press(Key::Enter)),
        (Page::P4, &|b| b.press(Key::Enter)),
        (Page::P5, &|b| { b.typed("5E6F"); b.hold(HOLD_MS + 100) }),
        (Page::P6, &|b| b.press(Key::Enter)),
        (Page::K1, &|b| b.press(Key::Enter)),
        (Page::K1Made, &|b| b.press(Key::Enter)),
        (Page::K2, &|b| b.press(Key::Enter)),
        (Page::K3, &|b| { b.typed("5e6f"); b.hold(HOLD_MS + 100) }),
    ];
    for (page, act) in steps {
        if b.page() == to {
            return;
        }
        if b.page() == *page {
            act(b);
        }
    }
    assert_eq!(b.page(), to, "log:\n{}", b.log);
}

fn line<'a>(log: &'a str, start: &str) -> &'a str {
    log.lines().rev().find(|l| l.starts_with(start)).unwrap_or_else(|| panic!("no {start} in\n{log}"))
}

fn bytes(h: &str) -> Vec<u8> {
    let h = h.trim_start_matches("0x");
    let mut b = vec![0u8; h.len() / 2];
    hex::decode(h, &mut b).unwrap();
    b
}

/// Each `kN: sig n hash … r … s …` line in the log, checked: the hash is test transaction n's,
/// worked out here, and the signature is over its WebAuthn digest, by `key`.
fn check_sigs(log: &str, key: &[u8], want: usize) {
    let vk = VerifyingKey::from_sec1_bytes(&[&[4u8][..], key].concat()).unwrap();
    let mut seen = 0;
    for l in log.lines().filter(|l| l.contains(": sig ")) {
        let w: Vec<&str> = l.split(' ').collect();
        let n: u8 = w[2].parse().unwrap();
        let hash = bytes(w[4]);
        assert_eq!(hash, ceremony::test_hash(n));
        let sig = Signature::from_slice(&[bytes(w[6]), bytes(w[8])].concat()).unwrap();
        let digest = webauthn::digest(&hash.try_into().unwrap());
        vk.verify_prehash(&digest, &sig).expect(l);
        seen += 1;
    }
    assert_eq!(seen, want, "{log}");
}

#[test]
fn the_whole_way() {
    let mut b = Bench::new(VChip::factory());
    assert_eq!(b.page(), Page::P1, "{}", b.log);
    walk(&mut b, Page::K4);
    let chip = &b.chip;
    assert!(chip.config_locked() && chip.data_locked());
    assert_eq!(chip.config[88..90], [0xfe, 0xff]);
    assert!(ceremony::holds_the_table(&chip.config));
    assert_eq!(chip.config[..16], VChip::factory().config[..16]);
    for s in 1..16 {
        assert!(chip.slots[s].iter().all(|&x| x == 0), "slot {s}");
    }
    assert_eq!(&chip.otp[..4], b"DK\x01\x01");
    assert_eq!(chip.otp[4..13], vchip::SERIAL);
    assert_eq!(chip.otp[19..23], COMMIT);
    assert!(!chip.watchdog_fired);
    // the locks it sent: the config with its CRC, the data with its CRC, slot 0; once each
    let locks: Vec<u8> = chip.sent.iter().filter(|c| c.0 == 0x17).map(|c| c.1).collect();
    assert_eq!(locks, [0x00, 0x01, 0x82]);
    // GenKey that makes a key: slot 0 once, and slot 1 once, refused
    let made: Vec<u16> = chip.sent.iter().filter(|c| c.0 == 0x40 && c.1 == 0x04).map(|c| c.2).collect();
    assert_eq!(made, [0, 1]);
    assert!(chip.keys[1].is_none());

    let key = bytes(line(&b.log, "k1: pubkey").split(' ').nth(2).unwrap());
    check_sigs(&b.log, &key, 3);
    let addr = safe_signer::address(&key.clone().try_into().unwrap());
    let mut s = [0u8; 42];
    assert_eq!(line(&b.log, "k4: address"), format!("k4: address {}", safe_signer::checksum(&addr, &mut s)));
    assert!(b.log.contains("k2: slot 1 refuses to sign; counter0 1 -> 3"), "{}", b.log);
    assert!(!b.log.contains("stop:"), "{}", b.log);

    // the config CRC on the screen is the one the chip locked with; the data CRC too
    let cfg = ceremony::config_crc(&chip.config);
    assert!(b.log.contains(&format!("p3: rules locked with crc {:04x}", ceremony::crc_value(cfg))));
    let mut rec = [0u8; 64];
    rec.copy_from_slice(&chip.otp);
    assert!(b.log.contains(&format!("p5: data locked with crc {:04x}", ceremony::crc_value(ceremony::data_crc(&rec)))));

    // started again, a sealed chip shows its address, and nothing makes a key or locks
    let sent = b.chip.sent.len();
    let b = b.restart();
    assert_eq!(b.page(), Page::K4);
    assert!(b.chip.sent[sent..].iter().all(|c| c.0 != 0x17 && c.0 != 0x12 && !(c.0 == 0x40 && c.1 != 0)));
    assert_eq!(line(&b.log, "k4: address"), format!("k4: address {}", safe_signer::checksum(&addr, &mut s)));
}

#[test]
fn a_restart_after_every_step_picks_up_where_the_chip_is() {
    let pages = [Page::P1, Page::P2, Page::P3, Page::Locked, Page::P4, Page::P5, Page::P6, Page::K1, Page::K1Made,
                 Page::K2, Page::K3, Page::K4];
    // where a restart lands, from each page: the rules and data, if written, stay; a key not sealed
    // means P6 again, and a new key
    let lands = [Page::P1, Page::P1, Page::P1, Page::Locked, Page::Locked, Page::Locked, Page::P6, Page::P6, Page::P6,
                 Page::P6, Page::P6, Page::K4];
    for (page, land) in pages.iter().zip(lands) {
        let mut b = Bench::new(VChip::factory());
        walk(&mut b, *page);
        let mut b = b.restart();
        assert_eq!(b.page(), land, "a restart on {page:?}");
        walk(&mut b, Page::K4);
        assert_eq!(b.chip.config[88..90], [0xfe, 0xff]);
        assert!(!b.chip.watchdog_fired);
    }
}

#[test]
fn del_on_every_page_writes_nothing() {
    for page in [Page::P1, Page::P2, Page::P3, Page::Locked, Page::P4, Page::P5, Page::P6, Page::K1, Page::K1Made,
                 Page::K2, Page::K3] {
        let mut b = Bench::new(VChip::factory());
        walk(&mut b, page);
        if matches!(page, Page::P3 | Page::P5 | Page::K3) {
            b.typed("5e6f");
            b.hold(HOLD_MS / 2); // let go early, then Del
            assert_eq!(b.page(), page);
        }
        let before = b.eeprom();
        let counter = b.chip.counter0;
        b.press(Key::Del);
        assert!(!matches!(b.page(), Page::Stopped(_)), "{page:?}: {}", b.log);
        let after = b.eeprom();
        assert!(before == after, "Del on {page:?} changed the chip");
        // reading a key that exists counts as a use; nothing else may
        assert!(b.chip.counter0 <= counter + 1, "{page:?}");
    }
}

#[test]
fn the_gate() {
    let mut b = Bench::new(VChip::factory());
    walk(&mut b, Page::P3);
    // Enter without the four does nothing, a wrong four does nothing, letting go early does nothing
    b.hold(HOLD_MS + 500);
    assert_eq!(b.page(), Page::P3);
    b.typed("826b");
    b.hold(HOLD_MS + 500);
    assert_eq!(b.page(), Page::P3);
    b.typed("5e6f");
    b.hold(HOLD_MS - 200);
    assert_eq!(b.page(), Page::P3);
    assert!(!b.chip.config_locked());
    // keys that aren't hex are ignored; then held long enough, it locks
    b.typed("5e6fz");
    b.hold(HOLD_MS + 100);
    assert_eq!(b.page(), Page::Locked, "{}", b.log);
    assert!(b.chip.config_locked());
}

#[test]
fn the_cable_pulled_in_the_middle_of_a_step() {
    // P4 is 50 writes; pull the cable after some of them, then start again
    for cut in [1, 9, 30, 49] {
        let mut b = Bench::new(VChip::factory());
        walk(&mut b, Page::Locked);
        b.chip.cut_after = Some(cut);
        b.press(Key::Enter);
        assert!(matches!(b.page(), Page::Stopped(Stop::Chip(_, Error::NoChip))), "{:?}", b.page());
        let mut b = b.restart();
        assert_eq!(b.page(), Page::Locked);
        walk(&mut b, Page::K4);
    }
    // and in K2, after the first signature: a new key next time
    let mut b = Bench::new(VChip::factory());
    walk(&mut b, Page::K1Made);
    let first = bytes(line(&b.log, "k1: pubkey").split(' ').nth(2).unwrap());
    b.chip.cut_after = Some(4);
    b.press(Key::Enter);
    assert!(matches!(b.page(), Page::Stopped(_)));
    let mut b = b.restart();
    assert_eq!(b.page(), Page::P6);
    walk(&mut b, Page::K4);
    let second = bytes(line(&b.log, "k1: pubkey").split(' ').nth(2).unwrap());
    assert_ne!(first, second);
    assert!(b.log.contains("key in slot 0 true"));
}

// ----------------------------------------------------------------------------- the steps' refusals
fn link_log() -> String {
    String::new()
}

#[test]
fn p1_refuses_chips_it_doesnt_prepare() {
    // my development chip: its config locked with Microchip's test table
    let mut dev = VChip::factory();
    dev.config[87] = 0;
    let b = Bench::new(dev);
    assert!(matches!(b.page(), Page::Stopped(Stop::Refused(s)) if s.contains("aren't these")), "{:?}", b.page());
    assert!(b.chip.sent.iter().all(|c| c.0 == 0x02 || c.0 == 0x30));

    let mut b608 = VChip::factory();
    b608.revision = [0x00, 0x00, 0x60, 0x03];
    assert!(matches!(Bench::new(b608).page(), Page::Stopped(Stop::Refused(s)) if s.contains("608B")));

    let mut set = VChip::factory();
    set.config[84] = 0x01;
    assert!(matches!(Bench::new(set).page(), Page::Stopped(Stop::Refused(_))));

    let mut sealed = VChip::factory();
    sealed.config[88] = 0xfe;
    assert!(matches!(Bench::new(sealed).page(), Page::Stopped(Stop::Refused(_))));
}

#[test]
fn a_lock_with_a_wrong_crc_locks_nothing() {
    let mut chip = VChip::factory();
    let mut log = link_log();
    let p1 = ceremony::p1(&mut chip, &mut log).unwrap();
    let crc = ceremony::p2(&mut chip, &p1.config, &mut log).unwrap();
    let wrong = [crc[0] ^ 1, crc[1]];
    assert!(matches!(ceremony::p3(&mut chip, wrong, &mut log), Err(Stop::Chip(_, Error::Status(0x0f)))));
    assert!(!chip.config_locked());
    // the chip holding a table one byte off, as if a write went wrong: the right CRC locks nothing
    chip.config[40] ^= 0x80;
    assert!(matches!(ceremony::p3(&mut chip, crc, &mut log), Err(Stop::Chip(_, Error::Status(0x0f)))));
    assert!(!chip.config_locked());
}

#[test]
fn a_data_lock_without_the_zeros_locks_nothing() {
    let mut chip = VChip::factory();
    let mut log = link_log();
    let p1 = ceremony::p1(&mut chip, &mut log).unwrap();
    let crc = ceremony::p2(&mut chip, &p1.config, &mut log).unwrap();
    ceremony::p3(&mut chip, crc, &mut log).unwrap();
    let record = ceremony::record(&vchip::SERIAL, &ceremony::REVISION_608A, crc, &COMMIT);
    // the CRC of zeros and the record, but slot 9 still holds the factory's bytes
    assert!(ceremony::p5(&mut chip, ceremony::data_crc(&record), &record, &mut log).is_err());
    assert!(!chip.data_locked());
    // 4-byte writes into a secret slot are refused before the data lock, as on silicon
    use signer_core::atecc::{run, Cmd};
    let w = Cmd { opcode: ceremony::OP_WRITE, p1: 0x02, p2: 1 << 3, resp_len: 1, wait_ms: 45 };
    chip.wake().unwrap();
    assert_eq!(run(&mut chip, &w, &[0; 4], &mut [0u8; 1]), Err(Error::Status(0x0f)));
    chip.sleep();
    ceremony::p4(&mut chip, &record, &mut log).unwrap();
    ceremony::p5(&mut chip, ceremony::data_crc(&record), &record, &mut log).unwrap();
    assert!(chip.data_locked());
}

#[test]
fn after_the_seal() {
    let mut b = Bench::new(VChip::factory());
    walk(&mut b, Page::K4);
    let mut chip = b.chip;
    let mut log = link_log();
    let before = chip.keys[0].clone().unwrap().to_bytes();
    // K1 on a sealed slot: refused, and the key stays
    assert!(ceremony::k1(&mut chip, &mut log).is_err());
    assert_eq!(chip.keys[0].clone().unwrap().to_bytes(), before);
    // the zone locks again: refused
    use signer_core::atecc::run;
    chip.wake().unwrap();
    assert!(run(&mut chip, &ceremony::lock_config([0, 0]), &[], &mut [0u8; 1]).is_err());
    assert!(run(&mut chip, &ceremony::write_slot_block(8, 0), &[1; 32], &mut [0u8; 1]).is_err());
    chip.sleep();
}

use signer_core::atecc::Link;
