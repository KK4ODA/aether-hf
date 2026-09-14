# ADR-0005: The front-end is plain ES modules, with no build step

Status: **accepted**, 2026-09-14. Amends ADR-0001 §3.

## Context

ADR-0001 says the desktop GUI is a Tauri v2 application with a **TypeScript** front-end, and
that the same front-end is served by `aetherd` for headless and remote monitoring. The second
half of that sentence turns out to constrain the first.

Serving the front-end from the daemon is not a nicety. A gateway is a headless machine in a
loft or a garage, and the only practical way to look at one is a browser pointed at it over an
SSH tunnel. That means the files `aetherd` serves have to exist in the repository as the
browser will load them — or the daemon has to embed the output of a build, and every gateway
build then needs Node installed to produce it.

## Decision

**The front-end is plain HTML, CSS and ES modules, with no build step and no npm
dependencies.** `aetherd` serves the directory as it stands. The Tauri shell loads the same
files.

## Consequences

Good:

* Building a gateway needs Rust and ALSA headers, and nothing else. No Node, no lockfile, no
  `node_modules` on a Raspberry Pi.
* What the browser runs is what is in the repository, which makes the front-end reviewable in
  the same way as everything else here and removes a whole class of "works in dev, broken in
  the bundle" failures.
* The daemon can serve it directly, which is what ADR-0001 asked for.

Bad, and accepted:

* **No type checking.** The front-end is a few hundred lines talking to one JSON API, and the
  API's shape is pinned by the tests on the Rust side; types would catch less here than they
  would cost. If it grows past the point where that is true, this decision should be revisited
  and a build step added — the front-end would not have to be rewritten, only compiled.
* No bundling or minification. Over loopback or an SSH tunnel this does not matter.
* No front-end framework. The UI is a status panel and a few controls; a framework would be
  more code than the thing it manages.

## Alternatives considered

| Option | Why not |
|---|---|
| TypeScript compiled by `tsc`, output committed | Committing build output invites the source and the artefact to diverge, and the divergence is invisible until somebody debugs the wrong file. |
| TypeScript compiled at daemon build time | Puts Node on the dependency list for every gateway, which is exactly the cost this avoids. |
| Embed a built bundle in the binary with `include_dir!` | Solves the serving problem and keeps the build step. Reasonable, and the right answer if the front-end grows enough to need a compiler. |
