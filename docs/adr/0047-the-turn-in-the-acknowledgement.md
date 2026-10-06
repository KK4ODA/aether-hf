# ADR-0047: The turn offered at the end of a burst, taken in the acknowledgement

**Status:** proposed, 2026-10-06 — built in the model, off (`LinkConfig.offer_turn = False`); not
ported. Flags `ControlFlags.OFFER` (TURN) and `TAKEN` (ACK); `LinkEngine._send_burst(prefix=…)`,
`_on_ack`, `_on_control`, `_send_ack`; stats `turn_offers`, `turns_taken`; `bench_chat.py`'s
`request+offer` policy.

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
4. A lost acknowledgement falls back to the rule two senders already follow: data heard wins.

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

## 4. Decision pending

The author's: port it and measure on the harness (protocol 7 if kept), or drop it. Until then
the code stays in the model behind `offer_turn`, tested both ways
(`test_the_turn_on_offer_is_taken_in_the_acknowledgement`).
