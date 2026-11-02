use super::*;
use crate::ui_bridge::with_parsed_program;

/// The cap `match_softcap` reads in `src`'s statements, and the count of elements it caps.
fn softcap_of(src: &str) -> Option<(f64, usize)> {
    with_parsed_program(src, |parsed, _| {
        let (stmts, _, _) = parsed.unwrap_or_else(|e| panic!("parses: {e}\n{src}"));
        recognize_softcap(stmts).map(|s| (s.cap.get(), s.n))
    })
}

/// `Y` bound to `element` of each `x = item p of X` over `X` (1 by 4), in the loop `Let Y be X capped at C` desugars to,
/// written out by hand.
fn by_hand(element: &str) -> String {
    let mut s = "## Main\nLet X be a 1 by 4 tensor of Real.\n".to_string();
    for (i, x) in [2.0, -4.0, 6.0, 8.0].iter().enumerate() {
        s.push_str(&format!("Set item {} of X to {x:?}.\n", i + 1));
    }
    s.push_str("Let mutable Y be a new Seq of Real.\nLet mutable p be 1.\nWhile p <= 4:\n");
    s.push_str(&format!("    Push {} to Y.\n    Set p to p + 1.\n", element.replace("x", "(item p of X)")));
    s
}

#[test]
fn the_cap_is_read_from_its_sugar_and_from_its_loop_written_out() {
    assert_eq!(
        softcap_of("## Main\nLet X be a 1 by 4 tensor of Real.\nLet Y be X capped at 5.0.\n"),
        Some((5.0, 4)),
        "the sugar"
    );
    assert_eq!(
        softcap_of(&by_hand("5.0 * (1.0 - 2.0 / (2.718281828459045 ** (x * 0.4) + 1.0))")),
        Some((5.0, 4)),
        "the sugar's loop written out is a cap at 5 over 4 elements"
    );
}

#[test]
fn no_near_miss_of_the_caps_form_is_read_as_a_cap() {
    for (element, what) in [
        ("5.0 * (1.0 - 2.0 / (2.718281828459045 ** (x * 0.2) + 1.0))", "a rate of 1/C"),
        ("5.0 * (1.0 - 2.0 / (2.718281828459045 ** (x * 0.8) + 1.0))", "a rate of 4/C"),
        ("5.0 * (1.5 - 2.0 / (2.718281828459045 ** (x * 0.4) + 1.0))", "a shift other than 1"),
        ("5.0 * (1.0 - 3.0 / (2.718281828459045 ** (x * 0.4) + 1.0))", "a numerator other than 2"),
        ("5.0 * (1.0 - 2.0 / (2.718281828459045 ** (x * 0.4) + 2.0))", "a denominator's term other than 1"),
        ("5.0 * (1.0 - 2.0 / (2.7 ** (x * 0.4) + 1.0))", "a base other than e"),
        ("5.0 * (1.0 + 2.0 / (2.718281828459045 ** (x * 0.4) + 1.0))", "a sum in place of the difference"),
        ("5.0 * (1.0 - 2.0 / (2.718281828459045 ** (x * 0.4) * 1.0))", "a product in place of the denominator's sum"),
        ("(1.0 - 2.0 / (2.718281828459045 ** (x * 0.4) + 1.0)) * 5.0", "the cap after the bracket"),
    ] {
        assert_eq!(softcap_of(&by_hand(element)), None, "{what} is no cap: {element}");
    }
}
