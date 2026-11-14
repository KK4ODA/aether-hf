# ADR-0118: What an FM station learns and keeps, per radio and per peer

**Status:** proposed, 2026-11-14. This is the Aether FM proposal's FM-6
(`docs/research/aether-fm/00-proposal.md` §E.5, §L). It is built with calibration in step 10.3
and accepted on §5's criteria. Nothing is built for it yet.

## 1. Context

Calibration (ADR-0117) finds the level at which the far receiver hears this station best. Part of
that is local: the knee of this station's own radio, set by its sound card, its menus and its
limiter. That part is the same for every peer. The rest depends on the peer: the far receiver's
filters and the path.

Kept apart, a known peer's call needs only a two- or three-step ladder to verify. A new peer
starts from this radio's knee, not from a guess. Kept together, every change of peer would look
like a change of radio.

## 2. Decision

### 2.1 Two profiles

| Profile | Key | Holds | Dropped when |
|---|---|---|---|
| **The local radio** | The sound card endpoints (by their system identifiers), the radio model (from CAT or declared), the port (voice or flat) | The transmit knee in modem dBFS (the median over peers); the emphasis choice; the operating system's volumes when calibrated; the receive level's statistics; keying latency (ADR-0071) | A volume, the device or (over CAT) the radio's audio menus change; or the loop needs more than 4 dB from it with three peers in a row |
| **A peer** | The peer's callsign (`base_callsign`, ADR-0035) and the local profile | The offset from the local knee; the last SNDR; simplex or repeater; its answer gap (ADR-0054) | 30 days unused; or Stage 1 disagrees by more than 3 dB |

### 2.2 Where they live

- In `fm-profiles.json` beside the configuration, as `heard.json` is.
- At most 8 local profiles and 200 peers, the oldest dropped first.
- They are hardware- and machine-scoped (ADR-0011): never in a portable profile and never
  shared, since they describe one machine's sound cards and radio.

### 2.3 What the station does with them

- **A call** centres Stage 1's ladder on the peer's offset when it has one, or else on the local
  knee, or else on the default.
- **A changed volume** is said on the panel, as P10-7 asked: *"the sound card's volume changed
  since calibration: 3 dB lower"*. The station offers to put it back; it never does so silently.
- **The panel's Setup** shows the local profile's knee, and a button to forget it.

## 3. What this removes

- The roadmap's P10-7 row in its FM form (the guard of the sound card's volume, here §2.3). Its
  HF form stays open.
- Any need for the operator to set the level by hand on a second visit. The first call to a new
  peer still runs the full ladder.

## 4. What it adds

- A file beside the configuration, with no configuration key.
- The panel line and button of §2.3.
- The manual's FM section says what is kept and how to forget it.

## 5. Criteria (step 10.3, on the model)

- **P1.** A second call to a known peer, nothing changed, runs at most three ladder steps and
  starts within 1 dB of the level the full ladder finds.
- **P2.** A new peer called after five others starts within 2 dB of its best level in 90 % of
  trials, from the local knee alone.
- **P3.** A sound-card volume moved 6 dB between sessions is noticed at the next call and named.
- **P4.** Dropping a profile never stops a call: the full ladder runs instead.
