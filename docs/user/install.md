# Installing Aether HF

Everything is on the [releases page](https://github.com/KK4ODA/aether-hf/releases). Each
release carries the desktop application for Windows, Linux and macOS, the daemon on its own
for gateways, a `SHA256SUMS` file, and a software bill of materials.

---

## 1. The desktop application

One thing to install and one thing to double-click. It contains the modem daemon
(`aetherd`), the station panel, and a window around them.

### Windows

Download `aether-hf_<version>_x64-setup.exe` and run it. It installs for the current user
under `%LOCALAPPDATA%\Aether HF\` and needs no administrator rights.

**About the SmartScreen warning.** The installer is not yet signed with a Windows code-signing
certificate, so Windows will say "Windows protected your PC" the first time. Click *More
info*, then *Run anyway*. What you *can* check: the `SHA256SUMS` file on the release page
lists the installer's hash —

```powershell
Get-FileHash .\aether-hf_<version>_x64-setup.exe
```

— and updates installed by the application itself are verified against an Ed25519 signature
that is built into it, so a download host cannot hand it something else. Certificate signing
through an open-source signing service is on the roadmap (`docs/ROADMAP.md` §9).

First run writes a configuration with a placeholder callsign and no keying, so nothing
transmits until you have been through **Setup** — which is where the panel opens until the
first save: callsign, radio interface and devices, receive level, save. The configuration lives at
`%APPDATA%\aether-hf\station.toml`; the daemon's output for the last run is `aetherd.log`
beside it, and *Help > Open the configuration folder* takes you there.

### Linux

A Debian package, `aether-hf_<version>_amd64.deb`, for Debian 12 / Ubuntu 22.04 and later:

```bash
sudo apt install ./aether-hf_<version>_amd64.deb
```

or an AppImage, `aether-hf_<version>_amd64.AppImage`, which runs from wherever you put it
(`chmod +x` it first). Both need ALSA; the package declares it. The configuration is
`~/.config/aether-hf/station.toml`.

### macOS

`aether-hf_<version>_aarch64.dmg` runs on Apple Silicon (any Mac from 2020 on, macOS 11 or
later). Open it and drag *Aether HF* into *Applications*.

**It is not signed.** Signing and notarizing a Mac application needs an Apple Developer
account, which this project does not have yet, so macOS will refuse the first launch
("cannot be opened because the developer cannot be verified", or on newer systems
"is damaged"). Clear the download flag once, in Terminal:

```bash
xattr -dr com.apple.quarantine "/Applications/Aether HF.app"
```

and open it normally after that. As on Windows, `SHA256SUMS` on the release page lets you
check the download (`shasum -a 256 aether-hf_<version>_aarch64.dmg`). The configuration is
`~/.config/aether-hf/station.toml`; the modem's audio goes through Core Audio, keying
through a serial line, CAT or a CM108-class interface's GPIO pin as on the other systems. Nobody has run this build on a Mac
with a radio yet — a report, good or bad, is worth an issue.

### Updating

The application looks for a newer version when it starts and asks before installing one.
Which releases it offers is a setting — **Setup → Updates**: *stable releases only* (the
default), *betas too*, or *nightlies too*. A stable installation is never offered a beta or
a nightly. *Help > Check for updates…* asks now.

Every version the application installs is kept on the machine (`%LOCALAPPDATA%\aether-hf\rollback\`
on Windows, `~/.local/state/aether-hf/rollback/` on Linux and macOS), so if an update does not work
for you, *Help > Restore the previous version…* goes back without a network. Your settings
are never touched by an update or a restore; if a version changes the shape of the
configuration file, the old file is backed up beside itself first (`station.toml.bak-v1`).

---

## 2. The daemon on its own

For a gateway, or a station run from a terminal. `aetherd-<version>-<target>.tar.gz` (Linux
x86_64 and aarch64 — a Raspberry Pi 4 or 5 on Raspberry Pi OS 12 or later) or `.zip`
(Windows) holds the binary, the station panel it serves, the systemd unit and the gateway
documentation. `docs/user/gateway-kit.md` is the rest of the story.

```bash
tar xzf aetherd-<version>-aarch64-unknown-linux-gnu.tar.gz
cd aetherd-<version>-aarch64-unknown-linux-gnu
./aetherd --example-config > station.toml     # then edit it
./aetherd --config station.toml
```

---

## 3. Checking a download

```bash
sha256sum -c SHA256SUMS --ignore-missing
```

The `.spdx.json` file on each release lists every component the release was built from and
its licence.

---

## 4. Building from source

`app/README.md` for the desktop application, `docs/user/gateway-kit.md` §2 for the daemon.
