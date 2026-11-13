# ADR-0112: Aether FM is a second air in this repository

**Status:** accepted, 2026-11-13, the author's decision 1 on the Aether FM proposal
(`docs/research/aether-fm/00-proposal.md`, the Decisions table and §I). This is the proposal's
FM-1. It changes no code by itself: it says where Aether FM's code goes, and what stays out of
Aether HF's way while it is built.

## 1. Context

Aether FM is an FM sibling of Aether HF for VHF and UHF, and a transport for RadioPost
(`docs/ROADMAP.md` Phase 10). The proposal measured how much of Aether HF carries over (§B):
the FEC, the granted-slot link engine and the rate controller, every host interface, keying,
audio, claims, recording, the scenario harness and the release machinery. That is about
four-fifths of the code. The physical layer does not carry over. The HF busy detector,
bandwidth moves and offset following do not apply on FM.

So the question was where the remaining fifth lives: in this repository beside HF, in a fork,
or in a repository of its own. The author chose this repository, one application and one
installer.

## 2. Decision

1. **One repository.** Aether FM is built in `KK4ODA/aether-hf`. Its shared code is Aether HF's
   own code, used as it is. Nothing is copied.
2. **The crates.**

   | Crate | Role |
   |---|---|
   | `aether-fec` | Shared, unchanged |
   | `aether-phy` | HF's physical layer, unchanged; no FM code goes in it |
   | `aether-phy-fm` | New: FM's physical layer (FM-4 decides the waveform) |
   | `aether-packet` | New: legacy packet, AFSK 1200, G3RUH 9600, FX.25, IL2P and the AX.25 2.2 link (FM-13 decides its inside) |
   | `aether-link` | Shared: the granted-slot engine, rate control, frames and datagrams; FM's numbers come in as data (FM-7) |
   | `aetherd` | One binary that runs either air |

3. **The model.** FM's physical layer is model first, as HF's is:
   `model/aether_model/phy_fm/` and an FM channel with the radio in it
   (`model/aether_model/fm_channel.py`, FM-3). The port is bit-exact with the model and checked by
   vectors that CI regenerates, as for every HF layer. Legacy packet takes its reference vectors
   from the published specifications where they give examples, and from the model where they do
   not (FM-13).
4. **One daemon process per radio.** A station with an HF radio and an FM radio runs two
   `aetherd` processes, each with its own configuration, sound card, keying and ports. The air
   a daemon runs is a configuration key (`[radio] air`). It arrives with the release that carries
   the FM air, with the schema bump every new key needs (the shell's `SCHEMA_HISTORY` ties a
   schema to a release). This matches RadioPost, whose gateway holds one RPMI connection per
   modem (its wishlist W26).
5. **One application.** One shell supervises one or more daemons. Each daemon serves the panel,
   which shows the views of its air. Supervising several daemons is new shell work, due before
   the FM air ships (step 10.8).
6. **One version, separate link protocols.** One version number covers everything
   (`tools/release.py`). FM's link protocol has a number space of its own (FM-7). The update
   manifest and the shell's check carry both, so a change to one air's protocol never asks the
   other air's stations to update.
7. **Aether HF is not held up.** HF's wire, vectors and sessions never change for FM's sake. The
   one change FM asks of HF's code is the PHY boundary (ADR-0113), proven unchanged first. HF
   betas keep shipping while FM is built.
8. **CI stays within the account's minutes.** FM's unit and vector tests join the existing jobs.
   FM's harness scenarios live in `bench/scenarios/fm/`, outside the nightly's glob, until
   step 10.4 has a daemon to run them.

## 3. Alternatives considered

- **A fork, or a repository of its own.** Four-fifths of the code would be copied and would
  drift. Every fix to the link engine, the host port or the release pipeline would have to land
  twice. RadioPost would see two modems with two update channels. Rejected by the author.
- **A second binary, `aetherd-fm`.** Two daemons to build, sign, stage as sidecars and
  supervise, sharing everything above the physical layer. One binary that runs either air
  costs one configuration key.
- **One daemon process for both radios.** One fault would take both radios down. The audio loop
  is already one per sound card, and RPMI expects one connection per modem. Rejected.
- **FM inside `aether-phy`.** It would churn HF's crate and its vectors for code HF never runs.
  A crate of its own keeps HF's physical layer, and its proof against the model, untouched.

## 4. Consequences

- The repository keeps its name. Renaming it, or the application (Aether HF today), is the
  author's call before the FM air ships. Nothing in steps 10.1 to 10.7 depends on the name.
- The installer carries one daemon binary as now. The bundle grows by FM's physical layer and
  legacy packet.
- The manual gains FM chapters when an operator can see the FM air (step 10.8), under the
  manual rule.
- What it removes: nothing yet. What it rules out: a second copy of the shared code, and FM code
  in HF's physical layer.
