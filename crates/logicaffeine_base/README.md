# logicaffeine-base

Pure structural atoms for the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace — arena allocation, string interning, source spans, spanned errors, a union-find — plus the exact numeric and measurement tower (`BigInt`/`Rational`/`Decimal`/`Complex`, machine words, quantities, money, calendars, UUIDs) and the shared MD5/SHA-1 hash oracle. Generic, reusable infrastructure with no knowledge of English vocabulary and no I/O.

Part of the Logicaffeine workspace. Tier 0 — no internal dependencies; everything else builds on it.

## Role in the workspace

This is the bottom of the stack. Every higher crate (`compile`, `language`, `kernel`, `lexicon`, `lsp`, `proof`, `data`, `system`, and the integration `tests` crate) depends on it: bump-allocated AST storage, interned symbols with O(1) equality, byte-offset spans, the `SpannedError`/`Result` error pair, one shared equivalence engine, and the exact types that let a number, a date, or a quantity survive every boundary without collapsing onto an IEEE-754 double. These value types live here — in the leaf crate — so the proof layer, the wire codec, and the tests share one implementation of every number, date, quantity, and identifier. See [architecture.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/architecture.md) for where Tier 0 sits relative to the rest.

## Public API

The crate root re-exports the workhorses: `Arena`; `Interner`/`Symbol`/`SymbolEq`; `Span`; `SpannedError`/`Result`; the numeric tower `BigInt`/`Rational`/`Decimal`/`Complex`/`Modular`/`RoundingMode`; the measurement types `BaseDim`/`Dimension`/`Exp`, `Quantity`/`Unit`, `Currency`/`Money`/`RateTable` (+ the `currency` catalog); `Uuid`/`Variant`; and the machine words `Word8`/`Word16`/`Word32`/`Word64`/`WordVal` with their SIMD lane packs (`Lanes4Word32` … `Lanes16Word8`). Everything else is reached through its module.

```rust
use logicaffeine_base::{Arena, Interner, Span};

let arena: Arena<&str> = Arena::new();
let mut interner = Interner::new();

let hello = interner.intern("hello");
assert_eq!(interner.resolve(hello), "hello");

let span = Span::new(0, 5);
assert_eq!(span.len(), 5);

let allocated = arena.alloc("hello");
assert_eq!(*allocated, "hello");
```

### Structural atoms

**`arena`** — bump allocation over `bumpalo::Bump`; references stay valid across later allocations, so AST nodes can point at each other without reference counting.
- `Arena::<T>::new()` / `Default`
- `alloc(&self, value: T) -> &T`
- `alloc_slice<I: IntoIterator<Item = T>>(&self, items: I) -> &[T]` — `I::IntoIter: ExactSizeIterator` (pre-sizes the allocation)
- `reset(&mut self)` — invalidate references, keep capacity (zero-allocation REPL loops)

**`intern`** — string interning; `Symbol` is a `Copy` `u32` handle, comparison is integer comparison regardless of string length. The empty string is pre-interned at index 0.
- `Interner::new()` / `Default`, `intern(&mut self, s: &str) -> Symbol`
- `resolve(&self, sym: Symbol) -> &str` (panics if `sym` is foreign), `lookup(&self, s: &str) -> Option<Symbol>`
- `len()` / `is_empty()` — `len` counts the empty string; `is_empty` is true when only it is present
- `Symbol::EMPTY` (= `Symbol::default()`), `index() -> usize`, `from_index(usize) -> Symbol` — dense round-trip used by the bounds prover to thread symbols through linear-expression variable ids
- `SymbolEq::is(&self, &Interner, &str) -> bool` — compare a symbol to a literal without an explicit `resolve`

**`span`** — `Span { start: usize, end: usize }`, `Copy` + `Default`, public fields; byte offsets match `&source[span.start..span.end]`.
- `Span::new(start, end)` (no validation; `start` may exceed `end`)
- `merge(self, other: Span) -> Span` (min start, max end), `len() -> usize` (saturating), `is_empty() -> bool` (true when `start >= end`)

**`error`** — `SpannedError { message: String, span: Span }` implements `std::error::Error` and `Display` as `"{message} at {start}..{end}"`.
- `SpannedError::new(message: impl Into<String>, span: Span)`
- `type Result<T> = std::result::Result<T, SpannedError>`

**`union_find`** — `UnionFind` over `usize` ids with path-compressed `find` and `union` by rank (near-constant amortized cost). One equivalence engine under two consumers: the kernel's congruence closure (`logicaffeine_kernel::cc`) and the compiler's equality-saturation e-graph.
- `make_set() -> usize`, `find(x) -> usize`, `union(x, y) -> bool` (true if the classes were distinct), `len()` / `is_empty()` (elements ever created, not live classes)

### Exact numeric tower

**`numeric`** — the tower's foundation, so a number's *type* survives every boundary (interpreter, VM, wire) instead of collapsing onto a double — no 2^53 cliff.
- `BigInt` — arbitrary-precision integer, sign + little-endian base-2^64 limbs (single-limb magnitudes stored inline, no heap for anything that fits 64 bits). `zero/from_i64/from_u64/parse_decimal`, `add/sub/mul/div_rem/pow/negated/abs`, `to_i64/to_f64/is_zero/is_negative`, `to_le_bytes/from_le_bytes`, `From<i64>`, full `Ord`/`Display`/`Debug`.
- `Rational` — exact fraction as a reduced `BigInt` numerator/denominator (`den > 0`, `gcd = 1`). `new/from_bigint/from_i64/from_ratio_i64/zero/one`, `numerator/denominator/is_integer`, `add/sub/mul/div/recip/pow/floor/ceil/round`, `to_bigint/to_i64/to_f64/parse`, `Ord`/`Display`.
- `Decimal` — exact base-10 fixed-point (the anti-float for money and human-entered numbers), with `RoundingMode` (banker's rounding included).
- `Complex` — complex numbers over the exact tower.
- `Modular` — modular arithmetic helpers (used by the proof layer's number-theory modules).

**`word`** — fixed-width wrapping integers, the ring ℤ/2ᵏℤ: `Word8`/`Word16`/`Word32`/`Word64` plus the type-erased `WordVal`. Unlike `BigInt`, arithmetic is total and wrapping (`Word32::MAX.add(Word32::ONE) == Word32::ZERO`) — the natural home of the bit-twiddling crypto substrate (ChaCha20 lives over `Word32`, Keccak over `Word64`). Rotation (`rotl`/`rotr`) is width-defined and lives only here. The lane packs (`Lanes4Word32`, `Lanes8Word32`, `Lanes4Word64`, `Lanes16Word8`, `Lanes16Word16`, `LanesVal`) are the scalar *specification* of the SIMD lanes the AOT tier compiles to AVX2.

**`describe`** — the integer-sequence description-length codec (the MDL primitive): `describe_int_seq` encodes an `&[i64]` as the *shortest* program from a fixed menu of generators (affine, geometric, degree-≤4 polynomial, periodic, sparse, a sandboxed `GenExpr`, and the columnar fallbacks — delta, delta-of-delta, FOR bit-pack, RLE, dictionary, raw, varint); `decode_int_seq` is its exact inverse, so the encoding is a re-checkable *witness* of a computable Kolmogorov-complexity upper bound, and never larger than plain varint. The wire codec (`logicaffeine_compile`'s `marshal`, `WireStructure::Auto`) and the proof layer's AIT certificates share this one implementation. The DoS bound (`max_elements`) is a caller-supplied parameter — no receiver policy here.

### Measurement, time, money, identity

**`dimension`** — physical dimensions as an abelian group of *rational* exponent vectors over the base dimensions (`BaseDim`, exponents `Exp`): `×` adds vectors, `÷` subtracts, roots divide (noise density `V·Hz^(−1/2)` works). `Dimension` is `Copy + Eq + Hash` — a cheap catalog key that rides inside the compiler's type lattice.

**`quantity`** — `Quantity`: an exact magnitude (SI-base `Rational`) carrying a `Dimension` and a `Unit`. Same-dimension add/sub, dimension-combining mul/div, exact unit conversion (1 inch = 127/5000 m *exactly*, so `2 inches + 5 centimetres` in feet is exactly `42/127`), affine units (°C/°F) convert with scale and offset, and cross-dimension casts are impossible by construction.

**`money`** — `Money`: an exact amount in a specific `Currency`, riding `Decimal` (`0.10 + 0.20` is exactly `0.30`), quantised to the currency's minor unit (USD 2, JPY 0, BHD 3) with banker's rounding. Cross-currency `add`/`sub` return `None` — the monetary analogue of forbidding `meter + gram`; `RateTable` performs explicit conversions; the `currency` module is the ISO-4217 catalog.

**`temporal`** — exact calendar primitives over one absolute coordinate, the day count since 1970-01-01: proleptic Gregorian (Hinnant's `days_from_civil`/`civil_from_days`) and proleptic Julian (via JDN) as two lossless lenses on the same day number, plus ISO-8601 week dates and weekday arithmetic. All exact integer arithmetic — no floats, no lookup tables.

**`uuid`** — `Uuid` (RFC 9562): a fixed `[u8; 16]` in network byte order, `Copy`, `Ord` by bytes (v6/v7 sort chronologically). Every standard version: nil/max, v1/v6 (gregorian time), v3/v5 (MD5/SHA-1 name-based), v4/v7 (random / Unix-millis time-ordered), v8 (vendor-defined). Generation takes entropy/time as *parameters* — pure and deterministic, so the execution tiers seed it for byte-identical cross-tier output. Validated against the `uuid` crate in the tests.

### Hash oracles and hardware specs

**`hash`** — MD5 (RFC 1321) and SHA-1 (RFC 3174) in pure Rust — the *reference oracle*. The language-level implementations are written in LOGOS (`uuid.lg`: `md5Digest`/`sha1Digest`) and compile natively through the Futamura pipeline; these Rust versions are the independent oracle the Logos ones are proven byte-exact against. Not for security (both are collision-broken); validated against the `md-5`/`sha1` crates. (SHA-3/Keccak, the modern hash, lives in `logicaffeine_system`.)

**`sha_ops`** — the four Intel SHA-NI operations (`sha1rnds4`/`sha1msg1`/`sha1msg2`/`sha1nexte`) in software, bit-for-bit: the spec the tree-walker runs so SHA-1-over-these-ops produces identical results interpreted or AOT-compiled to the real instruction. Tests assert the software op equals the hardware intrinsic on random inputs. (The pure number-theory / cryptanalysis substrate these once anchored — factoring, ECM, LLL, order-finding, isogeny graphs — now lives one tier up in `logicaffeine_proof`, alongside the prover code that is its only consumer.)

Every module carries runnable doctests plus inline `#[cfg(test)]` units; run with `cargo test -p logicaffeine-base`.

## Dependencies

No internal (workspace) dependencies — this is Tier 0. The sole external dependency is `bumpalo 3.19` (backing `Arena`). The `md-5`/`sha1`/`uuid` reference crates are **dev-dependencies only** — differential oracles our own implementations are validated (and benchmarked) against; they never enter the shipped graph. There are no feature flags and no `build.rs`. The version is lockstep with the workspace.

## License

Business Source License 1.1 — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/CHANGELOG.md)
