use super::*;
use crate::vm::CompiledProgram;
use logicaffeine_base::Interner;

fn with_compiled<R>(src: &str, f: impl FnOnce(&CompiledProgram, &Interner) -> R) -> R {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _policies) = parsed.expect("source parses");
        let program = crate::vm::Compiler::compile_with_types(stmts, interner, Some(types))
            .expect("source compiles");
        f(&program, interner)
    })
}

/// Totality smoke: every ISA variant classifies and enumerates its writes without
/// panicking (the exhaustive matches make a silent gap impossible to compile).
#[test]
fn every_isa_op_has_a_memory_class_and_write_set() {
    for op in Op::all_variants() {
        let _ = site_kind(&op);
        op.for_each_written_reg(|_| {});
    }
    // The ISA only grows; a new op cannot compile until the exhaustive matches above
    // classify it, so this floor just witnesses that the census is non-trivial.
    assert!(
        Op::all_variants().len() >= 112,
        "ISA op count regressed below the known floor"
    );
}

/// An affine fill-then-scan loop is FULLY static: pushes are streams, reads are
/// induction-indexed.
#[test]
fn affine_fill_and_scan_is_fully_static() {
    let src = "Let xs be a new Seq of Int.\n\
                   Let i be 1.\n\
                   While i is at most 100:\n\
                   \tPush i to xs.\n\
                   \tSet i to i + 1.\n\
                   Let total be 0.\n\
                   Set i to 1.\n\
                   While i is at most 100:\n\
                   \tSet total to total + xs[i].\n\
                   \tSet i to i + 1.\n\
                   Show total.";
    with_compiled(src, |prog, _| {
        let r = analyze(prog);
        assert!(
            r.datapath_sites() > 0,
            "expected memory sites in the fill/scan kernel: {r:?}"
        );
        assert_eq!(
            r.dynamic_sites, 0,
            "affine kernel must have zero dynamic sites: {r:?}"
        );
        assert_eq!(r.static_fraction(), 1.0);
    });
}

/// A pointer-chase (`xs[xs[i]]` — the index comes from memory) is DYNAMIC at the
/// gather site, and the analysis refuses to call it static.
#[test]
fn pointer_chase_gather_is_dynamic() {
    let src = "Let xs be a new Seq of Int.\n\
                   Let i be 1.\n\
                   While i is at most 10:\n\
                   \tPush i to xs.\n\
                   \tSet i to i + 1.\n\
                   Let j be xs[3].\n\
                   Let y be xs[j].\n\
                   Show y.";
    with_compiled(src, |prog, _| {
        let r = analyze(prog);
        assert!(
            r.dynamic_sites >= 1,
            "the xs[j] gather must be classified dynamic: {r:?}"
        );
        assert!(r.static_fraction() < 1.0);
    });
}

/// Hash structures are honestly dynamic — a set membership test is not a stream.
#[test]
fn hash_structure_access_is_dynamic() {
    let src = "Let s be a new Set of Int.\n\
                   Add 3 to s.\n\
                   Show s contains 3.";
    with_compiled(src, |prog, _| {
        let r = analyze(prog);
        assert!(
            r.dynamic_sites >= 2,
            "SetAdd + Contains must both be dynamic: {r:?}"
        );
    });
}

/// A register that is stepped by a NON-constant amount is not an induction
/// variable, and an access through it is dynamic (the floor never lies upward).
#[test]
fn non_constant_step_is_not_induction() {
    let src = "Let xs be a new Seq of Int.\n\
                   Let i be 1.\n\
                   While i is at most 10:\n\
                   \tPush i to xs.\n\
                   \tSet i to i + 1.\n\
                   Let step be xs[2].\n\
                   Let k be 1.\n\
                   Let total be 0.\n\
                   While k is at most 5:\n\
                   \tSet total to total + xs[k].\n\
                   \tSet k to k + step.\n\
                   Show total.";
    with_compiled(src, |prog, _| {
        let r = analyze(prog);
        assert!(
            r.dynamic_sites >= 1,
            "xs[k] with a data-dependent step must be dynamic: {r:?}"
        );
    });
}
