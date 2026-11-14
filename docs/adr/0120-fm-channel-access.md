# ADR-0120: FM channel access is the squelch, a slot time and a reservation

**Status:** proposed, 2026-11-14. This is the Aether FM proposal's FM-8
(`docs/research/aether-fm/00-proposal.md` §F.4, §L). It is built in step 10.4 and accepted on
§5's harness criteria. Nothing is built for it yet.

## 1. Context

Inside a session, granted slots decide who keys (ADR-0069). Outside one, a station keys on its own
initiative: a call, a probe, a beacon, a datagram. It must wait for a clear channel.

On HF that was the busy detector. It has been patched about eight times, is about 1 600 lines with
26 constants, and is under the author's no-more-patches rule (2026-10-22). FM must not inherit it.
An FM radio already answers the question in hardware: its squelch.

## 2. Decision

### 2.1 Carrier sense is the squelch

In order of preference:
1. the radio's squelch state over CAT, where the radio reports it;
2. the squelch-status line of the data jack (pin 6 on many radios' 6-pin mini-DIN), read through
   a serial input or a CM108 GPIO;
3. the audio, as a fallback: silence behind a closed squelch, or the quieting of the
   discriminator's noise above the signal's band on a flat port.

The panel names which source is in use. Setup offers the first one the radio has.

### 2.2 Activity, not only carrier

A carrier with no audio is busy too: someone is about to speak, or a repeater is holding its hang
time. On the repeater profile (ADR-0121), carrier alone is not busy; audio activity is.

Activity is the received audio's level in the profile's band, against the level behind a closed
squelch. That is one comparison with one constant, judged on recordings before it is used.

### 2.3 When a station may start

- **p-persistence and a slot time** for calls, probes, beacons and datagrams, as KISS programs
  expect (ADR-0019). Defaults: p = 0.25 and a slot of 100 ms. A KISS client's own values win for
  its frames.
- **The reservation** (`station/reserve.rs`, ADR-0069 Step 3): a session heard holds this
  station's own calls.

### 2.4 Hidden stations

In RadioPost the client calls, so it is the master and the gateway the slave. A second client may
hear only the gateway. So each slave answer states when the master's next transmission may start
(the slot it grants back). A third station that hears the answer reserves until then, and until
the session's end if it hears a DISC.

FM capture often lets the stronger of two colliding calls through. No rule depends on it.

## 3. What this removes, on FM

- HF's busy detector: its floor, its gain reference, its shape tests and its constants. The FM air
  reports `ChannelSense` (ADR-0113 §2.2) from the squelch and the activity reading alone.

## 4. What it adds

- The squelch sources of §2.1. CAT's squelch reading is per radio family, like `Ptt::s_meter`.
- The activity reading.
- The answer's next-start field (ADR-0119 §2.3).
- The reservation's use of it.

## 5. Criteria (step 10.4, on the harness)

- **A1.** On an idle channel with the squelch closed, a call waits at most one slot time beyond its
  persistence draw.
- **A2.** With a second session on the channel, no call or probe of this station overlaps it.
- **A3.** In a hidden-station scenario (a second client that hears only the gateway), the second
  client's calls overlap the session's master transmissions in at most 1 % of the slots it could
  have hit.
- **A4.** Voice on the channel (a recorded voice QSO on the harness) holds every call while it
  lasts.
