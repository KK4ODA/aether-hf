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
  power limits differ by country and by licence class. Aether occupies 2.3 kHz, which is
  wider than some segments allow.
* **In the United States**, 47 CFR §97.307(f) governs what may be transmitted where, and
  §97.309(a)(4) requires that a digital code be publicly documented — which is why
  `docs/spec/air-interface.md` exists and is public. §97.221 governs a station under
  **automatic control** — a gateway, or a station left listening for calls with nobody at
  it — and confines one to the sub-bands in §3 below unless it is answering, at 500 Hz or
  less.
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
2.3 kHz occupying roughly 300–2600 Hz above the dial, and every one of them is chosen so that
the whole signal sits inside the segment the FCC allows an **automatically controlled digital
station** (47 CFR §97.221(b)) — because a station left listening for calls with nobody at
it is under automatic control the moment it answers one, and a frequency that is legal to
leave the station on is legal to sit at as well. Those segments are inside the RTTY/data
segments of the Region 2 band plan; check your own national plan, which may differ.

| Band | §97.221(b) segment | Suggested dial (USB) | Signal occupies | Notes |
|---|---|---|---|---|
| 80 m | 3.585–3.600 MHz | **3.590 MHz** | 3.5903–3.5926 | Region 2. 80 m allocations vary more than any other band; check yours. |
| 40 m | 7.100–7.105 MHz | **7.101 MHz** | 7.1013–7.1036 | The segment is 5 kHz wide: 7.101 is the only dial that fits a 2.3 kHz signal with margin. Away from FT8 (7.074) and JS8 (7.078). |
| 30 m | 10.140–10.150 MHz | — | — | **Not at 2.3 kHz.** The IARU plans limit 30 m to 500 Hz; the 500 Hz waveform (`[radio] bandwidth = 500`) is what 30 m calls for, and 10.141 is its place. |
| 20 m | 14.0950–14.0995 and 14.1005–14.112 MHz | **14.107 MHz** | 14.1073–14.1096 | The band most likely to be usable for a first contact. The gap at 14.0995–14.1005 protects the International Beacon Project on 14.100; stay above it. |
| 17 m | 18.105–18.110 MHz | **18.107 MHz** | 18.1073–18.1096 | 18.106 or lower runs into JS8 (18.104); 18.108 spills past 18.110. |
| 15 m | 21.090–21.100 MHz | **21.094 MHz** | 21.0943–21.0966 | |
| 12 m | 24.925–24.930 MHz | **24.926 MHz** | 24.9263–24.9286 | JS8 (24.922) ends just below the segment. |
| 10 m | 28.120–28.189 MHz | **28.126 MHz** | 28.1263–28.1286 | Wide open when the band is open at all. Keep clear of FT4 at 28.180. |

Segments from the ARRL's summary of §97.221 ([arrl.org/link-remote-control](http://www.arrl.org/link-remote-control));
verify against the current text of the rule before relying on it.

**Before you transmit on any of these, listen.** The busy detector will refuse to start a
session on an occupied channel, but it cannot tell you that the frequency is somebody's net
every Tuesday.

### The 500 Hz caveat

§97.221(c) lets an automatically controlled station transmit **outside** those segments
only if its bandwidth is 500 Hz or less **and it is responding to a station under local or
remote control** — that is, somebody called it. An unattended station may then answer
anywhere data is permitted, but it may never call, beacon or start a session on its own.
That is a design constraint on the 500 Hz waveform rather than a footnote: an Aether
station running unattended at 500 Hz outside the segments must be answer-only, and the
daemon has a setting for exactly that — `[radio] answer_only = true` (Setup → Misc modem
settings → *answer only*), under which it takes calls and refuses to make one or to
beacon. At 2.3 kHz there is no such allowance; the segments above are the whole of it.

If the mode gets enough operators for these to matter, they should be agreed publicly and
recorded here with whoever agreed them — not asserted by the software's authors.

---

## 4. What the modem does on its own

These are on by default and should stay that way.

* **Busy detection.** The channel is measured against a noise floor the detector learns
  (minimum statistics, since an HF noise floor moves by tens of dB between bands and hours).
  A station will not *start* a session on an occupied channel. One already in session answers
  regardless — the peer is waiting for that acknowledgement, and staying silent would only
  make it retransmit into the same channel.
* **A detected preamble marks the channel busy outright**, because acquisition works far below
  the power threshold. That is what lets one Aether station hear another too weak to measure.
* **A key-time watchdog**, 30 seconds by default. It bounds a stuck transmitter, and the trip
  latches so a runaway cannot re-key by asking again.

### What it does not do yet

**The busy detector does not recognise VARA or ARDOP specifically.** It measures energy and
detects Aether preambles. A signal well above the noise floor reads as busy whatever it is,
which covers the case that matters — but a *weak* VARA station, below the power threshold and
invisible to Aether's acquisition, will not be seen. Recognising other modes' preambles is an
open item (`COMMUNITY-CONCERNS.md` §11), and until it is done, **listen before you call.**

---

## 5. Automatic and unattended operation

A gateway transmits without anybody watching, and most jurisdictions regulate that separately:

* In the United States, automatically controlled digital stations are restricted to the
  §97.221(b) sub-bands in §3 above — outside them only at 500 Hz or less, and only to answer
  (§97.221(c)) — and the control operator remains responsible for everything the station
  transmits.
* Many national plans require a station under automatic control to be in a designated segment
  regardless of bandwidth.

"Unattended" includes the ordinary case of leaving the daemon listening so friends can try
to reach you: it answers a call to its callsign whether or not you are in the room, and with
`[record] auto = true` it records each session, from connect to disconnect, with the
standing `[record] notes` and — under `rigctld` keying — the dial frequency written into
the sidecar. Park it on one of the frequencies in §3 and it is where the rules allow it to
answer.

If you are running a gateway, find the rule that applies to you before you leave it running.
The software will not know.
