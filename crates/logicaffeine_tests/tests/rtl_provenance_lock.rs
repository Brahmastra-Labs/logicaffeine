//! rtl_provenance_lock — a hardened block's silicon must come from the RTL that is in the repo.
//!
//! WHY THIS EXISTS. `scripts/openlane-run.sh` stages RTL by copying `benchmarks/silicon/<f>.v`
//! into `tools/OpenLane/designs/logos_<block>/src/`, and the flow synthesises `dir::src/*.v`.
//! So `src/` holds exactly what the LAST run compiled, and the signoff ledger describes that
//! same last run. That makes one invariant checkable with no re-run at all:
//!
//!     staged RTL != canonical RTL  =>  the recorded GDS was built from source the repo no
//!                                      longer contains.
//!
//! Nothing in the flow checked it. No run directory keeps a copy of its RTL and no ledger entry
//! carries an RTL hash, so a block could drift arbitrarily far from its source and still read as
//! signoff-clean. This campaign has already been burned by precisely that: `caravel-macro-harden.sh`
//! re-synthesised a stale gitignored file and produced a byte-identical GDS, and a fix that was
//! believed to be in silicon was not. That script now stages every run and records
//! `logos_rtl.sha256`; this lock is the same discipline for every other block.
//!
//! MEASURED when this was written — five staged files had drifted, three of them in blocks
//! recorded signoff-clean:
//!     logos_dram_executor_wb/src/dram_executor_wb.v   4013 vs 5526 bytes
//!     logos_soft_core_scalar/src/soft_core_scalar.v   5388 vs 6412
//!     logos_soft_core_scalar/src/alu.v                2578 vs 3461
//!     logos_nexys_soc_scalar/src/soft_core_scalar.v   6311 vs 6412
//!     logos_nexys_soc_scalar/src/alu.v                2578 vs 3461
//! `dram_executor_wb` is stale against the issue-counter fix that replaced its sticky `issued_c`
//! flag — the exact change that had to reach silicon for firmware to sequence commands post-fab.
//!
//! The remedy is always to RE-RUN the block, never to copy the stale file forward.
//!
//! The staged designs live in the machine-local EDA tree, which a clone does not carry: each test that reads
//! it asks `common::require_tool("eda-tools")` first, so a clone records a counted skip and a gate that
//! requires the tree fails loud without it.

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// RTL that legitimately comes from a vendored dependency rather than our own source tree, by its path under
/// `tools/`. Each is pinned to its origin and checked BYTE-FOR-BYTE, so this is a statement of fact about
/// where a file came from, not a licence to differ.
const VENDORED: &[(&str, &str)] = &[("defines.v", "caravel_user_project/verilog/rtl/defines.v")];

/// The `logos_*` designs under `dir` that stage RTL in `src/`.
fn designs(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = p
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if name.starts_with("logos_") && p.join("src").is_dir() {
            out.push(p);
        }
    }
    out.sort();
    out
}

/// `.v`, `.hex` and `.mem`. A `$readmemh`/`$readmemb` ROM image is compiled into the mask exactly as the RTL
/// is, so a drifted `program.hex` would tape out a different program with no gate noticing.
fn staged_rtl(design: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(design.join("src")) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        match p.extension().and_then(|s| s.to_str()) {
            Some("v") | Some("hex") | Some("mem") => out.push(p),
            _ => {}
        }
    }
    out.sort();
    out
}

/// The lines of `scripts/openlane-run.sh` that stage a block's boot ROM: a block with a ROM of its own
/// ships it as `benchmarks/silicon/<block>.hex` and it is staged as `src/program.hex`, the name the
/// core's `$readmemh` reads; every other block stages the shared `program.hex` under its own name.
const ROM_STAGING: [&str; 4] = [
    r#"if [ -f "$SIL/$BLOCK.hex" ]; then"#,
    r#"  cp "$SIL/$BLOCK.hex" "$OL/designs/$DES/src/program.hex""#,
    r#"elif [ -f "$SIL/program.hex" ]; then"#,
    r#"  cp "$SIL/program.hex" "$OL/designs/$DES/src/""#,
];

/// The repo file a staged file of design `logos_<block>` was copied from, by the staging rule above. A
/// comparative-synthesis block — `qorsyn_<candidate>_p<period>`, a candidate in its fold wrapper at one clock, or
/// `qorpwr_<candidate>_p<period>`, a candidate in its functional harness — stages everything from its own directory,
/// `benchmarks/silicon/qor/sky130/<block>/`, which `scripts/qor/run-qor-sky130.py` fills from the candidate manifest
/// and records in `qor_sky130_stage.json` — third-party RTL never sits at the top of `benchmarks/silicon`.
fn repo_source(design: &str, staged: &str) -> PathBuf {
    let silicon = root().join("benchmarks/silicon");
    let block = design.strip_prefix("logos_").unwrap_or(design);
    if ["qorsyn_", "qorpwr_"].iter().any(|p| block.starts_with(p)) {
        return silicon.join("qor/sky130").join(block).join(staged);
    }
    let own_rom = silicon.join(format!("{block}.hex"));
    if staged == "program.hex" && own_rom.is_file() {
        own_rom
    } else {
        silicon.join(staged)
    }
}

/// Every staged file is traced back by one rule: a block's own ROM, a comparative block's staging directory,
/// otherwise its name at the top of `benchmarks/silicon` — the rule the drift and orphan checks both read by.
#[test]
fn a_staged_file_is_traced_to_its_one_repo_source() {
    let silicon = root().join("benchmarks/silicon");
    assert_eq!(repo_source("logos_alu", "alu.v"), silicon.join("alu.v"));
    assert_eq!(
        repo_source("logos_qorsyn_k1_calyx_dahlia_p12_5", "main.v"),
        silicon.join("qor/sky130/qorsyn_k1_calyx_dahlia_p12_5/main.v")
    );
    assert_eq!(
        repo_source("logos_qorsyn_k4_bambu_fsm_ports_a7p10_p20", "array_ref_7.mem"),
        silicon.join("qor/sky130/qorsyn_k4_bambu_fsm_ports_a7p10_p20/array_ref_7.mem")
    );
    assert_eq!(
        repo_source("logos_qorpwr_k5_logos_p7_5", "qorpwr_k5_logos_p7_5.v"),
        silicon.join("qor/sky130/qorpwr_k5_logos_p7_5/qorpwr_k5_logos_p7_5.v")
    );
}

/// The staging rule this lock reads by is the one the flow runs: if `openlane-run.sh` stages a ROM any
/// other way, this goes red until `repo_source` says the same.
#[test]
fn the_rom_staging_rule_is_the_flows() {
    let script =
        std::fs::read_to_string(root().join("scripts/openlane-run.sh")).expect("openlane-run.sh");
    let lines: Vec<&str> = script.lines().collect();
    let at = lines
        .windows(ROM_STAGING.len())
        .position(|w| w == ROM_STAGING)
        .unwrap_or_else(|| {
            panic!(
                "openlane-run.sh no longer stages boot ROMs as:\n{}",
                ROM_STAGING.join("\n")
            )
        });
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.contains("/src/program.hex"))
            .count(),
        1,
        "exactly one line of openlane-run.sh may stage a file as src/program.hex (line {})",
        at + 2
    );
}

/// The staged copy and the repo copy it was staged from must be the same bytes.
#[test]
fn every_staged_rtl_file_is_byte_identical_to_the_repo_copy() {
    if !common::require_tool("eda-tools") {
        return;
    }
    let mut drift = Vec::new();
    let mut compared = 0;
    for d in designs(&root().join("tools/OpenLane/designs")) {
        let block = d
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();
        for f in staged_rtl(&d) {
            let name = f
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let canon = repo_source(&block, &name);
            if !canon.is_file() {
                continue; // origin is the other test's business
            }
            compared += 1;
            let (a, b) = (std::fs::read(&f), std::fs::read(&canon));
            match (a, b) {
                (Ok(a), Ok(b)) if a == b => {}
                (Ok(a), Ok(b)) => drift.push(format!(
                    "  {block}/src/{name}: staged {} bytes, {} {} bytes",
                    a.len(),
                    canon.strip_prefix(root()).unwrap_or(&canon).display(),
                    b.len()
                )),
                _ => drift.push(format!("  {block}/src/{name}: unreadable")),
            }
        }
    }
    assert!(
        compared >= 33,
        "only {compared} staged RTL files compared — the design scan is not working, so this \
         lock would pass vacuously"
    );
    assert!(
        drift.is_empty(),
        "{} staged RTL file(s) differ from the repo copy. `src/` is what the LAST run \
         synthesised and the ledger describes that run, so each of these means a recorded GDS \
         was built from source the repo no longer contains.\n{}\n\nRe-run the affected blocks. \
         Do NOT copy the stale file back over the canonical one — that would bury the change \
         the repo copy is carrying.",
        drift.len(),
        drift.join("\n")
    );
}

/// No design may synthesise RTL that exists nowhere else in the tree.
///
/// The flow globs `dir::src/*.v`, so a file left in a build directory is compiled on every run
/// while being invisible to the repo. `hierarchy -top` prunes it if it is unreferenced, which
/// makes it harmless AND undetectable — until the day one of them shadows a real module.
#[test]
fn no_design_synthesises_rtl_that_exists_only_in_its_build_directory() {
    if !common::require_tool("eda-tools") {
        return;
    }
    let tools = root().join("tools");
    let mut orphans = Vec::new();
    for d in designs(&tools.join("OpenLane/designs")) {
        let block = d
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();
        for f in staged_rtl(&d) {
            let name = f
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if repo_source(&block, &name).is_file() {
                continue;
            }
            match VENDORED.iter().find(|(n, _)| *n == name) {
                Some((_, origin)) => {
                    let o = tools.join(origin);
                    let same =
                        matches!((std::fs::read(&f), std::fs::read(&o)), (Ok(a), Ok(b)) if a == b);
                    assert!(
                        same,
                        "{block}/src/{name} is declared vendored from {origin} but does not \
                         match it byte-for-byte. A vendored file that has been edited in place \
                         is an orphan wearing a label."
                    );
                }
                None => orphans.push(format!("  {block}/src/{name}")),
            }
        }
    }
    assert!(
        orphans.is_empty(),
        "{} RTL file(s) are synthesised from a build directory and exist nowhere else:\n{}\n\n\
         Either move the file into benchmarks/silicon (it is real source) or delete it (it is \
         a leftover). Add a VENDORED entry only when it genuinely comes from a dependency.",
        orphans.len(),
        orphans.join("\n")
    );
}
