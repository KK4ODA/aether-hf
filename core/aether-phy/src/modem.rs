//! Payload in, burst out; buffer in, decoded frames out.
//!
//! A thin tie between the frame codec, transmitter, detector and receiver. Everything here is
//! a convenience over parts that already work on their own; the point is that a caller — the
//! daemon, a benchmark, a test — does not have to re-derive which codec goes with which
//! layout, or remember that a close chip decision is worth a second attempt.

use std::collections::HashMap;

use aether_fec::FecError;

use crate::{
    Complex,
    blanker::NoiseBlanker,
    codec::FrameCodec,
    constellation::NoiseVar,
    modes::{CONTROL_MODE, FrameLayout, LONG, MODES, Mode, SHORT},
    ofdm::DemodError,
    passband::band_limit_taps,
    preamble::{FrameHeader, FrameType, HeaderError},
    rx::{FrameReceiver, FrameSync, ReceivedFrame},
    sync::FrameDetector,
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

/// A frame the receiver got all the way through.
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    /// The payload, or `None` if the check failed.
    pub payload: Option<Vec<u8>>,
    /// The demodulated frame behind it, whose symbols a link layer can combine.
    pub frame: ReceivedFrame,
    /// The mode it was decoded as.
    pub mode: Mode,
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
    tx: FrameTransmitter,
    detector: FrameDetector,
    rx: FrameReceiver,
    /// Impulse blanker run ahead of band-limiting (P2-5). On by default: measured to cost a
    /// clean channel nothing at any mode while removing impulsive noise that otherwise takes
    /// the link to 100 % frame errors.
    blanker: Option<NoiseBlanker>,
    band_taps: Vec<f64>,
    codecs: HashMap<(usize, &'static str), FrameCodec>,
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
            blanker: blank_impulses.then(NoiseBlanker::default),
            band_taps: band_limit_taps(&params),
            codecs: HashMap::new(),
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

    /// The detector, for a caller that wants to acquire on its own.
    #[must_use]
    pub fn detector(&self) -> &FrameDetector {
        &self.detector
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

    // ── transmit ──────────────────────────────────────────────────────

    /// Baseband for one data frame.
    ///
    /// # Errors
    /// If the payload is the wrong length for the mode.
    pub fn data_burst(
        &mut self,
        payload: &[u8],
        mode: Mode,
        rv: u8,
    ) -> Result<Vec<Complex>, ModemError> {
        let qam = self.codec(mode, LONG)?.encode(payload, rv)?;
        let header = FrameHeader::new(FrameType::Data, mode.index, rv)?;
        Ok(self.tx.baseband(&header, &LONG, &qam)?)
    }

    /// Baseband for one control frame.
    ///
    /// # Errors
    /// If the payload is the wrong length for the control mode.
    pub fn control_burst(&mut self, payload: &[u8], rv: u8) -> Result<Vec<Complex>, ModemError> {
        let qam = self.codec(CONTROL_MODE, SHORT)?.encode(payload, rv)?;
        let header = FrameHeader::new(FrameType::Control, CONTROL_MODE.index, rv)?;
        Ok(self.tx.baseband(&header, &SHORT, &qam)?)
    }

    /// Payload bytes one frame of a mode carries; the control mode when `mode` is `None`.
    #[must_use]
    pub fn payload_bytes(&self, mode: Option<Mode>) -> usize {
        mode.map_or_else(
            || CONTROL_MODE.payload_bytes(&SHORT),
            |m| m.payload_bytes(&LONG),
        )
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

    /// Decode a demodulated frame with the redundancy version it announced, optionally
    /// combining with an earlier transmission of the same block.
    ///
    /// Returns the payload (or `None` if the check failed) and the buffer to keep.
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
            CONTROL_MODE
        } else {
            MODES[frame.mode]
        };
        let layout = if control { SHORT } else { LONG };
        Ok(self.codec(mode, layout)?.decode(
            &frame.symbols,
            NoiseVar::PerSymbol(&frame.noise_var),
            frame.rv,
            buffer,
        )?)
    }

    /// Demodulate and decode one located frame.
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
        if sync.frame_type == FrameType::Control {
            let (payload, _) = self.decode_frame(&frame, buffer)?;
            return Ok(DecodedFrame {
                payload,
                frame,
                mode: CONTROL_MODE,
            });
        }
        let mode = MODES[frame.mode];
        let (payload, _) = self.decode_frame(&frame, buffer)?;
        if payload.is_none() && frame.mode_confidence < MODE_RETRY_CONFIDENCE {
            let alternative = self.rx.receive(samples, sync, Some(frame.chip_runner_up))?;
            let (retry, _) = self.decode_frame(&alternative, buffer)?;
            if retry.is_some() {
                let mode = MODES[alternative.mode];
                return Ok(DecodedFrame {
                    payload: retry,
                    frame: alternative,
                    mode,
                });
            }
        }
        Ok(DecodedFrame {
            payload,
            frame,
            mode,
        })
    }

    /// Blank, band-limit, detect and decode every frame in a baseband buffer.
    pub fn decode_buffer(&mut self, samples: &[Complex], max_frames: usize) -> Vec<DecodedFrame> {
        let conditioned = self.condition(samples);
        let acquisitions = self.detector.detect(&conditioned, max_frames);
        acquisitions
            .iter()
            .filter_map(|a| self.decode_sync(&conditioned, &a.sync, None).ok())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(decoded[0].mode.index, mode.index);
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
}
