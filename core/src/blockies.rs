//! Blockies identicons (github.com/ethereum/blockies), ported from the MicroPython `blockies.py`
//! that started in picowallet by Austin Griffith (MIT), so the same seed draws the same picture as the Safe app and the
//! website. Every quirk of the JS is kept: 32-bit shifts mixed with plain numbers, rand() in
//! [0, 2), CSS's hsl() clamping. Checked against `vectors/blockies.json` (CPython's output).
use crate::gfx::Rgb;

pub struct Blockie {
    /// 8x8 cells, row-major: 0 background, 1 colour, 2 spot colour
    pub cells: [u8; 64],
    pub color: Rgb,
    pub bg: Rgb,
    pub spot: Rgb,
}

fn wrap32(x: i64) -> i64 {
    x as i32 as i64
}

struct Rand([i64; 4]);

impl Rand {
    fn new(seed: &[u8]) -> Self {
        let mut rs = [0i64; 4];
        for (i, &ch) in seed.iter().enumerate() {
            let k = i % 4;
            // The JS: (rs << 5) is 32-bit, the subtraction and the add are not.
            rs[k] = wrap32(wrap32(rs[k]) << 5) - rs[k] + ch as i64;
        }
        Rand(rs)
    }

    /// The JS rand() numerator: an unsigned 32-bit value; the JS divides it by 2**31.
    fn u(&mut self) -> u64 {
        let rs = &mut self.0;
        let r0 = wrap32(rs[0]);
        let t = r0 ^ wrap32(r0 << 11);
        rs[0] = rs[1];
        rs[1] = rs[2];
        rs[2] = rs[3];
        let r3 = wrap32(rs[3]);
        rs[3] = r3 ^ (r3 >> 19) ^ t ^ (t >> 8);
        (rs[3] as u64) & 0xFFFF_FFFF
    }
}

/// (h, s, l) as the JS builds them: h an int that can pass 359, s and l percentages.
fn hsl(r: &mut Rand) -> (u64, f64, f64) {
    let h = (r.u() * 360) >> 31;
    let s = (r.u() * 60) as f64 / 2147483648.0 + 40.0;
    let l = (r.u() + r.u() + r.u() + r.u()) as f64 * 25.0 / 2147483648.0;
    (h, s, l)
}

fn abs(x: f64) -> f64 {
    if x < 0.0 { -x } else { x }
}

fn floor(x: f64) -> f64 {
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

/// Python's round(): half to even.
fn round(x: f64) -> i32 {
    let f = floor(x);
    let d = x - f;
    let r = if d > 0.5 { f + 1.0 } else if d < 0.5 { f } else if (f as i64) % 2 == 0 { f } else { f + 1.0 };
    r as i32
}

/// CSS hsl(): the hue wraps, saturation and lightness clamp to 0..100.
fn rgb((h, s, l): (u64, f64, f64)) -> Rgb {
    let h = (h % 360) as f64;
    let s = s.clamp(0.0, 100.0) / 100.0;
    let l = l.clamp(0.0, 100.0) / 100.0;
    let c = (1.0 - abs(2.0 * l - 1.0)) * s;
    let hp = h / 60.0;
    let hmod2 = hp - 2.0 * floor(hp / 2.0);
    let x = c * (1.0 - abs(hmod2 - 1.0));
    let (r, g, b) = if hp < 1.0 {
        (c, x, 0.0)
    } else if hp < 2.0 {
        (x, c, 0.0)
    } else if hp < 3.0 {
        (0.0, c, x)
    } else if hp < 4.0 {
        (0.0, x, c)
    } else if hp < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    let m = l - c / 2.0;
    let ch = |v: f64| round((v + m) * 255.0).clamp(0, 255) as u8;
    Rgb(ch(r), ch(g), ch(b))
}

pub fn build(seed: &[u8]) -> Blockie {
    let mut r = Rand::new(seed);
    let color = hsl(&mut r);
    let bg = hsl(&mut r);
    let spot = hsl(&mut r);
    let mut cells = [0u8; 64];
    for row in 0..8 {
        let mut half = [0u8; 4];
        for v in half.iter_mut() {
            *v = ((r.u() * 23) / (10 << 31)) as u8; // floor(rand() * 2.3), exactly
        }
        for col in 0..4 {
            cells[row * 8 + col] = half[col];
            cells[row * 8 + 7 - col] = half[col];
        }
    }
    Blockie { cells, color: rgb(color), bg: rgb(bg), spot: rgb(spot) }
}
