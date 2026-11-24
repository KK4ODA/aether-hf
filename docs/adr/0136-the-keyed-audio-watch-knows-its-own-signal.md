# ADR-0136: The keyed-audio watch knows the station's own signal coming back

**Status:** accepted, 2026-11-24 (the author's answer to report 21 §9's fix 5, "recognise its own
signal"). The daemon only: **no wire change, no configuration key, no schema bump.** It amends the
keyed-audio watch (`station/keyed_audio.rs`, 2026-11-02; its band, 2026-11-12).

## 1. Context

The keyed-audio watch warns when the radio's receive audio does not change while it is keyed:
three transmissions in a row whose level in 1–2.5 kHz sits between the receive audio's quiet end
less 6 dB and its loud end plus 3 dB. A radio that mutes its receiver falls under that; one that
monitors its own signal loudly rises over it.

On 2026-11-24 the portable's Lab599 TX-500 did neither
(`docs/research/link-architecture/21-wle-p2p-40m-beta174.md` §6). It mutes its receiver while
keyed — outside its signal the audio fell 28–54 dB — and sends its own transmitted signal back to
the sound card, at −19 to −23 dBFS in the watch's band, where a busy 40 m read −20 to −29 while
receiving. The watch said *the radio may not be transmitting* while home decoded every frame.
What came back was the station's own signal: the portable's frames, decoded from its own keyed
audio and rendered again, matched it at |r| 0.98–0.99, about 0.26 s after the key, the sign
flipping between keyings.

## 2. Decision

A transmission the level calls unchanged is checked for the station's own signal before it
counts toward the warning. The watch keeps what the transmission played, from its first sample
(the key's lead of silence first), and one second (`MONITOR_LISTEN_S`) of what the radio sent
back, from the key's settle (0.3 s) on. `monitored` correlates the two in the watch's band at
every delay up to `MONITOR_LOOP_S` (0.8 s: the card's output and input latencies and the radio's
own, with room), by FFT, each delay normalised by the energy of the part it compares. A
correlation of 0.5 or more, of either sign (`MONITOR_CORRELATION`), makes the transmission
monitored: it is on the air, and the run of unchanged transmissions starts again.

A key that does nothing leaves the band's own noise, which no delay of the played signal matches:
in the tests it reads under 0.2. For the check to pass on a leak inside an interface instead of
the radio, the leaked signal would have to be within about 5 dB of the band's noise (a
correlation of 0.5 needs the matching part to hold a quarter of the power); a leak sits tens of
decibels lower. The check runs at either bandwidth, since it reads the signal itself and not the
band around it.

## 3. What it removes

Nothing. A level test that alone could not tell a radio that returns its own signal from one that
never keyed; it stays the first test, and the correlation runs only when it says unchanged.

## 4. Rejected

* **The fall outside the station's own signal** (report 21 §6's first way): at 500 Hz the band
  leaves 1–1.22 and 1.78–2.5 kHz outside the channel, where the TX-500 fell 28–54 dB, but at
  2300 Hz the signal fills the band.
* **A wider margin above the loudest receive level.** The TX-500's return sat inside the busy
  band's range; any margin wide enough to pass it would pass a dead key on a quiet band.

## 5. Checked

`keyed_audio.rs`'s tests: the TX-500's case (its own signal back at the band's level, 0.26 s late,
the sign flipped) is not warned about; a key that does nothing, at the same levels, is still
warned about after three; the match reads the signal 0.4 s behind under noise as at least 0.9,
and noise alone against the played signal under 0.2. Owed: the portable's next session on the air
with the TX-500.
