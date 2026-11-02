//! The declaration meter: the reader reports every declaration it hands to the kernel — once, in
//! export order, bracketing exactly the kernel's work on it — with its name, its kind and whether
//! it was admitted. Per-declaration costs (`logos_kernel --decl-costs`) are measured through it.

use logicaffeine_kernel::dag::export::{DeclMeter, Observe};
use logicaffeine_kernel::dag::{check_export, check_export_observed, Verdict};
use std::cell::RefCell;
use std::io::BufReader;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Debug, PartialEq)]
enum Event {
    Begin,
    End {
        name: String,
        kind: &'static str,
        admitted: bool,
    },
}

struct Log(Rc<RefCell<Vec<Event>>>);

impl DeclMeter for Log {
    fn begin(&mut self) {
        self.0.borrow_mut().push(Event::Begin);
    }

    fn end(&mut self, name: &str, kind: &'static str, admitted: bool) {
        self.0.borrow_mut().push(Event::End {
            name: name.to_string(),
            kind,
            admitted,
        });
    }
}

struct Metered {
    exit: i32,
    declarations: u64,
    events: Vec<Event>,
}

fn metered(input: Vec<u8>) -> Metered {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            let log = Rc::new(RefCell::new(Vec::new()));
            let observe = Observe {
                meter: Some(Box::new(Log(log.clone()))),
                ..Observe::default()
            };
            let (v, stats) =
                check_export_observed(BufReader::new(&input[..]), None, |_| {}, observe);
            let events = log.take();
            Metered {
                exit: v.exit_code(),
                declarations: stats.declarations,
                events,
            }
        })
        .unwrap()
        .join()
        .unwrap()
}

fn corpus(path: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/arena_corpus")
            .join(path),
    )
    .unwrap()
}

fn ends(m: &Metered) -> Vec<(&str, &'static str, bool)> {
    m.events
        .iter()
        .filter_map(|e| match e {
            Event::End {
                name,
                kind,
                admitted,
            } => Some((name.as_str(), *kind, *admitted)),
            Event::Begin => None,
        })
        .collect()
}

fn assert_bracketed(m: &Metered) {
    for (i, e) in m.events.iter().enumerate() {
        let expect_begin = i % 2 == 0;
        assert_eq!(
            matches!(e, Event::Begin),
            expect_begin,
            "event {i} breaks begin/end alternation: {e:?}"
        );
    }
    assert_eq!(
        m.events.len() % 2,
        0,
        "a declaration was begun and never ended"
    );
}

fn records_of_kind(text: &[u8], kind: &str) -> usize {
    let prefix = format!("{{\"{kind}\":");
    text.split(|b| *b == b'\n')
        .filter(|l| l.starts_with(prefix.as_bytes()))
        .count()
}

#[test]
fn every_declaration_of_an_accepted_export_is_bracketed_once_and_admitted() {
    let text = corpus("good/init-prelude.ndjson");
    let m = metered(text.clone());
    assert_eq!(m.exit, 0);
    assert_bracketed(&m);
    let ends = ends(&m);
    assert_eq!(
        ends.len() as u64,
        m.declarations,
        "one report per declaration the reader counted"
    );
    assert!(
        ends.iter().all(|(_, _, admitted)| *admitted),
        "an accepted export admits every declaration"
    );
    for kind in ["thm", "def", "axiom", "opaque", "quot", "inductive"] {
        let reported = ends.iter().filter(|(_, k, _)| *k == kind).count();
        assert_eq!(
            reported,
            records_of_kind(&text, kind),
            "reports of kind {kind} against the export's records"
        );
    }
}

#[test]
fn declarations_are_reported_in_export_order_by_name() {
    let m = metered(corpus("good/init-prelude.ndjson"));
    let ends = ends(&m);
    assert_eq!(
        ends[0],
        ("Nat", "inductive", true),
        "the prelude opens with the Nat block"
    );
    let names: Vec<&str> = ends.iter().map(|(n, _, _)| *n).collect();
    let nat_add = names
        .iter()
        .position(|n| *n == "Nat.add")
        .expect("Nat.add is reported");
    let nat_mul = names
        .iter()
        .position(|n| *n == "Nat.mul")
        .expect("Nat.mul is reported");
    assert!(
        nat_add < nat_mul,
        "Nat.mul is defined through Nat.add, so it comes later"
    );
}

#[test]
fn a_rejected_declaration_is_reported_as_not_admitted_and_ends_the_run() {
    let m = metered(corpus("bad/tutorial/002_badDef.ndjson"));
    assert_eq!(m.exit, 1);
    assert_eq!(
        m.events,
        vec![
            Event::Begin,
            Event::End {
                name: "badDef".into(),
                kind: "def",
                admitted: false
            }
        ]
    );
}

const MUTUAL_UNSAFE: &str = concat!(
    r#"{"meta":{"exporter":{"name":"lean4export","version":"3.1.0"},"format":{"version":"3.1.0"},"lean":{"githash":"f72c35b3f637c8c6571d353742168ab66cc22c00","version":"4.29.1"}}}"#,
    "\n",
    r#"{"in":1,"str":{"pre":0,"str":"evenish"}}"#,
    "\n",
    r#"{"in":2,"str":{"pre":0,"str":"oddish"}}"#,
    "\n",
    r#"{"il":1,"succ":0}"#,
    "\n",
    r#"{"ie":0,"sort":1}"#,
    "\n",
    r#"{"ie":1,"sort":0}"#,
    "\n",
    r#"{"def":{"all":[1,2],"hints":"opaque","levelParams":[],"name":1,"safety":"unsafe","type":0,"value":1}}"#,
    "\n",
    r#"{"def":{"all":[1,2],"hints":"opaque","levelParams":[],"name":2,"safety":"unsafe","type":0,"value":1}}"#,
    "\n",
);

#[test]
fn a_mutual_block_is_one_report_named_by_its_first_member() {
    let m = metered(MUTUAL_UNSAFE.as_bytes().to_vec());
    assert_eq!(m.exit, 0);
    assert_eq!(m.declarations, 1);
    assert_eq!(
        m.events,
        vec![
            Event::Begin,
            Event::End {
                name: "evenish".into(),
                kind: "mutual",
                admitted: true
            }
        ]
    );
}

#[test]
fn metering_does_not_change_any_verdict() {
    for case in [
        "good/init-prelude.ndjson",
        "bad/tutorial/002_badDef.ndjson",
        "bad/tutorial/140_falseFromUnsafe.ndjson",
        "bad/tutorial/141_falseFromPartial.ndjson",
    ] {
        let text = corpus(case);
        let plain = {
            let t = text.clone();
            std::thread::Builder::new()
                .stack_size(1 << 30)
                .spawn(move || check_export(BufReader::new(&t[..])).0.exit_code())
                .unwrap()
                .join()
                .unwrap()
        };
        assert_eq!(
            metered(text).exit,
            plain,
            "{case}: metered and unmetered verdicts differ"
        );
    }
}

#[test]
fn an_unmetered_run_reports_nothing_and_checks_the_same() {
    let text = corpus("good/init-prelude.ndjson");
    let v = std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            check_export_observed(BufReader::new(&text[..]), None, |_| {}, Observe::default()).0
        })
        .unwrap()
        .join()
        .unwrap();
    assert!(matches!(v, Verdict::Accept));
}
