# ADR-0127: The first burst after a call starts safe and tries the bold rung with one frame

**Status:** proposed, 2026-11-16 — behind `GrantConfig.first_burst_try`, off by default, judged on
the scenario harness against criteria registered before any run (`docs/research/first-burst/00-plan.md`).
An exception the author approved to the rule of no more patches to the rate controller; no review
follows it. Results: `docs/research/first-burst/01-results.md`.

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

To be filled from the results: the criteria F1–F6 of `00-plan.md` §4.
