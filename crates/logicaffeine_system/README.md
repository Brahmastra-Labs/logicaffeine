# logicaffeine-system

The runtime library where LOGOS meets the outside world: console and clock, files and sockets, HTTP serving, peer-to-peer networking, persisted CRDTs, and the post-quantum cryptography primitives. Where `logicaffeine_data` keeps values pure and WASM-safe, this crate owns the effects: the console, clocks, files and sockets that a program's `Show`, file read or `Sync` bottoms out in, natively or in a browser. The default build is lean: networking, persistence, parallelism and the HTTP server are Cargo features, off until you ask for them.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace, tier 2. The module list below marks what is native-only, `wasm32`-only, or feature-gated.

## Role in the workspace

This is the runtime library linked into compiled LOGOS programs (via `logicaffeine-compile`) and shared with the interpreter and the web app. A program's `Show`, file access, `Spawn`, `Sync`/`Connect`, and persisted-CRDT verbs all bottom out in these primitives. The same surface compiles on native and `wasm32` behind platform seams (`get_platform_vfs()` for files, `net::Net` for networking), so generated code is written once and runs on both. [concurrency.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/concurrency.md) covers networking and distributed state.

## Public API

Always available (native and `wasm32`):

- `failure`: where a runtime failure ends. `caught(f)` runs `f` as a boundary that catches a panic it raises — a served request answering `500`, a replay task failing alone — and `is_caught()` tells a generated program's failure hook whether a boundary will catch this one or the program ends with exit status 1. Every place the runtime catches a panic goes through it.
- `io`: console and stream IO (`show`, `print`, `println`, `eprintln`, `read_line`) and the `Showable` trait behind the LOGOS `Show` verb. Primitives print unquoted, collections as `[..]`, maps as `{..}`, `None` as `nothing`, CRDTs as their logical value, and a `Duration` human-formatted.
- `obs`: the observation channel for compiled (AOT-Rust) programs. In the observed profile, `Note`/`Warn`/`Record`/`During` emit events into a thread-local buffer that a host sink drains, separate from program output. It is a no-op when no sink is installed, and the elided release profile never calls it.
- `temporal`: the temporal values of `logicaffeine_data::temporal` re-exported (`LogosDate`, days since epoch with Hinnant `to_ymd`; `LogosMoment`, nanoseconds; `LogosTime`; `LogosSpan`, months and days kept incommensurable), beside `today()` and `now()`, the day and the instant the clock reads (the ones `LOGOS_FIXED_DAYS` and `LOGOS_FIXED_NANOS` pin, when the environment pins them), and the `Showable` printing of each.
- `testworld`: the sealed test world behind `largo test`, a thread-local that intercepts the IO natives with simulated resources (a virtual clock frozen at a synthetic epoch, an in-memory filesystem, an empty environment, a per-test seeded RNG). A test is a pure function of `(program, seed)`, and capability grants opt individual resources back to reality under `--live`.
- `csrf`: the page half of CSRF protection. `with_csrf_meta` puts a visitor's token in a page document's head as `<meta name="csrf-token">` (`CSRF_META_NAME`), where the browser host reads it and sends it back as `x-csrf-token` on the page's state-changing requests. The dev server, a compiled server and the in-process host share this one definition.
- `relay_proto`: the `RelayFrame` WebSocket wire protocol (`Subscribe`/`Unsubscribe`/`Publish`/`SubAck`/`Event`), serde and bincode only, shared verbatim by the native and browser relay clients.
- `addr`: `multiaddr_to_ws_url` normalizes a libp2p multiaddr (or a raw `ws://`/`wss://` address) to the URL the relay dials. Pure string logic, no libp2p.
- `time` and `random`: `now()` (ms since epoch; `js_sys::Date` on wasm32) and `sleep(ms)` (a no-op on wasm32, which cannot block its thread); `randomInt(min, max)` and `randomFloat()` (a thread-local RNG, seeded from `getrandom`'s `js` backend on wasm32). Every execution tier's `now`/`randomInt`/`randomFloat` natives route through these two, and both honour the sealed test world.
- `net` *(wherever a relay client exists)*: `Net`, the cross-target relay handle the interpreter holds, with `connect`/`subscribe`/`publish`/`drain` over the native `tokio-tungstenite` client or the browser `web-sys` socket. Messages are drained, not awaited, so `Sync` stays a sync point.

Native-only (`cfg(not(target_arch = "wasm32"))`, no feature needed):

- `env`: `get(key)`, `args()`.
- `stack`: `run_main(main)`, the stack a compiled program's `Main` runs on: `MAIN_STACK_BYTES` (64 MiB) mapped between two guard pages and switched to on the thread that started the program, so deep recursion has its room and the process stays single-threaded (a second thread, even one only waiting, puts every `malloc` and `free` on the C library's locked path). A run past the end of the stack writes out what the program had shown, reports `Stack overflow: maximum call depth exceeded`, the sentence every tier reports for it, and ends with exit status 1. A target the switch does not support runs `Main` on a thread of its own.
- `text`: `parseInt`, `parseFloat`, `chr` (camelCase to match the codegen builtins).
- `tcp`: the synchronous TCP client natives behind the stdlib `tcp.lg` (`tcpConnect`/`tcpSend`/`tcpRecvExact`/`tcpListen`/`tcpAccept`, and the rest) that the AOT execution tier calls through `map_native_function`, plus one shared set of error-message formatters every tier reuses so their `Err` text cannot drift. TLS rides the same handles: `connect_tls` opens a rustls client connection verified against the `webpki-roots` store, `upgrade_tls` upgrades an open connection in place (STARTTLS), and `LOGOS_TLS_EXTRA_ROOT` names a PEM file of extra trusted roots.

Feature-gated:

- `file` *(persistence)*: synchronous `read`/`write` returning `Result<_, String>`, for callers that do not need the async VFS.
- `fs` *(persistence)*: the async `Vfs` trait (read/write/append/exists/remove/rename/create_dir_all/list_dir) with `VfsError`/`VfsResult`/`DirEntry`. Backends: `NativeVfs` (tokio::fs, sandboxed paths, atomic write-then-rename), `UringVfs` (a Linux io_uring worker), and on `wasm32` `OpfsVfs`/`WorkerOpfsVfs`/`IndexedDbVfs` behind a `WebVfs` enum (OPFS with an IndexedDB fallback). `get_platform_vfs()` selects the best backend per target.
- `storage` *(persistence)*: `Persistent<T>`, journal-based, crash-resilient CRDT storage (mount/get/mutate/compact/entry_count/maybe_compact) over an append-only WAL replayed entry by entry on mount.
- `relay` *(relay, native)*: the thin WebSocket relay server and client. `serve` is a pure browser-to-browser hub, and `serve_bridged` cross-forwards the libp2p gossipsub mesh, so a browser dialing a native node joins the real mesh. No libp2p reaches the browser.
- `relay_browser` *(wasm32)*: `RelayBrowserClient`, a `web-sys` WebSocket speaking `relay_proto`, the browser's door into a native node's relay.
- `relay::RelayHub` *(relay)*: the relay without a listener of its own, for embedding in another server's accept loop. Its `upgrades()` table registers `/_relay`, the path every LOGOS page dials for `Publish`, message handlers and `shared on` stores, on `largo serve` and on a compiled backend's `Serve`.
- `http` *(http, native)*: the HTTP/1.1 server a LOGOS `Serve` statement runs on; the program does the routing, and this crate serves the connections.
  - `wire` is a hand-rolled parser and writer (no hyper) with the client-side mirror.
  - `server` is the tokio accept loop (`serve`, `spawn`, `serve_with_upgrades`, `serve_with_statics`), including RFC 6455 upgrade dispatch (`try_upgrade`).
  - `engine` picks between that loop and a Linux thread-per-core io_uring engine (`engine_uring`, behind `io-uring`) by `LOGOS_HTTP_ENGINE` (shards: `LOGOS_HTTP_SHARDS`). It defaults to io_uring wherever a ring initializes and announces the choice on stderr.
  - `lifecycle` is the production state machine the compiled server and `largo serve` share. It provides graceful drain, readiness separate from liveness (`LOGOS_READY_PATH`/`LOGOS_HEALTH_PATH`), bounded backpressure that sheds with `503` (`LOGOS_MAX_INFLIGHT`), a per-request time budget (`LOGOS_REQUEST_TIMEOUT_MS`), body and read-deadline caps, and a correlation id on every response (`X-Correlation-Id`). Setting a lifecycle knob the io_uring engine cannot enforce (the request budget, the in-flight cap, a probe path, or `LOGOS_CORRELATION_ID`) serves on tokio instead, and says so.
- `network` *(networking)*: libp2p peer-to-peer networking with `listen`/`connect`/`send`, `local_peer_id`, `PeerAgent`, `MeshNode`, GossipSub `gossip_publish`/`gossip_subscribe`, mDNS discovery and `NetworkError`, plus `FileSipper` chunked transfer (`FileManifest`/`FileChunk`/`DEFAULT_CHUNK_SIZE`).
- `crdt` *(networking)*: `Synced<T>`, an auto-replicated (ephemeral) CRDT wrapper.
- `concurrency` *(concurrency)*: `spawn`/`TaskHandle`, Go-like bounded `Pipe` channels, and `seeded_pick`/`deterministic_replay_enabled` for replayable scheduling.
- `memory` *(concurrency)*: `Zone`, region-based allocation with bulk deallocation. A zone is a bumpalo heap arena or, with `persistence` also on, a read-only memory-mapped file; values allocated in a zone borrow from it and cannot outlive it, so the whole region frees at once.
- `replay` *(concurrency)*: the executor a deterministic Rust build (`compile_to_rust_deterministic`) runs its tasks on: `run`/`run_seeded`, `spawn`/`Task`, `Pipe` (`open`/`send`/`recv`/`try_send`/`try_recv`/`close`), `sleep`, and `select`/`ready_arms`/`Arm`. It is a port of `logicaffeine_runtime`'s scheduler, since a compiled program never links the runtime, and it draws every choice from one seeded chooser, so a binary run at `LOGOS_SEED=s` prints what the interpreter prints at seed `s`.
- `distributed` *(networking + persistence)*: `Distributed<T>`, the bridge between the mesh and the journal. Local mutations go RAM → journal → network, remote updates go network → RAM → journal, and the journal compacts itself once it passes a fixed entry count.

### Post-quantum cryptography and runtime primitives

Always available (native and `wasm32`, no feature needed). These are the native routines behind `crypto.lg`'s `## To native` functions (ML-KEM's NTT, sampling and serialization; the ML-KEM and ML-DSA primitives), which every execution tier reaches through `logicaffeine_compile`'s native-function table (`semantics::native_kernels`). They are also the primitives of the post-quantum channel handshake:

- `keccak`: Keccak-f\[1600\] and the FIPS-202 sponge (`sha3_256`/`sha3_512`/`shake128`/`shake256`, multi-block squeeze), the hash and XOF layer everything below rides on.
- `ntt`: the ML-KEM (Kyber) negacyclic NTT, a verified scalar reference plus an AVX2 i16×16 path, with the byte-encode/decode, compression and CBD samplers (`mlkem_ntt`/`mlkem_inv_ntt`/`mlkem_base_mul`/`mlkem_byte_encode` and the rest) re-exported at the crate root.
- `mlkem`: ML-KEM-768 (FIPS-203) keygen, encapsulation and decapsulation, the post-quantum key exchange for the channel handshake, composed from the NTT and Keccak primitives. `encaps` and `decaps` apply FIPS 203 §7's input checks (the encapsulation key's length and modulus check, the ciphertext's and decapsulation key's lengths, the decapsulation key's hash check) and return `KemError` for whatever fails one; decapsulation always derives both K′ and K̄ and chooses between them by a mask over a full-length compare.
- `mldsa`: ML-DSA-65 (FIPS-204, Dilithium) keygen, sign and verify, the post-quantum signature complement to ML-KEM. `sign` returns `SignError` where FIPS 204 returns ⊥: a context longer than 255 bytes, a secret key of the wrong length, or one with an s₁ or s₂ coefficient outside [−η, η]. `sign_randomized` is the hedged variant, and `verify` answers false for every malformed input.
- `aead`: ChaCha20-Poly1305 (RFC 8439), the symmetric seal that closes the post-quantum channel once the shared secret is established.
- `word_rt`: runtime support for the `Word8`/`Word16`/`Word32`/`Word64` ring types in compiled LOGOS (`word32`, `rotl`, and the `Showable` glue), the execution-side complement to `logicaffeine_base::word`.

Every LOGOS-facing function among them answers `Result<_, String>`. It refuses an argument that breaks its contract (an item that is not a byte, a seed or key of the wrong length, a coefficient outside its range, a width outside FIPS 203's) with the reason, which every tier reports after the kernel's name, and never folds the argument into one it was not given. The arithmetic on secret coefficients divides by no modulus: Barrett multiplies and masked corrections stand in for `/` and `%`, so no target compiles a variable-time divide into it.

The crate root re-exports the `io`/`temporal` items and the `keccak`/`ntt` functions, plus `tokio` and `tokio_rustls` (native), and adds `panic_with(reason)` and a `fmt` helper module.

## Feature flags

The default feature set is empty: no networking, no persistence, no parallelism.

| Feature | Default | Enables |
|---------|---------|---------|
| `relay` | off | The thin WebSocket relay (`relay`, and `net` on native) via tokio-tungstenite and futures; no libp2p |
| `http` | off | The native HTTP/1.1 `Serve` server (`http`) over tokio's `net`; no hyper |
| `networking` | off | libp2p peer-to-peer networking (`network`, `crdt`); implies `relay` |
| `persistence` | off | `file`, `fs` (VFS) and `storage`, pulling in memmap2 and sha2 |
| `concurrency` | off | `concurrency`, `memory` and `replay`, pulling in bumpalo |
| `io-uring` | off | `UringVfs` and the io_uring HTTP engine (Linux only) via io-uring, crossbeam-channel and libc; implies `persistence` |
| `full` | off | `networking` + `persistence` + `concurrency` + `http` (`io-uring` stays opt-in) |
| `distributed` | off | `Distributed<T>`, as `networking` + `persistence` |

docs.rs documents with `full`.

## Dependencies

Internal: `logicaffeine-base`, `logicaffeine-data`.

Always-on external: indexmap, serde, bincode, async-trait, once_cell, async-lock, crc32fast. Native targets also pull tokio, rustls, webpki-roots, tokio-rustls, rand, getrandom and uuid; unix targets pull psm and libc (`stack`); `wasm32` targets pull wasm-bindgen(-futures), js-sys, web-sys, futures, rand, and getrandom (`js`).

Feature-gated external: libp2p (`networking`), tokio-tungstenite and futures (`relay`), memmap2 and sha2 (`persistence`), bumpalo (`concurrency`), io-uring and crossbeam-channel (`io-uring`, Linux).

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_system/CHANGELOG.md)
