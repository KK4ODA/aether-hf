# Where to operate, and how not to be a nuisance

**Status: a proposal, not a standard.** Nobody can assign frequencies to an amateur mode by
writing a document. What follows is guidance for choosing one, the reasoning behind it, and a
set of suggested calling frequencies that become real only if operators agree to use them.
`COMMUNITY-CONCERNS.md` §11 is the concern this answers: a new mode "burning over active
VARA HF" is how it earns a bad name in a fortnight.

---

## 1. The rules that are actually rules

These come first and override everything below.

* **Your licence and your national band plan.** Bandwidth limits, permitted segments and
  power limits differ by country and by licence class. Aether's widest signal occupies about
  2.5 kHz (measured by the 26 dB rule of §97.3(a)(8)), which is wider than some segments
  allow.
* **In the United States**, 47 CFR §97.307(f) governs what may be transmitted where, and
  §97.309(a)(4) requires that a digital code be publicly documented — which is why
  `docs/spec/air-interface.md` exists and is public. §97.221 governs a station under
  **automatic control** — a gateway, or a station left listening for calls with nobody at
  it — and confines one to the sub-bands in §3 below unless it is answering, at 500 Hz or
  less. Under the US profile the daemon judges every transmission against these rules before
  it keys the radio and refuses what they do not allow
  ([fcc-regulatory-controls.md](fcc-regulatory-controls.md)); you remain the control
  operator.
* **IARU Region 1, 2 and 3 band plans** each designate segments for data modes of this
  bandwidth. Use them.

Nothing in this document overrides any of that.

---

## 2. The principle

**Aether is a new mode on a crowded band, so it moves.** Not the other way round.

A station that finds itself sharing a frequency with an established mode should change
frequency, not wait the other station out. There are more of them and they were there first,
and the goodwill of the people already using the band is worth more than any particular
frequency.

Concretely:

* **Stay clear of known VARA and ARDOP calling frequencies and of the Winlink RMS channel
  lists.** Those are published and busy. A gateway parked on one will be heard as
  interference by people who have no way to decode what it is.
* **Stay clear of the FT8, FT4 and JS8 segments.** They are narrow, extremely busy, and their
  users are the least able to work around a 2.3 kHz signal sitting on top of them.
* **Stay clear of the digital calling frequencies your band plan names** unless calling is
  exactly what you are doing.

---

## 3. Suggested calling frequencies

These are proposals. They are **dial frequencies for upper sideband**, with the modem's
widest signal occupying 240–2760 Hz above the dial (the 500 Hz air's 1140–1850 Hz; measured,
[fcc-regulatory-controls.md](fcc-regulatory-controls.md)), and every one of them is chosen so that
the whole signal sits inside the segment the FCC allows an **automatically controlled digital
station** (47 CFR §97.221(b)) — because a station left listening for calls with nobody at
it is under automatic control the moment it answers one, and a frequency that is legal to
leave the station on is legal to sit at as well. Those segments are inside the RTTY/data
segments of the Region 2 band plan; check your own national plan, which may differ.

| Band | §97.221(b) segment | Suggested dial (USB) | Signal occupies | Notes |
|---|---|---|---|---|
| 80 m | 3.585–3.600 MHz | **3.590 MHz** | 3.5902–3.5928 | Region 2. 80 m allocations vary more than any other band; check yours. |
| 40 m | 7.100–7.105 MHz | **7.101 MHz** | 7.1012–7.1038 | The segment is 5 kHz wide: a dial from 7.0998 to 7.1022 MHz fits the widest signal with the default 50 Hz margin, and 7.101 is its middle. Away from FT8 (7.074) and JS8 (7.078). |
| 30 m | 10.140–10.150 MHz | **10.141 MHz** at 500 Hz | 10.1421–10.1429 | The IARU plans mark 30 m for narrow modes of 500 Hz or less, so 10.141 is a 500 Hz dial (`[radio] bandwidth = 500`). 2.3 kHz signals are nonetheless in use on the band, Winlink gateways among them; what your own plan and licence allow is yours to check. |
| 20 m | 14.0950–14.0995 and 14.1005–14.112 MHz | **14.107 MHz** | 14.1072–14.1098 | The band most likely to be usable for a first contact. The gap at 14.0995–14.1005 protects the International Beacon Project on 14.100; stay above it. |
| 17 m | 18.105–18.110 MHz | **18.107 MHz** | 18.1072–18.1098 | 18.106 or lower runs into JS8 (18.104); 18.108 spills past 18.110. |
| 15 m | 21.090–21.100 MHz | **21.094 MHz** | 21.0942–21.0968 | |
| 12 m | 24.925–24.930 MHz | **24.926 MHz** | 24.9262–24.9288 | JS8 (24.922) ends just below the segment. |
| 10 m | 28.120–28.189 MHz | **28.126 MHz** | 28.1262–28.1288 | Wide open when the band is open at all. Keep clear of FT4 at 28.180. |
| 6 m | all of 6 m where data goes, 50.1–54.0 MHz | **50.690 MHz** | 50.6902–50.6928 | Inside the band plan's non-voice area (50.6–50.8 MHz), clear of the 50.62 MHz digital (packet) calling frequency and below the radio-control channels at 50.8–51.0. FM packet channel plans vary by area: listen first. Every class from Technician up holds 6 m. |

Segments from the ARRL's summary of §97.221 ([arrl.org/link-remote-control](http://www.arrl.org/link-remote-control)),
checked against the e-CFR text of 2026-09-23; verify against the current text of the rule
before relying on it. The panel offers every one of these dials in the Session tab's dial list
(a radio keyed over CAT or `rigctld` is tuned to it with *Tune*).

**Before you transmit on any of these, listen.** The busy detector will refuse to start a
session on an occupied channel, but it cannot tell you that the frequency is somebody's net
every Tuesday.

### The 500 Hz allowance, and why Aether does not use it

§97.221(c) lets an automatically controlled station transmit **outside** those segments
only if its bandwidth is 500 Hz or less **and it is responding to a station under local or
remote control** — that is, somebody called it. The 500 Hz waveform was designed with that
allowance in mind, but measured by the 26 dB rule of §97.3(a)(8) its signal is wider than its
name — 690–710 Hz for the OFDM rungs, 610 Hz for the four-tone rungs, 551 Hz for the tone
floor, by the wider of the rule's two readings — so the daemon treats every Aether signal as
more than 500 Hz, and **an automatically controlled Aether station transmits only inside the
§97.221(b) sub-bands above, or on 6 m**, at either bandwidth.

`[radio] answer_only = true` (Setup → Modem settings → *answer only*) is still the setting
for a station left listening: it takes calls and refuses to make one, to beacon, to probe or
to send a KISS datagram (it answers probes, which is a response). A repeating beacon needs a
control operator whatever this says: the daemon refuses one under automatic control
(§97.203(d) allows an automatically controlled beacon only in a few segments, 28.20–28.30 MHz
the only one on HF).

If the mode gets enough operators for these to matter, they should be agreed publicly and
recorded here with whoever agreed them — not asserted by the software's authors.

---

## 4. What the modem does on its own

These are on by default and should stay that way.

* **Busy detection.** The channel is measured two ways. Its *level* is compared with a
  noise floor the detector learns — the median of the last ten seconds' quiet, steady blocks,
  held while a signal stays, since an HF noise floor moves by tens of dB between bands and
  hours — and is busy when it stands `busy_threshold_db` (6 dB) above it for half of the last
  400 ms. Its *shape* catches what a receiver's AGC hides from the level: a narrowband peak
  well above the rest of the passband (FT8, CW, PSK, RTTY, a voice) marks it busy, as long as
  the peak stands near the learned floor — a station's own faint spur does not. A station will
  not *start* a session, a probe or a beacon on an occupied channel. One already in session
  answers regardless — the peer is waiting for that acknowledgement, and staying silent would
  only make it retransmit into the same channel.
* **A decoded frame marks the channel busy outright** — it is there whatever the level says.
  A merely *acquired* preamble does not: on a real band acquisition false-alarms many times a
  minute, and only a decode is evidence a phantom cannot produce.
* **It does not key over the other station's identifier.** At a session's end each station
  waits out the other's Morse ID (up to 15 s) before its own last frames and ID (ADR-0022).
* **A key-time watchdog**, 30 seconds by default. It bounds a stuck transmitter, and the trip
  latches so a runaway cannot re-key by asking again. Every burst is sized to finish inside it,
  the Morse ID included (ADR-0017), so on a working station the watchdog never fires.
* **The rules before the key.** Under the US profile every transmission — a burst, a tune
  tone, a Morse ID — is judged against Part 97 at the dial the radio is on, and what the rules
  do not allow is not sent (ADR-0018).

### What it does not do yet

**The busy detector does not recognise VARA or ARDOP specifically.** It measures the
passband's level and shape and knows the Aether frames it decodes. A signal well above the
noise floor reads as busy whatever it is, and a narrowband one even behind an AGC, which
covers the cases that matter — but a wideband signal is as flat as noise, so a *weak* VARA
station, below the level threshold and invisible to Aether's acquisition, will not be seen. Recognising other modes' preambles is an
open item (`COMMUNITY-CONCERNS.md` §11), and until it is done, **listen before you call.**

---

## 5. Automatic and unattended operation

A gateway transmits without anybody watching, and most jurisdictions regulate that separately:

* In the United States, automatically controlled digital stations are restricted to the
  §97.221(b) sub-bands in §3 above — outside them only at 500 Hz or less, and only to answer
  (§97.221(c)), which no Aether signal is narrow enough for — and the control operator
  remains responsible for everything the station transmits. Tell the daemon: Setup step 1's
  *Control* is **Automatic** (`[regulatory] control = "automatic"`) whenever nobody is at the
  radio; it is never worked out for you.
* Many national plans require a station under automatic control to be in a designated segment
  regardless of bandwidth.

"Unattended" includes the ordinary case of leaving the daemon listening so friends can try
to reach you: it answers a call to its callsign whether or not you are in the room, and with
`[record] auto = true` it records each session, from connect to disconnect, with the
standing `[record] notes` and — under CAT or `rigctld` keying — the dial frequency written
into the sidecar. Park it on one of the frequencies in §3 and it is where the rules allow it
to answer.

If you are running a gateway, find the rule that applies to you before you leave it running.
The software will not know.
