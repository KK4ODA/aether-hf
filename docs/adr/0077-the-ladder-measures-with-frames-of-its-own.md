# ADR-0077: The ladder measures with frames of its own

**Status:** accepted, 2026-10-17, the author's decision on report 13 §14 change 10
(`docs/research/link-architecture/13-wc4y-beta111.md`): "at this stage, it does not matter if
changes modify what goes in the air, as we are trying to build the best modem possible and the
userbase is very small". Model first (`aether_model/link/frames.py`, `granted.py`), then the port
(`aether-link/src/frames.rs`, `granted.rs`), the replay vectors and the daemon's Test
(`aetherd/src/station/fieldtest.rs`). **Link protocol 9**: a station on this build does not
connect to one on v0.2.0-beta.106 to beta.112 (protocol 8), and says so; the release notes open
with *Update required* and the shell titles the offer the same way.

## 1. Context

The Test's ladder measured a rung by pinning the session's own data there: a few test bytes, in
frames padded to the rung's size, sent at the rung and acknowledged like any data (P6-7).
ADR-0074 re-encoded such a frame at the recommended rung after its first copy, since that copy was
the measurement; ADR-0076 stopped the disconnect from waiting for what was left. Neither removed
the cost, because a frame that failed at its rung was still data, and data has to arrive:

* **On WC4Y's Test of 2026-10-17** (report 13 §10) the ladder's failed frames, re-encoded at
  rung 3, were 13 tone frames of ten-odd bytes each, 5.36 s apiece, every batch alone (one family
  a transmission) with its own answer. Delivering them took about 93 s of the Test's 435 (21 %),
  more than the ladder's measurements themselves; 26 s of it was the disconnect's drain, which
  ADR-0076 removed, and the rest stayed.
* **On ND1J's Tests of 2026-10-15** (report 12 §21) the same leftovers ran to about 130 s of each
  weak Test, before ADR-0074 moved the ladder after the file.
* **The measured frames were not what data sends.** Ten bytes padded with zeros to the rung's
  capacity is a codeword unlike any data frame's, and on the four-tone kinds a lopsided spectrum
  (report 12 §9).

ADR-0076 §5 named the answer — a frame kind the other station counts and never delivers — and
left it for a protocol change.

## 2. Decision

1. **A measurement frame.** DATA kind `MEASURE` (7, the kind's last free value): filler the
   rung's full size (`measure_body`: SplitMix64 from the session and the frame's index, the
   engines' `Draws`, so both engines send the same bytes and the frame looks like data), `seq`
   the frame's index among its transmission's measurement frames. It takes no sequence number,
   is never delivered and is never sent again.
2. **`measure(mode, frames)`** (both engines; `measuring`; `take_ladder` returns the reports as
   `LadderRung`s). The frames go in a transmission of their own once none of the station's data
   is unacknowledged, and no new data goes while one is asked for, so it waits only for what is in
   flight. As many go as the transmission holds — `MAX_DATA` (6), the key limit, a slave's grant —
   at the station's ceiling at most.
3. **The `measure` bit.** In a header and an answer, bit 6 of the control frame's byte 2 says the
   data frames after it are measurement frames, and with none after it that the station's last
   were and it has not read their report. Every control frame of the station's carries it until
   the report, and none of them carries data. A measurement's header is never left out (R7). In
   protocol 8 the bit asked for, or granted, the slave's identifier time inside a session, which
   ADR-0075 had already retired: a slave of beta.110 on never set it, and the master's handling
   of an older slave's request (`_slave_id_due`, `Slot.ident_s`) goes with it. A DISC's and a
   DISC_ACK's bit is the closing identifier's, as before.
4. **The report.** An acknowledgement whose bitmap has bit 0 set (`REPORT`) reports the other
   station's last measurement — bit `i + 1` set for frame `i` arrived — and acknowledges nothing
   at or above `base` (`GrantFrame.received`, `reports`, `measured`). Bit 0 of a plain
   acknowledgement is always clear, since `base` is the first sequence number not received, so no
   report is ever read as an acknowledgement of data. A station reports in every acknowledgement
   from the measurement on, until the measuring station's data comes again: an opening control
   frame that counts data without `measure`, or a data frame decoded. The measuring station takes
   the first report it reads; one decoded late is not taken, as the next says it again.
5. **Nothing to combine.** A station that learns of a measurement — the bit, or a measurement
   frame it decodes — drops every buffer it holds to combine, and keeps none for a measurement
   frame that fails. None can be of data, since none is in flight; and when the measurement's
   header is lost and none of its frames decodes, the receiver cannot tell them from data and
   keeps them under the sequence numbers the next data takes, where a buffer of another codeword
   would be combined with it. The measuring station's next control frame carries the bit, so the
   receiver drops them before any data comes.
6. **Rate.** A measurement's frames feed the receiver's rate controller as the pinned frames did
   (ADR-0020's rule for a burst above every rung asked for). A slot heard silent after a
   measurement steps no rung down (R9): its header always goes, so the silence says the header or
   the answer was lost, not that the rung measured was too fast. The report then follows a poll's
   answer and its SNR is not the rung's (ADR-0076): the rung is recorded with none.
7. **A measurement that never reached the air is asked again**: dropped by the keying gate
   (`on_tx_dropped`) or cut before any of it could be read (`on_tx_cut_unread`).
8. **ADR-0074's pinned re-encode goes** (`_TxRecord.pinned`, `TxRecord::pinned`). `pin_mode`
   stays, for the benches and the tests, pinning new data as protocol 7's does.
9. **The Test** calls `measure(rung, rung_frames)` at each rung and starts the next rung at the
   report: there is nothing to deliver. `body_bytes` and `ladder_over` are gone. The disconnect
   still drains nothing (ADR-0076): after the ladder only a file whose time ran out can be left.
10. **Protocol 9.** `PROTOCOL_VERSION` 9 in both engines and the shell's `LINK_PROTOCOL`;
    `GRANTED_VERSION` (8) is where the connect body's turnaround and identifier bytes begin.
    `04-formal-spec.md` has the rule as R14; `07-protocol-8.md` §4.1, §4.2 and §7 the frames and
    the interface. The daemon reports a decoded measurement frame as kind `measure`
    (`control-api.md`), names a measurement transmission in the rules check's wording, and counts
    another session's measurement frames as that session's for its reservation (NAV).

## 3. What it costs, measured

On the model's simulator (`test_granted.py`): every rung reported and nothing delivered, no byte
counted; a measurement sent once, alone, its frames indexed, the data behind it delivered whole;
a slave's measurement; data held until the report; no buffer left to combine; the header kept
whatever R7's rule; a lost answer reported after the poll with no SNR. On the port, the replay
(`granted_vectors.rs`) runs the `ladder` (the master measures five rungs, the last with data
queued behind it, and the slave one), `ladder-unheard` (the answer lost; then the header lost),
`late-station` (a slave measuring while the gate drops its transmissions) and the new
`measure-unread` session (a measurement at the data's own rung, its header lost and its frames
failed while the slave keeps answering, then the data's first copies failed too). Three bugs
planted in the port were caught: a report bit shifted one place, the buffers not dropped (by
`measure-unread` only), a dropped measurement not asked again. The harness A/B is §6.

## 4. Consequences

* **A rung costs its exchange**, and a rung whose answer is lost one poll more. Nothing of the
  ladder is delivered, so nothing is left after it; the Test's air time falls by what report 13
  §10 counted for the leftovers.
* **A ladder row means what it says**: the frames are the rung's full size and go once. Rows from
  earlier builds counted ten-byte bodies, and re-encoded leftovers no longer follow them.
* **Data waits for a measurement's report.** A station measuring in a session that also carries
  its data sends nothing of it from the measurement to the report — one exchange, two when the
  answer is lost. The Test measures after its transfers, so it does not notice.
* **A slave's measurement goes with as many frames as its grant holds**, which its answer's
  queue count asks for; a master's always has room for `rung_frames` up to six.
* **Every station updates**: protocol 8 and 9 do not connect. The userbase is three stations.

## 5. Not done

* **Telling a measurement frame from data without its header**, before the poll: it would need
  the PHY (a measurement frame's own chips or sync pattern). The poll's bit covers what it costs —
  only a buffer to combine — and nothing else depends on it.
* **A measurement split over transmissions**: a measurement is one transmission's worth.

## 6. The harness

The 14 Test scenarios against beta.112's daemon: running; the numbers follow.
