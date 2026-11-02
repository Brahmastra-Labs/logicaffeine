//! What a host call leaves behind. A trapped call and a clean one must both hand the machine
//! back in exactly the shape it was in — register file, frame base, iterator stack — because
//! nothing a program can observe would notice a leak: iterators are stack-disciplined and a
//! later frame is laid out past whatever an earlier one abandoned. Only the memory grows, and
//! only a page that answers enough events finds out.

use super::*;
use crate::interpreter::RuntimeValue;

/// `deep` walks a Text, which runs on the iterator stack (a range of Ints is a counted loop with no iterator), so a
/// failure inside it leaves frames and iterators open.
const SRC: &str = "\
## To deep (n: Int) -> Int:
    Repeat for c in \"abc\":
        If n is 0:
            Return item 9 of [1].
        Set counter to counter + deep(n - 1).
    Return 0.

## To sumTo (n: Int) -> Int:
    Let total be 0.
    Repeat for i from 1 to n:
        Set total to total + i.
    Return total.

## Main
Let counter be 1.
";

fn with_vm(src: &str, f: impl FnOnce(&mut Vm<'_>, &crate::intern::Interner)) {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _) = parsed.expect("parses");
        let program = crate::vm::Compiler::compile_with_types(stmts, interner, Some(types))
            .expect("compiles");
        let mut vm = Vm::new(&program);
        f(&mut vm, interner);
    });
}

fn fi(vm: &Vm<'_>, interner: &crate::intern::Interner, name: &str) -> FuncIdx {
    let sym = interner.lookup(name).expect("the name is interned");
    vm.program.fn_index[&sym]
}

fn shape(vm: &Vm<'_>) -> (usize, usize, usize, bool) {
    (
        vm.registers.len(),
        vm.base,
        vm.iter_stack.len(),
        vm.sched_active,
    )
}

#[test]
fn a_clean_host_call_leaves_the_machine_as_it_found_it() {
    with_vm(SRC, |vm, interner| {
        vm.run().unwrap();
        let before = shape(vm);
        let sum = vm
            .invoke_function(fi(vm, interner, "sumTo"), vec![RuntimeValue::Int(4)])
            .unwrap();
        assert!(matches!(sum, RuntimeValue::Int(10)));
        assert_eq!(shape(vm), before);
    });
}

#[test]
fn a_trapped_host_call_leaves_the_machine_as_it_found_it() {
    with_vm(SRC, |vm, interner| {
        vm.run().unwrap();
        let before = shape(vm);
        let deep = fi(vm, interner, "deep");
        assert!(vm
            .invoke_function(deep, vec![RuntimeValue::Int(3)])
            .is_err());
        assert_eq!(
            shape(vm),
            before,
            "three frames and their open loops are all given back"
        );
    });
}

#[test]
fn a_closure_call_leaves_the_machine_as_it_found_it() {
    let src = "## Main\nLet f be (n: Int) -> item n of [1, 2].\n";
    with_vm(src, |vm, _| {
        vm.run().unwrap();
        let closure = (0..vm.program.register_count)
            .map(|r| vm.registers[r].as_runtime().clone())
            .find(|v| matches!(v, RuntimeValue::Function(_)))
            .expect("main left its closure in a register");
        let before = shape(vm);
        assert!(matches!(
            vm.call_closure(&closure, vec![RuntimeValue::Int(2)]),
            Ok(RuntimeValue::Int(2))
        ));
        assert_eq!(shape(vm), before);
        assert!(vm
            .call_closure(&closure, vec![RuntimeValue::Int(9)])
            .is_err());
        assert_eq!(shape(vm), before);
    });
}

/// The loop walks a Text, which runs on the iterator stack (a range of Ints is a counted loop with no iterator), so the
/// failure leaves a frame and an iterator open for `quiesce` to unwind.
#[test]
fn a_main_that_failed_deep_is_quiesced_to_its_own_frame() {
    let src = "\
## To deep (n: Int) -> Int:
    Repeat for c in \"abc\":
        If n is 0:
            Return item 9 of [1].
        Set counter to counter + deep(n - 1).
    Return 0.

## Main
Let counter be 1.
Let r be deep(3).
";
    with_vm(src, |vm, _| {
        assert!(vm.run().is_err());
        assert!(
            vm.base > 0 && !vm.iter_stack.is_empty(),
            "the failure stopped inside the calls"
        );
        vm.quiesce();
        assert_eq!(shape(vm), (vm.program.register_count, 0, 0, false));
    });
}

#[test]
fn a_session_whose_main_failed_deep_is_back_on_mains_frame() {
    let src = "\
## To deep (n: Int) -> Int:
    Repeat for c in \"abc\":
        If n is 0:
            Return item 9 of [1].
        Set counter to counter + deep(n - 1).
    Return 0.

## Main
Let counter be 1.
Let r be deep(3).
";
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _) = parsed.expect("parses");
        let program = crate::vm::Compiler::compile_with_types(stmts, interner, Some(types))
            .expect("compiles");
        let mut session = crate::vm::session::Session::new(Vm::new(&program), interner);
        assert!(session.run_main().is_err());
        let vm = session.machine();
        assert_eq!(shape(vm), (program.register_count, 0, 0, false));
    });
}
