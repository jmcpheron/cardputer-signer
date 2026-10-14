//! The signer on the Cardputer v1.1. Milestone 2: a transaction arrives over USB as one line
//! (core/src/proto.rs), the device works out its Safe hash itself, and shows what it does page by
//! page; Del rejects. Nothing signs yet. The ATECC608 on the Grove port is only read.
//! Everything drawn comes from signer-core (`../core`); this file is the hardware. Pins are the
//! Stamp-S3A's (checked on the bench).
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
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::main;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::spi::Mode as SpiMode;
use esp_hal::time::{Duration, Instant, Rate};
use esp_hal::usb::usb_serial_jtag::{UsbSerialJtag, UsbSerialJtagTx};
use esp_hal::Blocking;
use signer_core::atecc;
use signer_core::gfx::Frame;
use signer_core::proto::{self, Cmd};
use signer_core::screens::{self, Buf, Chip, ChipInfo, Pending};

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

/// Lines to the computer over the USB cable. Waits up to 200 ms for it to take each write; after
/// one is missed, only 2 ms, so the device never stalls with nothing on the other end. (esp-println
/// gives up so soon that macOS lost bytes of a hash.)
struct Out<'d> {
    tx: UsbSerialJtagTx<'d, Blocking>,
    missed: bool,
}

impl Out<'_> {
    fn send(&mut self, s: &[u8]) -> bool {
        let wait = Duration::from_millis(if self.missed { 2 } else { 200 });
        let start = Instant::now();
        for &b in s {
            while self.tx.write_byte_nb(b).is_err() {
                if start.elapsed() > wait {
                    self.missed = true;
                    return false;
                }
            }
        }
        while self.tx.flush_tx_nb().is_err() {
            if start.elapsed() > wait {
                self.missed = true;
                return false;
            }
        }
        self.missed = false;
        true
    }
}

impl Write for Out<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.send(s.as_bytes());
        Ok(())
    }
}

/// One line to the computer: a reply when it starts with "= ", else a log line.
macro_rules! say {
    ($out:expr, $($t:tt)*) => {{
        let _ = writeln!($out, $($t)*);
    }};
}

/// The ST7789V2: 240x135 of glass at (40, 52) in the controller's RAM, landscape (MADCTL 0x60).
struct Panel<'d> {
    spi: Spi<'d, Blocking>,
    dc: Output<'d>,
    rst: Output<'d>,
    delay: Delay,
}

impl Panel<'_> {
    const X_OFF: u16 = 40;
    const Y_OFF: u16 = 52;

    fn cmd(&mut self, c: u8, data: &[u8]) {
        self.dc.set_low();
        self.spi.write(&[c]).unwrap();
        if !data.is_empty() {
            self.dc.set_high();
            self.spi.write(data).unwrap();
        }
    }

    /// As the MicroPython lcd.py (from picowallet) sets up this panel: its power-on defaults, 16-bit colour, inversion on.
    fn init(&mut self) {
        self.rst.set_high();
        self.delay.delay_millis(10);
        self.rst.set_low();
        self.delay.delay_millis(20);
        self.rst.set_high();
        self.delay.delay_millis(150);
        self.cmd(0x01, &[]); // software reset
        self.delay.delay_millis(150);
        self.cmd(0x11, &[]); // sleep out
        self.delay.delay_millis(120);
        self.cmd(0x3A, &[0x55]); // 16-bit colour
        self.cmd(0x36, &[0x60]); // MX|MV: landscape, the keyboard below
        self.cmd(0x21, &[]); // inversion on (IPS)
        self.cmd(0x13, &[]); // normal display mode
        self.cmd(0x29, &[]); // display on
        self.delay.delay_millis(20);
    }

    fn show(&mut self, frame: &[u8]) {
        let (x0, y0) = (Self::X_OFF, Self::Y_OFF);
        let (x1, y1) = (x0 + screens::W as u16 - 1, y0 + screens::H as u16 - 1);
        self.cmd(0x2A, &[(x0 >> 8) as u8, x0 as u8, (x1 >> 8) as u8, x1 as u8]);
        self.cmd(0x2B, &[(y0 >> 8) as u8, y0 as u8, (y1 >> 8) as u8, y1 as u8]);
        self.cmd(0x2C, &[]);
        self.dc.set_high();
        self.spi.write(frame).unwrap();
    }
}

/// The ATECC608 at 0x60, read only: the config zone, the revision, counter 0.
struct Atecc<'d> {
    i2c: I2c<'d, Blocking>,
    delay: Delay,
}

impl Atecc<'_> {
    fn wake(&mut self) -> Result<(), atecc::Error> {
        // Three rounds, as atecc.py: a chip still finishing a sleep can miss the first token.
        for _ in 0..3 {
            let _ = self.i2c.write(0x00u8, &[0x00]); // SDA low long enough to be the wake token
            self.delay.delay_millis(3);
            for _ in 0..3 {
                let mut r = [0u8; 4];
                if self.i2c.read(atecc::ADDR, &mut r).is_ok() && r == atecc::WAKE_OK {
                    return Ok(());
                }
                self.delay.delay_millis(2);
            }
            self.sleep();
            self.delay.delay_millis(10);
        }
        Err(atecc::Error::NoChip)
    }

    fn sleep(&mut self) {
        let _ = self.i2c.write(atecc::ADDR, &[0x01]);
    }

    fn command(&mut self, cmd: &atecc::Cmd, out: &mut [u8]) -> Result<usize, atecc::Error> {
        let mut pkt = [0u8; 8];
        let n = atecc::packet(cmd.opcode, cmd.p1, cmd.p2, &[], &mut pkt);
        self.i2c.write(atecc::ADDR, &pkt[..n]).map_err(|_| atecc::Error::NoChip)?;
        self.delay.delay_millis(cmd.wait_ms);
        let len = cmd.resp_len + 3;
        let mut resp = [0u8; 40];
        for _ in 0..100 {
            if self.i2c.read(atecc::ADDR, &mut resp[..len]).is_ok() {
                let p = atecc::response(&resp[..len])?;
                out[..p.len()].copy_from_slice(p);
                return Ok(p.len());
            }
            self.delay.delay_millis(3);
        }
        Err(atecc::Error::NoChip)
    }

    fn info(&mut self) -> Result<ChipInfo, atecc::Error> {
        self.wake()?;
        let r = self.read_info();
        self.sleep();
        r
    }

    fn read_info(&mut self) -> Result<ChipInfo, atecc::Error> {
        let (mut b0, mut b2, mut rev, mut ctr) = ([0u8; 32], [0u8; 32], [0u8; 4], [0u8; 4]);
        self.command(&atecc::read_config(0), &mut b0)?;
        self.command(&atecc::read_config(2), &mut b2)?;
        self.command(&atecc::REVISION, &mut rev)?;
        self.command(&atecc::read_counter(0), &mut ctr)?;
        let (config_locked, data_locked) = atecc::locks(&b2);
        Ok(ChipInfo { serial: atecc::serial(&b0), revision: rev, config_locked, data_locked, counter0: u32::from_le_bytes(ctr) })
    }
}

/// The keyboard: a 74HC138 picks one of 8 lines (3 select pins), 7 inputs read that line's keys,
/// low while down. A key is (line, input).
struct Keys<'d> {
    sel: [Output<'d>; 3],
    cols: [Input<'d>; 7],
    delay: Delay,
}

impl Keys<'_> {
    /// The first key down, if any.
    fn scan(&mut self) -> Option<(u8, u8)> {
        let mut found = None;
        for line in 0..8u8 {
            for (i, p) in self.sel.iter_mut().enumerate() {
                p.set_level(if line >> i & 1 == 1 { Level::High } else { Level::Low });
            }
            self.delay.delay_micros(30);
            for (col, p) in self.cols.iter().enumerate() {
                if found.is_none() && p.is_low() {
                    found = Some((line, col as u8));
                }
            }
        }
        found
    }
}

// The keys: ; . up and down, , / left and right, Enter, Del.
const UP: (u8, u8) = (1, 5);
const DOWN: (u8, u8) = (0, 5);
const LEFT: (u8, u8) = (4, 5);
const RIGHT: (u8, u8) = (4, 6);
const ENTER: (u8, u8) = (1, 6);
const DEL: (u8, u8) = (3, 6);

fn read_chip(chip: &mut Atecc, out: &mut Out) -> Chip {
    match chip.info() {
        Ok(c) => {
            say!(out, "chip: serial {:02x?} revision {:02x?} config locked {} data locked {} counter0 {}",
                     c.serial, c.revision, c.config_locked, c.data_locked, c.counter0);
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
    let p = esp_hal::init(config);
    let delay = Delay::new();
    let out = OutputConfig::default();
    let pull_up = InputConfig::default().with_pull(Pull::Up);

    let _backlight = Output::new(p.GPIO38, Level::High, out);
    let spi = Spi::new(p.SPI2, SpiConfig::default().with_frequency(Rate::from_mhz(40)).with_mode(SpiMode::_0))
        .unwrap()
        .with_sck(p.GPIO36)
        .with_mosi(p.GPIO35)
        .with_cs(p.GPIO37);
    let mut panel = Panel { spi, dc: Output::new(p.GPIO34, Level::High, out), rst: Output::new(p.GPIO33, Level::High, out), delay };
    panel.init();

    let i2c = I2c::new(p.I2C0, I2cConfig::default().with_frequency(Rate::from_khz(100)))
        .unwrap()
        .with_sda(p.GPIO2)
        .with_scl(p.GPIO1);
    let mut chip = Atecc { i2c, delay };

    let mut keys = Keys {
        sel: [Output::new(p.GPIO8, Level::Low, out), Output::new(p.GPIO9, Level::Low, out), Output::new(p.GPIO11, Level::Low, out)],
        cols: [
            Input::new(p.GPIO13, pull_up),
            Input::new(p.GPIO15, pull_up),
            Input::new(p.GPIO3, pull_up),
            Input::new(p.GPIO4, pull_up),
            Input::new(p.GPIO5, pull_up),
            Input::new(p.GPIO6, pull_up),
            Input::new(p.GPIO7, pull_up),
        ],
        delay,
    };

    // The computer's side of the USB cable: lines in, and lines out through Out.
    let (mut usb, tx) = UsbSerialJtag::new(p.USB_DEVICE).split();
    let mut out = Out { tx, missed: false };

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
