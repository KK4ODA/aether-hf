//! Whether the radio seems to transmit when the modem keys it.
//!
//! A radio that transmits changes what it sends the sound card: it mutes its receiver (the
//! FTDX10 at KK4ODA: −23 dBFS of band noise to −47 while keyed), or it monitors its own signal
//! (louder than anything heard). A radio that the keying line does not reach goes on receiving,
//! and the audio while "keyed" reads as the band does. On 2026-11-02 the house station's `VarAC`
//! profile keyed RTS on the FTDX10's CAT port, which the radio ignores: every acceptance went to
//! a dead key, the house sat in sessions the truck never heard of, and nothing said why. Its
//! receive audio while keyed read −27 dBFS, the band's own level before and after.
//!
//! So: the level of the radio's audio while keyed, from [`KEY_SETTLE_S`] after the key (the
//! radio's switch-over and the card's capture lag), against the levels heard while receiving
//! over the last [`HEARD_WINDOW_S`]. Both are measured in [`BAND_HZ`], 1–2.5 kHz, where a
//! receiver passes the band (both airs' signals lie inside it) and a radio's mute shows. Measured
//! over the whole audio, the mobile's TS-480 on 2026-11-12 was said not to transmit while every
//! frame it sent was decoded: its receiver drops 16–25 dB in that band, but the hum its interface
//! adds at 60, 120, 180 and 300 Hz is no part of the receiver and no mute removes it, so the whole
//! level fell only 2–13 dB, under [`MUTED_BELOW_DB`] in 12 of 62 transmissions. In the band, none,
//! the least fall 11.9 dB; the dead key of 2026-11-02 reads the same in it as over the whole
//! audio, the band's own level. Inside their range — not [`MUTED_BELOW_DB`] under the
//! quiet end, not [`MONITOR_ABOVE_DB`] over the loud end — the transmission changed nothing.
//! [`UNCHANGED_RUN`] such transmissions in a row are said once, and again after [`SAY_AGAIN_S`];
//! a keying test (*Key 1 s*) is judged on its own, since the operator pressed it to find out.
//! A report only: nothing is refused on it.
//!
//! A radio may send its own signal back at the band's level, and then the level cannot tell. The
//! portable's Lab599 TX-500 on 2026-11-24 mutes its receiver while keyed (outside its signal the
//! audio fell 28–54 dB) and sends the transmitted audio back to the sound card, at −19 to −23 dBFS
//! in the band, where a busy 40 m read −20 to −29 while receiving: three transmissions in a row
//! read "unchanged", and the warning came while every frame was decoded at the other end. What
//! came back was the station's own signal: the frames decoded from it and rendered again matched
//! it at |r| 0.98–0.99, about 0.26 s after the key, the sign flipping between keyings. So a
//! transmission the level calls unchanged is checked for that ([`monitored`]): the audio heard
//! while keyed against what was played, at every delay up to [`MONITOR_LOOP_S`]. A key that does
//! nothing leaves the band's own noise, which no delay of the played signal matches (ADR-0136).

use std::collections::VecDeque;

use rustfft::{FftPlanner, num_complex::Complex64};

/// The band the watch measures, Hz.
pub const BAND_HZ: (f64, f64) = (1000.0, 2500.0);

/// The start of a keyed span not judged: the radio's switch to transmit and the sound card's
/// capture lag, which still delivers audio from before the key.
pub const KEY_SETTLE_S: f64 = 0.3;
/// The least keyed audio, after [`KEY_SETTLE_S`], a transmission is judged on: a keying test
/// of a second leaves 0.7 s, a 2 300 Hz acknowledgement less than this.
pub const MIN_KEYED_S: f64 = 0.5;
/// How far back the levels heard while receiving go.
const HEARD_WINDOW_S: f64 = 60.0;
/// The least receive audio in that window a judgement needs.
const MIN_HEARD_S: f64 = 5.0;
/// A receiver muted for transmit reads at least this far under the quiet end of what it hears.
pub const MUTED_BELOW_DB: f64 = 6.0;
/// A radio that monitors its own signal reads at least this far over the loud end.
pub const MONITOR_ABOVE_DB: f64 = 3.0;
/// Transmissions in a row that changed nothing before it is said.
pub const UNCHANGED_RUN: u32 = 3;
/// How long before it is said again while it goes on.
const SAY_AGAIN_S: f64 = 600.0;
/// How much of the audio heard while keyed, from [`KEY_SETTLE_S`], is checked against what was
/// played.
pub const MONITOR_LISTEN_S: f64 = 1.0;
/// The longest delay from playing a sample to hearing it back: the card's output and input
/// latencies and the radio's own (the TX-500's 0.26 s, with room).
pub const MONITOR_LOOP_S: f64 = 0.8;
/// The least correlation, either sign, that makes the audio heard while keyed the station's own
/// signal. The TX-500 read 0.98–0.99; the band's noise against a second of a played signal reads
/// about 0.1 at its best delay.
pub const MONITOR_CORRELATION: f64 = 0.5;

/// What one transmission's receive audio said.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    /// The median level while keyed, dBFS.
    pub keyed: f64,
    /// The quiet end of what was heard while receiving (its 10th percentile), dBFS.
    pub heard_low: f64,
    /// The loud end (its 90th percentile), dBFS.
    pub heard_high: f64,
    /// What the radio sent back while keyed was the station's own signal ([`monitored`]).
    pub monitored: bool,
}

impl Reading {
    /// Whether the radio's audio while keyed reads as it does while receiving, and is not the
    /// station's own signal coming back.
    #[must_use]
    pub fn unchanged(&self) -> bool {
        !self.monitored
            && self.keyed >= self.heard_low - MUTED_BELOW_DB
            && self.keyed <= self.heard_high + MONITOR_ABOVE_DB
    }
}

/// The levels heard and keyed, and how many transmissions in a row changed nothing.
#[derive(Debug, Clone, Default)]
pub struct KeyedAudio {
    /// Blocks heard while receiving and not deaf: (when, seconds, level dBFS), oldest first.
    heard: VecDeque<(f64, f64, f64)>,
    /// When the key went up, for the transmission on the air.
    keyed_at: Option<f64>,
    /// Blocks while keyed, from [`KEY_SETTLE_S`] after the key: (seconds, level dBFS).
    keyed: Vec<(f64, f64)>,
    /// The transmission on the air is a keying test.
    test: bool,
    /// The next transmission keyed will be one.
    test_next: bool,
    /// The next transmission with a reading is judged on its own: the first since a host
    /// program attached, where the keying the program and the station were each set up for
    /// meets.
    alone_next: bool,
    /// Transmissions in a row whose audio changed nothing.
    run: u32,
    /// When it was last said.
    said_at: Option<f64>,
    /// The filter for [`BAND_HZ`], at the rate it was made for: its state runs on from block to
    /// block.
    band: Option<(usize, InBand)>,
    /// The start of what the transmission played, its lead's silence first, at `rate`.
    played: Vec<f32>,
    /// What the radio sent back while keyed, from [`KEY_SETTLE_S`], at `rate`.
    heard_keyed: Vec<f32>,
    /// The sample rate of those two.
    rate: usize,
}

impl KeyedAudio {
    /// The level of a block of the radio's audio at `rate` samples a second, in [`BAND_HZ`],
    /// dBFS of a full-scale sine's power (a full-scale sine reads −3).
    pub fn level_db(&mut self, audio: &[f32], rate: usize) -> f64 {
        if self.band.as_ref().is_none_or(|(at, _)| *at != rate) {
            self.band = Some((rate, InBand::new(rate as f64)));
        }
        let Some((_, band)) = self.band.as_mut() else {
            return f64::NEG_INFINITY;
        };
        if audio.is_empty() {
            return f64::NEG_INFINITY;
        }
        let power = audio
            .iter()
            .map(|&x| {
                let y = band.filter(f64::from(x));
                y * y
            })
            .sum::<f64>()
            / audio.len() as f64;
        10.0 * power.max(1e-12).log10()
    }

    /// One block of the radio's audio, at `now`: `seconds` long at `level_db` dBFS, while the
    /// station was transmitting or not, and deaf to its own tail or not.
    pub fn block(&mut self, now: f64, seconds: f64, level_db: f64, transmitting: bool, deaf: bool) {
        if !level_db.is_finite() || seconds <= 0.0 {
            return;
        }
        if transmitting {
            if self
                .keyed_at
                .is_some_and(|at| now - seconds >= at + KEY_SETTLE_S)
            {
                self.keyed.push((seconds, level_db));
            }
            return;
        }
        if deaf {
            return;
        }
        self.heard.push_back((now, seconds, level_db));
        while self
            .heard
            .front()
            .is_some_and(|&(at, _, _)| now - at > HEARD_WINDOW_S)
        {
            self.heard.pop_front();
        }
    }

    /// The next transmission is a keying test, judged on its own.
    pub fn expect_test(&mut self) {
        self.test_next = true;
    }

    /// The next transmission that can be judged is said on its own when it changed nothing,
    /// not after [`UNCHANGED_RUN`] in a row.
    pub fn judge_next_alone(&mut self) {
        self.alone_next = true;
    }

    /// The key went up at `now`.
    pub fn keyed(&mut self, now: f64) {
        self.keyed_at = Some(now);
        self.keyed.clear();
        self.played.clear();
        self.heard_keyed.clear();
        self.test = std::mem::take(&mut self.test_next);
    }

    /// What the transmission keyed now plays, from its first sample (its lead's silence), at
    /// `rate`: what the radio may send back. Only as much as a check needs is kept.
    pub fn playing(&mut self, audio: impl IntoIterator<Item = f32>, rate: usize) {
        let keep = ((KEY_SETTLE_S + MONITOR_LISTEN_S + MONITOR_LOOP_S) * rate as f64) as usize;
        self.played.clear();
        self.played.extend(audio.into_iter().take(keep));
        self.rate = rate;
    }

    /// A block of what the radio sends while keyed, at `now`, `seconds` long: kept from
    /// [`KEY_SETTLE_S`] after the key for [`MONITOR_LISTEN_S`].
    pub fn sound(&mut self, now: f64, seconds: f64, audio: &[f32]) {
        let Some(at) = self.keyed_at else {
            return;
        };
        if now - seconds < at + KEY_SETTLE_S || self.rate == 0 {
            return;
        }
        let keep = (MONITOR_LISTEN_S * self.rate as f64) as usize;
        let room = keep.saturating_sub(self.heard_keyed.len());
        self.heard_keyed.extend(audio.iter().take(room));
    }

    /// The key came down on the watchdog or a failing interface: no reading.
    pub fn abandon(&mut self) {
        self.keyed_at = None;
        self.keyed.clear();
        self.played.clear();
        self.heard_keyed.clear();
        self.test = false;
    }

    /// The key came down at `now`: what to say of it, if anything. A keying test says its
    /// reading whatever it is; any other transmission only once [`UNCHANGED_RUN`] in a row
    /// changed nothing.
    pub fn released(&mut self, now: f64) -> Option<Said> {
        self.keyed_at = None;
        let test = std::mem::take(&mut self.test);
        let mut reading = self.reading()?;
        // only a reading the level calls unchanged is checked for the station's own signal
        if reading.unchanged() {
            reading.monitored =
                monitored(&self.heard_keyed, &self.played, self.rate) >= MONITOR_CORRELATION;
        }
        self.played.clear();
        self.heard_keyed.clear();
        let unchanged = reading.unchanged();
        if test {
            return Some(Said::Test(reading));
        }
        let alone = std::mem::take(&mut self.alone_next);
        if !unchanged {
            self.run = 0;
            return None;
        }
        self.run += 1;
        if !alone
            && (self.run < UNCHANGED_RUN || self.said_at.is_some_and(|at| now - at < SAY_AGAIN_S))
        {
            return None;
        }
        self.said_at = Some(now);
        Some(Said::Unchanged(reading, self.run))
    }

    /// The reading of the transmission just keyed, when there is enough of both to judge.
    fn reading(&self) -> Option<Reading> {
        let keyed_s: f64 = self.keyed.iter().map(|&(s, _)| s).sum();
        let heard_s: f64 = self.heard.iter().map(|&(_, s, _)| s).sum();
        if keyed_s < MIN_KEYED_S || heard_s < MIN_HEARD_S {
            return None;
        }
        let mut keyed: Vec<f64> = self.keyed.iter().map(|&(_, db)| db).collect();
        let mut heard: Vec<f64> = self.heard.iter().map(|&(_, _, db)| db).collect();
        keyed.sort_by(f64::total_cmp);
        heard.sort_by(f64::total_cmp);
        let at = |v: &[f64], share: f64| v[((v.len() - 1) as f64 * share).round() as usize];
        Some(Reading {
            keyed: at(&keyed, 0.5),
            heard_low: at(&heard, 0.1),
            heard_high: at(&heard, 0.9),
            monitored: false,
        })
    }
}

/// How closely what the radio sent back while keyed (`heard`, from [`KEY_SETTLE_S`] after the
/// key) matches what was played (`played`, from the transmission's first sample), at the best
/// delay up to [`MONITOR_LOOP_S`]: the largest correlation of either sign, both in [`BAND_HZ`].
/// 0 when there is too little of either.
#[must_use]
pub fn monitored(heard: &[f32], played: &[f32], rate: usize) -> f64 {
    if rate == 0 || heard.len() < rate / 10 || played.len() < rate / 10 {
        return 0.0;
    }
    // the played signal behind a loop's worth of silence: a sample played at `j` is heard at
    // `j + delay`, and with the silence in front every delay up to the loop's is a lag of 0 or
    // more
    let lead = (MONITOR_LOOP_S * rate as f64) as usize;
    let settle = (KEY_SETTLE_S * rate as f64) as usize;
    let heard_band = in_band(heard.iter().map(|&x| f64::from(x)), rate, heard.len());
    let mut padded = vec![0.0; lead];
    padded.extend(played.iter().map(|&x| f64::from(x)));
    let played_band = in_band(padded.iter().copied(), rate, padded.len());
    let size = (heard_band.len() + played_band.len()).next_power_of_two();
    let mut planner = FftPlanner::new();
    let forward = planner.plan_fft_forward(size);
    let inverse = planner.plan_fft_inverse(size);
    let mut fa: Vec<Complex64> = heard_band.iter().map(|&x| Complex64::new(x, 0.0)).collect();
    fa.resize(size, Complex64::new(0.0, 0.0));
    let mut fb: Vec<Complex64> = played_band
        .iter()
        .map(|&x| Complex64::new(x, 0.0))
        .collect();
    fb.resize(size, Complex64::new(0.0, 0.0));
    forward.process(&mut fa);
    forward.process(&mut fb);
    // cross[k] = sum_i heard[i] played[i + k]
    let mut cross: Vec<Complex64> = fb.iter().zip(&fa).map(|(y, x)| y * x.conj()).collect();
    inverse.process(&mut cross);
    let energy_heard: f64 = heard_band.iter().map(|x| x * x).sum();
    if energy_heard <= 0.0 {
        return 0.0;
    }
    let mut prefix = Vec::with_capacity(played_band.len() + 1);
    prefix.push(0.0);
    for x in &played_band {
        prefix.push(prefix[prefix.len() - 1] + x * x);
    }
    // heard sample `i` is `settle + i` after the key; played sample `j` (in the padded signal) is
    // heard at `j - lead + delay`: the lag `k = j - i` runs from `settle` (a delay of a whole
    // loop) to `settle + lead` (none)
    let mut best: f64 = 0.0;
    for k in settle..=(settle + lead) {
        let end = (k + heard_band.len()).min(played_band.len());
        if k >= end {
            break;
        }
        let energy_played = prefix[end] - prefix[k];
        if energy_played <= 0.0 {
            continue;
        }
        let r = cross[k].re / size as f64 / (energy_heard * energy_played).sqrt();
        best = best.max(r.abs());
    }
    best
}

/// `len` samples of `audio` in [`BAND_HZ`], through the watch's own filter.
fn in_band(audio: impl Iterator<Item = f64>, rate: usize, len: usize) -> Vec<f64> {
    let mut band = InBand::new(rate as f64);
    let mut out = Vec::with_capacity(len);
    out.extend(audio.map(|x| band.filter(x)));
    out
}

/// One second-order section, transposed direct form II.
#[derive(Debug, Clone)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    /// A low-pass (`high` false) or high-pass section at `corner_hz` with quality `q`, by the
    /// bilinear transform with the corner prewarped (R. Bristow-Johnson's *Audio EQ Cookbook*).
    fn new(rate: f64, corner_hz: f64, q: f64, high: bool) -> Self {
        let w = std::f64::consts::TAU * corner_hz / rate;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * q);
        let a0 = 1.0 + alpha;
        let b = if high {
            let half = f64::midpoint(1.0, cos);
            [half, -(1.0 + cos), half]
        } else {
            [(1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0]
        };
        Self {
            b: [b[0] / a0, b[1] / a0, b[2] / a0],
            a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
            z: [0.0; 2],
        }
    }

    fn filter(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// [`BAND_HZ`]: a fourth-order Butterworth high-pass at its lower edge and low-pass at its upper
/// one. Hum at 300 Hz is about 40 dB down, at 60 Hz past 80.
#[derive(Debug, Clone)]
struct InBand {
    sections: [Biquad; 4],
}

impl InBand {
    fn new(rate: f64) -> Self {
        // a fourth-order Butterworth is two sections of these qualities: 1 / (2 cos(π/8)) and
        // 1 / (2 cos(3π/8))
        let q = [0.541_196_100_146_197, 1.306_562_964_876_376_5];
        let (low, high) = BAND_HZ;
        Self {
            sections: [
                Biquad::new(rate, low, q[0], true),
                Biquad::new(rate, low, q[1], true),
                Biquad::new(rate, high, q[0], false),
                Biquad::new(rate, high, q[1], false),
            ],
        }
    }

    fn filter(&mut self, x: f64) -> f64 {
        self.sections.iter_mut().fold(x, |y, s| s.filter(y))
    }
}

/// What a release has to say.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Said {
    /// A keying test's reading.
    Test(Reading),
    /// This many transmissions in a row changed nothing; the last one's reading.
    Unchanged(Reading, u32),
}

impl Said {
    /// The log's line: its name (`keyed` is a warning, `keying` not) and its words.
    #[must_use]
    pub fn line(&self) -> (&'static str, String) {
        match *self {
            Self::Test(reading) if !reading.unchanged() => (
                "keying",
                format!(
                    "the keying test changed the radio's receive audio ({:.0} dBFS keyed; {:.0} \
                     to {:.0} dBFS receiving, in 1–2.5 kHz): the radio transmits",
                    reading.keyed, reading.heard_low, reading.heard_high
                ),
            ),
            Self::Test(reading) => (
                "keyed",
                format!(
                    "the keying test did not change the radio's receive audio ({:.0} dBFS keyed; \
                     {:.0} to {:.0} dBFS receiving, in 1–2.5 kHz): the radio may not have keyed. \
                     Check the \
                     keying in Setup step 2 and that the radio's own keying setting matches it",
                    reading.keyed, reading.heard_low, reading.heard_high
                ),
            ),
            Self::Unchanged(reading, run) => (
                "keyed",
                format!(
                    "the radio's receive audio did not change while it was keyed, {run} \
                     transmission{} in a row ({:.0} dBFS keyed; {:.0} to {:.0} dBFS receiving, \
                     in 1–2.5 kHz): \
                     the radio may not be transmitting. Check the keying in Setup step 2, then \
                     press Key 1 s under Keying and drive",
                    if run == 1 { "" } else { "s" },
                    reading.keyed,
                    reading.heard_low,
                    reading.heard_high
                ),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ten seconds of receive audio at `low`…`high` dBFS in 20 ms blocks, then a keyed span of
    /// `keyed_s` at `keyed_db`, released; what the release said.
    fn transmission(ka: &mut KeyedAudio, t: &mut f64, keyed_s: f64, keyed_db: f64) -> Option<Said> {
        let block = 0.02;
        for i in 0..500 {
            // band noise with now and then a signal on it
            let db = if i % 10 == 0 {
                -22.0
            } else {
                -30.0 + f64::from(i % 3)
            };
            *t += block;
            ka.block(*t, block, db, false, false);
        }
        ka.keyed(*t);
        let end = *t + keyed_s;
        while *t < end {
            *t += block;
            ka.block(*t, block, keyed_db, true, false);
        }
        ka.released(*t)
    }

    #[test]
    fn a_radio_that_mutes_its_receiver_while_keyed_transmits() {
        // the FTDX10 on CAT keying, 2026-11-02: −23 dBFS before, −47 keyed
        let mut ka = KeyedAudio::default();
        let mut t = 0.0;
        for _ in 0..5 {
            assert_eq!(transmission(&mut ka, &mut t, 5.6, -47.4), None);
        }
    }

    #[test]
    fn a_radio_that_monitors_its_own_signal_transmits() {
        let mut ka = KeyedAudio::default();
        let mut t = 0.0;
        for _ in 0..5 {
            assert_eq!(transmission(&mut ka, &mut t, 5.6, -8.0), None);
        }
    }

    #[test]
    fn a_keying_line_the_radio_ignores_is_said_after_three_transmissions_and_not_again_soon() {
        // the VarAC profile's RTS on the FTDX10's CAT port, 2026-11-02: −27 dBFS keyed, the
        // band's own level
        let mut ka = KeyedAudio::default();
        let mut t = 0.0;
        assert_eq!(transmission(&mut ka, &mut t, 5.6, -27.0), None);
        assert_eq!(transmission(&mut ka, &mut t, 5.6, -27.0), None);
        let said = transmission(&mut ka, &mut t, 5.6, -27.0).expect("said");
        let Said::Unchanged(reading, 3) = said else {
            panic!("{said:?}")
        };
        assert!(reading.unchanged());
        let (name, line) = said.line();
        assert_eq!(name, "keyed");
        assert!(line.contains("may not be transmitting"), "{line}");
        // not every transmission after
        for _ in 0..10 {
            assert_eq!(transmission(&mut ka, &mut t, 5.6, -27.0), None);
        }
        // one that changes it starts the count over
        assert_eq!(transmission(&mut ka, &mut t, 5.6, -47.0), None);
        assert_eq!(ka.run, 0);
    }

    #[test]
    fn the_first_transmission_after_a_program_attaches_is_judged_on_its_own() {
        // a program keying the radio, or the station keying it for the program: where the two
        // setups meet, a dead key is said at the first transmission, not the third
        let mut ka = KeyedAudio::default();
        let mut t = 0.0;
        ka.judge_next_alone();
        // too short to judge: the check waits for one that can be
        assert_eq!(transmission(&mut ka, &mut t, 0.2, -27.0), None);
        let said = transmission(&mut ka, &mut t, 5.6, -27.0).expect("said at once");
        assert!(matches!(said, Said::Unchanged(_, 1)), "{said:?}");
        let (name, line) = said.line();
        assert_eq!(name, "keyed");
        assert!(line.contains("1 transmission in a row"), "{line}");
        // a radio that does transmit says nothing, and the next is counted as before
        ka.judge_next_alone();
        assert_eq!(transmission(&mut ka, &mut t, 5.6, -47.0), None);
        assert_eq!(transmission(&mut ka, &mut t, 5.6, -27.0), None);
    }

    #[test]
    fn a_keying_test_is_judged_on_its_own_and_a_short_one_not_at_all() {
        let mut ka = KeyedAudio::default();
        let mut t = 0.0;
        ka.expect_test();
        let said = transmission(&mut ka, &mut t, 1.0, -27.0).expect("judged");
        assert!(matches!(said, Said::Test(_)));
        assert_eq!(said.line().0, "keyed");
        ka.expect_test();
        let said = transmission(&mut ka, &mut t, 1.0, -47.0).expect("judged");
        assert_eq!(said.line().0, "keying");
        assert!(said.line().1.ends_with("the radio transmits"), "{said:?}");
        // a key shorter than the settle and the least judged: nothing to say
        ka.expect_test();
        assert_eq!(transmission(&mut ka, &mut t, 0.6, -27.0), None);
    }

    #[test]
    fn nothing_is_judged_without_enough_heard_first() {
        let mut ka = KeyedAudio::default();
        ka.keyed(0.0);
        for i in 1..=200 {
            ka.block(f64::from(i) * 0.02, 0.02, -27.0, true, false);
        }
        assert_eq!(ka.released(4.0), None);
    }

    /// A tone of `amplitude` at `hz` through the watch's band, settled: its level, dBFS.
    fn tone_level(hz: f64, amplitude: f64) -> f64 {
        let rate = 48_000_usize;
        let mut ka = KeyedAudio::default();
        let block = |start: usize, n: usize| -> Vec<f32> {
            (start..start + n)
                .map(|i| {
                    let t = i as f64 / rate as f64;
                    (amplitude * (std::f64::consts::TAU * hz * t).sin()) as f32
                })
                .collect()
        };
        let _ = ka.level_db(&block(0, rate / 2), rate);
        ka.level_db(&block(rate / 2, rate / 10), rate)
    }

    #[test]
    fn the_band_passes_the_signals_and_not_an_interfaces_hum() {
        // a sine of amplitude 0.1 has the power of -23 dBFS
        let inside = tone_level(1500.0, 0.1);
        assert!((inside + 23.0).abs() < 0.5, "1.5 kHz read {inside:.1}");
        for edge in [1000.0, 2500.0] {
            let at = tone_level(edge, 0.1);
            assert!((at - (inside - 3.0)).abs() < 1.0, "{edge} Hz read {at:.1}");
        }
        let hum = tone_level(300.0, 0.1);
        assert!(hum < inside - 35.0, "300 Hz read {hum:.1}");
        let mains = tone_level(60.0, 0.1);
        assert!(mains < inside - 70.0, "60 Hz read {mains:.1}");
    }

    /// `seconds` of white noise of `noise` amplitude and hum (60, 120, 180 and 300 Hz) of `hum`
    /// each, from `start` s, deterministically.
    fn noise_and_hum(seed: &mut u64, start: f64, seconds: f64, noise: f64, hum: f64) -> Vec<f32> {
        let rate = 48_000.0;
        (0..(seconds * rate) as usize)
            .map(|i| {
                *seed ^= *seed << 13;
                *seed ^= *seed >> 7;
                *seed ^= *seed << 17;
                let u = (*seed >> 11) as f64 / (1_u64 << 53) as f64 * 2.0 - 1.0;
                let t = start + i as f64 / rate;
                let h: f64 = [60.0, 120.0, 180.0, 300.0]
                    .iter()
                    .map(|f| (std::f64::consts::TAU * f * t).sin())
                    .sum();
                (noise * u + hum * h) as f32
            })
            .collect()
    }

    #[test]
    fn hum_the_keying_cannot_mute_does_not_hide_a_muted_receiver() {
        // the mobile's TS-480 on 2026-11-12: its receiver mutes while keyed, the hum its interface
        // adds does not, and over the whole audio the hum held the level up — "may not be
        // transmitting" while every frame it sent was decoded. In the band the mute shows.
        let rate = 48_000_usize;
        let block_s = 0.02;
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let (mut in_band, mut whole) = (KeyedAudio::default(), KeyedAudio::default());
        let mut said_whole = None;
        let mut t = 0.0;
        for _ in 0..4 {
            for keyed in [false, true] {
                if keyed {
                    in_band.keyed(t);
                    whole.keyed(t);
                }
                let (span, noise) = if keyed { (5.6, 0.005) } else { (10.0, 0.05) };
                let end = t + span;
                while t < end {
                    let audio = noise_and_hum(&mut seed, t, block_s, noise, 0.05);
                    t += block_s;
                    let band_db = in_band.level_db(&audio, rate);
                    in_band.block(t, block_s, band_db, keyed, false);
                    let power = audio.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>()
                        / audio.len() as f64;
                    whole.block(t, block_s, 10.0 * power.log10(), keyed, false);
                }
            }
            assert_eq!(in_band.released(t), None, "in the band the radio transmits");
            said_whole = said_whole.or(whole.released(t));
        }
        // and over the whole audio, as before, it would have been said
        assert!(
            matches!(said_whole, Some(Said::Unchanged(_, 3))),
            "{said_whole:?}"
        );
    }

    /// A station's transmissions through a radio that sends the played signal back at `back`
    /// times its level, `delay_s` late and of `sign`, or — with `back` 0 — through a key that
    /// does nothing, the band's noise going on; receive audio of the same level between. What
    /// the releases said.
    fn monitored_session(back: f64, delay_s: f64, sign: f64) -> Vec<Option<Said>> {
        let rate = 48_000_usize;
        let block = 960_usize;
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut ka = KeyedAudio::default();
        let mut said = Vec::new();
        let mut t = 0.0;
        let delay = (delay_s * rate as f64) as usize;
        for _ in 0..4 {
            // ten seconds of the band, noise at about -27 dBFS in the watch's band
            for _ in 0..(10 * rate / block) {
                let audio = noise_and_hum(&mut seed, t, block as f64 / rate as f64, 0.06, 0.0);
                t += block as f64 / rate as f64;
                let db = ka.level_db(&audio, rate);
                ka.block(t, block as f64 / rate as f64, db, false, false);
            }
            // a transmission: a stand-in for a tone frame, a tone stepping across 1.3-1.7 kHz
            // every 40 ms, at about the band's level, after a 150 ms lead
            let len = 3 * rate;
            let lead = rate * 15 / 100;
            let played: Vec<f32> = (0..len)
                .map(|i| {
                    if i < lead {
                        return 0.0;
                    }
                    let symbol = (i - lead) / (rate / 25);
                    let hz = 1300.0 + 25.0 * ((symbol * 7) % 16) as f64;
                    (0.05 * (std::f64::consts::TAU * hz * i as f64 / rate as f64).sin()) as f32
                })
                .collect();
            ka.keyed(t);
            ka.playing(played.iter().copied(), rate);
            let mut i = 0;
            while i < len {
                let noise = noise_and_hum(&mut seed, t, block as f64 / rate as f64, 0.06, 0.0);
                let audio: Vec<f32> = (0..block)
                    .map(|k| {
                        let at = i + k;
                        let echo = if back > 0.0 && at >= delay {
                            sign * back * f64::from(played[at - delay])
                        } else {
                            0.0
                        };
                        // a muted receiver leaves little noise under the radio's own signal
                        let floor = if back > 0.0 { 0.05 } else { 1.0 };
                        (echo + floor * f64::from(noise[k])) as f32
                    })
                    .collect();
                t += block as f64 / rate as f64;
                i += block;
                let db = ka.level_db(&audio, rate);
                ka.block(t, block as f64 / rate as f64, db, true, false);
                ka.sound(t, block as f64 / rate as f64, &audio);
            }
            said.push(ka.released(t));
        }
        said
    }

    #[test]
    fn a_radio_that_sends_its_own_signal_back_at_the_bands_level_transmits() {
        // the TX-500 on 2026-11-24: its own signal back at -19 to -23 dBFS in the band, the band
        // at -20 to -29 dBFS — unchanged by the level, "may not be transmitting" after three
        // transmissions, while every frame was decoded at the other end. The audio was the
        // station's own signal, 0.26 s late, its sign flipping between keyings.
        for sign in [1.0, -1.0] {
            let said = monitored_session(1.0, 0.26, sign);
            assert!(said.iter().all(Option::is_none), "sign {sign}: {said:?}");
        }
    }

    #[test]
    fn a_key_that_does_nothing_is_still_said() {
        // the band's own noise goes on under a dead key: no delay of the played signal matches it
        let said = monitored_session(0.0, 0.26, 1.0);
        assert!(
            said.iter()
                .any(|s| matches!(s, Some(Said::Unchanged(r, _)) if !r.monitored)),
            "{said:?}"
        );
    }

    #[test]
    fn the_match_reads_the_signal_and_not_the_noise() {
        // the correlation at the best delay: near 1 for the signal sent back, small for noise
        let rate = 48_000_usize;
        let mut seed = 0x1234_5678_9abc_def0_u64;
        let played = noise_and_hum(&mut seed, 0.0, 2.1, 0.1, 0.0);
        let settle = (KEY_SETTLE_S * rate as f64) as usize;
        let delay = (0.4 * rate as f64) as usize;
        // heard from the settle, 0.4 s behind: its first tenth of a second is before playback began
        let back: Vec<f32> = (0..rate)
            .map(|i| (settle + i).checked_sub(delay).map_or(0.0, |j| -played[j]))
            .collect();
        let r = monitored(&back, &played, rate);
        assert!(r > 0.9, "the signal sent back read {r:.2}");
        let noise = noise_and_hum(&mut seed, 0.0, 1.0, 0.1, 0.0);
        let r = monitored(&noise, &played, rate);
        assert!(r < 0.2, "noise read {r:.2}");
    }
}
