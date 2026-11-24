# ADR-0135: Hand-overs and endings wait for the program, and end in one exchange

**Status:** accepted, 2026-11-24 (the author's answers to report 21 §9's fixes 2, 3 and 4, and
"go ahead with all fixes"). Model first, then the port and the granted vectors. **No protocol
bump:** link protocol 13 as before, and a station of 0.2.0-beta.174 connects and ends sessions
with this one as it did (§2.2). No configuration key, no schema bump. Fix 3 judged on the
harness (§5).

## 1. Context

Three Winlink Express P2P sessions on 40 m at 500 Hz on 2026-11-24
(`docs/research/link-architecture/21-wle-p2p-40m-beta174.md`) carried every frame at its first
copy and still spent a third of their time at the hand-overs and the end:

1. **A program's reply missed the next transmission.** Winlink Express wrote its reply 0.17–0.26 s
   after the data it answers was handed to it (RadioPost Gateway 0.23–0.32 s on 2026-11-21, report
   20 §4). By then the station had decided its next transmission: the master as soon as the
   slave's slot ended, the slave at the moment of delivery (render-ahead, ADR-0069 Step 2). Each
   miss cost an exchange, about 5 s at 500 Hz. Session 1 lost 10 of its 38 s this way and at the
   end; session 2, three exchanges (about 15 s, report 21 §8.1).
2. **The slave's last data was granted again.** The slave's answer says in `n` how many frames it
   still has, counting the frames it sends in that very transmission. The master granted that
   many again after they had all arrived, and the slave answered the slot with nothing: an
   exchange at the end of every run of the slave's data.
3. **The DISC waited for a header.** A slave that asks to leave sends its last bytes (Winlink
   Express's `FQ`) with the request. The master's DISC carried no acknowledgement, so it first sent
   a header to acknowledge them, and the slave answered it with nothing: another exchange.
4. **A drain sent what could never finish.** Winlink Express gave up on a 52 kB file after five
   minutes (ADR-0134), and the drain (ADR-0072) then kept the session up 70 s, delivering 1.7 kB of
   a message nobody would ever have whole.

## 2. Decision

### 2.1 A hand-over waits for the program's reply (fix 3)

`GrantConfig.reply_wait_s`, 0.35 s by default (0 turns both halves off):

* **The master** holds its next transmission for its program's reply when the slave's last data
  has just been delivered to it — the slave wants nothing more, the master has nothing to send,
  nobody is leaving (`_wait_for_reply`). It keys as soon as the reply comes (`send`), or after
  0.35 s without one. The wait only ever delays the master's key (rule 4's deadline is not moved
  earlier), so the slave's slot timing is untouched.
* **The slave** cannot key outside its slot, so the master gives the slot room: the transmission
  that carries the last of the master's data grants at least one data frame, at no lower a level
  than the one that covers 0.35 s (`_level_for`), and keeps its header for it. The slave decides
  that answer at the last moment its slot allows, not at the moment of delivery, so a reply its
  program writes in time goes in it. An answer states its own length, so room it leaves unused
  costs nothing once heard.

Expected without a protocol change (report 21 §9), and so it is: the master's header already
says the level and the room, and the slave's decision time is its own. This also settles report
20's change 4.

### 2.2 The DISC acknowledges, and the slave's wants are exact (fix 4)

* **The DISC carries the acknowledgement** of the slave's data (base and bitmap, as a header's),
  and says so with its `flag` bit (byte 2 bit 7, unused in a DISC before). A slave that reads a
  DISC with the flag takes the acknowledgement before its `DISC_ACK`, so its last bytes are
  acknowledged and the session ends in one exchange. A DISC without the flag (beta.174's) carries
  zeros, which read as an acknowledgement would acknowledge every sequence number half a window
  behind 0: it is not read. A beta.174 slave ignores the fields, so a mixed pair ends as before:
  the slave's last bytes are delivered and reported unacknowledged.
* **The master grants the slave's count less what of it it decoded** in the slot the count was
  stated in (`_want_decoded`): the frames the slave sent in that transmission and that arrived are
  not granted again.

### 2.3 A drain that cannot finish gives up (fix 2)

A disconnect whose queue could not go within the drain (60 s, ADR-0072) at the rate the session
has delivered leaves the queue at once (`_give_up_if_hopeless`): the bytes acknowledged since the
session came up, over the time since then. It judges only once this station's data has been
acknowledged for `RATE_KNOWN_AFTER_S` (30 s); sooner, the drain runs as before. The queue stays
counted as undelivered, so a program is never told its message went, and the event says so:
*43665 bytes not delivered: at this session's 224 bit/s they would take about 1559 s, more than
the 60 s a disconnect waits for them; closing without them*. A queue that would go in time — the
`FQ` a program writes before it asks to leave — still goes first. The daemon's own *closing* line
is not said when the engine has given up (`GrantedEngine::queue_given_up`).

## 3. What it removes

The exchange of headers that acknowledged the slave's last bytes before a DISC, the slot granted
for frames already received, and the minute of drain after a transfer a program has abandoned.
No code is removed: the rules are added where the old ones decided.

## 4. Checked

* Model (`model/tests/test_handover.py`, the simulator's `send_at` and `on_deliver` for a program
  that replies a set time after it is handed data): the master's program's reply arrives 1.5 s
  after it is written where it took 6.5 s; the slave's 2.3 s where it took 3.7 s (0.4 s replies,
  12 dB at 500 Hz); a session whose programs never reply is no slower by more than 3 s; the
  slave's last frames are not granted again; the DISC acknowledges the `FQ` and a DISC without the
  flag acknowledges nothing; a 20 kB queue at 2 dB is left at the disconnect while a short one
  still drains. Five earlier tests of header elision run with `reply_wait_s = 0`: they watch the
  last of a transfer, which now keeps its header.
* Port: `aether-link`'s granted engine follows the model rule for rule; the granted vectors gain
  a `hand-overs` session (a reply from each side, a DISC with its acknowledgement, a drain given
  up), and the replay compares every action and the state after every input.

## 5. On the harness

Report 21 §9 asked for fix 3 to be judged on the harness's Winlink-shaped scenarios.
`bench/scenarios/40m-wle-p2p-500.toml` is report 21's session 2 (WD5EMA calling KK4ODA at 500 Hz,
−2/+8 dB, the B2F lines and their sizes, `think_s` 0.25: the harness's programs now take that long
to answer each line). The results are in §5.1.

### 5.1 Results

To be filled from the A/B run.
