#![allow(unused_imports, dead_code)]
mod common;
use common::assert_all_tiers;
/// Replace up to n occurrences. go: strings.Replace · py: str.replace(count)
#[test]
fn parity_str_replace_n() {
    assert_all_tiers(
        "## Main\nShow replaceN(\"a-a-a-a\", \"-\", \"+\", 2).\n",
        "a+a+a-a",
    );
}
/// Split keeping the separator on each piece. go: strings.SplitAfter
#[test]
fn parity_str_split_after() {
    assert_all_tiers(
        "## Main\nLet p be splitAfter(\"a,b,c\", \",\").\nShow length of p.\nShow item 1 of p.\n",
        "3\na,",
    );
}
