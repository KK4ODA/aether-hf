//! Whether the receive audio clips (ADR-0105 §2.2, kept by ADR-0132).
//!
//! The share of the audio heard at full scale over the last ten seconds. Clipped audio spreads a
//! strong signal across the band and costs every frame, and the operator fixes it with one knob:
//! the radio's audio output or the sound card's input. Every recording of the mobile's sessions
//! of 2026-11-07 clipped, and nothing on the panel had said so.

use std::collections::VecDeque;

/// A sample this near full scale counts as clipped.
pub const FULL_SCALE: f32 = 0.99;
/// How much of the audio the station listened to clipping is read over, seconds.
pub const CLIP_WINDOW_S: f64 = 10.0;
/// The share of samples at full scale over which the audio clips: every one of the mobile's
/// sessions of 2026-11-07 passed it (1.3–1.7 %) and no other recording reached it.
pub const CLIP_SHARE: f64 = 0.001;

/// The share of samples at full scale over the last [`CLIP_WINDOW_S`] of audio heard. While it is
/// over [`CLIP_SHARE`] the audio clips.
#[derive(Debug, Clone, Default)]
pub struct Clipping {
    /// Per block (clipped, samples), oldest first.
    blocks: VecDeque<(usize, usize)>,
    held: usize,
    window: usize,
}

impl Clipping {
    /// A fresh reading at `rate` samples a second.
    #[must_use]
    pub fn new(rate: f64) -> Self {
        Self {
            window: (rate * CLIP_WINDOW_S) as usize,
            ..Self::default()
        }
    }

    /// A block of audio the station heard.
    pub fn hear(&mut self, audio: &[f32]) {
        let clipped = audio.iter().filter(|x| x.abs() >= FULL_SCALE).count();
        self.blocks.push_back((clipped, audio.len()));
        self.held += audio.len();
        while self.held > self.window
            && let Some((_, n)) = self.blocks.pop_front()
        {
            self.held -= n;
        }
    }

    /// The share of the samples held at full scale.
    #[must_use]
    pub fn share(&self) -> f64 {
        let clipped: usize = self.blocks.iter().map(|&(c, _)| c).sum();
        clipped as f64 / self.held.max(1) as f64
    }

    /// Whether the audio clips now.
    #[must_use]
    pub fn clips(&self) -> bool {
        self.share() > CLIP_SHARE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipping_is_read_over_the_last_ten_seconds() {
        let rate = 1000.0;
        let mut c = Clipping::new(rate);
        let mut block = vec![0.1f32; 100];
        block[0] = 1.0;
        block[1] = -0.995;
        c.hear(&block);
        assert!(c.clips(), "{}", c.share());
        for _ in 0..100 {
            c.hear(&[0.1f32; 100]);
        }
        assert!(c.share() < f64::EPSILON, "{}", c.share());
        assert!(!c.clips());
    }
}
