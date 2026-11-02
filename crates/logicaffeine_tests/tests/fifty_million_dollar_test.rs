//! ═══════════════════════════════════════════════════════════════════════════════════════════════
//! THE FIFTY-MILLION-DOLLAR TEST — the pit-of-success ratchet that makes a $50M tape-out defect of the
//! "model-proven but RTL-missing / RTL-unexercised" class IMPOSSIBLE.
//! ═══════════════════════════════════════════════════════════════════════════════════════════════
//!
//! WHY THIS FILE EXISTS ($50M rationale)
//! ─────────────────────────────────────
//! An opcode can be proven `model == VM` — every Rust differential (`universal_completeness_lock`,
//! `soft_core_op_support_lock`, `soft_core_width_parity`) GREEN — and STILL be silently UNDECODED in the
//! real Verilog `benchmarks/silicon/soft_core.v`. On tape-out silicon such an op reads 0: the datapath
//! computes nothing, and the bug ships. At ~$50M per mask-set respin, a single such hole is catastrophic.
//!
//! THIS ALREADY HAPPENED. The Word32 arithmetic ops 226/227/228 (`Word32Add`/`Sub`/`Mul`, used by the
//! SHA-256 message schedule) were lowered on the model AND the encoder — `word32_matches_vm` was GREEN —
//! yet `soft_core.v` had NO decode arm for them, so they fell through its `default:` → `alu.v`, whose own
//! `default: y = 32'd0` returned 0. `verify-word32-soft-core.sh` failed (`wrapping_add` want 1 got 0),
//! but only because ITS curated corpus happened to exercise a wrapping add. No single guard covered the
//! CLASS: each `verify-*.sh` iverilogs only its own hand-picked programs.
//!
//! THE INVARIANT (strict — never weaken it to go green)
//! ─────────────────────────────────────────────────────
//! Every opcode the soft-core encoder can EMIT from a real LOGOS program must:
//!   (A) have a DECODE arm in `soft_core.v` (its own `case(opcode)` arm, its op-38 builtin sub-dispatch,
//!       its reversible-core arm, OR the general `alu.v` datapath the `default:` arm routes to), AND
//!   (B) be EXERCISED by at least one iverilog testbench corpus that a `verify-*.sh` run actually runs on
//!       the real RTL ≡ the VM.
//!
//!   • GUARD A (decode coverage) — the KILLER guard — asserts EMITTED ⊆ DECODED. It parses the real
//!     `soft_core.v` + `alu.v` text for their decode arms; a missing arm is listed by opcode + mnemonic.
//!     This catches 226/227/228 the instant they are emitted but undecoded, with NO simulator needed.
//!   • GUARD B (iverilog-exercise coverage) asserts EMITTED ⊆ EXERCISED. EXERCISED is the union of the
//!     opcodes in the `emit_*_tb` corpora the `verify-*.sh` scripts glob (the `tb_gen_*.v` / `tb_chip_*`
//!     images), so every emitted op is actually run on `soft_core.v` ≡ VM by a real iverilog pass.
//!
//! EMITTED is computed by compiling a COMPREHENSIVE corpus (the same union of programs the catalog +
//! rtl.rs corpora + the on-silicon FPGA benches draw from — every `Lowered` op category) through the REAL
//! `compile_to_full_words` and walking each image with the correct multi-word `op_width` (so an immediate
//! is never misread as an opcode), exactly like `op_coverage_lock::exercised_opcodes`.
//!
//! ⚠️  DO NOT weaken this file to make a RED pass. A RED means the chip is missing a decode arm (fix
//! `soft_core.v` / `alu.v`) or a `verify-*.sh` corpus does not exercise an emitted op (add a program).
//! The whole point is that the invariant is STRICT: that is what makes the $50M defect impossible. ⚠️

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use logicaffeine_compile::codegen_sva::full_asm::compile_to_full_words;
use logicaffeine_compile::vm::binisa::ISA_OPCODES;

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// op_width / mnemonic — the SAME multi-word walk + reverse-map as op_coverage_lock (kept in lockstep
// with `full_asm::imem_width` and the model's PC advancement, so the opcode walk never misreads an
// embedded immediate as an opcode).
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// The multi-word chip-op table: `Call` (37) = 2 words, `LoadConstWide` (200) = 2, `FConst` (201) = 3,
/// `TextConst` (216) = 2, `LoadConstWide64` (222) = 3. Every other opcode is a single word.
fn op_width(oc: u8) -> usize {
    match oc {
        37 => 2,
        200 => 2,
        201 => 3,
        216 => 2,
        222 => 3,
        _ => 1,
    }
}

/// Reverse-map a decoded chip opcode byte to its mnemonic: real ops (0..115) index the ISA table; the
/// synthetic float/heap/wide/word32/text-builtin plane (200+) is named explicitly.
fn mnemonic(oc: u8) -> String {
    if (oc as usize) < ISA_OPCODES.len() {
        return ISA_OPCODES[oc as usize].to_string();
    }
    match oc {
        200 => "LoadConstWide",
        201 => "FConst",
        202 => "FAdd",
        203 => "FSub",
        204 => "FMul",
        205 => "FDiv",
        206 => "FSqrt",
        207 => "FLt",
        208 => "FEq",
        209 => "IntToF",
        210 => "FToInt",
        211 => "FMove",
        212 => "ShowF",
        213 => "FListPush",
        214 => "FIndex",
        215 => "FSetIndex",
        216 => "TextConst",
        217 => "ShowText",
        218 => "TextEq",
        220 => "FormatValueText",
        221 => "ShowWord32",
        222 => "LoadConstWide64",
        223 => "MapIndexInt",
        224 => "MapSetIndexInt",
        225 => "FormatValueFloat",
        226 => "Word32Add",
        227 => "Word32Sub",
        228 => "Word32Mul",
        229 => "FLe",
        230 => "FNe",
        231 => "FIterNext",
        233 => "Chr",
        234 => "TextContains",
        235 => "RepeatSeq",
        236 => "ParseInt",
        237 => "TextBytes",
        _ => "unknown-synthetic",
    }
    .to_string()
}

/// Walk one compiled chip image and collect the opcode byte of every instruction, advancing the pc by
/// the correct `op_width` so a following immediate word is never misread as an opcode.
fn opcodes_of_image(words: &[u32], into: &mut BTreeSet<u8>) {
    let mut pc = 0usize;
    while pc < words.len() {
        let oc = (words[pc] >> 24) as u8;
        into.insert(oc);
        pc += op_width(oc);
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// THE EMITTED CORPUS — comprehensive, one representative per `Lowered` op category, the SAME union the
// catalog `LOWERED_CORPUS`, the rtl.rs differential corpora, and `universal_completeness_lock` draw
// from. Every program is compiled through the REAL encoder; the union of their opcodes is the ground
// truth of what the chip MUST run. A `"conc"`/`"net"` kind is irrelevant to compilation (the encoder
// lowers the chip words regardless of which VM oracle drives them at runtime); the kind is dropped here.
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// The catalog's representative-per-category corpus (mirrors `soft_core_op_support_lock::LOWERED_CORPUS`,
/// which is `const` in a sibling test binary and cannot be imported). Kept broad on purpose.
const CATALOG_CORPUS: &[&str] = &[
    "## Main\n    Show 2 + 3 * 4 - 1.\n",
    "## Main\n    Show 100 / 7.\n    Show 100 % 7.\n",
    "## Main\n    Show 2 ** 10.\n",
    "## Main\n    Let x be 7.\n    If x is greater than 5:\n        Show 1.\n    Otherwise:\n        Show 0.\n",
    "## To addbase (n: Int) -> Int:\n    Return n + base.\n## Main\n    Let base be 1000.\n    Show addbase(5).\n",
    "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n## Main\n    Show fib(10).\n",
    "## Main\n    Let mutable xs be [].\n    Let mutable i be 1.\n    While i is at most 3:\n        Push i * i to xs.\n        Set i to i + 1.\n    Set item 1 of xs to 42.\n    Pop from xs into y.\n    Show y.\n    Show the length of xs.\n",
    "## Main\n    Let a be [1, 2].\n    Let b be [3, 4].\n    Let c be a followed by b.\n    Let d be copy of c.\n    Show item 3 of d.\n",
    "## Main\n    Let mutable s be 0.\n    Repeat for x from 1 to 5:\n        Set s to s + x.\n    Show s.\n",
    "## Main\n    Let evens be a new Set of Int.\n    Add 2 to evens.\n    Add 4 to evens.\n    Add 2 to evens.\n    Show length of evens.\n",
    "## Main\n    Let mutable m be a new Map of Int to Int.\n    Set item 1 of m to 5.\n    Set item 2 of m to 9.\n    Show item 1 of m + item 2 of m.\n",
    "## Main\n    Let pairs be [(1, 10), (2, 20)].\n    Repeat for (a, b) in pairs:\n        Show a.\n        Show b.\n",
    "## Main\n    Show 3.5 + 1.25.\n    Show 2.5 * 4.0.\n",
    "## Main\n    Let x be 3.0.\n    Let y be 4.0.\n    Let d be sqrt(x * x + y * y).\n    Show d.\n",
    "## Main\n    Let s be \"ab\" + \"cd\".\n    Show s.\n",
    "## Main\n    Let x be 3.\n    Show \"depth {x} deep\".\n",
    "## Main\n    Show intOfWord32(word_and(word32(4042322160), word32(267399664))).\n",
    "## Main\n    Let mutable a be 1.\n    Let mutable b be 2.\n    Swap a and b.\n    Show a.\n    Show b.\n",
    "## Main\n    Let a be 1.0.\n    Let b be 1.0.\n    If a is approximately b:\n        Show 1.\n    Otherwise:\n        Show 0.\n",
    "## Main\n    Let xs be [10, 20, 30, 40].\n    Let ys be items 2 through 3 of xs.\n    Show item 1 of ys.\n    Show item 2 of ys.\n    Show length of ys.\n",
    "## Main\n    Let a be a new Set of Int.\n    Add 1 to a.\n    Add 2 to a.\n    Let b be a new Set of Int.\n    Add 2 to b.\n    Add 3 to b.\n    Let c be a union b.\n    Show length of c.\n",
    "## Main\n    Let a be a new Set of Int.\n    Add 1 to a.\n    Add 2 to a.\n    Let b be a new Set of Int.\n    Add 2 to b.\n    Add 3 to b.\n    Let c be a intersection b.\n    Show length of c.\n",
    "## Main\n    Let a be a new Set of Int.\n    Add 1 to a.\n    Add 2 to a.\n    Add 3 to a.\n    Remove 2 from a.\n    Show length of a.\n",
    "## A Bag has:\n    An items: Seq of Int.\n\n## Main\n    Let mutable b be a new Bag with items (a new Seq of Int).\n    Push 7 to b's items.\n    Push 9 to b's items.\n    Show length of b's items.\n    Show item 1 of b's items.\n",
    "## Main\n    Let a be args().\n    Show length of a.\n",
    "## Main\n    Show abs(0 - 7).\n    Show abs(5).\n",
    "## Main\n    Show min(3, 9).\n    Show max(3, 9).\n",
    "## Main\n    Show count_ones(12).\n    Show count_ones(255).\n",
    "## Main\n    Let x be 0.0 - 2.5.\n    Show abs(x).\n",
    "## Main\n    Let a be 1.5.\n    Let b be 2.5.\n    Show min(a, b).\n    Show max(a, b).\n",
    "## Main\n    Let x be 2.7.\n    Show floor(x).\n    Show ceil(x).\n    Show round(x).\n",
    "## Main\n    Show chr(65).\n    Show chr(97).\n    Show chr(955).\n",
    "## Main\n    Let s be \"hello world\".\n    If s contains \"world\":\n        Show 1.\n    Otherwise:\n        Show 0.\n",
    "## Main\n    Let xs be 5 copies of 7.\n    Show length of xs.\n    Show item 1 of xs.\n    Show item 5 of xs.\n",
    "## Main\n    Show parseInt(\"42\").\n    Show parseInt(\"-7\").\n",
    "## Definition\nA Point has:\n    x: Int.\n    y: Int.\n## Main\n    Let p be a new Point with x 10 and y 20.\n    Show p's y.\n",
    "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n## Main\nLet s be a new Circle with radius 42.\nInspect s:\n    When Circle (r): Show r.\n    When Rectangle (w, h): Show w.\n",
    "## Definition\nA Tally has:\n    n: a Counter.\n## Main\n    Let mutable t be a new Tally.\n    Increase t's n by 5.\n    Decrease t's n by 2.\n    Show t's n.\n",
    "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nShow apply((n: Int) -> n * 2, 21).\n",
    "## Main\n    Show \"depth \" + 3.\n",
    // concurrency (chip words emit regardless of oracle): channels + handle-launch + non-blocking try + select + sleep
    "## To worker (ch: Int):\n    Send 7 into ch.\n## Main\n    Let jobs be a Pipe of Int.\n    Let h be Launch a task to worker with jobs.\n    Receive first from jobs.\n    Stop h.\n    Show first.\n",
    "## Main\n    Let ch be a Pipe of Int.\n    Try to send 99 into ch.\n    Try to receive x from ch.\n    Show x.\n",
    "## To producer (ch: Int):\n    Send 5 into ch.\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to producer with jobs.\n    Await the first of:\n        Receive x from jobs:\n            Show x.\n        After 10 seconds:\n            Show 0.\n",
    "## Main\n    Sleep 100.\n    Show \"done\".\n",
    // networking (offline single-node) — the chip words emit; the VM's net oracle drives them at runtime
    "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send 42 to peer.\n    Await response from peer into got.\n    Show got.\n",
    "## Main\n    Connect to \"ws://127.0.0.1:7777\".\n    Show \"connected\".\n",
    "## Definition\nA Tally has:\n    n: a Counter.\n## Main\n    Let mutable state be a new Tally.\n    Increase state's n by 3.\n    Sync state on \"topic\".\n    Show state's n.\n",
];

/// The rtl.rs differential corpora — the EXACT programs the `emit_*_tb` generators write to iverilog,
/// replicated here (they are `#[cfg(test)]`-private to the compile crate) so the EMITTED set includes
/// every op these Word32 / Text / float-compare / text-seq-builtin programs drive.
const WORD32_CORPUS: &[&str] = &[
    "## Main\nShow intOfWord32(word32(3820012610)).\n",
    "## Main\nShow word32(4294967295).\n",
    "## Main\nShow intOfWord32(word32Shr(word32(2147483648), 3)).\n",
    "## Main\nShow intOfWord32(rotr(word32(1), 1)).\n",
    "## Main\nShow intOfWord32(rotl(word32(2147483648), 1)).\n",
    "## Main\nLet a be word32(4042322160).\nLet b be word32(267399664).\nLet c be word_and(a, b).\nLet d be word_or(word32(252645135), word32(1)).\nLet e be c xor d.\nShow intOfWord32(word_not(e)).\n",
    // The wrapping ADD across 2^32 — Word32Add (226). This is the program that failed `want 1 got 0`.
    "## Main\nLet a be word32(4294967295).\nLet b be word32(2).\nShow intOfWord32(a + b).\n",
    // The wrapping SUB below zero — Word32Sub (227). 1 -ᵤ 3 = 2^32 - 2 (the ring op must NOT go negative).
    "## Main\nLet a be word32(1).\nLet b be word32(3).\nShow intOfWord32(a - b).\n",
    // The wrapping MUL past 2^32 — Word32Mul (228). 3 · 2_000_000_000 = 6e9 mod 2^32 (must truncate, not promote to BigInt).
    "## Main\nLet a be word32(3).\nLet b be word32(2000000000).\nShow intOfWord32(a * b).\n",
    "## To sig (x: Word32) -> Word32:\n    Return rotr(x, 7) xor rotr(x, 18) xor word32Shr(x, 3).\n## Main\nShow intOfWord32(sig(word32(305419896))).\n",
    "## Main\nLet mutable xs be a new Seq of Word32.\nLet mutable i be 1.\nWhile i is at most 4:\n    Push word32(i * i) to xs.\n    Set i to i + 1.\nShow intOfWord32(item 3 of xs).\n",
];

const TEXT_CORPUS: &[&str] = &[
    "## Main\nShow \"hello\".\n",
    "## Main\nLet mutable s be \"\".\nSet s to s + \"ok\".\nShow s.\n",
    "## Main\nLet s be \"ab\" + \"cd\".\nShow s.\n",
    "## Main\nShow \"depth \" + 3.\n",
    "## Main\nShow 123 + \"!\".\n",
    "## Main\nLet x be 0 - 7.\nShow \"n = \" + x.\n",
    "## Main\nLet mutable r be \"\".\nLet mutable i be 0.\nWhile i is less than 3:\n    Set r to r + i + \" \".\n    Set i to i + 1.\nShow \"[\" + r + \"]\".\n",
    "## Main\nLet t be \"a b c\".\nLet mutable spaces be 0.\nLet mutable j be 1.\nWhile j is at most the length of t:\n    If item j of t equals \" \":\n        Set spaces to spaces + 1.\n    Set j to j + 1.\nShow spaces.\n",
    "## To bang (s: Text) -> Text:\n    Return s + \"!\".\n## Main\nShow bang(\"hi\").\n",
    "## Main\nLet d be 5.\nLet c be 63.\nShow \"stretch tree of depth \" + d + \" check: \" + c.\n",
];

const FLOAT_HEAP_CORPUS: &[&str] = &[
    "## Main\nLet mutable xs be a new Seq of Float.\nPush 1.5 to xs.\nPush 2.25 to xs.\nLet a be item 1 of xs.\nLet b be item 2 of xs.\nShow a plus b.\n",
    "## Main\nLet mutable base be a new Seq of Float.\nPush 1.5 to base.\nPush 2.5 to base.\nLet mutable dup be base.\nPush 9.5 to dup.\nLet x be item 1 of base.\nLet y be item 2 of base.\nShow x plus y.\n",
    "## Main\nLet mutable xs be a new Seq of Float.\nLet mutable i be 1.\nWhile i is at most 6:\n    Push i * 0.5 to xs.\n    Set i to i + 1.\nLet mutable total be 0.0.\nSet i to 1.\nWhile i is at most 6:\n    Set total to total + item i of xs.\n    Set i to i + 1.\nShow total.\n",
    "## Main\nLet mutable xs be a new Seq of Float.\nPush 1.0 to xs.\nPush 2.0 to xs.\nPush 3.0 to xs.\nSet item 2 of xs to 7.25.\nShow item 2 of xs.\n",
    "## To firstTwo (v: Seq of Float) -> Float:\n    Return item 1 of v + item 2 of v.\n## Main\nLet mutable xs be a new Seq of Float.\nPush 1.25 to xs.\nPush 1.75 to xs.\nShow firstTwo(xs).\n",
];

const FLOAT_CMP_ITER_CORPUS: &[&str] = &[
    "## Main\nLet a be 1.5.\nLet b be 2.5.\nIf a is at most b:\n    Show 1.\nOtherwise:\n    Show 0.\nIf b is at most a:\n    Show 1.\nOtherwise:\n    Show 0.\nIf b is at most b:\n    Show 1.\nOtherwise:\n    Show 0.\n",
    "## Main\nLet a be 3.25.\nLet b be 1.75.\nIf a is at least b:\n    Show 1.\nOtherwise:\n    Show 0.\nIf b is at least a:\n    Show 1.\nOtherwise:\n    Show 0.\n",
    "## Main\nLet a be 1.5.\nLet b be 2.5.\nIf a is not b:\n    Show 1.\nOtherwise:\n    Show 0.\nIf b is not b:\n    Show 1.\nOtherwise:\n    Show 0.\n",
    "## Main\nLet mutable xs be a new Seq of Float.\nPush 1.5 to xs.\nPush 2.25 to xs.\nPush 0.25 to xs.\nLet mutable total be 0.0.\nRepeat for v in xs:\n    Set total to total + v.\nShow total.\n",
    "## Main\nLet mutable xs be a new Seq of Float.\nPush 1.0 to xs.\nPush 5.0 to xs.\nPush 2.0 to xs.\nPush 9.0 to xs.\nLet t be 3.0.\nLet mutable count be 0.\nRepeat for v in xs:\n    If v is at most t:\n        Set count to count + 1.\nShow count.\n",
    "## Main\nLet a be 1.0.\nLet b be 1.0.\nIf a is approximately b:\n    Show 1.\nOtherwise:\n    Show 0.\nLet c be 0.1 + 0.2.\nLet d be 0.3.\nIf c is approximately d:\n    Show 1.\nOtherwise:\n    Show 0.\nLet e be 1.0.\nLet f be 2.0.\nIf e is approximately f:\n    Show 1.\nOtherwise:\n    Show 0.\n",
];

const TEXT_SEQ_BUILTIN_CORPUS: &[&str] = &[
    "## Main\n    Show chr(65).\n    Show chr(97).\n    Show chr(10).\n    Show chr(955).\n",
    "## Main\n    Let s be \"hello world\".\n    If s contains \"world\":\n        Show 1.\n    Otherwise:\n        Show 0.\n    If s contains \"zzz\":\n        Show 111.\n    Otherwise:\n        Show 222.\n",
    "## Main\n    If contains(\"logos\", \"go\"):\n        Show 1.\n    Otherwise:\n        Show 0.\n    If contains(\"logos\", \"\"):\n        Show 3.\n    Otherwise:\n        Show 4.\n",
    "## Main\n    Let xs be 5 copies of 7.\n    Show length of xs.\n    Show item 1 of xs.\n    Show item 5 of xs.\n",
    "## Main\n    Show parseInt(\"42\").\n    Show parseInt(\"-7\").\n    Show parseInt(\"0\").\n    Show parseInt(\"+5\").\n    Show parseInt(\"007\").\n    Show parseInt(\"  123  \").\n",
    "## Main\n    Show 1.\n    Show parseInt(\"abc\").\n    Show 2.\n",
    "## Main\n    Let b be text_bytes(\"aé€😀\").\n    Show length of b.\n    Repeat for x in b:\n        Show x.\n",
];

/// The soft-core FPGA benchmark directory — the REAL `.lg` programs that stream to silicon over UART and
/// feed the `emit_fpu_soft_core_tb` / chip-sized-benchmark differentials.
fn bench_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/fpga/bench")
}

/// A deliberate encoder guard (mirrors `rtl::tests::chip_lowering_pending`): a `.lg` bench using an op
/// category with no SOUND full-core lowering yet is REJECTED loudly rather than silently miscompiled.
/// Such a program cannot be part of the emitted ground truth — the op it needs is not on the chip at all
/// (a distinct, honestly-tracked frontier, not a decode hole) — so it is skipped here exactly as the
/// model tests skip it.
fn chip_lowering_pending(err: &str) -> bool {
    err.contains("no sound full-core chip lowering yet")
}

/// The set of `.lg` bench sources that DO compile through the full-core encoder (skipping the honest
/// not-yet-lowered ones). Sorted for a deterministic union.
fn bench_sources() -> Vec<(String, String)> {
    let dir = bench_dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("benchmarks/silicon/fpga/bench must exist: {e}"))
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

/// EMITTED = the union of opcode bytes over every program in the comprehensive corpus, each compiled by
/// the REAL encoder and walked with `op_width`. Returns opcode → the corpus program name that first
/// emitted it, so a RED can point at a concrete reproducer. A bench that hits the honest "not yet
/// lowered" guard is skipped (its op isn't on the chip yet — tracked elsewhere, not a decode hole).
fn emitted_opcodes() -> BTreeMap<u8, String> {
    let mut origin: BTreeMap<u8, String> = BTreeMap::new();
    let mut record = |name: &str, src: &str, allow_pending: bool| {
        let words = match compile_to_full_words(src) {
            Ok(w) => w,
            Err(e) if allow_pending && chip_lowering_pending(&e) => return,
            Err(e) => panic!(
                "EMITTED corpus program `{name}` failed to encode: {}\n  src={src:?}",
                e.lines().next().unwrap_or("")
            ),
        };
        let mut set = BTreeSet::new();
        opcodes_of_image(&words, &mut set);
        for oc in set {
            origin.entry(oc).or_insert_with(|| name.to_string());
        }
    };
    for (i, src) in CATALOG_CORPUS.iter().enumerate() {
        record(&format!("catalog[{i}]"), src, false);
    }
    for (i, src) in WORD32_CORPUS.iter().enumerate() {
        record(&format!("word32[{i}]"), src, false);
    }
    for (i, src) in TEXT_CORPUS.iter().enumerate() {
        record(&format!("text[{i}]"), src, false);
    }
    for (i, src) in FLOAT_HEAP_CORPUS.iter().enumerate() {
        record(&format!("float_heap[{i}]"), src, false);
    }
    for (i, src) in FLOAT_CMP_ITER_CORPUS.iter().enumerate() {
        record(&format!("float_cmp_iter[{i}]"), src, false);
    }
    for (i, src) in TEXT_SEQ_BUILTIN_CORPUS.iter().enumerate() {
        record(&format!("text_seq_builtin[{i}]"), src, false);
    }
    for (name, src) in bench_sources() {
        record(&format!("bench/{name}"), &src, true);
    }
    origin
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// DECODED — every opcode the real Verilog can execute: an explicit `case(opcode)` arm in `soft_core.v`,
// its op-38 builtin sub-dispatch, its reversible/CRDT/wide arms, PLUS the general `alu.v` datapath the
// `default:` arm routes any unhandled opcode to (Add/Sub/Mul/bitwise/compare). An opcode that has NO
// soft_core.v arm AND is not in `alu.v` falls through to `alu.v`'s `default: y = 32'd0` — that is a
// silent-zero hole (exactly 226/227/228 before the fix).
// ─────────────────────────────────────────────────────────────────────────────────────────────────

fn silicon_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon")
}

/// Parse `soft_core.v` for the opcode arms of its top-level `case (opcode)` dispatch ONLY. The file has
/// several `case(...)` blocks (`fpm_sel`/`b_op`/`i_op`/`ms_ph`/`ld_op` FSM sub-dispatches, plus the
/// op-38 `case(rb)` builtin dispatch and the op-114 `case(dst[7:4])` wide dispatch nested INSIDE the
/// opcode case). Only labels whose innermost enclosing case selector is exactly `opcode` are true decode
/// arms; the nested `rb`/`dst[7:4]` labels are builtin ordinals / wide sub-ops, NOT opcodes, and must be
/// excluded. We track case/endcase nesting and the selector of each open case to do this precisely.
fn soft_core_decoded() -> BTreeSet<u8> {
    let text = std::fs::read_to_string(silicon_dir().join("soft_core.v"))
        .expect("benchmarks/silicon/soft_core.v must exist");
    let mut decoded = BTreeSet::new();
    // A stack of the selector expressions of the currently-open `case(...)` blocks. `case (X)` and `endcase`
    // are detected ANYWHERE in the (comment-stripped) line — not merely at line-start — because a nested
    // dispatch can be written MID-LINE (`8'd38: begin … case (rb)`), whose line-start `endcase` would
    // otherwise pop `opcode` early and drop the real arms after it (the 200/221/222 class). We scan each line
    // left-to-right, applying every `case (…)` open and `endcase` close in source order, so the enclosing
    // selector is always accurate; `8'dN` labels count only while the innermost open case is exactly `opcode`.
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
        // Strip a trailing `//` comment so a commented `8'dN:` / `case(…)` never counts.
        let line = match raw.find("//") {
            Some(idx) => &raw[..idx],
            None => raw,
        };
        // Collect this line's decode-arm labels BEFORE mutating the case stack, using the CURRENT enclosing
        // selector (the label belongs to the case that is open as the line begins). A label list is the part
        // before the first `:`; a bare `8'dN,` continuation line (comma-first multi-line label) has no `:` but
        // its tokens still belong to the following arm — collect those too.
        if case_stack.last().map(String::as_str) == Some("opcode") {
            let trimmed = line.trim();
            if let Some(colon) = line.find(':') {
                collect_labels(&line[..colon], &mut decoded);
            } else if trimmed.ends_with(',') && trimmed.starts_with("8'd") {
                collect_labels(trimmed, &mut decoded);
            }
        }
        // Now apply every `case (…)` open and `endcase` close on this line, in source order, so a mid-line
        // nested `case (…)` (or a mid-line `endcase`) updates the enclosing selector for subsequent lines.
        let mut rest = line;
        loop {
            let next_case = rest.find("case");
            let next_end = rest.find("endcase");
            // `endcase` contains "case", so a bare `find("case")` inside `endcase` is the same index+3; treat
            // whichever token starts first, and make sure an `endcase` is not mis-read as a `case` open.
            match (next_case, next_end) {
                (Some(ci), Some(ei)) if ei <= ci => {
                    case_stack.pop();
                    rest = &rest[ei + "endcase".len()..];
                }
                (Some(ci), _) => {
                    // A real `case (SELECTOR)` open — require the `(` after `case` (else it is `endcase` or a
                    // word merely containing "case", which we skip).
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

/// Parse `alu.v` for the opcodes its combinational datapath computes. The `soft_core.v` `default:` arm
/// routes ANY opcode with no explicit arm through `alu.v` (`alu_y`), so an opcode `alu.v` handles is
/// decoded even without a soft_core.v arm — but an opcode neither file handles hits `alu.v`'s
/// `default: y = 32'd0` (the silent-zero hole).
fn alu_decoded() -> BTreeSet<u8> {
    let text = std::fs::read_to_string(silicon_dir().join("alu.v"))
        .expect("benchmarks/silicon/alu.v must exist");
    let mut decoded = BTreeSet::new();
    // A case label list can SPAN multiple lines when comma-separated: `8'd29,` on its own line then
    // `8'd30: y = …` on the next (the alu.v `Eq`/`ApproxEq` arm). A line with a trailing `,` and no `:` is a
    // label CONTINUATION — its `8'dN` tokens are decode labels for the arm whose `:` follows. A naive
    // `find(':')`-only parser drops op 29 (its line has no colon), a silent decode-coverage hole. Collect every
    // `8'dN` on a label-continuation line, and on an arm line the tokens BEFORE the `:` (never the RHS, which
    // in alu.v is arithmetic/`$signed`/`32'd…` and carries no `8'dN`). `default:` is the silent-zero, skipped.
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
                continue; // the silent-zero fallthrough, not a real decode
            }
            collect(label, &mut decoded);
        } else if trimmed.ends_with(',') && trimmed.starts_with("8'd") {
            // A bare `8'dN,` label-continuation line (the comma-first multi-line label form).
            collect(trimmed, &mut decoded);
        }
    }
    decoded
}

/// The full DECODED set the real RTL can execute: soft_core.v opcode arms ∪ the alu.v datapath.
fn decoded_opcodes() -> BTreeSet<u8> {
    let mut d = soft_core_decoded();
    d.extend(alu_decoded());
    d
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// EXERCISED — the union of opcodes across the iverilog testbench corpora the `verify-*.sh` scripts glob
// and run on the REAL `soft_core.v`:
//   • tb_gen_*.v          (SC_TB — the hand-assembled scalar/heap/control differential, verify-soft-core-rtl.sh)
//   • tb_chip_word32_*.v  (word32,      verify-word32-soft-core.sh)   ← reproduced from WORD32_CORPUS
//   • tb_chip_text_*.v    (text,        verify-text-soft-core.sh)     ← reproduced from TEXT_CORPUS
//   • tb_chip_tsb_*.v     (text/seq-builtin, verify-text-soft-core.sh)← reproduced from TEXT_SEQ_BUILTIN_CORPUS
//   • tb_chip_fpu_*.v     (float,       verify-fpu-soft-core.sh)      ← from the FPU bench .lg + float corpora
// The `tb_chip_*` images equal `compile_to_full_words(program)` for the same corpora that feed EMITTED,
// so we reconstruct them from those corpora. The SC_TB `tb_gen_*.v` images are hand-assembled u32 arrays
// (not reachable via `compile_to_full_words`), so we parse the checked-in `tb_gen_*.v` files' `imem[N] =
// 32'h…` lines directly — drift-free (they are regenerated by verify-soft-core-rtl.sh from the model).
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// Parse every `dut.imem[N] = 32'hXXXXXXXX;` line of the checked-in `tb_gen_*.v` (SC_TB) files and walk
/// each program image for its opcodes — the exact instructions verify-soft-core-rtl.sh runs on the RTL.
fn tb_gen_exercised() -> BTreeSet<u8> {
    let dir = silicon_dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("benchmarks/silicon must exist")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map_or(false, |n| n.starts_with("tb_gen_") && n.ends_with(".v"))
        })
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "expected checked-in tb_gen_*.v (the SC_TB corpus verify-soft-core-rtl.sh runs) in {dir:?}"
    );
    let mut set = BTreeSet::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        // Reassemble the image in address order so the multi-word op_width walk is correct.
        let mut image: BTreeMap<usize, u32> = BTreeMap::new();
        for line in text.lines() {
            let l = line.trim();
            // `dut.imem[12] = 32'h1B040102;`
            let Some(after) = l.strip_prefix("dut.imem[") else {
                continue;
            };
            let Some(rb) = after.find(']') else { continue };
            let Ok(addr) = after[..rb].trim().parse::<usize>() else {
                continue;
            };
            let Some(hpos) = l.find("32'h") else { continue };
            let hex: String = l[hpos + 4..]
                .chars()
                .take_while(|c| c.is_ascii_hexdigit())
                .collect();
            if let Ok(word) = u32::from_str_radix(&hex, 16) {
                image.insert(addr, word);
            }
        }
        if image.is_empty() {
            continue;
        }
        let max = *image.keys().max().unwrap();
        let mut words = vec![0u32; max + 1];
        for (a, w) in image {
            words[a] = w;
        }
        opcodes_of_image(&words, &mut set);
    }
    set
}

/// EXERCISED = SC_TB (parsed tb_gen_*.v) ∪ the `tb_chip_*` corpora (word32/text/text-seq-builtin) ∪ the
/// float corpora + FPU benches — every opcode a `verify-*.sh` run actually iverilogs on `soft_core.v`.
fn exercised_opcodes() -> BTreeSet<u8> {
    let mut set = tb_gen_exercised();
    let mut add = |src: &str, allow_pending: bool| {
        let words = match compile_to_full_words(src) {
            Ok(w) => w,
            Err(e) if allow_pending && chip_lowering_pending(&e) => return,
            Err(e) => panic!(
                "EXERCISED corpus program failed to encode: {}\n  src={src:?}",
                e.lines().next().unwrap_or("")
            ),
        };
        opcodes_of_image(&words, &mut set);
    };
    // The word32 / text / text-seq-builtin tb_chip_* corpora are 1:1 the compiled corpus images.
    for src in WORD32_CORPUS {
        add(src, false);
    }
    for src in TEXT_CORPUS {
        add(src, false);
    }
    for src in TEXT_SEQ_BUILTIN_CORPUS {
        add(src, false);
    }
    // The FPU tb corpus = the float-datapath bench .lg files + the float-heap / float-cmp-iter corpora.
    for src in FLOAT_HEAP_CORPUS {
        add(src, false);
    }
    for src in FLOAT_CMP_ITER_CORPUS {
        add(src, false);
    }
    for (_name, src) in bench_sources() {
        add(&src, true);
    }
    set
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// THE GUARDS
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// ★ GUARD A — DECODE COVERAGE (the killer guard) ★
/// Every opcode the encoder can EMIT from a real LOGOS program MUST have a decode arm on the real
/// silicon (`soft_core.v` opcode/op-38/reversible/wide arms, OR the `alu.v` datapath the `default:` arm
/// routes to). A hole here is a $50M silent-zero on tape-out: the op reads 0 and the bug ships. Fails
/// LOUD listing every emitted-but-undecoded opcode with its mnemonic and a reproducing corpus program.
#[test]
fn every_emitted_opcode_is_decoded_in_soft_core_v() {
    let emitted = emitted_opcodes();
    let decoded = decoded_opcodes();

    // Visibility for the reviewer / the report.
    println!("── EMITTED opcode surface (encoder → chip words) ──");
    for (oc, origin) in &emitted {
        let mark = if decoded.contains(oc) { "  " } else { "‼ " };
        println!(
            "  {mark}{oc:>3}  {:<18} (first seen in {origin})",
            mnemonic(*oc)
        );
    }
    println!(
        "  ({} distinct emitted, {} decoded in soft_core.v + alu.v)",
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
        "\n★ $50M DECODE HOLE ★ — {} opcode(s) the encoder EMITS from real LOGOS programs have NO decode \
         arm in benchmarks/silicon/soft_core.v (nor in alu.v). On tape-out silicon each reads 0 (falls \
         through soft_core.v's `default:` → alu.v's `default: y = 32'd0`). This is the 226/227/228 class: \
         model-proven `chip == VM` in Rust, yet SILENTLY UNDECODED in the real Verilog. Add the missing \
         `8'dN:` decode arm(s) to soft_core.v (or the alu.v datapath). NEVER weaken this guard.\n\
         Missing opcodes (opcode  mnemonic  first-seen-in):\n{}",
        holes.len(),
        holes
            .iter()
            .map(|(oc, mn, origin)| format!("    {oc:>3}  {mn:<18} {origin}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// ★ GUARD B — IVERILOG-EXERCISE COVERAGE ★
/// Every opcode the encoder can EMIT must be in at least one iverilog testbench corpus that a
/// `verify-*.sh` run actually executes on the real `soft_core.v` ≡ VM — so a decode arm is not merely
/// PRESENT but PROVEN to run correctly on the RTL. Fails LOUD listing every emitted-but-unexercised
/// opcode with its mnemonic and a reproducing corpus program: that is the real exercise-coverage
/// frontier (a model-proven op no `verify-*.sh` iverilogs).
#[test]
fn every_emitted_opcode_is_exercised_by_an_iverilog_testbench() {
    let emitted = emitted_opcodes();
    let exercised = exercised_opcodes();

    println!("── EXERCISED opcode surface (verify-*.sh iverilog corpora) ──");
    for oc in &exercised {
        println!("  {oc:>3}  {}", mnemonic(*oc));
    }
    println!(
        "  ({} distinct exercised by an iverilog tb)",
        exercised.len()
    );

    let unexercised: Vec<(u8, String, String)> = emitted
        .iter()
        .filter(|(oc, _)| !exercised.contains(oc))
        .map(|(oc, origin)| (*oc, mnemonic(*oc), origin.clone()))
        .collect();

    assert!(
        unexercised.is_empty(),
        "\n★ IVERILOG-EXERCISE GAP ★ — {} opcode(s) the encoder EMITS are in NO iverilog testbench corpus \
         that a `verify-*.sh` run executes on the real soft_core.v. Each is (or should be) model-proven, \
         but no RTL differential actually RUNS it on silicon ≡ VM — so a hardware bug in its arm would go \
         uncaught by every verify-*.sh. Add a program exercising it to the relevant emit_*_tb corpus in \
         codegen_sva/rtl.rs (word32 / text / float / text-seq-builtin / the SC_TB differential). NEVER \
         weaken this guard.\n\
         Unexercised opcodes (opcode  mnemonic  first-seen-in):\n{}",
        unexercised.len(),
        unexercised
            .iter()
            .map(|(oc, mn, origin)| format!("    {oc:>3}  {mn:<18} {origin}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// A sanity floor so the guards can never go vacuous by a corpus / parser regression that empties a set:
/// the EMITTED surface is broad by construction, the DECODED / EXERCISED sets are non-trivial, and the
/// 226/227/228 Word32-arithmetic ops (the original $50M hole) MUST be in EMITTED (so the guards keep
/// watching exactly the class that bit us).
#[test]
fn the_guarded_sets_are_non_vacuous_and_watch_the_word32_class() {
    let emitted = emitted_opcodes();
    let decoded = decoded_opcodes();
    let exercised = exercised_opcodes();
    assert!(
        emitted.len() >= 60,
        "EMITTED shrank to {} distinct opcodes — the comprehensive corpus regressed; the guards would be \
         vacuous",
        emitted.len()
    );
    assert!(
        decoded.len() >= 60,
        "DECODED shrank to {} — the soft_core.v / alu.v parser regressed",
        decoded.len()
    );
    assert!(
        exercised.len() >= 40,
        "EXERCISED shrank to {} — the tb corpus parser regressed",
        exercised.len()
    );
    for oc in [226u8, 227, 228] {
        assert!(
            emitted.contains_key(&oc),
            "the Word32 op {oc} ({}) — the original $50M hole class — is no longer in EMITTED; the guards \
             stopped watching it. Restore the word32 wrapping-arithmetic corpus program.",
            mnemonic(oc)
        );
    }
}
