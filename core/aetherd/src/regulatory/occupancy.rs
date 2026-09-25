//! What each Aether transmission occupies.
//!
//! `data/occupancy.json` is measured, not typed: `tools/make_occupancy.py` sends every rung
//! of both airs and both control frames through the model's transmit chain and reads the
//! averaged spectrum under both readings of 47 CFR §97.3(a)(8) (ADR-0018). The regulatory
//! policy turns these audio edges into RF edges with the dial and the sideband; nothing
//! compares a mode's *name* with a limit. The "500 Hz" air's OFDM rungs measure 560–710 Hz.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// Which reading of §97.3(a)(8) turns a spectrum into a bandwidth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reading {
    /// The band holding all but 1/398 of the power.
    Power,
    /// The band outside which the density stays 26 dB below the mean density within it.
    Spectral,
    /// The wider of the two — the conservative choice where the rule can be read either way.
    Wider,
}

/// The audio a transmission occupies, hertz, under both readings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Edges {
    /// All but 1/398 of the power: low and high edge.
    pub power: [f64; 2],
    /// Where the density stays 26 dB below the mean density within: low and high edge.
    pub spectral: [f64; 2],
}

impl Edges {
    /// A narrow emission — a keyed or steady tone — `half_width` either side of `centre`,
    /// the same under both readings.
    #[must_use]
    pub const fn tone(centre: f64, half_width: f64) -> Self {
        let edges = [centre - half_width, centre + half_width];
        Self {
            power: edges,
            spectral: edges,
        }
    }

    /// The same edges moved by `by` hertz: offsets from the audio centre made absolute.
    #[must_use]
    pub fn shifted(self, by: f64) -> Self {
        Self {
            power: [self.power[0] + by, self.power[1] + by],
            spectral: [self.spectral[0] + by, self.spectral[1] + by],
        }
    }

    /// The edges a reading takes.
    #[must_use]
    pub fn range(&self, reading: Reading) -> (f64, f64) {
        match reading {
            Reading::Power => (self.power[0], self.power[1]),
            Reading::Spectral => (self.spectral[0], self.spectral[1]),
            Reading::Wider => (
                self.power[0].min(self.spectral[0]),
                self.power[1].max(self.spectral[1]),
            ),
        }
    }

    /// The smallest edges that hold both: a burst of frames of more than one kind.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        Self {
            power: [
                self.power[0].min(other.power[0]),
                self.power[1].max(other.power[1]),
            ],
            spectral: [
                self.spectral[0].min(other.spectral[0]),
                self.spectral[1].max(other.spectral[1]),
            ],
        }
    }
}

/// Half the width the Morse identifier is allowed, hertz, either side of its tone: a 20 wpm
/// keying with 5 ms edges measures well inside it (`the_identifier_and_the_tune_tone_stay_
/// inside_their_bounds`).
pub const CW_HALF_WIDTH_HZ: f64 = 100.0;

/// Half the width the tune tone is allowed, either side of its frequency.
pub const TONE_HALF_WIDTH_HZ: f64 = 25.0;

/// One rung of an air's ladder.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RungOccupancy {
    /// The rung's name: a tone kind or an OFDM mode.
    pub name: String,
    /// Whether it is a tone-floor kind.
    pub tone: bool,
    /// What it occupies, absolute audio hertz.
    pub edges: Edges,
}

/// One air interface's transmissions, as absolute audio frequencies.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AirOccupancy {
    /// The air's nominal bandwidth, hertz: 2300 or 500.
    pub bandwidth_hz: usize,
    /// The audio frequency the waveform is centred on.
    pub centre_hz: f64,
    /// Every rung of the ladder, in order.
    pub rungs: Vec<RungOccupancy>,
    /// The ordinary family's control frame.
    pub control_ordinary: Edges,
    /// The tone floor's control frame.
    pub control_floor: Edges,
}

impl AirOccupancy {
    /// What a rung's data frames occupy.
    #[must_use]
    pub fn rung(&self, rung: usize) -> Option<Edges> {
        self.rungs.get(rung).map(|r| r.edges)
    }

    /// What the control frame of a family occupies.
    #[must_use]
    pub fn control(&self, floor: bool) -> Edges {
        if floor {
            self.control_floor
        } else {
            self.control_ordinary
        }
    }
}

#[derive(Deserialize)]
struct File {
    airs: std::collections::BTreeMap<String, AirFile>,
}

#[derive(Deserialize)]
struct AirFile {
    centre_hz: f64,
    rungs: Vec<RungFile>,
    control: ControlFile,
}

#[derive(Deserialize)]
struct RungFile {
    rung: usize,
    name: String,
    family: String,
    power: [f64; 2],
    spectral: [f64; 2],
}

#[derive(Deserialize)]
struct EdgesFile {
    power: [f64; 2],
    spectral: [f64; 2],
}

#[derive(Deserialize)]
struct ControlFile {
    ordinary: EdgesFile,
    floor: EdgesFile,
}

const FILE: &str = include_str!("../../data/occupancy.json");

fn parse(text: &str) -> Result<Vec<AirOccupancy>, String> {
    let file: File =
        serde_json::from_str(text).map_err(|e| format!("occupancy.json is malformed: {e}"))?;
    let mut airs = Vec::new();
    for (name, air) in file.airs {
        let bandwidth_hz: usize = name
            .parse()
            .map_err(|_| format!("occupancy.json names an air {name:?}"))?;
        let centre = air.centre_hz;
        let edges = |power: [f64; 2], spectral: [f64; 2]| Edges { power, spectral }.shifted(centre);
        let mut rungs = Vec::new();
        for (index, rung) in air.rungs.into_iter().enumerate() {
            if rung.rung != index || rung.power[0] >= rung.power[1] {
                return Err(format!("occupancy.json: rung {index} of the {name} Hz air"));
            }
            rungs.push(RungOccupancy {
                name: rung.name,
                tone: rung.family == "tone",
                edges: edges(rung.power, rung.spectral),
            });
        }
        airs.push(AirOccupancy {
            bandwidth_hz,
            centre_hz: centre,
            rungs,
            control_ordinary: edges(air.control.ordinary.power, air.control.ordinary.spectral),
            control_floor: edges(air.control.floor.power, air.control.floor.spectral),
        });
    }
    Ok(airs)
}

fn table() -> &'static Result<Vec<AirOccupancy>, String> {
    static TABLE: OnceLock<Result<Vec<AirOccupancy>, String>> = OnceLock::new();
    TABLE.get_or_init(|| parse(FILE))
}

/// The occupancy of the air with this nominal bandwidth.
///
/// # Errors
/// If the table is malformed or has no such air — which the policy treats as a reason not
/// to transmit, never as a reason to guess.
pub fn air(bandwidth_hz: usize) -> Result<&'static AirOccupancy, String> {
    match table() {
        Ok(airs) => airs
            .iter()
            .find(|a| a.bandwidth_hz == bandwidth_hz)
            .ok_or_else(|| format!("no occupancy is measured for the {bandwidth_hz} Hz air")),
        Err(error) => Err(error.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_measured_table_describes_both_airs_rung_by_rung() {
        for (bandwidth, rungs) in [(2300, 20), (500, 15)] {
            let air = air(bandwidth).expect("measured");
            assert_eq!(air.rungs.len(), rungs);
            assert!((air.centre_hz - 1500.0).abs() < 1e-9);
            for rung in &air.rungs {
                let (lo, hi) = rung.edges.range(Reading::Wider);
                assert!(lo < air.centre_hz && air.centre_hz < hi, "{}", rung.name);
                // the spectral reading is the wider for every Aether emission
                assert!(
                    rung.edges.spectral[1] - rung.edges.spectral[0]
                        >= rung.edges.power[1] - rung.edges.power[0]
                );
            }
        }
        // what the regulatory decisions rest on: the tone floor under 500 Hz by power and over
        // it by density, and the narrow air's OFDM over it either way
        let narrow = air(500).expect("measured");
        let width = |e: Edges, r| {
            let (lo, hi) = e.range(r);
            hi - lo
        };
        assert!(width(narrow.rungs[0].edges, Reading::Power) < 500.0);
        assert!(width(narrow.rungs[0].edges, Reading::Spectral) > 500.0);
        assert!(width(narrow.rungs[4].edges, Reading::Power) > 500.0);
        let wide = air(2300).expect("measured");
        assert!(width(wide.control_ordinary, Reading::Wider) < 2800.0);
        assert!(air(9600).is_err());
    }

    #[test]
    fn a_malformed_table_is_an_error_not_a_guess() {
        assert!(parse("{not json").is_err());
        let backwards = r#"{"airs": {"500": {"centre_hz": 1500, "rungs": [
            {"rung": 0, "name": "x", "family": "tone", "power": [200, -200], "spectral": [-250, 250]}],
            "control": {"ordinary": {"power": [-1, 1], "spectral": [-1, 1]},
                        "floor": {"power": [-1, 1], "spectral": [-1, 1]}}}}}"#;
        assert!(parse(backwards).is_err());
    }
}
