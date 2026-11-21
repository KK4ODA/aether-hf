# RadioPost mail checks on 80 m and 40 m to the three-dial scanning gateway (2026-11-21)

Three short sessions in the afternoon of 2026-11-21 (18:39–18:56Z, local 13:39–13:56), from the
mobile's RadioPost client to the house's RadioPost Gateway, which scanned three channels. The
mobile changed band by hand between sessions: 80 m, 40 m, then 80 m. All three ran at 2300 Hz
on beta.168 at both ends and completed. Before them, the author tried keying the mobile by
Kenwood CAT, and the radio would not tune (§7).

* **Who**:
  * **The mobile**: station `KK4ODA-2` (TS-480, 65 W, screwdriver, a 22-core PC), beta.168,
    keyed by RTS on the DigiRig's COM9 for the sessions. RadioPost client 0.1.0-beta.32, calling
    as `WD5EMA`.
  * **The house**: `KK4ODA-1` (FTDX10 on Yaesu CAT, 50 W, T2D, the 4-core mini PC), beta.168,
    under automatic control. RadioPost Gateway 0.1.0-beta.32 holding `KK4ODA-1` and scanning
    7.101, 3.590 and 14.107 MHz since 13:46Z (`scan_s` 54 s).
* **The files**, none committed: the house's debug zips `20261121-184101Z KK4ODA-1 …`,
  `20261121-184321Z KK4ODA-1 …` and `20261121-185711Z KK4ODA-1 …`, and the mobile's
  `20261121-184102Z KK4ODA-2 …` and `20261121-185502Z KK4ODA-2 …`. Both sides of sessions 1 and
  2 are there. **The mobile's side of session 3 was never uploaded**; it has been asked for, and
  session 3 is read from the house's side alone until it comes.
* **Read with** `tools/two_sides.py` (with and without `--from-loops`), `tools/air_account.py
  --sidecars`, `tools/rung_reading.py --timeline`, `tools/c1_replay.py`, `tools/floor_trace.py`
  (beta.161's `aetherd.exe`, the only daemon on this PC; the busy detector is unchanged since), the
  daemons' logs and kept run logs, both programs' logs, the diagnostic bundles, and a scratch
  count of the house's 12 205 dial moves.

## 0. Summary

**What happened.** Three gateway mail checks, each the client's 163-byte opening, the gateway's
52-byte reply and the client's 5-byte close. Every data frame decoded at its first copy, in both
directions. Neither program's log names a message, so nothing was waiting. The house read the mobile at −0.2 to +2.9 dB and
the mobile read the house at +9 to +11 dB, the usual 8–10 dB lopsided path.

**The granted slots held** (§2). No overlaps; the least clear air was 0.564 s; every answer
reached the air 71–92 ms after its slot opened. Report 19's two fixes worked on the air (§6): the
mobile identified as `WD5EMA` after RadioPost had left, and a both-ways try no longer taught the
house a longer gap.

**Findings, ranked by what they cost. Each is the author's call.**

1. **The scan's lingers on a busy band** (§3). From 15:26 to 15:56Z the house sat on 14.107 MHz
   for eight visits of 184–214 s each, with one 4–5 s look at 40 m and 80 m between them. That
   is the scan's cap, the longest call (fifteen tries) plus a dwell. Since 13:46Z the next visit to
   3.590 MHz was more than 54 s away for 8.9 % of the afternoon, and to 7.101 MHz for 7.3 %. The
   gateway publishes 54 s. Session 1's call met a short version of this: the house sat 37 s on
   14.107 MHz while the mobile called on 80 m, and the call was found at its fourth and last try,
   42.5 s in. Nothing in the logs says what the house heard on 20 m that kept it there.
2. **The client called with 49 s and offered two channels** (§3). The gateway had registered three
   channels and 54 s at 13:46Z. The client used the older record: `call_s` 49 in each `connect`,
   and only 7.101 and 3.590 MHz in its dial prompts. This one is RadioPost's to fix.
3. **Two waits a session from the programs' timing** (§4). The gateway's reply missed the house's
   answer by 3–85 ms in all three sessions, which cost one exchange, about 2.2 s, each time. In
   sessions 2 and 3 the client asked to disconnect after the mobile had already granted an
   open window, and the DISC waited out that window, about 5.6 s. Together that is about 7.8 of
   the 17–18 s from acceptance to DISC.
4. **Kenwood CAT keyed with no error and the radio did not tune** (§7). The dial was set and read
   back through Kenwood CAT, so the link worked. Every Tune press was `ok` and `ptt: keyed`
   within 2–5 ms. Aether keys a Kenwood with a plain `TX;`. The other session is reviewing it.
5. **Small things** (§8): the mobile's rules check judged 80 m while it ran on 40 m (the
   declared dial, as in reports 18 and 19); Tune was refused for a busy channel six times at the
   mobile; `air_account.py` calls an open window the slave rightly left empty a "forfeit"; the
   `keying` late line fires for a start inside an open window; the mobile waited 403 ms on
   writing its recording while it transmitted.

## 1. The sessions

Times UTC. *Call* is from the mobile's `connect` to `connected`; *body* is from the house keying
its acceptance to the end of the mobile's DISC, on the house's clock. *House read* is the
mobile's data frames as the house read them (the call in brackets); *mobile read* the house's
data frame at the mobile and, in brackets, what the mobile reported in its control frames.

| # | start | band | tries | call | body | house read | mobile read | session no. |
|---|---|---|---|---|---|---|---|---|
| 1 | 18:39:04 | 80 m, 3.590 MHz | 4, both-ways | 42.5 s | 12.6 s | +1.8 … +2.9 (+2.0 / +6.1) | +9.2 (+10.4 … +11) | 55 |
| 2 | 18:41:52 | 40 m, 7.101 MHz | 3, OFDM | 9.2 s | 18.2 s | −0.2 … +0.7 (+1.0) | +11.4 (+10.0 … +10.3) | 13 |
| 3 | 18:55:3x | 80 m, 3.590 MHz | ≥ 1, both-ways | — | 17.2 s | +3.1 … +4.3 (+3.6 / +4.6) | — (+10.1 … +11) | 55 |

* The band is the house's dial (its sidecar's `frequency_hz` and dial readings) and the mobile's
  RadioPost prompts. The mobile cannot set or read its dial with RTS keying; the author changed
  band by hand and pressed Tune before each session.
* The call's tries follow ADR-0126. Before sessions 1 and 3 the mobile's daemon had just been
  restarted. What it heard lately is kept in memory only (`Station::heard_lately`), so it called
  as to a station not heard: both-ways tries, 6.6 s keyed. In session 2 it had heard the house at
  +9 dB two minutes before, so it sent short OFDM tries.
* The mobile's daemon drew session number 55 for its first call after each restart. The engine's
  draws are seeded from the callsign (ADR-0098), so that is expected. It did no harm here.

## 2. The granted slots

`two_sides.py`, on the two two-sided sessions. A signal both stations heard placed them weakly
(r 0.41 and 0.15), so the figures are the loops' offset's (`--from-loops`). The loop latency summed
to 116 ms both times, about 58 ms each way, as in report 19.

| session | handovers | least clear air | overlaps | house answers, late | least margin |
|---|---|---|---|---|---|
| 1 | 9 | 0.575 s | 0 | 71–92 ms | 258 ms |
| 2 | 9 | 0.564 s | 0 | 72–84 ms | 266 ms |

`air_account.py --sidecars`: no forfeit and no late drop at either end by the engines' counters.
It labels one master transmission of session 2 a "forfeit": the idle header at 18:42:12.85 that
granted an open window the house had nothing to put in. That is the tool's word, not a fault
(§8). The house keyed in 1 ms (CAT) and rendered in 10–70 ms; the mobile keyed in 0 ms (RTS).

Session 3, from the house's clock alone: its three answers keyed 0.54–0.57 s after the end of
the mobile's frames, as in sessions 1 and 2, and every frame of the mobile's it recorded decoded.

## 3. The calls and the scan

### 3.1 Session 1: found at the last try

The mobile's four both-ways tries against where the house's radio was (both logs; the two PCs'
clocks agree to about 30 ms):

| try | keyed | the house was on | what the house made of it |
|---|---|---|---|
| 1 | 18:39:04.4–11.1 | 14.107 MHz (there since 18:38:49.9) | nothing: another band |
| 2 | 18:39:15.6–22.2 | 14.107 MHz | nothing |
| 3 | 18:39:26.8–33.4 | 7.101 MHz from 26.76, then 3.590 MHz from 31.40 | the end of the tone part, heard at 31.9 (the passband "peaked" at 32.78); a linger began |
| 4 | 18:39:38.5–45.1 | 3.590 MHz | OFDM part decoded 39.977 at +2.0 dB, tone part at 45.273 at +6.1 dB; accepted, keyed 46.156 |

The call lasted 49 s (`call_s`), so four tries, and the fourth was the last. The house spent
36.9 s on 14.107 MHz before it. A dwell is 4–5 s, and a linger 19.3 s after anything at
confidence 1.1 or more that does not decode. So the house had heard something on 20 m twice in a
row, which the log does not say. The only line is the busy detector's, on 14.107 MHz from 18:39:19.4 to
21.4: "energy over the threshold for the attack". Had the house stayed on 20 m one dwell longer,
the call would have failed.

The scan did what ADR-0129 §11 and §12 say. It arrived on 80 m 1.5 s before try 3 ended, heard
the try's last sync block from its later blocks, lingered, and caught try 4.

**Session 2**: three OFDM tries 3 s apart. The house moved from 14.107 to 7.101 MHz at 55.748,
180 ms after try 2 began, and decoded try 3 at 18:42:00.153 (+1.0 dB). **Session 3**: the house
moved to 3.590 MHz at 18:55:40.490 and decoded a both-ways try's OFDM part 5.9 s later. How many
tries came before it waits for the mobile's side.

### 3.2 The afternoon: where the scan spent its time

All 12 205 dial moves in the house's log (00:56–18:57Z; three dials from 13:46:10Z). The house
had no other session today.

| dial | share of the time since 13:46 | visits | median visit | visits over 30 s | longest | next visit over 54 s away |
|---|---|---|---|---|---|---|
| 7.101 MHz | 36 % | 1 053 | 4.6 s | 27 | 124.5 s | 7.3 % of the time |
| 3.590 MHz | 26 % | 1 053 | 4.5 s | 4 | 38.0 s | 8.9 % |
| 14.107 MHz | 38 % | 1 053 | 4.5 s | 14 | 214.1 s | 1.6 % |

The last column is the share of the afternoon in which a call begun then would have met no visit
to that dial for 54 s, the time the gateway publishes. It is a floor on a call's chance of going
unheard, not the chance itself: a visit must also overlap a try.

Most of 3.590 MHz's long absences are one half hour. **From 15:26:38 to 15:55:41 the house went to
14.107 MHz eight times and stayed 204.8, 206.7, 208.2, 214.1, 205.3, 208.6, 207.4 and 183.9 s**,
with one 4–5 s visit each to 7.101 and 3.590 MHz between. In the hour 15:00–16:00Z the scan
gave 20 m 62 % of its time. The house's log for those minutes is the busy detector flipping every
few seconds: "the passband is peaked" and "energy over the threshold", with the radio's gain
6–14 dB down. Other stations were on 14.107 MHz.

Why the visits stop at about 205 s: a linger is extended by every frame announced or reported at
confidence 1.1 or more that does not decode as another station's (`scan_arriving`, `scan_heard`
in `station/scan.rs`). A visit ends only when no linger is running, or when it has lasted
`cap_s`, the longest call Aether makes (fifteen both-ways tries) plus a dwell. On a band carrying
other modes, the house keeps hearing things it cannot decode, and the visit runs to that cap. The
published 54 s (`must_last`: a round of dwells and two lingers) assumes at most two lingers in a
round.

ADR-0129 measured the notice threshold against noise (1.07 in 20 minutes, §11). It was not
measured against other stations' signals, which is what 14.107 MHz carried.

**What the logs cannot say**: which frames the house heard on 20 m, at what confidence, or of
which family. A scan writes no line when it lingers, and there was no recording then.

**Options, for the author** (none built):

* **First, an instrument**: log, once per visit, why the scan stayed past its dwell (the
  frame's family, confidence and time). That would answer the question above from the next day's
  logs. It changes no behaviour.
* Cap a visit's lingers at what `scan_s` assumes (two lingers), so the published time holds. A
  slow caller's later tries are then caught on the next round.
* Let only what could be a call keep a linger going: the families a call is sent in (the robust
  OFDM data frame and tone-24), at its expected lengths.
* Take 14.107 MHz out of the scan. Each dial costs every caller 5 s of `scan_s`, and a crowded dial
  costs far more.

### 3.3 The client's call time

The gateway's log at 13:46:10: "scanning 3 channels, 7.101, 3.590, 14.107 MHz: a call must keep
trying 54 s to find the gateway", then "registered again, since what it listens on changed". The
client's `connect` at 18:39:04 and 18:41:52 asked `call_s` 49, the figure the gateway published
with two channels on 11-17 and 11-18. The client's dial prompts named only 7.101 and 3.590 MHz.
The client was working from an older record of the gateway. With 49 s it made four both-ways
tries (42.5 s), and the fourth was the one heard. **For RadioPost**: the client refreshes a gateway's
record (channels and call time) before calling it, or the gateway's record is not cached that
long.

## 4. Where the time went in a session

Each session is the same exchange. The house's sidecars, on its clock (session 2):

| at | who | what |
|---|---|---|
| 30.87 | house | acceptance |
| 32.13 | mobile | header, 2 data frames: the 163-byte opening |
| 35.25 | house | answer, nothing else |
| 36.45 | mobile | idle header: an open window |
| 37.45 | house | its 52-byte reply, in the open window |
| 39.75 | mobile | header, 1 data frame: the 5-byte close |
| 41.82 | house | answer |
| 43.02 | mobile | idle header: an open window |
| 48.66 | mobile | DISC, after the window ran out |

**The reply misses the answer.** The house's answer to the opening is decided as soon as the
opening's frames are reported (ADR-0069's render-ahead), and the opening is delivered to RadioPost
Gateway at the same moment. Its reply came about 0.23–0.32 s later (from the opening's last frame's
end plus the ~0.15 s decode): the house's `send` at 18:39:50.347, 18:42:05.127 and 18:55:55.291,
keyed answers at 50.486, 05.266 and 55.359. The
transmission was decided 142–153 ms before its key, so the reply missed even the latest
moment the answer could have been decided, by about 3, 14 and 85 ms. It went 2.2 s later in the
master's next open window. That happened in every session. Saving it needs both sides: a
slave that has just delivered data to a program decides its answer at the last moment, not when
final; and a program that replies within about 0.2 s. Neither alone would have caught these three.
The same exchange is in every request and reply through Aether, B2F turnarounds included.

**The DISC waits out an open window.** In session 2 the mobile's log reads:

```
18:42:12.626 sent: 5 bytes of text delivered
18:42:12.846 ptt: keyed                       (the idle header: an open window of 4 s)
18:42:13.116 control: disconnect {}: ok
18:42:18.483 ptt: keyed (Disconnecting)       (the DISC)
```

A master with nothing to send grants an open window 0.22 s after its data is acknowledged, and
cannot key again until the window has passed. RadioPost asked to disconnect 0.49 s after the
delivery, in session 1 after 0.19 s, which was in time. Session 3 matches session 2 on the house's side
(the idle header at 46.93, the DISC at 52.57). Each time it cost 5.6 s. **For RadioPost**: ask
`disconnect` together with the last `send`. The engine sends what is queued first and the DISC
after it (ADR-0072's drain), and the `sent` event still comes before the session ends. Nothing
in Aether needs to change for that.

Without the two waits, the bodies would have been about 10.4, 10.4 and 9.3 s, against 12.6, 18.2
and 17.2.

## 5. What each direction carried (for the rate controller)

Every data frame was sent once and decoded alone at its first copy. `rung_reading.py` numbered
all of them as the senders recorded. Session 3 is from the house's side alone.

| session | direction | rung | frame | sends | never detected | decoded alone | by combining | SNR read | exchange bit/s |
|---|---|---|---|---|---|---|---|---|---|
| 1 | mobile → house | 9 | QPSK ⅓ | 2 | 0 | 2 | 0 | +2.6 | 340 |
| 1 | mobile → house | 7 | BPSK ⅓ | 1 | 0 | 1 | 0 | +1.8 | 104 |
| 1 | house → mobile | 8 | BPSK ½ | 1 | 0 | 1 | 0 | +9.2 | 123 |
| 2 | mobile → house | 9 | QPSK ⅓ | 2 | 0 | 2 | 0 | +0.3 | 341 |
| 2 | mobile → house | 7 | BPSK ⅓ | 1 | 0 | 1 | 0 | +0.2 | 105 |
| 2 | house → mobile | 8 | BPSK ½ | 1 | 0 | 1 | 0 | +11.4 | 124 |
| 3 | mobile → house | 11 | QPSK ⅔ | 1 | 0 | 1 | 0 | +4.3 | — |
| 3 | mobile → house | 7 | BPSK ⅓ | 1 | 0 | 1 | 0 | +3.1 | — |
| 3 | house → mobile | 8 | BPSK ½ | 1 | — | — | — | — | — |

The bit/s are this session's few bytes over whole exchanges, not what the rungs can do.
`c1_replay.py` places every new-block transmission under C1's choice in its family: ADR-0130's
short transmissions at the most robust rung that carries them in as few frames. The opening went
as two rung-9 frames at +2 dB and as one rung-11 frame at +4 dB. Nothing here is evidence for
or against the controller beyond that: no copy, no loss, no climb.

## 6. Identifiers, and report 19's fixes on the air

* **Order and spacing.** In all three sessions the house (the called station) identified first,
  in the data signal as `KK4ODA`, in the 1.1 s transmission that answered the mobile's DISC. The
  mobile followed 0.54–0.55 s after the house released, as `WD5EMA`, and the house decoded it
  ("ident: identified WD5EMA").
* **The identifier names the session's callsign** (report 19 change 1, ADR-0075 §7). At the
  mobile, session 1: RadioPost's claim ended at 18:40:00.151 ("no longer answering for WD5EMA"),
  and at 18:40:01.454 "the closing identifier went out as WD5EMA, in the data signal". The same
  in session 2. Before the fix this was refused.
* **A both-ways try teaches no gap** (report 19 change 3, ADR-0054 §5). In sessions 1 and 3 the
  house heard the try's tone part 5.3 s after its OFDM part, before the acceptance went on the
  air, and wrote "answer gap for WD5EMA now 475 ms: its call was accepted the first time". Over
  the three sessions the gap eased 500 → 475 → 450 → 425 ms, 25 ms a clean session.

## 7. The Kenwood CAT attempt (for the other session's review)

From the mobile's kept run logs (`logs/aetherd-20261121-18*.log` in its 18:41 zip), times UTC.
The Setup form first saved CAT with the Yaesu protocol (18:31:53, a run of 17 s, nothing keyed),
then Kenwood (18:32:10), on COM9 at 38 400 Bd:

```
18:32:10.569 ptt: KK4ODA-2 keying via Kenwood CAT on COM9
18:33:16.891 control: frequency.set {"hz":7101000}: ok
18:33:16.891 tune: dial set to 7101000 Hz           (RadioPost: "tuned the radio to 7.101 MHz")
18:33:48.944 control: tune {"duration_s":10}: ok
18:33:48.948 ptt: keyed
18:33:51.335 control: tune {"duration_s":0}: ok     (stopped by hand, 2.4 s)
18:33:51.336 ptt: released
18:34:01.573 ptt: keyed  … 18:34:02.559 released    (tune, stopped after 1.0 s)
18:34:06.230 ptt: keyed  … 18:34:06.956 released    (tune, stopped after 0.7 s)
18:34:39.651 control: shutdown {}                   (restarted 18:34:43, still Kenwood CAT)
18:35:09.893 ptt: keyed  … 18:35:15.390 released    (tune, stopped after 5.5 s)
18:35:41.009 control: config.set {… "ptt.kind":"serial","ptt.line":"rts" …}
18:35:59.597 ptt: keyed  … 18:36:00.281 released    (RTS, stopped after 0.7 s)
18:36:06.968 ptt: keyed  … 18:36:08.232 released    (RTS, 1.3 s)
18:38:05.053 ptt: keyed  … 18:38:15.259 released    (RTS, the whole 10 s)
```

What the logs say:

* **The CAT link worked.** The dial was set to 7.101 MHz, and since a Kenwood says nothing back
  to `FA…;`, the daemon read it back to check, and it had moved.
* **Every Tune press under Kenwood CAT was `ok` and keyed within 2–5 ms**, with no error, no
  `keying:` warning and no fault, four times across two runs.
* **Nothing in Aether's log can say whether the TS-480 transmitted, or from which audio input.**
  The daemon reads no S-meter or power outside a recording.

For the review, not checked here: Aether keys a Kenwood with a plain `TX;`
(`CatProtocol::Kenwood.keying`, `core/aetherd/src/ptt.rs`). Kenwood radios whose CAT tells the
microphone from the data input (`TX0;`/`TX1;` on the TS-590 and its kin) key from the
microphone on a plain `TX;`. The DigiRig's audio goes to the TS-480's data input, so a radio
keyed that way would send no tone for the tuner. That fits "would not tune", and RTS, the data
port's own PTT, tuned at 18:38:05. Whether the TS-480 takes `TX1;` is for its CAT manual. Also
from these logs: the first CAT save carried the Yaesu protocol, for a station whose operator
section names a TS-480; the author changed it to Kenwood 17 s later.

## 8. The stations

* **Capture.** The house: 78 skips since its start at 00:56Z, 3.2 s in all, the largest 198 ms,
  all padded (ADR-0089), none during a session. The mobile: two (10 and 17 ms) at 18:36, none in
  a session. All five recordings are within 60 ms of their wall-clock spans (report 19's session 4
  was 153 ms short), so no unreported loss either.
* **Slow passes.** The house: 17 since 00:56Z, the worst 1.32 s at 13:49:04 with a sleeping
  thread 448 ms late (the machine, not the modem), none in a session. The scan's restart passes
  are counted apart now (report 19 change 5): 12 204, the slowest 1.9 s. The mobile: one, 403 ms
  at 18:42:11.07 in session 2, "waited on writing the recording 403 ms", while it transmitted, so
  it cost nothing. A stall that long while receiving would make the station answer late.
* **The busy floor.** `floor_trace.py` on all five recordings. The live floor and the trace agree
  within 0.1–0.6 dB (median). In every session the floor sat at the noise between frames: within
  1 dB of the 25th percentile of the blocks outside frames. The floor did not learn the other
  station this time. The busy detector's count, for its review: at the mobile, Tune was refused
  "the channel is busy" six times (18:36:14, 18:36:18, 18:38:25, 18:54:38, 18:54:59, 18:55:00).
  The busy state at 18:54:03 came on as "the passband is peaked (14.9 dB over its median)" and was
  still on at 18:55:00. There is no recording of those minutes, so what peaked is not known.
* **The rules check at the mobile** judged every transmission at its declared dial, 3.590 MHz
  (`dial_source` declared). That was right for sessions 1 and 3 and wrong for session 2, on 40 m.
  At 7.101 MHz USB the signal is inside the 40 m data segment, so nothing went out of bounds. This
  is reports 18 and 19 again, and CAT keying is what fixes it (§7).
* **The mobile's restarts.** The mobile's daemon started at 18:29:42 on the *VarAC default*
  profile left from 11-18 (keyed by the host program). The author loaded the RadioPost profile at
  18:31:14, then tried Yaesu CAT, Kenwood CAT and RTS: nine starts between 18:29 and 18:54, each
  the author's own. RadioPost reconnected after each.
* **Clipping**: none in either log; the house's transmissions peaked at −7.2 dBFS.

## 9. For the rate controller (P9-21)

* **Paths**: 80 m twice and 40 m once, early afternoon, the mobile to the house at 2300 Hz. The
  house read the mobile at −0.2 to +4.3 dB and the mobile read the house at +9 to +11 dB. Class
  good by what was measured: every frame decoded at its first copy at its reading.
* **Nothing to judge**: each direction sent one to three frames a session, all decoded alone (§5).
* `rung_reading.py --replay` refuses protocol 12 and later sessions by design (no advice to
  compare with). `c1_replay.py` is the replay for these.

## 10. Changes proposed, in order

| # | change | where | whose call |
|---|---|---|---|
| 1 | Log why a scan stays past its dwell (the frame's family, confidence, time), once per visit; then decide between capping lingers at `scan_s`'s two, letting only call-shaped frames extend one, or fewer dials | Aether, `station/scan.rs` (ADR-0129) | the author's |
| 2 | The client refreshes a gateway's channels and call time before calling | RadioPost | the author's |
| 3 | The client asks `disconnect` with its last `send` | RadioPost | the author's |
| 4 | A slave that has just delivered data to a program decides its answer at the last moment, and the gateway replies within ~0.2 s; both, or neither | Aether, the engine (model first, ADR-0069); RadioPost | the author's |
| 5 | Kenwood keying: `TX1;` (the data input) where the radio has it | Aether, `ptt.rs` | the Kenwood review, in hand |
| 6 | `air_account.py` names an open window left empty as such, not a forfeit; the `keying` late line skips starts inside an open window | Aether, tools and `station.rs` | the author's; small |
| 7 | The mobile's side of session 3 | the mobile | the author: upload asked for |

The rate controller (§9) and the busy detector (§8) wait for their reviews, by the author's
rules.
