# Field log

One row per session on the air. `docs/user/field-test.md` §6 says what goes in each column;
`tools/compare_air.py` prints the numbers. Phase 6 closes at twenty rows across three
channel classes with the simulator within 20 %.

| Date | Band | Distance | Class | Stations | Recording | SNR dB | Goodput measured / predicted | Note |
|---|---|---|---|---|---|---|---|---|
| 2026-09-14 | bench | 0 (simulated channel, 15 dB set) | awgn | KK4ODA → KK4ODA-1 | not kept (bench) | 14.3 | 686 / 1059 b/s (0.65) | Pat 1.0.0 at both ends, a P2P message with a 6 kB incompressible attachment, byte-identical. The gap is B2F's proposal/answer/FF/FQ round trips on top of the single transfer the simulator models; the rate ramp 0→2→4→6→9→10 in six bursts. Not an air session: here for scale |
| 2026-09-14 | bench | 0 (simulated channel, 15 dB set) | awgn | KK4ODA-1 → KK4ODA-2 | not kept (bench) | 14.5 | 854 / 1049 b/s (0.81) | Winlink Express 1.8.5.0 at both ends, a P2P message with a 6 kB incompressible attachment (zipped by Winlink Express), byte-identical. The rate ramp 0→2→4→6→9→10, no frame lost. Found that `MYCALL` had never reached the modem (fixed: `callsigns.set`). Not an air session: here for scale |
| 2026-09-16 | bench, 2300 Hz | ? (EM73 → ?) | awgn | KK4ODA-1 → KK4ODA-2 | `20260916-223635_KK4ODA-1_KK4ODA-2_test` | 11.8 | 1977 / — b/s | The first Test session, two daemons over the simulated channel at 12 dB set: the ladder is the modem's own AWGN table read back (modes 11–13 fail at 12 dB, as the bench says they should), and the replay agrees with the measured goodput within 3–11 % at a fitted penalty of −0.25 dB. Not an air session: here for scale; Test session: complete; probe 12 dB there, 11.8 dB here; message 1041 b/s, file 1977 b/s; ladder 0:5/5@12; 1:5/5@12; 2:5/5@12; 3:5/5@12; 4:5/5@12; 5:5/5@11; 6:5/5@12; 7:5/5@12; 8:5/5@11; 9:5/5@11; 10:5/5@11; 11:0/5@12; 12:0/5@13; 13:0/5@12; bench, 50 W, a socket |
