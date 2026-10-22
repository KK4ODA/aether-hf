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
turnaround_ms = 0           # [radio] turnaround_ms, the radio's own (a daemon of schema 12
                            # and up; an older build's file leaves it out)
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

cw_id = false               # [radio] cw_id; a US callsign identifies whatever it says
                            # (ADR-0075), and an older build reads cw_id_interval_s
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
                            # spread over `spread_hz`, each ± 6 dB, on their 15 s slots),
                            # carrier (a steady tone: a broadcast carrier, a key held down)
windows = [[60, 80]]        # on exactly through these [start, end] seconds of air, steady,
                            # instead of the kind's own rhythm
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
min_message_bps = 75        # the Test's message at least this fast
identifiers_last = true     # every session a station that identifies was in ends with its
                            # Morse identifier, and one that sends none sends none (ADR-0075)
```

Steps: `beacon`, `probe`, `connect`, `message N` (N incompressible bytes from a to b),
`reply N` (from b to a), `test` (a Test session, a → b: it makes its own probe, call, message,
mode ladder, file and disconnect, so it starts from idle), `wait S` (S seconds of air),
`disconnect` (`disconnect b`: the called station leaves; `disconnect both`: at the same
moment), `abort` (`abort a`: waits for the caller alone; `abort b`: the called station drops the
session, and the caller closes it). For what goes wrong mid-session: `send N` (queued, not waited for; `send b N` at the called station),
`message? N` (a message that may fail — reported, not judged), `outage S` (from now neither
station hears the other for S seconds: a station switched off, a band gone), `wait_idle S`
(both back to idle by themselves within S seconds of air). `at T` waits until second T of
air (to line a script up with a signal's `windows`); `clear WHO S` passes when that station's
channel reads clear, and stays clear for 2 s, within S seconds of air, and `busy WHO S` when it
reads busy (the time each took is in `transfers`). `stall WHO S` (that station's
computer stops for S seconds of air). With `continue_on_failure = true` in `[script]` a failed
step does not end the script. `setup = [...]` runs once before `steps`, and `repeat = N` runs
`steps` N times; each daemon's memory, open files and threads are sampled after every round
(the result's `resources`, from `/proc`, Linux only), and `[expect]`'s `max_rss_growth_mb`,
`max_fd_growth` and `max_thread_growth` judge their growth from the first round's end to the
last. `transfer_s` and `connect_within_s` set how
much air a message or a call may take (300 and 180). Each message step's rate is in the
result's `transfers`.

**A slow computer** (`[stations.<who>.machine]`): `stall_every_s` (mean seconds between
stalls, drawn from the seed), `stall_s` (a length, or `[shortest, longest]`), `backlog_s` (2, the
daemon's capture queue). In a stall the station neither hears nor answers; what it queued plays
on, the newest `backlog_s` of what it heard is kept, and it catches up when the stall ends
(`channel.json` lists each station's `stalls` and `stall_dropped_s`). `skip_every_s` (mean seconds
between skips) and `skip_ms` (a length, or `[shortest, longest]`; 20–60) make its sound card's
capture skip audio: what fell in a skip never arrives. A build that greets the channel server with
`AEC3` is told each skip as its capture timestamps would say it and puts it back as silence
(ADR-0089); an older one hears the audio either side run together (`channel.json`'s
`capture_skips`, `[start, seconds]`).

**Host programs** (`host = true` at a station): `host a client` attaches a Winlink Express-like
program to a's host port (its opening line as the bench recorded it), `host b trimode` an RMS
Trimode-like gateway that scans — `LISTEN` off half a second in every three and a half of air,
and a `CONNECTED` it is told while not listening is a failure (ADR-0051), and `host a varac` a
VarAC-like program (`CHAT ON`, its `-T` alias in `MYCALL`, no `CWID`). Then `host-connect`
(a's program calls b), `host-ping` (VarAC's ping: a `CONNECT` to b's `-T` alias, a 14-byte report
back, a's `DISCONNECT`), `host-send N` / `host-reply N` (bytes over the data ports, a → b and
b → a) and `host-disconnect` (a's program's `DISCONNECT`; both told `DISCONNECTED`). A program
told `DISCONNECTED` before its station's closing identifier has gone out is a failure
(ADR-0075).

**More stations** (`[stations.c]`, `[stations.d]`, …: other clients of the gateway `b`). Each
receiver hears its main peer — `b` for a client, `a` for `b` — through the path with its noise,
static and other signals, and every other station through a noise-free fading path of its own
(`[path.c_to_a]`, …, `[path]` by default) at that path's SNR against the same floor: one noise
floor per receiver, however many it hears. A two-station scenario draws exactly what it did
before. The host steps name the client: `host-connect c`, `host-send N c`, `host-reply N c`,
`host-disconnect c` (`a` when none is named); `host-call c` places a call the script does not
wait for, `host-wait c` waits for it to come up — a call into a gateway busy with another
client (`40m-gateway-busy-when-called-2300`). Every client runs `--daemon`, `b` `--daemon-b`.

**Always judged**, whatever `[expect]` says: a daemon that exits or panics, a station keyed
longer than 32 s at once (a stuck key), a host told `CONNECTED` while not listening, a host told
`DISCONNECTED` before its station's closing identifier went out, and — for a build that places
the Morse identifier last (schema 13 and up, ADR-0075) — an identifier anywhere but alone or
behind a `DISC_ACK`, or anything of a session keyed after the identifier that closed it. "Idle"
everywhere means idle with no closing identifier still to go out. So is the run itself: air that
stands still for a minute of wall clock ("the air stopped at N s") and an error of the runner's
own ("harness: …") fail that run alone, and the rest of the set goes on. A channel server whose
air has stood still for 30 s writes to its `channel.log` what it was doing, what it was reading
from each station, the bytes read and the READYs it was owed, what the kernel still holds at both
ends of each connection (Linux), and every thread's stack.
`tools/soak.py` draws random scenarios — every fading profile, offsets, drift, echoes, QSB, the
other signals, AGCs, VOX, both bandwidths, aborts, outages, host programs — and judges them on
these alone, plus being back to idle after the script has left (`--count`, `--seed`, `--jobs`;
each scenario's TOML is kept beside its run).

`long/` holds runs of hours (a gateway's day) that the weekly workflow runs and the nightly's
glob does not reach.

Tags: `mixed` (the nightly runs these between this build and the last release, both ways round:
`tools/mixed_pair.py`); `quick` (CI runs these: `--tag quick`); the nightly job runs every scenario, the stress set
included, and `nightly` marks the ones expected to pass there; `stress` (conditions chosen to
break things — `--tag stress --jobs 4` by hand; not every one is expected to pass: on
`80m-asymmetric-500` a file transfer after the Test's ladder stalls about one run in three, an
open item); `20m`, `15m`, `40m` and `80m` the band; `robustness` and `gateway` what an operator
or a gateway's day does to a session (aborts, outages, leaving together, a long transfer
between identifying stations); `identifier` the ways a session ends, each judged on its closing
identifiers coming last (`40m-closing-identifiers-2300` with two US callsigns, `-ve3-` with one
that sends none); `host` the host-program sessions; `band` the 20 m and 15 m paths (DX travel time,
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
daemons' logs and recordings under `--out`; `--csv` appends one line per scenario.

A *collision* there is both radios keyed at once, which includes a keyed lead or tail of
silence and a VOX's hold. The report splits that time (ADR-0069 §5): `overlap_s` gives
`radiated` (signal against signal: the protocol's collisions, also listed as `rf_overlaps`),
`vox` and `guard` (keyed silence). Each station's `transmissions` place the daemon's own
account of every transmission on the air — the engine's cause (`timer:ack`, `timer:wait`,
`frame`, …), its role and state, its frames, how long it was held — and `deaf_hits_s` says how
much of its peer's signal reached it while it could not hear, and why. The results row carries
`rf_overlaps`, the split, and `mechanisms`: each radiated overlap named by
`tools/air_account.py` (`request`, `answer held`, `answer over a repeat`, `answer over a burst`,
`repeat over an answer`, `both initiating`, `leaving`). `python tools/air_account.py
runs/<scenario>/channel.json` lists them with both transmissions' accounts. `max_collisions`
still counts keyed overlaps. Runs of one
build are not identical (the daemons' threads), so judge a single difference against a second
run, and an A/B on the same seeds; a scenario's `seed = N` and a `-sN` in its name make a
variant.

`--seeds N` runs each scenario under N seeds (its own, then +1000, +2000, named `-s<seed>`) and
`--variant K` only the K-th; the nightly runs variants 0–2 side by side. `--drop-passing-audio`
deletes a passing run's recordings. `tools/scenario_trend.py` compares a night's CSVs with
`bench/baselines/scenarios.csv` — each scenario's median air time and Test rates over its
passing runs, and its pass count — against the *range* of the baseline's own runs, and fails
on a step (15 % and 30 s past the longest baseline run, a rate 15 % under the slowest, a third
fewer passing): one build's three seeds of a scenario can differ by half; `--update` rewrites the baseline when a change is meant to alter the
timing. A failing night opens or comments on the issue labelled `nightly`
(`tools/nightly_report.py`); a passing one closes it.
