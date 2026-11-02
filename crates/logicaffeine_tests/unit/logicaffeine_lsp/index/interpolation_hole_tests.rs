use super::interpolation_holes;

fn texts(raw: &str) -> Vec<&str> {
    interpolation_holes(raw).into_iter().map(|r| &raw[r]).collect()
}

#[test]
fn each_hole_is_the_text_between_its_braces() {
    assert_eq!(texts("\"The first {n} of {total}\""), ["n", "total"]);
}

#[test]
fn a_doubled_brace_is_a_literal_not_a_hole() {
    assert_eq!(texts("\"{{not}} {x}\""), ["x"]);
}

#[test]
fn a_hole_keeps_its_format_and_nested_braces() {
    assert_eq!(texts("\"{price:.2} {f({x})}\""), ["price:.2", "f({x})"]);
}

#[test]
fn an_unclosed_hole_is_no_hole() {
    assert_eq!(texts("\"{oops\""), Vec::<&str>::new());
}
