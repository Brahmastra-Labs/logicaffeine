# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- `temporal::today()` and `temporal::now()`: the day and the instant the clock reads, the ones `LOGOS_FIXED_DAYS` and
  `LOGOS_FIXED_NANOS` pin when the environment pins them. `Showable` for `LogosDuration` and the crate-root
  re-export of it.

### Changed

- `temporal::current_moment()` is `temporal::now()`, which honors `LOGOS_FIXED_NANOS`. A duration shows, ticks and
  times out as `LogosDuration`, whose display is signed; `Showable`, `replay::Ticks` and `replay::TimeoutTicks` for
  `std::time::Duration` are gone. Migration: call `temporal::now()`; hold a duration as `LogosDuration`.

## [0.11.0] - 2026-11-02

### Added

- `file::read_reals(path)`: a file of little-endian binary64 values as `Vec<f64>`, eight bytes a value in order,
  refused by name when it cannot be read or is not a whole number of values — the one read behind `readReals` on the
  in-process tiers and in generated Rust.
- `Showable` for `LogosOpt<T>`: the value it holds, or `nothing`.
- `mldsa::sign_randomized` (FIPS 204's hedged signing), `mldsa::SignError`, `mlkem::KemError` and
  `aead::MAX_PLAINTEXT`.
- `aead::hchacha20`, `aead::xchacha20poly1305_seal` and `_open` (draft-irtf-cfrg-xchacha-03), and the box:
  `aead::seal_box` (a nonce drawn from the system's entropy), `seal_box_under`, `open_box`, `BOX_BYTES_LABEL`,
  `BOX_VALUE_LABEL` and `BOX_OVERHEAD`. `random::fill_from_entropy`, which no sealed test world decides.

- **`stack`: the stack a compiled program's `Main` runs on.** `run_main(main)` maps `MAIN_STACK_BYTES` (64 MiB) between two guard pages and switches to it on the calling thread (`psm`), so a program keeps its deep stack without a second thread. A fault in a guard page writes out what was shown (`io` flushes only a buffer no one holds), reports `failure::CALL_DEPTH_EXCEEDED` and exits with status 1; any other fault goes to the handler that was installed before. A native target the switch does not support runs `main` on a thread of its own, and WebAssembly runs it in place.
- `failure`: `caught(f)` runs `f` as a boundary that catches a panic it raises, and `is_caught()` tells a generated program's failure hook whether one will (never under `panic = "abort"`). The HTTP engines and the replay scheduler catch through it.
- **A native HTTP/1.1 server behind the new `http` feature, with a production lifecycle.** The server a LOGOS `Serve` statement runs on: raw tokio and a hand-rolled wire core, no hyper.
  - **`http::wire`:** the dependency-free parser and writer (`parse_request`, `serialize_response`, and the client-side `serialize_request`/`parse_response`), with keep-alive and pipelining. Bodies are framed by `Content-Length` only; `Transfer-Encoding: chunked` is refused (`ParseError::UnsupportedTransferEncoding`). Bodies are capped at 8 MiB (`MAX_BODY`) and header blocks at 64 KiB (`MAX_HEADERS`). No `Date` header is sent, so a response depends only on its handler. `response_header_allowed` and `safe_content_type` strip hop-by-hop headers and reject CR, LF and NUL.
  - **`http::server`:** `Listener`/`Conn` for the interpreter's `!Send` accept loop, `serve`, `serve_with_statics` (one shared handler whose calls are not serialized), `serve_here`, and `spawn`/`spawn_with_lifecycle` returning an `HttpServer` (`set_ready`, `drain`, `lifecycle`). Every tier answers with the same `bad_request` (400), `payload_too_large` (413) and `internal_error` (500); a handler panic answers 500 and the server keeps serving; at most `MAX_INFLIGHT` (64) connections are in flight.
  - **Lifecycle:** `http::lifecycle` holds `Lifecycle`, `Admission`, `DrainOutcome` and the `X-Correlation-Id` header. `LifecycleConfig::from_env` (and its pure `from_env_with` twin) is the one place every serving tier reads `LOGOS_MAX_INFLIGHT`, `LOGOS_REQUEST_TIMEOUT_MS`, `LOGOS_READ_DEADLINE_MS`, `LOGOS_DRAIN_MS`, `LOGOS_MAX_BODY`, `LOGOS_CORRELATION_ID`, `LOGOS_READY_PATH` and `LOGOS_HEALTH_PATH`, so the compiled `Serve` inherits every knob with no change to code generation; an unparseable value is ignored rather than fatal. `LifecycleConfig::max_response_body` answers an oversized response with a defined 500, and `LOGOS_HTTP_ACCESS_LOG` writes one line per request to stderr.
  - **Shutdown and probes:** `http::shutdown_requested()` is a future that resolves on `SIGTERM`/`SIGINT` (Ctrl-C off unix), so the `!Send` interpreted and bytecode loops select on it as the tokio loop does: the server stops advertising readiness, stops accepting, and drains in-flight work to the configured deadline. `Lifecycle::probe_response` answers the opt-in readiness and liveness paths before admission, so a monitor gets a reply during an overload; readiness reports 503 once draining, liveness stays 200. With neither path configured nothing is reserved.
  - **Upgrades:** `serve_with_upgrades` takes a table of protocol-upgrade paths on either engine; `Conn::upgrade` is the one hand-off (`101`, then the socket and the bytes read past the request); `relay::RelayHub::upgrades()` registers the relay hub at `relay::RELAY_PATH`.
  - **Binding:** `http::server::env_port_override` takes `LOGOS_PORT`, then `PORT`, then the program's own port; `env_bind_address` takes `LOGOS_HOST`, defaulting to `127.0.0.1`, with a blank value counted as unset and an IPv6 host bracketed. Every tier announces what it bound through `http::server::announce_serving`: `Serving on http://<addr>` on stdout, flushed, with the real port when `0` was asked for, so whoever started a server on port 0 dials where it listens.
  - Locks: `http_headers_lock`, `http_shared_handler_lock`, `http_lifecycle_lock`, `http_lifecycle_bounds_lock`, `bind_address_lock`; `crates/logicaffeine_tests/tests/http_lifecycle_parity.rs` drives the real binaries.
- **A thread-per-core io_uring HTTP engine.** `http::engine_uring`, behind the `io-uring` feature (Linux only; the feature now also pulls `libc`).
  - **Design:** each shard owns an `SO_REUSEPORT` listener, a ring and a connection table, accepts with multishot accept, and serves pre-serialized static responses (`http::engine::StaticResponses`). Both engines write through one wire module and send identical bytes.
  - **Selection:** `http::engine::choose_engine` picks the engine; `LOGOS_HTTP_ENGINE=uring|tokio` overrides it and `LOGOS_HTTP_SHARDS` sets the shard count. The default is io_uring wherever a probe ring initializes, and the choice is always named on stderr.
  - **Fallback:** a failed bind falls back to tokio with a second line. The engine runs its handler inline on the shard thread, so it cannot enforce a wall-clock request budget, a probe path or admission shedding: when any of those is configured, `choose_engine` falls back to tokio and names the variable that caused it. The caps it does enforce (body size, read deadline) are properties of parsing. With an upgrade table it answers the requests pipelined ahead of an upgrade and hands the socket to the tokio runtime it was bound on.
  - Tests: `http_serve_bench_lock`, `crates/logicaffeine_tests/tests/http_engine_uring.rs`, `crates/logicaffeine_tests/tests/http_engine_fallback.rs`.
- **Sockets with verified TLS.** The `tcp` module (native, no feature) implements `tcp.lg` for the compiled tier: `connect`, `connect_tls`, `upgrade_tls`, `send`, `recv`, `recv_exact`, `close`, `listen`, `accept`, `listener_port`, `shutdown_write` and `set_timeout` over opaque `Int` handles, with one error wording shared by every tier and a 64 MiB receive cap (`MAX_RECV`). Each connection has its own lock, so one stalled peer cannot freeze another. `tcp::tls_config` trusts the Mozilla roots bundled by `webpki-roots` plus a PEM named by `LOGOS_TLS_EXTRA_ROOT`, and `tokio_rustls` is re-exported so the interpreter's sockets share the configuration. Locks: `tcp_tls_lock`, `tcp_registry_concurrency_lock`, `tcp_walker_lock`, `postgres_aot_lock`.
- **`replay` (feature `concurrency`): deterministic replay for compiled concurrent programs.** The executor a program compiled with `logicaffeine_compile::compile_to_rust_deterministic` runs its tasks on: `logicaffeine_runtime`'s scheduler ported step for step, drawing every choice from the same seeded chooser, so such a program run at `LOGOS_SEED=s` prints what the interpreter prints at seed `s`. It provides `run`/`run_seeded`, `spawn`/`Task` (`stop`, `join`), `Pipe<T>` (`open`, `send`, `recv`/`recv_or_closed`, `try_send`, `try_recv`, `close`, `arm`, `received`/`received_or_closed`), `sleep`, `select`/`ready_arms`/`Arm`, and the `Ticks`/`TimeoutTicks` duration readings; `concurrency::seeded_pick` draws from the running replay's chooser while one is active. `replay_executor_port_lock`.
- **`testworld` and `fs::MemVfs`: the sealed world a `## Test` runs in.** A clock frozen at `SEALED_EPOCH_MS` (2020-01-01T00:00:00Z), an in-memory filesystem, an empty environment and a seeded random stream. `time::now`/`sleep`, `env::get`, `file::read`/`write` and `randomInt`/`randomFloat` consult the world before the host; a sealed `sleep` advances the clock instantly, and `randomInt` draws uniformly. `Capabilities` grants one real resource at a time, and `take`/`restore` nest worlds. Locks: `random_int_lock`, `page_host_world_isolation_lock`.
- **`obs`: the observation channel of compiled programs.** `record`, `span_enter` and `span_exit` collect `ObsEvent`s into a thread-local buffer a host opens with `install` and empties with `drain`; with no sink installed each call is a no-op. Locks: `obs_aot_emission_lock`, `obs_zero_cost_codegen_lock`.
- **`csrf` and `http::server::page_with_visitor_token`.** `with_csrf_meta` places a visitor's token in a page head under `CSRF_META_NAME`. `page_with_visitor_token` composes a page for one visitor: the token meta, `Set-Cookie`, `Cache-Control: no-store`, validators stripped. Lock: `csrf_page_token_lock`.
- **Compiled programs print what the interpreter prints.** New `Showable` impls for `Result` (`Ok(v)`/`Err(e)`), `Box<T>`, `LogosComplexF`, `LogosTuple`, `ORSet` (sorted braces), `RGA`/`YATA` (brackets) and `ORMap<K, LatestWrite<T>>` (`{k: v}`); also `io::show_string`, `LogosDate::from_ymd`, `LogosMoment::from_ymd`, `word_rt::int_of_word8` and `file::read_bytes`. Locks: `complex_seq_aot_lock`, `http_binary_parity_lock`.
- **The exact extension's values in compiled programs.** `Showable` for `logicaffeine_data::LogosValue` prints a value of the extension as the interpreter does; `LogosDate`, `LogosMoment`, `LogosTime` and `LogosSpan` implement `logicaffeine_data::exact::OtherValue` (how each prints, compares, orders and combines with the others, `Date + Span` and `Moment − Moment` among them, exactly as the interpreter's values do) and convert into a `LogosValue`, so a date clock beside an infinitesimal compiles.
- **`time` and `random` compile for `wasm32`.** `now` reads `js_sys::Date`, `random` seeds from getrandom's `js` backend, and a live `sleep` is a no-op in the browser; both stay sealed-world aware. They were native-only.

### Breaking changes

- `mldsa::sign` returns `Result<Vec<u8>, SignError>`; `mlkem::encaps` and `decaps` return `Result<_, KemError>`; every
  LOGOS-facing kernel function in `ntt`, `mlkem`, `mldsa`, `keccak` and `aead` returns `Result<_, String>`, refusing an
  argument that breaks its contract (an item that is not a byte, a seed or key of the wrong length, a coefficient out
  of range, a width outside FIPS 203's) with the reason.

- **`LogosDate::weekday` and `LogosMoment::weekday` answer ISO 8601: 1 = Monday … 7 = Sunday.** 0.10.1 answered 0 = Sunday … 6 = Saturday, so only Sunday moves, from 0 to 7; this is the language's `the weekday of` on the tree-walker, the VM, AOT-Rust and C. Migration: `(the weekday of d) % 7` restores the old numbering. `weekday_iso_convention_lock`.
- **The temporal values are `logicaffeine-data`'s**, re-exported here unchanged (`temporal::LogosDate`, `LogosMoment`, `LogosTime`, `LogosSpan` and the crate-root re-exports keep their paths). `LogosMoment::now()` is `temporal::current_moment()`, since a value type in a crate without I/O does not read a clock. Migration: call `logicaffeine_system::temporal::current_moment()`.

### Changed

- Licensed under the MIT License, which replaces the Business Source License 1.1, and the package ships it as `LICENSE.md`.

- **`libc` and `psm` are unix dependencies** of every build, for `stack`; `io-uring` no longer lists `libc` among its own.
- `time::sleep` takes a duration of any integer type (`T: TryInto<u64>`), and a negative one is no wait: LOGOS declares `sleep (ms: Int)`, so a program's `sleep(ms)` compiles on AOT-Rust whatever integer it holds.
- `io` buffers standard output: `show`, `println` and `print` write through one process-wide 64 KiB buffer (a line at a time to a terminal), emptied by `flush_stdout`, by `read_line` and `eprintln` before they touch stdin or stderr, and by an `atexit` handler; `http::server::announce_serving` writes through it and flushes.
- **`full` includes `http`.** Every native capability that is not platform-specific is in `full`; only the Linux-only `io-uring` stays opt-in. A generated project staged with `full` sees `http.lg`'s bindings. Lock: `system_full_feature_lock`.
- **Every native build links TLS.** `rustls` (ring provider only), `webpki-roots` and `tokio-rustls` are required native dependencies; none is linked on `wasm32`. `tokio` gains its `signal` feature.
- **The published crate carries only its source.** An anchored `include` allowlist ships `src/`, the README and the CHANGELOG, so neither `tests/` nor `examples/` is in the tarball; the integration suites moved to `crates/logicaffeine_tests/tests/` and the unit-test bodies to `crates/logicaffeine_tests/unit/logicaffeine_system/`. The declared `rust-version` is 1.88 (0.10.1 declared 1.75).

### Fixed

- `mldsa::sign` refuses a secret key of the wrong length or with an s₁/s₂ coefficient outside [−η, η]; keygen, sign and
  verify reduce each matrix–vector row before the inverse NTT.
- `mlkem::encaps` and `decaps` apply FIPS 203 §7's input checks (lengths, the ek modulus check, the dk hash check);
  `decaps` compares and selects without a branch on the ciphertext.
- No division by q on a secret coefficient: Compress, ByteDecode₁₂, CBD, the NTT outputs and ML-DSA's `freeze` are
  Barrett multiplies and masked corrections.
- `aead::chacha20poly1305_seal` and `open` hold a plaintext to RFC 8439's 2³⁸ − 64 bytes, so the block counter never
  wraps onto the one-time key's keystream.

- **A compiled `Moment` prints like the interpreter's.** `LogosMoment`'s `Display` printed `Moment(<s>s + <ns>ns)`; it prints civil UTC `YYYY-MM-DD HH:MM`, correct before 1970 too. Rust code that parsed the old text must parse the new one. `crates/logicaffeine_tests/tests/parity_date_ctor.rs`.
- **A malformed timestamp or an unknown time zone stops a compiled program in the engines' words.** `LogosMoment::parse_rfc3339`, `in_zone` and `local_instant` panicked with `LOGOS runtime error: …` text of their own; they report `failure::not_a_timestamp` and `failure::unknown_time_zone`.

### Security

- `mldsa::sign` and `verify` refuse a context longer than 255 bytes (FIPS 204); a 256·k + n byte context had been
  encoded as an n-byte one, so a 256-byte context aliased the empty context. Affected 0.10.0 and 0.10.1. Reported by
  Bhabani Sankar Das ([@007bsd](https://github.com/007bsd)) under coordinated disclosure; thank you.

## [0.10.0] - 2026-07-08

Synced to workspace version 0.10.0. See root CHANGELOG for full history.

## [0.8.12] - 2026-02-14

Synced to workspace version 0.8.12. See root CHANGELOG for full history.

## [0.6.0] - 2026-01-17

Initial crates.io release.

### Added

- Platform IO abstraction layer
- Networking support via libp2p (optional `networking` feature)
- Persistence layer with memmap2 (optional `persistence` feature)
- Concurrency via tokio and rayon (optional `concurrency` feature)
- Distributed coordination (optional `distributed` feature)
- Synced<T> wrapper for CRDT network synchronization
- Feature flags: `networking`, `persistence`, `concurrency`, `full`
