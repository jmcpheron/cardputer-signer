//! A whole-screen RGB565 framebuffer and the few shapes the screens use. The bytes are in the
//! panel's order (high byte first), so the firmware sends `bytes()` as is.
use crate::blockies;
use crate::font::{self, Face};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn to565(self) -> u16 {
        ((self.0 as u16 & 0xF8) << 8) | ((self.1 as u16 & 0xFC) << 3) | (self.2 as u16 >> 3)
    }

    /// From self (coverage 0) to `to` (coverage 15).
    pub fn mix(self, to: Rgb, cov: u8) -> Rgb {
        let m = |a: u8, b: u8| (a as i32 + (b as i32 - a as i32) * cov as i32 / 15) as u8;
        Rgb(m(self.0, to.0), m(self.1, to.1), m(self.2, to.2))
    }
}

/// The dark theme.
pub mod theme {
    use super::Rgb;
    pub const BG: Rgb = Rgb(11, 13, 17);
    pub const SURFACE: Rgb = Rgb(24, 28, 36);
    pub const RAISED: Rgb = Rgb(40, 47, 58);
    pub const LINE: Rgb = Rgb(48, 55, 66);
    pub const TEXT: Rgb = Rgb(242, 244, 247);
    pub const MUTED: Rgb = Rgb(152, 160, 172);
    pub const FAINT: Rgb = Rgb(98, 106, 118);
    pub const OK: Rgb = Rgb(52, 199, 89);
    pub const ON_OK: Rgb = Rgb(6, 30, 12);
    pub const BAD: Rgb = Rgb(255, 92, 84);
    pub const BAD_BAR: Rgb = Rgb(200, 38, 38);
    pub const ON_BAD: Rgb = Rgb(255, 255, 255);
    pub const WARN: Rgb = Rgb(255, 184, 48);
    pub const ON_WARN: Rgb = Rgb(40, 26, 0);
    pub const BASE_SEPOLIA: Rgb = Rgb(150, 90, 255);
    pub const WHITE: Rgb = Rgb(255, 255, 255);
}

pub struct Frame<'a> {
    buf: &'a mut [u8],
    pub w: i32,
    pub h: i32,
}

fn isqrt(n: i32) -> i32 {
    let mut r = 0;
    while (r + 1) * (r + 1) <= n {
        r += 1;
    }
    r
}

impl<'a> Frame<'a> {
    pub fn new(buf: &'a mut [u8], w: i32, h: i32) -> Self {
        assert_eq!(buf.len(), (w * h * 2) as usize);
        Frame { buf, w, h }
    }

    pub fn bytes(&self) -> &[u8] {
        self.buf
    }

    /// The colour at (x, y), back from RGB565 (for previews and tests).
    pub fn get(&self, x: i32, y: i32) -> Rgb {
        let i = ((y * self.w + x) * 2) as usize;
        let v = u16::from_be_bytes([self.buf[i], self.buf[i + 1]]);
        let (r, g, b) = ((v >> 11) as u8, (v >> 5 & 63) as u8, (v & 31) as u8);
        Rgb(r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2)
    }

    pub fn pixel(&mut self, x: i32, y: i32, c: Rgb) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            let i = ((y * self.w + x) * 2) as usize;
            self.buf[i..i + 2].copy_from_slice(&c.to565().to_be_bytes());
        }
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgb) {
        let (x0, y0) = (x.max(0), y.max(0));
        let (x1, y1) = ((x + w).min(self.w), (y + h).min(self.h));
        let v = c.to565().to_be_bytes();
        for yy in y0..y1 {
            for xx in x0..x1 {
                let i = ((yy * self.w + xx) * 2) as usize;
                self.buf[i..i + 2].copy_from_slice(&v);
            }
        }
    }

    pub fn fill(&mut self, c: Rgb) {
        self.fill_rect(0, 0, self.w, self.h, c);
    }

    pub fn hline(&mut self, x: i32, y: i32, w: i32, c: Rgb) {
        self.fill_rect(x, y, w, 1, c);
    }

    /// A line `t` pixels thick (Bresenham, with the pen a t x t square).
    pub fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, t: i32, c: Rgb) {
        let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
        let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
        let (mut x, mut y, mut err) = (x0, y0, dx + dy);
        loop {
            self.fill_rect(x - t / 2, y - t / 2, t, t, c);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// A round check (ok) or cross.
    pub fn mark(&mut self, ok: bool, cx: i32, cy: i32, r: i32) {
        self.disc(cx, cy, r, if ok { theme::OK } else { theme::BAD_BAR });
        let ink = if ok { theme::ON_OK } else { theme::ON_BAD };
        let (k, t) = (r * 45 / 100, r / 6 + 1);
        if ok {
            self.line(cx - k, cy, cx - k / 3, cy + k * 2 / 3, t, ink);
            self.line(cx - k / 3, cy + k * 2 / 3, cx + k, cy - k * 2 / 3, t, ink);
        } else {
            self.line(cx - k, cy - k, cx + k, cy + k, t, ink);
            self.line(cx - k, cy + k, cx + k, cy - k, t, ink);
        }
    }

    pub fn disc(&mut self, cx: i32, cy: i32, r: i32, c: Rgb) {
        for dy in -r..=r {
            let half = isqrt(r * r - dy * dy);
            self.hline(cx - half, cy + dy, 2 * half + 1, c);
        }
    }

    pub fn rrect(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, c: Rgb) {
        for dy in 0..h {
            let k = if dy < r { r - dy } else if dy >= h - r { dy - (h - r) + 1 } else { 0 };
            let inset = if k > 0 { r - isqrt(r * r - (k - 1) * (k - 1)) } else { 0 };
            self.hline(x + inset, y + dy, w - 2 * inset, c);
        }
    }

    /// `s` with the top of its line box at y, blended onto `bg`. Returns the x after it.
    pub fn text(&mut self, s: &str, x: i32, y: i32, face: Face, fg: Rgb, bg: Rgb) -> i32 {
        let f = font::get(face);
        let mut x = x;
        for ch in s.chars() {
            let g = f.glyph(ch);
            for gy in 0..g.h as usize {
                for gx in 0..g.w as usize {
                    let cov = g.at(gx, gy);
                    if cov != 0 {
                        self.pixel(x + g.xo as i32 + gx as i32, y + g.yo as i32 + gy as i32, bg.mix(fg, cov));
                    }
                }
            }
            x += g.adv as i32;
        }
        x
    }

    pub fn text_r(&mut self, s: &str, xr: i32, y: i32, face: Face, fg: Rgb, bg: Rgb) -> i32 {
        let w = font::get(face).width(s);
        self.text(s, xr - w, y, face, fg, bg)
    }

    pub fn text_c(&mut self, s: &str, y: i32, face: Face, fg: Rgb, bg: Rgb) -> i32 {
        let w = font::get(face).width(s);
        self.text(s, (self.w - w) / 2, y, face, fg, bg)
    }

    /// Text in a rounded tag, top-left at (x, y). Returns its width.
    pub fn pill(&mut self, s: &str, x: i32, y: i32, face: Face, fill: Rgb, ink: Rgb) -> i32 {
        let f = font::get(face);
        let w = f.width(s) + 12;
        let h = f.height as i32;
        self.rrect(x, y, w, h, h / 2 - 1, fill);
        self.text(s, x + 6, y, face, ink, fill);
        w
    }

    /// The blockie for `seed`, 8x8 cells of `scale` px.
    pub fn blockie(&mut self, seed: &[u8], x: i32, y: i32, scale: i32) {
        let b = blockies::build(seed);
        for (i, &v) in b.cells.iter().enumerate() {
            let c = match v { 0 => b.bg, 1 => b.color, _ => b.spot };
            self.fill_rect(x + (i % 8) as i32 * scale, y + (i / 8) as i32 * scale, scale, scale, c);
        }
    }
}
