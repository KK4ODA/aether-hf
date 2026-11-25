# ADR-0137: The sound card comes back, and the same codec in another USB port is found

**Status:** accepted, 2026-11-25 (the author's report from the portable's laptop, beta.177). The
daemon, the panel and the VARA adapter: **no wire change, no configuration key, no schema bump.**

## 1. Context

The author plugged the portable's DigiRig (a CM108-class USB codec) into another USB-C port.
Windows made it a new audio endpoint — `Speakers (2- USB Audio CODEC)` where Setup named
`Speakers (USB Audio CODEC)` — with Windows' own defaults for a new endpoint, and Aether did not
use it. It took resetting the device and a restart of the laptop. Every soundcard modem has this
problem; the author asked whether Aether could be more robust than the others.

What Aether did before this ADR:

| Case | Before |
|---|---|
| Configured device missing at start | `SoundCard::open` failed once. The daemon ran on `Silence` with `status.audio_fault`, the panel's fault banner, and a host program attaching heard `MISSING SOUNDCARD`. The card was never tried again: only a Setup save (which restarts the modem) or a restart opened it. |
| Device at the wrong rate (Windows' default for a new endpoint, often 44.1 kHz) | The same, with the error naming the rate. Fixing the rate in Windows was not enough: the modem had to be restarted too. |
| Device unplugged mid-run | cpal's error callback printed to standard error and nothing else. The capture queue went empty: the station's clock stopped, the panel froze on its last readings, and a keyed radio was released only by `AUDIO_STALL_RELEASE` (2 s). `audio_fault` stayed null, so the panel and a host program said nothing. |
| Device plugged back in, same port | Nothing: the dead streams were never reopened. Restart needed. |
| Same device in another port, renamed `2- …` | At start, as *missing*; Setup showed the old name *not on this computer* with no suggestion (the profile's suggestion by description existed for serial ports, not audio). |
| Transmitting with no sound card | Nothing refused it: the panel's Tune, Beacon or Connect keyed the radio with the audio going to `Silence`. |

## 2. Decision

### 2.1 The same device under another instance number is the same device

`card::resolve` opens a configured name that is present as it is. One that is absent is matched
to the device whose name is the same but for Windows' instance prefix (`N- `, at the start of
the name or just inside its last parentheses: `card::instance_of`), **only when exactly one
device matches**. Never another model (the name without the number must be identical), never a
guess between two (two DigiRigs plugged in leave the station waiting, saying so). The capture and
playback sides of one codec (`card::device_part`, the parenthesised device name) must resolve to
the same instance, so a station never listens to one radio while transmitting through another of
the same model. The system default (`None`) is never resolved.

The device found is used **for the run** and not written to `station.toml`: the panel shows a
banner, *Sound card in another USB port: X in place of Y*, with **Keep**, which saves the new
names in Setup step 2 (`config.set`, then the restart a sound card needs). The log says it once
with the Windows settings to check. A profile applied on a machine where its codec has another
number gets the found name as its `missing_hardware` suggestion (as a serial port by
description did already), offered, never taken.

### 2.2 A card that fails is tried again

`card::ResilientCard` wraps the sound card. It is the card while the card is open; otherwise it
is `Silence` on the clock, as before, so the station runs on, the panel live. A card fails when
its stream reports `DeviceNotAvailable` (`SoundCard::failure`; cpal maps WASAPI's
`AUDCLNT_E_DEVICE_INVALIDATED` and ALSA's `ENODEV` to it) or delivers nothing for `card::STALL`
(3 s: Windows does not always report a vanished endpoint). It is opened again, by the rule of
§2.1, every 2 s for the first minute and every 10 s after (`card::retry_interval`): a cable pushed
back in, the same codec in another port, or a rate fixed in Windows' Sound settings all come back
without a restart. The playback clock and the loss counters run on across a reopen (the closed
card's are carried), so the station's account of them never steps back.

### 2.3 Nothing is keyed without a sound card

While there is no card, `Station::set_audio_down` holds the station: what is on the air is let go
at once (`abandon_tx`, not 2 s later), what waits is dropped, and the keying point in
`Station::playback` refuses to key (an `error:audio` event). The control API refuses what would
transmit (`connect`, `beacon`, `probe`, `test.start`, `ptt.test`, `datagram.send`, a non-zero
`tune`, `drive.set`, `beacon.every`) with `refused` and `NO_SOUND_CARD`, the same list
ADR-0025's host-keyed refusal uses (`methods::transmits`). A session under way is not ended by
the modem: the engine's own timeouts end it, as for a keying failure. Coming back, the station
keys again only on a device the operator chose or §2.1 identified; the keyed-audio watch, the PTT
and the regulatory gate are untouched and judge the next transmission as any other.

### 2.4 What is said

- `status.audio_card` (`fault`, `description`, `input`, `output`, `moved`, `tries`) beside
  `status.audio_fault`, and an `audio` event when either changes.
- The panel: the fault banner says *Sound card unavailable — <why>*, its tooltip what to do
  (plug it back in; Windows' format 48000 Hz, input level, exclusive mode and enhancements); the
  moved banner with **Keep**.
- A host program hears `MISSING SOUNDCARD` when the card goes while it is attached, as at attach.
  VARA's command set has no word for its return.
- The log: the failure, the return, and a device found under another name.

## 3. What it removes

The once-only open in `main.rs::sound` and its separate fault path; the panel's longer audio fault
sentence. `AUDIO_STALL_RELEASE` stays for a keyed card that stalls short of `STALL`.

## 4. Not built, for the author

1. **Write the found name to the file by itself.** Today it is used for the run and offered with
   **Keep**. Writing it automatically would save a click and a restart, but rewrites the operator's
   settings without asking, and a profile switch would undo it.
2. **Match by hardware identity instead of name.** Windows' endpoint ID or the USB VID:PID and
   serial would tell two DigiRigs apart and survive a rename the prefix rule does not cover (a
   device renamed in Sound settings). cpal does not expose them; it needs the Windows API directly
   (`IMMDevice`/`PKEY_Device_InstanceId`), Windows only.
3. **Set Windows' format for the operator.** Setting a new endpoint to 48 kHz, its level, and
   turning off exclusive mode and enhancements is possible through undocumented `IPolicyConfig`
   or the registry; not built, and only advised.
4. **Resample** a device stuck at 44.1 kHz instead of refusing it (ADR-0002's numerology is 48 kHz;
   a resampler is a receiver change to bench).
5. **Watch the device list** (`IMMNotificationClient`) instead of trying every 2–10 s.
6. **End a session** at once when the card goes, rather than leaving it to the link timeout.

## 5. Checked

`card.rs` tests: the instance rule (prefix inside parentheses or at the start; a model whose name
has a number in it is not taken for one), present names kept, both sides found in another port and
back, two candidates refused, another model refused, one codec's capture never paired with
another's playback, the system default never resolved, the retry intervals, and a fake machine
whose card is unplugged (silence, clock kept) and comes back in another port, and one missing at
start opened when it appears. `station.rs`: nothing keyed while down, a transmission on the air let
go. `methods.rs`: the refusals and what stays allowed. `host/server.rs`: `MISSING SOUNDCARD` mid-run.
`profile.rs`: the suggestion. `panel_check.mjs`: the moved banner with **Keep**, and the fault
banner with its tooltip. Windows behaviour (the endpoint's name in another port, the stream's error
on unplug, the reopen) needs the author's machine: the steps are in the session's report.
