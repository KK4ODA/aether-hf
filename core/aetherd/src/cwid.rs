//! Morse identification (roadmap P3-6, ADR-0075).
//!
//! A station has to say who it is. Under link protocol 8 the callsigns are on the air only in a
//! session's call and acceptance, a probe, a beacon and a datagram: a session's own frames carry
//! a session number. So, as VARA HF does, a station operating under a **United States callsign
//! always sends its Morse identifier** at the end of each session, whatever its setting or a
//! host program's `CWID OFF` says ([`required_for`]). It is the last thing each station sends
//! for the session; nothing inside a session, a beacon or a KISS frame carries one. It is also a
//! courtesy on a shared band: anybody listening with an ear rather than a decoder learns who is
//! transmitting.
//!
//! For any other callsign it is the operator's choice, off by default. A station that
//! identifies when it does not need to is spending air time; one that fails to when it must is
//! breaking the rules. Only the operator knows which applies outside the United States.
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

/// Why a station sends its Morse identifier whatever its operator set (ADR-0075).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Required {
    /// The station operates under a United States callsign: §97.119, and VARA HF's own rule,
    /// which host programs and their operators expect of a modem in its place.
    UnitedStates,
}

impl Required {
    /// The reason, in the operator's words: the panel's note and `status.identifier`.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::UnitedStates => "Required for US callsigns (§97.119)",
        }
    }
}

/// Whether identifying as `callsign` requires the Morse identifier, and why: the one rule the
/// station, the log, `status.identifier` and the panel (`identifier.check`) all use.
///
/// It goes by where the station operates, which is what decides whose rules apply: the
/// callsign's own prefix, or the location a portable form names — a prefix in front
/// (`W4/G4ABC`, `VE3/KK4ODA`) or a location after the callsign (`G4ABC/W4`, `KK4ODA/VE3`). A
/// suffix with no digit (`/P`, `/M`, `/MM`, `/QRP`) or a bare call-area digit (`/4`) moves
/// nothing. An SSID and VARA's `-T`/`-R` are not part of the callsign.
#[must_use]
pub fn required_for(callsign: &str) -> Option<Required> {
    is_united_states(&location_of(callsign)).then_some(Required::UnitedStates)
}

/// The part of a callsign that says where its station operates (see [`required_for`]).
fn location_of(callsign: &str) -> String {
    let call = callsign.trim().to_ascii_uppercase();
    let call = call.split('-').next().unwrap_or_default();
    let parts: Vec<&str> = call.split('/').filter(|p| !p.is_empty()).collect();
    let Some(base) = parts
        .iter()
        .enumerate()
        .max_by_key(|(_, part)| part.len())
        .map(|(index, _)| index)
    else {
        return String::new();
    };
    // a prefix in front is where the station is; a location after the callsign has a letter
    // and a digit (VE3, W4, KH6), where a modifier (P, MM, QRP) or a call area (4) does not
    let names_a_place = |part: &str| {
        part.chars().any(|c| c.is_ascii_digit()) && part.chars().any(|c| c.is_ascii_alphabetic())
    };
    let location = parts[..base]
        .first()
        .copied()
        .or_else(|| parts[base + 1..].iter().copied().find(|p| names_a_place(p)))
        .unwrap_or(parts[base]);
    location.to_owned()
}

/// Whether a prefix is one the ITU allocates to the United States: AA–AL, K, N and W.
fn is_united_states(prefix: &str) -> bool {
    let mut chars = prefix.chars();
    match (chars.next(), chars.next()) {
        (Some('K' | 'N' | 'W'), _) => true,
        (Some('A'), Some(second)) => ('A'..='L').contains(&second),
        _ => false,
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
    fn a_us_callsign_must_identify_and_any_other_may() {
        // the ITU's allocations to the United States: AA–AL, K, N, W
        for us in [
            "KK4ODA", "W4ODA", "N7XYZ", "K0XYZ", "AA1ZZ", "AL7XX", "KH6XX", "KP4XX", "WD5EMA",
            "kk4oda", " KK4ODA ",
        ] {
            assert_eq!(required_for(us), Some(Required::UnitedStates), "{us}");
        }
        for other in [
            "VE3XYZ", "G4XYZ", "LU1XYZ", "PY2XYZ", "EA8XYZ", "VK2XYZ", "JA1XX", "XE1XX", "4X1AB",
            "AM1XX", "A61XX", "", "/",
        ] {
            assert_eq!(required_for(other), None, "{other}");
        }
    }

    #[test]
    fn an_ssid_and_a_suffix_that_names_no_place_change_nothing() {
        for us in [
            "KK4ODA-1", "KK4ODA-15", "KK4ODA-T", "KK4ODA-1-T", "KK4ODA/P", "KK4ODA/M",
            "KK4ODA/MM", "KK4ODA/QRP", "KK4ODA/4", "KK4ODA/P-1",
        ] {
            assert!(required_for(us).is_some(), "{us}");
        }
        for other in ["G4ABC/P", "VE3XYZ-2", "G4ABC/MM"] {
            assert!(required_for(other).is_none(), "{other}");
        }
    }

    #[test]
    fn a_portable_form_goes_by_where_the_station_operates() {
        // a US prefix in front, or a US location after the callsign: operating in the US
        for us in ["W4/G4ABC", "K/G4ABC", "KH6/VE3XYZ", "G4ABC/W4", "VE3XYZ/KH6", "W4/G4ABC/P"] {
            assert!(required_for(us).is_some(), "{us}");
        }
        // a US station operating elsewhere, either way of writing it: that country's rules
        for other in ["VE3/KK4ODA", "KK4ODA/VE3", "VE3/KK4ODA/P", "KK4ODA/VE3/P", "G/KK4ODA"] {
            assert!(required_for(other).is_none(), "{other}");
        }
    }

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
        assert!(!audio.is_empty(), "audio is empty");
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
        let unexpected = cw.audio("", RATE);
        assert!(unexpected.is_empty(), "{unexpected:?}");
        let unexpected = cw.audio("!!!", RATE);
        assert!(unexpected.is_empty(), "{unexpected:?}");
        assert!((cw.duration_s("") - 0.0).abs() < 1e-12);
    }
}
