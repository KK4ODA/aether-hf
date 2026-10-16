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

*(the harness: filled in below once the runs are in)*

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

`tools/mixed_pair.py` against v0.2.0-beta.108's daemon, both ways round (§4).

## 6. Configuration

Schema 13 (`identifier_at_the_end`) removes `radio.cw_id_interval_s`, and the settings registry,
the panel, the manual and the profiles lose it. A profile carrying it goes through the migration
chain. Fixture `0.2.0-beta.110-identifier.toml`. Forcing needs no key.
