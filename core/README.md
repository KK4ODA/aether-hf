# Aether HF core

The shipped modem, in Rust (ADR-0001). The Python model in `../model/` is the specification;
these crates are the implementation, and the two are required to agree bit-for-bit.

| Crate | What |
|---|---|
| `aether-fec` | TS 38.212 CRCs, LDPC (BG1/BG2) encode and layered decode, rate matching with incremental redundancy |
| `aether-phy` | the waveforms of both airs (2 300 and 500 Hz): constellations, the ladders of rungs, the frame codec, OFDM, the preamble, peak reduction (ADR-0004), the tone frames — the tone floor, fast tones and the 500 Hz middle kinds (ADR-0013–0015) — the transmitter, acquisition, the receiver, the 48 kHz audio front end, the impulse blanker and the streaming receiver |
| `aether-link` | frame formats, rate control, the ARQ engine (sessions, HARQ, probes, datagrams, the regulatory ceiling) and a two-station simulator over a lossy pipe (the fading pipe of the link bench is the model's) |
| `aetherd` | the daemon: audio (cpal) and keying (serial, CAT, CM108, `rigctld`), the key watchdog, the busy detector, the regulatory gate (`data/regulatory/`, `data/occupancy.json`), the control API and the panel's files, the VARA-compatible host interface, the KISS port, profiles, recordings and replay, the simulated channel |

All four are done and in every release; `docs/ROADMAP.md` says what is next.

```
cargo test --release --workspace             # unit tests + cross-validation against the model
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Use `--release` for the tests. The acquisition search is real signal processing and an
unoptimised build runs it about twenty times slower without covering anything extra.

## Staying bit-exact

Every ported layer has a `tests/model_vectors.rs` fed by vectors the model generates
(`python tools/make_fec_vectors.py`, `make_phy_vectors.py` and `make_link_vectors.py`,
committed under each crate's `tests/data/`), and `aether-link/tests/protocol.rs` runs the
engine's protocol scenarios against the model's. CI regenerates the vectors and fails on any
difference, so the model cannot change without the core being re-checked against it.

**If a vector test fails, the core is wrong.** Regenerate the vectors only when the model
changed on purpose — and then say so in the commit.

Not everything can be bit-exact, and the tests say which is which. Integer-valued quantities
— the mode table, frame layouts, the interleaver permutation, CRC remainders, codewords,
rate-matched and interleaved bits — are compared for equality. Floating-point results (LLRs,
the OFDM and tone waveforms) are compared to a tolerance carried in the vector file, because
two correct implementations of the same arithmetic are not required to produce identical
doubles.

The LDPC base-graph tables are not duplicated here: `aether-fec/build.rs` reads the model's
`nr_ldpc_base_graphs.json` (extracted from TS 38.212) and generates Rust constants, so the
shipped crate carries no JSON parser and both implementations provably use the same table.
`aether-phy/build.rs` does the same for the preamble, chip and tone tables of both airs
(`python tools/make_phy_tables.py` exports them from the model).

`aetherd/tests/two_daemons.rs` runs two real daemons through sessions over the simulated
channel, and `aetherd/tests/field.rs` replays every recorded session in `field/sessions/`
through the receiver and fails if fewer frames decode than did on the day.
