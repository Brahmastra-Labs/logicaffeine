//! The one classification truth for LOGOS tokens: parts of speech ARE the
//! syntax — verbs paint as functions, nouns as types, adjectives as
//! modifiers. Every highlighting surface (LSP semantic tokens, the terminal
//! REPL, any future renderer) derives its colors from THIS mapping, so they
//! can never disagree.
//!
//! The match is deliberately wildcard-free: adding a `TokenType` variant
//! does not compile until someone decides its class — the decision lives
//! here, next to the enum, not in an editor plugin.

use crate::token::TokenType;

/// What a token IS, presentation-wise. Mirrors the LSP semantic-token
/// vocabulary (the LSP legend maps 1:1; the REPL maps to ANSI colors).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenClass {
    Keyword,
    Type,
    Function,
    Variable,
    String,
    Number,
    Operator,
    Namespace,
    Modifier,
    Property,
    Comment,
    Parameter,
    EnumMember,
}

/// Classify a token by its part of speech. `None` = structural tokens
/// (indent/dedent/newline/EOF) that no surface paints.
pub fn classify(kind: &TokenType) -> Option<TokenClass> {
    classify_inner(kind, 0)
}

const MAX_AMBIGUOUS_DEPTH: usize = 3;

fn classify_inner(kind: &TokenType, depth: usize) -> Option<TokenClass> {
    match kind {
        // Quantifiers → keyword
        TokenType::All | TokenType::No | TokenType::Some | TokenType::Any
        | TokenType::Both | TokenType::Most | TokenType::Few | TokenType::Many
        | TokenType::Cardinal(_) | TokenType::AtLeast(_) | TokenType::AtMost(_) => {
            Some(TokenClass::Keyword) // keyword
        }

        // Logical connectives → operator
        TokenType::And | TokenType::Or | TokenType::Not
        | TokenType::If | TokenType::Then | TokenType::Iff | TokenType::Because
        | TokenType::Although
        | TokenType::Until | TokenType::Release | TokenType::WeakUntil
        | TokenType::Implies => {
            Some(TokenClass::Operator) // operator
        }

        // Modal operators → keyword
        TokenType::Must | TokenType::Shall | TokenType::Should | TokenType::Can
        | TokenType::May | TokenType::Cannot | TokenType::Would | TokenType::Could
        | TokenType::Might | TokenType::Had => {
            Some(TokenClass::Keyword) // keyword
        }

        // Imperative keywords → keyword
        TokenType::Let | TokenType::Set | TokenType::Return | TokenType::Break | TokenType::Be
        | TokenType::While | TokenType::Repeat | TokenType::For | TokenType::In
        | TokenType::From | TokenType::Assert | TokenType::Trust
        | TokenType::Require | TokenType::Requires | TokenType::Ensures
        | TokenType::Otherwise | TokenType::Else | TokenType::Elif
        | TokenType::Call | TokenType::New | TokenType::Either | TokenType::Inspect
        | TokenType::Native | TokenType::Escape | TokenType::Given | TokenType::Prove
        | TokenType::Auto | TokenType::Read | TokenType::Write | TokenType::Console
        | TokenType::File | TokenType::Give | TokenType::Show | TokenType::Push
        | TokenType::Pop | TokenType::Copy | TokenType::Through | TokenType::Length
        | TokenType::At | TokenType::Add | TokenType::Remove | TokenType::Contains
        | TokenType::Union | TokenType::Intersection | TokenType::Inside
        | TokenType::Zone | TokenType::Called | TokenType::Size | TokenType::Mapped
        | TokenType::Attempt | TokenType::Following | TokenType::Simultaneously
        | TokenType::Spawn | TokenType::Send | TokenType::Await | TokenType::Portable
        | TokenType::Manifest | TokenType::Chunk | TokenType::Shared | TokenType::Merge
        | TokenType::Increase | TokenType::Decrease | TokenType::Tally
        | TokenType::SharedSet | TokenType::SharedSequence | TokenType::CollaborativeSequence
        | TokenType::SharedMap | TokenType::Divergent | TokenType::Append
        | TokenType::Resolve | TokenType::RemoveWins | TokenType::AddWins
        | TokenType::YATA | TokenType::Values | TokenType::Check | TokenType::Listen
        | TokenType::NetConnect | TokenType::Sleep | TokenType::Sync | TokenType::Mount
        | TokenType::Persistent | TokenType::Combined | TokenType::Followed | TokenType::Launch | TokenType::Task
        | TokenType::Pipe | TokenType::Receive | TokenType::Stop | TokenType::Try
        | TokenType::Into | TokenType::First | TokenType::After | TokenType::Mut => {
            Some(TokenClass::Keyword) // keyword
        }

        // Nouns → type
        TokenType::Noun(_) => Some(TokenClass::Type),

        // Verbs → function
        TokenType::Verb { .. } => Some(TokenClass::Function),

        // Adjectives → modifier
        TokenType::Adjective(_) | TokenType::NonIntersectiveAdjective(_) => {
            Some(TokenClass::Modifier) // modifier
        }

        // Proper names → variable with declaration modifier
        TokenType::ProperName(_) => Some(TokenClass::Variable), // variable + declaration

        // Pronouns → variable
        TokenType::Pronoun { .. } => Some(TokenClass::Variable),

        // Articles → keyword
        TokenType::Article(_) => Some(TokenClass::Keyword),

        // Copula → keyword
        TokenType::Is | TokenType::Are | TokenType::Was | TokenType::Were => {
            Some(TokenClass::Keyword)
        }

        // Wh-words → keyword
        TokenType::That | TokenType::Who | TokenType::Whose | TokenType::What | TokenType::Where
        | TokenType::When | TokenType::Why | TokenType::Does | TokenType::Do => {
            Some(TokenClass::Keyword)
        }

        // Identity/reflexive → keyword
        TokenType::Identity | TokenType::Equals | TokenType::Reflexive
        | TokenType::Reciprocal | TokenType::Respectively => {
            Some(TokenClass::Keyword)
        }

        // String literals → string
        TokenType::StringLiteral(_) | TokenType::InterpolatedString(_) | TokenType::CharLiteral(_) => Some(TokenClass::String),

        // Number literals → number
        TokenType::Number(_) | TokenType::MoneyLiteral { .. } | TokenType::DurationLiteral { .. }
        | TokenType::DateLiteral { .. } | TokenType::TimeLiteral { .. } => {
            Some(TokenClass::Number)
        }

        // Operators → operator
        TokenType::Plus | TokenType::Minus | TokenType::Star | TokenType::Slash
        | TokenType::Percent | TokenType::Lt | TokenType::Gt | TokenType::LtEq
        | TokenType::GtEq | TokenType::EqEq | TokenType::NotEq | TokenType::Arrow
        | TokenType::Assign | TokenType::Xor | TokenType::Shifted
        | TokenType::Amp | TokenType::VBar | TokenType::Tilde | TokenType::Caret
        | TokenType::Dot | TokenType::StarStar | TokenType::SlashSlash
        | TokenType::PlusEq | TokenType::MinusEq | TokenType::StarEq
        | TokenType::SlashEq | TokenType::PercentEq => {
            Some(TokenClass::Operator)
        }

        // Block headers → namespace
        TokenType::BlockHeader { .. } => Some(TokenClass::Namespace),

        // Prepositions → keyword
        TokenType::Preposition(_) => Some(TokenClass::Keyword),

        // Generic identifier → variable
        TokenType::Identifier => Some(TokenClass::Variable),

        // Comparatives/superlatives → modifier
        TokenType::Comparative(_) | TokenType::Superlative(_) | TokenType::Than => {
            Some(TokenClass::Modifier)
        }

        // Calendar/temporal → keyword
        TokenType::CalendarUnit(_) | TokenType::Ago | TokenType::Hence
        | TokenType::Before | TokenType::TemporalAdverb(_) => {
            Some(TokenClass::Keyword)
        }

        // Item/Items → keyword
        TokenType::Item | TokenType::Items => Some(TokenClass::Keyword),

        // Adverbs → modifier
        TokenType::Adverb(_) | TokenType::ScopalAdverb(_) => Some(TokenClass::Modifier),

        // Focus/presup → keyword
        TokenType::Focus(_) | TokenType::PresupTrigger(_) | TokenType::Measure(_) => {
            Some(TokenClass::Keyword)
        }

        // Auxiliary → keyword
        TokenType::Auxiliary(_) => Some(TokenClass::Keyword),

        // Escape blocks → string (raw code)
        TokenType::EscapeBlock(_) => Some(TokenClass::String),

        // Performative → keyword
        TokenType::Performative(_) => Some(TokenClass::Keyword),
        TokenType::Exclamation => Some(TokenClass::Keyword),

        // NPIs → keyword
        TokenType::Anything | TokenType::Anyone | TokenType::Nothing
        | TokenType::Nobody | TokenType::NoOne | TokenType::Nowhere
        | TokenType::Ever | TokenType::Never => {
            Some(TokenClass::Keyword)
        }

        // Ambiguous → try primary classification with depth guard
        TokenType::Ambiguous { primary, .. } => {
            if depth >= MAX_AMBIGUOUS_DEPTH {
                return None;
            }
            classify_inner(primary, depth + 1)
        }

        // Particles → keyword
        TokenType::Particle(_) => Some(TokenClass::Keyword),

        // Control → keyword
        TokenType::To => Some(TokenClass::Keyword),

        // Possessive → operator
        TokenType::Possessive => Some(TokenClass::Operator),

        // Punctuation → skip (or operator for comma/period)
        TokenType::Period | TokenType::Comma | TokenType::Colon => Some(TokenClass::Operator),
        TokenType::LParen | TokenType::RParen | TokenType::LBracket | TokenType::RBracket
        | TokenType::LBrace | TokenType::RBrace => {
            Some(TokenClass::Operator)
        }

        // Structural tokens → skip
        TokenType::Indent | TokenType::Dedent | TokenType::Newline | TokenType::EOF => {
            None
        }
    }
}
