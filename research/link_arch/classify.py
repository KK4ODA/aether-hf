"""What kind of transmission each keying was, and why two of them overlapped.

A transmission is one of three things with respect to channel access:

* **solicited** — an answer to a frame the station decoded: an acknowledgement, a
  connect acceptance, a DISC_ACK, the first burst or poll that answers a TURN, an
  acknowledgement that takes an offered turn with its burst after it;
* **holder** — sent on the station's own initiative as the station that holds the turn: a
  burst, a repeat after silence, a poll, a keepalive, a TURN, its DISC, a call;
* **unsolicited** — sent on its own initiative by a station that does *not* hold the turn:
  the receiving station's request for the turn (ADR-0027), its DISC between bursts
  (ADR-0023), an abort's DISC and its repeats (ADR-0052), a probe.

An overlap of the two transmitters is classified by the pair (first keyed, second keyed):

* ``keying`` — they overlap only in PTT lead or VOX hold, never in radiated signal;
* ``dual-holder`` — both were sent on initiative by stations that each held the turn;
* ``early-answer`` — an answer keyed while the transmission it answers was still on the air:
  the receiver's idea of where the burst ended was wrong;
* ``late-answer/<cause>`` — an answer and the holder's *next* transmission met: the answer
  went out after the window the holder had kept for it. Causes: ``held`` (the answering
  daemon or radio keyed it late), ``decoded-late`` (what it answers reached the engine late),
  ``holder-timeout`` (the holder's wait ran out while the answer was on time),
  ``holder-moved-on`` (the holder acted on something else — a stale answer — and keyed);
* ``unsolicited`` — one of the two was sent by a station that did not hold the turn;
* ``answer-answer`` — both were answers (each answering something of the other's);
* ``other``.
"""

from __future__ import annotations

from research.link_arch.advsim import TxRec

SOLICITED = "solicited"
HOLDER = "holder"
UNSOLICITED = "unsolicited"


def kind_of(rec: TxRec) -> tuple[str, str]:
    """(access class, what) for one transmission."""
    r = rec.reason
    trig = rec.trigger
    if r.startswith(("answer:", "holder:", "unsolicited:")):
        cls, what = r.split(":", 1)
        return {"answer": SOLICITED}.get(cls, cls), what
    # the model's engine: read the chain of methods that keyed it
    if "_repeat_leave" in r or "_answer_left" in r:
        return UNSOLICITED, "leave"
    if "_send_probe" in r:
        return (SOLICITED, "probe_ack") if "_handle_probe" in r else (UNSOLICITED, "probe")
    if "_send_connect" in r:
        if "_handle_connect_req" in r or "_accept" in r:
            return SOLICITED, "connect_ack"
        return HOLDER, "connect_req"
    if "_send_request" in r:
        return UNSOLICITED, "request"
    if trig.startswith("abort") or r.endswith("abort"):
        return UNSOLICITED, "abort_disc"
    if "_on_control" in r and "disc_ack" in rec.kinds:
        return SOLICITED, "disc_ack"
    if "_answer_burst" in r or "_send_ack" in r:
        if "_send_disc" in r:
            return SOLICITED, "disc_for_ack"
        if "data" in rec.kinds:
            return SOLICITED, "ack+burst"
        return SOLICITED, "ack"
    if "_answer_turn" in r:
        return SOLICITED, "answer_turn"
    if "_send_disc" in r:
        if "disconnect" in trig and rec.belief_self.get("role") == "irs":
            return UNSOLICITED, "irs_disc"
        return HOLDER, "disc"
    if "_send_turn" in r:
        return HOLDER, "turn"
    if "_send_poll" in r:
        if "timer:keepalive" in trig:
            return HOLDER, "keepalive"
        if "_repeat" in r or "_resend" in r:
            return HOLDER, "repeat_poll"
        return HOLDER, "poll"
    if "_send_burst" in r:
        if "_repeat" in r or "_resend" in r:
            return HOLDER, "repeat_burst"
        return HOLDER, "burst"
    if "_on_control" in r:
        return SOLICITED, "control_answer"
    return "?", r or trig


def classify(x: TxRec, y: TxRec, keyed_overlap: float, rf_overlap: float) -> str:
    """The mechanism of one overlap; ``x`` keyed first."""
    if rf_overlap <= 0.0:
        return "keying"
    cx, wx = kind_of(x)
    cy, wy = kind_of(y)
    if UNSOLICITED in (cx, cy):
        what = wy if cy == UNSOLICITED else wx
        return f"unsolicited/{what}"
    if cx == HOLDER and cy == HOLDER:
        return "dual-holder"
    if cx == HOLDER and cy == SOLICITED:
        if y.anchor is not None and y.anchor >= x.rad[0] - 0.05:
            # it answered the very transmission still on the air: where that ended was
            # misjudged (a misread count, a phantom, an inferred end)
            return "early-answer"
        # it answered something before: the answer came after the holder had moved on
        return "late-answer/" + _late_cause(y, x)
    if cx == SOLICITED and cy == HOLDER:
        return "late-answer/" + _late_cause(x, y)
    if cx == SOLICITED and cy == SOLICITED:
        return "answer-answer"
    return f"other/{wx}/{wy}"


def _late_cause(answer: TxRec, holder: TxRec) -> str:
    """Why an answer and the holder's next transmission met: the answering daemon or radio
    keyed it late, its engine decoded what it answers late, or the holder did not wait."""
    if answer.hold_s > 0.05 or answer.lead_extra_s > 0.1:
        return "held"
    if answer.anchor is not None and answer.t_req - answer.anchor > 0.5:
        return "decoded-late"
    if "timer:wait" in holder.trigger or "timer:resend" in holder.trigger:
        return "holder-timeout"
    return "holder-moved-on"
