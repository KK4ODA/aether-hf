"""The FM channel model's checks (ADR-0115, FM-3).

These hold the model to its conventions (block-exact streaming, the 16 kHz CNR, units of the
rated deviation) and to the physics that every FM number rests on: emphasis, the limiter, the
FM threshold and its clicks, squelch and keying times, Clarke fading, the sound cards' clocks.
The measure is checked here too, because Gate 2 compares recordings and the model through it.
"""

from __future__ import annotations

import itertools
import math
import sys
from dataclasses import replace
from pathlib import Path

import numpy as np
import pytest
from scipy import signal

from aether_model.fm import channel as ch
from aether_model.fm import measure as me
from aether_model.fm import signals as sg

FS = ch.FS


def _quiet_receiver(**kw: object) -> ch.ReceiverConfig:
    return replace(ch.ReceiverConfig(), squelch=None, floor_db=None, **kw)  # type: ignore[arg-type]


def _tone_snr(y: np.ndarray, freq: float) -> tuple[float, np.ndarray, float]:
    """The tone's SNR in ``y``, the residual, and the tone's amplitude (least squares)."""
    t = np.arange(len(y)) / FS
    a = np.vstack([np.sin(2 * np.pi * freq * t), np.cos(2 * np.pi * freq * t)]).T
    c, *_ = np.linalg.lstsq(a, y, rcond=None)
    fit = a @ c
    r = y - fit
    return 10 * math.log10(np.mean(fit**2) / np.mean(r**2)), r, float(np.hypot(*c))


# ── Streaming ─────────────────────────────────────────────────────────


def _everything_on(seed: int) -> ch.FmLinkConfig:
    return ch.FmLinkConfig(
        sender_card=ch.SoundCardConfig(volume_db=-2.0, clock_ppm=180.0, noise_dbfs=-90.0),
        transmitter=ch.TransmitterConfig(
            compressor=ch.Compressor(),
            limiter_smoothness=3.0,
            ctcss_hz=100.0,
            rf_offset_hz=700.0,
            gain_db=4.0,
        ),
        path=ch.PathConfig(cnr_db=14.0, fading="rician", doppler_hz=12.0, rician_k_db=3.0),
        receiver=ch.ReceiverConfig(hum_db=-50.0, gain_db=-3.0),
        receiver_card=ch.SoundCardConfig(volume_db=1.0, clock_ppm=-120.0, noise_dbfs=-85.0),
        seed=seed,
    )


def test_the_link_is_block_exact_with_every_stage_on() -> None:
    rng = np.random.default_rng(7)
    x = 0.3 * sg.single_carrier(2000, 1650, 1.2, rng)
    keys = [(0, 9600, False), (9600, 48000, True), (48000, len(x), False)]
    whole = ch.FmLink(_everything_on(11))
    out_whole = np.concatenate([whole.process(x[a:b], k) for a, b, k in keys])
    split = ch.FmLink(_everything_on(11))
    cut_rng = np.random.default_rng(3)
    parts = []
    for a, b, k in keys:
        cuts = np.sort(cut_rng.integers(a, b, 6))
        edges = [a, *cuts.tolist(), b]
        parts += [split.process(x[p:q], k) for p, q in itertools.pairwise(edges)]
    out_split = np.concatenate(parts)
    assert len(out_whole) == len(out_split)
    assert np.array_equal(out_whole, out_split)


def test_the_same_seed_gives_the_same_output_and_another_does_not() -> None:
    x = 0.3 * sg.single_carrier(2000, 1650, 0.5, np.random.default_rng(1))
    a = ch.FmLink(_everything_on(5)).process(x)
    b = ch.FmLink(_everything_on(5)).process(x)
    c = ch.FmLink(_everything_on(6)).process(x)
    assert np.array_equal(a, b)
    assert not np.array_equal(a, c)


# ── Conventions ───────────────────────────────────────────────────────


def test_cnr_is_referenced_to_16_khz_whatever_the_if() -> None:
    p = ch.Path(ch.PathConfig(cnr_db=10.0), np.random.default_rng(1), np.random.default_rng(2))
    n = 4 * int(FS)
    noise = p.process(np.ones(n, dtype=np.complex128)) - 1.0
    spec = np.abs(np.fft.fft(noise)) ** 2 / n
    f = np.fft.fftfreq(n, 1 / FS)
    in_band = float(np.sum(spec[np.abs(f) <= ch.CNR_REFERENCE_BANDWIDTH_HZ / 2])) / n
    assert 10 * math.log10(1.0 / in_band) == pytest.approx(10.0, abs=0.1)


def test_the_cnr_follows_the_power_at_the_antenna() -> None:
    # kTB in 16 kHz is -131.96 dBm; a 6 dB noise figure puts the floor at -125.96 dBm
    assert ch.cnr_db(-125.96, 6.0) == pytest.approx(0.0, abs=0.01)
    assert ch.cnr_db(-110.0, 6.0) - ch.cnr_db(-120.0, 6.0) == pytest.approx(10.0)


def test_a_tone_comes_through_at_its_deviation_on_the_flat_port() -> None:
    cfg = replace(
        ch.flat_port(cnr=60.0),
        receiver=replace(ch.flat_port().receiver, floor_db=None),
        receiver_card=ch.SoundCardConfig(bits=None),
    )
    y = ch.FmLink(cfg).process(sg.tone(1000.0, 1.0, 0.5))[9600:]
    snr, _, amp = _tone_snr(y, 1000.0)
    assert 20 * math.log10(amp / 0.5) == pytest.approx(0.0, abs=0.1)
    assert snr > 40.0


def test_a_tone_comes_through_at_its_deviation_on_the_voice_path() -> None:
    cfg = replace(ch.wide_voice(cnr=60.0), receiver=_quiet_receiver())
    y = ch.FmLink(cfg).process(sg.tone(1000.0, 1.0, 0.3))[9600:]
    _, _, amp = _tone_snr(y, 1000.0)
    assert 20 * math.log10(amp / 0.3) == pytest.approx(0.0, abs=0.3)


def test_a_narrow_receiver_reads_a_narrow_transmitter_at_the_same_level() -> None:
    cfg = replace(
        ch.narrow_voice(cnr=60.0), receiver=_quiet_receiver(if_half_hz=4250.0, deviation_hz=2500.0)
    )
    y = ch.FmLink(cfg).process(sg.tone(1000.0, 1.0, 0.3))[9600:]
    _, _, amp = _tone_snr(y, 1000.0)
    assert 20 * math.log10(amp / 0.3) == pytest.approx(0.0, abs=0.3)


# ── The transmitter ───────────────────────────────────────────────────


def test_emphasis_rises_six_db_an_octave_and_the_receiver_undoes_it() -> None:
    b, a = ch.emphasis(750.0)
    f = np.array([500.0, 1000.0, 2000.0, 3000.0])
    _, h = signal.freqz(b, a, f, fs=FS)
    gain = 20 * np.log10(np.abs(h))
    fz, fp = 1 / (2 * math.pi * 750e-6), 6000.0

    def analog(fr: float) -> float:
        return 20 * math.log10(abs(1 + 1j * fr / fz) / abs(1 + 1j * fr / fp))

    for fr, g in zip(f, gain, strict=True):
        assert g == pytest.approx(analog(fr) - analog(1000.0), abs=0.35)
    x = np.random.default_rng(0).standard_normal(int(FS))
    db_, da = ch.deemphasis(750.0)
    back = signal.lfilter(db_, da, signal.lfilter(b, a, x))
    assert np.max(np.abs(back - x)) < 1e-9


def test_the_limiter_holds_the_voice_path_and_the_flat_port_has_none() -> None:
    x = sg.tone(1000.0, 0.5, 1.0)

    def rms(port: ch.Port, gain_db: float) -> float:
        tx = ch.Transmitter(ch.TransmitterConfig(port=port, gain_db=gain_db))
        m = tx.modulation(x)[4800:]
        return 20 * math.log10(float(np.sqrt(np.mean(m**2))))

    assert rms("voice", 30.0) - rms("voice", 20.0) == pytest.approx(0.0, abs=0.5)
    assert rms("flat", 10.0) - rms("flat", 0.0) == pytest.approx(10.0, abs=0.1)


def test_the_compressor_settles_at_its_ratio() -> None:
    comp = ch.Compressor(threshold=0.25, ratio=4.0)
    tx = ch.Transmitter(
        ch.TransmitterConfig(compressor=comp, emphasis_us=None, splatter_hz=None, highpass_hz=None)
    )
    m = tx.modulation(sg.tone(1000.0, 2.0, 0.9))[48000:]
    expected = comp.threshold * (0.9 / comp.threshold) ** (1 / comp.ratio)
    # the peak envelope sags a little between a tone's peaks (its release), so the gain sits
    # a few percent above the static curve's
    assert float(np.max(np.abs(m))) == pytest.approx(expected, rel=0.05)


def test_the_carrier_follows_the_key_after_its_delay_and_attack() -> None:
    c = ch.TransmitterConfig(key_delay_s=0.05, attack_s=0.01, release_s=0.005)
    tx = ch.Transmitter(c)
    off = tx.process(np.zeros(4800), keyed=False)
    assert np.all(off == 0)
    on = np.abs(tx.process(np.zeros(9600), keyed=True))
    up = round(c.key_delay_s * FS)
    assert np.all(on[:up] == 0)
    assert np.all(on[up + round(c.attack_s * FS) :] == pytest.approx(1.0))
    down = np.abs(tx.process(np.zeros(4800), keyed=False))
    assert np.all(down[round(c.release_s * FS) :] == 0)


# ── The path and the receiver ─────────────────────────────────────────


def test_the_fm_threshold_and_its_clicks_arise_by_themselves() -> None:
    x = sg.tone(1000.0, 2.0, 0.5)
    snr: dict[float, float] = {}
    clicks: dict[float, int] = {}
    for cnr in (30.0, 15.0, 12.0, 8.0, 4.0, 2.0):
        cfg = replace(ch.wide_voice(cnr=cnr), receiver=_quiet_receiver(), seed=3)
        y = ch.FmLink(cfg).process(x)[9600:]
        snr[cnr], r, _ = _tone_snr(y, 1000.0)
        clicks[cnr] = int(np.sum(np.abs(r) > 0.5))
    # above threshold: one decibel of audio SNR per decibel of CNR
    assert snr[30.0] - snr[15.0] == pytest.approx(15.0, abs=1.5)
    # below it, the audio falls apart much faster
    assert snr[8.0] - snr[2.0] > 12.0
    assert clicks[30.0] == clicks[15.0] == clicks[12.0] == 0
    assert clicks[4.0] > 100


def test_the_squelch_opens_and_closes_on_time_and_passes_its_tail() -> None:
    cfg = replace(ch.wide_voice(cnr=30.0), receiver_card=ch.SoundCardConfig(bits=None))
    link = ch.FmLink(cfg)
    t = cfg.transmitter
    sq = cfg.receiver.squelch
    assert sq is not None
    before = link.process(np.zeros(int(0.3 * FS)), keyed=False)
    during = link.process(sg.tone(1000.0, 1.0, 0.3), keyed=True)
    after = link.process(np.zeros(int(0.5 * FS)), keyed=False)
    assert np.all(before == 0)
    opened = int(np.argmax(during != 0)) / FS
    assert opened == pytest.approx(t.key_delay_s + t.attack_s / 2 + sq.open_s, abs=0.008)
    closed = (len(after) - int(np.argmax(after[::-1] != 0))) / FS
    # the smoothed IF power falls from the carrier's to the threshold before the tail starts
    falls = sq.smoothing_s * math.log(
        1.0 / (link.receiver.noise_if_power * 10 ** (sq.threshold_db / 10))
    )
    assert closed == pytest.approx(t.release_s / 2 + falls + sq.tail_s, abs=0.008)
    tail = after[int(0.03 * FS) : int(0.14 * FS)]
    assert me.level_db(tail) > -20.0  # the squelch tail is the discriminator's noise


def test_mobile_fading_has_unit_power_and_a_clarke_spectrum() -> None:
    fd = 13.5
    fad = ch.ClarkeFading(fd, np.random.default_rng(4))
    g = fad.lowrate(400_000)
    assert float(np.mean(np.abs(g) ** 2)) == pytest.approx(1.0, rel=0.05)
    f, p = signal.welch(g, fs=fad.fs_low, nperseg=4096, return_onesided=False)
    inside = float(np.sum(p[np.abs(f) <= 1.1 * fd])) / float(np.sum(p))
    assert inside > 0.99
    # Clarke's spectrum peaks at its edges: more power near ±f_d than near 0
    edge = float(np.mean(p[(np.abs(f) > 0.8 * fd) & (np.abs(f) < fd)]))
    middle = float(np.mean(p[np.abs(f) < 0.3 * fd]))
    assert edge > 1.5 * middle


def test_a_rician_path_keeps_unit_mean_power() -> None:
    path = ch.Path(
        ch.PathConfig(cnr_db=200.0, fading="rician", doppler_hz=20.0, rician_k_db=6.0),
        np.random.default_rng(1),
        np.random.default_rng(2),
    )
    y = path.process(np.ones(20 * int(FS), dtype=np.complex128))
    assert float(np.mean(np.abs(y) ** 2)) == pytest.approx(1.0, rel=0.05)


# ── Sound cards ───────────────────────────────────────────────────────


def test_a_sound_card_clips_at_full_scale_and_quantises() -> None:
    card = ch.SoundCardOut(ch.SoundCardConfig(volume_db=6.0), np.random.default_rng(0))
    y = card.process(sg.tone(1000.0, 0.1, 0.9))
    assert float(np.max(np.abs(y))) == 1.0
    assert np.all(np.round(y * 32768) == y * 32768)


@pytest.mark.parametrize(("role", "ppm"), [("adc", 150.0), ("dac", -80.0)])
def test_the_markers_read_a_clock_offset(role: str, ppm: float) -> None:
    marker = sg.chirp(400.0, 2800.0, 0.25)
    gap = np.zeros(int(4.0 * FS))
    x = np.concatenate([marker, gap, marker, np.zeros(4800)])
    y = ch.Clock(ppm, role).process(x)  # type: ignore[arg-type]
    heard_ppm, d0 = me.clock_ppm(marker, y, 0, len(marker) + len(gap), 400)
    expected = ppm if role == "adc" else -ppm  # a fast DAC shortens what the reference hears
    assert heard_ppm == pytest.approx(expected, abs=3.0)
    # the marker drifts by the clock across its own length; its middle is found
    assert abs(d0) <= 1


# ── The measure ───────────────────────────────────────────────────────


def test_the_measure_reads_a_known_sndr() -> None:
    rng = np.random.default_rng(9)
    x = sg.single_carrier(2000, 1650, 6.0, rng)
    # noise with the signal's own spectrum, so the SNDR is 20 dB at every frequency
    noise = sg.single_carrier(2000, 1650, 6.0, np.random.default_rng(10))[: len(x)]
    reading = me.sndr(x, x + 0.1 * noise, (600, 2700))
    assert reading.effective_db == pytest.approx(20.0, abs=0.6)


def test_the_measure_reads_the_band() -> None:
    x = np.random.default_rng(2).standard_normal(int(6 * FS))
    heard = signal.sosfilt(signal.butter(4, 2000, "lowpass", fs=FS, output="sos"), x)
    f, h = me.response(x, heard, (300, 3000))
    assert float(np.interp(1000.0, f, h)) == pytest.approx(0.0, abs=0.01)
    assert float(np.interp(2000.0, f, h)) == pytest.approx(-3.0, abs=0.3)


def test_the_delay_is_found() -> None:
    x = sg.single_carrier(2000, 1650, 1.0, np.random.default_rng(3))
    heard = np.concatenate([np.zeros(321), x])
    assert me.delay(x, heard, 2000) == 321


# ── The scratch study's anchor ────────────────────────────────────────


def test_the_proposal_s_point_is_reproduced() -> None:
    """The proposal's single carrier at the scratch model's best drive, CNR 18 dB in its IF
    (17.8 dB in 16 kHz), measured 24.6 dB. The model agrees within a decibel once the key-up
    is left out of the measure: one bad segment in 140 caps a coherence estimate near 18 dB."""
    scratch = Path(__file__).resolve().parents[2] / "docs/research/aether-fm/scratch"
    sys.path.insert(0, str(scratch))
    try:
        import fmchain  # type: ignore[import-not-found]
    finally:
        sys.path.remove(str(scratch))
    x = sg.single_carrier(2000, 1650, 6.0, np.random.default_rng(11))
    pre = signal.lfilter(*fmchain.PRE, signal.sosfilt(fmchain.TX_HP, x))
    g = 10 ** (-7.5 / 20) / math.sqrt(float(np.mean(pre**2)))
    cnr16 = 18.0 - 10 * math.log10(
        ch.CNR_REFERENCE_BANDWIDTH_HZ / fmchain.Radio("mic", 5000.0, 7500.0).noise_bw
    )
    cfg = replace(
        ch.wide_voice(cnr=cnr16, seed=5),
        receiver=replace(ch.ReceiverConfig(), squelch=None),
        sender_card=ch.SoundCardConfig(bits=None),
        receiver_card=ch.SoundCardConfig(bits=None),
    )
    y = ch.FmLink(cfg).process(x * g)
    d = me.delay(x, y, 2000)
    skip = int(0.2 * FS)
    reading = me.sndr(x[skip : len(y) - d], y[d + skip :], (450, 2850))
    assert reading.effective_db == pytest.approx(24.6, abs=1.0)
