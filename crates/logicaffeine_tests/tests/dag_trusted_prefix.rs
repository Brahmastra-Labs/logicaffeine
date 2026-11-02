//! A trusted prefix: the declarations before an index are admitted as the export states them,
//! unchecked, so that one declaration deep in a large export can be measured without re-checking
//! everything before it. Such a run claims nothing — every outcome is a decline, a failure among
//! the checked declarations included — and the primitives, quotients and primitive inductive
//! blocks are checked regardless, so the validated literal operations match a full run.

use logicaffeine_kernel::dag::export::{DeclMeter, Observe};
use logicaffeine_kernel::dag::{check_export_observed, Verdict};
use std::cell::RefCell;
use std::io::BufReader;
use std::path::PathBuf;
use std::rc::Rc;

fn corpus(path: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/arena_corpus")
            .join(path),
    )
    .unwrap()
}

/// The declarations an export states, counted from its records without the kernel: one per `axiom`,
/// `def`, `thm`, `opaque` and `quot` record, and one per `inductive` block.
fn declarations(text: &[u8]) -> usize {
    const KINDS: [&str; 6] = ["axiom", "def", "thm", "opaque", "quot", "inductive"];
    text.split(|&b| b == b'\n')
        .filter(|line| !line.is_empty())
        .filter(|line| {
            let record: serde_json::Value = serde_json::from_slice(line).unwrap();
            KINDS.iter().any(|k| record.get(*k).is_some())
        })
        .count()
}

struct Names(Rc<RefCell<Vec<String>>>);

impl DeclMeter for Names {
    fn begin(&mut self) {}

    fn end(&mut self, name: &str, _kind: &'static str, _admitted: bool) {
        self.0.borrow_mut().push(name.to_string());
    }
}

struct Run {
    verdict: Verdict,
    checked: Vec<String>,
    primitives: Vec<String>,
}

fn run(text: Vec<u8>, trust_before: u64) -> Run {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            let log = Rc::new(RefCell::new(Vec::new()));
            let observe = Observe {
                meter: Some(Box::new(Names(log.clone()))),
                trust_before,
            };
            let (verdict, stats) =
                check_export_observed(BufReader::new(&text[..]), None, |_| {}, observe);
            Run {
                verdict,
                checked: log.take(),
                primitives: stats.validated_primitives,
            }
        })
        .unwrap()
        .join()
        .unwrap()
}

fn declines_with(r: &Run, needle: &str) {
    match &r.verdict {
        Verdict::Decline(e) => assert!(
            e.message.contains(needle),
            "decline says {:?}, expected {needle:?}",
            e.message
        ),
        other => panic!(
            "a trusted-prefix run must decline, got exit {}",
            other.exit_code()
        ),
    }
}

#[test]
fn an_untrusted_run_is_an_ordinary_check() {
    let text = corpus("good/init-prelude.ndjson");
    let stated = declarations(&text);
    let r = run(text, 0);
    assert!(matches!(r.verdict, Verdict::Accept));
    assert_eq!(
        r.checked.len(),
        stated,
        "every declaration the export states is checked, once"
    );
}

#[test]
fn a_trusted_prefix_is_admitted_unchecked_and_the_run_declines() {
    let r = run(corpus("bad/tutorial/002_badDef.ndjson"), 1);
    declines_with(&r, "trusted");
    assert!(
        r.checked.is_empty(),
        "a trusted declaration is not checked, so not metered: {:?}",
        r.checked
    );
}

#[test]
fn a_passing_suffix_still_declines() {
    let r = run(corpus("good/init-prelude.ndjson"), 1000);
    declines_with(&r, "passed");
    assert!(!r.checked.is_empty());
}

#[test]
fn a_failure_after_the_prefix_is_reported_but_declined() {
    let r = run(corpus("bad/tutorial/046_inductBadNonSort.ndjson"), 1);
    declines_with(&r, "inductBadNonSort");
    assert_eq!(
        r.checked,
        vec!["inductBadNonSort".to_string()],
        "only the declaration after the prefix is checked"
    );
}

#[test]
fn primitives_are_validated_even_inside_the_trusted_prefix() {
    let everything = declarations(&corpus("good/init-prelude.ndjson")) as u64;
    let full = run(corpus("good/init-prelude.ndjson"), 0);
    let trusted = run(corpus("good/init-prelude.ndjson"), everything);
    assert!(!full.primitives.is_empty());
    assert_eq!(
        trusted.primitives, full.primitives,
        "the literal operations validated must not depend on trust"
    );
    assert!(
        trusted.checked.iter().any(|n| n == "Nat.add"),
        "a primitive is checked though trusted: {:?}",
        trusted.checked
    );
    assert!(
        trusted.checked.iter().all(|n| n != "Nat.le"),
        "an ordinary declaration inside the prefix is not checked"
    );
}
