//! ═══════════════════════════════════════════════════════════════════════════════════════════════
//! THE META-RATCHET — every way LOGOS bytecode executes on silicon MUST have a $50M coverage ratchet,
//! and the ratcheted set can only GROW. A new bytecode-executing core `.v` with no ratchet goes RED.
//! ═══════════════════════════════════════════════════════════════════════════════════════════════
//!
//! WHY THIS FILE EXISTS (the capstone)
//! ────────────────────────────────────
//! Three sibling ratchets prove that every opcode a silicon core executes is DECODED + iverilog-EXERCISED
//! ≡ the VM, killing the "$50M silent-zero" class (a model-proven op that reads 0 on tape-out because its
//! RTL decode arm is missing / never simulated):
//!   • `fifty_million_dollar_test.rs`        → the full 115-op soft-core `benchmarks/silicon/soft_core.v`
//!   • `scalar_fifty_million_dollar_test.rs` → the Sky130 tape-out scalar core `soft_core_scalar.v`
//!   • `fabric_fifty_million_dollar_test.rs` → the spatial-fabric emitter
//! Each caught a REAL silent-zero hole (the Word32Add/Sub/Mul 226/227/228 class; the fabric Pow/MulOvfS/
//! ApproxEqF/FFloorF class). But NOTHING forces a FOURTH bytecode-executing core — or a newly-added
//! instruction-decoding `.v` — to get a ratchet. Someone could add one ratchet-less and silently REOPEN
//! the entire $50M silent-zero class: the new core's decode arms would never be proven ⊆ EXERCISED.
//!
//! THE INVARIANT (strict — never weaken it to go green)
//! ─────────────────────────────────────────────────────
//! Every BYTECODE-EXECUTING CORE `.v` in `benchmarks/silicon/` (a module that FETCHES an instruction from
//! its own instruction memory driven by a program counter, then DECODES it through a top-level
//! `case (opcode)` / `case (op)` ISA dispatch with MANY opcode arms) MUST be mapped to a $50M coverage
//! ratchet test in `CORE_RATCHETS`, and that ratchet file MUST exist on disk.
//!
//!   • GUARD 1 (completeness — the KILLER guard): every bytecode-executing core the classifier finds must
//!     be in `CORE_RATCHETS`. A NEW unmapped core `.v` → RED, LOUD. The excludes are EXPLICIT + auditable
//!     via `NON_CORE_EXECUTABLE` (each with a reason), so a `.v` that IS a bytecode core but sits on the
//!     allowlist by mistake still can't hide — the allowlist only suppresses the classifier's own
//!     borderline shapes, never a genuine fetch/decode core.
//!   • GUARD 2 (the ratchets are real): every ratchet file named in `CORE_RATCHETS` must EXIST on disk, so
//!     renaming/deleting a ratchet goes RED.
//!   • NON-VACUOUS: the mapped set is ≥ 3 and contains `soft_core.v`.
//!
//! ⚠️  DO NOT weaken this file to make a RED pass. A RED means either a new bytecode-executing silicon core
//! shipped WITHOUT a $50M coverage ratchet (add one — mirror `fifty_million_dollar_test.rs` — or, if it is
//! genuinely not a bytecode executor, refine the classifier / add it to `NON_CORE_EXECUTABLE` with a
//! documented reason), or a ratchet file was renamed/deleted. NEVER ship a core whose decode arms aren't
//! ratcheted — that IS the $50M silent-zero class. ⚠️

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// The curated ratchet map: (bytecode-executing core, its $50M coverage ratchet test binary).
// The fabric ratchet covers the spatial-fabric EMITTER, which is not a fixed `.v` file — it emits fresh
// Verilog per (cfg, program) — so it is keyed by the sentinel `"fabric (emitter, not a fixed .v)"`.
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// Every way LOGOS bytecode executes on silicon → its $50M coverage ratchet test binary (a
/// `crates/logicaffeine_tests/tests/<name>.rs` file). GUARD 1 requires every classified core `.v` to
/// appear here as a first-tuple entry; GUARD 2 requires every second-tuple ratchet file to exist.
const CORE_RATCHETS: &[(&str, &str)] = &[
    // The full 115-op soft core — the FPGA / general silicon core.
    ("soft_core.v", "fifty_million_dollar_test"),
    // The Sky130-signoff scalar tape-out core.
    ("soft_core_scalar.v", "scalar_fifty_million_dollar_test"),
    // The spatial-fabric emitter (not a fixed `.v` — emits per (cfg, program)).
    (
        "fabric (emitter, not a fixed .v)",
        "fabric_fifty_million_dollar_test",
    ),
];

/// Parallel to `CORE_RATCHETS`: the RANDOM-PROGRAM differential fuzz for each core. A `$50M` ratchet proves
/// every op the encoder emits is DECODED + iverilog-EXERCISED on a CURATED per-op corpus; a random-program
/// fuzz proves the core matches the VM on RANDOMLY-GENERATED programs. These are COMPLEMENTARY layers — the
/// ratchets are curated coverage, the fuzz is random edge cases. The random fuzz is the layer that caught FIVE
/// real silicon bugs the curated ratchets/tests missed (negative-`lo` range loop, negative-constant load, map
/// key overwrite, dropped closure args, engine Call-with-multiply frame-save). Every executable core must have
/// BOTH; GUARD 3 (`every_bytecode_executing_core_has_a_random_program_fuzz`) makes shipping a core without a
/// random fuzz impossible — the curated-passes-but-random-fails class this session proved exists on EVERY core.
const CORE_FUZZES: &[(&str, &str)] = &[
    ("soft_core.v", "soft_core_rtl_fuzz_differential"),
    (
        "soft_core_scalar.v",
        "soft_core_scalar_rtl_fuzz_differential",
    ),
    // The fabric engine's random-program-on-emitted-RTL fuzz (single- + multi-arg).
    ("fabric (emitter, not a fixed .v)", "wprog_engine_rtl_fuzz"),
];

/// The EXPLICIT, AUDITABLE exclusion allowlist: `.v` files that a naïve reader might mistake for a
/// bytecode-executing core (they contain a `case(opcode)` / `case(op)` and/or many `8'dN:` labels) but
/// are NOT instruction-fetching ISA executors. Each carries a reason. The classifier already excludes
/// these on principle (no `imem[pc]` fetch), but they are listed so the exclusion is documented + a
/// reviewer can audit exactly WHY each is not ratcheted. If any of these ever GAINS a real instruction
/// fetch (an `imem[pc]` dispatch), the classifier reclassifies it as a core and GUARD 1 fires — the
/// allowlist cannot mask a genuine fetch/decode core, only the datapath / decoder shapes below.
const NON_CORE_EXECUTABLE: &[(&str, &str)] = &[
    (
        "alu.v",
        "SAT-proven combinational ALU datapath — decodes a WIRE OPCODE BYTE off an `input [7:0] opcode` \
         port with no program counter and no instruction fetch. It is the functional unit the cores \
         ISSUE to (their `default:` arm routes here), not an ISA executor.",
    ),
    (
        "rev_alu_core.v",
        "SAT-proven reversible-ALU datapath (the 0..4 reversible core selects) — a functional unit fed a \
         control byte, no fetch/decode.",
    ),
    (
        "isa_core_decoder.v",
        "A pure DECODER: maps a wire opcode byte to a reversible-core `sel`/`core` control — 5 label arms, \
         an `input [7:0] opcode` port, no imem/pc/fetch. It classifies an opcode; it does not execute a \
         program.",
    ),
];

fn silicon_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon")
}

fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// Strip a trailing `//` line comment so a commented-out `case (…)` / `8'dN:` never counts. (The
/// candidate cores use no block comments; a `//`-strip matches the sibling ratchets' parsers exactly.)
fn strip_comment(raw: &str) -> &str {
    match raw.find("//") {
        Some(idx) => &raw[..idx],
        None => raw,
    }
}

/// Count the top-level `8'dN:` decode-arm labels whose innermost enclosing `case (…)` selector is exactly
/// `opcode` or `op` — the ISA dispatch. Nested sub-dispatches (`case (rb)`, `case (dst[7:4])`, FSM
/// selectors) do NOT count, so a core with a big builtin sub-dispatch isn't over-counted and a datapath
/// whose `case` selector is NOT `opcode`/`op` scores 0. Comment-stripped, case/endcase-nesting-aware —
/// the same left-to-right source-order scan the `soft_core_decoded` parser uses.
fn isa_dispatch_arm_count(text: &str) -> usize {
    let mut arms: BTreeSet<u16> = BTreeSet::new();
    let collect = |label: &str, into: &mut BTreeSet<u16>| {
        for tok in label.split(|c: char| !(c.is_ascii_digit() || c == '\'' || c == 'd')) {
            if let Some(num) = tok.strip_prefix("8'd") {
                if let Ok(n) = num.parse::<u16>() {
                    into.insert(n);
                }
            }
        }
    };
    let is_isa_selector = |sel: &str| sel == "opcode" || sel == "op";
    let mut case_stack: Vec<String> = Vec::new();
    for raw in text.lines() {
        let line = strip_comment(raw);
        // Collect this line's labels using the CURRENT enclosing selector (the label belongs to the case
        // open as the line begins), before mutating the stack.
        if case_stack.last().map(String::as_str).map(is_isa_selector) == Some(true) {
            let trimmed = line.trim();
            if let Some(colon) = line.find(':') {
                collect(&line[..colon], &mut arms);
            } else if trimmed.ends_with(',') && trimmed.starts_with("8'd") {
                collect(trimmed, &mut arms);
            }
        }
        // Apply every `case (…)` open and `endcase` close on this line in source order.
        let mut rest = line;
        loop {
            let next_case = rest.find("case");
            let next_end = rest.find("endcase");
            match (next_case, next_end) {
                (Some(ci), Some(ei)) if ei <= ci => {
                    case_stack.pop();
                    rest = &rest[ei + "endcase".len()..];
                }
                (Some(ci), _) => {
                    let after = rest[ci + "case".len()..].trim_start();
                    if let Some(inner) = after.strip_prefix('(') {
                        if let Some(close) = inner.find(')') {
                            case_stack.push(inner[..close].trim().to_string());
                        }
                    }
                    rest = &rest[ci + "case".len()..];
                }
                (None, Some(ei)) => {
                    case_stack.pop();
                    rest = &rest[ei + "endcase".len()..];
                }
                (None, None) => break,
            }
        }
    }
    arms.len()
}

/// Does this module FETCH instructions? A bytecode executor reads its instruction word from an
/// instruction memory indexed by a program counter — `imem[pc]` (possibly with whitespace). A pure
/// datapath / decoder is FED an opcode on an `input` port and has no such fetch. This is the decisive
/// discriminator between `soft_core.v`/`soft_core_scalar.v` (fetch) and `alu.v`/`isa_core_decoder.v`
/// (opcode is an input port). Comment-stripped so a commented fetch never counts.
fn has_instruction_fetch(text: &str) -> bool {
    for raw in text.lines() {
        let line = strip_comment(raw);
        // Look for `imem[<ws>pc` — the instruction fetch. `pc` may be followed by `]`, `+`, `-`, or ws.
        let mut hay = line;
        while let Some(idx) = hay.find("imem") {
            let after = hay[idx + "imem".len()..].trim_start();
            if let Some(inner) = after.strip_prefix('[') {
                let inner = inner.trim_start();
                // The index expression begins with the `pc` register (as a whole word).
                if inner == "pc"
                    || inner.starts_with("pc]")
                    || inner.starts_with("pc ")
                    || inner.starts_with("pc+")
                    || inner.starts_with("pc-")
                    || inner.starts_with("pc[")
                {
                    return true;
                }
            }
            hay = &hay[idx + "imem".len()..];
        }
    }
    false
}

/// The minimum ISA-dispatch arm count for a module to be considered an instruction decoder. A real core
/// decodes MANY opcodes; a datapath ALU tops out around the arithmetic/bitwise/compare set, and a decoder
/// stub has a handful. 15 cleanly separates the cores (16 / 169 arms) from every non-core (≤ 5 arms with
/// an `opcode`/`op` selector — `isa_core_decoder.v` has 5; `alu.v`'s 20 arms are gated OUT by the fetch
/// requirement, not the count).
const MIN_ISA_ARMS: usize = 15;

/// The SOUND classifier: a `.v` module is a BYTECODE-EXECUTING CORE iff it BOTH (a) has a top-level
/// `case (opcode)` / `case (op)` ISA dispatch with ≥ `MIN_ISA_ARMS` opcode arms (it decodes an ISA) AND
/// (b) FETCHES those instructions from `imem[pc]` (it executes a PROGRAM, not a single wire byte). Both
/// conditions are necessary: (a) alone admits `alu.v` (20 arms, but fed a wire byte); (b) alone admits
/// nothing that isn't already a core. Their conjunction is exactly "fetches + decodes an ISA".
fn is_bytecode_executing_core(text: &str) -> bool {
    isa_dispatch_arm_count(text) >= MIN_ISA_ARMS && has_instruction_fetch(text)
}

/// Enumerate the candidate `.v` files: everything in `benchmarks/silicon/` that is NOT a `tb_*.v`
/// testbench and NOT an emitted/generated netlist (`*_netlist.v`). Returns (filename, text), sorted.
fn candidate_v_files() -> Vec<(String, String)> {
    let dir = silicon_dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("benchmarks/silicon must exist")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            name.ends_with(".v") && !name.starts_with("tb_") && !name.ends_with("_netlist.v")
        })
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let name = p.file_name().unwrap().to_str().unwrap().to_string();
            let text =
                std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
            (name, text)
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// The meta-ratchet.
// ─────────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn every_bytecode_executing_core_has_a_fifty_million_dollar_ratchet() {
    let mapped: BTreeSet<&str> = CORE_RATCHETS.iter().map(|(core, _)| *core).collect();
    let excluded: BTreeSet<&str> = NON_CORE_EXECUTABLE.iter().map(|(f, _)| *f).collect();

    // Classify every candidate .v, printing the verdict for auditability.
    let candidates = candidate_v_files();
    let mut cores: Vec<String> = Vec::new();
    println!(
        "\n═══ META-RATCHET: bytecode-executing-core classification of benchmarks/silicon/*.v ═══\n\
         (a CORE = a top-level case(opcode|op) ISA dispatch with ≥{MIN_ISA_ARMS} arms AND an imem[pc] fetch)\n"
    );
    for (name, text) in &candidates {
        let arms = isa_dispatch_arm_count(text);
        let fetch = has_instruction_fetch(text);
        let is_core = is_bytecode_executing_core(text);
        if is_core {
            cores.push(name.clone());
            println!("  CORE      {name:<26} isa_arms={arms:<4} imem[pc]_fetch={fetch}");
        } else if arms >= MIN_ISA_ARMS || excluded.contains(name.as_str()) {
            // Print the borderline / explicitly-excluded shapes with WHY they are not cores, so a
            // reviewer can audit every near-miss.
            let reason = if let Some((_, r)) = NON_CORE_EXECUTABLE.iter().find(|(f, _)| f == name) {
                *r
            } else if !fetch {
                "no imem[pc] instruction fetch (fed an opcode wire, not a program)"
            } else {
                "fewer ISA-dispatch arms than the core threshold"
            };
            println!(
                "  excluded  {name:<26} isa_arms={arms:<4} imem[pc]_fetch={fetch}  — {reason}"
            );
        }
    }
    println!("\n  CORE_RATCHETS map:");
    for (core, ratchet) in CORE_RATCHETS {
        println!("    {core:<34} → {ratchet}");
    }
    println!();

    // ── GUARD 1 (completeness): every classified bytecode-executing core MUST be in CORE_RATCHETS.
    // A core on the NON_CORE_EXECUTABLE allowlist is a CONTRADICTION — the classifier proved it fetches
    // + decodes an ISA, so the allowlist entry is stale/wrong and must be removed (and a ratchet added).
    let mut unratcheted: Vec<String> = Vec::new();
    let mut contradictory_allowlist: Vec<String> = Vec::new();
    for core in &cores {
        if !mapped.contains(core.as_str()) {
            unratcheted.push(core.clone());
        }
        if excluded.contains(core.as_str()) {
            contradictory_allowlist.push(core.clone());
        }
    }
    assert!(
        contradictory_allowlist.is_empty(),
        "\n\n╔═══════════════════════════════════════════════════════════════════════════════════════╗\n\
         ║  META-RATCHET FAILURE — a `.v` on the NON_CORE_EXECUTABLE allowlist IS a bytecode core   ║\n\
         ╚═══════════════════════════════════════════════════════════════════════════════════════╝\n\
         The classifier proved these FETCH from imem[pc] AND decode a ≥{MIN_ISA_ARMS}-arm ISA dispatch — they\n\
         ARE bytecode-executing cores — yet they sit on the NON_CORE_EXECUTABLE exclusion allowlist:\n\
           {contradictory_allowlist:#?}\n\
         Remove each from NON_CORE_EXECUTABLE and ADD a $50M coverage ratchet to CORE_RATCHETS (mirror\n\
         fifty_million_dollar_test.rs). The allowlist may only suppress datapath/decoder SHAPES, never a\n\
         genuine fetch/decode core — that is the $50M silent-zero class.\n"
    );
    assert!(
        unratcheted.is_empty(),
        "\n\n╔═══════════════════════════════════════════════════════════════════════════════════════╗\n\
         ║  META-RATCHET FAILURE — a new bytecode-executing silicon core has NO $50M ratchet        ║\n\
         ╚═══════════════════════════════════════════════════════════════════════════════════════╝\n\
         These `benchmarks/silicon/*.v` files FETCH instructions from imem[pc] and DECODE a ≥{MIN_ISA_ARMS}-arm\n\
         `case(opcode|op)` ISA dispatch — they are bytecode-executing cores — but are NOT mapped in\n\
         CORE_RATCHETS to a $50M coverage ratchet:\n\
           {unratcheted:#?}\n\
         ADD a $50M coverage ratchet for each (mirror fifty_million_dollar_test.rs: prove every opcode the\n\
         core's encoder emits is DECODED in its real Verilog AND iverilog-EXERCISED ≡ the VM), then map it\n\
         here. OR, if it is genuinely NOT a bytecode executor, refine the classifier or add it to\n\
         NON_CORE_EXECUTABLE with a documented reason. NEVER ship a core whose decode arms aren't\n\
         ratcheted — that is the $50M silent-zero class this meta-ratchet exists to prevent.\n"
    );

    // ── GUARD 2 (the ratchets are real): every ratchet file named in CORE_RATCHETS must exist on disk.
    let mut missing_ratchets: Vec<String> = Vec::new();
    for (core, ratchet) in CORE_RATCHETS {
        let path = tests_dir().join(format!("{ratchet}.rs"));
        if !path.exists() {
            missing_ratchets.push(format!(
                "{core} → {ratchet}.rs (expected at {})",
                path.display()
            ));
        }
    }
    assert!(
        missing_ratchets.is_empty(),
        "\n\n╔═══════════════════════════════════════════════════════════════════════════════════════╗\n\
         ║  META-RATCHET FAILURE — a mapped $50M ratchet test file is MISSING from disk             ║\n\
         ╚═══════════════════════════════════════════════════════════════════════════════════════╝\n\
         CORE_RATCHETS names these ratchet test binaries, but their `.rs` files do not exist (a rename or\n\
         delete removed a $50M coverage ratchet, reopening the silent-zero class for its core):\n\
           {missing_ratchets:#?}\n\
         Restore the ratchet file (or fix the name in CORE_RATCHETS). A core's ratchet must ALWAYS exist.\n"
    );

    // ── NON-VACUOUS: the mapped set is ≥ 3 and contains the flagship soft_core.v.
    assert!(
        CORE_RATCHETS.len() >= 3,
        "META-RATCHET is vacuous: CORE_RATCHETS has {} entries, expected ≥ 3 (soft_core.v, \
         soft_core_scalar.v, fabric). The three known executable cores must always be present.",
        CORE_RATCHETS.len()
    );
    assert!(
        mapped.contains("soft_core.v"),
        "META-RATCHET is vacuous: the flagship `soft_core.v` is not in CORE_RATCHETS — its $50M ratchet \
         (fifty_million_dollar_test) must always be mapped."
    );

    // The classification itself must not have silently collapsed: at least the two on-disk cores
    // (soft_core.v + soft_core_scalar.v) MUST be detected by the classifier, else the heuristic broke
    // (e.g. the imem[pc] fetch pattern or the arm parser regressed) and GUARD 1 would pass VACUOUSLY.
    assert!(
        cores.contains(&"soft_core.v".to_string())
            && cores.contains(&"soft_core_scalar.v".to_string()),
        "META-RATCHET classifier regression: the on-disk fetch/decode cores were not detected — found \
         cores = {cores:#?}. soft_core.v AND soft_core_scalar.v must both classify as bytecode cores; if \
         they don't, the classifier (isa_dispatch_arm_count / has_instruction_fetch) is broken and GUARD \
         1 would pass vacuously. Fix the classifier — do NOT weaken the assertion."
    );
}

/// GUARD 3 — every bytecode-executing core must ALSO have a RANDOM-PROGRAM differential fuzz, not just a $50M
/// coverage ratchet. This session, ALL FIVE real silicon bugs were caught by random-program fuzzing, NONE by
/// the decode/exercise ratchets or the curated iverilog suites — so a core with a ratchet but no random fuzz
/// can silently harbor the curated-passes-but-random-fails class. Mirrors the ratchet guards: every classified
/// core MUST be in CORE_FUZZES (a new core → RED), and every mapped fuzz file MUST exist on disk.
#[test]
fn every_bytecode_executing_core_has_a_random_program_fuzz() {
    let mapped: BTreeSet<&str> = CORE_FUZZES.iter().map(|(core, _)| *core).collect();
    let cores: Vec<String> = candidate_v_files()
        .iter()
        .filter(|(_, text)| is_bytecode_executing_core(text))
        .map(|(name, _)| name.clone())
        .collect();

    // GUARD F1 (completeness): every classified bytecode core must have a random-program fuzz.
    let unfuzzed: Vec<&String> = cores
        .iter()
        .filter(|c| !mapped.contains(c.as_str()))
        .collect();
    assert!(
        unfuzzed.is_empty(),
        "\n\n╔═══════════════════════════════════════════════════════════════════════════════════════╗\n\
         ║  META-RATCHET FAILURE — a bytecode-executing silicon core has NO random-program fuzz     ║\n\
         ╚═══════════════════════════════════════════════════════════════════════════════════════╝\n\
         These cores are $50M-ratcheted (decode+exercise on a CURATED corpus) but have NO random-program\n\
         differential fuzz mapped in CORE_FUZZES:\n  {unfuzzed:#?}\n\
         Add a random-on-Verilog fuzz (mirror soft_core_rtl_fuzz_differential: generate random programs, run\n\
         them through the REAL Verilog under iverilog, assert ≡ VM) and map it here. The ratchet proves every\n\
         op is exercised on CURATED vectors; only a RANDOM fuzz catches the edge-case class that produced FIVE\n\
         real silicon bugs this session (negative-range loop, negative constants, map overwrite, closure args,\n\
         engine Call-multiply) — every one passed the curated tests. NEVER ship a core without both layers.\n"
    );

    // GUARD F2 (the fuzzes are real): every mapped fuzz file must exist on disk.
    let missing: Vec<String> = CORE_FUZZES
        .iter()
        .filter(|(_, f)| !tests_dir().join(format!("{f}.rs")).exists())
        .map(|(core, f)| format!("{core} → {f}.rs"))
        .collect();
    assert!(
        missing.is_empty(),
        "META-RATCHET (fuzz) FAILURE — a mapped random-fuzz test file is MISSING from disk (a rename/delete\n\
         removed a core's random-program fuzz, reopening the curated-passes-but-random-fails class):\n  \
         {missing:#?}\n  Restore the fuzz file or fix the name in CORE_FUZZES."
    );

    // NON-VACUOUS: ≥ 3 entries incl. the flagship soft_core.v, and the classifier still detects the 2 cores.
    assert!(
        CORE_FUZZES.len() >= 3 && mapped.contains("soft_core.v"),
        "META-RATCHET (fuzz) is vacuous: CORE_FUZZES needs ≥3 entries incl. soft_core.v (has {})",
        CORE_FUZZES.len()
    );
    assert!(
        cores.contains(&"soft_core.v".to_string()) && cores.contains(&"soft_core_scalar.v".to_string()),
        "META-RATCHET (fuzz) classifier regression: soft_core.v + soft_core_scalar.v must both classify as \
         cores — found {cores:#?}."
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// Classifier self-tests — lock the heuristic's boundary behavior so a future edit that would let a new
// unratcheted core slip past GUARD 1 (or misclassify a datapath as a core) goes RED here first. These
// use SYNTHETIC Verilog strings — no files, no simulation.
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// A synthetic bytecode core: fetches `imem[pc]` and decodes a ≥15-arm `case (opcode)` ISA dispatch.
fn synthetic_core() -> String {
    let mut s = String::from(
        "module newcore(input clk);\n  reg [7:0] pc;\n  wire [31:0] instr = imem[pc];\n  \
         wire [7:0] opcode = instr[31:24];\n  always @(posedge clk) begin\n    case (opcode)\n",
    );
    for n in 0..20u16 {
        s.push_str(&format!("      8'd{n}: pc <= pc + 8'd1;\n"));
    }
    s.push_str("      default: pc <= pc;\n    endcase\n  end\nendmodule\n");
    s
}

#[test]
fn classifier_flags_a_new_unratcheted_bytecode_core() {
    let core = synthetic_core();
    assert!(
        is_bytecode_executing_core(&core),
        "the synthetic fetch+decode core must classify as a bytecode-executing core"
    );
    // And it is NOT in CORE_RATCHETS → GUARD 1 would list it. Simulate GUARD 1's membership check.
    let mapped: BTreeSet<&str> = CORE_RATCHETS.iter().map(|(c, _)| *c).collect();
    assert!(
        !mapped.contains("newcore.v"),
        "sanity: the synthetic core name is not pre-mapped, so GUARD 1 would flag it RED"
    );
}

#[test]
fn classifier_rejects_a_pure_alu_datapath_fed_a_wire_opcode() {
    // 20 opcode arms but NO instruction fetch — opcode is an input port. This is exactly `alu.v`'s shape.
    let mut s = String::from(
        "module alu(input [7:0] opcode, input [31:0] a, input [31:0] b, output reg [31:0] y);\n  \
         always @(*) begin\n    case (opcode)\n",
    );
    for n in 0..20u16 {
        s.push_str(&format!("      8'd{n}: y = a + b;\n"));
    }
    s.push_str("      default: y = 32'd0;\n    endcase\n  end\nendmodule\n");
    assert!(
        isa_dispatch_arm_count(&s) >= MIN_ISA_ARMS,
        "the datapath has enough arms to be a decode-count candidate"
    );
    assert!(
        !has_instruction_fetch(&s),
        "a datapath fed a wire opcode has no imem[pc] fetch"
    );
    assert!(
        !is_bytecode_executing_core(&s),
        "an ALU datapath fed a wire opcode is NOT a bytecode-executing core — no fetch"
    );
}

#[test]
fn classifier_rejects_a_small_decoder_stub() {
    // A decoder like `isa_core_decoder.v`: an `input` opcode, few arms, no fetch.
    let s = [
        "module dec(input [7:0] opcode, output reg [2:0] sel);",
        "  always @(*) begin",
        "    case (opcode)",
        "      8'd0: sel = 3'd0;",
        "      8'd1: sel = 3'd1;",
        "      8'd2: sel = 3'd2;",
        "      8'd3: sel = 3'd3;",
        "      8'd4: sel = 3'd4;",
        "      default: sel = 3'd0;",
        "    endcase",
        "  end",
        "endmodule",
    ]
    .join("\n");
    let s = s.as_str();
    assert!(
        isa_dispatch_arm_count(s) < MIN_ISA_ARMS,
        "a 5-arm decoder is below the ISA-dispatch threshold"
    );
    assert!(
        !is_bytecode_executing_core(s),
        "a small decoder stub is not a bytecode-executing core"
    );
}

#[test]
fn classifier_ignores_a_soc_wrapper_that_only_instantiates_a_core() {
    // A SoC wrapper instantiates a core (`soft_core u_core(...)`) but does not itself decode the ISA:
    // it has NO top-level case(opcode) and NO imem[pc] of its own.
    let s = "module nexys_soc(input clk, input rx, output tx);\n  reg [7:0] uart_byte;\n  \
             soft_core u_core(.clk(clk), .rst(1'b0));\n  always @(posedge clk) uart_byte <= 8'd0;\n\
             endmodule\n";
    assert_eq!(
        isa_dispatch_arm_count(s),
        0,
        "a SoC wrapper has no top-level opcode dispatch of its own"
    );
    assert!(
        !is_bytecode_executing_core(s),
        "a SoC wrapper that only instantiates a core is not itself a bytecode-executing core"
    );
}

#[test]
fn classifier_does_not_count_a_nested_sub_dispatch_as_the_isa() {
    // A `case (rb)` builtin sub-dispatch nested inside a small `case (opcode)` must NOT push the ISA arm
    // count over the threshold — only the ENCLOSING-`opcode` labels count. Here the opcode dispatch has 3
    // arms; the nested rb dispatch has 20. The module must NOT be a core (3 < 15).
    let mut s = [
        "module m(input clk);",
        "  reg [7:0] pc;",
        "  wire [31:0] instr = imem[pc];",
        "  wire [7:0] opcode = instr[31:24];",
        "  always @(posedge clk) begin",
        "    case (opcode)",
        "      8'd0: pc <= pc + 8'd1;",
        "      8'd1: pc <= pc + 8'd1;",
        "      8'd38: begin case (rb)",
        "",
    ]
    .join("\n");
    for n in 0..20u16 {
        s.push_str(&format!("        8'd{n}: pc <= pc;\n"));
    }
    s.push_str("        default: pc <= pc;\n      endcase end\n      default: pc <= pc;\n    endcase\n  end\nendmodule\n");
    assert_eq!(
        isa_dispatch_arm_count(&s),
        3,
        "only the labels enclosed by case(opcode) count — the nested case(rb) arms do not"
    );
    assert!(
        !is_bytecode_executing_core(&s),
        "a 3-opcode module with a big nested builtin sub-dispatch is not a full ISA core"
    );
}
