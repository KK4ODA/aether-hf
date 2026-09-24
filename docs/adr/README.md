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
