# Aether HF with Winlink Express

**One page, on Windows.** Updated 2026-11-25, for Aether 0.2.0-beta.178 or later. The manual's
[Finding Winlink's programs](../manual/manual.md#finding-winlinks-programs) and
[Winlink Express, Pat and VarAC](../manual/manual.md#21-winlink-express-pat-and-varac) have the
whole of it.

## You need

- Winlink Express, set up for your radio under your **plain registered callsign**: it proposes
  no mail to a callsign with an SSID.
- A radio that sends upper-sideband data, and its sound card interface.
- Aether at the other end too: a station running Aether, or an Aether gateway. Aether cannot
  reach a VARA station or gateway.

## 1. Install Aether

1. On the [releases page](https://github.com/KK4ODA/aether-hf/releases), download the file
   ending in `x64-setup.exe` and run it. SmartScreen asks once: **More info**, then **Run
   anyway**.
2. Open **Aether HF**. A window asks about **debug mode**: **Keep it on** sends your sessions'
   recordings to the project, to find problems on real paths; **Turn it off** keeps them here.

## 2. Setup, steps 1 to 3

1. **Aether callsign**; **Rules** *United States — FCC Part 97*; **Control** *Local*; your
   **Licence class**; **Sideband** USB.
2. **Preset**: your interface. **Capture** and **Playback**: the radio's sound card.
3. **Receive level**: on a quiet frequency, turn the radio's audio until the meter says
   **Good**.

Then step 6, **Save**.

## 3. Winlink Express, in one step

1. Close Winlink Express: it writes its settings back when it closes.
2. Setup step 5, **Winlink programs**, lists each install, with its callsigns and folder.
3. In the row's **Keying**, choose who keys the radio. **Winlink Express**: it keeps its radio
   setup and keys on Aether's `PTT ON`, as with VARA. **Aether**: Aether keys the way Winlink
   Express did, and Winlink Express's PTT is turned off. One keyer, never both.
4. Tick the row. A window says what will change. Leave **Switch to it now** ticked and press
   **Set up**. Or **Make a copy first**: the copy, mail included, runs on Aether, and the
   install you have stays on VARA.
5. Start Aether first, then Winlink Express. Aether has turned its automatic launch off.

## 4. Set the drive

Open Aether's **Session** tab, **Keying and drive**, **Set drive**. Raise the transmit level until
the ALC just moves, then back it off. If Winlink Express keys the radio, open its session window
first, so it is there to key it.

## 5. Sessions

- **Peer to peer**: a *Vara HF P2P* session, with the other station's callsign and the dial you
  agreed.
- **Through an Aether gateway**: set *Settings*, *Winlink Express Setup*, *Service Codes* to
  `AETHER` (or `PUBLIC AETHER`). In a *Vara HF Winlink* session, *Channel Selection*, *Update
  Table Via Internet*, and pick a gateway.
- **Bandwidth** is the session window's; 2750 runs as 2300. Below about +3 dB SNR, 500 Hz
  carries more.
- If Winlink Express keys the radio, Aether cannot read its dial: after a change of channel, set
  **Dial is at** on Aether's Session tab. The rules check uses it.

## Back to VARA

Setup step 5: untick the row, then **Go back to VARA**. Aether puts back what it changed.

## If something is wrong

| What you see | What to do |
|---|---|
| The session connects and ends with nothing sent | Use your plain callsign in Winlink Express, with no SSID. |
| Aether closes when Winlink Express closes | Turn off Winlink Express's automatic launch, and start Aether yourself. |
| *Disconnecting* five minutes into a long message | That is Winlink Express's own timer. Aether carries on while the message can arrive within 15 minutes; a banner offers **Disconnect** and **Abort**. |
| The radio does not key | One keyer: change the row's **Keying** (go back to VARA, then set it up again). |
| Nobody answers | Both ends need Aether, and a current one: an update marked *Update required* means different versions cannot connect. Gateways: service code `AETHER`. |
