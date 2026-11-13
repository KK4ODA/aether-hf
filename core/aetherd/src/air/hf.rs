//! Aether HF's air: the OFDM and tone-floor waveforms of `aether-phy`, at 2 300 or 500 Hz.
//!
//! Everything here was the station's own before ADR-0113 and works exactly as it did: the
//! station's audio goes to baseband through [`AudioToBaseband`], the streaming receiver finds
//! frames in it, the decoder is shared with every frame handed to the link engine, and a burst
//! is modulated in baseband and brought back to audio through [`BasebandToAudio`]. The busy
//! detector, which reads the channel in the same baseband, is the air's too.

use std::{any::Any, cell::RefCell, collections::VecDeque, rc::Rc};

use aether_link::{Container, HarqBuffer, PhyTiming, TxFrame};
use aether_phy::{
    AudioToBaseband, BasebandToAudio, Complex, DecodedFrame, Modem, PendingFrame, Received,
    StreamingReceiver,
    modes::{AirInterface, air_interface},
    waveform::WaveformParams,
};

use super::{
    Air, AirInfo, Arrival, ChannelSense, FrameDecoder, Heard, HeardFrame, Listening, Rendered,
    RungInfo,
};
use crate::busy::{BusyConfig, BusyDetector, BusyReading, BusyReason, BusyState};

/// A piece of the block the receiver has still to be fed.
#[derive(Debug, Clone, Copy)]
enum Piece {
    /// Silence this long, in baseband samples: this station's own transmission.
    Muted(usize),
    /// The block from this baseband sample on.
    Heard(usize),
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
    /// Where the receiver's positions start in the station's baseband count: a move to the
    /// other bandwidth or a loss of captured audio builds another receiver, and what this one
    /// found is placed by its own count.
    rx_origin: usize,
    /// Baseband samples the station has heard since it started, lost ones included.
    baseband_seen: usize,
    /// The channel's occupancy, read in this air's baseband (ADR-0078, ADR-0088).
    busy: BusyDetector,
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

    /// A frame the receiver finished with, read out on the station's clock; `origin` is where
    /// the receiver that found it counts from in the station's baseband.
    #[must_use]
    pub fn read_out(&self, decoded: DecodedFrame, origin: usize) -> HeardFrame {
        let air = self.interface();
        let fs = self.params.fs_baseband;
        let frame = &decoded.frame;
        HeardFrame {
            control: frame.is_control(),
            floor: frame.is_tone(),
            rung: frame.rung(&air),
            start_s: (origin + frame.start()) as f64 / fs,
            length_s: frame.samples(&air) as f64 / fs,
            rv: frame.rv(),
            snr_3k_db: frame.snr_3k_db(),
            cfo_hz: frame.cfo_hz(),
            mode_confidence: frame.mode_confidence(),
            detect_confidence: frame.detect_confidence(&air),
            follows: frame.follows(),
            echo: frame.echo(),
            symbols: frame
                .ofdm()
                .map_or_else(Vec::new, |ofdm| ofdm.symbols.clone()),
            payload: decoded.payload,
            soft: Box::new(decoded.frame),
        }
    }

    /// A frame announced, on the station's clock.
    fn arrival(&self, pending: &PendingFrame, origin: usize) -> Arrival {
        let fs = self.params.fs_baseband;
        Arrival {
            start_s: (origin + pending.start) as f64 / fs,
            end_s: (origin + pending.end) as f64 / fs,
            length_s: pending.end.saturating_sub(pending.start) as f64 / fs,
            floor: pending.tone,
            detect_confidence: pending.detect_confidence,
        }
    }

    /// A burst's baseband to audio, the filter's tail included.
    fn upconvert(&mut self, baseband: &[Complex]) -> Vec<f32> {
        let mut rendered = self.to_audio.process(baseband);
        rendered.extend(self.to_audio.flush());
        rendered
    }
}

impl Air for HfAir {
    fn info(&self) -> AirInfo {
        AirInfo {
            audio_rate: self.params.audio_rate,
            bandwidth_hz: self.params.bandwidth.hz(),
            data_frame_s: self.interface().long.duration_s(),
            rungs: self.interface().n_rungs(),
        }
    }

    fn timing(&self) -> PhyTiming {
        phy_timing(self.params)
    }

    fn ladder(&self) -> Vec<RungInfo> {
        self.interface()
            .ladder()
            .iter()
            .map(|rung| RungInfo {
                name: rung.name(),
                payload_bytes: rung.payload_bytes(),
                duration_s: rung.duration_s(),
            })
            .collect()
    }

    /// Deaf while transmitting: the receiver is still fed — with silence, the same length.
    /// Skipping the blocks outright would splice the stream either side of the transmission
    /// together, and a frame that straddled the join would be assembled from two pieces of
    /// signal that were never adjacent. Muted is not the same as paused. The deafness ends
    /// where the transmission did, though, not where the loop noticed: the card's clock says
    /// how much of the block came in after the last sample had left, and that much is heard.
    fn take_block(&mut self, audio: &[f32], how: Listening, now: f64) -> Option<f64> {
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

    fn listen_next(&mut self) -> Heard {
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
        Heard {
            frames: frames
                .into_iter()
                .map(|decoded| self.read_out(decoded, origin))
                .collect(),
            arrivals: preambles
                .iter()
                .map(|pending| self.arrival(pending, origin))
                .collect(),
        }
    }

    fn block_power(&self) -> Option<f64> {
        (!self.block.is_empty()).then(|| {
            self.block.iter().map(|&(i, q)| i * i + q * q).sum::<f64>() / self.block.len() as f64
        })
    }

    fn rung_of(&self, frame: &HeardFrame) -> Option<usize> {
        frame
            .soft
            .downcast_ref::<Received>()
            .and_then(|received| received.rung(&self.interface()))
    }

    fn decoder(&self) -> Rc<RefCell<dyn FrameDecoder>> {
        Rc::clone(&self.decoder) as Rc<RefCell<dyn FrameDecoder>>
    }

    fn render(&mut self, frames: &[TxFrame]) -> Result<Option<Rendered>, String> {
        let mut baseband: Vec<Complex> = Vec::new();
        let mut first = 0;
        for frame in frames {
            let burst = match frame.container {
                // a rung of the ladder: the tone floor's, at its peak, or an OFDM mode; an OFDM
                // frame says how many of the burst follow it (ADR-0041)
                Container::Data => self.transmitter.rung_burst_following(
                    &frame.payload,
                    frame.mode,
                    frame.rv,
                    frame.follows,
                ),
                Container::Control => {
                    self.transmitter
                        .control_burst_of(&frame.payload, frame.rv, frame.floor)
                }
            };
            match burst {
                Ok(samples) => {
                    baseband.extend(samples);
                    if first == 0 {
                        first = baseband.len();
                    }
                }
                // The engine only ever asks for payloads it sized from this same table, so
                // this cannot happen without a bug; dropping the burst keeps a bug off the
                // air rather than transmitting something malformed.
                Err(error) => return Err(error.to_string()),
            }
        }
        if baseband.is_empty() {
            return Ok(None);
        }
        // flush the chain after the burst, or its filters keep the tail of the last frame
        let audio = self.upconvert(&baseband);
        let first_frame = audio.len() * first / baseband.len();
        Ok(Some(Rendered { audio, first_frame }))
    }

    fn sense(&self) -> &dyn ChannelSense {
        &self.busy
    }

    fn sense_mut(&mut self) -> &mut dyn ChannelSense {
        &mut self.busy
    }

    fn restart_front_end(&mut self) {
        self.from_audio = AudioToBaseband::new(self.params);
    }

    fn lose_captured(&mut self, samples: usize) {
        let rate = self.params.audio_rate as f64;
        self.baseband_seen += (samples as f64 * self.params.fs_baseband / rate).round() as usize;
        self.receiver = StreamingReceiver::new(self.params, 6.0, true);
        self.rx_origin = self.baseband_seen;
    }

    /// Every part new, the station's baseband count carried on, the new receiver counting from
    /// it. The busy detector learns its floor again through the new band filter, at the
    /// threshold it had, and keeps the radio's levels on quiet (ADR-0078 §5). What is left of
    /// the block being heard goes to the new receiver: a move made while a block is heard in
    /// pieces.
    fn moved_to(&mut self, bandwidth_hz: usize, busy: BusyConfig) -> Option<Box<dyn Air>> {
        let params = crate::station::params_for(bandwidth_hz)?;
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
        Some(Box::new(moved))
    }

    fn receive_offset_hz(&self) -> f64 {
        self.from_audio.offset_hz()
    }

    fn set_receive_offset_hz(&mut self, hz: f64) {
        self.from_audio.set_offset_hz(hz);
    }
}

impl FrameDecoder for Modem {
    fn decode(
        &mut self,
        frame: &dyn Any,
        buffer: Option<&HarqBuffer>,
    ) -> (Option<Vec<u8>>, HarqBuffer) {
        let Some(frame) = frame.downcast_ref::<Received>() else {
            return (None, Vec::new());
        };
        match self.decode_received(frame, buffer.map(Vec::as_slice)) {
            Ok((payload, llrs)) => (payload, llrs),
            // A frame the codec will not even look at is not a frame; there is nothing to
            // keep for a later combine either.
            Err(_) => (None, Vec::new()),
        }
    }
}

impl ChannelSense for BusyDetector {
    fn busy(&self, now: f64) -> bool {
        Self::busy(self, now)
    }

    fn settled(&self) -> bool {
        Self::settled(self)
    }

    fn reason(&self) -> Option<BusyReason> {
        Self::reason(self)
    }

    fn level_db(&self) -> f64 {
        self.level_db
    }

    fn floor_db(&self) -> f64 {
        self.floor_db
    }

    fn excess_peak_db(&self) -> f64 {
        Self::excess_peak_db(self)
    }

    fn shape_db(&self) -> f64 {
        self.shape_db
    }

    fn gain_db(&self) -> Option<f64> {
        Self::gain_db(self)
    }

    fn threshold_db(&self) -> f64 {
        self.config().threshold_db
    }

    fn set_threshold_db(&mut self, threshold_db: f64) {
        Self::set_threshold_db(self, threshold_db);
    }

    fn explain(&mut self, from: f64, until: f64) {
        Self::explain(self, from, until);
    }

    fn mark_frame(&mut self, now: f64, detect_confidence: f64) {
        Self::mark_frame(self, now, detect_confidence);
    }

    fn retune(&mut self, now: f64) {
        Self::retune(self, now);
    }

    fn reset_excess_peak(&mut self) {
        Self::reset_excess_peak(self);
    }

    fn state(&self, now: f64) -> BusyState {
        Self::state(self, now)
    }

    fn reading(&self, now: f64) -> BusyReading {
        Self::reading(self, now)
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

/// Link-layer timing derived from the waveform tables — what the model's harness
/// (`phy_timing`) hands its engines.
///
/// Every number here comes from [`WaveformParams`] or a measurement, never a guess: the frame
/// durations are what the frames actually occupy, and the capacities are what the rungs of
/// the air's ladder — the tone floor's two, then its OFDM modes (ADR-0013) — actually carry.
///
/// # Panics
/// If the air's tone floor has data kinds of different lengths, which the link layer cannot
/// time: the fixed tables never have.
#[must_use]
pub fn phy_timing(params: WaveformParams) -> PhyTiming {
    let air = aether_phy::modes::air_interface(params);
    let narrow = params.bandwidth == aether_phy::waveform::Bandwidth::Narrow500;
    // the thresholds the air's benchmarks measured, so the rate controller steps whichever
    // ladder the air has (empty: the wide one), and its control frames' for the family
    let (thresholds, controls) = if narrow {
        (
            aether_link::rate::NARROW_AWGN_THRESHOLD_DB.to_vec(),
            aether_link::rate::NARROW_CONTROL_THRESHOLD_DB,
        )
    } else {
        (Vec::new(), aether_link::rate::CONTROL_THRESHOLD_DB)
    };
    let floor_s = air.tone_data()[0].duration_s();
    assert!(
        air.tone_data()
            .iter()
            .all(|k| (k.duration_s() - floor_s).abs() < 1e-12),
        "the link layer takes one floor data-frame length"
    );
    PhyTiming {
        data_frame_s: air.long.duration_s(),
        control_frame_s: air.short.duration_s(),
        // PTT, the radio's own transmit delay, and the audio buffers at both ends
        turnaround_s: 0.25,
        detect_latency_s: 0.15,
        // the station fills this in from its keying lead and the daemon's playback backlog
        tx_latency_s: 0.0,
        // acquisition reports an OFDM frame once its preamble is in, plus the sidelobe guard
        // and the search block: four symbols
        preamble_detect_s: Some(
            (aether_phy::PREAMBLE_SYMBOLS + 2) as f64 * params.symbol_period_s(),
        ),
        data_capacity: air
            .ladder()
            .iter()
            .map(aether_phy::Rung::payload_bytes)
            .collect(),
        mode_threshold_db: thresholds,
        // the tone floor (ADR-0013): its frames' air times and how many rungs are its
        floor_data_frame_s: Some(floor_s),
        floor_control_frame_s: Some(air.tone_control().duration_s()),
        floor_modes: air.floor_modes(),
        control_threshold_db: Some(controls),
        // a tone frame is announced once its first sync block is in and has beaten its
        // neighbours, plus this receiver's own lateness (blanker, filter and a block)
        floor_preamble_detect_s: Some(aether_phy::tone::announce_delay_s(0.1)),
        // rungs a path's echo breaks, against what the tone floor's frames read of it (ADR-0087)
        echo_limit: if narrow {
            aether_link::rate::NARROW_ECHO_LIMIT.to_vec()
        } else {
            aether_link::rate::ECHO_LIMIT.to_vec()
        },
    }
}
