//! The busy detector over a recording, block by block (`aetherd --busy-trace`).
//!
//! The panel draws the floor twice a second and the log says only when the busy state
//! changed; neither says *why* a floor went where it went. This runs a recording's audio
//! through the same front end and the same [`BusyDetector`] a live station runs — muted where
//! the sidecar says the transmitter was keyed, with the frames the station found explained and
//! the decoded ones marked, as they were at the time — and reports every 25 ms block: its
//! power, each test that decided whether it was noise, the floor and the busy state. Beside
//! that it measures the radio's whole audio passband from the 48 kHz audio, in Aether's channel
//! and outside it, because a receiver's AGC answers to all of it: a signal the detector never
//! sees can still turn the radio's gain down (`tools/floor_trace.py` reads the result).

use std::fmt::Write as _;
use std::path::Path;

use aether_phy::{AudioToBaseband, WIDE_2300, WaveformParams};

use crate::busy::{BlockNote, BusyConfig, BusyDetector, BusyReason};
use crate::record::read_wav;
use crate::replay::Expectation;

/// The audio a live station hands the detector at a time: 20 ms, as the daemon's loop does.
const FEED_S: f64 = 0.02;
/// How long a live station stays deaf to its own tail after the key is released: the card's
/// latency and the capture's allowance, the sum `Station` uses (every sidecar's `tx_end`
/// says `deaf_for 0.750`). It was 0.3 s here, which heard 0.45 s of each gap a live station
/// never hears; in a session those gaps are nearly all the noise there is, and the trace
/// missed the floor taking the other station's signal (report 14 §7).
const DEAF_AFTER_S: f64 = crate::audio::DEVICE_LATENCY_S + crate::station::CAPTURE_LAG_ALLOWANCE_S;
/// The radio's audio passband, for the wide measurements: a 3 kHz SSB filter's.
const PASSBAND_HZ: (f64, f64) = (200.0, 3000.0);
/// How far outside Aether's channel the out-of-channel measurement begins, so the channel's
/// own skirts do not count.
const GUARD_HZ: f64 = 100.0;

/// One block of the trace.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceRow {
    /// The detector's verdict on the block.
    pub note: BlockNote,
    /// The floor after the block, dBFS.
    pub floor_db: f64,
    /// The busy threshold after the block, dBFS.
    pub threshold_db: f64,
    /// Whether the channel read busy at the block's end.
    pub busy: bool,
    /// Why it was last marked busy.
    pub reason: Option<BusyReason>,
    /// The latest shape reading, dB.
    pub shape_db: f64,
    /// Power of the 48 kHz audio inside Aether's channel over the block, dBFS.
    pub channel_db: f64,
    /// Power of the 48 kHz audio in the rest of the radio's passband, dBFS.
    pub outside_db: f64,
    /// The median 40 Hz bin of the radio's passband, scaled to the channel's width, dBFS:
    /// noise is flat across the passband and a narrowband signal holds a few bins, so this
    /// follows the radio's gain whatever narrowband signal is on top.
    pub median_db: f64,
}

/// The detector's settings by name: `plain` (the floor in dBFS, as before ADR-0078),
/// `normalised` (the floor following the radio's gain), or empty for the default.
///
/// # Errors
/// For a name it does not know.
pub fn variant(name: &str) -> Result<BusyConfig, String> {
    use crate::busy::GainHandling;
    let base = BusyConfig::default();
    Ok(match name {
        "" => base,
        "plain" => BusyConfig {
            gain: GainHandling::Plain,
            ..base
        },
        "normalised" => BusyConfig {
            gain: GainHandling::Normalised,
            ..base
        },
        other => return Err(format!("no busy variant {other:?}")),
    })
}

/// The waveform of a bandwidth.
fn params_for(bandwidth_hz: u32) -> Result<WaveformParams, String> {
    match bandwidth_hz {
        2300 => Ok(WIDE_2300),
        500 => Ok(aether_phy::waveform::NARROW_500),
        other => Err(format!("no waveform for {other} Hz")),
    }
}

/// Run a recording through the busy detector.
///
/// `expectation` is the recording's sidecar when there is one: the keyed spans are muted, the
/// frames it found and the ones it announced are explained and the decoded ones mark the
/// channel, as the station did, its bandwidth is used unless `bandwidth_hz` says otherwise,
/// and where it kept what the live detector had learned when the recording began, the trace
/// takes that over at that moment — a detector that has heard only the recording knows
/// nothing of the minutes before it (report 14 §7.1).
///
/// # Errors
/// If the file cannot be read or is not at the modem's rate.
pub fn trace(
    wav: &Path,
    expectation: Option<&Expectation>,
    bandwidth_hz: Option<u32>,
    config: BusyConfig,
) -> Result<Vec<TraceRow>, String> {
    let bandwidth = bandwidth_hz
        .or_else(|| expectation.map(|e| e.bandwidth_hz))
        .unwrap_or(500);
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
        ..config
    });
    let mut wide = WideBands::new(params, rate, config.block_s);
    let muted = expectation.map(|e| e.muted.clone()).unwrap_or_default();
    // the frames, in the order the station reported them
    let mut frames = expectation.map(|e| e.frames.clone()).unwrap_or_default();
    frames.sort_by(|a, b| a.t_s.total_cmp(&b.t_s));
    let mut next_frame = 0;
    let announcements = expectation.map_or(&[][..], |e| &e.announcements[..]);
    let mut next_announcement = 0;
    let mut state = expectation.and_then(|e| e.busy_state.as_ref());

    let feed = (FEED_S * rate) as usize;
    let mut rows = Vec::new();
    let mut seen = 0usize;
    let mut last_end = f64::NEG_INFINITY;
    for chunk in audio.samples.chunks(feed) {
        let t = seen as f64 / rate;
        seen += chunk.len();
        let now = seen as f64 / rate;
        wide.push(chunk);
        if let Some(live) = state.filter(|s| s.at_s <= now) {
            busy.restore(live, rate);
            state = None;
        }
        if !deaf_at(&muted, t) {
            busy.hear_passband(chunk, rate, now);
        }
        let baseband = front.process(chunk);
        let deaf = deaf_at(&muted, t);
        if deaf {
            busy.skip(&baseband);
        } else {
            busy.push(&baseband, now);
        }
        while let Some(announced) = announcements
            .get(next_announcement)
            .filter(|a| a.t_s <= now)
        {
            busy.explain(announced.start_s, announced.end_s);
            next_announcement += 1;
        }
        while let Some(frame) = frames.get(next_frame).filter(|f| f.t_s <= now) {
            if let (Some(start), Some(end)) = (frame.start_s, frame.end_s) {
                busy.explain(start, end);
            }
            if frame.decoded {
                busy.mark_frame(frame.t_s, frame.detect_confidence);
            }
            next_frame += 1;
        }
        if let Some(note) = busy.last_block.filter(|n| n.end_s > last_end) {
            last_end = note.end_s;
            let (channel_db, outside_db, median_db) = wide.at(note.end_s);
            rows.push(TraceRow {
                note,
                floor_db: busy.floor_db,
                threshold_db: busy.floor_db + busy.config().threshold_db,
                busy: busy.busy(now),
                reason: busy.reason(),
                shape_db: busy.shape_db,
                channel_db,
                outside_db,
                median_db,
            });
        }
    }
    Ok(rows)
}

/// Whether a live station was deaf at `t`: keyed, or hearing its own tail.
fn deaf_at(muted: &[crate::replay::Muted], t: f64) -> bool {
    muted
        .iter()
        .any(|m| t >= m.from_s && t < m.until_s + DEAF_AFTER_S)
}

/// The trace as CSV, one line a block.
#[must_use]
pub fn to_csv(rows: &[TraceRow]) -> String {
    let mut out = String::from(
        "t_s,power_db,floor_db,threshold_db,steady,over,peaked,signal,in_frame,evidence,held,\
         busy,reason,shape_db,channel_db,outside_db,median_db,gain_db,drawn_db\n",
    );
    let flag = |b: bool| if b { "1" } else { "0" };
    for row in rows {
        let n = &row.note;
        let reason = match row.reason {
            Some(BusyReason::Level { .. }) => "level",
            Some(BusyReason::Shape { .. }) => "shape",
            Some(BusyReason::Frame { .. }) => "frame",
            None => "",
        };
        // writing to a String cannot fail
        let _ = writeln!(
            out,
            "{:.3},{:.2},{:.2},{:.2},{},{},{},{},{},{},{},{},{},{:.1},{:.2},{:.2},{:.2},{:.2},{:.2}",
            n.end_s,
            n.power_db,
            row.floor_db,
            row.threshold_db,
            flag(n.steady),
            flag(n.over),
            flag(n.peaked),
            flag(n.signal),
            flag(n.in_frame),
            flag(n.evidence),
            flag(n.held),
            flag(row.busy),
            reason,
            row.shape_db,
            row.channel_db,
            row.outside_db,
            row.median_db,
            n.gain_db,
            n.drawn_db,
        );
    }
    out
}

/// The audio's power by band, block by block: inside Aether's channel and in the rest of the
/// radio's passband.
struct WideBands {
    block: usize,
    rate: f64,
    pending: Vec<f32>,
    fft: std::sync::Arc<dyn rustfft::Fft<f64>>,
    channel: (f64, f64),
    /// (block end in seconds, channel dBFS, outside dBFS, median dBFS), newest last.
    measured: std::collections::VecDeque<(f64, f64, f64, f64)>,
    seen: usize,
}

impl WideBands {
    fn new(params: WaveformParams, rate: f64, block_s: f64) -> Self {
        let block = (block_s * rate) as usize;
        let half = params.occupied_bandwidth_hz() / 2.0;
        Self {
            block,
            rate,
            pending: Vec::new(),
            fft: rustfft::FftPlanner::new().plan_fft_forward(block),
            channel: (params.centre_hz - half, params.centre_hz + half),
            measured: std::collections::VecDeque::new(),
            seen: 0,
        }
    }

    fn push(&mut self, audio: &[f32]) {
        self.pending.extend_from_slice(audio);
        while self.pending.len() >= self.block {
            let mut spectrum: Vec<rustfft::num_complex::Complex64> = self.pending[..self.block]
                .iter()
                .enumerate()
                .map(|(i, &x)| {
                    let w = 0.5
                        - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / self.block as f64).cos();
                    rustfft::num_complex::Complex64::new(f64::from(x) * w, 0.0)
                })
                .collect();
            self.pending.drain(..self.block);
            self.seen += self.block;
            self.fft.process(&mut spectrum);
            let bin_hz = self.rate / self.block as f64;
            // a Hann window's power gain, so a band's power reads as the signal's own
            let norm = 2.0 / (0.375 * self.block as f64 * self.block as f64);
            let (mut inside, mut outside) = (0.0, 0.0);
            let mut bins = Vec::new();
            for (k, value) in spectrum.iter().enumerate().take(self.block / 2) {
                let hz = k as f64 * bin_hz;
                let power = value.norm_sqr() * norm;
                if hz >= PASSBAND_HZ.0 && hz <= PASSBAND_HZ.1 {
                    bins.push(power);
                }
                if hz >= self.channel.0 && hz <= self.channel.1 {
                    inside += power;
                } else if hz >= PASSBAND_HZ.0
                    && hz <= PASSBAND_HZ.1
                    && (hz < self.channel.0 - GUARD_HZ || hz > self.channel.1 + GUARD_HZ)
                {
                    outside += power;
                }
            }
            let db = |p: f64| 10.0 * p.max(1e-20).log10();
            bins.sort_by(f64::total_cmp);
            let median = bins.get(bins.len() / 2).copied().unwrap_or(0.0)
                * (self.channel.1 - self.channel.0)
                / bin_hz;
            self.measured.push_back((
                self.seen as f64 / self.rate,
                db(inside),
                db(outside),
                db(median),
            ));
            if self.measured.len() > 64 {
                self.measured.pop_front();
            }
        }
    }

    /// The measurement of the block ending nearest `end_s`.
    fn at(&self, end_s: f64) -> (f64, f64, f64) {
        self.measured
            .iter()
            .min_by(|a, b| (a.0 - end_s).abs().total_cmp(&(b.0 - end_s).abs()))
            .map_or((f64::NAN, f64::NAN, f64::NAN), |&(_, c, o, m)| (c, o, m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trace_starts_from_the_recorded_state_and_explains_what_was_announced() {
        // report 14 §7.1: the trace starts from what the live detector had learned when the
        // recording began, and takes the frames the station announced out of the floor as the
        // station did
        let params = aether_phy::waveform::NARROW_500;
        let config = BusyConfig {
            fs: params.fs_baseband,
            passband_hz: params.occupied_bandwidth_hz(),
            centre_hz: params.centre_hz,
            ..BusyConfig::default()
        };
        // the live detector heard 15 s of louder noise before the recording began at 10 s
        let mut live = BusyDetector::new(config);
        let per_block = (config.block_s * config.fs) as usize;
        let mut now = -5.0;
        let mut seed = 3u64;
        while now < 10.5 {
            now += config.block_s;
            let block = (0..per_block)
                .map(|_| {
                    seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                    let u = (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5;
                    (0.05 * u, 0.05 * u)
                })
                .collect::<Vec<_>>();
            live.push(&block, now);
        }
        assert!(
            live.floor_db.is_finite(),
            "the live detector learned no floor"
        );

        let dir = std::env::temp_dir().join(format!("aether-trace-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let mut recording = crate::record::Recording::start(
            &dir,
            "trace",
            48_000,
            10.0,
            serde_json::json!({"bandwidth_hz": 500}),
        )
        .expect("start");
        recording.busy_state(live.state(now));
        let rate = 48_000.0;
        let audio: Vec<f32> = (0..(4.0 * rate) as usize)
            .map(|i| {
                (0.01
                    * (f64::from(i as u32) * 1.7).sin().signum()
                    * (f64::from(i as u32) * 0.37).cos()) as f32
            })
            .collect();
        recording.captured(&audio).expect("write");
        recording.event(
            12.0,
            "preamble",
            "ago 0.300 frame_s 1.000 tone 1 confidence 1.40 heeded 1",
            "Idle",
        );
        let summary = recording.finish(&serde_json::json!({})).expect("finish");
        let expectation = Expectation::from_sidecar(&summary.sidecar).expect("sidecar");
        assert!(expectation.busy_state.is_some(), "the state was not kept");
        assert_eq!(expectation.announcements.len(), 1);

        let rows = trace(&summary.wav, Some(&expectation), Some(500), config).expect("trace");
        let first = rows.first().expect("rows");
        assert!(
            first.note.end_s < 1.0 && (first.floor_db - live.floor_db).abs() < 0.1,
            "the trace began at {:.2} s with a floor of {:.1} dBFS; the live one was {:.1}",
            first.note.end_s,
            first.floor_db,
            live.floor_db
        );
        let framed = |from: f64, to: f64| {
            rows.iter()
                .filter(|r| r.note.end_s > from && r.note.end_s < to)
                .map(|r| r.note.in_frame)
                .collect::<Vec<_>>()
        };
        assert!(
            framed(2.05, 2.65).iter().all(|&f| f),
            "the announced frame was not explained"
        );
        assert!(
            framed(1.2, 1.6).iter().all(|&f| !f),
            "blocks before it were"
        );

        // without the state the trace knows nothing for its first second
        let bare = Expectation {
            busy_state: None,
            ..expectation
        };
        let rows = trace(&summary.wav, Some(&bare), Some(500), config).expect("trace");
        assert!(rows.first().is_none_or(|r| r.note.end_s > 1.0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tone_outside_the_channel_is_measured_outside_it() {
        let params = aether_phy::waveform::NARROW_500;
        let rate = params.audio_rate as f64;
        let mut bands = WideBands::new(params, rate, 0.025);
        let tone: Vec<f32> = (0..4800)
            .map(|i| {
                (0.5 * (2.0 * std::f64::consts::PI * 700.0 * f64::from(i) / rate).sin()) as f32
            })
            .collect();
        bands.push(&tone);
        let (channel, outside, _) = bands.at(0.1);
        // a sine of amplitude 0.5 has power 0.125, −9 dBFS
        assert!((outside + 9.0).abs() < 0.5, "outside {outside:.1}");
        assert!(channel < outside - 40.0, "channel {channel:.1}");
    }
}
