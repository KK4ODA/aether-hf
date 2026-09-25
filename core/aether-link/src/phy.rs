//! What the link layer needs from a physical layer — and nothing more.
//!
//! The engine consumes *soft frames*: anything that can report its kind, mode, redundancy
//! version, air time and SNR, and can be decoded against an opaque HARQ buffer. Decoding is
//! the engine's call rather than the PHY's, because the engine is what knows which earlier
//! transmission a failed frame should be combined with.

/// Which container a frame is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Container {
    /// A data frame.
    Data,
    /// A control frame.
    Control,
}

/// Soft information carried between transmissions of the same block.
///
/// The real receiver passes full-codeword log-likelihood ratios; a simulator may pass
/// whatever stands in for them. The engine never looks inside.
pub type HarqBuffer = Vec<f64>;

/// A frame the engine wants transmitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxFrame {
    /// Which container to send it in.
    pub container: Container,
    /// The bytes to put in it.
    pub payload: Vec<u8>,
    /// Mode index, for data frames.
    pub mode: usize,
    /// Redundancy version, for data frames.
    pub rv: u8,
    /// A control frame to go out on the floor — the tone floor's control frame (ADR-0013).
    /// A data frame's family follows its mode; this flag is only read for control frames.
    pub floor: bool,
}

/// A frame the physical layer detected, whether or not its payload decoded.
pub trait SoftFrame {
    /// Which container it is.
    fn container(&self) -> Container;
    /// The rung of the ladder the frame was sent at (0 for a control frame).
    fn mode(&self) -> usize;
    /// Whether the frame is the floor's (the tone floor, ADR-0013). A data frame's family is
    /// also its mode's; for a control frame this is the only way the engine learns it.
    fn floor(&self) -> bool;
    /// Redundancy version read from the frame.
    fn rv(&self) -> u8;
    /// SNR measured on this frame, referenced to 3 kHz.
    fn snr_db(&self) -> f64;
    /// Whether the frame's measurements mean something when it does not decode: acquisition
    /// was confident enough that a real frame was there (ADR-0020). A detection just over its
    /// threshold that does not decode is as likely the correlator on noise, and its SNR an
    /// estimate of that noise — on the air one read -12 dB between frames decoding at +5 to +8.
    /// A frame that decodes is real whatever this says; a simulated frame is always real.
    fn trusted(&self) -> bool {
        true
    }
    /// When the frame started, in the receiver's clock.
    fn t_start(&self) -> f64;
    /// When it finished.
    fn t_end(&self) -> f64;
    /// Decode it, optionally combining with an earlier transmission of the same block.
    ///
    /// Returns the payload (or `None` if the check failed) and the buffer to keep for this
    /// sequence number. Combining must never mutate `buffer`.
    fn decode(&self, buffer: Option<&HarqBuffer>) -> (Option<Vec<u8>>, HarqBuffer);
}

/// The durations the engine's timers are built from.
#[derive(Debug, Clone, PartialEq)]
pub struct PhyTiming {
    /// Air time of one data frame.
    pub data_frame_s: f64,
    /// Air time of one control frame.
    pub control_frame_s: f64,
    /// Guard from the end of a received burst to keying up: PTT, audio latency, receive flush.
    pub turnaround_s: f64,
    /// Worst-case delay from a frame's last sample to the engine hearing about it.
    pub detect_latency_s: f64,
    /// Delay from the engine asking for a transmission to its first sample leaving the
    /// antenna: the keying lead, and the audio the daemon keeps queued ahead of the sound
    /// card. Zero for a simulator that plays what it is handed at once; a real station's is
    /// a few hundred milliseconds, and an engine that does not know it under-waits for every
    /// reply by that much. Found by two real daemons over a socket, not by the simulator.
    pub tx_latency_s: f64,
    /// Delay from a frame's *first* sample to the PHY reporting its preamble.
    ///
    /// When a physical layer can report this, the receiver learns a burst is continuing that
    /// quickly. When it is `None` the receiver must instead wait a whole data frame of
    /// silence to be sure a burst has ended, which costs roughly a quarter of the air time.
    pub preamble_detect_s: Option<f64>,
    /// Payload bytes per data frame, indexed by mode.
    pub data_capacity: Vec<usize>,
    /// Minimum usable SNR (3 kHz, AWGN) per mode, as the PHY's own benchmark measured it —
    /// what the rate controller steps along. Empty means the wide waveform's table
    /// ([`crate::rate::AWGN_THRESHOLD_DB`]); a PHY with another mode table — the 500 Hz
    /// waveform — hands its own here, and the engine never knows which air it is on.
    pub mode_threshold_db: Vec<f64>,
    /// Air time of a DATA frame at a floor mode — the tone floor's frame (ADR-0013), five
    /// times an ordinary one. `None` on an air without a floor family.
    pub floor_data_frame_s: Option<f64>,
    /// Air time of the floor's control frame.
    pub floor_control_frame_s: Option<f64>,
    /// How many of the leading modes are the floor's (the slowest ones).
    pub floor_modes: usize,
    /// The AWGN 10 % points of the air's two control frames, indexed by family (ordinary,
    /// floor) — what a simulated channel judges a control frame by. `None`: the wide air's
    /// ([`crate::rate::CONTROL_THRESHOLD_DB`]).
    pub control_threshold_db: Option<[f64; 2]>,
    /// The most margin the rate controller holds the first OFDM rung to against the floor
    /// ([`RateController::floor_margin_db`](crate::rate::RateController::floor_margin_db)):
    /// an air whose first rung stays productive on a fading path below the learned margin
    /// says how far; `None` leaves the learned margin in charge.
    pub floor_margin_db: Option<f64>,
    /// [`preamble_detect_s`](Self::preamble_detect_s) for the floor's frames: the tone floor
    /// announces a frame once its first sync block is in and has beaten its neighbours,
    /// 0.54 s after it starts, where an ordinary preamble takes 0.12 s. `None`: as
    /// `preamble_detect_s`.
    pub floor_preamble_detect_s: Option<f64>,
}

impl PhyTiming {
    /// Payload bytes a mode carries.
    ///
    /// # Panics
    /// If the mode is outside the capacity table.
    #[must_use]
    pub fn capacity(&self, mode: usize) -> usize {
        self.data_capacity[mode]
    }

    /// Whether a mode goes out on the floor layouts.
    #[must_use]
    pub const fn is_floor(&self, mode: usize) -> bool {
        mode < self.floor_modes
    }

    /// Air time of a DATA frame at a mode.
    #[must_use]
    pub fn data_frame_s_for(&self, mode: usize) -> f64 {
        match self.floor_data_frame_s {
            Some(floor) if self.is_floor(mode) => floor,
            _ => self.data_frame_s,
        }
    }

    /// How soon a frame of the given family is announced (`None`: it is not).
    #[must_use]
    pub fn preamble_detect_s_for(&self, floor: bool) -> Option<f64> {
        match self.floor_preamble_detect_s {
            Some(late) if floor => Some(late),
            _ => self.preamble_detect_s,
        }
    }

    /// Air time of a control frame of a family.
    #[must_use]
    pub fn control_frame_s_for(&self, floor: bool) -> f64 {
        match self.floor_control_frame_s {
            Some(long) if floor => long,
            _ => self.control_frame_s,
        }
    }

    /// Air time of a frame the engine is about to send.
    #[must_use]
    pub fn frame_s(&self, frame: &TxFrame) -> f64 {
        match frame.container {
            Container::Data => self.data_frame_s_for(frame.mode),
            Container::Control => self.control_frame_s_for(frame.floor),
        }
    }
}
