#![cfg(feature = "verification")]
//! Solve the FULL Simon puzzle with its CLUES READ STRAIGHT FROM THE JSON — no hand
//! normalization. The six clues vary the row noun freely (trip / vacation / holiday)
//! and use contractions ("wasn't"); the solve handles them through occasion
//! soft-typing (synonym heads range over the one row domain) and the existing
//! label→relation convergence. Only the bijection scaffold (domain declarations +
//! closure + exactly-one) is synthesized structurally from the `categories` block;
//! every CLUE is the verbatim JSON string.

use logicaffeine_compile::answer_question;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Puzzle {
    story: String,
    clues: Vec<String>,
    categories: BTreeMap<String, Category>,
}

#[derive(Deserialize)]
struct Category {
    #[allow(dead_code)]
    group: String,
    items: Vec<String>,
}

fn load_simon() -> Puzzle {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../puzzles/3cf932ef3e458bf4fcba3081d831f5ae.json"
    );
    let raw = std::fs::read_to_string(path).expect("read Simon puzzle JSON");
    serde_json::from_str(&raw).expect("parse Simon puzzle JSON")
}

/// The relation a category contributes, inferred from its items (the JSON `group`
/// label is empty). People → "with"; gerund activities → a bare predicate; anything
/// else (years, states) → "in". General, not puzzle-specific.
enum Relation {
    In,
    With,
    Predicate,
}

fn infer_relation(items: &[String], clues: &[String]) -> Relation {
    // The clues reference each item with its natural preposition ("with Neal", "in
    // Kentucky"); read the relation off that usage. People → "with"; gerund
    // activities (referenced bare / as labels) → a predicate; everything else → "in".
    let appears_with = |item: &str| {
        let needle = format!("with {}", item.to_lowercase());
        clues.iter().any(|c| c.to_lowercase().contains(&needle))
    };
    if items.iter().any(|i| appears_with(i)) {
        Relation::With
    } else if items.iter().all(|i| i.ends_with("ing")) {
        Relation::Predicate
    } else {
        Relation::In
    }
}

/// A 4-way domain closure + an exactly-one per value — the general grid bijection,
/// no puzzle knowledge.
fn bijection(items: &[String], rel: &Relation) -> String {
    let phrase = |item: &str| match rel {
        Relation::In => format!("in {item}"),
        Relation::With => format!("with {item}"),
        Relation::Predicate => item.to_string(),
    };
    let disjuncts = items.iter().map(|i| phrase(i)).collect::<Vec<_>>().join(" or ");
    let mut s = format!("Given: Every trip is {disjuncts}.\n");
    for i in items {
        s.push_str(&format!("Given: Exactly one trip is {}.\n", phrase(i)));
    }
    s
}

/// The category's declaration ("2001, 2002, 2003 and 2004 are four different years")
/// so the labels in the clues converge to the right relation.
fn declaration(items: &[String], noun: &str) -> String {
    let n = items.len();
    let count = match n {
        4 => "four",
        3 => "three",
        2 => "two",
        _ => "several",
    };
    let list = if items.len() >= 2 {
        let (last, head) = items.split_last().unwrap();
        format!("{}, and {}", head.join(", "), last)
    } else {
        items.join("")
    };
    format!("Given: {list} are {count} different {noun}.\n")
}

/// Category nouns for the declarations. The JSON `group` is empty, so name them by
/// the inferred relation + item shape (years/states both "in"; people; activities).
fn category_noun(items: &[String], rel: &Relation) -> &'static str {
    match rel {
        Relation::With => "friends",
        Relation::Predicate => "activities",
        Relation::In => {
            if items.iter().all(|i| i.chars().all(|c| c.is_ascii_digit())) {
                "years"
            } else {
                "states"
            }
        }
    }
}

fn build_theorem(p: &Puzzle) -> String {
    let mut doc = String::from(
        "## Theorem: Simon\nGiven: Alpha, Beta, Gamma, and Delta are four different trips.\n",
    );
    // Declarations + bijection per category, structurally from `categories`.
    let mut year_items: Option<Vec<String>> = None;
    for cat in p.categories.values() {
        let rel = infer_relation(&cat.items, &p.clues);
        doc.push_str(&declaration(&cat.items, category_noun(&cat.items, &rel)));
        if matches!(rel, Relation::In)
            && cat.items.iter().all(|i| i.chars().all(|c| c.is_ascii_digit()))
        {
            year_items = Some(cat.items.clone());
        }
    }
    for cat in p.categories.values() {
        let rel = infer_relation(&cat.items, &p.clues);
        doc.push_str(&bijection(&cat.items, &rel));
    }
    // Anchor the four trips to the four years to break the cell-name symmetry.
    if let Some(years) = year_items {
        let mut sorted = years.clone();
        sorted.sort();
        for (name, y) in ["Alpha", "Beta", "Gamma", "Delta"].iter().zip(sorted.iter()) {
            doc.push_str(&format!("Given: {name} is in {y}.\n"));
        }
    }
    // The SIX real clues — verbatim from the JSON.
    for clue in &p.clues {
        doc.push_str(&format!("Given: {clue}\n"));
    }
    let _ = &p.story;
    doc
}

#[test]
fn full_simon_solves_from_json_clues() {
    let p = load_simon();
    assert_eq!(p.clues.len(), 6, "Simon has six clues");
    let doc = build_theorem(&p);
    let ask = |q: &str| -> Vec<String> {
        answer_question(&format!("{doc}Prove: {q}\nProof: Auto.\n"))
            .expect("the JSON-clue Simon puzzle should be answerable")
    };
    // Unique solution: 2001/Kentucky, 2002/Florida, 2003/Maine, 2004/Connecticut,
    // with the year anchor → Alpha/Kentucky, Beta/Florida, Gamma/Maine, Delta/Connecticut.
    assert_eq!(ask("Who is in Florida?"), vec!["Beta".to_string()], "2002 ↔ Florida");
    assert_eq!(ask("Who is in Kentucky?"), vec!["Alpha".to_string()], "2001 ↔ Kentucky");
    assert_eq!(ask("Who is in Maine?"), vec!["Gamma".to_string()], "2003 ↔ Maine");
    assert_eq!(ask("Who is in Connecticut?"), vec!["Delta".to_string()], "2004 ↔ Connecticut");
    assert!(ask("Who is with Neal?").contains(&"Beta".to_string()), "2002 ↔ Neal");
    assert!(ask("Who is hunting?").contains(&"Beta".to_string()), "2002 ↔ hunting");
}
