use super::*;

/// Remove ANSI SGR sequences (`ESC [ … m`).
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

#[test]
fn highlighting_never_alters_the_line_content() {
    for line in [
        "Let x be 5.",
        "Show \"Hello, {name}!\".",
        "## To double (n: Int) -> Int:",
        "If x is at least 3:",
        ":help",
        "gibberish that will not parse ~~ ///",
        "",
        "Let café be \"héllo\".",
    ] {
        assert_eq!(
            strip_ansi(&highlight_line(line)),
            line,
            "highlighting must only ADD escapes"
        );
    }
}

#[test]
fn keywords_strings_and_numbers_get_painted() {
    let painted = highlight_line("Let x be 5.");
    assert!(painted.contains('\u{1b}'), "keywords paint: {painted:?}");

    let painted = highlight_line("Show \"hi\".");
    let string_style = style_for(TokenClass::String).to_string();
    assert!(
        painted.contains(&format!("{string_style}\"hi\"")),
        "strings paint green: {painted:?}"
    );

    let painted = highlight_line("Let n be 42.");
    let number_style = style_for(TokenClass::Number).to_string();
    assert!(
        painted.contains(&format!("{number_style}42")),
        "numbers paint: {painted:?}"
    );
}

#[test]
fn meta_commands_paint_as_a_unit() {
    let painted = highlight_line(":vars");
    assert!(
        painted.starts_with('\u{1b}'),
        "meta prefix paints: {painted:?}"
    );
    assert_eq!(strip_ansi(&painted), ":vars");
}
