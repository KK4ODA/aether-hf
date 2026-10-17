"""Both stations' recordings of one session, on one clock (ADR-0069 Step 4).

    python tools/two_sides.py A.json B.json [--band 2700:3000] [--window 60] [--offset S]

Each station's sidecar places what it heard on its own recording's clock and what it did —
keyed, released — on its own station clock. Neither says when the other station's radio was on
the air, and a slot's timing is a question about both. This puts the two on one clock and
measures what protocol 8's timing rests on, on real hardware (11-step-4-first-air.md):

* **The received-audio offset**: the lag at which the two recordings' envelopes agree in a band
  both stations hear the same — atmospheric crashes above the 500 Hz air's passband by default,
  or another station both heard (``--band 1790:1810`` for a PACTOR carrier) — over the stretches
  both were listening, window by window. The sidecars' ``started`` stamps give it to a few
  tenths; the audio to a millisecond. A step between windows is audio one recording lost.
* **Per keying**, on the station's own clock: the *handover delay* — release less key, less the
  key lead, the frames and the keyed tail — which is what blocked the run loop between the key
  and the burst reaching the sound card (a CAT round trip, rendering); the *mute onset*, where
  the radio's receive audio goes silent after the key in the station's own recording (the
  capture path and the radio's switch); and the *loop latency*: from the key to the first frame
  as the other station recorded it, put on this station's received-audio clock, less the lead,
  the card's reported latency and the handover delay. What is left is the radios' audio paths
  and the card beyond its report. A step in one station's loop latency mirrored in the other's
  is a step in the offset: audio lost.
* **Every keying of both stations** with its radiation where the other station recorded it,
  and the gap between the two stations' radiation each time the air changes hands — negative
  is an overlap.
* **Each answer against its slot**: the master's transmission ending at E grants a slot starting
  ``E + D + KEY + GUARD + LEVEL`` (no declared turnaround); the answer's lateness is its first
  frame less that, and its margin is what is left of ``SLACK`` — the keying gate's ``not_after``.
* **The offset from the loops** (``--from-loops``), for a session with no signal both stations
  heard — on 80 m by day the band is quiet between the two stations' transmissions
  (12-two-paths-80m.md §2): at each change of hands the offset at which the two stations' loop
  latencies come out equal. Their sum does not depend on the offset, and holds steady (130–140
  ms for these radios) whatever the clocks do; where a pair's sum strays the frame it was placed
  by is the wrong one, and the pair is left out. Two radios' paths are not quite equal, so the
  offset is good to a few milliseconds; its *changes* are exact: a step is audio one recording
  lost (or repeated), a slope is the two sound cards' clocks running at different rates. Each
  keying is then placed with the offset of its time, and the stations' own ``capture_skip``
  events (ADR-0071 §3) are listed beside the steps on the same clock.

Times are printed on the first sidecar's recording clock. Only the standard library, numpy and
the reference model: run from the repository with ``uv run --project model``.
"""

from __future__ import annotations

import argparse
import itertools
import json
import re
import statistics
import sys
import wave
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np
from numpy.typing import NDArray

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "model"))

from aether_model.frame.modes import NARROW, WIDE, AirInterface  # noqa: E402
from aether_model.link.granted import (  # noqa: E402
    DECODE_BUDGET_S,
    GUARD_S,
    KEY_S,
    LEVELS_S,
    SLACK_S,
)

KEY_LEAD_S = 0.1
"""The daemon's key lead (``[ptt] key_lead_s``), which no sidecar records."""
RATE = 1000
"""Envelope samples a second."""
MIN_WINDOW_S = 5.0
"""Seconds both stations must have listened in a window for its lag to be printed: in two or
three seconds of gaps the atmospherics correlate by chance at a lag tenths of a second off."""
MIN_WINDOW_R = 0.1
"""And the correlation it must reach: below it a window's best lag is chance."""
MUTED = 1e-8
"""Mean power under which a stretch of a recording is the radio's mute, not the band."""


@dataclass
class Keying:
    who: str
    key: float
    release: float
    frames_s: float
    tail_s: float
    latency_s: float
    handover_s: float
    mute_s: float | None = None
    rf_start: float | None = None
    """On the common clock, where the transmission's first frame was on the air as the other
    station recorded it; None if it recorded none of the transmission's frames."""
    loop_s: float | None = None
    sent: list[tuple[str, int, float]] = field(default_factory=list)
    """The frames it carried, in order: kind, rung (0 for a control frame) and air time."""

    @property
    def rf_end(self) -> float | None:
        return None if self.rf_start is None else self.rf_start + self.frames_s


@dataclass
class Side:
    name: str
    doc: dict
    audio: NDArray[np.float64]
    fs: int
    air: AirInterface
    keyings: list[Keying] = field(default_factory=list)


def load(path: Path) -> Side:
    doc = json.loads(path.read_text(encoding="utf-8"))
    with wave.open(str(path.with_suffix(".wav")), "rb") as w:
        fs = w.getframerate()
        raw = np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16)
        audio = raw.reshape(-1, w.getnchannels())[:, 0].astype(np.float64) / 32768.0
    air = NARROW if (doc.get("session") or {}).get("bandwidth_hz") == 500 else WIDE
    return Side(str((doc.get("session") or {}).get("callsign") or path.stem), doc, audio, fs, air)


def frame_s(air: AirInterface, sent: dict) -> float:
    """A sent frame's air time, from its kind, family and rung."""
    if sent.get("kind") == "control":
        return air.tone_control.duration_s if sent.get("floor") else air.short.duration_s
    return air.ladder[int(sent["mode"])].duration_s


def keyings(side: Side) -> list[Keying]:
    """Each keying, with the frames it carried — the ``sent`` records of the last batch handed
    over at or just before the key — and what its ``tx_end`` says of the tail and the card."""
    events = side.doc.get("events") or []
    sent = side.doc.get("sent") or []
    out: list[Keying] = []
    key: float | None = None
    for e in events:
        if e.get("event") == "ptt" and e.get("detail") == "keyed":
            key = float(e["t_s"])
        elif e.get("event") == "ptt" and e.get("detail") == "released" and key is not None:
            release = float(e["t_s"])
            tx_end = next(
                (
                    x["detail"]
                    for x in events
                    if x.get("event") == "tx_end" and abs(float(x["t_s"]) - release) < 1e-6
                ),
                "",
            )
            tail = re.search(r"tail ([\d.]+)", tx_end)
            latency = re.search(r"latency ([\d.]+)", tx_end)
            batch = [s for s in sent if key - 1.2 <= float(s["t_s"]) <= key + 1e-6]
            if batch:
                last = max(float(s["t_s"]) for s in batch)
                batch = [s for s in batch if abs(float(s["t_s"]) - last) < 1e-6]
            frames = sum(frame_s(side.air, s) for s in batch)
            tail_s = float(tail[1]) if tail else 0.1
            out.append(
                Keying(
                    side.name,
                    key,
                    release,
                    frames,
                    tail_s,
                    float(latency[1]) if latency else 0.0,
                    (release - key) - (KEY_LEAD_S + frames + tail_s),
                    sent=[
                        (str(s.get("kind")), int(s.get("mode") or 0), frame_s(side.air, s))
                        for s in batch
                    ],
                )
            )
            key = None
    return out


def mean_power(x: NDArray[np.float64], fs: int, seconds: float = 0.002) -> NDArray[np.float64]:
    k = max(1, int(seconds * fs))
    return np.convolve(x * x, np.ones(k) / k, mode="same")


def mute_onsets(side: Side) -> None:
    """Where each keying's receive mute begins in the station's own recording, after its key."""
    p = mean_power(side.audio, side.fs)
    for k in side.keyings:
        i = int(k.key * side.fs)
        quiet = np.flatnonzero(p[i : i + int(0.5 * side.fs)] < MUTED)
        k.mute_s = float(quiet[0]) / side.fs if len(quiet) else None


def envelope(
    side: Side, band: tuple[float, float]
) -> tuple[NDArray[np.float64], NDArray[np.bool_]]:
    """Log power in ``band`` at :data:`RATE` a second, and where the station was listening: not
    muted, and a quarter second clear of its own keyings either side."""
    x = side.audio
    n = len(x)
    spec = np.fft.rfft(x)
    f = np.fft.rfftfreq(n, 1.0 / side.fs)
    spec[(f < band[0]) | (f > band[1])] = 0.0
    y = np.fft.irfft(spec, n)
    step = side.fs // RATE
    k = max(1, side.fs // 200)
    env = np.convolve(y * y, np.ones(k) / k, mode="same")[::step]
    listening = mean_power(x, side.fs, 0.005)[::step] > MUTED
    for kk in side.keyings:
        a, b = int((kk.key - 0.25) * RATE), int((kk.release + 0.25) * RATE)
        listening[max(0, a) : max(0, b)] = False
    return np.log10(env + 1e-12), listening


def best_lag(
    ea: NDArray[np.float64],
    la: NDArray[np.bool_],
    eb: NDArray[np.float64],
    lb: NDArray[np.bool_],
    span: tuple[float, float],
    guess: float,
    search: float,
) -> tuple[float, float, int]:
    """The lag (a's clock less b's, for a signal both heard) at which the envelopes correlate
    best over ``span`` of a's clock: (lag, correlation, samples both were listening)."""
    best = (guess, -1.0, 0)
    ta = np.arange(int(span[0] * RATE), min(int(span[1] * RATE), len(ea)))
    for ms in range(int((guess - search) * 1000), int((guess + search) * 1000) + 1):
        tb = ta - ms
        ok = (tb >= 0) & (tb < len(eb))
        a, b = ta[ok], tb[ok]
        both = la[a] & lb[b]
        if both.sum() < RATE:
            continue
        u = ea[a][both] - ea[a][both].mean()
        v = eb[b][both] - eb[b][both].mean()
        den = float(np.sqrt((u * u).sum() * (v * v).sum()))
        r = float((u * v).sum()) / den if den > 0 else 0.0
        if r > best[1]:
            best = (ms / 1000.0, r, int(both.sum()))
    return best


def started_offset(a: Side, b: Side) -> float:
    """b's recording clock to a's, from the ``started`` stamps: each recording's clock reads its
    preroll when the recording started."""
    from datetime import datetime

    def t0(side: Side) -> float:
        stamp = datetime.fromisoformat(str(side.doc["started"]).replace("Z", "+00:00"))
        return stamp.timestamp() - float((side.doc.get("session") or {}).get("preroll_s") or 0.0)

    return t0(b) - t0(a)


Offset = float | Callable[[float], float]
"""b's recording clock to a's: one number, or one for each moment of b's clock (a session whose
offset steps or drifts, :func:`offset_along`)."""


def offset_of(offset: Offset, t_b: float) -> float:
    return offset(t_b) if callable(offset) else offset


PLACE_TOLERANCE_S = 0.03
"""How close a recorded frame must lie to where a candidate start puts its frame to count for
it: a frame's start is estimated to a few milliseconds."""


GUESS_SLACK_S = 0.3
"""How early before its key, on the offset given, a transmission's frames are looked for: the
offset a placement starts from may be the stamps', a few tenths off. A station's previous
transmission always ends seconds before its next key (the other station answered between), so
nothing else is in reach."""
RADIO_PATHS_S = 0.07
"""What the two radios' audio paths and the card beyond its report add between a key and the
first frame on the other station's recording: 61–78 ms measured on every pair so far
(11-step-4-first-air.md §4.1, 12-two-paths-80m.md §4.1) — only to choose between positions a
lone recorded frame could hold."""


@dataclass(frozen=True)
class Heard:
    """A frame the other station recorded of a transmission, on a's clock."""

    start: float
    kind: str
    rung: int
    decoded: bool = True
    follows: int | None = None
    """The exact count of the transmission's data frames after it (ADR-0068), saturating at
    :data:`MOST_FOLLOWS`; None where the frame carries none or did not decode."""


MOST_FOLLOWS = 3
"""The largest count a frame states: three or more follow."""


def transmission_start(k: Keying, heard: list[Heard], expected: float) -> float | None:
    """Where the transmission's first frame began, from the frames the other station recorded
    of it: each recorded frame is put at each position in the transmission of a frame of its
    kind and rung — a decoded data frame only where its count of frames to follow puts it — and
    the start that most *decoded* frames agree on wins; then the one most frames of any kind
    agree on, then the one nearest ``expected``, where the keying puts it. A first frame the
    other station missed no longer makes the second the transmission's start, a lone frame of
    a burst of one rung is placed by its count, and a detection that never decoded (a phantom
    a few milliseconds off, a frame read as another kind) cannot outvote one that did. ``None``
    when nothing recorded fits; then the earliest recorded frame is all there is."""
    if not heard:
        return None
    positions = []
    at = 0.0
    for kind, rung, seconds in k.sent:
        positions.append((kind, rung, at))
        at += seconds
    data_after = [
        sum(1 for kind, *_ in positions[j + 1 :] if kind == "data") for j in range(len(positions))
    ]

    def fits(h: Heard, j: int) -> bool:
        kind, rung, _ = positions[j]
        if h.kind != kind or (kind == "data" and h.rung != rung):
            return False
        if h.decoded and h.follows is not None and kind == "data":
            return min(data_after[j], MOST_FOLLOWS) == h.follows
        return True

    decoded = [h for h in heard if h.decoded]
    sources = decoded or heard
    candidates = [
        h.start - positions[j][2] for h in sources for j in range(len(positions)) if fits(h, j)
    ]
    if not candidates:
        return None

    def agreeing(t0: float, among: list[Heard]) -> int:
        return sum(
            any(
                fits(h, j) and abs(h.start - (t0 + positions[j][2])) < PLACE_TOLERANCE_S
                for j in range(len(positions))
            )
            for h in among
        )

    return max(
        candidates,
        key=lambda c: (agreeing(c, decoded), agreeing(c, heard), -abs(c - expected)),
    )


def place(a: Side, b: Side, offset: Offset) -> None:
    """Each keying's radiation on a's clock, from the frames the other station recorded of it
    (:func:`transmission_start`), and the loop latency it implies."""
    for side, other in ((a, b), (b, a)):
        for k in side.keyings:
            to_a = offset_of(offset, k.key) if side is b else 0.0
            heard = []
            for f in other.doc.get("frames") or []:
                start = float(f["start_s"])
                if other is b:
                    start += offset_of(offset, start)
                key = k.key + to_a
                if key - GUESS_SLACK_S < start < k.release + to_a + 0.5:
                    follows = f.get("follows")
                    # a decoded measurement frame (ADR-0077) is a data container, as the
                    # sending side records it
                    kind = "data" if f.get("kind") == "measure" else str(f.get("kind"))
                    heard.append(
                        Heard(
                            start,
                            kind,
                            int(f.get("mode") or 0),
                            bool(f.get("decoded", True)),
                            int(follows) if isinstance(follows, int) else None,
                        )
                    )
            if heard:
                expected = k.key + to_a + KEY_LEAD_S + k.latency_s + k.handover_s + RADIO_PATHS_S
                found = transmission_start(k, heard, expected)
                rf = found if found is not None else min(h.start for h in heard)
                k.rf_start = rf
                k.loop_s = rf - (k.key + to_a) - KEY_LEAD_S - k.latency_s - k.handover_s


def gaps(a: Side, b: Side) -> list[tuple[Keying, Keying, float]]:
    """Each change of hands on the air: the transmission before, the one after, and the gap."""
    placed = sorted(
        (k for k in a.keyings + b.keyings if k.rf_start is not None),
        key=lambda k: float(k.rf_start or 0.0),
    )
    out = []
    for x, y in itertools.pairwise(placed):
        if x.who != y.who and x.rf_end is not None and y.rf_start is not None:
            out.append((x, y, y.rf_start - x.rf_end))
    return out


def answers(master: Side, slave: Side, offset_slave: Offset) -> list[dict[str, float]]:
    """Each of the master's transmissions the slave answered: the answer's lateness against the
    slot its header granted (level from the header the slave decoded, 0 without one) and the
    margin left to ``not_after``. ``offset_slave`` takes the slave's recording clock to the
    common one."""
    gap = DECODE_BUDGET_S[0] + KEY_S + GUARD_S
    out = []
    headers = [
        (float(f["start_s"]) + offset_of(offset_slave, float(f["start_s"])), f.get("control") or "")
        for f in slave.doc.get("frames") or []
        if f.get("kind") == "control" and f.get("decoded")
    ]
    placed = [k for k in master.keyings if k.rf_start is not None and k.rf_end is not None]
    for k, after in itertools.zip_longest(placed, placed[1:]):
        assert k.rf_start is not None and k.rf_end is not None
        end = k.rf_end
        # an answer belongs to the master's last transmission before it: a slot left silent
        # was followed by a poll, and the poll's answer is not the silent slot's
        next_start = after.rf_start if after is not None else None
        before = next_start if next_start is not None else float("inf")
        reply = [
            s
            for s in slave.keyings
            if s.rf_start is not None and end < s.rf_start < min(end + 3.5, before)
        ]
        if not reply:
            continue
        header = next((h for t, h in headers if abs(t - k.rf_start) < 0.3), "")
        level = re.search(r"level (\d)", header)
        start = end + gap + LEVELS_S[int(level[1]) if level else 0]
        late = float(reply[0].rf_start or 0.0) - start
        out.append(
            {"master_key": k.key, "slot_start": start, "late": late, "margin": SLACK_S - late}
        )
    return out


LOOP_SUM_SPREAD_S = 0.03
"""How far a pair's loop sum may stray from the session's median and still place the offset:
the sum is the two radios' paths and does not move with the clocks; one 30 ms off was placed by
the wrong frame (a first frame the other station never recorded)."""
STEP_MIN_S = 0.008
"""The least change in the loops' offset read as a step: pairs scatter by a few milliseconds
(frame starts estimated to ~2 ms, keying jitter), and the clocks' drift moves it under a
millisecond between neighbours."""
STEP_AROUND = 3
"""Pairs either side of a candidate step whose medians are compared."""


@dataclass(frozen=True)
class LoopOffset:
    t: float
    """On a's clock: the second keying of the pair."""
    offset: float
    """b's recording clock to a's that makes the two loops equal."""
    loop_sum_s: float


def loop_offsets(a: Side, b: Side, offset: Offset) -> list[LoopOffset]:
    """At each change of hands between a keying of each station, the offset at which their loop
    latencies are equal, after :func:`place` with ``offset``: a's loop grows with the offset and
    b's shrinks by as much, so the pair fixes it whatever ``offset`` was."""
    events = sorted(
        [(k.key, "a", k) for k in a.keyings if k.loop_s is not None]
        + [(k.key + offset_of(offset, k.key), "b", k) for k in b.keyings if k.loop_s is not None],
        key=lambda e: e[0],
    )
    pairs = []
    for (_, w1, k1), (t2, w2, k2) in itertools.pairwise(events):
        if w1 == w2:
            continue
        ka, kb = (k1, k2) if w1 == "a" else (k2, k1)
        assert ka.loop_s is not None and kb.loop_s is not None
        # the offset this pair was placed with: a's keying's own, b's frames placed by it
        used = offset_of(offset, kb.key)
        pairs.append(LoopOffset(t2, used + (kb.loop_s - ka.loop_s) / 2, ka.loop_s + kb.loop_s))
    if not pairs:
        return []
    middle = statistics.median(p.loop_sum_s for p in pairs)
    return [p for p in pairs if abs(p.loop_sum_s - middle) <= LOOP_SUM_SPREAD_S]


def offset_steps(series: list[LoopOffset]) -> list[tuple[float, float, float]]:
    """Where the loops' offset changes by :data:`STEP_MIN_S` or more: (from, to, step) on a's
    clock. Pairs where the median of the next :data:`STEP_AROUND` differs from the median of
    the last by that much are one candidate stretch; the step is the largest single change
    between neighbours inside it, and its size the stretch's largest median difference."""
    n = STEP_AROUND
    diffs = {
        i: statistics.median(p.offset for p in series[i : i + n])
        - statistics.median(p.offset for p in series[i - n : i])
        for i in range(n, len(series) - n + 1)
    }
    out: list[tuple[float, float, float]] = []
    stretch: list[int] = []
    for i in [*sorted(diffs), None]:
        if i is not None and abs(diffs[i]) >= STEP_MIN_S and (not stretch or i == stretch[-1] + 1):
            stretch.append(i)
            continue
        if stretch:
            at = max(stretch, key=lambda j: abs(series[j].offset - series[j - 1].offset))
            size = max((diffs[j] for j in stretch), key=abs)
            out.append((series[at - 1].t, series[at].t, size))
            stretch = []
        if i is not None and abs(diffs[i]) >= STEP_MIN_S:
            stretch = [i]
    return out


def drift_ppm(series: list[LoopOffset], steps: list[tuple[float, float, float]]) -> float | None:
    """How fast b's recording runs against a's, parts per million, from the slope of the loops'
    offset with each step taken out."""
    if len(series) < 4:
        return None
    t = np.array([p.t for p in series])
    y = np.array([p.offset for p in series])
    for _, to, step in steps:
        y[t >= to] -= step
    slope = float(np.polyfit(t, y, 1)[0])
    return slope * 1e6


def offset_along(series: list[LoopOffset], base: float) -> Callable[[float], float]:
    """The offset at each moment of b's clock: the median of the three pairs nearest it, so a
    keying is placed with the offset in force at its time (``base`` takes b's clock to a's
    closely enough to find them)."""
    times = np.array([p.t for p in series])
    values = [p.offset for p in series]

    def at(t_b: float) -> float:
        nearest = np.argsort(np.abs(times - (t_b + base)))[:3]
        return float(statistics.median(values[i] for i in nearest))

    return at


def capture_skips(side: Side) -> list[tuple[float, str]]:
    """The station's own reports of audio its sound card's timestamps say it lost or repeated,
    on its recording clock (ADR-0071 §3)."""
    return [
        (float(e["t_s"]), str(e.get("detail", "")))
        for e in side.doc.get("events") or []
        if e.get("event") == "capture_skip"
    ]


def role(side: Side) -> str:
    return next(
        (str(e.get("detail")) for e in side.doc.get("events") or [] if e.get("event") == "role"),
        "",
    )


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("sidecars", nargs=2, type=Path)
    ap.add_argument("--band", default="2700:3000", help="Hz:Hz both stations hear the same")
    ap.add_argument("--window", type=float, default=90.0, help="seconds per offset window")
    ap.add_argument("--search", type=float, default=0.6, help="seconds either side of the stamps")
    ap.add_argument("--offset", type=float, default=None, help="use this offset (b to a), s")
    ap.add_argument(
        "--from-loops",
        action="store_true",
        help="place each keying with the offset the loop latencies imply at its time",
    )
    args = ap.parse_args()
    a, b = (load(p) for p in args.sidecars)
    for side in (a, b):
        side.keyings = keyings(side)
        mute_onsets(side)
    guess = started_offset(a, b)
    print(f"{b.name}'s recording clock to {a.name}'s: {guess:+.3f} s from the started stamps")
    lo, hi = (float(v) for v in args.band.split(":"))
    ea, la = envelope(a, (lo, hi))
    eb, lb = envelope(b, (lo, hi))
    # one fit over the whole session: the longest stretch without a step decides it
    lag, r, n = best_lag(ea, la, eb, lb, (0.0, len(ea) / RATE), guess, args.search)
    print(f"  whole session: lag {lag:+.3f} s  r {r:.3f}  ({n / RATE:.1f} s both listening)")
    # and window by window, to see a step: only windows with enough listening in both say much
    t = 0.0
    while t < len(ea) / RATE:
        wl, wr, wn = best_lag(ea, la, eb, lb, (t, t + args.window), lag, args.search)
        if wn >= MIN_WINDOW_S * RATE and wr >= MIN_WINDOW_R:
            step = "" if abs(wl - lag) < 0.005 else f"  {1000 * (wl - lag):+.0f} ms from the whole"
            print(
                f"  {a.name} {t:6.0f}-{t + args.window:<6.0f} lag {wl:+.3f} s  r {wr:.3f}  ({wn / RATE:.1f} s){step}"
            )
        t += args.window
    if args.offset is not None:
        offset: Offset = args.offset
    elif r >= MIN_WINDOW_R:
        offset = lag
    else:
        # nothing both heard: the stamps are a few tenths off, near enough to place the frames
        # by which the loops then fix it
        print(f"  no signal both heard (r {r:.3f}): the stamps' {guess:+.3f} s to start from")
        offset = guess
    place(a, b, offset)
    series = loop_offsets(a, b, offset)
    if series:
        steps = offset_steps(series)
        ppm = drift_ppm(series, steps)
        middle = statistics.median(p.offset for p in series)
        print(
            f"the loops' offset: {series[0].offset:+.4f} s at {series[0].t:.0f} s to "
            f"{series[-1].offset:+.4f} s at {series[-1].t:.0f} s, median {middle:+.4f} s "
            f"({len(series)} changes of hands; loop sum median "
            f"{1000 * statistics.median(p.loop_sum_s for p in series):.0f} ms)"
            + (f"; drift {ppm:+.0f} ppm ({b.name} against {a.name})" if ppm is not None else "")
        )
        for t0, t1, jump in steps:
            print(f"  step {1000 * jump:+.0f} ms between {t0:.1f} and {t1:.1f} s")
        along = offset_along(series, middle)
        for side in (a, b):
            for t, detail in capture_skips(side):
                at = t + (along(t) if side is b else 0.0)
                print(
                    f"  {side.name} capture_skip {detail} at {at:.1f} s ({side.name}'s {t:.1f} s)"
                )
        if args.from_loops:
            offset = along
            place(a, b, offset)
    if callable(offset):
        print(f"offset used: the loops', at each keying's time ({a.name} = {b.name} + offset)")
    else:
        print(f"offset used: {offset:+.3f} s (a signal both heard: {a.name} = {b.name} + offset)")
    for side in (a, b):
        hand = [k.handover_s for k in side.keyings]
        loops = [k.loop_s for k in side.keyings if k.loop_s is not None]
        mutes = [k.mute_s for k in side.keyings if k.mute_s is not None]
        print(
            f"\n{side.name}: {len(side.keyings)} keyings; handover delay median "
            f"{1000 * statistics.median(hand):.0f} ms ({1000 * min(hand):.0f}-{1000 * max(hand):.0f}); "
            + (f"mute onset median {1000 * statistics.median(mutes):.0f} ms; " if mutes else "")
            + (
                f"loop latency median {1000 * statistics.median(loops):.0f} ms "
                f"({1000 * min(loops):.0f}-{1000 * max(loops):.0f})"
                if loops
                else "no frame of it recorded by the other"
            )
        )
        for k in side.keyings:
            loop = f"{1000 * k.loop_s:6.0f}" if k.loop_s is not None else "     -"
            print(
                f"  key {k.key:8.3f} release {k.release:8.3f}  frames {k.frames_s:6.2f} s  "
                f"handover {1000 * k.handover_s:4.0f} ms  loop {loop} ms"
            )
    print("\nthe air changing hands (gap between the two stations' radiation):")
    changes = gaps(a, b)
    for x, y, g in changes:
        flag = "  OVERLAP" if g < 0 else ""
        print(f"  {x.who} -> {y.who} at {float(y.rf_start or 0):8.3f}: {g:6.3f} s{flag}")
    if changes:
        print(
            f"  smallest {min(g for *_, g in changes):.3f} s; overlaps {sum(g < 0 for *_, g in changes)}"
        )
    master, slave = (a, b) if role(a) == "master" or role(b) == "slave" else (b, a)
    rows = answers(master, slave, offset if slave is b else 0.0)
    for row in rows:
        print(
            f"  master key {row['master_key']:8.2f}: slot {row['slot_start']:8.2f}, late "
            f"{1000 * row['late']:4.0f} ms, margin {1000 * row['margin']:4.0f} ms"
        )
    if rows:
        late = [r["late"] for r in rows]
        print(
            f"\n{slave.name}'s answers against their slots: {len(rows)}, late "
            f"{1000 * min(late):.0f}-{1000 * max(late):.0f} ms (median {1000 * statistics.median(late):.0f}), "
            f"least margin to not_after {1000 * min(r['margin'] for r in rows):.0f} ms"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
