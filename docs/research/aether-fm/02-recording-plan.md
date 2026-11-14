# Recordings of FM radios (step 10.2b)

These recordings decide whether the FM channel model (ADR-0115) can be trusted. Every number in the
FM proposal comes from a model of the radio. A real radio may lose more to its limiter, its
microphone compressor, its filters or its sound card than the model says (the proposal's risks
R1–R4 and R14). So real radios are recorded first, and the waveform is chosen afterwards.

**No other station is needed, and nothing goes on the air.** One radio transmits into a dummy
load; a second radio hears it through an attenuator.

## What a recording measures

`tools/fm_bench.py` plays a test sequence into one radio and records the other. It then
reports:

| Reading | What it shows | Risk it settles |
|---|---|---|
| SNDR against drive, 24 steps 1.5 dB apart | How good the audio gets, and at what drive, before the limiter bends it over | R1, R2 |
| The band's response, 150–3 500 Hz (voice) or 40–6 500 Hz (flat) | The radio's filters, its emphasis and any tone-squelch high-pass | R3 |
| The floor | The radio's own hum and noise, with a carrier and no audio | R1 |
| Key to audio, release to noise, release to silence | Transmitter attack, squelch opening, squelch closing | R4 |
| Clock offset | The two sound cards' clocks against each other | R14 |

The sequence is 117 s long, in eight transmissions:
1. The first transmission holds a carrier with no audio (the floor), then a wideband probe (the
   band).
2. The next six step a 16-QAM signal through the levels. It has the data's own spectrum and peaks,
   because a tone would mislead the limiter.
3. The last is a marker alone.

Every transmission opens with a marker, so the recording can be lined up and the clocks compared.

## What you need

- **Two FM radios**, with an audio interface for each (a DigiRig, a SignaLink or the radio's own
  USB). One of them should have a 9600 data port. Later, more models: 5–10 in all, a handheld
  among them.
- **A dummy load** rated for the transmitter's lowest power, at least 5 W.
- **A way to bring the signal down to the receiver**, at least 90 dB below the transmitter.
  Either:
  - **Best:** a power sampler (a tap, −40 to −60 dB) on the dummy load, then a step attenuator
    (0–100 dB) to the receiver's antenna jack. You can then set the received level exactly.
  - **Simplest:** the transmitter on its lowest power into the dummy load, and the receiver on a
    short piece of wire a few metres away. The leakage is enough. You change the level by moving
    the wire. You don't know the level exactly, but the S-meter reading helps.
- **One computer with both interfaces** is best, so one program plays and records together. Two
  computers work too: the markers line them up.
- Optional, for R4: an SDR, to see when the carrier really comes up.

**Never connect the transmitter's output straight to the receiver.** Even 5 W through 40 dB is
+7 dBm, enough to overload or damage a receiver's front end. Aim for −110 to −50 dBm at the
receiver.

Use a simplex frequency nobody is using, and the lowest power. Most of these runs radiate almost
nothing. If anything could be heard, say your callsign before you start.

## Once, before the first recording

1. **Install the audio extra** (from the repository folder):
   ```bash
   python -m uv sync --extra audio
   ```
2. **List the sound cards** and note the names of the two interfaces:
   ```bash
   python -m uv run python tools/fm_bench.py devices
   ```
3. **Make the sequences.** This writes `voice.wav`, `voice.json`, `flat.wav` and `flat.json`:
   ```bash
   python -m uv run python tools/fm_bench.py make --port voice --out C:\Dev\AetherBench\fm\seq
   ```
   ```bash
   python -m uv run python tools/fm_bench.py make --port flat --out C:\Dev\AetherBench\fm\seq
   ```
4. **Windows sound settings.** For both interfaces, turn off *Audio enhancements* and any
   *Communications* ducking. Write down the playback and recording volumes, and don't change them
   during a radio's recordings.

## For each radio pair

Write down, in a `notes.txt` beside the recordings:
- both radio models;
- the port: the microphone/data jack at 1200, or the 9600 data port;
- the radio menu items that touch the audio: data speed, mic gain, wide or narrow, the tone
  squelch;
- the transmitter's power;
- how the signal reaches the receiver: attenuator setting, or wire distance and the S-meter
  reading;
- the frequency;
- the interfaces.

1. **Set both radios** to the same simplex frequency, wide (25 kHz) if the radio allows it, no
   tone squelch. Set the receiver's squelch where you would use it, and its volume to about half.
2. **Play and record.** With VOX keying (the interface keys on the audio):
   ```bash
   python -m uv run python tools/fm_bench.py play C:\Dev\AetherBench\fm\seq\voice.json --out C:\Dev\AetherBench\fm\ftm400-tmv71\voice-strong.wav --play "USB Audio CODEC" --record "SignaLink"
   ```
   With RTS keying on a serial port, add `--key rts:COM9` and run it with pyserial:
   ```bash
   python -m uv run --with pyserial python tools/fm_bench.py play C:\Dev\AetherBench\fm\seq\voice.json --out C:\Dev\AetherBench\fm\ftm400-tmv71\voice-strong.wav --play "USB Audio CODEC" --record "DigiRig" --key rts:COM9
   ```
   It takes two minutes and writes the recording and a `.keys.json` beside it.
3. **Check the curve straight away:**
   ```bash
   python -m uv run python tools/fm_bench.py analyse C:\Dev\AetherBench\fm\seq\voice.json C:\Dev\AetherBench\fm\ftm400-tmv71\voice-strong.wav --out C:\Dev\AetherBench\fm\ftm400-tmv71\voice-strong.json
   ```
   The peak must fall inside the steps:
   - If it says *still rising at the top step*, raise the playback volume about 10 dB and repeat.
   - If it says *falling from the first step*, lower it 10 dB and repeat.
   - Write down the volume that worked.
4. **Three signal levels.** Repeat step 2 at a strong signal (about −60 dBm, or the wire close), a
   medium one (about −100 dBm), and a weak one near where the receiver starts to hiss (about
   −115 dBm). Name the files `-strong`, `-medium` and `-weak`.
5. **Once with the squelch open.** Turn the receiver's squelch fully open and repeat the strong
   run, named `-open`. Opening the squelch separates the transmitter's attack from the squelch's
   delay.
6. **The other way round.** Swap the radios and repeat 2–5. Each radio's transmitter and receiver
   are both measured.
7. **The flat port**, if a radio has a 9600 port. Set its data speed to 9600, use `flat.json`, and
   repeat 2–5. The receiving radio's data-port output is usually not squelched; that is fine.

## What to send

- The recordings (`.wav`, about 11 MB each) and their `.keys.json`: in a shared Google Drive
  folder, as the field uploads were.
- The `notes.txt` and the `.json` analyses: in the same folder.

I fit the model to each radio and keep the fit as that radio's profile under
`model/aether_model/fm/radios/`. Gate 2 passes when the model reproduces every recording within
ADR-0115 §3's limits:
- SNDR against drive within 2 dB from 15 dB under the peak to 6 dB over it, at each level;
- the peak's drive within 2 dB;
- the band within 2 dB wherever it is within 20 dB of its peak;
- the floor within 2 dB.

A radio the model cannot fit is a finding: a stage is missing, and it gets added.

## Checked so far

- The sequence and the analysis are checked on the model (`model/tests/test_fm_bench.py`). The
  model's sound cards were set 60 ppm fast and 90 ppm slow, and the recording started 0.37 s
  early. The analysis found −150 ppm and 0.37 s, and read back the curve, the band, the squelch
  and the times the model was built with.
- `play` has not run on real sound cards yet. The first bench session will show whether its
  keying times need work.
