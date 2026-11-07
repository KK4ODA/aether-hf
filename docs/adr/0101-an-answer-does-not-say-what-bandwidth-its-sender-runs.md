# ADR-0101: An answer does not say what bandwidth its sender runs

**Status:** accepted, 2026-11-06, a fix to ADR-0035's learning. Daemon only
(`core/aetherd/src/station.rs`, the frame report): **no wire change, no model change, no
configuration key.**

## 1. Context

ADR-0035 has a station learn the bandwidth another runs from any frame that states one — a
beacon, a call, an answer, a probe, a probe answer — and a 2 300 Hz station that calls one known
to run 500 Hz move to 500 Hz first (`move_for_call`). ADR-0026 has a 2 300 Hz station that is
called at 500 Hz move to 500 Hz to answer, so its answer states 500 Hz.

On 2026-11-06 the author's two stations, KK4ODA (the mobile) and KK4ODA-1 (the gateway), both set
to 2 300 Hz, ran both of the night's sessions at 500 Hz. Each station's heard list said the other
ran 500 Hz, from some earlier session. The mobile moved to 500 Hz to call (*calling KK4ODA-1,
which runs it*), the gateway moved to 500 Hz to answer (*KK4ODA called in it*), and the answer,
stating 500 Hz, renewed the mobile's belief. Neither station sent the other anything that states
2 300 Hz — no beacon, no probe — so nothing corrected it. The alternate tries of each call, which
go in the ordinary OFDM family (ADR-0016), were spent on an air the 2 300 Hz gateway cannot
decode: the second session took three tries and 46 s to connect.

An answer goes in the bandwidth of the call it answers, whatever its sender runs. It says nothing
of what the sender runs.

## 2. Decision

An answer to a call (`ConnectAck`) is reported with no bandwidth: the station learns nothing from
it, and the heard list keeps what it had. A beacon, a call, a probe and a probe answer still teach,
as before. A probe is answered without moving, so its answer states the bandwidth the answering
station runs.

## 3. Consequences

* A 500 Hz session no longer makes the callee look like a 500 Hz station to its caller.
* A belief already held stays until something corrects it: the author's two stations each probe
  the other once (the Session tab's *Probe*), and the answer says 2 300 Hz.
* **Left open:** a call made by a station that moved for it (`Why::Calling`) still states the
  bandwidth it moved to, and the called station learns that. It matters when the gateway calls a
  client it wrongly believes narrow, which is rare. Telling the two apart needs the sender to say
  so — a "moved for this call" bit in the capability byte (bits 5–7 are free) — which is a wire
  change and the author's call.
* **Also open, the author's call:** a host program's own bandwidth does not stop
  `move_for_call`. The mobile moved to 500 Hz to call with Winlink Express attached, and Winlink
  Express asks for 2 300 Hz: after the first session the mobile went back to 2 300 Hz because
  *the host program asked for it*.
* Test: `a_wide_station_answers_a_narrow_call_at_500_hz_and_goes_back_after` now checks that the
  narrow caller's report of the answer carries no bandwidth and that it has learnt none.
* Manual: the troubleshooting entry for a session that runs at 500 Hz between two 2 300 Hz
  stations.
