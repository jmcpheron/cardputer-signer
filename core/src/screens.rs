//! The screens, drawn from plain state into a `Frame` 240x135. The laptop draws the same ones to
//! PNG (`cargo run --example screens`).
use core::fmt::Write;

use crate::atecc;
pub use crate::buf::Buf;
use crate::decode::{self, Item, Level, Review};
use crate::font::{self, Face};
use crate::gfx::{theme::*, Frame, Rgb};
use crate::hex;
use crate::num;
use crate::proto::TxFields;
use crate::safe_tx::{self, Address, DELEGATECALL};

pub const W: i32 = 240;
pub const H: i32 = 135;
const M: i32 = 6;
const R: i32 = W - M;
const TW: i32 = W - 2 * M;
const FOOT: i32 = 117;

#[derive(Clone, Copy)]
pub struct ChipInfo {
    pub serial: [u8; 9],
    pub revision: [u8; 4],
    pub config_locked: bool,
    pub data_locked: bool,
    pub counter0: u32,
    /// slot 0's public key, x ‖ y, when it has one
    pub pubkey0: Option<[u8; 64]>,
}

#[derive(Clone, Copy)]
pub enum Chip {
    Missing,
    Error(atecc::Error),
    Found(ChipInfo),
    /// The emulator's key: a passkey on P-256, named by the start of its credential id.
    Passkey([u8; 8]),
}

/// A transaction under review: its fields, what it does, and the hash this device worked out.
pub struct Pending {
    pub fields: TxFields,
    pub review: Review,
    pub hash: [u8; 32],
}

impl Pending {
    pub fn new(fields: TxFields, data: &[u8]) -> Result<Self, safe_tx::Error> {
        let tx = fields.tx(data);
        let hash = safe_tx::safe_tx_hash(fields.chain, &fields.safe, &tx)?;
        let review = decode::review(fields.chain, &fields.safe, &tx);
        Ok(Pending { fields, review, hash })
    }

    /// What page `page` shows: each item, then its note on a page of its own when the item also
    /// has an amount (there is no room for both), then the fields, then the hash.
    pub fn page(&self, page: usize) -> Page {
        let mut k = 0;
        for (i, it) in self.review.items.iter().enumerate() {
            let Some(it) = it else { continue };
            if k == page {
                return Page::Item(i);
            }
            k += 1;
            if !it.amount.is_empty() && !it.note.is_empty() {
                if k == page {
                    return Page::Note(i);
                }
                k += 1;
            }
        }
        if k == page { Page::Fields } else { Page::Hash }
    }

    pub fn pages(&self) -> usize {
        let notes = self.review.items.iter().flatten().filter(|it| !it.amount.is_empty() && !it.note.is_empty()).count();
        self.review.count() + notes + 2
    }

    /// Whether the computer's `expect` matches: None when it sent none.
    pub fn matches(&self) -> Option<bool> {
        self.fields.expect.map(|e| e == self.hash)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Page {
    Item(usize),
    Note(usize),
    Fields,
    Hash,
}

// ----------------------------------------------------------------------------- kit
/// `s` broken anywhere (it's a number) into lines of at most `w` pixels, from y; at most `max`
/// lines. Returns the y after it, or None if it needs more lines than that.
fn wrap_chars(f: &mut Frame, s: &str, x: i32, y: i32, w: i32, face: Face, fg: Rgb, max: i32) -> Option<i32> {
    let font = font::get(face);
    let pitch = font.height as i32 - 2;
    let (mut lines, mut used) = (1, 0);
    for c in s.chars() {
        let a = font.glyph(c).adv as i32;
        if used + a > w {
            lines += 1;
            used = 0;
        }
        used += a;
    }
    if lines > max {
        return None;
    }
    let (mut y, mut line, mut used): (i32, Buf<160>, i32) = (y, Buf::new(), 0);
    for c in s.chars() {
        let a = font.glyph(c).adv as i32;
        if used + a > w {
            f.text(line.as_str(), x, y, face, fg, BG);
            y += pitch;
            line = Buf::new();
            used = 0;
        }
        let _ = line.write_char(c);
        used += a;
    }
    f.text(line.as_str(), x, y, face, fg, BG);
    Some(y + pitch)
}

/// An amount as big as it fits, never cut: one line of bold26 or bold18, else wrapped smaller.
fn amount(f: &mut Frame, s: &str, y: i32) -> i32 {
    for (face, max) in [(Face::Bold26, 1), (Face::Bold18, 2), (Face::Bold14, 3), (Face::Sans12, 6)] {
        if let Some(after) = wrap_chars(f, s, M, y, TW, face, TEXT, max) {
            return after;
        }
    }
    y
}

pub(crate) fn short(a: &Address) -> Buf<16> {
    let mut h = [0u8; 40];
    let s = hex::encode(a, &mut h);
    let mut b = Buf::new();
    let _ = write!(b, "0x{}…{}", &s[..4], &s[36..]);
    b
}

fn nonce(n: &safe_tx::U256) -> Buf<80> {
    let mut d = [0u8; 78];
    let k = num::dec(n, &mut d);
    Buf::from(core::str::from_utf8(&d[..k]).unwrap_or("?"))
}

/// `s` cut with an ellipsis to fit `w` pixels.
pub(crate) fn fit(s: &str, w: i32, face: Face) -> Buf<160> {
    let f = font::get(face);
    if f.width(s) <= w {
        return Buf::from(s);
    }
    let room = w - f.width("…");
    let mut out: Buf<160> = Buf::new();
    let mut used = 0;
    for c in s.chars() {
        let a = f.glyph(c).adv as i32;
        if used + a > room {
            break;
        }
        used += a;
        let _ = out.write_char(c);
    }
    let _ = out.write_char('…');
    out
}

/// Wrapped text from y, stopping before `bottom`. Returns the y after it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn para(f: &mut Frame, s: &str, x: i32, y: i32, w: i32, face: Face, fg: Rgb, bottom: i32) -> i32 {
    let font = font::get(face);
    let pitch = font.height as i32 - 2;
    let (mut y, mut line): (i32, Buf<120>) = (y, Buf::new());
    for word in s.split(' ') {
        let mut cand: Buf<120> = line;
        if !cand.is_empty() {
            let _ = cand.write_char(' ');
        }
        let _ = cand.write_str(word);
        if font.width(cand.as_str()) > w && !line.is_empty() {
            if y + pitch > bottom {
                return y;
            }
            f.text(line.as_str(), x, y, face, fg, BG);
            y += pitch;
            line = Buf::from(word);
        } else {
            line = cand;
        }
    }
    if !line.is_empty() && y + pitch <= bottom {
        f.text(line.as_str(), x, y, face, fg, BG);
        y += pitch;
    }
    y
}

/// A keycap and what it does, on the footer line. Returns its width.
#[allow(clippy::too_many_arguments)]
pub(crate) fn key(f: &mut Frame, k: &str, label: &str, x: i32, fill: Rgb, ink: Rgb, lab: Rgb, right: bool) -> i32 {
    let kw = font::get(Face::Bold14).width(k) + 10;
    let w = kw + 5 + font::get(Face::Sans12).width(label);
    let x = if right { x - w } else { x };
    let y = FOOT + 1;
    f.rrect(x, y, kw, 16, 4, fill);
    f.text(k, x + 5, y - 1, Face::Bold14, ink, fill);
    f.text(label, x + kw + 5, y, Face::Sans12, lab, BG);
    w
}

fn chain(id: u64) -> (&'static str, Rgb, Rgb) {
    match id {
        1 => ("Ethereum", Rgb(98, 126, 234), WHITE),
        8453 => ("Base", Rgb(0, 82, 255), WHITE),
        84532 => ("Base Sepolia", BASE_SEPOLIA, WHITE),
        11155111 => ("Sepolia", Rgb(120, 120, 120), WHITE),
        31337 => ("Local", Rgb(255, 149, 0), Rgb(26, 16, 0)),
        _ => ("Chain ?", Rgb(120, 120, 120), WHITE),
    }
}

fn accent(l: Level) -> (Rgb, Rgb) {
    match l {
        Level::Ok => (OK, ON_OK),
        Level::Warn => (WARN, ON_WARN),
        Level::Red => (BAD_BAR, ON_BAD),
    }
}

fn header(f: &mut Frame, p: &Pending, page: usize) {
    let (name, fill, ink) = chain(p.fields.chain);
    let pw = f.pill(name, M - 2, 1, Face::Sans12, fill, ink);
    let mut n: Buf<8> = Buf::new();
    let _ = write!(n, "{}/{}", page + 1, p.pages());
    let nw = font::get(Face::Sans12).width(n.as_str());
    let mut s: Buf<120> = Buf::new();
    let _ = write!(s, "Safe {} · #{}", short(&p.fields.safe), nonce(&p.fields.nonce));
    f.text(fit(s.as_str(), R - nw - 8 - (M + pw + 4), Face::Sans12).as_str(), M + pw + 4, 1, Face::Sans12, MUTED, BG);
    f.text_r(n.as_str(), R, 1, Face::Sans12, FAINT, BG);
    f.hline(0, 18, W, LINE);
}

fn footer(f: &mut Frame, p: &Pending, page: usize, can_sign: bool) {
    f.hline(0, FOOT - 3, W, LINE);
    key(f, "Del", "Reject", M, BAD_BAR, ON_BAD, MUTED, false);
    if page + 1 < p.pages() {
        key(f, "Enter", "Next", R, RAISED, TEXT, MUTED, true);
    } else if p.matches() == Some(false) {
        f.text_r("Can't sign this", R, FOOT, Face::Sans12, BAD, BG);
    } else if can_sign {
        key(f, "Enter", "Sign", R, OK, ON_OK, MUTED, true);
    } else {
        f.text_r("Signing: milestone 3", R, FOOT, Face::Sans12, FAINT, BG);
    }
}

// ----------------------------------------------------------------------------- idle
/// Waiting for a transaction. `last` is what happened to the one before ("Rejected #12").
pub fn idle(f: &mut Frame, chip: &Chip, last: Option<&str>) {
    f.fill(BG);
    f.text("Cardputer signer", M, 0, Face::Bold14, TEXT, BG);
    f.text_r("over USB", R, 1, Face::Sans12, FAINT, BG);
    f.hline(0, 19, W, LINE);
    match chip {
        Chip::Found(c) => {
            let mut h = [0u8; 18];
            let s = hex::encode(&c.serial, &mut h);
            f.blockie(s.as_bytes(), M, 26, 3);
            let model = match c.revision[2..] {
                [0x60, 0x02] => "ATECC608A",
                [0x60, 0x03] => "ATECC608B",
                [0x60, 0x05] => "ATECC608C",
                _ => "ATECC608?",
            };
            f.text(model, M + 32, 23, Face::Bold14, TEXT, BG);
            // counter 0, to check against the number on the key's sticker (docs/CEREMONY.md)
            let mut n: Buf<20> = Buf::new();
            let _ = write!(n, "counter {}", c.counter0);
            f.text_r(n.as_str(), R, 25, Face::Sans12, FAINT, BG);
            let mut g: Buf<24> = Buf::new();
            for (i, ch) in s.chars().enumerate() {
                if i > 0 && i % 4 == 0 {
                    let _ = g.write_char(' ');
                }
                let _ = g.write_char(ch);
            }
            f.text(g.as_str(), M + 32, 39, Face::Mono13, MUTED, BG);
        }
        Chip::Missing => {
            f.disc(M + 9, 36, 9, WARN);
            f.text("!", M + 7, 27, Face::Bold14, ON_WARN, WARN);
            f.text("No chip: review only", M + 26, 27, Face::Bold14, TEXT, BG);
        }
        Chip::Error(_) => {
            f.disc(M + 9, 36, 9, BAD_BAR);
            f.text("Chip error: review only", M + 26, 27, Face::Bold14, TEXT, BG);
        }
        Chip::Passkey(id) => {
            let mut h = [0u8; 16];
            let s = hex::encode(id, &mut h);
            f.blockie(s.as_bytes(), M, 26, 3);
            f.text("Passkey", M + 32, 23, Face::Bold14, TEXT, BG);
            let mut g: Buf<24> = Buf::new();
            let _ = write!(g, "{} {} · P-256", &s[..4], &s[4..8]);
            f.text(g.as_str(), M + 32, 39, Face::Mono13, MUTED, BG);
        }
    }
    f.text("Waiting for a transaction", M, 62, Face::Bold18, TEXT, BG);
    f.text("from a computer, over the USB cable", M, 86, Face::Sans12, MUTED, BG);
    f.hline(0, FOOT - 3, W, LINE);
    match last {
        Some(s) => f.text(fit(s, TW, Face::Sans12).as_str(), M, FOOT, Face::Sans12, MUTED, BG),
        None => f.text("Enter: read the chip again", M, FOOT, Face::Sans12, FAINT, BG),
    };
}

// ----------------------------------------------------------------------------- review
fn to_line(f: &mut Frame, label: &str, a: &Address, y: i32, scale: i32) {
    let lw = f.text(label, M, y + scale * 4 - 8, Face::Sans12, MUTED, BG) - M;
    let mut h = [0u8; 42];
    h[..2].copy_from_slice(b"0x");
    let seed = hex::encode(a, &mut h[2..]).len() + 2;
    let x = M + lw + 6;
    f.blockie(&h[..seed], x, y, scale);
    f.text(short(a).as_str(), x + 8 * scale + 6, y + scale * 4 - 9, Face::Bold14, TEXT, BG);
}

fn item_page(f: &mut Frame, it: &Item) {
    let (fill, ink) = accent(it.level);
    f.pill(it.title.as_str(), M - 2, 23, Face::Sans12, fill, ink);
    let bottom = FOOT - 2;
    if !it.amount.is_empty() {
        // the note, if any, is on the next page (Page::Note)
        let y = amount(f, it.amount.as_str(), 41);
        if let Some(to) = &it.to {
            let scale = if y + 2 + 32 <= bottom { 4 } else if y + 2 + 24 <= bottom { 3 } else { 2 };
            to_line(f, it.label, to, y + 2, scale);
        }
    } else {
        f.text(fit(it.what.as_str(), TW, Face::Bold18).as_str(), M, 41, Face::Bold18, TEXT, BG);
        let mut y = 66;
        if let Some(to) = &it.to {
            to_line(f, it.label, to, 64, 2);
            y = 83;
        }
        para(f, it.note, M, y, TW, Face::Sans12, MUTED, bottom);
    }
}

/// An item's note, for an item whose amount filled its own page.
fn note_page(f: &mut Frame, it: &Item) {
    let (fill, ink) = accent(it.level);
    f.pill(it.title.as_str(), M - 2, 23, Face::Sans12, fill, ink);
    para(f, it.note, M, 44, TW, Face::Sans14, TEXT, FOOT - 2);
}

fn fields_page(f: &mut Frame, p: &Pending, data: &[u8]) {
    let t = &p.fields;
    let x = M + 40;
    let label = |f: &mut Frame, s: &str, y: i32| {
        f.text(s, M, y, Face::Sans12, MUTED, BG);
    };
    let mut h = [0u8; 40];
    let s = hex::encode(&t.to, &mut h);
    label(f, "to", 23);
    let mut l1: Buf<24> = Buf::new();
    let _ = write!(l1, "0x{}", &s[..20]);
    f.text(l1.as_str(), x, 22, Face::Mono13, TEXT, BG);
    f.text(&s[20..], x + font::get(Face::Mono13).width("0x"), 37, Face::Mono13, TEXT, BG);
    label(f, "value", 54);
    let mut v: Buf<120> = Buf::new();
    let _ = num::units(&t.value, 18, &mut v);
    let _ = v.write_str(" ETH");
    let y = match wrap_chars(f, v.as_str(), x, 54, R - x, Face::Sans12, TEXT, 2) {
        Some(y) => y,
        None => {
            f.text("too long for here: on page 1", x, 54, Face::Sans12, WARN, BG);
            54 + 14
        }
    };
    label(f, "data", y + 2);
    let mut d: Buf<64> = Buf::new();
    if t.data_len == 0 {
        let _ = d.write_str("none");
    } else {
        let mut sel = [0u8; 8];
        let n = t.data_len.min(4);
        let _ = write!(d, "{} bytes, 0x{}", t.data_len, hex::encode(&data[..n], &mut sel));
    }
    let _ = d.write_str(" · ");
    let dx = f.text(d.as_str(), x, y + 2, Face::Sans12, TEXT, BG);
    if t.operation == DELEGATECALL {
        f.text("DELEGATECALL", dx, y + 2, Face::Sans12, BAD, BG);
    } else {
        f.text("call", dx, y + 2, Face::Sans12, TEXT, BG);
    }
    label(f, "nonce", y + 18);
    if wrap_chars(f, nonce(&t.nonce).as_str(), x, y + 18, R - x, Face::Sans12, TEXT, 2).is_none() {
        f.text("too long to show", x, y + 18, Face::Sans12, BAD, BG);
    }
}

fn hash_page(f: &mut Frame, p: &Pending) {
    let mut h = [0u8; 66];
    h[..2].copy_from_slice(b"0x");
    hex::encode(&p.hash, &mut h[2..]);
    let hs = core::str::from_utf8(&h).unwrap_or("");
    let code = safe_tx::verify_code(&p.hash);
    let code = core::str::from_utf8(&code).unwrap_or("");
    let ok = p.matches() != Some(false);
    let (status, col) = match p.matches() {
        None => ("Worked out on this device", MUTED),
        Some(true) => ("Matches the computer's", OK),
        Some(false) => ("Not the computer's hash", BAD),
    };
    f.text(fit(status, R - 46 - M, Face::Sans12).as_str(), M, 21, Face::Sans12, col, BG);
    f.text(code, M, 36, Face::MonoB28, if ok { TEXT } else { BAD }, BG);
    f.blockie(hs.as_bytes(), R - 40, 24, 5);
    if ok {
        let body = &hs[2..];
        for (i, y) in [70, 84, 98].into_iter().enumerate() {
            let chunks = &body[i * 24..(i * 24 + 24).min(64)];
            let mut line: Buf<32> = Buf::new();
            for (j, c) in chunks.as_bytes().chunks(4).enumerate() {
                if j > 0 {
                    let _ = line.write_char(' ');
                }
                let _ = line.write_str(core::str::from_utf8(c).unwrap_or(""));
            }
            if i == 2 {
                let s = line.as_str();
                let x = f.text(&s[..s.len() - 9], 4, y, Face::Mono13, FAINT, BG);
                f.text(&s[s.len() - 9..], x, y, Face::Mono13, TEXT, BG);
            } else {
                f.text(line.as_str(), 4, y, Face::Mono13, FAINT, BG);
            }
        }
    } else if let Some(e) = p.fields.expect {
        let c = safe_tx::verify_code(&e);
        let mut s: Buf<40> = Buf::new();
        let _ = write!(s, "The computer says {}", core::str::from_utf8(&c).unwrap_or(""));
        f.text(s.as_str(), M, 68, Face::Sans12, MUTED, BG);
        para(f, "Don't sign. Something between you and the Safe changed it.", M, 84, TW, Face::Sans12, TEXT, FOOT - 2);
    }
}

/// Page `page` of the review: an item, the fields, or the hash. Nothing can sign here yet
/// (milestone 3); `review_sign` is the same with a key that can.
pub fn review(f: &mut Frame, p: &Pending, data: &[u8], page: usize) {
    review_sign(f, p, data, page, false);
}

/// The review, with Enter to sign on the hash page when `can_sign` (the emulator's passkey).
pub fn review_sign(f: &mut Frame, p: &Pending, data: &[u8], page: usize, can_sign: bool) {
    f.fill(BG);
    header(f, p, page);
    match p.page(page) {
        Page::Item(i) => {
            if let Some(it) = &p.review.items[i] {
                item_page(f, it);
            }
        }
        Page::Note(i) => {
            if let Some(it) = &p.review.items[i] {
                note_page(f, it);
            }
        }
        Page::Fields => fields_page(f, p, data),
        Page::Hash => hash_page(f, p),
    }
    footer(f, p, page, can_sign);
}

/// Signed by the key, over the hash this device worked out.
pub fn signed(f: &mut Frame, p: &Pending) {
    f.fill(BG);
    f.mark(true, M + 13, 17, 13);
    let mut t: Buf<96> = Buf::new();
    let _ = write!(t, "Signed #{}", nonce(&p.fields.nonce));
    f.text(fit(t.as_str(), R - M - 33, Face::Bold18).as_str(), M + 33, 6, Face::Bold18, TEXT, BG);
    let code = safe_tx::verify_code(&p.hash);
    f.text(core::str::from_utf8(&code).unwrap_or(""), M, 36, Face::MonoB28, OK, BG);
    let mut h = [0u8; 66];
    h[..2].copy_from_slice(b"0x");
    hex::encode(&p.hash, &mut h[2..]);
    f.blockie(&h, R - 40, 34, 5);
    f.text("Over the hash this device", M, 70, Face::Sans14, MUTED, BG);
    f.text("worked out, and no other.", M, 87, Face::Sans14, MUTED, BG);
    f.hline(0, FOOT - 3, W, LINE);
    f.text(fit(p.review.summary(), R - M - 80, Face::Sans12).as_str(), M, FOOT, Face::Sans12, FAINT, BG);
    key(f, "Enter", "Done", R, RAISED, TEXT, MUTED, true);
}

/// One press of Del: nothing signed.
pub fn rejected(f: &mut Frame, p: &Pending) {
    f.fill(BG);
    f.mark(false, M + 13, 17, 13);
    let mut t: Buf<96> = Buf::new();
    let _ = write!(t, "Rejected #{}", nonce(&p.fields.nonce));
    f.text(fit(t.as_str(), R - M - 33, Face::Bold18).as_str(), M + 33, 6, Face::Bold18, TEXT, BG);
    f.text("Nothing was signed.", M, 40, Face::Bold18, TEXT, BG);
    f.text("The key wasn't used.", M, 64, Face::Sans14, MUTED, BG);
    f.text(fit(p.review.summary(), TW, Face::Sans12).as_str(), M, 88, Face::Sans12, FAINT, BG);
    f.hline(0, FOOT - 3, W, LINE);
    key(f, "Enter", "Back", R, RAISED, TEXT, MUTED, true);
}
