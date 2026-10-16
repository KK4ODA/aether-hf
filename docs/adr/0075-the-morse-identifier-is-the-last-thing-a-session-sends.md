# ADR-0075: The Morse identifier is the last thing a session sends

**Status:** accepted, 2026-10-15, the author's decision as licensee and author, after
v0.2.0-beta.108. Model first (`aether_model/link/granted.py`: the closing identifier's placement),
then the port (`aether-link/src/granted.rs`) and the replay vectors (`granted_vectors.json`: the
`identifiers`, `idle-identifier`, `dead-path`, `aborts`, `leaving`, `called-again` and
`late-station` sessions), then the daemon (`aetherd/src/station.rs`, `cwid.rs`, `config.rs`,
`host/server.rs`, `station/reserve.rs`), the panel and the manual. No wire change: a station on this
build and one on v0.2.0-beta.106 to beta.110 close a session together safely (§5).

Supersedes, for sessions: ADR-0055 §2.2, §2.3 and §2.5 (the ten-minute identifier, the
identifier carried by the DISC or DISC_ACK, the beacons' and datagrams' identifier); ADR-0022's
waits for the other station's identifier; `07-protocol-8.md` §5.4's first text (the identifier
ahead of the DISC_ACK and of the master's first DISC, and the mid-session identifier in a slot
granted with `id`). ADR-0055 §2.1 and §2.4 and its §4 amendment stand.

## 1. Context

The author wants the station to identify the way VARA HF does:

* VARA HF (*VARA HF 4.7 Quick Guide*): "CW ID — For USA calls, the CW ID is forced." On the air,
  in the author's experience, it goes at the end of a session, the last thing each station sends.
  When it goes is not in VARA's public documents (report 12 §20.3).
* VarAC leaves the Morse identifier to the modem and sends neither `CWID ON` nor `CWID OFF`.
  Winlink Express and RMS Trimode say `CWID ON` at every start. VARA's published command list has
  no `CWID` command; the adapter answers it `OK`.

Beta.108 did otherwise. It identified at a session's end ahead of the DISC or DISC_ACK, every ten
minutes during a session (`cw_id_interval_s`), and on beacons and datagrams when the interval
said so; and the identifier was off unless the operator ticked *Morse ID*, US callsign or not. A
host program waiting for `DISCONNECTED` could change channel or call again while the other
station's identifier was still to come.

## 2. Decision

1. **Once a session, at its end, last.** Each station's Morse identifier is the last thing it
   sends for a session, however the session ends. Nothing inside a session carries one: the
   ten-minute identifier is gone, and so is `cw_id_interval_s` (configuration schema 13 removes it;
   §6). Beacons and KISS datagrams carry none.
2. **Where it goes** (`07-protocol-8.md` §5.4, `04-formal-spec.md` R13):
   * **A closing exchange.** The DISC grants the DISC_ACK's slot with the slave's declared
     identifier time added, as protocol 8 always has. The slave sends its DISC_ACK and its
     identifier right behind it, in that slot. The master keys its own alone once that slot is
     over: at the DISC_ACK's end plus the slave's declared identifier and R4's wait, or at
     `end_max` when it read no DISC_ACK.
   * **A DISC heard again** is answered again, identifier and all, so the identifier stays last.
   * **After its DISC_ACK the slave starts nothing of its own** until the master's identifier can
     be over: the slot's latest end, the master's wait at the top level with a slow key, and the
     master's declared identifier (`_closing_hold`).
   * **A slave's disconnect** is the master's DISC in reply to its `flag`; an aborting slave's
     DISC is answered by the master's own closing DISC (`_peer_left`), so both close the same way.
   * **An aborting master** identifies after its leave DISC's slot: after the DISC_ACK and the
     identifier behind it, or after the last repeat's slot.
   * **No closing exchange** (a link timeout, "no response", every DISC unanswered, a DISC_ACK
     that did not go out): each station sends its identifier on its own, listening first
     (`Transmit.listen_first`; the daemon waits for a clear channel up to 15 s). A master that read
     nothing of the slave at the end listens too: the slave may have given up first and be
     sending its own. A slave that left a session still heard waits one exchange, since the
     master's next words may close it properly. A slave that timed out waits an exchange and the
     master's declared identifier: it cannot hear the master, so listening cannot part their
     identifiers, and the master gives up no later than its own link timeout after the last
     answer it can have read and identifies then (§4).
   * **Called again** (ADR-0056): no identifier between the sessions. The communication goes on
     in the new one (ADR-0055 §4); a call to another station sends the owed identifier first.
   * **A contact that went no further than its setup** is identified on its own 30 s after
     (ADR-0055 §4). Beta.109 had identified an unanswered call's end at once; it now waits the 30 s
     as ADR-0055 says.
3. **Forced for US callsigns** (`cwid::required_for`, one pure function used by the station, the
   panel through `identifier.check`, and `status.identifier`):
   * a US callsign always identifies, whatever `[radio] cw_id` or a host's `CWID OFF` says. Any
     other callsign identifies only with `cw_id = true`, which stays off by default;
   * decided from the callsign the identifier sends, the session's own (`callsigns.set`, a host's
     `MYCALL`: a call made as another of the station's callsigns declares that one's length);
   * US means the prefixes AA–AL, K, N and W. An SSID, `-T`/`-R`, and `/P`, `/M`, `/MM`, `/QRP` or
     a bare digit change nothing. **A portable form goes by where the station operates**: a prefix
     in front decides (`W4/G4ABC` is US, `VE3/KK4ODA` is not), and so does a part after the
     callsign that names a place, a letter and a digit (`G4ABC/W4` is US, `KK4ODA/VE3` is not).
     §97.119(g) puts a reciprocal operator's indicator before the callsign, and after it for a
     Canadian licence; the rule reads both. A station abroad identifies under that country's
     rules, which this function does not know;
   * `cw_id` is never rewritten, in `station.toml` or in a profile: the forcing is computed.
     `status.identifier` says `enabled`, the `setting`, whether it is `forced` and the `reason`
     ("Required for US callsigns (§97.119)"), the callsign, the speeds and whether one is
     `pending`. The log says once at start, and once on a callsign change, when a US callsign is
     forced over a setting of off;
   * a host's `CWID OFF` on a US station is answered `OK`, recorded, and changes nothing;
     `CWID ON` is recorded and acts on nothing;
   * the panel's *Morse ID* box is ticked and greyed out for a US callsign, with the note
     *Required for US callsigns (§97.119)* and a tooltip; what the operator had set is kept for
     any other callsign;
   * the 20 wpm cap on an automatic identifier (§97.119(b)(1)) is unchanged.
4. **The host program hears `DISCONNECTED` after its station's identifier** (`identified` state
   event, 150 s at most): VarAC going back to its calling frequency, a scanning Trimode changing
   channel or Winlink Express placing its next call do not go over it. A host-keyed station
   (`[ptt] kind = "host"`) is still attached then and keys for it. A host-keyed station with no
   host attached logs that the closing identifier did not go out.
5. **A third station waits out the closing.** A station that heard another session's DISC_ACK
   holds its own calls until the channel has been quiet `RESERVE_GAP_S` (4 s), at most 40 s
   (`station/reserve.rs`): the identifier behind the DISC_ACK, the quiet of the slot's end and the
   other station's identifier after it. The harness's `40m-gateway-busy-when-called-2300` found
   it: a waiting client called in the 0.7 s between the gateway's identifier and its client's.
6. **A stop waits for the end.** The daemon's wind-down is 22 s (was 12): its DISC, the other
   station's answer and both identifiers. The shell waits 30 s before it kills the daemon (was 20)
   and the systemd unit 30 s (was 15). The log says when a stop came first.

**The invariant each rule keeps** is I1, no radiated overlap: every identifier keys in a slot the
other station granted and keeps, after a slot that can no longer be in use, or on a channel heard
clear (S8, a new assumption: carrier sense, which protects only where the two stations hear each
other). **No deadline moves on undecoded evidence**: the master's is a decoded DISC_ACK's stated
end, the slave's hold counts from its own slot, and listening first delays only a transmission
outside any slot, as a call already waits.

## 3. Regulatory: what identifies a session (reported, nothing changed)

From the e-CFR (Title 47 current to 2026-10-13):

* **§97.119(a)**: the call sign "at the end of each communication, and at least every 10 minutes
  during a communication".
* **§97.119(b)**: (1) a CW emission, an automatic device at 20 wpm at most; (3) "a RTTY emission
  using a specified digital code when all or part of the communications are transmitted by a RTTY
  or data emission".
* **§97.309(a)**: the specified digital codes are Baudot, AMTOR and ASCII; (a)(4) allows any
  publicly documented technique carrying one.

What identifies a session now:

* **Aether's frames.** Calls, acceptances, probes, probe answers, beacons and datagrams carry the
  callsigns, packed in Aether's own 6-bit code (`pack_callsign`) — not a specified code. Session
  frames carry none.
* **Aether's Morse identifier**: at the end only.
* **VarAC**: its *Call ID interval* (a third-party manual; VarAC's quick start says it
  "periodically sends your callsign on air") sends `DE <callsign>` as chat text, which Aether
  carries as data inside its own frames — compressed when `[radio] compress` is on, off by default.
* **Winlink B2F** (Winlink Express, Pat, RMS Trimode): the callsigns at the start only.
* **The panel's sessions and Tests**: nothing after the call.

So **a session longer than ten minutes is identified between its call and its end by nothing in
Aether**, and in the host programs only by VarAC's *Call ID interval* text when set. Whether that
text, or the callsigns in Aether's call frames, meet (b)(3) is the open question every data mode
lives with (report 12 §20). The earlier claim, "off by default because Aether's frames carry both
callsigns (§97.119(b)(3))", was wrong on two counts: session frames carry no callsign, and (b)(3)
names a specified code, which the packing is not. `docs/user/fcc-regulatory-controls.md` now says
this. Filling the gap is the author's decision; nothing was added for it.

## 4. Measured

**The model** (`model/tests/test_granted.py`): both identifiers last for every closing (master,
slave, both, both airs), the slave holding its own initiatives until the master's identifier can
be over, a lost DISC_ACK answered again with its identifier, the master giving up on the DISC_ACK,
each abort, a dead path, a band that closes, calling again, a station that sends none, and the
callsign a session runs under. Sent at once after a link timeout, a slave's identifier went over
the master's last tries in 24 of 96 band-closing sessions (35 dB in 20 s, both airs, two sizes,
three closing times, two fall rates, four seeds); with the wait of §2.2, in none.

**The port** replays the model input by input (`granted_vectors.json`); the identifier sessions
were extended until every rule here, removed from the port, failed the replay.

**The harness**, three draws of all 50 scenarios on this build (`session_matrix.py --seeds 3`):
150 of 150 pass, no radiated overlap (`tools/air_account.py`), and every run judged on the
identifier invariants — an identifier only alone or behind a DISC_ACK, nothing of a session
after the identifier that closed it, and with `identifiers_last` every session of an identifying
station ending with its identifier. The host scenarios pass with each program told
`DISCONNECTED` only after its station's identifier (the emulator checks `status.identifier`
when it hears it): the Winlink client (`*-winlink-exchange-*`), the Trimode gateway and its
clients (`40m-trimode-gateway-2300`, `80m-trimode-client-vanishes-500`, the three `40m-gateway-*`),
and VarAC's chat and ping (`80m-varac-chat-and-ping-500`, new). The five endings of
`40m-closing-identifiers-2300`, as the account of the air has them (W4ODA calls KK4ODA):

| ending | the closing on the air (s) |
|---|---|
| the caller closes | W4ODA DISC 34.2–35.1 · KK4ODA DISC_ACK + Morse 35.4–40.4 · W4ODA Morse 41.1–44.8 |
| the called station asks to leave | W4ODA DISC 69.4–70.3 · KK4ODA DISC_ACK + Morse 70.6–75.6 · W4ODA Morse 76.3–80.0 |
| the caller aborts | W4ODA leave DISC (floor) 109.9–113.5 · KK4ODA DISC_ACK + Morse 113.8–121.6 · W4ODA Morse 122.3–126.0 |
| the called station aborts | KK4ODA DISC 163.5–167.1 · W4ODA DISC 167.4–168.3 · KK4ODA DISC_ACK + Morse 168.6–173.5 · W4ODA Morse 174.3–178.0 |
| the path dies (outage 196–436) | KK4ODA times out, Morse 383.3–387.7 · W4ODA "no response", Morse 452.6–456.3 |

**What it costs.** From the end of the last DISC_ACK's frame, where a session ended before, to
the end of the last identifier: a median 8.4 s a closing exchange (4.0–15.1 s over 213 closings),
3.6 % of the scenarios' air time — by class, VOX 12.4 % (many short sessions), identifier 5.4 %,
host programs 4.9 %, band 3.5 %, other 3.2 %, robustness 2.6 %, stress 2.4 %. Against master's
daemon (beta.110) on the same scenarios the pooled air time is ×1.042; the per-class ratios
(host programs ×1.07, stress ×1.06, VOX ×1.23, band ×0.92) carry the harness's spread between
draws. The nightly's baseline was taken from these runs (`bench/baselines/scenarios.csv`).

**Found on the way, and fixed:**

* A third station waiting to call a gateway called in the 0.7 s between the gateway's
  identifier and its client's (`40m-gateway-busy-when-called-2300`, 18 radiated overlaps): its
  reserve ended at the DISC_ACK (§2.5).
* Both stations of `80m-band-closes-2300` timed out on a path 25 dB under the noise and sent
  their identifiers 0.6 s apart (§2.2, the timed-out slave's wait).
* A stop's 12 s wind-down could not hold the end of a session (§2.6).
* Two harness faults, neither the daemon's: a host emulator that connected before a loaded
  daemon had opened its host port ended the whole run, and a torn read of the channel server's
  air clock (0.0 s) timed a step out at once — it cost `80m-vox-caller-declared-500`'s third draw
  a session.

**Not removable: two stations that cannot hear each other.** A slave that timed out cannot hear
the master, so nothing tells it the master has stopped. When the master retries long
transmissions well past the slave's link timeout, the slave's identifier can fall on the
master's last tries: the outage session above, 383.3–387.7 s against the master's 367.9–393.0 s.
Neither station hears the other, so the session loses nothing, but the two radiate together and
a third station hears both. The harness counts no overlap inside its scripted outages, and found
none on any path the stations could hear. The wait of §2.2 rests on both stations working out the
link timeout alike, which holds when their configurations match.

## 5. Wire compatibility, without a protocol bump

No frame changes, so link protocol 8 stays. A station of beta.106 to beta.110 identifies ahead of
its DISC_ACK and ahead of its first DISC.

* **An older master, a new slave.** The slave's identifier behind its DISC_ACK lies inside the
  older master's grant, which is the same slot. The older master keys next no sooner than its
  `end_max`, after it.
* **A new master, an older slave.** The older slave's identifier ahead of its DISC_ACK is inside
  the same granted time. The new master waits for the DISC_ACK's end plus the declared identifier,
  later than the real end.
* **Mid-session**: an older slave's request for an identifier slot (`id` in its ANS) is still
  granted.

**`tools/mixed_pair.py` against v0.2.0-beta.108's daemon**, the seven scenarios tagged `mixed`
(`40m-closing-identifiers-2300` among them), both ways round: 14 of 14 pass, no radiated overlap.
beta.108 does not force the identifier, so its stations there sent none. With `cw_id = true` at
both stations of the closing scenario:

* **beta.108 calls this build:** passes, no radiated overlap.
* **This build calls beta.108:** one collision, 16 radiated overlaps, in the session the called
  station aborts. The beta.108 slave sends its closing identifier alone after its own abort,
  while the master is still in the session and keying its next transmission.
* **Two beta.108 stations do the same:** 38 radiated overlaps, and a later session failed to
  connect.

So the overlap is beta.108's own, and it ends when that station updates.

## 6. Configuration

Schema 13 (`identifier_at_the_end`) removes `radio.cw_id_interval_s`, and the settings registry,
the panel, the manual and the profiles lose it. A profile carrying it goes through the migration
chain. Fixture `0.2.0-beta.110-identifier.toml`. Forcing needs no key.
