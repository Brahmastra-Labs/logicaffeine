//! wprog_engine_lock — the COMPOSITION-completeness lock for the whole-program spatial fabric. The goal is
//! not "these 5 programs run" but "ANY program runs." This lock fuzzes ARBITRARY branchy programs — random
//! guard ladders over random comparisons, constants, and arithmetic returns, at random shapes — and proves
//! every one runs on the whole-program fabric config bit-identically to the VM:
//!
//!     eval_wprog_config  ≡  run_program_cfg_on_grid  ≡  eval_program_cfg_fabric  ≡  the VM
//!
//! Together with `fabric_coverage_lock` (every one of the 115 ISA ops is realized on the fabric AND proven
//! ≡VM, `UNCOVERED_CEILING = 0`), this is the composition half of the totality argument: op-closure + per-op
//! ≡VM (fabric_coverage) + arbitrary-program composition (this fuzzer) ⇒ any program in the supported family
//! runs on the fabric. The family here is the acyclic branchy control-flow fragment (Arith + Select); loops
//! (the loop fabric), the state plane (heap), and calls (a stack) extend the family toward the whole language,
//! each with its own composition fuzzer.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(wprog_engine_lock)'`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::fabric_grid::run_program_cfg_on_grid;
use logicaffeine_compile::codegen_sva::fabric_rtl::{eval_program_cfg_fabric, program_cfg_fabric};
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config, serialize_program, WBlock, WCell, WProgConfig, WSrc, WTerm,
};
use logicaffeine_compile::compile::vm_outcome;

/// The RECURSION mechanism test: hand-build the whole-program config for `f(n) = if n<=0 { 0 } else
/// { n + f(n-1) }` (a self-recursive function with a `Call` terminator) and prove the frame-stack interpreter
/// computes `sum(0..=n)` for a range of depths — i.e. the call stack composes recursion correctly. This
/// proves the CORE spatial call mechanism; the lifter that emits `Call` terminators from source is the
/// separate mechanical piece that closes the census `recursion` gap.
fn recursive_sumdown_config() -> WProgConfig {
    let n = WSrc {
        mode: 0,
        reference: 0,
        konst: 0,
    }; // In(0)
    let rest = WSrc {
        mode: 0,
        reference: 1,
        konst: 0,
    }; // In(1) in the join block
    let c0 = WSrc {
        mode: 1,
        reference: 0,
        konst: 0,
    }; // Const(0)
    let c1 = WSrc {
        mode: 1,
        reference: 0,
        konst: 1,
    }; // Const(1)
    let v0 = WSrc {
        mode: 2,
        reference: 0,
        konst: 0,
    }; // Val(0) — the block's first cell
    WProgConfig {
        n_params: 1,
        blocks: vec![
            // 0 (entry): cmp = (n <= 0); Branch(cmp, then=1 else=2)
            WBlock {
                livein: vec![0],
                cells: vec![WCell {
                    kind: 1,
                    code: 2, /*Le*/
                    ops: [n, c0, c1, c0],
                }],
                writes: vec![],
                term: WTerm::Branch {
                    cond: v0,
                    then_blk: 1,
                    else_blk: 2,
                },
            },
            // 1 (base case): Return 0
            WBlock {
                livein: vec![],
                cells: vec![],
                writes: vec![],
                term: WTerm::Return(c0),
            },
            // 2 (recurse): a = n - 1; Call f(a) → rest (reg 3), resume at block 3
            WBlock {
                livein: vec![0],
                cells: vec![WCell {
                    kind: 0,
                    code: 1, /*Sub*/
                    ops: [n, c1, c0, c0],
                }],
                writes: vec![],
                term: WTerm::Call {
                    callee_block: 0,
                    args: vec![v0],
                    dst_reg: 3,
                    ret_block: 3,
                },
            },
            // 3 (join): r = n + rest; Return r
            WBlock {
                livein: vec![0, 3],
                cells: vec![WCell {
                    kind: 0,
                    code: 0, /*Add*/
                    ops: [n, rest, c0, c0],
                }],
                writes: vec![],
                term: WTerm::Return(v0),
            },
        ],
    }
}

#[test]
fn recursion_runs_on_the_frame_stack_interpreter() {
    let cfg = recursive_sumdown_config();
    for n in [0i64, 1, 2, 5, 8, 12, 20] {
        let got = eval_wprog_config(&cfg, &[n]);
        let expected: i64 = (0..=n).sum();
        assert_eq!(
            got, expected,
            "call-stack f({n}) = {got}, expected sum(0..={n}) = {expected}"
        );
    }
    println!("recursion: the frame-stack interpreter composes self-recursion correctly (sum-down, depth ≤ 20)");
}

/// A deterministic PRNG (SplitMix64) — reproducible fuzzing, no external `rand`.
fn mix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Render a signed constant as LOGOS source (negatives via `(0 - n)`, the language's negation form).
fn konst(n: i64) -> String {
    if n < 0 {
        format!("(0 - {})", -n)
    } else {
        n.to_string()
    }
}

/// A random arithmetic return expression over the input `x` and constants — the block bodies (Arith cells).
fn rand_arith(st: &mut u64) -> String {
    let k = (mix(st) % 13) as i64 - 6;
    let m = (mix(st) % 41) as i64 - 20;
    match mix(st) % 6 {
        0 => "x".to_string(),
        1 => format!("x + {}", konst(m)),
        2 => format!("x - {}", konst(m)),
        3 => format!("x * {}", konst(k)),
        4 => format!("x * {} + {}", konst(k), konst(m)),
        _ => konst(m),
    }
}

/// A random COMPARISON on the input `x` vs a **literal non-negative** constant. The comparison operands must
/// be inputs/constants for the spatial placer (`route_loop_body`) to route the predicate — and a LOGOS
/// negative is `(0 - n)`, a COMPUTED value, which the placer refuses. So `If x is greater than -5` is a valid
/// program the soft-core runs but the fabric cannot place yet (computed-predicate routing = the placer's
/// documented future work). The generated family is therefore the FABRIC-PLACEABLE branchy fragment; this
/// exclusion is exactly one of the gaps between "runs on the chip (soft-core)" and "runs on the fabric".
fn rand_cmp(st: &mut u64) -> String {
    let cmp = [
        "is less than",
        "is greater than",
        "is at most",
        "is at least",
        "is equal to",
    ][(mix(st) % 5) as usize];
    let c = (mix(st) % 41) as i64; // literal, non-negative ⇒ a placeable comparison operand
    format!("x {cmp} {c}")
}

/// Generate a random branchy program: a guard ladder `If <cmp>: Return <arith>.` (2..=6 guards) then a final
/// `Return <arith>.` — an arbitrary acyclic multi-block control-flow function over one integer input.
fn rand_program(st: &mut u64) -> String {
    let n_guards = 2 + (mix(st) % 5) as usize; // 2..=6 guards ⇒ a genuine multi-block CFG
    let mut body = String::new();
    for _ in 0..n_guards {
        body.push_str(&format!(
            "    If {}:\n        Return {}.\n",
            rand_cmp(st),
            rand_arith(st)
        ));
    }
    body.push_str(&format!("    Return {}.\n", rand_arith(st)));
    format!("## To f (x: Int) -> Int:\n{body}")
}

/// Call `f(x)` on the VM — the ground-truth oracle.
fn vm_f(src: &str, x: i64) -> i64 {
    let arg = if x < 0 {
        format!("(0 - {})", -x)
    } else {
        x.to_string()
    };
    let prog = format!("{src}## Main\n    Show f({arg}).\n");
    let out = vm_outcome(&prog);
    assert!(
        out.error.is_none(),
        "VM error on f({x}): {:?}\n{src}",
        out.error
    );
    out.output
        .split_whitespace()
        .next()
        .expect("a Show")
        .parse::<i64>()
        .expect("int")
}

#[test]
fn any_branchy_program_runs_on_the_whole_program_fabric_matching_the_vm() {
    let mut st = 0xC0FFEE_1234_5678u64;
    let mut programs = 0usize;
    let mut runs = 0usize;

    // 300 RANDOM branchy programs — arbitrary guard ladders — each run over a battery of inputs. Every one
    // must agree: the flat whole-program fabric config ≡ the place-per-block grid walk ≡ the CFG model ≡ VM.
    for _ in 0..300 {
        let src = rand_program(&mut st);
        let pf = program_cfg_fabric(&src, "f");
        assert!(
            pf.blocks.len() >= 3,
            "a guard ladder is a genuine multi-block CFG:\n{src}"
        );
        let cfg = serialize_program(&pf);
        programs += 1;

        for _ in 0..8 {
            let x = mix(&mut st) as i64 % 300 - 150; // inputs spanning the guard constants (both signs)
            let flat = eval_wprog_config(&cfg, &[x]);
            let grid = run_program_cfg_on_grid(&pf, &[x]);
            let model = eval_program_cfg_fabric(&pf, &[x]);
            let vm = vm_f(&src, x);
            assert_eq!(
                flat, grid,
                "flat config {flat} != grid walk {grid} on x={x}\n{src}"
            );
            assert_eq!(
                flat, model,
                "flat config {flat} != CFG model {model} on x={x}\n{src}"
            );
            assert_eq!(flat, vm, "flat config {flat} != VM {vm} on x={x}\n{src}");
            runs += 1;
        }
    }

    println!(
        "whole-program fabric: {programs} RANDOM branchy programs × inputs = {runs} runs, EVERY one \
         ≡ the VM — the acyclic branchy control-flow family runs on the fabric (composition-complete)"
    );
}

/// A random WHILE-LOOP program: an accumulator loop `While i <= x: acc += <f(i)>; i += 1` with random init
/// and body. A `While` compiles to a header block (condition → Branch) + a body block whose terminator jumps
/// BACK — a cyclic CFG. The generated loops terminate (i counts up to a positive x) and stay in i64 range.
fn rand_loop_program(st: &mut u64) -> String {
    let init = (mix(st) % 11) as i64;
    let body = match mix(st) % 4 {
        0 => "i".to_string(),
        1 => "i * i".to_string(),
        2 => format!("i + {}", (mix(st) % 7) as i64),
        _ => format!("i * {}", (mix(st) % 5) as i64 + 1),
    };
    format!(
        "## To f (x: Int) -> Int:\n    Let mutable acc be {init}.\n    Let mutable i be 1.\n    \
         While i is at most x:\n        Set acc to acc + {body}.\n        Set i to i + 1.\n    Return acc.\n"
    )
}

/// LOOPS run on the fabric: with the acyclic gate lifted, the block-walk re-enters the loop header each
/// iteration (loop state in the register file). This fuzzes arbitrary while-loop accumulators and proves each
/// runs on the whole-program fabric config ≡ the place-per-block grid walk ≡ the VM — the FIRST proof that
/// `run_program_cfg_on_grid` composes CYCLIC control flow bit-exactly, extending "any program on the fabric"
/// from the acyclic fragment to loops.
#[test]
fn while_loops_run_on_the_whole_program_fabric_matching_the_vm() {
    let mut st = 0x100F_0F0F_5EED_1234u64;
    let mut programs = 0usize;
    let mut runs = 0usize;

    for _ in 0..120 {
        let src = rand_loop_program(&mut st);
        let pf = program_cfg_fabric(&src, "f");
        let cfg = serialize_program(&pf);
        programs += 1;
        for _ in 0..6 {
            let x = (mix(&mut st) % 25 + 1) as i64; // positive & bounded ⇒ terminates, no i64 overflow
            let flat = eval_wprog_config(&cfg, &[x]);
            let grid = run_program_cfg_on_grid(&pf, &[x]);
            let vm = vm_f(&src, x);
            assert_eq!(
                flat, grid,
                "loop: flat {flat} != grid {grid} on x={x}\n{src}"
            );
            assert_eq!(flat, vm, "loop: flat {flat} != VM {vm} on x={x}\n{src}");
            runs += 1;
        }
    }

    println!(
        "whole-program fabric: {programs} RANDOM while-loop programs × inputs = {runs} runs, EVERY one \
         ≡ the VM — LOOPS run on the fabric (cyclic control-flow composition, the block-walk re-enters the header)"
    );
}

/// A random COLLECTION program: build a 3-element list (including the input), optionally mutate it in place
/// (`Set item`) or grow it (`Push`), then read an element back — the heap state plane (HeapAlloc/Write/Read
/// tiles), threaded across blocks.
fn rand_collection_program(st: &mut u64) -> String {
    let e0 = (mix(st) % 40) as i64;
    let e1 = (mix(st) % 40) as i64;
    let (mutate, read) = match mix(st) % 3 {
        0 => {
            let i = mix(st) % 3 + 1; // 1-based index in range
            let v = (mix(st) % 100) as i64;
            let k = mix(st) % 3 + 1;
            (
                format!("    Set item {i} of xs to {v}.\n"),
                format!("item {k} of xs"),
            )
        }
        1 => {
            let v = (mix(st) % 100) as i64;
            let k = mix(st) % 4 + 1; // after the push the list has 4 elements
            (format!("    Push {v} to xs.\n"), format!("item {k} of xs"))
        }
        _ => {
            let k = mix(st) % 3 + 1;
            (String::new(), format!("item {k} of xs"))
        }
    };
    format!("## To f (x: Int) -> Int:\n    Let mutable xs be [{e0}, x, {e1}].\n{mutate}    Return {read}.\n")
}

/// COLLECTIONS run on the fabric: with the heap threaded through the flat interpreter, arbitrary
/// list-alloc / set / push / index programs run on the whole-program fabric ≡ the place-per-block grid walk
/// (heap-threaded) ≡ the VM — the state plane composes with whole-program execution.
#[test]
fn collections_run_on_the_whole_program_fabric_matching_the_vm() {
    let mut st = 0xC011_EC75_EED9_9999u64;
    let mut programs = 0usize;
    let mut runs = 0usize;

    for _ in 0..120 {
        let src = rand_collection_program(&mut st);
        let pf = program_cfg_fabric(&src, "f");
        let cfg = serialize_program(&pf);
        programs += 1;
        for _ in 0..6 {
            let x = mix(&mut st) as i64 % 200 - 100;
            let flat = eval_wprog_config(&cfg, &[x]);
            let grid = run_program_cfg_on_grid(&pf, &[x]);
            let vm = vm_f(&src, x);
            assert_eq!(
                flat, grid,
                "collection: flat {flat} != grid {grid} on x={x}\n{src}"
            );
            assert_eq!(
                flat, vm,
                "collection: flat {flat} != VM {vm} on x={x}\n{src}"
            );
            runs += 1;
        }
    }

    println!(
        "whole-program fabric: {programs} RANDOM collection programs × inputs = {runs} runs, EVERY one \
         ≡ the VM — COLLECTIONS run on the fabric (heap state plane threaded through whole-program execution)"
    );
}
