# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- **`describe` searches representations, not only generators.** `T_INTS_MULT` describes a column sharing a common
  factor with its minimum by its quotients (NYC taxi timestamps, whole seconds held in microseconds: 26.1 → 12.2 bits a
  value, under pcodec's 12.5). `T_INTS_CXOR` splits wide bit patterns at their high bits and XORs each value's low bits
  against the last value that shared them, the split chosen by encoded size on a prefix (POI coordinates: 47.3 → 41.4
  bits a value, under xz's 43.2). `T_INTS_CM` is the entropy-coded form: whole-value predictors (order-2 and order-1
  followers, the previous value, a move-to-front cache of recent values) gated by their running hit rates, then sign,
  bit length and mantissa under a context-mixing model with a full-prefix trie, plus a lean adaptive-order-0 variant
  for independent columns (TPC-H `l_quantity` 5.65 bits a value, entropy-floor columns within 0.1% of pcodec).
- **Float columns describe as integers.** `emit_decimal_float_column` (`T_FLOATS_DECIMAL`) ships the exponent `e` under
  which the values are the correctly rounded `k / 10^e`, raw exceptions verbatim, and `k` through the integer menu;
  `emit_float_bits_column` (`T_FLOATS_BITS`) ships the IEEE-754 bit patterns as an integer column;
  `decode_float_column_body` inverts both bit for bit. On the ALP corpus sample (30 datasets) the two forms are the
  smallest of any codec measured on every dataset: 12.67 bits a value against pcodec's 15.42 and xz's 15.63.
- `Menu` (`Structural` by default, `Full`) and the scoped `with_menu` set how much a description searches; the
  entropy-coded form and the conditional-XOR split run only under `Full`, so a latency-bound caller pays for neither.
  `describe_int_seq` searches the full menu. `COLUMN_TAGS` lists every column tag this module writes.
- **`entropy`**: a carry-less binary arithmetic coder (`Encoder`, `Decoder`, the `BitIo` trait that drives one model
  in both directions) that takes `P(1)` as a 32-bit fraction (`encode_fine`, `decode_fine`, `BitIo::code_fine`; the
  12-bit `encode`/`decode`/`code` are it at `p12 · 2²⁰`, byte for byte), `squash`/`stretch`, `AdaptiveProb`, `HashedProbs` and `ProbBuckets` (64-byte buckets, one cache
  line per group of decisions), a gated logistic `Mixer`, the two-layer `Mixer2` (16-bit weights, its dot product and
  update SSE2 multiply-adds on x86-64 and the same integer arithmetic elsewhere, a dead zone that skips negligible
  updates), an `Apm`, a `Calibration` (per context and 12-bit probability, the zeros and ones that followed it, read
  back as a 32-bit probability), the integer coding cost `cost`, and `prefetch` — all integer arithmetic, so predictions are
  bit-identical on every machine. Bucketing the entropy-coded column's contexts made it about ten times faster at the
  same size.
- **`cm`**: context-mixing compression of byte streams. The encoder first changes the representation where a byte model
  cannot see the structure, each change exactly reversible and named in the header: DEFLATE streams (ZIP entries, gzip
  members, zlib streams, PNG image data stitched from its `IDAT` chunks) and GIF image data are opened to their decoded
  bytes when they re-compress to the same bytes, under zlib's settings or the block rules gzip and InfoZIP's zip follow
  (`cm::streams`) — streams inside opened ones too, three levels deep, and a GIF also when its encoder flushed one more
  zero byte after the end code or ended matches early where its dictionary had lost a string (the pixels where it did
  are listed in the header), a GIF image's indices renumbered by the luma of its colour table so that close colours are
  close numbers in every image, the streams' headers, every level's, themselves entropy-coded under the fast model; the
  fields a container's own bytes determine are blanked and computed again on decoding — a tar header's checksum (zeros
  in the field's own shape), a ZIP entry's CRC-32 and sizes (in its local header, or its data descriptor) with the
  central directory's copies and each local header's offset, every PNG chunk's CRC-32 except the `IDAT` chunks' (which
  opening the image data checks), the Adler-32 after a zlib stream and the CRC-32 and length after a gzip member — once
  the encoder has checked that the decoder computes them back exactly, every level's containers and each opened stream's
  trailer named in the header (`cm::derived`); a 16-bit image, found by its byte parity and row width, becomes the
  residuals of an adaptive least-squares predictor over ten causal neighbours (`cm::ols`); x86 code, in 64 KiB blocks
  whose calls return to the same places (their targets repeat more than their operands as written, which a data table of
  call-like bytes does not do), gets its near call and jump targets (`E8`/`E9` and `0F 80..8F`) made absolute, the low
  three bytes most significant first; DEC Alpha code (an ECOFF object's code sections exactly, from its section table,
  and elsewhere the blocks its opcodes give away) gets what it computes from its own address made absolute — call
  targets, call hints, the global pointer each procedure and call site rebuilds — and each format's fields reordered so
  what decides the rest comes first (an operate's function, then its destination), every word most significant byte
  first (`cm::alpha`). The model then codes one bit at a time from order-1 to order-16, sparse, word, word-pair and
  word-triple, token (numbers read as words, and the bytes between tokens), record, line, indirect and word-successor
  contexts, the byte the match model predicts (with its length band, after the last byte, and in the order-3 context),
  the markup around text (a tag's name with the innermost open element, so a closing tag spells the element it closes,
  an attribute with its tag, the elements around the text between tags; `cm::markup`'s tracker), the prose around text
  (the words before the current one in the same sentence, the phrase it continues — words joined by single spaces — the
  current word's pattern of letter kinds with its length, the quote or bracket last opened, the keyword that opened the
  section, the paragraph (what it has held so far), the kind of byte its line began with and the column, and the groups
  of the last bytes read, live only while text is prose — a quarter of the last 64 bytes letters; `cm::prose`'s
  tracker), the line around a byte (the run of bytes this line has matched the line above at the same columns, the
  previous line's length with the column, the columns where the two lines above agree, the column with the last bytes,
  and what the line has held so far; `cm::lines`'s tracker), where a byte sits in source code (identifiers whole,
  letters, digits and underscores with their case, and the two before; the parentheses, braces and brackets open around
  them; the classes of the last tokens and the punctuation lately; a line's indentation and first identifier; whether a
  comment or a string is open; `cm::code`'s tracker), the byte that followed the word so far last time with how often it
  came again (inside prose only; `cm::completion`'s tracker), the number a line is expected to lead with (the one that
  led the line after an equal line, plus one — a record's ID after its separator — and after a line of the same shape,
  again and plus one), the text inside a PDF's text objects, contexts from an image's neighbours (a GIF's palette
  indices, which name colours, under exact neighbourhoods near and far, each image its own, and weight sets chosen by
  which neighbours match), and contexts from an instruction decoder's view of x86 code or from a DEC Alpha instruction's
  neighbours, the instructions that last wrote its registers, the registers last touched and the last six opcodes; the
  text contexts are live only after text, the image and code ones only in an image or a code block. Each context's bit
  history (its counts of zeros and ones and its last three bits in order) and last bytes share one cache line per nibble
  (a bucket told apart by a 16-bit check in the first nibble's table and by any of 255 check bytes in the second's), in
  one table every context draws on, so a context that meets many distinct histories takes the room one that meets few
  leaves; each gives four predictions, and orders 1 and 2 also learn their own probabilities directly (order 1 at a slow
  and a fast rate). The binary contexts include a chart (for each value of a bit field of the byte before last, the
  bytes that last surrounded it), the byte a record's 4-byte field predicts (the same field one record up, moved by the
  delta its bytes so far show, so offsets and addresses growing from record to record carry exactly), the place in an
  aligned 8-byte word with the bytes around it, and the bytes one and two similarity periods back (the lag whose bytes
  have differed least); outside text, eight predictors of the next byte (the last byte, the bytes a 16-bit and a 32-bit
  word back, the byte a period back, their extrapolations, the gradient) each give the distribution of their residuals
  in the current activity bucket (`cm::residual`). Beside them run ten dynamic-Markov machines — bit-level state
  machines that start as the 256 order-1 byte trees and clone a state when one incoming transition carries most of its
  traffic, so they learn their own variable-order contexts (12 bytes a state; the fast ones start over when full, at
  their own times, the slow ones keep growing). At `Level::Max` a long short-term memory network reads the bytes too
  (`cm::lstm`): two layers of 160 cells with coupled input and forget gates, predicting the next byte a bit at a time
  through 255 logistic units (one per partial byte), learning online by backpropagation through time and an Adam step
  every 20 bytes, its rate falling with the steps (by a factor of √2 after the first 20,000, to a floor a tenth of the
  first) — on a worker thread one window behind, or in place where there are no threads, with the same result — all in
  `f32` arithmetic of one fixed order, so it predicts the same bits on every machine; its probability is a mixer input
  and, in 32 bands, keys the first probability map. A two-layer mixer with fifteen weight-set selectors (among them a
  record's column over the whole record, how the partial byte agrees with the byte one record up, and the place in a
  4-byte word) combines everything, each weight set's step settling from the full learning rate toward three eighths of
  it as the set is chosen again and again (half way after 4,096 choices), six probability maps refine the result, each
  point of a map's curve stepping by the reciprocal of its own update count, up to 255 (quick while a context is young,
  settled once it is old; the last map keyed on the model's recent misses), a final mixer weighs them per partial byte
  and per band of how long the model has been right, and two calibrations per bit position — of the final probability
  and of the refinements' average, joined by the geometric mean of their probabilities of the other bit when they lean
  the same way — give the coder a 32-bit one, so a near-certain bit costs what it is worth rather than the 0.00035 bits
  a 12-bit probability allows. `Level::Fast` (orders 1–6, the word and the match model) and `Level::Max` are named in
  the header. `compress`/`compress_with`, `decompress`/`decompress_within` (which also caps the bytes the model codes),
  `cost_profile` (every modeled byte's cost, exact to the coder's: an entropy profile).
- `cm::deflate`: a DEFLATE decoder and an encoder byte-exact with zlib at every level, memory level and window (192
  reference digests from zlib 1.3 in its tests) and with gzip at every level (72 from gzip 1.12, whose block rules
  InfoZIP's zip shares); `cm::lzw`: GIF's LZW both ways, with the matches an encoder ended early (`cuts`, `encode_cut`);
  `cm::x86`: an x86 instruction-length decoder that names the field and instruction of every byte and what each
  instruction does (sixteen kinds, from the opcode maps; a group opcode's from its ModRM reg field).

### Fixed

- Every `describe` column form checks its element count against `max_elements` before expanding: `T_BYTES`,
  `T_INTS_DELTA`, `T_INTS_DOD`, `T_INTS_FOR` (a width-0 body expanded a claimed count with no bytes behind it),
  `T_INTS_DICT` (likewise for a one-entry dictionary) and `T_INTS_RLE` (capped at 2²⁸ rather than the budget) did not.

## [0.11.0] - 2026-11-02

### Added
- **Several ordered infinitesimals.** `Scale` gains its order: `Scale::ordered(name, residues, larger, smaller)`, `order`, `same_infinitesimal` (by declaration, a residue class's copy its declaration's) and `with_residues`; `tower::new_ordered_infinitesimal`. New modules: `ordered_field` (`OrderedField`, `PolyOver<K>` with gcd, Yun's square-free part, the resultant, Lagrange interpolation, Padé, `sum_poly`/`product_poly`/`power_poly` — a binomial's powers read off without a resultant), `multi` (`Multi`, a value of several in its one form, `is_integral`, `render_reciprocals`), `closure` (`AlgValue`, the real closure at the smallest infinitesimal), `expansion` (`Expansion<C>`, a lazy series over a `Coefficient` ring), `number` (`Number`, base's own `TowerValue`), `nested` (`NestedGerm`, a germ of several) and `nested_flow` (`matrix_exp`, `flow`, `power`, and `exp(N·log A)` for a count infinite in the series' own infinitesimal). `HyperForm` gains `Multi`, `Nested` and `Integer`, an infinite integer of several (`scaled_bytes` kinds 4, 5 and 6). `tower::{standard_part_relative, is_infinitesimal_relative, is_limited_relative}` read from an infinitesimal's standpoint, and `extension::Builtin::OrderedInfinitesimal` plus an optional standpoint on `StandardPart`, `IsInfinitesimal`, `IsLimited` and `InfinitelyClose`.
- **Residues of several generators.** A dependence on several is canonically one on the coarsest whose branches read the finer, and `from_depends` alone gives that form: it re-nests a dependence whose branches read a coarser residue, so `lift` maps over the left operand's classes whichever generator is coarser. `apply_limit` resolves the finer within each class of the coarser and `undecided_branch` names every generator read. The integer part of a value of several (`polar_floor`) splits it in its smallest infinitesimal and reads the larger on each class; `supertask::affine_power` and `cycle_at` take a count infinite in several, and `on_each_class` reads one generator's classes at a time.
- **Functions of the hyperreals.** `Germ::exp`, `Germ::ln` and `Germ::root(n)`: the exponential of a limited germ, the logarithm of one whose standard part is positive and the real `n`-th root, each coefficient from the ones before it; what is no germ is named (`GermError::BeyondEveryPower`, `CarriesLog`, `Nonpositive`, `NegativeRoot`). `tower::real_fn` takes the exponential and the logarithm of a hyperreal or an infinite integer, and the square root of a germ, through them, and a germ takes a rational power as the power of its root.
- **Germs of fractional orders.** A `Germ` has a ramification `q` (`ramification`): order `k` is the coefficient of `dt^(k/q)`. `Germ::from_puiseux` reads a value of the real closure on its branch's grid (`Puiseux::ramification`, `coefficient`, `order`, `rational_function`), and two germs meet on the least grid both lie on, so a root of a hyperreal meets a computable real as a germ. A flow starts from a value of fractional orders, and `Flow::on_grid` runs an update that is a polynomial in a root of `dt`; the supertask engine reads one from a step whose rate holds a root, and refuses a step with a term between `dt⁰` and `dt`.
- `ExactReal::nth_root`: exact on algebraic numbers and on a single real exponential term (`√e` is `e^(1/2)`); `sqrt` is `nth_root(2)`, and a computable real's power `x^(p/q)` is the power of its root.
- **`puiseux`, the real closure of the hyperreals.** `Closed` is a value of it in its one form — a `Hyperreal` (`Field`), or a `Puiseux` series (`Series`), a real algebraic Puiseux series in `dt` held as a squarefree polynomial over the rational functions of `dt^(1/q)` and the branch of its real roots it is, found by Newton's polygon and extended term by term on demand. `add`, `sub`, `mul`, `div`, `neg`, `recip`, `pow` and `root(n)` take a resultant and pick the root whose expansion is the operation's on the operands' expansions; `compare` is exact; a value that is a rational function of a root of `dt` is held as one (Padé reconstruction within Gauss's lemma's degree bounds, checked exactly). `render`: a rational function of a root of `dt`, a radical, or the leading terms and the order of the rest.
- **`elimination`**: `Polynomial`, sparse polynomials in several variables over ℚ, with `resultant(o, v)` interpolated one variable at a time from univariate resultants.
- `Algebraic::real_roots`: the real roots, with multiplicities, of a polynomial whose coefficients are algebraic numbers, found among the roots of its norm over ℚ and each checked exactly.
- `Hyperreal::monomial`, `ramify`, `unramify`, `laurent_coefficient`, `pade`, `render_powers` and `degrees`. `Germ` reads a rational function's coefficients through `laurent_coefficient`.
- `tower`: `HyperForm::Puiseux`, a root of a hyperreal that is no rational function of `dt`, through every operation, relation, predicate, integer part and the transport bytes (`scaled_bytes` kind `3`); `real_fn` takes the square root of a hyperreal or an infinite integer in the real closure; `exact_power`, `**` to an exact exponent, a fraction taking a root; `from_closed`.
- `extension::Builtin::ExactPower` (`exactPower`).
- **`supertask::first_exit`**: how many passes `While <guard>:` makes when the guard compares terms linear in a counter it steps, read where the guard's truth can change and on each residue class of the generator that point depends on; `−1`, the loop as written, when every number is standard. `extension::Builtin::FirstExit` (`firstExit`).
- `tower::ValueAlgebra::modulo`: the language's `%`, for the supertask runtime.
- `limit_value::Converging::joined`: the evidence of a value made from two answers.
- **`ExactReal::defined`**: a real number an `Approximant` gives — its digits in a base (`Definition::Digits`) or rationals within `2^-n` of it (`Definition::Approximations`) — read only as a refinement asks, each digit or approximation once, every answer checked against those read before it. `Definition::named`, `not_a_rational_answer`. A defined real stays with the program that defines it (`ExactReal::is_transmissible`, `Germ::is_transmissible`): `tower` gives it, and a germ holding one, no value bytes, `tower::transmissible` says whether any value of the exact types can travel, and `exact_real::not_sendable` is the sentence a program stops with when it hands a task one (`STAYS_WITH_ITS_PROGRAM`).
- `supertask::affine_power` powers an update holding a residue-dependent value on each class of the generator and joins the classes' answers, lists item by item.
- `Algebraic`: two numbers of one quadratic field add and multiply as `a + b·√d` in that field, landing in their one form as the root of `(x − a)² − b²·d`, with no resultant; numbers of two fields still meet through theirs. A thousand sums in the field of `√2` take 70 ms where they took 11 s.
- `puiseux::Closed`: two rational functions of roots of `dt` compute in their own field, on the least root both are functions of (`ramify`, then `Hyperreal`'s arithmetic, then the one form), with no resultant; `Closed::pow` answers a `PowerRefusal`, the hyperreal's refusal with the root of `dt` its degree counts in.
- `GermError::NotComputed` and `GermError::StandardOnly` carry their order as an exponent of `dt` (a `Rational`), so a germ of fractional orders names `the dt^(3/2) term`. A flow whose update terms do not commute computes every order below its exponent's first nonzero term, not only its lowest.
- A germ prints by the rule the real closure's expansions print by: a coefficient that is a quotient in parentheses with its sign outside (`-(1/2)·dt^2`), a pole as `e/dt`, and the rest from the standard order on as `O(1)`.
- `tower::TowerValue::exact_rational` (an integer or a rational, never a float) and `tower::ValueAlgebra::power` (the language's `**`) are required of a value model.
- `Hypernatural::from_polynomial` reads an integer polynomial at once and any other by its residues, where it evaluated `deg + 1` points of huge integers: `1/dt**1000` took 14 s and now 0.05 s.
- A power of a root of a hyperreal past degree 4,096 (`(1 + √dt) ** 5000`) panicked in the resultant that took it; it is refused, its degree counted in the root it is a function of (`the power would have degree 5000 in √dt; one power reaches at most degree 4,096`).
- `tower::carry_evidence` and `tower::provisional` join an answer's evidence to the one a value already carries (`Converging::joined`) instead of wrapping one answer in another.
- An `ExactReal` keeps the best enclosure it has computed, so a refinement to a precision it has reached reads it back: a product of general reals nested `n` deep refines each factor once per precision, not `2^n` times.
- **`gf256`: the field GF(2⁸).** Byte arithmetic under the primitive polynomial `0x11d` (`add`, `sub`, `mul`, `div`, `inv`, `pow`, and the bulk forms `mul_table` and `mul_add_region`) and exact linear algebra over it: `invert`, `matmul`, `matvec`, `rank`, `solve` (a particular solution of `A x = b`) and `nullspace` (a basis of `{x : A x = 0}`). One implementation serves the Reed-Solomon codes and `logicaffeine-proof`'s Rainbow signatures and XL solver.
- **`fec::ReedSolomon`: a systematic MDS erasure code.** `ReedSolomon::new(data_shards, parity_shards)` builds the generator once from a Vandermonde matrix over `gf256`; `encode` returns the `k` data shards unchanged followed by the parity shards, and `reconstruct` rebuilds every shard exactly from any `k` of the `n` (`data_shards`, `parity_shards`, `total_shards`). Re-exported at the crate root; the `reedSolomonEncode` and `reedSolomonRecover` builtins run on it. `fec_language`.
- **`si`: dimensioned `f64` newtypes.** `Volts`, `Seconds`, `Nanoseconds`, `Hertz`, `Siemens`, `Kelvin`, `Watts`, `PerKelvin` and `VoltsPerRootHz`, each with `dimension()` returning its exact `Dimension`, rational exponents included (`VoltsPerRootHz` is V·Hz^(−1/2)); the payload stays in the named unit, with no hidden scale. `analog_dimension_lock`.
- **`ComplexF`: a floating-point complex number.** `new`, `from_f64`, `re`, `im`, `is_zero`, `is_real`, `conjugate`, `norm_sqr`, `abs`, `negated`, `add`, `sub`, `mul`, `div` and `recip` (`None` at zero), `sqrt` and integer `pow`, beside the exact `Complex`; re-exported at the crate root. `complexf_arith`.
- **`temporal::date_add_span`.** Calendar-span arithmetic on a day number: months first, clamped to the end of the month with leap years respected, then days, wrapping at `i32`. It is the one implementation of `Date + Span` on every tier. `date_span_all_tiers`.
- **`BigInt` operations.** `to_u64`, `bit_len`, `gcd`, `shl_bits`, `shr_bits` (`None` for a negative value), `bitand_natural`/`bitor_natural`/`bitxor_natural` (`None` if either operand is negative), an in-place `mul_assign_i64` that grows the existing magnitude buffer instead of allocating a product, and `clone_payload_bytes`. `perf_pass_bignum_inplace_lock`.
- **Electrical units by name.** `quantity::units::by_name` resolves `volt`/`volts`, `millivolt`/`mv`, `kilovolt`/`kv`, `ampere`/`amp` (and plurals), `milliampere`/`milliamp`, `coulomb`, `ampere hour`/`ah`, `milliampere hour`/`mah`, `ohm`/`ω` and `millisiemens`, so `5 volts` is a quantity literal; the bare `v` and `a` stay unassigned.
- **Words of every width.** `WordVal::{W8, W16}` beside `W32` and `W64`, with `WordVal::WIDTHS`, `type_name`, `div` and `rem`, and `from_u64` for all four widths; `LanesVal::bitxor` and `LanesVal::rotl` answer `L4W64`. `word_narrow_all_tiers`.
- `Symbol` implements `PartialOrd` and `Ord`.
- **`poly`: exact univariate polynomials over ℚ.** `Poly` with ring operations, Euclidean `div_rem`, monic `gcd`, `derivative`, `compose`, `squarefree_part` and `primitive_int`; real-root counting on any half-open interval by Sturm chains and isolation of every real root into a `RootInterval`; `resultant`; and `factor_over_q`, the factorization over ℤ (Berlekamp modulo a small prime, Hensel lifting past the Mignotte bound, recombination), so `x^4 + 1` is irreducible though it splits modulo every prime.
- **`algebraic`: real algebraic numbers.** `Algebraic` is a root of its minimal polynomial, named by the polynomial and the root's index, with exact field operations (built from resultants), `sqrt`, `nth_root`, a decided total order, `floor`, `ceil` and a correctly rounded `to_f64`; it prints as `√2`, `(1 + √5)/2` or `root 1 of x^3 - x - 1`.
- **`exact_real`: computable reals.** `ExactReal` evaluates `π`, `e`, `exp`, `log`, `sin`, `cos`, `sqrt` and the field operations to any precision through rigorous dyadic enclosures. Equality is decided exactly on the algebraic numbers, on sums of exponentials of algebraic numbers (by the Lindemann–Weierstrass theorem, so `sin²(1) + cos²(1) = 1` is proved, not estimated) and on polynomials in `π`; anything else is compared by refinement under a budget, and two values that agree to it fail with `RealError::Inseparable` rather than being called equal. `compare_within` is total.
- **`hyperreal`: the infinitesimal `dt`.** `Hyperreal` is the exact field of rational functions of `dt` with real algebraic coefficients, ordered by the transfer principle, with `valuation`, `standard_part`, `is_infinitesimal`, `is_limited` and `infinitely_close`; it prints as `3 + 2·dt` or `1/dt`. `principal_part` splits off the Laurent principal part at `dt = 0` with an exact limited rest, `from_reciprocal_powers`/`as_reciprocal_powers` move between a polynomial in `1/dt` and its coefficients with no polynomial gcd, and `pow` refuses a power past degree 4,096 (`MAX_POWER_DEGREE`, `PowerError`) before doing any work.
- **`hypernatural`: infinite integers.** `Hypernatural` is an integer polynomial in the infinite generator `H = 1/dt`, one per residue class of `H`, with `rem`, `scaled_floor`, `scaled_ceil` and comparison: the trip counts of loops that run infinitely many times. `rem` reads a residue from the period Kummer's theorem gives each binomial term (so `H^64 mod 2` reads two classes and `C(H, 25) mod 2` the thirty-two Lucas's theorem names), along the classes the type admits; `floor_of`/`ceil_of` take the integer part of any hyperreal whose principal part is rational, `from_polynomial`/`from_branches` admit exactly the polynomials that are integers on their classes, and `pow` squares its way up under the same degree bound.
- **`limit_value`: what a supertask leaves.** `ResidueType` records what is known about the residues of `H`; `Depends<T>` holds a value that depends on them (`0 when ⌈1/dt⌉ is even, 1 when it is odd`) and resolves it under a `LimitRule` (the standard part, the limit, the limit superior and inferior, the Cesàro mean); `Converging<T>` is a settled value or a provisional one with the cases it checked. One value is split over at most 65,536 residue classes (`MAX_MODULUS`, the wire's collection bound): `from_allowed`, `and`, `from_fn` and `zip_with` refuse a finer system with `ResidueError::TooFine` before building it, `allows` answers any modulus by the Chinese remainder theorem, and a dependence decodes in linear time.
- **`ordinal`: ordinals below ε₀.** `Ordinal` in Cantor normal form with ordinal addition, multiplication and exponentiation (`1 + ω = ω`, `2^ω = ω`), successor and limit tests, and the Cantor-normal-form order.
- **`germ`: the values flows leave.** `Germ` is a lazy Laurent series in `dt` with `ExactReal` coefficients: rational functions of `dt`, constants, field operations, and the components of a `Flow`, the `T/dt + δ` Euler steps of `A(dt) = I + dt·A₁ + dt²·A₂ + …` taken as `exp(N·log A(dt))·x₀`. Its standard part is `e^{T·A₁}·x₀`; every coefficient is known when the `Aₖ` commute, and a non-commuting flow refuses an order past its first (`GermError::NotComputed`). `render` prints `e - (1/2)·e·dt + O(dt^2)`; a sign is decided within the first 64 orders or refused (`GermError::Flat`).
- **`matrix_exp`: exact matrix exponentials.** `characteristic_polynomial` (Faddeev–LeVerrier) and `exp_matrix(M, t)`, which interpolates `e^{t·x}` on the eigenvalues to their multiplicities through a confluent Vandermonde system over the complex algebraic numbers, so each entry is an exponential sum: `e^{[[0, 1], [−1, 0]]}` is `[[cos(1), sin(1)], [−sin(1), cos(1)]]`.
- **`complex_algebraic`: complex roots, exactly.** `ComplexAlgebraic` (`re + im·i`, real algebraic parts) with field operations, `conj`, `pow` and `eval`; `complex_roots(p)` gives every root of a rational polynomial once each, found through the resultants whose roots are the sums and the differences of roots, each accepted only once it checks exactly.
- **`ode`: validated integration.** `flow(field, start, t, q, cap)` encloses the solution of a polynomial differential equation `x' = f(x)` (`Field`, `Monomial`) from a box of starts: an a priori box closed by Picard's operator, a Taylor model from the coefficients' Cauchy products, and Lagrange's remainder over the box, so the enclosure always contains the solution; `FlowError::Escapes` when no box closes above the smallest step. `escape_time_bound(p, x₀, cap)` bounds from above the time a scalar `x' = p(x)` takes to leave every bound. `ExactReal::flow(field, start, t)` is that solution as computable reals, one per component, sharing one integration and encoded by their field, start and time; `Germ::standard` holds a value known by its standard part and `Germ::unlimited` one beyond every bound with its sign, which dominates any limited value it is added to.
- **`tower` and `supertask`: one semantic core for every value model.** How the exact types mix with the numeric tower and with each other (canonical smallest forms, lifting over residue branches and through provisional values, germs, every refusal's wording), and the supertask runtime's algorithms (`trip_count`, `affine_power` with `check_annihilator`, `polynomial_flow`, `cycle_at`, `state_size`, `search_candidates`), generic over `TowerValue` and `ValueAlgebra`. The interpreter's runtime value implements both, and a compiled program's value does the same, so every tier runs one implementation. `Scale` is held in an `Arc`, so an exact value can cross a task.
- **`extension`: the extension's builtins, read and run once.** `Builtin` names the builtins of the exact extension as a program calls them (`Builtin::named`, `Builtin::name`, `Builtin::ALL`), and `call(builtin, args)` decodes their arguments and runs them on any `ValueAlgebra`, with each refusal's wording ("an infinitesimal's name is Text, got Int", "a limit rule is named by Text, got Int", …). `TowerValue` gains `text`, a `Text` value's characters. `tower::arith` leaves `+` with a `Text` operand to the value model, so text joins an exact value as it prints, as it joins every other value. Unit bodies: `unit/logicaffeine_base/extension/tests.rs`.
- **`hybrid`: the ideal run of a loop that flows between jumps.** `run(model, start, span)` follows a `Model` (the body in order as `Item`s, runs of flow statements by each slot's rate and jump checks, and each `Jump`'s guard and statements over `MPoly`s in the slots) in real algebraic numbers: flights from a terminating Lie series, events at the exact roots of each guard atom along a flight (`real_roots`, through a free norm for coefficients that are not rational), jumps in three-valued logic that decides values infinitely close to each other only by exactness or a sign the guard or a reset implies, tails of a cycle of firings that a `Scaling` maps round to round (every later value a polynomial in `z = sᵏ` plus a count, the firings accumulating at a geometric sum for `s < 1`; `check_scaling` is the certificate checked on its own), and continuous firing, in which a jump that fires again at once holds what it sets while the rest flows. Each `Cell` it leaves says whether the loop's own value is exactly it; `EVENT_BUDGET` bounds the firings followed one by one. `supertask::hybrid_flow` runs a model the engine encodes on any `ValueAlgebra`, each residue class of the step count apart, a finite count step by step, and `extension::Builtin::HybridFlow` (`hybridFlow`) calls it.
- **The exact constructors.** `tower::real_fn(RealFn::{Sqrt, Exp, Ln}, v)` takes the square root, the exponential or the natural logarithm of a standard number, an algebraic number or a computable real exactly, held in its smallest exact form (`sqrt(4)` is `2`, `sqrt(2)` is `√2`, `ln(e)` is `1`), lifted over residue branches and keeping a provisional answer's evidence, and refusing every function of an ordinal ("the exponential of ω has no exact form"); `tower::pi`, `tower::euler` and `tower::omega` are π, e and ω. `Builtin::{ExactSqrt, ExactExp, ExactLn, ExactPi, ExactE, ExactOmega}` (`exactSqrt`, …, `exactOmega`) run them through `call`.
- **`numeric::float_min` and `numeric::float_max`**: the language's `min` and `max` of two reals — a NaN gives way to the other operand, and of two equal values the one with the negative sign (`min`) or the positive sign (`max`) — one answer under every optimizer, where `f64::min` leaves the sign of two equal zeros open.
- **A hyperreal quotient with a product below the line prints in parentheses.** `1/(2·dt)` printed as `1/2·dt`, which reads as `(1/2)·dt`; a denominator that is a product or a sum is now written in parentheses. `hyperreal::tests::a_product_in_the_denominator_reads_as_the_denominator`.
- **`numeric::rational_to_f64_nearest`.** The nearest `f64` to a `Rational`, ties to even, rounded once from the exact value, subnormals and overflow included; `Rational::to_f64` divides two rounded doubles.

### Breaking changes
- **`WordVal` gains the variants `W8` and `W16`.** An exhaustive `match` over `WordVal` no longer compiles. Migration: add the two arms, or a wildcard.

### Changed

- Licensed under the MIT License, which replaces the Business Source License 1.1, and the package ships it as `LICENSE.md`.

- **`BigInt` multiplies by Karatsuba and divides by Knuth's Algorithm D.** A product whose smaller operand has 32 limbs or more takes Karatsuba; division normalizes the divisor, estimates each quotient limb from the top two limbs of the running remainder and corrects it, where it went a bit at a time. Results are bit-identical: the schoolbook and bitwise algorithms remain as the test oracles each is checked against.

### Fixed
- Arena values now run their destructors when the arena is dropped or reset, releasing owned vectors and strings. Typed storage keeps references stable, retains capacity across resets, and cleans up initialized values if an iterator or destructor panics.
- **`BigInt::to_f64` is the nearest `f64`, ties to even.** It rounded once per limb (Horner over 64-bit limbs), which misses the nearest float for 1.5% of random 64–200-bit integers: (2^53+1)·2^64+1 gave 2^117 instead of 2^117+2^65. It now rounds the top 64 significant bits with a sticky bit, once, and scales exactly, reaching infinity exactly where the exact value's rounding does.
- **`Word8` and `Word16` wrap at their own width on the interpreted tiers.** `WordVal` carried only 32- and 64-bit words, so `word16(65535) + word16(3)` showed 65538 on the tree-walker and the VM and 2 in a compiled binary. `word_narrow_all_tiers`.
- **A small `describe` descriptor cannot demand unbounded work.** The int-column decoder (`decode_int_seq`, `decode_int_column_body`), which the wire codec runs on frames it receives, bounded the element count but not the work per element, so a descriptor of a few bytes could ask for an LFSR or FCSR keystream or a `GenExpr` column that took minutes to expand. Each generator now charges its work against the column tree's `max_elements` budget, and a tree over budget is refused before it is expanded.

## [0.10.0] - 2026-07-08

Synced to workspace version 0.10.0. See root CHANGELOG for full history.

## [0.8.12] - 2026-02-14

Synced to workspace version 0.8.12. See root CHANGELOG for full history.

## [0.6.0] - 2026-01-17

Initial crates.io release.

### Added

- Arena allocation via bumpalo with stable AST references
- Symbol interning with O(1) equality comparison
- Span tracking for error reporting with merge support
- SpannedError base type with source locations
- WASM-compatible design (no platform-specific code)
