//! A regulatory profile: one administration's rules for the bands Aether uses, as data.
//!
//! The United States profile (`data/regulatory/us-fcc-part97.json`) was read from the e-CFR
//! text of 47 CFR Part 97; every range carries the rule it comes from. Another country's
//! rules are another file of the same shape and an entry in [`load`]. A profile that does not
//! parse or does not hold together is not used: the policy refuses to transmit and says why.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::occupancy::Reading;

/// A frequency range, hertz, both ends inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Range {
    /// Lower edge.
    pub low_hz: f64,
    /// Upper edge.
    pub high_hz: f64,
}

impl Range {
    /// Whether `low..=high` lies inside.
    #[must_use]
    pub fn holds(&self, low: f64, high: f64) -> bool {
        low >= self.low_hz && high <= self.high_hz
    }

    /// Whether `low..=high` touches it at all.
    #[must_use]
    pub fn overlaps(&self, low: f64, high: f64) -> bool {
        low <= self.high_hz && high >= self.low_hz
    }

    /// The range in kilohertz, for a sentence.
    #[must_use]
    pub fn khz(&self) -> String {
        format!("{}–{} kHz", khz(self.low_hz), khz(self.high_hz))
    }
}

/// Hertz as kilohertz with no more decimals than it needs: 7125000 → "7125", 14099500 →
/// "14099.5".
#[must_use]
pub fn khz(hz: f64) -> String {
    let k = hz / 1000.0;
    if (k - k.round()).abs() < 1e-6 {
        format!("{k:.0}")
    } else if (k * 10.0 - (k * 10.0).round()).abs() < 1e-6 {
        format!("{k:.1}")
    } else {
        format!("{k:.3}")
    }
}

/// A band, for naming a frequency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Band {
    /// "40 m".
    pub name: String,
    /// Its edges.
    #[serde(flatten)]
    pub range: Range,
}

/// A segment where RTTY and data emissions are authorized (§97.305(c)).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    /// The band it is in.
    pub band: String,
    /// Its edges.
    #[serde(flatten)]
    pub range: Range,
    /// The rule it comes from.
    pub rule: String,
    /// The widest RTTY or data emission allowed here, when it is not the profile's general
    /// limit ([`DataLimit`]): on 6 m, §97.307(f)(2) and (5) rather than the HF rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bandwidth_hz: Option<f64>,
    /// The rule that limit comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bandwidth_rule: Option<String>,
}

impl Segment {
    /// The widest RTTY or data emission allowed in this segment, and the rule for it: its
    /// own when it has one, the profile's general limit otherwise.
    #[must_use]
    pub fn data_limit<'a>(&'a self, general: &'a DataLimit) -> (f64, &'a str) {
        match self.max_bandwidth_hz {
            Some(max) => (max, self.bandwidth_rule.as_deref().unwrap_or(&general.rule)),
            None => (general.max_bandwidth_hz, &general.rule),
        }
    }
}

/// A privilege of a license class (§97.301), with the emissions it allows when they are
/// fewer than the segments' (a Technician's CW-only HF segments, §97.307(f)(9)).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Privilege {
    /// Its edges.
    #[serde(flatten)]
    pub range: Range,
    /// The emissions allowed, when restricted; every emission the segments allow otherwise.
    #[serde(default)]
    pub emissions: Option<Vec<String>>,
    /// The rule that restricts them.
    #[serde(default)]
    pub rule: Option<String>,
}

impl Privilege {
    /// Whether this privilege allows an emission of this name.
    #[must_use]
    pub fn allows(&self, emission: &str) -> bool {
        self.emissions
            .as_ref()
            .is_none_or(|list| list.iter().any(|e| e == emission))
    }
}

/// The privileges, per class.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Privileges {
    /// Amateur Extra.
    pub extra: Vec<Privilege>,
    /// Advanced.
    pub advanced: Vec<Privilege>,
    /// General.
    pub general: Vec<Privilege>,
    /// Technician.
    pub technician: Vec<Privilege>,
    /// Novice.
    pub novice: Vec<Privilege>,
    /// The rule the table comes from.
    pub rule: String,
}

/// The 60 m discrete channels (§97.303(h)(3)).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Channels {
    /// The band they are in.
    pub band: String,
    /// The rule.
    pub rule: String,
    /// A channel's width, hertz, centred on its centre frequency.
    pub width_hz: f64,
    /// How far below the centre a data emission's carrier is set.
    pub carrier_offset_hz: f64,
    /// How far from the prescribed carrier a dial may be.
    pub carrier_tolerance_hz: f64,
    /// The sideband a data emission is sent on.
    pub sideband: String,
    /// The channels.
    pub list: Vec<Channel>,
    /// Why they work as they do, for the explanation.
    pub why: String,
}

/// One channel.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Channel {
    /// Its centre frequency, hertz.
    pub center_hz: f64,
}

/// The emissions allowed on 60 m (§97.307(f)(14)(i)).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SixtyEmissions {
    /// Their names.
    pub allowed: Vec<String>,
    /// The rule.
    pub rule: String,
}

/// A band where automatic control is not allowed at all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Excluded {
    /// The band.
    pub band: String,
    /// The rule.
    pub rule: String,
    /// Why, for the explanation.
    pub why: String,
}

/// Automatic control outside the designated segments (§97.221(c)).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Elsewhere {
    /// The most an automatically controlled station's transmission may occupy there.
    pub max_bandwidth_hz: f64,
    /// Whether it may only respond to interrogation by a station under local or remote
    /// control.
    pub requires_response: bool,
    /// The rule.
    pub rule: String,
}

/// Automatic control (§97.221).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Automatic {
    /// The segments where it is allowed for RTTY and data (§97.221(b)).
    pub segments: Vec<Range>,
    /// The rule for them.
    pub segments_rule: String,
    /// The conditions elsewhere, when it is allowed elsewhere at all.
    pub elsewhere: Option<Elsewhere>,
    /// Bands where it is not allowed.
    #[serde(default)]
    pub excluded_bands: Vec<Excluded>,
}

/// A power limit, for a reminder: Aether cannot measure power, and says what applies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PowerLimit {
    /// Where it applies.
    #[serde(flatten)]
    pub range: Range,
    /// The limit, watts.
    pub max_w: f64,
    /// PEP or ERP.
    pub measure: String,
    /// The classes it applies to; every class when absent.
    #[serde(default)]
    pub classes: Option<Vec<String>>,
    /// The rule.
    pub rule: String,
}

/// A voluntary band-plan area.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanArea {
    /// Its edges.
    #[serde(flatten)]
    pub range: Range,
    /// What it is.
    pub label: String,
}

/// The voluntary band plan: guidance, never a rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BandPlan {
    /// Whose plan.
    pub authority: String,
    /// Where it is published.
    pub source: String,
    /// When it was read.
    pub as_of: String,
    /// The areas for data.
    pub data: Vec<PlanArea>,
    /// Areas to stay out of.
    #[serde(default)]
    pub avoid: Vec<PlanArea>,
}

/// How a profile reads "bandwidth".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BandwidthRule {
    /// The definition.
    pub rule: String,
    /// Which reading applies.
    pub reading: Reading,
    /// Why, for the explanation.
    pub why: String,
}

/// A limit with its rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataLimit {
    /// The most a RTTY or data emission may occupy, hertz.
    pub max_bandwidth_hz: f64,
    /// The rule.
    pub rule: String,
}

/// The Morse identifier's limit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CwIdLimit {
    /// The fastest an automatic identifier may be sent, words per minute.
    pub max_wpm: f64,
    /// The rule.
    pub rule: String,
}

/// One administration's rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// Its identifier: `us-fcc-part97`.
    pub id: String,
    /// Its name.
    pub name: String,
    /// The country.
    pub country: String,
    /// The authority and the rules.
    pub authority: String,
    /// When the rules were read.
    pub rules_as_of: String,
    /// When they last changed.
    pub last_amended: String,
    /// Where they are published.
    pub source: String,
    /// The ITU region the tables are for.
    pub itu_region: u8,
    /// Notes on the table.
    #[serde(default)]
    pub notes: Vec<String>,
    /// How bandwidth is read.
    pub bandwidth: BandwidthRule,
    /// The RTTY and data limit.
    pub data: DataLimit,
    /// The Morse identifier's limit.
    pub cw_id: Option<CwIdLimit>,
    /// The bands.
    pub bands: Vec<Band>,
    /// Where RTTY and data are authorized.
    pub data_segments: Vec<Segment>,
    /// The 60 m channels.
    pub channels: Option<Channels>,
    /// The emissions allowed on 60 m.
    pub sixty_meter_emissions: Option<SixtyEmissions>,
    /// The privileges.
    pub privileges: Privileges,
    /// Automatic control.
    pub automatic: Automatic,
    /// Power limits, for reminders.
    #[serde(default)]
    pub power: Vec<PowerLimit>,
    /// The voluntary band plan.
    pub band_plan: Option<BandPlan>,
}

/// The profiles this version knows, by identifier.
pub const KNOWN: &[(&str, &str)] = &[("us-fcc-part97", "United States — FCC Part 97")];

/// Load a profile by identifier.
///
/// # Errors
/// If there is no such profile, or it does not parse or hold together.
pub fn load(id: &str) -> Result<Profile, String> {
    let text = match id {
        "us-fcc-part97" => include_str!("../../data/regulatory/us-fcc-part97.json"),
        other => return Err(format!("there is no regulatory profile called {other:?}")),
    };
    let profile: Profile = serde_json::from_str(text)
        .map_err(|e| format!("the {id} regulatory profile is malformed: {e}"))?;
    profile.check()?;
    Ok(profile)
}

impl Profile {
    /// A profile that does not hold together is not used.
    ///
    /// # Errors
    /// What is wrong with it.
    pub fn check(&self) -> Result<(), String> {
        let mut ranges: Vec<(&str, &Range)> = Vec::new();
        ranges.extend(self.bands.iter().map(|b| ("band", &b.range)));
        ranges.extend(
            self.data_segments
                .iter()
                .map(|s| ("data segment", &s.range)),
        );
        ranges.extend(
            self.automatic
                .segments
                .iter()
                .map(|r| ("automatic segment", r)),
        );
        for list in self.privileges.all().values() {
            ranges.extend(list.iter().map(|p| ("privilege", &p.range)));
        }
        for (what, range) in ranges {
            if !(range.low_hz.is_finite()
                && range.high_hz.is_finite()
                && range.low_hz < range.high_hz)
            {
                return Err(format!(
                    "the {} profile has a {what} of {}",
                    self.id,
                    range.khz()
                ));
            }
        }
        if self.bands.is_empty() || self.data_segments.is_empty() {
            return Err(format!(
                "the {} profile names no bands or no data segments",
                self.id
            ));
        }
        for segment in &self.data_segments {
            if !self
                .bands
                .iter()
                .any(|b| b.range.holds(segment.range.low_hz, segment.range.high_hz))
            {
                return Err(format!(
                    "the {} profile's data segment {} is in no band",
                    self.id,
                    segment.range.khz()
                ));
            }
        }
        if self
            .data_segments
            .iter()
            .filter_map(|s| s.max_bandwidth_hz)
            .any(|max| !max.is_finite() || max <= 0.0)
        {
            return Err(format!(
                "the {} profile has a data segment with no bandwidth limit",
                self.id
            ));
        }
        if !self.data.max_bandwidth_hz.is_finite() || self.data.max_bandwidth_hz <= 0.0 {
            return Err(format!(
                "the {} profile has no data bandwidth limit",
                self.id
            ));
        }
        Ok(())
    }

    /// The band holding `low..=high`, if one does.
    #[must_use]
    pub fn band_holding(&self, low: f64, high: f64) -> Option<&Band> {
        self.bands.iter().find(|b| b.range.holds(low, high))
    }

    /// A band `low..=high` touches, if any.
    #[must_use]
    pub fn band_touching(&self, low: f64, high: f64) -> Option<&Band> {
        self.bands.iter().find(|b| b.range.overlaps(low, high))
    }
}

impl Privileges {
    /// Every class's list, by name.
    #[must_use]
    pub fn all(&self) -> BTreeMap<&'static str, &[Privilege]> {
        BTreeMap::from([
            ("extra", self.extra.as_slice()),
            ("advanced", self.advanced.as_slice()),
            ("general", self.general.as_slice()),
            ("technician", self.technician.as_slice()),
            ("novice", self.novice.as_slice()),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_united_states_profile_loads_and_holds_together() {
        let us = load("us-fcc-part97").expect("loads");
        assert_eq!(us.itu_region, 2);
        assert!((us.data.max_bandwidth_hz - 2800.0).abs() < 1e-9);
        assert_eq!(us.data_segments.len(), 11);
        assert_eq!(us.automatic.segments.len(), 10);
        // 6 m: its own bandwidth rule, the HF bands the general one
        let six = us
            .data_segments
            .iter()
            .find(|s| s.band == "6 m")
            .expect("6 m");
        assert_eq!(six.data_limit(&us.data), (2800.0, "§97.307(f)(2), (5)"));
        let twenty = us
            .data_segments
            .iter()
            .find(|s| s.band == "20 m")
            .expect("20 m");
        assert_eq!(twenty.data_limit(&us.data), (2800.0, "§97.307(f)(3)"));
        assert!(load("uk-ofcom").is_err());
        assert_eq!(khz(7_125_000.0), "7125");
        assert_eq!(khz(14_099_500.0), "14099.5");
    }

    #[test]
    fn a_profile_that_does_not_hold_together_is_refused() {
        let mut us = load("us-fcc-part97").expect("loads");
        us.data_segments[0].range.high_hz = 1_000_000.0;
        assert!(us.check().is_err());
        let mut us = load("us-fcc-part97").expect("loads");
        us.data_segments[0].range.low_hz = 1_700_000.0;
        assert!(us.check().is_err(), "a segment outside every band");
        let mut us = load("us-fcc-part97").expect("loads");
        us.data.max_bandwidth_hz = 0.0;
        assert!(us.check().is_err());
        let mut us = load("us-fcc-part97").expect("loads");
        us.data_segments.last_mut().expect("6 m").max_bandwidth_hz = Some(f64::NAN);
        assert!(us.check().is_err(), "a segment limit that is no number");
    }
}
