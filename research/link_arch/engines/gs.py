"""Candidate D, "granted slots": one master for the whole session, every slot granted.

The protocol in one paragraph
-----------------------------
The caller is the master for the whole session and the only station that ever keys on its
own initiative. Each master transmission is a header control frame (HDR) and up to
:attr:`~research.link_arch.engines.core.CoreConfig.max_frames` data frames; the header
acknowledges the slave's data, recommends the slave's rung, states how many data frames
follow it, and *grants* the slave one slot that starts a gap after the transmission ends:
an answer only, an answer and ``n`` data frames, or — when neither has anything to send — an
open window in which the slave may start whenever it has something. The slave keys only in a
slot it was granted by a master frame it decoded in time to start the slot on time (the
keying gate drops a transmission that would start late: the *forfeit* rule); its answer
(ANS) acknowledges the master's data, says how many data frames follow it and how many it
has queued. The master keys again a gap after the end a decoded slave frame stated, or a gap
after the latest end of the slot when it decoded nothing — never earlier, never on silence
alone. There is no turn and nothing to hand over: data flows both ways in every exchange,
sized by the grants. Answers are attributed to the slot they were sent in by their air time.

What it keeps from the model engine: the PHY frames, selective repeat with a 16-frame window,
HARQ with slot inference, the rate controller, re-encoding, the connect body. What it drops:
TURN, OFFER/TAKEN, POLL, the turn request, every announcement- or silence-driven timer, the
caller tie-break, ``_turn_unread``, ``_unread_there``, ``_took_unread`` and their kind.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Any

from aether_model.frame.modes import AirInterface
from aether_model.link.frames import (
    TURNS_VERSION,
    ConnectBody,
    DataHeader,
    DataKind,
    bandwidth_code,
    decode_data,
    encode_data,
)
from aether_model.link.harness import bandwidth_capabilities
from aether_model.link.phy import Container, PhyTiming, SoftFrame, TxFrame
from research.link_arch.engines.core import Core, CoreConfig, Ctl, Kind, State

GAP_STEPS = (0.0, 0.5, 1.0, 2.0)
"""Extra gap before the slave's slot, by level (two bits in the header, where the exchange id
was: answers are attributed by their air time). A slave whose computer decodes late asks for
a level in its answer; a master that hears nothing twice in a row raises it; both come down
as answers arrive on time. A longer gap costs dead air; a slot the slave cannot reach in time
costs the whole exchange."""


@dataclass
class GsConfig(CoreConfig):
    first_grant: int = 2
    """Data frames granted to the slave before it has said what it has (a gateway greets)."""
    early_reclaim: bool = False
    """The master takes a granted slot back when no preamble of the slave's has been heard by
    the slot's start, its slack and the family's announcement delay. Saves the rest of a lost
    grant; safe only if the master's detector never misses the start of a transmission it
    could otherwise not hear end either — a detection assumption, measured separately."""
    open_window_s: float = 4.0
    """An open window's base length on the ordinary family; the floor's is 2.5 times it, and
    each idle window in a row doubles it, up to :attr:`window_doublings` times."""
    window_doublings: int = 1
    elide: bool = True
    """Leave the header out of a data transmission when it would grant nothing new (see
    ``GsEngine._elide``): a bulk transfer's sender then sends what a turn-holding sender
    sends, data alone, and its header only when something changes."""
    """How far idle windows grow: the master's own new data waits up to the window, so this
    is its worst-case latency on an idle link against one header per window of air."""

    @classmethod
    def for_air(
        cls, air: AirInterface, arm: str = "gs", knobs: tuple[float, ...] = (0.0, 0.3)
    ) -> GsConfig:
        cfg = cls(max_mode=air.n_rungs - 1, capabilities=bandwidth_capabilities(air.params))
        cfg.turnaround_s, cfg.slack_s = knobs[0], knobs[1]
        if len(knobs) > 2:
            cfg.decode_budget_s = (knobs[2], knobs[3])
        if arm == "gs4":
            cfg.max_frames = 4
        if arm == "gs_cs":
            cfg.early_reclaim = True
        if arm.startswith("gs_w"):
            cfg.window_doublings = int(arm[4:])
        if arm.endswith("_m0"):
            cfg.answer_floor_margin_db = 0.0
        if arm.endswith("_noelide"):
            cfg.elide = False
        return cfg


def belief(engine: Any) -> dict[str, Any]:
    up = engine.state.value in ("connected", "disconnecting")
    return {
        "state": engine.state.value,
        "role": "master" if engine.master else "slave",
        "holder": up and engine.master,
    }


class GsEngine(Core):
    def __init__(self, my_call: str, timing: PhyTiming, cfg: GsConfig, seed: int = 0) -> None:
        self.gcfg = cfg
        super().__init__(my_call, timing, cfg, seed)

    def reset(self) -> None:
        super().reset()
        # master
        self._level = 0
        """The gap level the master grants with."""
        self._silence_level = 0
        self._asked_level = 0
        """The level the slave last asked for."""
        self._on_time = 0
        self._slot: dict[str, Any] | None = None
        self._stated_end: float | None = None
        self._stated_heard = 0.0
        self._slot_heard = False
        self._slave_want: int | None = None
        self._hdr_floor = True
        self._ans_cmd = True
        self._ans_prev = True
        self._possible: set[bool] = {True}
        """Every family the slave may hold for an answer to a transmission whose header it did
        not read: the family of its last answer read here, and every command sent since. A
        reservation covers the longest of them; a header is left out only when there is one,
        and it is the one wanted (header elision)."""
        self.elided = 0
        self._idle_windows = 0
        self._last_kind = ""
        self._silent = 0
        self._ans_t_end: float | None = None
        self._leave_asked = False
        self._patience_until: float | None = None
        # slave
        self._m_start: float | None = None
        self._m_end: float | None = None
        self._grant: Ctl | None = None
        self._answered: float | None = None
        self._ans_sticky = True
        self._grant_floor = False
        self._m_last_floor = False
        self._m_level = 0
        """Slave: the gap level of the last header read."""
        self._need_level = 0
        """Slave: the level its own decode lateness needs."""
        self._window: tuple[float, float] | None = None
        self._answer_what: tuple[str, SoftFrame | None, int] = ("", None, 0)
        self._answer_slot = 0.0
        self._pending_end = ""
        self._connect_tries = 0

    # ── commands ──────────────────────────────────────────────────────
    def connect(self, remote: str) -> None:
        if self._state is not State.IDLE:
            raise RuntimeError("already in a session")
        self.reset()
        self.remote_call = remote.upper()
        self.session = self.rng.randrange(1, 64)
        self._state = State.CONNECTING
        self.master = True
        self._trigger = "connect"
        self._send_connect()

    def send(self, data: bytes) -> None:
        self.send_queue(data)
        if not self.master and self._window is not None:
            self._trigger = "send"
            self._try_window()

    def disconnect(self) -> None:
        if self._state is State.IDLE:
            return
        self._disc_requested = True
        if not self.master:
            return  # the slave asks in its next answer; the master ends the session

    def abort(self) -> None:
        if self._state is State.IDLE:
            return
        self._end("aborted")

    # ── time ──────────────────────────────────────────────────────────
    def tick(self, now: float) -> None:
        self.now = max(self.now, now)
        for name in sorted(self._deadlines, key=self._deadlines.__getitem__):
            if self._deadlines.get(name, float("inf")) <= self.now + 1e-9:
                del self._deadlines[name]
                self._trigger = f"tick/timer:{name}"
                self._expire(name)

    def _expire(self, name: str) -> None:
        if name == "link":
            self._end("link timeout")
        elif name == "connect":
            if self._state is State.CONNECTING:
                if self._connect_tries >= self.cfg.connect_retries:
                    self._end("no answer")
                else:
                    self._send_connect()
        elif name == "proceed":
            self._proceed()
        elif name == "reclaim":
            slot = self._slot
            if slot is not None and not slot.get("announced") and not self._slot_heard:
                self._disarm("proceed")
                self._proceed()
        elif name == "answer":
            self._answer()
        elif name == "left":
            self._left = None

    def on_tx_done(self, now: float) -> None:
        self.now = max(self.now, now)
        self._trigger = "on_tx_done"
        kind, self._last_kind = self._last_kind, ""
        if self.master and self._state in (State.CONNECTED, State.DISCONNECTING) and self._slot:
            # the slot is measured from where this transmission really ended
            self._open_slot(now)
        elif kind == "connect_req":
            fs = self.timing.data_frame_s_for(self.robust_mode(True))
            wait = (
                self.gap_s(True)
                + GAP_STEPS[self._level]
                + self.cfg.slack_s
                + fs
                + self.heard_by(True)
                + self.rng.uniform(0.0, (1 + self._connect_tries) * fs)
            )
            self._arm("connect", now + wait)
        elif kind == "disc_ack" and self._state is not State.IDLE:
            self._end("peer disconnected", left=True)

    def on_tx_dropped(self, now: float) -> None:
        """The gate could not start a transmission in its slot: it was not sent."""
        self.now = max(self.now, now)
        self.stats.dropped += 1
        if self._last_kind == "disc_ack" and self._state is not State.IDLE:
            self._end("peer disconnected", left=True)
        self._last_kind = ""

    # ── frames in ─────────────────────────────────────────────────────
    def on_frame(self, frame: SoftFrame, now: float) -> None:
        self.now = max(self.now, now)
        self._trigger = "on_frame"
        payload, _ = frame.decode(None)
        if frame.container is Container.CONTROL:
            if payload is None:
                return  # a frame not read changes nothing
            try:
                ctl = Ctl.decode(payload)
            except ValueError:
                return
            self._on_control(ctl, frame)
            return
        if payload is not None:
            try:
                header, body = decode_data(payload)
            except ValueError:
                return
            if header.kind in (DataKind.CONNECT_REQ, DataKind.CONNECT_ACK):
                self._on_connect(header, body, frame)
                return
        if self._state not in (State.CONNECTED, State.DISCONNECTING):
            return
        if not frame.trusted and payload is None:
            return  # untrusted and unread: nothing at all
        if frame.floor != self.timing.is_floor(frame.mode):
            return
        if self.master:
            self._master_data(frame, payload is not None)
        else:
            self._slave_data(frame)

    def _heard(self, frame: SoftFrame) -> None:
        self._last_heard = self.now
        self.heard_db = frame.snr_db
        self._arm("link", self.now + self.link_timeout())

    # ── connection ────────────────────────────────────────────────────
    def _call_level(self) -> int:
        """The gap level a call grants its acceptance: up a level every second try, so a
        called station whose computer decodes late is reached (capability bits 3-4)."""
        return min(len(GAP_STEPS) - 1, max(0, self._connect_tries - 1) // 2)

    def _send_connect(self) -> None:
        floor = self.timing.floor_modes > 0 and self._connect_tries % 2 == 0
        mode = self.robust_mode(floor)
        level = self._call_level() if self._connect_tries else 0
        caps = self.cfg.capabilities | (level << 3)
        body = ConnectBody(self.my_call, self.remote_call, caps=caps).encode()
        self._level = level
        payload = encode_data(
            DataHeader(DataKind.CONNECT_REQ, 0, self.session), body, self.timing.capacity(mode)
        )
        self._connect_tries += 1
        self._last_kind = "connect_req"
        self.transmit([TxFrame(Container.DATA, payload, mode=mode)], "holder:connect_req")

    def _on_connect(self, header: DataHeader, body: bytes, frame: SoftFrame) -> None:
        try:
            cb = ConnectBody.decode(body)
        except ValueError:
            return
        if cb.dst != self.my_call or cb.version != TURNS_VERSION:
            return
        if bandwidth_code(cb.caps) != bandwidth_code(self.cfg.capabilities):
            return
        if header.kind is DataKind.CONNECT_REQ:
            fresh = self._state is State.IDLE or (
                not self.master and cb.src == self.remote_call and header.session != self.session
            )
            if fresh:
                self.reset()
                self._state = State.CONNECTED
                self.master = False
                self.session = header.session
                self.remote_call = cb.src
                self.rate.seed(frame.snr_db, lower_bound=frame.floor)
                self.event("connected", f"{self.remote_call} (slave)")
            elif self.master or header.session != self.session:
                return
            self._heard(frame)
            # the acceptance goes in the request's slot, or not at all
            level = (cb.caps >> 3) & 3
            slot = frame.t_end + self.gap_s(frame.floor) + GAP_STEPS[level]
            self._schedule_answer(slot, ("connect_ack", frame, self.session))
        elif (
            header.kind is DataKind.CONNECT_ACK
            and self._state is State.CONNECTING
            and header.session == self.session
        ):
            self._disarm("connect")
            self._state = State.CONNECTED
            self._heard(frame)
            self.rate.seed(frame.snr_db, lower_bound=frame.floor)
            if cb.snr_db is not None:
                self.send_rung = max(
                    self.cfg.initial_mode, min(self.cap(), self.rate.first_mode(cb.snr_db))
                )
                self.peer_report_db = cb.snr_db
                # the acceptance says how the slave heard the call: the first header goes in
                # the family that reading asks for, as every later one goes in the family the
                # slave's answers ask for (a floor reading is a lower bound: never too fast)
                if self.timing.floor_modes > 0:
                    self._hdr_floor = cb.snr_db < (
                        self.ctl_threshold() + self.cfg.answer_floor_margin_db
                    )
            self.event("connected", f"{self.remote_call} (master)")
            self._arm("proceed", frame.t_end + self.heard_by(frame.floor))

    # ── the master ────────────────────────────────────────────────────
    def _proceed(self) -> None:
        if not self.master or self._state not in (State.CONNECTED, State.DISCONNECTING):
            return
        if self._slot is not None:
            self._close_slot()
            if self._state is State.IDLE:
                return
        if self._state is State.DISCONNECTING:
            if self._disc_tries >= 3:
                self._end("closed (no DISC_ACK)", left=True)
                return
            self._send_disc()
            return
        if self._disc_requested and self._can_leave():
            self._state = State.DISCONNECTING
            self._send_disc()
            return
        self._send_hdr()

    def _can_leave(self) -> bool:
        if not self.has_work() and not self._slave_want:
            return True
        if self._patience_until is None:
            self._patience_until = self.now + max(20.0, 2 * self.link_timeout() / 4)
        return self.now >= self._patience_until

    def _grant_size(self) -> tuple[int, bool, int]:
        """(data frames granted, open window, window index)."""
        fs = self.frame_s(self.is_floor(self.rate.recommend()))
        fit = max(1, int((self.cfg.max_burst_s - self.ctl_s(True)) / fs + 1e-9))
        if self._slave_want is None:
            return min(self.gcfg.first_grant, fit), False, 0
        if self._slave_want > 0:
            return min(self._slave_want, self.cfg.max_frames, fit), False, 0
        if not self.has_work():
            return 0, True, min(self.gcfg.window_doublings, self._idle_windows)
        return 0, False, 0

    def _elide(self, n: int, open_window: bool, wanted: bool) -> bool:
        """Whether this transmission may go without its header: data only, nothing to grant
        the slave beyond its answer, the gap at level 0, the last slot heard, and the answer's
        family one the slave surely holds already."""
        return (
            self.gcfg.elide
            and not open_window
            and n == 0
            and self._slave_want == 0
            and self._silent == 0
            and max(self._asked_level, self._silence_level) == 0
            and self._possible == {wanted}
            and self.has_work()
        )

    def _send_hdr(self) -> None:
        n, open_window, w_index = self._grant_size()
        wanted = self.wants_floor() or self._silent >= 2
        elide = self._elide(n, open_window, wanted)
        self._ans_prev = self._ans_cmd
        self._ans_cmd = wanted
        if not elide:
            self._possible.add(wanted)
        hdr_floor = self._hdr_floor or self._silent >= 2
        room = self.cfg.max_burst_s - (0.0 if elide else self.ctl_s(hdr_floor))
        data = [] if open_window else self.build_data(room)
        self._level = 0 if elide else max(self._asked_level, self._silence_level)
        data_floor = bool(data) and self.is_floor(data[0].mode)
        ctl = Ctl(
            Kind.HDR,
            self.session,
            xid=self._level,
            count=len(data),
            data_floor=data_floor,
            fam=self._ans_cmd,
            flag=open_window,
            snr_db=self.heard_db,
            rung=self.recommend(),
            n=w_index if open_window else n,
            **self.ack_fields(),
        )
        frames = [TxFrame(Container.CONTROL, ctl.encode(), floor=hdr_floor), *data]
        # without a header, every data frame must state exactly how many follow (two bits:
        # four frames at most), or a receiver that loses the last ones cannot know the end
        if elide and data and len(data) <= 4:
            frames = data
            self.elided += 1
        elif elide:
            elide = False
            self._possible.add(wanted)
            self._level = max(self._asked_level, self._silence_level)
            ctl = replace(ctl, xid=self._level)
            frames = [TxFrame(Container.CONTROL, ctl.encode(), floor=hdr_floor), *data]
        slave_floor = self.is_floor(self.rate.recommend())
        # the answer may come in any family the slave may hold: reserve for the longest
        ans_floor = any(self._possible | {self._ans_cmd})
        ans_s = self.ctl_s(ans_floor)
        if open_window:
            base = self.gcfg.open_window_s * (2.5 if hdr_floor else 1.0)
            grant_s = base * (1 << w_index)
        else:
            grant_s = ans_s + n * self.frame_s(slave_floor)
        self._slot = {"grant_s": grant_s, "open": open_window, "data": bool(data), "n": n}
        self._stated_end = None
        self._slot_heard = False
        self._ans_t_end = None
        self.begin_rx()
        reason = (
            "holder:idle_window" if open_window else ("holder:hdr+data" if data else "holder:hdr")
        )
        if self._retries:
            reason = "holder:repeat"
        self._slot["last_floor"] = data_floor if data else hdr_floor
        self._slot["ans_floor"] = ans_floor
        end = self.transmit(frames, reason)
        self._last_kind = "hdr"
        # until on_tx_done says where it really ended
        self._open_slot(end, provisional=True)

    def _send_disc(self) -> None:
        self._disc_tries += 1
        self._ans_prev = self._ans_cmd
        self._ans_cmd = True
        ctl = Ctl(Kind.DISC, self.session, fam=True, snr_db=self.heard_db)
        floor = True
        self._slot = {
            "grant_s": self.ctl_s(True),
            "open": False,
            "data": False,
            "n": 0,
            "disc": True,
            "last_floor": floor,
            "ans_floor": True,
        }
        self._stated_end = None
        self._slot_heard = False
        end = self.transmit([TxFrame(Container.CONTROL, ctl.encode(), floor=floor)], "holder:disc")
        self._last_kind = "disc"
        self._open_slot(end, provisional=True)

    def _open_slot(self, end: float, provisional: bool = False) -> None:
        slot = self._slot
        if slot is None:
            return
        slot["tx_end"] = end
        slot["start"] = end + self.gap_s(slot["last_floor"]) + GAP_STEPS[self._level]
        slot["end_max"] = slot["start"] + self.cfg.slack_s + slot["grant_s"]
        if slot.get("open"):
            slot["end_max"] = slot["start"] + slot["grant_s"]
        # what ends the slot is the slave's answer or its data: wait until either is heard
        heard = max(self.heard_by(slot["ans_floor"]), self.heard_by(True))
        due = slot["end_max"] + heard
        if self._stated_end is not None:
            due = min(due, self._stated_end + self._stated_heard)
        self._arm("proceed", max(self.now, due))
        if self.gcfg.early_reclaim and not slot.get("open") and not self._slot_heard:
            announce = self.timing.preamble_detect_s_for(slot["ans_floor"]) or 0.0
            check = slot["start"] + self.cfg.slack_s + announce + self.heard_by(False)
            if check < due:
                self._arm("reclaim", check)

    def on_preamble(self, t_start: float, now: float, frame_s: float | None = None) -> None:
        """Announcements change nothing — except that, with :attr:`GsConfig.early_reclaim`,
        one in the slot means the slave started and the slot is not taken back."""
        self.now = max(self.now, now)
        slot = self._slot
        if self.master and slot is not None and "start" in slot and t_start >= slot["start"] - 0.3:
            slot["announced"] = True

    def _in_slot(self, frame: SoftFrame) -> bool:
        """A slave frame belongs to the slot after the master's last transmission: the slave
        keys only after a master transmission ends, whatever gap level it read."""
        slot = self._slot
        return slot is not None and "tx_end" in slot and frame.t_start >= slot["tx_end"]

    def _master_data(self, frame: SoftFrame, decoded: bool) -> None:
        if not self._in_slot(frame):
            # a frame of an earlier slot, decoded late: what it carries is kept (sequence
            # numbers make that safe), its timing is not; one that failed is dropped
            if decoded:
                payload, _ = frame.decode(None)
                if payload is not None:
                    from research.link_arch.engines.core import RxRec

                    self._accept(RxRec(frame, 0), payload)
            return
        if decoded:
            self._heard(frame)
            self._slot_heard = True
        first = self._ans_t_end
        ok = self.take_data(frame, first)
        if ok and frame.follows is not None and frame.follows < 3:
            self._state_end(
                frame.t_end + frame.follows * (frame.t_end - frame.t_start),
                self.is_floor(frame.mode),
            )

    def _state_end(self, end: float, floor: bool) -> None:
        if self._stated_end is None or end >= self._stated_end:
            self._stated_end = end
            self._stated_heard = self.heard_by(floor)
        due = self._stated_end + self._stated_heard
        self._arm("proceed", max(self.now, due))

    def _on_control(self, ctl: Ctl, frame: SoftFrame) -> None:
        if self._state is State.IDLE:
            # a DISC of the session this station left: answer it again, in its slot
            if ctl.kind is Kind.DISC and getattr(self, "_left", None) == ctl.session:
                self._schedule_answer(
                    frame.t_end + self.gap_s(frame.floor), ("disc_ack", frame, ctl.session)
                )
            return
        if ctl.session != (self.session & 0x3F):
            return
        if self.master:
            if not self._in_slot(frame):
                return
            self._heard(frame)
            self._slot_heard = True
            if ctl.kind is Kind.DISC_ACK:
                self._end("closed")
                return
            if ctl.kind is not Kind.ANS:
                return
            if self.take_ack(ctl):
                self._patience_until = None
            self._slave_want = ctl.n
            self._asked_level = ctl.xid
            self._hdr_floor = ctl.fam
            # the slave answered this slot in this family: it holds it now
            self._possible = {frame.floor}
            self._leave_asked = ctl.flag
            self._ans_t_end = frame.t_end
            last_floor = ctl.data_floor if ctl.count else frame.floor
            self._state_end(frame.t_end + ctl.count * self.frame_s(ctl.data_floor), last_floor)
            return
        # the slave
        self._heard(frame)
        if ctl.kind is Kind.DISC:
            self._schedule_answer(
                frame.t_end + self.gap_s(frame.floor), ("disc_ack", frame, ctl.session)
            )
            return
        if ctl.kind is not Kind.HDR:
            return
        self.take_ack(ctl)
        self._ans_sticky = ctl.fam
        self._grant = ctl
        self._grant_floor = frame.floor
        self._m_level = ctl.xid
        self._measure(frame)
        self._m_start = frame.t_start
        self._answered = None
        self.begin_rx()
        self._rx_first = frame.t_end
        self._m_end = frame.t_end + ctl.count * self.frame_s(ctl.data_floor)
        self._m_last_floor = ctl.data_floor if ctl.count else frame.floor
        self._schedule_slave()

    def _close_slot(self) -> None:
        """The slot is over: what the slave sent goes to the rate controller; silence is
        counted, and stepped down from."""
        slot = self._slot
        self._slot = None
        if slot is None:
            return
        self.observe_rx()
        if self._slot_heard:
            self._retries = 0
            self._silent = 0
            self._on_time += 1
            if self._on_time >= 4 and self._silence_level > 0:
                self._silence_level -= 1
                self._on_time = 0
            if slot.get("open") and self._stated_end is None:
                self._idle_windows += 1
            else:
                self._idle_windows = 0
            return
        if slot.get("open"):
            self._idle_windows += 1  # an open window nobody used is no silence
            self._retries = 0
            return
        self.stats.silent_slots += 1
        self._silent += 1
        self._retries += 1
        self._on_time = 0
        if self._silent >= 2 and self._silence_level < len(GAP_STEPS) - 1:
            self._silence_level += 1
        if slot.get("data"):
            self.back_off()
        if slot.get("n") and self._silent >= 2:
            self.rate.observe(None, 0, 1)
        if self._retries > self.cfg.max_retries:
            self._end("no response")

    # ── the slave ─────────────────────────────────────────────────────
    def _slave_data(self, frame: SoftFrame) -> None:
        if self._m_end is not None and frame.t_start > self._m_end + 0.3:
            # a new master transmission whose header was not read
            self._grant = None
            self._m_start = None
            self._m_end = None
            self._answered = None
            self.begin_rx()
        decoded = self.take_data(frame, self._rx_first)
        if decoded:
            self._heard(frame)
            self._measure(frame)
            if frame.follows is not None and frame.follows < 3:
                end = frame.t_end + frame.follows * (frame.t_end - frame.t_start)
                self._m_end = end if self._m_end is None else max(self._m_end, end)
                self._m_last_floor = self.is_floor(frame.mode)
                self._schedule_slave()

    def _schedule_slave(self) -> None:
        if self._m_end is None or self._answered == self._m_end:
            return
        grant = self._grant
        # A value the master can change is taken only from this transmission's own header;
        # without it, the one that keeps the answer inside every slot the master may have
        # granted: the earliest start (level 0), the shortest answer, no data. A level read
        # from an older header, after the master had lowered it, started the answer late and
        # ran it into the master's next transmission (the main campaign, 500 Hz, ITU Good).
        level = self._m_level if grant is not None else 0
        slot = self._m_end + self.gap_s(self._m_last_floor) + GAP_STEPS[level]
        if grant is not None and grant.flag:
            # an open window: start when there is something to say, end inside it
            base = self.gcfg.open_window_s * (2.5 if self._hdr_was_floor() else 1.0)
            self._window = (slot, slot + base * (1 << grant.n))
            self._answered = self._m_end
            if self.has_work() or self._disc_requested:
                self._arm("answer", max(self.now, slot - self.timing.tx_latency_s))
                self._pending_end = "window"
            return
        self._window = None
        self._schedule_answer(slot, ("ans", None, self.session))

    def _measure(self, frame: SoftFrame) -> None:
        """How late this station's receiver reported a master frame, against the budget its
        slot assumes: the gap level its next answer asks for. Easing down one level at a time
        when it is on time."""
        late = self.now - frame.t_end - self.cfg.decode_budget_s[int(frame.floor)]
        need = next((i for i, g in enumerate(GAP_STEPS) if late <= g + 1e-9), len(GAP_STEPS) - 1)
        if need > self._need_level:
            self._need_level = need
        elif need < self._need_level:
            self._ok_run = getattr(self, "_ok_run", 0) + 1
            if self._ok_run >= 6:
                self._need_level -= 1
                self._ok_run = 0

    def _hdr_was_floor(self) -> bool:
        return self._grant is not None and self._grant_floor

    def _schedule_answer(self, slot: float, what: tuple[str, SoftFrame | None, int]) -> None:
        lead = self.timing.tx_latency_s
        if self.now > slot - lead + self.cfg.slack_s:
            self.stats.forfeits += 1  # decoded too late for the slot: no answer at all
            return
        self._answer_what = what
        self._answer_slot = slot
        self._pending_end = ""
        if what[0] == "ans":
            self._answered = self._m_end
        self._arm("answer", max(self.now, slot - lead))

    def _answer(self) -> None:
        if self._pending_end == "window":
            self._pending_end = ""
            self._try_window()
            return
        what, frame, session = self._answer_what
        slot = self._answer_slot
        not_after = slot + self.cfg.slack_s
        if what == "connect_ack" and frame is not None:
            floor = frame.floor
            mode = self.robust_mode(floor)
            body = ConnectBody(
                self.my_call, self.remote_call, caps=self.cfg.capabilities, snr_db=frame.snr_db
            ).encode()
            payload = encode_data(
                DataHeader(DataKind.CONNECT_ACK, 0, self.session), body, self.timing.capacity(mode)
            )
            self.transmit(
                [TxFrame(Container.DATA, payload, mode=mode)],
                "answer:connect_ack",
                anchor=frame.t_end,
                not_after=not_after,
            )
            return
        if what == "disc_ack" and frame is not None:
            ctl = Ctl(Kind.DISC_ACK, session, snr_db=self.heard_db)
            self._last_kind = "disc_ack"
            self.transmit(
                [TxFrame(Container.CONTROL, ctl.encode(), floor=True)],
                "answer:disc_ack",
                anchor=frame.t_end,
                not_after=not_after,
            )
            return
        if what != "ans" or self._state is not State.CONNECTED:
            return
        self._send_ans(not_after, self._m_end)

    def _try_window(self) -> None:
        """Inside an open window: say something now if there is something, and only if it
        ends inside the window."""
        if self._window is None or self._state is not State.CONNECTED:
            return
        start, end = self._window
        on_air = self.now + self.timing.tx_latency_s
        if on_air < start:
            self._arm("answer", start - self.timing.tx_latency_s)
            self._pending_end = "window"
            return
        floor = self._ans_sticky
        room = end - on_air - self.ctl_s(floor)
        if room < 0 or not (self.has_work() or self._disc_requested):
            return
        self._window = None
        self._send_ans(None, None, room_s=room, window_end=end)

    def _send_ans(
        self,
        not_after: float | None,
        anchor: float | None,
        room_s: float | None = None,
        window_end: float | None = None,
    ) -> None:
        snr = self.observe_rx()
        if snr is not None:
            self.heard_db = snr
        grant = self._grant
        # the family the master asked for; without its header for this transmission, the
        # ordinary family, the shortest — it fits any slot the master may have granted
        floor = grant.fam if grant is not None else self._ans_sticky
        if room_s is None:
            n = grant.n if grant is not None and not grant.flag else 0
            fs = self.frame_s(self.is_floor(min(self.send_rung, self.cap())))
            room_s = n * fs
        data = self.build_data(room_s) if room_s > 0 else []
        ctl = Ctl(
            Kind.ANS,
            self.session,
            xid=self._need_level,
            count=len(data),
            data_floor=bool(data) and self.is_floor(data[0].mode),
            fam=self.wants_floor(),
            flag=self._disc_requested,
            snr_db=self.heard_db,
            rung=self.recommend(),
            n=min(7, self.queued_frames()),
            **self.ack_fields(),
        )
        frames = [TxFrame(Container.CONTROL, ctl.encode(), floor=floor), *data]
        if window_end is not None:
            dur = sum(self.timing.frame_s(f) for f in frames)
            not_after = window_end - dur
        self.begin_rx()
        reason = "answer:ans+data" if data else "answer:ans"
        if window_end is not None:
            reason = "answer:window"
        self.transmit(frames, reason, anchor=anchor, not_after=not_after)

    # ── the end ───────────────────────────────────────────────────────
    def _end(self, reason: str, left: bool = False) -> None:
        session = self.session & 0x3F
        self._state = State.IDLE
        self._deadlines.clear()
        self.reset()
        self.master = False
        self._left = session if left else None
        if left:
            self._arm("left", self.now + 60.0)
        self.event("disconnected", reason)
