//! ★ LOCK — the C both generators emit computes integers in defined C, and answers as the VM does. ★
//!
//! A signed overflow, and a shift by 64 or more, by a negative count or into the sign bit, are undefined in C: gcc
//! may fold them to anything, and a run that reaches one means nothing even when it prints the right digits. Every
//! row is built by AST C and by bytecode C with the undefined-behavior sanitizer on the integer operations, so a run
//! that reaches one stops there and says so, and each prints what the VM prints or fails in its words. The integers
//! come from text, so neither the optimizer nor gcc knows them.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test c_defined_integers_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::{compile_to_c, compile_to_c_via_bytecode, vm_outcome};

/// The build: optimized, with every signed overflow and every undefined shift a stop.
const SANITIZED: &[&str] = &[
    "-O2",
    "-fsanitize=signed-integer-overflow,shift",
    "-fno-sanitize-recover=all",
];

/// One C tier's run of a row: what it printed, and, if it failed, what it said.
struct Run {
    tier: &'static str,
    output: String,
    failure: Option<String>,
}

fn run_c(
    tier: &'static str,
    c: Result<String, logicaffeine_compile::ParseError>,
    src: &str,
) -> Run {
    let c = c.unwrap_or_else(|e| panic!("the {tier} generator refused ({e:?}):\n{src}"));
    let dir = tempfile::tempdir().expect("temp dir");
    let (c_path, bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&c_path, &c).expect("write the C");
    let built = std::process::Command::new(common::c_compiler())
        .args(SANITIZED)
        .arg("-o")
        .arg(&bin)
        .arg(&c_path)
        .arg("-lm")
        .output()
        .expect("run the C compiler");
    assert!(
        built.status.success(),
        "the {tier} does not build sanitized:\n{}\n{src}",
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

/// `src` on both C tiers answers as on the VM: the same lines, and the same sentence when it fails.
fn defined_and_alike(src: &str) {
    let vm = vm_outcome(src);
    let expected_output: String = vm.output.lines().map(|l| format!("{l}\n")).collect();
    let expected_failure = vm.error.as_ref().map(|why| format!("{why}\n"));
    let mut report = String::new();
    for run in [
        run_c("AST C", compile_to_c(src), src),
        run_c("bytecode C", compile_to_c_via_bytecode(src), src),
    ] {
        if run.output != expected_output || run.failure != expected_failure {
            report.push_str(&format!(
                "\n== {}\nprinted:\n{}failed: {:?}\n",
                run.tier, run.output, run.failure
            ));
        }
    }
    assert!(
        report.is_empty(),
        "the VM prints:\n{expected_output}and fails with {expected_failure:?} for:\n{src}{report}"
    );
}

/// A program whose Ints are read from text, then `body`.
fn over(values: &[(&str, &str)], body: &str) -> String {
    let mut src = String::from("## To native parseInt (s: Text) -> Int\n\n## Main\n");
    for (name, value) in values {
        src.push_str(&format!("Let {name} be parseInt(\"{value}\").\n"));
    }
    src.push_str(body);
    src
}

const EDGES: [&str; 5] = [
    "1",
    "-8",
    "1024",
    "9223372036854775807",
    "-9223372036854775808",
];

#[test]
fn a_shift_takes_its_count_modulo_the_word_whatever_the_count() {
    let counts = ["0", "1", "63", "64", "65", "-1", "-64", "4294967297"];
    for x in EDGES {
        let values: Vec<(String, &str)> = counts
            .iter()
            .enumerate()
            .map(|(i, k)| (format!("k{i}"), *k))
            .collect();
        let mut body = String::new();
        for (name, _) in &values {
            body.push_str(&format!(
                "Show x shifted left by {name}.\nShow x shifted right by {name}.\n"
            ));
        }
        let mut all: Vec<(&str, &str)> = vec![("x", x)];
        all.extend(values.iter().map(|(n, v)| (n.as_str(), *v)));
        defined_and_alike(&over(&all, &body));
    }
}

#[test]
fn arithmetic_at_the_edges_of_the_word_is_exact() {
    for x in EDGES {
        defined_and_alike(&over(
            &[("x", x), ("m", "-1")],
            "Show x + 1.\nShow x - 1.\nShow x * 2.\nShow 0 - x.\nShow x * m.\nShow x / m.\nShow x % m.\n\
             Show x + x > x.\nShow \"{x * 3}\".\n",
        ));
    }
}

#[test]
fn an_exact_value_at_the_bottom_of_the_word_is_the_word_where_one_is_needed() {
    defined_and_alike(&over(
        &[("a", "9223372036854775807")],
        "Let b be a + 1.\nLet c be 0 - b.\nShow c.\nLet mutable xs be a new Seq of Int.\nPush c to xs.\n\
         Show item 1 of xs.\nShow c + 1.\nShow item 1 of [c, 7].\n",
    ));
}

#[test]
fn a_shared_records_fields_merge_as_on_the_vm() {
    for (verb, a, b) in [
        ("Merge", "9223372036854775807", "1"),
        ("Merge", "-9223372036854775808", "-1"),
        ("Merge", "40", "2"),
        ("Converge", "9223372036854775807", "1"),
        ("Converge", "-5", "3"),
    ] {
        defined_and_alike(&over(
            &[("a", a), ("b", b)],
            &format!(
                "Let mutable local be a new Board.\nLet mutable remote be a new Board.\n\
                 Set local's games to a.\nSet remote's games to b.\n\
                 Set local's title to \"home\".\nSet remote's title to \"away\".\n\
                 Add 1 to local's seen.\nAdd 2 to remote's seen.\n\
                 {verb} remote into local.\n\
                 Show local's games.\nShow local's title.\nShow length of local's seen.\n"
            ),
        )
        .replace(
            "## Main\n",
            "## Definition\nA Board is Shared and has:\n    a games, which is Int.\n    a title, which is Text.\n    \
             a seen, which is Set of Int.\n\n## Main\n",
        ));
    }
}

/// A floor division reads each operand once: a call in either runs once, as on the VM.
#[test]
fn a_floor_division_reads_each_operand_once() {
    defined_and_alike(
        "## To next (x: Int) -> Int:\n    Show x.\n    Return x + 1.\n\n## Main\n\
         Show next(-8) // 2.\nShow 7 // next(-3).\nShow next(6) // next(1).\n",
    );
}

/// A product past the word divided back within it and narrowed to its low 16 bits: `a * b` is 3·2^63 before it is
/// divided, and only its exact value divides to the bits the reference keeps (AST C wrapped the product first and
/// printed 57344). The tree-walker and the VM agree on it, and both C tiers answer as they do. The Rust tier's half is
/// held by `codegen_divergence_lock`; the WebAssembly half, without the 16-bit word self-contained WebAssembly does not
/// yet have, by `exact_int_every_tier_lock::a_product_past_the_word_divided_back_within_it`.
#[test]
fn a_product_past_the_word_divided_back_within_it_keeps_its_low_word() {
    let src = "## To lowWord (a: Int) and (b: Int) and (c: Int) -> Int:\n    Return intOfWord16(word16((a * b) / c)).\n\n\
               ## Main\nShow lowWord(6917529027641081856, 4, 1125899906842624).\nShow lowWord(1000, 3, 7).\n";
    let agreed = common::run_all_tiers(src);
    assert_eq!(agreed.error, None, "{src}");
    assert_eq!(agreed.output.trim(), "24576\n428", "{src}");
    defined_and_alike(src);
}
