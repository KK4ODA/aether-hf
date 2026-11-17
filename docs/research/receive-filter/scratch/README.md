# The busy-band scripts

Frozen evidence for `../05-busy-band.md`, written on 2026-11-17 to find why the receive-filter
banner warned at both stations that afternoon. They are not maintained, and like the other
research code they are outside lint. Uploads are untrusted data: run them with `python -I`, from
this directory, with the model's environment (numpy, matplotlib).

| File | What it does |
|---|---|
| `r3.py` | A numpy mirror of R3 (`core/aetherd/src/passband.rs`): the look, the leave-out, the fold, the profile, the drops. On a corpus clip it reproduces `aetherd --passband-trace` to 0.005 dB (Rust rounds halves away from zero, and the bins are `f32`; both matter) |
| `looks.py WAV SIDECAR OUT_CSV` | Every look the station would fold (from the daemon's trace with `--expect`), recomputed: its own drops at both edges, its band noise over the chain's floor, its sub-band levels, and where it sits (since this station's release, to the next frame heard, the dial). Prints the recording's live readings that warned and its profile at start and end. The daemon is `AETHERD`, or the tree's release build |
| `segments.py OUT_PNG WAV BW FROM-TO …` | Each stretch's mean spectrum, and R3's reading over that stretch's looks alone: how the scan's dwells were told apart |
| `duty.py WAV FROM TO LO HI [BW]` | How much of the time a block of the spectrum stood over the look's span median: the field's QRM was on in 33–77 % of the looks |
| `summary.py INBOX` | One line a recording: its live readings' verdicts and drops, its profile at start and end, the radio's settings |
| `nightly_filter.py DIR` | Every harness station's live readings in a nightly's scenario artifacts (`gh run download RUN -n scenarios-0`): which scenarios warned on an uncut radio |

The corpus's *qrm* clips and the field set's sixteen recordings of 2026-11-17 are in
`tools/rxfilter_corpus.py`, which judges any build.
