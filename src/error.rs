use crate::intern::Interner;
use crate::style::Style;
use crate::suggest::{find_similar, KNOWN_WORDS};
use crate::token::{Span, TokenType};

#[derive(Debug, Clone)]
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub span: Span,
}

impl ParseError {
    pub fn display_with_source(&self, source: &str) -> String {
        let (line_num, line_start, line_content) = self.find_context(source);
        let col = self.span.start.saturating_sub(line_start);
        let len = (self.span.end - self.span.start).max(1);
        let underline = format!("{}{}", " ".repeat(col), "^".repeat(len));

        let error_label = Style::bold_red("error");
        let kind_str = format!("{:?}", self.kind);
        let line_num_str = Style::blue(&format!("{:4}", line_num));
        let pipe = Style::blue("|");
        let underline_colored = Style::red(&underline);

        let mut result = format!(
            "{}: {}\n\n{} {} {}\n     {} {}",
            error_label, kind_str, line_num_str, pipe, line_content, pipe, underline_colored
        );

        if let Some(word) = self.extract_word(source) {
            if let Some(suggestion) = find_similar(&word, KNOWN_WORDS, 2) {
                let hint = Style::cyan("help");
                result.push_str(&format!("\n     {} {}: did you mean '{}'?", pipe, hint, Style::green(suggestion)));
            }
        }

        result
    }

    fn extract_word<'a>(&self, source: &'a str) -> Option<&'a str> {
        if self.span.start < source.len() && self.span.end <= source.len() {
            let word = &source[self.span.start..self.span.end];
            if !word.is_empty() && word.chars().all(|c| c.is_alphabetic()) {
                return Some(word);
            }
        }
        None
    }

    fn find_context<'a>(&self, source: &'a str) -> (usize, usize, &'a str) {
        let mut line_num = 1;
        let mut line_start = 0;

        for (i, c) in source.char_indices() {
            if i >= self.span.start {
                break;
            }
            if c == '\n' {
                line_num += 1;
                line_start = i + 1;
            }
        }

        let line_end = source[line_start..]
            .find('\n')
            .map(|off| line_start + off)
            .unwrap_or(source.len());

        (line_num, line_start, &source[line_start..line_end])
    }
}

#[derive(Debug, Clone)]
pub enum ParseErrorKind {
    UnexpectedToken {
        expected: TokenType,
        found: TokenType,
    },
    ExpectedContentWord {
        found: TokenType,
    },
    ExpectedCopula,
    UnknownQuantifier {
        found: TokenType,
    },
    UnknownModal {
        found: TokenType,
    },
    ExpectedVerb {
        found: TokenType,
    },
    ExpectedTemporalAdverb,
    ExpectedPresuppositionTrigger,
    ExpectedFocusParticle,
    ExpectedScopalAdverb,
    ExpectedSuperlativeAdjective,
    ExpectedComparativeAdjective,
    ExpectedThan,
    ExpectedNumber,
    EmptyRestriction,
    GappingResolutionFailed,
    StativeProgressiveConflict,
    UndefinedVariable {
        name: String,
    },
    UseAfterMove {
        name: String,
    },
    IsValueEquality {
        variable: String,
        value: String,
    },
    ZeroIndex,
    ExpectedStatement,
    ExpectedKeyword { keyword: String },
    ExpectedExpression,
    ExpectedIdentifier,
    // Phase 35: Respectively operator
    RespectivelyLengthMismatch {
        subject_count: usize,
        object_count: usize,
    },
    // Phase 43: Type checking
    TypeMismatch {
        expected: String,
        found: String,
    },
    // Phase 43C: Refinement types
    InvalidRefinementPredicate,
}

#[cold]
pub fn socratic_explanation(error: &ParseError, _interner: &Interner) -> String {
    let pos = error.span.start;
    match &error.kind {
        ParseErrorKind::UnexpectedToken { expected, found } => {
            format!(
                "I was following your logic, but I stumbled at position {}. \
                I expected {:?}, but found {:?}. Perhaps you meant to use a different word here?",
                pos, expected, found
            )
        }
        ParseErrorKind::ExpectedContentWord { found } => {
            format!(
                "I was looking for a noun, verb, or adjective at position {}, \
                but found {:?} instead. The logic needs a content word to ground it.",
                pos, found
            )
        }
        ParseErrorKind::ExpectedCopula => {
            format!(
                "At position {}, I expected 'is' or 'are' to link the subject and predicate. \
                Without it, the sentence structure is incomplete.",
                pos
            )
        }
        ParseErrorKind::UnknownQuantifier { found } => {
            format!(
                "At position {}, I found {:?} where I expected a quantifier like 'all', 'some', or 'no'. \
                These words tell me how many things we're talking about.",
                pos, found
            )
        }
        ParseErrorKind::UnknownModal { found } => {
            format!(
                "At position {}, I found {:?} where I expected a modal like 'must', 'can', or 'should'. \
                Modals express possibility, necessity, or obligation.",
                pos, found
            )
        }
        ParseErrorKind::ExpectedVerb { found } => {
            format!(
                "At position {}, I expected a verb to describe an action or state, \
                but found {:?}. Every sentence needs a verb.",
                pos, found
            )
        }
        ParseErrorKind::ExpectedTemporalAdverb => {
            format!(
                "At position {}, I expected a temporal adverb like 'yesterday' or 'tomorrow' \
                to anchor the sentence in time.",
                pos
            )
        }
        ParseErrorKind::ExpectedPresuppositionTrigger => {
            format!(
                "At position {}, I expected a presupposition trigger like 'stopped', 'realized', or 'regrets'. \
                These words carry hidden assumptions.",
                pos
            )
        }
        ParseErrorKind::ExpectedFocusParticle => {
            format!(
                "At position {}, I expected a focus particle like 'only', 'even', or 'just'. \
                These words highlight what's important in the sentence.",
                pos
            )
        }
        ParseErrorKind::ExpectedScopalAdverb => {
            format!(
                "At position {}, I expected a scopal adverb that modifies the entire proposition.",
                pos
            )
        }
        ParseErrorKind::ExpectedSuperlativeAdjective => {
            format!(
                "At position {}, I expected a superlative adjective like 'tallest' or 'fastest'. \
                These words compare one thing to all others.",
                pos
            )
        }
        ParseErrorKind::ExpectedComparativeAdjective => {
            format!(
                "At position {}, I expected a comparative adjective like 'taller' or 'faster'. \
                These words compare two things.",
                pos
            )
        }
        ParseErrorKind::ExpectedThan => {
            format!(
                "At position {}, I expected 'than' after the comparative. \
                Comparisons need 'than' to introduce the thing being compared to.",
                pos
            )
        }
        ParseErrorKind::ExpectedNumber => {
            format!(
                "At position {}, I expected a numeric value like '2', '3.14', or 'aleph_0'. \
                Measure phrases require a number.",
                pos
            )
        }
        ParseErrorKind::EmptyRestriction => {
            format!(
                "At position {}, the restriction clause is empty. \
                A relative clause needs content to restrict the noun phrase.",
                pos
            )
        }
        ParseErrorKind::GappingResolutionFailed => {
            format!(
                "At position {}, I see a gapped construction (like '...and Mary, a pear'), \
                but I couldn't find a verb in the previous clause to borrow. \
                Gapping requires a clear action to repeat.",
                pos
            )
        }
        ParseErrorKind::StativeProgressiveConflict => {
            format!(
                "At position {}, a stative verb like 'know' or 'love' cannot be used with progressive aspect. \
                Stative verbs describe states, not activities in progress.",
                pos
            )
        }
        ParseErrorKind::UndefinedVariable { name } => {
            format!(
                "At position {}, I found '{}' but this variable has not been defined. \
                In imperative mode, all variables must be declared before use.",
                pos, name
            )
        }
        ParseErrorKind::UseAfterMove { name } => {
            format!(
                "At position {}, I found '{}' but this value has been moved. \
                Once a value is moved, it cannot be used again.",
                pos, name
            )
        }
        ParseErrorKind::IsValueEquality { variable, value } => {
            format!(
                "At position {}, I found '{} is {}' but 'is' is for type/predicate checks. \
                For value equality, use '{} equals {}'.",
                pos, variable, value, variable, value
            )
        }
        ParseErrorKind::ZeroIndex => {
            format!(
                "At position {}, I found 'item 0' but indices in LOGOS start at 1. \
                In English, 'the 1st item' is the first item, not the zeroth. \
                Try 'item 1 of list' to get the first element.",
                pos
            )
        }
        ParseErrorKind::ExpectedStatement => {
            format!(
                "At position {}, I expected a statement like 'Let', 'Set', or 'Return'.",
                pos
            )
        }
        ParseErrorKind::ExpectedKeyword { keyword } => {
            format!(
                "At position {}, I expected the keyword '{}'.",
                pos, keyword
            )
        }
        ParseErrorKind::ExpectedExpression => {
            format!(
                "At position {}, I expected an expression (number, variable, or computation).",
                pos
            )
        }
        ParseErrorKind::ExpectedIdentifier => {
            format!(
                "At position {}, I expected an identifier (variable name).",
                pos
            )
        }
        ParseErrorKind::RespectivelyLengthMismatch { subject_count, object_count } => {
            format!(
                "At position {}, 'respectively' requires equal-length lists. \
                The subject has {} element(s) and the object has {} element(s). \
                Each subject must pair with exactly one object.",
                pos, subject_count, object_count
            )
        }
        ParseErrorKind::TypeMismatch { expected, found } => {
            format!(
                "At position {}, I expected a value of type '{}' but found '{}'. \
                Types must match in LOGOS. Check that your value matches the declared type.",
                pos, expected, found
            )
        }
        ParseErrorKind::InvalidRefinementPredicate => {
            format!(
                "At position {}, the refinement predicate is not valid. \
                A refinement predicate must be a comparison like 'x > 0' or 'n < 100'.",
                pos
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::Span;

    #[test]
    fn parse_error_has_span() {
        let error = ParseError {
            kind: ParseErrorKind::ExpectedCopula,
            span: Span::new(5, 10),
        };
        assert_eq!(error.span.start, 5);
        assert_eq!(error.span.end, 10);
    }

    #[test]
    fn display_with_source_shows_line_and_underline() {
        let error = ParseError {
            kind: ParseErrorKind::ExpectedCopula,
            span: Span::new(8, 14),
        };
        let source = "All men mortal are.";
        let display = error.display_with_source(source);
        assert!(display.contains("mortal"), "Should contain source word: {}", display);
        assert!(display.contains("^^^^^^"), "Should contain underline: {}", display);
    }

    #[test]
    fn display_with_source_suggests_typo_fix() {
        let error = ParseError {
            kind: ParseErrorKind::ExpectedCopula,
            span: Span::new(0, 5),
        };
        let source = "logoc is the study of reason.";
        let display = error.display_with_source(source);
        assert!(display.contains("did you mean"), "Should suggest fix: {}", display);
        assert!(display.contains("logic"), "Should suggest 'logic': {}", display);
    }

    #[test]
    fn display_with_source_has_color_codes() {
        let error = ParseError {
            kind: ParseErrorKind::ExpectedCopula,
            span: Span::new(0, 3),
        };
        let source = "Alll men are mortal.";
        let display = error.display_with_source(source);
        assert!(display.contains("\x1b["), "Should contain ANSI escape codes: {}", display);
    }
}
