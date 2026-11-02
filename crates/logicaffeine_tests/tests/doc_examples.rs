//! Doc-example coverage harness — every code example in `README.md`, `docs/*.md`
//! and every crate's README (its crates.io page and rustdoc root) is extracted and
//! verified here, so the documentation set cannot silently rot.
//!
//! - Every ```logos block must compile (`compile_to_rust` → `Ok`, with no
//!   embedded refusal). Complete programs (with a `##` block header) are
//!   compiled as-is; headerless fragments are wrapped in a minimal,
//!   content-keyed context so the snippet shown in the docs is exercised
//!   exactly.
//! - A ```logos block followed by an ```output block is RUN on the
//!   tree-walker, and what it prints must be exactly that output — a shown
//!   result is a claim, and this is what holds it.
//! - A ```logos block followed by a ```rust block is shown next to the Rust it
//!   compiles to: that Rust must appear, whitespace for whitespace up to runs,
//!   in what `compile_to_rust` emits for the program.
//! - Every `## Test` in a ```logos block must pass.
//! - Every ```logos-dom block (a page) is held by `canonical_teaching_lock`:
//!   parsed, mounted and rendered on the tree-walker and the VM, compiled for
//!   the browser, and its `## Test` blocks run.
//! - Every English → First-Order-Logic pair (the `"sentence" → ∀x…` lines) must
//!   reproduce its documented output via `compile`, and every sentence shown with
//!   its numbered readings (`"sentence"` then `1. …`, `2. …`) must have exactly
//!   those readings, in that order, from `compile_all_scopes`.
//! - No ```logos or ```logos-dom block may contain a `...` placeholder — doc
//!   examples are real, runnable code, not schematics.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test doc_examples`

#![cfg(not(target_arch = "wasm32"))]

#[path = "tier_parity/docs.rs"]
mod docs;

use docs::{all_blocks, doc_files, fenced_blocks, make_program, Block};
use logicaffeine_compile::compile::{compile_to_rust, tw_outcome};
use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};
use logicaffeine_language::{compile, compile_all_scopes};

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extract `"english" → FOL` pairs from an untagged block (both the one-line form
/// and the two-line form used in the guides).
fn fol_pairs(body: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = body.lines().collect();
    let mut pairs = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        // One-line: "english"  →  FOL
        if line.starts_with('"') {
            if let Some(close) = line[1..].find('"') {
                let english = &line[1..1 + close];
                let rest = line[1 + close + 1..].trim_start();
                if let Some(fol) = rest.strip_prefix('→') {
                    pairs.push((english.to_string(), fol.trim().to_string()));
                    i += 1;
                    continue;
                }
                // Two-line: "english" then a following → FOL line.
                let mut j = i + 1;
                while j < lines.len() && lines[j].trim().is_empty() {
                    j += 1;
                }
                if j < lines.len() {
                    if let Some(fol) = lines[j].trim().strip_prefix('→') {
                        pairs.push((english.to_string(), fol.trim().to_string()));
                        i = j + 1;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    pairs
}

#[test]
fn doc_no_placeholder_examples() {
    let bad: Vec<String> = all_blocks()
        .iter()
        .filter(|b| b.lang == "logos" && b.body.contains("..."))
        .map(|b| format!("{}:{}", b.file, b.line))
        .collect();
    assert!(
        bad.is_empty(),
        "logos doc examples must be real, not `...` schematics: {bad:?}"
    );
}

/// The sentence inside an emitted `compile_error!("…")` — what the backend actually said, rather
/// than the bare fact that it said something.
fn refusal_message(rust: &str) -> String {
    rust.find("compile_error!")
        .and_then(|at| {
            let rest = &rust[at..];
            let open = rest.find('"')?;
            let body = &rest[open + 1..];
            let end = body.find('"')?;
            Some(body[..end].to_string())
        })
        .unwrap_or_else(|| "the backend embedded a compile_error! with no readable message".into())
}

#[test]
fn doc_logos_examples_compile() {
    let mut failures = Vec::new();
    let mut tested = 0;
    for b in all_blocks().iter().filter(|b| b.lang == "logos") {
        let program = make_program(&b.body);
        match compile_to_rust(&program) {
            // ★ `Ok` PROVES NOTHING HERE, and this test's NAME is the claim it was failing to
            // make. `compile_to_rust` returns `Ok` for a program the backend has REFUSED — the
            // refusal is a `compile_error!` embedded in the returned source, which is well-formed
            // Rust that fails on purpose. So an example the compiler declines to build counted as
            // an example that compiles, in the one lock whose whole job is to say otherwise.
            Ok(rust) if rust.contains("compile_error!") => failures.push(format!(
                "{}:{} — REFUSED: {}",
                b.file,
                b.line,
                refusal_message(&rust)
            )),
            Ok(_) => tested += 1,
            Err(e) => failures.push(format!("{}:{} — {e:?}", b.file, b.line)),
        }
    }
    assert!(tested > 0, "expected to find logos doc examples");
    assert!(
        failures.is_empty(),
        "{} logos doc example(s) failed to compile:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Every ```output block, paired with the ```logos block it follows.
fn outputs() -> Vec<(Block, Block)> {
    let mut pairs = Vec::new();
    for file in doc_files() {
        let blocks = fenced_blocks(&file);
        for w in blocks.windows(2) {
            if w[1].lang == "output" {
                assert!(
                    w[0].lang == "logos" && w[1].follows_previous,
                    "{}:{} — an ```output block must directly follow the ```logos block whose output it is",
                    w[1].file,
                    w[1].line
                );
                pairs.push((
                    Block {
                        file: w[0].file.clone(),
                        line: w[0].line,
                        lang: w[0].lang.clone(),
                        body: w[0].body.clone(),
                        follows_previous: w[0].follows_previous,
                    },
                    Block {
                        file: w[1].file.clone(),
                        line: w[1].line,
                        lang: w[1].lang.clone(),
                        body: w[1].body.clone(),
                        follows_previous: w[1].follows_previous,
                    },
                ));
            }
        }
        if let Some(first) = blocks.first() {
            assert!(
                first.lang != "output",
                "{}:{} — an ```output block with no program above it",
                first.file,
                first.line
            );
        }
    }
    pairs
}

#[test]
fn doc_logos_outputs_match() {
    let mut failures = Vec::new();
    let pairs = outputs();
    assert!(
        !pairs.is_empty(),
        "expected ```output blocks in the documentation"
    );
    for (program, shown) in &pairs {
        let run = tw_outcome(&make_program(&program.body));
        let printed = run.output.trim_end().to_string();
        let claimed = shown.body.trim_end().to_string();
        match run.error {
            Some(e) => failures.push(format!("{}:{} — the program fails: {e}", program.file, program.line)),
            None if printed != claimed => failures.push(format!(
                "{}:{} — it prints:\n{printed}\n  and the documentation shows (at line {}):\n{claimed}",
                program.file, program.line, shown.line
            )),
            None => {}
        }
    }
    assert!(
        failures.is_empty(),
        "{} documented output(s) are not what the program prints:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn doc_logos_tests_pass() {
    let mut failures = Vec::new();
    let mut ran = 0;
    for b in all_blocks()
        .iter()
        .filter(|b| b.lang == "logos" && b.body.contains("## Test"))
    {
        match run_tests(&make_program(&b.body), &TestConfig::default()) {
            Err(e) => failures.push(format!(
                "{}:{} — the tests do not parse: {e}",
                b.file, b.line
            )),
            Ok(report) => {
                if report.outcomes.is_empty() {
                    failures.push(format!(
                        "{}:{} — holds `## Test` but no test ran",
                        b.file, b.line
                    ));
                }
                for o in &report.outcomes {
                    ran += 1;
                    if o.verdict != Verdict::Passed {
                        failures.push(format!(
                            "{}:{} `{}` — {}",
                            b.file,
                            b.line,
                            o.name,
                            o.failures.join("; ")
                        ));
                    }
                }
            }
        }
    }
    assert!(
        ran > 0,
        "expected `## Test` blocks in the documentation's logos examples"
    );
    assert!(
        failures.is_empty(),
        "{} documented test(s) fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn doc_no_placeholder_browser_examples() {
    let bad: Vec<String> = all_blocks()
        .iter()
        .filter(|b| b.lang == "logos-dom" && b.body.contains("..."))
        .map(|b| format!("{}:{}", b.file, b.line))
        .collect();
    assert!(
        bad.is_empty(),
        "logos-dom doc examples must be real, not `...` schematics: {bad:?}"
    );
}

#[test]
fn doc_fol_pairs_match() {
    let mut failures = Vec::new();
    let mut tested = 0;
    for b in all_blocks().iter().filter(|b| b.lang.is_empty()) {
        for (english, expected) in fol_pairs(&b.body) {
            match compile(&english) {
                Ok(got) if norm(&got) == norm(&expected) => tested += 1,
                Ok(got) => failures.push(format!(
                    "{}:{} {english:?}\n   want: {expected}\n   got:  {got}",
                    b.file, b.line
                )),
                Err(e) => failures.push(format!("{}:{} {english:?} — {e:?}", b.file, b.line)),
            }
        }
    }
    assert!(tested > 0, "expected to find English→FOL doc pairs");
    assert!(
        failures.is_empty(),
        "{} doc FOL pair(s) drifted from the compiler:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Every ```rust block that directly follows a ```logos block, paired with it.
fn rust_pairs() -> Vec<(Block, Block)> {
    let mut pairs = Vec::new();
    for file in doc_files() {
        let mut blocks = fenced_blocks(&file).into_iter().peekable();
        while let Some(program) = blocks.next() {
            if program.lang != "logos" {
                continue;
            }
            if let Some(next) = blocks.peek() {
                if next.lang == "rust" && next.follows_previous {
                    let shown = blocks.next().unwrap();
                    pairs.push((program, shown));
                }
            }
        }
    }
    pairs
}

#[test]
fn doc_logos_rust_pairs_match() {
    let pairs = rust_pairs();
    assert!(
        !pairs.is_empty(),
        "the README shows a LOGOS function next to the Rust it compiles to"
    );
    let mut failures = Vec::new();
    for (program, shown) in &pairs {
        match compile_to_rust(&make_program(&program.body)) {
            Ok(rust) if norm(&rust).contains(&norm(&shown.body)) => {}
            Ok(rust) => failures.push(format!(
                "{}:{} — the Rust shown at line {} is not what the compiler emits. It emits:\n{rust}",
                program.file, program.line, shown.line
            )),
            Err(e) => failures.push(format!("{}:{} — {e:?}", program.file, program.line)),
        }
    }
    assert!(
        failures.is_empty(),
        "{} documented translation(s) drifted:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// Every `"english"` line in an untagged block followed directly by numbered `1. …` reading lines.
fn fol_readings(body: &str) -> Vec<(String, Vec<String>)> {
    let lines: Vec<&str> = body.lines().map(str::trim).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.starts_with('"') && line.ends_with('"') && line.len() > 2 {
            let mut readings = Vec::new();
            let mut j = i + 1;
            while let Some(rest) = lines
                .get(j)
                .and_then(|l| l.strip_prefix(&format!("{}. ", readings.len() + 1)))
            {
                readings.push(rest.to_string());
                j += 1;
            }
            if !readings.is_empty() {
                out.push((line[1..line.len() - 1].to_string(), readings));
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

#[test]
fn doc_fol_readings_match() {
    let mut failures = Vec::new();
    let mut found = 0;
    for b in all_blocks().iter().filter(|b| b.lang.is_empty()) {
        for (english, shown) in fol_readings(&b.body) {
            found += 1;
            match compile_all_scopes(&english) {
                Ok(got)
                    if got
                        .iter()
                        .map(|g| norm(g))
                        .eq(shown.iter().map(|s| norm(s))) => {}
                Ok(got) => failures.push(format!(
                    "{}:{} {english:?}\n   shows: {shown:#?}\n   gets:  {got:#?}",
                    b.file, b.line
                )),
                Err(e) => failures.push(format!("{}:{} {english:?} — {e:?}", b.file, b.line)),
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} documented reading list(s) drifted from the compiler:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        found > 0,
        "the logic guide shows a sentence with every reading"
    );
}

#[test]
fn numbered_readings_are_read_as_one_list() {
    let body = "\"Every woman loves a man.\"\n1. A\n2. B\n\n\"Not a list.\"\nprose\n";
    assert_eq!(
        fol_readings(body),
        vec![(
            "Every woman loves a man.".to_string(),
            vec!["A".to_string(), "B".to_string()]
        )]
    );
    assert!(
        fol_pairs(body).is_empty(),
        "a reading list is not an arrow pair"
    );
}
