# Logos Wire Codec — Report Card

Head-to-head vs **bincode · postcard · MessagePack · CBOR · JSON · Apache Arrow · protobuf ·
Cap'n Proto**. Same logical data every codec sees, **seeded-random** (fair — no cherry-picked
sequences), 20k iters. `size` = encoded bytes (no envelope). `decode` = ns to decode AND touch
every value (the honest measure). "logos BEST" = every knob turned (numerics × structure × floats
× Zstd), smallest that round-trips. Lower is better.

> Reading the BEST row: its **size** is real (the smallest we can ship). Its **encode time is a
> SEARCH cost** (it tries ~24 configs) — in a real `Send` you pick ONE knob and encode once
> (varint ≈ 1.5µs, fixed ≈ 130ns). Compressed modes decode slower (decompress) — that's the
> size↔latency dial, and it's YOUR choice.

---

## Per-type grades

### Integers — random (ids/counts/sizes) — **A**
| | size | decode |
|---|---|---|
| **logos BEST** | **3116 B** (smallest) | — |
| logos fixed | 8005 B | **83 ns** (fastest) |
| protobuf | 3229 B | 2142 ns |
| postcard | 3338 B | 1234 ns |
| capnp | 8024 B | 225 ns |
Smallest *and* fastest-decoding. The dial trades size↔speed: varint 3116 B ↔ fixed 8005 B/83 ns.

### Integers — signed / negative — **A+**
logos **3278 B** vs **protobuf 10003 B** → **3.05× smaller**. protobuf `int64` spends 10 bytes per
negative; we pick zig-zag automatically (the new adaptive sign mode).

### Integers — repetitive (small value set) — **A++ (categorical)**
logos BEST **49 B** vs postcard 7002 · protobuf 6753 · bincode 16008 → **≈138× smaller**. The
Zstd knob; fixed-width codecs cannot shrink at all.

### Integers — huge 64-bit — **A**
logos fixed **8005 B** (ties bincode 8008 / capnp 8024 / arrow 8456), decode **82 ns** (fastest).
varint loses here (9508 B) — the dial picks `fixed`, so we never lose.

### Integers — structured / sequential — **A++ (categorical)**
logos affine **9 B** vs postcard 3002 · json 8001 → **≈333× smaller**. Ship the generating
formula `(base, stride, n)`, not the data. Nobody else does this.

### Floats — random f64 (sensor / mesh) — **A+**
logos BEST **5145 B** (XOR-delta + Zstd) vs bincode 8008 · arrow 8456 · msgpack 9003 → **~36%
smaller**. Decode **81 ns** vs bincode 532 / arrow 989 → **6–12× faster**.

### Structs — Point {x,y} signed — **A**
logos BEST **5198 B** beats postcard 5833, and **3.3× smaller than protobuf (17137 B)**. Decode:
fixed 261 ns vs capnp 375.

### Structs — Record {id,name,active} — **A**
logos BEST **2092 B** = smallest (postcard 2640 · protobuf 3420). Decode **887 ns** vs bincode
5155 / protobuf 7678 → **6–8× faster**.

### Strings — **A+**
logos BEST **2192 B** beats postcard 3387 (**35% smaller**). Decode **318 ns** vs capnp 1380 /
bincode 4166 → **4–13× faster**. The string column also ships **generators**, not just bytes:
- `T_STRINGS_TEMPLATE` — a sequential-id column (`https://…/items/0…999`, `file_0.txt…`, generated
  labels) is `prefix + (base + i·stride) + suffix`; ship the two affixes once + the affine index. A
  1000-URL column goes out in **~40 B vs ~37 KB flat**, reconstructed byte-exact (exact-decimal guard
  refuses zero-padding / `+sign`, so it never mis-fires).
- `T_STRINGS_FRONT` — front-coding for sorted / hierarchical columns the dictionary can't help (all
  distinct) and the template can't (non-affine / zero-padded): each string ships
  `(shared-prefix-len, suffix)`, cut on a UTF-8 boundary. Sorted log paths / object-store keys crush
  **3×+ below flat**; no shared prefix ⇒ `consider` keeps the flat form (never a loss).
- `T_STRINGS_AFFIX` — common **prefix AND suffix** with arbitrary middles: the one case the other
  four miss because the shared part is a *suffix* — emails `…@example.com`, extensions `…​.log`,
  wrapped ids `https://cdn/v2/<x>/asset.json`. Ships both affixes once + each middle (**2×+ below
  flat**).
- plus the existing **dictionary** for low-cardinality categorical labels.
The five string forms (flat · dictionary · front-code · template · affix) are all bake-off
candidates; `consider` ships only the smallest, so the column is never larger than plain `T_STRINGS`.

### Exact numbers (i64 > 2^53, BigInt, 1/3) — **A++ (only us)**
We ship `T_BIGINT` / `T_RATIONAL` losslessly. JSON corrupts any `i64 > 2^53` and cannot represent
`1/3`; protobuf/bincode have no rational; MessagePack rounds. **Nobody else even competes here.**

### Ship the computation, not the data — **A++ (only us)**
Three forms, each a candidate the size-menu only keeps when it wins, so never larger than G5:
- `T_INTS_POLY` — a polynomial column ships its finite-difference **generator** (degree + a few
  seeds). A 10,000-value `3i² − 5i + 7` goes out as **9 bytes vs 39,625 raw — 4402× smaller**,
  reconstructed bit-exact by a difference engine. Any degree ≤ 4.
- `T_INTS_GEOMETRIC` — exponential growth (doubling, powers, compounding) is neither affine nor
  polynomial, so it ships `(base, ratio, n)` — **3 numbers regardless of length**. A 40-value `3·2ⁱ`
  doubling column goes out in **~4 B vs 139 B raw (≈35×)**, and stays bit-exact **even when the
  sequence overflows i64** — the encoder verifies reconstruction by replaying the SAME `wrapping_mul`
  the decoder uses, so the win never costs correctness.
- `T_INTS_PERIODIC` — an arbitrary cyclic column (`pattern[i mod p]` — weekly schedules, repeating
  categories, sawtooth) ships **ONE period block, regardless of n** — and the block is itself run
  through the whole menu (a byte block → raw, an affine block → 3 numbers), so periodic *composes*
  with every other generator. A 1001-value 7-element-block column ships in **~14 B vs 391 B raw
  (≈28×)**; because the descriptor is n-independent, the same 7-value block describes a million rows.
- **Float generators** — the same idea on `f64`, always BIT-exact (`to_bits` verified, so it fires
  only when reconstruction is perfect — never a lossy quantizer): constant, affine (`base + i·stride`
  linspace / integer-valued JSON floats), sparse (one dominant value + outliers), periodic (cyclic
  waveforms), geometric (compounding / exponential decay, replayed by the same accumulation). A
  1000-element constant or linear float column ships in **tens of bytes, not 8 KB**.
- **Bool generators** — the bool column ships its shape too, not just 1 bit/flag: `T_BOOLS_PERIODIC`
  (constant all-true/all-false at p=1, alternating at p=2, weekly flag at p=7 → one tiny period block)
  and `T_BOOLS_RLE` (two big runs or a handful of clustered flips → a few varints). A 1000-flag
  constant column goes out in **~5 B vs 125 packed bytes**; a random column correctly keeps the
  bit-pack. So *every* column kind — int, float, string, bool — now ships a generator when one fits.
- `T_GEN` — a general sandboxed **generator expression** over the index (`a + b·(i mod p)` sawtooth
  auto-detected; total, bounded, hostile-tree-safe).
- `T_FUNC` — a **shipped callable function**: a user's pure single-arg arithmetic function lowers to
  the same sandboxed `GenExpr` and crosses the wire, invoked on a peer that never compiled it (the
  receiver runs only the bounded evaluator, never arbitrary code). The safe form of "ship a function."
- **Acceptance contract (C2 Layer C)** — the receiver's form-validator at the network edge. Before
  evaluating a shipped function it validates the **signature** (must be the bounded sandbox, not an
  ordinary closure) and the **argument range** (`lo ≤ arg ≤ hi`); an out-of-range argument is
  **refused, never silently clamped**. The attack surface is exactly the typed, bounded interface the
  receiver wrote down — nothing more. O(1): two integer comparisons.
protobuf/capnp/arrow/msgpack all ship the n raw values; none ship the computation — and none can
ship it *safely*.

### Robustness — knobs compose, nothing crashes, every data shape round-trips
A **matrix blast** (`wire_matrix_blast_every_knob_combo_composes`) proves every combination of
numerics × structure × floats × compression × integrity × struct-view — **4,896 chained-knob
round-trips over 17 payload shapes** — is canonically identical end to end, so no knob silently
corrupts another. A **spectrum sweep** (`wire_spectrum.rs`) then attacks from the data side: **46
payloads from perfectly ordered (constant / affine / geometric / polynomial / periodic) through
structured (runs / clustered / low-cardinality / shared-prefix) to literally random**, plus the
common workflows people actually transmit (timestamps, status codes, latencies, prices, counters,
geo floats, sensor walks, flags, categorical labels, record tables, id→record maps) — each round-
tripped across **54 dial combinations (≈2,500 round-trips)** AND proven `Auto ≤ plain-varint` on
*every* shape (the menu never loses to doing nothing), with the ordered columns crushing far below
their random peers (the generators provably firing). Cyclic/pathological values return a clean
`Err`, never a stack overflow. Adding a knob is one new matrix dimension; the tag-dispatch decoder
makes every addition purely additive.

---

## GPA: **A+**

**Where we win (measured this run, capnp 1.0.1):**

| Axis | Us | Rival | Margin | How |
|---|---|---|---|---|
| sequential ints | 9 B | postcard 3002 B | **333×** | ship the affine generator `(base,stride,n)` |
| repetitive ints | 49 B | rival 7002 B | **143×** | per-column RLE/dict + Zstd menu |
| polynomial column | 9 B | 39,625 B raw | **4402×** | ship the finite-difference generator (`T_INTS_POLY`) |
| geometric column | ~4 B | 139 B raw | **≈35×** | ship `(base, ratio, n)` — overflow-exact (`T_INTS_GEOMETRIC`) |
| periodic column | ~14 B | 391 B raw | **≈28×** | ship one period block, recursively best-encoded (`T_INTS_PERIODIC`) |
| bit-packed bools | 130 B | 1002 B | **7.7×** | 1 bit/flag |
| methods (low-cardinality str) | 923 B | 4714 B | **5.1×** | dictionary |
| http status | 497 B | 2002 B | **4.0×** | FOR bit-pack |
| sensor walk (f64) | 2609 B | 8002 B | **3.1×** | XOR-delta |
| timestamps | 2056 B | 6002 B | **2.9×** | delta-of-delta |
| signed ints | 3398 B | protobuf 10000 B | **2.9×** | adaptive zig-zag |
| **capnp: primitive open+read-1** | 74 ns | 432 ns | **5.8×** | cheaper unframe than capnp's segment-table parse |
| **capnp: record-list random field** | 240 ns | 467 ns | **1.94× + 33% smaller** | offset-table view (`T_STRUCTS_VIEW`) vs word-aligned records |
| **capnp: LAN round-trip** | 74.4 µs | 107.1 µs | **1.44× + 32% smaller wire** | fewer bytes through the kernel |
| decode (fixed) | ~83 ns | bincode 2× / capnp 225 ns | **2–13×** | memcpy / zero-copy slice |
| exact i64>2⁵³, BigInt, 1/3 | lossless | JSON corrupts, others round | **alone** | `T_BIGINT`/`T_RATIONAL` |

**Random-access guarantees (locked, always-run, not behind the capnp toolchain):** reading one
field of one row of a record-list message does **0 heap allocations** (counting-allocator proof) and
is **O(1) in the row count** — a field read in a **1,000,000-row (28 MB)** message costs **217 ns**
vs **193 ns** in a 1,000-row one (**1.12×**; a linear scan would be ~1000×). Genuine random access,
matched to capnp's signature claim and beaten on size.

**Smallest by default, provably:** `Send smallest`/`best` runs the message-level auto-tuner —
measures the full numerics × structure × float-coding × compression cross-product and ships the
minimum, so the result is **provably ≤ every single knob on every workload** (every single-dial
config is a candidate). The codec is crash-safe on cyclic/pathological values (clean `Err`, never a
stack overflow).

**Gaps we have since closed:**
1. **Zero-copy field access — CLOSED, and we now beat capnp at it.** The **G3 WireView** reads any
   scalar/array in place; **Pillar A** adds an offset-table struct view (`T_STRUCT_VIEW`) for O(1)
   single-field random access and 8-byte-aligned numeric columns (`T_INTS_ALIGNED`/`T_FLOATS_ALIGNED`)
   that `WireView::as_i64_slice`/`as_f64_slice` read as a true zero-copy `&[T]`. The lock-in
   `we_beat_capnp_on_receive_and_read_one_field` measures **67 ns vs capnp 422 ns (6.31×)** on the
   open + read-one-element path — capnp's own "cheap open" axis — while staying 24 B smaller.
2. **Raw varint vs postcard — CLOSED for Logos↔Logos.** **G4 schema-mode** elides the inline schema
   to a 1-byte tag; **Pillar B** type-id (`Send shared`/`known`) elides struct/enum *names* entirely
   (`T_STRUCT_TID`/`T_INDUCTIVE_TID`), so a first-message struct ships `tag + id + values` — strictly
   smaller than postcard with no priming. The self-describing default stays for non-Logos peers.

**Where the honest tradeoffs remain:**
3. **Smallest ≠ fastest at once.** The compressed/smallest modes decode slower (decompress); the
   aligned zero-copy column is fastest but raw-8-byte, not varint-small. The dial makes this an
   explicit, per-message choice — not a hidden loss.

**Also closed:**
4. **Struct-LIST random access — CLOSED, and we beat capnp on it too.** `T_STRUCTS_VIEW` adds a
   per-row offset table plus per-row field-offset tables, so `WireView::structs_row_field(row, name)`
   reaches any *(row, field)* in O(1). The lock-in `we_beat_capnp_on_record_list_random_field_access`
   measures **225 ns vs capnp 444 ns (1.97×)** on open + read-one-field over a 2000-record list, while
   shipping **53,257 vs 80,048 bytes (34% smaller)** — capnp's word-aligned records bloat where our
   schema-once + offset-table layout stays tight.
5. **End-to-end LAN latency — CLOSED.** A real loopback TCP echo round-trip (length-prefixed send →
   echo → recv → read one field), pre-serialized for both (capnp's no-serialize best case + our
   amortized-snapshot case), isolates the wire path. The lock-in `we_beat_capnp_on_lan_round_trip`
   measures **71.7 µs vs capnp 104.1 µs (1.45×)** on a 20k-record message — driven by **542,858 vs
   800,048 bytes on the wire (32% smaller)** + a cheaper open. Honest scope: this is the
   message-already-built / broadcast-a-snapshot case; serializing fresh per message would add our
   one encode pass (capnp builds in-place), so the wire-path win is what's isolated here.
6. **Build-in-place encode — CLOSED (the last capnp edge).** `marshal::build_columnar_record(from,
   type, &[(name, WireColumn::Ints/Floats)])` writes a record straight into the offset-table
   `T_STRUCT_VIEW` + aligned-column layout **from borrowed slices** — no intermediate `RuntimeValue`,
   no materialize-then-serialize. The output is **byte-identical** to the audited struct-view encode
   (so it inherits its correctness), reads back O(1) zero-copy via `view_message().struct_field().
   as_i64_slice()`, and measures **1.33× faster** than serializing the equivalent pre-built value
   (8×256 i64 record: 219 ms vs 292 ms / 4000 iters — it skips the field sort, HashMap walk,
   per-field dispatch, and compression probe). That is capnp's "build in the wire buffer" on Logos
   terms — paired with the read side, the dual zero-encode/zero-decode story is complete, while we
   stay name-elided and 24–34 % smaller. Tests: `build_in_place_*` in `concurrency::marshal::tests`.
7. **mmap / shared-memory / inter-process — CLOSED (capnp's flagship).** The columnar layout is
   **position-independent** (`T_STRUCT_VIEW` uses byte offsets relative to the values block, never
   absolute pointers), so the bytes read in place from ANY backing store at any base: a memory-mapped
   file, a shared-memory segment, a network buffer — no relocation/fixup. `columnar_record_mmaps_a_
   column_zero_copy_from_disk` writes a 50 k-row record to disk, `memmap2::Mmap::map`s it, and reads a
   column as `&[i64]`/`&[f64]` **aliasing the mapped pages** (asserted: the slice pointer lies inside
   the mapping — not a decoded heap copy); the OS pages in only what's touched. `columnar_record_is_
   position_independent_mmap_and_ipc_ready` locks the same zero-copy read after relocating the message
   to a fresh base (the IPC/shared-segment case), correct at any base, zero-copy at any aligned one.
   That is capnp's "mmap the file / share across processes, read in place" — matched, and smaller.
8. **NO DECODE IN PRODUCTION — CLOSED (capnp's actual home).** A running Logos program that receives
   a record list now reads it IN PLACE, no full decode. `Await view from <peer> into rows` holds the
   received frame as raw bytes (`ListRepr::WireStructs`) and decodes a cell only when a field is
   touched (`rows`' fields resolve O(1) through `WireView::structs_row_field`); an untouched row/field
   is never decoded. The drain path defers self-describing record lists (`peek_record_list_sender`)
   while still decoding cached/compressed/scalar messages eagerly in arrival order (schema-cache
   ordering preserved); plain `Await` stays eager and byte-for-byte unchanged. It covers BOTH record
   lists (`ListRepr::WireStructs`) AND aligned numeric columns (`ListRepr::WireColumn` — a received
   `Seq of Int`/`Seq of Float` read straight out of the borrowed `&[i64]`/`&[f64]`, capnp's
   `List<i64>` read-in-place). PROVEN LIVE: `interp_await_view_zerocopy` runs a real interpreter over
   a loopback relay — a peer publishes a record list, the program `Await view`s it and reads
   `first's score` → `11`, no decode. Tests: `wireview_decode_and_schema`, `lazy_wirestructs_*`,
   `lazy_wirecolumn_*`, `message_from_wire_view_*`, `production_receive_path_defers_*`
   (`concurrency::marshal::tests`) + the live e2e; 92/92 concurrency e2e + 796/796 compile-lib green.

9. **Streaming — CLOSED (capnp's streaming/incremental-reads home).** `concurrency::stream`:
   `frame_for_stream` (LEB128-length-delimited) + `StreamDeframer` process a byte STREAM of messages
   INCREMENTALLY as bytes arrive — any chunking (byte-at-a-time, frame split mid-body OR mid-length-
   varint, many frames per chunk) — and hand back each complete frame ZERO-COPY (a borrowed body
   slice) the instant it is buffered, never holding more than one partial frame. Paired with
   `view_message`, a streamed message is read IN PLACE with no per-frame decode and no whole-stream
   buffering — the gRPC/Arrow-Flight/Kafka framing every streaming protocol is built on. Lock-in
   `we_stream_with_protobuf_framing_constant_memory_and_zero_copy` (superiority.rs) streams 10k
   messages and asserts: framing byte-identical to protobuf length-delimited (LEB128 prefix),
   **constant memory** (peak buffered < 1% of stream length — never the whole stream), and every
   message read zero-copy. The LANGUAGE surface is live too: `Stream <list> to <peer>` batches a
   list into one framed message and `Await stream from <peer> into <var>` deframes it back —
   Kafka-style batch streaming proven end-to-end through the running interpreter over a loopback
   relay (`interp_stream_surface.rs`, both send + receive). The TRANSPORT is a one-word knob (like
   `Send`): `Stream <list> to <peer>` = the relay/NETWORK pro (tree-walker async tier); `Stream
   <list> into <pipe>` = an in-process channel send that's CROSS-TIER (lowers to `SendPipe` → runs
   identically on the bytecode VM, AOT, and tree-walker — `diff_stream_into_pipe_is_cross_tier`
   proves VM≡tree-walker). The knob exposes the full union of pros; network I/O is un-bytecodeable,
   so no single execution has both — you pick. 5/5 `concurrency::stream::tests` + the superiority
   lock-in + 3 e2e/differential + framing round-trip green.

10. **Capability handshake — CLOSED (zero-config "beat raw varint by default").** A peer's
    acceptance profile — receive limits, type-registry epoch, supported feature flags
    (`deflate`/`lz4`/`zstd`/`type-id`/`computed`/`fec`) — rides a dedicated `<peer>#hs` sub-topic on
    first contact, so a raw consumer on the data topic is unaffected (that separation is why the
    byte-identical data path is untouched). `negotiate(mine, theirs)` is **conservative — a knob is
    used only if BOTH ends expose it** — so the sender automatically stays inside exactly what the
    receiver can decode, and **type-id name elision turns on by itself** when the two epochs match
    (two peers running the same program), with a safe self-describing fallback otherwise. The
    receiver also gets an **admission contract**: a `DecodeDepthGuard` (stack-smash DoS), a bounded
    generator-expansion count (a 12-byte `T_GEN`/affine descriptor can't inflate to gigabytes), a
    max-bytes ceiling, and an explicit `accept_computed` gate (an executable `T_FUNC` is refused at
    decode unless the receiver opted in — the first of the three gates, with the C2 contract gating
    invocation and the sandbox bounding the call). PROVEN on BOTH tiers byte-identically:
    `typeid_name_elision_fires_end_to_end_on_both_tiers` (a struct's type/field names vanish from the
    wire once the peers handshake) + `program_advertises_its_profile_on_first_contact_on_both_tiers`,
    over a live loopback relay through the running interpreter AND the bytecode VM.

11. **Rc-dedup of shared subtrees — CLOSED (the all-types-completeness tail).** A subtree the SAME
    `Rc` reaches more than once (a shared lookup table referenced by N records, one big object aliased
    across a structure) ships ONCE — `T_SHARED_DEF id + value` on first sight, a tiny `T_SHARED_REF id`
    on every repeat — and the **decoder rebuilds the SHARING**: the decoded copies alias ONE `Rc`
    (proven by `Rc::ptr_eq`), not N heap copies. capnp/protobuf/bincode/postcard all explode a shared
    subtree into N independent copies; we ship the reference graph itself. Opt-in (`with_dedup`), so
    the default wire is byte-unchanged — and a value with NO sharing is byte-identical even with the
    knob on (no def/ref tag is emitted). Cycle-safe (the gather descends a pointer only on its first
    sighting) and panic-safe under every truncation / single-byte mutation. Complementary to the G5
    string **dictionary** (which already dedups a homogeneous string column); Rc-dedup owns the shared
    *containers* and shared values spread through heterogeneous structure. Tests: `wire_dedup.rs`.

## How you turn the knobs (shipped)
One word on `Send`, all shipped: `fast` (memcpy / fixed-width) · `compact` (varint) · `packed`
(group-varint, SIMD-decode) · **`smallest`/`best`** (the provably-minimal auto-tuner) · `shared`/`known`
(type-id name elision, Logos↔Logos) · `cached` (schema dictionary) · `compressed [with zstd|lz4|deflate]`
· `redundant` (FEC, reconstruct from K-of-N shards) · `computed` (ship the function, not the data) ·
`unchecked` (drop the checksum). The wire is self-describing by tag, so any peer decodes regardless of
which knobs the sender turned.

## Win even harder (next levers)
1. **Language `Send deduped` surface + cyclic-graph decode.** Rc-dedup ships as a codec knob today
   (`with_dedup`); the one-word `Send deduped` surface + a pre-order-shell decode that rebuilds true
   CYCLES (not just DAGs) finish the structure-sharing story.
2. **Word-level BV / SIMD on the wide-integer path.** Group-varint already SIMD-decodes; extend the
   shuffle path and the FOR bit-packer to AVX-512 for the wide-column decode floor.
3. **`computed` over multi-arg + the rational/float sandbox.** The `GenExpr` sandbox is single-arg
   integer today; widening it to multiple args and exact rationals lets more real functions ship as
   computation (and the acceptance contract already has the typed-bounds machinery to gate them).
4. **`Send best` cost amortization.** The auto-tuner pays N encode passes for the true minimum; a
   cheap cost-model pre-filter (skip dominated candidates by a size estimate) would make `best` the
   effortless default even on the hot path.
