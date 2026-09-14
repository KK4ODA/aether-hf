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
  `docs/spec/air-interface.md` exists and is public.
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

These are proposals. They are chosen to sit inside the wide-data segments of the major band
plans and away from the busiest established users, and they are **dial frequencies for upper
sideband**, with the modem's 2.3 kHz occupying roughly 300–2600 Hz above the dial.

| Band | Suggested dial (USB) | Notes |
|---|---|---|
| 80 m | 3.586 MHz | Region 2. Check your national plan; 80 m allocations vary more than any other band. |
| 40 m | 7.104 MHz | Away from the FT8 and JS8 segments and from the common VARA channels. |
| 30 m | — | **Not recommended.** 30 m is narrow, shared with other services, and many band plans limit it to 500 Hz or restrict automatic operation. |
| 20 m | 14.107 MHz | The band most likely to be usable for a first contact. |
| 17 m | 18.108 MHz | Quiet, and a good place to test without bothering anybody. |
| 15 m | 21.107 MHz | |
| 10 m | 28.126 MHz | Wide open when the band is open at all. |

**Before you transmit on any of these, listen.** The busy detector will refuse to start a
session on an occupied channel, but it cannot tell you that the frequency is somebody's net
every Tuesday.

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

* In the United States, automatically controlled digital stations are restricted to particular
  sub-bands by §97.221, and the control operator remains responsible for everything the
  station transmits.
* Many national plans require a station under automatic control to be in a designated segment
  regardless of bandwidth.

If you are running a gateway, find the rule that applies to you before you leave it running.
The software will not know.
