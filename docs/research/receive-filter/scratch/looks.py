"""Which looks put the receive filter's edge where it is.

    python -I looks.py WAV SIDECAR OUT_CSV

Runs aetherd --passband-trace WAV --expect SIDECAR for the looks the station would have folded
in, recomputes each with the R3 mirror, and writes one row a look: when it was taken, its own
drops at the two edges (the look alone, smoothed as the profile is), its band noise over the
chain's floor, whether it was kept, and where it sits: seconds since this station's last
release, seconds to the next frame it heard start (and that frame's SNR and kind), the dial.
It prints the live readings that warned (from the sidecar) and the profile at the start."""

import csv
import os
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import numpy as np  # noqa: E402

import r3  # noqa: E402

DAEMON = os.environ.get("AETHERD") or str(
    Path(__file__).resolve().parents[4] / "core" / "target" / "release" / "aetherd"
)
"""The daemon whose --passband-trace says which looks a station folds in: AETHERD, or the
tree's release build. On 2026-11-17 it was beta.161's (its reader is the tree's: passband.rs is
unchanged since beta.145)."""


def single_drops(look, span):
    return r3.span_drops(look, span)


def main():
    wav, sidecar_path, out_path = sys.argv[1:4]
    side = r3.load_sidecar(sidecar_path)
    bw = side.get("session", {}).get("bandwidth_hz") or 2300
    span = r3.SPANS[bw]
    trace = subprocess.run(
        [DAEMON, "--passband-trace", wav, "--expect", sidecar_path],
        capture_output=True, text=True, check=True,
    ).stdout
    rows = list(csv.DictReader(trace.splitlines()))
    audio = r3.read_wav(wav)

    releases = sorted(e["t_s"] for e in side["events"] if e["event"] == "ptt" and e["detail"] == "released")
    keys = sorted(e["t_s"] for e in side["events"] if e["event"] == "ptt" and e["detail"] == "keyed")
    frames = sorted(
        (f for f in side["frames"] if f.get("start_s") is not None), key=lambda f: f["start_s"]
    )
    dials = side.get("radio", {}).get("dial") or []

    def dial_at(t):
        d = None
        for tt, hz in dials:
            if tt <= t:
                d = hz
        return d

    looks = []
    out = []
    for row in rows:
        if not row["taken_s"]:
            continue
        t = float(row["taken_s"])
        spec = r3.spectrum_at(audio, t)
        look, over, why = r3.fold(spec, span)
        lo = hi = None
        if look is not None:
            looks.append(look)
            looks = looks[-r3.LOOKS_KEPT:]
            d = single_drops(look, span)
            if d:
                lo, hi = d
        since_release = t - max((r for r in releases if r <= t), default=float("nan"))
        since_key = t - max((k for k in keys if k <= t), default=float("nan"))
        nxt = next((f for f in frames if f["start_s"] >= t - 0.0853), None)
        prev = None
        for f in frames:
            if f["end_s"] is not None and f["end_s"] <= t:
                prev = f
        bands = {}
        floor_abs = r3.quantile(r3.between(spec, *r3.FLOOR_BAND), 0.5)
        bands["floor_dbfs"] = round(floor_abs, 1)
        bands["noise_dbfs"] = round(float(np.median(r3.between(spec, 1750, 2050))), 1)
        bands["q1300_dbfs"] = round(float(np.median(r3.between(spec, 1150, 1450))), 1)
        if look is not None:
            for a, b in ((400, 800), (800, 1200), (1200, 1600), (1600, 2000), (2000, 2400), (2400, 2640), (2700, 3200)):
                v = r3.between(look, a, b)
                v = v[np.isfinite(v)]
                bands[f"b{a}"] = round(float(np.median(v)), 1) if v.size else None
        out.append({
            "taken_s": round(t, 3),
            "why": why,
            "over_floor": None if over is None else round(over, 1),
            "look_low": None if lo is None else round(lo, 1),
            "look_high": None if hi is None else round(hi, 1),
            "trace_verdict": row["verdict"],
            "trace_low": row["low_drop_db"],
            "trace_high": row["high_drop_db"],
            "since_release": round(since_release, 3),
            "since_key": round(since_key, 3),
            "to_next_frame": None if nxt is None else round(nxt["start_s"] - t, 3),
            "next_snr": None if nxt is None else nxt.get("snr_3k_db"),
            "next_kind": None if nxt is None else f'{nxt["kind"]}/{nxt["mode"]}/{int(nxt["decoded"])}',
            "since_prev_end": None if prev is None else round(t - prev["end_s"], 3),
            "dial": dial_at(t),
            **bands,
        })
    with open(out_path, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(max(out, key=len).keys()), restval="")
        w.writeheader()
        w.writerows(out)
    print(f"{len(out)} looks folded, {sum(1 for o in out if o['why'] == 'kept')} kept; bandwidth {bw}")
    for name in ("start", "end"):
        prof = r3.profile_from_sidecar(side.get("passband", {}).get(name))
        if prof is not None:
            d = r3.span_drops(prof, span)
            print(f"profile at {name}: {side['passband'][name]['looks']} looks, drops low {d[0]:.1f} high {d[1]:.1f}")
    readings = side.get("passband", {}).get("readings", [])
    warned = [r for r in readings if r[2] in ("top", "bottom", "both")]
    print(f"live readings {len(readings)}, warning {len(warned)}")
    for r in warned[:3] + warned[-3:]:
        print("  ", r)


main()
