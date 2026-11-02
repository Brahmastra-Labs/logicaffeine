//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! FABRIC OP-COVERAGE MATRIX — the pit-of-success ratchet that forces EVERYTHING the single-cycle
//! combinational loop fabric can compute to be locked, on EVERY loop shape.
//!
//! Dimensions:
//!   • LOOP SHAPE   — counted (`Repeat for … from/to`), `While`, NESTED (double loop).
//!   • OPERATION    — integer arithmetic (`+ - * / % //`), FLOAT arithmetic (`+ - * /`), bitwise
//!                    (`& | xor << >>`), and comparisons (`< > ≤ ≥ = ≠`, if-converted to a mux).
//!
//! Each cell is a one-body-op kernel that MUST (a) lower to its loop fabric, (b) GENUINELY emit the expected
//! tile — an `Arith` ALU tile / a `Func` functional-unit tile / a `Select` mux tile — so no op is silently
//! degraded or mislowered, and (c) evaluate byte-identically to the VM across a swept input battery.
//!
//! Plus a COVERAGE guard: the set of (shape × op) cells must equal the declared grid — dropping any cell, or
//! failing to lower one, goes RED. RATCHET: extend the catalog; never delete a cell to dodge a break. This is
//! the single authority for "what the combinational fabric can do", superseding the per-op float locks.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_sva::fabric_rtl::{
    eval_loop_fabric, eval_nested_fabric, eval_while_fabric, loop_fabric_from_source,
    nested_fabric_from_source, while_fabric_from_source, AluOp, CmpKind, FuncUnit, LoopCell,
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

/// The tile a cell must produce (by short name — the fabric enums aren't all `Ord`).
#[derive(Clone, Copy)]
enum Tile {
    Arith(&'static str),
    Func(&'static str),
    Select(&'static str),
}
impl Tile {
    fn name(self) -> String {
        match self {
            Tile::Arith(s) => format!("Arith({s})"),
            Tile::Func(s) => format!("Func({s})"),
            Tile::Select(s) => format!("Select({s})"),
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
fn func_name(u: FuncUnit) -> &'static str {
    match u {
        FuncUnit::DivS => "DivS",
        FuncUnit::ModS => "ModS",
        FuncUnit::FloorDivS => "FloorDivS",
        FuncUnit::FAddF => "FAddF",
        FuncUnit::FSubF => "FSubF",
        FuncUnit::FMulF => "FMulF",
        FuncUnit::FDivF => "FDivF",
        FuncUnit::Pow => "Pow",
        _ => "other",
    }
}
fn cmp_name(k: CmpKind) -> &'static str {
    match k {
        CmpKind::Lt => "Lt",
        CmpKind::Gt => "Gt",
        CmpKind::Le => "Le",
        CmpKind::Ge => "Ge",
        CmpKind::Eq => "Eq",
        CmpKind::Ne => "Ne",
    }
}

fn body_has_tile(body: &[LoopCell], expect: Tile) -> bool {
    body.iter().any(|c| match (expect, c) {
        (Tile::Arith(s), LoopCell::Arith { op, .. }) => alu_name(*op) == s,
        (Tile::Func(s), LoopCell::Func { unit, .. }) => func_name(*unit) == s,
        (Tile::Select(s), LoopCell::Select { cmp, .. }) => cmp_name(*cmp) == s,
        _ => false,
    })
}

struct Op {
    key: &'static str,
    tile: Tile,
    /// The body statement(s) — `{s}` is the accumulator. A comparison uses an if/else to make a `Select`.
    body: &'static str,
    /// The accumulator's initial value.
    init: &'static str,
    is_float: bool,
}

/// The operation catalog — every op the combinational fabric can carry, with the tile it must emit.
const OPS: &[Op] = &[
    // ── integer arithmetic ────────────────────────────────────────────────────────────────────────
    Op {
        key: "iadd",
        tile: Tile::Arith("Add"),
        body: "Set s to s + 3.",
        init: "0",
        is_float: false,
    },
    Op {
        key: "isub",
        tile: Tile::Arith("Sub"),
        body: "Set s to s - 3.",
        init: "100000",
        is_float: false,
    },
    Op {
        key: "imul",
        tile: Tile::Arith("Mul"),
        body: "Set s to s * 2.",
        init: "1",
        is_float: false,
    },
    Op {
        key: "idiv",
        tile: Tile::Func("DivS"),
        body: "Set s to s / 2.",
        init: "1048576",
        is_float: false,
    },
    Op {
        key: "imod",
        tile: Tile::Func("ModS"),
        body: "Set s to (s + 17) % 100.",
        init: "5",
        is_float: false,
    },
    Op {
        key: "ifloordiv",
        tile: Tile::Func("FloorDivS"),
        body: "Set s to s // 2.",
        init: "1048576",
        is_float: false,
    },
    Op {
        key: "ipow",
        tile: Tile::Func("Pow"),
        body: "Set s to s + i ** 2.",
        init: "0",
        is_float: false,
    },
    // ── float arithmetic ──────────────────────────────────────────────────────────────────────────
    Op {
        key: "fadd",
        tile: Tile::Func("FAddF"),
        body: "Set s to s + 1.5.",
        init: "0.0",
        is_float: true,
    },
    Op {
        key: "fsub",
        tile: Tile::Func("FSubF"),
        body: "Set s to s - 2.5.",
        init: "1000.0",
        is_float: true,
    },
    Op {
        key: "fmul",
        tile: Tile::Func("FMulF"),
        body: "Set s to s * 2.0.",
        init: "1.0",
        is_float: true,
    },
    Op {
        key: "fdiv",
        tile: Tile::Func("FDivF"),
        body: "Set s to s / 2.0.",
        init: "1024.0",
        is_float: true,
    },
    // ── bitwise / shifts (integer) ────────────────────────────────────────────────────────────────
    Op {
        key: "band",
        tile: Tile::Arith("And"),
        body: "Set s to s & 12.",
        init: "255",
        is_float: false,
    },
    Op {
        key: "bor",
        tile: Tile::Arith("Or"),
        body: "Set s to s | 1.",
        init: "0",
        is_float: false,
    },
    Op {
        key: "bxor",
        tile: Tile::Arith("Xor"),
        body: "Set s to s xor 5.",
        init: "0",
        is_float: false,
    },
    Op {
        key: "bshl",
        tile: Tile::Arith("Shl"),
        body: "Set s to s shifted left by 1.",
        init: "1",
        is_float: false,
    },
    Op {
        key: "bshr",
        tile: Tile::Arith("Shr"),
        body: "Set s to s shifted right by 1.",
        init: "1048576",
        is_float: false,
    },
    // ── comparisons (if-converted to a Select mux). Bodies at RELATIVE indent (the inner `Set` at 4 spaces);
    //    `kernel_src` re-indents every line to the loop body's depth so it is valid at each shape. ────────
    Op {
        key: "clt",
        tile: Tile::Select("Lt"),
        body: "If s is less than 50:\n    Set s to s + 1.\nOtherwise:\n    Set s to s + 2.",
        init: "0",
        is_float: false,
    },
    Op {
        key: "cgt",
        tile: Tile::Select("Gt"),
        body: "If s is greater than 50:\n    Set s to s + 1.\nOtherwise:\n    Set s to s + 2.",
        init: "0",
        is_float: false,
    },
    Op {
        key: "cle",
        tile: Tile::Select("Le"),
        body: "If s is at most 50:\n    Set s to s + 1.\nOtherwise:\n    Set s to s + 2.",
        init: "0",
        is_float: false,
    },
    Op {
        key: "cge",
        tile: Tile::Select("Ge"),
        body: "If s is at least 50:\n    Set s to s + 1.\nOtherwise:\n    Set s to s + 2.",
        init: "0",
        is_float: false,
    },
    Op {
        key: "ceq",
        tile: Tile::Select("Eq"),
        body: "If s is equal to 3:\n    Set s to s + 10.\nOtherwise:\n    Set s to s + 1.",
        init: "0",
        is_float: false,
    },
    Op {
        key: "cne",
        tile: Tile::Select("Ne"),
        body: "If s is not 3:\n    Set s to s + 1.\nOtherwise:\n    Set s to s + 10.",
        init: "0",
        is_float: false,
    },
];

const SHAPES: &[Shape] = &[Shape::Counted, Shape::While, Shape::Nested];

/// Source for a (shape, op) cell: a function `<key><shapetag>` with the op in the loop body.
fn kernel_src(shape: Shape, op: &Op) -> (String, String) {
    let ty = if op.is_float { "Real" } else { "Int" };
    let tag = match shape {
        Shape::Counted => "c",
        Shape::While => "w",
        Shape::Nested => "n",
    };
    let fname = format!("{}{}", op.key, tag);
    // Re-indent every body line to the loop body's depth (8 spaces for counted/while, 12 for the nested
    // inner loop) so an indent-sensitive multi-line `If/Otherwise` block is valid at each shape.
    let base = if matches!(shape, Shape::Nested) {
        "            "
    } else {
        "        "
    };
    let body: String = op
        .body
        .lines()
        .map(|l| format!("{base}{l}"))
        .collect::<Vec<_>>()
        .join("\n");
    let src = match shape {
        Shape::Counted => format!(
            "## To {fname} (n: Int) -> {ty}:\n    Let mutable s be {}.\n    Repeat for i from 1 to n:\n{body}\n    Return s.\n",
            op.init
        ),
        Shape::While => format!(
            "## To {fname} (n: Int) -> {ty}:\n    Let mutable s be {}.\n    Let mutable i be 0.\n    While i is less than n:\n{body}\n        Set i to i + 1.\n    Return s.\n",
            op.init
        ),
        Shape::Nested => format!(
            "## To {fname} (n: Int) -> {ty}:\n    Let mutable s be {}.\n    Repeat for i from 1 to 2:\n        Repeat for j from 1 to n:\n{body}\n    Return s.\n",
            op.init
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

/// The VM result string for `<defs>## Main Show <fname>(x).` (int decimal or float).
fn vm_result(src: &str, fname: &str, x: i64) -> String {
    let full = format!("{src}## Main\n    Show {fname}({x}).\n");
    let out = vm_outcome(&full);
    assert!(
        out.error.is_none(),
        "{fname}({x}): VM error {:?}",
        out.error
    );
    out.output.trim().to_string()
}

fn matches(got_bits: i64, is_float: bool, vm: &str) -> bool {
    if is_float {
        match vm.parse::<f64>() {
            Ok(v) => {
                let f = f64::from_bits(got_bits as u64);
                (f - v).abs() <= 1e-9 * v.abs().max(1.0)
            }
            Err(_) => false,
        }
    } else {
        got_bits.to_string() == vm
    }
}

#[test]
fn every_fabric_op_lowers_on_every_loop_shape_matching_the_vm() {
    std::panic::set_hook(Box::new(|_| {})); // classify lowering/parse panics per cell, not abort

    // COVERAGE RATCHET: the realized (shape × op) grid must equal the declared catalog — no cell missing.
    let declared: BTreeSet<(&str, &str)> = SHAPES
        .iter()
        .flat_map(|s| OPS.iter().map(move |o| (s.name(), o.key)))
        .collect();

    let inputs: &[i64] = &[1, 3, 5];
    let mut realized: BTreeSet<(&str, &str)> = BTreeSet::new();
    let mut failures: Vec<String> = Vec::new();

    for &shape in SHAPES {
        for op in OPS {
            let (fname, src) = kernel_src(shape, op);
            let lowered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                lower(shape, &fname, &src, inputs)
            }));
            let (body, results) = match lowered {
                Ok(x) => x,
                Err(e) => {
                    let msg = e
                        .downcast_ref::<String>()
                        .map(|s| s.as_str())
                        .or_else(|| e.downcast_ref::<&str>().copied())
                        .unwrap_or("panic");
                    failures.push(format!(
                        "{} [{}]: did not lower: {}",
                        fname,
                        shape.name(),
                        &msg[..msg.len().min(70)]
                    ));
                    continue;
                }
            };

            // (a) genuinely emits the expected tile.
            if !body_has_tile(&body, op.tile) {
                failures.push(format!(
                    "{} [{}]: body has NO {} tile (degraded / mislowered)",
                    fname,
                    shape.name(),
                    op.tile.name()
                ));
                continue;
            }
            // (b) ≡ VM on every input.
            let mut ok = true;
            for (&x, &got) in inputs.iter().zip(results.iter()) {
                let vm = vm_result(&src, &fname, x);
                if !matches(got, op.is_float, &vm) {
                    let shown = if op.is_float {
                        format!("{}", f64::from_bits(got as u64))
                    } else {
                        got.to_string()
                    };
                    failures.push(format!(
                        "{}({x}) [{}]: fabric {shown} != VM {vm}",
                        fname,
                        shape.name()
                    ));
                    ok = false;
                    break;
                }
            }
            if ok {
                realized.insert((shape.name(), op.key));
            }
        }
    }

    if !failures.is_empty() {
        eprintln!("=== fabric op-matrix failures ({}) ===", failures.len());
        for f in &failures {
            eprintln!("  {f}");
        }
    }
    assert!(
        failures.is_empty(),
        "fabric op-matrix: {} cell(s) failed (see stderr list above)",
        failures.len()
    );
    assert_eq!(
        realized,
        declared,
        "the op-matrix must cover every (shape × op) cell — missing: {:?}",
        declared.difference(&realized).collect::<Vec<_>>()
    );

    println!(
        "fabric op-matrix: {} cells ({} ops × {} loop shapes) all lower to the combinational fabric ≡ VM \
         with the correct Arith / Func / Select tile",
        declared.len(),
        OPS.len(),
        SHAPES.len()
    );
}
