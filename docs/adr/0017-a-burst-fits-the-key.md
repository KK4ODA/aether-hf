# ADR-0017: A burst fits the key; a session ends on the air; the Test climbs before it carries

**Status:** accepted, 2026-09-25. Model first (`aether_model/link/engine.py`, `sim.py`), then
the port (`aether-link`), then the daemon and the desktop application. No change to the frames
or the link protocol: a beta.57 station and a beta.58 station connect and carry data.

## 1. Context

Three Test sessions between KK4ODA (Senoia, GA; 50 W) and ND1J (Atlanta; 100–120 W) on the
night of 2026-09-24/25 — 40 m at 2 300 Hz, 80 m at 2 300 Hz, 80 m at 500 Hz — looked like a
modem that would not climb: on the 80 m sessions, which the operators judged excellent, the
panel said *0 rungs* throughout and the tests ran long. Both stations' recordings, sidecars
and logs were read frame by frame. What they show:

* **The ladder never ran.** The Test ran probe → call → message → file → ladder, and all three
  runs ended — the budget, an abort, ND1J closing the application — before the ladder step.
  *0 rungs* was true, and meant "not reached", not "failed".
* **The link did climb, and was pulled back to the floor by its own transmitter.** On 80 m at
  2 300 Hz the session started at rung 7 (BPSK ⅓), stepped down through 5 and 4 to 3, which
  genuinely failed, then decoded 9 of 9 frames at rung 2; at 500 Hz it decoded 6 of 6 at
  rung 5 (QPSK ½) and climbed to rung 7 (8PSK ½). Both then fell to the tone floor and stayed
  there. A burst is six frames, and a tone frame is 5.36 s: a tone burst is 32 s of audio
  plus the key lead and tail, and the key-time watchdog (`max_key_s`, 30 s) cut every one
  of them — 7, 6 and 6 trips in the three sessions. The cut frame is acquired and fails
  to decode, the rate controller counts a failure, steps down, and the next burst is tone
  again: once on the floor, a session could not leave it. This has been so since the tone
  floor arrived (beta.52); the bench never saw it because the simulators had no key.
* **The path itself was not excellent in the direction that mattered.** ND1J heard KK4ODA at
  about 0 dB on 80 m (+5 dB the other way, at twice the power), and the message was sized
  from the wrong direction — the SNR this station heard, not the one the message travels at
  — so a kilobyte took five minutes of a ten-minute budget.
* **An abort did not stop the transmitter.** A long tone burst ran on after *Stop test*
  until the watchdog cut it, ten seconds later, with the DISC queued behind it: the log has
  `watchdog: key time exceeded (Idle)` 11 s after the first test's own abort and 10 s after
  the operator stopped the second.
* **A session that ended without an orderly close was never identified**, and closing the
  desktop application in a session left the other station sending into nothing until its
  link timed out (ND1J's first 80 m session).

## 2. Decision

1. **A burst fits the key.** `LinkConfig.max_burst_s` caps the frames a burst carries at what
   fits in that many seconds (never fewer than one); the link timeout's exchange is sized from
   the same capacity. The daemon sets it from its own key budget: `max_key_s` less the key
   lead and tail and a second in hand — so a tone burst is five frames (26.8 s) where it was
   six — and, for a station that identifies, less the Morse identifier too on the bursts it
   rides on: the first transmission's, and those shaped within a minute of the identifier
   falling due (four tone frames). An identifier that would still overrun the key waits for
   the next transmission. Model first: `TwoStationSim` gained a transmitter key limit
   (`key_limit_s`), and a frame the key cuts arrives undecodable, as on the air.
2. **An abort cuts the burst.** The rest of the burst on the air and any queued behind it are
   dropped at once, the sound card is flushed and the key released; the DISC goes out after a
   pause long enough for the cut frame's remainder and the other station's answer to pass,
   so the other station hears it rather than timing out.
3. **Every session's end is identified.** When a station that identifies has transmitted
   since its last identifier and a session ends — however it ends — the next transmission (the
   DISC or DISC_ACK) carries the identifier whatever the interval says, or one goes out on
   its own when nothing else will (a link timeout, a peer that went silent). Exactly one.
4. **A stopping daemon ends the session on the air**: it aborts (the DISC, and the
   identifier), waits until nothing is left to send or twelve seconds pass, then stops. The
   desktop shell waits for it (its kill timeout is now 20 s), and before closing the window
   it asks, naming what would be interrupted — a session, a Test, a call, a probe, a
   transmission — with *Keep running* as the way out.
5. **The Test climbs before it carries.** The order is probe → call → message → **ladder** →
   file → disconnect; the message is sized from how the other station hears this one (the
   probe answer's `heard_there_db`), and the file from what the budget has left after the
   ladder. `status.test` reports the step of six, the time elapsed and the most the budget
   leaves (no countdown), the bytes acknowledged, the rung under test out of the ladder, the
   highest rung passed, the failures in a row, the last rung's verdict and the rung the link
   is using; the panel shows all of it where it showed *N rungs*.

With it, three things the operator asked for: a **session history** (`sessions.json`,
`sessions.list`, the Sessions list on the Stations tab — one line per session, where the
stations heard keep one per callsign), a **Sent** pane beside Received, and an **uninstaller**
that asks before removing settings, profiles, logs or recordings, one kind at a time, keeping
them by default.

## 3. Alternatives considered

* **Raise the watchdog.** It is the last line of defence against a stuck transmitter and the
  operator's setting; the modem must fit inside it, not argue with it.
* **Fewer frames for the tone family only.** The capacity rule is the same for every family
  and costs nothing where six frames fit (OFDM: six 1.05 s frames), so there is no special
  case to keep.
* **Treat a cut frame as not sent.** The receiver cannot tell a cut frame from a faded one,
  and the transmitter should not be sending what it knows will be cut.
* **Identify at the start of every transmission.** Longer bursts, more of them cut; §97.119
  asks for the end of each communication and every ten minutes, which (3) and the interval
  already give.
* **Keep the ladder last and cap the transfers harder.** The ladder is the measurement the
  Test exists for; the file is the one step that can be shortened to whatever time is left.

## 4. Consequences

* On the model's simulators with a 30 s key, a 2 kB session at −4 dB (2 300 Hz) took 139 s
  instead of 614 s, and at −8 dB (500 Hz) 247 s instead of 778 s; retransmissions fell from
  84–114 to 0–3. `test_a_burst_fits_the_transmitters_key_time` (model) and
  `a_burst_fits_the_transmitters_key_time` (port) hold it; `a_burst_ends_inside_the_key_watchdog`,
  `an_abort_drops_the_rest_of_the_burst` and `a_session_ends_with_an_identifier_however_it_ends`
  hold the daemon.
* A tone burst carries a sixth less per turn. Nothing slower was measured: the frame that
  no longer fits was the one the watchdog destroyed.
* A station that identifies sends one more identifier per session when the session ends
  inside the interval; a station that does not identify is unchanged.
* The Test report gains `ladder_rungs` and `highest_passed`; a run on a slow path now
  measures the ladder and shortens or skips the file.
* Open: the 80 m rung-3 failures at about 0 dB (tone50-75 is a 50-baud kind on a dispersive
  NVIS path) and whether the tone floor's own SNR reading (a lower bound, ADR-0016) held the
  controller low after the floor calls — both want the next on-air runs with this build.
