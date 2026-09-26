# Field testing

How a session on the air becomes a number the project can use. Phase 6 of the roadmap ends
when there are twenty logged sessions across three channel classes and the simulator's
throughput prediction is within 20 % of what the air delivered; every session that reaches
`field/sessions/` is also a test the receiver has to keep passing.

---

## 1. Before the first session

**Bench first.** Two daemons on one machine over the simulated channel, at the SNR you expect
on the air, with the same host software you will use in the field. If it does not work on
the bench it will not work on the air, and the bench is where the log is easy to read.

```toml
# station a                          # station b
[sim]                                [sim]
listen = "127.0.0.1:8600"            connect = "127.0.0.1:8600"
snr_db = 10.0                        snr_db = 10.0
```

Both daemons key nothing with `[sim]` set. `core/aetherd/tests/two_daemons.rs` is the same
thing as a test; `tools/compare_air.py` reads the recording it makes like any other.

**Then the audio cable.** Two sound cards (or two machines) joined by a cable, no radio:
this is the first place the real audio path — the sound cards' buffers, the daemon's
playback backlog, the receiver's own latency — meets the protocol's timers. The engine
knows the daemon's own latency (`PhyTiming.tx_latency_s`); the cable proves it.

**Set the level.** Setup step 3 for the receive level, and the Session tab's **Keying and
drive** → *Set drive* for the transmit level against the rig's ALC (the guide is in that
card), on both stations. An overdriven card is the most common reason a mode "does not
work".

**The rig's AGC.** FAST or AUTO, or OFF with the RF gain set so the band noise sits well
above the sound card's own floor. The busy detector learns the noise floor from the
quiet, steady moments of the last five seconds; a receiver's AGC cuts its gain the
instant something strong appears anywhere in its passband and lets it back over the
next half second, and the detector is built to see through that ramp (an FTDX10 on
AUTO recovers at 40–55 dB/s). A SLOW setting recovers gently enough to look like a
genuine drop in the noise, and the floor on the Status tab will follow the AGC down
for a few seconds after every strong adjacent signal. `tools/floor_trace.py` replays a
recording through the detector and lists every such dip, with its depth and length.

## 2. Recording

Turn it on once and forget it:

```toml
[record]
auto = true
```

Every session then records itself from connect to disconnect — a 48 kHz WAV of what the
radio delivered and a JSON sidecar of what the modem made of it — under `recordings/` beside
the configuration. Or press **Record** on the Session tab for a listening session with no
peer. Either way, **write the notes**: the band, the frequency, the other station, the
distance, the time of day, what the S-meter said. `record.notes` on the API, the notes field
beside the Record button, or `record.start {"notes": ...}`. The modem can measure SNR; it
cannot know it was 40 m at dusk over 900 km.

A minute of audio is 5.8 MB. Sessions of a few minutes are the useful size.

### What the modem sent, not only what it heard

A recording is what the *radio delivered*; the transmitter's side of the story is not in
it. When a burst looks wrong on the air — unsteady on a scope, "choppy" at the other end —
the question is where it went wrong: in the modem, at the sound card, in the transmitter,
or only in the receiver's AGC. For that:

```toml
[record]
tx_audio = true
```

keeps every transmission's exact audio as it was handed to the sound card — the transmit
level applied, the keying lead and tail included — as a 32-bit float WAV under `tx/` in the
recordings folder, with a sidecar giving its level, crest factor, clipped samples, the
onset envelope at ten milliseconds and any holes, and one `tx:` line in the log. It writes a
file per burst, so it is for a test session, not for a gateway. Then

```
python tools/tx_envelope.py recordings/tx/*.wav
python tools/tx_envelope.py recordings/tx/*.wav other-end.wav --png bursts.png
```

prints the same numbers for the capture and for any recording of the same bursts — the
other station's, the rig's monitor, another modem's — burst by burst: where the level
settled, and every hole. A rendered burst has no holes and is at level within twenty
milliseconds; whatever appears downstream was put there downstream.

Two more lines in the log are for the same question. `audio: the sound card ran dry for
N ms inside a transmission` means the modem did not hand the card its next samples in time
and there was a hole on the air; it should never appear (a whole burst is queued the moment
it is rendered, ADR-0010), and if it does the `loop:` line beside it says which part of
the modem's pass took the time. Both counts are in the diagnostic bundle.

## 3. What a session should be

| Step | Who | What to send |
|---|---|---|
| connect | the caller | — |
| transfer | the caller, then the other way if there is time | at least 4 kB of *incompressible* data — a small photo, a `.zip` — so the link is measured and not the compressor; a text message is fine as a first check but says little about throughput |
| disconnect | the caller | — |

Both stations record. The receiving side's recording is the one the tools want (the
frames it heard are the channel); keep both anyway.

## 3a. The Test session

The one-button version of §3, for volunteers. Put the other station's callsign in
**Call** on the Session tab and press **Test session**. The modem then runs, and records,
a fixed sequence: a probe (both directions' SNR), a call, a 2 kB message, the **mode
ladder** — a short burst at every mode from the floor up, each one's acknowledgement kept
as a rung, until three rungs in a row fail — then a 16 kB file and an orderly
disconnect. About five minutes of transmitting, ten at most: the message and the file
are sized to what the path can do (how the other station hears you, from the probe's
answer, sizes the message; the message's measured rate sizes the file to about two
minutes' worth of what is left), the run keeps to a time budget, and **Stop test** — the
same button while it runs — ends it at once, keeping what was learned. The ladder runs
before the file: with the file first, three tests with ND1J on a slow path spent the whole
budget on the transfers and never reached a rung (2026-09-25).

While it runs, the Session tab shows the step (*step 4 of 6, climbing the mode ladder*),
the time elapsed and the most the budget leaves — never a countdown, since how long a
step takes is the path's to say — a bar for the bytes acknowledged or the rungs tried, the
rung under test (*rung 3 (tone50-75), rung 4 of 20, 4 frames*), the fastest rung that has
passed, the failures in a row that end the ladder (three), the last rung's result, and
the rung the link is using with how the other station hears you. The log says what each
step found. The other station needs to do nothing but listen: an answer-only station
(`[radio] answer_only`) is a fine partner, and the author's runs that way at agreed times.

What it leaves is a recording named `…_<you>_<them>_test` whose sidecar carries, under
`session.test`, the probe's numbers, both transfers' goodput, and the ladder — the
frame error rate per mode at the SNR the other station measured, on a real path,
which no bench can give — with your grid, rig, power and antenna from **Setup › step 1**
beside it, and the path length if you gave the other station's grid (`test.start` on
the API takes `remote_grid`). Those four Setup fields are optional and only for this:
fill them in once.

Every session, tested or not, is also a line in the **Sessions** list on the Stations
tab when it ends: when and how long, who called, what crossed each way, the fastest rung
each way, how the other station heard you, how it ended, and the recording's name.
**Sessions** on a station's row shows only that station's.

**Contribute it**: *Contribute the last test session*, at the foot of the Session tab's
**Recording and last session** box, opens a pre-filled GitHub issue (in a plain browser it
copies the link instead); attach the sidecar — the `.json` beside the `.wav` in the
recordings folder, which *Open folder* in the same box shows you — and send. The audio is yours to attach or not. `tools/field_ingest.py` folds what
arrives into `field/LOG.md` and `field/paths.csv`, and `tools/bench_link.py --replay
<sidecar>` runs the model's engines against what the path did.

## 4. Channel class

The simulator's baselines are for four classes. Write down which one the air was, from what
you saw, not from what you hoped:

| Class | What it means | How to tell |
|---|---|---|
| `awgn` | no fading worth the name | audio cable; ground wave at VHF-like stability; a very short skip with a steady S-meter |
| `good` | ITU-R F.1487 Good: 0.5 ms spread, 0.1 Hz Doppler | slow, shallow fading; an S-meter that drifts |
| `moderate` | Moderate: 1 ms, 0.5 Hz | fading you can hear as a slow flutter; the usual daytime skip |
| `poor` | Poor: 2 ms, 1 Hz | fast, deep fading; NVIS at the wrong hour; polar or auroral paths |

## 5. After

```bash
aetherd --replay recordings/<name>.wav            # does it still decode what it decoded?
python tools/compare_air.py recordings/<name>.json --channel moderate
```

The first prints every frame and compares with the sidecar; the second puts the measured
frame error rate per mode and the session's goodput beside the simulator's prediction at
the same SNR, and says whether they are within 20 %.

**Keep the sessions that teach something**: a path, a band, a condition the simulator does
not reproduce; one at the edge of a mode; one that failed and should not have. Copy the
WAV and the sidecar into `field/sessions/`, commit them together, and from then on
`cargo test` replays them. `field/README.md` says what belongs there.

## 6. The log

Twenty sessions, three classes. A row per session in `field/LOG.md`: date, band, distance,
class, both callsigns, the recording's name, the mean SNR, the goodput measured and
predicted, and a sentence. The comparison tool prints the numbers; the sentence is yours.
