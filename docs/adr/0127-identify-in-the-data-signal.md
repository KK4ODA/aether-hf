# ADR-0127: A station may identify in its data signal instead of Morse

**Status:** accepted for building, 2026-11-16. The author asked for it after the exchange-timing
study of the same day (§1): identification is to be a Setup choice, Morse or the data signal,
never both. The same day the author, as licensee, confirmed §3's regulatory reading and answered
§9's five questions. Nothing is built yet; §6's criteria judge it.

## 1. Context

Every session ends with both stations' Morse identifiers, one after the other (ADR-0075). In
model runs of a complete exchange on beta.160's engines (a call, a 2 000-byte message, a 400-byte
reply and a disconnect, on six paths, the link bench's fading pipe), the two identifiers took
9.0 s in every exchange: W4ODA 3.3 s and KK4XYZ 4.7 s of Morse at 20 wpm, each with half a second
of silence before it. That is 23 % of a strong 2 300 Hz exchange (38.8 s) and 11 % of a good
500 Hz one (82.6 s).

20 wpm is the limit §97.119(b)(1) sets for a Morse identifier keyed by an automatic device. Aether
holds every US station to it (`us-fcc-part97.json`, `cw_id.max_wpm`; ADR-0075 §2.3). Sending
Morse faster would need a different reading of that rule. This ADR does not take one.

§97.119(b)(3) allows another way: "a RTTY emission using a specified digital code when all or
part of the communications are transmitted by a RTTY or data emission". ADR-0123 takes that way on
FM: the callsign in ASCII inside the data emission. On HF the question was left open:
- ADR-0075 §3 found that Aether's call frames carry callsigns in its own 6-bit packing, which is
  not a specified code.
- `docs/user/fcc-regulatory-controls.md` (*What identifies a long session*) says so.

## 2. Decision

### 2.1 One Setup choice: how the station identifies

`[radio] identify_by = "morse" | "data"`, live, in profiles, shown in Setup step 4 as **Identify
by: Morse / Data signal**. One value, so the two can never both be on. The existing settings keep
their jobs:

- **`cw_id`, and its forcing for a US callsign** (`cwid::required_for`), still decide *whether*
  the station identifies at the end. A US callsign always does, by the chosen method.
- **`cw_id_wpm`** applies only to Morse. Setup shows the speed only when Morse is chosen.
- **A host program's `CWID ON`** (ADR-0075 §2.7) still means "identify". The station's choice
  says how. The program cannot turn Morse back on: `CWID` is not in VARA's published command
  list, and the programs that send it want the station to identify, not a particular emission.

The default is `morse`: today's behaviour, so nothing changes for an operator who does nothing
(the author's answer, §9).

### 2.2 The identification frame

The data identifier is one control frame: the same 7 bytes and the same two families as every
other control frame, 0.43 s in OFDM and 3.2 s on the tone floor, on either air.
- **How it is recognised.** It is a DISC with session field 0. No session's control frame has
  session field 0, because session numbers are drawn with nonzero low seven bits
  (`_new_session_number`). The call and the acceptance use session 0 with the HEADER and ANSWER
  kinds (ADR-0126 §2.1); DISC is the free kind.
- **What it holds.** The 47 bits left hold six characters in 7-bit ASCII (ITU-T T.50, the
  specified code of §97.309(a)(3)), a continuation bit and four reserved bits.
- **What it says.** The callsign as the Morse identifier sends it (`cwid::sent_as`): no SSID,
  `-T` or `-R`, and any `/` part kept.
- **Longer callsigns.** Up to six characters go in one frame. That covers every US callsign
  (at most 2×3) and most others. Seven to twelve characters go as two frames back to back, the
  first with the continuation bit set, for example `W4/G4ABC` or `G4ABC/W4`.
- **Its family** is the family of the last frame the station sent in the session: its DISC_ACK, or
  its own last DISC. A station that sends its identifier alone (`listen_first`, ADR-0075 §2.2)
  uses the floor when its last frame was on the floor.

The frame's layout goes in `docs/spec/air-interface.md`, which is public. A listener with only that
document can read the callsign (§97.309(a)(4), and §6's criterion C3).

### 2.3 Where it goes: unchanged

Everything in ADR-0075 §2 stands. The data identifier takes the Morse identifier's place, its
slot, its holds and its waits, and only its length changes. The declared identifier length in the
call and the acceptance (`AcceptFrame.identifier_s`, 0.5 s steps) carries the frame's length:
- one OFDM control frame: 0.5 s;
- one floor control frame: 3.5 s;
- two frames: the sum, rounded up.

The master's wait after the DISC_ACK, the slave's `_closing_hold`, a third station's reserve
(`station/reserve.rs`) and the host program's `DISCONNECTED` after `identified` all work from that
length, as they do for Morse.

### 2.4 Receiving it

A station that decodes an identification frame records the callsign as heard:
- the `identified <callsign>` log line;
- a `frame` event of kind `ident`;
- the recording's frame record;
- the heard list.

It changes no session state. Older builds ignore it. In the model,
`_on_control` drops a control frame whose session field is not the session's, and `_answer_left`
drops one whose session field is not the left session's. Both are nonzero, so a session-0 DISC
is dropped. The port must be checked to do the same (§6, C4) before any field build carries the
frame. If it does, no link protocol bump is needed.

### 2.5 The regulatory gate

The gate judges the identification frame as the data emission it is, at the dial it goes out on,
like any other frame (ADR-0018). It also checks, before keying, that the callsign the frame
carries is one the station is licensed to send (the session's callsign, `sent_as`). It is not
judged as CW, so the 20 wpm cap does not apply.

## 3. The reading, confirmed by the author (2026-11-16)

**The reading.** A callsign in ASCII (§97.309(a)(3)), sent in a publicly documented data format
(§97.309(a)(4)), at the end of a communication that was itself data, identifies the station "by a
RTTY emission using a specified digital code when all or part of the communications are
transmitted by a RTTY or data emission" (§97.119(b)(3)).

**Why it is a reading.** It is the same as ADR-0123 §3. Part 97 defines RTTY as telegraphy for
automatic reception (third symbol B). Aether HF's emission is data (third symbol D). So the
identification is, strictly, in a data emission, not a separate RTTY one.

**Why it is the usual one.** Packet radio identifies with the callsign in the AX.25 address field,
in the same emission as the data. PACTOR and ARDOP put the callsigns in their own frames and treat
that as identification (`fcc-regulatory-controls.md`). ADR-0123 took this reading on FM, and the
author confirmed it on 2026-11-14.

**What differs from those.** Aether's callsign packing in its call frames is not a specified
code, so this ADR adds a frame in ASCII instead of leaning on the packing. Only that frame is
proposed as identification.

**Confirmed** by the author, as licensee, on 2026-11-16, as he confirmed ADR-0123 §3 for FM.

## 4. What it saves, and what it does not

From the model runs of §1, the same seed with each identifier. The runs are identical up to the
identifiers.

| Path | With Morse | Data signal | Saved |
|---|---|---|---|
| 2 300 Hz Good +15 dB | 38.8 s | 30.5 s | 8.3 s |
| 2 300 Hz Moderate +5 dB | 65.2 s | 56.9 s | 8.3 s |
| 2 300 Hz Moderate −10 dB (closing on the floor) | 273.0 s | 270.4 s | 2.6 s |
| 500 Hz Good +10 dB | 82.6 s | 74.3 s | 8.3 s |
| 500 Hz Poor +2 dB (closing on the floor) | 198.8 s | 194.2 s | 4.6 s |
| 500 Hz Moderate −8 dB (closing on the floor) | 360.5 s | 358.0 s | 2.5 s |

The runs modelled the identifier as one control frame plus 50 ms. They are estimates on the
fading pipe; the harness measures it (§6, C5).

**What it does not do.**
- **Identify during a long session.** ADR-0075 §3 found that nothing in Aether identifies a
  session between its call and its end. That stays so: by the author's answer (§9), Aether adds no
  identifier inside a session, with either method. Identifying during a long session stays with
  the host programs and the operator, as `fcc-regulatory-controls.md` says.
- **Change Morse.** Morse is untouched, and it stays the default.

## 5. What it removes and adds

- **Removes:** nothing. Morse stays as an option. For a station that chooses the data signal, the
  closing is 2.5–8.3 s shorter in the runs above.
- **Adds:**
  - one control-frame layout, in the model, the port and the spec;
  - the engine's identifier as a frame instead of a duration;
  - the configuration key (schema 15, a no-op migration step, a fixture, the shell's
    `SCHEMA_HISTORY` line);
  - Setup step 4's choice;
  - the gate's callsign check;
  - the receive side of §2.4;
  - the manual's and `fcc-regulatory-controls.md`'s words.

## 6. Criteria, registered before anything is built

- **C1, the model.** For both choices, every closing of ADR-0075 §4's list ends with each
  identifying station's identifier last, with no radiated overlap. That covers both roles, both
  airs, aborts, a dead path, being called again, and a station that sends none.
- **C2, the vectors.** With `identify_by = "morse"`, the granted and link vectors regenerate byte
  for byte. With `"data"`, new vector sessions hold the port to the model, frame by frame.
- **C3, a listener with the spec.** A test decodes the identification frame's callsign with the
  model's control-frame decoder and the layout in `air-interface.md` alone, for one-frame and
  two-frame callsigns.
- **C4, older builds.** The port, like the model, ignores a session-0 DISC in a session, idle,
  and after leaving one. A mixed pair (`tools/mixed_pair.py`) with beta.160 passes every
  `mixed` scenario both ways round, with no radiated overlap and nothing done on the frame.
- **C5, the harness.** The identifier scenarios (`40m-closing-identifiers-2300`, its `-ve3-`
  twin, `80m-varac-chat-and-ping-500`) and the quick set pass, three draws each, with
  `identify_by = "data"`. The identifier invariants (`identifier_faults`) judge the frame as they
  judge Morse. The closing's median time falls, paired by draw against `morse`.
- **C6, the gate.** No identification frame keys with a callsign the station is not configured
  for, and each is judged at the dial it goes out on (a station test).

## 7. Configuration

`[radio] identify_by` arrives with configuration schema 15:
- a migration step that changes nothing (`identify_by`), so an older build goes back to
  `station.toml.bak-v14` as CLAUDE.md's schema rule requires;
- a fixture of beta.160's file;
- the shell's `SCHEMA_HISTORY` line;
- the example file's `schema_version`.

The key is portable, in profiles, with the settings registry's options `morse` and `data`. Like
every configuration key, it ships with a release.

## 8. The manual

Setup step 4's words, its screenshot and a change-log row. The Morse identifier's section in
`fcc-regulatory-controls.md` gains *Identifying in the data signal*: what is sent, when, the
reading of §3, and the operator's choice. *What identifies a long session* changes only to say
that the data identifier is also at the end only.

## 9. The author's answers (2026-11-16)

1. **The reading of §3:** confirmed, as for FM (ADR-0123 §3).
2. **The default:** `morse`. No change for an operator who does nothing; the data signal is chosen
   in Setup step 4.
3. **The SSID:** sent without it, as the Morse identifier is (`cwid::sent_as`). Every US callsign
   fits one frame.
4. **A program's `CWID ON`:** means "identify", by the station's choice (§2.1). RMS Trimode says
   `CWID ON` at every start, so forcing Morse would have undone the setting on every gateway.
5. **Identifying during a long session:** not at all. Aether adds no in-session identifier with
   either method (§4).
