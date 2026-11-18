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

## R4, judged (`../06-r4-results.md`)

| File | What it does |
|---|---|
| `r4.patch` | R4 against the tree (`git apply`): the look divided by its span median, each bin's 20th percentile over the looks, and the two tests. Not merged: it misses §7's Q1, Q2, and K2/K3 on draw 1 |
| `q_counts.py JUDGE_CSV` | Q1 and Q2 of `05-busy-band.md` §7 from `rxfilter_corpus.py judge --csv` |
| `criteria_table.py JUDGE_DIR DRAWS BUILDS` | `06-r4-results.md` §2's table, from each `<draw>-<build>.txt` and `.csv` the judge wrote |
| `q1_timing.py CORPUS JUDGE_CSV DAEMONS [WARM_S]` | Q1's clips traced again: whether each warned only in the first `WARM_S` (35) seconds of listening or after too |
| `warn_spans.py CORPUS DAEMONS CLIP@BW …` | Each clip's warning spans and its last reading, build by build |
| `harness_traces.py RUNS DAEMONS` | Every uncut station of a harness run: its live readings and each build's trace over its recording, as warnings |
| `offset_check.py` | `06-r4-results.md` §3.5's synthetic check: a block on in two looks of three, under R4's 20th percentile and the median |
| `r5.patch` | R5 against the tree (`git apply`, R4's patch included): each look the mean spectrum of its half second. Not merged: it misses more of §7 than R4 (`../08-r5-results.md`) |
| `rejudge_filter.py RUNS REPO` | `session_matrix.filter_warnings` (H1) read again over every run of a harness out dir |
