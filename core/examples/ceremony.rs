//! The key ceremony's pages (docs/CEREMONY.md), to PNG at 3x, for the ceremony page
//! (web/ceremony.html):
//!   cargo run --example ceremony [outdir]      (default target/screens)
//! A rehearsal: the ceremony image's own pages (`ceremony_pages`), run on a virtual chip
//! (`common/vchip.rs`, serial 0123 4a7b 9c2d 5e6f ee, a 608A) from P1 to K4, and drawn on the way.
//! The key is the virtual chip's, made with the p256 crate. Its log goes to ceremony-rehearsal.txt,
//! which web/test.mjs checks the way the page checks a real chip's. The brand pages and the key-c
//! pages (a 608C's red page, the key's name) are mockups: no firmware draws them.
use signer_core::ceremony_pages::{check, header, hold, keys, Ceremony, Key, Page, HOLD_MS};
use signer_core::font::{self, Face};
use signer_core::gfx::{theme::*, Frame, Rgb};
use signer_core::hex;
use signer_core::screens::{H, W};

#[path = "common/png.rs"]
mod png;
use png::png;
#[path = "common/vchip.rs"]
mod vchip;
use vchip::VChip;

const M: i32 = 6;
const R: i32 = W - M;
const TW: i32 = W - 2 * M;
const FOOT: i32 = 117;
/// The key-c mockups' name for the key (docs/CEREMONY.md, "Its name").
const NAME: &str = "key 2 blue";

/// The virtual chip with the device's pages in front of it, and a clock.
struct Run {
    chip: VChip,
    c: Ceremony,
    log: String,
    t: u32,
}

impl Run {
    fn new(chip: VChip) -> Self {
        let mut r = Run { chip, c: Ceremony::new([0; 4]), log: String::new(), t: 0 };
        r.settle();
        r
    }
    fn input(&mut self, k: Option<Key>) {
        self.t += 40;
        self.c.input(k, self.t, &mut self.chip, &mut self.log);
    }
    fn settle(&mut self) {
        while matches!(self.c.page, Page::Working(_)) {
            self.input(None);
        }
    }
    fn press(&mut self, k: Key) {
        self.input(Some(k));
        self.input(None);
        self.settle();
    }
    fn typed(&mut self, s: &str) {
        s.chars().for_each(|ch| self.press(Key::Char(ch)));
    }
    /// Enter held until `pct` of the hold.
    fn hold_to(&mut self, pct: u32) {
        let end = self.t + 40 + HOLD_MS * pct / 100;
        while self.t < end && !matches!(self.c.page, Page::Working(_)) {
            self.input(Some(Key::Enter));
        }
    }
    fn hold_through(&mut self) {
        self.hold_to(110);
        self.input(Some(Key::Enter));
        self.settle();
        self.input(None);
    }
    fn draw(&self, f: &mut Frame) {
        self.c.draw(f, self.t);
    }
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "target/screens".into());
    std::fs::create_dir_all(&out).unwrap();

    let mut buf = vec![0u8; (W * H * 2) as usize];
    let mut save = |name: &str, draw: &dyn Fn(&mut Frame)| {
        let mut f = Frame::new(&mut buf, W, H);
        f.fill(BG);
        draw(&mut f);
        let path = format!("{out}/ceremony-{name}.png");
        std::fs::write(&path, png(&f, 3)).unwrap();
        println!("{path}");
    };

    // ------------------------------------------------------------------------- the rehearsal
    // my development chip, refused: its config locked with another table
    let mut dev = VChip::factory();
    dev.config[..4].copy_from_slice(&[0x01, 0x23, 0xf3, 0xac]);
    dev.config[8..13].copy_from_slice(&[0xfd, 0x2a, 0x82, 0x6b, 0xee]);
    dev.config[87] = 0;
    let r = Run::new(dev);
    save("p1-refused", &|f| r.draw(f));

    let mut r = Run::new(VChip::factory());
    save("p1", &|f| r.draw(f));
    r.press(Key::Enter);
    save("p2", &|f| r.draw(f));
    r.press(Key::Enter);
    r.typed("5e");
    save("p3", &|f| r.draw(f));
    r.typed("6f");
    r.hold_to(60);
    save("p3-hold", &|f| r.draw(f));
    r.hold_through();
    r.press(Key::Enter);
    save("p4", &|f| r.draw(f));
    r.press(Key::Enter);
    r.typed("5e6f");
    r.hold_to(35);
    save("p5", &|f| r.draw(f));
    r.hold_through();
    save("p6", &|f| r.draw(f));
    r.press(Key::Enter);
    save("k1", &|f| r.draw(f));
    r.press(Key::Enter);
    save("k1-made", &|f| r.draw(f));
    r.press(Key::Enter);
    save("k2", &|f| r.draw(f));
    r.press(Key::Enter);
    r.typed("5e6f");
    r.hold_to(85);
    save("k3", &|f| r.draw(f));
    r.hold_through();
    assert_eq!(r.c.page, Page::K4, "{}", r.log);
    save("k4", &|f| r.draw(f));
    let path = format!("{out}/ceremony-rehearsal.txt");
    std::fs::write(&path, &r.log).unwrap();
    println!("{path}");

    // ------------------------------------------------------------------------- a key-c
    save("p1-new", &|f| {
        header(f, "P1", "Not tried yet", "Spare only");
        f.text("Not tried on a 608C yet.", M, 22, Face::Bold14, TEXT, BG);
        para(f, "Its rules should be a 608A's. Go on only with a spare you can lose.",
             M, 41, TW, Face::Sans12, MUTED);
        gate(f, "5e6f");
        keys(f, Some(("Del", "Cancel")), None);
        hold(f, 50);
    });
    save("p4-name", &|f| {
        header(f, "P4", "Its name", "Part 1 of 2");
        f.text("A name for this key", M, 22, Face::Bold14, TEXT, BG);
        f.rrect(M, 42, TW, 24, 4, RAISED);
        let end = f.text(NAME, M + 6, 45, Face::Mono13, TEXT, RAISED);
        f.rrect(end + 1, 46, 2, 16, 1, TEXT);
        f.text_r("10 / 16", R - 6, 47, Face::Sans12, FAINT, RAISED);
        para(f, "Shown beside its serial. Anyone holding the key can read it. Frozen at P5.",
             M, 74, TW, Face::Sans12, MUTED);
        keys(f, Some(("Del", "No name")), Some(("Enter", "Next", RAISED, TEXT)));
    });

    // ------------------------------------------------------------------------- a branded key
    let brand_seed = hex_of(&vchip::SERIAL) + "0042";
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

/// The serial gate, as the ceremony's red pages draw it, with `typed` in its boxes.
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

fn hex_of(b: &[u8]) -> String {
    let mut h = vec![0u8; b.len() * 2];
    hex::encode(b, &mut h).to_string()
}
