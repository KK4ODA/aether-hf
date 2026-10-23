# Gate 0: the instruments, and what the field record says

The brief (`00-brief.md` §4) asks for the instruments before any design: a per-session reading
from both stations' recordings, the current controller replayed on what a receiver recorded, an
oracle on the harness, and the field record in hand. This document reports what was built, runs
it on every two-sided session, and ends with what the eleven adjustments rested on.

Nothing in the controller changed. Field builds hold still for rate changes until the decision
ADR is accepted (the author, 2026-10-23); fixes outside the controller may ship.

## 0. Summary

* **The reading exists.** `tools/rung_reading.py` turns both sidecars of a session into each
  rung's account: frames sent, never detected, failed, decoded alone, delivered by combining,
  the air each rung spent, the data it delivered per second of it, the receiver's
  recommendations over time, the ladder's measurements, and what the rungs either side would have
  delivered where they were measured (§1.1, §3).
* **The controller replays exactly.** Fed what the receiver recorded, as the engine feeds it,
  the build's own controller reproduces every one of the 203 recommendations the slaves sent in
  the nine sessions, and every one the masters sent (§4). A candidate controller can now be
  replayed open-loop on the field record.
* **The oracle exists.** `rate.pin` fixes a station's rung for the benches, the harness pins
  with `pin_rung` or `--pin`, and `tools/rung_oracle.py` runs a scenario at each fixed rung and
  reports the controller's regret against the best in hindsight (§7). On its first pass (14
  scenarios, one draw, a coarse set of rungs) a fixed rung in hindsight beat the controller in
  eleven: by +110 % on a strong Moderate 500 Hz path, +107 % on 15 m flutter, +73 % on 3 kB over
  AWGN; within 4 % on the air's 2300 Hz path; and the controller beat every fixed rung on the two
  weakest 500 Hz Tests.
* **The field record** is nine two-sided sessions (2026-10-14 to 2026-10-23, 500 Hz but one,
  80 m, three testers) and the one-sided ladders of thirteen more (§2).
* **What the sessions show** (§5): the climb stops below what the ladder decodes (three
  sessions, 15–77 % left at the top); the first OFDM rungs were chosen where the floor's top
  carried more (four sessions, up to 4× left); the learned margin walked a strong session down
  from rung 12 to 5 at steady readings; and a frame never detected counted for nothing (a third
  of rung 7's frames on 2300 Hz). Two sessions were on the best rung measured.
* **The eleven adjustments** (§6): four rest on the fading pipe alone, three on the pipe with a
  real-modem point, three on a field session and the harness, one on the harness and the air.
  None was judged on both sides of an on-air session, because no instrument read both sides.

## 1. What was built

### 1.1 The reading: `tools/rung_reading.py`

```
python tools/rung_reading.py MASTER.json SLAVE.json [--replay] [--timeline] [--markdown]
                             [--tree <worktree of the build>] [--json OUT] [--csv OUT]
python tools/rung_reading.py --inbox field/inbox [--replay] [--csv OUT]
```

* **One clock.** Both recordings are placed by the loop latencies (`tools/two_sides.py
  --from-loops`); a transmission the other station recorded nothing of is placed where its key
  puts it.
* **Every frame sent, matched.** The sender's `sent` records are laid out on the air from each
  keying, and each is matched to the receiver's record within the engine's own grid tolerance
  (0.15 s), or to none: *never detected*.
* **Sequence numbers.** A sidecar records no sequence numbers and no combining. They are
  reconstructed from the engine's own sending order (the unacknowledged frames first, the oldest
  first, those whose answer went unheard held back through one silent slot; then new ones), the
  redundancy versions 0, 0, 2, 3, a re-encode starting the count over at a lower rung, and the
  acknowledgements the sender decoded (`base`, `bitmap`). A block acknowledged after a copy that
  did not decode on its own was *delivered by combining*. A block the receiver decoded and the
  next acknowledgement does not cover is flagged; none is, in the nine sessions.
* **Measurement frames** (ADR-0077) are told by the header the receiver decoded, or by the
  master's ladder naming a transmission of that rung and size whose answer reported; they take
  no sequence number.
* **Per rung** (one table a direction): sends, first copies, never detected, failed, decoded
  alone, blocks delivered by a first copy / a later copy / combining, lost, data bytes delivered,
  the frames' air and the whole exchanges' air at that rung, and bit/s of each; the median SNR
  read; the ladder's frames at that rung, what the slave decoded of what it detected, and what
  the master recorded of them.
* **The neighbours and the regret.** A rung's *first-copy share* is what decoded on its own of
  its first copies and measurement frames, a frame never detected counted as lost. A rung's
  counterfactual rate is six frames' data at that share over the session's own exchange overhead
  (the median of exchange air less frame air) plus six frames. The regret is the best measured
  neighbour's rate over the rung's own, less one.
* **Old sidecars.** A `/4` sidecar numbers the 500 Hz ladder as it was before ADR-0081 (rung 4
  = QPSK ⅓ on the ordinary frame, 1.05 s); the tool reads it with that ladder.
* **`--replay`** (§1.2). **`--timeline`** lists every transmission with each frame's fate and the
  control frame that answered it. **`--tree`** reads with the model of another checkout.
* Tests: `model/tests/test_rung_reading.py` (a synthetic session with a frame never found, one
  failed, one delivered by combining, a measurement whose header was read and one whose header
  was missed, the regret arithmetic, the replay against the controller fed by hand, the
  pairing of sidecars).

### 1.2 The controller, replayed

With `--replay` the tool builds the controller the engine builds for the air (through
`GrantedEngine` itself, so a tree that still had the 2300 Hz cap replays with it), seeds it from
the connect frame the station decoded (the call at the slave, the acceptance at the master; a
floor frame as a lower bound), feeds every decoded frame's echo reading where the controller
takes one (ADR-0087), and at each transmission the station answered or forfeited feeds what
`granted._observe` feeds: frames decoded (alone, or delivered by this copy's combining) as `ok`;
detected, trusted frames at RV 0 or 3 or combined as failures; the SNR over the decoded and the
trusted; the rung most of the transmission was read at; the sender's-choice rule by the slowest
rung read against what the station had asked for. It then compares each recommendation with the
one the station's control frame carried, as the other station decoded it.

Four things had to be right for the replay to match, each found on a session that did not:

1. the controller of the build that ran, built as its engine builds it (the beta.120 session
   needed the cap ADR-0093 removed the day after);
2. a re-encode starts the redundancy versions over (`_TxRecord.tx_count = 0`), and the
   re-encoded block's family is the transmission's — without this the sequence numbers drifted
   through the old pinned ladders;
3. a transmission whose header was not read: the engine takes any data frame found since its
   last answer and before the first it decoded as the transmission's, so a phantom there is a
   counted failure (a trusted noise detection at rung 11 stepped ND1J's morning session 6 → 4);
4. the sender's-choice rule compares the *slowest* rung read in the transmission with what the
   station asked for, not the most common: a transmission of four stranded rung-10 copies and two
   new rung-6 frames is judged, and steps the link down.

### 1.3 The oracle: `rate.pin`, `pin_rung`, `tools/rung_oracle.py`

* **`rate.pin {rung}`** (control API, `docs/spec/control-api.md` §4.2): a bench-only method.
  The station's own new data goes at that rung whatever the other station recommends
  (`Station::pin_rung` → the engine's `pin_mode`). The recommendation still arrives and still
  decides what a stranded frame is re-encoded at; a Test's ladder measures its own rungs as
  before. Nothing on the panel sets it; the daemon forgets it at restart.
* **The harness**: `pin_rung = N` on a station in a scenario, or `--pin N` for every station,
  applied after the daemons come up; the result row carries `pin`. Checked on
  `quick-awgn-2300` at rung 7: both stations sent every data frame at rung 7.
* **`tools/rung_oracle.py`**: each scenario run as it is (`auto`) and at each rung of `--rungs`
  (rungs the air has not are skipped); a run with its `result.json` is not run again, so a set
  continues after a stop. The measure is what a run delivered: a `message` step's rate (0 when
  lost), else a Test's file rate, else its message rate. The summary gives, per scenario and
  seed, the controller's rate, the best fixed rung, its rate, and the regret.

### 1.4 The field record, fetched

`tools/fetch_uploads.py` brought both sides of every protocol-8 session into `field/inbox/`
(git-ignored, read with `python -I`): 17 uploads, 21 recordings, nine sessions with both sides
(§2). The daemon's log, the diagnostic bundle and the audio came with each.

## 2. The field record in hand

### 2.1 Both sides

| session (master's recording) | master → slave | Hz | build | report | message | file |
|---|---|---|---|---|---|---|
| 2026-10-14 23:59Z `20261014-235902_WC4Y_KK4ODA-1_test` | WC4Y → KK4ODA-1 | 500 | beta.106 | 11 | 512 B, 47 bit/s | 512 B, 52 bit/s |
| 2026-10-15 02:37Z `20261015-023718_ND1J_KK4ODA-1_test` | ND1J → KK4ODA-1 | 500 | beta.106 | 12 (night) | 2048 B, 404 bit/s | 5888 B, 225 bit/s |
| 2026-10-15 15:29Z `20261015-152940_ND1J_KK4ODA-1_test` | ND1J → KK4ODA-1 | 500 | beta.108 | 12 (morning 1) | 512 B, 58 bit/s | 768 B, 55 bit/s |
| 2026-10-15 15:42Z `20261015-154212_ND1J_KK4ODA-1_test` | ND1J → KK4ODA-1 | 500 | beta.108 | 12 (morning 2) | 512 B, 40 bit/s | 512 B, 32 bit/s |
| 2026-10-17 02:05Z `20261017-020502_WC4Y_KK4ODA-1_test` | WC4Y → KK4ODA-1 | 500 | beta.111 | 13 | 1024 B, 72 bit/s | 1024 B, 74 bit/s |
| 2026-10-22 00:29Z `20261022-002910_WC4Y_KK4ODA-1_test` | WC4Y → KK4ODA-1 | 500 | beta.118 | 14 | 1024 B, 74 bit/s | 1024 B, 104 bit/s |
| 2026-10-23 00:52Z `20261023-005231_WC4Y_KK4ODA-1_test` | WC4Y → KK4ODA-1 | 2300 | beta.120 | 15 | 512 B, 55 bit/s | 768 B, 61 bit/s |
| 2026-10-23 16:04Z `20261023-160400_ND1J_KK4ODA-1_test` | ND1J → KK4ODA-1 | 500 | beta.121 | 16 (Test 1) | 512 B, 204 bit/s | 2816 B, 413 bit/s |
| 2026-10-23 16:07Z `20261023-160704_ND1J_KK4ODA-1_test` | ND1J → KK4ODA-1 | 500 | beta.121 | 16 (Test 2) | 2048 B, 514 bit/s | 7680 B, 579 bit/s |

All nine are Tests on 80 m with KK4ODA-1 the slave, so the data read here is the master's; the
slave sent answers only. The five on beta.106–111 ran the old Test (the ladder as pinned data
frames in the middle, before ADR-0074 and ADR-0077), the four on beta.118–121 the current one
(measurement frames, the ladder last). The 2300 Hz session is the only one on that air. KQ4SAX's
uploads hold no session with KK4ODA-1 (another band, report 16 §6). Sessions before protocol 8
(2026-10-11 and earlier) are not read: another link protocol, and not what a candidate would
run on.

### 2.2 One side: the ladders

`field/ladders.csv` holds every Test's ladder as the master recorded it, thirteen sessions on
the air (2026-09-25 to 2026-10-23). Per rung, what the slave reported decoding, with the SNR it
reported; the 500 Hz rows before 2026-10-20 number the old ladder (rung 4 = QPSK ⅓):

| Hz | rung | ladders | frames | decoded | SNR reported, median (range) |
|---|---|---|---|---|---|
| 500 | 0 | 3 | 12 | 12 | −4 (−6 … −1) |
| 500 | 1 | 4 | 16 | 16 | −3 (−6 … −1) |
| 500 | 2 | 7 | 28 | 27 | −5 (−6 … −4) |
| 500 | 3 | 8 | 32 | 28 | −4 (−9 … −2) |
| 500 | 4 | 9 | 36 | 29 | −3 (−7 … +1) |
| 500 | 5 | 9 | 36 | 21 | −3 (−13 … +1) |
| 500 | 6 | 8 | 32 | 13 | −3 (−14 … 0) |
| 500 | 7 | 8 | 32 | 7 | 0 (−10 … +1) |
| 500 | 8 | 5 | 20 | 5 | −4 (−4 … +2) |
| 500 | 9 | 6 | 24 | 2 | −2 (−5 … 0) |
| 500 | 10 | 3 | 12 | 4 | −3 (−6 … +10) |
| 500 | 11 | 4 | 16 | 12 | +10 (−7 … +12) |
| 500 | 12 | 4 | 16 | 12 | +10 (−6 … +13) |
| 500 | 13 | 3 | 12 | 12 | +10 (+8 … +11) |
| 500 | 14 | 3 | 12 | 3 | +10 (+8 … +10) |
| 2300 | 5, 7, 8, 9–12 | 1 each | 4 each | 4, 0, 3, 0 | −1, −2, +1, −1 … 0 |

The record is thin above rung 8 and on 2300 Hz, and the 500 Hz rows below rung 10 are one
path (WC4Y's and ND1J's 80 m NVIS at −6 … +1 dB) read many times. It says what report 16
said: where the readings reached +8 … +13 dB, rungs 11–13 decoded nearly everything and rung 14
a quarter.

## 3. Every session's reading

The tables are the tool's (`--markdown`), the master's data read at KK4ODA-1. Columns: sends
(every copy), 1st (first copies), never (never detected), failed (detected, not decoded alone),
alone (decoded on their own), d: blocks delivered by a first copy / a later copy / combining,
lost, bytes delivered, the frames' air and bit/s over it, the exchanges' air and bit/s over it,
the median SNR read, and the ladder's frames at the rung: sent, what the slave decoded of what
it detected, what the master recorded (with the SNR it recorded). The *recommended* line is the
slave's recommendation in each answer the master read (the master's clock, rung @ the slave's
reading). The *rung* lines are the regret: what the data at that rung delivered per second of
exchange air against what the neighbours measured in the same session would have.

### 3.1 2026-10-14 23:59Z, WC4Y → KK4ODA-1, 500 Hz, beta.106 (report 11)

29 of 37 transmissions answered with a control frame WC4Y read.

| rung | frame | sends | 1st | never | failed | alone | d:1st | d:later | d:comb | lost | bytes | frame s | bit/s | exch s | bit/s | SNR | meas | slave | master |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | tone-36 | 5 | 4 | 1 | 0 | 4 | 3 | 1 | 0 | 0 | 132 | 26.8 | 39 | 29.4 | 36 | −4.6 | | | |
| 2 | tone4x100-51 | 4 | 4 | 0 | 0 | 4 | 4 | 0 | 0 | 0 | 192 | 21.4 | 72 | 26.2 | 59 | −4.3 | | | 4@−5 |
| 3 | tone4x100-75 | 19 | 16 | 0 | 1 | 18 | 15 | 3 | 0 | 0 | 1296 | 101.8 | 102 | 143.7 | 72 | −3.9 | | | 4@−4 |
| 4 | QPSK ⅓ | 9 | 4 | 3 | 1 | 5 | 1 | 4 | 0 | 0 | 60 | 9.5 | 51 | 35.7 | 13 | −2.2 | | | 1@−2 |
| 5 | QPSK ½ | 8 | 6 | 2 | 0 | 6 | 4 | 2 | 0 | 0 | 132 | 8.4 | 125 | 14.6 | 72 | +0.3 | | | 3@1 |
| 6 | QPSK ⅔ | 9 | 6 | 1 | 2 | 6 | 3 | 3 | 0 | 0 | 186 | 9.5 | 157 | 16.7 | 89 | +0.1 | | | 1@−1 |
| 7 | 8PSK ½ | 6 | 4 | 0 | 4 | 2 | 2 | 0 | 2 | 0 | 144 | 6.3 | 182 | 15.8 | 73 | +1.5 | | | 2@0 |
| 8 | 8PSK ⅔ | 11 | 4 | 3 | 6 | 2 | 1 | 1 | 0 | 0 | 100 | 11.6 | 69 | 30.6 | 26 | +2.5 | | | 1@0 |
| 9 | 16QAM ½ | 13 | 4 | 4 | 9 | 0 | 0 | 0 | 3 | 0 | 150 | 13.7 | 88 | 38.0 | 32 | | | | 0@−2 |
| 10 | 16QAM ⅔ | 15 | 4 | 3 | 12 | 0 | 0 | 0 | 4 | 0 | 272 | 15.8 | 138 | 50.4 | 43 | | | | 0@−3 |

Recommended: 3 for the first 220 s (readings −5 … +1), then 4, 5 (249–299 s), 6 (330–404 s),
5, 3 (433 s on). The ladder was pinned data then; the master's ladder column is its record of
those bursts.

* rung 3: 144 s of exchanges at 72 bit/s; neighbours rung 2 (100 % of 4) → 62, rung 4 (25 % of
  4) → 13: the best neighbour −13 %. The best measured rung, 7: 78 bit/s.
* rung 4 (QPSK ⅓): 36 s at 13 bit/s; rung 3 (94 % of 16) → 88: **+553 %**.
* rungs 8–10: 119 s at 26–43 bit/s, where rung 7 (50 % of 4) → 78 and rung 3 → 88.
* The slave's answers: 12 of 29 read by WC4Y; every one recommended 3 (readings −7 … −1 at WC4Y).

### 3.2 2026-10-15 02:37Z, ND1J → KK4ODA-1, 500 Hz, beta.106 (report 12, the night)

35 of 38 transmissions answered with a control frame ND1J read.

| rung | frame | sends | 1st | never | failed | alone | d:1st | d:later | d:comb | lost | bytes | frame s | bit/s | exch s | bit/s | SNR | meas | slave | master |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 5 | QPSK ½ | 6 | 6 | 0 | 0 | 6 | 6 | 0 | 0 | 0 | 132 | 6.3 | 167 | 8.6 | 123 | +10.6 | | | |
| 6 | QPSK ⅔ | 42 | 34 | 5 | 3 | 34 | 26 | 8 | 0 | 0 | 1054 | 44.3 | 190 | 66.4 | 127 | +10.7 | | | |
| 7 | 8PSK ½ | 5 | 4 | 0 | 1 | 4 | 3 | 1 | 0 | 0 | 144 | 5.3 | 219 | 8.6 | 133 | +10.6 | | | |
| 9 | 16QAM ½ | 43 | 39 | 1 | 3 | 39 | 35 | 4 | 0 | 0 | 1950 | 45.3 | 344 | 60.4 | 258 | +10.9 | | | |
| 10 | 16QAM ⅔ | 20 | 19 | 0 | 1 | 19 | 18 | 1 | 0 | 0 | 1292 | 21.1 | 490 | 34.4 | 300 | +11.0 | | | |
| 11 | 16QAM ¾ | 25 | 23 | 0 | 3 | 22 | 21 | 1 | 1 | 0 | 1794 | 26.3 | 545 | 37.5 | 383 | +11.3 | | | 4@12 |
| 12 | 64QAM ⅔ | 34 | 22 | 0 | 19 | 15 | 13 | 2 | 7 | 0 | 2332 | 35.8 | 521 | 49.4 | 378 | +11.2 | | | 4@13 |
| 13 | 64QAM ¾ | 4 | 4 | 0 | 0 | 4 | 4 | 0 | 0 | 0 | 480 | 4.2 | 911 | 6.1 | 632 | +11.6 | | | 4@11 |
| 14 | 64QAM ⅚ | 9 | 4 | 0 | 8 | 1 | 0 | 1 | 3 | 0 | 536 | 9.5 | 452 | 20.9 | 206 | +11.4 | | | 0@10 |

Recommended: 10, 12, 12, 11 … 11 (to 157 s), 12, 11, 10, 10, 11, 10, 9 … 9 (223–266 s), 7, 6
… 6, 5, 5, 6, 6, 5, 5 — readings +8 … +13 throughout.

* rung 12: 49 s at 378 bit/s; rung 11 (91 % of 23) → 397, rung 13 (100 % of 4) → 668:
  **+77 %**.
* rung 9: 60 s at 258; rung 10 (95 % of 19) → 359: +39 %. rung 10: 34 s at 300; rung 11 →
  397: +32 %.
* rung 6: 66 s at 127; rung 7 (75 % of 4) → 150: +18 %. rung 5: 9 s at 123.
* The slave recommended 12 at 73 s and 5 at 318 s on readings that never left +8 … +13 dB.

### 3.3 2026-10-15 15:29Z, ND1J → KK4ODA-1, 500 Hz, beta.108 (report 12, morning 1)

29 of 39 transmissions answered with a control frame ND1J read.

| rung | frame | sends | 1st | never | failed | alone | d:1st | d:later | d:comb | lost | bytes | frame s | bit/s | exch s | bit/s | SNR | meas | slave | master |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | tone-36 | 4 | 4 | 0 | 0 | 4 | 4 | 0 | 0 | 0 | 132 | 21.4 | 49 | 29.3 | 36 | −8.6 | | | |
| 2 | tone4x100-51 | 4 | 4 | 0 | 0 | 4 | 4 | 0 | 0 | 0 | 192 | 21.4 | 72 | 26.1 | 59 | −6.2 | | | 4@−6 |
| 3 | tone4x100-75 | 29 | 21 | 0 | 0 | 29 | 21 | 8 | 0 | 0 | 2088 | 155.4 | 107 | 200.4 | 83 | −2.9 | | | 4@−3 |
| 4 | QPSK ⅓ | 4 | 4 | 0 | 0 | 4 | 4 | 0 | 0 | 0 | 48 | 4.2 | 91 | 8.8 | 43 | −6.2 | | | 4@−6 |
| 5 | QPSK ½ | 13 | 4 | 3 | 2 | 8 | 2 | 6 | 0 | 0 | 176 | 13.7 | 103 | 32.1 | 44 | −2.2 | | | 2@−6 |
| 6 | QPSK ⅔ | 11 | 4 | 3 | 6 | 2 | 0 | 2 | 2 | 0 | 124 | 11.6 | 86 | 30.6 | 32 | −1.9 | | | 0@−4 |
| 7 | 8PSK ½ | 4 | 4 | 0 | 0 | 4 | 4 | 0 | 0 | 0 | 144 | 4.2 | 273 | 8.8 | 130 | +1.7 | | | 4@1 |
| 8 | 8PSK ⅔ | 4 | 4 | 0 | 0 | 4 | 4 | 0 | 0 | 0 | 200 | 4.2 | 380 | 9.0 | 178 | +2.1 | | | 4@2 |
| 9 | 16QAM ½ | 7 | 4 | 0 | 5 | 2 | 2 | 0 | 2 | 0 | 200 | 7.4 | 217 | 21.5 | 74 | +2.8 | | | 2@0 |
| 10 | 16QAM ⅔ | 8 | 4 | 4 | 4 | 0 | 0 | 0 | 0 | 0 | 0 | 8.4 | 0 | 18.0 | 0 | | | | 0@−6 |
| 11 | 16QAM ¾ | 8 | 4 | 1 | 7 | 0 | 0 | 0 | 0 | 0 | 0 | 8.4 | 0 | 21.6 | 0 | | | | 0@−7 |
| 12 | 64QAM ⅔ | 8 | 4 | 1 | 7 | 0 | 0 | 0 | 0 | 0 | 0 | 8.4 | 0 | 25.1 | 0 | | | | 0@−6 |

Recommended: 3 for 254 s (readings −8 … +2), 4, 5 … 5 (272–340 s), 4, 4, 3 … 3.

* rung 3: 200 s at 83 bit/s; rung 2 → 62, rung 4 (QPSK ⅓, 100 % of 4) → 52: the rung used was
  the better neighbour (−25 %). The best measured rung, 8 (100 % of 4): 216 bit/s; rung 7
  (100 % of 4): 156. Rungs 5 and 6 between them decoded 50 % and 0 %: report 12's 7.8 dB gap
  between 3 and 5, and nothing to climb through.
* rung 5: 32 s at 44; rung 6: 31 s at 32 — rung 7 → 156: **+380 %** over rung 6.
* rungs 10–12 (the pinned ladder): 65 s, nothing delivered, 6 of 24 frames never detected.

### 3.4 2026-10-15 15:42Z, ND1J → KK4ODA-1, 500 Hz, beta.108 (report 12, morning 2)

25 of 37 transmissions answered with a control frame ND1J read.

| rung | frame | sends | 1st | never | failed | alone | d:1st | d:later | d:comb | lost | bytes | frame s | bit/s | exch s | bit/s | SNR | meas | slave | master |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | tone-24 | 8 | 8 | 0 | 0 | 8 | 8 | 0 | 0 | 0 | 168 | 42.9 | 31 | 58.5 | 23 | −10.1 | | | |
| 1 | tone-36 | 11 | 11 | 0 | 0 | 11 | 11 | 0 | 0 | 0 | 363 | 59.0 | 49 | 81.5 | 36 | −5.2 | | | 4@−6 |
| 2 | tone4x100-51 | 12 | 11 | 0 | 1 | 11 | 10 | 1 | 0 | 0 | 528 | 64.3 | 66 | 73.0 | 58 | −6.6 | | | 4@−5 |
| 3 | tone4x100-75 | 15 | 9 | 0 | 2 | 13 | 9 | 4 | 1 | 0 | 1008 | 80.4 | 100 | 106.8 | 75 | −6.6 | | | 4@−9 |
| 4 | QPSK ⅓ | 5 | 4 | 1 | 0 | 4 | 3 | 1 | 0 | 0 | 48 | 5.3 | 73 | 14.6 | 26 | −4.8 | | | 3@−5 |
| 5 | QPSK ½ | 16 | 4 | 10 | 5 | 1 | 0 | 1 | 0 | 0 | 22 | 16.9 | 10 | 36.1 | 5 | −4.6 | | | 0@−13 |
| 6 | QPSK ⅔ | 13 | 4 | 5 | 7 | 1 | 0 | 1 | 3 | 0 | 124 | 13.7 | 72 | 32.6 | 30 | −3.1 | | | 0@−14 |
| 7 | 8PSK ½ | 12 | 4 | 3 | 7 | 2 | 0 | 2 | 0 | 0 | 72 | 12.6 | 46 | 31.8 | 18 | −1.9 | | | 0@−10 |

Recommended: 2, 1, 2, 2, 3 … 3 (184–318 s), 2 … 2 (339–439 s), 3 … 3 — readings −15 … −2.

* rungs 0–2: 213 s at 23–58 bit/s, each time the next rung up measured better (+87 %, +59 %,
  +62 %); rung 3 (100 % of 9) → 94 was the best measured, and the link ran it for 107 s at 75.
* rung 5: 36 s at 5 bit/s — 10 of 16 sends never detected, 5 failed, 1 decoded. The ladder's
  rung 5 at −13 dB reported.
* The learned margin and R9's silence steps on the ladder's leftovers kept the link under rung 3
  (report 12 §12).

### 3.5 2026-10-17 02:05Z, WC4Y → KK4ODA-1, 500 Hz, beta.111 (report 13)

22 of 25 transmissions answered with a control frame WC4Y read.

| rung | frame | sends | 1st | never | failed | alone | d:1st | d:later | d:comb | lost | bytes | frame s | bit/s | exch s | bit/s | SNR | meas | slave | master |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2 | tone4x100-51 | 4 | 4 | 0 | 0 | 4 | 4 | 0 | 0 | 0 | 192 | 21.4 | 72 | 31.9 | 48 | +2.5 | | | |
| 3 | tone4x100-75 | 41 | 25 | 1 | 2 | 38 | 24 | 14 | 0 | 0 | 2736 | 219.8 | 100 | 271.5 | 81 | −0.8 | | | |
| 4 | QPSK ⅓ | 4 | 4 | 1 | 0 | 3 | 3 | 0 | 0 | 0 | 36 | 4.2 | 68 | 8.9 | 33 | +1.7 | | | 3@1 |
| 5 | QPSK ½ | 15 | 10 | 3 | 3 | 9 | 6 | 3 | 1 | 0 | 220 | 15.8 | 111 | 38.0 | 46 | −0.3 | | | 4@1 |
| 6 | QPSK ⅔ | 4 | 4 | 0 | 1 | 3 | 3 | 0 | 0 | 0 | 93 | 4.2 | 176 | 8.9 | 84 | −0.3 | | | 3@0 |
| 7 | 8PSK ½ | 4 | 4 | 0 | 3 | 1 | 1 | 0 | 0 | 0 | 36 | 4.2 | 68 | 8.8 | 33 | +0.2 | | | 1@0 |
| 8 | 8PSK ⅔ | 4 | 4 | 0 | 4 | 0 | 0 | 0 | 0 | 0 | 0 | 4.2 | 0 | 9.0 | 0 | | | | 0@−4 |
| 9 | 16QAM ½ | 4 | 4 | 3 | 1 | 0 | 0 | 0 | 0 | 0 | 0 | 4.2 | 0 | 9.0 | 0 | | | | 0@−1 |

Recommended: 3 throughout but two 2s (233, 399 s) — readings −7 … +2.

* rung 3: 271 s at 81 bit/s; rung 2 → 63, rung 5 (60 % of 10) → 58: the rung used was the better
  neighbour (−22 %). The best measured, rung 6 (75 % of 4): 102.
* rung 5: 38 s at 46; rung 6 → 102: +120 %.

### 3.6 2026-10-22 00:29Z, WC4Y → KK4ODA-1, 500 Hz, beta.118 (report 14)

13 of 16 transmissions answered with a control frame WC4Y read; six measurements.

| rung | frame | sends | 1st | never | failed | alone | d:1st | d:later | d:comb | lost | bytes | frame s | bit/s | exch s | bit/s | SNR | meas | slave | master |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 3 | tone4x100-75 | 0 | 0 | 0 | 0 | 4 | 0 | 0 | 0 | 0 | 0 | 0.0 | | 0.0 | | −1.9 | 4 | 4/4 | 4@−2 |
| 4 | tone4x100-93 | 29 | 24 | 1 | 5 | 27 | 20 | 3 | 1 | 0 | 2160 | 155.4 | 111 | 189.4 | 91 | +0.4 | 4 | 4/4 | 4@0 |
| 5 | QPSK ½ | 0 | 0 | 0 | 1 | 3 | 0 | 0 | 0 | 0 | 0 | 0.0 | | 0.0 | | −0.6 | 4 | 3/4 | 3@0 |
| 6 | QPSK ⅔ | 0 | 0 | 0 | 3 | 1 | 0 | 0 | 0 | 0 | 0 | 0.0 | | 0.0 | | −0.5 | 4 | 1/4 | 1@0 |
| 7 | 8PSK ½ | 0 | 0 | 0 | 4 | 0 | 0 | 0 | 0 | 0 | 0 | 0.0 | | 0.0 | | | 4 | 0/4 | 0@1 |
| 9 | 16QAM ½ | 0 | 0 | 1 | 3 | 0 | 0 | 0 | 0 | 0 | 0 | 0.0 | | 0.0 | | | 4 | 0/3 | 0 |

Recommended: 4 throughout — readings −10 … +3.

* rung 4: 189 s at 91 bit/s; rung 3 (100 % of 4) → 94 (+3 %), rung 5 (75 % of 4) → 72. **On
  the best rung measured.** (Report 14's capture skips cost one never-detected and one failed
  frame.)

### 3.7 2026-10-23 00:52Z, WC4Y → KK4ODA-1, 2300 Hz, beta.120 (report 15)

19 of 24 transmissions answered with a control frame WC4Y read; seven measurements.

| rung | frame | sends | 1st | never | failed | alone | d:1st | d:later | d:comb | lost | bytes | frame s | bit/s | exch s | bit/s | SNR | meas | slave | master |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2 | tone50-51 | 2 | 0 | 0 | 0 | 2 | 0 | 2 | 0 | 0 | 96 | 10.7 | 72 | 15.4 | 50 | −1.2 | | | |
| 4 | tone100-105 | 5 | 5 | 0 | 0 | 5 | 5 | 0 | 0 | 0 | 510 | 26.8 | 152 | 31.9 | 128 | −3.4 | | | |
| 5 | tone100-153 | 0 | 0 | 0 | 0 | 4 | 0 | 0 | 0 | 0 | 0 | 0.0 | | 0.0 | | −0.8 | 4 | 4/4 | 4@−1 |
| 7 | BPSK ⅓ | 54 | 18 | 20 | 31 | 7 | 1 | 6 | 9 | 0 | 688 | 56.9 | 97 | 123.5 | 45 | −0.7 | 4 | 0/3 | 0@−2 |
| 8 | BPSK ½ | 0 | 0 | 1 | 0 | 3 | 0 | 0 | 0 | 0 | 0 | 0.0 | | 0.0 | | +0.6 | 4 | 3/3 | 3@1 |
| 9 | QPSK ⅓ | 0 | 0 | 1 | 3 | 0 | 0 | 0 | 0 | 0 | 0 | 0.0 | | 0.0 | | | 4 | 0/3 | 3 |
| 10 | QPSK ½ | 0 | 0 | 2 | 2 | 0 | 0 | 0 | 0 | 0 | 0 | 0.0 | | 0.0 | | | 4 | 0/2 | 0@−1 |
| 11 | QPSK ⅔ | 0 | 0 | 2 | 2 | 0 | 0 | 0 | 0 | 0 | 0 | 0.0 | | 0.0 | | | 4 | 0/2 | 0@0 |
| 12 | 8PSK ½ | 0 | 0 | 0 | 4 | 0 | 0 | 0 | 0 | 0 | 0 | 0.0 | | 0.0 | | | 4 | 0/4 | 0@−1 |

Recommended: 7 for 188 s (readings −3 … 0), 5 (202–261 s), 4 (273 s on).

* rung 7: 123 s of exchanges at 45 bit/s — 54 sends of 18 blocks, 20 never detected, 31 failed,
  7 decoded alone; 1 block delivered by its first copy, 6 by a later copy, 9 by combining.
  Rung 5 (100 % of 4) → 193, rung 8 (75 % of 4) → 211: **+374 %**.
* rung 4: 32 s at 128; rung 5 → 193: +51 %.
* The ladder's rung 9 "3" at the master is rung 8's report taken for rung 9's (ADR-0092 since).

### 3.8 2026-10-23 16:04Z, ND1J → KK4ODA-1, 500 Hz, beta.121 (report 16, Test 1)

15 of 16 transmissions answered with a control frame ND1J read; five measurements.

| rung | frame | sends | 1st | never | failed | alone | d:1st | d:later | d:comb | lost | bytes | frame s | bit/s | exch s | bit/s | SNR | meas | slave | master |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 6 | QPSK ⅔ | 6 | 6 | 0 | 0 | 6 | 6 | 0 | 0 | 0 | 186 | 6.3 | 235 | 8.6 | 173 | +7.8 | | | |
| 9 | 16QAM ½ | 6 | 6 | 0 | 0 | 6 | 6 | 0 | 0 | 0 | 300 | 6.3 | 380 | 8.6 | 280 | +8.1 | | | |
| 10 | 16QAM ⅔ | 1 | 1 | 0 | 0 | 5 | 1 | 0 | 0 | 0 | 68 | 1.1 | 516 | 2.9 | 186 | +9.5 | 4 | 4/4 | 4@10 |
| 11 | 16QAM ¾ | 37 | 37 | 0 | 0 | 41 | 37 | 0 | 0 | 0 | 2886 | 39.0 | 592 | 54.6 | 423 | +8.7 | 4 | 4/4 | 4@8 |
| 12 | 64QAM ⅔ | 0 | 0 | 0 | 0 | 4 | 0 | 0 | 0 | 0 | 0 | 0.0 | | 0.0 | | +9.1 | 4 | 4/4 | 4@9 |
| 13 | 64QAM ¾ | 0 | 0 | 0 | 0 | 4 | 0 | 0 | 0 | 0 | 0 | 0.0 | | 0.0 | | +8.6 | 4 | 4/4 | 4@8 |
| 14 | 64QAM ⅚ | 0 | 0 | 0 | 2 | 2 | 0 | 0 | 0 | 0 | 0 | 0.0 | | 0.0 | | +8.7 | 4 | 2/4 | 2@10 |

Recommended: 9, 10, 11 … 11 — readings +8 … +11.

* rung 11: 55 s at 423 bit/s; rung 10 → 379, rung 12 (100 % of 4) → 591: **+40 %**. The best
  measured, rung 13 (100 % of 4): 669.
* Every frame detected and decoded at its first copy below rung 14.

### 3.9 2026-10-23 16:07Z, ND1J → KK4ODA-1, 500 Hz, beta.121 (report 16, Test 2)

21 of 23 transmissions answered with a control frame ND1J read; five measurements.

| rung | frame | sends | 1st | never | failed | alone | d:1st | d:later | d:comb | lost | bytes | frame s | bit/s | exch s | bit/s | SNR | meas | slave | master |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 10 | 16QAM ⅔ | 6 | 6 | 0 | 0 | 6 | 6 | 0 | 0 | 0 | 408 | 6.3 | 516 | 8.6 | 379 | +9.0 | | | |
| 11 | 16QAM ¾ | 0 | 0 | 0 | 0 | 4 | 0 | 0 | 0 | 0 | 0 | 0.0 | | 0.0 | | +10.2 | 4 | 4/4 | 4@10 |
| 12 | 64QAM ⅔ | 89 | 89 | 0 | 0 | 93 | 89 | 0 | 0 | 0 | 9434 | 93.8 | 805 | 129.5 | 583 | +8.4 | 4 | 4/4 | 4@10 |
| 13 | 64QAM ¾ | 0 | 0 | 0 | 0 | 8 | 0 | 0 | 0 | 0 | 0 | 0.0 | | 0.0 | | +9.7 | 8 | 8/8 | 4@10 |
| 14 | 64QAM ⅚ | 0 | 0 | 0 | 3 | 1 | 0 | 0 | 0 | 0 | 0 | 0.0 | | 0.0 | | +9.4 | 4 | 1/4 | 1@8 |

Recommended: 12 throughout — readings +8 … +10.

* rung 12: 129 s at 583 bit/s; rung 11 → 435, rung 13 (100 % of 8) → 669: **+15 %**.
* Rung 13 was measured twice (the first slot went silent to a false detection, report 16 §3)
  and decoded 8 of 8; the master recorded 4 of 4.

## 4. The controller, replayed

Every session, read with the model of the build it ran (`--tree`):

| session | build | slave's recommendations read by the master | reproduced | master's headers read by the slave | reproduced |
|---|---|---|---|---|---|
| 10-14 23:59Z WC4Y | beta.106 | 29 (2 unread) | 29 | 13 (2 unread) | 13 |
| 10-15 02:37Z ND1J | beta.106 | 35 (1 unread) | 35 | 29 | 29 |
| 10-15 15:29Z ND1J | beta.108 | 29 (1 unread) | 29 | 13 (2 unread) | 13 |
| 10-15 15:42Z ND1J | beta.108 | 25 (2 unread) | 25 | 12 | 12 |
| 10-17 02:05Z WC4Y | beta.111 | 22 (1 unread) | 22 | 6 (2 unread) | 6 |
| 10-22 00:29Z WC4Y | beta.118 | 13 (1 unread) | 13 | 7 (4 unread) | 7 |
| 10-23 00:52Z WC4Y | beta.120 | 19 (1 unread) | 19 | 17 (3 unread) | 17 |
| 10-23 16:04Z ND1J | beta.121 | 15 (1 unread) | 15 | 12 (1 unread) | 12 |
| 10-23 16:07Z ND1J | beta.121 | 21 (2 unread) | 21 | 20 | 20 |

203 of 203 and 129 of 129. "Unread" is a transmission the other station decoded no control frame
of, so there is nothing to compare; the replayed controller runs through it as the engine did.

What this establishes: the controller's inputs are all in the two sidecars, and the tool feeds
them as the engine does. A candidate can be replayed on the nine sessions' recorded readings and
outcomes, and compared rung for rung with what was sent. The replay is open-loop: a candidate
that recommended another rung would have been sent other frames, so the comparison is of
decisions on the same evidence, not of outcomes.

The masters' controllers recommended one rung each all session (the slaves sent no data): the
rung the acceptance's floor reading seeded, 4 or 5 at 500 Hz, 4 at 2300 Hz, 1 on the two
weakest paths. A slave with data to send would have started there.

## 5. Where the controller left speed or lost frames, and why

Read from §3, with the reports' analyses behind each.

1. **The climb stops below what the ladder decodes.** Test 1 of 10-23 ran at rung 11 where rung
   12 (4 of 4) would have delivered +40 % and rung 13 (4 of 4) +54 %; Test 2 at rung 12 where
   rung 13 (8 of 8) was +15 %; the night session of 10-15 at rung 12 for 49 s where rung 13 (4
   of 4) was +77 %, and at 9 and 10 where 10 and 11 were +39 % and +32 %. In each the readings
   were steady at +8 … +13 dB and the rung above needed the next threshold plus the minimum
   margin plus the step-up headroom (report 16 §4.2). The reading was not the limit; the rule
   was.
2. **The first OFDM rungs were chosen where the floor's top carried more.** 10-23 at 2300 Hz:
   rung 7 for 123 s at 45 bit/s with 20 of 54 sends never detected and 31 failed, where rung 5
   (4 of 4) would have carried 193 and rung 8 (3 of 4) 211 — the cap ADR-0093 removed the next
   day, and two blind spots that remain: a frame never detected counts for nothing, and a block
   delivered by combining counts as a clean transmission (9 of 18 blocks were). 10-15 morning 1:
   rungs 5 and 6 for 63 s at 44 and 32 bit/s where rung 3 carried 83 and rung 7 (4 of 4) would
   have carried 156. 10-17: rung 5 for 38 s at 46 where rung 6 (3 of 4) → 102. 10-14: rung 4
   (QPSK ⅓, since dropped by ADR-0073) for 36 s at 13 bit/s.
3. **The learned margin walked a strong session down.** The night session of 10-15: from rung
   12 at 73 s to rung 5 at 318 s, on readings that never left +8 … +13 dB. Rung 12 lost 19 of 34
   sends and rung 14 8 of 9 (the pinned ladder), and each failure widened the margin by a full
   step (ADR-0084's partial-failure rule came two betas later); clean bursts at 11, 10 and 9
   gave it back a quarter decibel at a time (ADR-0007's schedule). Report 12 called it the margin
   at its cap.
4. **Holding the floor's low rungs.** 10-15 morning 2: rungs 0–2 for 213 s at 23–58 bit/s while
   rung 3 (9 of 9) carried 94; R9's silence steps on the ladder's leftover retransmissions and
   the margin kept the link under 3 (report 12 §12). 10-14: the link sat on rung 3 for 144 s
   (72 bit/s) where rung 3 was the better neighbour; the loss there was the ladder's pinned
   bursts above it, 119 s at 26–43 bit/s.
5. **Right choices, for the record.** 10-22 ran rung 4 (tone4x100-93) at 91 bit/s with the
   neighbours at 94 and 72: on the best rung measured. 10-17 and morning 1 ran rung 3 where both
   neighbours measured lower; what they left was above a gap (rungs 5–6 decoding half or none),
   not next door.
6. **Frames never detected.** Across the nine sessions, of the master's data sends at OFDM
   rungs: 10-23 2300 Hz, 20 of 54 at rung 7; morning 2, 18 of 41 at rungs 5–7; 10-14, 10 of 39
   at rungs 8–10; the night session, 6 of 182. At the tone rungs, 2 of 158. The receiver's
   acquisition at 500 Hz loses about one OFDM frame in five on the WC4Y path at −1 dB (report
   13); the controller never hears of them.

Two blind spots the brief lists are measured now: never-detected frames (item 6) and blocks
delivered by combining (item 2: 9 of 18 at rung 7 on 10-23). The third, the tone-floor reading's
spread, shows in the masters' seeds: the acceptance read −3.0 … +6.2 dB at ND1J on 10-23 for a
path whose OFDM data read +8 … +10 at the other end.

## 6. The eleven adjustments, and what each rested on

Evidence classes: *pipe* (the link bench's lossy or fading pipe, which detects every frame and
adds 3 dB a copy), *phy bench* (the real modem on a bench channel), *model sim* (the model's
two-station simulator), *harness* (two real daemons on the channel server), *field* (an on-air
session). What each ADR says of its own evidence, read from the ADR.

| ADR | What it added | What it removed | Evidence | The numbers |
|---|---|---|---|---|
| 0007 faster climb | `decay_growth` ×2 per clean burst after the sticky ones, `max_down_step_db` 1 | nothing; "a lone failure is an accident" tried and rejected | pipe (the old lossy pipe), phy bench (16 kB at 12 dB), a model replica of the VarAC case | wide grid +1.8 %, narrow +3.2 %; the accident rule −11 % Moderate 16 dB; VarAC case 228 → 212 s |
| 0008 faster start | `seed`, `first_mode`, `first_mode_back` 2, the connect body's SNR | the start at mode 0 | pipe (2 kB), phy bench (12 dB AWGN) | wide −24 %, narrow −9 %; one step in hand +17 % Poor 8 dB narrow; real modem 29 → 15 s |
| 0012 holding the link | `silence_step` 2, the link timeout of four exchanges | nothing; starting lower, 4 steps in hand, wider margins rejected | pipe (the new fading pipe, 30 a point) | 500 Hz completions at −6 … 0 dB 1–28 → 17–30 of 30 |
| 0013 §4 the tone floor | the ladder, `floor_modes`, the 1 dB cap on the first wide OFDM rung and "once" | ADR-0009's OFDM floor | pipe (`link_tone_floor.csv`), phy bench (`tone_floor.csv`) | without the cap 2300 Hz Good −2 dB 89 → 46 bit/s; "not measured on the air" |
| 0014 §4 fast tones | rungs 2–5 at 2300 Hz; the cap kept | BPSK ⅕ from the usable set | pipe, phy bench | cap off: AWGN 0 dB 263 → 142 bit/s; "not measured on the air" |
| 0016 calls on the floor | `seed(lower_bound)`, `reseed_margin_db` 3 | nothing | phy bench (the floor's SNR ceilings), pipe (sessions) | ceilings +17.5/+15.5/+12/+5 dB; Poor +24 dB 36 → 23 s |
| 0020 what a failure says | `trusted`, RV 0/3 or combined, `_asked`, `observe_snr` | nothing | field (ND1J 2026-09-25, both recordings, replayed open-loop), pipe (no regression) | margin 12 → 2–6 dB, rungs 0–3 → 7–10 on the replay |
| 0066 the start on the floor | `_required`, steps in decibels on the floor | nothing | field (WC4Y 2026-10-11, one side), harness (`80m-floor-top-start-500`, 3 draws), pipe | harness 57 → 77 bit/s; pipe −1.6 … −6.9 % |
| 0084 partial failures | `partial_share` 0.5, `_at_edge`, `_widen(share)` | nothing; per-class thresholds, a per-rung offset, an exchange-priced ladder rejected | pipe (`link_partial_failures.csv`), harness (56 × 2, with 0082–0085) | pipe up to +59 %; harness air −6.5 %, file ×1.14; per-class thresholds −5 … −47 % |
| 0087 echo | `observe_echo`, `echo_limit`, `echo_weight` 0.4 | nothing; learning refused rungs from failures, a delivery filter rejected | phy bench (`bench_echo` 704 points; NVIS link bench), harness (NVIS 8 × 4; 56 × 2) | 500 Hz NVIS 1 kB 256 → 213 s; harness NVIS air −5.5 % |
| 0093 the last patch | `_at_edge` on both airs | the cap, "once", `floor_margin_db`, `WIDE_FLOOR_MARGIN_DB` | field (WC4Y 2026-10-23 2300 Hz, both sides), harness (every 2300 Hz scenario; the new one × 4) | the air's draw 80/95 → 173/171 bit/s |

Related rules: ADR-0038 (the hopeless rule: field one side + pipe), 0073 (`usable_modes` by data
after the header: field both sides + pipe + harness), 0077 and 0092 (measurement frames: field +
harness), 0085 and 0086 (stranded frames: harness + model sim), R9 (the silence steps: from
0012's pipe through the link review's simulator).

Three readings of the table:

* **Seven of eleven were judged on the fading pipe**, four of them on it alone (0007, 0008,
  0012, 0013/0014's cap). The pipe detects every frame and credits every copy; the two blind
  spots in §5 cannot show on it. ADR-0093 shows what that cost: a cap the pipe said paid, held
  for a month, and on the air held a rung that lost a third of its frames.
* **The field sessions behind 0020, 0066 and 0093 were read by hand**, one or both sides; no
  ADR before this gate had a per-rung reading of both sides.
* **The parameters are the residue of the evidence.** Nineteen in `RateController`, each set
  by one bench at one time; none has been measured against another since.

## 7. The oracle on the harness

### 7.1 What it is

For a scenario, the harness is run as it is (the controller choosing) and at each fixed rung.
The best fixed rung's delivery is what a controller that knew the path in advance, and never
spent an exchange finding out, would have reached: a ceiling, not a target. A controller's
regret is that over its own, less one. The measure is the data a run delivered per second of
air on its message step (0 when it was lost), or its Test's file rate, or its message rate.

A pinned station follows no recommendation for its own data; the other station's recommendation
still decides what a stranded block is re-encoded at after `max_combines` copies (ADR-0085),
so a rung the path cannot carry at all degrades into its re-encodes rather than stalling, as it
would on the air.

### 7.2 The smoke test

`quick-awgn-2300` (AWGN, 15 dB, a 3000 B message and a 600 B reply):

| run | message | air |
|---|---|---|
| the controller | 1263 bit/s | 58 s |
| rung 9 (QPSK ⅓) | 511 bit/s | 91 s |
| rung 5 (tone100-153) | 209 bit/s | 176 s |

The controller beats both fixed rungs by a wide margin, as it should at 15 dB AWGN: the best
fixed rung there is near the top of the ladder.

### 7.3 The first pass

Fourteen scenarios, one draw each, at a coarse set of rungs. The measure is the largest
`message` step's rate (a short first message is mostly the exchange's overhead) or, for a Test,
the file's. *lost* is a run whose message did not arrive within the scenario's limits: a rung too
slow for the budget (`quick-awgn-2300` runs 240 s of air; a 3000 B message at tone-36 needs 670)
or too fast for the path.

At 500 Hz, rungs 1, 3, 4, 5, 7, 9, 11 and 13:

| scenario | path | the controller | best fixed rung | regret | by rung, bit/s |
|---|---|---|---|---|---|
| `80m-weak-500` | Poor −7 dB, 1000 B | 70 | 3: 82 | +17 % | 1:39 3:82 4:52 5:14 7:16 9:22 11:23 13:22 |
| `80m-nvis-evening-500` | NVIS 9/4 dB, Test | 41 | 1: 39 | −5 % | 1:39 3:38 4:19 5:14 7:16 9:15 11:24 13:16 |
| `80m-weak-both-ways-ladder-500` | Good −5/−10 dB, Test | 78 | 4: 70 | −11 % | 1:39 3:55 4:70 5:37 7:25 9:26 11:28 13:30 |
| `80m-flutter-500` | flutter 8 dB, Test | 129 | 9: 163 | +26 % | 1:41 3:88 4:108 5:92 7:147 9:163 11:114 13:68 |
| `80m-qsb-dropout-500` | Moderate 8 dB, a dropout, 8000 B | 146 | 11: 198 | +36 % | 1:lost 3:87 4:107 5:82 7:136 9:150 11:198 13:172 |
| `20m-dx-500` | Moderate 6 dB, 6000 B | 146 | 11: 219 | +50 % | 1:lost 3:95 4:122 5:95 7:146 9:171 11:219 13:201 |
| `80m-strong-moderate-500` | Moderate 11/5 dB, Test | 151 | 13: 317 | +110 % | 1:lost 3:90 4:117 5:81 7:152 9:163 11:282 13:316 |

At 2300 Hz, rungs 1, 3, 5, 7, 9, 11, 14, 16 and 18:

| scenario | path | the controller | best fixed rung | regret | by rung, bit/s |
|---|---|---|---|---|---|
| `80m-weak-wide-test-2300` | Moderate −2/−6 dB, Test (the air's path, report 15) | 167 | 5: 175 | +4 % | 1:37 3:83 5:175 7:108 9:137 11:86 14:73 16:86 18:86 |
| `40m-good-throughput-2300` | Good 20 dB, 16000 B | 2327 | 18: 2612 | +12 % | 1–7:lost 9:512 11:1041 14:1580 16:2246 18:2612 |
| `80m-thunderstorm-agc-2300` | Moderate 10 dB, crashes, an AGC, Test | 591 | 14: 826 | +40 % | 1:lost 3:92 5:203 7:178 9:400 11:762 14:826 16:599 18:204 |
| `20m-polar-path-2300` | high Moderate 12 dB, 2000 B | 198 | 9: 286 | +44 % | 1:45 3:98 5:198 7:148 9:286 11:112 14:95 16:92 18:99 |
| `20m-dx-moderate-2300` | Moderate 8 dB, 1500 B | 632 | 14: 923 | +46 % | 1:lost 3:98 5:211 7:240 9:500 11:706 14:923 16:706 18:286 |
| `quick-awgn-2300` | AWGN 15 dB, 3000 B | 1263 | 16: 2182 | +73 % | 1,3:lost 5:209 7:240 9:511 11:1091 14:1600 16:2182 18:1846 |
| `15m-equatorial-flutter-2300` | flutter 10 dB, 1500 B | 387 | 14: 800 | +107 % | 1:44 3:98 5:211 7:240 9:500 11:632 14:800 16:632 18:lost |

Read with the caution one draw deserves (§8):

* **A fixed rung beat the controller in eleven of fourteen.** On the two weakest 500 Hz Tests
  the controller beat every fixed rung (−5 %, −11 %): there it moves between the floor's rungs
  as the path moves, which no fixed rung can. On the air's own 2300 Hz path it was within 4 %
  of the best rung, which is rung 5: ADR-0093 at work.
* **The largest regrets are on the stronger paths and the short sessions**: +110 % on
  `80m-strong-moderate-500` (the file at rungs 7–9 where 13 carried twice as much), +107 % on
  the 15 m flutter, +73 % on 3000 B over AWGN at 15 dB, where the climb from the call's rung to
  the top is most of the session. These are the top-of-ladder cases of §5 item 1.
* **The floor's top against the first OFDM rungs**, as in §5 item 2: on `80m-weak-500` rung 3
  carried 82 and rungs 5–7 carried 14–16; on `80m-weak-both-ways-ladder-500` rung 4 carried 70
  and rungs 5–9 25–37; on `80m-weak-wide-test-2300` rung 5 carried 175 and rung 7 108.
* **The ceiling is not reachable.** A fixed rung two steps too fast loses the message outright
  (every *lost* above); the oracle knows the path, a controller must find it. The regret says how
  much there is to find, not what a controller should get.
* **One draw each.** The controller's run and a pinned run are the same seed on different session
  timings, so each is another draw of the fading (`00-brief.md` §8). A second pass at the rungs
  either side of each best, and three draws of each, are the next runs; Gate 2's criteria will
  say how the oracle is read.

## 8. What this gate did not settle

* **The replay is open-loop.** It tells whether a candidate would have recommended otherwise on
  the same evidence, not what the session would then have done. The harness is where outcomes
  are judged.
* **One path, mostly.** Eight of the nine sessions are 80 m NVIS between the author and two
  testers, at −10 … +13 dB; one is 2300 Hz. The oracle's scenarios cover the classes the field
  does not.
* **The sidecars still record no sequence numbers and no combining.** The reconstruction holds
  on these nine (no block decoded and unacknowledged); a `seq` and a `combined` field on the
  sidecar's frame and sent records would make it exact for every session from the next beta on.
  Outside the controller; it can ship.
* **The 2300 Hz record** is one session. The air at 2300 Hz is where the floor-to-OFDM choice
  cost the most (§5 item 2) and where the ladder record is thinnest.
* **The oracle's first pass is one draw at a coarse set of rungs.** Its numbers rank the
  scenarios and size the gap; they do not yet measure it. Several draws, the rungs either side of
  each best, and criteria registered before the comparison (Gate 2) come before any candidate is
  judged on it.

## 9. Reproducing

```
# both sides of every protocol-8 session, into field/inbox (one at a time)
python tools/fetch_uploads.py get 20261023-174737 --days 30 --extract   # and the rest
# a session, with the model of the build it ran
git worktree add /tmp/beta.121 v0.2.0-beta.121
uv run --project model python -I tools/rung_reading.py MASTER.json SLAVE.json \
    --tree /tmp/beta.121 --replay --timeline
# every pair under the inbox, with this tree's model
uv run --project model python -I tools/rung_reading.py --inbox field/inbox --replay --csv rungs.csv
# the oracle
uv run --project model python tools/rung_oracle.py bench/scenarios/80m-weak-500.toml \
    --rungs 1,3,4,5,7,9,11,13 --jobs 2 --out runs/oracle --drop-audio --summary oracle.csv
```
