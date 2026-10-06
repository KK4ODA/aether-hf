# ADR-0040: A faint arrival where the next frame of a burst would begin holds the acknowledgement

**Status:** accepted, 2026-10-06. The daemon only (`station.rs` `heed_preambles`,
`FOLLOW_ON_WINDOW_S`, `FOLLOW_ON_EARLY_S`, `StationStats::follow_on_heeded`;
`LinkEngine::acknowledgement_at` in the port). No wire change, no configuration change. ADR-0041
adds the burst's own countdown on the air; this rule stays beside it.

## 1. Context

ND1J called KK4ODA-1 on 3.590 MHz at 500 Hz on 2026-10-06 01:58Z and ran a Test session for
eight and a half minutes, the first both stations recorded. Matched frame by frame (ND1J's
`20261006-015832_ND1J_KK4ODA-1_test`, KK4ODA-1's `20261006-015852_KK4ODA-1_ND1J`, 19.66 s apart):

| KK4ODA-1's control frames | ND1J decoded | detected, not decoded | never detected |
|---|---|---|---|
| ordinary family (0.43 s) | 38 | 8 | 22 |
| tone floor | 9 | 0 | 1 |

21 of the 22 never detected went out while ND1J's own key was down: KK4ODA-1 acknowledged in the
middle of ND1J's burst. The frame it keyed over was lost too, and ND1J sent the burst again —
ND1J's counters: 30 acknowledgement timeouts, 135 frames resent; his Test's file step timed out.
Without the collisions he decoded 38 of 46 (83 %), in line with the 3–13 dB he reported hearing
this station at.

A burst carries no length (the DATA header must be identical across the retransmissions the
receiver soft-combines), so the receiver infers its end from silence: an acknowledgement is armed
a reply delay after each frame and moved past every frame it hears arriving. An arrival moves it
only when acquisition is trusted (`DETECT_CONFIDENCE_TRUSTED`, 1.3), because a crowded band makes
phantoms up to 1.54 and a phantom heeded held the station's own turn (OTA-2). On this path the
fourth frame of a burst often arrived faded: in 17 of the 21 collisions this station had
announced that frame at confidence 1.00–1.27 — often read as a 0.43 s control frame — and
ignored it. 35 arrivals were below the gate in the session; at least 17 were ND1J's frames.

## 2. Decision

While the engine owes a burst its acknowledgement (`acknowledgement_at`), an ordinary-family
arrival below the gate is heeded when it begins between `FOLLOW_ON_EARLY_S` (0.15 s) before and
`FOLLOW_ON_WINDOW_S` (0.6 s) after the end of the last frame heard (`heard_end`) — where the next
frame of a contiguous burst begins — and taken to be at least a long (data) frame. It is counted
(`follow_on_heeded`, in the counters) and recorded as `heeded 2`. Elsewhere, and with no
acknowledgement owed, the gate stands as before; tone arrivals are trusted already.

Replayed on the session's recorded announcements: 17 of the 21 collisions held, 2 acknowledgements
that did reach ND1J held a frame's length (about a second), none lost. The window was chosen there:
0.3 s held 10, 0.5 s 14, 0.6 and 0.8 s 17.

## 3. Consequences

* A phantom in the window costs a late acknowledgement; ignoring a real frame cost the frame, the
  acknowledgement and up to three repeat bursts. At the OTA-2 phantom rate (13.6 a minute) a
  0.75 s window catches one in about one burst end in six.
* It helps the receiving station only, and needs no change at the other end: beta.79 works with
  beta.78.
* Four of the collisions had no arrival at all in the window: the next frame was not detected.
  Only the burst saying it is not over can cover those (ADR-0041).
* Tests: `a_faint_arrival_where_the_next_frame_would_begin_holds_the_acknowledgement`,
  `a_faint_arrival_holds_nothing_when_no_acknowledgement_is_owed`.
