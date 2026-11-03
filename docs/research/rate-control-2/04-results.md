# Gate 3: the candidates on the harness

*In progress (2026-11-03). The order is the author's (`03-candidates.md` §4, answer 4): D3 first,
then D1 against D2, then the dip and the climb. Each is judged on the criteria of
`03-candidates.md` §3 against today's pair (`01-evidence.md` §1, `bench/baselines/
rate_oracle_beta127.csv`).*

## 1. D3: a family refused

### 1.1 What was built

Bench only, off by default: `GrantConfig.family_refusal` in the model and the port, turned on in
a simulated daemon by `AETHER_BENCH_LINK=family_refusal` (`aetherd`'s `bench_link`; a daemon
with a radio never reads it) and in the oracle workflow by its `bench_link` input. With it:

1. **The echo.** Every OFDM rung gets the family's echo limit (`FAMILY_ECHO_LIMIT`, model and
   port), so above it both controllers and the start keep off OFDM with no new rule
   (`_echo_limits`). The floor's own limits (500 Hz rungs 3–4) stand.
2. **Outcomes.** C1 at the sender refuses the OFDM family while its choice is a floor rung and
   the last OFDM frames it sent delivered nothing (`DeliveryController.refuses`).
3. **Stuck blocks.** A block cut at a refused family comes down at its next transmission — whole
   to the most robust floor rung that fits it, or in pieces — instead of after
   `max_combines` copies and the wait for the combining copy (`_data_frames`).

Item 2 and the inheritance cap differ from the text of `03-candidates.md` §2.3; the reasons are
recorded there.

### 1.2 The family's echo limit: not set

`tools/bench_echo.py --harq` measured every floor rung and OFDM rungs 5, 6, 7 and 9 of the
500 Hz air on `bench_echo`'s 22 paths at 10 and 20 dB, 16 blocks a point, each block sent until
it decoded or four times (RV 0, 0, 2, 3) and combined, every copy through its own fade. Priced
per second of an exchange with the answers in the data's own family, **OFDM's best rung holds
level with the floor's best on a 7 ms echo**: ITU NVIS 43 against 44 bit/s at 10 dB and 57
against 42 at 20 dB; two rays 7 ms apart 48 against 52 and 51 against 42; 8 ms apart 52 against
40 and 69 against 40. The echo reading cannot tell the paths apart on this measure: OFDM loses
from high-latitude moderate (a reading of 0.0019) and wins again at 8 ms (0.0194), so
`--family-limits` sets no limit (`FAMILY_ECHO_LIMIT` stays empty).

The harness says otherwise. On the Trimode NVIS test at 8 dB (the oracle's per-rung rows of
draws 0–2), data pinned at OFDM rungs 5, 6 and 7 lost the send in two draws of three each, and at
rungs 10 and 12 in all three, every loss a link timeout; rung 2 delivered in all three. In one
rung-5 session the sender's 17 ordinary control frames
(its headers) were never detected at the other end, 21 of its 142 OFDM data copies decoded, and
none of the gateway's 30 OFDM replies decoded the other way. The floor's frames came through
(`01-evidence.md` §1: rung 2 delivered 40–45 bit/s in every draw). What the bench leaves out is
what makes the difference there: an OFDM transmission's header and answers in the ordinary
family, and copies that share the session's one fade. A limit drawn from it would be no limit,
which the harness contradicts. **Item 1 is not set from this bench;** if items 2–3 leave the
NVIS cost standing, the limit is measured on the harness itself: the oracle on two-ray paths
across the delays, each run's echo read from its sidecars.

### 1.3 On the harness

D3 as committed (`5d57f18`: items 2 and 3, no family limit), the oracle workflow with
`bench_link = family_refusal`, ten draws, every rung (runs of 2026-11-03).

*(pending)*
