//! frozen_reds_lock — a RED test handed to another agent stays the test it was when it was handed over.
//!
//! A ticket's RED tests are its specification: they are written and run RED before the ticket is posted, and
//! the agent that takes the ticket makes them pass by changing the implementation, never the test (CLAUDE.md
//! rule 4). `fixtures/frozen_reds.tsv` pins each handed-over test by the SHA-256 of its normalized body, and each
//! shared harness function the tests judge through (a bring-up harness's run and comparison). Editing a frozen
//! function turns this lock red. Re-pinning (`LOGOS_UPDATE_FIXTURES=1`) changes a specification, and needs
//! Tristen's approval before it lands.
//!
//! A row is `<path under crates/logicaffeine_tests>\t<test function>\t<sha256>\t<ticket>`.

use logicaffeine_kernel::interface::lean::package::sha256_hex;
use std::path::{Path, PathBuf};

fn tests_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture() -> PathBuf {
    tests_root().join("tests/fixtures/frozen_reds.tsv")
}

/// The text of `fn name(` through its matching closing brace, if the file holds exactly one such function.
fn function_text<'a>(src: &'a str, name: &str) -> Result<&'a str, String> {
    let needle = format!("fn {name}(");
    let starts: Vec<usize> = src.match_indices(&needle).map(|(i, _)| i).collect();
    let start = match starts.as_slice() {
        [one] => *one,
        [] => return Err(format!("no `fn {name}(`")),
        many => return Err(format!("{} definitions of `fn {name}(`", many.len())),
    };
    let body = src[start..].find('{').ok_or_else(|| format!("`fn {name}` has no body"))? + start;
    let bytes = src.as_bytes();
    let mut depth = 0usize;
    let mut i = body;
    let mut in_str = false;
    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
        } else if c == b'"' {
            in_str = true;
        } else if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i = src[i..].find('\n').map_or(bytes.len(), |n| i + n);
            continue;
        } else if c == b'{' {
            depth += 1;
        } else if c == b'}' {
            depth -= 1;
            if depth == 0 {
                return Ok(&src[start..=i]);
            }
        }
        i += 1;
    }
    Err(format!("`fn {name}` never closes"))
}

/// The body as layout cannot change it: every run of whitespace removed, and a comma before a closing
/// bracket dropped, so reformatting never moves the digest and any change to a token does.
fn normalized(text: &str) -> String {
    let squeezed: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    squeezed.replace(",)", ")").replace(",]", "]").replace(",}", "}")
}

fn digest(text: &str) -> String {
    sha256_hex(normalized(text).as_bytes())
}

struct Row {
    path: String,
    test: String,
    sha: String,
    ticket: String,
}

fn rows() -> Vec<Row> {
    let text = std::fs::read_to_string(fixture()).expect("tests/fixtures/frozen_reds.tsv");
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            assert_eq!(f.len(), 4, "a frozen_reds.tsv row is path, test, sha256, ticket: {l:?}");
            Row { path: f[0].into(), test: f[1].into(), sha: f[2].into(), ticket: f[3].into() }
        })
        .collect()
}

#[test]
fn every_frozen_red_is_the_test_that_was_handed_over() {
    let rows = rows();
    if std::env::var("LOGOS_UPDATE_FIXTURES").is_ok() {
        let mut out = String::from(
            "# path under crates/logicaffeine_tests\ttest\tsha256 of the normalized body\tticket\n",
        );
        for r in &rows {
            let src = std::fs::read_to_string(tests_root().join(&r.path)).expect("read test file");
            let f = function_text(&src, &r.test).unwrap_or_else(|e| panic!("{}::{}: {e}", r.path, r.test));
            out.push_str(&format!("{}\t{}\t{}\t{}\n", r.path, r.test, digest(f), r.ticket));
        }
        std::fs::write(fixture(), out).expect("write frozen_reds.tsv");
        return;
    }
    let mut changed = Vec::new();
    for r in &rows {
        let src = std::fs::read_to_string(tests_root().join(&r.path))
            .unwrap_or_else(|e| panic!("{} ({}): {e}", r.path, r.ticket));
        match function_text(&src, &r.test) {
            Ok(f) if digest(f) == r.sha => {}
            Ok(_) => changed.push(format!("{}::{} ({}) was edited", r.path, r.test, r.ticket)),
            Err(e) => changed.push(format!("{}::{} ({}): {e}", r.path, r.test, r.ticket)),
        }
    }
    assert!(
        changed.is_empty(),
        "a handed-over RED test changed — make the implementation pass it instead; re-pinning a test changes its \
         ticket's specification and needs Tristen's approval:\n{}",
        changed.join("\n")
    );
}

#[test]
fn every_row_names_a_test_file_under_the_tests_crate_and_a_ticket() {
    for r in rows() {
        assert!(
            r.path.starts_with("tests/") || r.path.starts_with("unit/"),
            "{} is not under crates/logicaffeine_tests/tests or unit",
            r.path
        );
        assert!(Path::new(&r.path).extension().is_some_and(|e| e == "rs"), "{} is not a Rust file", r.path);
        assert!(r.sha.len() == 64 && r.sha.bytes().all(|b| b.is_ascii_hexdigit()), "{}: bad sha256", r.test);
        assert!(!r.ticket.is_empty() && r.ticket.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-'), "{}: bad ticket id {:?}", r.test, r.ticket);
    }
}

#[test]
fn the_digest_ignores_layout_and_sees_every_token() {
    let a = "fn t() {\n    assert_eq!(f(1, 2), 3);\n}";
    let reflowed = "fn t() { assert_eq!(\n        f(\n            1,\n            2,\n        ),\n        3,\n    );\n}";
    assert_eq!(digest(a), digest(reflowed), "reformatting must not move the digest");
    for edited in [
        "fn t() {\n    assert_eq!(f(1, 2), 4);\n}",
        "fn t() {\n    assert_ne!(f(1, 2), 3);\n}",
        "fn t() {\n    assert_eq!(f(2, 1), 3);\n}",
        "fn t() {\n    let _ = f(1, 2);\n}",
    ] {
        assert_ne!(digest(a), digest(edited), "an edit must move the digest:\n{edited}");
    }
    let src = "fn other() { }\nfn t() {\n    let s = \"}\"; // }\n    assert!(s.len() == 1);\n}\nfn after() {}";
    assert_eq!(
        function_text(src, "t").unwrap(),
        "fn t() {\n    let s = \"}\"; // }\n    assert!(s.len() == 1);\n}"
    );
}

/// The work-order books whose tickets a handed RED may name.
const BOOKS: [&str; 2] = ["FUSIONS_WE_NEED.md", "PREPARE_FOR_SAMBA.md"];

/// The tickets of every book's board index and whether each is done.
fn board() -> std::collections::BTreeMap<String, bool> {
    board_of(&BOOKS.map(|name| (name, std::fs::read_to_string(tests_root().join("../../benchmarks").join(name)).expect("the book"))))
}

/// The tickets of the board indexes of `books` (each its name and text) and whether each is done. A ticket id names
/// one ticket of one book.
fn board_of(books: &[(&str, String)]) -> std::collections::BTreeMap<String, bool> {
    let mut board = std::collections::BTreeMap::new();
    for (name, book) in books {
        let mut tickets = 0;
        for l in book.lines() {
            let (done, rest) = match (l.strip_prefix("- [x] **"), l.strip_prefix("- [ ] **")) {
                (Some(r), _) => (true, r),
                (_, Some(r)) => (false, r),
                _ => continue,
            };
            if let Some((id, _)) = rest.split_once("** · ") {
                assert!(board.insert(id.to_string(), done).is_none(), "{id} is a ticket of two books; {name} is the second");
                tickets += 1;
            }
        }
        assert!(tickets > 0, "{name} has no board index");
    }
    board
}

/// The test binaries staged behind `handed-reds`, each with its `#[test]` functions.
fn staged() -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(tests_root().join("tests")).expect("tests/") {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        if !text.lines().any(|l| l == "#![cfg(feature = \"handed-reds\")]") {
            continue;
        }
        let mut tests = Vec::new();
        let mut attributed = false;
        for line in text.lines() {
            if line.starts_with("#[test]") {
                attributed = true;
            } else if let Some(rest) = line.strip_prefix("fn ") {
                if attributed {
                    tests.push(rest.split('(').next().unwrap().to_string());
                }
                attributed = false;
            }
        }
        out.push((format!("tests/{}", path.file_name().unwrap().to_string_lossy()), tests));
    }
    out
}

#[test]
fn every_staged_red_is_frozen_under_an_open_ticket() {
    let board = board();
    let rows = rows();
    let staged = staged();
    assert!(!staged.is_empty(), "no RED binary is staged behind `handed-reds`");
    for (file, tests) in &staged {
        assert!(!tests.is_empty(), "{file} is staged and holds no test");
        for t in tests {
            let row = rows
                .iter()
                .find(|r| &r.path == file && &r.test == t)
                .unwrap_or_else(|| panic!("{file}::{t} is a handed-over RED with no row in frozen_reds.tsv"));
            match board.get(&row.ticket) {
                None => panic!("{file}::{t} names {}, which is not on the board", row.ticket),
                Some(true) => panic!("{} is done, so {file} must not be staged: delete its `#![cfg]` line", row.ticket),
                Some(false) => {}
            }
        }
    }
}

#[test]
fn a_ticket_id_names_one_ticket_of_one_book() {
    let first = "- [ ] **X-1** · a ticket · phase\n- [x] **Y-2** · another · phase\n".to_string();
    let second = "- [x] **Z-3** · a third · phase\n".to_string();
    assert_eq!(
        board_of(&[("first.md", first.clone()), ("second.md", second)]),
        [("X-1", false), ("Y-2", true), ("Z-3", true)].into_iter().map(|(id, done)| (id.to_string(), done)).collect(),
        "each book's tickets, open and done"
    );
    let again = "- [x] **X-1** · the same id · phase\n".to_string();
    let twice = std::panic::catch_unwind(|| board_of(&[("first.md", first), ("again.md", again)]));
    assert!(twice.is_err(), "an id two books use is refused");
    let empty = std::panic::catch_unwind(|| board_of(&[("empty.md", "no index here\n".to_string())]));
    assert!(empty.is_err(), "a book with no board index is refused");
}
