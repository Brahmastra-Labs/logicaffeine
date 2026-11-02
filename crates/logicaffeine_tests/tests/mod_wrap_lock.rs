//! `Opt::ModWrap`: a remainder by a constant `m > 0` of a value the range oracle bounds to `[0, 2m)`.
//!
//! Such a value `x` has `x % m` equal to `x - m` when `x >= m` and to `x` otherwise, so the remainder compiles to
//! one compare and one subtract instead of a multiply-shift. A loop that carries a residue
//! (`Set b to (a + b) % 1000000007`) has the remainder on its critical path: fib_iterative ran 4.3x faster with it.
//! A value bounded below `m` is its own remainder. A value that may reach `2m`, or fall below zero, keeps its
//! remainder. Every case answers the same on every tier, the C tier included.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_c_output, build_c_release, compile_logos, compile_to_rust, run_all_tiers};
use logicaffeine_compile::compile::{
    compile_to_c, compile_to_c_via_bytecode, compile_to_c_via_bytecode_traced, optimizations_fired_vm,
    run_strict_tw_with_args, run_strict_vm_with_args,
};
use logicaffeine_compile::optimization::{decorate_source, Opt};
use logicaffeine_compile::ui_bridge::with_optimized_program;
use logicaffeine_compile::vm::{Compiler, Op};

/// The residue loop: `a` and `b` stay below `M`, so `a + b` is below `2M`.
const FIB: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable a be 0.
Let mutable b be 1.
Let mutable i be 0.
While i is less than n:
    Let temp be b.
    Set b to (a + b) % 1000000007.
    Set a to temp.
    Set i to i + 1.
Show a.
"#;

/// The text of the program from its first loop to its last `show(`.
fn loops_of(rust: &str) -> &str {
    let start = rust.find("for ").or_else(|| rust.find("while ")).expect("a loop");
    &rust[start..rust.rfind("show(").expect("the program shows its result")]
}

#[test]
fn a_carried_residue_wraps_with_one_subtraction() {
    let rust = compile_to_rust(FIB).unwrap();
    let loops = loops_of(&rust);
    assert!(
        !loops.contains("% 1000000007"),
        "`a + b` is below 2 * 1000000007, so its remainder is one conditional subtraction. Got:\n{rust}"
    );
    assert!(loops.contains(">= 1000000007"), "Got:\n{rust}");
}

#[test]
fn turning_the_optimization_off_keeps_the_remainder() {
    let rust = compile_to_rust(&decorate_source(FIB, &["modwrap"])).unwrap();
    assert!(loops_of(&rust).contains("% 1000000007"), "Got:\n{rust}");
}

#[test]
fn a_value_already_below_the_divisor_is_its_own_remainder() {
    let source = "## Main
Let mutable total be 0.
Let mutable i be 0.
While i is less than 5000:
    Let r be i % 1000.
    Set total to total + (r % 1000).
    Set i to i + 1.
Show total.
";
    let rust = compile_to_rust(source).unwrap();
    let loops = loops_of(&rust);
    assert!(
        !loops.contains("r % 1000") && loops.contains("i % 1000"),
        "`r` is below 1000: `r % 1000` is `r`, and only `i % 1000` divides. Got:\n{rust}"
    );
    let agreed = run_all_tiers(source);
    assert_eq!((agreed.output.trim(), agreed.error.as_deref()), ("2497500", None));
    assert_c_output(source, "2497500");
    common::assert_compiled_equals_interpreted(source);
}

#[test]
fn a_value_that_may_reach_twice_the_divisor_keeps_its_remainder() {
    // `x` and `y` reach 1000, so `x + y` reaches 2000: one subtraction would leave 1000 where `2000 % 1000` is 0.
    let source = "## Main
Let mutable total be 0.
Let mutable i be 0.
While i is less than 3003:
    Let x be i % 1001.
    Let y be (i * 7) % 1001.
    Set total to total + ((x + y) % 1000).
    Set i to i + 1.
Show total.
";
    let rust = compile_to_rust(source).unwrap();
    assert!(
        loops_of(&rust).contains("(x + y) % 1000"),
        "`x + y` may be 2000, past the one-subtraction range. Got:\n{rust}"
    );
    let agreed = run_all_tiers(source);
    assert!(agreed.error.is_none(), "{:?}", agreed.error);
    assert_c_output(source, agreed.output.trim());
    common::assert_compiled_equals_interpreted(source);
}

#[test]
fn a_value_that_may_be_negative_keeps_its_remainder() {
    let source = "## Main
Let mutable total be 0.
Let mutable i be 0.
While i is less than 2000:
    Let x be (i % 1000) - 500.
    Set total to total + (x % 1000) * 3.
    Set i to i + 1.
Show total.
";
    let rust = compile_to_rust(source).unwrap();
    assert!(loops_of(&rust).contains("x % 1000"), "a negative `x` keeps its remainder. Got:\n{rust}");
    let agreed = run_all_tiers(source);
    assert!(agreed.error.is_none(), "{:?}", agreed.error);
    assert_c_output(source, agreed.output.trim());
    common::assert_compiled_equals_interpreted(source);
}

#[test]
fn the_c_tier_wraps_the_carried_residue_too() {
    let c = compile_to_c(FIB).unwrap();
    let main = &c[c.rfind("int main(").expect("a C main")..];
    assert!(
        !main.contains("logos_imod(") && !main.contains("% 1000000007") && main.contains(">= 1000000007"),
        "the C tier computes the residue with one conditional subtraction too. Got:\n{main}"
    );
}

#[test]
fn the_residue_loop_answers_exactly_on_every_tier() {
    let aot = compile_logos(FIB);
    assert!(aot.success, "AOT build failed:\n{}", aot.stderr);
    let dir = tempfile::tempdir().unwrap();
    let (c_src, c_bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&c_src, compile_to_c(FIB).unwrap()).unwrap();
    let built = build_c_release(&c_src, &c_bin).unwrap();
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let mut fib = (0u64, 1u64);
    let mut expected = vec![(0u64, 0u64)];
    for k in 1..=2000u64 {
        fib = (fib.1, (fib.0 + fib.1) % 1_000_000_007);
        if [1, 2, 3, 44, 45, 46, 100, 1000, 2000].contains(&k) {
            expected.push((k, fib.0));
        }
    }
    for (n, want) in expected {
        let (n, want) = (n.to_string(), want.to_string());
        let argv = vec!["prog".to_string(), n.clone()];
        let tw = run_strict_tw_with_args(FIB, &argv);
        let vm = run_strict_vm_with_args(FIB, &argv);
        assert_eq!((tw.output.trim(), tw.error.as_deref()), (want.as_str(), None), "tree-walker, n = {n}");
        assert_eq!((vm.output.trim(), vm.error.as_deref()), (want.as_str(), None), "vm, n = {n}");
        for (tier, bin) in [("aot-rust", aot.binary_path.as_path()), ("c", c_bin.as_path())] {
            let out = std::process::Command::new(bin).arg(&n).output().unwrap();
            assert!(out.status.success(), "{tier}, n = {n}: {}", String::from_utf8_lossy(&out.stderr));
            assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), want, "{tier}, n = {n}");
        }
    }
}

fn benchmark(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../benchmarks/programs/{name}/main.lg"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn the_fib_iterative_and_coins_benchmarks_carry_their_residues_without_dividing() {
    for name in ["fib_iterative", "coins"] {
        let rust = compile_to_rust(&benchmark(name)).unwrap();
        assert!(
            !loops_of(&rust).contains("% 1000000007"),
            "{name}'s residue is below 2 * 1000000007 and wraps with one subtraction. Got:\n{rust}"
        );
    }
}

/// The ops the VM compiler emits for `src`, compiled as every bytecode tier compiles it: from the optimized program,
/// with the Oracle the AST emitters read.
fn vm_ops(src: &str) -> Vec<Op> {
    with_optimized_program(src, |parsed, interner| {
        let (stmts, types, _policies) = parsed.expect("program parses");
        let oracle = logicaffeine_compile::optimize::oracle_analyze_with(stmts, interner);
        Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
            .expect("program compiles")
            .code
    })
}

/// Each remainder the VM compiler emits: `(divisor, wrap)` for every `MagicDivU` remainder, and the count of `Mod`s.
fn remainders(src: &str) -> (Vec<(i64, bool)>, usize) {
    let ops = vm_ops(src);
    let magic = ops
        .iter()
        .filter_map(|op| match op {
            Op::MagicDivU { mul_back, divisor, wrap, .. } if *mul_back != 0 => Some((*divisor, *wrap)),
            _ => None,
        })
        .collect();
    (magic, ops.iter().filter(|op| matches!(op, Op::Mod { .. })).count())
}

/// The bytecode tiers decide the reduction from the Oracle the AST emitters read: the carried residue keeps its
/// `MagicDivU`, marked `wrap`, and the VM trace reports `modwrap`.
#[test]
fn the_vm_compiler_marks_the_carried_residue_as_one_subtraction() {
    assert_eq!(
        remainders(FIB),
        (vec![(1000000007, true)], 0),
        "`a + b` is below 2 * 1000000007: its remainder is a MagicDivU marked wrap"
    );
    assert!(
        optimizations_fired_vm(FIB).contains(&"modwrap"),
        "the VM trace reports the one-subtraction remainder: {:?}",
        optimizations_fired_vm(FIB)
    );
    let off = decorate_source(FIB, &["modwrap"]);
    assert_eq!(remainders(&off), (vec![(1000000007, false)], 0), "`## No modwrap` keeps the magic remainder");
    assert!(!optimizations_fired_vm(&off).contains(&"modwrap"));
}

/// A value the Oracle bounds below its divisor is its own remainder on the bytecode tiers too: no remainder is
/// emitted for it, while a value that reaches past twice the divisor keeps its unmarked remainder.
#[test]
fn the_vm_compiler_reduces_a_value_below_the_divisor_to_itself() {
    let source = "## Main
Let mutable total be 0.
Let mutable i be 0.
While i is less than 5000:
    Let r be i % 1000.
    Set total to total + (r % 1000).
    Set i to i + 1.
Show total.
";
    assert_eq!(
        remainders(source),
        (vec![(1000, false)], 0),
        "`r % 1000` is `r`; only `i % 1000`, which reaches 4999, keeps its remainder"
    );
}

/// A sum of two residues below the divisor reaches `2m - 2`: every value across the wrap, `m - 1`, `m` and
/// `2m - 2` among them, answers the same on every tier and on the C walker's build, against the arithmetic itself.
/// `i % 1000` (`i` below 2000) and `x + y` are below twice the divisor and keep a remainder marked `wrap`; `i / 2`
/// is below the divisor, so `(i / 2) % 1000` is `i / 2`.
#[test]
fn a_sum_of_two_residues_wraps_exactly_on_every_tier() {
    let source = "## Main
Let mutable total be 0.
Let mutable i be 0.
While i is less than 2000:
    Let x be i % 1000.
    Let y be (i / 2) % 1000.
    Set total to total + ((x + y) % 1000) * (i + 1).
    Set i to i + 1.
Show total.
";
    let want: i64 = (0..2000i64).map(|i| ((i % 1000 + (i / 2) % 1000) % 1000) * (i + 1)).sum();
    let sums: std::collections::BTreeSet<i64> = (0..2000i64).map(|i| i % 1000 + (i / 2) % 1000).collect();
    assert!(
        [999, 1000, 1998].iter().all(|s| sums.contains(s)),
        "the program crosses the wrap at m - 1, m and 2m - 2"
    );
    assert_eq!(remainders(source), (vec![(1000, true), (1000, true)], 0));
    let want = want.to_string();
    let agreed = run_all_tiers(source);
    assert_eq!((agreed.output.trim(), agreed.error.as_deref()), (want.as_str(), None));
    assert_c_output(source, &want);
    assert_eq!(run_bytecode_c(source), want, "the C walker's build");
    common::assert_compiled_equals_interpreted(source);
}

/// The C walker spells a `wrap` remainder as one conditional subtraction, reports it in its lowering trace, and
/// keeps the remainder under `## No modwrap`.
#[test]
fn the_bytecode_c_tier_wraps_the_carried_residue_too() {
    let (c, trace) = compile_to_c_via_bytecode_traced(FIB).unwrap();
    assert!(
        trace.lowering.fired(Opt::ModWrap),
        "the C walker reports the one-subtraction remainder: {trace:?}"
    );
    assert!(
        !c.contains("% 1000000007LL") && c.contains(">= 1000000007LL ?"),
        "the C walker computes the residue with one conditional subtraction. Got:\n{c}"
    );
    let (off, off_trace) = compile_to_c_via_bytecode_traced(&decorate_source(FIB, &["modwrap"])).unwrap();
    assert!(!off_trace.all_fired().fired(Opt::ModWrap), "{off_trace:?}");
    assert!(off.contains("% 1000000007LL"), "`## No modwrap` keeps the remainder. Got:\n{off}");
}

/// The C walker's build of the residue loop answers exactly where the residue crosses the divisor.
#[test]
fn the_bytecode_c_residue_loop_answers_exactly() {
    let mut fib = (0u64, 1u64);
    let mut expected = vec![(0u64, 0u64)];
    for k in 1..=2000u64 {
        fib = (fib.1, (fib.0 + fib.1) % 1_000_000_007);
        if [1, 2, 3, 44, 45, 46, 100, 1000, 2000].contains(&k) {
            expected.push((k, fib.0));
        }
    }
    let bin = build_bytecode_c(FIB);
    for (n, want) in expected {
        let out = std::process::Command::new(bin.path().join("main")).arg(n.to_string()).output().unwrap();
        assert!(out.status.success(), "n = {n}: {}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), want.to_string(), "n = {n}");
    }
}

/// `src` built through the C walker: the directory holding its binary, `main`.
fn build_bytecode_c(src: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let (c_src, c_bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&c_src, compile_to_c_via_bytecode(src).unwrap()).unwrap();
    let built = build_c_release(&c_src, &c_bin).unwrap();
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    dir
}

/// `src` built through the C walker and run: its trimmed stdout.
fn run_bytecode_c(src: &str) -> String {
    let bin = build_bytecode_c(src);
    let out = std::process::Command::new(bin.path().join("main")).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// SHA3's padding loop: the body pushes onto `padded`, so the length the condition reads grows past `136` and past
/// `272` though it is `201` when the loop is entered. A loop's condition is evaluated again after every iteration, so
/// its operands are bounded by what the body leaves, not by the state the loop is entered in.
const GROWN: &str = "## Main\n    Let mutable padded be a new Seq of Int.\n    Repeat for x from 1 to 200:\n        Push x to padded.\n    Push 6 to padded.\n    While ((length of padded) % 136) is greater than 0:\n        Push 0 to padded.\n    Show length of padded.\n";

/// A remainder in a loop condition whose dividend the body grows keeps its division on the VM, the C and the Rust tier:
/// reduced against the entry state's length it would never reach zero and the loop would never end.
#[test]
fn a_remainder_in_a_loop_condition_keeps_its_division_when_the_body_grows_its_dividend() {
    let (magic, mods) = remainders(GROWN);
    assert!(
        magic == vec![(136, false)] || (magic.is_empty() && mods == 1),
        "the VM keeps the condition's remainder by 136 unreduced: MagicDivU remainders {magic:?}, Mods {mods}"
    );
    let c = compile_to_c(GROWN).unwrap();
    let main = &c[c.rfind("int main(").expect("a C main")..];
    let cond = main.lines().find(|l| l.trim_start().starts_with("while (")).expect("a C while");
    assert!(cond.contains("136"), "the C tier keeps the remainder by 136 in the condition: {cond}");
    let rust = compile_to_rust(GROWN).unwrap();
    let main = &rust[rust.find("fn main(").expect("a Rust main")..];
    let cond = main.lines().find(|l| l.trim_start().starts_with("while ")).expect("a Rust while");
    assert!(cond.contains("136"), "the Rust tier keeps the remainder by 136 in the condition: {cond}");
}

/// The padding loop ends at the first multiple of 136 past the message on every tier — the VM under a step budget and
/// the C binary under a wall-clock limit, so a loop that never ends fails rather than hangs.
#[test]
fn a_loop_whose_condition_reads_a_grown_remainder_ends_on_every_tier() {
    for (src, want) in [(GROWN, "272"), (GROWN_FROM_EXACT, "4")] {
        let tw = run_strict_tw_with_args(src, &[]);
        assert_eq!((tw.output.trim(), tw.error.as_deref()), (want, None), "tree-walker: {src}");
        let _fuel = logicaffeine_compile::interpreter::install_fuel(10_000_000);
        let vm = run_strict_vm_with_args(src, &[]);
        logicaffeine_compile::interpreter::clear_fuel();
        assert_eq!((vm.output.trim(), vm.error.as_deref()), (want, None), "vm: {src}");
        let dir = tempfile::tempdir().unwrap();
        let (c_src, c_bin) = (dir.path().join("main.c"), dir.path().join("main"));
        std::fs::write(&c_src, compile_to_c(src).unwrap()).unwrap();
        let built = build_c_release(&c_src, &c_bin).unwrap();
        assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
        let out = std::process::Command::new("timeout").arg("20").arg(&c_bin).output().unwrap();
        assert_eq!(
            (String::from_utf8_lossy(&out.stdout).trim(), out.status.code()),
            (want, Some(0)),
            "c: {src}"
        );
    }
}

/// The same loop entered with a length known exactly: two pushes, then a remainder by 4 the body grows past. Read at
/// the entry state alone the length is 2, below the divisor, and the remainder would be taken as the length itself.
const GROWN_FROM_EXACT: &str = "## Main\n    Let mutable xs be a new Seq of Int.\n    Push 1 to xs.\n    Push 2 to xs.\n    While ((length of xs) % 4) is greater than 0:\n        Push 0 to xs.\n    Show length of xs.\n";
