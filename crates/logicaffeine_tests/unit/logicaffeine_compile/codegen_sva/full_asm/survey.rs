//! Temporary: measure the compiled shape of the benchmark suite so the encoder's field widths are
//! sized to what the programs actually use (removed once the encoder is built).
use crate::vm::{Constant, Op};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn compile(src: &str) -> Result<crate::vm::CompiledProgram, String> {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _policies) = parsed.map_err(|advice| format!("{advice}"))?;
        let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
        crate::vm::Compiler::compile_for_chip(stmts, interner, Some(types), Some(oracle))
    })
}

fn const_kinds(p: &crate::vm::CompiledProgram) -> String {
    use crate::vm::Constant::*;
    let mut k: std::collections::BTreeSet<&'static str> = Default::default();
    for c in &p.constants {
        k.insert(match c {
            Int(_) => "Int",
            Bool(_) => "Bool",
            Float(_) => "Float",
            Text(_) => "Text",
            _ => "other",
        });
    }
    k.into_iter().collect::<Vec<_>>().join("/")
}

fn max_reg(op: &Op) -> u16 {
    // A conservative upper bound on the register indices an op touches (for regfile sizing).
    use Op::*;
    let m = |a: &[u16]| a.iter().copied().max().unwrap_or(0);
    match op {
        Call {
            dst,
            args_start,
            arg_count,
            ..
        } => m(&[
            *dst,
            *args_start,
            args_start.saturating_add(arg_count.saturating_sub(1)),
        ]),
        NewList { dst, start, count } | NewTuple { dst, start, count } => {
            m(&[*dst, *start, start.saturating_add(count.saturating_sub(1))])
        }
        _ => 0,
    }
}

#[test]
#[ignore = "survey — run explicitly with --ignored --nocapture"]
fn bench_op_survey() {
    let mut names: Vec<String> = Vec::new();
    let progs = root().join("benchmarks/programs");
    let mut dirs: Vec<_> = std::fs::read_dir(&progs)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    dirs.sort();
    let mut all_ops: std::collections::BTreeSet<&'static str> = Default::default();
    for d in dirs {
        let f = d.join("interp.lg");
        if !f.exists() {
            continue;
        }
        let name = d.file_name().unwrap().to_string_lossy().to_string();
        let src = std::fs::read_to_string(&f).unwrap();
        match compile(&src) {
            Ok(p) => {
                let mut maxreg = p.register_count;
                for func in &p.functions {
                    maxreg = maxreg.max(func.register_count);
                }
                let mut maxr2 = 0u16;
                let mut ops: std::collections::BTreeSet<&'static str> = Default::default();
                for op in &p.code {
                    ops.insert(op.name());
                    all_ops.insert(op.name());
                    maxr2 = maxr2.max(max_reg(op));
                }
                let maxc = p
                    .constants
                    .iter()
                    .filter_map(|c| match c {
                        Constant::Int(v) => Some(v.unsigned_abs()),
                        _ => None,
                    })
                    .max()
                    .unwrap_or(0);
                let ck = const_kinds(&p);
                names.push(format!(
                    "{:16} code={:4} fns={:2} regcount={:3} argmax={:3} maxconst={:<12} consts={:12} ops=[{}]",
                    name, p.code.len(), p.functions.len(), maxreg, maxr2, maxc, ck, ops.into_iter().collect::<Vec<_>>().join(",")
                ));
            }
            Err(e) => names.push(format!(
                "{name:16} COMPILE-ERR: {}",
                e.lines().next().unwrap_or("")
            )),
        }
    }
    println!("\n===== BENCHMARK OP SURVEY =====");
    for l in &names {
        println!("{l}");
    }
    println!(
        "\n===== UNION OF ALL OPS =====\n{}",
        all_ops.into_iter().collect::<Vec<_>>().join(", ")
    );
}
