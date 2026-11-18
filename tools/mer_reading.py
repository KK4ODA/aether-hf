"""The modulation error ratio of OFDM frames, beside the receiver's own SNR (issue #15).

    python -I tools/mer_reading.py field RECORDING.wav SIDECAR.json [WAV SIDECAR ...]
                                   [--frames] [--carriers] [--receiver KEY=VALUE ...]
    python -I tools/mer_reading.py sim [--hz 2300|500] [--profile awgn,good,moderate,poor,nvis]
                                   [--snr 6,12,18,24] [--rung R,R,...] [--frames N] [--no-papr]
                                   [--no-blanker] [--seed S] [--receiver KEY=VALUE ...]

Every frame is received again by the model. A frame that decodes has its payload re-encoded at
the redundancy version it came at: the symbols that went on the air, exactly, so its error is
measured against the truth, not against the nearest constellation point. Per frame, in dB:

``pilot``  the receiver's SNR per carrier, from the known pilots' residual after the channel
           estimate (``ReceivedFrame.snr_carrier_db``; the 3 kHz figure the panel shows, ``snr3k``,
           is this less 10·log10(3000 / occupied bandwidth)). It is already an error ratio, on the
           pilots.
``mer``    the textbook MER: sent power over error power on the equalised data symbols. A
           carrier in a fade has its noise multiplied by 1/|H|², so on a selective path it is
           ruled by the few deepest carriers.
``mer|h|`` the same with each symbol weighted by |H|² (error and signal before equalising): the
           like-for-like comparison with ``pilot``; ``gap`` is ``pilot − mer|h|``, what the data
           carriers lose that the pilots do not.
``da``     ``mer|h|`` again against a channel re-estimated from the data itself, now that it is
           known: each symbol's channel the mean of the same carrier's over the two data symbols
           either side (itself left out). It reads about 1 dB low from that estimate's own noise,
           and low on a channel that moves within a few symbols (flutter). Where ``da`` stands well
           above ``mer|h|``, the loss was the receiver's channel estimate across frequency, between
           its pilots; where it does not, the loss is in the signal (distortion, noise the pilots
           did not see).
``dd``     the decision-directed MER, against the nearest point, as a receiver without the
           payload would read it: it reads high wherever a decision is wrong. The only figure for
           a frame that did not decode, whose mode is the one its chips read.
``model``  how much larger the data's error is than the noise the receiver handed the decoder
           for it (mean |error|² / noise variance): 0 if the decoder's LLRs are scaled right,
           positive if they are too confident. Against the nearest point for a frame that did not
           decode.

``--carriers`` prints, per 100 Hz, the median over decoded frames of each data carrier's
``mer|h|`` relative to its frame's: a narrowband interferer, a receive filter or a tilted transmit
audio shows as a band that stays down from frame to frame. Tone frames have no constellation and
are counted, not measured.

``--receiver KEY=VALUE`` sets a ``FrameReceiver`` setting for the run (``derotate=false`` is the
receiver before ADR-0131; the settings as ``bench_cut.py`` takes them), to measure a receiver
change.

``sim`` runs the same measurement on the model's channel (OFDM rungs at a set SNR), where the SNR
is known: the check that the tool reads what is there, and the loss by channel class.
"""

from __future__ import annotations

import argparse
import json
import sys
import wave
from dataclasses import dataclass
from pathlib import Path

import numpy as np
from numpy.typing import NDArray

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from aether_model.channel import make_channel
from aether_model.frame.modes import air_interface
from aether_model.phy.constellation import constellation
from aether_model.phy.passband import AudioToBaseband
from aether_model.phy.pipeline import DecodedFrame, Modem
from aether_model.phy.preamble import FrameType
from aether_model.phy.tx import FrameTransmitter
from aether_model.waveform import NARROW_500, WIDE_2300, WaveformParams

FloatArray = NDArray[np.float64]
ComplexArray = NDArray[np.complex128]

MARGIN_S = 0.3
"""How far either side of a recorded frame's span the slice reaches: the front end's delay and
the recorded timing's rounding (as ``passband_witness.py``)."""
BAND_HZ = 100
"""The width per-carrier readings are gathered into for printing."""
NAN = float("nan")


def db(x: float) -> float:
    return float(10 * np.log10(max(x, 1e-30)))


@dataclass
class FrameMer:
    kind: str
    """``control`` or ``data``."""
    decoded: bool
    mode: int
    """The OFDM mode index (the control mode for a control frame)."""
    rung: int | None
    """The ladder rung of a data frame's mode; ``None`` for a control frame or a mode on no rung."""
    rv: int
    modulation: str
    snr_3k_db: float
    pilot_db: float
    mer_db: float
    mer_h_db: float
    da_db: float
    dd_db: float
    model_db: float
    motion: float | None
    carrier_hz: FloatArray
    carrier_mer_h_db: FloatArray
    """Each data carrier's |H|²-weighted MER over the frame's data symbols (NaN undecoded)."""

    @property
    def gap_db(self) -> float:
        return self.pilot_db - self.mer_h_db


def rung_of(params: WaveformParams, mode: int) -> int | None:
    for i, r in enumerate(air_interface(params).ladder):
        if r.mode is not None and r.mode.index == mode:
            return i
    return None


def neighbours_mean(g: ComplexArray, reach: int = 2) -> ComplexArray:
    """Each element's mean over the ``reach`` symbols either side of it on its own carrier (fewer
    at the frame's ends), itself left out: an estimate that follows the channel across frequency
    exactly, whatever the delay spread, and in time as far as it barely moves over a few symbols."""
    rows = g.shape[0]
    padded = np.pad(g, ((reach, reach), (0, 0)))
    ones = np.pad(np.ones(g.shape), ((reach, reach), (0, 0)))
    total = sum(padded[i : i + rows] for i in range(2 * reach + 1))
    count = sum(ones[i : i + rows] for i in range(2 * reach + 1))
    return np.asarray((total - g) / (count - 1), dtype=np.complex128)


def measure(modem: Modem, dec: DecodedFrame) -> FrameMer | None:
    """The error of an OFDM frame's data symbols; ``None`` for a tone frame."""
    frame, mode, payload = dec.frame, dec.mode, dec.payload
    if frame is None or mode is None:
        return None
    p = modem.p
    layout = frame.layout
    data_c = modem.rx.data_c
    n_data = len(data_c)
    eq = frame.symbols.reshape(-1, n_data)
    nv = frame.noise_var.reshape(-1, n_data)
    # ``channel`` is every symbol after the preamble; the data symbols are those that are not
    # full pilot symbols, in order (FrameReceiver.receive)
    rows = [i for i in range(frame.channel.shape[0]) if i not in layout.pilot_symbol_indices]
    h = frame.channel[rows][:, data_c]
    h2 = np.abs(h) ** 2
    points = constellation(mode.modulation).points
    nearest = points[np.argmin(np.abs(eq[:, :, None] - points[None, None, :]), axis=2)]
    dd2 = np.abs(eq - nearest) ** 2
    n = p.n_carriers
    hz = p.centre_hz + (np.arange(n) - n // 2) * p.subcarrier_spacing_hz
    mer = mer_h = da = NAN
    carrier = np.full(n_data, NAN)
    model = db(float(np.mean(dd2 / nv)))
    if payload is not None:
        sent = np.asarray(modem.codec(mode, layout).encode(payload, frame.rv)).reshape(-1, n_data)
        err2 = np.abs(eq - sent) ** 2
        s2 = np.abs(sent) ** 2
        mer = db(float(s2.sum()) / float(err2.sum()))
        mer_h = db(float((h2 * s2).sum()) / float((h2 * err2).sum()))
        model = db(float(np.mean(err2 / nv)))
        carrier = 10 * np.log10((h2 * s2).sum(axis=0) / np.maximum((h2 * err2).sum(axis=0), 1e-30))
        # the channel re-estimated from the data, now that the data is known
        raw = eq * h
        h_da = neighbours_mean(raw / sent)
        da = db(
            float(np.sum(np.abs(h_da * sent) ** 2)) / float(np.sum(np.abs(raw - h_da * sent) ** 2))
        )
    control = frame.sync.header.frame_type is FrameType.CONTROL
    return FrameMer(
        kind="control" if control else "data",
        decoded=payload is not None,
        mode=mode.index,
        rung=None if control else rung_of(p, mode.index),
        rv=frame.rv,
        modulation=mode.modulation.name,
        snr_3k_db=frame.snr_3k_db,
        pilot_db=frame.snr_carrier_db,
        mer_db=mer,
        mer_h_db=mer_h,
        da_db=da,
        dd_db=db(float((np.abs(nearest) ** 2).sum()) / float(dd2.sum())),
        model_db=model,
        motion=frame.motion_weight,
        carrier_hz=np.asarray(hz[data_c], dtype=np.float64),
        carrier_mer_h_db=np.asarray(carrier, dtype=np.float64),
    )


# ── a summary ─────────────────────────────────────────────────────────


LABEL = 24
HEADER = (
    f"{'':>{LABEL}} {'n':>4} {'snr3k':>6} {'pilot':>6} {'mer':>6} {'mer|h|':>6} {'da':>6} "
    f"{'gap':>5} {'dd':>6} {'model':>6}"
)


def summary_line(label: str, rows: list[FrameMer]) -> str:
    def med(values: list[float], width: int, sign: str = "") -> str:
        kept = [v for v in values if not np.isnan(v)]
        return f"{float(np.median(kept)):>{sign}{width}.1f}" if kept else f"{'-':>{width}}"

    return (
        f"{label:>{LABEL}} {len(rows):>4} {med([r.snr_3k_db for r in rows], 6)} "
        f"{med([r.pilot_db for r in rows], 6)} {med([r.mer_db for r in rows], 6)} "
        f"{med([r.mer_h_db for r in rows], 6)} {med([r.da_db for r in rows], 6)} "
        f"{med([r.gap_db for r in rows], 5)} {med([r.dd_db for r in rows], 6)} "
        f"{med([r.model_db for r in rows], 6, '+')}"
    )


def group_label(r: FrameMer) -> str:
    what = (
        f"control {r.modulation}"
        if r.kind == "control"
        else f"{f'rung {r.rung}' if r.rung is not None else f'mode {r.mode}'} {r.modulation}"
    )
    return what if r.decoded else f"{what} failed"


def print_summary(rows: list[FrameMer]) -> None:
    groups: dict[str, list[FrameMer]] = {}
    for r in rows:
        groups.setdefault(group_label(r), []).append(r)
    print(
        "  medians over frames, dB (gap = pilot − mer|h|; da = mer|h| on a data-aided channel; "
        "model = error over the noise the decoder was given)"
    )
    print("  " + HEADER)
    for label in sorted(
        groups,
        key=lambda g: (groups[g][0].kind == "data", groups[g][0].mode, not groups[g][0].decoded),
    ):
        print("  " + summary_line(label, groups[label]))
    decoded = [r for r in rows if r.decoded]
    if len(groups) > 1 and decoded:
        print("  " + summary_line("all decoded", decoded))


def print_carriers(rows: list[FrameMer]) -> None:
    bands: dict[int, list[float]] = {}
    for r in rows:
        if not r.decoded:
            continue
        rel = r.carrier_mer_h_db - r.mer_h_db
        per_band: dict[int, list[float]] = {}
        for f, v in zip(r.carrier_hz.tolist(), rel.tolist(), strict=True):
            per_band.setdefault(int(f // BAND_HZ * BAND_HZ), []).append(v)
        for band, values in per_band.items():
            bands.setdefault(band, []).append(float(np.mean(values)))
    if not bands:
        return
    print(f"  each data carrier's mer|h| against its frame's, median over frames, by {BAND_HZ} Hz")
    print("    " + "  ".join(f"{band}:{np.median(v):+.1f}" for band, v in sorted(bands.items())))


# ── recordings ────────────────────────────────────────────────────────


def read_wav(wav: Path) -> FloatArray:
    with wave.open(str(wav)) as w:
        raw = w.readframes(w.getnframes())
        x = np.frombuffer(raw, dtype=np.int16).astype(np.float64) / 32768
        channels = w.getnchannels()
    return x.reshape(-1, channels)[:, 0] if channels > 1 else x


def field(wav: Path, side: Path, frames: bool, carriers: bool) -> list[FrameMer]:
    d = json.loads(side.read_text(encoding="utf-8"))
    hz_air = int(d["session"].get("bandwidth_hz") or 2300)
    params = WIDE_2300 if hz_air == 2300 else NARROW_500
    bb = AudioToBaseband(params).process(read_wav(wav))
    fs = params.fs_baseband
    modem = Modem(params)
    rows: list[FrameMer] = []
    live: list[float] = []
    tone = missed = 0
    for fr in d["frames"]:
        if fr.get("start_s") is None or (fr.get("kind") == "measure" and not fr.get("decoded")):
            continue
        a = max(0, int((fr["start_s"] - MARGIN_S) * fs))
        b = min(len(bb), int((fr["end_s"] + MARGIN_S) * fs))
        found = modem.decode_buffer(bb[a:b], max_frames=2)
        # the frame the station recorded: the one that decoded if one did, else the first
        # ordinary frame found there
        pick = next((x for x in found if x.ok), None) if fr.get("decoded") else None
        pick = pick or next((x for x in found if x.frame is not None), None)
        if pick is None:
            tone += bool(found)
            missed += not found
            continue
        if pick.frame is None:
            tone += 1
            continue
        r = measure(modem, pick)
        if r is None:
            continue
        rows.append(r)
        if r.decoded and fr.get("decoded") and fr.get("snr_3k_db") is not None:
            live.append(r.snr_3k_db - float(fr["snr_3k_db"]))
        if frames:
            print(
                f"  {fr['start_s']:8.2f}s {group_label(r):>{LABEL}} rv {r.rv} "
                f"snr3k {r.snr_3k_db:5.1f} pilot {r.pilot_db:5.1f} mer {r.mer_db:5.1f} "
                f"mer|h| {r.mer_h_db:5.1f} da {r.da_db:5.1f} dd {r.dd_db:5.1f} "
                f"model {r.model_db:+5.1f}"
                + (f" motion {r.motion:.2f}" if r.motion is not None else "")
            )
    print(
        f"{wav.name} ({hz_air} Hz air): {len(rows)} OFDM frames measured "
        f"({sum(r.decoded for r in rows)} decoded), {tone} tone frames, {missed} not found again"
        + (
            f"; the model's SNR reads {np.median(live):+.2f} dB from the live receiver's"
            if live
            else ""
        )
    )
    if rows:
        print_summary(rows)
        if carriers:
            print_carriers(rows)
    return rows


# ── the model's channel ───────────────────────────────────────────────


def sim(args: argparse.Namespace) -> None:
    params = WIDE_2300 if args.hz == 2300 else NARROW_500
    air = air_interface(params)
    modem = Modem(params, blank_impulses=not args.no_blanker)
    if args.no_papr:
        modem.tx = FrameTransmitter(params, papr_reduction=False)
    fs = params.fs_baseband
    rungs = (
        [int(r) for r in args.rung.split(",")]
        if args.rung
        else [i for i, r in enumerate(air.ladder) if r.mode is not None]
    )
    rng = np.random.default_rng(args.seed)
    print(
        f"{args.hz} Hz air, {args.frames} frames a point, peak reduction "
        f"{'off' if args.no_papr else 'on'}, impulse blanker {'off' if args.no_blanker else 'on'}; "
        "'set' is the channel's 3 kHz SNR less the frame's own power under 1; decoded frames only"
    )
    print(f"  {'profile':>9} {'set':>5} {'dec':>5} " + HEADER)
    for profile in args.profile.split(","):
        for snr in [float(s) for s in args.snr.split(",")]:
            for rung in rungs:
                mode = air.ladder[rung].mode
                if mode is None:
                    continue
                rows: list[FrameMer] = []
                powers: list[float] = []
                for _ in range(args.frames):
                    payload = rng.integers(
                        0, 256, modem.rung_payload_bytes(rung), dtype=np.uint8
                    ).tobytes()
                    burst = modem.rung_burst(payload, rung)
                    powers.append(float(np.mean(np.abs(burst) ** 2)))
                    lead = int(rng.uniform(0.3, 0.6) * fs)
                    x = np.concatenate(
                        (np.zeros(lead, complex), burst, np.zeros(int(0.8 * fs), complex))
                    )
                    y = make_channel(
                        profile,
                        snr_db=snr,
                        fs=fs,
                        seed=int(rng.integers(0, 2**31)),
                        signal_power=1.0,
                    ).process(x)
                    for dec in modem.decode_buffer(y, max_frames=2):
                        if (
                            dec.ok
                            and dec.payload is not None
                            and dec.payload[: len(payload)] == payload
                        ):
                            r = measure(modem, dec)
                            if r is not None:
                                rows.append(r)
                            break
                set_db = snr + db(float(np.mean(powers)))
                label = f"rung {rung} {mode.modulation.name}"
                if rows:
                    print(
                        f"  {profile:>9} {set_db:>5.1f} {len(rows):>2}/{args.frames:<2} "
                        + summary_line(label, rows)
                    )
                else:
                    print(f"  {profile:>9} {set_db:>5.1f}  0/{args.frames:<2} {label:>{LABEL}}")


def main() -> None:
    # Windows' console code page cannot print the docstring's or the summary's signs
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    f = sub.add_parser("field", help="frames of recordings, each WAV with its sidecar")
    f.add_argument("pairs", nargs="+", type=Path)
    f.add_argument("--frames", action="store_true", help="a line per frame")
    f.add_argument("--carriers", action="store_true", help="per-carrier readings by band")
    f.add_argument("--receiver", action="append", default=[], help="a FrameReceiver KEY=VALUE")
    s = sub.add_parser("sim", help="OFDM rungs through the model's channel")
    s.add_argument("--hz", type=int, default=2300, choices=(2300, 500))
    s.add_argument("--profile", default="awgn")
    s.add_argument("--snr", default="6,12,18,24", help="3 kHz SNR, dB, comma-separated")
    s.add_argument("--rung", default="", help="rungs, comma-separated (default every OFDM rung)")
    s.add_argument("--frames", type=int, default=10)
    s.add_argument("--no-papr", action="store_true", help="the transmitter's peak reduction off")
    s.add_argument(
        "--no-blanker",
        action="store_true",
        help="the receiver's impulse blanker off (with --no-papr: it takes the peaks for impulses)",
    )
    s.add_argument("--seed", type=int, default=1)
    s.add_argument("--receiver", action="append", default=[], help="a FrameReceiver KEY=VALUE")
    args = ap.parse_args()
    if args.receiver:
        from bench_cut import configure

        configure(tuple(args.receiver))
        print("receiver:", ", ".join(args.receiver))
    if args.cmd == "field":
        if len(args.pairs) % 2:
            raise SystemExit("field takes WAV SIDECAR pairs")
        everything: list[FrameMer] = []
        for wav, side in zip(args.pairs[::2], args.pairs[1::2], strict=True):
            everything += field(wav, side, args.frames, args.carriers)
        if len(args.pairs) > 2 and everything:
            print("all recordings")
            print_summary(everything)
            if args.carriers:
                print_carriers(everything)
    else:
        sim(args)


if __name__ == "__main__":
    main()
