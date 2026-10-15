# ADR-0072: A disconnect waits a minute at most, and says so

**Status:** accepted, 2026-10-15. Model first (`aether_model/link/granted.py`:
`GrantConfig.disc_drain_s`, `leave_by`, `_can_leave`, `_say_left`), then the port
(`aether-link/src/granted.rs`) and the replay vectors (`slow-drain`). Daemon: `Station::disconnect`,
`Station::leave_in_s`, `status.leave_in_s`. Panel: the session banner, the line under the call
buttons, Disconnect's tooltip and the banner of a session that ended. No wire change: the DISC is
the same frame, only its timing changes. A station on this build and one on v0.2.0-beta.106 or
beta.107 work together; an older caller keeps the old wait.

## 1. Context

The nightly soak of 2026-10-15 (run 33) failed on three of its 24 random scenarios. In each,
both stations were still connected after the script's disconnects and five minutes of waiting
(`soak-330001`, `-330004`, `-330005`):

* **The paths were slow but alive.** They carried 12–31 bit/s: 500 Hz flutter at +1 dB one way,
  a 2300 Hz path at −10 dB, a scanning gateway's 6 kB. The operator had 6–15 kB queued when
  Disconnect was pressed.
* **ADR-0039's patience never ran out.** It waits `max(20 s, 2 exchanges)` with nothing
  acknowledged. An exchange is a worst-case figure, 76.8 s on both airs, so the patience is
  154 s. Every acknowledgement starts it again. A path that delivers a frame every minute or two
  never reaches it. At 30 bit/s, 15 kB is about 67 minutes.
* **The build made no difference.** The same three failed on the build before the day's changes,
  with the same air times (1065, 1153 and 1083 s).

The operator's view is the same as ND1J's and KE4QCM's (ADR-0023, ADR-0039): Disconnect seems to
do nothing. The author's direction (2026-10-15): cap the wait, and tell the operator. Otherwise
the button looks broken. Also tell them that Abort closes at once.

## 2. Decision

1. **The wait has a limit, counted from the request.** `disc_drain_s` is 60 s.
   * The clock starts at the operator's own Disconnect at the caller. At the called station's
     request, it starts when the caller decodes the `flag` in an answer.
   * Once the limit has passed, the caller's next `proceed` sends the DISC, delivered or not.
   * The patience stays. It still closes sooner when nothing moves.
   * The clock starts only on a request or a decoded frame. No deadline moves on undecoded
     evidence (ADR-0069 §5).
2. **The caller says what it left**, once, as it decides to close. It sends an event
   `disconnect` such as:
   * `10228 bytes not delivered; closing 66 s after the disconnect was asked`;
   * `the other station's last 7 frames not received; closing 65 s after …`;
   * `…; closing nothing acknowledged for 154 s`.

   Its own queue counts only when the disconnect is its own. When the called station asked, its
   operator has stopped receiving, so the caller's queue was never going to be sent
   (`07-protocol-8.md` §5.3).
3. **`leave_by()`** gives the engine time at which the limit falls. It is `None` until a
   disconnect is asked, and `None` once the session is no longer up.
4. **The daemon says so when Disconnect is pressed with something queued.** A `log` event
   `closing` reads: `14796 bytes still queued go first, for at most 60 s; Abort closes at once`.
   The called station says "about": the caller counts from when it reads the request, an answer
   later. `status.leave_in_s` is the seconds left, or null.
5. **The panel shows it where the operator is looking.**
   * The banner across every tab and the line under the call buttons say how much is queued,
     the seconds left, and that Abort closes now.
   * Disconnect's tooltip says "for a minute at most".
   * The banner of the ended session adds what was left, for example: *Session ended: closed.
     13788 bytes not delivered (66 s after the disconnect was asked).*
   * That banner stays red, like every ended session. Amber is for something still under way.
6. **Liveness L6** in `04-formal-spec.md`: a disconnect asked ends the session. "Allowed" in the
   `proceed` row now means:
   * both queues delivered; or
   * `disc_drain_s` since the request; or
   * the patience with nothing acknowledged.

   So the DISC goes within the minute and one exchange of the request. The session ends on the
   DISC_ACK, or after three DISCs.
7. **The benches lift the limit.** `bench_link.py`'s session and the research campaigns' bulk
   workload send and disconnect in the same instant, so for them the disconnect's wait *is* the
   transfer. They set `disc_drain_s = inf`, so their numbers keep their meaning.
   `--link disc_drain_s=60` measures the operator's case.

## 3. Alternatives considered

* **A limit in exchanges** (four, say). `_exchange_s` is a worst-case bound, 76.8 s on both airs,
  so four exchanges would be five minutes. That is longer than any operator waits before
  reaching for Abort.
* **A limit in bytes.** The operator does not know the rate, and the limit they feel is time.
* **A setting.** A new configuration key is a schema step and has to ship with a release.
  Nobody has asked to wait longer. Whoever wants a long transfer through waits for it before
  pressing Disconnect. The engine constant is there for benches.
* **Leave the wait open and only tell the operator.** The soak's stations stayed in their
  sessions for half an hour. A station left in a session answers no calls and blocks its
  frequency's other users. The notice alone does not fix that.

## 4. Measured

* **Model:**
  * `test_a_disconnect_on_a_slow_path_closes_within_its_minute_and_says_what_it_left`: a 15 kB
    queue at 0 dB, 500 Hz. It fails without the limit, still connected 150 s after the request.
  * `test_a_disconnect_with_a_small_queue_still_delivers_it_and_says_nothing`.
* **Port:** `granted_vectors.rs` replays the new `slow-drain` session input by input: the caller's
  own queue, then the called station's. No other session reaches the limit.
* **Daemon:** `a_disconnect_with_data_queued_says_how_long_it_may_take_and_that_abort_is_at_once`.
* **Harness:**
  * The three soak scenarios, on the same seeds, now end "closed / peer disconnected", idle,
    with no collision. Air time 1065 → 572 s, 1153 → 665 s and 1083 → 583 s.
  * Their messages still do not arrive. The soak does not judge that, and a path that slow
    should not deliver them.
  * The callers closed 60, 83 and 62 s after the request. The 83 s is a floor transmission
    under way when the minute ran out.
  * `bench/scenarios/80m-disconnect-slow-drain-500.toml` is `soak-330001`'s path with both
    operators' disconnects. It passes in 335 s of air: the caller closes 66 s after its request
    and says "13788 bytes not delivered". The called station's request is closed 79 s after the
    caller read it.

## 5. Consequences

* **On a slow path, Disconnect now drops what one more minute would not carry.** The log and the
  banner say how much. Anyone who wants all of it waits before pressing Disconnect, or sends it
  again later.
* **A host program's `DISCONNECT` is the same disconnect**, so the minute applies to it too.
  Winlink's B2F ends with `FQ`, and the program disconnects after it, with nothing left queued.
  A program that writes a long transfer and disconnects at once would see it cut after a
  minute; the host benches have not been rerun to look for one.
* **The Test is unaffected.** It disconnects only once its file has been acknowledged. A step
  that runs out of time aborts.
