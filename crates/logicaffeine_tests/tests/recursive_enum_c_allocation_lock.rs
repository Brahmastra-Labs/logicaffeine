use logicaffeine_compile::compile::{
    compile_to_c, compile_to_c_traced, compile_to_c_via_bytecode_traced,
};
use logicaffeine_compile::optimization::{decorate_source, Opt};

const TREE: &str = r#"
## A Tree is either:
    A Leaf.
    A Node with left Tree and right Tree.

## To makeNode (left: Tree, right: Tree) -> Tree:
    Return a new Node with left left and right right.

## Main
Let tree be makeNode(a new Leaf, a new Leaf).
Inspect tree:
    When Leaf: Show 0.
    When Node (left, right): Show 1.
"#;

#[test]
fn direct_c_batches_sibling_recursive_enum_fields_into_one_allocation() {
    let (c, trace) = compile_to_c_traced(TREE).expect("tree program should compile to C");

    assert!(
        c.contains("lc_gc_alloc(2 * sizeof(Tree))"),
        "a binary recursive variant should allocate both child values together"
    );
    assert_eq!(
        c.matches("lc_gc_alloc(sizeof(Tree))").count(),
        0,
        "per-child allocations silently restored the binary-tree malloc multiplier"
    );
    assert!(
        trace.lowering.fired(Opt::Scalarize),
        "recursive allocation coalescing must be visible in the C lowering trace"
    );

    let scalarize_off = compile_to_c(&decorate_source(TREE, &["scalarize"]))
        .expect("tree program should compile without scalarization");
    assert!(!scalarize_off.contains("lc_gc_alloc(2 * sizeof(Tree))"));
    assert_eq!(scalarize_off.matches("lc_gc_alloc(sizeof(Tree))").count(), 2);
}

#[test]
fn bytecode_c_arenas_recursive_enum_allocations() {
    let (c, trace) = compile_to_c_via_bytecode_traced(TREE)
        .expect("tree program should compile through bytecode to C");

    assert!(
        c.contains("lc_enum_alloc(3)"),
        "a binary enum value should come from the bytecode-C enum arena"
    );
    assert!(
        trace.lowering.fired(Opt::Scalarize),
        "enum arena allocation must be visible in the bytecode-C lowering trace"
    );

    let disabled = decorate_source(TREE, &["scalarize"]);
    let (scalarize_off, off_trace) = compile_to_c_via_bytecode_traced(&disabled)
        .expect("tree program should compile through bytecode without scalarization");
    assert!(!scalarize_off.contains("lc_enum_alloc(3)"));
    assert!(
        scalarize_off.contains("calloc(3, sizeof(int64_t))"),
        "disabling scalarize must retain the ordinary per-enum allocation"
    );
    assert!(!off_trace.lowering.fired(Opt::Scalarize));
}
