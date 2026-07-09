//! REPL output rendering — pure string transforms (color decisions are the
//! caller's; everything routes through anstream at the print site).

use anstyle::AnsiColor;

/// Highlight logical operators in a FOL string (cyan bold). With `color`
/// off, the input passes through untouched.
pub fn colorize_fol(fol: &str, color: bool) -> String {
    if !color {
        return fol.to_string();
    }
    const OPERATORS: [char; 12] = ['∀', '∃', '∧', '∨', '→', '↔', '¬', '□', '◇', '⊕', 'λ', '⊤'];
    let style = AnsiColor::Cyan.on_default().bold();
    let mut out = String::with_capacity(fol.len() * 2);
    for ch in fol.chars() {
        if OPERATORS.contains(&ch) {
            out.push_str(&format!("{style}{ch}{style:#}"));
        } else {
            out.push(ch);
        }
    }
    out
}

/// Render `:explain <word>` — the same lesson table the LSP's hover and
/// completion teach from (`logicaffeine_language::teach`), styled for the
/// terminal. A word can name more than one construct (`Set` the statement,
/// `Set` the type); every match renders. A miss suggests the closest name.
pub fn render_explain(word: &str, color: bool) -> String {
    use logicaffeine_language::teach::{docs_for_word, ALL_DOCS};

    let lessons = docs_for_word(word);
    if lessons.is_empty() {
        let names: Vec<&str> = ALL_DOCS.iter().map(|d| d.name).collect();
        return match logicaffeine_language::suggest::find_similar(word, &names, 2) {
            Some(similar) => {
                format!("nothing taught for `{word}` — did you mean `:explain {similar}`?")
            }
            None => format!("nothing taught for `{word}` — try :explain give, :explain seq, …"),
        };
    }

    let bold = AnsiColor::Cyan.on_default().bold();
    let dim = anstyle::Style::new().dimmed();
    lessons
        .iter()
        .map(|lesson| {
            let name = if color {
                format!("{bold}{}{bold:#}", lesson.name)
            } else {
                lesson.name.to_string()
            };
            let example = lesson
                .example
                .lines()
                .map(|l| format!("    {l}"))
                .collect::<Vec<_>>()
                .join("\n");
            let question = if color {
                format!("{dim}{}{dim:#}", lesson.question_or_tip)
            } else {
                lesson.question_or_tip.to_string()
            };
            format!("{name} — {}\n\n{example}\n\n{question}", lesson.what)
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

/// Render the `:vars` table: aligned `name : Type = value` rows. Column
/// widths count characters, not bytes, so multibyte names stay aligned.
pub fn render_vars(rows: &[(String, String, String)]) -> String {
    if rows.is_empty() {
        return "no bindings yet".to_string();
    }
    let chars = |s: &str| s.chars().count();
    let name_width = rows.iter().map(|(n, _, _)| chars(n)).max().unwrap_or(0);
    let type_width = rows.iter().map(|(_, t, _)| chars(t)).max().unwrap_or(0);
    rows.iter()
        .map(|(n, t, v)| {
            let name_pad = " ".repeat(name_width - chars(n));
            let type_pad = " ".repeat(type_width - chars(t));
            format!("{n}{name_pad} : {t}{type_pad} = {v}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
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
        assert!(out.contains("    Give x to processor."), "indented example: {out}");
        assert!(out.contains('?'), "the socratic question must render: {out}");
    }

    #[test]
    fn explain_shows_every_matching_lesson() {
        // `Set` names both the statement and the type — an honest teacher
        // shows both.
        let out = render_explain("set", false);
        assert!(out.contains("mutable variable"), "the statement lesson: {out}");
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
        assert_eq!(strip_ansi(&colored), plain, "ANSI styling must only style, never reword");
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
        assert_eq!(cols[0], cols[1], "the `:` column must align in chars:\n{table}");
    }
}
