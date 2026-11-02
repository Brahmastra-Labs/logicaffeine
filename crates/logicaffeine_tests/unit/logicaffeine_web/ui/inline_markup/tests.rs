use super::*;

#[path = "shown_as_text.rs"]
mod shown_as_text;
use shown_as_text::markup_shown_as_text;

fn html(s: &str) -> String {
    inline_markup_html(s)
}

#[test]
fn rendered_markup_leaves_nothing_shown_as_text() {
    assert!(markup_shown_as_text("<p>an <em>x</em> and <strong>y</strong> <a href=\"/z\">z</a></p><hr/>").is_empty());
    assert!(markup_shown_as_text("<pre><code>**a** *b* [c](/d)\n---</code></pre><p><code>*x*</code></p>").is_empty());
    assert!(markup_shown_as_text("<p>2 * 3 and A* is B*</p>").is_empty());
}

#[test]
fn markup_left_as_text_is_found() {
    for page in ["<p>an *x* y</p>", "<p>a **b** c</p>", "<p>see [a](/b)</p>", "<p>---</p>", "<li>*Try it:* go</li>"] {
        assert_eq!(markup_shown_as_text(page).len(), 1, "{page}: {:?}", markup_shown_as_text(page));
    }
}

#[test]
fn bold_pairs_open_and_close() {
    assert_eq!(html("**Logic** is the analysis of arguments."), "<strong>Logic</strong> is the analysis of arguments.");
}

#[test]
fn emphasis_pairs_open_and_close() {
    assert_eq!(html("check the *form* of arguments"), "check the <em>form</em> of arguments");
}

#[test]
fn several_marks_in_one_paragraph_each_close() {
    assert_eq!(
        html("An argument is **valid** if the premises *are* true and **sound** if *also* true."),
        "An argument is <strong>valid</strong> if the premises <em>are</em> true and <strong>sound</strong> if <em>also</em> true."
    );
}

#[test]
fn nested_strong_inside_emphasis() {
    assert_eq!(html("*a **b** c*"), "<em>a <strong>b</strong> c</em>");
}

#[test]
fn nested_emphasis_inside_strong() {
    assert_eq!(html("**bold with *em* inside**"), "<strong>bold with <em>em</em> inside</strong>");
}

#[test]
fn triple_stars_are_strong_emphasis() {
    assert_eq!(html("***both***"), "<em><strong>both</strong></em>");
}

#[test]
fn the_star_test_keeps_its_literal_stars() {
    assert_eq!(html("all A* is B"), "all A* is B");
    assert_eq!(html("no A* is B*"), "no A* is B*");
    assert_eq!(html("S*-D, D-U, S-U*"), "S*-D, D-U, S-U*");
    assert_eq!(html("∴ a* is f*"), "∴ a* is f*");
}

#[test]
fn spaced_stars_are_multiplication_not_emphasis() {
    assert_eq!(html("2 * 3 * 4"), "2 * 3 * 4");
}

#[test]
fn an_unmatched_marker_prints_as_written() {
    assert_eq!(html("**unclosed and fine"), "**unclosed and fine");
    assert_eq!(html("a *lonely star"), "a *lonely star");
    assert_eq!(html("trailing**"), "trailing**");
}

#[test]
fn text_is_escaped_before_markup() {
    assert_eq!(html("x < y & y > z"), "x &lt; y &amp; y &gt; z");
    assert_eq!(html("<script>alert(1)</script>"), "&lt;script&gt;alert(1)&lt;/script&gt;");
    assert_eq!(html("say \"hi\" it's"), "say &quot;hi&quot; it&#39;s");
    assert_eq!(html("**<b>**"), "<strong>&lt;b&gt;</strong>");
}

#[test]
fn code_spans_are_literal() {
    assert_eq!(html("run `a*b*c` now"), "run <code>a*b*c</code> now");
    assert_eq!(html("`x < y`"), "<code>x &lt; y</code>");
    assert_eq!(html("``a ` b``"), "<code>a ` b</code>");
}

#[test]
fn an_unclosed_backtick_is_text() {
    assert_eq!(html("a ` b *c*"), "a ` b <em>c</em>");
}

#[test]
fn underscores_are_left_alone() {
    assert_eq!(html("snake_case_name and _this_"), "snake_case_name and _this_");
}

#[test]
fn unicode_and_logic_symbols_pass_through() {
    assert_eq!(html("**∀x** (P(x) → Q(x))"), "<strong>∀x</strong> (P(x) → Q(x))");
}

#[test]
fn empty_and_plain_text() {
    assert_eq!(html(""), "");
    assert_eq!(html("plain"), "plain");
    assert_eq!(html("*"), "*");
    assert_eq!(html("**"), "**");
    assert_eq!(html("****"), "****");
}

/// Tags must come out balanced and properly nested, whatever the input.
fn assert_well_formed(input: &str, output: &str) {
    let mut stack = Vec::new();
    let mut rest = output;
    while let Some(i) = rest.find('<') {
        let tail = &rest[i..];
        let end = tail.find('>').unwrap_or_else(|| panic!("unterminated tag in {output:?} (from {input:?})"));
        let tag = &tail[1..end];
        if let Some(name) = tag.strip_prefix('/') {
            assert_eq!(stack.pop(), Some(name.to_string()), "misnested </{name}> in {output:?} (from {input:?})");
        } else {
            assert!(["strong", "em", "code"].contains(&tag), "unexpected tag <{tag}> in {output:?} (from {input:?})");
            stack.push(tag.to_string());
        }
        rest = &tail[end + 1..];
    }
    assert!(stack.is_empty(), "unclosed {stack:?} in {output:?} (from {input:?})");
}

#[test]
fn every_arrangement_of_marks_renders_well_formed() {
    let atoms = ["*", "**", "***", "a", " ", "b*", "*c", "`", "<", "&", "x_y"];
    let mut n = 0;
    for a in atoms {
        for b in atoms {
            for c in atoms {
                for d in atoms {
                    let input = format!("{a}{b}{c}{d}");
                    assert_well_formed(&input, &html(&input));
                    n += 1;
                }
            }
        }
    }
    assert_eq!(n, atoms.len().pow(4));
}

#[test]
fn stars_are_conserved() {
    // Every `*` in the input is either printed or consumed by exactly one tag side.
    for input in ["**a** *b* c*", "***x*** y", "*a **b** c*", "a* b* *c", "** a **"] {
        let out = html(input);
        let printed = out.matches('*').count();
        let strong = out.matches("<strong>").count();
        let em = out.matches("<em>").count();
        assert_eq!(printed + 4 * strong + 2 * em, input.matches('*').count(), "{input:?} -> {out:?}");
    }
}

#[test]
fn a_link_renders_as_an_anchor() {
    assert_eq!(html("see [the guide](/guide) now"), "see <a href=\"/guide\">the guide</a> now");
    assert_eq!(
        html("[the crate](https://crates.io/crates/logicaffeine-lsp)"),
        "<a href=\"https://crates.io/crates/logicaffeine-lsp\">the crate</a>"
    );
}

#[test]
fn a_link_escapes_its_url_and_marks_up_its_text() {
    assert_eq!(
        html("[**bold** link](https://x.dev/?a=1&b=\"2\")"),
        "<a href=\"https://x.dev/?a=1&amp;b=&quot;2&quot;\"><strong>bold</strong> link</a>"
    );
}

#[test]
fn emphasis_reaches_across_a_link() {
    assert_eq!(html("*see [a](/b) now*"), "<em>see <a href=\"/b\">a</a> now</em>");
}

#[test]
fn only_a_web_or_site_address_is_a_link() {
    assert_eq!(html("[x](javascript:alert(1))"), "[x](javascript:alert(1))");
    assert_eq!(html("[x](data:text/html,hi)"), "[x](data:text/html,hi)");
}

#[test]
fn brackets_that_name_no_address_are_text() {
    assert_eq!(html("an array [1, 2] here"), "an array [1, 2] here");
    assert_eq!(html("[unclosed](/x"), "[unclosed](/x");
}

#[test]
fn emphasis_reaches_across_code() {
    assert_eq!(
        html("**spelling: `==`** and *change `x` to `y`*"),
        "<strong>spelling: <code>==</code></strong> and <em>change <code>x</code> to <code>y</code></em>"
    );
    assert_eq!(html("**`Rc<T>`** wraps"), "<strong><code>Rc&lt;T&gt;</code></strong> wraps");
}

#[test]
fn stars_inside_code_are_code() {
    assert_eq!(html("*a `*b*` c*"), "<em>a <code>*b*</code> c</em>");
}

#[test]
fn a_link_inside_code_stays_code() {
    assert_eq!(html("`[a](/b)`"), "<code>[a](/b)</code>");
}

#[test]
fn every_shipped_paragraph_renders_well_formed_with_no_stray_stars() {
    use crate::content::{ContentBlock, ContentEngine};
    let mut checked = 0;
    for era in ContentEngine::shared().eras() {
        for module in &era.modules {
            for section in &module.sections {
                for block in &section.content {
                    if let ContentBlock::Paragraph { text } = block {
                        let out = html(text);
                        assert_well_formed(text, &out);
                        assert!(
                            !out.contains("**"),
                            "{}/{} {}: a ** survived rendering: {out:?}",
                            era.meta.id, module.meta.id, section.id
                        );
                        checked += 1;
                    }
                }
            }
        }
    }
    assert!(checked > 100, "only {checked} paragraphs checked");
}
