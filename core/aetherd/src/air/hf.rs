//! Aether HF's air: the OFDM and tone-floor waveforms of `aether-phy`, at 2 300 or 500 Hz.
//!
//! Everything here was the station's own before ADR-0113 and works exactly as it did: the
//! station's audio goes to baseband through [`AudioToBaseband`], the streaming receiver finds
//! frames in it, the decoder is shared with every frame handed to the link engine, and a burst
//! is modulated in baseband and brought back to audio through [`BasebandToAudio`].

use std::{cell::RefCell, rc::Rc};

use aether_phy::{
    AudioToBaseband, BasebandToAudio, Complex, DecodedFrame, Modem, PendingFrame,
    StreamingReceiver,
    modes::{AirInterface, air_interface},
    waveform::WaveformParams,
};

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
}

impl HfAir {
    /// The air for `params`, from nothing.
    #[must_use]
    pub fn new(params: WaveformParams) -> Self {
        Self {
            params,
            receiver: StreamingReceiver::new(params, 6.0, true),
            decoder: Rc::new(RefCell::new(Modem::new(params, false))),
            transmitter: Modem::new(params, false),
            to_audio: BasebandToAudio::new(params),
            from_audio: AudioToBaseband::new(params),
            rx_origin: 0,
            baseband_seen: 0,
        }
    }

    /// The air for `params` built again on a move to the other bandwidth (ADR-0026): every
    /// part new, the station's baseband count carried on, the new receiver counting from it.
    #[must_use]
    pub fn moved_to(&self, params: WaveformParams) -> Self {
        Self {
            rx_origin: self.baseband_seen,
            baseband_seen: self.baseband_seen,
            ..Self::new(params)
        }
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

    /// Captured audio to baseband.
    pub fn downconvert(&mut self, audio: &[f32]) -> Vec<Complex> {
        self.from_audio.process(audio)
    }

    /// Feed the receiver a block of baseband, muted or heard, and take what it found.
    pub fn listen(&mut self, baseband: &[Complex]) -> Listened {
        let origin = self.rx_origin;
        let frames = self.receiver.feed(baseband);
        let preambles = self.receiver.take_preambles();
        self.baseband_seen += baseband.len();
        Listened {
            origin,
            frames,
            preambles,
        }
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
