# ADR-0064: The tone floor searches ±150 Hz

**Status:** accepted, 2026-10-11. Model first (`tone.MAX_CFO_HZ`, the `ToneDetector` default),
then the port (`max_cfo_hz` in `core/aether-phy/data/preamble_tables.json`, read by `build.rs`).
Receiver only: no wire change, no configuration key.

## 1. Context

The nightly harness run had failed `15m-old-rig-offset-500` on 2026-10-10 and 10-11: a 500 Hz
session with one rig 70 Hz off and warming 0.3 Hz/s. Every transfer crossed, but the session's end
did not: one station's DISC went unacknowledged and the other waited out its link timeout, or both
did. Run locally on the current build, three seeds of four failed the same way.

The receiving station's sidecar showed why. Its last decoded frame was at 176 s with a carrier
offset of 123 Hz; nothing the other station sent after that was found. The data frames were OFDM,
whose acquisition searches ±300 Hz (`sync.rs`, `DEFAULT_MAX_CFO_HZ`), and kept decoding. The
session's control frames by then were tone-floor frames (ADR-0013, ADR-0059), and the tone
detector searched only ±100 Hz (`ToneDetector(max_cfo_hz=100)`). Frames up to about 119 Hz still
landed on the edge of the search and decoded; past that the DISC and its answer were never
detected.

The same limit applies to every tone-floor frame on the air: calls, probes, beacons, the
slowest rungs and, on a lopsided path, every acknowledgement. A station more than about 100 Hz
off frequency could hear OFDM and not the floor.

## 2. Decision

The tone detector searches **±150 Hz** of carrier offset (`MAX_CFO_HZ`).

- **On noise**, measured with the streaming detector over 30 minutes of white noise (three seeds
  of ten minutes): the highest first-block statistic was 5.26 at both widths, 3 first blocks
  crossed the announce threshold (4.8) at ±100 Hz against 4 at ±150 Hz, and no frame was found at
  either. The detector's cost rose about 6 %.
- **At 500 Hz**, a frame 150 Hz off puts its outer tone at 350 Hz from the centre, inside the band
  filter's transition and above its stopband, so the wider search finds frames the filter
  passes.
- `test_the_detector_finds_a_frame_up_to_150_hz_off` (model): frames 112–148 Hz off either way are
  found, named and placed at −12 dB; it fails with the old ±100 Hz.

## 3. Consequences

- `15m-old-rig-offset-500`: 4 of 4 seeds pass (1 of 4 before). Ten other 500 Hz and 2300 Hz
  scenarios, QRM and FT8 neighbours included, pass unchanged.
- The specification's receiver requirement (§6) and the tone detector's table row now say
  ±150 Hz. The vectors are unchanged.
- A station more than 150 Hz off is still lost to the floor. Following the session peer's
  measured offset (centring the search on it) is the complete answer for a rig that keeps
  drifting; it waits for a field report that needs it. Built 2026-10-20 as ADR-0079.
