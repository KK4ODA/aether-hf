# ADR-0047: The turn offered at the end of a burst, taken in the acknowledgement

**Status:** proposed, 2026-10-06 — built in the model and ported, off (`LinkConfig.offer_turn`
false in both); measured on the scenario harness (§5), the author's decision pending. Flags
`OFFER` (TURN) and `TAKEN` (ACK); `_send_burst(prefix=…)` / `send_burst_after`, `_on_ack`,
`_on_control`, `_send_ack`, `_turn_taken_unread` / `turn_taken_unread`; stats `turn_offers`,
`turns_taken`; `bench_chat.py`'s `request+offer` policy.

## 1. Context

ADR-0045 left two transmissions to a change of direction: the receiving station's acknowledgement
asking for the turn (WANT_TX), and the sender's TURN — each keyed, each with its lead, tail and
turnaround. The author's bar is VARA HF, where neither station is seen waiting at a turnaround.

## 2. The proposal

1. A sender whose ordinary burst empties its queue (and has no disconnect asked) ends the burst
   with a TURN carrying OFFER.
2. A receiving station with data to send answers with an acknowledgement carrying TAKEN|WANT_TX
   and its own data burst after it, in the same transmission; it is now the sender.
3. The sender, reading TAKEN, becomes the receiving station (`_take_irs`) and acknowledges the
   burst that followed. Reading an acknowledgement without TAKEN, it keeps the turn as before.
4. A lost acknowledgement falls back to the rule two senders already follow: data heard wins —
   and, while the offer is unanswered, a data frame that does not decode but that the physical
   layer trusts counts too (§5): only data can follow an offer that way.

It is a wire change (the two flags; a version-6 station ignores neither safely), so it would be
protocol 7.

## 3. Measured, and why it is not decided

`bench_chat.py`, 16 trials, both bandwidths, AWGN/Good/Moderate/Poor at 0/6/12 dB, `request`
against `request+offer`, the simulator stepped at 50 ms (`STEP_S`; at 1 s the reply was always
typed after its line's acknowledgement had gone, which hid the case the offer serves):

| host's reply | sum of the reply's median waits | keyed time |
|---|---|---|
| at once (0–50 ms) | 131.7 → 136.6 s; worse at 0 dB (2300 Good 8.2 → 10.6 s, 500 Poor 12.9 → 15.4 s) | +2 % |
| 0.2–1 s later | 147.4 → 134.9 s (−8.5 %) | +2 % |

No line lost, no session dropped; `bench_link.py` transfers are unchanged (a transfer ends with
a disconnect asked, so nothing is offered).

The offer frame costs the air the TURN it replaces did. What it saves is a keying of the
transmitter — the lead, the tail that covers the sound card, the radio's turnaround: 0.5–1 s a
change of direction on a real station — and the link bench charges none of it. Only the scenario
harness (ADR-0042), which runs the daemons through the radios' keying, can say whether that
saving is there; that needs the port.

## 4. The port

The Rust engine has the same rules behind `LinkConfig::offer_turn` (false); a daemon built with
it true is what the harness ran. The offer frame is counted in the burst's countdown, the
sender's wait for the acknowledgement covers two frames more when it offered, and
`the_turn_on_offer_is_taken_in_the_acknowledgement` holds both settings in both suites.

## 5. Measured on the scenario harness

Two daemons through the channel server, the two Winlink-shaped scenarios (eight changes of
direction, each reply sent the moment the last line arrived): 40 m at 2300 Hz on Good +10 dB,
seeds 31–35, and 80 m at 500 Hz on Moderate +6 dB, seeds 31–41. Both builds carry the two fixes
below.

| | offer off | offer on |
|---|---|---|
| 40 m, 5 sessions, air | 437 s | 378 s (−13.5 %) |
| 80 m, 11 sessions, air | 3 571 s | 3 251 s (−9 %) |
| 80 m, the seven short exchanges | 873 s | 624 s (−29 %) |
| 80 m, the two messages (1.5 kB, 2 kB) | 2 434 s | 2 376 s |
| collisions (40 m / 80 m) | 0 / 1 | 0 / 2 |
| sessions failed | 0 | 0 |

Every 40 m session was faster with the offer (84→75, 91→74, 92→82, 76→67, 92→79 s on the first
round). The saving is where §3 said the link bench could not see it: the keying of a
transmitter per change of direction.

Two faults the first rounds found, fixed before the table:

1. **A frame under the station's own transmission** (`aetherd`, both builds): a frame that did
   not decode and lay more than 50 ms under this station's own keying was handed to the engine;
   one acquired late out of a burst already acknowledged took the burst to be still arriving,
   and the acknowledgement went again over the sender's next burst. Such frames are now counted
   (`frames_under_own_tx`) and dropped.
2. **A lost acceptance** (§2.4): the offering sender missed the acknowledgement that took the
   turn and the first frames behind it, waited out the acknowledgement and sent its burst again
   over the other station's. It now reads the turn as taken from a trusted data frame
   (`a_lost_acceptance_of_the_turn_is_read_from_the_burst_after_it`, both suites; without the
   rule the Rust test counts four collisions).

Open, not fixed:

* Seed 40's collision is not the offer's: a failed frame whose chips read mode 13 in a burst at
  rung 7 also read "none follow", and the receiver answered at the turnaround (ADR-0045) over
  the rest of the burst. A countdown from a frame that did not decode is believed when its
  acquisition is trusted; a mode faster than any the receiver has asked for says its chips are
  not.
* Seed 31's collision repeats with the offer on (three rounds) and not off; the two stations'
  recordings do not yet reconcile with the channel's keying record around it.

## 6. Decision pending

The author's: keep it — protocol 7, on by default, released with protocol 6 so that testers
update once — or drop it. Until then the code stays behind `offer_turn`, off.
