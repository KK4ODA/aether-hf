# Aether HF operator's manual

This manual covers the desktop application: what each part of the window does, how to make a
contact, what the readings mean, and what to try when something goes wrong. The other guides in
this folder go deeper on one subject each, and this manual links to them rather than repeating
them.

It describes **0.2.0-beta.109**. When the application changes, this manual changes in the same
commit (see *Keeping this manual current* at the end).

## Contents

1. [What Aether HF is](#1-what-aether-hf-is)
2. [Installing and first start](#2-installing-and-first-start)
3. [The window](#3-the-window)
4. [Setup](#4-setup)
5. [Session](#5-session)
6. [Status](#6-status)
7. [Stations](#7-stations)
8. [Diagnostics](#8-diagnostics)
9. [Log, and sending files](#9-log-and-sending-files)
10. [Help / About and updates](#10-help--about-and-updates)
11. [Making a contact, step by step](#11-making-a-contact-step-by-step)
12. [Using Winlink Express, Pat, VarAC and KISS programs](#12-using-winlink-express-pat-varac-and-kiss-programs)
13. [What the readings mean](#13-what-the-readings-mean)
14. [Troubleshooting](#14-troubleshooting)
15. [Glossary](#15-glossary)
16. [Where to read more](#16-where-to-read-more)

---

## 1. What Aether HF is

Aether HF is an open-source data modem for HF amateur radio. It sends error-free data between
two stations over a sound card and a radio. It does the same job as VARA HF: Winlink Express,
Pat and VarAC talk to it the way they talk to VARA, and it puts its own signal on the air.

Two things follow from that:

- **Aether talks only to Aether.** The host programs see something that behaves like VARA, but
  the signal on the air is Aether's own. Both stations need Aether, and both need a version with
  the same link protocol (the panel warns you when they differ).
- **Both stations of a contact use the same bandwidth.** Aether has two: **2300 Hz**, for
  Winlink and the most data, and **500 Hz**, for peer-to-peer contacts and VarAC. A 2300 Hz
  station answers a 500 Hz call at 500 Hz by itself and goes back afterwards.

A session starts slow and robust and climbs. The slowest steps are the **tone floor**, single
tones that get through at −14 dB or lower. Above them are faster tone kinds, then OFDM modes up
to 64-QAM. Each step is a **rung**. The receiving station measures every frame and tells the
sender which rung to use next, so the link settles where the path allows.

The application has two parts. The **daemon** (`aetherd`) is the modem itself. The **panel** is
the window you see. On a desktop the application starts the daemon and opens the panel for you.
A gateway can run the daemon alone with no window (see the [gateway guide](gateway-kit.md)).

## 2. Installing and first start

Download the installer for your system from the
[releases page](https://github.com/KK4ODA/aether-hf/releases) and follow
[Installing Aether HF](install.md).

On the first start the panel opens on **Setup**, because the callsign is still the placeholder
`N0CALL`. Work through the six steps (section 4), press **Save these settings**, and you are
ready to set your drive and make a contact.

Until step 1 says which rules you operate under, Aether transmits nothing, and a banner across
the top says so. That is deliberate: the modem judges every transmission against the rules
before it keys the radio.

## 3. The window

### The header

Across the top is the **display well**, the station at a glance:

- **State**: *Idle*, *Calling*, *Connected* (with the other station's callsign) or *Closing*.
- **Dial**: the radio's frequency, when the radio can report it (CAT, rigctld or FLRig).
- **Rung**: the rung the sender is using and its rate on the air.
- **SNR**: the signal-to-noise ratio of the last frame heard, in a 3 kHz bandwidth.
- **Lamps**: **TX** while the radio is keyed, **RX** while a burst is arriving, **Busy** while
  the channel carries a signal.

To the right:

- **The rules badge** says *Legal*, *Warning* or *TX blocked* for your widest signal where the
  dial is now. Click it for the reasoning.
- **Modem** is quiet when the panel is talking to the daemon and red when it is not.
- **Compact** shrinks the window to the state and four link readings, for a corner of the
  screen. Press it again to come back.
- A chip shows **programs attached** (a host program or KISS programs), and another shows the
  **bandwidth** when the station is running one that is not its own setting.

### Banners

A banner across every tab tells you about something that needs attention:

| Banner | What it means | What to do |
|---|---|---|
| Session banner | Calling, connected, disconnecting, a probe, a Test, a beacon waiting, or how the last session ended | Nothing; it is there so you can see it from any tab |
| Setup needed | The rules, the control or the license class is not set, so nothing transmits | **Open Setup**, step 1 |
| Unsaved changes | Setup has edits the modem is not using yet | **Review in Setup** and save, or **Discard changes** |
| Mismatch | A station called or probed in the other bandwidth, under another of your callsigns, or in another protocol version; or your call went unanswered while the operator was heard under another SSID | Follow its buttons: Setup step 4, Check for Updates, Answer as it too, Call it, or Dismiss |
| Debug mode | Shown once: sessions run by a host program are sent to the project | **Keep it on** or **Turn it off** |
| Fault | The modem cannot hear or transmit (no sound card, a port it cannot open), or it had to start from older settings | Read what it says; usually Setup step 2 |

A **chime** plays when a session comes up and when it ends. Untick **Chime** on the Session tab
to silence it.

### Tooltips and the ? buttons

Every control and reading has a tooltip: hover over it. Every box has a small **?** that opens an
explanation of that box. Press Escape or click elsewhere to close it.

## 4. Setup

Setup is one numbered flow. The index at the top jumps to a step; a step's number turns green
when it is done and amber when it has unsaved changes.

### Profiles

The **Profile** bar at the top keeps every Setup setting, the transmit level and the waterfall
settings under a name: one per radio, per place, or per way of working (for example
"Standalone" and "VarAC", which differ in who keys the radio).

- Pick a profile to switch to it. A `*` after the name means the running settings have changed
  since it was saved; switching asks first.
- **Save** writes the current settings to the profile, **Save as…** keeps them under a new name.
- **Export…** writes an `.aetherprofile` file to back up or carry to another computer;
  **Import…** brings one in. A sound card the other computer does not have is shown as *not on
  this computer* until you choose one.
- **More** has New profile, Rename, Duplicate and Delete.

What belongs to this computer stays out of a profile: the update channel, debug mode, the KISS
port's address, the panel's port, the log file and the recordings folder.

### Step 1: Callsign and rules

- **Callsign**: yours, with an SSID (`KK4ODA-1`) or a suffix (`KK4ODA/P`) if you use one, up to
  nine characters. A host program's own callsign is answered to as well.
- **Rules**: *United States — FCC Part 97*, or *None — I check every transmission myself*.
- **Control**: *Local* (you are at the radio), *Remote* (you are at the radio through a control
  link), or *Automatic* (nobody at the control point: an unattended mailbox or gateway). Aether
  never guesses this.
- **License class**: decides which frequencies are yours.
- **Sideband**: USB, the convention for data. On LSB the signal lands below the dial, mirrored.
- **For the field log**: grid (six characters lets the log work out the distance), rig, power in
  watts, antenna. They go into every recording, so a report needs no follow-up.

What the rules check, and why, is in [FCC regulatory controls](fcc-regulatory-controls.md).
Aether keys nothing the rules refuse, but you remain the control operator.

### Step 2: Radio interface

- **Interface**: picking a known interface fills in the fields below once. The fields are what
  the modem uses, and you can change any of them afterwards.
- **Modem devices**: the sound card's **Capture** (the radio's receive audio) and **Playback**
  (the radio's transmit audio). The transmit level is shown here; it is set on the Session tab.
- **Keying**: how the radio is keyed.
  - A **serial port** keys through its RTS or DTR line, or with the radio's own **CAT command**
    on its CAT port. CAT needs no second port and also reads the dial, which the rules need.
    Choose the radio's CAT dialect and rate; an Icom also needs its CI-V address (IC-7300 94,
    IC-705 A4, IC-9700 A2). Yaesu radios keyed over CAT transmit from the DATA input.
  - **rigctld** (Hamlib) or **FLRig** keys the radio and reads its dial; a host program can tune
    through either at the same time.
  - A **CM108-class** interface (DRA, URI) keys through its codec's GPIO pin, 3 on those boards.
  - **Host program**: VarAC or Winlink Express owns the radio and keys it when Aether says
    PTT ON, as with VARA. Aether then cannot read the dial, so the rules in step 1 are set to
    None and you check every transmission. Keep this in a profile of its own.
  - **None**: a VOX interface such as a SignaLink, or listening only.

A radio's USB connection often shows two ports. On a Yaesu the *Enhanced* port carries CAT and
the *Standard* port the RTS line; an Icom's single port carries both.

Without CAT, rigctld or FLRig the radio cannot report its dial. Tell the modem where it is with
**Dial is at** on the Session tab before transmitting.

### Step 3: Receive level

With the radio on a quiet frequency, adjust the radio's audio output or the sound card's input
until the meter says **Good**. Clipping is never acceptable. Too quiet leaves the signal inside
the sound card's own noise.

### Step 4: Modem settings

- **Bandwidth**: *2300 Hz* for Winlink, Pat and the most data; *500 Hz* for peer-to-peer and
  VarAC. Both stations of a session use the same. A change takes effect once the station is
  idle, with no restart. A host program's `BW500` or `BW2300` moves it while the program is
  attached.
- **Answer only**: take calls and answer probes, but make no call and send no beacon.
- **Fastest mode**: the highest rung the link may climb to. The rules may hold it lower on some
  frequencies.
- **Compress**: off by default. It compresses the whole session, so on a weak path one lost
  frame can spoil the rest. Use it only on a solid link.
- **Busy channel**: *wait for a clear channel before calling*, and how far above the noise floor
  counts as busy (6 dB by default). With it on, a call, probe or beacon also waits while Aether
  hears two other stations in a session. It waits through the pauses between their
  transmissions, at least 4 s after the last one it heard. It stops waiting when the session
  ends. Its own answers never wait: they go in their slot.
- **Turnaround**: how long after your radio stops sending it hears again. 0 for a radio keyed
  by CAT, a serial line or a host program. With VOX (a SignaLink), set its delay time plus
  100 ms. Aether tells the other station, which waits that long before it answers. Up to 1500 ms.
- **Answer gap**: the least time between the end of the other station's transmission and yours.
  Leave it at 0 unless the other station keys by VOX and has not set its own turnaround; then
  500–800 ms gives its radio time to get back to receive. Aether also learns this per station
  when a station keeps calling again after being answered.
- **Longest key**: the watchdog's limit on one transmission, 30 s by default. Aether sizes its
  bursts to fit inside it.
- **Morse ID**: off by default. Aether's frames carry both callsigns; whether that satisfies
  your licence is your call. Under the US rules an automatic identifier goes no faster than
  20 wpm, whatever is set here.
- **Band edges**: the margin kept between your signal and a segment's edge (50 Hz by default),
  for dial error and the radio's transmit chain. **Band-plan guidance** says when a
  transmission the rules allow is outside the voluntary band plan's digital areas.

### Step 5: Application settings

- **Updates**: which releases to be offered (*stable only*, *betas too*, *nightlies too*), and
  whether to check when the application starts.
- **Recording**: record every session from connect to disconnect.
- **Debug mode**: on by default. After each session that Winlink Express, Pat or VarAC runs
  through this station, its recording (with the audio), the logs and your settings (never a
  password) go to the Aether project, so problems on real paths can be found. Sessions you run
  from the panel and Tests are not sent this way.
- **Recording notes**: written into every recording that starts on its own.
- **Host programs**: let Winlink Express, Pat or VarAC use this station on the VARA-compatible
  interface. The command port is 8300 by default and the data port the next number up.
- **KISS programs**: open a KISS-over-TCP port for APRS and packet programs, and for VarAC's
  broadcasts. The default, 127.0.0.1 port 8100, is VARA HF's own, so a program set up for VARA
  needs no change. Keep the address at 127.0.0.1 unless the program runs on another computer of
  yours on a network you trust: anyone who can reach the port can make your station transmit.
  KISS frames go at rung 1 (tone-36) by default, which stations of either bandwidth hear.

### Step 6: Save

**Save these settings** writes them to the modem's configuration. Most take effect at once. A new
sound card, any keying setting, Host programs, compression, the update channel or the callsign
restarts the modem, which takes a few seconds; the panel comes back by itself. **Copy as TOML** copies the settings for a configuration file, and **Discard changes**
puts every field back to what the modem is using.

The folded **Running configuration** and **Mode table** at the bottom show what the modem is
running now, and every rung of the current bandwidth with its rate and the SNR it needs. A greyed
rung is one the modem never asks for: another rung carries as much data from a lower SNR.

## 5. Session

The Session tab is where you operate. It has five boxes.

### Radio dial

- The **dial** as the radio reports it, and a chip with what the rules say about transmitting
  there.
- The **dial list**: remembered frequencies, starting from the
  [frequency plan](frequency-plan.md)'s proposals. **Picking one tunes the radio there** when
  it is keyed over CAT, rigctld or FLRig. The list is held still during a session, a call, a
  probe, a Test or a transmission. **Add** keeps a dial with a name (adding the same dial again
  renames it); **Remove** drops the chosen one. When the radio's own dial matches an entry, the
  list follows it.
- **Dial is at**: shown when the radio cannot report its dial. Type where it is and press
  **Set**; the rules judge every transmission against it until you change it.

When a host program keys the radio, this box says so and the dial list is hidden: choose the
frequency in the program.

### Call

Type the other station's callsign, with its SSID if it uses one, then:

- **Connect**: call it and start a session. Calls go out on the tone floor first.
- **Disconnect**: send what is queued, then close. At the station that was called, it sends
  what that station has queued and asks the caller to close; the caller stops sending and
  closes. It waits a minute at most: past that the session closes anyway. While it waits, the
  banner and the line under the buttons say how much is still queued and how many seconds are
  left. When the session ends, the banner says what did not get through. While calling it reads
  **Stop calling**.
- **Abort**: drop the session at once, sending nothing more. Use it when you cannot wait for
  Disconnect.
- **Probe**: one frame and one answer, no session. It tells you whether the other station hears
  you and how well, both ways.
- **Beacon**: puts your callsign on the air once, on the tone floor, addressed to nobody, so
  anyone listening can report hearing you. The list beside it repeats it every 10, 15, 20, 30 or
  60 minutes while you are at the station. It waits for a clear channel and for any session to
  finish, and stops when the modem restarts. Aether never beacons under automatic control.
- **Test session**: the field test (below). The same button stops it.

The line under the buttons reports the last probe, the beacon timer, or the Test's progress.

#### The Test session

A Test turns a contact into a measurement the project can use. With the other callsign in Call,
press **Test session**. It runs:

1. a probe;
2. a call;
3. a message, sized to how well the probe says you are heard;
4. a file, sized to about two minutes at the message's rate (at least 256 bytes);
5. a **ladder**: a short burst pinned at each rung, starting one rung under the fastest the
   message and file were acknowledged at, until three rungs in a row fail;
6. a disconnect.

The message and file go first, so the speeds the Test reports are the speeds you would get.
The ladder then tries rungs the link would not choose by itself. Each ladder frame is tried
once at its rung; a frame that fails goes again at the speed the other station asks for. After
the last rung the Test disconnects without waiting for the ladder's leftovers.

It takes about five minutes and stops itself at ten. The other station only has to be listening.
Both stations must run the same bandwidth and protocol version. The result is recorded as two
files ending in `_test`: a `.json` report and the `.wav` audio. See
[Field testing](field-test.md) and the [tester's guide](tester-guide.md).

### Keying and drive level

Click the box's heading to open it.

- **Key 1 s**: keys the radio for one second with no audio, to check the PTT.
- **Tuner tone, 10 s**: a steady tone for an antenna tuner. Do not set drive with it.
- **Set drive, 4 bursts**: four six-second bursts of the real waveform, five seconds apart.
- **Transmit level**: the slider, in dB below full scale. **Last transmit peak** is the modem's
  own headroom.

**How to set drive.** Press **Set drive** and watch the radio's ALC meter. Raise the transmit
level until the ALC just begins to move, then back it off until it barely tickles. If the ALC
moves even with the level low, turn down the radio's USB (rear) data input gain and try again.
Keep the last transmit peak around −6 to −12 dB and never red. For more power, turn up the
radio's power control, not the slider.

A tone is a pure sine, and the data waveform's peaks land 6 to 7 dB higher, so drive set on a
tone runs into ALC limiting on real traffic. The waveform's average power is a fraction of its
peak power, so a few watts average on the meter is normal.

All three buttons are refused during a session; Tuner tone and Set drive are also refused while
the channel is busy. The rules judge each one first.

### Send and Received

- **Send**: type and press Enter (Shift+Enter for a new line, or untick **Enter sends**). What you
  sent stays in the box with its time and a mark: **…** on its way, **✓** once the other station
  has all of it, **✗** if the session ended first.
- **Received**: what the other station sent. Tick **Sound** for a note when text arrives.

**Copy** copies the text without the times; **Clear** empties the box and takes nothing back
from the session.

### Recording

**Record** records the radio's audio at 48 kHz, with a `.json` file beside it saying what the
modem made of every frame, until you press it again. Add notes (band, the other station,
distance) in the field beside it. **Open folder** opens the recordings folder. A recording is what
a bug report about the air should carry.

## 6. Status

- **Link**: the mode, SNR, throughput and session (who, and for how long).
- **Channel**: signal above noise, tuning offset, receive level, and bytes still queued.
- **History**: two charts. **Speed** is the session's bytes per second, both ways added
  together. Two dashed lines show the rate on the air of the rung the data went at: **sending
  at** (your data, in the transmit colour) and **receiving at** (the other station's data, in
  the receive colour). Each line shows only while data moves that way. **SNR by frame** is every frame heard, what the other station
  reports hearing you at, and the current mode's threshold. Below them, **Channel and activity**
  is the received level over the last two minutes, with this station's transmissions ticked and
  the busy periods shaded.
- **Counters**: totals since the modem started; **Reset counters** sets them back to zero.

What each reading means is in section 13.

## 7. Stations

- **Stations heard**: every station whose frame carried a callsign (a beacon, a call, a probe, an
  answer, a KISS frame, or the other end of a session), one line per callsign, the 200 most
  recent. Filters: *All*, *Beacons*, *Worked*. Click a heading to sort. **Hz** is the bandwidth
  the station runs, amber when it is not yours. **Call** puts the callsign on the Session tab;
  **Sessions** shows your sessions with it.
- **Sessions**: one line per session, newest first, the 500 most recent: when, how long, who
  called, what crossed each way, the fastest rung each way, how each station heard the other, and
  how it ended. **Ask for files** writes a request to the other station for its side.

Both lists survive a restart. **Forget all** and **Forget sessions** empty them; recordings are
not touched.

## 8. Diagnostics

- **Signal analysis**: the constellation, spectrum and waterfall. **Undock** opens them in a
  window of their own, to keep in view from any tab. A tone frame has no constellation.
- **Receiver**: the rate controller's smoothed SNR and margin, how the other station hears you,
  mode confidence (below 1.3 is a guess), every number behind the busy detector's decision, and
  what host program, KISS programs and audio are attached.
- **Frames**: every frame the receiver found, decoded or not, newest first: kind, callsigns,
  mode, redundancy version, SNR, offset, confidence, whether it decoded, and what a control frame
  said.
- **Rules**: what the rules know about this station and what they decided, with where each
  waveform fits on the current band.

## 9. Log, and sending files

The **Log** is everything the modem and the panel reported since the panel opened, newest last.
Filters: *All*, *Problems*, *Rules*, *Sessions*, *Beacons*. **Clear** empties the panel's copy;
the modem's own log file is kept.

A `keying` line says how long your radio takes to key and how long Aether takes to prepare a
transmission. Aether starts its transmissions that much sooner, so its answers reach the air on
time. The line comes back only when those times change.

A `keying` line that says *on the air about … ms after its slot opened* means one answer went
out late, and says why: a slow render, a slow radio, or the computer falling behind. One now and
then is harmless. Many in a session mean the computer is too busy: close other programs, or send
your files to the project so we can look.

**Send files…** is how anything leaves your station. Choose what to do:

- **Send my files to the Aether project**: one zip of your logs, the diagnostic bundle and the
  sessions you choose (one station, or all; how far back; with or without the audio) goes to the
  project, with a progress bar.
- **Email my files to someone**: writes the same zip and opens an email to attach it to.
- **Ask a station to send me its files**: writes an email with a link that opens the other
  station's Aether HF with the form filled in.
- **Report my last test as a GitHub issue**: opens a pre-filled issue. Drag the Test's `.json`
  into it.

Nothing leaves without your press, and nothing you typed in Send or Received is included. The
zip stays in the `shared` folder beside your settings.

## 10. Help / About and updates

**About** shows the version, the daemon, the update channel and the latest release, and where
this station keeps its configuration, logs and recordings. **Check for Updates** asks the release
channel for a newer version; the updates window shows what it finds and installs it when you say.

The desktop application's **Help** menu has *Check for updates…*, *Restore the previous
version…*, *Open the configuration folder* and *Releases on GitHub*. Restoring an older version
puts back the settings file that version can read, and keeps the newer one beside it. Restore
works on Windows and with the Linux AppImage; on macOS or a `.deb` install, reinstall the older
release from the releases page.

A beta installation follows the beta channel unless you choose otherwise in Setup step 5. When a
release changes the link protocol, the offer is titled **Update required**: stations on different
protocols cannot connect.

The other boxes on this tab are short guides to getting on the air, Test sessions, beacons and
reading the panel.

## 11. Making a contact, step by step

1. **Agree** a frequency, a time and a bandwidth with the other operator, and check you both run
   a version with the same protocol.
2. **Tune** the radio there: pick it from the dial list, or tune by hand. If the radio cannot
   report its dial, set **Dial is at**.
3. **Check the rules badge** says *Legal*. If it says *TX blocked*, click it to see why.
4. **Listen** for a minute. The **Busy** lamp and the Channel chart show whether the frequency is
   in use.
5. Optionally **Probe** the other station to see how the path is both ways.
6. Type the callsign in **Call** and press **Connect**. The banner says *Calling*, then
   *Connected*, and the chime plays.
7. **Type** in Send. Watch for the ✓.
8. **Disconnect** when you are done. The session appears in **Stations → Sessions**.

To be called instead, leave the modem running: it answers a call to your callsign whenever one
comes. With **record every session** on, every one is kept.

## 12. Using Winlink Express, Pat, VarAC and KISS programs

Turn on **Host programs** in Setup step 5, then point the program's VARA HF settings at
`127.0.0.1` and port 8300. The two ways to run the radio:

- **Aether keys the radio** (CAT, rigctld, FLRig, serial, CM108 or VOX): the usual setup for
  Winlink Express and Pat.
- **The host program keys the radio** (Setup step 2, *host program*): VarAC with its RIG tab's PTT
  and frequency control on CAT, as with VARA.

The full setup for each program is in [VarAC, Winlink Express and Pat with Aether](host-programs.md).

The host program sees only its own sessions. A session you start from the panel is not reported
to it, though the program's BUSY reads ON while it runs.

For APRS and packet programs, and VarAC's broadcasts, turn on **KISS programs** and see
[KISS programs](kiss.md).

## 13. What the readings mean

| Reading | Where | What it is |
|---|---|---|
| **SNR** | Header, Link | What this receiver measured on the last frame it found, in a 3 kHz bandwidth. Negative numbers are normal on HF; the tone floor works below −14 dB. On a tone frame it is a lower bound and reads no higher than about 17 dB. |
| **They hear you at** | Diagnostics, SNR chart | What the other station measured on your last burst, sent back in its acknowledgement or control frames. Compare it with your SNR: a big difference is a lopsided path. |
| **Rung / Mode** | Header, Link | The step the sender is using, with its name and rate on the air. Rungs 0 and 1 are the tone floor. |
| **Throughput** | Link, Speed chart | Application bytes both ways over the last 30 seconds, with the session total under it. Data going one way stays below the rate of the rung it went at: acknowledgements, turnarounds and retransmissions take their share. With data going both ways, the total can stand above either line. |
| **Tuning** | Channel | The carrier offset the receiver removed from the last frame. Positive means the other station is above your dial. A few hertz is nothing; a hundred is a dial to check. |
| **Signal above noise** | Channel | The busy detector's reading: the received level over its learned noise floor. |
| **Receive level** | Channel, Setup step 3 | The sound card's input. Aim for *Good*. |
| **Queued** | Channel | Bytes still waiting to be sent. |
| **Mode confidence** | Diagnostics | How clearly the last frame's mode was read. Below 1.3 is a guess. |
| **Busy lamp** | Header | The channel carries a signal: the level over the threshold, a narrowband signal's shape, or a frame that decoded. |

The rungs, their rates and the SNR each needs are in Setup's **Mode table**, and in full in the
[air-interface specification](../spec/air-interface.md#4-modes--the-ladder). Roughly: at
**2300 Hz**, rungs 0–5 are tone kinds from 36 to 228 bit/s, and rungs 6–19 OFDM from about 200
bit/s to 5.5 kbit/s. At **500 Hz**, rungs 0–3 are tone kinds from 36 to 112 bit/s, and rungs
4–14 OFDM up to about 1 kbit/s. At 500 Hz the modem never asks for rung 4: after each frame's
header it carries less than rung 3, from 7 dB more SNR. The Test's ladder still measures it.

## 14. Troubleshooting

**Nothing transmits, and the rules badge says *TX blocked*.** Click the badge. The usual causes:
step 1 is not filled in; the dial is outside your class's data segment, or the signal would cross
a segment edge; the radio cannot report its dial and **Dial is at** is not set; or the station is
under automatic control outside the automatic sub-bands.

**The radio does not key.** Press **Key 1 s** on the Session tab. If the radio's transmit light
stays off, check Setup step 2: the port, the line (RTS, DTR or CAT), and for CAT the dialect, the
rate and an Icom's CI-V address. On a Yaesu, CAT is on the *Enhanced* port and RTS on the
*Standard* port.

**The radio keys but no signal goes out.** Check the playback device in step 2. A Yaesu keyed over
CAT transmits from the DATA input: check the radio's data-input settings.

**The ALC moves even at a low transmit level.** Turn down the radio's USB (rear) data input gain
and run **Set drive** again.

**The other station does not answer my call.**
- Check you both run the same bandwidth. If not, the mismatch banner says so on the other side,
  and a 2300 Hz station calling a station it knows runs 500 Hz moves there by itself.
- Check you both run versions with the same protocol. A newer one shows **Update required**.
- Check the callsign, SSID included. If the operator was heard under another SSID, the banner
  offers to call that one.
- Probe first: it tells you whether either of you hears the other at all.
- After four unanswered tries at 2300 Hz to a station not known to run 2300 Hz, Aether tries at
  500 Hz.

**Disconnect does not close the session at once.** It sends what is still queued first. On a
slow path that can take a while, so it waits a minute at most, then closes anyway. The banner
counts the seconds down. To close now, press **Abort**.

**The other station keys by VOX (a SignaLink) and keeps calling after I answer.** Set the **answer
gap** in step 4 to 500–800 ms. Aether learns a gap for such a station by itself, but setting it
helps the first contact.

**The Busy lamp stays on with nothing on the air.** Look at the Diagnostics tab's busy detector
line, which says which test marked the channel busy. A steady carrier or a spur in your own
receiver can do it. Try with the antenna disconnected: if it is still busy, the signal is in your
station. Use the radio's AGC on FAST or AUTO, or off; SLOW confuses the noise floor.

**Sessions connect but run slowly.** Look at the SNR both ways (SNR and *They hear you at*). A
lopsided path, where the other station hears you much worse than you hear it, slows every
exchange. More power, a better antenna or another band helps more than any setting. If the
Frames table shows many failed frames at good SNR, check the receive level and the drive.

**The log says the sound card's capture skipped some milliseconds.** Your computer or its USB
audio lost a moment of what the radio sent it. A frame across the gap may not decode, and Aether
asks for it again. Once in a session is nothing to act on. If it happens often, close other
programs, plug the radio into a USB port on the computer itself rather than a hub, and turn off
USB power saving. Send your files to the project too: these lines are being studied.

**A host program says nothing is connected, or VARA is missing.** Check **Host programs** is on in
step 5 and the program points at 127.0.0.1 and the right port. With *host program* keying, the
program must be attached before the panel can transmit.

**The panel says the modem is not running (the Modem lamp is red).** The daemon has stopped or is
restarting. Wait a few seconds after a save. If it stays red, restart the application and look at
the log (Help menu, *Open the configuration folder*, then the `logs` folder).

**Something went wrong on the air and you want it looked at.** Log tab → **Send files…** → *Send
my files to the Aether project*, choosing the sessions with that station and ticking audio. Say
what happened in an issue on GitHub or by email.

## 15. Glossary

- **Acknowledgement (ACK)**: which frames arrived, how well, and which rung to use next. Each
  station's goes with the next thing it sends: the caller's ahead of its frames, the called
  station's in its slot.
- **Answer gap**: the least time Aether waits after the other station's transmission before its
  own, for a station that keys by VOX and has not set its turnaround.
- **Bandwidth**: 2300 Hz or 500 Hz. Both stations of a session use the same.
- **Beacon**: your callsign on the air once, addressed to nobody.
- **Burst**: the frames sent in one transmission.
- **Control (local, remote, automatic)**: how the station is operated, under §97.109; it decides
  which rules apply.
- **Daemon (aetherd)**: the modem itself, which the window talks to.
- **Datagram**: one KISS frame sent on the air on its own, outside any session, with no
  acknowledgement.
- **Floor / tone floor**: the slowest, most robust rungs, single tones that get through at very
  low SNR. Calls, probes and beacons go on it.
- **Host program**: Winlink Express, Pat or VarAC, using Aether through the VARA-compatible
  interface.
- **Ladder**: the rungs of a bandwidth, slowest first. Also the step of a Test that tries each
  rung.
- **Link protocol**: the version of the rules both stations follow on the air. Different
  versions cannot connect.
- **Probe**: one frame and one answer that measure the path both ways without a session.
- **Profile**: a saved set of Setup settings under a name.
- **Rung (mode)**: one step of the ladder: a waveform and code rate with its own speed and the
  SNR it needs.
- **Session**: a connection between two stations that carries data with error correction and
  retransmission.
- **Sidecar**: the `.json` file beside a recording, saying what the modem made of every frame.
- **SNR**: signal-to-noise ratio, here always in a 3 kHz bandwidth.
- **Slot**: the time the calling station gives the called one to send in, after each of its own
  transmissions. The called station keys only in its slot; an answer that would start too late
  for it is not sent, so the two stations never key over each other.
- **Test session**: the field test: probe, call, message, file, ladder, recorded.
- **Turnaround**: how long after a radio stops sending it hears again. Each station tells the
  other its own, and the other waits that long before it answers.

## 16. Where to read more

| Subject | Document |
|---|---|
| Installing, verifying downloads, uninstalling | [install.md](install.md) |
| On-air testing, for testers | [tester-guide.md](tester-guide.md) |
| Winlink Express, Pat and VarAC | [host-programs.md](host-programs.md) |
| KISS programs | [kiss.md](kiss.md) |
| The rules Aether checks | [fcc-regulatory-controls.md](fcc-regulatory-controls.md) |
| Where to operate | [frequency-plan.md](frequency-plan.md) |
| Running a gateway | [gateway-kit.md](gateway-kit.md) |
| Field testing and the bench | [field-test.md](field-test.md) |
| The signal on the air | [air-interface.md](../spec/air-interface.md) |
| The host interface | [host-interfaces.md](../spec/host-interfaces.md) |
| The control API | [control-api.md](../spec/control-api.md) |
| Releases | [github.com/KK4ODA/aether-hf/releases](https://github.com/KK4ODA/aether-hf/releases) |
| Bugs and ideas | [github.com/KK4ODA/aether-hf/issues](https://github.com/KK4ODA/aether-hf/issues) |

---

## Keeping this manual current

This manual is part of the application. A change that adds, removes, renames or changes the
behaviour of anything an operator sees or sets (a tab, a box, a button, a setting, a default, a
banner, a reading, a host-program behaviour) updates the matching section here in the same
commit, and the version named at the top moves with each release. The rule is written down in
`CONTRIBUTING.md`, `docs/MAINTAINING.md` and the pull-request checklist.
