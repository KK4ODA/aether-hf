# Aether HF with Winlink Express

Windows only, for Aether 0.2.0-beta.178 and later (updated 2026-11-25). The manual has the
details in [Finding Winlink's programs](../manual/manual.md#finding-winlinks-programs) and
[Winlink Express, Pat and VarAC](../manual/manual.md#21-winlink-express-pat-and-varac).

## What you need

- Winlink Express, set up for your radio under your plain registered callsign. If its callsign
  has an SSID, sessions connect but no mail moves.
- A radio that sends upper-sideband data, with a sound card interface.
- Aether at the other end as well, either on another station or on an Aether gateway. Aether
  can't reach a VARA station or gateway.

## 1. Install Aether

1. Download the file ending in `x64-setup.exe` from the
   [releases page](https://github.com/KK4ODA/aether-hf/releases) and run it. The first time,
   Windows SmartScreen will stop it: click **More info**, then **Run anyway**.
2. Open **Aether HF**. It first asks about **debug mode**. **Keep it on** sends recordings of your
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

## 3. Set up Winlink Express

1. Close Winlink Express first, or it will undo Aether's changes when it closes.
2. In Aether, go to Setup step 5, **Winlink programs**. Each Winlink Express install has its own
   row, with its callsigns and folder.
3. In that row's **Keying** list, choose which program keys the radio. **Winlink Express** keeps
   its radio setup and keys on Aether's `PTT ON`, as it does with VARA. **Aether** keys the radio
   the way Winlink Express did, and Winlink Express's own PTT is turned off.
4. Tick the row. A window shows what will change. Leave **Switch to it now** ticked and click
   **Set up**. To keep your current install on VARA, click **Make a copy first**: Aether sets up a
   copy of the whole folder, mail included.
5. From now on, start Aether before Winlink Express: Aether has turned off its automatic launch
   of the modem.

## 4. Set the drive

Go to Aether's **Session** tab, open **Keying and drive** and click **Set drive**. Turn the
transmit level up until the ALC just starts to move, then back it off a little. If Winlink
Express keys the radio, open its session window first so it's there to key it.

## 5. Making connections

For a peer-to-peer contact, open a *Vara HF P2P* session and enter the other station's callsign
and the frequency you agreed on.

To use an Aether gateway, first set *Settings › Winlink Express Setup › Service Codes* to
`AETHER` (or `PUBLIC AETHER`). Then open a *Vara HF Winlink* session, choose *Channel Selection ›
Update Table Via Internet*, and pick a gateway from the list.

You choose the bandwidth in the session window; 2750 Hz runs as 2300. Below about +3 dB SNR,
500 Hz gets more through.

If Winlink Express keys the radio, Aether can't read its dial, so each time you change channel,
type the new dial into **Dial is at** on the Session tab for the rules check.

## Going back to VARA

In Setup step 5, untick the row and click **Go back to VARA**. Aether puts back the Winlink
Express settings it changed, unless you've changed them since.

## If something is wrong

| What you see | What to do |
|---|---|
| The session connects but no mail moves | Use your plain callsign in Winlink Express, without an SSID. |
| Aether closes when you close Winlink Express | Turn off Winlink Express's automatic launch of the modem, and start Aether yourself. |
| Winlink Express says *Disconnecting* five minutes into a long message | That's Winlink Express's own timeout. If the message can still arrive within 15 minutes, Aether keeps sending it and shows a banner with **Disconnect** and **Abort**. |
| The radio doesn't key | Only one program can key it. To switch, go back to VARA, pick the other option under **Keying**, and set the row up again. |
| Nobody answers | The other end has to run a version of Aether that can talk to yours. If Aether's update window says *Update required*, both stations need it. For gateways, check the service code is `AETHER`. |
