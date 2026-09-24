//! Frame layouts, the mode tables and the ladder — everything derives from
//! [`WaveformParams`].
//!
//! An OFDM *frame* is a preamble (two Schmidl–Cox symbols whose PN sequence encodes the frame
//! type) followed by `data_symbols` OFDM symbols, every `pilot_symbol_period`-th of which is
//! a full pilot symbol. A *mode* is a (modulation, code rate) pair; with a layout it fixes
//! the coded bits, information bits and payload bytes of a frame.
//!
//! Base-graph choice follows the public 5G rule (TS 38.212 §7.2.2), one code block per frame.
//!
//! Two air interfaces share this module (P7-0): the **wide** 2 300 Hz waveform with its
//! fourteen modes, and the **narrow** 500 Hz waveform — twelve carriers, the same symbol
//! timing and the same frame layouts, so the link layer's clocks do not change — with its own
//! table. A 500 Hz signal puts its power into a fifth of the band, ≈ 6.8 dB more per carrier
//! at the same 3 kHz-referenced SNR, so its control mode can be QPSK ½ where the wide table
//! starts at BPSK ⅕ and still reach the same floor; it has to be, because with eight data
//! carriers a control frame's seven bytes fit a SHORT frame at nothing slower.
//!
//! Below both tables is the **tone floor** (ADR-0013, [`crate::tone`]): a steady-envelope
//! sixteen-tone FSK family, sent at the OFDM frames' peak amplitude and detected by energy.
//! What the link layer calls "mode N" is a rung of the air's **ladder**: the tone floor's data
//! kinds — on the 2 300 Hz air with its fast kinds (ADR-0014) — then the air's OFDM modes,
//! most robust first ([`Rung`], [`AirInterface::ladder`]). An OFDM frame's chips carry its
//! OFDM mode index, which is not its rung: the wide ladder puts OFDM mode 0 at rung 6, the
//! narrow one skips the OFDM modes the floor replaced.
//! [`AirInterface`] bundles a waveform with its layouts, modes and ladder; [`air_interface`]
//! finds the one for a [`WaveformParams`].

use aether_fec::{
    CRC24A,
    ldpc::{select_base_graph, select_lifting_size},
};

use crate::{
    tone::{self, ToneKind},
    waveform::{Bandwidth, Modulation, NARROW_500, WIDE_2300, WaveformParams},
};

/// Symbols of preamble ahead of a frame.
pub const PREAMBLE_SYMBOLS: usize = 2;

/// How many OFDM symbols a frame carries, and which of them are full pilot symbols.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameLayout {
    /// `"long"` or `"short"`.
    pub name: &'static str,
    /// Symbols after the preamble.
    pub data_symbols: usize,
    /// The numerology the layout is measured against.
    pub waveform: WaveformParams,
    /// Identical preamble symbols ahead of the data symbols: two on every layout since the
    /// OFDM floor family (ADR-0009, eight) gave way to the tone floor (ADR-0013).
    pub preamble_symbols: usize,
    /// Symbols either side over which the receiver averages its comb-pilot channel
    /// estimate: ±1.
    pub pilot_smoothing: usize,
}

/// Data frames: 34 symbols, about 1.05 s.
pub const LONG: FrameLayout = FrameLayout {
    name: "long",
    data_symbols: 32,
    waveform: WIDE_2300,
    preamble_symbols: PREAMBLE_SYMBOLS,
    pilot_smoothing: 1,
};
/// Control frames (ACK, connect, ping): 14 symbols, about 0.43 s.
pub const SHORT: FrameLayout = FrameLayout {
    name: "short",
    data_symbols: 12,
    waveform: WIDE_2300,
    preamble_symbols: PREAMBLE_SYMBOLS,
    pilot_smoothing: 1,
};

impl FrameLayout {
    /// Indices, within the data symbols, that are full pilot symbols.
    #[must_use]
    pub fn pilot_symbol_indices(&self) -> Vec<usize> {
        let period = self.waveform.pilot_symbol_period;
        if period == 0 {
            return Vec::new();
        }
        (0..self.data_symbols).step_by(period).collect()
    }

    /// How many full pilot symbols the frame has.
    #[must_use]
    pub fn n_pilot_symbols(&self) -> usize {
        self.pilot_symbol_indices().len()
    }

    /// Data symbols that carry payload rather than pilots.
    #[must_use]
    pub fn n_payload_symbols(&self) -> usize {
        self.data_symbols - self.n_pilot_symbols()
    }

    /// Data-carrier slots available for coded bits in one frame.
    #[must_use]
    pub fn qam_symbols(&self) -> usize {
        self.n_payload_symbols() * self.waveform.n_data_carriers()
    }

    /// Preamble plus data symbols.
    #[must_use]
    pub fn total_symbols(&self) -> usize {
        self.preamble_symbols + self.data_symbols
    }

    /// Frame duration in seconds.
    #[must_use]
    pub fn duration_s(&self) -> f64 {
        self.total_symbols() as f64 * self.waveform.symbol_period_s()
    }

    /// Frame length in baseband samples.
    #[must_use]
    pub fn samples(&self) -> usize {
        self.total_symbols() * self.waveform.symbol_samples()
    }
}

/// One (modulation, code rate) pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    /// Position in [`MODES`], and the value signalled in the pilot chips.
    pub index: usize,
    /// Constellation.
    pub modulation: Modulation,
    /// Code rate numerator.
    pub rate_num: usize,
    /// Code rate denominator.
    pub rate_den: usize,
}

impl Mode {
    /// Name as it appears in the mode table, e.g. `QAM16-3/4`.
    #[must_use]
    pub fn name(&self) -> String {
        format!(
            "{}-{}/{}",
            self.modulation.name(),
            self.rate_num,
            self.rate_den
        )
    }

    /// Code rate as a float.
    #[must_use]
    pub fn code_rate(&self) -> f64 {
        self.rate_num as f64 / self.rate_den as f64
    }

    /// Coded bits carried by one frame.
    #[must_use]
    pub fn coded_bits(&self, layout: &FrameLayout) -> usize {
        layout.qam_symbols() * self.modulation.bits_per_symbol()
    }

    /// `K'` — payload plus CRC, rounded down to a whole number of payload bytes.
    #[must_use]
    pub fn info_bits(&self, layout: &FrameLayout) -> usize {
        let raw = self.coded_bits(layout) * self.rate_num / self.rate_den;
        let payload_bytes = (raw - CRC24A.width as usize) / 8;
        payload_bytes * 8 + CRC24A.width as usize
    }

    /// Payload bytes carried by one frame.
    #[must_use]
    pub fn payload_bytes(&self, layout: &FrameLayout) -> usize {
        (self.info_bits(layout) - CRC24A.width as usize) / 8
    }

    /// Base graph chosen by TS 38.212 §7.2.2.
    #[must_use]
    pub fn base_graph(&self, layout: &FrameLayout) -> u8 {
        select_base_graph(self.payload_bytes(layout) * 8, self.code_rate())
    }

    /// Lifting size for this mode's information block.
    ///
    /// # Panics
    /// If the information block does not fit the base graph, which the fixed mode table
    /// never produces.
    #[must_use]
    pub fn lifting_size(&self, layout: &FrameLayout) -> usize {
        select_lifting_size(self.base_graph(layout), self.info_bits(layout))
            .expect("mode table fits its base graph")
    }

    /// Payload bits per second of frame air time, ignoring ACK turnaround.
    #[must_use]
    pub fn net_bit_rate(&self, layout: &FrameLayout) -> f64 {
        8.0 * self.payload_bytes(layout) as f64 / layout.duration_s()
    }
}

const fn mode(index: usize, modulation: Modulation, rate_num: usize, rate_den: usize) -> Mode {
    Mode {
        index,
        modulation,
        rate_num,
        rate_den,
    }
}

/// The mode table, ordered from most robust to fastest; the rate controller steps along it.
pub const MODES: [Mode; 14] = [
    mode(0, Modulation::Bpsk, 1, 5),
    mode(1, Modulation::Bpsk, 1, 3),
    mode(2, Modulation::Bpsk, 1, 2),
    mode(3, Modulation::Qpsk, 1, 3),
    mode(4, Modulation::Qpsk, 1, 2),
    mode(5, Modulation::Qpsk, 2, 3),
    mode(6, Modulation::Psk8, 1, 2),
    mode(7, Modulation::Psk8, 2, 3),
    mode(8, Modulation::Qam16, 1, 2),
    mode(9, Modulation::Qam16, 2, 3),
    mode(10, Modulation::Qam16, 3, 4),
    mode(11, Modulation::Qam64, 2, 3),
    mode(12, Modulation::Qam64, 3, 4),
    mode(13, Modulation::Qam64, 5, 6),
];

/// Every control frame (ACK, connect, ping) uses the most robust mode on the SHORT layout.
pub const CONTROL_MODE: Mode = MODES[0];

/// 500 Hz data frames: the same 34 symbols; 28 payload symbols × 8 carriers = 224 slots.
pub const NARROW_LONG: FrameLayout = FrameLayout {
    name: "long",
    data_symbols: 32,
    waveform: NARROW_500,
    preamble_symbols: PREAMBLE_SYMBOLS,
    pilot_smoothing: 1,
};
/// 500 Hz control frames: the same 14 symbols; 10 × 8 = 80 slots → 7 payload bytes at QPSK ½.
pub const NARROW_SHORT: FrameLayout = FrameLayout {
    name: "short",
    data_symbols: 12,
    waveform: NARROW_500,
    preamble_symbols: PREAMBLE_SYMBOLS,
    pilot_smoothing: 1,
};

/// The 500 Hz OFDM mode table, most robust first. Modes 0 and 1 were the OFDM floor family's
/// (ADR-0009) until the tone floor (ADR-0013) replaced them; they stay in the table only
/// because an OFDM frame's chip sequence is indexed by its position in it, and are on no rung
/// of the ladder. Mode 2, QPSK ⅓, is the ladder's first OFDM rung. Mode 3 is QPSK ½: the
/// slowest mode whose SHORT frame carries a control frame and whose LONG frame carries a
/// connect request; ordinary control frames, connect requests, beacons and probes go out at
/// it. Thirteen modes is what the 32-chip sequence set holds at |ρ| ≤ 0.25.
pub const NARROW_MODES: [Mode; 13] = [
    mode(0, Modulation::Qpsk, 1, 10),
    mode(1, Modulation::Qpsk, 1, 5),
    mode(2, Modulation::Qpsk, 1, 3),
    mode(3, Modulation::Qpsk, 1, 2),
    mode(4, Modulation::Qpsk, 2, 3),
    mode(5, Modulation::Psk8, 1, 2),
    mode(6, Modulation::Psk8, 2, 3),
    mode(7, Modulation::Qam16, 1, 2),
    mode(8, Modulation::Qam16, 2, 3),
    mode(9, Modulation::Qam16, 3, 4),
    mode(10, Modulation::Qam64, 2, 3),
    mode(11, Modulation::Qam64, 3, 4),
    mode(12, Modulation::Qam64, 5, 6),
];

/// The narrow control mode's index: QPSK ½.
pub const NARROW_CONTROL_MODE_INDEX: usize = 3;
/// The narrow control mode.
pub const NARROW_CONTROL_MODE: Mode = NARROW_MODES[NARROW_CONTROL_MODE_INDEX];

/// The wide ladder's OFDM rungs: every mode of the table.
const WIDE_OFDM_LADDER: [usize; 14] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13];
/// The narrow ladder's OFDM rungs: the table from QPSK ⅓ up.
const NARROW_OFDM_LADDER: [usize; 11] = [2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];

/// One step of an air's ladder — what the link layer, the rate controller and the operator
/// call "mode N": a tone-floor kind, or an OFDM mode on the layout it goes out on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rung {
    /// A tone-floor data kind (ADR-0013, ADR-0014).
    Tone(&'static ToneKind),
    /// An OFDM mode on its data layout.
    Ofdm(Mode, FrameLayout),
}

impl Rung {
    /// Whether the rung is the tone floor's.
    #[must_use]
    pub const fn is_floor(&self) -> bool {
        matches!(self, Self::Tone(_))
    }

    /// Its name: the tone kind's (`tone-24`) or the OFDM mode's (`QPSK-1/2`).
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::Tone(kind) => kind.name.to_string(),
            Self::Ofdm(mode, _) => mode.name(),
        }
    }

    /// Payload bytes a DATA frame at this rung carries.
    #[must_use]
    pub fn payload_bytes(&self) -> usize {
        match self {
            Self::Tone(kind) => kind.payload_bytes,
            Self::Ofdm(mode, layout) => mode.payload_bytes(layout),
        }
    }

    /// A DATA frame's length in seconds.
    #[must_use]
    pub fn duration_s(&self) -> f64 {
        match self {
            Self::Tone(kind) => kind.duration_s(),
            Self::Ofdm(_, layout) => layout.duration_s(),
        }
    }

    /// Payload bits per second of frame air time.
    #[must_use]
    pub fn net_bps(&self) -> f64 {
        8.0 * self.payload_bytes() as f64 / self.duration_s()
    }

    /// The OFDM mode, if the rung is one.
    #[must_use]
    pub const fn ofdm_mode(&self) -> Option<Mode> {
        match self {
            Self::Tone(_) => None,
            Self::Ofdm(mode, _) => Some(*mode),
        }
    }
}

/// One waveform with the layouts, modes and ladder that go with it — what a transmitter,
/// receiver, detector or link needs to know about the air it is on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AirInterface {
    /// The numerology.
    pub params: WaveformParams,
    /// The data-frame layout.
    pub long: FrameLayout,
    /// The control-frame layout.
    pub short: FrameLayout,
    /// The OFDM mode table, most robust first: what an OFDM frame's chips index. The link
    /// runs the ladder.
    pub modes: &'static [Mode],
    /// Largest pairwise correlation allowed between the (mode, RV) chip sequences: 0.2 for
    /// the wide waveform's 56 sequences of 168 chips, 0.25 for the narrow one's 52 of 32.
    pub chip_correlation_bound: f64,
    /// The detector's normalised matched-filter peak above which a preamble is declared,
    /// set just above the statistic's maximum over 60 s of band-limited noise: 0.36 at
    /// 2 300 Hz, 0.56 at 500 Hz (a fifth of the degrees of freedom in a preamble's span,
    /// and the signal peaks rise by about as much, so the two floors coincide).
    pub acquisition_threshold: f64,
    /// The OFDM modes on the ladder, ascending, above the tone floor's rungs.
    pub ofdm_ladder: &'static [usize],
    /// The OFDM mode ordinary control frames, connect requests, beacons and probes go out
    /// at: the slowest whose SHORT frame carries a control frame.
    pub control_mode_index: usize,
    /// Whether the ladder carries the tone floor's fast kinds (ADR-0014) above its own two:
    /// the 2 300 Hz air's does; their 800 and 1 600 Hz do not fit in 500.
    pub fast_tones: bool,
}

impl AirInterface {
    /// The control mode: ordinary control frames, connect requests, beacons and probes go
    /// out at it.
    #[must_use]
    pub const fn control_mode(&self) -> Mode {
        self.modes[self.control_mode_index]
    }

    /// The tone floor's data kinds on this air, slowest first: the ladder's first rungs —
    /// the floor's own two, then on the 2 300 Hz air the fast kinds (ADR-0014).
    #[must_use]
    pub fn tone_data(&self) -> Vec<&'static ToneKind> {
        let fast: &'static [ToneKind] = if self.fast_tones {
            tone::fast_kinds()
        } else {
            &[]
        };
        tone::data_kinds().iter().chain(fast).collect()
    }

    /// Every tone kind a receiver on this air looks for: the control frame and the data
    /// kinds of [`tone_data`](Self::tone_data).
    #[must_use]
    pub fn tone_kinds(&self) -> Vec<&'static ToneKind> {
        std::iter::once(self.tone_control())
            .chain(self.tone_data())
            .collect()
    }

    /// The tone floor's control frame: control frames while the link runs the floor.
    #[must_use]
    pub fn tone_control(&self) -> &'static ToneKind {
        tone::control_kind()
    }

    /// How many of the ladder's leading rungs are the floor's: rung `floor_modes()` is the
    /// first OFDM one.
    #[must_use]
    pub fn floor_modes(&self) -> usize {
        tone::data_kinds().len()
            + if self.fast_tones {
                tone::fast_kinds().len()
            } else {
                0
            }
    }

    /// Every rung, most robust first: the tone floor's data kinds, then the OFDM modes of
    /// [`ofdm_ladder`](Self::ofdm_ladder) on the LONG layout.
    #[must_use]
    pub fn ladder(&self) -> Vec<Rung> {
        self.tone_data()
            .into_iter()
            .map(Rung::Tone)
            .chain(
                self.ofdm_ladder
                    .iter()
                    .map(|&m| Rung::Ofdm(self.modes[m], self.long)),
            )
            .collect()
    }

    /// The rung at `index`.
    ///
    /// # Panics
    /// If the ladder has no such rung.
    #[must_use]
    pub fn rung(&self, index: usize) -> Rung {
        let floor = self.floor_modes();
        if index < floor {
            return Rung::Tone(self.tone_data()[index]);
        }
        let mode = *self
            .ofdm_ladder
            .get(index - floor)
            .unwrap_or_else(|| panic!("the ladder has no rung {index}"));
        Rung::Ofdm(self.modes[mode], self.long)
    }

    /// Rungs on the ladder — the link layer's mode count.
    #[must_use]
    pub fn n_rungs(&self) -> usize {
        self.floor_modes() + self.ofdm_ladder.len()
    }

    /// OFDM modes in the table — what the chip sequences are indexed by.
    #[must_use]
    pub const fn n_modes(&self) -> usize {
        self.modes.len()
    }

    /// Whether a rung is the tone floor's.
    #[must_use]
    pub fn is_floor(&self, rung: usize) -> bool {
        rung < self.floor_modes()
    }

    /// The rung an OFDM mode sits on, if it sits on one.
    #[must_use]
    pub fn rung_of(&self, ofdm_mode: usize) -> Option<usize> {
        self.ofdm_ladder
            .iter()
            .position(|&m| m == ofdm_mode)
            .map(|j| self.floor_modes() + j)
    }

    /// The rung of the control mode.
    ///
    /// # Panics
    /// If the control mode is on no rung, which the fixed tables never produce.
    #[must_use]
    pub fn control_rung(&self) -> usize {
        self.rung_of(self.control_mode_index)
            .expect("the control mode is on the ladder")
    }

    /// The layout an OFDM frame of this kind has.
    #[must_use]
    pub const fn layout_for(&self, data: bool) -> FrameLayout {
        if data { self.long } else { self.short }
    }

    /// Every OFDM layout of this air.
    #[must_use]
    pub const fn layouts(&self) -> [FrameLayout; 2] {
        [self.long, self.short]
    }

    /// The nominal bandwidth in hertz.
    #[must_use]
    pub const fn bandwidth_hz(&self) -> usize {
        self.params.bandwidth.hz()
    }
}

/// The 2 300 Hz air interface.
pub const WIDE: AirInterface = AirInterface {
    params: WIDE_2300,
    long: LONG,
    short: SHORT,
    modes: &MODES,
    chip_correlation_bound: 0.2,
    acquisition_threshold: 0.36,
    ofdm_ladder: &WIDE_OFDM_LADDER,
    control_mode_index: 0,
    fast_tones: true,
};

/// The 500 Hz air interface.
pub const NARROW: AirInterface = AirInterface {
    params: NARROW_500,
    long: NARROW_LONG,
    short: NARROW_SHORT,
    modes: &NARROW_MODES,
    chip_correlation_bound: 0.25,
    acquisition_threshold: 0.56,
    ofdm_ladder: &NARROW_OFDM_LADDER,
    control_mode_index: NARROW_CONTROL_MODE_INDEX,
    fast_tones: false,
};

/// The air interface a waveform belongs to, by bandwidth.
///
/// The numerology of a bandwidth is fixed by ADR-0002, so a [`WaveformParams`] with the
/// same bandwidth and different numbers is not a waveform this modem has.
///
/// # Panics
/// If the bandwidth has no air interface (2 750 Hz is P9-3, not yet) or the numbers differ
/// from the air interface's: a programming error, not a runtime condition.
#[must_use]
pub fn air_interface(params: WaveformParams) -> AirInterface {
    let air = match params.bandwidth {
        Bandwidth::Wide2300 => WIDE,
        Bandwidth::Narrow500 => NARROW,
        Bandwidth::Wide2750 => panic!("no air interface for 2750 Hz yet"),
    };
    assert!(
        air.params == params,
        "{:?} numerology differs from the air interface's",
        params.bandwidth
    );
    air
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_are_the_specification() {
        assert_eq!(LONG.total_symbols(), 34);
        assert_eq!(LONG.samples(), 8432);
        assert_eq!(LONG.qam_symbols(), 1176);
        assert_eq!(LONG.pilot_symbol_indices(), vec![0, 8, 16, 24]);
        assert_eq!(SHORT.total_symbols(), 14);
        assert_eq!(SHORT.qam_symbols(), 420);
        assert_eq!(SHORT.pilot_symbol_indices(), vec![0, 8]);
        assert!((LONG.duration_s() - 1.054).abs() < 1e-9);
    }

    #[test]
    fn payload_grows_monotonically_with_mode() {
        let payloads: Vec<usize> = MODES.iter().map(|m| m.payload_bytes(&LONG)).collect();
        assert!(payloads.windows(2).all(|w| w[0] <= w[1]), "{payloads:?}");
        assert_eq!(payloads[0], 26);
        assert_eq!(payloads[13], 732);
    }

    #[test]
    fn coded_bits_are_the_slots_times_the_constellation() {
        for m in &MODES {
            assert_eq!(m.coded_bits(&LONG), 1176 * m.modulation.bits_per_symbol());
        }
    }

    #[test]
    fn every_mode_fits_its_lifting_size() {
        for m in &MODES {
            let z = m.lifting_size(&LONG);
            let kb = if m.base_graph(&LONG) == 1 { 22 } else { 10 };
            assert!(kb * z >= m.info_bits(&LONG), "{}", m.name());
        }
    }

    #[test]
    fn the_control_mode_carries_seven_bytes() {
        assert_eq!(CONTROL_MODE.payload_bytes(&SHORT), 7);
        assert_eq!(NARROW_CONTROL_MODE.payload_bytes(&NARROW_SHORT), 7);
        assert_eq!(WIDE.tone_control().payload_bytes, 7);
    }

    #[test]
    fn the_narrow_table_is_the_specification() {
        // the same clock as the wide waveform: the link layer's timers do not change
        assert_eq!(NARROW_LONG.samples(), LONG.samples());
        assert!((NARROW_SHORT.duration_s() - SHORT.duration_s()).abs() < 1e-12);
        assert_eq!(NARROW_LONG.qam_symbols(), 28 * 8);
        assert_eq!(NARROW_SHORT.qam_symbols(), 10 * 8);
        let payloads: Vec<usize> = NARROW_MODES
            .iter()
            .map(|m| m.payload_bytes(&NARROW_LONG))
            .collect();
        assert_eq!(
            payloads,
            vec![2, 8, 15, 25, 34, 39, 53, 53, 71, 81, 109, 123, 137],
            "the narrow table's payloads are the model's"
        );
        assert!((NARROW_MODES[12].net_bit_rate(&NARROW_LONG) - 1040.0).abs() < 1.0);
        assert_eq!(NARROW.control_mode(), NARROW_MODES[3]);
        for (i, m) in NARROW_MODES.iter().enumerate() {
            assert_eq!(m.index, i);
            let z = m.lifting_size(&NARROW_LONG);
            let kb = if m.base_graph(&NARROW_LONG) == 1 {
                22
            } else {
                10
            };
            assert!(kb * z >= m.info_bits(&NARROW_LONG), "{}", m.name());
        }
        assert_eq!(air_interface(NARROW_500), NARROW);
        assert_eq!(air_interface(WIDE_2300), WIDE);
        assert_eq!(NARROW.n_modes(), 13);
        assert_eq!(WIDE.control_mode(), CONTROL_MODE);
        assert_eq!(NARROW.layout_for(false), NARROW_SHORT);
    }

    #[test]
    fn the_ladders_are_the_models() {
        // ADR-0013: the tone floor's two data kinds under each air's OFDM rungs; ADR-0014:
        // on the wide air its four fast kinds between them
        let wide: Vec<(String, usize)> = WIDE
            .ladder()
            .iter()
            .map(|r| (r.name(), r.payload_bytes()))
            .collect();
        assert_eq!(wide.len(), 20);
        assert_eq!(WIDE.n_rungs(), 20);
        assert_eq!(wide[0], ("tone-24".to_string(), 24));
        assert_eq!(wide[1], ("tone-36".to_string(), 36));
        assert_eq!(wide[2], ("tone50-51".to_string(), 51));
        assert_eq!(wide[5], ("tone100-153".to_string(), 153));
        assert_eq!(wide[6], ("BPSK-1/5".to_string(), 26));
        assert_eq!(wide[19], ("QAM64-5/6".to_string(), 732));
        assert_eq!(
            NARROW
                .ladder()
                .iter()
                .map(Rung::payload_bytes)
                .collect::<Vec<_>>(),
            vec![24, 36, 15, 25, 34, 39, 53, 53, 71, 81, 109, 123, 137]
        );
        assert_eq!((WIDE.floor_modes(), NARROW.floor_modes()), (6, 2));
        assert_eq!((WIDE.control_rung(), NARROW.control_rung()), (6, 3));
        assert_eq!(NARROW.rung_of(0), None);
        assert_eq!(NARROW.rung_of(2), Some(2));
        assert!(WIDE.is_floor(5) && !WIDE.is_floor(6));
        assert_eq!(WIDE.rung(6).ofdm_mode(), Some(MODES[0]));
        assert!(NARROW.rung(0).is_floor());
        assert_eq!(WIDE.tone_kinds().len(), 7);
        assert_eq!(NARROW.tone_kinds().len(), 3);
        // bytes per second climb the ladder, which is what the rate controller steps along —
        // but for the first OFDM rung of the wide air, which its fastest tone kind beats and
        // which stays on the ladder as the ordinary family's robust mode
        for air in [WIDE, NARROW] {
            let rates: Vec<f64> = air
                .ladder()
                .iter()
                .enumerate()
                .filter(|&(i, _)| !(air.fast_tones && i == air.floor_modes()))
                .map(|(_, r)| r.net_bps())
                .collect();
            assert!(rates.windows(2).all(|w| w[0] <= w[1] + 1e-9), "{rates:?}");
        }
    }

    #[test]
    fn mode_names_match_the_table() {
        assert_eq!(MODES[0].name(), "BPSK-1/5");
        assert_eq!(MODES[10].name(), "QAM16-3/4");
        assert_eq!(MODES[13].name(), "QAM64-5/6");
    }
}
