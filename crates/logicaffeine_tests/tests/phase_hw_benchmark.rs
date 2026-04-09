//! Hardware Benchmark: Broad FOL→SVA Synthesis Probe
//!
//! A discovery-oriented benchmark that runs ~105 handwritten hardware property
//! sentences + 379 FVEval (NVIDIA) NL-to-SVA specs through
//! `synthesize_sva_from_spec()` and reports results.
//!
//! Two layers:
//! - Layer 1 (probe): table-driven, non-failing — discovers gaps
//! - Layer 2 (regression): individual `#[test]` fns for confirmed-passing specs

use logicaffeine_compile::codegen_sva::fol_to_sva::{synthesize_sva_from_spec, SynthesizedSva};
use logicaffeine_compile::codegen_sva::sva_model::parse_sva;
use std::path::Path;

// ═══════════════════════════════════════════════════════════════════════════
// DATA STRUCTURES
// ═══════════════════════════════════════════════════════════════════════════

struct HwSpec {
    spec: &'static str,
    tags: &'static [&'static str],
}

/// Check if SVA body is degenerate (vacuous or trivially broken)
fn is_degenerate(body: &str) -> bool {
    let trimmed = body.trim();
    trimmed == "0"
        || trimmed == "1"
        || trimmed == "0 |-> 0"
        || trimmed == "1 |-> 1"
        || trimmed.starts_with("0 |->")
        || trimmed.ends_with("|-> 0")
}

/// Synthesize and assert: parse OK + non-degenerate + parseable SVA
fn synth_hw(spec: &str) -> SynthesizedSva {
    let r = synthesize_sva_from_spec(spec, "clk")
        .unwrap_or_else(|e| panic!("Parse failed for '{}': {}", spec, e));
    assert!(!is_degenerate(&r.body), "Degenerate body for '{}': '{}'", spec, r.body);
    let parse = parse_sva(&r.body);
    assert!(parse.is_ok(), "Unparseable SVA for '{}': body='{}' err={:?}", spec, r.body, parse.err());
    r
}

// ═══════════════════════════════════════════════════════════════════════════
// THE CORPUS (~105 unique hardware specs)
// ═══════════════════════════════════════════════════════════════════════════

const CORPUS: &[HwSpec] = &[

    // ── Safety Invariants ──
    HwSpec { spec: "Always, every signal is valid.", tags: &["safety", "basic"] },
    HwSpec { spec: "Always, every signal is ready.", tags: &["safety", "basic"] },
    HwSpec { spec: "Always, every wire is high.", tags: &["safety", "basic"] },
    HwSpec { spec: "Always, every register is valid.", tags: &["safety", "basic"] },
    HwSpec { spec: "The signal is always valid.", tags: &["safety", "copula_temporal"] },
    HwSpec { spec: "The clock is never low.", tags: &["safety", "copula_temporal", "negation"] },

    // ── Conditional Safety ──
    HwSpec { spec: "Always, if every request is valid then every grant is valid.", tags: &["conditional", "handshake"] },
    HwSpec { spec: "Always, if every valid holds then every ready holds.", tags: &["conditional", "handshake"] },
    HwSpec { spec: "Always, if every request holds then every acknowledgment holds.", tags: &["conditional", "handshake"] },
    HwSpec { spec: "Always, if every request holds, then eventually, every response holds.", tags: &["conditional", "liveness", "handshake"] },
    HwSpec { spec: "If the wire is high, the latch is full.", tags: &["conditional"] },

    // ── Conditional + Stability ──
    HwSpec { spec: "If valid is asserted and ready is not asserted, data remains stable.", tags: &["conditional", "stability", "known_gap"] },
    HwSpec { spec: "If enable is low, the register holds its value.", tags: &["conditional", "stability", "known_gap"] },
    HwSpec { spec: "If enable is low, the output remains valid.", tags: &["conditional", "stability", "known_gap"] },

    // ── Conditional + Temporal Delay ──
    HwSpec { spec: "If request is asserted, acknowledge is asserted within 2 cycles.", tags: &["conditional", "delay", "known_gap"] },
    HwSpec { spec: "If request is deasserted, grant is deasserted within 1 cycle.", tags: &["conditional", "delay", "known_gap"] },
    HwSpec { spec: "If reset is asserted, valid is low within 1 cycle.", tags: &["conditional", "delay", "reset", "known_gap"] },
    HwSpec { spec: "If reset is deasserted, ready is asserted within 3 cycles.", tags: &["conditional", "delay", "reset", "known_gap"] },
    HwSpec { spec: "If start is asserted, done is asserted in the next cycle.", tags: &["conditional", "delay", "known_gap"] },
    HwSpec { spec: "If an error is detected, the interrupt is raised within 1 cycle.", tags: &["conditional", "delay", "error", "known_gap"] },
    HwSpec { spec: "If start bit is detected, stop bit follows within 10 cycles.", tags: &["conditional", "delay", "serial", "known_gap"] },

    // ── FIFO Properties ──
    HwSpec { spec: "If the FIFO is full, write is not asserted.", tags: &["conditional", "fifo", "known_gap"] },
    HwSpec { spec: "If the FIFO is empty, read is not asserted.", tags: &["conditional", "fifo", "known_gap"] },
    HwSpec { spec: "If write is asserted and full is low, the write pointer advances.", tags: &["conditional", "fifo", "known_gap"] },
    HwSpec { spec: "If read is asserted and empty is low, the read pointer advances.", tags: &["conditional", "fifo", "known_gap"] },

    // ── Reset Properties ──
    HwSpec { spec: "After reset, the counter is zero.", tags: &["reset", "after", "known_gap"] },
    HwSpec { spec: "After reset is deasserted, the signal is valid.", tags: &["reset", "after", "known_gap"] },
    HwSpec { spec: "The output is never high while reset is asserted.", tags: &["reset", "while", "negation", "known_gap"] },

    // ── Error Handling ──
    HwSpec { spec: "If parity error is detected, the error flag is asserted.", tags: &["conditional", "error", "known_gap"] },
    HwSpec { spec: "If overflow is detected, the status bit is set.", tags: &["conditional", "error", "known_gap"] },

    // ── Mutex / Mutual Exclusion ──
    HwSpec { spec: "Always, not both every request is valid and every grant is valid.", tags: &["mutex", "negation"] },
    HwSpec { spec: "Grant0 and Grant1 are not both asserted.", tags: &["mutex", "known_gap"] },
    HwSpec { spec: "Read and write are not both asserted.", tags: &["mutex", "known_gap"] },
    HwSpec { spec: "At most one signal is valid.", tags: &["mutex", "counting"] },
    HwSpec { spec: "At most one grant is valid.", tags: &["mutex", "counting"] },
    HwSpec { spec: "At most one of grant0, grant1, and grant2 is asserted.", tags: &["mutex", "counting", "known_gap"] },
    HwSpec { spec: "At most one request is granted at any time.", tags: &["mutex", "counting", "known_gap"] },

    // ── Liveness ──
    HwSpec { spec: "Eventually, every signal is valid.", tags: &["liveness"] },
    HwSpec { spec: "Eventually, every done is valid.", tags: &["liveness"] },
    HwSpec { spec: "Always, eventually every signal is ready.", tags: &["liveness", "nested_temporal"] },

    // ── Next-Cycle ──
    HwSpec { spec: "Next, every signal is valid.", tags: &["nexttime"] },
    HwSpec { spec: "Next, every grant is valid.", tags: &["nexttime"] },

    // ── Temporal Binary ──
    HwSpec { spec: "Every request holds until every grant holds.", tags: &["until"] },
    HwSpec { spec: "Every signal is valid until every done is valid.", tags: &["until"] },
    HwSpec { spec: "If valid is asserted, it remains asserted until ready is asserted.", tags: &["until", "stability", "known_gap"] },

    // ── Modal / Obligation ──
    HwSpec { spec: "The receiver shall acknowledge every request.", tags: &["modal"] },
    HwSpec { spec: "The transmitter shall not send data while idle.", tags: &["modal", "while", "negation", "known_gap"] },
    HwSpec { spec: "Every request must be granted.", tags: &["modal", "passive"] },

    // ── While / Duration ──
    HwSpec { spec: "While valid is asserted and ready is not asserted, data is stable.", tags: &["while", "stability", "known_gap"] },

    // ── After / Sequencing ──
    HwSpec { spec: "After request, grant follows within 3 cycles.", tags: &["after", "delay", "known_gap"] },

    // ── Conjunction / Disjunction ──
    HwSpec { spec: "Always, every request is valid and every grant is valid.", tags: &["conjunction"] },
    HwSpec { spec: "Always, every request is valid or every grant is valid.", tags: &["disjunction"] },
    HwSpec { spec: "Always, every request is valid and every grant is valid and every signal is ready.", tags: &["conjunction", "three_way"] },

    // ── Negation ──
    HwSpec { spec: "Always, not every signal is valid.", tags: &["negation"] },
    HwSpec { spec: "If request is not asserted, acknowledge is not asserted.", tags: &["negation", "conditional", "known_gap"] },

    // ── Counting / Cardinality ──
    HwSpec { spec: "At most two signals are valid.", tags: &["counting"] },
    HwSpec { spec: "At least one signal is valid.", tags: &["counting"] },
    HwSpec { spec: "At least two signals are valid.", tags: &["counting"] },

    // ── Passive Voice ──
    HwSpec { spec: "The signal must be acknowledged.", tags: &["passive", "modal"] },
    HwSpec { spec: "The data is transferred when valid and ready are both high.", tags: &["passive", "conditional", "known_gap"] },

    // ── NeoEvent / Action ──
    HwSpec { spec: "The arbiter grants the request.", tags: &["action", "neoevent"] },
    HwSpec { spec: "The controller enables the latch.", tags: &["action", "neoevent"] },
    HwSpec { spec: "The bus acknowledges the request.", tags: &["action", "neoevent"] },

    // ═══════════════════════════════════════════════════════════════════════
    // ASSERTIONFORGE-INSPIRED: REAL SoC/PROTOCOL PROPERTIES
    // ═══════════════════════════════════════════════════════════════════════

    // ── Reset Propagation ──
    HwSpec { spec: "If reset_n is low, puc_rst is high in the next cycle.", tags: &["reset", "nexttime", "assertionforge", "rtl_names"] },
    HwSpec { spec: "If reset_n is low, every register is invalid.", tags: &["reset", "conditional", "assertionforge"] },

    // ── IRQ Consistency ──
    HwSpec { spec: "If irq is zero, irq_ack is zero.", tags: &["irq", "conditional", "assertionforge", "rtl_names"] },
    HwSpec { spec: "If irq is not asserted, irq_ack is not asserted.", tags: &["irq", "conditional", "assertionforge", "rtl_names"] },
    HwSpec { spec: "If irq is asserted, irq_ack is asserted within 3 cycles.", tags: &["irq", "delay", "assertionforge", "rtl_names", "known_gap"] },

    // ── DMA Safety ──
    HwSpec { spec: "If dma_en and dma_we are asserted, dma_din is never unknown.", tags: &["dma", "safety", "assertionforge", "rtl_names", "stretch"] },
    HwSpec { spec: "If dma_en is asserted, dma_addr is valid.", tags: &["dma", "conditional", "assertionforge", "rtl_names"] },
    HwSpec { spec: "If dma_en is not asserted, dma_we is not asserted.", tags: &["dma", "conditional", "assertionforge", "rtl_names"] },

    // ── Clock/Domain Checks ──
    HwSpec { spec: "If smclk_en and cpu_en are asserted, smclk is never unknown.", tags: &["clock", "safety", "assertionforge", "rtl_names", "stretch"] },
    HwSpec { spec: "If clk_en is low, the register holds its value.", tags: &["clock", "stability", "assertionforge", "rtl_names", "known_gap"] },

    // ── Wakeup / Power ──
    HwSpec { spec: "If dco_enable is low and cpu_en is high, dco_wkup is asserted.", tags: &["power", "conditional", "assertionforge", "rtl_names", "stretch"] },
    HwSpec { spec: "If wakeup is asserted, cpu_en is asserted within 2 cycles.", tags: &["power", "delay", "assertionforge", "rtl_names", "known_gap"] },

    // ── APB Bus Timing ──
    HwSpec { spec: "If PADDR is valid, PWRITE is stable until PENABLE is asserted.", tags: &["apb", "stability", "until", "assertionforge", "known_gap"] },
    HwSpec { spec: "If PSEL and PENABLE are asserted, PREADY follows within 1 cycle.", tags: &["apb", "delay", "assertionforge", "known_gap"] },
    HwSpec { spec: "If PSEL is not asserted, PENABLE is not asserted.", tags: &["apb", "conditional", "assertionforge"] },
    HwSpec { spec: "If PENABLE is asserted, PSEL is asserted.", tags: &["apb", "conditional", "assertionforge"] },

    // ── UART Transmit Control ──
    HwSpec { spec: "If tx_busy is low and new_tx_data is asserted, tx_busy is asserted in the next cycle.", tags: &["uart", "nexttime", "assertionforge", "known_gap"] },
    HwSpec { spec: "If tx_busy is high, new_tx_data is not asserted.", tags: &["uart", "conditional", "assertionforge"] },
    HwSpec { spec: "If rx_valid is asserted, rx_data is valid.", tags: &["uart", "conditional", "assertionforge"] },

    // ── AXI Protocol ──
    HwSpec { spec: "If AWVALID is asserted, AWVALID remains asserted until AWREADY is asserted.", tags: &["axi", "until", "stability", "assertionforge", "known_gap"] },
    HwSpec { spec: "If ARVALID is asserted, ARADDR is valid.", tags: &["axi", "conditional", "assertionforge"] },
    HwSpec { spec: "If WVALID and WREADY are asserted, BVALID is asserted within 3 cycles.", tags: &["axi", "delay", "assertionforge", "known_gap"] },
    HwSpec { spec: "If RVALID is asserted, RDATA is valid.", tags: &["axi", "conditional", "assertionforge"] },
    HwSpec { spec: "If BVALID is asserted, BRESP is valid.", tags: &["axi", "conditional", "assertionforge"] },

    // ── SPI Interface ──
    HwSpec { spec: "If spi_cs is low, spi_clk is valid.", tags: &["spi", "conditional", "assertionforge"] },
    HwSpec { spec: "If spi_cs is high, spi_mosi is not asserted.", tags: &["spi", "conditional", "assertionforge"] },
    HwSpec { spec: "If spi_cs is low, spi_miso is valid within 1 cycle.", tags: &["spi", "delay", "assertionforge", "known_gap"] },

    // ── Arbiter Fairness ──
    HwSpec { spec: "If request is asserted, grant is eventually asserted.", tags: &["arbiter", "liveness", "assertionforge", "known_gap"] },
    HwSpec { spec: "At most one grant is asserted at any time.", tags: &["arbiter", "mutex", "assertionforge", "known_gap"] },
    HwSpec { spec: "If grant is asserted and request is not asserted, grant is not asserted in the next cycle.", tags: &["arbiter", "nexttime", "assertionforge", "known_gap"] },

    // ── Pipeline Hazards ──
    HwSpec { spec: "If stall is asserted, the register holds its value.", tags: &["pipeline", "stability", "assertionforge", "known_gap"] },
    HwSpec { spec: "If flush is asserted, valid is low in the next cycle.", tags: &["pipeline", "nexttime", "assertionforge", "known_gap"] },
    HwSpec { spec: "If valid is asserted and stall is not asserted, done is asserted in the next cycle.", tags: &["pipeline", "nexttime", "assertionforge", "known_gap"] },

    // ── Memory Interface ──
    HwSpec { spec: "If mem_req is asserted, mem_ack is asserted within 4 cycles.", tags: &["memory", "delay", "assertionforge", "known_gap"] },
    HwSpec { spec: "If mem_we is asserted, mem_addr is valid.", tags: &["memory", "conditional", "assertionforge"] },
    HwSpec { spec: "Read and write are not both asserted.", tags: &["memory", "mutex"] },

    // ── Counter / Timer ──
    HwSpec { spec: "If enable is high and the counter is full, the counter is zero in the next cycle.", tags: &["counter", "nexttime", "assertionforge", "known_gap"] },
    HwSpec { spec: "If enable is low, the counter holds its value.", tags: &["counter", "stability", "assertionforge", "known_gap"] },

    // ── CDC (Clock Domain Crossing) ──
    HwSpec { spec: "If sync_req is asserted, sync_ack is asserted within 3 cycles.", tags: &["cdc", "delay", "assertionforge", "known_gap"] },
];

// ═══════════════════════════════════════════════════════════════════════════
// LAYER 1: PROBE HARNESS (discovery mode — never hard-fails)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn benchmark_corpus_probe() {
    let mut pass = 0u32;
    let mut parse_fail = 0u32;
    let mut degenerate = 0u32;
    let mut unparseable_sva = 0u32;
    let mut suspicious = 0u32;

    for case in CORPUS {
        let result = synthesize_sva_from_spec(case.spec, "clk");
        match result {
            Err(e) => {
                parse_fail += 1;
                eprintln!("PARSE-FAIL [{}]: {} — {}", case.tags.join(","), case.spec, e);
            }
            Ok(r) if is_degenerate(&r.body) => {
                degenerate += 1;
                eprintln!("DEGENERATE [{}]: {} → {}", case.tags.join(","), case.spec, r.body);
            }
            Ok(r) => {
                if parse_sva(&r.body).is_err() {
                    unparseable_sva += 1;
                    eprintln!("BAD-SVA [{}]: {} → {}", case.tags.join(","), case.spec, r.body);
                } else if r.signals.is_empty() {
                    suspicious += 1;
                    eprintln!("SUSPICIOUS [{}]: {} → {} (no signals)", case.tags.join(","), case.spec, r.body);
                } else {
                    pass += 1;
                }
            }
        }
    }
    let total = CORPUS.len();
    eprintln!("\n=== HW BENCHMARK ===");
    eprintln!("  total:         {total}");
    eprintln!("  pass:          {pass}/{total}");
    eprintln!("  parse-fail:    {parse_fail}");
    eprintln!("  degenerate:    {degenerate}");
    eprintln!("  bad-sva:       {unparseable_sva}");
    eprintln!("  suspicious:    {suspicious}");
    // Don't assert on pass count — this is discovery, not regression
}

// ═══════════════════════════════════════════════════════════════════════════
// LAYER 1b: FVEval PROBE (NVIDIA NL2SVA benchmark — discovery mode)
// Source: https://github.com/NVlabs/FVEval (arXiv:2410.23299)
// 300 machine-generated + 79 human-written specs from real designs
// ═══════════════════════════════════════════════════════════════════════════

/// Load prompts from an FVEval CSV file. Returns (prompt, task_id) pairs.
fn load_fveval_csv(path: &Path) -> Vec<(String, String)> {
    let mut rdr = csv::Reader::from_path(path)
        .unwrap_or_else(|e| panic!("Failed to open {}: {}", path.display(), e));
    let mut specs = Vec::new();
    for result in rdr.records() {
        let record = result.expect("bad CSV record");
        let task_id = record.get(1).unwrap_or("?").to_string();
        let prompt = record.get(2).unwrap_or("").to_string();
        if !prompt.is_empty() {
            specs.push((prompt, task_id));
        }
    }
    specs
}

/// Run the probe on a set of (prompt, label) pairs and print summary.
/// Returns (pass, parse_fail, degenerate, bad_sva, suspicious, total).
fn run_fveval_probe(specs: &[(String, String)], label: &str) -> (u32, u32, u32, u32, u32, usize) {
    let mut pass = 0u32;
    let mut parse_fail = 0u32;
    let mut degenerate = 0u32;
    let mut bad_sva = 0u32;
    let mut suspicious = 0u32;

    for (prompt, task_id) in specs {
        let result = synthesize_sva_from_spec(prompt, "clk");
        match result {
            Err(e) => {
                parse_fail += 1;
                eprintln!("FVEVAL-PARSE-FAIL [{}:{}]: {} — {}", label, task_id, &prompt[..prompt.len().min(80)], e);
            }
            Ok(r) if is_degenerate(&r.body) => {
                degenerate += 1;
                eprintln!("FVEVAL-DEGENERATE [{}:{}]: {} → {}", label, task_id, &prompt[..prompt.len().min(80)], r.body);
            }
            Ok(r) => {
                if parse_sva(&r.body).is_err() {
                    bad_sva += 1;
                    eprintln!("FVEVAL-BAD-SVA [{}:{}]: {} → {}", label, task_id, &prompt[..prompt.len().min(80)], r.body);
                } else if r.signals.is_empty() {
                    suspicious += 1;
                } else {
                    pass += 1;
                }
            }
        }
    }
    let total = specs.len();
    eprintln!("\n=== FVEVAL {} ===", label.to_uppercase());
    eprintln!("  total:         {total}");
    eprintln!("  pass:          {pass}/{total}");
    eprintln!("  parse-fail:    {parse_fail}");
    eprintln!("  degenerate:    {degenerate}");
    eprintln!("  bad-sva:       {bad_sva}");
    eprintln!("  suspicious:    {suspicious}");

    (pass, parse_fail, degenerate, bad_sva, suspicious, total)
}

#[test]
fn fveval_machine_probe() {
    let csv_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../FVEval/data_nl2sva/data/nl2sva_machine.csv");
    if !csv_path.exists() {
        eprintln!("SKIP: FVEval machine CSV not found at {}", csv_path.display());
        return;
    }
    let specs = load_fveval_csv(&csv_path);
    assert!(!specs.is_empty(), "Machine CSV loaded 0 specs");
    let (pass, _pf, _deg, _bad, _sus, total) = run_fveval_probe(&specs, "machine");
    eprintln!("  pass-rate:     {:.1}%", pass as f64 / total as f64 * 100.0);
}

#[test]
fn fveval_human_probe() {
    let csv_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../FVEval/data_nl2sva/data/nl2sva_human.csv");
    if !csv_path.exists() {
        eprintln!("SKIP: FVEval human CSV not found at {}", csv_path.display());
        return;
    }
    let specs = load_fveval_csv(&csv_path);
    assert!(!specs.is_empty(), "Human CSV loaded 0 specs");
    let (pass, _pf, _deg, _bad, _sus, total) = run_fveval_probe(&specs, "human");
    eprintln!("  pass-rate:     {:.1}%", pass as f64 / total as f64 * 100.0);
}

// ═══════════════════════════════════════════════════════════════════════════
// SEPARATE TESTS: ERROR HANDLING (not in corpus)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn error_empty_spec() {
    let result = synthesize_sva_from_spec("", "clk");
    assert!(result.is_err(), "Empty spec should return error");
}

#[test]
fn error_gibberish_does_not_panic() {
    let _ = synthesize_sva_from_spec("asdf qwerty zxcv", "clk");
}

// ═══════════════════════════════════════════════════════════════════════════
// SEPARATE TESTS: CLOCK VARIATIONS (not in corpus)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn clock_variation_sys_clk() {
    let r = synthesize_sva_from_spec("Always, every signal is valid.", "sys_clk").unwrap();
    assert!(r.sva_text.contains("sys_clk"),
        "Should contain sys_clk. Got: {}", r.sva_text);
}

#[test]
fn clock_variation_pclk() {
    let r = synthesize_sva_from_spec("Always, every signal is valid.", "pclk").unwrap();
    assert!(r.sva_text.contains("pclk"),
        "Should contain pclk. Got: {}", r.sva_text);
}

// ═══════════════════════════════════════════════════════════════════════════
// LAYER 2: REGRESSION TESTS (must-pass, promoted from probe results)
// Promoted after benchmark_corpus_probe confirmed these pass cleanly.
// ═══════════════════════════════════════════════════════════════════════════

// ── Safety Invariants ──

#[test]
fn reg_safety_always_signal_valid() {
    let r = synth_hw("Always, every signal is valid.");
    assert!(r.sva_text.contains("assert property"));
}

#[test]
fn reg_safety_always_register_valid() {
    synth_hw("Always, every register is valid.");
}

#[test]
fn reg_safety_always_wire_high() {
    synth_hw("Always, every wire is high.");
}

#[test]
fn reg_safety_copula_signal_always_valid() {
    let r = synth_hw("The signal is always valid.");
    assert!(r.sva_text.contains("assert property") || r.sva_text.contains("@(posedge"));
}

#[test]
fn reg_safety_copula_clock_never_low() {
    let r = synth_hw("The clock is never low.");
    assert!(r.body.contains("!") || r.body.contains("not") || r.body.contains("~"),
        "Negation expected in body. Got: {}", r.body);
}

// ── Conditional Safety / Handshake ──

#[test]
fn reg_cond_request_grant() {
    let r = synth_hw("Always, if every request is valid then every grant is valid.");
    assert!(r.body.contains("|->") || r.body.contains("|=>"),
        "Should produce implication. Got: {}", r.body);
}

#[test]
fn reg_cond_valid_ready() {
    let r = synth_hw("Always, if every valid holds then every ready holds.");
    assert!(r.signals.len() >= 2, "Should extract >= 2 signals. Got: {:?}", r.signals);
}

#[test]
fn reg_cond_request_ack() {
    synth_hw("Always, if every request holds then every acknowledgment holds.");
}

#[test]
fn reg_cond_liveness_request_response() {
    let r = synth_hw("Always, if every request holds, then eventually, every response holds.");
    assert!(r.body.contains("s_eventually") || r.sva_text.contains("cover"),
        "Should contain s_eventually. Got body: {}", r.body);
}

#[test]
fn reg_cond_wire_latch() {
    synth_hw("If the wire is high, the latch is full.");
}

// ── Conditional + Stability ──

#[test]
fn reg_stability_valid_ready_data() {
    synth_hw("If valid is asserted and ready is not asserted, data remains stable.");
}

#[test]
fn reg_stability_enable_register() {
    synth_hw("If enable is low, the register holds its value.");
}

#[test]
fn reg_stability_enable_output() {
    synth_hw("If enable is low, the output remains valid.");
}

// ── Conditional + Temporal Delay ──

#[test]
fn reg_delay_request_ack_2_cycles() {
    synth_hw("If request is asserted, acknowledge is asserted within 2 cycles.");
}

#[test]
fn reg_delay_request_grant_deasserted() {
    synth_hw("If request is deasserted, grant is deasserted within 1 cycle.");
}

#[test]
fn reg_delay_reset_valid_low() {
    synth_hw("If reset is asserted, valid is low within 1 cycle.");
}

#[test]
fn reg_delay_start_done_next() {
    synth_hw("If start is asserted, done is asserted in the next cycle.");
}

#[test]
fn reg_delay_error_interrupt() {
    synth_hw("If an error is detected, the interrupt is raised within 1 cycle.");
}

#[test]
fn reg_delay_start_stop_serial() {
    synth_hw("If start bit is detected, stop bit follows within 10 cycles.");
}

// ── FIFO ──

#[test]
fn reg_fifo_full_no_write() {
    synth_hw("If the FIFO is full, write is not asserted.");
}

#[test]
fn reg_fifo_empty_no_read() {
    synth_hw("If the FIFO is empty, read is not asserted.");
}

#[test]
fn reg_fifo_write_pointer_advance() {
    synth_hw("If write is asserted and full is low, the write pointer advances.");
}

#[test]
fn reg_fifo_read_pointer_advance() {
    synth_hw("If read is asserted and empty is low, the read pointer advances.");
}

// ── Reset ──

#[test]
fn reg_reset_counter_zero() {
    synth_hw("After reset, the counter is zero.");
}

#[test]
fn reg_reset_deasserted_valid() {
    synth_hw("After reset is deasserted, the signal is valid.");
}

#[test]
fn reg_reset_output_never_high() {
    synth_hw("The output is never high while reset is asserted.");
}

// ── Error Handling ──

#[test]
fn reg_error_parity_flag() {
    synth_hw("If parity error is detected, the error flag is asserted.");
}

#[test]
fn reg_error_overflow_status() {
    synth_hw("If overflow is detected, the status bit is set.");
}

// ── Liveness ──

#[test]
fn reg_liveness_eventually_valid() {
    let r = synth_hw("Eventually, every signal is valid.");
    assert!(r.body.contains("s_eventually") || r.kind == "cover",
        "Liveness should use s_eventually or cover. Got body: {}, kind: {}", r.body, r.kind);
}

#[test]
fn reg_liveness_eventually_done() {
    synth_hw("Eventually, every done is valid.");
}

#[test]
fn reg_liveness_always_eventually_ready() {
    synth_hw("Always, eventually every signal is ready.");
}

// ── Next-Cycle ──

#[test]
fn reg_next_signal_valid() {
    let r = synth_hw("Next, every signal is valid.");
    assert!(!r.body.is_empty(), "nexttime should produce non-empty body");
}

#[test]
fn reg_next_grant_valid() {
    synth_hw("Next, every grant is valid.");
}

// ── Until ──

#[test]
fn reg_until_request_grant() {
    synth_hw("Every request holds until every grant holds.");
}

#[test]
fn reg_until_signal_done() {
    synth_hw("Every signal is valid until every done is valid.");
}

// ── Modal ──

#[test]
fn reg_modal_receiver_acknowledge() {
    synth_hw("The receiver shall acknowledge every request.");
}

#[test]
fn reg_modal_request_must_be_granted() {
    synth_hw("Every request must be granted.");
}

#[test]
fn reg_modal_signal_acknowledged() {
    synth_hw("The signal must be acknowledged.");
}

// ── Conjunction / Disjunction ──

#[test]
fn reg_conjunction_request_and_grant() {
    let r = synth_hw("Always, every request is valid and every grant is valid.");
    assert!(r.body.contains("&&"), "Conjunction should produce &&. Got: {}", r.body);
}

#[test]
fn reg_disjunction_request_or_grant() {
    let r = synth_hw("Always, every request is valid or every grant is valid.");
    assert!(r.body.contains("||"), "Disjunction should produce ||. Got: {}", r.body);
}

#[test]
fn reg_conjunction_three_way() {
    synth_hw("Always, every request is valid and every grant is valid and every signal is ready.");
}

// ── Negation ──

#[test]
fn reg_negation_not_every() {
    let r = synth_hw("Always, not every signal is valid.");
    assert!(r.body.contains("!") || r.body.contains("~"),
        "Negation expected. Got: {}", r.body);
}

// ── NeoEvent / Action ──

#[test]
fn reg_neoevent_arbiter_grants() {
    synth_hw("The arbiter grants the request.");
}

#[test]
fn reg_neoevent_controller_enables() {
    synth_hw("The controller enables the latch.");
}

// ── AssertionForge: Reset Propagation ──

#[test]
fn reg_af_reset_n_puc_rst() {
    synth_hw("If reset_n is low, puc_rst is high in the next cycle.");
}

#[test]
fn reg_af_reset_n_registers_invalid() {
    synth_hw("If reset_n is low, every register is invalid.");
}

// ── AssertionForge: IRQ ──

#[test]
fn reg_af_irq_zero_ack_zero() {
    synth_hw("If irq is zero, irq_ack is zero.");
}

#[test]
fn reg_af_irq_not_asserted_ack_not() {
    synth_hw("If irq is not asserted, irq_ack is not asserted.");
}

// ── AssertionForge: DMA ──

#[test]
fn reg_af_dma_en_addr_valid() {
    synth_hw("If dma_en is asserted, dma_addr is valid.");
}

#[test]
fn reg_af_dma_en_not_we_not() {
    synth_hw("If dma_en is not asserted, dma_we is not asserted.");
}

// ── AssertionForge: APB ──

#[test]
fn reg_af_apb_psel_not_penable_not() {
    synth_hw("If PSEL is not asserted, PENABLE is not asserted.");
}

#[test]
fn reg_af_apb_penable_psel() {
    synth_hw("If PENABLE is asserted, PSEL is asserted.");
}

// ── AssertionForge: UART ──

#[test]
fn reg_af_uart_tx_busy_no_new_data() {
    synth_hw("If tx_busy is high, new_tx_data is not asserted.");
}

#[test]
fn reg_af_uart_rx_valid_data() {
    synth_hw("If rx_valid is asserted, rx_data is valid.");
}

// ── AssertionForge: AXI ──

#[test]
fn reg_af_axi_arvalid_araddr() {
    synth_hw("If ARVALID is asserted, ARADDR is valid.");
}

#[test]
fn reg_af_axi_rvalid_rdata() {
    synth_hw("If RVALID is asserted, RDATA is valid.");
}

#[test]
fn reg_af_axi_bvalid_bresp() {
    synth_hw("If BVALID is asserted, BRESP is valid.");
}

// ── AssertionForge: SPI ──

#[test]
fn reg_af_spi_cs_low_clk_valid() {
    synth_hw("If spi_cs is low, spi_clk is valid.");
}

#[test]
fn reg_af_spi_cs_high_mosi_not() {
    synth_hw("If spi_cs is high, spi_mosi is not asserted.");
}

// ── AssertionForge: Memory ──

#[test]
fn reg_af_mem_we_addr_valid() {
    synth_hw("If mem_we is asserted, mem_addr is valid.");
}

#[test]
fn reg_af_mem_read_write_mutex() {
    synth_hw("Read and write are not both asserted.");
}

// ── AssertionForge: Delay specs (known_gap tagged but actually passing) ──

#[test]
fn reg_af_irq_ack_within_3() {
    synth_hw("If irq is asserted, irq_ack is asserted within 3 cycles.");
}

#[test]
fn reg_af_reset_deasserted_ready_3() {
    synth_hw("If reset is deasserted, ready is asserted within 3 cycles.");
}

#[test]
fn reg_af_wakeup_cpu_en_2() {
    synth_hw("If wakeup is asserted, cpu_en is asserted within 2 cycles.");
}

#[test]
fn reg_af_psel_penable_pready_1() {
    synth_hw("If PSEL and PENABLE are asserted, PREADY follows within 1 cycle.");
}

#[test]
fn reg_af_spi_cs_miso_1() {
    synth_hw("If spi_cs is low, spi_miso is valid within 1 cycle.");
}

#[test]
fn reg_af_mem_req_ack_4() {
    synth_hw("If mem_req is asserted, mem_ack is asserted within 4 cycles.");
}

#[test]
fn reg_af_sync_req_ack_3() {
    synth_hw("If sync_req is asserted, sync_ack is asserted within 3 cycles.");
}

// ── AssertionForge: Stability / Pipeline ──

#[test]
fn reg_af_clk_en_register_holds() {
    synth_hw("If clk_en is low, the register holds its value.");
}

#[test]
fn reg_af_stall_register_holds() {
    synth_hw("If stall is asserted, the register holds its value.");
}

#[test]
fn reg_af_flush_valid_low_next() {
    synth_hw("If flush is asserted, valid is low in the next cycle.");
}

#[test]
fn reg_af_valid_stall_done_next() {
    synth_hw("If valid is asserted and stall is not asserted, done is asserted in the next cycle.");
}

#[test]
fn reg_af_enable_counter_holds() {
    synth_hw("If enable is low, the counter holds its value.");
}

// ── AssertionForge: Power / Wakeup ──

#[test]
fn reg_af_dco_enable_wkup() {
    synth_hw("If dco_enable is low and cpu_en is high, dco_wkup is asserted.");
}

// ── AssertionForge: UART nexttime ──

#[test]
fn reg_af_uart_tx_busy_next() {
    synth_hw("If tx_busy is low and new_tx_data is asserted, tx_busy is asserted in the next cycle.");
}

// ── AssertionForge: AXI until ──

#[test]
fn reg_af_axi_awvalid_until_awready() {
    synth_hw("If AWVALID is asserted, AWVALID remains asserted until AWREADY is asserted.");
}

// ── AssertionForge: AXI delay ──

#[test]
fn reg_af_axi_wvalid_bvalid_3() {
    synth_hw("If WVALID and WREADY are asserted, BVALID is asserted within 3 cycles.");
}

// ── AssertionForge: APB until ──

#[test]
fn reg_af_apb_paddr_pwrite_until_penable() {
    synth_hw("If PADDR is valid, PWRITE is stable until PENABLE is asserted.");
}

// ── AssertionForge: Arbiter ──

#[test]
fn reg_af_arbiter_grant_request_next() {
    synth_hw("If grant is asserted and request is not asserted, grant is not asserted in the next cycle.");
}

// ── AssertionForge: Counter ──

#[test]
fn reg_af_counter_full_zero_next() {
    synth_hw("If enable is high and the counter is full, the counter is zero in the next cycle.");
}

// ── Conditional negation ──

#[test]
fn reg_cond_negation_request_ack() {
    synth_hw("If request is not asserted, acknowledge is not asserted.");
}

// ── Until + stability ──

#[test]
fn reg_until_valid_remains_until_ready() {
    synth_hw("If valid is asserted, it remains asserted until ready is asserted.");
}

// ── Passive ──

#[test]
fn reg_passive_data_transferred() {
    synth_hw("The data is transferred when valid and ready are both high.");
}
