//! The emulator: the signer's core in WebAssembly, for web/emu.js. It keeps the firmware's state
//! (firmware/src/bin/main.rs) and answers in the same lines, so the page treats it as another
//! Cardputer. Its key is a passkey. On the hash page, after every page before it, Enter asks the
//! page to have the passkey sign the hash this worked out (`= sign 0x…`), and no other.
//!
//! No bindings crate: the page writes into `line_buf`, calls an export, and reads the lines it
//! returns from `out_buf` and the screen from `rgba_buf`.
use core::fmt::Write;
use std::cell::RefCell;

use signer_core::gfx::Frame;
use signer_core::proto::{self, Cmd};
use signer_core::screens::{self, Buf, Chip, Pending};
use signer_core::{hex, num};

const W: usize = screens::W as usize;
const H: usize = screens::H as usize;
const LINE_LEN: usize = 5120;

// Keys, as the page numbers them: the Cardputer's ; . , / Enter Del.
const UP: u32 = 1;
const DOWN: u32 = 2;
const LEFT: u32 = 3;
const RIGHT: u32 = 4;
const ENTER: u32 = 5;
const DEL: u32 = 6;

enum Screen {
    Idle,
    Review(usize),
    /// Waiting for the passkey, on the hash page.
    Signing,
    Signed,
    Rejected,
}

struct Emu {
    fb: Vec<u8>,
    rgba: Vec<u8>,
    line: Vec<u8>,
    data: Vec<u8>,
    out: String,
    chip: Chip,
    mode: Screen,
    pending: Option<Pending>,
    last: Buf<96>,
}

thread_local! {
    static EMU: RefCell<Emu> = RefCell::new(Emu {
        fb: vec![0; W * H * 2],
        rgba: vec![0; W * H * 4],
        line: vec![0; LINE_LEN],
        data: vec![0; 2048],
        out: String::new(),
        chip: Chip::Missing,
        mode: Screen::Idle,
        pending: None,
        last: Buf::new(),
    });
}

fn with<T>(f: impl FnOnce(&mut Emu) -> T) -> T {
    EMU.with(|e| f(&mut e.borrow_mut()))
}

macro_rules! say {
    ($e:expr, $($t:tt)*) => {{
        let _ = writeln!($e.out, $($t)*);
    }};
}

impl Emu {
    fn chip_line(&mut self) {
        match self.chip {
            Chip::Passkey(id) => {
                let mut h = [0u8; 16];
                say!(self, "chip: passkey {}", hex::encode(&id, &mut h));
            }
            _ => say!(self, "chip: no key (make a passkey to sign)"),
        }
    }

    fn submit(&mut self, len: usize) {
        if len > self.line.len() {
            say!(self, "= error line too long");
            return;
        }
        let line = core::mem::take(&mut self.line);
        let text = core::str::from_utf8(&line[..len]).unwrap_or("");
        let text = text.trim();
        if text == "ping" {
            say!(self, "= pong cardputer-signer emulator");
        } else if text.is_empty() {
        } else if text.split_ascii_whitespace().next() != Some("tx") {
            say!(self, "= error unknown command");
        } else if matches!(self.mode, Screen::Review(_) | Screen::Signing) {
            // never swap the transaction under someone's eyes
            say!(self, "= busy");
        } else {
            match proto::parse(text, &mut self.data) {
                Ok(Cmd::Tx(t)) => match Pending::new(t, &self.data) {
                    Ok(pd) => {
                        let mut h = [0u8; 64];
                        say!(self, "= hash 0x{}", hex::encode(&pd.hash, &mut h));
                        if pd.matches() == Some(false) {
                            say!(self, "= mismatch");
                        }
                        say!(self, "tx: {} ({} pages)", pd.review.summary(), pd.pages());
                        self.pending = Some(pd);
                        self.mode = Screen::Review(0);
                    }
                    Err(e) => say!(self, "= error {:?}", e),
                },
                Ok(Cmd::Ping) => say!(self, "= pong cardputer-signer emulator"),
                Err(e) => say!(self, "= error {}", e),
            }
        }
        self.line = line;
    }

    fn done(&mut self, word: &str) {
        if let Some(p) = &self.pending {
            self.last = Buf::new();
            let mut d = [0u8; 78];
            let len = num::dec(&p.fields.nonce, &mut d);
            let _ = write!(self.last, "{} #{}: {}", word, core::str::from_utf8(&d[..len]).unwrap_or("?"), p.review.summary());
        }
        self.mode = Screen::Idle;
        self.pending = None;
    }

    fn key(&mut self, k: u32) {
        let can_sign = matches!(self.chip, Chip::Passkey(_));
        match (&self.mode, &self.pending) {
            (Screen::Review(page), Some(p)) => {
                let (page, pages) = (*page, p.pages());
                if k == DEL {
                    say!(self, "= rejected");
                    self.mode = Screen::Rejected;
                } else if (k == ENTER || k == RIGHT || k == DOWN) && page + 1 < pages {
                    self.mode = Screen::Review(page + 1);
                } else if (k == LEFT || k == UP) && page > 0 {
                    self.mode = Screen::Review(page - 1);
                } else if k == ENTER && page + 1 == pages && p.matches() != Some(false) && can_sign {
                    // Paging is one step at a time, so being here means every page was seen.
                    let mut h = [0u8; 64];
                    say!(self, "= sign 0x{}", hex::encode(&p.hash, &mut h));
                    self.mode = Screen::Signing;
                }
            }
            (Screen::Rejected, Some(_)) if k == ENTER || k == DEL => self.done("Rejected"),
            (Screen::Signed, Some(_)) if k == ENTER || k == DEL => self.done("Signed"),
            (Screen::Idle, _) if k == ENTER => self.chip_line(),
            _ => {}
        }
    }

    fn signed(&mut self, ok: bool) {
        if !matches!(self.mode, Screen::Signing) {
            return;
        }
        if ok {
            say!(self, "= signed");
            self.mode = Screen::Signed;
        } else {
            say!(self, "= not signed");
            let last = self.pending.as_ref().map_or(0, |p| p.pages() - 1);
            self.mode = Screen::Review(last);
        }
    }

    fn draw(&mut self) {
        let can_sign = matches!(self.chip, Chip::Passkey(_));
        let mut f = Frame::new(&mut self.fb, screens::W, screens::H);
        match (&self.mode, &self.pending) {
            (Screen::Review(page), Some(p)) => screens::review_sign(&mut f, p, &self.data, *page, can_sign),
            (Screen::Signing, Some(p)) => screens::review_sign(&mut f, p, &self.data, p.pages() - 1, can_sign),
            (Screen::Signed, Some(p)) => screens::signed(&mut f, p),
            (Screen::Rejected, Some(p)) => screens::rejected(&mut f, p),
            _ => screens::idle(&mut f, &self.chip, if self.last.is_empty() { None } else { Some(self.last.as_str()) }),
        }
        for y in 0..H {
            for x in 0..W {
                let c = f.get(x as i32, y as i32);
                let i = (y * W + x) * 4;
                self.rgba[i..i + 4].copy_from_slice(&[c.0, c.1, c.2, 255]);
            }
        }
    }
}

/// Runs `f`, then returns how many bytes of reply lines are in `out_buf`.
fn reply(f: impl FnOnce(&mut Emu)) -> usize {
    with(|e| {
        e.out.clear();
        f(e);
        e.out.len()
    })
}

#[no_mangle]
pub extern "C" fn width() -> usize {
    W
}

#[no_mangle]
pub extern "C" fn height() -> usize {
    H
}

#[no_mangle]
pub extern "C" fn line_buf() -> *mut u8 {
    with(|e| e.line.as_mut_ptr())
}

#[no_mangle]
pub extern "C" fn line_cap() -> usize {
    LINE_LEN
}

#[no_mangle]
pub extern "C" fn out_buf() -> *const u8 {
    with(|e| e.out.as_ptr())
}

#[no_mangle]
pub extern "C" fn rgba_buf() -> *const u8 {
    with(|e| e.rgba.as_ptr())
}

/// The boot banner and the key, as the firmware logs them at start-up.
#[no_mangle]
pub extern "C" fn boot() -> usize {
    reply(|e| {
        say!(e, "cardputer-signer: emulator");
        e.chip_line();
    })
}

/// One line from the computer, `len` bytes in `line_buf`.
#[no_mangle]
pub extern "C" fn submit(len: usize) -> usize {
    reply(|e| e.submit(len))
}

/// A key press: 1 up, 2 down, 3 left, 4 right, 5 Enter, 6 Del.
#[no_mangle]
pub extern "C" fn key(k: u32) -> usize {
    reply(|e| e.key(k))
}

/// The passkey: its credential id's first 8 bytes in `line_buf`, or `n` = 0 for none.
#[no_mangle]
pub extern "C" fn set_passkey(n: usize) -> usize {
    reply(|e| {
        e.chip = if n >= 8 {
            let mut id = [0u8; 8];
            id.copy_from_slice(&e.line[..8]);
            Chip::Passkey(id)
        } else {
            Chip::Missing
        };
        e.chip_line();
    })
}

/// What the passkey did with `= sign`: 1 signed (the page checked the signature), 0 not.
#[no_mangle]
pub extern "C" fn sign_result(ok: u32) -> usize {
    reply(|e| e.signed(ok == 1))
}

/// Draws the screen into `rgba_buf` (240 × 135, RGBA).
#[no_mangle]
pub extern "C" fn draw() {
    with(|e| e.draw())
}
