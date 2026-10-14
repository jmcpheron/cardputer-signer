//! A short string built on the stack.
use core::fmt;

#[derive(Clone, Copy)]
pub struct Buf<const N: usize> {
    b: [u8; N],
    n: usize,
}

impl<const N: usize> Buf<N> {
    pub const fn new() -> Self {
        Buf { b: [0; N], n: 0 }
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("")
    }

    pub fn from(s: &str) -> Self {
        let mut b = Self::new();
        let _ = fmt::Write::write_str(&mut b, s);
        b
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
}

impl<const N: usize> Default for Buf<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> fmt::Write for Buf<N> {
    /// Fails, leaving what fit, once full; never splits a character.
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            let w = c.len_utf8();
            if self.n + w > N {
                return Err(fmt::Error);
            }
            c.encode_utf8(&mut self.b[self.n..self.n + w]);
            self.n += w;
        }
        Ok(())
    }
}

impl<const N: usize> fmt::Display for Buf<N> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
