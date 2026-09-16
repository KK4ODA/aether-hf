"""Generate the numeric tables inside the air-interface specification (roadmap P2-7).

    python tools/make_spec.py [--check]

``docs/spec/air-interface.md`` is a public document, and a public document that disagrees
with the implementation is worse than none at all. Every number in it that the code also
knows — waveform numerology, the mode table, frame layouts, the chip and preamble constants
— is generated from the model and written between ``<!-- BEGIN:name -->`` markers. Prose
outside the markers is written by hand and never touched.

``--check`` regenerates into memory and fails if the file is out of date, so CI catches a
spec that has drifted from the code instead of a reader catching it on the air.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.frame.modes import (
    LONG,
    NARROW,
    PAYLOAD_CRC,
    PREAMBLE_SYMBOLS,
    WIDE,
    AirInterface,
    FrameLayout,
)
from aether_model.link.frames import CALL_BYTES, CONTROL_BYTES, DATA_HEADER, WINDOW
from aether_model.link.rate import AWGN_THRESHOLD_DB, NARROW_AWGN_THRESHOLD_DB
from aether_model.phy.papr import CLIP_TARGET_DB, CLIP_TARGET_DENSE_DB
from aether_model.phy.preamble import (
    FLOOR_CHIP_CORRELATION_BOUND,
    FLOOR_SC_SEEDS,
    MODE_CHIP_SEED,
    N_RV,
    SC_SEEDS,
    preamble,
)

SPEC = Path(__file__).resolve().parents[1] / "docs" / "spec" / "air-interface.md"


def _table(headers: list[str], rows: list[list[str]]) -> str:
    out = ["| " + " | ".join(headers) + " |", "|" + "---|" * len(headers)]
    out += ["| " + " | ".join(r) + " |" for r in rows]
    return "\n".join(out)


def waveform_block(air: AirInterface = WIDE) -> str:
    P = air.params
    pre = preamble(P)
    rows = [
        ["Baseband sample rate", f"{P.fs_baseband:.0f} Hz", "complex"],
        ["Audio sample rate", f"{P.audio_rate:.0f} Hz", f"interpolation x{P.resample_factor}"],
        ["FFT size N", f"{P.fft_size}", "subcarriers in the transform"],
        ["Subcarrier spacing", f"{P.subcarrier_spacing_hz:.0f} Hz", "fs / N"],
        ["Useful symbol time", f"{P.useful_symbol_s * 1e3:.1f} ms", "1 / spacing"],
        ["Cyclic prefix", f"{P.cp_samples} samples ({P.cp_s * 1e3:.1f} ms)", "before windowing"],
        [
            "Window taper",
            f"{P.taper_samples} samples",
            f"raised cosine; effective CP {P.effective_cp_s * 1e3:.1f} ms",
        ],
        [
            "Symbol period",
            f"{P.symbol_samples} samples ({P.symbol_period_s * 1e3:.2f} ms)",
            "CP + N",
        ],
        ["Symbol rate", f"{P.symbol_rate_bd:.2f} Bd", ""],
        ["Active subcarriers", f"{P.n_carriers}", "centred on the passband centre"],
        ["Comb pilots", f"{P.n_pilot_carriers}", f"every {P.pilot_carrier_spacing}th, both edges"],
        ["Data subcarriers", f"{P.n_data_carriers}", "per ordinary symbol"],
        ["Occupied bandwidth", f"{P.occupied_bandwidth_hz:.0f} Hz", "active carriers x spacing"],
        ["Passband centre", f"{P.centre_hz:.0f} Hz", "audio"],
        [
            "Full pilot symbols",
            f"every {P.pilot_symbol_period}th data symbol",
            "all carriers known",
        ],
        ["Preamble", f"{PREAMBLE_SYMBOLS} symbols", "two identical Schmidl-Cox symbols"],
        [
            "Mode/RV chips",
            f"{pre.n_chips}",
            f"{N_RV} x {air.n_modes} sequences, pairwise |correlation| <= "
            f"{air.chip_correlation_bound}",
        ],
        [
            "Acquisition threshold",
            f"{air.acquisition_threshold}",
            "normalised matched-filter peak",
        ],
    ]
    if air.floor_long is not None:
        rows += [
            [
                "Floor preamble",
                f"{air.floor_long.preamble_symbols} symbols",
                "identical Schmidl-Cox symbols of the floor sequences (ADR-0009)",
            ],
            [
                "Floor mode/RV chips",
                f"{pre.n_chips_for(air.floor_long)}",
                f"{N_RV} x {air.n_modes} sequences, pairwise |correlation| <= "
                f"{FLOOR_CHIP_CORRELATION_BOUND}",
            ],
            [
                "Floor acquisition threshold",
                f"{air.floor_acquisition_threshold}",
                "seven-window average of the floor references' normalised peak",
            ],
        ]
    return _table(["Parameter", "Value", "Notes"], rows)


def layout_block(air: AirInterface = WIDE) -> str:
    rows = []
    for layout in air.layouts:
        rows.append(
            [
                layout.name.upper(),
                f"{layout.preamble_symbols} + {layout.data_symbols} = {layout.total_symbols}",
                f"{layout.duration_s * 1e3:.0f} ms",
                f"{layout.samples}",
                ", ".join(str(i) for i in layout.pilot_symbol_indices),
                f"{layout.qam_symbols}",
            ]
        )
    return _table(
        ["Layout", "Symbols", "Duration", "Samples (8 kHz)", "Full pilot symbols", "QAM slots"],
        rows,
    )


def mode_block(layout: FrameLayout = LONG, air: AirInterface = WIDE) -> str:
    thresholds = AWGN_THRESHOLD_DB if air is WIDE else NARROW_AWGN_THRESHOLD_DB
    floor = air.floor_long is not None
    rows = []
    for m in air.modes:
        # a floor mode is tabulated on the layout it actually goes out on (ADR-0009)
        layout = air.data_layout(m.index) if floor else layout
        rows.append(
            [
                str(m.index),
                m.name,
                *([layout.name.upper()] if floor else []),
                str(m.modulation.bits_per_symbol),
                str(m.code_rate),
                f"BG{m.base_graph(layout)}",
                str(m.lifting_size(layout)),
                str(m.info_bits(layout)),
                str(m.coded_bits(layout)),
                str(m.payload_bytes(layout)),
                f"{m.net_bit_rate(layout):.0f}",
                f"{thresholds[m.index]:+.1f}",
            ]
        )
    return _table(
        [
            "Mode",
            "Name",
            *(["Layout"] if floor else []),
            "bits/sym",
            "Rate",
            "Base graph",
            "Z",
            "K'",
            "E",
            "Payload B",
            "Net bps",
            "AWGN dB",
        ],
        rows,
    )


def constants_block() -> str:
    rows = [
        [
            "Payload CRC",
            f"{PAYLOAD_CRC.name}, polynomial 0x{PAYLOAD_CRC.poly:06X}, {PAYLOAD_CRC.width} bits",
        ],
        ["Schmidl-Cox PN seed, DATA", str(SC_SEEDS[0])],
        ["Schmidl-Cox PN seed, CONTROL", str(SC_SEEDS[1])],
        ["Schmidl-Cox PN seed, floor DATA", str(FLOOR_SC_SEEDS[0])],
        ["Schmidl-Cox PN seed, floor CONTROL", str(FLOOR_SC_SEEDS[1])],
        ["Mode/RV chip seed", str(MODE_CHIP_SEED)],
        ["Redundancy versions", str(N_RV)],
        ["Peak reduction target, PSK modes", f"{CLIP_TARGET_DB:.1f} dB"],
        ["Peak reduction target, QAM modes", f"{CLIP_TARGET_DENSE_DB:.1f} dB"],
        ["Link DATA header", f"{DATA_HEADER} bytes (5 with an explicit length)"],
        ["Link CONTROL frame", f"{CONTROL_BYTES} bytes"],
        ["Callsign encoding", f"6 bits/character, 9 characters in {CALL_BYTES} bytes"],
        ["Selective-repeat window", f"{WINDOW} frames"],
    ]
    return _table(["Constant", "Value"], rows)


BLOCKS = {
    "waveform": waveform_block,
    "layouts": layout_block,
    "modes": mode_block,
    "constants": constants_block,
    "waveform500": lambda: waveform_block(NARROW),
    "layouts500": lambda: layout_block(NARROW),
    "modes500": lambda: mode_block(NARROW.long, NARROW),
}


def render(text: str) -> str:
    for name, builder in BLOCKS.items():
        pattern = re.compile(rf"<!-- BEGIN:{name} -->.*?<!-- END:{name} -->", re.S)
        if not pattern.search(text):
            raise SystemExit(f"marker BEGIN:{name} missing from {SPEC}")
        block = f"<!-- BEGIN:{name} -->\n{builder()}\n<!-- END:{name} -->"
        text = pattern.sub(lambda _m, replacement=block: replacement, text)
    return text


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--check", action="store_true", help="fail if the spec is out of date")
    args = ap.parse_args()

    if not SPEC.exists():
        raise SystemExit(f"{SPEC} does not exist")
    original = SPEC.read_text(encoding="utf-8")
    updated = render(original)
    if args.check:
        if original != updated:
            print(f"{SPEC.name} is out of date - run: python tools/make_spec.py")
            return 1
        print(f"{SPEC.name} is up to date")
        return 0
    if original != updated:
        SPEC.write_text(updated, encoding="utf-8", newline="\n")
        print(f"updated {SPEC}")
    else:
        print(f"{SPEC.name} already up to date")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
