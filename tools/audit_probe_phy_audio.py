"""Audit probe (2026-09-13): OFDM window ICI, pilot grid, preamble bandwidth, audio path, SNR reference, sync.
Run from repo root: python tools/audit_probe_phy_audio.py
See docs/AUDIT.md for interpretation."""
import numpy as np, sys
sys.path.insert(0, '.')
from aether_model.dsp.ofdm import OFDMModulator, OFDMDemodulator, SubcarrierMap
from aether_model.dsp.modulation import Mapper, Demapper
from aether_model.dsp.preamble import PreambleGenerator, PreambleDetector
from aether_model.audio.soundcard import AudioInterface
from aether_model.speed_levels import Modulation
from aether_model.constants import BASEBAND_RATE, FFT_SIZE_W
np.random.seed(1)

print("== 1. Self-inflicted EVM from RC window applied without overlap-add ==")
smap = SubcarrierMap("wide"); mod = OFDMModulator("wide"); dem = OFDMDemodulator("wide")
mp = Mapper(Modulation.QPSK)
bits = np.random.randint(0,2,smap.n_data*2).astype(np.int8)
tx = mp.map(bits)
x = mod.modulate(tx)
rx = dem.demodulate(x)
evm = np.sqrt(np.mean(np.abs(rx-tx)**2)/np.mean(np.abs(tx)**2))
print(f"  window taper = {int(0.08*256)} samples; noiseless EVM = {evm*100:.2f}%  (-> equivalent SNR ceiling {(-20*np.log10(evm)):.1f} dB)")
# Compare with window disabled
mod2 = OFDMModulator("wide"); mod2._window[:] = 1.0
rx2 = dem.demodulate(mod2.modulate(tx))
evm2 = np.sqrt(np.mean(np.abs(rx2-tx)**2)/np.mean(np.abs(tx)**2))
print(f"  without window: EVM = {evm2*100:.4f}%")

print("\n== 2. Pilot placement ==")
print("  pilot carrier idx:", smap.pilot_indices)
print("  data idx range:", min(smap.data_indices), "..", max(smap.data_indices))
print("  data carriers beyond last pilot (extrapolated):", sum(1 for d in smap.data_indices if d > max(smap.pilot_indices)))

print("\n== 3. Preamble occupied bandwidth ==")
gen = PreambleGenerator("wide"); pre = gen.generate()
for name, seg in [("ZC part A", pre[:294]), ("sync part B", pre[588:882]), ("train part C", pre[882:])]:
    X = np.abs(np.fft.fft(seg[38:38+256]))**2
    f = np.fft.fftfreq(256, 1/BASEBAND_RATE)
    tot = X.sum(); inband = X[np.abs(f) <= 1500].sum()
    print(f"  {name}: fraction of power within ±1.5 kHz = {inband/tot*100:.1f}%  (occupies {np.sum(X>0.01*X.max())*46.875:.0f} Hz)")

print("\n== 4. Audio path: complex baseband -> real audio -> baseband ==")
# a full OFDM symbol through AudioInterface._upsample/_downsample
sym = mod2.modulate(tx)  # no window to isolate the audio effect
a48 = AudioInterface._upsample(sym)
bb = AudioInterface._downsample(a48)
# scale match
bb = bb[:len(sym)]
rx3 = dem.demodulate(bb / (np.abs(bb).max()/np.abs(sym).max()))
b3 = Demapper(Modulation.QPSK).hard_demap(rx3)
print(f"  bit errors after TX->audio->RX: {np.sum(b3 != bits)}/{len(bits)}  (real() collapses ±k subcarriers)")

print("\n== 5. SNR reference: noise bandwidth 12 kHz vs 3 kHz ==")
print(f"  reported SNR is {10*np.log10(BASEBAND_RATE/3000):.1f} dB lower than the 3 kHz-referenced SNR used by HF-modem convention")
print(f"  (i.e. 'SNR=0 dB' in the tests == +{10*np.log10(BASEBAND_RATE/3000):.0f} dB SNR in 3 kHz)")

print("\n== 6. Preamble detector timing/CFO ambiguity (no noise, CFO=0) ==")
det = PreambleDetector("wide")
rx = np.concatenate([np.zeros(500), pre, np.zeros(500)])
d, off, cfo = det.detect(rx, 0.3)
print(f"  no noise: detected={d} offset={off} (true 500) cfo={cfo} (true 0)")
for true_cfo in [-100, 50, 200]:
    t = np.arange(len(rx))/BASEBAND_RATE
    d, off, cfo = det.detect(rx*np.exp(2j*np.pi*true_cfo*t), 0.3)
    print(f"  true cfo {true_cfo:+4d}: detected={d} offset={off} est cfo={cfo:+.0f}")

print("\n== 7. Effective symbol timing tolerance (CP=38 samples; detector step=73) ==")
print("  search step 73 samples > CP 38 samples -> timing error can exceed CP, guaranteed ISI even on perfect detection")
