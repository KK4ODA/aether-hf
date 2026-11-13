//! Aether HF's air: the OFDM and tone-floor waveforms of `aether-phy`, at 2 300 or 500 Hz.
//!
//! Everything here was the station's own before ADR-0113 and works exactly as it did: the
//! station's audio goes to baseband through [`AudioToBaseband`], the streaming receiver finds
//! frames in it, the decoder is shared with every frame handed to the link engine, and a burst
//! is modulated in baseband and brought back to audio through [`BasebandToAudio`]. The busy
//! detector, which reads the channel in the same baseband, is the air's too.

use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use aether_phy::{
    AudioToBaseband, BasebandToAudio, Complex, DecodedFrame, Modem, PendingFrame,
    StreamingReceiver,
    modes::{AirInterface, air_interface},
    waveform::WaveformParams,
};

use crate::busy::{BusyConfig, BusyDetector};

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
    /// Listening: the receiver hears everything; the busy detector only when `sense`, which is
    /// false in the deafness a lead after the key is released.
    Hearing {
        /// Whether the busy detector hears this block.
        sense: bool,
    },
}

/// A piece of the block the receiver has still to be fed.
#[derive(Debug, Clone, Copy)]
enum Piece {
    /// Silence this long, in baseband samples: this station's own transmission.
    Muted(usize),
    /// The block from this baseband sample on.
    Heard(usize),
}

/// What the receiver found in a block of baseband.
pub struct Listened {
    /// Where the receiver's sample positions start, in the station's baseband count: a move to
    /// the other bandwidth or a loss of captured audio builds another receiver, and what this
    /// one found is placed by its own count.
    pub origin: usize,
    /// Frames the receiver finished with, decoded or not.
    pub frames: Vec<DecodedFrame>,
    /// Frames announced on their preamble, still arriving.
    pub preambles: Vec<PendingFrame>,
}

/// HF's physical layer for one waveform.
pub struct HfAir {
    params: WaveformParams,
    receiver: StreamingReceiver,
    /// Shared with every frame handed to the engine, for decoding and combining.
    decoder: Rc<RefCell<Modem>>,
    transmitter: Modem,
    to_audio: BasebandToAudio,
    from_audio: AudioToBaseband,
    /// Where the receiver's positions start in the station's baseband count.
    rx_origin: usize,
    /// Baseband samples the station has heard since it started, lost ones included.
    baseband_seen: usize,
    /// The channel's occupancy, read in this air's baseband (ADR-0078, ADR-0088).
    pub(crate) busy: BusyDetector,
    /// The block of baseband taken last, and the pieces of it the receiver has still to be fed.
    block: Vec<Complex>,
    pieces: VecDeque<Piece>,
}

impl HfAir {
    /// The air for `params`, from nothing; `busy` the busy detector's settings, whose band
    /// this air sets.
    #[must_use]
    pub fn new(params: WaveformParams, busy: BusyConfig) -> Self {
        Self {
            params,
            receiver: StreamingReceiver::new(params, 6.0, true),
            decoder: Rc::new(RefCell::new(Modem::new(params, false))),
            transmitter: Modem::new(params, false),
            to_audio: BasebandToAudio::new(params),
            from_audio: AudioToBaseband::new(params),
            rx_origin: 0,
            baseband_seen: 0,
            busy: BusyDetector::new(BusyConfig {
                fs: params.fs_baseband,
                passband_hz: params.occupied_bandwidth_hz(),
                centre_hz: params.centre_hz,
                ..busy
            }),
            block: Vec::new(),
            pieces: VecDeque::new(),
        }
    }

    /// The air for `params` built again on a move to the other bandwidth (ADR-0026): every
    /// part new, the station's baseband count carried on, the new receiver counting from it.
    /// The busy detector learns its floor again through the new band filter, at the threshold
    /// it had, and keeps the radio's levels on quiet (ADR-0078 §5). What is left of the block
    /// being heard goes to the new receiver: a move made while a block is heard in pieces.
    #[must_use]
    pub fn moved_to(&mut self, params: WaveformParams, busy: BusyConfig) -> Self {
        let mut moved = Self::new(
            params,
            BusyConfig {
                threshold_db: self.busy.config().threshold_db,
                ..busy
            },
        );
        moved.busy.keep_levels_from(&self.busy);
        moved.rx_origin = self.baseband_seen;
        moved.baseband_seen = self.baseband_seen;
        moved.block = std::mem::take(&mut self.block);
        moved.pieces = std::mem::take(&mut self.pieces);
        moved
    }

    /// The numerology.
    #[must_use]
    pub const fn params(&self) -> WaveformParams {
        self.params
    }

    /// The air interface: the ladder, the layouts, the acquisition threshold.
    #[must_use]
    pub fn interface(&self) -> AirInterface {
        air_interface(self.params)
    }

    /// Take a block of captured audio: to baseband, the busy detector fed or spared, and the
    /// block cut into what the receiver hears muted and what it hears as it came. Returns,
    /// when the block holds the end of this station's transmission, the time it ended: the
    /// station ends the transmission between the two pieces ([`Self::listen_next`]).
    ///
    /// Deaf while transmitting: the receiver is still fed — with silence, the same length.
    /// Skipping the blocks outright would splice the stream either side of the transmission
    /// together, and a frame that straddled the join would be assembled from two pieces of
    /// signal that were never adjacent. Muted is not the same as paused. The deafness ends
    /// where the transmission did, though, not where the loop noticed: the card's clock says
    /// how much of the block came in after the last sample had left, and that much is heard.
    pub fn take_block(&mut self, audio: &[f32], how: Listening, now: f64) -> Option<f64> {
        let baseband = self.from_audio.process(audio);
        self.pieces.clear();
        let ended = match how {
            Listening::Transmitting {
                past,
                playback_empty,
            } => {
                self.busy.skip(&baseband);
                let after = after_transmission(past, audio.len(), baseband.len());
                if after == 0 || !playback_empty {
                    self.pieces.push_back(Piece::Muted(baseband.len()));
                    None
                } else {
                    let during = baseband.len() - after;
                    self.pieces.push_back(Piece::Muted(during));
                    self.pieces.push_back(Piece::Heard(during));
                    Some(now - after as f64 * self.params.fs_baseband.recip())
                }
            }
            Listening::Hearing { sense } => {
                if sense {
                    // the whole passband first: the radio's gain, which the channel's own
                    // blocks are judged at (ADR-0078)
                    self.busy
                        .hear_passband(audio, self.params.audio_rate as f64, now);
                    self.busy.push(&baseband, now);
                } else {
                    self.busy.skip(&baseband);
                }
                self.pieces.push_back(Piece::Heard(0));
                None
            }
        };
        self.block = baseband;
        ended
    }

    /// Feed the receiver the next piece of the block taken last, and take what it found.
    pub fn listen_next(&mut self) -> Listened {
        let origin = self.rx_origin;
        let (frames, fed) = match self.pieces.pop_front().unwrap_or(Piece::Muted(0)) {
            Piece::Muted(samples) => (self.receiver.feed(&vec![(0.0, 0.0); samples]), samples),
            Piece::Heard(from) => {
                let heard = self.block.get(from..).unwrap_or_default();
                (self.receiver.feed(heard), heard.len())
            }
        };
        let preambles = self.receiver.take_preambles();
        self.baseband_seen += fed;
        Listened {
            origin,
            frames,
            preambles,
        }
    }

    /// The mean power of the block taken last, in its baseband; `None` for an empty block.
    #[must_use]
    pub fn block_power(&self) -> Option<f64> {
        (!self.block.is_empty()).then(|| {
            self.block.iter().map(|&(i, q)| i * i + q * q).sum::<f64>() / self.block.len() as f64
        })
    }

    /// The decoder every frame handed to the engine decodes through.
    #[must_use]
    pub fn decoder(&self) -> Rc<RefCell<Modem>> {
        Rc::clone(&self.decoder)
    }

    /// The modulator.
    pub const fn transmitter(&mut self) -> &mut Modem {
        &mut self.transmitter
    }

    /// A burst's baseband to audio, the filter's tail included.
    pub fn upconvert(&mut self, baseband: &[Complex]) -> Vec<f32> {
        let mut rendered = self.to_audio.process(baseband);
        rendered.extend(self.to_audio.flush());
        rendered
    }

    /// Where the receive mixer is centred, hertz from the dial (ADR-0079).
    #[must_use]
    pub const fn offset_hz(&self) -> f64 {
        self.from_audio.offset_hz()
    }

    /// Centre the receive mixer `hz` from the dial (ADR-0079).
    pub const fn set_offset_hz(&mut self, hz: f64) {
        self.from_audio.set_offset_hz(hz);
    }

    /// Build the receive front end again from nothing, after captured audio was lost.
    pub fn restart_front_end(&mut self) {
        self.from_audio = AudioToBaseband::new(self.params);
    }

    /// Move the baseband count on past `samples` of lost captured audio and start the receiver
    /// again after the gap: a frame across it is lost with the audio.
    pub fn lose_captured(&mut self, samples: usize) {
        let rate = self.params.audio_rate as f64;
        self.baseband_seen += (samples as f64 * self.params.fs_baseband / rate).round() as usize;
        self.receiver = StreamingReceiver::new(self.params, 6.0, true);
        self.rx_origin = self.baseband_seen;
    }

    /// Where the current receiver's positions start.
    #[must_use]
    pub const fn rx_origin(&self) -> usize {
        self.rx_origin
    }

    /// A position of the current receiver's, in seconds on the station's clock.
    #[must_use]
    pub fn seconds_at(&self, position: usize) -> f64 {
        (self.rx_origin + position) as f64 / self.params.fs_baseband
    }

    /// The baseband-to-audio chain, for tests that render as the station does.
    #[cfg(test)]
    pub(crate) const fn upconverter(&self) -> &BasebandToAudio {
        &self.to_audio
    }
}

/// How many of a block's `baseband` samples came in after the transmission now running had
/// wholly left the sound card, `past` audio samples ago on its clock. The card's playback clock
/// and the capture run at one rate, so what the card has played past the transmission's last
/// sample is what the capture holds past it — near enough: a card captures its own tail a
/// little later still, and that is silence either way. `captured` audio samples became
/// `baseband` baseband ones.
fn after_transmission(past: usize, captured: usize, baseband: usize) -> usize {
    if past == 0 || captured == 0 {
        return 0;
    }
    let after = past.min(captured) as f64 * baseband as f64 / captured as f64;
    (after.round() as usize).min(baseband)
}
