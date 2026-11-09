//! The receive-filter review's candidates (Gate 3, `docs/research/receive-filter/03-candidates.md`).
//!
//! Today's estimate ([`PassbandMonitor`]) reads the passband's width from the noise between
//! signals and warns when it is under the signal's width less 400 Hz. Gate 0 found it sees a cut
//! and judges it wrongly: its edge rule puts the edge 7–25 dB down the skirt, and its test is the
//! width, so a passband moved by an IF shift, or a low edge cut, never warns. The candidates here
//! change one part at a time, so each change's worth shows on its own:
//!
//! * **R0** is today's, judged as the panel judges it.
//! * **R1** judges today's profile at the signal's two edges against the passband's own level,
//!   and names the side cut.
//! * **R2** divides each look by its own level over the signal's span and keeps a look only when
//!   that level stands over the audio chain's floor, read between 6 and 10 kHz where no radio
//!   passes audio, by the line [`LINE_DB`] or more; the profile is each bin's median over the
//!   looks kept. A look in an AGC's dip, where the band's noise sinks to the chain's, says
//!   nothing about the filter and is left out.
//! * **R3** is R2 with a look taken whenever the station listens, quiet or not: in each look the
//!   bins a signal holds, more than [`OCCUPIED_DB`] over the noise on both sides of them, are left
//!   out first. It needs no quiet window, so a band of FT8 reads in seconds, not minutes.
//!
//! Nothing here changes what the modem does. Until the review's decision these run only in
//! `aetherd --passband-trace --passband-variant`.

use std::collections::VecDeque;
use std::str::FromStr;

use crate::spectrum::{PassbandMonitor, Spectrum, TOP_HZ};

/// The warning's line: an edge of the signal this far under the passband's own level is cut.
/// On beta.144's receiver a cut 10 dB down at an edge costs nothing on the harness and 20 dB
/// costs; over the mobile's shallow chain floor (15 dB under the band's noise) a 20 dB cut reads
/// about 14 dB down and a 10 dB cut about 9 (`03-candidates.md` §1).
pub const LINE_DB: f32 = 12.0;
/// How far either side of an edge of the signal its level is averaged, hertz: eight or nine
/// bins, so one noisy bin does not decide.
pub const EDGE_HALF_HZ: f64 = 50.0;
/// Where the audio chain's own floor is read, hertz: above every radio's audio filter, below the
/// codec's anti-alias filter.
pub const FLOOR_BAND_HZ: (f64, f64) = (6000.0, 10_000.0);
/// Looks kept before a reading is given.
pub const LOOKS_NEEDED: usize = 12;
/// Looks each bin's median is read over (R2, R3): a minute at two a second, so a change of filter
/// shows within half of it.
pub const LOOKS_KEPT: usize = 120;
/// A bin this far over the noise on both its sides in a look is a signal's, not the noise's (R3).
/// Each side's noise is its lower quartile ([`OCCUPIED_SIDE_QUANTILE`]), which a second signal
/// beside the first moves only once it holds three quarters of the side; one bin of noise goes
/// this far over it about once in a thousand (as 10 dB over the median, Gate 3's third build,
/// which two FT8 signals 35 Hz apart defeated).
pub const OCCUPIED_DB: f32 = 14.0;
/// Where in each side's bins its noise is read (R3).
pub const OCCUPIED_SIDE_QUANTILE: f64 = 0.25;
/// How far each side of a bin the noise it is judged against is read, hertz (R3): wider than an
/// FT8 signal (50 Hz) with room for a second beside it, and narrow enough that a filter's skirt
/// changes little across it. Each side is judged alone, so a bin at a steep edge stands against
/// the passband's side and is not taken for a signal.
pub const OCCUPIED_SIDE_HZ: f64 = 150.0;
/// Bins either side of one a signal holds that are left out with it (R3): the Hann window's main
/// lobe is two bins each side, and its skirt under [`OCCUPIED_DB`] still stands over the noise.
/// Without it (Gate 3's second build) a band of FT8 raised the passband's level against its
/// edges by 2–4 dB and a filter 8 dB down at an edge read past the line.
pub const OCCUPIED_SPREAD_BINS: usize = 2;
/// The quantile of the bins inside the signal's span the passband's own level is read at: the
/// ninth decile, so a passband a tenth of the span wide still sets it. A 500 Hz filter under the
/// 2300 Hz signal passes a fifth of the span; at the upper quartile (Gate 3's first build) the
/// level fell in the cut, and over a shallow chain floor every look was taken for one too close
/// to the floor.
pub const REFERENCE_QUANTILE: f64 = 0.9;
/// How high the candidates that read the chain's floor need the spectrum.
pub const SPECTRUM_TOP_HZ: f64 = FLOOR_BAND_HZ.1;
/// The panel's slack today: it warns when the width is under the signal's less this (R0).
pub const WARNING_SLACK_HZ: f64 = 400.0;

/// A sample this near full scale counts as clipped (C1), as the Receive level tile counts it.
pub const FULL_SCALE: f32 = 0.99;
/// How much of the audio the station listened to clipping is read over (C1), seconds.
pub const CLIP_WINDOW_S: f64 = 10.0;
/// The share of samples at full scale over which the audio clips (C1): the panel's Receive level
/// line today, which every one of Saturday's sessions passed (1.3–1.7 %) and no other recording
/// reached.
pub const CLIP_SHARE: f64 = 0.001;

/// C1: the share of samples at full scale over the last [`CLIP_WINDOW_S`] of audio heard. While
/// it is over [`CLIP_SHARE`] the audio clips: a warning of its own, and no look of the passband
/// is taken, since a clipped spectrum is not the radio's response.
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

/// Which candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Variant {
    /// Today's estimate.
    #[default]
    R0,
    /// The span judged at its edges.
    R1,
    /// Looks divided by their own level, kept over the chain's floor.
    R2,
    /// R2 with occupied bins left out of looks taken outside the quiet.
    R3,
}

impl FromStr for Variant {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "r0" => Ok(Self::R0),
            "r1" => Ok(Self::R1),
            "r2" => Ok(Self::R2),
            "r3" => Ok(Self::R3),
            other => Err(format!("no passband variant {other}: r0, r1, r2 or r3")),
        }
    }
}

impl Variant {
    /// Whether this candidate reads the chain's floor, and so needs the spectrum to
    /// [`SPECTRUM_TOP_HZ`].
    #[must_use]
    pub fn reads_floor(self) -> bool {
        matches!(self, Self::R2 | Self::R3)
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
    /// The span `occupied_hz` wide around `centre_hz`.
    #[must_use]
    pub fn around(centre_hz: f64, occupied_hz: f64) -> Self {
        Self {
            low_hz: centre_hz - occupied_hz / 2.0,
            high_hz: centre_hz + occupied_hz / 2.0,
        }
    }

    /// Its width, hertz.
    #[must_use]
    pub fn width_hz(&self) -> f64 {
        self.high_hz - self.low_hz
    }
}

/// What a candidate says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Not enough looks yet.
    Waiting,
    /// Looks offered, but the band's noise is too close to the chain's floor for a cut to show.
    Unknown,
    /// Both edges inside the line.
    Clear,
    /// Today's warning: the width under the signal's less [`WARNING_SLACK_HZ`] (R0 only).
    Narrow,
    /// The top edge of the signal is cut.
    Top,
    /// The bottom edge is cut.
    Bottom,
    /// Both edges are cut.
    Both,
}

impl Verdict {
    /// The word a trace writes.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Unknown => "unknown",
            Self::Clear => "clear",
            Self::Narrow => "narrow",
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Both => "both",
        }
    }

    /// Whether the panel would warn.
    #[must_use]
    pub fn warns(self) -> bool {
        matches!(self, Self::Narrow | Self::Top | Self::Bottom | Self::Both)
    }
}

/// A candidate's reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Judgement {
    /// What it says.
    pub verdict: Verdict,
    /// How far the signal's low edge is under the passband's level, dB.
    pub low_drop_db: Option<f32>,
    /// How far its high edge is, dB.
    pub high_drop_db: Option<f32>,
    /// How far the last look's band noise stood over the chain's floor, dB (R2, R3).
    pub over_floor_db: Option<f32>,
    /// Looks kept.
    pub kept: usize,
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
/// half the window or more): on a skirt a median
/// keeps the value at its centre where a mean of decibels reads the slope's curve (Gate 3's
/// second build read a filter 8 dB down at 360 Hz as 11), and it takes the noise of a profile of
/// few looks out of the level. The level is the smoothed profile's [`REFERENCE_QUANTILE`] over
/// the span, each edge its value there. `None` where the profile holds no finite bin.
#[must_use]
pub fn span_drops(profile: &[f32], bin_hz: f64, span: Span) -> Option<(f32, f32)> {
    if bin_hz <= 0.0 || profile.is_empty() {
        return None;
    }
    let half = (EDGE_HALF_HZ / bin_hz).round() as usize;
    let bin_of = |hz: f64| ((hz / bin_hz).round().max(0.0) as usize).min(profile.len() - 1);
    let (first, last) = (bin_of(span.low_hz), bin_of(span.high_hz));
    let mut column = Vec::with_capacity(2 * half + 1);
    // half a window or more held by signals reads nothing
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
pub fn verdict_of(low_drop_db: f32, high_drop_db: f32) -> Verdict {
    match (low_drop_db >= LINE_DB, high_drop_db >= LINE_DB) {
        (true, true) => Verdict::Both,
        (false, true) => Verdict::Top,
        (true, false) => Verdict::Bottom,
        (false, false) => Verdict::Clear,
    }
}

/// `spectrum` cut at `top_hz`: today's monitor is handed the bins up to [`TOP_HZ`] whatever a
/// candidate needs.
#[must_use]
pub fn truncated(spectrum: &Spectrum, top_hz: f64) -> Spectrum {
    let top = ((top_hz / spectrum.bin_hz) as usize).min(spectrum.bins_db.len().saturating_sub(1));
    Spectrum {
        bin_hz: spectrum.bin_hz,
        bins_db: spectrum.bins_db[..=top].to_vec(),
    }
}

/// Leave out of `look` (dB a bin) the bins a signal holds: those more than [`OCCUPIED_DB`] over
/// the noise of the [`OCCUPIED_SIDE_HZ`] on each side of them, and the [`OCCUPIED_SPREAD_BINS`]
/// beside each. A bin left out is NaN. One pass: reading the sides again without what it left
/// out (three passes, Gate 3's fourth build) changed nothing on the corpus or the field, since
/// each side's lower quartile already stands clear of a signal beside it.
pub fn leave_out_occupied(look: &mut [f32], bin_hz: f64) {
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

/// One candidate, fed looks as a station takes them.
#[derive(Debug, Clone)]
pub struct Candidate {
    variant: Variant,
    span: Span,
    /// Today's estimate (R0, R1).
    monitor: PassbandMonitor,
    /// Looks kept, each divided by its own level, dB a bin to [`TOP_HZ`]; a bin a signal held is
    /// NaN (R2, R3).
    looks: VecDeque<Vec<f32>>,
    bin_hz: f64,
    /// Looks left out because the band's noise stood too close to the chain's floor.
    weak: usize,
    last_over: Option<f32>,
}

impl Candidate {
    /// A fresh candidate judging `span`.
    #[must_use]
    pub fn new(variant: Variant, span: Span) -> Self {
        Self {
            variant,
            span,
            monitor: PassbandMonitor::new(),
            looks: VecDeque::new(),
            bin_hz: 0.0,
            weak: 0,
            last_over: None,
        }
    }

    /// Which candidate this is.
    #[must_use]
    pub fn variant(&self) -> Variant {
        self.variant
    }

    /// A look of quiet audio, held through the busy detector's attack time (R0, R1, R2; R3
    /// takes every look through [`Self::fold_any`] and ignores this).
    pub fn fold_quiet(&mut self, spectrum: &Spectrum) {
        match self.variant {
            Variant::R0 | Variant::R1 => self.monitor.observe(&truncated(spectrum, TOP_HZ)),
            Variant::R2 => self.fold_normalised(spectrum, false),
            Variant::R3 => {}
        }
    }

    /// A look of any audio the station listened to, quiet or not (R3 only; the others ignore
    /// it).
    pub fn fold_any(&mut self, spectrum: &Spectrum) {
        if self.variant == Variant::R3 {
            self.fold_normalised(spectrum, true);
        }
    }

    /// Divide a look by its own level and keep it if its band's noise stands over the chain's
    /// floor, first leaving out the bins a signal holds when `remove_occupied`.
    fn fold_normalised(&mut self, spectrum: &Spectrum, remove_occupied: bool) {
        if (self.bin_hz - spectrum.bin_hz).abs() > 1e-9 {
            self.looks.clear();
            self.bin_hz = spectrum.bin_hz;
        }
        let bin_hz = spectrum.bin_hz;
        let floor = quantile(
            &mut bins_between(&spectrum.bins_db, bin_hz, FLOOR_BAND_HZ.0, FLOOR_BAND_HZ.1),
            0.5,
        );
        let mut look = truncated(spectrum, TOP_HZ).bins_db;
        if remove_occupied {
            leave_out_occupied(&mut look, bin_hz);
            let in_span = bins_between(&look, bin_hz, self.span.low_hz, self.span.high_hz);
            if in_span.iter().filter(|v| v.is_finite()).count() * 2 < in_span.len() {
                // more than half the span held by signals: nothing to read the noise from
                self.last_over = None;
                return;
            }
        }
        let level = quantile(
            &mut bins_between(&look, bin_hz, self.span.low_hz, self.span.high_hz),
            REFERENCE_QUANTILE,
        );
        // a look whose band's noise stands less than the line over the chain's floor could not
        // show a cut at the line (a margin over it, Gate 3's first builds, changed nothing)
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

    /// Each bin's median over the looks kept that it was free in (R2, R3); NaN where a signal
    /// held it in more than half of them. The looks that kept such a bin are the ones where a
    /// signal's skirt slipped under [`OCCUPIED_DB`]: on a band of FT8 a median of those alone
    /// raised a 500 Hz span's level 12 dB (Gate 3's third build). Counting a held bin as high
    /// instead (the fourth) moved every half-held bin's median up into its tail.
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

    /// What it says now.
    #[must_use]
    pub fn judgement(&self) -> Judgement {
        let waiting = Judgement {
            verdict: Verdict::Waiting,
            low_drop_db: None,
            high_drop_db: None,
            over_floor_db: self.last_over,
            kept: self.kept(),
        };
        let from = |profile: &[f32], bin_hz: f64, verdict: Option<Verdict>| -> Judgement {
            match span_drops(profile, bin_hz, self.span) {
                Some((low, high)) => Judgement {
                    verdict: verdict.unwrap_or_else(|| verdict_of(low, high)),
                    low_drop_db: Some(low),
                    high_drop_db: Some(high),
                    ..waiting
                },
                None => waiting,
            }
        };
        match self.variant {
            Variant::R0 => {
                let centre = f64::midpoint(self.span.low_hz, self.span.high_hz);
                let Some(edges) = self.monitor.edges(centre) else {
                    return waiting;
                };
                let narrow = edges.width_hz() < self.span.width_hz() - WARNING_SLACK_HZ;
                let verdict = if narrow {
                    Verdict::Narrow
                } else {
                    Verdict::Clear
                };
                self.monitor
                    .noise_profile()
                    .map_or(Judgement { verdict, ..waiting }, |(bin_hz, profile)| {
                        from(&profile, bin_hz, Some(verdict))
                    })
            }
            Variant::R1 => {
                if self.monitor.looks() < LOOKS_NEEDED {
                    return waiting;
                }
                self.monitor
                    .noise_profile()
                    .map_or(waiting, |(bin_hz, profile)| from(&profile, bin_hz, None))
            }
            Variant::R2 | Variant::R3 => {
                if self.looks.len() < LOOKS_NEEDED {
                    let verdict = if self.weak >= LOOKS_NEEDED {
                        Verdict::Unknown
                    } else {
                        Verdict::Waiting
                    };
                    return Judgement { verdict, ..waiting };
                }
                from(&self.profile(), self.bin_hz, None)
            }
        }
    }

    /// Looks kept now.
    #[must_use]
    pub fn kept(&self) -> usize {
        match self.variant {
            Variant::R0 | Variant::R1 => self.monitor.looks(),
            Variant::R2 | Variant::R3 => self.looks.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIN_HZ: f64 = 48_000.0 / 4096.0;

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

    fn wide() -> Span {
        Span::around(1500.0, 2280.0)
    }

    #[test]
    fn a_passband_moved_down_reads_as_its_top_cut() {
        let s = look(-60.0, -110.0, low_pass(1600.0));
        let (low, high) = span_drops(&s.bins_db, s.bin_hz, wide()).expect("drops");
        assert!(low < 1.0, "the low edge is in the passband: {low}");
        assert!(high > 30.0, "the top edge is far down: {high}");
        assert_eq!(verdict_of(low, high), Verdict::Top);
    }

    #[test]
    fn a_low_cut_set_high_reads_as_its_bottom_cut() {
        let s = look(-60.0, -110.0, band_pass(600.0, 3000.0));
        let (low, high) = span_drops(&s.bins_db, s.bin_hz, wide()).expect("drops");
        assert_eq!(verdict_of(low, high), Verdict::Bottom, "{low} / {high}");
    }

    #[test]
    fn a_wide_filter_is_clear_and_a_narrow_one_is_cut_both_sides() {
        let s = look(-60.0, -110.0, band_pass(300.0, 3000.0));
        let (low, high) = span_drops(&s.bins_db, s.bin_hz, wide()).expect("drops");
        assert_eq!(verdict_of(low, high), Verdict::Clear, "{low} / {high}");
        let s = look(-60.0, -110.0, band_pass(1250.0, 1750.0));
        let (low, high) = span_drops(&s.bins_db, s.bin_hz, wide()).expect("drops");
        assert_eq!(verdict_of(low, high), Verdict::Both, "{low} / {high}");
    }

    #[test]
    fn a_cut_over_a_shallow_floor_still_reads_past_the_line() {
        // the mobile's chain floor 15 dB under the band's noise: a 20 dB cut reads about 14
        let corner = 2640.0 / 99f64.powf(1.0 / 16.0);
        let s = look(-60.0, -75.0, low_pass(corner));
        let (_, high) = span_drops(&s.bins_db, s.bin_hz, wide()).expect("drops");
        assert!((LINE_DB..16.0).contains(&high), "{high}");
        // and a 10 dB one stays under it
        let corner = 2640.0 / 9f64.powf(1.0 / 16.0);
        let s = look(-60.0, -75.0, low_pass(corner));
        let (_, high) = span_drops(&s.bins_db, s.bin_hz, wide()).expect("drops");
        assert!(high < LINE_DB, "{high}");
    }

    #[test]
    fn looks_whose_band_noise_sits_on_the_chain_floor_say_nothing() {
        let mut c = Candidate::new(Variant::R2, wide());
        for _ in 0..LOOKS_NEEDED + 3 {
            c.fold_quiet(&look(-90.0, -95.0, low_pass(1600.0)));
        }
        let j = c.judgement();
        assert_eq!(j.verdict, Verdict::Unknown, "{j:?}");
        assert_eq!(j.kept, 0);
        // with the band's noise well over the floor the same filter is read
        for _ in 0..LOOKS_NEEDED {
            c.fold_quiet(&look(-60.0, -95.0, low_pass(1600.0)));
        }
        assert_eq!(c.judgement().verdict, Verdict::Top);
    }

    #[test]
    fn a_look_divided_by_its_level_reads_the_same_shape_at_any_level() {
        let mut c = Candidate::new(Variant::R2, wide());
        for k in 0..LOOKS_KEPT {
            // the band's noise swinging 20 dB look to look, an AGC at work
            let band = -60.0 + 20.0 * (k % 2) as f32;
            c.fold_quiet(&look(band, -120.0, band_pass(300.0, 2700.0)));
        }
        let j = c.judgement();
        assert_eq!(j.verdict, Verdict::Clear, "{j:?}");
    }

    #[test]
    fn a_signal_in_a_busy_look_is_left_out_of_the_profile() {
        let mut c = Candidate::new(Variant::R3, wide());
        // busy looks, no quiet one: signals 50 Hz wide every 200 Hz across the span, over the
        // noise of a filter that cuts nothing
        let mut busy = look(-60.0, -110.0, band_pass(300.0, 3000.0));
        for (k, v) in busy.bins_db.iter_mut().enumerate() {
            let hz = k as f64 * BIN_HZ;
            if (450.0..2550.0).contains(&hz) && (hz - 450.0) % 200.0 < 50.0 {
                *v += 30.0;
            }
        }
        for _ in 0..LOOKS_NEEDED {
            c.fold_any(&busy);
        }
        let j = c.judgement();
        assert_eq!(j.verdict, Verdict::Clear, "{j:?}");
        assert!(
            j.low_drop_db.is_some_and(|d| d.abs() < 1.0)
                && j.high_drop_db.is_some_and(|d| d.abs() < 1.0),
            "{j:?}"
        );
    }

    #[test]
    fn an_edge_a_signal_holds_in_every_look_is_not_read() {
        // a station parked on the top edge would read as the passband standing there
        let mut c = Candidate::new(Variant::R3, wide());
        let mut busy = look(-60.0, -110.0, low_pass(1600.0));
        for (k, v) in busy.bins_db.iter_mut().enumerate() {
            if (2580.0..2700.0).contains(&(k as f64 * BIN_HZ)) {
                *v = -40.0;
            }
        }
        for _ in 0..LOOKS_NEEDED {
            c.fold_any(&busy);
        }
        assert_eq!(c.judgement().verdict, Verdict::Waiting);
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

    #[test]
    fn variants_are_named_as_the_trace_takes_them() {
        assert_eq!("r2".parse::<Variant>(), Ok(Variant::R2));
        assert_eq!("R3".parse::<Variant>(), Ok(Variant::R3));
        assert!("r9".parse::<Variant>().is_err());
    }
}
