"""Candidate C, "fixed cycle": a scheduled ARQ superframe, PACTOR- and 3G-HDL-like.

The protocol in one paragraph
-----------------------------
After the connection the master (the caller) runs a cycle of fixed length ``T`` for the
current *gear* — the family of the cycle's data frames and of its control frames. Cycle ``k``
starts at ``t0 + k·T`` with the master's header (HDR) and its data frames; it ends with the
slave's transmission, whose last frame — its answer (ANS) — always ends at the same point,
``T`` less a gap and the slack before the next cycle. The cycle's ``D`` data frames are
shared: the master grants the slave ``n`` of them in the header, and the slave's data frames
go just before its answer. The master keys at every cycle start, whatever it heard; the
slave answers in every cycle at its fixed point — *blind*, with "nothing heard", when it
decoded nothing of the cycle, as long as its lock on the cycle is fresh and no change of
gear is pending (a 3G-HDL / PACTOR flywheel). A change of gear (the floor to OFDM, ordinary
control frames to the floor's) is announced in a header, confirmed in the answer, and made
at the next cycle only if the master read the confirmation; a slave that confirmed answers
nothing until it has read a header of the cycle in use. There is no early end to a cycle:
a short transmission leaves the rest of its part of the cycle silent.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from aether_model.frame.modes import AirInterface
from aether_model.link.frames import (
    PROTOCOL_VERSION,
    ConnectBody,
    DataHeader,
    DataKind,
    bandwidth_code,
    decode_data,
    encode_data,
)
from aether_model.link.harness import bandwidth_capabilities
from aether_model.link.phy import Container, PhyTiming, SoftFrame, TxFrame
from research.link_arch.engines.core import Core, CoreConfig, Ctl, Kind, RxRec, State

GEARS = ((False, False), (False, True), (True, True))
"""(data on the floor, control frames on the floor), by gear number."""


@dataclass
class FcConfig(CoreConfig):
    data_frames: tuple[int, int] = (6, 4)
    """A cycle's data frames, ordinary and floor: what the master and the slave share."""
    lock_cycles: int = 4
    """A slave answers blind only within this many cycles of the last header it read."""

    @classmethod
    def for_air(
        cls, air: AirInterface, arm: str = "fc", knobs: tuple[float, ...] = (0.0, 0.3)
    ) -> FcConfig:
        cfg = cls(max_mode=air.n_rungs - 1, capabilities=bandwidth_capabilities(air.params))
        cfg.turnaround_s, cfg.slack_s = knobs[0], knobs[1]
        if len(knobs) > 2:
            cfg.decode_budget_s = (knobs[2], knobs[3])
        if arm == "fc4":
            cfg.data_frames = (4, 2)
        return cfg


def belief(engine: Any) -> dict[str, Any]:
    up = engine.state.value in ("connected", "disconnecting")
    return {
        "state": engine.state.value,
        "role": "master" if engine.master else "slave",
        "holder": up and engine.master,
    }


class FcEngine(Core):
    def __init__(self, my_call: str, timing: PhyTiming, cfg: FcConfig, seed: int = 0) -> None:
        self.fcfg = cfg
        super().__init__(my_call, timing, cfg, seed)

    def reset(self) -> None:
        super().reset()
        self.gear = 2
        self._t0: float | None = None
        """Start of cycle 0 of the current gear (the master's HDR, on the air)."""
        self._k = 0
        self._pending: int | None = None
        """Master: the gear announced and not yet confirmed. Slave: the gear confirmed."""
        self._confirmed = False
        self._slave_want: int | None = None
        self._ctl_req_floor = True
        self._heard_cycle = False
        self._silent = 0
        self._sent_data = False
        self._lock_k: int | None = None
        """Slave: the cycle of the last header read."""
        self._cycle_grant: Ctl | None = None
        self._cycle_sent = -1
        self._connect_tries = 0
        self._last_kind = ""
        self._ack_slot = 0.0
        self._ack_frame: SoftFrame | None = None
        self._ack_kind = ""
        self._ack_session = 0
        self._left: int | None = None

    # ── the cycle ─────────────────────────────────────────────────────
    def gear_floor(self, gear: int | None = None) -> tuple[bool, bool]:
        return GEARS[self.gear if gear is None else gear]

    def cycle_s(self, gear: int | None = None) -> float:
        data_floor, ctl_floor = self.gear_floor(gear)
        d = self.fcfg.data_frames[int(data_floor)]
        gap = self.gap_s(data_floor or ctl_floor)
        return (
            2 * self.ctl_s(ctl_floor)
            + d * self.frame_s(data_floor)
            + 2 * gap
            + 2 * self.cfg.slack_s
        )

    def cycle_start(self, k: int) -> float:
        assert self._t0 is not None
        return self._t0 + k * self.cycle_s()

    def ans_end(self, k: int) -> float:
        """Where the slave's transmission ends in cycle ``k``: the same point every cycle."""
        _, ctl_floor = self.gear_floor()
        return self.cycle_start(k + 1) - self.gap_s(ctl_floor) - self.cfg.slack_s

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

    def disconnect(self) -> None:
        if self._state is not State.IDLE:
            self._disc_requested = True

    def abort(self) -> None:
        if self._state is not State.IDLE:
            self._end("aborted")

    def on_preamble(self, t_start: float, now: float, frame_s: float | None = None) -> None:
        self.now = max(self.now, now)

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
        elif name == "cycle":
            self._master_cycle()
        elif name == "slave":
            self._slave_turn()
        elif name == "ack":
            self._send_solicited()
        elif name == "left":
            self._left = None

    def on_tx_done(self, now: float) -> None:
        self.now = max(self.now, now)
        kind, self._last_kind = self._last_kind, ""
        if kind == "connect_req":
            fs = self.timing.data_frame_s_for(self.robust_mode(True))
            wait = (
                self.gap_s(True)
                + self.cfg.slack_s
                + fs
                + self.heard_by(True)
                + self.rng.uniform(0.0, (1 + self._connect_tries) * fs)
            )
            self._arm("connect", now + wait)
        elif kind == "disc_ack" and self._state is not State.IDLE:
            self._end("peer disconnected", left=True)

    def on_tx_dropped(self, now: float) -> None:
        self.now = max(self.now, now)
        self.stats.dropped += 1
        if self._last_kind == "disc_ack" and self._state is not State.IDLE:
            self._end("peer disconnected", left=True)
        self._last_kind = ""

    # ── connection (as granted slots) ─────────────────────────────────
    def _send_connect(self) -> None:
        floor = self.timing.floor_modes > 0 and self._connect_tries % 2 == 0
        mode = self.robust_mode(floor)
        body = ConnectBody(self.my_call, self.remote_call, caps=self.cfg.capabilities).encode()
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
        if cb.dst != self.my_call or cb.version != PROTOCOL_VERSION:
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
            self._solicit(frame.t_end + self.gap_s(frame.floor), "connect_ack", frame, self.session)
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
            self.event("connected", f"{self.remote_call} (master)")
            # the first gear: control frames on the floor until the slave says otherwise;
            # data in the family the call measured
            self.gear = 2 if self.is_floor(self.send_rung) else 1
            self._t0 = max(frame.t_end + self.heard_by(frame.floor), self.now) + (
                self.timing.tx_latency_s
            )
            self._k = 0
            self._arm("cycle", self._t0 - self.timing.tx_latency_s)

    def _solicit(self, slot: float, kind: str, frame: SoftFrame, session: int) -> None:
        lead = self.timing.tx_latency_s
        if self.now > slot - lead + self.cfg.slack_s:
            self.stats.forfeits += 1
            return
        self._ack_slot, self._ack_kind, self._ack_frame = slot, kind, frame
        self._ack_session = session
        self._arm("ack", max(self.now, slot - lead))

    def _send_solicited(self) -> None:
        frame, kind, not_after = self._ack_frame, self._ack_kind, self._ack_slot + self.cfg.slack_s
        if frame is None:
            return
        if kind == "connect_ack":
            mode = self.robust_mode(frame.floor)
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
        elif kind == "disc_ack":
            ctl = Ctl(Kind.DISC_ACK, self._ack_session, snr_db=self.heard_db)
            self._last_kind = "disc_ack"
            self.transmit(
                [TxFrame(Container.CONTROL, ctl.encode(), floor=True)],
                "answer:disc_ack",
                anchor=frame.t_end,
                not_after=not_after,
            )

    def _heard(self, frame: SoftFrame) -> None:
        self._last_heard = self.now
        self.heard_db = frame.snr_db
        self._arm("link", self.now + self.link_timeout())

    # ── frames in ─────────────────────────────────────────────────────
    def on_frame(self, frame: SoftFrame, now: float) -> None:
        self.now = max(self.now, now)
        self._trigger = "on_frame"
        payload, _ = frame.decode(None)
        if frame.container is Container.CONTROL:
            if payload is None:
                return
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
        if (not frame.trusted and payload is None) or frame.floor != self.is_floor(frame.mode):
            return
        if payload is not None:
            self._heard(frame)
        if self._t0 is None:
            if payload is not None:
                self._accept(RxRec(frame, 0), payload)
            return
        # which cycle this frame was in, by its air time: an old cycle's frame is content only
        k = int((frame.t_start - self._t0) // self.cycle_s())
        if k != self._k:
            if payload is not None:
                self._accept(RxRec(frame, 0), payload)
            return
        if self.master:
            self._heard_cycle = self._heard_cycle or payload is not None
        self.take_data(frame, self._rx_first)

    def _on_control(self, ctl: Ctl, frame: SoftFrame) -> None:
        if self._state is State.IDLE:
            if ctl.kind is Kind.DISC and self._left == ctl.session:
                self._solicit(frame.t_end + self.gap_s(frame.floor), "disc_ack", frame, ctl.session)
            return
        if ctl.session != (self.session & 0x3F):
            return
        self._heard(frame)
        if self.master:
            if ctl.kind is Kind.DISC_ACK:
                self._end("closed")
                return
            if ctl.kind is not Kind.ANS or self._t0 is None:
                return
            k = int((frame.t_start - self._t0) // self.cycle_s())
            if k != self._k:
                self.take_ack(ctl)  # an old cycle's answer: its acknowledgement only
                return
            self._heard_cycle = True
            self.take_ack(ctl)
            self._slave_want = ctl.n
            self._ctl_req_floor = ctl.fam
            if self._pending is not None and ctl.flag and ctl.xid == self._pending:
                self._confirmed = True
            return
        # the slave
        if ctl.kind is Kind.DISC:
            self._disarm("slave")
            self._solicit(frame.t_end + self.gap_s(frame.floor), "disc_ack", frame, ctl.session)
            return
        if ctl.kind is not Kind.HDR:
            return
        gear = (
            GEARS.index((ctl.data_floor, frame.floor))
            if ((ctl.data_floor, frame.floor) in GEARS)
            else self.gear
        )
        if self._t0 is None or gear != self.gear or self._pending is not None:
            # (re)lock: this header starts a cycle of the gear it is in
            self.gear = gear
            self._t0 = frame.t_start
            self._k = 0
            self._pending = None
            self._cycle_sent = -1  # a new count of cycles: nothing of it answered yet
        else:
            k = round((frame.t_start - self._t0) / self.cycle_s())
            drift = frame.t_start - (self._t0 + k * self.cycle_s())
            if abs(drift) <= 0.1:
                self._t0 += drift  # follow the master's clock, never a late header
            self._k = k
        self._lock_k = self._k
        self.take_ack(ctl)
        self._cycle_grant = ctl
        self.begin_rx()
        self._rx_first = frame.t_end
        if ctl.flag:
            self._pending = ctl.xid  # confirmed in this cycle's answer
        self._schedule_slave(self._k)

    # ── the master's cycle ────────────────────────────────────────────
    def _master_cycle(self) -> None:
        if not self.master or self._state not in (State.CONNECTED, State.DISCONNECTING):
            return
        if self._t0 is None:
            return
        # the cycle that ended: what was heard in it
        if self._cycle_sent >= 0:
            self._close_cycle()
            if self._state is State.IDLE:
                return
        # a confirmed change of gear starts with this cycle
        if self._pending is not None and self._confirmed:
            start = self.cycle_start(self._k + 1)
            self.gear = self._pending
            self._t0 = start
            self._k = 0
            self._pending = None
            self._confirmed = False
        elif self._cycle_sent >= 0:
            self._k += 1
        self._cycle_sent = self._k
        self._heard_cycle = False
        self.begin_rx()
        if self._state is State.DISCONNECTING or (
            self._disc_requested and not self.has_work() and not self._slave_want
        ):
            if self._disc_tries >= 3:
                self._end("closed (no DISC_ACK)", left=True)
                return
            self._state = State.DISCONNECTING
            self._disc_tries += 1
            ctl = Ctl(Kind.DISC, self.session, fam=True, snr_db=self.heard_db)
            self.transmit(
                [TxFrame(Container.CONTROL, ctl.encode(), floor=True)],
                "holder:disc",
                not_after=self.cycle_start(self._k) + self.cfg.slack_s,
            )
            self._arm("cycle", self.cycle_start(self._k + 1) - self.timing.tx_latency_s)
            return
        data_floor, ctl_floor = self.gear_floor()
        # the gear the link wants: data on the floor when this station's rung is there, control
        # frames on the floor when either station asks for them
        want = GEARS.index(
            (
                self.is_floor(min(self.send_rung, self.cap())),
                self._ctl_req_floor
                or self.wants_floor()
                or self.is_floor(min(self.send_rung, self.cap())),
            )
        )
        if want != self.gear and self._pending is None:
            self._pending = want
            self._confirmed = False
        d = self.fcfg.data_frames[int(data_floor)]
        want_s = d if self._slave_want is None else min(self._slave_want, d)
        if self._slave_want is None:
            want_s = min(2, d)
        # the slave's share first when it has asked, the rest for this station
        mine_max = d - want_s if self.has_work() else 0
        if self.has_work() and mine_max == 0:
            mine_max = max(1, d // 2)
            want_s = d - mine_max
        rung = min(self.send_rung, self.cap())
        if self.is_floor(rung) != data_floor:
            # the gear has not followed yet: the nearest rung of the gear's family
            rung = self._nearest(rung, data_floor)
        saved = self.send_rung
        self.send_rung = rung
        data = self.build_data(mine_max * self.frame_s(data_floor))
        self.send_rung = saved
        self._sent_data = bool(data)
        hdr = Ctl(
            Kind.HDR,
            self.session,
            xid=self._pending if self._pending is not None else 0,
            count=len(data),
            data_floor=data_floor,
            fam=ctl_floor,
            flag=self._pending is not None,
            snr_db=self.heard_db,
            rung=self.recommend(),
            n=want_s,
            **self.ack_fields(),
        )
        frames = [TxFrame(Container.CONTROL, hdr.encode(), floor=ctl_floor), *data]
        # the cycle starts on time or not at all: a header held past the slack would move
        # the slave's clock, or run into its answer
        self.transmit(
            frames,
            "holder:cycle" if data else "holder:cycle_hdr",
            not_after=self.cycle_start(self._k) + self.cfg.slack_s,
        )
        self._arm("cycle", self.cycle_start(self._k + 1) - self.timing.tx_latency_s)

    def _nearest(self, rung: int, floor: bool) -> int:
        modes = [m for m in self.rate.modes if self.is_floor(m) == floor and m <= self.cap()]
        if not modes:
            return rung
        return min(modes) if not floor else max(modes)

    def _close_cycle(self) -> None:
        self.observe_rx()
        if self._heard_cycle:
            self._silent = 0
            self._retries = 0
            return
        self.stats.silent_slots += 1
        self._silent += 1
        self._retries += 1
        if self._sent_data:
            self.back_off()
        if self._silent >= 2:
            self.rate.observe(None, 0, 1)
        if self._retries > self.cfg.max_retries:
            self._end("no response")

    # ── the slave's part of each cycle ────────────────────────────────
    def _schedule_slave(self, k: int) -> None:
        """Arm the slave's transmission for cycle ``k``: as early as its granted data need,
        ending at the cycle's fixed answer point."""
        if self._cycle_sent == k:
            return
        data_floor, ctl_floor = self.gear_floor()
        grant = self._cycle_grant
        n = 0
        if grant is not None and self._lock_k == k:
            n = min(grant.n, self.queued_frames())
        start = self.ans_end(k) - self.ctl_s(ctl_floor) - n * self.frame_s(data_floor)
        lead = self.timing.tx_latency_s
        if self.now > start - lead:
            # too late for the data: the answer alone, at its fixed point
            n = 0
            start = self.ans_end(k) - self.ctl_s(ctl_floor)
        self._slave_plan = (k, n, start)
        self._arm("slave", max(self.now, start - lead))

    def _slave_turn(self) -> None:
        if self.master or self._state is not State.CONNECTED or self._t0 is None:
            return
        k, n, start = getattr(self, "_slave_plan", (self._k, 0, 0.0))
        data_floor, ctl_floor = self.gear_floor()
        blind = self._lock_k != k
        stale = self._lock_k is None or k - self._lock_k > self.fcfg.lock_cycles
        if blind and (stale or self._pending is not None):
            self._next_slave(k)  # no fresh lock, or a change of gear unresolved: silence
            return
        self._cycle_sent = k
        snr = self.observe_rx()
        if snr is not None:
            self.heard_db = snr
        if blind:
            # nothing of the cycle was read, not even its header: the path is worse than the
            # recommendation assumed
            self.rate.observe(None, 0, 1)
        saved = self.send_rung
        if self.is_floor(min(self.send_rung, self.cap())) != data_floor:
            self.send_rung = self._nearest(self.send_rung, data_floor)
        data = self.build_data(n * self.frame_s(data_floor)) if n else []
        self.send_rung = saved
        confirm = self._pending is not None and not blind
        ans = Ctl(
            Kind.ANS,
            self.session,
            xid=self._pending if confirm else 0,
            count=0,
            data_floor=data_floor,
            fam=self.wants_floor(),
            flag=confirm,
            snr_db=self.heard_db if not blind else None,
            rung=self.recommend(),
            n=min(7, self.queued_frames()),
            **self.ack_fields(),
        )
        frames = [*data, TxFrame(Container.CONTROL, ans.encode(), floor=ctl_floor)]
        reason = (
            "answer:cycle_blind" if blind else ("answer:cycle+data" if data else "answer:cycle")
        )
        self.transmit(
            frames, reason, anchor=self.cycle_start(k), not_after=start + self.cfg.slack_s
        )
        self.begin_rx()
        self._next_slave(k)

    def _next_slave(self, k: int) -> None:
        """Expect the next cycle: answer blind at its fixed point unless a header comes."""
        self._k = k + 1
        self._cycle_grant = None
        self._slave_plan = (self._k, 0, self.ans_end(self._k) - self.ctl_s(self.gear_floor()[1]))
        self._arm("slave", self._slave_plan[2] - self.timing.tx_latency_s)

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
