//! Sound cards: capture in, playback out, at the waveform's audio rate.
//!
//! The modem is written against [`AudioIo`], not against a sound card, so the run loop is the
//! same whether the samples come from a radio interface, a recording or another process. That
//! is what lets two stations be tested against each other without hardware.
//!
//! # Why there are queues in the middle
//!
//! A sound card's callbacks run on a thread the operating system owns, and that thread must
//! never be made to wait: it has a few milliseconds to produce its next buffer and no more.
//! So the callbacks only move samples in and out of a queue, and the modem — which does the
//! transforms, the decoding and the protocol — runs on its own thread and takes whatever has
//! arrived. The queues are bounded: a modem that has fallen behind must drop old audio rather
//! than grow until the machine runs out of memory, and it says so when it does, because
//! silently losing audio looks exactly like a bad band.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Anything the run loop can take audio from and give audio to.
pub trait AudioIo {
    /// Take whatever has been captured since the last call.
    fn capture(&mut self) -> Vec<f32>;
    /// Queue samples to play.
    fn playback(&mut self, samples: &[f32]);
    /// How many samples are waiting to be played.
    fn queued(&self) -> usize;
    /// Samples dropped because a queue was full, since the device was opened.
    fn dropped(&self) -> usize;
}

/// Anything that went wrong with a sound card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioError {
    /// No device of that name, or no default.
    NoDevice(String),
    /// The device will not run at the rate and format the modem needs.
    Unsupported(String),
    /// The stream could not be built or started.
    Stream(String),
}

impl core::fmt::Display for AudioError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoDevice(name) if name.starts_with("default ") => write!(
                f,
                "this machine has no {name} device. Plug the radio interface in, or name a \
                 device in [audio]; `aetherd --list-devices` shows what is there"
            ),
            Self::NoDevice(name) => write!(
                f,
                "there is no audio device named {name:?}. The name has to match exactly what \
                 `aetherd --list-devices` prints; if the interface was unplugged, plug it back \
                 in and start again"
            ),
            Self::Unsupported(detail) => write!(
                f,
                "the audio device cannot do what the modem needs ({detail}). The modem runs at \
                 48 kHz; on Windows, check the device's default format in Sound settings"
            ),
            Self::Stream(detail) => write!(
                f,
                "the audio stream failed: {detail}. Another program may have exclusive use of \
                 the device"
            ),
        }
    }
}

impl core::error::Error for AudioError {}

/// A bounded queue shared with a sound-card callback.
#[derive(Debug, Default)]
struct Shared {
    samples: VecDeque<f32>,
    dropped: usize,
}

type Queue = Arc<Mutex<Shared>>;

fn push_bounded(queue: &Queue, samples: &[f32], limit: usize) {
    let Ok(mut shared) = queue.lock() else { return };
    shared.samples.extend(samples);
    if shared.samples.len() > limit {
        let excess = shared.samples.len() - limit;
        shared.samples.drain(..excess);
        shared.dropped += excess;
    }
}

/// What a station asks of a sound card.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioConfig {
    /// Capture device name, or `None` for the system default.
    pub input: Option<String>,
    /// Playback device name, or `None` for the system default.
    pub output: Option<String>,
    /// Sample rate. Must match the waveform's audio rate.
    pub sample_rate: u32,
    /// Longest backlog either queue may hold, in seconds. Past this, old audio is dropped:
    /// a modem that has fallen behind cannot catch up by buffering, and unbounded queues turn
    /// a slow machine into a crash.
    pub max_backlog_s: f64,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            input: None,
            output: None,
            sample_rate: 48_000,
            max_backlog_s: 2.0,
        }
    }
}

/// Name and channel count of a device the system offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// What to put in the configuration file.
    pub name: String,
    /// Whether it can capture.
    pub input: bool,
    /// Whether it can play.
    pub output: bool,
}

/// Every audio device the system offers, for an operator choosing one.
///
/// # Errors
/// If the audio host cannot be queried at all.
pub fn list_devices() -> Result<Vec<DeviceInfo>, AudioError> {
    let host = cpal::default_host();
    let devices = host
        .devices()
        .map_err(|e| AudioError::Stream(format!("cannot enumerate devices: {e}")))?;
    Ok(devices
        .map(|device| {
            let name = device.name().unwrap_or_else(|_| "(unnamed)".to_owned());
            DeviceInfo {
                name,
                input: device.default_input_config().is_ok(),
                output: device.default_output_config().is_ok(),
            }
        })
        .collect())
}

/// A pair of live sound-card streams.
///
/// The streams stop when this is dropped.
pub struct SoundCard {
    captured: Queue,
    to_play: Queue,
    limit: usize,
    /// The capture stream. Held so it keeps running; cpal stops a stream when it is dropped.
    _input: cpal::Stream,
    /// The playback stream.
    _output: cpal::Stream,
    /// What the streams are, for a log line.
    pub description: String,
}

impl std::fmt::Debug for SoundCard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SoundCard")
            .field("description", &self.description)
            .field("queued", &self.queued())
            .field("dropped", &self.dropped())
            .finish_non_exhaustive()
    }
}

impl SoundCard {
    /// Open the capture and playback streams.
    ///
    /// Multi-channel devices are handled by taking the first channel on capture and writing
    /// the same samples to every channel on playback — a radio interface is mono, and a
    /// device that only offers stereo should not be a reason the modem will not start.
    ///
    /// # Errors
    /// If a named device does not exist, will not run at the configured rate, or the streams
    /// cannot be built or started.
    pub fn open(config: &AudioConfig) -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let input_device = pick(&host, config.input.as_deref(), true)?;
        let output_device = pick(&host, config.output.as_deref(), false)?;
        let limit = (config.max_backlog_s * f64::from(config.sample_rate)) as usize;

        let in_name = input_device.name().unwrap_or_else(|_| "?".to_owned());
        let out_name = output_device.name().unwrap_or_else(|_| "?".to_owned());

        let in_config = stream_config(&input_device, config.sample_rate, true)?;
        let out_config = stream_config(&output_device, config.sample_rate, false)?;
        let in_channels = in_config.channels as usize;
        let out_channels = out_config.channels as usize;

        let captured: Queue = Arc::new(Mutex::new(Shared::default()));
        let to_play: Queue = Arc::new(Mutex::new(Shared::default()));

        let capture_queue = Arc::clone(&captured);
        let input = input_device
            .build_input_stream(
                &in_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    // one channel is all a radio gives; taking the first is honest and cheap
                    let mono: Vec<f32> = data.iter().step_by(in_channels).copied().collect();
                    push_bounded(&capture_queue, &mono, limit);
                },
                |error| eprintln!("aetherd: capture stream: {error}"),
                None,
            )
            .map_err(|e| AudioError::Stream(format!("input: {e}")))?;

        let play_queue = Arc::clone(&to_play);
        let output = output_device
            .build_output_stream(
                &out_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    // a poisoned queue means the modem thread panicked; play silence rather
                    // than join it, so the radio is not left with a stuck carrier
                    let Ok(mut shared) = play_queue.lock() else {
                        data.fill(0.0);
                        return;
                    };
                    for frame in data.chunks_mut(out_channels) {
                        let sample = shared.samples.pop_front().unwrap_or(0.0);
                        frame.fill(sample);
                    }
                },
                |error| eprintln!("aetherd: playback stream: {error}"),
                None,
            )
            .map_err(|e| AudioError::Stream(format!("output: {e}")))?;

        input
            .play()
            .map_err(|e| AudioError::Stream(format!("start input: {e}")))?;
        output
            .play()
            .map_err(|e| AudioError::Stream(format!("start output: {e}")))?;

        Ok(Self {
            captured,
            to_play,
            limit,
            _input: input,
            _output: output,
            description: format!(
                "in {in_name} ({in_channels} ch), out {out_name} ({out_channels} ch), \
                 {} Hz",
                config.sample_rate
            ),
        })
    }
}

impl AudioIo for SoundCard {
    fn capture(&mut self) -> Vec<f32> {
        self.captured
            .lock()
            .map(|mut shared| shared.samples.drain(..).collect())
            .unwrap_or_default()
    }

    fn playback(&mut self, samples: &[f32]) {
        push_bounded(&self.to_play, samples, self.limit);
    }

    fn queued(&self) -> usize {
        self.to_play.lock().map_or(0, |shared| shared.samples.len())
    }

    fn dropped(&self) -> usize {
        let captured = self.captured.lock().map_or(0, |shared| shared.dropped);
        let played = self.to_play.lock().map_or(0, |shared| shared.dropped);
        captured + played
    }
}

fn pick(host: &cpal::Host, name: Option<&str>, input: bool) -> Result<cpal::Device, AudioError> {
    match name {
        None => {
            let device = if input {
                host.default_input_device()
            } else {
                host.default_output_device()
            };
            device.ok_or_else(|| {
                AudioError::NoDevice(
                    if input {
                        "default input"
                    } else {
                        "default output"
                    }
                    .into(),
                )
            })
        }
        Some(wanted) => {
            let devices = host
                .devices()
                .map_err(|e| AudioError::Stream(format!("cannot enumerate devices: {e}")))?;
            devices
                .filter(|device| device.name().is_ok_and(|n| n == wanted))
                .find(|device| {
                    if input {
                        device.default_input_config().is_ok()
                    } else {
                        device.default_output_config().is_ok()
                    }
                })
                .ok_or_else(|| AudioError::NoDevice(wanted.to_owned()))
        }
    }
}

/// A stream configuration at the rate the modem needs.
///
/// The modem resamples nothing: its numerology is built around 48 kHz, and a device running
/// at another rate would put every timing estimate out by the ratio. A device that cannot do
/// it is a configuration error the operator has to see, not something to paper over.
fn stream_config(
    device: &cpal::Device,
    rate: u32,
    input: bool,
) -> Result<cpal::StreamConfig, AudioError> {
    let supported = if input {
        device.supported_input_configs().map(Iterator::collect)
    } else {
        device.supported_output_configs().map(Iterator::collect)
    };
    let supported: Vec<cpal::SupportedStreamConfigRange> =
        supported.map_err(|e| AudioError::Unsupported(format!("{e}")))?;

    let wanted = cpal::SampleRate(rate);
    let chosen = supported
        .iter()
        .filter(|range| range.sample_format() == cpal::SampleFormat::F32)
        .find(|range| range.min_sample_rate() <= wanted && wanted <= range.max_sample_rate())
        .ok_or_else(|| {
            AudioError::Unsupported(format!(
                "{} at {rate} Hz in 32-bit float",
                if input { "capture" } else { "playback" }
            ))
        })?;
    Ok((*chosen).with_sample_rate(wanted).config())
}

/// An [`AudioIo`] with no hardware: what is played comes straight back as what is captured.
///
/// For tests, for a dry run, and for an operator checking their configuration without keying
/// a radio.
#[derive(Debug, Default)]
pub struct Loopback {
    queue: VecDeque<f32>,
    /// Attenuation applied on the way round, so a test can set a signal-to-noise ratio.
    pub gain: f32,
}

impl Loopback {
    /// A loopback at unity gain.
    #[must_use]
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            gain: 1.0,
        }
    }
}

impl AudioIo for Loopback {
    fn capture(&mut self) -> Vec<f32> {
        self.queue.drain(..).map(|x| x * self.gain).collect()
    }

    fn playback(&mut self, samples: &[f32]) {
        self.queue.extend(samples);
    }

    fn queued(&self) -> usize {
        self.queue.len()
    }

    fn dropped(&self) -> usize {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_returns_what_was_played() {
        let mut audio = Loopback::new();
        assert!(audio.capture().is_empty());
        audio.playback(&[0.1, 0.2, 0.3]);
        assert_eq!(audio.queued(), 3);
        assert_eq!(audio.capture(), vec![0.1, 0.2, 0.3]);
        assert!(
            audio.capture().is_empty(),
            "it handed the same audio back twice"
        );
    }

    #[test]
    fn a_bounded_queue_drops_the_oldest_audio_and_counts_it() {
        // a modem that has fallen behind cannot catch up by buffering; what it must not do is
        // grow until the machine gives out, and it must be able to say that it lost audio
        let queue: Queue = Arc::new(Mutex::new(Shared::default()));
        let samples: Vec<f32> = (0..100).map(|i| i as f32).collect();
        push_bounded(&queue, &samples, 40);
        let shared = queue.lock().expect("lock");
        assert_eq!(shared.samples.len(), 40);
        assert_eq!(shared.dropped, 60);
        assert!(
            (shared.samples[0] - 60.0).abs() < 1e-9,
            "it dropped the newest audio rather than the oldest"
        );
    }

    #[test]
    fn enumerating_devices_does_not_fail_on_a_machine_without_any() {
        // CI has no sound card; the modem must still start and say so rather than panic
        match list_devices() {
            Ok(devices) => {
                for device in devices {
                    assert!(!device.name.is_empty());
                }
            }
            Err(error) => {
                // an error is a legitimate answer here, as long as it is an error
                assert!(!error.to_string().is_empty());
            }
        }
    }
}
