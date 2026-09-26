# The desktop application

Two pieces:

* **`ui/`** — the station panel. Plain HTML, CSS and ES modules with no build step
  (ADR-0005). `aetherd` serves this directory at `http://127.0.0.1:8515/`, so a headless
  gateway can be watched from a browser over an SSH tunnel, and the desktop shell shows the
  same files.
* **`src-tauri/`** — the desktop shell. It starts `aetherd`, waits for the control port,
  loads the panel from it, and asks the daemon to stop when the window closes — over the
  control API rather than by killing it, so the transmitter is released whatever the keying
  backend. If a daemon is already running it attaches to that one instead of starting a
  second.

The header carries the lamps (TX, RX, BUSY, LINK), the rules' verdict for the dial the radio is
on (**LEGAL**, **WARNING** or **TX BLOCKED**; click it for the reasoning), a chip while a host or
KISS program uses the station, the dial and the callsign. A banner across every tab says, in
large type, when a session is up, being called or how the last one ended, and another when
Setup has changes that are not saved. The tabs, each a set of titled boxes:

* **Status** — the link (mode, SNR, throughput, the session) and the channel (signal above
  noise, tuning, receive level, queued) as tiles; the history of every frame's speed or SNR
  and of the channel and activity; the counters.
* **Session** — the radio's dial (the rules' chip, the dial list with *Tune*, *Add* and
  *Remove*, and *Dial is at* for a radio that cannot report it); the call (Connect, Disconnect —
  *Stop calling* while a call is being made — Abort, Probe, Beacon with its *Repeat* interval,
  Test session and its progress, the chime); keying and drive (*Key 1 s*, *Tune tone*,
  *Set drive*, the transmit level, the drive guide); the conversation (Send, where a message
  gets its ✓ when the other station has all of it, and Received); the recording and the last
  session (Record, notes, the folder, *Contribute the last test session*).
* **Stations** — everyone heard (beacons, calls, probes, answers, KISS frames and session
  partners, with when, how strong and how many beacons; filters for beacons and stations
  worked), kept by the modem in `heard.json`; and the history of sessions, all of them or one
  station's (`sessions.json`).
* **Diagnostics** — signal analysis (the last frame's constellation, a spectrum and a
  waterfall, undockable into a window of its own), the receiver's readings, the host and KISS
  connections, the rules (the ceiling on the link, the last decision, the dial ranges where each
  waveform fits) and a table of the last frames.
* **Setup** — the profile bar, then one numbered flow: 1 callsign and rules (and the field log's
  grid, rig, power and antenna), 2 radio interface, 3 receive level, 4 modem settings (the
  bandwidth — 2300 Hz, or 500 Hz for peer-to-peer contacts — answer only, the fastest mode,
  compression, the busy channel, the longest key, the Morse identifier, band edges and
  band-plan guidance), 5 application settings (updates, recording, host programs, KISS
  programs), 6 save. Nothing changes until it is saved, and the tab says what is not.
* **Log** — one entry a row with filters (problems, rules, sessions, beacons) and the
  *Copy diagnostic bundle* button: that bundle (`diagnostics` in `docs/spec/control-api.md`
  §4.6) is what to paste into a bug report.
* **Help / About** — the version and its update channel, *Check for Updates*, getting on the
  air, beacons, running a test session, reading the panel, and where to read more.

**Compact**, in the header, shrinks the panel to the state and four readings for a small window
beside a logging program. Every control and indicator carries a tooltip (`title`), and a new one
without one is not done (`CONTRIBUTING.md` §8).

Every reading is the modem's own (`metrics` and `frame` events, and the `spectrum`,
`constellation` and `heard.list` methods of the control API); the panel draws and never
computes. The spectrum and constellation are polled only while the Diagnostics tab is on
screen, so a gateway nobody is watching pays nothing for them.

## Running from a checkout

```bash
cd core && cargo build --release -p aetherd      # the shell looks for this binary
cd ../app/src-tauri && cargo run                 # first run writes a receive-only config
```

The configuration lands in `%APPDATA%\aether-hf\station.toml` on Windows and
`~/.config/aether-hf/station.toml` elsewhere, or wherever `AETHER_CONFIG` points. A first run
gets a placeholder callsign and no keying: the station listens and transmits nothing until
Setup says otherwise.

## Packaging

```bash
python tools/stage_daemon.py            # builds aetherd and copies it to src-tauri/binaries/
cd app/src-tauri && npx @tauri-apps/cli@^2 build
```

The bundler packages the daemon as a *sidecar* (installed beside the shell's binary, which
is where the shell looks first) and the `ui/` directory as a resource (the shell asks Tauri
where resources landed and hands the path to the daemon). On Windows that is a per-user NSIS
installer needing no administrator rights, `Aether HF_<version>_x64-setup.exe`, which
installs to `%LOCALAPPDATA%\Aether HF\` with `aether-hf.exe`, `aetherd.exe`, `ui\` and an
uninstaller; on Linux, `.deb` and AppImage. `.github/workflows/release.yml` does the same
from a tag.

If the daemon will not start — a sound card at the wrong rate, a serial port held by
another program — the shell shows what it said in a message box and leaves the window open;
the daemon's own output for the last run is in `aetherd.log` beside the configuration file.

## Appearance

The panel is dark by design — an instrument for a shack at night — and does not follow the
operating system's theme. Everything is a token at the top of `ui/style.css` (`--bg`,
`--surface`, `--text`, `--accent`, `--status-tx`, `--status-link`, …), so a change of palette
is one edit; the semantic status colours carry meaning and are never used for decoration. The
stylesheet opens with the design system — the type scale, spacing, radii, the 30/24 px control
heights — and the pieces every tab is built from: `.card` with its `.card-title` inside the
box, `.tiles`, `.badge`, `.chip`, the `?` that opens an explanation (`details.help-pop`), the
folding card and the sticky save bar. The constellation, spectrum and waterfall are one card
built by `ui/scopes.js`, used by the Diagnostics tab and by the undocked window
(`signal.html`, `signal.js`). A
light variant exists for whoever asks for it: set `data-theme="light"` on `<html>`. The
window's title bar is asked to be dark in `tauri.conf.json` (`theme`). Numbers are set in
the monospace stack, prose in the system UI face; no fonts are bundled or fetched.

The logo shows for about a second when the panel first opens in a session and then gets out
of the way (`.splash` in the stylesheet, `splash()` in `app.js`). The artwork lives in
`Logos/`; `python tools/make_icons.py` cuts the icon tile out of its black surround and
writes `src-tauri/icons/` (every size the bundler wants, including the `.ico`) and the
panel's `mark.png`, `favicon.png` and `logo.png`.

## Updates

The shell's Help › Check for updates — or *Check for Updates* on the panel's Help / About tab,
which asks the shell to run the same check — opens the shell's own window (`ui/update.html`, `update.js`,
`update.css`; bundled into the binary, so it works while the modem is stopped and the
network is down). It shows the version you have, the version on offer and its release
notes, and one phase at a time — checking, available, downloading with progress,
installing, restart required — or what went wrong and what to do about it. On Windows the
installer relaunches the shell, so "updated to …" is shown by the next start, which finds
the note the install left (`update-note.json` beside the kept installers in
`%LOCALAPPDATA%\aether-hf\`); if the version running is still the old one, the window says
the update did not install rather than nothing. The page talks to the shell through Tauri's
IPC (`withGlobalTauri`, the `updater` window's capability in `src-tauri/capabilities/`),
which the panel's window — served by the daemon — does not have. Opened in a browser
instead, `update.html?demo=<phase>` shows what each phase looks like.
