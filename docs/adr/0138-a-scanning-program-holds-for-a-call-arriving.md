# ADR-0138: A scanning program holds for a call arriving, and a call heard across its deaf step is not answered

**Status:** accepted, 2026-11-25 (the author's go after KK4ODA-1's sessions of that evening). The
station and the VARA adapter: **no wire change.** Amended 2026-11-26 (§6, the author's go): the
rules apply only with `[host] scans` on, a configuration key (schema 16); §7 is a scanning VarAC on
the harness.

## 1. Context

On 2026-11-25 the mobile (KK4ODA-2, Winlink Express as `WD5EMA`, TS-480) called the gateway
(KK4ODA-1, RMS Trimode 1.4.4.2, FTDX10) at 2300 Hz on 3.590 MHz, four times, beta.178 at both
ends. Trimode keys the gateway's radio itself (`[ptt] kind = "host"`) and **scans two channels**:
its main log moved the FTDX10 between 3590.0 and 7101.0 kHz dial (channels 3591.5 and 7102.5) every
4.4 s all evening, 752 and 692 times, each move inside the half second it turns `LISTEN` off.
Aether cannot read that dial: Trimode holds the CAT port.

The fourth call (22:02:39) left Trimode connected to nobody:

| Time (UTC) | What happened |
|---|---|
| 22:02:42.5 | The mobile's second try (OFDM, 1.3 s). The gateway decodes it at 22:02:43.86 as Trimode turns `LISTEN` off and moves to 40 m: not answered ("the host program has not said LISTEN ON"), rightly. |
| 22:02:45.6 | Third try: the radio is on 40 m, nothing heard. |
| 22:02:49.2–54.8 | Fourth try, a tone frame (5.4 s). Its first 2.6 s arrive on 80 m; at 22:02:52 Trimode moves to 7101 kHz. |
| 22:02:54.95 | The gateway decodes the call from what it heard on 80 m (two clean sync blocks, ADR-0049/0070) at 8.2 dB. Trimode is listening again, on 40 m, so the station answers. |
| 22:02:55–59 | The adapter says `PENDING`; Trimode holds its scanner for 120 s on 40 m and keys the FTDX10 there for the acceptance (`TX1;` with `FA007101000` set at 22:02:52). Trimode logs *Vara 2300 connect from WD5EMA on 7102.50 KHz*. |
| 22:02:59–22:03:59 | The mobile's tries 5–8, on 80 m, unheard. 22:04:05.9: *no answer*. |
| 22:04:44 | An abort from the gateway's own panel (Trimode sent none) ends its side; 22:05:09 Trimode: *No PTT from Vara. Connection appears to be hung*. |

The three calls before it show the same scan: two tries decoded as Trimode moved to 40 m were
refused, and most of the tries the gateway never decoded were sent while the radio was on 40 m.
The sessions that worked (21:54, 22:00) had their decoded tries inside an 80 m dwell.

Two things let it happen. A tone try (5.4 s) is longer than Trimode's 4.4 s on a channel, and the
receiver decodes it from part of the frame, so a call heard on one dial can be answered after the
radio has moved to another. And the adapter told Trimode `PENDING` only once the call was taken
(ADR-0096), at the end of the frame: by then Trimode had already moved.

## 2. Decision

### 2.1 A call heard across the program's deaf step is not answered

Each claim keeps when its answering last came on (`Claim::listening_since`, the station's clock).
A call or probe to a callsign a program holds is answered only if the program listened through
the whole frame: a frame that began before its last `LISTEN ON` (less 0.25 s for the adapter's
tell to reach the station, `LISTEN_SLACK_S`) is not (`Station::listened_throughout`, in
`unanswered`), and the log says *the host program stopped listening while it arrived, and may have
moved the radio*. The caller's next try comes. A callsign no program holds is the station's own,
and nothing moves its radio.

### 2.2 `PENDING` while a frame that may be a call arrives

The station keeps each claim's last four listening windows, `LISTEN ON` to `LISTEN OFF`
(`ScanBeat`), all but the first, which begins wherever the program's scan happened to be when it
said `LISTEN ON`. A program with one such window scans, and its shortest is its beat.

While the station is idle, a frame announced at the confidence a scan takes for a try under way
(`NOTICE_CONFIDENCE`, 1.1, ADR-0129; ahead of the 1.3 acquisition gate, so a weak OFDM try counts
too), as long as a DATA frame of its family (a call or a probe; headers, answers and acceptances
are shorter) and begun while the host program listened, is a frame that may be a call
(`Station::note_call_arriving`). A program that scans is told of it
(`Station::advance_call_arriving`, the `arriving` event `{on: true}`, not logged) once it has
listened 0.3 s (`ScanBeat::SETTLE_S`: RMS Trimode ignores a line said just after its own
`LISTEN ON`), unless it is in the last quarter of its window or the quarter second before it
(`ScanBeat::steps_soon`, `STEP_MARGIN_S`). There a `PENDING` meets the `LISTEN OFF` its scan has
decided (ADR-0100 §4), and a `LISTEN OFF` already said reaches the station a pass later. From then
the station counts the program as holding (`held_since`) until it hears that nothing came of the
frames, `{on: false}` a second after the last of them ends (`ARRIVAL_REPORT_S`), or turns its
answering off.

The VARA adapter turns `{on: true}` into `PENDING` (when no call or session of the program's is
under way and it listens) and Trimode holds its scanner: the radio stays on the dial the call is
coming in on. If the station takes the call, that `PENDING` is the call's, and the program hears
`PTT ON` and `CONNECTED` as before (ADR-0103) with no second `PENDING`. `{on: false}` is
`CANCELPENDING`, and the program scans on. If the program's `LISTEN OFF` crosses the `PENDING`, the
adapter says `CANCELPENDING` at once, and §2.1 keeps the station from answering the frame. A program
that does not scan (Winlink Express, Pat and VarAC say `LISTEN` once) is told nothing.

### 2.3 A call is not taken just before the program steps

A call or probe to a program that scans, decoded in the last quarter of its window when it is not
holding, is not answered (`Station::answered_before_its_step`): the `PENDING` said as the call is
taken would cross the program's step, and the acceptance would go out after the move. The first
build had no such rule; on the harness's two-band gateway a both-ways try was read at its weak OFDM
part (announced under 1.1, so nobody was told), 3.0 s into a 3.9 s window, its `PENDING` crossed
Trimode's step, and the acceptance went out after the tone part, on 40 m (4 of 12 draws).

### 2.4 What it removes, and what it does not change

It removes nothing: the `PENDING` said when a call is taken (ADR-0096), the holds for a listening
window (ADR-0051, ADR-0100) and the order of `PENDING`, `PTT ON`, `CONNECTED` (ADR-0103) stand, and
are what a call the program was not told of in time still goes through. Nothing on the air changes.
A gateway whose Trimode scans one channel pays for §2.1 and §2.3 with a try now and then, since the
station cannot tell a step that moves the radio from one that does not; `80m-trimode-one-band-host-keyed-2300`
measures it.

## 3. The harness

The channel server takes `move WHO HZ`: another program moved the radio over a CAT link of its own,
and the daemon is not told (`knob` tells it). Commands are read every tenth of a second of air, not
every second, and only when the file grew. The runner's RMS Trimode stand-in takes `dials=` (it
moves the radio to the next in each deaf step, also when its `LISTEN OFF` crossed a `PENDING`, the
pessimistic reading of what Trimode does) and `dwell=` (3.9 s: the field's 4.4 s cycle). A station
in a scenario takes `ptt = "host"`. Scenarios:

* `80m-trimode-scans-two-bands-2300`: the evening's gateway and client, the gateway hearing the
  client at 2 dB and the client it at 9, *Call tries* 6.
* `80m-trimode-scans-two-bands-500`: the same at 500 Hz.
* `80m-trimode-scans-two-bands-busy-2300`: a third Aether station on the 40 m dial calls an absent
  station for 120 s before each of the client's calls, with RTTY and a VARA-class ARQ signal at the
  gateway.
* `80m-trimode-scans-three-bands-2300` and `80m-trimode-scans-four-bands-2300`: three channels
  (80, 40, 20 m) and four (80, 40, 30, 20 m), the gateway on the client's dial a third and a
  quarter of the time (the author's question, 2026-11-25).
* `80m-trimode-scans-four-channels-one-twice-2300`: four channels with the client's frequency
  listed twice (80, 40, 80, 20 m): a step between the two 80 m channels is deaf and moves nothing.
* `80m-trimode-one-band-host-keyed-2300`: the gateway with one channel, which never moves the
  radio: what §2 costs a one-channel gateway.

Each scenario places two calls, as the mobile did: the second after a session, when the client
has heard the gateway and opens with three short OFDM tries.

## 4. Measured on the harness

Each scenario on beta.178's daemon (before) and on this one (after), the same draws (`--seeds`),
three at a time on a four-core container, 2026-11-26. *Wrong dial* is the channel server's dial
invariant: an acceptance on a dial with no call on it, or after the station moved since the call
ended. *Left connected* is a station left in a session nobody is in.

| Scenario | Before: passed | wrong dial | left connected | After: passed | wrong dial | left connected |
|---|---|---|---|---|---|---|
| two channels, 2300 Hz | 2 / 12 | 10 | 1 | 10 / 12 | 0 | 1 |
| two channels, 500 Hz | 3 / 8 | 5 | 1 | 6 / 8 | 0 | 0 |
| two channels, busy bands | 4 / 8 | 0 | 2 | 6 / 8 | 0 | 0 |
| three channels | 0 / 8 | 4 | 4 | 8 / 8 | 0 | 0 |
| four channels | 0 / 8 | 4 | 4 | 3 / 8 | 0 | 0 |
| four channels, 80 m twice | 0 / 8 | 7 | 1 | 8 / 8 | 0 | 0 |
| one channel | 12 / 12 | 0 | 0 | 11 / 12 | 0 | 0 |

The before arm accepted on a dial with no call 30 times in 64 draws, the after arm never. What
the after arm still fails is a call that did not connect: the caller's tries ended unheard, or
heard across a deaf step and refused, before one fell where the gateway could answer it. With four
channels the gateway is on the caller's dial a quarter of the time and hears 3 to 5 of the 10 to 12
tries of two calls; with the caller's *Call tries* at 15 (`connect_within_s` 400) the same scenario
passed 8 of 8, its calls 20 to 215 s, and the scenario is committed that way (the table's row is
its six-try draws). The one-channel miss is the harness's: daemon b could not
bind its data port (*Address already in use*) and the run never began. The *left connected* after
the change is the fall-back to 500 Hz (§5).

The busy-band scenario is judged on the client's calls and messages and the invariants; its third
station calls nobody and never ends a session (its first runs had asked for a clean end of every
station and failed both arms on it; the counts above are those runs judged as the scenario now
says). The stand-in's check that a `CONNECTED` follows the acceptance's `PTT ON` failed seven
baseline one-channel draws whose sessions carried their traffic: a program that keys the radio is
told the acceptance's keying as it keys, which can be just before the `PENDING` (`host-interfaces.md`
§2.2), and the check now allows a `PTT ON` in the 10 s before the `CONNECTED` for a station with
`ptt = "host"`. The table is from runs judged that way.

The existing host-program scenarios on this daemon, three draws each (`40m-gateway-busy-when-called-2300`,
`40m-gateway-three-clients-2300`, `40m-trimode-gateway-2300`, `40m-wle-abort-and-recall-2300`,
`40m-wle-program-callsign-2300`, `80m-gateway-fq-hangup-2300`, `80m-trimode-client-vanishes-500`,
`80m-varac-chat-and-ping-500`, `quick-awgn-2300`): 24 of 27 passed. `80m-trimode-client-vanishes-500`
failed two draws that beta.178's daemon fails the same way (its second call does not connect within
the budget: the nightly's edge-of-budget scenario). `40m-gateway-busy-when-called-2300` failed one
draw that beta.178 passes: the second client, calling with the default two tries while the gateway
was in another session, had one try left when the gateway was free, and it was read late in the
one-channel Trimode's window and refused (§2.3). That is what the change costs a gateway whose
Trimode scans one channel: the same scenario with the client's default two tries
(`80m-trimode-one-band-host-keyed-2300` less its `call_tries`), eight draws each, passed 8 of 8 on
both daemons, and on this one 2 of the 8 first calls took 44 s where they had taken 11 (a refused
try, then the next).

Four builds failed the harness before this one held, each for a reason now in §2:

1. The early `PENDING` alone, its timing the adapter's (§2.1, §2.2 without the station's beat):
   7 of 12 on two channels. A weak OFDM try (decoded at -2 dB) was announced under 1.1 or not at
   all; the call was taken late in a window, its `PENDING` crossed the step, and the acceptance
   went after the move (§2.3).
2. With §2.3: 10 of 12. The beat was the shortest window, and the first, partial, was 0.8 s: at
   3 s into a 3.9 s window the station took the program to have stopped stepping.
3. Skipping the first window, two needed: 6 of 12. The call came in the third window, before
   the station knew the program scanned. One is enough.
4. 11 of 12 on two channels, but on four one acceptance on 40 m: after a hold ended, the beat
   ran from the last `LISTEN ON` 10 s before; Trimode restarts its scan at `CANCELPENDING`.

## 5. Open

* **Real Trimode.** The stand-in holds its scan on `PENDING` and restarts it on `CANCELPENDING`,
  as RMS Trimode was seen to on the bench (2026-11-19) and at KK4ODA-1. Not yet seen: a Trimode
  told `PENDING` for a frame that turns out not to be a call, many times an evening, and a Trimode
  scanning channels on the Trimode bench (its radio is *Manual* there, and it moves nothing). The
  gateway's next evening on the air, with Trimode's main and VARA logs, is the check.
* **The fall-back to 500 Hz.** A 2300 Hz call unanswered after four tries goes on at 500 Hz
  (ADR-0058, ADR-0102). With more tries refused, a gateway sometimes accepts the fourth try at
  2300 Hz after the caller has moved its next to 500 Hz; the caller does not hear the
  acceptance, and the gateway waits in a session nobody is in until its link timeout (one draw of
  `80m-trimode-scans-two-bands-2300`). The rule is ADR-0058's; it is not changed here.
* **A one-channel Trimode** paid for §2.1 and §2.3 with a try now and then (§4): the station
  cannot tell a step that moves the radio from one that does not. §6 makes the rules a setting.

## 6. Scans channels (2026-11-26)

The author's go: the rules of §2 apply only to a program the operator says moves the radio between
channels as it scans. `[host] scans` (Setup step 5's **Scans channels**, off by default, live, a
portable setting, so in profiles) is the station's `program_scans`. Off, no frame arriving is told
(`note_call_arriving` returns at once), and a call is neither refused for having begun before the
program's last `LISTEN ON` nor for coming late in one of its windows (`heard_on_its_channel` asks
§2.1 and §2.3 only when it is on): a program cycling `LISTEN` on one channel is treated as it was
before this ADR. The beat is still learned either way; it costs nothing. Configuration schema 16
(`scanning_program`, a step that changes nothing; fixture `0.2.0-beta.179-trimode.toml`); the
shell's schema history names beta.180. The harness writes `[host] scans` from a scenario station's
`host_scans` for a schema-16 daemon; the six scenarios whose Trimode scans several channels set it.

Off by default: the operator knows how many channels Trimode scans, and nothing on the VARA
interface says it, so a one-channel gateway keeps its calls as they were. A gateway
that scans several channels with the box clear is answered on the wrong band as before this ADR,
which is what the two-channel scenario with the box clear measures below. The program finder
(ADR-0133) could tick it from Trimode's channel list; Trimode's file for that has not been read.

Measured on this build, the switch's runs (`--seeds`, three or more draws by seed):

| Scenario | Scans channels | Passed | Notes |
|---|---|---|---|
| the six that scan several channels (two, two at 500 Hz, two on busy bands, three, four, four with 80 m twice), 4 draws each | on | 24 of 24 | no acceptance on a dial with no call |
| `80m-trimode-scans-two-bands-2300` | off | 0 of 4 | every draw accepted on 7101 kHz with no call there, as beta.178 |
| `80m-trimode-one-band-host-keyed-2300` with the client's two tries, 8 draws | off | 8 of 8 | first calls 8.1–11.1 s, as beta.178 (§4: 2 of 8 took 44 s with the rules on) |
| `40m-gateway-busy-when-called-2300` and `80m-trimode-one-band-host-keyed-2300`, 3 draws each | off | 6 of 6 | the draw §4 lost passes |

The station tests run both ways: `a_program_on_one_channel_answers_a_call_heard_across_its_step`
(off) fails when the switch is taken out of `heard_on_its_channel`; the §2 tests set it on.

## 7. A scanning VarAC, on the harness only (2026-11-26)

VarAC 15.0.18 added a Frequency Scanner: a plan holds, for each frequency, how long to stay, during
which hours, whether to beacon and whether to tune, and VarAC moves the radio over its own CAT. Its
release notes and announcement say nothing of what it tells the modem when it moves, or what keeps
it on a frequency. The harness's VarAC stand-in takes both as options (`session_matrix.py`'s `Host`):
`moves=silent` (nothing said) or `moves=listen` (`LISTEN` off for the half second it moves, as
Trimode), and `holds=none` (only a session), `pending` (a `PENDING` until its `CONNECTED` or
`CANCELPENDING`) or `busy` (while the modem says `BUSY ON`). The step `host-calls N` makes N calls,
each 5–40 s after the last is over, the gap drawn from the seed.

Seven cases, three draws each, on this build: two VarAC stations at 500 Hz on Good at 6 dB, one
scanning 7.105 and 14.105 MHz with 30 s on each and keying its own radio, the other on 7.105 MHz
making ten calls; 30 calls a case.

| VarAC moves | VarAC holds for | Scans channels | Answered | Answered on the wrong band |
|---|---|---|---|---|
| silently | a session only | off | 22 of 30 | 3 |
| silently | `PENDING` | off | 30 of 30 | 0 |
| silently | `BUSY ON` | off | 29 of 30 | 0 |
| with `LISTEN` | a session only | off | 24 of 30 | 3 |
| with `LISTEN` | a session only | on | 14 of 30 | 0 |
| with `LISTEN` | `PENDING` | off | 29 of 30 | 0 |
| with `LISTEN` | `PENDING` | on | 14 of 30 | 0 |

A silent mover gives the station no beat, so the box changes nothing for it, and those cases ran
with it off. Every wrong-band answer was the same: the call's first try was both ways (ADR-0126),
the station took it at its OFDM part and said `PENDING`, and the acceptance keyed only after the
5.4 s tone part ended; a VarAC that does not hold for `PENDING` moved in those seconds (draw
`s1081` of the silent case: `PENDING` at 61.5 s, the move at 65.7 s, `PTT ON` at 69.1 s).

The box costs a VarAC-like scanner half its calls because two of §2's rules fit Trimode and not a
long stay. After a `PENDING` that came to nothing the station takes the next step as unknown until
the program's next `LISTEN ON`; Trimode restarts its scan 1.5 s after `CANCELPENDING`, a scanner on
a 30 s timer steps up to 30 s later, and every call meanwhile is refused (22 of the 28 refusal
lines in the six runs with the box on; the log says one a minute for a caller). And the last quarter of a 30 s window is 7.5 s of refusals where an answer
needs about a second. Neither is changed: VarAC's behaviour is not known, and Trimode's is measured.
For VarAC the box stays clear (the manual and the panel say so); with it clear, a VarAC that holds
for `PENDING` or `BUSY ON` lost nothing here, and one that holds only for a session was answered on
the wrong band in 3 of 30 calls.

What would settle it is on the bench: a scratch VarAC whose rig control is Hamlib's dummy rig
(`rigctld -m 1`) and whose modem is the stand-in that logs every line (`vara_standin.py`), with a
scanning plan of two frequencies. The two logs side by side say whether VarAC tells the modem
anything when it moves; an injected `PENDING` or `BUSY ON` before a move says whether either keeps it.
