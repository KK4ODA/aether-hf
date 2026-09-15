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

The panel's Setup tab opens with a five-step wizard — callsign, interface profile, receive
level, keying and drive, save — and keeps the raw form underneath for everything else. Its Log
tab has a "Copy diagnostic bundle" button; that bundle (`diagnostics` in
`docs/spec/control-api.md` §4.5) is what to paste into a bug report.

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
is one edit; the semantic status colours carry meaning and are never used for decoration. A
light variant exists for whoever asks for it: set `data-theme="light"` on `<html>`. The
window's title bar is asked to be dark in `tauri.conf.json` (`theme`). Numbers are set in
the monospace stack, prose in the system UI face; no fonts are bundled or fetched.

The logo shows for about a second when the panel first opens in a session and then gets out
of the way (`.splash` in the stylesheet, `splash()` in `app.js`). The artwork lives in
`Logos/`; `python tools/make_icons.py` cuts the icon tile out of its black surround and
writes `src-tauri/icons/` (every size the bundler wants, including the `.ico`) and the
panel's `mark.png`, `favicon.png` and `logo.png`.
