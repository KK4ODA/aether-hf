"""A numpy mirror of R3 (core/aetherd/src/passband.rs), for reading which looks put an edge where
it is. Run with python -I; it reads untrusted recordings only as data."""

from __future__ import annotations

import json
import math
import wave

import numpy as np

RATE = 48_000
WINDOW = 4096
BIN_HZ = RATE / WINDOW
TOP_HZ = 4000.0
FLOOR_BAND = (6000.0, 10_000.0)
LINE_DB = 12.0
EDGE_HALF_HZ = 50.0
LOOKS_NEEDED = 12
LOOKS_KEPT = 120
OCCUPIED_DB = 14.0
SIDE_Q = 0.25
SIDE_HZ = 150.0
SPREAD = 2
REF_Q = 0.9
SPANS = {2300: (360.0, 2640.0), 500: (1260.0, 1740.0)}
HANN = 0.5 - 0.5 * np.cos(2 * np.pi * np.arange(WINDOW) / (WINDOW - 1))
GAIN = HANN.sum() / 2.0


def read_wav(path):
    with wave.open(str(path), "rb") as w:
        assert w.getframerate() == RATE and w.getnchannels() == 1 and w.getsampwidth() == 2
        data = np.frombuffer(w.readframes(w.getnframes()), dtype="<i2")
    return data.astype(np.float32) / 32768.0


def rround(x):
    return int(math.floor(x + 0.5))


def quantile(values, q):
    v = np.asarray(values, dtype=float)
    v = v[~np.isnan(v)]
    if v.size == 0:
        return None
    at = rround((v.size - 1) * q)
    return float(np.partition(v, at)[at])


def between(bins, lo, hi):
    first = max(rround(lo / BIN_HZ), 0)
    last = min(rround(hi / BIN_HZ), len(bins) - 1)
    if first > last:
        return np.array([])
    return bins[first : last + 1]


def spectrum_at(audio, taken_s):
    end = int(round(taken_s * RATE))
    seg = audio[end - WINDOW : end].astype(np.float64)
    if seg.size < WINDOW:
        return None
    spec = np.fft.fft(seg * HANN)
    top = int(10_000.0 / BIN_HZ)
    amp = np.abs(spec[: top + 1]) / GAIN
    return (20 * np.log10(np.maximum(amp, 1e-12))).astype(np.float32)


def leave_out(look):
    side = max(rround(SIDE_HZ / BIN_HZ), 1)
    n = len(look)
    held = []
    for k in range(n):
        below = quantile(look[max(k - side, 0) : k], SIDE_Q) if k > 0 else None
        above = quantile(look[min(k + 1, n) : min(k + 1 + side, n)], SIDE_Q)
        if below is None and above is None:
            continue
        ok = all(m is None or look[k] - m > OCCUPIED_DB for m in (below, above))
        if ok:
            held.append(k)
    out = look.copy()
    for k in held:
        out[max(k - SPREAD, 0) : min(k + SPREAD + 1, n)] = np.nan
    return out


def fold(spec, span):
    """(look normalised or None, over_floor_db, why)."""
    floor = quantile(between(spec, *FLOOR_BAND), 0.5)
    top = int(TOP_HZ / BIN_HZ)
    look = leave_out(spec[: top + 1].copy())
    in_span = between(look, *span)
    if np.isfinite(in_span).sum() * 2 < len(in_span):
        return None, None, "held"
    level = quantile(between(look, *span), REF_Q)
    over = None if level is None or floor is None else level - floor
    if over is None or over < LINE_DB:
        return None, over, "weak"
    return look - level, over, "kept"


def profile(looks):
    arr = np.array(looks)
    n = len(looks)
    out = np.full(arr.shape[1], np.nan)
    for b in range(arr.shape[1]):
        col = arr[:, b]
        col = col[~np.isnan(col)]
        if col.size * 2 < n:
            continue
        out[b] = quantile(col, 0.5)
    return out


def smoothed_at(prof, k):
    half = rround(EDGE_HALF_HZ / BIN_HZ)
    window = prof[max(k - half, 0) : min(k + half + 1, len(prof))]
    col = window[~np.isnan(window)]
    if col.size * 2 < len(window):
        return None
    return quantile(col, 0.5)


def span_drops(prof, span):
    first = min(max(rround(span[0] / BIN_HZ), 0), len(prof) - 1)
    last = min(max(rround(span[1] / BIN_HZ), 0), len(prof) - 1)
    inside = [smoothed_at(prof, k) for k in range(first, last + 1)]
    inside = [v for v in inside if v is not None]
    ref = quantile(inside, REF_Q)
    lo, hi = smoothed_at(prof, first), smoothed_at(prof, last)
    if ref is None or lo is None or hi is None:
        return None
    return ref - lo, ref - hi


def verdict(lo, hi):
    a, b = lo >= LINE_DB, hi >= LINE_DB
    return {(True, True): "both", (False, True): "top", (True, False): "bottom"}.get((a, b), "clear")


def load_sidecar(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def profile_from_sidecar(entry):
    """A sidecar's passband.start/end as an array (NaN where null)."""
    if not entry:
        return None
    return np.array([np.nan if v is None else v for v in entry["db"]], dtype=float)
