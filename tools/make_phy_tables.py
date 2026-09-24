"""Export the air-interface sequence tables for the Rust core (roadmap P3-2, P7-0).

    python tools/make_phy_tables.py [--out core/aether-phy/data/preamble_tables.json]

The Schmidl-Cox preamble sequences, the (mode, RV) chip sequences and the pilot sequence
are constants of the air interface. The model derives them from seeds through NumPy's
generator; the core compiles them in as literals (``core/aether-phy/build.rs``), because a
modem that re-derived them would depend on reproducing another language's random-number
generator bit for bit — and the whole point of the tables is that the two implementations
agree. One block per waveform: the wide one and, since P7-0, the narrow one, each with its
carrier map, its sequences, its chip-correlation bound, its acquisition threshold and the
OFDM modes on its ladder; and one block for the tone floor (ADR-0013), which both airs share:
its numerology, sync patterns, frame kinds — the fast ones of ADR-0014 and the narrow
middle ones of ADR-0015 with their data numerologies — and detector constants. Each waveform block names the tone kinds on its
ladder.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.frame.modes import (
    NARROW,
    SYNC_PATTERNS,
    SYNC_SYMBOLS,
    TONE_CONTROL,
    TONE_DATA,
    TONE_FAST,
    TONE_NARROW,
    TONE_NUMEROLOGY,
    WIDE,
    AirInterface,
    ToneKind,
)
from aether_model.phy import tone
from aether_model.phy.ofdm import PILOT_ROOT, carrier_map, zadoff_chu
from aether_model.phy.preamble import (
    MODE_CHIP_SEED,
    N_RV,
    SC_SEEDS,
    FrameType,
    preamble,
)


def pack_signs(values: np.ndarray) -> str:
    """±1 values → bits (1 meaning −1), MSB-first, hex."""
    bits = (np.asarray(values).real < 0).astype(np.uint8)
    return np.packbits(bits).tobytes().hex()


def waveform_block(air: AirInterface) -> dict[str, object]:
    params = air.params
    pre = preamble(params)
    cmap = carrier_map(params)
    even = np.asarray(pre.even)

    # The Schmidl-Cox symbols carry a PN sequence on the even carriers only; recover the raw
    # signs by dividing out the power-normalising scale the model applies.
    sc = {}
    for frame_type in FrameType:
        values = pre.sc_values(frame_type)[even]
        sc[frame_type.name] = pack_signs(values / np.abs(values).mean())

    chip_table = {}
    for rv in range(N_RV):
        for mode in range(air.n_modes):
            chip_table[f"{rv}_{mode}"] = pack_signs(pre.sequences[pre.chip_index(mode, rv)])

    return {
        "bandwidth_hz": params.bandwidth.hz,
        "n_carriers": int(cmap.n_carriers),
        "even_carriers": [int(c) for c in even],
        "sc_length": len(even),
        "schmidl_cox": sc,
        "n_modes": air.n_modes,
        "chip_length": int(pre.n_chips),
        "chips_per_symbol": int(pre.n_data),
        "chip_correlation_bound": air.chip_correlation_bound,
        "acquisition_threshold": air.acquisition_threshold,
        "mode_chips": chip_table,
        "control_mode_index": air.control_mode_index,
        # the ladder (ADR-0013, ADR-0014, ADR-0015): the tone kinds, then these OFDM modes
        "tone_data": [k.name for k in air.tone_data],
        "ofdm_ladder": list(air.ofdm_ladder),
        "layouts": [
            {
                "name": layout.name,
                "data_symbols": layout.data_symbols,
                "preamble_symbols": layout.preamble_symbols,
                "pilot_smoothing": layout.pilot_smoothing,
            }
            for layout in air.layouts
        ],
        "pilot_sequence": [
            [float(v.real), float(v.imag)] for v in zadoff_chu(cmap.n_carriers, PILOT_ROOT)
        ],
    }


def tone_kind(kind: ToneKind) -> dict[str, object]:
    return {
        "name": kind.name,
        "payload_bytes": kind.payload_bytes,
        "data_symbols": kind.data_symbols,
        "patterns": list(kind.patterns),
        "control": kind.control,
        # the data's numerology (ADR-0014, ADR-0015): the sync blocks' own for the floor's
        # kinds
        "data_symbol_samples": kind.data.symbol_samples,
        "data_ramp_samples": kind.data.ramp_samples,
        "data_tones": kind.data.tones,
    }


def tone_block() -> dict[str, object]:
    """The tone floor (ADR-0013): the same frames on both airs."""
    num = TONE_NUMEROLOGY
    det = tone.ToneDetector()
    return {
        "fs": num.fs,
        "symbol_samples": num.symbol_samples,
        "tones": num.tones,
        "ramp_samples": num.ramp_samples,
        "edge_samples": num.edge_samples,
        "gain_db": tone.TONE_GAIN_DB,
        "sync_symbols": SYNC_SYMBOLS,
        "sync_patterns": [list(p) for p in SYNC_PATTERNS],
        "control": tone_kind(TONE_CONTROL),
        "data": [tone_kind(k) for k in TONE_DATA],
        "fast": [tone_kind(k) for k in TONE_FAST],
        "narrow": [tone_kind(k) for k in TONE_NARROW],
        "detector": {
            "hop_div": det.HOP_DIV,
            "bin_div": det.BIN_DIV,
            "clip": det.CLIP,
            "max_cfo_hz": det.cfo_bins * det.bin_hz,
            "threshold": det.threshold,
            "min_hits": det.MIN_HITS,
            "min_block_hits": det.MIN_BLOCK_HITS,
            "min_first_hits": det.MIN_FIRST_HITS,
            "contradiction": det.CONTRADICTION,
            "max_contradictions": det.MAX_CONTRADICTIONS,
            "announce_threshold": tone.ANNOUNCE_THRESHOLD,
            "lookahead": tone.ToneStream.LOOKAHEAD,
            "announce_lookahead": tone.ToneStream.ANNOUNCE_LOOKAHEAD,
        },
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", default="core/aether-phy/data/preamble_tables.json")
    args = ap.parse_args()

    document = {
        "note": "Generated by tools/make_phy_tables.py. Air-interface constants: sequences "
        "are +-1, stored as bits (1 means -1) packed MSB-first into hex. One block per "
        "waveform.",
        "seeds": {
            "schmidl_cox": {
                k.name if hasattr(k, "name") else str(k): v for k, v in SC_SEEDS.items()
            },
            "mode_chips": MODE_CHIP_SEED,
        },
        "n_rv": N_RV,
        "pilot_root": PILOT_ROOT,
        "waveforms": {
            "WIDE_2300": waveform_block(WIDE),
            "NARROW_500": waveform_block(NARROW),
        },
        "tone": tone_block(),
    }

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(document, indent=1) + "\n", encoding="utf-8", newline="\n")
    for name, block in document["waveforms"].items():  # type: ignore[union-attr]
        print(
            f"{name}: SC 2 x {block['sc_length']}, chips {len(block['mode_chips'])} x "
            f"{block['chip_length']}"
        )
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
