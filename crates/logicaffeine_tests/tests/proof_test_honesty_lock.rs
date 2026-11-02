//! TEST HONESTY — no assertion in this suite may be incapable of failing.
//!
//! Tests are the IP. An assertion that cannot fail is not a test; it is a comment that costs CPU
//! time and buys false confidence: `result.is_err() || result.is_ok()` holds for every result, so
//! a suite that asserts it proves nothing about the code under test.
//!
//! Four shapes are banned outright:
//!
//!   `assert!(r.is_ok() || r.is_err())`   — true for every `Result` ever constructed
//!   `assert!(o.is_some() || o.is_none())` — the same thing in the `Option` spelling
//!   `assert!(!v.is_empty() || v.is_empty())` — `!P || P`, with the complement spelled by hand
//!   `assert!(v.len() + w.len() >= 0)`    — an unsigned count is never negative
//!   `let _ = result;`                     — asserts nothing at all
//!
//! The first was the only one the lock knew until wave 3. The other three were all live in this
//! workspace: `phase_hw_type_infer.rs:184`, `analysis/check.rs:1593` (whose comment reads
//! `// just don't panic`) and `phase_hw_cdc.rs:174`, in a test named `cdc_report_domains_listed`.
//! One detector, one spelling, is a detector that protects one spelling.
//!
//! Neither even achieves the "at least it doesn't panic" goal their comments claim: a panic inside
//! the call under test propagates and fails the test *before* reaching the assertion, so the
//! assertion contributes exactly nothing in every possible execution.
//!
//! When two outcomes are genuinely both acceptable, discriminate and still assert something
//! falsifiable — the shape `proof_nonvacuity_lock.rs` already uses:
//!
//! ```ignore
//! match result {
//!     Err(_) => {}                                    // a hard error is an acceptable signal
//!     Ok(r)  => assert!(r.failed > 0, "...: {r:?}"),  // but the Ok arm must still prove something
//! }
//! ```
//!
//! ## Why this is a ratchet and not a hard zero
//!
//! Eleven sites predate this lock. Each one's *intended* assertion is knowable only by running the
//! code — most say "this should fail" in a comment and then hedge, because the author did not know
//! what the behaviour actually was. Replacing a vacuous assertion with a *guessed* one is worse
//! than leaving it, so they are grandfathered here, counted, and driven down deliberately. What the
//! lock buys immediately is that the count can never **rise**.
//!
//!  ⚠️  YOU DO NOT GET TO ADD TO THE GRANDFATHER LIST.  ⚠️
//!  It is a record of debt incurred before the lock existed, not a place to put new debt.
//!
//! ## What the lock can SEE, and why that is half the job
//!
//! A detector that never reads a file protects nothing there — the same hole as a detector that
//! never fires, and harder to notice. Two populations are scanned:
//!
//! * every `.rs` file under a crate's `tests/`, **at any depth**. The one-level walk this replaced
//!   could not see `tests/common/mod.rs` and its fifteen siblings — shared harness code every suite
//!   in the crate runs through, and therefore the best place in the workspace to hide an assertion
//!   that cannot fail;
//! * every `src/` file's `#[cfg(test)]` **regions** — 511 blocks across 443 files, none of which
//!   this lock used to see. Only the regions: outside one, `let _ = result;` is a legitimate
//!   production idiom for a deliberately discarded `Result`, not a missing assertion.
//!
//! Measured before widening: zero vacuous sites in either population. The blind spot was real and
//! unexploited, and it is now shut.
//!
//! ## Why widening a DETECTOR needed its own ledger
//!
//! `VACUOUS_CEILING` is one number over all shapes, so writing a better detector and writing a
//! worse test are the same event to it. Wave 2 found `phase_hw_cdc.rs`'s `usize + usize >= 0` BY
//! HAND and deliberately declined to wire a detector, because doing so would have taken the count
//! past the ceiling on a site no lane was permitted to edit. That was a correct reading of the
//! rules and exactly the wrong outcome: a ratchet that pays you to keep the lock blind will be
//! obeyed by not looking. So debt a wider detector surfaces is counted in its own ledger,
//! `SURFACED_BY_A_WIDER_DETECTOR`, which is stricter than the grandfather list in both directions —
//! keyed on the exact source line, and each entry carries the assertion that retires it.
//!
//! ## One lexer, four detectors — and two views of every line
//!
//! Every detector reads `code_only`, a carried-state scanner that blanks comments, string literals,
//! char literals and raw strings — across lines, so a multi-line string and a `/* … */` can neither
//! hide a violation nor let a neighbouring line read as text it is not. Before it, each detector
//! made its own guess: a `split("//")` that truncates at a `//` inside a URL, and an "is there a
//! quote to my left" test that read `assert!(x, "a"); assert!(true);` as clean and
//! `contains("say \"assert!(true)\"")` as a violation — wrong in both directions at once.
//!
//! That is not a hypothetical. `is_assert_true` was broken in BOTH directions, once each: a false
//! positive on `assert!(rust.contains("debug_assert!(true)"))`, then a false negative on every
//! `assert!(true, "<plausible message>")` — the exact shape someone writes to make a vacuous
//! assertion look deliberate — because the needle carried a closing paren. The guard self-test sat
//! RED and this ratchet protected nothing, while the matrix above it read green. Both directions
//! are now pinned by their own tests, with the neighbours one rung either side.
//!
//! Blanking a literal is also what makes the lexer BLIND to the one question a receiver-aware
//! detector must ask. `!host.contains("TT_METAL_HOME") || host.contains("getenv")` — a live,
//! falsifiable assertion — blanks to `!host.contains()||host.contains()`, two calls made
//! indistinguishable by the very step that made the line safe to read. So there are two views of
//! every line, from one state machine: `code_only` (literals blanked) decides whether a shape is
//! CODE, and `without_comments` (literals intact) decides whether two sides are the SAME
//! EXPRESSION. Using either alone is wrong in a measured direction — three real assertions flagged,
//! or `map.get("a").is_none() || map.get("b").is_some()` called a tautology.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Sites that predate this lock, with the file they live in. Entries may be REMOVED as they are
/// fixed (and `VACUOUS_CEILING` lowered); none may be added.
const GRANDFATHERED: &[(&str, usize, &str)] = &[
    // `assert!(result.is_err() || result.is_ok())` — tautologies.
    ("crates/logicaffeine_tests/tests/phase101a_poly_inductive.rs", 1, "empty parameter list: the author never decided whether `Inductive List ()` should parse"),
    ("crates/logicaffeine_tests/tests/phase_literate_lia.rs", 2, "both comments say \"This should fail\"; needs a run to confirm lia rejects before asserting is_err"),
    ("crates/logicaffeine_tests/tests/phase_literate_cc.rs", 1, "`f x = g x` is not a congruence; needs a run to confirm cc refuses"),
    ("crates/logicaffeine_tests/tests/phase_literate_ring.rs", 1, "`x equals y` is not a ring identity; comment says it should fail"),
    ("crates/logicaffeine_tests/tests/phase43_discourse_scope.rs", 1, "doc comment says empty discourse SHOULD SUCCEED — likely a plain is_ok"),
    ("crates/logicaffeine_tests/tests/phase54_concurrency.rs", 1, "`Stop handle` on a LaunchTask; the author flagged LaunchTaskWithHandle as unparsed"),
    // `let _ = result;` — discarded outcomes.
    ("crates/logicaffeine_tests/tests/phase_hints.rs", 1, "doc comment and inline comment contradict each other on whether a hint without a proof registers"),
    ("crates/logicaffeine_tests/tests/phase_literate_induction.rs", 2, "the file's premise is FALSE — it claims induction is unimplemented; the real defect is that `build_induction_derivation` never SLam-abstracts its motive, so literate induction can never succeed. Fix that first, then assert."),
    ("crates/logicaffeine_tests/tests/phase_verification_refinement.rs", 1, "author did not know whether refinement checking propagates through arithmetic"),
];

/// Total vacuous assertions of the three shapes this lock has always seen. 2026-09-14: 11, all
/// predating this lock. Only ever lower it.
///
/// It counts the ORIGINAL shapes on purpose. Debt a newly-written detector discovers is counted in
/// `SURFACED_CEILING` below, because "we got worse" and "we can see more" are different facts and a
/// single number cannot tell them apart — see the note there.
const VACUOUS_CEILING: usize = 11;

/// Debt that was always there and that the lock could not SEE until a detector was widened.
///
/// ## Why this is not the grandfather list with a new name
///
/// The list above is a record of debt incurred before the lock existed, and adding to it is banned.
/// This one exists because of a defect in the MECHANISM, found in wave 3: with one ceiling over all
/// shapes, writing a better detector is indistinguishable from writing a worse test. Wave 2 found
/// `phase_hw_cdc.rs`'s `usize + usize >= 0` by hand, and deliberately did NOT wire a detector for
/// it — a correct reading of the rules, and exactly the wrong outcome, because the effect is that
/// the ratchet pays you to keep the lock blind. A mechanism that punishes looking harder will be
/// obeyed by not looking.
///
/// So the two are separated, and this one is the STRICTER of the two: an entry is keyed by the
/// exact source text (not a count, and not a line number, which drifts), and each must name the
/// assertion that retires it. The eleven originals are grandfathered precisely because their right
/// assertion was NOT knowable without running the code; every entry here was settled by reading the
/// implementation, so there is no excuse for one to sit unfixed. Removing an entry is a one-line
/// edit in the offending file plus lowering `SURFACED_CEILING`.
///
///  ⚠️  An entry may be added ONLY in the same change that adds the detector which found it.  ⚠️
///  Debt a detector that already existed could have caught is not "surfaced"; it is new, and
///  `no_new_vacuous_assertion_enters_the_suite` is the test that says so.
struct Surfaced {
    file: &'static str,
    /// The offending source line, trimmed — the entry's identity. Line numbers drift under eleven
    /// concurrent editors; this does not, and it names WHICH assertion when a file has two.
    text: &'static str,
    shape: &'static str,
    /// The assertion that replaces it, written out and ready to paste. Not a sketch: each was read
    /// off the implementation.
    honest: &'static str,
}

const SURFACED_BY_A_WIDER_DETECTOR: &[Surfaced] = &[
    Surfaced {
        file: "crates/logicaffeine_tests/tests/phase_hw_cdc.rs",
        text: "assert!(report.crossings.len() + report.violations.len() >= 0);",
        shape: SHAPE_UNSIGNED,
        honest: "let crossing = report.crossings.iter().find(|c| c.signal == \"a_x\").expect(\"`a_x` crosses clk_a to clk_b and must be reported\"); assert_eq!((crossing.source_domain.as_str(), crossing.dest_domain.as_str()), (\"a\", \"b\"), \"the report must name the domains a crossing runs between: {report:?}\");",
    },
    Surfaced {
        file: "crates/logicaffeine_tests/tests/phase_hw_type_infer.rs",
        text: "assert!(types.get(\"x\").is_none() || types.get(\"x\").is_some());",
        shape: SHAPE_PAIR,
        honest: "assert!(types.get(\"x\").is_none(), \"a variable with no constraint must not appear in the inferred map: {types:?}\");",
    },
];

/// Sites the wider detectors surfaced. 2026-09-15: 3; 2026-09-27: 2 (check.rs's assertion is gone). Only
/// ever lower it.
const SURFACED_CEILING: usize = 2;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every `.rs` file under a directory, at any depth. `target`, `node_modules` and dot-directories
/// are skipped; nothing else is.
fn walk_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name == "target" || name == "node_modules" || name.starts_with('.') {
                continue;
            }
            walk_rs(&p, out);
        } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
            out.push(p);
        }
    }
}

/// Every integration-test file in the workspace, **at any depth** under a crate's `tests/`.
///
/// The one-level-deep walk this replaced could not see `tests/common/mod.rs` and its fifteen
/// siblings — shared harness code every suite in the crate runs through, and therefore the single
/// best place in the workspace to hide an assertion that cannot fail. Measured 2026-09-15 before
/// widening: sixteen such files, zero vacuous sites in them. The blind spot was real and
/// unexploited; it is now shut.
fn test_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for crate_dir in ["crates", "apps"] {
        let base = root().join(crate_dir);
        let Ok(entries) = fs::read_dir(&base) else {
            continue;
        };
        for e in entries.flatten() {
            walk_rs(&e.path().join("tests"), &mut out);
        }
    }
    out.sort();
    out
}

/// Every `src/` file carrying a `#[cfg(test)]`. Unit tests live in these and the lock never saw
/// one: 511 such blocks across 443 files, measured 2026-09-15. Only the `#[cfg(test)]` REGIONS of
/// these files are scanned — `let _ = result;` is a legitimate production idiom for a deliberately
/// discarded `Result`, and it is only vacuous when it is standing in for an assertion.
fn unit_test_sources() -> Vec<PathBuf> {
    let mut all = Vec::new();
    for crate_dir in ["crates", "apps"] {
        let base = root().join(crate_dir);
        let Ok(entries) = fs::read_dir(&base) else {
            continue;
        };
        for e in entries.flatten() {
            walk_rs(&e.path().join("src"), &mut all);
        }
    }
    all.retain(|p| {
        fs::read_to_string(p)
            .map(|t| t.contains("#[cfg(test)]"))
            .unwrap_or(false)
    });
    all.sort();
    all
}

fn rel(p: &Path) -> String {
    p.strip_prefix(root())
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

// ============================================================================================
// ONE LEXER, TWO VIEWS, FOUR DETECTORS — a guard that cannot tell code from a string literal fires
// on its own documentation, a guard that cannot see into a `/* … */` misses whatever hides there,
// and a guard that reads ONLY the blanked line cannot tell `map.get("a")` from `map.get("b")`.
// ============================================================================================

/// What the scanner is in the middle of when a line ends. Carried across lines, because a Rust
/// string literal and a block comment both span them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lex {
    Code,
    Str,
    /// `r##"…"##` — the `usize` is the hash count that must close it.
    RawStr(usize),
    Block,
}

/// The CODE of one line: every comment, string literal and char literal blanked to spaces, the
/// same length in characters so nothing shifts.
///
/// This is what every detector below actually reads, and it is why they cannot disagree about
/// what is code. It replaces three separate ad-hoc guesses — a `split("//")`, which truncates a
/// line at a `//` inside a URL string; and a "is there a quote to my left" test, which reads
/// `assert!(x, "a"); assert!(true);` as safe and `assert!(c("say \"assert!(true)\""))` as a
/// violation, i.e. wrong in both directions at once.
fn code_only(line: &str, state: &mut Lex) -> String {
    scan(line, state, false)
}

/// The same line with COMMENTS blanked and string literals LEFT ALONE.
///
/// Blanking a literal is what makes `code_only` safe, and it is also what makes it blind to the
/// one question a receiver-aware detector has to ask: are these two calls the same call?
/// `!host.contains("TT_METAL_HOME") || host.contains("getenv")` — a real implication, and a live
/// assertion in `spatial_tensix_lowering_lock.rs` — blanks to `!host.contains()||host.contains()`,
/// in which the two receivers are indistinguishable. Measured: a receiver-aware detector reading
/// the blanked line flags it, and two of its neighbours, as tautologies. They are not.
///
/// So the two views are used together and each answers only what it can: the blanked line decides
/// whether a shape is CODE (a literal quoting `is_ok() || is_err()` is documentation), and this one
/// decides whether the two sides are the SAME EXPRESSION. Comments are blanked in both, so a
/// comment can never supply either half.
fn without_comments(line: &str, state: &mut Lex) -> String {
    scan(line, state, true)
}

/// One state machine, two outputs. The transitions are identical — a `//` inside a string is not a
/// comment either way — so the only difference is whether a character inside a literal is emitted
/// or blanked. Writing it twice would be two lexers that can disagree about where a string ends,
/// which is the exact class of bug this file was built to stop.
fn scan(line: &str, state: &mut Lex, keep_literals: bool) -> String {
    let c: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(c.len());
    let mut i = 0usize;
    let blank = |out: &mut String, n: usize| {
        for _ in 0..n {
            out.push(' ');
        }
    };
    // A literal's characters: blanked, or copied through when the caller wants to read them.
    // Always the same COUNT, so a column never shifts between the two views of one line.
    let lit = |out: &mut String, c: &[char], at: usize, n: usize| {
        for k in 0..n {
            out.push(if keep_literals {
                *c.get(at + k).unwrap_or(&' ')
            } else {
                ' '
            });
        }
    };
    while i < c.len() {
        match *state {
            Lex::Block => {
                if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    blank(&mut out, 2);
                    *state = Lex::Code;
                    i += 2;
                } else {
                    blank(&mut out, 1);
                    i += 1;
                }
            }
            Lex::Str => {
                if c[i] == '\\' {
                    let n = (c.len() - i).min(2);
                    lit(&mut out, &c, i, n);
                    i += 2;
                } else {
                    lit(&mut out, &c, i, 1);
                    if c[i] == '"' {
                        *state = Lex::Code;
                    }
                    i += 1;
                }
            }
            Lex::RawStr(h) => {
                let closes =
                    c[i] == '"' && i + h < c.len() && c[i + 1..=i + h].iter().all(|x| *x == '#');
                if closes {
                    lit(&mut out, &c, i, h + 1);
                    *state = Lex::Code;
                    i += h + 1;
                } else {
                    lit(&mut out, &c, i, 1);
                    i += 1;
                }
            }
            Lex::Code => {
                match c[i] {
                    '/' if c.get(i + 1) == Some(&'/') => {
                        blank(&mut out, c.len() - i);
                        return out;
                    }
                    '/' if c.get(i + 1) == Some(&'*') => {
                        blank(&mut out, 2);
                        *state = Lex::Block;
                        i += 2;
                    }
                    '"' => {
                        lit(&mut out, &c, i, 1);
                        *state = Lex::Str;
                        i += 1;
                    }
                    'r' | 'b' => {
                        // `r"…"`, `r#"…"#`, `br"…"`, `br#"…"#` — but only when `r` starts a token,
                        // so the `r` in `other#` or in an identifier never opens a raw string.
                        let starts_token = i == 0 || {
                            let prev = c[i - 1];
                            !(prev.is_alphanumeric() || prev == '_')
                        };
                        let mut j = i;
                        if c[j] == 'b' {
                            j += 1;
                        }
                        let opens = starts_token && c.get(j) == Some(&'r') && {
                            let mut k = j + 1;
                            while c.get(k) == Some(&'#') {
                                k += 1;
                            }
                            c.get(k) == Some(&'"')
                        };
                        if opens {
                            let mut k = j + 1;
                            let mut hashes = 0usize;
                            while c.get(k) == Some(&'#') {
                                hashes += 1;
                                k += 1;
                            }
                            lit(&mut out, &c, i, k + 1 - i);
                            *state = Lex::RawStr(hashes);
                            i = k + 1;
                        } else {
                            out.push(c[i]);
                            i += 1;
                        }
                    }
                    '\'' => {
                        // A char literal — `'a'`, `'\n'`, `'"'` — and NOT a lifetime (`&'a str`),
                        // which has no closing quote where a char literal's would be.
                        let ch_lit = if c.get(i + 1) == Some(&'\\') {
                            (i + 2..c.len().min(i + 8))
                                .find(|k| c[*k] == '\'')
                                .map(|k| k + 1 - i)
                        } else if c.get(i + 2) == Some(&'\'') {
                            Some(3)
                        } else {
                            None
                        };
                        match ch_lit {
                            Some(n) => {
                                lit(&mut out, &c, i, n);
                                i += n;
                            }
                            None => {
                                out.push(c[i]);
                                i += 1;
                            }
                        }
                    }
                    ch => {
                        out.push(ch);
                        i += 1;
                    }
                }
            }
        }
    }
    out
}

/// Every line of `text` as CODE, one-based, with lexer state carried across lines.
fn code_lines(text: &str) -> Vec<(usize, String)> {
    let mut state = Lex::Code;
    text.lines()
        .enumerate()
        .map(|(i, l)| (i + 1, code_only(l, &mut state)))
        .collect()
}

/// The same lines with comments gone and literals intact — the second view the receiver-aware
/// detector needs. Same lexer, same state machine, so the two can never disagree about where a
/// string ends; only about whether its contents are readable.
fn comment_free_lines(text: &str) -> Vec<String> {
    let mut state = Lex::Code;
    text.lines()
        .map(|l| without_comments(l, &mut state))
        .collect()
}

/// The one-based line numbers inside a `#[cfg(test)]` item — the region of a `src/` file that is
/// test code. Everything outside it is production code, where a discarded `Result` is an idiom
/// rather than a missing assertion.
fn cfg_test_regions(code: &[(usize, String)]) -> BTreeSet<usize> {
    const ATTR: &str = "#[cfg(test)]";
    let mut inside = BTreeSet::new();
    let mut armed = false;
    let mut depth = 0usize;
    for (n, line) in code {
        if depth > 0 {
            inside.insert(*n);
        }
        // Scanning starts AFTER the attribute when it is on this line, so the common one-liner
        // `#[cfg(test)] mod tests { … }` opens on the same line it is announced on.
        let mut from = 0usize;
        if depth == 0 && !armed {
            let Some(at) = line.find(ATTR) else { continue };
            armed = true;
            from = line[..at].chars().count() + ATTR.chars().count();
        }
        for ch in line.chars().skip(from) {
            if depth == 0 {
                if !armed {
                    continue;
                }
                match ch {
                    '{' => {
                        depth = 1;
                        inside.insert(*n);
                    }
                    // `#[cfg(test)] use super::*;` attaches to an item with no block at all.
                    ';' => armed = false,
                    _ => {}
                }
            } else {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            armed = false;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    inside
}

/// The predicate pairs that PARTITION a type: whichever value the receiver holds, exactly one of
/// the two is true, so their disjunction is true for every value that can ever reach the line.
/// `is_empty` is the odd one out — its complement has no name, so its dual is its own negation.
const EXHAUSTIVE_PAIRS: &[(&str, &str)] = &[
    ("is_ok", "is_err"),
    ("is_err", "is_ok"),
    ("is_some", "is_none"),
    ("is_none", "is_some"),
];

/// The expression that ENDS at byte offset `end`, read backwards with balanced brackets.
///
/// `types.get("x").is_none()` at the `.is_none()` gives back `types.get("x")` — the call's
/// arguments included, which is the entire point: they are what tells two calls apart.
fn expression_start(s: &str, end: usize) -> usize {
    let mut depth = 0usize;
    let mut start = end;
    for (idx, ch) in s[..end].char_indices().rev() {
        if depth > 0 {
            match ch {
                ')' | ']' => depth += 1,
                '(' | '[' => depth -= 1,
                _ => {}
            }
            start = idx;
            continue;
        }
        match ch {
            ')' | ']' => {
                depth += 1;
                start = idx;
            }
            c if c.is_alphanumeric() || c == '_' || c == '.' => start = idx,
            _ => break,
        }
    }
    start
}

/// A disjunction of a predicate and its exhaustive complement over the SAME receiver — true for
/// every value that receiver can hold, in every possible execution.
///
/// TWO VIEWS, because neither alone can answer the question:
///
/// * `code` — literals blanked — decides whether the shape is CODE at all. A test matching
///   generated source for the string `"result.is_ok() || result.is_err()"` is documentation.
/// * `literals` — literals intact, comments gone — decides whether the two sides are the SAME
///   EXPRESSION. This half is not optional and it is not defensive. MEASURED 2026-09-15: reading
///   the blanked line instead flags `!host.contains("TT_METAL_HOME") || host.contains("getenv")`
///   (spatial_tensix_lowering_lock.rs:149) and two neighbours as tautologies, because blanking
///   turns two different calls into `host.contains()` twice. They are real implications. The
///   inverse is just as real: `map.get("a").is_none() || map.get("b").is_some()` is falsifiable and
///   blanks to a tautology.
///
/// A caller holding only one view — a self-test with a raw line — passes it as both.
fn is_exhaustive_pair(code: &str, literals: &str) -> bool {
    let c = code.replace([' ', '\t'], "");
    let l = literals.replace([' ', '\t'], "");

    for (a, b) in EXHAUSTIVE_PAIRS {
        let head = format!(".{a}()||");
        let tail = format!(".{b}()");
        if !(c.contains(&head) && c.contains(&tail)) {
            continue;
        }
        let mut from = 0usize;
        while let Some(rel) = l[from..].find(&head) {
            let i = from + rel;
            let recv = &l[expression_start(&l, i)..i];
            let after = i + head.len();
            if !recv.is_empty() && l[after..].starts_with(&format!("{recv}{tail}")) {
                return true;
            }
            from = i + head.len();
        }
    }

    // `X.is_empty() || !X.is_empty()`, either way round. The complement is spelled with `!`, so the
    // receiver has to match ACROSS the negation — the half that separates it from
    // `!a.is_empty() || b.is_empty()`, which is a perfectly good implication.
    const EMPTY_HEAD: &str = ".is_empty()||";
    const EMPTY: &str = ".is_empty()";
    if c.matches(EMPTY).count() >= 2 && c.contains(EMPTY_HEAD) {
        let mut from = 0usize;
        while let Some(rel) = l[from..].find(EMPTY_HEAD) {
            let i = from + rel;
            let start = expression_start(&l, i);
            let recv = &l[start..i];
            let negated_left = start > 0 && l[..start].ends_with('!');
            let right = &l[i + EMPTY_HEAD.len()..];
            let same = |r: &str| !recv.is_empty() && r.starts_with(&format!("{recv}{EMPTY}"));
            if negated_left && same(right) {
                return true;
            }
            if !negated_left && right.starts_with('!') && same(&right[1..]) {
                return true;
            }
            from = i + EMPTY_HEAD.len();
        }
    }
    false
}

/// A line asserting `is_ok() || is_err()` (either order) — true for every `Result`.
///
/// Kept as the one-view door for callers that hold a raw line, and delegating so there is exactly
/// one implementation of "is this an exhaustive pair". The name is the one the grandfather list and
/// four self-tests were written against.
fn is_tautology(line: &str) -> bool {
    is_exhaustive_pair(line, line)
}

/// A comparison an unsigned count satisfies for every possible input — `v.len() >= 0`, and the sum
/// of any number of such counts, which is the shape that hides best because it looks like
/// arithmetic. `usize + usize >= 0` is true for every state the program can reach, so the line
/// tests nothing at all while reading like a bounds check.
///
/// Deliberately narrow, and each restriction is load-bearing:
///
/// * every term must END in `.len()` or `.count()`. `(a.len() as i64 - b.len() as i64) >= 0` is a
///   REAL check — the cast makes the difference signed — and it ends in `)`, so it is refused.
/// * only `+` joins terms. Subtraction of unsigned counts is a different defect (it panics on
///   underflow before the assertion runs) and deserves its own finding, not this one's.
/// * the comparison must END the expression — `)`, `,`, `;`, `}` or end of line. `v.len() >= 0 + k`
///   is falsifiable, and a detector that cannot see the `+ k` would call it vacuous.
fn is_unsigned_ge_zero(line: &str) -> bool {
    let l: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    let closes = |rest: &str| matches!(rest.chars().next(), None | Some(')' | ',' | ';' | '}'));
    let opens = |head: &str| matches!(head.chars().next_back(), None | Some('(' | ',' | '!'));

    let mut from = 0usize;
    while let Some(rel) = l[from..].find(">=0") {
        let i = from + rel;
        if closes(&l[i + 3..]) && is_count_chain(&l, i) {
            return true;
        }
        from = i + 1;
    }
    let mut from = 0usize;
    while let Some(rel) = l[from..].find("0<=") {
        let i = from + rel;
        if opens(&l[..i]) && is_count_chain_forward(&l[i + 3..]) {
            return true;
        }
        from = i + 1;
    }
    false
}

/// `a.len() + b.count() + …`, read backwards from `end`.
fn is_count_chain(l: &str, end: usize) -> bool {
    let mut e = end;
    loop {
        let s = expression_start(l, e);
        let term = &l[s..e];
        if !(term.ends_with(".len()") || term.ends_with(".count()")) {
            return false;
        }
        if s > 0 && l[..s].ends_with('+') {
            e = s - 1;
            continue;
        }
        return true;
    }
}

/// The same chain read FORWARDS, for the `0 <= …` spelling. It ends where the enclosing call does.
fn is_count_chain_forward(rest: &str) -> bool {
    let end = rest
        .char_indices()
        .scan(0usize, |depth, (i, ch)| {
            match ch {
                '(' | '[' => *depth += 1,
                ')' | ']' if *depth == 0 => return Some((i, true)),
                ')' | ']' => *depth -= 1,
                ',' | ';' if *depth == 0 => return Some((i, true)),
                _ => {}
            }
            Some((i, false))
        })
        .find(|(_, stop)| *stop)
        .map(|(i, _)| i)
        .unwrap_or(rest.len());
    let chain = &rest[..end];
    !chain.is_empty()
        && chain
            .split('+')
            .all(|t| t.ends_with(".len()") || t.ends_with(".count()"))
}

/// A discarded `Result` the test never inspects.
fn is_discarded_result(line: &str) -> bool {
    let l = line.trim();
    l == "let _ = result;" || l.starts_with("let _ = result;")
}

/// `assert!(true)` as an actual assertion — not `debug_assert!(true)`, and not the same text
/// appearing inside a string literal a test is matching against generated source.
///
/// Two guards separate a violation from its lookalikes, and the detector has been broken in both
/// directions once each, which is why both are pinned by self-tests:
///
/// * the **message form**. `assert!(true, "why")` is exactly as vacuous as `assert!(true)`, and is
///   the shape someone writes to make a vacuous assertion look deliberate. Searching for the
///   closing paren — `"assert!(true)"` — missed every one of them and left this ratchet protecting
///   nothing. The needle stops at `true` and the character after it must close the argument.
/// * the **boundary**. A real call has no identifier character before `assert!`, so
///   `debug_assert!(true)` and `my_assert!(true)` are not violations.
///
/// The third guard, **a quote to the left means a string literal**, is kept although `code_only`
/// has already made it redundant — a blanked line contains no quote at all, which
/// `the_quote_guard_is_redundant_after_the_lexer` pins. It stays because it is what makes this
/// predicate safe to call on a RAW line, and a guard that is only correct when its caller
/// remembers to pre-process is the kind of guard that silently stops working. On a raw line it is
/// imprecise in the safe direction: it reads `assert!(x, "a"); assert!(true);` as clean, a miss the
/// lexer does not make.
fn is_assert_true(line: &str) -> bool {
    const NEEDLE: &str = "assert!(true";
    let l = line.replace(' ', "");
    let bytes = l.as_bytes();
    l.match_indices(NEEDLE).any(|(i, _)| {
        let closes = matches!(bytes.get(i + NEEDLE.len()), Some(b')') | Some(b','));
        let boundary_ok = i == 0 || {
            let c = bytes[i - 1] as char;
            !(c.is_alphanumeric() || c == '_')
        };
        let in_string = l[..i].contains('"');
        closes && boundary_ok && !in_string
    })
}

/// One assertion that cannot fail, and which of the shapes it is.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Vacuous {
    file: String,
    line: usize,
    text: String,
    shape: &'static str,
}

/// The shapes, by the name the ledgers and the report use.
const SHAPE_PAIR: &str = "exhaustive-pair";
const SHAPE_DISCARDED: &str = "discarded-result";
const SHAPE_ASSERT_TRUE: &str = "assert-true";
const SHAPE_UNSIGNED: &str = "unsigned-ge-zero";

/// Every vacuous site in the workspace.
///
/// Two populations are scanned. Integration-test files are scanned WHOLE, at any depth under a
/// crate's `tests/`. `src/` files are scanned only inside their `#[cfg(test)]` regions, because
/// outside one a discarded `Result` is a legitimate production idiom and not a missing assertion.
fn vacuous_sites() -> Vec<Vacuous> {
    static CACHE: OnceLock<Vec<Vacuous>> = OnceLock::new();
    CACHE.get_or_init(scan_for_vacuous_sites).clone()
}

fn scan_for_vacuous_sites() -> Vec<Vacuous> {
    let mut out = Vec::new();
    let mut consider = |path: &Path, scoped: bool| {
        let Ok(text) = fs::read_to_string(path) else {
            return;
        };
        let raw: Vec<&str> = text.lines().collect();
        let code = code_lines(&text);
        // The receiver-aware half of `is_exhaustive_pair` reads this view: same lexer, comments
        // gone, literals intact. Two calls that differ only inside their arguments are two calls.
        let kept = comment_free_lines(&text);
        let region = if scoped {
            Some(cfg_test_regions(&code))
        } else {
            None
        };
        for (n, c) in &code {
            if region.as_ref().is_some_and(|r| !r.contains(n)) {
                continue;
            }
            let l = kept.get(n - 1).map(String::as_str).unwrap_or(c);
            let shape = if is_exhaustive_pair(c, l) {
                SHAPE_PAIR
            } else if is_discarded_result(c) {
                SHAPE_DISCARDED
            } else if is_assert_true(c) {
                SHAPE_ASSERT_TRUE
            } else if is_unsigned_ge_zero(c) {
                SHAPE_UNSIGNED
            } else {
                continue;
            };
            out.push(Vacuous {
                file: rel(path),
                line: *n,
                text: raw[n - 1].trim().to_string(),
                shape,
            });
        }
    };
    for path in test_files() {
        // Do not flag this lock's own documentation of the patterns it bans.
        if rel(&path).ends_with("proof_test_honesty_lock.rs") {
            continue;
        }
        consider(&path, false);
    }
    for path in unit_test_sources() {
        consider(&path, true);
    }
    out
}

/// A site the `SURFACED_BY_A_WIDER_DETECTOR` ledger accounts for, keyed on its exact source text
/// so that a file with two vacuous lines cannot let one stand in for the other.
fn is_surfaced(v: &Vacuous) -> bool {
    SURFACED_BY_A_WIDER_DETECTOR
        .iter()
        .any(|s| s.file == v.file && s.text == v.text)
}

#[test]
fn no_new_vacuous_assertion_enters_the_suite() {
    let sites = vacuous_sites();
    let allowed: BTreeSet<&str> = GRANDFATHERED.iter().map(|(f, _, _)| *f).collect();
    let novel: Vec<String> = sites
        .iter()
        .filter(|v| !allowed.contains(v.file.as_str()) && !is_surfaced(v))
        .map(|v| format!("{}:{}  [{}]  {}", v.file, v.line, v.shape, v.text))
        .collect();

    assert!(
        novel.is_empty(),
        "a new assertion that cannot fail entered the suite. A predicate or-ed with its own \
         complement (`is_ok() || is_err()`, `is_some() || is_none()`, `x.is_empty() || \
         !x.is_empty()`) is true for every value that receiver can hold; `let _ = result` asserts \
         nothing; an unsigned count is always `>= 0`. None of them even catches a panic, since a \
         panic propagates before the assertion runs. If two outcomes are genuinely acceptable, \
         discriminate and still assert a falsifiable property of the Ok arm:\n  {}",
        novel.join("\n  ")
    );
}

#[test]
fn the_vacuous_assertion_count_only_falls() {
    let sites = vacuous_sites();
    let (surfaced, original): (Vec<_>, Vec<_>) = sites.iter().partition(|v| is_surfaced(v));
    let render = |rows: &[&Vacuous]| {
        rows.iter()
            .map(|v| format!("{}:{}  [{}]  {}", v.file, v.line, v.shape, v.text))
            .collect::<Vec<_>>()
            .join("\n  ")
    };

    let n = original.len();
    assert!(
        n <= VACUOUS_CEILING,
        "vacuous assertions rose to {n}, ceiling {VACUOUS_CEILING}:\n  {}",
        render(&original)
    );
    if n < VACUOUS_CEILING {
        eprintln!("\n  ✓ vacuous count is {n}, ceiling {VACUOUS_CEILING} — lower the ceiling to bank it.\n");
    }

    let m = surfaced.len();
    assert!(
        m <= SURFACED_CEILING,
        "sites in the surfaced ledger rose to {m}, ceiling {SURFACED_CEILING}. That ledger is a \
         record of debt a WIDER DETECTOR found, not a place to put new debt — a site a detector \
         that already existed could have caught is caught by \
         `no_new_vacuous_assertion_enters_the_suite` instead:\n  {}",
        render(&surfaced)
    );
    if m < SURFACED_CEILING {
        eprintln!("\n  ✓ surfaced count is {m}, ceiling {SURFACED_CEILING} — lower the ceiling to bank it.\n");
    }
}

/// Every entry in the surfaced ledger must still name a line that is still there and still vacuous,
/// and must still say what replaces it. This is the half that makes the ledger a to-do list rather
/// than a burial ground: fix the site and this test tells you to delete the entry.
#[test]
fn every_surfaced_entry_still_offends_and_names_its_replacement() {
    let sites = vacuous_sites();
    let mut problems = Vec::new();
    for s in SURFACED_BY_A_WIDER_DETECTOR {
        if !root().join(s.file).exists() {
            problems.push(format!("{} no longer exists — remove its entry", s.file));
            continue;
        }
        let Some(found) = sites.iter().find(|v| v.file == s.file && v.text == s.text) else {
            problems.push(format!(
                "{}: `{}` is gone or no longer vacuous — remove the entry and lower \
                 SURFACED_CEILING to {}",
                s.file,
                s.text,
                SURFACED_CEILING.saturating_sub(1)
            ));
            continue;
        };
        // The entry must describe the defect that is actually there. A drifted label means the
        // ledger is documenting one bug while grandfathering another.
        if found.shape != s.shape {
            problems.push(format!(
                "{}: the entry calls this `{}` but the detector reports `{}` — the ledger is \
                 describing a different defect from the one on that line",
                s.file, s.shape, found.shape
            ));
        }
        if !(s.honest.contains("assert!") || s.honest.contains("assert_eq!")) {
            problems.push(format!(
                "{}: the replacement must be an ASSERTION, or the entry is a wish: {:?}",
                s.file, s.honest
            ));
        }
        if s.honest.contains(&format!("{}\n", s.text)) || s.honest == s.text {
            problems.push(format!(
                "{}: the replacement is the offending line itself",
                s.file
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "the surfaced ledger must track reality exactly, so it can be emptied:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn every_grandfathered_file_still_exists_and_still_offends() {
    let sites = vacuous_sites();
    let mut problems = Vec::new();
    for (file, expected, _why) in GRANDFATHERED {
        let path = root().join(file);
        if !path.exists() {
            problems.push(format!(
                "{file} no longer exists — remove its grandfather entry"
            ));
            continue;
        }
        let found = sites
            .iter()
            .filter(|v| &v.file == file && !is_surfaced(v))
            .count();
        if found == 0 {
            problems.push(format!(
                "{file} is grandfathered but is now CLEAN — remove its entry and lower \
                 VACUOUS_CEILING by {expected}"
            ));
        } else if found < *expected {
            problems.push(format!(
                "{file} now has {found} vacuous sites, down from {expected} — update the entry and \
                 lower VACUOUS_CEILING"
            ));
        } else if found > *expected {
            problems.push(format!(
                "{file} has {found} vacuous sites, up from {expected} — the grandfather list is a \
                 record of old debt, not a licence to add more"
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "the grandfather list must track reality exactly, so progress is visible:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn every_grandfathered_entry_says_why_it_is_still_here() {
    const MIN: usize = 40;
    let thin: Vec<&str> = GRANDFATHERED
        .iter()
        .filter(|(_, _, why)| why.trim().len() < MIN)
        .map(|(f, _, _)| *f)
        .collect();
    assert!(
        thin.is_empty(),
        "each grandfathered file must record what the right assertion would be, or why it is not \
         yet knowable — otherwise nobody can ever fix it: {thin:?}"
    );
}

#[test]
fn the_report() {
    let sites = vacuous_sites();
    let surfaced = sites.iter().filter(|v| is_surfaced(v)).count();
    eprintln!(
        "\n  ══ TEST HONESTY ══  {} vacuous assertion(s): {} original (ceiling {VACUOUS_CEILING}), \
         {surfaced} surfaced by a wider detector (ceiling {SURFACED_CEILING})",
        sites.len(),
        sites.len() - surfaced
    );
    for v in &sites {
        let t = if v.text.chars().count() > 76 {
            format!("{}…", v.text.chars().take(76).collect::<String>())
        } else {
            v.text.clone()
        };
        let tag = if is_surfaced(v) {
            "surfaced"
        } else {
            "original"
        };
        eprintln!("    {}:{}  [{}, {tag}]\n      {t}", v.file, v.line, v.shape);
    }
    eprintln!(
        "\n  {} test file(s) and {} src file(s) with a #[cfg(test)] block scanned.\n",
        test_files().len(),
        unit_test_sources().len()
    );
}

// ============================================================================================
// GUARD SELF-TESTS — the detectors are fed synthetic violations and must fire.
// ============================================================================================

#[cfg(test)]
mod guard_selftests {
    use super::*;

    #[test]
    fn the_tautology_detector_fires_on_both_orders() {
        assert!(is_tautology("assert!(result.is_ok() || result.is_err());"));
        assert!(is_tautology(
            "assert!(result.is_err() || result.is_ok(), \"msg\");"
        ));
        assert!(is_tautology("assert!( r.is_ok()  ||  r.is_err() );"));
    }

    #[test]
    fn the_tautology_detector_does_not_fire_on_a_real_assertion() {
        assert!(!is_tautology(
            "assert!(result.is_ok(), \"should succeed: {result:?}\");"
        ));
        assert!(!is_tautology(
            "assert!(result.is_err(), \"should be refused\");"
        ));
        // The legitimate discriminating shape must survive.
        assert!(!is_tautology(
            "match result { Err(_) => {} Ok(r) => assert!(r.failed > 0) }"
        ));
    }

    #[test]
    fn the_discard_detector_fires_only_on_a_discarded_result() {
        assert!(is_discarded_result("let _ = result;"));
        assert!(is_discarded_result(
            "    let _ = result; // documents behavior"
        ));
        // `let _ = tx.send(..)` is a legitimate Rust idiom and must NOT be flagged.
        assert!(!is_discarded_result("let _ = tx.send((result, report));"));
        assert!(!is_discarded_result("let _ = vm.run();"));
        assert!(!is_discarded_result("let result = compile(src);"));
    }

    #[test]
    fn the_assert_true_detector_fires() {
        assert!(is_assert_true("assert!(true);"));
        assert!(is_assert_true("assert!( true , \"why\");"));
        assert!(!is_assert_true("assert!(truthy);"));
        // A test asserting on generated source that CONTAINS the text must not be flagged.
        assert!(!is_assert_true(
            "assert!(rust.contains(\"debug_assert!(true)\"));"
        ));
    }

    // ────────────────────────────────────────────────────────────────────────────────────────
    // REG-C — the detector must fire in BOTH message forms, and must still not fire on the text
    // inside a literal. It has been broken once in each direction; both are pinned here forever.
    // ────────────────────────────────────────────────────────────────────────────────────────

    /// Direction one: a vacuous assertion dressed up with a plausible message. This is the shape
    /// the old detector was blind to — it searched for `"assert!(true)"` WITH the closing paren,
    /// so every `assert!(true, "…")` in the workspace was invisible to the ratchet.
    #[test]
    fn the_assert_true_detector_fires_on_every_message_form() {
        for vector in [
            "assert!(true);",
            "assert!(true)",
            "assert!(true,);",
            "assert!( true , \"why\");",
            "assert!(true, \"the elaborator is fine\");",
            "        assert!(true); ",
            "if ok { assert!(true) }",
            "for _ in 0..3 { assert!(true, \"loops too\"); }",
        ] {
            assert!(
                is_assert_true(vector),
                "a vacuous assertion must be caught whatever message it wears: {vector}"
            );
        }
    }

    /// Direction two: the same TEXT, inside a string literal a test matches against generated
    /// source. Flagging this is the false positive that broke the detector the first time, and it
    /// is now refused twice over — by the boundary check, and by the lexer blanking the literal.
    #[test]
    fn the_assert_true_detector_does_not_fire_on_the_text_inside_a_literal() {
        for vector in [
            "assert!(rust.contains(\"debug_assert!(true)\"));",
            "assert!(rust.contains(\"assert!(true)\"));",
            "assert!(src.contains(\"assert!(true, \\\"why\\\")\"));",
            "let banned = \"assert!(true)\";",
            "eprintln!(\"never write assert!(true, \\\"x\\\") here\");",
        ] {
            let mut state = Lex::Code;
            let code = code_only(vector, &mut state);
            assert!(
                !is_assert_true(&code),
                "the banned text INSIDE a literal is documentation, not an assertion: {vector}"
            );
        }
    }

    /// The neighbours one rung either side. `assert!(truthy)` and `assert!(true_positive)` are real
    /// assertions; `debug_assert!(true)` and `my_assert!(true)` are other macros; `assert!(true ==
    /// f())` compares something.
    #[test]
    fn the_assert_true_detector_does_not_fire_on_its_neighbours() {
        for vector in [
            "assert!(truthy);",
            "assert!(truthiness, \"msg\");",
            "assert!(true_positive);",
            "debug_assert!(true);",
            "my_assert!(true);",
            "assert!(true == compute());",
            "assert_eq!(flag, true);",
            "assert!(!truth);",
        ] {
            assert!(
                !is_assert_true(vector),
                "a real assertion must not be mistaken for a vacuous one: {vector}"
            );
        }
    }

    /// A second vacuous assertion later on a line that also contains a string is a violation the
    /// old "a quote to my left" stand-in could never see. The lexer blanks the literal away, so it
    /// is now caught — this is the miss the two guards together close.
    #[test]
    fn a_violation_after_a_string_literal_on_the_same_line_is_caught() {
        let raw = "assert!(x, \"real\"); assert!(true);";
        assert!(
            !is_assert_true(raw),
            "on a RAW line the quote guard is deliberately conservative — it misses this"
        );
        let mut state = Lex::Code;
        let code = code_only(raw, &mut state);
        assert!(
            is_assert_true(&code),
            "…but after the lexer the literal is gone and the violation is plain: {code:?}"
        );
    }

    /// The quote guard inside `is_assert_true` is redundant AFTER `code_only`, and that must stay
    /// true: if the lexer ever stopped blanking the quote characters themselves, the guard would
    /// start suppressing real violations on every line that contains any string at all.
    #[test]
    fn the_quote_guard_is_redundant_after_the_lexer() {
        let mut state = Lex::Code;
        let code = code_only(
            "assert!(a, \"x\"); let s = r#\"y\"#; assert!(true);",
            &mut state,
        );
        assert!(
            !code.contains('"'),
            "code_only must leave no quote behind, or the quote guard stops being redundant and \
             starts being a hole: {code:?}"
        );
    }

    // ────────────────────────────────────────────────────────────────────────────────────────
    // THE LEXER — every detector reads its output, so a bug here is a bug in all three at once.
    // ────────────────────────────────────────────────────────────────────────────────────────

    fn code_of(line: &str) -> String {
        let mut state = Lex::Code;
        code_only(line, &mut state)
    }

    #[test]
    fn the_lexer_blanks_comments_strings_and_char_literals() {
        assert_eq!(
            code_of("let a = 1; // assert!(true);").trim_end(),
            "let a = 1;"
        );
        assert_eq!(
            code_of("/* assert!(true); */ let a = 1;").trim(),
            "let a = 1;"
        );
        assert!(!code_of("let s = \"assert!(true)\";").contains("assert"));
        assert!(!code_of("let s = r#\"assert!(true)\"#;").contains("assert"));
        assert!(!code_of("let s = br#\"assert!(true)\"#;").contains("assert"));
        // A `//` inside a string is not a comment — truncating there would silently stop scanning
        // the rest of a legitimate line.
        assert!(code_of("let u = \"http://x\"; assert!(true);").contains("assert!(true)"));
    }

    #[test]
    fn the_lexer_does_not_mistake_a_lifetime_for_a_char_literal() {
        let code = code_of("fn f<'a>(s: &'a str) { assert!(true); }");
        assert!(
            code.contains("assert!(true)"),
            "a lifetime tick must not open a char literal and swallow the rest of the line: {code:?}"
        );
        // …and a genuine char literal containing a quote must not open a string.
        let code = code_of("if c == '\"' { assert!(true); }");
        assert!(
            code.contains("assert!(true)"),
            "a quote inside a char literal must not be read as a string opener: {code:?}"
        );
    }

    #[test]
    fn the_lexer_carries_a_multi_line_string_and_a_block_comment_across_lines() {
        let text = "let s = \"opens here\n  assert!(true);\n  closes\";\nassert!(true);";
        let lines = code_lines(text);
        assert!(
            !lines[1].1.contains("assert"),
            "a line INSIDE a multi-line string is not code: {:?}",
            lines[1].1
        );
        assert!(
            lines[3].1.contains("assert!(true)"),
            "…and the line after it is: {:?}",
            lines[3].1
        );

        let text = "/* opens\n assert!(true);\n closes */\nassert!(true);";
        let lines = code_lines(text);
        assert!(
            !lines[1].1.contains("assert"),
            "inside a block comment: {:?}",
            lines[1].1
        );
        assert!(
            lines[3].1.contains("assert!(true)"),
            "after it: {:?}",
            lines[3].1
        );
    }

    #[test]
    fn an_escaped_quote_does_not_close_a_string() {
        let code = code_of("assert!(s.contains(\"say \\\"assert!(true)\\\" now\"));");
        assert!(
            !code.contains("assert!(true)"),
            "an escaped quote is not the end of the literal; everything through the real closing \
             quote is text: {code:?}"
        );
    }

    #[test]
    fn the_lexer_preserves_the_rest_of_a_line_after_a_literal() {
        assert_eq!(code_of("f(\"ab\", 3);").replace(' ', ""), "f(,3);");
    }

    // ────────────────────────────────────────────────────────────────────────────────────────
    // THE WALK — what the lock can see. A detector that never reads a file protects nothing
    // there, which is the same hole as a detector that never fires.
    // ────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn the_scanner_sees_a_nested_tests_directory() {
        let files = test_files();
        let nested: Vec<String> = files
            .iter()
            .map(|p| rel(p))
            .filter(|r| r.matches('/').count() > 3)
            .collect();
        assert!(
            !nested.is_empty(),
            "shared harness code under `tests/<subdir>/` is where an assertion that cannot fail \
             hides best — every suite in the crate runs through it. The walk must reach it."
        );
        assert!(
            nested.iter().any(|r| r.ends_with("/tests/common/mod.rs")),
            "expected at least one `tests/common/mod.rs`; found {nested:?}"
        );
    }

    #[test]
    fn the_scanner_sees_unit_test_modules_in_src() {
        let srcs = unit_test_sources();
        assert!(
            srcs.len() > 100,
            "hundreds of `src/` files carry a `#[cfg(test)]` module; the lock saw {} — if this \
             collapses, every unit test in the workspace is unguarded",
            srcs.len()
        );
    }

    #[test]
    fn the_cfg_test_region_covers_the_test_module_and_nothing_else() {
        let text = "fn prod() {\n    let _ = result;\n}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {\n        let _ = result;\n    }\n}\nfn after() { let _ = result; }\n";
        let code = code_lines(text);
        let region = cfg_test_regions(&code);
        assert!(
            !region.contains(&2),
            "production code is not test code: {region:?}"
        );
        assert!(
            region.contains(&9),
            "the body of a `#[cfg(test)]` module is: {region:?}"
        );
        assert!(
            !region.contains(&12),
            "and the region closes with its brace: {region:?}"
        );
    }

    #[test]
    fn a_one_line_cfg_test_module_still_opens_its_region() {
        let text = "#[cfg(test)] mod tests { #[test] fn t() { let _ = result; } }\nfn after() {}\n";
        let region = cfg_test_regions(&code_lines(text));
        assert!(
            region.contains(&1),
            "`#[cfg(test)] mod tests {{ … }}` on one line must still be scanned: {region:?}"
        );
        assert!(
            !region.contains(&2),
            "and must close at its brace: {region:?}"
        );
    }

    #[test]
    fn a_cfg_test_on_an_item_without_a_block_does_not_arm_the_next_one() {
        let text = "#[cfg(test)]\nuse super::*;\n\nfn prod() {\n    let _ = result;\n}\n";
        let region = cfg_test_regions(&code_lines(text));
        assert!(
            region.is_empty(),
            "`#[cfg(test)] use …;` has no block; arming on it would swallow the next function and \
             scan production code as tests: {region:?}"
        );
    }

    #[test]
    fn a_commented_out_violation_is_not_counted() {
        let commented = "// assert!(result.is_ok() || result.is_err());";
        let code = commented.split("//").next().unwrap_or(commented);
        assert!(
            !is_tautology(code),
            "a commented-out line is not a live assertion"
        );
    }

    // ────────────────────────────────────────────────────────────────────────────────────────
    // THE EXHAUSTIVE-PAIR DETECTOR — the Result spelling was the only one anyone had written a
    // detector for, and it is one of three. Each was found live in this workspace.
    // ────────────────────────────────────────────────────────────────────────────────────────

    /// The Option spelling. `phase_hw_type_infer.rs:184` is `types.get("x").is_none() ||
    /// types.get("x").is_some()` — the same tautology as the banned `is_ok() || is_err()`, in a
    /// spelling the ratchet had never been taught, sitting in the suite uncounted.
    #[test]
    fn the_pair_detector_fires_on_the_option_spelling() {
        for vector in [
            "assert!(o.is_some() || o.is_none());",
            "assert!(o.is_none() || o.is_some(), \"either way\");",
            "assert!(types.get(\"x\").is_none() || types.get(\"x\").is_some());",
            "assert!(m[&k].is_some() || m[&k].is_none());",
        ] {
            assert!(
                is_tautology(vector),
                "a predicate or-ed with its own complement is true for every Option: {vector}"
            );
        }
    }

    /// The `is_empty` spelling, whose complement has no name and is spelled with `!`.
    /// `analysis/check.rs:1593` is `assert!(!legacy.is_empty() || legacy.is_empty());` with the
    /// comment `// just don't panic` — the exact self-justification this file's header refutes,
    /// living in a `#[cfg(test)]` block in `src/`, which the lock only started reading in wave 2.
    #[test]
    fn the_pair_detector_fires_on_a_predicate_or_ed_with_its_own_negation() {
        for vector in [
            "assert!(!legacy.is_empty() || legacy.is_empty());",
            "assert!(v.is_empty() || !v.is_empty(), \"documents behaviour\");",
            "assert!(!report.items.is_empty() || report.items.is_empty());",
        ] {
            assert!(
                is_tautology(vector),
                "`!P || P` is true for every input: {vector}"
            );
        }
    }

    /// THE MEASURED FALSE POSITIVES, and the reason the detector reads two views of a line rather
    /// than one. Every vector here is a live, falsifiable assertion in this workspace that a
    /// receiver-BLIND detector — or a receiver-aware one reading the literal-blanked line — calls a
    /// tautology. A ratchet that fires on real assertions gets switched off, so this is the
    /// direction that decides whether the widening survives contact.
    #[test]
    fn the_pair_detector_does_not_fire_on_a_real_implication() {
        for vector in [
            // spatial_tensix_lowering_lock.rs:149 and :194 — `!A || B`, two DIFFERENT calls that
            // blank to the same text.
            "assert!(!host.contains(\"TT_METAL_HOME\") || host.contains(\"getenv\"), \"msg\");",
            "assert!(!k.source.contains(\"get_semaphore(0)\") || k.source.contains(\"get_arg_val\"));",
            // spatial_fabric_lowering_lock.rs:304.
            "assert!(!text.contains(\"problems\") || text.contains(\"Found and reported 0 problems\"));",
            // phase60_proof_engine.rs:476 — two different FIELDS, an exhaustive pair by name only.
            "assert!(result.error.is_none() || result.derivation.is_some(), \"msg\");",
            // phase_optimize.rs:2624 — a complement or-ed with an unrelated comparison.
            "assert!(loop_pos.is_some() && (if_pos.is_none() || if_pos > loop_pos));",
            // …and the shape blanking gets exactly backwards: same method pair, different key.
            "assert!(map.get(\"a\").is_none() || map.get(\"b\").is_some());",
            "assert!(!a.is_empty() || b.is_empty());",
            "assert!(left.is_ok() || right.is_err());",
        ] {
            assert!(
                !is_tautology(vector),
                "this is a falsifiable implication, not a tautology — the two sides are different \
                 expressions: {vector}"
            );
        }
    }

    /// The literal-blanked view is what proves a shape is CODE; the literal-preserving view is what
    /// proves the two sides are the same expression. Pin both halves against each other, because
    /// dropping either one is a silent regression in one direction only.
    #[test]
    fn the_pair_detector_needs_both_views_of_a_line() {
        let views = |line: &str| {
            let mut s = Lex::Code;
            let code = code_only(line, &mut s);
            let mut s = Lex::Code;
            let kept = without_comments(line, &mut s);
            (code, kept)
        };

        let (code, kept) =
            views("assert!(types.get(\"x\").is_none() || types.get(\"x\").is_some());");
        assert!(
            is_exhaustive_pair(&code, &kept),
            "the real site must fire when both views are supplied: code={code:?} kept={kept:?}"
        );

        // The measured false positive, and the whole reason the second view exists: two lookups of
        // DIFFERENT keys. It must not fire…
        let (dcode, dkept) =
            views("assert!(map.get(\"a\").is_none() || map.get(\"b\").is_some());");
        assert!(
            !is_exhaustive_pair(&dcode, &dkept),
            "two lookups of different keys are a falsifiable assertion: kept={dkept:?}"
        );
        // …and it WOULD, on a detector that read the blanked line for the receiver half, because
        // blanking turns `map.get("a")` and `map.get("b")` into the same text. Asserting the buggy
        // behaviour explicitly is what stops someone "simplifying" the two views back into one.
        assert!(
            is_exhaustive_pair(&dcode, &dcode),
            "blanking must be what makes these two look identical — if this stops holding, the \
             second view is no longer earning its keep and this test is the place to say so: \
             code={dcode:?}"
        );

        // The mirror: the banned text inside a string literal is documentation, and only the
        // BLANKED view can tell. The literal-preserving view alone would fire on it.
        let (qcode, qkept) = views("assert!(src.contains(\"r.is_ok() || r.is_err()\"));");
        assert!(
            !is_exhaustive_pair(&qcode, &qkept),
            "a tautology quoted inside a literal is text a test matches against, not an assertion: \
             code={qcode:?}"
        );
        assert!(
            is_exhaustive_pair(&qkept, &qkept),
            "…and the literal-preserving view alone would fire on it, which is why the blanked view \
             decides whether a shape is code: kept={qkept:?}"
        );
    }

    /// A comment can supply neither half. Both views blank comments, so a tautology written in
    /// prose beside a real assertion is invisible to the detector in both directions.
    #[test]
    fn a_comment_cannot_supply_either_half_of_a_pair() {
        let line = "assert!(x.is_ok() || y.is_err()); // unlike r.is_ok() || r.is_err()";
        let mut s = Lex::Code;
        let code = code_only(line, &mut s);
        let mut s = Lex::Code;
        let kept = without_comments(line, &mut s);
        assert!(
            !kept.contains("unlike"),
            "without_comments must still blank the comment: {kept:?}"
        );
        assert!(
            !is_exhaustive_pair(&code, &kept),
            "the live assertion compares two different receivers; the tautology is in the comment"
        );
    }

    /// `without_comments` keeps the literal and `code_only` does not, and they must agree about
    /// every other character — same length, same code. Two lexers that disagree about where a
    /// string ends would make the two halves read two different lines.
    #[test]
    fn the_two_views_differ_only_inside_literals() {
        let line = "assert_eq!(f(\"a b\"), g(r#\"c\"#), 'z'); // note";
        let mut s = Lex::Code;
        let code = code_only(line, &mut s);
        let mut s = Lex::Code;
        let kept = without_comments(line, &mut s);
        assert_eq!(
            code.chars().count(),
            kept.chars().count(),
            "{code:?} vs {kept:?}"
        );
        assert!(
            kept.contains("\"a b\"") && kept.contains("r#\"c\"#"),
            "literals kept: {kept:?}"
        );
        assert!(!code.contains('"'), "literals blanked: {code:?}");
        for (a, b) in code.chars().zip(kept.chars()) {
            assert!(
                a == b || a == ' ',
                "outside a literal the two views must agree: {a:?} {b:?}"
            );
        }
    }

    // ────────────────────────────────────────────────────────────────────────────────────────
    // THE UNSIGNED-COUNT DETECTOR — `usize + usize >= 0`, true for every possible input, in a
    // test whose name promises it checks the report.
    // ────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn the_unsigned_detector_fires_on_a_count_that_cannot_be_negative() {
        for vector in [
            "assert!(report.crossings.len() + report.violations.len() >= 0);",
            "assert!(v.len() >= 0);",
            "assert!(v.len() >= 0, \"non-empty\");",
            "assert!(xs.iter().count() >= 0);",
            "assert!(0 <= v.len());",
            "assert!(a.len() + b.len() + c.len() >= 0);",
        ] {
            assert!(
                is_unsigned_ge_zero(vector),
                "an unsigned count satisfies `>= 0` for every possible input: {vector}"
            );
        }
    }

    /// The neighbours, each one rung away and each a real check. The cast is the important one: it
    /// is what turns the same-looking expression into a genuine sign test.
    #[test]
    fn the_unsigned_detector_does_not_fire_on_a_real_comparison() {
        for vector in [
            "assert!(v.len() >= 1);",
            "assert!(v.len() > 0);",
            "assert!(delta >= 0);",
            "assert!(x as i64 >= 0);",
            "assert!((a.len() as i64 - b.len() as i64) >= 0);",
            "assert!(v.len() >= 0 + k);",
            "assert!(v.len() >= n);",
            "assert!(0 <= idx);",
            "assert!(score >= 0.0);",
            "assert!(v.len() >= 0x10);",
        ] {
            assert!(
                !is_unsigned_ge_zero(vector),
                "a falsifiable comparison must not be mistaken for a vacuous one: {vector}"
            );
        }
    }

    /// The backward expression reader is shared by both new detectors, so a bug in it is a bug in
    /// both at once. It must take the call's ARGUMENTS with it — they are what tells two calls
    /// apart — and must stop at the enclosing bracket rather than swallowing it.
    #[test]
    fn the_expression_reader_takes_the_arguments_and_stops_at_the_enclosing_call() {
        let cases = [
            ("assert!(types.get(\"x\").is_none()", "types.get(\"x\")"),
            ("assert!(a.b[2].c.is_ok()", "a.b[2].c"),
            ("assert!(f(g(1,2)).is_some()", "f(g(1,2))"),
            ("assert!(!v.is_empty()", "v"),
            ("if x.is_ok()", "x"),
        ];
        for (src, want) in cases {
            let at = src
                .rfind(".is")
                .expect("a method call to read backwards from");
            let got = &src[expression_start(src, at)..at];
            assert_eq!(got, want, "reading backwards from {at} in {src:?}");
        }
    }

    #[test]
    fn the_scanner_actually_finds_files() {
        let files = test_files();
        assert!(
            files.len() > 500,
            "the scanner must see the whole suite; found only {} files — if this drops, the lock \
             is silently checking nothing",
            files.len()
        );
    }
}
