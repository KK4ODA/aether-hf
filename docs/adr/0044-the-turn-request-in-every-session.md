# ADR-0044: The station without the turn asks for it, in every session

**Status:** accepted, 2026-10-06, the author's decision. `LinkConfig.chat` defaults to on in both
suites; the daemon no longer switches it with a host's `CHAT ON` (`station/host.rs`). No wire
change: every version since ADR-0027 answers a request.

## 1. Context

ADR-0027 gave a chat session a turn request: the station that does not hold the turn, with
something to send, asks for it as soon as the channel is quiet — an acknowledgement nobody asked
for, with WANT_TX, which an idle sender answers with a TURN — instead of waiting for the sender's
next poll, up to `keepalive_s` (10 s) on an idle link, and then for the poll's answer and a TURN.
It was on only under a host program's `CHAT ON` (VarAC). Winlink programs never say it, and a
Winlink B2F session changes direction at every handshake line, proposal, answer and message.

The stress scenarios (ADR-0043) put a number on it. On the link bench with a host that answers
the moment a line arrives — B2F's shape: `bench_chat.py` with 0.2–1 s between a line's delivery
and the reply — a reply's median wait was 13 s at +6 to +12 dB on every fading class at both
bandwidths, and 3.4–6 s with the request, with no line lost, no session dropped and slightly less
keyed time (2.2–5.6 s a line against 2.5–5.7). At 0 dB the request still halved the wait at
2300 Hz (13 → 7–8 s) and took 1–4 s off at 500 Hz.

## 2. Decision

The request is part of every session. `LinkConfig.chat` keeps its name — where it began — and
`set_chat` stays so that a bench can compare the engine before (`bench_chat.py`'s `base` policy
is now `chat = False`). A host's `CHAT ON` still decides the KISS programs' priority (ADR-0019).

## 3. Measured on whole sessions

The scenario harness, this build against beta.82, the same seeds, two Winlink-shaped scenarios
(`*-winlink-exchange-*`: a call, then 60, 80, 120, 40, 1 500, 120, 40, 2 000 and 40 bytes
alternating direction, each sent the moment the last arrived, and a disconnect):

| scenario | beta.82 | this build |
|---|---|---|
| 40 m, 2300 Hz, Good +10 dB | 131 s of air | 100 s |
| 80 m, 500 Hz, Moderate +6 dB | 436 s | 326 s |

The other eighteen scenarios move within what two runs of one build differ by (the daemons'
threads make each run its own): the nightly set passed better than on beta.82 (three Tests that
aborted on the file completed), and a PACTOR-and-SignaLink reply that took 129 s against 55 lost
its first frame four bursts running on the Poor path after a turnaround that took 9 s.

## 4. Consequences

* `turn_requests` counts in every session; the control API's `status.host.chat` says what the host
  said, which no longer changes the link.
* Found on the way, not fixed: a link that falls to the tone floor on a Moderate 500 Hz path can
  stay there for minutes with every frame decoding, since the floor's SNR reading is a lower bound
  (ADR-0016) — the "trial climb off the tone rungs" ADR-0020 left for evidence, which this is
  (`40m-wide-calls-narrow`, one run in two).
