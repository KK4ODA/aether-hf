# Fuzzing

libFuzzer harnesses, through [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz), for the parsers
that take bytes from outside the station. Every encoding is published, so anyone on the frequency
can send a frame whose CRC passes, and any program on the computer can write to the host ports. A
panic in one of these parsers stops the daemon. Mercury runs the same kind of harnesses on its own
parsers (its `tests/fuzz/`).

| Target | What it feeds | Where it comes from |
|---|---|---|
| `air_frames` | One frame's payload to every link decoder: data, probe, connect, grant, callsign, datagram | the air |
| `datagrams` | A run of frames through `read_fragment` and the `Reassembler`, each a length byte and its bytes | the air, no session needed |
| `inflate` | A compressed session's bytes to the `Decompressor`, in pieces the first byte sizes | the air |
| `kiss` | A KISS client's stream through the `Decoder`, `classify` and `guess_client` | the KISS port |
| `vara_commands` | A host program's lines, each ended by a carriage return, through `HostState` | the VARA-compatible port |

This is not part of CI. libFuzzer needs a nightly toolchain, and CI builds on stable, so the
directory is a workspace of its own and not a member of `core/`. **What a fuzzer finds is fixed in
the crate it was found in, with a unit test that replays the input**, and CI runs that.

## Running

Once: `rustup toolchain install nightly` and `cargo install cargo-fuzz`. On Linux the daemon's
crate needs `libasound2-dev`, as it does for any build. Then, from `core/fuzz/`:

```
cargo +nightly fuzz run air_frames -- -max_total_time=300
cargo +nightly fuzz run kiss corpus/kiss seeds/kiss -- -max_total_time=300
```

`seeds/` holds hand-made starting inputs: a host program's commands, KISS frames, DEFLATE streams.
The two air targets start from nothing; libFuzzer finds their headers within seconds. `corpus/` and
`artifacts/` are cargo-fuzz's own and are not committed. A crash leaves its input in
`artifacts/<target>/`; `cargo +nightly fuzz run <target> <that file>` replays it.
