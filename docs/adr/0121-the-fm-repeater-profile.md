# ADR-0121: The repeater profile is off by default and learns each repeater

**Status:** proposed, 2026-11-14. This is the Aether FM proposal's FM-9
(`docs/research/aether-fm/00-proposal.md` §G, §L), with the author's decision 6 (repeaters off by
default). It needs recordings of real repeaters (risk R9, step 10.5) before it is accepted.
Nothing is built for it yet.

## 1. Context

A repeater changes the MAC, not the PHY. It adds:
- a key-up before the first audio;
- a delay line;
- a hang time, during which its carrier stays up after the input drops;
- a courtesy tone;
- a time-out timer;
- a second limiter;
- voice users who cannot break into a data transmission.

Published figures:
- controller delays of 12–200 ms;
- hang times of 5–10 s;
- courtesy tones about 1 s after the input drops;
- time-out timers of about 3 minutes.

No published measurement of a repeater's end-to-end key-up was found (proposal §G.1).

Repeater owners' rules come first, so the profile is off until the operator turns it on for a
channel.

## 2. Decision

### 2.1 The operator turns it on for a channel

The profile is a property of a dial memory (the dial list, `memories.rs` and `frequencies.json`), off by
default. Turned on, the guide's line beside it says: *ask the repeater's owner first*.

### 2.2 What it learns

From what the station hears after each transmission through the repeater:
- **the lead:** from the key to its own audio coming back;
- **the hang time:** how long the carrier holds after the input drops;
- **the courtesy tone:** when it sounds, and its frequencies;
- **the delay line.**

The first call uses a long lead (1 s) and learns. The values are kept per repeater in the peer
profiles' file (ADR-0118).

### 2.3 What it does with them

- **Turnarounds inside the hang time.** When the turnaround fits before the courtesy tone, the
  reply needs no new key-up and no beep sounds.
- **A burst cap of 10 s**, well under the time-out timer.
- **A break window every minute of session:** the master grants a slot no one uses, so a voice
  user can be heard.
- **Voice yields.** Audio activity in a gap that is not a frame (ADR-0120 §2.2) pauses the session
  until the channel has been quiet 5 s.
- **Activity, not carrier,** is busy (ADR-0120 §2.2).

## 3. What this removes

Nothing; it is new. It adds a dial-memory flag, the learned values, and the rules of §2.3.

## 4. Criteria

- **R1 (step 10.5, recordings).** 5–10 local repeaters are recorded with the owners' leave, by
  the bench of `tools/fm_bench.py` extended to time key-up, hang and tone. The learned values are
  within 50 ms (lead, delay) and 0.5 s (hang) of the recording's.
- **R2 (harness).** The harness's repeater (ADR-0115 §2.3) is built from those recordings. A
  session keeps every turnaround inside the hang time in 90 % of exchanges, and never runs a
  transmission past the burst cap.
- **R3 (on the air).** Through a real repeater with its owner's leave, a session completes, and a
  voice user keying in a break window stops it within one exchange.
