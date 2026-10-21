# ADR-0085: A stranded frame goes at the most robust rung of its new family

**Status:** accepted, 2026-10-21. Model first (`GrantedEngine._reencode_target` in
`model/aether_model/link/granted.py`), then the port (`reencode_target` in
`aether-link/src/granted.rs`), held together by the regenerated `granted_vectors.json`.
**No wire change, no configuration key.** A sender re-encodes the same frames as before, at a
slower rung.

## 1. Context

The harness A/B for ADR-0082…0084 (56 scenarios × 2 draws against beta.117) found one new
failure: `80m-agc-carrier-outside-500` at its own seed, 31 — ITU NVIS (7 ms, 1 Hz) at 10 dB on
the 500 Hz air, a `quick` scenario, whose 1 kB message must arrive within 300 s of air.
beta.117 delivers it in 280–290 s. This build either delivered it in about 203 s or lost it.

The sender's records of the lost runs all show the same thing. The session's first burst went
at the first OFDM rung (QPSK ½, 22 data bytes a frame), which a 7 ms echo past the 5 ms prefix
does not carry. After four copies (`max_combines`), the frames were re-encoded at the
recommendation, then rung 4, tone4x100-93. Its 10 ms symbols are smeared by the same echo, so
rung 4 does not carry the path either. The frames went four more copies there, 4 × 4 × 5.36 s,
about 86 s, before they could be re-encoded again. Where the recommendation was rung 3 or
lower at that moment, the same frames got through at once.

The re-encoding rule took the slowest rung at or above the recommendation that the body fits,
which is usually the recommendation itself. Every data frame of a family is the same length:
at 500 Hz every floor frame is 5.36 s and every OFDM frame 1.054 s, and at 2 300 Hz likewise.
So for a frame whose body is already fixed, a rung below the recommendation in the same family
costs no air. The recommendation is the receiver's guess for the session's new data, filled to
the rung's capacity. A stranded frame is a frame the path has already refused four times.

## 2. Decision

A stranded frame is re-encoded at **the most robust usable rung, in the family of the
recommendation, below its present rung, that its body fits**:

* *usable*: on the rate controller's ladder (`RateController.modes`), so never a beaten rung
  (2 300 Hz rung 6, BPSK ⅕; 500 Hz rung 8);
* *the family of the recommendation*: a frame never goes into the floor while the
  recommendation is an OFDM rung. That would make it five times as long;
* when and why a frame is re-encoded is unchanged (`max_combines` copies, or
  `reencode_early_after` when hopeless, ADR-0038), and so is the one family per transmission.

A 22-byte body stranded at 500 Hz rung 5 with the recommendation at rung 4 now goes at rung 1,
tone-36, rather than rung 4. On the 2 300 Hz air a 20-byte body under a fast-tone
recommendation goes at tone-36 too. Among the OFDM rungs a body goes as low as it fits.

## 3. Measured

**The scenario**, its own draw (seed 31) run again and again. Runs of one build differ (the
daemons' threads), and this draw sits near its budget on every build:

| `80m-agc-carrier-outside-500`, seed 31 | delivered | message, median (range) |
|---|---|---|
| beta.117 | 9 of 10 | 286 s (203–290) |
| ADR-0082…0084 | 6 of 8 | 203 s (203–285) |
| ADR-0082…0085 | 11 of 12 | 259 s (203–294) |

Builds with ADR-0084 and one of ADR-0082 or ADR-0083 left out lost two runs in four more, the
same way. A session whose first burst goes on the floor (the call read just under the first
OFDM rung) delivers in about 203 s on every build; one whose first burst goes at the OFDM rung
strands it there, and that is where the builds differ. The one lost run of beta.117 and the one
of this build fail the same way, and not by this rule: §5.

**The link bench** (the fading pipe, the old rule against this one, the same seeds): identical
at every point. 500 Hz, AWGN, Good, Moderate and Poor at −8, −4, 0 and +4 dB, 20 sessions of
1 kB and 12 of 8 kB a point. The pipe seldom strands a frame (4 re-encodes in 45 sessions of
8 kB), and those it strands decode at either rung.

## 4. Alternatives

* **Re-encode sooner** (after two copies once re-encoded). It halves the waste and keeps it.
  The frame would still go first at a rung the path may not carry, for nothing.
* **Re-encode at the slowest rung that has delivered in the session.** It needs a record the
  rule above makes unnecessary: in the same family the most robust rung costs nothing.
* **A memory of the rung that lost a whole burst**, held until a reading clears the one it
  failed at by a few decibels. On a fading path the smoothed reading after a fade comes back to
  where it was, not above it, so the memory would hold a rung down for the session. ADR-0084 §4
  measured a per-rung offset to no effect, and the remaining loss here (§5) is not the
  controller's choice of rung for new data alone.

## 5. Consequences

* A frame the path has refused goes next at the rung most likely to carry it, at no cost in
  air.
* New data still goes at the recommendation; the rate a station climbs to is unchanged.
* Nothing an operator sees changes but the time a stalled transfer takes to recover.
* **Open:** a body that fits no slower rung stays where it is. Both lost runs of §3 sent their
  new data at rung 3 (72 data bytes) after the first OFDM burst failed, and on this path rung 3
  decoded 2 of 20 frames: those bodies fit no rung below. Splitting a body across two frames
  needs a wire change (each body has its sequence number), so it waits for a protocol bump that
  has other reasons.
