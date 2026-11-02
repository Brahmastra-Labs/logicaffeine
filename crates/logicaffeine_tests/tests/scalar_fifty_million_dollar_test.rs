//! ═══════════════════════════════════════════════════════════════════════════════════════════════
//! THE SCALAR FIFTY-MILLION-DOLLAR TEST — the pit-of-success ratchet that makes a $50M *tape-out* defect
//! of the "model-proven but scalar-RTL-missing / scalar-RTL-unexercised" class IMPOSSIBLE on the SCALAR
//! CORE (`benchmarks/silicon/soft_core_scalar.v`) that actually goes to Sky130 silicon.
//! ═══════════════════════════════════════════════════════════════════════════════════════════════
//!
//! WHY THIS FILE EXISTS ($50M rationale — the THIRD core)
//! ──────────────────────────────────────────────────────
//! Two sibling ratchets already exist: `fifty_million_dollar_test.rs` (the full 115-op `soft_core.v`) and
//! `fabric_fifty_million_dollar_test.rs` (the spatial fabric). Each caught real silent-zero holes. The
//! THIRD core — the SCALAR TAPE-OUT core `soft_core_scalar.v`, Sky130-signoff-clean, the chip that loads
//! scalar bytecode over UART and runs on REAL silicon — had NO such ratchet. It was proven only by
//! `rtl.rs::encode_scalar_matches_vm` (the `.lg`→chip encoder ≡ the VM model) and `verify-nexys-soc.sh`
//! (iverilog on a HANDFUL of demos). So a scalar opcode the encoder EMITS but `soft_core_scalar.v` does
//! not decode is a silent-zero on the actual tape-out — uncaught if no demo happens to hit it (exactly the
//! 226/227/228 Word32 class that bit the full core).
//!
//! THE INVARIANT (strict — never weaken it to go green)
//! ─────────────────────────────────────────────────────
//! Every opcode the SCALAR encoder (`codegen_sva::scalar_asm`) can EMIT from a real function-free LOGOS
//! program must:
//!   (A) have a DECODE arm in `soft_core_scalar.v` (its own `case (opcode)` arm) OR in the `alu.v` datapath
//!       its `default:` arm routes to — else the op falls through to `alu.v`'s `default: y = 32'd0` and
//!       reads 0 on silicon, AND
//!   (B) be EXERCISED by at least one iverilog testbench that `verify-nexys-soc.sh` actually runs on the
//!       real `soft_core_scalar.v` ≡ the VM — so a decode arm is not merely PRESENT but PROVEN to run.
//!
//! EMITTED is computed by compiling a COMPREHENSIVE scalar-compatible corpus (every category the scalar
//! subset supports: reversible-core, LoadConst/Move, all arithmetic/bitwise/shift/comparison ALU ops, the
//! branches, Show/Halt/FailWith — the exact whitelist `scalar_asm::encode_op` accepts) through the REAL
//! `compile_to_scalar_words` and walking each image. The scalar word format is uniform ONE-WORD-PER-OP
//! (`[op:8][dst:8][a:8][b:8]`; LoadConst/Jump carry a 16-bit immediate in the low half — still one word),
//! so unlike the full core there are no multi-word ops and the opcode walk is a trivial stride-1 scan.
//!
//! ⚠️  DO NOT weaken this file to make a RED pass. A RED means the scalar chip is missing a decode arm (fix
//! `soft_core_scalar.v` / `alu.v`) or a `verify-nexys-soc.sh` corpus does not exercise an emitted op (add a
//! program to the scalar tb corpus). Alternatively an op the ENCODER emits that the scalar core genuinely
//! cannot run is a bug in the ENCODER's whitelist — fix the encoder to reject it, never the guard. The
//! whole point is that the invariant is STRICT: that is what makes the $50M tape-out defect impossible. ⚠️

#![cfg(not(target_arch = "wasm32"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use logicaffeine_compile::codegen_sva::scalar_asm::compile_to_scalar_words;
use logicaffeine_compile::vm::binisa::ISA_OPCODES;

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// op_width / mnemonic — the scalar core has a UNIFORM one-word instruction format (no multi-word ops),
// so every opcode strides by exactly 1. The mnemonic is the ISA table name (the scalar surface is entirely
// within the real 0..115 opcode range — no synthetic 200+ float/wide/word32 plane, that lives only on the
// full core).
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// Reverse-map a decoded scalar opcode byte to its mnemonic. The scalar surface is entirely real ISA ops.
fn mnemonic(oc: u8) -> String {
    if (oc as usize) < ISA_OPCODES.len() {
        return ISA_OPCODES[oc as usize].to_string();
    }
    format!("opcode-{oc}")
}

/// Walk one compiled scalar image and collect the opcode byte of every instruction. The scalar format is
/// one word per op, so the stride is always 1 — there is no embedded-immediate word that could be misread
/// as an opcode (LoadConst/Jump keep their immediate in the SAME word's low 16 bits).
fn opcodes_of_image(words: &[u32], into: &mut BTreeSet<u8>) {
    for w in words {
        into.insert((w >> 24) as u8);
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// THE EMITTED CORPUS — comprehensive across the scalar subset. Every program is function-free (the scalar
// encoder rejects user functions), uses NO division/modulo/power (rejected — needs the sequential divider
// the scalar core omits), NO collections/heap/structs/CRDTs/concurrency (rejected — outside the routable
// core). What remains — reversible-core, LoadConst/Move, arithmetic/bitwise/shift/comparison, control flow,
// Show/Halt — is exactly the whitelist `scalar_asm::encode_op` accepts. Each program is compiled through
// the REAL encoder; the union of their opcodes is the ground truth of what the scalar chip MUST run.
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// One representative per scalar op category. The comparison/arithmetic/bitwise/shift/control coverage is
/// deliberately exhaustive so EMITTED contains every opcode the encoder can produce.
const SCALAR_CORPUS: &[&str] = &[
    // LoadConst + Show + Halt (the const_7_42 demo shape)
    "## Main\n    Show 7.\n    Show 42.\n",
    // Add / Sub / Mul (the expr_13 demo shape) — 2 + 3 * 4 - 1
    "## Main\n    Show 2 + 3 * 4 - 1.\n",
    "## Main\n    Show (10 - 3) * 2.\n",
    // AddAssign path: `Set x to x + y` compiles to AddAssign, which the encoder REWRITES to the Add opcode.
    "## Main\n    Let mutable x be 10.\n    Set x to x + 5.\n    Show x.\n",
    // While loop with LtEq + JumpIfFalse + Jump (the sum_loop demo shape)
    "## Main\n    Let mutable sum be 0.\n    Let mutable i be 1.\n    While i is at most 5:\n        Set sum to sum + i.\n        Set i to i + 1.\n    Show sum.\n",
    // If/Otherwise with Gt (JumpIfFalse)
    "## Main\n    Let x be 7.\n    If x is greater than 5:\n        Show 1.\n    Otherwise:\n        Show 0.\n",
    // Countdown: Gt in a loop, Show inside
    "## Main\n    Let mutable n be 5.\n    While n is greater than 0:\n        Show n.\n        Set n to n - 1.\n    Show 0.\n",
    // Equality (Eq)
    "## Main\n    Let x be 5.\n    If x is equal to 5:\n        Show 1.\n    Otherwise:\n        Show 0.\n",
    // Inequality (NotEq)
    "## Main\n    Let x be 5.\n    If x is not 6:\n        Show 1.\n    Otherwise:\n        Show 0.\n",
    // Less-than (Lt)
    "## Main\n    Let x be 3.\n    If x is less than 5:\n        Show 1.\n    Otherwise:\n        Show 0.\n",
    // At-least (GtEq)
    "## Main\n    Let x be 5.\n    If x is at least 5:\n        Show 1.\n    Otherwise:\n        Show 0.\n",
    // At-most on a plain If (LtEq outside a loop)
    "## Main\n    Let x be 4.\n    If x is at most 5:\n        Show 1.\n    Otherwise:\n        Show 0.\n",
    // Bitwise AND / OR / XOR / NOT on plain Int (`&`, `|`, `xor`, `~`) — lower to BitAnd/BitOr/BitXor/Not.
    "## Main\n    Let a be 12.\n    Let b be 10.\n    Let c be a & b.\n    Show c.\n",
    "## Main\n    Let a be 12.\n    Let b be 10.\n    Let c be a | b.\n    Show c.\n",
    "## Main\n    Let a be 12.\n    Let b be 10.\n    Let c be a xor b.\n    Show c.\n",
    "## Main\n    Let a be 5.\n    Let mask be (1 shifted left by 3) - 1.\n    Let b be a & ~mask.\n    Show b.\n",
    // Shifts: Shl / Shr on plain Int (`shifted left by` / `shifted right by`).
    "## Main\n    Let a be 1.\n    Let b be a shifted left by 4.\n    Show b.\n",
    "## Main\n    Let a be 256.\n    Let b be a shifted right by 2.\n    Show b.\n",
    // JumpIfTrue path: an `Otherwise` with an inverted-sense branch, plus a while whose body Shows.
    "## Main\n    Let mutable i be 0.\n    While i is less than 3:\n        If i is equal to 1:\n            Show 100.\n        Set i to i + 1.\n    Show 999.\n",
    // Swap (RevSwap) — `Swap a and b` lowers to the reversible swap.
    "## Main\n    Let mutable a be 1.\n    Let mutable b be 2.\n    Swap a and b.\n    Show a.\n    Show b.\n",
    // Move: aliasing a binding to another register (`Let y be x`).
    "## Main\n    Let x be 41.\n    Let y be x.\n    Show y.\n",
    // Negative constant (LoadConst with the sign-extended low-16 immediate) + Sub.
    "## Main\n    Let x be 0 - 5.\n    Show x.\n",
    // A denser mixed program: comparisons + arithmetic + nested control (sumsq without div).
    "## Main\n    Let mutable total be 0.\n    Let mutable i be 1.\n    While i is at most 4:\n        Set total to total + i * i.\n        Set i to i + 1.\n    Show total.\n",
];

/// The `.hex` demo programs and the `.lg` sources under `benchmarks/silicon/programs` that COMPILE through
/// the scalar encoder (the function-free, division-free ones). Their opcodes join EMITTED so the ground
/// truth includes every op the on-silicon demos actually stream. The `.lg` sources that use div/mod or
/// functions (collatz, gcd, fib, primes) are honestly skipped — the scalar encoder rejects them, so they
/// are not part of the scalar emit surface (a distinct, honestly-tracked frontier, not a decode hole).
fn scalar_lg_sources() -> Vec<(String, String)> {
    let dir = silicon_dir().join("programs");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("benchmarks/silicon/programs must exist: {e}"))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |x| x == "lg"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for f in &files {
        let name = f.file_stem().unwrap().to_string_lossy().to_string();
        let src = std::fs::read_to_string(f).unwrap();
        out.push((name, src));
    }
    out
}

/// EMITTED = the union of opcode bytes over every scalar-corpus program, each compiled by the REAL scalar
/// encoder and walked. Returns opcode → the corpus program name that first emitted it, so a RED can point
/// at a concrete reproducer. A program the scalar encoder honestly REJECTS (functions / division / heap /
/// concurrency) is skipped — its op is not on the scalar chip at all (tracked elsewhere, not a decode hole).
fn emitted_opcodes() -> BTreeMap<u8, String> {
    let mut origin: BTreeMap<u8, String> = BTreeMap::new();
    let mut record = |name: &str, src: &str, must_compile: bool| {
        let words = match compile_to_scalar_words(src) {
            Ok(w) => w,
            Err(e) if must_compile => panic!(
                "EMITTED scalar-corpus program `{name}` must compile through the scalar encoder but did \
                 not — either the program strayed outside the scalar subset or the encoder regressed: {}\n  \
                 src={src:?}",
                e.lines().next().unwrap_or("")
            ),
            // A programs/*.lg that uses functions/div/heap is not scalar-runnable — honestly skipped.
            Err(_) => return,
        };
        let mut set = BTreeSet::new();
        opcodes_of_image(&words, &mut set);
        for oc in set {
            origin.entry(oc).or_insert_with(|| name.to_string());
        }
    };
    for (i, src) in SCALAR_CORPUS.iter().enumerate() {
        record(&format!("scalar[{i}]"), src, true);
    }
    for (name, src) in scalar_lg_sources() {
        record(&format!("programs/{name}"), &src, false);
    }
    origin
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// DECODED — every opcode the real `soft_core_scalar.v` can execute: an explicit `case (opcode)` arm, PLUS
// the general `alu.v` datapath its `default:` arm routes any unhandled opcode to. An opcode with NO
// soft_core_scalar.v arm AND not in `alu.v` falls through to `alu.v`'s `default: y = 32'd0` — a silent-zero.
// ─────────────────────────────────────────────────────────────────────────────────────────────────

fn silicon_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon")
}

/// Parse `soft_core_scalar.v` for the opcode arms of its single top-level `case (opcode)` dispatch. The
/// file has one `case (opcode)` in the execute `always` block. We track case/endcase nesting and the
/// selector of each open case (the SAME robust scanner as the full-core ratchet — mid-line `case (`,
/// comma-first continuation labels, and only labels whose innermost enclosing selector is exactly `opcode`)
/// so a future refactor that nests a sub-dispatch inside can never fool the parser into counting a
/// sub-ordinal as an opcode.
fn scalar_core_decoded() -> BTreeSet<u8> {
    let text = std::fs::read_to_string(silicon_dir().join("soft_core_scalar.v"))
        .expect("benchmarks/silicon/soft_core_scalar.v must exist");
    let mut decoded = BTreeSet::new();
    let collect_labels = |label: &str, into: &mut BTreeSet<u8>| {
        for tok in label.split(|c: char| !(c.is_ascii_digit() || c == '\'' || c == 'd')) {
            if let Some(num) = tok.strip_prefix("8'd") {
                if let Ok(n) = num.parse::<u8>() {
                    into.insert(n);
                }
            }
        }
    };
    let mut case_stack: Vec<String> = Vec::new();
    for raw in text.lines() {
        let line = match raw.find("//") {
            Some(idx) => &raw[..idx],
            None => raw,
        };
        if case_stack.last().map(String::as_str) == Some("opcode") {
            let trimmed = line.trim();
            if let Some(colon) = line.find(':') {
                collect_labels(&line[..colon], &mut decoded);
            } else if trimmed.ends_with(',') && trimmed.starts_with("8'd") {
                collect_labels(trimmed, &mut decoded);
            }
        }
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
    decoded
}

/// Parse `alu.v` for the opcodes its combinational datapath computes — the SAME datapath the scalar core's
/// `default:` arm routes to (`alu the_alu(.opcode(opcode) …)`). Identical parser to the full-core ratchet.
fn alu_decoded() -> BTreeSet<u8> {
    let text = std::fs::read_to_string(silicon_dir().join("alu.v"))
        .expect("benchmarks/silicon/alu.v must exist");
    let mut decoded = BTreeSet::new();
    let collect = |label: &str, into: &mut BTreeSet<u8>| {
        for tok in label.split(|c: char| !(c.is_ascii_digit() || c == '\'' || c == 'd')) {
            if let Some(num) = tok.strip_prefix("8'd") {
                if let Ok(n) = num.parse::<u8>() {
                    into.insert(n);
                }
            }
        }
    };
    for raw in text.lines() {
        let line = match raw.find("//") {
            Some(idx) => &raw[..idx],
            None => raw,
        };
        let trimmed = line.trim();
        if let Some(colon) = line.find(':') {
            let label = &line[..colon];
            if label.contains("default") {
                continue;
            }
            collect(label, &mut decoded);
        } else if trimmed.ends_with(',') && trimmed.starts_with("8'd") {
            collect(trimmed, &mut decoded);
        }
    }
    decoded
}

/// The full DECODED set the real scalar RTL can execute: soft_core_scalar.v opcode arms ∪ the alu.v datapath.
fn decoded_opcodes() -> BTreeSet<u8> {
    let mut d = scalar_core_decoded();
    d.extend(alu_decoded());
    d
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// EXERCISED — the union of opcodes across the iverilog testbenches that `verify-nexys-soc.sh` ACTUALLY
// runs on the real `soft_core_scalar.v`:
//   • the SCALAR SoC step [4/4] runs the demo `.hex` programs (sum_loop/expr_13/const_7_42) through
//     tb_nexys_run.v on `nexys_soc_scalar` (which instantiates soft_core_scalar), AND
//   • the standalone `tb_scalar_*.v` corpus (e2/expr/sumdown/sum_loop) that the scalar-tb iverilog step
//     runs directly on `soft_core_scalar.v` ≡ VM (their imem images come from the run_soft_core model).
// EXERCISED is the union of those images' opcodes — every op a `verify-nexys-soc.sh` run actually
// iverilogs on soft_core_scalar.v.
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// Parse a `.hex` program (one 8-hex-digit word per line, `//` comments allowed) into its opcode set.
fn hex_program_opcodes(path: &Path, into: &mut BTreeSet<u8>) {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    for line in text.lines() {
        let l = line.trim();
        let hex: String = l.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        if hex.len() < 8 {
            continue; // blank / comment-only line
        }
        if let Ok(word) = u32::from_str_radix(&hex[..8], 16) {
            into.insert((word >> 24) as u8);
        }
    }
}

/// Parse every `dut.imem[N] = 32'hXXXXXXXX;` line of a checked-in `tb_scalar_*.v` file into its opcode set.
fn tb_scalar_opcodes(path: &Path, into: &mut BTreeSet<u8>) {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    for line in text.lines() {
        let l = line.trim();
        let Some(after) = l.strip_prefix("dut.imem[") else {
            continue;
        };
        let Some(_rb) = after.find(']') else { continue };
        let Some(hpos) = l.find("32'h") else { continue };
        let hex: String = l[hpos + 4..]
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        if let Ok(word) = u32::from_str_radix(&hex, 16) {
            into.insert((word >> 24) as u8);
        }
    }
}

/// The exact demo `.hex` programs `verify-nexys-soc.sh` step [4/4] runs on `nexys_soc_scalar`.
const SCALAR_SOC_DEMOS: &[&str] = &["sum_loop.hex", "expr_13.hex", "const_7_42.hex"];

/// EXERCISED = the scalar SoC demo `.hex` programs ∪ the standalone `tb_scalar_*.v` corpus — every opcode a
/// `verify-nexys-soc.sh` run actually iverilogs on `soft_core_scalar.v` ≡ the VM.
fn exercised_opcodes() -> BTreeSet<u8> {
    let mut set = BTreeSet::new();
    let progs = silicon_dir().join("programs");
    for demo in SCALAR_SOC_DEMOS {
        hex_program_opcodes(&progs.join(demo), &mut set);
    }
    // The standalone tb_scalar_*.v differential corpus run directly on soft_core_scalar.v.
    let dir = silicon_dir();
    let mut tbs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("benchmarks/silicon must exist")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map_or(false, |n| n.starts_with("tb_scalar_") && n.ends_with(".v"))
        })
        .collect();
    tbs.sort();
    for tb in &tbs {
        tb_scalar_opcodes(tb, &mut set);
    }
    set
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// THE GUARDS
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// ★ GUARD A — DECODE COVERAGE (the killer guard) ★
/// Every opcode the SCALAR encoder can EMIT from a real function-free LOGOS program MUST have a decode arm
/// on the real scalar silicon (`soft_core_scalar.v` opcode arms, OR the `alu.v` datapath its `default:` arm
/// routes to). A hole here is a $50M silent-zero on the Sky130 tape-out: the op reads 0 and the bug ships.
#[test]
fn every_emitted_scalar_opcode_is_decoded_in_soft_core_scalar_v() {
    let emitted = emitted_opcodes();
    let decoded = decoded_opcodes();

    println!("── EMITTED scalar opcode surface (scalar encoder → chip words) ──");
    for (oc, origin) in &emitted {
        let mark = if decoded.contains(oc) { "  " } else { "‼ " };
        println!(
            "  {mark}{oc:>3}  {:<14} (first seen in {origin})",
            mnemonic(*oc)
        );
    }
    println!(
        "  ({} distinct emitted, {} decoded in soft_core_scalar.v + alu.v)",
        emitted.len(),
        decoded.len()
    );

    let holes: Vec<(u8, String, String)> = emitted
        .iter()
        .filter(|(oc, _)| !decoded.contains(oc))
        .map(|(oc, origin)| (*oc, mnemonic(*oc), origin.clone()))
        .collect();

    assert!(
        holes.is_empty(),
        "\n★ $50M SCALAR DECODE HOLE ★ — {} opcode(s) the SCALAR encoder EMITS from real LOGOS programs have \
         NO decode arm in benchmarks/silicon/soft_core_scalar.v (nor in alu.v). On the Sky130 tape-out each \
         reads 0 (falls through soft_core_scalar.v's `default:` → alu.v's `default: y = 32'd0`). This is the \
         226/227/228 class: model-proven `chip == VM` in Rust, yet SILENTLY UNDECODED in the real scalar \
         Verilog. Add the missing `8'dN:` decode arm(s) to soft_core_scalar.v (or the alu.v datapath) — or, \
         if the scalar core genuinely cannot run the op, fix scalar_asm's whitelist to REJECT it. NEVER \
         weaken this guard.\n\
         Missing opcodes (opcode  mnemonic  first-seen-in):\n{}",
        holes.len(),
        holes
            .iter()
            .map(|(oc, mn, origin)| format!("    {oc:>3}  {mn:<14} {origin}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// ★ GUARD B — IVERILOG-EXERCISE COVERAGE ★
/// Every opcode the SCALAR encoder can EMIT must be in at least one iverilog testbench that a
/// `verify-nexys-soc.sh` run actually executes on the real `soft_core_scalar.v` ≡ VM — so a decode arm is
/// not merely PRESENT but PROVEN to run correctly on the scalar RTL. Fails LOUD listing every
/// emitted-but-unexercised opcode with a reproducing corpus program.
#[test]
fn every_emitted_scalar_opcode_is_exercised_by_an_iverilog_testbench() {
    let emitted = emitted_opcodes();
    let exercised = exercised_opcodes();

    println!("── EXERCISED scalar opcode surface (verify-nexys-soc.sh iverilog on soft_core_scalar.v) ──");
    for oc in &exercised {
        println!("  {oc:>3}  {}", mnemonic(*oc));
    }
    println!(
        "  ({} distinct exercised by an iverilog tb on soft_core_scalar.v)",
        exercised.len()
    );

    let unexercised: Vec<(u8, String, String)> = emitted
        .iter()
        .filter(|(oc, _)| !exercised.contains(oc))
        .map(|(oc, origin)| (*oc, mnemonic(*oc), origin.clone()))
        .collect();

    assert!(
        unexercised.is_empty(),
        "\n★ SCALAR IVERILOG-EXERCISE GAP ★ — {} opcode(s) the SCALAR encoder EMITS are in NO iverilog \
         testbench that a `verify-nexys-soc.sh` run executes on the real soft_core_scalar.v. Each is \
         model-proven (encode_scalar_matches_vm), but no RTL differential actually RUNS it on the scalar \
         core ≡ VM — so a hardware bug in its arm would go uncaught by verify-nexys-soc.sh. Add a program \
         exercising it to the scalar tb corpus (a `tb_scalar_*.v` whose imem image comes from the \
         run_soft_core model, wired into verify-nexys-soc.sh). NEVER weaken this guard.\n\
         Unexercised opcodes (opcode  mnemonic  first-seen-in):\n{}",
        unexercised.len(),
        unexercised
            .iter()
            .map(|(oc, mn, origin)| format!("    {oc:>3}  {mn:<14} {origin}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// A sanity floor so the guards can never go vacuous by a corpus / parser regression that empties a set:
/// the EMITTED surface is broad by construction, the DECODED / EXERCISED sets are non-trivial, and the
/// core scalar-arithmetic ops (Add 8, Sub 10, Mul 11) plus the loop-control ops (LtEq 27, JumpIfFalse 35,
/// Jump 34) and Show 89 / Halt 43 — the exact ops the on-silicon demos depend on — MUST be in EMITTED.
#[test]
fn the_guarded_scalar_sets_are_non_vacuous() {
    let emitted = emitted_opcodes();
    let decoded = decoded_opcodes();
    let exercised = exercised_opcodes();
    assert!(
        emitted.len() >= 20,
        "EMITTED shrank to {} distinct scalar opcodes — the comprehensive scalar corpus regressed; the \
         guards would be vacuous",
        emitted.len()
    );
    assert!(
        decoded.len() >= 20,
        "DECODED shrank to {} — the soft_core_scalar.v / alu.v parser regressed",
        decoded.len()
    );
    assert!(
        exercised.len() >= 7,
        "EXERCISED shrank to {} — the scalar tb / demo corpus parser regressed",
        exercised.len()
    );
    // The scalar spine: arithmetic + loop control + Show/Halt. These MUST be watched by EMITTED.
    for (oc, name) in [
        (8u8, "Add"),
        (10, "Sub"),
        (11, "Mul"),
        (27, "LtEq"),
        (34, "Jump"),
        (35, "JumpIfFalse"),
        (89, "Show"),
        (43, "Halt"),
    ] {
        assert!(
            emitted.contains_key(&oc),
            "the scalar op {oc} ({name}) is no longer in EMITTED — the scalar corpus stopped watching it. \
             Restore the corpus program that emits it.",
        );
    }
}
