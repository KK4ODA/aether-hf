# ADR-0026: The bandwidth follows the host program and the caller; LISTEN decides whether calls are answered

**Status:** accepted, 2026-09-26. The link engine (`LinkEngine.set_air`, `set_max_mode`, model
first), the daemon (`station/bandwidth.rs`, `station/host.rs`, `bandwidth.set`,
`status.bandwidth`, `status.answering`, the `bandwidth` event, `[radio] bandwidth` live), the
host adapter (`BW500`/`BW2300`/`BW2750`, `LISTEN`, `CHAT ON`), the replay (`air` events), and the
panel's bandwidth chip.

## 1. Context

A station ran one bandwidth, `[radio] bandwidth`, and changing it restarted the modem. The host
adapter answered `BW<n>` `OK` only for that one and `WRONG` for the other, so an operator going
from VarAC (500 Hz on its calling frequencies) to Winlink Express or Pat (2300 Hz) changed Setup
and waited for a restart each time — and a caller in the other bandwidth was simply not
answered, the classic "station won't answer" of ND1J's Winlink P2P article.

VARA's published TNC command list (EA5HVK, *VARA Protocol Native TNC Commands*, 13 February
2022) says what these commands do: `BW500` "Set VARA HF to 500Hz Narrow mode", `BW2300` to
2300 Hz standard mode (the default), `BW2750` to 2750 Hz; `LISTEN ON` "Incomming connections
enabled", `LISTEN OFF` — **the default** — disabled; `CHAT ON` "Includes the LISTEN ON command".
VARA's own settings also offer *Accept 500 Hz connections* on a 2300 Hz station. Aether's host
adapter had recorded `LISTEN` and answered every call either way.

Two things make following possible. The two waveforms share their numerology (8 kHz baseband,
the same symbol, the same audio rate) and differ in carriers, band filters, ladders and
acquisition tables, all built from `WaveformParams` — so a station can rebuild its physical
layer in a few milliseconds between blocks. And calls begin on the tone floor (ADR-0016), whose
frames are the same on both airs: a 2300 Hz station decodes a 500 Hz station's first try, and
already did — and ignored it by the handshake's bandwidth bits.

## 2. Decision

* **The engine runs one air at a time and moves between sessions.** `LinkEngine.set_air(timing,
  capabilities, max_mode)` replaces its timing, the capability byte it offers (the bandwidth
  bits), its fastest rung and its rate controller; refused while a session, a call or a probe is
  under way. Callsigns, counters, session numbering and the last probe stay. Model first (both
  suites: `test_a_station_moved_to_another_air_answers_calls_in_it`).
* **The station moves its physical layer with it** (`Station::move_to`): the streaming receiver,
  the modems, both band filters, the busy detector (its floor was learned through the other
  filter), the occupancy the rules judge by, the engine. It is moved only when nothing is under
  way on the air it would leave — no session, call, probe, Test, nothing queued or on the air.
  The new receiver counts its samples from the move (`rx_origin`), so frame times stay in the
  station's clock.
* **A host program's `BW<n>` moves the station** (`bandwidth.set {hz}`): at once when idle, and
  the adapter says `OK`; when something is under way it stays, and the adapter says `WRONG`
  unless the station already runs what was asked. `BW2750` is 2300 Hz: a narrower signal is
  always inside what was asked. The request holds while the program is attached; when it goes,
  the station goes back to its own once idle.
* **A 2300 Hz station answers a 500 Hz call at 500 Hz**, as *Accept 500 Hz connections* does:
  a decoded connect request to one of its callsigns, stating 500 Hz and this link protocol, moves
  the idle station before the engine sees it, and the engine answers on the caller's air. After
  the session and [`RETURN_QUIET_S`] (20 s) of quiet — the other station's goodbye on the floor,
  the call that often follows a ping — it goes back. **Never the other way**: a 500 Hz station
  does not answer a 2300 Hz call, because a 2300 Hz signal where the operator or the program
  chose 500 Hz — a 500 Hz calling frequency, most of all — is nobody's choice.
* **`[radio] bandwidth` is the station's own and is live**: a change moves the station once idle,
  with no restart. `[radio] max_mode` indexes the ladder of whichever bandwidth runs, as it always
  said, and a change of it now reaches the engine too (`set_max_mode`) — it had reached only the
  gate and the Test's ladder, and the link kept the value it started with until a restart.
* **`LISTEN` decides whether calls are answered while a host program is attached**: off until the
  program says `LISTEN ON`, `LISTEN CQ` or `CHAT ON`, as VARA's default is. A call or a probe to
  the station is then kept from the engine, counted (`calls_unanswered`) and said once a minute a
  caller. With no program attached the station answers, as it always did. The published note that
  `LISTEN` received mid-connection disconnects is not copied: it reads as a side effect to avoid.
* **Visible and replayable**: `status.bandwidth` (the running and own bandwidth, a host's request,
  why, the caller) and `status.answering`; a `bandwidth` event on every move, which the host
  adapter follows for `CONNECTED` and `BITRATE`; a header chip on the panel while the station runs
  another bandwidth than its own; and an `air` event in a recording, where the replay builds its
  receiver again as the station did.

No configuration key and no wire change: both behaviours are VARA's, and the published commands
are the operator's switch (a program that should not move the station sends no `BW`).

## 3. Consequences

* One configuration serves VarAC and Winlink Express; the host program's own bandwidth setting
  is what counts while it is attached, as with VARA.
* A 2300 Hz station on a 500 Hz calling frequency is reachable by 500 Hz stations. It hears only
  their floor tries (the narrow OFDM tries are silence to it), so the first answer comes a try
  later at worst.
* A host program attached without saying `LISTEN ON` takes the station off the air for incoming
  calls. Every program known on the bench says it (VarAC `LISTEN ON`/`CQ` and `CHAT ON`, Winlink
  Express `LISTEN ON`); one that does not behaves as it would with VARA.
* The rules (ADR-0018) judge each transmission on the air it goes out on; a narrower signal at
  the same dial is inside the wider one's span.
* Rejected: a configuration switch for either behaviour (VARA has none for `BW`, and the option
  it has for answering is the one Aether now always does); answering a wider call; queuing a
  host's `BW` until a session ends (a program told `OK` calls at once).

[`RETURN_QUIET_S`]: ../../core/aetherd/src/station/bandwidth.rs
