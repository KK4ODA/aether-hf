"""The hidden-frame bench: whether a weak candidate's span may hide a stronger frame.

The streaming receiver drops an OFDM candidate that starts inside a frame it already knows, so a
false detection a few tenths of a second before a real frame takes the real frame's place, and
the real one is never looked at. On the air it hid a header and two answers (reports 16 §3 and
17 §6). The rule under test (``StreamingReceiver(weak_spans_hide=False)``): a known frame
acquired under ``TRUSTED_CONFIDENCE`` that has not decoded does not hide a candidate at or over
it. The plan and its criteria are ``docs/research/hidden-frames/00-plan.md``.

    python -I tools/bench_hidden.py field RECORDING.wav [--bandwidth 500] [--block-ms 20]
    python -I tools/bench_hidden.py synth --out m2.csv [--trials 20] [--jobs 4]
    python -I tools/bench_hidden.py traffic --out m3.csv [--seconds 120] [--jobs 4]
    python -I tools/bench_hidden.py noise --out m4.csv [--chunks 6] [--seconds 600]

``field`` (M1) replays a recording through the model's receiver under both rules and lists the
frames each found, and where they differ. ``synth`` (M2) puts a real frame inside a false
candidate's span and asks which rule decodes it; ``traffic`` (M3, M5) runs the link's own
transmissions through the channel and counts the frames each rule loses, the false frames the
rule under test adds and the decode attempts; ``noise`` (M4) counts candidates on noise alone.
Each appends its rows to the CSV as they finish (losses and additions to ``<out>.notes.txt``),
so a stopped run keeps what it had; ``synth-summary`` and ``traffic-summary`` tabulate a CSV.
``--rule r2`` puts the offline rule (the stronger candidate wins) under test in place of R1.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
import wave
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

import numpy as np
from numpy.typing import NDArray

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.phy.passband import AudioToBaseband
from aether_model.phy.pipeline import DecodedFrame
from aether_model.phy.stream import TRUSTED_CONFIDENCE, StreamingReceiver
from aether_model.waveform import NARROW_500, WIDE_2300, WaveformParams

AUDIO_RATE = 48_000

RULES = ("r0", "r1", "r2")
"""The rules the bench compares (``00-plan.md`` §2): R0 as built, R1 under test, R2 the offline
rule, measured because R1 missed K2."""


def receiver(params: WaveformParams, rule: str) -> StreamingReceiver:
    if rule not in RULES:
        raise ValueError(f"no rule {rule!r}")
    return StreamingReceiver(params, weak_spans_hide=rule == "r0", strongest_wins=rule == "r2")


@dataclass(frozen=True)
class Found:
    """A frame the receiver handed out: where (seconds of the recording), what, and whether it
    decoded."""

    start_s: float
    tone: bool
    control: bool
    mode: int
    decoded: bool
    detect_confidence: float
    snr_db: float

    def label(self) -> str:
        kind = "tone" if self.tone else ("control" if self.control else f"data r{self.mode}")
        state = "decoded" if self.decoded else "failed"
        return (
            f"{self.start_s:9.3f} s  {kind:12} {state:7}  acq {self.detect_confidence:5.2f}  "
            f"{self.snr_db:+6.1f} dB"
        )


def read_audio(path: Path) -> NDArray[np.float64]:
    with wave.open(str(path), "rb") as w:
        if w.getframerate() != AUDIO_RATE:
            raise SystemExit(f"{path}: {w.getframerate()} Hz, not {AUDIO_RATE}")
        raw = np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16)
        return raw.reshape(-1, w.getnchannels())[:, 0].astype(np.float64) / 32768.0


def bandwidth_of(path: Path) -> int | None:
    sidecar = path.with_suffix(".json")
    if not sidecar.exists():
        return None
    session = json.loads(sidecar.read_text(encoding="utf-8")).get("session") or {}
    hz = session.get("bandwidth_hz")
    return int(hz) if hz else None


def found_of(decoded: DecodedFrame, params: WaveformParams, delay_s: float) -> Found:
    fs = params.fs_baseband
    if decoded.tone_sync is not None:
        ts = decoded.tone_sync
        tone = decoded.tone
        return Found(
            ts.start / fs - delay_s,
            True,
            ts.kind.control,
            0,
            decoded.ok,
            float(getattr(ts, "statistic", 0.0)),
            float(tone.snr_db) if tone is not None else float("nan"),
        )
    frame = decoded.frame
    assert frame is not None
    from aether_model.frame.modes import air_interface
    from aether_model.phy.sync import FrameType

    air = air_interface(params)
    return Found(
        frame.sync.start / fs - delay_s,
        False,
        frame.sync.header.frame_type is FrameType.CONTROL,
        int(frame.mode),
        decoded.ok,
        frame.sync.detect_confidence(air),
        float(frame.snr_3k_db),
    )


def replay(
    audio: NDArray[np.float64], params: WaveformParams, rule: str, block_ms: float
) -> list[Found]:
    """The recording through the model's streaming receiver, block by block as the daemon
    feeds it."""
    front = AudioToBaseband(params)
    rx = receiver(params, rule)
    # absolute indices count the samples the blanker has released at their input positions:
    # the stream lags the input by the band filter alone
    delay_s = rx.delay_samples / params.fs_baseband
    block = int(AUDIO_RATE * block_ms / 1000)
    out: list[Found] = []
    for i in range(0, len(audio), block):
        for decoded in rx.feed(front.process(audio[i : i + block])):
            out.append(found_of(decoded, params, delay_s))
    return out


def field(args: argparse.Namespace) -> None:
    path = Path(args.wav)
    hz = args.bandwidth or bandwidth_of(path) or 2300
    params = NARROW_500 if hz == 500 else WIDE_2300
    audio = read_audio(path)
    runs: dict[str, list[Found]] = {}
    for name, rule in (("as built", "r0"), ("under test", args.rule)):
        t0 = time.monotonic()
        runs[name] = replay(audio, params, rule, args.block_ms)
        took = time.monotonic() - t0
        ok = sum(f.decoded for f in runs[name])
        print(
            f"{path.name} at {hz} Hz, {name}: {len(runs[name])} frames, {ok} decoded "
            f"({took:.0f} s for {len(audio) / AUDIO_RATE:.0f} s of audio)"
        )
    built = {round(f.start_s, 2): f for f in runs["as built"]}
    test = {round(f.start_s, 2): f for f in runs["under test"]}
    gained = [f for k, f in test.items() if k not in built]
    lost = [f for k, f in built.items() if k not in test]
    changed = [
        (built[k], test[k])
        for k in built.keys() & test.keys()
        if built[k].decoded != test[k].decoded
    ]
    print(f"  only under test: {len(gained)}")
    for f in sorted(gained, key=lambda f: f.start_s):
        print("    +", f.label())
    print(f"  only as built: {len(lost)}")
    for f in sorted(lost, key=lambda f: f.start_s):
        print("    -", f.label())
    for a, b in changed:
        print("    ~", a.label(), "->", b.label())
    if args.list:
        for f in runs["under test"]:
            print("   ", f.label())


# ── M2: a false candidate, then a real frame inside its span ─────────────────

BLOCK = 160
"""20 ms of baseband at 8 kHz: the daemon's block."""

REAL_RUNGS = {2300: (8, 12), 500: (7, 11)}
"""Data rungs the real frame is drawn at: a low and a middle OFDM rung of each air."""

SNR_ABOVE_DB = (1.0, 3.0, 6.0, 10.0, 15.0)
"""The real frame's SNR above its rung's AWGN threshold."""


def params_of(hz: int) -> WaveformParams:
    return NARROW_500 if hz == 500 else WIDE_2300


def unit(x: NDArray[np.complex128]) -> NDArray[np.complex128]:
    return x / np.sqrt(np.mean(np.abs(x) ** 2))


def noise(rng: np.random.Generator, n: int, fs: float) -> NDArray[np.complex128]:
    """Noise against which a unit-power signal is at 0 dB in 3 kHz."""
    sigma = np.sqrt(fs / 3000.0 / 2.0)
    return sigma * (rng.standard_normal(n) + 1j * rng.standard_normal(n))


def cut_preamble(modem: object, control: bool) -> NDArray[np.complex128]:
    """The two preamble symbols of a frame with nothing after them: acquisition takes it as a
    frame of that type, and nothing in it decodes — what a noise trigger is to the receiver."""
    from aether_model.phy.pipeline import Modem

    assert isinstance(modem, Modem)
    if control:
        frame = modem.control_burst(b"\x01")
    else:
        frame = modem.rung_burst(bytes(modem.rung_payload_bytes(8)), 8)
    return unit(frame)[: 2 * modem.p.symbol_samples]


def stream_out(
    params: WaveformParams, x: NDArray[np.complex128], rule: str
) -> tuple[list[DecodedFrame], int, int]:
    """The frames a stream hands out over ``x``, its decode attempts, and its index lag."""
    rx = receiver(params, rule)
    out: list[DecodedFrame] = []
    for i in range(0, len(x), BLOCK):
        out += rx.feed(x[i : i + BLOCK])
    return out, rx.frames_decoded, rx.delay_samples


def ofdm_found_at(out: list[DecodedFrame], at: int, tolerance: int) -> DecodedFrame | None:
    for d in out:
        if (
            d.frame is not None
            and d.tone_sync is None
            and abs(d.frame.sync.start - at) <= tolerance
        ):
            return d
    return None


def calibrate(hz: int, control: bool, seed: int = 7) -> tuple[float, float]:
    """The SNR range over which acquisition reads a cut preamble at 1.0–1.25 (the medians)."""
    from aether_model.frame.modes import air_interface
    from aether_model.phy.pipeline import Modem

    params = params_of(hz)
    modem = Modem(params, blank_impulses=False)
    air = air_interface(params)
    cut = cut_preamble(modem, control)
    rng = np.random.default_rng(seed)
    fs = params.fs_baseband
    medians: list[tuple[float, float]] = []
    for snr in np.arange(-16.0, 8.01, 0.5):
        confs = []
        for _ in range(12):
            x = noise(rng, 6000, fs)
            x[2000 : 2000 + len(cut)] += cut * 10 ** (snr / 20)
            y = modem.detector.condition(x)
            found = [s for s in modem.detector.detect(y, max_frames=4) if abs(s.start - 2000) < 160]
            confs.append(found[0].detect_confidence(air) if found else 0.0)
        medians.append((float(snr), float(np.median(confs))))
    inside = [s for s, c in medians if 1.0 <= c <= 1.25]
    if not inside:
        raise SystemExit(f"{hz} Hz: no SNR puts a cut preamble at 1.0-1.25: {medians}")
    return min(inside), max(inside)


@dataclass(frozen=True)
class Trial:
    hz: int
    false_control: bool
    real: str  # "control" or "r<rung>"
    snr_above_db: float
    false_snr: tuple[float, float]
    seed: int
    rule: str = "r1"


SYNTH_FIELDS = [
    "hz",
    "false_type",
    "real",
    "rung",
    "snr_db",
    "delay_s",
    "false_snr_db",
    "false_conf",
    "real_conf",
    "ref",
    "r0",
    "test",
    "r0_attempts",
    "test_attempts",
    "rule",
]
"""``test`` is the rule under test, named in ``rule`` (the first M2 run, R1 only, called its
columns ``r1``)."""


def run_trial(t: Trial) -> dict[str, object]:
    from aether_model.frame.modes import air_interface
    from aether_model.link.rate import AWGN_THRESHOLD_DB, NARROW_AWGN_THRESHOLD_DB
    from aether_model.phy.pipeline import Modem
    from aether_model.phy.preamble import FrameType
    from aether_model.phy.rx import layout_for

    params = params_of(t.hz)
    fs = params.fs_baseband
    modem = Modem(params)
    air = air_interface(params)
    thresholds = NARROW_AWGN_THRESHOLD_DB if t.hz == 500 else AWGN_THRESHOLD_DB
    rng = np.random.default_rng(t.seed)
    if t.real == "control":
        rung = next(i for i, r in enumerate(air.ladder) if r.mode is air.control_mode)
        payload = rng.integers(1, 256, modem.payload_bytes(None), dtype=np.uint8).tobytes()
        real = unit(modem.control_burst(payload))
    else:
        rung = int(t.real[1:])
        payload = rng.integers(0, 256, modem.rung_payload_bytes(rung), dtype=np.uint8).tobytes()
        real = unit(modem.rung_burst(payload, rung))
    snr = thresholds[rung] + t.snr_above_db
    cut = cut_preamble(modem, t.false_control)
    false_span = layout_for(
        FrameType.CONTROL if t.false_control else FrameType.DATA, params
    ).samples
    delay = int(rng.uniform(0.1 * fs, false_span))
    false_snr = float(rng.uniform(*t.false_snr))
    lead = int(rng.uniform(0.5, 1.0) * fs)
    n = lead + delay + len(real) + int(1.5 * fs)
    base = noise(rng, n, fs)
    with_real = base.copy()
    with_real[lead + delay : lead + delay + len(real)] += real * 10 ** (snr / 20)
    with_false = with_real.copy()
    with_false[lead : lead + len(cut)] += cut * 10 ** (false_snr / 20)

    ref, _, lag = stream_out(params, with_real, "r0")
    r0, r0_n, _ = stream_out(params, with_false, "r0")
    r1, r1_n, _ = stream_out(params, with_false, t.rule)
    sym = params.symbol_samples

    def has(out: list[DecodedFrame]) -> bool:
        return any(d.ok and d.payload == payload for d in out)

    real_ref = ofdm_found_at(ref, lead + delay + lag, sym)
    false_r0 = ofdm_found_at(r0, lead + lag, sym)
    return {
        "hz": t.hz,
        "false_type": "control" if t.false_control else "data",
        "real": t.real,
        "rung": rung,
        "snr_db": round(snr, 2),
        "delay_s": round(delay / fs, 3),
        "false_snr_db": round(false_snr, 2),
        "false_conf": round(false_r0.frame.sync.detect_confidence(air), 3)
        if false_r0 is not None and false_r0.frame is not None
        else "",
        "real_conf": round(real_ref.frame.sync.detect_confidence(air), 3)
        if real_ref is not None and real_ref.frame is not None
        else "",
        "ref": int(has(ref)),
        "r0": int(has(r0)),
        "test": int(has(r1)),
        "r0_attempts": r0_n,
        "test_attempts": r1_n,
        "rule": t.rule,
    }


def synth(args: argparse.Namespace) -> None:
    import csv
    from concurrent.futures import ProcessPoolExecutor, as_completed

    out_path = Path(args.out)
    trials: list[Trial] = []
    seed = args.seed
    for hz in args.airs:
        for false_control in (True, False):
            window = calibrate(hz, false_control)
            print(
                f"{hz} Hz, cut {'control' if false_control else 'data'} preamble: "
                f"1.0-1.25 at {window[0]:+.1f} … {window[1]:+.1f} dB",
                flush=True,
            )
            for real in ("control", *(f"r{r}" for r in REAL_RUNGS[hz])):
                for above in SNR_ABOVE_DB:
                    for _ in range(args.trials):
                        seed += 1
                        trials.append(
                            Trial(hz, false_control, real, above, window, seed, args.rule)
                        )
    new = not out_path.exists()
    with out_path.open("a", newline="", encoding="utf-8") as fh:
        w = csv.DictWriter(fh, SYNTH_FIELDS)
        if new:
            w.writeheader()
        with ProcessPoolExecutor(args.jobs) as pool:
            futures = [pool.submit(run_trial, t) for t in trials]
            for k, fut in enumerate(as_completed(futures), 1):
                w.writerow(fut.result())
                fh.flush()
                if k % 50 == 0:
                    print(f"  {k}/{len(trials)}", flush=True)
    summarise_synth(out_path)


def summarise_synth(path: Path) -> None:
    """Per cell: the trials that count (the reference decodes the real frame, the as-built
    stream handed out the false candidate), what each rule decoded, and the same for the case
    the rule is for — the false candidate under 1.3 and the real frame at or over it."""
    import csv
    from collections import defaultdict

    rows = list(csv.DictReader(path.open(encoding="utf-8")))
    cells: dict[tuple[str, str, str], list[dict[str, str]]] = defaultdict(list)
    for r in rows:
        r.setdefault("test", r.get("r1", ""))
        cells[(r["hz"], r["false_type"], r["real"])].append(r)

    def share(rs: list[dict[str, str]], key: str) -> str:
        k = sum(r[key] == "1" for r in rs)
        return f"{k:4}/{len(rs):<4} {100 * k / max(len(rs), 1):5.1f} %"

    def weak_false(r: dict[str, str]) -> bool:
        return float(r["false_conf"]) < TRUSTED_CONFIDENCE

    def strong_real(r: dict[str, str]) -> bool:
        return r["real_conf"] != "" and float(r["real_conf"]) >= TRUSTED_CONFIDENCE

    print(
        "air  false    real     | counted: as built        under test       "
        "| false < 1.3, real >= 1.3: as built   under test"
    )
    totals: dict[str, list[dict[str, str]]] = defaultdict(list)
    for key in sorted(cells):
        rs = [r for r in cells[key] if r["ref"] == "1" and r["false_conf"] != ""]
        aimed = [r for r in rs if weak_false(r) and strong_real(r)]
        totals[key[0]] += rs
        totals[key[0] + " aimed"] += aimed
        print(
            f"{key[0]:>4} {key[1]:8} {key[2]:8} | {share(rs, 'r0')}  {share(rs, 'test')} "
            f"| {share(aimed, 'r0')}  {share(aimed, 'test')}"
        )
    for hz in sorted(k for k in totals if " " not in k):
        print(
            f"{hz:>4} all: counted as built {share(totals[hz], 'r0')}, under test "
            f"{share(totals[hz], 'test')}; aimed as built {share(totals[hz + ' aimed'], 'r0')}, "
            f"under test {share(totals[hz + ' aimed'], 'test')}"
        )


# ── M3: the link's own traffic through the channel ───────────────────────────

CHANNELS = ("awgn", "good", "moderate", "poor", "flutter")
TRAFFIC_SNRS = (-8.0, -4.0, 0.0, 4.0, 8.0, 12.0, 16.0, 20.0)

TRAFFIC_FIELDS = [
    "hz", "channel", "snr_db", "seconds", "real_ofdm", "real_tone", "r0_decoded",
    "r1_decoded", "losses", "gains", "r0_unmatched", "r1_unmatched", "false_added",
    "false_added_decoded", "r0_attempts", "r1_attempts", "rule",
]  # fmt: skip
"""The ``r1`` columns are the rule under test, named in ``rule``."""


Job = tuple[int, str, float, float, int, str]
"""(air, channel or "noise", SNR, seconds, seed, rule under test): one condition of M3 or chunk
of M4."""


@dataclass(frozen=True)
class Sent:
    start: int  # input sample index
    tone: bool
    payload: bytes


def traffic(
    hz: int, snr: float, seconds: float, rng: np.random.Generator
) -> tuple[NDArray[np.complex128], list[Sent]]:
    """Transmissions as a link sends them, separated by turnarounds: a header and one to six
    data frames back to back, a lone answer, or a tone frame, at rungs about what the SNR
    carries (some above it, so some fail)."""
    from aether_model.frame.modes import air_interface
    from aether_model.link.rate import AWGN_THRESHOLD_DB, NARROW_AWGN_THRESHOLD_DB
    from aether_model.phy.pipeline import Modem

    params = params_of(hz)
    fs = params.fs_baseband
    modem = Modem(params)
    air = air_interface(params)
    thresholds = NARROW_AWGN_THRESHOLD_DB if hz == 500 else AWGN_THRESHOLD_DB
    ofdm = [i for i, r in enumerate(air.ladder) if r.mode is not None]
    tones = [i for i, r in enumerate(air.ladder) if r.tone is not None]
    fitting = [r for r in ofdm if snr - 10 <= thresholds[r] <= snr + 2] or [ofdm[0]]
    parts: list[NDArray[np.complex128]] = []
    sent: list[Sent] = []
    at = 0

    def put(x: NDArray[np.complex128], tone: bool, payload: bytes) -> None:
        nonlocal at
        parts.append(x)
        sent.append(Sent(at, tone, payload))
        at += len(x)

    def gap(lo: float, hi: float) -> None:
        nonlocal at
        n = int(rng.uniform(lo, hi) * fs)
        parts.append(np.zeros(n, dtype=np.complex128))
        at += n

    def ctl() -> bytes:
        return rng.integers(1, 256, modem.payload_bytes(None), dtype=np.uint8).tobytes()

    gap(0.5, 1.0)
    while at < seconds * fs:
        kind = rng.choice(["burst", "answer", "tone"], p=[0.55, 0.30, 0.15])
        if kind == "burst":
            p = ctl()
            put(modem.control_burst(p), False, p)
            rung = int(rng.choice(fitting))
            for _ in range(int(rng.integers(1, 7))):
                p = rng.integers(0, 256, modem.rung_payload_bytes(rung), dtype=np.uint8).tobytes()
                put(modem.rung_burst(p, rung, follows=0), False, p)
        elif kind == "answer":
            p = ctl()
            put(modem.control_burst(p), False, p)
        elif rng.random() < 0.5:
            p = ctl()
            put(modem.control_burst(p, floor=True), True, p)
        else:
            rung = int(rng.choice(tones))
            p = rng.integers(0, 256, modem.rung_payload_bytes(rung), dtype=np.uint8).tobytes()
            put(modem.rung_burst(p, rung), True, p)
        gap(0.15, 1.0)
    gap(1.5, 1.6)
    return np.concatenate(parts), sent


def run_traffic(job: Job) -> tuple[dict[str, object], list[str]]:
    from aether_model.channel import make_channel
    from aether_model.frame.modes import air_interface

    hz, channel, snr, seconds, seed, rule = job
    params = params_of(hz)
    rng = np.random.default_rng(seed)
    x, sent = traffic(hz, snr, seconds, rng)
    y = make_channel(
        channel, snr_db=snr, fs=params.fs_baseband, seed=seed, signal_power=1.0
    ).process(x)
    r0, r0_n, lag = stream_out(params, y, "r0")
    r1, r1_n, _ = stream_out(params, y, rule)
    sym = params.symbol_samples
    starts = np.array([s.start + lag for s in sent])

    def ofdm_starts(out: list[DecodedFrame]) -> list[DecodedFrame]:
        return [d for d in out if d.frame is not None and d.tone_sync is None]

    def decoded(out: list[DecodedFrame]) -> set[int]:
        got = {d.payload for d in out if d.ok}
        return {i for i, s in enumerate(sent) if not s.tone and s.payload in got}

    d0, d1 = decoded(r0), decoded(r1)
    notes: list[str] = []

    def unmatched(out: list[DecodedFrame]) -> list[DecodedFrame]:
        return [
            d
            for d in ofdm_starts(out)
            if len(starts) == 0 or np.min(np.abs(starts - d.frame.sync.start)) > sym  # type: ignore[union-attr]
        ]

    u0, u1 = unmatched(r0), unmatched(r1)
    u0_at = np.array([d.frame.sync.start for d in u0])  # type: ignore[union-attr]
    added = [
        d
        for d in u1
        if len(u0_at) == 0 or np.min(np.abs(u0_at - d.frame.sync.start)) > sym  # type: ignore[union-attr]
    ]
    fs = params.fs_baseband
    for i in sorted(d0 - d1):
        notes.append(f"loss {hz} {channel} {snr:+.0f} dB seed {seed} at {sent[i].start / fs:.3f} s")
    for d in added:
        assert d.frame is not None
        notes.append(
            f"added {hz} {channel} {snr:+.0f} dB seed {seed} at "
            f"{(d.frame.sync.start - lag) / fs:.3f} s ok={d.ok} "
            f"conf={d.frame.sync.detect_confidence(air_interface(params)):.2f}"
        )
    row = {
        "hz": hz,
        "channel": channel,
        "snr_db": snr,
        "seconds": round(len(x) / fs, 1),
        "real_ofdm": sum(not s.tone for s in sent),
        "real_tone": sum(s.tone for s in sent),
        "r0_decoded": len(d0),
        "r1_decoded": len(d1),
        "losses": len(d0 - d1),
        "gains": len(d1 - d0),
        "r0_unmatched": len(u0),
        "r1_unmatched": len(u1),
        "false_added": len(added),
        "false_added_decoded": sum(d.ok for d in added),
        "r0_attempts": r0_n,
        "r1_attempts": r1_n,
        "rule": rule,
    }
    return row, notes


def run_jobs(
    jobs: list[Job],
    fn: Callable[[Job], tuple[dict[str, object], list[str]]],
    fields: list[str],
    out_path: Path,
    workers: int,
) -> None:
    import csv
    from concurrent.futures import ProcessPoolExecutor, as_completed

    new = not out_path.exists()
    notes_path = out_path.with_suffix(".notes.txt")
    with (
        out_path.open("a", newline="", encoding="utf-8") as fh,
        notes_path.open("a", encoding="utf-8") as nh,
    ):
        w = csv.DictWriter(fh, fields)
        if new:
            w.writeheader()
        with ProcessPoolExecutor(workers) as pool:
            futures = [pool.submit(fn, j) for j in jobs]
            for k, fut in enumerate(as_completed(futures), 1):
                row, notes = fut.result()
                w.writerow(row)
                fh.flush()
                for line in notes:
                    nh.write(line + "\n")
                nh.flush()
                print(f"  {k}/{len(jobs)}  {row}", flush=True)


def traffic_cmd(args: argparse.Namespace) -> None:
    jobs = []
    seed = args.seed
    for hz in args.airs:
        for channel in args.channels:
            for snr in args.snrs:
                seed += 1
                jobs.append((hz, channel, float(snr), args.seconds, seed, args.rule))
    run_jobs(jobs, run_traffic, TRAFFIC_FIELDS, Path(args.out), args.jobs)
    summarise_traffic(Path(args.out))


def summarise_traffic(path: Path) -> None:
    import csv

    rows = list(csv.DictReader(path.open(encoding="utf-8")))
    for hz in sorted({r["hz"] for r in rows}):
        rs = [r for r in rows if r["hz"] == hz]

        def total(key: str, rs: list[dict[str, str]] = rs) -> int:
            return sum(int(r[key]) for r in rs)

        real = total("real_ofdm")
        print(
            f"{hz} Hz: {len(rs)} conditions, {sum(float(r['seconds']) for r in rs) / 60:.0f} min, "
            f"{real} OFDM frames ({total('r0_decoded')} decoded as built, "
            f"{total('r1_decoded')} under test)"
        )
        print(
            f"  losses {total('losses')}, gains {total('gains')}, false frames added "
            f"{total('false_added')} ({1000 * total('false_added') / max(real, 1):.2f} per 1000), "
            f"decoded {total('false_added_decoded')}; unmatched candidates "
            f"{total('r0_unmatched')} -> {total('r1_unmatched')}; decode attempts "
            f"{total('r0_attempts')} -> {total('r1_attempts')} "
            f"({100 * (total('r1_attempts') / max(total('r0_attempts'), 1) - 1):+.1f} %)"
        )


# ── M4: noise alone ──────────────────────────────────────────────────────────

NOISE_FIELDS = ["hz", "seed", "seconds", "r0_candidates", "r1_candidates", "r1_decoded", "rule"]


def run_noise(job: Job) -> tuple[dict[str, object], list[str]]:
    hz, _, _, seconds, seed, rule = job
    params = params_of(hz)
    fs = params.fs_baseband
    x = noise(np.random.default_rng(seed), int(seconds * fs), fs)
    r0, _, _ = stream_out(params, x, "r0")
    r1, _, lag = stream_out(params, x, rule)

    def ofdm(out: list[DecodedFrame]) -> list[DecodedFrame]:
        return [d for d in out if d.frame is not None and d.tone_sync is None]

    o0, o1 = ofdm(r0), ofdm(r1)
    at0 = {d.frame.sync.start for d in o0}  # type: ignore[union-attr]
    notes = [
        f"noise {hz} seed {seed} at {(d.frame.sync.start - lag) / fs:.3f} s under test only"
        for d in o1
        if d.frame is not None and d.frame.sync.start not in at0
    ]
    row: dict[str, object] = {
        "hz": hz,
        "seed": seed,
        "seconds": seconds,
        "r0_candidates": len(o0),
        "r1_candidates": len(o1),
        "r1_decoded": sum(d.ok for d in o1),
        "rule": rule,
    }
    return row, notes


def noise_cmd(args: argparse.Namespace) -> None:
    jobs = []
    seed = args.seed
    for hz in args.airs:
        for _ in range(args.chunks):
            seed += 1
            jobs.append((hz, "noise", 0.0, args.seconds, seed, args.rule))
    run_jobs(jobs, run_noise, NOISE_FIELDS, Path(args.out), args.jobs)


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="what", required=True)
    f = sub.add_parser("field", help="a recording through both rules")
    f.add_argument("wav")
    f.add_argument("--bandwidth", type=int, choices=(500, 2300))
    f.add_argument("--block-ms", type=float, default=20.0)
    f.add_argument("--list", action="store_true", help="list every frame the rule under test found")
    f.add_argument("--rule", choices=("r1", "r2"), default="r1", help="the rule under test")
    f.set_defaults(run=field)
    s = sub.add_parser("synth", help="M2: a false candidate, then a real frame inside its span")
    s.add_argument("--out", required=True, help="CSV, appended to as trials finish")
    s.add_argument("--airs", type=int, nargs="+", default=[2300, 500])
    s.add_argument("--trials", type=int, default=20, help="per cell")
    s.add_argument("--seed", type=int, default=1000)
    s.add_argument("--jobs", type=int, default=4)
    s.add_argument("--rule", choices=("r1", "r2"), default="r1", help="the rule under test")
    s.set_defaults(run=synth)
    sm = sub.add_parser("synth-summary", help="the M2 table from a CSV")
    sm.add_argument("csv")
    sm.set_defaults(run=lambda a: summarise_synth(Path(a.csv)))
    t = sub.add_parser("traffic", help="M3/M5: the link's traffic through the channel")
    t.add_argument("--out", required=True, help="CSV, appended to as conditions finish")
    t.add_argument("--airs", type=int, nargs="+", default=[2300, 500])
    t.add_argument("--channels", nargs="+", default=list(CHANNELS))
    t.add_argument("--snrs", type=float, nargs="+", default=list(TRAFFIC_SNRS))
    t.add_argument("--seconds", type=float, default=120.0, help="of air per condition")
    t.add_argument("--seed", type=int, default=5000)
    t.add_argument("--jobs", type=int, default=4)
    t.add_argument("--rule", choices=("r1", "r2"), default="r1", help="the rule under test")
    t.set_defaults(run=traffic_cmd)
    ts = sub.add_parser("traffic-summary", help="the M3 table from a CSV")
    ts.add_argument("csv")
    ts.set_defaults(run=lambda a: summarise_traffic(Path(a.csv)))
    n = sub.add_parser("noise", help="M4: noise alone through both rules")
    n.add_argument("--out", required=True)
    n.add_argument("--airs", type=int, nargs="+", default=[2300, 500])
    n.add_argument("--chunks", type=int, default=6, help="per air")
    n.add_argument("--seconds", type=float, default=600.0, help="per chunk")
    n.add_argument("--seed", type=int, default=9000)
    n.add_argument("--jobs", type=int, default=4)
    n.add_argument("--rule", choices=("r1", "r2"), default="r1", help="the rule under test")
    n.set_defaults(run=noise_cmd)
    args = ap.parse_args(argv)
    args.run(args)


if __name__ == "__main__":
    main()
