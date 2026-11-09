//! The receive filter, read at the signal's edges (ADR-0105), and whether the audio clips.
//!
//! The radio's receive filter decides what of the other station's signal reaches the modem. Set
//! narrower than the signal, moved by an IF shift, or with its low cut set high, it takes an edge
//! of every frame away, and nothing in the modem says why the link runs slow. The station reads
//! the filter from the band's own noise: every half second that it listens it takes a spectrum,
//! leaves out the bins a signal holds, holds the look until no frame under it can still be
//! announced, divides it by its own level over the signal's span, and keeps it when that level
//! stands over the audio chain's own floor (6–10 kHz, where no radio passes audio). The profile
//! is each bin's median over the looks kept; the reading is how far each edge of the signal's
//! span lies under the profile's level there, and an edge [`LINE_DB`] or more down is cut.
//!
//! It replaced an estimate of the passband's width (`spectrum::PassbandMonitor`, beta.14 to
//! beta.144) that saw a cut and judged it wrongly: it drew the edge far down the filter's skirt,
//! tested the width, so a moved passband never warned, and needed whole windows of quiet
//! (`docs/research/receive-filter/`).
//!
//! [`Clipping`] (C1) is the share of the audio heard at full scale over the last ten seconds: a
//! warning of its own. The reading goes on while the audio clips; it read clipped audio right
//! everywhere it was tried.

use std::collections::VecDeque;

use aether_phy::WaveformParams;

use crate::spectrum::{Spectrum, TOP_HZ};

/// The warning's line: an edge of the signal this far under the passband's own level is cut. A
/// cut 10 dB down at an edge costs nothing on the harness and 20 dB costs; over a shallow chain
/// floor (15 dB under the band's noise) a 20 dB cut reads about 14 dB down and a 10 dB cut about
/// 9 (ADR-0105 §2.1).
pub const LINE_DB: f32 = 12.0;
/// How far either side of a bin the profile is smoothed, hertz: eight or nine bins, so one noisy
/// bin does not decide an edge.
pub const EDGE_HALF_HZ: f64 = 50.0;
/// Where the audio chain's own floor is read, hertz: above every radio's audio filter, below the
/// codec's anti-alias filter.
pub const FLOOR_BAND_HZ: (f64, f64) = (6000.0, 10_000.0);
/// Looks kept before there is a reading.
pub const LOOKS_NEEDED: usize = 12;
/// Looks each bin's median is read over: a minute at two a second, so a change of filter shows
/// within half of it.
pub const LOOKS_KEPT: usize = 120;
/// A bin this far over the noise on both its sides in a look is a signal's, not the noise's.
/// Each side's noise is its lower quartile ([`OCCUPIED_SIDE_QUANTILE`]), which a second signal
/// beside the first moves only once it holds three quarters of the side; one bin of noise goes
/// this far over it about once in a thousand.
pub const OCCUPIED_DB: f32 = 14.0;
/// Where in each side's bins its noise is read.
pub const OCCUPIED_SIDE_QUANTILE: f64 = 0.25;
/// How far each side of a bin the noise it is judged against is read, hertz: wider than an FT8
/// signal (50 Hz) with room for a second beside it, and narrow enough that a filter's skirt
/// changes little across it. Each side is judged alone, so a bin at a steep edge stands against
/// the passband's side and is not taken for a signal.
pub const OCCUPIED_SIDE_HZ: f64 = 150.0;
/// Bins either side of one a signal holds that are left out with it: the Hann window's main lobe
/// is two bins each side, and its skirt under [`OCCUPIED_DB`] still stands over the noise.
/// Without it a band of FT8 raised the passband's level against its edges by 2–4 dB.
pub const OCCUPIED_SPREAD_BINS: usize = 2;
/// The quantile of the bins inside the signal's span the passband's own level is read at: the
/// ninth decile, so a passband a tenth of the span wide still sets it.
pub const REFERENCE_QUANTILE: f64 = 0.9;
/// How often a look is taken while the station listens, seconds.
pub const LOOK_EVERY_S: f64 = 0.5;
/// How high a look's spectrum reaches: the top of the chain's floor band.
pub const SPECTRUM_TOP_HZ: f64 = FLOOR_BAND_HZ.1;

/// A sample this near full scale counts as clipped.
pub const FULL_SCALE: f32 = 0.99;
/// How much of the audio the station listened to clipping is read over, seconds.
pub const CLIP_WINDOW_S: f64 = 10.0;
/// The share of samples at full scale over which the audio clips: every one of the mobile's
/// sessions of 2026-11-07 passed it (1.3–1.7 %) and no other recording reached it.
pub const CLIP_SHARE: f64 = 0.001;

/// C1: the share of samples at full scale over the last [`CLIP_WINDOW_S`] of audio heard. While
/// it is over [`CLIP_SHARE`] the audio clips.
#[derive(Debug, Clone, Default)]
pub struct Clipping {
    /// Per block (clipped, samples), oldest first.
    blocks: VecDeque<(usize, usize)>,
    held: usize,
    window: usize,
}

impl Clipping {
    /// A fresh reading at `rate` samples a second.
    #[must_use]
    pub fn new(rate: f64) -> Self {
        Self {
            window: (rate * CLIP_WINDOW_S) as usize,
            ..Self::default()
        }
    }

    /// A block of audio the station heard.
    pub fn hear(&mut self, audio: &[f32]) {
        let clipped = audio.iter().filter(|x| x.abs() >= FULL_SCALE).count();
        self.blocks.push_back((clipped, audio.len()));
        self.held += audio.len();
        while self.held > self.window
            && let Some((_, n)) = self.blocks.pop_front()
        {
            self.held -= n;
        }
    }

    /// The share of the samples held at full scale.
    #[must_use]
    pub fn share(&self) -> f64 {
        let clipped: usize = self.blocks.iter().map(|&(c, _)| c).sum();
        clipped as f64 / self.held.max(1) as f64
    }

    /// Whether the audio clips now.
    #[must_use]
    pub fn clips(&self) -> bool {
        self.share() > CLIP_SHARE
    }
}

/// Where the modem's signal sits in the audio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    /// Its low edge, hertz.
    pub low_hz: f64,
    /// Its high edge, hertz.
    pub high_hz: f64,
}

impl Span {
    /// The waveform's own span: its centre ± half its occupied width.
    #[must_use]
    pub fn of(params: WaveformParams) -> Self {
        let half = params.occupied_bandwidth_hz() / 2.0;
        Self {
            low_hz: params.centre_hz - half,
            high_hz: params.centre_hz + half,
        }
    }
}

/// What the reading says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Not enough looks yet.
    Waiting,
    /// Looks offered, but the band's noise too close to the chain's floor for a cut to show.
    Unknown,
    /// Both edges inside the line.
    Clear,
    /// The top edge of the signal is cut.
    Top,
    /// The bottom edge is cut.
    Bottom,
    /// Both edges are cut.
    Both,
}

impl Verdict {
    /// The word the control API and a trace write.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Unknown => "unknown",
            Self::Clear => "clear",
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Both => "both",
        }
    }
}

/// The reading now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilterReading {
    /// What it says.
    pub verdict: Verdict,
    /// How far the signal's low edge lies under the passband's level, dB.
    pub low_drop_db: Option<f32>,
    /// How far its high edge lies, dB.
    pub high_drop_db: Option<f32>,
    /// How far the last look's band noise stood over the chain's floor, dB.
    pub over_floor_db: Option<f32>,
    /// Looks kept.
    pub looks: usize,
}

/// The `q` quantile of the values, NaN left out.
fn quantile(values: &mut Vec<f32>, q: f64) -> Option<f32> {
    values.retain(|v| !v.is_nan());
    if values.is_empty() {
        return None;
    }
    let at = ((values.len() - 1) as f64 * q).round() as usize;
    Some(*values.select_nth_unstable_by(at, f32::total_cmp).1)
}

/// The bins from `low_hz` to `high_hz`, both included.
fn bins_between(bins: &[f32], bin_hz: f64, low_hz: f64, high_hz: f64) -> Vec<f32> {
    let first = (low_hz / bin_hz).round().max(0.0) as usize;
    let last = ((high_hz / bin_hz).round() as usize).min(bins.len().saturating_sub(1));
    if bins.is_empty() || first > last {
        return Vec::new();
    }
    bins[first..=last].to_vec()
}

/// How far each edge of `span` lies under the passband's own level in `profile` (dB a bin). The
/// profile is first smoothed by a running median over ±[`EDGE_HALF_HZ`] (none where signals held
/// half the window or more): on a skirt a median keeps the value at its centre where a mean of
/// decibels reads the slope's curve, and it takes the noise of a profile of few looks out of the
/// level. The level is the smoothed profile's [`REFERENCE_QUANTILE`] over the span, each edge its
/// value there. `None` where the profile holds no finite bin there.
#[must_use]
fn span_drops(profile: &[f32], bin_hz: f64, span: Span) -> Option<(f32, f32)> {
    if bin_hz <= 0.0 || profile.is_empty() {
        return None;
    }
    let half = (EDGE_HALF_HZ / bin_hz).round() as usize;
    let bin_of = |hz: f64| ((hz / bin_hz).round().max(0.0) as usize).min(profile.len() - 1);
    let (first, last) = (bin_of(span.low_hz), bin_of(span.high_hz));
    let mut column = Vec::with_capacity(2 * half + 1);
    let mut smoothed = |k: usize| -> Option<f32> {
        let window = &profile[k.saturating_sub(half)..(k + half + 1).min(profile.len())];
        column.clear();
        column.extend(window.iter().copied().filter(|v| !v.is_nan()));
        if column.len() * 2 < window.len() {
            return None;
        }
        quantile(&mut column, 0.5)
    };
    let mut inside: Vec<f32> = (first..=last).filter_map(&mut smoothed).collect();
    let reference = quantile(&mut inside, REFERENCE_QUANTILE)?;
    Some((reference - smoothed(first)?, reference - smoothed(last)?))
}

/// The verdict on the drops at the two edges.
#[must_use]
fn verdict_of(low_drop_db: f32, high_drop_db: f32) -> Verdict {
    match (low_drop_db >= LINE_DB, high_drop_db >= LINE_DB) {
        (true, true) => Verdict::Both,
        (false, true) => Verdict::Top,
        (true, false) => Verdict::Bottom,
        (false, false) => Verdict::Clear,
    }
}

/// Leave out of `look` (dB a bin) the bins a signal holds: those more than [`OCCUPIED_DB`] over
/// the noise of the [`OCCUPIED_SIDE_HZ`] on each side of them, and the [`OCCUPIED_SPREAD_BINS`]
/// beside each. A bin left out is NaN. One pass: reading the sides again without what it left
/// out changed nothing on the corpus or the field, since each side's lower quartile already
/// stands clear of a signal beside it.
fn leave_out_occupied(look: &mut [f32], bin_hz: f64) {
    if bin_hz <= 0.0 || look.is_empty() {
        return;
    }
    let side = ((OCCUPIED_SIDE_HZ / bin_hz).round() as usize).max(1);
    let mut column = Vec::with_capacity(side);
    let mut noise_of = |values: &[f32]| -> Option<f32> {
        column.clear();
        column.extend_from_slice(values);
        quantile(&mut column, OCCUPIED_SIDE_QUANTILE)
    };
    let n = look.len();
    let held: Vec<usize> = (0..n)
        .filter(|&k| {
            let below = noise_of(&look[k.saturating_sub(side)..k]);
            let above = noise_of(&look[(k + 1).min(n)..(k + 1 + side).min(n)]);
            let over = |noise: Option<f32>| noise.is_none_or(|m| look[k] - m > OCCUPIED_DB);
            // a bin with neither side to judge by is left alone
            (below.is_some() || above.is_some()) && over(below) && over(above)
        })
        .collect();
    for k in held {
        for v in
            &mut look[k.saturating_sub(OCCUPIED_SPREAD_BINS)..(k + OCCUPIED_SPREAD_BINS + 1).min(n)]
        {
            *v = f32::NAN;
        }
    }
}

/// The receive filter, read from the noise the station hears while it listens.
#[derive(Debug, Clone)]
pub struct ReceiveFilter {
    span: Span,
    /// How long a look is held before it is folded in: the air's longest frame and the half
    /// second a report of it takes, so no frame under it can still be announced.
    hold_s: f64,
    /// Looks kept, each divided by its own level, dB a bin to [`TOP_HZ`]; a bin a signal held
    /// is NaN.
    looks: VecDeque<Vec<f32>>,
    bin_hz: f64,
    /// Looks left out because the band's noise stood too close to the chain's floor.
    weak: usize,
    last_over: Option<f32>,
    /// Looks taken and not yet folded in, oldest first, each with when its audio ended.
    held: VecDeque<(f64, Spectrum)>,
    /// Where frames announced or reported lay, on the audio's clock: a look under one is the
    /// frame's spectrum, whose edges are the signal's own, and is dropped.
    frames: VecDeque<(f64, f64)>,
    /// How far the receiver's clock runs behind the audio's, seconds: the band filter's and the
    /// decimator's delay. A frame is placed on the receiver's clock and a look on the audio's.
    rx_delay_s: f64,
    /// Since when the station has listened without a break.
    listening_since: f64,
    next_look: f64,
}

impl ReceiveFilter {
    /// A fresh reading of `span`, each look held `hold_s` before it is folded in.
    #[must_use]
    pub fn new(span: Span, hold_s: f64) -> Self {
        Self {
            span,
            hold_s,
            looks: VecDeque::new(),
            bin_hz: 0.0,
            weak: 0,
            last_over: None,
            held: VecDeque::new(),
            frames: VecDeque::new(),
            rx_delay_s: 0.0,
            listening_since: f64::NEG_INFINITY,
            next_look: f64::NEG_INFINITY,
        }
    }

    /// How long a look is held on the air of `params`: its longest frame of any family, the
    /// tone floor's included, and half a second. On the harness half of the looks taken while
    /// listening fell under frames announced only part way in, and a 10 dB cut read 15–19 dB.
    #[must_use]
    pub fn hold_for(params: WaveformParams) -> f64 {
        let air = aether_phy::modes::air_interface(params);
        air.tone_kinds()
            .iter()
            .map(|k| k.duration_s())
            .fold(air.long.duration_s(), f64::max)
            + 0.5
    }

    /// How far the receiver's clock runs behind the audio's on the air of `params`, seconds.
    fn rx_delay_for(params: WaveformParams) -> f64 {
        aether_phy::AudioToBaseband::new(params).rx_delay_samples() as f64
            / params.audio_rate as f64
    }

    /// A fresh reading of the air of `params`.
    #[must_use]
    pub fn for_air(params: WaveformParams) -> Self {
        Self {
            rx_delay_s: Self::rx_delay_for(params),
            ..Self::new(Span::of(params), Self::hold_for(params))
        }
    }

    /// The station moved to the air of `params` (ADR-0026). The looks kept are the same radio's
    /// noise and are kept, each divided again by its level over the new span.
    pub fn set_air(&mut self, params: WaveformParams) {
        self.span = Span::of(params);
        self.hold_s = Self::hold_for(params);
        self.rx_delay_s = Self::rx_delay_for(params);
        let span = self.span;
        let bin_hz = self.bin_hz;
        for look in &mut self.looks {
            if let Some(level) = quantile(
                &mut bins_between(look, bin_hz, span.low_hz, span.high_hz),
                REFERENCE_QUANTILE,
            ) {
                for v in look.iter_mut() {
                    *v -= level;
                }
            }
        }
    }

    /// A frame announced or reported from `start_s` to `end_s`, seconds on the receiver's clock,
    /// where the station places its frames: about 20 ms after their audio arrives. Without the
    /// shift a look ending in a frame's first 20 ms was kept.
    pub fn frame(&mut self, start_s: f64, end_s: f64) {
        self.frames
            .push_back((start_s - self.rx_delay_s, end_s - self.rx_delay_s));
    }

    /// One pass at `now`: fold in the looks held long enough, dropping those a frame lay under,
    /// and take a new one with `take` when one is due and the station has listened for a whole
    /// `window_s` (the audio a spectrum covers). Returns when each look folded in was taken.
    pub fn step(
        &mut self,
        now: f64,
        listening: bool,
        window_s: f64,
        take: impl FnOnce() -> Option<Spectrum>,
    ) -> Vec<f64> {
        let mut folded = Vec::new();
        while self
            .held
            .front()
            .is_some_and(|&(taken, _)| now - taken >= self.hold_s)
        {
            let Some((taken, spectrum)) = self.held.pop_front() else {
                break;
            };
            if self
                .frames
                .iter()
                .any(|&(start, end)| start < taken && end > taken - window_s)
            {
                continue;
            }
            self.fold(&spectrum);
            folded.push(taken);
        }
        // a frame that lies under no look held or to come is forgotten
        let oldest = self.held.front().map_or(now, |&(taken, _)| taken);
        self.frames.retain(|&(_, end)| end > oldest - window_s);
        if !listening {
            self.listening_since = now;
            return folded;
        }
        if now >= self.next_look
            && now - self.listening_since >= window_s
            && let Some(spectrum) = take()
        {
            self.held.push_back((now, spectrum));
            self.next_look = now + LOOK_EVERY_S;
        }
        folded
    }

    /// Divide a look by its own level and keep it if its band's noise stands over the chain's
    /// floor, first leaving out the bins a signal holds.
    fn fold(&mut self, spectrum: &Spectrum) {
        if (self.bin_hz - spectrum.bin_hz).abs() > 1e-9 {
            self.looks.clear();
            self.bin_hz = spectrum.bin_hz;
        }
        let bin_hz = spectrum.bin_hz;
        let floor = quantile(
            &mut bins_between(&spectrum.bins_db, bin_hz, FLOOR_BAND_HZ.0, FLOOR_BAND_HZ.1),
            0.5,
        );
        let top = ((TOP_HZ / bin_hz) as usize).min(spectrum.bins_db.len().saturating_sub(1));
        let mut look = spectrum.bins_db[..=top].to_vec();
        leave_out_occupied(&mut look, bin_hz);
        let in_span = bins_between(&look, bin_hz, self.span.low_hz, self.span.high_hz);
        if in_span.iter().filter(|v| v.is_finite()).count() * 2 < in_span.len() {
            // more than half the span held by signals: nothing to read the noise from
            self.last_over = None;
            return;
        }
        let level = quantile(
            &mut bins_between(&look, bin_hz, self.span.low_hz, self.span.high_hz),
            REFERENCE_QUANTILE,
        );
        // a look whose band's noise stands less than the line over the chain's floor could not
        // show a cut at the line; a silent input reads no level over its floor at all
        self.last_over = level.zip(floor).map(|(l, f)| l - f);
        let Some(level) = level.filter(|_| self.last_over.is_some_and(|o| o >= LINE_DB)) else {
            self.weak = self.weak.saturating_add(1);
            return;
        };
        for v in &mut look {
            *v -= level;
        }
        self.looks.push_back(look);
        while self.looks.len() > LOOKS_KEPT {
            self.looks.pop_front();
        }
    }

    /// Each bin's median over the looks kept that it was free in; NaN where a signal held it in
    /// more than half of them. The looks that kept such a bin are those where a signal's skirt
    /// slipped under [`OCCUPIED_DB`]: on a band of FT8 a median of those alone raised a 500 Hz
    /// span's level 12 dB.
    fn profile(&self) -> Vec<f32> {
        let Some(first) = self.looks.front() else {
            return Vec::new();
        };
        let mut column = Vec::with_capacity(self.looks.len());
        (0..first.len())
            .map(|bin| {
                column.clear();
                column.extend(
                    self.looks
                        .iter()
                        .map(|look| look[bin])
                        .filter(|v| !v.is_nan()),
                );
                if column.len() * 2 < self.looks.len() {
                    return f32::NAN;
                }
                quantile(&mut column, 0.5).unwrap_or(f32::NAN)
            })
            .collect()
    }

    /// The profile and its bin width, for a recording: `None` before the first look kept.
    #[must_use]
    pub fn noise_profile(&self) -> Option<(f64, Vec<f32>)> {
        (!self.looks.is_empty()).then(|| (self.bin_hz, self.profile()))
    }

    /// Looks kept.
    #[must_use]
    pub fn looks(&self) -> usize {
        self.looks.len()
    }

    /// What it says now.
    #[must_use]
    pub fn reading(&self) -> FilterReading {
        let mut reading = FilterReading {
            verdict: Verdict::Waiting,
            low_drop_db: None,
            high_drop_db: None,
            over_floor_db: self.last_over,
            looks: self.looks.len(),
        };
        if self.looks.len() < LOOKS_NEEDED {
            if self.weak >= LOOKS_NEEDED {
                reading.verdict = Verdict::Unknown;
            }
            return reading;
        }
        if let Some((low, high)) = span_drops(&self.profile(), self.bin_hz, self.span) {
            reading.verdict = verdict_of(low, high);
            reading.low_drop_db = Some(low);
            reading.high_drop_db = Some(high);
        }
        reading
    }

    /// When the next look may be taken, station seconds.
    #[cfg(test)]
    pub(crate) fn next_look_s(&self) -> f64 {
        self.next_look
    }
}

/// Band noise through a receiver's filter, for the tests here and the station's.
#[cfg(test)]
pub(crate) mod test_noise {
    /// One second-order section (the RBJ cookbook's), run in direct form I.
    struct Biquad {
        b: [f64; 3],
        a: [f64; 2],
        x: [f64; 2],
        y: [f64; 2],
    }

    impl Biquad {
        fn new(high_pass: bool, hz: f64, rate: f64) -> Self {
            let w = std::f64::consts::TAU * hz / rate;
            let alpha = w.sin() / std::f64::consts::SQRT_2;
            let cos = w.cos();
            let a0 = 1.0 + alpha;
            let b = if high_pass {
                [
                    f64::midpoint(1.0, cos),
                    -(1.0 + cos),
                    f64::midpoint(1.0, cos),
                ]
            } else {
                [(1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0]
            };
            Self {
                b: b.map(|v| v / a0),
                a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
                x: [0.0; 2],
                y: [0.0; 2],
            }
        }

        fn run(&mut self, x: f64) -> f64 {
            let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1]
                - self.a[0] * self.y[0]
                - self.a[1] * self.y[1];
            self.x = [x, self.x[0]];
            self.y = [y, self.y[0]];
            y
        }
    }

    /// The band's noise through a passband from `low` to `high` Hz (four sections a side), at
    /// `band` (a uniform draw's scale), over the audio chain's own white floor at `floor`.
    pub(crate) struct ReceiverNoise {
        sections: Vec<Biquad>,
        band: f64,
        floor: f64,
        seed: u64,
    }

    impl ReceiverNoise {
        pub(crate) fn new(low: f64, high: f64, rate: f64, band: f64, floor: f64) -> Self {
            let mut sections: Vec<Biquad> = (0..4).map(|_| Biquad::new(true, low, rate)).collect();
            sections.extend((0..4).map(|_| Biquad::new(false, high, rate)));
            Self {
                sections,
                band,
                floor,
                seed: 0x2545_F491_4F6C_DD1D,
            }
        }

        fn uniform(&mut self) -> f64 {
            self.seed ^= self.seed << 13;
            self.seed ^= self.seed >> 7;
            self.seed ^= self.seed << 17;
            (self.seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        }

        /// The next sample.
        pub(crate) fn sample(&mut self) -> f32 {
            let x = self.uniform();
            let band = self.sections.iter_mut().fold(x, |x, s| s.run(x));
            let floor = self.uniform();
            (self.band * band + self.floor * floor) as f32
        }

        /// The next `n` samples.
        pub(crate) fn take(&mut self, n: usize) -> Vec<f32> {
            (0..n).map(|_| self.sample()).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spectrum::SpectrumAnalyser;

    const RATE: f64 = 48_000.0;
    const BIN_HZ: f64 = RATE / crate::spectrum::WINDOW as f64;

    /// A look to [`SPECTRUM_TOP_HZ`]: the band's noise at `band_db` through a response that is
    /// `gain(hz)` dB, over a chain floor at `floor_db`, powers added.
    fn look(band_db: f32, floor_db: f32, gain: impl Fn(f64) -> f32) -> Spectrum {
        let bins = (SPECTRUM_TOP_HZ / BIN_HZ) as usize + 1;
        let bins_db = (0..bins)
            .map(|k| {
                let hz = k as f64 * BIN_HZ;
                let band = if hz <= 4000.0 {
                    band_db + gain(hz)
                } else {
                    -200.0
                };
                10.0 * (10f32.powf(band / 10.0) + 10f32.powf(floor_db / 10.0)).log10()
            })
            .collect();
        Spectrum {
            bin_hz: BIN_HZ,
            bins_db,
        }
    }

    /// An eighth-order low-pass at `corner` Hz, dB.
    fn low_pass(corner: f64) -> impl Fn(f64) -> f32 {
        move |hz| (-10.0 * (1.0 + (hz / corner).powi(16)).log10()) as f32
    }

    /// An eighth-order band-pass, dB.
    fn band_pass(low: f64, high: f64) -> impl Fn(f64) -> f32 {
        move |hz| {
            let hz = hz.max(1.0);
            (-10.0 * ((1.0 + (hz / high).powi(16)) * (1.0 + (low / hz).powi(16))).log10()) as f32
        }
    }

    fn wide() -> ReceiveFilter {
        ReceiveFilter::for_air(aether_phy::WIDE_2300)
    }

    fn narrow() -> ReceiveFilter {
        ReceiveFilter::for_air(aether_phy::waveform::NARROW_500)
    }

    /// `n` copies of `look` folded in.
    fn fed(mut filter: ReceiveFilter, look: &Spectrum, n: usize) -> ReceiveFilter {
        for _ in 0..n {
            filter.fold(look);
        }
        filter
    }

    fn verdict(filter: &ReceiveFilter) -> Verdict {
        filter.reading().verdict
    }

    #[test]
    fn the_span_is_the_waveforms_own() {
        let span = Span::of(aether_phy::WIDE_2300);
        assert!((span.low_hz - 360.0).abs() < 1.0 && (span.high_hz - 2640.0).abs() < 1.0);
        let span = Span::of(aether_phy::waveform::NARROW_500);
        assert!((span.low_hz - 1260.0).abs() < 1.0 && (span.high_hz - 1740.0).abs() < 1.0);
    }

    #[test]
    fn a_passband_moved_down_reads_as_its_top_cut() {
        // the mobile's IF shift turned fully left (2026-11-07): flat to 1.4 kHz, and the
        // 2300 Hz signal's top far down; its width alone (today's test) never warned
        let r = fed(wide(), &look(-60.0, -110.0, low_pass(1600.0)), LOOKS_NEEDED).reading();
        assert_eq!(r.verdict, Verdict::Top, "{r:?}");
        assert!(r.low_drop_db.is_some_and(|d| d < 1.0), "{r:?}");
        assert!(r.high_drop_db.is_some_and(|d| d > 30.0), "{r:?}");
    }

    #[test]
    fn a_low_cut_set_high_reads_as_its_bottom_cut() {
        let f = fed(
            wide(),
            &look(-60.0, -110.0, band_pass(600.0, 3000.0)),
            LOOKS_NEEDED,
        );
        assert_eq!(verdict(&f), Verdict::Bottom, "{:?}", f.reading());
    }

    #[test]
    fn a_wide_filter_is_clear_and_a_narrow_one_is_cut_both_sides() {
        let f = fed(
            wide(),
            &look(-60.0, -110.0, band_pass(300.0, 3000.0)),
            LOOKS_NEEDED,
        );
        assert_eq!(verdict(&f), Verdict::Clear, "{:?}", f.reading());
        // a CW filter under the 2300 Hz signal
        let f = fed(
            wide(),
            &look(-60.0, -110.0, band_pass(1250.0, 1750.0)),
            LOOKS_NEEDED,
        );
        assert_eq!(verdict(&f), Verdict::Both, "{:?}", f.reading());
        // and the same filter passes the 500 Hz signal's 1260-1740 Hz only just: its edges sit
        // on the skirt's knee, a few decibels down
        let f = fed(
            narrow(),
            &look(-60.0, -110.0, band_pass(1100.0, 1900.0)),
            LOOKS_NEEDED,
        );
        assert_eq!(verdict(&f), Verdict::Clear, "{:?}", f.reading());
    }

    #[test]
    fn flat_noise_passes_the_signal() {
        let f = fed(wide(), &look(-60.0, -110.0, |_| 0.0), LOOKS_NEEDED);
        let r = f.reading();
        assert_eq!(r.verdict, Verdict::Clear, "{r:?}");
        assert!(r.low_drop_db.is_some_and(|d| d.abs() < 0.5));
        assert!(r.high_drop_db.is_some_and(|d| d.abs() < 0.5));
    }

    #[test]
    fn a_notch_inside_the_span_is_not_an_edge() {
        // a radio's DSP notch 120 Hz wide at 1 kHz: deep, but the edges stand
        let notch = |hz: f64| {
            if (hz - 1000.0).abs() < 60.0 {
                -30.0
            } else {
                0.0
            }
        };
        let f = fed(
            wide(),
            &look(-60.0, -110.0, move |hz| {
                band_pass(300.0, 3000.0)(hz) + notch(hz)
            }),
            LOOKS_NEEDED,
        );
        assert_eq!(verdict(&f), Verdict::Clear, "{:?}", f.reading());
    }

    #[test]
    fn a_cut_over_a_shallow_floor_still_reads_past_the_line() {
        // the mobile's chain floor 15 dB under the band's noise: a 20 dB cut reads about 14
        let corner = 2640.0 / 99f64.powf(1.0 / 16.0);
        let f = fed(wide(), &look(-60.0, -75.0, low_pass(corner)), LOOKS_NEEDED);
        let r = f.reading();
        assert_eq!(r.verdict, Verdict::Top, "{r:?}");
        assert!(
            r.high_drop_db.is_some_and(|d| (LINE_DB..16.0).contains(&d)),
            "{r:?}"
        );
        // and a 10 dB one stays under it
        let corner = 2640.0 / 9f64.powf(1.0 / 16.0);
        let f = fed(wide(), &look(-60.0, -75.0, low_pass(corner)), LOOKS_NEEDED);
        assert_eq!(verdict(&f), Verdict::Clear, "{:?}", f.reading());
    }

    #[test]
    fn looks_whose_band_noise_sits_on_the_chain_floor_say_nothing() {
        // can't check: an AGC's dip, or the radio's audio turned right down
        let mut f = fed(
            wide(),
            &look(-90.0, -95.0, low_pass(1600.0)),
            LOOKS_NEEDED - 1,
        );
        assert_eq!(verdict(&f), Verdict::Waiting);
        f.fold(&look(-90.0, -95.0, low_pass(1600.0)));
        let r = f.reading();
        assert_eq!(r.verdict, Verdict::Unknown, "{r:?}");
        assert_eq!(r.looks, 0);
        assert!(r.over_floor_db.is_some_and(|o| o < LINE_DB), "{r:?}");
        // a silent input says nothing either
        let silent = Spectrum {
            bin_hz: BIN_HZ,
            bins_db: vec![-240.0; (SPECTRUM_TOP_HZ / BIN_HZ) as usize + 1],
        };
        assert_eq!(fed(wide(), &silent, LOOKS_NEEDED).reading().looks, 0);
        // with the band's noise well over the floor the same filter is read
        let f = fed(f, &look(-60.0, -95.0, low_pass(1600.0)), LOOKS_NEEDED);
        assert_eq!(verdict(&f), Verdict::Top);
    }

    #[test]
    fn a_look_divided_by_its_level_reads_the_same_shape_at_any_level() {
        let mut f = wide();
        for k in 0..LOOKS_KEPT {
            // the band's noise swinging 20 dB look to look, an AGC at work
            let band = -60.0 + 20.0 * (k % 2) as f32;
            f.fold(&look(band, -120.0, band_pass(300.0, 2700.0)));
        }
        assert_eq!(verdict(&f), Verdict::Clear, "{:?}", f.reading());
    }

    #[test]
    fn a_band_of_signals_is_left_out_of_the_profile() {
        // signals 50 Hz wide every 200 Hz across the span, an FT8 band, in every look: there
        // is no quiet look to wait for
        let mut busy = look(-60.0, -110.0, band_pass(300.0, 3000.0));
        for (k, v) in busy.bins_db.iter_mut().enumerate() {
            let hz = k as f64 * BIN_HZ;
            if (450.0..2550.0).contains(&hz) && (hz - 450.0) % 200.0 < 50.0 {
                *v += 30.0;
            }
        }
        let r = fed(wide(), &busy, LOOKS_NEEDED).reading();
        assert_eq!(r.verdict, Verdict::Clear, "{r:?}");
        assert!(
            r.low_drop_db.is_some_and(|d| d.abs() < 1.0)
                && r.high_drop_db.is_some_and(|d| d.abs() < 1.0),
            "{r:?}"
        );
    }

    /// Looks of a band of FT8 through a radio's filter `gain` (dB), to [`SPECTRUM_TOP_HZ`]: in
    /// each the band's noise drawn afresh a bin (an exponential, so one look varies by several
    /// decibels), over a chain floor 50 dB down, and in thirteen looks of fifteen (the period's
    /// quiet gap is the rest) fifteen signals 50 Hz wide at random places from 200 to 2900 Hz,
    /// 5–30 dB over the noise, through the same filter.
    fn ft8_band(gain: impl Fn(f64) -> f32, looks: usize, seed: u64) -> Vec<Spectrum> {
        let mut state = seed;
        let mut uniform = move || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            ((z >> 11) as f64 + 0.5) / (1u64 << 53) as f64
        };
        let bins = (SPECTRUM_TOP_HZ / BIN_HZ) as usize + 1;
        (0..looks)
            .map(|look| {
                let signals: Vec<(f64, f64)> = if look % 15 < 13 {
                    (0..15)
                        .map(|_| (200.0 + 2650.0 * uniform(), 5.0 + 25.0 * uniform()))
                        .collect()
                } else {
                    Vec::new()
                };
                let bins_db = (0..bins)
                    .map(|k| {
                        let hz = k as f64 * BIN_HZ;
                        let band = if hz <= 4000.0 {
                            -60.0 + f64::from(gain(hz))
                        } else {
                            -250.0
                        };
                        let mut power = 10f64.powf(band / 10.0) * -uniform().ln() + 1e-11;
                        for &(at, over) in &signals {
                            if (at..at + 50.0).contains(&hz) {
                                power += 10f64.powf((band + over) / 10.0);
                            }
                        }
                        (10.0 * power.log10()) as f32
                    })
                    .collect();
                Spectrum {
                    bin_hz: BIN_HZ,
                    bins_db,
                }
            })
            .collect()
    }

    #[test]
    fn a_band_of_ft8_reads_the_filter_from_its_first_reading() {
        // the author on 7074 kHz with a 3 kHz filter, beta.130: the estimate before read the
        // band as a filter 850 Hz wide. A look needs no quiet: the signals are left out of it
        let mut f = wide();
        for (k, s) in ft8_band(band_pass(200.0, 3200.0), LOOKS_KEPT, 1)
            .iter()
            .enumerate()
        {
            f.fold(s);
            if k + 1 >= LOOKS_NEEDED {
                assert_eq!(
                    verdict(&f),
                    Verdict::Clear,
                    "after {} looks: {:?}",
                    k + 1,
                    f.reading()
                );
            }
        }
        // and a CW filter under the same band still reads cut both sides
        let mut f = wide();
        for s in ft8_band(band_pass(1250.0, 1750.0), LOOKS_NEEDED, 2) {
            f.fold(&s);
        }
        assert_eq!(verdict(&f), Verdict::Both, "{:?}", f.reading());
    }

    #[test]
    fn an_edge_a_signal_holds_in_every_look_is_not_read() {
        // a station parked on the top edge would read as the passband standing there
        let mut busy = look(-60.0, -110.0, low_pass(1600.0));
        for (k, v) in busy.bins_db.iter_mut().enumerate() {
            if (2580.0..2700.0).contains(&(k as f64 * BIN_HZ)) {
                *v = -40.0;
            }
        }
        assert_eq!(verdict(&fed(wide(), &busy, LOOKS_NEEDED)), Verdict::Waiting);
    }

    #[test]
    fn a_strong_signal_and_its_window_skirt_are_left_out() {
        let mut look = look(-60.0, -110.0, band_pass(300.0, 3000.0)).bins_db;
        let at = (1500.0 / BIN_HZ).round() as usize;
        look[at] = -20.0;
        // the Hann window's main lobe beside it, under OCCUPIED_DB over the noise
        look[at - 1] = -50.0;
        look[at + 1] = -50.0;
        look[at - 2] = -55.0;
        look[at + 2] = -55.0;
        leave_out_occupied(&mut look, BIN_HZ);
        assert!(look[at - 2..=at + 2].iter().all(|v| v.is_nan()));
        assert!(look[at - 3].is_finite() && look[at + 3].is_finite());
        assert_eq!(look.iter().filter(|v| v.is_nan()).count(), 5);
    }

    #[test]
    fn the_reading_waits_for_looks_and_holds_each_until_no_frame_can_be_under_it() {
        let mut f = wide();
        let s = look(-60.0, -110.0, low_pass(1600.0));
        let window = 0.085;
        let hold = ReceiveFilter::hold_for(aether_phy::WIDE_2300);
        assert!(hold > 5.0, "the tone floor's 5.36 s frames: {hold}");
        let mut taken = 0;
        let mut folded = Vec::new();
        let mut now = 0.0;
        while now < hold + 7.0 {
            folded.extend(f.step(now, true, window, || {
                taken += 1;
                Some(s.clone())
            }));
            if now < hold {
                assert!(
                    folded.is_empty(),
                    "a look folded in before its hold at {now}"
                );
            }
            now += 0.02;
        }
        // a look every half second, the first as soon as a window was heard, and folded in
        // once held: those taken by the run's last 5.9 s are still held
        assert!((25..=27).contains(&taken), "{taken} looks taken");
        assert!((14..=16).contains(&folded.len()), "{} folded", folded.len());
        assert!(
            folded
                .windows(2)
                .all(|w| (w[1] - w[0] - LOOK_EVERY_S).abs() < 0.03)
        );
        assert!(folded.len() >= LOOKS_NEEDED, "{} folded", folded.len());
        assert_eq!(verdict(&f), Verdict::Top);
    }

    #[test]
    fn a_look_a_frame_lay_under_is_dropped() {
        let mut f = wide();
        let s = look(-60.0, -110.0, |_| 0.0);
        let window = 0.085;
        let mut now = 0.0;
        let mut folded = Vec::new();
        while now < 20.0 {
            folded.extend(f.step(now, true, window, || Some(s.clone())));
            // a 3 s frame from 5 s, announced only 2 s in
            if (now - 7.0).abs() < 0.01 {
                f.frame(5.0, 8.0);
            }
            now += 0.02;
        }
        assert!(
            folded.iter().all(|&t| t <= 5.0 || t - window >= 8.0),
            "a look under the frame was kept: {folded:?}"
        );
        assert!(folded.iter().any(|&t| t < 5.0) && folded.iter().any(|&t| t > 8.0));
    }

    #[test]
    fn no_look_is_taken_until_a_whole_window_has_been_heard() {
        let mut f = wide();
        let s = look(-60.0, -110.0, |_| 0.0);
        let window = 0.085;
        let mut taken = Vec::new();
        let mut now = 0.0;
        while now < 3.0 {
            // keyed, then deaf, until 1 s
            let listening = now >= 1.0;
            f.step(now, listening, window, || {
                taken.push(now);
                Some(s.clone())
            });
            now += 0.02;
        }
        // the last pass not listening was at 0.98 s: the audio heard since is the window
        assert!(
            taken
                .first()
                .is_some_and(|&t| t >= 0.98 + window - 1e-9 && t < 1.0 + window + 0.03),
            "{taken:?}"
        );
        assert!(
            (f.next_look_s() - taken.last().copied().unwrap_or(0.0) - LOOK_EVERY_S).abs() < 1e-9
        );
    }

    #[test]
    fn a_change_of_bandwidth_keeps_the_looks_and_judges_the_new_span() {
        // a filter 1100-1900 Hz: cut both sides of the 2300 Hz signal, clear of the 500 Hz one
        let s = look(-60.0, -110.0, band_pass(1100.0, 1900.0));
        let mut f = fed(wide(), &s, LOOKS_NEEDED);
        assert_eq!(verdict(&f), Verdict::Both);
        f.set_air(aether_phy::waveform::NARROW_500);
        assert_eq!(verdict(&f), Verdict::Clear, "{:?}", f.reading());
        assert_eq!(f.reading().looks, LOOKS_NEEDED);
        // and new looks, divided by the new span's level, agree with the old ones
        let f = fed(f, &s, LOOKS_NEEDED);
        let r = f.reading();
        assert!(r.low_drop_db.is_some_and(|d| d.abs() < 6.0), "{r:?}");
    }

    #[test]
    fn a_receivers_noise_reads_its_own_passband() {
        // real samples through the station's analyser: noise through 300-1900 Hz, the top of the
        // 2300 Hz signal cut, over the chain's floor 50 dB down
        let mut noise = test_noise::ReceiverNoise::new(300.0, 1900.0, RATE, 0.3, 0.001);
        let mut analyser = SpectrumAnalyser::new(RATE);
        let mut f = wide();
        let block = 960;
        let mut now = 0.0;
        while now < 15.0 {
            analyser.push(&noise.take(block));
            now += block as f64 / RATE;
            f.step(now, true, analyser.window_s(), || {
                analyser.compute_to(SPECTRUM_TOP_HZ)
            });
        }
        let r = f.reading();
        assert_eq!(r.verdict, Verdict::Top, "{r:?}");
        assert!(r.over_floor_db.is_some_and(|o| o > 30.0), "{r:?}");
        // the same receiver opened to 250-3500 Hz passes it
        let mut noise = test_noise::ReceiverNoise::new(250.0, 3500.0, RATE, 0.3, 0.001);
        let mut analyser = SpectrumAnalyser::new(RATE);
        let mut f = wide();
        let mut now = 0.0;
        while now < 15.0 {
            analyser.push(&noise.take(block));
            now += block as f64 / RATE;
            f.step(now, true, analyser.window_s(), || {
                analyser.compute_to(SPECTRUM_TOP_HZ)
            });
        }
        assert_eq!(f.reading().verdict, Verdict::Clear, "{:?}", f.reading());
    }

    #[test]
    fn clipping_is_read_over_the_last_ten_seconds() {
        let rate = 1000.0;
        let mut c = Clipping::new(rate);
        let mut block = vec![0.1f32; 100];
        block[0] = 1.0;
        block[1] = -0.995;
        c.hear(&block);
        assert!(c.clips(), "{}", c.share());
        for _ in 0..100 {
            c.hear(&[0.1f32; 100]);
        }
        assert!(c.share() < f64::EPSILON, "{}", c.share());
        assert!(!c.clips());
    }
}
