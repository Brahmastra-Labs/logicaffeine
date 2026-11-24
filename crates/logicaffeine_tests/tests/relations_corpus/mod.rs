//! Relations as values: programs whose state is a `Set` of tuples, shared by every lock that holds a
//! tier to the tree-walker on them (`param_return_tier_coverage_lock` for the VM, AOT-Rust, C and WASM
//! lowering; `wasm_aot_lock` for WASM execution), so the two cannot drift apart.
//!
//! - `points_to`: Andersen's analysis as a fixpoint of joins over `vP ⊆ Var × Heap` and
//!   `hP ⊆ Heap × Field × Heap` (Whaley & Lam's four rules), returned as a tuple of sets.
//! - `triples_union_and_grow`: three-place members, union, and a walk over a copy while the set grows.
//! - `signed_zero_is_one_key`: members compare as `==` does, so `(-0.0, 1)` is the key `(0.0, 1)`.
//! - `tuple_of_relation_and_set`: a relation and a set returned together and destructured.
//! - `seq_of_pairs_destructured`: a `Seq` of tuples walked with a destructuring `for`, summed in a
//!   callee, and collected into a set (duplicates collapse).

pub const PROGRAMS: &[(&str, &str)] = &[
    ("points_to", include_str!("../fixtures/relations/points_to.lg")),
    (
        "triples_union_and_grow",
        include_str!("../fixtures/relations/triples_union_and_grow.lg"),
    ),
    (
        "signed_zero_is_one_key",
        include_str!("../fixtures/relations/signed_zero_is_one_key.lg"),
    ),
    (
        "tuple_of_relation_and_set",
        include_str!("../fixtures/relations/tuple_of_relation_and_set.lg"),
    ),
    (
        "seq_of_pairs_destructured",
        include_str!("../fixtures/relations/seq_of_pairs_destructured.lg"),
    ),
];
