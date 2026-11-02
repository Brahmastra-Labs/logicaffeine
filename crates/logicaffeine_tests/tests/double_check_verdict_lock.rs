//! `DoubleCheck::Agreed` is two verdicts under one name: both kernels ACCEPTED at definitionally
//! equal types, or both REJECTED. An assertion of `Agreed` alone therefore passes whichever way
//! the kernels went, and a test that meant "both accept" stays green when both reject for an
//! unrelated reason — a name typed as `Global` that only a `Const` at its levels can type, a
//! name the environment never declared, an ill-typed citation, an unbound variable.
//!
//! The rule: a test function that asserts `DoubleCheck::Agreed` also states the verdict of one
//! kernel on its own — an `infer_type`, `recheck` or `lean_type` call whose statement reads the
//! result as a verdict (`.is_ok()`, `.is_err()`, `.unwrap()`, `.expect(…)`, or a `match`,
//! `matches!` or `let … else` on `Ok(…)`/`Err(…)`). Agreement plus one side's verdict is the
//! verdict of both.
//!
//! The functions that do not are frozen in `goldens/double_check_agreed_debt.txt`: no function may
//! join it, and a row whose function now states its verdict (or is gone) must leave it, so the
//! debt only shrinks.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::PathBuf;

const DEBT: &str = "goldens/double_check_agreed_debt.txt";
const SELF: &str = "tests/double_check_verdict_lock.rs";

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` under `tests/` and `unit/`, as (crate-relative path, text).
fn sources() -> Vec<(String, String)> {
    let root = crate_dir();
    let mut out = Vec::new();
    let mut stack = vec![root.join("tests"), root.join("unit")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                let rel = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, std::fs::read_to_string(&path).unwrap_or_default()));
            }
        }
    }
    out.sort();
    out
}

/// `(name, start, end)` of every braced `fn` body in [`code_only`] text; `end` is one past the
/// closing brace.
fn functions(text: &str) -> Vec<(String, usize, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(found) = text[at..].find("fn ") {
        let start = at + found;
        at = start + 3;
        if start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
            continue;
        }
        let name: String = text[at..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        let Some(open) = body_open(text, at) else {
            continue;
        };
        let Some(close) = matching_close(bytes, open) else {
            continue;
        };
        out.push((name, start, close));
    }
    out
}

/// The `{` opening a function body, or `None` for a declaration ending in `;`.
fn body_open(text: &str, from: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (offset, c) in text[from..].char_indices() {
        match c {
            '(' | '<' | '[' => depth += 1,
            ')' | '>' | ']' => depth -= 1,
            '{' if depth <= 0 => return Some(from + offset),
            ';' if depth <= 0 => return None,
            _ => {}
        }
    }
    None
}

/// The `}` closing the `{` at `open`, on text already passed through [`code_only`].
fn matching_close(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// `text` with every comment and string/char literal body blanked to spaces, byte offsets kept.
fn code_only(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = bytes.to_vec();
    let mut i = 0;
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for b in &mut out[from..to] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
    };
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                let start = i;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                blank(&mut out, start, i);
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                let start = i;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
                blank(&mut out, start, i);
                continue;
            }
            b'r' if (bytes.get(i + 1) == Some(&b'#') || bytes.get(i + 1) == Some(&b'"'))
                && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_')) =>
            {
                let hashes = bytes[i + 1..].iter().take_while(|b| **b == b'#').count();
                if bytes.get(i + 1 + hashes) != Some(&b'"') {
                    i += 1;
                    continue;
                }
                let start = i + 2 + hashes;
                let mut j = start;
                let closing = |j: usize| {
                    bytes[j] == b'"'
                        && bytes[j + 1..]
                            .iter()
                            .take(hashes)
                            .filter(|b| **b == b'#')
                            .count()
                            == hashes
                };
                while j < bytes.len() && !closing(j) {
                    j += 1;
                }
                blank(&mut out, start, j.min(bytes.len()));
                i = (j + 1 + hashes).min(bytes.len());
                continue;
            }
            b'"' => {
                let start = i + 1;
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                blank(&mut out, start, i.min(bytes.len()));
            }
            b'\'' if bytes.get(i + 2) == Some(&b'\'') => {
                blank(&mut out, i + 1, i + 2);
                i += 2;
            }
            b'\'' if bytes.get(i + 1) == Some(&b'\\') && bytes.get(i + 3) == Some(&b'\'') => {
                blank(&mut out, i + 1, i + 3);
                i += 3;
            }
            _ => {}
        }
        i += 1;
    }
    String::from_utf8(out).expect("blanking keeps UTF-8 boundaries of ASCII delimiters")
}

/// Whether `body` (already [`code_only`]) states one kernel's verdict on its own: some statement
/// holding an `infer_type`, `recheck` or `lean_type` call also reads its result as a verdict —
/// `.is_ok()`, `.unwrap()`, a `match`/`matches!`/`let … else` on `Ok(…)`/`Err(…)`, and so on.
fn states_a_verdict(body: &str) -> bool {
    const CHECKERS: [&str; 3] = ["infer_type(", "recheck(", "lean_type("];
    const VERDICTS: [&str; 10] = [
        ".is_ok()",
        ".is_err()",
        ".is_ok_and(",
        ".is_err_and(",
        ".unwrap()",
        ".expect(",
        ".unwrap_err()",
        ".expect_err(",
        "Ok(",
        "Err(",
    ];
    let bytes = body.as_bytes();
    CHECKERS.iter().any(|checker| {
        body.match_indices(checker).any(|(at, _)| {
            if at > 0 && (bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_') {
                return false;
            }
            let start = body[..at].rfind([';', '{', '}']).map_or(0, |i| i + 1);
            let end = body[at..].find(';').map_or(body.len(), |i| at + i);
            let statement = &body[start..end];
            VERDICTS.iter().any(|verdict| statement.contains(verdict))
        })
    })
}

/// `path::function` for every function that asserts `DoubleCheck::Agreed` without stating a
/// verdict, innermost function per occurrence.
fn silent_agreements(sources: &[(String, String)]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (path, text) in sources {
        if path == SELF {
            continue;
        }
        let code = code_only(text);
        let fns = functions(&code);
        for (at, _) in code.match_indices("DoubleCheck::Agreed") {
            let Some((name, start, end)) = fns
                .iter()
                .filter(|(_, s, e)| *s <= at && at < *e)
                .min_by_key(|(_, s, e)| e - s)
            else {
                continue;
            };
            if !states_a_verdict(&code[*start..*end]) {
                out.insert(format!("{path}::{name}"));
            }
        }
    }
    out
}

fn debt() -> BTreeSet<String> {
    let text = std::fs::read_to_string(crate_dir().join("tests").join(DEBT))
        .expect("the Agreed debt ledger");
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

#[test]
fn every_asserted_agreement_states_which_verdict_both_kernels_reached() {
    let silent = silent_agreements(&sources());
    let debt = debt();
    let joined: Vec<_> = silent.difference(&debt).collect();
    let paid: Vec<_> = debt.difference(&silent).collect();
    assert!(joined.is_empty(),
        "these functions assert DoubleCheck::Agreed without stating one kernel's verdict — Agreed alone \
         also means BOTH REJECTED; add `infer_type(..).is_ok()` (or `.is_err()`) for the same term: {joined:#?}");
    assert!(
        paid.is_empty(),
        "these debt rows now state their verdict or are gone; remove them from {DEBT}: {paid:#?}"
    );
}

#[test]
fn the_debt_ledger_is_sorted_and_names_real_files() {
    let text = std::fs::read_to_string(crate_dir().join("tests").join(DEBT)).unwrap();
    let rows: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    let mut sorted = rows.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(rows, sorted, "{DEBT} is sorted and duplicate-free");
    for row in rows {
        let (path, name) = row
            .rsplit_once("::")
            .unwrap_or_else(|| panic!("row {row:?} is path::function"));
        let text = std::fs::read_to_string(crate_dir().join(path))
            .unwrap_or_else(|_| panic!("{path} exists"));
        assert!(
            functions(&code_only(&text))
                .iter()
                .any(|(n, _, _)| n == name),
            "{path} defines fn {name}"
        );
    }
}

/// The scanner's own contract, on literals: a silent agreement is found, a stated verdict of
/// either direction clears it, and a verdict inside a string does not.
#[test]
fn the_scanner_finds_a_silent_agreement_and_clears_a_stated_one() {
    let file = |body: &str| {
        vec![(
            "tests/x.rs".to_string(),
            format!("#[test]\nfn t() {{\n{body}\n}}\n"),
        )]
    };
    let silent = "assert_eq!(double_check(&c, &t), DoubleCheck::Agreed);";
    assert_eq!(
        silent_agreements(&file(silent)),
        BTreeSet::from(["tests/x.rs::t".to_string()])
    );
    for stated in [
        "assert!(infer_type(&c, &t).is_ok());",
        "assert!(recheck(&c, &t).is_err(), \"refused\");",
        "let ty = infer_type(&c, &t).expect(\"typed\");",
        "assert!(matches!(recheck(&c, &t), Err(ReCheckError::Ill(_))));",
        "let ty = match infer_type(&c, &t) { Ok(ty) => ty, Err(e) => panic!(\"{e:?}\") };",
        "let Ok(ty) = lean_type(&c, &t) else { panic!() };",
    ] {
        assert!(
            silent_agreements(&file(&format!("{stated}\n{silent}"))).is_empty(),
            "{stated}"
        );
    }
    let quoted = "let m = \"infer_type(&c, &t).is_ok()\";";
    assert_eq!(
        silent_agreements(&file(&format!("{quoted}\n{silent}"))).len(),
        1,
        "a verdict spelled inside a string literal is not a verdict"
    );
    let unread = "let ty = infer_type(&c, &t);";
    assert_eq!(
        silent_agreements(&file(&format!("{unread}\n{silent}"))).len(),
        1,
        "a checker call whose result is never read as a verdict states nothing"
    );
}
