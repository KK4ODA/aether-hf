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

## The setup

One radio transmits into a box that soaks up nearly all its power, so nothing goes on the air. A
tiny, measured share of the signal goes on, through a second box that takes off as much more as
you choose, into the other radio's antenna jack. One computer plays the test sequence into the
first radio and records what the second one hears.

### The picture

```
mini PC ──DigiRig──► TM-D710 (sends, Low: about 5 W)
                        │
                  PL-259 patch cable
                        │
                 ┌──────┴──────┐
                 │ 40 dB power │  the dummy load and the first 40 dB, in one box
                 │ attenuator  │
                 └──────┬──────┘
                        │  about −3 dBm
                 ┌──────┴──────┐
                 │    Step     │  you choose how much more comes off: 0–120 dB
                 │ attenuator  │
                 └──────┬──────┘
                        │
               IC-9700, 2 m antenna jack (receives only)
                        │
mini PC ◄──USB─── IC-9700
```

For the other direction, swap the two radios' places in the chain. The sending radio is always the
one on the power attenuator.

### Why two boxes

- 5 W is +37 dBm. The receiver should get −60 dBm for a strong signal and about −115 dBm for a
  weak one. So 97–152 dB has to come off.
- **The power attenuator** is a 50 Ω load, a dummy load, that lets out one ten-thousandth of the
  power at its other end (40 dB). It turns the rest into heat. It does the job of a dummy load
  with a sampling port, in one box.
- **The step attenuator** takes off 0–120 dB more, in switched steps. It can take only half a
  watt, which is why it sits after the power attenuator, never on a radio directly.

**Never connect a transmitter straight to the receiver**, or to the step attenuator. Even 5 W
through 40 dB alone is +7 dBm at the receiver, enough to overload it.

### The levels

With 40 dB in the power attenuator (the cables' loss at 2 m is small enough to ignore):

| Level | At the receiver | Step attenuator, TM-D710 sending at 5 W | Step attenuator, IC-9700 sending at 0.5 W |
|---|---|---|---|
| Strong (full quieting) | about −63 dBm | 60 dB | 50 dB |
| Medium | about −103 dBm | 100 dB | 90 dB |
| Weak | about −113 dBm | 110 dB | 100 dB |

For the weak run, go by ear as well: the step where the receiver's audio first gets a hiss.

### What to buy

1. **A 40 dB coaxial power attenuator, rated 100 W continuous or more, DC to at least 1 GHz,
   N connectors.** The 100 W rating is insurance: if a radio is keyed at full power by mistake
   (50 W on the TM-D710, 100 W on the IC-9700's 2 m), it survives. Its output is then still under
   the step attenuator's half watt. 1 GHz covers 2 m and 70 cm. Generic ones are sold widely;
   used Bird, Narda, Weinschel or JFW units are as good.
2. **A step attenuator, 0–110 dB or more, 50 Ω, good to at least 500 MHz.** The classic is the
   HP (later Agilent) 355D, common used:
   - 0–120 dB in 10 dB steps;
   - DC to 1 GHz;
   - half a watt at most;
   - BNC connectors.

   Its sibling, the 355C (0–12 dB in 1 dB steps), is a nice extra for fine steps, not needed.
3. **Or, cheaper than a step attenuator: fixed attenuator pads**, BNC or SMA, rated 2 W, a few
   dollars each. Three 30 dB, one 20 dB and one 10 dB cover every level, after the power
   attenuator:
   - 60 dB: 30 + 20 + 10;
   - 100 dB: 30 + 30 + 30 + 10;
   - 110 dB: 30 + 30 + 30 + 20.

   You swap pads instead of turning a knob.
4. **Adapters.** From your PL-259 patch cable into the power attenuator (UHF to N), from the power
   attenuator to the step attenuator (N to BNC, and a short BNC jumper), and from the step
   attenuator to the receiving radio's jack (BNC to UHF). Check the genders against what you buy.
   For 70 cm later, the IC-9700's 70 cm jack is N, so an N jumper goes straight to it.
5. **Optional:** an inline wattmeter, to check each radio's lowest power once.

### The all-N kit (the parts that are easy to buy new)

New step attenuators sold to hobbyists are push-button units that stop at 90 dB. That is not
enough after 40 dB alone, so a fixed 30 dB pad goes between them. Everything is N, so only three
adapters are needed:

```
sending radio ─ PL-259 patch cable ─ [N female to SO-239 adapter]
  ─ 40 dB, 100 W (N male in, N female out)
  ─ 30 dB, 10 W  (N male in, N female out)
  ─ [N male to N male barrel]
  ─ 0–90 dB push-button step attenuator (N female both sides)
  ─ [N male to SO-239 adapter] ─ PL-259 patch cable ─ receiving radio
```

The fixed attenuators' male end is their input: it faces the sending radio. Many are made to take
their power that way round only.

| Level | At the receiver | Step setting, TM-D710 sending at 5 W | Step setting, IC-9700 sending at 0.5 W |
|---|---|---|---|
| Strong | about −63 dBm | 30 dB | 20 dB |
| Medium | about −103 dBm | 70 dB | 60 dB |
| Weak | about −113 dBm | 80 dB | 70 dB |

A push-button unit loses up to about 3 dB even at 0, and each button is accurate to a decibel or
so. That is fine: the analysis needs the levels roughly, and the weak one is set by ear.

### Your station: the TM-D710 and the IC-9700 on the mini PC

A good setup as it is:
- one computer plays and records, so the two sound cards' clocks are compared exactly;
- both radios can send, so each transmitter and each receiver is measured;
- one of them, the TM-D710, has a 9600 data port.

- **TM-D710:** power **Low** (about 5 W), the DigiRig on its data port. It is keyed by the DigiRig's
  serial line, RTS (`--key rts:COMx`, the DigiRig's COM port). Write down its DATA SPEED (1200 or
  9600), and which receive line the DigiRig cable uses: the 1200 audio or the 9600 line. That
  decides whether a recording is the voice path or the flat port.
- **IC-9700:** RF power at its lowest (0.5 W on 2 m), in FM data mode (FM-D) with the USB audio as
  the modulation source. Set its USB SEND to RTS on one of its USB serial ports, so the tool keys
  it with `--key rts:COMy`. Use its 2 m antenna jack.
- **Start on 2 m.** Both radios have SO-239 jacks there. 70 cm comes later.
- **For the weak runs, let the IC-9700 send** if caution 2 below shows leakage. At 0.5 W it leaks
  10 dB less than the TM-D710 at 5 W.
- **A free look at the signal's width:** while the TM-D710 sends, the IC-9700's spectrum scope
  shows how wide its signal is.
- **Close every other program that uses these sound cards or COM ports** during the runs: Aether
  HF, RadioPost, WSJT-X, rig control. They can hold a device or key a radio.

### Three cautions

1. **The receiving radio must never transmit into the chain.** Its power would go backwards into
   the step attenuator (half a watt at most) and burn it.
   - `fm_bench.py` opens only the sending radio's COM port, so the other radio's keying line
     stays unused.
   - Keep the receiving radio's VOX off, and its power at the minimum as a backstop.
   - To change direction, swap the radios in the chain; never just key the other one.
2. **Leakage.** At the weak level, the receiving radio may hear the sending one straight through the
   air, around the cables.
   - Before the weak runs, set the step attenuator to its maximum, and key the sending radio for a
     few seconds with its microphone, saying nothing.
   - If the receiver still shows a signal, it is leaking. Move the radios a couple of metres
     apart, use well-shielded jumpers, and tighten every connector.
3. **Heat.** The sequence keeps the radio keyed for most of two minutes. At Low that barely warms
   a 100 W attenuator, but don't run it at full power.

### If you don't want to buy the step attenuator yet

The sending radio goes into the power attenuator (or a plain dummy load) at its lowest power. The
receiving radio uses a short rubber duck across the room. The leakage alone gives a signal, and you
vary it by distance. You lose exact levels, but it is enough for a first try of the tool.

Use a simplex frequency nobody is using. These runs radiate almost nothing. If anything could be
heard, say your callsign before you start.

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
2. **Play and record.** Use the sound card names `devices` printed, and the sending radio's COM
   port. The TM-D710 sending, keyed by the DigiRig's RTS, the IC-9700 receiving (the DigiRig is
   usually "USB PnP Sound Device", the IC-9700 "USB Audio CODEC"; COM7 stands for the DigiRig's
   port):
   ```bash
   python -m uv run --with pyserial python tools/fm_bench.py play C:\Dev\AetherBench\fm\seq\voice.json --out C:\Dev\AetherBench\fm\tmd710-to-ic9700\voice-strong.wav --play "USB PnP Sound Device" --record "USB Audio CODEC" --key rts:COM7
   ```
   The other way round, the IC-9700 sending, keyed by RTS on the USB serial port its USB SEND
   names (COM8 here):
   ```bash
   python -m uv run --with pyserial python tools/fm_bench.py play C:\Dev\AetherBench\fm\seq\voice.json --out C:\Dev\AetherBench\fm\ic9700-to-tmd710\voice-strong.wav --play "USB Audio CODEC" --record "USB PnP Sound Device" --key rts:COM8
   ```
   A radio with a VOX interface (a SignaLink) needs no `--key`: the interface keys on the audio.
   A run takes two minutes and writes the recording and a `.keys.json` beside it.
3. **Check the curve straight away:**
   ```bash
   python -m uv run python tools/fm_bench.py analyse C:\Dev\AetherBench\fm\seq\voice.json C:\Dev\AetherBench\fm\tmd710-to-ic9700\voice-strong.wav --out C:\Dev\AetherBench\fm\tmd710-to-ic9700\voice-strong.json
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
