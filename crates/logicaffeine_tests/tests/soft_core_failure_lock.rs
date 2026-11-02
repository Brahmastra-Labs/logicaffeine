//! The soft core keeps the failure contract: a program that stops with a reason (`Fail with …`, a
//! contract's `, otherwise …`, the VM's own `FailWith`) prints the same output, then stops as FAILED
//! with the SAME reason — on the behavioral model AND on the real Verilog (`soft_core.v`, iverilog).
//!
//! On the chip a failure streams its reason over the Text port (`ShowText`) and then trap-halts
//! with the `reasoned` flag, so the host reads the last Text record as the reason. A trap that has no
//! text (a failed policy check, an invalid codepoint) halts `failed` without `reasoned`.

mod common;

use logicaffeine_compile::codegen_sva::full_asm::compile_to_full_words;
use logicaffeine_compile::codegen_sva::soft_core_model::{run_soft_core_outcome, ChipFailure};

const FAILURES: &[(&str, &str, &str, &str)] = include!("failure_corpus.in");

fn expected_failure(reason: &str) -> Option<ChipFailure> {
    (!reason.is_empty()).then(|| ChipFailure::Reason(reason.to_string()))
}

#[test]
fn the_failure_corpus_stops_with_the_same_reason_on_the_soft_core_model() {
    let mut diverged = Vec::new();
    for (name, src, expected_out, reason) in FAILURES {
        let words = match compile_to_full_words(src) {
            Ok(w) => w,
            Err(e) => {
                diverged.push(format!("{name}: refused: {e}\n{src}"));
                continue;
            }
        };
        let run = run_soft_core_outcome(&words, 1_000_000);
        let shown: Vec<String> = run.shows.iter().map(|v| v.fmt()).collect();
        if shown.join("\n") != *expected_out || run.failure != expected_failure(reason) {
            diverged.push(format!("{name}: shows {shown:?}, failure {:?}; expected {expected_out:?}, {reason:?}\n{src}", run.failure));
        }
    }
    assert!(
        diverged.is_empty(),
        "{} of {} diverge on the soft-core model:\n\n{}",
        diverged.len(),
        FAILURES.len(),
        diverged.join("\n\n")
    );
}

/// A testbench that prints every Int / Text record the core streams, then how it halted.
fn capture_tb(prog: &[u32]) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "`timescale 1ns/1ps");
    let _ = writeln!(s, "module tb;");
    let _ = writeln!(s, "  reg clk = 0, rst = 1; wire [31:0] out; wire [63:0] outf; wire [31:0] outw; wire [31:0] outs;");
    let _ = writeln!(
        s,
        "  wire ov, ofv, owv, osv, osf, halted, failed, reasoned;"
    );
    let _ = writeln!(s, "  integer left = 0;");
    let _ = writeln!(
        s,
        "  soft_core #(.FRAMES(64), .REGS(256), .IMEM_WORDS(1024), .HEAP(4096))"
    );
    let _ = writeln!(
        s,
        "    dut(.clk(clk), .rst(rst), .out(out), .out_valid(ov), .outf(outf), .outf_valid(ofv),"
    );
    let _ = writeln!(
        s,
        "        .outw(outw), .outw_valid(owv), .outs(outs), .outs_valid(osv), .outs_first(osf),"
    );
    let _ = writeln!(
        s,
        "        .halted(halted), .failed(failed), .reasoned(reasoned));"
    );
    let _ = writeln!(s, "  always #5 clk = ~clk;");
    let _ = writeln!(s, "  always @(posedge clk) begin");
    let _ = writeln!(s, "    if (ov) $display(\"INT %0d\", $signed(out));");
    let _ = writeln!(s, "    if (osv) begin");
    let _ = writeln!(
        s,
        "      if (osf) begin left = outs; $write(\"TEXT \"); if (outs == 0) $display(\"\"); end"
    );
    let _ = writeln!(s, "      else begin $write(\"%c\", outs[7:0]); left = left - 1; if (left == 0) $display(\"\"); end");
    let _ = writeln!(s, "    end");
    let _ = writeln!(s, "  end");
    let _ = writeln!(s, "  initial begin");
    let _ = writeln!(s, "    #1;");
    for (i, word) in prog.iter().enumerate() {
        let _ = writeln!(s, "    dut.imem[{i}] = 32'h{word:08X};");
    }
    let _ = writeln!(s, "    #3 rst = 0;");
    let _ = writeln!(s, "    wait (halted);");
    let _ = writeln!(s, "    #20;");
    let _ = writeln!(
        s,
        "    $display(\"HALT failed=%0d reasoned=%0d\", failed, reasoned);"
    );
    let _ = writeln!(s, "    $finish;");
    let _ = writeln!(s, "  end");
    let _ = writeln!(
        s,
        "  initial begin #50000000 $display(\"TIMEOUT\"); $finish; end"
    );
    let _ = writeln!(s, "endmodule");
    s
}

/// Run `prog` on the real `soft_core.v`: (shown records, failure) decoded exactly as the model reports.
fn run_on_rtl(prog: &[u32], name: &str) -> Result<(Vec<String>, Option<ChipFailure>), String> {
    let sil = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon");
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let tb = dir.path().join(format!("{name}.v"));
    let vvp = dir.path().join(format!("{name}.vvp"));
    std::fs::write(&tb, capture_tb(prog)).map_err(|e| e.to_string())?;
    let mut c = std::process::Command::new("iverilog");
    c.arg("-g2012").arg("-o").arg(&vvp);
    for d in [
        "alu.v",
        "shared_divider.v",
        "crdt_merge.v",
        "bigint_alu.v",
        "bigint_div.v",
        "fpu.v",
        "soft_core.v",
    ] {
        c.arg(sil.join(d));
    }
    c.arg(&tb);
    let built = c.output().map_err(|e| e.to_string())?;
    if !built.status.success() {
        return Err(format!(
            "iverilog: {}",
            String::from_utf8_lossy(&built.stderr)
        ));
    }
    let run = std::process::Command::new("vvp")
        .arg(&vvp)
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&run.stdout);
    let mut records = Vec::new();
    let mut halt = None;
    for line in stdout.lines() {
        if let Some(n) = line.strip_prefix("INT ") {
            records.push(n.trim().to_string());
        } else if let Some(t) = line.strip_prefix("TEXT ") {
            records.push(t.to_string());
        } else if line == "TEXT" {
            records.push(String::new());
        } else if let Some(h) = line.strip_prefix("HALT ") {
            halt = Some(h.to_string());
        } else if line.contains("TIMEOUT") {
            return Err("timeout".into());
        }
    }
    let halt = halt.ok_or_else(|| format!("never halted:\n{stdout}"))?;
    let failure = match halt.as_str() {
        "failed=0 reasoned=0" => None,
        "failed=1 reasoned=1" => Some(ChipFailure::Reason(
            records.pop().ok_or("reasoned with no record")?,
        )),
        "failed=1 reasoned=0" => Some(ChipFailure::Trap),
        other => return Err(format!("impossible halt state {other}")),
    };
    Ok((records, failure))
}

#[test]
fn the_failure_corpus_stops_with_the_same_reason_on_the_soft_core_rtl() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut diverged = Vec::new();
    for (name, src, expected_out, reason) in FAILURES {
        let words = match compile_to_full_words(src) {
            Ok(w) => w,
            Err(e) => {
                diverged.push(format!("{name}: refused: {e}"));
                continue;
            }
        };
        match run_on_rtl(&words, name) {
            Ok((shown, failure)) => {
                if shown.join("\n") != *expected_out || failure != expected_failure(reason) {
                    diverged.push(format!("{name}: shows {shown:?}, failure {failure:?}; expected {expected_out:?}, {reason:?}"));
                }
            }
            Err(e) => diverged.push(format!("{name}: {e}")),
        }
    }
    assert!(
        diverged.is_empty(),
        "{} of {} diverge on the soft-core RTL:\n\n{}",
        diverged.len(),
        FAILURES.len(),
        diverged.join("\n\n")
    );
}

#[test]
fn a_trap_without_text_halts_failed_but_not_reasoned() {
    // `parseInt` of a non-number traps on the chip with no streamed reason: the core reports FAILED,
    // never a reason it does not have.
    let words = compile_to_full_words("## Main\nShow 1.\nLet n be parseInt(\"12x\").\nShow n.\n")
        .expect("assembles");
    let run = run_soft_core_outcome(&words, 1_000_000);
    assert_eq!(run.shows.iter().map(|v| v.fmt()).collect::<Vec<_>>(), ["1"]);
    assert_eq!(run.failure, Some(ChipFailure::Trap));
}
