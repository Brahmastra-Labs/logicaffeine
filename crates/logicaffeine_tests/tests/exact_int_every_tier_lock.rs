//! ★ LOCK — an `Int` that outgrows a machine word is exact on every tier. ★
//!
//! A LOGOS `Int` is unbounded: the VM promotes a value past i64 to a big integer. Each compiled tier holds an integer
//! exact wherever the shared decision says one can outgrow a word (`analysis::exactness`): bytecode C and
//! WebAssembly ask it of the bytecode (`vm::exactness`), AST C of the syntax tree. WebAssembly carries a tagged
//! word, the value itself while it fits and a handle to the host's big integer when it does not. Each row runs on
//! the tree-walker (the reference), the VM, AST C, bytecode C, the WebAssembly module through the test host and
//! through `largo run --emit wasm` (the Node host the browser runtime mirrors); every one prints the same bytes, or
//! fails with the same sentence after printing the same bytes.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test exact_int_every_tier_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

#[cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod compile_common;

#[path = "cli_common/mod.rs"]
mod cli_common;

use logicaffeine_compile::compile::{
    compile_to_c, compile_to_c_via_bytecode, tw_lines, vm_outcome,
};

/// One tier's run, as a process writes it: the bytes of its standard output, and, if it failed, of its standard
/// error. An engine that runs in this one hands back its shown lines and its sentence; each becomes a line.
struct Run {
    tier: &'static str,
    output: String,
    failure: Option<String>,
}

/// Shown lines (or one sentence) as a process writes them: each ended by a newline.
fn written<S: AsRef<str>>(lines: &[S]) -> String {
    lines
        .iter()
        .map(|line| format!("{}\n", line.as_ref()))
        .collect()
}

/// A C program built as a release C build is, run: what it printed and what it said when it failed.
fn run_c(
    tier: &'static str,
    c: Result<String, logicaffeine_compile::ParseError>,
    src: &str,
) -> Run {
    let c = c.unwrap_or_else(|e| panic!("the {tier} generator refused ({e:?}):\n{src}"));
    let dir = tempfile::tempdir().expect("temp dir");
    let (c_path, bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&c_path, &c).expect("write the C");
    let built = common::build_c_release(&c_path, &bin).expect("run the C compiler");
    assert!(
        built.status.success(),
        "the {tier} does not compile:\n{}\n{src}",
        String::from_utf8_lossy(&built.stderr)
    );
    let run = std::process::Command::new(&bin)
        .output()
        .expect("run the C program");
    Run {
        tier,
        output: String::from_utf8_lossy(&run.stdout).into_owned(),
        failure: (!run.status.success()).then(|| String::from_utf8_lossy(&run.stderr).into_owned()),
    }
}

/// What `src` does through `largo run --emit wasm` under Node.
fn through_node(src: &str) -> Run {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir_all(dir.path().join("src")).expect("src dir");
    std::fs::write(dir.path().join("src/main.lg"), src).expect("write the program");
    std::fs::write(
        dir.path().join("Largo.toml"),
        "[package]\nname = \"t\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    )
    .expect("write the manifest");
    let out = cli_common::largo()
        .current_dir(dir.path())
        .args(["-q", "run", "--emit", "wasm"])
        .output()
        .expect("run largo");
    Run {
        tier: "WebAssembly under Node",
        output: String::from_utf8_lossy(&out.stdout).into_owned(),
        failure: (!out.status.success()).then(|| String::from_utf8_lossy(&out.stderr).into_owned()),
    }
}

/// `src` on every tier but the reference.
fn every_tier(src: &str) -> Vec<Run> {
    let vm = vm_outcome(src);
    let mut runs = vec![
        Run {
            tier: "VM",
            output: vm.output,
            failure: vm.error.map(|why| written(&[why])),
        },
        run_c("AST C", compile_to_c(src), src),
        run_c("bytecode C", compile_to_c_via_bytecode(src), src),
    ];
    #[cfg(feature = "wasm-jit")]
    {
        let run = compile_common::wasm_host::run_outcome(src)
            .unwrap_or_else(|why| panic!("the module did not build ({why}):\n{src}"));
        runs.push(Run {
            tier: "WebAssembly",
            output: written(&run.lines),
            failure: run.failure.map(|why| written(&[why])),
        });
        if cli_common::require_tool("node") {
            runs.push(through_node(src));
        }
    }
    runs
}

/// `src` prints on every tier what it prints on the tree-walker.
fn exact_everywhere(src: &str) {
    let tw = tw_lines(src);
    assert_eq!(tw.error, None, "the tree-walker refused:\n{src}");
    for run in every_tier(src) {
        assert_eq!(
            run.failure, None,
            "{} failed:\n{src}\n{}",
            run.tier, run.output
        );
        assert_eq!(
            run.output,
            written(&tw.lines),
            "{}'s answer:\n{src}",
            run.tier
        );
    }
}

/// `src` fails on every tier where the tree-walker fails, with its sentence, after printing what it printed.
fn fails_alike(src: &str) {
    let tw = tw_lines(src);
    let why = tw
        .error
        .as_deref()
        .unwrap_or_else(|| panic!("the tree-walker ran it:\n{src}"));
    for run in every_tier(src) {
        assert_eq!(
            run.failure,
            Some(written(&[why])),
            "{}'s failure:\n{src}\n{}",
            run.tier,
            run.output
        );
        assert_eq!(
            run.output,
            written(&tw.lines),
            "what {} printed before failing:\n{src}",
            run.tier
        );
    }
}

#[test]
fn a_product_that_outgrows_the_word() {
    exact_everywhere(
        "## Main
Let mutable p be 1.
Let mutable i be 1.
While i is at most 30:
    Set p to p * i.
    Set i to i + 1.
Show p.
",
    );
}

#[test]
fn a_sum_that_doubles_past_the_word() {
    exact_everywhere(
        "## Main
Let mutable x be 1.
Let mutable i be 0.
While i is less than 70:
    Set x to x + x.
    Set i to i + 1.
Show x.
Show 0 - x.
",
    );
}

#[test]
fn fibonacci_past_two_to_the_sixty_third() {
    exact_everywhere(
        "## Main
Let mutable a be 0.
Let mutable b be 1.
Let mutable i be 0.
While i is less than 100:
    Let t be a + b.
    Set a to b.
    Set b to t.
    Set i to i + 1.
Show a.
Show b.
",
    );
}

#[test]
fn an_exact_result_comes_back_from_a_recursion() {
    exact_everywhere(
        "## To fact (n: Int) -> Int:
    If n is less than 2:
        Return 1.
    Return n * fact(n - 1).

## Main
Show fact(25).
Show fact(5).
",
    );
}

#[test]
fn an_exact_value_crosses_a_parameter_and_comes_back_through_a_mutable_one() {
    exact_everywhere(
        "## To square (x: mutable Int):
    Set x to x * x.

## To twice (y: Int) -> Int:
    Return y + y.

## Main
Let mutable a be 3037000500.
square(a).
Show a.
Show twice(a).
square(a).
Show a.
",
    );
}

#[test]
fn exact_values_compare_and_test_equal() {
    exact_everywhere(
        "## Main
Let mutable p be 1.
Let mutable i be 1.
While i is at most 25:
    Set p to p * i.
    Set i to i + 1.
Let mutable q be p.
Let bound be 1000000000000000000 * 1000.
If p is greater than bound:
    Show \"big\".
If p is less than 5:
    Show \"small\".
If p equals q:
    Show \"equal\".
Set q to q + 1.
If p is not q:
    Show \"different\".
Show p is at most q.
",
    );
}

#[test]
fn exact_values_divide_and_take_remainders() {
    exact_everywhere(
        "## Main
Let mutable p be 1.
Let mutable i be 1.
While i is at most 22:
    Set p to p * i.
    Set i to i + 1.
Show p / 7.
Show p % 1000007.
Show p // 1000007.
Let n be 0 - p.
Show n / 7.
Show n % 1000007.
Show n // 1000007.
",
    );
}

#[test]
fn a_power_past_the_word() {
    exact_everywhere(
        "## Main
Let base be 3.
Show base ** 50.
Show 2 ** 64.
",
    );
}

#[test]
fn an_exact_value_interpolates_into_text() {
    exact_everywhere(
        "## Main
Let mutable x be 1.
Let mutable i be 0.
While i is less than 80:
    Set x to x + x.
    Set i to i + 1.
Show \"two to the eighty is {x}.\".
",
    );
}

#[test]
fn an_exact_value_that_fits_indexes_a_list() {
    exact_everywhere(
        "## Main
Let xs be [10, 20, 30, 40].
Let mutable p be 1.
Let mutable i be 1.
While i is at most 25:
    Set p to p * i.
    Set i to i + 1.
Let mutable k be p.
Set i to 1.
While i is at most 25:
    Set k to k / i.
    Set i to i + 1.
Show item (k + 2) of xs.
",
    );
}

#[test]
fn an_exact_value_too_big_for_an_index_fails_alike() {
    fails_alike(
        "## Main
Let xs be [10, 20, 30, 40].
Let mutable p be 1.
Let mutable i be 1.
While i is at most 25:
    Set p to p * i.
    Set i to i + 1.
Show \"before\".
Show item p of xs.
",
    );
}

#[test]
fn an_exact_program_binding_is_read_by_a_function() {
    exact_everywhere(
        "## To report:
    Show total.

## Main
Let mutable total be 1.
Let mutable i be 1.
While i is at most 22:
    Set total to total * i.
    Set i to i + 1.
report().
",
    );
}

/// The integers on either side of where a tagged word stops holding the value itself, against constants: each
/// operation that crosses the edge is held, each that comes back is small again.
#[test]
fn arithmetic_across_the_edge_of_the_small_range() {
    exact_everywhere(
        "## Main
Let mutable x be 1.
Let mutable i be 0.
While i is less than 62:
    Set x to x * 2.
    Set i to i + 1.
Show x.
Let mutable y be x - 1.
Show y.
Show y + 1.
Show y - 1.
Show y * 2.
Show y * 3.
Show y * -1.
Show y / 2.
Show y % 1000.
Let mutable z be 0 - x.
Show z.
Show z - 1.
Show z + 1.
Show z * 2.
Show z * -1.
Show z / 2.
Show z / -1.
Show z % 7.
Show z // 3.
Show z // -3.
Show y is greater than 4611686018427387902.
Show z is less than -4611686018427387903.
",
    );
}

/// A machine word just past the small range — an element read out of a list — tagged as it enters an exact
/// variable: the host holds it, on either side of zero.
#[test]
fn a_word_past_the_small_range_enters_an_exact_variable() {
    exact_everywhere(
        "## Main
Let xs be [4611686018427387904, -4611686018427387905, 3].
Let mutable p be item 1 of xs.
Let mutable q be item 2 of xs.
Let mutable i be 0.
While i is less than 2:
    Set p to p * 3.
    Set q to q * 3.
    Set i to i + 1.
Show p.
Show q.
",
    );
}

#[test]
fn a_copy_of_an_exact_value_is_its_own() {
    exact_everywhere(
        "## Main
Let mutable p be 1.
Let mutable i be 1.
While i is at most 25:
    Set p to p * i.
    Set i to i + 1.
Let mutable q be p.
Set p to p * 2.
Show q.
Show p.
Set q to q - p.
Show q.
",
    );
}

#[test]
fn a_closure_takes_and_returns_an_exact_value() {
    exact_everywhere(
        "## Main
Let mutable p be 1.
Let mutable i be 1.
While i is at most 25:
    Set p to p * i.
    Set i to i + 1.
Let twice be (n: Int) -> n + n.
Let square be (n: Int) -> n * n.
Show twice(p).
Show square(p).
Show square(3).
",
    );
}

#[test]
fn a_real_computed_from_an_exact_value() {
    exact_everywhere(
        "## Main
Let mutable p be 1.
Let mutable i be 1.
While i is at most 25:
    Set p to p * i.
    Set i to i + 1.
Show p * 0.5.
Show p / 4.0.
",
    );
}

#[test]
fn a_function_holding_an_exact_value_returns_a_list() {
    exact_everywhere(
        "## To grow (xs: Seq of Int) -> Seq of Int:
    Let mutable result be xs.
    Let mutable p be 1.
    Let mutable i be 1.
    While i is at most 25:
        Set p to p * i.
        Set i to i + 1.
    Push p % 997 to result.
    Return result.

## Main
Let xs be [1, 2].
Show grow(xs).
",
    );
}

#[test]
fn an_exact_value_handed_to_a_real_parameter_widens() {
    exact_everywhere(
        "## To half (x: Real) -> Real:
    Return x / 2.0.

## Main
Let mutable p be 1.
Let mutable i be 1.
While i is at most 25:
    Set p to p * i.
    Set i to i + 1.
Show half(p).
",
    );
}

#[test]
fn a_function_stores_an_exact_value_into_a_program_binding() {
    exact_everywhere(
        "## To grow:
    Set total to total * total.

## Main
Let mutable total be 3037000500.
grow().
Show total.
grow().
Show total.
",
    );
}

#[test]
fn an_exact_value_and_a_word_trade_places() {
    exact_everywhere(
        "## Main
Let mutable a be 3.
Let mutable b be 5.
Repeat for i from 1 to 7:
    Set a to a * a.
Swap a and b.
Show a.
Show b.
Set b to b + 1.
Show b.
",
    );
}

/// A swap hands a machine word to a variable that outgrows one later, from a side never read again: the word
/// enters the exact variable at the swap itself.
#[test]
fn a_swap_hands_a_word_to_a_variable_that_outgrows_it() {
    exact_everywhere(
        "## Main
Let mutable a be 3.
Let mutable b be 5.
Let mutable n be 0.
Repeat for i from 1 to 4:
    Set n to n + i.
If n is greater than 5:
    Swap a and b.
Repeat for i from 1 to 7:
    Set a to a * a.
Show a.
",
    );
}

#[test]
fn exact_values_trade_places_around_a_loop() {
    exact_everywhere(
        "## Main
Let mutable a be 3.
Let mutable b be 5.
Repeat for i from 1 to 9:
    Set a to a * a.
    Swap a and b.
Show a.
Show b.
",
    );
}

#[test]
fn a_function_swaps_an_exact_value_with_a_word() {
    exact_everywhere(
        "## To grow (n: Int) -> Int:
    Let mutable a be n.
    Let mutable b be 2.
    Repeat for i from 1 to 7:
        Set a to a * a.
    Swap a and b.
    Return b + a.

## Main
Show grow(3).
Show grow(9223372036).
",
    );
}

/// The host holds an integer only while the module does: a loop that makes thousands of integers past a word
/// leaves the host holding none of them once the program ends.
#[cfg(feature = "wasm-jit")]
#[test]
fn a_long_exact_loop_gives_back_every_integer_it_held() {
    let src = "## To fib (n: Int) -> Int:
    Let mutable a be 0.
    Let mutable b be 1.
    Let mutable i be 0.
    While i is less than n:
        Let t be a + b.
        Set a to b.
        Set b to t.
        Set i to i + 1.
    Return a.

## Main
Let mutable k be 0.
Let mutable last be 0.
While k is less than 40:
    Set last to fib(300).
    Set k to k + 1.
Show last.
";
    let before = compile_common::wasm_host::held_integers();
    exact_everywhere(src);
    assert_eq!(
        compile_common::wasm_host::held_integers(),
        before,
        "the module ended still owning integers the host holds: a copy that was never given back"
    );
}

/// Both JavaScript hosts answer the module's exact integers with the one definition, `web_host::EXACT_INT_JS`.
#[test]
fn both_javascript_hosts_carry_the_one_exact_integer_host() {
    use logicaffeine_compile::web_host::{wasm_host_shim, BROWSER_RUNTIME_JS, EXACT_INT_JS};
    for (host, js) in [
        ("assets/web/runtime.js", BROWSER_RUNTIME_JS.to_string()),
        ("the node shim", wasm_host_shim("probe")),
    ] {
        assert!(
            js.contains(EXACT_INT_JS),
            "{host} must answer exact integers with `web_host::EXACT_INT_JS`, the one definition"
        );
        assert!(
            js.contains("...exactImports,"),
            "{host} must hand the module every exact-integer import"
        );
    }
}
