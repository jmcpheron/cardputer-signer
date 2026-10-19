//! The hum image's pages (docs/HUM.md), to PNG at 3x:
//!   cargo run --example hum [outdir]      (default target/screens)
//! A run through the image's own pages (`hum_pages`), with the device's tones standing in for a
//! voice on the mic: a tune set in three rounds, tried, and the mic checked. The tune is the
//! example's, nobody's secret.
use signer_core::gfx::{theme::BG, Frame};
use signer_core::hum::Tones;
use signer_core::hum_pages::{Hum, Key, Page};
use signer_core::screens::{H, W};

#[path = "common/png.rs"]
mod png;
use png::png;

const TUNE: &[(u8, u16)] = &[(60, 300), (62, 300), (64, 600), (64, 300), (67, 300), (65, 600), (62, 300), (60, 900)];
const OTHER: &[(u8, u16)] = &[(60, 300), (62, 300), (64, 600), (64, 300), (67, 300), (69, 600), (62, 300), (60, 900)];

/// What the mic sends while `notes` are hummed: 32 kHz, two slots, the second dead, a quiet room
/// before and after.
fn mic(notes: &'static [(u8, u16)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut seed = 1u32;
    let mut push = |x: i16| {
        for _ in 0..2 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            out.extend_from_slice(&(x + (seed >> 28) as i16 - 8).to_le_bytes());
            out.extend_from_slice(&0i16.to_le_bytes());
        }
    };
    (0..8000).for_each(|_| push(0));
    Tones::new(notes).for_each(&mut push);
    (0..36000).for_each(|_| push(0));
    out
}

fn press(h: &mut Hum, k: Key) {
    h.input(Some(k), 0);
    h.input(None, 0);
}

/// The opening played, then `notes` heard.
fn go(h: &mut Hum, notes: &'static [(u8, u16)]) {
    h.played();
    hear(h, &mic(notes));
}

fn hear(h: &mut Hum, bytes: &[u8]) {
    for chunk in bytes.chunks(4092) {
        h.feed(chunk);
        h.input(None, 0);
        if !h.listening() {
            break;
        }
    }
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "target/screens".into());
    std::fs::create_dir_all(&out).unwrap();
    let mut buf = vec![0u8; (W * H * 2) as usize];
    let mut save = |name: &str, h: &Hum| {
        let mut f = Frame::new(&mut buf, W, H);
        f.fill(BG);
        h.draw(&mut f, 0);
        let path = format!("{out}/hum-{name}.png");
        std::fs::write(&path, png(&f, 3)).unwrap();
        println!("{path}");
    };

    let mut h = Hum::new();
    save("home", &h);
    press(&mut h, Key::Enter);
    press(&mut h, Key::Right);
    save("pick", &h);
    press(&mut h, Key::Enter);
    save("play", &h);
    h.played();
    let bytes = mic(TUNE);
    let half = bytes.len() / 2 / 4 * 4;
    hear(&mut h, &bytes[..half]);
    save("listen", &h);
    hear(&mut h, &bytes[half..]);
    assert!(matches!(h.page, Page::Heard(_)), "{:?}", h.page);
    save("heard", &h);
    press(&mut h, Key::Enter);
    go(&mut h, TUNE);
    press(&mut h, Key::Enter);
    go(&mut h, TUNE);
    assert_eq!(h.page, Page::Set);
    save("set", &h);
    press(&mut h, Key::Enter);
    save("home-set", &h);

    press(&mut h, Key::Enter);
    go(&mut h, TUNE);
    assert_eq!(h.page, Page::Match(true));
    save("match", &h);
    press(&mut h, Key::Enter);
    press(&mut h, Key::Enter);
    go(&mut h, OTHER);
    assert_eq!(h.page, Page::Match(false));
    save("no-match", &h);
    press(&mut h, Key::Enter);

    // three rounds that differ
    press(&mut h, Key::Up);
    press(&mut h, Key::Enter);
    press(&mut h, Key::Enter);
    go(&mut h, TUNE);
    press(&mut h, Key::Enter);
    go(&mut h, OTHER);
    press(&mut h, Key::Enter);
    go(&mut h, TUNE);
    assert_eq!(h.page, Page::Differ);
    save("differ", &h);
    press(&mut h, Key::Enter);
    press(&mut h, Key::Enter);
    go(&mut h, &[(64, 400), (64, 400), (65, 400)]);
    save("short", &h);
    press(&mut h, Key::Del);

    // the mic page, halfway through a tune
    press(&mut h, Key::Down);
    press(&mut h, Key::Down);
    press(&mut h, Key::Enter);
    let bytes = mic(TUNE);
    for chunk in bytes[..bytes.len() / 3].chunks(4092) {
        h.feed(chunk);
    }
    save("mic", &h);
}
