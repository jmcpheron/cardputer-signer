//! The hum image on the screen: its pages, and what each key does on each (docs/HUM.md). The hum
//! image (firmware/src/bin/hum.rs) feeds it the keys held down and the time, plays the opening and
//! listens when it says, and draws what it says. `cargo test` drives it the same way.
//!
//! An experiment: nothing here signs or touches the chip, and the tune lives in RAM only, until the
//! power goes off. To set a tune, the Cardputer plays an opening, you hum or whistle your own ending,
//! three times; the three must come out the same. To try it, once.
use core::fmt::Write;

use crate::buf::Buf;
use crate::ceremony_pages::{header, keys};
use crate::font::Face;
use crate::gfx::{theme::*, Frame, Rgb};
use crate::hum::{self, Code, Listener, Mic, Move, Note, MAX_NOTES, MIN_NOTES, OPENINGS};
use crate::screens::{fit, para, W};

const M: i32 = 6;
const R: i32 = W - M;
const TW: i32 = W - 2 * M;
const FOOT: i32 = 117;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Key {
    Enter,
    Del,
    Up,
    Down,
    Left,
    Right,
}

/// Which go this is: one of the three that set the tune, or a try.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Round {
    Set(u8),
    Try,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Page {
    Home,
    /// Pick the opening of the tune being set.
    Pick,
    /// The firmware plays the opening, then calls `played`.
    Play(Round),
    /// The firmware feeds the mic in; Enter, or 1.5 s of quiet, ends it.
    Listen(Round),
    /// What a round heard, before the next.
    Heard(Round),
    /// Fewer than MIN_NOTES heard, or more than MAX_NOTES.
    Short(Round),
    Long(Round),
    /// The three rounds came out the same: the tune is set.
    Set,
    /// They didn't.
    Differ,
    /// A try: the tune, or not.
    Match(bool),
    /// The mic, live.
    Mic,
}

pub struct Hum {
    pub page: Page,
    /// The line picked on the home page.
    sel: usize,
    /// The opening, for the tune being set.
    opening: usize,
    /// The tune: its opening and its code. In RAM, until the power goes off.
    tune: Option<(usize, Code)>,
    rounds: [Code; 3],
    /// The last go: its notes and its code.
    notes: [Note; MAX_NOTES],
    n: usize,
    heard: Code,
    listener: Listener,
    mic: Mic,
    /// The mic's slot, chosen by hand on the mic page, or None to pick it each time.
    slot: Option<usize>,
    held: Option<Key>,
}

impl Default for Hum {
    fn default() -> Self {
        Hum::new()
    }
}

impl Hum {
    pub const fn new() -> Self {
        Hum {
            page: Page::Home,
            sel: 0,
            opening: 0,
            tune: None,
            rounds: [Code::EMPTY; 3],
            notes: [Note { start: 0, end: 0, pitch: 0.0 }; MAX_NOTES],
            n: 0,
            heard: Code::EMPTY,
            listener: Listener::new(),
            mic: Mic::new(None),
            slot: None,
            held: None,
        }
    }

    /// The notes to play now, on a Play page.
    pub fn opening(&self) -> &'static [(u8, u16)] {
        let o = match self.page {
            Page::Play(Round::Try) => self.tune.map_or(0, |t| t.0),
            _ => self.opening,
        };
        OPENINGS[o].notes
    }

    /// The opening has played: listen.
    pub fn played(&mut self) {
        if let Page::Play(r) = self.page {
            self.listen(Page::Listen(r));
        }
    }

    fn listen(&mut self, page: Page) {
        self.page = page;
        self.listener.reset(page == Page::Mic);
        self.mic = Mic::new(self.slot);
    }

    /// Whether the firmware should be feeding the mic in.
    pub fn listening(&self) -> bool {
        matches!(self.page, Page::Listen(_) | Page::Mic)
    }

    /// Bytes from the mic: see `Mic::feed`.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.mic.feed(bytes, &mut self.listener);
    }

    /// The slot the mic is on, once picked, and the level of each, for the log.
    pub fn mic(&self) -> (Option<usize>, [f32; 2]) {
        (self.mic.slot, self.mic.level)
    }

    /// The firmware lost the mic: a go ends with what it heard, the mic page goes home.
    pub fn stop(&mut self) {
        match self.page {
            Page::Listen(_) => self.finish(),
            Page::Mic => self.page = Page::Home,
            _ => {}
        }
    }

    /// The last go, for the log: how many notes, over how many 10 ms frames. Never the code.
    pub fn heard(&self) -> (usize, usize) {
        (self.n, self.listener.frames().len())
    }

    /// The key held down now (None for none), at `now` ms. Returns whether the screen needs
    /// drawing again. While listening, it is drawn again anyway, every 100 ms or so.
    pub fn input(&mut self, down: Option<Key>, _now: u32) -> bool {
        let mut dirty = false;
        if down != self.held {
            self.held = down;
            if let Some(k) = down {
                dirty = self.press(k);
            }
        }
        if matches!(self.page, Page::Listen(_)) && self.listener.done() {
            self.finish();
            dirty = true;
        }
        dirty
    }

    fn press(&mut self, k: Key) -> bool {
        use Page::*;
        match (self.page, k) {
            (Home, Key::Up) => self.sel = (self.sel + 2) % 3,
            (Home, Key::Down) => self.sel = (self.sel + 1) % 3,
            (Home, Key::Enter) => match self.sel {
                0 => self.page = Pick,
                1 if self.tune.is_some() => self.page = Play(Round::Try),
                2 => self.listen(Mic),
                _ => return false,
            },
            (Pick, Key::Left) => self.opening = (self.opening + OPENINGS.len() - 1) % OPENINGS.len(),
            (Pick, Key::Right) => self.opening = (self.opening + 1) % OPENINGS.len(),
            (Pick, Key::Enter) => self.page = Play(Round::Set(1)),
            (Listen(_), Key::Enter) => self.finish(),
            (Heard(Round::Set(i)), Key::Enter) => self.page = Play(Round::Set(i + 1)),
            (Short(r) | Long(r), Key::Enter) => self.page = Play(r),
            (Differ, Key::Enter) => self.page = Pick,
            (Set | Match(_), Key::Enter) => self.page = Home,
            (Mic, Key::Left) => self.choose(Some(0)),
            (Mic, Key::Right) => self.choose(Some(1)),
            (Mic, Key::Enter) => self.choose(None),
            (Home, Key::Del) => return false,
            (_, Key::Del) => self.page = Home,
            _ => return false,
        }
        true
    }

    fn choose(&mut self, slot: Option<usize>) {
        self.slot = slot;
        self.mic = Mic::new(slot);
    }

    /// Listening is over: what was heard, and where that leads.
    fn finish(&mut self) {
        let Page::Listen(r) = self.page else { return };
        self.n = self.listener.notes(&mut self.notes);
        if self.n > MAX_NOTES {
            self.page = Page::Long(r);
            return;
        }
        if self.n < MIN_NOTES {
            self.page = Page::Short(r);
            return;
        }
        let c = Code::of(&self.notes[..self.n]).unwrap_or_default();
        self.heard = c;
        self.page = match r {
            Round::Try => Page::Match(self.tune.is_some_and(|t| t.1 == c)),
            Round::Set(i) => {
                self.rounds[i as usize - 1] = c;
                if i < 3 {
                    Page::Heard(r)
                } else if self.rounds.iter().all(|&x| x == c) {
                    self.tune = Some((self.opening, c));
                    self.sel = 1;
                    Page::Set
                } else {
                    Page::Differ
                }
            }
        };
    }

    // ------------------------------------------------------------------------- drawing
    pub fn draw(&self, f: &mut Frame, _now: u32) {
        f.fill(BG);
        match self.page {
            Page::Home => self.home(f),
            Page::Pick => {
                header(f, "Set", "Pick an opening", "Your tune");
                let name = OPENINGS[self.opening].name;
                f.text("<", M + 2, 30, Face::Bold18, MUTED, BG);
                f.text_r(">", R - 2, 30, Face::Bold18, MUTED, BG);
                f.text_c(name, 30, Face::Bold18, TEXT, BG);
                para(f, "It plays these first notes. Then you hum or whistle your own ending, not the song's: that is the tune.",
                     M, 60, TW, Face::Sans12, MUTED, FOOT - 3);
                keys(f, Some(("Del", "Back")), Some(("Enter", "Play it", RAISED, TEXT)));
            }
            Page::Play(r) => {
                round(f, r, "Listen");
                let o = &OPENINGS[match r { Round::Try => self.tune.map_or(0, |t| t.0), _ => self.opening }];
                f.text("Listen…", M, 26, Face::Bold18, TEXT, BG);
                f.text(o.name, M, 52, Face::Sans14, MUTED, BG);
                f.text("Then it's your turn.", M, 72, Face::Sans12, FAINT, BG);
                keys(f, Some(("Del", "Cancel")), None);
            }
            Page::Listen(r) => {
                round(f, r, "Your turn");
                f.text("Hum or whistle your ending", M, 22, Face::Bold14, TEXT, BG);
                trace(f, &self.listener, M, 42, TW, 50);
                level(f, &self.listener, M, 97, TW);
                f.text("Stops when you do", M, 101, Face::Sans12, FAINT, BG);
                keys(f, Some(("Del", "Cancel")), Some(("Enter", "Done", RAISED, TEXT)));
            }
            Page::Heard(r) => {
                round(f, r, "Heard");
                let mut s: Buf<40> = Buf::new();
                let _ = write!(s, "{} notes", self.n);
                f.text(s.as_str(), M, 21, Face::Bold14, TEXT, BG);
                roll(f, &self.notes[..self.n], M, 40, TW, 40);
                tokens(f, &self.heard, M, 86, None);
                let next = if let Round::Set(i) = r { i + 1 } else { 1 };
                let mut k: Buf<12> = Buf::new();
                let _ = write!(k, "Round {next}");
                keys(f, Some(("Del", "Stop")), Some(("Enter", k.as_str(), RAISED, TEXT)));
            }
            Page::Short(r) | Page::Long(r) => {
                round(f, r, "Again");
                f.mark(false, M + 9, 32, 9);
                let mut s: Buf<40> = Buf::new();
                if self.page == Page::Short(r) {
                    let _ = write!(s, "Only {} notes", self.n);
                } else {
                    let _ = write!(s, "{} notes", self.n);
                }
                f.text(s.as_str(), M + 25, 22, Face::Bold14, TEXT, BG);
                let mut t: Buf<80> = Buf::new();
                let _ = write!(t, "A tune here has {MIN_NOTES} to {MAX_NOTES} notes. Repeated notes need a short break between them.");
                para(f, t.as_str(), M, 48, TW, Face::Sans12, MUTED, FOOT - 3);
                keys(f, Some(("Del", "Stop")), Some(("Enter", "Again", RAISED, TEXT)));
            }
            Page::Set => {
                header(f, "Set", "Your tune is set", "");
                f.mark(true, M + 11, 33, 11);
                let mut s: Buf<48> = Buf::new();
                let _ = write!(s, "{} notes, after {}", self.heard.notes(), OPENINGS[self.opening].name);
                f.text(fit(s.as_str(), TW - 30, Face::Bold14).as_str(), M + 30, 26, Face::Bold14, TEXT, BG);
                tokens(f, &self.heard, M, 52, None);
                para(f, "All three came out the same. It is kept until the power goes off.", M, 76, TW, Face::Sans12, MUTED, FOOT - 3);
                keys(f, None, Some(("Enter", "Done", RAISED, TEXT)));
            }
            Page::Differ => {
                header(f, "Set", "Not the same", "");
                f.text("The three rounds came out different:", M, 21, Face::Sans12, MUTED, BG);
                for (i, c) in self.rounds.iter().enumerate() {
                    tokens(f, c, M, 38 + 24 * i as i32, Some(&self.rounds[0]));
                }
                keys(f, Some(("Del", "Stop")), Some(("Enter", "Start over", RAISED, TEXT)));
            }
            Page::Match(ok) => {
                header(f, "Try", if ok { "Your tune" } else { "Not your tune" }, "");
                f.mark(ok, M + 11, 33, 11);
                let want = self.tune.map_or(0, |t| t.1.notes());
                let mut s: Buf<48> = Buf::new();
                let _ = write!(s, "Heard {} notes; the tune has {want}", self.heard.notes());
                f.text(s.as_str(), M + 30, 26, Face::Sans12, TEXT, BG);
                tokens(f, &self.heard, M, 56, self.tune.as_ref().map(|t| &t.1).filter(|_| !ok));
                if !ok {
                    f.text("Red: where it differs from the tune", M, 80, Face::Sans12, FAINT, BG);
                }
                keys(f, None, Some(("Enter", "Done", RAISED, TEXT)));
            }
            Page::Mic => self.mic_page(f),
        }
    }

    fn home(&self, f: &mut Frame) {
        header(f, "Hum", "A tune as a second factor", "Experiment");
        let lines = ["Set your tune", "Try your tune", "Check the mic"];
        for (i, l) in lines.iter().enumerate() {
            let y = 24 + 19 * i as i32;
            let off = i == 1 && self.tune.is_none();
            if i == self.sel {
                f.rrect(M - 2, y - 1, TW + 4, 18, 4, RAISED);
            }
            let bg = if i == self.sel { RAISED } else { BG };
            f.text(l, M + 6, y, Face::Bold14, if off { FAINT } else { TEXT }, bg);
        }
        let mut s: Buf<64> = Buf::new();
        match self.tune {
            Some((o, c)) => {
                let _ = write!(s, "Your tune: {} notes, after {}", c.notes(), OPENINGS[o].name);
            }
            None => {
                let _ = write!(s, "No tune yet.");
            }
        }
        f.text(fit(s.as_str(), TW, Face::Sans12).as_str(), M, 84, Face::Sans12, MUTED, BG);
        f.text("Nothing here signs. Power off forgets it.", M, 99, Face::Sans12, FAINT, BG);
        keys(f, None, Some(("Enter", "Go", RAISED, TEXT)));
    }

    fn mic_page(&self, f: &mut Frame) {
        header(f, "Mic", "Check the mic", if self.slot.is_some() { "Slot by hand" } else { "Slot picked" });
        for k in 0..2 {
            let y = 23 + 13 * k as i32;
            let on = self.mic.slot == Some(k);
            let mut s: Buf<8> = Buf::new();
            let _ = write!(s, "Slot {}", k + 1);
            f.text(s.as_str(), M, y - 2, Face::Sans12, if on { TEXT } else { FAINT }, BG);
            bar(f, M + 44, y + 2, TW - 44, db(self.mic.level[k]) / 90.0, if on { OK } else { FAINT });
        }
        let last = self.listener.frames().last().filter(|fr| self.listener.voiced(fr));
        let mut s: Buf<24> = Buf::new();
        match last {
            Some(fr) => {
                let _ = write!(s, "{}  {} Hz", hum::name(fr.pitch), hz(fr.pitch));
            }
            None => {
                let _ = write!(s, "Hum or whistle");
            }
        }
        f.text(s.as_str(), M, 50, Face::Bold14, if last.is_some() { TEXT } else { MUTED }, BG);
        trace(f, &self.listener, M, 70, TW, 34);
        keys(f, Some(("Del", "Back")), None);
        f.text_r(", / slot  Enter auto", R, FOOT, Face::Sans12, FAINT, BG);
    }
}

/// The header of a round: "Set 2/3" or "Try".
fn round(f: &mut Frame, r: Round, title: &str) {
    let mut tag: Buf<8> = Buf::new();
    match r {
        Round::Set(i) => {
            let _ = write!(tag, "Set {i}/3");
        }
        Round::Try => {
            let _ = write!(tag, "Try");
        }
    }
    header(f, tag.as_str(), title, "");
}

/// The pitch track as dots, the last 3 seconds, scaled to what it holds (at least an octave).
fn trace(f: &mut Frame, l: &Listener, x: i32, y: i32, w: i32, h: i32) {
    f.rrect(x, y, w, h, 4, SURFACE);
    let fr = l.frames();
    let shown = &fr[fr.len().saturating_sub(300)..];
    let voiced = || shown.iter().filter(|p| l.voiced(p)).map(|p| p.pitch);
    let (lo, hi) = (voiced().fold(f32::MAX, f32::min), voiced().fold(f32::MIN, f32::max));
    if lo > hi {
        return;
    }
    let mid = 0.5 * (lo + hi);
    let span = (hi - lo).max(12.0) + 2.0;
    for (i, p) in shown.iter().enumerate() {
        if l.voiced(p) {
            let px = x + 2 + i as i32 * (w - 4) / 300;
            let py = y + h / 2 - ((p.pitch - mid) / span * (h - 6) as f32) as i32;
            f.fill_rect(px, py - 1, 2, 2, TEXT);
        }
    }
}

/// How loud it is over the quietest stretch, 0 to 40 dB.
fn level(f: &mut Frame, l: &Listener, x: i32, y: i32, w: i32) {
    let now = l.frames().last().map_or(0.0, |p| p.power);
    let quiet = l.quiet().unwrap_or(now);
    bar(f, x, y - 3, w, (db(now.max(1.0)) - db(quiet)) / 40.0, if l.frames().last().is_some_and(|p| l.voiced(p)) { OK } else { FAINT });
}

fn bar(f: &mut Frame, x: i32, y: i32, w: i32, frac: f32, c: Rgb) {
    f.rrect(x, y, w, 4, 2, RAISED);
    let fill = (frac.clamp(0.0, 1.0) * w as f32) as i32;
    if fill > 0 {
        f.rrect(x, y, fill.max(4), 4, 2, c);
    }
}

/// A power in decibels.
fn db(p: f32) -> f32 {
    3.010_3 * hum::log2(p.max(1.0))
}

fn hz(pitch: f32) -> u32 {
    let m = pitch as u8;
    let frac = pitch - m as f32;
    (hum::hz(m) * (1.0 + frac * 0.059_463)) as u32
}

/// The notes heard, as bars: time across, pitch up.
fn roll(f: &mut Frame, notes: &[Note], x: i32, y: i32, w: i32, h: i32) {
    f.rrect(x, y, w, h, 4, SURFACE);
    let (Some(first), Some(last)) = (notes.first(), notes.last()) else { return };
    let (t0, t1) = (first.start as i32, (last.end as i32).max(first.start as i32 + 1));
    let lo = notes.iter().map(|n| n.pitch).fold(f32::MAX, f32::min);
    let hi = notes.iter().map(|n| n.pitch).fold(f32::MIN, f32::max);
    let (mid, span) = (0.5 * (lo + hi), (hi - lo).max(8.0) + 2.0);
    for n in notes {
        let x0 = x + 3 + (n.start as i32 - t0) * (w - 6) / (t1 - t0);
        let x1 = x + 3 + (n.end as i32 - t0) * (w - 6) / (t1 - t0);
        let py = y + h / 2 - ((n.pitch - mid) / span * (h - 8) as f32) as i32;
        f.rrect(x0, py - 2, (x1 - x0 - 1).max(3), 4, 2, TEXT);
    }
}

/// The code as marks, a note each: a dot for the first, then how each note moves (an arrow, with
/// a second head for a leap, a bar for the same), and a line under each long note. Red where it
/// differs from `against`. As wide as the row allows, up to 16 px a note.
fn tokens(f: &mut Frame, c: &Code, x: i32, y: i32, against: Option<&Code>) {
    let (steps, n) = (c.steps(), c.notes());
    let cw = (TW / n.max(1) as i32).clamp(9, 16);
    let a = if cw >= 13 { 6 } else { 4 };
    let theirs = against.map(|a| a.steps());
    for t in 0..n {
        let mv = if t == 0 { None } else { Some(steps[t - 1].0) };
        let long = t + 1 < n && steps[t].1;
        let differs = theirs.is_some_and(|o| {
            let m = if t == 0 { o.is_empty() } else { o.get(t - 1).map(|s| s.0) != mv };
            let l = t + 1 < n && o.get(t).map(|s| s.1) != Some(long);
            m || l
        });
        let ink = if differs { BAD } else { TEXT };
        let (cx, cy) = (x + cw * t as i32 + cw / 2, y + a + 3);
        match mv {
            None => f.disc(cx, cy, 2, ink),
            Some(Move::Same) => f.fill_rect(cx - 3, cy - 1, 7, 2, ink),
            Some(m) => {
                let s = if matches!(m, Move::Up | Move::UpLeap) { -1 } else { 1 };
                let tip = cy + s * a;
                f.line(cx, cy - s * a, cx, tip, 2, ink);
                f.line(cx, tip, cx - 3, tip - s * 3, 2, ink);
                f.line(cx, tip, cx + 3, tip - s * 3, 2, ink);
                if matches!(m, Move::UpLeap | Move::DownLeap) {
                    f.line(cx, tip - s * 4, cx - 3, tip - s * 7, 2, ink);
                    f.line(cx, tip - s * 4, cx + 3, tip - s * 7, 2, ink);
                }
            }
        }
        if long {
            f.fill_rect(cx - 3, cy + a + 4, 7, 2, ink);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hum::Tones;
    use crate::screens::H;

    /// The tune, as the mic sends it: an opening's own tones stand in for a voice. 32 kHz, two
    /// slots, the second dead, with a quiet room before and after.
    fn mic_bytes(notes: &'static [(u8, u16)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut seed = 1u32;
        let mut push = |x: i16| {
            for _ in 0..2 {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let room = (seed >> 29) as i16 - 4;
                out.extend_from_slice(&(x + room).to_le_bytes());
                out.extend_from_slice(&0i16.to_le_bytes());
            }
        };
        (0..8000).for_each(|_| push(0));
        Tones::new(notes).for_each(&mut push);
        (0..36000).for_each(|_| push(0));
        out
    }

    const TUNE: &[(u8, u16)] = &[(60, 300), (62, 300), (64, 600), (64, 300), (67, 300), (65, 600), (62, 300), (60, 900)];
    const OTHER: &[(u8, u16)] = &[(60, 300), (62, 300), (64, 600), (64, 300), (67, 300), (69, 600), (62, 300), (60, 900)];

    /// One go: the page plays, the mic hears `notes`, and it ends by itself.
    fn go(h: &mut Hum, notes: &'static [(u8, u16)]) {
        assert!(matches!(h.page, Page::Play(_)), "{:?}", h.page);
        h.played();
        assert!(h.listening());
        for chunk in mic_bytes(notes).chunks(4092) {
            h.feed(chunk);
            h.input(None, 0);
            if !h.listening() {
                break;
            }
        }
        assert!(!h.listening(), "still listening");
    }

    fn press(h: &mut Hum, k: Key) {
        h.input(Some(k), 0);
        h.input(None, 0);
    }

    #[test]
    fn set_and_try() {
        let mut h = Hum::new();
        // no tune yet: Try does nothing
        press(&mut h, Key::Down);
        press(&mut h, Key::Enter);
        assert_eq!(h.page, Page::Home);
        press(&mut h, Key::Up);
        press(&mut h, Key::Enter);
        assert_eq!(h.page, Page::Pick);
        press(&mut h, Key::Right);
        press(&mut h, Key::Enter);
        for i in 1..=3 {
            assert_eq!(h.page, Page::Play(Round::Set(i)));
            go(&mut h, TUNE);
            if i < 3 {
                assert_eq!(h.page, Page::Heard(Round::Set(i)));
                assert_eq!(h.n, TUNE.len());
                press(&mut h, Key::Enter);
            }
        }
        assert_eq!(h.page, Page::Set);
        assert_eq!(h.tune.map(|t| (t.0, t.1.notes())), Some((1, TUNE.len())));
        press(&mut h, Key::Enter);
        assert_eq!((h.page, h.sel), (Page::Home, 1));

        // the tune, then another
        press(&mut h, Key::Enter);
        assert_eq!(h.opening(), OPENINGS[1].notes);
        go(&mut h, TUNE);
        assert_eq!(h.page, Page::Match(true));
        press(&mut h, Key::Enter);
        press(&mut h, Key::Enter);
        go(&mut h, OTHER);
        assert_eq!(h.page, Page::Match(false));
    }

    #[test]
    fn rounds_that_differ_set_nothing() {
        let mut h = Hum::new();
        press(&mut h, Key::Enter);
        press(&mut h, Key::Enter);
        go(&mut h, TUNE);
        press(&mut h, Key::Enter);
        go(&mut h, TUNE);
        press(&mut h, Key::Enter);
        go(&mut h, OTHER);
        assert_eq!(h.page, Page::Differ);
        assert!(h.tune.is_none());
        press(&mut h, Key::Enter);
        assert_eq!(h.page, Page::Pick);
    }

    #[test]
    fn too_many_notes() {
        const MANY: &[(u8, u16)] = &[
            (60, 250), (64, 250), (60, 250), (64, 250), (60, 250), (64, 250), (60, 250), (64, 250), (60, 250), (64, 250),
            (60, 250), (64, 250), (60, 250), (64, 250), (60, 250), (64, 250), (60, 250), (64, 250), (60, 250), (64, 250),
            (60, 250), (64, 250), (60, 250), (64, 250), (60, 250), (64, 250),
        ];
        let mut h = Hum::new();
        press(&mut h, Key::Enter);
        press(&mut h, Key::Enter);
        go(&mut h, MANY);
        assert_eq!((h.page, h.n), (Page::Long(Round::Set(1)), MANY.len()));
        let mut buf = [0u8; (W * crate::screens::H * 2) as usize];
        h.draw(&mut Frame::new(&mut buf, W, crate::screens::H), 0);
    }

    #[test]
    fn too_few_notes_and_del() {
        let mut h = Hum::new();
        press(&mut h, Key::Enter);
        press(&mut h, Key::Enter);
        go(&mut h, OPENINGS[0].notes); // 4 notes
        assert_eq!(h.page, Page::Short(Round::Set(1)));
        press(&mut h, Key::Enter);
        assert_eq!(h.page, Page::Play(Round::Set(1)));
        // Del backs out from anywhere, with one press
        h.played();
        press(&mut h, Key::Del);
        assert_eq!(h.page, Page::Home);
        assert!(!h.listening());
    }

    // The mic page listens until Del, and a slot chosen by hand stays chosen.
    #[test]
    fn the_mic_page() {
        let mut h = Hum::new();
        press(&mut h, Key::Down);
        press(&mut h, Key::Down);
        press(&mut h, Key::Enter);
        assert_eq!(h.page, Page::Mic);
        for chunk in mic_bytes(TUNE).chunks(4092) {
            h.feed(chunk);
            h.input(None, 0);
        }
        assert_eq!(h.page, Page::Mic);
        assert_eq!(h.mic().0, Some(0));
        press(&mut h, Key::Right);
        assert_eq!(h.mic().0, Some(1));
        press(&mut h, Key::Del);
        assert_eq!(h.page, Page::Home);
        assert_eq!(h.slot, Some(1));
    }

    // Every page draws inside the screen without panicking.
    #[test]
    fn every_page_draws() {
        let mut buf = [0u8; (W * H * 2) as usize];
        let mut f = Frame::new(&mut buf, W, H);
        let mut h = Hum::new();
        press(&mut h, Key::Enter);
        press(&mut h, Key::Enter);
        go(&mut h, TUNE);
        for page in [Page::Home, Page::Pick, Page::Play(Round::Set(2)), Page::Listen(Round::Try), Page::Heard(Round::Set(1)),
                     Page::Short(Round::Try), Page::Long(Round::Set(3)), Page::Set, Page::Differ, Page::Match(true),
                     Page::Match(false), Page::Mic] {
            h.page = page;
            h.draw(&mut f, 0);
        }
    }
}
