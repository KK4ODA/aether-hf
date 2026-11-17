"""ADR-0129 §6's S3 and S4 grids as scenario files for ``tools/session_matrix.py``.

A gateway (b) scans 2, 3 or 4 dials; a client (a) sits on the last of them and calls with the
time the gateway's scan says (``connect call_s=scan``), then sends a message and disconnects.
Each cell has ten draws: the call placed at ten points spread evenly over the gateway's cycle
(``wait`` before the call), so every phase of the scan is tried, not the one a fixed script
lands on, with the channel's own draws changed by the seed.

    python make_grid.py OUT_DIR                 # S3: +10 dB, AWGN and Moderate, both airs
    python make_grid.py OUT_DIR --s4 SNR_2300 SNR_500   # S4: weak, Moderate, 3 channels
"""

import sys
from pathlib import Path

DIALS = [3_590_000, 10_145_000, 14_105_000, 18_105_000]
DWELL_S = 19.6  # the dwell `scan.start` answers on the harness's stations (a turnaround, the deferral)


def scenario(
    name: str, bw: int, profile: str, snr: float, n: int, k: int, seed: int, weak: bool
) -> str:
    dials = DIALS[:n]
    wait = round(2.0 + k * n * DWELL_S / 10, 1)
    # S4 judges the call alone: a kilobyte at the floor's edge would outlast any budget
    carry = '' if weak else '\n  "message 1000",'
    expect = "connected = true" if weak else "connected = true\ndelivered = true"
    return f'''name = "{name}"
description = "ADR-0129 grid: a gateway scanning {n} dials, called from the last at phase {k} of 10."
tags = ["scan-grid"]
bandwidth = {bw}
seconds = 600
seed = {seed}
connect_within_s = 240

[stations.a]
callsign = "W4ODA"
dial_hz = {dials[-1]}
[stations.b]
callsign = "KK4XYZ-10"

[path]
profile = "{profile}"
snr_db = {snr}

[script]
steps = [
  "scan b {",".join(str(d) for d in dials)}",
  "wait {wait}",
  "connect call_s=scan",{carry}
  "disconnect",
  "wait 15",
]

[expect]
{expect}
max_collisions = 0
'''


def main() -> None:
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    weak = "--s4" in sys.argv
    if weak:
        snrs = {2300: float(sys.argv[-2]), 500: float(sys.argv[-1])}
        cells = [(bw, "moderate", snrs[bw], 3) for bw in (2300, 500)]
    else:
        cells = [(bw, p, 10.0, n) for bw in (2300, 500) for p in ("awgn", "moderate") for n in (2, 3, 4)]
    for bw, profile, snr, n in cells:
        for k in range(10):
            name = f"scan-{'weak-' if weak else ''}{profile}-{bw}-{n}ch-p{k}"
            seed = 100 * n + k + (0 if bw == 2300 else 50) + (5000 if weak else 0)
            (out / f"{name}.toml").write_text(scenario(name, bw, profile, snr, n, k, seed, weak))
    print(f"{len(list(out.glob('*.toml')))} scenarios in {out}")


if __name__ == "__main__":
    main()
