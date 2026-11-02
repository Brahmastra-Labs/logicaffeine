//! Stdlib-parity north-star corpus — domain: regex
//!
//! Auto-scaffolded from benchmarks/parity/catalog/superset.json by
//! scripts/bench/gen-parity-corpus.py. Each test mirrors one deduplicated capability of the
//! C/Python/Rust/Go (+Java) superset. `#[ignore]` = parity-pending (not yet implemented);
//! the tree stays green. To LAND a capability: implement it, replace the body with a real
//! `assert_all_tiers` behavioral spec, DELETE the `#[ignore]` line, flip the catalog entry
//! to `status: green`, and bump GREEN_FLOOR in parity_catalog_lock.rs.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;

/// First match of a pattern in a string.
/// provenance — python: re.search · go: regexp.Compile
/// LOGOS surface: `the first match of pat in s`
#[test]
fn parity_regex_find() {
    assert_all_tiers(
        "## Main
Show regexFind(\"l.o\", \"hello\").
Show regexFind(\"ll\", \"hello\").
Show regexFind(\"xyz\", \"hello\").
Show regexFind(\"al*o\", \"aaaloha\").
",
        "3
3
0
3",
    );
}

/// Whether a pattern matches anywhere in the string. go: regexp.MatchString / regexp.Match
#[test]
fn parity_regex_match() {
    assert_all_tiers(
        "## Main\nShow regexMatch(\"l.o\", \"hello\").\nShow regexMatch(\"xyz\", \"hello\").\nShow regexMatch(\"al*o\", \"aaaloha\").\n",
        "true\nfalse\ntrue",
    );
}

/// 1-based start positions of every non-overlapping match. go: regexp.FindAllStringIndex / regexp.FindAllIndex
#[test]
fn parity_regex_find_all() {
    assert_all_tiers(
        "## Main\nShow regexFindAll(\"a\", \"banana\").\nShow regexFindAll(\"an\", \"banana\").\nShow regexFindAll(\"z\", \"banana\").\n",
        "[2, 4, 6]\n[2, 4]\n[]",
    );
}

/// The matched substring of every non-overlapping match. go: regexp.FindAllString · python: re.findall
#[test]
fn parity_regex_find_all_text() {
    assert_all_tiers(
        "## Main\nLet ms be regexFindAllText(\"an\", \"banana\").\nShow length of ms.\nShow item 1 of ms.\nShow item 2 of ms.\n",
        "2\nan\nan",
    );
}

/// Replace every non-overlapping match with a replacement string.
/// go: regexp.ReplaceAllString / regexp.ReplaceAll / regexp.ReplaceAllLiteralString · python: re.sub
#[test]
fn parity_regex_replace_all() {
    assert_all_tiers(
        "## Main\nShow regexReplaceAll(\"an\", \"banana\", \"X\").\nShow regexReplaceAll(\"a\", \"banana\", \"X\").\nShow regexReplaceAll(\"z\", \"banana\", \"X\").\n",
        "bXXa\nbXnXnX\nbanana",
    );
}

/// Split a string around every non-overlapping match. go: regexp.Split · python: re.split
#[test]
fn parity_regex_split() {
    assert_all_tiers(
        "## Main\nLet ps be regexSplit(\",\", \"a,b,c\").\nShow length of ps.\nShow item 1 of ps.\nShow item 3 of ps.\nShow length of regexSplit(\",\", \"abc\").\n",
        "3\na\nc\n1",
    );
}
