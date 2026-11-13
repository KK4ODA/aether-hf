import json, collections
import numpy as np
rows = json.load(open("sweep.json"))
g = collections.defaultdict(list)
for r in rows: g[(r["case"], r["cnr"], r["wave"])].append(r)
print(f"{'case':8s} {'CNR':>4s} {'waveform':20s} {'max kb/s':>8s} {'SNDR@opt':>8s} {'opt dB':>6s} {'90% window':>12s} {'w dB':>5s} {'@opt+6':>7s} {'@opt-10':>7s}")
for k, v in g.items():
    v.sort(key=lambda r: r["drive"])
    d = np.array([r["drive"] for r in v]); rt = np.array([r["rate"] for r in v]); sn = np.array([r["sndr"] for r in v])
    i = int(np.argmax(rt)); mx = rt[i]
    ok = d[rt >= 0.9 * mx]
    def at(x):
        j = int(np.argmin(abs(d - x))); return rt[j] / mx
    print(f"{k[0]:8s} {k[1]:4d} {k[2]:20s} {mx/1000:8.2f} {sn[i]:8.1f} {d[i]:6.1f} {ok.min():5.1f}..{ok.max():5.1f} {ok.max()-ok.min():5.1f} {at(d[i]+6):7.2f} {at(d[i]-10):7.2f}")
