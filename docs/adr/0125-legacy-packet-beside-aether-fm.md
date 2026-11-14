# ADR-0125: Legacy packet runs beside Aether FM, built from public specifications

**Status:** proposed, 2026-11-14. This is the Aether FM proposal's FM-13
(`docs/research/aether-fm/00-proposal.md` §D.6, §L), with the author's decision 7 (legacy packet
from the start). It is its own track: specified now, built in step 10.4 beside the core modem,
and tested in step 10.5. Nothing is built for it yet.

## 1. Context

On VHF and UHF, packet radio is what most stations already run: APRS, BBSs, nodes, and Winlink's
RMS Packet gateways. A station that speaks Aether FM and packet on one radio serves both, and
needs no second program for the old one.

Packet's signals are long established. So the research risk is small; the cost is building and
testing time (risk R16).

## 2. Decision

### 2.1 What is built, from which document

| Part | What it is | Audio path | Specification |
|---|---|---|---|
| 1200 baud AFSK | Bell 202 tones (1 200 / 2 200 Hz), NRZI, HDLC framing with bit stuffing and the FCS | Voice path | AX.25 2.2 (TAPR); Bell 202 |
| 9600 baud | G3RUH: scrambled (1 + x¹² + x¹⁷), shaped baseband | Flat port | James Miller's 1988 design note |
| FX.25 | Reed-Solomon around an unchanged AX.25 frame, so plain TNCs still read it | Either | The FX.25 specification |
| IL2P | Reed-Solomon, a scrambler and a compact header | Either | The IL2P draft 0.6 (TARPN) |
| AX.25 connected mode | Connect and disconnect, I-frames, RR, REJ and SREJ, windows and timers | Either | AX.25 2.2 |

The repository's licence (MIT or Apache-2.0) forbids copying GPL code. Dire Wolf (GPL-2.0) is a
reference for interoperability and for its published test results, never a source of code.

### 2.2 Where it lives

- A crate `aether-packet` (ADR-0112): the modems, the framing and AX.25.
- Model first, as every layer has been: a Python reference in `model/aether_model/packet/` with
  vectors the port is held to bit for bit, for:
  - HDLC with its stuffing and FCS;
  - NRZI;
  - the G3RUH scrambler;
  - the FX.25 and IL2P encoders.

### 2.3 How it shares the station

- **Shared:** audio, keying, the regulatory gate, recording and the panel.
- **Both receivers run on the same audio.** A station hears packet and Aether FM on one channel at
  once.
- **Packet keeps its own rules on the air:** CSMA with p-persistence, and AX.25's own ARQ. Granted
  slots are Aether FM's alone.
- **Aether FM counts packet as busy:** a packet frame heard is activity (ADR-0120 §2.2).
- **Levels:**
  - AFSK has a constant envelope, so it goes just under the limiter knee calibration found
    (ADR-0117).
  - 9600 goes at the flat port's knee and the operator's deviation check (G3RUH's ±3 kHz).
  - Packet partners report nothing back, so packet's level is never looped.
- **Identification:** AX.25 frames carry the callsign in their address field, which identifies the
  station by packet's established reading (ADR-0123 §3).

### 2.4 Interfaces

- The KISS port, in its packet mode, is a KISS TNC (ADR-0124 §2.3).
- An AGWPE port serves connected-mode programs.
- RadioPost could reach AX.25 through RP-4 §9.4's AGWPE mapping. It is not needed: RadioPost uses
  Aether FM itself.

## 3. What this removes

Nothing on HF. It replaces the roadmap's old plan of an AGW adapter alone.

## 4. What it adds

The crate, the model, the vectors, the two ports' packet modes, and Setup's packet options:
- off;
- 1200 on the voice path;
- 9600 on the flat port;
- FX.25 or IL2P on transmit, both always decoded.

## 5. Criteria (step 10.5)

- **K1 (decoding).** On the WA8LMF TNC test recordings that Dire Wolf's own documentation uses,
  the 1200 baud receiver decodes at least 95 % of the frames Dire Wolf's User Guide reports for
  the same track. The comparison is with its published count, not a copy of its method.
- **K2 (interoperability).** Both ways, with Dire Wolf and with a NinoTNC: UI frames; AX.25
  connected sessions carrying 10 kB; FX.25 (Dire Wolf) and IL2P (NinoTNC) frames decoded by the
  other side.
- **K3 (sharing).** On the harness, a packet frame on the channel holds every Aether FM call, and
  Aether FM's sessions cause no packet frame to be lost to a collision that the reservation should
  have prevented.
- **K4 (bit-exactness).** The port matches the model's vectors bit for bit, as every ported layer
  does.
