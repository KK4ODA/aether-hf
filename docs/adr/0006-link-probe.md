# ADR-0006: The link probe — a beacon with a destination, answered with the SNR heard

**Status:** accepted (2026-09-15) · **Code:** `model/aether_model/link/frames.py`
(`ProbeBody`, `DataKind.PROBE` / `PROBE_ACK`), `engine.py` (`probe`, `_handle_probe`,
`_handle_probe_ack`) · **Roadmap:** P7-1 · **Spec:** `docs/spec/air-interface.md` §7.1

## Context

A receiver can measure how well it hears a station; it cannot measure how well it is heard.
On HF that second number is the one an operator wants before calling: it says whether the
path works in the direction that matters for their traffic, and whether to change band,
antenna or power before spending a session finding out. The beacon (P3-6) gives a listening
station the first number only, and never draws an answer, by design.

Peer-to-peer operating practice around VarAC and VARA Chat starts a contact with a "ping"
for exactly this reason. What a ping *is* on the air is not published; on the host side it
is not a modem command at all — VARA's public command set has no `PING`, and VarAC's ping
is a short session (connect, exchange the reports, disconnect), which is what a modem
without a probe frame has to offer. (The `PING` / `PINGACK` vocabulary belongs to ARDOP's
host protocol.) A frame that asks the question directly costs two frames instead of a
session, needs no host program attached, and is one thing the user-facing benchmark against
VARA HF / VARA Chat (`docs/VARA-UX-BENCHMARK.md`) found the dashboard could not give
without a new air frame.

## Decision

Two DATA-container kinds, `PROBE` (4) and `PROBE_ACK` (5), sharing one sixteen-byte body:
source and destination callsigns packed as the connect body packs them, the SNR byte in
the CONTROL frame's convention (signed whole decibels, 3 kHz reference, ties to even,
clamped to ±40, 0x7F for "not measured"), and the capability byte with the bandwidth bits.

- A probe is sent outside any session — session id zero, sequence zero, the most robust
  mode — and exactly once; the prober waits one data frame's turnaround for an answer and
  then reports "no answer". No retries: a question the operator can ask again must not be
  able to fill a channel on its own.
- The probed station answers only when it is idle, only when addressed by one of its
  callsigns, and only when the probe's stated bandwidth is its own; the answer carries the
  SNR the probe arrived at. A station in a session ignores probes.
- The prober reports both directions: the SNR in the answer (how it is heard) and the SNR
  it measured on the answer (how it hears). The event is `probe` with the detail
  `"<call> hears us at <x> dB, heard at <y> dB"`; the answering side reports `probed`
  with `"<call> at <y> dB"`.
- Answering a probe is a *response* (§97.221(c)), so a station configured answer-only may
  answer one; sending a probe is a call and is refused there, as `connect` and `beacon`
  are.

## Alternatives considered

- **A short session as the probe**, which is what VarAC does over VARA. It works with what
  the modem already has, and it stays available; but it commits both stations to a
  handshake, its retries and a close, for a question two frames can answer, and it needs a
  host program to drive it.
- **A CONTROL-container probe.** Callsigns do not fit a seven-byte control frame; the
  connect frames had the same problem and the same answer.
- **Retries and a backoff, as connect requests have.** A connect request represents a
  commitment to a session; a probe represents a question, and a station that asked it
  three times without an answer would only be adding to the noise on a path that had
  already answered "no". One frame, one report.
- **Answering during a session.** A session's frames are considered more important than an
  outsider's question, and a probe addressed to a busy station is treated as if it had not
  been heard. The prober's "no answer" is then the honest report.
- **Carrying the probe's own SNR back in a session's first burst instead.** That is P9-2
  (the faster start), a different question: it uses the connect exchange a session already
  has, and needs no frame of its own.

## Consequences

- The control API gets `probe` and a `probe` event; the panel a Probe button beside Beacon
  and both numbers on the dashboard.
- The VARA-compatible host adapter gets no `PING` command: VARA's published command set has
  none, and VarAC's ping is a connect the adapter already serves
  (`docs/spec/host-interfaces.md`). A host protocol that does have one (ARDOP's
  `PING <call> <count>` / `PINGACK`) would map onto this frame if such an adapter is ever
  written.
- Two new engine statistics: `probes_sent`, `probes_answered`, `probe_replies`.
- A station running an earlier version ignores the two kinds (an unknown kind fails to
  decode), so probes are harmless to it and simply go unanswered.
