"""
aether_hf/constants.py

Core protocol constants derived from the AETHER HF v2.0 specification.
"""

# ── Audio ─────────────────────────────────────────────────────────────
SAMPLE_RATE       = 48_000   # Hz
BIT_DEPTH         = 16
AUDIO_BUFFER_SIZE = 2048     # samples per buffer (42.7 ms)

# ── OFDM Parameters (Wide Mode, 2300 Hz) ─────────────────────────────
FFT_SIZE           = 256
SUBCARRIER_SPACING = SAMPLE_RATE / FFT_SIZE   # 187.5 Hz... wait

# Actually at 48kHz with FFT 256: spacing = 48000/256 = 187.5 Hz
# That's wrong — the spec says 46.875 Hz spacing.
# 46.875 Hz = 48000 / 1024.  So FFT_SIZE should be 1024?
# No — the spec says FFT_SIZE=256, spacing=46.875 Hz.
# 46.875 * 256 = 12000 Hz.  So the baseband sample rate is 12000 Hz,
# not 48000 Hz.  We downsample from 48kHz to 12kHz before OFDM.
#
# Correction: The modem operates at a baseband sample rate of 12 kHz.
# Audio I/O is at 48 kHz; we resample 4:1.

BASEBAND_RATE      = 12_000  # Hz (48000 / 4)
RESAMPLE_FACTOR    = SAMPLE_RATE // BASEBAND_RATE  # 4

# With 12 kHz baseband and FFT 256:
# subcarrier spacing = 12000 / 256 = 46.875 Hz  ✓
FFT_SIZE_W         = 256
SUBCARRIER_SPACING_W = BASEBAND_RATE / FFT_SIZE_W  # 46.875 Hz

# Wide mode carriers
DATA_CARRIERS_W    = 52
PILOT_CARRIERS_W   = 12
TOTAL_CARRIERS_W   = DATA_CARRIERS_W + PILOT_CARRIERS_W  # 64

# Narrow mode carriers
DATA_CARRIERS_N    = 8
PILOT_CARRIERS_N   = 2
TOTAL_CARRIERS_N   = DATA_CARRIERS_N + PILOT_CARRIERS_N  # 10

# Cyclic prefix
CP_DEFAULT_SAMPLES = int(3.2e-3 * BASEBAND_RATE)   # 38 samples (3.17 ms)
CP_EXTENDED_SAMPLES = int(6.4e-3 * BASEBAND_RATE)  # 76 samples (6.33 ms)

# Symbol timing
FFT_DURATION_MS    = 1000.0 * FFT_SIZE_W / BASEBAND_RATE  # 21.33 ms
CP_DURATION_MS     = 1000.0 * CP_DEFAULT_SAMPLES / BASEBAND_RATE  # ~3.17 ms
SYMBOL_DURATION_MS = FFT_DURATION_MS + CP_DURATION_MS  # ~24.5 ms
SYMBOL_RATE        = 1000.0 / SYMBOL_DURATION_MS  # ~40.8 baud

# PAPR target
PAPR_TARGET_DATA_DB = 8.5
PAPR_TARGET_ACK_DB  = 5.0

# Windowing
RAISED_COSINE_BETA = 0.08

# ── Preamble ──────────────────────────────────────────────────────────
ZC_LENGTH          = 128     # Zadoff-Chu sequence length
ZC_ROOT            = 29      # ZC root index (prime, good correlation)
PREAMBLE_SYMBOLS   = 4       # Part A (2) + Part B (1) + Part C (1)
ACK_PREAMBLE_SYMS  = 2       # Shortened preamble for ACK frames

# Frequency acquisition
CFO_SWEEP_RANGE_HZ = 250.0   # ±250 Hz sweep
CFO_SWEEP_STEP_HZ  = 25.0    # 25 Hz steps
CFO_NUM_HYPOTHESES = int(2 * CFO_SWEEP_RANGE_HZ / CFO_SWEEP_STEP_HZ) + 1  # 21

# ── Pilot Grid ────────────────────────────────────────────────────────
PILOT_FREQ_SPACING = 4       # every 4th subcarrier
PILOT_TIME_SPACING = 4       # every 4th symbol
PILOT_DENSITY      = 1.0 / (PILOT_FREQ_SPACING * PILOT_TIME_SPACING)  # 6.25%

# ── ARQ ───────────────────────────────────────────────────────────────
ARQ_WINDOW_SIZE    = 8
SEQ_NUM_BITS       = 4
SEQ_NUM_MOD        = 2 ** SEQ_NUM_BITS  # 16
MAX_HARQ_ROUNDS    = 3
CRC_BITS           = 32

# ── ACK Types ─────────────────────────────────────────────────────────
ACK_OK          = 0x0
ACK_UP          = 0x1
ACK_DOWN        = 0x2
NACK            = 0x3
NACK_IR         = 0x4
BREAK           = 0x5
PROBE_REQ       = 0x6
QRT             = 0x7
WAIT            = 0x8
CONNECT_ACK     = 0x9

# ── Guard Intervals ──────────────────────────────────────────────────
GUARD_FAST_SYMS     = 1   # 24.5 ms — modern solid-state T/R
GUARD_STANDARD_SYMS = 2   # 49 ms — typical relay-based T/R
GUARD_SLOW_SYMS     = 3   # 73.5 ms — slow relay / amplifier

# ── Session Timeouts ──────────────────────────────────────────────────
CONNECT_TIMEOUT_S      = 30
DISCONNECT_TIMEOUT_S   = 30
PROBE_PAUSE_S          = 2.0
CONSECUTIVE_FAIL_PROBE = 2
CONSECUTIVE_FAIL_WAIT  = 5
CONSECUTIVE_FAIL_DISC  = 10

# ── TCP Host Interface (VARA-compatible) ──────────────────────────────
DEFAULT_CMD_PORT  = 8400
DEFAULT_DATA_PORT = 8401

# ── Emergency Mode (DSSS) ────────────────────────────────────────────
DSSS_CHIP_RATE      = 200     # chips/second
DSSS_CODE_LENGTH    = 31      # Gold code length
DSSS_PROCESSING_GAIN_DB = 10 * 2.7  # 10*log10(31) ≈ 14.9 dB
DSSS_DATA_RATE      = DSSS_CHIP_RATE / DSSS_CODE_LENGTH  # ~6.45 bps
DSSS_BANDWIDTH_HZ   = 200
