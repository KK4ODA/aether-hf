# Security policy

Aether HF is amateur-radio software; it never carries secrets and (by law) never encrypts
traffic. Security still matters because the modem exposes local TCP services, parses
frames received off the air, keys a transmitter, and installs updates.

## Reporting

Open a private security advisory on GitHub (Security → Report a vulnerability) or e-mail
the maintainer listed on the repository profile. Please include the version, platform and
a way to reproduce. You should hear back within a week.

## Scope we care about

- Network-facing services (the control API, the VARA-compatible host interface, the KISS
  port): they bind to `127.0.0.1` by default; anything that lets a remote host reach them
  without opt-in is a bug. The control API refuses to start on another address without a
  token.
- Malformed over-the-air frames: the receiver must never crash, hang or key the
  transmitter because of what it decoded.
- Update mechanism: manifests are signed; any path that installs an unsigned build, or an
  older one the operator did not ask for, is a bug.
- Configuration files: no code execution, no path traversal from config values.
- PTT safety: a stuck transmitter is treated as a safety bug (watchdog, maximum key time).
- The regulatory gate: any path that keys the transmitter without the policy's leave
  (ADR-0018) — a frame, a tone, a Morse identifier, a program's KISS frame — is a bug.

## Out of scope

Encryption of amateur traffic (illegal in most jurisdictions), and anything requiring
physical access to the operator's computer.
