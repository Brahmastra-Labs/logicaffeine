//! op_coverage_lock — the coverage ratchet for the spatial-interpreter correctness spine.
//!
//! `universal_completeness_lock` proves the corpus runs ≡ VM; this lock proves the corpus is BROAD — it
//! exercises the soft-core's opcode surface — so coverage cannot silently rot as ops are added. It
//! decodes the EXACT chip words `compile_to_full_words` emits (walking the only multi-word ops,
//! `LoadConstWide`=200 → 2 words and `FConst`=201 → 3 words, so an immediate is never mis-read as an
//! opcode), maps each opcode byte back to a mnemonic through the ISA ground truth
//! (`binisa::ISA_OPCODES`, whose index IS the chip opcode for real ops 0..115; the float/heap/wide
//! synthetic plane 200+ by name), and asserts a REQUIRED core surface stays covered. The full exercised
//! set is printed for visibility, and the not-yet-reachable set (structs/inductive/maps/channels) is the
//! honest complement locked by `universal_completeness_lock::encoder_rejects_unlowered_ops_loudly`.

use logicaffeine_compile::codegen_sva::full_asm::compile_to_full_words;
use logicaffeine_compile::vm::binisa::ISA_OPCODES;
use std::collections::BTreeSet;

/// One representative `.lg` program per op-category the full-core encoder supports — the same surface
/// `universal_completeness_lock` proves ≡ VM. (That lock owns the differential; this one owns coverage.)
const CORPUS: &[&str] = &[
    "## Main\n    Show 2 + 3 * 4 - 1.\n",
    "## Main\n    Show 100000 + 1.\n",                                     // wide const → LoadConstWide
    "## Main\n    Show 100 / 7.\n    Show 100 % 7.\n",                      // divider
    "## Main\n    Let x be 7.\n    If x is greater than 5:\n        Show 1.\n    Otherwise:\n        Show 0.\n",
    "## To sumdown (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Let rest be sumdown(n - 1).\n    Return n + rest.\n## Main\n    Show sumdown(6).\n",
    "## To addbase (n: Int) -> Int:\n    Return n + base.\n## Main\n    Let base be 1000.\n    Show addbase(5).\n",
    "## Main\n    Let mutable xs be [].\n    Let mutable i be 1.\n    While i is at most 3:\n        Push i * i to xs.\n        Set i to i + 1.\n    Set item 1 of xs to 42.\n    If xs contains 42:\n        Show item 1 of xs.\n    Pop from xs into y.\n    Show y.\n    Show the length of xs.\n",
    "## Main\n    Let a be [1, 2].\n    Let b be [3, 4].\n    Let c be a followed by b.\n    Let d be copy of c.\n    Show item 3 of d.\n",   // SeqConcat + DeepClone
    "## Main\n    Let mutable s be 0.\n    Repeat for x from 1 to 5:\n        Set s to s + x.\n    Show s.\n",           // range + iterator
    "## Main\n    Let b be 2.5.\n    Repeat for x from 1 to b:\n        Show x.\n",                                   // a bound not provably Int keeps the range value → NewRange
    "## Main\n    Let xs be [10, 20, 30].\n    Let mutable s be 0.\n    Repeat for x in xs:\n        Set s to s + x.\n    Show s.\n",
    "## Main\n    Let evens be a new Set of Int.\n    Add 2 to evens.\n    Show length of evens.\n",
    "## Main\n    Show 3.5 + 1.25.\n    Show 2.5 * 4.0.\n",                 // float FConst/FAdd/FMul/ShowF
    "## Main\n    Let s be \"ab\" + \"cd\".\n    Show s.\n",                 // Text Concat/ShowText
    "## Main\n    Show intOfWord32(word_and(word32(4042322160), word32(267399664))).\n",
    // residual deterministic ops (this file only ENCODES — the concurrency/net programs decline on the plain
    // VM but compile to chip words fine): the sequential power unit (18), tuple destructure in a loop (71),
    // the deterministic scheduler Sleep no-op (88), the non-blocking channels (95/96), the handle-bound
    // launch + task abort (99/101), the select arms + wait (102/103/104), and the offline networking (105/
    // 106/107/109/110/111). Each is proven ≡ its VM oracle in `universal_completeness_lock`.
    "## Main\n    Show 2 ** 10.\n",                                                                       // Pow
    "## Main\n    Let pairs be [(1, 10), (2, 20)].\n    Repeat for (a, b) in pairs:\n        Show a.\n",   // DestructureTuple
    "## Main\n    Sleep 100.\n    Show \"done\".\n",                                                       // Sleep
    "## Main\n    Let ch be a Pipe of Int.\n    Try to send 99 into ch.\n    Try to receive x from ch.\n    Show x.\n", // ChanTrySend/Recv
    "## To worker (ch: Int):\n    Send 7 into ch.\n## Main\n    Let jobs be a Pipe of Int.\n    Let h be Launch a task to worker with jobs.\n    Receive first from jobs.\n    Stop h.\n    Show first.\n", // SpawnHandle/TaskAbort
    "## To producer (ch: Int):\n    Send 5 into ch.\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to producer with jobs.\n    Await the first of:\n        Receive x from jobs:\n            Show x.\n        After 10 seconds:\n            Show 0.\n", // SelectArmRecv/Timeout/Wait
    "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send 42 to peer.\n    Await response from peer into got.\n    Show got.\n", // NetListen/MakePeer/Send/Await
    "## Main\n    Connect to \"ws://127.0.0.1:7777\".\n    Show \"connected\".\n",                          // NetConnect
    "## Definition\nA Tally has:\n    n: a Counter.\n## Main\n    Let mutable state be a new Tally.\n    Increase state's n by 3.\n    Sync state on \"topic\".\n    Show state's n.\n", // NetSync
];

/// The complete multi-word chip-op table (every op that embeds an immediate in a following word, so the
/// linear opcode walk must skip it): `Call` (37) = 2 words ([op][result][args_start][arg_count] ‖
/// [entry:32]), `LoadConstWide` (200) = 2, `FConst` (201) = 3 ([op] ‖ bits[31:0] ‖ bits[63:32]),
/// `TextConst` (216) = 2 ([op][dst][len:16] ‖ pool_word_offset), `LoadConstWide64` (222) = 3 ([op] ‖
/// bits[31:0] ‖ bits[63:32] — the 64-bit-datapath wide constant). Every other opcode is a single word
/// (float `IntToF` conversions and `FormatValue`/`Concat` are each their own 1-word instruction). Must
/// stay in lock-step with `full_asm::imem_width` and the model's PC advancement.
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
/// synthetic float/heap/wide plane (200+) is named explicitly (these deliberately live ABOVE the ISA
/// table so they never drift an `Op` variant).
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
        _ => "unknown-synthetic",
    }
    .to_string()
}

/// The set of opcode bytes the corpus's emitted chip words actually execute (correctly PC-walked).
fn exercised_opcodes() -> BTreeSet<u8> {
    let mut set = BTreeSet::new();
    for src in CORPUS {
        let words = compile_to_full_words(src).unwrap_or_else(|e| {
            panic!("op_coverage corpus program failed to encode: {e}\n  src={src:?}")
        });
        let mut pc = 0usize;
        while pc < words.len() {
            let oc = (words[pc] >> 24) as u8;
            set.insert(oc);
            pc += op_width(oc);
        }
    }
    set
}

#[test]
fn corpus_exercises_core_opcode_surface() {
    let ex = exercised_opcodes();
    let names: BTreeSet<String> = ex.iter().map(|&oc| mnemonic(oc)).collect();

    // Visibility: the full exercised surface (opcode → mnemonic), so a reviewer sees exactly what the
    // universal corpus drives through the chip.
    println!("── soft-core opcode surface exercised by the universal corpus ──");
    for &oc in &ex {
        println!("  {oc:>3}  {}", mnemonic(oc));
    }
    println!("  ({} distinct opcodes)", ex.len());

    // The REQUIRED core surface: every op the universal interpreter must keep exercising. A dropped
    // corpus program or a de-lowered op makes this RED — coverage can never silently shrink (rule 14).
    let required = [
        // arithmetic / comparison / control
        "Add",
        "Sub",
        "Mul",
        "Gt",
        "Jump",
        "JumpIfFalse",
        // constants (inline + wide)
        "LoadConst",
        "LoadConstWide",
        // functions
        "Call",
        "Return",
        // division through the shared divider
        "Div",
        "Mod",
        // heap collections (the optimizer emits the bounds-proven `*Unchecked` index writers)
        "NewList",
        "ListPush",
        "Index",
        "SetIndexUnchecked",
        "Length",
        "Contains",
        "ListPop",
        "NewRange",
        "IterPrepare",
        "IterNext",
        // sets
        "NewEmptySet",
        "SetAdd",
        // freshly-lowered list ops (P0b-gap)
        "SeqConcat",
        "DeepClone",
        // globals + output
        "GlobalGet",
        "GlobalSet",
        "Show",
        // float datapath
        "FConst",
        "FAdd",
        "FMul",
        "ShowF",
        // Text datapath (TextConst literal + Concat + ShowText)
        "TextConst",
        "Concat",
        "ShowText",
        // Word32 datapath (bitwise through the word plane)
        "BitAnd",
        // residual deterministic ops (the model + RTL decode; each proven ≡ its VM oracle)
        "Pow",
        "DestructureTuple",
        "Sleep",
        "ChanTrySend",
        "ChanTryRecv",
        "SpawnHandle",
        "TaskAbort",
        "SelectArmRecv",
        "SelectArmTimeout",
        "SelectWait",
        "NetConnect",
        "NetListen",
        "NetSend",
        "NetAwait",
        "NetMakePeer",
        "NetSync",
    ];
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|r| !names.contains(*r))
        .collect();
    assert!(
        missing.is_empty(),
        "the universal corpus no longer exercises required soft-core opcodes {missing:?} — coverage regressed; \
         restore the corpus program that drives them (exercised = {names:?})"
    );

    // A comprehensive base is broad by construction: guard the raw count too, so quietly deleting several
    // categories at once still trips even if each individual op above were somehow re-routed.
    assert!(
        ex.len() >= 30,
        "the universal corpus exercises only {} distinct opcodes — expected the full core surface (>=30)",
        ex.len()
    );
}
