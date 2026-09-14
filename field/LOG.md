# Field log

One row per session on the air. `docs/user/field-test.md` §6 says what goes in each column;
`tools/compare_air.py` prints the numbers. Phase 6 closes at twenty rows across three
channel classes with the simulator within 20 %.

| Date | Band | Distance | Class | Stations | Recording | SNR dB | Goodput measured / predicted | Note |
|---|---|---|---|---|---|---|---|---|
| 2026-09-14 | bench | 0 (simulated channel, 15 dB set) | awgn | KK4ODA → KK4ODA-1 | not kept (bench) | 14.3 | 686 / 1059 b/s (0.65) | Pat 1.0.0 at both ends, a P2P message with a 6 kB incompressible attachment, byte-identical. The gap is B2F's proposal/answer/FF/FQ round trips on top of the single transfer the simulator models; the rate ramp 0→2→4→6→9→10 in six bursts. Not an air session: here for scale |
| 2026-09-14 | bench | 0 (simulated channel, 15 dB set) | awgn | KK4ODA-1 → KK4ODA-2 | not kept (bench) | 14.5 | 854 / 1049 b/s (0.81) | Winlink Express 1.8.5.0 at both ends, a P2P message with a 6 kB incompressible attachment (zipped by Winlink Express), byte-identical. The rate ramp 0→2→4→6→9→10, no frame lost. Found that `MYCALL` had never reached the modem (fixed: `callsigns.set`). Not an air session: here for scale |
