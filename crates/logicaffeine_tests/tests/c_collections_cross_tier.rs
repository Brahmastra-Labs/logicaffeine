//! Maps, Sets, and collection interpolation — locked byte-identical across every executing tier.
//!
//! The C backend now carries insertion-ORDERED maps and sets (an `order` array threaded through
//! `new`/`set`/`add`/resize, so iteration/`Show` matches the interpreter oracle's insertion order even
//! after a rehash) plus `map_*_to_string` / `set_*_to_string` renderers reused by top-level `Show`,
//! struct fields, and string interpolation. Before this, `Show`ing a Map/Set (or interpolating one, or
//! a struct holding one) was a C compile error or printed a garbage integer.
//!
//! Map display is `{k: v, …}`, Set display is `{a, …}` — both in insertion order, keys/values unquoted,
//! exactly like the interpreter. Each shape is pinned on all four executing tiers: tree-walker ≡ VM
//! (`assert_all_tiers`), AOT-Rust ≡ interpreter (`assert_compiled_equals_interpreted`), and C == the
//! oracle string (`assert_c_output`). WASM whole-collection `Show` is a separate documented deferral.

#![cfg(not(target_arch = "wasm32"))]
#![allow(unused_imports)]
mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Pin one collection-`Show` / interpolation shape on every executing tier.
macro_rules! lock_all_tiers {
    ($name:ident, $src:expr, $expected:expr) => {
        mod $name {
            use super::*;
            #[test]
            fn tw_vm() {
                assert_all_tiers($src, $expected);
            }
            #[test]
            fn aot() {
                assert_compiled_equals_interpreted($src);
            }
            #[test]
            fn c() {
                assert_c_output($src, $expected);
            }
        }
    };
}

// ── Maps (top-level Show, insertion order) ──────────────────────────────────────────────────────────
lock_all_tiers!(
    map_str_int,
    "## Main\nLet mutable m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\nSet item \"b\" of m to 2.\nSet item \"c\" of m to 3.\nShow m.\n",
    "{a: 1, b: 2, c: 3}"
);
lock_all_tiers!(
    map_str_str,
    "## Main\nLet mutable m be a new Map of Text to Text.\nSet item \"k\" of m to \"v\".\nSet item \"j\" of m to \"w\".\nShow m.\n",
    "{k: v, j: w}"
);
lock_all_tiers!(
    map_int_int,
    "## Main\nLet mutable m be a new Map of Int to Int.\nSet item 1 of m to 10.\nSet item 2 of m to 20.\nShow m.\n",
    "{1: 10, 2: 20}"
);
lock_all_tiers!(
    map_str_money,
    "## Main\nLet mutable m be a new Map of Text to Money.\nSet item \"x\" of m to money(3, \"USD\").\nSet item \"y\" of m to money(9, \"USD\").\nShow m.\n",
    "{x: 3.00 USD, y: 9.00 USD}"
);

// Insertion order MUST survive a rehash (14 entries forces a resize past the 0.75 load factor of cap 16).
lock_all_tiers!(
    map_insertion_order_across_resize,
    "## Main\nLet mutable m be a new Map of Int to Int.\nRepeat for i from 1 to 14:\n    Set item i of m to i.\nShow m.\n",
    "{1: 1, 2: 2, 3: 3, 4: 4, 5: 5, 6: 6, 7: 7, 8: 8, 9: 9, 10: 10, 11: 11, 12: 12, 13: 13, 14: 14}"
);

// ── Sets (top-level Show, insertion order) ──────────────────────────────────────────────────────────
lock_all_tiers!(
    set_int,
    "## Main\nLet mutable s be a new Set of Int.\nAdd 3 to s.\nAdd 1 to s.\nAdd 2 to s.\nShow s.\n",
    "{3, 1, 2}"
);
lock_all_tiers!(
    set_str,
    "## Main\nLet mutable s be a new Set of Text.\nAdd \"b\" to s.\nAdd \"a\" to s.\nShow s.\n",
    "{b, a}"
);

// ── Struct fields that are Maps / Sets ──────────────────────────────────────────────────────────────
lock_all_tiers!(
    struct_map_field,
    "## A Reg has:\n    A lookup: Map of Text to Int.\n\n## Main\nLet mutable mm be a new Map of Text to Int.\nSet item \"a\" of mm to 1.\nSet item \"b\" of mm to 2.\nLet r be a new Reg with lookup mm.\nShow r.\n",
    "Reg { lookup: {a: 1, b: 2} }"
);
lock_all_tiers!(
    struct_set_field,
    "## A Grp has:\n    A ids: Set of Int.\n\n## Main\nLet mutable ss be a new Set of Int.\nAdd 3 to ss.\nAdd 1 to ss.\nLet g be a new Grp with ids ss.\nShow g.\n",
    "Grp { ids: {3, 1} }"
);

// ── Collection interpolation (`"{c}"` matches `Show c`) ─────────────────────────────────────────────
lock_all_tiers!(
    interp_seq,
    "## Main\nLet xs be [1, 2, 3].\nShow \"xs={xs}\".\n",
    "xs=[1, 2, 3]"
);
lock_all_tiers!(
    interp_map,
    "## Main\nLet mutable m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\nShow \"m={m}\".\n",
    "m={a: 1}"
);
lock_all_tiers!(
    interp_set,
    "## Main\nLet mutable s be a new Set of Text.\nAdd \"a\" to s.\nAdd \"b\" to s.\nShow \"s={s}\".\n",
    "s={a, b}"
);
lock_all_tiers!(
    interp_struct,
    "## A P has:\n    An x: Int.\n\n## Main\nLet p be a new P with x 5.\nShow \"p={p}\".\n",
    "p=P { x: 5 }"
);

// ── Nested sequences (Seq of Seq) ───────────────────────────────────────────────────────────────────
// The C `Seq_Seq_…` family (typedef + `to_string`/`show`) is now synthesized for nested-seq types
// reached from STATEMENT expressions, not just function signatures.
lock_all_tiers!(
    nested_seq_int,
    "## Main\nShow [[1, 2], [3, 4]].\n",
    "[[1, 2], [3, 4]]"
);
lock_all_tiers!(
    nested_seq_str,
    "## Main\nShow [[\"a\", \"b\"], [\"c\"]].\n",
    "[[a, b], [c]]"
);
lock_all_tiers!(
    nested_seq_var,
    "## Main\nLet g be [[1, 2], [3, 4]].\nShow g.\n",
    "[[1, 2], [3, 4]]"
);
lock_all_tiers!(
    nested_seq_deep,
    "## Main\nShow [[[1], [2]], [[3]]].\n",
    "[[[1], [2]], [[3]]]"
);
lock_all_tiers!(
    nested_seq_interp,
    "## Main\nLet g be [[1, 2]].\nShow \"g={g}\".\n",
    "g=[[1, 2]]"
);

// ── Seq-valued maps (Map of Text to Seq of …) ───────────────────────────────────────────────────────
// A Map whose value is a Seq → the generic `Map_str_Seq_…` C family (insertion-ordered, value rendered
// via the seq's `to_string`). Works for scalar and value-type seq values, top-level / field / interp.
lock_all_tiers!(
    map_of_seq_int,
    "## Main\nLet mutable m be a new Map of Text to Seq of Int.\nSet item \"a\" of m to [1, 2].\nSet item \"b\" of m to [3].\nShow m.\n",
    "{a: [1, 2], b: [3]}"
);
lock_all_tiers!(
    map_of_seq_money,
    "## Main\nLet mutable m be a new Map of Text to Seq of Money.\nSet item \"a\" of m to [money(1, \"USD\"), money(2, \"USD\")].\nShow m.\n",
    "{a: [1.00 USD, 2.00 USD]}"
);
lock_all_tiers!(
    map_of_seq_field,
    "## A Reg has:\n    A groups: Map of Text to Seq of Int.\n\n## Main\nLet mutable mm be a new Map of Text to Seq of Int.\nSet item \"a\" of mm to [1, 2].\nLet r be a new Reg with groups mm.\nShow r.\n",
    "Reg { groups: {a: [1, 2]} }"
);
lock_all_tiers!(
    map_of_seq_interp,
    "## Main\nLet mutable m be a new Map of Text to Seq of Int.\nSet item \"a\" of m to [1, 2].\nShow \"m={m}\".\n",
    "m={a: [1, 2]}"
);

// ── Tuples (PARENS, distinct from Seq brackets) ─────────────────────────────────────────────────────
// AOT tuples are a `LogosTuple` newtype (Derefs to `Vec<Value>` so index/destructure/return are
// unchanged) that Shows with parens; on C a tuple is an `LTuple_…` struct with a parens `to_string`.
// Both distinguish a tuple from a Seq even when nested (`[(1, 2), (3, 4)]`).
lock_all_tiers!(tuple_show, "## Main\nShow (1, 2).\n", "(1, 2)");
lock_all_tiers!(
    tuple_show_var,
    "## Main\nLet t be (10, 20).\nShow t.\n",
    "(10, 20)"
);
lock_all_tiers!(
    tuple_mixed,
    "## Main\nShow (1, \"a\", true).\n",
    "(1, a, true)"
);
lock_all_tiers!(
    seq_of_tuple,
    "## Main\nShow [(1, 2), (3, 4)].\n",
    "[(1, 2), (3, 4)]"
);
lock_all_tiers!(
    tuple_interp,
    "## Main\nLet t be (1, 2).\nShow \"t={t}\".\n",
    "t=(1, 2)"
);
