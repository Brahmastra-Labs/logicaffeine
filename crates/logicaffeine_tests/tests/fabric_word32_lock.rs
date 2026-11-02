//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! WORD32 (ℤ/2³²) LANE ON THE FABRIC — the pit-of-success ratchet for SHA-256-style bit mixing lowering to
//! the single-cycle combinational loop fabric, on EVERY loop shape.
//!
//! The Word32 builtins (`word32`/`intOfWord32`/`word_and`/`word_or`/`word_not`/`rotl`/`rotr`/`word32Shr`)
//! were `exec_ops`-unsupported; now each lowers to combinational `Arith` tiles, self-masking to 32 bits.
//! Two kernels exercise the lane: SHA-256's σ0 (`rotr ^ rotr ^ shr`) and a bitwise/rotate chain
//! (`and`/`or`/`not`/`rotl`). Each MUST lower on counted / while / nested loops, use a 32-bit-mask `And`
//! tile in the body, and evaluate ≡ VM. COVERAGE RATCHET: the (kernel × shape) grid must be complete.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_sva::fabric_rtl::{
    eval_loop_fabric, eval_nested_fabric, eval_while_fabric, loop_fabric_from_source,
    nested_fabric_from_source, while_fabric_from_source, AluOp, LoopCell,
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

/// A word32 kernel: `key`, the loop-body statement(s) over the int accumulator `s`, and the accumulator init.
struct Kernel {
    key: &'static str,
    body: &'static str,
    s_init: &'static str,
}

const KERNELS: &[Kernel] = &[
    // SHA-256 σ0: rotr(x,7) ^ rotr(x,18) ^ (x >> 3), applied to the accumulator each iteration.
    Kernel {
        key: "sigma0",
        body: "Let w be word32(s).\nSet s to intOfWord32(rotr(w, 7) xor rotr(w, 18) xor word32Shr(w, 3)).",
        s_init: "305419896",
    },
    // A bitwise + rotate chain: and / or / not / rotl (the other half of the Word32 vocabulary).
    Kernel {
        key: "bitchain",
        body: "Let w be word32(s).\nLet a be word_and(w, word32(4042322160)).\nLet b be word_or(a, word32(252645135)).\nLet c be word_not(b).\nSet s to intOfWord32(rotl(c, 5)).",
        s_init: "305419896",
    },
    // A SHA-256-style round: the WRAPPING add `w + σ0(w)` mod 2³² — the `+` on two word32 values must mask
    // to 32 bits (needs word32 register tracking, else the fabric's unmasked add diverges from the VM).
    Kernel {
        key: "round",
        body: "Let w be word32(s).\nLet mixed be word32(rotr(w, 7) xor rotr(w, 18) xor word32Shr(w, 3)).\nSet s to intOfWord32(w + mixed).",
        s_init: "305419896",
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

#[test]
fn word32_bit_mixing_lowers_on_every_loop_shape_matching_the_vm() {
    std::panic::set_hook(Box::new(|_| {}));

    let declared: BTreeSet<(&str, &str)> = SHAPES
        .iter()
        .flat_map(|s| KERNELS.iter().map(move |k| (s.name(), k.key)))
        .collect();
    let inputs: &[i64] = &[1, 2, 5];
    let mut realized: BTreeSet<(&str, &str)> = BTreeSet::new();
    let mut failures: Vec<String> = Vec::new();

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
            // A word32 op self-masks to 32 bits → the body must contain an `And` tile (the `& 0xFFFF_FFFF`).
            let has_mask = body
                .iter()
                .any(|c| matches!(c, LoopCell::Arith { op: AluOp::And, .. }));
            if !has_mask {
                failures.push(format!(
                    "{fname} [{}]: no 32-bit-mask And tile — word32 masking absent",
                    shape.name()
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
        eprintln!("=== word32-fabric failures ({}) ===", failures.len());
        for f in &failures {
            eprintln!("  {f}");
        }
    }
    assert!(
        failures.is_empty(),
        "word32 lane: {} cell(s) failed (see stderr)",
        failures.len()
    );
    assert_eq!(
        realized,
        declared,
        "word32 coverage gap: {:?}",
        declared.difference(&realized).collect::<Vec<_>>()
    );

    println!(
        "word32-fabric: {} cells ({} kernels × {} loop shapes) — SHA-256 bit mixing lowers to the fabric ≡ VM",
        declared.len(),
        KERNELS.len(),
        SHAPES.len()
    );
}
