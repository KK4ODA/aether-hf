//! Morse identification (roadmap P3-6).
//!
//! A station has to say who it is. Aether's own frames carry both callsigns, so in a
//! jurisdiction that accepts identification by the mode in use — the United States, under
//! §97.119(b)(3), for a documented digital code — nothing more is needed. Elsewhere the rules
//! are read more strictly, and a Morse identifier at the end of a transmission is the usual
//! answer. It is also a courtesy on a shared band: anybody listening with an ear rather than
//! a decoder learns who is transmitting.
//!
//! Off by default. A station that identifies when it does not need to is spending air time
//! and annoying its neighbours; one that fails to when it must is breaking the rules. Only
//! the operator knows which applies, so the daemon does not guess.
//!
//! # Keying
//!
//! Each element is a sine gated by a raised-cosine envelope. Switching a tone on and off
//! abruptly splatters — the discontinuity is a step, and a step has energy everywhere — and
//! key clicks from digital modes are exactly the kind of thing that makes a mode unwelcome.
//! A 5 ms rise and fall is the usual figure and keeps the sidebands inside the passband the
//! rest of the waveform already occupies.
//!
//! Timing is the PARIS standard: a dit is `1.2 / wpm` seconds, a dah is three dits, the gap
//! inside a character is one dit, between characters three, and between words seven.

/// Morse for the characters a callsign can contain.
///
/// The link layer's callsign alphabet is `A–Z 0–9 - /`, so that is exactly what is here.
const MORSE: &[(char, &str)] = &[
    ('A', ".-"),
    ('B', "-..."),
    ('C', "-.-."),
    ('D', "-.."),
    ('E', "."),
    ('F', "..-."),
    ('G', "--."),
    ('H', "...."),
    ('I', ".."),
    ('J', ".---"),
    ('K', "-.-"),
    ('L', ".-.."),
    ('M', "--"),
    ('N', "-."),
    ('O', "---"),
    ('P', ".--."),
    ('Q', "--.-"),
    ('R', ".-."),
    ('S', "..."),
    ('T', "-"),
    ('U', "..-"),
    ('V', "...-"),
    ('W', ".--"),
    ('X', "-..-"),
    ('Y', "-.--"),
    ('Z', "--.."),
    ('0', "-----"),
    ('1', ".----"),
    ('2', "..---"),
    ('3', "...--"),
    ('4', "....-"),
    ('5', "....."),
    ('6', "-...."),
    ('7', "--..."),
    ('8', "---.."),
    ('9', "----."),
    ('/', "-..-."),
    ('-', "-....-"),
];

/// Rise and fall time of each element, in seconds. Shorter than this and the clicks are
/// audible either side of the signal.
const EDGE_S: f64 = 0.005;

/// How a station identifies in Morse.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CwId {
    /// Speed in words per minute, by the PARIS standard.
    pub wpm: f64,
    /// Tone frequency in hertz, in the audio passband.
    pub tone_hz: f64,
    /// Amplitude as a fraction of full scale.
    pub level: f64,
}

impl Default for CwId {
    fn default() -> Self {
        Self {
            // fast enough not to waste air time, slow enough for a human to copy by ear
            wpm: 20.0,
            // near the middle of the occupied band, so it passes the same filters the
            // waveform does and needs no separate thought about bandwidth
            tone_hz: 1500.0,
            // below the data waveform, because it is an identifier and not the signal
            level: 0.2,
        }
    }
}

/// The Morse for one character, or `None` if it has none.
#[must_use]
pub fn morse(character: char) -> Option<&'static str> {
    let upper = character.to_ascii_uppercase();
    MORSE
        .iter()
        .find(|(c, _)| *c == upper)
        .map(|(_, code)| *code)
}

impl CwId {
    /// Seconds in one dit.
    #[must_use]
    pub fn dit_s(&self) -> f64 {
        1.2 / self.wpm
    }

    /// How long identifying with this callsign will take, in seconds.
    ///
    /// A caller needs this before it transmits: the identifier is part of the transmission
    /// and counts against the key-time watchdog like everything else.
    #[must_use]
    pub fn duration_s(&self, callsign: &str) -> f64 {
        let dit = self.dit_s();
        let mut units = 0.0;
        let mut first = true;
        for character in callsign.chars() {
            let Some(code) = morse(character) else {
                continue;
            };
            if !first {
                units += 3.0; // between characters
            }
            first = false;
            for (index, element) in code.chars().enumerate() {
                if index > 0 {
                    units += 1.0; // inside a character
                }
                units += if element == '-' { 3.0 } else { 1.0 };
            }
        }
        units * dit
    }

    /// Audio for one identification, at the given sample rate.
    ///
    /// Empty if the callsign contains nothing that can be sent.
    #[must_use]
    pub fn audio(&self, callsign: &str, sample_rate: f64) -> Vec<f32> {
        let dit = self.dit_s();
        let mut out: Vec<f32> = Vec::new();
        let mut first = true;
        for character in callsign.chars() {
            let Some(code) = morse(character) else {
                continue;
            };
            if !first {
                Self::push_silence(&mut out, 3.0 * dit, sample_rate);
            }
            first = false;
            for (index, element) in code.chars().enumerate() {
                if index > 0 {
                    Self::push_silence(&mut out, dit, sample_rate);
                }
                let length = if element == '-' { 3.0 * dit } else { dit };
                self.push_tone(&mut out, length, sample_rate);
            }
        }
        out
    }

    fn push_silence(out: &mut Vec<f32>, seconds: f64, sample_rate: f64) {
        let count = (seconds * sample_rate).round() as usize;
        out.extend(std::iter::repeat_n(0.0f32, count));
    }

    /// One element: a sine gated by a raised-cosine envelope, so it does not click.
    fn push_tone(&self, out: &mut Vec<f32>, seconds: f64, sample_rate: f64) {
        let count = (seconds * sample_rate).round() as usize;
        if count == 0 {
            return;
        }
        // an edge cannot be longer than half the element, or the tone never reaches full
        let edge = ((EDGE_S * sample_rate).round() as usize)
            .min(count / 2)
            .max(1);
        let start = out.len();
        for index in 0..count {
            let phase =
                2.0 * std::f64::consts::PI * self.tone_hz * (start + index) as f64 / sample_rate;
            let envelope = if index < edge {
                0.5 * (1.0 - (std::f64::consts::PI * index as f64 / edge as f64).cos())
            } else if index >= count - edge {
                let from_end = count - 1 - index;
                0.5 * (1.0 - (std::f64::consts::PI * from_end as f64 / edge as f64).cos())
            } else {
                1.0
            };
            out.push((self.level * envelope * phase.sin()) as f32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    #[test]
    fn every_character_a_callsign_can_hold_has_a_code() {
        // the link layer packs `A-Z 0-9 - /`, so anything it will carry must be sendable
        for character in "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-/".chars() {
            assert!(morse(character).is_some(), "no Morse for {character}");
        }
        assert!(morse('!').is_none());
        assert_eq!(morse('w'), morse('W'), "lower case should still send");
    }

    #[test]
    fn the_codes_are_the_ones_everybody_knows() {
        // a spot check against the international alphabet; an error here is inaudible to a
        // test that only measures durations
        assert_eq!(morse('S'), Some("..."));
        assert_eq!(morse('O'), Some("---"));
        assert_eq!(morse('K'), Some("-.-"));
        assert_eq!(morse('4'), Some("....-"));
        assert_eq!(morse('/'), Some("-..-."));
    }

    #[test]
    fn timing_follows_the_paris_standard() {
        let cw = CwId {
            wpm: 20.0,
            ..CwId::default()
        };
        assert!((cw.dit_s() - 0.06).abs() < 1e-12, "20 wpm is a 60 ms dit");

        // PARIS itself is 50 units by definition, which is what makes a word per minute mean
        // anything at all
        let units = cw.duration_s("PARIS") / cw.dit_s();
        assert!(
            (units - 43.0).abs() < 1e-9,
            "PARIS without its trailing word gap is 43 units, got {units}"
        );
    }

    #[test]
    fn the_audio_is_as_long_as_the_timing_says() {
        // a caller budgets the key-time watchdog against `duration_s` before transmitting, so
        // the two must not disagree
        let cw = CwId::default();
        for call in ["W4ODA", "KK4XYZ", "M0ABC/P"] {
            let audio = cw.audio(call, RATE);
            let expected = (cw.duration_s(call) * RATE).round() as usize;
            let difference = audio.len().abs_diff(expected);
            assert!(
                difference <= call.len(),
                "{call}: {} samples against {expected} expected",
                audio.len()
            );
        }
    }

    #[test]
    fn the_identifier_is_audible_and_stays_inside_full_scale() {
        let cw = CwId::default();
        let audio = cw.audio("W4ODA", RATE);
        let peak = audio.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(peak > 0.5 * cw.level as f32, "too quiet to copy: {peak}");
        assert!(peak <= 1.0, "it would clip the sound card: {peak}");
        assert!(
            (peak - cw.level as f32).abs() < 0.01,
            "the level is not what was asked for: {peak} against {}",
            cw.level
        );
    }

    #[test]
    fn elements_are_shaped_rather_than_switched() {
        // A tone switched on abruptly is a step, and a step has energy everywhere. Key clicks
        // from a digital mode are exactly what makes one unwelcome on a shared band.
        let cw = CwId::default();
        let audio = cw.audio("E", RATE); // a single dit
        assert!(!audio.is_empty());
        let edge = (EDGE_S * RATE) as usize;
        let first = audio[..edge / 2]
            .iter()
            .fold(0.0f32, |a, &b| a.max(b.abs()));
        let middle = audio[edge..audio.len() - edge]
            .iter()
            .fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(
            first < 0.5 * middle,
            "the tone starts at {first} against {middle} in the middle, which is a click"
        );
        let last = audio[audio.len() - edge / 2..]
            .iter()
            .fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(last < 0.5 * middle, "and it ends with one: {last}");
    }

    #[test]
    fn the_tone_is_where_it_was_asked_to_be() {
        // it has to sit inside the passband the rest of the waveform already occupies
        let cw = CwId {
            tone_hz: 1500.0,
            wpm: 10.0,
            ..CwId::default()
        };
        let audio = cw.audio("T", RATE); // one long dah, plenty to measure
        let correlate = |frequency: f64| {
            let (mut re, mut im) = (0.0, 0.0);
            for (index, &sample) in audio.iter().enumerate() {
                let phase = 2.0 * std::f64::consts::PI * frequency * index as f64 / RATE;
                re += f64::from(sample) * phase.cos();
                im += f64::from(sample) * phase.sin();
            }
            re.hypot(im) / audio.len() as f64
        };
        let at_tone = correlate(1500.0);
        assert!(at_tone > 5.0 * correlate(1000.0), "not at 1500 Hz");
        assert!(at_tone > 5.0 * correlate(2200.0));
    }

    #[test]
    fn a_callsign_with_nothing_sendable_produces_nothing() {
        let cw = CwId::default();
        assert!(cw.audio("", RATE).is_empty());
        assert!(cw.audio("!!!", RATE).is_empty());
        assert!((cw.duration_s("") - 0.0).abs() < 1e-12);
    }
}
