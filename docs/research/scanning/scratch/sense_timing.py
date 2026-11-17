"""Sense-and-stay scanning against Aether's call, with the detector's measured notices.

The rule (the author's, 2026-11-17): the gateway visits each dial for ``dwell`` s, the first
``DEAF_S`` s deaf after its move. If it hears a try from its start it stays and reads it (the hold
on an announced frame); if it notices a try already under way, it stays until the caller's next
try has begun and reads that one; on a quiet dial it moves on. A false notice on noise keeps it
``stay`` s on a dial nobody is calling on.

Inputs, from ``notice.py``'s CSVs:

* for each air, path and SNR, and each point of the try at which the gateway starts listening
  (``offset``, from the try's start, before the deafness), whether it noticed anything within the
  listening time a dwell gives, and whether a try it heard from the start decoded;
* noise: the false notices an hour.

The caller's tries start as the engine schedules them for a station not heard lately (0, 11.0,
22.0, 33.5, 45.1, 57.1, 69.1 s …). Measured, over random phases of the scan against the call and
over which dial the caller is on: the share of calls the gateway reads within a call of
``call_s``, and when.

    python -I sense_timing.py TRIES.csv NOISE.csv [--dwell 3.5] [--call 40]
"""

from __future__ import annotations

import csv
import random
import statistics as st
import sys
from collections import defaultdict

STARTS = [0.0, 11.01, 22.03, 33.54, 45.06, 57.07, 69.08, 81.1, 93.1]
"""When a not-heard caller's tries start (`GrantedEngine.call_offsets`, eight tries and more:
the pause before the last left out, the call made long enough)."""
DEAF_S = 0.5
OFFSETS_STEP = 0.4


def load(tries_csv: str, threshold: float):
    """{(air, profile, snr): {offset: [(noticed_at_s or None, decoded_from_start)]}}: per listen,
    when the first notice at or over ``threshold`` came (seconds after listening began), and
    whether the try decoded."""
    by: dict = defaultdict(lambda: defaultdict(dict))
    for r in csv.DictReader(open(tries_csv, encoding="utf-8")):
        key = (r["air"], r["profile"], r["snr"])
        cell = by[key][float(r["offset"])].setdefault(r["trial"], [None, False])
        if r["event"] == "none":
            continue
        conf = float(r["confidence"])
        at = float(r["at"])
        if r["event"] == "report" and r["ok"] == "ok":
            cell[1] = True
        if conf >= threshold and (cell[0] is None or at < cell[0]):
            cell[0] = at
    return {k: {o: list(v.values()) for o, v in d.items()} for k, d in by.items()}


def noise_rate(noise_csv: str, threshold: float) -> dict[str, float]:
    """False notices a second, per air, at or over ``threshold``."""
    count: dict[str, int] = defaultdict(int)
    seconds: dict[str, float] = defaultdict(float)
    chunks: dict[str, set] = defaultdict(set)
    for r in csv.DictReader(open(noise_csv, encoding="utf-8")):
        air = r["air"]
        if (r["offset"], r["trial"]) not in chunks[air]:
            chunks[air].add((r["offset"], r["trial"]))
            seconds[air] += float(r["offset"])
        if r["event"] != "none" and float(r["confidence"]) >= threshold:
            count[air] += 1
    return {a: count[a] / seconds[a] for a in seconds}


def visit(table, offset: float, listen: float, rng: random.Random):
    """One visit to the caller's dial that starts ``offset`` s after a try began: ('read',
    decoded) when it heard the try from its start, ('noticed', None) when it noticed a try under
    way, or None."""
    offs = sorted(table)
    o = min(offs, key=lambda x: abs(x - offset))
    noticed, decoded = rng.choice(table[o])
    if o < 0.0 and noticed is not None:
        return ("read", decoded)  # it listened from before the try's start: the hold reads it
    if noticed is not None and noticed <= listen:
        return ("noticed", None)
    return None


def one_call(table, decode_p: float, n: int, dwell: float, stay: float, false_p: float,
             call_s: float, rng: random.Random) -> float | None:
    """When the gateway reads one of the caller's tries (s from the call's first try), or None."""
    tries = [s for s in STARTS if s <= call_s]
    mine = rng.randrange(n)
    t = -rng.uniform(0, n * dwell) + rng.choice([0.0, 0.0, -30.0])  # any phase, any dial first
    k = rng.randrange(n)
    while t <= tries[-1] + 7.0:
        if k == mine:
            # the try on the air, or the last one that began, relative to this visit's start
            begun = [s for s in tries if s <= t + DEAF_S]
            ahead = [s for s in tries if t + DEAF_S < s <= t + dwell]
            if ahead:  # a try begins while the gateway listens: heard from its start
                if rng.random() < decode_p:
                    return ahead[0]
                # heard but not read: it stays for the next one (the trusted failed frame)
                nxt = [s for s in tries if s > ahead[0]]
                if nxt and nxt[0] - t <= stay + dwell and rng.random() < decode_p:
                    return nxt[0]
                t += dwell + stay
                k = (k + 1) % n
                continue
            if begun:
                got = visit(table, t - begun[-1], dwell - DEAF_S, rng)
                if got is not None:
                    nxt = [s for s in tries if s > begun[-1]]
                    if nxt and nxt[0] - t <= stay and rng.random() < decode_p:
                        return nxt[0]
                    t += stay
                    k = (k + 1) % n
                    continue
        elif rng.random() < false_p:
            t += stay  # noise taken for a try: it stays on a dead dial
        t += dwell
        k = (k + 1) % n
    return None


def main() -> None:
    tries_csv, noise_csv = sys.argv[1], sys.argv[2]
    dwell = float(sys.argv[sys.argv.index("--dwell") + 1]) if "--dwell" in sys.argv else 3.5
    call_s = float(sys.argv[sys.argv.index("--call") + 1]) if "--call" in sys.argv else 40.0
    threshold = float(sys.argv[sys.argv.index("--threshold") + 1]) if "--threshold" in sys.argv else 1.0
    stay = 13.0 + 5.7 + DEAF_S
    tables = load(tries_csv, threshold)
    noise = noise_rate(noise_csv, threshold)
    print(f"dwell {dwell} s, stay {stay:.1f} s, a call of {call_s:.0f} s, notices >= {threshold}")
    print("false notices an hour:", {a: round(3600 * v, 1) for a, v in noise.items()})
    for key in sorted(tables, key=lambda k: (k[0], k[1], -float(k[2]))):
        air, profile, snr = key
        table = tables[key]
        heard = [d for o, v in table.items() if o < 0 for _, d in v]
        decode_p = sum(heard) / len(heard) if heard else 0.0
        false_p = noise.get(air, 0.0) * (dwell - DEAF_S)
        row = []
        for n in (2, 3, 4, 5, 6, 8):
            rng = random.Random(3)
            got = [one_call(table, decode_p, n, dwell, stay, false_p, call_s, rng) for _ in range(3000)]
            ok = sorted(g for g in got if g is not None)
            med = st.median(ok) if ok else float("nan")
            row.append(f"{n}:{len(ok) / len(got):4.0%} {med:4.0f}s")
        print(f"{air:>4} {profile:8} {snr:>6} read {decode_p:4.0%} | " + " | ".join(row))


if __name__ == "__main__":
    main()
