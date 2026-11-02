//! ONE DOCUMENT DOOR — every Studio math example, read by `Repl::execute_document`.
//!
//! The Studio ships 28 math examples (`ALL_MATH_EXAMPLES` in
//! `apps/logicaffeine_web/src/ui/examples.rs`) and locks them through its own line splitter and a
//! per-statement `Repl::execute`. The kernel's door for a whole document is
//! [`Repl::execute_document`], and it is the door the Lean emitter writes for (ruling 2026-09-22),
//! so a document the Studio accepts and this door refuses is two readings of one text. This file
//! reads the Studio's registry and sources from the web crate's source file itself — nothing is
//! copied, so an example edited there is the example checked here — and holds every one of them
//! to the door, by the expectation the registry states:
//!
//! * `KernelAllStatementsOk` — the whole document is accepted;
//! * `KernelAdmitsAt(marker)` — everything before the block declaring `marker` is accepted, and
//!   the document is refused AT that block, by the termination checker, naming `marker`;
//! * `TheoryAllVerified` — a `## Theory` development, which the theory driver certifies; the
//!   kernel door refuses it by name and says where it goes, rather than as an unknown command.

use logicaffeine_kernel::interface::{InterfaceError, ParseError, Repl};
use logicaffeine_kernel::KernelError;
use std::path::PathBuf;

const EXAMPLES: &str = "apps/logicaffeine_web/src/ui/examples.rs";

#[derive(Debug, Clone, PartialEq, Eq)]
enum Expected {
    AllStatementsOk,
    AdmitsAt(String),
    TheoryAllVerified,
}

#[derive(Debug, Clone)]
struct Spec {
    path: String,
    constant: String,
    expected: Expected,
    source: String,
}

fn examples_file() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(EXAMPLES);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// The raw string constant `name` — `const` or `pub const`, `r#"…"#` or `r###"…"###` — exactly as
/// the compiler would read it. Exactly one declaration of the name may exist.
fn raw_constant(file: &str, name: &str) -> String {
    let declarations: Vec<usize> = [
        format!("\nconst {name}: &str = "),
        format!("\npub const {name}: &str = "),
    ]
    .iter()
    .flat_map(|prefix| {
        file.match_indices(prefix.as_str())
            .map(|(at, p)| at + p.len())
            .collect::<Vec<_>>()
    })
    .collect();
    assert_eq!(
        declarations.len(),
        1,
        "`{name}` must be declared exactly once in {EXAMPLES}"
    );
    let rest = &file[declarations[0]..];
    let rest = rest
        .strip_prefix('r')
        .unwrap_or_else(|| panic!("`{name}` is not a raw string"));
    let hashes = rest.chars().take_while(|c| *c == '#').count();
    let body = rest[hashes..]
        .strip_prefix('"')
        .unwrap_or_else(|| panic!("`{name}` has no opening quote"));
    let close = format!("\"{}", "#".repeat(hashes));
    let end = body
        .find(&close)
        .unwrap_or_else(|| panic!("`{name}` is never closed"));
    body[..end].to_string()
}

fn field<'a>(entry: &'a str, key: &str, stop: char) -> &'a str {
    let start = entry
        .find(key)
        .unwrap_or_else(|| panic!("no `{key}` in {entry}"))
        + key.len();
    let rest = &entry[start..];
    &rest[..rest.find(stop).unwrap_or(rest.len())]
}

/// The math registry, entry by entry, with each entry's source resolved.
fn registry() -> Vec<Spec> {
    let file = examples_file();
    let start = file
        .find("pub const ALL_MATH_EXAMPLES")
        .expect("the math registry");
    let block = &file[start..];
    let block = &block[..block.find("\n];").expect("the math registry closes")];
    block
        .split("ExampleSpec {")
        .skip(1)
        .map(|entry| {
            let path = field(entry, "vfs_path: \"", '"').to_string();
            let constant = field(entry, "source: ", ',').trim().to_string();
            let expected = match field(entry, "expected: ", '}')
                .trim()
                .trim_end_matches(',')
                .trim_end()
            {
                "Expected::KernelAllStatementsOk" => Expected::AllStatementsOk,
                "Expected::TheoryAllVerified" => Expected::TheoryAllVerified,
                other => match other
                    .strip_prefix("Expected::KernelAdmitsAt(\"")
                    .and_then(|m| m.strip_suffix("\")"))
                {
                    Some(marker) => Expected::AdmitsAt(marker.to_string()),
                    None => panic!("a math expectation this lock does not know: {other} ({path})"),
                },
            };
            let source = raw_constant(&file, &constant);
            Spec {
                path,
                constant,
                expected,
                source,
            }
        })
        .collect()
}

fn spec(constant: &str) -> Spec {
    registry()
        .into_iter()
        .find(|s| s.constant == constant)
        .unwrap_or_else(|| panic!("`{constant}` is not a registered math example"))
}

/// The byte offset of the line that DECLARES `marker`: the first line that is not prose and names
/// it.
fn declaring_line(source: &str, marker: &str) -> usize {
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("--") && trimmed.contains(marker) {
            return offset;
        }
        offset += line.len();
    }
    panic!("no line declares `{marker}`")
}

fn run(source: &str) -> Result<Vec<String>, InterfaceError> {
    Repl::new().execute_document(source)
}

/// The reader itself. A lock whose registry came back empty would pass every loop below without
/// checking anything, so the registry's shape is pinned first.
#[test]
fn the_registry_is_read_whole() {
    let specs = registry();
    assert_eq!(
        specs.len(),
        28,
        "the Studio registers 28 math examples: {:#?}",
        specs.iter().map(|s| &s.path).collect::<Vec<_>>()
    );
    for s in &specs {
        assert!(
            s.path.starts_with("/examples/math/") && s.path.ends_with(".logos"),
            "{}",
            s.path
        );
        assert!(
            s.source.trim().len() > 20,
            "`{}` read as {:?}",
            s.constant,
            s.source
        );
    }
    let count = |e: &dyn Fn(&Expected) -> bool| specs.iter().filter(|s| e(&s.expected)).count();
    assert_eq!(count(&|e| *e == Expected::AllStatementsOk), 26);
    assert_eq!(count(&|e| *e == Expected::TheoryAllVerified), 1);
    assert_eq!(
        count(&|e| matches!(e, Expected::AdmitsAt(m) if m == "collatz_len")),
        1
    );
    // The reader keeps a raw string's text byte for byte — the `r###` sources hold `#` and `"`.
    let collatz = spec("MATH_COLLATZ").source;
    assert!(
        collatz.starts_with("-- ====") && collatz.ends_with("connected.\n"),
        "{collatz:?}"
    );
    assert!(spec("MATH_PROPERTY")
        .source
        .contains("Definition add : Num -> Num -> Num"));
}

/// Every example the Studio runs through the kernel is accepted by the one document door.
#[test]
fn every_kernel_math_example_is_accepted_by_the_document_door() {
    let mut failures = Vec::new();
    for s in registry()
        .into_iter()
        .filter(|s| s.expected == Expected::AllStatementsOk)
    {
        if let Err(e) = run(&s.source) {
            failures.push(format!("  {} ({}): {e}", s.path, s.constant));
        }
    }
    assert!(
        failures.is_empty(),
        "{} math example(s) refused by `execute_document`:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The examples this lock was written for, one by one, each with what it demonstrates still true
/// after it ran.
#[test]
fn the_four_examples_the_gate_and_the_parser_broke_each_run_whole() {
    // A user `add` over their own `Num`, in a document: it shadows the prelude's `add`.
    let property = spec("MATH_PROPERTY");
    let mut repl = Repl::new();
    let out = repl
        .execute_document(&property.source)
        .unwrap_or_else(|e| panic!("{}: {e}", property.path));
    assert_eq!(
        out.last().map(String::as_str),
        Some("(S (S Z))"),
        "`Eval add one one` computes with the user's `add`"
    );
    assert_eq!(
        repl.execute("Check add.").unwrap(),
        "add : Num -> Num -> Num"
    );
    assert_eq!(
        repl.execute("Check _root_.add.").unwrap(),
        "_root_.add : Int -> Int -> Int"
    );

    // A literate `add` and `mul` over `Nat`, nested `Consider`s written on their own lines.
    let collatz = spec("MATH_COLLATZ");
    let mut repl = Repl::new();
    repl.execute_document(&collatz.source)
        .unwrap_or_else(|e| panic!("{}: {e}", collatz.path));
    assert_eq!(
        repl.execute("Check add.").unwrap(),
        "add : Nat -> Nat -> Nat"
    );
    assert_eq!(
        repl.execute("Check mul.").unwrap(),
        "mul : Nat -> Nat -> Nat"
    );
    assert_eq!(
        repl.execute("Eval halve (Succ (Succ (Succ (Succ Zero)))).")
            .unwrap(),
        "(Succ (Succ Zero))"
    );
    assert_eq!(
        repl.execute("Eval ten.").unwrap(),
        repl.execute("Eval add six four.").unwrap()
    );

    // `there exists a d: Derivation such that …` binds `d` at `Derivation`.
    for constant in ["MATH_GODEL_LITERATE", "MATH_INCOMPLETENESS_LITERATE"] {
        let s = spec(constant);
        let mut repl = Repl::new();
        repl.execute_document(&s.source)
            .unwrap_or_else(|e| panic!("{}: {e}", s.path));
        assert_eq!(
            repl.execute("Check Provable.").unwrap(),
            "Provable : Syntax -> Prop",
            "{}",
            s.path
        );
    }
}

/// The termination wall is refused where it says it is: everything before `collatz_len` is
/// accepted, and the document stops at `collatz_len` with the termination checker's refusal.
#[test]
fn the_termination_wall_is_refused_exactly_at_its_marker() {
    let walls: Vec<Spec> = registry()
        .into_iter()
        .filter(|s| matches!(s.expected, Expected::AdmitsAt(_)))
        .collect();
    assert!(!walls.is_empty());
    for s in walls {
        let Expected::AdmitsAt(marker) = &s.expected else {
            unreachable!()
        };
        let at = declaring_line(&s.source, marker);
        run(&s.source[..at]).unwrap_or_else(|e| {
            panic!(
                "{}: everything before `{marker}` must be accepted: {e}",
                s.path
            )
        });
        match run(&s.source) {
            Err(InterfaceError::Kernel(KernelError::TerminationViolation { fix_name, .. })) => {
                assert_eq!(fix_name, *marker, "{}", s.path)
            }
            other => panic!(
                "{}: the document must stop at `{marker}` with a termination refusal: {other:?}",
                s.path
            ),
        }
    }
}

/// A `## Theory` development is the theory driver's document. The kernel door says so — naming the
/// development and the driver — instead of reporting `##` as an unknown command.
#[test]
fn a_theory_development_is_refused_by_name_and_sent_to_the_theory_driver() {
    let theories: Vec<Spec> = registry()
        .into_iter()
        .filter(|s| s.expected == Expected::TheoryAllVerified)
        .collect();
    assert!(!theories.is_empty());
    for s in theories {
        match run(&s.source) {
            Err(InterfaceError::Parse(ParseError::Unsupported { what, why })) => {
                assert!(what.contains("## Theory Tarski"), "{}: {what}", s.path);
                assert!(
                    why.contains("theory driver") && why.contains("compile_theory"),
                    "{}: {why}",
                    s.path
                );
            }
            other => panic!(
                "{}: a `## Theory` development is refused by name: {other:?}",
                s.path
            ),
        }
    }
}
