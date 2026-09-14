"""Impulsive-noise defence: blanker and per-symbol noise variance (roadmap P2-5)."""

from __future__ import annotations

import numpy as np
import pytest

from aether_model.channel import ChannelConfig, HfChannel, make_channel
from aether_model.frame.modes import MODES
from aether_model.phy.blanker import NoiseBlanker
from aether_model.phy.pipeline import Modem
from aether_model.waveform import WIDE_2300 as P

FS = P.fs_baseband


def _impulsive(signal: np.ndarray, snr_db: float, probability: float, burst_db: float, seed: int):
    cfg = ChannelConfig(
        profile="awgn",
        snr_db=snr_db,
        fs=FS,
        seed=seed,
        signal_power=1.0,
        impulsive_probability=probability,
        impulsive_db_above_noise=burst_db,
    )
    return HfChannel(cfg).process(signal)


def _buffer(burst: np.ndarray) -> np.ndarray:
    return np.concatenate((np.zeros(900), burst, np.zeros(900)))


# ── the blanker on its own ────────────────────────────────────────────


def test_blanker_leaves_clean_gaussian_noise_almost_untouched() -> None:
    """The threshold is in units of the local RMS envelope, and a complex Gaussian envelope
    is Rayleigh, so P(blank) = exp(-k^2): vanishing at the default k = 3.5."""
    rng = np.random.default_rng(1)
    x = (rng.standard_normal(40000) + 1j * rng.standard_normal(40000)) / np.sqrt(2)
    result = NoiseBlanker().process(x)
    assert result.fraction < 1e-3


def test_blanker_removes_impulses() -> None:
    rng = np.random.default_rng(2)
    x = (rng.standard_normal(20000) + 1j * rng.standard_normal(20000)) / np.sqrt(2)
    hits = rng.choice(len(x), 200, replace=False)
    x[hits] += 30.0 * (rng.standard_normal(200) + 1j * rng.standard_normal(200))
    result = NoiseBlanker().process(x)
    assert result.blanked[hits].mean() > 0.9  # nearly every impulse caught
    assert np.max(np.abs(result.samples)) < np.max(np.abs(x)) / 10


def test_blanker_survives_a_sustained_burst() -> None:
    """A static crash lasts tens of milliseconds — far longer than the median window. A plain
    moving median would measure the burst itself and raise the threshold to match; the
    median-of-medians reference keeps its footing up to ``robust_span_samples``."""
    rng = np.random.default_rng(3)
    blanker = NoiseBlanker()
    burst = blanker.robust_span_samples // 2  # comfortably inside the robust span
    x = (rng.standard_normal(8000) + 1j * rng.standard_normal(8000)) / np.sqrt(2)
    x[3000 : 3000 + burst] += 40.0 * (rng.standard_normal(burst) + 1j * rng.standard_normal(burst))
    result = blanker.process(x)
    assert result.blanked[3000 : 3000 + burst].mean() > 0.8
    assert result.blanked[:2000].mean() < 0.01  # and it leaves the clean part alone


def test_blanker_robust_span_is_documented_and_finite() -> None:
    """Past this length a burst becomes the local level and no threshold based on it can
    help; the number is stated so a caller can widen the window if their noise is worse."""
    blanker = NoiseBlanker()
    assert blanker.robust_span_samples > 400  # > 50 ms at 8 kHz
    assert NoiseBlanker(window=201, segment_span=15).robust_span_samples > (
        blanker.robust_span_samples
    )


def test_clip_mode_limits_instead_of_zeroing() -> None:
    rng = np.random.default_rng(4)
    x = (rng.standard_normal(4000) + 1j * rng.standard_normal(4000)) / np.sqrt(2)
    x[100] += 50.0
    clipped = NoiseBlanker(mode="clip").process(x)
    assert abs(clipped.samples[100]) > 0  # limited, not removed
    assert abs(clipped.samples[100]) < abs(x[100]) / 5
    assert NoiseBlanker(mode="blank").process(x).samples[100] == 0


def test_blanker_rejects_bad_configuration() -> None:
    with pytest.raises(ValueError):
        NoiseBlanker(threshold_sigma=0)
    with pytest.raises(ValueError):
        NoiseBlanker(mode="squash")


def test_blanker_handles_an_empty_block() -> None:
    result = NoiseBlanker().process(np.zeros(0, dtype=complex))
    assert result.samples.size == 0 and result.fraction == 0.0


# ── end to end ────────────────────────────────────────────────────────


@pytest.mark.parametrize("probability", [0.005, 0.02])
def test_blanker_rescues_frames_from_impulsive_noise(probability: float) -> None:
    """Without it, impulses at these rates take QPSK 1/2 to a total loss at an SNR where it
    is otherwise comfortable."""
    rng = np.random.default_rng(50)
    defended = Modem(P)
    plain = Modem(P, blank_impulses=False)
    ok_defended = ok_plain = 0
    trials = 6
    for t in range(trials):
        payload = rng.integers(0, 256, defended.payload_bytes(MODES[4]), dtype=np.uint8).tobytes()
        y = _impulsive(
            _buffer(defended.data_burst(payload, MODES[4])), 4.0, probability, 25.0, 900 + t
        )
        for modem, counter in ((defended, "d"), (plain, "p")):
            result = modem.decode_buffer(y)
            got = len(result) == 1 and result[0].payload == payload
            if counter == "d":
                ok_defended += int(got)
            else:
                ok_plain += int(got)
    assert ok_defended >= trials - 1
    assert ok_defended > ok_plain


@pytest.mark.parametrize(("mode_idx", "snr_db"), [(0, -4.0), (4, 4.0), (13, 20.0)])
def test_blanker_costs_nothing_on_a_clean_channel(mode_idx: int, snr_db: float) -> None:
    """It is on by default, so it has to be free when there is nothing to defend against."""
    rng = np.random.default_rng(60 + mode_idx)
    defended = Modem(P)
    plain = Modem(P, blank_impulses=False)
    for t in range(4):
        n = defended.payload_bytes(MODES[mode_idx])
        payload = rng.integers(0, 256, n, dtype=np.uint8).tobytes()
        y = make_channel("awgn", snr_db=snr_db, fs=FS, seed=300 + t, signal_power=1.0).process(
            _buffer(defended.data_burst(payload, MODES[mode_idx]))
        )
        a = defended.decode_buffer(y)
        b = plain.decode_buffer(y)
        assert (len(a) == 1 and a[0].payload == payload) == (
            len(b) == 1 and b[0].payload == payload
        )


def test_per_symbol_noise_variance_tracks_a_damaged_symbol() -> None:
    """One impulse should raise the reported variance of the symbol it lands in, not the
    whole frame — that is what turns it into an erasure for the decoder."""
    rng = np.random.default_rng(7)
    modem = Modem(P, blank_impulses=False)
    payload = rng.integers(0, 256, modem.payload_bytes(MODES[4]), dtype=np.uint8).tobytes()
    clean = _buffer(modem.data_burst(payload, MODES[4]))
    y = make_channel("awgn", snr_db=12.0, fs=FS, seed=5, signal_power=1.0).process(clean)
    hit = y.copy()
    victim = 900 + 10 * P.symbol_samples + P.symbol_samples // 2
    hit[victim] += 60.0  # one very hot sample, inside one OFDM symbol

    def variances(buf: np.ndarray) -> np.ndarray:
        cond = modem.detector.condition(buf)
        (sync,) = modem.detector.detect(cond)
        frame = modem.demodulate(cond, sync)
        return frame.noise_var.reshape(-1, len(modem.rx.data_c)).mean(axis=1)

    before, after = variances(y), variances(hit)
    worst = int(np.argmax(after / before))
    assert after[worst] / before[worst] > 3.0  # that symbol is now distrusted
    others = np.delete(after / before, worst)
    assert np.median(others) < 2.0  # the rest of the frame is not


# ── streaming (P3-3) ──────────────────────────────────────────────────


@pytest.mark.parametrize("block", [251, 683, 1024, 4096])
def test_streaming_blanker_is_independent_of_block_size(block: int) -> None:
    """The property the streaming wrapper exists for.

    The reference is a *local* statistic, so blanking block by block judges each block
    against itself: the start of a burst is measured against the silence in front of it and
    removed, and how much is removed depends on where the sound card happened to put its
    buffer boundaries. A receiver whose output depends on its buffer size is not a receiver
    anybody can measure."""
    from aether_model.phy.blanker import StreamingBlanker

    rng = np.random.default_rng(23)
    x = (0.01 * (rng.standard_normal(30_000) + 1j * rng.standard_normal(30_000))).astype(
        np.complex128
    )
    x[8_000:20_000] = np.exp(1j * 0.7 * np.arange(8_000, 20_000))  # an abrupt burst
    x[14_000] = 60.0  # and one genuine impulse inside it

    whole = StreamingBlanker()
    reference = np.concatenate((whole.process(x), whole.flush()))
    assert len(reference) == len(x)

    streamed = StreamingBlanker()
    pieces = [streamed.process(x[i : i + block]) for i in range(0, len(x), block)]
    got = np.concatenate([*pieces, streamed.flush()])
    assert len(got) == len(x)
    assert np.max(np.abs(got - reference)) == 0.0


def test_streaming_blanker_holds_back_exactly_its_stated_latency() -> None:
    from aether_model.phy.blanker import StreamingBlanker

    blanker = StreamingBlanker()
    rng = np.random.default_rng(31)
    x = (0.01 * (rng.standard_normal(10_000) + 1j * rng.standard_normal(10_000))).astype(
        np.complex128
    )
    emitted = sum(len(blanker.process(x[i : i + 1000])) for i in range(0, len(x), 1000))
    assert emitted == len(x) - blanker.latency_samples
    assert len(blanker.flush()) == blanker.latency_samples


def test_blanking_block_by_block_depends_on_the_block_size() -> None:
    """What the streaming wrapper is for, shown on a real frame rather than a synthetic one.

    Applied block by block the blanker removes a different set of samples depending on where
    the sound card happened to put its buffer boundaries, because its reference is a local
    statistic of whatever block it was handed. A receiver whose output depends on its buffer
    size cannot be measured, and it was worth 20 dB of SNR in the Rust core's receive path at
    its own block size before this was fixed."""
    from aether_model.phy.blanker import NoiseBlanker, StreamingBlanker
    from aether_model.phy.passband import AudioToBaseband

    modem = Modem(P, blank_impulses=False)
    mode = MODES[0]
    burst = modem.data_burst(bytes(range(modem.payload_bytes(mode))), mode)
    baseband = np.concatenate((np.zeros(800), burst, np.zeros(4000))).astype(np.complex128)
    x = AudioToBaseband(P).process(modem.audio(baseband))

    def stateless(block: int) -> set[int]:
        out = np.concatenate(
            [NoiseBlanker().process(x[i : i + block]).samples for i in range(0, len(x), block)]
        )
        return set(np.flatnonzero(out == 0).tolist())

    def streaming(block: int) -> set[int]:
        blanker = StreamingBlanker()
        pieces = [blanker.process(x[i : i + block]) for i in range(0, len(x), block)]
        out = np.concatenate([*pieces, blanker.flush()])
        return set(np.flatnonzero(out == 0).tolist())

    assert stateless(683) != stateless(4096), "the bug this fixes is not being reproduced"
    assert streaming(683) == streaming(4096)
