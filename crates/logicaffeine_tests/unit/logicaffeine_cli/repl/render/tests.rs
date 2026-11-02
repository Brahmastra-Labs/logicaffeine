use super::*;

#[test]
fn colorize_wraps_operators_when_on() {
    let out = colorize_fol("∀x(Man(x) → Mortal(x))", true);
    assert!(out.contains("\x1b["), "must contain ANSI: {out:?}");
    assert!(out.contains("Man(x)"), "content preserved: {out:?}");
}

#[test]
fn colorize_passes_through_when_off() {
    let fol = "∀x(Man(x) → Mortal(x))";
    assert_eq!(colorize_fol(fol, false), fol);
}

#[test]
fn vars_table_aligns_columns() {
    let rows = vec![
        ("x".to_string(), "Int".to_string(), "5".to_string()),
        ("name".to_string(), "Text".to_string(), "logos".to_string()),
    ];
    let table = render_vars(&rows);
    assert!(table.contains("x    : Int  = 5"), "table:\n{table}");
    assert!(table.contains("name : Text = logos"), "table:\n{table}");
}

#[test]
fn empty_vars_say_so() {
    assert_eq!(render_vars(&[]), "no bindings yet");
}

#[test]
fn explain_renders_the_full_lesson() {
    let out = render_explain("give", false);
    assert!(out.contains("Give"), "{out}");
    assert!(
        out.contains("Transfers ownership of a value to a new owner."),
        "the lesson's one-liner must render: {out}"
    );
    assert!(
        out.contains("    Give x to processor."),
        "indented example: {out}"
    );
    assert!(
        out.contains('?'),
        "the socratic question must render: {out}"
    );
}

#[test]
fn explain_shows_every_matching_lesson() {
    // `Set` names both the statement and the type — an honest teacher
    // shows both.
    let out = render_explain("set", false);
    assert!(
        out.contains("mutable variable"),
        "the statement lesson: {out}"
    );
    assert!(out.contains("each value once"), "the type lesson: {out}");
    assert!(out.contains("---"), "multiple lessons are separated: {out}");
}

#[test]
fn explain_suggests_on_a_near_miss() {
    let out = render_explain("shw", false);
    assert!(
        out.contains(":explain Show") || out.contains(":explain show"),
        "a typo should suggest the nearest lesson: {out}"
    );
}

#[test]
fn explain_color_never_alters_content() {
    // Remove ANSI SGR sequences (`ESC [ … m`).
    fn strip_ansi(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                for e in chars.by_ref() {
                    if e == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }
    let plain = render_explain("repeat", false);
    let colored = render_explain("repeat", true);
    assert_eq!(
        strip_ansi(&colored),
        plain,
        "ANSI styling must only style, never reword"
    );
}

#[test]
fn vars_table_aligns_by_characters_not_bytes() {
    let rows = vec![
        ("π".to_string(), "Float".to_string(), "3.14".to_string()),
        ("xy".to_string(), "Int".to_string(), "1".to_string()),
    ];
    let table = render_vars(&rows);
    let cols: Vec<usize> = table
        .lines()
        .map(|l| l.chars().take_while(|c| *c != ':').count())
        .collect();
    assert_eq!(
        cols[0], cols[1],
        "the `:` column must align in chars:\n{table}"
    );
}
