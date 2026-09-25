# Architecture decision records

Short, numbered, immutable once accepted (amend by a new ADR that supersedes). Template:
context → decision → alternatives considered → consequences.

| # | Title | Status |
|---|---|---|
| [0001](0001-language-and-stack.md) | Python reference model, Rust production core, Tauri desktop shell | accepted |
| [0002](0002-waveform-parameters.md) | Aether HF v1 waveform — OFDM numerology and starting parameters | accepted (starting point) |
| [0003](0003-fec-family.md) | FEC = 3GPP TS 38.212 LDPC BG2 with rate matching | accepted |
| [0004](0004-papr-reduction.md) | Peak-to-average power reduction | accepted |
| [0005](0005-front-end-without-a-build-step.md) | The front-end is plain ES modules, with no build step | accepted (amends 0001 §3) |
| [0006](0006-link-probe.md) | The link probe — a beacon with a destination, answered with the SNR heard | accepted |
| [0007](0007-faster-climb.md) | The faster climb — a learned margin is given back at an accelerating rate | accepted |
| [0008](0008-faster-start.md) | The faster start — a session begins where the connect frames measured it | accepted |
| [0009](0009-the-floor.md) | The floor — a frame family for the SNR region below the mode table | superseded by 0013 (the tone floor replaced it) |
| [0010](0010-whole-burst-to-the-card.md) | A whole burst goes to the sound card at once, and the key follows the card's clock | accepted |
| [0011](0011-profiles.md) | Profiles — a station's settings as one portable file, projected through a settings registry | accepted |
| [0012](0012-holding-the-link.md) | Holding the link on a fading path — the timeout spans whole exchanges, silence steps the mode down, the ACK waits for the announced frame | accepted |
| [0013](0013-the-tone-floor.md) | The tone floor — a steady-envelope FSK family under both ladders; the ladder; crossing the floor boundary by what the rungs are worth | accepted |
| [0014](0014-fast-tones.md) | Fast tones — the floor's frame with its data at 50 and 100 baud, the 2 300 Hz ladder's middle rungs; link protocol 3 | accepted |
| [0015](0015-narrow-middle-kinds.md) | The narrow middle kinds — four tones at 100 baud inside the floor's 400 Hz, the 500 Hz ladder's middle rungs; contradicted sync symbols; link protocol 4 | accepted |
| [0016](0016-calls-on-the-floor.md) | Calls, probes and beacons start on the tone floor; a reading from the floor is a lower bound | accepted (amends 0006 and 0009's connect rule) |
| [0017](0017-a-burst-fits-the-key.md) | A burst fits the key; a session ends on the air; the Test climbs before it carries | accepted |
| [0018](0018-the-regulatory-gate.md) | The regulatory gate — nothing is keyed without the policy's leave; the rules as data; the occupancy measured | accepted |
| [0019](0019-datagrams-and-the-kiss-port.md) | Datagrams, and a KISS port that answers as VARA's does — another program's frame outside sessions; the VARA frame types; Winlink priority | accepted |
| [0020](0020-what-a-failure-says.md) | What a failure says — the receiver learns only from frames that could tell it something: trusted SNRs, self-decodable or combined failures, not the sender's own choices | accepted |
