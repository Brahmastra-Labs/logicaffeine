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

### Strings — **A**
logos BEST **2192 B** beats postcard 3387 (**35% smaller**). Decode **318 ns** vs capnp 1380 /
bincode 4166 → **4–13× faster**.

### Exact numbers (i64 > 2^53, BigInt, 1/3) — **A++ (only us)**
We ship `T_BIGINT` / `T_RATIONAL` losslessly. JSON corrupts any `i64 > 2^53` and cannot represent
`1/3`; protobuf/bincode have no rational; MessagePack rounds. **Nobody else even competes here.**

### Ship the computation, not the data — **A++ (only us)**
Three forms, each a candidate the size-menu only keeps when it wins, so never larger than G5:
- `T_INTS_POLY` — a polynomial column ships its finite-difference **generator** (degree + a few
  seeds). A 10,000-value `3i² − 5i + 7` goes out as **9 bytes vs 39,625 raw — 4402× smaller**,
  reconstructed bit-exact by a difference engine. Any degree ≤ 4.
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

### Robustness — knobs compose, nothing crashes
A **matrix blast** (`wire_matrix_blast_every_knob_combo_composes`) proves every combination of
numerics × structure × floats × compression × integrity × struct-view — **4,896 chained-knob
round-trips over 17 payload shapes** — is canonically identical end to end, so no knob silently
corrupts another. Cyclic/pathological values return a clean `Err`, never a stack overflow. Adding a
knob is one new matrix dimension; the tag-dispatch decoder makes every addition purely additive.

---

## GPA: **A+**

**Where we win (measured this run, capnp 1.0.1):**

| Axis | Us | Rival | Margin | How |
|---|---|---|---|---|
| sequential ints | 9 B | postcard 3002 B | **333×** | ship the affine generator `(base,stride,n)` |
| repetitive ints | 49 B | rival 7002 B | **143×** | per-column RLE/dict + Zstd menu |
| polynomial column | 9 B | 39,625 B raw | **4402×** | ship the finite-difference generator (`T_INTS_POLY`) |
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
   ordering preserved); plain `Await` stays eager and byte-for-byte unchanged. So zero-copy is no
   longer test-only — it's the live receive. Tests: `wireview_decode_and_schema`, `lazy_wirestructs_*`,
   `message_from_wire_view_*`, `production_receive_path_defers_then_reads_record_list_lazily`
   (`concurrency::marshal::tests`); 91/91 concurrency e2e + 795/795 compile-lib green (no regression).

## How you turn the knobs (shipped)
One word on `Send`, all shipped: `fast` (memcpy / fixed-width) · `compact` (varint) · `packed`
(group-varint, SIMD-decode) · **`smallest`/`best`** (the provably-minimal auto-tuner) · `shared`/`known`
(type-id name elision, Logos↔Logos) · `cached` (schema dictionary) · `compressed [with zstd|lz4|deflate]`
· `redundant` (FEC, reconstruct from K-of-N shards) · `computed` (ship the function, not the data) ·
`unchecked` (drop the checksum). The wire is self-describing by tag, so any peer decodes regardless of
which knobs the sender turned.

## Win even harder (next levers)
1. **Capability handshake (D2).** On `Connect`/`Listen`, peers advertise their type-registry epoch +
   supported codecs, so `shared` name-elision and `computed` can be turned on **automatically** when
   both ends agree — and fall back safely when they don't. Turns "beat raw varint by default" into a
   zero-config default.
2. **Word-level BV / SIMD on the wide-integer path.** Group-varint already SIMD-decodes; extend the
   shuffle path and the FOR bit-packer to AVX-512 for the wide-column decode floor.
3. **`computed` over multi-arg + the rational/float sandbox.** The `GenExpr` sandbox is single-arg
   integer today; widening it to multiple args and exact rationals lets more real functions ship as
   computation (and the acceptance contract already has the typed-bounds machinery to gate them).
4. **`Send best` cost amortization.** The auto-tuner pays N encode passes for the true minimum; a
   cheap cost-model pre-filter (skip dominated candidates by a size estimate) would make `best` the
   effortless default even on the hot path.
5. **Rc-dedup of shared subtrees (G8 tail).** Pointer-identity backrefs (`T_REF`) so a value that
   appears N times ships once — the last "all-types completeness" gap.
