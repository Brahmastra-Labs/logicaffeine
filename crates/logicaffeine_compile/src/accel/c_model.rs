//! The executable device model as C: what the host binary links and calls when it runs on a machine that
//! has no accelerator.
//!
//! The model is generated from the same objects the device artifact is: each region's control-flow graph and
//! the lane routines its cells are, at the device's lane shape. Every lane routine becomes a C function over
//! `uint32_t` words masked to the lane width (one statement per lane operation, a `for` per `Repeat`), and
//! every region becomes a C function that walks its blocks with `goto`, calls a routine per cell, and returns
//! 0 with the divergence code the moment a cell hands the call back. `logos_dev_call` is the driver the
//! host's dispatch calls; it counts what every region did and, at exit, appends one line of accounting to the
//! file `LOGOS_ACCEL_STATS` names. `LOGOS_ACCEL_OFFLOAD=off` makes it hand every call back. Neither ever
//! writes to the program's standard output or standard error.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::codegen_sva::fabric_rtl::Divergence;

use super::cells::{routine, CellOp};
use super::int64::FAULT_BITS;
use super::lane::{split_u64, LInst, LOp, LStmt, LaneShape, Routine};
use super::region::{RStmt, RTerm, RegionCfg};

/// The C symbol of `op`'s routine at `shape`.
pub fn routine_symbol(op: CellOp, shape: LaneShape) -> String {
    format!("lr_{}_w{}m{}", op.name(), shape.word, shape.mul)
}

fn mask(shape: LaneShape) -> String {
    format!("0x{:X}u", shape.mask())
}

fn inst_c(out: &mut String, i: &LInst, shape: LaneShape, pad: &str) {
    let m = mask(shape);
    let w = shape.word;
    let d = i.dst;
    match i.op {
        LOp::Const(k) => writeln!(out, "{pad}r[{d}] = {k}u;"),
        LOp::Mov(a) => writeln!(out, "{pad}r[{d}] = r[{a}];"),
        LOp::Add { a, b, cin } => {
            let c = i.dst2.expect("an add writes its carry");
            writeln!(
                out,
                "{pad}{{ uint64_t s_ = (uint64_t)r[{a}] + (uint64_t)r[{b}] + (uint64_t)r[{cin}]; r[{d}] = (uint32_t)(s_ & {m}); r[{c}] = (uint32_t)(s_ >> {w}); }}"
            )
        }
        LOp::Sub { a, b, bin } => {
            let c = i.dst2.expect("a subtract writes its borrow");
            writeln!(
                out,
                "{pad}{{ int64_t s_ = (int64_t)r[{a}] - (int64_t)r[{b}] - (int64_t)r[{bin}]; r[{d}] = (uint32_t)((uint64_t)s_ & {m}); r[{c}] = (uint32_t)(s_ < 0); }}"
            )
        }
        LOp::Mul { a, b } => writeln!(out, "{pad}r[{d}] = r[{a}] * r[{b}];"),
        LOp::And { a, b } => writeln!(out, "{pad}r[{d}] = r[{a}] & r[{b}];"),
        LOp::Or { a, b } => writeln!(out, "{pad}r[{d}] = r[{a}] | r[{b}];"),
        LOp::Xor { a, b } => writeln!(out, "{pad}r[{d}] = r[{a}] ^ r[{b}];"),
        LOp::Not { a } => writeln!(out, "{pad}r[{d}] = ~r[{a}] & {m};"),
        LOp::Shl { a, k } => writeln!(out, "{pad}r[{d}] = (r[{a}] << {k}) & {m};"),
        LOp::Shr { a, k } => writeln!(out, "{pad}r[{d}] = r[{a}] >> {k};"),
        LOp::Ltu { a, b } => writeln!(out, "{pad}r[{d}] = r[{a}] < r[{b}];"),
        LOp::Eq { a, b } => writeln!(out, "{pad}r[{d}] = r[{a}] == r[{b}];"),
        LOp::Sel { c, t, f } => writeln!(out, "{pad}r[{d}] = r[{c}] ? r[{t}] : r[{f}];"),
    }
    .expect("writing to a String");
}

fn body_c(out: &mut String, body: &[LStmt], shape: LaneShape, depth: usize) {
    let pad = "    ".repeat(depth + 1);
    for s in body {
        match s {
            LStmt::Inst(i) => inst_c(out, i, shape, &pad),
            LStmt::Repeat { times, body } => {
                writeln!(out, "{pad}for (uint32_t t{depth}_ = 0; t{depth}_ < {times}u; t{depth}_++) {{").expect("write");
                body_c(out, body, shape, depth + 1);
                writeln!(out, "{pad}}}").expect("write");
            }
        }
    }
}

/// C for one lane routine: `static void <symbol>(const uint32_t *in, uint32_t *out)`. Its register file is a
/// local that starts at zero on every call, as the lane interpreter's does, so a call depends on nothing but
/// its inputs and the model is reentrant.
pub fn routine_c(r: &Routine, symbol: &str) -> String {
    let mut out = String::new();
    writeln!(out, "static void {symbol}(const uint32_t *in, uint32_t *out) {{").expect("write");
    writeln!(out, "    uint32_t r[{}] = {{0}};", r.regs.max(1)).expect("write");
    for (k, reg) in r.inputs.iter().enumerate() {
        writeln!(out, "    r[{reg}] = in[{k}];").expect("write");
    }
    body_c(&mut out, &r.body, r.shape, 0);
    for (k, reg) in r.outputs.iter().enumerate() {
        writeln!(out, "    out[{k}] = r[{reg}];").expect("write");
    }
    writeln!(out, "}}\n").expect("write");
    out
}

/// The cell ops a region uses, selects included.
fn ops_of(region: &RegionCfg) -> Vec<CellOp> {
    let mut ops = Vec::new();
    for b in &region.blocks {
        for s in &b.stmts {
            let op = match s {
                RStmt::Cell { op, .. } => *op,
                RStmt::Select { cmp, .. } => CellOp::Select(*cmp),
                _ => continue,
            };
            if !ops.contains(&op) {
                ops.push(op);
            }
        }
    }
    ops
}

/// C for a region at `shape`: `static int <symbol>(const int64_t *args, int64_t *out, uint32_t *fault)`,
/// returning 1 with `*out` set, or 0 with `*fault` the divergence code.
pub fn region_c(region: &RegionCfg, shape: LaneShape, symbol: &str) -> String {
    let n = shape.words_for(64);
    let nf = shape.words_for(FAULT_BITS);
    let w = shape.word;
    let mut out = String::new();
    writeln!(out, "static int {symbol}(const int64_t *args, int64_t *out, uint32_t *fault) {{").expect("write");
    writeln!(out, "    uint32_t v[{}][{n}] = {{{{0}}}};", region.regs.max(1)).expect("write");
    writeln!(out, "    uint32_t in_[{}], o_[{}];", 4 * n, n + nf).expect("write");
    for i in 0..region.params.len() {
        for k in 0..n {
            writeln!(out, "    v[{i}][{k}] = (uint32_t)(((uint64_t)args[{i}] >> {}) & {});", k as u32 * w, mask(shape)).expect("write");
        }
    }
    writeln!(out, "    goto B0;").expect("write");
    let copy = |out: &mut String, dst: &str, src: &str| {
        writeln!(out, "    memcpy({dst}, {src}, sizeof(uint32_t) * {n});").expect("write");
    };
    for (bi, b) in region.blocks.iter().enumerate() {
        writeln!(out, "B{bi}:;").expect("write");
        for s in &b.stmts {
            match *s {
                RStmt::Const { dst, value } => {
                    for (k, word) in split_u64(shape, value as u64, n).iter().enumerate() {
                        writeln!(out, "    v[{dst}][{k}] = {word}u;").expect("write");
                    }
                }
                RStmt::Copy { dst, src } => copy(&mut out, &format!("v[{dst}]"), &format!("v[{src}]")),
                RStmt::Cell { dst, op, a, b } => {
                    copy(&mut out, "in_", &format!("v[{a}]"));
                    copy(&mut out, &format!("in_ + {n}"), &format!("v[{b}]"));
                    writeln!(out, "    {}(in_, o_);", routine_symbol(op, shape)).expect("write");
                    if op.faults() {
                        let joined: Vec<String> = (0..nf).map(|k| format!("((uint32_t)o_[{}] << {})", n + k, k as u32 * w)).collect();
                        writeln!(out, "    {{ uint32_t f_ = {}; if (f_) {{ *fault = f_; return 0; }} }}", joined.join(" | ")).expect("write");
                    }
                    copy(&mut out, &format!("v[{dst}]"), "o_");
                }
                RStmt::Select { dst, cmp, a, b, t, e } => {
                    copy(&mut out, "in_", &format!("v[{a}]"));
                    copy(&mut out, &format!("in_ + {n}"), &format!("v[{b}]"));
                    copy(&mut out, &format!("in_ + {}", 2 * n), &format!("v[{t}]"));
                    copy(&mut out, &format!("in_ + {}", 3 * n), &format!("v[{e}]"));
                    writeln!(out, "    {}(in_, o_);", routine_symbol(CellOp::Select(cmp), shape)).expect("write");
                    copy(&mut out, &format!("v[{dst}]"), "o_");
                }
            }
        }
        match b.term {
            RTerm::Jump(t) => writeln!(out, "    goto B{t};"),
            RTerm::Branch { cond, then_, else_ } => {
                let any: Vec<String> = (0..n).map(|k| format!("v[{cond}][{k}]")).collect();
                writeln!(out, "    if ({}) goto B{then_}; else goto B{else_};", any.join(" | "))
            }
            RTerm::Return(r) => {
                let joined: Vec<String> = (0..n).map(|k| format!("((uint64_t)v[{r}][{k}] << {})", k as u32 * w)).collect();
                writeln!(out, "    *out = (int64_t)({});\n    return 1;", joined.join(" | "))
            }
        }
        .expect("write");
    }
    writeln!(out, "}}\n").expect("write");
    out
}

/// The model translation unit for `regions` (each with its host-side region number) on a device whose lanes
/// are `shape`: every routine the regions use, every region, and the `logos_dev_call` driver with its
/// accounting. `device` names the device in the accounting line.
pub fn model_c(regions: &[(u32, RegionCfg)], shape: LaneShape, device: &str) -> String {
    let mut out = String::new();
    writeln!(out, "/* The {device} device model: lane routines at {}-bit words with {}-bit multipliers, and the regions they run. */", shape.word, shape.mul)
        .expect("write");
    writeln!(out, "#include <stdint.h>\n#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n").expect("write");
    let mut used: BTreeMap<String, CellOp> = BTreeMap::new();
    for (_, r) in regions {
        for op in ops_of(r) {
            used.insert(routine_symbol(op, shape), op);
        }
    }
    for (sym, op) in &used {
        out.push_str(&routine_c(&routine(shape, *op), sym));
    }
    for (id, r) in regions {
        out.push_str(&region_c(r, shape, &format!("logos_region_{id}")));
    }
    let count = regions.iter().map(|(id, _)| *id as usize + 1).max().unwrap_or(0).max(1);
    let codes = Divergence::ALL.iter().map(|d| d.code()).max().unwrap_or(0) as usize + 1;
    writeln!(out, "static uint64_t logos_dev_done[{count}];").expect("write");
    writeln!(out, "static uint64_t logos_dev_declined[{count}][{codes}];").expect("write");
    writeln!(out, "static uint64_t logos_dev_off_calls[{count}];").expect("write");
    writeln!(out, "static int logos_dev_mode = -1;\n").expect("write");
    let names: Vec<String> = (0..count)
        .map(|i| {
            regions.iter().find(|(id, _)| *id as usize == i).map(|(_, r)| format!("\"{}\"", r.name.replace('\\', "\\\\").replace('"', "\\\""))).unwrap_or_else(|| "\"\"".to_string())
        })
        .collect();
    writeln!(out, "static const char *logos_dev_names[{count}] = {{ {} }};\n", names.join(", ")).expect("write");
    let reasons: Vec<String> = (0..codes)
        .map(|c| format!("\"{}\"", Divergence::from_code(c as u32).map(|d| d.name()).unwrap_or("none")))
        .collect();
    writeln!(out, "static const char *logos_dev_reasons[{codes}] = {{ {} }};\n", reasons.join(", ")).expect("write");
    writeln!(
        out,
        r#"static void logos_dev_account(void) {{
    const char *path = getenv("LOGOS_ACCEL_STATS");
    if (!path || !*path) return;
    FILE *f = fopen(path, "a");
    if (!f) return;
    fprintf(f, "{{\"device\":\"{device}\",\"regions\":[");
    for (int i = 0; i < {count}; i++) {{
        fprintf(f, "%s{{\"region\":%d,\"name\":\"%s\",\"done\":%llu,\"offload_off\":%llu,\"declined\":{{", i ? "," : "", i, logos_dev_names[i], (unsigned long long)logos_dev_done[i], (unsigned long long)logos_dev_off_calls[i]);
        int first = 1;
        for (int c = 1; c < {codes}; c++) {{
            if (!logos_dev_declined[i][c]) continue;
            fprintf(f, "%s\"%s\":%llu", first ? "" : ",", logos_dev_reasons[c], (unsigned long long)logos_dev_declined[i][c]);
            first = 0;
        }}
        fprintf(f, "}}}}");
    }}
    fprintf(f, "]}}\n");
    fclose(f);
}}

int logos_dev_call(int region, const int64_t *args, int64_t *out) {{
    if (logos_dev_mode < 0) {{
        const char *m = getenv("LOGOS_ACCEL_OFFLOAD");
        logos_dev_mode = (m && strcmp(m, "off") == 0) ? 0 : 1;
        atexit(logos_dev_account);
    }}
    if (region < 0 || region >= {count}) return 0;
    if (!logos_dev_mode) {{ logos_dev_off_calls[region]++; return 0; }}
    uint32_t fault = 0;
    int ok = 0;
    switch (region) {{"#
    )
    .expect("write");
    for (id, _) in regions {
        writeln!(out, "        case {id}: ok = logos_region_{id}(args, out, &fault); break;").expect("write");
    }
    writeln!(
        out,
        r#"        default: return 0;
    }}
    if (ok) logos_dev_done[region]++;
    else if (fault < {codes}) logos_dev_declined[region][fault]++;
    return ok;
}}"#
    )
    .expect("write");
    out
}
