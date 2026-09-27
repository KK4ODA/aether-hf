# ADR-0035: The bandwidth trap — probes are answered across bandwidths, a known 500 Hz station is called at 500 Hz, and a call that cannot be answered is said on the panel

**Status:** accepted, 2026-09-27. The link engine (the probe's answer, `ProbeResult.bandwidth_hz`,
the events' wording; model first), the daemon (`station/bandwidth.rs`: the bandwidths learned,
`Why::Calling`, `Mismatch`; `heard.rs` `bandwidth_hz`; `status.bandwidth.mismatch`/`callee`, the
`mismatch` event) and the panel (the warning banner, the Stations list's Bandwidth column, the
bandwidth chip's *calling*). No wire change.

## 1. Context

KK4ODA's `aetherd.log` for 2026-09-27 01:08–06:36Z (beta.70; the log stays on the author's
machine): KK4ODA-1 ran 500 Hz and WC4Y ran 2300 Hz. Every one of WC4Y's 9 probes and 5 calls was
logged `ignored: WC4Y probes/calls in another bandwidth`, and KK4ODA's own 3 probes to WC4Y got
no answer. Each station heard the other for five hours, and neither operator could tell why
nothing came back.

Three rules made the trap, each sound alone:

* A probe was answered only in its own bandwidth (ADR-0006) — copied from the connect request,
  whose bandwidth bits say what air the session would run on.
* ADR-0026's narrower call moves a 2300 Hz station to 500 Hz for a 500 Hz *connect request*,
  never for a probe, and a 500 Hz station never answers a 2300 Hz call (a 2300 Hz signal on a
  500 Hz calling frequency is nobody's choice).
* The only word of it was the engine's `ignored` event, in the log.

Yet everything needed to get out was already on the air. PROBE and PROBE_ACK go on the tone floor
(ADR-0016), and the floor's frames are the same 400 Hz frames on both airs (ADR-0013) — each
station decoded the other's probes, which is how it could log them. Every beacon (ADR-0024),
call, answer, probe and probe answer states its sender's bandwidth in its capability byte.

## 2. Decision

1. **A probe is answered across bandwidths** (engine, model first). The bandwidth test goes;
   an answer to a probe stating another bandwidth goes on the floor whatever family the station
   last heard, since the floor is the one family both airs share. A probe is a question, not a
   session, and "you hear me, but we run different bandwidths" is the answer the prober needs.
   The prober's `ProbeResult` carries the answerer's bandwidth (`bandwidth_hz`, from the
   PROBE_ACK's capability byte), and both stations' events name the difference:
   `probe:WC4Y hears us at 3 dB, heard at 2.1 dB — runs 2300 Hz, this station 500 Hz` and
   `probed:KK4ODA-1 at 2.4 dB — runs 500 Hz, this station 2300 Hz`. A call across bandwidths is
   still ignored — a session lives in one — and its event now names both too.
2. **A 2300 Hz station calling a station it knows runs 500 Hz moves to 500 Hz first**
   (`Station::connect_as` → `move_for_call`, `Why::Calling`, `status.bandwidth.callee`). It
   knows from any frame that states a bandwidth — a beacon, a call, an answer, a probe or a
   probe's answer — and from the stations-heard list, which keeps each station's `bandwidth_hz`
   (serde default: an older `heard.json` reads) and seeds the station at start. A station is
   known under the callsign its frames carried first; failing that, under its other names —
   every callsign heard with the same base (`base_callsign`: no SSID, nothing after it, no
   portable prefix or suffix), provided they all agree. `VarAC` beacons as `KK4ODA-9` and
   pings `KK4ODA-1-T` while the operator calls `KK4ODA-1`; one operator running two stations
   in two bandwidths disagrees, and the call then goes out in the station's own. The move is
   ADR-0026's `move_to`, and the move back is a call's: after the session or the call's end,
   once the channel has been quiet for `RETURN_QUIET_S`. A station of this version would have
   moved on its own on hearing the 500 Hz call; one of an earlier version would not, and this is
   what reaches it.
3. **A call or probe this station cannot answer as a call is said where the operator looks**
   (`note_mismatch`, `Mismatch`, `status.bandwidth.mismatch`, the `mismatch` event and log
   line, the panel's banner on every tab). Only the one crossing that cannot be made counts: a
   wider call or probe to a narrower station (a 2300 Hz station answers 500 Hz calls by moving).
   The sentence names the station, both bandwidths and the fix — Setup step 4, or the host
   program's `BW` command when a host program chose the bandwidth, or asking the other station
   to call at 500 Hz. Said once a minute per station and kind, not once a try; shown only while
   the station still runs the bandwidth it heard it in; the panel's *Dismiss* hides it until a
   newer one.
4. **The Stations list shows each station's bandwidth**, marked when it differs from the
   station's own, with a tooltip saying what that means for a call either way.

## 3. What was not done

* **A 500 Hz station moving up to answer a 2300 Hz call.** ADR-0026's reason stands: the
  operator or the host program chose 500 Hz, most often for a 500 Hz calling frequency. The
  warning says what to do instead.
* **Moving to answer a probe.** Not needed: the floor frames are the same on both airs.
* **A wire change.** None is needed. An earlier station still ignores a cross-bandwidth probe
  (and is still reached by a known-500 Hz call, item 2); a probe answered by an earlier station
  of the same bandwidth carries the same bits as before.
* **Matching a base callsign whose names disagree.** Item 2 matches the other names of a
  callsign only when every one heard runs the same bandwidth; a guess between two is not made.

## 4. Consequences

* Tests that fail before the fix, in both suites: `test_a_probe_across_bandwidths_is_answered_
  and_names_the_mismatch` / `a_probe_across_bandwidths_is_answered_and_names_the_mismatch`;
  ADR-0006's "not answered … in another bandwidth" tests now assert the answer, on the floor,
  with the mismatch named (the target changed; this ADR is why). The daemon:
  `a_wide_station_probes_a_narrow_one_and_calls_it_at_500_hz`, the mismatch asserted in
  `a_narrow_station_does_not_answer_a_wide_call`; `two_daemons.rs`
  `a_narrow_and_a_wide_daemon_probe_each_other_and_connect`.
* `FrameReport.bandwidth_hz` (the `frame` event) is now set for calls, answers, probes and probe
  answers as well as beacons.
* A host program sees the move of item 2 as ADR-0026's: a `bandwidth` event, and
  `CONNECTED <mycall> <remote> 500`.
