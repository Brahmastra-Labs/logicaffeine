use super::*;

/// Run the emitted module's `f` through the pure-Rust wasmi interpreter (instantiation
/// also validates the bytes), returning `f(arg)` — the end-to-end codegen proof.
fn run(module: &[u8], arg: i64) -> i64 {
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, module).expect("emitted bytes are valid wasm");
    let mut store = wasmi::Store::new(&engine, ());
    let instance = wasmi::Linker::<()>::new(&engine)
        .instantiate(&mut store, &m)
        .unwrap()
        .start(&mut store)
        .unwrap();
    let f = instance.get_typed_func::<i64, i64>(&store, "f").unwrap();
    f.call(&mut store, arg).unwrap()
}

#[test]
fn wasm_jit_emits_and_runs_straight_line() {
    // f(x) = x*x + x
    let ops = vec![
        Op::Mul {
            dst: 1,
            lhs: 0,
            rhs: 0,
        },
        Op::Add {
            dst: 2,
            lhs: 1,
            rhs: 0,
        },
        Op::Return { src: 2 },
    ];
    let module = compile_region_to_wasm(&ops, &[], 1, 3).expect("emits");
    assert_eq!(run(&module, 5), 30);
    assert_eq!(run(&module, 7), 56);
}

#[test]
fn wasm_jit_emits_and_runs_loop() {
    // f(n) = n + (n-1) + ... + 1   (triangular number) via a counted loop:
    //   acc = 0; one = 1; zero = 0;
    //   loop:  cond = n > zero;  if !cond goto exit;  acc += n;  n -= one;  goto loop;
    //   exit:  return acc
    // regs: 0=n, 1=acc, 2=one, 3=zero, 4=cond. consts: [0, 1].
    let ops = vec![
        Op::LoadConst { dst: 1, idx: 0 }, // acc = 0
        Op::LoadConst { dst: 2, idx: 1 }, // one = 1
        Op::LoadConst { dst: 3, idx: 0 }, // zero = 0
        Op::Gt {
            dst: 4,
            lhs: 0,
            rhs: 3,
        }, // [pc 3] cond = n > 0
        Op::JumpIfFalse { cond: 4, target: 8 }, // if !cond goto 8
        Op::Add {
            dst: 1,
            lhs: 1,
            rhs: 0,
        }, // acc += n
        Op::Sub {
            dst: 0,
            lhs: 0,
            rhs: 2,
        }, // n -= 1
        Op::Jump { target: 3 },           // goto 3
        Op::Return { src: 1 },            // [pc 8] return acc
    ];
    let consts = vec![Constant::Int(0), Constant::Int(1)];
    let module = compile_region_to_wasm(&ops, &consts, 1, 5).expect("loop region emits");
    assert_eq!(run(&module, 5), 15); // 5+4+3+2+1
    assert_eq!(run(&module, 10), 55);
    assert_eq!(run(&module, 1), 1);
    assert_eq!(run(&module, 0), 0);
    assert_eq!(run(&module, 100), 5050);
}

#[test]
fn wasm_jit_emits_and_runs_branch() {
    // f(x) = if x < 10 { x * 2 } else { x + 100 }
    // regs: 0=x, 1=ten, 2=cond, 3=two, 4=result, 5=hundred. consts: [10, 2, 100].
    let ops = vec![
        Op::LoadConst { dst: 1, idx: 0 }, // ten = 10
        Op::Lt {
            dst: 2,
            lhs: 0,
            rhs: 1,
        }, // cond = x < 10
        Op::JumpIfFalse { cond: 2, target: 6 }, // if !cond goto else(6)
        Op::LoadConst { dst: 3, idx: 1 }, // two = 2
        Op::Mul {
            dst: 4,
            lhs: 0,
            rhs: 3,
        }, // result = x * 2
        Op::Return { src: 4 },
        Op::LoadConst { dst: 5, idx: 2 }, // [pc 6] hundred = 100
        Op::Add {
            dst: 4,
            lhs: 0,
            rhs: 5,
        }, // result = x + 100
        Op::Return { src: 4 },
    ];
    let consts = vec![Constant::Int(10), Constant::Int(2), Constant::Int(100)];
    let module = compile_region_to_wasm(&ops, &consts, 1, 6).expect("branch region emits");
    assert_eq!(run(&module, 3), 6);
    assert_eq!(run(&module, 9), 18);
    assert_eq!(run(&module, 10), 110);
    assert_eq!(run(&module, 50), 150);
}

#[test]
fn wasm_jit_rejects_concurrency_op() {
    let ops = vec![Op::ChanRecv { dst: 1, chan: 0 }, Op::Return { src: 1 }];
    assert!(
        compile_region_to_wasm(&ops, &[], 1, 2).is_none(),
        "concurrency ops are WASM-JIT-ineligible"
    );
}

#[test]
fn wasm_jit_requires_a_return() {
    let ops = vec![Op::Add {
        dst: 1,
        lhs: 0,
        rhs: 0,
    }];
    assert!(compile_region_to_wasm(&ops, &[], 1, 2).is_none());
}

/// `Not` negates TRUTHINESS — for a raw i64 register the WASM-JIT cannot
/// tell a Bool 0/1 from a general Int without kind context, so it MUST
/// reject it (region deopts to the VM) rather than guess. This locks the
/// soundness boundary: the integer fragment stops where semantics need kinds.
#[test]
fn wasm_jit_rejects_type_ambiguous_ops() {
    for op in [Op::Not { dst: 1, src: 0 }] {
        let ops = vec![op, Op::Return { src: 1 }];
        assert!(
            compile_region_to_wasm(&ops, &[], 1, 2).is_none(),
            "type-overloaded op must be WASM-JIT-ineligible (deopt, not miscompile): {:?}",
            ops[0]
        );
    }
}
