# Aether HF core

The shipped modem, in Rust (ADR-0001). The Python model in `../model/` is the specification;
these crates are the implementation, and the two are required to agree bit-for-bit.

| Crate | Status | What |
|---|---|---|
| `aether-fec` | done | TS 38.212 CRCs, LDPC (BG1/BG2) encode and layered decode, rate matching with incremental redundancy |
| `aether-phy-hf` | planned (P3-2) | OFDM, preamble, acquisition, channel estimation |
| `aether-link` | planned (P3-2) | ARQ engine and session state machine |
| `aether-hal`, `aether-api`, `aetherd` | planned (P3-3, P3-4) | audio, PTT, control plane, daemon |

```
cargo test                                   # unit tests + cross-validation against the model
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

## Staying bit-exact

`aether-fec/tests/model_vectors.rs` checks this crate against vectors the model generates
(`python tools/make_fec_vectors.py`, committed at `aether-fec/tests/data/fec_vectors.json`).
CI regenerates them and fails on any difference, so the model cannot change without the core
being re-checked against it.

**If a vector test fails, the core is wrong.** Regenerate the vectors only when the model
changed on purpose — and then say so in the commit.

The LDPC base-graph tables are not duplicated here: `aether-fec/build.rs` reads the model's
`nr_ldpc_base_graphs.json` (extracted from TS 38.212) and generates Rust constants, so the
shipped crate carries no JSON parser and both implementations provably use the same table.
