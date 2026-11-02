//! `Repeat for t from a to b` on the AOT-Rust tier.
//!
//! The loop compiles to a Rust `for` over a range. The half-open `a..b+1` is the form LLVM optimizes; an
//! inclusive `a..=b` (`RangeInclusive`) carries an exhausted flag through every iteration, and in sha256's
//! 64-round compression that flag cost 40% more instructions than C. The half-open form is exact only when
//! `b + 1` exists, so it is used for a literal `b` below `i64::MAX` and for a `b` the range oracle bounds below
//! it; any other `b` keeps the inclusive range, which reaches `i64::MAX` itself. Every case runs on every tier
//! with the same answer.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_c_output, build_c_release, compile_logos, compile_to_rust, run_all_tiers};
use logicaffeine_compile::compile::{compile_to_c, run_strict_tw_with_args, run_strict_vm_with_args};
use std::time::{Duration, Instant};

/// The `for` header the loop over `counter` compiled to.
fn loop_header<'r>(rust: &'r str, counter: &str) -> &'r str {
    let at = rust
        .find(&format!("for {counter} in "))
        .unwrap_or_else(|| panic!("no `for {counter} in` loop in:\n{rust}"));
    rust[at..].lines().next().unwrap()
}

#[test]
fn a_literal_range_compiles_half_open() {
    let source = "## Main
Let mutable total be 0.
Repeat for t from 16 to 63:
    Set total to total + t * t.
Show total.
";
    let rust = compile_to_rust(source).unwrap();
    let header = loop_header(&rust, "t");
    assert!(
        header.contains("16..64") && !header.contains("..="),
        "`from 16 to 63` is the half-open `16..64`. Got: {header}\n{rust}"
    );
    let agreed = run_all_tiers(source);
    assert_eq!((agreed.output.trim(), agreed.error.as_deref()), ("84104", None));
    assert_c_output(source, "84104");
    common::assert_compiled_equals_interpreted(source);
}

#[test]
fn a_range_ending_at_the_largest_int_stays_inclusive_and_exact() {
    let source = "## Main
Repeat for t from 9223372036854775805 to 9223372036854775807:
    Show t.
";
    let rust = compile_to_rust(source).unwrap();
    assert!(
        loop_header(&rust, "t").contains("..="),
        "`to 9223372036854775807` has no `end + 1`; the range stays inclusive. Got:\n{rust}"
    );
    let want = "9223372036854775805\n9223372036854775806\n9223372036854775807";
    let agreed = run_all_tiers(source);
    assert_eq!((agreed.output.trim(), agreed.error.as_deref()), (want, None));
    common::assert_compiled_equals_interpreted(source);
}

#[test]
fn an_empty_literal_range_runs_no_iteration() {
    let source = "## Main
Let mutable total be 0.
Repeat for t from 5 to 4:
    Set total to total + 1.
Show total.
";
    let rust = compile_to_rust(source).unwrap();
    assert!(!loop_header(&rust, "t").contains("..="), "Got:\n{rust}");
    let agreed = run_all_tiers(source);
    assert_eq!((agreed.output.trim(), agreed.error.as_deref()), ("0", None));
    common::assert_compiled_equals_interpreted(source);
}

/// `n` is bounded by its `% 1000`, so `n + 1` exists and the range is half-open.
const BOUNDED_END: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments) % 1000.
Let mutable total be 0.
Repeat for t from 1 to n:
    Set total to total + t.
Show total.
"#;

/// `n` is any Int: `n + 1` may not exist, so the range stays inclusive.
const UNBOUNDED_END: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable total be 0.
Repeat for t from n - 2 to n:
    Set total to total + 1.
Show total.
"#;

#[test]
fn an_end_the_oracle_bounds_compiles_half_open() {
    let rust = compile_to_rust(BOUNDED_END).unwrap();
    let header = loop_header(&rust, "t");
    assert!(
        !header.contains("..=") && header.contains("+ 1)"),
        "`n` is below 1000, so the loop is `1..(n + 1)`. Got: {header}\n{rust}"
    );
}

#[test]
fn an_unbounded_end_stays_inclusive() {
    let rust = compile_to_rust(UNBOUNDED_END).unwrap();
    assert!(
        loop_header(&rust, "t").contains("..="),
        "`n` may be the largest Int, so the range stays inclusive. Got:\n{rust}"
    );
}

/// How long one tier may take on one of these programs: each runs a handful of passes, so a tier still
/// running at the deadline is walking past the end of its range.
const DEADLINE: Duration = Duration::from_secs(60);

/// `bin` run with `args`, killed at [`DEADLINE`]: (stdout trimmed, stderr), or the failure.
fn run_binary(bin: &std::path::Path, args: &[&str]) -> Result<(String, String), String> {
    let mut child = std::process::Command::new(bin)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > DEADLINE {
            child.kill().ok();
            return Err(format!("{} was still running after {DEADLINE:?}", bin.display()));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().unwrap();
    let (stdout, stderr) = (
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    );
    if out.status.success() {
        Ok((stdout, stderr))
    } else {
        Err(format!("{} exited {}: {stderr}", bin.display(), out.status))
    }
}

/// An in-process engine's run, on a thread of its own so a tier that never ends fails at [`DEADLINE`].
fn run_engine(
    run: impl FnOnce() -> logicaffeine_compile::compile::EngineOutcome + Send + 'static,
) -> Result<String, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || tx.send(run()).ok());
    match rx.recv_timeout(DEADLINE) {
        Ok(o) => match o.error {
            None => Ok(o.output.trim().to_string()),
            Some(e) => Err(e),
        },
        Err(_) => Err(format!("still running after {DEADLINE:?}")),
    }
}

/// The C program `source` compiles to, built as a release C build builds it, in `dir`.
fn build_c(source: &str, dir: &std::path::Path) -> std::path::PathBuf {
    let c_code = compile_to_c(source).unwrap_or_else(|e| panic!("LOGOS→C: {e:?}"));
    let (c_src, c_bin) = (dir.join("main.c"), dir.join("main"));
    std::fs::write(&c_src, &c_code).unwrap();
    let built = build_c_release(&c_src, &c_bin).unwrap();
    assert!(built.status.success(), "C build failed:\n{}", String::from_utf8_lossy(&built.stderr));
    c_bin
}

/// Each tier's outcome for `source` run with `args` (no program name): tree-walker, VM, AOT-Rust, C.
fn every_tier(source: &str, args: &[&str]) -> Vec<(&'static str, Result<String, String>)> {
    let aot = compile_logos(source);
    assert!(aot.success, "AOT build failed:\n{}", aot.stderr);
    let dir = tempfile::tempdir().unwrap();
    let c_bin = build_c(source, dir.path());
    let argv: Vec<String> = std::iter::once("prog").chain(args.iter().copied()).map(String::from).collect();
    let (tw_src, tw_argv) = (source.to_string(), argv.clone());
    let (vm_src, vm_argv) = (source.to_string(), argv);
    vec![
        ("tree-walker", run_engine(move || run_strict_tw_with_args(&tw_src, &tw_argv))),
        ("vm", run_engine(move || run_strict_vm_with_args(&vm_src, &vm_argv))),
        ("aot-rust", run_binary(&aot.binary_path, args).map(|(out, _)| out)),
        ("c", run_binary(&c_bin, args).map(|(out, _)| out)),
    ]
}

#[test]
fn every_range_answers_exactly_on_every_tier() {
    for (source, n, want) in [
        (BOUNDED_END, "10", "55"),
        (BOUNDED_END, "1999", "499500"),
        (BOUNDED_END, "0", "0"),
        (BOUNDED_END, "-5", "0"),
        (UNBOUNDED_END, "10", "3"),
        (UNBOUNDED_END, "9223372036854775807", "3"),
        (UNBOUNDED_END, "-9223372036854775806", "3"),
    ] {
        for (tier, out) in every_tier(source, &[n]) {
            assert_eq!(out.as_deref(), Ok(want), "n = {n}: {tier}\n{source}");
        }
    }
}

#[test]
fn a_body_that_sets_its_loop_variable_changes_that_pass_only() {
    let source = "## Main
Repeat for t from 1 to 3:
    Set t to t + 10.
    Show t.
Repeat for u from 9223372036854775806 to 9223372036854775807:
    Set u to 0.
    Show u.
";
    for (tier, out) in every_tier(source, &[]) {
        assert_eq!(out.as_deref(), Ok("11\n12\n13\n0\n0"), "{tier}");
    }
}

#[test]
fn the_sha256_benchmark_compresses_over_half_open_ranges() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/programs/sha256/main.lg");
    let source = std::fs::read_to_string(&path).expect("benchmarks/programs/sha256/main.lg");
    let rust = compile_to_rust(&source).unwrap();
    assert!(
        !rust.contains("..="),
        "sha256's `Repeat for t from 16 to 63` and `from 0 to 63` are half-open loops. Got:\n{rust}"
    );
}
