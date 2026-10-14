//! Anti-aliased fonts (Atkinson Hyperlegible Next and Mono, SIL OFL 1.1,
//! `fonts/OFL.txt`), read in place from the `qf_*.bin` files built by its `tools/fonts/gen.py`.
//!
//! Format: "QF\x01", height, ascent, first char, count, one pad byte; then `count` glyph records
//! of 8 bytes (u16 offset, w, h, i8 x offset, i8 y offset, advance, pad); then the pixels, 4-bit
//! coverage, two to a byte, high nibble first, rows padded to a whole byte.

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Face {
    Sans12,
    Sans14,
    Bold14,
    Bold18,
    Bold26,
    Mono13,
    MonoB28,
}

static SANS12: &[u8] = include_bytes!("../../fonts/qf_sans12.bin");
static SANS14: &[u8] = include_bytes!("../../fonts/qf_sans14.bin");
static BOLD14: &[u8] = include_bytes!("../../fonts/qf_bold14.bin");
static BOLD18: &[u8] = include_bytes!("../../fonts/qf_bold18.bin");
static BOLD26: &[u8] = include_bytes!("../../fonts/qf_bold26.bin");
static MONO13: &[u8] = include_bytes!("../../fonts/qf_mono13.bin");
static MONOB28: &[u8] = include_bytes!("../../fonts/qf_monob28.bin");

pub struct Font {
    buf: &'static [u8],
    pub height: u8,
    pub ascent: u8,
    first: u8,
    count: u8,
}

pub struct Glyph {
    pub w: u8,
    pub h: u8,
    pub xo: i8,
    pub yo: i8,
    pub adv: u8,
    /// 4-bit coverage, `(w + 1) / 2` bytes a row
    pub px: &'static [u8],
}

impl Glyph {
    /// Coverage 0..15 of the pixel at (x, y) in the glyph.
    pub fn at(&self, x: usize, y: usize) -> u8 {
        let b = self.px[y * ((self.w as usize + 1) / 2) + x / 2];
        if x % 2 == 0 { b >> 4 } else { b & 15 }
    }
}

pub fn get(face: Face) -> Font {
    let buf = match face {
        Face::Sans12 => SANS12,
        Face::Sans14 => SANS14,
        Face::Bold14 => BOLD14,
        Face::Bold18 => BOLD18,
        Face::Bold26 => BOLD26,
        Face::Mono13 => MONO13,
        Face::MonoB28 => MONOB28,
    };
    assert!(&buf[..3] == b"QF\x01");
    Font { buf, height: buf[3], ascent: buf[4], first: buf[5], count: buf[6] }
}

impl Font {
    /// The glyph for `c`; "?" for anything the font doesn't have. Three extras sit after "~".
    pub fn glyph(&self, c: char) -> Glyph {
        let code = match c {
            '…' => 127,
            '·' => 128,
            '—' => 129,
            _ => c as u32,
        };
        let mut i = code.wrapping_sub(self.first as u32);
        if i >= self.count as u32 {
            i = '?' as u32 - self.first as u32;
        }
        let r = &self.buf[8 + 8 * i as usize..];
        let off = u16::from_le_bytes([r[0], r[1]]) as usize;
        let (w, h) = (r[2], r[3]);
        let data = &self.buf[8 + 8 * self.count as usize..];
        let n = h as usize * ((w as usize + 1) / 2);
        Glyph { w, h, xo: r[4] as i8, yo: r[5] as i8, adv: r[6], px: &data[off..off + n] }
    }

    pub fn width(&self, s: &str) -> i32 {
        s.chars().map(|c| self.glyph(c).adv as i32).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_face_parses() {
        for f in [Face::Sans12, Face::Sans14, Face::Bold14, Face::Bold18, Face::Bold26, Face::Mono13] {
            let font = get(f);
            assert!(font.height > 10 && font.ascent < font.height);
            assert!(font.width("Send 0.01 ETH") > 0);
            assert!(font.glyph('…').adv > 0);
            assert_eq!(font.glyph('\u{1F600}').adv, font.glyph('?').adv);
        }
        // The big mono holds only what a verify code needs: hex digits.
        assert!(get(Face::MonoB28).width("9ca4cdd4") > 0);
        let m = get(Face::Mono13);
        assert_eq!(m.width("0000"), m.width("ffff"));
    }
}
