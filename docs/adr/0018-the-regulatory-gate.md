# ADR-0018: The regulatory gate — nothing is keyed without the policy's leave

**Status:** accepted, 2026-09-25. Model first for the link's ceiling
(`aether_model/link/engine.py`), then the port (`aether-link`) and the daemon
(`core/aetherd/src/regulatory/`, the station's gate). Configuration schema 6. The frames and the
link protocol are unchanged.

## 1. Context

Aether transmitted wherever the radio was tuned, at whatever rung the link climbed to, under
whatever control the station happened to be. The operator is responsible for every emission
(47 CFR §97.103, §97.105) — and an unattended station, an operator new to data modes, or a dial
turned in the middle of a session can put a signal where the rules do not allow it without
anybody noticing. The rules that matter to a data modem are few and precise:

* data (RTTY and data emissions) only in the data segments of each band, inside the control
  operator's privileges (§97.301, §97.305(c), §97.307(f)(9)), and no wider than 2.8 kHz
  (§97.307(f)(3));
* the whole occupied signal, not the dial, is what must be inside a segment — on USB it sits
  above the dial, on LSB below it, mirrored;
* an automatically controlled station (§97.3(a)(6), §97.109(d)) transmitting data only inside
  the §97.221(b) sub-bands, and elsewhere only when *responding to interrogation* by a station
  under local or remote control, with a signal of 500 Hz or less (§97.221(c)), and not on the
  60 m channels;
* 60 m: four channels (a data carrier 1.5 kHz below each centre, a CW carrier on it, 2.8 kHz)
  and the 5351.5–5366.5 kHz segment, phone, RTTY, data and CW only (§97.303(h)(3),
  §97.305(c)(3)(iii), §97.307(f)(14)).

Bandwidth itself is defined in §97.3(a)(8) — "the width of a frequency band outside of which the
mean power of the transmitted signal is attenuated at least 26 dB below the mean power of the
transmitted signal within the band" — which can be read two ways: as a ratio of powers (the band
holding all but 1/398 of the power) or as a spectral level (where the density falls 26 dB below
the mean density within the band, as an analyser's "−26 dB bandwidth" reads it). For Aether's
narrow emissions the two differ by 20 %.

The rules were read from the e-CFR (the versioner API, Title 47 current to 2026-09-23, Part 97
last amended 2026-02-13): §97.3, §97.109, §97.119, §97.221, §97.301, §97.303, §97.305,
§97.307 and §97.313. The ARRL band plan is voluntary and is kept apart from them.

## 2. Decision

### 2.1 One gate, in front of the transmitter

Every transmission the station makes waits at the head of its queue (`Station::pending`) and is
judged there before it is rendered: a session's bursts and retransmissions, a call and its
answer, a probe and its answer, a beacon, the Morse identifier, the tune tone, drive bursts and
the keying test. `Policy::authorize` returns an `Authorization` — a type only the policy can
construct — or the refusal; the station keeps the leave and `playback()`, the one place in the
daemon that keys the radio, spends it (`authorized.take()`) before `ptt.key()`. Audio that reached
the playback queue by any other path is dropped and reported, and nothing is keyed. A Morse
identifier appended inside a burst's keying is judged again, as CW, before it is appended.

A refusal drops the transmission; when it was a session's, the session ends — without a DISC,
which would be judged the same way and is unlawful where the data was. The control API refuses a
call, probe, beacon, tune, drive check or keying test the rules forbid at once, with the reasoning
(`ApiError` code `regulatory`, the decision attached); the gate decides again when it is sent.

### 2.2 The rules as data

A `RegulatoryProfile` is a JSON file compiled into the daemon (`data/regulatory/<id>.json`): bands,
data segments with their citations, privileges by license class (with the emission restrictions of
§97.307(f)(9)–(10)), the 60 m channels and segment and their emissions, the automatic-control
segments and the §97.221(c) rule, power limits, the CW-identifier speed limit, the bandwidth
reading, and the band plan — marked voluntary and never a reason to refuse. `us-fcc-part97` is the
first; another administration is another file (`profile::KNOWN`). The profile covers ITU Region 2;
a station set to another region is refused, never judged against the wrong table.

The profile is chosen in the configuration (`[regulatory] profile`): `us-fcc-part97`, or `none`
— the operator's explicit statement that no profile applies and they check every transmission
themselves (logged and shown as *NO RULES*). Left empty, nothing is transmitted.

### 2.3 What a transmission occupies is measured, not named

`tools/make_occupancy.py` sends every rung of both airs and both control frames through the
model's transmit chain and records their edges under both readings of §97.3(a)(8) in
`core/aetherd/data/occupancy.json`; `model/tests/test_occupancy.py` holds the table to a fresh
measurement. The US profile decides by the **wider** reading (`bandwidth.reading = "wider"`).
Measured: the tone floor 434–445 Hz by power, 551 Hz by density; the 500 Hz air's middle tones
445/609 Hz and its OFDM rungs 557–709 Hz; the 2300 Hz air's OFDM 2303–2320/2496–2520 Hz; every
rung under 2.8 kHz either way. The Morse identifier is held to ±100 Hz of its tone and the tune
tone to ±25 Hz.

### 2.4 From the dial to the air

`rf_range(dial, sideband, audio_low, audio_high)`: USB is `dial + audio`, LSB `dial − audio`
mirrored. A transmission is lawful when `rf_low − margin ≥ segment_low` and
`rf_high + margin ≤ segment_high` for a segment the rules allow it in — the margin
(`edge_margin_hz`, 50 Hz by default, 0–1000) covering the dial's error and the radio's own chain.
The dial is read from the radio over CAT or `rigctld` just before a transmission (no older than
2 s, and a reading older than 15 s is not used); a radio that cannot report it has its dial
declared by the operator (`[regulatory] dial_hz`, the Session tab's *Dial is at*). With neither,
nothing is transmitted.

`Policy::safe_dials` computes, for any waveform, the dial ranges inside every segment the station
may use; the Diagnostics tab lists them for the widest waveform and the tone floor.

### 2.5 Control is said, never inferred

`[regulatory] control = "local" | "remote" | "automatic"`, in the settings registry, the profiles
and Setup step 1. Local and remote control follow the same rules. Nothing about the network,
the host program or `answer_only` sets it: schema 6's migration writes `automatic` for a file
that had `answer_only = true` — an unattended station, by that file's own account — and leaves
everything else to the operator. Automation inside the modem — ARQ, rate adaptation, retries,
acknowledgements, beacons the operator asked for — is not automatic control; the control mode is
how the *station* is controlled.

### 2.6 Automatic control (§97.221)

For an automatically controlled station each data transmission is judged with the direction of
the exchange it belongs to (`Direction`): `Originate` (this station began it: a call, a probe, a
beacon, every frame of a session it called), `Respond` (another station began it: the answer to a
call or a probe, every frame of a session it was called into) and `Operator` (a test at the
control point: tune, drive, a keying test, judged under local rules). Inside a (b) segment it may
do anything data may; outside, only respond, only at 500 Hz or less by the profile's reading, and
never on 60 m. Aether cannot verify that the calling station is under local or remote control, so
a session another station began is treated as interrogation — the conservative reading available
to a modem, and the one §97.221(c) was written for.

### 2.7 The rules cap the link

`LinkEngine::set_ceiling(rung)` (model first) caps every rung the engine chooses — its bursts, a
backed-off mode, the recommendation it sends, the mode it expects back — and a ceiling inside the
tone floor makes its control frames floor frames too. The station computes the ceiling
(`Policy::ceiling`: each rung judged with its control frame and every slower rung) at session
start, twice a second while the dial or settings may move, and whenever they change; negotiation
never raises it, and an automatic station outside the (b) segments is never upgraded to the
2300 Hz air's wide rungs. The gate stays the backstop.

### 2.8 A change in the middle of a session

A dial turned, a sideband or control mode changed: the ceiling is recomputed at once, the next
transmission is judged at the new facts, and a refused one ends the session as in §2.1 —
logged, with no illegal DISC. The session banner and the log say why.

### 2.9 60 m

On a channel, a data emission's carrier (the dial, USB) must be within 10 Hz (Aether's
tolerance) of 1.5 kHz below the centre and the whole signal inside the 2.8 kHz channel; a CW
emission's carrier on the centre; the tune tone is not an emission the band allows. In the
segment, 2.8 kHz and 9.15 W ERP (a warning when the configured power is higher; ERP includes
antenna gain, which Aether does not know). No automatic operation anywhere on 60 m: §97.221(c)
excepts the channels, and whether it reaches the segment is ambiguous.

### 2.10 License classes

Technician/Novice, General, Advanced and Amateur Extra privileges from §97.301 for Region 2, with
the CW-only segments (§97.307(f)(9)) and 10 m's CW/RTTY/data and CW/phone parts. The class is
never assumed: unset, nothing is transmitted.

### 2.11 Guidance, not refusal

With `band_plan = true` (the default) a lawful transmission outside the ARRL band plan's digital
areas, or on one of its "avoid" areas (the NCDXF/IARU beacons at 14.100 MHz, 10 m beacons), is a
**warning**, never a block; so is a configured power above a limit the rules set (200 W PEP on
30 m and for Technicians, 9.15 W / 100 W ERP on 60 m). Everything else the gate allows is
**legal**. The verdict is shown as LEGAL / WARNING / TX BLOCKED beside the dial on every tab
(`status.regulatory.indicator`, the verdict for the station's widest transmission), its reasoning
one click away.

### 2.12 Failing closed

No profile, a profile that does not read, no control mode, no license class, no sideband, no
dial, a region the profile does not cover, a transmission whose spectrum is not measured: each is
a refusal with its reason, and the station stays on receive. A simulated channel (`[sim]`) keys no
radio and is judged only if its file names a profile.

### 2.13 Every decision is logged

Every refusal — and, with `log_permitted = true`, every permitted transmission of an
automatically controlled station — is one structured line in the daemon's log (`decision`,
`rule`, `code`, `callsign`, `dial_hz`, `dial_source`, `sideband`, `rf_hz`, `bandwidth_hz`, `mode`,
`emission`, `control`, `direction`, `session`, `license`, `reason`) and a `regulatory` event with
the whole decision. The recording of a session carries its refusals.

## 3. Alternatives considered

* **Check at the control API only.** Every path that queues a transmission would need its own
  check, and the engine queues most of them itself (retries, acknowledgements, the DISC). One
  gate where everything converges, with a token only the policy makes, cannot be walked around.
* **Mode names and nominal bandwidths.** The "500 Hz" air's OFDM measures 560–710 Hz; a table of
  names would have let an automatic station answer at 700 Hz outside the (b) segments.
* **The power reading of §97.3(a)(8).** It admits the tone floor (434–445 Hz) as a 500 Hz answer.
  The rule can be read either way, so the wider reading decides; a profile can say otherwise.
* **Inferring automatic control** from a host program, the network or `answer_only`. The rules
  depend on how the station is actually controlled, which only the operator knows.

## 4. Consequences

* An updated station transmits nothing until Setup step 1 says which rules apply, how it is
  controlled and the license class — the panel says so in a banner on every tab. A station keyed
  over a serial line or VOX has its dial declared on the Session tab.
* An automatic station under the US profile is confined to the (b) segments: no Aether emission
  is 500 Hz or less by the wider reading.
* Near a segment edge the link is held to the rungs that fit (the indicator reads WARNING and says
  which), rather than refused.
* The panel gained the verdict badge and its reasoning, the Diagnostics tab's rules card (the
  situation, the ceiling, the last decision, the safe dials), the declared dial, and Setup's rules
  fields; the control API gained `status.regulatory`, `regulatory.check` and
  `regulatory.profile`, the `regulatory` event and the `regulatory` error code.
* `docs/user/fcc-regulatory-controls.md` explains it to operators. Aether's checks do not replace
  the control operator's responsibility for the station.

## 5. The bypass audit (2026-09-25)

Every path in the daemon that can assert PTT, send modem audio, start a test, start or answer a
session, or send an identifier was traced:

* **Asserting PTT.** Each backend keys only in its `Ptt::key` — serial RTS/DTR (`SerialPtt::set`),
  CAT `TX1;`/`TX;`/CI-V `1C 00 01` (`CatPtt`), `rigctld` `T 1`, CM108 GPIO (`GpioPtt`). `Ptt::key`
  is called only by the key-time watchdog (`Watchdog::key`), and the watchdog's only runtime
  caller is `Station::playback`, behind `authorized.take()`. A dry run and `[sim]` use `NullPtt`.
* **Modem audio.** The playback queue is filled only by `render` (frames, drive bursts),
  `render_identifier`, `render_audio` (tune tone, keying test) — all reached only from
  `start_pending` after `gate()` — and `append_cw_id`, which now judges the identifier itself.
  Audio in the queue without leave is dropped (`the_keying_path_refuses_audio_the_gate_did_not_see`).
* **Starting a test.** `test.start` checks `check_originate` first, the ladder stops at the
  ceiling, and every frame of the Test is a session frame through the gate.
* **Starting or answering a session.** `connect`, `probe` and `beacon` are refused early by the
  control API and judged again at the gate; answers to calls and probes are engine frames through
  the gate with the `Respond` direction.
* **Identifiers.** A standalone identifier (`Outgoing::Identifier`, the end of a session) is
  gated as CW; one appended to a burst is judged as CW before it is appended
  (`an_identifier_inside_a_burst_is_judged_as_the_cw_it_is`); an automatic identifier is sent no
  faster than the profile's 20 wpm.
* **Everything else.** The host interface is a client of the control API; `frequency.set` tunes
  the radio and transmits nothing, and is refused in a session or while keyed; shutdown only
  unkeys.

`every_way_to_the_transmitter_passes_the_gate` holds the paths to it: with no profile chosen, a
call, a beacon, a probe, the tune tone, a keying test, drive bursts and a Morse identifier are
each judged, and none keys the radio.
