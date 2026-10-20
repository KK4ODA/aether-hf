# ADR-0079: The receiver follows a session's station off frequency

**Status:** accepted, 2026-10-20. Model first (`AudioToBaseband.offset_hz` in
`model/aether_model/phy/passband.py`), then the port (`AudioToBaseband::set_offset_hz`,
bit-exact through `phy_vectors.json`'s `baseband_moved`), then the daemon
(`core/aetherd/src/station/centre.rs`). Receiver only: no wire change, no configuration key.

## 1. Context

ADR-0064 widened the tone floor's search to ±150 Hz after `15m-old-rig-offset-500` (a rig 70 Hz
off, warming 0.3 Hz/s) lost the floor's DISC and its answer three minutes into a session. It left
the complete answer open: a rig that keeps drifting passes any fixed search, and ±150 Hz only
moves the edge. OFDM's ±300 Hz is the same edge further out. At 500 Hz the band filter is a
second edge: it passes ±280 Hz flat and stops at ±520 Hz, so a station 230 Hz off already loses
its upper tones.

Older rigs without a TCXO are common on HF, and a session on the tone floor can last many
minutes: long enough for a drifting rig to walk out of the search.

## 2. Decision

**The receiver centres on the station it is in session with.** The receive mixer moves to that
station's carrier offset, as its last decoded frame measured it, and the band filter and both
detectors' searches move with it.

1. **The mixer has an offset** (`offset_hz`): it mixes down from `centre_hz + offset_hz`. At 0
   it is the mixer it was, to the bit (`test_the_receive_mixer_at_no_offset_is_the_mixer_it_was`;
   the vectors are unchanged and gain a case moved partway through).
2. **It moves only at this station's own key.** The station is deaf while it transmits, so no
   frame is arriving for the mixer's step to cut in two, and every exchange of a session has a
   transmission of each station's. It moves when the offset to follow differs by 10 Hz or more
   (`FOLLOW_STEP_HZ`).
3. **It follows only the session's station**, in `Connected` and `Disconnecting`, from the last
   decoded frame that named it (a call or acceptance, a probe, the session's control and data
   frames). A call goes from the dial: what was heard of the station called may be a session
   old, and its answer is looked for where it was sent.
4. **Back on the dial between sessions**, at the first moment no frame is arriving once the
   engine is idle: a station that answers calls must hear them where they are sent. A capture
   gap inside a session keeps the centre; a move to the other bandwidth starts on the dial.
5. **Offsets are reported true**: what a frame measured plus the mixer's offset where it began
   (the panel's *Tuning*, the `frame` event, the sidecar's frames).
6. **Said and recorded.** A `receiver` log line when a followed station is 30 Hz or more off
   (`FOLLOW_SAID_HZ`), and `back on the dial` when the receiver returns from there. Each move
   is a recording event, `rx_offset` with the offset in hertz; the replay moves its receiver at
   the first block from then (`CentreMove`, `Expectation.centres`).

Each station follows what it hears and neither transmitter moves, so two stations following each
other cannot chase one another.

## 3. Measured

`bench/scenarios/40m-drifting-rig-500`: a weak 40 m path at 500 Hz (Good, −4 dB) so the session
runs on the tone floor, with one rig 100 Hz off at the start and drifting 0.4 Hz/s; a probe, a
call, 1 000 bytes, a 500-byte reply, 1 000 bytes and a disconnect. Three seeds:

| | passed | air time | the rig's offset at the end |
|---|---|---|---|
| before (`baf2b2a`) | 0 of 3 | 463–484 s, then a link timeout | — |
| this ADR | 3 of 3 | 360–490 s | 245–295 Hz |

Before, every seed carried the first 1 000 bytes in 103–124 s, while the rig was still inside
the floor's search, and then lost the reply: the session ended "no response / link timeout".
On the first seed the called station's receiver moved 14 times, from −107 to −285 Hz, each at
its own key, and went back to the dial after the session.

## 4. Tests

* Model: `test_a_receiver_centred_on_its_peer_hears_a_rig_past_the_search` (a floor frame 230 Hz
  off is not found on the dial and is found with the mixer at 200 Hz, its offset true) and
  `test_the_receive_mixer_at_no_offset_is_the_mixer_it_was`.
* Port: the `baseband_moved` vector (`model_vectors.rs`), the mixer moved to 73.25 Hz halfway.
* Daemon: `the_receiver_centres_on_a_station_off_frequency_and_comes_back_to_the_dial` (two
  stations, one rig 120 Hz high: each receiver centres on the other, reports the offset true and
  returns to the dial after the session; the test harness's `Air::with_rig_offset`), and the
  replay's `a_recording_whose_receiver_followed_a_station_off_frequency_replays_as_it_ran`.

## 5. Not done

* **Following within a frame.** A drift of 0.4 Hz/s moves a 5.4 s tone frame 2 Hz, which the
  detector's half-bin refinement absorbs.
* **Following on a call's first try.** The caller does not yet know where the station called is;
  the call's alternating families (ADR-0016) and the two searches are what find it.
