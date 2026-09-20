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
//!
//! # Why the playback queue holds a whole burst
//!
//! The playback callback plays silence when the queue is empty — it has nothing else to
//! play — and a transmission with a hole in it is what that silence is when the transmitter
//! is keyed. The modem thread used to keep a quarter second queued and top it up between
//! blocks, so any single block that cost it more than that — a frame of somebody else's
//! landing in the decoder, a slow disk, the scheduler — put a hole in the burst on the air,
//! and nothing counted it (`field/TX-ONSET-FINDINGS.md`). So the modem now hands the whole
//! rendered burst over as soon as it is rendered, the queue is bounded by the longest
//! transmission rather than by a fraction of a second, and the callback counts every sample
//! of silence it had to play while a burst was supposed to be running. The key is released
//! from the callback's own clock ([`AudioIo::played`]), when the last sample has really
//! left, not when the modem has nothing more to hand over.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// How long a sample handed to the sound card is assumed to take to leave it: the card's
/// own buffering, which its playback clock cannot see. The keying tail covers it, the
/// engine's timers are told of it, and the simulated channel delivers what a station plays
/// this much later, as a card would. A quarter second is generous for a USB codec.
///
/// This used to be how much audio the loop kept queued ahead of the card, topping it up
/// between blocks — and a block that cost the receiver more than that put a hole in the
/// burst on the air, uncounted (`field/TX-ONSET-FINDINGS.md`). A whole burst is queued at
/// once now; the constant only names the card's latency.
pub const DEVICE_LATENCY_S: f64 = 0.25;

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
    /// The playback clock: samples the device has consumed since it was opened, whether
    /// they came from the queue or were silence played for want of any. A sample queued
    /// when this reads `n` leaves the device when it reads `n + queued()`.
    fn played(&self) -> u64;
    /// Whether a burst is in flight, so that silence played for an empty queue counts as
    /// starvation rather than as the idle state of a station with nothing to say.
    fn set_playing(&mut self, playing: bool);
    /// Samples of silence the device played while a burst was in flight, since it was
    /// opened: every one of them is a hole in a transmission.
    fn starved(&self) -> usize;
    /// Drop whatever is queued for playback, for a transmission cut short.
    fn clear(&mut self);
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
                "{detail}. On Windows: Settings > System > Sound > the device > Advanced, set \
                 the format to 48000 Hz; on Linux the card's rate is whatever ALSA or \
                 PipeWire is configured for"
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
    /// Frames the playback device has consumed, from the queue or as silence.
    played: u64,
    /// Whether the modem says a burst is in flight.
    playing: bool,
    /// Frames of silence played for an empty queue while a burst was in flight.
    starved: usize,
}

type Queue = Arc<Mutex<Shared>>;

/// Fill one playback callback's buffer from the queue: the same sample on every channel,
/// silence when the queue is empty, and the accounting the modem reads back.
///
/// Kept apart from the callback so it can be tested without a sound card.
fn fill_playback(shared: &mut Shared, data: &mut [f32], channels: usize) {
    for frame in data.chunks_mut(channels.max(1)) {
        let sample = shared.samples.pop_front().unwrap_or_else(|| {
            if shared.playing {
                shared.starved += 1;
            }
            0.0
        });
        frame.fill(sample);
        shared.played += 1;
    }
}

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
    /// Longest backlog the capture queue may hold, in seconds. Past this, old audio is
    /// dropped: a modem that has fallen behind cannot catch up by buffering, and unbounded
    /// queues turn a slow machine into a crash.
    pub max_backlog_s: f64,
    /// Longest the playback queue may hold, in seconds: a whole transmission, since the
    /// modem hands a burst over in one piece (see the module notes). Past this the oldest
    /// audio is dropped, which is the *start* of a burst — so this must exceed the longest
    /// transmission the station is allowed to make.
    pub max_playback_s: f64,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            input: None,
            output: None,
            sample_rate: 48_000,
            max_backlog_s: 2.0,
            max_playback_s: 40.0,
        }
    }
}

/// A device the system offers, and the sample rates it will run at.
///
/// The rates are the point. On Windows a USB radio codec runs at whatever its "default
/// format" is set to in Sound settings, and 44.1 kHz is a common factory setting; the modem
/// needs 48 kHz and does not resample, so a panel that knows the rates can say "set this
/// device to 48 kHz" before the daemon fails to start, instead of after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// What to put in the configuration file.
    pub name: String,
    /// Whether it can capture.
    pub input: bool,
    /// Whether it can play.
    pub output: bool,
    /// Sample rates it captures at, in Hz. Empty when it cannot capture, or will not say.
    pub input_rates: Vec<u32>,
    /// Sample rates it plays at, in Hz.
    pub output_rates: Vec<u32>,
}

/// The distinct sample rates a set of stream configurations covers.
///
/// A range is reported as its two ends: a shared-mode Windows device has one rate and
/// reports it twice; ALSA reports a real span, and naming its ends is enough to tell an
/// operator whether 48 kHz is inside it.
fn rates_of<I>(ranges: I) -> Vec<u32>
where
    I: IntoIterator<Item = cpal::SupportedStreamConfigRange>,
{
    let mut rates: Vec<u32> = ranges
        .into_iter()
        .flat_map(|range| [range.min_sample_rate().0, range.max_sample_rate().0])
        .collect();
    rates.sort_unstable();
    rates.dedup();
    rates
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
                input_rates: device
                    .supported_input_configs()
                    .map(rates_of)
                    .unwrap_or_default(),
                output_rates: device
                    .supported_output_configs()
                    .map(rates_of)
                    .unwrap_or_default(),
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
    playback_limit: usize,
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
        let playback_limit = (config.max_playback_s * f64::from(config.sample_rate)) as usize;

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
                    fill_playback(&mut shared, data, out_channels);
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
            playback_limit,
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
        push_bounded(&self.to_play, samples, self.playback_limit);
    }

    fn queued(&self) -> usize {
        self.to_play.lock().map_or(0, |shared| shared.samples.len())
    }

    fn dropped(&self) -> usize {
        let captured = self.captured.lock().map_or(0, |shared| shared.dropped);
        let played = self.to_play.lock().map_or(0, |shared| shared.dropped);
        captured + played
    }

    fn played(&self) -> u64 {
        self.to_play.lock().map_or(0, |shared| shared.played)
    }

    fn set_playing(&mut self, playing: bool) {
        if let Ok(mut shared) = self.to_play.lock() {
            shared.playing = playing;
        }
    }

    fn starved(&self) -> usize {
        self.to_play.lock().map_or(0, |shared| shared.starved)
    }

    fn clear(&mut self) {
        if let Ok(mut shared) = self.to_play.lock() {
            shared.samples.clear();
        }
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
            // Say what the device *does* offer: "44100" is the whole diagnosis, and the
            // fix is one setting away
            let offered = rates_of(supported.iter().copied());
            let direction = if input { "capture" } else { "playback" };
            let name = device.name().unwrap_or_else(|_| "the device".to_owned());
            AudioError::Unsupported(if offered.is_empty() {
                format!("{direction} on {name:?} offers no format the modem can use")
            } else {
                format!(
                    "{direction} on {name:?} runs at {} Hz, and the modem needs {rate} Hz",
                    offered
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(" or ")
                )
            })
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
    /// Samples handed back so far: the loopback's playback clock.
    played: u64,
}

impl Loopback {
    /// A loopback at unity gain.
    #[must_use]
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            gain: 1.0,
            played: 0,
        }
    }
}

impl AudioIo for Loopback {
    fn capture(&mut self) -> Vec<f32> {
        self.played += self.queue.len() as u64;
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

    fn played(&self) -> u64 {
        self.played
    }

    fn set_playing(&mut self, _playing: bool) {}

    fn starved(&self) -> usize {
        0
    }

    fn clear(&mut self) {
        self.queue.clear();
    }
}

/// A sound card that could not be opened, kept so the daemon can start anyway.
///
/// It delivers silence and discards what is played, but it does so *on the clock*: the
/// station's sense of time comes from the samples it is handed, so a backend that
/// returned nothing would freeze the modem rather than run it quietly. Silence paced
/// like a real card leaves the panel live, the log ticking and Setup reachable, which
/// is the whole point — the screen that names the sound card is served by the daemon a
/// missing sound card would otherwise stop.
#[derive(Debug)]
pub struct Silence {
    rate: f64,
    started: std::time::Instant,
    delivered: u64,
}

impl Silence {
    /// A silent card at the modem's sample rate.
    #[must_use]
    pub fn new(rate: u32) -> Self {
        Self {
            rate: f64::from(rate),
            started: std::time::Instant::now(),
            delivered: 0,
        }
    }
}

impl AudioIo for Silence {
    fn capture(&mut self) -> Vec<f32> {
        // however long has really passed, that many samples — no more, so the modem
        // does not race ahead of the clock, and no fewer, so it does not stall
        let due = (self.started.elapsed().as_secs_f64() * self.rate) as u64;
        let owed = usize::try_from(due.saturating_sub(self.delivered)).unwrap_or(0);
        self.delivered = due;
        vec![0.0; owed]
    }

    fn playback(&mut self, _samples: &[f32]) {}

    fn queued(&self) -> usize {
        0
    }

    fn dropped(&self) -> usize {
        0
    }

    fn played(&self) -> u64 {
        // what would have played by now: the same clock the capture side runs on
        (self.started.elapsed().as_secs_f64() * self.rate) as u64
    }

    fn set_playing(&mut self, _playing: bool) {}

    fn starved(&self) -> usize {
        0
    }

    fn clear(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_playback_callback_counts_its_clock_and_the_silence_it_had_to_play() {
        // the queue holds four samples and the device asks for six frames of stereo: it
        // plays the four, then two frames of silence — which only count as starvation
        // while the modem says a burst is in flight
        let mut shared = Shared::default();
        shared.samples.extend([0.1, 0.2, 0.3, 0.4]);
        let mut data = [9.0f32; 12];
        fill_playback(&mut shared, &mut data, 2);
        assert_eq!(
            &data[..4],
            &[0.1, 0.1, 0.2, 0.2],
            "the same sample on both channels"
        );
        assert_eq!(
            &data[8..],
            &[0.0; 4],
            "silence for want of anything to play"
        );
        assert_eq!(
            shared.played, 6,
            "the clock counts frames, played or silent"
        );
        assert_eq!(shared.starved, 0, "an idle station is not starving");

        shared.playing = true;
        let mut data = [9.0f32; 6];
        fill_playback(&mut shared, &mut data, 2);
        assert_eq!(shared.played, 9);
        assert_eq!(shared.starved, 3, "every silent frame in a burst is a hole");
    }

    #[test]
    fn the_loopback_clock_follows_what_it_handed_back_and_a_clear_empties_it() {
        let mut audio = Loopback::new();
        audio.playback(&[0.5; 100]);
        assert_eq!(audio.played(), 0, "nothing has left until it is captured");
        assert_eq!(audio.capture().len(), 100);
        assert_eq!(audio.played(), 100);
        audio.playback(&[0.5; 50]);
        audio.clear();
        assert_eq!(audio.queued(), 0);
        assert!(audio.capture().is_empty());
        assert_eq!(audio.played(), 100, "cleared audio never played");
    }

    #[test]
    fn silence_keeps_the_clock_rather_than_stopping_it() {
        let mut audio = Silence::new(48_000);
        // nothing is owed the instant it opens, and what it hands over follows the clock
        assert!(audio.capture().len() < 4_800);
        std::thread::sleep(std::time::Duration::from_millis(50));
        let block = audio.capture();
        assert!(
            block.len() >= 1_200 && block.len() <= 12_000,
            "50 ms of 48 kHz silence, got {}",
            block.len()
        );
        assert!(block.iter().all(|&x| x == 0.0), "it is silence");
        audio.playback(&[0.5; 128]);
        assert_eq!(audio.queued(), 0, "what is played goes nowhere");
    }

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
