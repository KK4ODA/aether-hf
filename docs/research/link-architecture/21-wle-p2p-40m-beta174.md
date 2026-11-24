# Winlink Express P2P on 40 m at 500 Hz, portable to home (2026-11-24)

Three sessions on 40 m at 500 Hz, 18:42–19:01Z (local 13:42–14:01): Winlink Express at the
portable calls Winlink Express at home in a *Vara HF P2P* session. §0–§7 read the first; §8 the
second (a 2.2 kB message, fine) and the third (a 50 kB file that Winlink Express gave up on after
5 minutes); §9 is the list of fixes the author decided. The first session, at 18:42:54Z, is the
first P2P session on the air run by a Winlink program through Aether since the *external
programs* build (beta.173, beta.174). Both stations uploaded every session by debug mode, with
audio. The author's note: the external
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

## 8. Two more sessions the same afternoon

The same stations and settings, beta.174 at both ends, both sides uploaded with audio (the zips
`20261124-185111Z KK4ODA-1 …`, `20261124-185118Z KK4ODA-2 …`, `20261124-190242Z KK4ODA-1 …` and
`20261124-190243Z KK4ODA-2 …`, fetched through temporary copies). Both read with `two_sides.py
--from-loops`; session 3 also with `rung_reading.py`. Neither had a capture skip inside it, and
neither overlapped (least clear air 0.639 and 0.646 s; home's answers 112–124 ms into their slots,
at least 226 ms inside their limit).

### 8.1 Session 2 (18:48Z): home sends a message

Recordings `20261124-184807_KK4ODA_WD5EMA` and `20261124-184816_WD5EMA_KK4ODA`. The portable
called (first try, 10.8 s; home read the call at +1.1 and +3.4 dB) and had nothing to send. Home's
Winlink Express proposed one message and sent it, 2 231 bytes, at rungs 11 and 12 in about 47 s;
the portable read home at +5 to +13 dB. Then `FF`, `FQ` and the disconnect: 94 s connected.

The end lost three exchanges, about 15 s, on the portable's clock:

| at | who | what |
|---|---|---|
| 94.73 | home | the message's last four frames |
| 100.03 | portable | header acknowledging them; its program wrote `FF` at this moment, 0.22 s after the frames were delivered: too late for this transmission |
| 104.05 | home | answer, nothing else |
| 105.13 | portable | `FF`, one frame at rung 5 |
| 107.00 | home | answer, nothing else; its program wrote `FQ` at 106.77, 0.17 s too late |
| 108.08 | portable | header with nothing: an open window |
| 112.10 | home | answer with the leave flag (its program asked to disconnect at 110.81) and `FQ` |
| 114.23 | portable | header acknowledging `FQ` |
| 118.25 | home | answer, nothing else |
| 119.33 | portable | DISC |

So §4's two waits happen at the calling station too: a master decides its next transmission as soon
as the other station's slot ends, and Winlink Express at the portable replied 0.22 s after its data
arrived.

### 8.2 Session 3 (18:55Z): a 50 kB file that Winlink Express gave up on

Recordings `20261124-185507_KK4ODA_WD5EMA` and `20261124-185513_WD5EMA_KK4ODA`. The portable called
(first try, 8.1 s; home read the call at +7.5 dB) and sent a message with a 50 kB incompressible
file. Its Winlink Express wrote the whole of it, 52 148 bytes, at 18:55:29.602. Aether sent it at
rung 9 (16-QAM ½, 53 bytes a frame), six frames a transmission, an exchange every 8.7 s.

| rung | sends | failed | decoded alone | delivered | over the exchanges | SNR read at home |
|---|---|---|---|---|---|---|
| 6 (QPSK ⅔) | 12 | 0 | 12 | 372 B | 130 bit/s | +1.8 dB |
| 9 (16-QAM ½) | 230 | 39 (and 3 never detected) | 188 | 9 700 B | 224 bit/s | +5.7 dB |
| 10 (16-QAM ⅔) | 8 | 5 | 3 | 272 B | 245 bit/s | +6.2 dB |

Home read the portable at −2 to +9 dB, fading through the session. Rung 10 measured worse than
rung 9 (25 % of 4 first copies against 82 %), so the rate fits the path; at 224 bit/s the file
needed about 31 minutes.

At 19:00:29.632, **exactly 300 s after it wrote the file**, Winlink Express sent `DISCONNECT`
("inactivity", the operator's report). `BUFFER` had fallen every 9 s throughout (52 148 → 43 665),
and `SN`, `BITRATE`, `PTT` and `IAMALIVE` lines kept coming, so none of them counts as activity to
it. The last data it received came about 0.2 s before it wrote the file, so a timer from either
moment fits. A web search found nothing on Winlink Express's timer.

Aether then kept the session up for **70 s**, sending what was queued as ADR-0072's drain does, and
closed with *41955 bytes not delivered*: it delivered 1.7 kB of a message that could never be
complete. The drain took 70 s rather than 60 because a silent slot (at 340 s on the portable's
recording clock) put home's answers on the floor for four slots (ADR-0069 Step 3's rule), which stretched the exchanges to 11.4 s.

Both identifiers went as before. The TX-500 warning of §6 came again at 18:55:36 (*−23 dBFS
keyed; −29 to −24 dBFS receiving*): it is said once every ten minutes of use.

## 9. The fixes, decided

From the three sessions; asked one at a time and answered by the author on 2026-11-24.

| # | finding | decided | where |
|---|---|---|---|
| 1 | Winlink Express gives up on a long one-way transfer after 5 minutes (§8.2) | **The author checks Winlink Express first**: its session log's text around 19:00:29Z and whether its settings have a timeout. If it is a setting in its ini file, the program finder can raise it when it sets Winlink Express up | the author, then `hostapps` |
| 2 | After a program's `DISCONNECT`, the drain sends what can never finish (§8.2) | **Close if it can't finish**: when what is queued could not go within the drain at the rate this session has delivered, it is dropped and the session closes at once, the log saying how much. A queue that fits (`FQ`) still goes first | the engine (model first, ADR-0072) |
| 3 | A program's reply misses the next transmission (§4, §8.1) | **Both stations wait at hand-overs**: when one side has just sent the last of what it had, the station that received it holds its next transmission up to about 0.35 s for its program's reply. The calling station waits; for the called station the caller opens the reply slot later, which its header can already say. Expected to need no protocol change. Settles report 20's change 4 | the engine (model first), judged on the harness's Winlink-shaped scenarios |
| 4 | The DISC waits for an exchange that acknowledges the slave's last bytes (§4, §8.1) | **The DISC carries the acknowledgement, no protocol bump**: the master's DISC acknowledges what the slave sent in the slot where it asked to leave, and the slave takes it before its DISC_ACK. beta.174 sends the fields as zero and ignores them, so mixed versions end as today; to be confirmed in the model and the vectors | the engine (model first) |
| 5 | The TX-500's own signal makes the keyed-audio watch warn (§6) | **Recognise its own signal**: a transmission counts as on the air when the audio heard while keyed is the station's own signal coming back at the radio's delay; a key that does nothing still warns | `station/keyed_audio.rs` |
| — | The portable's data on the floor at −2 dB in session 1 (§5) | No change: evidence for the rate controller's review (P9-21), by rule | — |

**Built the same evening** (the author's "go ahead with all fixes", with Winlink Express's own log
of the day). Its log says only *P2P session inactivity timeout. Disconnecting.* at 19:00:29, 300 s
after *Sending 07MVP34173Z0*, and then waited 78 s for the modem; its settings have no such
timeout, so fix 1 went into the host adapter instead of the program finder:

* fix 1, **ADR-0134**: a `DISCONNECT` from a program quiet for 240 s, with its own data still
  going, is answered and not acted on; a second within a minute, or `ABORT`, ends the session;
* fixes 2, 3 and 4, **ADR-0135**: a drain that could not finish at the session's rate closes at
  once; the master waits up to 0.35 s for its program's reply after the slave's last data, and
  after its own last data grants the slave's answer a frame of room at a later level, which the
  slave decides at the last moment; the slave's count of its own frames is no longer granted
  again; the DISC acknowledges the slave's data, said by its `flag` bit (no protocol bump);
* fix 5, **ADR-0136**: the keyed-audio watch takes a transmission whose audio is the station's
  own signal coming back as on the air.
