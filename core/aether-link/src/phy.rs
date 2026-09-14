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
}

/// A frame the physical layer detected, whether or not its payload decoded.
pub trait SoftFrame {
    /// Which container it is.
    fn container(&self) -> Container;
    /// Mode read from the frame.
    fn mode(&self) -> usize;
    /// Redundancy version read from the frame.
    fn rv(&self) -> u8;
    /// SNR measured on this frame, referenced to 3 kHz.
    fn snr_db(&self) -> f64;
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
}
