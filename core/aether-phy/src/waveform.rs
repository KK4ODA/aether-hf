//! OFDM numerology (ADR-0002) and everything derived from it.
//!
//! This is the single place the numbers live, exactly as `waveform.py` is in the reference
//! model. Downstream code derives from a [`WaveformParams`] rather than restating constants,
//! so the published specification cannot disagree with what the modem transmits.

/// Constellations the v1 waveform supports (Gray-labelled, BICM).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Modulation {
    /// Binary PSK, 1 bit per symbol.
    Bpsk,
    /// Quadrature PSK, 2 bits per symbol.
    Qpsk,
    /// 8-PSK, 3 bits per symbol.
    Psk8,
    /// 16-QAM, 4 bits per symbol.
    Qam16,
    /// 64-QAM, 6 bits per symbol.
    Qam64,
}

impl Modulation {
    /// Bits carried by one constellation symbol.
    #[must_use]
    pub const fn bits_per_symbol(self) -> usize {
        match self {
            Self::Bpsk => 1,
            Self::Qpsk => 2,
            Self::Psk8 => 3,
            Self::Qam16 => 4,
            Self::Qam64 => 6,
        }
    }

    /// Number of points, `2^m`.
    #[must_use]
    pub const fn order(self) -> usize {
        1 << self.bits_per_symbol()
    }

    /// Short name as it appears in the mode table and the specification.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bpsk => "BPSK",
            Self::Qpsk => "QPSK",
            Self::Psk8 => "PSK8",
            Self::Qam16 => "QAM16",
            Self::Qam64 => "QAM64",
        }
    }
}

/// Occupied-bandwidth options, mirroring what host applications already ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bandwidth {
    /// 500 Hz.
    Narrow500,
    /// 2300 Hz — the v1 default.
    Wide2300,
    /// 2750 Hz.
    Wide2750,
}

impl Bandwidth {
    /// Nominal occupied bandwidth in hertz.
    #[must_use]
    pub const fn hz(self) -> usize {
        match self {
            Self::Narrow500 => 500,
            Self::Wide2300 => 2300,
            Self::Wide2750 => 2750,
        }
    }
}

/// OFDM numerology for one bandwidth option.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaveformParams {
    /// Which bandwidth option this describes.
    pub bandwidth: Bandwidth,
    /// Complex baseband sample rate (Hz); 48 kHz audio divided by six.
    pub fs_baseband: f64,
    /// FFT length — 200 gives 40 Hz spacing and a 25 ms useful symbol.
    pub fft_size: usize,
    /// Cyclic prefix in samples (6 ms), covering ITU Poor with margin.
    pub cp_samples: usize,
    /// Audio centre frequency of the passband signal.
    pub centre_hz: f64,
    /// Comb pilots on every symbol: every N-th carrier, both band edges always pilots.
    pub pilot_carrier_spacing: usize,
    /// Every N-th OFDM symbol is a full pilot symbol.
    pub pilot_symbol_period: usize,
    /// Raised-cosine taper on each symbol edge; adjacent symbols overlap-add by this much,
    /// so the effective cyclic prefix is `cp_samples - taper_samples`.
    pub taper_samples: usize,
    /// Audio sample rate.
    pub audio_rate: usize,
}

/// The v1 default: 2.3 kHz occupied bandwidth (ADR-0002).
pub const WIDE_2300: WaveformParams = WaveformParams {
    bandwidth: Bandwidth::Wide2300,
    fs_baseband: 8000.0,
    fft_size: 200,
    cp_samples: 48,
    centre_hz: 1500.0,
    pilot_carrier_spacing: 4,
    pilot_symbol_period: 8,
    taper_samples: 8,
    audio_rate: 48000,
};

/// The 500 Hz waveform (ADR-0002, P7-0): the same numerology with twelve carriers.
pub const NARROW_500: WaveformParams = WaveformParams {
    bandwidth: Bandwidth::Narrow500,
    ..WIDE_2300
};

impl Default for WaveformParams {
    fn default() -> Self {
        WIDE_2300
    }
}

impl WaveformParams {
    /// Subcarrier spacing in hertz.
    #[must_use]
    pub fn subcarrier_spacing_hz(&self) -> f64 {
        self.fs_baseband / self.fft_size as f64
    }

    /// Useful (transform) symbol duration in seconds.
    #[must_use]
    pub fn useful_symbol_s(&self) -> f64 {
        self.fft_size as f64 / self.fs_baseband
    }

    /// Cyclic prefix duration in seconds.
    #[must_use]
    pub fn cp_s(&self) -> f64 {
        self.cp_samples as f64 / self.fs_baseband
    }

    /// Samples per transmitted symbol, `cp + N`.
    #[must_use]
    pub fn symbol_samples(&self) -> usize {
        self.fft_size + self.cp_samples
    }

    /// Symbol period in seconds.
    #[must_use]
    pub fn symbol_period_s(&self) -> f64 {
        self.symbol_samples() as f64 / self.fs_baseband
    }

    /// Symbol rate in baud.
    #[must_use]
    pub fn symbol_rate_bd(&self) -> f64 {
        1.0 / self.symbol_period_s()
    }

    /// Guard that survives windowing: the taper is taken out of the cyclic prefix.
    #[must_use]
    pub fn effective_cp_s(&self) -> f64 {
        (self.cp_samples - self.taper_samples) as f64 / self.fs_baseband
    }

    /// Active carriers that fit inside the nominal bandwidth.
    #[must_use]
    pub fn n_carriers(&self) -> usize {
        (self.bandwidth.hz() as f64 / self.subcarrier_spacing_hz()) as usize
    }

    /// Bandwidth the active carriers actually occupy.
    #[must_use]
    pub fn occupied_bandwidth_hz(&self) -> f64 {
        self.n_carriers() as f64 * self.subcarrier_spacing_hz()
    }

    /// Comb pilots at indices `0, s, 2s, …` plus the upper edge if it is not on the grid.
    #[must_use]
    pub fn n_pilot_carriers(&self) -> usize {
        let last = self.n_carriers() - 1;
        let on_grid = last / self.pilot_carrier_spacing + 1;
        on_grid + usize::from(last % self.pilot_carrier_spacing != 0)
    }

    /// Carriers left for data on an ordinary symbol.
    #[must_use]
    pub fn n_data_carriers(&self) -> usize {
        self.n_carriers() - self.n_pilot_carriers()
    }

    /// Fraction of symbols given over to full pilot symbols.
    #[must_use]
    pub fn pilot_symbol_fraction(&self) -> f64 {
        if self.pilot_symbol_period == 0 {
            0.0
        } else {
            1.0 / self.pilot_symbol_period as f64
        }
    }

    /// Interpolation factor between baseband and audio rates.
    ///
    /// # Panics
    /// If the audio rate is not an integer multiple of the baseband rate.
    #[must_use]
    pub fn resample_factor(&self) -> usize {
        let ratio = self.audio_rate as f64 / self.fs_baseband;
        assert!(
            (ratio - ratio.round()).abs() < 1e-9,
            "audio rate must be an integer multiple of the baseband rate"
        );
        ratio.round() as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_v1_numerology_is_adr_0002() {
        let p = WIDE_2300;
        assert!((p.subcarrier_spacing_hz() - 40.0).abs() < 1e-12);
        assert!((p.useful_symbol_s() - 0.025).abs() < 1e-12);
        assert_eq!(p.symbol_samples(), 248);
        assert_eq!(p.n_carriers(), 57);
        assert_eq!(p.n_pilot_carriers(), 15);
        assert_eq!(p.n_data_carriers(), 42);
        assert!((p.occupied_bandwidth_hz() - 2280.0).abs() < 1e-12);
        assert_eq!(p.resample_factor(), 6);
        assert!((p.effective_cp_s() - 0.005).abs() < 1e-12);
    }

    #[test]
    fn both_band_edges_are_pilots() {
        let p = WIDE_2300;
        // 57 carriers, every 4th from 0: 0,4,…,56 is 15 positions and 56 is the last carrier
        assert_eq!((p.n_carriers() - 1) % p.pilot_carrier_spacing, 0);
        assert_eq!(p.n_pilot_carriers(), (p.n_carriers() - 1) / 4 + 1);
    }

    #[test]
    fn modulation_orders_are_consistent() {
        for m in [
            Modulation::Bpsk,
            Modulation::Qpsk,
            Modulation::Psk8,
            Modulation::Qam16,
            Modulation::Qam64,
        ] {
            assert_eq!(m.order(), 1 << m.bits_per_symbol());
        }
        assert_eq!(Modulation::Qam64.bits_per_symbol(), 6);
    }
}
