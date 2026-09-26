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
first save: callsign and the rules you operate under, radio interface and devices, receive
level, save. Aether judges every transmission against those rules before it keys the radio, and
transmits nothing until you have said which rules apply, how the station is controlled (local,
remote or automatic) and your license class — an update from an earlier version asks for them
too, in a banner on every tab (`docs/user/fcc-regulatory-controls.md`). The configuration lives at
`%APPDATA%\aether-hf\station.toml`; the daemon's output for this run is `aetherd.log`
beside it and the run before it is `aetherd.prev.log`, and *Help > Open the configuration
folder* takes you there. If restarting the modem made a problem go away, the log worth
reading is `aetherd.prev.log` — the one from the run that misbehaved.

**A change in Setup takes effect when you save it.** Until then the modem runs on what it had,
and the panel says so: the Setup tab carries an amber dot, the step you changed is marked, the
save bar at the foot of the tab names what is not saved, and every other tab shows a banner with
*Review in Setup* and *Discard changes*. Most settings apply the moment they are saved; a sound
card, a serial port or the callsign restarts the modem, which takes a few seconds.

**Keying, and the dial.** Setup step 2 keys the radio through a serial line (RTS or DTR), the
radio's own *CAT command*, a CM108-class interface's GPIO pin, or `rigctld`. Its *Interface*
list fills the fields in for a known interface (an Icom with USB audio, a Yaesu, a
SignaLink, a DRA or URI board) once, when you pick it; the fields are what the modem uses, and
changing one by hand shows *Manual*. Only CAT and
`rigctld` also read the dial, which the rules are judged at; with any other keying you say
where the dial is on the Session tab, and again after every change of frequency. An **Icom
with USB audio** (IC-7300, IC-7610, IC-9700, IC-705) needs nothing but its USB cable: choose
the rig's COM port, *CAT command* and *Icom CI-V*, and in the rig's menu (*MENU › SET ›
Connectors › CI-V*) set *CI-V USB Port* to *Unlink from [REMOTE]* and *CI-V USB Baud Rate* to
the rate in Aether (19200 unless you already use another, such as 38400); the CI-V address is the rig's (IC-7300 94, IC-7610 98, IC-9700
A2, IC-705 A4). Only one program can hold the port: close a logger or rig-control program
that has it, or share the radio through `rigctld`. When CAT works, the dial appears in the
header; when it does not, a banner names the port and what the radio answered.

**Closing the window stops the modem.** When that would interrupt something — a session,
a Test session, a call, a probe, a transmission — the application asks first and names
it; *Keep running* leaves everything as it was. If you close anyway, the modem ends a
session on the air before it stops (a disconnect, and your callsign in Morse when it is
set to identify), which can take a quarter of a minute after the window has gone. A
daemon the application did not start (a gateway service) runs on, and closing asks
nothing.

**Uninstalling** (*Settings › Apps*, or *Uninstall Aether HF* from the Start menu) removes
the program. What you made is yours, and it stays unless you say otherwise; the
uninstaller asks, one kind at a time, and pressing Enter keeps it:

| Asked | What it is | Where |
|---|---|---|
| Settings and caches | `station.toml` and its backups, the dial memories; the installers kept for going back; the window's stored data (the text sent, the waterfall's controls) | `%APPDATA%\aether-hf\`, `%LOCALAPPDATA%\aether-hf\`, `%LOCALAPPDATA%\org.aetherhf.desktop\` |
| Profiles, logs and history | the profiles, `aetherd.log` and the one before it, the stations heard, the session history | `%APPDATA%\aether-hf\` |
| Recordings | the audio and sidecars of your recorded and Test sessions — asked with a warning of its own, and only after the other two | `%APPDATA%\aether-hf\recordings\` |

A recordings folder you moved elsewhere (`[record] dir`) is never touched, and neither is
anything you put in these folders yourself. An update, and a silent or passive uninstall,
keep everything without asking. The uninstaller's own *Delete the application data* box
removes only the window's stored data.

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
Which releases it offers is a setting — **Setup → Updates**: *stable releases only*, *betas
too*, or *nightlies too*. Unset, it follows the installation: a beta follows the betas and a
stable release the stable releases. (Beta 56 moved a beta installation still set to stable
to *betas too*: every release so far has been a beta, and the panel had written *stable*
into nearly every configuration whether or not anybody chose it. Choosing stable again is
kept; the updates window then says there is no stable release yet.) A stable installation
is never offered a beta or a nightly. *Help > Check for updates…*, or *Check for Updates* on
the panel's Help / About tab, asks now, on whatever the setting says at that moment.

Every version the application installs is kept on the machine (`%LOCALAPPDATA%\aether-hf\rollback\`
on Windows, `~/.local/state/aether-hf/rollback/` on Linux and macOS), so if an update does not work
for you, *Help > Restore the previous version…* goes back without a network. When a
version changes the shape of the configuration file, it backs up the old file beside itself
first (`station.toml.bak-v4`, named after the shape it was in). An older version cannot read
a newer shape, so going back across such a change puts that copy back: from beta 57 a restore
does it before installing, keeping your settings as they are now as
`station.toml.newer-v5`, and a daemon that finds a newer file starts from the newest copy it
can read and says so on the panel. Otherwise your settings are not touched by an update or a
restore.

### Profiles

Everything on the Setup tab, the transmit level and the waterfall's controls are saved
under a **profile**, chosen at the top of Setup. Your settings became the profile
*Default* the first time this version started; **Save as…** keeps the current settings
under another name (a second radio, the truck, a portable setup) and switching profiles
changes the station to it — a sound card, port or callsign change restarts the modem, as
a save does. A `*` after the name means the running settings have changed since the
profile was last saved; **Save** writes them to it, and switching with unsaved changes
asks first.

A profile is a file — `profiles\<name>.aetherprofile` beside the configuration, JSON —
so **Export…** saves a copy to keep or carry, and **Import…** on another computer brings
it in. What is this computer's own (the panel's port, the log file, the recordings
folder) is never in the file; a device the other computer does not have is shown as
*not on this computer* in the device lists, with the port that has the same interface
behind it named when there is one, and the modem stays receive-only until you choose.
A profile written by an older version loads with defaults for what it does not have; one
written by a newer version says so rather than loading half of itself. **More** holds the
rest: a new profile from the defaults, rename, duplicate, delete.

---

## 2. The daemon on its own

For a gateway, or a station run from a terminal. `aetherd-<version>-<target>.tar.gz` (Linux
x86_64 and aarch64 — a Raspberry Pi 4 or 5 on Raspberry Pi OS 12 or later — and macOS on
Apple Silicon) or `.zip` (Windows) holds the binary, the station panel it serves, the systemd unit and the gateway
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
