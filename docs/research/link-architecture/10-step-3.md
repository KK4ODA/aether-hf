# Step 3: the harness A/B, protocol 8 against protocol 7

ADR-0069 §5's third step: the scenario harness, both protocols, ten draws a scenario, judged on
criteria stated before the runs. [06-recommendation.md](06-recommendation.md) §4 states them.
§1 makes each one exact. It was committed, with the judge that applies it
(`research/link_arch/step3.py`, tested in `research/link_arch/tests/test_step3.py`), before
either arm ran. The runs and the verdict follow it.

## 1. Registered before the runs

### 1.1 The arms

* **Protocol 7**: the daemon built from `55d8ac1`, the last commit whose daemon runs protocol 7.
  That is beta.105's engine with ADR-0065…0067, reading configuration schema 11.
* **Protocol 8**: the daemon built from the commit that registers this, reading schema 12.
* **One harness for both**: `tools/session_matrix.py` and `tools/channel_server.py` at that same
  commit, including the VOX radio keyed on audio ([09-step-2.md](09-step-2.md) §3.2).
* **One configuration file per daemon**: each gets the file its own schema reads (`schema_of`).
  A scenario's `turnaround_ms` therefore reaches protocol 8 only. Protocol 7 has no such key.

### 1.2 The runs

* **Which scenarios**: every scenario in `bench/scenarios/*.toml`, 44 of them. The stress set
  (tag `stress`) is among them. `long/` is not.
* **Ten draws each**: variant *k* = 0…9 has seed *s* + 1000*k*, where *s* is the scenario's own
  seed (`--seeds 10`). Both arms use the same seeds.
* **The order**: variant by variant, protocol 7's pass of variant *k* and then protocol 8's, for
  *k* = 0…9. Each pass runs with `--jobs 3` on the same machine, with nothing else running.
  A passing run's audio is dropped (`--drop-passing-audio`). A failing run keeps its record.
* **Reruns**: a run whose runner failed (no `result.json` or `channel.json`) is run again once
  in its arm, and the report says so. Nothing else is run again.
* **The record**: `step3.py collect` folds each pass into one CSV line a run, in
  `research/link_arch/results/step3_p7.csv` and `step3_p8.csv`. These are committed after each
  pair of passes. `step3.py judge` gives the verdict.

### 1.3 The criteria

**C1: no radiated overlap, and no frame keyed into a receiver the protocol was told was
deaf.** This is judged on the protocol 8 arm.

* **(a) Radiated overlaps: zero.**
  * A radiated overlap is a stretch in which the channel server reports two transmitters
    radiating (`rf_overlaps`). It is put down to the transmission each station had on the air
    when it started.
  * Only one kind is left out: an overlap of two *initiations*. An initiation is a
    transmission a station starts on its own, outside any exchange with the other. These are a
    call's first try (`connect`) and a probe's (`probe`), a beacon, a datagram, drive bursts, a
    keying test, and an identifier sent alone. Carrier sense orders these, not a link protocol.
  * Anything else counts, whichever station keyed first: a retry, an answer, anything in a
    session.
* **(b) Deaf hits: zero seconds counted.**
  * The measure is the channel server's `deaf_hits_s`: the other station's signal that arrived
    while this station was keyed in its own silence, held by its VOX, or recovering. Arriving
    while it transmitted is (a).
  * A station's hits count when its radio keeps assumption S4 and not every transmission found
    arriving in its deafness was an initiation.
  * A radio keeps S4 when its deafness after its frames is no longer than its declared
    turnaround or `D + KEY` (0.65 s), whichever is longer. That deafness is the VOX hold plus
    the receiver's recovery, or, for a radio keyed by a line, the daemon's keyed tail (at most
    0.25 s) plus the recovery.
  * Hits into a radio that breaks S4 are reported. `80m-vox-caller-learned-gap-500` has one: an
    undeclared 0.9 s VOX hold.
* **Reported, not judged**: the VOX and guard seconds of the keyed-overlap split (06 §4), and
  the overlaps of two initiations.

**C2: completion at least protocol 7's, scenario by scenario.**

* For each scenario, protocol 8 completes at least as many of its ten runs as protocol 7.
* A run completes when every judgement of the runner holds except `[expect] max_collisions`.
  That count includes keyed silence ([09-step-2.md](09-step-2.md) §3.3), and C1 judges
  overlaps instead.
* The runner's invariants count among the judgements: no daemon exits or panics, no key is held
  over 32 s, and no host is told `CONNECTED` while not listening.

**C3: air time.**

* For each scenario, *r* is the median air time of protocol 8's ten runs over protocol 7's.
  Every run counts, completed or not.
* A class's figure is the geometric mean of its scenarios' *r*. It must be at most 1.15 in
  every class.
* The geometric mean over all 44 must be at most 1.10.
* The classes are disjoint. A scenario goes in the first that matches:

| class | rule | scenarios |
|---|---|---:|
| gateway | tagged `gateway` or `host` | 5 |
| robustness | tagged `robustness`, `machine` or `vox` | 8 |
| band | 20 m and 15 m | 7 |
| 500 Hz | the rest, at 500 Hz | 12 |
| 2300 Hz | the rest | 12 |

**C4: Test rates, protocol 8 at least 0.85 × protocol 7's, pooled.**

* For each scenario with a Test, the median message rate and the median file rate are taken
  over the runs that report one.
* A scenario counts where both arms report the rate in at least three runs.
* The geometric mean of the ratios, protocol 8 over protocol 7, must be at least 0.85, for the
  message and for the file separately.

### 1.4 If a criterion fails

The A/B stops (06 §4).

* **The explanation comes first.** The failure is read from its runs' records: both stations'
  recordings, the transmission tags and the channel's account of the air. It is named as a
  broken assumption (S1–S7) or rule (R1–R12) of [04-formal-spec.md](04-formal-spec.md), with a
  trace, before anything changes.
* **A new rule must justify itself.** It names the invariant it restores, and it moves no
  deadline on undecoded evidence.
* **Then the arm runs again in full.** Protocol 8's whole arm is run again on the changed build,
  not the failing scenario alone.

## 2. The runs

*To be written from the runs.*
