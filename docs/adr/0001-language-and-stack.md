# ADR-0001: Python reference model, Rust production core, Tauri desktop shell

**Status:** accepted (2026-09-13) · **Roadmap:** §4.3, P3-1, Phase 4

## Context

The audit (`docs/AUDIT.md`) found no working code worth carrying forward, so the language
question is open. Requirements that drive it (`docs/ROADMAP.md` §1, `docs/COMMUNITY-CONCERNS.md`):

- Windows-first desktop software that installs in one step, updates itself securely, and
  never asks a user to install a runtime or swap a DLL (the Mercury thread's loudest
  complaint: "3 hours to get running", "Python dependency hell", "hand-copied hamlib DLL").
- Real-time audio DSP with a GUI that cannot stall or kill a transmission.
- Headless operation on Linux/ARM64 (Raspberry Pi gateways) from the same code.
- Fast iteration on the waveform, which needs plotting, notebooks and a calibrated
  simulator — a research environment, not a product runtime.
- A future FM PHY sharing everything above the PHY.

## Decision

1. **Python is the reference model and benchmark harness** (`model/`). All waveform,
   FEC and protocol design happens here first; it produces golden vectors and the
   FER/throughput curves. It is never shipped to users.
2. **The shipped modem is a Rust workspace** (`core/`): `aether-fec`, `aether-phy-hf`,
   `aether-link`, `aether-modem`, `aether-hal`, `aether-api`, and the `aetherd` daemon.
   Modules are ported from the model against golden vectors and must stay bit-exact with it.
3. **The desktop GUI is a Tauri v2 application** (`app/`) with a TypeScript front-end that
   talks to `aetherd` over the native WebSocket/REST API. The same front-end is served by
   `aetherd` for headless/remote monitoring.
4. **Hamlib is bundled** with the Windows build (with an override path and a `rigctld`
   option) so radio support never requires user-side file surgery.

## Alternatives considered

| Option | Why not |
|---|---|
| Python-only product (PySide6 + PyInstaller + tufup) | Fastest to first demo, but 150–250 MB installs, antivirus false positives on PyInstaller binaries, GIL/GC jitter in the audio path, and the dependency-hell failure mode users already punished Mercury for. Kept as the fallback if the Rust port stalls: the model *is* runnable. |
| C++ core + Qt | Equivalent performance; weaker packaging/updater story (WinSparkle/Velopack, hand-rolled installers), more memory-safety risk in a frame parser that eats off-air bytes, smaller pool of contributors comfortable with modern C++ than expected for a 2026 project. |
| Rust core + egui/iced native GUI | Fewer moving parts than Tauri, but loses the "same UI in a browser for remote/headless" property, which the gateway use-case needs. |

## Consequences

- Two implementations of the PHY/codec exist (model + core). Divergence is prevented by
  golden vectors in `vectors/` and a CI job that runs both against the same inputs.
- Contributors need Rust for the product and Python for the model; the model stays the
  low-barrier entry point.
- Cross-platform builds come almost free (cpal, serialport, hidapi, Tauri bundler); Linux
  x86-64 and ARM64 become first-class targets in Phase 3.
- Signed auto-updates use Tauri's updater plug-in (Ed25519 manifests) — no custom updater.
- The Rust port is not started until the model's waveform and FEC are stable (P1-9 golden
  vectors), to avoid porting twice.
