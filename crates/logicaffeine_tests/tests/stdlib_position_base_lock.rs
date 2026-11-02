//! Every position the standard library takes or returns is 1-based (Tristen, 2026-09-24: "every
//! stdlib position taken/returned: 1-based," matching `item 1 of xs` and the loop-index base).
//!
//! This reads the SOURCE of every `crates/logicaffeine_compile/assets/std/*.lg` module — its doc
//! comments, not just its function signatures — because a signature substring grep for
//! `index`/`pos`/`offset` is a false-negative machine: `calendar.lg`'s `dayName(weekday: Int)` and
//! `session.lg`'s `sessionField(payload, i)` are both position-shaped functions whose PARAMETER
//! NAMES don't contain any of those substrings, and only a doc-comment read catches them. This
//! lock instead looks at every `## Note` block immediately preceding a `## To` declaration, since
//! this stdlib's own house style is to state a function's base explicitly in prose
//! ("1-based index", "0-based position", "1-indexed", …) — verified true of every function this
//! lock's own hand-audited catalogue found.
//!
//! Three outcomes per declaration whose Note states a base:
//! - states "1-based"/"1-indexed" (or an ISO/domain-specific 1-based phrasing) → compliant, pass.
//! - states "0-based"/"zero-based"/"0-indexed" and is NOT in `KNOWN_EXCEPTIONS` → FAIL: a new or
//!   regressed 0-based position function.
//! - states "0-based"/"zero-based"/"0-indexed" and IS in `KNOWN_EXCEPTIONS` → pass, but only
//!   because a human reviewed it and wrote down why (an internal encapsulated adapter with no
//!   external caller, or an explicit cross-cutting exception awaiting a separate ruling).
//!
//! `calendar.lg`'s `dayName`/`dayAbbr` take `weekday` on the ISO 8601 convention (Monday=1 …
//! Sunday=7), matching `the weekday of` / `BuiltinId::WeekdayOf` on every compiled tier — both
//! Notes state it with the literal phrase "ISO 8601" in their first line, which `classify` reads
//! as a one-based claim like any other, so they need no special-cased entry here.

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// Every `.lg` file under `assets/std/` — the census's file universe. A new stdlib module lands
/// here automatically (directory scan, not a hardcoded name list) so this lock never goes stale by
/// omission.
fn stdlib_files() -> Vec<(String, String)> {
    let dir = repo_root().join("crates/logicaffeine_compile/assets/std");
    let mut out: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("lg"))
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            let src = std::fs::read_to_string(&p)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()));
            (name, src)
        })
        .collect();
    out.sort();
    out
}

/// One `## To` declaration together with the `## Note` prose immediately above it (empty if the
/// declaration has none). Mirrors the file's own convention: a `## Note` block, then zero or more
/// blank lines, then the `## To <name> (...) -> <Type>:` header.
struct Declaration {
    name: String,
    signature: String,
    note: String,
}

fn declarations(src: &str) -> Vec<Declaration> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    let mut note_start: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        if *line == "## Note" {
            note_start = Some(i + 1);
            continue;
        }
        if let Some(rest) = line.strip_prefix("## To ") {
            let rest = rest.strip_prefix("native ").unwrap_or(rest);
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if name.is_empty() {
                note_start = None;
                continue;
            }
            let note = match note_start {
                Some(s) if s <= i => lines[s..i].join("\n"),
                _ => String::new(),
            };
            out.push(Declaration {
                name,
                signature: line.to_string(),
                note,
            });
            note_start = None;
        }
    }
    out
}

#[derive(Debug, PartialEq, Eq)]
enum Base {
    /// Note explicitly states a 1-based/1-indexed convention.
    OneBased,
    /// Note explicitly states a 0-based/zero-based/0-indexed convention.
    ZeroBased,
    /// No position/base vocabulary found in the Note at all — not a position-taking function
    /// under this lock's classification (e.g. a codepoint lookup, a byte count, a bit pattern).
    NotPositional,
}

/// Classifies a declaration's Note prose. This is the one place the "read the source, not a
/// signature substring grep" rule lives — the trigger phrases here are vocabulary this stdlib's
/// authors already use, UNIVERSALLY, to SELF-DOCUMENT a position's base (verified against the full
/// hand-audited catalogue: every position-taking/returning function
/// found there states its base in exactly these words, in its Note's FIRST LINE — the codebase's
/// consistent house style is a one-sentence direct contract first, with rationale/elaboration in
/// later paragraphs). This is deliberately a PRECISE, low-noise signal rather than a broad "mentions
/// position-adjacent vocabulary anywhere" guess: an earlier version of this lock scanned the WHOLE
/// note and that fired on three real false positives — `bits.lg`'s `bitLength` ("the position of
/// its highest set bit," a BIT position inside an Int, not a sequence position),
/// `builder.lg`'s `newReader` ("positioned at the first byte," already 1-based but not phrased with
/// the literal word "1-based"), and `ui.lg`'s `pageStart` ("1-based because LOGOS sequences are. A
/// module answering **0-based** bounds would be correct in isolation and wrong at every call
/// site" — the word "0-based" appears only to reject it, in the SECOND sentence, after the actual
/// contract already said "1-based" in the first). Restricting the scan to the first line closes
/// exactly this last case while still finding every genuine violation (every one found by the
/// original census states its base in that first sentence). Exhaustiveness over UNDOCUMENTED
/// functions is the separate, always-true invariant `every_stdlib_declaration_has_a_note` enforces
/// below, not this function's job.
fn classify(note: &str) -> Base {
    let first_line = note.lines().next().unwrap_or("").to_lowercase();
    let zero = first_line.contains("0-based")
        || first_line.contains("zero-based")
        || first_line.contains("0-indexed");
    let one = first_line.contains("1-based")
        || first_line.contains("one-based")
        || first_line.contains("1-indexed")
        || first_line.contains("iso 8601");
    if zero {
        // Any explicit 0-based claim is flagged, even if the same first line confusingly also
        // claims 1-based — that combination needs a human, not a silent pass.
        Base::ZeroBased
    } else if one {
        Base::OneBased
    } else {
        Base::NotPositional
    }
}

/// `(file, function)` pairs that are 0-based BY DESIGN and reviewed: internal, encapsulated
/// adapters with zero external caller, never reachable as "the position a LOGOS program's own
/// data is addressed by." Think twice before adding an entry here — this
/// list is a liability, not a convenience, and every entry must cite why flipping it is either
/// unsafe (risks a fuzz-locked algorithm) or meaningless (never externally observable).
const KNOWN_EXCEPTIONS: &[(&str, &str, &str)] = &[
    (
        "strings.lg",
        "byteAt",
        "0-based adapter so the Two-Way (Crochemore-Perrin) matcher can use the reference \
         algorithm's own 0-based coordinate space directly; every public entry point built on it \
         (indexOf, lastIndexOf, twoWayIndexBytes[From]) converts back to 1-based before returning, \
         and this function is never reachable with an externally-supplied position. Flipping it \
         risks the two_way_search_lock fuzz oracle for zero observable gain.",
    ),
    (
        "strings.lg",
        "twMaxSuf",
        "Internal to the same Two-Way matcher as byteAt, same reasoning.",
    ),
    (
        "strings.lg",
        "twCritical",
        "Internal to the same Two-Way matcher as byteAt, same reasoning.",
    ),
];

#[test]
fn every_stdlib_position_is_1_based_or_a_documented_exception() {
    let mut violations: Vec<String> = Vec::new();

    for (file, src) in stdlib_files() {
        for decl in declarations(&src) {
            if classify(&decl.note) == Base::ZeroBased {
                let key = (file.as_str(), decl.name.as_str());
                if KNOWN_EXCEPTIONS.iter().any(|(f, n, _)| (*f, *n) == key) {
                    continue;
                }
                violations.push(format!(
                    "{file}::{} — doc comment states a 0-based position\n    signature: {}\n    note: {}",
                    decl.name,
                    decl.signature.trim(),
                    decl.note.replace('\n', " / ")
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "stdlib position(s) found that are 0-based or undocumented, and not in \
         KNOWN_EXCEPTIONS:\n{}\n\n\
         Every stdlib position taken or returned must be 1-based (Tristen, 2026-09-24). If this is \
         a genuine internal adapter, add it to KNOWN_EXCEPTIONS with a citation. If it needs a \
         cross-cutting ruling like the weekday builtin, add it to PENDING_WEEKDAY_RULING and \
         PENDING_DOC_FRAGMENTS with the reason beside the entry.",
        violations.join("\n")
    );
}

/// Mutation self-test: the classifier and the exceptions/pending lookup must actually be able to
/// fail. Feeds a synthetic module exercising every branch (a plain 1-based function, an
/// undocumented-but-positional one, a 0-based one absent from every allowlist, one present in
/// `KNOWN_EXCEPTIONS`, and `calendar.lg`'s real `dayName`/`dayAbbr` pair) directly through
/// `declarations`/`classify` and checks each lands where it should — proving this lock is not a
/// check that can never fail.
#[test]
fn classifier_self_test_every_branch_is_reachable() {
    let synthetic = "\
## Note
Returns the 1-based index of the first occurrence of `x` in `xs`, or 0 if absent.

## To compliantIndexOf (xs: Seq of Int, x: Int) -> Int:
    Return 1.

## Note
Not a position at all — the Unicode category code for codepoint `cp`.

## To unicodeCategoryCode (cp: Int) -> Int:
    Return 0.

## To noDocAtAll (xs: Seq of Int, pos: Int) -> Int:
    Return pos.

## Note
The byte at 0-based position `i` of `xs` — a thin adapter for the reference algorithm.

## To byteAt (xs: Seq of Int, i: Int) -> Int:
    Return 0.

## Note
A brand-new, unreviewed 0-based offset into `xs`. Nobody has approved this.

## To newHypotheticalViolation (xs: Seq of Int, off: Int) -> Int:
    Return off.
";
    let decls = declarations(synthetic);
    let by_name = |n: &str| {
        decls
            .iter()
            .find(|d| d.name == n)
            .unwrap_or_else(|| panic!("missing {n} in self-test fixture"))
    };

    assert_eq!(classify(&by_name("compliantIndexOf").note), Base::OneBased);
    assert_eq!(
        classify(&by_name("unicodeCategoryCode").note),
        Base::NotPositional
    );
    // A declaration with NO preceding `## Note` at all classifies as NotPositional here (empty
    // note text triggers neither the "0-based" nor "1-based" phrase match) — this classifier does
    // NOT catch an undocumented position function by itself. `every_stdlib_declaration_has_a_note`
    // is the separate invariant that makes this case provably unreachable in the real stdlib
    // (verified true today for all ~40 modules), so `noDocAtAll`-shaped functions cannot exist.
    assert_eq!(classify(&by_name("noDocAtAll").note), Base::NotPositional);
    assert_eq!(classify(&by_name("byteAt").note), Base::ZeroBased);
    assert_eq!(
        classify(&by_name("newHypotheticalViolation").note),
        Base::ZeroBased
    );

    // `byteAt` is a KNOWN_EXCEPTIONS entry (for strings.lg) — the same NAME in a hypothetical
    // OTHER file must NOT ride along on that exemption: exceptions are keyed by (file, name).
    assert!(KNOWN_EXCEPTIONS
        .iter()
        .any(|(f, n, _)| *f == "strings.lg" && *n == "byteAt"));
    assert!(!KNOWN_EXCEPTIONS
        .iter()
        .any(|(f, n, _)| *f == "some_other_module.lg" && *n == "byteAt"));

    // The real calendar.lg pair classifies as OneBased: both Notes state the ISO 8601 convention
    // (Monday=1..Sunday=7) with the literal phrase "ISO 8601" in their first line, which the `one`
    // branch above reads exactly like "1-based"/"1-indexed". No KNOWN_EXCEPTIONS or special-cased
    // registry is needed for this pair — the general mechanism already gets it right.
    let calendar_src = read("crates/logicaffeine_compile/assets/std/calendar.lg");
    let calendar_decls = declarations(&calendar_src);
    let day_name = calendar_decls
        .iter()
        .find(|d| d.name == "dayName")
        .expect("dayName declared");
    let day_abbr = calendar_decls
        .iter()
        .find(|d| d.name == "dayAbbr")
        .expect("dayAbbr declared");
    assert_eq!(classify(&day_name.note), Base::OneBased);
    assert_eq!(classify(&day_abbr.note), Base::OneBased);
}

/// `noDocAtAll`-shaped functions (a position-like parameter name, e.g. `pos`/`idx`/`offset`, but
/// NO preceding `## Note` at all) are exactly the case a signature-only grep would need to catch
/// and a note-text classifier cannot see. This lock does not attempt to parse signatures for
/// position-like parameter names (that heuristic both over- and under-matches, per the campaign's
/// own inventory finding it missed `calendar.lg`'s `weekday`/`session.lg`'s `i`) — instead it
/// relies on this stdlib's own house style of ALWAYS writing a `## Note` before a `## To`
/// declaration. This test locks that house style itself: every declaration in every stdlib module
/// has a non-empty Note, so "no note at all" can never be how a real 0-based function hides from
/// `every_stdlib_position_is_1_based_or_a_documented_exception`.
#[test]
fn every_stdlib_declaration_has_a_note() {
    let mut undocumented: Vec<String> = Vec::new();
    for (file, src) in stdlib_files() {
        for decl in declarations(&src) {
            if decl.note.trim().is_empty() {
                undocumented.push(format!("{file}::{}", decl.name));
            }
        }
    }
    assert!(
        undocumented.is_empty(),
        "stdlib declaration(s) with no preceding `## Note` block — this lock's position-base \
         classifier depends on every declaration being documented; an undocumented function is \
         invisible to it:\n{}",
        undocumented.join("\n")
    );
}
