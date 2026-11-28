# ADR-0141: The radio's own transmitting holds what the station starts by itself

**Status:** accepted, 2026-11-28 (RadioPost's design 07, the author's decision 19: built with the
beacons, before repeating beacons ship). The daemon: **no wire change, no configuration key, no
schema bump.** Tested against stand-ins; owed on the FTDX10 and the TS-480 by hand.

## 1. Context

A beacon on a timer (RadioPost's client, Aether's **Repeat**) keys the radio when it falls due.
If its operator is on the microphone at that moment, the radio is already transmitting: its
receiver is muted, so the busy detector hears a quiet channel, and Aether keys the PTT into the
operator's voice. The same holds for a call or a probe that the panel queued a moment before.
The rules allow it (the operator is at the station), and it is still wrong: the beacon goes out
under someone else's transmission, the operator's.

A radio on CAT can say whether it is transmitting, whoever keyed it.

## 2. Decision

### 2.1 Reading the radio

`Ptt::radio_transmitting` asks the radio and `Ptt::reads_transmit` says whether a backend can:

| Backend | Question | Answer |
|---|---|---|
| Yaesu CAT | `TX;` | `TX0;` receiving, `TX1;` (CAT's key) or `TX2;` (the microphone, MOX) transmitting (FTDX10 CAT manual) |
| Kenwood CAT | `IF;` | P7, the digit ten characters before the end of the 35 after `IF`: 0 receiving, 1 transmitting |
| Icom CI-V | `1C 00` | data `00` receiving, `01` transmitting |
| rigctld | `t` | `0` receiving, anything else transmitting |
| FLRig | `rig.get_ptt` | 0 receiving, 1 transmitting |

A keying line (RTS, DTR), a GPIO pin, VOX and a host program's `PTT` cannot ask.

The radio is asked in the background on the keying thread (`ThreadedPtt`'s readings, `Asked.transmit`,
`Readings.transmitting`), twice a second (`TRANSMIT_EVERY_S`), whenever the station is not
transmitting and no session is up: idle, calling or probing. A reading is never on the port when a
key is wanted: the thread serves the run loop's requests first, and reads nothing while keyed.

### 2.2 Holding

A beacon, a call or a probe this station starts by itself (`held_for_busy`) waits while the radio
says it is transmitting: a fresh answer (`TRANSMIT_FRESH_S`, 2 s) taken at least `OWN_RELEASE_S`
(0.5 s) after the station's own last release, so the station's own transmission ending is not
taken for the operator's. It waits as for a busy channel: a call's timers move with it, a
program's beacon is dropped when its window runs out (ADR-0139). The log says *the radio is
transmitting* once a hold. An answer older than 2 s holds nothing: a radio that stopped answering
does not stop the station.

What belongs to an exchange already under way (an answer in a slot, a session's transmissions) is
not held: under granted slots a held answer misses its slot (ADR-0069), and in a session the radio
is only keyed by this station.

### 2.3 A keying that cannot ask

The first beacon, call or probe on a keying that cannot ask writes one log line: *… cannot ask the
radio whether it is already transmitting: a beacon, a call or a probe goes whatever is keyed at
the radio*.

## 3. Alternatives considered

- **Reading the audio.** The receiver is muted while the radio transmits, which is the problem.
- **Asking at the moment of keying.** A CAT round trip on the run loop is what ADR-0071's thread
  took off it; asked in the background the answer is at most half a second old.
- **Holding answers too.** A slot missed is a forfeit (ADR-0069 §2.3); the operator who keys over
  a session already has the session's own evidence of it.

## 4. Consequences

- The keying thread carries one more question every half second while idle. A Yaesu or a Kenwood
  answers in 10–20 ms at 38 400 Bd; an Icom's CI-V at 9 600 Bd in about 50 ms.
- Tests: `the_radios_own_transmitting_is_asked_and_read_in_each_dialect`,
  `rigctld_and_flrig_say_whether_the_radio_transmits` (ptt), and
  `a_beacon_waits_while_the_operator_transmits_at_the_radio` (station).
- Owed: the FTDX10 (Yaesu CAT) and the TS-480 (Kenwood) by hand, the operator on the microphone
  while a beacon falls due.
