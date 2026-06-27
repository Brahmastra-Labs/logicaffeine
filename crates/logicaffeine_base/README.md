# logicaffeine-base

Pure structural atoms for the [Logicaffeine](../../NEW_README.md) workspace: arena allocation, string interning, source spans, spanned errors, an arbitrary-precision numeric tower, and a union-find — generic, reusable infrastructure with no knowledge of English vocabulary and no I/O.
Part of the [Logicaffeine](../../NEW_README.md) workspace. Tier 0 — no internal dependencies; everything else builds on it.

## Role in the workspace

This is the bottom of the stack. Every higher crate (`compile`, `language`, `kernel`, `lexicon`, `lsp`, `proof`, `data`, `system`, and the integration `tests` crate) depends on it for the same handful of primitives: bump-allocated AST storage, interned symbols with O(1) equality, byte-offset spans, the `SpannedError`/`Result` error pair, exact numerics, and one shared equivalence engine. See [architecture.md](../../new_docs/architecture.md) for where Tier 0 sits relative to the rest.

## Public API

The crate root re-exports `Arena`, `Interner`, `Symbol`, `SymbolEq`, `BigInt`, `Rational`, `Span`, `SpannedError`, and `Result`. `UnionFind` lives in the `union_find` module and is reached as `logicaffeine_base::union_find::UnionFind`.

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

**`numeric`** — the exact numeric tower's foundation, so a number's type survives every boundary instead of collapsing onto an IEEE-754 double (no 2^53 cliff).
- `BigInt` — arbitrary-precision integer, sign + little-endian base-2^64 limbs (normalized; zero is the empty magnitude). `zero/from_i64/from_u64/parse_decimal`, `add/sub/mul/div_rem/pow/negated/abs`, `to_i64/to_f64/is_zero/is_negative`, `to_le_bytes/from_le_bytes`, `From<i64>`, full `Ord`/`Display`/`Debug`.
- `Rational` — exact fraction as a reduced `BigInt` numerator/denominator (`den > 0`, `gcd = 1`). `new/from_bigint/from_i64/from_ratio_i64/zero/one`, `numerator/denominator/is_integer`, `add/sub/mul/div/recip/pow/floor/ceil/round`, `to_bigint/to_i64/to_f64/parse`, `From<i64>`/`From<BigInt>`, `Ord`/`Display`.

**`union_find`** — `UnionFind` over `usize` ids with path-compressed `find` and `union` by rank (near-constant amortized cost). One equivalence engine under two consumers: the kernel's congruence closure (`logicaffeine_kernel::cc`) and the compiler's equality-saturation e-graph.
- `make_set() -> usize`, `find(x) -> usize`, `union(x, y) -> bool` (true if the classes were distinct), `len()` / `is_empty()` (elements ever created, not live classes)

Every module carries runnable doctests plus inline `#[cfg(test)]` units; run with `cargo test -p logicaffeine-base`.

## Dependencies

No internal (workspace) dependencies — this is Tier 0. The sole external dependency is `bumpalo 3.19` (backing `Arena`). There are no feature flags and no `build.rs`. Version `0.9.17`, lockstep with the workspace.

## License

Business Source License 1.1 — see [LICENSE.md](../../LICENSE.md).

---
[Docs index](../../new_docs/README.md) · [Root README](../../NEW_README.md) · [Changelog](../../CHANGELOG.md)
