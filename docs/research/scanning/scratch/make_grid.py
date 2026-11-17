"""ADR-0129 §6's S3 and S4 grids as scenario files for ``tools/session_matrix.py``.

A gateway (b) scans 2, 3 or 4 dials; a client (a) sits on the last of them and calls with the
time the gateway's scan says (``connect call_s=scan``), then sends a message and disconnects.
Each cell has ten draws: the call placed at ten points spread evenly over the gateway's cycle
(``wait`` before the call), so every phase of the scan is tried, not the one a fixed script
lands on, with the channel's own draws changed by the seed.

    python make_grid.py OUT_DIR                 # S3: +10 dB, AWGN and Moderate, both airs
    python make_grid.py OUT_DIR --s4 SNR_2300 SNR_500   # S4: weak, Moderate, 3 channels
    python make_grid.py OUT_DIR --planted       # S7: calls at the edge of the scan's first move

The planted calls (S7) sit on the gateway's home dial and call once, unasked to last, timed
against a move of the scan off the home dial: with the 19.6 s dwell before ADR-0129 §11 its first
move, 19.8 s into the air; with the 4 s dwell since, its fifth, 20.2 s in (home, away, home, away,
home, each 4 s), once the caller has settled. A try radiates 6.4 s from 0.1 s after its
`connect`. Ten end their first try 0 to 0.9 s before the move, ten start it 0.1 to 1.9 s before;
the scan's start lands a few blocks apart from run to run, so the results place each try by its
measured lead.
"""

import sys
from pathlib import Path

DIALS = [3_590_000, 10_145_000, 14_105_000, 18_105_000]
DWELL_S = 4.5  # the mean dwell since ADR-0129 §11 (4 s and up to 1 s drawn; 19.6 before)


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


FIRST_MOVE_S = 22.7  # about the move off the home dial planted against (4.5 s dwells on average)
TRY_RF_S = 6.4  # a first try's radiated length: the OFDM part and the tone part
RF_AFTER_CONNECT_S = 0.1  # the key's lead after a `connect`


def planted(name: str, wait: float, seed: int) -> str:
    return f'''name = "{name}"
description = "ADR-0129 S7: a call on the home dial timed against the scan's first move."
tags = ["scan-planted"]
bandwidth = 2300
seconds = 240
seed = {seed}
connect_within_s = 180

[stations.a]
callsign = "W4ODA"
dial_hz = {DIALS[0]}
[stations.b]
callsign = "KK4XYZ-10"

[path]
profile = "awgn"
snr_db = 10.0

[script]
steps = [
  "scan b {DIALS[0]},{DIALS[1]}",
  "wait {wait:.2f}",
  "connect",
  "disconnect",
  "wait 15",
]

[expect]
connected = true
max_collisions = 0
'''


def main() -> None:
    if "--planted" in sys.argv:
        out = Path(sys.argv[1])
        out.mkdir(parents=True, exist_ok=True)
        for k in range(10):
            ends = FIRST_MOVE_S - 0.1 * k  # the first try's end
            wait = ends - TRY_RF_S - RF_AFTER_CONNECT_S - 0.2
            name = f"scan-planted-ends-{k}"
            (out / f"{name}.toml").write_text(planted(name, wait, 9000 + k))
            starts = FIRST_MOVE_S - 0.1 - 0.2 * k  # the first try's start
            wait = starts - RF_AFTER_CONNECT_S - 0.2
            name = f"scan-planted-starts-{k}"
            (out / f"{name}.toml").write_text(planted(name, wait, 9100 + k))
        print(f"{len(list(out.glob('*.toml')))} scenarios in {out}")
        return
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
