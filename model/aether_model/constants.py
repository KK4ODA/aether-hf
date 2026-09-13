"""LEGACY parameters of the pre-audit prototype.

These numbers are consumed only by the legacy DSP/protocol/host modules that are scheduled
for rewrite in Phase 1 (``docs/ROADMAP.md`` P1-3 … P1-6). They are **not** the Aether HF v1
waveform: that is defined in :mod:`aether_model.waveform` (ADR-0002) and will replace this
file as each legacy module is retired. Do not add new parameters here.

Known inconsistency, kept verbatim so the audit's ``xfail`` tests keep their meaning
(``docs/AUDIT.md`` §2): 64 carriers × 46.875 Hz = 3 000 Hz, not the 2 300 Hz "wide mode".
"""

# ── Audio / baseband ─────────────────────────────────────────────────
SAMPLE_RATE = 48_000  # Hz, sound-card rate
BASEBAND_RATE = 12_000  # Hz, complex baseband of the legacy OFDM (v1 waveform uses 8 kHz)
RESAMPLE_FACTOR = SAMPLE_RATE // BASEBAND_RATE  # 4
AUDIO_BUFFER_SIZE = 2048  # samples per sound-card callback

# ── Legacy OFDM (FFT 256 @ 12 kHz → 46.875 Hz spacing) ──────────────
FFT_SIZE_W = 256
DATA_CARRIERS_W = 52
PILOT_CARRIERS_W = 12
TOTAL_CARRIERS_W = DATA_CARRIERS_W + PILOT_CARRIERS_W  # 64 → 3 000 Hz occupied
DATA_CARRIERS_N = 8
PILOT_CARRIERS_N = 2
TOTAL_CARRIERS_N = DATA_CARRIERS_N + PILOT_CARRIERS_N  # 10
CP_DEFAULT_SAMPLES = int(3.2e-3 * BASEBAND_RATE)  # 38
CP_EXTENDED_SAMPLES = int(6.4e-3 * BASEBAND_RATE)  # 76
PILOT_FREQ_SPACING = 4
RAISED_COSINE_BETA = 0.08

# ── Legacy preamble ──────────────────────────────────────────────────
ZC_LENGTH = 128
ZC_ROOT = 29
CFO_SWEEP_RANGE_HZ = 250.0
CFO_SWEEP_STEP_HZ = 25.0

# ── Legacy ARQ / session ─────────────────────────────────────────────
ARQ_WINDOW_SIZE = 8
SEQ_NUM_BITS = 4
SEQ_NUM_MOD = 2**SEQ_NUM_BITS
MAX_HARQ_ROUNDS = 3

ACK_OK = 0x0
ACK_UP = 0x1
ACK_DOWN = 0x2
NACK = 0x3
NACK_IR = 0x4
BREAK = 0x5
PROBE_REQ = 0x6
QRT = 0x7
WAIT = 0x8
CONNECT_ACK = 0x9

GUARD_FAST_SYMS = 1
GUARD_STANDARD_SYMS = 2
GUARD_SLOW_SYMS = 3

CONNECT_TIMEOUT_S = 30
DISCONNECT_TIMEOUT_S = 30
PROBE_PAUSE_S = 2.0
CONSECUTIVE_FAIL_PROBE = 2
CONSECUTIVE_FAIL_WAIT = 5
CONSECUTIVE_FAIL_DISC = 10

# ── Host interface (VARA-compatible TCP; VARA HF's own defaults) ─────
DEFAULT_CMD_PORT = 8300
DEFAULT_DATA_PORT = 8301
