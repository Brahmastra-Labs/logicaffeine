//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! A NON-FINITE CONSTANT IS SPELLED OR REFUSED BY NAME — NEVER PRINTED AS `inf`.
//!
//! The blocked arg-max masks with `−∞`, the scan's identity, and a fill or a body constant may hold `±∞` or NaN.
//! Rust's `{:?}` prints those as `inf`, `-inf` and `NaN`, none of which is a C++ or Scala literal, so a printer that
//! formats a constant that way hands its vendor toolchain a program it rejects, far from the IR that caused it. The
//! Tensix host is C++ with `<cmath>` and spells them `INFINITY`, `-INFINITY` and `NAN`. The Spatial printer has no
//! verified spelling for a non-finite `FltPt`, so it refuses one by name before printing anything.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_nonfinite_constant_lock`
//!
//! ★ MUTATION PROBES (each is the tree before this lock): the Tensix host formatting a constant with `{:?}` prints
//! `(-inf)` and a table of `inf, -inf, NaN`; the Spatial printer without the non-finite refusal prints `(-inf).to[T]`.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::spatial::ir::*;
use logicaffeine_compile::spatial::lower::spatial_lang::emit_spatial_program;
use logicaffeine_compile::spatial::lower::{
    tensix::TensixBackend, SpatialArtifact, SpatialBackend, SpatialLowerError,
};
use logicaffeine_compile::spatial::passes::from_tensor_op::{ident_map, Template};
use logicaffeine_compile::spatial::passes::{LV_GEN_COL, LV_GEN_ROW};
use logicaffeine_compile::spatial::pipeline::run_passes;
use logicaffeine_compile::spatial::target::SpatialTarget;

fn konst(v: f64) -> Box<TensorBody> {
    Box::new(TensorBody::Const(v.to_bits()))
}

fn op(i: usize) -> Box<TensorBody> {
    Box::new(TensorBody::Operand(i))
}

/// One tile-row input `x` of `cols` columns filled by `init`, one pointwise `Generic` computing `body`, one output —
/// placed, scheduled and verified by the ordinary passes.
fn pointwise(
    target: &SpatialTarget,
    name: &str,
    cols: usize,
    init: TensorInit,
    body: TensorBody,
) -> SpatialProgram {
    let t = target.tile.rows;
    let cols_t = cols.div_ceil(target.tile.cols);
    let mut tp = Template::new(target).expect("a template builder");
    let x = tp.tensor("x", TensorRole::Input, t, cols, Precision::Fp32, init);
    let xb = tp.buffer("in_x", cols_t, Precision::Fp32);
    let l = tp.load("load_x", x, vec![], cols_t, xb);
    let y = tp.tensor(
        "y",
        TensorRole::Output,
        t,
        cols,
        Precision::Fp32,
        TensorInit::Zero,
    );
    let yb = tp.buffer("out", cols_t, Precision::Fp32);
    let iters = vec![
        IterSpec {
            var: LV_GEN_ROW,
            extent: t,
            kind: IterKind::Parallel,
        },
        IterSpec {
            var: LV_GEN_COL,
            extent: cols,
            kind: IterKind::Parallel,
        },
    ];
    let g = tp.stage(
        "generic",
        StageKind::Compute(ComputeOp::Generic {
            iters,
            ins: vec![ident_map(cols)],
            out: ident_map(cols),
            body,
        }),
        vec![xb],
        vec![yb],
    );
    let st = tp.store("store_y", y, vec![], cols_t, yb);
    tp.edge(l, xb, g);
    tp.edge(g, yb, st);
    let items = vec![WorkItem {
        id: WorkItemId::new(0),
        coord: (0, 0),
        out: TileRect {
            row0_t: 0,
            col0_t: 0,
            h_t: 1,
            w_t: cols_t,
        },
    }];
    let p = tp
        .finish(
            name.to_string(),
            items,
            AccPolicy::F32,
            NumericsClaim::Approx {
                reason: "a hand-built printer probe".into(),
            },
            target,
        )
        .unwrap_or_else(|e| panic!("{name}: the template validates: {e:?}"));
    run_passes(p, target)
        .unwrap_or_else(|e| panic!("{name} on {}: the passes verify: {e:?}", target.name))
}

/// A `t × cols` fill whose first elements are `+∞`, `−∞` and NaN, and ordinary values after.
fn non_finite_fill(target: &SpatialTarget, cols: usize) -> TensorInit {
    let n = target.tile.rows * cols;
    let mut v: Vec<f64> = (0..n).map(|i| (i % 13) as f64 / 4.0 - 1.5).collect();
    v[0] = f64::INFINITY;
    v[1] = f64::NEG_INFINITY;
    v[2] = f64::NAN;
    TensorInit::FloatValues(v.iter().map(|x| x.to_bits()).collect())
}

/// The words of `text` that are Rust's debug spellings of a non-finite float — no C++ or Scala literal is.
fn bare_non_finite(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| matches!(*w, "inf" | "inff" | "NaN" | "nan" | "NaNf" | "nanf"))
        .map(str::to_string)
        .collect()
}

fn tensix_host(p: &SpatialProgram, target: &SpatialTarget) -> String {
    match TensixBackend::new(target.clone())
        .lower(p)
        .unwrap_or_else(|e| panic!("{}: Tensix prints it: {e:?}", p.name))
    {
        SpatialArtifact::Tensix(a) => a.host_cpp,
        other => panic!("a Tensix artifact, got {other:?}"),
    }
}

/// ── THE TENSIX HOST SPELLS `±∞` AND NaN IN `<cmath>`: in a body constant, a table fill, and a constant fill.
#[test]
fn the_tensix_host_spells_non_finite_constants_in_cmath() {
    let wh = SpatialTarget::wormhole();
    let cols = wh.tile.cols;
    let masked = pointwise(
        &wh,
        "nonfinite_body_and_table",
        cols,
        non_finite_fill(&wh, cols),
        TensorBody::Add(op(0), konst(f64::NEG_INFINITY)),
    );
    let host = tensix_host(&masked, &wh);
    assert!(
        bare_non_finite(&host).is_empty(),
        "no `inf`/`NaN` word in the host: {:?}",
        bare_non_finite(&host)
    );
    for spelled in ["-INFINITY", "INFINITY", "NAN"] {
        assert!(
            host.contains(spelled),
            "the host spells `{spelled}` where the program has it"
        );
    }
    let constant = pointwise(
        &wh,
        "nonfinite_const_fill",
        cols,
        TensorInit::ConstF(f64::INFINITY.to_bits()),
        TensorBody::Mul(op(0), konst(f64::NAN)),
    );
    let host = tensix_host(&constant, &wh);
    assert!(
        bare_non_finite(&host).is_empty(),
        "no `inf`/`NaN` word in the host: {:?}",
        bare_non_finite(&host)
    );
    assert!(
        host.contains("INFINITY") && host.contains("NAN"),
        "the constant fill and the body constant are both spelled"
    );
}

/// ── AND THE SPELLING IS C++: the printed table fill compiles against `<cmath>` alone.
#[test]
fn the_tensix_table_fill_is_cpp_a_compiler_accepts() {
    if !common::require_tool("g++") {
        return;
    }
    let wh = SpatialTarget::wormhole();
    let cols = wh.tile.cols;
    let p = pointwise(
        &wh,
        "nonfinite_table",
        cols,
        non_finite_fill(&wh, cols),
        TensorBody::Add(op(0), konst(0.5)),
    );
    let host = tensix_host(&p, &wh);
    let table = host
        .lines()
        .find(|l| l.contains("static const double"))
        .expect("the host prints the fill as a table");
    let dir = std::env::temp_dir().join(format!("logos_nonfinite_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("table.cpp");
    std::fs::write(
        &src,
        format!("#include <cmath>\n{table}\nint main() {{ return 0; }}\n"),
    )
    .unwrap();
    let out = std::process::Command::new("g++")
        .args(["-std=c++17", "-fsyntax-only"])
        .arg(&src)
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "the printed fill is C++:\n{}\n{}",
        &table[..table.len().min(200)],
        String::from_utf8_lossy(&out.stderr)
    );
}

fn refused(r: Result<String, SpatialLowerError>, must_name: &str) {
    match r {
        Err(SpatialLowerError::Unsupported(why)) => assert!(
            why.contains(must_name),
            "the refusal names `{must_name}`: {why}"
        ),
        Ok(src) => panic!(
            "refused by name (`{must_name}`), was printed:\n{}",
            &src[..src.len().min(2000)]
        ),
        Err(other) => panic!("refused by name (`{must_name}`), got {other:?}"),
    }
}

/// ── THE SPATIAL PRINTER REFUSES A NON-FINITE CONSTANT BY NAME — in a body (on an unselected branch, so the output
/// is finite and nothing else refuses the program) and in a fill — and prints the same program with a finite one.
#[test]
fn the_spatial_printer_refuses_a_non_finite_constant_by_name() {
    let rdu = SpatialTarget::sn40l_rdu();
    let cols = rdu.tile.cols;
    let finite_fill = || {
        TensorInit::FloatValues(
            (0..rdu.tile.rows * cols)
                .map(|i| ((i % 7) as f64 - 3.0).to_bits())
                .collect(),
        )
    };
    let never = |k: f64| TensorBody::Select {
        cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Const(0)),
        then: konst(k),
        otherwise: op(0),
    };
    let control = pointwise(&rdu, "finite_constant", cols, finite_fill(), never(-2.5));
    emit_spatial_program(&control, &rdu, None)
        .unwrap_or_else(|e| panic!("the finite twin prints: {e:?}"));
    for k in [f64::NEG_INFINITY, f64::INFINITY, f64::NAN] {
        let p = pointwise(&rdu, "nonfinite_constant", cols, finite_fill(), never(k));
        refused(emit_spatial_program(&p, &rdu, None), "non-finite");
    }
    let p = pointwise(
        &rdu,
        "nonfinite_fill",
        cols,
        non_finite_fill(&rdu, cols),
        TensorBody::Operand(0),
    );
    refused(emit_spatial_program(&p, &rdu, None), "non-finite");
}
