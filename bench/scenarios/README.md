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
max_mode = 19               # [radio] max_mode (default 19 at 2300 Hz, 14 at 500 Hz)
[stations.a.radio]          # the radio, as the channel server models it
tx_delay_ms = 20            # the start of each transmission that never radiates
rx_recovery_ms = 280        # silence from the receiver after the key comes up
vox_hold_ms = 0             # a VOX interface's hold after the audio stops
agc = "off"                 # off, fast, auto or slow: the receiver's AGC on the passband
agc_threshold_db = 12.0     # where it acts, above the noise the station hears
agc_hang_ms = 100           # how long a gain cut holds before it recovers
agc_decay_db_s = 40.0       # how fast it recovers

cw_id = false               # [radio] cw_id, and cw_id_interval_s
host = false                # a host (VARA-compatible) port, for the host-* steps

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
delay_ms = 40.0             # the path's travel time: 10–60 ms on a DX path
echo = { delay_ms = 90.0, db = -6.0 }  # long path behind short path: a second arrival
[path.a_to_b]               # … each overridable
snr_db = 8.0
[path.b_to_a]
snr_db = 4.0

[[qrm]]                     # another station, heard at "a", "b" or "both"
kind = "ofdm-arq"           # ofdm-arq (VARA-class), pactor, rtty, ft8 (`count` stations
                            # spread over `spread_hz`, each ± 6 dB, on their 15 s slots)
at = "both"
audio_hz = 2600             # its centre, in the receiver's audio
power_db = -6.0             # against this session's signal
partner_db = -10.0          # its partner's acknowledgements (ARQ kinds)
profile = "moderate"        # its own path's fading

[crashes]                   # static from lightning
at = "both"
rate_per_s = 0.5
peak_db = 25.0              # above the noise floor
spread_db = 6.0             # how much the crashes' peaks vary

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
`disconnect` (`disconnect b`: the called station leaves; `disconnect both`: at the same
moment), `abort`. For what goes wrong mid-session: `send N` (queued, not waited for),
`message? N` (a message that may fail — reported, not judged), `outage S` (from now neither
station hears the other for S seconds: a station switched off, a band gone), `wait_idle S`
(both back to idle by themselves within S seconds of air). With `continue_on_failure = true` in
`[script]` a failed step does not end the script. `transfer_s` and `connect_within_s` set how
much air a message or a call may take (300 and 180). Each message step's rate is in the
result's `transfers`.

**Host programs** (`host = true` at a station): `host a client` attaches a Winlink Express-like
program to a's host port (its opening line as the bench recorded it), `host b trimode` an RMS
Trimode-like gateway that scans — `LISTEN` off half a second in every three and a half of air,
and a `CONNECTED` it is told while not listening is a failure (ADR-0051). Then `host-connect`
(a's program calls b), `host-send N` / `host-reply N` (bytes over the data ports, a → b and
b → a) and `host-disconnect` (a's program's `DISCONNECT`; both told `DISCONNECTED`).

**Always judged**, whatever `[expect]` says: a daemon that exits or panics, a station keyed
longer than 32 s at once (a stuck key), a host told `CONNECTED` while not listening.
`tools/soak.py` draws random scenarios — every fading profile, offsets, drift, echoes, QSB, the
other signals, AGCs, VOX, both bandwidths, aborts, outages, host programs — and judges them on
these alone, plus being back to idle after the script has left (`--count`, `--seed`, `--jobs`;
each scenario's TOML is kept beside its run).

Tags: `quick` (CI runs these: `--tag quick`); the nightly job runs every scenario, the stress set
included, and `nightly` marks the ones expected to pass there; `stress` (conditions chosen to
break things — `--tag stress --jobs 4` by hand; not every one is expected to pass: on
`80m-asymmetric-500` a file transfer after the Test's ladder stalls about one run in three, an
open item); `20m`, `15m`, `40m` and `80m` the band; `robustness` and `gateway` what an operator
or a gateway's day does to a session (aborts, outages, leaving together, a Morse identifier
mid-transfer); `host` the host-program sessions; `band` the 20 m and 15 m paths (DX travel time,
offsets that grow with the band, polar Doppler, equatorial flutter, long-path echoes, openings
closing, FT8 beside the passband); `winlink` the Winlink-shaped exchanges (ADR-0044's and
ADR-0047's measurements). `[test]` passes `test.start`'s parameters (`message_bytes`, `file_bytes`,
`budget_s`, …).

## Running them

```
uv run python tools/session_matrix.py [scenarios…] [--tag T] [--jobs N] [--daemon PATH]
                                      [--daemon-b PATH] [--out runs/] [--csv FILE]
```

The daemon is `core/target/release/aetherd` unless `--daemon` names another; `--daemon-b` runs
the called station on another build (a release against this tree). Each run leaves
`result.json`, `channel.json` (when each station was keyed, and every collision) and both
daemons' logs and recordings under `--out`; `--csv` appends one line per scenario. Runs of one
build are not identical (the daemons' threads), so judge a single difference against a second
run, and an A/B on the same seeds; a scenario's `seed = N` and a `-sN` in its name make a
variant.
