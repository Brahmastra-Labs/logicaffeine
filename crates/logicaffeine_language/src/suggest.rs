//! Spelling suggestions for unknown words.
//!
//! This module provides fuzzy matching to suggest corrections for misspelled
//! words in the input. It uses Levenshtein distance to find similar words
//! from a known vocabulary list.
//!
//! When the parser encounters an unknown word, this module can suggest
//! "Did you mean X?" alternatives based on edit distance.

/// Compute the Levenshtein edit distance between two strings.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }

    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0; n + 1];

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

pub fn find_similar<'a>(
    word: &str,
    candidates: &[&'a str],
    max_distance: usize,
) -> Option<&'a str> {
    let word_lower = word.to_lowercase();
    let mut best: Option<(&str, usize)> = None;

    for &candidate in candidates {
        let dist = levenshtein(&word_lower, &candidate.to_lowercase());
        if dist <= max_distance {
            match best {
                None => best = Some((candidate, dist)),
                Some((_, d)) if dist < d => best = Some((candidate, dist)),
                _ => {}
            }
        }
    }

    best.map(|(s, _)| s)
}

/// The candidate an author most likely meant by `word` — the nearest by edit distance, compared
/// case-insensitively, when it is within a third of the word's length (at least one edit). The
/// first of equally near candidates wins, so a caller that lists candidates in a stable order gets
/// a stable answer. `word` itself never counts as a suggestion for itself.
///
/// The one did-you-mean rule every LGX refusal uses: a misspelt event qualifier, an unknown
/// component, an unresolved name in a template, a prop a placement does not declare.
pub fn did_you_mean<'c>(
    word: &str,
    candidates: impl IntoIterator<Item = &'c str>,
) -> Option<&'c str> {
    let lower = word.to_lowercase();
    let budget = (lower.chars().count() / 3).max(1);
    let mut best: Option<(usize, &'c str)> = None;
    for candidate in candidates {
        if candidate == word {
            continue;
        }
        let distance = typo_distance(&lower, &candidate.to_lowercase());
        if distance <= budget && best.map_or(true, |(d, _)| distance < d) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, c)| c)
}

/// Edit distance where swapping two adjacent characters is one edit (the optimal string alignment
/// distance) — the typo a fast typist makes most (`doen` for `done`), which plain Levenshtein
/// counts as two.
pub fn typo_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    let mut d = vec![vec![0usize; n + 1]; m + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=n {
        d[0][j] = j;
    }
    for i in 1..=m {
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[m][n]
}

pub const KNOWN_WORDS: &[&str] = &[
    "all",
    "some",
    "no",
    "most",
    "few",
    "every",
    "the",
    "a",
    "an",
    "this",
    "that",
    "is",
    "are",
    "was",
    "were",
    "be",
    "and",
    "or",
    "if",
    "then",
    "not",
    "must",
    "can",
    "may",
    "should",
    "would",
    "could",
    "who",
    "what",
    "where",
    "when",
    "why",
    "how",
    "man",
    "men",
    "woman",
    "women",
    "dog",
    "cat",
    "bird",
    "mortal",
    "happy",
    "sad",
    "tall",
    "fast",
    "slow",
    "loves",
    "runs",
    "sees",
    "knows",
    "thinks",
    "logic",
    "reason",
    "truth",
    "false",
    "John",
    "Mary",
    "Socrates",
    "Aristotle",
    "to",
    "by",
    "with",
    "from",
    "for",
    "in",
    "on",
    "at",
    "himself",
    "herself",
    "itself",
    "themselves",
    "he",
    "she",
    "it",
    "they",
    "him",
    "her",
    "them",
];

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_language/suggest/tests.rs"]
mod tests;
