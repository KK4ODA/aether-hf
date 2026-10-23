# ADR-0092: A measurement whose slot went silent is measured again

**Status:** accepted, 2026-10-23, the author's go on report 15's change 4
(`docs/research/link-architecture/15-wc4y-2300-beta120.md` §6, §11). Amends ADR-0076 §2's rule
for a measurement whose answer was not read, and R14 of `04-formal-spec.md`. The engine, model
first; **no wire change, no configuration key, no schema bump**.

## 1. Context

The Test's ladder measures each rung with frames of its own (ADR-0077). The other station reports
which arrived in every acknowledgement it sends, until the measuring station's data comes again.
A report does not say which measurement it is of.

ADR-0076 handled a measurement whose slot went silent: the master polls, the answer to the poll
reports the frames, and the rung is recorded with the frames counted and no SNR (the SNR is the
poll's). That is right when the slave read the measurement and only its answer was lost.

On WC4Y's Test of 2026-10-23 the other case happened. KK4ODA-1 missed rung 9's header and decoded
none of the three frames it detected; the slot went silent, and so did the next poll. The poll
after that, on the floor, was read, and KK4ODA-1, which had heard no new measurement, answered with
the report it held: rung 8's, frames 0, 1 and 3. The master took it for rung 9's. The Test recorded
rung 9 as 3 of 4 decoded, highest passed 9 where it was 8, and the ladder ran a rung further than
three failures in a row allow.

The master cannot tell the two cases apart: in both it sees a silent slot and then a report.

## 2. Decision

1. **A measurement whose slot went silent is asked again** (`_measurement_unheard`, model and
   port), as ADR-0077 already does for one the keying gate dropped or cut unread. The repeat's
   header is read or not; if read, the other station drops what it held and reports the repeat.
2. **Once.** The repeat's slot silent too, the rung is recorded with nothing decoded and no SNR,
   and the session goes on. Nothing came back from the rung twice, and whatever report follows
   may be another rung's.
3. ADR-0076's no-SNR rule stays for an answer read without a report: the report then follows a
   poll, its frames the rung's, its SNR the poll's.

The cost is one more transmission at the rung when only an answer was lost: 5–10 s at an OFDM
rung, about 25 s at a tone rung.

## 3. Measured

* `test_a_rung_the_slave_never_read_is_not_credited_with_the_rung_before` (model): rung 8, then
  rung 12 with its first measurement header lost and its frames above what the path carries.
  Before: rung 12 recorded as 4 of 4, rung 8's report. After: 0 of 4, with the repeat's SNR.
* `test_a_measurement_silent_twice_is_recorded_as_nothing_decoded`: every rung-12 header lost;
  recorded 0 of 4 with no SNR, the session still up.
* `test_a_ladder_rung_whose_answer_was_lost_is_measured_again` replaces ADR-0076's test of the
  lost answer: the rung goes twice and is reported from the repeat, with its SNR.
* The port: the granted vectors' `ladder-unheard` and `measure-unread` sessions change (their
  silent measurement slots are measured again) and a `measure-silent` session is added for the
  second silence; `granted_vectors.rs` replays every input and action of all 31 sessions. With
  the port's old rule the replay fails.
