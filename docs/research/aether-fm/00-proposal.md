# Aether FM — engineering proposal

*Written 2026-11-13 at the author's request: research into Aether FM, an FM sibling of Aether HF that is a first-class transport for RadioPost, with the architectural alternatives and a recommendation. The author answered its eleven decisions the same day (see Decisions), and the text follows those answers. Its numbers come from a scratch model ([01-measurements.md](01-measurements.md), scripts in [`scratch/`](scratch/)), not committed benchmarks: Phase 2 replaces them. The plan is `docs/ROADMAP.md` Phase 10.*

Build Aether FM as a second physical layer under Aether's existing link layer, daemon and control API, not as a separate modem. Use one single-carrier waveform family with frequency-domain equalisation, from π/2-BPSK to 256-QAM, with aether-fec's LDPC and HARQ. Keep ADR-0069's granted slots. Let the far station measure each transmitter's level: a level ladder in the call, then a two-level probe on every exchange. RadioPost needs no protocol change to run over it, because RPMI is already the right boundary.

Two measurements shape the plan. They come from a scratch model, not committed benchmarks ([01-measurements.md](01-measurements.md)). Through an FM radio's limiter, every waveform keeps 90 % of its throughput only inside a 3–6 dB drive window, so calibration, not the waveform, fixes the level problem. And in a typical RadioPost session the data is 7–35 % of the air time, so turnaround and the number of transmissions matter more than peak bit rate.

Nothing of Aether FM is coded yet. RadioPost's two spec changes it asked for are merged (KK4ODA/RadioPost#1 and KK4ODA/RadioPost#2).

## A. Executive recommendation

The strongest practical design is a new FM physical layer under the Aether stack that exists, with the far station measuring each transmitter's level, and RadioPost reached through RPMI as it stands.

| Layer | Recommendation | Why |
| --- | --- | --- |
| Waveform | Single carrier in blocks with a known guard word, equalised in the frequency domain (SC-FDE, the shape of IEEE 802.11ad's single-carrier PHY) | 1–1.5 dB more signal-to-noise-and-distortion (SNDR) through the radio's limiter than OFDM, at each one's best drive; keeps OFDM's per-frequency noise handling; one frame family from the call to the top rate |
| Audio profiles | Voice path: about 2 000 Bd in 450–2 850 Hz. Flat data port: about 4 000 Bd in 300–5 100 Hz. Exact numbers from Phase 2 | The radio's audio path, not the RF channel, sets the usable bandwidth |
| FEC and HARQ | `aether-fec` unchanged: NR LDPC, rate matching, incremental redundancy | Bit-exact with the model, tested, nothing FM-specific wrong with it |
| Link | ADR-0069's granted slots: the caller is master, every frame states its count | It removed the collisions that cost HF a month of patches. FM's short turnarounds suit it |
| Rate control | ADR-0099's controller (each sender chooses by measured delivery), with FM's ladder | Already the outcome of a review; an FM channel is steadier than HF |
| Calibration | A level ladder in the call and the acceptance, judged by the far station; then a ±1.5 dB two-level probe on data frames, reported in every answer; learned profiles per radio and per peer | Only the far receiver can see a station's deviation |
| Channel access | The radio's squelch (CAT, data-port SQL line) or an audio noise-quieting detector | FM says when a carrier is up. HF's busy detector (3 469 lines, under a no-more-patches rule) has no FM benefit |
| Host interfaces | The control API (RadioPost), the VARA-style TNC port (Winlink Express, Pat and RMS Packet in their VARA FM modes), KISS | Already built and benched on HF |
| RadioPost | No RadioPost protocol change. One modem-name registry entry, `aether-fm`, through RadioPost's own planned gap G11; two clarifications for FM channels | RP-3 needs only a reliable byte stream, and RP-4 already says what a modem reports |

**Legacy packet (Decision 7).** Beside its own waveform, Aether FM also speaks packet radio: 1200 baud AFSK, 9600 baud G3RUH, FX.25 and IL2P error correction, and AX.25 connected mode, for APRS, BBSs, nodes and Winlink's RMS Packet. D.6 says how.

**Expected performance** (estimates from the scratch model, to be replaced by Phase 2 curves): voice path 6–14 kb/s at the PHY on good simplex, flat data port 20–27 kb/s. A typical RadioPost session takes 7–10 s on simplex and 9–18 s through a repeater, with no Morse identifier, since FM stations identify in the digital signal (Decision 4).

### Assumptions this proposal challenges

1. **"Choose the waveform well and level sensitivity goes away."** It does not. On the voice path every waveform tested keeps 90 % of its throughput only within a 3–6 dB drive window (up to 9 dB on a flat port at strong signal). The roadmap's P10-2 idea, that constant-envelope rungs leave the limiter nothing to distort, holds for those rungs only. Calibration, not modulation, is the differentiator.
2. **"OFDM, as on HF."** Single carrier wins, but modestly (about 2 dB with pre-emphasis compensation). The larger gain is having one frame family: ADR-0068 §1 traces most of HF's timing rules to having three.
3. **"A second, constant-envelope family for the robust rungs"** (P10-2). Not needed on present evidence: low-order PSK on the same carrier still has 9–12 dB SNDR at gross overdrive, enough for QPSK ½. Phase 2 tests 4-FSK as a candidate floor and adopts it only if it wins by a set margin.
4. **"Throughput is the race with VARA FM."** For RadioPost it is not. Data is 7–35 % of a session's air time; doubling a 24 kb/s PHY saves 0.5 s of 6.5 s. Turnaround, the number of transmissions and the identifier decide the session.
5. **"Calibration is a step before each connection."** Fold it into the call, and remember the result: about 1 s each way on a first contact with a peer, nearly nothing after.
6. **"The receiver can tell overdrive from noise in one transmission."** Only weakly: error power at signal peaks rose 1.3–2 dB at the knee. A two-level slope is reliable; a single-shot classifier is not.
7. **"RadioPost integration is Phase 6."** Its needs shape the MAC now: a turn is a transmission, PDUs are small, the modem never re-links silently, `receiving` must be truthful. The adapter itself is nearly free.
8. **"Everything above the PHY is reused"** (roadmap §11). Mostly, but the daemon is not PHY-agnostic yet: about 130 references to `aether_phy` types across `aetherd`, 43 in `station.rs`. Extracting a PHY boundary, in HF first so HF's tests prove it, is the first engineering task.

## B. Aether HF reuse assessment

About four-fifths of Aether HF carries over: the FEC, the link engine, the rate controller, every host interface, keying, audio, recording, the harness and the release machinery. The physical layer is replaced, the HF busy detector and the bandwidth moves are dropped, and the daemon needs a PHY boundary it does not have yet. Line counts are from the tree at 2026-11-13.

### Physical layer (`core/aether-phy`, 11 576 lines; `model/aether_model/phy/`)

| Component | Files | Verdict | What changes |
| --- | --- | --- | --- |
| LDPC, rate matching, CRC | `core/aether-fec/src/{ldpc,rate_match,crc}.rs`; `model/aether_model/fec/nr_ldpc.py` | **Reuse unchanged** | Nothing. Redundancy versions give IR-HARQ for free |
| Constellations, BICM demapper | `aether-phy/src/constellation.rs`, `phy/constellation.py` | **Reuse, extend** | Add π/2-BPSK and 256-QAM |
| Numerology as the single source | `model/aether_model/waveform.py`, `aether-phy/src/waveform.rs`, `tools/make_spec.py` | **Reuse the discipline, replace the numbers** | A `FmWaveformParams` per audio profile; the spec's tables generated from it as now |
| OFDM modulator and receiver | `aether-phy/src/{ofdm,rx}.rs`, `phy/{ofdm,rx}.py` | **Refactor into SC-FDE** | The FFT, the MMSE weights and the per-bin noise shape (ADR-0104, `noise.rs`) carry over. Comb pilots, HF motion tracking (ADR-0082) and CFO search go |
| Preamble and acquisition | `aether-phy/src/{preamble,sync}.rs` | **Replace** | Built for HF Doppler and CFO search. FM audio has neither, only sound-card clock error: a Golay or Zadoff-Chu sync word with a known-word guard |
| Tone floor | `aether-phy/src/tone.rs` (3 259 lines) | **Do not carry over** | HF-specific (−14 dB, echo reading). Only its energy detector returns, if Phase 2 adopts a 4-FSK floor |
| Streaming receiver | `aether-phy/src/stream.rs` | **Rewrite smaller** | Keep cached correlator rows (ADR-0010 §6) and the span rule of ADR-0095. Most of its size serves three families and the floor's settle time |
| Impulse blanker | `aether-phy/src/blanker.rs` | **Modify** | FM clicks below threshold: mark symbols as erasures (LLR 0) rather than blank samples |
| Peak reduction | `aether-phy/src/papr.rs` (ADR-0004) | **Drop** | Measured: 1.3 dB at the 10⁻⁴ level on real audio. Single carrier starts 2.8 dB lower |
| Passband conversion, filters | `aether-phy/src/{passband,fir}.rs` | **Reuse, re-parameterise** | Centre frequency and bandwidth per profile |

### Link layer (`core/aether-link`, 7 728 lines; `model/aether_model/link/`)

| Component | Files | Verdict | What changes |
| --- | --- | --- | --- |
| Granted-slot engine | `aether-link/src/granted.rs` (5 110), `link/granted.py`; spec `docs/research/link-architecture/04-formal-spec.md` | **Reuse, generalise** | "floor" appears 252 times in `granted.rs`. FM has one family, so the family becomes a parameter. Re-check assumptions S1–S7 with FM's latencies; keep the TLA+ models |
| Frame formats | `aether-link/src/frames.rs` | **Reuse, new protocol number** | `GrantFrame` and the connect body carry over; the five rung bits and family bits get FM meanings; a calibration report joins the connect body |
| Rate controller | `aether-link/src/rate.rs`, `link/rate.py` (ADR-0099) | **Reuse** | New ladder and thresholds; check its prior on a steady channel |
| Datagrams | `aether-link/src/datagram.rs` (ADR-0019) | **Reuse unchanged** | Carries KISS and RPMI broadcasts |
| PHY boundary | `aether-link/src/phy.rs` (`PhyTiming`, `SoftFrame`) | **Refactor** | `floor_*`, `echo` and `follows` become generic optional properties |

### Daemon (`core/aetherd`, about 57 000 lines with its modules)

| Component | Files | Verdict | What changes |
| --- | --- | --- | --- |
| The station loop | `aetherd/src/station.rs` (13 203) | **Refactor first** | 43 direct references to `aether_phy` here, about 130 in the daemon. Extract a `Phy` trait, in HF first, so HF's suite proves it |
| Audio HAL | `aetherd/src/{audio,devices}.rs`, `AudioIo` | **Reuse** | Add the operating-system volume guard (P10-7). Capture-skip padding (ADR-0089) carries over |
| Keying | `aetherd/src/ptt.rs` (serial, CAT, CM108, rigctld, FLRig, host, none for VOX) | **Reuse, extend** | Add squelch/busy reads where the radio reports them over CAT, and a COS input line |
| Keying latency, learned gaps | `station/{latency,gaps}.rs` (ADR-0071, ADR-0054) | **Reuse** | More important on FM: VOX interfaces and repeater key-up |
| Keyed-audio watch | `station/keyed_audio.rs` | **Reuse** | Its 1–2.5 kHz band suits the voice path |
| Busy detector | `aetherd/src/busy.rs` (3 469), `busy_trace.rs` | **Replace** | Squelch from the radio, or a noise-quieting detector on the audio |
| Receive-filter reading | `aetherd/src/passband.rs` (ADR-0105) | **Modify** | Its method becomes part of calibration: FM's audio passband is the thing to measure |
| Bandwidth moves | `station/bandwidth.rs`, ADR-0026/0035/0058/0101/0102 | **Drop** | FM negotiates a profile in the call from both stations' hardware |
| Following an offset | `station/centre.rs` (ADR-0079) | **Drop** | FM audio carries no RF offset |
| Claims, host, datagrams, beacons, reserve | `station/{claims,host,datagrams,beacons,reserve}.rs` | **Reuse** | Claims are what RadioPost sharing rests on (ADR-0106–0108) |
| Test session | `station/fieldtest.rs` | **Reuse, extend** | Add the level ladder as a step (P10-5) |
| Regulatory gate | ADR-0018, `data/regulatory/us-fcc-part97.json`, `data/occupancy.json` | **Modify** | Data stops at 6 m. Add 2 m, 1.25 m and 70 cm with §97.307(f)'s symbol-rate and bandwidth limits; FM needs its own occupancy model, since `occupancy.json` measures audio on an SSB carrier |
| Morse identifier | `aetherd/src/cwid.rs` (ADR-0075) | **Reuse** | On FM the callsign goes in the digital signal (Decision 4); the Morse identifier stays for HF |
| Control API | `aetherd/src/control/`, `docs/spec/control-api.md` | **Reuse unchanged in shape** | Add calibration readings; the RadioPost adapter keeps working |
| VARA-style host port | `aetherd/src/host/{server,vara}.rs` | **Reuse** | Map VARA FM's published commands; bench Winlink Express and Pat in their VARA FM modes |
| KISS port | `aetherd/src/kiss/` | **Reuse unchanged** |  |
| Configuration, profiles | `aetherd/src/{config,profile,settings}.rs` (ADR-0011) | **Reuse** | New `[fm]` keys; learned radio profiles live beside them |
| Recording, replay, sharing, debug uploads | `aetherd/src/{record,replay,share,upload,debug}.rs` | **Reuse** | Sidecars gain calibration events |

### Tools, harness, application

| Component | Files | Verdict | What changes |
| --- | --- | --- | --- |
| HF channel model | `model/aether_model/channel.py` (Watterson) | **Replace** | An FM radio model: emphasis, limiter, IF filter, discriminator, clicks, squelch, repeater |
| Scenario harness | `tools/{channel_server,session_matrix,soak}.py`, `bench/scenarios/` (ADR-0042) | **Reuse, new channel** | The same lockstep `--channel` link; FM scenarios set each station's level wrong on purpose |
| Vector discipline | `tools/make_*_vectors.py`, `tools/check_vectors.py` | **Reuse** | Model first, port bit-exact, CI fails on drift |
| Panel and shell | `app/ui/`, `app/src-tauri/` | **Reuse, extend** | A calibration view naming the side at fault (P10-3); one shell supervising an HF and an FM daemon |
| Release pipeline | `tools/release.py`, `.github/workflows/release.yml` | **Reuse** | One version, two daemons in the bundle |

### Lessons from ADR-0068 and ADR-0069

ADR-0068 (the answer goes when the burst says) is superseded: ADR-0069 kept only its exact frame counts. The history behind both is the most useful thing HF teaches FM.

| HF lesson | For FM |
| --- | --- |
| The right to transmit was each station's private belief; no message exchange makes two stations agree (ADR-0069 §1) | **Transfers whole.** Granted slots from day one |
| Only decoded content changes protocol state; silence and announcements move no deadline (ADR-0069 §2.5) | **Transfers whole** |
| A burst must state its length (ADR-0068 §2.1) | **Transfers whole** |
| Three frame families made every timing rule have cases (ADR-0068 §1) | **Avoid the cause:** one family |
| A detector set low for the floor made phantoms routine | **Reconsider:** FM's useful SNR range is narrower and squelch exists, so the threshold can sit higher |
| Latencies were assumed, then measured (ADR-0036, 0054, 0071) | **Transfers, matters more:** VOX hang and repeater key-up are hundreds of milliseconds and vary by radio |
| A part on its third fix gets a review, not a patch | **Transfers as process** |
| The link bench's pipe misled the rate controller; the harness and the air decided (ADR-0093, rate reviews) | **Transfers:** judge FM on an FM channel with the radio in it, never on a pipe |

## C. RadioPost integration architecture

Aether FM sits below RPMI (RP-4) as one more modem, with the same control API as Aether HF, and RadioPost reaches it through its existing Aether adapter. RadioPost's protocols do not change. Read from RadioPost at `f8cee6c`: RP-3 0.11.0, RP-4 0.6.6, RP-5 0.16.0, RP-7 0.9.0, the concept, the detailed design and the wishlist.

### C.1 Where the boundary goes

RP-3 §1 asks the link for "a reliable, ordered, two-way byte stream … or that the link is reported broken", and nothing more. RP-4 §1 already gives the modem "keying the radio, choosing frequencies and modulation, retransmission and turnaround". So the boundary is fixed by the spec, and the choice is only how Aether FM meets it.

| Option | Verdict | Reason |
| --- | --- | --- |
| RPMI link through the Aether adapter (`adapters/aether`) | **Now** | The adapter maps RPMI to Aether's control API one to one (RP-4 §9.1). It hardcodes the name `Aether HF` (`bridge.rs` line 509); otherwise it reads `capabilities`, `status`, `metrics` and `state` generically |
| RPMI spoken natively by the daemon | **Next, for HF and FM together** | RP-4 §1 allows it ("a modem may speak RPMI itself") and §9.1 expects it. It removes an adapter process and the class of faults where the adapter outlived its modem (RP-4 0.6.0) |
| VARA-style TNC port (RP-4 §9.2) | **Only for VARA-only programs** | Loses delivery, sharing by claims and quality reports |
| KISS or another packet interface | **Rejected for sessions** | No reliability or delivery: RadioPost would have to grow its own ARQ, which is the redundant loop to avoid. KISS stays for packet programs and RPMI broadcasts (RP-4 §6) |
| Link-level relaying (digipeating) | **Rejected** | RadioPost relays with custody at the application layer (RP-7), and an analog repeater relays RF transparently. Two-hop ARQ over half-duplex doubles the air time and breaks granted slots' timing |

### C.2 What Aether FM guarantees to the host

- **The stream.** In order, complete, no duplicates. A break is `disconnected` with its reason, at once.
- **No silent re-linking.** An RP-3 session (its nonce and statement) belongs to one link. After a break the modem reports and stops; RadioPost calls again and resumes by content ID and offset (RP-3 §7).
- **Two acknowledgements, two facts, no duplicate loop.** `delivered` means the far modem holds the bytes; RP-3's `RECEIPT` means the far program stored them durably (RP-3 §5.4). RadioPost never retransmits over a live link and does not depend on `delivered` (concept §7). Keep it so: the modem never acknowledges on a host's behalf, and RadioPost adds no timer shorter than the modem's link timeout.
- **Timers that nest.** RP-3 ends a session after 300 s with nothing received while the modem reports nothing being received (RP-3 §8). Aether FM's link timeout must be well inside that, tens of seconds on FM, so the modem reports the break first. `receiving` is true only while a frame of this link is arriving, not while any carrier is up: a repeater's carrier is up during everyone's transmissions.
- **Flow control.** `caps.max_queue` (Aether HF says 65 536 bytes) and `queued` from `metrics.queued_bytes`. At 10 kb/s that queue is 52 s of air, ample for RP-3's PDUs.
- **A turn is a transmission.** RP-3 §3: a station writes everything it has for its turn, and the modem sends it in one go. The MAC therefore coalesces writes for a few tens of milliseconds before keying, so one RP-3 turn never splits into two transmissions.
- **Think time.** A gateway may take up to a second to have `WELCOME` ready while it asks the post office (RP-3 §9). On HF that hides inside long transmissions; on FM the slave's slot opens in about 0.4 s. So a slave whose host has received bytes and not yet answered says so in its answer, and the master grants an open window of about 2 s (ADR-0069 §2.6) instead of waiting for its 10 s keepalive. The flag is generic ("host input unanswered"), not knowledge of RP-3.
- **Calibration stays inside.** RP-4 has no member for it and RP-3 has no use for it, so the author's preference holds against the spec. RadioPost sees only `quality` (`snr_db`, `rate_bps`, `receiving`), and a failure's `text` may name a level fault in words.
- **Profiles are the modem's choice.** RPMI's `bandwidth` is the host's choice "if the modem has a choice" (RP-4 §3). An FM profile depends on both stations' hardware, so `caps.bandwidths` lists only what this radio supports, the call negotiates, and `connected.bandwidth` says which profile is in use.
- **The rest as Aether HF.** `busy` (here the squelch, the repeater's carrier included), `heard`, broadcasts as datagrams, `regulatory` (once the gate has VHF data), identification by the modem, and sharing by claims (Aether ADR-0106 to 0108).

### C.3 The scenarios

| Scenario | What exists today | What Aether FM adds or must do |
| --- | --- | --- |
| **Station to station, no Internet** | RP-3 §10: one client answers and proves itself; the post office keys held locally verify both; each reports to its post office later (RP-5 §6.6) | A link. Nothing else |
| **Client to gateway** | RP-3 sessions; an offline gateway checks credentials itself and queues (RP-3 §9, RP-7) | The think-time window above. A gateway record naming an FM channel (C.5) |
| **Through an analog repeater** | Transparent: RP-3 sees a slower turn | Aether FM's repeater profile (section G). The repeater licensee's rules apply; no unattended forwarding windows on VHF yet, since RP-7 §5.1 lists no VHF rows |
| **Connectivity lost, later regained** | RadioPost resumes by content ID and offset, keeps a gateway's pieces 24 h, re-offers its outbox (RP-3 §7, RP-7 §3.6) | Detect the loss, report `disconnected` with a reason, never re-link, drop the queue and end the claim's session. Failure to connect is `failed` `timeout`; when to retry is RadioPost's |
| **Several transports** | A gateway holds one RPMI connection per adapter and lists every channel with its modem (W26, "fits today"). A client uses one modem at a time; switching radios is planned (W44; the author's decision of 2026-11-11: switching only for now) | Honest `hello` (modem name, profiles) and honest `quality.rate_bps`, so a future selector can choose by time on the air, as W44's item (4) proposes. Automatic failover does not exist and this proposal does not invent it |

### C.4 Performance requirements from RadioPost's traffic

RP-3 §11 sizes: `HELLO` 201–361 bytes, `WELCOME` 232–356, a 960-byte `MSG` piece 991–995, `RECEIPT` 33, `DONE` 5. RP-3 §12's example session is five transmissions and about 2.5 kB. So the design optimises for short, bursty, store-and-forward exchanges with small control traffic, and secondarily for large transfers (attachments, RP-7 forwarding batches).

| Session estimate (RP-3 §12's exchange) | Session | Data alone | Data share |
| --- | --- | --- | --- |
| Simplex, voice path, 10 kb/s, 0.4 s turnaround | 7.9 s | 2.0 s | 25 % |
| The same, first contact, 1 s level ladder each way | 9.9 s | 2.0 s | 20 % |
| The same, slow radios, 0.8 s turnaround | 11.1 s | 2.0 s | 18 % |
| Data port, 24 kb/s | 6.5 s | 0.8 s | 13 % |
| Data port, a hypothetical 48 kb/s | 6.0 s | 0.4 s | 7 % |
| Repeater, every turn waits a 1.6 s key-up | 17.5 s | 2.0 s | 11 % |
| Repeater, replies inside its hang time | 8.7 s | 2.0 s | 23 % |
| Aether HF 2300 Hz, a good path, for scale | 38.8 s | 16.4 s | 42 % |

Estimates from a timing model, not measurements ([01-measurements.md](01-measurements.md)). No Morse identifier is added: FM stations identify in the digital signal (Decision 4). Two at 20 wpm would add 5–7 s to every row, more than the data.

Design targets that follow: each RP-3 turn exactly one transmission; a turnaround of 0.4 s or less on simplex, measured RF to RF; the call and acceptance only, calibration included, for a known peer; the caller's disconnect riding with its last turn (RPMI's `disconnect` drains first, RP-4 §4); bursts capped under a repeater's timeout; 85 % of the PHY rate on large transfers.

### C.5 Compatibility matrix

Classes: **1** supported by the existing specification · **2** adapter or modem only, no protocol change · **3** optional extension · **4** normative specification change.

| Feature | Class | Detail and impact on others |
| --- | --- | --- |
| Aether FM as an RPMI modem: stream, `delivered`, `queued`, `quality`, `busy`, `heard`, broadcasts, `regulatory`, `shared` | 1 | RP-4 §1–§8 as written |
| The Aether adapter driving Aether FM | 2 | Read the modem's name from `status` instead of hardcoding `Aether HF`. RP-4 §9.1 gains an informative note |
| Native RPMI in the daemon | 1 | RP-4 §1 |
| Profile choice through `bandwidth` | 1 | The modem's choice; `connected.bandwidth` reports it |
| Calibration inside the modem | 1 | Nothing exposed |
| Think-time window, write coalescing | 2 | Modem-internal |
| An RPMI "end of turn" hint on `send` | 3 | Would save about 0.05 s a turn. **Not recommended** unless Phase 4 measures more |
| A gateway record listing an FM channel, `"modem": "aether-fm"`, `"mode": "FM"` | 3 | Needs the modem-name registry RadioPost already proposes for Mercury (gap G11), with its rule that a client ignores a name it does not know: harmless to third parties. The registry entry can also say what `bandwidth_hz` means for `aether-fm`, so RP-5 itself does not change |
| The same with `"modem": "aether"`, `"mode": "FM"` | rejected | Ambiguous: Aether HF's own waveform also runs through an FM radio, and a client would call with the wrong modem |
| RP-7 windows on FM: what `occupied_hz` means | 4 | RP-7 §5 defines it as the modem's measured occupied bandwidth; no audio modem can measure FM deviation. Proposed: an FM window states the channel (carrier ± half the channel spacing). Affects only gateways that publish FM windows; none exist |
| RP-7 §5.1 rows for 2 m, 1.25 m, 70 cm | 4 | The table is normative, and the spec already says how rows are added ("with the rule cited"). §97.221(b) permits automatic control of data on 6 m and shorter |
| Calibration or PHY state reported to RadioPost | not recommended | No consumer in any RadioPost spec |
| Automatic transport selection or failover | not existing | Future RadioPost work (W26, W44); needs nothing new from the modem |

Nothing in this list is needed for a first RadioPost session over Aether FM: classes 1 and 2 suffice. Classes 3 and 4 are needed only to list FM gateways and to forward unattended on VHF.

## D. Physical layer: candidates and choice

Recommended: single carrier with frequency-domain equalisation (SC-FDE), in blocks separated by a known guard word, from π/2-BPSK to 256-QAM, coded with NR LDPC. On the voice path it keeps about 1.5–2.5 dB more SNDR through the limiter than CP-OFDM; on a flat data port the two are within about half a decibel. The deciding reasons are architectural: one frame family, headroom in the sound cards, and OFDM's frequency-domain tools kept.

### D.1 How the FM audio channel differs from HF

| Property | HF (what Aether HF was built for) | FM audio (what Aether FM sees) |
| --- | --- | --- |
| Time variation | Fading, Doppler spread to 10 Hz | Static at a fixed station; picket-fence dropouts when mobile; sound-card clock drift |
| Frequency response | Multipath delay spread of milliseconds | Fixed filters: the 300 Hz tone-squelch high-pass, a 3 kHz low-pass, emphasis mismatch; group delay at the band edges |
| Nonlinearity | Mostly linear once set against the ALC | A hard deviation limiter after pre-emphasis (voice path); IF-filter distortion when over-deviated (data port); sound-card clipping at both ends |
| Noise | Broadly flat, impulsive QRN | Above threshold: Gaussian, rising as f² before de-emphasis. Below threshold: clicks. Plus the radio's own hum floor |
| Usable SNR range | −14 to +30 dB | Roughly +10 to +30 dB; it collapses over a few dB of RF below the FM threshold |
| Frequency offset | Dial mismatch, tens of Hz | None in the audio; sound-card ppm only (0.3 Hz at 2.7 kHz for 100 ppm) |

So HF's machinery for Doppler, CFO search and a −14 dB floor has nothing to do on FM. What FM needs, a signal the limiter treats well, static equalisation and level calibration, HF never had.

### D.2 The candidates, measured through a simulated radio

The scratch model passes each waveform through pre-emphasis, a hard limiter, a splatter filter, an FM modulator, IF noise and filter, a discriminator, de-emphasis and the receive filters, then measures SNDR at each waveform's best drive ([01-measurements.md](01-measurements.md)). Voice path on a 25 kHz channel (±5 kHz).

| Candidate | Peak over RMS at 10⁻⁴, real audio | SNDR at best drive, CNR 18 / 30 dB | Flat data port, CNR 18 / 30 dB | Notes |
| --- | --- | --- | --- | --- |
| CP-OFDM, 40 Hz carriers | 11.6 dB | 23.1 / 27.3 dB | 22.2 / 29.7 dB | Aether HF's family |
| Aether HF 2300 Hz with ADR-0004 peak reduction | 10.3 dB | 24.3 / 28.6 dB | — | Its clipping helps, but it uses 2.2 kHz of the band |
| DFT-spread OFDM, no spectral shaping | 9.8 dB | 24.2 / 27.8 dB | — | SC-FDE without the roll-off |
| **Single carrier, raised-cosine 0.2, 16-QAM** | **8.8 dB** | **24.6 / 28.8 dB** | **23.1 / 30.6 dB** | The recommendation's shape |
| The same, with pre-emphasis compensated at the modem | 8.8 dB at the limiter | **25.4 / 29.5 dB** | not applicable | Chosen by calibration, per radio |
| 4-level continuous-phase FSK, 1 200 Bd | 3.0 dB | not comparable | — | Constant envelope; about 2.4 kb/s at most in the voice band |

Two checks on the receiver. A linear frequency-domain equaliser gives up 0.0–0.7 dB on the voice path and 0.7–1.2 dB on a flat port, against ideal per-frequency coding (OFDM's best case). And deep overdrive still leaves 9–12 dB SNDR on single carrier, so QPSK ½ call frames survive a level 10–15 dB too high.

### D.3 Why SC-FDE, and the alternatives turned down

- **One frame family** for calls, control, data and calibration: ADR-0068 §1 shows what three families cost HF.
- **Frequency-domain processing kept.** One FFT a block and per-bin MMSE weights equalise the steep audio filters simply. With the per-bin noise reading of ADR-0104 it also suppresses narrowband interferers: CTCSS residue on a flat port, a repeater's courtesy tone, hum harmonics.
- **Clicks stay local.** A sub-threshold click hits a few symbols, which are marked as erasures. In OFDM the same click spreads over every carrier of the symbol.
- **The guard word does two jobs.** As in IEEE 802.11ad's single-carrier PHY, a known word between blocks is both the cyclic guard and the pilot that tracks clock drift and phase.

| Alternative | Why not |
| --- | --- |
| CP-OFDM (HF's family, renumbered) | 1.5–2.5 dB worse through the limiter on the voice path; its strength, Doppler-proof equalisation, is not needed |
| OFDM with per-carrier bit loading | Recovers the equaliser's 0.7–1.2 dB on a flat port only, at the cost of loading tables in the handshake and OFDM's peaks |
| Time-domain single carrier with an adaptive decision-feedback equaliser | Works, but long training for the 300 Hz high-pass's ringing; kept as the fallback if SC-FDE's guard proves too short |
| Baseband multi-level PAM, G3RUH style | Needs a port flat to near DC; many are not. A passband carrier placed at 300 Hz takes the same band |
| GMSK or multi-level FSK for everything | About 1 bit/s/Hz: at most 2.4 kb/s in the voice band |
| A constant-envelope floor family beside SC (roadmap P10-2) | Phase 2 tests 4-FSK against π/2-BPSK at and below the FM threshold and on picket fencing; adopted only if it wins by 2 dB or more |
| 1 024-QAM | Needs 33 dB or more; the model's ceiling is 29–31 dB with a −42 dB hum floor |

### D.4 Proposed numerology (fixed in Phase 2)

| Profile | Symbol rate | Audio band | Block | Guard word | Use |
| --- | --- | --- | --- | --- | --- |
| Voice | 2 000 Bd, roll-off about 0.2 | 450–2 850 Hz | 512 symbols, 256 ms | 32 symbols, 16 ms | Microphone/speaker jacks, the emphasized 1200 pin, repeaters |
| Voice, narrow fallback | 1 600 Bd | 600–2 600 Hz | 512 symbols | 32 symbols | Radios with poor voice filters |
| Flat | 4 000 Bd | 300–5 100 Hz | 512 symbols, 128 ms | 32 symbols, 8 ms | The 9600 data pin, 25 kHz IF |

The 16 ms voice-path guard is a guess at the ringing of a 300 Hz high-pass; Phase 2 measures it (risk R3). A preamble of two repeated sync blocks (about 50 ms) opens each transmission.

### D.5 The ladder and what it delivers

SNDR thresholds are approximate (AWGN, long LDPC codewords, 10 % block error); Phase 2 replaces them with the model's curves. Net rates allow 6 % for the guard word and 4 % for headers and CRC.

| Rung | Modulation and code rate | About SNDR | Voice, 2 000 Bd | Flat, 4 000 Bd |
| --- | --- | --- | --- | --- |
| 0 | π/2-BPSK 1/5 | −5 dB | 0.36 kb/s | 0.72 kb/s |
| 1 | π/2-BPSK ½ | +1 dB | 0.90 kb/s | 1.8 kb/s |
| 2 | QPSK ½ | +2 dB | 1.8 kb/s | 3.6 kb/s |
| 3 | QPSK ¾ | +5.5 dB | 2.7 kb/s | 5.4 kb/s |
| 4 | 16-QAM ½ | +8 dB | 3.6 kb/s | 7.2 kb/s |
| 5 | 16-QAM ¾ | +12 dB | 5.4 kb/s | 10.8 kb/s |
| 6 | 64-QAM ⅔ | +16 dB | 7.2 kb/s | 14.4 kb/s |
| 7 | 64-QAM ⅚ | +19.5 dB | 9.0 kb/s | 18.0 kb/s |
| 8 | 256-QAM ¾ | +23 dB | 10.8 kb/s | 21.6 kb/s |
| 9 | 256-QAM ⅞ | +26.5 dB | 12.6 kb/s | 25.2 kb/s |

At the best drive the model gives 17.6–29.5 dB on a 25 kHz voice path, 10.7–27.1 dB on a 12.5 kHz one, and 13.1–30.6 dB on a flat port, for CNR from 9 to 30 dB. That is rungs 5–9: **5.4–12.6 kb/s on the voice path, 10.8–25.2 kb/s on a flat port.** The model is optimistic: ideal emphasis, no microphone compressor, no sound-card noise. Real radios may sit a rung or two lower.

Through a repeater, two FM hops and two limiters cost an estimated 3–6 dB (unmeasured), and repeater audio is voice-band, so the voice profile only. Large one-way transfers run at about 87 % of the PHY rate on simplex (8 s bursts) and 69 % through a repeater that makes every turn wait for key-up.

### D.6 Legacy packet beside the native waveform (Decision 7)

Aether FM also speaks packet radio, so one station serves APRS, BBSs, nodes and Winlink's RMS Packet as well as its own faster waveform. It is built from public specifications only. Dire Wolf (GPL-2.0) is a reference for interoperability, never a source of code: Aether is MIT or Apache-2.0.

| Part | What it is | Audio path | Specification |
| --- | --- | --- | --- |
| 1200 baud AFSK | Bell 202 tones (1 200 / 2 200 Hz), NRZI, HDLC framing | Voice path | AX.25 2.2 (TAPR); Bell 202 |
| 9600 baud | G3RUH: scrambled (1 + x¹² + x¹⁷) shaped baseband | Flat 9600 port | James Miller's 1988 design note |
| FX.25 | Reed-Solomon around an unchanged AX.25 frame, so plain TNCs still read it | Either | The FX.25 specification |
| IL2P | Reed-Solomon, a scrambler and a compact header; plain AX.25 cannot read it | Either | The IL2P draft 0.6 (TARPN) |
| AX.25 connected mode | Connect and disconnect, I-frames, RR, REJ and SREJ, windows and timers | Either | AX.25 2.2 |

How it fits with the rest:

- **Shared:** audio, keying, the regulatory gate, recording and the panel. Both receivers run on the same audio, so a station hears packet and Aether FM on one channel at once.
- **Its own rules on the air:** packet keeps CSMA with p-persistence and AX.25's own ARQ, not granted slots. Aether FM treats packet traffic on the channel as busy.
- **Interfaces:** the KISS port carries real AX.25 frames to packet programs, as a KISS TNC does; an AGWPE port serves connected-mode programs; RadioPost could reach AX.25 through RP-4 §9.4's AGWPE mapping.
- **Levels:** packet partners report nothing back. AFSK has a constant envelope, so it can sit just under the limiter knee that Aether FM's calibration found. 9600 follows the flat port's knee, then a one-time deviation check (G3RUH: ±3 kHz).
- **Tested** in Phase 5 against Dire Wolf, a NinoTNC and a hardware TNC, both ways, with decode counts on the WA8LMF test recordings that Dire Wolf's own results use.

## E. Automatic audio calibration

The far station does the measuring, in two stages: a level ladder inside the call and the acceptance, then a two-level probe on the data of every exchange. Each station also guards its own sound card and receive level. Results are kept per radio and per peer, so a known peer costs almost nothing. Every number below is from the scratch model and is a target for Phase 3, not a result.

### E.1 What software can and cannot control

| Quantity | Control | Observation | Aether FM's use |
| --- | --- | --- | --- |
| Digital amplitude at the modem | Direct, continuous | Exact | **The only actuator the loop moves** |
| Sound-card output gain (OS mixer) | Sometimes (WASAPI, ALSA, CoreAudio); Windows resets it after updates or a replug | Readable | Set once, then held and watched (P10-7); never a loop actuator |
| Radio microphone or data-port gain | Only over CAT, on some radios, by model-specific menus | Readable on some | A starting value from radio presets (P10-8); never moved by the loop |
| FM deviation | Indirect, through the above | **Not locally**: only at the far receiver, and only relative | The far station reports it |
| Receive audio level | Radio volume on a speaker jack; fixed on a data port; OS input gain | Exact at the ADC | Guarded: clipping runs and too-low levels named to the operator |
| Software AGC | Ours, after the ADC | Exact | Per-transmission normalisation; cannot undo clipping or quantisation |
| OS processing (enhancements, AGC, noise suppression, macOS voice processing) | Must be off | Detectable | Calibration refuses and names the setting |
| RF signal strength | No | Inferred from FM quieting and clicks; S-meter over CAT on some radios | Explains a weak path; never moves the drive |

**What no algorithm does.** It cannot recover hard clipping, inside the radio or in either sound card. It cannot measure absolute deviation, so it cannot certify occupied bandwidth: on a data port with no limiter the operator checks deviation once with a meter or a service monitor. It cannot defeat a microphone compressor or AGC inside the radio: it can only detect one and say so.

### E.2 Stage 1: the level ladder in the call

1. The caller sends its call frame on a robust rung (π/2-BPSK or QPSK, low rate), which survives a level 10–15 dB off. Then, in the same transmission, a ladder: six segments of a known training sequence, 3 dB apart, about 120 ms each (0.72 s). The sequence has the data's spectrum and peak statistics: a tone would mislead the limiter.
2. The ladder is centred on the stored best level for this radio and this peer, or on a conservative default when there is none. With a stored value, two or three segments verify it (about 0.3 s).
3. The called station measures each segment: SNDR from the known sequence, received level, noise in the empty bins, the band's response (tilt, edges, a tone-squelch high-pass), and clipping at its own ADC.
4. It takes the SNDR maximum, refined by a parabola through the top three points, and backs off 1 dB. The curve rises about 1 dB per dB below the peak and falls 2–3 dB per dB above it, so erring low costs less.
5. It reports in its acceptance: the offset (0.5 dB steps), the SNDR there, the curve's class, and a summary of the band. The acceptance carries its own ladder; the caller answers in its first header. **Both directions are calibrated with no extra transmission.**

**Each rung gets its own level.** Peaks differ by constellation: on this waveform QPSK sits 1.1 dB under 16-QAM at the 10⁻⁴ level and 64-QAM 0.3 dB over it. So each rung transmits at the calibrated level less its own peak difference, and a rung change moves the level by a known amount, not by the loop. A level found with a tone would over-drive data by 6–9 dB, which is why the ladder never uses tones.

| Curve class | What it means | Response |
| --- | --- | --- |
| Peaked | The normal case | Best level minus 1 dB |
| Rising throughout | Weak RF or a level far too low | Top step; the next call starts higher, up to the ceiling |
| Falling throughout | Overdriven everywhere | Bottom step; the next call starts lower |
| Flat | An ALC ahead of pre-emphasis (as in the TM-V71A), a compressor, an AGC, or a fixed-level path | Take the knee where SNDR stops rising; if flat throughout, keep the level and tell the operator |
| Irregular | Interference or fading during the ladder | Keep the stored value; measure again later |

Whether the modem should pre-compensate the radio's pre-emphasis is a property of the local radio, not the peer. So it is tried once per radio setup, as a step of the Test session (P10-5): one ladder flat, one compensated. The model gains 0.5–1.3 dB from it on an emphasized voice path and loses on a flat port.

### E.3 Stage 2: the closed loop during a session

1. Each transmission's data frames alternate between the level d − 1.5 dB and d + 1.5 dB. Frame parity says which, so no header bit is spent.
2. The receiver re-encodes each decoded frame, so it knows the symbols sent, and measures the frame's SNDR. Only decoded frames count, as ADR-0069 §2.5 requires.
3. Its next answer carries the difference between the two groups, Δ in 0.25 dB steps (6 bits).
4. The sender moves: d ← d + μ·Δ, with a 0.5 dB deadband, at most 1 dB down or 0.5 dB up per exchange, inside ±6 dB of the calibrated value (P10-4's window) and never above the ceiling.

The model's test: with 0.25 s of 16-QAM at each level, Δ pointed the right way in 100 % of trials for overdrive of 1.5 dB or more, and for underdrive of 6 dB or more. Within 3 dB below the optimum it was right 62–97 % of the time, where the loss is under 10 % anyway. A single transmission at one level is not enough: error power at the signal's peaks rose only 1.3–2 dB at the knee, too little to steer by.

**Why it cannot confuse a weak signal with overdrive.** Above threshold, an FM receiver's audio level is set by the deviation, not the RF strength. A weak path raises the noise, the slope is positive, and the loop raises the level, which is right: more deviation buys more FM improvement. It stops where the limiter or the far IF filter bends the curve over. The loop never needs to know the cause.

**Keeping the loops apart.**

- Time scales differ: the rate controller decides every transmission; the level moves at most 1 dB an exchange.
- At the optimum, SNDR is flat in the level (±1.5 dB costs 0.3–1.0 dB in the model), so the probe barely disturbs the rate controller, and the rate controller judges a level change as a small SNDR change.
- The level holds still while the rate controller tries a rung above, for one exchange after a rung change, and when the answer reports interference.
- HARQ copies of a frame are sent at the level of their first copy.

### E.4 What the receiver absorbs by itself

- **Any linear response** with up to about 20 dB of variation in band: per-block frequency-domain equalisation with the per-bin noise reading of ADR-0104.
- **Receive levels** from about −40 to −3 dBFS with a 16-bit ADC, normalised per transmission from the preamble.
- **Sound-card clocks** to ±200 ppm, tracked by the guard words.
- **CTCSS or DCS** on a flat port: a high-pass under 300 Hz and per-bin weighting.
- **Clicks** below threshold: detected as short wideband spikes and marked as erasures.

Not absorbed: OS noise suppression, ADC clipping, a microphone compressor. These are detected and named.

### E.5 Learned profiles

| Profile | Key | Holds | Invalidated when |
| --- | --- | --- | --- |
| Local radio | Sound card and OS endpoint, radio model (from CAT or declared), port (voice or flat) | The transmit knee in modem dBFS (median over peers), pre-emphasis choice, the OS volumes at calibration, receive level statistics, keying latency and turnaround (ADR-0054, ADR-0071) | An OS volume, the device or (over CAT) the radio's menu settings change; or the loop needs more than 4 dB from it on several peers |
| Peer | The peer's callsign and the profile | The offset from the local knee, the last SNDR, simplex or repeater, its learned answer gap | It ages out, or Stage 1 disagrees by more than 3 dB |

The split learns which part is local: the transmit knee is the same for every peer, the far receiver's part differs. These are hardware- and machine-scoped settings in ADR-0011's terms, never in a portable profile.

### E.6 Failure modes

| Failure | How it shows | Response |
| --- | --- | --- |
| OS processing on the capture or playback device | Ladder flat or wrong-shaped; residual with spectral holes | Refuse to calibrate; name the setting |
| Microphone compressor in the radio | Flat ladder | Advise the data port, or turning the compressor off |
| Receive ADC clipping | Full-scale runs (ADR-0105's clipping reading) | Tell the receiving operator; exclude those segments |
| Squelch closing on a weak signal | Segments missing | Report; the ladder starts after the call frame, never before |
| VOX or repeater key-up cutting the start | First segment short or absent | Same placement; the learned lead covers it |
| Interference or fading during the ladder | Irregular curve | Keep the stored value; mobile sessions do not update the local profile |
| Over-deviation on a data port | The far IF filter's knee | The ceiling, and a one-time deviation check by the operator |
| Limit cycling | Level wandering in a steady session | Deadband, asymmetric bounded steps, the freeze rules; tested in Phase 3 |

### E.7 Acceptance criteria for Phase 3

On the FM channel model, before any radio:

- From any starting level error in −15 to +15 dB, the call connects in 99 % of trials at CNR 12 dB or more.
- After the call and acceptance the level error is within 2 dB in 95 % of trials; after ten exchanges, within 1.5 dB.
- A session started 10 dB off takes at most 1.15 times as long as one started right. The roadmap asks only that it completes.
- In a steady channel the level's standard deviation over 50 exchanges is under 0.5 dB.
- The level never exceeds its ceiling.
- An AGC, a compressor or receive clipping is detected and named in 90 % of trials or more.

## F. MAC and link-layer architecture

Carry ADR-0069's granted slots and ADR-0099's rate controller over with FM's numbers in them, and add FM channel access. Most of the work is removal: with one frame family, the family-specific timers, floor control frames and bandwidth moves go.

### F.1 Framing

- **A transmission** is a preamble (two sync blocks, about 50 ms), a header control frame on a robust rung, and up to six data frames. Each data frame states exactly how many follow (ADR-0068 §2.1). With one family the count goes in a small header field of the frame, not in HF's chip phase.
- **A data frame** is one LDPC codeword, one to four 512-symbol blocks long. At rung 9 on the voice path a block carries about 400 bytes in 0.27 s.
- **Aggregation.** The master's transmission carries every byte its host has queued, up to the burst cap: one RP-3 turn, one transmission.
- **The connect body** names the air interface (FM), the profiles each station supports, its declared turnaround, and Stage 1's report (E.2).
- **Its own link protocol number.** An HF station and an FM station cannot meet on the air anyway; the number keeps versions apart, as ADR-0041 §4 does on HF.

### F.2 ARQ and HARQ

Unchanged from HF: selective repeat with a base and a 16-bit bitmap, IR-HARQ with the RV order 0, 0, 2, 3 (ADR-0043), soft combining, and the re-encode and split rules for stuck frames (ADR-0085, ADR-0086). On a steady FM channel a stuck frame usually means a wrong level or rung, which the two loops fix; HARQ earns its keep on mobile paths and at a rung's edge.

### F.3 Granted slots with FM's numbers

ADR-0069 §2.2: a slot starts at `end + gap`, and `gap = decode budget + keying latency + guard + the other's declared turnaround`. On FM:

| Term | HF value (measured) | FM, to be measured in Phase 4 |
| --- | --- | --- |
| Decode budget | 0.30 s (decode 0.157 s or less, research report 08) | Likely 0.1–0.15 s: one family, no floor detector |
| Keying latency | Learned per station (ADR-0071) | Learned the same way; plus the radio's transmitter attack |
| The other's turnaround | Declared in the call (`turnaround_ms`), learned from repeats (ADR-0054) | Dominant on FM: squelch opening, CTCSS decode if the far radio uses tone squelch, VOX hang on a SignaLink, repeater key-up |

- **Open windows** bound a host's think time (C.2): about 2 s while the slave's host has unanswered input, then the 10 s keepalive.
- **The burst cap** is the smaller of the key watchdog's budget and the repeater profile's limit (section G).
- **Re-check the formal model.** Run the TLA+ models (`research/link_arch/tla/`) and the review's simulator with FM's latencies, and check assumptions S1–S7 of `04-formal-spec.md` against them. FM breaks none that I can see; VOX hang longer than the declared turnaround is the case to model (S2).

### F.4 Channel access outside a session

Calls, probes, beacons and datagrams start an exchange on the station's own initiative, so they wait for a clear channel.

- **Carrier sense is the squelch.** From CAT where the radio reports it, from the data jack's squelch-status line (pin 6 on the 6-pin mini-DIN of many radios) through a serial input or a CM108 GPIO, or from the audio: silence behind a closed squelch, or the quieting of out-of-band noise on a flat port.
- **p-persistence and slot time** for calls, as KISS programs expect, and the reservation of `station/reserve.rs`: a session heard holds this station's calls.
- **Hidden stations on simplex.** In RadioPost the client calls, so it is the master and the gateway the slave. A second client may hear only the gateway's answers. So each slave answer states when the master's next transmission can start, and a third station reserves until then.
- **FM capture** often lets the stronger of two colliding calls through: no rule depends on it.
- **On a repeater** every station hears the repeater, so nothing is hidden. But its carrier stays up through its hang time, so carrier presence alone is not busy: Aether FM looks for audio activity (data, voice) as well (section G).

### F.5 Rate control

ADR-0099's controller: each sender chooses its own rung by measured delivery, frames never detected included, with an SNR prior and tries of the rung above. Two FM differences help it. Stage 1 gives it an accurate SNDR before the first data frame, so the first rung is close to right. And a fixed station's channel is steady, so it settles quickly. On mobile paths, picket-fence dropouts appear as losses, which is what the controller already counts.

### F.6 What HF has that FM drops

- The tone floor and the fast and four-tone kinds (ADR-0013, 0014, 0015, 0081): FM's floor is π/2-BPSK on the same carrier.
- Family rules: which family an answer goes in, floor control frames, the family in the gap (ADR-0016, 0034, 0059, 0066).
- Bandwidth moves (ADR-0026, 0035, 0058, 0101, 0102): FM negotiates a profile in the call.
- Offset following (ADR-0079) and echo limits (ADR-0087).
- The HF busy detector (ADR-0078, 0088): replaced by squelch and activity detection.

## G. Repeater and simplex strategies

Repeaters need a different MAC profile, not a different PHY. One waveform serves both; what changes is the lead before a transmission, the turnaround, the burst cap and politeness. Three operating profiles follow from the audio path and the RF path together.

### G.1 The published timing figures

| Item | Figure | Source |
| --- | --- | --- |
| Radio receive-to-transmit | 15–155 ms in recent ARRL Lab reviews (FTM-6000R 43 ms); an old IC-449A over 410 ms | [ARRL review of the FTM-6000R](https://www.arrl.org/files/file/ProductReviewsForDeb/2023/pr082022.pdf); [TAPR DCC 1995](https://web.tapr.org/meetings/DCC_1995/DCC1995-DataRadioTesting-VE2BMQ-N2IRZ.pdf) |
| Radio transmit-to-receive (release to audio) | 25–200 ms | ARRL Lab reviews |
| Squelch opening and closing | About 60 ms (a land-mobile base station's specification) | [Tait TB9400](https://www.manualslib.com/manual/1505674/Tait-Tb9400.html?page=25) |
| CTCSS decode | 60–300 ms (140 ms on a common decoder IC, about 300 ms on a Tait mobile) | [Tait TN-1031](https://www.manuals.repeater-builder.com/2006/TECHNOTE/TN-1031-SR_TM8100_CTCSS.pdf) |
| Repeater controller audio delay | 12–200 ms, 100 ms typical | [Link RLC-ADM](https://www.repeater-builder.com/link-rlc/pdfs/rlc_adm.pdf), [CAT DL-1000](https://www.repeater-builder.com/cat-auto/cat-archive/dl1000man.pdf) |
| Hang time | 5–10 s (one controller's defaults) | [NHRC-2.1 manual](https://www.repeater-builder.com/nhrc/manuals/current/nhrc-2.1-manual.pdf) |
| Courtesy tone | About 1 s after the input drops; four 50 ms notes, 659–1 175 Hz, on one controller | NHRC-2.1; [Link RLC-2](https://www.manualslib.com/manual/2313004/Link-Rlc-2.html?page=101) |
| Time-out timer | 180 s default, "generally 3 minutes" | NHRC-2.1 |
| Repeater key-up end to end | **No published measurement found** | Phase 5 measures it (R9) |

Echolink's GSM 06.10 and the AMBE+2 vocoder of DMR or Fusion bridges model speech, so a modem waveform cannot be expected to survive them. An AllStar link carrying µ-law might pass the lower rungs. Aether FM treats a linked path as unsupported unless the ladder shows it works.

### G.2 The three profiles

|  | Simplex, voice path | Simplex, flat port | Repeater, voice path |
| --- | --- | --- | --- |
| Audio | 2 000 Bd, 450–2 850 Hz | 4 000 Bd, 300–5 100 Hz | 2 000 Bd (or 1 600 Bd), whatever the repeater passes |
| Carrier sense | The radio's squelch | Squelch, or out-of-band noise quieting on the discriminator output | Audio activity, not carrier: the repeater's carrier stays up through its hang time |
| Lead before the first frame | Learned (radio attack plus far squelch): about 0.1–0.2 s | The same | Learned per repeater: squelch, CTCSS decode, controller delay and transmitter attack, perhaps 0.3–0.8 s. A first call uses a long lead and learns |
| Turnaround target | 0.4 s or less; modern radios suggest 0.2–0.3 s | The same | Inside the courtesy-tone delay, so no new key-up and no beep; otherwise the next key-up |
| Burst cap | 20 s (the key watchdog) | 20 s | 10 s, well under the time-out timer, leaving room for others |
| Politeness | p-persistence and reservation | The same | Off by default. When enabled: a quiet interval before a call, a break window every minute of session, and a session that pauses when non-modem audio (a voice) appears in a gap |
| Expected PHY rate (model) | 5.4–12.6 kb/s | 10.8–25.2 kb/s | Lower by an estimated 3–6 dB of SNDR: two FM hops, possibly two limiters |

### G.3 What the repeater profile has to handle

- **The hang time is an asset.** While the repeater still holds its carrier, a reply needs no key-up. A turnaround shorter than the courtesy-tone delay (about 1 s on the controllers above) avoids the beep entirely.
- **The courtesy tone, when it does sound,** is a few narrow tones. SC-FDE's per-bin noise reading weights those bins down, as it does any narrowband interferer. Its timing and frequencies are learned from what the station hears after the other's transmissions.
- **Delay lines** add 12–200 ms to every gap; learned like any answer gap (ADR-0054).
- **DTMF muting** (0.5 s on one controller) must never be triggered. The training sequence and the data are broadband, never a pair of tones.
- **Two limiters in cascade**, the user's and the repeater's. The far station's ladder sees the whole path, so the knee it finds is the path's, whichever limiter it is. Each direction is calibrated separately, so asymmetric paths are fine.
- **Tone squelch** at a receiver adds 60–300 ms before audio arrives. The guide recommends carrier squelch for data, or the unsquelched discriminator output where the radio has one.
- **Voice users.** No one can break into a data transmission, so the profile keeps transmissions short, leaves break windows, and yields when it hears a voice. Repeater owners' rules come first: the guide says to ask.

### G.4 Calibration on each profile

- **Flat ports** reach the far receiver without emphasis, so pre-compensation is off; polarity inversion, reported on several radios, is just a sign the equaliser absorbs. Port levels differ by about 20 dB between radio models, which is why calibration finds the knee and never assumes an absolute level.
- **Amateur mobiles may regulate rather than clip.** The TM-V71A's service manual places an ALC amplifier ahead of pre-emphasis and no separate clipper after it. Above the ALC's threshold the deviation stops rising and the ladder reads flat. The ladder then chooses the knee where SNDR stops rising, just under the ALC's action, since an ALC pumping inside a frame hurts dense constellations.
- **Through a repeater** the ladder is placed after the learned lead, so key-up never eats a segment.

## H. Competitive assessment

Aether FM can plausibly match VARA FM's published peak rates, not exceed them. Where it can lead is the level problem, session efficiency, openness and RadioPost. Comparisons with VARA FM rest only on its published figures, and say so: by Decision 10 there is no A/B. Labels: **[spec]** the developer's own document, **[meas]** a published measurement with a method, **[anec]** user reports and community guides. VARA appears only through its published documents and user experience, under the project's rule.

### H.1 The systems

| System | Waveform | Audio path | Peak net rate | Measured goodput | Level handling | Open |
| --- | --- | --- | --- | --- | --- | --- |
| [VARA FM 4.x](https://rosmodem.wordpress.com/) | OFDM, 42 Bd, 4PSK to 256-QAM; NARROW 14–58 carriers, WIDE to 116 [spec] | NARROW: microphone/speaker or 1200 pin; WIDE: 9600 port [anec] | 12 750 b/s NARROW, 25 210 b/s WIDE [spec] | About 12.7 kb/s at 20 dB and 4.8 kb/s at 10 dB on a white-noise audio simulator, noise bandwidth unstated [meas, KN6KB/N5TW]; users report 3–9 kb/s on HTs [anec] | Auto Tune with a partner station; Ping shows S/N and VU both ways [spec] | No; about US$69 [anec] |
| [MercuryFM](https://github.com/Guru-RF/MercuryFM) | A fork of Mercury 1.9.9; one FM mode, 16-QAM OFDM, 33 carriers, LDPC rate 0.8 [spec] | Not yet tested on radios | About 4.6 kb/s in about 2.06 kHz [spec] | Offline AWGN loopback only; nothing on the air [meas] | A `tx_gain_db` setting | GPL-3.0 |
| [pdn-soundmodem OFDM-FM](https://github.com/packet-net/pdn-soundmodem) | OFDM, BPSK to 256-QAM, convolutional and LDPC codes | Tait radios' flat taps, 211 Hz–8 kHz; a narrow mode to 2.9 kHz | 38.5 kb/s coded | 17.5–29.8 kb/s on the 8 kHz mode, 2.2 kb/s narrow, two radios on coax through 100 dB [meas] | Receive meter and clip lamp; deviation set by Bessel null [spec] | AGPL |
| [Dire Wolf](https://github.com/wb2osz/direwolf) | AFSK 1200, 2400 QPSK, 4800 8PSK, 9600 G3RUH; FX.25 and IL2P | Voice path to 2400; 9600 needs a data port | 9 600 b/s | 1 066 B/s at 9600 with large frames and windows (88 %), 128 B/s at defaults (10 %) [meas] | A receive meter | GPL-2.0 |
| [NinoTNC / IL2P](https://tarpn.net/t/nino-tnc/n9600a/n9600a_operation.html) | AFSK, BPSK/QPSK (3600 QPSK at 1 800 Bd on 1 650 Hz for the voice path), GFSK, 4FSK; Reed-Solomon | Voice path to 3600; data port above | 19 200 b/s | 122 B/s at 9600 with one frame per transmission [meas, "old"] | A clip LED | Open |
| [M17](https://m17project.org/) | 4FSK, 4 800 Bd, RRC | 9600 port | Over 4.5 kb/s for large packets [spec] | None found | Set with an SDR decoder | Open |
| [ARDOP](https://sources.debian.org/src/golang-github-la5nta-wl2k-go/0.11.9-1/docs/ardop/_ARDOP%20Specification.pdf) FM frames | 4FSK, 600 Bd | FM | About 780 b/s ARQ [spec] | None | — | Open, dormant |

FreeDV 2400B and FSK_LDPC target FM radios, but no published measurement through commodity radios was found. FreeDATA is HF-only.

### H.2 Where Aether FM stands, criterion by criterion

| Criterion | VARA FM | Aether FM as proposed | Basis |
| --- | --- | --- | --- |
| Peak throughput | 12.75 kb/s NARROW, 25.2 kb/s WIDE [spec] | 12.6 kb/s voice, 25.2 kb/s flat (scratch model) | **Parity at best.** No basis to claim more |
| Throughput at 10–20 dB | The simulator curve above | Unknown until Phase 2 | Not claimed: with no A/B (Decision 10) nothing measures it. SC-FDE's limiter margin is a reason to expect an edge, no more |
| Calibration effort | Auto Tune with a partner, plus receive level set by hand to about −11 to −13 dBFS [anec]; level setting the most-cited difficulty, about 7 sources [anec] | Automatic in every call, then held by the in-session loop; receive level guarded | **The main differentiator** (E.2, E.3) |
| Sensitivity to drive errors | Users say it "is far less tolerant of incorrectly-set audio levels" and that "most users overdrive" [anec] | Setup survives ±10–15 dB; the session corrects to within about 2 dB | E.7's criteria, to be proved |
| Robustness to distortion | Unpublished | SC-FDE keeps 1.5–2.5 dB more than OFDM through a limiter (model) | D.2 |
| Connection setup | Unpublished timing | Call and acceptance only, calibration included | F.1, E.2 |
| Low-SNR operation | Level 1: 566 b/s [spec] | Rung 0: 360 b/s voice, 720 b/s flat | Similar floors; FM's threshold limits both |
| Repeaters | "Improved" [spec]; mixed user results, turnaround and HT tail problems in about 4 reports each [anec] | A repeater profile that learns key-up, hang and courtesy tone (section G) | To be shown in Phase 5 |
| Half-duplex efficiency and latency | Unpublished | Granted slots, one transmission per RP-3 turn, a 0.4 s target turnaround | C.4 |
| Channel sharing | A Winlink sysop guide: "not CSMA… hogs the channel"; packet stations transmit over it [anec] | Squelch-based carrier sense, p-persistence, reservation (F.4) | A real difference for shared packet channels |
| Relaying | Digipeaters, one or two hops [spec] | None at the link layer; RadioPost relays with custody (RP-7) | **A gap** for Winlink users; not in the first release (Decision 5) |
| Hardware compatibility | Windows; Pat runs it under Wine [spec] | Windows, Linux, macOS, Raspberry Pi; rigctld, FLRig, CAT, CM108, serial | Aether HF's base |
| Gateway support | RMS Packet listens for VARA FM [spec, excerpt]; RMS Trimode's list does not name it [excerpt] | The VARA-style port for RMS Packet, Winlink Express and Pat; RadioPost Gateway natively | Bench in Phase 6 |
| Open interoperability | Closed; reverse engineering forbidden [spec] | Published air interface, test vectors, open source | Aether's existing practice |
| RadioPost | Through RP-4 §9.2: no delivery, no sharing | Through RPMI with delivery, quality, sharing | C.5 |

**Packet compatibility (Decision 7).** VARA FM's KISS frames are "incompatible with Packet signals" [spec]. Aether FM speaks AX.25 at 1200 and 9600 baud with FX.25 and IL2P, connected mode included, so one station serves packet users, APRS and its own faster waveform.

### H.3 The advantages worth building for

1. **Calibration that measures with data, not tones.** A tone's peak is 3 dB over its RMS; this waveform's data is 8.8 dB at the 10⁻⁴ level, CP-OFDM's 11.6 dB ([01-measurements.md](01-measurements.md)). So a level found with a tone over-drives data by 6–9 dB through a limiter. This is physics, not an inference about any product. Community guides describe VARA FM's Auto Tune as stepped tones and report that distorted tones can fool it [anec]. Aether FM's ladder uses the data's own statistics, and each rung's level is offset by that rung's peak (64-QAM 0.3 dB over 16-QAM, QPSK 1.1 dB under). pdn-soundmodem measured the same effect: set the drive against the densest constellation [meas].
2. **A level that stays right.** The in-session loop corrects drift (an OS update resetting a codec, a different peer, a repeater) without the operator.
3. **The far station names the fault.** "KK4ODA-1 hears you compressed by about 4 dB" instead of a VU bar (roadmap P10-3).
4. **Session efficiency for RadioPost's traffic**: turnaround, coalesced turns, the think-time window.
5. **Polite access** to shared channels, which a Winlink sysop guide says VARA FM lacks [anec].
6. **Open, native RadioPost transport** with delivery reports and program sharing on one radio.

What not to claim: a higher peak rate, or better performance than VARA FM at any SNR. With no A/B (Decision 10), its published table is the only comparison, and every use of it says so.

## I. Proposed software architecture

One repository, one application, one daemon binary that runs either air, and one daemon process per radio. Five parts are new: the FM physical layer, the packet physical layer and its AX.25 link (Decision 7), level control, and the PHY boundary that lets the daemon carry them all.

```
Programs   RadioPost (RPMI host) | Winlink Express, Pat, RMS Packet | KISS and AGWPE clients | Aether's panel
Ports      RPMI (adapter now, native later) | VARA-style TNC | KISS and AGWPE | control API (+ calibration)
Station    sessions and rules: claims, regulatory gate, identifier, recording, profiles, the Test
           level control (NEW): the ladder in the call, the probe after, learned profiles
Links      granted slots and rate control (aether-link, shared) | AX.25 2.2 link (NEW)
           ---- Phy trait: the new boundary, built in HF first (NEW) ----
Physical   aether-phy (HF only) | aether-phy-fm (NEW: SC-FDE) | packet PHY (NEW: AFSK 1200,
           G3RUH 9600, FX.25, IL2P) | aether-fec (shared)
Hardware   audio (cpal, AudioIo; OS volume guard added) | keying and radio control (serial, CAT,
           CM108, rigctld, FLRig, host; squelch reads) | channel sense (HF: busy detector;
           FM: squelch and activity)
```

Everything plain is Aether HF's code, reused; the dashed part stays HF's alone. RadioPost appears only at the top, through RPMI.

### I.1 The separations the author asked for

| Concern | Where it lives | Shared with HF | Tested by |
| --- | --- | --- | --- |
| 1. Physical-layer DSP | `aether-phy-fm` (new), `aether-fec` | FEC only; the packet PHY is new too | Model-first vectors, bit-exact (`tools/make_*_vectors.py`) |
| 2. Audio hardware | `aetherd/src/audio.rs`, `AudioIo` | Yes | The `[sim]` and lockstep `--channel` backends |
| 3. Radio control and keying | `aetherd/src/ptt.rs` | Yes, plus squelch reads | `CatProtocol` unit tests; bench radios |
| 4. MAC and link | `aether-link` (`granted.rs`, `rate.rs`, `frames.rs`) | Yes, family made generic; the AX.25 link is new | Granted-vector replays, TLA+ models, the harness |
| 5. Adaptive optimisation | Level control in the station; the rate controller in the link | Rate controller yes; level control new | The FM channel model, criteria E.7 |
| 6. Application interfaces | `aetherd/src/{control,host,kiss}/` | Yes | Host benches, the harness's host emulators |
| 7. RadioPost integration | RadioPost's `adapters/aether`, later native RPMI in the daemon | Yes | RadioPost's conformance tester A-01 to A-04 over two daemons |
| 8. User interface | `app/ui/`, `app/src-tauri/` | Yes, views per air | Playwright against a stand-in modem |

Two rules keep the layers honest. Nothing in the PHY or the link knows RadioPost exists. And RadioPost knows nothing of Aether beyond RPMI and an adapter.

### I.2 Repository layout

```
core/aether-fec/            shared, unchanged
core/aether-phy/            HF physical layer, unchanged
core/aether-phy-fm/         new: SC-FDE, sync, level measurements
core/aether-packet/         new: AFSK 1200, G3RUH 9600, FX.25, IL2P, AX.25 2.2
core/aether-link/           shared; the frame family becomes a parameter
core/aetherd/               one binary; [radio] air = "hf" | "fm"
model/aether_model/phy_fm/  new, model first
model/aether_model/fm_channel.py  new: the radio in the channel
app/                        one panel and shell, views per air
bench/scenarios/fm/         new harness scenarios
```

One daemon per radio, each on its own ports, matches RadioPost's model: a gateway holds one RPMI connection per modem (W26). A station with an HF radio and an FM radio runs two daemons under one shell.

## J. Development roadmap

The author's order holds, with two moves that cut architectural risk: real radios are recorded in Phase 2, before the waveform is chosen, and RadioPost runs on the harness in Phase 4, before the air. Each gate has a measurable criterion; a failing one stops the phase and is explained before anything changes, as ADR-0069's steps were.

1. **Architecture, and the PHY boundary in HF.** This proposal; ADRs FM-1 to FM-13; Aether HF's
   daemon refactored behind a `Phy` trait with no wire change.
   *Gate 1: HF's suite and a harness A/B unchanged.*
2. **2a. The FM channel model**, beside **2b. radio recordings, early** (5–10 radios: SNDR against
   drive, the band's response, turnaround, sound-card clocks).
   *Gate 2: the model reproduces the recordings within 2 dB.*
3. **2c. The waveform decided** (SC-FDE, CP-OFDM and a 4-FSK floor compared on the calibrated model,
   criteria set first), beside **3. calibration on the model** (the ladder and the probe, both loops
   together, criteria E.7).
   *Gate 3: ADRs FM-4 and FM-5 accepted on their criteria.*
4. **4. The core modem, legacy packet, and RadioPost on the harness.** Rust port bit-exact with the
   model; granted slots with FM's numbers; the daemon's FM air; RadioPost's conformance tester
   (A-01 to A-04) over two FM daemons.
   *Gate 4: no radiated overlap; RadioPost conformance passes.*
5. **5. Real radios: the bench, simplex, then repeaters.** Two radios through a dummy load and an
   attenuator, then the air; three radio models, both audio profiles; repeaters with their owners'
   leave; packet against Dire Wolf and a NinoTNC.
   *Gate 5: calibration converges on radios as on the model.*
6. **6. RadioPost end to end on the air.** Client to gateway, station to station, intermittent
   links, through a repeater.
   *Gate 6: the RadioPost criteria of J.2 met.*
7. **7. Performance against published figures** (VARA FM by its published table only; Winlink
   Express, Pat and RMS Packet benched), then **8. release**: installers, the manual, calibration
   diagnostics, the beta channel.

Phases 2b and 4 are the ones moved earlier. Phases 2a and 2b run side by side, as do 2c and 3.

### J.1 What changed from the author's sequence, and why

- **Radio recordings move into Phase 2 (2b).** Every number in this proposal rests on a model of the radio. If the model is wrong (R1–R4), choosing the waveform on it would be the HF lesson again: tuning on a pipe the air disagrees with. A dummy load, an attenuator and a few radios are enough; no on-air test is needed.
- **Calibration runs beside the waveform decision (3 with 2c).** The ladder only needs a signal with the data's statistics, so it can be developed on any candidate and finished on the chosen one. That gives the independent calibration testing the author asked for.
- **RadioPost moves to Phase 4, on the harness.** Its adapter is nearly free, and RadioPost's own conformance tester already runs A-01 to A-04 over two Aether daemons. Interface faults surface before any radio is involved. Phase 6 then tests RadioPost on the air.
- **The PHY boundary in HF comes first (Phase 1).** It is the riskiest change to shipped code, so it is done under HF's full suite before any FM code depends on it.

**Legacy packet (Decision 7) is its own track:** specified in Phase 2 from the public documents, built in Phase 4 beside the core modem, and tested in Phase 5 against Dire Wolf and a NinoTNC. Its signals are long established, so it carries little research risk; it adds build and test time.

### J.2 Test framework and acceptance criteria

The harness gets an FM radio per station (`tools/channel_server.py`'s `[stations.<who>.radio]`), with each impairment a scenario key: audio bandwidth, noise and interference, the limiter and its level, gain mismatch, emphasis at either end or neither, IF bandwidth, clicks, frequency and sample-clock errors, OS processing, keying and squelch timing, and a repeater's key-up, delay line, hang, courtesy tone and second limiter. Calibration is also tested on its own, offline in the model, with no link.

| RadioPost test | Method | Criterion |
| --- | --- | --- |
| Station to station | `session-03` across two FM daemons, as RadioPost's `tests/aetherd.rs` does on HF | Byte for byte, every run |
| Client to gateway | `session-01` with RadioPost Gateway and a post office in-process over two FM daemons | Completes, within 20 % of C.4's estimate for its profile |
| Weak or intermittent links | Harness outages mid-message, then a new session | Every message stored exactly once within three sessions; resumed at a non-zero offset |
| Recovery after a disconnect | Break the path; count which timer fires | The modem reports `disconnected` within its link timeout (under 60 s); RP-3's 300 s timer fires first in 0 runs |
| Retransmission and buffering | Count RP-3 PDUs sent twice over a live link; watch `queued` | 0 duplicates; `queued` never over `max_queue`; `delivered` before the matching `RECEIPT`, always |
| Large messages and short control traffic | A 50 kB attachment, then a session of small PDUs | 85 % of the PHY rate on simplex; 0 RP-3 turns split across transmissions |
| No conflicting retries | RadioPost-level retries counted while the link is up | 0 |
| Independent implementations | RadioPost's conformance tester, which plays the host from the spec's tables without RadioPost's engines | A-01 to A-04 pass against the Aether FM adapter |

Modem-level gates beside these: no radiated overlap on the harness (ADR-0069's C1a); E.7's calibration criteria; and, by Decision 10, comparisons with VARA FM only against its published table; Aether FM's own curves committed under `bench/`.

## K. Risk register

The largest risk is that real radios fall short of the model: their SNDR ceiling, their limiter's position and their turnaround decide the top rungs and the session times. So the first experiment with hardware comes early, in Phase 2, before the waveform is fixed.

| ID | Uncertainty | If it goes wrong | Settled by | When |
| --- | --- | --- | --- | --- |
| R1 | Real radios' SNDR ceiling is lower than the model's 29–31 dB (microphone compressors, poor emphasis matching, sound-card noise) | The top rungs go unused; the voice path tops out near 7–9 kb/s, not 12.6 | Record a known training sequence through 5–10 radio models, voice and flat ports, transmitter into a dummy load with an attenuator to a second radio; SNDR against drive for each | Phase 2b |
| R2 | The limiter sits elsewhere or clips softly; some radios compress the microphone path | Pre-compensation and the ladder's curve classes are wrong for those radios | The same recordings | Phase 2b |
| R3 | The 16 ms voice-path guard is shorter than the 300 Hz high-pass's ringing | An ISI floor caps the high rungs | Guard length against residual error on the recorded responses | Phase 2b |
| R4 | RF-to-RF turnaround is well over 0.4 s on common radios | Sessions 30–40 % longer (C.4) | Time key-up to RF and squelch opening with an SDR on each radio | Phase 2b |
| R5 | VOX interfaces (SignaLink) hold the key 150–500 ms | Every turn costs that | Measure; the learned gap absorbs it; the guide recommends CAT or serial keying | Phase 5 |
| R6 | OS audio processing is on without the operator knowing | Calibration fails or misleads | Detection tests on Windows, macOS, Linux and a Pi; the setup wizard checks | Phase 3 |
| R7 | Flutter or interference fools the two-level probe | The level wanders | Harness scenarios with picket fencing and QRM; the freeze rules | Phase 3 |
| R8 | The level loop and the rate controller interact | Oscillation, lost throughput | Both loops together on the harness, criteria E.7 | Phase 3 |
| R9 | Repeaters vary widely (courtesy-tone timing, audio delay, DTMF muting, codecs on links) | Some repeaters fail | Record 5–10 local repeaters; the repeater profile learns their timing | Phase 5 |
| R10 | Occupied bandwidth at 4 000 Bd on a flat port exceeds the channel or §97.307 | A regulatory or neighbourliness problem | Measure with an SDR; a deviation ceiling; the operator's one-time deviation check | Phase 5 |
| R11 | Click noise near the FM threshold hurts single carrier more than modelled | The floor rung fails early, mobile paths suffer | Phase 2's gate: π/2-BPSK against 4-FSK at and below threshold | Phase 2 |
| R12 | A gateway's think time exceeds the 2 s open window | Extra polls, a slower first reply | Measure on the harness with RadioPost Gateway and a real post office | Phase 6 |
| R13 | The PHY-boundary refactor destabilises HF | HF regressions while testers are on the air | Do it under HF's full suite and a harness A/B, before any FM code | Phase 1 |
| R14 | Cheap USB codecs drift beyond the tracking range | Long transmissions fail late | Measure codec ppm; size the tracking loop | Phase 2b |
| R15 | With no A/B (Decision 10), VARA FM's published table is the only comparison | Users compare on the air anyway, and a published table flatters a real channel | Claim only what Aether FM measures itself, with its method; label every comparison as against published figures | Phase 7 |
| R16 | Legacy packet from the start (Decision 7) delays the native modem | The first release comes later | Its own track from public specifications, no GPL code copied; tested against Dire Wolf and a NinoTNC | Phases 4–5 |
| R17 | Data on voice repeaters is unwelcome | Repeater owners object | Off by default; a profile the operator enables, with duty limits; the guide says to ask the owner | Phase 5 |
| R18 | No dummy load or attenuator on the bench yet (Decision 9) | Phase 2b's recordings wait, and so does the waveform choice | A dummy load rated for the transmitter and a 40–60 dB attenuator or tap; failing that, low-power recordings on the air, identified | Phase 2b |

## L. Architecture Decision Records to write before coding

Thirteen decisions need an ADR before any FM code. They continue Aether's own sequence (ADR-0112 onward) if Aether FM lives in this repository. Each one states what it removes as well as what it adds, as the project's rule now asks. "Needs" is the evidence that must exist before it is accepted.

| # | Decision | Decides | Needs |
| --- | --- | --- | --- |
| FM-1 | Product structure | One repository and application; an `aether-phy-fm` crate beside `aether-phy`; one daemon per radio, selected by air; one shell supervising several | Decided: decision 1 |
| FM-2 | The PHY boundary | A `Phy` trait in `aetherd` and a generic `PhyTiming`/`SoftFrame` in `aether-link`; done in HF first | HF's full suite and a harness A/B unchanged |
| FM-3 | The FM channel model | Emphasis, limiter, splatter filter, IF filter, discriminator, clicks, squelch, sound cards, repeaters; calibrated against recorded radios | Phase 2b recordings |
| FM-4 | The waveform | SC-FDE against CP-OFDM, and the 4-FSK floor candidate; profiles, symbol rates, guard words, preamble, ladder | Phase 2 curves on the channel model, criteria registered before the runs |
| FM-5 | Calibration | The ladder's format and placement, the curve classes, the two-level probe, the control law, the ceiling, the freeze rules, per-rung offsets | Phase 3 on the model, E.7's criteria |
| FM-6 | Learned profiles | What is kept per radio and per peer, their keys, scopes and invalidation | Phase 3 |
| FM-7 | The FM link protocol | Granted slots with FM's gap terms, framing, open windows for think time, burst caps, its protocol number | The formal model re-run with FM's latencies |
| FM-8 | Channel access | Squelch sources, activity detection, p-persistence, reservation, hidden stations | Phase 4 harness |
| FM-9 | Repeater profile | Detecting a repeater, learning its key-up, hang and courtesy tone, using the hang time, duty limits | Phase 5 recordings of real repeaters |
| FM-10 | VHF/UHF rules | Regulatory data for 2 m, 1.25 m, 70 cm; an FM occupancy model; the deviation ceiling; automatic control | The rule text (as ADR-0018 did) |
| FM-11 | Identification on FM | The callsign in the digital signal (Decision 4): which code carries it, how often, and what the gate checks | The rule text it rests on, cited as ADR-0018 did |
| FM-12 | Host interfaces | Control-API additions, VARA FM's published commands (`CONNECT … VIA`), KISS, native RPMI for HF and FM | Bench with Winlink Express, Pat, RMS Packet and RadioPost |
| FM-13 | Legacy packet | 1200 AFSK, 9600 G3RUH, FX.25, IL2P and AX.25 2.2 connected mode; the KISS and AGWPE ports; hearing packet and Aether FM at once | Public specifications only; interoperability with Dire Wolf and a NinoTNC |

RadioPost's side (the `aether-fm` registry entry under G11, and RP-7's FM clarifications) belongs in RadioPost pull requests, not in Aether ADRs. FM-12 records Aether's half and links them.

## Decisions

The author answered all eleven on 2026-11-13, with two follow-ups. The sections above follow them.

| # | Question | The author's answer | What it changes |
| --- | --- | --- | --- |
| 1 | Where Aether FM lives | In this repository, one app and installer | Section I as laid out |
| 2 | Rework HF's daemon behind a PHY boundary while testers are on the air | Yes, now, proven unchanged by HF's tests and a harness A/B | Phase 1 starts with it |
| 3 | Voice path or flat data port first | Both simulated; the voice path first on the air | D.4's profiles; Phase 5's order |
| 4 | How a station identifies on FM | The callsign in the digital signal, no Morse | Sessions lose 5–7 s. ADR FM-11 still cites the rule text it rests on (§97.119), and the regulatory gate checks the ten-minute and end-of-contact rules |
| 5 | Relaying through other stations | Not in the first release | H's gap stays; revisit once repeaters work |
| 6 | Repeaters out of the box | Off by default | G.2's repeater profile is an operator setting |
| 7 | Legacy packet radio | From the start: 1200 AFSK, 9600 G3RUH, FX.25 and IL2P, AX.25 connected mode | A packet physical layer and an AX.25 link beside Aether's own (D.6, I, J, K, L) |
| 8 | When to prepare RadioPost's spec changes | Now, as a draft pull request; nothing merges without the author | The `aether-fm` modem name, FM channels' meaning, RP-7's VHF rows: draft PRs [KK4ODA/RadioPost#1](https://github.com/KK4ODA/RadioPost/pull/1) and [#2](https://github.com/KK4ODA/RadioPost/pull/2), merged on 2026-11-13 |
| 9 | Bench equipment | Two or more FM radios, one with a 9600 port, and a handheld; no dummy load or attenuator yet | Phase 2b needs a dummy load and an attenuator (risk R18) |
| 10 | Comparing with VARA FM | Its published figures only; no A/B | No claim beyond the published table (H, J, K) |
| 11 | The phase order | The new order: radios recorded before the waveform choice, RadioPost on the harness before the air | Section J as drawn |

## Sources

Repository files were read at the checkouts named; web pages were opened by the two research passes on 2026-11-13. Where a page could be seen only as a search excerpt, the text above says so.

**Aether HF** (`KK4ODA/aether-hf` at `9d3707c`): `CLAUDE.md`; `docs/ROADMAP.md` §11 and Phase 10; `docs/adr/0068-the-answer-goes-when-the-burst-says.md`; `docs/adr/0069-granted-slots.md`; `model/aether_model/waveform.py`; `model/aether_model/phy/tx.py`; `core/aether-link/src/phy.rs`; `core/aetherd/src/{audio,config}.rs`; `core/aetherd/data/regulatory/us-fcc-part97.json`; line counts across `core/`.

**RadioPost** (`KK4ODA/radiopost` at `f8cee6c`): `README.md`; `CLAUDE.md`; `spec/README.md`; `spec/rp-0003-session.md`; `spec/rp-0004-modem-interface.md`; `spec/rp-0005-post-office-api.md` §4.4; `spec/rp-0007-forwarding.md` §1, §2, §5; `spec/rp-0000-conventions.md` §9; `docs/concept/00-concept.md` §1, §2, §7; `docs/design/00-detailed-design.md` §3, §5.4; `docs/design/02-wishlist.md` (W20, W23, W26, W44, G3, G4, G11); `adapters/aether/src/bridge.rs`.

**VARA FM**, published documents only: [author's site](https://rosmodem.wordpress.com/); [VARA FM 4.0 Features](https://mega.nz/file/XCB1wYqC#7t0uYoM8pLwDHGtjooMXzLl0n8_q80ZWJSCtEFQto0g); [TNC Commands](https://mega.nz/file/SaRFmISC#A8wDxP293ggkHjngqkA6xED07c9R6fNBpQebv6TEg5E); [KISS Interface](https://mega.nz/file/eCxBFZzA#F6mfrEtQ0UCh7dxxKinwgU0livyB_87QI2delWbYn_U); [Terms of use](https://mega.nz/file/bOYWAaIA#SJOpLAl862QsxFTuzFEIX-KnvYeu1VbwWudZ_kjXcPw). User experience and the one measured curve: [KN6KB's guide](https://winlink.org/sites/default/files/brickcomputerforwinink_vhf_2.1.pdf); [K6OLI guide](https://qsl.net/nf4rc/2022/VARA_FM_4.x_2021Nov.pdf); [SEAPAC 2025](<https://seapac.org/workshop-presentations_files/2025/SEAPAC%202025%20VARA%20FM%20SETUP%20(3).pdf>); [AE7G primer](https://www.masterscommunications.com/products/radio-adapter/dra/vara-primer.html); [NS7C dual-mode guide](https://winlink.org/sites/default/files/RMSE_FORMS/rms_packet_dual_mode_single_radio_setup_0_0.pdf); [Pat wiki](https://github.com/la5nta/pat/wiki/VARA).

**Other modems:** [MercuryFM](https://github.com/Guru-RF/MercuryFM) and its [NOTES.md](https://github.com/Guru-RF/MercuryFM/blob/main/NOTES.md); [Mercury](https://github.com/Rhizomatica/mercury); [pdn-soundmodem modes](https://github.com/packet-net/pdn-soundmodem/blob/main/docs/05-modes.md) and [on-air campaign](https://github.com/packet-net/pdn-soundmodem/blob/main/docs/dev/ofdm-fm/on-air-campaign.md); [Dire Wolf User Guide](https://github.com/wb2osz/direwolf/blob/dev/doc/User-Guide.pdf), [Going beyond 9600](https://github.com/wb2osz/direwolf/blob/master/doc/Going-beyond-9600-baud.pdf), [Why 9600 is only twice as fast](https://github.com/wb2osz/direwolf/blob/dev/doc/Why-is-9600-only-twice-as-fast-as-1200.pdf); [IL2P draft 0.6](https://tarpn.net/t/il2p/il2p-specification_draft_v0-6.pdf); [NinoTNC manual](https://tarpn.net/t/nino-tnc/n9600a/n9600a_operation.html); [Codec2 FreeDV README](https://github.com/drowe67/codec2/blob/main/README_freedv.md); [M17 specification](https://m17project.org/wp-content/uploads/2025/11/M17_spec.pdf); [ARDOP specification 0.7](https://sources.debian.org/src/golang-github-la5nta-wl2k-go/0.11.9-1/docs/ardop/_ARDOP%20Specification.pdf); [G3RUH 9600 modem](https://www.amsat.org/amsat/articles/g3ruh/109.html).

**Radios, repeaters, rules:** [Kenwood TM-V71A service manual](https://www.repeater-builder.com/kenwood/pdfs/tm-v71ae-svc-man.pdf); [Yaesu FTM-400 technical supplement](https://rigpix.com/yaesu/ftm400-series_service.pdf); [ETSI EN 300 086-1](https://www.etsi.org/deliver/etsi_en/300001_300099/30008601/01.02.01_60/en_30008601v010201p.pdf); [Tait TB9400](https://www.manualslib.com/manual/1505674/Tait-Tb9400.html?page=25); [ARRL FTM-6000R review](https://www.arrl.org/files/file/ProductReviewsForDeb/2023/pr082022.pdf); [TAPR DCC 1995 radio tests](https://web.tapr.org/meetings/DCC_1995/DCC1995-DataRadioTesting-VE2BMQ-N2IRZ.pdf); [OARC 9600 rig support](https://wiki.oarc.uk/packet:9k6-rig-support); [NHRC-2.1 manual](https://www.repeater-builder.com/nhrc/manuals/current/nhrc-2.1-manual.pdf); [Link RLC-ADM](https://www.repeater-builder.com/link-rlc/pdfs/rlc_adm.pdf); [CAT DL-1000](https://www.repeater-builder.com/cat-auto/cat-archive/dl1000man.pdf); [Tait TN-1031](https://www.manuals.repeater-builder.com/2006/TECHNOTE/TN-1031-SR_TM8100_CTCSS.pdf); [47 CFR §97.305](https://www.law.cornell.edu/cfr/text/47/97.305), [§97.307](https://www.law.cornell.edu/cfr/text/47/97.307), [§97.221](https://www.law.cornell.edu/cfr/text/47/97.221), [§97.205](https://www.law.cornell.edu/cfr/text/47/97.205); [CM108 data sheet](https://mikrocontroller.net/attachment/192639/CM108_DataSheet_v1.6.pdf); [Windows audio processing modes](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/audio-signal-processing-modes); [IEEE Commun. Mag. 40(4):58, SC-FDE](https://doi.org/10.1109/35.995852).
