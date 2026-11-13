"""Time for one RadioPost session (RP-3 §12's exchange, byte counts from RP-3 §11) over a
granted-slot half-duplex link, against the link's parameters. Estimates, not measurements."""
TURNS = [201, 530, 345, 1351, 38]          # RP-3 transmissions 1..5, bytes incl. framing
def session(rate_bps, gap_s, pre_s, call_s, ladder_s=0.0, think_s=0.5, id_s=0.0, ctl_s=0.12):
    t = 0.0
    t += call_s + ladder_s + gap_s            # call (+ level ladder)
    t += call_s + ladder_s + gap_s            # acceptance (+ its ladder)
    for i, b in enumerate(TURNS):             # master/slave alternate; each carries a header/answer
        t += pre_s + ctl_s + b * 8 * 1.08 / rate_bps + gap_s   # 8 % frame overhead (CRC, headers, guard)
        if i in (0, 2):                       # the gateway answers after HELLO and after REQUEST
            t += think_s if i == 0 else 0.2   # post-office round trip after HELLO (RP-3 §9)
    t += pre_s + ctl_s + gap_s                # the slave's last answer + DISC_ACK (DISC rode with turn 5)
    return t + id_s
data_s = lambda r: sum(TURNS) * 8 / r
rows = [
 ("Aether FM simplex, mic path, 64QAM 5/6 (10 kb/s)", dict(rate_bps=10000, gap_s=0.40, pre_s=0.08, call_s=0.35)),
 ("  same, first contact: 1.0 s level ladder each way", dict(rate_bps=10000, gap_s=0.40, pre_s=0.08, call_s=0.35, ladder_s=1.0)),
 ("  same, slow radios: 0.8 s turnaround", dict(rate_bps=10000, gap_s=0.80, pre_s=0.08, call_s=0.35)),
 ("Aether FM simplex, mic path, 16QAM 3/4 (6 kb/s)", dict(rate_bps=6000, gap_s=0.40, pre_s=0.08, call_s=0.35)),
 ("Aether FM data port, 256QAM 3/4 (24 kb/s)", dict(rate_bps=24000, gap_s=0.40, pre_s=0.06, call_s=0.30)),
 ("  hypothetical 2x PHY (48 kb/s), same turnaround", dict(rate_bps=48000, gap_s=0.40, pre_s=0.06, call_s=0.30)),
 ("Aether FM via repeater, every turn waits key-up (1.6 s)", dict(rate_bps=10000, gap_s=1.60, pre_s=0.08, call_s=0.35)),
 ("Aether FM via repeater, replies inside hang time (0.5 s)", dict(rate_bps=10000, gap_s=0.50, pre_s=0.08, call_s=0.35)),
 ("Aether HF 2300, good path (~1.2 kb/s, 1.0 s gap, 0.6 s preamble)", dict(rate_bps=1200, gap_s=1.0, pre_s=0.6, call_s=3.2, ctl_s=0.4)),
]
print(f"{'case':66s} {'session s':>9s} {'data s':>7s} {'share':>6s}")
for name, kw in rows:
    t = session(**kw); d = data_s(kw['rate_bps'])
    print(f"{name:66s} {t:9.1f} {d:7.1f} {100*d/t:5.0f}%")
print("\nadd for Morse IDs of both stations at 20 wpm (~2.5-3.5 s each): +5 to +7 s to every row")
# a large transfer: 50 kB one way, 30 s key cap, 6 data frames of ~1 s max per grant etc.
for name, r, gap in (("mic 10 kb/s", 10000, 0.4), ("data port 24 kb/s", 24000, 0.4), ("repeater 10 kb/s, 1.6 s", 10000, 1.6)):
    burst_s = 8.0; per = burst_s + 0.08 + 0.12 + gap + 0.12 + 0.08 + gap
    n = 50_000 * 8 * 1.08 / r / burst_s
    print(f"50 kB {name:24s}: {n*per:6.1f} s, goodput {50_000*8/(n*per)/1000:5.2f} kb/s ({100*burst_s/per:3.0f}% of PHY)")
