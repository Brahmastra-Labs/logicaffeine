//! A store's index proven in range from the equalities a loop keeps.
//!
//! A merge writes `item k of result` while `k` advances with one of `li` and `ri` on every pass, so `k = li + ri - 1`
//! at every visit to its loop; its guard bounds `li` and `ri` by the lengths of `left` and `right`, and the builds
//! before it make those lengths add up to the length of `result`. No guard bounds `k` itself, so the store kept its
//! bounds check, the one check left in mergesort's hot loop. The range oracle now carries the affine equalities among a
//! function's integers and sequence lengths (Karr's domain): a push adds one to a length, a counted loop ends with its
//! counter one past its bound, a function that returns a sequence as long as its argument says so at its calls, and
//! the store is proven from them. A store whose index the equalities do not bound keeps its check, and every program
//! here answers alike on every tier, a failing one included.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_c_output, assert_compiled_equals_interpreted, compile_to_rust, run_all_tiers};

const MERGE_SORT: &str = "## To mergeSort (arr: Seq of Int) -> Seq of Int:
    Let n be length of arr.
    If n is less than 2:
        Return arr.
    Let mid be n / 2.
    Let mutable left be a new Seq of Int.
    Let mutable right be a new Seq of Int.
    Let mutable i be 1.
    While i is at most mid:
        Push item i of arr to left.
        Set i to i + 1.
    While i is at most n:
        Push item i of arr to right.
        Set i to i + 1.
    Set left to mergeSort(left).
    Set right to mergeSort(right).
    Let mutable result be arr.
    Let mutable li be 1.
    Let mutable ri be 1.
    Let mutable k be 1.
    While li is at most length of left and ri is at most length of right:
        If item li of left is at most item ri of right:
            Set item k of result to item li of left.
            Set li to li + 1.
        Otherwise:
            Set item k of result to item ri of right.
            Set ri to ri + 1.
        Set k to k + 1.
    While li is at most length of left:
        Set item k of result to item li of left.
        Set li to li + 1.
        Set k to k + 1.
    While ri is at most length of right:
        Set item k of result to item ri of right.
        Set ri to ri + 1.
        Set k to k + 1.
    Return result.
";

/// `MERGE_SORT` with a `Main` that sorts 300 values from the shared generator and shows the ends, the length and a
/// checksum.
fn sorting_program(sort: &str) -> String {
    format!(
        "{sort}
## Main
Let mutable arr be a new Seq of Int.
Let mutable seed be 42.
Let mutable i be 0.
While i is less than 300:
    Set seed to (seed * 1103515245 + 12345) % 2147483648.
    Push (seed / 65536) % 1000 to arr.
    Set i to i + 1.
Set arr to mergeSort(arr).
Let mutable checksum be 0.
Set i to 1.
While i is at most length of arr:
    Set checksum to (checksum * 31 + item i of arr) % 1000000007.
    Set i to i + 1.
Show item 1 of arr.
Show item 300 of arr.
Show length of arr.
Show checksum.
"
    )
}

/// The text of the generated function `name`, from its `fn` to the next top-level `fn`.
fn function<'r>(rust: &'r str, name: &str) -> &'r str {
    let at = rust
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("no `fn {name}` in:\n{rust}"));
    let end = rust[at + 1..]
        .find("\nfn ")
        .map_or(rust.len(), |e| at + 1 + e);
    &rust[at..end]
}

/// The text of the first loop in `body` whose header names `guard`, through the end of its block.
fn loop_with<'r>(body: &'r str, guard: &str) -> &'r str {
    let at = body
        .find(guard)
        .unwrap_or_else(|| panic!("no loop on `{guard}` in:\n{body}"));
    let open = at + body[at..].find('{').expect("a loop body");
    let mut depth = 0;
    for (i, ch) in body[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &body[at..open + i + 1];
                }
            }
            _ => {}
        }
    }
    &body[at..]
}

/// Whether `code` reads or writes an item of `coll` without its check: an unchecked access, or a hint that assumes an
/// index below its length, which is undefined behavior when the index is not.
fn unchecked_on(code: &str, coll: &str) -> bool {
    [
        ".get_unchecked",
        ".borrow_mut().get_unchecked",
        ".borrow().get_unchecked",
    ]
    .iter()
    .any(|access| code.contains(&format!("{coll}{access}")))
        || code
            .lines()
            .any(|l| l.contains("assert_unchecked") && l.contains(&format!("{coll}.len()")))
}

/// The compiled program fails as the interpreter does: the same output, then the same failure sentence.
fn compiled_fails_alike(source: &str) {
    let compiled = common::run_logos(source);
    let interpreted = logicaffeine_compile::compile::tw_outcome(source);
    let want = interpreted.error.expect("the interpreter fails");
    assert!(
        !compiled.success,
        "the compiled program must fail as the interpreter does ({want}); it printed:\n{}\n\n{}",
        compiled.stdout, compiled.rust_code
    );
    assert_eq!(compiled.stdout.trim(), interpreted.output.trim());
    assert!(
        compiled.stderr.contains(&want),
        "the compiled failure {:?} is not the interpreter's {:?}\n\n{}",
        compiled.stderr,
        want,
        compiled.rust_code
    );
}

#[test]
fn the_merge_store_is_proven_in_range() {
    let rust = compile_to_rust(&sorting_program(MERGE_SORT)).unwrap();
    let merge = loop_with(function(&rust, "mergeSort"), "li <= ");
    assert!(
        merge.contains("get_unchecked_mut") && !merge.contains("item_front_mut(k)"),
        "`k = li + ri - 1` with `li` and `ri` within `left` and `right`, whose lengths add to `result`'s: the store \
         needs no check. Got:\n{merge}\n\n{rust}"
    );
}

#[test]
fn the_merge_sort_sorts_alike_on_every_tier() {
    let program = sorting_program(MERGE_SORT);
    let agreed = run_all_tiers(&program);
    assert!(agreed.error.is_none(), "{:?}", agreed.error);
    let lines: Vec<&str> = agreed.output.lines().collect();
    assert_eq!(lines.len(), 4, "{}", agreed.output);
    assert_eq!(lines[2], "300");
    assert!(lines[0].parse::<i64>().unwrap() <= lines[1].parse::<i64>().unwrap());
    assert_c_output(&program, agreed.output.trim());
    assert_compiled_equals_interpreted(&program);
}

#[test]
fn the_mergesort_benchmark_merges_without_a_check() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/programs/mergesort/main.lg");
    let rust = compile_to_rust(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let merge = loop_with(function(&rust, "mergeSort"), "li <= ");
    assert!(merge.contains("get_unchecked_mut"), "Got:\n{merge}");
}

/// The merge into a `result` two items shorter than `left` and `right` together: the loop stores up to one fewer
/// than both hold, so its last store is past the end.
const SHORT_RESULT: &str = "## To merge (left: Seq of Int, right: Seq of Int) -> Seq of Int:
    Let mutable result be a new Seq of Int.
    Let mutable i be 3.
    While i is at most length of left + length of right:
        Push 0 to result.
        Set i to i + 1.
    Let mutable li be 1.
    Let mutable ri be 1.
    Let mutable k be 1.
    While li is at most length of left and ri is at most length of right:
        If item li of left is at most item ri of right:
            Set item k of result to item li of left.
            Set li to li + 1.
        Otherwise:
            Set item k of result to item ri of right.
            Set ri to ri + 1.
        Set k to k + 1.
    Return result.

## Main
Let merged be merge([1, 3, 5], [2, 4, 6]).
Show length of merged.
";

#[test]
fn a_result_too_short_for_the_merge_keeps_its_check_and_fails_alike() {
    let rust = compile_to_rust(SHORT_RESULT).unwrap();
    let merge = loop_with(function(&rust, "merge"), "li <= ");
    assert!(
        !unchecked_on(merge, "result"),
        "`result` holds two items fewer than `left` and `right`: the store must keep its check. Got:\n{merge}"
    );
    let agreed = run_all_tiers(SHORT_RESULT);
    assert!(
        agreed.error.is_some(),
        "the last store is past the end of `result`: {:?} {:?}",
        agreed.output,
        agreed.error
    );
    compiled_fails_alike(SHORT_RESULT);
}

/// A loop that advances `k` by two on one path: `k` is not `li + ri - 1`.
const UNEVEN_STEPS: &str = "## Main
Let left be [1, 3, 5, 7].
Let right be [2, 4, 6, 8].
Let mutable result be a new Seq of Int.
Let mutable i be 1.
While i is at most 8:
    Push 0 to result.
    Set i to i + 1.
Let mutable li be 1.
Let mutable ri be 1.
Let mutable k be 1.
While li is at most length of left and ri is at most length of right:
    If item li of left is at most item ri of right:
        Set item k of result to item li of left.
        Set li to li + 1.
        Set k to k + 1.
    Otherwise:
        Set item k of result to item ri of right.
        Set ri to ri + 1.
        Set k to k + 2.
Show result.
";

#[test]
fn an_index_that_does_not_move_in_step_keeps_its_check() {
    let rust = compile_to_rust(UNEVEN_STEPS).unwrap();
    let merge = loop_with(&rust, "li <= ");
    assert!(
        !unchecked_on(merge, "result"),
        "`k` gains two on one path. Got:\n{merge}"
    );
    let agreed = run_all_tiers(UNEVEN_STEPS);
    assert!(
        agreed.error.is_some(),
        "the store passes the end of `result`: {:?} {:?}",
        agreed.output,
        agreed.error
    );
    compiled_fails_alike(UNEVEN_STEPS);
}

/// A merge whose left half comes back from a function one item longer, into a `result` one item shorter than the
/// input: a summary claiming the function keeps its argument's length would prove every store in range, and the last
/// one is past the end.
const GROWING_HALF: &str = "## To grow (arr: Seq of Int) -> Seq of Int:
    Let n be length of arr.
    If n is less than 3:
        Return arr.
    Let mutable out be arr.
    Push 9 to out.
    Return out.

## To mergeGrown (arr: Seq of Int) -> Seq of Int:
    Let n be length of arr.
    Let mid be n / 2.
    Let mutable left be a new Seq of Int.
    Let mutable right be a new Seq of Int.
    Let mutable i be 1.
    While i is at most mid:
        Push item i of arr to left.
        Set i to i + 1.
    While i is at most n:
        Push item i of arr to right.
        Set i to i + 1.
    Set left to grow(left).
    Let mutable result be copy of arr.
    Pop from result.
    Let mutable li be 1.
    Let mutable ri be 1.
    Let mutable k be 1.
    While li is at most length of left and ri is at most length of right:
        If item li of left is at most item ri of right:
            Set item k of result to item li of left.
            Set li to li + 1.
        Otherwise:
            Set item k of result to item ri of right.
            Set ri to ri + 1.
        Set k to k + 1.
    Return result.

## Main
Show mergeGrown([1, 3, 5, 7, 2, 4, 6, 8]).
";

#[test]
fn a_function_that_changes_a_length_carries_no_summary() {
    let rust = compile_to_rust(GROWING_HALF).unwrap();
    let merge = loop_with(function(&rust, "mergeGrown"), "li <= ");
    assert!(
        !unchecked_on(merge, "result"),
        "`grow` adds an item. Got:\n{merge}"
    );
    let agreed = run_all_tiers(GROWING_HALF);
    assert!(
        agreed.error.is_some(),
        "the merge stores eight items into seven: {:?} {:?}",
        agreed.output,
        agreed.error
    );
    compiled_fails_alike(GROWING_HALF);
}

/// A copy of a sequence grown through another name for it: the copy's length is not the original's.
const GROWN_THROUGH_AN_ALIAS: &str = "## Main
Let mutable a be [1, 2, 3].
Let mutable b be a.
Push 4 to b.
Let n be length of a.
Let mutable k be 1.
Let mutable total be 0.
While k is at most n:
    Set total to total + item k of b.
    Set k to k + 1.
Show total.
Show length of a.
Show length of b.
";

#[test]
fn a_push_through_another_name_answers_alike() {
    let agreed = run_all_tiers(GROWN_THROUGH_AN_ALIAS);
    assert!(agreed.error.is_none(), "{:?}", agreed.error);
    assert_compiled_equals_interpreted(GROWN_THROUGH_AN_ALIAS);
    assert_c_output(GROWN_THROUGH_AN_ALIAS, agreed.output.trim());
}

/// A map written by key: a store into a map may add an entry, so its length is not tracked.
const MAP_STORE: &str = "## Main
Let mutable m be a new Map of Int to Int.
Let mutable i be 1.
While i is at most 5:
    Set item i of m to i * i.
    Set i to i + 1.
Show length of m.
Show item 3 of m.
";

#[test]
fn a_map_store_answers_alike() {
    let agreed = run_all_tiers(MAP_STORE);
    assert_eq!(
        (agreed.output.trim(), agreed.error.as_deref()),
        ("5\n9", None)
    );
    assert_compiled_equals_interpreted(MAP_STORE);
}

/// A counted loop whose counter may start past its bound: it then runs no pass and the counter keeps its start, so
/// `i = n` at its exit holds only when `i` starts at most `n`. A read at `i` after it is proven only on that.
const NO_PASS_COUNT: &str = "## To probe (arr: Seq of Int, start: Int) -> Int:
    Let n be length of arr.
    Let mutable i be start.
    While i is at most n - 1:
        Set i to i + 1.
    Let mutable total be 0.
    Let mutable r be 1.
    While r is at most 2:
        If i is at least 1:
            Set total to total + item i of arr.
        Set r to r + 1.
    Return total.

## Main
Show probe([1, 2, 3], 2).
Show probe([1, 2, 3], 10).
";

#[test]
fn a_counted_loop_that_may_run_no_pass_keeps_its_counter() {
    let rust = compile_to_rust(NO_PASS_COUNT).unwrap();
    let probe = function(&rust, "probe");
    assert!(
        !unchecked_on(probe, "arr"),
        "`i` may start past `n - 1`, so it need not end at `n`: no read at `i` goes unchecked. Got:\n{probe}"
    );
    let agreed = run_all_tiers(NO_PASS_COUNT);
    assert_eq!(
        agreed.output.trim(),
        "6",
        "{:?} {:?}",
        agreed.output,
        agreed.error
    );
    assert!(
        agreed.error.is_some(),
        "probe([1, 2, 3], 10) reads item 10: {:?} {:?}",
        agreed.output,
        agreed.error
    );
    compiled_fails_alike(NO_PASS_COUNT);
}

/// Forty names moving in step with a loop's counter, each indexing a sequence: at every access the space holds forty
/// equalities. The proof substitutes them out and hands the prover only the constraints an access needs, so the
/// program compiles in moments and prints alike; a prover handed every equality as two inequalities grows its system
/// with each variable it eliminates, past any memory.
#[test]
fn a_loop_carrying_forty_names_in_step_compiles_promptly() {
    let names: Vec<String> = (1..=40).map(|k| format!("v{k}")).collect();
    let mut src = String::from(
        "## Main\nLet n be 50.\nLet mutable arr be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than n + 50:\n    Push i to arr.\n    Set i to i + 1.\n",
    );
    for (k, v) in names.iter().enumerate() {
        src += &format!("Let mutable {v} be {k}.\n");
    }
    src += "Let mutable total be 0.\nSet i to 1.\nWhile i is at most n:\n";
    for v in &names {
        src += &format!("    Set total to total + item ({v} + 1) of arr.\n");
    }
    for v in &names {
        src += &format!("    Set {v} to {v} + 1.\n");
    }
    src += "    Set i to i + 1.\nShow total.\n";
    let program = src.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(compile_to_rust(&program).is_ok());
    });
    let compiled = rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("the program compiles within 30 seconds");
    assert!(compiled, "the program compiles");
    assert_compiled_equals_interpreted(&src);
}

/// The tiny Qwen forward the GPU lock runs, generated from the committed seeded weights by
/// `scripts/accel/gen_qwen_tiny_lg.py`.
fn qwen_tiny_lg() -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = std::process::Command::new("python3")
        .current_dir(&root)
        .args(["scripts/accel/gen_qwen_tiny_lg.py", "--toks", "1,2,3"])
        .output()
        .expect("python3 runs the generator");
    assert!(
        out.status.success(),
        "the generator fails: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// This process's resident memory, from `/proc/self/statm`.
fn resident_bytes() -> u64 {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    statm
        .split_whitespace()
        .nth(1)
        .and_then(|p| p.parse::<u64>().ok())
        .unwrap_or(0)
        * 4096
}

/// A hundred loops of matrix-vector arithmetic in the tiny Qwen forward hold dozens of equalities at each access.
/// Lowering it to CUDA, which runs the C oracle's analysis and the bytecode's, stays within a few gigabytes and two
/// minutes; a prover handed every equality as two inequalities grew the process by gigabytes a second until the
/// box ran out of memory. The watchdog fails the test long before that.
#[test]
fn the_tiny_qwen_forward_lowers_without_the_prover_running_away() {
    if !common::require_tool("python3") {
        return;
    }
    let lg = qwen_tiny_lg();
    let start = std::time::Instant::now();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(logicaffeine_compile::compile::compile_to_cuda(&lg).is_ok());
    });
    loop {
        if let Ok(lowered) = rx.try_recv() {
            assert!(lowered, "the tiny Qwen forward lowers to CUDA");
            return;
        }
        let held = resident_bytes();
        assert!(held < 6 << 30, "the lowering holds {} MiB", held >> 20);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(120),
            "the lowering finishes within two minutes"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
