//! fabric_sequencer_state_lock — Stage 2a of the self-synthesizing bytecode XPU: the STATE PLANE in the
//! block-walk sequencer. Stage 1 (`fabric_sequencer_lock`) ran branchy STATELESS whole programs on the
//! sequencer. This lock adds the heap: a MULTI-block program that allocates a collection in one block and
//! reads/mutates it in OTHER blocks — the collection's pointer flowing through the register file, its
//! contents living in ONE shared heap threaded across every block reconfiguration.
//!
//! This is the semantic foundation the state-plane RTL (Stage 2b) targets: `program_cfg_fabric` now lowers
//! collection ops (NewList / item-of / Set item / push / pop) to `Mem` (heap) + `Arith` tiles exactly as the
//! whole-program model's `exec_ops` does, and `eval_program_cfg_fabric` / `run_program_cfg_on_grid` /
//! `eval_sequencer` thread ONE heap across the block walk. The lock pins the whole chain to the VM:
//!
//!   eval_sequencer ≡ run_program_cfg_on_grid ≡ eval_program_cfg_fabric ≡ the VM
//!
//! over a battery per program — for programs whose heap effects persist ACROSS block boundaries (a mutation
//! in a `then` block observed by a later block), which is exactly what a shared state plane must guarantee.
//! (The single-block / if-converted state plane is already proven in synthesizable BRAM by
//! `fabric_whole_program_lock::state_plane_rtl_matches_vm_under_iverilog`; this closes the MULTI-block gap at
//! the model level.)
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(fabric_sequencer_state_lock)'`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::fabric_grid::run_program_cfg_on_grid;
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    eval_program_cfg_fabric, program_cfg_fabric, LoopCell, ProgramCfgFabric,
};
use logicaffeine_compile::codegen_sva::sequencer_rtl::eval_sequencer;
use logicaffeine_compile::compile::vm_outcome;

/// One multi-block STATEFUL program: a LOGOS function using a collection across branches, its callee, and an
/// input battery (every vector's indices stay in range so the VM does not fault).
struct StateFn {
    src: &'static str,
    name: &'static str,
    args: &'static [&'static [i64]],
}

const CORPUS: &[StateFn] = &[
    // Allocate a list in the entry block; a branch on the input reads a DIFFERENT element in each successor.
    // The list pointer is live-in to both successor blocks — the register file carries it across the
    // reconfiguration, the heap carries the contents.
    StateFn {
        name: "pick",
        src: "## To pick (i: Int) -> Int:\n    Let xs be [10, 20, 30].\n    If i is greater than 0:\n        Return item i of xs.\n    Return item 1 of xs.\n",
        args: &[&[1], &[2], &[3], &[0], &[-5]],
    },
    // The SHARED-HEAP-PERSISTENCE test: mutate the list in a `then` block, then read it in a LATER (join)
    // block. The write must survive the block reconfiguration — the whole point of a persistent state plane.
    StateFn {
        name: "setpick",
        src: "## To setpick (i: Int, v: Int) -> Int:\n    Let mutable xs be [10, 20, 30].\n    If i is greater than 0:\n        Set item i of xs to v.\n    Return item 2 of xs.\n",
        args: &[&[2, 99], &[1, 99], &[3, 99], &[0, 99], &[-1, 7]],
    },
    // A length read across a branch: allocate, branch, then read the length in the join — exercises HeapRead
    // of the length header from a later block.
    StateFn {
        name: "lenpick",
        src: "## To lenpick (i: Int) -> Int:\n    Let xs be [5, 6, 7, 8].\n    If i is greater than 0:\n        Return item i of xs.\n    Return length of xs.\n",
        args: &[&[1], &[4], &[0], &[-2]],
    },
];

/// True if any block body carries a state-plane (heap) tile — i.e. the program genuinely uses the heap.
fn has_state(pf: &ProgramCfgFabric) -> bool {
    pf.blocks.iter().any(|b| {
        b.body.iter().any(|c| {
            matches!(
                c,
                LoopCell::HeapRead { .. } | LoopCell::HeapWrite { .. } | LoopCell::HeapAlloc { .. }
            )
        })
    })
}

/// Call `name(args)` on the VM and return its integer result — the ground-truth oracle.
fn vm_call(src: &str, name: &str, args: &[i64]) -> i64 {
    let arg = |n: i64| {
        if n < 0 {
            format!("(0 - {})", -n)
        } else {
            n.to_string()
        }
    };
    let arglist = args.iter().map(|a| arg(*a)).collect::<Vec<_>>().join(", ");
    let prog = format!("{src}## Main\n    Show {name}({arglist}).\n");
    let out = vm_outcome(&prog);
    assert!(
        out.error.is_none(),
        "VM error calling {name}{args:?}: {:?}",
        out.error
    );
    out.output
        .split_whitespace()
        .next()
        .expect("a Show")
        .parse::<i64>()
        .expect("int output")
}

#[test]
fn the_state_plane_sequencer_equals_the_grid_walk_equals_the_vm() {
    for f in CORPUS {
        let pf = program_cfg_fabric(f.src, f.name);
        assert!(
            pf.blocks.len() >= 2,
            "{}: must be a genuine MULTI-block CFG (an early-return branch), got {}",
            f.name,
            pf.blocks.len()
        );
        assert!(
            has_state(&pf),
            "{}: must carry a state-plane (heap) tile — else it is not a state-plane program",
            f.name
        );
        for args in f.args {
            let (seq, steps) = eval_sequencer(&pf, args);
            let grid = run_program_cfg_on_grid(&pf, args);
            let model = eval_program_cfg_fabric(&pf, args);
            let vm = vm_call(f.src, f.name, args);
            assert_eq!(
                seq, grid,
                "{}: eval_sequencer {seq} != grid walk {grid} on {args:?}",
                f.name
            );
            assert_eq!(
                seq, model,
                "{}: eval_sequencer {seq} != CFG model {model} on {args:?}",
                f.name
            );
            assert_eq!(
                seq, vm,
                "{}: eval_sequencer {seq} != VM {vm} on {args:?}",
                f.name
            );
            assert!(
                steps >= 2,
                "{}: a multi-block run takes ≥2 block steps on {args:?}",
                f.name
            );
        }
    }
    println!(
        "Stage 2a: {} MULTI-block STATEFUL programs run on the sequencer's shared-heap block walk (collection allocated in one block, read/mutated in others) ≡ the CFG model ≡ the VM",
        CORPUS.len()
    );
}
