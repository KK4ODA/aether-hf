### Completion and overlaps, by arm and station profile

Completed: every message delivered intact. Clean: both stations closed the session
with a DISC exchange. Overlaps: instants both transmitters radiated (per run, mean,
95 % bootstrap interval over runs).

| arm | profile | runs | completed | clean end | RF overlaps / run | runs with any |
|---|---|---:|---:|---:|---:|---:|
| legacy (protocol 7) | ideal | 1008 | 100.0 % | 98.9 % | 0.24 [0.20, 0.28] | 13.4 % |
| legacy (protocol 7) | field | 1008 | 99.4 % | 95.9 % | 0.80 [0.73, 0.87] | 43.6 % |
| legacy (protocol 7) | field-asym | 1008 | 98.4 % | 93.5 % | 1.16 [1.07, 1.24] | 56.5 % |
| legacy (protocol 7) | hostile-c | 1008 | 95.9 % | 93.2 % | 2.60 [2.43, 2.76] | 73.5 % |
| legacy (protocol 7) | hostile | 1008 | 93.9 % | 87.7 % | 3.12 [2.94, 3.31] | 78.8 % |
| ADR-0068 | ideal | 1008 | 100.0 % | 99.0 % | 0.18 [0.15, 0.20] | 14.3 % |
| ADR-0068 | field | 1008 | 98.6 % | 95.9 % | 0.79 [0.73, 0.86] | 45.7 % |
| ADR-0068 | field-asym | 1008 | 98.4 % | 95.8 % | 0.80 [0.74, 0.87] | 45.0 % |
| ADR-0068 | hostile-c | 1008 | 95.2 % | 92.8 % | 0.81 [0.75, 0.87] | 50.9 % |
| ADR-0068 | hostile | 1008 | 95.3 % | 92.3 % | 1.50 [1.41, 1.60] | 67.6 % |
| ADR-0068, 6-frame | ideal | 1008 | 100.0 % | 99.2 % | 0.17 [0.14, 0.20] | 13.5 % |
| ADR-0068, 6-frame | field | 1008 | 97.9 % | 95.7 % | 0.76 [0.70, 0.82] | 45.9 % |
| ADR-0068, 6-frame | field-asym | 1008 | 98.2 % | 96.4 % | 0.80 [0.73, 0.86] | 47.0 % |
| ADR-0068, 6-frame | hostile-c | 1008 | 95.4 % | 92.5 % | 0.86 [0.79, 0.92] | 51.2 % |
| ADR-0068, 6-frame | hostile | 1008 | 94.5 % | 92.0 % | 1.44 [1.35, 1.53] | 67.7 % |
| D: granted slots | ideal | 1008 | 100.0 % | 100.0 % | 0.00 [0.00, 0.00] | 0.0 % |
| D: granted slots | field | 1008 | 100.0 % | 100.0 % | 0.00 [0.00, 0.00] | 0.0 % |
| D: granted slots | field-asym | 1008 | 99.3 % | 99.1 % | 0.00 [0.00, 0.00] | 0.0 % |
| D: granted slots | hostile-c | 1008 | 99.9 % | 99.9 % | 0.00 [0.00, 0.00] | 0.0 % |
| D: granted slots | hostile | 1008 | 99.8 % | 99.0 % | 0.00 [0.00, 0.00] | 0.0 % |
| C: fixed cycle | ideal | 1008 | 100.0 % | 100.0 % | 0.00 [0.00, 0.00] | 0.0 % |
| C: fixed cycle | field | 1008 | 99.8 % | 99.8 % | 0.00 [0.00, 0.00] | 0.0 % |
| C: fixed cycle | field-asym | 1008 | 97.6 % | 97.0 % | 0.00 [0.00, 0.00] | 0.0 % |
| C: fixed cycle | hostile-c | 1008 | 99.9 % | 99.9 % | 0.00 [0.00, 0.00] | 0.0 % |
| C: fixed cycle | hostile | 1008 | 98.6 % | 96.1 % | 0.00 [0.00, 0.00] | 0.1 % |

### Overlap mechanisms (RF overlaps per run)

**Profile field**

| mechanism | legacy (protocol 7) | ADR-0068 | ADR-0068, 6-frame | D: granted slots | C: fixed cycle |
|---|---:|---:|---:|---:|---:|
| answer-answer | 0.031 | 0.014 | 0.013 | 0.000 | 0.000 |
| dual-holder | 0.001 | 0.000 | 0.003 | 0.000 | 0.000 |
| early-answer | 0.134 | 0.000 | 0.000 | 0.000 | 0.000 |
| late answer (decoded-late) | 0.244 | 0.067 | 0.063 | 0.000 | 0.000 |
| late answer (held) | 0.157 | 0.280 | 0.271 | 0.000 | 0.000 |
| late answer (holder-moved-on) | 0.007 | 0.035 | 0.017 | 0.000 | 0.000 |
| late answer (holder-timeout) | 0.000 | 0.005 | 0.005 | 0.000 | 0.000 |
| unsolicited (request) | 0.227 | 0.392 | 0.388 | 0.000 | 0.000 |

**Profile hostile-c**

| mechanism | legacy (protocol 7) | ADR-0068 | ADR-0068, 6-frame | D: granted slots | C: fixed cycle |
|---|---:|---:|---:|---:|---:|
| answer-answer | 0.169 | 0.007 | 0.009 | 0.000 | 0.000 |
| dual-holder | 0.022 | 0.013 | 0.012 | 0.000 | 0.000 |
| early-answer | 0.473 | 0.001 | 0.000 | 0.000 | 0.000 |
| late answer (decoded-late) | 0.220 | 0.085 | 0.060 | 0.000 | 0.000 |
| late answer (held) | 1.436 | 0.392 | 0.449 | 0.000 | 0.000 |
| late answer (holder-moved-on) | 0.000 | 0.003 | 0.001 | 0.000 | 0.000 |
| late answer (holder-timeout) | 0.001 | 0.000 | 0.000 | 0.000 | 0.000 |
| unsolicited (irs_disc) | 0.000 | 0.001 | 0.000 | 0.000 | 0.000 |
| unsolicited (leave) | 0.005 | 0.000 | 0.000 | 0.000 | 0.000 |
| unsolicited (request) | 0.271 | 0.306 | 0.324 | 0.000 | 0.000 |

### Completion time against ADR-0068, paired by seed

Geometric mean of the ratio of completion times over the runs both arms completed
(< 1: faster than the reference arm), 95 % bootstrap interval over pairs; and how
many of the paired runs each completed.

| profile | workload | legacy (protocol 7) | ADR-0068, 6-frame | D: granted slots | C: fixed cycle |
|---|---|---|---|---|---|
| ideal | bulk | ×0.982 [0.967, 0.995]<br>336 vs 336 of 336 | ×0.975 [0.962, 0.989]<br>336 vs 336 of 336 | ×1.044 [1.027, 1.063]<br>336 vs 336 of 336 | ×1.248 [1.226, 1.271]<br>336 vs 336 of 336 |
| ideal | exchange | ×0.955 [0.945, 0.964]<br>336 vs 336 of 336 | ×0.970 [0.960, 0.979]<br>336 vs 336 of 336 | ×1.017 [1.003, 1.031]<br>336 vs 336 of 336 | ×1.507 [1.480, 1.536]<br>336 vs 336 of 336 |
| ideal | chat | ×0.990 [0.985, 0.996]<br>336 vs 336 of 336 | ×1.001 [0.998, 1.004]<br>336 vs 336 of 336 | ×1.083 [1.074, 1.092]<br>336 vs 336 of 336 | ×1.473 [1.453, 1.493]<br>336 vs 336 of 336 |
| field | bulk | ×0.873 [0.857, 0.888]<br>336 vs 336 of 336 | ×0.964 [0.950, 0.978]<br>336 vs 336 of 336 | ×0.962 [0.943, 0.982]<br>336 vs 336 of 336 | ×1.167 [1.138, 1.197]<br>336 vs 335 of 336 |
| field | exchange | ×0.821 [0.807, 0.836]<br>333 vs 336 of 336 | ×0.931 [0.917, 0.945]<br>333 vs 332 of 336 | ×0.905 [0.889, 0.922]<br>333 vs 336 of 336 | ×1.336 [1.306, 1.369]<br>333 vs 335 of 336 |
| field | chat | ×0.988 [0.981, 0.995]<br>325 vs 330 of 336 | ×0.997 [0.993, 1.000]<br>325 vs 319 of 336 | ×1.024 [1.016, 1.034]<br>325 vs 336 of 336 | ×1.462 [1.440, 1.484]<br>325 vs 336 of 336 |
| field-asym | bulk | ×0.882 [0.867, 0.899]<br>336 vs 334 of 336 | ×0.954 [0.941, 0.968]<br>336 vs 335 of 336 | ×0.983 [0.961, 1.007]<br>336 vs 336 of 336 | ×1.193 [1.162, 1.225]<br>336 vs 334 of 336 |
| field-asym | exchange | ×0.818 [0.804, 0.831]<br>327 vs 333 of 336 | ×0.963 [0.949, 0.976]<br>327 vs 325 of 336 | ×0.915 [0.898, 0.934]<br>327 vs 329 of 336 | ×1.437 [1.394, 1.478]<br>327 vs 317 of 336 |
| field-asym | chat | ×0.985 [0.975, 0.994]<br>329 vs 325 of 336 | ×1.002 [0.997, 1.008]<br>329 vs 330 of 336 | ×0.981 [0.969, 0.993]<br>329 vs 336 of 336 | ×1.501 [1.474, 1.529]<br>329 vs 333 of 336 |
| hostile-c | bulk | ×0.897 [0.876, 0.919]<br>336 vs 335 of 336 | ×0.906 [0.888, 0.925]<br>336 vs 336 of 336 | ×0.948 [0.926, 0.971]<br>336 vs 336 of 336 | ×1.088 [1.062, 1.117]<br>336 vs 335 of 336 |
| hostile-c | exchange | ×0.882 [0.867, 0.896]<br>322 vs 328 of 336 | ×0.948 [0.936, 0.960]<br>322 vs 321 of 336 | ×0.939 [0.923, 0.957]<br>322 vs 335 of 336 | ×1.283 [1.249, 1.318]<br>322 vs 336 of 336 |
| hostile-c | chat | ×1.019 [1.010, 1.030]<br>302 vs 304 of 336 | ×0.998 [0.993, 1.003]<br>302 vs 305 of 336 | ×0.997 [0.987, 1.006]<br>302 vs 336 of 336 | ×1.392 [1.371, 1.412]<br>302 vs 336 of 336 |
| hostile | bulk | ×0.859 [0.841, 0.878]<br>335 vs 336 of 336 | ×0.933 [0.916, 0.950]<br>335 vs 336 of 336 | ×0.961 [0.937, 0.985]<br>335 vs 336 of 336 | ×1.432 [1.386, 1.482]<br>335 vs 330 of 336 |
| hostile | exchange | ×0.789 [0.772, 0.807]<br>322 vs 321 of 336 | ×0.913 [0.896, 0.930]<br>322 vs 312 of 336 | ×0.960 [0.940, 0.981]<br>322 vs 334 of 336 | ×1.492 [1.456, 1.528]<br>322 vs 328 of 336 |
| hostile | chat | ×1.026 [1.015, 1.038]<br>304 vs 290 of 336 | ×1.004 [0.999, 1.008]<br>304 vs 305 of 336 | ×0.994 [0.982, 1.005]<br>304 vs 336 of 336 | ×1.541 [1.512, 1.571]<br>304 vs 336 of 336 |

### Completion time against legacy (protocol 7), paired by seed

Geometric mean of the ratio of completion times over the runs both arms completed
(< 1: faster than the reference arm), 95 % bootstrap interval over pairs; and how
many of the paired runs each completed.

| profile | workload | ADR-0068 | ADR-0068, 6-frame | D: granted slots | C: fixed cycle |
|---|---|---|---|---|---|
| ideal | bulk | ×1.019 [1.005, 1.034]<br>336 vs 336 of 336 | ×0.994 [0.984, 1.003]<br>336 vs 336 of 336 | ×1.064 [1.047, 1.080]<br>336 vs 336 of 336 | ×1.272 [1.249, 1.295]<br>336 vs 336 of 336 |
| ideal | exchange | ×1.047 [1.037, 1.058]<br>336 vs 336 of 336 | ×1.016 [1.007, 1.024]<br>336 vs 336 of 336 | ×1.065 [1.051, 1.080]<br>336 vs 336 of 336 | ×1.578 [1.551, 1.606]<br>336 vs 336 of 336 |
| ideal | chat | ×1.010 [1.005, 1.015]<br>336 vs 336 of 336 | ×1.010 [1.006, 1.015]<br>336 vs 336 of 336 | ×1.093 [1.084, 1.102]<br>336 vs 336 of 336 | ×1.487 [1.466, 1.508]<br>336 vs 336 of 336 |
| field | bulk | ×1.146 [1.126, 1.167]<br>336 vs 336 of 336 | ×1.105 [1.086, 1.125]<br>336 vs 336 of 336 | ×1.103 [1.083, 1.123]<br>336 vs 336 of 336 | ×1.338 [1.312, 1.365]<br>336 vs 335 of 336 |
| field | exchange | ×1.217 [1.197, 1.240]<br>336 vs 333 of 336 | ×1.133 [1.115, 1.150]<br>336 vs 332 of 336 | ×1.101 [1.085, 1.119]<br>336 vs 336 of 336 | ×1.626 [1.594, 1.658]<br>336 vs 335 of 336 |
| field | chat | ×1.012 [1.005, 1.019]<br>330 vs 325 of 336 | ×1.009 [1.002, 1.016]<br>330 vs 319 of 336 | ×1.035 [1.026, 1.044]<br>330 vs 336 of 336 | ×1.479 [1.458, 1.502]<br>330 vs 336 of 336 |
| field-asym | bulk | ×1.133 [1.113, 1.154]<br>334 vs 336 of 336 | ×1.082 [1.063, 1.101]<br>334 vs 335 of 336 | ×1.116 [1.094, 1.140]<br>334 vs 336 of 336 | ×1.353 [1.321, 1.384]<br>334 vs 334 of 336 |
| field-asym | exchange | ×1.223 [1.204, 1.244]<br>333 vs 327 of 336 | ×1.179 [1.158, 1.198]<br>333 vs 325 of 336 | ×1.123 [1.103, 1.142]<br>333 vs 329 of 336 | ×1.752 [1.701, 1.800]<br>333 vs 317 of 336 |
| field-asym | chat | ×1.015 [1.006, 1.025]<br>325 vs 329 of 336 | ×1.017 [1.007, 1.028]<br>325 vs 330 of 336 | ×0.997 [0.985, 1.009]<br>325 vs 336 of 336 | ×1.525 [1.495, 1.555]<br>325 vs 333 of 336 |
| hostile-c | bulk | ×1.115 [1.088, 1.141]<br>335 vs 336 of 336 | ×1.010 [0.990, 1.029]<br>335 vs 336 of 336 | ×1.057 [1.036, 1.078]<br>335 vs 336 of 336 | ×1.214 [1.185, 1.243]<br>335 vs 335 of 336 |
| hostile-c | exchange | ×1.134 [1.115, 1.153]<br>328 vs 322 of 336 | ×1.070 [1.053, 1.087]<br>328 vs 321 of 336 | ×1.063 [1.046, 1.080]<br>328 vs 335 of 336 | ×1.457 [1.423, 1.492]<br>328 vs 336 of 336 |
| hostile-c | chat | ×0.981 [0.971, 0.991]<br>304 vs 302 of 336 | ×0.983 [0.973, 0.993]<br>304 vs 305 of 336 | ×0.976 [0.966, 0.987]<br>304 vs 336 of 336 | ×1.371 [1.350, 1.391]<br>304 vs 336 of 336 |
| hostile | bulk | ×1.164 [1.138, 1.189]<br>336 vs 335 of 336 | ×1.086 [1.066, 1.107]<br>336 vs 336 of 336 | ×1.119 [1.094, 1.144]<br>336 vs 336 of 336 | ×1.675 [1.626, 1.726]<br>336 vs 330 of 336 |
| hostile | exchange | ×1.268 [1.240, 1.295]<br>321 vs 322 of 336 | ×1.149 [1.127, 1.171]<br>321 vs 312 of 336 | ×1.208 [1.183, 1.233]<br>321 vs 334 of 336 | ×1.894 [1.844, 1.946]<br>321 vs 328 of 336 |
| hostile | chat | ×0.975 [0.963, 0.985]<br>290 vs 304 of 336 | ×0.980 [0.968, 0.990]<br>290 vs 305 of 336 | ×0.969 [0.955, 0.983]<br>290 vs 336 of 336 | ×1.506 [1.477, 1.536]<br>290 vs 336 of 336 |

### Completion time against ADR-0068 by air and channel (field profile)

| air | channel | workload | legacy (protocol 7) | ADR-0068, 6-frame | D: granted slots | C: fixed cycle |
|---|---|---|---|---|---|---|
| 2300 | awgn | bulk | ×0.91 | ×0.98 | ×1.00 | ×1.26 |
| 2300 | awgn | exchange | ×0.85 | ×0.92 | ×0.88 | ×1.41 |
| 2300 | awgn | chat | ×1.00 | ×1.00 | ×1.02 | ×1.43 |
| 2300 | good | bulk | ×0.85 | ×0.99 | ×0.95 | ×1.22 |
| 2300 | good | exchange | ×0.86 | ×0.94 | ×0.97 | ×1.50 |
| 2300 | good | chat | ×0.98 | ×1.00 | ×1.03 | ×1.49 |
| 2300 | moderate | bulk | ×0.86 | ×0.96 | ×0.98 | ×1.21 |
| 2300 | moderate | exchange | ×0.83 | ×0.93 | ×0.90 | ×1.44 |
| 2300 | moderate | chat | ×0.99 | ×1.00 | ×1.04 | ×1.53 |
| 2300 | poor | bulk | ×0.93 | ×0.99 | ×1.02 | ×1.27 |
| 2300 | poor | exchange | ×0.84 | ×0.96 | ×0.93 | ×1.43 |
| 2300 | poor | chat | ×1.01 | ×1.00 | ×1.06 | ×1.51 |
| 500 | awgn | bulk | ×0.85 | ×0.95 | ×0.98 | ×1.14 |
| 500 | awgn | exchange | ×0.80 | ×0.92 | ×0.87 | ×1.25 |
| 500 | awgn | chat | ×0.99 | ×1.00 | ×1.00 | ×1.37 |
| 500 | good | bulk | ×0.84 | ×0.89 | ×0.92 | ×1.04 |
| 500 | good | exchange | ×0.82 | ×0.94 | ×0.95 | ×1.26 |
| 500 | good | chat | ×0.97 | ×0.99 | ×1.00 | ×1.47 |
| 500 | moderate | bulk | ×0.87 | ×1.00 | ×0.94 | ×1.11 |
| 500 | moderate | exchange | ×0.77 | ×0.92 | ×0.86 | ×1.23 |
| 500 | moderate | chat | ×0.96 | ×0.99 | ×1.03 | ×1.46 |
| 500 | poor | bulk | ×0.87 | ×0.95 | ×0.92 | ×1.10 |
| 500 | poor | exchange | ×0.82 | ×0.92 | ×0.88 | ×1.20 |
| 500 | poor | chat | ×1.00 | ×1.00 | ×1.02 | ×1.43 |

### Bulk goodput (bit/s, median over trials and channels), field profile

| air | SNR dB | legacy (protocol 7) | ADR-0068 | ADR-0068, 6-frame | D: granted slots | C: fixed cycle |
|---|---:|---:|---:|---:|---:|---:|
| 2300 | -6.0 | 110 | 92 | 93 | 103 | 92 |
| 2300 | -3.0 | 159 | 132 | 132 | 144 | 119 |
| 2300 | 0.0 | 148 | 116 | 123 | 130 | 113 |
| 2300 | 3.0 | 209 | 201 | 210 | 197 | 145 |
| 2300 | 6.0 | 385 | 361 | 353 | 356 | 268 |
| 2300 | 9.0 | 606 | 611 | 618 | 553 | 421 |
| 2300 | 12.0 | 830 | 818 | 815 | 726 | 507 |
| 500 | -6.0 | 59 | 46 | 46 | 53 | 49 |
| 500 | -3.0 | 69 | 55 | 55 | 55 | 58 |
| 500 | 0.0 | 73 | 60 | 63 | 63 | 56 |
| 500 | 3.0 | 86 | 72 | 82 | 92 | 73 |
| 500 | 6.0 | 126 | 119 | 141 | 136 | 105 |
| 500 | 9.0 | 203 | 192 | 212 | 198 | 163 |
| 500 | 12.0 | 307 | 313 | 305 | 284 | 241 |

### Chat: a line's latency and the air it costs

Median over sessions of each session's median line latency; 90th percentile
over sessions of each session's slowest line; transmitters' duty cycle (keyed
seconds over session seconds, both stations).

| profile | arm | sessions completed | median line latency s | p90 of slowest s | duty |
|---|---|---:|---:|---:|---:|
| field | legacy (protocol 7) | 330/336 | 7.7 | 38.3 | 0.400 |
| field | ADR-0068 | 325/336 | 8.5 | 36.5 | 0.429 |
| field | ADR-0068, 6-frame | 319/336 | 7.9 | 35.4 | 0.424 |
| field | D: granted slots | 336/336 | 8.3 | 44.3 | 0.368 |
| field | C: fixed cycle | 336/336 | 17.5 | 95.3 | 0.302 |
| hostile-c | legacy (protocol 7) | 304/336 | 10.2 | 53.9 | 0.406 |
| hostile-c | ADR-0068 | 302/336 | 10.2 | 45.1 | 0.405 |
| hostile-c | ADR-0068, 6-frame | 305/336 | 10.1 | 44.0 | 0.402 |
| hostile-c | D: granted slots | 336/336 | 9.8 | 50.1 | 0.351 |
| hostile-c | C: fixed cycle | 336/336 | 20.1 | 97.2 | 0.291 |

### Air per delivered kilobyte (keyed seconds, both stations), field profile

| workload | legacy (protocol 7) | ADR-0068 | ADR-0068, 6-frame | D: granted slots | C: fixed cycle |
|---|---:|---:|---:|---:|---:|
| bulk | 42.7 | 53.2 | 47.1 | 46.5 | 53.3 |
| exchange | 47.0 | 59.6 | 50.9 | 48.7 | 61.4 |
| chat | 118.5 | 141.6 | 136.1 | 113.4 | 153.5 |

### How sessions that did not complete ended (all profiles)

| arm | not completed | most common ends (caller, called) |
|---|---:|---|
| legacy (protocol 7) | 124 | link timeout / peer disconnected: 40; up / up: 39; link timeout / link timeout: 24; peer disconnected / link timeout: 21 |
| ADR-0068 | 125 | link timeout / link timeout: 117; up / up: 7; link timeout / peer disconnected: 1 |
| ADR-0068, 6-frame | 140 | link timeout / link timeout: 126; up / up: 9; link timeout / peer disconnected: 5 |
| D: granted slots | 10 | up / up: 10 |
| C: fixed cycle | 41 | up / up: 33; no response / link timeout: 8 |

