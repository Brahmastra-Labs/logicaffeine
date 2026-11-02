//! sota_contract_lock — `benchmarks/FUSIONS_WE_NEED.md` holds together.
//!
//! The book is the dispatch board every agent takes work from, so its derived parts are checked, never kept by
//! hand: every ticket has exactly one board index line that agrees with its body; every index line names one
//! stream that the streams table defines; every coverage row in section 11 maps to tickets on the board; and the
//! coverage count a ticket states is the number of rows that map to it. This is the book-internal slice of
//! LEDGER-3; the cell ledger and the re-extraction at the pins join it there.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The book; `FUSIONS_BOOK` points a mutation run at a mutated copy.
fn book() -> String {
    let path = std::env::var("FUSIONS_BOOK")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/FUSIONS_WE_NEED.md"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The text between the heading that starts with `start` and the next heading of the same level.
fn section<'a>(book: &'a str, start: &str) -> &'a str {
    let at = book.find(start).unwrap_or_else(|| panic!("no section {start:?}"));
    let level = start.chars().take_while(|c| *c == '#').count();
    let rest = &book[at + start.len()..];
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

/// One line of the board index: `- [ ] **ID** · title · phase · labels · depends on: deps`.
#[derive(Debug)]
struct IndexLine {
    id: String,
    title: String,
    phase: String,
    labels: Vec<String>,
    depends: Vec<String>,
}

fn index_lines(book: &str) -> Vec<IndexLine> {
    section(book, "### 12.1 Board index")
        .lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("- [ ] **").or_else(|| l.strip_prefix("- [x] **"))?;
            let (id, rest) = rest.split_once("** · ")?;
            let parts: Vec<&str> = rest.split(" · ").collect();
            assert_eq!(parts.len(), 4, "an index line is title · phase · labels · depends on: {l}");
            let depends = parts[3].strip_prefix("depends on: ").unwrap_or_else(|| panic!("no `depends on:` in {l}"));
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

fn list(text: &str) -> Vec<String> {
    if text.trim() == "none" {
        return Vec::new();
    }
    text.split(',').map(|s| s.trim().to_string()).collect()
}

/// A ticket body: `#### ID · title` and its field table.
#[derive(Debug)]
struct Body {
    id: String,
    title: String,
    fields: BTreeMap<String, String>,
}

fn bodies(book: &str) -> Vec<Body> {
    let board = section(book, "## 12. The board");
    let mut out = Vec::new();
    let lines: Vec<&str> = board.lines().collect();
    for (i, l) in lines.iter().enumerate() {
        let Some(rest) = l.strip_prefix("#### ") else { continue };
        let Some((id, title)) = rest.split_once(" · ") else { continue };
        if !id.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-') || id.is_empty() {
            continue;
        }
        let mut fields = BTreeMap::new();
        for row in lines[i + 1..].iter().skip_while(|r| r.trim().is_empty()).take_while(|r| r.starts_with('|')) {
            let cells: Vec<&str> = row.trim_matches('|').split(" | ").map(str::trim).collect();
            if cells.len() == 2 && cells[0] != "Field" && !cells[0].starts_with("---") {
                fields.insert(cells[0].to_string(), cells[1].to_string());
            }
        }
        out.push(Body { id: id.to_string(), title: title.to_string(), fields });
    }
    out
}

/// The compact tickets (sections 12.14 and 12.16): `- [ ] **ID**: …`, each with its stated coverage.
fn compact_tickets(book: &str) -> BTreeMap<String, Option<usize>> {
    let board = section(book, "## 12. The board");
    let mut out = BTreeMap::new();
    for l in board.lines() {
        let Some(rest) = l.strip_prefix("- [ ] **").or_else(|| l.strip_prefix("- [x] **")) else { continue };
        let Some((id, text)) = rest.split_once("**: ") else { continue };
        let coverage = text.split("Coverage: ").nth(1).map(|c| {
            c.split_whitespace().next().unwrap().parse::<usize>().expect("a coverage count")
        });
        out.insert(id.to_string(), coverage);
    }
    out
}

/// Section 11's rows: each maps one entry at the pins to one or more tickets.
fn coverage_rows(book: &str) -> Vec<Vec<String>> {
    section(book, "## 11. The coverage map")
        .lines()
        .filter(|l| l.starts_with("- [ ] `") || l.starts_with("- [x] `"))
        .map(|l| {
            let (_, tickets) = l.rsplit_once(" → ").unwrap_or_else(|| panic!("a coverage row names its tickets: {l}"));
            tickets.split(',').map(|t| t.trim().to_string()).collect()
        })
        .collect()
}

/// The streams the streams table of section 1.5 defines.
fn streams(book: &str) -> BTreeSet<String> {
    section(book, "### 1.5 Streams")
        .lines()
        .filter_map(|l| l.strip_prefix("| S"))
        .map(|l| format!("S{}", l.split_whitespace().next().unwrap()))
        .collect()
}

#[test]
fn every_ticket_is_in_one_stream() {
    let book = book();
    let streams = streams(&book);
    assert!(streams.contains("S0") && streams.contains("S1"), "the streams table: {streams:?}");
    for line in index_lines(&book) {
        let named: Vec<&String> = line.labels.iter().filter(|l| l.starts_with("stream:")).collect();
        assert_eq!(named.len(), 1, "{} names {} streams: {:?}", line.id, named.len(), line.labels);
        let stream = named[0].trim_start_matches("stream:");
        assert!(streams.contains(stream), "{} is in stream {stream}, which section 1.5 does not define", line.id);
    }
}

#[test]
fn every_ticket_body_has_one_index_line_that_agrees() {
    let book = book();
    let index = index_lines(&book);
    let mut by_id: BTreeMap<&str, &IndexLine> = BTreeMap::new();
    for line in &index {
        assert!(by_id.insert(line.id.as_str(), line).is_none(), "{} has two index lines", line.id);
    }
    let compact = compact_tickets(&book);
    let bodies = bodies(&book);
    assert!(bodies.len() > 150 && compact.len() > 30, "the parse found {} bodies and {} compact tickets", bodies.len(), compact.len());
    assert_eq!(index.len(), bodies.len() + compact.len(), "every index line is one body or one compact ticket");
    let mut seen = BTreeSet::new();
    for body in &bodies {
        assert!(seen.insert(body.id.as_str()), "{} has two bodies", body.id);
        let line = by_id.get(body.id.as_str()).unwrap_or_else(|| panic!("{} has a body and no index line", body.id));
        assert_eq!(line.title, body.title, "{}: the index line's title differs from the body's", body.id);
        if let Some(phase) = body.fields.get("Phase") {
            assert_eq!(&line.phase, phase, "{}: the index line's phase differs from the body's", body.id);
        }
        if let Some(labels) = body.fields.get("Labels") {
            let body_labels: Vec<String> = labels.split_whitespace().map(|s| s.trim_matches('`').to_string()).collect();
            let line_labels: Vec<String> = line.labels.iter().filter(|l| !l.starts_with("stream:")).cloned().collect();
            assert_eq!(line_labels, body_labels, "{}: the index line's labels differ from the body's", body.id);
        }
        if let Some(depends) = body.fields.get("Depends on") {
            assert_eq!(line.depends, list(depends), "{}: the index line's dependencies differ from the body's", body.id);
        }
    }
    for line in &index {
        assert!(
            seen.contains(line.id.as_str()) || compact.contains_key(&line.id),
            "{} is on the board index with no body",
            line.id
        );
        for dep in &line.depends {
            assert!(by_id.contains_key(dep.as_str()), "{} depends on {dep}, which is not on the board", line.id);
        }
    }
}

#[test]
fn every_coverage_row_names_tickets_on_the_board() {
    let book = book();
    let board: BTreeSet<String> = index_lines(&book).into_iter().map(|l| l.id).collect();
    let rows = coverage_rows(&book);
    assert!(rows.len() >= 1294, "section 11 holds {} rows, fewer than the 1,294 extracted at the pins", rows.len());
    for tickets in rows {
        for t in tickets {
            assert!(board.contains(&t), "a coverage row maps to {t}, which is not on the board");
        }
    }
}

#[test]
fn every_tickets_coverage_count_is_the_rows_that_map_to_it() {
    let book = book();
    let mut owned: BTreeMap<String, usize> = BTreeMap::new();
    for tickets in coverage_rows(&book) {
        for t in tickets {
            *owned.entry(t).or_insert(0) += 1;
        }
    }
    let bodies = bodies(&book);
    let compact = compact_tickets(&book);
    let stated = |id: &str| -> Option<usize> {
        bodies
            .iter()
            .find(|b| b.id == id)
            .and_then(|b| b.fields.get("Coverage").map(|c| c.split_whitespace().next().unwrap().parse().expect("a count")))
            .or_else(|| compact.get(id).copied().flatten())
    };
    for line in index_lines(&book) {
        let n = owned.get(&line.id).copied().unwrap_or(0);
        assert_eq!(
            stated(&line.id).unwrap_or(0),
            n,
            "{} states a coverage of {:?}, and {n} rows of section 11 map to it",
            line.id,
            stated(&line.id)
        );
    }
}
