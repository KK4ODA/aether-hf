"""§7's corpus criteria for each judged draw and build, from `rxfilter_corpus.py judge`'s
printout (K1-K6) and its --csv (Q1, Q2 by q_counts' rules).

    python3 criteria_table.py JUDGE_DIR DRAW[,DRAW] BUILD[,BUILD]
"""
import re
import subprocess
import sys
from pathlib import Path

here = Path(__file__).resolve().parent


def main():
    folder, draws, builds = Path(sys.argv[1]), sys.argv[2].split(","), sys.argv[3].split(",")
    print("| draw | build | K1 | K2 | K3 | K5 in time / cleared / warned | K6 | Q1 (of 84) | strong own (of 36, reported) | Q2 (of 24) | Q2 moderate extra (of 12) | strong extra (reported) |")
    print("|---|---|---|---|---|---|---|---|---|---|---|---|")
    for d in draws:
        for b in builds:
            text = (folder / f"{d}-{b}.txt").read_text()
            k = re.search(r"K1 (\d+) of (\d+).*?K2 (\d+) of (\d+).*?K3 (\d+) clips.*?K5 (\d+) of (\d+) in time, (\d+) of (\d+) joined clips cleared.*?(\d+) of (\d+) warned of a cut made.*?K6 C1 on (\d+) of (\d+) hot clips in time, on (\d+) others", text)
            g = k.groups()
            q = subprocess.run([sys.executable, str(here / "q_counts.py"), str(folder / f"{d}-{b}.csv")],
                               capture_output=True, text=True).stdout.splitlines()
            q1 = re.search(r"Q1 (\d+) of (\d+) warn; strong own rhythm (\d+) of (\d+)", q[0]).groups()
            q2 = re.search(r"Q2 (\d+) of (\d+) named.*?moderate (\d+) of (\d+) with.*?strong (\d+) with", q[1]).groups()
            print(f"| {d} | {b} | {g[0]} of {g[1]} | {g[2]} of {g[3]} | {g[4]} | {g[5]}/{g[6]} / {g[7]}/{g[8]} / {g[9]}/{g[10]} | {g[11]}/{g[12]}, {g[13]} others | {q1[0]} | {q1[2]} | {q2[0]} | {q2[2]} | {q2[4]} |")


main()
