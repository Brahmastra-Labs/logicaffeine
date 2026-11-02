//! Runtime helper calls preserve frame operands, live registers, and precise exits.
#![cfg(all(target_arch = "x86_64", not(target_arch = "wasm32")))]

use logicaffeine_forge::jit::{ChainOutcome, MicroOp};
use logicaffeine_forge::regalloc::compile_function_regalloc_precise;
use std::sync::{atomic::AtomicI64, Arc};

unsafe extern "C" fn helper(frame: *mut i64, dst: i64, src: i64, count: i64, imm: i64) -> i64 {
    let accepted = imm >= 0;
    if accepted {
        let mut sum = imm;
        for i in 0..count {
            sum += *frame.add((src + i) as usize);
        }
        *frame.add(dst as usize) = sum;
    }
    // Deliberately exercise the full caller-saved register contract.
    core::arch::asm!(
        "xor rax, rax",
        "xor rcx, rcx",
        "xor rdx, rdx",
        "xor rsi, rsi",
        "xor rdi, rdi",
        "xor r8, r8",
        "xor r9, r9",
        "xor r10, r10",
        "xor r11, r11",
        "pxor xmm0, xmm0",
        "pxor xmm1, xmm1",
        "pxor xmm2, xmm2",
        "pxor xmm3, xmm3",
        "pxor xmm4, xmm4",
        "pxor xmm5, xmm5",
        "pxor xmm6, xmm6",
        "pxor xmm7, xmm7",
        "pxor xmm8, xmm8",
        "pxor xmm9, xmm9",
        "pxor xmm10, xmm10",
        "pxor xmm11, xmm11",
        "pxor xmm12, xmm12",
        "pxor xmm13, xmm13",
        "pxor xmm14, xmm14",
        "pxor xmm15, xmm15",
        clobber_abi("C"),
        options(nostack)
    );
    i64::from(accepted)
}

fn run(imm: i64, alias: bool) -> (ChainOutcome, Vec<i64>) {
    let mut ops = vec![MicroOp::LoadConst { dst: 26, value: -1 }];
    for slot in 0..8 {
        ops.push(MicroOp::LoadConst {
            dst: slot,
            value: slot as i64 + 1,
        });
    }
    for slot in 8..12 {
        ops.push(MicroOp::LoadConst {
            dst: slot,
            value: (slot as f64).to_bits() as i64,
        });
        ops.push(MicroOp::AddF {
            dst: slot,
            lhs: slot,
            rhs: slot,
        });
    }
    // Rank helper inputs highly enough that omitting their frame residency
    // exposes stale frame reads, even under a register-pressure heuristic.
    ops.push(MicroOp::LoadConst { dst: 23, value: 0 });
    for _ in 0..32 {
        ops.push(MicroOp::Add {
            dst: 0,
            lhs: 0,
            rhs: 23,
        });
        ops.push(MicroOp::Add {
            dst: 1,
            lhs: 1,
            rhs: 23,
        });
    }
    // A value dead on the successful path still belongs to a precise exit.
    ops.push(MicroOp::LoadConst {
        dst: 13,
        value: 999,
    });
    let dst = if alias { 0 } else { 12 };
    let host = ops.len();
    ops.push(MicroOp::Host {
        dst,
        src: 0,
        count: 2,
        imm,
        helper_addr: helper as usize as i64,
    });
    // Keep unrelated GP and XMM values live across the host call.
    for slot in 2..8 {
        ops.push(MicroOp::Add {
            dst: slot,
            lhs: slot,
            rhs: slot,
        });
    }
    for slot in 8..12 {
        ops.push(MicroOp::AddF {
            dst: slot,
            lhs: slot,
            rhs: slot,
        });
    }
    ops.push(MicroOp::Return { src: dst });
    let mut codes = vec![1; ops.len()];
    codes[host] = 43;
    let status = Arc::new(AtomicI64::new(0));
    let depth = Arc::new(AtomicI64::new(0));
    let chain =
        compile_function_regalloc_precise(&ops, Some(status), Arc::as_ptr(&depth) as i64, &codes)
            .expect("a precise function must support its runtime helper calls");
    let mut frame = vec![0; 64];
    let outcome = chain.run_with_frame(&mut frame);
    (outcome, frame)
}

#[test]
fn host_calls_preserve_live_integer_and_float_registers() {
    let (outcome, frame) = run(39, false);
    assert!(matches!(outcome, ChainOutcome::Return(42)), "{outcome:?}");
    for slot in 2..8 {
        assert_eq!(frame[slot], 2 * (slot as i64 + 1));
    }
    for slot in 8..12 {
        assert_eq!(f64::from_bits(frame[slot] as u64), 4.0 * slot as f64);
    }
}

#[test]
fn host_destination_can_alias_an_input() {
    let (outcome, frame) = run(39, true);
    assert!(matches!(outcome, ChainOutcome::Return(42)), "{outcome:?}");
    assert_eq!(frame[0], 42);
    assert_eq!(frame[1], 2);
}

#[test]
fn rejected_host_calls_exit_at_the_precise_operation() {
    let (outcome, frame) = run(-1, false);
    assert!(matches!(outcome, ChainOutcome::Deopt(43)), "{outcome:?}");
    for slot in 2..8 {
        assert_eq!(frame[slot], slot as i64 + 1);
    }
    for slot in 8..12 {
        assert_eq!(f64::from_bits(frame[slot] as u64), 2.0 * slot as f64);
    }
    assert_eq!(frame[12], 0);
    assert_eq!(frame[13], 999);
}
