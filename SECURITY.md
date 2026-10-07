# Security policy

Aether HF is amateur-radio software; it never encrypts traffic (by law), and the few secrets
it holds — the control API's token, an upload code — never go over the air. Security still
matters because the modem exposes local TCP services, parses frames received off the air,
keys a transmitter, installs updates, and can upload files a station asked for.

## Reporting

Open a private security advisory on GitHub (Security → Report a vulnerability) or e-mail
the maintainer listed on the repository profile. Please include the version, platform and
a way to reproduce (Help / About shows the version). You should hear back within a week.

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
- File sharing (`share.upload`): the daemon uploads only a zip `share.prepare` wrote, only
  to a Google Apps Script address (`https://script.google.com/…/exec`) with a code the
  asking station issued. Any path that sends other files, sends elsewhere, or puts the
  settings' secrets in the zip is a bug.

## Out of scope

Encryption of amateur traffic (illegal in most jurisdictions), and anything requiring
physical access to the operator's computer.
