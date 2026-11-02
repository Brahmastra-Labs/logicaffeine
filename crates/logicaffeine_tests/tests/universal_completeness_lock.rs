//! universal_completeness_lock — the correctness spine of the spatial-interpreter campaign.
//!
//! "Send the bytecode down the wire and the chip runs it." This lock proves that claim in sim, at the
//! language level: for a broad corpus spanning every op-category the full-core encoder supports, the
//! EXACT chip words `compile_to_full_words` emits — the words `scripts/run-on-nexys-full.sh` streams to
//! the FPGA over UART — execute on the cycle-accurate soft-core oracle (`run_soft_core_typed`, the model
//! of `benchmarks/silicon/soft_core.v`) IDENTICALLY to the VM. Green here == the universal interpreter
//! in silicon runs the language bit-exact; it is the safety net every later fabric phase falls back on.
//!
//! Methodology (the campaign's three-layer proof, reproducible / no simulator): this file is layer 1 —
//! `compile_to_full_words` → `run_soft_core_typed` == `vm_outcome`. Layer 2 (iverilog RTL ≡ this model)
//! is `scripts/verify-soft-core-rtl.sh`; layer 3 (Sky130 signoff) is the demo scripts. The soft-core
//! model is the single shared oracle (`codegen_sva::soft_core_model`), so this lock and the in-crate RTL
//! tests check the same executable spec.
//!
//! The corpus is DIFFERENTIAL (asserts soft-core == VM, never a hand-coded golden) and FUZZED over
//! operand sweeps where a datapath bug would hide, so a silent chip/VM divergence goes RED. Categories
//! not yet lowered by the encoder (structs, inductive+match, `followed by`, `copy of`, maps, float-seq
//! literals, concurrency) are closed incrementally in follow-on RED→GREEN increments; the encoder must
//! REJECT any op it cannot lower soundly (proven by `encoder_rejects_unlowered_ops_loudly`), so a missing
//! category can never become a SILENT miscompile.

use logicaffeine_compile::codegen_sva::full_asm::compile_to_full_words;
use logicaffeine_compile::codegen_sva::soft_core_model::{run_soft_core_typed, ShowVal};
use logicaffeine_compile::compile::{vm_outcome, vm_outcome_concurrent, vm_outcome_net};

/// Normalize a VM `output` for comparison against the chip's `Show`-joined `got`: strip ONLY the single
/// trailing Show-terminator newline (`output == got + "\n"`), never `.trim()` — trimming would also eat a
/// value that legitimately IS whitespace (`chr(160)` = U+00A0 nbsp, `chr(9)` = tab, `chr(32)` = space) on the
/// VM side only, forging a divergence out of an exact agreement (the class `soft_core_fuzz_differential` caught).
fn vm_shows(output: &str) -> &str {
    output.strip_suffix('\n').unwrap_or(output)
}

/// The differential core: the chip words the encoder emits, run on the soft-core oracle, must print
/// EXACTLY what the VM prints. Panics with context on encode failure, model panic, or divergence.
#[track_caller]
fn assert_chip_eq_vm(name: &str, src: &str) {
    let words = compile_to_full_words(src)
        .unwrap_or_else(|e| panic!("{name}: full-core encode failed: {e}"));
    let shows = run_soft_core_typed(&words, 50_000_000);
    let got = shows
        .iter()
        .map(ShowVal::fmt)
        .collect::<Vec<_>>()
        .join("\n");
    let vm = vm_outcome(src).output;
    assert_eq!(
        got.as_str(),
        vm_shows(&vm),
        "{name}: soft-core chip run != VM\n  src = {src:?}"
    );
}

/// The differential core for a DETERMINISTIC-CONCURRENCY program (`Pipe`/`Launch`/`Receive`). The plain
/// [`vm_outcome`] DECLINES a scheduler op (`requires the scheduler driver`), so it is not the oracle here;
/// the deterministic-offline semantics the chip runs (synchronous `Launch`, FIFO channels — the same
/// convention the WASM AOT backend is proven against) match [`vm_outcome_concurrent`] (the cooperative VM
/// scheduler, seed 0), which IS the oracle the AOT concurrency lock compares against. So the chip must
/// print EXACTLY what the deterministic concurrency VM prints.
#[track_caller]
fn assert_chip_eq_vm_concurrent(name: &str, src: &str) {
    let words = compile_to_full_words(src)
        .unwrap_or_else(|e| panic!("{name}: full-core encode failed: {e}"));
    let shows = run_soft_core_typed(&words, 50_000_000);
    let got = shows
        .iter()
        .map(ShowVal::fmt)
        .collect::<Vec<_>>()
        .join("\n");
    let vm = vm_outcome_concurrent(src).output;
    assert_eq!(
        got.as_str(),
        vm_shows(&vm),
        "{name}: soft-core chip run != deterministic-concurrency VM\n  src = {src:?}"
    );
}

/// The differential core for a peer-NETWORKING program (`Connect`/`Listen`/`Send`/`Await`/`Sync`/
/// `PeerAgent`). Neither [`vm_outcome`] (which errors — a bare `Connect` "requires the scheduler
/// driver") nor [`vm_outcome_concurrent`] (which the VM's cooperative scheduler leaves BLOCKED on a
/// `Net*` block, printing nothing) is the oracle here. The deterministic OFFLINE single-node semantics
/// the chip runs — `Connect`/`Listen`/`Stream`/`Sync` are no-ops, `NetSend`/`NetAwait` a loopback
/// round-trip through the shared `NetInbox` — match [`vm_outcome_net`] (offline mode, `tw == VM-net ==
/// AOT`), the SAME oracle the AOT networking lock compares against. So the chip must print EXACTLY what
/// the offline-networking VM prints.
#[track_caller]
fn assert_chip_eq_vm_net(name: &str, src: &str) {
    let words = compile_to_full_words(src)
        .unwrap_or_else(|e| panic!("{name}: full-core encode failed: {e}"));
    let shows = run_soft_core_typed(&words, 50_000_000);
    let got = shows
        .iter()
        .map(ShowVal::fmt)
        .collect::<Vec<_>>()
        .join("\n");
    let vm = vm_outcome_net(src).output;
    assert_eq!(
        got.as_str(),
        vm_shows(&vm),
        "{name}: soft-core chip run != offline-networking VM\n  src = {src:?}"
    );
}

/// Assert the emitted chip image actually contains an instruction with `opcode` in its high byte — proof
/// the differential above EXERCISES the op under test (a compiler change that stopped emitting it would
/// otherwise make the differential vacuously pass). The op BYTE is `binisa::opcode(op)` (== the ISA index).
#[track_caller]
fn assert_uses_opcode(name: &str, src: &str, opcode: u8) {
    let words = compile_to_full_words(src)
        .unwrap_or_else(|e| panic!("{name}: full-core encode failed: {e}"));
    assert!(
        words.iter().any(|w| (w >> 24) as u8 == opcode),
        "{name}: expected opcode {opcode} in the emitted chip image but it is absent — the differential \
         would test nothing\n  src = {src:?}"
    );
}

/// A tiny splitmix64 so the fuzzes are deterministic (no `rand` dep, no `Math.random` flake).
fn mix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

// ── arithmetic, comparison, control flow ─────────────────────────────────────────────────────────────

#[test]
fn arithmetic_and_control_flow() {
    assert_chip_eq_vm("expr", "## Main\n    Show 2 + 3 * 4 - 1.\n");
    assert_chip_eq_vm("paren", "## Main\n    Show (10 - 3) * 2.\n");
    assert_chip_eq_vm("neg", "## Main\n    Show 0 - 5.\n");
    assert_chip_eq_vm("wide", "## Main\n    Show 100000 + 1.\n");
    assert_chip_eq_vm("big-mul", "## Main\n    Show 40000 * 50.\n");
    assert_chip_eq_vm(
        "if-else",
        "## Main\n    Let x be 7.\n    If x is greater than 5:\n        Show 1.\n    Otherwise:\n        Show 0.\n",
    );
    assert_chip_eq_vm(
        "while-sum",
        "## Main\n    Let mutable sum be 0.\n    Let mutable i be 1.\n    While i is at most 100:\n        Set sum to sum + i.\n        Set i to i + 1.\n    Show sum.\n",
    );

    // FUZZ: FULL-RANGE i64 operands over +, -, *, and compare — NO range dodge. The full-core value datapath
    // auto-promotes to BigInt on overflow exactly like the VM's unbounded `Int`, so the chip must differ
    // NOWHERE even when a sum/difference overflows i64 or a product blows out to a 38-digit BigInt. A
    // datapath / sign / promotion bug diverges. Compared by EXACT rendered decimal (no `mod`, no clamping).
    let mut st = 0x1234_5678_9ABC_DEF0u64;
    for _ in 0..200 {
        let a = mix(&mut st) as i64; // full i64 range, both signs
        let b = mix(&mut st) as i64;
        for op in ["+", "-", "*"] {
            assert_chip_eq_vm(
                "fuzz-arith",
                &format!("## Main\n    Let a be {a}.\n    Let b be {b}.\n    Show a {op} b.\n"),
            );
        }
        assert_chip_eq_vm(
            "fuzz-cmp",
            &format!("## Main\n    Let a be {a}.\n    Let b be {b}.\n    If a is greater than b:\n        Show 1.\n    Otherwise:\n        Show 0.\n"),
        );
    }
}

#[test]
fn division_and_modulo() {
    assert_chip_eq_vm("div", "## Main\n    Show 100 / 7.\n");
    assert_chip_eq_vm("mod", "## Main\n    Show 100 % 7.\n");

    // FUZZ div/mod incl. negative dividends (truncated-toward-zero, sign-of-dividend remainder) — the
    // shared-divider path is where a signed-division bug lives. Nonzero divisor (div-by-zero is a Phase-1
    // fabric-trap concern, not this lock).
    let mut st = 0xDEAD_BEEF_0BAD_F00Du64;
    for _ in 0..200 {
        let a = mix(&mut st) as i64; // FULL i64 dividend, both signs (incl. the i64::MIN / -1 overflow edge)
        let b = ((mix(&mut st) % 999) as i64 + 1) * if mix(&mut st) & 1 == 0 { 1 } else { -1 }; // ±[1,999]
        assert_chip_eq_vm(
            "fuzz-div",
            &format!("## Main\n    Let a be {a}.\n    Let b be {b}.\n    Show a / b.\n"),
        );
        assert_chip_eq_vm(
            "fuzz-mod",
            &format!("## Main\n    Let a be {a}.\n    Let b be {b}.\n    Show a % b.\n"),
        );
    }
}

// ── user functions: call, recursion, globals ─────────────────────────────────────────────────────────

#[test]
fn functions_recursion_globals() {
    assert_chip_eq_vm(
        "call",
        "## To addbase (n: Int) -> Int:\n    Return n + 100.\n## Main\n    Show addbase(5).\n",
    );
    assert_chip_eq_vm(
        "two-args",
        "## To sub2 (a: Int, b: Int) -> Int:\n    Return a - b.\n## Main\n    Show sub2(50, 8).\n",
    );
    assert_chip_eq_vm("global", "## To addbase (n: Int) -> Int:\n    Return n + base.\n## Main\n    Let base be 1000.\n    Show addbase(5).\n");

    let sumdown = "## To sumdown (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Let rest be sumdown(n - 1).\n    Return n + rest.\n## Main\n    Show sumdown({N}).\n";
    let fib = "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n## Main\n    Show fib({N}).\n";
    // FUZZ recursion depth/shape — the call/return frame-stack discipline must match the VM at every input.
    let mut st = 0x0F0F_0F0F_1111_2222u64;
    for _ in 0..30 {
        let n = mix(&mut st) % 20; // fib(19) is the heaviest, still fast on the model
        assert_chip_eq_vm("fuzz-sumdown", &sumdown.replace("{N}", &n.to_string()));
        assert_chip_eq_vm("fuzz-fib", &fib.replace("{N}", &n.to_string()));
    }
}

// ── collections: lists (index/push/pop/set/contains), build-in-loop, value semantics ─────────────────

#[test]
fn collections_lists_and_value_semantics() {
    assert_chip_eq_vm(
        "index",
        "## Main\n    Let xs be [10, 20, 30].\n    Show item 2 of xs.\n",
    );
    assert_chip_eq_vm(
        "push-len",
        "## Main\n    Let xs be [1, 2].\n    Push 3 to xs.\n    Show the length of xs.\n",
    );
    assert_chip_eq_vm(
        "pop",
        "## Main\n    Let xs be [10, 20, 30].\n    Pop from xs into y.\n    Show y.\n",
    );
    assert_chip_eq_vm("setidx", "## Main\n    Let xs be [10, 20, 30].\n    Set item 2 of xs to 99.\n    Show item 2 of xs.\n");
    assert_chip_eq_vm("contains", "## Main\n    Let xs be [10, 20, 30].\n    If xs contains 20:\n        Show 111.\n    Otherwise:\n        Show 222.\n");
    assert_chip_eq_vm("contains-no", "## Main\n    Let xs be [10, 20, 30].\n    If xs contains 99:\n        Show 111.\n    Otherwise:\n        Show 222.\n");
    // value semantics (Rc COW): a mutated copy must not disturb the aliased base binding.
    assert_chip_eq_vm(
        "cow-preserve",
        "## To addone (xs: Seq of Int) -> Int:\n    Let mutable ys be xs.\n    Push 99 to ys.\n    Return the length of xs.\n## Main\n    Let xs be [1, 2].\n    Show addone(xs).\n",
    );
    // `a followed by b` — fresh concatenation (SeqConcat, opcode 33).
    assert_chip_eq_vm(
        "concat",
        "## Main\n    Let a be [1, 2].\n    Let b be [3, 4].\n    Let c be a followed by b.\n    Show item 3 of c.\n",
    );
    // `copy of a` — an independent deep copy; mutating the original must not touch the copy (DeepClone, 66).
    assert_chip_eq_vm(
        "clone",
        "## Main\n    Let a be [10, 20].\n    Let b be copy of a.\n    Set item 1 of a to 99.\n    Show item 1 of b.\n",
    );

    // FUZZ a build-in-loop then index — the heap layout (header + cells) and 1-based index must match VM.
    let mut st = 0xABCD_1234_5678_9999u64;
    for _ in 0..60 {
        let n = (mix(&mut st) % 8 + 1) as usize; // 1..=8 elements
        let k = mix(&mut st) % n as u64 + 1; // 1-based index in range
        let src = format!(
            "## Main\n    Let mutable xs be [].\n    Let mutable i be 1.\n    While i is at most {n}:\n        Push i * i to xs.\n        Set i to i + 1.\n    Show item {k} of xs.\n"
        );
        assert_chip_eq_vm("fuzz-list-build", &src);
    }
}

// ── tuples, sets, ranges, iterators ──────────────────────────────────────────────────────────────────

#[test]
fn tuples_sets_ranges_iterators() {
    assert_chip_eq_vm(
        "tuple",
        "## Main\n    Let t be (10, 20, 30).\n    Show item 2 of t.\n",
    );
    assert_chip_eq_vm("set", "## Main\n    Let evens be a new Set of Int.\n    Add 2 to evens.\n    Add 4 to evens.\n    Add 2 to evens.\n    Show length of evens.\n");
    assert_chip_eq_vm("range", "## Main\n    Let mutable s be 0.\n    Repeat for x from 1 to 5:\n        Set s to s + x.\n    Show s.\n");
    assert_chip_eq_vm("foreach", "## Main\n    Let xs be [10, 20, 30].\n    Let mutable s be 0.\n    Repeat for x in xs:\n        Set s to s + x.\n    Show s.\n");

    // FUZZ a range-sum and a foreach-sum — the iterator cursor/exit must match the VM across bounds.
    let mut st = 0x5555_AAAA_3333_CCCCu64;
    for _ in 0..40 {
        let hi = mix(&mut st) % 30 + 1;
        assert_chip_eq_vm(
            "fuzz-range",
            &format!("## Main\n    Let mutable s be 0.\n    Repeat for x from 1 to {hi}:\n        Set s to s + x.\n    Show s.\n"),
        );
    }
}

// ── typed datapaths: floats, Text, Word32 ────────────────────────────────────────────────────────────

#[test]
fn floats_text_word32() {
    assert_chip_eq_vm("float-add", "## Main\n    Show 3.5 + 1.25.\n");
    assert_chip_eq_vm("float-mul", "## Main\n    Show 2.5 * 4.0.\n");
    assert_chip_eq_vm(
        "text-concat",
        "## Main\n    Let s be \"ab\" + \"cd\".\n    Show s.\n",
    );
    assert_chip_eq_vm("text-interp", "## Main\n    Show \"depth \" + 3.\n");
    assert_chip_eq_vm(
        "word32-unsigned",
        "## Main\n    Show intOfWord32(word32(4294967295)).\n",
    );
    assert_chip_eq_vm("word32-and-xor", "## Main\n    Let a be word32(4042322160).\n    Let b be word32(267399664).\n    Show intOfWord32(word_and(a, b)).\n");
    assert_chip_eq_vm(
        "word32-rotr",
        "## Main\n    Show intOfWord32(rotr(word32(1), 1)).\n",
    );

    // Float comparisons — `<`, `>`, `<=`, `>=`, `==`, `!=` (int 0/1 via FLt/FEq/FLe/FNe). Shown through an
    // if/else so the branch — the datapath's 0/1 result — is what's asserted equal to the VM.
    for (name, cmp, rhs) in [
        ("float-lt", "is less than", "2.5"),
        ("float-gt", "is greater than", "1.5"),
        ("float-le-eq", "is at most", "2.5"), // boundary: equal operands
        ("float-ge-lt", "is at least", "3.5"), // false side
        ("float-eq", "is equal to", "2.5"),
        ("float-ne", "is not", "3.5"),
    ] {
        assert_chip_eq_vm(
            name,
            &format!("## Main\n    If 2.5 {cmp} {rhs}:\n        Show 1.\n    Otherwise:\n        Show 0.\n"),
        );
    }

    // FUZZ float add/sub/mul — the FPU path must be bit-exact vs the VM's f64 through fmt_f64.
    let mut st = 0x2468_ACE0_1357_9BDFu64;
    for _ in 0..60 {
        let a = (mix(&mut st) % 2000) as f64 / 8.0 - 125.0;
        let b = (mix(&mut st) % 2000) as f64 / 8.0 - 125.0;
        for op in ["+", "-", "*"] {
            assert_chip_eq_vm(
                "fuzz-float",
                &format!("## Main\n    Show {a:.3} {op} {b:.3}.\n"),
            );
        }
    }

    // FUZZ float compares over the SIX operators — b is sometimes exactly a (the `<=`/`>=`/`==`/`!=` equality
    // boundary). A compare-datapath or NaN-handling bug diverges from the VM's f64 comparison semantics.
    for _ in 0..60 {
        let a = (mix(&mut st) % 400) as f64 / 8.0 - 25.0;
        let b = if mix(&mut st) & 3 == 0 {
            a
        } else {
            (mix(&mut st) % 400) as f64 / 8.0 - 25.0
        };
        for cmp in [
            "is less than",
            "is greater than",
            "is at most",
            "is at least",
            "is equal to",
            "is not",
        ] {
            assert_chip_eq_vm(
                "fuzz-float-cmp",
                &format!("## Main\n    Let x be {a:.3}.\n    Let y be {b:.3}.\n    If x {cmp} y:\n        Show 1.\n    Otherwise:\n        Show 0.\n"),
            );
        }
    }
}

// ── multi-feature integer algorithms (the benchmark subset) ──────────────────────────────────────────

#[test]
fn multi_feature_algorithms() {
    assert_chip_eq_vm(
        "gcd",
        "## To gcd (a: Int, b: Int) -> Int:\n    Let mutable x be a.\n    Let mutable y be b.\n    While y is greater than 0:\n        Let t be y.\n        Set y to x % y.\n        Set x to t.\n    Return x.\n## Main\n    Show gcd(1071, 462).\n",
    );
    assert_chip_eq_vm(
        "primes",
        "## Main\n    Let mutable count be 0.\n    Let mutable n be 2.\n    While n is at most 50:\n        Let mutable d be 2.\n        Let mutable prime be 1.\n        While d is less than n:\n            If n % d is equal to 0:\n                Set prime to 0.\n            Set d to d + 1.\n        Set count to count + prime.\n        Set n to n + 1.\n    Show count.\n",
    );
    assert_chip_eq_vm(
        "nqueens",
        "## To isSafe (queens: Seq of Int, row: Int, col: Int) -> Int:\n    Let mutable i be 1.\n    While i is less than row:\n        Let qc be item i of queens.\n        If qc equals col:\n            Return 0.\n        If qc - col equals i - row:\n            Return 0.\n        If col - qc equals i - row:\n            Return 0.\n        Set i to i + 1.\n    Return 1.\n## To solve (queens: Seq of Int, row: Int, n: Int) -> Int:\n    If row is greater than n:\n        Return 1.\n    Let mutable count be 0.\n    Let mutable col be 1.\n    While col is at most n:\n        If isSafe(queens, row, col) equals 1:\n            Let mutable next be queens.\n            Push col to next.\n            Set count to count + solve(next, row + 1, n).\n        Set col to col + 1.\n    Return count.\n## Main\n    Let queens be a new Seq of Int.\n    Show solve(queens, 1, 6).\n",
    );
}

// ── inductives (enums) + Inspect/match ───────────────────────────────────────────────────────────────

#[test]
fn inductives_and_match() {
    // `a new Circle …` builds a tagged record; `Inspect` matches by constructor + binds the payload
    // positionally. The variant tag is the constructor-name const index (NewInductive/TestArm agree by
    // dedup — the VM's name equality), so the matched arm's `Show r` == the VM's 42.
    assert_chip_eq_vm(
        "inductive-circle",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n## Main\nLet s be a new Circle with radius 42.\nInspect s:\n    When Circle (r): Show r.\n    When Rectangle (w, h): Show w.\n",
    );
    // The OTHER arm: a Rectangle matches the second arm and binds two payloads (positions 0 and 1). Shows
    // width=7 (the `w` binding), so a wrong tag or a wrong payload index would diverge from the VM.
    assert_chip_eq_vm(
        "inductive-rect",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n## Main\nLet s be a new Rectangle with width 7 and height 9.\nInspect s:\n    When Circle (r): Show r.\n    When Rectangle (w, h): Show h.\n",
    );

    // FUZZ over BOTH the chosen constructor and the payload values — the tag compare + positional bind must
    // match the VM for every branch. A datapath bug in the tag test or the payload extraction diverges.
    let mut st = 0x51A5_E00D_1234_ABCDu64;
    for _ in 0..80 {
        let is_circle = mix(&mut st) & 1 == 0;
        let a = (mix(&mut st) % 100_000) as i64;
        let b = (mix(&mut st) % 100_000) as i64;
        let src = if is_circle {
            format!("## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n## Main\nLet s be a new Circle with radius {a}.\nInspect s:\n    When Circle (r): Show r.\n    When Rectangle (w, h): Show w.\n")
        } else {
            format!("## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n## Main\nLet s be a new Rectangle with width {a} and height {b}.\nInspect s:\n    When Circle (r): Show r.\n    When Rectangle (w, h): Show h.\n")
        };
        assert_chip_eq_vm("fuzz-inductive", &src);
    }
}

// ── structs (records): construct, read, nested/multi-field, value semantics ────────────────────────────

#[test]
fn structs_construct_and_read() {
    // `a new Point with x 10 and y 20` → a positional record; `p's y` reads the y slot. The usage-derived
    // layout maps (Point, x)→slot0, (Point, y)→slot1, so `Show p's y` == the VM's 20.
    assert_chip_eq_vm(
        "struct-point-y",
        "## Definition\nA Point has:\n    x: Int.\n    y: Int.\n## Main\n    Let p be a new Point with x 10 and y 20.\n    Show p's y.\n",
    );
    assert_chip_eq_vm(
        "struct-point-x",
        "## Definition\nA Point has:\n    x: Int.\n    y: Int.\n## Main\n    Let p be a new Point with x 10 and y 20.\n    Show p's x.\n",
    );
    // A THREE-field struct read of the middle field — a wrong slot (off-by-one) diverges from the VM.
    assert_chip_eq_vm(
        "struct-three",
        "## Definition\nA Vec3 has:\n    a: Int.\n    b: Int.\n    c: Int.\n## Main\n    Let v be a new Vec3 with a 1 and b 2 and c 3.\n    Show v's b.\n",
    );
    // VALUE SEMANTICS: a struct is value-typed. `Let q be p` then reassigning q's field via a fresh
    // construction must not disturb p (the isolation barrier keeps p's slots intact).
    assert_chip_eq_vm(
        "struct-value-sem",
        "## Definition\nA Point has:\n    x: Int.\n    y: Int.\n## Main\n    Let p be a new Point with x 10 and y 20.\n    Let q be p.\n    Show p's x.\n    Show q's y.\n",
    );

    // FUZZ the two field values, reading each field back — the positional layout must be stable and match
    // the VM for arbitrary payloads.
    let mut st = 0x0DEC_0DED_5555_1111u64;
    for _ in 0..80 {
        let x = (mix(&mut st) % 1_000_000) as i64;
        let y = (mix(&mut st) % 1_000_000) as i64;
        assert_chip_eq_vm(
            "fuzz-struct-x",
            &format!("## Definition\nA Point has:\n    x: Int.\n    y: Int.\n## Main\n    Let p be a new Point with x {x} and y {y}.\n    Show p's x.\n"),
        );
        assert_chip_eq_vm(
            "fuzz-struct-y",
            &format!("## Definition\nA Point has:\n    x: Int.\n    y: Int.\n## Main\n    Let p be a new Point with x {x} and y {y}.\n    Show p's y.\n"),
        );
    }
}

// ── CRDTs, single-replica projection: G-counter bump + merge ───────────────────────────────────────────

#[test]
fn crdts_single_replica() {
    // A `Counter` field bumped by `Increase … by k`: the field default-fills to Nothing (the additive
    // identity 0 on the chip), then CrdtBump accumulates → 5, matching the VM.
    assert_chip_eq_vm(
        "crdt-bump",
        "## Definition\nA Tally has:\n    n: a Counter.\n## Main\n    Let mutable t be a new Tally.\n    Increase t's n by 5.\n    Show t's n.\n",
    );
    // Increase then Decrease (the negate bit): 5 − 2 = 3.
    assert_chip_eq_vm(
        "crdt-bump-negate",
        "## Definition\nA Tally has:\n    n: a Counter.\n## Main\n    Let mutable t be a new Tally.\n    Increase t's n by 5.\n    Decrease t's n by 2.\n    Show t's n.\n",
    );
    // MERGE two counters field-wise (sum-join): x.a = 5, y.b = 3; Merge y into x → x keeps a=5, gains b=3.
    assert_chip_eq_vm(
        "crdt-merge",
        "## A Counter has:\n    a: Int.\n    b: Int.\n## Main\n    Let mutable x be a new Counter with a 5.\n    Let mutable y be a new Counter with b 3.\n    Merge y into x.\n    Show x's a.\n    Show x's b.\n",
    );

    // FUZZ a bump sequence — the accumulator (+= per bump, wrapping in the counter's cyclic group) must
    // track the VM's `crdt_counter_bump` exactly.
    let mut st = 0xC0DE_C0FF_EE00_1234u64;
    for _ in 0..60 {
        let a = (mix(&mut st) % 10_000) as i64;
        let b = (mix(&mut st) % 10_000) as i64;
        assert_chip_eq_vm(
            "fuzz-crdt",
            &format!("## Definition\nA Tally has:\n    n: a Counter.\n## Main\n    Let mutable t be a new Tally.\n    Increase t's n by {a}.\n    Increase t's n by {b}.\n    Show t's n.\n"),
        );
    }
}

// ── float-seq literal: build through the float payload plane ───────────────────────────────────────────

#[test]
fn float_seq_literal() {
    // `[1.5, 2.5, 3.0]` builds through the float plane (NewEmptyList + FListPush per element), so `item i`
    // reads the f64 payload — NOT the all-zero plane a plain NewList would leave. Indexed → 2.5 == VM.
    assert_chip_eq_vm(
        "float-seq-index",
        "## Main\n    Let xs be [1.5, 2.5, 3.0].\n    Show item 2 of xs.\n",
    );
    assert_chip_eq_vm(
        "float-seq-index1",
        "## Main\n    Let xs be [1.5, 2.5, 3.0].\n    Show item 1 of xs.\n",
    );
    // Sum the float-seq literal in a loop — every element must ride the plane (a dropped payload diverges).
    assert_chip_eq_vm(
        "float-seq-sum",
        "## Main\n    Let xs be [1.5, 2.5, 3.0].\n    Let mutable s be 0.0.\n    Repeat for v in xs:\n        Set s to s + v.\n    Show s.\n",
    );

    // FUZZ the literal's three values, indexing each — the payload plane must match the VM's f64 element.
    let mut st = 0xFEED_FACE_2468_1357u64;
    for _ in 0..40 {
        let a = (mix(&mut st) % 2000) as f64 / 8.0 - 125.0;
        let b = (mix(&mut st) % 2000) as f64 / 8.0 - 125.0;
        let c = (mix(&mut st) % 2000) as f64 / 8.0 - 125.0;
        let k = mix(&mut st) % 3 + 1;
        assert_chip_eq_vm(
            "fuzz-float-seq",
            &format!(
                "## Main\n    Let xs be [{a:.3}, {b:.3}, {c:.3}].\n    Show item {k} of xs.\n"
            ),
        );
    }
}

// ── non-capturing closures: pass a closure and call it ────────────────────────────────────────────────

#[test]
fn non_capturing_closures() {
    // A closure that captures NOTHING, passed to a higher-order function and invoked. `MakeClosure` stores
    // `[entry]`; `CallValue` window-copies the arg to the body's base register and jumps → 42 == VM.
    assert_chip_eq_vm(
        "closure-apply",
        "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nShow apply((n: Int) -> n * 2, 21).\n",
    );
    assert_chip_eq_vm(
        "closure-apply3",
        "## To apply3 (f: fn(Int) -> Int) -> Int:\n    Return f(3).\n\n## Main\nShow apply3((n: Int) -> n + 100).\n",
    );

    // FUZZ the closure body's constant and the argument — the indirect-call arg window must reach the
    // closure's parameter for every value.
    let mut st = 0xC105_0BEE_7777_9999u64;
    for _ in 0..40 {
        let k = (mix(&mut st) % 1000) as i64;
        let x = (mix(&mut st) % 1000) as i64;
        assert_chip_eq_vm(
            "fuzz-closure",
            &format!("## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nShow apply((n: Int) -> n + {k}, {x}).\n"),
        );
    }
}

// ── deterministic concurrency: channels + Launch (against the CONCURRENCY oracle) ──────────────────────

#[test]
fn deterministic_concurrency_channels() {
    // The producer/consumer: `Launch a task to produce with jobs` runs SYNCHRONOUSLY (deterministic
    // offline Launch), the producer sends into the FIFO channel, then Main receives it. The plain
    // `vm_outcome` DECLINES scheduler ops, so this is checked against `vm_outcome_concurrent` (the
    // deterministic cooperative VM), which the chip's Spawn/Chan* semantics mirror → prints 1.
    assert_chip_eq_vm_concurrent(
        "chan-prodcons",
        "## To produce (ch: Int):\n    Send 1 into ch.\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to produce with jobs.\n    Receive first from jobs.\n    Show first.\n",
    );
    // Two sends → two FIFO receives, in order.
    assert_chip_eq_vm_concurrent(
        "chan-two",
        "## To produce (ch: Int):\n    Send 1 into ch.\n    Send 2 into ch.\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to produce with jobs.\n    Receive first from jobs.\n    Receive second from jobs.\n    Show first.\n    Show second.\n",
    );

    // FUZZ the sent value — the channel ring + windowed spawn arg-copy must deliver it byte-identically.
    let mut st = 0xBEEF_CAFE_0102_0304u64;
    for _ in 0..40 {
        let v = (mix(&mut st) % 1_000_000) as i64;
        assert_chip_eq_vm_concurrent(
            "fuzz-chan",
            &format!("## To produce (ch: Int):\n    Send {v} into ch.\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to produce with jobs.\n    Receive first from jobs.\n    Show first.\n"),
        );
    }
}

// ── integer power: the sequential-multiply Pow unit (op 18) ────────────────────────────────────────────

#[test]
fn integer_power() {
    // `a ** b` (and its English `to the power of`) computes on the chip's sequential power unit: acc = 1,
    // then multiply by `a`, `b` times (so `a**0 == 1`). Byte-identical to the VM's `int_power`.
    assert_uses_opcode("pow", "## Main\n    Show 2 ** 10.\n", 18);
    assert_chip_eq_vm("pow-small", "## Main\n    Show 2 ** 10.\n");
    assert_chip_eq_vm("pow-english", "## Main\n    Show 2 to the power of 10.\n");
    assert_chip_eq_vm("pow-zero", "## Main\n    Show 7 ** 0.\n");
    assert_chip_eq_vm("pow-one", "## Main\n    Show 999 ** 1.\n");
    assert_chip_eq_vm("pow-base-zero", "## Main\n    Show 0 ** 5.\n");
    assert_chip_eq_vm("pow-neg-base-even", "## Main\n    Show (0 - 3) ** 4.\n");
    assert_chip_eq_vm("pow-neg-base-odd", "## Main\n    Show (0 - 3) ** 3.\n");

    // FUZZ base (both signs) and a small non-negative exponent — every product stays in range, so the model
    // (BigInt-exact) and the RTL (32-bit wrapping) agree, and both match the VM. A power-unit accumulator /
    // count-down bug diverges.
    let mut st = 0x9051_1234_ABCD_0001u64;
    for _ in 0..120 {
        let base = (mix(&mut st) % 21) as i64 - 10; // −10..=10
        let exp = mix(&mut st) % 6; // 0..=5 (10**5 = 100000 fits i32)
        assert_chip_eq_vm(
            "fuzz-pow",
            &format!("## Main\n    Let a be {base}.\n    Show a ** {exp}.\n"),
        );
    }
}

// ── tuple destructure in a `Repeat for (a, b) in …` loop (op 71) ───────────────────────────────────────

#[test]
fn tuple_destructure_in_loop() {
    // `Repeat for (a, b) in pairs` iterates a list of tuples: each `IterNext` yields a tuple pointer,
    // `DestructureTuple` spreads its payload into the loop's two registers → Show a / Show b == the VM.
    let prog = "## Main\n    Let pairs be [(1, 10), (2, 20), (3, 30)].\n    Repeat for (a, b) in pairs:\n        Show a.\n        Show b.\n";
    assert_uses_opcode("destructure", prog, 71);
    assert_chip_eq_vm("destructure-pairs", prog);
    // A THREE-element tuple pattern — a wrong spread width (off-by-one) diverges from the VM.
    assert_chip_eq_vm(
        "destructure-triple",
        "## Main\n    Let rows be [(1, 2, 3), (4, 5, 6)].\n    Repeat for (a, b, c) in rows:\n        Show a.\n        Show b.\n        Show c.\n",
    );

    // FUZZ the tuple payloads — the spread must land each element in the right register for arbitrary values.
    let mut st = 0x7071_1E5C_A77E_0002u64;
    for _ in 0..80 {
        let a = (mix(&mut st) % 1_000_000) as i64;
        let b = (mix(&mut st) % 1_000_000) as i64;
        let c = (mix(&mut st) % 1_000_000) as i64;
        assert_chip_eq_vm(
            "fuzz-destructure",
            &format!("## Main\n    Let xs be [({a}, {b}), ({c}, {a})].\n    Repeat for (p, q) in xs:\n        Show p.\n        Show q.\n"),
        );
    }
}

// ── deterministic scheduler residue: Sleep, non-blocking channels, handle-bound launch, select ─────────

#[test]
fn sleep_is_a_deterministic_noop() {
    // On the deterministic single-node runtime a `Sleep` is a pure scheduler no-op (op 88) — the program's
    // OUTPUT is unchanged. Checked against the concurrency oracle (the plain VM declines scheduler ops).
    let prog = "## Main\n    Sleep 100.\n    Show \"done\".\n";
    assert_uses_opcode("sleep", prog, 88);
    assert_chip_eq_vm_concurrent("sleep", prog);
    // A Sleep between two Shows must not perturb their order or values.
    assert_chip_eq_vm_concurrent(
        "sleep-ordered",
        "## Main\n    Show 1.\n    Sleep 50.\n    Show 2.\n    Sleep 25.\n    Show 3.\n",
    );
}

#[test]
fn non_blocking_channels() {
    // `Try to send` (ChanTrySend, op 95) into a fresh channel succeeds; `Try to receive` (ChanTryRecv, op
    // 96) then dequeues it FIFO → 99, byte-identical to the deterministic cooperative VM.
    let prog = "## Main\n    Let ch be a Pipe of Int.\n    Try to send 99 into ch.\n    Try to receive x from ch.\n    Show x.\n";
    assert_uses_opcode("try-send", prog, 95);
    assert_uses_opcode("try-recv", prog, 96);
    assert_chip_eq_vm_concurrent("try-send-recv", prog);

    // FUZZ the sent value — the non-blocking ring enqueue/dequeue must deliver it byte-identically.
    let mut st = 0xC0A1_9500_9600_0003u64;
    for _ in 0..40 {
        let v = (mix(&mut st) % 1_000_000) as i64;
        assert_chip_eq_vm_concurrent(
            "fuzz-try-chan",
            &format!("## Main\n    Let ch be a Pipe of Int.\n    Try to send {v} into ch.\n    Try to receive x from ch.\n    Show x.\n"),
        );
    }
}

#[test]
fn handle_bound_launch_and_abort() {
    // `Let h be Launch a task to worker with jobs` binds a task handle (SpawnHandle, op 99): the worker
    // runs synchronously, sends into the channel, then `Stop h` (TaskAbort, op 101) is a no-op — the child
    // already ran to completion. Main receives the sent value → 7, matching the concurrency VM.
    let prog = "## To worker (ch: Int):\n    Send 7 into ch.\n## Main\n    Let jobs be a Pipe of Int.\n    Let h be Launch a task to worker with jobs.\n    Receive first from jobs.\n    Stop h.\n    Show first.\n";
    assert_uses_opcode("spawn-handle", prog, 99);
    assert_uses_opcode("task-abort", prog, 101);
    assert_chip_eq_vm_concurrent("spawn-handle-stop", prog);

    // FUZZ the sent value through the handle-bound launch.
    let mut st = 0x5A57_9900_0101_0004u64;
    for _ in 0..40 {
        let v = (mix(&mut st) % 1_000_000) as i64;
        assert_chip_eq_vm_concurrent(
            "fuzz-spawn-handle",
            &format!("## To worker (ch: Int):\n    Send {v} into ch.\n## Main\n    Let jobs be a Pipe of Int.\n    Let h be Launch a task to worker with jobs.\n    Receive first from jobs.\n    Stop h.\n    Show first.\n"),
        );
    }
}

#[test]
fn select_wait_arms() {
    // `Await the first of:` with only a timeout arm (SelectArmTimeout 103 + SelectWait 104): no recv is
    // ready, so the timeout fires → "timeout", matching the deterministic concurrency VM.
    let timeout_only = "## Main\n    Let p be a Pipe of Text.\n    Await the first of:\n        After 1 seconds:\n            Show \"timeout\".\n";
    assert_uses_opcode("select-timeout-arm", timeout_only, 103);
    assert_uses_opcode("select-wait", timeout_only, 104);
    assert_chip_eq_vm_concurrent("select-timeout", timeout_only);

    // A recv arm (SelectArmRecv 102) whose channel a synchronously-launched producer already filled: the
    // recv fires (not the timeout), delivering the value → 5.
    let recv_ready = "## To producer (ch: Int):\n    Send 5 into ch.\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to producer with jobs.\n    Await the first of:\n        Receive x from jobs:\n            Show x.\n        After 10 seconds:\n            Show 0.\n";
    assert_uses_opcode("select-recv-arm", recv_ready, 102);
    assert_chip_eq_vm_concurrent("select-recv", recv_ready);

    // FUZZ the value delivered through the ready recv arm.
    let mut st = 0x5E1E_C702_0304_0005u64;
    for _ in 0..40 {
        let v = (mix(&mut st) % 1_000_000) as i64;
        assert_chip_eq_vm_concurrent(
            "fuzz-select-recv",
            &format!("## To producer (ch: Int):\n    Send {v} into ch.\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to producer with jobs.\n    Await the first of:\n        Receive x from jobs:\n            Show x.\n        After 10 seconds:\n            Show 0.\n"),
        );
    }
}

// ── offline single-node networking: the loopback inbox + no-op transport (ops 105..=111) ────────────────

#[test]
fn offline_networking() {
    // `Send v to peer` then `Await response from peer` round-trips through the loopback inbox on a single
    // node (NetSend 107 publishes (peer key, message); NetAwait 109 drains it) → the sent value. `Listen`
    // (106) and `NetMakePeer` (110) set up the offline identity. Checked against the offline-networking VM.
    let round_trip = "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send 42 to peer.\n    Await response from peer into got.\n    Show got.\n";
    assert_uses_opcode("net-send", round_trip, 107);
    assert_uses_opcode("net-await", round_trip, 109);
    assert_uses_opcode("net-makepeer", round_trip, 110);
    assert_chip_eq_vm_net("net-round-trip", round_trip);

    // `Connect` (105) is an offline no-op — the program's output is unchanged.
    let connect = "## Main\n    Connect to \"ws://127.0.0.1:7777\".\n    Show \"connected\".\n";
    assert_uses_opcode("net-connect", connect, 105);
    assert_chip_eq_vm_net("net-connect", connect);

    // `Sync var on topic` (NetSync 111) is an offline no-op: with no peer to merge, the synced CRDT value is
    // unchanged, so the counter still reads its locally-bumped 3.
    let sync = "## Definition\nA Tally has:\n    n: a Counter.\n## Main\n    Let mutable state be a new Tally.\n    Increase state's n by 3.\n    Sync state on \"topic\".\n    Show state's n.\n";
    assert_uses_opcode("net-sync", sync, 111);
    assert_chip_eq_vm_net("net-sync", sync);

    // FUZZ the message value through the loopback round-trip — the inbox keyed by the peer must deliver it.
    let mut st = 0x4E37_1070_9010_0006u64;
    for _ in 0..40 {
        let v = (mix(&mut st) % 1_000_000) as i64;
        assert_chip_eq_vm_net(
            "fuzz-net",
            &format!("## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send {v} to peer.\n    Await response from peer into got.\n    Show got.\n"),
        );
    }
}

// ── PIT OF SUCCESS: the encoder must never SILENTLY miscompile an op it cannot lower ──────────────────
//
// The full-core encoder is honest: an op with no sound chip lowering is REJECTED with a diagnostic, not
// emitted as words that compute the wrong answer on silicon. This is the same guard the scalar encoder
// uses for `Mod`. As each category is lowered (P0b-gap), its op moves from this rejection list into a
// green differential above — so this test SHRINKS as coverage grows and never lets a gap become silent.
#[test]
fn encoder_rejects_unlowered_ops_loudly() {
    let must_reject = |name: &str, src: &str, op: &str| {
        match compile_to_full_words(src) {
            Ok(_) => panic!("{name}: expected the encoder to REJECT `{op}` (no sound chip lowering yet), but it emitted words — a silent miscompile risk"),
            Err(e) => assert!(
                e.contains(op) || e.contains("no full-core chip lowering"),
                "{name}: rejection must name the unlowered op `{op}`, got: {e}"
            ),
        }
    };
    // NOTE: structs (NewStruct), inductives+match (NewInductive), float-seq literals (NewList of floats),
    // and deterministic channels (ChanNew) are NO LONGER on this list — each now LOWERS and runs == the VM
    // (proven GREEN by `structs_construct_and_read`, `inductives_and_match`, `float_seq_literal`, and
    // `deterministic_concurrency_channels` above). Likewise maps: both key families lower (the int-key path
    // MapIndexInt/MapSetIndexInt ops 223/224 by `soft_core_width_parity`; the string-key path SetIndex/Index
    // 57/59 `key_eq` by the RTL lock `the_soft_core_runs_logos_programs_identically`).

    // HONEST BOUNDARY — REAL-TIME (`now`/`today`, model ops 72/73): a wall-clock read is a NONDETERMINISTIC
    // HOST EFFECT, exactly like real IO / networking / RNG. `vm_outcome("Show now.")` returns a live host
    // timestamp (e.g. "2026-08-17 01:03"); the chip has no RTC, so the model returns a placeholder 0 that
    // legitimately DIFFERS from the host clock. There is nothing to differentially lock — a "match" could
    // only be faked by pinning the clock, which would hide the boundary. So `LoadNow`/`LoadToday` stay
    // REJECTED (the encoder emits no words for a program that reads real time), consistent with the plan's
    // HARD host boundary: the chip differs NOWHERE it CAN, and honestly declines where it cannot.
    must_reject("now", "## Main\n    Show now.\n", "LoadNow");
    must_reject("today", "## Main\n    Show today.\n", "LoadToday");

    // HONEST BOUNDARY — POLICY EVALUATION (`Check … can/is …`, `CheckPolicy` op 44): the check GRANTS iff
    // the subject satisfies its policy CONDITION — an arbitrary predicate over the subject's fields (a Text
    // equality `role equals "admin"`, a disjunction, a capability delegating to another predicate). The
    // chip-for compile path (`compile_for_chip`) drops the `## Policy` registry, so the emitted
    // `Op::CheckPolicy` carries only the predicate NAME, never the resolved condition — the encoder cannot
    // see WHICH field (or comparison) grants. The soft-core's op-44 models a single "capability field is
    // truthy" proxy, which is NOT equivalent to a Text-equality / disjunctive policy, so lowering it would
    // risk a SILENT miscompile (granting where the VM denies, or vice versa). Resolving a general policy to
    // a field-level boolean is a compile-front-end change (threading the policy registry into the compiled
    // program), out of the encoder's scope — so a `Check` stays REJECTED, honestly, rather than mis-lowered.
    must_reject(
        "check-policy",
        "## Definition\nA User has:\n    a role: Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\n\n## Main\n    Let u be a new User with role \"admin\".\n    Check that u is admin.\n    Show \"ok\".\n",
        "CheckPolicy",
    );

    // HONEST BOUNDARY — HOST / reference-value BUILTINS (`CallBuiltin`): the numeric tower lowers
    // (`abs`/`min`/`max`/`count_ones`/`sqrt`/`floor`/`ceil`/`round` via the op-38 coprocessor + `float_shape`),
    // and the deterministic string/fill/parse/encode builtins now lower too (`chr`/`contains`/`repeatSeq`/
    // `parseInt`/`text_bytes` — opcodes 233–237, proven `chip == VM` in `deterministic_string_and_fill_builtins`,
    // `parse_int_builtin_matches_vm_including_traps` and `text_bytes_builtin_matches_vm`), but a HOST builtin with no deterministic chip form stays
    // REJECTED. `pow(a, b)` is the polymorphic `pow` BUILTIN (a negative exponent is a float power — distinct
    // from the `**` operator, which lowers via `Op::Pow`); `parseFloat` is IEEE-754 `str::parse::<f64>()`, a
    // substantial rounding subsystem best realized as a dedicated accelerator, not a single tile. Neither has a
    // clean chip form, so the encoder rejects each loudly rather than mis-lowering. (The soft-core `CallBuiltin`
    // frontier reached ∅: every builtin either lowers or is a Boundary — see
    // `soft_core_op_support_lock::DEFERRED_FRONTIER`.)
    must_reject(
        "pow-builtin",
        "## Main\n    Show pow(2, 3).\n",
        "CallBuiltin",
    );
    must_reject(
        "parse-float-builtin",
        "## Main\n    Show parseFloat(\"3.5\").\n",
        "CallBuiltin",
    );

    // POSITIVE: `parseInt("42")` — a bounded decimal char-scan (op 236) — used to be on the reject list above;
    // it now LOWERS and runs on the soft-core EXACTLY like the VM (proven exhaustively, with traps, in
    // `parse_int_builtin_matches_vm_including_traps`). A representative here keeps the two tests in lockstep.
    assert_chip_eq_vm(
        "parse-int-lowers",
        "## Main\n    Show parseInt(\"-2024\").\n",
    );

    // POSITIVE: the string-key map (`Map of Text to Int`) that used to be on the reject list now LOWERS and
    // runs on the soft-core EXACTLY like the VM — the string-key path is sound in sim, so rejecting it would
    // be a stale gate. This is the same program the reject list once carried, now asserted GREEN.
    assert_chip_eq_vm(
        "map-string-key",
        "## Main\n    Let mutable m be a new Map of Text to Int.\n    Set item \"a\" of m to 5.\n    Set item \"a\" of m to 9.\n    Set item \"b\" of m to 4.\n    Show item \"a\" of m.\n    Show length of m.\n",
    );
}

// ── NEWLY-LOWERED FRONTIER OPS: set / slice / swap / remove / field-push / tolerant-compare / offline argv
// and the numeric-builtin coprocessor. Each was a Deferred gap the encoder rejected; it now lowers against
// its model arm, proven chip == VM here (broad fuzzing complements the op-support lock's curated corpus).
#[test]
fn heap_set_slice_and_frontier_ops() {
    // `Swap a and b` (RevSwap, op 4) — the reversible register exchange.
    let swap = "## Main\n    Let mutable a be 11.\n    Let mutable b be 22.\n    Swap a and b.\n    Show a.\n    Show b.\n";
    assert_uses_opcode("swap", swap, 4);
    assert_chip_eq_vm("swap", swap);

    // `items i through j of xs` (SliceOp, op 65) — a 1-based inclusive subsequence. The chip's slice unit
    // takes IMMEDIATE nibble bounds, so the encoder constant-folds small literal bounds (`slice_bound_folds`).
    let slice = "## Main\n    Let xs be [10, 20, 30, 40, 50].\n    Let ys be items 2 through 4 of xs.\n    Show length of ys.\n    Show item 1 of ys.\n    Show item 3 of ys.\n";
    assert_uses_opcode("slice", slice, 65);
    assert_chip_eq_vm("slice", slice);
    // A CONSTANT-bounded slice whose upper bound exceeds the list length → empty (byte-identical to the VM's
    // `collections::slice`, which is empty when `end > len`); the bounds still fold (both ≤ 15).
    assert_chip_eq_vm(
        "slice-oob",
        "## Main\n    Let xs be [1, 2, 3].\n    Let ys be items 2 through 9 of xs.\n    Show length of ys.\n",
    );
    // A slice with a genuinely RUNTIME bound (a value READ from the list, not a constant the encoder can
    // fold) has no chip form — the immediate nibble unit can't carry it — so the encoder REJECTS it loudly
    // (a genuine hardware limit, honestly declined). NOTE: a bound bound to a constant (even via a `mutable`
    // let) DOES fold and lower — the const-prop traces it — so this uses a list element as the bound.
    match compile_to_full_words("## Main\n    Let xs be [3, 4, 5, 6].\n    Let j be item 1 of xs.\n    Let ys be items 1 through j of xs.\n    Show length of ys.\n") {
        Ok(_) => panic!("slice-runtime-bound: expected the encoder to REJECT a runtime slice bound (the chip's slice unit takes immediate nibble bounds), but it emitted words"),
        Err(e) => assert!(
            e.contains("SliceOp") || e.contains("slice") || e.contains("no full-core chip lowering"),
            "slice-runtime-bound: rejection must name the unlowered SliceOp bound, got: {e}"
        ),
    }

    // `a union b` / `a intersection b` (UnionOp/IntersectOp, ops 68/69) — fresh dedup'd set ops.
    let union = "## Main\n    Let a be a new Set of Int.\n    Add 1 to a.\n    Add 2 to a.\n    Add 3 to a.\n    Let b be a new Set of Int.\n    Add 3 to b.\n    Add 4 to b.\n    Let c be a union b.\n    Show length of c.\n";
    assert_uses_opcode("union", union, 68);
    assert_chip_eq_vm("union", union);
    let intersect = "## Main\n    Let a be a new Set of Int.\n    Add 1 to a.\n    Add 2 to a.\n    Add 3 to a.\n    Let b be a new Set of Int.\n    Add 2 to b.\n    Add 3 to b.\n    Add 9 to b.\n    Let c be a intersection b.\n    Show length of c.\n";
    assert_uses_opcode("intersect", intersect, 69);
    assert_chip_eq_vm("intersect", intersect);

    // `Remove v from s` (RemoveFrom, op 56) — swap-remove by value equality. Membership after remove is the
    // observable (a set is unordered, so positional indexing is not the VM contract).
    let remove = "## Main\n    Let mutable a be a new Set of Int.\n    Add 5 to a.\n    Add 6 to a.\n    Add 7 to a.\n    Remove 6 from a.\n    Show length of a.\n    If a contains 6:\n        Show 1.\n    Otherwise:\n        Show 0.\n    If a contains 5:\n        Show 1.\n    Otherwise:\n        Show 0.\n";
    assert_uses_opcode("remove", remove, 56);
    assert_chip_eq_vm("remove", remove);

    // `Push x to obj's field` (ListPushField, op 47) — struct-field-seq push.
    let pushfield = "## A Bag has:\n    An items: Seq of Int.\n\n## Main\n    Let mutable b be a new Bag with items (a new Seq of Int).\n    Push 7 to b's items.\n    Push 8 to b's items.\n    Push 9 to b's items.\n    Show length of b's items.\n    Show item 2 of b's items.\n";
    assert_uses_opcode("push-field", pushfield, 47);
    assert_chip_eq_vm("push-field", pushfield);

    // `args()` — argv, program name first, on every tier: a chip has no host argv, so it is `["logos"]`
    // (== the VM's with no host argv). The assembler builds it from ops the chip already runs — a fresh
    // list, the pooled Text `logos` (TextConst, op 216), one push — so all three must be in the image.
    let args = "## Main\n    Let a be args().\n    Show length of a.\n";
    let op = |name: &str| {
        logicaffeine_compile::vm::binisa::opcode_of_name(name).expect("a cataloged op") as u8
    };
    assert_uses_opcode("args: fresh list", args, op("NewEmptyList"));
    assert_uses_opcode("args: pooled program name", args, 216);
    assert_uses_opcode("args: push", args, op("ListPush"));
    assert_chip_eq_vm("args", args);
}

#[test]
fn approximate_equality_lowers() {
    // FLOAT `is approximately` — the isclose compare (`logos_approx_eq`), DECOMPOSED into the primitive
    // float ops (FEq/FSub/FAbs/FMax/FConst/FMul/FLe + int BitOr) by `float_shape`.
    assert_chip_eq_vm(
        "approx-equal-floats",
        "## Main\n    Let a be 1.0.\n    Let b be 1.0.\n    If a is approximately b:\n        Show 1.\n    Otherwise:\n        Show 0.\n",
    );
    assert_chip_eq_vm(
        "approx-close-floats",
        "## Main\n    Let a be 0.1 + 0.2.\n    Let b be 0.3.\n    If a is approximately b:\n        Show 1.\n    Otherwise:\n        Show 0.\n",
    );
    assert_chip_eq_vm(
        "approx-far-floats",
        "## Main\n    Let a be 1.0.\n    Let b be 2.0.\n    If a is approximately b:\n        Show 1.\n    Otherwise:\n        Show 0.\n",
    );
    // INTEGER `is approximately` — collapses to exact `==` (op 30 ALU), no float plane needed.
    assert_chip_eq_vm(
        "approx-int-equal",
        "## Main\n    Let a be 7.\n    Let b be 7.\n    If a is approximately b:\n        Show 1.\n    Otherwise:\n        Show 0.\n",
    );
    assert_chip_eq_vm(
        "approx-int-diff",
        "## Main\n    Let a be 7.\n    Let b be 8.\n    If a is approximately b:\n        Show 1.\n    Otherwise:\n        Show 0.\n",
    );
}

#[test]
fn numeric_builtin_coprocessor() {
    // INTEGER plane (op 38 ordinals 5/7/8/9) — abs / min / max / count_ones, BigInt-exact to `call_builtin`.
    let abs = "## Main\n    Show abs(0 - 42).\n    Show abs(42).\n    Show abs(0).\n";
    assert_uses_opcode("abs-int", abs, 38);
    assert_chip_eq_vm("abs-int", abs);
    assert_chip_eq_vm("min-max-int", "## Main\n    Show min(3, 9).\n    Show max(3, 9).\n    Show min(0 - 5, 0 - 2).\n    Show max(0 - 5, 0 - 2).\n");
    let cones = "## Main\n    Show count_ones(0).\n    Show count_ones(12).\n    Show count_ones(255).\n    Show count_ones(1023).\n";
    assert_uses_opcode("count-ones", cones, 38);
    assert_chip_eq_vm("count-ones", cones);

    // FLOAT plane (op 38 ordinals 36..=41) — abs / min / max (→ float) and floor / ceil / round (→ int).
    assert_chip_eq_vm("abs-float", "## Main\n    Let x be 0.0 - 3.5.\n    Show abs(x).\n    Let y be 2.25.\n    Show abs(y).\n");
    assert_chip_eq_vm("min-max-float", "## Main\n    Let a be 1.5.\n    Let b be 2.75.\n    Show min(a, b).\n    Show max(a, b).\n");
    assert_chip_eq_vm(
        "floor-ceil-round",
        "## Main\n    Let x be 2.7.\n    Show floor(x).\n    Show ceil(x).\n    Show round(x).\n    Let y be 0.0 - 2.7.\n    Show floor(y).\n    Show ceil(y).\n    Show round(y).\n    Let h be 2.5.\n    Show round(h).\n",
    );

    // FUZZ the integer coprocessor — abs / min / max / count_ones over a wide value range == VM exactly.
    let mut st = 0x9E37_79B9_7F4A_7C15u64;
    for _ in 0..60 {
        let a = (mix(&mut st) % 2_000_000) as i64 - 1_000_000;
        let b = (mix(&mut st) % 2_000_000) as i64 - 1_000_000;
        assert_chip_eq_vm("fuzz-abs", &format!("## Main\n    Show abs({a}).\n"));
        assert_chip_eq_vm("fuzz-min", &format!("## Main\n    Show min({a}, {b}).\n"));
        assert_chip_eq_vm("fuzz-max", &format!("## Main\n    Show max({a}, {b}).\n"));
        let u = mix(&mut st) as i64;
        assert_chip_eq_vm(
            "fuzz-count-ones",
            &format!("## Main\n    Show count_ones({u}).\n"),
        );
    }
}

// ── THE DETERMINISTIC STRING / FILL BUILTINS: chr / contains(TEXT) / repeatSeq. Each was a Deferred gap on
// the `CallBuiltin` frontier; it now lowers to a text/seq-plane chip opcode (233/234/235), byte-identical to
// `semantics::builtins::call_builtin`, so the soft-core `CallBuiltin` frontier reached ∅ (every builtin now
// either lowers or is an honest Boundary). Proven `chip == VM` here — the culmination of the frontier drive.
#[test]
fn deterministic_string_and_fill_builtins() {
    // `chr(code)` (op 233) — a fresh 1-char Text `[1, codepoint]`; a downstream `Show` decodes it via
    // ShowText (op 217) byte-identically to the VM's `RuntimeValue::Text(c.to_string())`.
    let chr =
        "## Main\n    Show chr(65).\n    Show chr(97).\n    Show chr(10).\n    Show chr(955).\n";
    assert_uses_opcode("chr", chr, 233);
    assert_chip_eq_vm("chr", chr);

    // Text `contains` — BOTH the OPERATOR (`s contains "sub"`, `Op::Contains` routed through the Text plane)
    // and the FUNCTION-CALL builtin (`contains(hay, needle)`), each op 234. The 0/1 result drives a branch
    // (a `Show` of the VM's Bool would print "true"/"false", which the chip's int datapath doesn't render).
    let contains_op = "## Main\n    Let s be \"hello world\".\n    If s contains \"world\":\n        Show 1.\n    Otherwise:\n        Show 0.\n    If s contains \"zzz\":\n        Show 111.\n    Otherwise:\n        Show 222.\n";
    assert_uses_opcode("contains-op", contains_op, 234);
    assert_chip_eq_vm("contains-op", contains_op);
    let contains_fn = "## Main\n    If contains(\"logos\", \"go\"):\n        Show 1.\n    Otherwise:\n        Show 0.\n    If contains(\"logos\", \"\"):\n        Show 3.\n    Otherwise:\n        Show 4.\n";
    assert_uses_opcode("contains-fn", contains_fn, 234);
    assert_chip_eq_vm("contains-fn", contains_fn);

    // `n copies of x` = `repeatSeq(x, n)` (op 235) — a fresh n-slot fill of a SCALAR element (`n ≤ 0` empty).
    let copies = "## Main\n    Let xs be 5 copies of 7.\n    Show length of xs.\n    Show item 1 of xs.\n    Show item 5 of xs.\n";
    assert_uses_opcode("copies", copies, 235);
    assert_chip_eq_vm("copies", copies);
    // The n=0 edge (empty fill) and a computed-count fill (a runtime register count) both match the VM.
    assert_chip_eq_vm(
        "copies-zero",
        "## Main\n    Let xs be 0 copies of 9.\n    Show length of xs.\n",
    );
    assert_chip_eq_vm("copies-runtime", "## Main\n    Let mutable k be 2.\n    Set k to k + 1.\n    Let xs be k copies of 4.\n    Show length of xs.\n    Show item 2 of xs.\n");

    // FUZZ each: a codepoint round-trips through chr→Show, a substring test matches Rust's `str::contains`,
    // and an n-slot fill has the right length + element == the VM exactly.
    let mut st = 0x1234_5678_9ABC_DEF0u64;
    for _ in 0..40 {
        let code = 33 + (mix(&mut st) % 90) as i64; // a printable ASCII range (no shell/format hazards)
        assert_chip_eq_vm("fuzz-chr", &format!("## Main\n    Show chr({code}).\n"));
        let n = (mix(&mut st) % 8) as i64;
        let v = (mix(&mut st) % 1000) as i64;
        let src = format!("## Main\n    Let xs be {n} copies of {v}.\n    Show length of xs.\n");
        assert_chip_eq_vm("fuzz-copies", &src);
    }
    // A handful of substring cases (present / absent / prefix / suffix / whole / empty needle) via the branch.
    for (hay, needle) in [
        ("abcdef", "cd"),
        ("abcdef", "xy"),
        ("abcdef", "abc"),
        ("abcdef", "def"),
        ("abcdef", "abcdef"),
        ("abcdef", ""),
    ] {
        let src = format!(
            "## Main\n    If contains(\"{hay}\", \"{needle}\"):\n        Show 1.\n    Otherwise:\n        Show 0.\n"
        );
        assert_chip_eq_vm("fuzz-contains", &src);
    }
}

// ── `parseInt(text)`: the bounded DECIMAL char-scan (op 236). Its sibling `parseFloat` stays a Boundary
// (IEEE-754 rounding is a substantial accelerator), but the integer parse is pure deterministic compute a
// tile can do — trim Unicode whitespace, an optional single leading `+`/`-`, then ≥1 ASCII digit, accumulating
// in the NEGATIVE magnitude so `-9223372036854775808` (= i64::MIN) parses where its positive twin overflows,
// EXACTLY as the VM's `str::trim().parse::<i64>()`. A non-numeric / empty / OVERFLOWING input has no i64
// value: the VM's `parse` returns `Err`, whose `?` aborts the run with prior output standing, so the chip
// TRAPS (halts, pre-trap output stands) — byte-identical. Proven `chip == VM` here to the point of absurdity.
#[test]
fn parse_int_builtin_matches_vm_including_traps() {
    // The op is actually emitted (the differential is not vacuous), and the happy path matches the VM.
    let basic = "## Main\n    Show parseInt(\"42\").\n";
    assert_uses_opcode("parse-int", basic, 236);
    assert_chip_eq_vm("parse-int", basic);

    // Every well-formed shape the VM accepts: positive, negative, zero, a `+` prefix, `-0`/`+0`, leading zeros,
    // surrounding whitespace (` \t\n`), and BOTH i64 boundaries (MAX and MIN — the negative-accumulation edge).
    for s in [
        "42",
        "-7",
        "0",
        "+5",
        "-0",
        "+0",
        "007",
        "-007",
        "  123  ",
        "\t-99\n",
        " +1 ",
        "1000000000",
        "9223372036854775807",
        "-9223372036854775808",
        "+9223372036854775807",
    ] {
        let src = format!("## Main\n    Show parseInt(\"{s}\").\n");
        assert_chip_eq_vm("parse-int-ok", &src);
    }

    // The TRAP path — the VM errors (its `?` aborts) on a non-numeric / empty / overflowing input. A leading
    // `Show 1.` proves the chip retires the SAME pre-trap output the VM does, then traps at the bad parse.
    // Cases: pure letters, trailing junk, an internal space, a lone/spaced sign, a decimal point, a digit
    // separator, a positive overflow past i64::MAX, a negative overflow past i64::MIN, and the empty string.
    for bad in [
        "abc",
        "12x",
        "1 2",
        "-",
        "+",
        "- 5",
        "3.5",
        "1_000",
        "9223372036854775808",
        "-9223372036854775809",
        "",
        "  ",
        "0x1F",
        "٠",
    ] {
        let src = format!("## Main\n    Show 1.\n    Show parseInt(\"{bad}\").\n    Show 2.\n");
        assert_chip_eq_vm("parse-int-trap", &src);
    }

    // FUZZ: a random i64 round-trips through its decimal string EXACTLY (both signs, the full range incl. the
    // boundaries), and each also survives a whitespace-padded form. The VM `str::parse` is the oracle both hit.
    let mut st = 0x0FED_CBA9_8765_4321u64;
    for _ in 0..200 {
        let v = mix(&mut st) as i64; // the full i64 range, so overflow is never spuriously hit (a valid i64 always parses)
        let src = format!("## Main\n    Show parseInt(\"{v}\").\n");
        assert_chip_eq_vm("fuzz-parse-int", &src);
        let padded = format!("## Main\n    Show parseInt(\"  {v}  \").\n");
        assert_chip_eq_vm("fuzz-parse-int-padded", &padded);
    }
}

// ── `text_bytes(text)`: the UTF-8 encoder (op 237). A Text on the chip is a codepoint list; its bytes are a
// fresh `Seq of Int` `[n, b0, …]` built by the range encoder (1 byte below U+80, 2 below U+800, 3 below
// U+10000, else 4), byte-identical to the VM's `s.as_bytes()`. Proven `chip == VM` over literal text of
// every width, both sides of every width boundary, the empty text, the `strings` benchmark's own fold, and a
// fuzz of random scalar values from every width class.
#[test]
fn text_bytes_builtin_matches_vm() {
    let basic = "## Main\n    Let b be text_bytes(\"aé€😀\").\n    Show length of b.\n    Repeat for x in b:\n        Show x.\n";
    assert_uses_opcode("text-bytes", basic, 237);
    assert_chip_eq_vm("text-bytes", basic);

    for text in [
        "",
        "a",
        "hello world",
        "é",
        "λx",
        "日本語",
        "a€b",
        "😀🎉",
        "Ω≈ç√∫",
        "\u{7f}\u{80}\u{7ff}\u{800}\u{ffff}\u{10000}\u{10ffff}",
    ] {
        let src = format!(
            "## Main\n    Let b be text_bytes(\"{text}\").\n    Show length of b.\n    Repeat for x in b:\n        Show x.\n"
        );
        assert_chip_eq_vm("text-bytes-literal", &src);
    }

    // Both sides of every width boundary, and the surrogate gap's neighbours, through `chr`.
    assert_chip_eq_vm(
        "text-bytes-boundaries",
        "## Main\n    Let cps be [0, 127, 128, 2047, 2048, 55295, 57344, 65535, 65536, 917569, 1114111].\n    Repeat for c in cps:\n        Let b be text_bytes(chr(c)).\n        Show length of b.\n        Repeat for x in b:\n            Show x.\n",
    );

    // An allocation after the encode lands past its last byte: neither Seq overwrites the other.
    assert_chip_eq_vm(
        "text-bytes-then-alloc",
        "## Main\n    Let c be text_bytes(\"€x\").\n    Let xs be 3 copies of 9.\n    Let d be text_bytes(\"\").\n    Show length of c.\n    Show item 1 of c.\n    Show item 4 of c.\n    Show item 3 of xs.\n    Show length of d.\n",
    );

    // The `strings` benchmark's body: a text built by interpolation, its spaces counted by char index, then
    // its bytes folded into a 28-bit rolling hash whose `h * 31` passes 2³² (the model promotes like the VM).
    assert_chip_eq_vm(
        "text-bytes-strings-benchmark",
        "## Main\nLet n be 40.\nLet result be \"\" with capacity n * 6.\nLet i be 0.\nWhile i is less than n:\n    Set result to result + i + \" \".\n    Set i to i + 1.\nLet count be 0.\nSet i to 1.\nWhile i is at most length of result:\n    If item i of result equals \" \":\n        Set count to count + 1.\n    Set i to i + 1.\nLet bytes be text_bytes(result).\nLet h be 0.\nRepeat for b in bytes:\n    Set h to (h * 31 + b) & 268435455.\nShow \"\" + count + \" \" + h.\n",
    );

    // FUZZ: 1–8 codepoints drawn evenly from the four width classes (any scalar value, surrogates excluded),
    // concatenated through `chr`, every byte and the count compared to the VM.
    let mut st = 0x7E57_B17E_5EED_0237u64;
    for _ in 0..200 {
        let n = 1 + mix(&mut st) % 8;
        let parts: Vec<String> = (0..n)
            .map(|_| {
                let r = mix(&mut st);
                let cp = match r % 4 {
                    0 => r >> 2 & 0x7F,
                    1 => 0x80 + (r >> 2) % (0x800 - 0x80),
                    2 => {
                        let c = 0x800 + (r >> 2) % (0x1_0000 - 0x800 - 0x800);
                        if c >= 0xD800 {
                            c + 0x800
                        } else {
                            c
                        }
                    }
                    _ => 0x1_0000 + (r >> 2) % (0x11_0000 - 0x1_0000),
                };
                format!("chr({cp})")
            })
            .collect();
        let src = format!(
            "## Main\n    Let t be {}.\n    Let b be text_bytes(t).\n    Show length of b.\n    Repeat for x in b:\n        Show x.\n",
            parts.join(" + ")
        );
        assert_chip_eq_vm("fuzz-text-bytes", &src);
    }
}
