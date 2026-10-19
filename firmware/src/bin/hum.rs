//! A tune as a second factor, on the Cardputer v1.1 (docs/HUM.md): its own image, an experiment.
//! It plays the start of a common tune on the speaker, you hum or whistle your own ending into the
//! mic, and it shows what it heard. Set a tune with three goes that come out the same, then try
//! it. It never wakes the chip and signs nothing, and the tune lives in RAM until power-off.
//!
//! The pages and the listening are signer-core's (`hum_pages`, `hum`), the same code `cargo test`
//! runs; this file plays and records. The speaker (an NS4168 on G41, G42, G43) and the mic (an
//! SPM1423, PDM, on G43 and G46) share G43: the speaker's LRCLK is the mic's clock. So it plays,
//! then listens, never both, and each gets its own I2S unit while it runs: I2S1 for the speaker,
//! I2S0 for the mic, the one with the S3's PDM-to-PCM filter. The log over USB says how each go
//! went (notes, slot, levels, time), never what was hummed.
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
use esp_hal::dma::{DmaRxStreamBuf, DmaTxStreamBuf};
use esp_hal::i2s::master::{Channels, DataFormat, I2s, PdmConfig, PdmRxConfig, PdmSlotMode, TdmConfig};
use esp_hal::peripherals::{DMA_CH0, DMA_CH1, GPIO41, GPIO42, GPIO43, GPIO46, I2S0, I2S1};
use esp_hal::time::{Instant, Rate};
use esp_hal::{dma_rx_stream_buffer, dma_tx_stream_buffer, main};
use firmware::{say, Board, Keys, Out, Panel, DEL, DOWN, ENTER, LEFT, RIGHT, UP};
use signer_core::gfx::Frame;
use signer_core::hum::{self, Mic, Tones, OPENINGS};
use signer_core::hum_pages::{Hum, Key, Page};
use signer_core::screens;

esp_bootloader_esp_idf::esp_app_desc!();

const FB_LEN: usize = (screens::W * screens::H * 2) as usize;
static mut FB: [u8; FB_LEN] = [0; FB_LEN];
static mut HUM: Hum = Hum::new();
static mut POPPED: [u8; 512] = [0; 512];
/// The commit this image was built from (build.rs).
const COMMIT: &str = env!("DIYKEY_COMMIT");

/// The speaker's buffer: 125 ms of 16 kHz stereo. The mic's: 125 ms of 32 kHz stereo.
const TX_BYTES: usize = 8192;
const RX_BYTES: usize = 16384;
const CHUNK: usize = 1024;
/// Silence after the opening, longer than the speaker's buffer, so stopping cuts only silence.
const TAIL: usize = hum::RATE as usize / 5;
/// While playing or listening, the keys are read this often (ms), and the page drawn again this
/// often.
const KEYS_MS: u64 = 20;
const DRAW_MS: u64 = 100;
/// No bytes from the mic for this long (ms), and it has stopped.
const STALL_MS: u64 = 500;

/// The speaker and the mic: their I2S units, DMA channels and pins, and a DMA buffer each.
struct Audio {
    i2s0: I2S0<'static>,
    i2s1: I2S1<'static>,
    dma0: DMA_CH0<'static>,
    dma1: DMA_CH1<'static>,
    bclk: GPIO41<'static>,
    dout: GPIO42<'static>,
    lrclk: GPIO43<'static>,
    din: GPIO46<'static>,
    tx: Option<DmaTxStreamBuf>,
    rx: Option<DmaRxStreamBuf>,
}

fn key(k: (u8, u8)) -> Option<Key> {
    match k {
        ENTER => Some(Key::Enter),
        DEL => Some(Key::Del),
        UP => Some(Key::Up),
        DOWN => Some(Key::Down),
        LEFT => Some(Key::Left),
        RIGHT => Some(Key::Right),
        _ => None,
    }
}

fn ms() -> u64 {
    Instant::now().duration_since_epoch().as_millis()
}

/// Stereo 16-bit frames of the tones into `b`, the same sample left and right; the bytes written.
fn fill(b: &mut [u8], tones: &mut impl Iterator<Item = i16>) -> usize {
    let mut n = 0;
    for fr in b.chunks_exact_mut(4) {
        let Some(s) = tones.next() else { break };
        fr[..2].copy_from_slice(&s.to_le_bytes());
        fr[2..].copy_from_slice(&s.to_le_bytes());
        n += 4;
    }
    n
}

/// The opening, on the speaker. Del stops it.
#[allow(clippy::large_stack_frames, reason = "the I2S driver and its DMA transfer live here while it plays")]
fn play(a: &mut Audio, hum: &mut Hum, keys: &mut Keys, out: &mut Out) {
    let notes = hum.opening();
    let name = OPENINGS.iter().find(|o| o.notes == notes).map_or("?", |o| o.name);
    say!(out, "hum: playing {name}");
    let config = TdmConfig::new_tdm_philips()
        .with_sample_rate(Rate::from_hz(hum::RATE))
        .with_data_format(DataFormat::Data16Channel16)
        .with_channels(Channels::STEREO);
    let Ok(i2s) = I2s::new(a.i2s1.reborrow(), a.dma1.reborrow(), config) else {
        say!(out, "hum: the speaker's I2S wouldn't start");
        return;
    };
    let tx = i2s.i2s_tx.with_bclk(a.bclk.reborrow()).with_ws(a.lrclk.reborrow()).with_dout(a.dout.reborrow()).build();
    let Some((d, b)) = a.tx.take().map(DmaTxStreamBuf::split) else { return };
    // a fresh buffer each time: one that has run keeps where it got to
    let mut buf = DmaTxStreamBuf::new(d, b).unwrap();
    let mut tones = Tones::new(notes).chain(core::iter::repeat_n(0, TAIL));
    buf.push_with(|b| fill(b, &mut tones));
    let mut t = match tx.write(buf) {
        Ok(t) => t,
        Err((e, _, buf)) => {
            say!(out, "hum: the speaker wouldn't start: {e:?}");
            a.tx = Some(buf);
            return;
        }
    };
    let mut read = ms();
    loop {
        if t.push_with(|b| fill(b, &mut tones)) == 0 && t.available_bytes() > 0 {
            break; // all of it, tail and all, is in
        }
        if ms() - read >= KEYS_MS {
            read = ms();
            hum.input(keys.scan().and_then(key), read as u32);
            if !matches!(hum.page, Page::Play(_)) {
                break; // Del
            }
        }
    }
    let (_, buf) = t.stop();
    a.tx = Some(buf);
}

/// The mic, into the pages, until they stop listening. The page is drawn again every 100 ms.
#[allow(clippy::large_stack_frames, reason = "the I2S driver and its DMA transfer live here while it listens")]
fn listen(a: &mut Audio, hum: &mut Hum, keys: &mut Keys, panel: &mut Panel, frame: &mut Frame, out: &mut Out) {
    let config = PdmConfig::rx_only(PdmRxConfig::new_pcm_default(Rate::from_hz(Mic::RATE), PdmSlotMode::Stereo));
    let Ok(i2s) = I2s::new_pdm(a.i2s0.reborrow(), a.dma0.reborrow(), config) else {
        say!(out, "hum: the mic's I2S wouldn't start");
        hum.stop();
        return;
    };
    let rx = i2s.i2s_rx.with_clk(a.lrclk.reborrow()).with_din(a.din.reborrow()).build();
    let Some(buf) = a.rx.take() else { return };
    let mut t = match rx.read(buf) {
        Ok(t) => t,
        Err((e, _, buf)) => {
            say!(out, "hum: the mic wouldn't start: {e:?}");
            a.rx = Some(buf);
            hum.stop();
            return;
        }
    };
    let (start, go) = (ms(), hum.page != Page::Mic);
    let (mut read, mut drawn, mut data, mut busy) = (start, start, start, 0u64);
    // SAFETY: only this function uses it, and it doesn't call itself.
    let chunk = unsafe { &mut *core::ptr::addr_of_mut!(POPPED) };
    while hum.listening() {
        let n = t.available_bytes().min(chunk.len()) & !3;
        if n > 0 {
            let got = t.pop(&mut chunk[..n]);
            let t0 = ms();
            hum.feed(&chunk[..got]);
            busy += ms() - t0;
            data = ms();
        } else if ms() - data > STALL_MS {
            say!(out, "hum: no sound from the mic for {STALL_MS} ms; stopped");
            hum.stop();
            break;
        }
        if ms() - read >= KEYS_MS {
            read = ms();
            hum.input(keys.scan().and_then(key), read as u32);
        }
        if ms() - drawn >= DRAW_MS {
            drawn = ms();
            hum.draw(frame, drawn as u32);
            panel.show(frame.bytes());
        }
    }
    let (_, buf) = t.stop();
    a.rx = Some(buf);
    let (slot, level) = hum.mic();
    let _ = write!(out, "hum: listened {} ms, analysis {busy} ms; slot {}, levels {:.0} and {:.0}",
                   ms() - start, slot.map_or(0, |s| s + 1), level[0], level[1]);
    if go {
        let (notes, frames) = hum.heard();
        let _ = write!(out, "; {notes} notes in {frames} frames");
    }
    say!(out, "");
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let Board { mut panel, chip: _, mut keys, usb: _, mut out, delay, backlight: _backlight } = Board::new(esp_hal::init(config));

    // SAFETY: Board::new took everything else; nothing there uses these, and each is taken once.
    let mut audio = unsafe {
        Audio {
            i2s0: I2S0::steal(),
            i2s1: I2S1::steal(),
            dma0: DMA_CH0::steal(),
            dma1: DMA_CH1::steal(),
            bclk: GPIO41::steal(),
            dout: GPIO42::steal(),
            lrclk: GPIO43::steal(),
            din: GPIO46::steal(),
            tx: Some(dma_tx_stream_buffer!(TX_BYTES, CHUNK)),
            rx: Some(dma_rx_stream_buffer!(RX_BYTES, CHUNK)),
        }
    };
    // SAFETY: each the one reference to its buffer, taken once, before anything else could.
    let fb: &'static mut [u8] = unsafe { &mut *core::ptr::addr_of_mut!(FB) };
    let hum: &'static mut Hum = unsafe { &mut *core::ptr::addr_of_mut!(HUM) };
    let mut frame = Frame::new(fb, screens::W, screens::H);

    say!(out, "diykey-hum: commit {}", COMMIT);
    let mut dirty = true;
    loop {
        let now = ms() as u32;
        if dirty {
            hum.draw(&mut frame, now);
            panel.show(frame.bytes());
        }
        let before = hum.page;
        if matches!(hum.page, Page::Play(_)) {
            play(&mut audio, hum, &mut keys, &mut out);
            hum.played();
            dirty = true;
            continue;
        }
        if hum.listening() {
            listen(&mut audio, hum, &mut keys, &mut panel, &mut frame, &mut out);
            dirty = true;
            match hum.page {
                Page::Set => say!(out, "hum: set"),
                Page::Differ => say!(out, "hum: the three goes differ"),
                Page::Match(true) => say!(out, "hum: a match"),
                Page::Match(false) => say!(out, "hum: no match"),
                _ => {}
            }
            continue;
        }
        dirty = hum.input(keys.scan().and_then(key), now) || hum.page != before;
        if !dirty {
            delay.delay_millis(10);
        }
    }
}
