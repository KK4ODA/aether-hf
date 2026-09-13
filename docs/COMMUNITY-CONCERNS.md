# Community concerns → roadmap coverage

Source: r/amateurradio thread "What do you think about Mercury, the VARA alternative now that
it's been out a couple months?" (28 comments, read 2026-09-13). Mercury is the closest
existing attempt at what Aether HF aims to be, so the reactions to it are the best available
proxy for how Aether will be judged. Concerns are paraphrased and deduplicated; "weight" is
the approximate number of commenters (and upvotes) behind each.

| # | Concern raised about Mercury / any VARA alternative | Weight | Roadmap coverage | Gap / change made |
|---|---|---|---|---|
| 1 | **Network effect beats technology.** "It doesn't matter how well it works — what matters is how many active, 24/7, well-run gateways there are." "Where are you finding RMSs?" "You need someone on the other end." | very high (top-voted technical comment, 4 commenters) | Headless daemon (§4.2, P3-3); Linux/Pi builds (§9); RMS Trimode notes (§12) | **Gap.** Gateways were a Phase 4–5 afterthought. Added an *Adoption & ecosystem* track (§13, Phase 3): headless Linux/ARM64 gateway build and systemd "gateway kit" land with the VARA adapter, plus a public gateway registry and Winlink dev-team coordination. |
| 2 | **Fragmentation / needs a compatibility layer.** "Just another digital mode splitting an already fragmented community"; "there needs to be a compatibility layer to not fragment the user base." | high (2 commenters, 5–9 pts each) | VARA-compatible host API so Winlink Express / Pat / VarAC / BPQ32 work unchanged (§6) | Over-the-air compatibility with a proprietary waveform is impossible; the roadmap addresses the *software* side. Added: coexistence rules (busy detector must recognise VARA/ARDOP signals; published frequency plan away from VARA calling channels). |
| 3 | **Setup takes hours; CLI-only; docs assume expertise.** "3 hours to get running", "man-style file is worthless to everyone else", "Python dependency hell on Ubuntu, gave up", "had to hand-copy a hamlib DLL". | very high (4 commenters, incl. 11-pt comment) | Setup wizard, single installer, auto-detected devices (§8); Rust core = no runtime/dependency install for users (§4.3); user docs incl. VarAC/Pat/Winlink guides (§10) | Covered; this thread is the strongest external validation of the Rust-core + installer decision. Added: ship a current Hamlib build inside the installer with an override path, so a new radio never requires DLL surgery. |
| 4 | **No unified GUI; the GUI that exists "technically works".** | high | Desktop app with connection panel, meters, waterfall, constellation, diagnostics (§8) | Covered. |
| 5 | **"No Windows version" (there was, but a zip, not an installer).** | medium | Windows-first, signed NSIS/MSI installer, auto-update (§9) | Covered; the confusion itself shows why a real installer and a Releases page matter. |
| 6 | **Headless Linux / Raspberry Pi is the killer advantage over VARA** (VARA is Windows-only; a Linux distro already bundles Mercury). | high (9 pts) | Headless `aetherd`; Linux `.deb`/AppImage (§9) | Promoted: Linux x86-64 **and ARM64** are first-class release targets from Phase 3, not "later"; package so distros (LiaisonOS-style) can include it. |
| 7 | **Weak-signal performance unknown** (OP asks; nobody could answer). | medium | Benchmark suite with committed FER/throughput curves; no claim without a curve (§7) | Covered. |
| 8 | **ALC spikes when transmission starts** ("high ALC spikes that dropped as data kept going — ugly, might cause issues"). | medium | PAPR study (P2-4); level calibration (§8) | **Addressed (P2-4 / ADR-0004).** Clip-and-filter cuts the burst PAPR from ≈ 10 dB to 5.7 dB (7.3 dB for 64-QAM), so the peak the ALC reacts to drops by more than 4 dB, and buys +1.0…+1.7 dB of delivered power at the same time. Preamble and data stay within ≈ 1.1 dB of each other, with soft symbol-edge ramps. Still to do: the wizard's ALC/peak meter (§8). |
| 9 | **Voice announcements over the data channel** ("beaconing", "sending to all", mangled callsign) waste bandwidth. | medium | Identification by clear-text callsigns in headers + optional CW ID (§12) | Covered — no synthesized speech; ID is in the documented protocol. |
| 10 | **Open source is the point; VARA's real sin is being undocumented.** "Hope it replaces VARA entirely" (36 pts); OP: "not fine with VARA not being documented enough to re-implement"; counter-view: closed-source is fine, it's $30. | very high | MIT/Apache dual license; public air-interface spec (§6, §10) | Covered — the public spec is exactly the OP's demand and a US legal requirement (§97.309(a)(4)). |
| 11 | **No official frequency / calling channel etiquette** ("burning over active VARA HF"). | medium | Busy-channel detector (§12) | Added: publish a recommended frequency plan and coexistence guidance in `docs/user/`; busy detector on by default. |
| 12 | **Radio support** (FTX-1 needed a newer Hamlib; laptops that won't talk to an IC-7300). | medium | Hamlib/rigctld/flrig/native CAT; wizard probes ports; auto-profiles (§8) | Covered; plus the bundled-Hamlib item above and a USB-audio troubleshooting page in the wizard. |
| 13 | **Interop with VarAC specifically** (people set Mercury up as a VarAC backend). | medium | VarAC listed as a consumer of the VARA-compatible API (§6) | Added VarAC to the Phase 3 verification matrix alongside Pat and Winlink Express. |
| 14 | **Discoverability: "where are the RMSs?"** | medium | — | Added: public gateway registry page + optional beacon/CQ frames so stations can be heard (§12). |
| 15 | **iOS / mobile** (raised, then noted VARA has none either). | low | Remote web UI served by the daemon (§4.2) | Not a target; the remote web UI gives phone/tablet *monitoring* of a headless station, which is the realistic ask. |
| 16 | **"Is this JNOS again?" — scepticism that alternatives ever stick.** | low | Phases 4–6 sequencing (working headless modem → polish → field validation) | The only answer is shipping; noted as the reason the roadmap gates public release on Phase 5. |

## What the thread changes in the plan

1. **Gateways are a Phase 3 deliverable, not Phase 5.** The headless Linux/ARM64 build,
   a systemd unit, and RMS Trimode / BPQ32 configuration guides ship with the VARA adapter.
2. **Winlink ecosystem coordination is a tracked risk.** Winlink Express and the RMS channel
   list only know VARA / ARDOP / Pactor session types. Speaking the VARA API gets Aether
   *running* under those programs, but a gateway advertised as "VARA" that actually runs
   Aether would strand real-VARA clients. Plan: build usage first where no central listing is
   needed (VarAC, Pat P2P, BBS/Emcomm BBS), publish the spec, and approach the Winlink
   Development Team about an "Aether HF" session type once field results exist.
3. **TX envelope quality is a requirement**, not a nice-to-have (ALC complaint).
4. **Frequency plan and coexistence** documentation is part of the user docs.
5. **Bundle Hamlib** (with an override) so no user ever swaps a DLL.
6. **Public gateway registry** and beacon/CQ discovery frames are in scope.

Everything else in the thread is already addressed by the roadmap as written.
