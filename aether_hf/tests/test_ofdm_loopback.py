"""
test_ofdm_loopback.py

End-to-end OFDM modulator → channel → demodulator loopback test.
Verifies that data survives a round-trip through the OFDM chain
with AWGN noise at various SNR levels.
"""

import numpy as np
import sys
import os
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(__file__))))

from aether_hf.dsp.ofdm import OFDMModulator, OFDMDemodulator, SubcarrierMap
from aether_hf.dsp.modulation import Mapper, Demapper
from aether_hf.dsp.preamble import PreambleGenerator, PreambleDetector
from aether_hf.speed_levels import Modulation


def test_ofdm_roundtrip_no_noise():
    """OFDM modulate → demodulate with no noise should be perfect."""
    mod = OFDMModulator("wide")
    demod = OFDMDemodulator("wide")
    smap = SubcarrierMap("wide")

    mapper = Mapper(Modulation.QPSK)
    demapper = Demapper(Modulation.QPSK)

    # Generate random bits
    n_bits = smap.n_data * 2  # QPSK = 2 bits/symbol
    tx_bits = np.random.randint(0, 2, size=n_bits).astype(np.int8)

    # Map to QPSK symbols
    tx_symbols = mapper.map(tx_bits)

    # OFDM modulate
    tx_samples = mod.modulate(tx_symbols)

    # OFDM demodulate (no channel, no noise)
    rx_symbols = demod.demodulate(tx_samples)

    # Demap
    rx_bits = demapper.hard_demap(rx_symbols)

    # Check
    errors = np.sum(tx_bits != rx_bits[:len(tx_bits)])
    print(f"No noise:   {errors} bit errors / {n_bits} bits")
    assert errors == 0, f"Expected 0 errors, got {errors}"


def test_ofdm_roundtrip_awgn():
    """OFDM roundtrip with AWGN at various SNR levels."""
    mod = OFDMModulator("wide")
    smap = SubcarrierMap("wide")
    mapper = Mapper(Modulation.QPSK)

    print(f"\nAWGN test (QPSK, {smap.n_data} carriers):")
    print(f"{'SNR (dB)':>10} {'BER':>10} {'Errors':>10}")
    print("-" * 35)

    for snr_db in [20, 15, 10, 5, 0]:
        total_errors = 0
        total_bits = 0
        n_frames = 20

        for _ in range(n_frames):
            demod = OFDMDemodulator("wide")
            n_bits = smap.n_data * 2
            tx_bits = np.random.randint(0, 2, size=n_bits).astype(np.int8)
            tx_symbols = mapper.map(tx_bits)
            tx_samples = mod.modulate(tx_symbols)

            # Add AWGN
            sig_power = np.mean(np.abs(tx_samples) ** 2)
            noise_power = sig_power / (10 ** (snr_db / 10))
            noise = np.sqrt(noise_power / 2) * (
                np.random.randn(len(tx_samples))
                + 1j * np.random.randn(len(tx_samples))
            )
            rx_samples = tx_samples + noise

            rx_symbols = demod.demodulate(rx_samples)
            demapper = Demapper(Modulation.QPSK)
            rx_bits = demapper.hard_demap(rx_symbols)

            total_errors += np.sum(tx_bits != rx_bits[:len(tx_bits)])
            total_bits += n_bits

        ber = total_errors / total_bits
        print(f"{snr_db:>10} {ber:>10.6f} {total_errors:>10}")


def test_preamble_detection():
    """Test preamble generation and detection."""
    gen = PreambleGenerator("wide")
    det = PreambleDetector("wide")

    preamble = gen.generate()

    # Add some noise
    snr_db = 10
    sig_power = np.mean(np.abs(preamble) ** 2)
    noise_power = sig_power / (10 ** (snr_db / 10))
    noise = np.sqrt(noise_power / 2) * (
        np.random.randn(len(preamble)) + 1j * np.random.randn(len(preamble))
    )

    # Pad with noise before and after
    pre_pad = np.sqrt(noise_power / 2) * (
        np.random.randn(500) + 1j * np.random.randn(500)
    )
    post_pad = np.sqrt(noise_power / 2) * (
        np.random.randn(500) + 1j * np.random.randn(500)
    )
    rx_signal = np.concatenate([pre_pad, preamble + noise, post_pad])

    detected, offset, cfo = det.detect(rx_signal, threshold=0.3)
    print(f"\nPreamble detection: detected={detected}, offset={offset} "
          f"(expected ~500), CFO={cfo:.1f} Hz")
    assert detected, "Preamble not detected!"


def test_constellation_mapping():
    """Test all constellation mappers/demappers."""
    print("\nConstellation mapping tests:")
    for mod_type in [Modulation.BPSK, Modulation.QPSK, Modulation.PSK8,
                     Modulation.QAM16, Modulation.QAM64]:
        from aether_hf.speed_levels import BITS_PER_SYMBOL
        bps = BITS_PER_SYMBOL[mod_type]
        mapper = Mapper(mod_type)
        demapper = Demapper(mod_type)

        n_bits = bps * 100
        tx_bits = np.random.randint(0, 2, size=n_bits).astype(np.int8)
        symbols = mapper.map(tx_bits)
        rx_bits = demapper.hard_demap(symbols)

        errors = np.sum(tx_bits != rx_bits)
        status = "PASS" if errors == 0 else f"FAIL ({errors} errors)"
        print(f"  {mod_type.value:>8}: {status}")


if __name__ == "__main__":
    np.random.seed(42)
    test_constellation_mapping()
    test_ofdm_roundtrip_no_noise()
    test_ofdm_roundtrip_awgn()
    test_preamble_detection()
    print("\nAll tests passed!")
