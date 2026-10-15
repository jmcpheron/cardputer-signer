//! The key ceremony's pages (docs/CEREMONY.md), drawn with the signer's own fonts and drawing code,
//! to PNG at 3x, for the ceremony page (web/ceremony.html):
//!   cargo run --example ceremony [outdir]      (default target/screens)
//! Mockups: no firmware draws these yet. The chip is made up (serial 0123 4a7b 9c2d 5e6f ee, a 608A),
//! and its CRCs are worked out from the ceremony's table. The key and its address are a test
//! vector's (vectors/webauthn.json, "eth-send-base"), not a chip's.
use signer_core::atecc::crc16;
use signer_core::font::{self, Face};
use signer_core::gfx::{theme::*, Frame, Rgb};
use signer_core::hex;
use signer_core::screens::{H, W};

#[path = "common/png.rs"]
mod png;
use png::png;

const M: i32 = 6;
const R: i32 = W - M;
const TW: i32 = W - 2 * M;
const FOOT: i32 = 117;

/// The ceremony's table, diykey v1 (docs/CEREMONY.md), bytes 16-127.
const TABLE: [u8; 112] = [
    0xC0, 0x00, 0x00, 0x00, 0xA1, 0x2F, 0x9F, 0x8F, 0x9F, 0x8F, 0x9F, 0x8F, 0x9F, 0x8F, 0x9F, 0x8F,
    0x9F, 0x8F, 0x9F, 0x8F, 0x1F, 0x8F, 0x9F, 0x8F, 0x9F, 0x8F, 0x9F, 0x8F, 0x9F, 0x8F, 0x9F, 0x8F,
    0x9F, 0x8F, 0x9F, 0x8F, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x55, 0x55, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x33, 0x00, 0x1C, 0x00, 0x1C, 0x00, 0x1C, 0x00, 0x1C, 0x00, 0x1C, 0x00, 0x1C, 0x00, 0x1C, 0x00,
    0x1C, 0x00, 0x1C, 0x00, 0x1C, 0x00, 0x1C, 0x00, 0x1C, 0x00, 0x1C, 0x00, 0x1C, 0x00, 0x1C, 0x00,
];
const SERIAL: [u8; 9] = [0x01, 0x23, 0x4a, 0x7b, 0x9c, 0x2d, 0x5e, 0x6f, 0xee];
const REVISION: [u8; 4] = [0x00, 0x00, 0x60, 0x02];
/// The test vector's public key x (vectors/webauthn.json) and its Safe signer address.
const KEY_X: &str = "8c856dc317df3805ccab682182e06c694f956991df3a213d204db4a886fbc196";
const OWNER: &str = "0xe46d0b1321f75d2b940ae1ec9b1603f774f6c916";

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "target/screens".into());
    std::fs::create_dir_all(&out).unwrap();

    // The config CRC that P3 locks with: the chip's own bytes, the table, the lock bytes still 55 55.
    let mut cfg = [0u8; 128];
    cfg[..4].copy_from_slice(&SERIAL[..4]);
    cfg[4..8].copy_from_slice(&REVISION);
    cfg[8..13].copy_from_slice(&SERIAL[4..]);
    cfg[14] = 0x01;
    cfg[16..].copy_from_slice(&TABLE);
    let config_crc = crc16(&cfg);
    // The record (docs/CEREMONY.md, "The record"): prepared by its owner, no brand, build 0 here.
    let mut record = [0u8; 64];
    record[..4].copy_from_slice(&[b'D', b'K', 1, 1]);
    record[4..13].copy_from_slice(&SERIAL);
    record[13..17].copy_from_slice(&REVISION);
    record[17..19].copy_from_slice(&[config_crc[1], config_crc[0]]);
    // The data CRC that P5 locks with: every slot that isn't a key (1-15, all zeros), then OTP.
    let mut data = vec![0u8; 7 * 36 + 416 + 7 * 72];
    data.extend_from_slice(&record);
    let data_crc = crc16(&data);
    let (ccrc, dcrc) = (hex2(&config_crc), hex2(&data_crc));

    let mut buf = vec![0u8; (W * H * 2) as usize];
    let mut save = |name: &str, draw: &dyn Fn(&mut Frame)| {
        let mut f = Frame::new(&mut buf, W, H);
        f.fill(BG);
        draw(&mut f);
        let path = format!("{out}/ceremony-{name}.png");
        std::fs::write(&path, png(&f, 3)).unwrap();
        println!("{path}");
    };

    // ------------------------------------------------------------------------- part 1
    save("p1", &|f| {
        header(f, "P1", "Read the chip", "Part 1 of 2");
        let s = grouped(&SERIAL);
        f.blockie(hex_of(&SERIAL).as_bytes(), M, 25, 3);
        f.text("ATECC608A", M + 32, 21, Face::Bold14, TEXT, BG);
        f.text(&s, M + 32, 38, Face::Mono13, MUTED, BG);
        check(f, 60, true, "Both zones open: nothing locked yet");
        check(f, 76, true, "Self-test passed");
        check(f, 92, true, "The factory config, sent to the log");
        keys(f, Some(("Del", "Back")), Some(("Enter", "Write the rules", RAISED, TEXT)));
    });
    save("p1-refused", &|f| {
        header(f, "P1", "Read the chip", "Part 1 of 2");
        f.mark(false, M + 9, 32, 9);
        f.text("Can't prepare this chip", M + 25, 22, Face::Bold14, TEXT, BG);
        f.text("0123 f3ac fd2a 826b ee", M, 44, Face::Mono13, MUTED, BG);
        para(f, "Its rules were locked before, and they aren't these. Nothing was written. Use a fresh chip.",
             M, 64, TW, Face::Sans12, TEXT);
        keys(f, Some(("Del", "Back")), None);
    });
    save("p2", &|f| {
        header(f, "P2", "Write the rules", "Part 1 of 2");
        check(f, 23, true, "Written and read back, not locked yet");
        f.text("Config CRC", M, 43, Face::Sans12, MUTED, BG);
        f.text(&ccrc, M, 56, Face::MonoB28, TEXT, BG);
        para(f, "The page works it out too. They should match.", M + 100, 60, R - M - 100, Face::Sans12, MUTED);
        keys(f, Some(("Del", "Back")), Some(("Enter", "Next", RAISED, TEXT)));
    });
    let p3_body = |f: &mut Frame| {
        header(f, "P3", "Lock the rules", "Permanent");
        f.text("Locks the rules for good.", M, 22, Face::Bold14, TEXT, BG);
        let mut s = String::from("The chip checks CRC ");
        s.push_str(&ccrc);
        s.push_str(" first. If one byte differs, it locks nothing.");
        para(f, &s, M, 41, TW, Face::Sans12, MUTED);
    };
    save("p3", &|f| {
        p3_body(f);
        gate(f, "5e");
        keys(f, Some(("Del", "Cancel")), None);
        f.text_r("Type all four first", R, FOOT, Face::Sans12, FAINT, BG);
    });
    save("p3-hold", &|f| {
        p3_body(f);
        gate(f, "5e6f");
        keys(f, Some(("Del", "Cancel")), None);
        hold(f, 60);
    });
    save("p4", &|f| {
        header(f, "P4", "Zeros and the record", "Part 1 of 2");
        check(f, 23, true, "Slots 1-15: zeros");
        check(f, 39, true, "The record, into OTP");
        f.text("Data CRC", M, 59, Face::Sans12, MUTED, BG);
        f.text(&dcrc, M, 72, Face::MonoB28, TEXT, BG);
        para(f, "Rewritable until P5 locks it.", M + 100, 76, R - M - 100, Face::Sans12, MUTED);
        keys(f, Some(("Del", "Back")), Some(("Enter", "Next", RAISED, TEXT)));
    });
    save("p5", &|f| {
        header(f, "P5", "Lock the data", "Permanent");
        let mut s = String::from("Freezes every slot and the record, checked against CRC ");
        s.push_str(&dcrc);
        s.push_str(". After this, only the chip's own GenKey can put a key in slot 0.");
        para(f, &s, M, 22, TW, Face::Sans12, TEXT);
        gate(f, "5e6f");
        keys(f, Some(("Del", "Cancel")), None);
        hold(f, 35);
    });
    save("p6", &|f| {
        header(f, "P6", "Check", "Part 1 of 2");
        f.mark(true, M + 11, 33, 11);
        f.text("Ready for a key", M + 30, 21, Face::Bold18, TEXT, BG);
        check(f, 52, true, "Rules and data locked");
        check(f, 68, true, "Slot 0: empty, not sealed");
        check(f, 84, true, "Counter 0: 0, never used");
        keys(f, Some(("Del", "Later")), Some(("Enter", "Make your key", OK, ON_OK)));
    });

    // ------------------------------------------------------------------------- part 2
    save("k1", &|f| {
        header(f, "K1", "Make your key", "Part 2 of 2");
        f.text("Made inside the chip", M, 21, Face::Bold18, TEXT, BG);
        para(f, "Nobody chooses it, and it can't be read out or copied. You can make it again until you seal it.",
             M, 50, TW, Face::Sans12, MUTED);
        keys(f, Some(("Del", "Back")), Some(("Enter", "Make it", OK, ON_OK)));
    });
    save("k1-made", &|f| {
        header(f, "K1", "Your key", "Part 2 of 2");
        let mut seed = String::from("0x");
        seed.push_str(KEY_X);
        f.blockie(seed.as_bytes(), M, 24, 4);
        f.text("P-256, in slot 0", M + 40, 22, Face::Bold14, TEXT, BG);
        let fp = format!("{} {} {} {}", &KEY_X[..4], &KEY_X[4..8], &KEY_X[8..12], &KEY_X[12..16]);
        f.text(&fp, M + 40, 39, Face::Mono13, MUTED, BG);
        check(f, 64, true, "Made by the chip's GenKey, just now");
        check(f, 80, true, "Slot 1 refuses GenKey");
        f.text("Not sealed yet. Next: test signatures.", M, 96, Face::Sans12, FAINT, BG);
        keys(f, Some(("Del", "Back")), Some(("Enter", "Test it", RAISED, TEXT)));
    });
    save("k2", &|f| {
        header(f, "K2", "Test it", "Part 2 of 2");
        check(f, 23, true, "Test signature 1: valid");
        check(f, 39, true, "Test signature 2: valid");
        check(f, 55, true, "Slot 1 refuses to sign");
        para(f, "Checked by the page, with WebCrypto. Counter 0 is now 3.",
             M, 75, TW, Face::Sans12, MUTED);
        keys(f, Some(("Del", "Back")), Some(("Enter", "Seal it", RAISED, TEXT)));
    });
    save("k3", &|f| {
        header(f, "K3", "Seal your key", "Permanent");
        para(f, "Your key can never be changed or replaced. Lose the chip, and your Safe's other owners swap it out.",
             M, 22, TW, Face::Sans12, TEXT);
        gate(f, "5e6f");
        keys(f, Some(("Del", "Cancel")), None);
        hold(f, 85);
    });
    save("k4", &|f| {
        header(f, "K4", "Its address", "Part 2 of 2");
        f.text("Safe signer address, sealed key", M, 21, Face::Sans12, MUTED, BG);
        f.blockie(OWNER.as_bytes(), M, 39, 4);
        let short = format!("0x{}…{}", &OWNER[2..6], &OWNER[38..]);
        f.text(&short, M + 40, 46, Face::Bold18, TEXT, BG);
        para(f, "A signer, not a wallet: don't send funds here. Funds go to your Safe.", M, 78, TW, Face::Sans12, MUTED);
        keys(f, None, Some(("Enter", "Done", OK, ON_OK)));
    });

    // ------------------------------------------------------------------------- a branded key
    let brand_seed = hex_of(&SERIAL) + "0042";
    save("brand", &|f| {
        header(f, "Brand", "Check it", "Part 2, first");
        f.blockie(brand_seed.as_bytes(), M, 24, 4);
        f.text("diykey key 0042", M + 40, 21, Face::Bold18, TEXT, BG);
        f.text("Does your label say 0042?", M + 40, 44, Face::Sans12, MUTED, BG);
        check(f, 64, true, "Signed by diykey key 0000");
        check(f, 80, true, "Serial and rules: this chip's");
        check(f, 96, true, "Locked, slot 0 empty, unused");
        keys(f, Some(("Del", "No")), Some(("Enter", "Yes, it matches", RAISED, TEXT)));
    });
    save("brand-bad", &|f| {
        header(f, "Brand", "Check it", "Stop");
        f.mark(false, M + 11, 33, 11);
        f.text("Not a diykey brand", M + 30, 21, Face::Bold18, TEXT, BG);
        para(f, "Its signature doesn't verify. Nothing was written. Don't use this chip, and tell the project the number on its label.",
             M, 54, TW, Face::Sans12, TEXT);
        keys(f, Some(("Del", "Back")), None);
    });
    // the label's picture: the same blockie, alone
    let mut cells = vec![0u8; 8 * 8 * 2];
    let mut f = Frame::new(&mut cells, 8, 8);
    f.blockie(brand_seed.as_bytes(), 0, 0, 1);
    let path = format!("{out}/ceremony-brand-label.png");
    std::fs::write(&path, png(&f, 12)).unwrap();
    println!("{path}");
}

// ----------------------------------------------------------------------------- kit
/// The step's tag and title, and on the right which part it is, or in red that it's permanent
/// (or a stop). Red steps have a red tag.
fn header(f: &mut Frame, code: &str, title: &str, right: &str) {
    let red = !right.starts_with("Part");
    let (fill, ink) = if red { (BAD_BAR, ON_BAD) } else { (RAISED, TEXT) };
    let pw = f.pill(code, M - 2, 1, Face::Sans12, fill, ink);
    let end = f.text(title, M + pw + 3, 0, Face::Bold14, TEXT, BG);
    if end + 8 + font::get(Face::Sans12).width(right) <= R {
        f.text_r(right, R, 1, Face::Sans12, if red { BAD } else { FAINT }, BG);
    }
    f.hline(0, 19, W, LINE);
}

fn check(f: &mut Frame, y: i32, ok: bool, s: &str) {
    f.mark(ok, M + 6, y + 8, 6);
    f.text(s, M + 18, y, Face::Sans12, TEXT, BG);
}

/// The serial's last group of four, typed from its sticker, in four boxes.
fn gate(f: &mut Frame, typed: &str) {
    f.text("Type the serial's last four", M, 72, Face::Sans12, MUTED, BG);
    let t: Vec<char> = typed.chars().collect();
    for i in 0..4 {
        let x = M + i * 22;
        f.rrect(x, 89, 19, 21, 4, if i < t.len() as i32 { RAISED } else { LINE });
        if let Some(c) = t.get(i as usize) {
            let mut s = [0u8; 4];
            let s = c.encode_utf8(&mut s);
            let w = font::get(Face::Mono13).width(s);
            f.text(s, x + (19 - w) / 2, 90, Face::Mono13, TEXT, RAISED);
        }
    }
    f.text("from its sticker", M + 4 * 22 + 4, 92, Face::Sans12, FAINT, BG);
}

/// Enter held: its keycap red, and a bar beside it filling, `pct` of 3 seconds.
fn hold(f: &mut Frame, pct: i32) {
    let kw = key(f, "Enter", "", R, BAD_BAR, ON_BAD, true);
    let (w, x) = (64, R - kw - 70);
    f.rrect(x, FOOT + 3, w, 12, 6, RAISED);
    f.rrect(x, FOOT + 3, (w * pct / 100).max(12), 12, 6, BAD_BAR);
}

/// The footer: a keycap at the left (Del) and one at the right, each with what it does.
fn keys(f: &mut Frame, left: Option<(&str, &str)>, right: Option<(&str, &str, Rgb, Rgb)>) {
    f.hline(0, FOOT - 3, W, LINE);
    if let Some((k, label)) = left {
        key(f, k, label, M, BAD_BAR, ON_BAD, false);
    }
    if let Some((k, label, fill, ink)) = right {
        key(f, k, label, R, fill, ink, true);
    }
}

fn key(f: &mut Frame, k: &str, label: &str, x: i32, fill: Rgb, ink: Rgb, right: bool) -> i32 {
    let kw = font::get(Face::Bold14).width(k) + 10;
    let w = if label.is_empty() { kw } else { kw + 5 + font::get(Face::Sans12).width(label) };
    let x = if right { x - w } else { x };
    let y = FOOT + 1;
    f.rrect(x, y, kw, 16, 4, fill);
    f.text(k, x + 5, y - 1, Face::Bold14, ink, fill);
    f.text(label, x + kw + 5, y, Face::Sans12, MUTED, BG);
    w
}

/// Words wrapped to `w` pixels from y, above the footer.
fn para(f: &mut Frame, s: &str, x: i32, y: i32, w: i32, face: Face, fg: Rgb) -> i32 {
    let font = font::get(face);
    let pitch = font.height as i32 - 2;
    let (mut y, mut line) = (y, String::new());
    for word in s.split(' ') {
        let cand = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if font.width(&cand) > w && !line.is_empty() {
            f.text(&line, x, y, face, fg, BG);
            y += pitch;
            line = word.to_string();
        } else {
            line = cand;
        }
    }
    if !line.is_empty() && y + pitch <= FOOT - 3 {
        f.text(&line, x, y, face, fg, BG);
        y += pitch;
    }
    y
}

fn hex_of(b: &[u8]) -> String {
    let mut h = vec![0u8; b.len() * 2];
    hex::encode(b, &mut h).to_string()
}

/// A CRC as the 16-bit number the Lock command takes (`crc[0] | crc[1] << 8`).
fn hex2(b: &[u8; 2]) -> String {
    hex_of(&[b[1], b[0]])
}

/// The serial in groups of four, as the idle screen shows it.
fn grouped(b: &[u8]) -> String {
    let h = hex_of(b);
    h.as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join(" ")
}
