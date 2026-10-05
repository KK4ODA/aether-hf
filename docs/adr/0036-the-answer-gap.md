# ADR-0036: The answer gap — a station waits a set time after another station's frame before it keys

**Status:** accepted, 2026-10-05. The daemon (`radio.answer_gap_ms`, live; `Station::held_for_busy`,
`heard_end`, `StationStats::deferred_for_gap`), configuration schema 10 (`answer_gap`, a step that
changes nothing), and the panel (Setup step 4, *Answer gap*). No wire change, no engine change.

## 1. Context

KE4QCM, 3.590 MHz at 500 Hz, five sessions on 2026-10-05 00:16–00:51Z (sidecars on the author's
machine): his calls decoded at this station at 1–6.5 dB, this station's acceptances mostly never
reached him — he called again after being accepted, a probe went unanswered, and when he did read
this station it was 8.5–13 dB weaker than this station read him. WC4Y did the same at 00:51. With
VARA HF, the same two stations connect reliably to this one at 35–50 W, same radio, same antenna,
so neither the path nor the drive explains it (the drive was measured: ALC just moving, the tone
floor's constant envelope reading 75 W on the FTDX10's PO meter).

KE4QCM keys his radio with a SignaLink. A SignaLink keys by VOX, and its DLY knob holds the
radio on transmit for a while after the audio stops. Aether answers fast:

* A connect acceptance and a probe answer are queued the moment the frame decodes, and a tone
  frame decodes on its last sync block, so they key at the end of the frame (the 00:51 sidecar:
  a decoded frame reported at 46.158 s, the key at 46.158 s).
* An acknowledgement goes `turnaround_s` (0.25 s) after the frame's end.
* The 01:02 recording, band-limited around the 500 Hz waveform: WC4Y's signal drops 0.1–0.2 s
  of this station's capture clock before this station keys, and its audio follows the key lead.

An answer that starts while the other radio is still holding its key, or still switching back
(the FTDX10 alone delivers 250–325 ms of digital silence after unkeying,
`CAPTURE_LAG_ALLOWANCE_S`), loses its start, and with it the preamble or the first sync block the
receiver acquires on. A call follows silence and is heard; an answer follows the other station's
transmission and is not — the pattern of both sessions. WC4Y keys an IC-7300 over USB, which
switches quickly, and his 01:02 session ran.

## 2. Decision

1. **`radio.answer_gap_ms`** (0–2000, default 0, live): the least time between the end of a
   frame heard from another station and this station keying. The daemon keeps where the last
   frame it heard ended (`heard_end`, every frame handed to the engine, read or not — an
   unreadable frame is answered too), and `held_for_busy` holds whatever radiates until
   `heard_end + gap`, moving the engine's timers by the hold (`on_tx_delayed`) exactly as a
   busy-channel hold does. Holds for the gap are counted apart (`deferred_for_gap`, in the
   `diagnostics` counters).
2. **In the daemon, not the engine.** Acceptances and probe answers go on decode, acknowledgements
   and polls on `turnaround_s`, the identifier and datagrams on their own paths; one hold in front
   of the key covers all of them. Raising `turnaround_s` would have missed the acceptances — the
   very frames lost — and would have changed every wait budget with them.
3. **The other station still takes a late answer.** A station waiting for an answer holds its
   wait for a frame it hears arriving (`on_preamble`: ADR-0016, ADR-0022, ADR-0030), and its wait
   runs a response's air time plus `turnaround_s`, `detect_latency_s` and the 0.4 s ACK margin
   past its own transmission, so an answer up to the frame's length later still announces itself
   inside it. Tested: a probe answered 0.8 s after it is taken
   (`an_answer_waits_out_the_gap_and_is_still_taken`), and a whole session with both stations at
   0.8 s delivers (`a_session_crosses_with_both_stations_answering_late`).
4. **Default 0.** The gap is the operator's fix for a station they work, set in Setup with the
   hint "500–800 if the other station keys by VOX (SignaLink)"; it costs every exchange the gap
   less the quarter second the link already waits. A non-zero default waits on the air's verdict.

## 3. Consequences

* It fixes one direction: this station's answers reach a VOX-keyed station. That station's
  answers to this one still go a quarter second after this station's frame ends, into this
  FTDX10's 250–325 ms of silence after unkeying — its own gap, on its own build, would fix that.
* Configuration schema 10: beta.74 cannot read a file with the key (every table refuses unknown
  keys); the shell's `SCHEMA_HISTORY` gains `("0.2.0-beta.75", 10)` with the release.
* Measured the same day and not built: ignoring OFDM detections within 4 % of the acquisition
  threshold. In the recordings it dropped 19 phantoms and no decoded frame, but the model at
  500 Hz rung 4 put 2 of 26 decoded frames at −8 dB and 1 of 38 at −7 dB at or below 1.04 — the
  weakest real frames, where these paths run — and 31 of the 50 phantoms sat at 1.1–1.5, among
  the real frames.

## 4. Open

* Whether a non-zero default pays: sessions with KE4QCM at 500–800 ms, and the cost on a fast
  path, will say.
* The link could learn a peer's turnaround — a frame that arrives, unread, where an answer was
  due — and widen the gap for that peer alone. Not before the setting has shown it is the cause.
