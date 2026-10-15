//! The key ceremony on the screen: its pages, and what each key does on each (docs/CEREMONY.md).
//! The ceremony image (firmware/src/bin/ceremony.rs) feeds it the keys held down and the time, and
//! draws what it says. `cargo test` drives it the same way, on a virtual chip.
//!
//! Enter does the step or goes on. Del backs out with one press and writes nothing: the ceremony
//! starts again from where the chip is. A permanent step is a red page: type the last group of four
//! of the chip's serial, from its sticker, then hold Enter for 3 seconds. Letting go early does
//! nothing.
use core::fmt::Write;

use crate::atecc::{self, Link};
use crate::buf::Buf;
use crate::ceremony::{self, Chip, Stage, Stop, K2, P1, P6};
use crate::font::{self, Face};
use crate::gfx::{theme::*, Frame, Rgb};
use crate::hex;
use crate::safe_tx::Address;
use crate::screens::{fit, para, short, W};
use crate::sha256::sha256;

const M: i32 = 6;
const R: i32 = W - M;
const TW: i32 = W - 2 * M;
const FOOT: i32 = 117;
/// How long Enter is held for a permanent step.
pub const HOLD_MS: u32 = 3000;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Key {
    Enter,
    Del,
    /// A key that types, lowercase.
    Char(char),
}

/// The steps that talk to the chip. Each runs after its Working page is on the screen.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Step {
    Start,
    P2,
    P3,
    P4,
    P5,
    K1,
    K2,
    K3,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Page {
    /// A step is running.
    Working(Step),
    /// Stopped: not a chip this ceremony prepares, or a step or check that failed.
    Stopped(Stop),
    P1,
    P2,
    /// Gate: lock the rules.
    P3,
    /// The rules are locked; the data zone is open.
    Locked,
    P4,
    /// Gate: lock the data.
    P5,
    P6,
    K1,
    K1Made,
    K2,
    /// Gate: seal the key.
    K3,
    K4,
}

pub struct Ceremony {
    pub page: Page,
    /// The first 4 bytes of the commit the image was built from, for the record.
    commit: [u8; 4],
    chip: Option<Chip>,
    p1: Option<P1>,
    config_crc: [u8; 2],
    record: [u8; 64],
    data_crc: [u8; 2],
    p6: Option<P6>,
    key: Option<[u8; 64]>,
    k2: Option<K2>,
    address: Address,
    /// The serial's last four, as typed at a gate.
    typed: Buf<4>,
    held: Option<Key>,
    hold_since: Option<u32>,
    hold_drawn: u32,
}

impl Ceremony {
    pub fn new(commit: [u8; 4]) -> Self {
        Ceremony {
            page: Page::Working(Step::Start),
            commit,
            chip: None,
            p1: None,
            config_crc: [0; 2],
            record: [0; 64],
            data_crc: [0; 2],
            p6: None,
            key: None,
            k2: None,
            address: [0; 20],
            typed: Buf::new(),
            held: None,
            hold_since: None,
            hold_drawn: 0,
        }
    }

    /// The key held down now (None for none), at `now` ms. Runs a step whose Working page is on the
    /// screen. Returns whether the screen needs drawing again.
    pub fn input(&mut self, down: Option<Key>, now: u32, l: &mut dyn Link, log: &mut dyn Write) -> bool {
        if let Page::Working(step) = self.page {
            self.page = match self.run(step, l, log) {
                Ok(p) => p,
                Err(stop) => {
                    let _ = writeln!(log, "stop: {}", Reason(stop));
                    Page::Stopped(stop)
                }
            };
            self.held = down; // a key still down from before the step doesn't count as a new press
            return true;
        }
        let mut dirty = false;
        if down != self.held {
            self.held = down;
            if self.hold_since.take().is_some() {
                dirty = true; // let go early: nothing happens
            }
            if let Some(k) = down {
                dirty |= self.press(k, now);
            }
        }
        if let Some(t) = self.hold_since {
            let held = now.wrapping_sub(t);
            if held >= HOLD_MS {
                self.hold_since = None;
                self.page = Page::Working(match self.page {
                    Page::P3 => Step::P3,
                    Page::P5 => Step::P5,
                    _ => Step::K3,
                });
                return true;
            }
            if held / 100 != self.hold_drawn / 100 {
                self.hold_drawn = held;
                dirty = true;
            }
        }
        dirty
    }

    fn gate(&self) -> bool {
        matches!(self.page, Page::P3 | Page::P5 | Page::K3)
    }

    fn matches(&self) -> bool {
        match &self.chip {
            Some(c) => self.typed.as_str().as_bytes() == ceremony::last_four(&c.serial()),
            None => false,
        }
    }

    fn press(&mut self, k: Key, now: u32) -> bool {
        use Page::*;
        if k == Key::Del {
            if matches!(self.page, K4) {
                return false;
            }
            self.page = Working(Step::Start);
            return true;
        }
        if self.gate() {
            match k {
                Key::Char(c) if c.is_ascii_hexdigit() => {
                    if self.typed.as_str().len() == 4 {
                        self.typed = Buf::new();
                    }
                    let _ = self.typed.write_char(c.to_ascii_lowercase());
                }
                Key::Enter if self.matches() => {
                    self.hold_since = Some(now);
                    self.hold_drawn = 0;
                }
                _ => return false,
            }
            return true;
        }
        if k != Key::Enter {
            return false;
        }
        self.typed = Buf::new();
        self.page = match self.page {
            Stopped(_) => Working(Step::Start),
            P1 => Working(Step::P2),
            P2 => P3,
            Locked => Working(Step::P4),
            P4 => P5,
            P6 => K1,
            K1 => Working(Step::K1),
            K1Made => Working(Step::K2),
            K2 => K3,
            _ => return false,
        };
        true
    }

    fn run(&mut self, step: Step, l: &mut dyn Link, log: &mut dyn Write) -> Result<Page, Stop> {
        Ok(match step {
            Step::Start => {
                *self = Ceremony::new(self.commit);
                let chip = ceremony::read_chip(l, log)?;
                self.chip = Some(chip);
                match chip.stage()? {
                    Stage::Fresh => {
                        self.p1 = Some(ceremony::p1(l, log)?);
                        Page::P1
                    }
                    Stage::RulesLocked => Page::Locked,
                    Stage::Prepared => {
                        self.p6 = Some(ceremony::p6(l, log)?);
                        Page::P6
                    }
                    Stage::Sealed => {
                        let key = ceremony::sealed_key(l, log)?;
                        self.key = Some(key);
                        self.address = ceremony::k4(&key, log);
                        Page::K4
                    }
                }
            }
            Step::P2 => {
                let factory = self.p1.as_ref().map(|p| p.config).ok_or(Stop::Check("P1 hasn't run"))?;
                self.config_crc = ceremony::p2(l, &factory, log)?;
                Page::P2
            }
            Step::P3 => {
                ceremony::p3(l, self.config_crc, log)?;
                Page::Locked
            }
            Step::P4 => {
                let c = self.chip.ok_or(Stop::Check("the chip wasn't read"))?;
                self.config_crc = ceremony::config_crc(&c.config);
                self.record = ceremony::record(&c.serial(), &c.revision, self.config_crc, &self.commit);
                self.data_crc = ceremony::p4(l, &self.record, log)?;
                Page::P4
            }
            Step::P5 => {
                ceremony::p5(l, self.data_crc, &self.record, log)?;
                self.p6 = Some(ceremony::p6(l, log)?);
                Page::P6
            }
            Step::K1 => {
                self.key = Some(ceremony::k1(l, log)?);
                self.k2 = None;
                Page::K1Made
            }
            Step::K2 => {
                self.k2 = Some(ceremony::k2(l, log)?);
                Page::K2
            }
            Step::K3 => {
                let key = self.key.ok_or(Stop::Check("no key was made"))?;
                ceremony::k3(l, &key, log)?;
                self.address = ceremony::k4(&key, log);
                Page::K4
            }
        })
    }

    // ------------------------------------------------------------------------- drawing
    pub fn draw(&self, f: &mut Frame, now: u32) {
        f.fill(BG);
        let serial = self.chip.map(|c| c.serial()).unwrap_or([0; 9]);
        match self.page {
            Page::Working(step) => working(f, step),
            Page::Stopped(stop) => stopped(f, stop, &serial),
            Page::P1 => {
                header(f, "P1", "Read the chip", "Part 1 of 2");
                let mut h = [0u8; 18];
                f.blockie(hex::encode(&serial, &mut h).as_bytes(), M, 25, 3);
                f.text("ATECC608A", M + 32, 21, Face::Bold14, TEXT, BG);
                f.text(grouped(&serial).as_str(), M + 32, 38, Face::Mono13, MUTED, BG);
                check(f, 60, true, "Both zones open: nothing locked yet");
                check(f, 76, true, "Self-test passed");
                check(f, 92, true, "The factory config, sent to the log");
                keys(f, Some(("Del", "Back")), Some(("Enter", "Write the rules", RAISED, TEXT)));
            }
            Page::P2 => {
                header(f, "P2", "Write the rules", "Part 1 of 2");
                check(f, 23, true, "Written and read back, not locked yet");
                f.text("Config CRC", M, 43, Face::Sans12, MUTED, BG);
                f.text(crc(self.config_crc).as_str(), M, 56, Face::MonoB28, TEXT, BG);
                para(f, "The page works it out too. They should match.", M + 100, 60, R - M - 100, Face::Sans12, MUTED, FOOT - 3);
                keys(f, Some(("Del", "Back")), Some(("Enter", "Next", RAISED, TEXT)));
            }
            Page::P3 => {
                header(f, "P3", "Lock the rules", "Permanent");
                f.text("Locks the rules for good.", M, 22, Face::Bold14, TEXT, BG);
                let mut s: Buf<120> = Buf::new();
                let _ = write!(s, "The chip checks CRC {} first. If one byte differs, it locks nothing.", crc(self.config_crc));
                para(f, s.as_str(), M, 41, TW, Face::Sans12, MUTED, 70);
                self.gate_footer(f, now);
            }
            Page::Locked => {
                header(f, "P3", "The rules are locked", "Part 1 of 2");
                check(f, 23, true, "Locked: the diykey v1 table");
                check(f, 39, true, "The data zone: still open");
                para(f, "Next: zeros into slots 1-15, and the record into OTP. Both can be written again until P5.",
                     M, 60, TW, Face::Sans12, MUTED, FOOT - 3);
                keys(f, Some(("Del", "Back")), Some(("Enter", "Write them", RAISED, TEXT)));
            }
            Page::P4 => {
                header(f, "P4", "Zeros and the record", "Part 1 of 2");
                check(f, 23, true, "Slots 1-15: zeros");
                check(f, 39, true, "The record, into OTP");
                f.text("Data CRC", M, 59, Face::Sans12, MUTED, BG);
                f.text(crc(self.data_crc).as_str(), M, 72, Face::MonoB28, TEXT, BG);
                para(f, "Rewritable until P5 locks it.", M + 100, 76, R - M - 100, Face::Sans12, MUTED, FOOT - 3);
                keys(f, Some(("Del", "Back")), Some(("Enter", "Next", RAISED, TEXT)));
            }
            Page::P5 => {
                header(f, "P5", "Lock the data", "Permanent");
                let mut s: Buf<160> = Buf::new();
                let _ = write!(s, "Freezes every slot and the record, checked against CRC {}. After this, only the chip's own GenKey can put a key in slot 0.", crc(self.data_crc));
                para(f, s.as_str(), M, 22, TW, Face::Sans12, TEXT, 70);
                self.gate_footer(f, now);
            }
            Page::P6 => {
                header(f, "P6", "Check", "Part 1 of 2");
                f.mark(true, M + 11, 33, 11);
                f.text("Ready for a key", M + 30, 21, Face::Bold18, TEXT, BG);
                check(f, 52, true, "Rules and data locked");
                let p6 = self.p6.as_ref();
                match p6.and_then(|p| p.key_in_slot0) {
                    Some(false) => check(f, 68, true, "Slot 0: empty, not sealed"),
                    Some(true) => warn(f, 68, "Slot 0 has a key: K1 replaces it"),
                    None => warn(f, 68, "Slot 0: not sealed; Info didn't say if empty"),
                }
                let mut s: Buf<40> = Buf::new();
                let n = p6.map_or(0, |p| p.counter0);
                let _ = write!(s, "Counter 0: {n}{}", if n == 0 { ", never used" } else { "" });
                check(f, 84, true, s.as_str());
                keys(f, Some(("Del", "Later")), Some(("Enter", "Make your key", OK, ON_OK)));
            }
            Page::K1 => {
                header(f, "K1", "Make your key", "Part 2 of 2");
                f.text("Made inside the chip", M, 21, Face::Bold18, TEXT, BG);
                para(f, "Nobody chooses it, and it can't be read out or copied. You can make it again until you seal it.",
                     M, 50, TW, Face::Sans12, MUTED, FOOT - 3);
                keys(f, Some(("Del", "Back")), Some(("Enter", "Make it", OK, ON_OK)));
            }
            Page::K1Made => {
                header(f, "K1", "Your key", "Part 2 of 2");
                let key = self.key.unwrap_or([0; 64]);
                let mut seed = [0u8; 66];
                seed[..2].copy_from_slice(b"0x");
                hex::encode(&key[..32], &mut seed[2..]);
                f.blockie(&seed, M, 24, 4);
                f.text("P-256, in slot 0", M + 40, 22, Face::Bold14, TEXT, BG);
                f.text(fingerprint(&key).as_str(), M + 40, 39, Face::Mono13, MUTED, BG);
                check(f, 64, true, "Made by the chip's GenKey, just now");
                check(f, 80, true, "Slot 1 refuses GenKey");
                f.text("Not sealed yet. Next: test signatures.", M, 96, Face::Sans12, FAINT, BG);
                keys(f, Some(("Del", "Back")), Some(("Enter", "Test it", RAISED, TEXT)));
            }
            Page::K2 => {
                header(f, "K2", "Test it", "Part 2 of 2");
                check(f, 23, true, "Test signature 1: sent to the page");
                check(f, 39, true, "Test signature 2: sent to the page");
                check(f, 55, true, "Slot 1 refuses to sign");
                let (a, b) = self.k2.as_ref().map_or((0, 0), |k| (k.counter_before, k.counter_after));
                let mut s: Buf<48> = Buf::new();
                let _ = write!(s, "Counter 0: {a} to {b}");
                check(f, 71, b == a.wrapping_add(2), s.as_str());
                f.text("Seal it once the page says both are valid.", M, 91, Face::Sans12, MUTED, BG);
                keys(f, Some(("Del", "Back")), Some(("Enter", "Seal it", RAISED, TEXT)));
            }
            Page::K3 => {
                header(f, "K3", "Seal your key", "Permanent");
                para(f, "Your key can never be changed or replaced. Lose the chip, and your Safe's other owners swap it out.",
                     M, 22, TW, Face::Sans12, TEXT, 70);
                self.gate_footer(f, now);
            }
            Page::K4 => {
                header(f, "K4", "Its address", "Part 2 of 2");
                f.text("Safe signer address, sealed key", M, 21, Face::Sans12, MUTED, BG);
                let mut seed = [0u8; 42];
                seed[..2].copy_from_slice(b"0x");
                hex::encode(&self.address, &mut seed[2..]);
                f.blockie(&seed, M, 39, 4);
                f.text(short(&self.address).as_str(), M + 40, 46, Face::Bold18, TEXT, BG);
                para(f, "A signer, not a wallet: don't send funds here. Funds go to your Safe.", M, 78, TW, Face::Sans12, MUTED, FOOT - 3);
                f.hline(0, FOOT - 3, W, LINE);
                f.text("Done. Next: flash the signer back.", M, FOOT, Face::Sans12, FAINT, BG);
            }
        }
    }

    /// The serial gate and, once the four match, Enter to hold.
    fn gate_footer(&self, f: &mut Frame, now: u32) {
        let typed = self.typed.as_str();
        let wrong = typed.len() == 4 && !self.matches();
        f.text(if wrong { "Not this chip's serial. Type it again" } else { "Type the serial's last four" },
               M, 72, Face::Sans12, if wrong { BAD } else { MUTED }, BG);
        for (i, c) in (0..4).zip(typed.chars().chain(core::iter::repeat(' '))) {
            let x = M + i * 22;
            f.rrect(x, 89, 19, 21, 4, if (i as usize) < typed.len() { if wrong { BAD_BAR } else { RAISED } } else { LINE });
            let mut s = [0u8; 4];
            let s = c.encode_utf8(&mut s);
            let w = font::get(Face::Mono13).width(s);
            f.text(s, x + (19 - w) / 2, 90, Face::Mono13, TEXT, if wrong { BAD_BAR } else { RAISED });
        }
        f.text("from its sticker", M + 4 * 22 + 4, 92, Face::Sans12, FAINT, BG);
        keys(f, Some(("Del", "Cancel")), None);
        match self.hold_since {
            Some(t) => hold(f, (now.wrapping_sub(t).min(HOLD_MS) * 100 / HOLD_MS) as i32),
            None if self.matches() => {
                key(f, "Enter", "Hold 3 s", R, BAD_BAR, ON_BAD, true);
            }
            None => {
                f.text_r("Type all four first", R, FOOT, Face::Sans12, FAINT, BG);
            }
        }
    }
}

/// Why it stopped, in words.
pub struct Reason(pub Stop);

impl core::fmt::Display for Reason {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self.0 {
            Stop::Refused(s) | Stop::Check(s) => f.write_str(s),
            Stop::Chip(what, atecc::Error::Status(c)) => write!(f, "The chip refused while {what}: {} ({c:02x}).", atecc::explain(c)),
            Stop::Chip(what, atecc::Error::NoChip) => write!(f, "No answer from the chip while {what}. Is it on the Grove port?"),
            Stop::Chip(what, e) => write!(f, "A garbled answer while {what} ({e:?})."),
        }
    }
}

fn working(f: &mut Frame, step: Step) {
    let (code, title, permanent) = match step {
        Step::Start => ("Chip", "Reading the chip", false),
        Step::P2 => ("P2", "Writing the rules", false),
        Step::P3 => ("P3", "Locking the rules", true),
        Step::P4 => ("P4", "Zeros and the record", false),
        Step::P5 => ("P5", "Locking the data", true),
        Step::K1 => ("K1", "Making your key", false),
        Step::K2 => ("K2", "Test signatures", false),
        Step::K3 => ("K3", "Sealing your key", true),
    };
    header(f, code, title, if permanent { "Permanent" } else { "Working" });
    f.text("Working…", M, 40, Face::Bold18, TEXT, BG);
    f.text("Don't unplug it now.", M, 66, Face::Sans14, MUTED, BG);
}

fn stopped(f: &mut Frame, stop: Stop, serial: &[u8; 9]) {
    header(f, "Stop", "The key ceremony", "Stop");
    f.mark(false, M + 9, 32, 9);
    let title = if matches!(stop, Stop::Refused(_)) { "Can't prepare this chip" } else { "Stopped" };
    f.text(title, M + 25, 22, Face::Bold14, TEXT, BG);
    if *serial != [0; 9] {
        f.text(grouped(serial).as_str(), M, 44, Face::Mono13, MUTED, BG);
    }
    let mut s: Buf<200> = Buf::new();
    let _ = write!(s, "{}", Reason(stop));
    para(f, s.as_str(), M, 62, TW, Face::Sans12, TEXT, FOOT - 3);
    keys(f, Some(("Del", "Start again")), None);
}

// ----------------------------------------------------------------------------- kit
/// The step's tag and title, and on the right which part it is, or in red that it's permanent (or a
/// stop). Red steps have a red tag.
pub fn header(f: &mut Frame, code: &str, title: &str, right: &str) {
    let red = right == "Permanent" || right.starts_with("Stop");
    let (fill, ink) = if red { (BAD_BAR, ON_BAD) } else { (RAISED, TEXT) };
    let pw = f.pill(code, M - 2, 1, Face::Sans12, fill, ink);
    let end = f.text(title, M + pw + 3, 0, Face::Bold14, TEXT, BG);
    if end + 8 + font::get(Face::Sans12).width(right) <= R {
        f.text_r(right, R, 1, Face::Sans12, if red { BAD } else { FAINT }, BG);
    }
    f.hline(0, 19, W, LINE);
}

pub fn check(f: &mut Frame, y: i32, ok: bool, s: &str) {
    f.mark(ok, M + 6, y + 8, 6);
    f.text(fit(s, TW - 18, Face::Sans12).as_str(), M + 18, y, Face::Sans12, TEXT, BG);
}

fn warn(f: &mut Frame, y: i32, s: &str) {
    f.disc(M + 6, y + 8, 6, WARN);
    f.text("!", M + 4, y + 1, Face::Sans12, ON_WARN, WARN);
    f.text(fit(s, TW - 18, Face::Sans12).as_str(), M + 18, y, Face::Sans12, TEXT, BG);
}

/// Enter held: its keycap red, and a bar beside it filling, `pct` of 3 seconds.
pub fn hold(f: &mut Frame, pct: i32) {
    let kw = key(f, "Enter", "", R, BAD_BAR, ON_BAD, true);
    let (w, x) = (64, R - kw - 70);
    f.rrect(x, FOOT + 3, w, 12, 6, RAISED);
    f.rrect(x, FOOT + 3, (w * pct / 100).max(12), 12, 6, BAD_BAR);
}

/// The footer: a keycap at the left (Del) and one at the right, each with what it does.
pub fn keys(f: &mut Frame, left: Option<(&str, &str)>, right: Option<(&str, &str, Rgb, Rgb)>) {
    f.hline(0, FOOT - 3, W, LINE);
    if let Some((k, label)) = left {
        key(f, k, label, M, BAD_BAR, ON_BAD, false);
    }
    if let Some((k, label, fill, ink)) = right {
        key(f, k, label, R, fill, ink, true);
    }
}

pub fn key(f: &mut Frame, k: &str, label: &str, x: i32, fill: Rgb, ink: Rgb, right: bool) -> i32 {
    let kw = font::get(Face::Bold14).width(k) + 10;
    let w = if label.is_empty() { kw } else { kw + 5 + font::get(Face::Sans12).width(label) };
    let x = if right { x - w } else { x };
    let y = FOOT + 1;
    f.rrect(x, y, kw, 16, 4, fill);
    f.text(k, x + 5, y - 1, Face::Bold14, ink, fill);
    f.text(label, x + kw + 5, y, Face::Sans12, MUTED, BG);
    w
}

/// The serial in groups of four, as the idle screen shows it.
pub fn grouped(serial: &[u8; 9]) -> Buf<24> {
    let mut h = [0u8; 18];
    let s = hex::encode(serial, &mut h);
    let mut g = Buf::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && i % 4 == 0 {
            let _ = g.write_char(' ');
        }
        let _ = g.write_char(ch);
    }
    g
}

/// A CRC as the 16-bit number the Lock command takes.
fn crc(c: [u8; 2]) -> Buf<4> {
    let mut b = Buf::new();
    let _ = write!(b, "{:04x}", ceremony::crc_value(c));
    b
}

/// The key's fingerprint, as the page shows it: the first 8 bytes of sha256(x ‖ y), in fours.
pub fn fingerprint(key: &[u8; 64]) -> Buf<24> {
    let d = sha256(&[key]);
    let mut h = [0u8; 16];
    let s = hex::encode(&d[..8], &mut h);
    let mut b = Buf::new();
    let _ = write!(b, "{} {} {} {}", &s[..4], &s[4..8], &s[8..12], &s[12..]);
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::H;

    // Every page draws inside the screen without panicking, the gate in each of its states.
    #[test]
    fn every_page_draws() {
        let mut buf = [0u8; (W * H * 2) as usize];
        let mut f = Frame::new(&mut buf, W, H);
        let mut c = Ceremony::new([0; 4]);
        for page in [Page::P1, Page::P2, Page::P3, Page::Locked, Page::P4, Page::P5, Page::P6, Page::K1, Page::K1Made,
                     Page::K2, Page::K3, Page::K4, Page::Working(Step::P3), Page::Stopped(Stop::Refused("no")),
                     Page::Stopped(Stop::Chip("signing", atecc::Error::Status(0x0f)))] {
            c.page = page;
            c.draw(&mut f, 0);
            c.typed = Buf::from("12");
            c.draw(&mut f, 0);
            c.typed = Buf::from("1234");
            c.hold_since = Some(0);
            c.draw(&mut f, 1500);
            c.hold_since = None;
        }
    }
}
