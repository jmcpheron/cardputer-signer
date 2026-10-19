//! A hummed or whistled tune, turned into a code that comes out the same each time: the hum image's
//! sound side (docs/HUM.md). An experiment: nothing here signs, and nothing here is kept.
//!
//! The mic gives 16-bit samples. `Mic` picks the PDM slot that carries the mic and brings it to
//! 16 kHz; `Listener` finds the pitch every 10 ms (YIN); `Listener::notes` cuts the pitch track
//! into notes; `Code::of` keeps only what a person hums the same way every time: up, down or the
//! same, a small step or a leap, and which notes are long. Not the key, the speed or the voice.
//! `Tones` plays the openings on the speaker.

/// Samples a second, into the listener and out to the speaker.
pub const RATE: u32 = 16_000;
/// One frame of the pitch track: 10 ms.
pub const HOP: usize = 160;
/// YIN's window, 20 ms, and the periods it looks for: 4 to 213 samples, 4 kHz down to 75 Hz.
const WIN: usize = 320;
const TAU_MIN: usize = 4;
const TAU_MAX: usize = 213;
const BUF: usize = WIN + TAU_MAX + 1;
/// YIN's dip: the first period this periodic.
const YIN_DIP: f32 = 0.15;
/// A frame is voiced when this periodic, and this much louder than the quietest stretch so far.
const APER_MAX: f32 = 0.3;
const LOUDER: f32 = 10.0;
/// Below this (power in LSB², RMS 4) it is silence even in a silent room: a dead slot reads 0.
/// Low, because how loud the mic reads is still to be seen on the bench.
const ABS_MIN: f32 = 16.0;

/// The pitch track holds 15 s; a tune is longer than that only by mistake.
pub const MAX_FRAMES: usize = 1500;
/// Listening ends 1.5 s after the last voiced frame, once 0.3 s were voiced.
const END_QUIET: usize = 150;
const MIN_VOICED: usize = 30;

/// Gaps up to 20 ms inside a note are dropouts; 30 ms or more ends it.
const GAP: usize = 2;
/// A new note starts where the pitch leaves the note by 0.4 semitones: the note's pitch is the
/// median of its last 240 ms, and the next 80 ms (their median) must have left it too. A cut that
/// comes out under half a semitone is undone (MERGE). This is on the pitch track's median over
/// 210 ms, which keeps the steps between notes and flattens a vibrato.
const SPLIT: f32 = 0.4;
const HOLD: usize = 8;
const RECENT: usize = 24;
const SLOW: usize = 10;
/// Shorter than 70 ms, a note is a glide into the next one (or a blip, dropped). Shorter than
/// 120 ms, joined to the notes either side and between them in pitch, it is the middle of a glide.
const MIN_NOTE: usize = 7;
const GLIDE: usize = 12;
/// Two notes with no gap and less than half a semitone apart are one note.
const MERGE: f32 = 0.5;
/// Room for the notes as first cut, glides and blips among them, before they are tidied.
const CUTS: usize = 4 * MAX_NOTES;

/// Intervals: the same under half a semitone, a step under 2.5 (a tone or less), a leap from there.
const SAME: f32 = 0.5;
const LEAP: f32 = 2.5;
/// Long notes: the lengths split in two where they separate best (Otsu's method, on their logs),
/// if the long ones are on average 1.5 times the short ones and at least two are short. Else all
/// are short.
const LONG: f32 = 0.585; // log2(1.5)

/// A tune has 6 to 24 notes.
pub const MIN_NOTES: usize = 6;
pub const MAX_NOTES: usize = 24;

// ----------------------------------------------------------------------------- the code
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Move {
    Same,
    Up,
    UpLeap,
    Down,
    DownLeap,
}

impl Move {
    fn of(semitones: f32) -> Move {
        let a = semitones.abs();
        match (a < SAME, a < LEAP, semitones > 0.0) {
            (true, _, _) => Move::Same,
            (false, true, true) => Move::Up,
            (false, true, false) => Move::Down,
            (false, false, true) => Move::UpLeap,
            (false, false, false) => Move::DownLeap,
        }
    }
}

/// A note of the tune: frames [start, end) of the pitch track, at `pitch` semitones (MIDI's
/// numbering: 69 is A4, 440 Hz).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Note {
    pub start: u16,
    pub end: u16,
    pub pitch: f32,
}

/// What a tune comes down to, note by note: how each note moves from the one before, and whether
/// the one before was long. A tune of n notes has n - 1 of them. The last note's length isn't in
/// it: people hold the last note as long as they like.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Code {
    n: u8,
    steps: [(Move, bool); MAX_NOTES - 1],
}

impl Default for Code {
    fn default() -> Self {
        Code::EMPTY
    }
}

impl Code {
    pub const EMPTY: Code = Code { n: 0, steps: [(Move::Same, false); MAX_NOTES - 1] };

    /// The notes' code, or None for more than MAX_NOTES notes.
    pub fn of(notes: &[Note]) -> Option<Code> {
        if notes.len() > MAX_NOTES {
            return None;
        }
        let mut c = Code::EMPTY;
        if notes.len() < 2 {
            return Some(c);
        }
        // the inter-onset intervals, in frames: a rest belongs to the note before it
        let mut ioi = [0u16; MAX_NOTES - 1];
        for (i, w) in notes.windows(2).enumerate() {
            ioi[i] = w[1].start - w[0].start;
        }
        let k = notes.len() - 1;
        let long_from = long_from(&ioi[..k]);
        for (i, w) in notes.windows(2).enumerate() {
            c.steps[i] = (Move::of(w[1].pitch - w[0].pitch), ioi[i] >= long_from);
        }
        c.n = k as u8;
        Some(c)
    }

    /// How many notes: one more than the steps.
    pub fn notes(&self) -> usize {
        if self.n == 0 { 0 } else { self.n as usize + 1 }
    }

    /// Step i: how note i + 1 moves from note i, and whether note i was long.
    pub fn steps(&self) -> &[(Move, bool)] {
        &self.steps[..self.n as usize]
    }
}

/// The shortest length that is long, in frames (u16::MAX for none): see LONG.
fn long_from(ioi: &[u16]) -> u16 {
    let mut sorted = [(0u16, 0f32); MAX_NOTES - 1];
    let k = ioi.len();
    for (s, &v) in sorted.iter_mut().zip(ioi) {
        *s = (v, log2(v.max(1) as f32));
    }
    let sorted = &mut sorted[..k];
    sorted.sort_unstable_by_key(|s| s.0);
    let total: f32 = sorted.iter().map(|s| s.1).sum();
    let (mut best, mut at, mut sum) = (0.0, None, 0.0);
    for i in 1..k {
        sum += sorted[i - 1].1;
        if i < 2 {
            continue;
        }
        let (m1, m2) = (sum / i as f32, (total - sum) / (k - i) as f32);
        let between = i as f32 * (k - i) as f32 * (m2 - m1) * (m2 - m1);
        if between > best && m2 - m1 > LONG {
            (best, at) = (between, Some(sorted[i].0));
        }
    }
    at.unwrap_or(u16::MAX)
}

// ----------------------------------------------------------------------------- the mic
/// The PDM mic, through the S3's PDM-to-PCM filter: stereo frames of two 16-bit slots at 32 kHz,
/// only one of which carries the mic. The first 200 ms are dropped while the mic wakes; the next
/// 100 ms pick the slot (the first, unless it is far quieter than the second); then each pair of
/// samples becomes one at 16 kHz.
pub struct Mic {
    /// Chosen by hand, or picked once the 100 ms are in.
    pub slot: Option<usize>,
    seen: u32,
    /// Mean and squared deviations per slot (Welford), over the 100 ms.
    mean: [f32; 2],
    m2: [f32; 2],
    count: u32,
    /// The level of each slot lately, as a power, for the mic page, around its DC.
    pub level: [f32; 2],
    dc: [f32; 2],
    half: Option<f32>,
}

impl Mic {
    /// Stereo frames a second.
    pub const RATE: u32 = 2 * RATE;
    const SETTLE: u32 = Mic::RATE / 5;
    const MEASURE: u32 = Mic::RATE / 10;

    /// A slot chosen by hand, or None to pick it.
    pub const fn new(slot: Option<usize>) -> Self {
        Mic { slot, seen: 0, mean: [0.0; 2], m2: [0.0; 2], count: 0, level: [0.0; 2], dc: [0.0; 2], half: None }
    }

    /// Bytes from the DMA: little-endian i16, the two slots interleaved. Whole frames only (4 bytes).
    pub fn feed(&mut self, bytes: &[u8], out: &mut Listener) {
        for fr in bytes.chunks_exact(4) {
            let s = [i16::from_le_bytes([fr[0], fr[1]]) as f32, i16::from_le_bytes([fr[2], fr[3]]) as f32];
            self.seen = self.seen.saturating_add(1);
            for (k, &x) in s.iter().enumerate() {
                if self.seen == 1 {
                    self.dc[k] = x;
                }
                self.dc[k] += (x - self.dc[k]) * (1.0 / 1024.0);
                let d = x - self.dc[k];
                self.level[k] += (d * d - self.level[k]) * (1.0 / 2048.0);
            }
            if self.seen <= Mic::SETTLE {
                continue;
            }
            if self.slot.is_none() {
                self.count += 1;
                for (k, &x) in s.iter().enumerate() {
                    let d = x - self.mean[k];
                    self.mean[k] += d / self.count as f32;
                    self.m2[k] += d * (x - self.mean[k]);
                }
                if self.seen == Mic::SETTLE + Mic::MEASURE {
                    self.slot = Some(if self.m2[0] * 16.0 < self.m2[1] { 1 } else { 0 });
                }
                continue;
            }
            let x = s[self.slot.unwrap_or(0)];
            match self.half.take() {
                None => self.half = Some(x),
                Some(h) => out.feed((h + x) * 0.5),
            }
        }
    }
}

// ----------------------------------------------------------------------------- the pitch track
/// One 10 ms frame: its pitch (semitones, valid when periodic), how aperiodic it is (YIN's dip,
/// 0 for a pure tone, near 1 for noise) and its power.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Frame {
    pub pitch: f32,
    pub aper: f32,
    pub power: f32,
}

/// The pitch every 10 ms, kept for the whole tune.
pub struct Listener {
    frames: [Frame; MAX_FRAMES],
    n: usize,
    /// Keep only the latest frames, for the mic page, instead of stopping when full.
    rolling: bool,
    buf: [f32; BUF],
    fill: usize,
    dc: Option<(f32, f32)>,
    d: [f32; TAU_MAX + 1],
    /// The smoothed pitch of each frame, NaN where unvoiced, and the notes before they are tidied:
    /// worked out by `notes`.
    sm: [f32; MAX_FRAMES],
    slow: [f32; MAX_FRAMES],
    cuts: [Note; CUTS],
    /// The quietest 30 ms so far, as a power.
    quiet: Option<f32>,
}

impl Default for Listener {
    fn default() -> Self {
        Listener::new()
    }
}

impl Listener {
    pub const fn new() -> Self {
        Listener {
            frames: [Frame { pitch: 0.0, aper: 0.0, power: 0.0 }; MAX_FRAMES],
            n: 0,
            rolling: false,
            buf: [0.0; BUF],
            fill: 0,
            dc: None,
            d: [0.0; TAU_MAX + 1],
            sm: [0.0; MAX_FRAMES],
            slow: [0.0; MAX_FRAMES],
            cuts: [Note { start: 0, end: 0, pitch: 0.0 }; CUTS],
            quiet: None,
        }
    }

    pub fn reset(&mut self, rolling: bool) {
        self.n = 0;
        self.rolling = rolling;
        self.fill = 0;
        self.dc = None;
        self.quiet = None;
    }

    pub fn frames(&self) -> &[Frame] {
        &self.frames[..self.n]
    }

    /// One sample at 16 kHz.
    pub fn feed(&mut self, x: f32) {
        if self.n == MAX_FRAMES {
            if !self.rolling {
                return;
            }
            self.frames.copy_within(MAX_FRAMES / 2.., 0);
            self.n = MAX_FRAMES / 2;
        }
        // a DC blocker (13 Hz): the S3's PDM filter has no high-pass
        let (px, py) = self.dc.unwrap_or((x, 0.0));
        let y = x - px + 0.995 * py;
        self.dc = Some((x, y));
        self.buf[self.fill] = y;
        self.fill += 1;
        if self.fill == BUF {
            let f = self.analyse();
            self.frames[self.n] = f;
            self.n += 1;
            if self.n >= 3 {
                let p = &self.frames[self.n - 3..self.n];
                let avg = (p[0].power + p[1].power + p[2].power) / 3.0;
                self.quiet = Some(self.quiet.map_or(avg, |q| q.min(avg)));
            }
            self.buf.copy_within(HOP.., 0);
            self.fill = BUF - HOP;
        }
    }

    /// YIN (de Cheveigné and Kawahara, 2002) on the window at the start of the buffer.
    fn analyse(&mut self) -> Frame {
        let x = &self.buf;
        let power = x[..WIN].iter().map(|v| v * v).sum::<f32>() / WIN as f32;
        let d = &mut self.d;
        d[0] = 1.0;
        let mut sum = 0.0;
        for tau in 1..=TAU_MAX {
            let mut s = 0.0;
            for j in 0..WIN {
                let e = x[j] - x[j + tau];
                s += e * e;
            }
            sum += s;
            // normalised by the mean so far: 1 for noise, near 0 at the period
            d[tau] = if sum > 0.0 { s * tau as f32 / sum } else { 1.0 };
        }
        // Above 2 kHz a dip can fall between two samples and read shallow: take a dip there that is
        // half as deep.
        let shallow = |t: usize| t < 8 && d[t] < 2.0 * YIN_DIP && d[t] <= d[t - 1] && d[t] <= d[t + 1];
        let mut tau = (TAU_MIN..=TAU_MAX).find(|&t| d[t] < YIN_DIP || shallow(t));
        if let Some(t) = tau.as_mut() {
            while *t < TAU_MAX && d[*t + 1] < d[*t] {
                *t += 1;
            }
        }
        let tau = tau.unwrap_or_else(|| (TAU_MIN..=TAU_MAX).fold(TAU_MIN, |b, t| if d[t] < d[b] { t } else { b }));
        let aper = d[tau];
        // A high note's period is a few samples, so read it off the dip k periods along, k times
        // as finely: a whistle at 2.6 kHz, from about 60 samples.
        let first = dip(d, tau);
        let k = ((48.0 / first) as usize).max(1);
        let at = if k > 1 {
            let near = (first * k as f32 + 0.5) as usize;
            let t = (near.saturating_sub(2)..=(near + 2).min(TAU_MAX - 1)).fold(near, |b, t| if d[t] < d[b] { t } else { b });
            dip(d, t) / k as f32
        } else {
            first
        };
        Frame { pitch: semitones(RATE as f32 / at), aper, power }
    }

    /// The quietest 30 ms so far, as a power, once there are 30 ms.
    pub fn quiet(&self) -> Option<f32> {
        self.quiet
    }

    fn threshold(&self) -> f32 {
        self.quiet.map_or(ABS_MIN, |q| (q * LOUDER).max(ABS_MIN))
    }

    pub fn voiced(&self, f: &Frame) -> bool {
        f.aper < APER_MAX && f.power > self.threshold()
    }

    /// Frames since the last voiced one, and how many were voiced: for when to stop listening.
    fn quiet_since(&self) -> (usize, usize) {
        let fr = self.frames();
        let voiced = fr.iter().filter(|f| self.voiced(f)).count();
        let last = fr.iter().rposition(|f| self.voiced(f));
        (last.map_or(fr.len(), |l| fr.len() - 1 - l), voiced)
    }

    /// The tune is over: quiet for 1.5 s after it, or the track is full.
    pub fn done(&self) -> bool {
        let (since, voiced) = self.quiet_since();
        (voiced >= MIN_VOICED && since >= END_QUIET) || (!self.rolling && self.n == MAX_FRAMES)
    }

    /// The pitch of frame i, a median over the voiced frames of the five around it, if voiced.
    fn smooth(&self, i: usize) -> Option<f32> {
        let fr = self.frames();
        if !self.voiced(&fr[i]) {
            return None;
        }
        let mut v = [0.0f32; 5];
        let mut k = 0;
        for f in &fr[i.saturating_sub(2)..(i + 3).min(fr.len())] {
            if self.voiced(f) {
                v[k] = f.pitch;
                k += 1;
            }
        }
        v[..k].sort_unstable_by(|a, b| a.total_cmp(b));
        Some(v[k / 2])
    }

    /// The notes in the pitch track, as many as `out` holds; how many there are. More than
    /// `out` holds is a tune too long: `Code::of` says so.
    pub fn notes(&mut self, out: &mut [Note]) -> usize {
        let n = self.n;
        for i in 0..n {
            self.sm[i] = self.smooth(i).unwrap_or(f32::NAN);
            self.slow[i] = f32::NAN;
        }
        // the slow track: a median over 21 frames, within a stretch of voice (a gap of 30 ms ends it)
        let mut i = 0;
        while i < n {
            if self.sm[i].is_nan() {
                i += 1;
                continue;
            }
            let (start, mut end) = (i, i + 1);
            for j in i + 1..n {
                if !self.sm[j].is_nan() {
                    end = j + 1;
                } else if j - end >= GAP {
                    break;
                }
            }
            for t in start..end {
                if !self.sm[t].is_nan() {
                    self.slow[t] = median_all(&self.sm, t.saturating_sub(SLOW).max(start), (t + SLOW + 1).min(end));
                }
            }
            i = end;
        }
        let k = self.cut();
        let w = k.min(out.len());
        out[..w].copy_from_slice(&self.cuts[..w]);
        k
    }

    /// The notes, from the smoothed and slow tracks, into `cuts`; how many.
    fn cut(&mut self) -> usize {
        let n = self.n;
        let sm = &self.sm[..n];
        let slow = &self.slow[..n];
        let voiced = |j: usize| !sm[j].is_nan();

        // 1. cut: at gaps of 30 ms or more (silence, or a dip 9 dB under the note), and where the
        // pitch leaves the note
        let mut cut = Cuts { out: &mut self.cuts, k: 0 };
        // the open note: its start, and how loud it has been
        let mut open: Option<(usize, f32)> = None;
        let mut last = 0;
        let mut i = 0;
        while i < n {
            let power = self.frames[i].power;
            if !voiced(i) || open.is_some_and(|(_, peak)| power * 8.0 < peak) {
                if open.is_some() && i - last > GAP {
                    cut.push(open.take().unwrap().0, last + 1);
                }
                i += 1;
                continue;
            }
            match open {
                None => open = Some((i, power)),
                Some((start, peak)) => {
                    // the note's pitch: its recent frames, past its first 20 ms
                    let from = (start + 2).min(i - 1).max(i.saturating_sub(RECENT));
                    let at = median(slow, from, i);
                    let off = slow[i] - at;
                    let ahead = || {
                        let m = median(slow, i, i + HOLD) - at;
                        m.abs() > SPLIT && m.signum() == off.signum()
                    };
                    if off.abs() > SPLIT && i + HOLD <= n && (i..i + HOLD).all(voiced) && ahead() {
                        cut.push(start, i);
                        open = Some((i, power));
                    } else {
                        open = Some((start, power.max(peak * 0.98)));
                    }
                }
            }
            last = i;
            i += 1;
        }
        if let Some((start, _)) = open {
            cut.push(start, last + 1);
        }
        let notes = &mut cut.out[..cut.k];
        let mut k = notes.len();

        // 2. each note's pitch: the median of its middle three fifths
        for note in notes.iter_mut() {
            note.pitch = middle(sm, note.start as usize, note.end as usize);
        }

        // 3. the middle of a glide, cut out as a note of its own, goes
        let mut i = 1;
        while i + 1 < k {
            let (a, b, c) = (notes[i - 1], notes[i], notes[i + 1]);
            let between = (b.pitch - a.pitch) * (c.pitch - b.pitch) > 0.0;
            if a.end == b.start && b.end == c.start && ((b.end - b.start) as usize) < GLIDE && between {
                let mid = b.start + (b.end - b.start) / 2;
                notes[i - 1].end = mid;
                notes[i + 1].start = mid;
                notes.copy_within(i + 1..k, i);
                k -= 1;
                continue;
            }
            i += 1;
        }

        // 4. where two notes join with no gap, the join is where the pitch crosses between them,
        // not where it first left: the glide's middle
        for i in 1..k {
            let (a, b) = (notes[i - 1], notes[i]);
            if a.end != b.start {
                continue;
            }
            let (mid, up) = (0.5 * (a.pitch + b.pitch), b.pitch > a.pitch);
            let lo = (a.end as usize).saturating_sub(8).max(a.start as usize + 1);
            let hi = (a.end as usize + 8).min(b.end as usize - 1);
            let next_side = |j: usize| voiced(j) && (sm[j] > mid) == up;
            // the join that leaves the fewest frames on the wrong side
            let wrong = |j: usize| (lo..j).filter(|&t| next_side(t)).count() + (j..hi).filter(|&t| !next_side(t)).count();
            if let Some(j) = (lo..=hi).min_by_key(|&j| (wrong(j), j.abs_diff(a.end as usize))) {
                notes[i - 1].end = j as u16;
                notes[i].start = j as u16;
            }
        }

        // 5. a short note is a glide into the next one, or out of the last, else a blip
        let mut i = 0;
        while i < k {
            let nt = notes[i];
            if ((nt.end - nt.start) as usize) < MIN_NOTE {
                if i + 1 < k && notes[i + 1].start == nt.end {
                    notes[i + 1].start = nt.start;
                } else if i > 0 && notes[i - 1].end == nt.start {
                    notes[i - 1].end = nt.end;
                }
                notes.copy_within(i + 1..k, i);
                k -= 1;
                continue;
            }
            i += 1;
        }
        // 6. two notes with no gap between them, that close in pitch, are one
        let mut i = 1;
        while i < k {
            if notes[i].start == notes[i - 1].end && (notes[i].pitch - notes[i - 1].pitch).abs() < MERGE {
                notes[i - 1].end = notes[i].end;
                notes[i - 1].pitch = middle(sm, notes[i - 1].start as usize, notes[i - 1].end as usize);
                notes.copy_within(i + 1..k, i);
                k -= 1;
                continue;
            }
            i += 1;
        }
        k
    }
}

/// The median of the voiced pitches in sm[a..b] (up to RECENT of them, the latest), NaN for none.
fn median(sm: &[f32], a: usize, b: usize) -> f32 {
    let mut v = [0.0f32; RECENT];
    let mut k = 0;
    for &p in sm[a..b].iter().rev() {
        if !p.is_nan() && k < RECENT {
            v[k] = p;
            k += 1;
        }
    }
    if k == 0 {
        return f32::NAN;
    }
    v[..k].sort_unstable_by(|x, y| x.total_cmp(y));
    v[k / 2]
}

/// A note's pitch: the median over the middle three fifths of frames [a, b).
fn middle(sm: &[f32], a: usize, b: usize) -> f32 {
    let trim = (b - a) / 5;
    let m = median_all(sm, a + trim, b - trim);
    if m.is_nan() { median_all(sm, a, b) } else { m }
}

/// The median of every voiced pitch in sm[a..b], however many, by counting: no buffer to sort.
fn median_all(sm: &[f32], a: usize, b: usize) -> f32 {
    let v = &sm[a..b];
    let k = v.iter().filter(|p| !p.is_nan()).count();
    if k == 0 {
        return f32::NAN;
    }
    // the value with k/2 voiced pitches below it
    *v.iter()
        .filter(|p| !p.is_nan())
        .find(|&&p| {
            let below = v.iter().filter(|q| !q.is_nan() && **q < p).count();
            let same = v.iter().filter(|q| **q == p).count();
            below <= k / 2 && k / 2 < below + same
        })
        .unwrap_or(&f32::NAN)
}

/// Where the dip at `tau` is, between samples: the parabola through it and its neighbours.
fn dip(d: &[f32], tau: usize) -> f32 {
    if tau < 1 || tau + 1 >= d.len() {
        return tau as f32;
    }
    let (a, b, c) = (d[tau - 1], d[tau], d[tau + 1]);
    let den = a - 2.0 * b + c;
    if den > 1e-9 { tau as f32 + ((a - c) / (2.0 * den)).clamp(-0.5, 0.5) } else { tau as f32 }
}

/// The notes as they are cut.
struct Cuts<'a> {
    out: &'a mut [Note],
    k: usize,
}

impl Cuts<'_> {
    fn push(&mut self, start: usize, end: usize) {
        if let Some(n) = self.out.get_mut(self.k) {
            *n = Note { start: start as u16, end: end as u16, pitch: 0.0 };
            self.k += 1;
        }
    }
}

// ----------------------------------------------------------------------------- the speaker
/// The start of a common tune, which the device plays and you finish your own way: MIDI notes
/// (an octave up, for the Cardputer's small speaker) and how long each lasts, in ms.
pub struct Opening {
    pub name: &'static str,
    pub notes: &'static [(u8, u16)],
}

pub const OPENINGS: [Opening; 4] = [
    Opening { name: "Twinkle, twinkle", notes: &[(72, 400), (72, 400), (79, 400), (79, 400)] },
    Opening { name: "Ode to Joy", notes: &[(76, 400), (76, 400), (77, 400), (79, 400)] },
    Opening { name: "Frere Jacques", notes: &[(72, 400), (74, 400), (76, 400), (72, 400)] },
    Opening { name: "Mary's lamb", notes: &[(76, 400), (74, 400), (72, 400), (74, 400)] },
];

/// Samples of the tones at 16 kHz: each note a sine with an 8 ms attack and a 40 ms release, then
/// 40 ms of silence; 100 ms of silence after the last.
pub struct Tones {
    notes: &'static [(u8, u16)],
    i: usize,
    t: u32,
    phase: u32,
    step: u32,
}

const AMP: f32 = 6000.0;
const ATTACK: u32 = RATE * 8 / 1000;
const RELEASE: u32 = RATE * 40 / 1000;
const REST: u32 = RATE * 40 / 1000;
const TAIL: u32 = RATE / 10;

impl Tones {
    pub fn new(notes: &'static [(u8, u16)]) -> Self {
        let mut t = Tones { notes, i: 0, t: 0, phase: 0, step: 0 };
        t.start();
        t
    }

    fn start(&mut self) {
        if let Some(&(m, _)) = self.notes.get(self.i) {
            self.step = (hz(m) / RATE as f32 * 4_294_967_296.0) as u32;
            self.phase = 0;
        }
    }
}

impl Iterator for Tones {
    type Item = i16;

    fn next(&mut self) -> Option<i16> {
        let Some(&(_, ms)) = self.notes.get(self.i) else {
            // the tail, so the last note's release isn't cut off
            self.t += 1;
            return (self.t <= TAIL).then_some(0);
        };
        let len = ms as u32 * RATE / 1000;
        let sound = len.saturating_sub(REST);
        let t = self.t;
        let s = if t < sound {
            let env = if t < ATTACK {
                t as f32 / ATTACK as f32
            } else if t + RELEASE > sound {
                (sound - t) as f32 / RELEASE as f32
            } else {
                1.0
            };
            let s = sine(self.phase) * env * AMP;
            self.phase = self.phase.wrapping_add(self.step);
            s as i16
        } else {
            0
        };
        self.t += 1;
        if self.t >= len {
            self.i += 1;
            self.t = 0;
            self.start();
        }
        Some(s)
    }
}

/// sin(2π · phase / 2³²), to about 0.1%: a parabola, then a correction.
fn sine(phase: u32) -> f32 {
    let x = phase as i32 as f32 / 2_147_483_648.0; // -1..1, half turns
    let p = 4.0 * x * (1.0 - x.abs());
    0.225 * (p * p.abs() - p) + p
}

// ----------------------------------------------------------------------------- numbers
/// The frequency of MIDI note m.
pub fn hz(m: u8) -> f32 {
    const RATIO: [f32; 12] = [
        1.0, 1.059_463_1, 1.122_462, 1.189_207_1, 1.259_921, 1.334_839_8, core::f32::consts::SQRT_2, 1.498_307_1, 1.587_401,
        1.681_792_9, 1.781_797_4, 1.887_748_6,
    ];
    let k = m as i32 - 69;
    let (oct, step) = (k.div_euclid(12), k.rem_euclid(12));
    let mut f = 440.0 * RATIO[step as usize];
    for _ in 0..oct.abs() {
        f = if oct > 0 { f * 2.0 } else { f * 0.5 };
    }
    f
}

/// A frequency in semitones, on MIDI's numbering.
pub fn semitones(hz: f32) -> f32 {
    69.0 + 12.0 * log2(hz / 440.0)
}

/// log2(x) for x > 0, to about 1e-5: the exponent, and a series for the mantissa.
pub fn log2(x: f32) -> f32 {
    let bits = x.to_bits();
    let e = ((bits >> 23) & 0xff) as i32 - 127;
    let m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000);
    // ln m = 2 atanh((m - 1) / (m + 1)), m in [1, 2)
    let t = (m - 1.0) / (m + 1.0);
    let t2 = t * t;
    let ln = 2.0 * t * (1.0 + t2 * (1.0 / 3.0 + t2 * (1.0 / 5.0 + t2 * (1.0 / 7.0 + t2 / 9.0))));
    e as f32 + ln * core::f32::consts::LOG2_E
}

/// A note's name: "A4" for 69.
pub fn name(pitch: f32) -> crate::buf::Buf<4> {
    use core::fmt::Write;
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let m = (pitch + 0.5) as i32;
    let mut b = crate::buf::Buf::new();
    if m > 0 {
        let _ = write!(b, "{}{}", NAMES[(m % 12) as usize], m / 12 - 1);
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small LCG: the same noise every run.
    struct Noise(u32);
    impl Noise {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (self.0 >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        }
    }

    /// A voice: harmonics falling off like a hum's, or a whistle's pure tone, with breath noise.
    #[derive(Clone, Copy)]
    enum Voice {
        Hum,
        Whistle,
    }

    /// One way of singing a tune: its key, speed, how far off each note is, vibrato and noise.
    struct Take {
        transpose: f32,
        tempo: f32,
        voice: Voice,
        /// semitones off per note, cycled
        drift: &'static [f32],
        /// ms per note off its length, cycled
        jitter: &'static [i32],
        vibrato: f32,
        noise: f32,
        /// ms of glide between legato notes
        glide: u32,
    }

    const PLAIN: Take = Take { transpose: 0.0, tempo: 1.0, voice: Voice::Hum, drift: &[0.0], jitter: &[0], vibrato: 0.0, noise: 0.0, glide: 0 };

    /// A tune: (MIDI note, ms, gap after it in ms). A gap of 0 is legato.
    type Tune = &'static [(f32, u32, u32)];

    /// "Your ending" for the tests: nobody's secret.
    const TUNE: Tune = &[
        (60.0, 300, 60), (62.0, 300, 0), (64.0, 600, 80), (64.0, 300, 0), (67.0, 300, 60), (65.0, 600, 0),
        (64.0, 300, 0), (62.0, 300, 0), (57.0, 600, 60), (60.0, 900, 0),
    ];

    /// The tune sung as `take` says, as the mic sends it: 32 kHz, two slots, the second dead.
    fn sing(tune: Tune, take: &Take) -> Vec<u8> {
        let rate = 32_000.0;
        let mut out = Vec::new();
        let mut noise = Noise(7);
        let mut phase = 0.0f32;
        let push = |x: f32, out: &mut Vec<u8>| {
            out.extend_from_slice(&((x * 3000.0) as i16).to_le_bytes());
            out.extend_from_slice(&0i16.to_le_bytes());
        };
        // 400 ms of a quiet room first: the mic wakes and the slot is picked
        for _ in 0..(rate * 0.4) as usize {
            push(take.noise * noise.next(), &mut out);
        }
        let mut t_all = 0.0;
        let mut legato = false;
        for (i, &(m, ms, gap)) in tune.iter().enumerate() {
            let len = ((ms as f32 + take.jitter[i % take.jitter.len()] as f32) / take.tempo) / 1000.0;
            let gap = gap as f32 / take.tempo / 1000.0;
            let from = m + take.transpose + take.drift[i % take.drift.len()];
            let next = tune.get(i + 1).map(|n| n.0 + take.transpose + take.drift[(i + 1) % take.drift.len()]);
            let n = (len * rate) as usize;
            for s in 0..n {
                let t = s as f32 / rate;
                let mut p = from + take.vibrato * (2.0 * std::f32::consts::PI * 5.5 * t_all).sin();
                // a legato note glides into the next over its last `glide` ms
                if let (Some(to), 0) = (next, gap as u32) {
                    let g = take.glide as f32 / 1000.0;
                    if g > 0.0 && t > len - g {
                        p += (to - from) * (t - (len - g)) / g;
                    }
                }
                let f = 440.0 * 2f32.powf((p - 69.0) / 12.0);
                phase += 2.0 * std::f32::consts::PI * f / rate;
                let attack = if legato { 1.0 } else { (t / 0.02).min(1.0) };
                let env = attack.min((len - t) / 0.02 + if gap == 0.0 { 1.0 } else { 0.0 });
                let x = match take.voice {
                    Voice::Hum => (1..=6).map(|h| (phase * h as f32).sin() / h as f32).sum::<f32>() * 0.5,
                    Voice::Whistle => phase.sin(),
                };
                push(x * env + take.noise * noise.next(), &mut out);
                t_all += 1.0 / rate;
            }
            for _ in 0..(gap * rate) as usize {
                push(take.noise * noise.next(), &mut out);
                t_all += 1.0 / rate;
            }
            legato = gap == 0.0;
        }
        for _ in 0..(rate * 1.7) as usize {
            push(take.noise * noise.next(), &mut out);
        }
        out
    }

    fn listen(bytes: &[u8]) -> (Box<Listener>, Vec<Note>) {
        let mut l = Box::new(Listener::new());
        let mut mic = Mic::new(None);
        for chunk in bytes.chunks(4092) {
            mic.feed(chunk, &mut l);
        }
        assert_eq!(mic.slot, Some(0));
        let mut notes = [Note::default(); MAX_NOTES + 1];
        let k = l.notes(&mut notes).min(MAX_NOTES + 1);
        (l, notes[..k].to_vec())
    }

    fn code(tune: Tune, take: &Take) -> Code {
        let (_, notes) = listen(&sing(tune, take));
        Code::of(&notes).unwrap()
    }

    #[test]
    fn log2_and_notes() {
        for x in [0.001f32, 0.5, 1.0, 1.5, 2.0, 3.0, 440.0, 1234.5, 16000.0] {
            assert!((log2(x) - x.log2()).abs() < 2e-5, "{x}");
        }
        assert!((semitones(440.0) - 69.0).abs() < 1e-4);
        assert!((semitones(hz(60)) - 60.0).abs() < 1e-3);
        assert!((hz(81) - 880.0).abs() < 1e-2 && (hz(57) - 220.0).abs() < 1e-3);
        assert_eq!(name(69.2).as_str(), "A4");
        assert_eq!(name(60.0).as_str(), "C4");
        assert_eq!(name(73.4).as_str(), "C#5");
    }

    #[test]
    fn the_sine_is_a_sine() {
        for k in 0..64u32 {
            let phase = k * (u32::MAX / 64);
            let want = (phase as f64 / 4_294_967_296.0 * std::f64::consts::TAU).sin() as f32;
            assert!((sine(phase) - want).abs() < 0.0015, "{k}");
        }
    }

    // A pure tone or a hum, from 80 Hz to 3 kHz, within a tenth of a semitone.
    #[test]
    fn pitch_of_tones() {
        for &(f, hum) in &[(80.0f32, true), (110.0, true), (196.0, true), (330.0, false), (523.0, false), (1000.0, false), (1760.0, false), (3000.0, false)] {
            let mut l = Listener::new();
            let mut ph = 0.0f32;
            for _ in 0..RATE / 4 {
                ph += 2.0 * std::f32::consts::PI * f / RATE as f32;
                let x = if hum { (1..=6).map(|h| (ph * h as f32).sin() / h as f32).sum::<f32>() } else { ph.sin() };
                l.feed(x * 2000.0);
            }
            let last = l.frames().last().unwrap();
            assert!(last.aper < 0.1, "{f}: aper {}", last.aper);
            assert!((last.pitch - semitones(f)).abs() < 0.1, "{f}: {} vs {}", last.pitch, semitones(f));
        }
    }

    // Noise is never a note.
    #[test]
    fn noise_has_no_notes() {
        let mut l = Listener::new();
        let mut n = Noise(1);
        for i in 0..RATE * 2 {
            let loud = if (i / 4000) % 2 == 0 { 3000.0 } else { 30.0 };
            l.feed(n.next() * loud);
        }
        let mut notes = [Note::default(); MAX_NOTES];
        assert_eq!(l.notes(&mut notes), 0);
    }

    // The device's own openings, played by Tones and heard by the listener, come back note for note.
    #[test]
    fn the_openings_come_back() {
        for o in &OPENINGS {
            let mut l = Listener::new();
            for s in Tones::new(o.notes) {
                l.feed(s as f32);
            }
            for _ in 0..2 * RATE {
                l.feed(0.0);
            }
            let mut notes = [Note::default(); MAX_NOTES];
            let k = l.notes(&mut notes);
            assert_eq!(k, o.notes.len(), "{}", o.name);
            for (n, &(m, _)) in notes[..k].iter().zip(o.notes) {
                assert!((n.pitch - m as f32).abs() < 0.1, "{}: {} vs {m}", o.name, n.pitch);
            }
            assert!(l.done());
        }
    }

    #[test]
    fn the_tune_is_heard() {
        let (l, notes) = listen(&sing(TUNE, &PLAIN));
        assert_eq!(notes.len(), TUNE.len(), "{notes:?}");
        for (n, t) in notes.iter().zip(TUNE) {
            assert!((n.pitch - t.0).abs() < 0.15, "{n:?} vs {t:?}");
        }
        assert!(l.done());
        let c = Code::of(&notes).unwrap();
        assert_eq!(c.notes(), 10);
        use Move::*;
        let moves: Vec<Move> = c.steps().iter().map(|s| s.0).collect();
        assert_eq!(moves, [Up, Up, Same, UpLeap, Down, Down, Down, DownLeap, UpLeap]);
        let long: Vec<bool> = c.steps().iter().map(|s| s.1).collect();
        assert_eq!(long, [false, false, true, false, false, true, false, false, true]);
    }

    // The point of it: hummed in another key, faster or slower, a little off, with vibrato,
    // glides and a noisy room, or whistled two octaves up, the code is the same.
    #[test]
    fn the_code_comes_out_the_same() {
        let want = code(TUNE, &PLAIN);
        let takes = [
            Take { transpose: 5.0, tempo: 1.15, ..PLAIN },
            Take { transpose: -7.0, tempo: 0.85, ..PLAIN },
            Take { drift: &[0.25, -0.2, 0.1, -0.3, 0.15], jitter: &[30, -40, 0, 50, -20], ..PLAIN },
            Take { vibrato: 0.3, noise: 0.01, ..PLAIN },
            Take { glide: 40, ..PLAIN },
            Take { voice: Voice::Whistle, transpose: 24.0, noise: 0.02, ..PLAIN },
            Take { voice: Voice::Whistle, transpose: 19.0, tempo: 1.1, drift: &[0.15, -0.15, 0.0], glide: 30, vibrato: 0.2, ..PLAIN },
        ];
        for (i, t) in takes.iter().enumerate() {
            assert_eq!(code(TUNE, t), want, "take {i}");
        }
    }

    #[test]
    fn another_tune_is_another_code() {
        const OTHER: Tune = &[
            (60.0, 300, 60), (62.0, 300, 0), (64.0, 600, 80), (64.0, 300, 0), (67.0, 300, 60), (65.0, 600, 0),
            (64.0, 300, 0), (65.0, 300, 0), (57.0, 600, 60), (60.0, 900, 0),
        ];
        const SLOWER: Tune = &[
            (60.0, 300, 60), (62.0, 300, 0), (64.0, 300, 80), (64.0, 300, 0), (67.0, 300, 60), (65.0, 600, 0),
            (64.0, 300, 0), (62.0, 300, 0), (57.0, 600, 60), (60.0, 900, 0),
        ];
        let want = code(TUNE, &PLAIN);
        assert_ne!(code(OTHER, &PLAIN), want); // one note moves the other way
        assert_ne!(code(SLOWER, &PLAIN), want); // one long note made short
    }

    // The slot that carries the mic is picked, whichever it is.
    #[test]
    fn the_mic_slot() {
        let bytes = sing(TUNE, &Take { noise: 0.01, ..PLAIN });
        let mut swapped = bytes.clone();
        for fr in swapped.chunks_exact_mut(4) {
            fr.rotate_left(2);
        }
        let mut l = Listener::new();
        let mut mic = Mic::new(None);
        mic.feed(&swapped, &mut l);
        assert_eq!(mic.slot, Some(1));
        let mut notes = [Note::default(); MAX_NOTES];
        assert_eq!(l.notes(&mut notes), TUNE.len());
        // chosen by hand, it stays
        let mut mic = Mic::new(Some(0));
        mic.feed(&swapped, &mut Listener::new());
        assert_eq!(mic.slot, Some(0));
    }

    #[test]
    fn too_many_notes() {
        let notes: Vec<Note> = (0..MAX_NOTES as u16 + 1).map(|i| Note { start: i * 20, end: i * 20 + 15, pitch: 60.0 + (i % 3) as f32 * 2.0 }).collect();
        assert!(Code::of(&notes).is_none());
        assert_eq!(Code::of(&notes[..MAX_NOTES]).unwrap().notes(), MAX_NOTES);
        assert_eq!(Code::of(&notes[..1]).unwrap().notes(), 0);
    }

    // Many takes at random: how often the code comes out different. Not a pass or fail: run it
    // when changing a threshold, `RUNS=100 DRIFT=0.2 JITTER=25 cargo test --release --lib sweep
    // -- --ignored --nocapture` (semitones off per note, ms off per note).
    #[test]
    #[ignore]
    fn sweep() {
        let tunes: [(&str, Tune); 3] = [
            ("tune", TUNE),
            ("ode", &[(64.0, 400, 50), (64.0, 400, 50), (65.0, 400, 0), (67.0, 400, 50), (67.0, 400, 0), (65.0, 400, 0),
                      (64.0, 400, 0), (62.0, 400, 0), (60.0, 400, 50), (60.0, 400, 0), (62.0, 400, 0), (64.0, 400, 0),
                      (64.0, 600, 0), (62.0, 200, 50), (62.0, 800, 0)]),
            ("leaps", &[(55.0, 250, 40), (62.0, 250, 40), (59.0, 500, 0), (67.0, 250, 40), (64.0, 250, 0), (60.0, 750, 60),
                        (57.0, 250, 0), (59.0, 250, 0), (60.0, 1000, 0)]),
        ];
        let env = |k: &str, d: f32| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
        let (runs, drift, jitter) = (env("RUNS", 40.0) as usize, env("DRIFT", 0.2), env("JITTER", 25.0));
        let mut rng = Noise(99);
        let mut uni = |a: f32, b: f32| a + (b - a) * (rng.next() * 0.5 + 0.5);
        for (name, tune) in tunes {
            let want = code(tune, &PLAIN);
            let mut bad = 0;
            for r in 0..runs {
                let drift: Vec<f32> = (0..7).map(|_| uni(-drift, drift)).collect();
                let jitter: Vec<i32> = (0..7).map(|_| uni(-jitter, jitter) as i32).collect();
                let whistle = r % 3 == 0;
                let take = Take {
                    transpose: if whistle { uni(12.0, 26.0) } else { uni(-12.0, 7.0) },
                    tempo: uni(0.8, 1.25),
                    voice: if whistle { Voice::Whistle } else { Voice::Hum },
                    drift: Box::leak(drift.into_boxed_slice()),
                    jitter: Box::leak(jitter.into_boxed_slice()),
                    vibrato: uni(0.0, 0.3),
                    noise: uni(0.0, 0.03),
                    glide: uni(0.0, 50.0) as u32,
                };
                let (_, notes) = listen(&sing(tune, &take));
                let Some(got) = Code::of(&notes) else {
                    bad += 1;
                    println!("{name} run {r}: {} notes, too many: t {:+.1} x{:.2} vib {:.2} glide {} noise {:.3} {:?}", notes.len(),
                             take.transpose, take.tempo, take.vibrato, take.glide, take.noise, take.jitter);
                    for nt in &notes { println!("  {:?} {:.2}", (nt.start, nt.end), nt.pitch); }
                    continue;
                };
                if got != want {
                    bad += 1;
                    println!("{name} run {r}: t {:+.1} x{:.2} vib {:.2} glide {} noise {:.3} {:?} {:?}\n  got  {:?}\n  want {:?}",
                             take.transpose, take.tempo, take.vibrato, take.glide, take.noise, take.drift, take.jitter,
                             got.steps(), want.steps());
                }
            }
            println!("{name}: {bad} of {runs} takes came out different");
        }
    }
}
