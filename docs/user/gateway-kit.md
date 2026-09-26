# Running an Aether gateway

A gateway is a station that is always on, always listening, and run by somebody who is not
sitting in front of it. `COMMUNITY-CONCERNS.md` §1 is that a new mode lives or dies by how
many of these exist — "it doesn't matter how well it works, what matters is how many active,
24/7, well-run gateways there are" — so this document is a deliverable and not an appendix.

It assumes Linux. A gateway on Windows works, but the machine will reboot itself for updates
and the mode needs stations that do not.

---

## 1. What you need

| | |
|---|---|
| Machine | Anything from a Raspberry Pi 4 upwards. The modem uses well under one core at 8 kHz. |
| Sound card | A radio interface: Digirig, SignaLink, a rig with USB audio built in. |
| Keying | A serial control line (most interfaces), the radio's own CAT commands on its CAT port (`[ptt] kind = "cat"`: Yaesu, Kenwood/Elecraft, Icom CI-V — which also records the dial frequency), the GPIO pin of a CM108-class interface (`[ptt] kind = "cm108"`: DRA, URI, RA-40 and the like — no serial port; on Linux give the daemon's user the device with a udev rule such as `SUBSYSTEM=="hidraw", ATTRS{idVendor}=="0d8c", MODE="0660", GROUP="plugdev"` in `/etc/udev/rules.d/70-aetherd.rules`, and the same line with `0c76` for an SSS-based board), or `rigctld` if you already run Hamlib. |
| Radio | Anything that will pass 2.3 kHz of audio and key from an external interface. |

Two things that are not obvious and cost people days:

* **The transmit audio level matters more than the power.** An overdriven sound card turns a
  clean OFDM signal into splatter, and splatter is what gets a mode a reputation. Start at
  the default `tx_level = 0.25`, watch the rig's ALC, and reduce until ALC barely moves — on
  real bursts, not the tune tone: the waveform peaks 6–7 dB above a tone of the same level.
  The panel's *Set drive* (Session tab → Keying and drive, or `drive.set` on the control
  API) sends such bursts for exactly this.
* **Disable every audio "enhancement" the operating system offers.** Automatic gain control,
  noise suppression and echo cancellation all destroy a data signal, and on a fresh install
  at least one of them is usually on.

---

## 2. Build

Release builds only. The acquisition search is about twenty times slower unoptimised, which
on a Pi is the difference between working and not.

```bash
sudo apt-get install -y build-essential pkg-config libasound2-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
git clone https://github.com/KK4ODA/aether-hf
cd aether-hf/core
cargo build --release -p aetherd
```

The binary is `target/release/aetherd`. It has no runtime dependencies beyond ALSA.

### Cross-compiling for a Pi

Building on the Pi itself works and takes about twenty minutes. If you would rather not:

```bash
rustup target add aarch64-unknown-linux-gnu
sudo apt-get install -y gcc-aarch64-linux-gnu
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
  cargo build --release -p aetherd --target aarch64-unknown-linux-gnu
```

ALSA's headers have to be the target's, not the host's. The simplest way to get that right is
to build in a container with the target's root filesystem, or to build on the Pi.

---

## 3. Configure

```bash
aetherd --example-config > station.toml
aetherd --list-devices    # find the exact device names
aetherd --list-ports      # and the serial port, if you key that way
```

Edit `station.toml`: the callsign, the two device names, the keying, and the rules the
station runs under. Then, **before connecting an antenna**:

```bash
aetherd --config station.toml --dry-run
```

A dry run keys nothing whatever the file says, so a wrong serial port is discovered without
putting a carrier on the air.

A minimal gateway configuration:

```toml
schema_version = 7
callsign = "W4ODA"

[audio]
input  = "USB Audio CODEC"
output = "USB Audio CODEC"
tx_level = 0.25

[ptt]
kind = "serial"
port = "/dev/ttyUSB0"
line = "rts"

[radio]
max_key_s = 30.0
wait_for_clear = true

[host]
enabled = true            # so Winlink software can use it
bind = "127.0.0.1:8300"

[regulatory]
profile = "us-fcc-part97"
control = "automatic"     # nobody at the radio (§97.109, §97.221)
license_class = "general"
sideband = "usb"
log_permitted = true      # log the basis of every transmission, not only the refusals

[record]
auto = true               # every session as a WAV and a sidecar
```

**Nothing is transmitted until `[regulatory]` names the rules, the control and the licence
class** (ADR-0018, [fcc-regulatory-controls.md](fcc-regulatory-controls.md)). Under
**automatic** control the daemon transmits only inside the automatic-control sub-bands of
§97.221(b), or on 6 m where data is allowed — every Aether signal measures more than the
500 Hz that §97.221(c) allows elsewhere — and near a sub-band's edge it holds the link to the
rungs that fit. It refuses a repeating beacon (§97.203(d)). A radio keyed over a serial line
or a CM108 interface cannot report its dial, so give it `dial_hz = …`; over CAT or `rigctld`
the dial is read before every transmission. `[radio] answer_only = true` makes the station
take calls and make none, and send no beacon, probe or datagram.

The daemon keeps the station's settings as a **profile** too: on its first start it writes
`profiles/Default.aetherprofile` beside the configuration (the portable settings as JSON —
no paths, no token, no sockets) and `profiles.json` naming it. A profile exported from a
desktop station can be copied into `profiles/` and loaded over the control API
(`profile.list`, `profile.load`; `docs/spec/control-api.md` §4.9) or through the panel over
an SSH tunnel; the gateway's own `[control]`, `[log] file` and `[record] dir` are kept
whatever the file says.

---

## 4. Install as a service

`deploy/aetherd.service` is a systemd unit with the installation commands in its header. The
parts that matter:

* It runs as its own unprivileged user in the `audio` and `dialout` groups. A gateway does not
  need root and must not have it.
* `TimeoutStopSec=15s` with `SIGTERM`: the daemon catches the signal and **releases the
  transmitter** before exiting. A station killed mid-burst leaves the radio keyed until
  somebody notices, which on an unattended gateway can be a very long time.
* `Nice=-5`: audio is soft real-time. Without it the modem is descheduled mid-burst on a busy
  machine and drops captured samples, which looks exactly like a bad band.

```bash
sudo systemctl enable --now aetherd
journalctl -u aetherd -f
```

---

## 5. Checking on it

The control API is on loopback, so from the gateway itself:

```bash
curl -s -X POST http://127.0.0.1:8515/v1/status -d '{}'
```

A gateway should record what it does — `[record] auto = true` writes every session, from
connect to disconnect, as a WAV and a sidecar under `recordings/` beside the configuration
(`docs/user/field-test.md`); a day of sessions is a few hundred megabytes, so give it a disk
or a cron job.

Every log line has a UTC timestamp and the modem's state; `[log] format = "json"` in the
configuration makes each line one JSON object, which `journalctl -o cat | jq` can filter by
`event` or `level`. When something goes wrong, one request collects what a bug report needs
— version, platform, settings with the token taken out, the state of the link, the devices
the machine reports, and the last few hundred log lines, and none of your traffic:

```bash
curl -s -X POST http://127.0.0.1:8515/v1/diagnostics -d '{}' > aether-diagnostics.json
```

To watch from elsewhere, forward the port over SSH rather than opening it:

```bash
ssh -N -L 8515:127.0.0.1:8515 gateway-host
```

Binding the control interface to a network address requires a token, and the daemon refuses
to start without one — that interface can key a transmitter, and an open one is not a
configuration to start and warn about.

---

## 6. Winlink software

The host interface (`docs/spec/host-interfaces.md`) speaks the published VARA TCP protocol, so
Pat, Winlink Express, VarAC and BPQ32 can use the station without being modified.

**On the bench, not yet on the air.** Pat 1.0.0 and Winlink Express 1.8.5.0 each complete a
peer-to-peer B2F session with an attachment over two daemons joined by the simulated channel,
and two VarAC copies ping and connect at 500 Hz; BPQ32 and RMS Trimode have not been run.
§7 of that document is the verification table and it is honest about what has and has not
been run. If you try one, please report what happened — that table is the compatibility
claim, and it should reflect what people have actually done.

Programs that use VARA's **KISS** port — APRS clients, VarAC's broadcasts, packet programs —
use `[kiss]` instead ([kiss.md](kiss.md)): off unless enabled, loopback, and each frame goes
on the air as a datagram outside any session, judged by the same rules.

### Pat

```
# ~/.config/pat/config.json
"varahf": { "host": "localhost", "cmdPort": 8300, "dataPort": 8301, "bandwidth": "2300" }
```

### RMS Trimode and BPQ32

Both expect a VARA modem on the same two ports. Point them at 8300 and they should find it.

**Do not list an Aether gateway as a VARA gateway.** A real-VARA client that called it would
find something it cannot talk to, and would blame VARA. The compatibility is in the host
interface, not on the air: an Aether station cannot decode a VARA signal and never will.

---

## 7. Being a good neighbour

* Leave `wait_for_clear = true`. The busy detector learns the noise floor — the typical
  level of the last minute, taken only from moments when nothing was on the channel — and
  refuses to start a session while the level sits `busy_threshold_db` (6 dB) over it for
  most of half a second, or while a frame is being decoded. A static crash does not count;
  a signal that stays does, for as long as it stays. On a stormy night 7 dB is a fair
  setting. A station already in session answers regardless, because the peer is waiting
  and silence would only make it retransmit.
* Pick a frequency with `docs/user/frequency-plan.md` in front of you. Sitting on a VARA
  calling frequency is the fastest way to make the mode unwelcome.
* Leave `max_key_s` at 30 seconds unless you have a reason. It bounds a stuck key, and a
  stuck key on an unattended station is the worst thing this software can do.
* If your licence requires identification in a particular form, turn on `cw_id`. Aether's
  frames carry both callsigns, but whether that satisfies your licence conditions is your
  call, not the modem's. With it on, every session's end is identified once, and a long
  session at least every `cw_id_interval_s` (ten minutes); under the US rules the identifier
  goes at 20 wpm at most (§97.119(b)(1)) whatever `cw_id_wpm` asks for, and the log says so.
* Leave a gateway **unattended only where the rules allow it** — the daemon enforces the
  sub-bands, but whether your station may run under automatic control at all, and on which
  bands, is yours to know.
