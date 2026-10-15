//! The Cardputer v1.1's hardware, shared by the signer (src/bin/main.rs) and the key ceremony
//! (src/bin/ceremony.rs): the screen, the keyboard, the ATECC608 on the Grove port and the USB
//! cable. Pins are the Stamp-S3A's (checked on the bench). What each image does with the chip is in
//! signer-core: `atecc` reads, and only the ceremony image uses `ceremony`.
#![no_std]

use core::fmt::Write;

use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::peripherals::Peripherals;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::spi::Mode as SpiMode;
use esp_hal::time::{Duration, Instant, Rate};
use esp_hal::usb::usb_serial_jtag::{UsbSerialJtag, UsbSerialJtagRx, UsbSerialJtagTx};
use esp_hal::Blocking;
use signer_core::atecc::{self, Link};
use signer_core::screens::{self, ChipInfo};

/// Lines to the computer over the USB cable. Waits up to 200 ms for it to take each write; after
/// one is missed, only 2 ms, so the device never stalls with nothing on the other end. (esp-println
/// gives up so soon that macOS lost bytes of a hash.)
pub struct Out<'d> {
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
#[macro_export]
macro_rules! say {
    ($out:expr, $($t:tt)*) => {{
        let _ = writeln!($out, $($t)*);
    }};
}

/// The ST7789V2: 240x135 of glass at (40, 52) in the controller's RAM, landscape (MADCTL 0x60).
pub struct Panel<'d> {
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

    pub fn show(&mut self, frame: &[u8]) {
        let (x0, y0) = (Self::X_OFF, Self::Y_OFF);
        let (x1, y1) = (x0 + screens::W as u16 - 1, y0 + screens::H as u16 - 1);
        self.cmd(0x2A, &[(x0 >> 8) as u8, x0 as u8, (x1 >> 8) as u8, x1 as u8]);
        self.cmd(0x2B, &[(y0 >> 8) as u8, y0 as u8, (y1 >> 8) as u8, y1 as u8]);
        self.cmd(0x2C, &[]);
        self.dc.set_high();
        self.spi.write(frame).unwrap();
    }
}

/// The ATECC608 at 0x60, over I2C. Sends the packets signer-core builds; which commands, the image
/// decides.
pub struct Atecc<'d> {
    i2c: I2c<'d, Blocking>,
    delay: Delay,
}

impl Link for Atecc<'_> {
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

    /// The chip doesn't answer while it works: read again every 3 ms, up to 300 ms past `wait_ms`.
    fn exchange(&mut self, pkt: &[u8], wait_ms: u32, resp: &mut [u8]) -> Result<(), atecc::Error> {
        self.i2c.write(atecc::ADDR, pkt).map_err(|_| atecc::Error::NoChip)?;
        self.delay.delay_millis(wait_ms);
        for _ in 0..100 {
            if self.i2c.read(atecc::ADDR, resp).is_ok() {
                return Ok(());
            }
            self.delay.delay_millis(3);
        }
        Err(atecc::Error::NoChip)
    }
}

impl Atecc<'_> {
    /// What the signer shows of the chip, read only: the config zone, the revision, counter 0, and
    /// slot 0's public key if it has one.
    pub fn info(&mut self) -> Result<ChipInfo, atecc::Error> {
        self.wake()?;
        let r = self.read_info();
        self.sleep();
        r
    }

    fn read_info(&mut self) -> Result<ChipInfo, atecc::Error> {
        let (mut b0, mut b2, mut rev, mut ctr) = ([0u8; 32], [0u8; 32], [0u8; 4], [0u8; 4]);
        atecc::run(self, &atecc::read_config(0), &[], &mut b0)?;
        atecc::run(self, &atecc::read_config(2), &[], &mut b2)?;
        atecc::run(self, &atecc::REVISION, &[], &mut rev)?;
        atecc::run(self, &atecc::read_counter(0), &[], &mut ctr)?;
        // slot 0's public key, if it has one: GenKey mode 0 only computes it (atecc::public_key)
        let mut pk = [0u8; 64];
        let pubkey0 = atecc::run(self, &atecc::public_key(0), &[], &mut pk).ok().filter(|&n| n == 64).map(|_| pk);
        let (config_locked, data_locked) = atecc::locks(&b2);
        Ok(ChipInfo { serial: atecc::serial(&b0), revision: rev, config_locked, data_locked, counter0: u32::from_le_bytes(ctr), pubkey0 })
    }
}

/// The keyboard: a 74HC138 picks one of 8 lines (3 select pins), 7 inputs read that line's keys,
/// low while down. A key is (line, input); `signer_core::keys` says which is which.
pub struct Keys<'d> {
    sel: [Output<'d>; 3],
    cols: [Input<'d>; 7],
    delay: Delay,
}

impl Keys<'_> {
    /// The first key down, if any.
    pub fn scan(&mut self) -> Option<(u8, u8)> {
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
pub const UP: (u8, u8) = (1, 5);
pub const DOWN: (u8, u8) = (0, 5);
pub const LEFT: (u8, u8) = (4, 5);
pub const RIGHT: (u8, u8) = (4, 6);
pub const ENTER: (u8, u8) = (1, 6);
pub const DEL: (u8, u8) = (3, 6);

/// Everything on the board, set up.
pub struct Board<'d> {
    /// On while the board is kept.
    pub backlight: Output<'d>,
    pub panel: Panel<'d>,
    pub chip: Atecc<'d>,
    pub keys: Keys<'d>,
    /// The computer's side of the USB cable: lines in here, and out through `out`.
    pub usb: UsbSerialJtagRx<'d, Blocking>,
    pub out: Out<'d>,
    pub delay: Delay,
}

impl Board<'static> {
    pub fn new(p: Peripherals) -> Self {
        let delay = Delay::new();
        let out = OutputConfig::default();
        let pull_up = InputConfig::default().with_pull(Pull::Up);

        let backlight = Output::new(p.GPIO38, Level::High, out);
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
        let chip = Atecc { i2c, delay };

        let keys = Keys {
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

        let (usb, tx) = UsbSerialJtag::new(p.USB_DEVICE).split();
        Board { backlight, panel, chip, keys, usb, out: Out { tx, missed: false }, delay }
    }
}
