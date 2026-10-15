//! The signer on the Cardputer v1.1. Milestone 2: a transaction arrives over USB as one line
//! (core/src/proto.rs), the device works out its Safe hash itself, and shows what it does page by
//! page; Del rejects. Nothing signs yet. The ATECC608 on the Grove port is only read.
//! Everything drawn comes from signer-core (`../core`); the hardware is in src/lib.rs.
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
use firmware::{say, Atecc, Board, Out, DEL, DOWN, ENTER, LEFT, RIGHT, UP};
use signer_core::atecc;
use signer_core::gfx::Frame;
use signer_core::proto::{self, Cmd};
use signer_core::screens::{self, Buf, Chip, Pending};

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

const FB_LEN: usize = (screens::W * screens::H * 2) as usize;
static mut FB: [u8; FB_LEN] = [0; FB_LEN];
/// One line from the computer: up to 2 KB of calldata as hex, and the other fields.
const LINE_LEN: usize = 5120;
static mut LINE: [u8; LINE_LEN] = [0; LINE_LEN];
const DATA_LEN: usize = 2048;
static mut DATA: [u8; DATA_LEN] = [0; DATA_LEN];

fn read_chip(chip: &mut Atecc, out: &mut Out) -> Chip {
    match chip.info() {
        Ok(c) => {
            say!(out, "chip: serial {:02x?} revision {:02x?} config locked {} data locked {} counter0 {}",
                     c.serial, c.revision, c.config_locked, c.data_locked, c.counter0);
            match &c.pubkey0 {
                Some(pk) => {
                    let mut h = [0u8; 128];
                    say!(out, "chip: pubkey0 0x{}", signer_core::hex::encode(pk, &mut h));
                }
                None => say!(out, "chip: pubkey0 none"),
            }
            Chip::Found(c)
        }
        Err(atecc::Error::NoChip) => {
            say!(out, "chip: no answer at 0x60");
            Chip::Missing
        }
        Err(e) => {
            say!(out, "chip: {:?}", e);
            Chip::Error(e)
        }
    }
}

enum Screen {
    Idle,
    Review(usize),
    Rejected,
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let Board { mut panel, mut chip, mut keys, mut usb, mut out, delay, backlight: _backlight } = Board::new(esp_hal::init(config));

    // SAFETY: each the one reference to its buffer, taken once, before anything else could.
    let fb: &'static mut [u8] = unsafe { &mut *core::ptr::addr_of_mut!(FB) };
    let line: &'static mut [u8] = unsafe { &mut *core::ptr::addr_of_mut!(LINE) };
    let data: &'static mut [u8] = unsafe { &mut *core::ptr::addr_of_mut!(DATA) };
    let mut frame = Frame::new(fb, screens::W, screens::H);

    say!(out, "cardputer-signer: milestone 2");
    let mut state = read_chip(&mut chip, &mut out);
    let mut mode = Screen::Idle;
    let mut pending: Option<Pending> = None;
    let mut last: Buf<96> = Buf::new();
    let (mut n, mut overflow) = (0usize, false);
    let mut held = None;
    let mut dirty = true;
    loop {
        if dirty {
            match (&mode, &pending) {
                (Screen::Review(page), Some(p)) => screens::review(&mut frame, p, data, *page),
                (Screen::Rejected, Some(p)) => screens::rejected(&mut frame, p),
                _ => screens::idle(&mut frame, &state, if last.is_empty() { None } else { Some(last.as_str()) }),
            }
            panel.show(frame.bytes());
            dirty = false;
        }

        // Lines from the computer.
        let mut rx = [0u8; 64];
        let got = usb.drain_rx_fifo(&mut rx);
        for &b in &rx[..got] {
            if b == b'\r' {
                continue;
            }
            if b != b'\n' {
                if n < line.len() {
                    line[n] = b;
                    n += 1;
                } else {
                    overflow = true;
                }
                continue;
            }
            let text = core::str::from_utf8(&line[..n]).unwrap_or("");
            if overflow {
                say!(out, "= error line too long");
            } else if text.trim() == "ping" {
                say!(out, "= pong cardputer-signer m2");
            } else if text.trim().is_empty() {
            } else if text.split_ascii_whitespace().next() != Some("tx") {
                say!(out, "= error unknown command");
            } else if matches!(mode, Screen::Review(_)) {
                // never swap the transaction under someone's eyes
                say!(out, "= busy");
            } else {
                match proto::parse(text, data) {
                    Ok(Cmd::Tx(t)) => match Pending::new(t, data) {
                        Ok(pd) => {
                            let mut h = [0u8; 64];
                            say!(out, "= hash 0x{}", signer_core::hex::encode(&pd.hash, &mut h));
                            if pd.matches() == Some(false) {
                                say!(out, "= mismatch");
                            }
                            say!(out, "tx: {} ({} pages)", pd.review.summary(), pd.pages());
                            pending = Some(pd);
                            mode = Screen::Review(0);
                            dirty = true;
                        }
                        Err(e) => say!(out, "= error {:?}", e),
                    },
                    Ok(Cmd::Ping) => say!(out, "= pong cardputer-signer m2"),
                    Err(e) => say!(out, "= error {}", e),
                }
            }
            n = 0;
            overflow = false;
        }

        // Keys.
        let k = keys.scan();
        if k != held {
            if let Some(kk) = k {
                match (&mode, &pending) {
                    (Screen::Review(page), Some(p)) => {
                        let page = *page;
                        if kk == DEL {
                            say!(out, "= rejected");
                            mode = Screen::Rejected;
                        } else if (kk == ENTER || kk == RIGHT || kk == DOWN) && page + 1 < p.pages() {
                            mode = Screen::Review(page + 1);
                        } else if (kk == LEFT || kk == UP) && page > 0 {
                            mode = Screen::Review(page - 1);
                        }
                    }
                    (Screen::Rejected, Some(p)) => {
                        if kk == ENTER || kk == DEL {
                            last = Buf::new();
                            let mut d = [0u8; 78];
                            let len = signer_core::num::dec(&p.fields.nonce, &mut d);
                            let _ = write!(last, "Rejected #{}: {}", core::str::from_utf8(&d[..len]).unwrap_or("?"), p.review.summary());
                            mode = Screen::Idle;
                            pending = None;
                        }
                    }
                    _ => {
                        if kk == ENTER {
                            state = read_chip(&mut chip, &mut out);
                        }
                    }
                }
                dirty = true;
            }
            held = k;
        }
        if got == 0 {
            delay.delay_millis(15);
        }
    }
}
