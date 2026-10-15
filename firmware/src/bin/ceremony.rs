//! The key ceremony on the Cardputer v1.1 (docs/CEREMONY.md): its own image, the only one that
//! writes, locks or makes a key. Flash it, run it, and flash the signer back.
//!
//! The steps and the pages are signer-core's (`ceremony`, `ceremony_pages`), the same code
//! `cargo test` runs on a virtual chip; this file feeds them the keys and the time, and draws. Every
//! line it prints goes to the computer for the page's log: the factory config, the CRCs, the public
//! key, the test signatures and the address. Nothing secret: the key never leaves the chip.
#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use core::fmt::Write;

use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::main;
use esp_hal::time::Instant;
use firmware::{say, Board, Out, DEL, ENTER};
use signer_core::ceremony_pages::{Ceremony, Key};
use signer_core::gfx::Frame;
use signer_core::{hex, keys, screens};

esp_bootloader_esp_idf::esp_app_desc!();

const FB_LEN: usize = (screens::W * screens::H * 2) as usize;
static mut FB: [u8; FB_LEN] = [0; FB_LEN];
/// The commit this image was built from (build.rs).
const COMMIT: &str = env!("DIYKEY_COMMIT");
/// Every line printed, kept, for a page that connects after they went by.
const LOG_LEN: usize = 16384;
static mut LOG: [u8; LOG_LEN] = [0; LOG_LEN];

/// The lines to the computer, kept as they go. When the buffer is full, the newest are only sent.
struct Log<'d> {
    out: Out<'d>,
    kept: &'static mut [u8],
    n: usize,
}

impl Write for Log<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        if let Some(room) = self.kept.get_mut(self.n..self.n + s.len()) {
            room.copy_from_slice(s.as_bytes());
            self.n += s.len();
        }
        self.out.write_str(s)
    }
}

impl Log<'_> {
    /// Every line kept, again, for the page: "log" from the computer.
    fn replay(&mut self) {
        // a line at a time: each write gets its own 200 ms
        for l in core::str::from_utf8(&self.kept[..self.n]).unwrap_or("").lines() {
            say!(self.out, "{l}");
        }
        say!(self.out, "= log end");
    }
}

fn key(k: (u8, u8)) -> Option<Key> {
    match k {
        ENTER => Some(Key::Enter),
        DEL => Some(Key::Del),
        (line, input) => keys::char_at(line, input).map(Key::Char),
    }
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let Board { mut panel, mut chip, mut keys, mut usb, out, delay, backlight: _backlight } = Board::new(esp_hal::init(config));

    // SAFETY: each the one reference to its buffer, taken once, before anything else could.
    let fb: &'static mut [u8] = unsafe { &mut *core::ptr::addr_of_mut!(FB) };
    let kept: &'static mut [u8] = unsafe { &mut *core::ptr::addr_of_mut!(LOG) };
    let mut out = Log { out, kept, n: 0 };
    let mut frame = Frame::new(fb, screens::W, screens::H);

    let mut commit = [0u8; 4];
    let _ = hex::decode(&COMMIT[..8], &mut commit);
    say!(out, "diykey-ceremony: commit {}", COMMIT);
    let mut c = Ceremony::new(commit);
    let mut dirty = true;
    let mut line = [0u8; 16];
    let mut n = 0;
    loop {
        let now = Instant::now().duration_since_epoch().as_millis() as u32;
        if dirty {
            c.draw(&mut frame, now);
            panel.show(frame.bytes());
        }
        // after the draw, so a step's Working page is on the screen while it runs
        dirty = c.input(keys.scan().and_then(key), now, &mut chip, &mut out);

        // The computer can only ask what this is, and for the log again: nothing it sends reaches
        // the chip.
        let mut rx = [0u8; 64];
        let got = usb.drain_rx_fifo(&mut rx);
        for &b in &rx[..got] {
            if b == b'\n' {
                match core::str::from_utf8(&line[..n]).map(str::trim) {
                    Ok("ping") => say!(out.out, "= pong diykey-ceremony"),
                    Ok("log") => out.replay(),
                    _ => {}
                }
                n = 0;
            } else if b != b'\r' && n < line.len() {
                line[n] = b;
                n += 1;
            }
        }
        if !dirty {
            delay.delay_millis(10);
        }
    }
}
