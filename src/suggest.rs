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
            let cost = if a_chars[i - 1] == b_chars[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1)
                .min(curr[j - 1] + 1)
                .min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

pub fn find_similar<'a>(word: &str, candidates: &[&'a str], max_distance: usize) -> Option<&'a str> {
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

pub const KNOWN_WORDS: &[&str] = &[
    "all", "some", "no", "most", "few", "every",
    "the", "a", "an", "this", "that",
    "is", "are", "was", "were", "be",
    "and", "or", "if", "then", "not",
    "must", "can", "may", "should", "would", "could",
    "who", "what", "where", "when", "why", "how",
    "man", "men", "woman", "women", "dog", "cat", "bird",
    "mortal", "happy", "sad", "tall", "fast", "slow",
    "loves", "runs", "sees", "knows", "thinks",
    "logic", "reason", "truth", "false",
    "John", "Mary", "Socrates", "Aristotle",
    "to", "by", "with", "from", "for", "in", "on", "at",
    "himself", "herself", "itself", "themselves",
    "he", "she", "it", "they", "him", "her", "them",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_identical() {
        assert_eq!(levenshtein("hello", "hello"), 0);
    }

    #[test]
    fn levenshtein_one_char_diff() {
        assert_eq!(levenshtein("hello", "hallo"), 1);
    }

    #[test]
    fn levenshtein_insertion() {
        assert_eq!(levenshtein("hello", "helllo"), 1);
    }

    #[test]
    fn levenshtein_deletion() {
        assert_eq!(levenshtein("hello", "helo"), 1);
    }

    #[test]
    fn levenshtein_empty() {
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
    }

    #[test]
    fn levenshtein_transposition() {
        assert_eq!(levenshtein("ab", "ba"), 2);
    }

    #[test]
    fn find_similar_typo() {
        let result = find_similar("logoc", KNOWN_WORDS, 2);
        assert_eq!(result, Some("logic"));
    }

    #[test]
    fn find_similar_no_match() {
        let result = find_similar("xyzzy", KNOWN_WORDS, 2);
        assert_eq!(result, None);
    }

    #[test]
    fn find_similar_case_insensitive() {
        let result = find_similar("LOGIC", KNOWN_WORDS, 2);
        assert_eq!(result, Some("logic"));
    }
}
