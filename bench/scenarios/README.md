# Scenarios

Each file is one on-air situation for `tools/session_matrix.py` (ADR-0042): two stations, the
path between them, the other signals and the static each one hears, their radios, and what the
operator does. The runner starts a channel server (`tools/channel_server.py`) and two real
daemons on it, drives them through the script over the control API, and judges the session.

```toml
name = "short-name"
description = "what this reproduces"
tags = ["quick"]            # `--tag quick` runs the ones CI runs
bandwidth = 500             # both stations' [radio] bandwidth
seconds = 900               # the most air time the session may take
seed = 1

[stations.a]                # the caller
callsign = "W4ODA"
tx_level = 0.25             # [audio] tx_level; the channel's SNR is against it
answer_gap_ms = 0           # [radio] answer_gap_ms
wait_for_clear = true
[stations.a.radio]          # the radio, as the channel server models it
tx_delay_ms = 20            # the start of each transmission that never radiates
rx_recovery_ms = 280        # silence from the receiver after the key comes up
vox_hold_ms = 0             # a VOX interface's hold after the audio stops
agc = "off"                 # off, fast, auto or slow: the receiver's AGC on the passband
agc_threshold_db = 12.0     # where it acts, above the noise the station hears

[stations.b]                # the called station, the same keys
callsign = "KK4XYZ"
bandwidth = 500             # a station's own [radio] bandwidth, over the scenario's

[path]                      # both directions …
profile = "nvis"            # aether_model.channel.PROFILES
snr_db = 6.0                # 3 kHz noise bandwidth, against the sender's level
cfo_hz = 3.0
cfo_drift_hz_per_s = 0.0    # the offset moving (a rig warming up)
sro_ppm = 0.0               # the receiving card's clock against the sender's
level = [[90, 0], [95, -30], [135, -30], [140, 0]]  # [seconds, dB]: QSB, the band closing
[path.a_to_b]               # … each overridable
snr_db = 8.0

[[qrm]]                     # another station, heard at "a", "b" or "both"
kind = "ofdm-arq"           # ofdm-arq (VARA-class), pactor, rtty
at = "both"
audio_hz = 2600             # its centre, in the receiver's audio
power_db = -6.0             # against this session's signal
partner_db = -10.0          # its partner's acknowledgements (ARQ kinds)
profile = "moderate"        # its own path's fading

[crashes]                   # static from lightning
at = "both"
rate_per_s = 0.5
peak_db = 25.0              # above the noise floor

[script]
steps = ["probe", "connect", "message 2000", "disconnect", "test"]

[expect]                    # what a pass is; leave a key out to not judge it
connected = true
delivered = true            # every message step arrived whole
test = "complete"           # the Test's outcome
clean_end = true            # both sessions ended by DISC, not a timeout
idle = true                 # both stations back to idle at the end, whatever happened
max_collisions = 3          # times both radios were keyed at once
```

Steps: `beacon`, `probe`, `connect`, `message N` (N incompressible bytes from a to b),
`reply N` (from b to a), `test` (a Test session, a → b: it makes its own probe, call, message,
mode ladder, file and disconnect, so it starts from idle), `wait S` (S seconds of air),
`disconnect`. `transfer_s` and `connect_within_s` set how much air a message or a call may
take (300 and 180). Each message step's rate is in the result's `transfers`.

Tags: `quick` (CI), `nightly` (the nightly job), `stress` (conditions chosen to break things —
run by hand, `--tag stress --jobs 4`; not every one is expected to pass). `[test]` passes `test.start`'s parameters (`message_bytes`, `file_bytes`,
`budget_s`, …).
