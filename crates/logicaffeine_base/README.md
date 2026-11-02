# logicaffeine-base

The structural atoms every Logicaffeine crate builds on (arenas, interned symbols, source spans, spanned errors, a union-find) and the exact value types LOGOS computes with: big integers, fractions, decimals, quantities with units, money, calendar dates and UUIDs. A double says `0.1 + 0.2` is `0.30000000000000004` and quietly rounds integers past 2^53; `Decimal` says `0.3`, and `BigInt` never rounds at all. It knows no English vocabulary and performs no I/O.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace, tier 0. It has no runtime dependencies.

## Role in the workspace

This is the bottom of the stack. Most of the workspace depends on it directly: `compile`, `data`, `kernel`, `language`, `lsp`, `proof`, `system`, the web app, largo, and the `tests` crate. They take from it owning arena storage for AST nodes, interned symbols with O(1) equality, byte-offset spans, the `SpannedError`/`Result` pair, and one shared equivalence engine.

The exact value types live here, in the leaf crate, so the proof layer, the wire codec and the tests share one implementation of every number, date, quantity and identifier. A number's *type* survives every boundary (interpreter, VM, wire) instead of collapsing onto an IEEE-754 double. See [architecture.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/architecture.md) for where tier 0 sits relative to the rest.

## Example

Exact decimal arithmetic, and money that refuses to add dollars to euros:

```rust
use logicaffeine_base::{currency, Decimal, Money};

assert_ne!(0.1_f64 + 0.2, 0.3);

let sum = Decimal::parse("0.1").unwrap().add(&Decimal::parse("0.2").unwrap());
assert_eq!(sum, Decimal::parse("0.3").unwrap());
assert_eq!(sum.to_string(), "0.3");

let usd = currency::by_code("USD").unwrap();
let dime = Money::of(Decimal::parse("0.10").unwrap(), usd);
let twenty_cents = Money::of(Decimal::parse("0.20").unwrap(), usd);
assert_eq!(dime.add(&twenty_cents).unwrap().to_string(), "0.30 USD");

let euro = Money::of(Decimal::parse("1.00").unwrap(), currency::by_code("EUR").unwrap());
assert_eq!(dime.add(&euro), None);
```

## Public API

The crate root re-exports the workhorses. The structural atoms are `Arena`, `Interner`/`Symbol`/`SymbolEq`, `Span` and `SpannedError`/`Result`. The value types are the numeric tower `BigInt`/`Rational`/`Decimal`/`Complex`/`ComplexF`/`Modular`/`RoundingMode`; the measurement types `BaseDim`/`Dimension`/`Exp`, `Quantity`/`Unit`, `Currency`/`Money`/`RateTable` (and the `currency` catalog); `Uuid`/`Variant`; and the machine words `Word8`/`Word16`/`Word32`/`Word64`/`WordVal` with their SIMD lane packs (`Lanes4Word32` through `Lanes16Word8`). The Reed-Solomon codec `ReedSolomon` is re-exported too, and everything else is reached through its module.

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

**`arena`**: typed storage with stable references across later allocations, so AST nodes can point at each other without reference counting. Dropping or resetting the arena runs each value’s destructor, releasing owned strings and vectors.
- `Arena::<T>::new()` / `Default`
- `alloc(&self, value: T) -> &T`
- `alloc_slice<I: IntoIterator<Item = T>>(&self, items: I) -> &[T]`, where `I::IntoIter: ExactSizeIterator` pre-sizes the allocation
- `reset(&mut self)`: drop every value and keep the capacity for subsequent evaluations

**`intern`**: string interning. `Symbol` is a `Copy` `u32` handle, so comparison is integer comparison regardless of string length. The empty string is pre-interned at index 0.
- `Interner::new()` / `Default`, `intern(&mut self, s: &str) -> Symbol`
- `resolve(&self, sym: Symbol) -> &str` (panics if `sym` is foreign), `lookup(&self, s: &str) -> Option<Symbol>`
- `len()` / `is_empty()`: `len` counts the empty string, and `is_empty` is true when only it is present
- `Symbol::EMPTY` (= `Symbol::default()`), `index() -> usize`, `from_index(usize) -> Symbol`: a dense round-trip the bounds prover uses to thread symbols through linear-expression variable ids
- `SymbolEq::is(&self, &Interner, &str) -> bool`: compare a symbol to a literal without an explicit `resolve`

**`span`**: `Span { start: usize, end: usize }`, `Copy` + `Default`, public fields; byte offsets match `&source[span.start..span.end]`.
- `Span::new(start, end)` (no validation; `start` may exceed `end`)
- `merge(self, other: Span) -> Span` (min start, max end), `len() -> usize` (saturating), `is_empty() -> bool` (true when `start >= end`)

**`error`**: `SpannedError { message: String, span: Span }` implements `std::error::Error` and `Display` as `"{message} at {start}..{end}"`.
- `SpannedError::new(message: impl Into<String>, span: Span)`
- `type Result<T> = std::result::Result<T, SpannedError>`

**`union_find`**: `UnionFind` over `usize` ids with path-compressed `find` and `union` by rank (near-constant amortized cost). One equivalence engine serves two consumers: the proof kernel's congruence closure (`logicaffeine_kernel::cc`) and the compiler's equality-saturation e-graph.
- `make_set() -> usize`, `find(x) -> usize`, `union(x, y) -> bool` (true if the classes were distinct), `len()` / `is_empty()` (elements ever created, not live classes)

### Exact numeric tower

**`numeric`**: the tower's foundation, so a number's type survives every boundary instead of hitting the 2^53 cliff of a double.
- `BigInt`: arbitrary-precision integer, sign plus little-endian base-2^64 limbs; a single-limb magnitude is stored inline, so anything that fits 64 bits needs no heap. `zero/from_i64/from_u64/parse_decimal`, `add/sub/mul/div_rem/pow/negated/abs/gcd`, in-place `mul_assign_i64`, bit operations on non-negative values (`bit_len`, `shl_bits`/`shr_bits`, `bitand_natural`/`bitor_natural`/`bitxor_natural`), `to_i64/to_u64/to_f64/is_zero/is_negative`, `to_le_bytes/from_le_bytes`, `From<i64>`, full `Ord`/`Display`/`Debug`. Multiplication switches from schoolbook to Karatsuba at 32 limbs; division is Knuth's Algorithm D.
- `Rational`: an exact fraction as a reduced `BigInt` numerator and denominator (`den > 0`, `gcd = 1`). `new/from_bigint/from_i64/from_ratio_i64/zero/one`, `numerator/denominator/is_integer`, `add/sub/mul/div/recip/pow/floor/ceil/round`, `to_bigint/to_i64/to_f64/parse`, `Ord`/`Display`.
- `Decimal`: exact base-10 fixed point, the anti-float for money and human-entered numbers, with `RoundingMode` (banker's rounding included).
- `Complex`: complex numbers with exact `Rational` parts.
- `ComplexF`: the floating-point complex (`f64` parts; `conjugate`, `abs`, `norm_sqr`, `recip`, `sqrt`, `pow`), the inexact counterpart that the sqrt-based complex decompositions need.
- `Modular`: an element of ℤ/nℤ over `BigInt`, wrapping into `[0, modulus)`, with modular exponentiation and the extended-Euclid inverse.

**`word`**: fixed-width wrapping integers, the ring ℤ/2ᵏℤ: `Word8`/`Word16`/`Word32`/`Word64` plus the type-erased `WordVal`. Unlike `BigInt`, arithmetic is total and wrapping (`Word32::MAX.add(Word32::ONE) == Word32::ZERO`), and `WordVal` wraps each width at its own modulus, `W8` and `W16` included. This is the home of the bit-twiddling crypto substrate: ChaCha20 runs over `Word32` and Keccak over `Word64`. Rotation (`rotl`/`rotr`) is width-defined and exists only here. The lane packs (`Lanes4Word32`, `Lanes8Word32`, `Lanes4Word64`, `Lanes16Word8`, `Lanes16Word16`, `LanesVal`) are the scalar *specification* of the SIMD lanes the AOT execution tier compiles to AVX2.

**`gf256`**: the finite field GF(2⁸), the Reed-Solomon and AES-style field with primitive polynomial `0x11d`, where addition is xor. `mul`/`div`/`pow`/`inv`, matrix `invert`/`matmul`/`matvec`, and exact linear algebra (`solve`, `nullspace`, `rank`). One implementation serves the wire codec's Reed-Solomon FEC, the Rainbow multivariate signature, and the XL algebraic-cryptanalysis engine, with no floating point anywhere.

**`fec`**: Reed-Solomon forward error correction over `gf256`. From `k` data shards it computes `m` parity shards, and the `n = k + m` shards are *Maximum Distance Separable*: any `k` of the `n` reconstruct the original exactly, so losing up to `m` shards costs no retransmit round-trip. This is the erasure layer the wire codec ships over.

**`describe`**: the integer-sequence description-length codec (the MDL primitive). `describe_int_seq` encodes an `&[i64]` as the *shortest* program from a fixed menu of generators: affine, geometric, degree-≤4 polynomial, linear recurrence, periodic, sparse, a sandboxed `GenExpr`, and the columnar fallbacks (delta, delta-of-delta, frame-of-reference bit-pack, run-length, dictionary, raw bytes, varint). `decode_int_seq` is its exact inverse, so the encoding is a re-checkable *witness* for a computable upper bound on Kolmogorov complexity, and it is never larger than plain varint.

The wire codec (`logicaffeine_compile`'s `marshal`, `WireStructure::Auto`) and the proof layer's AIT certificates share this one implementation. The denial-of-service bound (`max_elements`) is a caller-supplied parameter, since the crate has no receiver policy of its own. It bounds generator work as well as output length: LFSR steps, FCSR steps and `GenExpr` nodes are charged against one budget per column tree, so a tiny descriptor that would expand to minutes of work is refused.

### Exact reals, infinitesimals and ordinals

The numbers behind LOGOS's infinitesimal `dt` and its supertasks. Every comparison here is decided
exactly where a theorem decides it, and fails loudly with a typed error where none does.

```rust
use logicaffeine_base::algebraic::Algebraic;
use logicaffeine_base::exact_real::ExactReal;
use logicaffeine_base::hyperreal::Hyperreal;
use logicaffeine_base::limit_value::{Depends, LimitRule, ResidueType};
use logicaffeine_base::ordinal::Ordinal;

let root2 = Algebraic::from_i64(2).sqrt().unwrap();
assert_eq!(root2.to_string(), "√2");
let phi = Algebraic::from_i64(1).add(&Algebraic::from_i64(5).sqrt().unwrap()).div(&Algebraic::from_i64(2)).unwrap();
assert_eq!(phi.mul(&phi), phi.add(&Algebraic::from_i64(1)));

let e2 = ExactReal::from_i64(2).exp();
assert!(e2.mul(&ExactReal::from_i64(-2).exp()).sub(&ExactReal::from_i64(1)).is_exactly_zero());
assert_eq!(ExactReal::pi().to_decimal(10), "3.141592654");

let dt = Hyperreal::dt();
assert!(dt.is_infinitesimal() && !dt.recip().unwrap().is_limited());
assert_eq!(Hyperreal::from_i64(3).add(&dt).standard_part(), Some(Algebraic::from_i64(3)));

let omega = Ordinal::omega();
assert_eq!(Ordinal::from_u64(1).add(&omega), omega);

let lamp = Depends::from_fn(2, &ResidueType::any(), |r| Algebraic::from_i64(r as i64))
    .expect("two residue classes are within the bound");
assert_eq!(lamp.render("⌈1/dt⌉", |v| v.to_string()), "0 when ⌈1/dt⌉ is even, 1 when it is odd");
assert_eq!(lamp.apply(LimitRule::CesaroMean).unwrap().to_string(), "1/2");
```

**`poly`**: exact univariate polynomials over ℚ. `Poly` keeps little-endian `Rational` coefficients
with no trailing zero, so equality is structural.
- ring operations, `div_rem`, monic `gcd`, `derivative`, `eval`, `compose`, `reverse`, `neg_var`,
  `squarefree_part`, `primitive_int`
- real roots: `count_real_roots`, `count_real_roots_in(lo, hi)` (Sturm chains on the squarefree part,
  half-open `(lo, hi]`), `isolate_real_roots() -> Vec<RootInterval>`, `refine`, `root_bound`
- `resultant`, and `factor_over_q()` into irreducible primitive integer factors with multiplicities
  (Zassenhaus: Berlekamp modulo a small prime, Hensel lifting past the Mignotte bound,
  recombination); `is_irreducible`, `canonical_order`

**`algebraic`**: `Algebraic`, a real algebraic number held as (minimal polynomial, root index) with a
refinable isolating interval. Equality and hashing read the canonical pair; order is decided by
refinement.
- `from_i64`, `from_rational`, `from_f64` (the exact dyadic value), `root_of(&Poly, k)`
- `real_roots(&[Algebraic])`: the real roots, with multiplicities, of a polynomial whose coefficients
  are algebraic numbers, found among the roots of its norm over ℚ and each checked exactly
- `add`, `sub`, `mul`, `div`, `neg`, `recip`, `pow`, `sqrt`, `nth_root` (resultants, then selection of
  the one root inside an interval enclosure)
- `floor`, `ceil`, `signum`, `to_f64` (correctly rounded), `enclosure(bits)`, `minimal_polynomial`,
  `root_index`, `degree`, `to_rational`
- `Display`: `√2`, `(1 + √5)/2`, `3√2/4`, `root 1 of x^3 - x - 1`

**`exact_real`**: `ExactReal`, a computable real. Three exact forms decide equality: algebraic numbers,
exponential-algebraic sums `Σ aₖ·e^{βₖ}` (Lindemann–Weierstrass; also `sin` and `cos` of algebraic
arguments), and Laurent polynomials in π. Every other value is an expression DAG enclosed in dyadic
intervals to any precision. There is no `PartialEq`: numeric equality is `compare`.
- `pi`, `e`, `exp`, `log`, `sin`, `cos`, `sinh`, `cosh`, `sqrt`, field operations, `pow`
- `nth_root(n)`: exact on algebraic numbers and on a single real exponential term,
  `ⁿ√(c·e^β) = ⁿ√c·e^(β/n)` (`√e` is `e^(1/2)`)
- `compare(&o, budget) -> Result<Ordering, RealError>` (`Inseparable` when values agree to `2^-budget`
  and no exact form decides them), `compare_within(&o, tolerance) -> Result<Closeness, RealError>`
- `enclosure(bits)`, `floor(budget)`, `to_decimal(digits)` (correctly rounded), `truncated_decimal`,
  `is_exactly_zero`, `exact_algebraic`, `describe`

**`ordered_field`**: polynomials over any ordered field with exact arithmetic. `OrderedField` is what a
coefficient field provides (field operations, a reciprocal that refuses zero, a decided sign); `Algebraic`
is one, and so is a value of several infinitesimals. `PolyOver<K>` has sums, products, Euclid's division,
the monic gcd, the derivative, Yun's square-free decomposition (`squarefree`), evaluation, `spread`
(`p(x^m)`), `shift`, `neg_var`, `reversed`, `without_zero_root`, the `resultant`, Lagrange `interpolate`
and the Padé approximant `pade`. The hyperreals' rational functions are built on it.

**`hyperreal`**: `Hyperreal`, the field generated by the infinitesimal `dt`: reduced rational functions
of `dt` with real algebraic coefficients. The order is the sign of the lowest-order coefficient (the
transfer principle for real closed fields).
- `dt`, `monomial(c, k)`, field operations, `pow`, `signum`, `valuation`, `leading_coefficient`,
  `degrees`
- `is_infinitesimal`, `is_limited`, `standard_part`, `infinitely_close`, `eval_at(t)`,
  `laurent_coefficient(k)`
- `ramify(m)` and `unramify(m)`: the same function of `dt^m`; `pade(series, l, m)`: the rational
  function of type `[l/m]` an expansion begins
- `render(name)`: `3 + 2·dt`, `1/dt`, `(1 + dt)/(2 - dt)`; `render_powers(power)` with any power text

**`puiseux`**: the real closure of the hyperreals. `Closed` is a value of it in its one form: `Field`, a
`Hyperreal`, or `Series`, a `Puiseux` — a real algebraic Puiseux series in `dt` held as a squarefree
polynomial over the rational functions of `dt^(1/q)` and the branch of its real roots it is, found by
Newton's polygon and extended term by term on demand.
- `from_hyperreal`, `add`, `sub`, `mul`, `div`, `neg`, `recip`, `pow`, `root(n)` (the positive root
  for even `n`; a negative value's even root refused): each result a root of a resultant, picked by
  the expansion of the operation on its operands' expansions through the order that separates
  that polynomial's roots
- `compare` (exact: the sign of the difference), `signum`, `valuation`, `is_infinitesimal`,
  `is_limited`, `standard_part`; `Eq`, `Hash`, `encode` and `decode`
- `Puiseux::ramification`, `coefficient(k)` and `order`: the expansion read on its branch's own grid,
  `dt^(k/q)`; `rational_function`, the rational function of `dt^(1/q)` a value is, when it is one
- `render(name)`: `√dt`, `1 + 2·√dt + dt`, `√(1 + dt)`, `2·∛(1 + dt)`, and otherwise the leading
  terms, `2√2 - (√2/16)·dt^2 + O(dt^3)`

**`multi`**: `Multi`, a value of several ordered infinitesimals — `dx` much smaller than `dt` — in its one
form: `Standard`; `One`, a value of one infinitesimal in that one's own form; `Over`, `num/den` in a root of
the smallest infinitesimal with every coefficient a `Multi` of the larger ones, reduced, the denominator
comonic, on the least root that holds it; `Alg`, a root there that is no rational function of it
([`closure`]). These are the real closure of the field `ℝ(dt)(dx)` in which `dx` lies below every
positive value of `dt`; infinitesimals are keyed by their declaration and combine only when ordered
(`Scale::order`), a value read on a residue class keeping the facts that class adds.
- field operations, `pow`, `root(n)`, `signum`, `leading`, `scales`, `top` (the smallest)
- `standard_part_at(at)`, `is_infinitesimal_at`, `is_limited_at`: read from the standpoint of `at`,
  every smaller infinitesimal at zero, the inner limit with `at` held fixed
- `is_integral`: a polynomial in the reciprocals that is an integer on every class its scales' facts
  admit — an infinite integer of several, `⌊1/dt + 1/dx⌋`
- `render`: `dt + dx`, `(dt + dx)/(dt - dx)` with denominators cleared by a recursive polynomial gcd;
  an infinite integer by its reciprocals, `1/(dt·dx) + 1/(2·dt) - 1/3`; `to_bytes`, `from_bytes`

**`closure`**: `AlgValue`, a value of the real closure at the smallest of several infinitesimals that
is no rational function of a root of it: its recipe, a squarefree annihilator over the values of the
larger infinitesimals, and its expansion. It is `0` when its polynomial's constant is or its expansion
vanishes through the largest order Newton's polygon puts a root at; `field_value` finds the rational
function a value is, by a Padé approximant within Gauss's bounds checked exactly.

**`expansion`**: `Expansion<C>`, a lazy series in a root of one infinitesimal over a `Coefficient` ring
(field operations that may refuse, a decided sign, roots, `exp`, `ln`): constants, fractions, sums,
products, reciprocals, refinement to a finer root, Miller's roots, `exp` and `ln` by their recurrences,
and any `Source`. Each coefficient is computed once, on demand; `leading_through(k)` reads the first
nonzero one.

**`number`**: `Number`, base's own value of the exact tower: a `TowerValue` whose arithmetic and
relations are the tower's own. The coefficients of a germ of several infinitesimals are numbers, and
the integer parts and flows of several are computed in it.

**`nested`**: `NestedGerm`, a germ of several infinitesimals — `exp(dt + dx)`, a flow's end state: its
recipe and its `Expansion<Number>` in the smallest infinitesimal, whose coefficients are numbers of the
larger ones. Two germs are equal by recipe and compare at the first order their expansions differ;
`exp`, `ln` (refused where the logarithm carries one of the infinitesimals), roots and powers,
`standard_part_at`, `render` (`1 + (1/dt)·dx + O(dx^2)`), and `to_bytes` by recipe.

**`nested_flow`**: flows and powers of matrices of numbers of several infinitesimals. `matrix_exp`:
`e^{t·A}`, by `exp_matrix` for a rational matrix and otherwise order by order in the smallest
infinitesimal, each order the corner of a block-Toeplitz exponential. `flow`: the Euler steps of a clock
whose rates are values of larger infinitesimals. `power`: `A^N·x₀` for rates of a smaller one, as
`exp(N·log A)` order by order, and for a count infinite in that one itself — `⌈1/(dt·dx)⌉` steps of
`1 + dt·dx` — as the exponential of `N·log A`, limited whenever the steps add up to a limited amount.

**`elimination`**: `Polynomial`, a sparse polynomial in several variables over the rationals, and
`resultant(o, v)`, which eliminates variable `v`, interpolated one variable at a time from univariate
resultants at points where neither leading coefficient vanishes. `algebraic`'s norms are built on it.

**`germ`**: `Germ`, the value of a flow at `dt = 0`: a lazy Puiseux series in `dt` with `ExactReal`
coefficients, its orders the multiples of `1/q` for its ramification `q`. It is built from rational
functions of `dt` (`from_hyperreal`), values of the real closure (`from_puiseux`), constants, field
operations, `exp`, `ln` and `root(n)`, and the components of a linear flow (`Flow::new(terms, t, δ,
start)`, `Flow::on_grid` for an update on a root of `dt`, `Germ::flow`). Two germs meet on the least
grid both lie on. A flow runs `T/dt + δ` Euler steps of `A(dt) = I + dt·A₁ + dt²·A₂ + …` as
`exp(N·log A(dt))·x₀`; its standard part is `e^{T·A₁}·x₀`, and every coefficient is known when the `Aₖ`
commute.
- `coefficient(k)`, the coefficient of `dt^(k/q)` (`GermError::NotComputed` past the orders a
  non-commuting flow determines), `ramification`, `leading`, `signum`, `standard_part`, `recip`, `div`
- `exp` (of a limited germ; `GermError::BeyondEveryPower` otherwise), `ln` (of one with a positive
  standard part; `GermError::CarriesLog` for an infinitesimal or infinite one, `Nonpositive` for a
  negative one), `root(n)` (`GermError::NegativeRoot` for an even root of a negative germ), each
  coefficient from the ones before it
- `render(name)`: `e - (1/2)·e·dt + O(dt^2)`, `1 + √dt + O(dt)`; the coefficients through
  `dt^SIGN_ORDERS` decide a sign, and a germ flat to that order is `GermError::Flat`, never a guess

**`matrix_exp`**: the exact matrix exponential of a rational matrix. `characteristic_polynomial` by
Faddeev–LeVerrier; `exp_matrix(M, t)` interpolates `e^{t·x}` on the eigenvalues to their
multiplicities through a confluent Vandermonde system over the complex algebraic numbers, so every
entry is an exponential sum: `e^{[[0, 1], [−1, 0]]}` is `[[cos(1), sin(1)], [−sin(1), cos(1)]]`.

**`ode`**: validated integration of a polynomial differential equation `x' = f(x)` (`Field`,
`Monomial`). `flow(field, start, t, q, cap)` returns interval enclosures that always contain the
solution: a Picard a priori box per step, a Taylor model from Cauchy products of the solution's
coefficients, Lagrange's remainder over the box; `FlowError::Escapes` when no box closes.
`escape_time_bound(p, x₀, cap)` bounds the time a scalar solution takes to leave every bound.
`ExactReal::flow` wraps the solution as computable reals.

**`complex_algebraic`**: `ComplexAlgebraic`, `re + im·i` with real algebraic parts, with field
operations, `conj`, `pow` and `eval`. `complex_roots(p)` returns every root of a rational polynomial
once each, reals ascending, then each conjugate pair upper root first: real parts are half the real
roots of `Res_y(g(y), g(x − y))`, imaginary parts come from the differences of roots, and a candidate
is accepted only once `g(a + b·i) = 0` checks exactly.

**`hypernatural`**: `Hypernatural`, an infinite integer polynomial in the generator `H = 1/dt`, one
polynomial per residue class of `H`: the trip counts of supertasks.
- `generator`, `from_integer`, `add`, `sub`, `mul`, `neg`, `scaled_floor`, `scaled_ceil`
- `rem(k)`, `cmp`, `is_infinite`, `is_negative`, `eval_at`, `to_hyperreal`
- `render(generator)`, `render_reciprocal(inf)`: `2/dt + 1`

**`limit_value`**: what a supertask leaves.
- `ResidueType`: the residues the generator may take (`any`, `congruent`, `and`, `allows`)
- `Depends<T>`: a quantity on every residue class, held modulo the smallest separating modulus
  (`from_fn`, `map`, `zip_with`, `single`, `at`, `render`); `apply(LimitRule)` for the standard part,
  the limit, the limit superior and inferior, and the Cesàro mean
- `Converging<T>`: a settled value with its certificate, or a provisional one with the cases checked
  and the revisions made (`render`: `false (provisional: checked 10,000,000 cases, 0 revisions; not
  proved)`)

**`exact_codec`**: `ExactCodec`, the canonical byte encoding every exact type crosses a task
boundary, the wire and storage in: `write`, `read`, `to_bytes`, and `from_bytes`, which accepts an
encoding only when it is complete, has nothing trailing, and is the canonical name of the value it
decodes to, so an unreduced fraction or a reducible "minimal polynomial" is refused. Sizes are
bounded before any work: `MAX_WIRE_DEGREE`, `MAX_WIRE_LEN`, `MAX_WIRE_MAGNITUDE`, `MAX_WIRE_DEPTH`.

**`ordinal`**: `Ordinal`, an ordinal below ε₀ in Cantor normal form: the stage counter of transfinite
execution.
- `zero`, `one`, `from_u64`, `omega`, `omega_pow`, `add`, `mul`, `pow` (ordinal arithmetic: `1 + ω = ω`,
  `2^ω = ω`), `successor`, `predecessor`, `is_limit`, `is_successor`, `to_natural`
- `Ord` (Cantor normal form, lexicographic), `Display`: `ω^2·3 + ω + 5`, `ω^(ω + 1)`

**`tower`**: how the exact types mix with the numeric tower and with each other, once, for every
value model. A model implements `TowerValue` (how to see one of its values as an exact type or a
standard number, and how to build one) and gets `arith`, `relation`, `display`, `standard_part`,
`integer_part`, `is_infinitesimal`, `is_limited`, `apply_limit`, `undecided_branch` and the transport
encodings. Every result is held in its smallest exact form: a hyperreal with no infinitesimal part is
its standard value, an algebraic number that is rational is a rational, a root of a hyperreal that is a
rational function of `dt` is that hyperreal. `real_fn` takes the square root of a hyperreal in the real
closure, and `exact_power` raises to an exact exponent, a fraction taking a root. A residue-dependent value
(`DependsValue`, branches of `Branch`) lifts every operation branch by branch; a provisional one
(`Converging`) carries its evidence through.

Several infinitesimals are ordered by their declarations (`new_ordered_infinitesimal`): a value of
several is a `Multi` or a `NestedGerm`, read from an infinitesimal's standpoint by
`standard_part_relative`, `is_infinitesimal_relative` and `is_limited_relative`, and refused, by name,
where it would combine infinitesimals whose order the program never states. A value that depends on the
residues of several generators is a dependence on the coarsest — the generator of the largest
infinitesimal — whose branches read the finer, whatever builds it; limit rules read the finer within
each class of the coarser. The integer part of a value infinite in several is an infinite integer of
several (`HyperForm::Integer`) on the classes its fraction needs, and is refused by name where no
residue decides that fraction, as in `⌊dt/dx⌋`. `+` with a `Text` operand is left to the model, which
joins the value as it prints.

**`extension`**: the extension's builtins (`infinitesimal`, `standardPart`, `isInfinitesimal`,
`isLimited`, `infinitelyClose`, `generatorOf`, `limitOf`, `provisional`, `atTheLimit`, `tripCount`,
`affinePower`, `polynomialFlow`, `hybridFlow`, `cycleAt`, `searchCandidates`, `stateSize`, `everyNatural`,
`exactPower`, `orderedInfinitesimal`) as one
enum, `Builtin`, with the names a program calls them by, and `call`, which reads their arguments and
runs them on any `ValueAlgebra`. The interpreter and a compiled program both answer through it, so a
builtin computes the same value and refuses in the same words on every tier.

**`supertask`**: the supertask engine's run-time algorithms, generic over a `ValueAlgebra` (a
`TowerValue` with its whole arithmetic, decided relations and lists): `trip_count`, `affine_power`
(with `check_annihilator`, the certificate `A^z·(A^p − I)^m = 0` multiplied out), `polynomial_flow`,
`hybrid_flow`, `cycle_at`, `state_size` and `search_candidates`. A step count may be infinite in several
infinitesimals: an affine update's closed form and a cycle are read on its classes, and a flow's power
is `exp(N·log A)`; values that depend on several generators are read on one's classes at a time, and the
answers join in the canonical nesting.

**`hybrid`**: the ideal run of a loop of infinitesimal steps whose state flows between jumps, in
real algebraic numbers. `run(model, start, span)` follows a `Model` — the body in order as `Item`s
(runs of flow statements by each slot's rate, and jump checks), and each `Jump`'s guard and
statements over polynomials in the slots (`MPoly`) — through its flights (polynomials in time, from a
terminating Lie series), its events (exact roots of each guard atom along a flight, `real_roots`),
its jumps (three-valued, deciding values infinitely close to each other only by exactness or a sign
the guard or a reset implies), its tails (a cycle of firings that a `Scaling` maps round to round:
every later value a polynomial in `z = sᵏ`, the firings accumulating at a geometric sum when
`s < 1`, `check_scaling` the certificate checked on its own) and its continuous firing (a jump that
fires again at once holds what it sets while the rest flows). Every `Cell` it leaves says whether
the loop's own value is exactly it or only infinitely close to it; a run whose end depends on which
infinitesimal `dt` is is refused by name.

### Measurement, time, money, identity

**`dimension`**: physical dimensions as an abelian group of *rational* exponent vectors over the base dimensions (`BaseDim`, exponents `Exp`). Multiplication adds vectors, division subtracts them, and roots divide them, so a noise density in `V·Hz^(−1/2)` is expressible. `Dimension` is `Copy + Eq + Hash`, a cheap catalog key that rides inside the compiler's type lattice.

**`quantity`**: `Quantity`, an exact magnitude (an SI-base `Rational`) carrying a `Dimension` and a `Unit`. Same-dimension add and sub, dimension-combining mul and div, and exact unit conversion: 1 inch is exactly 127/5000 m, so `2 inches + 5 centimetres` in feet is exactly `42/127`. Affine units (°C, °F) convert with scale and offset, and cross-dimension casts are impossible by construction. `quantity::units::by_name` resolves unit words, the electrical family included (`volt`, `millivolt`, `amp`, `milliamp`, `coulomb`, `ohm`, `millisiemens`, and more), so `5 volts` is a quantity literal.

**`si`**: SI-dimensioned `f64` newtypes (`Volts`, `Nanoseconds`, `Hertz`, `VoltsPerRootHz`, and others) for characterized hardware descriptors. Each names its unit, knows its `Dimension` (rational exponents included), and keeps the payload in the named unit, so switching a bare field to its newtype never moves a pinned value.

**`money`**: `Money`, an exact amount in a specific `Currency`, riding `Decimal` and quantised to the currency's minor unit (USD 2, JPY 0, BHD 3) with banker's rounding. Cross-currency `add`/`sub` return `None`, the monetary analogue of forbidding `meter + gram`. `RateTable` performs explicit conversions, and the `currency` module is the ISO-4217 catalog.

**`temporal`**: exact calendar primitives over one absolute coordinate, the day count since 1970-01-01. Proleptic Gregorian (Hinnant's `days_from_civil`/`civil_from_days`) and proleptic Julian (via the Julian Day Number) are two lossless lenses on the same day number, alongside ISO-8601 week dates and weekday arithmetic. All exact integer arithmetic: no floats, no lookup tables.

**`uuid`**: `Uuid` (RFC 9562), a fixed `[u8; 16]` in network byte order, `Copy`, and `Ord` by bytes, so v6 and v7 ids sort chronologically. Every standard version is here: nil and max, v1/v6 (Gregorian time), v3/v5 (MD5/SHA-1 name-based), v4/v7 (random and Unix-millisecond time-ordered), and v8 (vendor-defined). Generation takes entropy and time as *parameters*, so it is pure and deterministic, and the execution tiers seed it for byte-identical output across tiers. The tests validate it against the `uuid` crate.

### Hash oracles and hardware specs

**`hash`**: MD5 (RFC 1321) and SHA-1 (RFC 3174) in pure Rust, as the *reference oracle*. The language's own implementations are written in LOGOS (`md5Digest`/`sha1Digest` in the stdlib's `uuid.lg`, which the `md5`, `sha1`, `uuid_v3` and `uuid_v5` functions call); these Rust versions are the independent oracle they are checked byte-exact against. Not for security, since both hashes are collision-broken. The tests validate them against the `md-5` and `sha1` crates. SHA-3/Keccak, the modern hash, lives in `logicaffeine_system`.

**`sha_ops`**: the four Intel SHA-NI operations (`sha1rnds4`/`sha1msg1`/`sha1msg2`/`sha1nexte`) in software, bit for bit. They are the spec the tree-walker runs, so SHA-1 written over these ops gives the same result interpreted or AOT-compiled to the real instruction. A test drives random inputs through both the software op and the hardware intrinsic and requires them to agree.

Number theory and cryptanalysis (factoring, ECM, LLL, order-finding, isogeny graphs) are not here: they live in `logicaffeine_proof`, next to the prover code that uses them.

White-box unit tests live in `crates/logicaffeine_tests/unit/logicaffeine_base/`, reached through one-line `#[path]` stubs so they keep private access; run them with `cargo nextest run -p logicaffeine-base`.

## Dependencies

No runtime dependencies. The `md-5`, `sha1` and `uuid` crates are dev-dependencies only: differential oracles that our own implementations are validated and benchmarked against, never part of the shipped dependency graph. There are no feature flags and no `build.rs`.

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_base/CHANGELOG.md)
