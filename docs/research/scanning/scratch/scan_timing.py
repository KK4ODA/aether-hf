"""A scanning gateway against the calls of `call_schedules.py`: how often a call is caught.

Timing only, no channel. The gateway visits its channels in turn, a dwell on each; a visit hears
nothing for SETTLE seconds after the radio moved (the card's input latency and the radio's own
settling, an estimate). A **part** is an H try's OFDM part or its floor part, or an S try. A
part is *heard* when the gateway is on the caller's channel, settled, at the part's start: a
part begun before the gateway got there is not counted (conservative: the floor's middle and end
sync blocks may still find it). A part heard holds the gateway to its end (the scan never leaves
a frame arriving) and decodes with probability `p` (1 on a good path; 0.5 stands for a weak one,
each part independently). **Linger**: after a part heard and not decoded, the gateway stays
LINGER more seconds for the next part's start.

The dwell and the linger are not chosen: they come from the calls. DWELL is the longest gap
between the starts of two tries before a call's pause, plus SETTLE; LINGER the longest gap of all
(the pause's), plus SETTLE. A call's tries for a scan of n channels (the rule): the fewest whose
tries before the pause start over at least (n - 1) dwells.

    python -m uv run python scan_timing.py         # after call_schedules.py; under a minute

Prints the tables of `../00-timing.md` and writes `scan_timing.csv`.
"""

import csv
import json
import random
import statistics
from pathlib import Path

HERE = Path(__file__).resolve().parent
SETTLE = 0.5
TRIALS = 6000


def parts(call: dict) -> list[tuple[float, float]]:
    out = []
    for kind, (a, b) in zip(call["plan"], call["tx"], strict=True):
        if kind == "H":
            out += [(a, a + call["ofdm"]), (a + call["ofdm"], b)]
        else:
            out.append((a, b))
    return out


def trial(call, n, dwell, jitter, p, linger, rng):
    """When the call was caught (the end of the part decoded), or None."""
    ps, end = parts(call), call["end"]
    caller = rng.randrange(n)
    t, ch = -rng.uniform(0, 2 * n * dwell) - 1.0, rng.randrange(n)
    while t <= end:
        leave = t + dwell * (1 + rng.uniform(-jitter, jitter))
        if ch == caller:
            heard_from = t + SETTLE
            for a, b in ps:
                if a < heard_from or a >= leave:
                    continue
                if rng.random() < p:
                    return b
                leave = max(leave, b + linger)
                heard_from = b
        t, ch = leave, (ch + 1) % n
    return None


def caught(calls, n, dwell, p, linger, jitter=0.0, seed=23, trials=TRIALS):
    rng = random.Random(seed)
    got = [trial(calls[k % len(calls)], n, dwell, jitter, p, linger, rng) for k in range(trials)]
    ok = [g for g in got if g is not None]
    return len(ok) / trials, (statistics.median(ok) if ok else None)


def main() -> None:
    calls = json.loads((HERE / "calls.json").read_text())
    wide = {k: v for k, v in calls.items() if k.startswith("2300/")}
    back, gaps = [], []
    for v in wide.values():
        for c in v:
            st = [a for a, _ in c["tx"]]
            g = [b - a for a, b in zip(st, st[1:])]
            gaps += g
            back += g[:-1]
    dwell, linger = max(back) + SETTLE, max(gaps) + SETTLE
    print(f"dwell {dwell:.2f} s (longest gap before the pause {max(back):.2f} s + {SETTLE})")
    print(f"linger {linger:.2f} s (longest gap {max(gaps):.2f} s + {SETTLE})")

    def of(plan, tries):
        return wide[f"2300/{tries}/{plan}"]

    def rule(plan, n):
        for tries in range(2, 16):
            st = [a for a, _ in of(plan, tries)[0]["tx"]]
            if st[-2] - st[0] >= (n - 1) * dwell:
                return tries
        return 15

    rows = []
    print("\nthe rule's tries, dwell as derived")
    print("plan channels cycle tries call_ends p1 p.5 p.5+linger median_s fewest>=.99 two_tries_p1")
    for plan in ("H", "S"):
        for n in (2, 3, 4, 5, 6, 8):
            t = rule(plan, n)
            a = caught(of(plan, t), n, dwell, 1.0, 0.0)
            b = caught(of(plan, t), n, dwell, 0.5, 0.0)
            c = caught(of(plan, t), n, dwell, 0.5, linger)
            two = caught(of(plan, 2), n, dwell, 1.0, 0.0)
            fewest = next(
                (u for u in range(2, 16) if caught(of(plan, u), n, dwell, 1.0, 0.0, trials=3000)[0]
                 >= 0.99),
                None,
            )
            ends = max(x["end"] for x in of(plan, t))
            row = [plan, n, round(n * dwell, 1), t, ends, a[0], b[0], c[0], a[1], fewest, two[0]]
            rows.append(["rule", *row])
            print(" ".join(f"{x:.3f}" if isinstance(x, float) and x < 1.001 else str(x) for x in row))

    print("\nother dwells, the rule's tries, p = 1 (plan H)")
    for n in (2, 3, 4, 6):
        t = rule("H", n)
        for d in (3.5, 6.5, dwell, 2 * dwell):
            for jit in (0.0, 0.25):
                r = caught(of("H", t), n, d, 1.0, 0.0, jitter=jit)
                rows.append(["dwell", "H", n, round(d, 2), t, jit, r[0], r[1]])
                print(n, t, round(d, 1), jit, f"{r[0]:.3f}")

    with (HERE / "scan_timing.csv").open("w", newline="") as f:
        csv.writer(f).writerows(rows)


if __name__ == "__main__":
    main()
