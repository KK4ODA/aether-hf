# Aether HF with VarAC

**One page, on Windows.** Updated 2026-11-25, for Aether 0.2.0-beta.178 or later. The manual's
[VarAC in one step](../manual/manual.md#varac-in-one-step) has the whole of it.

## You need

- VarAC, installed and set up for your radio, as for VARA HF.
- A radio that sends upper-sideband data, and its sound card interface.
- Aether at the other station too: Aether and VARA do not hear each other on the air.

## 1. Install Aether

1. On the [releases page](https://github.com/KK4ODA/aether-hf/releases), download the file
   ending in `x64-setup.exe` and run it. SmartScreen asks once: **More info**, then **Run
   anyway**.
2. Close VARA HF. While it runs it holds the ports and the sound card.
3. Open **Aether HF**. A window asks about **debug mode**: **Keep it on** sends your sessions'
   recordings to the project, to find problems on real paths; **Turn it off** keeps them here.

## 2. Setup, steps 1 to 3

1. **Aether callsign**; **Rules** *United States — FCC Part 97*; **Control** *Local*; your
   **Licence class**; **Sideband** USB.
2. **Preset**: your interface. **Capture** and **Playback**: the radio's sound card.
3. **Receive level**: on a quiet frequency, turn the radio's audio until the meter says
   **Good**.

Then step 6, **Save**.

## 3. VarAC, in one step

1. The banner says *VarAC is on this computer*: press **Set up**. Aether writes
   **VarAC_Aether.ini**, a copy of your VarAC profile with only its modem settings changed, and
   saves the Aether profile **VarAC default**. VarAC not in `C:\VarAC`? Type its folder in
   Setup step 5, **VarAC**.
2. Press **Switch to VarAC default**.
3. In VarAC, **Switch profile**, then **VarAC_Aether**. VarAC restarts and starts Aether.

## 4. Set the drive

With VarAC running, open Aether's **Session** tab, **Keying and drive**, **Set drive**. Raise the
transmit level until the ALC just moves, then back it off. VarAC's drive level changes nothing.

## 5. On the air

- VarAC keys and tunes the radio, as with VARA. Aether cannot read the dial then, so check your
  frequency yourself.
- Use your plain callsign in VarAC, with no SSID.
- Ping, chat, files, CQ, beacons, broadcasts and relayed VMail all work.

## Back to VARA

In VarAC, **Switch profile**, then **VarAC**. In Aether, pick your usual profile on Setup's
Profile bar, or close Aether.

## If something is wrong

| What you see | What to do |
|---|---|
| VarAC still talks to VARA | Close VARA HF, and check VarAC's profile is *VarAC_Aether*. |
| *VarAC_Aether.ini needs updating* | Press **Update** on the banner. Only Aether's settings change; the old file is kept as `.bak`. |
| The beacon button reads *BEACON PAUSED* | VarAC waits five minutes after **Call CQ**, and while the channel is busy. |
| Log: *the radio may not be transmitting* | Check VarAC's RIG tab: PTT on CAT, as with VARA. |
| Nobody answers your call | The other station must run Aether, and a current one: an update marked *Update required* means stations of different versions cannot connect. |
