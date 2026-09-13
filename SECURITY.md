# Security policy

Aether HF is amateur-radio software; it never carries secrets and (by law) never encrypts
traffic. Security still matters because the modem exposes local TCP services, parses
frames received off the air, and (later) installs updates.

## Reporting

Open a private security advisory on GitHub (Security → Report a vulnerability) or e-mail
the maintainer listed on the repository profile. Please include the version, platform and
a way to reproduce. You should hear back within a week.

## Scope we care about

- Network-facing services (VARA-compatible TCP, native control API): they bind to
  `127.0.0.1` by default; anything that lets a remote host reach them without opt-in is a
  bug.
- Malformed over-the-air frames: the receiver must never crash, hang or key the
  transmitter because of what it decoded.
- Update mechanism: manifests are signed; any path that installs an unsigned or
  downgraded build is a bug.
- Configuration files: no code execution, no path traversal from config values.
- PTT safety: a stuck transmitter is treated as a safety bug (watchdog, maximum key time).

## Out of scope

Encryption of amateur traffic (illegal in most jurisdictions), and anything requiring
physical access to the operator's computer.
