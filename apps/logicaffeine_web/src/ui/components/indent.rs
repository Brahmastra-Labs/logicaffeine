//! What Enter and Tab do in the Studio's editor. Each language's indentation is worked out on
//! the text alone — which lines open a block, which close one, how wide a step is — so the
//! browser only has to apply the edit.

use super::code_editor::Language;

/// How a language indents: its step, and the lines that open and close a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Indentation {
    pub step: &'static str,
    grammar: Grammar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grammar {
    /// LOGOS: a line ending in `:` opens a block.
    Logos,
    /// Logic mode: sentences and theorems keep the line's own indentation.
    Logic,
    /// Math: literate lines ending in `:`, `## Theorem:` headers, and the kernel's `:=`,
    /// `match … with` and `Proof.` open blocks; `end`/`end.` and `Qed.` close them.
    Vernacular,
    /// English hardware specs keep the line's own indentation.
    Spec,
    /// Verilog: `module … ;`, `begin` and `case (…)` open blocks; `end…` closes them.
    Verilog,
    /// Generated Rust: `{` opens a block, `}` closes it.
    Rust,
}

impl Indentation {
    /// The indentation `language` follows; Hardware by what `source` is, Verilog or a spec.
    pub fn of(language: Language, source: &str) -> Indentation {
        use crate::ui::pages::hardware_report::is_verilog;
        let (step, grammar) = match language {
            Language::Logos => ("    ", Grammar::Logos),
            Language::Logic => ("    ", Grammar::Logic),
            Language::Vernacular => ("    ", Grammar::Vernacular),
            Language::Rust => ("    ", Grammar::Rust),
            Language::Hardware if is_verilog(source) => ("  ", Grammar::Verilog),
            Language::Hardware => ("    ", Grammar::Spec),
        };
        Indentation { step, grammar }
    }

    /// Whether `line` opens a block whose lines sit one step deeper.
    pub fn opens_block(&self, line: &str) -> bool {
        let code = self.code_of(line).trim_end();
        match self.grammar {
            Grammar::Logos => code.ends_with(':'),
            Grammar::Logic | Grammar::Spec => false,
            Grammar::Vernacular => {
                let header = code.trim_start();
                code.ends_with(':')
                    || code.ends_with(":=")
                    || ends_with_word(code, "with")
                    || header == "Proof."
                    || (header.starts_with("## ") && is_theorem_header(&header[3..]))
            }
            Grammar::Verilog => {
                let head = code.trim_start();
                ends_with_word(code, "begin")
                    || (starts_with_word(head, "module") && code.ends_with(';'))
                    || ["case", "casez", "casex"].iter().any(|w| starts_with_word(head, w))
            }
            Grammar::Rust => code.ends_with('{'),
        }
    }

    /// Whether `line` closes the block it sits in, and belongs one step shallower than it.
    pub fn closes_block(&self, line: &str) -> bool {
        let code = self.code_of(line).trim();
        match self.grammar {
            Grammar::Logos | Grammar::Logic | Grammar::Spec => false,
            Grammar::Vernacular => code == "end" || code == "end." || code == "Qed.",
            Grammar::Verilog => {
                ["end", "endmodule", "endcase", "endfunction", "endtask", "endgenerate"]
                    .iter()
                    .any(|w| starts_with_word(code, w))
            }
            Grammar::Rust => code.starts_with('}'),
        }
    }

    /// `line` without its comment.
    fn code_of<'l>(&self, line: &'l str) -> &'l str {
        let marker = match self.grammar {
            Grammar::Logos | Grammar::Vernacular => "--",
            Grammar::Verilog | Grammar::Rust => "//",
            Grammar::Logic | Grammar::Spec => "#",
        };
        match line.find(marker) {
            Some(at) if !(self.grammar == Grammar::Logic && line.trim_start().starts_with("##")) => &line[..at],
            _ => line,
        }
    }

    /// What Enter does at a caret whose line holds `line`, `at` bytes of it before the caret.
    /// It breaks the line, and the new line starts at the line's own indentation — one step
    /// deeper when the text before the caret opens a block. A line that closes a block but
    /// still sits at its block's depth is first moved back one step.
    pub fn enter(&self, line: &str, at: usize) -> Enter {
        let before = &line[..at];
        let own = leading_whitespace(line);
        let reindent = (self.closes_block(line) && !own.is_empty()).then(|| self.outdent_line(own));
        let base = reindent.clone().unwrap_or_else(|| own.to_string());
        let deeper = self.opens_block(before);
        let next = if deeper { format!("{base}{}", self.step) } else { base };
        // A caret inside the leading space carries no more indentation than it has.
        let next = if before.trim().is_empty() { before.to_string() } else { next };
        Enter { reindent, insert: format!("\n{next}") }
    }

    /// The spaces Tab inserts at `column` (counted in characters from 0): to the next stop.
    pub fn tab_fill(&self, column: usize) -> String {
        let width = self.step.len();
        " ".repeat(width - column % width)
    }

    /// `lines` (whole lines, joined by `\n`) moved one step deeper; blank lines stay empty.
    pub fn indent_lines(&self, lines: &str) -> String {
        lines
            .split('\n')
            .map(|l| if l.trim().is_empty() { l.to_string() } else { format!("{}{l}", self.step) })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// `lines` moved one step shallower, each as far as its own leading spaces allow (a tab
    /// counts as a whole step).
    pub fn outdent_lines(&self, lines: &str) -> String {
        lines.split('\n').map(|l| self.outdent_line(l)).collect::<Vec<_>>().join("\n")
    }

    fn outdent_line(&self, line: &str) -> String {
        if let Some(rest) = line.strip_prefix('\t') {
            return rest.to_string();
        }
        let spaces = line.len() - line.trim_start_matches(' ').len();
        line[spaces.min(self.step.len())..].to_string()
    }
}

/// What Enter does: the indentation to give the caret's own line first, if it changes, and the
/// text to insert at the caret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enter {
    pub reindent: Option<String>,
    pub insert: String,
}

fn leading_whitespace(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

fn is_theorem_header(rest: &str) -> bool {
    ["Theorem", "Lemma", "Corollary", "Proposition"]
        .iter()
        .any(|w| rest.strip_prefix(w).is_some_and(|after| after.starts_with(':')))
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn starts_with_word(text: &str, word: &str) -> bool {
    text.strip_prefix(word).is_some_and(|rest| !rest.starts_with(is_word))
}

fn ends_with_word(text: &str, word: &str) -> bool {
    text.strip_suffix(word).is_some_and(|rest| !rest.ends_with(is_word))
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/indent/tests.rs"]
mod tests;
