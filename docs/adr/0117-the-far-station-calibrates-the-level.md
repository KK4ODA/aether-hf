# ADR-0117: The far station calibrates the level, in the call and on the data

**Status:** proposed, 2026-11-14. This is the Aether FM proposal's FM-5
(`docs/research/aether-fm/00-proposal.md` §E, §J, §K, §L), from the author's request of
2026-10-24 (roadmap Phase 10, *Level tolerance*). It registers the design and the criteria before
any run. Step 10.3 builds it on the FM channel model, beside the waveform runs (ADR-0116), and
Gate 3 accepts it on §5's criteria. Nothing is built for it yet.

## 1. Context

From the author's experience of VARA FM: a link fails unless both stations' levels are right, and
getting them right is hard. An FM radio shows its operator no meter for their own deviation. On
HF the drive is set against the ALC; on FM only the other station's receiver sees what was sent.

Aether FM is built so that a wrong level costs speed, not the contact, and so that the far station
does the measuring. The scratch model gave the shape to design for (`01-measurements.md`):
- SNDR rises about 1 dB per dB below the limiter's knee and falls 2–3 dB per dB above it;
- overdrive by 10–15 dB still leaves 9–12 dB, enough for QPSK ½ call frames;
- one transmission at one level does not tell clipping from noise reliably.

## 2. Decision

### 2.1 The only thing the loop moves is the modem's digital amplitude

The operating system's volume, the radio's menus and the far receiver's level are set once and
watched, never moved by the loop (proposal §E.1). A starting value may come from a radio preset.

### 2.2 Stage 1: a level ladder in the call and in the acceptance

1. **The caller's transmission.** The call frame goes on a robust rung. In the same transmission
   come six segments of a known training sequence, 3 dB apart, about 120 ms each. It has the
   data's spectrum and peaks, never a tone.
2. **Where the ladder is centred.** On the stored best level for this radio and this peer, or on
   a conservative default. With a stored level, two or three segments verify it.
3. **What the called station measures** in each segment:
   - SNDR, from the known sequence;
   - the received level;
   - the band's response;
   - clipping at its own ADC.
4. **How it chooses.** It takes the SNDR maximum, refined by a parabola through the top three, and
   backs off 1 dB.
5. **The report.** Its acceptance reports the offset (0.5 dB steps), the SNDR there and the curve's
   class. The acceptance carries the called station's own ladder, which the caller reports in its
   first header. Both directions are calibrated with no extra transmission.

**Each rung gets its own level:** the calibrated level less the rung's peak difference from the
ladder's sequence. On the scratch model QPSK sits 1.1 dB under 16-QAM at the 10⁻⁴ level, and
64-QAM 0.3 dB over it.

The curve classes and their responses:

| Class | Meaning | Response |
|---|---|---|
| Peaked | The normal case | Best level less 1 dB |
| Rising throughout | Weak RF or a level far too low | The top step; the next call starts higher, up to the ceiling |
| Falling throughout | Overdriven everywhere | The bottom step; the next call starts lower |
| Flat | An ALC ahead of the emphasis, a compressor, an AGC, a fixed-level path | The knee where SNDR stops rising; if flat throughout, keep the level and tell the operator |
| Irregular | Interference or fading during the ladder | Keep the stored value; measure again later |

### 2.3 Stage 2: a two-level probe on the data

1. **Alternating levels.** Each transmission's data frames alternate between d − 1.5 dB and
   d + 1.5 dB, by the frame's parity.
2. **What the receiver measures.** It re-encodes each decoded frame and measures its SNDR. Only
   decoded frames count (ADR-0069 §2.5).
3. **The report.** The next answer carries Δ, the difference between the two groups, in 0.25 dB
   steps.
4. **The control law.** d ← d + μ·Δ, with:
   - a 0.5 dB deadband;
   - at most 1 dB down or 0.5 dB up per exchange;
   - inside ±6 dB of the calibrated level;
   - never above the ceiling.

**The freeze rules**, which keep the level loop apart from the rate controller (ADR-0099). The
level holds still:
- while the rate controller tries a rung above;
- for one exchange after a rung change;
- when the answer reports interference.

HARQ copies go at their first copy's level.

### 2.4 What each station guards on its own side

- Receive clipping, from ADR-0105's clipping reading.
- A too-low receive level.
- The operating system's processing (enhancements, AGC, noise suppression): calibration refuses
  while it is on, and names the setting.

### 2.5 The ceiling

The calibrated level's maximum is the highest level at which no sound card clips. On a flat port,
the operator's one-time deviation check also bounds it: no algorithm can measure absolute
deviation (proposal §E.1).

## 3. What it adds to the wire

All of it goes in FM's own link protocol (FM-7):
- the ladder segments in the call and the acceptance;
- the report in the acceptance and the first header (offset, SNDR, class);
- Δ in every answer (6 bits);
- the frame parity rule.

Nothing changes on HF.

## 4. What this removes

- The roadmap's P10-3, P10-4 and P10-5 rows, replaced here. The Test session keeps a one-time
  pre-emphasis check per radio.
- The operator's level-setting procedure, as a requirement. A guide still says how to start near
  the right level, but a level 10 dB off must cost speed, not the contact.

## 5. The runs and the criteria (registered now)

Step 10.3 runs on the FM channel model after Gate 2. Each trial is a call and a session.

**The grid:**
- starting level error from −15 to +15 dB in 3 dB steps, at both ends independently;
- CNR 12, 18, 24 and 30 dB;
- the three profiles and every recorded radio;
- static paths and 13.5 Hz Rician fading;
- 50 seeds a cell.

The results go to `docs/research/aether-fm/04-calibration.md` with their curves under
`bench/baselines/`.

**E.7, the proposal's criteria:**

- **C1.** From any starting error in −15 to +15 dB, the call connects in 99 % of trials at
  CNR 12 dB or more.
- **C2.** After the call and the acceptance, the level error is within 2 dB in 95 % of trials.
  After ten exchanges it is within 1.5 dB.
- **C3.** A session started 10 dB off takes at most 1.15 times as long as one started right.
- **C4.** In a steady channel the level's standard deviation over 50 exchanges is under 0.5 dB.
- **C5.** The level never exceeds its ceiling.
- **C6.** An AGC, a compressor or receive clipping is detected and named in 90 % of trials or
  more.

**Added here:**

- **C7 (the loops together, risk R8).** With both loops on, a session's time is within 5 % of the
  same session with the level held at its true best. No oscillation: the rung and the level each
  change direction at most twice in 50 exchanges of a steady channel.
- **C8 (simplicity).** Each part must fail a criterion when it is taken out, or it goes. The parts
  are Stage 1, Stage 2, the curve classes, the per-rung offsets and each freeze rule. The ablation
  runs are part of step 10.3's results.

A narrow miss goes to the author with its trace, case by case. It does not pass by rule.

## 6. Consequences

- FM-6 (learned profiles) keeps what Stage 1 finds: the local radio's knee, and each peer's
  offset from it.
- FM-7 carries §3's fields, and its gap terms include the ladder's 0.3–0.7 s in the call.
- The panel names the side at fault. For example, "KK4ODA-1 hears you compressed, about 4 dB", or
  "your receive audio is clipping", rather than one level bar.
