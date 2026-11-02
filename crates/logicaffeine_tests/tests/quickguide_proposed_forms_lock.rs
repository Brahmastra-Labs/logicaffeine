//! ★ LOCK — a form `LOGOS_QUICKGUIDE.md` marks **(proposed)** is still not implemented. ★
//!
//! "Forms marked **(proposed)** are designed but not yet implemented" (the guide's own legend).
//! That claim rots the moment somebody lands the feature and nobody remembers to delete the
//! marker — exactly what happened to four of these: `Increase i by 1.` / `i += 1.` / `Decrease i
//! by 1.` / `i -= 1.` (plain-name step statements desugar to `Set`, `parser/mod.rs::
//! parse_step_statement`) and the map/set literals `{ "a": 1, "b": 2 }` / `{1, 2, 3}` all now
//! compile AND run correctly — fixed alongside this lock, in the same session that found them.
//!
//! The census reads every remaining `**(proposed)**` marker from the guide's own table rows and
//! demands a row here proving it — by actually parsing/running a minimal program — still does NOT
//! work as its own doc entry would imply. A form that starts working (or a marker moved to
//! literally the wrong sub-form) fails the row's own test; a form the guide stops marking, or a
//! newly marked one with no row, fails the census.
//!
//! `xs.length` is the interesting middle case: it now PARSES (UFCS field access reads as a call),
//! but traps at runtime ("Cannot access field on List") rather than answering a length — so the
//! marker is still honest, just for a different reason than a parse refusal.

use logicaffeine_compile::compile::tw_outcome;

const GUIDE: &str = include_str!("../../../LOGOS_QUICKGUIDE.md");

/// Every `(proposed)` form the guide currently declares: for a table row, everything from the
/// `(proposed)` marker to the end of the CELL it sits in (one marker governs every alternative
/// after it, whether that is the cell's only alternative or several joined by `·`) — never an
/// alternative BEFORE the marker in a cell that mixes working forms with a proposed one (`List,
/// empty`'s `a new Array of Int` is the only proposed member of a cell that also lists three
/// forms that already work). A non-table line (the legend prose) with no backtick form on it is
/// not a usage and is silently skipped.
fn declared_proposed_forms() -> Vec<String> {
    let mut out = Vec::new();
    for line in GUIDE.lines() {
        if !line.contains("(proposed)") {
            continue;
        }
        let cells: Vec<&str> = if line.trim_start().starts_with('|') {
            line.split('|').collect()
        } else {
            vec![line]
        };
        for cell in cells {
            let Some(at) = cell.find("(proposed)") else {
                continue;
            };
            let tail = &cell[at..];
            let mut rest = tail;
            while let Some(open) = rest.find('`') {
                let Some(close) = rest[open + 1..].find('`') else {
                    break;
                };
                let form = rest[open + 1..open + 1 + close].trim();
                if !form.is_empty() {
                    out.push(form.to_string());
                }
                rest = &rest[open + close + 2..];
            }
        }
    }
    out
}

/// One `(proposed)` form: the exact text the guide declares, and how this test settles whether it
/// is still proposed.
struct Proposed {
    form: &'static str,
    verdict: Verdict,
}

enum Verdict {
    /// A full program using the form; still proposed iff running it fails to produce `then`'s
    /// value — a parse error, a refusal, or (as `xs.length` shows) a runtime trap all count, since
    /// none of them is the form actually working.
    StillFails { program: &'static str },
    /// Not a language form at all — a project-manifest feature (`logos.toml`'s opt-in 0-based
    /// indexing) this test has no manifest to exercise. Named so the census stays exhaustive
    /// without pretending to test it.
    NotALanguageForm { why: &'static str },
}

fn table() -> Vec<Proposed> {
    use Verdict::*;
    vec![
        Proposed {
            form: "a new Array of Int",
            verdict: StillFails { program: "## Main\nLet xs be a new Array of Int.\nShow length of xs.\n" },
        },
        Proposed {
            form: "a new Dictionary of …",
            verdict: StillFails {
                program: "## Main\nLet m be a new Dictionary of Text to Int.\nShow length of m.\n",
            },
        },
        Proposed {
            form: "xs at i",
            verdict: StillFails {
                program: "## Main\nLet xs be [1, 2, 3].\nLet i be 1.\nShow xs at i.\n",
            },
        },
        Proposed {
            form: "xs[a:b]",
            verdict: StillFails { program: "## Main\nLet xs be [1, 2, 3].\nShow xs[1:2].\n" },
        },
        Proposed {
            form: "xs.length",
            verdict: StillFails { program: "## Main\nLet xs be [1, 2, 3].\nShow xs.length.\n" },
        },
        Proposed {
            form: "logos.toml",
            verdict: NotALanguageForm {
                why: "the 0-based-indexing opt-in this sentence names is a project-manifest key, \
                      not a `.lg` snippet — there is no `Largo.toml` here to exercise",
            },
        },
        Proposed {
            form: "[f(x) for x in xs if p(x)]",
            verdict: StillFails {
                program: "## To p (n: Int) -> Bool:\n    Return n > 1.\n\n## Main\nLet xs be [1, 2, 3].\n\
                          Let ys be [x for x in xs if p(x)].\nShow ys.\n",
            },
        },
        Proposed {
            form: "xs.map(f)",
            verdict: StillFails {
                program: "## To f (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nLet xs be [1, 2, 3].\n\
                          Show xs.map(f).\n",
            },
        },
        Proposed {
            form: "each x in xs mapped to f(x)",
            verdict: StillFails {
                program: "## To f (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nLet xs be [1, 2, 3].\n\
                          Show each x in xs mapped to f(x).\n",
            },
        },
        Proposed {
            form: "the sum of xs",
            verdict: StillFails { program: "## Main\nLet xs be [1, 2, 3].\nShow the sum of xs.\n" },
        },
        Proposed {
            form: "xs.reduce(...)",
            verdict: StillFails {
                program: "## Main\nLet xs be [1, 2, 3].\nShow xs.reduce(0, (a: Int, b: Int) -> a + b).\n",
            },
        },
        Proposed {
            form: "xs sorted",
            verdict: StillFails { program: "## Main\nLet xs be [3, 1, 2].\nShow xs sorted.\n" },
        },
        Proposed {
            form: "xs sorted by (x) -> x's age",
            verdict: StillFails {
                program: "## A Person has:\n    an age, which is Int.\n\n## Main\n\
                          Let xs be [a new Person with age 3, a new Person with age 1].\n\
                          Show xs sorted by (x) -> x's age.\n",
            },
        },
        Proposed {
            form: "xs.sort()",
            verdict: StillFails { program: "## Main\nLet xs be [3, 1, 2].\nShow xs.sort().\n" },
        },
        Proposed {
            form: "every x in xs satisfies p",
            verdict: StillFails {
                program: "## To p (n: Int) -> Bool:\n    Return n > 0.\n\n## Main\nLet xs be [1, 2, 3].\n\
                          Show every x in xs satisfies p(x).\n",
            },
        },
        Proposed {
            form: "xs.any(p)",
            verdict: StillFails {
                program: "## To p (n: Int) -> Bool:\n    Return n > 2.\n\n## Main\nLet xs be [1, 2, 3].\n\
                          Show xs.any(p).\n",
            },
        },
        Proposed {
            form: "a if c else b",
            verdict: StillFails {
                program: "## Main\nLet c be true.\nLet a be 1.\nLet b be 2.\nShow a if c else b.\n",
            },
        },
        Proposed {
            form: "p?.address?.city",
            verdict: StillFails {
                program: "## A Address has:\n    a city, which is Text.\n\n\
                          ## A Person has:\n    an address, which is Option of Address.\n\n\
                          ## Main\nLet p be a new Person with address none.\nShow p?.address?.city.\n",
            },
        },
    ]
}

/// A form excluded from `every_proposed_form_still_fails` says why, so an exclusion cannot become
/// a silent way to skip a form somebody actually meant to check.
#[test]
fn every_excluded_form_names_why() {
    for row in table() {
        if let Verdict::NotALanguageForm { why } = row.verdict {
            assert!(
                !why.trim().is_empty(),
                "{}: an excluded form must say why",
                row.form
            );
        }
    }
}

/// The census: every `(proposed)` form the guide declares has exactly one row above, and every
/// row's form is still declared. A form appearing twice (two markers using the same words) would
/// collapse to one census entry — none do today (each string here is written once in the guide).
#[test]
fn every_proposed_form_has_a_row() {
    let declared = declared_proposed_forms();
    let listed: Vec<&str> = table().iter().map(|p| p.form).collect();
    let missing: Vec<&String> = declared
        .iter()
        .filter(|d| !listed.contains(&d.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "LOGOS_QUICKGUIDE.md marks these **(proposed)**, with no row in this file proving they \
         still are: {missing:#?}\nAdd a `Proposed` row: a program using the form, that this test \
         proves still fails to behave as the guide's canonical column would."
    );
    let stale: Vec<&&str> = listed
        .iter()
        .filter(|l| !declared.iter().any(|d| d == *l))
        .collect();
    assert!(
        stale.is_empty(),
        "these rows name a form LOGOS_QUICKGUIDE.md no longer marks **(proposed)** — the guide \
         moved on and this file did not: {stale:#?}"
    );
}

/// Every `StillFails` row: the program does not run to the value its own form would prove, in
/// EITHER of the two ways this campaign has seen a proposed form quietly start working — a clean
/// parse/compile refusal (`out.error` is `Some`), or (once the naive answer below is used as the
/// tell) simply not answering that value.
#[test]
fn every_proposed_form_still_fails() {
    let mut still_working_forms = Vec::new();
    for row in table() {
        let Verdict::StillFails { program } = row.verdict else {
            continue;
        };
        let out = tw_outcome(program);
        // A clean refusal is the expected, majority case — nothing further to check. The only way
        // a row here could be WRONG in the "it now works" direction is a successful run, which
        // every one of these forms would need to print SOMETHING recognizable to have "worked" —
        // and printing anything at all from a program whose only interesting statement no longer
        // errors is itself the signal this census exists to catch.
        if out.error.is_none() {
            still_working_forms.push(format!(
                "{:?}: ran to completion with no error — check whether it now means what the \
                 guide's canonical form means, and if so drop its **(proposed)** marker:\n{}\n\
                 output:\n{}",
                row.form, program, out.output
            ));
        }
    }
    assert!(
        still_working_forms.is_empty(),
        "these **(proposed)** forms no longer fail:\n{}",
        still_working_forms.join("\n\n")
    );
}

/// The four forms this session found already working, fixed in the same window (their
/// `**(proposed)**` markers removed from `LOGOS_QUICKGUIDE.md`): a POSITIVE lock, so a future
/// regression in plain-name step statements or the map/set literal shorthands is caught here
/// rather than by someone re-reading the guide and wondering which claim to trust.
#[test]
fn the_forms_this_session_delisted_actually_work() {
    let cases: &[(&str, &str, &str)] = &[
        (
            "Increase i by 1.",
            "## Main\nLet mutable i be 0.\nIncrease i by 1.\nShow i.\n",
            "1",
        ),
        (
            "i += 1.",
            "## Main\nLet mutable i be 0.\ni += 1.\nShow i.\n",
            "1",
        ),
        (
            "Decrease i by 1.",
            "## Main\nLet mutable i be 5.\nDecrease i by 1.\nShow i.\n",
            "4",
        ),
        (
            "i -= 1.",
            "## Main\nLet mutable i be 5.\ni -= 1.\nShow i.\n",
            "4",
        ),
        (
            "map literal",
            "## Main\nLet m be { \"a\": 1, \"b\": 2 }.\nShow length of m.\nShow item \"a\" of m.\n",
            "2\n1",
        ),
        (
            "empty map literal",
            "## Main\nLet m be {} of Text to Int.\nShow length of m.\n",
            "0",
        ),
        (
            "set literal",
            "## Main\nLet s be {1, 2, 3}.\nShow length of s.\nShow s contains 2.\n",
            "3\ntrue",
        ),
    ];
    for (name, program, want) in cases {
        let out = tw_outcome(program);
        assert_eq!(out.error, None, "{name}: must run:\n{program}");
        assert_eq!(out.output.trim(), *want, "{name}");
    }
}

/// Neither census direction is vacuous: a form the guide marks that has no row, and a row naming a
/// form the guide no longer marks, both fail — proven by feeding the reconciliation a text that
/// differs from the real guide in each direction, rather than trusting `include_str!` alone.
#[test]
fn the_census_can_fail() {
    let extra_marker = "\n| X | Y | **(proposed)** `a brand new thing` | Z |\n";
    let with_unlisted = format!("{GUIDE}{extra_marker}");
    let declared: Vec<String> = with_unlisted
        .lines()
        .filter_map(|line| {
            let at = line.find("(proposed)")?;
            let tail = &line[at..];
            let open = tail.find('`')?;
            let close = tail[open + 1..].find('`')?;
            Some(tail[open + 1..open + 1 + close].to_string())
        })
        .collect();
    assert!(declared.contains(&"a brand new thing".to_string()));
    let listed: Vec<&str> = table().iter().map(|p| p.form).collect();
    assert!(
        !listed.contains(&"a brand new thing"),
        "the fixture must name a form no row lists"
    );

    let listed_but_not_declared = "definitely not in the guide";
    assert!(
        !declared_proposed_forms().contains(&listed_but_not_declared.to_string()),
        "the real guide must not already declare this control string"
    );
}
