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

**The rig's AGC.** Any setting, or OFF with the RF gain set so the band noise sits well
above the sound card's own floor. A receiver's AGC cuts its gain the instant something
strong appears anywhere in its passband, holds it down while the signal lasts, and lets
it back when it goes. The busy detector learns the noise floor against the noise across
the radio's whole audio passband, which the AGC turns down and up together, so the floor
follows the gain both ways (ADR-0078; before it, the floor learned the turned-down noise
and the channel read busy for about a minute after every strong signal). The Diagnostics
tab's busy line shows *radio gain* while the AGC holds it down. At 2300 Hz the channel
fills the passband, so the floor waits until the gain is within 3 dB of normal: a slow AGC
reads busy for a few seconds after a deep cut. The gain cannot be read, and the floor
behaves as before, with a radio filter narrower than about 1 kHz around a 500 Hz channel or
a band of strong signals filling the rest of the passband (a busy FT8 segment). In USB mode
an FTDX10's AUTO is SLOW (3 s); DATA-U (MID, 0.5 s) is better. A change that makes all the noise
louder at once — the RF gain up, the attenuator off, a preamp on, a noisier band — reads busy for about a
minute while the floor learns the new level; make it before you start listening. `tools/floor_trace.py`
replays a recording through the detector and lists every AGC event in it: how deep and how
long, how fast the noise came back, whether its cause was in the channel, and how long the
channel read busy after it.

## 2. Recording

Turn it on once and forget it:

```toml
[record]
auto = true
```

Every session then records itself from connect to disconnect, and on until the closing Morse
identifiers have gone (this station's, and the other station's when it identifies after this
one; at most 30 s past the end) — a 48 kHz WAV of what the radio delivered and a JSON sidecar
of what the modem made of it — under `recordings/` beside the configuration. Or press **Start
recording** on the Session tab (the *Record the radio's audio* box) for a listening session with
no peer. Either way, **write the notes**: the band, the frequency, the other station, the
distance, the time of day, what the S-meter said. `record.notes` on the API, the notes field
beside **Start recording**, or `record.start {"notes": ...}`. The modem can measure SNR; it
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

Two lines are about the timing of a session's answers (ADR-0071). `keying: the radio takes N ms
to key and a transmission M ms to render` is what the station has learned of its own radio and
computer; it starts its transmissions that much sooner, and says so again only when the times
change. The diagnostic bundle's `keying` has the recent waits. `audio: the sound card's capture
skipped N ms of audio, by its own timestamps` means the card's capture timestamps jumped past
audio that never arrived: the receiver heard the audio either side run together, and a frame
across it may be lost. The bundle's `audio` counts these, and the recording marks each one
(`capture_skip`). A recording made with both stations' audio, read with `tools/two_sides.py`,
shows such a loss as a step in the offset between the two recordings.

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
a fixed sequence: a probe (both directions' SNR), a call, a message (512 B to 2 kB, sized
to how well the probe says you are heard, and sent in the session's first burst when the probe
was answered), a file sized to about two minutes at the message's rate (at least 256 B), then the
**mode ladder** — a short measurement at every mode the link uses, from a rung under the
fastest the message and file were acknowledged at (the log names both), the other station's
report of each kept as a rung, until three rungs in a row fail — and an orderly disconnect. About five minutes of transmitting, ten at most: the message and the file are sized
to what the path can do, the run keeps to a time budget, and **Stop test** — the same button
while it runs — ends it at once, keeping what was learned.

**The transfers go first, the ladder last** (ADR-0074). The message and file are measured the
way an operator's transfer would go, before the ladder tries anything else, so the rates the
Test reports are the link's own. The ladder tries rungs the link would not choose, and each rung
is a **measurement** (ADR-0077): a few frames of test filler at the rung, the full size of its
frames, sent once in a transmission of their own. The other station's acknowledgement says
which arrived, and none is sent again or delivered, so a rung costs one exchange however badly
it does. The ladder skips the rungs the link never asks
for — 4 and 8 at 500 Hz, 6 and 13 at 2300 Hz, each beaten by another on both speed and SNR
(ADR-0073) — since measuring them tells nothing the link can use (ADR-0076). A rung whose
answer was lost is kept with no SNR (*at ? dB*): the report comes after a poll, and its reading
is of the poll, not of the rung's frames (report 13 §6). After the last rung's report the Test
disconnects. Through 0.2.0-beta.112 the ladder pinned the session's own test bytes at each rung,
so a frame that failed was data still to deliver: on WC4Y's Test of 2026-10-17 that took 93 s of
435, one 5.4 s tone frame for each 10 bytes on a weak 500 Hz path (report 13 §10), and
0.2.0-beta.113 cannot connect to those builds (link protocol 9). Through 0.2.0-beta.109 the ladder ran
between the message and the file, sent its failed frames again at the rung that had failed,
and on a weak path left the file behind its leftovers (ND1J's Tests of 2026-10-15, report 12
§21): file rates from those builds read low on weak paths. The file leaves the ladder its time
(two and a half minutes); a ladder short of time stops (*ladder stopped after N rungs: the
budget ran out*), and a file the path slows under ends when the budget does, the report giving
the bytes it carried.

Both stations must run the same link protocol: a release that changes it says *Update
required*, and a Test, like any call, does not connect across it.

While it runs, the Session tab shows the step (*step 5 of 6, climbing the mode ladder*),
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

**Contribute it**: on the Log tab, **Send files…** → *Report my last test as a GitHub issue* →
**Open the report** opens a pre-filled GitHub issue (when the browser cannot be opened, the link
goes to the clipboard); attach the sidecar — the `.json` beside the `.wav` in the recordings
folder, which *Open folder* on the Session tab's **Recording** box shows you — and submit. The audio is yours to attach or not.
Simpler still: **Send files…** → *Send my files to the Aether project* → **Send** (§5a). The
recording, the sidecar and your logs go to the project's folder in one press, audio included unless
you untick it; a request link from the author opens the form ready. `tools/field_ingest.py` folds what
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

## 5a. When a contact fails: both sides' files

A contact that did not get through has two sides, and the side that was not heard cannot be
read from yours. In the Log tab, **Send files…** has four choices, each showing only its own
fields:

* **Send my files to the Aether project** — the first choice, and the one to use when in doubt.
  It writes one zip — your logs (this run, the one before and the earlier runs kept), the
  session history, the stations heard, the diagnostic bundle (your settings without their
  secrets) and the summaries of the recordings from the period you choose, with one station or
  with anybody — and **Send** uploads it to the project's Google Drive folder, in pieces,
  picking up after a dropped connection. No address, no code, nothing to attach, and no limit at
  email's 25 MB, which a Test session's recording passes. The audio is included unless you
  untick *audio*: a summary lists every frame with its SNR and is a few kB, a recording is megabytes.
* **Email my files to someone** writes the same zip and opens an email to the address you give,
  with the zip's folder open to drag it in.
* **Ask a station to send me its files** writes the request: an email with a link that, opened
  on their computer with Aether HF running, opens their *Send files…* form ready — to the
  project's folder when the author asks, to your email address otherwise. The Stations tab's
  session history has an **Ask for files** button on each row that opens it for that station
  and that session's time.
* **Report my last test as a GitHub issue** (§3).

Every zip waits in the `shared` folder beside your settings. The project's folder takes a
limited number of uploads a day ([`tools/drive_upload/`](../../tools/drive_upload/README.md));
when it is full, email is the way round.

**Debug mode** (Setup step 5, on by default during the field trials; ADR-0050) does the first
choice by itself for every session a host program runs — Winlink Express, Pat or VarAC, P2P or
through a gateway: the session is recorded, and a minute after it ends its recording, the logs
and the diagnostic bundle go to the project, while the station is idle. The panel says so once
when it is on, with *Turn it off*; the Log tab shows each upload (`debug`). The panel's own
sessions and Test sessions are not sent.

The logs record how much was sent, never what. A recording's audio is different: Aether can
decode what was sent from it, which is why the *audio* box says so and can be unticked — and why
debug mode, which includes it, says so.

**Your calls are heard, your answers are not.** When the other station keeps calling after you
accepted, or its probes report no answer while yours get through, the start of your answers
is probably landing while its radio is still on transmit. A station keyed by VOX — a SignaLink,
whose DLY knob holds the key after the audio stops — is the usual case. The fix is at that
station: Setup → Modem settings → **Turnaround**, its DLY time plus 100 ms; every station it
works is told and waits that long. Until it does, **Answer gap** at your end makes your station
wait that long after the other station's transmission before its own: 500–800 ms suits a
SignaLink. It costs a little on every exchange, so leave it at 0 unless a station you work
needs it (ADR-0036, ADR-0069).

**What the receiver hears after you unkey.** `python tools/turnaround_plot.py <recording>.wav
--png turn.png` lines up every release of a recording at t = 0: how soon the radio's audio came
back, whether it rose above the settled noise, the busy detector's reading and any frame that
started. `--detect` finds the releases in a recording made by anything else — a plain recorder
on the radio's USB audio — so the radio's recovery can be compared on one plot (ADR-0037).

## 6. The log

Twenty sessions, three classes. A row per session in `field/LOG.md`: date, band, distance,
class, both callsigns, the recording's name, the mean SNR, the goodput measured and
predicted, and a sentence. The comparison tool prints the numbers; the sentence is yours.
