//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! NUMERIC-COPROCESSOR BUILTINS ON THE FABRIC — the pit-of-success ratchet for `sqrt`/`floor`/`abs`/`min`/
//! `max` lowering to the single-cycle combinational loop fabric, on EVERY loop shape.
//!
//! These builtins were `exec_ops`-unsupported (a `sqrt`/`abs`/… in a loop body panicked); now `exec_ops`
//! lowers each to the matching tile — `FSqrtF`/`FFloorF` functional units, a float sign-clear / integer
//! negate+select for `abs`, an integer select for `min`/`max`. Each kernel MUST lower on counted / while /
//! nested loops, GENUINELY use its tile in the body, and evaluate ≡ VM. COVERAGE RATCHET: the realized
//! (builtin × shape) grid must equal the declared catalog. Extend it; never delete a cell.
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

#[derive(Clone, Copy)]
enum Tile {
    Arith(&'static str),
    Func(&'static str),
    Select(&'static str),
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
        FuncUnit::FSqrtF => "FSqrtF",
        FuncUnit::FFloorF => "FFloorF",
        FuncUnit::FAddF => "FAddF",
        FuncUnit::FSubF => "FSubF",
        FuncUnit::FLtF => "FLtF",
        FuncUnit::FGtF => "FGtF",
        FuncUnit::PopCount => "PopCount",
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

struct Builtin {
    key: &'static str,
    ret: &'static str,
    s_init: &'static str,
    prelude: &'static str, // extra `Let mutable` before the loop (empty for most)
    body: &'static str,    // loop-body statement(s), relative indent
    tile: Tile,
    is_float: bool,
}

const BUILTINS: &[Builtin] = &[
    // `sqrt(s + 12)` — converges; the FSqrtF float functional unit.
    Builtin {
        key: "sqrt",
        ret: "Real",
        s_init: "0.0",
        prelude: "",
        body: "Set s to sqrt(s + 12.0).",
        tile: Tile::Func("FSqrtF"),
        is_float: true,
    },
    // integer `abs(i - 5)` — the negate+select form.
    Builtin {
        key: "absi",
        ret: "Int",
        s_init: "0",
        prelude: "",
        body: "Set s to s + abs(i - 5).",
        tile: Tile::Select("Lt"),
        is_float: false,
    },
    // float `abs(s - 100.0)` — the sign-bit clear (`& 0x7FFF…`).
    Builtin {
        key: "absf",
        ret: "Real",
        s_init: "0.0",
        prelude: "",
        body: "Set s to abs(s - 100.0).",
        tile: Tile::Arith("And"),
        is_float: true,
    },
    // integer `min` / `max` — a select mux.
    Builtin {
        key: "min",
        ret: "Int",
        s_init: "0",
        prelude: "",
        body: "Set s to min(s + 7, 30).",
        tile: Tile::Select("Lt"),
        is_float: false,
    },
    Builtin {
        key: "max",
        ret: "Int",
        s_init: "20",
        prelude: "",
        body: "Set s to max(s - 2, 3).",
        tile: Tile::Select("Gt"),
        is_float: false,
    },
    // float `min` / `max` — a float-compare functional unit (`FLtF`/`FGtF`) driving a select mux.
    Builtin {
        key: "fmin",
        ret: "Real",
        s_init: "100.0",
        prelude: "",
        body: "Set s to min(s, 50.5).",
        tile: Tile::Func("FLtF"),
        is_float: true,
    },
    Builtin {
        key: "fmax",
        ret: "Real",
        s_init: "0.0",
        prelude: "",
        body: "Set s to max(s, 3.5).",
        tile: Tile::Func("FGtF"),
        is_float: true,
    },
    // `floor` bridges a float accumulator `f` down to the integer sum `s` — the FFloorF functional unit.
    Builtin {
        key: "floor",
        ret: "Int",
        s_init: "0",
        prelude: "Let mutable f be 0.0.",
        body: "Set f to f + 1.7.\nSet s to s + floor(f).",
        tile: Tile::Func("FFloorF"),
        is_float: false,
    },
    // `ceil(x) = -floor(-x)` — reuses FFloorF; the negative-seeded `f` exercises ceil across the sign boundary.
    Builtin {
        key: "ceil",
        ret: "Int",
        s_init: "0",
        prelude: "Let mutable f be -3.0.",
        body: "Set f to f + 1.3.\nSet s to s + ceil(f).",
        tile: Tile::Func("FFloorF"),
        is_float: false,
    },
    // `round(x)` — IEEE round-HALF-AWAY-FROM-ZERO; the `-3.0`/`+0.5` walk hits exact ties on BOTH signs
    // (round(-2.5) = -3, round(0.5) = 1), exercising the sign-select branch of the lowering.
    Builtin {
        key: "round",
        ret: "Int",
        s_init: "0",
        prelude: "Let mutable f be -3.0.",
        body: "Set f to f + 0.5.\nSet s to s + round(f).",
        tile: Tile::Func("FFloorF"),
        is_float: false,
    },
    // `count_ones` — a combinational popcount functional unit over a varied bit pattern.
    Builtin {
        key: "popcount",
        ret: "Int",
        s_init: "0",
        prelude: "",
        body: "Set s to s + count_ones(i * 123456789).",
        tile: Tile::Func("PopCount"),
        is_float: false,
    },
];

const SHAPES: &[Shape] = &[Shape::Counted, Shape::While, Shape::Nested];

fn kernel_src(shape: Shape, b: &Builtin) -> (String, String) {
    let tag = match shape {
        Shape::Counted => "c",
        Shape::While => "w",
        Shape::Nested => "n",
    };
    let fname = format!("{}{}", b.key, tag);
    let base = if matches!(shape, Shape::Nested) {
        "            "
    } else {
        "        "
    };
    let ibody: String = b
        .body
        .lines()
        .map(|l| format!("{base}{l}"))
        .collect::<Vec<_>>()
        .join("\n");
    let pre = if b.prelude.is_empty() {
        String::new()
    } else {
        format!("    {}\n", b.prelude)
    };
    let src = match shape {
        Shape::Counted => format!(
            "## To {fname} (n: Int) -> {}:\n    Let mutable s be {}.\n{pre}    Repeat for i from 1 to n:\n{ibody}\n    Return s.\n",
            b.ret, b.s_init
        ),
        Shape::While => format!(
            "## To {fname} (n: Int) -> {}:\n    Let mutable s be {}.\n{pre}    Let mutable i be 0.\n    While i is less than n:\n{ibody}\n        Set i to i + 1.\n    Return s.\n",
            b.ret, b.s_init
        ),
        Shape::Nested => format!(
            "## To {fname} (n: Int) -> {}:\n    Let mutable s be {}.\n{pre}    Repeat for i from 1 to 2:\n        Repeat for j from 1 to n:\n{ibody}\n    Return s.\n",
            b.ret, b.s_init
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
            Ok(v) => (f64::from_bits(got_bits as u64) - v).abs() <= 1e-9 * v.abs().max(1.0),
            Err(_) => false,
        }
    } else {
        got_bits.to_string() == vm
    }
}

#[test]
fn numeric_builtins_lower_on_every_loop_shape_matching_the_vm() {
    std::panic::set_hook(Box::new(|_| {}));

    let declared: BTreeSet<(&str, &str)> = SHAPES
        .iter()
        .flat_map(|s| BUILTINS.iter().map(move |b| (s.name(), b.key)))
        .collect();
    let inputs: &[i64] = &[1, 3, 6];
    let mut realized: BTreeSet<(&str, &str)> = BTreeSet::new();
    let mut failures: Vec<String> = Vec::new();

    for &shape in SHAPES {
        for b in BUILTINS {
            let (fname, src) = kernel_src(shape, b);
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
            if !body_has_tile(&body, b.tile) {
                failures.push(format!(
                    "{fname} [{}]: body has NO expected tile",
                    shape.name()
                ));
                continue;
            }
            let mut ok = true;
            for (&x, &got) in inputs.iter().zip(results.iter()) {
                let vm = vm_result(&src, &fname, x);
                if !matches(got, b.is_float, &vm) {
                    let shown = if b.is_float {
                        format!("{}", f64::from_bits(got as u64))
                    } else {
                        got.to_string()
                    };
                    failures.push(format!(
                        "{fname}({x}) [{}]: fabric {shown} != VM {vm}",
                        shape.name()
                    ));
                    ok = false;
                    break;
                }
            }
            if ok {
                realized.insert((shape.name(), b.key));
            }
        }
    }

    if !failures.is_empty() {
        eprintln!("=== builtin-fabric failures ({}) ===", failures.len());
        for f in &failures {
            eprintln!("  {f}");
        }
    }
    assert!(
        failures.is_empty(),
        "numeric builtins: {} cell(s) failed (see stderr)",
        failures.len()
    );
    assert_eq!(
        realized,
        declared,
        "builtin coverage gap: {:?}",
        declared.difference(&realized).collect::<Vec<_>>()
    );

    println!(
        "builtin-fabric: {} cells ({} builtins × {} loop shapes) lower to the combinational fabric ≡ VM",
        declared.len(),
        BUILTINS.len(),
        SHAPES.len()
    );
}
