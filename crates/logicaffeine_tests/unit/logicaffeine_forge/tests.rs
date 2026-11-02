use super::*;

/// Machine code for `extern "C" fn(i64, i64) -> i64 { a.wrapping_add(b) }`
/// on aarch64: `add x0, x0, x1` ; `ret`.
#[cfg(target_arch = "aarch64")]
const ADD_CODE: [u8; 8] = [0x00, 0x00, 0x01, 0x8b, 0xc0, 0x03, 0x5f, 0xd6];

/// Same on x86-64 (SysV ABI: a=rdi, b=rsi, ret=rax):
/// `lea rax, [rdi+rsi]` (48 8D 04 37) ; `ret` (C3).
#[cfg(target_arch = "x86_64")]
const ADD_CODE: [u8; 5] = [0x48, 0x8d, 0x04, 0x37, 0xc3];

#[test]
fn jit_add_3_5_equals_8() {
    let page = JitPage::new(&ADD_CODE).expect("could not allocate executable page");
    let add = unsafe { page.as_fn_i64_i64() };
    assert_eq!(add(3, 5), 8);
    assert_eq!(add(100, -1), 99);
    assert_eq!(add(0, 0), 0);
    // The hardware ADD wraps (matching the interpreter's wrapping arithmetic).
    assert_eq!(add(i64::MAX, 1), i64::MIN);
}

#[test]
fn jit_page_reports_lengths_and_alignment() {
    let page = JitPage::new(&ADD_CODE).unwrap();
    assert_eq!(page.len(), ADD_CODE.len());
    assert!(!page.is_empty());
    assert!(!page.as_ptr().is_null());
    // The mapping is page-aligned and page-granular.
    let ps = super::page_size();
    assert_eq!(page.alloc_len() % ps, 0);
    assert!(page.alloc_len() >= page.len());
    assert_eq!(page.as_ptr() as usize % ps, 0);
}

#[test]
fn jit_page_empty_code_is_error() {
    match JitPage::new(&[]) {
        Err(JitError::EmptyCode) => {}
        other => panic!("expected EmptyCode error, got {:?}", other.map(|p| p.len())),
    }
}

#[test]
fn jit_page_built_on_one_thread_executes_on_another() {
    // Send + cross-thread execution: validates the icache flush and the
    // claim that W^X toggling is confined to the constructing thread.
    let page = JitPage::new(&ADD_CODE).unwrap();
    let handle = std::thread::spawn(move || {
        let add = unsafe { page.as_fn_i64_i64() };
        add(20, 22)
    });
    assert_eq!(handle.join().unwrap(), 42);
}

#[test]
fn jit_many_threads_create_write_execute_drop() {
    // W^X per-thread toggling must not interfere across threads.
    let handles: Vec<_> = (0..8)
        .map(|k| {
            std::thread::spawn(move || {
                for _ in 0..200 {
                    let page = JitPage::new(&ADD_CODE).unwrap();
                    let add = unsafe { page.as_fn_i64_i64() };
                    assert_eq!(add(k, 1), k + 1);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn jit_create_and_drop_many_pages() {
    // munmap correctness smoke: leaking mappings would exhaust the address
    // space quota long before 1000 iterations of page-sized maps.
    for _ in 0..1000 {
        let page = JitPage::new(&ADD_CODE).unwrap();
        let add = unsafe { page.as_fn_i64_i64() };
        assert_eq!(add(1, 2), 3);
    }
}

// ---- F2: stencil table introspection ------------------------------------

fn stencil(name: &str) -> &'static Stencil {
    super::STENCILS
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("stencil '{name}' missing from table"))
}

#[test]
fn stencil_table_contains_the_full_cps_set() {
    for name in [
        "logos_stencil_const",
        "logos_stencil_addi",
        "logos_stencil_subi",
        "logos_stencil_muli",
        "logos_stencil_lti",
        "logos_stencil_branch_if",
        "logos_stencil_return",
        "logos_stencil_add",
    ] {
        let st = stencil(name);
        assert!(!st.code.is_empty(), "{name} has no code");
    }
}

#[test]
fn checked_div_mod_stencils_have_two_distinct_cont_holes() {
    for name in ["logos_stencil_divi_checked", "logos_stencil_modi_checked"] {
        let st = stencil(name);
        let conts: std::collections::HashSet<_> = st
            .relocs
            .iter()
            .filter_map(|r| match r.target {
                HoleId::Cont(n) => Some(n),
                _ => None,
            })
            .collect();
        assert_eq!(
            conts,
            [0u8, 1u8].into_iter().collect(),
            "{name} must expose both the success and side-exit continuations"
        );
    }
}

#[test]
fn deopt_stencil_has_const_hole_and_no_continuations() {
    let st = stencil("logos_stencil_deopt");
    assert!(
        st.relocs
            .iter()
            .any(|r| matches!(r.target, HoleId::ConstI64(0))),
        "deopt stencil lacks its status-cell address hole: {:?}",
        st.relocs
    );
    assert!(
        !st.relocs
            .iter()
            .any(|r| matches!(r.target, HoleId::Cont(_))),
        "deopt is a terminal — it must not continue anywhere"
    );
}

#[test]
fn checked_div_chain_computes_and_side_exits() {
    use crate::jit::{compile_straightline, ChainOutcome, MicroOp};
    // frame[2] = frame[0] / frame[1]; return frame[2].
    let prog = [
        MicroOp::Div {
            dst: 2,
            lhs: 0,
            rhs: 1,
        },
        MicroOp::Return { src: 2 },
    ];
    let chain = compile_straightline(&prog).expect("compile");
    let mut frame = [40i64, 5, 0];
    assert_eq!(chain.run_with_frame(&mut frame), ChainOutcome::Return(8));
    // Exact arithmetic: the overflowing quotient i64::MIN / -1 side-exits
    // (deopt → the promoting tiers), never wraps.
    let mut frame = [i64::MIN, -1, 0];
    assert_eq!(chain.run_with_frame(&mut frame), ChainOutcome::Deopt(1));
    // Zero divisor: side exit, then the NEXT run is clean (cell resets).
    let mut frame = [40i64, 0, 0];
    assert_eq!(chain.run_with_frame(&mut frame), ChainOutcome::Deopt(1));
    let mut frame = [40i64, 4, 0];
    assert_eq!(chain.run_with_frame(&mut frame), ChainOutcome::Return(10));
}

#[test]
fn checked_mod_chain_computes_and_side_exits() {
    use crate::jit::{compile_straightline, ChainOutcome, MicroOp};
    let prog = [
        MicroOp::Mod {
            dst: 2,
            lhs: 0,
            rhs: 1,
        },
        MicroOp::Return { src: 2 },
    ];
    let chain = compile_straightline(&prog).expect("compile");
    let mut frame = [43i64, 5, 0];
    assert_eq!(chain.run_with_frame(&mut frame), ChainOutcome::Return(3));
    let mut frame = [i64::MIN, -1, 0];
    assert_eq!(chain.run_with_frame(&mut frame), ChainOutcome::Return(0));
    let mut frame = [-7i64, 2, 0];
    assert_eq!(chain.run_with_frame(&mut frame), ChainOutcome::Return(-1));
    let mut frame = [43i64, 0, 0];
    assert_eq!(chain.run_with_frame(&mut frame), ChainOutcome::Deopt(1));
}

/// A float `Move` whose SOURCE and DESTINATION are both XMM-pinned must move
/// the value REGISTER-to-REGISTER (V_FMOV f→f), not round-trip a stale frame
/// cell. The pinned-float emitter used the GP location (`loc`) for `Move`, so
/// a float move between pins wrote/read frame bits that the register-resident
/// value had never been spilled to — corrupting the loop-carried accumulator
/// (the `Set zr to zr2` step of mandelbrot). Reproduces `x = x*x` over a pin:
/// without the fix `x` keeps its prologue value (3.0) instead of 9.0.
#[test]
fn float_move_between_xmm_pins_threads_the_register() {
    use crate::jit::{compile_straightline_pinned_float, ChainOutcome, MicroOp};
    let prog = [
        MicroOp::MulF {
            dst: 2,
            lhs: 0,
            rhs: 0,
        }, // sq = x * x   (f-reg threaded)
        MicroOp::Move { dst: 0, src: 2 }, // x = sq       (f→f move)
        MicroOp::Return { src: 0 },
    ];
    let chain = compile_straightline_pinned_float(&prog, &[], &[0, 2], None).expect("compile");
    let mut frame = [3.0f64.to_bits() as i64, 0, 0];
    assert_eq!(
        chain.run_with_frame(&mut frame),
        ChainOutcome::Return(9.0f64.to_bits() as i64),
        "Move between XMM pins must copy the register value (9.0), not a stale frame cell"
    );
}

/// A float `Move` from a FRAME slot into an XMM pin (the common `Set acc to
/// temp` shape where `temp` is unpinned) must reload the frame bits into the
/// pinned register — otherwise the epilogue spills the pin's stale prologue
/// value back over the result.
#[test]
fn float_move_frame_into_xmm_pin_reloads() {
    use crate::jit::{compile_straightline_pinned_float, ChainOutcome, MicroOp};
    let prog = [
        MicroOp::Move { dst: 0, src: 2 }, // x(pin) = temp(frame)
        MicroOp::Return { src: 0 },
    ];
    let chain = compile_straightline_pinned_float(&prog, &[], &[0], None).expect("compile");
    let mut frame = [1.0f64.to_bits() as i64, 0, 7.5f64.to_bits() as i64];
    assert_eq!(
        chain.run_with_frame(&mut frame),
        ChainOutcome::Return(7.5f64.to_bits() as i64),
        "Move from frame into an XMM pin must load 7.5 into the pin"
    );
}

#[test]
fn return_stencil_has_zero_relocs() {
    assert!(stencil("logos_stencil_return").relocs.is_empty());
    assert!(stencil("logos_stencil_add").relocs.is_empty());
}

#[test]
fn binop_stencils_have_exactly_one_cont_hole() {
    for name in [
        "logos_stencil_addi",
        "logos_stencil_subi",
        "logos_stencil_muli",
        "logos_stencil_lti",
    ] {
        let st = stencil(name);
        let conts: Vec<_> = st
            .relocs
            .iter()
            .filter(|r| matches!(r.target, HoleId::Cont(0)))
            .collect();
        assert_eq!(
            conts.len(),
            1,
            "{name}: expected exactly one Cont(0) reloc, got {:?}",
            st.relocs
        );
    }
}

#[test]
fn const_stencil_has_const_hole_and_cont_hole() {
    let st = stencil("logos_stencil_const");
    assert!(
        st.relocs
            .iter()
            .any(|r| matches!(r.target, HoleId::ConstI64(0))),
        "const stencil lacks its ConstI64 hole: {:?}",
        st.relocs
    );
    assert!(
        st.relocs
            .iter()
            .any(|r| matches!(r.target, HoleId::Cont(0))),
        "const stencil lacks its continuation: {:?}",
        st.relocs
    );
}

#[test]
fn branch_if_has_two_distinct_cont_holes() {
    let st = stencil("logos_stencil_branch_if");
    let has0 = st
        .relocs
        .iter()
        .any(|r| matches!(r.target, HoleId::Cont(0)));
    let has1 = st
        .relocs
        .iter()
        .any(|r| matches!(r.target, HoleId::Cont(1)));
    assert!(
        has0 && has1,
        "branch_if needs Cont(0) and Cont(1): {:?}",
        st.relocs
    );
}

#[test]
fn every_reloc_is_in_bounds_and_aligned() {
    for st in super::STENCILS {
        for r in st.relocs {
            assert!(
                (r.offset as usize) < st.code.len(),
                "{}: reloc out of bounds",
                st.name
            );
            #[cfg(target_arch = "aarch64")]
            assert_eq!(r.offset % 4, 0, "{}: arm64 reloc misaligned", st.name);
        }
    }
}

/// The headline: a stencil written in Rust, compiled to an object at build
/// time, its machine code extracted, then JIT-loaded and executed.
#[test]
fn jit_extracted_rust_stencil_runs() {
    assert!(
        !super::ADD_STENCIL.is_empty(),
        "stencil extraction produced no bytes"
    );
    let page = JitPage::new(super::ADD_STENCIL).expect("load extracted stencil");
    let add = unsafe { page.as_fn_i64_i64() };
    assert_eq!(add(3, 5), 8);
    assert_eq!(add(40, 2), 42);
    assert_eq!(add(-10, 7), -3);
}

/// The extracted stencil must be a relocation-free leaf — small, and ending
/// in `ret` — so it can be copied verbatim into a JIT page. (The exact `add`
/// encoding is rustc's choice: `add` is commutative, so `x0+x1` and `x1+x0`
/// are both valid; we assert the structural property, not the bytes.)
#[test]
fn extracted_add_is_a_relocation_free_leaf() {
    #[cfg(target_arch = "aarch64")]
    {
        let bytes = super::ADD_STENCIL;
        // An `add x0, _, _` ; `ret` leaf is two 4-byte instructions.
        assert!(
            bytes.len() <= 16,
            "leaf stencil unexpectedly large: {} bytes",
            bytes.len()
        );
        assert!(
            bytes.ends_with(&[0xc0, 0x03, 0x5f, 0xd6]),
            "stencil does not end in `ret`: {:02x?}",
            bytes
        );
        // First instruction is in the ADD (shifted register, 64-bit) family:
        // top byte 0x8b.
        assert_eq!(bytes[3], 0x8b, "first instruction is not a 64-bit ADD");
    }
}
