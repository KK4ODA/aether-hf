# ADR-0119: The FM link protocol is granted slots with FM's numbers

**Status:** proposed, 2026-11-14. This is the Aether FM proposal's FM-7
(`docs/research/aether-fm/00-proposal.md` §F, §L). It is accepted when §5's checks pass, in step
10.4 and before the FM air goes on the air. Nothing is built for it yet.

## 1. Context

HF's link protocol is granted slots (ADR-0069): the caller is the master, every master
transmission grants the other one slot, and the slave keys only in a slot granted by a frame it
decoded in time. The rate controller is each sender's own (ADR-0099). Both were judged on the
harness and on the air. Most of what HF added around them exists because HF has two frame
families and moves between bandwidths:
- the family an answer goes in;
- floor control frames;
- bandwidth moves.

FM has one family (ADR-0116 §2) and one profile a session.

## 2. Decision

### 2.1 The same engine, FM's numbers

FM runs `GrantedEngine` (model and port) unchanged in its rules. What differs comes in through
`PhyTiming` (ADR-0113 §1):
- one family, so `floor_*` empty;
- FM's frame lengths and decode budget;
- the count of frames to follow read from a header field;
- no echo limits.

A rule the engine has only because HF has two families is switched off when the PHY gives no
floor, never forked into an FM copy.

### 2.2 The gap

ADR-0069 §2.2's gap is the decode budget, plus keying latency, plus the guard, plus the other
station's declared turnaround. FM's terms:
- **Decode budget:** measured on the port (step 10.4), as HF's was (0.30 s).
- **Keying latency:** learned per station (ADR-0071), with the radio's transmitter attack.
- **The other's turnaround:** declared in the call and the acceptance (`turnaround_ms`), and
  learned from repeats (ADR-0054). On FM it is dominated by squelch opening, a tone-squelch
  decoder, a VOX interface's hang, or a repeater's key-up.

### 2.3 Framing

- **A transmission** is a preamble, a header control frame, and up to six data frames. Each data
  frame states exactly how many follow (ADR-0068 §2.1).
- **A data frame** is one LDPC codeword of one to four blocks (ADR-0116 §4.4).
- **The connect body** names the air (FM), the profiles the station supports, its declared
  turnaround, and Stage 1's report (ADR-0117 §2.2).
- **Every transmission's header carries the sender's callsign** in ASCII, for identification
  (ADR-0123) and for a third station's reservation (ADR-0120).

### 2.4 Its own protocol number

FM's link protocol numbers start at 1 and are separate from HF's 12. An HF and an FM station can
never meet on the air; the number keeps FM's own versions apart, and the shell's update notice
follows it as it follows `LINK_PROTOCOL` (ADR-0041 §4).

### 2.5 Windows and caps

- **Open windows** bound a host's think time: about 2 s while the slave's host has unanswered
  input, then the 10 s keepalive (proposal §C.2).
- **The burst cap** is the smaller of the key watchdog's budget and the repeater profile's limit
  (ADR-0121).

## 3. What this removes, on FM

- The family rules (ADR-0016, 0034, 0059, 0066), floor control frames, and the family in the gap.
- Bandwidth moves (ADR-0026, 0035, 0058, 0101, 0102): a session's profile is agreed in the call.
- Offset following (ADR-0079) and echo limits (ADR-0087).

## 4. What it adds

- `PhyTiming`'s count field and the switch-off of §2.1, model first, with granted vectors for an
  FM air.
- The callsign field, the connect body's FM fields, and FM's protocol number.

## 5. Checks before it is accepted

- **L1 (the formal model).** The TLA+ models (`research/link_arch/tla/`, frozen at `b0d422c`)
  and the review's simulator are re-run with FM's measured latencies. Assumptions S1–S7 of
  `docs/research/link-architecture/04-formal-spec.md` are checked against them. The case to model
  is a VOX interface's hang longer than the declared turnaround (S2).
- **L2 (no collision).** On the harness, with FM radios (ADR-0115), no radiated overlap in any
  scenario (ADR-0069's C1a).
- **L3 (RadioPost).** RadioPost's conformance tester passes A-01 to A-04 over two FM daemons, and
  the proposal's J.2 table holds.
- **L4 (nothing HF changes).** HF's granted vectors, link vectors and harness baseline are
  unchanged by §2.1's switch.
