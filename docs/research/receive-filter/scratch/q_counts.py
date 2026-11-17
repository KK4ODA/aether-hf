"""Q1 and Q2 of 05-busy-band.md §7 from a judge CSV (rxfilter_corpus.py judge --csv)."""
import csv
import sys

SILENT_DB = 10.0


def main(path: str) -> None:
    rows = [r for r in csv.DictReader(open(path, encoding="utf-8")) if r["kind"] == "qrm"]

    def parts(r):
        p = r["clip"].split("-")
        strength = next(x for x in p if x in ("moderate", "strong"))
        duty = next(x for x in p if x in ("own", "half"))
        return strength, duty

    silent = [
        r for r in rows
        if not r["need"] and float(r["loss_low"]) <= SILENT_DB and float(r["loss_high"]) <= SILENT_DB
    ]
    judged = [r for r in silent if parts(r) != ("strong", "own")]
    reported = [r for r in silent if parts(r) == ("strong", "own")]
    warn = lambda r: int(r["false_warnings"] or 0) > 0
    q1 = [r for r in judged if warn(r)]
    rep = [r for r in reported if warn(r)]
    cut = [r for r in rows if r["need"]]
    named = [r for r in cut if r["first_right_s"] != "" and r["right_at_end"] == "1"]
    lenient = [r for r in cut if set(r["need"].split("+")) <= {"top": {"top"}, "bottom": {"bottom"}, "both": {"top", "bottom"}}.get(r["final"], set())]
    mod_cut = [r for r in cut if parts(r)[0] == "moderate"]
    mod_extra = [r for r in mod_cut if int(r["wrong_side"] or 0) > 0]
    strong_extra = [r for r in cut if parts(r)[0] == "strong" and int(r["wrong_side"] or 0) > 0]
    print(f"Q1 {len(q1)} of {len(judged)} warn; strong own rhythm {len(rep)} of {len(reported)} warn (reported)")
    print(f"Q2 {len(named)} of {len(cut)} named right by the end (its cut side named at the end: {len(lenient)});"
          f" moderate {len(mod_extra)} of {len(mod_cut)} with an extra side; strong {len(strong_extra)} with an extra side (reported)")
    for r in sorted(q1, key=lambda r: (r["clip"], r["bandwidth"])):
        print(f"   Q1 warn {r['clip']} at {r['bandwidth']}: {r['false_warnings']} of {r['readings']}, ended {r['final']} {r['low_drop']}/{r['high_drop']}")
    for r in sorted(rep, key=lambda r: (r["clip"], r["bandwidth"])):
        print(f"   reported {r['clip']} at {r['bandwidth']}: {r['false_warnings']} of {r['readings']}, ended {r['final']} {r['low_drop']}/{r['high_drop']}")
    for r in sorted(cut, key=lambda r: (r["clip"], r["bandwidth"])):
        if r not in named or int(r["wrong_side"] or 0):
            print(f"   Q2 {r['clip']} at {r['bandwidth']} need {r['need']}: ended {r['final']} {r['low_drop']}/{r['high_drop']}, wrong-side readings {r['wrong_side']}")


if __name__ == "__main__":
    main(sys.argv[1])
