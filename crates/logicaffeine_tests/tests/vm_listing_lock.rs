//! ★ LOCK — the bytecode listing shows the program the VM runs, as the benchmarks page's LOGOS (VM) tab reads it. ★
//!
//! `ui_bridge::vm_listing` compiles a source the way `largo run --interpret` does (the run path's optimizer, the oracle,
//! the VM compiler) and prints the bytecode one section per function, `## Main` first, each instruction at its pc. The
//! page calls it in the browser with the optimization toggles applied, so a `## No <X>` decorator must reach it.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test vm_listing_lock`

use logicaffeine_compile::optimization::{decorate_source, REGISTRY};
use logicaffeine_compile::ui_bridge::vm_listing;

const PROGRAM: &str = "## To square (x: Int) -> Int:
    Return x * x.

## To total (n: Int) -> Int:
    Let mutable s be 0.
    Repeat for i from 1 to n:
        Set s to s + square(i).
    Return s.

## Main
Let k be 2 + 3.
Show total(k * 40).
";

/// `(section header, [pc])` in listing order.
fn sections(listing: &str) -> Vec<(String, Vec<usize>)> {
    let mut out: Vec<(String, Vec<usize>)> = Vec::new();
    for line in listing.lines() {
        if line.starts_with("## ") {
            out.push((line.to_string(), Vec::new()));
        } else if !line.trim().is_empty() {
            let pc = line.trim_start().split_whitespace().next().unwrap_or("");
            let pc: usize = pc
                .parse()
                .unwrap_or_else(|_| panic!("an instruction line starts with its pc: {line:?}"));
            out.last_mut()
                .unwrap_or_else(|| panic!("an instruction before any section: {line:?}"))
                .1
                .push(pc);
        }
    }
    out
}

#[test]
fn every_instruction_is_listed_once_under_its_function() {
    let listing = vm_listing(PROGRAM).expect("the program compiles");
    let sections = sections(&listing);
    assert_eq!(sections[0].0, "## Main", "Main comes first:\n{listing}");
    for (header, _) in &sections[1..] {
        let name = header
            .strip_prefix("## To ")
            .unwrap_or_else(|| panic!("a function section: {header:?}\n{listing}"));
        assert!(
            !name.trim().is_empty(),
            "a function section names its function:\n{listing}"
        );
    }
    let mut pcs: Vec<usize> = sections
        .iter()
        .flat_map(|(_, p)| p.iter().copied())
        .collect();
    for (_, p) in &sections {
        assert!(
            p.windows(2).all(|w| w[0] + 1 == w[1]),
            "a section's pcs run consecutively:\n{listing}"
        );
    }
    pcs.sort_unstable();
    let n = pcs.len();
    assert!(n > 0, "the listing has instructions");
    assert_eq!(
        pcs,
        (0..n).collect::<Vec<_>>(),
        "every pc appears exactly once:\n{listing}"
    );
}

#[test]
fn a_recursive_function_keeps_its_own_section() {
    let listing = vm_listing(
        "## To fib (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show fib(20).
",
    )
    .expect("fib compiles");
    let headers: Vec<String> = sections(&listing).into_iter().map(|(h, _)| h).collect();
    assert!(
        headers.iter().any(|h| h.starts_with("## To fib")),
        "fib is a function the VM calls:\n{listing}"
    );
}

#[test]
fn the_listing_follows_the_optimization_toggles() {
    let all_on = vm_listing(PROGRAM).expect("all on");
    let every_keyword: Vec<&str> = REGISTRY.iter().map(|m| m.keyword).collect();
    let all_off = vm_listing(&decorate_source(PROGRAM, &every_keyword)).expect("all off");
    assert_ne!(
        all_on, all_off,
        "turning every optimization off must change the bytecode the VM runs"
    );
}

#[test]
fn a_program_that_does_not_parse_says_why() {
    let err = vm_listing("## Main\nLet be.\n").expect_err("no program");
    assert!(!err.trim().is_empty(), "the refusal says why");
}
