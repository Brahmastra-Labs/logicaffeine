//! The reader's byte-exact fast paths build exactly the store the general JSON parser builds —
//! the same expression nodes under the same ids, the same names — on every vendored export, when
//! records arrive split across reads, and on the layout edge cases a fast path must hand over.

use logicaffeine_kernel::dag::export::Reader;
use logicaffeine_kernel::dag::expr::{ExprId, Node};
use logicaffeine_kernel::dag::Kernel;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

/// Hands the reader at most `chunk` bytes per read, so that records straddle reads.
struct Trickle<'a> {
    data: &'a [u8],
    chunk: usize,
}

impl Read for Trickle<'_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = self.data.len().min(self.chunk).min(out.len());
        out[..n].copy_from_slice(&self.data[..n]);
        self.data = &self.data[n..];
        Ok(n)
    }
}

struct Parsed {
    ok: bool,
    exprs: Vec<Node>,
    names: usize,
}

fn parse(input: impl BufRead, fast: bool) -> Parsed {
    let mut k = Kernel::new();
    let mut r = Reader::new(&mut k);
    r.parse_only = true;
    r.fast_paths = fast;
    let ok = r.run(input).is_ok();
    let exprs = (0..k.s.expr_count() as u32)
        .map(|i| k.s.node(ExprId(i)))
        .collect();
    Parsed {
        ok,
        exprs,
        names: k.s.names.len(),
    }
}

fn same(label: &str, text: &[u8], chunk: Option<usize>) {
    let run = |fast| match chunk {
        None => parse(BufReader::new(text), fast),
        Some(c) => parse(
            BufReader::with_capacity(
                c.max(1),
                Trickle {
                    data: text,
                    chunk: c,
                },
            ),
            fast,
        ),
    };
    let (general, fast) = (run(false), run(true));
    assert_eq!(
        fast.ok, general.ok,
        "{label}: the fast paths change whether the export reads"
    );
    assert_eq!(fast.names, general.names, "{label}: name count");
    assert_eq!(
        fast.exprs.len(),
        general.exprs.len(),
        "{label}: expression count"
    );
    for (i, (a, b)) in fast.exprs.iter().zip(&general.exprs).enumerate() {
        assert_eq!(a, b, "{label}: expression #{i}");
    }
}

fn corpus_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "ndjson") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/arena_corpus"),
        &mut out,
    );
    out.sort();
    out
}

#[test]
fn every_vendored_export_reads_the_same_either_way() {
    let files = corpus_files();
    assert!(
        files.len() > 200,
        "the vendored corpus is present ({} files)",
        files.len()
    );
    for f in files {
        let text = std::fs::read(&f).unwrap();
        same(&f.display().to_string(), &text, None);
    }
}

#[test]
fn records_split_across_reads_read_the_same() {
    let text = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/arena_corpus/good/init-prelude.ndjson"),
    )
    .unwrap();
    for chunk in [1, 7, 31, 64, 65, 80, 81, 127, 4093] {
        same(
            &format!("init-prelude in {chunk}-byte reads"),
            &text,
            Some(chunk),
        );
    }
}

const META: &str = r#"{"meta":{"exporter":{"name":"lean4export","version":"3.1.0"},"format":{"version":"3.1.0"},"lean":{"githash":"f72c35b3f637c8c6571d353742168ab66cc22c00","version":"4.29.1"}}}"#;

/// An export whose expression ids have every length from one digit to nineteen, applications
/// between them in both argument orders, and binders, constants and sorts around them.
fn wide_ids() -> String {
    let mut s = String::from(META);
    s.push('\n');
    s.push_str(r#"{"in":1,"str":{"pre":0,"str":"c"}}"#);
    s.push('\n');
    s.push_str(r#"{"il":1,"succ":0}"#);
    s.push('\n');
    let mut ids: Vec<u64> = vec![0];
    s.push_str(r#"{"ie":0,"sort":1}"#);
    s.push('\n');
    let mut next: u64 = 1;
    for digits in 1..=19u32 {
        let base = 10u64.pow(digits - 1);
        let id = base.max(next) + digits as u64;
        let (a, f) = (ids[ids.len() / 2], *ids.last().unwrap());
        s.push_str(&format!(r#"{{"app":{{"arg":{a},"fn":{f}}},"ie":{id}}}"#));
        s.push('\n');
        ids.push(id);
        let id2 = id + 1;
        s.push_str(&format!(r#"{{"app":{{"arg":{f},"fn":{a}}},"ie":{id2}}}"#));
        s.push('\n');
        ids.push(id2);
        let id3 = id2 + 1;
        s.push_str(&format!(r#"{{"forallE":{{"binderInfo":"default","body":{id},"name":0,"type":{id2}}},"ie":{id3}}}"#));
        s.push('\n');
        ids.push(id3);
        let id4 = id3 + 1;
        s.push_str(&format!(r#"{{"const":{{"name":1,"us":[1]}},"ie":{id4}}}"#));
        s.push('\n');
        ids.push(id4);
        next = id4 + 1;
    }
    s
}

#[test]
fn ids_of_every_length_read_the_same() {
    let text = wide_ids();
    same("ids of 1..19 digits", text.as_bytes(), None);
    for chunk in [3, 50, 64, 79, 80, 81] {
        same(
            &format!("ids of 1..19 digits in {chunk}-byte reads"),
            text.as_bytes(),
            Some(chunk),
        );
    }
}

#[test]
fn layouts_a_fast_path_must_hand_over_read_the_same() {
    let cases: &[(&str, &str)] = &[
        ("leading zero", r#"{"app":{"arg":00,"fn":0},"ie":1}"#),
        ("leading zero, fn", r#"{"app":{"arg":0,"fn":01},"ie":1}"#),
        ("leading zero, ie", r#"{"app":{"arg":0,"fn":0},"ie":01}"#),
        ("spaced", r#"{"app": {"arg":0,"fn":0},"ie":1}"#),
        ("reordered", r#"{"ie":1,"app":{"arg":0,"fn":0}}"#),
        (
            "twenty digits",
            r#"{"app":{"arg":0,"fn":0},"ie":12345678901234567890}"#,
        ),
        ("negative", r#"{"app":{"arg":0,"fn":0},"ie":-1}"#),
        ("trailing space", "{\"app\":{\"arg\":0,\"fn\":0},\"ie\":1} "),
        (
            "carriage return",
            "{\"app\":{\"arg\":0,\"fn\":0},\"ie\":1}\r",
        ),
        ("eight digits", r#"{"app":{"arg":0,"fn":0},"ie":12345678}"#),
        ("nine digits", r#"{"app":{"arg":0,"fn":0},"ie":123456789}"#),
        (
            "sixteen digits",
            r#"{"app":{"arg":0,"fn":0},"ie":1234567890123456}"#,
        ),
        (
            "seventeen digits",
            r#"{"app":{"arg":0,"fn":0},"ie":12345678901234567}"#,
        ),
    ];
    for (label, line) in cases {
        let text = format!("{META}\n{{\"ie\":0,\"sort\":0}}\n{line}\n{{\"ie\":2,\"sort\":0}}\n");
        same(label, text.as_bytes(), None);
        let padded = format!(
            "{META}\n{{\"ie\":0,\"sort\":0}}\n{line}\n{}",
            "{\"ie\":2,\"sort\":0}\n".repeat(8)
        );
        same(&format!("{label} (padded)"), padded.as_bytes(), None);
    }
}

/// One line of every layout a fast path reads, each id it names defined by [`PRELUDE`].
const LAYOUTS: &[&str] = &[
    r#"{"app":{"arg":3,"fn":4},"ie":20}"#,
    r#"{"app":{"arg":12345678,"fn":4},"ie":20}"#,
    r#"{"app":{"arg":3,"fn":12345678},"ie":87654321}"#,
    r#"{"forallE":{"binderInfo":"default","body":3,"name":1,"type":4},"ie":20}"#,
    r#"{"forallE":{"binderInfo":"instImplicit","body":3,"name":1,"type":4},"ie":20}"#,
    r#"{"ie":20,"lam":{"binderInfo":"implicit","body":3,"name":1,"type":4}}"#,
    r#"{"ie":20,"lam":{"binderInfo":"strictImplicit","body":3,"name":1,"type":4}}"#,
    r#"{"ie":20,"sort":1}"#,
    r#"{"bvar":0,"ie":20}"#,
    r#"{"const":{"name":1,"us":[1]},"ie":20}"#,
    r#"{"const":{"name":1,"us":[]},"ie":20}"#,
    r#"{"in":2,"str":{"pre":1,"str":"d"}}"#,
    r#"{"in":2,"num":{"i":7,"pre":1}}"#,
];

const PRELUDE: &str = concat!(
    r#"{"in":1,"str":{"pre":0,"str":"c"}}"#,
    "\n",
    r#"{"il":1,"succ":0}"#,
    "\n",
    r#"{"ie":0,"sort":0}"#,
    "\n",
    r#"{"ie":1,"sort":1}"#,
    "\n",
    r#"{"ie":2,"sort":0}"#,
    "\n",
    r#"{"ie":3,"sort":1}"#,
    "\n",
    r#"{"ie":4,"sort":0}"#,
    "\n",
    r#"{"ie":12345678,"sort":1}"#,
    "\n",
);

/// Every byte of every fast layout matters: each one replaced (by bytes that keep the line JSON
/// and bytes that break it) or deleted, the line reads the same through the fast paths as through
/// the general parser — so no fast path skips a byte it must check, whether a key, a separator, a
/// digit or the newline. Followed by padding, so the fixed-window reads see the line too.
#[test]
fn every_byte_of_every_fast_layout_is_checked() {
    let pad = "{\"ie\":30,\"sort\":0}\n".repeat(8);
    let mut variants = 0;
    for line in LAYOUTS {
        let line = line.as_bytes();
        let mut edits: Vec<Vec<u8>> = vec![line.to_vec()];
        for i in 0..line.len() {
            for c in [b'x', b'0', b'9', b' ', b'"', b',', b'}', b':', b'\n'] {
                if line[i] != c {
                    let mut e = line.to_vec();
                    e[i] = c;
                    edits.push(e);
                }
            }
            let mut e = line.to_vec();
            e.remove(i);
            edits.push(e);
        }
        for e in edits {
            let mut text = format!("{META}\n{PRELUDE}").into_bytes();
            text.extend_from_slice(&e);
            text.push(b'\n');
            text.extend_from_slice(pad.as_bytes());
            same(&String::from_utf8_lossy(&e), &text, None);
            variants += 1;
        }
    }
    assert!(
        variants > 5000,
        "the sweep covers the layouts ({variants} variants)"
    );
}
