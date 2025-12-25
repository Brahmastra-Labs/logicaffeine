//! Multi-Word Expression (MWE) processing
//!
//! Post-tokenization pipeline that collapses multi-token sequences
//! into single semantic units (e.g., "fire engine" -> FireEngine).

use std::collections::HashMap;
use crate::token::{Token, TokenType};
use crate::lexicon::{VerbClass, Time, Aspect};
use crate::intern::Interner;

#[derive(Debug, Clone)]
pub struct MweTarget {
    pub lemma: &'static str,
    pub pos: &'static str,
    pub class: Option<VerbClass>,
}

#[derive(Default, Debug)]
pub struct MweTrie {
    pub children: HashMap<String, MweTrie>,
    pub target: Option<MweTarget>,
}

impl MweTrie {
    pub fn insert(&mut self, pattern: &[&str], target: MweTarget) {
        if pattern.is_empty() {
            self.target = Some(target);
            return;
        }
        self.children
            .entry(pattern[0].to_lowercase())
            .or_default()
            .insert(&pattern[1..], target);
    }
}

/// Apply MWE collapsing to a token stream.
/// Matches on lemmas (not raw strings) to handle morphological variants.
pub fn apply_mwe_pipeline(
    tokens: Vec<Token>,
    trie: &MweTrie,
    interner: &mut Interner,
) -> Vec<Token> {
    let mut result = Vec::new();
    let mut i = 0;

    while i < tokens.len() {
        if let Some((match_len, target)) = find_longest_match(&tokens[i..], trie, interner) {
            let merged = create_merged_token(&tokens[i], target, interner);
            result.push(merged);
            i += match_len;
        } else {
            result.push(tokens[i].clone());
            i += 1;
        }
    }
    result
}

/// Extract lemma from a token for MWE matching.
/// Uses lowercase for case-insensitive matching.
fn get_lemma(token: &Token, interner: &Interner) -> String {
    match &token.kind {
        TokenType::Verb { lemma, .. } => interner.resolve(*lemma).to_lowercase(),
        TokenType::Noun(sym) => interner.resolve(*sym).to_lowercase(),
        TokenType::Adjective(sym) => interner.resolve(*sym).to_lowercase(),
        TokenType::NonIntersectiveAdjective(sym) => interner.resolve(*sym).to_lowercase(),
        TokenType::Preposition(sym) => interner.resolve(*sym).to_lowercase(),
        TokenType::Particle(sym) => interner.resolve(*sym).to_lowercase(),
        TokenType::Article(_) => interner.resolve(token.lexeme).to_lowercase(),
        _ => interner.resolve(token.lexeme).to_lowercase(),
    }
}

/// Find the longest MWE match starting at the beginning of the token slice.
fn find_longest_match<'a>(
    tokens: &[Token],
    trie: &'a MweTrie,
    interner: &Interner,
) -> Option<(usize, &'a MweTarget)> {
    let mut node = trie;
    let mut best: Option<(usize, &MweTarget)> = None;

    for (i, token) in tokens.iter().enumerate() {
        let lemma = get_lemma(token, interner);
        if let Some(child) = node.children.get(&lemma) {
            node = child;
            if let Some(target) = &node.target {
                best = Some((i + 1, target));
            }
        } else {
            break;
        }
    }
    best
}

/// Create a merged token from the MWE target, inheriting tense from the head token.
fn create_merged_token(head: &Token, target: &MweTarget, interner: &mut Interner) -> Token {
    let lemma_sym = interner.intern(target.lemma);

    let kind = match target.pos {
        "Noun" => TokenType::Noun(lemma_sym),
        "Verb" => {
            let (time, aspect) = match &head.kind {
                TokenType::Verb { time, aspect, .. } => (*time, *aspect),
                _ => (Time::Present, Aspect::Simple),
            };
            TokenType::Verb {
                lemma: lemma_sym,
                time,
                aspect,
                class: target.class.unwrap_or(VerbClass::Activity),
            }
        }
        "Preposition" => TokenType::Preposition(lemma_sym),
        "Conjunction" => TokenType::And,
        "Quantifier" => TokenType::NoOne,
        _ => TokenType::Noun(lemma_sym),
    };

    Token {
        kind,
        lexeme: lemma_sym,
        span: head.span,
    }
}

include!(concat!(env!("OUT_DIR"), "/mwe_data.rs"));
