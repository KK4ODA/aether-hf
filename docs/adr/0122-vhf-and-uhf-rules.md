# ADR-0122: The regulatory gate learns 2 m, 1.25 m and 70 cm

**Status:** proposed, 2026-11-14. This is the Aether FM proposal's FM-10
(`docs/research/aether-fm/00-proposal.md` §L). The rule text is the e-CFR's, read on 2026-11-14
from the version of 2026-11-12 (title 47 last amended that day; Part 97 last amended by 91 FR
1431, 14 January 2026). It is built with the FM air in step 10.4. §4's three readings were
confirmed by the author, as the licensee, on 2026-11-14.

## 1. Context

Nothing is keyed without the regulatory gate's leave (ADR-0018). Its rules are data
(`core/aetherd/data/regulatory/us-fcc-part97.json`), and they stop at 6 m. Aether FM needs the
bands it will use, read from the rule text as ADR-0018 read HF's, and an occupancy model of an FM
signal, since the gate judges what a transmission occupies.

## 2. What the rules say, quoted where they decide something

**Bands and privileges (§97.301(a)).** A Technician or higher in ITU Region 2 has:
- 2 m: 144–148 MHz;
- 1.25 m: 219–220 and 222–225 MHz;
- 70 cm: 420–450 MHz.

The sharing requirements are §97.303's (a), (b), (k), (l) and (m).

**Emission types (§97.305(c)):**

| Band | Frequencies | Emission types | Standards, §97.307(f) |
|---|---|---|---|
| 2 m | 144.1–148.0 MHz | MCW, phone, image, RTTY, data, test | (2), (5), (8) |
| 1.25 m | 219–220 MHz | Data | (13) |
| 1.25 m | 222–225 MHz | RTTY, data, test, MCW, phone, SS, image | (2), (6), (8) |
| 70 cm | Entire band | MCW, phone, image, RTTY, data, SS, test | (6), (8) |

**The standards that decide Aether FM's signal:**
- **§97.307(f)(2):** "No non-phone emission shall exceed the bandwidth of a communications
  quality phone emission of the same modulation type."
- **§97.307(f)(5)** (2 m): a specified digital code at no more than 19.6 kilobauds, or an
  unspecified code under §97.309(b). "The authorized bandwidth is 20 kHz."
- **§97.307(f)(6)** (222–225 MHz, 70 cm): the same at 56 kilobauds. "The authorized bandwidth is
  100 kHz."
- **§97.307(f)(8):** "A RTTY or data emission having designators with A, B, C, D, E, F, G, H, J or
  R as the first symbol; 1, 2, 7, 9 or X as the second symbol; and D or W as the third symbol is
  also authorized."

**Automatic control (§97.221(b)):** "A station may be automatically controlled while transmitting
a RTTY or data emission on the 6 m or shorter wavelength bands". So all of these bands are open to
an automatically controlled station, with no 500 Hz limit (§97.221(c) is for other frequencies).

**Power (§97.313):**
- 1.5 kW PEP (b);
- 25 W PEP on 1.25 m for a Novice (d);
- 50 W PEP on 219–220 MHz (h);
- 50 W PEP on 70 cm from the areas of §2.106(c)(270)(i), unless the FCC authorizes more (f).

## 3. Decision

### 3.1 The data file gains the three bands

- 144.1–148.0, 222–225 and 420–450 MHz, as data segments with their standards. 219–220 MHz is
  left out: it is a data-only segment with sharing rules of its own (§97.303(l)), not a place for
  a general modem.
- Each segment's limit is its authorized bandwidth: 20 kHz on 2 m, 100 kHz above.
- Every new value is read from the e-CFR text, as the 6 m values were, and the file's
  `rules_as_of` moves to the day of the reading.

### 3.2 Aether FM's emission is F2D, authorized by §97.307(f)(8)

An audio modem through an FM transmitter is frequency modulation (F) of a modulating sub-carrier
(2) carrying data (D): F2D.

Part 97's definition of *data* (§97.3(c)(2)) names designators with 1 as the second symbol. F2D is
therefore authorized on these bands by §97.307(f)(8), which every one of them cites, not by that
definition. The gate records the emission as F2D, and the rule it rests on as (f)(8).

### 3.3 The FM occupancy model

The gate judges the occupied bandwidth by §97.3(a)(8)'s 26 dB measure, the wider reading as on HF
(ADR-0018). An FM signal's bandwidth depends on its deviation, which the station sets only
indirectly (ADR-0117 §2.1). So:
- **Measured, not computed.** `tools/make_occupancy.py` gains FM. For each profile and rung, the
  channel model's transmitter (ADR-0115) is driven at the calibration ceiling, and the RF spectrum
  is measured by the 26 dB rule.
- **The deviation ceiling.** On the voice path the radio's limiter bounds the deviation. On a flat
  port, the operator's one-time deviation check (ADR-0117 §2.5) does. The gate judges at that
  ceiling.
- **§97.307(f)(2)'s comparison** is with a communications-quality F3E emission on the same channel:
  16 kHz on a 25 kHz channel (Carson's bandwidth at ±5 kHz and 3 kHz audio), 11 kHz on a 12.5 kHz
  one (±2.5 kHz). Aether FM must not exceed it.

### 3.4 What the gate checks on FM

The same as on HF:
- the band and segment;
- the control operator's privileges;
- the emission authorized there;
- the occupied bandwidth against the segment's limit and §3.3's comparison;
- automatic control's segments;
- the power where the radio reports it.

## 4. Readings, confirmed by the author (2026-11-14)

1. **§97.307(f)(2)'s "same modulation type"** is read as FM phone on the same channel spacing. A
   flat-port signal at 4 000 Bd, ±3 kHz deviation, is measured against 16 kHz. If the occupancy
   measure shows it over, the flat profile's symbol rate or deviation comes down.
2. **(f)(5)'s symbol rate** is read as the modem's symbol rate (2 000–4 800 Bd), far under
   19.6 kilobauds. It does not matter either way.
3. **219–220 MHz is left out** (§3.1).

## 5. What this removes and adds

It removes nothing. It adds the three bands to the US data file, FM to the occupancy tool, and the
gate's FM checks. Other countries' profiles are not written until someone asks for one.

## 6. Criteria

- The data file's VHF/UHF values match the e-CFR text, item by item, in a test that names the
  paragraph for each, as the 6 m values' test does.
- The occupancy table for FM is regenerated by the tool and held to a fresh run, as HF's is
  (`test_occupancy.py`).
- The gate refuses a 2 m transmission on 144.0–144.1 MHz (CW only) and outside the band.
