# ADR-0114: A program may say who it is without claiming a callsign

**Status:** accepted, 2026-11-13, the author's choice ("the app stays connected"). Control API
only: no wire change, no model change, no configuration key.

## 1. Context

The panel's header chip names the programs using the station (ADR-0106). When no program is
there and calls to the station's own callsign are not answered, it says *calls to KK4ODA-1 not
answered* (ADR-0107: under automatic control only a program answers calls).

The author ran the RadioPost app on KK4ODA-1 and saw that warning. To the operator the app was
connected to Aether, so the chip looked wrong. In fact the app opens a connection to Aether's
control API only for a call or while it listens (ADR-0108 §2.2). Between calls it holds none, so
Aether cannot see it. The author asked for the chip to show the program instead, in green with a
check.

Aether has only one way for a program to name itself: a `claim`. A claim of callsigns makes the
program speak for the station (ADR-0108), so the station's own callsigns rest while it lasts. A
program that only calls out must not change what is answered just by being open.

## 2. Decision

1. **`hello {app}`.** A control-API method. The program on this connection says who it is, for as
   long as the connection lasts. It claims no callsign, so nothing about answering changes.
   * Said again, the new name replaces the old.
   * A `claim` on the same connection that names no `app` takes the hello's name.
   * Over `POST` it is refused (`unsupported`), as a claim is.
   * The name is 1–64 characters (`bad_params` otherwise).
   * A `log` event named `claims` says *\<app\> is using the station*, and *\<app\> has gone*
     when the connection ends. The panel refreshes on it.
2. **`status.programs`** lists every connection that said hello: `id`, `app`, `kind`,
   `since_ms`. A program that also claims callsigns is in `claims` too.
3. **The chip.** Every program connected is a green check and its name, without the version at
   its end (the tooltip keeps the whole name). A program in both lists, or two connections with
   one name, is shown once. *calls to … not answered* shows only when no program is there; with
   a program that holds no callsign, it moves to the tooltip.
4. **RadioPost's part** (in that repository): the app holds one connection to Aether while it
   runs and says `hello` with the name its calls claim under, so the two merge on the chip.

## 3. Consequences

* With the RadioPost app open, the chip reads *✓ RadioPost client*; while it listens,
  *✓ RadioPost client · listening*. Closed, the chip says what it said before.
* Host programs and KISS programs get the green check too. They were already named.
* A RadioPost app older than its part of this decision still shows the warning between calls.

## 4. Not done

* **Aether looking for the program on its own** (its process, or its panel's port). That is
  fragile, and it would say a program is there when it is not using Aether.
* **A claim with no callsigns.** It would make the program speak for the station (ADR-0108)
  and silence the station's own callsigns while the app is merely open.
