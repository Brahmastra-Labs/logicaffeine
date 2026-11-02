//! Symbol Dictionary Extraction
//!
//! Extracts logical symbols from FOL strings for display in a symbol dictionary.
//! Groups symbols by kind and provides descriptions.

use std::collections::HashSet;

/// Categories of logical symbols
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Quantifier,
    Connective,
    Variable,
    Predicate,
    Constant,
    Modal,
    Identity,
    Punctuation,
    Temporal,
    Role,
}

impl SymbolKind {
    pub fn label(&self) -> &'static str {
        match self {
            SymbolKind::Quantifier => "Quantifier",
            SymbolKind::Connective => "Connective",
            SymbolKind::Variable => "Variable",
            SymbolKind::Predicate => "Predicate",
            SymbolKind::Constant => "Constant",
            SymbolKind::Modal => "Modal",
            SymbolKind::Identity => "Identity",
            SymbolKind::Punctuation => "Punctuation",
            SymbolKind::Temporal => "Temporal",
            SymbolKind::Role => "Role",
        }
    }
}

/// A single symbol entry in the dictionary
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SymbolEntry {
    pub symbol: String,
    pub kind: SymbolKind,
    /// What the symbol means, in words — never the symbol said again.
    pub description: String,
    /// The symbol as the formula writes it: `Dog(x)`, `See(John, x)`, `∀`.
    pub usage: String,
}

/// Extract symbols from a FOL logic string as the Unicode (Full) view prints it.
pub fn extract_symbols(logic: &str) -> Vec<SymbolEntry> {
    extract_symbols_for(logic, &crate::formatter::UnicodeFormatter)
}

/// Extract symbols from `logic` as `view` printed it: each view writes its symbols its own way
/// (`∀` or `ForAll`, `P` or `Past`), and the dictionary reads them in that notation.
pub fn extract_symbols_for(logic: &str, view: &impl crate::formatter::LogicFormatter) -> Vec<SymbolEntry> {
    let notation = Notation::of(view);
    let mut entries = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    extract_fixed_symbols(logic, &notation, &mut entries, &mut seen);
    extract_applications(logic, &notation, &mut entries, &mut seen);
    extract_variables(logic, &notation, &mut entries, &mut seen);
    extract_constants(logic, &notation, &mut entries, &mut seen);
    entries
}

/// Identity, with what it means.
const IDENTITY: (&str, &str) = ("=", "… is the same individual as …");
/// The deontic operator LOGOS prints for "ought" and "should", with what it means.
const OUGHT: (&str, &str) = ("O", "it ought to be that …");

/// The symbols LOGOS writes with a fixed spelling, as the Unicode view prints them, each with what
/// it means: the quantifiers, the connectives, the modals, identity, and the deontic O.
pub fn notation() -> Vec<SymbolEntry> {
    let notation = Notation::of(&crate::formatter::UnicodeFormatter);
    let entry = |symbol: &str, kind: SymbolKind, meaning: &str| SymbolEntry {
        usage: symbol.to_string(),
        symbol: symbol.to_string(),
        kind,
        description: meaning.to_string(),
    };
    notation
        .fixed
        .iter()
        .filter(|(symbol, _, _)| !symbol.is_empty())
        .map(|(symbol, kind, meaning)| entry(symbol, *kind, meaning))
        .chain([entry(IDENTITY.0, SymbolKind::Identity, IDENTITY.1), entry(OUGHT.0, SymbolKind::Modal, OUGHT.1)])
        .collect()
}

/// How one view writes the symbols with a fixed spelling, each with what it means.
struct Notation {
    /// Quantifiers, connectives and modals: the view's spelling, its kind, its meaning.
    fixed: Vec<(String, SymbolKind, &'static str)>,
    /// The quantifier spellings that bind a variable, and which quantifier each is.
    binders: Vec<(String, &'static str)>,
    /// Tense and aspect operators: the view's name, what it means wrapped around a formula, and
    /// what it says applied to an event (`Past(e)`: "e lies in the past").
    operators: Vec<(&'static str, &'static str, &'static str)>,
}

impl Notation {
    fn of(view: &impl crate::formatter::LogicFormatter) -> Notation {
        let spelled = |s: &str| s.trim().to_string();
        let (every, some) = (spelled(&view.universal()), spelled(&view.existential()));
        Notation {
            fixed: vec![
                (every.clone(), SymbolKind::Quantifier, "for every …"),
                (some.clone(), SymbolKind::Quantifier, "there is at least one …"),
                ("∃!".to_string(), SymbolKind::Quantifier, "there is exactly one …"),
                ("MOST".to_string(), SymbolKind::Quantifier, "most …"),
                ("FEW".to_string(), SymbolKind::Quantifier, "few …"),
                (spelled(view.and()), SymbolKind::Connective, "… and …"),
                (spelled(view.or()), SymbolKind::Connective, "… or …"),
                (spelled(view.implies()), SymbolKind::Connective, "if … then …"),
                (spelled(view.iff()), SymbolKind::Connective, "… exactly when …"),
                (spelled(view.not()), SymbolKind::Connective, "it is not the case that …"),
                (spelled(view.necessity()), SymbolKind::Modal, "necessarily …"),
                (spelled(view.possibility()), SymbolKind::Modal, "possibly …"),
            ],
            binders: vec![(every, "∀"), (some, "∃")],
            operators: vec![
                (view.past(), "past tense — it was the case that …", "lies in the past"),
                (view.future(), "future tense — it will be the case that …", "lies in the future"),
                (view.progressive(), "progressive — … is going on", "is going on"),
                (view.perfect(), "perfect — … has been completed", "has been completed"),
                (view.habitual(), "habitual — … happens as a habit", "happens as a habit"),
                (view.iterative(), "iterative — … happens again and again", "happens again and again"),
            ],
        }
    }

    /// Whether `name` is one of the view's fixed symbols spelled as a word (`Box`, `Not`).
    fn is_fixed_word(&self, name: &str) -> bool {
        self.fixed.iter().any(|(symbol, _, _)| symbol == name)
    }
}

/// Whether `logic` writes `symbol`: anywhere for a sign (`∀`), only as a whole word for a word
/// (`And`, so that `Android` is no conjunction).
fn mentions(logic: &str, symbol: &str) -> bool {
    if !symbol.chars().any(char::is_alphabetic) {
        return logic.contains(symbol);
    }
    logic.match_indices(symbol).any(|(at, _)| {
        let before = logic[..at].chars().next_back();
        let after = logic[at + symbol.len()..].chars().next();
        !before.is_some_and(|c| c.is_alphanumeric()) && !after.is_some_and(|c| c.is_alphanumeric())
    })
}

/// The symbols with a fixed spelling: quantifiers, connectives, modals, identity.
fn extract_fixed_symbols(logic: &str, notation: &Notation, entries: &mut Vec<SymbolEntry>, seen: &mut HashSet<String>) {
    let mut add = |symbol: &str, kind: SymbolKind, description: &str| {
        if seen.insert(symbol.to_string()) {
            entries.push(SymbolEntry { usage: symbol.to_string(), symbol: symbol.to_string(), kind, description: description.to_string() });
        }
    };
    for (symbol, kind, meaning) in &notation.fixed {
        if !symbol.is_empty() && mentions(logic, symbol) {
            add(symbol, *kind, meaning);
        }
    }
    if logic.contains("O_") {
        add(OUGHT.0, SymbolKind::Modal, OUGHT.1);
    }
    if logic.contains(" = ") {
        add(IDENTITY.0, SymbolKind::Identity, IDENTITY.1);
    }
}

/// Whether the argument list opening at `chars[open]` begins with a formula (a quantifier, a
/// connective, a modal, or a predicate applied to arguments) rather than a term.
fn wraps_formula(chars: &[char], open: usize, notation: &Notation) -> bool {
    let mut i = open + 1;
    while i < chars.len() && chars[i] == ' ' {
        i += 1;
    }
    let rest: String = chars[i..].iter().take(16).collect();
    if rest.starts_with('(') || notation.fixed.iter().any(|(symbol, kind, _)| {
        !symbol.is_empty() && *kind != SymbolKind::Connective && rest.starts_with(symbol.as_str())
    }) || rest.starts_with('¬') || rest.starts_with("Not ") {
        return true;
    }
    match chars.get(i) {
        Some(c) if c.is_ascii_uppercase() => {
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            chars.get(i) == Some(&'(')
        }
        _ => false,
    }
}

/// The arguments of the list opening at `chars[open]`, as written, and the index just past it.
fn arguments(chars: &[char], open: usize) -> (Vec<String>, usize) {
    let mut depth = 0;
    let mut args = Vec::new();
    let mut current = String::new();
    let mut i = open;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '(' => {
                depth += 1;
                if depth > 1 {
                    current.push(c);
                }
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    args.push(current.trim().to_string());
                    return (args, i + 1);
                }
                current.push(c);
            }
            ',' if depth == 1 => args.push(std::mem::take(&mut current).trim().to_string()),
            _ => current.push(c),
        }
        i += 1;
    }
    (args, i)
}

/// A variable that names an event: `e`, `e1`, `e2`.
fn is_event(term: &str) -> bool {
    term.starts_with('e') && term[1..].chars().all(|c| c.is_ascii_digit())
}

/// A possible world the Deep view evaluates in: `w0`, `w1`. It says where, not who.
fn is_world(term: &str) -> bool {
    term.len() > 1 && term.starts_with('w') && term[1..].chars().all(|c| c.is_ascii_digit())
}

fn article(noun: &str) -> &'static str {
    if noun.starts_with(|c: char| "aeiou".contains(c)) { "an" } else { "a" }
}

/// What `Name(args)` says, in words, as the lexicon reads the name: "x is a dog", "John sees x",
/// "x is red", "e happens quickly". A name the lexicon does not know reads as a noun, as a
/// restrictor usually is.
fn reading(name: &str, args: &[String]) -> String {
    use crate::lexicon::{analyze_word, LexiconTrait, StaticLexicon, WordAnalysis};
    let lower = name.to_lowercase();
    let noun = || match analyze_word(&lower) {
        Some(WordAnalysis::Noun(meta)) => Some(meta.lemma.to_lowercase()),
        Some(WordAnalysis::DerivedNoun { lemma, .. }) => Some(lemma.to_lowercase()),
        None => None,
    };
    let verb = StaticLexicon.lookup_verb(&lower).map(|meta| meta.lemma.to_lowercase());
    match (args, verb) {
        ([], _) => format!("the proposition {name}"),
        ([e], Some(verb)) if is_event(e) => format!("{e} is a {verb} event"),
        ([a], Some(verb)) => format!("{a} {}", logicaffeine_lexicon::present_3s_of(&verb)),
        ([a, rest @ ..], Some(verb)) => format!("{a} {} {}", logicaffeine_lexicon::present_3s_of(&verb), rest.join(" ")),
        ([a], None) if StaticLexicon.lookup_adjective(&lower).is_some() => format!("{a} is {lower}"),
        // A manner adverb (`Quickly`) says how the event goes, of the event or, once the Simple
        // view has flattened it, of whoever does it.
        ([e], None) if lower.ends_with("ly") && noun().is_none() && is_event(e) => format!("{e} happens {lower}"),
        ([a], None) if lower.ends_with("ly") && noun().is_none() => format!("{a} does it {lower}"),
        ([a], None) => {
            let noun = noun().unwrap_or(lower);
            format!("{a} is {} {noun}", article(&noun))
        }
        (args, None) => format!("{} stand in the relation {name}", args.join(", ")),
    }
}

/// Every `Name(…)` in `logic`: a tense or aspect wrapped around a formula, a thematic role relating
/// an event to a participant, or else a predicate — each with how the formula writes it and what
/// it means.
fn extract_applications(logic: &str, notation: &Notation, entries: &mut Vec<SymbolEntry>, seen: &mut HashSet<String>) {
    use crate::ast::ThematicRole;
    let chars: Vec<char> = logic.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_uppercase() && (i == 0 || !chars[i - 1].is_ascii_alphanumeric()) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            if i < chars.len() && chars[i] == '(' {
                let name: String = chars[start..i].iter().collect();
                if notation.is_fixed_word(&name) {
                    continue;
                }
                let (written, _) = arguments(&chars, i);
                let args: Vec<String> = written.iter().filter(|a| !is_world(a)).cloned().collect();
                let operator = notation.operators.iter().find(|(op, _, _)| *op == name);
                let role = ThematicRole::ALL.iter().any(|r| r.name() == name) && args.len() >= 2;
                // An event's tense is the parser's `Past`/`Future` modifier in every view.
                let of_event = operator.map(|(_, _, of_event)| *of_event).or(match name.as_str() {
                    "Past" => Some("lies in the past"),
                    "Future" => Some("lies in the future"),
                    _ => None,
                });
                let placed = match (of_event, args.as_slice()) {
                    (Some(of_event), [e]) if is_event(e) => Some(format!("{e} {of_event}")),
                    _ => None,
                };
                let (kind, usage, description) = match (operator.filter(|_| wraps_formula(&chars, i, notation)), placed, role) {
                    (Some((_, meaning, _)), _, _) => (SymbolKind::Temporal, format!("{name}(…)"), meaning.to_string()),
                    (None, Some(placed), _) => (SymbolKind::Temporal, format!("{name}({})", written.join(", ")), placed),
                    (None, None, true) => (
                        SymbolKind::Role,
                        format!("{name}({})", written.join(", ")),
                        format!("{} is the {} of {}", args[1..].join(", "), name.to_lowercase(), args[0]),
                    ),
                    (None, None, false) => (SymbolKind::Predicate, format!("{name}({})", written.join(", ")), reading(&name, &args)),
                };
                if seen.insert(format!("{kind:?}_{name}")) {
                    entries.push(SymbolEntry { usage, symbol: name, kind, description });
                }
                continue;
            }
            continue;
        }
        i += 1;
    }
}

fn extract_variables(logic: &str, notation: &Notation, entries: &mut Vec<SymbolEntry>, seen: &mut HashSet<String>) {
    // A variable is introduced by its binder — `∀x`, `ForAll x`, `∃e` — and means what that binder
    // says of it.
    for (binder, quantifier) in &notation.binders {
        for (at, _) in logic.match_indices(binder.as_str()) {
            let after = logic[at + binder.len()..].trim_start_matches(|c: char| c == ' ' || c == '!' || c == '=' || c == '≥' || c == '≤' || c.is_ascii_digit());
            let var: String = after
                .chars()
                .take(1)
                .filter(char::is_ascii_lowercase)
                .chain(after.chars().skip(1).take_while(char::is_ascii_digit))
                .collect();
            if var.is_empty() || after.chars().nth(var.len()).is_some_and(|c| c.is_ascii_alphabetic()) {
                continue;
            }
            let description = match (is_event(&var), *quantifier) {
                (true, _) => format!("an event, bound by {binder}"),
                (false, "∀") => format!("any individual, bound by {binder}"),
                (false, _) => format!("some individual, bound by {binder}"),
            };
            if seen.insert(format!("var_{var}")) {
                entries.push(SymbolEntry { usage: var.clone(), symbol: var, kind: SymbolKind::Variable, description });
            }
        }
    }
}

fn extract_constants(logic: &str, notation: &Notation, entries: &mut Vec<SymbolEntry>, seen: &mut HashSet<String>) {
    // Constants are capitalised names standing as terms: John, Mary, J2 — never applied to
    // arguments (a predicate or operator), and never a generalized quantifier's keyword.
    let chars: Vec<char> = logic.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i].is_ascii_uppercase() && (i == 0 || !chars[i - 1].is_ascii_alphanumeric()) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric()) {
                i += 1;
            }
            if i >= chars.len() || (chars[i] != '(' && chars[i] != '_') {
                let constant: String = chars[start..i].iter().collect();
                if !["MOST", "FEW", "ALL", "THE"].contains(&constant.as_str())
                    && !notation.is_fixed_word(&constant)
                    && seen.insert(format!("const_{}", constant))
                {
                    entries.push(SymbolEntry {
                        usage: constant.clone(),
                        symbol: constant.clone(),
                        kind: SymbolKind::Constant,
                        description: "a particular individual".to_string(),
                    });
                }
            }
        }
        i += 1;
    }
}

/// Get symbols grouped by kind for display
pub fn group_symbols_by_kind(entries: &[SymbolEntry]) -> Vec<(SymbolKind, Vec<&SymbolEntry>)> {
    let kinds = [
        SymbolKind::Quantifier,
        SymbolKind::Connective,
        SymbolKind::Modal,
        SymbolKind::Temporal,
        SymbolKind::Identity,
        SymbolKind::Predicate,
        SymbolKind::Role,
        SymbolKind::Variable,
        SymbolKind::Constant,
    ];

    kinds
        .iter()
        .filter_map(|&kind| {
            let matching: Vec<_> = entries.iter().filter(|e| e.kind == kind).collect();
            if matching.is_empty() {
                None
            } else {
                Some((kind, matching))
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_language/symbol_dict/tests.rs"]
mod tests;

