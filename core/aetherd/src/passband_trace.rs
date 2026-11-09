//! The receive-filter estimate over a recording (`aetherd --passband-trace`).
//!
//! The panel shows one number, the estimated width, and only when it falls under the signal's.
//! That says nothing of how the estimate got there: which looks it took, what each held, where
//! it drew the edges and against what level. This runs a recording's audio through the same
//! [`SpectrumAnalyser`] and [`PassbandMonitor`] a live station runs, with the looks taken by the
//! station's own rule (`Station::sample_passband`) as near as a sidecar allows: the busy
//! detector over the same audio, deaf where the transmitter was keyed and for its tail, the
//! station counted as receiving from a heeded announcement for the frame's length (at least the
//! air's longest) and for half a second after a frame that decoded or was trusted. It reports
//! every look the monitor folds in, with the edges as they stood after it (the receive-filter
//! review, `docs/research/receive-filter/`).
//!
//! With a [`Variant`] other than today's it also runs one of the review's candidates
//! (`crate::passband`) beside the monitor, on the same looks, and reports what the candidate says
//! after each. R3 takes a look every half second whenever the station listens, quiet or not, holds
//! it for the air's longest frame and drops it if a frame announced or reported by then lay under
//! it. A candidate takes no look while the audio heard clips (C1), unless told to go on
//! (`--no-clip-veto`); C1's share of samples at full scale is on every line.

use std::fmt::Write as _;
use std::path::Path;

use aether_phy::{AudioToBaseband, WIDE_2300, WaveformParams};

use crate::busy::{BusyConfig, BusyDetector};
use crate::passband::{Candidate, Clipping, Judgement, SPECTRUM_TOP_HZ, Span, Variant};
use crate::record::read_wav;
use crate::replay::Expectation;
use crate::spectrum::{PassbandEdges, PassbandMonitor, Spectrum, SpectrumAnalyser};
use crate::station::{
    AUDIO_CENTER_HZ, DETECT_CONFIDENCE_TRUSTED, PASSBAND_MIN_LEVEL_DBFS, PASSBAND_SAMPLE_S,
};

/// The audio a live station hands over at a time: 20 ms, as the daemon's loop does.
const FEED_S: f64 = 0.02;
/// How long after a decoded or trusted frame is reported the station still counts itself as
/// receiving, as `Station::report` sets it.
const RECEIVING_AFTER_FRAME_S: f64 = 0.5;

/// One look the monitor folded in, and the estimate after it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    /// When the look was folded in, seconds of the recording.
    pub t_s: f64,
    /// When the look's audio ended (it covers the 85 ms before), seconds of the recording.
    pub taken_s: f64,
    /// Looks kept after it.
    pub looks: usize,
    /// The look's own level around the centre, dBFS ([`Spectrum::level_around`]).
    pub look_db: f32,
    /// The estimate after it: `None` until enough looks are kept.
    pub edges: Option<PassbandEdges>,
    /// What the candidate traced says after it.
    pub judgement: Judgement,
    /// The share of the audio heard over the last ten seconds at full scale (C1).
    pub clipping: f64,
    /// Whether no look was taken here, the audio clipping (a row of C1's alone).
    pub clipped: bool,
}

/// The waveform of a bandwidth.
fn params_for(bandwidth_hz: u32) -> Result<WaveformParams, String> {
    match bandwidth_hz {
        2300 => Ok(WIDE_2300),
        500 => Ok(aether_phy::waveform::NARROW_500),
        other => Err(format!("no waveform for {other} Hz")),
    }
}

/// Run a recording through the receive-filter estimate, and the candidate `variant` beside it;
/// with `clip_veto` the candidate takes no look while the audio clips (C1, as registered).
///
/// `expectation` is the recording's sidecar when there is one: the keyed spans are deaf, the
/// frames and announcements hold the station receiving and are explained to the busy detector,
/// the decoded ones mark the channel, and the busy detector starts from what the live one had
/// learned when the recording began. Without one the whole recording is listened to.
///
/// # Errors
/// If the file cannot be read or is not at the modem's rate.
pub fn trace(
    wav: &Path,
    expectation: Option<&Expectation>,
    bandwidth_hz: Option<u32>,
    variant: Variant,
    clip_veto: bool,
) -> Result<Vec<Reading>, String> {
    let bandwidth = bandwidth_hz
        .or_else(|| expectation.map(|e| e.bandwidth_hz))
        .unwrap_or(2300);
    let params = params_for(bandwidth)?;
    let audio = read_wav(wav).map_err(|e| format!("{}: {e}", wav.display()))?;
    let rate = params.audio_rate as f64;
    if usize::try_from(audio.sample_rate).ok() != Some(params.audio_rate) {
        return Err(format!(
            "{} is at {} Hz; the modem runs at {rate} Hz",
            wav.display(),
            audio.sample_rate
        ));
    }
    let mut front = AudioToBaseband::new(params);
    let mut busy = BusyDetector::new(BusyConfig {
        fs: params.fs_baseband,
        passband_hz: params.occupied_bandwidth_hz(),
        centre_hz: params.centre_hz,
        ..BusyConfig::default()
    });
    let mut heard = Heard::new(expectation, params);
    let span = Span::around(params.centre_hz, params.occupied_bandwidth_hz());
    let mut looks = Looks::new(rate, Candidate::new(variant, span));
    let mut clipping = Clipping::new(rate);
    let muted = expectation.map(|e| e.muted.clone()).unwrap_or_default();
    let mut state = expectation.and_then(|e| e.busy_state.as_ref());

    let feed = (FEED_S * rate) as usize;
    let mut readings = Vec::new();
    let mut seen = 0usize;
    for chunk in audio.samples.chunks(feed) {
        let t = seen as f64 / rate;
        seen += chunk.len();
        let now = seen as f64 / rate;
        looks.analyser.push(chunk);
        if let Some(live) = state.filter(|s| s.at_s <= now) {
            busy.restore(live, rate);
            state = None;
        }
        let keyed = muted.iter().any(|m| t >= m.from_s && t < m.until_s);
        let deaf = crate::busy_trace::deaf_at(&muted, t);
        let baseband = front.process(chunk);
        if deaf {
            busy.skip(&baseband);
        } else {
            busy.hear_passband(chunk, rate, now);
            busy.push(&baseband, now);
        }
        heard.until(now, &mut busy);
        if !keyed && !deaf {
            clipping.hear(chunk);
        }
        looks.clipping = clipping.share();
        // Station::sample_passband, look for look; R3 needs no level, its floor test refusing
        // a silent input; a candidate takes no look while the audio clips (C1), today's
        // estimate (R0) does
        let clips = clip_veto && clipping.clips() && variant != Variant::R0;
        let listening = !keyed && now >= heard.receiving_until && !deaf && !clips;
        let quiet = listening
            && busy.settled()
            && !busy.busy(now)
            && busy.level_db > PASSBAND_MIN_LEVEL_DBFS;
        looks.step(now, quiet, busy.attack_s(), &mut readings);
        looks.listen(
            now,
            listening,
            heard.any_frame_s + RECEIVING_AFTER_FRAME_S,
            &heard.spans,
            &mut readings,
        );
        looks.clips(now, clips && !keyed && !deaf, &mut readings);
    }
    Ok(readings)
}

/// What the station heard, from the sidecar, fed to the busy detector as the station fed it and
/// kept as how long the station counted itself receiving.
struct Heard<'a> {
    frames: Vec<crate::record::FrameRecord>,
    next_frame: usize,
    announcements: &'a [crate::replay::Announcement],
    next_announcement: usize,
    /// The air's longest frame: a heeded announcement holds the station at least this long.
    longest_s: f64,
    receiving_until: f64,
    /// Where every frame announced or reported so far lay, seconds of the recording (R3).
    spans: Vec<(f64, f64)>,
    /// The air's longest frame of any family, the tone floor's included: how long a look is
    /// held before no frame under it can still be reported (R3).
    any_frame_s: f64,
}

impl<'a> Heard<'a> {
    fn new(expectation: Option<&'a Expectation>, params: WaveformParams) -> Self {
        let mut frames = expectation.map(|e| e.frames.clone()).unwrap_or_default();
        frames.sort_by(|a, b| a.t_s.total_cmp(&b.t_s));
        Self {
            frames,
            next_frame: 0,
            announcements: expectation.map_or(&[][..], |e| &e.announcements[..]),
            next_announcement: 0,
            longest_s: aether_phy::modes::air_interface(params).long.duration_s(),
            receiving_until: f64::NEG_INFINITY,
            spans: Vec::new(),
            any_frame_s: {
                let air = aether_phy::modes::air_interface(params);
                air.tone_kinds()
                    .iter()
                    .map(|k| k.duration_s())
                    .fold(air.long.duration_s(), f64::max)
            },
        }
    }

    /// Take in what was announced or reported by `now`.
    fn until(&mut self, now: f64, busy: &mut BusyDetector) {
        while let Some(announced) = self
            .announcements
            .get(self.next_announcement)
            .filter(|a| a.t_s <= now)
        {
            busy.explain(announced.start_s, announced.end_s);
            self.spans.push((announced.start_s, announced.end_s));
            if announced.heeded {
                let frame_s = announced.end_s - announced.start_s;
                self.receiving_until = self
                    .receiving_until
                    .max(announced.t_s + frame_s.max(self.longest_s));
            }
            self.next_announcement += 1;
        }
        while let Some(frame) = self.frames.get(self.next_frame).filter(|f| f.t_s <= now) {
            if let (Some(start), Some(end)) = (frame.start_s, frame.end_s) {
                busy.explain(start, end);
                self.spans.push((start, end));
            }
            if frame.decoded || frame.detect_confidence >= DETECT_CONFIDENCE_TRUSTED {
                self.receiving_until = self
                    .receiving_until
                    .max(frame.t_s + RECEIVING_AFTER_FRAME_S);
            }
            if frame.decoded {
                busy.mark_frame(frame.t_s, frame.detect_confidence);
            }
            self.next_frame += 1;
        }
    }
}

/// The looks, taken and folded in by `Station::sample_passband`'s rule, and the candidate's.
struct Looks {
    analyser: SpectrumAnalyser,
    monitor: PassbandMonitor,
    candidate: Candidate,
    quiet_since: f64,
    next_look: f64,
    /// Looks taken, waiting for the quiet to hold the busy detector's attack time.
    pending: std::collections::VecDeque<(f64, Spectrum)>,
    /// Since when the station has listened, quiet or not (R3).
    listening_since: f64,
    next_any: f64,
    /// R3's looks, held until no frame can still be found under them.
    held: std::collections::VecDeque<(f64, Spectrum)>,
    /// The share of the audio clipping now (C1), and when its next row is due.
    clipping: f64,
    next_clip: f64,
}

impl Looks {
    fn new(rate: f64, candidate: Candidate) -> Self {
        Self {
            analyser: SpectrumAnalyser::new(rate),
            monitor: PassbandMonitor::new(),
            candidate,
            quiet_since: f64::NEG_INFINITY,
            next_look: 0.0,
            pending: std::collections::VecDeque::new(),
            listening_since: f64::NEG_INFINITY,
            next_any: 0.0,
            held: std::collections::VecDeque::new(),
            clipping: 0.0,
            next_clip: 0.0,
        }
    }

    /// C1's rows: one every [`PASSBAND_SAMPLE_S`] while the audio heard clips, no look taken.
    fn clips(&mut self, now: f64, clips: bool, readings: &mut Vec<Reading>) {
        if clips && now >= self.next_clip {
            readings.push(Reading {
                t_s: now,
                taken_s: now,
                looks: self.monitor.looks(),
                look_db: f32::NAN,
                edges: self.monitor.edges(AUDIO_CENTER_HZ),
                judgement: self.candidate.judgement(),
                clipping: self.clipping,
                clipped: true,
            });
            self.next_clip = now + PASSBAND_SAMPLE_S;
        }
    }

    /// The spectrum the candidate needs: today's, or up to [`SPECTRUM_TOP_HZ`] for one that
    /// reads the chain's floor. Today's monitor is handed the same bins up to its own top.
    fn compute(&self) -> Option<Spectrum> {
        if self.candidate.variant().reads_floor() {
            self.analyser.compute_to(SPECTRUM_TOP_HZ)
        } else {
            self.analyser.compute()
        }
    }

    /// The reading after a look.
    fn reading(&self, now: f64, taken: f64, spectrum: &Spectrum) -> Reading {
        Reading {
            t_s: now,
            taken_s: taken,
            looks: self.monitor.looks(),
            look_db: spectrum.level_around(AUDIO_CENTER_HZ),
            edges: self.monitor.edges(AUDIO_CENTER_HZ),
            judgement: self.candidate.judgement(),
            clipping: self.clipping,
            clipped: false,
        }
    }

    /// R3's looks: one every [`PASSBAND_SAMPLE_S`] while the station listens, once a whole
    /// window of it has been heard. Each is held for `hold_s` (the air's longest frame and the
    /// half second a report takes) and dropped if a frame announced or reported by then lay
    /// under it: a look under the other station's frame is the frame's spectrum, whose edges
    /// are the span's own (Gate 3's harness, where half of R3's looks fell under frames
    /// announced only part way in and a 10 dB cut read 15–19).
    fn listen(
        &mut self,
        now: f64,
        listening: bool,
        hold_s: f64,
        spans: &[(f64, f64)],
        readings: &mut Vec<Reading>,
    ) {
        if self.candidate.variant() != Variant::R3 {
            return;
        }
        let window = self.analyser.window_s();
        while self
            .held
            .front()
            .is_some_and(|&(taken, _)| now - taken >= hold_s)
        {
            let Some((taken, spectrum)) = self.held.pop_front() else {
                break;
            };
            if spans
                .iter()
                .any(|&(start, end)| start < taken && end > taken - window)
            {
                continue;
            }
            self.candidate.fold_any(&spectrum);
            readings.push(self.reading(now, taken, &spectrum));
        }
        if !listening {
            self.listening_since = now;
            return;
        }
        if now >= self.next_any
            && now - self.listening_since >= window
            && let Some(spectrum) = self.compute()
        {
            self.held.push_back((now, spectrum));
            self.next_any = now + PASSBAND_SAMPLE_S;
        }
    }

    /// One pass at `now`: forget the pending looks if the channel is not quiet, fold in those
    /// the quiet has held for `attack` seconds, and take a new one when one is due.
    fn step(&mut self, now: f64, quiet: bool, attack: f64, readings: &mut Vec<Reading>) {
        if !quiet {
            self.quiet_since = now;
            self.pending.clear();
            return;
        }
        while self
            .pending
            .front()
            .is_some_and(|&(taken, _)| now - taken >= attack)
        {
            if let Some((taken, spectrum)) = self.pending.pop_front() {
                self.monitor.observe(&crate::passband::truncated(
                    &spectrum,
                    crate::spectrum::TOP_HZ,
                ));
                self.candidate.fold_quiet(&spectrum);
                // R3 reports its own looks
                if self.candidate.variant() != Variant::R3 {
                    readings.push(self.reading(now, taken, &spectrum));
                }
            }
        }
        if now >= self.next_look
            && now - self.quiet_since >= self.analyser.window_s()
            && let Some(spectrum) = self.compute()
        {
            self.pending.push_back((now, spectrum));
            self.next_look = now + PASSBAND_SAMPLE_S;
        }
    }
}

/// The trace as CSV, one line a look.
#[must_use]
pub fn to_csv(readings: &[Reading]) -> String {
    let mut out = String::from(
        "t_s,taken_s,looks,look_db,low_hz,high_hz,width_hz,reference_db,threshold_db,floor_db,\
         verdict,low_drop_db,high_drop_db,over_floor_db,kept,clipping_pct,clipped\n",
    );
    for r in readings {
        let edges = r.edges.map_or_else(
            || ",,,,,".to_owned(),
            |e| {
                format!(
                    "{:.0},{:.0},{:.0},{:.2},{:.2},{:.2}",
                    e.low_hz,
                    e.high_hz,
                    e.width_hz(),
                    e.reference_db,
                    e.threshold_db,
                    e.floor_db
                )
            },
        );
        let db = |v: Option<f32>| v.map_or_else(String::new, |v| format!("{v:.2}"));
        let j = &r.judgement;
        // writing to a String cannot fail
        let _ = writeln!(
            out,
            "{:.3},{:.3},{},{:.2},{},{},{},{},{},{},{:.3},{}",
            r.t_s,
            r.taken_s,
            r.looks,
            r.look_db,
            edges,
            j.verdict.word(),
            db(j.low_drop_db),
            db(j.high_drop_db),
            db(j.over_floor_db),
            j.kept,
            r.clipping * 100.0,
            u8::from(r.clipped)
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// `seconds` of band noise through a receiver whose passband is `low`–`high` Hz (four
    /// sections a side), over the audio chain's own white floor 50 dB down.
    fn receiver_noise(seconds: f64, low: f64, high: f64, rate: f64) -> Vec<f32> {
        let mut sections = [
            Biquad::new(true, low, rate),
            Biquad::new(true, low, rate),
            Biquad::new(true, low, rate),
            Biquad::new(true, low, rate),
            Biquad::new(false, high, rate),
            Biquad::new(false, high, rate),
            Biquad::new(false, high, rate),
            Biquad::new(false, high, rate),
        ];
        let mut seed = 0x2545_F491_4F6C_DD1D_u64;
        let mut uniform = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        (0..(seconds * rate) as usize)
            .map(|_| {
                let band = sections.iter_mut().fold(uniform(), |x, s| s.run(x));
                (0.3 * band + 0.001 * uniform()) as f32
            })
            .collect()
    }

    /// The trace of `audio` recorded with a sidecar whose transmitter was keyed over `keyed`
    /// (seconds of the recording), if any.
    fn trace_of(audio: &[f32], name: &str, keyed: Option<(f64, f64)>) -> Vec<Reading> {
        let dir = std::env::temp_dir().join(format!(
            "aether-passband-trace-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let mut recording = crate::record::Recording::start(
            &dir,
            name,
            48_000,
            0.0,
            serde_json::json!({"bandwidth_hz": 2300}),
        )
        .expect("start");
        recording.captured(audio).expect("write");
        if let Some((from, until)) = keyed {
            recording.event(from, "ptt", "keyed", "Connected");
            recording.event(until, "ptt", "released", "Connected");
        }
        let summary = recording.finish(&serde_json::json!({})).expect("finish");
        let expectation = Expectation::from_sidecar(&summary.sidecar).expect("sidecar");
        let readings =
            trace(&summary.wav, Some(&expectation), None, Variant::R0, true).expect("trace");
        let _ = std::fs::remove_dir_all(&dir);
        readings
    }

    /// How much audio one look covers at 48 kHz.
    const WINDOW_S: f64 = crate::spectrum::WINDOW as f64 / 48_000.0;

    #[test]
    fn a_trace_reports_the_estimates_edges_look_by_look() {
        // the instrument, not the estimate: it folds in looks of quiet audio, reports both
        // edges once enough are kept, and the edges it reports bracket a 300-2700 Hz passband
        // (today's rule draws them where the noise is halfway down to the floor, outside the
        // filter's -6 dB points: the receive-filter review's evidence)
        let rate = 48_000.0;
        let readings = trace_of(&receiver_noise(20.0, 300.0, 2700.0, rate), "wide", None);
        assert!(
            readings.len() >= 20,
            "only {} looks in 20 s of quiet",
            readings.len()
        );
        assert!(readings.iter().all(|r| r.taken_s < r.t_s));
        assert!(
            readings
                .iter()
                .all(|r| r.edges.is_some() == (r.looks >= 12)),
            "an estimate before twelve looks, or none after"
        );
        let edges = readings.last().and_then(|r| r.edges).expect("an estimate");
        assert!(
            edges.low_hz <= 450.0 && edges.high_hz >= 2550.0,
            "a 300-2700 Hz passband read {:.0}-{:.0} Hz",
            edges.low_hz,
            edges.high_hz
        );
        assert!(edges.threshold_db < edges.reference_db && edges.floor_db < edges.threshold_db);
    }

    #[test]
    fn no_look_is_taken_while_the_station_is_keyed_or_deaf_after() {
        let rate = 48_000.0;
        let readings = trace_of(
            &receiver_noise(20.0, 300.0, 2700.0, rate),
            "keyed",
            Some((8.0, 12.0)),
        );
        let deaf_after = crate::audio::DEVICE_LATENCY_S + crate::station::CAPTURE_LAG_ALLOWANCE_S;
        assert!(readings.iter().any(|r| r.taken_s < 8.0));
        assert!(readings.iter().any(|r| r.taken_s > 13.0));
        for r in &readings {
            assert!(
                r.taken_s - WINDOW_S >= 12.0 + deaf_after || r.taken_s <= 8.0,
                "a look of {:.2}-{:.2} s overlaps the keying or the deafness after it",
                r.taken_s - WINDOW_S,
                r.taken_s
            );
        }
    }

    #[test]
    fn a_csv_line_carries_the_edges_once_there_are_any() {
        let edges = PassbandEdges {
            low_hz: 300.0,
            high_hz: 2700.0,
            reference_db: -40.0,
            threshold_db: -46.0,
            floor_db: -90.0,
        };
        let waiting = Judgement {
            verdict: crate::passband::Verdict::Waiting,
            low_drop_db: None,
            high_drop_db: None,
            over_floor_db: None,
            kept: 1,
        };
        let csv = to_csv(&[
            Reading {
                t_s: 1.0,
                taken_s: 0.5,
                looks: 1,
                look_db: -40.0,
                edges: None,
                judgement: waiting,
                clipping: 0.0,
                clipped: false,
            },
            Reading {
                t_s: 9.0,
                taken_s: 8.5,
                looks: 12,
                look_db: -40.5,
                edges: Some(edges),
                judgement: Judgement {
                    verdict: crate::passband::Verdict::Top,
                    low_drop_db: Some(0.4),
                    high_drop_db: Some(23.0),
                    over_floor_db: Some(48.0),
                    kept: 12,
                },
                clipping: 0.015,
                clipped: false,
            },
        ]);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1], "1.000,0.500,1,-40.00,,,,,,,waiting,,,,1,0.000,0");
        assert_eq!(
            lines[2],
            "9.000,8.500,12,-40.50,300,2700,2400,-40.00,-46.00,-90.00,top,0.40,23.00,48.00,12,\
             1.500,0"
        );
    }
}
