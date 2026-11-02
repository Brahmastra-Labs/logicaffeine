//! samba_book_lock — `benchmarks/PREPARE_FOR_SAMBA.md` holds together.
//!
//! The book is the work order for model bring-up on RDU-class spatial chips, and agents take tickets from it, so its
//! derived parts are checked, never kept by hand:
//! - every ticket has one board index line that agrees with its body, and every body carries the whole template;
//! - every ticket is in one stream the streams table defines, every stream holds a ticket, and no file is owned by
//!   two streams;
//! - every dependency is a ticket on this board or a `FUSIONS:` ticket on the fusion book's board, and the
//!   dependencies never close a cycle;
//! - every exam test names the lock that holds it and a ticket on the board, and every exam family holds three tests;
//! - every written exam test is where its row says: a ticked row's in a suite the baseline runs, an open row's behind
//!   the handed-reds gate, and every test of an exam suite is some row's;
//! - every Copy bar cites a bibliography entry, and every bibliography entry is cited;
//! - every repository path the book cites exists;
//! - the preface's counts are the book's own.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test samba_book_lock`

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The book; `SAMBA_BOOK` points a mutation run at a mutated copy.
fn book() -> String {
    let path = std::env::var("SAMBA_BOOK")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo().join("benchmarks/PREPARE_FOR_SAMBA.md"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The fusion book, whose board the `FUSIONS:` references name.
fn fusions() -> String {
    let path = repo().join("benchmarks/FUSIONS_WE_NEED.md");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The text after the heading line that starts with `start`, up to the next heading of the same or a higher level.
fn section<'a>(text: &'a str, start: &str) -> &'a str {
    let at = text
        .match_indices(start)
        .map(|(i, _)| i)
        .find(|&i| i == 0 || text.as_bytes()[i - 1] == b'\n')
        .unwrap_or_else(|| panic!("no heading {start:?}"));
    let level = start.chars().take_while(|c| *c == '#').count();
    let rest = &text[at + start.len()..];
    let end = rest
        .match_indices('\n')
        .map(|(i, _)| i + 1)
        .find(|&i| {
            let line = &rest[i..];
            let hashes = line.chars().take_while(|c| *c == '#').count();
            hashes > 0 && hashes <= level && line[hashes..].starts_with(' ')
        })
        .unwrap_or(rest.len());
    &rest[..end]
}

/// The lines of `text` outside fenced blocks.
fn prose_lines(text: &str) -> Vec<&str> {
    let mut fenced = false;
    text.lines()
        .filter(|l| {
            if l.trim_start().starts_with("```") {
                fenced = !fenced;
                return false;
            }
            !fenced
        })
        .collect()
}

fn is_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && s.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
}

/// One line of a board index: `- [ ] **ID** · title · phase · labels · depends on: deps`.
#[derive(Debug)]
struct IndexLine {
    id: String,
    title: String,
    phase: String,
    labels: Vec<String>,
    depends: Vec<String>,
}

fn list(text: &str) -> Vec<String> {
    if text.trim() == "none" {
        return Vec::new();
    }
    text.split(',').map(|s| s.trim().to_string()).collect()
}

fn index_lines_in(index: &str) -> Vec<IndexLine> {
    index
        .lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("- [ ] **").or_else(|| l.strip_prefix("- [x] **"))?;
            let (id, rest) = rest.split_once("** · ")?;
            let parts: Vec<&str> = rest.split(" · ").collect();
            assert_eq!(parts.len(), 4, "an index line is title · phase · labels · depends on: {l}");
            let depends = parts[3]
                .strip_prefix("depends on: ")
                .unwrap_or_else(|| panic!("no `depends on:` in {l}"));
            Some(IndexLine {
                id: id.to_string(),
                title: parts[0].to_string(),
                phase: parts[1].to_string(),
                labels: parts[2].split_whitespace().map(|s| s.trim_matches('`').to_string()).collect(),
                depends: list(depends),
            })
        })
        .collect()
}

fn index_lines(book: &str) -> Vec<IndexLine> {
    index_lines_in(section(book, "### 11.1 Board index"))
}

/// The ids on the fusion book's board.
fn fusions_board() -> BTreeSet<String> {
    let fusions = fusions();
    let index = section(&fusions, "### 12.1 Board index");
    let mut ids: BTreeSet<String> = index
        .lines()
        .filter_map(|l| l.strip_prefix("- [ ] **").or_else(|| l.strip_prefix("- [x] **")))
        .filter_map(|r| r.split_once("**").map(|(id, _)| id.to_string()))
        .collect();
    ids.extend(
        section(&fusions, "## 12. The board")
            .lines()
            .filter_map(|l| l.strip_prefix("#### "))
            .filter_map(|r| r.split_once(" · ").map(|(id, _)| id.to_string()))
            .filter(|id| is_id(id)),
    );
    ids
}

/// A ticket body: `#### ID · title`, its field table and its text up to the next heading.
#[derive(Debug)]
struct Body {
    id: String,
    title: String,
    fields: BTreeMap<String, String>,
    text: String,
}

fn bodies(book: &str) -> Vec<Body> {
    let board = section(book, "## 11. The board");
    let lines: Vec<&str> = board.lines().collect();
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let Some(rest) = l.strip_prefix("#### ") else { continue };
        let Some((id, title)) = rest.split_once(" · ") else { continue };
        if !is_id(id) {
            continue;
        }
        let mut fields = BTreeMap::new();
        for row in lines[i + 1..].iter().skip_while(|r| r.trim().is_empty()).take_while(|r| r.starts_with('|')) {
            let cells: Vec<&str> = row.trim_matches('|').split(" | ").map(str::trim).collect();
            if cells.len() == 2 && cells[0] != "Field" && !cells[0].starts_with("---") {
                fields.insert(cells[0].to_string(), cells[1].to_string());
            }
        }
        let text: Vec<&str> = lines[i + 1..].iter().take_while(|r| !r.starts_with('#')).copied().collect();
        out.push(Body { id: id.to_string(), title: title.to_string(), fields, text: text.join("\n") });
    }
    out
}

/// The streams of section 1.3, each with its Owns column: the backticked paths it owns and whether it also owns
/// modules a ticket will create (`(new)`), which cannot be cited as paths before they exist.
fn streams(book: &str) -> BTreeMap<String, (Vec<String>, bool)> {
    section(book, "### 1.3 Streams")
        .lines()
        .filter_map(|l| l.strip_prefix("| R"))
        .map(|l| {
            let name = format!("R{}", l.split_whitespace().next().unwrap());
            let owns = l.split(" | ").nth(1).unwrap_or_default();
            let paths = owns.split('`').skip(1).step_by(2).map(str::to_string).collect();
            (name, (paths, owns.contains("(new)")))
        })
        .collect()
}

/// The text of `text` after `from` and before `to`.
fn between<'a>(text: &'a str, from: &str, to: &str) -> &'a str {
    let start = text.find(from).map(|i| i + from.len()).unwrap_or(text.len());
    let end = text[start..].find(to).map(|i| start + i).unwrap_or(text.len());
    &text[start..end]
}

const TEMPLATE: &[&str] = &[
    "**Goal.**",
    "**Read first.**",
    "**RED tests.**",
    "**Edge cases.**",
    "**Success.**",
    "**Mutants to kill.**",
    "**Verify.**",
    "**Done when.**",
];

#[test]
fn every_ticket_body_has_one_index_line_that_agrees() {
    let book = book();
    let index = index_lines(&book);
    let mut by_id: BTreeMap<&str, &IndexLine> = BTreeMap::new();
    for line in &index {
        assert!(by_id.insert(line.id.as_str(), line).is_none(), "{} has two index lines", line.id);
    }
    let bodies = bodies(&book);
    assert!(bodies.len() >= 120, "the parse found {} ticket bodies", bodies.len());
    assert_eq!(index.len(), bodies.len(), "every index line is one body");
    let mut seen = BTreeSet::new();
    for body in &bodies {
        assert!(seen.insert(body.id.as_str()), "{} has two bodies", body.id);
        let line = by_id.get(body.id.as_str()).unwrap_or_else(|| panic!("{} has a body and no index line", body.id));
        assert_eq!(line.title, body.title, "{}: the index line's title differs from the body's", body.id);
        let field = |name: &str| {
            body.fields.get(name).unwrap_or_else(|| panic!("{}: the field table has no {name} row", body.id))
        };
        assert_eq!(&line.phase, field("Phase"), "{}: the index line's phase differs from the body's", body.id);
        let body_labels: Vec<String> = field("Labels").split_whitespace().map(|s| s.trim_matches('`').to_string()).collect();
        let line_labels: Vec<String> = line.labels.iter().filter(|l| !l.starts_with("stream:")).cloned().collect();
        assert_eq!(line_labels, body_labels, "{}: the index line's labels differ from the body's", body.id);
        assert_eq!(line.depends, list(field("Depends on")), "{}: the index line's dependencies differ from the body's", body.id);
        field("Touches");
        field("Status");
        for part in TEMPLATE {
            assert!(body.text.contains(part), "{}: the body has no {part}", body.id);
        }
        let red = between(&body.text, "**RED tests.**", "**Edge cases.**");
        assert!(red.contains("- [ ] `") || red.contains("- [x] `"), "{}: no RED test is named", body.id);
        let success = between(&body.text, "**Success.**", "**Mutants to kill.**");
        assert!(success.contains("- [ ] ") || success.contains("- [x] "), "{}: no Success box", body.id);
    }
    for line in &index {
        assert!(seen.contains(line.id.as_str()), "{} is on the board index with no body", line.id);
    }
}

#[test]
fn every_ticket_is_in_one_stream_and_every_stream_holds_work() {
    let book = book();
    let streams = streams(&book);
    assert!(streams.contains_key("R0") && streams.len() >= 10, "the streams table: {:?}", streams.keys());
    let mut held: BTreeSet<String> = BTreeSet::new();
    for line in index_lines(&book) {
        let named: Vec<&String> = line.labels.iter().filter(|l| l.starts_with("stream:")).collect();
        assert_eq!(named.len(), 1, "{} names {} streams: {:?}", line.id, named.len(), line.labels);
        let stream = named[0].trim_start_matches("stream:");
        assert!(streams.contains_key(stream), "{} is in stream {stream}, which section 1.3 does not define", line.id);
        held.insert(stream.to_string());
    }
    for stream in streams.keys() {
        assert!(held.contains(stream), "stream {stream} holds no ticket");
    }
}

#[test]
fn no_file_belongs_to_two_streams() {
    let book = book();
    let mut owner: BTreeMap<String, String> = BTreeMap::new();
    for (stream, (paths, new_modules)) in streams(&book) {
        assert!(!paths.is_empty() || new_modules, "stream {stream} owns no file and no new module");
        for path in paths {
            if let Some(other) = owner.insert(path.clone(), stream.clone()) {
                panic!("`{path}` is owned by {other} and by {stream}");
            }
        }
    }
    let owned: Vec<&String> = owner.keys().collect();
    for (i, a) in owned.iter().enumerate() {
        for b in &owned[i + 1..] {
            let (a, b) = (a.trim_end_matches('/'), b.trim_end_matches('/'));
            assert!(
                !b.starts_with(&format!("{a}/")) && !a.starts_with(&format!("{b}/")),
                "`{a}` and `{b}` overlap: {} and {}",
                owner[a],
                owner[b]
            );
        }
    }
}

#[test]
fn the_dependencies_resolve_and_never_cycle() {
    let book = book();
    let index = index_lines(&book);
    let board: BTreeSet<&str> = index.iter().map(|l| l.id.as_str()).collect();
    let theirs = fusions_board();
    let mut edges: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for line in &index {
        for dep in &line.depends {
            if let Some(f) = dep.strip_prefix("FUSIONS:") {
                assert!(theirs.contains(f), "{} depends on {dep}, which is not on the fusion book's board", line.id);
            } else {
                assert!(board.contains(dep.as_str()), "{} depends on {dep}, which is not on this board", line.id);
                assert_ne!(dep, &line.id, "{} depends on itself", line.id);
                edges.entry(line.id.as_str()).or_default().push(dep.as_str());
            }
        }
    }
    // Depth-first search with three colours: a grey node reached again closes a cycle.
    let mut colour: BTreeMap<&str, u8> = BTreeMap::new();
    fn visit<'a>(n: &'a str, edges: &BTreeMap<&'a str, Vec<&'a str>>, colour: &mut BTreeMap<&'a str, u8>, path: &mut Vec<&'a str>) {
        match colour.get(n) {
            Some(2) => return,
            Some(1) => panic!("the dependencies close a cycle: {} -> {n}", path.join(" -> ")),
            _ => {}
        }
        colour.insert(n, 1);
        path.push(n);
        for d in edges.get(n).map(Vec::as_slice).unwrap_or(&[]) {
            visit(d, edges, colour, path);
        }
        path.pop();
        colour.insert(n, 2);
    }
    for id in &board {
        visit(id, &edges, &mut colour, &mut Vec::new());
    }
}

#[test]
fn every_fusions_ticket_the_book_names_is_on_that_board() {
    let book = book();
    let theirs = fusions_board();
    let mut named = 0;
    for (at, _) in book.match_indices("FUSIONS:") {
        let id: String = book[at + 8..].chars().take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '-').collect();
        let id = id.trim_end_matches('-');
        if id.is_empty() {
            continue;
        }
        assert!(theirs.contains(id), "the book names FUSIONS:{id}, which is not on the fusion book's board");
        named += 1;
    }
    assert!(named >= 20, "the book names {named} fusion-book tickets; its shared sides live there");
}

/// The exam's tests: `- [ ] **E<k>.<n>** … Lock: `suite::test`. Ticket: ID.`
fn exam_items(book: &str) -> Vec<(String, String, String)> {
    prose_lines(section(book, "## 10. The exam catalog"))
        .into_iter()
        .filter_map(|l| {
            let rest = l.strip_prefix("- [ ] **E").or_else(|| l.strip_prefix("- [x] **E"))?;
            let (num, rest) = rest.split_once("**")?;
            let lock = rest.split("Lock: `").nth(1).and_then(|r| r.split('`').next()).unwrap_or_else(|| panic!("E{num} names no lock: {l}"));
            let ticket: String = rest
                .split("Ticket: ")
                .nth(1)
                .unwrap_or_else(|| panic!("E{num} names no ticket: {l}"))
                .chars()
                .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '-')
                .collect();
            Some((format!("E{num}"), lock.to_string(), ticket))
        })
        .collect()
}

#[test]
fn every_exam_test_names_its_lock_and_its_ticket() {
    let book = book();
    let board: BTreeSet<String> = index_lines(&book).into_iter().map(|l| l.id).collect();
    let items = exam_items(&book);
    assert!(items.len() >= 80, "the exam holds {} tests", items.len());
    let mut ids = BTreeSet::new();
    let mut locks = BTreeSet::new();
    let mut per_family: BTreeMap<String, usize> = BTreeMap::new();
    for (id, lock, ticket) in &items {
        assert!(ids.insert(id.clone()), "{id} appears twice");
        let (suite, test) = lock.split_once("::").unwrap_or_else(|| panic!("{id}'s lock `{lock}` is not suite::test"));
        assert!(suite.ends_with("_lock") && !test.is_empty(), "{id}'s lock `{lock}` is not a lock test");
        assert!(locks.insert(lock.clone()), "{id} reuses the lock test `{lock}`");
        assert!(board.contains(ticket), "{id} names {ticket}, which is not on the board");
        *per_family.entry(id.split('.').next().unwrap().to_string()).or_insert(0) += 1;
    }
    for k in 1..=13 {
        let n = per_family.get(&format!("E{k}")).copied().unwrap_or(0);
        assert!(n >= 3, "exam family E{k} holds {n} tests");
    }
}

/// The `#[test]` functions of a suite's source.
fn test_functions(src: &str) -> BTreeSet<String> {
    let mut tests = BTreeSet::new();
    let mut attributed = false;
    for line in src.lines() {
        if line.starts_with("#[test]") {
            attributed = true;
        } else if let Some(rest) = line.strip_prefix("fn ") {
            if attributed {
                tests.insert(rest.split('(').next().unwrap_or("").to_string());
            }
            attributed = false;
        }
    }
    tests
}

#[test]
fn every_written_exam_test_is_where_its_row_says() {
    let book = book();
    let ticked: BTreeMap<String, bool> = prose_lines(section(&book, "## 10. The exam catalog"))
        .into_iter()
        .filter_map(|l| match (l.strip_prefix("- [x] **E"), l.strip_prefix("- [ ] **E")) {
            (Some(r), _) => r.split_once("**").map(|(n, _)| (format!("E{n}"), true)),
            (_, Some(r)) => r.split_once("**").map(|(n, _)| (format!("E{n}"), false)),
            _ => None,
        })
        .collect();
    let tests = repo().join("crates/logicaffeine_tests/tests");
    let gate = "#![cfg(feature = \"handed-reds\")]";
    let mut named: BTreeSet<(String, String)> = BTreeSet::new();
    let mut written = 0;
    for (id, lock, _) in exam_items(&book) {
        let (suite, test) = lock.split_once("::").unwrap();
        named.insert((suite.to_string(), test.to_string()));
        let path = tests.join(format!("{suite}.rs"));
        let done = ticked[&id];
        let Ok(src) = std::fs::read_to_string(&path) else {
            assert!(!done, "{id} is ticked, but its suite {suite} is not written");
            continue;
        };
        assert!(test_functions(&src).contains(test), "{id}: {suite} holds no test `{test}`");
        let handed = src.lines().any(|l| l == gate);
        assert!(
            done != handed,
            "{id} is {} and its test is {} the handed-reds gate",
            if done { "ticked" } else { "open" },
            if handed { "behind" } else { "outside" }
        );
        written += 1;
    }
    assert!(written >= 12, "{written} exam tests are written");
    // Every test of an exam suite is the test of some row.
    for entry in std::fs::read_dir(&tests).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        if !(name.starts_with("samba_exam_") && path.extension().is_some_and(|e| e == "rs")) {
            continue;
        }
        for t in test_functions(&std::fs::read_to_string(&path).unwrap()) {
            assert!(named.contains(&(name.clone(), t.clone())), "{name}::{t} is the test of no exam row");
        }
    }
}

/// The bibliography's keys: `- [KEY] …` lines of section 12.3.
fn bibliography(book: &str) -> BTreeSet<String> {
    section(book, "### 12.3 Bibliography")
        .lines()
        .filter_map(|l| l.strip_prefix("- ["))
        .filter_map(|r| r.split_once(']').map(|(k, _)| k.to_string()))
        .collect()
}

/// Every `[KEY]` citation in `text` whose key is upper-case letters, digits and hyphens.
fn citations(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (at, _) in text.match_indices('[') {
        let rest = &text[at + 1..];
        let Some((key, after)) = rest.split_once(']') else { continue };
        if key.len() >= 3 && is_id(key) && !after.starts_with('(') {
            out.push(key.to_string());
        }
    }
    out
}

#[test]
fn every_copy_bar_cites_the_bibliography_and_every_entry_is_cited() {
    let book = book();
    let keys = bibliography(&book);
    assert!(keys.len() >= 15, "the bibliography holds {} entries", keys.len());
    let bars = section(&book, "### 2.2 The Copy bars");
    let rows: Vec<&str> = bars
        .lines()
        .filter(|l| l.starts_with("| C-"))
        .collect();
    assert!(rows.len() >= 10, "section 2.2 holds {} Copy-bar rows", rows.len());
    for row in rows {
        let cited = citations(row);
        assert!(!cited.is_empty(), "a Copy bar cites no source: {row}");
        for key in cited {
            assert!(keys.contains(&key), "a Copy bar cites [{key}], which the bibliography does not hold: {row}");
        }
    }
    let body = book.split("### 12.3 Bibliography").next().unwrap();
    let used: BTreeSet<String> = citations(body).into_iter().collect();
    for key in &keys {
        assert!(used.contains(key), "the bibliography holds [{key}], which the book never cites");
    }
    for key in &used {
        assert!(keys.contains(key), "the book cites [{key}], which the bibliography does not hold");
    }
}

/// Repository roots whose cited paths must exist; `external/` is checked where the clones are present.
const CHECKED_ROOTS: &[&str] = &["crates/", "apps/", "scripts/", "benchmarks/", "docs/", ".config/", "external/"];

#[test]
fn every_repository_path_the_book_cites_exists() {
    let book = book();
    let root = repo();
    let clones = root.join("external").is_dir();
    let mut missing = Vec::new();
    let mut checked = 0;
    for line in prose_lines(&book) {
        for span in line.split('`').skip(1).step_by(2) {
            let bare = span.split('#').next().unwrap_or_default().trim();
            let bare = bare
                .split_once(':')
                .filter(|(_, n)| n.chars().all(|c| c.is_ascii_digit() || c == '-'))
                .map_or(bare, |(p, _)| p);
            if !CHECKED_ROOTS.iter().any(|r| bare.starts_with(r)) || bare.chars().any(|c| " *<>{}…$|\\\"'=,;()[]".contains(c)) {
                continue;
            }
            if bare.starts_with("external/") && !clones {
                continue;
            }
            checked += 1;
            if !root.join(bare.trim_end_matches('/')).exists() {
                missing.push(bare.to_string());
            }
        }
    }
    missing.sort();
    missing.dedup();
    assert!(checked >= 60, "the book cites {checked} repository paths");
    assert!(missing.is_empty(), "the book cites paths that do not exist:\n{}", missing.join("\n"));
}

#[test]
fn the_preface_counts_are_the_books_own() {
    let book = book();
    let line = book
        .lines()
        .find(|l| l.starts_with("**Counts.**"))
        .expect("the preface states the book's counts on a `**Counts.**` line");
    let numbers: Vec<usize> = line
        .split(|c: char| !c.is_ascii_digit() && c != ',')
        .map(|s| s.replace(',', ""))
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().unwrap())
        .collect();
    assert_eq!(numbers.len(), 4, "`**Counts.** T tickets, E exam tests, B checkboxes, X ticked.`: {line}");
    let lines = prose_lines(&book);
    let boxes = lines.iter().filter(|l| l.trim_start().starts_with("- [ ] ") || l.trim_start().starts_with("- [x] ")).count();
    let ticked = lines.iter().filter(|l| l.trim_start().starts_with("- [x] ")).count();
    let computed = [index_lines(&book).len(), exam_items(&book).len(), boxes, ticked];
    assert_eq!(numbers, computed, "the preface states {numbers:?}; the book holds {computed:?} (tickets, exam tests, boxes, ticked)");
}

/// The prefixes of this board's ticket ids.
const PREFIXES: &[&str] = &[
    "BOOK", "STUDY", "ACCESS", "PLAST", "CHIP", "TIME", "INTAKE", "AUTHOR", "ARCH", "QWEN4", "KERNEL", "SCHED", "MULTI",
    "NUM", "MEM", "SERVE-RDU", "MINE", "CLIMB", "EXAM", "SCORE", "DEPLOY", "TRAIN",
];

#[test]
fn every_ticket_the_book_names_is_on_the_board() {
    let book = book();
    let board: BTreeSet<String> = index_lines(&book).into_iter().map(|l| l.id).collect();
    let mut unknown = BTreeSet::new();
    let mut named = 0;
    for prefix in PREFIXES {
        let needle = format!("{prefix}-");
        for (at, _) in book.match_indices(&needle) {
            let before = book[..at].chars().next_back();
            if before.is_some_and(|c| c.is_ascii_alphanumeric() || "-:_".contains(c)) {
                continue;
            }
            let id: String = book[at..]
                .chars()
                .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '-')
                .collect();
            let id = id.trim_end_matches('-');
            if id.len() <= needle.len() {
                continue;
            }
            named += 1;
            if !board.contains(id) {
                unknown.insert(id.to_string());
            }
        }
    }
    assert!(named >= 300, "the book names {named} tickets");
    assert!(unknown.is_empty(), "the book names tickets the board does not hold: {unknown:?}");
}
