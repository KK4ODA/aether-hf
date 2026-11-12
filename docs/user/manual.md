# Aether HF operator's manual

This manual covers the desktop application: what each part of the window does, how to make a
contact, what the readings mean, and what to try when something goes wrong. The other guides in
this folder go deeper on one subject each, and this manual links to them rather than repeating
them.

It describes **0.2.0-beta.150**. When the application changes, this manual changes in the same
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
to 64-QAM. Each step is a **rung**. Each station chooses the rung for its own data: it keeps
count of how much data each rung has actually delivered for its airtime, from what the other
station acknowledges and how it says it hears you, and now and then tries the next rung up with
one frame. So the link settles on the rung that moves the most data on this path.

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

- **State**: *Idle*, *Calling*, *Connected* or *Closing*, with the other station's callsign in
  front. In a narrow window the words shorten; the callsign does not.
- **Dial**: the radio's frequency, when the radio can report it (CAT, rigctld or FLRig).
- **Rung**: the rung the data is moving at and its rate on the air: the other station's while
  you receive (↓), yours while you send (↑), both when data goes both ways. With no data moving,
  the rung your station would send at.
- **SNR**: the signal-to-noise ratio of the last frame heard, in a 3 kHz bandwidth.
- **Lamps**: **TX** while the radio is keyed, **RX** while a burst is arriving, **Busy** while
  the channel carries a signal.

To the right:

- **The rules badge** says *Legal*, *Warning* or *TX blocked* for your widest signal where the
  dial is now. Click it for the reasoning.
- **Modem** is quiet when the panel is talking to the daemon and red when it is not.
- **Your callsign**, the one the station is using now: the session's while one is up; while a
  program answers for the station, the program's (section 12); otherwise Setup step 1's.
- **Full**, **Compact** and **Mini** choose how much of the panel to show. **Full** is every
  tab. **Compact** shrinks the window to the state, the four link readings and the Status
  tab's two plots, small and side by side: the speed over the last ten minutes (or the SNR,
  if that is the one you chose in Full) and the channel's activity (what the radio hears, and
  when you transmitted), drawn with the time span and level scale you set with **Axes** in Full.
  Compact shows no **Axes** button, but the chart keeps its two level sliders and **Auto**; the
  time span is changed in Full. The plots take whatever height the window has left: when a
  banner comes up during a session, or the header needs a second row, they get shorter for as
  long as it shows, and they never go below the window's edge. Make the window taller for
  taller plots; it keeps that size while you stay in Compact.
  **Mini** is the state and the four readings alone, for a corner of the screen. **Full**
  brings the window back to its size. Maximizing the window in Compact or Mini switches it
  to Full; restored, it goes back to Full's size. Aether remembers the view, and the size of
  the window in Full, for the next start.
- **Light** turns the panel black on white, for a screen in daylight or sunlight. The lit
  lamps are filled with their colour so they show in glare. Press it again for the dark
  panel. The undocked signal window and the updates window follow, and Aether remembers the
  choice for the next start. The waterfall keeps its own colours.
- A chip shows the **programs using the station**: a host program, a program on the control
  API that answers for callsigns of its own (a RadioPost gateway, for example), and KISS
  programs. A program that listens for calls shows *listening*. A program shown by its name
  alone can call out, but calls to its callsigns go unanswered until it listens: that is normal
  for a program that only calls, such as the RadioPost client. A program that scans, as RMS
  Trimode does, stays *listening* through its short gaps. The chip's tooltip names the callsigns
  each program uses. When no program is there and calls to the station's own callsign are not
  answered, the chip says *calls to* the callsign *not answered*, and its tooltip says why:
  under automatic control only a program answers calls (section 12).
  Another chip shows the **bandwidth** when the station is running one that is not its own
  setting.

### Banners

A banner across every tab tells you about something that needs attention:

| Banner | What it means | What to do |
|---|---|---|
| Session banner | Calling, connected, disconnecting, a probe, a Test, a beacon waiting, or how the last session ended. A program's call or session says *for* the program | Nothing; it is there so you can see it from any tab |
| Setup needed | The rules, the control or the license class is not set, so nothing transmits | **Open Setup**, step 1 |
| Unsaved changes | Setup has edits the modem is not using yet | **Review in Setup** and save, or **Discard changes** |
| Mismatch | A station called or probed in the other bandwidth, under another of your callsigns, or in another protocol version; or your call went unanswered while the operator was heard under another SSID; or a station called a callsign a program answered for, and that program is not running | Follow its buttons: Setup step 4, Check for Updates, Answer as it too, Call it, or Dismiss. For a program that is not running, start it again |
| Receive filter | The radio's receive filter cuts the top, the bottom or both edges of Aether's signal, 12 dB or more down, and names the knob: the IF shift, the high or low cut, or the filter width | Turn that knob; the banner goes when the cut does. **Dismiss** hides it until the reading changes: another edge, 3 dB further down, or another bandwidth. Both edges cut at 2300 Hz also offers **Setup step 4**, to run at 500 Hz |
| Clipping | More than 0.1 % of the audio heard over the last ten seconds reached full scale | Turn the radio's audio output or the sound card's input down. **Dismiss** hides it until the audio stops clipping and starts again |
| Debug mode | Shown once: sessions run by a program (Winlink Express, Pat, VarAC, RadioPost) are sent to the project | **Keep it on** or **Turn it off** |
| Fault | The modem cannot hear or transmit (no sound card, a port it cannot open), or it had to start from older settings | Read what it says; usually Setup step 2 |

The receive filter and clipping banners show in Compact too. Aether tells a host program
nothing of either.

A **chime** plays when a session comes up and when it ends. Untick **Chime** on the Session tab
to silence it.

### Tooltips

Every control, reading and box title has a tooltip: hover over it. A box's title explains the
whole box; a field's tooltip says what the field does, its range and its default. The window shows
only names, values and units.

## 4. Setup

Setup is one numbered flow. The index at the top jumps to a step; a step's number turns green
when it is done and amber when it has unsaved changes.

### Profiles

The **Profile** bar at the top keeps every Setup setting and the transmit level under a name: one per radio, per place, or per way of working (for example
"Standalone" and "VarAC", which differ in who keys the radio).

- Pick a profile to switch to it. A `*` after the name means the running settings have changed
  since it was saved; switching asks first.
- **Save** writes the current settings to the profile, **Save as…** keeps them under a new name.
  Changes on the Setup tab that you have not saved yet are saved to the modem first, so the
  profile gets them too.
- **Export…** writes an `.aetherprofile` file to back up or carry to another computer;
  **Import…** brings one in. A sound card the other computer does not have is shown as *not on
  this computer* until you choose one.
- **More** has New profile, Rename, Duplicate and Delete.

What belongs to this computer stays out of a profile: the update channel, debug mode, the KISS
port's address, the panel's port, the log file, the recordings folder and the waterfall's
settings.

### Step 1: Callsign and rules

- **Callsign**: yours, with an SSID (`KK4ODA-1`) or a suffix (`KK4ODA/P`) if you use one, up to
  nine characters. What you start from this window (a call, a probe, a Test, a beacon) goes out
  as it. While a program answers for the station, the box shows whose callsign is in use
  instead, greyed:
  - **Host**, with **Host programs** on (step 5). The program's own callsign (its `MYCALL`) is
    the one the modem answers to and identifies as while the program is attached. Turn Host
    programs off to change the callsign here.
  - **The program's callsign and its name**, such as *KK4ODA-1 · RadioPost Gateway*, while a
    RadioPost program holds a callsign. The gateway holds its callsign all the time; the
    RadioPost app on a mobile holds its callsign only while it calls or listens. Stop the
    program to change the callsign here.

  Either way the callsign set here is kept, and comes back when the program leaves. Set a
  program's callsign in the program itself, not here (section 12).
- **Rules**: *United States — FCC Part 97*, or *None — I check every transmission myself*.
  With **Keying** set to *Host* (step 2), Rules, Control, License class and Sideband read
  **Host**: the host program keys the radio and owns its dial, which Aether cannot read, so the
  rules are None and you check every transmission. Choose another keying in step 2 to set them.
- **Control**: *Local* (you are at the radio), *Remote* (you are at the radio through a control
  link), or *Automatic* (nobody at the control point: an unattended mailbox or gateway). Aether
  never guesses this. Under *Automatic*, Aether answers calls only for a program that holds the
  callsign called and is listening, such as RMS Trimode or a RadioPost gateway (section 12).
- **License class**: decides which frequencies are yours.
- **Sideband**: USB, the convention for data. On LSB the signal lands below the dial, mirrored.
- **Field log**: grid (six characters lets the log work out the distance), rig, power in
  watts, antenna. They go into every recording, so a report needs no follow-up. Each may be left
  empty.

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
  - **Host**: VarAC or Winlink Express owns the radio and keys it when Aether says PTT ON, as
    with VARA. Aether then cannot read the dial, so the rules in step 1 are set to None and you
    check every transmission; their fields read **Host**. Keep this in a profile of its own.
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
  idle, with no restart. With **Host programs** on (step 5) the box reads **Host**: the
  program's `BW500` or `BW2300` sets it while the program is attached, and the setting here comes
  back when it leaves. Set the bandwidth in the program.
- **Answer only**: take calls and answer probes, but make no call and send no beacon.
- **Fastest mode**: the highest rung the link may climb to. The last entry is the fastest your
  bandwidth has, and stays the fastest when you change bandwidth. Choose a lower one only to cap
  the link. The rules may hold it lower on some frequencies.
  - Before 0.2.0-beta.123, saving Setup while running 500 Hz could leave a 2300 Hz station at
    rung 14 of 19. If yours shows 14 at 2300 Hz and you did not choose it, choose the last entry
    and save.
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
- **Morse ID**: your callsign in Morse at the end of each session. It is the last thing your
  station sends for the session, after the other station's goodbye. Nothing during a session
  sends one, and neither do calls, probes, beacons or KISS frames.
  - It sends your callsign without its SSID: `KK4ODA-1` goes out as `KK4ODA`, and a `-T` or
    `-R` is dropped too. The rules ask for your assigned call sign (§97.119), and the SSID is
    not part of it. A `/` part such as `W4/G4ABC` or `/P` is kept.
  - A US callsign always sends it. The box is ticked and greyed out; its tooltip says why
    (§97.119). What you had set is kept for any other callsign.
  - Any other callsign: your choice, off by default. A host program's `CWID ON` turns it on for
    the program's sessions.
  - A call or probe that leads to no session is identified 30 s after, on its own.
  - The US rules cap an automatic identifier at 20 wpm, whatever is set here (§97.119(b)(1)).
    The speed is marked when the cap applies, and its tooltip says so. 20 wpm can sound slow: a callsign is
    long in Morse.
  - A US station identifies only at the end. A session longer than ten minutes is not identified
    in between: see *FCC regulatory controls*.
- **Band edges**: the margin kept between your signal and a segment's edge (50 Hz by default),
  for dial error and the radio's transmit chain. **Band-plan guidance** says when a
  transmission the rules allow is outside the voluntary band plan's digital areas.

### Step 5: Application settings

- **Updates**: which releases to be offered (*stable only*, *betas too*, *nightlies too*), and
  whether to check when the application starts.
- **Recording**: record every session from connect to disconnect, and on until the closing
  Morse IDs have gone (yours, and the other station's when it identifies after you), at most
  30 s after the end.
- **Debug mode**: on by default. After each session that Winlink Express, Pat, VarAC or
  RadioPost runs through this station, its recording (with the audio), the logs and your settings (never a
  password) go to the Aether project, so problems on real paths can be found. Sessions you run
  from the panel and Tests are not sent this way.
- **Recording notes**: written into every recording that starts on its own.
- **Host programs**: let Winlink Express, Pat or VarAC use this station on the VARA-compatible
  interface. The command port is 8300 by default and the data port the next number up. The
  RadioPost client and gateway do not use this port: they connect to the control port. Never
  give it 8515, or 8514, whose data port is 8515: that is Aether's control port, which this
  panel and RadioPost use. Setup refuses a port that would share a number with another of the
  station's (the control port, the host programs' two, the KISS port), and so does Aether at
  start when the file was edited by hand.
- **KISS programs**: open a KISS-over-TCP port for APRS and packet programs, and for VarAC's
  broadcasts. The default, 127.0.0.1 port 8100, is VARA HF's own, so a program set up for VARA
  needs no change. Keep the address at 127.0.0.1 unless the program runs on another computer of
  yours on a network you trust: anyone who can reach the port can make your station transmit.
  KISS frames go at rung 1 (tone-36) by default, which stations of either bandwidth hear.

### Step 6: Save

**Save** writes them to the modem's configuration. Most take effect at once. A new
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
  there. Over CAT, rigctld or FLRig it is read once a second while the station is idle, so a
  knob turned at the radio shows within a second or so. During a session it is read less often.
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
  banner and the line under the buttons say how much is left to send and the seconds left, for
  example *Disconnecting from WC4Y: sending the last 1.2 kB (45 s at most). Abort closes now.*
  When the session ends, the banner says what did not get through. While calling it reads
  **Stop calling**. Each station that identifies sends its Morse ID last: the other station's
  right after its goodbye, then yours.
- **Abort**: drop the session at once. Aether sends no more data: it only tells the other
  station it has gone, then sends your Morse ID if you identify. Use it when you cannot wait for
  Disconnect.
- **Probe**: one frame and one answer, no session. It tells you whether the other station hears
  you and how well, both ways. If the first probe gets no answer, Aether sends a second; the line
  under the buttons says *Probing KK4XYZ… 1/2*, then *2/2*. If neither is answered it says
  *KK4XYZ: no answer to 2 probes.* A gateway run by RMS Trimode ignores a probe for half a second every few
  seconds, so one probe can be missed when the other station hears you fine.
- **Beacon**: puts your callsign on the air once, on the tone floor, addressed to nobody, so
  anyone listening can report hearing you. The list beside it repeats it every 10, 15, 20, 30 or
  60 minutes while you are at the station. It waits for a clear channel and for any session to
  finish, including the other station's Morse identifier at its end, and stops when the modem
  restarts. Aether never beacons under automatic control.
- **Test session**: the field test (below). The same button stops it.

The line under the buttons reports the last probe, the beacon timer, or the Test's progress.

#### The Test session

A Test turns a contact into a measurement the project can use. With the other callsign in Call,
press **Test session**. It runs:

1. a probe. If you probed the same station on the same dial in the last minute and were
   answered, that probe counts and no new one is sent;
2. a call. If the probe found the other station at 500 Hz and you run 2300 Hz, the call and
   the whole Test go at 500 Hz, and your station goes back after;
3. a message, sized to how well the probe says you are heard;
4. a file, sized to about two minutes at the message's rate (at least 256 bytes);
5. a **ladder**: a short measurement at each rung the link uses, starting one rung under the
   fastest the message and file were acknowledged at, until three rungs in a row fail;
6. a disconnect.

The message and file go first, so the speeds the Test reports are the speeds you would get.
The ladder then tries rungs the link would not choose by itself. It skips the rungs the link
never uses: 8 at 500 Hz, 6 and 13 at 2300 Hz. Each rung sends a few frames of test filler
once. The other station says which arrived, and nothing is sent again. A rung whose answer was
lost is logged with no SNR (*at ? dB*). After the last rung the Test disconnects.

It takes about five minutes and stops itself at ten. The other station only has to be listening.
If you wait for a clear channel, the probe and the call wait too, and the wait does not count
against them. It counts against the ten minutes: a channel that never clears ends the Test.
The two stations may run different bandwidths: the Test then runs at 500 Hz, as a call
would. They must run versions with the same protocol. The result is recorded as two
files ending in `_test`: a `.json` report and the `.wav` audio. See
[Field testing](field-test.md) and the [tester's guide](tester-guide.md).

### Keying and drive level

Click the box's heading to open it.

- **Key 1 s**: keys the radio for one second with no audio, to check the PTT. The log then
  says whether the radio's receive audio changed while it was keyed. A radio that transmits goes
  quiet (or plays back its own signal). If it reads the same as the band, the radio did not key.
- **Tuner tone**: a steady tone for ten seconds, for an antenna tuner. Do not set drive with it.
- **Set drive**: four six-second bursts of the real waveform, five seconds apart.
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

### Record the radio's audio

**Start recording** saves what the radio hears to a file, at 48 kHz, until you press **Stop and
save**. It works at any time, just listening or during a session, and needs no other station. A
`.json` file beside the audio says what the modem made of it: every frame, every change of the
busy indicator, what the busy detector had learned when the recording started, and its readings
four times a second. Type notes first, in the field beside the button: band, frequency, radio settings,
the other station. **Open folder** opens the recordings folder. A recording is what a bug report
about the air should carry: Log tab → **Send files…** sends it.

**Mark** writes the moment you press it into the recording's `.json` file, numbered 1, 2, 3 and
so on. Press it each time you change something on purpose, such as swapping VFOs, instead of
timing it with a stopwatch. The line beside the box title counts the marks.

With the radio keyed over CAT, rigctld or FLRig, a recording also keeps what the radio says about
itself: its S-meter, four times a second, and its dial whenever it changes. On a Yaesu over CAT it
also keeps the AGC (including what AUTO picked), the mode, the filter width, the RF gain, the
attenuator and the preamp whenever they change. The radio is asked only while nothing else is
going on: never during a session, a call, a probe, a Test or a transmission. The line beside the
box title says how many S-meter readings the recording holds. A serial line, a CM108 codec or a
host program keying the radio gives none of these.

## 6. Status

- **Link**: the mode, SNR, throughput and session (who, and for how long).
- **Channel**: signal above noise, tuning offset, receive level, bytes still queued, and the
  **Receive filter** line: what the radio's receive filter does to the edges of Aether's signal.
- **History**: two charts. **Speed** is the session's bytes per second, both ways added
  together, drawn only during a session. Two dashed lines show the rate on the air of the rung the data went at: **sending
  at** (your data, in the transmit colour) and **receiving at** (the other station's data, in
  the receive colour). Each line shows only while data moves that way. **SNR by frame** is every frame heard, what the other station
  reports hearing you at, and the current mode's threshold. Below them, **Channel and activity**
  is the received level, a bar every half second, and the busy periods shaded along the top.
  Each bar is the loudest level the busy detector measured in its half second. A bar that
  reaches the dotted line, the busy threshold, marked the channel busy by its level. Your own
  transmissions (**TX**) are short red ticks along the bottom edge, with no bar above them:
  the receiver is muted while you transmit. The dashed line is the busy detector's noise floor. Frames the modem
  finds never count as noise, and the quiet after them does, so in a session the floor stays
  the band's own and does not climb to the other station's signal, however long the session
  runs. After the modem starts, and after the dial moves 10 kHz or more, the detector listens a
  second before it learns anything: a sound card can start quieter than it goes on, and a radio
  settles after a retune. With *wait for a clear channel* on, a call or probe made then waits
  until the detector has listened, about 3½ seconds.

  **Axes**, at the right of the chart's title, sets how the chart is drawn:

  - **Time**: 30 s, 1 min, 2 min (the default), 5 min or 10 min. The title says the span.
    At the longer spans, and in a narrow window, each bar is the loudest of the half seconds
    it covers.
  - **Level**: with **Auto** ticked, the scale fits the noise floor and the signals in view,
    and **Top** and **Bottom** show what it chose. Clear **Auto** to set them yourself, in
    dBFS: Top from −130 to 0, Bottom from −140 to −10, at least 10 dB apart. Type a value and
    press Enter, or use the arrow keys (1 dB a press, 10 with Shift). A level above Top is
    drawn to the top edge. With a fixed scale the title shows its range.
  - **Reset** goes back to 2 minutes and Auto.

  Aether keeps these for the next start, on this computer. Compact draws the chart the same
  way. Escape or a click elsewhere closes **Axes**.

  **Two sliders** at the right of the chart set the same level scale while you watch: the left
  one is **Top**, the right one **Bottom**. Both run from 0 dBFS at the top to −140 at the
  bottom, so Top's cap always sits above Bottom's. Drag a cap, or click a slider and use the
  arrow keys (1 dB a press, 10 with Shift; Page Up and Page Down 10). The chart redraws as the
  cap moves, and the slider shows its level beside it. Moving a slider clears **Auto** and
  starts from the scale Auto had. Pushing one bound within 10 dB of the other moves the other
  along. While **Auto** is on, the caps are hollow and follow the automatic scale. **Auto**,
  under the sliders, turns it back on. The sliders and **Axes** are the same setting, kept for
  the next start, and Compact shows the sliders too.
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
  - The undocked window has its own **Full**, **Compact** and **Mini** at the right of its
    title bar. **Full** is the whole card. **Compact** is the spectrum and the waterfall.
    **Mini** is the waterfall alone, with a frequency scale under it, for a corner of the screen
    while a digital program runs. The waterfall's controls are in Full and hold in every view.
    Each view keeps its own window size: size it once and it comes back that way. The window
    reopens in the view you left it in. **Keep on top** works in every view; Compact and Mini
    call it **On top**. In Compact and Mini
    the **Modem** lamp shows only while the modem is out of reach, and Mini's frequency scale
    follows the **Span**. Resizing the window or
    changing the view keeps what the waterfall has drawn.
  - Every view of the undocked window has a **Busy** lamp, the same as the panel's, and **Tune**,
    the Session tab's *Tuner tone*: a steady tone for ten seconds for an antenna tuner. Press it
    again (**Stop**) to end the tone early. If the modem refuses the tone (a session, a busy
    channel, the rules), the reason shows under the title bar for a few seconds.
  - The red lines on the spectrum and the waterfall are the edges of your widest transmission at
    the bandwidth you run, by the FCC's 26 dB measure. They sit a little outside the bandwidth's
    own number: the signal's skirts reach past it. A signal between them is on your frequency;
    wait for it to clear before you call. The lines move when the bandwidth changes, and narrow
    if *Fastest mode* keeps the station on the tone rungs.
  - **Span** is the audio the spectrum and the waterfall show. **0–3 kHz**, the default, holds
    any SSB filter's audio whole, with your signal centred. **0–4 kHz** also shows a wide data
    filter and its skirt. **Channel** zooms on your own channel: the red lines with 200 Hz either
    side, about 900–2100 Hz at 500 Hz (at 2300 Hz it is 0–3 kHz). The spectrum's title says the
    span. Changing it keeps what the waterfall has drawn, moved to the new scale.
  - The waterfall draws the noise dark and steady and a signal bright above it. **Floor** sets
    the level drawn black; with **auto** it follows the noise in the modem's passband. **Gain**
    is how many decibels above the floor reach full white, 3 to 90 dB (20 by default): lower it
    for more contrast. **Speed** is lines a second; **Palette** the colours.
  - These settings are kept for this computer and survive a restart. A profile does not carry
    them.
- **Receiver**: how the other station hears you, smoothed, which is what your station chooses its
  speed by, and how many frames of your last transmission the other station never heard at all;
  how the other station hears you on its last report,
  mode confidence (below 1.3 is a guess), every number behind the busy detector's decision (with
  *radio gain*, how far the radio's AGC holds its gain down when it does), and what host program,
  KISS programs and audio are attached.
- **Frames**: every frame the receiver found, decoded or not, newest first: kind, callsigns,
  mode, redundancy version, SNR, offset, confidence, whether it decoded, and what a control frame
  said.
- **Rules**: what the rules know about this station and what they decided, with where each
  waveform fits on the current band.

## 9. Log, and sending files

The **Log** is everything the modem and the panel reported since the panel opened, newest last,
one line an entry: hover over a line for the whole of it. Filters: *All*, *Problems*, *Rules*, *Sessions*, *Beacons*. **Clear** empties the panel's copy;
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

The zip also carries the logs of the programs that use Aether, when they offer them: RadioPost
and RadioPost Gateway do (`radiopost.log`, `gateway.log`). They go in the zip's `programs`
folder, with the copy before each, whenever they were written in the period you chose. Aether
remembers a program's log after the program has closed, for 30 days. It takes only files named
`.log`, and leaves out one over 8 MB. The zip's `README.txt` names them. Debug mode's zips carry
them too.

## 10. Help / About and updates

**About** shows the version, the daemon, the update channel and the latest release, and where
this station keeps its configuration, logs and recordings. **Check for Updates** asks the release
channel for a newer version; the updates window shows what it finds and installs it when you say.
Its buttons, **Install and restart** among them, stay at the bottom of the window at any size; the
release notes scroll above them.
On Windows the installer waits for the application to close, closes it if it must (the modem
stops first), and moves aside a file something else still holds
([install.md](install.md#updating) says how).

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
- **The host program keys the radio** (Setup step 2, *Host*): VarAC with its RIG tab's PTT
  and frequency control on CAT, as with VARA.

The full setup for each program is in [VarAC, Winlink Express and Pat with Aether](host-programs.md).
A Winlink gateway — RMS Trimode at the gateway, Winlink Express in the field — is set up step
by step in [Winlink through an Aether gateway](winlink-gateway.md).

While **Host programs** is on, the program sets the callsign and the bandwidth: Setup shows
**Host** in their places. The program's `MYCALL` names the callsigns it answers for. While it is
attached, Aether answers no other callsign of its own: the program speaks for the station. What
the program sets lasts while it is attached; when it leaves, Aether goes back to its own callsign
and bandwidth. A program's `CWID ON` turns the Morse ID on
for its sessions. The first transmission after a program attaches is checked on its own: if the
receive audio does not change while the radio is keyed, the Log says the radio may not be
transmitting, so a program set to key a port nothing is wired to is found at once.

The host program sees only its own sessions. A session you start from the panel is not reported
to it, though the program's BUSY reads ON while it runs. Unless the program keys the radio, it is
not told PTT for that session either: RMS Trimode stops sending commands while it thinks the
radio is keyed.

One program at a time uses the host port. If a program connects again, for example after it
restarted its modem, the new connection replaces the old one. The Log tab says so (*host:* lines),
and says when a program attaches or leaves.

**A program speaks for the station.** This holds on either port: a host program (Trimode,
Winlink Express, VarAC) and a RadioPost program (RadioPost Gateway, or the RadioPost app) alike.

- While a program holds a callsign, only the programs' callsigns are answered. Setup step 1's
  callsign is not answered meanwhile, and comes back when the last program leaves.
- Setup step 1's box shows whose callsign is in use: **Host**, or the program's callsign and
  name.
- Set a program's callsign in the program: RadioPost Gateway's Setup, the RadioPost app's
  enrollment, the host program's own settings. Do not copy it into Aether.
- Setup step 1's callsign is the station's own. What you start from this window goes out as
  it, and it is what is answered when no program is running.
- If they differ (Setup KK4ODA, the gateway KK4ODA-1), a call to KK4ODA is not answered while
  the gateway runs, even under local control. If they are the same, nothing changes.
- When a program has left, a station calling its callsign brings up the mismatch banner, which
  names the program to start again. Aether never offers to answer that callsign itself.

**Two programs on one station.** A program on the host port and a program on the control API
(a RadioPost gateway beside RMS Trimode, for example) can share one radio. Each answers for its
own callsigns: give them different ones, such as KK4ODA-1 for Trimode and KK4ODA-10 for
RadioPost. RadioPost connects to the control port, 8515, not to the host port, and needs Aether
to key the radio itself (Setup step 2): keyed by the host program, the station transmits only
through that program. A call is answered only while the program holding its callsign is listening, and the
session is that program's alone: the other program is not told of it and sees the channel busy
until it ends. The station still runs one session at a time. Two programs cannot hold the same
callsign: the first to claim it keeps it, and the Log says which program was refused it
(*claims:* lines).

**Under automatic control only a program answers calls.** With the rules set to *Automatic*
(Setup step 1), nobody is at the station to carry a session of its own, so Aether answers a call
only for a program that holds the callsign called and is listening: RMS Trimode, a RadioPost
gateway. While no program holds the station's callsign, calls to it go unanswered; the header's
chip says *calls to* the callsign *not answered*, and the Log says each caller left unanswered
(*listen:* lines). Probes are still answered: an answer to a probe opens no session. Under local
or remote control the station answers its own callsign as before.

**A program that leaves takes its session with it.** When the program that owns a session or a
call disconnects from Aether (it closed, crashed, or lost its connection), Aether ends that
session as Disconnect does: what is already queued may still go, then the closing exchange and
the identifiers. A call it placed stops. The Log says so (*claims:* lines), and when the
connection broke rather than being closed by the program, it says how in brackets: *RadioPost
client has gone (a read failed: …)*. A connection that held no callsign and broke gets a
*control:* line of its own.

RMS Trimode starts its modem itself and, to restart it, closes what it started. When Trimode
starts Aether, Aether runs a small helper with no window for Trimode to watch, so a restart no
longer closes Aether's window. Winlink Express also closes the program it started: turn off its
automatic launch and start Aether yourself.

For APRS and packet programs, and VarAC's broadcasts, turn on **KISS programs** and see
[KISS programs](kiss.md).

## 13. What the readings mean

| Reading | Where | What it is |
|---|---|---|
| **SNR** | Header, Link | What this receiver measured on the last frame it found, in a 3 kHz bandwidth. Negative numbers are normal on HF; the tone floor works below −14 dB. On a tone frame it is a lower bound and reads no higher than about 17 dB. On a fast flutter (a polar or auroral path, a signal that warbles) it is what the frame could carry, which stays a few decibels under the path however strong it is: the channel changes within each frame. |
| **They hear you at** | Diagnostics, SNR chart | What the other station measured on your last burst, sent back in its acknowledgement or control frames. Compare it with your SNR: a big difference is a lopsided path. |
| **Rung / Mode** | Header, Link | The step the data is moving at, with its name and rate on the air: the other station's while you receive, yours while you send, both (*sending / receiving*) when data goes both ways. With no data moving, the rung your station would send at. The low rungs are the tone floor. |
| **Throughput** | Link, Speed chart | Application bytes both ways over the last 30 seconds, with the session total under it. Data going one way stays below the rate of the rung it went at: acknowledgements, turnarounds and retransmissions take their share. With data going both ways, the total can stand above either line. |
| **Tuning** | Channel | The carrier offset of the last frame, from your dial. Positive means the other station is above your dial. A few hertz is nothing; a hundred is a dial to check. In a session the receiver follows a station that is off frequency, so a rig that drifts as it warms stays heard; the log says so, and the receiver goes back to your dial when the session ends. |
| **Signal above noise** | Channel | The busy detector's reading in Aether's channel (500 or 2300 Hz wide): the level of its latest 25 ms of audio over its learned noise floor. The floor is the noise's middle level, so noise alone reads a few decibels either side of 0 dB, and a negative value is noise under the floor. Frames the modem finds, decoded or not, never count as noise. The floor follows the radio's gain: when its AGC turns everything down for a strong signal, the floor goes down with it and back up with it. |
| **Receive level** | Channel, Setup step 3 | The sound card's whole input over the last 3 seconds: the radio's whole passband, every signal in it included. Aim for *Good*. It measures more than Aether's channel, so it reads higher than the floor under *Signal above noise*: on noise alone a 500 Hz channel holds about 7 dB less than a 2.4 kHz passband. |
| **Queued** | Channel | Bytes still waiting to be sent or acknowledged, counted as you or your program gave them, before compression. |
| **Receive filter** | Channel | What the radio's receive filter does to the edges of Aether's signal (360–2640 Hz at 2300 Hz, 1260–1740 Hz at 500 Hz), read from the band's noise while the station listens: how far each edge lies under the passband's own level. *Passes the signal* with both edges within 12 dB; *Cuts the top*, *Cuts the bottom* or *Cuts both edges* past it, and a banner names the knob. A cut of 10 dB at an edge costs nothing; 20 dB costs speed. It reads in seconds, even on a busy band: signals on the band are left out of what it reads, and so are Aether's own frames. *Can't check, band noise too weak* means the band's noise sits too close to the sound card's own floor to show a cut: the radio's audio is turned right down, or an AGC holds it down. It needs twelve looks, about twelve seconds of listening after the modem starts. |
| **Clipping** | Receive level, banner | The share of the audio heard over the last ten seconds at full scale. Over 0.1 % the audio clips, and the Receive level line says so. |
| **Mode confidence** | Diagnostics | How clearly the last frame's mode was read. Below 1.3 is a guess. |
| **Busy lamp** | Header | The channel carries a signal: the level over the threshold, a narrowband signal's shape, or a frame that decoded. |

The rungs, their rates and the SNR each needs are in Setup's **Mode table**, and in full in the
[air-interface specification](../spec/air-interface.md#4-modes--the-ladder). Roughly: at
**2300 Hz**, rungs 0–5 are tone kinds from 36 to 228 bit/s, and rungs 6–19 OFDM from about 200
bit/s to 5.5 kbit/s. At **500 Hz**, rungs 0–4 are tone kinds from 36 to 139 bit/s, and rungs
5–14 OFDM up to about 1 kbit/s. Some rungs are never asked for, because another carries as
much or more from less SNR: 8 at 500 Hz, 6 and 13 at 2300 Hz. The Test's ladder skips them too.
Rung 4 at 500 Hz was an OFDM rung before 0.2.0-beta.117; the link protocol changed with it, so
both stations need a version with the new protocol (see **Update required** above). It changed
again in 0.2.0-beta.118: a frame the path will not carry at any slower rung is sent in smaller
pieces, so a transfer no longer stalls on it. Stations before and after beta.118 cannot
connect.

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

**Two 2300 Hz stations keep meeting at 500 Hz.** One of them remembers the other as a 500 Hz
station, from an earlier session or a time it really ran 500 Hz. The Stations tab's *Hz* column
shows what each station remembers. Probe the other station once: the answer says the bandwidth it
really runs, and the next call goes at 2300 Hz. An answer to a call does not count, because it
always goes in the call's bandwidth. A call from a host program goes in the bandwidth the program
asked for, whatever Aether remembers; the Log tab says so when the two differ.

**Disconnect does not close the session at once.** It sends what is still queued first. On a
slow path that can take a while, so it waits a minute at most, then closes anyway. The banner
counts the seconds down. To close now, press **Abort**.

**The other station keys by VOX (a SignaLink) and keeps calling after I answer.** Set the **answer
gap** in step 4 to 500–800 ms. Aether learns a gap for such a station by itself, but setting it
helps the first contact.

**The Busy lamp stays on with nothing on the air.** Look at the Diagnostics tab's busy detector
line, which says which test marked the channel busy. A steady carrier or a spur in your own
receiver can do it. Try with the antenna disconnected: if it is still busy, the signal is in your
station.

**The Busy lamp stayed on for about a minute after a strong signal went.** Up to 0.2.0-beta.114
the floor learned the noise the radio's AGC had turned down, and the true noise read busy until
the floor caught up (ADR-0078). Now the floor follows the radio's gain. While the AGC holds it
down, the Diagnostics tab's busy line says *radio gain −20 dB* (or however far). At 500 Hz the
floor comes back up as the gain does. At 2300 Hz the channel fills the radio's passband, so the
floor waits until the gain is nearly back: a second with AUTO, a few with SLOW. Any AGC setting
works. In USB mode an FTDX10's AUTO picks SLOW; DATA-U (AUTO picks MID) suits data better. Two
setups leave the gain unreadable, and the floor then behaves as it did before: a radio filter
narrower than about 1 kHz around a 500 Hz channel, and a band of strong signals filling the
radio's passband beside the channel, such as a busy FT8 segment.

**The Busy lamp stayed on for about a minute after you changed something at the radio.** Turning
the RF gain up, switching the attenuator off or a preamp on, raising the radio's USB audio level or moving
to a noisier band makes all the noise louder at once. Aether reads the louder noise as a signal until
its floor learns the new level, about a minute. Wait it out, or make such changes before you start
listening. The other way round (the noise gets quieter) costs nothing. A move of the dial that
Aether knows of costs nothing either: when it tunes the radio from the dial list, or reads the dial
over CAT, rigctld or FLRig, a move of 10 kHz or more starts the busy detector over on the new band.

**The receive filter banner says the top of the signal is cut.** The radio's passband ends
below the top of Aether's signal. Usually the IF shift (or passband tuning) has moved the passband
down: centre it. Otherwise the high cut is set low, or the filter is narrower than about
2.4 kHz for a 2300 Hz signal: open it. The reading follows the knob within about a minute.

**The receive filter banner says the bottom of the signal is cut.** The IF shift has moved the
passband up, or the low cut is set high: centre the shift, or set the low cut to 300 Hz or below.

**The receive filter banner says both edges are cut.** The filter is narrower than the signal:
a CW or narrow data filter under the 2300 Hz signal. Widen it to 2.4–3 kHz, or run the modem at
500 Hz (Setup step 4), which fits a filter 600 Hz or wider centred on 1500 Hz.

**The Receive filter line says *Can't check*.** The band's noise is too close to the sound card's
own noise for a cut to show. Turn the radio's audio output up, or the sound card's input (Setup
step 3), or wait for the band: at a quiet moment with the AGC holding the gain down there is no
noise to read. Nothing is wrong with the filter for saying so.

**The clipping banner shows.** The radio sends the sound card more than it can take. Turn the
radio's USB or data audio output down, or the sound card's input level in the computer's sound
settings, until **Receive level** reads *Good* (Setup step 3). Clipped audio spreads the strong
signals across the band and costs every frame.

**Sessions connect but run slowly.** Look at the SNR both ways (SNR and *They hear you at*). A
lopsided path, where the other station hears you much worse than you hear it, slows every
exchange. More power, a better antenna or another band helps more than any setting. If the
Frames table shows many failed frames at good SNR, check the receive level and the drive.

**The log says the sound card's capture skipped some milliseconds.** Your computer or its USB
audio lost a moment of what the radio sent it. Aether puts silence back in its place, so
everything after the gap stays in time. A frame across the gap may still not decode, and Aether
asks for it again. Once in a session is nothing to act on. If it happens often, close other
programs, plug the radio into a USB port on the computer itself rather than a hub, and turn off
USB power saving. Send your files to the project too: these lines are being studied.

**The log says the radio's receive audio did not change while it was keyed.** Aether keyed the
radio, but what the radio sent the sound card stayed the same as the band, so the radio probably
did not transmit. The other station hears nothing, and a session can look connected on one side
only. Check the keying in Setup step 2, and that the radio's own setting for it matches (on many
radios, RTS or DTR keying works only on one of its two USB serial ports, or after a menu setting).
Then press **Key 1 s** and watch the radio and the log. Aether says this once after three
transmissions in a row, and again every ten minutes while it goes on. The first transmission after
a host program attaches is judged on its own, so a program keying the wrong port is caught at its
first try.

**The log says the sound card's playback skipped some milliseconds inside a transmission.** Your
computer or its USB audio stalled while Aether was sending. What went on the air had a short
hole, and the other station may not decode the frame across it; it asks again. The cures are the
ones for capture skips above.

**A host program says nothing is connected, or VARA is missing.** Check **Host programs** is on in
step 5 and the program points at 127.0.0.1 and the right port. With *Host* keying, the
program must be attached before the panel can transmit.

**The panel says the modem is not running (the Modem lamp is red).** The daemon has stopped or is
restarting. Wait a few seconds after a save. If it stays red, restart the application and look at
the log (Help menu, *Open the configuration folder*, then the `logs` folder).

**Something went wrong on the air and you want it looked at.** Log tab → **Send files…** → *Send
my files to the Aether project*, choosing the sessions with that station and ticking audio. Say
what happened in an issue on GitHub or by email.

## 15. Glossary

- **Acknowledgement (ACK)**: which frames arrived, how well the other station hears you, and how
  many frames of your last transmission it never heard at all. Each station's goes with the next
  thing it sends: the caller's ahead of its frames, the called station's in its slot.
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
| A Winlink gateway with RMS Trimode, and its client | [winlink-gateway.md](winlink-gateway.md) |
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
