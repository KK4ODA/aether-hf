//! The physical layer as the station sees it: one air per station (ADR-0113).
//!
//! The station hands an air the sound card's audio and gets audio back; baseband, acquisition,
//! decoding and modulation stay inside it. What the station needs of an air is [`Air`]: what the
//! air is, a block of captured audio heard in the pieces the station names, a burst rendered,
//! and the channel's occupancy ([`ChannelSense`]). Aether HF's air is [`hf::HfAir`]; Aether FM's
//! will sit beside it (ADR-0112).

pub mod hf;

use std::{any::Any, cell::RefCell, rc::Rc};

use aether_link::{HarqBuffer, PhyTiming, TxFrame};

use crate::busy::{BusyConfig, BusyReading, BusyReason, BusyState};

pub use hf::{HfAir, phy_timing};

/// An in-phase and quadrature pair: a received symbol, for the constellation display.
pub type Iq = (f64, f64);

/// What an air is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AirInfo {
    /// The sound card's rate this air runs at, samples a second.
    pub audio_rate: usize,
    /// The bandwidth it transmits in, hertz: what a call states and the rules judge.
    pub bandwidth_hz: usize,
    /// Its ordinary data frame's length, seconds: how long the receive lamp stays lit for a
    /// frame announced.
    pub data_frame_s: f64,
    /// The rungs of its ladder.
    pub rungs: usize,
}

/// One rung of an air's ladder, for the displays and the Test.
#[derive(Debug, Clone, PartialEq)]
pub struct RungInfo {
    /// Its name ("tone-36", "QPSK 1/2"), for the panel.
    pub name: String,
    /// The bytes one of its frames carries.
    pub payload_bytes: usize,
    /// One frame's length, seconds.
    pub duration_s: f64,
}

/// What the station knows of a block of captured audio that the air does not.
#[derive(Debug, Clone, Copy)]
pub enum Listening {
    /// This station is transmitting: what came in is its own sidetone, muted, except what the
    /// card captured after the transmission's last sample had played — `past` audio samples on
    /// the card's clock — once nothing more is queued behind it.
    Transmitting {
        /// Audio samples the card has played past the transmission's last sample.
        past: usize,
        /// Whether nothing more waits in the playback queue.
        playback_empty: bool,
    },
    /// Listening: the receiver hears everything; channel sense only when `sense`, which is
    /// false in the deafness a lead after the key is released.
    Hearing {
        /// Whether channel sense hears this block.
        sense: bool,
    },
}

/// A frame the receiver finished with, decoded or not, read out for the station. Every time is
/// on the station's clock, in seconds.
pub struct HeardFrame {
    /// A control frame (an acknowledgement, a header, a disconnect), not a DATA frame.
    pub control: bool,
    /// Sent in the air's second, robust family (HF's tone floor).
    pub floor: bool,
    /// The rung of the air's ladder a DATA frame names; `None` for a control frame, and for
    /// chips naming no rung — noise, or a station on another version.
    pub rung: Option<usize>,
    /// Where the frame starts.
    pub start_s: f64,
    /// Its length.
    pub length_s: f64,
    /// Its redundancy version.
    pub rv: u8,
    /// Its SNR, referred to 3 kHz.
    pub snr_3k_db: f64,
    /// Its carrier offset as the receiver measured it, hertz.
    pub cfo_hz: f64,
    /// How surely the receiver read its mode and redundancy version.
    pub mode_confidence: f64,
    /// How strongly it was acquired, against the air's threshold.
    pub detect_confidence: f64,
    /// How many frames of its burst follow it, if it says (ADR-0068).
    pub follows: Option<u8>,
    /// The echo it read, if its family reads one (ADR-0087).
    pub echo: Option<f64>,
    /// Its equalised symbols, for the constellation; none for a frame detected by energy.
    pub symbols: Vec<Iq>,
    /// What it decoded to.
    pub payload: Option<Vec<u8>>,
    /// The air's own frame, which decodes through the air that heard it ([`Air::decoder`]).
    pub soft: Box<dyn Any>,
}

impl HeardFrame {
    /// Whether it decoded.
    #[must_use]
    pub const fn ok(&self) -> bool {
        self.payload.is_some()
    }
}

/// A frame announced on its preamble, still arriving.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arrival {
    /// Where it starts.
    pub start_s: f64,
    /// Where it ends.
    pub end_s: f64,
    /// Its length, as the preamble named it.
    pub length_s: f64,
    /// Sent in the air's robust family, trusted without the acquisition gate.
    pub floor: bool,
    /// How strongly it was acquired.
    pub detect_confidence: f64,
}

/// What the receiver found in a piece of a block.
pub struct Heard {
    /// Frames it finished with, in the order found.
    pub frames: Vec<HeardFrame>,
    /// Frames announced, still arriving.
    pub arrivals: Vec<Arrival>,
}

/// A burst rendered to audio.
pub struct Rendered {
    /// The audio, at unit level, the filters' tails included.
    pub audio: Vec<f32>,
    /// How much of it is the first frame.
    pub first_frame: usize,
}

/// What decodes the frames an air hears, for the link engine, which owns the HARQ buffers.
pub trait FrameDecoder {
    /// Decode `frame` — one of this air's [`HeardFrame::soft`] — combined with `buffer`.
    fn decode(
        &mut self,
        frame: &dyn Any,
        buffer: Option<&HarqBuffer>,
    ) -> (Option<Vec<u8>>, HarqBuffer);
}

/// Whether someone else is on the frequency, and the panel's reading of it.
pub trait ChannelSense {
    /// Whether the channel is occupied now.
    fn busy(&self, now: f64) -> bool;
    /// Whether it has heard enough to say.
    fn settled(&self) -> bool;
    /// What last marked it busy.
    fn reason(&self) -> Option<BusyReason>;
    /// The level heard, dB.
    fn level_db(&self) -> f64;
    /// The noise floor learned, dB.
    fn floor_db(&self) -> f64;
    /// The largest level over the floor since the last reading, dB.
    fn excess_peak_db(&self) -> f64;
    /// The passband's highest spectral bin over its median, dB.
    fn shape_db(&self) -> f64;
    /// How far the radio's AGC holds its gain down, dB, when it is read.
    fn gain_db(&self) -> Option<f64>;
    /// The threshold over the floor, dB.
    fn threshold_db(&self) -> f64;
    /// Set the threshold over the floor, dB.
    fn set_threshold_db(&mut self, threshold_db: f64);
    /// A frame's span, `from` to `until`: a frame's, not the noise's.
    fn explain(&mut self, from: f64, until: f64);
    /// A frame acquired at `detect_confidence` marks the channel busy.
    fn mark_frame(&mut self, now: f64, detect_confidence: f64);
    /// The dial moved: what was learned is another frequency's.
    fn retune(&mut self, now: f64);
    /// Start a new interval for the peak reading.
    fn reset_excess_peak(&mut self);
    /// Its state, for a recording to start a trace from.
    fn state(&self, now: f64) -> BusyState;
    /// Its reading, for a recording.
    fn reading(&self, now: f64) -> BusyReading;
}

/// One air: the physical layer and channel sense, as the station sees them (ADR-0113 §2.2).
pub trait Air {
    /// What it is.
    fn info(&self) -> AirInfo;
    /// The link engine's numbers.
    fn timing(&self) -> PhyTiming;
    /// Its ladder.
    fn ladder(&self) -> Vec<RungInfo>;

    /// Take a block of captured audio: channel sense fed or spared, and the block cut into
    /// what the receiver hears muted and what it hears as it came. Returns, when the block
    /// holds the end of this station's transmission, the time it ended: the station ends the
    /// transmission between the two pieces ([`Self::listen_next`]).
    fn take_block(&mut self, audio: &[f32], how: Listening, now: f64) -> Option<f64>;
    /// Feed the receiver the next piece of the block taken last, and take what it found.
    fn listen_next(&mut self) -> Heard;
    /// The mean power of the block taken last, as the receiver sees it; `None` for none.
    fn block_power(&self) -> Option<f64>;
    /// The rung `frame` names on this air's ladder: one heard on another air (a move between
    /// them) read again.
    fn rung_of(&self, frame: &HeardFrame) -> Option<usize>;
    /// The decoder every frame handed to the engine decodes through.
    fn decoder(&self) -> Rc<RefCell<dyn FrameDecoder>>;

    /// Render a burst; `Ok(None)` when there is nothing to send, `Err` with what would not
    /// modulate.
    ///
    /// # Errors
    /// A frame the air cannot modulate.
    fn render(&mut self, frames: &[TxFrame]) -> Result<Option<Rendered>, String>;

    /// Whether someone else is on the frequency.
    fn sense(&self) -> &dyn ChannelSense;
    /// The same, to tell it what the station knows.
    fn sense_mut(&mut self) -> &mut dyn ChannelSense;

    /// Build the receive front end again from nothing, after captured audio was lost.
    fn restart_front_end(&mut self);
    /// Move on past `samples` of lost captured audio and start the receiver again after it.
    fn lose_captured(&mut self, samples: usize);

    /// The same air in another bandwidth (ADR-0026), carrying over what carries; `None` where
    /// the air runs one bandwidth.
    fn moved_to(&mut self, _bandwidth_hz: usize, _busy: BusyConfig) -> Option<Box<dyn Air>> {
        None
    }
    /// Where the receive mixer is centred, hertz from the dial (ADR-0079); 0 on an air that
    /// does not follow an offset.
    fn receive_offset_hz(&self) -> f64 {
        0.0
    }
    /// Centre the receive mixer `hz` from the dial, on an air that can.
    fn set_receive_offset_hz(&mut self, _hz: f64) {}
}
