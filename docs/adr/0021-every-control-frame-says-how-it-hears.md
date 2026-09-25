# ADR-0021: Every control frame says how its sender hears the other station

**Status:** accepted, 2026-09-25. Model first (`aether_model/link/engine.py`), then the port
(`aether-link` `engine.rs`) and the daemon (`station.rs`, the session history). The frame
format is unchanged; a station of an earlier version sends "unknown" where this one sends a
reading, and ignores the reading this one sends.

## 1. Context

The session history (beta.58) shows how the other station heard this one — the SNR it reported
— beside how this one heard it. A station reports that SNR in its acknowledgements, so only a
station that *sent* data ever learns it. In ND1J's sessions of 2026-09-25 he called and sent,
KK4ODA-1 only acknowledged, and every row of its history read "—" for *heard there*: the
column looked broken, and the one number an operator cannot read off their own receiver was
missing from exactly the sessions where the other station did all the talking.

Every CONTROL frame already has an SNR byte; only the acknowledgement filled it.

## 2. Decision

* A station's control frames other than acknowledgements — poll, turn, disconnect and the
  disconnect's answer — carry the SNR of the last frame of the session it decoded from the
  other station (`_heard_peer_db`: the connect frame, then each data or control frame of the
  session). An acknowledgement still carries the SNR of the burst it answers (ADR-0020).
* A station that receives such a frame with a reading takes it as how the other station hears
  it (`peer_snr_db`), as it does an acknowledgement's.
* The report is the session's and goes when the session ends, as before; the one the session
  ended with is kept as `ended_peer_snr_db`, because a disconnect is the frame that brings it
  to a station that only received, and the daemon writes the session's account once the
  session has ended. The history takes it first.

## 3. Consequences

* A station that was called and only received learns, from the caller's disconnect, how it was
  heard; its history shows it. A session that ends without a disconnect (a timeout) or with a
  station of an earlier version still has none, and the panel says so in words.
* Tested in both suites (a one-way session over the simulator leaves the receiver with the
  caller's reading) and in the daemon's history test.
