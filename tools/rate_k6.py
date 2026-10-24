"""The rate controller's review, criterion K6: a prototype against the current controller on
the field's two-sided sessions, open loop (``docs/research/rate-control/03-candidates.md`` §3).

    python -I tools/rate_k6.py --inbox field/inbox --candidate delivery

Both controllers are replayed on what each receiver recorded (``tools/rung_reading.py``'s
replay, which reproduces the current controller's recommendations exactly). At every step where
the session measured both controllers' choices — data or the Test's ladder sent at that rung in
that direction — each choice is worth what its rung's measured first copies say it delivers in
this session's exchanges (``rung_reading.rate_at`` on ``RungAccount.first_copy_share``, a frame
never detected counted lost: Gate 0's neighbour estimate, one measure for data and ladder rungs
alike); K6 passes when the candidate's sum is at least the current's and no session's is ten
per cent under. A step whose choice the session never measured is not judged, and the share
judged is reported: open loop, a candidate is only ever judged on what the session happened to
measure.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import rung_reading as rr


def judge(master: Path, slave: Path, candidate: str) -> list[tuple[str, int, int, float, float]]:
    """Per direction: (label, steps judged, steps, current's sum, candidate's sum)."""
    current = rr.read_session(master, slave, replay_controller=True)
    other = rr.read_session(master, slave, replay_controller=True, controller=candidate)
    out = []
    air = rr.air_for(rr.load_side(master).doc)
    for d_cur, d_cand in zip(current.directions, other.directions, strict=True):
        judged = 0
        cur_sum = cand_sum = 0.0
        overhead = rr.exchange_overhead_s(d_cur.txs)
        worth = {
            r: rr.rate_at(air, r, acc.first_copy_share, overhead)
            for r, acc in d_cur.accounts.items()
            if overhead is not None and acc.first_copy_share is not None and r < air.n_rungs
        }
        for s_cur, s_cand in zip(d_cur.steps, d_cand.steps, strict=True):
            a = worth.get(s_cur.rung_replayed)
            b = worth.get(s_cand.rung_replayed)
            if a is None or b is None:
                continue
            judged += 1
            cur_sum += a
            cand_sum += b
        out.append(
            (
                f"{current.recording} {d_cur.sender}->{d_cur.receiver}",
                judged,
                len(d_cur.steps),
                cur_sum,
                cand_sum,
            )
        )
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--inbox", type=Path, required=True)
    ap.add_argument("--candidate", required=True)
    args = ap.parse_args()
    found = rr.pairs(sorted(args.inbox.rglob("*.json")))
    total_cur = total_cand = 0.0
    worst = None
    for master, slave in found:
        rows = judge(master, slave, args.candidate)
        s_cur = sum(r[3] for r in rows)
        s_cand = sum(r[4] for r in rows)
        total_cur += s_cur
        total_cand += s_cand
        for label, judged, steps, cur, cand in rows:
            ratio = f"{cand / cur:.3f}" if cur > 0 else "—"
            print(
                f"{label}: {judged}/{steps} steps judged, current {cur:.0f}, "
                f"{args.candidate} {cand:.0f} ({ratio})"
            )
        if s_cur > 0:
            r = s_cand / s_cur
            worst = r if worst is None else min(worst, r)
    ok = total_cand >= total_cur and (worst is None or worst >= 0.9)
    print(
        f"\nK6: candidate {total_cand:.0f} against current {total_cur:.0f} "
        f"({total_cand / total_cur if total_cur else float('nan'):.3f}); worst session "
        f"{'—' if worst is None else f'{worst:.3f}'}: {'pass' if ok else 'fail'}"
    )
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
