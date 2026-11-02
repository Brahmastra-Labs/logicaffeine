//! silicon_antenna_safety_lock — the family-wide pit-of-success guard: no emitted LOGOS silicon may
//! contain an UNBOUNDED wide single-bit select, the met1-antenna shape that the GPU's 64-bit
//! combinational multiply had (a lone select bit `mp[0]` fanning out across the full datapath via
//! `{64{mp[0]}} & mc`). That net's fan-out is exactly the met1 area-ratio violation antenna checks
//! flag, and no diode/jog repairs it — only bounding the fan-out at the source does (see the GPU's
//! `SequentialFma` fix and `codegen_sva::fanout::bounded_fanout_select`).
//!
//! This lock makes that impossible BY CONSTRUCTION across every emitter: it scans each emitted design
//! golden for a `{N{sig}}` replication of a NON-constant one-bit signal with `N` above the
//! antenna-safe fan-out bound, and FAILS unless the emitter routed it through `bounded_fanout_select`
//! (which slices to ≤ the bound) — or the pattern is an explicitly justified benign case in `ALLOW`.
//! So a new emitter cannot silently ship unsignoffable RTL: it must bound the fan-out or consciously
//! justify why the wide replication is safe.
//!
//! SCOPE / honesty: this targets the AND-replication select shape (`{N{sig}} & bus`) — the *proven*
//! GPU-multiply hazard. It does NOT flag wide 1-bit ternary muxes (`sig ? wideA : wideB`): the GPU's
//! own `mask[j] ? k : 0` output mux is exactly that at 64-bit width and signed off antenna-clean, so
//! a ternary's danger is P&R-context-dependent (density), not syntactic — flagging it would be a
//! false positive. Antenna is ultimately a place-and-route outcome; this guard catches the one
//! syntactic pattern that reliably caused it, and leaves the rest to the signoff receipt.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(silicon_antenna_safety_lock)'`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

/// The antenna-safe single-bit fan-out bound. `soft_core.v`'s `{16{imm[15]}}` sign-extension is
/// signed off CLEAN on Sky130, so 16 is a proven-safe replication width; `bounded_fanout_select`
/// emits groups of `WIDTH/slice` (8 for the 64-bit GPU lane), comfortably under it. A replication
/// wider than this must be bounded (via the primitive) or justified in `ALLOW`.
const ANTENNA_SAFE_FANOUT: usize = 16;

/// Explicitly justified benign wide replications: `(golden file, exact "{N{X}}" text, why it is safe)`.
/// Keep this list SHORT and each entry auditable — it is the escape hatch that keeps the guard from
/// blocking legitimate patterns without hiding a real hazard. Every entry is re-checked for presence
/// by `allowlist_has_no_stale_entries`, so a pattern that changes/disappears cannot rot here.
const ALLOW: &[(&str, &str, &str)] = &[(
    "decision_hft_realbist.v",
    "{56{lfsr[31]}}",
    "sign-extension of an LFSR register bit (`inventory = {{56{lfsr[31]}}, lfsr[31:24]}`) in an \
     FPGA-only BIST block that is NOT a Sky130 signoff target; a register-bit sign-extend is not the \
     dense combinational-multiply-select shape that antennas.",
), (
    "nexys_matmul_soc.v",
    "{56{rx_byte[7]}}",
    "sign-extension of a received int8 UART byte (`mem_wdata = {{56{rx_byte[7]}}, rx_byte}`) on the cold \
     operand-load path of an FPGA-only SoC (one write per serial byte, not a Sky130 signoff target); a \
     register-bit sign-extend is not the dense combinational-multiply-select shape that antennas — the \
     same benign class as the decision_hft_realbist entry above.",
)];

/// Directory of emitted silicon goldens. Resolved from `CARGO_MANIFEST_DIR` (the crate dir) up to the
/// workspace root, so the test works regardless of the process CWD — the same anchor the other silicon
/// locks use.
fn silicon_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon")
}

/// Extract every `{N{X}}` replication (N = digits, X = the replicated expression with no nested
/// braces) from Verilog source, as `(N, X)`. A `{{56{..}}, ..}` concatenation is handled: after the
/// first `{`, a `{` (not a digit) means it is not a replication start, so the inner `{56{..}}` is the
/// one caught. A `{58'd0, ..}` zero-pad is skipped: `58` is followed by `'`, not `{`.
fn replications(rtl: &str) -> Vec<(usize, String)> {
    let b = rtl.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'{' {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            // `{` <digits> `{` … `}}`
            if j > i + 1 && j < b.len() && b[j] == b'{' {
                let n: usize = rtl[i + 1..j].parse().unwrap_or(0);
                let inner = j + 1;
                if let Some(rel) = rtl[inner..].find("}}") {
                    let x = &rtl[inner..inner + rel];
                    if !x.contains('{') && !x.contains('}') {
                        out.push((n, x.trim().to_string()));
                        i = inner + rel + 2;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    out
}

/// A replicated expression is a CONSTANT (benign — no signal fan-out) if it is a Verilog sized/based
/// literal (`1'b0`, `1'b1`, `8'hFF`, `64'd0` — they carry a `'`) or a bare decimal. A SIGNAL is an
/// identifier, optionally bit-selected (`lfsr[31]`, `imm[15]`, `msel7_l0`) — those DO fan out.
fn is_constant(x: &str) -> bool {
    x.contains('\'') || x.chars().all(|c| c.is_ascii_digit())
}

/// Every emitted DESIGN golden (`benchmarks/silicon/*.v`), excluding testbenches (`tb_*.v`, which
/// drive stimulus and are never synthesized to silicon).
fn design_goldens() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for entry in fs::read_dir(silicon_dir()).expect("benchmarks/silicon is readable") {
        let path = entry.expect("dir entry").path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.ends_with(".v") && !name.starts_with("tb_") {
            let src = fs::read_to_string(&path).unwrap_or_default();
            out.push((name, src));
        }
    }
    assert!(
        !out.is_empty(),
        "no design goldens found under benchmarks/silicon"
    );
    out
}

#[test]
fn no_emitter_produces_an_unbounded_wide_antenna_select() {
    let allow: std::collections::HashSet<(&str, &str)> =
        ALLOW.iter().map(|(f, p, _)| (*f, *p)).collect();

    let mut violations = Vec::new();
    for (name, src) in design_goldens() {
        for (n, x) in replications(&src) {
            if n <= ANTENNA_SAFE_FANOUT || is_constant(&x) {
                continue;
            }
            let pat = format!("{{{n}{{{x}}}}}");
            if allow.contains(&(name.as_str(), pat.as_str())) {
                continue;
            }
            violations.push(format!("{name}: `{pat}` (fan-out {n})"));
        }
    }
    assert!(
        violations.is_empty(),
        "unbounded wide single-bit select(s) exceed the antenna-safe fan-out ({ANTENNA_SAFE_FANOUT}) — \
         the met1-antenna shape the GPU multiply had. Slice each through \
         `codegen_sva::fanout::bounded_fanout_select` (fan-out ≤ the bound), or add a justified entry \
         to ALLOW in this file:\n  {}",
        violations.join("\n  ")
    );
}

/// The allowlist may not rot: every `ALLOW` entry must still name a real golden that still contains
/// exactly that pattern. A stale entry (pattern edited away, file renamed) is itself a lock failure,
/// so the escape hatch can never silently mask a future hazard by carrying a dead exception.
#[test]
fn allowlist_has_no_stale_entries() {
    for (file, pat, why) in ALLOW {
        assert!(
            !why.trim().is_empty(),
            "ALLOW entry {file} {pat} has no justification"
        );
        let path = silicon_dir().join(file);
        let src = fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!("ALLOW names {file}, which does not exist under benchmarks/silicon")
        });
        assert!(
            src.contains(pat),
            "ALLOW entry `{pat}` no longer appears in {file} — remove the stale exception"
        );
    }
}

/// The GPU golden — the block this whole guard is modeled on — must itself pass: its sequential
/// multiply's select is sliced to the bounded `{8{msel…}}` form, so it carries NO replication wider
/// than the bound. A regression that un-bounded it would fail `no_emitter_produces_…` above; this
/// pins the intent directly on the flagship.
#[test]
fn the_gpu_lane_select_is_bounded_by_construction() {
    let src =
        fs::read_to_string(silicon_dir().join("gpu_simt.v")).expect("gpu_simt.v golden exists");
    let widest = replications(&src)
        .into_iter()
        .filter(|(_, x)| !is_constant(x))
        .map(|(n, _)| n)
        .max()
        .unwrap_or(0);
    assert!(
        widest <= ANTENNA_SAFE_FANOUT,
        "gpu_simt.v has a single-bit replication of fan-out {widest} > {ANTENNA_SAFE_FANOUT} — the \
         SequentialFma select is no longer bounded via bounded_fanout_select"
    );
}

/// The guard's discrimination is CORRECT — so it can never pass *vacuously* (a scanner bug that found
/// nothing would silently disarm it). It must FLAG the un-sliced wide select (the GPU-multiply hazard
/// shape) and PASS every safe form: the bounded primitive output, a sign-extend at the bound, a
/// constant fill, and a zero-pad concat (not a replication at all).
#[test]
fn the_guard_flags_a_hazard_and_passes_the_safe_forms() {
    let flag = |rtl: &str| -> Vec<(usize, String)> {
        replications(rtl)
            .into_iter()
            .filter(|(n, x)| *n > ANTENNA_SAFE_FANOUT && !is_constant(x))
            .collect()
    };

    // The un-sliced 64-bit multiply select — MUST be caught.
    assert_eq!(
        flag("wire [63:0] pp = {64{sel}} & data;"),
        vec![(64, "sel".to_string())],
        "the guard must FLAG a `{{64{{sel}}}}` unbounded wide select"
    );

    // Every safe form — none may be flagged.
    for safe in [
        "wire [63:0] pp = {8{msel7_l0}} & mc;", // bounded primitive output: 8 <= 16
        "wire [63:0] x = {16{imm[15]}};", // sign-extend at the proven-safe bound: 16 is not > 16
        "wire [37:0] m = {38{1'b1}};",    // constant fill: no signal fan-out
        "wire [63:0] p = {58'd0, lfsr[18:13]};", // zero-pad concat: not a replication at all
    ] {
        assert!(
            flag(safe).is_empty(),
            "the guard wrongly flagged a safe form: `{safe}`"
        );
    }
}

/// `ok` holds for every input of `module miter` in `src`, by yosys's SAT over every input bit.
fn proves(dir: &Path, src: &str) -> bool {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("miter.v"), src).unwrap();
    let run = std::process::Command::new("yosys")
        .args(["-p", "read_verilog miter.v; hierarchy -top miter; proc; flatten; opt; sat -prove ok 1 -verify miter"])
        .current_dir(dir)
        .output()
        .expect("yosys");
    run.status.success()
}

/// The rewrites this guard asked of the shipped RTL compute what they replaced, over every input. Each of the FPU's
/// incrementers carries `cin` into its parallel-prefix scan as the scan's bit 0, and is `v + cin`; the engine's
/// signed-multiply overflow test reads `{hi, sign}` as neither all ones nor all zeros, and is `hi != {64{sign}}`.
#[test]
fn the_rewritten_selects_compute_what_they_replaced() {
    if !common::require_tool("yosys") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("antenna_rewrites_{}", std::process::id()));
    let fpu = logicaffeine_compile::codegen_sva::wprog_engine_rtl::FPU_V;
    let incs: Vec<&str> = fpu
        .match_indices("function [63:0] inc64(")
        .map(|(at, _)| &fpu[at..at + fpu[at..].find("endfunction").expect("an incrementer ends") + "endfunction".len()])
        .collect();
    assert_eq!(incs.len(), 5, "the FPU's five modules each define their incrementer");
    for (k, inc) in incs.iter().enumerate() {
        let miter = format!(
            "module miter(input [63:0] v, input cin, output ok);\n  {inc}\n  assign ok = inc64(v, cin) == v + {{63'd0, cin}};\nendmodule\n"
        );
        assert!(proves(&dir.join(format!("inc{k}")), &miter), "incrementer {k} of the FPU is v + cin");
        let wrong = miter.replace("v + {63'd0, cin}", "v + {63'd0, cin} + 64'd1");
        assert!(!proves(&dir.join(format!("inc{k}_wrong")), &wrong), "the proof refuses a wrong sum");
    }
    let wpe = fs::read_to_string(silicon_dir().join("wpe_user_project.v")).expect("wpe_user_project.v");
    let ovf = "((&{mu_hi, mu_prod[63]}) || !(|{mu_hi, mu_prod[63]})) ? 64'sd0 : 64'sd1";
    assert!(wpe.contains(ovf), "the engine's overflow test is the reduction form");
    let miter = format!(
        "module miter(input [63:0] mu_hi, input [127:0] mu_prod, output ok);\n  \
         assign ok = ({ovf}) == ((mu_hi != {{64{{mu_prod[63]}}}}) ? 64'sd1 : 64'sd0);\nendmodule\n"
    );
    assert!(proves(&dir.join("ovf"), &miter), "the overflow test is the sign-extension compare");
    let _ = fs::remove_dir_all(&dir);
}

