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
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_cli/repl/render/tests.rs"]
mod tests;
