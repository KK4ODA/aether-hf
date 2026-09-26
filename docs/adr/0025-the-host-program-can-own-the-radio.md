# ADR-0025: The host program can own the radio — keying on PTT ON, as VARA is keyed

**Status:** accepted, 2026-09-26. The daemon (`[ptt] kind = "host"`, `ptt::HostKeyed`, the
control API's refusal with no host attached), the panel (Setup step 2, the Session tab's dial
card) and configuration schema 8.

## 1. Context

Aether is meant to drop in where VARA HF is used. With VARA, the host program owns the radio:
VarAC reads and sets the frequency over CAT (for its Auto-QSY, CQ slots and return to the
calling frequency) and keys the radio itself when the modem says `PTT ON`; VARA only makes
and hears audio. Aether keyed the radio itself (a serial line, CAT, a CM108 pin, `rigctld`) or
not at all (VOX), and holding the radio's CAT port shut the host program out of it. A review of
the VarAC and Winlink Express integration (2026-09-26) proposed Aether serving the radio to the
host over a Hamlib port; the author chose VARA's model instead — simpler, and what operators
already know.

Aether's rules check (ADR-0018) judges every transmission at the dial, and a station whose host
program owns the radio can read no dial. Three ways were weighed: no rules (as VARA, the
operator checking every transmission); rules with a dial the operator declares, which a host's
QSY would leave stale; and rules with the dial read from a Hamlib or FLRig server the host
program also uses. The author chose the first for this mode.

## 2. Decision

* **`[ptt] kind = "host"`**: Aether opens no port on the radio. Its keying is the `PTT ON` and
  `PTT OFF` lines of the host interface, which the host program keys the radio on (VarAC: RIG
  tab, PTT Configuration and Frequency Control on CAT). The first audio follows `PTT ON` by
  `lead_ms` (150 by default, 50–1000): VARA allows the host 100 ms.
* **Choosing it sets the rules to None** in the panel (Setup step 1): the operator checks every
  transmission, as with VARA. The daemon does not force it — an operator who declares the dial
  may keep a regulatory profile, and the Session tab then asks for the dial as for any radio
  that cannot report it.
* **A station keyed this way starts nothing with no host program attached**: nobody would key
  the radio. The panel's and the KISS programs' requests to transmit (`connect`, `beacon`,
  `beacon.every`, `probe`, `test.start`, `tune`, `drive.set`, `ptt.test`, `datagram.send`) are
  refused, retryable, with the reason. What the host program asks comes while it is attached.
* **Keying and rules are profile settings**, so a "Standalone" profile and a host-program
  profile switch the station between the two.
* **Configuration schema 8** (`host_keying`, a step that changes nothing): a version from before
  cannot read a file keyed this way, and the new number sends it — and the shell's restore — to
  the copy kept before (`station.toml.bak-v7`).

## 3. Consequences

* VarAC's frequency features work as they do with VARA; Winlink Express keeps its own radio
  setup.
* With no rules, nothing in Aether stops a transmission outside the operator's privileges or a
  segment: the responsibility is the operator's, exactly as with VARA. The safety net of
  ADR-0018 needs a profile, and a dial Aether can know.
* The Session tab hides the dial list and the declared dial in this mode and says the host
  program tunes the radio.
* A call answered while no host program is attached goes to a radio nobody keys; the caller
  hears nothing and gives up, as it would of a station switched off.
* Not built: reading the dial from the host program's Hamlib or FLRig server, which would let
  the rules run with a host-owned radio. It waits for an operator who wants both.
