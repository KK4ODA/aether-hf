"""A labelled corpus for the receive filter's reading, and the reading judged over it.

The receive-filter review (``docs/research/receive-filter/``), Gate 0 item 3. Each clip is 48 kHz
mono audio as a transceiver's codec delivers it with nobody on the channel: the band's noise
through a receive response whose shape is known — a filter of a given width, the same filter
moved by an IF shift, a low cut set high, a tilt, a notch — over the audio chain's own white
floor, under one condition: steady, an AGC answering a strong signal the audio does not carry,
QSB, a band full of FT8, or audio driven hot enough to clip. The labels say where the response
is 6 dB under its peak and how far down it is at the edges of the signal Aether sends at each
bandwidth (360 and 2640 Hz at 2300 Hz, 1260 and 1740 Hz at 500 Hz).

Gate 3 added two kinds (``03-candidates.md`` §4): *line* clips, a cut of 12, 15 or 20 dB at one
edge of one bandwidth's signal, steady, judged at that bandwidth only and against no criterion
(they show where the reading puts its line); and *joined* clips, a cut clip followed by its
uncut twin (``ssb2700`` under the same condition and floor) or the reverse, for how soon a
reading follows the operator's knob.

    python tools/rxfilter_corpus.py make OUT_DIR [--only NAME-PART] [--seed N] [--list]
    python tools/rxfilter_corpus.py judge OUT_DIR [--daemon PATH] [--csv FILE]
        [--only NAME-PART] [--jobs N]

``field`` runs the same over the nineteen field recordings (``01-evidence.md`` §2) found under
``INBOX`` (``field/inbox/``, git-ignored), each with its sidecar, and judges them against what
the review knows of each (K2, K4).

    python tools/rxfilter_corpus.py field INBOX [--daemon PATH] [--csv FILE] [--jobs N]

``harness`` runs the same over both stations' recordings of every run of the cut grid
(``tools/rxfilter_cost.py OUT --keep-audio``): the called station's radio is the one cut (its
side and depth in the run's name), the caller's is not (K1, K2, K6 on the harness), and it sets
the first frame a witness could read the cut edge from beside the first warning (K8).

    python tools/rxfilter_corpus.py harness RUNS_DIR [--daemon PATH] [--csv FILE] [--jobs N]

``judge`` runs ``aetherd --passband-trace`` over every clip at both bandwidths (no sidecar: the
whole clip is listened to) and judges every reading against the truth: a side 20 dB or more down
must be named, a side 10 dB or less down must not be, a side between is free. It prints the
criteria's counts (K1, K2, K3, K5, K6; ADR-0105 §4) and writes one row a clip and bandwidth.

The building blocks are the busy detector's corpus's (``tools/busy_corpus.py``): its FT8
stations, its AGC model and its WAV writer.
"""

from __future__ import annotations

import argparse
import csv
import json
import math
import multiprocessing
import os
import subprocess
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))

from busy_corpus import AGC_PRESETS, Source, source_audio, write_wav
from channel_server import Agc

AUDIO_RATE = 48_000
NOISE_3K_DBFS = -30.0
"""The band's noise in 3 kHz of audio, dBFS: a codec set for Setup step 3's *Good*."""
HOT_NOISE_RMS_DBFS = -8.0
"""The whole audio's RMS when the codec is driven hot, dBFS: the mobile on 2026-11-08 (−7 to
−8 dBFS over its recordings, 1.3–1.7 % of samples at full scale)."""
SIGNAL_EDGES_HZ = {2300: (360.0, 2640.0), 500: (1260.0, 1740.0)}
"""The edges of what Aether sends at each bandwidth: its occupied width around 1500 Hz."""
HEADROOM = 10 ** (-1.0 / 20.0)
"""The highest peak a clip outside the hot condition reaches: −1 dBFS, turned down to it."""
FLOORS_DB = {"deep": 50.0, "shallow": 15.0}
"""How far the audio chain's own floor lies under the band's noise, in 3 kHz each: the FTDX10's
about 50 dB, the mobile's TS-480 on 2026-11-08 about 13 dB."""
DEFAULT_DAEMON = Path(__file__).resolve().parents[1] / "core" / "target" / "release" / "aetherd"


@dataclass
class Response:
    """A receive response: a band-pass of ``low``–``high`` Hz (eighth order), then a tilt and
    a notch. ``low`` at or under 30 Hz is a low-pass alone, as a passband moved down by an IF
    shift reaches the bottom of the audio."""

    low: float
    high: float
    tilt_db_per_khz: float = 0.0
    """From 1 kHz up, the response falls this much a kilohertz (a negative tilt rises)."""
    notch: tuple[float, float, float] | None = None
    """(centre Hz, width Hz, depth dB)."""

    def gain_db(self, hz: np.ndarray) -> np.ndarray:
        """The response's power gain at ``hz``, dB."""
        hz = np.maximum(np.asarray(hz, dtype=float), 1e-3)
        order = 8
        high = 1.0 / (1.0 + (hz / self.high) ** (2 * order))
        low = 1.0 if self.low <= 30.0 else 1.0 / (1.0 + (self.low / hz) ** (2 * order))
        db = 10 * np.log10(high * low)
        db -= self.tilt_db_per_khz * np.maximum(hz - 1000.0, 0.0) / 1000.0
        if self.notch is not None:
            centre, width, depth = self.notch
            db -= depth * np.exp(-0.5 * ((hz - centre) / (width / 2.355)) ** 2)
        return db


@dataclass
class Clip:
    """One recording's worth of conditions."""

    name: str
    response: Response
    condition: str  # steady, agc, qsb, ft8, hot
    floor: str  # deep, shallow
    seconds: float = 30.0
    seed: int = 1
    labels: dict = field(default_factory=dict)
    kind: str = "corpus"  # corpus, line, joined
    bandwidths: tuple[int, ...] = (2300, 500)
    """The bandwidths the clip is judged at."""
    then: Clip | None = None
    """A joined clip's second part, heard straight after this one."""


RESPONSES: dict[str, Response] = {
    "ssb2700": Response(300.0, 3000.0),
    "ssb2400": Response(300.0, 2700.0),
    "ssb2100": Response(400.0, 2500.0),
    "ssb1800": Response(600.0, 2400.0),
    "narrow500": Response(1250.0, 1750.0),
    "shift-down-300": Response(30.0, 2700.0),
    "shift-down-600": Response(30.0, 2400.0),
    "shift-down-1100": Response(30.0, 1600.0),  # the mobile's TS-480 on 2026-11-08
    "shift-up-300": Response(600.0, 3300.0),
    "shift-up-600": Response(900.0, 3600.0),
    "lowcut600": Response(600.0, 3000.0),
    "tilt": Response(300.0, 3000.0, tilt_db_per_khz=3.0),  # the FTDX10's gentle slope
    "notch1800": Response(300.0, 3000.0, notch=(1800.0, 60.0, 30.0)),
}
CONDITIONS = ("steady", "agc", "qsb", "ft8", "hot")
SECONDS = {"ft8": 150.0, "agc": 120.0}
"""How long a clip runs, by condition (30 s otherwise): the busy detector calls a band of FT8
busy 86 % of the time and an AGC's dips 75 %, and the estimate takes its looks only in the
quiet, so a live station needs a minute or two to read there."""


def truth(response: Response) -> dict:
    """Where the response is 6 dB under its peak (outward from the peak), and how far down it
    is at the edges of the signal at each bandwidth, both relative to its peak (the notch left
    out: a notch inside the passband is not its edge)."""
    plain = Response(response.low, response.high, response.tilt_db_per_khz)
    hz = np.arange(0.0, 4000.0, 1.0)
    db = plain.gain_db(hz)
    peak_at = int(np.argmax(db))
    peak = float(db[peak_at])
    under = db < peak - 6.0
    above = np.nonzero(under[peak_at:])[0]
    below = np.nonzero(under[: peak_at + 1][::-1])[0]
    high = float(peak_at + above[0]) if len(above) else 4000.0
    low = float(peak_at - below[0]) if len(below) else 0.0
    out = {"low_6db": low, "high_6db": high}
    for bandwidth, (a, b) in SIGNAL_EDGES_HZ.items():
        out[f"loss_low_{bandwidth}"] = round(peak - float(plain.gain_db(np.array([a]))[0]), 1)
        out[f"loss_high_{bandwidth}"] = round(peak - float(plain.gain_db(np.array([b]))[0]), 1)
    return out


LINE_DEPTHS_DB = (12.0, 15.0, 20.0)
"""The line clips' cuts at the signal's edge (``03-candidates.md`` §4 registered 12 and 15; 20 is
added so the 500 Hz air, where no corpus filter cuts 20 dB, has a cut the criteria would warn
of)."""
JOINED = ("shift-down-1100", "ssb1800", "narrow500")
"""The cut filters joined to their uncut twin: the top, the bottom, and both, at 2300 Hz."""
JOINED_SECONDS = {"steady": (40.0, 90.0), "ft8": (120.0, 120.0)}
"""How long each part of a joined clip runs, by condition."""
UNCUT = "ssb2700"


def edge_cut(bandwidth: int, edge: str, depth_db: float) -> Response:
    """An eighth-order filter whose skirt is ``depth_db`` down at one edge of ``bandwidth``'s
    signal, the other edge in a passband 300–3000 Hz wide."""
    low_edge, high_edge = SIGNAL_EDGES_HZ[bandwidth]
    ratio = (10 ** (depth_db / 10.0) - 1.0) ** (1.0 / 16.0)
    if edge == "top":
        return Response(200.0, high_edge / ratio)
    return Response(low_edge * ratio, 3000.0)


def corpus(seed: int = 0, additions: bool = True) -> list[Clip]:
    """Every filter under every condition, over both chain floors; and, with ``additions``, the
    line and joined clips."""
    clips = []
    for k, (rname, response) in enumerate(RESPONSES.items()):
        for c, condition in enumerate(CONDITIONS):
            for f, floor in enumerate(FLOORS_DB):
                clips.append(
                    Clip(
                        name=f"{rname}-{condition}-{floor}",
                        response=response,
                        condition=condition,
                        floor=floor,
                        seconds=SECONDS.get(condition, 30.0),
                        seed=1 + seed * 10_000 + k * 100 + c * 10 + f,
                        labels=truth(response),
                    )
                )
    if not additions:
        return clips
    for b, bandwidth in enumerate(SIGNAL_EDGES_HZ):
        for e, edge in enumerate(("top", "bottom")):
            for d, depth in enumerate(LINE_DEPTHS_DB):
                response = edge_cut(bandwidth, edge, depth)
                for f, floor in enumerate(FLOORS_DB):
                    clips.append(
                        Clip(
                            name=f"line-{edge}-{depth:.0f}-{bandwidth}-steady-{floor}",
                            response=response,
                            condition="steady",
                            floor=floor,
                            seed=50_001 + seed * 10_000 + b * 1000 + e * 100 + d * 10 + f,
                            labels=truth(response),
                            kind="line",
                            bandwidths=(bandwidth,),
                        )
                    )
    for k, cut in enumerate(JOINED):
        for c, (condition, (first_s, second_s)) in enumerate(JOINED_SECONDS.items()):
            for f, floor in enumerate(FLOORS_DB):
                for r, (a, b) in enumerate(((cut, UNCUT), (UNCUT, cut))):
                    base = 70_001 + seed * 10_000 + k * 1000 + c * 100 + f * 10 + r * 2
                    second = Clip(
                        name=b,
                        response=RESPONSES[b],
                        condition=condition,
                        floor=floor,
                        seconds=second_s,
                        seed=base + 1,
                        labels=truth(RESPONSES[b]),
                    )
                    clips.append(
                        Clip(
                            name=f"joined-{a}-then-{b}-{condition}-{floor}",
                            response=RESPONSES[a],
                            condition=condition,
                            floor=floor,
                            seconds=first_s,
                            seed=base,
                            labels=truth(RESPONSES[a]),
                            kind="joined",
                            bandwidths=(2300,),
                            then=second,
                        )
                    )
    return clips


# ── building the audio ────────────────────────────────────────────────


def shaped(white: np.ndarray, response: Response) -> np.ndarray:
    """White noise through the response, by FFT over the whole clip."""
    spectrum = np.fft.rfft(white)
    hz = np.fft.rfftfreq(len(white), 1.0 / AUDIO_RATE)
    return np.fft.irfft(spectrum * 10 ** (response.gain_db(hz) / 20.0), len(white))


def build(clip: Clip) -> tuple[np.ndarray, dict]:
    """The clip's audio and its labels; a joined clip's parts one after the other, its labels
    the first part's with the second's under ``then`` and the join's time under ``join_s``."""
    audio, labels = build_part(clip)
    labels.update(kind=clip.kind, bandwidths=list(clip.bandwidths))
    if clip.then is not None:
        second, then = build_part(clip.then)
        labels.update(join_s=clip.seconds, then=then, seconds=clip.seconds + clip.then.seconds)
        labels["clipped_share"] = float(np.mean(np.abs(np.concatenate([audio, second])) >= 1.0))
        audio = np.concatenate([audio, second])
    return audio, labels


def build_part(clip: Clip) -> tuple[np.ndarray, dict]:
    """One part's audio and labels."""
    n = int(clip.seconds * AUDIO_RATE)
    rng = np.random.default_rng(clip.seed)
    band = shaped(rng.standard_normal(n), clip.response)
    # the band's noise at NOISE_3K_DBFS in 3 kHz of the response's flat top
    flat = Response(clip.response.low, clip.response.high)
    hz = np.fft.rfftfreq(n, 1.0 / AUDIO_RATE)
    top = float(np.max(flat.gain_db(hz)))
    white_3k = 3000.0 / (AUDIO_RATE / 2.0)  # the share of a white noise's power in 3 kHz
    band *= 10 ** ((NOISE_3K_DBFS - top) / 20.0) / math.sqrt(white_3k)
    felt = np.zeros(n)
    t = np.arange(n) / AUDIO_RATE
    if clip.condition == "qsb":
        band *= 10 ** (6.0 * np.sin(2 * np.pi * t / 8.0) / 20.0)
    if clip.condition == "ft8":
        srng = np.random.default_rng(clip.seed + 3)
        for k in range(10):
            source = Source("ft8", float(srng.uniform(400.0, 2600.0)), float(srng.uniform(3, 15)))
            clean = source_audio(source, n, clip.seed * 100 + k)
            band += shaped(clean, clip.response) * 10 ** ((NOISE_3K_DBFS + source.snr_3k_db) / 20.0)
    if clip.condition == "agc":
        # a strong station the roofing filter passes and the DSP filter does not: the AGC
        # answers to it and takes the band's noise down with it, the chain's floor not
        noise_rms_3k = 10 ** (NOISE_3K_DBFS / 20.0)
        windows = [(float(s), float(s) + 1.5) for s in np.arange(2.0, clip.seconds, 5.0)]
        source = Source("carrier", 3600.0, 30.0, windows=windows)
        felt = source_audio(source, n, clip.seed) * noise_rms_3k * 10 ** (30.0 / 20.0)
        hang, decay = AGC_PRESETS["mid"]
        agc = Agc(noise_rms_3k * 10 ** (12.0 / 20.0), hang, decay)
        steps = n // Agc.STEP * Agc.STEP
        gains = np.ones(n)
        gains[:steps] = np.repeat(10 ** (agc.gains_db((band + felt)[:steps]) / 20.0), Agc.STEP)
        band *= gains
    floor_3k_dbfs = NOISE_3K_DBFS - FLOORS_DB[clip.floor]
    chain = rng.standard_normal(n) * 10 ** (floor_3k_dbfs / 20.0) / math.sqrt(white_3k)
    audio = band + chain
    if clip.condition == "hot":
        audio *= 10 ** (HOT_NOISE_RMS_DBFS / 20.0) / math.sqrt(np.mean(audio**2))
    elif np.max(np.abs(audio)) > HEADROOM:
        # Gate 3: the FT8 band's ten stations summed reached full scale (1.3 % of the
        # samples in one clip), and clipping is the hot condition's alone
        audio *= HEADROOM / np.max(np.abs(audio))
    labels = {
        "name": clip.name,
        "response": clip.response.__dict__,
        "condition": clip.condition,
        "floor": clip.floor,
        "floor_db_under_noise": FLOORS_DB[clip.floor],
        "seconds": clip.seconds,
        "clipped_share": float(np.mean(np.abs(audio) >= 1.0)),
        **clip.labels,
    }
    return np.clip(audio, -1.0, 1.0), labels


def write_clip(job: tuple[Clip, Path]) -> str:
    clip, out = job
    audio, labels = build(clip)
    write_wav(out / f"{clip.name}.wav", audio)
    (out / f"{clip.name}.json").write_text(json.dumps(labels, indent=1), encoding="utf-8")
    return clip.name


# ── judging the candidates ────────────────────────────────────────────

WARNINGS = {"top": {"top"}, "bottom": {"bottom"}, "both": {"top", "bottom"}}
"""The sides each warning names."""
CUT_DB = 20.0
"""A side this far down must be named (K1)."""
SILENT_DB = 10.0
"""A side this far down or less must not be (K2, K3)."""
WITHIN_S = {"steady": 15.0, "ft8": 60.0}
"""How soon the warning must come, by condition (K5); the others are reported, not judged."""
CLEARS_WITHIN_S = 60.0
"""How soon after a cut is taken away its warning must clear (K5)."""
CLIP_SHARE = 0.001
"""C1's line, as the daemon's ``passband::CLIP_SHARE``."""
CLIP_WITHIN_S = 10.0
"""How soon C1 must say so (K6)."""


def sides(labels: dict, bandwidth: int) -> tuple[set[str], set[str]]:
    """The sides that must be named, and those that must not, at ``bandwidth``."""
    losses = {
        "bottom": labels[f"loss_low_{bandwidth}"],
        "top": labels[f"loss_high_{bandwidth}"],
    }
    need = {side for side, loss in losses.items() if loss >= CUT_DB}
    forbid = {side for side, loss in losses.items() if loss <= SILENT_DB}
    return need, forbid


def right(verdict: str, need: set[str], forbid: set[str]) -> bool:
    """Whether a reading says what the truth asks: every side cut named and none uncut, or, with
    nothing cut, no side named that is not free."""
    if verdict not in WARNINGS:
        return not need
    named = WARNINGS[verdict]
    return need <= named and not named & forbid


def wrong_side(verdict: str, forbid: set[str]) -> bool:
    """Whether a warning names a side that is not cut, or warns with nothing cut (K2, K3)."""
    named = WARNINGS.get(verdict)
    return named is not None and bool(named & forbid)


def c1(readings: list[dict]) -> tuple[float | str, int]:
    """When C1 first said the audio clips, and how many readings said so."""
    clipping = [r for r in readings if float(r["clipping_pct"]) > 100 * CLIP_SHARE]
    return (float(clipping[0]["t_s"]) if clipping else "", len(clipping))


def trace(daemon: Path, wav: Path, bandwidth: int) -> list[dict]:
    """Every reading of ``aetherd --passband-trace`` over a clip."""
    out = subprocess.run(
        [str(daemon), "--passband-trace", str(wav), "--bandwidth", str(bandwidth)],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return list(csv.DictReader(out.splitlines()))


def settled(readings: list[dict], since: float, ok) -> float | None:
    """How long after ``since`` the readings became ``ok`` and stayed so to the end: the time
    of the first reading of the last run of them."""
    after = [r for r in readings if float(r["t_s"]) >= since]
    if not after or not ok(after[-1]["verdict"]):
        return None
    k = len(after) - 1
    while k > 0 and ok(after[k - 1]["verdict"]):
        k -= 1
    return round(float(after[k]["t_s"]) - since, 2)


def judge_one(job: tuple[Path, Path, int]) -> dict:
    daemon, labels_path, bandwidth = job
    labels = json.loads(labels_path.read_text(encoding="utf-8"))
    readings = trace(daemon, labels_path.with_suffix(".wav"), bandwidth)
    need, forbid = sides(labels, bandwidth)
    row = {
        "clip": labels["name"],
        "kind": labels.get("kind", "corpus"),
        "condition": labels["condition"],
        "floor": labels["floor"],
        "bandwidth": bandwidth,
        "loss_low": labels[f"loss_low_{bandwidth}"],
        "loss_high": labels[f"loss_high_{bandwidth}"],
        "hot": int(labels["condition"] == "hot"),
        "readings": len(readings),
    }
    last = readings[-1] if readings else None
    row.update(
        final=last["verdict"] if last else "",
        low_drop=last["low_drop_db"] if last else "",
        high_drop=last["high_drop_db"] if last else "",
        over_floor=last["over_floor_db"] if last else "",
        looks=last["looks"] if last else 0,
    )
    first_part = readings
    if "then" in labels:
        join = float(labels["join_s"])
        first_part = [r for r in readings if float(r["t_s"]) < join]
        then_need, then_forbid = sides(labels["then"], bandwidth)
        row["then"] = labels["then"]["name"]
        row["settles_s"] = settled(readings, join, lambda v: right(v, then_need, then_forbid))
    row["c1_first_s"], row["c1_readings"] = c1(readings)
    warned = [r for r in first_part if r["verdict"] in WARNINGS]
    ok = [r for r in first_part if r["verdict"] in WARNINGS and right(r["verdict"], need, forbid)]
    row.update(
        need="+".join(sorted(need)),
        first_warning_s=float(warned[0]["t_s"]) if warned else "",
        first_right_s=float(ok[0]["t_s"]) if need and ok else "",
        right_at_end=int(bool(first_part) and right(first_part[-1]["verdict"], need, forbid)),
        wrong_side=sum(wrong_side(r["verdict"], forbid) for r in first_part),
        false_warnings=sum(r["verdict"] in WARNINGS for r in first_part)
        if not need and forbid == {"top", "bottom"}
        else 0,
    )
    return row


def summary(rows: list[dict]) -> None:
    """The criteria's counts."""
    if rows:
        mine = [r for r in rows if r["kind"] == "corpus"]
        cut = [r for r in mine if r["need"] and not r["hot"]]
        detected = [r for r in cut if r["first_right_s"] != "" and r["right_at_end"]]
        silent = [
            r
            for r in mine
            if not r["need"]
            and not r["hot"]
            and r["loss_low"] <= SILENT_DB
            and r["loss_high"] <= SILENT_DB
        ]
        false = [r for r in silent if r["false_warnings"]]
        wrong = [r for r in mine if not r["hot"] and r["wrong_side"]]
        late = [
            r
            for r in detected
            if r["condition"] in WITHIN_S and float(r["first_right_s"]) > WITHIN_S[r["condition"]]
        ]
        timed = [r for r in detected if r["condition"] in WITHIN_S]
        joined = [r for r in rows if r["kind"] == "joined"]
        clears = [r for r in joined if r["then"] == UNCUT]
        cleared = [
            r
            for r in clears
            if r.get("settles_s") not in (None, "") and float(r["settles_s"]) <= CLEARS_WITHIN_S
        ]
        cuts_after = [r for r in joined if r["then"] != UNCUT]
        warned_after = [r for r in cuts_after if r.get("settles_s") not in (None, "")]
        hot = [r for r in mine if r["hot"]]
        hot_warns = [r for r in hot if r["first_warning_s"] != ""]
        c1_hot = [r for r in hot if r["c1_first_s"] != "" and r["c1_first_s"] <= CLIP_WITHIN_S]
        c1_other = [r for r in rows if not r["hot"] and r["c1_first_s"] != ""]
        print(
            f"\nK1 {len(detected)} of {len(cut)} cuts named right by the end;"
            f" K2 {len(false)} of {len(silent)} uncut clips warned;"
            f" K3 {len(wrong)} clips with a warning naming a side not cut;"
            f" K5 {len(timed) - len(late)} of {len(timed)} in time,"
            f" {len(cleared)} of {len(clears)} joined clips cleared within"
            f" {CLEARS_WITHIN_S:.0f} s, {len(warned_after)} of {len(cuts_after)} warned of a"
            f" cut made; hot clips warning {len(hot_warns)} of {len(hot)};"
            f" K6 C1 on {len(c1_hot)} of {len(hot)} hot clips in time, on"
            f" {len(c1_other)} others"
        )
        for r in sorted(cut, key=lambda r: r["clip"]):
            if r not in detected:
                print(
                    f"    missed {r['clip']} at {r['bandwidth']}: ended {r['final']}"
                    f" ({r['low_drop']}/{r['high_drop']} dB, {r['looks']} looks)"
                )
        for r in sorted(false, key=lambda r: r["clip"]):
            print(
                f"    false {r['clip']} at {r['bandwidth']}: {r['false_warnings']} readings,"
                f" ended {r['final']} ({r['low_drop']}/{r['high_drop']} dB)"
            )
        lines = [r for r in rows if r["kind"] == "line"]
        for r in sorted(lines, key=lambda r: r["clip"]):
            print(f"    line {r['clip']}: {r['final']} {r['low_drop']}/{r['high_drop']} dB")
        for r in sorted(joined, key=lambda r: r["clip"]):
            print(
                f"    joined {r['clip']}: first {r['first_right_s'] or r['first_warning_s']}"
                f" then settles {r.get('settles_s')} s after the join, ended {r['final']}"
            )


def judge(out: Path, daemon: Path, table: Path | None, only: str | None, jobs: int) -> None:
    work = []
    for labels_path in sorted(out.glob("*.json")):
        labels = json.loads(labels_path.read_text(encoding="utf-8"))
        if only and only not in labels["name"]:
            continue
        for bandwidth in labels.get("bandwidths", [2300, 500]):
            work.append((daemon, labels_path, int(bandwidth)))
    rows = []
    with multiprocessing.Pool(jobs) as pool:
        for row in pool.imap_unordered(judge_one, work):
            rows.append(row)
    rows.sort(key=lambda r: (r["clip"], r["bandwidth"]))
    summary(rows)
    if table is not None:
        fields = list(dict.fromkeys(k for r in rows for k in r))
        with table.open("w", newline="", encoding="utf-8") as f:
            writer = csv.DictWriter(f, fieldnames=fields)
            writer.writeheader()
            writer.writerows(rows)


# ── the field recordings ──────────────────────────────────────────────

WORKING = "working"
FIELD: dict[str, str] = {
    # the gateway's nine, the FTDX10 (its own slope 5–9 dB down at 2.6–2.8 kHz)
    "20261106-223113_KK4ODA-1_KK4ODA": WORKING,
    "20261106-224135_KK4ODA-1_KK4ODA": WORKING,
    "20261107-135324_KK4ODA-1_KK4ODA": WORKING,
    "20261107-150555_KK4ODA-1_KK4ODA": WORKING,
    "20261108-005452_KK4ODA-1_KK4ODA": WORKING,
    "20261108-010116_KK4ODA-1_KK4ODA": WORKING,
    "20261108-010443_KK4ODA-1_KK4ODA": WORKING,
    "20261108-011000_KK4ODA-1_KK4ODA": WORKING,
    "20261108-012424_KK4ODA_WD5EMA": WORKING,
    # the mobile's four of 2026-11-06/07 and its quiet minute with the IF shift centred
    "20261106-223113_KK4ODA_KK4ODA-1": WORKING,
    "20261106-224142_KK4ODA_KK4ODA-1": WORKING,
    "20261107-135330_KK4ODA_KK4ODA-1": WORKING,
    "20261107-150601_KK4ODA_KK4ODA-1": WORKING,
    "20261108-163057_KK4ODA-2_listen": WORKING,
    # Saturday night, the IF shift left: the top cut at 2300 Hz, about 8 dB at 1740 Hz at 500
    "20261108-005459_KK4ODA_KK4ODA-1": "top",
    "20261108-010450_KK4ODA_KK4ODA-1": "top",
    "20261108-011008_KK4ODA_KK4ODA-1": "top",
    "20261108-010123_KK4ODA_KK4ODA-1": "high 8",
    "20261108-012431_WD5EMA_KK4ODA": "high 8",
}
"""What the review knows of each field recording: a working station (no passband warning, K2),
a cut side that must be named before the recording ends (K4), or the witness's drop at an edge
the reading must place within 3 dB (K4)."""
WITNESS_WITHIN_DB = 3.0
SATURDAY = (
    "20261108-005459_",
    "20261108-010123_",
    "20261108-010450_",
    "20261108-011008_",
    "20261108-012431_",
)
"""The mobile's five sessions of Saturday night, every one clipped (K6)."""


def trace_field(daemon: Path, wav: Path) -> list[dict]:
    """Every reading over a field recording, with its sidecar."""
    out = subprocess.run(
        [str(daemon), "--passband-trace", str(wav), "--expect", str(wav.with_suffix(".json"))],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return list(csv.DictReader(out.splitlines()))


def field_one(job: tuple[Path, Path, str]) -> dict:
    daemon, wav, expect = job
    readings = trace_field(daemon, wav)
    c1_first, c1_readings = c1(readings)
    warned = [r for r in readings if r["verdict"] in WARNINGS]
    last = readings[-1] if readings else {}
    row = {
        "recording": wav.stem,
        "expect": expect,
        "readings": len(readings),
        "warnings": len(warned),
        "first_warning_s": float(warned[0]["t_s"]) if warned else "",
        "warned_sides": "+".join(sorted({r["verdict"] for r in warned})),
        "final": last.get("verdict", ""),
        "low_drop": last.get("low_drop_db", ""),
        "high_drop": last.get("high_drop_db", ""),
        "looks": last.get("looks", 0),
        "unknown": sum(r["verdict"] == "unknown" for r in readings),
        "c1_first_s": c1_first,
        "c1_readings": c1_readings,
    }
    highs = [float(r["high_drop_db"]) for r in readings if r["high_drop_db"]]
    row["median_high_drop"] = round(float(np.median(highs)), 2) if highs else ""
    if expect == WORKING:
        row["passes"] = int(not warned)
    elif expect.startswith("high "):
        target = float(expect.split()[1])
        row["passes"] = int(
            not warned
            and last.get("high_drop_db", "") != ""
            and abs(float(last["high_drop_db"]) - target) <= WITNESS_WITHIN_DB
        )
    else:
        row["passes"] = int(any(r["verdict"] in (expect, "both") for r in readings))
    return row


def field(inbox: Path, daemon: Path, table: Path | None, jobs: int) -> None:
    work = []
    for name, expect in FIELD.items():
        found = sorted(inbox.rglob(f"{name}.wav"))
        if not found:
            print(f"    {name}: not under {inbox}")
            continue
        work.append((daemon, found[0], expect))
    with multiprocessing.Pool(jobs) as pool:
        rows = list(pool.imap_unordered(field_one, work))
    rows.sort(key=lambda r: (r["expect"] != WORKING, r["recording"]))
    if rows:
        mine = rows
        working = [r for r in mine if r["expect"] == WORKING]
        cut = [r for r in mine if r["expect"] != WORKING]
        saturday = [r for r in mine if r["recording"].startswith(SATURDAY)]
        c1_ok = [r for r in saturday if r["c1_first_s"] != "" and r["c1_first_s"] <= CLIP_WITHIN_S]
        c1_other = [r for r in mine if r not in saturday and r["c1_first_s"] != ""]
        print(
            f"\nK2 {sum(not r['passes'] for r in working)} of {len(working)} working"
            f" recordings warned; K4 {sum(r['passes'] for r in cut)} of {len(cut)};"
            f" K6 C1 on {len(c1_ok)} of {len(saturday)} of Saturday's in time, on"
            f" {len(c1_other)} others"
        )
        for r in mine:
            print(
                f"    {r['recording']:34s} {r['expect']:8s} {'ok ' if r['passes'] else 'NO '}"
                f" {r['warnings']:3d}/{r['readings']:<4d} warned {r['warned_sides'] or '-':12s}"
                f" first {r['first_warning_s']!s:8s} ended {r['final']:8s}"
                f" {r['low_drop']}/{r['high_drop']} dB (median top {r['median_high_drop']}),"
                f" {r['looks']} looks, {r['unknown']} unknown, C1 first {r['c1_first_s']}"
            )
    if table is not None:
        with table.open("w", newline="", encoding="utf-8") as f:
            writer = csv.DictWriter(f, fieldnames=list(rows[0]))
            writer.writeheader()
            writer.writerows(rows)


# ── the harness's recordings ──────────────────────────────────────────


def filter_losses(
    low: float, high: float, low_edge: float, high_edge: float
) -> tuple[float, float]:
    """How far the channel server's receive filter is down at each edge, against its peak."""
    from scipy.signal import butter, sosfreqz

    if low <= 30.0:
        sos = butter(8, high, btype="lowpass", fs=AUDIO_RATE, output="sos")
    else:
        sos = butter(8, [low, high], btype="bandpass", fs=AUDIO_RATE, output="sos")
    hz = np.concatenate([[low_edge, high_edge], np.linspace(100.0, 4000.0, 3901)])
    _, h = sosfreqz(sos, worN=hz, fs=AUDIO_RATE)
    db = 20 * np.log10(np.maximum(np.abs(h), 1e-12))
    peak = float(db[2:].max())
    return peak - float(db[0]), peak - float(db[1])


def run_truth(run: Path) -> dict | None:
    """What the cut grid set for the called station's radio (``stations.b.radio``): each edge's
    loss through the filter the channel server builds from it (``channel_server.receiver``: a
    low-pass alone under 30 Hz, a band-pass otherwise, whose lower skirt is steeper than a
    low-pass prototype's), against its peak, and whether the codec was driven hot. ``None`` for
    a run that is not the grid's."""
    path = run / "scenario.toml"
    if not run.name.startswith("rxcost-") or not path.exists():
        return None
    scenario = tomllib.loads(path.read_text(encoding="utf-8"))
    bandwidth = int(scenario.get("bandwidth", 2300))
    radio = scenario.get("stations", {}).get("b", {}).get("radio", {})
    low_edge, high_edge = SIGNAL_EDGES_HZ[bandwidth]
    band = radio.get("rx_filter_hz")
    if band is None:
        loss_low = loss_high = 0.0
    else:
        loss_low, loss_high = filter_losses(float(band[0]), float(band[1]), low_edge, high_edge)
    return {
        "bandwidth": bandwidth,
        "loss_low": round(loss_low, 1),
        "loss_high": round(loss_high, 1),
        "hot": int(float(radio.get("rx_gain_db", 0.0)) > 0.0),
    }


def harness_one(job: tuple[Path, Path, dict]) -> dict:
    daemon, wav, truth_ = job
    readings = trace_field(daemon, wav)
    bandwidth = truth_["bandwidth"]
    losses = {
        f"loss_low_{bandwidth}": truth_["loss_low"],
        f"loss_high_{bandwidth}": truth_["loss_high"],
    }
    need, forbid = sides(losses, bandwidth)
    warned = [r for r in readings if r["verdict"] in WARNINGS]
    ok = [r for r in warned if right(r["verdict"], need, forbid)]
    c1_first, c1_readings = c1(readings)
    sidecar = json.loads(wav.with_suffix(".json").read_text(encoding="utf-8"))
    frames = sorted(sidecar.get("frames") or [], key=lambda f: f.get("t_s", 0.0))
    # a witness reads the edges only from frames that span them: OFDM data, or the fast tones
    # at 2300 Hz (rungs 2-5); slow tones and every 500 Hz tone frame span 1300-1700 Hz only
    edges = [
        f
        for f in frames
        if f.get("decoded")
        and f.get("kind") == "data"
        and int(f.get("mode", 0)) >= (2 if bandwidth == 2300 else 5)
    ]
    last = readings[-1] if readings else {}
    return {
        "run": truth_["run"],
        "station": truth_["station"],
        "bandwidth": bandwidth,
        "need": "+".join(sorted(need)),
        "silent": int(forbid == {"top", "bottom"}),
        "hot": truth_["hot"],
        "loss_low": truth_["loss_low"],
        "loss_high": truth_["loss_high"],
        "readings": len(readings),
        "warnings": len(warned),
        "wrong_side": sum(wrong_side(r["verdict"], forbid) for r in readings),
        "first_warning_s": float(warned[0]["t_s"]) if warned else "",
        "first_right_s": float(ok[0]["t_s"]) if need and ok else "",
        "right_at_end": int(bool(readings) and right(last.get("verdict", ""), need, forbid)),
        "final": last.get("verdict", ""),
        "low_drop": last.get("low_drop_db", ""),
        "high_drop": last.get("high_drop_db", ""),
        "c1_first_s": c1_first,
        "c1_readings": c1_readings,
        "first_frame_s": frames[0]["t_s"] if frames else "",
        "first_edge_frame_s": edges[0]["t_s"] if edges else "",
    }


def harness(runs: Path, daemon: Path, table: Path | None, jobs: int) -> None:
    work = []
    for run in sorted(p for p in runs.iterdir() if p.is_dir()):
        called = run_truth(run)
        if called is None:
            continue
        for station in ("a", "b"):
            for wav in sorted((run / station / "recordings").glob("*.wav")):
                # the caller's radio is left alone
                truth_ = dict(called, run=run.name, station=station)
                if station == "a":
                    truth_.update(loss_low=0.0, loss_high=0.0, hot=0)
                work.append((daemon, wav, truth_))
    with multiprocessing.Pool(jobs) as pool:
        rows = list(pool.imap_unordered(harness_one, work))
    rows.sort(key=lambda r: (r["run"], r["station"]))
    if rows:
        mine = rows
        cut = [r for r in mine if r["need"] and not r["hot"]]
        named = [r for r in cut if r["first_right_s"] != "" and r["right_at_end"]]
        silent = [r for r in mine if r["silent"] and not r["hot"]]
        false = [r for r in silent if r["warnings"]]
        wrong = [r for r in mine if r["wrong_side"]]
        hot = [r for r in mine if r["hot"]]
        c1_hot = [r for r in hot if r["c1_first_s"] != "" and r["c1_first_s"] <= CLIP_WITHIN_S]
        c1_other = [r for r in mine if not r["hot"] and r["c1_first_s"] != ""]
        sooner = [
            r
            for r in named
            if r["first_edge_frame_s"] != "" and r["first_edge_frame_s"] < r["first_right_s"]
        ]
        print(
            f"\ncuts named {len(named)} of {len(cut)}; K2 {len(false)} of"
            f" {len(silent)} uncut or ≤10 dB recordings warned; {len(wrong)} naming a side not"
            f" cut; K6 C1 on {len(c1_hot)} of {len(hot)} hot in time, on {len(c1_other)} others;"
            f" K8 a frame spanning the cut decoded before the warning in {len(sooner)} of"
            f" {len(named)}"
        )
        for r in hot:
            print(
                f"    hot {r['run']} {r['station']}: C1 from {r['c1_first_s']} s, ended"
                f" {r['final']} {r['low_drop']}/{r['high_drop']} dB"
            )
        for r in mine:
            if (r in cut and r not in named) or r in false or r["wrong_side"]:
                print(
                    f"    {r['run']} {r['station']}: need {r['need'] or '-'} warned"
                    f" {r['warnings']}/{r['readings']} ended {r['final']}"
                    f" {r['low_drop']}/{r['high_drop']} dB"
                )
        for r in named:
            print(
                f"    {r['run']} {r['station']}: named at {r['first_right_s']} s, first frame"
                f" {r['first_frame_s']}, first spanning the edges {r['first_edge_frame_s']}"
            )
    if table is not None:
        with table.open("w", newline="", encoding="utf-8") as f:
            writer = csv.DictWriter(f, fieldnames=list(rows[0]))
            writer.writeheader()
            writer.writerows(rows)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="command", required=True)
    make = sub.add_parser("make", help="build the clips")
    make.add_argument("out", type=Path)
    make.add_argument("--only", help="build only clips whose name contains this")
    make.add_argument("--list", action="store_true", help="list the clips and stop")
    make.add_argument("--jobs", type=int, default=os.cpu_count() or 1)
    make.add_argument("--seed", type=int, default=0, help="draw the clips again")
    jud = sub.add_parser("judge", help="the reading over the clips")
    jud.add_argument("out", type=Path)
    jud.add_argument("--daemon", type=Path, default=DEFAULT_DAEMON)
    jud.add_argument("--csv", type=Path)
    jud.add_argument("--only", help="judge only clips whose name contains this")
    jud.add_argument("--jobs", type=int, default=os.cpu_count() or 1)
    fld = sub.add_parser("field", help="the reading over the field recordings")
    fld.add_argument("inbox", type=Path)
    fld.add_argument("--daemon", type=Path, default=DEFAULT_DAEMON)
    fld.add_argument("--csv", type=Path)
    fld.add_argument("--jobs", type=int, default=os.cpu_count() or 1)
    har = sub.add_parser("harness", help="the reading over the cut grid's recordings")
    har.add_argument("runs", type=Path)
    har.add_argument("--daemon", type=Path, default=DEFAULT_DAEMON)
    har.add_argument("--csv", type=Path)
    har.add_argument("--jobs", type=int, default=os.cpu_count() or 1)
    args = ap.parse_args()
    if args.command == "harness":
        harness(args.runs, args.daemon, args.csv, args.jobs)
        return
    if args.command == "field":
        field(args.inbox, args.daemon, args.csv, args.jobs)
        return
    if args.command == "judge":
        judge(args.out, args.daemon, args.csv, args.only, args.jobs)
        return
    clips = [c for c in corpus(args.seed) if not args.only or args.only in c.name]
    if args.list:
        for clip in clips:
            print(clip.name, json.dumps(clip.labels))
        return
    args.out.mkdir(parents=True, exist_ok=True)
    with multiprocessing.Pool(args.jobs) as pool:
        for name in pool.imap_unordered(write_clip, [(c, args.out) for c in clips]):
            print(name, flush=True)


if __name__ == "__main__":
    main()
