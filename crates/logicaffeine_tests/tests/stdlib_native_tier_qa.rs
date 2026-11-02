//! Native-tier QA sweep: representative functions from each stdlib domain that claims `aot_c` /
//! `aot_native`, exercised on the C and AOT-Rust tiers that `assert_all_tiers` (tw≡VM only) never
//! touches. The campaign's recurring anti-pattern is a cap claiming a native tier it was never
//! actually compiled on; this pins one program per under-locked domain so a codegen divergence goes
//! RED instead of hiding behind a tw/vm-only differential. A failure here is a real bug (fix the
//! codegen) or a dishonest tier claim (correct the catalog) — never something to silence.

#![allow(unused_imports)]
mod common;
use common::{assert_c_output, assert_compiled_equals_interpreted};

// Two tests per domain: AOT-Rust (compiled ≡ interpreted) and C (gcc output-exact).
macro_rules! native_domain {
    ($name:ident, $src:expr, $out:expr) => {
        mod $name {
            use super::*;
            #[test]
            fn aot() {
                assert_compiled_equals_interpreted($src);
            }
            #[test]
            fn c() {
                assert_c_output($src, $out);
            }
        }
    };
}

// AOT-Rust ONLY — the cap legitimately claims `aot_native` but NOT `aot_c` (the C backend is raw
// int64: arbitrary-precision bignum and the complex struct are documented N/A there). We still verify
// the native-Rust tier that `assert_all_tiers` never runs.
macro_rules! native_aot_only {
    ($name:ident, $src:expr) => {
        mod $name {
            use super::*;
            #[test]
            fn aot() {
                assert_compiled_equals_interpreted($src);
            }
        }
    };
}

// C tier VERIFIED + AOT-Rust tier TRACKED-AS-PENDING. The C output is asserted (the `c` test runs);
// a matching `aot` test EXISTS but is `#[ignore]`d with the exact reason it can't pass yet — so the
// Rust tier is NEVER silently skipped for a value-type capability the C tier covers. Un-ignore the
// instant the AOT gap is fixed to lock both tiers together. This is the symmetric-coverage guarantee:
// every value-type test below has BOTH a `::c` and an `::aot`, so a tier can't be missed.
macro_rules! native_c_aot_pending {
    ($name:ident, $src:expr, $out:expr, $why:expr) => {
        mod $name {
            use super::*;
            #[test]
            fn c() {
                assert_c_output($src, $out);
            }
            #[test]
            #[ignore = $why]
            fn aot() {
                assert_compiled_equals_interpreted($src);
            }
        }
    };
}

native_domain!(
    bytes_dom,
    "## Main\nShow bytesToUpper([104, 105, 33]).\n",
    "[72, 73, 33]"
);
native_domain!(
    statistics_dom,
    "## Main\nShow floor(meanReal([1.0, 2.0, 3.0, 4.0]) * 100.0).\n",
    "250"
);
native_domain!(
    operator_dom,
    "## Main\nShow opAdd(3, 4).\nShow opMul(6, 7).\n",
    "7\n42"
);
native_domain!(
    calendar_dom,
    "## Main\nShow isLeapYear(2000).\nShow daysInMonth(2024, 2).\n",
    "true\n29"
);
native_domain!(
    chars_dom,
    "## Main\nShow charIsDigit(53).\nShow charIsDigit(65).\n",
    "true\nfalse"
);
native_domain!(
    collections_dom,
    "## Main\nShow counterTotal(countInts([1, 2, 2, 3, 2, 1])).\n",
    "6"
);
native_domain!(
    builder_dom,
    "## Main\nShow readerReadByte(newReader([65, 66, 67])).\n",
    "65"
);
native_domain!(
    convert_dom,
    "## Main\nShow toBinary(5).\nShow toHexInt(255).\n",
    "101\nff"
);

// Wave 2 — complex-codegen shapes (Seq slicing/algebra/scan + float ops) on both native tiers.
native_domain!(
    bytes_trim,
    "## Main\nShow bytesTrimSpace([32, 9, 65, 66, 10]).\nShow bytesTrimSpace([65]).\n",
    "[65, 66]\n[65]"
);
native_domain!(
    iter_take,
    "## Main\nLet t be takeSeq(2, [10, 20, 30]).\nShow length of t.\nShow item 2 of t.\n",
    "2\n20"
);
native_domain!(
    iter_contains,
    "## Main\nShow containsSeq(20, [10, 20, 30]).\nShow containsSeq(99, [10, 20, 30]).\n",
    "true\nfalse"
);
native_domain!(setalg_union, "## Main\nLet r be seqUnion([1, 2, 3], [3, 4, 5]).\nShow length of r.\nShow isSubset([1, 2], [1, 2, 3]).\n", "5\ntrue");
native_domain!(
    scan_prefix,
    "## Main\nLet r be scanSeq([1, 2, 3, 4]).\nShow item 2 of r.\nShow item 4 of r.\n",
    "3\n10"
);
native_domain!(
    real_ops,
    "## Main\nShow fmodReal(7.0, 3.0).\nShow distReal(0.0, 0.0, 3.0, 4.0).\n",
    "1\n5"
);

// bignum now works on the C backend too — a provably-overflowing literal product promotes to the exact
// tagged LInt (int64 fast path + LBig on overflow), byte-identical to the interpreter's LogosInt.
native_domain!(
    bignum_mult,
    "## Main\nLet a be 10000000000 * 10000000000.\nShow a + 123.\nShow a - 1.\nShow 6 * 7.\n",
    "100000000000000000123\n99999999999999999999\n42"
);
native_domain!(
    bignum_divmodneg,
    "## Main\nLet a be 10000000000 * 10000000000.\nShow a / 7.\nShow a mod 7.\nShow 0 - a.\n",
    "14285714285714285714\n2\n-100000000000000000000"
);
// RUNTIME-accumulated overflow: a self-referential product (factorial) grows past i64 at runtime; the
// C backend's BigInt pre-pass declares the accumulator `LInt` so it promotes exactly, like the interp.
native_domain!(bignum_factorial, "## To fact (n: Int) -> Int:\n    Let mutable result be 1.\n    Repeat for i from 1 to n:\n        Set result to result * i.\n    Return result.\n## Main\nShow fact(25).\nShow fact(5).\n", "15511210043330985984000000\n120");
// complex now works on the C backend too (exact LComplex over LRat/LBig) — full native coverage.
native_domain!(
    complex_ops,
    "## Main\nShow complex(1, 2) + complex(3, 4).\nShow abs(complex(3, 4)).\n",
    "4+6i\n5"
);
native_domain!(
    complex_muldiv,
    "## Main\nShow complex(3, 4) * complex(1, 0 - 1).\nShow complex(1, 2) / complex(3, 4).\n",
    "7+i\n11/25+2/25i"
);

// Wave 3 — the complex transcendentals (aot_native only; `cConj` over ints returns an EXACT complex,
// the same shape as the `abs` bug) and the exact/operator surface that claims aot_c.
native_aot_only!(
    complex_conj,
    "## Main\nShow cConj(3, 4).\nShow cConj(5, 0).\n"
);
native_aot_only!(complex_exp, "## Main\nShow cExp(0.0, 0.0).\n");
native_aot_only!(
    complex_phase,
    "## Main\nShow floor(cPhase(0.0, 1.0) * 1000.0).\n"
);
native_domain!(
    operator_invert,
    "## Main\nShow opInvert(5).\nShow opInvert(0).\n",
    "-6\n-1"
);
native_domain!(
    operator_truediv,
    "## Main\nShow opTrueDiv(7.0, 2.0).\n",
    "3.5"
);
native_domain!(
    convert_oct,
    "## Main\nShow toOct(8).\nShow quoteRune(65).\n",
    "10\n'A'"
);
native_domain!(
    math_cmpabs,
    "## Main\nShow cmpAbsInt(0 - 5, 3).\nShow cmpAbsInt(0 - 4, 4).\n",
    "1\n0"
);
// decimal (exact LogosRational) now works on the C backend too (LRat over LBig).
native_domain!(
    decimal_ops,
    "## Main\nShow decimal(10) + decimal(2).\nShow decimal(1) / decimal(3).\n",
    "12\n1/3"
);
native_domain!(decimal_frac, "## Main\nShow decimal(1) / decimal(3) + decimal(1) / decimal(6).\nShow decimal(7) * decimal(3).\n", "1/2\n21");
// scale-aware decimal-from-string (LDec on C): fixed-point construction + scale-preserving +−×.
native_domain!(decimal_scale_string, "## Main\nShow decimal(\"0.10\").\nShow decimal(\"0.5\") + decimal(\"0.25\").\nShow decimal(\"1.50\") * decimal(2).\nShow decimal(\"-1.50\").\n", "0.10\n0.75\n3.00\n-1.50");

// money (the C mirror LMoney — LDec amount quantised to an ISO-4217 currency) on both native tiers:
// int/decimal amounts at scale 0/2/3, same-currency +/-, integer scaling, and a dimensionless ratio.
native_domain!(money_construct, "## Main\nShow money(10, \"USD\").\nShow money(100, \"JPY\").\nShow money(5, \"BHD\").\nShow money(decimal(\"19.99\"), \"USD\").\n", "10.00 USD\n100 JPY\n5.000 BHD\n19.99 USD");
native_domain!(money_arith, "## Main\nShow money(decimal(\"0.10\"), \"USD\") + money(decimal(\"0.20\"), \"USD\").\nShow money(30, \"USD\") - money(10, \"USD\").\nShow money(decimal(\"19.99\"), \"USD\") * 3.\nShow money(30, \"USD\") / money(10, \"USD\").\n", "0.30 USD\n20.00 USD\n59.97 USD\n3");

// modular (exact ℤ/nℤ — LogosModular / the C LMod over LBig) on both native tiers: ring `+ − ×`,
// field `÷` (multiply by the modular inverse), and `pow(m, e)` modular exponentiation. Euclidean
// reduction into [0, modulus); a large modulus stays exact.
native_domain!(modular_ring, "## Main\nShow modular(5, 7) + modular(4, 7).\nShow modular(2, 7) - modular(5, 7).\nShow modular(6, 7) * modular(6, 7).\n", "2 (mod 7)\n4 (mod 7)\n1 (mod 7)");
native_domain!(
    modular_field,
    "## Main\nShow modular(3, 7) / modular(5, 7).\nShow modular(1, 7) / modular(3, 7).\n",
    "2 (mod 7)\n5 (mod 7)"
);
native_domain!(
    modular_pow,
    "## Main\nShow pow(modular(2, 7), 5).\nShow pow(modular(3, 10), 4).\n",
    "4 (mod 7)\n1 (mod 10)"
);
native_domain!(modular_bigmod, "## Main\nShow modular(10, 1000000007).\nShow modular(3, 1000000007) * modular(7, 1000000007).\n", "10 (mod 1000000007)\n21 (mod 1000000007)");

// uuid (RFC 9562 — the C mirror LUuid over 16 bytes) on both native tiers: canonical parse/round-trip,
// nil/max/namespace constants, the version nibble, 16-byte equality, and the Seq-of-bytes round-trip.
native_domain!(uuid_parse_show, "## Main\nShow uuid(\"550e8400-e29b-41d4-a716-446655440000\").\nShow uuid_nil().\nShow uuid_max().\nShow uuid_dns().\n", "550e8400-e29b-41d4-a716-446655440000\n00000000-0000-0000-0000-000000000000\nffffffff-ffff-ffff-ffff-ffffffffffff\n6ba7b810-9dad-11d1-80b4-00c04fd430c8");
native_domain!(uuid_version_eq, "## Main\nShow uuid_version(uuid(\"550e8400-e29b-41d4-a716-446655440000\")).\nShow uuid_nil() is equal to uuid_nil().\nShow uuid_nil() is equal to uuid_max().\n", "4\ntrue\nfalse");
native_domain!(uuid_bytes_roundtrip, "## Main\nShow uuid_bytes(uuid(\"550e8400-e29b-41d4-a716-446655440000\")).\nShow uuid_from_bytes(uuid_bytes(uuid(\"550e8400-e29b-41d4-a716-446655440000\"))).\n", "[85, 14, 132, 0, 226, 155, 65, 212, 167, 22, 68, 102, 85, 68, 0, 0]\n550e8400-e29b-41d4-a716-446655440000");

// quantity (the C mirror LQuant — exact-Rational SI magnitude + a 10-axis dimension vector + a display
// unit) on both native tiers: construction (natural `N unit` + `quantity(v, "unit")`), same-dimension
// +/-, scalar scaling (keeps the unit), exact rational `in`-conversion, and dimension-combining ×/÷
// that render the dimension SIGNATURE (`6 L^2`, dimensionless `25 1`). Units resolve at codegen time.
native_domain!(
    quantity_construct,
    "## Main\nShow 5 meters.\nShow quantity(2, \"inch\").\nShow 3 kilograms.\n",
    "5 m\n2 in\n3 kg"
);
native_domain!(
    quantity_add_sub,
    "## Main\nShow 2 meters + 3 meters.\nShow 5 meters - 2 meters.\nShow 5 meters * 2.\n",
    "5 m\n3 m\n10 m"
);
native_domain!(
    quantity_convert,
    "## Main\nShow 5 meters in feet.\nShow 1 mile in feet.\n",
    "6250/381 ft\n5280 ft"
);
native_domain!(
    quantity_dimension,
    "## Main\nShow 2 meters * 3 meters.\nShow 100 meters / 4 meters.\nShow 6 meters / 3 meters.\n",
    "6 L^2\n25 1\n2 1"
);

// Value types inside COLLECTIONS on the C tier — the runtime's `Seq_<E>` families (`Seq of Money` →
// `Seq_LMoney`). Construction, item access, length, the `[a, b]` Show, `Push`, and loop/reduce.
native_domain!(seq_of_money, "## Main\nLet ms be [money(10, \"USD\"), money(20, \"USD\")].\nShow item 1 of ms.\nShow length of ms.\nShow ms.\n", "10.00 USD\n2\n[10.00 USD, 20.00 USD]");
native_domain!(seq_of_value_show, "## Main\nShow [modular(3, 7), modular(5, 7)].\nShow [decimal(\"0.10\"), decimal(\"0.20\")].\nShow [uuid_nil()].\nShow [complex(1, 2), complex(3, 4)].\n", "[3 (mod 7), 5 (mod 7)]\n[0.10, 0.20]\n[00000000-0000-0000-0000-000000000000]\n[1+2i, 3+4i]");
// Value types as MAP VALUES — `Map of Text to Money/Quantity` → the runtime's `Map_str_<E>` hash-map
// family. Set/get/contains, byte-identical to the interpreter, both native tiers.
native_domain!(map_str_money, "## Main\nLet mutable prices be a new Map of Text to Money.\nSet item \"coffee\" of prices to money(3, \"USD\").\nSet item \"tea\" of prices to money(2, \"USD\").\nShow item \"coffee\" of prices.\nShow item \"tea\" of prices.\n", "3.00 USD\n2.00 USD");
native_domain!(map_str_quantity, "## Main\nLet mutable dist be a new Map of Text to Quantity.\nSet item \"home\" of dist to 5 meters.\nShow item \"home\" of dist.\n", "5 m");
native_domain!(map_str_value_contains, "## Main\nLet mutable prices be a new Map of Text to Money.\nSet item \"coffee\" of prices to money(3, \"USD\").\nShow prices contains \"coffee\".\nShow prices contains \"milk\".\n", "true\nfalse");

// Loop/reduce over a value-type Seq — locked on BOTH native tiers. The AOT-Rust gap (a value-type
// loop variable lowered `total + m` to a raw Rust `+` → E0369) is fixed at its root: a `Seq of Money`
// / `Seq of Quantity` literal now registers as `LogosSeq<LogosMoney>` (was the lossy `LogosSeq<_>`),
// so the loop variable resolves to its value type and `total + m` lowers to `total.add(&m)`.
native_domain!(seq_of_quantity_reduce, "## Main\nLet qs be [5 meters, 3 meters, 2 meters].\nLet mutable sum be 0 meters.\nRepeat for q in qs:\n    Set sum to sum + q.\nShow sum.\n", "10 m");
native_domain!(seq_of_money_push_reduce, "## Main\nLet mutable ms be [money(10, \"USD\")].\nPush money(20, \"USD\") to ms.\nLet mutable total be money(0, \"USD\").\nRepeat for m in ms:\n    Set total to total + m.\nShow total.\n", "30.00 USD");

// Value types in STRING INTERPOLATION — `"{m}"` renders via the value type's `*_to_string` (was a
// silent `%lld`→`0` divergence on C). Both native tiers.
native_domain!(interp_value_types, "## Main\nLet m be money(10, \"USD\").\nLet q be 5 meters.\nLet d be decimal(\"1.50\").\nLet n be modular(3, 7).\nShow \"bal {m}, dist {q}, d={d}, n={n}\".\n", "bal 10.00 USD, dist 5 m, d=1.50, n=3 (mod 7)");

// Value types as FUNCTION PARAMS + RETURNS — a value-returning fn and value-type passthrough params.
// This context was probe-verified but previously UNLOCKED on both tiers (the gap this audit caught).
native_domain!(
    fn_value_return,
    "## To makeM -> Money:\n    Return money(7, \"USD\").\n\n## Main\nShow makeM().\n",
    "7.00 USD"
);
native_domain!(
    fn_money_param,
    "## To idM (m: Money) -> Money:\n    Return m.\n\n## Main\nShow idM(money(5, \"USD\")).\n",
    "5.00 USD"
);
native_domain!(
    fn_quantity_param,
    "## To idQ (q: Quantity) -> Quantity:\n    Return q.\n\n## Main\nShow idQ(5 meters).\n",
    "5 m"
);

// Value types as STRUCT FIELDS — field access / mutation / multi-field now locked on BOTH native tiers.
// The AOT-Rust gap (the generated struct `#[derive(Default)]`s and constructs with `..Default::default()`
// for CRDT/partial-init hidden fields, but the value types had no `Default` → E0277) is fixed at its
// root by `impl Default for Logos{Money,Quantity,Modular,Uuid,Complex,Decimal}` (zero/nil placeholders).
native_domain!(struct_money_field, "## A Wallet has:\n    A balance: Money.\n\n## Main\nLet w be a new Wallet with balance money(50, \"USD\").\nShow w's balance.\n", "50.00 USD");
native_domain!(struct_quantity_field, "## A Box has:\n    A size: Quantity.\n\n## Main\nLet b be a new Box with size 5 meters.\nShow \"size {b's size}\".\n", "size 5 m");
native_domain!(struct_multi_field, "## A Item has:\n    A name: Text.\n    A price: Money.\n\n## Main\nLet it be a new Item with name \"Coffee\" and price money(4, \"USD\").\nShow it's name.\nShow it's price.\n", "Coffee\n4.00 USD");
native_domain!(struct_field_mutation, "## A Wallet has:\n    A balance: Money.\n\n## Main\nLet mutable w be a new Wallet with balance money(50, \"USD\").\nSet w's balance to money(60, \"USD\").\nShow w's balance.\n", "60.00 USD");
// WHOLE-struct `Show w` — now locked on BOTH native tiers, byte-identical to the interpreter oracle:
// `Name { field: value }` (braces, fields SORTED by name). This was a pre-existing 3-way divergence
// (NOT value-type-specific — it failed for a plain `Int` struct too): the C backend rendered
// `Name(f: v)` (parens, declaration order) while the oracle + ~1690 suite tests use braces+sorted, and
// AOT-Rust emitted no `Showable` impl for user structs at all. Fixed by canonicalizing the C struct-show
// codegen to braces+sorted and generating a `Showable` impl for user structs on AOT-Rust (both above).
native_domain!(struct_value_field_show, "## A Wallet has:\n    A balance: Money.\n\n## Main\nLet w be a new Wallet with balance money(50, \"USD\").\nShow w.\n", "Wallet { balance: 50.00 USD }");

// Wave 4 — struct/Map-returning + bit shapes on both native tiers.
native_domain!(heap_ops, "## Main\nLet mutable h be heapNew().\nSet h to heapPush(h, 5).\nSet h to heapPush(h, 3).\nSet h to heapPush(h, 8).\nSet h to heapPush(h, 1).\nShow heapPeek(h).\nShow heapLen(h).\n", "1\n4");
native_domain!(
    bits_ops,
    "## Main\nShow popcount(7).\nShow bitLength(255).\n",
    "3\n8"
);
native_domain!(
    sort_ops,
    "## Main\nShow isSortedSeq([1, 2, 2, 3]).\nShow isSortedSeq([1, 3, 2]).\n",
    "true\nfalse"
);
native_domain!(time_ops, "## Main\nLet m be parse_timestamp(\"2024-03-15T10:30:45Z\").\nShow year_of(m).\nShow month_of(m).\nShow day_of(m).\n", "2024\n3\n15");
