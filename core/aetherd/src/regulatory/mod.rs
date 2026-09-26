//! The regulatory policy (ADR-0018): whether a transmission is lawful where it would go, and
//! which rungs of the link's ladder are.
//!
//! One place decides, for the panel, the control API and — the one that matters — the
//! transmitter. `Station` asks [`Policy::authorize`] before every transmission it renders,
//! whatever it is (a data burst, an acknowledgement, a call, a probe, a beacon, a
//! disconnect, the Morse identifier, a tune tone, a drive burst, a keying test), and keys the
//! radio only with the [`Authorization`] that comes back, which nothing else can make. The
//! link's rate control climbs only as far as [`Policy::ceiling`] lets it.
//!
//! The rules are data: a [`Profile`] per administration (the first,
//! `data/regulatory/us-fcc-part97.json`, read from the e-CFR), and the spectrum every Aether
//! transmission occupies, measured from the transmitter itself (`data/occupancy.json`). The
//! question is always the RF range a transmission covers — the dial, the sideband and the
//! measured audio edges — never a mode's name. Nothing here guesses: a question it cannot
//! answer (no dial, no sideband, no license class, no control mode, a profile that did not
//! load) is a refusal that says what is missing.
//!
//! What the rules say, as implemented for the United States:
//!
//! * RTTY and data only in the §97.305(c) segments, the whole occupied range inside one of
//!   them (§97.307(b)), within the operator's §97.301 privileges (a Technician's HF data is
//!   28.0–28.3 MHz, §97.307(f)(9)), and no wider than 2.8 kHz (§97.307(f)(3));
//! * 60 m: the four channels, the dial on the carrier frequency 1.5 kHz below the centre and
//!   the emission inside the channel's 2.8 kHz, or the 5351.5–5366.5 kHz segment; phone,
//!   RTTY, data and CW only (§97.303(h)(3), §97.307(f)(14));
//! * CW and test emissions anywhere in the operator's privileges (§97.305(a), (b));
//! * automatic control (§97.3(a)(6), §97.109(d)) of RTTY and data in the §97.221(b)
//!   segments; elsewhere only answering a station under local or remote control, at 500 Hz
//!   or less (§97.221(c)); never on 60 m. Local and remote control (§97.109(b), (c)) carry no
//!   such conditions — software that retries, adapts and acknowledges is not automatic
//!   control, and neither is a control link over a network.

pub mod occupancy;
pub mod profile;

use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub use occupancy::{AirOccupancy, Edges, Reading};
pub use profile::{Profile, Range, khz};

/// How the station is controlled (47 CFR §97.3(a)(6), (39); §97.109).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ControlMode {
    /// The control operator is at the control point.
    Local,
    /// The control operator is at the control point through a control link.
    Remote,
    /// No control operator at the control point: the station keeps to the rules itself.
    Automatic,
}

impl ControlMode {
    /// Parse the configuration's word for it.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "local" => Some(Self::Local),
            "remote" => Some(Self::Remote),
            "automatic" => Some(Self::Automatic),
            _ => None,
        }
    }

    /// The configuration's word for it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
            Self::Automatic => "automatic",
        }
    }
}

/// The operator's license class (§97.301).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LicenseClass {
    /// Novice.
    Novice,
    /// Technician.
    Technician,
    /// General.
    General,
    /// Advanced.
    Advanced,
    /// Amateur Extra.
    Extra,
}

impl LicenseClass {
    /// Parse the configuration's word for it.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "novice" => Some(Self::Novice),
            "technician" => Some(Self::Technician),
            "general" => Some(Self::General),
            "advanced" => Some(Self::Advanced),
            "extra" => Some(Self::Extra),
            _ => None,
        }
    }

    /// The configuration's word for it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Novice => "novice",
            Self::Technician => "technician",
            Self::General => "general",
            Self::Advanced => "advanced",
            Self::Extra => "extra",
        }
    }

    /// Its name in a sentence.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Novice => "Novice",
            Self::Technician => "Technician",
            Self::General => "General",
            Self::Advanced => "Advanced",
            Self::Extra => "Amateur Extra",
        }
    }
}

/// Which sideband the radio transmits the modem's audio on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sideband {
    /// Upper: RF = dial + audio.
    Usb,
    /// Lower: RF = dial − audio, the spectrum mirrored.
    Lsb,
}

impl Sideband {
    /// Parse the configuration's word for it.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "usb" => Some(Self::Usb),
            "lsb" => Some(Self::Lsb),
            _ => None,
        }
    }

    /// The configuration's word for it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Usb => "usb",
            Self::Lsb => "lsb",
        }
    }
}

/// What a transmission is, as the rules class it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EmissionKind {
    /// RTTY or data: Aether's frames (J2D).
    Data,
    /// CW: the Morse identifier (J2A, §97.3(c)(1)).
    Cw,
    /// A test emission: the tune tone (§97.3(c)(9)).
    Test,
    /// Nothing: the transmitter keyed with no audio — the keying test.
    Nothing,
}

impl EmissionKind {
    /// The profile's word for it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::Cw => "cw",
            Self::Test => "test",
            Self::Nothing => "nothing",
        }
    }
}

/// Who began the exchange a transmission belongs to — §97.221(c)(1)'s question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// This station starts it: a call, a probe, a beacon, every frame of a session it began.
    Originate,
    /// It answers another station: a call's acceptance, a probe's answer, every frame of a
    /// session the other station began — responding to interrogation.
    Respond,
    /// Somebody at the control point is testing the station: tune, drive, a keying test.
    Operator,
}

/// Where the dial frequency came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DialSource {
    /// Read from the radio (CAT or rigctld) just before transmitting.
    Radio,
    /// Entered by the operator: the radio cannot say, and Aether cannot see it move.
    Declared,
}

/// The station's regulatory settings, as the configuration's `[regulatory]` names them.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// The profile: `""` not chosen, `"none"`, or a profile's identifier.
    pub profile: String,
    /// How the station is controlled.
    pub control: Option<ControlMode>,
    /// The control operator's class.
    pub license: Option<LicenseClass>,
    /// The sideband the radio transmits the modem's audio on.
    pub sideband: Option<Sideband>,
    /// The station's ITU region.
    pub itu_region: u8,
    /// Room kept at a segment's edges, hertz.
    pub margin_hz: f64,
    /// Whether to say where the voluntary band plan differs.
    pub band_plan: bool,
    /// The dial the operator entered, for a radio that cannot report its own.
    pub dial_hz: Option<u64>,
    /// Log the basis of every permitted automatic-control transmission, not only refusals.
    pub log_permitted: bool,
}

impl Settings {
    /// No regulatory checks: what a test harness and a station on another administration's
    /// rules run under.
    #[must_use]
    pub fn unchecked() -> Self {
        Self {
            profile: "none".to_owned(),
            control: Some(ControlMode::Local),
            license: None,
            sideband: Some(Sideband::Usb),
            itu_region: 2,
            margin_hz: 50.0,
            band_plan: true,
            dial_hz: None,
            log_permitted: false,
        }
    }
}

/// Everything about the station the rules need, besides the transmission itself.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Situation {
    /// The dial (the suppressed carrier), hertz, when known.
    pub dial_hz: Option<f64>,
    /// Where it came from.
    pub dial_source: Option<DialSource>,
    /// The sideband, when set.
    pub sideband: Option<Sideband>,
    /// How the station is controlled, when set.
    pub control: Option<ControlMode>,
    /// The operator's class, when set.
    pub license: Option<LicenseClass>,
    /// The ITU region the station is in.
    pub itu_region: u8,
    /// Room kept between the occupied edges and a segment's, hertz: frequency error, and the
    /// radio's own transmit chain.
    pub margin_hz: f64,
    /// Whether to say where the voluntary band plan differs.
    pub band_plan: bool,
    /// The transmitter's power, watts, when the operator said.
    pub power_w: Option<f64>,
}

/// A transmission to judge.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Transmission {
    /// What it is, in words: "a data burst at rung 7 (BPSK-1/3)".
    pub what: String,
    /// How the rules class it.
    pub kind: EmissionKind,
    /// The audio it occupies, absolute hertz.
    pub audio: Edges,
    /// Who began the exchange.
    pub direction: Direction,
}

/// The policy's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    /// Permitted.
    Legal,
    /// Permitted, with something the operator should know: outside the voluntary band plan,
    /// or a power limit that applies.
    Warning,
    /// Not permitted: nothing is sent.
    Blocked,
}

/// A decision and everything that went into it: for the indicator, the log and the
/// diagnostics panel.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Decision {
    /// The verdict.
    pub verdict: Verdict,
    /// A word for the reason, for a program: `permitted`, `outside_data_segment`, …
    pub code: &'static str,
    /// The rule that decided.
    pub rule: String,
    /// One line, for the indicator.
    pub summary: String,
    /// The reasoning, with the numbers.
    pub detail: String,
    /// What was judged.
    pub what: String,
    /// How the rules class it.
    pub kind: EmissionKind,
    /// Who began the exchange.
    pub direction: Direction,
    /// The dial, hertz.
    pub dial_hz: Option<f64>,
    /// Where it came from.
    pub dial_source: Option<DialSource>,
    /// The sideband.
    pub sideband: Option<Sideband>,
    /// The station's control.
    pub control: Option<ControlMode>,
    /// The operator's class.
    pub license: Option<LicenseClass>,
    /// The occupied audio, hertz, as the profile reads bandwidth.
    pub audio_low_hz: f64,
    /// Its upper edge.
    pub audio_high_hz: f64,
    /// The occupied RF range, hertz, when the dial is known.
    pub rf_low_hz: Option<f64>,
    /// Its upper edge.
    pub rf_high_hz: Option<f64>,
    /// The occupied bandwidth, hertz.
    pub bandwidth_hz: f64,
    /// The room kept at a segment's edges.
    pub margin_hz: f64,
    /// The band.
    pub band: Option<String>,
    /// The segment the transmission is judged against.
    pub segment: Option<Range>,
    /// That segment's rule.
    pub segment_rule: Option<String>,
    /// The §97.221(b) segment, for an automatically controlled station inside one.
    pub automatic_segment: Option<Range>,
    /// What the voluntary band plan says, when it was asked.
    pub guidance: Option<String>,
    /// Anything else the operator should know.
    pub notes: Vec<String>,
}

impl Decision {
    /// Whether the transmission may go.
    #[must_use]
    pub fn allowed(&self) -> bool {
        self.verdict != Verdict::Blocked
    }
}

/// Leave to transmit. Made only by [`Policy::authorize`], so a transmission the policy has
/// not judged cannot key the radio: the station's keying path asks for one.
#[derive(Debug, Clone, PartialEq)]
pub struct Authorization {
    decision: Decision,
}

impl Authorization {
    /// The decision it was made from.
    #[must_use]
    pub fn decision(&self) -> &Decision {
        &self.decision
    }
}

/// The fastest rung the rules allow, and why the one above it is not.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Ceiling {
    /// The fastest rung allowed; `None` when none is.
    pub rung: Option<usize>,
    /// The refusal of the rung above it, when there is one.
    pub limit: Option<Decision>,
}

/// A dial range where a transmission fits a segment.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SafeDial {
    /// The band.
    pub band: String,
    /// The segment it fits, as the rules name it.
    pub segment: Range,
    /// Why this segment: the rule.
    pub rule: String,
    /// The lowest dial, hertz.
    pub dial_low_hz: f64,
    /// The highest dial.
    pub dial_high_hz: f64,
}

/// The policy in force: an administration's rules, or the operator's word that none apply,
/// or — failing either — nothing is sent.
#[derive(Debug, Clone)]
pub enum Policy {
    /// No profile chosen yet: nothing is sent until one is.
    Unset,
    /// The operator has said no profile applies (another administration's rules): Aether
    /// checks nothing and the control operator everything.
    NoProfile,
    /// An administration's rules.
    Rules(Arc<Profile>),
    /// A profile that could not be loaded: nothing is sent.
    Broken(String),
}

/// The RF range an audio range covers on a dial and a sideband: `dial + audio` on USB, and
/// mirrored, `dial − audio`, on LSB. The one place the conversion is made.
#[must_use]
pub fn rf_range(dial_hz: f64, sideband: Sideband, audio_low: f64, audio_high: f64) -> (f64, f64) {
    match sideband {
        Sideband::Usb => (dial_hz + audio_low, dial_hz + audio_high),
        Sideband::Lsb => (dial_hz - audio_high, dial_hz - audio_low),
    }
}

/// The dial range that puts an audio range inside `segment` with `margin` to spare on each
/// side, if any does.
#[must_use]
pub fn dial_range(
    segment: Range,
    sideband: Sideband,
    audio_low: f64,
    audio_high: f64,
    margin: f64,
) -> Option<(f64, f64)> {
    let (low, high) = match sideband {
        Sideband::Usb => (
            segment.low_hz - audio_low + margin,
            segment.high_hz - audio_high - margin,
        ),
        Sideband::Lsb => (
            segment.low_hz + audio_high + margin,
            segment.high_hz + audio_low - margin,
        ),
    };
    (low <= high).then_some((low, high))
}

/// The policy's settings, as the configuration holds them, resolved.
impl Policy {
    /// The policy a configuration names: `""` unset, `"none"`, or a profile's identifier.
    #[must_use]
    pub fn from_setting(id: &str) -> Self {
        match id.trim() {
            "" => Self::Unset,
            "none" => Self::NoProfile,
            other => match profile::load(other) {
                Ok(profile) => Self::Rules(Arc::new(profile)),
                Err(error) => Self::Broken(error),
            },
        }
    }

    /// The profile's identifier, or what stands in for one.
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Unset => "",
            Self::NoProfile => "none",
            Self::Rules(profile) => &profile.id,
            Self::Broken(_) => "broken",
        }
    }

    /// The profile, when rules apply.
    #[must_use]
    pub fn profile(&self) -> Option<&Profile> {
        match self {
            Self::Rules(profile) => Some(profile),
            _ => None,
        }
    }

    /// The fastest an automatic Morse identifier may be sent, when the rules say.
    #[must_use]
    pub fn cw_id_max_wpm(&self) -> Option<f64> {
        self.profile()
            .and_then(|p| p.cw_id.as_ref())
            .map(|c| c.max_wpm)
    }

    /// Judge a transmission, and hand back leave to send it when it may go.
    ///
    /// # Errors
    /// The refusal, when it may not.
    pub fn authorize(
        &self,
        s: &Situation,
        tx: &Transmission,
    ) -> Result<Authorization, Box<Decision>> {
        let decision = self.evaluate(s, tx);
        if decision.allowed() {
            Ok(Authorization { decision })
        } else {
            Err(Box::new(decision))
        }
    }

    /// Judge a transmission.
    #[must_use]
    pub fn evaluate(&self, s: &Situation, tx: &Transmission) -> Decision {
        let draft = Draft::new(s, tx, self.reading());
        match self {
            Self::Unset => draft.blocked(
                "no_profile",
                "",
                "TX BLOCKED: choose a regulatory profile (Setup, step 1)",
                "Aether does not know which rules this station operates under. Choose the \
                 United States (FCC Part 97) profile, or \"none\" if another administration's \
                 rules apply and you will check every transmission yourself.",
            ),
            Self::Broken(error) => draft.blocked(
                "profile_broken",
                "",
                "TX BLOCKED: the regulatory profile could not be read",
                &format!("{error}. Nothing is sent until it can be."),
            ),
            Self::NoProfile => draft.legal(
                "no_rules",
                "",
                "No regulatory profile: you check every transmission",
                "The operator has chosen no regulatory profile: Aether makes no regulatory \
                 checks, and the control operator is responsible for every transmission.",
            ),
            Self::Rules(profile) => evaluate(profile, s, tx, draft),
        }
    }

    /// The refusal of a transmission whose spectrum is not known: nothing is measured for it,
    /// so nothing says where it would land.
    #[must_use]
    pub fn unmeasured(&self, s: &Situation, what: &str, why: &str) -> Decision {
        let tx = Transmission {
            what: what.to_owned(),
            kind: EmissionKind::Data,
            audio: Edges::tone(0.0, 0.0),
            direction: Direction::Originate,
        };
        Draft::new(s, &tx, self.reading()).blocked(
            "unmeasured",
            "",
            "TX BLOCKED: the signal's spectrum is not known",
            &format!("{why}. Without it Aether cannot say where the signal lands."),
        )
    }

    fn reading(&self) -> Reading {
        self.profile()
            .map_or(Reading::Wider, |p| p.bandwidth.reading)
    }

    /// The fastest rung of `air` the rules allow for a data exchange of `direction`: each
    /// rung judged with its control frame and every slower rung (so the answer is always a
    /// run from rung 0, and a rung's acknowledgements are as lawful as its data).
    #[must_use]
    pub fn ceiling(
        &self,
        s: &Situation,
        air: &AirOccupancy,
        direction: Direction,
        is_floor: &dyn Fn(usize) -> bool,
    ) -> Ceiling {
        let mut envelope: Option<Edges> = None;
        let mut best = None;
        for (index, rung) in air.rungs.iter().enumerate() {
            let edges = rung.edges.union(air.control(is_floor(index)));
            let edges = envelope.map_or(edges, |e| e.union(edges));
            envelope = Some(edges);
            let tx = Transmission {
                what: format!("rung {index} ({})", rung.name),
                kind: EmissionKind::Data,
                audio: edges,
                direction,
            };
            let decision = self.evaluate(s, &tx);
            if !decision.allowed() {
                return Ceiling {
                    rung: best,
                    limit: Some(decision),
                };
            }
            best = Some(index);
        }
        Ceiling {
            rung: best,
            limit: None,
        }
    }

    /// Every dial range where a data transmission occupying `audio` is lawful for this
    /// station: the data segments inside the operator's privileges — for an automatically
    /// controlled station, the §97.221(b) segments, or wherever §97.221(c) allows an answer
    /// that narrow — and the 60 m channels it fits.
    #[must_use]
    pub fn safe_dials(&self, s: &Situation, audio: Edges, direction: Direction) -> Vec<SafeDial> {
        let Self::Rules(profile) = self else {
            return Vec::new();
        };
        let (Some(sideband), Some(license), Some(control)) = (s.sideband, s.license, s.control)
        else {
            return Vec::new();
        };
        let (a_lo, a_hi) = audio.range(profile.bandwidth.reading);
        let width = a_hi - a_lo;
        let automatic = control == ControlMode::Automatic && direction != Direction::Operator;
        let elsewhere_ok = profile.automatic.elsewhere.as_ref().is_some_and(|e| {
            width <= e.max_bandwidth_hz && (direction == Direction::Respond || !e.requires_response)
        });
        let privileges = privileges_of(profile, license);
        let mut out = Vec::new();
        for segment in &profile.data_segments {
            if width > segment.data_limit(&profile.data).0 {
                continue;
            }
            let excluded = automatic
                && profile
                    .automatic
                    .excluded_bands
                    .iter()
                    .any(|x| x.band == segment.band);
            if excluded {
                continue;
            }
            for privilege in privileges.iter().filter(|p| p.allows("data")) {
                let Some(inter) = intersect(segment.range, privilege.range) else {
                    continue;
                };
                let mut pieces = vec![(inter, segment.rule.clone())];
                if automatic && !elsewhere_ok {
                    pieces = profile
                        .automatic
                        .segments
                        .iter()
                        .filter_map(|auto| intersect(inter, *auto))
                        .map(|r| (r, profile.automatic.segments_rule.clone()))
                        .collect();
                }
                for (range, rule) in pieces {
                    if let Some((low, high)) = dial_range(range, sideband, a_lo, a_hi, s.margin_hz)
                    {
                        out.push(SafeDial {
                            band: segment.band.clone(),
                            segment: range,
                            rule,
                            dial_low_hz: low,
                            dial_high_hz: high,
                        });
                    }
                }
            }
        }
        if let Some(channels) = &profile.channels
            && !automatic
            && width <= profile.data.max_bandwidth_hz
            && sideband == Sideband::Usb
            && privileges.iter().any(|p| {
                channels
                    .list
                    .iter()
                    .any(|c| p.range.holds(c.center_hz, c.center_hz) && p.allows("data"))
            })
        {
            for channel in &channels.list {
                let dial = channel.center_hz - channels.carrier_offset_hz;
                let range = channel_range(channels, *channel);
                let (lo, hi) = rf_range(dial, sideband, a_lo, a_hi);
                if range.holds(lo - s.margin_hz, hi + s.margin_hz) {
                    out.push(SafeDial {
                        band: channels.band.clone(),
                        segment: range,
                        rule: channels.rule.clone(),
                        dial_low_hz: dial,
                        dial_high_hz: dial,
                    });
                }
            }
        }
        out.sort_by(|a, b| a.dial_low_hz.total_cmp(&b.dial_low_hz));
        out
    }
}

/// The overlap of two ranges, if they have one.
fn intersect(a: Range, b: Range) -> Option<Range> {
    let low = a.low_hz.max(b.low_hz);
    let high = a.high_hz.min(b.high_hz);
    (low < high).then_some(Range {
        low_hz: low,
        high_hz: high,
    })
}

fn privileges_of(profile: &Profile, license: LicenseClass) -> &[profile::Privilege] {
    let p = &profile.privileges;
    match license {
        LicenseClass::Extra => &p.extra,
        LicenseClass::Advanced => &p.advanced,
        LicenseClass::General => &p.general,
        LicenseClass::Technician => &p.technician,
        LicenseClass::Novice => &p.novice,
    }
}

fn channel_range(channels: &profile::Channels, channel: profile::Channel) -> Range {
    Range {
        low_hz: channel.center_hz - channels.width_hz / 2.0,
        high_hz: channel.center_hz + channels.width_hz / 2.0,
    }
}

/// A decision being made: the facts, before the verdict.
struct Draft {
    what: String,
    kind: EmissionKind,
    direction: Direction,
    s: Situation,
    audio: (f64, f64),
    rf: Option<(f64, f64)>,
    band: Option<String>,
    segment: Option<(Range, String)>,
    automatic_segment: Option<Range>,
    notes: Vec<String>,
}

impl Draft {
    fn new(s: &Situation, tx: &Transmission, reading: Reading) -> Self {
        let audio = tx.audio.range(reading);
        let rf = match (s.dial_hz, s.sideband) {
            (Some(dial), Some(sideband)) => Some(rf_range(dial, sideband, audio.0, audio.1)),
            _ => None,
        };
        Self {
            what: tx.what.clone(),
            kind: tx.kind,
            direction: tx.direction,
            s: s.clone(),
            audio,
            rf,
            band: None,
            segment: None,
            automatic_segment: None,
            notes: Vec::new(),
        }
    }

    fn width(&self) -> f64 {
        self.audio.1 - self.audio.0
    }

    fn finish(
        self,
        verdict: Verdict,
        code: &'static str,
        rule: &str,
        summary: &str,
        detail: &str,
    ) -> Decision {
        Decision {
            verdict,
            code,
            rule: rule.to_owned(),
            summary: summary.to_owned(),
            detail: detail.to_owned(),
            what: self.what,
            kind: self.kind,
            direction: self.direction,
            dial_hz: self.s.dial_hz,
            dial_source: self.s.dial_source,
            sideband: self.s.sideband,
            control: self.s.control,
            license: self.s.license,
            audio_low_hz: self.audio.0,
            audio_high_hz: self.audio.1,
            rf_low_hz: self.rf.map(|r| r.0),
            rf_high_hz: self.rf.map(|r| r.1),
            bandwidth_hz: self.audio.1 - self.audio.0,
            margin_hz: self.s.margin_hz,
            band: self.band,
            segment: self.segment.as_ref().map(|s| s.0),
            segment_rule: self.segment.map(|s| s.1),
            automatic_segment: self.automatic_segment,
            guidance: None,
            notes: self.notes,
        }
    }

    fn blocked(self, code: &'static str, rule: &str, summary: &str, detail: &str) -> Decision {
        self.finish(Verdict::Blocked, code, rule, summary, detail)
    }

    fn legal(self, code: &'static str, rule: &str, summary: &str, detail: &str) -> Decision {
        self.finish(Verdict::Legal, code, rule, summary, detail)
    }
}

/// "3 585.000 kHz"-style frequency for a sentence: kilohertz with three decimals.
fn at(hz: f64) -> String {
    format!("{:.3} kHz", hz / 1000.0)
}

/// The facts every judgement works from, once `evaluate` has them all.
struct Case<'a> {
    profile: &'a Profile,
    s: &'a Situation,
    tx: &'a Transmission,
    control: ControlMode,
    license: LicenseClass,
    /// The dial, hertz.
    dial: f64,
    /// The RF edges of the transmission, hertz.
    lo: f64,
    hi: f64,
    /// The same edges widened by the margin: what has to fit.
    glo: f64,
    ghi: f64,
    /// Whether §97.221 applies: an automatically controlled station, not an operator's test.
    automatic: bool,
    /// "… occupies … on the air with the dial at …": where every reasoning starts.
    rf_text: String,
}

/// A refusal before any rule is reached: something the question needs is not known.
struct Missing {
    code: &'static str,
    rule: &'static str,
    summary: &'static str,
    detail: String,
}

/// What the question needs, first: nothing is guessed.
fn prerequisites<'a>(
    profile: &'a Profile,
    s: &'a Situation,
    tx: &'a Transmission,
    d: &Draft,
) -> Result<Case<'a>, Missing> {
    if s.itu_region != profile.itu_region {
        return Err(Missing {
            code: "region",
            rule: "§97.301",
            summary: "TX BLOCKED: this profile does not cover your ITU region",
            detail: format!(
                "The {} tables cover ITU Region {} only; this station is set to Region {}.",
                profile.name, profile.itu_region, s.itu_region
            ),
        });
    }
    let Some(control) = s.control else {
        return Err(Missing {
            code: "no_control",
            rule: "§97.109",
            summary: "TX BLOCKED: say how the station is controlled (Setup, step 1)",
            detail: "Local, remote or automatic control decides which rules apply (§97.109); \
                     Aether does not guess it."
                .to_owned(),
        });
    };
    let Some(license) = s.license else {
        return Err(Missing {
            code: "no_license",
            rule: "§97.301",
            summary: "TX BLOCKED: set your license class (Setup, step 1)",
            detail: "The frequencies a station may use depend on the control operator's license \
                     class (§97.301); Aether does not assume one."
                .to_owned(),
        });
    };
    let Some(sideband) = s.sideband else {
        return Err(Missing {
            code: "no_sideband",
            rule: "",
            summary: "TX BLOCKED: set the sideband the radio transmits on",
            detail: "Where the signal lands depends on whether the radio sends the modem's audio \
                     on the upper or the lower sideband."
                .to_owned(),
        });
    };
    let (Some(dial), Some((lo, hi))) = (s.dial_hz, d.rf) else {
        return Err(Missing {
            code: "no_dial",
            rule: "",
            summary: "TX BLOCKED: the dial frequency is not known",
            detail: "Aether needs the radio's dial frequency to know where the signal lands. CAT \
                     or rigctld keying reads it from the radio; with serial-line keying, enter it \
                     on the Session tab (Dial)."
                .to_owned(),
        });
    };
    let rf_text = format!(
        "{} occupies {:.0} Hz of audio ({:.0}–{:.0} Hz), so {}–{} on the air with the dial at {} ({})",
        d.what,
        d.width(),
        d.audio.0,
        d.audio.1,
        at(lo),
        at(hi),
        at(dial),
        sideband.name().to_uppercase(),
    );
    Ok(Case {
        profile,
        s,
        tx,
        control,
        license,
        dial,
        lo,
        hi,
        glo: lo - s.margin_hz,
        ghi: hi + s.margin_hz,
        automatic: control == ControlMode::Automatic && tx.direction != Direction::Operator,
        rf_text,
    })
}

/// Judge a transmission against an administration's rules.
fn evaluate(profile: &Profile, s: &Situation, tx: &Transmission, mut d: Draft) -> Decision {
    let c = match prerequisites(profile, s, tx, &d) {
        Ok(c) => c,
        Err(missing) => {
            return d.blocked(missing.code, missing.rule, missing.summary, &missing.detail);
        }
    };
    let Some(band) = profile.band_holding(c.glo, c.ghi) else {
        let detail = match profile.band_touching(c.glo, c.ghi) {
            Some(band) => format!(
                "{}: it crosses the edge of the {} band ({}), counting {:.0} Hz of margin.",
                c.rf_text,
                band.name,
                band.range.khz(),
                s.margin_hz
            ),
            None => format!(
                "{}: outside the amateur bands this profile covers.",
                c.rf_text
            ),
        };
        return d.blocked(
            "out_of_band",
            "§97.301",
            "TX BLOCKED: the signal is outside the amateur bands",
            &detail,
        );
    };
    d.band = Some(band.name.clone());
    if c.automatic
        && let Some(x) = profile
            .automatic
            .excluded_bands
            .iter()
            .find(|x| x.band == band.name)
    {
        let summary = format!("TX BLOCKED: no automatic operation on {}", band.name);
        return d.blocked("automatic_excluded", &x.rule, &summary, &x.why);
    }
    // 60 m: channels and a segment, its own emissions
    if let Some(channels) = profile.channels.as_ref().filter(|ch| ch.band == band.name) {
        return sixty(&c, channels, d);
    }
    match tx.kind {
        EmissionKind::Data => judge_data(&c, &band.name, d),
        EmissionKind::Cw | EmissionKind::Test | EmissionKind::Nothing => judge_other(&c, d),
    }
}

/// A data emission off 60 m: no wider than 2.8 kHz, inside a data segment and the operator's
/// privileges, and — for an automatically controlled station — where §97.221 allows.
fn judge_data(c: &Case, band: &str, mut d: Draft) -> Decision {
    let profile = c.profile;
    let rf_text = &c.rf_text;
    let width = d.width();
    let segment = profile
        .data_segments
        .iter()
        .find(|seg| seg.range.holds(c.glo, c.ghi));
    // the segment's own limit where it has one (6 m: §97.307(f)(2), (5)), the profile's
    // general one elsewhere — and outside every segment, the general one decides the wording
    let (limit, limit_rule) = segment.map_or(
        (profile.data.max_bandwidth_hz, profile.data.rule.as_str()),
        |seg| seg.data_limit(&profile.data),
    );
    if width > limit {
        let detail = format!(
            "{rf_text}: {width:.0} Hz is wider than the {limit:.0} Hz a RTTY or data emission may occupy."
        );
        let summary = format!(
            "TX BLOCKED: the signal is wider than {:.1} kHz",
            limit / 1000.0
        );
        let rule = limit_rule.to_owned();
        return d.blocked("too_wide", &rule, &summary, &detail);
    }
    let Some(segment) = segment else {
        return outside_segment(profile, d, c.lo, c.hi, c.s.margin_hz, rf_text);
    };
    d.segment = Some((segment.range, segment.rule.clone()));
    let privileges = privileges_of(profile, c.license);
    if let Some((rule, summary, detail)) =
        privilege_refusal(privileges, c.license, c.glo, c.ghi, "data", rf_text)
    {
        return d.blocked("privilege", rule, summary, &detail);
    }
    if c.automatic {
        return automatic_data(profile, c.s, c.tx, d, (c.glo, c.ghi), rf_text, segment);
    }
    let control_word = match c.control {
        ControlMode::Local => "local control",
        ControlMode::Remote => "remote control",
        ControlMode::Automatic => "an operator at the control point",
    };
    let summary = format!("FCC: data permitted — {width:.0} Hz fits the {band} data segment");
    let detail = format!(
        "{rf_text}: inside the {band} data segment {} ({}), within {} privileges, under \
         {control_word}.",
        segment.range.khz(),
        segment.rule,
        c.license.title()
    );
    let decision = d.legal("permitted", &segment.rule, &summary, &detail);
    advise(profile, c.s, c.license, decision, (c.lo, c.hi))
}

/// A Morse identification, a tune tone, or a keying with no audio: wherever the operator's
/// privileges allow that emission (§97.305(a), (b)).
fn judge_other(c: &Case, d: Draft) -> Decision {
    let kind = c.tx.kind;
    let privileges = privileges_of(c.profile, c.license);
    if kind == EmissionKind::Nothing {
        if !privileges.iter().any(|p| p.range.holds(c.dial, c.dial)) {
            let detail = format!(
                "The dial, {}, is outside the {} privileges: the transmitter is not keyed there, \
                 even with no audio.",
                at(c.dial),
                c.license.title()
            );
            return d.blocked(
                "privilege",
                &c.profile.privileges.rule,
                "TX BLOCKED: the dial is outside your privileges",
                &detail,
            );
        }
    } else {
        let emission = if kind == EmissionKind::Cw {
            "cw"
        } else {
            "test"
        };
        if let Some((rule, summary, detail)) =
            privilege_refusal(privileges, c.license, c.glo, c.ghi, emission, &c.rf_text)
        {
            return d.blocked("privilege", rule, summary, &detail);
        }
    }
    let (rule, summary) = match kind {
        EmissionKind::Cw => (
            "§97.305(a)",
            "FCC: CW permitted anywhere in your privileges",
        ),
        EmissionKind::Test => (
            "§97.305(b)",
            "FCC: a brief test emission, permitted in your privileges",
        ),
        _ => ("§97.301", "FCC: keying only — no emission"),
    };
    let detail = format!(
        "{}: within {} privileges ({rule}).",
        c.rf_text,
        c.license.title()
    );
    d.legal("permitted", rule, summary, &detail)
}

/// Data outside every data segment: say how far outside, or that there is none here.
fn outside_segment(
    profile: &Profile,
    d: Draft,
    lo: f64,
    hi: f64,
    m: f64,
    rf_text: &str,
) -> Decision {
    let near = profile
        .data_segments
        .iter()
        .filter(|seg| seg.range.overlaps(lo - m, hi + m))
        .min_by(|a, b| {
            let dist = |seg: &&profile::Segment| {
                (seg.range.low_hz - lo).max(0.0) + (hi - seg.range.high_hz).max(0.0)
            };
            dist(a).total_cmp(&dist(b))
        });
    if let Some(seg) = near {
        let below = (seg.range.low_hz - lo).max(0.0);
        let above = (hi - seg.range.high_hz).max(0.0);
        let (by, edge) = if above >= below {
            (above, "upper")
        } else {
            (below, "lower")
        };
        let (summary, detail) = if by > 0.0 {
            (
                format!("TX BLOCKED: signal extends {by:.0} Hz beyond the FCC data segment"),
                format!(
                    "{rf_text}: {by:.0} Hz beyond the {edge} edge of the {} data segment {} ({}).",
                    seg.band,
                    seg.range.khz(),
                    seg.rule
                ),
            )
        } else {
            (
                "TX BLOCKED: signal too close to the edge of the FCC data segment".to_owned(),
                format!(
                    "{rf_text}: inside the {} data segment {} but closer than the {m:.0} Hz margin \
                     to its {edge} edge ({}).",
                    seg.band,
                    seg.range.khz(),
                    seg.rule
                ),
            )
        };
        let mut d = d;
        d.segment = Some((seg.range, seg.rule.clone()));
        return d.blocked("outside_data_segment", &seg.rule, &summary, &detail);
    }
    let detail = format!(
        "{rf_text}: RTTY and data emissions are not authorized there (§97.305(c)); the data \
         segments are the lower parts of the bands."
    );
    d.blocked(
        "no_data_here",
        "§97.305(c)",
        "TX BLOCKED: data is not authorized on this frequency",
        &detail,
    )
}

/// The operator's privileges for an emission over `glo..=ghi`: the rule, the summary and the
/// reasoning when they do not reach.
fn privilege_refusal(
    privileges: &[profile::Privilege],
    license: LicenseClass,
    glo: f64,
    ghi: f64,
    emission: &str,
    rf_text: &str,
) -> Option<(&'static str, &'static str, String)> {
    let holding = privileges.iter().find(|p| p.range.holds(glo, ghi));
    match holding {
        Some(p) if p.allows(emission) => None,
        Some(p) => Some((
            "§97.307(f)(9)",
            "TX BLOCKED: your license class may send only CW here",
            format!(
                "{rf_text}: {} privileges there allow {} only ({}).",
                license.title(),
                p.emissions
                    .as_deref()
                    .unwrap_or_default()
                    .join(", ")
                    .to_uppercase(),
                p.rule.as_deref().unwrap_or("§97.301")
            ),
        )),
        None => Some((
            "§97.301",
            "TX BLOCKED: outside your license class's privileges",
            format!(
                "{rf_text}: outside the {} privileges (§97.301).",
                license.title()
            ),
        )),
    }
}

/// Data from an automatically controlled station (§97.221).
fn automatic_data(
    profile: &Profile,
    s: &Situation,
    tx: &Transmission,
    mut d: Draft,
    (glo, ghi): (f64, f64),
    rf_text: &str,
    segment: &profile::Segment,
) -> Decision {
    let width = d.width();
    if let Some(auto) = profile
        .automatic
        .segments
        .iter()
        .find(|a| a.holds(glo, ghi))
    {
        d.automatic_segment = Some(*auto);
        let summary = format!(
            "FCC: automatic control — permitted inside the §97.221(b) segment {}",
            auto.khz()
        );
        let detail = format!(
            "{rf_text}: inside the automatic-control segment {} ({}), within the {} data \
             segment ({}).",
            auto.khz(),
            profile.automatic.segments_rule,
            segment.band,
            segment.rule
        );
        let decision = d.legal(
            "automatic_segment",
            &profile.automatic.segments_rule,
            &summary,
            &detail,
        );
        return advise(
            profile,
            s,
            s.license.unwrap_or(LicenseClass::Extra),
            decision,
            (glo + s.margin_hz, ghi - s.margin_hz),
        );
    }
    let Some(elsewhere) = &profile.automatic.elsewhere else {
        let detail = format!(
            "{rf_text}: automatic control is allowed only inside the {} segments.",
            profile.automatic.segments_rule
        );
        return d.blocked(
            "automatic_outside",
            &profile.automatic.segments_rule,
            "TX BLOCKED: automatic station outside the automatic-control segments",
            &detail,
        );
    };
    let responding = tx.direction == Direction::Respond;
    if width > elsewhere.max_bandwidth_hz {
        let summary = format!(
            "TX BLOCKED: automatic station using a {width:.0} Hz emission outside the §97.221(b) \
             segments"
        );
        let detail = format!(
            "{rf_text}: outside the automatic-control segments ({}), an automatically \
             controlled station may transmit only {:.0} Hz or less ({}), and {} measures {width:.0} Hz.",
            profile.automatic.segments_rule, elsewhere.max_bandwidth_hz, elsewhere.rule, d.what
        );
        return d.blocked("automatic_bandwidth", &elsewhere.rule, &summary, &detail);
    }
    if elsewhere.requires_response && !responding {
        let detail = format!(
            "{rf_text}: outside the automatic-control segments ({}), an automatically \
             controlled station may transmit only while responding to interrogation by a \
             station under local or remote control ({}); this transmission would start an \
             exchange.",
            profile.automatic.segments_rule, elsewhere.rule
        );
        return d.blocked(
            "automatic_originate",
            &elsewhere.rule,
            &format!(
                "TX BLOCKED: {width:.0} Hz automatic transmission outside §97.221(b) may only respond to a locally or remotely controlled station"
            ),
            &detail,
        );
    }
    let summary = format!(
        "FCC: automatic control — answering at {width:.0} Hz, within §97.221(c)'s {:.0} Hz",
        elsewhere.max_bandwidth_hz
    );
    let detail = format!(
        "{rf_text}: outside the automatic-control segments, responding to interrogation with \
         {width:.0} Hz, no more than {:.0} Hz ({}).",
        elsewhere.max_bandwidth_hz, elsewhere.rule
    );
    let decision = d.legal("automatic_response", &elsewhere.rule, &summary, &detail);
    advise(
        profile,
        s,
        s.license.unwrap_or(LicenseClass::Extra),
        decision,
        (glo + s.margin_hz, ghi - s.margin_hz),
    )
}

/// 60 m (§97.303(h)(3), §97.305(c)(3)(iii), §97.307(f)(14)): the operator's privileges and the
/// band's own emissions, then a channel or the segment.
fn sixty(c: &Case, channels: &profile::Channels, d: Draft) -> Decision {
    let (profile, rf_text) = (c.profile, &c.rf_text);
    if !privileges_of(profile, c.license)
        .iter()
        .any(|p| p.range.holds(c.lo, c.hi))
    {
        let detail = format!(
            "{rf_text}: outside the {} privileges (§97.301).",
            c.license.title()
        );
        return d.blocked(
            "privilege",
            "§97.301",
            "TX BLOCKED: outside your license class's privileges",
            &detail,
        );
    }
    if c.tx.kind == EmissionKind::Nothing {
        return d.legal(
            "permitted",
            "§97.301",
            "FCC: keying only — no emission",
            rf_text,
        );
    }
    if let Some(allowed) = &profile.sixty_meter_emissions
        && !allowed.allowed.iter().any(|e| e == c.tx.kind.name())
    {
        let detail = format!(
            "{rf_text}: on 60 m a station may transmit only {} ({}); tune up on another band.",
            allowed.allowed.join(", ").to_uppercase(),
            allowed.rule
        );
        return d.blocked(
            "sixty_emission",
            &allowed.rule,
            "TX BLOCKED: this emission is not allowed on 60 m",
            &detail,
        );
    }
    let m = c.s.margin_hz;
    if let Some(channel) = channels
        .list
        .iter()
        .find(|ch| channel_range(channels, **ch).overlaps(c.lo - m, c.hi + m))
    {
        return sixty_channel(c, channels, *channel, d);
    }
    if let Some(segment) = profile
        .data_segments
        .iter()
        .find(|seg| seg.band == channels.band && seg.range.holds(c.glo, c.ghi))
    {
        return sixty_segment(c, segment, d);
    }
    let detail = format!(
        "{rf_text}: on 60 m a station transmits only on the four channels or inside \
         5351.5–5366.5 kHz ({}).",
        channels.rule
    );
    d.blocked(
        "sixty_off_channel",
        &channels.rule,
        "TX BLOCKED: not on a 60 m channel or in the 60 m segment",
        &detail,
    )
}

/// On a 60 m channel: a CW carrier on its centre; a data emission on the channel's sideband,
/// its carrier (the dial) where §97.303(h)(3) puts it, and all of it inside the channel.
fn sixty_channel(
    c: &Case,
    channels: &profile::Channels,
    channel: profile::Channel,
    mut d: Draft,
) -> Decision {
    let rf_text = &c.rf_text;
    let range = channel_range(channels, channel);
    d.segment = Some((range, channels.rule.clone()));
    let (ok, why) = if c.tx.kind == EmissionKind::Cw {
        let centre = f64::midpoint(c.lo, c.hi);
        (
            (centre - channel.center_hz).abs() <= channels.carrier_tolerance_hz,
            format!(
                "a CW emission's carrier goes on the centre frequency, {}",
                at(channel.center_hz)
            ),
        )
    } else {
        let carrier = channel.center_hz - channels.carrier_offset_hz;
        let sideband_ok =
            c.s.sideband
                .is_some_and(|sb| sb.name() == channels.sideband);
        (
            sideband_ok
                && (c.dial - carrier).abs() <= channels.carrier_tolerance_hz
                && range.holds(c.glo, c.ghi),
            format!(
                "a data emission goes on {} with its carrier (the dial) at {} and all of it \
                 inside the channel {}",
                channels.sideband.to_uppercase(),
                at(carrier),
                range.khz()
            ),
        )
    };
    if !ok {
        let detail = format!(
            "{rf_text}: on the 60 m channel centred on {}, {why} ({}).",
            at(channel.center_hz),
            channels.rule
        );
        return d.blocked(
            "sixty_channel",
            &channels.rule,
            "TX BLOCKED: not set up for the 60 m channel",
            &detail,
        );
    }
    let over = c.s.power_w.filter(|&p| p > 100.0);
    if let Some(power) = over {
        d.notes.push(format!(
            "At {power:.0} W: 60 m channels are limited to 100 W ERP (§97.313(i)); the \
             antenna's gain counts."
        ));
    }
    let detail = format!(
        "{rf_text}: on the 60 m channel {} ({}). {}",
        range.khz(),
        channels.rule,
        channels.why
    );
    let mut decision = d.legal(
        "sixty_channel",
        &channels.rule,
        "FCC: 60 m channel — permitted on the channel",
        &detail,
    );
    decision.notes.push(
        "60 m is shared with government stations: keep transmissions short \
         (§97.307(f)(14)(ii)); only one signal at a time on a channel (band plan)."
            .to_owned(),
    );
    if over.is_some() {
        decision.verdict = Verdict::Warning;
    }
    decision
}

/// Inside the 60 m segment (§97.303(h)(3)): no wider than 2.8 kHz, 9.15 W ERP.
fn sixty_segment(c: &Case, segment: &profile::Segment, mut d: Draft) -> Decision {
    let profile = c.profile;
    let rf_text = &c.rf_text;
    d.segment = Some((segment.range, segment.rule.clone()));
    if c.tx.kind == EmissionKind::Data && d.width() > profile.data.max_bandwidth_hz {
        let detail = format!("{rf_text}: wider than 2.8 kHz ({}).", profile.data.rule);
        return d.blocked(
            "too_wide",
            &profile.data.rule,
            "TX BLOCKED: the signal is wider than 2.8 kHz",
            &detail,
        );
    }
    let detail = format!(
        "{rf_text}: inside the 60 m segment {} ({}). Radiated power there is limited to 9.15 W \
         ERP (§97.313(i)).",
        segment.range.khz(),
        segment.rule
    );
    let mut decision = d.legal(
        "sixty_segment",
        &segment.rule,
        "FCC: 60 m segment — permitted (9.15 W ERP limit)",
        &detail,
    );
    if let Some(power) = c.s.power_w.filter(|&p| p > 9.15) {
        decision.verdict = Verdict::Warning;
        decision.notes.push(format!(
            "At {power:.0} W: the 5351.5–5366.5 kHz segment is limited to 9.15 W ERP (§97.313(i))."
        ));
    }
    decision
}

/// A lawful data transmission, with what the operator should know: the voluntary band plan
/// and any power limit that applies.
fn advise(
    profile: &Profile,
    s: &Situation,
    license: LicenseClass,
    mut decision: Decision,
    (lo, hi): (f64, f64),
) -> Decision {
    for limit in &profile.power {
        let applies = limit.range.overlaps(lo, hi)
            && limit
                .classes
                .as_ref()
                .is_none_or(|c| c.iter().any(|x| x == license.name()));
        if applies && let Some(power) = s.power_w.filter(|&p| p > limit.max_w) {
            decision.verdict = Verdict::Warning;
            decision.notes.push(format!(
                "At {power:.0} W: {} is limited to {} W {} here ({}).",
                if limit.classes.is_some() {
                    "a Novice or Technician"
                } else {
                    "a station"
                },
                limit.max_w,
                limit.measure,
                limit.rule
            ));
        }
    }
    if s.band_plan
        && let Some(plan) = &profile.band_plan
    {
        if let Some(avoid) = plan.avoid.iter().find(|a| a.range.overlaps(lo, hi)) {
            decision.verdict = Verdict::Warning;
            decision.guidance = Some(format!(
                "Legal under Part 97, but on {} ({}, voluntary).",
                avoid.label, plan.authority
            ));
        } else if let Some(area) = plan.data.iter().find(|a| a.range.holds(lo, hi)) {
            decision.guidance = Some(format!(
                "Inside the band plan's {} ({}).",
                area.label, plan.authority
            ));
        } else {
            decision.verdict = Verdict::Warning;
            "FCC warning: legal under Part 97, but outside the customary digital band plan"
                .clone_into(&mut decision.summary);
            decision.guidance = Some(format!(
                "Legal under Part 97; the {} puts data elsewhere on this band. It is voluntary.",
                plan.authority
            ));
        }
    }
    decision
}

#[cfg(test)]
mod tests;
