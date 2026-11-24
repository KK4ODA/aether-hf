# Winlink Express P2P on 40 m at 500 Hz, portable to home (2026-11-24)

One session at 18:42:54Z (local 13:42), on 40 m at 500 Hz: Winlink Express at the portable
calls Winlink Express at home in a *Vara HF P2P* session. It is the first P2P session on the air
run by a Winlink program through Aether since the *external programs* build (beta.173, beta.174).
Both stations uploaded the session by debug mode, with audio. The author's note: the external
programs interface worked great.

* **Who**:
  * **The portable**: station `KK4ODA-2` (Lab599 TX-500, 10 W, linked EFHW, grid EM73, a
    22-core PC, a USB sound card), beta.174, profile *Winlink Express Aether TX-500 P2P 500Hz*.
    Winlink Express as `WD5EMA` keys the radio on Aether's `PTT ON` (`[ptt] kind = "host"`).
    Aether identifies in the data signal.
  * **Home**: `KK4ODA-1` (FTDX10, 50 W, T2D, EM73UU, the 4-core mini PC), beta.174, profile
    *Winlink Express FTDx10 Aether P2P 500Hz*. Winlink Express as `KK4ODA`, listening, keys the
    radio on `PTT ON`. Aether identifies in Morse.
* **The files**, none committed: the debug zips `20261124-184444Z KK4ODA-1 …` and
  `20261124-184444Z KK4ODA-2 …` (recordings `20261124-184255_KK4ODA_WD5EMA` and
  `20261124-184304_WD5EMA_KK4ODA`, both with audio), fetched through temporary copies.
* **Read with** `tools/two_sides.py` (with and without `--from-loops`), both daemons' logs with
  the host trace, the sidecars, the diagnostic bundles and the portable's audio while keyed.

## 0. Summary

**What happened.** The B2F handshake with nothing to send either way: home's SID and prompt (68
bytes), the portable's SID and `FF` (73 bytes), home's `FQ` (3 bytes), then both Winlink
Express windows closed. The call connected at its first try in 10.9 s. The session ran 38.3 s
from `CONNECTED` to the end, and the contact took 57.4 s from the call's key to the end of the
last identifier. Every frame decoded at its first copy, both ways. Home read the portable at −5
to +1 dB SNR (in 3 kHz) and the portable read home at +5 to +9 dB: 50 W against 10 W makes
7 dB of that.

**The granted slots held** (§2). No overlap; the least clear air was 0.645 s; home's answers
reached the air 121 ms after their slots opened, 229 ms inside their limit. That is with
Winlink Express keying both radios, so Aether's `PTT ON` went through each program before the
radio keyed.

**The host interface did what Winlink Express expects** (§3). `BW500`, `MYCALL`, `LISTEN ON`,
`P2P SESSION` and `CONNECT` came back `OK`; home's `CONNECTED` waited for its acceptance's
`PTT ON` (ADR-0103); each program heard `PTT ON`/`PTT OFF` for every transmission; `DISCONNECTED`
came after each station's closing identifier. Both identifiers went, in ADR-0075's order, 0.72 s
apart, and home decoded the portable's identification frame (`ident: identified WD5EMA`).

**Findings, ranked by what they cost. Each is the author's call.**

1. **Two exchanges lost at the end of the session** (§4), about 10 of its 38 s. Home's Winlink
   Express wrote `FQ` 0.26 s after the portable's `FF` was delivered, 50 ms after the latest
   moment home's answer could take it, so `FQ` waited for the next slot: 5.1 s. Then home asked
   to disconnect with `FQ` still queued, and the portable had to acknowledge `FQ` in a header
   before it could send its DISC: 5.1 s more. The first is report 20's §4 again, now with
   Winlink Express, which no one can make reply faster. The second needs a DISC that carries
   the acknowledgement.
2. **A false *the radio may not be transmitting* at the portable** (§6). The TX-500 sends its
   own signal back to the sound card while it transmits, at −19 dBFS in the watch's band, while
   the band noise it hears reaches −20 dBFS. The keyed-audio watch took that for a radio still
   receiving and said so after three transmissions. Everything it sent was decoded at home.
3. **The portable's data went on the floor** (§5): two four-tone frames, 10.7 s, at −2 dB, where
   the first OFDM rungs (3 and 1.6 dB over their thresholds) would have taken three or four
   1.05 s frames. A data point for the rate controller's review; no change by the author's rule.

## 1. The session

Every transmission on one clock, seconds from the call's key (18:42:53.966Z by the portable's
log), with what the other station read. The two recordings are joined by
`two_sides.py --from-loops` (home's recording runs 9.4207 s behind the portable's; loop latency
94 ms each way, drift 16 ppm).

| from | to | who | what | read at the other end |
|---:|---:|---|---|---|
| 0.00 | 6.68 | portable | the call: robust OFDM part and tone-24 part | −3.2 and −4.8 dB |
| 7.23 | 10.70 | home | the acceptance, on the floor | +7.7 dB |
| 11.08 | 14.55 | portable | header, on the floor: a slot for home | −3.1 dB |
| 15.09 | 16.86 | home | answer and its 68-byte greeting, rung 11 (16-QAM ¾), one frame | +9.3, +8.8 dB |
| 17.23 | 31.44 | portable | header and its 73 bytes (SID, `FF`): two frames at rung 2 (tone4x100-51) | −2.6; −1.9, −2.4 dB |
| 31.99 | 32.70 | home | answer, nothing else (`FQ` came 50 ms too late) | +7.1 dB |
| 33.07 | 36.54 | portable | header with nothing: an open window | −1.1 dB |
| 37.08 | 38.85 | home | answer with the leave flag, and `FQ` at rung 5 (QPSK ½) | +8.8, +7.7 dB |
| 39.22 | 42.70 | portable | header acknowledging `FQ` | +1.4 dB |
| 43.25 | 43.96 | home | answer, nothing else | +8.1 dB |
| 44.34 | 47.81 | portable | DISC, on the floor | +0.1 dB |
| 48.35 | 53.19 | home | DISC_ACK, then its Morse identifier, `KK4ODA` | +5.3 dB |
| 53.95 | 57.41 | portable | its identification frame, `WD5EMA` | +1.2 dB |

The README's figure (`docs/images/air-40m-p2p.png`, drawn by `tools/air_p2p_figure.py`) shows
this table over both stations' receive audio.

The call was one both-ways try (ADR-0126: neither station had heard the other within 15
minutes). Home read its OFDM part at −3.2 dB, 1.3 dB over the 500 Hz ordinary control threshold
of −4.5 dB and under the 6 dB an OFDM acceptance needs, so it answered on the floor once the
tone part had ended. The portable's headers stayed on the floor because home reported −3 dB,
under that threshold plus 3 dB. Home's answers went in the ordinary family, which the portable
read at +7 to +9 dB.

## 2. The granted slots

`two_sides.py --from-loops`:

* **No overlap.** The air changed hands 11 times. Home to portable the gaps were 0.645–0.661 s;
  portable to home 0.821 s each time.
* **Home's answers** reached the air 121 ms after their slots opened, every time, 229 ms inside
  `not_after`. The earlier sessions on the air measured 64–197 ms. Home's DISC_ACK and its identifier are
  the slot the DISC granted (ADR-0075).
* **Nothing late.** `dropped_late` 0, `forfeits` 0, `silent_slots` 0 at both ends; the bundles'
  `keying.late` 0.

The envelope match on a signal both stations heard was weak this time (r 0.31 at 1–2.5 kHz, 0.27
at 0.3–2.7 kHz; the band was busy) and put the offset at −9.563 and −9.426 s. The first would
make the answers 263 ms late, the second agrees with the loops. The loops' offset holds still
over the session (−9.4209 to −9.4202 s), so it is the one used.

**Host keying.** With `[ptt] kind = "host"` Aether cannot time the key itself: the bundles say
*the radio takes 0 ms to key*, and the lead is the profile's 150 ms. Winlink Express then keys
the FTDX10 and the TX-500 by its own means. Every frame decoded at its first copy, so no
transmission lost its start to a slow key, and the answers stayed 229 ms inside their slots.

## 3. The host interface

What each Winlink Express said and heard (`[host] trace` is on at both ends; the program's data
is not in the logs):

* **Home** at 18:38:47: `PUBLIC ON`, `CWID ON`, `COMPRESSION ON`, `BW500`, `MYCALL KK4ODA`,
  `LISTEN ON`, each `OK`. Aether answered for `KK4ODA` from then on (*listen: answering calls to
  KK4ODA*). Station `KK4ODA-1`'s own callsign stayed unanswered, as ADR-0108 has it: the program
  speaks for the station.
* **The portable** the same with `MYCALL WD5EMA`, then at 18:42:53.9 `P2P SESSION` and
  `CONNECT WD5EMA KK4ODA`. The adapter set `keep_bandwidth` (ADR-0102): the call went at 500 Hz
  because the program said `BW500`.
* **The call at home**: `SN -3`, `PENDING`, then *CONNECTED waits for the acceptance's PTT ON*.
  The acceptance keyed 5.8 s later, once the try's tone part had ended, and the program heard
  `PTT ON`, `CONNECTED WD5EMA KK4ODA 500`, `PTT OFF`, the order it hears for its own calls
  (ADR-0103; `ACCEPTANCE_KEY_WAIT` is 12 s).
* **The portable** heard `CONNECTED WD5EMA KK4ODA 500` with `SN 8` as the acceptance decoded.
* **The end**: home's program sent `DISCONNECT` at 35.9 s (4.0 s after `FQ`, with `FQ` still
  queued: `BUFFER 3`) and again at 44.1 s, after `BUFFER 0`. Aether said *sending the last 9
  bytes (about 60 s). Abort closes now.*, sent them, and closed when the portable's DISC came.
  Each program heard `DISCONNECTED` after its own station's closing identifier (ADR-0075).
* **Learned gap**: home's answer gap for `WD5EMA` eased from 225 to 200 ms (*its call was
  accepted the first time*). The tone part heard after the OFDM part had been decoded did not
  count as a call heard again: report 19's fix held.

## 4. Where the time went

**`FQ` missed the answer.** The portable's second data frame ended at 31.44 s and was delivered
at about 31.60 s, when home's answer was decided (render-ahead, ADR-0069). Home's Winlink
Express wrote `FQ` at 31.86 s. The answer keyed at 31.99 s and is decided 182 ms ahead of its
key, so the latest it could have taken `FQ` was 31.81 s: missed by 50 ms. `FQ` went in the next
slot, the portable's open window, at 37.08 s: 5.1 s later. Report 20 §4 found the same with
RadioPost Gateway (0.23–0.32 s to reply, missed by 3–85 ms, three sessions of three). Its
proposal was a slave that decides its answer at the last moment *and* a program that replies
within about 0.2 s. Winlink Express took 0.26 s, and nobody can change it. So the second half
would have to be Aether's: the slave would have to ask for a slot that opens later after it has
just handed data to a program. That costs every exchange where the program has nothing to say,
so it is a trade, not a fix.

The portable's own program was quicker in the other direction: it wrote its SID and `FF` 0.04–0.1
s before the portable keyed at 17.23 s, and they went in that transmission. The master decides
when its slot ends, and that left Winlink Express just enough time.

**The DISC waits for an acknowledgement.** Home's program asked to disconnect at 35.88 s, and its
answer at 37.08 s carried the leave flag with `FQ`. The portable could not send its DISC next:
`_can_leave` waits until nothing the slave sent is unacknowledged, and only a header
acknowledges. So it sent a header at 39.22 s, home answered at 43.25 s, and the DISC went at
44.34 s: 5.1 s for the acknowledgement. A DISC has the header's acknowledgement fields (base and
bitmap), sent as zero and not read. If the master's DISC acknowledged what the slave sent in the
slot that asked to leave, and the slave took that acknowledgement before it answers, the
exchange would go. Winlink Express sends `FQ` and then `DISCONNECT` at the end of every B2F
session it answers, so it would save one exchange in each. Whether an older station would read
such a DISC correctly (it ignores the fields today) is the protocol question.

Without the two waits the session would have run about 28 s, not 38.3 s. At 500 Hz with headers
on the floor an exchange costs about 5 s; at 2300 Hz with ordinary headers, about 2 s.

## 5. What each direction carried (for the rate controller)

* **Home to the portable** (read at +7 to +9 dB): the 68-byte greeting in one rung-11 frame
  (16-QAM ¾, 81 bytes, threshold +3.5 dB), decoded at +8.8 dB. `FQ` at rung 5, eased from the
  choice by ADR-0130 (one frame either way).
* **The portable to home** (read at −1.9 and −2.4 dB on its data): C1 chose rung 4
  (tone4x100-93, threshold −11.1 dB), and ADR-0130 eased the two frames to rung 2
  (tone4x100-51): 10.7 s of data. Rung 5 (QPSK ½, 25 bytes, −5.2 dB) would have taken four
  frames, and rung 6 (QPSK ⅔, 34 bytes, −3.6 dB) three: 4.2 or 3.2 s, at 3.2 and 1.6 dB over
  their thresholds. At 500 Hz C1's guess holds 3 dB back from an untried rung. Under the
  author's rule (*The rate controller waits for the air*), this is evidence for P9-21, not a
  change.

## 6. The TX-500's monitor and the keyed-audio watch

At 18:43:41.8, during the DISC, the portable logged:

```
keyed: the radio's receive audio did not change while it was keyed, 3 transmissions in a row
(-19 dBFS keyed; -29 to -20 dBFS receiving, in 1–2.5 kHz): the radio may not be transmitting.
```

It was transmitting: home decoded every one of those frames. The portable's recording while
keyed shows why the watch could not tell:

| audio Hz | keyed, against receiving |
|---|---|
| 500 | −48 dB |
| 800 | −53 dB |
| 1100 | −28 dB |
| 1350, 1450, 1550, 1650 (its four tones) | +7 to +9 dB |
| 1400, 1500, 1600 (between them) | +3 to +4 dB |
| 2000 | −54 dB |

(Each bin keyed, over the 10 s of four-tone data, against the same bin over 21 s of the
pre-roll.) The TX-500 mutes its receiver, and outside the signal the audio falls 28–54 dB. But it
sends its own transmitted signal back on its audio output, so the band 1–2.5 kHz the watch
measures holds the signal, at −19 dBFS. The watch takes a monitor as one only when it reads 3 dB
over the loudest receive level (`MONITOR_ABOVE_DB`), and on a busy 40 m the portable's receive
audio reached −20 dBFS. A monitor at the band's own level looks like a radio that never keyed.

Two ways to tell, for the author:

* **Measure the fall outside the station's own signal** in that band. At 500 Hz the channel is
  ±280 Hz around 1500 Hz and leaves 1–1.22 and 1.78–2.5 kHz, where the TX-500 fell 28–54 dB. At
  2300 Hz the signal fills the band, and this does not work.
* **Recognise the station's own signal** in what the radio sends back while keyed: the audio
  correlated with what was played, at the loop's delay. A radio that returns its own signal is
  transmitting, at either bandwidth, whatever its level. A USB interface that leaks its output
  into its input would pass this test too, so it would need a level floor: here the returned
  signal was within about 10 dB of what was played, which a leak inside an interface does not
  come near.

## 7. The stations

* **Home's capture** skipped 8 times before the session (18–48 ms, 244 ms in all, every skip
  padded, ADR-0089), none during it. Home's audio, and the busy detector's line, show a busy 40 m:
  the busy lamp turned on and off every few seconds while idle, at both ends. The busy detector
  waits for its review; a call from a host program is the operator's, and none was held.
* **The portable's PC** lost nothing (`capture_skips` 0, the slowest pass 30 ms).
* **Profiles** were made by the program finder at both ends (the names above), and the home
  profile was renamed on the panel before the session.

## 8. Changes proposed

| # | change | where | whose call |
|---|---|---|---|
| 1 | The master's DISC acknowledges what the slave sent in the slot that asked to leave, and the slave takes it before its DISC_ACK; one exchange less whenever a program asks to leave with its last bytes (every B2F session Winlink Express answers) | Aether, the engine (model first, ADR-0069 / ADR-0072); a protocol question for older stations | the author's |
| 2 | After a slave hands data to a program, its slot opens late enough for a reply ~0.3 s later. Report 20's change 4, with the evidence that Winlink Express takes 0.26 s; costs every exchange where the program has nothing to send | Aether, the engine | the author's |
| 3 | The keyed-audio watch recognises a radio that returns its own signal (the fall outside the signal, or the signal itself in the keyed audio) | Aether, `station/keyed_audio.rs` | the author's |
| 4 | The portable's data on the floor at −2 dB where an OFDM rung had 1.6–3.2 dB in hand | the rate controller's review (P9-21) | waits for the air, by rule |
