# Field sessions

Recordings from the air, kept so the receiver can be held to them. Each session is two
files with one name under `sessions/`:

* `<name>.wav` — what the sound card delivered, mono 16-bit at 48 kHz;
* `<name>.json` — what the modem made of it: every frame it found (mode, SNR, offset,
  decoded or not), every event, when the transmitter was keyed, the counters at the end, and
  the operator's notes.

`aetherd` writes them: **Record** on the panel's Session tab, `record.start` on the control
API, or `[record] auto = true` for every session. `docs/user/field-test.md` is the protocol.

`core/aetherd/tests/field.rs` replays every session here through the receiver on every
`cargo test` and fails if fewer frames decode than did on the day. That is the whole point:
a receiver change that loses a frame on real air is a regression, whatever the simulator
says. Try one by hand with

```bash
aetherd --replay field/sessions/<name>.wav
```

## What to keep

Not every session. Keep the ones that teach something the simulator does not: a path, a
band, a condition — a session at the edge of a mode, one with an interferer, one on a Pi
through a cheap interface, one that failed and should not have. Say why in the sidecar's
`session.notes` (the `notes` parameter of `record.start`, or `record.notes` before an
automatic recording). A minute of audio is 5.8 MB; keep sessions short, or trim them with
any audio editor — the sidecar's `t_s` values are relative to the start of the file, so a
trim from the front shifts them.

## Naming

`YYYYMMDD-HHMMSS_<mycall>_<remote>` as the daemon writes it, `_listen` for a monitoring
session with no peer. Slashes in callsigns become dashes.
