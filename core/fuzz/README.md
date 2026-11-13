# Fuzzing

libFuzzer harnesses, through [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz), for the code
that takes bytes from outside the station. Every encoding is published, so anyone on the frequency
can send a frame whose CRC passes, and any program on the computer can write to the host ports. A
panic in that code stops the daemon. Mercury runs the same kind of harnesses on its own parsers
(its `tests/fuzz/`).

| Target | What it feeds | Where it comes from |
|---|---|---|
| `air_frames` | One frame's payload to every link decoder: data, probe, connect, grant, callsign, datagram | the air |
| `datagrams` | A run of frames through `read_fragment` and the `Reassembler`, each a length byte and its bytes | the air, no session needed |
| `inflate` | A compressed session's bytes to the `Decompressor`, in pieces the first byte sizes | the air |
| `kiss` | A KISS client's stream through the `Decoder`, `classify` and `guess_client` | the KISS port |
| `vara_commands` | A host program's lines, each ended by a carriage return, through `HostState` | the VARA-compatible port |
| `link_engine` | Two granted engines in a session, on WIDE_2300 or NARROW_500, carrying data both ways over an air the input steers: the caller's frames heard, lost or altered on the way, frames anyone could send put on the air, time passing, aborts — each frame one the PHY could hand over (a mode of the ladder, a two-bit redundancy version, a payload of the frame's size) | the air, in a session |
| `phy_receive` | A control frame and a data frame, rendered once, damaged as the input says (stretches overwritten, scaled, silenced, spliced, a burst of static, a carrier) and fed to a `StreamingReceiver` in pieces the input sizes | the air, as audio |

`link_engine` also checks the engines while the air is honest (frames lost, none altered or put on
it): what each delivers is the start of what the other was given to send, in order. `phy_receive`
checks that the signal undamaged gives both frames back whole, however the audio is cut; the
receiver hands a frame over up to 0.2 s after its end, by how the audio was cut, so the signal ends
0.3 s after its last frame. Each check was shown to stop on a planted fault: a missing block skipped
by the engine, and the receiver dropping a block's audio past 2,048 samples. `phy_receive` runs the
whole receiver on every input, two frames' worth of audio, so it is slow: run it without the address
sanitizer (`-s none`), and with `-fork=N` on a machine with cores to spare.

This is not part of CI. libFuzzer needs a nightly toolchain, and CI builds on stable, so the
directory is a workspace of its own and not a member of `core/`. **What a fuzzer finds is fixed in
the crate it was found in, with a unit test that replays the input**, and CI runs that.

## Running

Once: `rustup toolchain install nightly` and `cargo install cargo-fuzz`. On Linux the daemon's
crate needs `libasound2-dev`, as it does for any build. Then, from `core/fuzz/`:

```
cargo +nightly fuzz run air_frames -- -max_total_time=300
cargo +nightly fuzz run kiss corpus/kiss seeds/kiss -- -max_total_time=300
cargo +nightly fuzz run -s none phy_receive -- -max_total_time=600 -fork=2
```

`seeds/` holds hand-made starting inputs: a host program's commands, KISS frames, DEFLATE streams.
The air targets start from nothing; libFuzzer finds their headers within seconds. `corpus/` and
`artifacts/` are cargo-fuzz's own and are not committed. A crash leaves its input in
`artifacts/<target>/`; `cargo +nightly fuzz run <target> <that file>` replays it.
