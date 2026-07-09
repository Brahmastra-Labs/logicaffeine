# logicaffeine-system

Platform IO and system services for LOGOS — console, clock, filesystem/VFS, and
networking. The effectful counterpart to `logicaffeine_data`: where that crate
holds pure, WASM-safe value types, this crate owns every interaction with the
outside world and gates the heavy dependencies behind features so a lean (or
`wasm32`) build pays only for what it uses.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace. Tier 2 — depends on
logicaffeine_base and logicaffeine_data. Provides the host primitives that
AOT-compiled LOGOS programs lower to.

## Role in the workspace

The runtime library linked into compiled LOGOS programs (via
`logicaffeine-compile`) and shared with the interpreter and the web app. A
program's `Show`, file access, `Spawn`, `Sync`/`Connect`, and persisted-CRDT
verbs all bottom out in these primitives. The same surface compiles on native
and `wasm32` behind platform seams — `get_platform_vfs()` for files, `net::Net`
for networking — so generated code is written once and runs on both. The
networking and distributed story is covered in
[concurrency.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/concurrency.md).

## Public API

Always available (native and `wasm32`):

- `io` — console/stream IO (`show`, `print`, `println`, `eprintln`, `read_line`)
  and the `Showable` trait behind the LOGOS `Show` verb: primitives unquoted,
  collections `[..]`, maps `{..}`, `None` as `nothing`, CRDTs as their logical
  value, `Duration` human-formatted.
- `temporal` — clock-agnostic time arithmetic over injected timestamps:
  `LogosDate` (days since epoch, Hinnant `to_ymd`), `LogosMoment` (nanoseconds),
  `LogosSpan` (months + days kept incommensurable).
- `relay_proto` — the `RelayFrame` WebSocket wire protocol
  (`Subscribe`/`Unsubscribe`/`Publish`/`SubAck`/`Event`), serde + bincode only,
  shared verbatim by the native and browser relay clients.
- `addr` — `multiaddr_to_ws_url`: normalize a libp2p multiaddr (or raw
  `ws://`/`wss://`) to the URL the relay dials. Pure string logic, no libp2p.
- `net` *(when a relay client exists)* — `Net`, the cross-target relay handle the
  interpreter holds: `connect`/`subscribe`/`publish`/`drain` over the native
  `tokio-tungstenite` client or the browser `web-sys` socket. Drained, not
  awaited, so `Sync` stays a sync point.

Native-only (`cfg(not(target_arch = "wasm32"))`, no feature needed):

- `time` — `now()` (ms since epoch), `sleep(ms)`.
- `env` — `get(key)`, `args()`.
- `random` — `randomInt(min, max)`, `randomFloat()` (thread-local RNG).
- `text` — `parseInt`, `parseFloat`, `chr` (camelCase to match codegen builtins).

Feature-gated:

- `file` *(persistence)* — synchronous `read`/`write` returning
  `Result<_, String>`, for callers that do not need the async VFS.
- `fs` *(persistence)* — async `Vfs` trait (read/write/append/exists/remove/
  rename/create_dir_all/list_dir) with `VfsError`/`VfsResult`/`DirEntry`.
  Backends: `NativeVfs` (tokio::fs, sandboxed paths, atomic write-then-rename),
  `UringVfs` (Linux io_uring worker), and on `wasm32` `OpfsVfs`/`WorkerOpfsVfs`/
  `IndexedDbVfs` behind a `WebVfs` enum (OPFS → IndexedDB fallback).
  `get_platform_vfs()` selects the best backend per target.
- `storage` *(persistence)* — `Persistent<T>`: journal-based, crash-resilient
  CRDT storage (mount/get/mutate/compact/entry_count/maybe_compact) over an
  append-only WAL replayed entry-by-entry on mount.
- `relay` *(native, behind the `relay` feature)* — the thin WebSocket relay
  server + client: `serve` (pure browser↔browser hub) and `serve_bridged`
  (cross-forwards the libp2p gossipsub mesh, so a browser dialing a native node
  joins the real mesh). Carries no libp2p into the browser.
- `relay_browser` *(wasm32)* — `RelayBrowserClient`, a `web-sys` WebSocket
  speaking `relay_proto`: the browser's door into a native node's relay.
- `network` *(networking)* — libp2p P2P: `listen`/`connect`/`send`,
  `local_peer_id`, `PeerAgent`, `MeshNode`, GossipSub `gossip_publish`/
  `gossip_subscribe`, mDNS discovery, `NetworkError`; `FileSipper` chunked
  transfer (`FileManifest`/`FileChunk`/`DEFAULT_CHUNK_SIZE`).
- `crdt` *(networking)* — `Synced<T>`, an auto-replicated (ephemeral) CRDT wrapper.
- `concurrency` — `spawn`/`TaskHandle`, Go-like bounded `Pipe` channels,
  `seeded_pick`/`deterministic_replay_enabled` for replayable scheduling.
- `memory` *(concurrency)* — `Zone` arena (heap via bumpalo, or zero-copy mmap),
  "Hotel California" bulk deallocation.
- `distributed` *(networking + persistence)* — `Distributed<T>`, the
  mesh-journal bridge: local mutations go RAM → journal → network and remote
  updates go network → RAM → journal, with auto-compaction at 1000 entries.

### Post-quantum cryptography and runtime kernels

Always available (native and `wasm32`, no feature needed) — the symmetric/PQC
primitives compiled LOGOS crypto lowers to, and validated bit-exact against the
Logos-native implementations:

- `keccak` — Keccak-f\[1600\] + FIPS-202 sponge (`sha3_256`/`sha3_512`/`shake128`/
  `shake256`, multi-block squeeze), the hash/XOF layer everything below rides on.
- `ntt` — the ML-KEM (Kyber) negacyclic NTT kernel: a verified scalar reference
  plus an AVX2 i16×16 path, with the byte-encode/decode, compression, and CBD
  samplers (`mlkem_ntt`/`mlkem_inv_ntt`/`mlkem_base_mul`/`mlkem_byte_encode`/… )
  re-exported at the crate root.
- `mlkem` — ML-KEM-768 (FIPS-203) keygen / encapsulation / decapsulation, the
  post-quantum key exchange for the channel handshake, composed from the NTT +
  Keccak kernels.
- `mldsa` — ML-DSA-65 (FIPS-204 / Dilithium) keygen / sign / verify, the
  post-quantum signature complement to ML-KEM.
- `aead` — ChaCha20-Poly1305 (RFC 8439), the symmetric seal that closes the
  post-quantum channel once the shared secret is established.
- `word_rt` — runtime support for the `Word8`/`Word16`/`Word32`/`Word64` ring
  types in compiled LOGOS (`word32`, `rotl`, and the `Showable` glue), the
  execution-side complement to `logicaffeine_base::word`.

The crate root re-exports the `io`/`temporal` items and the `keccak`/`ntt`
kernels, plus `tokio` (native), and adds `panic_with(reason)` and a `fmt` helper
module.

## Feature flags

Default is `[]` — lean: no networking, no persistence, no parallelism.

| Feature | Pulls in | Adds / implies |
|---------|----------|----------------|
| `relay` | tokio-tungstenite, futures | thin WS relay (`relay`/`relay_browser`/`net`); no libp2p |
| `networking` | libp2p, futures | `network`, `crdt`; implies `relay` |
| `persistence` | memmap2, sha2 | `file`, `fs` (VFS), `storage` |
| `concurrency` | rayon, bumpalo | `concurrency`, `memory` |
| `io-uring` | io-uring, crossbeam-channel | `UringVfs` (Linux only); implies `persistence` |
| `full` | the three below | `networking` + `persistence` + `concurrency` |
| `distributed` | networking + persistence | `Distributed<T>` |

docs.rs documents with `full`.

## Dependencies

Internal: `logicaffeine-base`, `logicaffeine-data`.

Always-on external: serde, bincode, async-trait, once_cell, async-lock,
crc32fast. Native targets also pull tokio, rand, getrandom, uuid; `wasm32`
targets pull wasm-bindgen(-futures), js-sys, web-sys, futures.

Feature-gated external: libp2p (`networking`), tokio-tungstenite + futures
(`relay`), memmap2 + sha2 (`persistence`), rayon + bumpalo (`concurrency`),
io-uring + crossbeam-channel (`io-uring`, Linux).

## License

Business Source License 1.1 — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/CHANGELOG.md)
