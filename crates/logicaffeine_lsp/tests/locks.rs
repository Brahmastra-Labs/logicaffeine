//! Ratchet locks for the language server.
//!
//! These tests pin the invariants that keep the LSP honest as the language
//! grows:
//!
//! - **Classification lock** — every `TokenType` variant classifies inside the
//!   advertised legend. The wildcard-free `token_type_guard` match breaks the
//!   build when a variant is added; `ALL_TOKEN_TYPE_COUNT` breaks this test
//!   when the sample list is not extended to match.
//! - **Decision lock** — every `ParseErrorKind` explicitly decides severity,
//!   diagnostic code, and quickfix (or records why none is safe), and every
//!   kind has a real socratic explanation.
//! - **Capability lock** — everything implemented is advertised and everything
//!   advertised is implemented; capabilities we deliberately do not offer are
//!   pinned with the reason.
//! - **Trigger lock** — trigger characters are single characters; LSP clients
//!   never send multi-character triggers.

mod harness;

use harness::error_kinds::{
    all_parse_error_kinds, parse_error_kind_guard, ALL_PARSE_ERROR_KIND_COUNT,
};
use harness::Harness;
use tower_lsp::lsp_types::*;

use logicaffeine_base::Interner;
use logicaffeine_language::error::{socratic_explanation, ParseError};
use logicaffeine_language::lexicon::{
    Aspect, Case, Definiteness, Gender, Number as GrammaticalNumber, Time, VerbClass,
};
use logicaffeine_language::token::{
    BlockType, CalendarUnit, FocusKind, MeasureKind, PresupKind, Span, TokenType,
};
use logicaffeine_lsp::diagnostics::{decision_for, DocsLink, Quickfix};
use logicaffeine_lsp::semantic_tokens::{classify_token, TOKEN_MODIFIERS, TOKEN_TYPES};

// ---------------------------------------------------------------------------
// Classification lock
// ---------------------------------------------------------------------------

/// One entry per `TokenType` variant. `token_type_guard` breaks the build when
/// the enum grows; update the guard, this list, and `ALL_TOKEN_TYPE_COUNT`
/// together.
const ALL_TOKEN_TYPE_COUNT: usize = 234;

fn all_token_types(interner: &mut Interner) -> Vec<TokenType> {
    let sym = interner.intern("sample");
    vec![
        TokenType::BlockHeader { block_type: BlockType::Main },
        TokenType::BlockHeader {
            block_type: BlockType::SuspectedTypo { found: sym, suggestion: sym },
        },
        TokenType::All,
        TokenType::No,
        TokenType::Some,
        TokenType::Any,
        TokenType::Both,
        TokenType::Most,
        TokenType::Few,
        TokenType::Many,
        TokenType::Cardinal(3),
        TokenType::AtLeast(2),
        TokenType::AtMost(4),
        TokenType::Anything,
        TokenType::Anyone,
        TokenType::Nothing,
        TokenType::Nobody,
        TokenType::NoOne,
        TokenType::Nowhere,
        TokenType::Ever,
        TokenType::Never,
        TokenType::And,
        TokenType::Or,
        TokenType::If,
        TokenType::Then,
        TokenType::Not,
        TokenType::Iff,
        TokenType::Because,
        TokenType::Although,
        TokenType::Until,
        TokenType::Release,
        TokenType::WeakUntil,
        TokenType::Implies,
        TokenType::Must,
        TokenType::Shall,
        TokenType::Should,
        TokenType::Can,
        TokenType::May,
        TokenType::Cannot,
        TokenType::Would,
        TokenType::Could,
        TokenType::Might,
        TokenType::Had,
        TokenType::Let,
        TokenType::Set,
        TokenType::Return,
        TokenType::Break,
        TokenType::Be,
        TokenType::While,
        TokenType::Repeat,
        TokenType::For,
        TokenType::In,
        TokenType::From,
        TokenType::Assert,
        TokenType::Trust,
        TokenType::Require,
        TokenType::Requires,
        TokenType::Ensures,
        TokenType::Otherwise,
        TokenType::Else,
        TokenType::Elif,
        TokenType::Call,
        TokenType::New,
        TokenType::Either,
        TokenType::Inspect,
        TokenType::Native,
        TokenType::Escape,
        TokenType::EscapeBlock(sym),
        TokenType::Given,
        TokenType::Prove,
        TokenType::Auto,
        TokenType::Read,
        TokenType::Write,
        TokenType::Console,
        TokenType::File,
        TokenType::Give,
        TokenType::Show,
        TokenType::Push,
        TokenType::Pop,
        TokenType::Copy,
        TokenType::Through,
        TokenType::Length,
        TokenType::At,
        TokenType::Add,
        TokenType::Remove,
        TokenType::Contains,
        TokenType::Union,
        TokenType::Intersection,
        TokenType::Inside,
        TokenType::Zone,
        TokenType::Called,
        TokenType::Size,
        TokenType::Mapped,
        TokenType::Attempt,
        TokenType::Following,
        TokenType::Simultaneously,
        TokenType::Spawn,
        TokenType::Send,
        TokenType::Await,
        TokenType::Portable,
        TokenType::Manifest,
        TokenType::Chunk,
        TokenType::Shared,
        TokenType::Merge,
        TokenType::Increase,
        TokenType::Decrease,
        TokenType::Tally,
        TokenType::SharedSet,
        TokenType::SharedSequence,
        TokenType::CollaborativeSequence,
        TokenType::SharedMap,
        TokenType::Divergent,
        TokenType::Append,
        TokenType::Resolve,
        TokenType::RemoveWins,
        TokenType::AddWins,
        TokenType::YATA,
        TokenType::Values,
        TokenType::Check,
        TokenType::Listen,
        TokenType::NetConnect,
        TokenType::Sleep,
        TokenType::Sync,
        TokenType::Mount,
        TokenType::Persistent,
        TokenType::Combined,
        TokenType::Followed,
        TokenType::Launch,
        TokenType::Task,
        TokenType::Pipe,
        TokenType::Receive,
        TokenType::Stop,
        TokenType::Try,
        TokenType::Into,
        TokenType::First,
        TokenType::After,
        TokenType::Colon,
        TokenType::Indent,
        TokenType::Dedent,
        TokenType::Newline,
        TokenType::Noun(sym),
        TokenType::Adjective(sym),
        TokenType::NonIntersectiveAdjective(sym),
        TokenType::Adverb(sym),
        TokenType::ScopalAdverb(sym),
        TokenType::TemporalAdverb(sym),
        TokenType::Verb {
            lemma: sym,
            time: Time::Present,
            aspect: Aspect::Simple,
            class: VerbClass::Activity,
        },
        TokenType::ProperName(sym),
        TokenType::Ambiguous {
            primary: Box::new(TokenType::Noun(sym)),
            alternatives: vec![TokenType::Identifier],
        },
        TokenType::Performative(sym),
        TokenType::Exclamation,
        TokenType::Article(Definiteness::Definite),
        TokenType::Auxiliary(Time::Past),
        TokenType::Is,
        TokenType::Are,
        TokenType::Was,
        TokenType::Were,
        TokenType::That,
        TokenType::Who,
        TokenType::What,
        TokenType::Where,
        TokenType::Whose,
        TokenType::When,
        TokenType::Why,
        TokenType::Does,
        TokenType::Do,
        TokenType::Identity,
        TokenType::Equals,
        TokenType::Reflexive,
        TokenType::Reciprocal,
        TokenType::Respectively,
        TokenType::Pronoun {
            gender: Gender::Female,
            number: GrammaticalNumber::Singular,
            case: Case::Subject,
        },
        TokenType::Preposition(sym),
        TokenType::Particle(sym),
        TokenType::Comparative(sym),
        TokenType::Superlative(sym),
        TokenType::Than,
        TokenType::To,
        TokenType::PresupTrigger(PresupKind::Stop),
        TokenType::Focus(FocusKind::Only),
        TokenType::Measure(MeasureKind::Much),
        TokenType::Number(sym),
        TokenType::MoneyLiteral { amount: sym, currency: sym },
        TokenType::DurationLiteral { nanos: 500, original_unit: sym },
        TokenType::DateLiteral { days: 20_000 },
        TokenType::TimeLiteral { nanos_from_midnight: 3_600_000_000_000 },
        TokenType::CalendarUnit(CalendarUnit::Day),
        TokenType::Ago,
        TokenType::Hence,
        TokenType::Before,
        TokenType::StringLiteral(sym),
        TokenType::InterpolatedString(sym),
        TokenType::CharLiteral(sym),
        TokenType::Item,
        TokenType::Items,
        TokenType::Possessive,
        TokenType::LParen,
        TokenType::RParen,
        TokenType::LBracket,
        TokenType::RBracket,
        TokenType::LBrace,
        TokenType::Amp,
        TokenType::VBar,
        TokenType::Tilde,
        TokenType::Caret,
        TokenType::RBrace,
        TokenType::Comma,
        TokenType::Period,
        TokenType::Dot,
        TokenType::Xor,
        TokenType::Shifted,
        TokenType::Plus,
        TokenType::Minus,
        TokenType::Star,
        TokenType::Slash,
        TokenType::Percent,
        TokenType::PlusEq,
        TokenType::MinusEq,
        TokenType::StarEq,
        TokenType::SlashEq,
        TokenType::PercentEq,
        TokenType::StarStar,
        TokenType::SlashSlash,
        TokenType::Lt,
        TokenType::Gt,
        TokenType::LtEq,
        TokenType::GtEq,
        TokenType::EqEq,
        TokenType::NotEq,
        TokenType::Arrow,
        TokenType::Assign,
        TokenType::Mut,
        TokenType::Identifier,
        TokenType::EOF,
    ]
}

/// Wildcard-free: a new `TokenType` variant fails to compile here, forcing an
/// update to `all_token_types` and `ALL_TOKEN_TYPE_COUNT` above.
fn token_type_guard(kind: &TokenType) {
    match kind {
        TokenType::BlockHeader { .. }
        | TokenType::All
        | TokenType::No
        | TokenType::Some
        | TokenType::Any
        | TokenType::Both
        | TokenType::Most
        | TokenType::Few
        | TokenType::Many
        | TokenType::Cardinal(_)
        | TokenType::AtLeast(_)
        | TokenType::AtMost(_)
        | TokenType::Anything
        | TokenType::Anyone
        | TokenType::Nothing
        | TokenType::Nobody
        | TokenType::NoOne
        | TokenType::Nowhere
        | TokenType::Ever
        | TokenType::Never
        | TokenType::And
        | TokenType::Or
        | TokenType::If
        | TokenType::Then
        | TokenType::Not
        | TokenType::Iff
        | TokenType::Because
        | TokenType::Although
        | TokenType::Until
        | TokenType::Release
        | TokenType::WeakUntil
        | TokenType::Implies
        | TokenType::Must
        | TokenType::Shall
        | TokenType::Should
        | TokenType::Can
        | TokenType::May
        | TokenType::Cannot
        | TokenType::Would
        | TokenType::Could
        | TokenType::Might
        | TokenType::Had
        | TokenType::Let
        | TokenType::Set
        | TokenType::Return
        | TokenType::Break
        | TokenType::Be
        | TokenType::While
        | TokenType::Repeat
        | TokenType::For
        | TokenType::In
        | TokenType::From
        | TokenType::Assert
        | TokenType::Trust
        | TokenType::Require
        | TokenType::Requires
        | TokenType::Ensures
        | TokenType::Otherwise
        | TokenType::Else
        | TokenType::Elif
        | TokenType::Call
        | TokenType::New
        | TokenType::Either
        | TokenType::Inspect
        | TokenType::Native
        | TokenType::Escape
        | TokenType::EscapeBlock(_)
        | TokenType::Given
        | TokenType::Prove
        | TokenType::Auto
        | TokenType::Read
        | TokenType::Write
        | TokenType::Console
        | TokenType::File
        | TokenType::Give
        | TokenType::Show
        | TokenType::Push
        | TokenType::Pop
        | TokenType::Copy
        | TokenType::Through
        | TokenType::Length
        | TokenType::At
        | TokenType::Add
        | TokenType::Remove
        | TokenType::Contains
        | TokenType::Union
        | TokenType::Intersection
        | TokenType::Inside
        | TokenType::Zone
        | TokenType::Called
        | TokenType::Size
        | TokenType::Mapped
        | TokenType::Attempt
        | TokenType::Following
        | TokenType::Simultaneously
        | TokenType::Spawn
        | TokenType::Send
        | TokenType::Await
        | TokenType::Portable
        | TokenType::Manifest
        | TokenType::Chunk
        | TokenType::Shared
        | TokenType::Merge
        | TokenType::Increase
        | TokenType::Decrease
        | TokenType::Tally
        | TokenType::SharedSet
        | TokenType::SharedSequence
        | TokenType::CollaborativeSequence
        | TokenType::SharedMap
        | TokenType::Divergent
        | TokenType::Append
        | TokenType::Resolve
        | TokenType::RemoveWins
        | TokenType::AddWins
        | TokenType::YATA
        | TokenType::Values
        | TokenType::Check
        | TokenType::Listen
        | TokenType::NetConnect
        | TokenType::Sleep
        | TokenType::Sync
        | TokenType::Mount
        | TokenType::Persistent
        | TokenType::Combined
        | TokenType::Followed
        | TokenType::Launch
        | TokenType::Task
        | TokenType::Pipe
        | TokenType::Receive
        | TokenType::Stop
        | TokenType::Try
        | TokenType::Into
        | TokenType::First
        | TokenType::After
        | TokenType::Colon
        | TokenType::Indent
        | TokenType::Dedent
        | TokenType::Newline
        | TokenType::Noun(_)
        | TokenType::Adjective(_)
        | TokenType::NonIntersectiveAdjective(_)
        | TokenType::Adverb(_)
        | TokenType::ScopalAdverb(_)
        | TokenType::TemporalAdverb(_)
        | TokenType::Verb { .. }
        | TokenType::ProperName(_)
        | TokenType::Ambiguous { .. }
        | TokenType::Performative(_)
        | TokenType::Exclamation
        | TokenType::Article(_)
        | TokenType::Auxiliary(_)
        | TokenType::Is
        | TokenType::Are
        | TokenType::Was
        | TokenType::Were
        | TokenType::That
        | TokenType::Who
        | TokenType::What
        | TokenType::Where
        | TokenType::Whose
        | TokenType::When
        | TokenType::Why
        | TokenType::Does
        | TokenType::Do
        | TokenType::Identity
        | TokenType::Equals
        | TokenType::Reflexive
        | TokenType::Reciprocal
        | TokenType::Respectively
        | TokenType::Pronoun { .. }
        | TokenType::Preposition(_)
        | TokenType::Particle(_)
        | TokenType::Comparative(_)
        | TokenType::Superlative(_)
        | TokenType::Than
        | TokenType::To
        | TokenType::PresupTrigger(_)
        | TokenType::Focus(_)
        | TokenType::Measure(_)
        | TokenType::Number(_)
        | TokenType::MoneyLiteral { .. }
        | TokenType::DurationLiteral { .. }
        | TokenType::DateLiteral { .. }
        | TokenType::TimeLiteral { .. }
        | TokenType::CalendarUnit(_)
        | TokenType::Ago
        | TokenType::Hence
        | TokenType::Before
        | TokenType::StringLiteral(_)
        | TokenType::InterpolatedString(_)
        | TokenType::CharLiteral(_)
        | TokenType::Item
        | TokenType::Items
        | TokenType::Possessive
        | TokenType::LParen
        | TokenType::RParen
        | TokenType::LBracket
        | TokenType::RBracket
        | TokenType::LBrace
        | TokenType::Amp
        | TokenType::VBar
        | TokenType::Tilde
        | TokenType::Caret
        | TokenType::RBrace
        | TokenType::Comma
        | TokenType::Period
        | TokenType::Dot
        | TokenType::Xor
        | TokenType::Shifted
        | TokenType::Plus
        | TokenType::Minus
        | TokenType::Star
        | TokenType::Slash
        | TokenType::Percent
        | TokenType::PlusEq
        | TokenType::MinusEq
        | TokenType::StarEq
        | TokenType::SlashEq
        | TokenType::PercentEq
        | TokenType::StarStar
        | TokenType::SlashSlash
        | TokenType::Lt
        | TokenType::Gt
        | TokenType::LtEq
        | TokenType::GtEq
        | TokenType::EqEq
        | TokenType::NotEq
        | TokenType::Arrow
        | TokenType::Assign
        | TokenType::Mut
        | TokenType::Identifier
        | TokenType::EOF => {}
    }
}

#[test]
fn every_token_type_classifies_within_the_legend() {
    let mut interner = Interner::new();
    let all = all_token_types(&mut interner);
    assert_eq!(
        all.len(),
        ALL_TOKEN_TYPE_COUNT,
        "all_token_types drifted from ALL_TOKEN_TYPE_COUNT — update both with the guard"
    );

    for kind in &all {
        token_type_guard(kind);
        let (token_type, modifiers) = classify_token(kind);
        if let Some(index) = token_type {
            assert!(
                (index as usize) < TOKEN_TYPES.len(),
                "{kind:?} classifies to index {index}, outside the advertised legend"
            );
        }
        assert!(
            (modifiers as u64) < (1u64 << TOKEN_MODIFIERS.len()),
            "{kind:?} sets modifier bits outside the advertised legend: {modifiers:#b}"
        );
    }
}

/// The legend is append-only: indices are a wire contract with every client
/// theme. Extending it is fine; reordering or removing entries is not.
#[test]
fn semantic_token_legend_is_append_only() {
    let pinned_types = [
        "keyword",
        "type",
        "function",
        "variable",
        "string",
        "number",
        "operator",
        "namespace",
        "modifier",
        "property",
        "comment",
        "parameter",
        "enumMember",
    ];
    for (index, expected) in pinned_types.iter().enumerate() {
        assert_eq!(
            TOKEN_TYPES[index].as_str(),
            *expected,
            "legend index {index} changed meaning — the legend is append-only"
        );
    }

    let pinned_modifiers = ["declaration", "readonly"];
    for (index, expected) in pinned_modifiers.iter().enumerate() {
        assert_eq!(
            TOKEN_MODIFIERS[index].as_str(),
            *expected,
            "modifier bit {index} changed meaning — the legend is append-only"
        );
    }
}

// ---------------------------------------------------------------------------
// Decision lock
// ---------------------------------------------------------------------------

// The kind list, guard, and count live in `harness::error_kinds` — shared
// with the socratic corpus and the quickfix parity ratchet.

#[test]
fn every_parse_error_kind_decides_and_explains() {
    let interner = Interner::new();
    let all = all_parse_error_kinds();
    assert_eq!(
        all.len(),
        ALL_PARSE_ERROR_KIND_COUNT,
        "all_parse_error_kinds drifted from ALL_PARSE_ERROR_KIND_COUNT — update both with the guard"
    );

    for kind in all {
        parse_error_kind_guard(&kind);
        let decision = decision_for(&kind);

        if let Some(code) = decision.code {
            assert!(
                !code.is_empty()
                    && code
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{kind:?}: diagnostic code {code:?} must be kebab-case"
            );
        }

        match decision.quickfix {
            Quickfix::Provided(title) => {
                assert!(
                    !title.is_empty(),
                    "{kind:?}: a provided quickfix must name its action"
                );
                assert!(
                    decision.code.is_some(),
                    "{kind:?}: quickfixes key on diagnostic codes, so a code is required"
                );
            }
            Quickfix::None(reason) => {
                assert!(
                    !reason.is_empty(),
                    "{kind:?}: declining a quickfix requires a recorded reason"
                );
            }
        }

        let guide_slugs =
            harness::quickguide::heading_slugs(include_str!("../../../LOGOS_QUICKGUIDE.md"));
        match decision.docs {
            DocsLink::Anchor(anchor) => {
                assert!(
                    !anchor.is_empty() && !anchor.starts_with('#'),
                    "{kind:?}: a docs anchor is a bare slug, got {anchor:?}"
                );
                assert!(
                    decision.code.is_some(),
                    "{kind:?}: clients render codeDescription alongside a code, so a code is required"
                );
                assert!(
                    guide_slugs.iter().any(|s| s == anchor),
                    "{kind:?}: docs anchor {anchor:?} matches no LOGOS_QUICKGUIDE.md heading"
                );
            }
            DocsLink::None(reason) => {
                assert!(
                    !reason.is_empty(),
                    "{kind:?}: declining a docs link requires a recorded reason"
                );
            }
        }

        let error = ParseError { kind: kind.clone(), span: Span::new(0, 1) };
        let explanation = socratic_explanation(&error, &interner);
        assert!(
            !explanation.is_empty(),
            "{kind:?}: every error kind needs a socratic explanation"
        );
        assert_ne!(
            explanation,
            format!("{:?}", kind),
            "{kind:?}: the explanation must be prose, not Debug output"
        );
    }
}

// ---------------------------------------------------------------------------
// Analysis-layer decision lock
// ---------------------------------------------------------------------------

/// One producing program per `ANALYSIS_DECISIONS` code — the severity
/// contract is only honest while every row is live.
const ANALYSIS_BATTERY: &[(&str, &str)] = &[
    ("unused-variable", "## Main\nLet unused be 5.\nShow 1.\n"),
    ("shadowed-variable", "## Main\nLet x be 5.\nLet x be 6.\nShow x.\n"),
    (
        "unused-function",
        "## To ghost (n: Int) -> Int:\n    Return n.\n\n## Main\nShow 1.\n",
    ),
];

#[test]
fn every_analysis_decision_is_live_with_its_recorded_severity() {
    use logicaffeine_lsp::diagnostics::ANALYSIS_DECISIONS;
    use logicaffeine_lsp::document::DocumentState;

    for (code, severity, reason) in ANALYSIS_DECISIONS {
        assert!(!reason.is_empty(), "{code}: severity decisions record their reason");
        let (_, snippet) = ANALYSIS_BATTERY
            .iter()
            .find(|(name, _)| name == code)
            .unwrap_or_else(|| panic!("{code}: no battery program keeps this decision live"));
        let doc = DocumentState::new(snippet.to_string(), 1);
        let produced = doc
            .diagnostics
            .iter()
            .find(|d| {
                matches!(&d.code, Some(NumberOrString::String(c)) if c == code)
            })
            .unwrap_or_else(|| {
                panic!("{code}: the battery program produced no such diagnostic: {:#?}",
                    doc.diagnostics)
            });
        assert_eq!(
            produced.severity,
            Some(*severity),
            "{code}: emitted severity drifted from the recorded decision"
        );
    }
    for (code, _) in ANALYSIS_BATTERY {
        assert!(
            logicaffeine_lsp::diagnostics::ANALYSIS_DECISIONS
                .iter()
                .any(|(name, _, _)| name == code),
            "{code}: battery snippet for a code with no recorded decision"
        );
    }
}

// ---------------------------------------------------------------------------
// Capability + trigger locks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn advertised_capabilities_match_implemented_handlers() {
    let harness = Harness::start().await;
    let caps = &harness.init.capabilities;

    // Implemented and advertised.
    assert!(caps.text_document_sync.is_some(), "text sync is implemented");
    assert!(caps.semantic_tokens_provider.is_some(), "semantic tokens are implemented");
    assert!(caps.document_symbol_provider.is_some(), "document symbols are implemented");
    assert!(caps.definition_provider.is_some(), "goto definition is implemented");
    assert!(caps.hover_provider.is_some(), "hover is implemented");
    assert!(caps.completion_provider.is_some(), "completion is implemented");
    assert!(caps.references_provider.is_some(), "references are implemented");
    assert!(caps.signature_help_provider.is_some(), "signature help is implemented");
    assert!(caps.code_action_provider.is_some(), "code actions are implemented");
    assert!(caps.rename_provider.is_some(), "rename is implemented");
    assert!(caps.folding_range_provider.is_some(), "folding is implemented");
    assert!(caps.inlay_hint_provider.is_some(), "inlay hints are implemented");
    assert!(caps.code_lens_provider.is_some(), "code lenses are implemented");
    assert!(caps.document_formatting_provider.is_some(), "formatting is implemented");

    assert!(caps.workspace_symbol_provider.is_some(), "workspace symbols are implemented");
    assert!(caps.document_highlight_provider.is_some(), "document highlights are implemented");
    assert!(caps.selection_range_provider.is_some(), "selection ranges are implemented");
    assert!(caps.call_hierarchy_provider.is_some(), "call hierarchy is implemented");
    assert!(
        caps.document_on_type_formatting_provider.is_some(),
        "on-type formatting is implemented"
    );
    assert!(
        caps.document_range_formatting_provider.is_some(),
        "range formatting is implemented (whole-doc structural format, line-filtered)"
    );
    assert!(
        caps.diagnostic_provider.is_some(),
        "pull diagnostics are implemented (push stays for older clients)"
    );

    // Not offered — each entry names the reason. Remove an assertion only in
    // the change that implements (or permanently rejects) the capability.
    assert!(
        caps.linked_editing_range_provider.is_none(),
        "LOGOS has no paired-token constructs; rename covers this need"
    );
    assert!(
        caps.execute_command_provider.is_none(),
        "code-lens commands execute client-side in the editor extension"
    );
}

#[tokio::test]
async fn all_trigger_characters_are_single_characters() {
    let harness = Harness::start().await;
    let caps = &harness.init.capabilities;

    let completion_triggers = caps
        .completion_provider
        .as_ref()
        .and_then(|p| p.trigger_characters.clone())
        .unwrap_or_default();
    for trigger in &completion_triggers {
        assert_eq!(
            trigger.chars().count(),
            1,
            "completion trigger {trigger:?} is not a single character; clients never send it"
        );
    }

    let signature_triggers = caps
        .signature_help_provider
        .as_ref()
        .and_then(|p| p.trigger_characters.clone())
        .unwrap_or_default();
    assert!(
        !signature_triggers.is_empty(),
        "signature help must keep at least one trigger character"
    );
    for trigger in &signature_triggers {
        assert_eq!(
            trigger.chars().count(),
            1,
            "signature-help trigger {trigger:?} is not a single character; clients never send it"
        );
    }
}

#[tokio::test]
async fn advertised_legend_matches_the_classifier_tables() {
    let harness = Harness::start().await;
    let legend = match harness.init.capabilities.semantic_tokens_provider.as_ref() {
        Some(SemanticTokensServerCapabilities::SemanticTokensOptions(opts)) => &opts.legend,
        other => panic!("expected plain semantic token options, got {other:?}"),
    };
    assert_eq!(legend.token_types.len(), TOKEN_TYPES.len());
    assert_eq!(legend.token_modifiers.len(), TOKEN_MODIFIERS.len());
}
