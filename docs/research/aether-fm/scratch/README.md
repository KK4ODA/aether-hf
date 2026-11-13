# The Aether FM proposal's scratch study

Frozen evidence for `../00-proposal.md` and `../01-measurements.md`, written on 2026-11-13 before
any design was agreed. It is not the model: Phase 2a writes the FM channel model properly in
`model/aether_model/`, model first, with tests, and these scripts are not maintained. Like the
other research code they are outside lint (`pyproject.toml`'s `extend-exclude` names `research`).

| File | What it measures |
|---|---|
| `fmchain.py` | The waveforms (CP-OFDM, DFT-spread OFDM, single carrier, 4-CPFSK, Aether HF's own frames) and a voice-path or flat-port FM radio: emphasis, limiter, splatter filter, IF noise and filter, discriminator, receive filters; SNDR and the information rate from coherence |
| `run_papr.py` | Peak-to-average of each waveform as real audio |
| `run_sweep.py` | Every waveform through the radio, 20 drive levels by 4 CNRs; writes `sweep.json` |
| `summarize.py` | Best drive, peak rate and the 90 % drive window from `sweep.json` |
| `distortion_probe.py` | Whether one transmission tells clipping from noise (it does not, reliably) |
| `slope_probe.py` | The two-level probe: ΔSNDR at the drive ±1.5 dB, 40 trials a point |
| `fde_gap.py` | A linear MMSE equaliser against per-frequency coding, over the flat band |
| `session_time.py` | RP-3 §12's session over a granted-slot link, and large transfers |

Run them from this directory with the model's environment, for example:

```
python -m uv run python run_papr.py
python -m uv run python run_sweep.py     # about two minutes; then summarize.py
```

`sweep.json` is the run behind the proposal's tables, kept so they can be checked without rerunning.
