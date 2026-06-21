//! Shared library for the wiki trace + triage harnesses.
//!
//! `wiki-trace` dumps verbose per-sentence traces. `wiki-triage` classifies each
//! sentence into the kind of work it implies (lexicon gap / parser gap / semantic
//! bug / design decision / isolate-as-noise), localizes it, and — where an
//! equivalent paraphrase already parses — derives the expected FOL ("the spec
//! writes itself"). The triage harness is READ-ONLY: it proposes, it never edits
//! source, lexicon, or tests.

use logicaffeine_compile::ui_bridge::{compile_for_ui, AstNode, CompileResult, TokenCategory};
use logicaffeine_language::lexicon;
use logicaffeine_language::{ParseError, ParseErrorKind, TokenType};
use serde::Serialize;

pub mod render;

// ═══════════════════════════════════════════════════════════════════
// Triage model
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    Partial,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Clean,
    ActionableLexiconGap,
    ParserGap,
    SemanticLossy,
    AmbiguityHuman,
    IsolateOutOfScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Subsystem {
    Lexicon,
    Parser,
    Semantics,
    None,
}

/// What the loop is permitted to do with a finding.
/// `Auto` — low blast radius with a concrete verifiable proposal (a lexicon
/// entry); a gated loop may apply it. `Investigate` — there's a lead (an oracle
/// paraphrase, a known parser construction) but an agent must implement it.
/// `Human` — design decision / noise; never acted on autonomously.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    Auto,
    Investigate,
    Human,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolateKind {
    Abbreviation,
    Parenthetical,
    Quote,
    Citation,
}

#[derive(Debug, Clone, Serialize)]
pub struct IsolatedSpan {
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub kind: IsolateKind,
}

#[derive(Debug, Clone, Serialize)]
pub struct Localization {
    /// Stable machine label for the parse error kind (empty on success).
    pub error_kind: String,
    pub error_span: Option<(usize, usize)>,
    pub offending_text: Option<String>,
    /// Content words that look like lexicon gaps (mis-tagged, absent from DBs).
    pub suspect_words: Vec<String>,
    pub isolated_spans: Vec<IsolatedSpan>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Oracle {
    /// Which metamorphic transform produced the equivalent paraphrase.
    pub transform: String,
    pub variant_sentence: String,
    /// FOL the failing sentence is expected to produce (from the parsing twin).
    pub expected_fol: String,
    /// How equivalence was established. `adopted` = the original failed and the
    /// variant parsed, so the variant's FOL is taken as the spec.
    pub equivalence: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Evidence {
    pub reading_count: usize,
    pub ast_nodes: usize,
    pub unhandled_ast_nodes: usize,
    pub lexicon_misses: Vec<String>,
    pub dropped_content_words: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LexiconEntry {
    pub lemma: String,
    pub pos: String,
    pub note: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Proposal {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub red_test: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lexicon_entry: Option<LexiconEntry>,
}

impl Proposal {
    fn is_empty(&self) -> bool {
        self.red_test.is_none() && self.lexicon_entry.is_none()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TriageRecord {
    pub index: usize,
    pub input: String,
    pub outcome: Outcome,
    pub category: Category,
    pub subsystem: Subsystem,
    pub gate: Gate,
    pub confidence: f32,
    /// Short human-facing FOL (Unicode) on success, else the socratic error.
    pub fol: Option<String>,
    pub localization: Localization,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oracle: Option<Oracle>,
    pub evidence: Evidence,
    #[serde(skip_serializing_if = "Proposal::is_empty")]
    pub proposal: Proposal,
}

// ═══════════════════════════════════════════════════════════════════
// Quarantine pass — isolate abbreviations / parens / quotes / citations
// ═══════════════════════════════════════════════════════════════════

/// Detect spans that are noise-for-now: abbreviations/acronyms, parentheticals,
/// quoted strings, and bracketed citations. These are flagged and removed from
/// the auto-fix path — the loop must never invent a lexicon entry for "ERPs" or
/// "fix" a quoted title.
pub fn quarantine(input: &str) -> Vec<IsolatedSpan> {
    let bytes = input.as_bytes();
    let mut spans = Vec::new();

    // Bracketed / parenthesized regions.
    for (open, close, kind) in [
        (b'(', b')', IsolateKind::Parenthetical),
        (b'[', b']', IsolateKind::Citation),
    ] {
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == open {
                if let Some(j) = (i + 1..bytes.len()).find(|&k| bytes[k] == close) {
                    spans.push(IsolatedSpan {
                        start: i,
                        end: j + 1,
                        text: input[i..j + 1].to_string(),
                        kind,
                    });
                    i = j + 1;
                    continue;
                }
            }
            i += 1;
        }
    }

    // Quoted regions (straight and curly double quotes).
    let mut quote_open: Option<usize> = None;
    for (i, ch) in input.char_indices() {
        match ch {
            '"' | '\u{201C}' => {
                if quote_open.is_none() {
                    quote_open = Some(i);
                } else if let Some(start) = quote_open.take() {
                    let end = i + ch.len_utf8();
                    spans.push(IsolatedSpan {
                        start,
                        end,
                        text: input[start..end].to_string(),
                        kind: IsolateKind::Quote,
                    });
                }
            }
            '\u{201D}' => {
                if let Some(start) = quote_open.take() {
                    let end = i + ch.len_utf8();
                    spans.push(IsolatedSpan {
                        start,
                        end,
                        text: input[start..end].to_string(),
                        kind: IsolateKind::Quote,
                    });
                }
            }
            _ => {}
        }
    }

    // Abbreviations / acronyms, token by token.
    for (start, word) in word_spans(input) {
        if looks_like_abbreviation(word) {
            spans.push(IsolatedSpan {
                start,
                end: start + word.len(),
                text: word.to_string(),
                kind: IsolateKind::Abbreviation,
            });
        }
    }

    spans.sort_by_key(|s| s.start);
    spans
}

/// Iterate over alphanumeric word spans with their byte offsets.
fn word_spans(input: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_alphanumeric() {
            let start = i;
            while i < bytes.len() && {
                let d = bytes[i] as char;
                d.is_ascii_alphanumeric()
            } {
                i += 1;
            }
            out.push((start, &input[start..i]));
        } else {
            i += 1;
        }
    }
    out
}

/// An acronym/abbreviation: ALL-CAPS (≥2), an interior capital (BSc, PhD),
/// or a digit mixed with a capital (N2pc). A normal Capitalized word (Mary)
/// is NOT flagged.
fn looks_like_abbreviation(word: &str) -> bool {
    let chars: Vec<char> = word.chars().collect();
    if chars.len() < 2 {
        return false;
    }
    let uppers = chars.iter().filter(|c| c.is_ascii_uppercase()).count();
    let has_digit = chars.iter().any(|c| c.is_ascii_digit());
    let all_caps = chars.iter().all(|c| c.is_ascii_uppercase());
    let interior_cap = chars.iter().skip(1).any(|c| c.is_ascii_uppercase());
    (all_caps && chars.len() >= 2) || interior_cap || (has_digit && uppers >= 1)
}

fn in_isolated(spans: &[IsolatedSpan], start: usize, end: usize) -> bool {
    spans.iter().any(|s| start < s.end && end > s.start)
}

// ═══════════════════════════════════════════════════════════════════
// Lexicon probe
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WordLex {
    Verb,
    Noun,
    Adjective,
    Unknown,
}

/// Query the lexicon for a surface word, resolving morphology (plurals, verb
/// inflections, derivations). Returns the POS it is known as, or `Unknown`.
fn lexical_status(word: &str) -> WordLex {
    let lower = word.to_lowercase();
    if lexicon::get_canonical_verb(&lower).is_some() {
        return WordLex::Verb;
    }
    if lexicon::analyze_word(&lower).is_some() {
        return WordLex::Noun;
    }
    if lexicon::lookup_adjective_db(&lower).is_some() || lexicon::is_adjective(&lower) {
        return WordLex::Adjective;
    }
    if lexicon::is_common_noun(&lower) {
        return WordLex::Noun;
    }
    WordLex::Unknown
}

/// Closed-class function words. The lexer maps copulas/auxiliaries to no specific
/// UI category, so they land in `Other`; this stoplist keeps them off the gap
/// path so the loop never proposes a lexicon entry for "is".
fn is_function_word(lower: &str) -> bool {
    matches!(
        lower,
        // copulas / auxiliaries
        "is" | "are" | "was" | "were" | "be" | "been" | "being" | "am"
            | "has" | "have" | "had" | "do" | "does" | "did"
            | "will" | "would" | "shall" | "should" | "can" | "could"
            | "may" | "might" | "must"
        // determiners / pronominal / relative
            | "the" | "a" | "an" | "this" | "that" | "these" | "those"
            | "which" | "who" | "whom" | "whose" | "where" | "when"
            | "there" | "here" | "such" | "not" | "no"
        // prepositions / subordinators / conjunctions
            | "of" | "to" | "as" | "in" | "on" | "at" | "by" | "for"
            | "from" | "with" | "into" | "onto" | "upon" | "about"
            | "after" | "before" | "during" | "since" | "until" | "while"
            | "through" | "between" | "among" | "within" | "without"
            | "over" | "under" | "above" | "below" | "near" | "via" | "per"
            | "and" | "or" | "nor" | "but" | "yet" | "so" | "than"
            | "because" | "although" | "though" | "however" | "whether" | "if"
    )
}

/// High-precision signal of an actionable lexicon gap: a lowercase content word
/// the lexer kept as an open-class token (`Adjective`/`Noun`/`Other`), that is
/// not a closed-class function word, and is absent from EVERY lexicon database in
/// every POS (with morphology resolved). The lexer's unknown-word FALLBACK is now
/// `Noun` (it treats unknown lowercase content words as nouns — domain items like
/// dance styles / place names — for NL/puzzle parsing), so a `Noun`-tagged word
/// is NOT proof of recognition; the database lookup (`lexical_status == Unknown`)
/// is the real discriminator and keeps genuinely-known nouns ("cat") out. A
/// `Verb` tag still means the lexer recognized the word. Unknown capitalized words
/// are left as proper-name candidates (precision over recall).
fn is_actionable_gap(text: &str, category: &TokenCategory) -> bool {
    if !matches!(
        category,
        TokenCategory::Adjective | TokenCategory::Noun | TokenCategory::Other
    ) {
        return false;
    }
    let first_lower = text.chars().next().map(|c| c.is_lowercase()).unwrap_or(false);
    if !first_lower || !text.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    let lower = text.to_lowercase();
    if is_function_word(&lower) {
        return false;
    }
    lexical_status(&lower) == WordLex::Unknown
}

// ═══════════════════════════════════════════════════════════════════
// Metamorphic oracle — curated, high-precision paraphrase transforms
// ═══════════════════════════════════════════════════════════════════

/// Generate equivalent paraphrases of `input`. Each is a directional rewrite the
/// grammar should treat as meaning-preserving. v1 ships one high-precision rule
/// (fronted PP → trailing PP); the registry is structured for more.
fn metamorphic_variants(input: &str, result: &CompileResult) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(v) = pp_fronting_to_trailing(input, result) {
        out.push(("pp_fronting_to_trailing".to_string(), v));
    }
    out
}

/// "After the meeting, Mary left." → "Mary left after the meeting."
/// Fires only when the sentence opens with a preposition and contains a comma.
fn pp_fronting_to_trailing(input: &str, result: &CompileResult) -> Option<String> {
    let first = result.tokens.first()?;
    if first.category != TokenCategory::Preposition {
        return None;
    }
    let comma = input.find(',')?;
    let fronted = input[..comma].trim();
    let rest = input[comma + 1..].trim().trim_end_matches('.').trim();
    if fronted.is_empty() || rest.is_empty() {
        return None;
    }
    // Lower-case the moved preposition; keep the now-leading subject as written.
    let mut pp = String::new();
    let mut chars = fronted.chars();
    if let Some(c) = chars.next() {
        pp.extend(c.to_lowercase());
        pp.extend(chars);
    }
    Some(format!("{rest} {pp}."))
}

// ═══════════════════════════════════════════════════════════════════
// AST node accounting
// ═══════════════════════════════════════════════════════════════════

/// Returns (unhandled_nodes, total_nodes). An "other" node is the compiler's
/// debug fallback for a LogicExpr variant with no proper visualization.
pub fn count_nodes(node: &AstNode) -> (usize, usize) {
    let mut unhandled = if node.node_type == "other" { 1 } else { 0 };
    let mut total = 1;
    for child in &node.children {
        let (u, t) = count_nodes(child);
        unhandled += u;
        total += t;
    }
    (unhandled, total)
}

// ═══════════════════════════════════════════════════════════════════
// Classification
// ═══════════════════════════════════════════════════════════════════

/// Map a parse error to (category, subsystem, stable label). Lexicon-ish errors
/// (unknown quantifier/modal, comparative/superlative word forms) route to the
/// lexicon; semantic conflicts to a human; everything else to the parser.
fn classify_error(kind: &ParseErrorKind) -> (Category, Subsystem, &'static str) {
    use ParseErrorKind::*;
    match kind {
        UnknownQuantifier { .. } => (Category::ActionableLexiconGap, Subsystem::Lexicon, "unknown_quantifier"),
        UnknownModal { .. } => (Category::ActionableLexiconGap, Subsystem::Lexicon, "unknown_modal"),
        ExpectedSuperlativeAdjective => (Category::ActionableLexiconGap, Subsystem::Lexicon, "expected_superlative"),
        ExpectedComparativeAdjective => (Category::ActionableLexiconGap, Subsystem::Lexicon, "expected_comparative"),

        StativeProgressiveConflict => (Category::SemanticLossy, Subsystem::Semantics, "stative_progressive_conflict"),
        ScopeViolation(_) => (Category::AmbiguityHuman, Subsystem::Semantics, "scope_violation"),
        UnresolvedPronoun { .. } => (Category::AmbiguityHuman, Subsystem::Semantics, "unresolved_pronoun"),
        IsValueEquality { .. } => (Category::SemanticLossy, Subsystem::Semantics, "is_value_equality"),

        ExpectedContentWord { .. } => (Category::ParserGap, Subsystem::Parser, "expected_content_word"),
        ExpectedVerb { .. } => (Category::ParserGap, Subsystem::Parser, "expected_verb"),
        ExpectedCopula => (Category::ParserGap, Subsystem::Parser, "expected_copula"),
        GappingResolutionFailed => (Category::ParserGap, Subsystem::Parser, "gapping_resolution_failed"),
        ExpectedTemporalAdverb => (Category::ParserGap, Subsystem::Parser, "expected_temporal_adverb"),
        ExpectedPresuppositionTrigger => (Category::ParserGap, Subsystem::Parser, "expected_presupposition_trigger"),
        ExpectedFocusParticle => (Category::ParserGap, Subsystem::Parser, "expected_focus_particle"),
        ExpectedScopalAdverb => (Category::ParserGap, Subsystem::Parser, "expected_scopal_adverb"),
        ExpectedThan => (Category::ParserGap, Subsystem::Parser, "expected_than"),
        ExpectedNumber => (Category::ParserGap, Subsystem::Parser, "expected_number"),
        EmptyRestriction => (Category::ParserGap, Subsystem::Parser, "empty_restriction"),
        RespectivelyLengthMismatch { .. } => (Category::ParserGap, Subsystem::Parser, "respectively_length_mismatch"),
        UnexpectedToken { .. } => (Category::ParserGap, Subsystem::Parser, "unexpected_token"),
        GrammarError(_) => (Category::ParserGap, Subsystem::Parser, "grammar_error"),
        ExpectedKeyword { .. } => (Category::ParserGap, Subsystem::Parser, "expected_keyword"),

        // LOGOS imperative-mode and type errors: shouldn't appear for prose, but
        // if they do they are a human decision, not an autonomous fix.
        _ => (Category::AmbiguityHuman, Subsystem::None, "other"),
    }
}

fn gate_for(category: Category) -> Gate {
    match category {
        Category::ActionableLexiconGap => Gate::Auto,
        Category::ParserGap | Category::SemanticLossy => Gate::Investigate,
        Category::AmbiguityHuman | Category::IsolateOutOfScope => Gate::Human,
        Category::Clean => Gate::Human,
    }
}

/// Classify a single sentence. Calls the genuine compiler twice: `compile_for_ui`
/// for tokens/AST/FOL/readings, and the structured `compile` for the typed
/// `ParseError`. Read-only.
pub fn classify(index: usize, input: &str) -> TriageRecord {
    let result = compile_for_ui(input);
    let structured = logicaffeine_language::compile(input);
    let isolated = quarantine(input);

    // Lexicon misses among content tokens not inside an isolated span.
    let bytes = input.as_bytes();
    let mut lexicon_misses = Vec::new();
    for t in &result.tokens {
        if in_isolated(&isolated, t.start, t.end) {
            continue;
        }
        // Skip hyphen fragments like "co" in "co-authored".
        let touches_hyphen = (t.start > 0 && bytes[t.start - 1] == b'-')
            || (t.end < bytes.len() && bytes[t.end] == b'-');
        if touches_hyphen {
            continue;
        }
        if is_actionable_gap(&t.text, &t.category) {
            lexicon_misses.push(t.text.clone());
        }
    }
    lexicon_misses.dedup();

    let (ast_other, ast_total) = result.ast.as_ref().map(count_nodes).unwrap_or((0, 0));

    let mut localization = Localization {
        error_kind: String::new(),
        error_span: None,
        offending_text: None,
        suspect_words: lexicon_misses.clone(),
        isolated_spans: isolated.clone(),
    };

    let mut oracle = None;
    let mut proposal = Proposal::default();

    let (outcome, category, subsystem) = match &structured {
        Err(err) => {
            let (kind_label, span, offending) = describe_error(err, input);
            localization.error_kind = kind_label.clone();
            localization.error_span = span;
            localization.offending_text = offending.clone();

            // Priority: noise → lexicon gap at the failure point → error taxonomy.
            let offending_isolated = span
                .map(|(s, e)| in_isolated(&isolated, s, e))
                .unwrap_or(false);

            let offending_is_gap = offending
                .as_deref()
                .map(|w| lexicon_misses.iter().any(|m| m.eq_ignore_ascii_case(w)))
                .unwrap_or(false);

            if offending_isolated {
                (Outcome::Fail, Category::IsolateOutOfScope, Subsystem::None)
            } else if offending_is_gap || (!lexicon_misses.is_empty() && is_lexical_error(&err.kind))
            {
                if let Some(w) = lexicon_misses.first() {
                    proposal.lexicon_entry = Some(propose_lexicon_entry(w));
                }
                (Outcome::Fail, Category::ActionableLexiconGap, Subsystem::Lexicon)
            } else {
                let (cat, sub, _) = classify_error(&err.kind);
                // Try the metamorphic oracle: a paraphrase that parses is the spec.
                for (name, variant) in metamorphic_variants(input, &result) {
                    let vr = compile_for_ui(&variant);
                    if vr.error.is_none() {
                        if let Some(fol) = vr.logic.clone() {
                            proposal.red_test = Some(red_test_stub(index, input, &variant, &fol));
                            oracle = Some(Oracle {
                                transform: name,
                                variant_sentence: variant,
                                expected_fol: fol,
                                equivalence: "adopted".to_string(),
                            });
                            break;
                        }
                    }
                }
                (Outcome::Fail, cat, sub)
            }
        }
        Ok(_) => {
            // Successful parse. Decide whether it is clean or quietly broken.
            let collapsed = ast_total <= 1 && ast_other >= 1;
            if !lexicon_misses.is_empty() {
                if let Some(w) = lexicon_misses.first() {
                    proposal.lexicon_entry = Some(propose_lexicon_entry(w));
                }
                (Outcome::Partial, Category::ActionableLexiconGap, Subsystem::Lexicon)
            } else if collapsed {
                (Outcome::Partial, Category::SemanticLossy, Subsystem::Semantics)
            } else if ast_other > 0 {
                (Outcome::Partial, Category::SemanticLossy, Subsystem::Semantics)
            } else if result.readings.len() >= 6 {
                (Outcome::Ok, Category::AmbiguityHuman, Subsystem::Semantics)
            } else {
                (Outcome::Ok, Category::Clean, Subsystem::None)
            }
        }
    };

    let gate = gate_for(category);
    let confidence = confidence_for(category, outcome, &oracle, ast_other, ast_total);

    let fol = match &structured {
        Ok(_) => result.logic.clone(),
        Err(_) => result.error.clone(),
    };

    TriageRecord {
        index,
        input: input.to_string(),
        outcome,
        category,
        subsystem,
        gate,
        confidence,
        fol,
        localization,
        oracle,
        evidence: Evidence {
            reading_count: result.readings.len(),
            ast_nodes: ast_total,
            unhandled_ast_nodes: ast_other,
            lexicon_misses,
            dropped_content_words: Vec::new(),
        },
        proposal,
    }
}

/// Errors that are intrinsically lexical regardless of the offending token.
/// `ExpectedContentWord`/`ExpectedVerb` are deliberately excluded: their offending
/// token is often a preposition/period (a structural/parser issue, e.g. a fronted
/// PP), so those route through the offending-token-is-a-gap check instead.
fn is_lexical_error(kind: &ParseErrorKind) -> bool {
    matches!(
        kind,
        ParseErrorKind::UnknownQuantifier { .. } | ParseErrorKind::UnknownModal { .. }
    )
}

fn confidence_for(
    category: Category,
    _outcome: Outcome,
    oracle: &Option<Oracle>,
    ast_other: usize,
    ast_total: usize,
) -> f32 {
    match category {
        Category::Clean => 0.95,
        Category::IsolateOutOfScope => 0.9,
        Category::ActionableLexiconGap => 0.75,
        Category::ParserGap => {
            if oracle.is_some() {
                0.85
            } else {
                0.5
            }
        }
        Category::SemanticLossy => {
            if ast_total <= 1 && ast_other >= 1 {
                0.6
            } else {
                0.35
            }
        }
        Category::AmbiguityHuman => 0.5,
    }
}

/// Stable label + span + offending text for a parse error.
fn describe_error(err: &ParseError, input: &str) -> (String, Option<(usize, usize)>, Option<String>) {
    let label = classify_error(&err.kind).2.to_string();
    let start = err.span.start.min(input.len());
    let end = err.span.end.min(input.len());
    let span = Some((start, end));
    let offending = if start < end {
        Some(input[start..end].to_string())
    } else {
        // Zero-width span: name the token type the parser tripped on.
        found_token_name(&err.kind)
    };
    (label, span, offending)
}

fn found_token_name(kind: &ParseErrorKind) -> Option<String> {
    let tok = match kind {
        ParseErrorKind::UnexpectedToken { found, .. }
        | ParseErrorKind::ExpectedContentWord { found }
        | ParseErrorKind::UnknownQuantifier { found }
        | ParseErrorKind::UnknownModal { found }
        | ParseErrorKind::ExpectedVerb { found } => found,
        _ => return None,
    };
    Some(token_kind_name(tok).to_string())
}

fn token_kind_name(tok: &TokenType) -> &'static str {
    match tok {
        TokenType::Preposition(_) => "<preposition>",
        TokenType::Period => "<period>",
        TokenType::Comma => "<comma>",
        TokenType::Article(_) => "<article>",
        _ => "<token>",
    }
}

fn propose_lexicon_entry(word: &str) -> LexiconEntry {
    let mut lemma = String::new();
    let mut chars = word.chars();
    if let Some(c) = chars.next() {
        lemma.extend(c.to_uppercase());
        lemma.extend(chars);
    }
    LexiconEntry {
        lemma,
        pos: guess_pos(word).to_string(),
        note: "POS inferred from suffix; verify before adding".to_string(),
    }
}

/// Suffix-based part-of-speech guess for a proposed lexicon entry. Conservative
/// and clearly a guess — the loop must confirm before adding the entry.
fn guess_pos(word: &str) -> &'static str {
    let w = word.to_lowercase();
    let ends = |s: &str| w.ends_with(s);
    if ends("ize") || ends("ise") || ends("ify") || ends("fy") {
        "Verb"
    } else if ends("ive") || ends("ous") || ends("ful") || ends("less")
        || ends("able") || ends("ible") || ends("ical") || ends("ic")
        || ends("al") || ends("ary") || ends("ed")
    {
        "Adjective"
    } else if ends("tion") || ends("sion") || ends("ment") || ends("ity")
        || ends("ness") || ends("ance") || ends("ence") || ends("ship")
        || ends("ism") || ends("ist") || ends("ology") || ends("er") || ends("or")
    {
        "Noun"
    } else {
        "Noun"
    }
}

fn red_test_stub(index: usize, input: &str, variant: &str, expected_fol: &str) -> String {
    format!(
        "#[test]\n\
         fn triage_{index:02}_paraphrase_equivalence() {{\n    \
         // The failing form should compile like its parsing paraphrase.\n    \
         // input:    {input:?}\n    \
         // oracle:   {variant:?}\n    \
         // expected: {expected_fol}\n    \
         let fol = compile({input:?}).expect(\"should parse like its paraphrase\");\n    \
         assert!(!fol.is_empty(), \"got: {{}}\", fol);\n}}\n"
    )
}

// ═══════════════════════════════════════════════════════════════════
// Clustering — dedup by root cause
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize)]
pub struct Cluster {
    pub signature: String,
    pub category: Category,
    pub subsystem: Subsystem,
    pub gate: Gate,
    pub count: usize,
    pub members: Vec<usize>,
    pub example: String,
}

/// Group records by `(category, subsystem, root-cause signature)` so the loop
/// fixes a class, not N duplicates.
pub fn cluster(records: &[TriageRecord]) -> Vec<Cluster> {
    use std::collections::BTreeMap;
    let mut map: BTreeMap<String, Cluster> = BTreeMap::new();
    for r in records {
        if r.category == Category::Clean {
            continue;
        }
        let sig = signature(r);
        let entry = map.entry(sig.clone()).or_insert_with(|| Cluster {
            signature: sig,
            category: r.category,
            subsystem: r.subsystem,
            gate: r.gate,
            count: 0,
            members: Vec::new(),
            example: r.input.clone(),
        });
        entry.count += 1;
        entry.members.push(r.index);
    }
    let mut out: Vec<Cluster> = map.into_values().collect();
    out.sort_by(|a, b| b.count.cmp(&a.count));
    out
}

fn signature(r: &TriageRecord) -> String {
    match r.category {
        Category::ActionableLexiconGap => format!(
            "lexicon:{}",
            r.localization.suspect_words.first().cloned().unwrap_or_default()
        ),
        Category::ParserGap => format!("parser:{}", r.localization.error_kind),
        Category::SemanticLossy => "semantics:lossy".to_string(),
        Category::AmbiguityHuman => format!("human:{}", r.localization.error_kind),
        Category::IsolateOutOfScope => "isolate".to_string(),
        Category::Clean => "clean".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abbreviation_detector_precision() {
        for w in ["EEG", "ERPs", "BSc", "MSc", "PhD", "N2pc"] {
            assert!(looks_like_abbreviation(w), "{w} should look like an abbreviation");
        }
        for w in ["Mary", "Paris", "cat", "lecture", "the"] {
            assert!(!looks_like_abbreviation(w), "{w} should NOT be an abbreviation");
        }
    }

    #[test]
    fn pos_guess_by_suffix() {
        assert_eq!(guess_pos("cognitive"), "Adjective");
        assert_eq!(guess_pos("neural"), "Adjective");
        assert_eq!(guess_pos("publications"), "Noun");
        assert_eq!(guess_pos("stimulation"), "Noun");
        assert_eq!(guess_pos("characterize"), "Verb");
    }

    #[test]
    fn function_words_are_never_gaps() {
        for w in ["is", "are", "from", "where", "of", "the", "than"] {
            assert!(
                !is_actionable_gap(w, &TokenCategory::Other),
                "{w} is a function word, never a lexicon gap"
            );
        }
    }
}
