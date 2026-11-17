# RadioPost mail on 40 m at 2300 Hz, the mobile to the scanning gateway (2026-11-17)

Nine sessions on the afternoon of 2026-11-17 (19:56–20:29Z, local time 14:56–15:29), from the
mobile's RadioPost client to the home RadioPost Gateway, which was scanning two channels. Both
stations uploaded their files by debug mode, with audio. The author's notes: the strongest link
yet, excellent speeds, every gateway session answered within two calls and ended cleanly, QRM at
times (VARA, PACTOR or Olivia by ear), and a P2P attempt that failed.

* **Who**:
  * **The mobile**: station `KK4ODA-2` (TS-480, 65 W, screwdriver, keyed by RTS on the DigiRig's
    COM9, no CAT), beta.161. RadioPost client 0.1.0-beta.23, calling as `WD5EMA` (eight sessions)
    and `KK4ODA` (one).
  * **The gateway**: `KK4ODA-1` (FTDX10 on Yaesu CAT, 50 W, T2D), beta.162. RadioPost Gateway
    0.1.0-beta.25, scanning 7.101 and 3.590 MHz from 17:12Z (`call_s` 49 s).
* **The files**, none committed: twelve debug zips, `20261117-195839Z KK4ODA-1 …` to
  `20261117-202947Z KK4ODA-1 …`. The mobile's side of the last two sessions (20:27, 20:28) was
  not uploaded.
* **Read with** `tools/two_sides.py`, `tools/rung_reading.py --inbox`, `tools/c1_replay.py
  --inbox`, `tools/passband_witness.py` and `tools/ladder_table.py --bandwidth 2300`.

## 0. Summary

**What happened.** Seven gateway sessions delivered everything: six messages from the mobile
(1.2–8.6 kB of text, which RadioPost compressed to 0.7–1.1 kB on the air, and a 51.9 kB binary)
and three messages waiting at the post office for `KK4ODA`. The binary took 451 s from connect to
delivery, about 920 bit/s, on a path the gateway read at +3 to +4 dB. Two short sessions at
20:27 and 20:28 were the P2P attempts. RadioPost Gateway welcomed the client as it welcomes any
client, and the client, expecting another client's proof, hung up both times.

**The granted slots held** (§2). No overlaps in seven two-sided sessions; the least clear air was
0.521 s; every gateway answer reached the air 55–154 ms after its slot opened.

**Four findings, ranked by what they cost.**

1. **The gateway's replies went in frames the mobile could not read** (§4). Each reply was one
   small block, about 40 bytes, sent in a 64-QAM frame (rungs 17–19). All nine such first copies
   failed at the mobile, and each block got through only on its third copy, by combining. Every
   OFDM data frame at 2300 Hz takes 1.054 s whatever its rung, so a slower rung would have carried
   the same bytes in the same airtime. It cost about 10 s per block and 20 s of a typical 30–40 s
   session: half of each short session. This is the rate controller's ground, so it waits on the
   author (CLAUDE.md, *The rate controller waits for the air*).
2. **The scan stopped after the 51.9 kB session and did not come back** (§3). At 20:14:50.881,
   5 ms after the gateway's closing identifier released the key, the scan asked the FTDX10 for
   3.590 MHz and the radio did not answer the read-back. `move_scan` drops a dial for good on any
   error, so the scan, left with one dial, ended. The gateway sat on 7.101 MHz for the rest of the
   day while its post-office record still said it scanned two channels.
3. **A P2P call to the gateway's callsign reaches the gateway program** (§5). RadioPost Gateway
   holds `KK4ODA-1`, so a station-to-station call to it gets a gateway's welcome, which carries no
   client's proof, and the calling client ends the session. For P2P with the home station, a
   RadioPost client there must listen under a callsign of its own.
4. **The mobile cannot tune, and its rules check judged the wrong band** (§6). Keyed by RTS with
   no CAT, the mobile's Aether cannot set or read the dial. RadioPost logged "tune the radio to
   7.101 MHz: this modem cannot" at every call. The declared dial was still 3.590 MHz, so the
   rules check judged every transmission as 80 m. The signal itself, 7.1013–7.1037 MHz, was inside
   the 40 m data segment.

The receive-filter warnings the author saw at both stations are another session's work.

## 1. The sessions

Times UTC. *Call* is from the mobile's `connect` to `connected`; *delivered* is from `connected`
to RadioPost's "delivered" or "received". *App* bytes are what RadioPost handed over; *air* bytes
are what the link acknowledged after compression.

| start | caller | what | tries | call | delivered | session | app bytes | air bytes | gateway read | mobile read |
|---|---|---|---|---|---|---|---|---|---|---|
| 19:56:48 | WD5EMA | first message | 1 (both-ways) | 8.0 s | 38 s | 48.3 s | 2 124 | 2 144 | +3.7 | +14.4 |
| 20:00:03 | WD5EMA | text file 1 | 1 | 2.6 s | 27 s | 31.5 s | 1 240 | 746 | +6.1 | +15.8 |
| 20:01:17 | WD5EMA | text file 2 | 1 | 2.6 s | 31 s | 35.9 s | 2 464 | 907 | +5.6 | +15.1 |
| 20:03:10 | WD5EMA | text file 3 | 2 | 5.6 s | 26 s | 30.4 s | 4 502 | 976 | +2.0 | +14.0 |
| 20:05:22 | WD5EMA | text file 4 | 1 | 2.6 s | 36 s | 39.7 s | 8 551 | 1 144 | +5.4 | +16.7 |
| 20:06:58 | WD5EMA | 50 kB binary | 3 | 12.6 s | 451 s | 455.7 s | 51 917 | 51 729 | +4.2 | +14.3 |
| 20:17:10 | KK4ODA | mail check: 3 received | 1 | 2.6 s | 13 s | 24.3 s | 491 in | — | +4.2 | — |
| 20:27:34 | WD5EMA | P2P attempt | — | — | refused | 25.2 s | 344 / 67 | — | +3.4 | — |
| 20:28:25 | WD5EMA | P2P attempt | — | — | refused | 19.6 s | 344 / 67 | — | +4.9 | — |

*Gateway read* is the last frame the gateway read from the mobile; *mobile read* is the mobile's
(sessions' histories). The two directions differ by about 10 dB. The mobile's 65 W into a
screwdriver arrives much weaker than the gateway's 50 W into a T2D.

## 2. The granted slots

`tools/two_sides.py` on each of the seven two-sided sessions:

| session | handovers | least clear air | overlaps | gateway answers, late | least margin |
|---|---|---|---|---|---|
| 19:56 | 9 | 0.546 s | 0 | 76–96 ms | 254 ms |
| 20:00 | 8 | 0.546 s | 0 | 76–96 ms | 254 ms |
| 20:01 | 8 | 0.562 s | 0 | 55–90 ms | 260 ms |
| 20:03 | 6 | 0.521 s | 0 | 101–121 ms | 229 ms |
| 20:05 | 10 | 0.544 s | 0 | 78–98 ms | 252 ms |
| 20:07 | 56 | 0.544 s | 0 | 70–154 ms | 196 ms |
| 20:17 | 5 | 0.555 s | 0 | 77–97 ms | 253 ms |

The loop latency was a median of about 50 ms from the mobile and 65 ms from the gateway. One 10 ms
step in the offset at 407 s of the long session is a capture skip at the gateway (ADR-0089 pads
it). Every session ended with both identifiers in ADR-0075's order: the gateway's (the called
station) first, the mobile's after it.

## 3. Calls and the scan

**The calls.** The mobile runs beta.161, which has no `call_s`, so RadioPost could not ask for a
call that lasts the scan (RP-4 §4 asks only a modem whose capabilities offer it). Each call was
Aether's own: a both-ways try the first time (the gateway not heard lately), then fast OFDM tries
on a station heard strong. Five calls were answered on the first try, one on the second, and the
20:06:58 call on the third, 9.5 s in, most likely after the gateway hopped back from 3.590 MHz.
On a path this strong a plain call was enough. A weak one would want beta.162 at the mobile, so
the call lasts the scan.

**The scan stopped.** The gateway's log at the end of the 51.9 kB session:

```
20:14:50.875 identified: the closing identifier went out as KK4ODA
20:14:50.876 ptt: released
20:14:50.881 scan: the radio would not go to 3.5900 MHz: keying: COM6: the radio did not answer after the frequency was set
20:14:50.882 scan: radiopost-gateway 0.1.0-beta.25 stopped scanning: fewer than two of its dials are left that the radio will take
```

The move came 5 ms after the key went down. A Yaesu says nothing back to `FA…;`, so
`CatPtt::set_frequency_hz` reads the dial back to check it moved, and the FTDX10, just out of
transmit, did not answer in time. `Station::move_scan` (`station/scan.rs`) treats every error
the same: it removes the dial for good, and below two dials it ends the scan. That is right for
"the radio reads another frequency" (a locked VFO, a band the radio has not got). It is wrong for
"the radio did not answer", which says nothing about the dial. The radio stayed on 7.101 MHz
(the later sessions' dial readings). After the five earlier sessions the scan's next move
followed the identifier the same way and the radio answered, so this is a matter of timing, and it
will happen again.

RadioPost Gateway logged "the scan ended" but did not register again, so its post-office record
went on offering 3.590 MHz and a 49 s call to callers.

**Proposed** (Aether, a bug: the code does not do what ADR-0129 meant):
- a read-back with no answer keeps the dial, and the scan tries it again on its next turn;
- a dial goes out only when the radio reads another frequency, or after three moves in a row with
  no answer.

This removes nothing; it narrows what drops a dial. It needs a machine with the Rust toolchain;
this PC has none. **For RadioPost**: when Aether ends the scan, the gateway registers again with
what it now listens on.

## 4. What each direction carried

**The mobile's data** (`rung_reading.py`), read at the gateway at +2.5 to +3.9 dB:

| session | rungs | blocks | delivered by the first copy | by a later copy | by combining | exchange bit/s |
|---|---|---|---|---|---|---|
| 19:56 | 10 (QPSK ½) | 16 | 16 | 0 | 0 | 713 |
| 20:00 | 11 (QPSK ⅔) | 5 | 5 | 0 | 0 | 709 |
| 20:01 | 10 | 7 | 7 | 0 | 0 | 576 |
| 20:03 | 10 | 8 | 8 | 0 | 0 | 703 |
| 20:05 | 10 | 9 | 9 | 0 | 0 | 650 |
| 20:07 | 9–12 (QPSK ⅓ to 8PSK ½) | 276 (302 frames sent) | 252 | 11 | 13 | 950 (rung 11), 1 156 (rung 12) |
| 20:17 | 11 | 2 | 2 | 0 | 0 | 466 |

Nothing was lost and no frame went undetected. In the long session C1 climbed from rung 9 to 11
in the first minute, lost a try of 12 at 88 s, and took 12 at 234 s. A try of 14 at 260 s failed
(0 of 3), and one of 12 at 389 s lost by 13 %. C1 rated the rung above better for 22 % of the
exchange time but held off it while a lost try had doubled its wait (`c1_replay.py`). The best
measured neighbour would have delivered 13 % more over the rungs used. That is the review's
familiar slow climb, not a new loss.

**The gateway's data**, read at the mobile at +13 to +19 dB (control frames). RadioPost Gateway
sends little, about 87 bytes a session in two blocks: its greeting and its receipt. C1 put each
block in one frame at the rung the mobile's reading allows:

| session | rung | frame | first copy | copies to deliver |
|---|---|---|---|---|
| 19:56 | 16, 17 | 16-QAM ¾, 64-QAM ⅔ | decoded, failed | 1, 3 |
| 20:00 | 18, 17 | 64-QAM ¾, ⅔ | failed, failed | 3, 3 |
| 20:01 | 18, 17 | | failed, failed | 3, 3 |
| 20:03 | 16, 15 | 16-QAM ¾, ⅔ | failed, decoded | 3, 1 |
| 20:05 | 18, 17 | | failed, failed | 3, 3 |
| 20:07 | 16, 17 | | failed, failed | 2, 3 |
| 20:17 | 19 | 64-QAM ⅚ | failed | 3 |

At rungs 17–19 none of nine first copies decoded alone, and every block was delivered by
combining its third copy with the first two. At rung 16, one first copy in three decoded (the
other two blocks took two and three copies); at rung 15, one in one. The failed frames themselves read 13.6–16.7 dB at the mobile. The 2300 Hz table
(`ladder_table.py`) puts 64-QAM ⅔ at 13.9 dB on AWGN and 21.3 dB on ITU Good: C1's guess for an
unmeasured rung (the AWGN curve 1 dB under the reading, ADR-0111) said yes, and the path said no.

The waste is that it bought nothing. Each block was a few dozen bytes, and every OFDM data frame
on the 2300 Hz air takes 1.054 s whatever its rung. A 40-byte block in a QPSK ⅓ frame (92 bytes)
takes exactly the air it took in 64-QAM. That rung is about 14 dB less demanding, and the mobile
decoded every control frame the gateway sent. Each block that went three copies cost two more
exchanges of the gateway's slot, about 10 s. The short sessions ran 24–48 s, and about 20 s of
most of them was this.

**Not the mobile's radio.** `passband_witness.py` on the gateway's 59 OFDM frames in the mobile's
20:07 recording reads the TS-480's response flat within ±1 dB from 500 to 2300 Hz, 2.1–2.4 dB
down at 300–400 Hz and 2.4–2.9 dB down at 2500–2600 Hz. Nothing at the mobile cut the signal.
The path is not AWGN, and the reading does not say so.

**Proposed** (the rate controller's ground; the author's call): when what a transmission carries
fits in fewer or the same frames at a slower rung, send it at the most robust such rung, as
ADR-0085 already does for a re-encode. Never drop into the floor's tone family under an OFDM
choice. On these sessions it removes every three-copy block, and it changes nothing for a
transmission that fills its frames. It does not decide which rung a full transmission should use,
so it does not take sides between the paths the review must weigh. CLAUDE.md asks for the same
common-case loss in several sessions before anything changes. This one is in all seven, at the end
of every RadioPost exchange.

## 5. The P2P attempts

The mobile's RadioPost client called `KK4ODA-1` station to station twice, as `WD5EMA`. Aether at
the gateway gave the call to the program holding `KK4ODA-1`, RadioPost Gateway, which took the
client's 344-byte `HELLO` and answered with 67 bytes. The mobile hung up 9 and 3 s later. The
gateway's log says only "WD5EMA called" and "the session with WD5EMA is over".

A station-to-station `HELLO` is the same as a gateway call's, so the gateway welcomed the client as
it welcomes any client. The calling client, though, checks that the station it called proves itself
as a client before it takes or sends anything (RP-3 §10, `check_station` in `client.rs`). A
gateway's welcome carries no such proof, so the client ended the session. The mobile's log for
these two sessions was not uploaded, so its words are not in hand, but nothing else in the two
logs fits. That a client will not hold a P2P session with a gateway is by design. What is missing:

* **For P2P with the home station**, a RadioPost client there must listen (*Listen*) under its
  own callsign, for instance `KK4ODA`, beside the gateway on the same Aether (ADR-0106). Calling
  the gateway's callsign can never reach it.
* **Neither side said why at the gateway.** From RadioPost Gateway's side a client simply left. A
  client that ends a station call because the other station is a gateway could say so in plain
  words ("KK4ODA-1 is a gateway: call its operator's own callsign for P2P").
* **The client's call box has no channel or bandwidth for a station call** (the author's note):
  `dial_hz` and `call_s` are sent only for a gateway call, and `opts.bandwidth` is never set from
  the panel.

The 20:17 call was not P2P. The client, switched to `KK4ODA`, called the gateway as a gateway and
received three messages waiting at the post office.

## 6. The mobile's radio setup

`diagnostics.json` at the mobile: `[ptt] kind = "serial"`, RTS on COM9, `status.can_tune =
false`, `[regulatory] dial_hz = 3590000` (declared). Two consequences:

* RadioPost could not tune the radio to the gateway's channel. The author's request to tune when
  the channel is chosen, so the radio and the antenna tuner can be set first, needs a modem that
  can set the dial.
* The rules check judged every transmission at 3.590 MHz while the radio was at 7.101 MHz. At
  7.101 MHz USB the 2300 Hz signal occupies 7.1013–7.1037 MHz, inside the 40 m RTTY and data
  segment, which ends at 7.125 MHz, so nothing went out of bounds. But the check was not looking.

COM9 is also the DigiRig's CAT port. `[ptt] kind = "cat"` with the Kenwood protocol on COM9 keys
by `TX;`/`RX;`, reads the dial and sets it. That fixes both. Setup step 2.

## 7. For the rate controller (P9-21)

* **Path**: 40 m, afternoon, the mobile to the home station. Class `good` by what it measured: the
  mobile's QPSK frames decoded at AWGN-level readings, the readings of the gateway's control frames
  scattered 12–19 dB within a minute, and 64-QAM failed where the AWGN curve said it would pass.
* **The common-case loss** is §4's short replies at 64-QAM, in seven sessions of seven.
* **The climb** in the long session (§4) adds one more session to the slow-climb evidence; nothing
  new.
* **The field log.** The nine sessions are in `field/LOG.md` with their goodput as `?`.
  `field_ingest.py` used to divide a sidecar's `bytes_delivered` by the recording's length, but that
  counter runs for the daemon's whole life. It read 57 646 bytes at the end of the 51.9 kB session
  and 57 880 at 20:17, so every plain session but the first since a daemon start came out too fast
  (20:28's 67 bytes read as 10 317 b/s). It no longer guesses. Earlier plain-session rows carry the
  old figure: 11-12's 2300 Hz session reads 1611 b/s, where 22.6 kB in 183 s is 988. Table §1 has
  this day's real numbers from the sessions' histories.

## 8. Changes proposed, in order

| # | change | where | whose call |
|---|---|---|---|
| 1 | A scan keeps a dial the radio did not answer for; drops it only on a wrong reading or three misses in a row | Aether, `station/scan.rs` | a bug fix; needs a Rust machine |
| 2 | A transmission that fits a slower rung's frames in the same count goes at the most robust such rung | Aether, the engine (model first) | the author's (the rate controller rule) |
| 3 | The mobile keys by Kenwood CAT on COM9 | the mobile's Setup | the author |
| 4 | RadioPost Gateway registers again when Aether ends its scan; the client says plainly when the station it called for P2P is a gateway | RadioPost | the author |
| 5 | The client's call box: *Call* stands out, a chosen channel is tuned with a tuner tone before *Call*, a station call gets a channel and a bandwidth | RadioPost | the author (his notes) |
| 6 | The mobile updates to beta.162, so a call lasts the scan | the mobile | the author |
