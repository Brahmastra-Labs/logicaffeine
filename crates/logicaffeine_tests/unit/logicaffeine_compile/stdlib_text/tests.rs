use super::*;

/// The section rule as the loader stated it before sections became ranges: the lines from a `## To` header to the
/// next `## ` header, joined, trimmed.
fn sections_by_lines(code: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut current: Option<(String, String)> = None;
    for line in code.split_inclusive('\n') {
        if line.starts_with("## ") {
            if let Some((name, text)) = current.take() {
                out.push((name, text.trim().to_string()));
            }
            if let Some(name) = fn_section_name(line) {
                current = Some((name, String::new()));
            }
        }
        if let Some((_, text)) = current.as_mut() {
            text.push_str(line);
        }
    }
    if let Some((name, text)) = current {
        out.push((name, text.trim().to_string()));
    }
    out
}

fn sections_by_ranges(code: &str) -> Vec<(String, String)> {
    function_section_ranges(code)
        .into_iter()
        .map(|(name, range)| (name, code[range].to_string()))
        .collect()
}

#[test]
fn section_ranges_cut_what_the_line_rule_cuts() {
    for code in [
        "",
        "## Main\nShow 1.\n",
        "## To f:\n    Return 1.\n",
        "## To f:\n    Return 1.",
        "\n\n## To f (x: Int) -> Int:\n    Return x.\n\n\n## To g:\n    Show 2.\n## Main\n    Show f(1).\n",
        "## To native now -> Int\n## To private h:\n  \n    Show 3.\n   \n## A Point has:\n    an x: Int.\n## To k:\n",
        "prose\n## To  spaced:\n    Show 1.\n## Note\nA note.\n## To last:\n    Show 2.\n\n",
    ] {
        assert_eq!(sections_by_ranges(code), sections_by_lines(code), "{code:?}");
    }
    for source in crate::loader::prelude_module_sources() {
        let code = spliced_code(source);
        assert_eq!(sections_by_ranges(&code), sections_by_lines(&code));
    }
}

#[test]
fn line_runs_rebuild_a_sequence_of_lines() {
    let source = "a\nb\nc\nd\n";
    assert_eq!(line_runs(source, "a\nb\nc\nd\n"), Some(vec![(0, 8)]));
    assert_eq!(line_runs(source, "a\nc\n"), Some(vec![(0, 2), (4, 6)]));
    assert_eq!(line_runs(source, "b\nc\nd"), Some(vec![(2, 7)]));
    assert_eq!(line_runs(source, "a\nd"), Some(vec![(0, 2), (6, 7)]));
    assert_eq!(line_runs(source, ""), Some(vec![]));
    assert_eq!(line_runs(source, "a\ne\n"), None, "a line the source lacks");
    assert_eq!(line_runs(source, "c\na\n"), None, "lines out of order");
    assert_eq!(
        line_runs(source, "b"),
        Some(vec![(2, 3)]),
        "the last line may drop its newline"
    );
    assert_eq!(line_runs(source, "b\nc"), Some(vec![(2, 5)]));
    assert_eq!(line_runs("x\n\ny\n", "x\ny"), Some(vec![(0, 2), (3, 4)]));
    assert_eq!(line_runs("x\n\n\ny\n", "\ny"), Some(vec![(2, 3), (4, 5)]));
}

#[test]
fn a_range_maps_to_the_source_only_inside_one_run() {
    let runs = [(10, 14), (20, 30)];
    assert_eq!(source_range(&runs, 0..4), Some((10, 14)));
    assert_eq!(source_range(&runs, 1..3), Some((11, 13)));
    assert_eq!(source_range(&runs, 4..14), Some((20, 30)));
    assert_eq!(source_range(&runs, 6..9), Some((22, 25)));
    assert_eq!(source_range(&runs, 3..6), None, "a range across the gap");
    assert_eq!(source_range(&runs, 12..15), None, "a range past the code");
}

#[test]
fn whole_words_are_the_maximal_identifier_runs() {
    let words = whole_words("Show trim(x_1) + café.\n## To f2:");
    let mut sorted: Vec<&str> = words.into_iter().collect();
    sorted.sort();
    assert_eq!(sorted, vec!["Show", "To", "caf", "f2", "trim", "x_1"]);
}

#[test]
fn a_name_absent_from_the_words_is_never_found() {
    let sources = [
        "## Main\nShow trim(s).\n",
        "Let MessageBox be 1.\nLet nowhere be 2.\nCall flush with xs.\n",
        "## To rank (s: Severity) -> Int:\n    Return 1.\n",
        "A Thing has:\n    a list, which is Seq of Message.\n",
        "Show \"Message\". Launch a task to flush.\n",
        "x_trim(1) trim_(2) 9trim(3) trim9(4) é trim(5)",
    ];
    let names = [
        "trim", "Message", "now", "flush", "Severity", "rank", "s", "x", "Seq", "List", "Box", "é",
        "a-b", "", "9trim",
    ];
    for source in sources {
        let words = whole_words(source);
        for name in names {
            if !may_name(&words, name) {
                assert!(!references(source, name), "{name:?} in {source:?}");
            }
        }
    }
    assert!(
        may_name(&whole_words("x"), "é"),
        "a name outside identifier bytes is left to the scan"
    );
    assert!(may_name(&whole_words("x"), "a-b"));
    assert!(!may_name(&whole_words("trimmed"), "trim"));
}

const MODULE_A: &str = "# A\n\nprose\n\n## Note\nAbout f.\n\nMore about f.\n## To f (x: Int) -> Int:\n    Return g(x).\n\n## To native clock -> Int\n\n## Definition\nA Map of [K] and [V] is a generic collection.\nA Shape is either:\n    A Dot.\n";
const MODULE_B: &str =
    "# B\n\n## To g (x: Int) -> Int:\n    Return x.\n\n## To f (x: Int) -> Int:\n    Return 2.\n";
const MODULE_C: &str = "# C\n\n## To h:\n    Show text_from_bytes(b).\n    Let s be a new Shape.\n";

#[test]
fn the_index_of_three_modules() {
    let ix = index(&[MODULE_A, MODULE_B, MODULE_C]);
    assert_eq!(ix.names[0], vec!["f", "clock", "Map", "Shape", "Dot"]);
    assert_eq!(ix.names[1], vec!["g", "f"]);
    assert_eq!(ix.names[2], vec!["h"]);
    assert_eq!(ix.natives, vec![vec!["clock".to_string()], vec![], vec![]]);
    assert_eq!(
        ix.pulls,
        vec![vec![1], vec![], vec![0]],
        "A calls g; B only declares an `f` of its own; C makes a Shape"
    );
    assert_eq!(ix.mentions_text_from_bytes, vec![false, false, true]);
    for (m, source) in [MODULE_A, MODULE_B, MODULE_C].iter().enumerate() {
        let rebuilt: String = ix.splice[m].iter().map(|&(s, e)| &source[s..e]).collect();
        assert_eq!(rebuilt, spliced_code(source));
    }
    assert!(!spliced_code(MODULE_A).contains("About f"));
    assert!(!spliced_code(MODULE_A).contains("A Map of"));
    let library: Vec<(&str, usize, &str)> = ix
        .library
        .iter()
        .map(|(n, m, s, e)| (n.as_str(), *m, &[MODULE_A, MODULE_B, MODULE_C][*m][*s..*e]))
        .collect();
    assert_eq!(
        library,
        vec![
            ("clock", 0, "## To native clock -> Int"),
            ("f", 1, "## To f (x: Int) -> Int:\n    Return 2."),
            ("g", 1, "## To g (x: Int) -> Int:\n    Return x."),
            (
                "h",
                2,
                "## To h:\n    Show text_from_bytes(b).\n    Let s be a new Shape."
            ),
        ],
        "sorted by name, and the later module's `f`"
    );
    assert_eq!(ix.functions, vec!["clock", "f", "g", "h"]);
}

#[test]
#[should_panic(expected = "spans bytes its splice strips")]
fn a_library_section_the_splice_cuts_stops_the_build() {
    index(&["## To f:\n    Show 1.\nA List of [T] is a generic collection.\n    Show 2.\n"]);
}
