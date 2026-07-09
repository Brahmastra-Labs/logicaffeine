//! Error types and display formatting for parse errors.
//!
//! This module provides structured error types for the lexer and parser,
//! with rich diagnostic output including:
//!
//! - Source location with line/column numbers
//! - Syntax-highlighted error messages
//! - Socratic explanations for common mistakes
//! - Spelling suggestions for unknown words
//!
//! # Error Display
//!
//! Errors can be displayed with source context using [`ParseError::display_with_source`],
//! which produces rustc-style error output with underlined spans.

use logicaffeine_base::Interner;
use crate::style::Style;
use crate::suggest::{find_similar, KNOWN_WORDS};
use crate::token::{Span, TokenType};

/// A parse error with location information.
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

        // Window very long lines around the error column — a generated
        // 20,000-character expression must not flood the terminal with its
        // own excerpt.
        const EXCERPT_MAX: usize = 160;
        let (line_content, col, len) = if line_content.chars().count() > EXCERPT_MAX {
            let chars: Vec<char> = line_content.chars().collect();
            let col_chars = line_content
                .char_indices()
                .take_while(|(i, _)| *i < col)
                .count();
            let from = col_chars.saturating_sub(EXCERPT_MAX / 2).min(chars.len());
            let to = (from + EXCERPT_MAX).min(chars.len());
            let mut windowed: String = chars[from..to].iter().collect();
            if from > 0 {
                windowed = format!("…{windowed}");
            }
            if to < chars.len() {
                windowed.push('…');
            }
            let new_col = col_chars - from + usize::from(from > 0);
            (
                std::borrow::Cow::Owned(windowed),
                new_col,
                len.min(EXCERPT_MAX / 2),
            )
        } else {
            (std::borrow::Cow::Borrowed(line_content), col, len)
        };
        let line_content: &str = &line_content;
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
    /// Subject and object lists have different lengths in a "respectively" construction.
    RespectivelyLengthMismatch {
        subject_count: usize,
        object_count: usize,
    },
    /// Type mismatch during static type checking.
    TypeMismatch {
        expected: String,
        found: String,
    },
    /// Type mismatch with context (e.g., "in argument 2 of 'compute'").
    TypeMismatchDetailed {
        expected: String,
        found: String,
        context: String,
    },
    /// A type variable would occur in its own definition (e.g., `T = List<T>`).
    InfiniteType {
        var_description: String,
        type_description: String,
    },
    /// Wrong number of arguments in a function call.
    ArityMismatch {
        function: String,
        expected: usize,
        found: usize,
    },
    /// A field name does not exist on the struct type.
    FieldNotFound {
        type_name: String,
        field_name: String,
        available: Vec<String>,
    },
    /// Tried to call something that is not a function.
    NotAFunction {
        found_type: String,
    },
    /// Invalid refinement predicate in a dependent type.
    InvalidRefinementPredicate,
    /// Grammar error (e.g., "its" vs "it's").
    GrammarError(String),
    /// DRS scope violation (pronoun trapped in negation, disjunction, etc.).
    ScopeViolation(String),
    /// Unresolved pronoun in discourse mode - no accessible antecedent found.
    UnresolvedPronoun {
        gender: crate::drs::Gender,
        number: crate::drs::Number,
    },
    /// The parser finished a sentence with input left over — accepting it
    /// would silently drop the remainder's meaning.
    TrailingTokens {
        found: TokenType,
    },
    /// The program's AST nests deeper than every downstream walker
    /// (optimizer, codegen, interpreter, VM compiler) can safely recurse —
    /// the depth gate rejects it here so no surface ever stack-overflows.
    AstTooDeep {
        /// The measured (or exceeded) nesting depth.
        depth: usize,
        /// The enforced limit ([`crate::ast_depth::max_ast_depth`]).
        max_depth: usize,
    },
    /// Custom error message (used for escape analysis, zone errors, etc.).
    Custom(String),
}

/// A human reading of a token for error prose — never the raw `Debug` name.
/// "a comma (',')" teaches; "Comma" is compiler debris.
pub fn describe_token(token: &TokenType) -> String {
    let simple = |s: &str| s.to_string();
    match token {
        TokenType::Period => simple("a period ('.')"),
        TokenType::Comma => simple("a comma (',')"),
        TokenType::Colon => simple("a colon (':')"),
        TokenType::Newline => simple("a line break"),
        TokenType::Indent => simple("an indent"),
        TokenType::Dedent => simple("the end of an indented block"),
        TokenType::EOF => simple("the end of the input"),
        TokenType::Noun(_) => simple("a noun"),
        TokenType::Verb { .. } => simple("a verb"),
        TokenType::Adjective(_) | TokenType::NonIntersectiveAdjective(_) => simple("an adjective"),
        TokenType::Adverb(_) | TokenType::ScopalAdverb(_) | TokenType::TemporalAdverb(_) => {
            simple("an adverb")
        }
        TokenType::ProperName(_) => simple("a name"),
        TokenType::Identifier => simple("an identifier"),
        TokenType::Number(_) => simple("a number"),
        TokenType::StringLiteral(_) | TokenType::InterpolatedString(_) => simple("a string"),
        TokenType::CharLiteral(_) => simple("a character literal"),
        TokenType::MoneyLiteral { .. } => simple("a money amount"),
        TokenType::DurationLiteral { .. }
        | TokenType::DateLiteral { .. }
        | TokenType::TimeLiteral { .. } => simple("a time literal"),
        TokenType::CalendarUnit(_) => simple("a calendar unit"),
        TokenType::Pronoun { .. } => simple("a pronoun"),
        TokenType::Article(_) => simple("an article"),
        TokenType::Preposition(_) => simple("a preposition"),
        TokenType::Particle(_) => simple("a particle"),
        TokenType::Comparative(_) => simple("a comparative"),
        TokenType::Superlative(_) => simple("a superlative"),
        TokenType::Auxiliary(_) => simple("an auxiliary verb"),
        TokenType::Performative(_) => simple("a performative verb"),
        TokenType::BlockHeader { .. } => simple("a '##' block header"),
        TokenType::EscapeBlock(_) => simple("an escape block"),
        TokenType::Ambiguous { primary, .. } => describe_token(primary),
        TokenType::Possessive => simple("a possessive ('s)"),
        TokenType::LParen => simple("'('"),
        TokenType::RParen => simple("')'"),
        TokenType::LBracket => simple("'['"),
        TokenType::RBracket => simple("']'"),
        TokenType::LBrace => simple("'{'"),
        TokenType::RBrace => simple("'}'"),
        TokenType::Plus => simple("'+'"),
        TokenType::Minus => simple("'-'"),
        TokenType::Star => simple("'*'"),
        TokenType::Slash => simple("'/'"),
        TokenType::Percent => simple("'%'"),
        TokenType::PlusEq => simple("'+='"),
        TokenType::MinusEq => simple("'-='"),
        TokenType::StarEq => simple("'*='"),
        TokenType::SlashEq => simple("'/='"),
        TokenType::PercentEq => simple("'%='"),
        TokenType::StarStar => simple("'**'"),
        TokenType::SlashSlash => simple("'//'"),
        TokenType::Lt => simple("'<'"),
        TokenType::Gt => simple("'>'"),
        TokenType::LtEq => simple("'<='"),
        TokenType::GtEq => simple("'>='"),
        TokenType::EqEq => simple("'=='"),
        TokenType::NotEq => simple("'!='"),
        TokenType::Arrow => simple("'->'"),
        TokenType::Assign => simple("'='"),
        TokenType::Amp => simple("'&'"),
        TokenType::VBar => simple("'|'"),
        TokenType::Tilde => simple("'~'"),
        TokenType::Caret => simple("'^'"),
        TokenType::Dot => simple("'.'"),
        other => format!("the word '{}'", format!("{other:?}").to_lowercase()),
    }
}

/// The surface pronoun a gender/number pair names, quoted for prose.
fn pronoun_word(gender: &crate::drs::Gender, number: &crate::drs::Number) -> &'static str {
    use crate::drs::{Gender, Number};
    match (number, gender) {
        (Number::Plural, _) | (Number::Singular, Gender::Unknown) => "'they'",
        (Number::Singular, Gender::Female) => "'she'",
        (Number::Singular, Gender::Male) => "'he'",
        (Number::Singular, Gender::Neuter) => "'it'",
    }
}

#[cold]
pub fn socratic_explanation(error: &ParseError, _interner: &Interner) -> String {
    match &error.kind {
        ParseErrorKind::UnexpectedToken { expected, found } => {
            let expected = describe_token(expected);
            format!(
                "I was following your sentence but stumbled here: I expected {expected}, \
                and found {} instead. The structure so far commits me to {expected} next. \
                Is a word missing just before this, or did two thoughts run together? \
                Splitting the sentence in two often shows which.",
                describe_token(found)
            )
        }
        ParseErrorKind::ExpectedContentWord { found } => {
            format!(
                "This spot needs a content word — a noun, a verb, or an adjective — but I \
                found {} instead. Content words carry the meaning; the little words only \
                arrange them. What thing, action, or quality did you mean to name here? \
                Put that word in and the sentence grounds itself.",
                describe_token(found)
            )
        }
        ParseErrorKind::ExpectedCopula => "The subject here is waiting for 'is' or 'are' to \
            link it to its predicate. English predication runs through a copula — without \
            one, the two halves never connect. What is the subject supposed to BE? Write it \
            as 'X is Y' ('are' for plurals)."
            .to_string(),
        ParseErrorKind::UnknownQuantifier { found } => {
            format!(
                "I needed a quantifier here — a word like 'all', 'some', or 'no' — but found \
                {}. Quantifiers say how many things the sentence talks about, and that choice \
                decides the whole logical shape. How many did you mean: every one, at least \
                one, or none? Lead with the word that says so.",
                describe_token(found)
            )
        }
        ParseErrorKind::UnknownModal { found } => {
            format!(
                "I needed a modal here — 'must', 'can', 'may', 'should' — but found {}. \
                Modals set the strength of the claim: necessity, possibility, permission, \
                obligation. Which strength did you mean? Pick the modal that carries it and \
                the logic follows.",
                describe_token(found)
            )
        }
        ParseErrorKind::ExpectedVerb { found } => {
            format!(
                "Every sentence needs a verb, and this one's verb should appear here — I \
                found {} instead. The verb names the action or state everything else hangs \
                on. What is the subject doing (or being)? Give the sentence that verb.",
                describe_token(found)
            )
        }
        ParseErrorKind::ExpectedTemporalAdverb => "I expected a time word here — 'yesterday', \
            'tomorrow', 'always'. Temporal adverbs anchor the sentence on the timeline, and \
            this construction promised one. When does this happen? Say it with a temporal \
            adverb, or drop the construction that required one."
            .to_string(),
        ParseErrorKind::ExpectedPresuppositionTrigger => "I expected a presupposition trigger \
            here — a word like 'stopped', 'realized', or 'regrets'. These verbs quietly \
            assume something was already true; that hidden assumption is what this \
            construction works with. What background fact is being taken for granted? The \
            trigger word is what carries it."
            .to_string(),
        ParseErrorKind::ExpectedFocusParticle => "I expected a focus particle here — 'only', \
            'even', 'just'. Focus particles single out one part of the sentence against its \
            alternatives. Which word should the emphasis land on? Put the particle directly \
            before it."
            .to_string(),
        ParseErrorKind::ExpectedScopalAdverb => "I expected a scopal adverb here — a word \
            like 'necessarily' or 'possibly' that comments on the whole claim rather than \
            the verb alone. Is the adverb meant to cover the entire sentence? If it only \
            describes the action, it belongs next to the verb instead."
            .to_string(),
        ParseErrorKind::ExpectedSuperlativeAdjective => "I expected a superlative here — \
            'tallest', 'fastest', the '-est' form. A superlative ranks one thing against all \
            the others, and that is the comparison this sentence set up. Are you comparing \
            against everything, or just one other thing? Use '-est' for everything, '-er \
            than' for one."
            .to_string(),
        ParseErrorKind::ExpectedComparativeAdjective => "I expected a comparative here — \
            'taller', 'faster', the '-er' form. A comparative weighs exactly two things \
            against each other. What quality are the two compared on? Name it in its '-er' \
            form and follow with 'than'."
            .to_string(),
        ParseErrorKind::ExpectedThan => "A comparative opened a comparison, and 'than' \
            introduces the other side — but it is missing here. Taller than what, exactly? \
            Add 'than' plus the thing being compared against."
            .to_string(),
        ParseErrorKind::ExpectedNumber => "This measure phrase needs a number — '2', '3.14' \
            — and none appeared. A measure without a quantity measures nothing. How much, \
            exactly? Put the number in front of the unit."
            .to_string(),
        ParseErrorKind::EmptyRestriction => "This relative clause is empty — 'the X that \
            ...' with nothing after 'that'. A restriction narrows the noun down, so an empty \
            one narrows nothing. Which ones did you mean? State the property that picks them \
            out, or drop the 'that' entirely."
            .to_string(),
        ParseErrorKind::GappingResolutionFailed => "This reads like a gapped construction — \
            '... and Mary, a pear' — where the verb is borrowed from the previous clause, \
            but I found no verb there to borrow. What is the second subject doing? Either \
            repeat the verb outright, or make sure the first clause states it plainly."
            .to_string(),
        ParseErrorKind::StativeProgressiveConflict => "A stative verb like 'know' or 'love' \
            is in the progressive here, and states do not run in progress — they simply hold \
            or they don't. Is this a state or an ongoing activity? States take the simple \
            form ('knows'); only activities take '-ing'."
            .to_string(),
        ParseErrorKind::UndefinedVariable { name } => {
            format!(
                "I found '{name}', but nothing has declared it. Every variable starts life \
                in a Let — reading a name before it exists gives me no value to read. Where \
                should '{name}' get its first value? Add 'Let {name} be ...' above this \
                line, or check the spelling against the name you declared."
            )
        }
        ParseErrorKind::UseAfterMove { name } => {
            format!(
                "Cannot use '{name}' after giving it away. 'Give {name} to ...' transferred \
                ownership — '{name}' belongs to the receiver now, and this later use has \
                nothing left to hold. Who really needs to own '{name}' here? To lend it and \
                keep it, 'Show {name}'; to hand over a duplicate, give 'a copy of {name}'."
            )
        }
        ParseErrorKind::IsValueEquality { variable, value } => {
            format!(
                "'{variable} is {value}' reads as a type or predicate claim — 'is' asks what \
                something IS, not which value it holds. Value comparison is spelled \
                'equals'. Are you asking what '{variable}' is, or whether it holds {value}? \
                For the value question, write '{variable} equals {value}'."
            )
        }
        ParseErrorKind::ZeroIndex => "This asks for item 0, but LOGOS indices start at 1 — \
            in English, 'the 1st item' is the first one; there is no item 0. Which element \
            did you want? The first is 'item 1 of xs' — the zero-based habit is the only \
            thing to unlearn."
            .to_string(),
        ParseErrorKind::ExpectedStatement => "I expected a statement here — a sentence that \
            does something, like 'Let', 'Set', 'Show', or 'Return'. What should happen at \
            this point in the program? Start the line with the verb that does it, and end \
            with a period."
            .to_string(),
        ParseErrorKind::ExpectedKeyword { keyword } => {
            format!(
                "This construction needs the word '{keyword}' here to complete its shape. \
                Statement forms have fixed connecting words — they are the joints of the \
                sentence. Which form did you start? Insert '{keyword}' where I stopped, or \
                compare your line against that form's example."
            )
        }
        ParseErrorKind::ExpectedExpression => "I expected an expression here — a value: a \
            number, a variable, a call, or a computation. Something just before this (a \
            'be', a 'to', an operator) promised a value that never arrived. What value \
            should flow into this spot? Write it, or remove the connector that promised it."
            .to_string(),
        ParseErrorKind::ExpectedIdentifier => "I expected a name here — an identifier for a \
            variable, function, or field. This position says which thing the statement acts \
            on, so a keyword or symbol cannot stand in. What is the thing called? Use its \
            declared name, or declare it first with Let."
            .to_string(),
        ParseErrorKind::RespectivelyLengthMismatch { subject_count, object_count } => {
            format!(
                "'Respectively' pairs the two lists one-to-one, but the subject side has \
                {subject_count} element(s) and the object side has {object_count}. With \
                uneven lists, something ends up with no partner. Which pairing did you \
                intend? Even the lists out, or split the sentence so each pairing is \
                explicit."
            )
        }
        ParseErrorKind::TypeMismatch { expected, found } => {
            format!(
                "This slot is typed '{expected}', but the value here is '{found}'. LOGOS \
                holds every value to its declared type — that agreement is what the later \
                guarantees stand on. Which side is right, the annotation or the value? \
                Change the one that is lying."
            )
        }
        ParseErrorKind::TypeMismatchDetailed { expected, found, context } => {
            let ctx_note =
                if context.is_empty() { String::new() } else { format!(" ({context})") };
            format!(
                "I expected '{expected}' here but found '{found}'{ctx_note}. The two sides \
                of this position must agree on one type. Which side has it right? Adjust \
                the annotation or the value so they tell the same story."
            )
        }
        ParseErrorKind::InfiniteType { var_description, type_description } => {
            format!(
                "This would make an infinite type: {var_description} would have to equal \
                {type_description}, which contains it — a type inside itself, forever. This \
                usually means a value is being folded into its own container. Did you mean \
                to push into the collection rather than rebuild it? A named struct or one \
                level of indirection breaks the cycle."
            )
        }
        ParseErrorKind::ArityMismatch { function, expected, found } => {
            format!(
                "'{function}' takes {expected} argument(s), but this call passes {found}. A \
                call must fill every parameter — each one is a promise the function body \
                relies on. Which parameter went missing, or which extra snuck in? Line the \
                call up against the function's '## To' header."
            )
        }
        ParseErrorKind::FieldNotFound { type_name, field_name, available } => {
            if available.is_empty() {
                format!(
                    "'{type_name}' has no field named '{field_name}'. A struct's fields are \
                    fixed at its definition — reads cannot invent new ones. Did you mean \
                    another field, or should the '## A {type_name} has:' definition grow a \
                    '{field_name}'?"
                )
            } else {
                format!(
                    "'{type_name}' has no field named '{field_name}'. A struct's fields are \
                    fixed at its definition — reads cannot invent new ones. Available \
                    fields: {}. Did you mean one of those, or should the '## A {type_name} \
                    has:' definition grow a '{field_name}'?",
                    available.join(", ")
                )
            }
        }
        ParseErrorKind::NotAFunction { found_type } => {
            format!(
                "This tries to call a value of type '{found_type}', but only functions and \
                closures can be called. Parentheses after a name mean 'invoke this'. Did \
                you want a function with a similar name, or is this variable shadowing the \
                function you meant?"
            )
        }
        ParseErrorKind::InvalidRefinementPredicate => "This refinement predicate is not one \
            I can check — refinements are comparisons over the declared name, like 'x > 0' \
            or 'n < 100'. What must always be true of this value? State it as a simple \
            comparison; anything richer belongs in an Assert."
            .to_string(),
        ParseErrorKind::GrammarError(msg) => {
            format!(
                "Grammar: {msg}. Small grammar slips change the logical reading, so I stop \
                rather than guess. Which meaning did you intend? Adjust the wording — the \
                right form usually reads aloud correctly."
            )
        }
        ParseErrorKind::ScopeViolation(msg) => {
            format!(
                "Scope problem: {msg}. A pronoun can only reach referents its clause can \
                see — things introduced under a negation or inside an 'or' are walled off \
                from later sentences. Who is the pronoun meant to point at? Name the \
                referent outright, or introduce it outside the wall."
            )
        }
        ParseErrorKind::UnresolvedPronoun { gender, number } => {
            let word = pronoun_word(gender, number);
            format!(
                "This pronoun ({word}) has nothing to refer to — no earlier sentence \
                introduced a matching referent this clause can reach. Pronouns only look \
                backward through accessible discourse. Who is {word} here? Introduce that \
                referent in an earlier sentence, or use the name directly."
            )
        }
        ParseErrorKind::TrailingTokens { found } => {
            format!(
                "I understood the sentence up to here, but the rest — beginning with {} — \
                does not fit the structure I built. Accepting it would silently drop that \
                meaning, and LOGOS never drops meaning. Is a connective missing, or are two \
                sentences sharing one period? End the first thought with '.' and start \
                fresh.",
                describe_token(found)
            )
        }
        ParseErrorKind::AstTooDeep { depth, max_depth } => {
            format!(
                "This program nests expressions or blocks {depth} levels deep, past the \
                current {max_depth}-level limit — a tower this tall usually comes from \
                generated code, and every downstream walker would overflow on it. Could \
                each layer land in its own 'Let'? Intermediate bindings reset the depth to \
                one; or raise the gate with LOGOS_MAX_AST_DEPTH={suggested} if your stacks \
                are deep.",
                suggested = (depth + depth / 4).next_power_of_two()
            )
        }
        ParseErrorKind::Custom(msg) => msg.clone(),
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

    // =========================================================================
    // Phase 4 — Type error reporting variants
    // =========================================================================

    #[test]
    fn type_mismatch_detailed_socratic_mentions_types() {
        let interner = logicaffeine_base::Interner::new();
        let error = ParseError {
            kind: ParseErrorKind::TypeMismatchDetailed {
                expected: "Int".to_string(),
                found: "Bool".to_string(),
                context: "in let binding".to_string(),
            },
            span: Span::new(0, 0),
        };
        let explanation = socratic_explanation(&error, &interner);
        assert!(explanation.contains("Int"), "Should mention expected type: {}", explanation);
        assert!(explanation.contains("Bool"), "Should mention found type: {}", explanation);
        assert!(explanation.contains("let binding"), "Should include context: {}", explanation);
    }

    #[test]
    fn type_mismatch_detailed_without_context_is_clean() {
        let interner = logicaffeine_base::Interner::new();
        let error = ParseError {
            kind: ParseErrorKind::TypeMismatchDetailed {
                expected: "Text".to_string(),
                found: "Int".to_string(),
                context: String::new(),
            },
            span: Span::new(0, 0),
        };
        let explanation = socratic_explanation(&error, &interner);
        assert!(explanation.contains("Text"), "Should mention expected type: {}", explanation);
        assert!(explanation.contains("Int"), "Should mention found type: {}", explanation);
        // No spurious "()" from empty context
        assert!(!explanation.contains("()"), "Empty context should not leave '()': {}", explanation);
    }

    #[test]
    fn infinite_type_socratic_mentions_both_descriptions() {
        let interner = logicaffeine_base::Interner::new();
        let error = ParseError {
            kind: ParseErrorKind::InfiniteType {
                var_description: "type variable α0".to_string(),
                type_description: "Seq of α0".to_string(),
            },
            span: Span::new(0, 0),
        };
        let explanation = socratic_explanation(&error, &interner);
        assert!(explanation.contains("α0"), "Should mention var: {}", explanation);
        assert!(explanation.contains("Seq of α0"), "Should mention type: {}", explanation);
    }

    #[test]
    fn arity_mismatch_socratic_mentions_function_and_counts() {
        let interner = logicaffeine_base::Interner::new();
        let error = ParseError {
            kind: ParseErrorKind::ArityMismatch {
                function: "double".to_string(),
                expected: 1,
                found: 3,
            },
            span: Span::new(0, 0),
        };
        let explanation = socratic_explanation(&error, &interner);
        assert!(explanation.contains("double"), "Should name the function: {}", explanation);
        assert!(explanation.contains("1"), "Should mention expected count: {}", explanation);
        assert!(explanation.contains("3"), "Should mention found count: {}", explanation);
    }

    #[test]
    fn field_not_found_socratic_mentions_type_and_field() {
        let interner = logicaffeine_base::Interner::new();
        let error = ParseError {
            kind: ParseErrorKind::FieldNotFound {
                type_name: "Point".to_string(),
                field_name: "z".to_string(),
                available: vec!["x".to_string(), "y".to_string()],
            },
            span: Span::new(0, 0),
        };
        let explanation = socratic_explanation(&error, &interner);
        assert!(explanation.contains("Point"), "Should name the type: {}", explanation);
        assert!(explanation.contains("z"), "Should name the missing field: {}", explanation);
        assert!(explanation.contains("x"), "Should list available fields: {}", explanation);
        assert!(explanation.contains("y"), "Should list available fields: {}", explanation);
    }

    #[test]
    fn not_a_function_socratic_mentions_found_type() {
        let interner = logicaffeine_base::Interner::new();
        let error = ParseError {
            kind: ParseErrorKind::NotAFunction {
                found_type: "Int".to_string(),
            },
            span: Span::new(0, 0),
        };
        let explanation = socratic_explanation(&error, &interner);
        assert!(explanation.contains("Int"), "Should mention the type found: {}", explanation);
        assert!(explanation.to_lowercase().contains("function"), "Should mention function: {}", explanation);
    }
}
