//! LOCK — every word table answers every word exactly as the `match` it replaced.
//!
//! `logicaffeine_language/build.rs` writes each lookup a word answers — `lookup_keyword`,
//! `is_common_noun`, `lookup_sort`, the ontology's and the axioms' tables, the phrasal verbs — as a
//! sorted key list searched with `binary_search`, answering through an integer `match`. Two halves
//! hold every table to that:
//! - the census: each table's keys are sorted and duplicate-free, so `binary_search` finds every
//!   one of them; and a key written twice with two different answers keeps the first — as the
//!   `match` kept it — and is one this file names, with the reason it is still there;
//! - the differential: under the `lexicon-reference` feature, `lexicon::reference` writes every
//!   table as the linear `match` it replaced, arm by arm, and `differences` / `pair_differences`
//!   name the tables whose answer is not their reference's. Every word the tables or the lexicon
//!   know, in every case, a letter longer and a letter shorter, and words nobody knows — longer
//!   than any key, outside ASCII, lowercasing into ASCII — answer the same from both.

use logicaffeine_language::lexicon::reference::{differences, pair_differences};
use logicaffeine_language::{lexicon, ontology, semantics};
use std::collections::BTreeSet;

/// Every word table the language crate generates, by module, with its keys.
fn tables() -> Vec<(&'static str, &'static str, &'static [&'static str])> {
    [
        ("lexicon", lexicon::WORD_TABLES),
        ("ontology", ontology::WORD_TABLES),
        ("semantics", semantics::WORD_TABLES),
    ]
    .into_iter()
    .flat_map(|(module, tables)| {
        tables
            .iter()
            .map(move |(name, keys)| (module, *name, *keys))
    })
    .collect()
}

/// Every string of the lexicon's JSON: each lemma, form, feature, class, sort, object key.
fn lexicon_strings() -> BTreeSet<String> {
    fn walk(value: &serde_json::Value, into: &mut BTreeSet<String>) {
        match value {
            serde_json::Value::String(s) => {
                into.insert(s.clone());
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| walk(v, into)),
            serde_json::Value::Object(fields) => {
                for (key, v) in fields {
                    into.insert(key.clone());
                    walk(v, into);
                }
            }
            _ => {}
        }
    }
    let json: serde_json::Value =
        serde_json::from_str(logicaffeine_lexicon::LEXICON_JSON).expect("the lexicon parses");
    let mut strings = BTreeSet::new();
    walk(&json, &mut strings);
    strings
}

/// `word` as a reader might write it: as is, lowercased, uppercased, capitalized, a letter longer,
/// a letter shorter, and with each `k` written as the Kelvin sign — not ASCII, lowercasing to `k`.
fn variants(word: &str) -> Vec<String> {
    let mut capitalized = word.to_lowercase();
    if let Some(first) = capitalized.chars().next() {
        capitalized.replace_range(..first.len_utf8(), &first.to_uppercase().to_string());
    }
    let mut shorter = word.to_string();
    shorter.pop();
    vec![
        word.to_string(),
        word.to_lowercase(),
        word.to_uppercase(),
        capitalized,
        format!("{word}s"),
        shorter,
        word.replace(['k', 'K'], "\u{212A}"),
    ]
}

/// Words no table knows, and words at the edges of the case fold.
const STRANGERS: &[&str] = &[
    "",
    " ",
    "_",
    "zzqx",
    "the ",
    " the",
    "qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq",
    "QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ",
    "naïve",
    "NAÏVE",
    "straße",
    "STRASSE",
    "\u{130}",
    "\u{130}t",
    "\u{1C5}",
    "\u{212A}",
    "don't",
    "can't",
    "isn",
    "ISN",
    "1",
    "3.14",
    "give_up",
];

/// Every word the differential asks each table about.
fn words() -> BTreeSet<String> {
    let mut words = BTreeSet::new();
    for (_, _, keys) in tables() {
        for key in keys {
            words.extend(variants(key));
        }
    }
    for s in lexicon_strings() {
        words.extend(variants(&s));
    }
    for s in STRANGERS {
        words.extend(variants(s));
    }
    words
}

#[test]
fn the_census_reads_every_module_it_names() {
    let all = tables();
    assert!(
        all.len() >= 60,
        "the census sees only {} word tables — the registry is broken",
        all.len()
    );
    for (module, name) in [
        ("lexicon", "lookup_keyword"),
        ("lexicon", "lookup_pronoun"),
        ("lexicon", "is_common_noun"),
        ("lexicon", "lookup_verb_db"),
        ("lexicon", "lookup_sort"),
        ("lexicon", "lookup_canonical"),
        ("lexicon", "lookup_negative_contraction"),
        ("lexicon", "lookup_phrasal_verb"),
        ("lexicon", "lookup_agentive_noun"),
        ("ontology", "get_possible_wholes"),
        ("ontology", "get_predicate_sort"),
        ("semantics", "lookup_noun_entailments"),
        ("semantics", "lookup_verb_entailment"),
    ] {
        let found = all.iter().find(|(m, n, _)| *m == module && *n == name);
        let (_, _, keys) = found.unwrap_or_else(|| panic!("the census has no `{module}::{name}`"));
        assert!(
            !keys.is_empty(),
            "`{module}::{name}` has no keys — the lexicon feeds it nothing"
        );
    }
    let names: BTreeSet<(&str, &str)> = all.iter().map(|(m, n, _)| (*m, *n)).collect();
    assert_eq!(names.len(), all.len(), "a module registers one table twice");
}

#[test]
fn every_word_table_is_sorted_and_duplicate_free() {
    let mut broken = Vec::new();
    for (module, name, keys) in tables() {
        if let Some(pair) = keys.windows(2).find(|pair| pair[0] >= pair[1]) {
            broken.push(format!(
                "{module}::{name}: {:?} before {:?}",
                pair[0], pair[1]
            ));
        }
    }
    assert!(
        broken.is_empty(),
        "word tables `binary_search` cannot search:\n{}",
        broken.join("\n")
    );
}

/// A key the lexicon writes twice into one table with two different answers: `(table, key, kept,
/// dropped)`. The table answers the first, as the `match` it replaced did.
///
/// `speaker` is two nouns in the lexicon — the device (`Physical`) and the one who speaks (`Human`,
/// derived from `speak`) — and one key of `lookup_sort`; which sort it has is the lexicon's to
/// decide.
const KNOWN_CONFLICTS: &[(&str, &str, &str, &str)] = &[(
    "lookup_sort",
    "speaker",
    "Some(crate::lexicon::Sort::Physical)",
    "Some(crate::lexicon::Sort::Human)",
)];

#[test]
fn a_key_written_twice_with_two_answers_keeps_the_first_and_is_known() {
    let conflicts: Vec<(&str, &str, &str, &str)> = [
        lexicon::WORD_TABLE_CONFLICTS,
        ontology::WORD_TABLE_CONFLICTS,
        semantics::WORD_TABLE_CONFLICTS,
    ]
    .concat();
    assert_eq!(
        conflicts, KNOWN_CONFLICTS,
        "the keys written twice with two answers changed — each new one is a lexicon entry to settle"
    );
    assert_eq!(
        lexicon::lookup_sort("speaker"),
        Some(lexicon::Sort::Physical),
        "the first answer is the one kept"
    );
    assert_eq!(
        lexicon::lookup_sort("Speaker"),
        Some(lexicon::Sort::Physical)
    );
}

#[test]
fn every_table_answers_every_word_as_its_reference_does() {
    let words = words();
    assert!(
        words.len() > 10_000,
        "the differential asks only {} words — the word list is broken",
        words.len()
    );
    let mut differ = Vec::new();
    for word in &words {
        let tables = differences(word);
        if !tables.is_empty() {
            differ.push(format!("{word:?}: {tables:?}"));
        }
    }
    assert!(
        differ.is_empty(),
        "{} words answer differently from their reference; the first:\n{}",
        differ.len(),
        differ
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn the_phrasal_verb_table_answers_every_pair_as_its_reference_does() {
    let phrasal = tables()
        .into_iter()
        .find(|(_, name, _)| *name == "lookup_phrasal_verb")
        .map(|(_, _, keys)| keys)
        .expect("the phrasal verb table is registered");
    let mut pairs: BTreeSet<(String, String)> = BTreeSet::new();
    for key in phrasal {
        for (at, _) in key.match_indices('_') {
            let (verb, particle) = (&key[..at], &key[at + 1..]);
            for (v, p) in variants(verb).into_iter().zip(variants(particle)) {
                pairs.insert((v, p));
            }
            pairs.insert((verb.to_uppercase(), particle.to_string()));
            pairs.insert((verb.to_string(), particle.to_uppercase()));
            pairs.insert((key.to_string(), String::new()));
            pairs.insert((String::new(), key.to_string()));
        }
    }
    let json: serde_json::Value =
        serde_json::from_str(logicaffeine_lexicon::LEXICON_JSON).expect("the lexicon parses");
    let particles: Vec<String> = json["particles"]
        .as_array()
        .expect("the lexicon lists its particles")
        .iter()
        .filter_map(|p| p.as_str().map(str::to_string))
        .collect();
    assert!(!particles.is_empty(), "the lexicon lists no particles");
    for s in lexicon_strings() {
        for particle in &particles {
            pairs.insert((s.clone(), particle.clone()));
        }
    }
    for s in STRANGERS {
        pairs.insert((s.to_string(), "up".to_string()));
        pairs.insert(("give".to_string(), s.to_string()));
    }
    assert!(
        pairs.len() > 10_000,
        "the differential asks only {} pairs — the pair list is broken",
        pairs.len()
    );
    let mut differ = Vec::new();
    for (verb, particle) in &pairs {
        let tables = pair_differences(verb, particle);
        if !tables.is_empty() {
            differ.push(format!("({verb:?}, {particle:?}): {tables:?}"));
        }
    }
    assert!(
        differ.is_empty(),
        "{} pairs answer differently from their reference; the first:\n{}",
        differ.len(),
        differ
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The case fold's edges, answered by the tables themselves: a word folds exactly as
/// `str::to_lowercase` folds it, however it is written.
#[test]
fn a_word_is_found_however_it_is_cased() {
    assert!(
        lexicon::is_common_noun("dog")
            && lexicon::is_common_noun("DOG")
            && lexicon::is_common_noun("Dog")
    );
    assert_eq!(
        lexicon::lookup_phrasal_verb("GIVE", "Up"),
        lexicon::lookup_phrasal_verb("give", "up")
    );
    assert!(
        lexicon::lookup_phrasal_verb("give", "up").is_some(),
        "`give up` is a phrasal verb of the lexicon"
    );
    assert_eq!(lexicon::lookup_negative_contraction("isn"), Some("is not"));
    assert_eq!(
        lexicon::lookup_negative_contraction("ISN"),
        None,
        "a contraction stem is matched as written"
    );
    let kelvin = "\u{212A}ILOGRAM";
    assert_eq!(kelvin.to_lowercase(), "kilogram");
    assert_eq!(
        lexicon::lookup_unit_dimension(kelvin),
        lexicon::lookup_unit_dimension("kilogram")
    );
    let longest = tables()
        .into_iter()
        .filter(|(module, _, _)| *module == "lexicon")
        .flat_map(|(_, _, keys)| keys.iter().copied())
        .max_by_key(|k| k.len())
        .expect("the lexicon has keys");
    let longer = format!("{longest}s");
    assert!(differences(&longer).is_empty() && differences(&longest.to_uppercase()).is_empty());
}
