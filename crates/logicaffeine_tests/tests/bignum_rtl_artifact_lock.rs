//! bignum_rtl_artifact_lock — the shipped bigint RTL must be what the compiler emits, and it
//! must still compute `a * b`.
//!
//! WHY THIS EXISTS. `benchmarks/silicon/bigint_mul.v` is a GENERATED artifact:
//! `codegen_sva::bignum_rtl::emit_bigint_mul_verilog()` is its source of truth. Nothing called
//! that function and nothing compared the two, so the file could drift from the compiler
//! indefinitely and the next regeneration would silently revert whatever the file had gained.
//! That is the same defect class as `rtl_provenance_lock` guards between the repo and the
//! OpenLane build directories, one level further up the chain.
//!
//! WHAT THE RTL CHANGED TO, AND WHY IT IS A SILICON DECISION. The body was `assign prod = a * b`,
//! which lets synthesis build a flat 64x64 array with 128-bit-wide partial-product nets spanning
//! the block. Those are an antenna source on sky130, and coproc_core's residual antenna
//! violations could not be repaired by any downstream mechanism: all 7 violating pins ALREADY
//! carried a diode, and the jog router cannot bridge a net TritonRoute has patched with a
//! via-enclosure RECT (odb exposes no way to round-trip that geometry from Python — checked
//! three ways: `getRect` needs out-params, there are no SWIG pointer helpers, and
//! `dbWireShapeItr` is not bound). The fix had to move up to the RTL, and the narrow
//! four-32x32 decomposition keeps every intermediate at <= 65 bits.
//!
//! The arithmetic is therefore load-bearing and is verified here rather than asserted.

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The checked-in artifact must be byte-identical to what the compiler emits.
#[test]
fn the_shipped_bigint_mul_is_exactly_what_the_compiler_emits() {
    let emitted = logicaffeine_compile::codegen_sva::bignum_rtl::emit_bigint_mul_verilog();
    let path = root().join("benchmarks/silicon/bigint_mul.v");
    let shipped = std::fs::read_to_string(&path).expect("read benchmarks/silicon/bigint_mul.v");
    assert_eq!(
        shipped, emitted,
        "benchmarks/silicon/bigint_mul.v has drifted from emit_bigint_mul_verilog(). This file \
         is GENERATED: whichever one is right, they must agree, or the next regeneration \
         silently discards the difference — and this artifact is synthesised into coproc_core's \
         silicon."
    );
}

/// The narrow decomposition must still be a 64x64 multiply.
///
/// A carry bug in a multiplier is exactly the defect a handful of directed vectors misses, so
/// this runs the corners AND a large random sweep against Verilog's own `*`.
#[test]
fn the_narrow_multiply_is_bit_identical_to_a_times_b() {
    let Ok(iv) = which_iverilog() else {
        panic!(
            "iverilog is required to verify the bigint_mul arithmetic. This lock guards a \
             hand-decomposed multiplier that is synthesised into silicon; skipping it silently \
             would be worse than not having it."
        )
    };
    let dir = std::env::temp_dir().join("logos_bigint_mul_lock");
    let _ = std::fs::create_dir_all(&dir);
    let rtl = dir.join("bigint_mul.v");
    std::fs::write(
        &rtl,
        logicaffeine_compile::codegen_sva::bignum_rtl::emit_bigint_mul_verilog(),
    )
    .expect("write rtl");

    let tb = dir.join("tb.v");
    std::fs::write(&tb, TB).expect("write tb");
    let exe = dir.join("tb.out");
    let build = std::process::Command::new(&iv)
        .args([
            "-o",
            exe.to_str().unwrap(),
            tb.to_str().unwrap(),
            rtl.to_str().unwrap(),
        ])
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog failed to compile the emitted bigint_mul:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new(&exe).output().expect("run tb");
    let log = String::from_utf8_lossy(&run.stdout).to_string();
    assert!(
        log.contains("PASS:"),
        "the emitted bigint_mul does not compute a*b:\n{log}"
    );
}

fn which_iverilog() -> Result<String, ()> {
    let out = std::process::Command::new("which")
        .arg("iverilog")
        .output()
        .map_err(|_| ())?;
    if !out.status.success() {
        return Err(());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

const TB: &str = r#"`timescale 1ns/1ps
module tb;
  reg [63:0] a, b; wire [127:0] prod; integer i, errors; reg [127:0] want;
  bigint_mul dut(.a(a), .b(b), .prod(prod));
  initial begin
    errors = 0;
    for (i = 0; i < 16; i = i + 1) begin
      case (i[3:2]) 0: a=64'd0; 1: a=64'd1; 2: a=64'hFFFFFFFFFFFFFFFF; 3: a=64'h00000000FFFFFFFF; endcase
      case (i[1:0]) 0: b=64'd0; 1: b=64'd1; 2: b=64'hFFFFFFFFFFFFFFFF; 3: b=64'hFFFFFFFF00000000; endcase
      #1; want = a * b;
      if (prod !== want) begin errors=errors+1; $display("FAIL corner %0d: %h * %h", i, a, b); end
    end
    for (i = 0; i < 100000; i = i + 1) begin
      a = {$random, $random}; b = {$random, $random}; #1; want = a * b;
      if (prod !== want) begin
        errors = errors + 1;
        if (errors < 5) $display("FAIL rand %0d: %h * %h = %h want %h", i, a, b, prod, want);
      end
    end
    if (errors == 0) $display("PASS: narrow 64x64 == a*b over 16 corners + 100000 random vectors");
    else $display("FAIL: %0d mismatches", errors);
    $finish;
  end
endmodule
"#;
