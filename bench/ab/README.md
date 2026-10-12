# The A/B bench: VARA HF and Aether through the same channel

Roadmap P9-1. The one measurement that settles "comparable or better" before the air does:
two modems, the same calibrated channel at the audio level, the same message, the same host
program at both ends, and the goodput of each per SNR and ITU profile. This directory holds
the protocol, the tool and the results. It holds nothing of VARA — no binary, no
configuration, no capture of its signal — and the runs are the author's, on their own
registered copies.

## The idea

`tools/channel_cable.py` is the reference model's channel simulator (the ITU-R F.1487
profiles and the 3 kHz-referenced SNR every curve in `bench/baselines/` is measured with)
running in real time between virtual audio cables. What one modem transmits into its cable
is captured, impaired, and played into the other modem's receive cable; the other
direction the same, through its own independent fading. A modem sees a sound card, as it
would at a radio; the channel between the two is the bench's, identical for both modems,
and the SNR is the number the rest of this repository means by SNR.

## What you need

* **Four virtual cables** on one Windows machine, all set to 48 000 Hz in Windows Sound
  (both the playback and the recording side of each; the tool and Aether run at 48 kHz).
  VB-Audio's free CABLE gives one; the A+B and C+D packs give four more. Name them by
  their role below so nothing gets crossed:

  | Cable | What plays into it | What captures from it |
  |---|---|---|
  | C1 | modem A transmits | the tool (`--a-tx`) |
  | C2 | the tool (`--b-rx`) | modem B receives |
  | C3 | modem B transmits | the tool (`--b-tx`) |
  | C4 | the tool (`--a-rx`) | modem A receives |

* **Two copies of the host program** — Winlink Express in two scratch installations (the
  Phase 6 bench's), each with its own callsign, each pointed at its modem's TCP port. The
  same message goes through both modems: a P2P message with a 6 kB incompressible
  attachment, or whatever fixed size you settle on, the same for every run.
* **Two instances of each modem.** Aether: two `aetherd` daemons from
  [`aether-a.toml`](aether-a.toml) and [`aether-b.toml`](aether-b.toml) — `[ptt] kind =
  "none"`, `[radio] wait_for_clear = false`, audio devices C4/C1 for A and C2/C3 for B, the
  host adapter on 8400 and 8420 and the control API on 8615 and 8616 (clear of an installed
  station's 8300 and 8515), `[regulatory] profile = "none"` (nothing radiates), and
  `[record] send_to_project = false` so bench sessions are not uploaded as field sessions.
  Copy them to a bench folder (`C:\Dev\AetherBench\ab\`) and run them there: a daemon writes
  its profiles and history beside its configuration. VARA: two copies, PTT none, the same
  devices, their TCP ports set in Winlink Express; both registered, as an unregistered VARA HF
  is limited in speed. Nothing keys a radio; the cables carry the audio.
* **A wiring check first**: Aether to Aether through the tool at `--channel awgn --snr 20`.
  The session's OFDM frames should read about 20 dB in each daemon's sidecar (tone frames read
  lower: their estimate tops out near 17 dB); none, or a reading far off, means a cable crossed
  or a level wrong.
* `uv sync --extra audio` once, for the tool's sound-card library.

## Levels

The SNR is relative to the modem's transmit level. Set both modems to the same drive and
tell the tool what it is: `--signal-dbfs -15` means an RMS of −15 dBFS on the cable, which
is Aether's default (`tx_level = 0.25`). The tool prints each direction's input level once
a second; adjust VARA's drive until its bursts read the same. Or run with `--auto-level`,
which follows each burst's measured power so the SNR holds whatever the drive — the fairer
setting when two modems will not sit at the same level, and the one the results below use
unless a row says otherwise.

## A run

```bash
python tools/channel_cable.py --a-tx "CABLE Output" --b-rx "CABLE-A Input" \
    --b-tx "CABLE-B Output" --a-rx "CABLE-C Input" --channel good --snr 10 --auto-level \
    --log bench/ab/logs/good_10_vara.csv
```

Then, with both host programs up, send the message from A to B and note the time from
*connected* to *disconnected* as the host program logs it. Goodput is the message's bytes
times eight over that time. One row per run in `results.csv`; three runs per point when
the numbers disagree, one when they do not. Stop the tool, change the modem, run the same
point again. Keep the pairs close in time: the machine's load is part of the channel.

## The grid

Start where the two are likely to differ, not where both are trivially fine:

| Profile | SNR (dB) |
|---|---|
| awgn | 0, 4, 10, 16 |
| good | 4, 8, 14 |
| poor | 6, 10, 14 |

Add points where a curve bends. Both modems are given the same tries; a session that fails
to connect is a row with `seconds` empty and the note saying so.

## Step by step, the first evening

1. **Cables.** The configurations use four: VB-Audio's free CABLE and cables A, B and C from
   its A+B and C+D packs (D is spare). Installing a pack wants a reboot. In Windows Sound, set
   each one's playback *and* recording side to 48 000 Hz (Properties → Advanced), turn off
   enhancements and *Listen to this device*, and keep the default playback device on your
   speakers: a system sound played into a cable goes into the channel.
2. **Close the installed Aether.** Nothing collides with it, but its daemon costs the machine
   time the channel needs.
3. **The tool.** In the checkout: `git pull`, `uv sync --extra audio`, then
   `uv run python tools\channel_cable.py --list-devices` to see the cables' names.
4. **The daemons.** Copy `aether-a.toml` and `aether-b.toml` to `C:\Dev\AetherBench\ab\`.
   Check the device names against `aetherd.exe --list-devices` and fix any that differ. To
   watch each station's panel, add `ui_dir = 'C:\Users\Facundo\AppData\Local\Aether HF\ui'` under
   `[control]`; the panels are then at `http://127.0.0.1:8615/` and `:8616`. Start each in its
   own window from the bench folder:
   `& "C:\Users\Facundo\AppData\Local\Aether HF\aetherd.exe" --config aether-a.toml` (and `-b`).
5. **The host program.** Two scratch Winlink Express copies under `C:\Dev\AetherBench\`, one
   `KK4ODA`, one `WD5EMA` (it proposes nothing to an SSID). Each: a *Vara HF P2P* session on
   127.0.0.1 at port 8400 (A) or 8420 (B), set before its session window is first opened; the
   TNC path filled in and auto-launch unticked; bandwidth **2300** in the session window (at
   2750 VARA would run wider than Aether can); any centre frequency; no rig control. The
   message: one P2P message from A to B with the same 6 000-byte random `.dat` attachment
   every run (`.bin` is refused). Make it once in PowerShell:
   `$b = [byte[]]::new(6000); [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($b); [IO.File]::WriteAllBytes('C:\Dev\AetherBench\ab\random6000.dat', $b)`.
6. **VARA.** Two copies in their own folders. Give them the same cables as the Aether
   stations (A: input CABLE-C Output, output CABLE Input; B: input CABLE-A Output, output
   CABLE-B Input), PTT none, and TCP command ports 8400 (A) and 8420 (B), so Winlink Express
   needs no change between modems. Check that *both* show registered before the first run.
7. **Wiring check.** Aether's daemons up, then
   `uv run python tools\channel_cable.py --a-tx "CABLE Output" --b-rx "CABLE-A Input" --b-tx "CABLE-B Output" --a-rx "CABLE-C Input" --channel awgn --snr 20 --auto-level`.
   Send the message. Each panel's SNR should read about 20 dB on OFDM frames.
8. **A point.** Stop the tool, start it at the point (`--channel good --snr 8`, with
   `--log bench\ab\logs\good_8_aether.csv`), send the message, and take *connected* to
   *disconnected* from Winlink Express's session window. Then stop the daemons, start the VARA
   copies, and run the same point the same way. Alternate which modem goes first from point to
   point. A run whose tool printout shows over/under counts above zero is the machine falling
   behind: run it again.
9. **Write it down.** One row per run in `results.csv`; `bytes` is the attachment's 6 000 for
   both modems (Winlink Express's own byte count goes in the note); `level` is `auto`. Then
   `uv run python tools\ab_summary.py`.

## The results

`results.csv`: `date, modem, version, profile, snr_db, level, bytes, seconds, goodput_bps,
connected, note`. `tools/ab_summary.py` prints the two modems side by side per point with
the ratio. When a grid is complete it is copied into the README here as a table, and the
same two modems are then alternated on one on-air path within minutes of each other, which
goes into `field/LOG.md`.

## What this bench cannot say

The cable carries no radio: no AGC, no ALC, no keying delay, no receiver front end. The
numbers compare the modems' waveforms, coding and ARQ on a common channel; they say nothing
about how either handles a real rig, which is what the alternated on-air runs are for.
