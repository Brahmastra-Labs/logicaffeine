//! caravel_bringup_lock — the chip we would FABRICATE is provably usable after it comes back.
//!
//! Signoff answers "can this be manufactured". It does not answer "will the silicon do anything
//! we can observe", and those are different questions with different failure modes. A wrapper can
//! pass all nineteen `mpw_precheck` gates — DRC, LVS, antenna, XOR, density — while its user macro
//! is wired to nothing, exposes no readable state, or disagrees with the pad configuration. LVS
//! cannot catch that: it proves the layout matches the netlist, including when both encode the
//! same mistake.
//!
//! WHAT THIS FOUND. Before this lock existed there was verification for `dram_executor_wb` (an
//! INNER module) and nothing at all for `user_project_wrapper` (the TOP CELL that is actually
//! taped out). Between them sit the full-interface macro and the wrapper's 1:1 pin mapping. The
//! first run of the top-cell testbench found a real defect: `issued_c` is sticky — set on issue
//! and cleared only by reset — so firmware polling the status register reads the PREVIOUS
//! command's `issued=1` immediately and cannot tell that its own command has gone out. The user
//! area's only post-fab observation path is the management SoC reading those registers, so that
//! makes command sequencing on real silicon unreliable. See the DEFECT note at the foot of
//! `benchmarks/silicon/caravel/dram/verilog/dv/logos_bringup/tb_user_project_wrapper.v` for the fix (an issue
//! COUNTER, not a flag).
//!
//! Also locked here: the submission must carry verification that names the macro it actually
//! contains. Every `verilog/dv/` testbench in the shipped submission targets `user_proj_example`
//! — the efabless scaffold counter — and not one file mentions `dram_user_project`. A bring-up
//! suite that tests a design which is not on the die is worse than none: it fails on good silicon.

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::caravel_project::DRAM_PROJECT;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Is this a machine that HAS produced the tapeout artifacts?
///
/// The assembled submission is ignored regenerated output, so a fresh clone legitimately has none and
/// these checks have nothing to inspect. But the OpenLane run directories are the fingerprint of
/// the box that hardened the wrapper — if they exist, the submission can be produced here, and
/// its ABSENCE is then a missing deliverable rather than a different environment.
///
/// This is the anti-cheat hinge. Every one of these tests used to open with
/// `if !path.exists() { return; }`, which means deleting the submission turned the entire
/// submission-integrity suite green. That is not a hypothetical failure mode; it is the single
/// easiest way to fake a passing tapeout, and it was available in code I wrote.
fn is_tapeout_box() -> bool {
    root()
        .join("tools/OpenLane/designs/logos_upw/runs")
        .is_dir()
}

/// Read a file that MUST be there, or fail with the reason it must.
fn must_read(p: &Path, why: &str) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("cannot read {}: {e}\n{why}", p.display()))
}

/// Demand an external tool. NOT gated on `LOGOS_REQUIRE_TOOLS`: the RTL is checked in, so the
/// simulation that proves the fabricated design responds can always be run, and "the simulator
/// was missing" is not a reason to call a chip verified.
fn demand_tool(tool: &str) {
    let ok = std::process::Command::new(tool)
        .arg("-V")
        .output()
        .map(|o| o.status.success() || !o.stdout.is_empty())
        .unwrap_or(false);
    assert!(
        ok,
        "`{tool}` is not available, and this gate proves the chip we would FABRICATE responds \
         over its only post-fab interface. A missing simulator is not a passing chip — install \
         iverilog or do not claim the design is verified."
    );
}

/// The TOP CELL runs, over Wishbone, exactly as the management SoC would drive it after fab.
///
/// Three commands including a non-zero countdown delta and the maximum op/bank/row, each checked
/// on BOTH observation paths — `la_data_out` and the `0x0c`/`0x10` readback registers — plus the
/// GPIO contract (`io_oeb` all-high, `io_out` low) that `user_defines.v` is written to match.
#[test]
fn the_fabricated_top_cell_responds_over_wishbone() {
    demand_tool("iverilog");
    let sil = root().join("benchmarks/silicon");
    let dir = std::env::temp_dir().join("logos_caravel_bringup");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let out = dir.join("tb_upw.out");

    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&out)
        .arg(sil.join("caravel/dram/verilog/dv/logos_bringup/tb_user_project_wrapper.v"))
        .arg(sil.join("user_project_wrapper.v"))
        .arg(sil.join("dram_user_project.v"))
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog could not build the top-cell bring-up testbench:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let run = std::process::Command::new("vvp")
        .arg(&out)
        .output()
        .expect("run vvp");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        log.contains("PASS: user_project_wrapper bring-up"),
        "the TOP CELL we would fabricate did not pass bring-up. This is the design that goes on \
         silicon — an inner-module test passing does not cover the macro's full interface or the \
         wrapper's pin mapping.\n{log}"
    );
    assert!(
        !log.contains("FAIL:"),
        "top-cell bring-up reported failures:\n{log}"
    );
}

/// The submission's bring-up suite must name the macro the submission actually contains.
///
/// This is the gap that motivated the file. The assembled submission's `verilog/dv/` shipped the
/// efabless scaffold tests, every one of which drives `user_proj_example`. If that submission were
/// fabricated, the bring-up firmware would exercise a counter that is not on the die, fail, and
/// give no way to distinguish broken silicon from a wrong test.
#[test]
fn the_submission_verifies_the_macro_it_actually_contains() {
    if !common::require_tool("eda-tools") {
        return;
    }
    let sub = root().join(DRAM_PROJECT.submission_dir);
    if !sub.is_dir() {
        assert!(
            !is_tapeout_box(),
            "this machine has hardened the wrapper (tools/OpenLane/designs/logos_upw/runs exists) \
             but its assembled submission is MISSING. Deleting the submission must never turn this \
             suite green — re-run scripts/caravel-submission.sh."
        );
        return; // a clean checkout genuinely has nothing to inspect
    }
    let ledger = root().join("benchmarks/silicon/caravel_signoff.json");
    let raw = must_read(
        &ledger,
        "this ledger is CHECKED IN — it is the tapeout claim itself. Its absence can never be a \
         reason for this gate to pass.",
    );
    let doc: serde_json::Value = serde_json::from_str(&raw).expect("caravel_signoff.json");
    let macro_name = doc["user_project_wrapper"]["macro"]
        .as_str()
        .expect("the ledger records which macro is in the wrapper")
        .to_string();

    let dv = sub.join("verilog/dv");
    if !dv.is_dir() {
        return;
    }
    let mut named = Vec::new();
    let mut scaffold_only = Vec::new();
    let mut stack = vec![dv.clone()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let Ok(body) = std::fs::read_to_string(&p) else {
                continue;
            };
            if body.contains(&macro_name) {
                named.push(p.clone());
            } else if body.contains("user_proj_example") {
                scaffold_only.push(p.strip_prefix(&sub).unwrap_or(&p).display().to_string());
            }
        }
    }
    assert!(
        !named.is_empty(),
        "the assembled submission carries {} bring-up file(s) that drive the SCAFFOLD design \
         `user_proj_example`, and NOT ONE that names `{macro_name}` — the macro the wrapper \
         actually instantiates and that would be on the die.\n  {}\n\nA bring-up suite testing a \
         design that is not fabricated fails on good silicon and cannot distinguish a broken chip \
         from a wrong test. Port a test onto the real register map (see \
         benchmarks/silicon/caravel/dram/verilog/dv/logos_bringup/tb_user_project_wrapper.v) before submitting.",
        scaffold_only.len(),
        scaffold_only.join("\n  ")
    );
}

/// The GPIO power-on configuration must agree with what the macro drives.
///
/// `user_defines.v` decides how each pad comes up on real silicon; the RTL decides whether the
/// user area drives it. If they disagree — the file says output while the macro ties `io_oeb`
/// high, or the reverse — a fabricated pad either fights its driver or floats. Nothing in the
/// signoff flow compares the two: `mpw_precheck`'s GPIO-Defines check validates the file's syntax
/// and that the placeholders were replaced, not that it matches the design.
#[test]
fn the_pad_configuration_agrees_with_what_the_macro_drives() {
    let sil = root().join("benchmarks/silicon");
    let rtl = must_read(
        &sil.join("dram_user_project.v"),
        "the user macro's RTL is CHECKED IN. If it is gone, nothing here describes a real chip.",
    );
    let drives_nothing =
        rtl.contains("assign io_out   = 38'd0;") || rtl.contains("assign io_out = 38'd0;");
    let all_inputs = rtl.contains("assign io_oeb   = {38{1'b1}};")
        || rtl.contains("assign io_oeb = {38{1'b1}};");
    assert!(
        drives_nothing && all_inputs,
        "dram_user_project no longer ties io_out low and io_oeb high. The pad configuration in \
         the submission's verilog/rtl/user_defines.v is written for an all-inputs design; \
         if the macro now drives a pad, that file must be updated in the same change or the \
         fabricated pad will fight the pad driver."
    );

    if !common::require_tool("eda-tools") {
        return;
    }
    let defs_path = root()
        .join(DRAM_PROJECT.submission_dir)
        .join("verilog/rtl/user_defines.v");
    if !defs_path.is_file() {
        assert!(
            !is_tapeout_box(),
            "the submission carries no verilog/rtl/user_defines.v — the file that decides how \
             every pad powers up on real silicon. Missing must not read as agreeing."
        );
        return;
    }
    let defs = must_read(
        &defs_path,
        "pad configuration is a fabrication-time contract",
    );
    assert!(
        !defs.contains("GPIO_MODE_INVALID\n") || defs.matches("GPIO_MODE_INVALID").count() <= 2,
        "user_defines.v still assigns the scaffold's GPIO_MODE_INVALID placeholder to a pad — \
         that value is deliberately erroneous and fails the shuttle precheck"
    );
    let outputs = defs.matches("USER_STD_OUTPUT").count() + defs.matches("OUT_MONITORED").count();
    assert!(
        outputs <= 2,
        "user_defines.v configures {outputs} pads as OUTPUTS, but the macro drives none \
         (io_oeb is tied all-high). A pad configured as an output that nothing drives floats on \
         silicon."
    );
}

/// RATCHET: the post-fab handshake may never regress to a sticky flag.
///
/// The behavioural proof is `the_fabricated_top_cell_responds_over_wishbone` — that testbench
/// polls the counter and was verified to FAIL with six errors against the pre-fix RTL, so it
/// cannot pass if the counter goes away. This adds the structural guard on top, because a
/// behavioural failure says "stale fields" while this says why.
///
/// `issue_count` must be ticked on issue and readable on BOTH observation paths, since the
/// management SoC uses the Wishbone one and a bench probe uses the logic-analyser one.
#[test]
fn the_issue_counter_handshake_cannot_regress_to_a_sticky_flag() {
    let sil = root().join("benchmarks/silicon");
    for f in ["dram_executor_wb.v", "dram_user_project.v"] {
        let rtl = match std::fs::read_to_string(sil.join(f)) {
            Ok(r) => r,
            Err(_) => continue,
        };
        assert!(
            rtl.contains("issue_count <= issue_count + 8'd1"),
            "{f}: `issue_count` is no longer incremented on issue. Without it firmware polls the \
             status register and reads the PREVIOUS command's `issued = 1`, so it cannot tell its \
             own command went out — and two identical back-to-back commands become \
             indistinguishable. The management SoC reading these registers is the user area's \
             ONLY post-fab observation path."
        );
        assert!(
            rtl.contains("{issued_c, 15'd0, issue_count, bank_c, op_c}"),
            "{f}: the 0x0c status register no longer exposes `issue_count` at bits [15:8] — that \
             is the address firmware polls"
        );
        assert!(
            rtl.contains("{issued_c, 15'd0, issue_count, bank_c, op_c, row_c}"),
            "{f}: la_data_out no longer exposes `issue_count` at bits [47:40]"
        );
        assert!(
            !rtl.contains("{issued_c, 23'd0, bank_c, op_c}"),
            "{f}: the pre-fix sticky-flag status layout is back"
        );
    }
}

/// RATCHET: the two copies of `dram_executor_wb` stay byte-identical.
///
/// The module is duplicated — `dram_executor_wb.v` for simulation, and inlined in
/// `dram_user_project.v` so OpenLane can be handed one self-contained file. Both define a module
/// of the same name, so a fix applied to one and not the other produces two different designs
/// that are indistinguishable by name: the testbench would verify one while the flow fabricates
/// the other.
#[test]
fn the_duplicated_executor_rtl_cannot_diverge() {
    let sil = root().join("benchmarks/silicon");
    let extract = |p: PathBuf| -> Option<String> {
        let s = std::fs::read_to_string(p).ok()?;
        let a = s.find("module dram_executor_wb")?;
        let b = s[a..].find("endmodule")? + a + "endmodule".len();
        Some(s[a..b].to_string())
    };
    let (Some(x), Some(y)) = (
        extract(sil.join("dram_executor_wb.v")),
        extract(sil.join("dram_user_project.v")),
    ) else {
        return;
    };
    assert_eq!(
        x, y,
        "the two copies of `dram_executor_wb` have diverged. They define the same module name, so \
         whichever is compiled first wins and the other is silently ignored — the simulation and \
         the fabricated netlist can end up being different designs. Apply every change to both, \
         or collapse them into one file."
    );
}

/// RATCHET: the inner-module testbench must actually RUN.
///
/// `tb_dram_executor_wb.v` existed for months and no test ever executed it, which is precisely
/// why the sticky-flag defect survived: verification that is never run is documentation.
#[test]
fn the_inner_module_testbench_still_passes() {
    demand_tool("iverilog");
    let sil = root().join("benchmarks/silicon");
    let dir = std::env::temp_dir().join("logos_dram_wb_tb");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let out = dir.join("tb_wb.out");
    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&out)
        .arg(sil.join("tb_dram_executor_wb.v"))
        .arg(sil.join("dram_executor_wb.v"))
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog could not build tb_dram_executor_wb:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&out)
        .output()
        .expect("run vvp");
    let log = String::from_utf8_lossy(&run.stdout).to_string();
    assert!(
        log.contains("PASS:"),
        "tb_dram_executor_wb did not pass:\n{log}"
    );
}

/// RATCHET: the submission's netlist manifest must DEFINE the module the wrapper instantiates.
///
/// `uprj_netlists.v` is what every simulation of the submission elaborates. The scaffold's copy
/// includes `user_proj_example` in both the GL and RTL branches; our wrapper instantiates
/// `dram_user_project`, which appeared in neither — so the top cell's macro was an undefined
/// module in both modes, while its gate-level netlist sat unreferenced in `gl/`. The GDS is what
/// gets fabricated, so this does not stop a tapeout; it stops anyone from ever simulating what
/// they taped out, which is worse.
#[test]
fn the_submission_manifest_defines_the_instantiated_macro() {
    if !common::require_tool("eda-tools") {
        return;
    }
    let sub = root().join(DRAM_PROJECT.submission_dir);
    let manifest = sub.join("verilog/rtl/uprj_netlists.v");
    if !manifest.is_file() {
        assert!(
            !is_tapeout_box(),
            "the submission exists but carries no verilog/rtl/uprj_netlists.v — the netlist \
             manifest every simulation elaborates. Its absence must not read as a pass."
        );
        return;
    }
    let ledger = root().join("benchmarks/silicon/caravel_signoff.json");
    let Ok(raw) = std::fs::read_to_string(&ledger) else {
        return;
    };
    let doc: serde_json::Value = serde_json::from_str(&raw).expect("caravel_signoff.json");
    let mac = doc["user_project_wrapper"]["macro"]
        .as_str()
        .expect("recorded macro")
        .to_string();

    let body = std::fs::read_to_string(&manifest).expect("read uprj_netlists.v");
    let body_incl = body
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("`include"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        body_incl.contains(&format!("gl/{mac}.v")),
        "uprj_netlists.v does not include `gl/{mac}.v`, so the GL branch leaves the module the \
         wrapper instantiates UNDEFINED.\n{body}"
    );
    assert!(
        body_incl.contains(&format!("\"{mac}.v\"")),
        "uprj_netlists.v does not include `{mac}.v`, so the RTL branch leaves the module the \
         wrapper instantiates UNDEFINED.\n{body}"
    );
    // Check the INCLUDE DIRECTIVES, not the file text. The manifest's own header explains why the
    // scaffold's `user_proj_example` was wrong here, so a whole-file substring search flags the
    // explanation as if it were the defect — which it did on the first run of this lock against a
    // correctly rebuilt submission.
    let included: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("`include"))
        .collect();
    assert!(
        !included.iter().any(|l| l.contains("user_proj_example")),
        "uprj_netlists.v still INCLUDES the scaffold's `user_proj_example`, which this project \
         does not contain:\n  {}",
        included.join("\n  ")
    );
    for branch in [format!("gl/{mac}.v"), format!("{mac}.v")] {
        let f = sub.join("verilog").join(if branch.starts_with("gl/") {
            branch.clone()
        } else {
            format!("rtl/{branch}")
        });
        assert!(
            f.is_file(),
            "uprj_netlists.v includes `{branch}` but {} is not in the submission",
            f.display()
        );
    }
}

/// RATCHET: the SHIPPED artifacts must have been built from the CURRENT RTL.
///
/// This is the one that would have caught the worst failure of the campaign. The macro's harden
/// script only checked that `designs/logos_upm/src/dram_user_project.v` existed — it never staged
/// the file from `benchmarks/silicon`. So after the issue-counter fix was applied to the real RTL,
/// the macro was re-hardened, re-synthesised the STALE file still sitting in that gitignored
/// working directory, and produced a byte-identical GDS (488,435 cells, 55,751,802 bytes — the
/// same as the build before it). The wrapper was hardened around it, recorded as a fresh signoff,
/// and the submission rebuilt from it. Every downstream check passed, because each asked whether
/// an artifact EXISTED rather than whether it matched its source. With the RTL actually staged the
/// macro comes out at 488,594 cells / 56,105,458 bytes — the counter, its incrementer and the
/// readback muxing, +159 cells.
///
/// A byte-identical rebuild after a source change is not reassuring; it is the symptom.
///
/// THE CHECK IS A HASH, NOT A NAME SEARCH. The first version of this ratchet required every
/// register the RTL declares to appear in the gate-level netlist, and that is simply false:
/// measured on this design, yosys keeps `cmd_word_r` (192 refs) and `cmd_valid_r` (4) while
/// renaming `op_c`, `bank_c`, `row_c`, `issued_c` and `issue_count` out of existence. That
/// version blocked a CORRECT build. Synthesis may rename anything it likes; the hash of the
/// input it consumed is the only thing renaming cannot confuse, so the harden records it
/// (`logos_rtl.sha256`) and the submission ships it.
#[test]
fn the_shipped_artifacts_were_built_from_the_current_rtl() {
    if !common::require_tool("eda-tools") {
        return;
    }
    let sil = root().join("benchmarks/silicon/dram_user_project.v");
    let shipped = root()
        .join(DRAM_PROJECT.submission_dir)
        .join("logos_rtl.sha256");
    if !sil.is_file() || !shipped.is_file() {
        assert!(
            !is_tapeout_box(),
            "the RTL or the shipped logos_rtl.sha256 is missing on a machine that has hardened \
             this design. Provenance cannot be proven, so the tapeout claim is unsupported — \
             re-run scripts/caravel-macro-harden.sh then scripts/caravel-submission.sh."
        );
        return;
    }
    let out = std::process::Command::new("sha256sum")
        .arg(&sil)
        .output()
        .expect("run sha256sum");
    let current = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    // The file carries an SPDX header (the shuttle scanner rejects the whole submission over one
    // un-tagged file, including this one), so skip comment lines rather than taking the first
    // token of the first line.
    let body = std::fs::read_to_string(&shipped).expect("read shipped logos_rtl.sha256");
    let recorded = body
        .lines()
        .find(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        .and_then(|l| l.split_whitespace().next())
        .unwrap_or("")
        .to_string();
    assert!(
        !current.is_empty() && !recorded.is_empty(),
        "could not read both hashes"
    );
    assert_eq!(
        recorded, current,
        "the submission was built from DIFFERENT RTL than benchmarks/silicon/dram_user_project.v \
         holds today.\n  shipped artifacts built from: {recorded}\n  current source:              \
         {current}\n\nRe-run scripts/caravel-macro-harden.sh (it stages the RTL from the repo and \
         records this hash), then the wrapper harden, then re-assemble. Do not trust a rebuild \
         that comes back byte-identical after a source change."
    );
}
