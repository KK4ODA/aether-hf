# ADR-0124: Aether FM's host interfaces are HF's, with the air named

**Status:** proposed, 2026-11-14. This is the Aether FM proposal's FM-12
(`docs/research/aether-fm/00-proposal.md` §C, §L). It is accepted when §5's benches pass, in
steps 10.4 and 10.5. Nothing is built for it yet.

## 1. Context

Programs reach Aether HF through four doors:
1. the control API (`docs/spec/control-api.md`);
2. the VARA-compatible host interface (`docs/spec/host-interfaces.md`), from VARA's published
   command list;
3. the KISS port (ADR-0019);
4. RadioPost, through its Aether adapter (RP-4 §9.1), which is a control-API client claiming
   callsigns (ADR-0106).

Aether FM is the same daemon with another air (ADR-0112, ADR-0113). A program should not need to
know more than which air it is talking to.

## 2. Decision

### 2.1 The control API names the air

- `capabilities` and `status` say `air: "fm"` and the profile in use (voice or flat).
- Calibration's state is shown: the local knee, and the last peer's report.
- No method is HF- or FM-only unless its subject is. Bandwidth moves are HF's; calibration is
  FM's.
- A method that makes no sense on the air in use answers `not_on_this_air`.

### 2.2 The VARA-compatible interface speaks VARA FM's published commands

VARA FM's published TNC command list is the reference, cited by name and date in
`host-interfaces.md` as VARA HF's is. VARA stays a black box: nothing is taken from its signal or
its behaviour, only from its published documents (the author's rule, 2026-10-30).

What differs from HF's adapter:
- `BW` commands, which have no FM meaning, are answered as the list says.
- `CONNECT <me> <them> VIA <digi>`, a relayed connection, is refused, since relaying is not in the
  first release (decision 5).
- `VERSION` answers with Aether's own name.

The shell's single instance, `PTT ON`/`PTT OFF` for a host-keyed radio (ADR-0025), and the
session ownership rules (ADR-0106, 0107, 0108) are unchanged.

### 2.3 KISS carries Aether's datagrams, and real AX.25 when packet is on

- With legacy packet off, the KISS port is ADR-0019's: Aether datagrams.
- With it on (ADR-0125), the KISS port is a KISS TNC: frames in and out are real AX.25 on 1200 or
  9600 baud.
- An AGWPE port serves connected-mode packet programs.

Setup says which the KISS port is. One port is never both.

### 2.4 RadioPost

RadioPost's Aether adapter needs no change beyond the modem name `aether-fm` (RP-0 §9.4, merged
2026-11-13) and RP-5 §4.4's FM channels. RadioPost's conformance tester's A-01 to A-04 are the
check.

## 3. What this removes

Nothing. It adds `air` and the FM profile to `capabilities`/`status`, the refusals of §2.1 and
§2.2, and the KISS port's packet mode.

## 4. What the manual says

The Setup and Session tabs on an FM station show its profile and calibration. The host-programs
guide gets a section per program, as `docs/user/host-programs.md` has for HF.

## 5. Benches (scratch copies only, never the author's real installs)

- **H1.** Winlink Express, with its VARA FM setting, completes a P2P B2F session with a 6 kB
  attachment over two FM daemons on the harness.
- **H2.** Pat's VARA FM driver does the same.
- **H3.** RMS Packet, or a packet BBS program through the AGWPE port, completes an AX.25 connected
  session once legacy packet works (ADR-0125).
- **H4.** RadioPost's conformance tester passes A-01 to A-04 over two FM daemons.
