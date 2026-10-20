"""Link layer: ARQ engine and session state machine (roadmap P2-1, ADR-0069).

Everything here is PHY-agnostic. The engine (:mod:`aether_model.link.granted`) consumes *soft
frames* (anything that can report its kind, mode, RV, timing and SNR and can be decoded against
an opaque HARQ buffer) and produces *actions* (frames to transmit, data to deliver, events to
report). :mod:`aether_model.link.sim` drives two engines through a lossy pipe for fast protocol
tests; :mod:`aether_model.link.harness` drives them through the real PHY and channel simulator.
"""
