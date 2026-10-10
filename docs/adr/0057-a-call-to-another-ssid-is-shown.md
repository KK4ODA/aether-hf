# ADR-0057: A call to another SSID of the station's callsign is shown, and an unanswered call suggests one

**Status:** accepted, 2026-10-10. Daemon (`station/bandwidth.rs`: `kin_of_ours`, `note_sender`,
`suggest_kin`, `KIN_HEARD_S`, `Mismatch.called`) and panel (the mismatch banner's *Answer as … too*
and *Call …*). No wire change.

## 1. Context

On 2026-10-10 KO4WX called **KK4ODA** for some minutes; the station answered as **KK4ODA-1** only, and
ignored every try without a word — the engine does not answer a call addressed to somebody else, and
nothing told either operator that the two names were one station. He was also at 2 300 Hz against
this station's 500 Hz, and the bandwidth warning (ADR-0035) only covered calls to the station's own
callsigns, so that said nothing either.

## 2. Decision

1. **A call or probe to another name of one of the station's callsigns** — the same base callsign
   (`base_callsign`: SSID, `-T`, `/P` off) — in a bandwidth the station answers is shown on the
   mismatch banner (`what` = `callsign`, `called` = the name called): *KO4WX is calling KK4ODA, which
   this station does not answer to: it answers as KK4ODA-1.* The banner's **Answer as KK4ODA too**
   adds the name to the callsigns the station answers to (`callsigns.set`) until the modem restarts
   or a host program names its own; the caller's next try is answered.
2. **The bandwidth warning covers those names too**: a 2 300 Hz call to KK4ODA of a 500 Hz station
   answering as KK4ODA-1 is the ADR-0035 warning, naming the callsign called.
3. **A call of this station's that nobody answered** suggests, when a station under the same base
   callsign with another SSID was heard within `KIN_HEARD_S` (30 min) — any decoded frame naming it
   as sender — calling that one (`what` = `suggest`): *No answer from KK4ODA. KK4ODA-1 was heard 3
   minutes ago: if that is the station you meant, call KK4ODA-1.* The banner's **Call KK4ODA-1** does.

Not done: answering a call to another SSID by itself. Two stations of one operator under different
SSIDs are a real arrangement (a gateway and a home station); which name answers is the operator's.
