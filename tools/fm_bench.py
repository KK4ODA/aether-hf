"""Step 10.2b's recordings of FM radios, and the same measurements on the model (ADR-0115).

    python tools/fm_bench.py make --port voice --out bench/fm/seq
    python tools/fm_bench.py play bench/fm/seq/voice.json --out rec.wav
                                  --play "USB Audio CODEC" --record "SignaLink" [--key rts:COM9]
    python tools/fm_bench.py analyse bench/fm/seq/voice.json rec.wav [--out result.json]
    python tools/fm_bench.py model bench/fm/seq/voice.json --preset wide_voice --cnr 18
    python tools/fm_bench.py devices

``make`` writes the test sequence, a WAV and a manifest saying what is where: a stretch with no
carrier, then eight transmissions — the first holds a carrier with no audio (the radio's floor)
and a wideband probe (the band's response); the next six step a single carrier of 16-QAM, the
data's own shape and peaks, through 24 levels 1.5 dB apart (SNDR against drive); the last is a
marker alone. Every transmission opens with a chirp marker, so a recording can be aligned and the
two sound cards' clocks compared from the first marker to the last.

``analyse`` reads a recording of the sequence through a radio pair with ``aether_model.fm.measure``
— the one measure Gate 2 uses — and reports the clock offset, SNDR against drive and its peak,
the band's response, the floor, and the times from key and release to the receiver's audio
changing. ``model`` plays the same sequence through the FM channel model and analyses it the
same way, so a radio and the model are compared through identical code.

``play`` plays the sequence on one sound card and records another at the same time, keying the
radio by VOX (the default: the interface keys on the audio), or by a serial line's RTS or DTR
(``--key rts:COM9``, which needs pyserial: ``uv run --with pyserial``). Playback and recording
need the ``audio`` extra (``uv sync --extra audio``). ``docs/research/aether-fm/02-recording-plan.md``
is the procedure.
"""

from __future__ import annotations

import argparse
import json
import math
import sys
import threading
import time
from dataclasses import asdict, dataclass, field, replace
from pathlib import Path
from typing import Any, Literal

import numpy as np
from numpy.typing import NDArray
from scipy import signal
from scipy.io import wavfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.fm import channel as ch
from aether_model.fm import measure as me
from aether_model.fm import signals as sg

FloatArray = NDArray[np.float64]
FS = int(ch.FS)
FORMAT = "aether-fm-bench/1"

Port = Literal["voice", "flat"]

PROFILES: dict[str, dict[str, Any]] = {
    # the proposal's §D.4 shapes: the signal's band, and the probe's (wider, to see the edges)
    "voice": {"rate": 2000.0, "centre": 1650.0, "band": (450.0, 2850.0), "probe": (150.0, 3500.0)},
    "flat": {"rate": 4000.0, "centre": 2700.0, "band": (300.0, 5100.0), "probe": (40.0, 6500.0)},
}

LEVELS_DBFS = [-45.0 + 1.5 * k for k in range(24)]
"""The drive steps' RMS in dBFS: −45 to −10.5. The single carrier's highest peak in a step sits
up to about 10 dB over its RMS, so the top step stays under full scale."""
STEP_S = 3.0
STEP_SKIP_S = 0.5
"""Left out at the start of each step: a compressor's release, the last step's filter tails."""
KEY_UP_S = 0.75
"""Silence after the key before the first audio: radios take 50–300 ms to put out a carrier."""
MARKER_S = 0.25
MARKER_DBFS = -20.0
GAP_S = 3.0
PROBE_DBFS = -30.0


@dataclass
class Segment:
    start_s: float
    end_s: float


@dataclass
class Step(Segment):
    level_dbfs: float = 0.0
    seed: int = 0


@dataclass
class Transmission:
    key_s: float
    unkey_s: float


@dataclass
class Manifest:
    format: str
    port: Port
    seed: int
    fs: int
    band: tuple[float, float]
    probe_band: tuple[float, float]
    length_s: float
    transmissions: list[Transmission] = field(default_factory=list)
    markers: list[float] = field(default_factory=list)
    steps: list[Step] = field(default_factory=list)
    floor: Segment = field(default_factory=lambda: Segment(0.0, 0.0))
    probe: Segment = field(default_factory=lambda: Segment(0.0, 0.0))
    noise: Segment = field(default_factory=lambda: Segment(0.0, 0.0))

    def to_json(self) -> str:
        return json.dumps(asdict(self), indent=1)

    @classmethod
    def from_json(cls, text: str) -> Manifest:
        d = json.loads(text)
        if d.get("format") != FORMAT:
            raise ValueError(f"not a {FORMAT} manifest")
        return cls(
            format=d["format"],
            port=d["port"],
            seed=d["seed"],
            fs=d["fs"],
            band=tuple(d["band"]),  # type: ignore[arg-type]
            probe_band=tuple(d["probe_band"]),  # type: ignore[arg-type]
            length_s=d["length_s"],
            transmissions=[Transmission(**t) for t in d["transmissions"]],
            markers=list(d["markers"]),
            steps=[Step(**s) for s in d["steps"]],
            floor=Segment(**d["floor"]),
            probe=Segment(**d["probe"]),
            noise=Segment(**d["noise"]),
        )


def _db(level: float) -> float:
    return 10.0 ** (level / 20.0)


def marker(port: Port) -> FloatArray:
    lo, hi = PROFILES[port]["band"]
    return sg.chirp(lo + 100.0, hi - 100.0, MARKER_S) * _db(MARKER_DBFS)


def probe(port: Port, seconds: float, seed: int) -> FloatArray:
    """Gaussian noise flat over the probe's band, at ``PROBE_DBFS`` RMS."""
    lo, hi = PROFILES[port]["probe"]
    rng = np.random.default_rng(seed)
    n = round(seconds * FS)
    sos = signal.butter(8, [lo, hi], "bandpass", fs=FS, output="sos")
    x = signal.sosfilt(sos, rng.standard_normal(n + FS))[FS:]
    return np.asarray(x / math.sqrt(float(np.mean(x**2))) * _db(PROBE_DBFS), dtype=np.float64)


def make(port: Port, seed: int = 1) -> tuple[Manifest, FloatArray]:
    """The test sequence and its manifest. The same ``port`` and ``seed`` always give the same
    samples, so ``analyse`` rebuilds what was sent from the manifest alone."""
    prof = PROFILES[port]
    parts: list[FloatArray] = []
    t = 0.0
    m = Manifest(FORMAT, port, seed, FS, prof["band"], prof["probe"], 0.0)

    def add(x: FloatArray) -> float:
        nonlocal t
        start = t
        parts.append(x)
        t += len(x) / FS
        return start

    def silence(seconds: float) -> float:
        return add(np.zeros(round(seconds * FS)))

    def open_transmission() -> float:
        key = t
        silence(KEY_UP_S)
        m.markers.append(add(marker(port)))
        silence(0.25)
        return key

    start = silence(2.0)
    m.noise = Segment(start + 0.25, start + 1.75)
    # transmission 0: the floor, then the probe
    key = open_transmission()
    f0 = silence(2.0)
    m.floor = Segment(f0 + 0.25, f0 + 1.75)
    p0 = add(probe(port, 6.0, seed * 1000 + 999))
    m.probe = Segment(p0 + 0.5, p0 + 6.0)
    silence(0.3)
    m.transmissions.append(Transmission(key, t))
    silence(GAP_S)
    # transmissions 1–6: four drive steps each
    for tx in range(6):
        key = open_transmission()
        for k in range(4):
            i = 4 * tx + k
            level = LEVELS_DBFS[i]
            s = seed * 1000 + i
            x = sg.single_carrier(prof["rate"], prof["centre"], STEP_S, np.random.default_rng(s))
            a = add(x[: round(STEP_S * FS)] * _db(level))
            m.steps.append(Step(a, a + STEP_S, level, s))
        silence(0.2)
        m.transmissions.append(Transmission(key, t))
        silence(GAP_S)
    # transmission 7: the last marker, for the clocks
    key = open_transmission()
    silence(0.25)
    m.transmissions.append(Transmission(key, t))
    silence(2.0)
    m.length_s = t
    return m, np.concatenate(parts)


def rebuild(m: Manifest) -> FloatArray:
    """What was sent, from the manifest."""
    m2, x = make(m.port, m.seed)
    if abs(m2.length_s - m.length_s) > 1e-9:
        raise ValueError("this manifest was made by another version of the sequence")
    return x


def _keyed_mask(m: Manifest, n: int) -> NDArray[np.bool_]:
    keyed = np.zeros(n, dtype=bool)
    for tr in m.transmissions:
        keyed[round(tr.key_s * FS) : round(tr.unkey_s * FS)] = True
    return keyed


# ── Analysis ──────────────────────────────────────────────────────────


@dataclass
class Result:
    port: str
    clock_ppm: float
    delay_s: float
    steps: list[dict[str, float]]
    peak_level_dbfs: float
    peak_sndr_db: float
    response_hz: list[float]
    response_db: list[float]
    floor_db: float
    noise_db: float
    squelch_closes: bool
    key_to_audio_s: list[float]
    release_to_noise_s: list[float]
    release_to_silence_s: list[float]
    notes: list[str] = field(default_factory=list)


def _envelope_db(x: FloatArray, window_s: float = 0.005) -> FloatArray:
    w = max(1, round(window_s * FS))
    p = np.convolve(x**2, np.ones(w) / w, mode="same")
    return np.asarray(10.0 * np.log10(np.maximum(p, 1e-20)), dtype=np.float64)


def _first(mask: NDArray[np.bool_]) -> float | None:
    """The first true sample, in seconds from the mask's start."""
    hit = np.flatnonzero(mask)
    return None if len(hit) == 0 else float(hit[0]) / FS


SQUELCH_SHUT_DB = -60.0
"""The no-carrier stretch this far under the peak's level means the receiver's squelch closes."""


def _peak(levels: list[float], sndr: list[float]) -> tuple[float, float]:
    i = int(np.argmax(sndr))
    if 0 < i < len(sndr) - 1:
        y0, y1, y2 = sndr[i - 1], sndr[i], sndr[i + 1]
        den = y0 - 2.0 * y1 + y2
        frac = 0.5 * (y0 - y2) / den if den != 0.0 else 0.0
        step = levels[1] - levels[0]
        return levels[i] + frac * step, y1 - 0.25 * (y0 - y2) * frac
    return levels[i], sndr[i]


def analyse(m: Manifest, heard: FloatArray, keys: list[Transmission] | None = None) -> Result:
    """Measure a recording of the sequence (or the model's output) with ``aether_model.fm.measure``.

    ``keys`` are the key times as played, when ``play`` logged them; otherwise the manifest's."""
    x = rebuild(m)
    notes: list[str] = []
    mk = marker(m.port)
    first = round(m.markers[0] * FS)
    last = round(m.markers[-1] * FS)
    # coarse: where the first marker landed (a recording starts before the playback)
    search = heard[: first + len(mk) + 10 * FS]
    lag0 = me.delay(mk, search[first:], min(len(search) - first - len(mk), 10 * FS))
    ppm, d0 = me.clock_ppm(mk, heard, first + lag0, last + lag0, 6000)
    ppm_total = ppm
    fixed = me.resample(heard, ppm_total)
    lag = lag0 + d0
    lag = first + me.delay(mk, fixed[first + max(0, lag - 200) :], 400) + max(0, lag - 200) - first
    aligned = fixed[lag:]
    if len(aligned) < len(x):
        aligned = np.concatenate([aligned, np.zeros(len(x) - len(aligned))])
    aligned = aligned[: len(x)]
    steps: list[dict[str, float]] = []
    for st in m.steps:
        a = round((st.start_s + STEP_SKIP_S) * FS)
        b = round(st.end_s * FS)
        r = me.sndr(x[a:b], aligned[a:b], m.band)
        steps.append(
            {
                "level_dbfs": st.level_dbfs,
                "sndr_db": round(r.effective_db, 3),
                "heard_dbfs": round(me.level_db(aligned[a:b], 1.0), 3),
            }
        )
    levels = [s["level_dbfs"] for s in steps]
    sndr = [s["sndr_db"] for s in steps]
    peak_level, peak_sndr = _peak(levels, sndr)
    if int(np.argmax(sndr)) == len(sndr) - 1:
        notes.append("still rising at the top step: the drive never reached the limiter")
    if int(np.argmax(sndr)) == 0:
        notes.append("falling from the first step: the drive was too high throughout")
    pa, pb = round(m.probe.start_s * FS), round(m.probe.end_s * FS)
    f, h = me.response(x[pa:pb], aligned[pa:pb], m.probe_band)
    grid = np.arange(math.ceil(m.probe_band[0] / 50) * 50, m.probe_band[1] + 1, 50.0)
    peak_heard = steps[int(np.argmax(sndr))]["heard_dbfs"]
    fa, fb = round(m.floor.start_s * FS), round(m.floor.end_s * FS)
    na, nb = round(m.noise.start_s * FS), round(m.noise.end_s * FS)
    floor = me.level_db(aligned[fa:fb], 1.0) - peak_heard
    noise = me.level_db(aligned[na:nb], 1.0) - peak_heard
    env = _envelope_db(aligned)
    squelch_closes = noise < SQUELCH_SHUT_DB
    quiet = float(np.median(env[na:nb]))
    up: list[float] = []
    burst: list[float] = []
    shut: list[float] = []
    played = keys if keys is not None else m.transmissions
    for tr in played:
        k, u = round(tr.key_s * FS), round(tr.unkey_s * FS)
        # key: the squelch opening, or (squelch open) the noise quieting under the carrier
        before = float(np.median(env[max(0, k - round(0.5 * FS)) : k]))
        c = _first(np.abs(env[k : k + round(KEY_UP_S * FS)] - before) >= 6.0)
        if c is not None:
            up.append(round(c, 4))
        # release: the carrier gone (the discriminator's noise rises), then the squelch shut
        during = float(np.median(env[u - round(0.15 * FS) : u]))
        late = u + round(1.5 * FS)
        c = _first(env[u:late] >= during + 6.0)
        if c is not None:
            burst.append(round(c, 4))
        if squelch_closes:
            c = _first(env[u:late] <= quiet + 6.0)
            if c is not None:
                shut.append(round(c, 4))
    return Result(
        port=m.port,
        clock_ppm=round(ppm_total, 3),
        delay_s=round(lag / FS, 6),
        steps=steps,
        peak_level_dbfs=round(peak_level, 3),
        peak_sndr_db=round(peak_sndr, 3),
        response_hz=[float(v) for v in grid],
        response_db=[round(float(v), 3) for v in np.interp(grid, f, h)],
        floor_db=round(floor, 3),
        noise_db=round(noise, 3),
        squelch_closes=squelch_closes,
        key_to_audio_s=up,
        release_to_noise_s=burst,
        release_to_silence_s=shut,
        notes=notes,
    )


# ── The model ─────────────────────────────────────────────────────────

PRESETS = {"wide_voice": ch.wide_voice, "narrow_voice": ch.narrow_voice, "flat_port": ch.flat_port}


def run_model(
    m: Manifest, config: ch.FmLinkConfig, lead_s: float = 0.37, block: int = 960
) -> FloatArray:
    """The sequence through the model, keyed as the manifest says, as a recording would hear it:
    ``lead_s`` of the receiver's audio before the playback starts."""
    x = rebuild(m)
    keyed = _keyed_mask(m, len(x))
    link = ch.FmLink(config)
    out = [link.process(np.zeros(round(lead_s * FS)), False)]
    for i in range(0, len(x), block):
        # the key is held for a block: transitions fall on the block grid, as a daemon's would
        out.append(link.process(x[i : i + block], bool(keyed[i])))
    return np.concatenate(out)


BASELINE = [
    # preset, the radio input's gain that puts the knee inside the steps, port
    ("wide_voice", 20.0, "voice"),
    ("narrow_voice", 20.0, "voice"),
    ("flat_port", 12.0, "flat"),
]
BASELINE_CNRS = [9.0, 12.0, 18.0, 30.0]


def baseline(out: Path) -> None:
    """Every preset at each CNR through the sequence: the uncalibrated model's curves
    (ADR-0115 §7), one row a drive step, then one summary row a run."""
    import csv

    rows: list[dict[str, object]] = []
    for preset, gain, port in BASELINE:
        m, _ = make(port, 1)  # type: ignore[arg-type]
        for cnr in BASELINE_CNRS:
            cfg = PRESETS[preset](cnr=cnr, seed=1)
            cfg = replace(cfg, transmitter=replace(cfg.transmitter, gain_db=gain))
            r = analyse(m, run_model(m, cfg))
            for s in r.steps:
                rows.append(
                    {
                        "preset": preset,
                        "cnr_db": cnr,
                        "level_dbfs": s["level_dbfs"],
                        "sndr_db": s["sndr_db"],
                        "what": "step",
                    }
                )
            rows.append(
                {
                    "preset": preset,
                    "cnr_db": cnr,
                    "level_dbfs": r.peak_level_dbfs,
                    "sndr_db": r.peak_sndr_db,
                    "what": "peak",
                }
            )
            print(
                f"{preset:13s} CNR {cnr:4.0f}: peak {r.peak_sndr_db:5.1f} dB at "
                f"{r.peak_level_dbfs:6.1f} dBFS, floor {r.floor_db:6.1f} dB"
            )
    with out.open("w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=["preset", "cnr_db", "what", "level_dbfs", "sndr_db"])
        w.writeheader()
        w.writerows(rows)


# ── Playback and recording ────────────────────────────────────────────


def play(
    m: Manifest,
    wav: Path,
    out: Path,
    play_device: str | None,
    record_device: str | None,
    key: str,
) -> None:  # pragma: no cover - needs sound cards and a radio
    import sounddevice as sd

    rate, data = wavfile.read(wav)
    if rate != FS:
        raise SystemExit(f"{wav} is {rate} Hz, not {FS}")
    x = data.astype(np.float32) / 32768.0
    line = None
    if key != "vox":
        kind, port = key.split(":", 1)
        import serial  # type: ignore[import-not-found]

        line = serial.Serial(port)
        line.rts = False
        line.dtr = False

    def set_key(on: bool) -> None:
        if line is None:
            return
        if kind == "rts":
            line.rts = on
        else:
            line.dtr = on

    tail = int(3 * FS)
    rec = np.zeros(len(x) + tail, dtype=np.float32)
    pos = {"play": 0, "rec": 0}
    played: list[Transmission] = []
    started = threading.Event()

    def out_cb(buf: Any, frames: int, _t: Any, _s: Any) -> None:
        a = pos["play"]
        chunk = x[a : a + frames]
        buf[: len(chunk), 0] = chunk
        buf[len(chunk) :, 0] = 0.0
        pos["play"] = a + frames
        started.set()

    def in_cb(buf: Any, frames: int, _t: Any, _s: Any) -> None:
        a = pos["rec"]
        n = min(frames, len(rec) - a)
        rec[a : a + n] = buf[:n, 0]
        pos["rec"] = a + n

    with (
        sd.InputStream(samplerate=FS, channels=1, device=record_device, callback=in_cb),
        sd.OutputStream(samplerate=FS, channels=1, device=play_device, callback=out_cb),
    ):
        started.wait()
        t0 = time.monotonic() - pos["play"] / FS
        for tr in m.transmissions:
            while time.monotonic() - t0 < tr.key_s:
                time.sleep(0.001)
            set_key(True)
            k = time.monotonic() - t0
            while time.monotonic() - t0 < tr.unkey_s:
                time.sleep(0.001)
            set_key(False)
            played.append(Transmission(round(k, 4), round(time.monotonic() - t0, 4)))
        while pos["rec"] < len(rec):
            time.sleep(0.05)
    wavfile.write(out, FS, (np.clip(rec, -1, 1) * 32767).astype(np.int16))
    out.with_suffix(".keys.json").write_text(
        json.dumps({"key": key, "transmissions": [asdict(t) for t in played]}, indent=1)
    )
    print(f"wrote {out} and {out.with_suffix('.keys.json')}")


# ── Command line ──────────────────────────────────────────────────────


def _read_wav(path: Path) -> FloatArray:
    rate, data = wavfile.read(path)
    if rate != FS:
        raise SystemExit(f"{path} is {rate} Hz, not {FS}")
    d = np.asarray(data)
    if d.ndim > 1:
        d = d[:, 0]
    if d.dtype == np.int16:
        return d.astype(np.float64) / 32768.0
    if d.dtype == np.int32:
        return d.astype(np.float64) / 2147483648.0
    return d.astype(np.float64)


def _pcm(x: FloatArray) -> NDArray[np.int16]:
    """16-bit samples, clipped first: a value over full scale would wrap around, not clip."""
    return np.asarray(np.round(np.clip(x, -1.0, 1.0) * 32767), dtype=np.int16)


def _print(result: Result) -> None:
    print(f"clock {result.clock_ppm:+.2f} ppm, delay {result.delay_s * 1000:.1f} ms")
    print(f"peak SNDR {result.peak_sndr_db:.1f} dB at {result.peak_level_dbfs:.1f} dBFS")
    for s in result.steps:
        bar = "#" * max(0, round(s["sndr_db"]))
        print(f"  {s['level_dbfs']:6.1f} dBFS  {s['sndr_db']:5.1f} dB  {bar}")
    shut = "squelch closes" if result.squelch_closes else "squelch open"
    print(f"floor {result.floor_db:.1f} dB, no carrier {result.noise_db:.1f} dB ({shut}),")
    print("  both against the received level at the peak")
    print(f"key to audio {result.key_to_audio_s} s")
    print(f"release to the carrier gone {result.release_to_noise_s} s")
    if result.squelch_closes:
        print(f"release to silence {result.release_to_silence_s} s")
    for n in result.notes:
        print("note:", n)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    mk = sub.add_parser("make", help="write the test sequence and its manifest")
    mk.add_argument("--port", choices=["voice", "flat"], default="voice")
    mk.add_argument("--seed", type=int, default=1)
    mk.add_argument("--out", type=Path, default=Path("."))
    an = sub.add_parser("analyse", help="measure a recording of the sequence")
    an.add_argument("manifest", type=Path)
    an.add_argument("recording", type=Path)
    an.add_argument("--out", type=Path)
    mo = sub.add_parser("model", help="the same measurements on the FM channel model")
    mo.add_argument("manifest", type=Path)
    mo.add_argument("--preset", choices=sorted(PRESETS), default="wide_voice")
    mo.add_argument("--cnr", type=float, default=30.0)
    mo.add_argument("--seed", type=int, default=0)
    mo.add_argument("--tx-gain-db", type=float, default=0.0)
    mo.add_argument("--wav", type=Path, help="also write the model's 'recording'")
    mo.add_argument("--out", type=Path)
    pl = sub.add_parser("play", help="play the sequence into one radio and record the other")
    pl.add_argument("manifest", type=Path)
    pl.add_argument("--out", type=Path, required=True)
    pl.add_argument("--play")
    pl.add_argument("--record")
    pl.add_argument("--key", default="vox", help="vox, rts:PORT or dtr:PORT")
    sub.add_parser("devices", help="list the sound cards")
    bl = sub.add_parser("baseline", help="the uncalibrated model's curves, as a CSV")
    bl.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()

    if args.cmd == "make":
        m, x = make(args.port, args.seed)
        args.out.mkdir(parents=True, exist_ok=True)
        wavfile.write(args.out / f"{args.port}.wav", FS, _pcm(x))
        (args.out / f"{args.port}.json").write_text(m.to_json())
        print(f"wrote {args.out / (args.port + '.wav')} ({m.length_s:.1f} s) and its manifest")
        return 0
    if args.cmd == "devices":  # pragma: no cover
        import sounddevice as sd

        print(sd.query_devices())
        return 0
    if args.cmd == "baseline":
        baseline(args.out)
        return 0
    m = Manifest.from_json(args.manifest.read_text())
    if args.cmd == "play":  # pragma: no cover
        play(m, args.manifest.with_suffix(".wav"), args.out, args.play, args.record, args.key)
        return 0
    if args.cmd == "analyse":
        keys_file = args.recording.with_suffix(".keys.json")
        keys = None
        if keys_file.exists():
            keys = [Transmission(**t) for t in json.loads(keys_file.read_text())["transmissions"]]
        result = analyse(m, _read_wav(args.recording), keys)
    else:
        cfg = PRESETS[args.preset](cnr=args.cnr, seed=args.seed)
        cfg = replace(cfg, transmitter=replace(cfg.transmitter, gain_db=args.tx_gain_db))
        heard = run_model(m, cfg)
        if args.wav:
            wavfile.write(args.wav, FS, _pcm(heard))
        result = analyse(m, heard)
    _print(result)
    if args.out:
        args.out.write_text(json.dumps(asdict(result), indent=1))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
