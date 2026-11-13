# ADR-0110: A decompressed stream grows at most a hundred times what came off the air

**Status:** accepted, 2026-11-13, the author's go. Daemon only (`core/aetherd/src/compress.rs`,
`station.rs`): **no wire change, no model change, no configuration key.**

## 1. Context

A session that both stations agreed to compress carries a deflate stream (RFC 1951), and the
`Decompressor` handed the program whatever it inflated to. Deflate expands up to about a thousand
times, and the other station chooses what it sends. Measured with this module's own coder: 8 MiB
of zeros goes on the air in **8,161 bytes**, 1,028 times smaller. A few kilobytes off the air could
ask for megabytes in the daemon, and a session's worth for a gigabyte, in `delivered` and then in
the program's data port. Found while writing the fuzz targets (`core/fuzz/`, the `inflate`
target), by reading the code: the fuzzer's inputs are too short to show it.

What real traffic does: text comes out under twice its size on the air (this module's tests: 27 %
saved on a 429-byte message, 44 % on a 1.5 kB one), and a Winlink message, already compressed by
its program, about once.

## 2. Decision

A stream may grow to **100 times** what has come off the air (`MAX_EXPANSION`), and **1 MiB** more
(`GROWTH_SLACK`). Past that it is refused as a malformed stream is: nothing more of it is
delivered, what the refused piece had inflated to is dropped, and the session goes on carrying
bytes the program will not see, as before for a malformed stream.

* **100** is fifty times what real text needs, and a tenth of what deflate allows.
* **The slack** lets a short stream that compresses unusually well through whole: the bound
  matters only once a stream is past a megabyte.
* The log says which it was, once, when the stream is given up on: "the peer's compressed stream
  grew past 100 times what came off the air", or, as before, that it could not be read. It used to
  say the second at every later piece.

## 3. Consequences

* The 8 MiB stream is stopped after **1,018,070 bytes** delivered, under its bound of 100 × 8,161
  bytes and 1 MiB.
* A 429-byte message repeated 600 times, 257,400 bytes, goes on the air in **2,700 bytes** (95
  times smaller) and still comes through whole, inside the slack. A stream that repetitive and
  longer than about a megabyte would be refused; no program sends one.
* Tests (`compress.rs`): `a_stream_that_grows_past_its_bound_is_refused`,
  `a_long_stream_well_within_its_bound_is_untouched`, `a_malformed_stream_is_not_called_too_large`.
