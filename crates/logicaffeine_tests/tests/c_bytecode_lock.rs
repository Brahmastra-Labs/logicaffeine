//! ★ THE BYTECODE-C LOCK ★ — Phase 1 of the bytecode-IR-as-source-of-truth refactor.
//!
//! The `CompiledProgram → C` Op-walker (`compile_to_c_via_bytecode`, `codegen_c::from_program`) must run
//! byte-for-byte the same as the VM. `program_outcome(compile_to_program(src))` is the meaning; the emitted C,
//! compiled by gcc and run, is the realization. A not-yet-lowered op is a typed `Err` from the emitter (it
//! REFUSES rather than emit wrong code), so this lock GROWS the covered op surface milestone by milestone —
//! start with the integer scalar/arith/comparison/loop/branch core, then add types, functions, collections.
//!
//! This is the trap that makes the re-base safe: once the Op-walker covers a program, its C output is pinned
//! to the VM here, and the whole-corpus `ultimate_gauntlet_lock` C leg is the outer safety net when we swap
//! `compile_to_c` over to it. gcc-gated; SKIPs clean when the toolchain is absent.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::codegen_c::from_program::compile_program_to_c;
use logicaffeine_compile::compile::{
    compile_to_c_via_bytecode, compile_to_program, compile_to_program_optimized, program_outcome,
};
use logicaffeine_compile::interpret_for_ui_sync;

#[path = "gauntlet/mod.rs"]
mod gauntlet;

#[path = "support/fixed_clock.rs"]
mod fixed_clock;

/// The integer core the first milestone lowers: constants, arithmetic, comparison-driven control flow, loops.
/// Every program's observable is an integer `Show` (the register file is i64 until typed registers land).
const INT_CORPUS: &[(&str, &str)] = &[
    ("scalar", "## Main\n    Show 42.\n"),
    ("negative", "## Main\n    Show (0 - 5) + 2.\n"),
    ("arith", "## Main\n    Show 2 + 3 * 4 - 1.\n"),
    ("loop_sum", "## Main\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is at most 100:\n        Set acc to acc + i.\n        Set i to i + 1.\n    Show acc.\n"),
    ("nested_loops", "## Main\n    Let mutable s be 0.\n    Let mutable i be 1.\n    While i is at most 20:\n        Let mutable j be 1.\n        While j is at most 20:\n            Set s to s + i * j.\n            Set j to j + 1.\n        Set i to i + 1.\n    Show s.\n"),
    ("branch_then", "## Main\n    Let x be 7.\n    If x is greater than 5:\n        Show 1.\n    Otherwise:\n        Show 0.\n"),
    ("branch_else", "## Main\n    Let x be 3.\n    If x is greater than 5:\n        Show 1.\n    Otherwise:\n        Show 0.\n"),
    ("countdown", "## Main\n    Let mutable i be 5.\n    While i is greater than 0:\n        Show i.\n        Set i to i - 1.\n    Show 0.\n"),
    // Functions (register-windowing call convention) — still integer.
    ("square_fn", "## To square (n: Int) -> Int:\n    Return n * n.\n\n## Main\n    Show square(9).\n"),
    ("add_two", "## To add (a: Int) (b: Int) -> Int:\n    Return a + b.\n\n## Main\n    Show add(3, 4).\n"),
    ("fib_recursive", "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\n    Show fib(20).\n"),
    ("call_in_loop", "## To dbl (n: Int) -> Int:\n    Return n + n.\n\n## Main\n    Let mutable s be 0.\n    Let mutable i be 1.\n    While i is at most 10:\n        Set s to s + dbl(i).\n        Set i to i + 1.\n    Show s.\n"),
];

/// The scalar-kinds milestone: Bool / Float / Text ride the same uniform i64 register file (bool as 0/1,
/// float as its IEEE-754 bit pattern, text as a `char*` pointer), and `Show` renders each by its inferred
/// kind — byte-for-byte the VM's display.
const SCALAR_CORPUS: &[(&str, &str)] = &[
    ("float_lit", "## Main\n    Show 3.14.\n"),
    ("float_arith", "## Main\n    Show 1.5 + 2.25.\n"),
    ("float_div", "## Main\n    Show 10.0 / 4.0.\n"),
    ("float_mul_integral", "## Main\n    Show 2.5 * 4.0.\n"),
    ("float_int_mix", "## Main\n    Show 3 + 1.5.\n"),
    ("float_sub", "## Main\n    Show 8.0 - 2.5.\n"),
    ("text_lit", "## Main\n    Show \"hello, world\".\n"),
    (
        "text_concat",
        "## Main\n    Show \"foo\" combined with \"bar\".\n",
    ),
    (
        "bool_true",
        "## Main\n    Let b be 2 is greater than 1.\n    Show b.\n",
    ),
    (
        "bool_false",
        "## Main\n    Let b be 5 is less than 3.\n    Show b.\n",
    ),
];

/// The collections milestone: sequences (Int/Float/Text elements) ride the uniform i64 register file as
/// heap-boxed typed `Seq_*` handles, built by `NewList`/`NewRange`, read by `Index`/`Length`, walked by the
/// `IterPrepare`/`IterNext` for-each machine, and rendered by `Show` — byte-for-byte the VM.
const COLLECTION_CORPUS: &[(&str, &str)] = &[
    ("list_show", "## Main\n    Show [10, 20, 30].\n"),
    ("float_list_show", "## Main\n    Show [1.5, 2.5, 3.5].\n"),
    ("list_index", "## Main\n    Let xs be [7, 8, 9].\n    Show item 2 of xs.\n"),
    ("list_index_dynamic", "## Main\n    Let xs be [100, 200, 300].\n    Let i be 3.\n    Show item i of xs.\n"),
    ("list_length", "## Main\n    Let xs be [1, 2, 3, 4].\n    Show length of xs.\n"),
    ("range_sum", "## Main\n    Let mutable s be 0.\n    For each i from 1 to 5:\n        Set s to s + i.\n    Show s.\n"),
    ("range_show_each", "## Main\n    For each i from 1 to 3:\n        Show i.\n"),
    ("list_sum", "## Main\n    Let xs be [4, 5, 6].\n    Let mutable s be 0.\n    For each x in xs:\n        Set s to s + x.\n    Show s.\n"),
    ("list_count", "## Main\n    Let xs be [1, 2, 3, 4].\n    Let mutable count be 0.\n    For each x in xs:\n        Set count to count + 1.\n    Show count.\n"),
    ("float_list_sum", "## Main\n    Let xs be [1.5, 2.5, 4.0].\n    Let mutable s be 0.0.\n    For each x in xs:\n        Set s to s + x.\n    Show s.\n"),
];

/// The incremental-list milestone: an empty sequence (`a new Seq of T` / `[]`) is boxed and grown by `Push`,
/// drained by `Pop`, iterated even when empty — the element kind flows back to the empty-creation site through
/// the forward fixpoint. Byte-for-byte the VM.
const INCREMENTAL_LIST_CORPUS: &[(&str, &str)] = &[
    ("build_push_show", "## Main\n    Let mutable xs be a new Seq of Int.\n    Push 10 to xs.\n    Push 20 to xs.\n    Push 30 to xs.\n    Show xs.\n"),
    ("build_push_sum", "## Main\n    Let mutable xs be a new Seq of Int.\n    Let mutable i be 1.\n    While i is less than 6:\n        Push i to xs.\n        Set i to i + 1.\n    Let mutable total be 0.\n    For each x in xs:\n        Set total to total + x.\n    Show total.\n"),
    ("pop_last", "## Main\n    Let mutable xs be [5, 6, 7].\n    Pop from xs into v.\n    Show v.\n"),
    ("empty_iter", "## Main\n    Let xs be [].\n    Let mutable total be 0.\n    For each x in xs:\n        Set total to total + x.\n    Show total.\n"),
    ("text_build_show", "## Main\n    Let mutable ws be a new Seq of Text.\n    Push \"a\" to ws.\n    Push \"b\" to ws.\n    Show ws.\n"),
];

/// The scalar-arithmetic-completion milestone: exponentiation, floor-division (rounding toward −∞), the five
/// bitwise/shift ops, and the VM's exact `is approximately` tolerance — all over the uniform register file,
/// byte-for-byte the VM.
const SCALAR_ARITH_CORPUS: &[(&str, &str)] = &[
    ("pow_int", "## Main\n    Show 2 ** 10.\n"),
    ("pow_float", "## Main\n    Show 2.0 ** 10.0.\n"),
    ("floordiv_pos", "## Main\n    Show 7 // 2.\n"),
    ("floordiv_neg", "## Main\n    Let a be 0 - 7.\n    Show a // 2.\n"),
    ("bitand", "## Main\n    Let a be 1023.\n    Let b be 256.\n    Let c be a & b.\n    Show c.\n"),
    ("bitor", "## Main\n    Let a be 0.\n    Let b be 170.\n    Let c be a | b.\n    Show c.\n"),
    ("bitxor", "## Main\n    Let a be 4242.\n    Let c be a xor a.\n    Show c.\n"),
    ("shl", "## Main\n    Let a be 5.\n    Let b be 3.\n    Let c be a shifted left by b.\n    Show c.\n"),
    ("shr", "## Main\n    Let a be 20.\n    Let b be 2.\n    Let c be a shifted right by b.\n    Show c.\n"),
    ("approx_true", "## Main\n    Let a be 0.1.\n    Let b be 0.1.\n    Show a is approximately b.\n"),
    ("approx_computed", "## Main\n    Let a be 0.1 + 0.2.\n    Show a is approximately 0.3.\n"),
];

/// The scalar-builtin milestone: the math builtins called as functions (`sqrt`/`abs`/`floor`/`ceil`/`round`/
/// `min`/`max`) lower to the C `math.h` intrinsics exactly as `codegen_riscv::call_builtin` does — Sqrt→Float,
/// Floor/Ceil/Round land an i64, Abs keeps its kind, Min/Max promote to Float if either arg is. The final
/// `sqrt_runtime` case accumulates its argument in a loop so the optimizer cannot constant-fold the call away —
/// the `CallBuiltin` op genuinely reaches the Op-walker there.
const BUILTIN_CORPUS: &[(&str, &str)] = &[
    ("sqrt_int", "## Main\n    Show sqrt(9).\n"),
    ("sqrt_float", "## Main\n    Show sqrt(2.0).\n"),
    ("abs_int", "## Main\n    Let a be 0 - 7.\n    Show abs(a).\n"),
    ("abs_float", "## Main\n    Let a be 0.0 - 2.5.\n    Show abs(a).\n"),
    ("floor_f", "## Main\n    Let a be 3.7.\n    Show floor(a).\n"),
    ("ceil_f", "## Main\n    Let a be 3.2.\n    Show ceil(a).\n"),
    ("round_f", "## Main\n    Let a be 2.5.\n    Show round(a).\n"),
    ("min_int", "## Main\n    Let a be 3.\n    Let b be 8.\n    Show min(a, b).\n"),
    ("max_int", "## Main\n    Let a be 3.\n    Let b be 8.\n    Show max(a, b).\n"),
    ("min_float", "## Main\n    Let a be 3.5.\n    Let b be 1.5.\n    Show min(a, b).\n"),
    ("max_mixed", "## Main\n    Let a be 3.\n    Let b be 8.5.\n    Show max(a, b).\n"),
    ("sqrt_runtime", "## Main\n    Let mutable x be 0.0.\n    Let mutable i be 0.\n    While i is less than 16:\n        Set x to x + 1.0.\n        Set i to i + 1.\n    Show sqrt(x).\n"),
];

/// The tuple milestone: an anonymous tuple is a heap buffer of one i64 cell per position (like a struct with
/// no field names). `NewTuple` builds it, `DestructureTuple` (`Let (a, b) be …`) reads the cells back into
/// registers, and `Show` renders `(a, b, …)` per interned position kind — byte-for-byte the VM. The
/// function-return case (`pair` → `Let (s, d) be pair(…)`) keeps the tuple alive across a call boundary so the
/// Op-walker genuinely lowers `NewTuple`/`DestructureTuple` rather than folding.
const TUPLE_CORPUS: &[(&str, &str)] = &[
    ("tuple_show_ints", "## Main\n    Let t be (7, 8).\n    Show t.\n"),
    ("tuple_show_mixed", "## Main\n    Let t be (1, true).\n    Show t.\n"),
    ("tuple_destructure_sum", "## Main\n    Let (a, b) be (3, 4).\n    Show a + b.\n"),
    ("tuple_return_destructure", "## To pair (a: Int, b: Int) -> (Int, Int):\n    Return (a + b, a - b).\n## Main\n    Let (s, d) be pair(10, 3).\n    Show s.\n    Show d.\n"),
    ("tuple_return_show", "## To pair (a: Int, b: Int) -> (Int, Int):\n    Return (a + b, a - b).\n## Main\n    Let t be pair(10, 3).\n    Show t.\n"),
];

/// The struct milestone: a user struct is a heap buffer of one i64 cell per field, so `NewStruct` allocates,
/// field-set writes a cell, and `p's field` reads one — including nested structs and Seq/Float/Text fields.
/// Field access is the overwhelmingly common shape; byte-for-byte the VM.
const STRUCT_CORPUS: &[(&str, &str)] = &[
    ("two_int_fields", "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet p be a new Point with x 10 and y 20.\nShow p's x.\nShow p's y.\n"),
    ("nested_struct", "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## A Line has:\n    A p: Point.\n\n## Main\nLet a be a new Point with x 1 and y 2.\nLet l be a new Line with p a.\nShow l's p's x.\n"),
    ("seq_field", "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet b be a new Bag with items [1, 2, 3].\nShow b's items.\n"),
    ("float_field", "## A V has:\n    A m: Float.\n\n## Main\nLet v be a new V with m 3.5.\nShow v's m.\n"),
    ("text_field", "## A P has:\n    A name: Text.\n\n## Main\nLet p be a new P with name \"hi\".\nShow p's name.\n"),
    ("field_arith", "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet p be a new Point with x 3 and y 4.\nShow p's x + p's y.\n"),
];

/// The set milestone: an insertion-ordered dedup'd `Set of Int`/`Set of Text` rides the uniform register file
/// as a heap-boxed `Set_i64`/`Set_str`, built by `Add`, membership-tested, sized, drained by `Remove`, and
/// rendered `{…}` by `Show` — byte-for-byte the VM (element kind flows to the empty-creation site via the
/// fixpoint; an unpopulated set renders `{}`).
const SET_CORPUS: &[(&str, &str)] = &[
    ("empty_int_set", "## Main\n    Let mutable s be a new Set of Int.\n    Show s.\n    Show length of s.\n"),
    ("int_set_add", "## Main\n    Let mutable s be a new Set of Int.\n    Add 7 to s.\n    Show s.\n    Show length of s.\n"),
    ("int_set_dedup", "## Main\n    Let mutable s be a new Set of Int.\n    Add -5 to s.\n    Add 0 to s.\n    Add -5 to s.\n    Add 3 to s.\n    Show s.\n    Show length of s.\n"),
    ("text_set", "## Main\n    Let mutable s be a new Set of Text.\n    Add \"alice\" to s.\n    Add \"bob\" to s.\n    Add \"alice\" to s.\n    Show s.\n    Show length of s.\n"),
    ("set_contains", "## Main\n    Let mutable s be a new Set of Int.\n    Add 1 to s.\n    Add 2 to s.\n    If s contains 1:\n        Show 1.\n    Otherwise:\n        Show 0.\n"),
    ("set_remove", "## Main\n    Let mutable s be a new Set of Int.\n    Add 1 to s.\n    Add 2 to s.\n    Remove 2 from s.\n    Show s.\n"),
];

/// The enum milestone: a user enum (inductive) is a heap buffer `[variant-tag, payload…]`, so `NewInductive`
/// boxes it, `Inspect`/`If it is a <Variant>` compiles to a `TestArm` tag-compare, and `(field: v)` bindings
/// to `BindArm` payload reads — nullary and multi-field variants, Int/Text payloads. Byte-for-byte the VM.
const ENUM_CORPUS: &[(&str, &str)] = &[
    ("nullary_variant", "## Definition\nA Color is either:\n    A Red.\n    A Green.\n    A Blue.\n\n## Main\nLet c be a new Green.\nInspect c:\n    If it is a Red:\n        Show \"red\".\n    If it is a Green:\n        Show \"green\".\n    Otherwise:\n        Show \"other\".\n"),
    ("one_int_field", "## Definition\nA Shape is either:\n    A Circle with a radius, which is Int.\n    A Point.\n\n## Main\nLet s be a new Circle with radius 5.\nInspect s:\n    If it is a Circle (radius: r):\n        Show r.\n    If it is a Point:\n        Show \"point\".\n"),
    ("two_int_fields", "## Definition\nA Shape is either:\n    A Rectangle with a width, which is Int, and a height, which is Int.\n    A Point.\n\n## Main\nLet s be a new Rectangle with width 4 and height 6.\nInspect s:\n    If it is a Rectangle (width: w, height: h):\n        Show w + h.\n    If it is a Point:\n        Show 0.\n"),
    ("text_field", "## Definition\nA Result is either:\n    A Success (value: Int).\n    A Failure (message: Text).\n\n## Main\nLet r be a new Failure with message \"oops\".\nInspect r:\n    If it is a Success (value: v):\n        Show v.\n    If it is a Failure (message: m):\n        Show m.\n"),
];

/// The map milestone: a `Map from K to V` (key/value ∈ {Int,Text}) rides the uniform register file as a
/// heap-boxed `Map_<k>_<v>*`, created empty (kind refined by the first insert), inserted/updated by `SetIndex`,
/// looked up by `Index` (traps ≡VM on an absent key), key-membership-tested, sized by `length`, and rendered
/// `{k: v, …}` in insertion order by `Show` — byte-for-byte the VM.
const MAP_CORPUS: &[(&str, &str)] = &[
    ("map_int_int", "## Main\n    Let mutable m be a new Map of Int to Int.\n    Set item 3 of m to 100.\n    Set item 5 of m to 200.\n    Show item 3 of m.\n    Show length of m.\n"),
    ("map_show", "## Main\n    Let mutable m be a new Map of Int to Int.\n    Set item 1 of m to 10.\n    Set item 2 of m to 20.\n    Show m.\n"),
    ("map_update", "## Main\n    Let mutable m be a new Map of Int to Int.\n    Set item 1 of m to 10.\n    Set item 1 of m to 99.\n    Show item 1 of m.\n    Show length of m.\n"),
    ("map_str_int", "## Main\n    Let mutable m be a new Map of Text to Int.\n    Set item \"a\" of m to 1.\n    Set item \"b\" of m to 2.\n    Show item \"b\" of m.\n    Show length of m.\n"),
    ("map_int_str", "## Main\n    Let mutable m be a new Map of Int to Text.\n    Set item 1 of m to \"one\".\n    Show item 1 of m.\n"),
    ("map_empty", "## Main\n    Let m be a new Map of Int to Int.\n    Show m.\n    Show length of m.\n"),
    ("map_contains", "## Main\n    Let mutable m be a new Map of Int to Int.\n    Set item 7 of m to 1.\n    If m contains 7:\n        Show 1.\n    Otherwise:\n        Show 0.\n"),
    // World-class values: a map value can be ANY kind — it rides the i64/str cell as its register encoding, so
    // insert + `item k of m` recover it exactly. A Float value round-trips its bits; a Bool its 0/1.
    ("map_text_float", "## Main\n    Let mutable m be a new Map of Text to Float.\n    Set item \"pi\" of m to 3.14.\n    Set item \"e\" of m to 2.5.\n    Show item \"pi\" of m.\n    Show length of m.\n"),
    ("map_text_bool", "## Main\n    Let mutable m be a new Map of Text to Bool.\n    Set item \"yes\" of m to true.\n    Set item \"no\" of m to false.\n    Show item \"yes\" of m.\n    Show item \"no\" of m.\n"),
    ("map_int_float", "## Main\n    Let mutable m be a new Map of Int to Float.\n    Set item 1 of m to 1.5.\n    Show item 1 of m.\n"),
];

/// Curated PER-OP EDGE CASES — the hand-picked corners the fuzzer reaches only rarely: identities (`x-x`,
/// `x xor x`, `x&0`), boundaries (`is at most` at equality, shift by 0 / by width, first/last list index),
/// empties (`[]`, empty set/map, zero-iteration loop), sign corners (negative floor-division / mod / abs / max),
/// exact floats, empty/unicode text, dedup/update, and recursion. Each is i64-safe and non-erroring, and must
/// run byte-for-byte the VM through the SUPERCOMPILED bytecode → C.
const EDGE_CORPUS: &[(&str, &str)] = &[
    ("add_zero", "## Main\n    Show 0 + 0.\n"),
    ("sub_self_zero", "## Main\n    Let a be 7.\n    Show a - a.\n"),
    ("mul_zero", "## Main\n    Show 0 * 999.\n"),
    ("mul_neg_neg", "## Main\n    Let a be 0 - 3.\n    Let b be 0 - 4.\n    Show a * b.\n"),
    ("floordiv_neg_toward_neg_inf", "## Main\n    Let a be 0 - 7.\n    Show a // 2.\n"),
    ("floordiv_by_one", "## Main\n    Show 42 // 1.\n"),
    ("mod_by_one_zero", "## Main\n    Show 42 mod 1.\n"),
    ("mod_self_zero", "## Main\n    Let a be 9.\n    Show a mod a.\n"),
    ("mod_neg", "## Main\n    Let a be 0 - 7.\n    Show a mod 3.\n"),
    ("pow_zero_is_one", "## Main\n    Show 5 ** 0.\n"),
    ("pow_one", "## Main\n    Show 5 ** 1.\n"),
    ("eq_true", "## Main\n    Show 5 is equal to 5.\n"),
    ("lteq_at_equality", "## Main\n    Show 5 is at most 5.\n"),
    ("lt_at_equality_false", "## Main\n    Show 5 is less than 5.\n"),
    ("gteq_at_equality", "## Main\n    Show 5 is at least 5.\n"),
    ("cmp_int_float", "## Main\n    Show 3 is less than 3.5.\n"),
    ("and_zero", "## Main\n    Let a be 255.\n    Show a & 0.\n"),
    ("or_zero_identity", "## Main\n    Let a be 170.\n    Show a | 0.\n"),
    ("xor_self_zero", "## Main\n    Let a be 12345.\n    Show a xor a.\n"),
    ("shl_by_zero", "## Main\n    Let a be 5.\n    Show a shifted left by 0.\n"),
    ("shr_off_the_end", "## Main\n    Let a be 255.\n    Show a shifted right by 8.\n"),
    ("not_true_is_false", "## Main\n    Let b be 1 is less than 2.\n    Show not b.\n"),
    ("not_of_comparison", "## Main\n    Show not (5 is greater than 3).\n"),
    ("float_zero", "## Main\n    Show 0.0.\n"),
    ("float_neg", "## Main\n    Show 0.0 - 2.5.\n"),
    ("float_integral_prints_clean", "## Main\n    Show 2.0 * 2.0.\n"),
    ("float_div_exact", "## Main\n    Show 9.0 / 3.0.\n"),
    ("text_empty", "## Main\n    Show \"\".\n"),
    ("concat_with_empty", "## Main\n    Show \"\" combined with \"x\".\n"),
    ("text_unicode", "## Main\n    Show \"héllo→世界\".\n"),
    ("list_single", "## Main\n    Let xs be [42].\n    Show item 1 of xs.\n    Show length of xs.\n"),
    ("list_last_index", "## Main\n    Let xs be [10, 20, 30].\n    Show item 3 of xs.\n"),
    ("list_empty_foreach", "## Main\n    Let xs be [].\n    Let mutable s be 0.\n    For each x in xs:\n        Set s to s + x.\n    Show s.\n"),
    ("set_empty_render", "## Main\n    Let s be a new Set of Int.\n    Show s.\n    Show length of s.\n"),
    ("set_dedup_len", "## Main\n    Let mutable s be a new Set of Int.\n    Add 3 to s.\n    Add 3 to s.\n    Show length of s.\n"),
    ("map_empty_len", "## Main\n    Let m be a new Map of Int to Int.\n    Show length of m.\n"),
    ("map_update_keeps_len", "## Main\n    Let mutable m be a new Map of Int to Int.\n    Set item 1 of m to 5.\n    Set item 1 of m to 9.\n    Show length of m.\n    Show item 1 of m.\n"),
    ("tuple_destructure_sub", "## Main\n    Let (a, b) be (7, 8).\n    Show a - b.\n"),
    ("struct_field_arith", "## A EPt has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet p be a new EPt with x 3 and y 4.\nShow p's x * p's y.\n"),
    ("enum_otherwise_arm", "## Definition\nA EC is either:\n    A ER.\n    A EG.\n    A EB.\n\n## Main\nLet c be a new EB.\nInspect c:\n    If it is a ER:\n        Show 1.\n    Otherwise:\n        Show 2.\n"),
    ("while_zero_iterations", "## Main\n    Let mutable i be 5.\n    While i is less than 0:\n        Show 99.\n        Set i to i + 1.\n    Show i.\n"),
    ("nested_if_else", "## Main\n    Let x be 5.\n    If x is greater than 0:\n        If x is greater than 3:\n            Show 1.\n        Otherwise:\n            Show 2.\n    Otherwise:\n        Show 3.\n"),
    ("fib_recursive_call", "## To efib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return efib(n - 1) + efib(n - 2).\n\n## Main\n    Show efib(10).\n"),
    ("abs_negative", "## Main\n    Let a be 0 - 9.\n    Show abs(a).\n"),
    ("min_equal", "## Main\n    Show min(5, 5).\n"),
    ("max_neg", "## Main\n    Let a be 0 - 3.\n    Let b be 0 - 8.\n    Show max(a, b).\n"),
    ("sqrt_perfect_square", "## Main\n    Show sqrt(16.0).\n"),
    ("floor_negative", "## Main\n    Show floor(0.0 - 2.5).\n"),
];

// ─────────────────────────────────────────────────────────────────────────────────────────────────────────
//  THE PER-OP GRIND CORPUS — one deterministic program per op the walker lowers from the OPTIMIZED bytecode,
//  each pinned by (label, EXPECTED-OP, source). The lock proves TWO things at once: the op genuinely SURVIVES
//  the 44-pass optimizer lattice into the `CompiledProgram` (present-in-bytecode assertion — so the case is not
//  vacuous), AND the walker lowers it byte-for-byte == VM. This is the deterministic complement to the random
//  fuzzer census: every op here is GUARANTEED covered every run, not covered-on-average. Each moves its name
//  into `OPS_VERIFIED` as it lands.
// ─────────────────────────────────────────────────────────────────────────────────────────────────────────
const GRIND_CORPUS: &[(&str, &str, &str)] = &[
    ("seq_concat", "SeqConcat", "## Main\n    Let a be [1, 2, 3].\n    Let b be [4, 5].\n    Let c be a followed by b.\n    Show length of c.\n    Show item 4 of c.\n"),
    ("set_union", "UnionOp", "## Main\n    Let mutable s be a new Set of Int.\n    Add 1 to s.\n    Add 2 to s.\n    Let mutable t be a new Set of Int.\n    Add 2 to t.\n    Add 3 to t.\n    Let u be s union t.\n    Show length of u.\n"),
    ("set_intersect", "IntersectOp", "## Main\n    Let mutable s be a new Set of Int.\n    Add 1 to s.\n    Add 2 to s.\n    Let mutable t be a new Set of Int.\n    Add 2 to t.\n    Add 3 to t.\n    Let w be s intersection t.\n    Show length of w.\n"),
    ("deep_clone", "DeepClone", "## Main\n    Let mutable xs be [5, 6, 7].\n    Let c be copy of xs.\n    Show length of c.\n    Show item 2 of c.\n"),
    ("set_index_unchecked", "SetIndexUnchecked", "## Main\n    Let mutable xs be [5, 6, 7].\n    Set item 2 of xs to 99.\n    Show item 2 of xs.\n"),
    ("rev_swap", "RevSwap", "## Main\n    Let mutable a be 3.\n    Let mutable b be 8.\n    Swap a and b.\n    Show a.\n    Show b.\n"),
    ("rev_add", "RevAdd", "## Main\n    Let mutable a be 10.\n    Reversibly:\n        Set a to a + 7.\n    Show a.\n"),
    ("rev_sub", "RevSub", "## Main\n    Let mutable a be 30.\n    Reversibly:\n        Set a to a - 7.\n    Show a.\n"),
    ("rev_xor", "RevXor", "## Main\n    Let mutable a be 12.\n    Reversibly:\n        Set a to a xor 6.\n    Show a.\n"),
    ("rev_neg", "RevNeg", "## Main\n    Let mutable a be 14.\n    Reversibly:\n        Set a to -a.\n    Show a.\n"),
    ("list_push_field", "ListPushField", "## A FzBag has:\n    An items: Seq of Int.\n\n## Main\n    Let mutable b be a new FzBag with items [1, 2].\n    Push 5 to b's items.\n    Show length of b's items.\n"),
    ("exact_div", "ExactDiv", "## Main\n    Let x: Rational be 7 / 2.\n    Show x.\n"),
    ("exact_div_arith", "ExactDiv", "## Main\n    Let x: Rational be 7 / 2.\n    Let y: Rational be 1 / 3.\n    Show x + y.\n    Show x * y.\n"),
    ("format_value_width", "FormatValue", "## Main\n    Let mutable a be 5.\n    Show \"n is {a:5}\".\n"),
    ("destructure_tuple", "DestructureTuple", "## Main\n    Let pairs be [(1, 2), (3, 4), (5, 6)].\n    Let mutable s be 0.\n    For each (a, b) in pairs:\n        Set s to s + a + b.\n    Show s.\n"),
    // Closures: a lambda `(n) -> n * 2` passed to a higher-order `apply` — MakeClosure builds the closure object,
    // CallValue dispatches into the (inline-embedded) closure body by its runtime body index.
    ("closure_make", "MakeClosure", "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nShow apply((n: Int) -> n * 2, 21).\n"),
    ("closure_call", "CallValue", "## To apply3 (f: fn(Int) -> Int) -> Int:\n    Return f(3).\n\n## Main\nShow apply3((n: Int) -> n + 100).\n"),
    // A closure that CAPTURES a free variable `k` from its environment (exercises the capture slots).
    ("closure_capture", "MakeClosure", "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\n    Let k be 10.\n    Show apply((n: Int) -> n + k, 5).\n"),
    // Arbitrary-precision integers: a constant-operand arithmetic op whose EXACT result overflows i64 must
    // promote to a bignum (the VM does), not wrap — the walker detects it at codegen and lowers via `lint_*`.
    ("bignum_mul", "Mul", "## Main\n    Show 10000000000 * 10000000000.\n"),
    ("bignum_pow", "Pow", "## Main\n    Show 2 ** 100.\n"),
    ("bignum_add", "Add", "## Main\n    Let a be 9223372036854775807.\n    Show a + 1.\n"),
    // `Pop` takes the last item and shortens the list; a pop from the list it emptied stops the program after
    // what was shown, through the guard `resolve_program` puts before every `Pop`.
    ("pop_last_items", "ListPop", "## Main\n    Let mutable xs be a new Seq of Int.\n    Push 7 to xs.\n    Push 8 to xs.\n    Pop from xs into a.\n    Show a.\n    Pop from xs into b.\n    Show b.\n    Pop from xs into c.\n    Show c.\n"),
    // Closure body arithmetic is EXACT: `(n) -> n * 2` applied to i64::MAX overflows → the walker promotes to
    // bignum via `lint_*` (VM does). Bignum promotion doesn't flow through closures in the shared analysis (the
    // Rust backend PANICS, AST-C wraps); being exact-by-default inside the rare closure body closes that hole.
    ("closure_bignum", "CallValue", "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nShow apply((n: Int) -> n * 2, 9223372036854775807).\n"),
    // ── CRDT lattice (single-replica) — the walker lowers these NATIVELY from the bytecode (mirroring
    // `codegen_riscv`, over the C `int64_t*` struct layout): a counter field is a `logos_crdt*` handle summed
    // on read + per-replica-MAX on merge; a sequence CRDT is a shared seq appended in place; a register is an
    // LWW cell. `sync_only`'s `Tally.n` field type does NOT resolve statically, so the walker synthesizes the
    // struct layout from the bytecode and a `CrdtBump` on the `Nothing` cell stays a plain `Int` — matching the
    // VM (`crdt_counter_bump(Nothing, n) = Int(n)`), where AST-C mis-renders `0`. ──
    ("crdt_new_bump", "NewCrdt", "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n\n## Main\nLet mutable c be a new Counter.\nIncrease c's points by 5.\nShow c's points.\n"),
    ("crdt_bump", "CrdtBump", "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n\n## Main\nLet mutable c be a new Counter.\nIncrease c's points by 5.\nShow c's points.\n"),
    ("crdt_merge", "CrdtMerge", "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n\n## Main\nLet mutable a be a new Counter.\nLet mutable b be a new Counter.\nIncrease a's points by 3.\nIncrease b's points by 5.\nMerge b into a.\nShow a's points.\n"),
    ("crdt_append", "CrdtAppend", "## Definition\nA Doc is Shared and has:\n    lines: SharedSequence of Text.\n\n## Main\nLet mutable d be a new Doc.\nAppend \"L1\" to d's lines.\nAppend \"L2\" to d's lines.\nShow the length of d's lines.\n"),
    ("crdt_resolve", "CrdtResolve", "## Definition\nA WikiPage is Shared and has:\n    a title, which is Divergent Text.\n\n## Main\nLet mutable page be a new WikiPage.\nSet page's title to \"Draft\".\nShow page's title.\nResolve page's title to \"Final\".\nShow page's title.\n"),
];

/// The offline NETWORKING grind — the `Net*` ops SUSPEND the VM at a blocking op (they materialize + `block`),
/// so the plain synchronous `program_outcome` VM never reaches the `Show` past them (its output is empty). The
/// CORRECT oracle is the ASYNC path (`interpret_for_ui_sync`, which drives the offline loopback transport), so a
/// dedicated lock pins the walker ≡ that oracle. Each entry is `(label, EXPECTED-OP, source)`: the op must
/// survive into the optimized bytecode (non-vacuous), and the walker's compiled output == the async VM's lines.
const NET_CORPUS: &[(&str, &str, &str)] = &[
    ("net_listen", "NetListen", "## Main\nListen on \"/ip4/0.0.0.0/tcp/0\".\nShow 1.\n"),
    ("net_sync", "NetSync", "## Definition\nA Tally has:\n    n: a Counter.\n## Main\n    Let mutable state be a new Tally.\n    Increase state's n by 3.\n    Sync state on \"topic\".\n    Show state's n.\n"),
    ("net_makepeer", "NetMakePeer", "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send 42 to peer.\n    Await response from peer into got.\n    Show got.\n"),
    ("net_send", "NetSend", "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send 42 to peer.\n    Await response from peer into got.\n    Show got.\n"),
    ("net_await", "NetAwait", "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send 42 to peer.\n    Await response from peer into got.\n    Show got.\n"),
    ("net_stream", "NetStream", "## Main\n    Listen on \"me\".\n    Let items be [1, 2, 3].\n    Stream items to \"sink\".\n    Show 7.\n"),
];

/// RAW-IR grind: ops the 44-pass optimizer LOWERS AWAY in an input-free program (`Div`/`DivPow2` strength-reduce
/// to `MagicDivU`/const-fold; `JumpIfTrue` short-circuit collapses once operands are known), so they never reach
/// the walker through the optimized path — yet the walker MUST lower any valid `CompiledProgram`. Each program's
/// UN-optimized bytecode contains the op (present assertion), and `C(raw) == VM(raw)`. This proves the walker's
/// defensive arms are correct, closing "the walker is only tested on optimizer output" as a coverage hole.
const RAW_GRIND_CORPUS: &[(&str, &str, &str)] = &[
    ("div_runtime", "Div", "## Main\n    Let a be 42.\n    Let b be 5.\n    Show a / b.\n"),
    ("div_pow2", "DivPow2", "## Main\n    Let a be 40.\n    Show a / 4.\n"),
    ("jump_if_true", "JumpIfTrue", "## Main\n    Let a be 3.\n    If a is less than 2 or a is greater than 5:\n        Show 1.\n    Otherwise:\n        Show 0.\n"),
];

// ─────────────────────────────────────────────────────────────────────────────────────────────────────────
//  THE TOTAL OP-CENSUS PARTITION — EVERY one of the 116 ISA ops is in EXACTLY ONE bucket, so nothing is
//  silently missed. `the_op_walker_census_partitions_the_whole_isa` asserts the four buckets are TOTAL (their
//  union is the whole ISA) and DISJOINT. As the generator grinds a TODO op, it moves to VERIFIED (and its name
//  appears in the fuzzer's op-coverage census). A new ISA op forces a decision here.
// ─────────────────────────────────────────────────────────────────────────────────────────────────────────

/// SEQUENTIAL ops proven ≡VM through the walker over the OPTIMIZED bytecode — the fuzzer's op-coverage census
/// drives most, and the deterministic `GRIND_CORPUS` pins the rest guaranteed-every-run (present-in-optimized +
/// C==VM). A name here that neither the census nor a grind case covers is drift caught by the fuzzer floor /
/// grind lock.
const OPS_VERIFIED: &[&str] = &[
    "Add",
    "AddAssign",
    "ApproxEq",
    "BindArm",
    "BitAnd",
    "BitOr",
    "BitXor",
    "Call",
    "CallBuiltin",
    "Concat",
    "CallValue",
    "Contains",
    "DeepClone",
    "DestructureTuple",
    "Eq",
    "ExactDiv",
    "FailWith",
    "FloorDiv",
    "FormatValue",
    "GetField",
    "GlobalGet",
    "GlobalSet",
    "Gt",
    "GtEq",
    "Halt",
    "Index",
    "IndexUnchecked",
    "IntersectOp",
    "IterNext",
    "IterPop",
    "IterPrepare",
    "Jump",
    "JumpIfFalse",
    "Length",
    "ListPop",
    "ListPush",
    "ListPushField",
    "LoadConst",
    "Lt",
    "LtEq",
    "MagicDivU",
    "MakeClosure",
    "Mod",
    "Move",
    "Mul",
    "NewEmptyList",
    "NewEmptyListWithCapacity",
    "NewEmptyMap",
    "NewEmptyMapWithCapacity",
    "NewEmptySet",
    "NewInductive",
    "NewList",
    "NewRange",
    "NewStruct",
    "NewTuple",
    "Not",
    "NotEq",
    "Pow",
    "RemoveFrom",
    "Return",
    "ReturnNothing",
    "RevAdd",
    "RevNeg",
    "RevSub",
    "RevSwap",
    "RevXor",
    "SeqConcat",
    "SetAdd",
    "SetIndex",
    "SetIndexUnchecked",
    "Shl",
    "Show",
    "Shr",
    "SliceOp",
    "StructInsert",
    "Sub",
    "TestArm",
    "UnionOp",
    // ── CRDT lattice + offline networking — the walker lowers these NATIVELY from the bytecode (mirroring
    // `codegen_riscv`); each is pinned ≡VM by a `GRIND_CORPUS` case (`crdt_*` / `net_*`). ──
    "NewCrdt",
    "CrdtBump",
    "CrdtMerge",
    "CrdtAppend",
    "CrdtResolve",
    "NetListen",
    "NetMakePeer",
    "NetSend",
    "NetStream",
    "NetSync",
    "NetAwait",
];
/// RAW-IR verified ops — the 44-pass optimizer LOWERS these away in an input-free program (so they never appear
/// in the optimized census), but the walker must lower ANY valid `CompiledProgram`, so `RAW_GRIND_CORPUS` feeds
/// it the UN-optimized bytecode where they DO appear and pins `C(raw) == VM(raw)`. Verified, just via the raw
/// path rather than the optimized census.
const OPS_VERIFIED_RAW: &[&str] = &["Div", "DivPow2", "JumpIfTrue"];
/// SEQUENTIAL ops the walker doesn't lower from the bytecode yet — SHRINK-ONLY: each moved to `OPS_VERIFIED`
/// (or `OPS_VERIFIED_RAW`) as its walker arm landed + a grind case pinned it ≡VM. ★ EMPTY: every sequential op
/// the walker CAN reach is now covered. The last arms landed this campaign — `ExactDiv` (the exact `LDec`
/// rational tower's `ldec_div`/`add`/`sub`/`mul` + `ldec_to_string`), `ListPushField` (typed `seq_*_push` into a
/// struct field), `FormatValue` with a format spec (mirroring the AST `c_format_spec_for_hole`), and
/// `DestructureTuple` over a `Seq-of-tuples` (the new `Kind::SeqTuple`). A newly-emittable op appears here first.
const OPS_COVERABLE_TODO: &[&str] = &[];
/// ASYNC / DELEGATED ops the flat single-threaded C op-walker still REFUSES: channels/tasks/select need a
/// SCHEDULER (covered on the stackless/CUDA tier, a separate port), and runtime authorization (`CheckPolicy`)
/// is DELEGATED to the AST-C backend. Here they are a documented, justified refusal — NOT a silent miss. (The
/// CRDT lattice `Crdt*`/`NewCrdt` and the offline networking transport `Net*` MOVED to `OPS_VERIFIED` in an
/// earlier campaign; the real-socket TCP client `Tcp` MOVED to `OPS_INTERNAL` this campaign — the walker now
/// lowers it NATIVELY over the `C_TCP_RUNTIME` POSIX socket runtime, witnessed by `tcp_walker_lock` — since,
/// like `NetConnect`, it dials a real socket and so has no deterministic OFFLINE ≡VM witness here.)
const OPS_ASYNC_REFUSED: &[&str] = &[
    "ChanNew",
    "ChanSend",
    "ChanRecv",
    "ChanClose",
    "ChanTrySend",
    "ChanTryRecv",
    "Spawn",
    "SpawnHandle",
    "Sleep",
    "SelectWait",
    "SelectArmRecv",
    "SelectArmTimeout",
    "TaskAwait",
    "TaskAbort",
    "CheckPolicy",
];
/// INTERNAL ops — NOT covered by the sequential-op grind, for one of three honest reasons:
///  • TRUE NO-OPS the walker lowers as nothing: optimizer markers `Obs`/`RegionBoundsGuard` (an emitted slow path
///    IS the correct code), the internal wide-bigint variant `BigIntWide`, compiler COW `EnsureOwned`, and the
///    i32-narrowing `NewEmptyListI32` (only fires for a genuinely runtime-sized non-constant-trip loop — which an
///    input-free program never has — and shares `NewEmptyList`'s arm, a width hint = transitively covered).
///  • HOST IO — `Args`/`LoadNow`/`LoadToday` read argv/wall-clock; nondeterministic, so not ≡VM-verifiable here.
///  • NOT SOURCE-EMITTABLE — `CrdtJoin` (the unconditional per-replica-MAX lattice join): no LOGOS surface emits
///    it (`Join x into y` does not parse), so it has no ≡VM witness — yet the walker HAS a native arm for it
///    (shared with `CrdtMerge`, mirroring `codegen_riscv`), so it is not a refusal, it is a covered internal.
///  • WALKER-COVERED but the ORACLE dials a real socket — `NetConnect` (`Connect to <addr>`) and `Tcp` (the
///    `tcp.lg` real-socket client): the walker lowers BOTH natively (`NetConnect` = the offline-loopback no-op;
///    `Tcp` = the real POSIX `C_TCP_RUNTIME` socket runtime), but they need a live socket / the Tokio async
///    reactor, so there is no deterministic OFFLINE ≡VM witness in this (sync, no-server) grind — like the
///    argv/wall-clock host-IO ops. `Tcp`'s real end-to-end ≡VM proof lives in `tcp_walker_lock` (a genuine TCP
///    round-trip + a refused connect, VM async ≡ walker C, byte-identical down to the OS-error wording).
/// (Closures `MakeClosure`/`CallValue` were here; they are now VERIFIED — the walker lowers inline-closure bodies
/// as their own regions [hole-subtracted from the enclosing block] + an indirect-call dispatch by body index.)
const OPS_INTERNAL: &[&str] = &[
    "Obs",
    "RegionBoundsGuard",
    "BigIntWide",
    "EnsureOwned",
    "Args",
    "LoadNow",
    "LoadToday",
    "NewEmptyListFixed",
    "NewEmptyListI32",
    "CrdtJoin",
    "NetConnect",
    "Tcp",
    "Serve",
];

#[test]
fn the_op_walker_census_partitions_the_whole_isa() {
    use std::collections::BTreeSet;
    let verified: BTreeSet<&str> = OPS_VERIFIED.iter().copied().collect();
    let verified_raw: BTreeSet<&str> = OPS_VERIFIED_RAW.iter().copied().collect();
    let todo: BTreeSet<&str> = OPS_COVERABLE_TODO.iter().copied().collect();
    let async_: BTreeSet<&str> = OPS_ASYNC_REFUSED.iter().copied().collect();
    let internal: BTreeSet<&str> = OPS_INTERNAL.iter().copied().collect();
    // No name may appear twice inside a bucket.
    assert_eq!(
        verified.len(),
        OPS_VERIFIED.len(),
        "duplicate in OPS_VERIFIED"
    );
    assert_eq!(
        verified_raw.len(),
        OPS_VERIFIED_RAW.len(),
        "duplicate in OPS_VERIFIED_RAW"
    );
    assert_eq!(
        todo.len(),
        OPS_COVERABLE_TODO.len(),
        "duplicate in OPS_COVERABLE_TODO"
    );
    assert_eq!(
        async_.len(),
        OPS_ASYNC_REFUSED.len(),
        "duplicate in OPS_ASYNC_REFUSED"
    );
    assert_eq!(
        internal.len(),
        OPS_INTERNAL.len(),
        "duplicate in OPS_INTERNAL"
    );
    // The five buckets are pairwise DISJOINT — every op has exactly one classification.
    let buckets = [
        ("verified", &verified),
        ("verified-raw", &verified_raw),
        ("coverable-todo", &todo),
        ("async-refused", &async_),
        ("internal", &internal),
    ];
    for (i, (na, a)) in buckets.iter().enumerate() {
        for (nb, b) in buckets.iter().skip(i + 1) {
            let overlap: Vec<&&str> = a.intersection(b).collect();
            assert!(
                overlap.is_empty(),
                "op(s) classified in both {na} and {nb}: {overlap:?}"
            );
        }
    }
    // The partition is TOTAL over the whole ISA — nothing is silently missed. A new ISA op bumps the size and
    // must be classified into a bucket here (the exhaustive per-backend `op_support` matches force the same
    // decision on riscv/cuda/wasm, and the fuzzer census would surface any newly-emittable op left uncovered).
    let total = verified.len() + verified_raw.len() + todo.len() + async_.len() + internal.len();
    // ★ DERIVED, NOT WRITTEN DOWN. This was `const ISA_SIZE: usize = 116;` — a literal, against an
    // ISA of 117 — so when `Serve` landed it fell into NO bucket and this test went on passing
    // while its own comment claimed "the partition is TOTAL over the whole ISA — nothing is
    // silently missed". A hand-copied denominator cannot enforce totality: it only ever measures
    // itself. Asking the ISA how big it is makes the claim true, and makes the next op that lands
    // unclassified fail here instead of disappearing.
    let isa_size = logicaffeine_compile::vm::Op::all_variants().len();
    assert_eq!(
        total, isa_size,
        "the op-census partition accounts for {total} ops but the ISA has {isa_size} — EVERY op must be classified \
         (verified / verified-raw / coverable-todo / async-refused / internal); add the new op to the right bucket."
    );
    // The TODO ledger is SHRINK-ONLY — as walker arms land, ops move todo→verified; it must only get smaller.
    const TODO_CEILING: usize = 0;
    assert!(
        todo.len() <= TODO_CEILING,
        "coverable-todo grew to {} (ceiling {TODO_CEILING}) — new coverable ops should be GROUND into the walker, not parked as TODO",
        todo.len()
    );
    eprintln!(
        "[c_bytecode PARTITION] {} verified ≡VM (opt) · {} verified ≡VM (raw) · {} coverable-todo · {} async-refused · {} internal = {}/{} ISA ops fully accounted for",
        verified.len(), verified_raw.len(), todo.len(), async_.len(), internal.len(), total, isa_size
    );
}

fn have_gcc() -> bool {
    common::require_tool("gcc")
}

/// Emit C via the Op-walker, gcc-compile, run at the harness instant ([`fixed_clock`]); `Ok(trimmed stdout)` or
/// `Err(reason)`.
fn c_bytecode_run(src: &str, label: &str) -> Result<String, String> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let c = compile_to_c_via_bytecode(src).map_err(|e| format!("emit refused: {e:?}"))?;
    let dir = std::env::temp_dir().join(format!("logos_bc_{}_{}_{}", std::process::id(), label, n));
    let _ = std::fs::create_dir_all(&dir);
    let cfile = dir.join("prog.c");
    let bin = dir.join("prog");
    std::fs::write(&cfile, &c).map_err(|e| e.to_string())?;
    let build = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&cfile)
        .arg("-lm")
        .output()
        .map_err(|e| e.to_string())?;
    if !build.status.success() {
        return Err(format!(
            "gcc failed:\n{}",
            String::from_utf8_lossy(&build.stderr)
                .lines()
                .take(4)
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    let run = std::process::Command::new(&bin)
        .envs(fixed_clock::env())
        .output()
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(String::from_utf8_lossy(&run.stdout).trim().to_string())
}

/// gcc-compile + run C the Op-walker emits directly from a `CompiledProgram` (no source round-trip). Used by the
/// RAW-IR lock: it feeds the walker the UN-optimized bytecode so ops the optimizer would lower away (`Div` →
/// `MagicDivU`/fold, `JumpIfTrue` short-circuit → `JumpIfFalse`) actually reach — and must be lowered by — the
/// walker. Proves the walker is correct over ANY valid `CompiledProgram`, not just the optimized subset.
fn c_bytecode_run_program(
    prog: &logicaffeine_compile::vm::CompiledProgram,
    label: &str,
) -> Result<String, String> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let c = compile_program_to_c(prog).map_err(|e| format!("emit refused: {e}"))?;
    let dir =
        std::env::temp_dir().join(format!("logos_bcr_{}_{}_{}", std::process::id(), label, n));
    let _ = std::fs::create_dir_all(&dir);
    let cfile = dir.join("prog.c");
    let bin = dir.join("prog");
    std::fs::write(&cfile, &c).map_err(|e| e.to_string())?;
    let build = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&cfile)
        .arg("-lm")
        .output()
        .map_err(|e| e.to_string())?;
    if !build.status.success() {
        return Err(format!(
            "gcc failed:\n{}",
            String::from_utf8_lossy(&build.stderr)
                .lines()
                .take(4)
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    let run = std::process::Command::new(&bin)
        .output()
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(String::from_utf8_lossy(&run.stdout).trim().to_string())
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────────────
//  THE DIFFERENTIAL FUZZER — put the Op-walker through the wringer over thousands of random programs.
//
//  A deterministic, seeded generator emits VALID, NON-ERRORING, i64-SAFE LOGOS programs across every facet the
//  Op-walker covers (int/float/bool/text scalars + arithmetic, comparisons, if/while control flow, list build /
//  index / length / for-each, sets, tuples, struct fields, the scalar-math builtins, floor-division by a
//  nonzero literal). "i64-safe" is load-bearing: values stay small so the VM never bignum-promotes, which means
//  `C(opt)` and `VM` must agree EXACTLY — every generated program that runs is a hard equality check. A single
//  `C ran but ≠ VM` is a silent miscompile and panics with the reproducing seed + source. Emitting C that gcc
//  cannot compile is likewise a hard failure (the Op-walker must emit valid C or cleanly REFUSE). A clean
//  refusal (a facet the walker hasn't modeled) is allowed and merely counted.
// ─────────────────────────────────────────────────────────────────────────────────────────────────────────

/// SplitMix64 — a tiny deterministic PRNG so every seed reproduces its exact program.
struct FRng(u64);
impl FRng {
    fn new(seed: u64) -> Self {
        FRng(seed.wrapping_add(0x9E37_79B9_7F4A_7C15))
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn pick<'a>(&mut self, xs: &'a [String]) -> &'a str {
        &xs[self.below(xs.len() as u64) as usize]
    }
}

/// Generate one valid, non-erroring, i64-safe program for `seed`. Every referenced variable is already bound,
/// every list index is in-range (1-based), every divisor is nonzero, and integer values stay small (no stored
/// multiply chains) so the run never overflows i64 into a bignum — keeping `C(opt) == VM` exact.
fn fuzz_program(seed: u64) -> String {
    let mut r = FRng::new(seed);
    let mut uid = 0usize;
    let fresh = |p: &str, uid: &mut usize| {
        *uid += 1;
        format!("{p}{uid}")
    };
    // A small toolkit of top-level definitions the Main body can draw on, so the generator exercises the
    // user-type + call ops too: a struct (`NewStruct`/`GetField`), an enum (`NewInductive`/`TestArm`/`BindArm`),
    // and a function (`Call`/`Return`). Unused ones are pruned by `keep_reachable_functions`, so they cost
    // nothing when a given seed doesn't touch them.
    let mut out = String::from(
        "## A FzPt has:\n    An x: Int.\n    A y: Int.\n\n\
         ## Definition\nA FzCol is either:\n    A FzRed.\n    A FzGreen.\n    A FzTag with a n, which is Int.\n\n\
         ## To fzdbl (n: Int) -> Int:\n    Return n + n.\n\n\
         ## To fzbump:\n    Set fzg to fzg + 1.\n\n\
         ## Main\n    Let mutable fzg be 0.\n",
    );
    let mut ints: Vec<String> = Vec::new();
    let mut floats: Vec<String> = Vec::new();

    // Seed a handful of small int + float variables to build on.
    for _ in 0..(2 + r.below(3)) {
        let v = fresh("iv", &mut uid);
        out += &format!("    Let mutable {v} be {}.\n", r.below(16));
        ints.push(v);
    }
    for _ in 0..(1 + r.below(2)) {
        let v = fresh("fv", &mut uid);
        out += &format!("    Let {v} be {}.{}.\n", r.below(10), r.below(10));
        floats.push(v);
    }

    let n = 5 + r.below(12);
    for _ in 0..n {
        match r.below(29) {
            0 => {
                // int arithmetic into a new var (+/- keep values linearly small)
                let (a, b) = (r.pick(&ints).to_string(), r.pick(&ints).to_string());
                let op = if r.below(2) == 0 { "+" } else { "-" };
                let v = fresh("iv", &mut uid);
                out += &format!("    Let {v} be {a} {op} {b}.\n");
                ints.push(v);
            }
            1 => {
                // one-shot multiply / floordiv (SHOWN, not stored — no compounding growth), nonzero divisor
                let a = r.pick(&ints).to_string();
                if r.below(2) == 0 {
                    out += &format!("    Show {a} * {}.\n", 1 + r.below(3));
                } else {
                    out += &format!("    Show {a} // {}.\n", 1 + r.below(5));
                }
            }
            2 => out += &format!("    Show {}.\n", r.pick(&ints)),
            3 => {
                // bounded accumulate loop (sum of 0..lim stays small)
                let acc = r.pick(&ints).to_string();
                let k = fresh("k", &mut uid);
                let lim = 3 + r.below(5);
                out += &format!(
                    "    Let mutable {k} be 0.\n    While {k} is less than {lim}:\n        Set {acc} to {acc} + {k}.\n        Set {k} to {k} + 1.\n"
                );
            }
            4 => {
                let (a, b) = (r.pick(&ints).to_string(), r.pick(&ints).to_string());
                out += &format!("    If {a} is less than {b}:\n        Show {a}.\n    Otherwise:\n        Show {b}.\n");
            }
            5 => {
                // list literal → index (1-based, in-range) + length + for-each sum
                let len = 3 + r.below(3);
                let items: Vec<String> = (0..len).map(|_| r.below(20).to_string()).collect();
                let xs = fresh("xs", &mut uid);
                out += &format!("    Let {xs} be [{}].\n", items.join(", "));
                out += &format!("    Show item {} of {xs}.\n", 1 + r.below(len));
                out += &format!("    Show length of {xs}.\n");
                let s = fresh("sm", &mut uid);
                let e = fresh("e", &mut uid);
                out += &format!(
                    "    Let mutable {s} be 0.\n    For each {e} in {xs}:\n        Set {s} to {s} + {e}.\n    Show {s}.\n"
                );
            }
            6 => {
                let (a, b) = (r.pick(&ints).to_string(), r.pick(&ints).to_string());
                let f = if r.below(2) == 0 { "min" } else { "max" };
                out += &format!("    Show {f}({a}, {b}).\n    Show abs({a} - {b}).\n");
            }
            7 => {
                let a = r.pick(&floats).to_string();
                out += &format!(
                    "    Show {a} + {}.{}.\n    Show sqrt({}.0).\n",
                    r.below(5),
                    r.below(10),
                    r.below(50)
                );
            }
            8 => {
                let (a, b) = (r.pick(&ints).to_string(), r.pick(&ints).to_string());
                let rel = [
                    "is at most",
                    "is less than",
                    "is greater than",
                    "is equal to",
                    "is at least",
                    "is not",
                ][r.below(6) as usize];
                out += &format!("    Show {a} {rel} {b}.\n");
            }
            9 => out += &format!("    Show \"txt{}\".\n", r.below(1000)),
            10 => {
                // anonymous tuple of two ints
                let (a, b) = (r.pick(&ints).to_string(), r.pick(&ints).to_string());
                let t = fresh("tp", &mut uid);
                out += &format!("    Let {t} be ({a}, {b}).\n    Show {t}.\n");
            }
            11 => {
                // set of small ints (dedup + length + RemoveFrom + membership)
                let s = fresh("st", &mut uid);
                out += &format!("    Let mutable {s} be a new Set of Int.\n");
                for _ in 0..(2 + r.below(4)) {
                    out += &format!("    Add {} to {s}.\n", r.below(8));
                }
                // Remove an element (RemoveFrom op) — a value that may or may not be present; both are valid.
                out += &format!("    Remove {} from {s}.\n", r.below(8));
                out += &format!("    Show length of {s}.\n    Show {s}.\n");
            }
            12 => {
                // ── bit-level ops ("bitblast" the walker's bitwise/shift lowering) — i64-safe: small operands
                // and a bounded shift so the shifted value never overflows into a bignum.
                let (a, b) = (r.pick(&ints).to_string(), r.pick(&ints).to_string());
                match r.below(5) {
                    0 => out += &format!("    Show {a} & {b}.\n"),
                    1 => out += &format!("    Show {a} | {b}.\n"),
                    2 => out += &format!("    Show {a} xor {b}.\n"),
                    3 => out += &format!("    Show {a} shifted left by {}.\n", 1 + r.below(4)),
                    _ => out += &format!("    Show {a} shifted right by {}.\n", 1 + r.below(3)),
                }
            }
            13 => {
                // ── a map: random Int/Text key×value, insert TRACKED entries (so a later `get` is never on an
                // absent key = a VM trap), exercise duplicate-key UPDATE, get an inserted key, length, `contains`
                // on a possibly-absent key (both branches over seeds), and whole-map Show (renderable values).
                let m = fresh("mp", &mut uid);
                let (key_text, val_text) = (r.below(2) == 0, r.below(2) == 0);
                let kty = if key_text { "Text" } else { "Int" };
                let vty = if val_text { "Text" } else { "Int" };
                out += &format!("    Let mutable {m} be a new Map of {kty} to {vty}.\n");
                // A KEY literal — an Int key is ≥1 (the parser rejects `item 0`, its 1-based-index guard, for any
                // collection); a Text key is `"k<n>"`. A VALUE literal may be any small Int (0 is a fine value).
                let key_lit = |text: bool, hi: u64, r: &mut FRng| {
                    if text {
                        format!("\"k{}\"", r.below(hi))
                    } else {
                        (1 + r.below(hi)).to_string()
                    }
                };
                let val_lit = |text: bool, hi: u64, r: &mut FRng| {
                    if text {
                        format!("\"v{}\"", r.below(hi))
                    } else {
                        r.below(hi).to_string()
                    }
                };
                let mut keys: Vec<String> = Vec::new();
                for _ in 0..(2 + r.below(4)) {
                    // small key space → duplicate keys recur → the insert exercises UPDATE, not just fresh keys
                    let k = key_lit(key_text, 5, &mut r);
                    let v = val_lit(val_text, 9, &mut r);
                    out += &format!("    Set item {k} of {m} to {v}.\n");
                    keys.push(k);
                }
                let gk = keys[r.below(keys.len() as u64) as usize].clone();
                out += &format!("    Show item {gk} of {m}.\n");
                out += &format!("    Show length of {m}.\n");
                let ck = key_lit(key_text, 8, &mut r); // may or may not be present → exercises both contains arms
                out += &format!(
                    "    If {m} contains {ck}:\n        Show 1.\n    Otherwise:\n        Show 0.\n"
                );
                out += &format!("    Show {m}.\n");
            }
            14 => {
                // a struct: create with two small fields, read them back (NewStruct + GetField)
                let p = fresh("pt", &mut uid);
                out += &format!(
                    "    Let {p} be a new FzPt with x {} and y {}.\n",
                    r.below(20),
                    r.below(20)
                );
                out += &format!("    Show {p}'s x + {p}'s y.\n");
            }
            15 => {
                // an enum: create a variant + Inspect (NewInductive + TestArm), with a payload BindArm on FzTag
                let c = fresh("col", &mut uid);
                match r.below(3) {
                    0 => out += &format!("    Let {c} be a new FzRed.\n"),
                    1 => out += &format!("    Let {c} be a new FzGreen.\n"),
                    _ => out += &format!("    Let {c} be a new FzTag with n {}.\n", r.below(20)),
                }
                out += &format!(
                    "    Inspect {c}:\n        If it is a FzRed:\n            Show 1.\n        If it is a FzGreen:\n            Show 2.\n        If it is a FzTag (n: v):\n            Show v.\n"
                );
            }
            16 => {
                // a function call (Call + Return); the arg stays small so the result never overflows
                out += &format!("    Show fzdbl({}).\n", r.below(20));
            }
            17 => {
                // modulo by a nonzero divisor (Mod op)
                let a = r.pick(&ints).to_string();
                out += &format!("    Show {a} mod {}.\n", 1 + r.below(9));
            }
            18 => {
                // a numeric range `for-each` over Int bounds: the counted loop, with no range value
                let fr = fresh("fr", &mut uid);
                let s = fresh("rs", &mut uid);
                out += &format!(
                    "    Let mutable {s} be 0.\n    For each {fr} from 1 to {}:\n        Set {s} to {s} + {fr}.\n    Show {s}.\n",
                    2 + r.below(5)
                );
            }
            19 => {
                // boolean negation (Not op)
                let (a, b) = (r.pick(&ints).to_string(), r.pick(&ints).to_string());
                out += &format!("    Show not ({a} is less than {b}).\n");
            }
            20 => {
                // grow-then-drain a sequence (NewEmptyList / Push / Pop) — 3 pushes then 1 pop keeps it non-empty
                let ps = fresh("ps", &mut uid);
                let pv = fresh("pv", &mut uid);
                out += &format!("    Let mutable {ps} be a new Seq of Int.\n");
                for _ in 0..3 {
                    out += &format!("    Push {} to {ps}.\n", r.below(20));
                }
                out += &format!(
                    "    Pop from {ps} into {pv}.\n    Show {pv}.\n    Show length of {ps}.\n"
                );
            }
            21 => {
                // small exponent (Pow op) — base stays small so the result never overflows i64
                let a = r.pick(&ints).to_string();
                out += &format!("    Show {a} ** {}.\n", 2 + r.below(2));
            }
            22 => {
                // string interpolation (FormatValue op) over a runtime int
                let a = r.pick(&ints).to_string();
                out += &format!("    Show \"n is {{{a}}}\".\n");
            }
            23 => {
                // approximate float equality (ApproxEq op) — the VM's exact tolerance
                let (a, b) = (r.pick(&floats).to_string(), r.pick(&floats).to_string());
                out += &format!("    Show {a} is approximately {b}.\n");
            }
            24 => {
                // a module GLOBAL (`fzg`) mutated INSIDE a callee (`fzbump`) — GlobalGet/GlobalSet, and the
                // adversarial stress test for the supercompiler's global-across-call soundness (the fixed bug):
                // Invariant A must still hold, i.e. `fzg` reads its post-loop value, not a folded constant.
                let times = 1 + r.below(4);
                for _ in 0..times {
                    out += "    fzbump().\n";
                }
                out += "    Show fzg.\n";
            }
            25 => {
                // subsequence slice (SliceOp) — `items lo through hi of xs`, with 1 ≤ lo ≤ hi ≤ len so the slice
                // is a valid non-empty run (empty slices are valid too, but this keeps the shape meaningful)
                let xs = fresh("sl", &mut uid);
                let len = 4 + r.below(3);
                let items: Vec<String> = (0..len).map(|_| r.below(20).to_string()).collect();
                out += &format!("    Let {xs} be [{}].\n", items.join(", "));
                let lo = 1 + r.below(len);
                let hi = lo + r.below(len - lo + 1);
                out += &format!("    Show items {lo} through {hi} of {xs}.\n");
            }
            26 => {
                // a for-each whose body writes the list it walks: it iterates the snapshot taken on entry
                // (IterPrepare + IterNext + IterPop), so the write never reaches the elements the loop reads
                let xs = fresh("wl", &mut uid);
                let len = 3 + r.below(3);
                let items: Vec<String> = (0..len).map(|_| r.below(20).to_string()).collect();
                out += &format!("    Let mutable {xs} be [{}].\n", items.join(", "));
                let s = fresh("ws", &mut uid);
                let e = fresh("we", &mut uid);
                out += &format!(
                    "    Let mutable {s} be 0.\n    For each {e} in {xs}:\n        Set item {} of {xs} to {e} + {}.\n        Set {s} to {s} + {e}.\n    Show {s}.\n    Show {xs}.\n",
                    1 + r.below(len),
                    1 + r.below(9)
                );
            }
            27 => {
                // a range whose end is a call's result, which the counted-range proof does not decide: a range value
                // and the iterator machine walk it (NewRange + IterPrepare + IterNext + IterPop)
                let fr = fresh("cr", &mut uid);
                let s = fresh("cs", &mut uid);
                out += &format!(
                    "    Let mutable {s} be 0.\n    For each {fr} from 1 to fzdbl({}):\n        Set {s} to {s} + {fr}.\n    Show {s}.\n",
                    1 + r.below(3)
                );
            }
            _ => {
                // text concat
                out += &format!(
                    "    Show \"a{}\" combined with \"b{}\".\n",
                    r.below(50),
                    r.below(50)
                );
            }
        }
    }
    // Always end with a Show so the program has an observable.
    out += &format!("    Show {}.\n", r.pick(&ints));
    out
}

/// Every generated program runs on the walker exactly as it runs on the VM: the same standard output, standard
/// error and exit status, byte for byte. A refusal, a program gcc cannot build, and a program the generator makes
/// that does not run are each a failure; the routine suite runs a fixed seed range, so every failure reproduces.
#[test]
fn bytecode_c_op_walker_fuzz_differential() {
    if !have_gcc() {
        return;
    }
    // The routine suite runs seeds `0..1000`; `LOGOS_C_FUZZ_SEEDS` widens the fixed range for a soak, and
    // `LOGOS_C_FUZZ_BASE` with `LOGOS_C_FUZZ_EXPLORE` runs that many seeds from another base.
    let seeds: u64 = std::env::var("LOGOS_C_FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);
    let explore: u64 = std::env::var("LOGOS_C_FUZZ_EXPLORE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let base: u64 = std::env::var("LOGOS_C_FUZZ_BASE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000_000);

    // One seed: the optimizer keeps the program's meaning (the VM over its raw and its optimized bytecode agree
    // exactly), and the walker's run of it is the VM's. Returns the ops of the optimized bytecode it verified.
    let check_seed = |seed: u64| -> Vec<&'static str> {
        let src = fuzz_program(seed);
        let raw = compile_to_program(&src).unwrap_or_else(|e| {
            panic!(
                "seed={seed}: the generated program does not compile: {e:?}\n--- source ---\n{src}"
            )
        });
        let opt = compile_to_program_optimized(&src)
            .unwrap_or_else(|e| panic!("seed={seed}: the generated program does not optimize: {e:?}\n--- source ---\n{src}"));
        let (want, opt_out) = (program_outcome(&raw), program_outcome(&opt));
        assert_eq!(
            (&want.output, &want.error),
            (&opt_out.output, &opt_out.error),
            "seed={seed}: the optimizer changed the program's meaning (the VM over the raw and the optimized bytecode \
             disagree)\n--- source ---\n{src}"
        );
        let vm = vm_observed(&src);
        let walker = walker_observed(&src, "fuzz")
            .unwrap_or_else(|why| panic!("seed={seed}: the walker did not run it: {why}\n--- source ---\n{src}\n--- vm ---\n{vm:?}"));
        assert_eq!(
            walker, vm,
            "seed={seed}: the walker's run differs from the VM's\n--- source ---\n{src}"
        );
        opt.code.iter().map(|o| o.name()).collect()
    };

    let mut verified_ops: std::collections::BTreeSet<&'static str> =
        std::collections::BTreeSet::new();
    for seed in 0..seeds {
        verified_ops.extend(check_seed(seed));
    }
    for seed in base..base + explore {
        check_seed(seed);
    }
    eprintln!(
        "[c_bytecode FUZZ] {seeds} fixed seeds and {explore} from base {base} run exactly as on the VM; {} distinct ops verified:\n    {}",
        verified_ops.len(),
        verified_ops.iter().copied().collect::<Vec<_>>().join(" ")
    );
    // The generator keeps exercising at least this many distinct ops end to end (source → optimize → bytecode → C →
    // gcc → run ≡ VM). Raise it as the generator grows; a drop is red.
    const OP_FLOOR: usize = 60;
    assert!(
        verified_ops.len() >= OP_FLOOR,
        "op coverage fell to {} distinct ops (floor {OP_FLOOR}) — the generator stopped exercising part of the surface",
        verified_ops.len()
    );
}

#[test]
fn the_bytecode_c_op_walker_matches_the_vm_on_per_op_edge_cases() {
    if !have_gcc() {
        return;
    }
    let mut covered = 0;
    for (label, src) in EDGE_CORPUS {
        let prog = compile_to_program(src)
            .unwrap_or_else(|e| panic!("[{label}] compile_to_program: {e:?}"));
        let want = program_outcome(&prog);
        let got = c_bytecode_run(src, label).unwrap_or_else(|e| {
            panic!(
                "[{label}] bytecode-C did not run — the Op-walker must lower this edge case:\n{e}"
            )
        });
        assert_eq!(
            got.as_str(),
            want.output.trim(),
            "[{label}] bytecode-C output != VM — the Op-walker miscompiled this edge case"
        );
        covered += 1;
    }
    eprintln!(
        "[c_bytecode] {covered}/{} curated per-op edge cases run byte-identical to the VM",
        EDGE_CORPUS.len()
    );
}

#[test]
fn the_bytecode_c_op_walker_lowers_each_optimized_survivor_op() {
    if !have_gcc() {
        return;
    }
    for (label, op, src) in GRIND_CORPUS {
        // The op must genuinely survive the 44-pass optimizer lattice into the bytecode the walker lowers — else
        // the case proves nothing about that op.
        let opt = compile_to_program_optimized(src)
            .unwrap_or_else(|e| panic!("[{label}] compile_to_program_optimized: {e:?}"));
        assert!(
            opt.code.iter().any(|o| o.name() == *op),
            "[{label}] optimized bytecode does NOT contain `{op}` — the 44-pass lattice lowered it away, so this \
             grind case is vacuous. Pick source where `{op}` survives, or move it to the RAW-IR grind."
        );
        let want = program_outcome(&opt);
        let got = c_bytecode_run(src, label).unwrap_or_else(|e| {
            panic!("[{label}] the Op-walker must lower `{op}` from the optimized bytecode — it refused/failed:\n{e}")
        });
        assert_eq!(
            got.as_str(),
            want.output.trim(),
            "[{label}] bytecode-C output != VM — the Op-walker miscompiled `{op}`"
        );
    }
    eprintln!(
        "[c_bytecode] {}/{} optimized-survivor ops lowered by the walker byte-identical to the VM",
        GRIND_CORPUS.len(),
        GRIND_CORPUS.len()
    );
}

#[test]
fn the_bytecode_c_op_walker_lowers_each_offline_net_op() {
    if !have_gcc() {
        return;
    }
    for (label, op, src) in NET_CORPUS {
        // Non-vacuous: the net op must survive into the optimized bytecode the walker lowers.
        let opt = compile_to_program_optimized(src)
            .unwrap_or_else(|e| panic!("[{label}] compile_to_program_optimized: {e:?}"));
        assert!(
            opt.code.iter().any(|o| o.name() == *op),
            "[{label}] optimized bytecode does NOT contain `{op}` — this net grind case is vacuous."
        );
        // The ASYNC oracle drives the offline loopback transport (the sync `program_outcome` VM suspends at the
        // blocking net op and never reaches the trailing `Show`), so compare the walker to THAT.
        let want = interpret_for_ui_sync(src).lines.join("\n");
        let got = c_bytecode_run(src, label).unwrap_or_else(|e| {
            panic!("[{label}] the Op-walker must lower `{op}` from the optimized bytecode — it refused/failed:\n{e}")
        });
        assert_eq!(
            got.as_str(),
            want.trim(),
            "[{label}] bytecode-C output != async-VM — the Op-walker miscompiled `{op}`"
        );
    }
    eprintln!(
        "[c_bytecode] {}/{} offline-net ops lowered by the walker byte-identical to the async VM",
        NET_CORPUS.len(),
        NET_CORPUS.len()
    );
}

#[test]
fn the_bytecode_c_op_walker_lowers_each_raw_ir_op() {
    if !have_gcc() {
        return;
    }
    for (label, op, src) in RAW_GRIND_CORPUS {
        // The walker is fed the UN-optimized bytecode, where these optimizer-lowered ops actually appear.
        let raw = compile_to_program(src)
            .unwrap_or_else(|e| panic!("[{label}] compile_to_program: {e:?}"));
        assert!(
            raw.code.iter().any(|o| o.name() == *op),
            "[{label}] raw bytecode does NOT contain `{op}` — the RAW-IR grind case no longer exercises it."
        );
        let want = program_outcome(&raw);
        let got = c_bytecode_run_program(&raw, label).unwrap_or_else(|e| {
            panic!("[{label}] the Op-walker must lower `{op}` from RAW bytecode — it refused/failed:\n{e}")
        });
        assert_eq!(
            got.as_str(),
            want.output.trim(),
            "[{label}] raw bytecode-C output != VM — the Op-walker miscompiled `{op}`"
        );
    }
    eprintln!("[c_bytecode] {}/{} raw-IR (optimizer-lowered) ops lowered by the walker byte-identical to the VM", RAW_GRIND_CORPUS.len(), RAW_GRIND_CORPUS.len());
}

#[test]
fn the_bytecode_c_op_walker_matches_the_vm_on_the_integer_core() {
    if !have_gcc() {
        return;
    }
    let mut covered = 0;
    for (label, src) in INT_CORPUS {
        let prog = compile_to_program(src)
            .unwrap_or_else(|e| panic!("[{label}] compile_to_program: {e:?}"));
        let want = program_outcome(&prog);
        let got = c_bytecode_run(src, label).unwrap_or_else(|e| {
            panic!("[{label}] bytecode-C did not run — the Op-walker must lower this program:\n{e}")
        });
        assert_eq!(
            got.as_str(),
            want.output.trim(),
            "[{label}] bytecode-C output != VM — the Op-walker miscompiled it"
        );
        covered += 1;
    }
    eprintln!(
        "[c_bytecode] {covered}/{} integer-core programs run byte-identical to the VM",
        INT_CORPUS.len()
    );
}

#[test]
fn the_bytecode_c_op_walker_matches_the_vm_on_the_scalar_kinds() {
    if !have_gcc() {
        return;
    }
    let mut covered = 0;
    for (label, src) in SCALAR_CORPUS {
        let prog = compile_to_program(src)
            .unwrap_or_else(|e| panic!("[{label}] compile_to_program: {e:?}"));
        let want = program_outcome(&prog);
        let got = c_bytecode_run(src, label).unwrap_or_else(|e| {
            panic!("[{label}] bytecode-C did not run — the Op-walker must lower this program:\n{e}")
        });
        assert_eq!(
            got.as_str(),
            want.output.trim(),
            "[{label}] bytecode-C output != VM — the Op-walker miscompiled it"
        );
        covered += 1;
    }
    eprintln!("[c_bytecode] {covered}/{} scalar-kind (bool/float/text) programs run byte-identical to the VM", SCALAR_CORPUS.len());
}

#[test]
fn the_bytecode_c_op_walker_matches_the_vm_on_collections() {
    if !have_gcc() {
        return;
    }
    let mut covered = 0;
    for (label, src) in COLLECTION_CORPUS {
        let prog = compile_to_program(src)
            .unwrap_or_else(|e| panic!("[{label}] compile_to_program: {e:?}"));
        let want = program_outcome(&prog);
        let got = c_bytecode_run(src, label).unwrap_or_else(|e| {
            panic!("[{label}] bytecode-C did not run — the Op-walker must lower this program:\n{e}")
        });
        assert_eq!(
            got.as_str(),
            want.output.trim(),
            "[{label}] bytecode-C output != VM — the Op-walker miscompiled it"
        );
        covered += 1;
    }
    eprintln!("[c_bytecode] {covered}/{} collection (seq/range/for-each) programs run byte-identical to the VM", COLLECTION_CORPUS.len());
}

#[test]
fn the_bytecode_c_op_walker_matches_the_vm_on_incremental_lists() {
    if !have_gcc() {
        return;
    }
    let mut covered = 0;
    for (label, src) in INCREMENTAL_LIST_CORPUS {
        let prog = compile_to_program(src)
            .unwrap_or_else(|e| panic!("[{label}] compile_to_program: {e:?}"));
        let want = program_outcome(&prog);
        let got = c_bytecode_run(src, label).unwrap_or_else(|e| {
            panic!("[{label}] bytecode-C did not run — the Op-walker must lower this program:\n{e}")
        });
        assert_eq!(
            got.as_str(),
            want.output.trim(),
            "[{label}] bytecode-C output != VM — the Op-walker miscompiled it"
        );
        covered += 1;
    }
    eprintln!("[c_bytecode] {covered}/{} incremental-list (new/push/pop/empty-iter) programs run byte-identical to the VM", INCREMENTAL_LIST_CORPUS.len());
}

#[test]
fn the_bytecode_c_op_walker_matches_the_vm_on_scalar_arithmetic() {
    if !have_gcc() {
        return;
    }
    let mut covered = 0;
    for (label, src) in SCALAR_ARITH_CORPUS {
        let prog = compile_to_program(src)
            .unwrap_or_else(|e| panic!("[{label}] compile_to_program: {e:?}"));
        let want = program_outcome(&prog);
        let got = c_bytecode_run(src, label).unwrap_or_else(|e| {
            panic!("[{label}] bytecode-C did not run — the Op-walker must lower this program:\n{e}")
        });
        assert_eq!(
            got.as_str(),
            want.output.trim(),
            "[{label}] bytecode-C output != VM — the Op-walker miscompiled it"
        );
        covered += 1;
    }
    eprintln!("[c_bytecode] {covered}/{} scalar-arith (pow/floordiv/bitwise/approx) programs run byte-identical to the VM", SCALAR_ARITH_CORPUS.len());
}

#[test]
fn the_bytecode_c_op_walker_matches_the_vm_on_scalar_builtins() {
    if !have_gcc() {
        return;
    }
    let mut covered = 0;
    for (label, src) in BUILTIN_CORPUS {
        let prog = compile_to_program(src)
            .unwrap_or_else(|e| panic!("[{label}] compile_to_program: {e:?}"));
        let want = program_outcome(&prog);
        let got = c_bytecode_run(src, label).unwrap_or_else(|e| {
            panic!("[{label}] bytecode-C did not run — the Op-walker must lower this builtin:\n{e}")
        });
        assert_eq!(
            got.as_str(),
            want.output.trim(),
            "[{label}] bytecode-C output != VM — the Op-walker miscompiled the builtin"
        );
        covered += 1;
    }
    // The loop-accumulated argument defeats constant-folding, so `CallBuiltin` genuinely survives into the
    // optimized IR and the Op-walker must emit the real `sqrt(` intrinsic — proof the arm is exercised, not
    // silently folded to a constant by CTFE.
    let runtime_src = BUILTIN_CORPUS
        .iter()
        .find(|(l, _)| *l == "sqrt_runtime")
        .unwrap()
        .1;
    let emitted = compile_to_c_via_bytecode(runtime_src).expect("sqrt_runtime must lower to C");
    assert!(
        emitted.contains("sqrt("),
        "the CallBuiltin arm did not fire — expected an emitted `sqrt(` intrinsic in the bytecode-C"
    );
    eprintln!("[c_bytecode] {covered}/{} scalar-builtin (sqrt/abs/floor/ceil/round/min/max) programs run byte-identical to the VM", BUILTIN_CORPUS.len());
}

#[test]
fn the_bytecode_c_op_walker_matches_the_vm_on_tuples() {
    if !have_gcc() {
        return;
    }
    let mut covered = 0;
    for (label, src) in TUPLE_CORPUS {
        let prog = compile_to_program(src)
            .unwrap_or_else(|e| panic!("[{label}] compile_to_program: {e:?}"));
        let want = program_outcome(&prog);
        let got = c_bytecode_run(src, label).unwrap_or_else(|e| {
            panic!("[{label}] bytecode-C did not run — the Op-walker must lower this tuple:\n{e}")
        });
        assert_eq!(
            got.as_str(),
            want.output.trim(),
            "[{label}] bytecode-C output != VM — the Op-walker miscompiled the tuple"
        );
        covered += 1;
    }
    // The function-returned tuple survives across the call boundary, so `NewTuple` genuinely reaches the
    // Op-walker — proof the arm is exercised, not folded away by CTFE.
    let survives = TUPLE_CORPUS
        .iter()
        .find(|(l, _)| *l == "tuple_return_destructure")
        .unwrap()
        .1;
    let emitted =
        compile_to_c_via_bytecode(survives).expect("tuple_return_destructure must lower to C");
    assert!(
        emitted.contains("_tup"),
        "the NewTuple arm did not fire — expected an emitted tuple buffer (`_tup…`) in the bytecode-C"
    );
    eprintln!("[c_bytecode] {covered}/{} tuple (construct/destructure/return/show) programs run byte-identical to the VM", TUPLE_CORPUS.len());
}

#[test]
fn the_bytecode_c_op_walker_matches_the_vm_on_maps() {
    if !have_gcc() {
        return;
    }
    let mut covered = 0;
    for (label, src) in MAP_CORPUS {
        let prog = compile_to_program(src)
            .unwrap_or_else(|e| panic!("[{label}] compile_to_program: {e:?}"));
        let want = program_outcome(&prog);
        let got = c_bytecode_run(src, label).unwrap_or_else(|e| {
            panic!("[{label}] bytecode-C did not run — the Op-walker must lower this map:\n{e}")
        });
        assert_eq!(
            got.as_str(),
            want.output.trim(),
            "[{label}] bytecode-C output != VM — the Op-walker miscompiled the map"
        );
        covered += 1;
    }
    // SAFETY: the VM permits any hashable key, but the C runtime only has typed maps for Int/Text keys and
    // values. A key/value type the walker cannot lower (a Float key here) must be REFUSED, never miscompiled
    // into a `Map_i64_i64` that stores the float's bits as an integer key. The VM runs these; the walker must
    // decline them — that is the "can't have differences" guarantee for the map facet.
    for (label, src) in &[
        ("map_float_key", "## Main\n    Let mutable m be a new Map of Float to Int.\n    Set item 1.5 of m to 10.\n    Show item 1.5 of m.\n"),
        ("map_bool_key", "## Main\n    Let mutable m be a new Map of Bool to Int.\n    Set item true of m to 1.\n    Show item true of m.\n"),
    ] {
        // Only assert refusal when the VM itself actually runs the program (so it is a genuine walker gap, not
        // a parse/VM error both sides share).
        //
        // ★ AND THE SKIP IS COUNTED. A refusal check that quietly stops running looks exactly like
        // a refusal check that passes, so if the VM ever stopped running these two the walker
        // would be free to miscompile a Float key with nothing to say so. The `checked` tally
        // below is the difference between "we asked" and "we were silent".
        let mut checked = 0usize;
        if let Ok(prog) = compile_to_program(src) {
            if program_outcome(&prog).error.is_none() {
                checked += 1;
                match c_bytecode_run(src, label) {
                    Err(e) if e.starts_with("emit refused") => {}
                    Err(e) if e.starts_with("gcc failed") => {}
                    other => panic!("[{label}] an unsupported map key type must REFUSE, not miscompile — got {other:?}"),
                }
            }
        }
        assert_eq!(
            checked, 1,
            "[{label}] the VM no longer runs this program, so the walker's REFUSAL was never \
             checked. The guarantee here is that a key type the walker cannot lower is declined \
             rather than miscompiled — that claim needs the program to run somewhere first."
        );
    }
    eprintln!("[c_bytecode] {covered}/{} map (create/insert/get/update/length/show/contains) programs run byte-identical to the VM (+unsupported key types safely refused)", MAP_CORPUS.len());
}

#[test]
fn the_bytecode_c_op_walker_matches_the_vm_on_structs() {
    if !have_gcc() {
        return;
    }
    let mut covered = 0;
    for (label, src) in STRUCT_CORPUS {
        let prog = compile_to_program(src)
            .unwrap_or_else(|e| panic!("[{label}] compile_to_program: {e:?}"));
        let want = program_outcome(&prog);
        let got = c_bytecode_run(src, label).unwrap_or_else(|e| {
            panic!("[{label}] bytecode-C did not run — the Op-walker must lower this program:\n{e}")
        });
        assert_eq!(
            got.as_str(),
            want.output.trim(),
            "[{label}] bytecode-C output != VM — the Op-walker miscompiled it"
        );
        covered += 1;
    }
    eprintln!("[c_bytecode] {covered}/{} struct (new/field-set/field-get, nested) programs run byte-identical to the VM", STRUCT_CORPUS.len());
}

#[test]
fn the_bytecode_c_op_walker_matches_the_vm_on_sets() {
    if !have_gcc() {
        return;
    }
    let mut covered = 0;
    for (label, src) in SET_CORPUS {
        let prog = compile_to_program(src)
            .unwrap_or_else(|e| panic!("[{label}] compile_to_program: {e:?}"));
        let want = program_outcome(&prog);
        let got = c_bytecode_run(src, label).unwrap_or_else(|e| {
            panic!("[{label}] bytecode-C did not run — the Op-walker must lower this program:\n{e}")
        });
        assert_eq!(
            got.as_str(),
            want.output.trim(),
            "[{label}] bytecode-C output != VM — the Op-walker miscompiled it"
        );
        covered += 1;
    }
    eprintln!("[c_bytecode] {covered}/{} set (new/add/contains/remove/len/show) programs run byte-identical to the VM", SET_CORPUS.len());
}

#[test]
fn the_bytecode_c_op_walker_matches_the_vm_on_enums() {
    if !have_gcc() {
        return;
    }
    let mut covered = 0;
    for (label, src) in ENUM_CORPUS {
        let prog = compile_to_program(src)
            .unwrap_or_else(|e| panic!("[{label}] compile_to_program: {e:?}"));
        let want = program_outcome(&prog);
        let got = c_bytecode_run(src, label).unwrap_or_else(|e| {
            panic!("[{label}] bytecode-C did not run — the Op-walker must lower this program:\n{e}")
        });
        assert_eq!(
            got.as_str(),
            want.output.trim(),
            "[{label}] bytecode-C output != VM — the Op-walker miscompiled it"
        );
        covered += 1;
    }
    eprintln!("[c_bytecode] {covered}/{} enum (new-inductive/test-arm/bind-arm) programs run byte-identical to the VM", ENUM_CORPUS.len());
}

/// ★ EXECUTION-CORRECTNESS RATCHET LOCK ★ — the HONEST coverage metric. Emit-success (the coverage ratchet)
/// only proves the Op-walker produced *some* C; this proves the C it produced RUNS byte-for-byte like the VM
/// across the whole plain corpus. Every plain non-error program is emitted, gcc-compiled, run, and its output
/// compared to the harvested VM truth. Three outcomes: CORRECT (ran ≡VM), WRONG (emitted but diverged — the
/// dangerous silent-miscompile, e.g. an i64 wrap where LOGOS uses bignum), or not-covered (a typed emit
/// refusal — honest). `CORRECT_FLOOR` ratchets UP; the WRONG set must stay EMPTY (a silent miscompile is the
/// worst failure — never ledger it away, FIX or REFUSE it). gcc-gated.
#[test]
fn bytecode_c_op_walker_execution_correctness_ratchet_lock() {
    if !have_gcc() {
        return;
    }
    let progs = gauntlet::harvested();
    let (mut correct, mut not_covered, mut build_fail) = (0usize, 0usize, 0usize);
    let mut wrong: Vec<(String, String, String)> = Vec::new();
    for (i, p) in progs.iter().enumerate() {
        if p.oracle != "plain" || p.expect_err {
            continue;
        }
        match c_bytecode_run(&p.src, &format!("exec{i}")) {
            Ok(got) => {
                if got == p.expect_out.trim() {
                    correct += 1;
                } else {
                    wrong.push((p.name.clone(), got, p.expect_out.trim().to_string()));
                }
            }
            Err(e) if e.starts_with("emit refused") => not_covered += 1,
            Err(_) => build_fail += 1,
        }
    }
    eprintln!(
        "[c_bytecode EXEC] correct {correct}  WRONG {}  not-covered {not_covered}  build/run-fail {build_fail}",
        wrong.len()
    );
    for (n, g, w) in wrong.iter().take(40) {
        eprintln!("  WRONG {n}: got {g:?} want {w:?}");
    }

    // The ONLY tolerated divergences — documented and SHRINK-ONLY. Either a FUNDAMENTAL semantic the WIP
    // Op-walker does not model yet (arbitrary-precision bignum, an Optional), OR a live OPTIMIZER soundness bug
    // this lock surfaced that the walker faithfully reproduces (VM(opt) ≠ VM(raw); the fix belongs in the
    // optimizer). NOT silently accepted as "covered" — pinned here, excluded from the run-correct count. Adding
    // to this list requires one of those two justifications; an actual op-walker miscompile must be FIXED.
    // ★ EMPTY — the bytecode→C walker has ZERO silent miscompiles over the whole corpus. Every historical
    // divergence was FIXED (never accepted): the constant-overflow bignum family (via the `lint_*` tower), the
    // pop-empty Optional (`Kind::Optional`), and even the RUNTIME closure-body overflow (closure bodies use exact
    // `lint_*` arithmetic — closing a hole the Rust/AST-C backends still share). A NEW wrong answer is RED.
    const KNOWN_DIVERGENCES: &[&str] = &[];
    let wrong_names: std::collections::BTreeSet<&str> =
        wrong.iter().map(|(n, _, _)| n.as_str()).collect();

    // A NEW silent miscompile (wrong, not in the ledger) is the worst possible failure — RED, fix or refuse it.
    let unexpected: Vec<&str> = wrong_names
        .iter()
        .copied()
        .filter(|n| !KNOWN_DIVERGENCES.contains(n))
        .collect();
    assert!(
        unexpected.is_empty(),
        "NEW bytecode→C silent miscompile(s) (ran but ≠VM, not in the known-divergence ledger): {unexpected:?} — \
         FIX the lowering or make it REFUSE; never accept a wrong answer."
    );
    // Shrink-only: a ledger entry that no longer diverges has been FIXED — remove it (keeps the ledger honest).
    let stale: Vec<&&str> = KNOWN_DIVERGENCES
        .iter()
        .filter(|k| !wrong_names.contains(**k))
        .collect();
    assert!(
        stale.is_empty(),
        "stale known-divergence ledger entries (now correct — REMOVE them): {stale:?}"
    );

    // The run-correctness floor ratchets UP as milestones land. Raise it to the printed `correct`; never lower.
    const CORRECT_FLOOR: usize = 540;
    assert!(
        correct >= CORRECT_FLOOR,
        "bytecode→C run-correctness REGRESSED: {correct} programs run ≡VM, floor is {CORRECT_FLOOR}"
    );
    if correct > CORRECT_FLOOR {
        eprintln!("[c_bytecode EXEC] ↑ run-correct rose to {correct} — RAISE CORRECT_FLOOR from {CORRECT_FLOOR} to {correct}");
    }
}

/// The harvested programs that read the clock record what the VM prints at the harness instant, so the ratchet,
/// which runs every binary at that instant, holds the walker to the VM on them too.
#[test]
fn the_harvested_clock_rows_are_what_the_vm_prints_at_the_harness_instant() {
    fixed_clock::set_vm();
    let clock_rows: Vec<_> = gauntlet::harvested()
        .into_iter()
        .filter(|p| matches!(p.op.as_str(), "LoadToday" | "LoadNow"))
        .collect();
    assert!(
        !clock_rows.is_empty(),
        "the harvested corpus reads the clock"
    );
    let mut differ = Vec::new();
    for p in &clock_rows {
        let vm = logicaffeine_compile::compile::vm_outcome(&p.src);
        if vm.error.is_some() || vm.output.trim() != p.expect_out.trim() {
            differ.push(format!(
                "{}: the VM prints {:?} (error {:?}), the corpus records {:?}",
                p.name,
                vm.output.trim(),
                vm.error,
                p.expect_out
            ));
        }
    }
    assert!(
        differ.is_empty(),
        "{} of {} clock rows are not what the VM prints at the harness instant:\n{}",
        differ.len(),
        clock_rows.len(),
        differ.join("\n")
    );
}

/// ★ COVERAGE RATCHET LOCK ★ — how far the bytecode→C Op-walker reaches across the full 687-program harvested
/// gauntlet corpus, emit-only (no gcc). A MONOTONE floor (`EMIT_COVERAGE_FLOOR`) traps coverage so it can only
/// rise: a broken op-walker arm drops the count below the floor → RED. It also prints the top unlowered-op /
/// unmodeled-kind reasons — the compass for the next milestone (chosen from DATA, not guesswork). When new ops
/// land and the count rises, RAISE the floor (the test tells you the new value); NEVER lower it to pass.
#[test]
fn bytecode_c_op_walker_coverage_ratchet_lock() {
    use std::collections::BTreeMap;
    let progs = gauntlet::harvested();
    let (mut ok, mut plain_ok, mut plain_total) = (0usize, 0usize, 0usize);
    let mut hist: BTreeMap<String, usize> = BTreeMap::new();
    for p in &progs {
        let is_plain = p.oracle == "plain" && !p.expect_err;
        if is_plain {
            plain_total += 1;
        }
        match compile_to_c_via_bytecode(&p.src) {
            Ok(_) => {
                ok += 1;
                if is_plain {
                    plain_ok += 1;
                }
            }
            Err(e) => {
                let msg = format!("{e:?}");
                let key = if let Some(i) = msg.find("op not yet lowered: ") {
                    let rest = &msg[i + "op not yet lowered: ".len()..];
                    let end = rest
                        .find(|c: char| c == '"' || c == '\\')
                        .unwrap_or(rest.len());
                    format!("op:{}", &rest[..end])
                } else if msg.contains("kind is not yet modeled") {
                    "show:unmodeled-kind".to_string()
                } else if let Some(i) = msg.find("Custom(\"") {
                    let rest = &msg[i + "Custom(\"".len()..];
                    let end = rest.find([':', '"']).unwrap_or(rest.len().min(48));
                    rest[..end.min(48)].to_string()
                } else {
                    "other".to_string()
                };
                *hist.entry(key).or_default() += 1;
            }
        }
    }
    let mut ranked: Vec<_> = hist.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    eprintln!(
        "[c_bytecode COVERAGE] emit-ok {ok}/{} total ({plain_ok}/{plain_total} plain non-err) — top unlowered reasons:",
        progs.len()
    );
    for (k, n) in ranked.iter().take(35) {
        eprintln!("  {n:4}  {k}");
    }

    // ── The ratchet: coverage over the plain (non-concurrent, non-error) corpus can only rise. ──────────────
    // Raise this to the printed `plain_ok` whenever a milestone lands; never lower it to make the suite pass —
    // a drop means an Op-walker arm regressed and must be fixed (rule 7: a failing lock is ALWAYS a regression).
    const EMIT_COVERAGE_FLOOR: usize = 445;
    assert!(
        plain_ok >= EMIT_COVERAGE_FLOOR,
        "bytecode→C emit-coverage REGRESSED: only {plain_ok} plain programs lower, floor is {EMIT_COVERAGE_FLOOR}. \
         An Op-walker arm broke — fix the emitter, do NOT lower the floor."
    );
    if plain_ok > EMIT_COVERAGE_FLOOR {
        eprintln!(
            "[c_bytecode COVERAGE] ↑ coverage rose to {plain_ok} — RAISE EMIT_COVERAGE_FLOOR from {EMIT_COVERAGE_FLOOR} to {plain_ok}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────────────
//  THE OBSERVATION LOCK — the walker's run IS the VM's run: the same bytes on standard output, the same bytes
//  on standard error and the same exit status, nothing trimmed. The reference is the VM driven the way the
//  program needs it: the synchronous VM, or the async host when the program does file or network IO.
// ─────────────────────────────────────────────────────────────────────────────────────────────────────────

/// Everything a run shows: both streams exactly and its exit status.
#[derive(Debug, PartialEq, Eq)]
struct Observed {
    stdout: String,
    stderr: String,
    status: i32,
}

/// The VM's run of `src`: every shown line ends in a newline on standard output, and a failure writes its
/// reason and a newline on standard error and exits 1.
fn vm_observed(src: &str) -> Observed {
    let needs_host =
        logicaffeine_compile::ui_bridge::with_parsed_program(src, |parsed, interner| {
            parsed
                .map(|(stmts, _, _)| {
                    logicaffeine_compile::interpreter::needs_async(stmts, interner)
                })
                .unwrap_or(false)
        });
    let (stdout, error) = if needs_host {
        let r = logicaffeine_compile::compile::vm_outcome_net(src);
        let stdout: String = if r.output.is_empty() {
            String::new()
        } else {
            r.output.split('\n').map(|l| format!("{l}\n")).collect()
        };
        (stdout, r.error)
    } else {
        let r = logicaffeine_compile::compile::vm_outcome(src);
        (r.output, r.error)
    };
    match error {
        None => Observed {
            stdout,
            stderr: String::new(),
            status: 0,
        },
        Some(why) => Observed {
            stdout,
            stderr: format!("{why}\n"),
            status: 1,
        },
    }
}

/// The walker's run of `src`: emitted, built by gcc and run in a directory of its own.
fn walker_observed(src: &str, label: &str) -> Result<Observed, String> {
    let c = compile_to_c_via_bytecode(src).map_err(|e| format!("emit refused: {e:?}"))?;
    observed_c(&c, label)
}

/// The run of a C translation unit `c`: built by gcc and run in a directory of its own.
fn observed_c(c: &str, label: &str) -> Result<Observed, String> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("logos_bco_{}_{}_{}", std::process::id(), label, n));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("prog.c"), &c).map_err(|e| e.to_string())?;
    let build = std::process::Command::new("gcc")
        .args(["-O2", "-o", "prog", "prog.c", "-lm"])
        .current_dir(&dir)
        .output()
        .map_err(|e| e.to_string())?;
    if !build.status.success() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(format!(
            "gcc failed:\n{}",
            String::from_utf8_lossy(&build.stderr)
                .lines()
                .take(6)
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    let mut child = std::process::Command::new(dir.join("prog"))
        .current_dir(&dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => break status,
            None if started.elapsed() > std::time::Duration::from_secs(30) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_dir_all(&dir);
                return Err("the program ran past 30 s".to_string());
            }
            None => std::thread::sleep(std::time::Duration::from_millis(5)),
        }
    };
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    use std::os::unix::process::ExitStatusExt;
    let status = status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0));
    Ok(Observed {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        status,
    })
}

/// Every program of `corpus` runs on the walker exactly as it runs on the VM. Every difference is reported at
/// once, each with the program that shows it.
fn assert_walker_observes_as_the_vm(corpus: &[(&str, &str)]) {
    if !have_gcc() {
        return;
    }
    let mut differences = Vec::new();
    for (label, src) in corpus {
        let want = vm_observed(src);
        match walker_observed(src, label) {
            Ok(got) if got == want => {}
            Ok(got) => differences.push(format!(
                "[{label}]\n  walker {got:?}\n  vm     {want:?}\n  source {src:?}"
            )),
            Err(why) => differences.push(format!(
                "[{label}] the walker did not run it: {why}\n  vm     {want:?}\n  source {src:?}"
            )),
        }
    }
    assert!(
        differences.is_empty(),
        "{} of {} programs run differently on the walker:\n{}",
        differences.len(),
        corpus.len(),
        differences.join("\n")
    );
}

/// A collection has value semantics: a change made through one name never reaches another, unless the name is
/// a `mutable` parameter, which is the caller's own collection.
const VALUE_SEMANTICS_CORPUS: &[(&str, &str)] = &[
    ("every_where_keeps_its_source", "## Main\nLet xs be [1, 5, 2, 8, 3].\nLet small be every n in xs where n is less than 4.\nShow small.\n"),
    ("list_alias_set_and_push", "## Main\nLet mutable xs be [1, 2, 3].\nLet ys be xs.\nSet item 1 of xs to 9.\nShow ys.\nShow xs.\nPush 4 to xs.\nShow ys.\nShow xs.\n"),
    ("list_alias_pop", "## Main\nLet mutable xs be [1, 2, 3].\nLet ys be xs.\nPop from xs into last.\nShow last.\nShow xs.\nShow ys.\n"),
    ("map_alias", "## Main\nLet mutable m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\nLet n be m.\nSet item \"a\" of m to 2.\nShow item \"a\" of n.\nShow item \"a\" of m.\nLet mutable k be m.\nSet item \"b\" of k to 3.\nShow length of m.\nShow length of k.\n"),
    ("set_alias", "## Main\nLet mutable s be a new Set of Int.\nAdd 1 to s.\nLet t be s.\nAdd 2 to s.\nShow length of t.\nShow length of s.\nRemove 1 from s.\nShow t.\nShow s.\n"),
    ("struct_alias_field", "## A Point has:\n    An x: Int.\n\n## Main\nLet mutable p be a new Point with x 1.\nLet q be p.\nSet p's x to 5.\nShow q's x.\nShow p's x.\n"),
    ("struct_alias_field_list", "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet mutable p be a new Bag with items [1, 2].\nLet q be p.\nPush 5 to p's items.\nShow length of q's items.\nShow length of p's items.\n"),
    ("list_into_a_field", "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet xs be [1, 2].\nLet mutable b be a new Bag with items xs.\nPush 3 to b's items.\nShow xs.\nShow b's items.\n"),
    ("field_list_read_then_pushed", "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet mutable b be a new Bag with items [1].\nLet mutable l be b's items.\nPush 2 to l.\nPush 3 to b's items.\nShow l.\nShow b's items.\n"),
    ("parameter_is_a_copy", "## To grow (xs: Seq of Int) -> Int:\n    Push 9 to xs.\n    Return length of xs.\n\n## Main\nLet mutable xs be [1, 2].\nShow grow(xs).\nShow xs.\n"),
    ("mutable_parameter_is_the_callers", "## To grow (xs: mutable Seq of Int):\n    Push 9 to xs.\n\n## Main\nLet mutable xs be [1, 2].\nLet ys be xs.\nCall grow with xs.\nShow xs.\nShow ys.\n"),
    ("iteration_sees_the_list_it_started_with", "## Main\nLet mutable xs be [1, 2, 3].\nFor each x in xs:\n    Push x * 10 to xs.\nShow xs.\n"),
    ("global_changed_by_a_function", "## To bump:\n    Push 4 to xs.\n\n## Main\nLet mutable xs be [1].\nLet ys be xs.\nCall bump.\nShow xs.\nShow ys.\n"),
    ("returned_list_is_its_own", "## To make -> Seq of Int:\n    Return [1, 2].\n\n## Main\nLet mutable a be make().\nLet b be a.\nPush 3 to a.\nShow a.\nShow b.\n"),
];

#[test]
fn the_walker_keeps_value_semantics_through_every_alias() {
    assert_walker_observes_as_the_vm(VALUE_SEMANTICS_CORPUS);
}

/// One register holds one variable's collections, then a later variable's of another kind: each new sequence
/// gets a buffer of its own kind, never the earlier collection emptied in place (a `Seq of Bool` holds a byte per
/// element where a `Seq of Int` writes eight).
const REGISTER_REUSE_CORPUS: &[(&str, &str)] = &[
    (
        "bools_then_ints",
        "## To build (n: Int) -> Int:\n    Let mutable total be 0.\n    Let mutable i be 0.\n    While i is less than n:\n        Let mutable bs be a new Seq of Bool.\n        Let mutable j be 0.\n        While j is less than n:\n            Push true to bs.\n            Set j to j + 1.\n        Set total to total + length of bs.\n        Set i to i + 1.\n    Set i to 0.\n    While i is less than n:\n        Let mutable xs be a new Seq of Int.\n        Let mutable j be 0.\n        While j is less than n * 10:\n            Push j to xs.\n            Set j to j + 1.\n        Set total to total + length of xs + item (n * 10) of xs.\n        Set i to i + 1.\n    Return total.\n\n## Main\nShow build(5).\nShow build(9).\n",
    ),
    (
        "a_set_then_ints",
        "## To build (n: Int) -> Int:\n    Let mutable total be 0.\n    Let mutable i be 0.\n    While i is less than n:\n        Let mutable seen be a new Set of Int.\n        Let mutable j be 0.\n        While j is less than n:\n            Add j to seen.\n            Set j to j + 1.\n        Set total to total + length of seen.\n        Set i to i + 1.\n    Set i to 0.\n    While i is less than n:\n        Let mutable xs be a new Seq of Int.\n        Let mutable j be 0.\n        While j is less than n * 10:\n            Push j to xs.\n            Set j to j + 1.\n        Set total to total + length of xs + item (n * 10) of xs.\n        Set i to i + 1.\n    Return total.\n\n## Main\nShow build(5).\nShow build(9).\n",
    ),
];

#[test]
fn a_new_sequence_never_takes_the_buffer_of_another_kind() {
    assert_walker_observes_as_the_vm(REGISTER_REUSE_CORPUS);
}

/// `Option` and `Result` are enums nobody declares: the walker builds, tests and binds them by constructor
/// name exactly as the VM does, and an `Option` shows as its payload or `nothing`.
const GENERIC_INDUCTIVE_CORPUS: &[(&str, &str)] = &[
    ("some_arm", "## Main\nLet v be some 0.\nInspect v:\n    When Some (n):\n        Show \"some\".\n    When None:\n        Show \"none\".\n"),
    ("none_arm", "## Main\nLet v be none.\nInspect v:\n    When Some (n):\n        Show \"some\".\n    When None:\n        Show \"none\".\n"),
    ("ok_from_a_function", "## To f (n: Int) -> Result of Int and Text:\n    Return a new Ok with value n.\n\n## Main\nInspect f(7):\n    When Ok (v):\n        Show v.\n    When Err (e):\n        Show e.\n"),
    ("err_and_ok", "## To half (n: Int) -> Result of Int and Text:\n    If n is less than 0:\n        Return a new Err with error \"negative\".\n    Return a new Ok with value n / 2.\n\n## Main\nInspect half(0 - 4):\n    When Ok (v):\n        Show v.\n    When Err (e):\n        Show e.\nInspect half(10):\n    When Ok (v):\n        Show v.\n    When Err (e):\n        Show e.\n"),
    ("an_option_shows_as_its_payload", "## Main\nLet v be some 42.\nShow v.\nLet w be none.\nShow w.\n"),
    ("option_of_text_from_a_function", "## To find (xs: Seq of Text) and (w: Text) -> Option of Text:\n    For each x in xs:\n        If x equals w:\n            Return some x.\n    Return none.\n\n## Main\nLet names be [\"ann\", \"bo\"].\nInspect find(names, \"bo\"):\n    When Some (n):\n        Show n.\n    When None:\n        Show \"missing\".\nInspect find(names, \"cy\"):\n    When Some (n):\n        Show n.\n    When None:\n        Show \"missing\".\n"),
    ("option_set_on_one_branch", "## Main\nLet x be 5.\nLet mutable v be none.\nIf x is greater than 2:\n    Set v to some x * 3.\nInspect v:\n    When Some (n):\n        Show n.\n    When None:\n        Show \"none\".\n"),
];

#[test]
fn the_walker_builds_tests_and_binds_every_generic_inductive() {
    assert_walker_observes_as_the_vm(GENERIC_INDUCTIVE_CORPUS);
}

/// Text and its UTF-8 bytes, both ways: the byte builtins the whole string library stands on.
const TEXT_BYTES_CORPUS: &[(&str, &str)] = &[
    ("upper", "## Main\nShow upper(\"hi\").\n"),
    ("bytes_both_ways", "## Main\nShow text_bytes(\"hé\").\nShow text_from_bytes([104, 105]).\nShow lower(\"HeLLo\").\n"),
    ("invalid_utf8", "## Main\nShow 1.\nShow text_from_bytes([104, 255, 105]).\n"),
    ("overlong_lead", "## Main\nShow 1.\nShow text_from_bytes([104, 192, 128]).\n"),
    ("surrogate", "## Main\nShow text_from_bytes([104, 237, 160, 128]).\n"),
    ("past_the_last_scalar", "## Main\nShow text_from_bytes([244, 144, 128, 128]).\n"),
    ("no_lead_byte_f5", "## Main\nShow text_from_bytes([65, 245, 128]).\n"),
    ("cut_short", "## Main\nShow text_from_bytes([226, 130]).\n"),
    ("every_width", "## Main\nShow text_from_bytes([226, 130, 172, 240, 159, 152, 128, 223, 191, 194, 128]).\n"),
];

#[test]
fn the_walker_turns_text_into_bytes_and_back() {
    assert_walker_observes_as_the_vm(TEXT_BYTES_CORPUS);
}

/// A sequence made empty holds nothing, whatever it was declared to hold, and every question about it is
/// answered as the VM answers it.
const EMPTY_SEQUENCE_CORPUS: &[(&str, &str)] = &[
    ("with_capacity", "## Main\nLet a be a new Seq of Int with capacity 4.\nShow length of a.\n"),
    ("manifest", "## Main\nLet m be manifest of \"main\".\nShow length of m.\n"),
    ("empty_text_seq", "## Main\nLet xs be a new Seq of Text.\nShow length of xs.\nShow xs contains \"a\".\nLet e be [].\nShow length of e.\n"),
    ("item_of_an_empty_seq", "## Main\nShow 1.\nLet xs be a new Seq of Int.\nShow item 1 of xs.\n"),
    ("loop_over_a_list_nothing_fills", "## A Point has:\n    An x: Int.\n\n## Main\nLet e be [].\nFor each p in e:\n    Show p's x.\nShow length of e.\n"),
];

#[test]
fn the_walker_answers_every_question_about_an_empty_sequence() {
    assert_walker_observes_as_the_vm(EMPTY_SEQUENCE_CORPUS);
}

/// A tuple handed to a `Pair` parameter arrives as the tuple it is.
const TUPLE_PARAMETER_CORPUS: &[(&str, &str)] = &[
    ("pair_of_ints", "## To addpair (p: Pair of Int and Int) -> Int:\n    Return item 1 of p + item 2 of p.\n\n## Main\nLet p be (3, 4).\nShow addpair(p).\n"),
    ("pair_shown_in_the_callee", "## To total (p: Pair of Int and Int) -> Int:\n    Show p.\n    Return item 1 of p + item 2 of p.\n\n## Main\nShow total((5, 6)).\n"),
    ("pair_of_int_and_text", "## To describe (p: Pair of Int and Text) -> Text:\n    Show p.\n    Return item 2 of p.\n\n## Main\nLet p be (3, \"x\").\nShow describe(p).\n"),
];

#[test]
fn the_walker_passes_a_tuple_as_itself() {
    assert_walker_observes_as_the_vm(TUPLE_PARAMETER_CORPUS);
}

/// A function value's result has the kind its body computes.
const FUNCTION_VALUE_CORPUS: &[(&str, &str)] = &[
    ("doubling", "## Main\nLet f be (x: Int) -> x * 2.\nShow f(21).\n"),
    ("text_and_bool", "## Main\nLet g be (s: Text) -> s combined with \"!\".\nShow g(\"hi\").\nLet h be (n: Int) -> n is greater than 2.\nShow h(5).\n"),
];

#[test]
fn the_walker_types_a_function_values_result() {
    assert_walker_observes_as_the_vm(FUNCTION_VALUE_CORPUS);
}

/// Offline networking: a message keeps its kind, a peer is a peer, `Await` needs a `Listen`, and an address
/// the transport cannot dial fails the run as the VM fails it.
const NETWORK_CORPUS: &[(&str, &str)] = &[
    ("await_before_listen", "## Main\nSpawn a Worker called \"w1\".\nSend 7 to \"w1\".\nAwait response from \"w1\" into reply.\nShow reply.\n"),
    ("text_message", "## Main\nListen on \"me\".\nLet peer be a PeerAgent at \"me\".\nSend \"hello\" to peer.\nAwait response from peer into got.\nShow got.\n"),
    ("bool_message", "## Main\nListen on \"me\".\nLet peer be a PeerAgent at \"me\".\nSend true to peer.\nAwait response from peer into got.\nShow got.\n"),
    ("float_message", "## Main\nListen on \"me\".\nLet peer be a PeerAgent at \"me\".\nSend 2.5 to peer.\nAwait response from peer into got.\nShow got.\n"),
    ("seq_message", "## Main\nListen on \"me\".\nLet peer be a PeerAgent at \"me\".\nSend [4, 5] to peer.\nAwait response from peer into got.\nShow got.\nShow length of got.\n"),
    ("two_messages_in_order", "## Main\nListen on \"me\".\nLet peer be a PeerAgent at \"me\".\nSend 1 to peer.\nSend 2 to peer.\nAwait response from peer into a.\nAwait response from peer into b.\nShow a + b.\n"),
    ("stream", "## Main\nListen on \"me\".\nLet peer be a PeerAgent at \"me\".\nLet vals be [1, 2, 3].\nStream vals to peer.\nAwait stream from peer into b.\nShow item 2 of b.\nShow length of b.\n"),
    ("a_peer_shows_its_topic", "## Main\nListen on \"/ip4/127.0.0.1/tcp/9944\".\nLet peer be a PeerAgent at \"ws://127.0.0.1:9944\".\nShow peer.\nSend \"multi\" to peer.\nAwait response from peer into got.\nShow got.\n"),
    ("connect_offline", "## Main\nConnect to \"ws://127.0.0.1:9\".\nListen on \"me\".\nLet peer be a PeerAgent at \"me\".\nSend 5 to peer.\nAwait response from peer into got.\nShow got.\n"),
    ("connect_to_no_address", "## Main\nShow 1.\nConnect to \"nowhere\".\nShow 2.\n"),
    ("send_to_a_number", "## Main\nListen on \"me\".\nSend 3 to 42.\nShow 1.\n"),
];

#[test]
fn the_walker_runs_the_offline_network_as_the_vm_does() {
    assert_walker_observes_as_the_vm(NETWORK_CORPUS);
}

/// A server's routes answer when called directly: the router's helpers the compiler writes calls to survive
/// the walker's tree-shake.
const ROUTER_CORPUS: &[(&str, &str)] = &[
    ("route", "## Route GET /hi -> Text:\n    Return \"hello\".\n\n## Main\nShow route(\"GET\", \"/hi\").\n"),
    ("redirect", "## Redirect /old -> /new.\n\n## Route GET /new -> Text:\n    Return \"moved\".\n\n## Main\nShow routeStatus(\"GET\", \"/old\").\n"),
    ("middleware", "## Before each request (request: Request) -> Option of Response:\n    Return none.\n\n## Route GET /hi -> Text:\n    Return \"hello\".\n\n## Main\nShow route(\"GET\", \"/hi\").\n"),
];

#[test]
fn the_walker_keeps_every_function_the_router_calls() {
    assert_walker_observes_as_the_vm(ROUTER_CORPUS);
}

/// File statements run against the sandboxed file system the VM runs them against.
const FILE_CORPUS: &[(&str, &str)] = &[
    ("write_then_read", "## Main\nWrite \"hello\" to file \"read.txt\".\nRead contents from file \"read.txt\".\nShow contents.\n"),
    ("read_a_missing_file", "## Main\nShow 1.\nRead contents from file \"nope.txt\".\nShow contents.\n"),
    ("write_a_number", "## Main\nWrite 42 to file \"n.txt\".\nShow \"written\".\n"),
];

#[test]
fn the_walker_reads_and_writes_the_sandboxed_file_system() {
    assert_walker_observes_as_the_vm(FILE_CORPUS);
}

/// A failure the VM reports while running is reported in its words, after exactly the output before it.
const RUNTIME_FAILURE_CORPUS: &[(&str, &str)] = &[
    (
        "division_by_zero",
        "## Main\nLet a be 7.\nLet b be 0.\nShow 1.\nShow a / b.\n",
    ),
    (
        "modulo_by_zero",
        "## Main\nLet a be 7.\nLet b be 0.\nShow 1.\nShow a % b.\n",
    ),
    (
        "float_division_by_zero",
        "## Main\nLet a be 7.0.\nLet b be 0.0.\nShow 1.\nShow a / b.\n",
    ),
    (
        "index_past_the_end",
        "## Main\nLet xs be [1, 2].\nShow 1.\nShow item 5 of xs.\n",
    ),
    (
        "set_index_past_the_end",
        "## Main\nLet mutable xs be [1, 2].\nShow 1.\nSet item 3 of xs to 5.\nShow xs.\n",
    ),
];

#[test]
fn the_walker_fails_in_the_vms_words() {
    assert_walker_observes_as_the_vm(RUNTIME_FAILURE_CORPUS);
}

/// Addresses as a relay reads them — `ws://` URLs, multiaddrs, bare names, whitespace Rust trims, ports Rust
/// parses or rejects, every protocol segment — each run as a `Listen` + `PeerAgent` + round trip and as a
/// `Connect`, so the walker's reading of every one is held to the VM's, refusals included.
const ADDRESSES: &[&str] = &[
    "me",
    "  me  ",
    "\u{a0}me\u{3000}",
    "ws://Host:1",
    "WS://HOST:1",
    "wss://h:2",
    "/ip4/127.0.0.1/tcp/9944",
    "/ip4/1.2.3.4/tcp/80/ws",
    "/ip4/1.2.3.4/tcp/80/wss",
    "/ip4/1.2.3.4/tcp/80/tls",
    "/dns4/example.com/tcp/443/wss/p2p/QmPeer",
    "/ip6/::1/tcp/9944",
    "/ip4/h/tcp/+80",
    "/ip4/h/tcp/0080",
    "/ip4/h/tcp/65535",
    "/ip4/h/tcp/65536",
    "/ip4/h/tcp/8a",
    "/ip4/h/tcp/",
    "/ip4//tcp/1",
    "/ip4",
    "/tcp/1",
    "/ip4/h",
    "/p2p-circuit/ip4/h/tcp/1",
    "/udp/1",
    "/ip4/h/tcp/1/",
    "/dnsaddr/x/tcp/9",
    "/dns6/x/tcp/9",
    "/dns/x/tcp/x",
    " /ip4/h/tcp/1 ",
    "",
    "   ",
];

#[test]
fn the_walker_reads_every_address_as_the_vm_does() {
    let mut corpus: Vec<(String, String)> = Vec::new();
    for (i, a) in ADDRESSES.iter().enumerate() {
        corpus.push((
            format!("topic_{i}"),
            format!("## Main\nListen on \"{a}\".\nLet peer be a PeerAgent at \"{a}\".\nShow peer.\nSend {i} to peer.\nAwait response from peer into got.\nShow got.\n"),
        ));
        corpus.push((
            format!("connect_{i}"),
            format!("## Main\nShow 0.\nConnect to \"{a}\".\nShow 1.\n"),
        ));
    }
    let borrowed: Vec<(&str, &str)> = corpus
        .iter()
        .map(|(l, s)| (l.as_str(), s.as_str()))
        .collect();
    assert_walker_observes_as_the_vm(&borrowed);
}

/// A text grown by appending in a loop, then measured and read character by character — the strings benchmark's
/// shape. Its size comes from the command line.
const GROWING_TEXT: &str = "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\n## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\nLet result be \"\".\nLet i be 0.\nWhile i is less than n:\n    Set result to result + i + \" \".\n    Set i to i + 1.\nLet count be 0.\nSet i to 1.\nWhile i is at most length of result:\n    If item i of result equals \" \":\n        Set count to count + 1.\n    Set i to i + 1.\nShow count.\n";

/// The walker grows a text in place: at a size where a copy per append (and a rescan per `length of` /
/// `item i of`) would need tens of gigabytes and hours, the program runs in a small fixed address space and a
/// few seconds, and prints what the VM prints.
#[test]
fn the_walker_grows_a_text_in_linear_memory_and_time() {
    if !have_gcc() {
        return;
    }
    const N: &str = "300000";
    let want = logicaffeine_compile::compile::vm_outcome_with_args(
        GROWING_TEXT,
        &["prog".to_string(), N.to_string()],
        None,
    );
    assert_eq!(want.error, None, "the VM runs the growing text");
    let c = compile_to_c_via_bytecode(GROWING_TEXT).expect("the walker lowers the growing text");
    let dir = std::env::temp_dir().join(format!("logos_bco_text_growth_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("prog.c"), &c).expect("write C");
    let build = std::process::Command::new("gcc")
        .args(["-O2", "-o", "prog", "prog.c", "-lm"])
        .current_dir(&dir)
        .output()
        .expect("gcc");
    assert!(
        build.status.success(),
        "gcc: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    // 256 MiB of address space: a copy per append needs Σ 7i bytes ≈ 3×10¹¹ at this size.
    let started = std::time::Instant::now();
    let run = std::process::Command::new("sh")
        .args(["-c", &format!("ulimit -v 262144; exec ./prog {N}")])
        .current_dir(&dir)
        .output()
        .expect("run");
    let elapsed = started.elapsed();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        run.status.success(),
        "the growing text failed under the cap: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        want.output,
        "the walker's count differs from the VM's"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "the growing text took {elapsed:?} — a rescan per append or per read"
    );
}

/// An `Int` is unbounded: wherever the program can make one outgrow i64 — a product or a power that feeds itself,
/// a doubling, a Fibonacci pair, a constant past the word, a recursive product — the walker holds it exact and
/// shows, divides, compares and fails on it exactly as the VM does.
const EXACT_INTEGER_CORPUS: &[(&str, &str)] = &[
    ("factorial_accumulator", "## Main\n    Let mutable acc be 1.\n    Let mutable i be 1.\n    While i is at most 25:\n        Set acc to acc * i.\n        Set i to i + 1.\n    Show acc.\n"),
    ("overflow_at_run_time", "## Main\n    Let mutable x be 0.\n    Let mutable i be 0.\n    While i is less than 30:\n        Set x to x + 1.\n        Set i to i + 1.\n    Let big be 4611686018427387904 + x - 30.\n    Show big * 4.\n"),
    ("recursive_factorial", "## To fact (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * fact(n - 1).\n\n## Main\n    Show fact(25).\n    Show fact(5).\n"),
    ("fibonacci_pair", "## Main\n    Let mutable a be 0.\n    Let mutable b be 1.\n    Let mutable i be 0.\n    While i is less than 100:\n        Let t be a + b.\n        Set a to b.\n        Set b to t.\n        Set i to i + 1.\n    Show a.\n    Show a / 7.\n    Show a % 1000.\n    Show a // 3.\n    Show (0 - a) // 7.\n    Show a is greater than 5.\n    Show a is equal to a.\n    Show \"big: {a}\".\n"),
    ("doubling_and_powers", "## Main\n    Let mutable x be 1.\n    Repeat for i from 1 to 70:\n        Set x to x + x.\n    Show x.\n    Let mutable k be 0.\n    While k is less than 3:\n        Set k to k + 1.\n    Show 3 ** 50.\n    Let e be 60 + k.\n    Show 2 ** e.\n    Show 2 ** 10.\n"),
    ("exact_parameter", "## To grow (x: Int) -> Int:\n    Repeat for i from 1 to 70:\n        Set x to x * 2.\n    Return x.\n\n## Main\n    Show grow(3).\n    Let mutable xs be a new Seq of Int.\n    Push grow(0) to xs.\n    Show xs.\n"),
    ("hash_with_and_without_a_modulus", "## Main\n    Let mutable h be 7.\n    Let mutable i be 0.\n    While i is less than 100:\n        Set h to (h * 31 + i) % 1000000007.\n        Set i to i + 1.\n    Show h.\n    Let mutable g be 7.\n    Set i to 0.\n    While i is less than 30:\n        Set g to g * 31 + i.\n        Set i to i + 1.\n    Show g.\n    Show g % 1024.\n    Show g / 1024.\n"),
    ("exact_meets_machine_slots", "## To half (x: Int) -> Int:\n    Return x / 2.\n\n## Main\n    Let mutable p be 1.\n    Repeat for i from 1 to 80:\n        Set p to p * 2.\n    Show half(p).\n    Show half(10).\n    Let mutable xs be a new Seq of Int.\n    Push p / (10 ** 21) to xs.\n    Show xs.\n    Show p is equal to 5.\n    Show p is less than 5.\n    Show p - p.\n    Show (p - p) is equal to 0.\n    Show p + 0.5.\n"),
    ("exact_division_by_zero", "## Main\n    Let mutable p be 1.\n    Repeat for i from 1 to 70:\n        Set p to p * 2.\n    Let z be 0.\n    Show p.\n    Show p / z.\n"),
    ("exact_remainder_by_zero", "## Main\n    Let mutable p be 1.\n    Repeat for i from 1 to 70:\n        Set p to p * 2.\n    Show p % 0.\n"),
    ("exact_base_negative_exponent", "## Main\n    Let mutable p be 1.\n    Repeat for i from 1 to 70:\n        Set p to p * 2.\n    Let e be 0 - 1.\n    Show p ** e.\n"),
    ("exact_bitwise_refused", "## Main\n    Let mutable p be 1.\n    Repeat for i from 1 to 70:\n        Set p to p * 2.\n    Let q be 0 - p.\n    Show q & 6.\n"),
    ("exact_bitwise_or_refused", "## Main\n    Let mutable p be 1.\n    Repeat for i from 1 to 70:\n        Set p to p * 2.\n    Show p | 1.\n"),
];

#[test]
fn the_walker_holds_every_integer_that_can_outgrow_a_word_exact() {
    assert_walker_observes_as_the_vm(EXACT_INTEGER_CORPUS);
}

/// Exactness is demanded, never assumed: an integer nothing can make outgrow i64 — a counter, a running sum, a
/// recursive sum (it cannot outgrow the calls it took to build it), a hash kept under a modulus, an index grown by
/// a counter — stays a machine integer, so the walker's arithmetic on it is the machine's.
const MACHINE_INTEGER_CORPUS: &[(&str, &str)] = &[
    ("counter_and_sum", "## Main\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is at most 100:\n        Set acc to acc + i.\n        Set i to i + 1.\n    Show acc.\n"),
    ("bounded_power", "## Main\n    Show 2 ** 10.\n    Let x be 7 * 6.\n    Show x ** 2.\n"),
];

/// These loops run on words even when the bytecode analysis conservatively
/// retains a checked exact fallback. The execution lock below makes entering
/// any exact helper a failure; the overflow corpus separately exercises fallback.
const GUARDED_MACHINE_INTEGER_CORPUS: &[(&str, &str)] = &[
    ("hash_under_a_modulus", "## Main\n    Let mutable h be 7.\n    Let mutable i be 0.\n    While i is less than 100:\n        Set h to (h * 31 + i) % 1000000007.\n        Set i to i + 1.\n    Show h.\n"),
    ("nested_products_of_counters", "## Main\n    Let mutable s be 0.\n    Let mutable i be 1.\n    While i is at most 20:\n        Let mutable j be 1.\n        While j is at most 20:\n            Set s to s + i * j.\n            Set j to j + 1.\n        Set i to i + 1.\n    Show s.\n"),
];

#[test]
fn an_integer_nothing_can_grow_stays_a_machine_integer() {
    let mut exact = Vec::new();
    for (label, src) in MACHINE_INTEGER_CORPUS {
        let c = compile_to_c_via_bytecode(src)
            .unwrap_or_else(|e| panic!("[{label}] the walker refused it: {e:?}"));
        let bodies = c.find("int main(").map(|at| {
            let first_fn = c
                .lines()
                .scan(0usize, |off, line| {
                    let here = *off;
                    *off += line.len() + 1;
                    Some((here, line))
                })
                .find(|(_, line)| line.starts_with("static int64_t fn_") && line.ends_with(") {"))
                .map(|(at, _)| at);
            first_fn.map_or(at, |f| f.min(at))
        });
        let bodies = &c[bodies.unwrap_or_else(|| panic!("[{label}] the emitted C has no main"))..];
        if bodies.contains("lc_x") {
            exact.push(format!(
                "[{label}] holds an integer exact that cannot outgrow a word:\n{bodies}"
            ));
        }
    }
    assert!(exact.is_empty(), "{}", exact.join("\n\n"));
    assert_walker_observes_as_the_vm(MACHINE_INTEGER_CORPUS);
}

/// Trap every exact-integer helper called by the program, after its runtime
/// definitions. A self-referential function macro calls the original function
/// after the trap, preserving its return type for C's type checker.
fn trap_exact_integer_calls(c: &str) -> String {
    let seam = logicaffeine_compile::codegen_c::USER_CODE_SENTINEL;
    let (runtime, program) = c.split_once(seam).expect("program/runtime seam");
    let names: std::collections::BTreeSet<_> = program
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .filter(|name| name.starts_with("lc_x"))
        .collect();
    let mut trapped = format!("{runtime}{seam}\n");
    for name in names {
        trapped.push_str(&format!(
            "#define {name}(...) (fputs(\"exact integer helper executed\\n\", stderr), exit(97), {name}(__VA_ARGS__))\n"
        ));
    }
    trapped.push_str(program);
    trapped
}

#[test]
fn bounded_loops_execute_without_exact_integer_helpers() {
    assert!(have_gcc(), "the bounded-word execution lock requires gcc");
    for (label, source) in GUARDED_MACHINE_INTEGER_CORPUS {
        let c = compile_to_c_via_bytecode(source).expect("bounded loop compiles");
        let expected = vm_observed(source);
        assert_eq!(expected.status, 0, "{label}: valid reference");
        assert_eq!(
            observed_c(&c, label).expect("ordinary C runs"),
            expected,
            "{label}: parity"
        );
        assert_eq!(
            observed_c(&trap_exact_integer_calls(&c), &format!("{label}_word_only"))
                .expect("instrumented C builds and runs"),
            expected,
            "{label}: bounded work must remain on machine words"
        );
    }
}

#[test]
fn the_word_execution_lock_detects_an_exact_helper_on_the_hot_path() {
    assert!(
        have_gcc(),
        "the word-only instrumentation control requires gcc"
    );
    let (_, source) = GUARDED_MACHINE_INTEGER_CORPUS[0];
    let c = compile_to_c_via_bytecode(source).expect("control compiles");
    assert!(
        c.contains("lc_xof("),
        "control contains the helper being trapped"
    );
    let main = "int main(int argc, char **argv) {";
    assert_eq!(c.matches(main).count(), 1);
    let forced = c.replacen(main, &format!("{main}\n    (void)lc_xof(1);"), 1);
    let observed = observed_c(&trap_exact_integer_calls(&forced), "forced_exact_helper")
        .expect("instrumentation control builds and runs");
    assert_eq!(
        observed.status, 97,
        "the injected helper must trip the lock"
    );
    assert_eq!(observed.stdout, "");
    assert_eq!(observed.stderr, "exact integer helper executed\n");
}

/// A native the program declares runs what the engine implements under its name — `get` reads the process
/// environment — on the walker as on the VM; `randomInt` and `randomFloat` draw from their ranges and refuse
/// crossed bounds in the VM's words.
const NATIVE_CORPUS: &[(&str, &str)] = &[
    ("native_without_a_body", "## To native get (key: Text) -> Option of Text\n\n## Main\nInspect get(\"HOME\"):\n    When Some (v):\n        Show \"some\".\n    When None:\n        Show \"none\".\n"),
    ("route_reads_an_unset_environment", "## Route POST /login -> Response:\n    Return withCookie(plain(\"ok\"), \"sid\", \"ada\").\n\n## Route GET /me (request: Request) -> Text:\n    Return cookieOf(request, \"sid\").\n\n## Main\nShow route(\"POST\", \"/login\").\nShow isCookieSafe(\"ada\").\nShow isCookieSafe(\"a;b\").\n"),
    ("random_within_bounds", "## Main\n    Let x be randomInt(3, 3).\n    Show x.\n    Let y be randomInt(1, 6).\n    Show y is at least 1 and y is at most 6.\n    Let f be randomFloat().\n    Show f is less than 1.0.\n    Show f is at least 0.0.\n"),
    ("random_crossed_bounds", "## Main\n    Show 1.\n    Let z be randomInt(5, 2).\n    Show z.\n"),
    ("random_bounds_crossed_by_one", "## Main\n    Show randomInt(3, 2).\n"),
    ("session_draws_its_secret", "## Session\n    a user, which is Text.\n\n## Route POST /login (name: Text) -> Response:\n    Set the session's user to name.\n    Return redirectTo(\"/\").\n\n## Route GET /me -> Text:\n    Return the session's user.\n"),
];

#[test]
fn the_walker_runs_natives_as_the_vm_does() {
    assert_walker_observes_as_the_vm(NATIVE_CORPUS);
}

/// An integer that can outgrow i64 — a product or power that feeds itself, a doubling, a pair that sums into itself,
/// a function's result that can — runs on machine words: its region's fast path adds, subtracts, multiplies and raises
/// with an overflow check, takes an exact result it is handed as its word when it is a small one, and only when a value
/// does not fit moves, with every live value, into the same region held exact (`X…` labels) and finishes there. So the
/// exactness costs a not-taken branch until it is needed. A value the region hands on as an exact integer — a result,
/// an argument, a global — is tagged where it leaves (`lc_xof`), and nothing else on the fast path is. Each word
/// boundary is crossed in a region of its own, so the check at that boundary is the first one to fire.
const SPECULATED_INTEGER_CORPUS: &[(&str, &str)] = &[
    ("recursive_sum", "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\n    Show fib(20).\n"),
    ("collatz", "## Main\n    Let mutable total be 0.\n    Let mutable i be 1.\n    While i is at most 3000:\n        Let mutable k be i.\n        While k is not 1:\n            If k % 2 equals 0:\n                Set k to k / 2.\n            Otherwise:\n                Set k to 3 * k + 1.\n            Set total to total + 1.\n        Set i to i + 1.\n    Show total.\n"),
    ("sift_down", "## To sift (xs: Seq of Int) (start: Int) (end: Int) -> Int:\n    Let mutable root be start.\n    Let mutable steps be 0.\n    While 2 * root + 1 is at most end:\n        Let child be 2 * root + 1.\n        Set steps to steps + item (child + 1) of xs.\n        Set root to child.\n    Return steps.\n\n## Main\n    Let xs be [5, 3, 9, 1, 7, 2, 8, 6, 4, 0, 11, 13, 12, 10, 14].\n    Show sift(xs, 0, 14).\n"),
    ("factorial_past_the_word", "## Main\n    Let mutable acc be 1.\n    Let mutable i be 1.\n    While i is at most 25:\n        Set acc to acc * i.\n        Set i to i + 1.\n    Show acc.\n    Show acc / 1000.\n"),
    ("doubling_past_the_word_in_a_function", "## To grow (x: Int) (n: Int) -> Int:\n    Repeat for i from 1 to n:\n        Set x to x + x.\n    Return x.\n\n## Main\n    Show grow(3, 10).\n    Show grow(3, 70).\n"),
    ("exact_result_arrives_in_a_loop", "## To powers (n: Int) -> Int:\n    Let mutable acc be 1.\n    Repeat for i from 1 to n:\n        Set acc to acc * 3.\n    Return acc.\n\n## Main\n    Let mutable total be 0.\n    Repeat for k from 1 to 5:\n        Set total to total + powers(k).\n    Show total.\n    Show powers(50) + 1.\n    Show total.\n"),
    ("word_boundaries", "## Main\n    Let mutable k be randomInt(3074457345618258602, 3074457345618258602).\n    Repeat for i from 1 to 2:\n        Set k to 3 * k + 1.\n        Show k.\n    Let mutable m be randomInt(-3074457345618258602, -3074457345618258602).\n    Repeat for i from 1 to 2:\n        Set m to m * 3 - 2.\n        Show m.\n    Let mutable n be randomInt(-4611686018427387903, -4611686018427387903).\n    Repeat for i from 1 to 2:\n        Set n to 1 - n * 2.\n        Show n.\n    Let mutable p be randomInt(3074457345618258603, 3074457345618258603).\n    Repeat for i from 1 to 1:\n        Set p to p * 3.\n        Show p.\n    Let mutable q be randomInt(-3074457345618258603, -3074457345618258603).\n    Repeat for i from 1 to 1:\n        Set q to 3 * q.\n        Show q.\n"),
    ("times_past_the_top", "## Main\n    Let mutable p be randomInt(3074457345618258603, 3074457345618258603).\n    Repeat for i from 1 to 1:\n        Set p to p * 3.\n    Show p.\n"),
    ("times_past_the_bottom", "## Main\n    Let mutable q be randomInt(-3074457345618258603, -3074457345618258603).\n    Repeat for i from 1 to 1:\n        Set q to 3 * q.\n    Show q.\n"),
    ("plus_past_the_top", "## Main\n    Let mutable k be randomInt(4611686018427387903, 4611686018427387903).\n    Repeat for i from 1 to 1:\n        Set k to 2 * k + 2.\n    Show k.\n"),
    ("minus_past_the_bottom", "## Main\n    Let mutable k be randomInt(-4611686018427387904, -4611686018427387904).\n    Repeat for i from 1 to 1:\n        Set k to 2 * k - 1.\n    Show k.\n"),
    ("constant_minus_past_the_bottom", "## Main\n    Let mutable x be randomInt(9223372036854775807, 9223372036854775807).\n    Repeat for i from 1 to 1:\n        Set x to (0 - 2) - x.\n        Set x to x * 2.\n    Show x.\n"),
    ("constant_minus_past_the_top", "## Main\n    Let mutable x be randomInt(-9223372036854775807, -9223372036854775807).\n    Repeat for i from 1 to 1:\n        Set x to 1 - x.\n        Set x to x * 2.\n    Show x.\n"),
    ("constant_increment_past_the_top", "## Main\n    Let mutable k be randomInt(9223372036854775806, 9223372036854775806).\n    Repeat for i from 1 to 2:\n        Set k to k + 1.\n        Show k.\n"),
    ("constant_sum_past_the_top", "## Main\n    Let mutable k be randomInt(9223372036854775801, 9223372036854775801).\n    Repeat for i from 1 to 3:\n        Set k to 3 + k.\n        Show k.\n"),
    ("negative_constant_sum_past_the_bottom", "## Main\n    Let mutable k be randomInt(-9223372036854775798, -9223372036854775798).\n    Repeat for i from 1 to 3:\n        Set k to k + (0 - 5).\n        Show k.\n"),
    ("constant_decrement_past_the_bottom", "## Main\n    Let mutable k be randomInt(-9223372036854775807, -9223372036854775807).\n    Repeat for i from 1 to 2:\n        Set k to k - 1.\n        Show k.\n"),
    ("negative_constant_difference_past_the_top", "## Main\n    Let mutable k be randomInt(9223372036854775793, 9223372036854775793).\n    Repeat for i from 1 to 3:\n        Set k to k - (0 - 7).\n        Show k.\n"),
    ("constant_minus_past_the_top_in_a_function", "## To flip (x: Int) -> Int:\n    Return 1 - x.\n\n## Main\n    Show flip(randomInt(-9223372036854775806, -9223372036854775806)).\n    Show flip(randomInt(-9223372036854775807, -9223372036854775807)).\n    Show flip(5).\n"),
    ("decrements_past_the_bottom_in_a_recursion", "## To tak (x: Int) (y: Int) (z: Int) -> Int:\n    If y is less than x:\n        Return tak(tak(x - 1, y, z), tak(y - 1, z, x), tak(z - 1, x, y)).\n    Return z.\n\n## Main\n    Show tak(18, 12, 6).\n    Let m be randomInt(-9223372036854775807, -9223372036854775807).\n    Show tak(m + 1, m, m - 1).\n    Show tak(m, m - 1, m).\n"),
    ("quotient_past_the_top", "## Main\n    Let mutable x be randomInt(-9223372036854775807, -9223372036854775807) - 1.\n    Let d be randomInt(-1, -1).\n    Repeat for i from 1 to 1:\n        Set x to x / d.\n        Set x to x * 2.\n    Show x.\n"),
    ("exact_argument", "## To digits (n: Int) -> Int:\n    Let mutable count be 0.\n    Let mutable m be n.\n    While m is greater than 0:\n        Set m to m / 10.\n        Set count to count + 1.\n    Return count.\n\n## Main\n    Let mutable p be 1.\n    Repeat for i from 1 to 70:\n        Set p to p * 2.\n    Show digits(p).\n    Show digits(randomInt(7, 7)).\n"),
    ("fibonacci_pair_in_a_for_each", "## Main\n    Let mutable a be 0.\n    Let mutable b be 1.\n    For each n in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99, 100]:\n        Let t be a + b.\n        Set a to b.\n        Set b to t.\n        If n % 25 equals 0:\n            Show a.\n    Show a.\n"),
];

/// The lines of the C function whose opening line starts with `head`, up to its closing brace.
fn c_function_body<'c>(c: &'c str, head: &str) -> Vec<&'c str> {
    let mut lines = c.lines().skip_while(|l| !l.starts_with(head));
    let mut body: Vec<&str> = Vec::new();
    lines
        .next()
        .unwrap_or_else(|| panic!("the emitted C has no function starting `{head}`"));
    for l in lines {
        if l == "}" {
            break;
        }
        body.push(l);
    }
    body
}

#[test]
fn a_growing_integer_runs_on_machine_words_until_it_outgrows_them() {
    for (label, src) in SPECULATED_INTEGER_CORPUS {
        let c = compile_to_c_via_bytecode(src)
            .unwrap_or_else(|e| panic!("[{label}] the walker refused it: {e:?}"));
        let heads: Vec<String> = c
            .lines()
            .filter(|l| {
                (l.starts_with("static int64_t fn_") || l.starts_with("int main("))
                    && l.ends_with(") {")
            })
            .map(|l| l.to_string())
            .collect();
        let mut speculated = 0;
        for head in &heads {
            let body = c_function_body(&c, head);
            let exact_from = body
                .iter()
                .position(|l| l.starts_with('X') && l.ends_with(":;"))
                .unwrap_or(body.len());
            if exact_from < body.len() {
                speculated += 1;
            }
            let leaves = |l: &str| {
                let l = l.trim_start();
                l.starts_with("return ")
                    || l.starts_with("g[")
                    || l.contains(" = fn_")
                    || l.contains("r[") && l.contains("fn_")
                    // A map cell has one representation across calls and
                    // word/exact regions, so storing a word tags that value.
                    || l.contains("map_") && l.contains("_set(")
            };
            let tagged: Vec<&&str> = body[..exact_from]
                .iter()
                .filter(|l| {
                    l.matches("lc_x").count() > l.matches("lc_xof(").count()
                        || (l.contains("lc_xof(") && !leaves(l))
                })
                .collect();
            assert!(
                tagged.is_empty(),
                "[{label}] `{head}` computes on tagged exact words on its fast path:\n{}",
                tagged
                    .iter()
                    .map(|l| l.trim())
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
        assert!(
            speculated > 0,
            "[{label}] no region holds its growing integer exact past an overflow:\n{c}"
        );
    }
    assert_walker_observes_as_the_vm(SPECULATED_INTEGER_CORPUS);
}

/// The program of the speculated-integer corpus named `label`.
fn speculated(label: &str) -> &'static str {
    SPECULATED_INTEGER_CORPUS
        .iter()
        .find(|(l, _)| *l == label)
        .map(|(_, src)| *src)
        .unwrap_or_else(|| panic!("the speculated-integer corpus has no `{label}`"))
}

/// The lines of a function body that run on machine words: those before the first label of a stub that leaves them
/// (`D<pc>:;`, `A<pc>:;`) or of the region held exact (`X<pc>:;`).
fn on_machine_words<'c>(body: &[&'c str]) -> Vec<&'c str> {
    body.iter()
        .take_while(|l| {
            let t = l.trim_start();
            !(t.len() > 3
                && matches!(t.as_bytes()[0], b'D' | b'A' | b'X')
                && t.ends_with(":;")
                && t[1..t.len() - 2].chars().all(|c| c.is_ascii_digit()))
        })
        .copied()
        .collect()
}

/// The integer literals a line of C writes (`12LL`, `-5LL`, `3074457345618258602ULL`), and the `INT64_MIN` /
/// `INT64_MAX` names as the values they stand for.
fn c_integer_literals(line: &str) -> Vec<i128> {
    let mut found = Vec::new();
    if line.contains("INT64_MIN") {
        found.push(i64::MIN as i128);
    }
    if line.contains("INT64_MAX") {
        found.push(i64::MAX as i128);
    }
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let starts = bytes[i].is_ascii_digit()
            && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'));
        if !starts {
            i += 1;
            continue;
        }
        let from = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        let suffixed = line[i..].starts_with("LL") || line[i..].starts_with("ULL");
        if suffixed {
            let negative = from > 0 && bytes[from - 1] == b'-';
            let v: i128 = line[from..i].parse().expect("a run of digits");
            found.push(if negative { -v } else { v });
        }
    }
    found
}

/// A sum or difference with a constant is checked by the machine's overflow flag on the operation itself
/// (`__builtin_add_overflow` / `__builtin_sub_overflow` with the constant as an immediate), never by a range test on
/// its other operand: that range's bound sits within the constant of an end of i64, past what an x86 compare takes as
/// an immediate, so gcc holds it in a register for the whole function — in a recursion like Takeuchi's, one of the
/// callee-saved registers its calls need, and a live value is spilled around every call to make room. A product by a
/// constant keeps the range test on its operand, so `3 * k + 1` still computes as one `lea`.
#[test]
fn a_constant_sum_or_difference_is_checked_by_the_overflow_flag() {
    let mut problems = Vec::new();
    let mut check = |label: &str, src: &str, head: &str| {
        let c = compile_to_c_via_bytecode(src)
            .unwrap_or_else(|e| panic!("[{label}] the walker refused it: {e:?}"));
        let head = c
            .lines()
            .find(|l| l.starts_with(head) && l.ends_with(") {"))
            .unwrap_or_else(|| panic!("[{label}] the emitted C defines no `{head}…{{`:\n{c}"));
        let body = c_function_body(&c, head);
        let words = on_machine_words(&body);
        let flagged = words
            .iter()
            .filter(|l| {
                (l.contains("__builtin_add_overflow(") || l.contains("__builtin_sub_overflow("))
                    && l.contains("goto D")
            })
            .count();
        if flagged == 0 {
            problems.push(format!(
                "[{label}] `{head}` checks no constant sum or difference by the overflow flag:\n{}",
                words.join("\n")
            ));
        }
        let wide: Vec<&&str> = words
            .iter()
            .filter(|l| {
                l.contains("goto D")
                    && c_integer_literals(l)
                        .iter()
                        .any(|v| *v < i32::MIN as i128 || *v > i32::MAX as i128)
            })
            .collect();
        if !wide.is_empty() {
            problems.push(format!(
                "[{label}] `{head}` guards a constant sum or difference by a bound past 32 bits:\n{}",
                wide.iter().map(|l| l.trim()).collect::<Vec<_>>().join("\n")
            ));
        }
    };
    for label in [
        "constant_increment_past_the_top",
        "constant_sum_past_the_top",
        "negative_constant_sum_past_the_bottom",
        "constant_decrement_past_the_bottom",
        "negative_constant_difference_past_the_top",
    ] {
        check(label, speculated(label), "int main(");
    }
    for label in [
        "constant_minus_past_the_top_in_a_function",
        "decrements_past_the_bottom_in_a_recursion",
    ] {
        check(label, speculated(label), "static lc_wret fn_");
    }
    assert!(problems.is_empty(), "{}", problems.join("\n\n"));

    for label in ["times_past_the_top", "times_past_the_bottom", "collatz"] {
        let c = compile_to_c_via_bytecode(speculated(label))
            .unwrap_or_else(|e| panic!("[{label}] the walker refused it: {e:?}"));
        let body = c_function_body(&c, "int main(");
        let words = on_machine_words(&body);
        assert!(
            !words.iter().any(|l| l.contains("__builtin_mul_overflow(")),
            "[{label}] a product by a constant is checked by the overflow flag, not by its operand's range:\n{}",
            words.join("\n")
        );
        assert!(
            words.iter().any(|l| l.trim_start().starts_with("if (") && l.contains("goto D")),
            "[{label}] a product by a constant is not guarded on machine words:\n{}",
            words.join("\n")
        );
    }
}

/// The bytecode C marks where the program's own code begins with the seam the AST generator writes, once: a reader of
/// either C output (the benchmarks page, the device tiers) finds the program part the same way.
#[test]
fn the_bytecode_c_marks_where_the_program_begins() {
    let c = logicaffeine_compile::compile::compile_to_c_via_bytecode(
        "## To double (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nShow double(21).\n",
    )
    .expect("compiles to C through its bytecode");
    let seam = logicaffeine_compile::codegen_c::USER_CODE_SENTINEL;
    assert_eq!(c.matches(seam).count(), 1, "the seam appears exactly once");
    let (runtime, program) = c.split_once(seam).expect("the seam");
    assert!(
        program.contains("int main("),
        "main is in the program's part:\n{program}"
    );
    assert!(
        !runtime.contains("int main("),
        "nothing of the program precedes the seam"
    );
}

#[test]
fn memoized_recursive_integers_remain_exact_across_the_word_boundary() {
    assert!(have_gcc(), "the exact recursion lock requires gcc");
    for (n, expected) in [
        (20, "6765"),
        (92, "7540113804746346429"),
        (94, "19740274219868223167"),
    ] {
        let source = format!(
            "## No Inline\n## To fib (n: Int) -> Int:\n    If n < 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib({n}).\n"
        );
        let want = Observed {
            stdout: format!("{expected}\n"),
            stderr: String::new(),
            status: 0,
        };
        // The plain VM does not memoize this recursion; use a linear independent
        // reference for the large cases while retaining the recursive C program.
        let reference = if n == 20 {
            source.clone()
        } else {
            format!("## Main\nLet a be 0.\nLet b be 1.\nLet i be 0.\nWhile i < {n}:\n    Let next be a + b.\n    Set a to b.\n    Set b to next.\n    Set i to i + 1.\nShow a.\n")
        };
        assert_eq!(vm_observed(&reference), want, "reference Fibonacci({n})");
        let got = walker_observed(&source, &format!("memo_fib_{n}"))
            .expect("memoized recursion compiles and runs as C");
        assert_eq!(
            got, want,
            "Fibonacci({n}) must preserve every integer bit through its memo cache"
        );
    }
}

#[test]
fn exact_map_values_survive_overwrites_aliases_and_iteration() {
    assert!(have_gcc(), "the exact map lock requires gcc");
    for (label, declaration, key) in [
        ("integer_keys", "Int", "7"),
        ("text_keys", "Text", "\"answer\""),
    ] {
        let source = format!(
            "## No Inline\n## To grow (n: Int) -> Int:\n    Let value be 1.\n    Repeat for i from 1 to n:\n        Set value to value * 2.\n    Return value.\n\n## Main\nLet m be a new Map of {declaration} to Int.\nSet item {key} of m to 0.\nShow item {key} of m.\nSet item {key} of m to grow(80) + 7.\nLet saved be m.\nSet item {key} of m to 3.\nShow item {key} of saved.\nShow item {key} of m.\nShow (item {key} of saved) + 1.\nShow saved.\nFor each entry in saved:\n    Show entry.\n"
        );
        let wide = "1208925819614629174706183";
        let shown_key = if declaration == "Text" { "answer" } else { "7" };
        let want = Observed {
            stdout: format!("0\n{wide}\n3\n1208925819614629174706184\n{{{shown_key}: {wide}}}\n({shown_key}, {wide})\n"),
            stderr: String::new(),
            status: 0,
        };
        assert_eq!(
            vm_observed(&source),
            want,
            "{label}: independent map output"
        );
        assert_eq!(
            walker_observed(&source, label).expect("C accepts the ordinary map"),
            want,
            "{label}"
        );
    }
}

#[test]
fn exact_map_values_keep_signed_word_extremes_and_boundary_arithmetic() {
    assert!(have_gcc(), "the exact map lock requires gcc");
    let source = "## Main\nLet m be a new Map of Int to Int.\nSet item 1 of m to randomInt(9223372036854775807, 9223372036854775807).\nSet item 2 of m to randomInt(-9223372036854775807, -9223372036854775807) - 1.\nShow item 1 of m.\nShow item 2 of m.\nShow (item 1 of m) + 1.\nShow (item 2 of m) - 1.\n";
    let want = Observed {
        stdout:
            "9223372036854775807\n-9223372036854775808\n9223372036854775808\n-9223372036854775809\n"
                .to_string(),
        stderr: String::new(),
        status: 0,
    };
    assert_eq!(vm_observed(source), want, "independent boundary output");
    assert_eq!(
        walker_observed(source, "map_word_edges").expect("C accepts integer map edges"),
        want
    );
}

#[test]
fn exact_map_values_cross_typed_function_boundaries() {
    assert!(have_gcc(), "the exact map lock requires gcc");
    let source = "## No Inline\n## To make (n: Int) -> Map of Int to Int:\n    Let value be 1.\n    Repeat for i from 1 to n:\n        Set value to value * 2.\n    Let m be a new Map of Int to Int.\n    Set item 7 of m to value.\n    Return m.\n\n## No Inline\n## To read (m: Map of Int to Int) -> Int:\n    Return item 7 of m.\n\n## Main\nLet small be make(3).\nLet big be make(80).\nShow read(small).\nShow read(big).\nShow read(small) + read(big).\n";
    let want = Observed {
        stdout: "8\n1208925819614629174706176\n1208925819614629174706184\n".to_string(),
        stderr: String::new(),
        status: 0,
    };
    assert_eq!(
        vm_observed(source),
        want,
        "independent function-boundary output"
    );
    assert_eq!(
        walker_observed(source, "map_functions").expect("C accepts typed map functions"),
        want
    );
}

/// Release input/result wrappers so the heap bound isolates floor-division temporaries.
#[cfg(target_os = "linux")]
#[test]
fn exact_floor_division_reclaims_internal_temporaries() {
    assert!(have_gcc(), "the exact-floor heap lock requires gcc");
    let emitted =
        compile_to_c_via_bytecode("## Main\nShow 0.\n").expect("C runtime fixture compiles");
    let (runtime, _) = emitted.split_once("int main(").expect("generated entry");
    for large_divisor in [false, true] {
        let mut samples = Vec::new();
        let dividend = -5_000_000_000_000_000_000i128 * 5_000_000_000_000_000_000i128;
        let divisor = if large_divisor {
            4_000_000_000_000_000_000i128 * 4_000_000_000_000_000_000i128
        } else {
            3
        };
        for turns in [1280, 12800] {
            let main = include_str!("support/c_exact_floor_heap.c")
                .replace("@TURNS@", &turns.to_string())
                .replace("@LARGE_DIVISOR@", if large_divisor { "1" } else { "0" });
            let observed = observed_c(&format!("{runtime}{main}"), "exact_floor_heap")
                .expect("exact-floor allocator fixture builds and runs");
            assert_eq!(observed.status, 0);
            assert_eq!(observed.stderr, "");
            let lines: Vec<_> = observed.stdout.lines().collect();
            assert_eq!(lines.len(), 2);
            assert_eq!(lines[0], dividend.div_euclid(divisor).to_string());
            let retained: usize = lines[1]
                .strip_prefix("retained=")
                .expect("retained heap report")
                .parse()
                .expect("byte count");
            samples.push(retained);
        }
        assert!(
            samples[1] <= samples[0] + 4096,
            "exact floor division retains internal temporaries: large_divisor={large_divisor}, live={samples:?}"
        );
    }
}

/// The modulo result is a machine word; any owned exact remainder must be
/// released inside the helper after conversion. Input storage is freed here.
#[cfg(target_os = "linux")]
#[test]
fn exact_power_of_two_modulo_reclaims_its_temporary() {
    assert!(have_gcc(), "the exact-modulo heap lock requires gcc");
    let emitted =
        compile_to_c_via_bytecode("## Main\nShow 0.\n").expect("C runtime fixture compiles");
    let (runtime, _) = emitted.split_once("int main(").expect("generated entry");
    let helpers = include_str!("../../logicaffeine_compile/src/codegen_c/checked_int.c");
    let mut samples = Vec::new();
    for turns in [1280, 12800] {
        let main = include_str!("support/c_pow2_heap.c").replace("@TURNS@", &turns.to_string());
        let observed = observed_c(&format!("{runtime}{helpers}{main}"), "exact_pow2_heap")
            .expect("exact-modulo allocator fixture builds and runs");
        assert_eq!(observed.status, 0);
        assert_eq!(observed.stderr, "");
        let lines: Vec<_> = observed.stdout.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            ((u128::from(u64::MAX)) % (1u128 << 62)).to_string()
        );
        samples.push(
            lines[1]
                .strip_prefix("retained=")
                .expect("retained heap report")
                .parse::<usize>()
                .expect("byte count"),
        );
    }
    assert!(
        samples[1] <= samples[0] + 4096,
        "exact modulo retains its owned remainder: live={samples:?}"
    );
}

// Observe physical heap peaks while the generated program is running, and
// retained storage after a collection but before process-exit teardown.
#[cfg(target_os = "linux")]
#[test]
fn overwritten_exact_map_values_keep_live_heap_bounded() {
    assert!(have_gcc(), "the exact-map reclamation lock requires gcc");
    for base in [42i64, 5_000_000_000_000_000_000i64] {
        let mut samples = Vec::new();
        for turns in [1_048_576, 10_485_760] {
            let source = format!(
                "## Main\nLet m be a new Map of Int to Int.\nLet i be 0.\nWhile i < {turns}:\n    Set item 1 of m to randomInt({base}, {base}) + (i % 3).\n    Set i to i + 1.\nShow length of m.\nShow item 1 of m.\n"
            );
            let c = compile_to_c_via_bytecode(&source).expect("fixed map workload compiles");
            let allocation = "return lc_gc_heap_alloc(n, atomic);";
            assert_eq!(
                c.matches(allocation).count(),
                1,
                "sample the allocator's single entry"
            );
            assert_eq!(c.matches("int main(").count(), 1, "one program entry");
            let c = c.replacen(allocation,
                "void *allocated = lc_gc_heap_alloc(n, atomic); map_heap_sample(); return allocated;", 1)
                .replacen("int main(", "static int map_program_main(", 1);
            let instrumented = format!(
                "{}\n{c}\n{}",
                include_str!("support/c_map_heap_prefix.c"),
                include_str!("support/c_map_heap_main.c"),
            );
            let out = observed_c(&instrumented, &format!("map_heap_{base}_{turns}"))
                .expect("allocator-instrumented C builds and runs");
            assert_eq!(out.status, 0, "generated workload: {out:?}");
            assert_eq!(out.stdout, format!("1\n{}\n", base + (turns - 1) % 3));
            let values: Vec<usize> = out
                .stderr
                .trim()
                .split_whitespace()
                .map(|v| v.parse().expect("numeric allocator report"))
                .collect();
            assert_eq!(
                values.len(),
                4,
                "retained / physical peak / collections during work / managed peak"
            );
            let (retained, peak, collections, managed_peak) =
                (values[0], values[1], values[2], values[3]);
            if base > (1i64 << 62) {
                assert!(
                    collections >= 2,
                    "the exact-value workload must span several collections: {values:?}"
                );
                assert!(managed_peak <= 8 * 1024 * 1024 + 64 * 1024,
                    "one map entry must stay within the collection threshold and fixed live-set slack: {values:?}");
            } else {
                assert_eq!(
                    collections, 0,
                    "small tagged integers must not allocate per update"
                );
            }
            samples.push((retained, peak));
        }
        assert!(samples[1].0 <= samples[0].0 + 16 * 1024,
            "a fixed one-entry map retains storage with the work before teardown: base={base}, samples={samples:?}");
        assert!(samples[1].1 <= samples[0].1 + 256 * 1024,
            "physical heap peak grows over tenfold work despite a fixed map: base={base}, samples={samples:?}");
    }
}
