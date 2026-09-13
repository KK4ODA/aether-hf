"""Hardware abstraction for the reference model: audio backends and level metering (P1-8).

The shipped modem's HAL lives in the Rust core (ADR-0001); this package exists so the model
can run end-to-end on real audio (sound card or WAV files) and so the same backend contract
can be mirrored in Rust.
"""
