# logicaffeine-wirebench

A fair, reproducible head-to-head benchmark of the Logos wire codec against industry serializers — measuring encoded size, encode ns/op, and decode ns/op on the same logical data (the Logos value space: int/float arrays, struct lists, records, strings).

Part of the [Logicaffeine](../../NEW_README.md) workspace. It is a thin `[[bin]]` (`wirebench`) over `logicaffeine_compile`'s `concurrency::marshal` codec and `interpreter` value types; it owns no library surface of its own. The crate is pinned to `version = "0.0.0"`, is `publish = false`, and — although it is listed as a workspace member — is deliberately kept **out of `default-members`** so its heavy competitor dependencies never touch the main build, test, or CI flow.

## Role in the workspace

`logicaffeine_compile` carries the peer-messaging wire codec (`message_to_wire_with` / `message_from_wire`) with its size↔speed dials: `WireNumerics` (Varint / Fixed / GroupVarint), `WireStructure` (Off / Affine), `WireFloats` (Memcpy / XorDelta Gorilla), and optional `WireCompression::Zstd`. This crate exists to answer "is that codec actually competitive?" honestly: it builds the identical payload as both a `RuntimeValue` (for our codec) and a plain Rust type (for serde competitors), encodes and decodes both, and prints a per-scenario table of byte size, encode ns/op, decode ns/op, and ratio-vs-JSON.

Payloads are **seeded random** via a deterministic SplitMix64 RNG (reproducible, but not a clean `0..n` sequence that would let varints and the affine hack win for free). Scenarios: random ints, sensor-like f64 floats, signed-coordinate point structs (the zig-zag path), mixed `{id,name,active}` records, random ASCII strings, three adversarial int shapes (all-negative, repetitive, huge-magnitude), and a clearly-separated affine showcase (an arithmetic progression that ships as `(base, stride, n)`). Our codec is measured under each dial individually plus a `BEST: all knobs` row that tries every dial combination and reports the smallest configuration that still round-trips. Competitor decoders are forced to touch every value, so lazy/zero-copy formats are credited only for the work they actually do.

## Running the benchmarks

The binary is `wirebench`. The supported path is the wrapper script, which logs to `logs/`:

```bash
./scripts/bench-wire-vs-protocols.sh          # pure-Rust competitors only (no toolchain)
./scripts/bench-wire-vs-protocols.sh --arrow  # + Arrow IPC (pure-Rust, still no toolchain)
./scripts/bench-wire-vs-protocols.sh --heavy  # + protobuf + Cap'n Proto (installs protoc/capnp)
```

Or invoke it directly:

```bash
cargo run --release -p logicaffeine-wirebench                       # default (pure-Rust)
cargo run --release -p logicaffeine-wirebench --features arrow-bench
cargo run --release -p logicaffeine-wirebench --features heavy
WIREBENCH_ITERS=50000 cargo run --release -p logicaffeine-wirebench # tune the op count
```

`WIREBENCH_ITERS` sets iterations per measurement (default `20000` in the binary; the script defaults to `30000`).

## Feature flags

| Feature | Default | Gates | Notes |
|---------|:-------:|-------|-------|
| `default` | yes | nothing extra | core run vs bincode, postcard, MessagePack, CBOR, JSON — all pure-Rust |
| `arrow-bench` | no | `dep:arrow` | Arrow IPC: pure-Rust columnar/zero-copy sibling, needs no external toolchain |
| `protobuf` | no | `dep:prost`, `dep:prost-build` | protobuf/gRPC payload codec; `build.rs` runs `protoc` over `schemas/bench.proto` |
| `capnproto` | no | `dep:capnp`, `dep:capnpc` | Cap'n Proto (zero-copy); `build.rs` runs the `capnp` compiler over `schemas/bench.capnp` |
| `heavy` | no | `arrow-bench` + `protobuf` + `capnproto` | everything; only builds where `protoc` and `capnp` are on PATH |

The toolchain-backed competitors are off by default so the core bench builds and runs anywhere; `build.rs` is a no-op unless `protobuf`/`capnproto` is enabled.

## Dependencies

- **Internal:** `logicaffeine-compile` (the wire codec under test plus the `RuntimeValue` / `ListRepr` / `StructValue` types).
- **Pure-Rust competitors (always on):** `serde`, `bincode`, `postcard`, `rmp-serde` (MessagePack), `ciborium` (CBOR), `serde_json` (JSON).
- **Optional heavyweights:** `arrow` (IPC), `prost` + `prost-build` (protobuf), `capnp` + `capnpc` (Cap'n Proto).

## License

Business Source License 1.1 — see [LICENSE.md](../../LICENSE.md).

---
[Docs index](../../new_docs/README.md) · [Root README](../../NEW_README.md) · [Changelog](../../CHANGELOG.md)
