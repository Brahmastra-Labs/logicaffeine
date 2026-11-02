//! A top-level write the program path does not compute is REFUSED, never dropped.
//!
//! The whole-program walk passes over simple statements, and a tensor's declared fill is folded from its
//! literal `Set item i of T to v` statements — so any other write to a declared tensor used to vanish, and the
//! lowered program then computed on values the program never had. The promises:
//!
//!   1. A write whose position or value is not a literal, a push that grows a declared tensor, and a rebinding
//!      of one are each refused by name (`kind: write`), naming the tensor.
//!   2. A LITERAL write placed after an op has used the tensor is refused too: folding it into the fill would
//!      move it before that op.
//!   3. Literal writes made before any op touches the tensor are the fill, and still lower exactly (the control).
//!   4. A name is bound ONCE: a `Let` that rebinds a declared tensor, a scalar or tensor declared a second time, and a
//!      write to a value an op has already read are each refused by name. Every section reads the one value a name
//!      was declared with; a second binding would leave the chip computing on the first while the VM reads the second.
//!
//! ★ MUTATION PROBE (run 2026-09-24 — break, observe red, restore byte-identical): the walk dropping every
//! `unlifted_write` → the four refusal tests red (`a write no section computes must stop the lowering`), the control
//! stays green.
//!
//! ★ MUTATION PROBES (run 2026-09-27 in a private copy; logs `logs/sn50/probe_binding_*.txt`):
//!   • a declaration of a name already bound admitted → the tensor declared twice and both scalar rebindings red;
//!   • a `Let` of a declared tensor passed over → `a_let_that_rebinds_a_declared_tensor_is_refused` reds;
//!   • a `Set` of a value an op has read passed over → `a_scalar_written_after_an_op_read_it_is_refused` reds.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_unlifted_write_lock`
#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::eval_choreography;
use logicaffeine_compile::spatial::ir::Phase;
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;
use tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs};

const HEAD: &str = "## Main\nLet T be a 4 by 3 tensor of Int.\nSet item 4 of T to 40.\nSet item 5 of T to 50.\nLet tok be 2.\n";

/// The source lowers not at all, and the refusal names a `write` to `T` with `why`.
fn refused(src: &str, why: &str) {
    refused_naming(src, "T", why);
}

/// The source lowers not at all, and the refusal names a `write` to `binder` with `why`.
fn refused_naming(src: &str, binder: &str, why: &str) {
    let pl = compile_program_to_spatial(src, &SpatialTarget::sn50_rdu())
        .unwrap_or_else(|e| panic!("walk refused outright: {e:?}\n{src}"));
    assert!(
        pl.whole().is_none(),
        "a write no section computes must stop the lowering:\n{src}"
    );
    let hit = pl
        .missing
        .iter()
        .find(|m| m.kind == "write")
        .unwrap_or_else(|| panic!("no `write` refusal among {:?}\n{src}", pl.missing));
    assert_eq!(hit.binder, binder, "the refusal names the value written");
    assert!(
        hit.why.contains(why),
        "the refusal says why ({why:?}), got {:?}",
        hit.why
    );
}

#[test]
fn a_write_whose_value_is_not_a_literal_is_refused() {
    refused(
        &format!(
            "{HEAD}Let v be 7.\nSet item 6 of T to v.\nLet x be row tok of T.\nShow item 3 of x.\n"
        ),
        "not a literal",
    );
}

#[test]
fn a_write_whose_position_is_not_a_literal_is_refused() {
    refused(
        &format!(
            "{HEAD}Let p be 6.\nSet item p of T to 9.\nLet x be row tok of T.\nShow item 3 of x.\n"
        ),
        "not a literal",
    );
}

#[test]
fn a_push_onto_a_declared_tensor_is_refused() {
    refused(
        &format!("{HEAD}Push 9 to T.\nLet x be row tok of T.\nShow item 3 of x.\n"),
        "push",
    );
}

#[test]
fn a_literal_write_after_an_op_used_the_tensor_is_refused() {
    // The VM gathers row 2 as [40, 50, 0] and only then writes 60; folding the write into T's fill would gather 60.
    let src = format!("{HEAD}Let x be row tok of T.\nSet item 6 of T to 60.\nShow item 3 of x.\n");
    let vm = vm_outcome(&src);
    assert_eq!(
        oracle_ints(&vm.output).expect("vm"),
        vec![0],
        "the VM reads the row before the write"
    );
    refused(&src, "after an op");
}

#[test]
fn literal_writes_before_any_op_are_the_fill_and_lower_exactly() {
    let src = format!("{HEAD}Set item 6 of T to 60.\nLet x be row tok of T.\nShow item 1 of x.\nShow item 2 of x.\nShow item 3 of x.\n");
    let pl = compile_program_to_spatial(&src, &SpatialTarget::sn50_rdu()).expect("walk");
    let sp = pl
        .whole()
        .unwrap_or_else(|| panic!("the control lowers: {}", pl.refusal()));
    assert_eq!(sp.phase(), Phase::Verified);
    let out = eval_choreography(sp, &inputs_from_fills(sp).expect("fills")).expect("eval");
    let vm = vm_outcome(&src);
    assert_eq!(
        shown_from_outputs(&src, sp, &out).expect("shown"),
        oracle_ints(&vm.output).expect("vm")
    );
    assert_eq!(oracle_ints(&vm.output).expect("vm"), vec![40, 50, 60]);
}

// ───────────────────────────── 4. a name is bound once ─────────────────────────────

/// The VM's shown values — the ones a lowering that kept the FIRST binding would disagree with.
fn vm_shows(src: &str) -> Vec<i64> {
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "{src}");
    oracle_ints(&vm.output).expect("vm")
}

#[test]
fn a_let_that_rebinds_a_declared_tensor_is_refused() {
    // The VM gathers row 2 of `V` (9); a section reading `T` would gather row 2 of `T` (40).
    let src = format!("{HEAD}Let V be a 4 by 3 tensor of Int.\nSet item 4 of V to 9.\nLet T be V.\nLet x be row tok of T.\nShow item 1 of x.\n");
    assert_eq!(vm_shows(&src), vec![9], "the VM reads the rebound tensor");
    refused(&src, "rebinding");
}

#[test]
fn a_scalar_bound_again_after_an_op_read_it_is_refused() {
    // The VM gathers row 2 (40) and then row 3 (70); one `tok` on the chip would gather row 2 twice.
    let src = format!("{HEAD}Set item 7 of T to 70.\nLet x be row tok of T.\nLet tok be 3.\nLet y be row tok of T.\nShow item 1 of x.\nShow item 1 of y.\n");
    assert_eq!(
        vm_shows(&src),
        vec![40, 70],
        "the VM reads each binding where it stands"
    );
    refused_naming(&src, "tok", "second binding");
}

#[test]
fn a_scalar_bound_twice_before_any_op_is_refused() {
    let src = format!(
        "{HEAD}Set item 7 of T to 70.\nLet tok be 3.\nLet x be row tok of T.\nShow item 1 of x.\n"
    );
    assert_eq!(vm_shows(&src), vec![70], "the VM reads the second binding");
    refused_naming(&src, "tok", "second binding");
}

#[test]
fn a_tensor_declared_twice_is_refused() {
    // The second declaration is zeros; the literal writes before it belong to the first.
    let src = format!(
        "{HEAD}Let T be a 4 by 3 tensor of Int.\nLet x be row tok of T.\nShow item 1 of x.\n"
    );
    assert_eq!(
        vm_shows(&src),
        vec![0],
        "the VM reads the second declaration"
    );
    refused(&src, "second binding");
}

#[test]
fn a_scalar_written_after_an_op_read_it_is_refused() {
    let src = format!("{HEAD}Set item 7 of T to 70.\nLet x be row tok of T.\nSet tok to 3.\nLet y be row tok of T.\nShow item 1 of x.\nShow item 1 of y.\n");
    assert_eq!(
        vm_shows(&src),
        vec![40, 70],
        "the VM reads the write where it stands"
    );
    refused_naming(&src, "tok", "already read");
}

#[test]
fn a_let_that_rebinds_an_ops_result_is_refused() {
    // `x` is an op's result; bound again to `V`'s rows, a later op reading `x` would read the op's result on the chip.
    let src = format!("{HEAD}Let V be a 4 by 3 tensor of Int.\nSet item 4 of V to 9.\nLet x be row tok of T.\nLet x be row tok of V.\nShow item 1 of x.\n");
    assert_eq!(vm_shows(&src), vec![9], "the VM reads the second binding");
    let pl = compile_program_to_spatial(&src, &SpatialTarget::sn50_rdu()).expect("walk");
    assert!(
        pl.whole().is_none(),
        "a second binding of an op's result must not lower as the first: {:?}",
        pl.missing
    );
}
