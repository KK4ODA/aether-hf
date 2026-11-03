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

### 1.2 The family's echo limit

*(pending: `tools/bench_echo.py --harq`, both airs.)*

### 1.3 On the harness

*(pending: the oracle workflow with `bench_link = family_refusal`, ten draws.)*
