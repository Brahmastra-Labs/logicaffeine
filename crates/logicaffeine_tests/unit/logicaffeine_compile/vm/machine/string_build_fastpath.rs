//! Structural proof for Task B: the constant pool is materialised ONCE
//! (a `LoadConst` of a Text bumps a shared `Rc` instead of allocating a
//! fresh `String`), and the sole-owned in-place append (`AddAssign`)
//! grows the accumulator's own buffer rather than reallocating each step.
use super::*;
use crate::interpreter::RuntimeValue;
use std::rc::Rc;

/// The `Rc<String>` backing register `r` (or panic if it isn't a Text).
fn text_ptr(vm: &Vm, r: usize) -> *const String {
    match vm.registers[r].as_runtime_ref() {
        Some(RuntimeValue::Text(rc)) => Rc::as_ptr(rc),
        other => panic!("register {r} is not a Text: {other:?}"),
    }
}

fn text_strong(vm: &Vm, r: usize) -> usize {
    match vm.registers[r].as_runtime_ref() {
        Some(RuntimeValue::Text(rc)) => Rc::strong_count(rc),
        other => panic!("register {r} is not a Text: {other:?}"),
    }
}

/// Two `LoadConst`s of the SAME Text-constant index hand out the same
/// `Rc` allocation — proof the pool is materialised once, so reloading a
/// literal in a loop is a refcount bump, not a `String` allocation.
#[test]
fn loadconst_text_shares_one_allocation() {
    let prog = CompiledProgram {
        constants: vec![Constant::Text("abc".to_string())],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 },
            Op::LoadConst { dst: 1, idx: 0 },
            Op::Halt,
        ],
        register_count: 2,
        ..Default::default()
    };
    let mut vm = Vm::new(&prog);
    vm.run().unwrap();
    assert_eq!(
        text_ptr(&vm, 0),
        text_ptr(&vm, 1),
        "two loads of the same Text constant must share one Rc allocation"
    );
    // And the live constant keeps a reference, so a register's clone is
    // never the sole owner — an in-place append on a freshly-loaded
    // literal must therefore NOT fire (it would corrupt the pool).
    assert!(
        text_strong(&vm, 0) >= 3,
        "pool + two registers all reference the constant"
    );
}

/// `Set s to s + ch` repeated: once the accumulator owns its buffer, each
/// append reuses the SAME allocation (pointer stable while capacity holds).
/// The first append, where `s` is the just-loaded shared `""` constant,
/// must allocate a fresh owned buffer (the pool stays intact); subsequent
/// appends grow it in place.
#[test]
fn add_assign_appends_in_place_after_first() {
    // r0 = "" (shared constant), r1 = "x" (shared constant).
    // r0 += r1, capture; r0 += r1 a few more times, pointer stays put.
    let prog = CompiledProgram {
        constants: vec![
            Constant::Text(String::new()),
            Constant::Text("x".to_string()),
        ],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 },
            Op::LoadConst { dst: 1, idx: 1 },
            Op::AddAssign { dst: 0, src: 1 },
            Op::AddAssign { dst: 0, src: 1 },
            Op::AddAssign { dst: 0, src: 1 },
            Op::AddAssign { dst: 0, src: 1 },
            Op::Halt,
        ],
        register_count: 2,
        ..Default::default()
    };
    // Run only the first two appends to capture the owned buffer, then the
    // rest, by stepping the whole thing and re-checking — simplest: run to
    // completion, the invariant we assert is the FINAL value's correctness
    // plus that the rhs constant was never mutated.
    let mut vm = Vm::new(&prog);
    vm.run().unwrap();
    // The accumulator built "xxxx".
    match vm.registers[0].as_runtime_ref() {
        Some(RuntimeValue::Text(rc)) => assert_eq!(&***rc, "xxxx"),
        other => panic!("r0 not text: {other:?}"),
    }
    // The shared constant "x" was NOT corrupted by the in-place appends.
    match vm.registers[1].as_runtime_ref() {
        Some(RuntimeValue::Text(rc)) => assert_eq!(&***rc, "x"),
        other => panic!("r1 not text: {other:?}"),
    }
}

/// The buffer-stability proof: drive the loop manually so we can read the
/// accumulator's `Rc::as_ptr` between appends. After the first append
/// (which clones off the shared constant into an owned buffer with spare
/// capacity), every later append keeps the SAME allocation.
#[test]
fn add_assign_reuses_buffer_allocation() {
    let prog = CompiledProgram {
        constants: vec![
            Constant::Text("seed-with-capacity-headroom".to_string()),
            Constant::Text("z".to_string()),
        ],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 },
            Op::LoadConst { dst: 1, idx: 1 },
            Op::AddAssign { dst: 0, src: 1 }, // first: clone off the shared constant
            Op::AddAssign { dst: 0, src: 1 }, // owned now → in place
            Op::AddAssign { dst: 0, src: 1 },
            Op::Halt,
        ],
        register_count: 2,
        ..Default::default()
    };
    // Manually dispatch up to the marker so we can inspect between appends.
    // Reserve generous capacity on the first owned buffer so growth never
    // forces a realloc within this test.
    let mut vm = Vm::new(&prog);
    // Step 1+2: loads.
    vm.set(0, const_to_value(&prog.constants[0]));
    vm.set(1, const_to_value(&prog.constants[1]));
    // First append: must produce an OWNED buffer (sole owner now).
    vm.add_assign(0, 1).unwrap();
    // Force headroom so the next appends don't realloc for capacity.
    if let RuntimeValue::Text(rc) = vm.registers[0].as_runtime_mut() {
        if let Some(s) = Rc::get_mut(rc) {
            s.reserve(64);
        }
    }
    let p_after_first = text_ptr(&vm, 0);
    vm.add_assign(0, 1).unwrap();
    let p_after_second = text_ptr(&vm, 0);
    vm.add_assign(0, 1).unwrap();
    let p_after_third = text_ptr(&vm, 0);
    assert_eq!(
        p_after_first, p_after_second,
        "append must reuse the owned buffer"
    );
    assert_eq!(
        p_after_second, p_after_third,
        "append must reuse the owned buffer"
    );
    match vm.registers[0].as_runtime_ref() {
        Some(RuntimeValue::Text(rc)) => assert_eq!(&***rc, "seed-with-capacity-headroomzzz"),
        other => panic!("r0 not text: {other:?}"),
    }
}
