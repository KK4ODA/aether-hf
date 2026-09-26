# Aether HF against VARA HF and VARA Chat — the user-facing benchmark

*2026-09-15, at `v0.2.0-beta.14`. Sources are public only: the author's site
(rosmodem.wordpress.com and its comment threads), Winlink's download and setup pages, the
BPQ32 and Pat integration notes, third-party setup guides (N1CLC, DK5SM, DL1GKK, K6OLI's
VARA FM guide, the Masters Communications VARA FM primer) and VarAC's FAQ. The official
VARA HF and VARA Chat manuals are hosted on mega.nz and could not be fetched, so VARA Chat's
window is thinly documented here; items marked* were not confirmed from any reachable
source and are stated as widely reported. Nothing was learned from VARA's binaries. This is
a comparison of what each program shows and lets an operator do, not of how it works.*

## 1. What VARA HF and VARA Chat offer

**VARA HF, the modem window.** A waterfall (switchable), an audio-input VU needle that
should sit "in the green", an S/N dial, a CPU gauge, a registration indicator, and the
current speed level / bandwidth. Settings dialogs: *VARA Setup* (callsign, TCP command and
data ports, registration key, retries, bandwidth), *SoundCard* (input and output device, a
drive-level slider, a Tune button; VARA FM adds Auto Tune and Ping), *PTT* (CAT with brand,
port, baud and CI-V address; RTS/DTR; VOX; the RA-board). A monitor mode that shows decoded
text of other stations' links. Bandwidths 500 / 2300 / 2750 Hz. An automatic
new-version notification through the author's server. Multiple instances on different port
pairs. The published TNC command set gives a host program `PTT`, `BUSY`, `BUFFER`,
`CONNECTED`, `REGISTERED`, `IAMALIVE`, `CQFRAME`, `PING`/`PINGACK`*, `SN`* and `BITRATE`*.

**VARA Chat.** Keyboard-to-keyboard chat and file transfer over VARA HF/FM, peer to peer
(the author declined a CQ macro: "VARA was designed for peer to peer connection"); a
connect/disconnect model with the modem's S/N and level readings alongside*, a file
transfer with progress*, and interoperation with VarAC for plain QSOs. Its window layout,
heard list and statistics could not be confirmed.

## 2. The matrix

Classification: **E** essential, **H** highly useful, **N** nice to have, **X** not
appropriate for Aether.

| Feature | VARA HF | VARA Chat | Aether HF now (beta.14) | Recommended |
|---|---|---|---|---|
| S/N of the received signal | dial | shown* | **SNR** reading per frame, 3 kHz reference, plus a ten-minute chart against the mode's threshold | E — done |
| What the other station hears you at | via `PINGACK`* | shown after a ping* | **they hear you at**, from every acknowledgement during a session (`peer_snr_db`) | E — done (in-session); H — a probe outside a session is roadmap P7-1 |
| Audio input level / VU | needle | — | **Receive level** (RMS, peak, clipping) and the Setup meter | E — done |
| Waterfall / spectrum | waterfall | —* | **Spectrum and waterfall** on the Diagnostics tab, polled only while looked at | H — done |
| Speed level / mode | speed level | — | **Mode** with name and net bit rate; the rate controller's smoothed SNR and margin on Diagnostics | E — done |
| Throughput | `BITRATE`* / host's own | file progress* | **Throughput** over 30 s from bytes acknowledged or delivered, and the session's bytes each way | H — done |
| Session timer | host's own | — | **Session** duration with the remote and the role | H — done |
| TX / RX / busy indication | PTT and busy lamps | — | TX, **RX**, BUSY, LINK lamps and a two-minute TX/RX strip | E — done |
| Retries, buffer, counters | `BUFFER`; host's own | — | Queued bytes; counters as pills (retransmitted, missed ACKs, HARQ rescues, held for busy…) | H — done |
| Tuning / frequency offset | — | — | **Tuning** (carrier offset of the last frame, with "the other station is high/low") | H — done; Aether has it, VARA does not show it |
| Constellation | — | — | Diagnostics: the last frame's equalised symbols | N — done |
| Stations heard | monitor mode; FM 4.01 lists "station IDs using the channel" | heard list* | **Stations** tab: beacons, calls, answers and session partners, first/last heard, SNR, dial, mode, activity, count; sortable; kept in `heard.json`, bounded at 200 | H — done |
| Drive level and tune | slider + Tune | — | Slider in dB and a stoppable 10 s tone on the Session tab | E — done earlier (beta.5) |
| Auto Tune (FM) | FM only | — | — | X — a VARA FM feature for a level protocol Aether does not have; the drive slider and ALC reading do the job |
| Ping | FM 4.01; HF* | ping* | — | H — roadmap P7-1: a new air frame, model first with an ADR, shaped by §97.221(c) |
| CAT / RTS / VOX PTT | yes | — | serial RTS/DTR/both, CAT (Yaesu/Kenwood/Icom), rigctld, none (VOX) | E — done earlier |
| Dial frequency | — | — | In the header and the stations list when CAT or rigctld can ask | N — done |
| Bandwidth 500 / 2300 / 2750 | yes | — | 2300 only | H — P7-0 (500) and P9-3 (2750) |
| Monitor mode (decode others' links) | yes | — | Frames of other sessions are reported (kind, mode, SNR) but their payload is not shown | N — payloads of other people's sessions are compressed application data; showing them is a host program's decision, not the modem's |
| Registration / speed limit display | yes | — | — | X — nothing to register; the host adapter already answers `REGISTERED` |
| CPU gauge | yes | — | — | N — the modem's cost is a fraction of a core; dropped-sample warnings in the log cover the case that matters |
| Update notification | via the author's server | ? | Signed updates from GitHub with a **window** of the shell's own: checking → available (notes) → downloading (progress) → installing → complete / incomplete / error; kept installers for going back | E — done |
| Chat, file transfer, emojis, "is typing" | — | yes | — | X — the host program's job (Pat, Winlink Express, VarAC, VARA Chat itself over the adapter); Aether is the modem |
| CQ / beacon | `CQFRAME` | no CQ macro | Beacon button and `CQFRAME`; beacons heard are listed | E — done earlier |
| Multiple instances | yes | — | yes (`--config`, any ports; the bench runs two) | done |
| Compact window | small modem window | — | **Compact** toggle: state and four readings | H — done |
| Diagnostics for a bug report | — | — | The diagnostic bundle, recordings with sidecars, replay | done earlier; Aether-specific |

## 3. What was added in beta.14

* **Core telemetry** (`aetherd`): a `frame` event per received frame with the receiver's own
  estimates and the callsigns the frame carries; `metrics` with the last frame's SNR and
  offset, the peer's report, the rate controller's readings, `receiving`, throughput and the
  session's account; `status.frequency_hz` and `status.host`.
* **Two engine additions, model first:** the sender keeps the SNR the peer reports
  (`peer_snr_db`) and counts acknowledged bytes (`bytes_acked`).
* **Visualisation:** SNR-by-frame chart with the mode's threshold, level-vs-floor chart,
  TX/RX activity strip, constellation, spectrum, waterfall.
* **Stations heard:** `heard.rs`, `heard.list` / `heard.clear` / `heard` events, persisted
  and bounded.
* **Panel:** the dashboard, Stations and Diagnostics tabs, Compact, the RX lamp, the dial in
  the header, Help text for the readings.
* **The updates window.**

## 4. Deliberately not added

* **PING as an air frame** — it needs a frame format, an ADR and the model first; it is on
  the roadmap as P7-1 with the answer-only rule already worked out. *Later the same day:*
  ADR-0006 and the model. A search for VARA's `PING`/`PINGACK` on the command port found
  no public source for it — the vocabulary is ARDOP's, and VarAC's ping is a short session
  over `CONNECT` — so the rows above that cite it describe VarAC's behaviour, not a VARA
  modem command.
* **A registration display** — there is nothing to register.
* **Chat features** — they belong to the host program; the adapter is where VARA Chat, VarAC,
  Pat and Winlink Express plug in.
* **Auto Tune** — it is a VARA FM level protocol between two VARA modems.
* **Payload monitoring of other stations' sessions** — the payloads are compressed
  application data belonging to somebody else's session.
* **A CPU gauge** — a number that would always read low; the log's dropped-sample warning is
  the reading that matters.

## 5. Since beta.14 (to beta.68)

The matrix above is the snapshot of 2026-09-15. What has changed in its rows since:

* **What the other station hears you at** — outside a session too: the probe (ADR-0006,
  beta.16), on the tone floor since ADR-0016 and answered down to −12 dB; and every control
  frame of a session carries it, so a station that only received learns it from the other's
  disconnect (ADR-0021).
* **Ping** — the probe is Aether's; VarAC's ping, a short session over `CONNECT`, works through
  the host interface at 500 Hz on the bench (beta.18).
* **Bandwidth** — 500 Hz shipped (P7-0, beta.15); 2 750 Hz is still P9-3.
* **CQ / beacon** — beacons repeat on a timer (10–240 minutes, beta.67), wait for a clear
  channel, and are counted per station on the Stations tab, which also keeps the history of
  sessions (beta.58).
* **Drive level and tune** — *Set drive* sends real bursts for the ALC to see; the tune tone
  reads 6–7 dB under the waveform's peaks and is for antenna tuners only.
* **Dial frequency** — the Session tab keeps dial memories and tunes the radio over CAT or
  `rigctld`.
* **KISS** — a KISS port that answers as VARA HF's does, for APRS and packet programs and
  VarAC's broadcasts (ADR-0019, beta.61).
* **The rules** — nothing VARA has: every transmission is judged against Part 97 at the dial
  the radio is on, with a LEGAL / WARNING / TX BLOCKED badge and its reasoning (ADR-0018,
  beta.60).
* **The panel** — redesigned in betas .60–.68, the signal card undockable into its own
  window.

## 6. Found on the way

A DATA body one byte short of a full frame is the one length the container cannot carry (a
partial body needs its two length bytes, and then it no longer fits); the engine built it
anyway and the modem panicked mid-session for a message of exactly that length. Fixed in the
model and the port, with a regression test in each.
