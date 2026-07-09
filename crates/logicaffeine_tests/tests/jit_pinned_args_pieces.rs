//! WS-A Lever A — machine-code PIECE-COUNT evidence. The copy-and-patch JIT
//! lowers exactly one stencil piece per `MicroOp` (`compile_straightline_coded`,
//! "piece index == op index"), and on this dispatch/piece-bound engine the
//! dominant per-call cost is the per-piece ABI threading, NOT memory traffic
//! (memory `jit-is-dispatch-bound`). Lever A replaces the `arg_count` separate
//! frame-to-frame `Move` pieces that stage a scalar self-call with ONE fused
//! `CallSelfCopy` piece that copies the argument block in-stencil. This test
//! compiles both lowerings of the SAME 3-argument self-call and asserts the
//! fused chain carries exactly `arg_count` (= 3) fewer pieces — the direct,
//! disassembly-level proof that the staging `Move`s are gone.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_forge::jit::{compile_straightline_coded, MicroOp};

/// A shared status cell address (any live i64 works for a compile-only test;
/// the chains are never run here).
fn status() -> std::sync::Arc<std::sync::atomic::AtomicI64> {
    std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0))
}

#[test]
fn fused_self_call_removes_one_piece_per_argument() {
    let depth = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0));
    let depth_addr = depth.as_ref() as *const std::sync::atomic::AtomicI64 as i64;
    let st = status();
    let status_addr = st.as_ref() as *const std::sync::atomic::AtomicI64 as i64;

    let arg_count: u16 = 3;
    // Frame layout mirroring the function tier (mode A): registers below, then
    // conv ×2, then the arena-limit slot; the callee window is `limit_slot + 1`.
    let limit_slot: u16 = 10;
    let window: u16 = limit_slot + 1;
    let frame_size = (limit_slot as i64) + 1;
    let src_start: u16 = 0;

    // BEFORE Lever A: `arg_count` Move pieces stage the window, then the
    // self-call piece — `arg_count + 1` pieces plus the trailing Return.
    let mut staged: Vec<MicroOp> = (0..arg_count)
        .map(|j| MicroOp::Move { dst: window + j, src: src_start + j })
        .collect();
    staged.push(MicroOp::CallSelf {
        dst: 5,
        args_start: window,
        depth_addr,
        status_addr,
        limit_slot,
        frame_size,
    });
    staged.push(MicroOp::Return { src: 5 });

    // AFTER Lever A: ONE fused piece does the copy and the call, then Return.
    let fused = vec![
        MicroOp::CallSelfCopy {
            dst: 5,
            args_start: window,
            src_start,
            arg_count,
            depth_addr,
            status_addr,
            limit_slot,
            frame_size,
        },
        MicroOp::Return { src: 5 },
    ];

    let staged_chain = compile_straightline_coded(&staged, Some(status()), None, depth_addr)
        .expect("staged lowering compiles");
    let fused_chain = compile_straightline_coded(&fused, Some(status()), None, depth_addr)
        .expect("fused lowering compiles");

    // The straightline lowering emits one piece per op (plus the shared
    // side-exit terminal, common to both), so the piece-count delta is exactly
    // the eliminated `Move` staging.
    assert_eq!(
        staged_chain.piece_count() - fused_chain.piece_count(),
        arg_count as usize,
        "the fused self-call must remove exactly one machine-code piece per \
         staged argument (staged={}, fused={})",
        staged_chain.piece_count(),
        fused_chain.piece_count(),
    );
}
