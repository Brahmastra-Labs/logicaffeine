//! samba_papers_lock — every source `benchmarks/PREPARE_FOR_SAMBA.md` cites is pinned, and every number the book
//! attributes to a source is in that source's pinned text.
//!
//! `benchmarks/samba_papers.tsv` pins each bibliography entry: title, authors, venue, year, a stable locator, the
//! capture date, the capture under `external/` (`scripts/accel/samba-papers.py`) and its SHA-256, and the capture's
//! searchable text. A sentence or table row that cites a source is a claim: each number in it must appear in the text
//! of a source it cites, unless the book writes out the arithmetic that derives it (`9 + 6.7 = 15.7`), whose operands
//! are then the numbers to find. Code spans, ticket and cell ids, section numbers and dates are the book's own.
//!
//! The captures live under the gitignored `external/papers/`; without them the digest and number checks record an
//! honest skip (`samba-papers`). `SAMBA_BOOK` and `SAMBA_PAPERS` point a mutation run at mutated copies.

mod common;
#[path = "common/samba_papers.rs"]
mod samba_papers;

use logicaffeine_compile::crypto_accel::sha256;
use samba_papers::{appears, pins, read, repo, source_texts, ungroup, Pin};
use std::collections::{BTreeMap, BTreeSet};

fn book() -> String {
    read("SAMBA_BOOK", "benchmarks/PREPARE_FOR_SAMBA.md")
}

/// The bibliography: each entry's key and its text, in the order section 12.3 lists them.
fn bibliography(book: &str) -> Vec<(String, String)> {
    let at = book.find("### 12.3 Bibliography").expect("section 12.3");
    let rest = &book[at..];
    let end = rest[1..].find("\n### ").map(|i| i + 1).unwrap_or(rest.len());
    let mut out: Vec<(String, String)> = Vec::new();
    for line in rest[..end].lines() {
        if let Some(entry) = line.strip_prefix("- [") {
            let key = entry.split(']').next().unwrap().to_string();
            out.push((key, line.to_string()));
        } else if line.starts_with("  ") {
            if let Some(last) = out.last_mut() {
                last.1.push(' ');
                last.1.push_str(line.trim());
            }
        }
    }
    out
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b[4] == b'-' && b[7] == b'-' && s.chars().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

#[test]
fn every_entry_names_a_venue_a_year_and_a_digest() {
    let book = book();
    let bib = bibliography(&book);
    let pins = pins();
    let keys: BTreeSet<&str> = bib.iter().map(|(k, _)| k.as_str()).collect();
    for (key, entry) in &bib {
        assert!(!entry.contains("to pin"), "[{key}] is still marked to pin: {entry}");
        let rows: Vec<&Pin> = pins.iter().filter(|p| p["key"] == *key).collect();
        assert!(!rows.is_empty(), "[{key}] has no pin");
        for p in rows {
            for col in ["title", "authors", "venue", "locator", "capture", "text"] {
                assert!(!p[col].trim().is_empty(), "[{key}]: no {col}");
            }
            assert!(p["year"].len() == 4 && p["year"].chars().all(|c| c.is_ascii_digit()), "[{key}]: year {:?}", p["year"]);
            assert!(is_date(&p["captured"]), "[{key}]: capture date {:?}", p["captured"]);
            assert!(p["sha256"].len() == 64 && p["sha256"].chars().all(|c| c.is_ascii_hexdigit()), "[{key}]: digest {:?}", p["sha256"]);
            assert!(p["capture"].starts_with("external/"), "[{key}]: the capture {:?} lives under external/", p["capture"]);
            assert!(entry.contains(&p["year"]), "[{key}]: the entry does not state the pinned year {}: {entry}", p["year"]);
        }
    }
    for p in &pins {
        assert!(keys.contains(p["key"].as_str()), "the pin of [{}] names no bibliography entry", p["key"]);
    }
}

fn hex(d: [u8; 32]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn every_pin_matches_its_capture() {
    if !common::require_tool("samba-papers") {
        return;
    }
    for p in pins() {
        let path = repo().join(&p["capture"]);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("[{}] {}: {e}", p["key"], path.display()));
        assert_eq!(hex(sha256(&bytes)), p["sha256"], "[{}] {}: the capture is not what was pinned", p["key"], p["capture"]);
    }
}

/// The claims of the book: each sentence of prose and each table row before the bibliography that cites a source.
fn claims(book: &str) -> Vec<String> {
    let body = &book[..book.find("### 12.3 Bibliography").expect("section 12.3")];
    let mut out = Vec::new();
    let mut fenced = false;
    let mut block: Vec<&str> = Vec::new();
    let mut flush = |block: &mut Vec<&str>, out: &mut Vec<String>| {
        if block.is_empty() {
            return;
        }
        if block[0].trim_start().starts_with('|') {
            out.extend(block.iter().map(|l| l.to_string()));
        } else {
            let flat = block.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
            let mut start = 0;
            let b = flat.as_bytes();
            for i in 0..b.len() {
                if (b[i] == b'.' || b[i] == b';') && b.get(i + 1) == Some(&b' ') {
                    if let Some(&n) = b.get(i + 2) {
                        if n.is_ascii_uppercase() || n == b'(' || n == b'*' || n == b'-' {
                            out.push(flat[start..=i].to_string());
                            start = i + 2;
                        }
                    }
                }
            }
            out.push(flat[start..].to_string());
        }
        block.clear();
    };
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            flush(&mut block, &mut out);
            continue;
        }
        if fenced {
            continue;
        }
        if line.trim().is_empty() || line.starts_with('#') {
            flush(&mut block, &mut out);
        } else {
            block.push(line);
        }
    }
    flush(&mut block, &mut out);
    out.into_iter().filter(|c| cited(c).next().is_some()).collect()
}

/// The keys a claim cites.
fn cited(claim: &str) -> impl Iterator<Item = &str> {
    claim.split('[').skip(1).filter_map(|s| {
        let key = s.split(']').next()?;
        (key.len() >= 2 && key.chars().next()?.is_ascii_uppercase() && key.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()))
            .then_some(key)
    })
}

/// A number the book writes: its digits as the source would show them (grouping removed), with a unit letter it
/// carries (`11.8K`), and where it sits.
#[derive(Clone, Debug)]
struct Number {
    text: String,
    value: f64,
    at: usize,
    end: usize,
}

/// The claim with what is the book's own blanked: code spans, citation keys, ticket and cell ids, section numbers,
/// dates and links.
fn scrub(claim: &str) -> String {
    let mut s: Vec<char> = claim.chars().collect();
    let blank = |s: &mut Vec<char>, a: usize, b: usize| (a..b).for_each(|i| s[i] = ' ');
    let mut i = 0;
    while i < s.len() {
        if s[i] == '`' {
            let end = s[i + 1..].iter().position(|&c| c == '`').map(|p| i + 1 + p + 1).unwrap_or(s.len());
            blank(&mut s, i, end);
            i = end;
            continue;
        }
        if s[i] == '[' {
            if let Some(p) = s[i + 1..].iter().position(|&c| c == ']') {
                let inner: String = s[i + 1..i + 1 + p].iter().collect();
                if inner.len() >= 2 && inner.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
                    blank(&mut s, i, i + p + 2);
                    i += p + 2;
                    continue;
                }
            }
        }
        if s[i..].starts_with(&['h', 't', 't', 'p']) {
            let end = s[i..].iter().position(|c| c.is_whitespace() || *c == ')' || *c == '|').map(|p| i + p).unwrap_or(s.len());
            blank(&mut s, i, end);
            i = end;
            continue;
        }
        // A capitalized word joined by hyphens with a digit in it names a ticket, a cell or a model (`SCHED-1`,
        // `C-SN50x64-DSR1-D`, `Llama-3.1-8B`): its digits are names, not claims.
        if s[i].is_ascii_uppercase() && (i == 0 || !s[i - 1].is_alphanumeric()) {
            let end = s[i..]
                .iter()
                .position(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '.'))
                .map(|p| i + p)
                .unwrap_or(s.len());
            let word: String = s[i..end].iter().collect();
            if word.contains('-') && word.chars().any(|c| c.is_ascii_digit()) {
                blank(&mut s, i, end);
                i = end;
                continue;
            }
        }
        i += 1;
    }
    // `section 7.1`, `sections 2 to 10`, `sections 4, 5 and 6`: the book's own numbering.
    let mut i = 0;
    while i < s.len() {
        let lead: String = s[i..s.len().min(i + 8)].iter().collect();
        if (lead.starts_with("section ") || lead.starts_with("sections")) && (i == 0 || !s[i - 1].is_alphanumeric()) {
            let mut j = i + if lead.starts_with("sections") { 8 } else { 7 };
            loop {
                while j < s.len() && s[j] == ' ' {
                    j += 1;
                }
                let rest: String = s[j..s.len().min(j + 4)].iter().collect();
                if j < s.len() && s[j].is_ascii_digit() {
                    while j < s.len() && (s[j].is_ascii_digit() || (s[j] == '.' && s.get(j + 1).is_some_and(char::is_ascii_digit))) {
                        j += 1;
                    }
                } else if rest.starts_with("and ") || rest.starts_with("to ") || rest.starts_with(", ") {
                    j += if rest.starts_with("and ") { 3 } else { 1 };
                    if rest.starts_with(", ") {
                        j += 1;
                    }
                } else {
                    break;
                }
            }
            blank(&mut s, i, j);
            i = j;
            continue;
        }
        i += 1;
    }
    let chars: Vec<char> = s;
    let mut cleaned = chars.clone();
    for i in 0..chars.len().saturating_sub(9) {
        let w: String = chars[i..i + 10].iter().collect();
        if is_date(&w) {
            (i..i + 10).for_each(|j| cleaned[j] = ' ');
        }
    }
    cleaned.into_iter().collect()
}

/// The numbers of a scrubbed claim, in order.
fn numbers(s: &str) -> Vec<Number> {
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let starts = c[i].is_ascii_digit() && (i == 0 || !(c[i - 1].is_alphanumeric() || c[i - 1] == '.' || c[i - 1] == '-' || c[i - 1] == '_' || c[i - 1] == '/' || c[i - 1] == ':'));
        if !starts {
            i += 1;
            continue;
        }
        let begin = i;
        let mut digits = String::new();
        while i < c.len() && c[i].is_ascii_digit() {
            digits.push(c[i]);
            i += 1;
        }
        // Thousands groups: `,` and exactly three digits.
        while i + 3 < c.len() && c[i] == ',' && c[i + 1..i + 4].iter().all(char::is_ascii_digit) && c.get(i + 4).is_none_or(|n| !n.is_ascii_digit()) {
            digits.extend(&c[i + 1..i + 4]);
            i += 4;
        }
        // A decimal part; further dotted groups make a version (`0.1.1`), read whole.
        let mut groups = 0;
        while i + 1 < c.len() && c[i] == '.' && c[i + 1].is_ascii_digit() {
            digits.push('.');
            i += 1;
            while i < c.len() && c[i].is_ascii_digit() {
                digits.push(c[i]);
                i += 1;
            }
            groups += 1;
        }
        // A unit letter glued to the number (`11.8K`) is part of it; any other letter makes it a name (`H100`).
        let mut text = digits.clone();
        if i < c.len() && matches!(c[i], 'K' | 'M' | 'G') && c.get(i + 1).is_none_or(|n| !n.is_alphanumeric()) {
            text.push(c[i]);
            i += 1;
        } else if i < c.len() && (c[i].is_alphabetic() || c[i] == '_' || c[i] == '/') {
            continue;
        }
        let value = if groups > 1 { f64::NAN } else { digits.parse().unwrap_or(f64::NAN) };
        out.push(Number { value, text, at: begin, end: i });
    }
    out
}

/// The numbers a claim derives by arithmetic it writes out (`a + b = c`, `a × b = c`, `a − b = c`, `a / b = c`), each
/// checked: the result's index in `numbers`, when the operands give it.
fn derived(s: &str, nums: &[Number]) -> Vec<usize> {
    let c: Vec<char> = s.chars().collect();
    let between = |a: &Number, b: &Number| -> String { c[a.end..b.at].iter().collect::<String>().trim().to_string() };
    let mut out = Vec::new();
    let mut i = 0;
    while i < nums.len() {
        let mut j = i;
        let mut value = nums[i].value;
        let mut ok = false;
        while j + 1 < nums.len() {
            match between(&nums[j], &nums[j + 1]).as_str() {
                "+" => value += nums[j + 1].value,
                "−" | "-" => value -= nums[j + 1].value,
                "×" | "*" => value *= nums[j + 1].value,
                "/" | "÷" => value /= nums[j + 1].value,
                "=" if j > i => {
                    let r = nums[j + 1].value;
                    let decimals = nums[j + 1].text.split('.').nth(1).map_or(0, str::len) as i32;
                    let scale = 10f64.powi(decimals);
                    assert!((value * scale).round() == (r * scale).round(), "the arithmetic in {s:?} does not give {}", nums[j + 1].text);
                    out.push(j + 1);
                    ok = true;
                    j += 1;
                    break;
                }
                _ => break,
            }
            j += 1;
        }
        i = if ok { j + 1 } else { i + 1 };
    }
    out
}

/// Every number of `claim` that is in no pinned text of a source it cites.
fn unfound(claim: &str, texts: &BTreeMap<String, String>) -> Vec<String> {
    let keys: Vec<&str> = cited(claim).filter(|k| texts.contains_key(*k)).collect();
    if keys.is_empty() {
        return Vec::new();
    }
    let s = scrub(claim);
    let nums = numbers(&s);
    let results = derived(&s, &nums);
    nums.iter()
        .enumerate()
        .filter(|(i, _)| !results.contains(i))
        .filter(|(_, n)| !keys.iter().any(|k| appears(&texts[*k], &n.text)))
        .map(|(_, n)| format!("{} (cites {})", n.text, keys.join(", ")))
        .collect()
}

#[test]
fn every_copy_bar_number_appears_in_its_pinned_source() {
    if !common::require_tool("samba-papers") {
        return;
    }
    let book = book();
    let texts = source_texts(&pins());
    let at = book.find("### 2.2 The Copy bars").expect("section 2.2");
    let rows: Vec<&str> = book[at..].lines().skip(1).take_while(|l| !l.starts_with("### ")).filter(|l| l.starts_with("| C-")).collect();
    assert!(rows.len() >= 15, "the Copy bars: {} rows", rows.len());
    let mut missing = Vec::new();
    for row in rows {
        for n in unfound(row, &texts) {
            missing.push(format!("{n}: {row}"));
        }
    }
    assert!(missing.is_empty(), "Copy bar numbers missing from their sources:\n{}", missing.join("\n"));
}

#[test]
fn every_number_the_book_attributes_to_a_source_is_in_its_pinned_text() {
    if !common::require_tool("samba-papers") {
        return;
    }
    let book = book();
    let texts = source_texts(&pins());
    let all = claims(&book);
    assert!(all.len() >= 100, "the claims: {}", all.len());
    let mut missing = Vec::new();
    for claim in &all {
        for n in unfound(claim, &texts) {
            missing.push(format!("{n}: {}", claim.chars().take(200).collect::<String>()));
        }
    }
    assert!(missing.is_empty(), "numbers missing from the sources they cite:\n{}", missing.join("\n"));
}

/// The scanner finds the numbers a claim states and none of the book's own.
#[test]
fn the_number_scanner_reads_claims_as_written() {
    let s = scrub("| C-SN40Lx8-LLAMA31-8B-D1-BF16 | SN40L, 8 sockets | 1,140 tok/s, 76% of roofline, 11.8K samples/s on H100 (section 7.1, `x = 3`, read 2026-10-29) [KL24] |");
    let got: Vec<String> = numbers(&s).into_iter().map(|n| n.text).collect();
    assert_eq!(got, ["8", "1140", "76", "11.8K"]);
    let s = scrub("the kit's tracer (stubbed 0.1.1) [STUBBED]");
    assert_eq!(numbers(&s).into_iter().map(|n| n.text).collect::<Vec<_>>(), ["0.1.1"]);
    let s = scrub("draft 9 + 6.7 = 15.7 ms; 33 + 12.6 = 45.6 on a DGX H100 [DIAYN26]");
    let nums = numbers(&s);
    assert_eq!(derived(&s, &nums).into_iter().map(|i| nums[i].text.clone()).collect::<Vec<_>>(), ["15.7", "45.6"]);
    assert!(appears(&ungroup("at 1,140 and 6.7 ms"), "1140") && appears("6.7 ms", "6.7"));
    assert!(!appears("16.7 and 6.75", "6.7") && !appears("6.7", "6") && !appears("0.6", "6"));
}

#[test]
#[should_panic(expected = "does not give")]
fn arithmetic_the_book_gets_wrong_is_refused() {
    let s = scrub("9 + 6.7 = 15.8 [DIAYN26]");
    derived(&s, &numbers(&s));
}
