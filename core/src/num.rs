//! uint256 amounts as people read them: every significant digit, thousands grouped, as
//! the MicroPython decoder's units() does (1500000 with 6 decimals is "1.5").
use core::fmt::{self, Write};

use crate::safe_tx::U256;

/// The decimal digits of `n`, most significant first ("0" for zero).
pub fn dec(n: &U256, out: &mut [u8; 78]) -> usize {
    let mut v = *n;
    let mut digits = [0u8; 78];
    let mut k = 0;
    loop {
        let mut rem = 0u32;
        for b in v.iter_mut() {
            let cur = rem << 8 | *b as u32;
            *b = (cur / 10) as u8;
            rem = cur % 10;
        }
        digits[k] = b'0' + rem as u8;
        k += 1;
        if v.iter().all(|&b| b == 0) {
            break;
        }
    }
    for i in 0..k {
        out[i] = digits[k - 1 - i];
    }
    k
}

/// "12345678" as "12,345,678".
pub fn group<W: Write>(digits: &str, out: &mut W) -> fmt::Result {
    let n = digits.len();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (n - i) % 3 == 0 {
            out.write_char(',')?;
        }
        out.write_char(c)?;
    }
    Ok(())
}

/// `n` in units with `decimals` places, trailing zeros dropped.
pub fn units<W: Write>(n: &U256, decimals: usize, out: &mut W) -> fmt::Result {
    let mut d = [0u8; 78];
    let len = dec(n, &mut d);
    let s = core::str::from_utf8(&d[..len]).unwrap_or("0");
    if decimals == 0 {
        return group(s, out);
    }
    let (whole, frac_digits) = if len > decimals { (&s[..len - decimals], &s[len - decimals..]) } else { ("0", s) };
    group(whole, out)?;
    let pad = decimals.saturating_sub(frac_digits.len());
    let frac = frac_digits.trim_end_matches('0');
    if !frac.is_empty() {
        out.write_char('.')?;
        for _ in 0..pad {
            out.write_char('0')?;
        }
        out.write_str(frac)?;
    }
    Ok(())
}

pub fn is_zero(n: &U256) -> bool {
    n.iter().all(|&b| b == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buf::Buf;
    use crate::safe_tx::{u256_from_dec, u256_from_u64};

    fn u(n: u64, d: usize) -> Buf<120> {
        let mut b = Buf::new();
        units(&u256_from_u64(n), d, &mut b).unwrap();
        b
    }

    #[test]
    fn amounts() {
        assert_eq!(u(1_500_000, 6).as_str(), "1.5");
        assert_eq!(u(0, 18).as_str(), "0");
        assert_eq!(u(1, 18).as_str(), "0.000000000000000001");
        assert_eq!(u(10_000_000_000_000_000, 18).as_str(), "0.01");
        assert_eq!(u(1_234_567, 0).as_str(), "1,234,567");
        assert_eq!(u(123_456_789_000_000, 6).as_str(), "123,456,789");
        let mut b: Buf<120> = Buf::new();
        units(&[0xff; 32], 0, &mut b).unwrap();
        assert!(b.as_str().starts_with("115,792,089,237"));
        let mut d = [0u8; 78];
        let max = u256_from_dec("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap();
        assert_eq!(dec(&max, &mut d), 78);
    }
}
