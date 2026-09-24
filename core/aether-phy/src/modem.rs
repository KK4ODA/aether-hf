//! Payload in, burst out; buffer in, decoded frames out.
//!
//! A thin tie between the frame codec, transmitter, detector and receiver. Everything here is
//! a convenience over parts that already work on their own; the point is that a caller — the
//! daemon, a benchmark, a test — does not have to re-derive which codec goes with which
//! layout, or remember that a close chip decision is worth a second attempt. Both families of
//! the air go through here: the OFDM frames, and the tone floor's (ADR-0013) — a rung of the
//! ladder says which.

use std::collections::HashMap;

use aether_fec::FecError;

use crate::{
    Complex,
    blanker::NoiseBlanker,
    codec::FrameCodec,
    constellation::NoiseVar,
    modes::{AirInterface, FrameLayout, Mode, Rung, air_interface},
    ofdm::DemodError,
    passband::band_limit_taps,
    preamble::{FrameHeader, FrameType, HeaderError},
    rx::{FrameReceiver, FrameSync, ReceivedFrame},
    sync::FrameDetector,
    tone::{self, ToneCodec, ToneDetector, ToneFrame, ToneKind, ToneSync},
    tx::{FrameTransmitter, TxError},
    waveform::{WIDE_2300, WaveformParams},
};

/// Anything that can go wrong on the way through the modem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModemError {
    /// A payload was the wrong length, or a codeword did not fit.
    Fec(FecError),
    /// The wrong number of constellation symbols for a layout.
    Tx(TxError),
    /// A mode or redundancy version outside what the preamble can signal.
    Header(HeaderError),
    /// A frame ran past the end of the buffer.
    Demod(DemodError),
}

impl core::fmt::Display for ModemError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Fec(e) => write!(f, "{e}"),
            Self::Tx(e) => write!(f, "{e}"),
            Self::Header(e) => write!(f, "{e}"),
            Self::Demod(e) => write!(f, "{e}"),
        }
    }
}

impl core::error::Error for ModemError {}

impl From<FecError> for ModemError {
    fn from(e: FecError) -> Self {
        Self::Fec(e)
    }
}

impl From<TxError> for ModemError {
    fn from(e: TxError) -> Self {
        Self::Tx(e)
    }
}

impl From<HeaderError> for ModemError {
    fn from(e: HeaderError) -> Self {
        Self::Header(e)
    }
}

impl From<DemodError> for ModemError {
    fn from(e: DemodError) -> Self {
        Self::Demod(e)
    }
}

/// Below this chip-metric ratio a failed check is worth retrying with the runner-up mode.
///
/// Above it the decision was not close, and a retry only costs time.
pub const MODE_RETRY_CONFIDENCE: f64 = 1.3;

/// A frame's soft information, of either family: what a link layer combines across
/// retransmissions.
#[derive(Debug, Clone)]
pub enum Received {
    /// An OFDM frame: equalised symbols, and the mode and redundancy version its chips named.
    Ofdm(ReceivedFrame),
    /// A tone-floor frame (ADR-0013): its soft bits, and where the detector found it — the
    /// kind and redundancy version its sync pattern named.
    Tone(ToneFrame, ToneSync),
}

impl Received {
    /// Where the frame starts, in samples of the band-limited stream.
    #[must_use]
    pub fn start(&self) -> usize {
        match self {
            Self::Ofdm(frame) => frame.sync.start,
            Self::Tone(_, sync) => sync.start,
        }
    }

    /// Whether it is a control frame (an acknowledgement, a poll, a disconnect).
    #[must_use]
    pub fn is_control(&self) -> bool {
        match self {
            Self::Ofdm(frame) => frame.sync.frame_type == FrameType::Control,
            Self::Tone(_, sync) => sync.kind.control,
        }
    }

    /// Whether it is the tone floor's.
    #[must_use]
    pub const fn is_tone(&self) -> bool {
        matches!(self, Self::Tone(..))
    }

    /// The redundancy version it announced.
    #[must_use]
    pub fn rv(&self) -> u8 {
        match self {
            Self::Ofdm(frame) => frame.rv,
            Self::Tone(_, sync) => sync.rv,
        }
    }

    /// Its SNR, referenced to 3 kHz and to an OFDM frame's average power: a tone frame's is
    /// taken back by the gain it goes out at, so both families read in one currency.
    #[must_use]
    pub fn snr_3k_db(&self) -> f64 {
        match self {
            Self::Ofdm(frame) => frame.snr_3k_db,
            Self::Tone(frame, _) => frame.snr_db,
        }
    }

    /// The carrier offset removed.
    #[must_use]
    pub fn cfo_hz(&self) -> f64 {
        match self {
            Self::Ofdm(frame) => frame.cfo_hz,
            Self::Tone(_, sync) => sync.cfo_hz,
        }
    }

    /// How sure the mode decision was: the chip metric over its runner-up for an OFDM DATA
    /// frame. A control frame has no chips, and a tone frame's kind is named by the sync
    /// pattern acquisition matched, so both report 1.0.
    #[must_use]
    pub fn mode_confidence(&self) -> f64 {
        match self {
            Self::Ofdm(frame) => frame.mode_confidence,
            Self::Tone(..) => 1.0,
        }
    }

    /// How far above the threshold that accepted it acquisition saw this frame.
    #[must_use]
    pub fn detect_confidence(&self, air: &AirInterface) -> f64 {
        match self {
            Self::Ofdm(frame) => frame.sync.detect_confidence(air),
            Self::Tone(_, sync) => sync.detect_confidence(),
        }
    }

    /// The rung of the ladder a DATA frame was sent at; `None` for a control frame, and for
    /// an OFDM frame whose chips name a mode on no rung — noise, or a station on another
    /// version.
    #[must_use]
    pub fn rung(&self, air: &AirInterface) -> Option<usize> {
        if self.is_control() {
            return None;
        }
        match self {
            Self::Ofdm(frame) => air.rung_of(frame.mode),
            Self::Tone(_, sync) => air.tone_data().iter().position(|k| k == sync.kind),
        }
    }

    /// The frame's length in samples.
    #[must_use]
    pub fn samples(&self, air: &AirInterface) -> usize {
        match self {
            Self::Ofdm(_) => air.layout_for(!self.is_control()).samples(),
            Self::Tone(_, sync) => sync.kind.samples(),
        }
    }

    /// The OFDM frame, if it is one.
    #[must_use]
    pub const fn ofdm(&self) -> Option<&ReceivedFrame> {
        match self {
            Self::Ofdm(frame) => Some(frame),
            Self::Tone(..) => None,
        }
    }
}

/// A frame the receiver got all the way through.
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    /// The payload, or `None` if the check failed.
    pub payload: Option<Vec<u8>>,
    /// The soft frame behind it, which a link layer can combine.
    pub frame: Received,
}

impl DecodedFrame {
    /// Whether the payload came through.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.payload.is_some()
    }
}

/// Transmit and receive front for one waveform. Codecs are built once per mode and layout.
pub struct Modem {
    params: WaveformParams,
    air: AirInterface,
    tx: FrameTransmitter,
    detector: FrameDetector,
    rx: FrameReceiver,
    tone_detector: ToneDetector,
    /// Impulse blanker run ahead of band-limiting (P2-5). On by default: measured to cost a
    /// clean channel nothing at any mode while removing impulsive noise that otherwise takes
    /// the link to 100 % frame errors.
    blanker: Option<NoiseBlanker>,
    band_taps: Vec<f64>,
    codecs: HashMap<(usize, &'static str), FrameCodec>,
    tone_codecs: HashMap<&'static str, ToneCodec>,
}

impl std::fmt::Debug for Modem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Modem")
            .field("params", &self.params)
            .field("blanking", &self.blanker.is_some())
            .finish_non_exhaustive()
    }
}

impl Default for Modem {
    fn default() -> Self {
        Self::new(WIDE_2300, true)
    }
}

impl Modem {
    /// Build a modem for a numerology, with or without impulse blanking.
    #[must_use]
    pub fn new(params: WaveformParams, blank_impulses: bool) -> Self {
        Self {
            tx: FrameTransmitter::new(params),
            detector: FrameDetector::new(params),
            rx: FrameReceiver::new(params),
            tone_detector: ToneDetector::new(),
            blanker: blank_impulses.then(NoiseBlanker::default),
            band_taps: band_limit_taps(&params),
            codecs: HashMap::new(),
            tone_codecs: HashMap::new(),
            air: air_interface(params),
            params,
        }
    }

    /// Use a blanker other than the default, or none.
    #[must_use]
    pub fn with_blanker(mut self, blanker: Option<NoiseBlanker>) -> Self {
        self.blanker = blanker;
        self
    }

    /// The numerology.
    #[must_use]
    pub fn params(&self) -> WaveformParams {
        self.params
    }

    /// The air interface in use: the layouts, mode table and ladder of this waveform.
    #[must_use]
    pub fn air(&self) -> AirInterface {
        self.air
    }

    /// The OFDM mode table of this waveform, most robust first.
    #[must_use]
    pub fn modes(&self) -> &'static [Mode] {
        self.air.modes
    }

    /// The detector, for a caller that wants to acquire on its own.
    #[must_use]
    pub fn detector(&self) -> &FrameDetector {
        &self.detector
    }

    /// The tone floor's detector.
    #[must_use]
    pub fn tone_detector(&self) -> &ToneDetector {
        &self.tone_detector
    }

    /// The receiver.
    #[must_use]
    pub fn receiver(&self) -> &FrameReceiver {
        &self.rx
    }

    /// The blanker, if one is fitted.
    #[must_use]
    pub fn blanker(&self) -> Option<&NoiseBlanker> {
        self.blanker.as_ref()
    }

    /// The band-limiting filter taps this modem conditions with.
    #[must_use]
    pub fn band_taps(&self) -> &[f64] {
        &self.band_taps
    }

    /// The codec for a (mode, layout) pair, built on first use.
    fn codec(&mut self, mode: Mode, layout: FrameLayout) -> Result<&FrameCodec, FecError> {
        let key = (mode.index, layout.name);
        if let std::collections::hash_map::Entry::Vacant(slot) = self.codecs.entry(key) {
            slot.insert(FrameCodec::new(mode, layout)?);
        }
        Ok(&self.codecs[&key])
    }

    /// The codec for a tone-floor kind, built on first use.
    fn tone_codec(&mut self, kind: &'static ToneKind) -> Result<&ToneCodec, FecError> {
        if let std::collections::hash_map::Entry::Vacant(slot) = self.tone_codecs.entry(kind.name) {
            slot.insert(ToneCodec::new(kind)?);
        }
        Ok(&self.tone_codecs[kind.name])
    }

    // ── transmit ──────────────────────────────────────────────────────

    /// Baseband for one OFDM data frame at `mode`, on the LONG layout.
    ///
    /// # Errors
    /// If the payload is the wrong length for the mode.
    pub fn data_burst(
        &mut self,
        payload: &[u8],
        mode: Mode,
        rv: u8,
    ) -> Result<Vec<Complex>, ModemError> {
        let layout = self.air.long;
        let qam = self.codec(mode, layout)?.encode(payload, rv)?;
        let header = FrameHeader::new(FrameType::Data, mode.index, rv)?;
        Ok(self.tx.baseband(&header, &layout, &qam)?)
    }

    /// Baseband for one data frame at a rung of the ladder: a tone-floor kind — at
    /// [`tone::gain_db`] above an OFDM frame, its peak — or an OFDM mode.
    ///
    /// # Errors
    /// If the payload is the wrong length for the rung.
    ///
    /// # Panics
    /// If the ladder has no such rung.
    pub fn rung_burst(
        &mut self,
        payload: &[u8],
        rung: usize,
        rv: u8,
    ) -> Result<Vec<Complex>, ModemError> {
        match self.air.rung(rung) {
            Rung::Tone(kind) => Ok(tone::burst(self.tone_codec(kind)?, payload, rv)?),
            Rung::Ofdm(mode, _) => self.data_burst(payload, mode, rv),
        }
    }

    /// Baseband for one ordinary control frame.
    ///
    /// # Errors
    /// If the payload is the wrong length for the control mode.
    pub fn control_burst(&mut self, payload: &[u8], rv: u8) -> Result<Vec<Complex>, ModemError> {
        self.control_burst_of(payload, rv, false)
    }

    /// Baseband for one control frame: SHORT at the control mode, or — while the link runs
    /// the floor — the tone floor's control frame (ADR-0013). A short payload is padded with
    /// zeros.
    ///
    /// # Errors
    /// If the payload is too long for the control frame.
    pub fn control_burst_of(
        &mut self,
        payload: &[u8],
        rv: u8,
        floor: bool,
    ) -> Result<Vec<Complex>, ModemError> {
        if floor {
            let kind = self.air.tone_control();
            let mut padded = payload.to_vec();
            padded.resize(padded.len().max(kind.payload_bytes), 0);
            // one redundancy version: the tone control frame names only one
            return Ok(tone::burst(self.tone_codec(kind)?, &padded, 0)?);
        }
        let layout = self.air.short;
        let control = self.air.control_mode();
        let codec = self.codec(control, layout)?;
        let mut padded = payload.to_vec();
        padded.resize(padded.len().max(codec.payload_bytes), 0);
        let qam = codec.encode(&padded, rv)?;
        let header = FrameHeader::new(FrameType::Control, control.index, rv)?;
        Ok(self.tx.baseband(&header, &layout, &qam)?)
    }

    /// Payload bytes one OFDM data frame of a mode carries on the LONG layout; the control
    /// mode's SHORT frame when `mode` is `None`.
    #[must_use]
    pub fn payload_bytes(&self, mode: Option<Mode>) -> usize {
        mode.map_or_else(
            || self.air.control_mode().payload_bytes(&self.air.short),
            |m| m.payload_bytes(&self.air.long),
        )
    }

    /// Payload bytes a data frame at a rung of the ladder carries.
    ///
    /// # Panics
    /// If the ladder has no such rung.
    #[must_use]
    pub fn rung_payload_bytes(&self, rung: usize) -> usize {
        self.air.rung(rung).payload_bytes()
    }

    // ── receive ───────────────────────────────────────────────────────

    /// Band-limit a buffer, blanking impulses first if a blanker is fitted.
    ///
    /// The order matters: the filter smears an impulse into a long tail, and once that has
    /// happened there is no small set of hot samples left to remove.
    #[must_use]
    pub fn condition(&self, samples: &[Complex]) -> Vec<Complex> {
        let blanked = self
            .blanker
            .as_ref()
            .map_or_else(|| samples.to_vec(), |b| b.process(samples).samples);
        let mut fir = crate::fir::Fir::<Complex>::new(self.band_taps.clone());
        fir.process(&blanked)
    }

    /// Equalised symbols plus the `(mode, rv)` read from the chips — the soft frame a link
    /// layer combines across retransmissions.
    ///
    /// # Errors
    /// If the frame runs past the end of the buffer.
    pub fn demodulate(
        &self,
        samples: &[Complex],
        sync: &FrameSync,
    ) -> Result<ReceivedFrame, DemodError> {
        self.rx.receive(samples, sync, None)
    }

    /// Decode a demodulated OFDM frame with the redundancy version it announced, optionally
    /// combining with an earlier transmission of the same block.
    ///
    /// Returns the payload (or `None` if the check failed) and the buffer to keep.
    ///
    /// # Errors
    /// If the frame does not hold the layout's slot count, which acquisition should have
    /// made impossible.
    pub fn decode_frame(
        &mut self,
        frame: &ReceivedFrame,
        buffer: Option<&[f64]>,
    ) -> Result<(Option<Vec<u8>>, Vec<f64>), ModemError> {
        let control = frame.sync.frame_type == FrameType::Control;
        let mode = if control {
            self.air.control_mode()
        } else {
            self.air.modes[frame.mode]
        };
        let layout = self.air.layout_for(!control);
        Ok(self.codec(mode, layout)?.decode(
            &frame.symbols,
            NoiseVar::PerSymbol(&frame.noise_var),
            frame.rv,
            buffer,
        )?)
    }

    /// Decode a soft frame of either family, optionally combining with an earlier
    /// transmission of the same block (HARQ-IR). Returns the payload (or `None` if the
    /// check failed) and the buffer to keep.
    ///
    /// # Errors
    /// As [`decode_frame`](Self::decode_frame), or a tone frame whose soft bits are not its
    /// kind's count.
    pub fn decode_received(
        &mut self,
        received: &Received,
        buffer: Option<&[f64]>,
    ) -> Result<(Option<Vec<u8>>, Vec<f64>), ModemError> {
        match received {
            Received::Ofdm(frame) => self.decode_frame(frame, buffer),
            Received::Tone(frame, _) => Ok(self
                .tone_codec(frame.kind)?
                .decode(&frame.llr, frame.rv, buffer)?),
        }
    }

    /// Demodulate and decode one located OFDM frame.
    ///
    /// A failed check whose chip decision was close is retried once against the runner-up
    /// `(mode, rv)` before being given up on — the metric says the decision could have gone
    /// either way, and a second decode is far cheaper than a retransmission.
    ///
    /// # Errors
    /// If the frame runs past the end of the buffer.
    pub fn decode_sync(
        &mut self,
        samples: &[Complex],
        sync: &FrameSync,
        buffer: Option<&[f64]>,
    ) -> Result<DecodedFrame, ModemError> {
        let frame = self.rx.receive(samples, sync, None)?;
        let (payload, _) = self.decode_frame(&frame, buffer)?;
        if sync.frame_type == FrameType::Data
            && payload.is_none()
            && frame.mode_confidence < MODE_RETRY_CONFIDENCE
        {
            let alternative = self.rx.receive(samples, sync, Some(frame.chip_runner_up))?;
            let (retry, _) = self.decode_frame(&alternative, buffer)?;
            if retry.is_some() {
                return Ok(DecodedFrame {
                    payload: retry,
                    frame: Received::Ofdm(alternative),
                });
            }
        }
        Ok(DecodedFrame {
            payload,
            frame: Received::Ofdm(frame),
        })
    }

    /// Demodulate and decode a tone-floor frame found at `sync`.
    ///
    /// # Errors
    /// If the frame runs past the end of the buffer.
    pub fn decode_tone(
        &mut self,
        samples: &[Complex],
        sync: &ToneSync,
    ) -> Result<DecodedFrame, ModemError> {
        let frame = tone::demodulate(samples, sync.kind, sync.rv, sync.start, sync.cfo_hz).ok_or(
            DemodError::OutOfRange {
                start: sync.start,
                available: samples.len(),
            },
        )?;
        let (payload, _) = self
            .tone_codec(sync.kind)?
            .decode(&frame.llr, sync.rv, None)?;
        Ok(DecodedFrame {
            payload,
            frame: Received::Tone(frame, *sync),
        })
    }

    /// Blank, band-limit, detect and decode every frame of either family in a baseband
    /// buffer, in the order they start.
    pub fn decode_buffer(&mut self, samples: &[Complex], max_frames: usize) -> Vec<DecodedFrame> {
        let conditioned = self.condition(samples);
        let acquisitions = self.detector.detect(&conditioned, max_frames);
        let mut out: Vec<DecodedFrame> = acquisitions
            .iter()
            .filter_map(|a| self.decode_sync(&conditioned, &a.sync, None).ok())
            .collect();
        let tones = self.tone_detector.detect(&conditioned, max_frames);
        out.extend(
            tones
                .iter()
                .filter_map(|sync| self.decode_tone(&conditioned, sync).ok()),
        );
        out.sort_by_key(|d| d.frame.start());
        out.truncate(max_frames);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modes::MODES;

    fn mode_of(frame: &DecodedFrame) -> usize {
        frame.frame.ofdm().expect("an OFDM frame").mode
    }

    #[test]
    fn a_narrow_data_frame_and_control_frame_survive_the_round_trip() {
        let mut modem = Modem::new(crate::waveform::NARROW_500, true);
        assert_eq!(modem.modes().len(), 13);
        assert_eq!(
            modem.payload_bytes(None),
            7,
            "the narrow control frame carries 7 bytes"
        );
        for mode in [modem.modes()[2], modem.modes()[3], modem.modes()[12]] {
            let payload: Vec<u8> = (0..modem.payload_bytes(Some(mode)))
                .map(|i| (i * 29 + 3) as u8)
                .collect();
            let burst = modem.data_burst(&payload, mode, 0).expect("burst");
            let mut buffer = vec![(0.0, 0.0); 1000];
            buffer.extend_from_slice(&burst);
            buffer.extend(std::iter::repeat_n((0.0, 0.0), 1200));
            let frames = modem.decode_buffer(&buffer, 1);
            assert_eq!(frames.len(), 1, "{}", mode.name());
            assert_eq!(frames[0].payload.as_deref(), Some(payload.as_slice()));
            assert_eq!(mode_of(&frames[0]), mode.index);
            // positions are in the band-limited stream, a filter's group delay behind
            let delay = crate::passband::band_limit_taps(&crate::waveform::NARROW_500).len() / 2;
            assert_eq!(frames[0].frame.start(), 1000 + delay);
        }
        let payload: Vec<u8> = (0..7).collect();
        let burst = modem.control_burst(&payload, 0).expect("burst");
        let mut buffer = vec![(0.0, 0.0); 1000];
        buffer.extend_from_slice(&burst);
        buffer.extend(std::iter::repeat_n((0.0, 0.0), 1200));
        let frames = modem.decode_buffer(&buffer, 1);
        assert_eq!(frames.len(), 1);
        assert!(frames[0].frame.is_control());
        assert_eq!(frames[0].payload.as_deref(), Some(payload.as_slice()));
    }

    #[test]
    fn a_data_frame_survives_the_round_trip() {
        let mut modem = Modem::default();
        let mode = MODES[4];
        let payload: Vec<u8> = (0..modem.payload_bytes(Some(mode)))
            .map(|i| (i % 251) as u8)
            .collect();
        let burst = modem.data_burst(&payload, mode, 0).expect("encode");
        // a little silence either side, as a real stream would have
        let mut buffer = vec![(0.0, 0.0); 400];
        buffer.extend(burst);
        buffer.extend(std::iter::repeat_n((0.0, 0.0), 400));

        let decoded = modem.decode_buffer(&buffer, 4);
        assert_eq!(decoded.len(), 1, "expected exactly one frame");
        assert_eq!(decoded[0].payload.as_deref(), Some(payload.as_slice()));
        assert_eq!(mode_of(&decoded[0]), mode.index);
        assert_eq!(decoded[0].frame.rung(&modem.air()), Some(4 + 2));
    }

    #[test]
    fn a_control_frame_survives_the_round_trip() {
        let mut modem = Modem::default();
        let payload: Vec<u8> = (0..modem.payload_bytes(None))
            .map(|i| (i * 7) as u8)
            .collect();
        let burst = modem.control_burst(&payload, 0).expect("encode");
        let mut buffer = vec![(0.0, 0.0); 400];
        buffer.extend(burst);
        buffer.extend(std::iter::repeat_n((0.0, 0.0), 400));

        let decoded = modem.decode_buffer(&buffer, 4);
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].payload.as_deref(), Some(payload.as_slice()));
    }

    #[test]
    fn two_frames_back_to_back_are_both_decoded() {
        let mut modem = Modem::default();
        let mode = MODES[2];
        let n = modem.payload_bytes(Some(mode));
        let first: Vec<u8> = (0..n).map(|i| i as u8).collect();
        let second: Vec<u8> = (0..n).map(|i| (255 - i % 256) as u8).collect();
        let mut buffer = vec![(0.0, 0.0); 300];
        buffer.extend(modem.data_burst(&first, mode, 0).expect("encode"));
        buffer.extend(modem.data_burst(&second, mode, 0).expect("encode"));
        buffer.extend(std::iter::repeat_n((0.0, 0.0), 300));

        let decoded = modem.decode_buffer(&buffer, 4);
        assert_eq!(decoded.len(), 2, "a burst is frames back to back");
        assert_eq!(decoded[0].payload.as_deref(), Some(first.as_slice()));
        assert_eq!(decoded[1].payload.as_deref(), Some(second.as_slice()));
    }

    #[test]
    fn the_tone_floor_rungs_and_control_frame_round_trip_on_both_airs() {
        // ADR-0013: the ladder's first two rungs and the floor's control frame are the tone
        // floor's, on the wide air and the narrow one alike
        for params in [WIDE_2300, crate::waveform::NARROW_500] {
            let mut modem = Modem::new(params, true);
            for rung in 0..modem.air().floor_modes() {
                let payload: Vec<u8> = (0..modem.rung_payload_bytes(rung))
                    .map(|i| (i * 13 + rung) as u8)
                    .collect();
                let burst = modem.rung_burst(&payload, rung, 0).expect("burst");
                let mut buffer = vec![(0.0, 0.0); 1500];
                buffer.extend(burst);
                buffer.extend(std::iter::repeat_n((0.0, 0.0), 2000));
                let frames = modem.decode_buffer(&buffer, 4);
                assert_eq!(frames.len(), 1, "{params:?} rung {rung}");
                assert!(frames[0].frame.is_tone());
                assert_eq!(frames[0].payload.as_deref(), Some(payload.as_slice()));
                assert_eq!(frames[0].frame.rung(&modem.air()), Some(rung));
            }
            let payload = [9u8, 8, 7, 6, 5];
            let burst = modem.control_burst_of(&payload, 0, true).expect("burst");
            let mut buffer = vec![(0.0, 0.0); 1500];
            buffer.extend(burst);
            buffer.extend(std::iter::repeat_n((0.0, 0.0), 2000));
            let frames = modem.decode_buffer(&buffer, 4);
            assert_eq!(frames.len(), 1);
            assert!(frames[0].frame.is_control() && frames[0].frame.is_tone());
            // padded to the control frame's seven bytes
            assert_eq!(
                frames[0].payload.as_deref(),
                Some(&[9u8, 8, 7, 6, 5, 0, 0][..])
            );
        }
    }

    #[test]
    fn a_tone_frame_is_combined_through_its_buffer() {
        // the soft frame is what the link layer keeps: decoding it again with its own buffer
        // must give the payload back, and the buffer must be the codeword's length
        let mut modem = Modem::default();
        let payload: Vec<u8> = (0..24u8).collect();
        let mut buffer = vec![(0.0, 0.0); 700];
        buffer.extend(modem.rung_burst(&payload, 0, 0).expect("burst"));
        buffer.extend(std::iter::repeat_n((0.0, 0.0), 1500));
        let frames = modem.decode_buffer(&buffer, 1);
        assert_eq!(frames.len(), 1);
        let (first, kept) = modem
            .decode_received(&frames[0].frame, None)
            .expect("decode");
        assert_eq!(first.as_deref(), Some(payload.as_slice()));
        let (again, _) = modem
            .decode_received(&frames[0].frame, Some(&kept))
            .expect("combine");
        assert_eq!(again.as_deref(), Some(payload.as_slice()));
    }
}
