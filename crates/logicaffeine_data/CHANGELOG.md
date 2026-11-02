# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.11.0] - 2026-11-02

### Added

- `LogosOpt<T>`: a value or `nothing`, the storage a compiled place takes where a program can store `nothing`
  in it; `held`, `held_mut` and `into_held` stop in the words they are given, `pair` reads two operands and stops
  naming which is `nothing`, `equals_with`, `Truthy`, and `Default` holding the type's own first value.
- `failure::field_access_on` and `failure::SET_FIELD_ON_NON_RECORD`.

- `LogosValue` holds the values of several ordered infinitesimals, their germs, their dependences on several residues and their infinite integers through the tower's own rules, and answers `TowerValue::is_nothing`.
- **`LogosRational` takes the VM's rules for an exact number.** Each delegates to `LogosValue`'s rule, so a whole value takes the integer's and a refusal is the interpreter's sentence:
  - `quotient`, `remainder`, `floor_quotient` and `power`;
  - `bit_and`, `bit_or`, `bit_xor`, `shl` and `shr`, each answering a machine integer;
  - `to_f64` and `order_real`, the exact order against a real;
  - `integer(past_word, fraction)`, where a machine integer is taken.
- `ToRationals`: a value of integers held where rationals are, an integer as the rational it equals, a list item by item at any depth. `From<LogosInt> for LogosRational`; a placeholder `Default` for `#[derive(Default)]`.
- `LogosValue::slice` and `logos_rt_value_slice`: `items start through end`, under `logos_slice_range`.
- `LogosValue::defined_real` and `LogosValue::define_real`: the real number a compiled function defines, read on any thread. `logos_rt_value_defined_real` and `abi::Definer`: one a C program or a linked module defines, read through a function the program hands the runtime. `LogosValue::sendable` and `logos_rt_value_sendable`: whether a value may cross to a task, refused in the interpreter's words for one holding a defined real.
- `LogosValue::integer_at` and `logos_rt_value_int_at`: the `Int` a value is where a machine integer is taken, refused in the place's own sentences.
- `LogosValue::into_real`.
- `LogosValue` implements `ValueAlgebra::modulo` by its own `%`.
- `LogosValue` implements `TowerValue::exact_rational` and `ValueAlgebra::power` by its own `**`, so the extension's `exactPower` computes on it.
- **`LogosBits`: a `Seq of Bool` one bit per item.** `new`, `with_capacity`, `filled(len, value)`, `push`, `len`, `item` and `set_item` (counted from 1, from the back when negative, failing past the end with `failure::index_out_of_bounds`), the unchecked `get_unchecked`, `set_unchecked` and `set_true_unchecked` at a 0-based offset, `iter`, and equality by items. The representation a compiled program gives a local sequence of flags (`Opt::BitPack`). The strided fills `fill_stride_unchecked` and `fill_stride_through_unchecked` store one value at `first`, `first + step`, … (`count` items, or up to the offset `through`, returning the steps past `first`): a step of a word or more, or fewer than eight items, store in place, and a shorter step writes each word it crosses once, the multiples of `step` below 64 shifted to where the progression enters the word (`fill_stride_words`, out of line). `fill_stride_through_unchecked` is out of line, so a loop that calls it keeps none of the progression's arithmetic.
- **`failure::CALL_DEPTH_EXCEEDED`**: the reason a program stops when its calls nest deeper than its stack holds, the one sentence the interpreter, the VM and a compiled program report for it.
- **Exact bit operations: `logos_band_exact` and `logos_word_operand`.** `logos_band_exact(a, b)` is `a & b` on two `LogosInt`-convertible values as every tier computes it: two words AND bit by bit, and a big integer that is not negative, beside a low mask `2^k - 1`, leaves its low `k` bits; any other big operand panics with `failure::BITAND_OPERANDS`. `logos_word_operand(a, why)` is an operand of `|`, `^`, `<<` or `>>`: its word, or a panic in `why`'s words. `failure::{BITAND_OPERANDS, BITOR_OPERANDS, BITXOR_OPERANDS, SHL_OPERANDS, SHR_OPERANDS}` are those words, shared with `semantics::arith`. `exact_bitwise_lock`.
- **`LogosItems::logos_item_front` and `logos_item_front_mut`.** The item a 1-based index names when the caller has proven the index is at least 1: one compare, and the past-the-end failure. The generated Rust reads and writes through it wherever the index is proven to count from the front, so a loop whose indices are proven in range compiles to plain loads (spectral_norm's inner loop vectorizes its divides again).
- **`failure::INT_TOO_WIDE`**: the reason a compiled program stops when an integer past a machine word reaches a place it holds one.
- **`SharedMap` converges: `crdt::LatestWrite<T>`.** A last-writer register stamped with the Lamport time of the map that holds it (`CausalContext::observed`, `VClock::total`) and merged by `(stamp, value)`: commutative, associative, idempotent, and independent of replica ids and wall clocks. `ORMap<K, LatestWrite<T>>` gains `put`, `value`, `LogosIndex`/`LogosIndexMut` (a missing key traps with the interpreter's message) and owned iteration in key order, and every `ORMap` gains `sorted_entries`, `PartialEq` (equal convergent state) and `LogosContains`. `SharedMap from K to V` compiles to it; the v0.10.1 form, `ORMap<String, i64>`, did not build. `shared_map_all_tiers`, `phase_crdt_variants`.
- **Owned iteration over `ORSet`, `RGA` and `YATA`.** A set iterates sorted and a sequence in its convergent order, so `Repeat for` walks a shared set or sequence on AOT-Rust.
- **Tuples hold collections and declared types.** `Value::Seq` (a boxed homogeneous sequence, built by `From<LogosSeq<T>>`); `Value::Boxed` with the `Boxable` trait and `LogosBoxed` (a record or variant the program declares, rendered by its own `Show`); the `LogosTuple` newtype (shows `(a, b)`, distinct from a list when nested, with 1-based `LogosIndex`/`LogosIndexMut`); and the unboxers `Value::{logos_i64, logos_f64, logos_bool, logos_string, logos_char, logos_boxed, logos_seq}` with `logos_seq_i64` through `logos_seq_seq_string`. `tuple_of_records_all_tiers`, `c_seq_of_tuple_lock`.
- **`failure::{ContractReason, contract_reason}`.** The report a broken `…, otherwise <reason>` contract gives on the compiled tiers: a Text reason is itself, and an `Int`, `Real`, `Bool` or `Char` reason is refused in the one sentence every engine uses ("The reason after `otherwise` must be Text, but it was Int."); a type with no implementation does not compile. `error_fail_and_contracts`.
- **Runtime values for compiled programs.** `LogosComplexF` (over `logicaffeine_base::ComplexF`); `LogosComplex::{conjugate, re, im, abs_f64}`; the `+ - * /` operators on `LogosRational`; `Default` for `LogosInt`, `LogosDecimal`, `LogosComplex`, `LogosModular`, `LogosUuid`, `LogosMoney` and `LogosQuantity`, so a record holding one can derive `Default`; `LogosInt::mul_assign_i64`, which grows a big accumulator in place; `LogosStrPushInt::push_i64`, integer append without the formatter; and `VecCowNoop`, a no-op `cow()` on an owned `Vec`, so a `Seq of Real` lowered to `Vec<f64>` builds. `complex_seq_aot_lock`, `perf_pass_bignum_inplace_lock`, `string_itoa_fastpath`, `seq_real_tier_lock`.
- **`backpressure::BoundedRing<T>`.** A fixed-capacity FIFO that evicts and returns the oldest item on overflow and counts every eviction in `dropped`. It is a standalone primitive: no relay path uses it.
- **The one index rule.** `logos_offset(index, len)` is the 0-based offset a 1-based LOGOS index names (a negative index counts from the back; 0 and anything past either end name nothing), in 64-bit arithmetic on every target; `logos_slice_range` is a slice's range under it, `logos_run_through`/`logos_run_before` the items a counted loop reads and where it stops, `logos_text_run_through`/`logos_text_run_before` the same over a Text's bytes, and `LogosItems` the read, write, slice, run and reverse accessors generated programs index through. `statement_tier_totality_lock`.
- **Every runtime failure's sentence, in `failure`.** `index_out_of_bounds`, `key_not_found`, `MODULO_BY_ZERO`, `invalid_utf8`, the quantity, money and modular sentences, `not_a_decimal`, `decimal_needs_text_or_int`, `invalid_uuid`, `uuid_needs_16_bytes`, `not_a_timestamp`, `unknown_time_zone` and the file sentences: the engines report these strings and generated programs panic with them. `statement_tier_totality_lock`.
- **`decimal_of` and `DecimalArg`.** `decimal(x)` as one definition over the argument's Rust type: Text is read (surrounding whitespace aside), an Int or a `LogosInt` widens exactly, a Decimal is itself, and an `f64`, `bool` or `char` stops with `decimal_needs_text_or_int`.
- **`exact`: the exact extension in compiled code.** `LogosValue` is the value a compiled program holds wherever the exact extension reaches: the standard numbers, booleans, text, nothing, lists and tuples, the exact types (algebraic numbers, computable reals, hyperreals, infinite integers, ordinals), residue-dependent and provisional values, and a kind the tower does not mix with through `OtherValue` (a date, a moment, a time of day, a span). It keeps every value in its canonical form, so a binding that holds an `Int` still divides, promotes to a big integer and prints as one, and it implements `logicaffeine_base::tower::{TowerValue, ValueAlgebra}`, so the interpreter's own glue and supertask algorithms run on it. Its operations are the interpreter's, sentence for sentence: `+ − × / // %` and powers, the relations (`Rel`), `not`, the bit operations, `is approximately`, `followed by`, `floor`/`ceil`/`round`/`abs`/`sqrt`/`min`/`max`/`pow`, and a collection's `item`, `length_of`, `contains_value`, `items`, `push_value`, `set_item` and `pop_value`. `logos_extension(ExtensionBuiltin, args)` calls a builtin of the extension through `logicaffeine_base::extension::call`. `LogosValue` is `Truthy` (a residue-dependent condition stops with the interpreter's sentence), `LogosIndex<i64>` and `LogosIndex<LogosValue>`, and converts from every machine integer, `f64`, `bool`, text, `()`, `LogosInt`, `LogosRational`, `LogosDecimal`, `LogosSeq<T>` and `Vec<T>`. `exact_value_model_parity` holds it to the interpreter's values over every operation.
- **`failure::answered`.** The value an operation of the exact extension answered, or a stop with its refusal, the sentence the interpreter reports.

- **`failure::pop_from_empty`**: the sentence a program stops with when it pops a list with no items, naming the list as the program writes it (`Cannot pop from xs: it is empty`).
- **`logos_min_f64` and `logos_max_f64`**: base's `float_min`/`float_max`, which `LogosValue::min`/`max` and a compiled program's `min`/`max` of reals call, so the sign of equal zeros and a NaN are decided as the interpreter decides them.
- **`abi` (feature `c-abi`): the C ABI over `LogosValue`.** The `logos_rt_value_*` functions a program compiled to C, or to linked WebAssembly, calls where the exact extension reaches it: constructors (an integer, a real, a boolean, nothing, text, an exact number from its digits, a decimal, a date, an instant, a time of day, a span, a list, a tuple), every operator and relation, the lifting builtins, `call` for a builtin of the extension by name, `item`/`item_at`/`length`/`contains`/`items`/`truth`/`int_of`, the in-place `push`/`set_item`/`pop`, `show` and `text_free`, `clone` and `drop`. An operation that has no answer returns a null handle or a status of 0 and keeps the interpreter's sentence for `logos_rt_value_error`; a panic is caught and kept the same way (`logos_rt_value_init` keeps the runtime from printing one). `Counting` is the allocator that counts the bytes the runtime holds (`logos_rt_value_heap_bytes`), `logos_rt_value_live` the handles not yet freed. A runtime that keeps numbers in Rust hands them across as values with `of_bigint`, `of_rational` and `of_decimal` (base's `BigInt`, `Rational` and `Decimal` behind the pointer), `tuple_of` makes a tuple of a list's items, and `apply(index, args)` calls the builtin of the extension at `index` of `extension::Builtin::ALL` on a list's items. Built for `wasm32`, a panic ends the module instead of being caught, since that target does not unwind.
- **`temporal`: the temporal values of a compiled program**, moved here from `logicaffeine-system` so every value model reaches them: `LogosDate`, `LogosMoment`, `LogosTime` and `LogosSpan`, their arithmetic and display, and each as an `OtherValue`.
- **`Default for LogosValue`**: `Nothing`, the value the interpreter starts an unset slot of an exact type at, so a record holding one derives `Default`.

### Breaking changes
- **`LogosItems` gains `logos_item_front` and `logos_item_front_mut`.** An implementation of the trait outside this crate no longer compiles. Migration: implement both as `logos_item`/`logos_item_mut` restricted to an index of at least 1.
- **`Value` gains the variants `Seq` and `Boxed`.** An exhaustive `match` over `Value` no longer compiles. Migration: add the two arms, or a wildcard.
- **`text_from_bytes` panics on invalid UTF-8.** It returned U+FFFD replacement text where the tree-walker and the VM fail; it now panics with their sentence (`text_from_bytes(): invalid UTF-8`), so `fromHex("ff")` fails on AOT-Rust as it does interpreted. `wasm_aot_lock`. Migration: test the bytes with `validUtf8` before decoding them, or repair them first with `toValidUTF8Bytes(bytes, replacement)`.

### Changed

- Licensed under the MIT License, which replaces the Business Source License 1.1, and the package ships it as `LICENSE.md`.
- `wire::canonical_f64`, `canonical_f64_bits`, `put_f64`, `put_f64s`: a NaN on the wire is `0x7FF8_0000_0000_0000`;
  `put_f64s` writes a float column in one pass (four lanes at a time with AVX2).

- **`push_i64` writes straight into the string's spare capacity** (`LogosStrPushInt`): the digit count from `ilog10`, two digits per division by 100 from a pair table, no stack buffer and no `push_str` copy.
- **The Int tables' hot paths are inline and their growth is cold**: `insert`, `get` and `contains_key` of `LogosI64Map`, `LogosI64Set`, `LogosI32Map` and `LogosI32Set` are `#[inline]`, and `grow` is `#[cold]` and `#[inline(never)]`.
- **`logos_cold_path`**: runs a closure out of line (`#[cold]`, `#[inline(never)]`), the branch a guard sends a computation to when its machine-word form may not hold. Generated code's exact fallbacks run through it.
- **The Int tables slot a key by the top bits of its Fibonacci product** (`fib_slot`): `LogosI64Map`, `LogosI64Set`, `LogosI32Map` and `LogosI32Set` keep `shift = 64 - log2(slots)` and take `(key * 2^64/phi) >> shift`, where they took the low bits after an xor-shift.
- **`LogosI64Map`, `LogosI64Set`, `LogosI32Map` and `LogosI32Set` stay at most half full** (one `INT_TABLE_MAX_LOAD` for all four, where each grew at three-quarters): a linear-probing miss walks (1 + 1/(1 - α)²) / 2 slots.
- **`LogosInt` arithmetic borrows its big operands.** `add`, `sub` and `mul` no longer clone a heap magnitude per operation. `perf_pass_bignum_inplace_lock`.

## [0.10.0] - 2026-07-08

Synced to workspace version 0.10.0. See root CHANGELOG for full history.

## [0.8.12] - 2026-02-14

Synced to workspace version 0.8.12. See root CHANGELOG for full history.

## [0.6.0] - 2026-01-17

Initial crates.io release.

### Added

- WASM-safe data structures (no IO dependencies)
- CRDT implementations: GCounter, PNCounter, LWWRegister, MVRegister
- Complex CRDTs: ORSet (with AddWins/RemoveWins bias), ORMap
- Sequence CRDTs: RGA, YATA
- Vector clocks and Dot contexts for causal ordering
- Delta-state synchronization via DeltaCrdt trait
- LogosIndex trait with 1-based indexing convention
- LogosContains trait for unified membership checking
- Lamport Invariant: no path to system IO
