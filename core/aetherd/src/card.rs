//! The sound card the station runs on, kept open across a device that goes and comes back
//! (ADR-0137).
//!
//! A USB radio interface (a `DigiRig`, a `SignaLink`, a radio's own codec) is a device the
//! operator unplugs, knocks loose or plugs into another port. On Windows each of those is
//! a new audio endpoint: the same codec in another port is named again with an instance
//! number in front of it — `Speakers (USB Audio CODEC)` becomes `Speakers (2- USB Audio
//! CODEC)` — and comes up with Windows' own defaults for a new endpoint (often 44.1 kHz,
//! its own input level, enhancements on). Before this module the daemon opened its card
//! once: a card missing at start, or at the wrong rate, left the station on silence until a
//! Setup save restarted it, and a card unplugged mid-run left it deaf and mute with nothing
//! said (the stream's error went to standard error). The author's report of 2026-11-25: a
//! `DigiRig` in another USB-C port was a new device, and it took a reset of the device and a
//! restart of the laptop to bring the station back.
//!
//! Two rules here:
//!
//! - **The same device under another instance number is the same device**
//!   ([`resolve`]): a configured name that is not there is matched to the one device whose
//!   name differs only by Windows' `N- ` instance prefix — exactly one, the same model, and
//!   the two directions of one codec together, never a microphone of one with the speakers
//!   of another. Anything else is not guessed: the station waits, and says so.
//! - **A card that fails is tried again** ([`ResilientCard`]): a stream that reports its
//!   device gone, or that delivers nothing for [`STALL`], puts the station on silence, which
//!   it says, and the card is opened again every [`retry_interval`] until it opens — no
//!   restart. While it is down nothing is keyed (the station refuses to transmit, see
//!   `Station::set_audio_down`).

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::audio::{
    AudioConfig, AudioError, AudioIo, CaptureSkips, OnsetGuard, PlaybackSkips, Silence, SoundCard,
};

/// A card that has delivered no audio for this long has stopped, whatever its stream said:
/// Windows does not always report an endpoint that went away, and a capture stream that
/// stops calling back looks exactly like this. A healthy card delivers every 10–20 ms.
pub const STALL: Duration = Duration::from_secs(3);

/// How long after a failure the card is tried again.
///
/// Every second for the first minute — a cable pushed back in, or the same codec in another
/// port, comes back within about a second — then every ten, so a card that is gone for good
/// costs the run loop an enumeration only that often (on Windows a few tens of milliseconds a
/// try). Windows can say when a device arrives (`IMMNotificationClient`), but only through
/// `unsafe` COM calls, which the workspace forbids; a second's polling is the author's choice
/// (ADR-0137 §6).
#[must_use]
pub fn retry_interval(since_failure: Duration) -> Duration {
    if since_failure < Duration::from_secs(60) {
        Duration::from_secs(1)
    } else {
        Duration::from_secs(10)
    }
}

/// A device name with Windows' instance number taken out, and the number.
///
/// Windows numbers a second endpoint of the same name `2- `, a third `3- `, in front of the
/// device's own name — inside the parentheses of an endpoint's friendly name
/// (`Microphone (2- USB Audio CODEC)`) or, for a device named without them, at the start
/// (`2- USB Audio CODEC`). No number is instance 1.
#[must_use]
pub fn instance_of(name: &str) -> (String, u32) {
    // the prefix sits at the start, or just after the opening parenthesis of the device part
    let at = name.rfind('(').map_or(0, |open| open + 1);
    let rest = &name[at..];
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && digits <= 3 && rest[digits..].starts_with("- ") {
        if let Ok(number) = rest[..digits].parse::<u32>() {
            let mut plain = String::with_capacity(name.len());
            plain.push_str(&name[..at]);
            plain.push_str(&rest[digits + 2..]);
            return (plain, number);
        }
    }
    (name.to_owned(), 1)
}

/// The physical device an endpoint's name says it belongs to: the part in its last
/// parentheses, instance number taken out (`Speakers (2- USB Audio CODEC)` → `USB Audio
/// CODEC`), or the whole name when it has none. A codec's microphone and speakers share it.
#[must_use]
pub fn device_part(name: &str) -> String {
    let (plain, _) = instance_of(name);
    match (plain.rfind('('), plain.rfind(')')) {
        (Some(open), Some(close)) if close > open => plain[open + 1..close].trim().to_owned(),
        _ => plain.trim().to_owned(),
    }
}

/// A direction of the card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// Capture: the radio's receive audio.
    Input,
    /// Playback: the audio the radio transmits.
    Output,
}

/// A configured device found under another name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Moved {
    /// Which side of the card.
    pub direction: Direction,
    /// The name in the configuration.
    pub configured: String,
    /// The name it has on this machine now.
    pub found: String,
}

/// The devices to open, and which of them were found under another name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Resolved {
    /// The capture device's name, `None` for the system default.
    pub input: Option<String>,
    /// The playback device's name, `None` for the system default.
    pub output: Option<String>,
    /// The configured names found under another one.
    pub moved: Vec<Moved>,
}

/// Why the configured devices cannot be opened as they are named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unresolved {
    /// The configured names not on this machine.
    pub missing: Vec<(Direction, String)>,
    /// Why no other name was taken for them, in a phrase.
    pub why: String,
}

/// The one device of `available` that is `wanted` under another instance number, if there
/// is exactly one.
fn renamed(wanted: &str, available: &[String]) -> Result<Option<String>, usize> {
    let (plain, _) = instance_of(wanted);
    let found: Vec<&String> = available
        .iter()
        .filter(|name| name.as_str() != wanted && instance_of(name).0 == plain)
        .collect();
    match found.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some((*one).clone())),
        many => Err(many.len()),
    }
}

/// The devices to open for a configured capture and playback device, given the devices
/// this machine has for each.
///
/// A configured name that is there is used as it is. One that is not is matched by
/// [`instance_of`]: the one device whose name is the same but for the instance number. When
/// the configured capture and playback devices are the two sides of one codec
/// ([`device_part`]), the two names used must be of one instance too — a codec's capture
/// side is never paired with the playback side of another of the same model, which would
/// transmit through one radio while listening to another. Unnamed (`None`) is the system
/// default and is never resolved.
///
/// # Errors
/// When a configured device is not there and no single device of the same model and
/// instance takes its place.
pub fn resolve(
    input: Option<&str>,
    output: Option<&str>,
    inputs: &[String],
    outputs: &[String],
) -> Result<Resolved, Unresolved> {
    let mut resolved = Resolved {
        input: input.map(str::to_owned),
        output: output.map(str::to_owned),
        moved: Vec::new(),
    };
    let mut missing = Vec::new();
    let mut why = Vec::new();
    for (direction, wanted, available, slot) in [
        (Direction::Input, input, inputs, &mut resolved.input),
        (Direction::Output, output, outputs, &mut resolved.output),
    ] {
        let Some(wanted) = wanted else { continue };
        if available.iter().any(|name| name == wanted) {
            continue;
        }
        match renamed(wanted, available) {
            Ok(Some(found)) => {
                *slot = Some(found.clone());
                resolved.moved.push(Moved {
                    direction,
                    configured: wanted.to_owned(),
                    found,
                });
            }
            Ok(None) => {
                missing.push((direction, wanted.to_owned()));
                why.push("no device of the same name under another number".to_owned());
            }
            Err(count) => {
                missing.push((direction, wanted.to_owned()));
                why.push(format!(
                    "{count} devices of the same name under other numbers, and which is \
                     the radio's cannot be told"
                ));
            }
        }
    }
    if !missing.is_empty() {
        why.dedup();
        return Err(Unresolved {
            missing,
            why: why.join("; "),
        });
    }
    // one codec's two sides stay one codec's
    if let (Some(wanted_in), Some(wanted_out), Some(used_in), Some(used_out)) =
        (input, output, &resolved.input, &resolved.output)
        && !resolved.moved.is_empty()
        && device_part(wanted_in) == device_part(wanted_out)
        && instance_of(wanted_in).1 == instance_of(wanted_out).1
        && instance_of(used_in).1 != instance_of(used_out).1
    {
        return Err(Unresolved {
            missing: resolved
                .moved
                .iter()
                .map(|m| (m.direction, m.configured.clone()))
                .collect(),
            why: format!("{used_in} and {used_out} are not the two sides of one device"),
        });
    }
    Ok(resolved)
}

/// What the card is doing, for the status, the panel and a host program.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct CardState {
    /// Changes with every change of what follows: the run loop applies a state once.
    #[serde(skip)]
    pub generation: u64,
    /// Why there is no audio, while there is none.
    pub fault: Option<String>,
    /// What the streams are, for a log line, while open.
    pub description: String,
    /// The capture device in use, while open.
    pub input: Option<String>,
    /// The playback device in use, while open.
    pub output: Option<String>,
    /// Configured devices in use under another name.
    pub moved: Vec<Moved>,
    /// Tries since the card failed, while it is down.
    pub tries: u32,
}

/// How a card is opened: by name, for the machine's devices, or a stand-in for tests.
pub trait Opener {
    /// The machine's capture and playback device names.
    ///
    /// # Errors
    /// If the audio host cannot be asked.
    fn names(&mut self) -> Result<(Vec<String>, Vec<String>), AudioError>;
    /// Open the streams on these devices.
    ///
    /// # Errors
    /// As [`SoundCard::open`].
    fn open(&mut self, config: &AudioConfig) -> Result<Box<dyn LiveCard>, AudioError>;
}

/// An open card, as [`ResilientCard`] needs it.
pub trait LiveCard: AudioIo {
    /// Why the card has stopped, when its stream has said so.
    fn failure(&self) -> Option<String>;
    /// What the streams are.
    fn describe(&self) -> String;
}

impl LiveCard for SoundCard {
    fn failure(&self) -> Option<String> {
        SoundCard::failure(self)
    }
    fn describe(&self) -> String {
        self.description.clone()
    }
}

/// The machine's own audio.
#[derive(Debug, Default)]
pub struct SystemAudio;

impl Opener for SystemAudio {
    fn names(&mut self) -> Result<(Vec<String>, Vec<String>), AudioError> {
        crate::audio::device_names()
    }
    fn open(&mut self, config: &AudioConfig) -> Result<Box<dyn LiveCard>, AudioError> {
        Ok(Box::new(SoundCard::open(config)?))
    }
}

/// Counters of a card that has been closed, added to the next one's, so the run loop's
/// differences of them never go backwards across a reopen.
#[derive(Debug, Default, Clone, Copy)]
struct Carried {
    played: u64,
    dropped: usize,
    starved: usize,
    capture: CaptureSkips,
    playback: PlaybackSkips,
}

fn add_capture(a: CaptureSkips, b: CaptureSkips) -> CaptureSkips {
    CaptureSkips {
        lost: a.lost + b.lost,
        lost_s: a.lost_s + b.lost_s,
        largest_s: a.largest_s.max(b.largest_s),
        padded: a.padded + b.padded,
        padded_s: a.padded_s + b.padded_s,
        repeated: a.repeated + b.repeated,
        repeated_s: a.repeated_s + b.repeated_s,
        span_s: a.span_s + b.span_s,
        small_s: a.small_s + b.small_s,
    }
}

fn add_playback(a: PlaybackSkips, b: PlaybackSkips) -> PlaybackSkips {
    PlaybackSkips {
        skipped: a.skipped + b.skipped,
        skipped_s: a.skipped_s + b.skipped_s,
        largest_s: a.largest_s.max(b.largest_s),
        repeated: a.repeated + b.repeated,
        repeated_s: a.repeated_s + b.repeated_s,
    }
}

/// A sound card that stays the station's across a device that goes and comes back.
///
/// While the card is open it is the card. While it is not — never opened, or failed — it
/// is [`Silence`] on the clock, so the station runs on, deaf and mute, the panel live, and
/// it is opened again every [`retry_interval`]. The playback clock and the loss counters
/// run on across the change, so the station's account of them never steps back.
pub struct ResilientCard {
    config: AudioConfig,
    opener: Box<dyn Opener>,
    card: Option<Box<dyn LiveCard>>,
    silence: Silence,
    carried: Carried,
    /// Silence's clock where the silence began counting, so its time is added once.
    silence_from: u64,
    state: CardState,
    failed_at: Instant,
    next_try: Instant,
    /// When the open card last delivered audio.
    last_audio: Instant,
}

impl std::fmt::Debug for ResilientCard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResilientCard")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl ResilientCard {
    /// The machine's card for `config`, opened now if it can be.
    #[must_use]
    pub fn system(config: AudioConfig) -> Self {
        Self::with_opener(config, Box::new(SystemAudio))
    }

    /// A card opened by `opener`, tried at once.
    #[must_use]
    pub fn with_opener(config: AudioConfig, opener: Box<dyn Opener>) -> Self {
        let now = Instant::now();
        let mut card = Self {
            silence: Silence::new(config.sample_rate),
            config,
            opener,
            card: None,
            carried: Carried::default(),
            silence_from: 0,
            state: CardState::default(),
            failed_at: now,
            next_try: now,
            last_audio: now,
        };
        card.try_open(now);
        card
    }

    /// What the card is doing.
    #[must_use]
    pub fn state(&self) -> &CardState {
        &self.state
    }

    /// Open the configured devices, as they are named or as [`resolve`] finds them.
    fn try_open(&mut self, now: Instant) {
        let attempt = self
            .opener
            .names()
            .map_err(|e| e.to_string())
            .and_then(|(ins, outs)| {
                resolve(
                    self.config.input.as_deref(),
                    self.config.output.as_deref(),
                    &ins,
                    &outs,
                )
                .map_err(|unresolved| {
                    let names = unresolved
                        .missing
                        .iter()
                        .map(|(_, name)| format!("{name:?}"))
                        .collect::<Vec<_>>()
                        .join(" and ");
                    format!(
                        "{names} {} not on this computer ({}). Plug the radio interface in; \
                     the modem opens it when it appears",
                        if unresolved.missing.len() > 1 {
                            "are"
                        } else {
                            "is"
                        },
                        unresolved.why
                    )
                })
            });
        let opened = attempt.and_then(|resolved| {
            let config = AudioConfig {
                input: resolved.input.clone(),
                output: resolved.output.clone(),
                ..self.config.clone()
            };
            self.opener
                .open(&config)
                .map(|card| (card, resolved))
                .map_err(|e| e.to_string())
        });
        match opened {
            Ok((card, resolved)) => {
                // the silence's time goes into the clock before the card's starts
                self.carried.played += self.silence.played().saturating_sub(self.silence_from);
                self.state = CardState {
                    generation: self.state.generation + 1,
                    fault: None,
                    description: card.describe(),
                    input: resolved.input,
                    output: resolved.output,
                    moved: resolved.moved,
                    tries: 0,
                };
                self.card = Some(card);
                self.last_audio = now;
            }
            Err(why) => {
                let tries = self.state.tries + 1;
                let changed = self.state.fault.as_deref() != Some(why.as_str());
                self.state.tries = tries;
                if changed {
                    self.state = CardState {
                        generation: self.state.generation + 1,
                        fault: Some(why),
                        tries,
                        ..CardState::default()
                    };
                }
                self.next_try = now + retry_interval(now.duration_since(self.failed_at));
            }
        }
    }

    /// The open card has failed: close it, carry its counters, run on silence.
    fn fail(&mut self, why: String, now: Instant) {
        if let Some(card) = self.card.take() {
            self.carried = Carried {
                played: self.carried.played + card.played(),
                dropped: self.carried.dropped + card.dropped(),
                starved: self.carried.starved + card.starved(),
                capture: add_capture(self.carried.capture, card.capture_skips()),
                playback: add_playback(self.carried.playback, card.playback_skips()),
            };
        }
        self.silence_from = self.silence.played();
        // the silence's capture clock starts now, not when the daemon did
        self.silence.capture();
        self.failed_at = now;
        self.next_try = now + retry_interval(Duration::ZERO);
        self.state = CardState {
            generation: self.state.generation + 1,
            fault: Some(why),
            ..CardState::default()
        };
    }

    /// Check the open card, or try a closed one again when it is due.
    fn watch(&mut self, now: Instant, delivered: bool) {
        if let Some(card) = &self.card {
            if delivered {
                self.last_audio = now;
            }
            let failure = card.failure().or_else(|| {
                (now.duration_since(self.last_audio) >= STALL).then(|| {
                    format!(
                        "the sound card stopped delivering audio ({} s without any); it may \
                         have been unplugged",
                        STALL.as_secs()
                    )
                })
            });
            if let Some(why) = failure {
                self.fail(why, now);
            }
        } else if now >= self.next_try {
            self.try_open(now);
        }
    }

    fn io(&self) -> &dyn AudioIo {
        match &self.card {
            Some(card) => card.as_ref(),
            None => &self.silence,
        }
    }

    fn io_mut(&mut self) -> &mut dyn AudioIo {
        match &mut self.card {
            Some(card) => card.as_mut(),
            None => &mut self.silence,
        }
    }

    /// The silence's own clock, counted from where it began standing in.
    fn silence_played(&self) -> u64 {
        if self.card.is_some() {
            0
        } else {
            self.silence.played().saturating_sub(self.silence_from)
        }
    }
}

impl AudioIo for ResilientCard {
    fn capture(&mut self) -> Vec<f32> {
        // a card that fails here has delivered its last; one that opens delivers next pass
        let samples = self.io_mut().capture();
        self.watch(Instant::now(), !samples.is_empty());
        samples
    }

    fn capture_gap(&mut self) -> usize {
        self.io_mut().capture_gap()
    }

    fn playback(&mut self, samples: &[f32]) {
        self.io_mut().playback(samples);
    }

    fn queued(&self) -> usize {
        self.io().queued()
    }

    fn dropped(&self) -> usize {
        self.carried.dropped + self.io().dropped()
    }

    fn played(&self) -> u64 {
        let inner = if self.card.is_some() {
            self.io().played()
        } else {
            self.silence_played()
        };
        self.carried.played + inner
    }

    fn drains_at(&self) -> u64 {
        self.played() + self.io().queued() as u64
    }

    fn set_playing(&mut self, playing: bool) {
        self.io_mut().set_playing(playing);
    }

    fn starved(&self) -> usize {
        self.carried.starved + self.io().starved()
    }

    fn clear(&mut self) {
        self.io_mut().clear();
    }

    fn output_latency_s(&self) -> Option<f64> {
        self.io().output_latency_s()
    }

    fn capture_skips(&self) -> CaptureSkips {
        add_capture(self.carried.capture, self.io().capture_skips())
    }

    fn playback_skips(&self) -> PlaybackSkips {
        add_playback(self.carried.playback, self.io().playback_skips())
    }

    fn guard_onset(&mut self, guard: OnsetGuard) {
        self.io_mut().guard_onset(guard);
    }

    fn take_onset_drop(&mut self) -> Option<f64> {
        self.io_mut().take_onset_drop()
    }

    fn annotate(&mut self, tag: &str) {
        self.io_mut().annotate(tag);
    }

    fn card_state(&self) -> Option<CardState> {
        Some(self.state.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|&s| s.to_owned()).collect()
    }

    #[test]
    fn windows_instance_numbers_are_read_from_the_device_part_of_a_name() {
        assert_eq!(
            instance_of("Microphone (2- USB Audio CODEC)"),
            ("Microphone (USB Audio CODEC)".to_owned(), 2)
        );
        assert_eq!(
            instance_of("Speakers (USB Audio CODEC)"),
            ("Speakers (USB Audio CODEC)".to_owned(), 1)
        );
        assert_eq!(
            instance_of("12- USB Audio CODEC"),
            ("USB Audio CODEC".to_owned(), 12)
        );
        // a number that is part of the model's own name is not an instance
        assert_eq!(
            instance_of("Speakers (2-Port USB Hub Audio)"),
            ("Speakers (2-Port USB Hub Audio)".to_owned(), 1)
        );
        assert_eq!(
            instance_of("FTDX10 (USB AUDIO  CODEC)"),
            ("FTDX10 (USB AUDIO  CODEC)".to_owned(), 1)
        );
        assert_eq!(
            device_part("Speakers (3- USB Audio CODEC)"),
            "USB Audio CODEC"
        );
        assert_eq!(device_part("hw:CARD=Device,DEV=0"), "hw:CARD=Device,DEV=0");
    }

    #[test]
    fn a_name_that_is_there_is_used_as_it_is() {
        let ins = names(&[
            "Microphone (USB Audio CODEC)",
            "Microphone (2- USB Audio CODEC)",
        ]);
        let outs = names(&[
            "Speakers (USB Audio CODEC)",
            "Speakers (2- USB Audio CODEC)",
        ]);
        let r = resolve(
            Some("Microphone (USB Audio CODEC)"),
            Some("Speakers (USB Audio CODEC)"),
            &ins,
            &outs,
        )
        .expect("resolved");
        assert_eq!(r.input.as_deref(), Some("Microphone (USB Audio CODEC)"));
        assert_eq!(r.output.as_deref(), Some("Speakers (USB Audio CODEC)"));
        assert!(r.moved.is_empty(), "{:?}", r.moved);
    }

    #[test]
    fn the_same_codec_in_another_port_is_found_under_its_new_number_both_ways() {
        let ins = names(&[
            "Microphone (Realtek Audio)",
            "Microphone (2- USB Audio CODEC)",
        ]);
        let outs = names(&["Speakers (Realtek Audio)", "Speakers (2- USB Audio CODEC)"]);
        let r = resolve(
            Some("Microphone (USB Audio CODEC)"),
            Some("Speakers (USB Audio CODEC)"),
            &ins,
            &outs,
        )
        .expect("resolved");
        assert_eq!(r.input.as_deref(), Some("Microphone (2- USB Audio CODEC)"));
        assert_eq!(r.output.as_deref(), Some("Speakers (2- USB Audio CODEC)"));
        assert_eq!(r.moved.len(), 2);
        // and back again, when the configuration has the number and the plain one is there
        let ins = names(&["Microphone (USB Audio CODEC)"]);
        let outs = names(&["Speakers (USB Audio CODEC)"]);
        let r = resolve(
            Some("Microphone (2- USB Audio CODEC)"),
            Some("Speakers (2- USB Audio CODEC)"),
            &ins,
            &outs,
        )
        .expect("resolved");
        assert_eq!(r.output.as_deref(), Some("Speakers (USB Audio CODEC)"));
    }

    #[test]
    fn two_candidates_are_never_guessed_between() {
        let ins = names(&[
            "Microphone (2- USB Audio CODEC)",
            "Microphone (3- USB Audio CODEC)",
        ]);
        let outs = names(&[
            "Speakers (2- USB Audio CODEC)",
            "Speakers (3- USB Audio CODEC)",
        ]);
        let e = resolve(
            Some("Microphone (USB Audio CODEC)"),
            Some("Speakers (USB Audio CODEC)"),
            &ins,
            &outs,
        )
        .expect_err("ambiguous");
        assert_eq!(e.missing.len(), 2);
        assert!(e.why.contains("2 devices"), "{}", e.why);
    }

    #[test]
    fn another_model_is_never_taken_for_the_configured_one() {
        let ins = names(&["Microphone (2- USB PnP Sound Device)"]);
        let outs = names(&["Speakers (2- USB PnP Sound Device)"]);
        let e = resolve(
            Some("Microphone (USB Audio CODEC)"),
            Some("Speakers (USB Audio CODEC)"),
            &ins,
            &outs,
        )
        .expect_err("another model");
        assert_eq!(e.missing.len(), 2);
    }

    #[test]
    fn one_codecs_capture_is_never_paired_with_anothers_playback() {
        // the configured codec's capture side is still there (instance 1), its playback side
        // is not, and the only other playback of that model is another codec's (instance 2):
        // the station would listen to one radio and transmit through another
        let ins = names(&["Microphone (USB Audio CODEC)"]);
        let outs = names(&["Speakers (2- USB Audio CODEC)"]);
        let e = resolve(
            Some("Microphone (USB Audio CODEC)"),
            Some("Speakers (USB Audio CODEC)"),
            &ins,
            &outs,
        )
        .expect_err("mismatched sides");
        assert_eq!(
            e.missing,
            vec![(Direction::Output, "Speakers (USB Audio CODEC)".into())]
        );
        // two different interfaces configured on purpose move independently
        let ins = names(&["Microphone (2- USB Audio CODEC)"]);
        let outs = names(&["Speakers (SignaLink USB)"]);
        let r = resolve(
            Some("Microphone (USB Audio CODEC)"),
            Some("Speakers (SignaLink USB)"),
            &ins,
            &outs,
        )
        .expect("independent");
        assert_eq!(r.moved.len(), 1);
    }

    #[test]
    fn the_system_default_is_never_resolved() {
        let r = resolve(None, None, &[], &[]).expect("defaults");
        assert_eq!(r, Resolved::default());
    }

    #[test]
    fn a_failed_card_is_tried_every_second_then_every_ten() {
        assert_eq!(retry_interval(Duration::ZERO), Duration::from_secs(1));
        assert_eq!(
            retry_interval(Duration::from_secs(59)),
            Duration::from_secs(1)
        );
        assert_eq!(
            retry_interval(Duration::from_secs(60)),
            Duration::from_secs(10)
        );
    }

    /// A card that plays into a counter and fails when told to.
    struct FakeCard {
        failed: Arc<Mutex<Option<String>>>,
        played: u64,
        name: String,
    }

    impl AudioIo for FakeCard {
        fn capture(&mut self) -> Vec<f32> {
            vec![0.0; 480]
        }
        fn playback(&mut self, samples: &[f32]) {
            self.played += samples.len() as u64;
        }
        fn queued(&self) -> usize {
            0
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
        fn clear(&mut self) {}
    }

    impl LiveCard for FakeCard {
        fn failure(&self) -> Option<String> {
            self.failed.lock().ok().and_then(|f| f.clone())
        }
        fn describe(&self) -> String {
            self.name.clone()
        }
    }

    /// A machine whose devices the test changes.
    #[derive(Clone, Default)]
    struct FakeMachine {
        devices: Arc<Mutex<(Vec<String>, Vec<String>)>>,
        failed: Arc<Mutex<Option<String>>>,
        opened: Arc<Mutex<Vec<AudioConfig>>>,
    }

    impl Opener for FakeMachine {
        fn names(&mut self) -> Result<(Vec<String>, Vec<String>), AudioError> {
            Ok(self.devices.lock().expect("lock").clone())
        }
        fn open(&mut self, config: &AudioConfig) -> Result<Box<dyn LiveCard>, AudioError> {
            let (ins, outs) = self.devices.lock().expect("lock").clone();
            let there = |name: &Option<String>, list: &[String]| {
                name.as_ref().is_none_or(|n| list.contains(n))
            };
            if !there(&config.input, &ins) || !there(&config.output, &outs) {
                return Err(AudioError::NoDevice("gone".into()));
            }
            self.opened.lock().expect("lock").push(config.clone());
            *self.failed.lock().expect("lock") = None;
            Ok(Box::new(FakeCard {
                failed: Arc::clone(&self.failed),
                played: 0,
                name: format!("{:?}", config.output),
            }))
        }
    }

    fn digirig(instance: &str) -> (Vec<String>, Vec<String>) {
        (
            vec![format!("Microphone ({instance}USB Audio CODEC)")],
            vec![format!("Speakers ({instance}USB Audio CODEC)")],
        )
    }

    fn configured() -> AudioConfig {
        AudioConfig {
            input: Some("Microphone (USB Audio CODEC)".into()),
            output: Some("Speakers (USB Audio CODEC)".into()),
            ..AudioConfig::default()
        }
    }

    #[test]
    fn a_card_unplugged_runs_on_silence_and_comes_back_in_another_port_without_a_restart() {
        let machine = FakeMachine::default();
        *machine.devices.lock().expect("lock") = digirig("");
        let mut card = ResilientCard::with_opener(configured(), Box::new(machine.clone()));
        assert!(card.state().fault.is_none(), "{:?}", card.state());
        card.playback(&[0.5; 1000]);
        assert_eq!(card.played(), 1000);
        let generation = card.state().generation;

        // unplugged: the stream says so, the card is closed, and the clock runs on
        *machine.devices.lock().expect("lock") = (vec![], vec![]);
        *machine.failed.lock().expect("lock") = Some("device not available".into());
        card.capture();
        assert_eq!(card.state().fault.as_deref(), Some("device not available"));
        assert!(card.state().generation > generation);
        assert!(card.played() >= 1000, "the clock never steps back");

        // plugged into another port: found under its new number at the next try
        *machine.devices.lock().expect("lock") = digirig("2- ");
        card.next_try = Instant::now();
        card.capture();
        let state = card.state();
        assert!(state.fault.is_none(), "{state:?}");
        assert_eq!(
            state.output.as_deref(),
            Some("Speakers (2- USB Audio CODEC)")
        );
        assert_eq!(state.moved.len(), 2);
        assert!(card.played() >= 1000);
        let opened = machine.opened.lock().expect("lock");
        assert_eq!(opened.len(), 2);
        assert_eq!(
            opened[1].input.as_deref(),
            Some("Microphone (2- USB Audio CODEC)")
        );
    }

    #[test]
    fn a_card_missing_at_start_is_tried_again_and_opened_when_it_appears() {
        let machine = FakeMachine::default();
        let mut card = ResilientCard::with_opener(configured(), Box::new(machine.clone()));
        let fault = card.state().fault.clone().expect("no card yet");
        assert!(fault.contains("not on this computer"), "{fault}");
        // not before it is due
        *machine.devices.lock().expect("lock") = digirig("");
        card.capture();
        assert!(card.state().fault.is_some());
        card.next_try = Instant::now();
        card.capture();
        assert!(card.state().fault.is_none(), "{:?}", card.state());
        assert!(card.state().moved.is_empty(), "{:?}", card.state().moved);
    }

    #[test]
    fn two_codecs_of_the_model_keep_the_station_waiting_and_say_why() {
        let machine = FakeMachine::default();
        *machine.devices.lock().expect("lock") = (
            names(&[
                "Microphone (2- USB Audio CODEC)",
                "Microphone (3- USB Audio CODEC)",
            ]),
            names(&[
                "Speakers (2- USB Audio CODEC)",
                "Speakers (3- USB Audio CODEC)",
            ]),
        );
        let card = ResilientCard::with_opener(configured(), Box::new(machine.clone()));
        let fault = card.state().fault.clone().expect("not guessed");
        assert!(fault.contains("cannot be told"), "{fault}");
        assert!(machine.opened.lock().expect("lock").is_empty());
    }
}
