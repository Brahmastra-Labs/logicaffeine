//! Whole-struct `Show` — the ONE canonical format, locked byte-identical across every executing tier.
//!
//! `Show <struct>` renders `Name { field: value, … }`: brace-delimited, fields SORTED by name (a
//! struct's field storage is unordered, so a stable display order is what keeps the tiers in lockstep),
//! nested structs recursive, and a `Seq of <Struct>` as `[a, b]`. This is the interpreter oracle's format
//! (`RuntimeValue::to_display_string`); the VM shares that path, and both the AOT-Rust `Showable` impl and
//! the C `<Name>_to_string` helper are generated to match it exactly.
//!
//! History: this was a pre-existing 3-way divergence — the C backend rendered `Name(f: v)` (parens,
//! declaration order) and AOT-Rust emitted no `Showable` impl for user structs at all (whole-struct `Show`
//! didn't compile). Both were canonicalized to the brace oracle. WASM whole-struct `Show` is a separate,
//! documented deferral (`vm/wasm/kind.rs`) — unsupported, not divergent — so it is out of scope here.
//!
//! Each shape is pinned on ALL FOUR executing tiers: tree-walker ≡ VM (`assert_all_tiers`), AOT-Rust ≡
//! interpreter (`assert_compiled_equals_interpreted`), and C == the oracle string (`assert_c_output`). A
//! regression in ANY tier's struct-show format goes RED here — this is the pit-of-success lock for the
//! whole surface, so it can only ever be widened.

#![cfg(not(target_arch = "wasm32"))]
#![allow(unused_imports)]
mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Pin one whole-struct-`Show` shape on every executing tier: tw ≡ VM, AOT ≡ interp, C == oracle.
macro_rules! lock_struct_show {
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

// Flat struct, scalar fields already in name order (x < y).
lock_struct_show!(
    flat_scalars,
    "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet p be a new Point with x 10 and y 20.\nShow p.\n",
    "Point { x: 10, y: 20 }"
);

// Fields render SORTED by name, NOT declaration order: declared (name, age) → shown (age, name).
lock_struct_show!(
    fields_sorted_not_declared,
    "## A Person has:\n    A name: Text.\n    An age: Int.\n\n## Main\nLet p be a new Person with name \"Bob\" and age 25.\nShow p.\n",
    "Person { age: 25, name: Bob }"
);

// A value-type field renders through its own display (`50.00 USD`), not a garbage int.
lock_struct_show!(
    value_type_field,
    "## A Wallet has:\n    A balance: Money.\n\n## Main\nLet w be a new Wallet with balance money(50, \"USD\").\nShow w.\n",
    "Wallet { balance: 50.00 USD }"
);

// Bool + Text fields, sorted (name < on).
lock_struct_show!(
    bool_and_text_fields,
    "## A Flag has:\n    An on: Bool.\n    A name: Text.\n\n## Main\nLet fl be a new Flag with on true and name \"x\".\nShow fl.\n",
    "Flag { name: x, on: true }"
);

// A nested struct field is shown recursively, not as a garbage int.
lock_struct_show!(
    nested_struct,
    "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## A Line has:\n    A p: Point.\n\n## Main\nLet a be a new Point with x 1 and y 2.\nLet l be a new Line with p a.\nShow l.\n",
    "Line { p: Point { x: 1, y: 2 } }"
);

// Three levels deep — recursion all the way down.
lock_struct_show!(
    deeply_nested,
    "## A A has:\n    A n: Int.\n\n## A B has:\n    A a: A.\n\n## A C has:\n    A b: B.\n\n## Main\nLet x be a new A with n 7.\nLet y be a new B with a x.\nLet z be a new C with b y.\nShow z.\n",
    "C { b: B { a: A { n: 7 } } }"
);

// A Seq of structs, shown from a literal — `[elem, elem]` with each struct in canonical form.
lock_struct_show!(
    seq_of_struct_literal,
    "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet a be a new Point with x 1 and y 2.\nLet b be a new Point with x 3 and y 4.\nShow [a, b].\n",
    "[Point { x: 1, y: 2 }, Point { x: 3, y: 4 }]"
);

// A Seq of structs bound to a variable first, then shown.
lock_struct_show!(
    seq_of_struct_var,
    "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet a be a new Point with x 1 and y 2.\nLet b be a new Point with x 3 and y 4.\nLet ps be [a, b].\nShow ps.\n",
    "[Point { x: 1, y: 2 }, Point { x: 3, y: 4 }]"
);

// A Seq of structs whose field is itself a value type — nested renderers all the way through.
lock_struct_show!(
    seq_of_value_field_struct,
    "## A Wallet has:\n    A balance: Money.\n\n## Main\nLet a be a new Wallet with balance money(5, \"USD\").\nLet b be a new Wallet with balance money(9, \"USD\").\nShow [a, b].\n",
    "[Wallet { balance: 5.00 USD }, Wallet { balance: 9.00 USD }]"
);

// ── Struct FIELDS that are collections ─────────────────────────────────────────────────────────────
// A `Seq` field renders `[a, b, …]` inline. On C this goes through the generated `seq_<E>_to_string`
// (the print-free companion of `show_seq_<E>`). (A `Map`/`Set` field is NOT yet locked: the C hash-map
// isn't insertion-ordered, so it can't match the interpreter's insertion-ordered display — that needs
// insertion-ordered C maps, tracked separately. AOT already handles Map/Set fields.)

lock_struct_show!(
    field_seq_of_int,
    "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet b be a new Bag with items [1, 2, 3].\nShow b.\n",
    "Bag { items: [1, 2, 3] }"
);

lock_struct_show!(
    field_seq_of_text,
    "## A Box has:\n    A labels: Seq of Text.\n\n## Main\nLet x be a new Box with labels [\"a\", \"b\"].\nShow x.\n",
    "Box { labels: [a, b] }"
);

lock_struct_show!(
    field_seq_of_real,
    "## A Data has:\n    A xs: Seq of Real.\n\n## Main\nLet d be a new Data with xs [1.5, 2.5].\nShow d.\n",
    "Data { xs: [1.5, 2.5] }"
);

// A scalar field next to a Seq field — sorted (id < tags), Seq rendered inline.
lock_struct_show!(
    field_scalar_and_seq,
    "## A Rec has:\n    A id: Int.\n    A tags: Seq of Text.\n\n## Main\nLet r be a new Rec with id 7 and tags [\"p\", \"q\"].\nShow r.\n",
    "Rec { id: 7, tags: [p, q] }"
);

// A Seq field of a value type — each element via the value type's own renderer.
lock_struct_show!(
    field_seq_of_money,
    "## A Cart has:\n    A prices: Seq of Money.\n\n## Main\nLet c be a new Cart with prices [money(3, \"USD\"), money(5, \"USD\")].\nShow c.\n",
    "Cart { prices: [3.00 USD, 5.00 USD] }"
);

// A Seq field of a nested struct — recursion through both the Seq and the element struct.
lock_struct_show!(
    field_seq_of_struct,
    "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## A Grid has:\n    A pts: Seq of Point.\n\n## Main\nLet a be a new Point with x 1 and y 2.\nLet g be a new Grid with pts [a].\nShow g.\n",
    "Grid { pts: [Point { x: 1, y: 2 }] }"
);

// A `Seq` chain across three structs whose NAME order runs both ways against the call direction:
// `Forest_to_string` calls `seq_Tree_to_string` (Forest < Tree), `Tree_to_string` calls
// `seq_Leaf_to_string` (Leaf < Tree). A container field puts no declaration order between two
// structs, so C resolves both calls through the Phase D prototypes, whichever struct sorts first.
lock_struct_show!(
    field_seq_chain_both_orders,
    "## A Leaf has:\n    A v: Int.\n\n## A Tree has:\n    A leaves: Seq of Leaf.\n\n## A Forest has:\n    A trees: Seq of Tree.\n\n## Main\nLet a be a new Leaf with v 1.\nLet b be a new Leaf with v 2.\nLet t be a new Tree with leaves [a, b].\nLet f be a new Forest with trees [t].\nShow f.\n",
    "Forest { trees: [Tree { leaves: [Leaf { v: 1 }, Leaf { v: 2 }] }] }"
);

// The element struct is declared AFTER the struct holding its `Seq`, and the holder is shown both
// alone and as a `Seq` element (`show_seq_Zone` → `Zone_to_string` → `seq_Atom_to_string`).
lock_struct_show!(
    field_seq_of_later_declared_struct,
    "## A Zone has:\n    A atoms: Seq of Atom.\n    A name: Text.\n\n## An Atom has:\n    A v: Int.\n\n## Main\nLet a be a new Atom with v 3.\nLet z be a new Zone with atoms [a, a] and name \"z\".\nShow z.\nShow [z].\n",
    "Zone { atoms: [Atom { v: 3 }, Atom { v: 3 }], name: z }\n[Zone { atoms: [Atom { v: 3 }, Atom { v: 3 }], name: z }]"
);
