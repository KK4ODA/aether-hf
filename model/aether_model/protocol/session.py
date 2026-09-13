"""
aether_model/protocol/session.py

AETHER HF session state machine.

Manages the lifecycle of a single point-to-point connection:
  IDLE → CONNECTING → CONNECTED → DISCONNECTING → IDLE

Handles rate adaptation, HARQ, and ARQ windowing.
"""

import logging
import secrets
import time
from collections.abc import Callable
from dataclasses import dataclass
from enum import Enum, auto

from aether_model.constants import (
    ACK_DOWN,
    ACK_OK,
    ACK_UP,
    GUARD_STANDARD_SYMS,
    MAX_HARQ_ROUNDS,
    NACK,
    NACK_IR,
    QRT,
    SEQ_NUM_MOD,
)
from aether_model.speed_levels import NARROW_LEVELS, WIDE_LEVELS, SpeedLevel

log = logging.getLogger(__name__)


class SessionState(Enum):
    IDLE = auto()
    CONNECTING = auto()
    CONNECTED = auto()
    DISCONNECTING = auto()


class SessionRole(Enum):
    ISS = "ISS"  # Information Sending Station
    IRS = "IRS"  # Information Receiving Station


@dataclass
class ChannelMetrics:
    """Channel quality indicators from the receiver."""

    snr_db: float = 0.0
    dispersion_index: float = 0.0
    ldpc_iterations: int = 0
    ldpc_max_iter: int = 40
    fer_ewma: float = 0.0
    consecutive_fails: int = 0


@dataclass
class SessionConfig:
    """Negotiated session parameters."""

    mode: str = "wide"  # wide, narrow, emergency
    protocol_version: int = 1
    speed_level: int = 5  # initial level
    guard_symbols: int = GUARD_STANDARD_SYMS
    profile: str = "throughput"  # throughput or latency


@dataclass
class PendingFrame:
    """A frame awaiting acknowledgment in the ARQ window."""

    seq_num: int
    data: bytes
    harq_round: int = 0
    sent_at: float = 0.0


class AetherSession:
    """Manages one AETHER HF radio session."""

    def __init__(
        self,
        my_call: str,
        mode: str = "wide",
        on_state_change: Callable | None = None,
        on_data_received: Callable | None = None,
    ):
        self.my_call = my_call.upper()
        self.remote_call = ""
        self.state = SessionState.IDLE
        self.role = SessionRole.ISS

        self._mode = mode
        self._levels = WIDE_LEVELS if mode == "wide" else NARROW_LEVELS
        self._config = SessionConfig(mode=mode)
        self._metrics = ChannelMetrics()

        self._current_level_idx = 4  # Start at level 5 (BPSK 1/2)
        self._nonce = ""
        self._session_key = b""

        # ARQ window
        self._tx_seq = 0
        self._pending: dict[int, PendingFrame] = {}

        # Callbacks
        self._on_state_change = on_state_change
        self._on_data_received = on_data_received

    @property
    def current_level(self) -> SpeedLevel:
        return self._levels[self._current_level_idx]

    @property
    def metrics(self) -> ChannelMetrics:
        return self._metrics

    # ── Connection lifecycle ──────────────────────────────────────────

    def initiate_connect(self, remote_call: str) -> dict:
        """Prepare a CONNECT frame for transmission.

        Returns dict with frame fields for the transmitter to encode.
        """
        self.remote_call = remote_call.upper()
        self._nonce = secrets.token_hex(8)
        self._set_state(SessionState.CONNECTING)

        return {
            "type": "CONNECT",
            "my_call": self.my_call,
            "remote_call": self.remote_call,
            "mode": self._mode,
            "version_min": 1,
            "version_max": 1,
            "nonce": self._nonce,
            "guard": self._config.guard_symbols,
        }

    def handle_connect(self, frame: dict) -> dict:
        """Process an incoming CONNECT frame (IRS side).

        Returns a CONNECT-ACK frame dict.
        """
        self.remote_call = frame.get("my_call", "").upper()
        self._nonce = secrets.token_hex(8)

        # Negotiate parameters
        self._config.mode = frame.get("mode", "wide")
        self._config.guard_symbols = max(
            self._config.guard_symbols,
            frame.get("guard", GUARD_STANDARD_SYMS),
        )

        # Set initial speed level based on preamble SNR
        initial_level = self._select_initial_level(self._metrics.snr_db)
        self._current_level_idx = initial_level - 1

        self._set_state(SessionState.CONNECTED)
        self.role = SessionRole.IRS

        return {
            "type": "CONNECT-ACK",
            "my_call": self.my_call,
            "remote_call": self.remote_call,
            "mode": self._config.mode,
            "version": 1,
            "level": self.current_level.level,
            "nonce": self._nonce,
            "guard": self._config.guard_symbols,
        }

    def handle_connect_ack(self, frame: dict):
        """Process CONNECT-ACK (ISS side). Session is now established."""
        level = frame.get("level", 5)
        self._current_level_idx = level - 1
        self._config.guard_symbols = max(
            self._config.guard_symbols,
            frame.get("guard", GUARD_STANDARD_SYMS),
        )
        self._set_state(SessionState.CONNECTED)
        self.role = SessionRole.ISS
        log.info(f"Session established with {self.remote_call} at Level {level}")

    def disconnect(self):
        """Initiate session termination."""
        self._set_state(SessionState.DISCONNECTING)
        self._pending.clear()

    def handle_disconnect(self):
        """Process remote disconnect."""
        self._set_state(SessionState.IDLE)
        self._pending.clear()
        self.remote_call = ""

    # ── Data transfer ─────────────────────────────────────────────────

    def prepare_data_frame(self, data: bytes) -> dict:
        """Prepare a DATA frame for transmission.

        Returns frame dict with seq_num, data, and current speed level.
        """
        seq = self._tx_seq
        self._tx_seq = (self._tx_seq + 1) % SEQ_NUM_MOD

        pf = PendingFrame(seq_num=seq, data=data, sent_at=time.monotonic())
        self._pending[seq] = pf

        return {
            "type": "DATA",
            "seq": seq,
            "level": self.current_level.level,
            "harq": 0,
            "data": data,
        }

    def handle_ack(self, ack_type: int, seq_num: int, snr: float, metrics: dict):
        """Process an ACK frame from the receiver."""
        self._metrics.snr_db = snr
        self._metrics.ldpc_iterations = metrics.get("ldpc_iter", 0)
        self._metrics.fer_ewma = metrics.get("fer", 0.0)

        if ack_type == ACK_OK:
            self._pending.pop(seq_num, None)
            self._metrics.consecutive_fails = 0

        elif ack_type == ACK_UP:
            self._pending.pop(seq_num, None)
            self._metrics.consecutive_fails = 0
            self._step_up()

        elif ack_type == ACK_DOWN:
            self._pending.pop(seq_num, None)
            self._metrics.consecutive_fails = 0
            self._step_down()

        elif ack_type == NACK:
            self._metrics.consecutive_fails += 1
            self._step_down()

        elif ack_type == NACK_IR:
            self._metrics.consecutive_fails += 1
            # Prepare HARQ increment
            pf = self._pending.get(seq_num)
            if pf and pf.harq_round < MAX_HARQ_ROUNDS:
                pf.harq_round += 1

        elif ack_type == QRT:
            self.handle_disconnect()

    # ── Rate adaptation ───────────────────────────────────────────────

    def _step_up(self):
        """Increase speed level by 1 (if possible)."""
        if self._current_level_idx < len(self._levels) - 1:
            self._current_level_idx += 1
            log.info(f"Rate UP → Level {self.current_level.level}")

    def _step_down(self):
        """Decrease speed level by 2 (if possible)."""
        self._current_level_idx = max(0, self._current_level_idx - 2)
        log.info(f"Rate DOWN → Level {self.current_level.level}")

    def _select_initial_level(self, snr_db: float) -> int:
        """Select initial speed level based on measured preamble SNR."""
        for sl in reversed(self._levels):
            if snr_db >= sl.min_snr_db + 3.0:  # 3 dB margin
                return sl.level
        return 1

    # ── Internal ──────────────────────────────────────────────────────

    def _set_state(self, new_state: SessionState):
        old = self.state
        self.state = new_state
        log.info(f"Session {old.name} → {new_state.name}")
        if self._on_state_change:
            self._on_state_change(old, new_state)
