# ADR-0123: On FM a station identifies with its callsign in the digital signal

**Status:** proposed, 2026-11-14. This is the Aether FM proposal's FM-11
(`docs/research/aether-fm/00-proposal.md` §L), with the author's decision 4 (the callsign in the
digital signal, no Morse). The rule text is the e-CFR's of 2026-11-12, read on 2026-11-14. §3's
regulatory reading was confirmed by the author, as the licensee, on 2026-11-14.

## 1. The rule

§97.119(a): "Each amateur station … must transmit its assigned call sign on its transmitting
channel at the end of each communication, and at least every 10 minutes during a communication,
for the purpose of clearly making the source of the transmissions from the station known to those
receiving the transmissions."

§97.119(b) gives four ways:
1. "By a CW emission. When keyed by an automatic device used only for identification, the speed
   must not exceed 20 words per minute";
2. "By a phone emission in the English language";
3. "By a RTTY emission using a specified digital code when all or part of the communications are
   transmitted by a RTTY or data emission";
4. by an image emission.

§97.309(a) names the specified codes, among them "(3) The 7-unit, International Alphabet No. 5,
code defined in ITU-T Recommendation T.50 (commonly known as 'ASCII')". It adds: "(4) An amateur
station transmitting a RTTY or data emission using a digital code specified in this paragraph may
use any technique whose technical characteristics have been documented publicly".

Two definitions matter on FM (§97.3(c)):
- **CW** is designators with A, C, H, J or R first. A Morse tone through an FM transmitter (F2A) is
  therefore not CW.
- **MCW** "for the purpose of performing the station identification procedure" is listed under
  *Phone*.

## 2. Decision

### 2.1 The callsign goes in every transmission

Every Aether FM transmission's header carries the sending station's callsign in ASCII (ITU-T
T.50), as written, its SSID or indicator included (ADR-0119 §2.3):
- **It identifies at the end of each communication:** the last transmission carries it.
- **It identifies at least every 10 minutes:** every transmission carries it, and a transmission is
  at most 20 s long.
- **It serves other purposes too:** a third station's reservation (ADR-0120) and a listener's
  decoder can name the sender without the session's state.

The cost is about ten bytes a transmission. FM-7's measurements put it at under 1 % of a
transmission's airtime above the lowest rungs. The gate checks that every keyed FM transmission
carries the callsign of the station it is licensed to, before keying.

### 2.2 No Morse, no voice identifier

By decision 4. Each session is 5–7 s shorter than with a Morse identifier at each end (ADR-0075's
closing), and nothing beeps on a channel others may be listening to by voice.

## 3. The reading, confirmed by the author (2026-11-14)

**The reading.** A callsign in ASCII, a specified code (§97.309(a)(3)), inside a data emission
whose format is published (`docs/spec/` will hold Aether FM's air interface, as
`air-interface.md` holds HF's; §97.309(a)(4)), identifies the station "by a RTTY emission using a
specified digital code when all or part of the communications are transmitted by a RTTY or data
emission" (§97.119(b)(3)).

**Why it is a reading.** Part 97 defines *RTTY* as telegraphy for automatic reception, with B as
the third symbol (§97.3(c)(7)). Aether FM's emission is data (D third, F2D, ADR-0122 §3.2). The
header that carries the callsign is the same emission, so the identification is, strictly, in a
data emission, not a separate RTTY one.

**Why it is the usual one.** It is how packet radio has identified on VHF for decades: the
callsign in the AX.25 address field, in the same emission as the data.

**If the author reads it otherwise,** the alternatives in the rule text are these, and each costs
something:
- **An MCW identifier:** a Morse tone through the FM transmitter, which §97.3(c)(5) counts as
  phone for identification. It is §97.119(b)(2)'s "phone emission in the English language" only
  under a further reading, and it is what decision 4 turned down.
- **A spoken identifier:** synthesized speech, §97.119(b)(2) plainly. It is on the voice path only,
  and costs about 2 s at each end and every 10 minutes.

## 4. What this removes and adds

- **Removes, on FM:** the Morse identifier and its timing rules (ADR-0055, ADR-0075). They stay on
  HF, where a US callsign always identifies in Morse at the end.
- **Adds:** the header's callsign field, the gate's check of §2.1, and the manual's line: *Aether
  FM sends your callsign in every transmission; no Morse is sent.*

## 5. Criteria

- On the harness, every keyed FM transmission carries the sender's licensed callsign, decodable by
  a listener with only the published specification (a test decodes it with the model alone).
- No FM session ends without a transmission carrying each station's callsign after its last data.
