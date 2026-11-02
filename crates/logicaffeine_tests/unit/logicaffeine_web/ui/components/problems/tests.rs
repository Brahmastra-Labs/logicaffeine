use super::*;

fn found(source: &str) -> Vec<Problem> {
    problems(&DocumentState::new(source.to_string(), 0))
}

fn problem(start: usize, end: usize, severity: Severity) -> Problem {
    Problem {
        start,
        end,
        severity,
        unnecessary: false,
        message: "m".to_string(),
        source: Some("logicaffeine".to_string()),
        code: None,
        docs: None,
    }
}

const TWO_BAD_FUNCTIONS: &str = "## To f () -> Int:\n    Return \"oops\".\n\n## To g () -> Int:\n    Return \"worse\".\n\n## Main\nShow 1.\n";

/// A squiggle points at what is wrong: every error in a guide example that fails on purpose
/// underlines words — never only punctuation, and never the `##` of a block header.
#[test]
fn every_expected_error_underlines_what_is_wrong() {
    use crate::ui::pages::guide::content::{Runnability, SECTIONS};
    let mut wrong = Vec::new();
    let mut errors = 0;
    for section in SECTIONS.iter() {
        for example in section.examples.iter().filter(|e| matches!(e.runnability, Runnability::ExpectedError { .. })) {
            for p in found(example.code).into_iter().filter(|p| p.severity == Severity::Error) {
                errors += 1;
                let text = &example.code[p.start..p.end];
                if !text.chars().any(char::is_alphanumeric) || text.trim_start().starts_with("##") {
                    wrong.push(format!("{}: {:?} underlines {text:?}", example.id, p.code));
                }
            }
        }
    }
    assert!(errors > 0, "no guide example reports an error to check");
    assert!(wrong.is_empty(), "{} error(s) underline no words:\n{}", wrong.len(), wrong.join("\n"));
}

#[test]
fn a_type_mismatch_is_an_error_under_its_statement() {
    let source = TWO_BAD_FUNCTIONS;
    let errors: Vec<Problem> = found(source)
        .into_iter()
        .filter(|p| p.code.as_deref() == Some("type-mismatch"))
        .collect();
    assert_eq!(errors.len(), 2, "both bad functions report: {errors:#?}");
    for p in &errors {
        assert_eq!(p.severity, Severity::Error);
        assert!(p.start < p.end && p.end <= source.len(), "{p:?} is not a span of the source");
        assert!(!source[p.start..p.end].trim().is_empty(), "{p:?} covers only blank space");
        assert!(p.message.contains("Int") && p.message.contains("Text"), "{}", p.message);
        assert_eq!(p.source.as_deref(), Some("logicaffeine"));
    }
}

#[test]
fn an_unused_binding_is_faded_and_explained() {
    let source = "## Main\nLet x be 5.\n";
    let unused: Vec<Problem> = found(source)
        .into_iter()
        .filter(|p| p.code.as_deref() == Some("unused-variable"))
        .collect();
    assert_eq!(unused.len(), 1, "{unused:#?}");
    let p = &unused[0];
    assert_eq!((p.severity, p.unnecessary), (Severity::Hint, true));
    assert_eq!(&source[p.start..p.end], "x");
    assert!(p.docs.as_deref().is_some_and(|d| d.starts_with("https://")), "{:?}", p.docs);
    assert_eq!(squiggle_class(&unused, p.start, p.end), "sq-unnecessary");
}

#[test]
fn a_clean_program_has_no_problems() {
    assert_eq!(found("## Main\nLet x be 5.\nShow x.\n"), Vec::new());
}

/// Where `name` is read in `source`: every whole-word use outside comment lines.
fn reads_of(source: &str, name: &str) -> Vec<usize> {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = Vec::new();
    let mut line_start = 0;
    for line in source.split_inclusive('\n') {
        let code = line.trim_start();
        if !(code.starts_with("--") || (code.starts_with('#') && !code.starts_with("##"))) {
            for (i, _) in line.match_indices(name) {
                let before = line[..i].chars().next_back();
                let after = line[i + name.len()..].chars().next();
                if !before.is_some_and(word) && !after.is_some_and(word) {
                    out.push(line_start + i);
                }
            }
        }
        line_start += line.len();
    }
    out
}

#[test]
fn a_name_read_inside_text_is_used() {
    let unused: Vec<Problem> = found("## Main\nLet n be 10.\nShow \"The first {n} numbers\".\n")
        .into_iter().filter(|p| p.code.as_deref() == Some("unused-variable")).collect();
    assert_eq!(unused, Vec::new());
}

#[test]
fn a_name_read_as_a_loop_bound_is_used() {
    let unused: Vec<Problem> = found("## Main\nLet n be 3.\nRepeat for i from 1 to n:\n    Show i.\n")
        .into_iter().filter(|p| p.code.as_deref() == Some("unused-variable")).collect();
    assert_eq!(unused, Vec::new());
}

/// "Never used" is a promise: no example is told a variable it reads is unused.
#[test]
fn no_example_is_told_a_variable_it_reads_is_never_used() {
    let mut wrong = Vec::new();
    for spec in crate::ui::examples::ALL_CODE_EXAMPLES {
        for p in found(spec.source).into_iter().filter(|p| p.code.as_deref() == Some("unused-variable")) {
            let name = &spec.source[p.start..p.end];
            let reads = reads_of(spec.source, name).into_iter().filter(|&at| at != p.start).count();
            if reads > 0 {
                let line = spec.source[..p.start].matches('\n').count() + 1;
                wrong.push(format!("{}:{line} '{name}' is read {reads} more time(s)", spec.vfs_path));
            }
        }
    }
    assert!(wrong.is_empty(), "{} false \"never used\" hint(s):\n{}", wrong.len(), wrong.join("\n"));
}

#[test]
fn every_code_example_opens_without_an_error_or_warning() {
    let loud: Vec<String> = crate::ui::examples::ALL_CODE_EXAMPLES
        .iter()
        .flat_map(|spec| {
            found(spec.source)
                .into_iter()
                .filter(|p| p.severity >= Severity::Warning)
                .map(move |p| {
                    let line = spec.source[..p.start].matches('\n').count() + 1;
                    format!(
                        "{}:{line} {:?} {} on {:?}: {}",
                        spec.vfs_path,
                        p.severity,
                        p.code.as_deref().unwrap_or("-"),
                        &spec.source[p.start..p.end],
                        p.message.lines().next().unwrap_or("")
                    )
                })
        })
        .collect();
    assert!(loud.is_empty(), "{} example problem(s):\n{}", loud.len(), loud.join("\n"));
}

#[test]
fn a_problem_on_blank_space_underlines_the_word_before_it() {
    assert_eq!(word_span("Show x.\n", 7, 8), (5, 7), "past the line's end → \"x.\"");
    assert_eq!(word_span("Let x be 5.", 3, 3), (0, 3), "an empty range on a space → \"Let\"");
}

#[test]
fn an_empty_range_inside_a_word_underlines_that_word() {
    assert_eq!(word_span("Let mutable a be 0.", 6, 6), (4, 11));
}

#[test]
fn a_problem_at_the_very_start_underlines_the_first_word() {
    assert_eq!(word_span("  Show x.", 0, 1), (2, 6));
}

#[test]
fn a_range_that_shows_text_is_kept_as_it_is() {
    assert_eq!(word_span("Let x be 5.", 4, 5), (4, 5));
    assert_eq!(word_span("Let x be 5.", 3, 6), (3, 6));
}

#[test]
fn a_blank_source_still_gets_a_nonempty_span_inside_it() {
    assert_eq!(word_span("   ", 1, 1), (1, 2));
    assert_eq!(word_span("", 0, 0), (0, 0));
}

#[test]
fn a_problem_never_splits_a_character() {
    let source = "Show \"é\".";
    let (start, end) = word_span(source, 7, 7);
    assert!(source.is_char_boundary(start) && source.is_char_boundary(end), "({start}, {end})");
    assert_eq!(&source[start..end], "\"é\".");
}

#[test]
fn the_most_severe_overlap_names_the_squiggle() {
    let ps = [problem(0, 5, Severity::Warning), problem(3, 8, Severity::Error)];
    assert_eq!(squiggle_class(&ps, 0, 2), "sq-warning");
    assert_eq!(squiggle_class(&ps, 3, 5), "sq-error");
    assert_eq!(squiggle_class(&ps, 6, 8), "sq-error");
    assert_eq!(squiggle_class(&ps, 8, 10), "");
}

#[test]
fn each_severity_has_its_own_squiggle() {
    for (severity, class) in [
        (Severity::Error, "sq-error"),
        (Severity::Warning, "sq-warning"),
        (Severity::Info, "sq-info"),
        (Severity::Hint, "sq-hint"),
    ] {
        assert_eq!(squiggle_class(&[problem(0, 3, severity)], 0, 3), class);
    }
}

#[test]
fn unnecessary_code_fades_under_any_squiggle_over_it() {
    let mut unused = problem(0, 3, Severity::Hint);
    unused.unnecessary = true;
    assert_eq!(squiggle_class(&[unused.clone()], 0, 3), "sq-unnecessary");
    assert_eq!(squiggle_class(&[unused, problem(0, 3, Severity::Error)], 0, 3), "sq-error sq-unnecessary");
}

#[test]
fn the_hover_leads_with_the_problem_and_links_its_code() {
    let mut p = problem(4, 5, Severity::Hint);
    p.message = "'x' is never used.\n\nRemove the statement, or use the value.".to_string();
    p.code = Some("unused-variable".to_string());
    p.docs = Some("https://example.com/guide#vars".to_string());
    let html = problems_html(&[p], 4).expect("a problem sits at offset 4");
    assert_eq!(
        html,
        "<div class=\"hint-problem\"><p>'x' is never used.</p><p>Remove the statement, or use the value.</p>\
             <p class=\"hint-source\">logicaffeine(<a href=\"https://example.com/guide#vars\" target=\"_blank\" \
             rel=\"noopener noreferrer\">unused-variable</a>)</p></div>"
    );
}

#[test]
fn the_hover_shows_nothing_where_there_is_no_problem() {
    let ps = [problem(4, 5, Severity::Error)];
    assert_eq!(problems_html(&ps, 3), None);
    assert_eq!(problems_html(&ps, 5), None);
    assert!(problems_html(&ps, 4).is_some());
}

#[test]
fn the_hover_lists_every_problem_there_most_severe_first() {
    let mut quiet = problem(0, 6, Severity::Hint);
    quiet.message = "quiet".to_string();
    let mut loud = problem(2, 4, Severity::Error);
    loud.message = "loud".to_string();
    let html = problems_html(&[quiet, loud], 3).unwrap();
    let (l, q) = (html.find("loud").unwrap(), html.find("quiet").unwrap());
    assert!(l < q, "the error comes first: {html}");
    assert_eq!(html.matches("class=\"hint-problem\"").count(), 2, "{html}");
}

#[test]
fn the_hover_names_a_code_it_cannot_link() {
    let mut p = problem(0, 1, Severity::Error);
    p.code = Some("type-mismatch".to_string());
    p.docs = Some("javascript:alert(1)".to_string());
    let html = problems_html(&[p], 0).unwrap();
    assert!(!html.contains("<a"), "{html}");
    assert!(html.contains("logicaffeine(type-mismatch)"), "{html}");
}

#[test]
fn the_hover_escapes_the_message() {
    let mut p = problem(0, 1, Severity::Error);
    p.message = "expected <Int> & got \"Text\"".to_string();
    let html = problems_html(&[p], 0).unwrap();
    assert!(html.contains("expected &lt;Int&gt; &amp; got &quot;Text&quot;"), "{html}");
}

#[test]
fn an_edit_before_a_problem_moves_it() {
    let old = "Let x be 5.\nShow y.\n";
    let new = "Let xyz be 5.\nShow y.\n";
    let y = old.find("y.").unwrap();
    let moved = rebase(&[problem(y, y + 1, Severity::Error)], old, new);
    assert_eq!(moved.len(), 1);
    assert_eq!(&new[moved[0].start..moved[0].end], "y");
}

#[test]
fn an_edit_after_a_problem_leaves_it() {
    let old = "Show y.\nLet x be 5.\n";
    let new = "Show y.\nLet x be 50.\n";
    let kept = rebase(&[problem(5, 6, Severity::Error)], old, new);
    assert_eq!(kept, vec![problem(5, 6, Severity::Error)]);
}

#[test]
fn an_edit_inside_a_problem_drops_it() {
    let old = "Show yes.\n";
    let new = "Show yeas.\n";
    assert_eq!(rebase(&[problem(5, 8, Severity::Error)], old, new), Vec::new());
}

#[test]
fn an_unchanged_source_keeps_every_problem() {
    let ps = vec![problem(0, 4, Severity::Warning), problem(5, 6, Severity::Error)];
    assert_eq!(rebase(&ps, "Show y.", "Show y."), ps);
}

#[test]
fn a_rebased_problem_never_splits_a_character() {
    let old = "Show \"é\". Show z.";
    let new = "Show \"è\". Show z.";
    let z = old.rfind('z').unwrap();
    let moved = rebase(&[problem(z, z + 1, Severity::Error)], old, new);
    assert_eq!(moved.len(), 1);
    assert_eq!(&new[moved[0].start..moved[0].end], "z");
}

#[test]
fn deleting_the_text_under_a_problem_drops_it() {
    let old = "Show y. Show z.";
    let new = "Show z.";
    let z = old.rfind('z').unwrap();
    let ps = [problem(5, 6, Severity::Error), problem(z, z + 1, Severity::Warning)];
    let moved = rebase(&ps, old, new);
    assert_eq!(moved.len(), 1, "{moved:?}");
    assert_eq!(&new[moved[0].start..moved[0].end], "z");
}
