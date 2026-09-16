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
* **Two instances of each modem.** Aether: two `aetherd` daemons with `[ptt] kind = "none"`,
  `[radio] wait_for_clear = false`, the host adapter on 8300 and 8320, audio devices C4/C1
  for A and C2/C3 for B. VARA: two copies, PTT none, the same devices, their TCP ports set
  in Winlink Express. Nothing keys a radio; the cables carry the audio.
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
