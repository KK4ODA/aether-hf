"""Audit probe (2026-09-13): FEC encoder validity, decoder sanity, Gray labeling.
Run from repo root: python tools/audit_probe_fec_mapping.py
See docs/AUDIT.md for interpretation."""
import numpy as np, sys, time
sys.path.insert(0, '.')
from aether_model.fec.ldpc_5gnr import LDPC5GNR
from aether_model.dsp.modulation import CONSTELLATIONS, Mapper, Demapper
from aether_model.speed_levels import Modulation

print("== LDPC5GNR encoder validity (syndrome should be all-zero) ==")
for n, r in [(288, 0.5), (1152, 0.5), (1152, 0.25), (1152, 0.75), (4104, 0.5)]:
    c = LDPC5GNR(n, r)
    bad = 0
    for _ in range(20):
        info = np.random.randint(0, 2, c.k).astype(np.int8)
        cw = c.encode(info)
        s = (c.H @ cw) % 2
        bad += int(np.any(s))
    print(f"  n={c.n} k={c.k} m={c.m} Z={c.Z} rate={c.k/c.n:.3f}: invalid codewords {bad}/20; "
          f"nonzero syndrome rows in last try: {int(np.sum(s))}/{c.m}")

print("\n== ldpc.py (random H) encoder validity ==")
print("  (module deleted in Phase 0; at commit 6477100 it produced 20/20 invalid codewords)")

print("\n== LDPC5GNR: check-node degree distribution (rate 1/3, n=1152) ==")
c = LDPC5GNR(1152, 1/3)
deg = c.H.sum(axis=1)
vals, counts = np.unique(deg, return_counts=True)
print("  row weights:", dict(zip(vals.tolist(), counts.tolist())))
cdeg = c.H.sum(axis=0)
vals, counts = np.unique(cdeg, return_counts=True)
print("  col weights:", dict(zip(vals.tolist(), counts.tolist())))

print("\n== LDPC5GNR: rate 1/4 n=1152 ==")
c = LDPC5GNR(1152, 0.25)
deg = c.H.sum(axis=1)
vals, counts = np.unique(deg, return_counts=True)
print("  row weights:", dict(zip(vals.tolist(), counts.tolist())))

print("\n== Decoder: noiseless LLRs from a VALID codeword (all-zero) ==")
c = LDPC5GNR(288, 0.5)
llr = np.full(c.n, +8.0)
t=time.time(); dec, conv, it = c.decode(llr); dt=time.time()-t
print(f"  all-zero codeword: converged={conv} iters={it} time={dt*1000:.0f} ms")

print("\n== Decoder: noiseless LLRs from encoder output ==")
info = np.random.randint(0, 2, c.k).astype(np.int8)
cw = c.encode(info)
llr = 8.0 * (1 - 2*cw.astype(float))
t=time.time(); dec, conv, it = c.decode(llr); dt=time.time()-t
print(f"  encoded codeword, no noise: converged={conv} iters={it} info_ok={np.array_equal(dec, info)} time={dt*1000:.0f} ms")

print("\n== Gray-mapping check: 16-QAM neighbours should differ by 1 bit ==")
for m in [Modulation.QAM16, Modulation.QAM64, Modulation.QAM32, Modulation.QAM128]:
    C = CONSTELLATIONS[m]
    bps = int(np.log2(len(C)))
    dmin = min(abs(C[i]-C[j]) for i in range(len(C)) for j in range(i+1,len(C)))
    bad=0; tot=0
    for i in range(len(C)):
        for j in range(i+1,len(C)):
            if abs(abs(C[i]-C[j])-dmin) < 1e-6:
                tot+=1
                if bin(i^j).count('1')!=1: bad+=1
    print(f"  {m.value}: {len(C)} pts, dmin={dmin:.3f}, nearest-neighbour pairs with Hamming dist != 1: {bad}/{tot}")
