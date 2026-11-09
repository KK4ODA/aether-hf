//! The receive filter's reading over a recording (`aetherd --passband-trace`).
//!
//! The panel shows the reading's verdict and the drop at each edge. That says nothing of how it
//! got there: which looks it took, which it dropped under a frame, how far each stood over the
//! chain's floor. This runs a recording's audio through the same [`SpectrumAnalyser`] and
//! [`ReceiveFilter`] a live station runs, with the looks taken by the station's own rule
//! (`Station::sample_passband`) as near as a sidecar allows: deaf where the transmitter was
//! keyed and for its tail, counted as receiving from a heeded announcement for the frame's
//! length (at least the air's longest) and for half a second after a frame that decoded or was
//! trusted, and every frame announced or reported handed to the reading as the station handed
//! it. It reports every look the reading folds in, and between them the reading on a half-second
//! beat, each with C1's share of the audio clipping (ADR-0105).

use std::fmt::Write as _;
use std::path::Path;

use aether_phy::{WIDE_2300, WaveformParams};

use crate::passband::{Clipping, FilterReading, LOOK_EVERY_S, ReceiveFilter, SPECTRUM_TOP_HZ};
use crate::record::read_wav;
use crate::replay::Expectation;
use crate::spectrum::SpectrumAnalyser;
use crate::station::DETECT_CONFIDENCE_TRUSTED;

/// The audio a live station hands over at a time: 20 ms, as the daemon's loop does.
const FEED_S: f64 = 0.02;
/// How long after a decoded or trusted frame is reported the station still counts itself as
/// receiving, as `Station::report` sets it.
const RECEIVING_AFTER_FRAME_S: f64 = 0.5;

/// The reading at one moment of the recording.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    /// When, seconds of the recording.
    pub t_s: f64,
    /// When the look folded in here was taken (the end of its 85 ms of audio); `None` on the
    /// beat's lines between looks.
    pub taken_s: Option<f64>,
    /// The reading after it.
    pub reading: FilterReading,
    /// The share of the audio heard over the last ten seconds at full scale (C1).
    pub clipping: f64,
}

/// The waveform of a bandwidth.
fn params_for(bandwidth_hz: u32) -> Result<WaveformParams, String> {
    match bandwidth_hz {
        2300 => Ok(WIDE_2300),
        500 => Ok(aether_phy::waveform::NARROW_500),
        other => Err(format!("no waveform for {other} Hz")),
    }
}

/// Run a recording through the receive filter's reading.
///
/// `expectation` is the recording's sidecar when there is one: the keyed spans are deaf, the
/// frames and announcements hold the station receiving and are handed to the reading, and a
/// move to the other bandwidth moves the span judged. Without one the whole recording is
/// listened to.
///
/// # Errors
/// If the file cannot be read or is not at the modem's rate.
pub fn trace(
    wav: &Path,
    expectation: Option<&Expectation>,
    bandwidth_hz: Option<u32>,
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
    let mut analyser = SpectrumAnalyser::new(rate);
    let mut filter = ReceiveFilter::for_air(params);
    let mut clipping = Clipping::new(rate);
    let mut heard = Heard::new(expectation, params);
    let muted = expectation.map(|e| e.muted.clone()).unwrap_or_default();
    let mut airs = expectation
        .map_or(&[][..], |e| &e.airs[..])
        .iter()
        .peekable();

    let feed = (FEED_S * rate) as usize;
    let mut readings = Vec::new();
    let mut next_beat = 0.0;
    let mut seen = 0usize;
    for chunk in audio.samples.chunks(feed) {
        let t = seen as f64 / rate;
        seen += chunk.len();
        let now = seen as f64 / rate;
        analyser.push(chunk);
        while let Some(air) = airs.next_if(|a| a.at_s <= now) {
            let params = params_for(air.bandwidth_hz)?;
            filter.set_air(params);
            heard.longest_s = aether_phy::modes::air_interface(params).long.duration_s();
        }
        let keyed = muted.iter().any(|m| t >= m.from_s && t < m.until_s);
        let deaf = crate::busy_trace::deaf_at(&muted, t);
        heard.until(now, &mut filter);
        if !keyed && !deaf {
            clipping.hear(chunk);
        }
        // Station::sample_passband
        let listening = !keyed && !deaf && now >= heard.receiving_until;
        let folded = filter.step(now, listening, analyser.window_s(), || {
            analyser.compute_to(SPECTRUM_TOP_HZ)
        });
        let reading = filter.reading();
        let share = clipping.share();
        if !folded.is_empty() {
            next_beat = now + LOOK_EVERY_S;
        } else if now >= next_beat {
            readings.push(Reading {
                t_s: now,
                taken_s: None,
                reading,
                clipping: share,
            });
            next_beat = now + LOOK_EVERY_S;
        }
        readings.extend(folded.into_iter().map(|taken| Reading {
            t_s: now,
            taken_s: Some(taken),
            reading,
            clipping: share,
        }));
    }
    Ok(readings)
}

/// What the station heard, from the sidecar: handed to the reading as the station handed it,
/// and kept as how long the station counted itself receiving.
struct Heard<'a> {
    frames: Vec<crate::record::FrameRecord>,
    next_frame: usize,
    announcements: &'a [crate::replay::Announcement],
    next_announcement: usize,
    /// The air's longest frame: a heeded announcement holds the station at least this long.
    longest_s: f64,
    receiving_until: f64,
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
        }
    }

    /// Take in what was announced or reported by `now`.
    fn until(&mut self, now: f64, filter: &mut ReceiveFilter) {
        while let Some(announced) = self
            .announcements
            .get(self.next_announcement)
            .filter(|a| a.t_s <= now)
        {
            filter.frame(announced.start_s, announced.end_s);
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
                filter.frame(start, end);
            }
            if frame.decoded || frame.detect_confidence >= DETECT_CONFIDENCE_TRUSTED {
                self.receiving_until = self
                    .receiving_until
                    .max(frame.t_s + RECEIVING_AFTER_FRAME_S);
            }
            self.next_frame += 1;
        }
    }
}

/// The trace as CSV, one line a look folded in and one a half second between them; the
/// columns are a recording's `passband.readings`.
#[must_use]
pub fn to_csv(readings: &[Reading]) -> String {
    let mut out = String::from(
        "t_s,taken_s,verdict,low_drop_db,high_drop_db,over_floor_db,looks,clipping_pct\n",
    );
    let db = |v: Option<f32>| v.map_or_else(String::new, |v| format!("{v:.2}"));
    for r in readings {
        let taken = r.taken_s.map_or_else(String::new, |t| format!("{t:.3}"));
        let f = &r.reading;
        // writing to a String cannot fail
        let _ = writeln!(
            out,
            "{:.3},{taken},{},{},{},{},{},{:.3}",
            r.t_s,
            f.verdict.word(),
            db(f.low_drop_db),
            db(f.high_drop_db),
            db(f.over_floor_db),
            f.looks,
            r.clipping * 100.0,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::passband::{Verdict, test_noise::ReceiverNoise};

    const RATE: f64 = 48_000.0;

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
        let readings = trace(&summary.wav, Some(&expectation), None).expect("trace");
        let _ = std::fs::remove_dir_all(&dir);
        readings
    }

    /// How much audio one look covers at 48 kHz.
    const WINDOW_S: f64 = crate::spectrum::WINDOW as f64 / RATE;

    #[test]
    fn a_trace_reports_the_reading_look_by_look() {
        // the instrument, not the reading: it folds in a look every half second once each has
        // been held, the reading comes once twelve are kept, and a receiver whose noise stops
        // at 1900 Hz reads the 2300 Hz signal's top cut
        let mut noise = ReceiverNoise::new(300.0, 1900.0, RATE, 0.3, 0.001);
        let readings = trace_of(&noise.take(20 * 48_000), "top", None);
        let looks: Vec<&Reading> = readings.iter().filter(|r| r.taken_s.is_some()).collect();
        assert!(looks.len() >= 20, "only {} looks in 20 s", looks.len());
        assert!(
            looks
                .iter()
                .all(|r| r.taken_s.is_some_and(|t| t + 5.0 < r.t_s)),
            "a look folded in before it was held"
        );
        assert!(
            readings
                .iter()
                .all(|r| (r.reading.looks >= 12) == (r.reading.verdict != Verdict::Waiting)),
            "a reading before twelve looks, or none after"
        );
        let last = readings.last().expect("readings").reading;
        assert_eq!(last.verdict, Verdict::Top, "{last:?}");
        // the beat's lines fill the time before the first look is folded in
        assert!(
            readings
                .first()
                .is_some_and(|r| r.taken_s.is_none() && r.t_s < 0.1)
        );
    }

    #[test]
    fn no_look_is_taken_while_the_station_is_keyed_or_deaf_after() {
        let mut noise = ReceiverNoise::new(300.0, 2900.0, RATE, 0.3, 0.001);
        let readings = trace_of(&noise.take(30 * 48_000), "keyed", Some((8.0, 12.0)));
        let deaf_after = crate::audio::DEVICE_LATENCY_S + crate::station::CAPTURE_LAG_ALLOWANCE_S;
        let taken: Vec<f64> = readings.iter().filter_map(|r| r.taken_s).collect();
        assert!(taken.iter().any(|&t| t < 8.0));
        assert!(taken.iter().any(|&t| t > 13.0));
        for t in taken {
            assert!(
                t - WINDOW_S >= 12.0 + deaf_after || t <= 8.0,
                "a look of {:.2}-{t:.2} s overlaps the keying or the deafness after it",
                t - WINDOW_S
            );
        }
    }

    #[test]
    fn a_csv_line_carries_the_reading_and_the_clipping() {
        let waiting = FilterReading {
            verdict: Verdict::Waiting,
            low_drop_db: None,
            high_drop_db: None,
            over_floor_db: None,
            looks: 0,
        };
        let csv = to_csv(&[
            Reading {
                t_s: 0.02,
                taken_s: None,
                reading: waiting,
                clipping: 0.0,
            },
            Reading {
                t_s: 9.0,
                taken_s: Some(3.1),
                reading: FilterReading {
                    verdict: Verdict::Top,
                    low_drop_db: Some(0.4),
                    high_drop_db: Some(23.0),
                    over_floor_db: Some(48.0),
                    looks: 12,
                },
                clipping: 0.015,
            },
        ]);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1], "0.020,,waiting,,,,0,0.000");
        assert_eq!(lines[2], "9.000,3.100,top,0.40,23.00,48.00,12,1.500");
    }
}
