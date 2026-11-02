# logicaffeine-wirebench

Every message two LOGOS peers exchange goes through the wire codec in `logicaffeine_compile`, and the fair question is whether you would have been better off with bincode, MessagePack or protobuf. This crate answers it with a reproducible head-to-head: the same logical data (int and float arrays, struct lists, records, strings) goes through the LOGOS codec and through industry serializers, measured for encoded size, encode ns/op and decode ns/op. It is the measuring instrument, not the verdict: the published results are [`benchmarks/results/latest-codec.json`](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/benchmarks/results/latest-codec.json), charted in the root README's [wire codec benchmarks](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md#wire-codec).

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace, tier 5. It is a `publish = false` tool pinned at `version = "0.0.0"` and kept outside `default-members`, so a plain `cargo build` never compiles its competitor serializers; the test suite builds it as a dev-dependency of `logicaffeine-tests`.

## Role in the workspace

`logicaffeine_compile` carries the peer-messaging wire codec (`message_to_wire_with` / `message_from_wire`) and its size↔speed dials. They are `WireNumerics` (Varint / Fixed / GroupVarint), `WireStructure` (Off / Affine / Auto, the per-column closed-form menu), `WireFloats` (Memcpy / XorDelta Gorilla), and optional `WireCompression` (Deflate / Lz4 / Zstd). This crate builds the identical payload twice, as a `RuntimeValue` for the LOGOS codec and as a plain Rust type for the serde competitors. It encodes and decodes both, then prints a per-scenario table of byte size, encode ns/op, decode ns/op, and the ratio against JSON.

It is a library (`logicaffeine_wirebench`) plus a thin `[[bin]]` (`wirebench`, which calls `run()`) over `logicaffeine_compile`'s `concurrency::marshal` codec and `interpreter` value types. The library's `build_report(iters) -> CodecReport` runs every scenario and returns the structured report: `CodecScenario` rows of `RowOut`, `CodecMetadata` provenance, and the all-knobs winner's `ChosenConfig`. The `superiority`, `bulk_cursor` and `codec_report` suites in `logicaffeine-tests` drive it directly.

Payloads are **seeded random** via a deterministic SplitMix64 RNG: reproducible, but not a clean `0..n` sequence that would let varints and the affine encoding win for free. Scenarios: random ints, sensor-like f64 floats, time-series floats, signed-coordinate point structs (the zig-zag path), mixed `{id,name,active}` records, random ASCII strings, booleans, adversarial int shapes (all-negative, repetitive, huge-magnitude), and a random-access scenario (open a received message and read one field). Clearly separated showcases cover structured data: an arithmetic progression that ships as `(base, stride, n)`, a polynomial column, categorical strings, an int set and an int map. The LOGOS codec is measured under each dial individually, plus a `BEST: all knobs` row that tries every dial combination and reports the smallest configuration that still round-trips. Competitor decoders are forced to touch every value, so lazy and zero-copy formats are credited only for the work they actually do.

## Running the benchmarks

The binary is `wirebench`. The supported path is the wrapper script, which logs to `logs/`:

```bash
./scripts/bench/bench-wire-vs-protocols.sh          # pure-Rust competitors only (no toolchain)
./scripts/bench/bench-wire-vs-protocols.sh --arrow  # + Arrow IPC (pure-Rust, still no toolchain)
./scripts/bench/bench-wire-vs-protocols.sh --heavy  # + protobuf + Cap'n Proto (installs protoc/capnp)
```

Or invoke it directly:

```bash
cargo run --release -p logicaffeine-wirebench                       # default (pure-Rust)
cargo run --release -p logicaffeine-wirebench --features arrow-bench
cargo run --release -p logicaffeine-wirebench --features heavy
WIREBENCH_ITERS=50000 cargo run --release -p logicaffeine-wirebench # tune the op count
```

`WIREBENCH_ITERS` sets iterations per measurement (default `20000` in the binary; the script defaults to `30000`). `WIREBENCH_JSON=<path>` also writes the report as JSON, the form `benchmarks/run.sh` records as `benchmarks/results/latest-codec.json` for the web Benchmarks page.

## Feature flags

| Feature | Default | Enables |
|---------|---------|---------|
| `arrow-bench` | off | `dep:arrow` — Arrow IPC, the pure-Rust columnar and zero-copy sibling; needs no external toolchain. |
| `protobuf` | off | `dep:prost`, `dep:prost-build` — the protobuf/gRPC payload codec; `build.rs` runs `protoc` over `schemas/bench.proto`. |
| `capnproto` | off | `dep:capnp`, `dep:capnpc` — Cap'n Proto (zero-copy); `build.rs` runs the `capnp` compiler over `schemas/bench.capnp`. |
| `heavy` | off | `arrow-bench` + `protobuf` + `capnproto` — everything; builds only where `protoc` and `capnp` are on `PATH`. |

With no features, the core run measures against bincode, postcard, MessagePack, CBOR and JSON, all pure Rust. The toolchain-backed competitors are off by default so the core bench builds and runs anywhere, and `build.rs` does nothing unless `protobuf` or `capnproto` is enabled. Under `capnproto`, `bench_capnp` exposes the generated Cap'n Proto readers and builders, so the head-to-head suites in `logicaffeine-tests` race exactly the code the bench runs.

## Dependencies

- **Internal:** `logicaffeine-compile` (the wire codec under test plus the `RuntimeValue` / `ListRepr` / `StructValue` types).
- **Pure-Rust competitors (always on):** `serde`, `bincode`, `postcard`, `rmp-serde` (MessagePack), `ciborium` (CBOR), `serde_json` (JSON).
- **Optional heavyweights:** `arrow` (IPC), `prost` + `prost-build` (protobuf), `capnp` + `capnpc` (Cap'n Proto).

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_wirebench/CHANGELOG.md)
