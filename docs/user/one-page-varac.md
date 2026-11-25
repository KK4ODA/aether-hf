# Aether HF with VarAC

Windows only, for Aether 0.2.0-beta.178 and later (updated 2026-11-25). The manual has the
details in [VarAC in one step](../manual/manual.md#varac-in-one-step).

## What you need

- VarAC, installed and working with your radio the way you would set it up for VARA HF.
- A radio that sends upper-sideband data, with a sound card interface.
- Someone to work who also runs Aether. Aether can't talk to VARA on the air.

## 1. Install Aether

1. Download the file ending in `x64-setup.exe` from the
   [releases page](https://github.com/KK4ODA/aether-hf/releases) and run it. The first time,
   Windows SmartScreen will stop it: click **More info**, then **Run anyway**.
2. Close VARA HF. While it's running, it holds the ports and the sound card that Aether needs.
3. Open **Aether HF**. It first asks about **debug mode**. **Keep it on** sends recordings of your
   sessions to the project to help find problems on real paths. **Turn it off** keeps them on
   your computer.

## 2. Set up Aether (steps 1 to 3)

Setup opens by itself the first time.

1. Type your callsign in **Aether callsign**. Choose the FCC Part 97 rules, *Local* control, your
   licence class, and USB for the sideband.
2. Under **Preset**, pick your interface. Then choose the radio's sound card for **Capture** and
   **Playback**.
3. Tune to a quiet frequency and adjust the radio's audio output until the receive level meter
   reads **Good**.

Click **Save** at the bottom (step 6).

## 3. Set up VarAC

1. When the banner says *VarAC is on this computer*, click **Set up**. Aether writes a new VarAC
   profile, **VarAC_Aether.ini**, which is a copy of yours with only the modem settings changed.
   It also saves an Aether profile called **VarAC default**. If VarAC isn't in `C:\VarAC`, type
   its folder in Setup step 5 under **VarAC**.
2. Click **Switch to VarAC default**.
3. In VarAC, use **Switch profile** to pick **VarAC_Aether**. VarAC restarts and starts Aether
   for you.

## 4. Set the drive

With VarAC running, go to Aether's **Session** tab, open **Keying and drive** and click **Set
drive**. Turn the transmit level up until the ALC just starts to move, then back it off a little.
The drive setting in VarAC has no effect on Aether.

## 5. On the air

VarAC keys and tunes the radio, just as it does with VARA. Aether can't read the dial in this
setup, so you have to check your frequency yourself. Use your plain callsign in VarAC, without an
SSID. Ping, chat, file transfers, CQ, beacons, broadcasts and relayed VMail all work.

## Going back to VARA

Switch VarAC back to its **VarAC** profile. In Aether, choose your usual profile from the Profile
bar in Setup, or just close Aether.

## If something is wrong

| What you see | What to do |
|---|---|
| VarAC still connects to VARA | Close VARA HF and make sure VarAC is on the *VarAC_Aether* profile. |
| The banner says *VarAC_Aether.ini needs updating* | Click **Update**. Aether changes only the settings it manages, keeps anything you changed in VarAC, and saves the old file as `VarAC_Aether.ini.bak`. |
| The beacon button says *BEACON PAUSED* | That's VarAC waiting. It holds beacons for five minutes after **Call CQ**, and while the channel is busy. |
| The log says *the radio may not be transmitting* | Check that VarAC's RIG tab has PTT on CAT, the way you had it for VARA. |
| Nobody answers your call | The other station has to be running Aether, in a version that can talk to yours. If Aether's update window says *Update required*, both stations need the update. |
