# ADR-0128: The first burst after a call starts safe and tries the bold rung with one frame

**Status:** proposed, 2026-11-16 — **results in, awaiting the author.** It fails the registered
criteria (F2, F3, F4; `docs/research/first-burst/01-results.md`). It is behind
`GrantConfig.first_burst_try`, off by default, and nothing ships. An exception the author approved to
the rule of no more patches to the rate controller; no review follows it. **A variant (J)**, moving
straight to the bold rung when its one frame decodes, was registered on 2026-11-16 at the author's
request (`00-plan.md` §5); it fails F2–F4 too, by less
(`01-results.md` §8).

## 1. Context

Before any data, C1 (ADR-0094, ADR-0099) chooses the first rung by its optimistic prior: the AWGN curve
`prior_margin_db` under the call's reading (1 dB at 2 300 Hz, ADR-0111; 3 dB at 500 Hz). On a fading
path a rung needs 6–10 dB more than on AWGN, and a call of one or two frames often reads a peak. On the
link bench's fading pipe the caller's first burst lost most of its frames on five of six paths, about
55 s of resending in six exchanges, and C1 then settled 1–3 rungs lower (`00-plan.md` §1). A cautious
start for the first rung alone was rejected in October (`rate-control/04-results.md` §3.6): it cost the
paths where the climb is the job.

## 2. Decision (proposed)

For each station's first choice in a session (the controller's seed), with the switch on:

1. The first rung is the best by the four channels' mean prior (`CLASS_PENALTY_DB`) at the reading, or
   today's if that is lower.
2. C1's try is armed at once: one new frame of the first transmission goes at the fastest usable rung of
   the first rung's family that is no faster than today's first rung. One family a transmission: a try
   across the floor's edge would take the whole transmission, which is today's start, so across the
   edge there is no try in the first transmission.
3. The try is decided by its one frame: decoded, it ends and the climb on clean frames (`_clean_climb`)
   goes on from the first rung; lost, that rung's next try waits twice as long.
4. Every later decision is today's.

The port carries the four channels' penalty tables again (`class_penalty_db`), held to the model's by
the link vectors.

## 3. What it removes

Nothing while the switch is off. If adopted, the decision deletes the switch and the first choice by the
optimistic prior.

## 4. Consequences

Measured on the harness, 25 scenarios and 114 paired runs an arm, with the flagged scenarios run
again (`01-results.md`):

* The first burst works as intended. First-burst blocks sent again went from 36.8 % to 22.6 %, and
  the air of their later copies from 486 to 219 s. The 500 Hz NVIS and edge scenarios ran ×0.93–1.00.
* The rest of the session pays. Air is ×1.083 pooled, and six scenarios are confirmed ×1.19–1.40
  (Trimode gateway, equatorial flutter, QSB, 500 Hz Winlink, 500 Hz DX, 2 300 Hz steady mail). Test
  messages are ×0.873 pooled, and the low-reading start pair's message is ×0.67.
* Completion and safety hold: 113 passes against 112, no new radiated overlap, no invariant broken.

The cause: the one bold frame measured the bold rung, but one frame outweighs nothing against the
prior, so the station then climbs from the safe rung one try (three frames) a rung. The same cost
rejected October's cautious start (`rate-control/04-results.md` §3.6).

**Variant J** (moving straight to the bold rung when its one frame decodes; `01-results.md` §8):
first-burst resends 37.3 % → 20.1 %, but air is ×1.051 pooled, four scenarios are confirmed
×1.28–1.33 and Test messages are ×0.855. The loss is entirely the starts on the floor where today's
first rung is OFDM (×1.14 over 39 runs; every other start ×1.006). Those same starts hold J's gains
on the 500 Hz NVIS edge (×0.83–0.89).
