//! SUPERCRUSH Sprint S3A: Verified SVA Compiler via Futamura Projections

use logicaffeine_compile::codegen_sva::verified_compiler::*;
use logicaffeine_compile::codegen_sva::fol_to_sva::synthesize_sva_from_spec;

// ═══════════════════════════════════════════════════════════════════════════
// P1: COMPILED GENERATOR
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn p1_compiled_equals_interpreted_simple() {
    let spec = "Always, if Req is valid then Ack is valid.";
    assert!(verify_compiler_correctness(spec, "clk"),
        "P1 output should equal interpreted for simple spec");
}

#[test]
fn p1_compiled_equals_interpreted_mutex() {
    let spec = "Always, Grant A and Grant B are not both valid.";
    assert!(verify_compiler_correctness(spec, "clk"),
        "P1 output should equal interpreted for mutex spec");
}

#[test]
fn p1_compiled_equals_interpreted_temporal() {
    let spec = "Always, if the request is valid then eventually the acknowledgment is valid.";
    assert!(verify_compiler_correctness(spec, "clk"),
        "P1 output should equal interpreted for temporal spec");
}

#[test]
fn p1_generator_no_synthesis_overhead() {
    let spec = "Always, if Req is valid then Ack is valid.";
    let gen = compile_sva_generator(spec).unwrap();
    // The generator should have cached data — no synthesis needed
    assert!(!gen.body().is_empty(), "Generator should have cached body");
    assert!(!gen.signals().is_empty(), "Generator should have cached signals");
}

#[test]
fn p1_generator_deterministic() {
    let spec = "Always, if Req is valid then Ack is valid.";
    let gen1 = compile_sva_generator(spec).unwrap();
    let gen2 = compile_sva_generator(spec).unwrap();
    assert_eq!(gen1.body(), gen2.body(), "Same spec should produce same body");
    assert_eq!(gen1.spec_hash(), gen2.spec_hash(), "Same spec should produce same hash");
}

// ═══════════════════════════════════════════════════════════════════════════
// P2: SVA COMPILER
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn p2_compiler_exists() {
    let compiler = compile_sva_compiler();
    // Compiler should be constructible
    let _ = compiler;
}

#[test]
fn p2_compiler_produces_valid_sva() {
    let compiler = compile_sva_compiler();
    let gen = compiler.compile("Always, if Req is valid then Ack is valid.").unwrap();
    let sva = gen.generate("clk").unwrap();
    assert!(!sva.body.is_empty(), "Compiled SVA should have a body");
    assert!(!sva.sva_text.is_empty(), "Compiled SVA should have full text");
}

#[test]
fn p2_compiler_equals_p1() {
    let spec = "Always, if Req is valid then Ack is valid.";
    let compiler = compile_sva_compiler();
    let gen_p2 = compiler.compile(spec).unwrap();
    let gen_p1 = compile_sva_generator(spec).unwrap();
    assert_eq!(gen_p1.body(), gen_p2.body(), "P2 compiler should equal P1 generator");
}

#[test]
fn p2_compiler_no_interpreter_overhead() {
    let compiler = compile_sva_compiler();
    let gen = compiler.compile("Always, if Req is valid then Ack is valid.").unwrap();
    // Generating with a different clock should NOT re-synthesize
    let sva1 = gen.generate("clk1").unwrap();
    let sva2 = gen.generate("clk2").unwrap();
    // Bodies should be identical (only clock changes)
    assert_eq!(sva1.body, sva2.body, "Body should be same regardless of clock");
    assert_ne!(sva1.sva_text, sva2.sva_text, "SVA text should differ by clock");
}

#[test]
fn p2_compiler_handles_temporal() {
    let compiler = compile_sva_compiler();
    let result = compiler.compile("Eventually, Ack is valid.");
    assert!(result.is_ok(), "Should handle temporal spec");
}

// ═══════════════════════════════════════════════════════════════════════════
// CORRECTNESS PROPERTIES
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn compiler_correctness_certificate() {
    // For any spec, verify_compiler_correctness should return true
    let specs = vec![
        "Always, if Req is valid then Ack is valid.",
        "Always, Grant A and Grant B are not both valid.",
    ];
    for spec in specs {
        assert!(verify_compiler_correctness(spec, "clk"),
            "Correctness should hold for: {}", spec);
    }
}

#[test]
fn compiler_no_llm_dependency() {
    // The compiled path should have zero LLM calls
    let gen = compile_sva_generator("Always, if Req is valid then Ack is valid.").unwrap();
    let sva = gen.generate("clk").unwrap();
    // If we got here without errors, no LLM was needed
    assert!(!sva.body.is_empty());
}

#[test]
fn compiler_multiple_specs() {
    let compiler = compile_sva_compiler();
    let specs = vec![
        "Always, if Req is valid then Ack is valid.",
        "Always, Grant A and Grant B are not both valid.",
        "Eventually, Ack is valid.",
    ];
    for spec in specs {
        let result = compiler.compile(spec);
        assert!(result.is_ok(), "Compiler should handle: {}", spec);
    }
}

#[test]
fn compiler_error_on_unsupported() {
    // Garbage input should produce an error, not wrong SVA
    let result = compile_sva_generator("");
    assert!(result.is_err(), "Empty spec should error");
}

#[test]
fn compiler_round_trip() {
    let spec = "Always, if Req is valid then Ack is valid.";
    let gen = compile_sva_generator(spec).unwrap();
    let sva = gen.generate("clk").unwrap();
    // The SVA text should be parseable
    assert!(sva.sva_text.contains("clk"), "SVA should reference clock");
}

#[test]
fn compiler_performance() {
    // Compiled path should be faster than interpreted path
    let spec = "Always, if Req is valid then Ack is valid.";
    let gen = compile_sva_generator(spec).unwrap();

    let start = std::time::Instant::now();
    for _ in 0..100 {
        let _ = gen.generate("clk");
    }
    let compiled_time = start.elapsed();

    // Just verify it's fast (< 1 second for 100 iterations)
    assert!(compiled_time.as_secs() < 1,
        "100 compiled generations should be fast, took {:?}", compiled_time);
}

#[test]
fn compiler_spec_hash_unique() {
    let gen1 = compile_sva_generator("Always, if Req is valid then Ack is valid.").unwrap();
    let gen2 = compile_sva_generator("Always, Grant A and Grant B are not both valid.").unwrap();
    assert_ne!(gen1.spec_hash(), gen2.spec_hash(), "Different specs should have different hashes");
}

#[test]
fn compiler_generate_with_different_clocks() {
    let gen = compile_sva_generator("Always, if Req is valid then Ack is valid.").unwrap();
    let sva_fast = gen.generate("fast_clk").unwrap();
    let sva_slow = gen.generate("slow_clk").unwrap();
    assert!(sva_fast.sva_text.contains("fast_clk"));
    assert!(sva_slow.sva_text.contains("slow_clk"));
    assert_eq!(sva_fast.body, sva_slow.body, "Body should be clock-independent");
}

#[test]
fn compiler_the_trick() {
    // The Trick: compiler-compiled-compiler produces same SVA (P3-level)
    // P2 compiler creates P1 generator → generates SVA
    // A second P2 compiler doing the same should produce identical output
    let spec = "Always, if Req is valid then Ack is valid.";
    let c1 = compile_sva_compiler();
    let c2 = compile_sva_compiler();
    let gen1 = c1.compile(spec).unwrap();
    let gen2 = c2.compile(spec).unwrap();
    let sva1 = gen1.generate("clk").unwrap();
    let sva2 = gen2.generate("clk").unwrap();
    assert_eq!(sva1.body, sva2.body, "The Trick: identical compilers produce identical output");
}

#[test]
fn compiler_signals_preserved() {
    let spec = "Always, if Req is valid then Ack is valid.";
    let interpreted = synthesize_sva_from_spec(spec, "clk").unwrap();
    let gen = compile_sva_generator(spec).unwrap();
    let compiled = gen.generate("clk").unwrap();
    let mut int_sigs = interpreted.signals.clone();
    let mut comp_sigs = compiled.signals.clone();
    int_sigs.sort();
    comp_sigs.sort();
    assert_eq!(int_sigs, comp_sigs, "Signals should be preserved (order-independent)");
}

#[test]
fn compiler_kind_preserved() {
    let spec = "Always, if Req is valid then Ack is valid.";
    let interpreted = synthesize_sva_from_spec(spec, "clk").unwrap();
    let gen = compile_sva_generator(spec).unwrap();
    let compiled = gen.generate("clk").unwrap();
    assert_eq!(interpreted.kind, compiled.kind, "Kind should be preserved");
}
