# Recordings for the busy-detector investigation

**No other station is needed.** Every recording here is listening only. The recording box works
with no session, any time the modem is running.

What we are after: the moments when a signal makes the FTDX10's AGC turn its gain down, the noise
in Aether's channel drops and stays down, and the channel then reads busy for about 71 s after
the signal has gone (the panel log of 22:12–22:14, and 20:41–20:42). The recordings let me measure
how deep and how long the AGC holds the gain down, how fast it lets go, and where the signal that
caused it sat.

## What a recording holds (checked in the code)

- **The audio**: what the radio delivers over USB, 48 kHz, 16-bit mono WAV, about 6 MB a minute.
  It is the radio's whole audio passband, so signals outside Aether's channel are in it too.
- **The 30 s before you pressed the button**: a recording starts with the last 30 s the modem
  heard.
- **The `.json` beside it**: your notes, the dial frequency when it started (read over CAT), every
  change of the busy indicator with the level, floor and reason, and every frame the modem found.
- **Not in it**: dial changes after the start. Aether reads the dial only now and then and does not
  write it into the recording. Time your VFO moves by hand (part C says how).
- **Where it goes**: `%APPDATA%\aether-hf\recordings\`. **Open folder** in the recording box opens it.

## Once, before the first recording

1. **AGC times.** In the FTDX10's menu, find the AGC settings. Write down the FAST, MID and SLOW
   delay times.
2. **AGC AUTO.** Write down what AUTO picks in the mode you run Aether in. The radio's manual has a
   table; the display may show it too.
3. **Mode and filter.** Write down the mode (USB or DATA-U) and the filter width (for example
   3000 Hz).
4. **USB audio.** Write down the radio's USB audio output level setting. Don't change it between
   recordings.
5. **Aether.** Open *Help / About* and write down the version. Any beta from 113 on records the
   same way. Don't update between batches unless I ask.
6. **Bandwidth.** Write down the bandwidth Aether runs (500 or 2300 Hz). Use **500 Hz** for these
   recordings if you can (Setup step 4). At 500 Hz Aether's channel is 1250–1750 Hz in the audio,
   which leaves room on both sides for part C. At 2300 Hz it fills almost the whole passband.
7. **Programs.** Close VarAC, Winlink Express, RMS Trimode and any KISS program. They can make the
   modem transmit.

## Before every recording

1. **Receiver.** RF gain fully up (except AGC OFF, below). ATT off. IPO as you normally run it.
   NB, DNR, notch and contour off.
2. **Settle first.** A recording starts with the last 30 s the modem heard. Make every change at the
   radio (band, mode, filter, AGC, RF gain, attenuator, preamp) at least 30 s before you press the
   button, then leave it alone until you stop. If you have to change something during a recording,
   write down the time.
3. **Nothing transmits.** No session, no beacon repeating, no Test. A transmission makes the modem
   deaf for a moment and moves the radio's AGC. If a station calls you during a recording, write
   down the time; the recording is still useful.
4. **Notes.** On the Session tab, scroll to the recording box at the bottom. In the field beside
   the button, type the recording's letter, the frequency, the mode and filter, and the AGC
   setting. For example: *A 3.590 DATA-U 3000 AGC AUTO*.
5. **Start.**
   - Up to beta.114: the box is called *Recording*. Press **Record** to start and
     **Stop recording** to stop.
   - From the next beta: the box is called *Record the radio's audio*. Press **Start recording**
     to start and **Stop and save** to stop.
   - The line beside the box title says what was saved.
6. **One recording per setting.** Stop and start a new recording each time you change the AGC
   setting, and type the new setting in the notes first.
7. **Watch.** Keep the Status tab open. When the busy indicator stays on after a signal has gone,
   take a screenshot of the *Channel and activity* chart and write down the clock time.

## A. The real case (the most important)

1. Your usual frequency, at a time you usually see the problem. For example 3.590 MHz at night.
2. AGC AUTO.
3. Record for 15 minutes.

## B. Each AGC setting

Right after A, on the same frequency:

1. FAST, 5 minutes.
2. MID, 5 minutes.
3. SLOW, 5 minutes.
4. AUTO, 5 minutes.
5. OFF, 5 minutes. Turn the RF gain down first, until the *Receive level* on the Status tab sits
   where it did with the AGC on (about −27 dBFS). Keep its *peak* under −3 dBFS.

## C. Make the event happen when you choose

This one has a signal come and go on a schedule, so I can line the audio up with your moves.

**Set up:**

1. Find a strong, steady signal: a broadcast carrier, WWV or CHU, or a strong station in a long over.
   Call its frequency **F**.
2. In USB or DATA-U, the dial sets where F lands in the audio: dial at F − 0.7 kHz puts it at
   700 Hz, F − 1.5 kHz in the middle of Aether's 500 Hz channel, F − 2.3 kHz at 2300 Hz. Dial
   **above** F (say F + 1 kHz) puts it outside the passband (USB does not hear below the dial).
3. Put the "outside" frequency in VFO-B and the "inside" frequency in VFO-A. Then the A/B swap
   button moves the signal in and out in one press. (Moving the dial or using IF SHIFT also works;
   write down which you used.)
4. Start a phone stopwatch at the moment you press the record button.

**Run 1: the signal outside Aether's channel.**

1. Start with the signal outside the passband. Wait one minute.
2. Swap so the signal sits at 700 Hz (or 2300 Hz) in the audio, outside Aether's channel. Hold
   20 s.
3. Swap back out. Wait 40 s.
4. Do steps 2–3 five times. Write down the stopwatch time of every swap.

**Run 2: the signal inside Aether's channel.**

1. Set VFO-A so the signal lands at 1500 Hz in the audio.
2. Do five more cycles: 20 s in, 40 s out. Write down the times.

**Do all of C twice:** once with AGC AUTO, once with FAST. A new recording for each, with the
notes saying which (for example *C1 WWV 5.000 AUTO*).

**If you can, film the radio during C.** Point your phone at the S-meter and the scope, with the
PC's clock or the panel in the frame, so the video lines up with the recording.

## D. Optional: a busy FT8 frequency

1. 3.573 MHz, AGC AUTO.
2. Record for 5 minutes. Signals come and go every 15 s, across the passband.

## Sending

1. After each batch: Log tab → **Send files…** → *Send my files to the Aether project*.
2. Leave *sessions with* empty (all stations).
3. Pick a period that covers the batch: *hour* if the batch started less than an hour ago,
   otherwise *3 hours*.
4. Keep *audio* ticked.
5. One upload can be at most 400 MB. A and B together are about 230 MB, C about 130 MB, D about
   30 MB. Send A+B, then C, then D, rather than everything at once.
6. Then tell me in the chat what the batch holds, with your notes and stopwatch times for C.

## What I do with each batch

1. Fetch it and check each recording: the audio at 48 kHz, your notes, the busy lines, and for C
   your times.
2. Find every AGC event: the noise in Aether's channel drops while the power elsewhere in the audio
   jumps.
3. Measure each one: how deep and how long the drop was, how fast the noise came back (dB/s),
   where the signal that caused it sat, whether the floor fell under it, and how long the false
   busy lasted after it.
4. Tell you in a few lines what each recording showed, what is missing, and what to record again
   or differently, before you record the next batch.

## Tools that would make the data better (each needs a beta and your go)

| Tool | What it gives | Cost |
|---|---|---|
| **S-meter in recordings** | The radio's S-meter read over CAT 4 times a second while recording, written into the `.json`. It follows the AGC itself, which the audio can only suggest. | About a day with tests. One short CAT read each time (FTDX10 `SM0;`, Icom `15 02`, Kenwood `SM0;`, `rigctld` strength, FLRig's S-meter call), on the CAT thread that already keys the radio, only while recording and never while transmitting. A key request goes first, so keying is never delayed. |
| **A Mark button** | Beside the record button. Each press writes the time into the running recording. Replaces the stopwatch in part C. | A couple of hours. One button with a tooltip, one line in the manual. |
| **Dial changes in the recording** | While recording, the dial is read once a second and every change is written into the `.json`, so VFO-A moves in part C mark themselves. | A couple of hours, shares the S-meter's CAT reads. Needs CAT, `rigctld` or FLRig keying, as your station has. |

A and B need none of these. You can record them today. C is better with the Mark button or the
dial log, but works with a stopwatch.

**Later, other radios:** recording A from ND1J (FTDX101MP), WC4Y, KE4QCM (FT-100) and an IC-7300
user would show whether other rigs do the same. I'd ask them through *Ask a station to send me its
files*, only once you agree.
