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
| [0021](0021-every-control-frame-says-how-it-hears.md) | Every control frame says how its sender hears the other station — a disconnect tells a station that only received how it was heard | accepted |
| [0022](0022-the-end-of-a-session-waits-out-the-identifier.md) | The end of a session waits out the other station's identifier — no repeated DISC, answer or closing identifier over a Morse ID | accepted |
| [0023](0023-leaving-and-handing-over.md) | Leaving and handing over — a receiver's Disconnect leaves between bursts, the TURN waits for the longest first frame, and the caller keeps the turn | accepted |
| [0024](0024-beacons-for-host-programs.md) | A beacon carries the name a host program gave it and its sender's bandwidth; no beacon under automatic control | accepted |
| [0025](0025-the-host-program-can-own-the-radio.md) | The host program can own the radio — keying on PTT ON, as VARA is keyed | accepted |
| [0026](0026-the-bandwidth-follows-the-host-and-the-caller.md) | The bandwidth follows the host program and the caller; LISTEN decides whether calls are answered | accepted |
| [0027](0027-chat-handover.md) | In a chat the receiving station asks for the turn — and the sender does not hand it over unasked | accepted |
| [0028](0028-a-polls-wait-covers-a-late-answer.md) | A poll's wait covers the answer to a poll heard and not decoded | accepted |
| [0029](0029-a-turn-heard-again.md) | A TURN heard again is answered, the turn is taken back only on evidence, and an acknowledgement is not acknowledged | accepted |
| [0030](0030-a-sender-waits-out-a-frame-it-hears-arriving.md) | A sender waits out a frame it hears arriving — no burst or poll repeated over its late answer | accepted |
| [0031](0031-a-re-encoded-frame-stays-unacknowledged.md) | A frame re-encoded and left out of its burst stays unacknowledged — no frame lost, no sender silenced, no session closed with a frame missing | accepted |
| [0032](0032-an-unanswered-poll-steps-down.md) | An unanswered poll steps the recommendation down, from the second in a row — the polls reach the tone floor within the retries | accepted |
| [0033](0033-the-link-timeout-follows-the-link-down.md) | The link timeout follows the link down to the floor — reckoned again whenever a station sends | accepted |
| [0034](0034-an-answer-that-did-not-read-its-frame.md) | A POLL or TURN answered without being read goes again at once, on the floor — an acknowledgement to a TURN, or a floor one to an ordinary POLL, answered the preamble alone | accepted |
| [0035](0035-the-bandwidth-trap.md) | The bandwidth trap — probes are answered across bandwidths, a station known to run 500 Hz is called at 500 Hz, and a call that cannot be answered is said on the panel | accepted |
| [0036](0036-the-answer-gap.md) | The answer gap — a station waits a set time after another station's frame before it keys, so a VOX-keyed station hears its answers | accepted |
| [0037](0037-the-turnaround-measured.md) | The turnaround, measured — no receive recovery window; the busy detector forgets the channel before its own transmission; the turnaround is recorded | accepted |
| [0038](0038-nd1j-two-sessions.md) | ND1J's two sessions — a hopeless frame is re-encoded sooner, a frame of nobody's session is no part of a burst, the keyed tail follows the sound card, and a waiting beacon does not go into a session | accepted |
| [0039](0039-disconnect-patience.md) | A sender's Disconnect gives up on what the path will not carry; the occupied width says which measure it is | accepted |
| [0040](0040-a-faint-arrival-holds-the-acknowledgement.md) | A faint arrival where the next frame of a burst would begin holds the acknowledgement | accepted |
| [0041](0041-the-burst-countdown.md) | Each frame of a burst says how many follow it, in a turn of its mode chips; link protocol 5 | accepted |
| [0042](0042-the-scenario-harness.md) | Whole sessions between real daemons through simulated band conditions; a receiver's DISC answers its burst at once | accepted |
| [0043](0043-per-carrier-noise-and-the-rv-order.md) | Each carrier weighed by its own noise, the reported SNR without the interfered pilots, and a frame's second copy RV 0 again | accepted |
| [0044](0044-the-turn-request-in-every-session.md) | The station without the turn asks for it in every session, not only under a host's CHAT ON | accepted |
| [0045](0045-the-answer-after-a-burst-said-to-be-over.md) | The answer after a burst said to be over waits only the turnaround; the request's quiet in the family in use | accepted |
| [0046](0046-the-countdown-in-pairs.md) | The burst countdown counts in pairs and never short; link protocol 6 | accepted |
| [0047](0047-the-turn-in-the-acknowledgement.md) | The turn offered at the end of a burst, taken in the acknowledgement; link protocol 7 | accepted |
| [0048](0048-an-acknowledgement-is-an-acceptance.md) | A caller that reads an acknowledgement of its own session has been accepted | accepted |
| [0049](0049-a-frame-stands-on-two-clean-blocks.md) | A tone frame stands on two clean sync blocks | accepted |
| [0050](0050-debug-mode.md) | Debug mode sends the host program's sessions to the project | accepted |
| [0051](0051-a-connection-waits-for-a-scanning-host-to-listen.md) | A connection the station answered waits for a scanning host (RMS Trimode) to be listening again before its CONNECTED is sent | accepted |
| [0052](0052-a-session-ends-at-both-ends.md) | A session ends at both ends, and takes its data with it | accepted |
| [0053](0053-the-callers-first-poll-offers-the-turn.md) | The caller's first poll offers the turn | accepted |
| [0054](0054-each-stations-answer-gap-is-learned.md) | Each station's answer gap is learned from its asking again | accepted |
