"""What the busy detector's noise floor did on a recording, and every AGC event in it.

The panel plots the floor twice a second and the log says only when the busy state changed;
neither says why a floor went where it went. This runs a recording through the daemon's own
detector (``aetherd --busy-trace``: the same front end and the same code, muted where the
sidecar says the station transmitted) and reads the result block by block — 25 ms each: the
level, whether the block was steady and taken as evidence of the noise, the floor, the busy
state and its reason, and from the 48 kHz audio the power in Aether's channel, in the rest of
the radio's passband, and the radio's gain as the passband shows it (each spectral bin against its
own level on quiet: a cut moves every bin down together, a signal only lifts the bins it holds).

Then it finds every **AGC event** — the gain reading 6 dB or more under its level on quiet, for at
least half a second — and measures each: how deep and how
long; how fast the noise came back (dB/s, from 90 % to 10 % of the depth); whether the signal that
caused it was in the channel or outside it (which band's power rose at the onset); what the floor
did; and how long the channel read busy by level after the gain came back (ADR-0078). When the
sidecar holds what the radio said of itself — its S-meter, dial and settings — and the operator's
marks, those are listed too, the S-meter beside each event.

    python tools/floor_trace.py RECORDING.wav [--expect SIDECAR.json] [--bandwidth 500]
                                [--variant normalised] [--daemon PATH] [--csv OUT] [--svg OUT]

The sidecar beside the WAV is used when ``--expect`` is not given. ``--variant`` runs one of the
detector's candidates instead of the one it ships with (``aetherd --help``).
"""

from __future__ import annotations

import argparse
import csv
import io
import json
import subprocess
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
BLOCK_S = 0.025
EVENT_DB = 6.0
"""How far the gain must read under its level on quiet to count as an AGC event."""
EVENT_MIN_S = 0.5


def daemon_default() -> Path:
    for name in ("aetherd", "aetherd.exe"):
        path = ROOT / "core" / "target" / "release" / name
        if path.exists():
            return path
    return ROOT / "core" / "target" / "release" / "aetherd"


def run_trace(
    daemon: Path, wav: Path, expect: Path | None, bandwidth: int | None, variant: str | None
) -> dict[str, np.ndarray]:
    cmd = [str(daemon), "--busy-trace", str(wav)]
    if expect is not None:
        cmd += ["--expect", str(expect)]
    if bandwidth is not None:
        cmd += ["--bandwidth", str(bandwidth)]
    if variant:
        cmd += ["--busy-variant", variant]
    out = subprocess.run(cmd, check=True, capture_output=True, text=True).stdout
    rows = list(csv.DictReader(io.StringIO(out)))
    cols: dict[str, np.ndarray] = {}
    for key in rows[0]:
        if key == "reason":
            cols[key] = np.array([r[key] for r in rows])
        else:
            cols[key] = np.array([float(r[key]) for r in rows])
    return cols


def spans(flag: np.ndarray) -> list[tuple[int, int]]:
    """Runs of True, as [start, end) index pairs."""
    out, start = [], None
    for i, f in enumerate(flag):
        if f and start is None:
            start = i
        elif not f and start is not None:
            out.append((start, i))
            start = None
    if start is not None:
        out.append((start, len(flag)))
    return out


def events(tr: dict[str, np.ndarray]) -> list[dict]:
    """Every AGC event, measured."""
    t, gain = tr["t_s"], tr["gain_db"]
    if not np.isfinite(gain).any():
        return []
    n = len(t)
    drop = -np.minimum(np.nan_to_num(gain, nan=0.0), 0.0)
    deep = drop >= EVENT_DB
    found = []
    for a, b in spans(deep):
        if (b - a) * BLOCK_S < EVENT_MIN_S:
            continue
        depth = float(np.nanmedian(drop[a:b]))
        # the release: from the last block still within 90 % of the event's typical depth to
        # the first back within 10 % of it
        r90 = max((i for i in range(a, b) if drop[i] >= 0.9 * depth), default=None)
        r10 = (
            next((i for i in range(r90, min(n, b + 400)) if drop[i] <= 0.1 * depth), None)
            if r90 is not None
            else None
        )
        ramp = (
            0.8 * depth / ((r10 - r90) * BLOCK_S)
            if r90 is not None and r10 is not None and r10 > r90
            else float("nan")
        )
        # where the cause sat: which band's power rose at the onset, against the second before
        pre = slice(max(0, a - 40), max(1, a - 2))
        on = slice(a + 2, min(n, a + 20))
        rise_in = float(np.median(tr["channel_db"][on]) - np.median(tr["channel_db"][pre]))
        rise_out = float(np.median(tr["outside_db"][on]) - np.median(tr["outside_db"][pre]))
        cause = "in the channel" if rise_in > rise_out else "outside the channel"
        # false busy after: a run of busy by level that begins within 2 s of the gain coming
        # back, to its end
        end = b
        start = next(
            (
                i
                for i in range(end, min(n, end + int(2.0 / BLOCK_S)))
                if tr["busy"][i] > 0 and tr["reason"][i] == "level"
            ),
            None,
        )
        k = start if start is not None else end
        while start is not None and k < n and tr["busy"][k] > 0 and tr["reason"][k] == "level":
            k += 1
        busy_after = (k - end) * BLOCK_S if start is not None else 0.0
        found.append(
            {
                "start_s": float(t[a]),
                "length_s": float((b - a) * BLOCK_S),
                "depth_db": depth,
                "deepest_db": float(np.nanmax(drop[a:b])),
                "release_db_per_s": ramp,
                "cause": cause,
                "channel_rise_db": rise_in,
                "outside_rise_db": rise_out,
                "floor_before_db": float(tr["floor_db"][max(0, a - 1)]),
                "floor_lowest_db": float(np.min(tr["floor_db"][a : min(n, b + 1)])),
                "floor_after_db": float(tr["floor_db"][min(n - 1, end)]),
                "busy_after_s": float(busy_after),
            }
        )
    return found


def radio_report(sidecar: dict, found: list[dict]) -> list[str]:
    """What the radio said of itself while the recording ran (the sidecar's ``radio``, from the
    beta after 0.2.0-beta.114) and the operator's marks: the S-meter beside each AGC event — the
    meter follows the AGC itself — every change of dial and setting, and every mark."""
    lines = []
    radio = sidecar.get("radio") or {}
    meter = radio.get("meter") or {}
    readings = np.array(meter.get("readings") or [], dtype=float).reshape(-1, 2)
    if len(readings):
        lines.append(f"  S-meter: {len(readings)} readings ({meter.get('scale')})")

        def median_in(a: float, b: float) -> float:
            inside = readings[(readings[:, 0] >= a) & (readings[:, 0] < b), 1]
            return float(np.median(inside)) if len(inside) else float("nan")

        for e in found:
            start, end = e["start_s"], e["start_s"] + e["length_s"]
            lines.append(
                f"    event at {start:8.1f} s: S-meter {median_in(start - 3.0, start - 0.5):.0f} "
                f"before, {median_in(start + 0.25 * e['length_s'], end - 0.25 * e['length_s']):.0f}"
                f" during, {median_in(end + 0.5, end + 3.0):.0f} after"
            )
    for t_s, hz in radio.get("dial") or []:
        lines.append(f"  dial at {t_s:8.1f} s: {hz / 1e6:.6f} MHz")
    for setting in radio.get("settings") or []:
        lines.append(f"  {setting['name']} at {setting['t_s']:8.1f} s: {setting['value']}")
    for event in sidecar.get("events") or []:
        if event.get("event") == "mark":
            lines.append(f"  mark {event.get('detail')} at {event.get('t_s', 0.0):8.1f} s")
    return lines


def write_svg(path: Path, tr: dict[str, np.ndarray], title: str) -> None:
    """Level, floor, the gain reading and busy, on one axis: the gain is drawn from the floor's
    median, so a cut shows as the noise would have gone with it."""
    t = tr["t_s"]
    width, height, left, top = 1400, 420, 60, 30
    anchor = float(np.nanmedian(tr["floor_db"][np.isfinite(tr["floor_db"])]))
    gain_line = anchor + np.minimum(tr["gain_db"], 0.0)
    series = [tr["power_db"], tr["floor_db"], gain_line]
    finite = np.concatenate([s[np.isfinite(s)] for s in series])
    low, high = float(np.floor(finite.min() / 6) * 6), float(np.ceil(finite.max() / 6) * 6)
    step = max(1, len(t) // 4000)
    x = left + (width - left - 20) * t[::step] / t[-1]

    def y(v: np.ndarray) -> np.ndarray:
        return top + (height - top - 40) * (high - v) / (high - low)

    def polyline(v: np.ndarray, colour: str, extra: str = "") -> str:
        v = np.nan_to_num(v[::step], nan=low)
        points = " ".join(f"{a:.1f},{b:.1f}" for a, b in zip(x, y(v), strict=True))
        return (
            f'<polyline fill="none" stroke="{colour}" stroke-width="1" {extra} points="{points}"/>'
        )

    busy = "".join(
        f'<rect x="{left + (width - left - 20) * t[a] / t[-1]:.1f}" y="{top}" '
        f'width="{max(1.0, (width - left - 20) * (t[b - 1] - t[a]) / t[-1]):.1f}" height="6" '
        f'fill="#e05050"/>'
        for a, b in spans(tr["busy"] > 0)
    )
    grid = "".join(
        f'<line x1="{left}" y1="{y(np.array(v)):.1f}" x2="{width - 20}" y2="{y(np.array(v)):.1f}" '
        f'stroke="#334" stroke-width="0.5"/><text x="{left - 6}" y="{y(np.array(v)) + 4:.1f}" '
        f'fill="#99a" font-size="11" text-anchor="end">{v:.0f}</text>'
        for v in np.arange(low, high + 1, 6)
    )
    svg = (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
        f'font-family="system-ui, sans-serif"><rect width="{width}" height="{height}" fill="#0f1117"/>'
        f'<text x="{left}" y="20" fill="#dde" font-size="13">{title}: level (green), floor (white), '
        f"gain reading from the floor's median (blue), busy (red bar), dBFS</text>"
        + grid
        + busy
        + polyline(gain_line, "#6aa0ff")
        + polyline(tr["power_db"], "#3fd9b4")
        + polyline(tr["floor_db"], "#f0f0f0")
        + "</svg>\n"
    )
    path.write_text(svg, encoding="utf-8", newline="\n")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("wav", type=Path)
    ap.add_argument("--expect", type=Path, help="the recording's sidecar (default: beside it)")
    ap.add_argument("--bandwidth", type=int, help="500 or 2300, when the sidecar does not say")
    ap.add_argument("--variant", help="a candidate of ADR-0078 instead of the shipped detector")
    ap.add_argument("--daemon", type=Path, default=daemon_default())
    ap.add_argument("--csv", type=Path, help="write the block-by-block trace")
    ap.add_argument("--svg", type=Path, help="draw it")
    ap.add_argument("--json", type=Path, help="write the events found")
    args = ap.parse_args()
    expect = args.expect
    beside = args.wav.with_suffix(".json")
    if expect is None and beside.exists():
        # a session sidecar, not any JSON (the corpus's labels sit beside its clips too)
        try:
            is_sidecar = str(json.loads(beside.read_text()).get("format", "")).startswith(
                "aether-hf-session"
            )
        except (ValueError, AttributeError):
            is_sidecar = False
        expect = beside if is_sidecar else None
    tr = run_trace(args.daemon, args.wav, expect, args.bandwidth, args.variant)
    t = tr["t_s"]
    hours = t[-1] / 3600
    busy = tr["busy"] > 0
    print(f"{args.wav.name}: {t[-1]:.0f} s, {len(t)} blocks of 25 ms")
    print(
        f"  busy {busy.mean() * 100:.1f} % (level {np.mean(busy & (tr['reason'] == 'level')) * 100:.1f} %, "
        f"shape {np.mean(busy & (tr['reason'] == 'shape')) * 100:.1f} %, "
        f"frame {np.mean(busy & (tr['reason'] == 'frame')) * 100:.1f} %)"
    )
    print(
        f"  evidence {np.mean(tr['evidence'] > 0) * 100:.0f} % of blocks, steady "
        f"{np.mean(tr['steady'] > 0) * 100:.0f} %; floor median {np.median(tr['floor_db']):.1f} dBFS"
    )
    found = events(tr)
    print(
        f"  AGC events (gain ≥ {EVENT_DB:.0f} dB under its level on quiet, ≥ {EVENT_MIN_S} s): "
        f"{len(found)}, {len(found) / max(hours, 1e-9):.1f} an hour"
    )
    for e in found:
        print(
            f"    {e['start_s']:8.1f} s  {e['length_s']:6.1f} s  {e['depth_db']:5.1f} dB deep, back at "
            f"{e['release_db_per_s']:5.1f} dB/s; cause {e['cause']} (channel {e['channel_rise_db']:+.1f}, "
            f"outside {e['outside_rise_db']:+.1f} dB); floor {e['floor_before_db']:.1f} → "
            f"{e['floor_lowest_db']:.1f} → {e['floor_after_db']:.1f}; busy after {e['busy_after_s']:.1f} s"
        )
    long_false = [
        (a, b) for a, b in spans(busy & (tr["reason"] == "level")) if (b - a) * BLOCK_S >= 20.0
    ]
    print(f"  busy by level for 20 s or more: {len(long_false)}")
    for a, b in long_false:
        print(
            f"    {t[a]:8.1f} s for {(b - a) * BLOCK_S:5.1f} s: level {np.median(tr['power_db'][a:b]):.1f}, "
            f"floor {tr['floor_db'][a]:.1f} → {tr['floor_db'][b - 1]:.1f}"
        )
    if expect is not None:
        for line in radio_report(json.loads(expect.read_text()), found):
            print(line)
    if args.json:
        args.json.write_text(json.dumps(found, indent=1))
    if args.csv:
        with args.csv.open("w", encoding="utf-8", newline="\n") as f:
            keys = list(tr)
            f.write(",".join(keys) + "\n")
            for i in range(len(t)):
                f.write(",".join(str(tr[k][i]) for k in keys) + "\n")
    if args.svg:
        write_svg(args.svg, tr, args.wav.name)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
