//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! WORD64 (ℤ/2⁶⁴) LANE ON THE FABRIC — the pit-of-success ratchet for SHA-512 / Keccak 64-bit bit mixing
//! lowering to the single-cycle combinational loop fabric, on EVERY loop shape.
//!
//! The Word64 builtins (`word64`/`intOfWord64`/`word64And`/`word64Shl`/`word64Shr`) lower to combinational
//! tiles. Unlike the Word32 lane, a Word64 value rides the FULL i64 bit pattern (`intOfWord64` is a pure
//! `to_u64() as i64` reinterpretation), so `word64`/`intOfWord64` are IDENTITY and wrapping `+`/`-`/`×` need
//! NO mask (native i64 arithmetic already wraps mod 2⁶⁴). The one op that is NOT arithmetic-identity is the
//! LOGICAL right shift `word64Shr`, which MUST use the zero-fill `AluOp::UShr` — the arithmetic `AluOp::Shr`
//! would sign-extend a bit-63-set value and silently diverge from the VM's `u64` `wrapping_shr`.
//!
//! Three kernels exercise the lane: SHA-512's σ0 (`rotr ^ rotr ^ shr`), a GUARANTEED-bit-63 logical shift
//! (the UShr witness — it diverges from an arithmetic shift, so it proves the zero-fill is both used AND
//! correct), and a wrapping add mod 2⁶⁴ (native i64 overflow, NO mask). Each MUST lower on counted / while /
//! nested loops, carry its expected tile, and evaluate ≡ VM. GLOBAL invariants: (a) a logical `UShr` tile is
//! realized somewhere, and (b) NO body carries the 32-bit `0xFFFFFFFF` mask (Word64 is not Word32).
//! COVERAGE RATCHET: the (kernel × shape) grid must be complete. Extend it; never delete a cell.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_sva::fabric_rtl::{
    eval_loop_fabric, eval_nested_fabric, eval_while_fabric, loop_fabric_from_source,
    nested_fabric_from_source, while_fabric_from_source, AluOp, LoopCell, Src,
};
use logicaffeine_compile::compile::vm_outcome;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Shape {
    Counted,
    While,
    Nested,
}
impl Shape {
    fn name(self) -> &'static str {
        match self {
            Shape::Counted => "counted",
            Shape::While => "while",
            Shape::Nested => "nested",
        }
    }
}

fn alu_name(op: AluOp) -> &'static str {
    match op {
        AluOp::Add => "Add",
        AluOp::Sub => "Sub",
        AluOp::Mul => "Mul",
        AluOp::And => "And",
        AluOp::Or => "Or",
        AluOp::Xor => "Xor",
        AluOp::Shl => "Shl",
        AluOp::Shr => "Shr",
        AluOp::UShr => "UShr",
    }
}

/// A word64 kernel: `key`, the loop-body statement(s) over the int accumulator `s`, the accumulator init,
/// and the ALU tile the body MUST contain (the lane fingerprint).
struct Kernel {
    key: &'static str,
    body: &'static str,
    s_init: &'static str,
    tile: &'static str,
}

const KERNELS: &[Kernel] = &[
    // SHA-512 σ0: rotr64(x,1) ^ rotr64(x,8) ^ (x >>ᵤ 7). Each rotate = a LOGICAL right shift OR a left shift;
    // the two halves cover DISJOINT bit ranges, so `xor` == `or` there (Word64 has no `word64Or` builtin).
    Kernel {
        key: "sigma512",
        body: "Let w be word64(s).\nSet s to intOfWord64((word64Shr(w, 1) xor word64Shl(w, 63)) xor (word64Shr(w, 8) xor word64Shl(w, 56)) xor word64Shr(w, 7)).",
        s_init: "1311768467463790320",
        tile: "UShr",
    },
    // The UShr WITNESS: `word64And` with 0x5555…5555 leaves bit 62 set, `word64Shl(_, 1)` moves it to bit 63,
    // then `word64Shr(_, 4)` LOGICAL-shifts a value whose high bit IS set. An arithmetic shift would sign-extend
    // (a wrong, negative result); this kernel diverges unless the zero-fill `UShr` tile is used — and ≡ VM proves
    // the zero-fill is the correct one.
    Kernel {
        key: "hishift",
        body: "Let w be word64(s).\nLet a be word64And(w, word64(6148914691236517205)).\nLet b be word64Shl(a, 1).\nSet s to intOfWord64(word64Shr(b, 4)).",
        s_init: "1311768467463790320",
        tile: "UShr",
    },
    // Wrapping add mod 2⁶⁴: `w + big + big` with `big` ≈ 7×10¹⁸ overflows i64 on the first add — native i64
    // `wrapping_add` IS the ℤ/2⁶⁴ ring, so NO mask is emitted (unlike the Word32 lane). ≡ VM over the wrap proves it.
    Kernel {
        key: "wrapadd",
        body: "Let w be word64(s).\nLet big be word64(7000000000000000000).\nSet s to intOfWord64(w + big + big).",
        s_init: "9000000000000000000",
        tile: "Add",
    },
];

const SHAPES: &[Shape] = &[Shape::Counted, Shape::While, Shape::Nested];

fn kernel_src(shape: Shape, k: &Kernel) -> (String, String) {
    let tag = match shape {
        Shape::Counted => "c",
        Shape::While => "w",
        Shape::Nested => "n",
    };
    let fname = format!("{}{}", k.key, tag);
    let base = if matches!(shape, Shape::Nested) {
        "            "
    } else {
        "        "
    };
    let ibody: String = k
        .body
        .lines()
        .map(|l| format!("{base}{l}"))
        .collect::<Vec<_>>()
        .join("\n");
    let src = match shape {
        Shape::Counted => format!(
            "## To {fname} (n: Int) -> Int:\n    Let mutable s be {}.\n    Repeat for i from 1 to n:\n{ibody}\n    Return s.\n",
            k.s_init
        ),
        Shape::While => format!(
            "## To {fname} (n: Int) -> Int:\n    Let mutable s be {}.\n    Let mutable i be 0.\n    While i is less than n:\n{ibody}\n        Set i to i + 1.\n    Return s.\n",
            k.s_init
        ),
        Shape::Nested => format!(
            "## To {fname} (n: Int) -> Int:\n    Let mutable s be {}.\n    Repeat for i from 1 to 2:\n        Repeat for j from 1 to n:\n{ibody}\n    Return s.\n",
            k.s_init
        ),
    };
    (fname, src)
}

fn lower(shape: Shape, fname: &str, src: &str, inputs: &[i64]) -> (Vec<LoopCell>, Vec<i64>) {
    match shape {
        Shape::Counted => {
            let lf = loop_fabric_from_source(src, fname);
            (
                lf.body.clone(),
                inputs
                    .iter()
                    .map(|&x| eval_loop_fabric(&lf, &[x]))
                    .collect(),
            )
        }
        Shape::While => {
            let wf = while_fabric_from_source(src, fname);
            (
                wf.body.clone(),
                inputs
                    .iter()
                    .map(|&x| eval_while_fabric(&wf, &[x]))
                    .collect(),
            )
        }
        Shape::Nested => {
            let nf = nested_fabric_from_source(src, fname);
            (
                nf.inner_body.clone(),
                inputs
                    .iter()
                    .map(|&x| eval_nested_fabric(&nf, &[x]))
                    .collect(),
            )
        }
    }
}

fn vm_int(src: &str, fname: &str, x: i64) -> String {
    let full = format!("{src}## Main\n    Show {fname}({x}).\n");
    let out = vm_outcome(&full);
    assert!(
        out.error.is_none(),
        "{fname}({x}): VM error {:?}",
        out.error
    );
    out.output.trim().to_string()
}

fn body_has_alu(body: &[LoopCell], name: &str) -> bool {
    body.iter()
        .any(|c| matches!(c, LoopCell::Arith { op, .. } if alu_name(*op) == name))
}

/// True iff the body masks to 32 bits (`& 0xFFFFFFFF`) — the Word32 fingerprint, which Word64 must NEVER emit.
fn body_masks_32(body: &[LoopCell]) -> bool {
    body.iter().any(|c| {
        matches!(
            c,
            LoopCell::Arith {
                op: AluOp::And,
                b: Src::Const(0xFFFF_FFFF),
                ..
            }
        )
    })
}

#[test]
fn word64_bit_mixing_lowers_on_every_loop_shape_matching_the_vm() {
    std::panic::set_hook(Box::new(|_| {}));

    let declared: BTreeSet<(&str, &str)> = SHAPES
        .iter()
        .flat_map(|s| KERNELS.iter().map(move |k| (s.name(), k.key)))
        .collect();
    let inputs: &[i64] = &[1, 2, 5];
    let mut realized: BTreeSet<(&str, &str)> = BTreeSet::new();
    let mut failures: Vec<String> = Vec::new();
    let mut saw_ushr = false;

    for &shape in SHAPES {
        for k in KERNELS {
            let (fname, src) = kernel_src(shape, k);
            let lowered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                lower(shape, &fname, &src, inputs)
            }));
            let (body, results) = match lowered {
                Ok(x) => x,
                Err(e) => {
                    let m = e
                        .downcast_ref::<String>()
                        .map(|s| s.as_str())
                        .or_else(|| e.downcast_ref::<&str>().copied())
                        .unwrap_or("panic");
                    failures.push(format!(
                        "{fname} [{}]: did not lower: {}",
                        shape.name(),
                        &m[..m.len().min(70)]
                    ));
                    continue;
                }
            };
            saw_ushr |= body_has_alu(&body, "UShr");
            // Word64 is a 64-bit ring — it must NEVER emit the Word32 `& 0xFFFFFFFF` mask.
            if body_masks_32(&body) {
                failures.push(format!(
                    "{fname} [{}]: emitted a 32-bit mask — Word64 must not mask to 32 bits",
                    shape.name()
                ));
                continue;
            }
            if !body_has_alu(&body, k.tile) {
                failures.push(format!(
                    "{fname} [{}]: body has NO expected `{}` tile",
                    shape.name(),
                    k.tile
                ));
                continue;
            }
            let mut ok = true;
            for (&x, &got) in inputs.iter().zip(results.iter()) {
                let vm = vm_int(&src, &fname, x);
                if got.to_string() != vm {
                    failures.push(format!(
                        "{fname}({x}) [{}]: fabric {got} != VM {vm}",
                        shape.name()
                    ));
                    ok = false;
                    break;
                }
            }
            if ok {
                realized.insert((shape.name(), k.key));
            }
        }
    }

    if !failures.is_empty() {
        eprintln!("=== word64-fabric failures ({}) ===", failures.len());
        for f in &failures {
            eprintln!("  {f}");
        }
    }
    assert!(
        failures.is_empty(),
        "word64 lane: {} cell(s) failed (see stderr)",
        failures.len()
    );
    assert!(saw_ushr, "word64 lane: no LOGICAL-shift (UShr) tile was realized — the zero-fill primitive is missing");
    assert_eq!(
        realized,
        declared,
        "word64 coverage gap: {:?}",
        declared.difference(&realized).collect::<Vec<_>>()
    );

    println!(
        "word64-fabric: {} cells ({} kernels × {} loop shapes) — SHA-512 / ℤ/2⁶⁴ bit mixing lowers to the fabric ≡ VM",
        declared.len(),
        KERNELS.len(),
        SHAPES.len()
    );
}
