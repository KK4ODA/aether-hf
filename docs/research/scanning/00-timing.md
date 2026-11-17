# A scanning gateway against Aether's call: the timing

*For ADR-0129. Run 2026-11-17 in a cloud container, on the model's engine at `78e8b23` (link
protocol 13, ADR-0126's call). Timing only: an estimate of the proposal's rules, with no channel
behind it. The harness judges the design (ADR-0129 §6). The scripts are in `scratch/` (its
README says how to run them).*

## 1. The call, from the engine

`scratch/call_schedules.py` runs two `GrantedEngine`s through the model's `TwoStationSim` with
the called station deaf, so every call runs to "no answer", and records what the caller keys. It
covers each air, the plan for a station not heard lately (H, both-ways tries) and for one heard
weak (S, floor tries), and 2 to 15 tries, with twenty seeds each (the pause has a random part).

**The two airs' calls are identical.** The OFDM data frame is 1.054 s on both, and the floor's
frames are the same frames.

| | H (not heard lately) | S (heard weak) |
|---|---|---|
| A try | 6.41 s: the OFDM data frame (1.05 s), then tone-24 (5.36 s) | tone-24, 5.36 s |
| From one try's start to the next, before the pause | 11.0 s at first, 13.0 s from the eighth | 10.0 s at first, 12.0 s from the eighth |
| Before the last try (the pause) | 26.1–34.0 s | 25.1–32.0 s |

When a call nobody answers ends:

| Tries | H | S |
|---|---|---|
| 2 | 39.1–45.1 s | 37.0–42.0 s |
| 4 | 61.7–67.6 s | 57.5–62.4 s |
| 5 | 73.2–79.1 s | 67.9–72.8 s |
| 6 | 85.2–91.2 s | 78.8–83.8 s |
| 8 | 110.2–116.2 s | 101.8–106.8 s |
| 10 | 136.2–142.2 s | 125.7–130.7 s |
| 15 | 201.3–207.2 s | 185.5–190.4 s |

## 2. The model of a scanning gateway

`scratch/scan_timing.py`. The gateway visits its *n* channels in turn, one dwell on each. The
caller is on one of them, chosen at random, and starts at a random moment of the gateway's cycle.
6 000 calls a cell.

* **Settling.** After a hop the gateway hears nothing for 0.5 s: the sound card's input latency
  (`DEVICE_LATENCY_S`, 0.25 s) and the radio's own move. This is an estimate.
* **A part is heard** when the gateway is on the caller's channel, settled, at the part's start.
  A part is an H try's OFDM part or its floor part, or an S try. A part already under way when
  the gateway arrives is not counted. That is conservative: the floor's middle and end sync blocks
  can find a frame whose start was missed (ADR-0070).
* **A part heard holds the gateway to its end.** The scan never leaves a frame that is arriving.
* **A part heard decodes** with probability *p*. *p* = 1 stands for a good path; *p* = 0.5 for a
  weak one, each part drawn on its own (a real fade joins neighbouring parts).
* **Linger.** After a part heard and not decoded, the gateway stays on for up to the linger, and
  hears the next part that starts in it.

Neither number is tuned. Both come from §1's calls:

* **The dwell** is the longest gap between two tries' starts before the pause, plus the settling:
  13.01 + 0.5 = **13.5 s**.
* **The linger** is the longest gap of all (the pause), plus the settling: 34.04 + 0.5 =
  **34.5 s**.

## 3. The dwell

The tries are those of the rule in §4. Each cell is *p* = 1, plan H, without jitter / with each
dwell drawn ±25 %:

| Channels (tries) | 3.5 s | 6.5 s | **13.5 s (derived)** | 27 s |
|---|---|---|---|---|
| 2 (4) | 1.000 / 0.999 | 0.910 / 0.916 | **1.000 / 1.000** | 0.958 / 0.966 |
| 3 (5) | 0.663 / 0.715 | 0.980 / 0.958 | **1.000 / 1.000** | 0.974 / 0.962 |
| 4 (6) | 1.000 / 0.976 | 0.941 / 0.916 | **1.000 / 0.995** | 0.932 / 0.907 |
| 6 (8) | 0.917 / 0.871 | 0.988 / 0.932 | **1.000 / 0.986** | 0.761 / 0.765 |

* **A short dwell locks in phase with the call.** At 3.5 s, about Trimode's cycle, three channels
  come round every 10.5 s and the tries every 11–11.5 s. A try that lands while the gateway is
  away is followed by tries that land the same way, and two calls in three are caught. The
  calling bench saw the same thing against its Trimode stand-in. There the callsign fixed where
  the tries fell against the scan, so every draw repeated the miss (`../calling/01-results.md`
  §3.1, §3.4).
* **Jitter does not cure it** (0.663 → 0.715), and at the derived dwell it costs a little (6
  channels: 1.000 → 0.986). It is left out.
* **The derived dwell catches every call at every count.** A visit at least as long as the gap
  between two tries always holds a try's start while the call is going. The part heard then holds
  the gateway to its end.
* **A longer dwell is worse.** The cycle grows faster than the visits gain.

## 4. The call's tries

**The rule:** the fewest tries whose tries before the pause start over at least (*n* − 1)
dwells. The columns:

* *Fewest*: the smallest number of tries that reaches 0.99 at *p* = 1, found by trying each.
* *Median*: when the try that was caught ended, measured from the call's start.
* *2 tries*: today's default, for comparison.

| Plan | Channels | Cycle | Tries | Ends by | *p* = 1 | *p* = 0.5 | *p* = 0.5, linger | Median | Fewest | 2 tries |
|---|---|---|---|---|---|---|---|---|---|---|
| H | 2 | 27.0 s | 4 | 67.6 s | 1.000 | 0.891 | 0.986 | 6.4 s | 4 | 0.607 |
| H | 3 | 40.5 s | 5 | 79.1 s | 1.000 | 0.852 | 0.989 | 12.1 s | 5 | 0.628 |
| H | 4 | 54.1 s | 6 | 91.2 s | 1.000 | 0.828 | 0.988 | 23.1 s | 6 | 0.518 |
| H | 5 | 67.6 s | 7 | 103.2 s | 1.000 | 0.816 | 0.984 | 23.1 s | 7 | 0.423 |
| H | 6 | 81.1 s | 8 | 116.2 s | 1.000 | 0.804 | 0.991 | 34.6 s | 8 | 0.343 |
| H | 8 | 108.1 s | 10 | 142.2 s | 1.000 | 0.792 | 0.990 | 46.1 s | 10 | 0.255 |
| S | 2 | 27.0 s | 4 | 62.4 s | 1.000 | 0.697 | 0.889 | 15.3 s | 4 | 0.540 |
| S | 3 | 40.5 s | 5 | 72.8 s | 1.000 | 0.617 | 0.895 | 15.3 s | 5 | 0.612 |
| S | 4 | 54.1 s | 6 | 83.8 s | 0.997 | 0.617 | 0.906 | 25.3 s | 6 | 0.478 |
| S | 5 | 67.6 s | 8 | 106.8 s | 1.000 | 0.623 | 0.951 | 35.7 s | 8 | 0.385 |
| S | 6 | 81.1 s | 9 | 118.7 s | 1.000 | 0.613 | 0.949 | 35.7 s | 9 | 0.318 |
| S | 8 | 108.1 s | 11 | 142.6 s | 1.000 | 0.564 | 0.951 | 46.2 s | 11 | 0.231 |

* **The rule's count is the fewest that works, in every row.** For plan H it is *n* + 2 tries.
* **Two tries do not find a scanning gateway.** Even at two channels they catch 54–61 %.
* **A scanning gateway costs the caller channel time.** At four channels a call nobody answers
  lasts about 91 s, not about 42. Six tries was already one of Setup's choices (ADR-0126 §7.4); the
  difference here is that the gateway says how many are needed.

## 5. The linger

On a path where each part decodes half the time, a gateway that leaves at the end of its dwell
misses the next try. Lingering after a part it heard and could not read catches 98–99 % with plan
H at every count, against 79–89 % without. With plan S it reaches 89–95 %, against 56–70 %:
an S try has one part, an H try two.

## 6. What this does not say

* There is no channel. Real parts fade together, a weak OFDM part is often followed by a floor
  part that decodes, and a frame can be found from its middle.
* A hop is modelled as 0.5 s of deafness. The radio, its AGC and the CAT round trip are not
  measured.
* There is one caller. There is no traffic from other stations to hold a linger, and nothing on
  the gateway's own channels.
* The call is today's (link protocol 13). A change to the call's timing changes the dwell, which
  is why the dwell is computed from the engine and not written down.

The harness has one channel today. ADR-0129 §6 says what it needs before the design is judged
there.
