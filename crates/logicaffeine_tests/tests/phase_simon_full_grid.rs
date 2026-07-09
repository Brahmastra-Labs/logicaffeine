#![cfg(feature = "verification")]
//! The WHOLE Simon grid, solved from the JSON clues — not six spot-checks but all
//! sixteen cells, each asked as a question and required to resolve to EXACTLY ONE
//! trip. A logic-grid puzzle is "solved" only when every (category-value → trip)
//! cell is forced; a cell that returns zero trips means the clues under-determine
//! it, and a cell that returns two means the bijection leaked. This is the
//! completeness gate the six-cell `phase_simon_json_solve` does not assert.
//!
//! Clues are read VERBATIM from the puzzle JSON (trip / vacation / holiday heads,
//! contractions and all). Only the bijection scaffold (domain declarations +
//! closure + exactly-one) and the year anchor are synthesized structurally from
//! the `categories` block — no clue is hand-normalized.

use logicaffeine_compile::answer_question;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Puzzle {
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

/// The relation a category contributes, inferred from how its items appear in the
/// clues. People → "with"; gerund activities → a bare predicate; years/states → "in".
enum Relation {
    In,
    With,
    Predicate,
}

fn infer_relation(items: &[String], clues: &[String]) -> Relation {
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

fn phrase(item: &str, rel: &Relation) -> String {
    match rel {
        Relation::In => format!("in {item}"),
        Relation::With => format!("with {item}"),
        Relation::Predicate => item.to_string(),
    }
}

fn bijection(items: &[String], rel: &Relation) -> String {
    let disjuncts = items.iter().map(|i| phrase(i, rel)).collect::<Vec<_>>().join(" or ");
    let mut s = format!("Given: Every trip is {disjuncts}.\n");
    for i in items {
        s.push_str(&format!("Given: Exactly one trip is {}.\n", phrase(i, rel)));
    }
    s
}

fn declaration(items: &[String], noun: &str) -> String {
    let count = match items.len() {
        4 => "four",
        3 => "three",
        2 => "two",
        _ => "several",
    };
    let (last, head) = items.split_last().unwrap();
    let list = format!("{}, and {}", head.join(", "), last);
    format!("Given: {list} are {count} different {noun}.\n")
}

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
    if let Some(years) = year_items {
        let mut sorted = years.clone();
        sorted.sort();
        for (name, y) in ["Alpha", "Beta", "Gamma", "Delta"].iter().zip(sorted.iter()) {
            doc.push_str(&format!("Given: {name} is in {y}.\n"));
        }
    }
    for clue in &p.clues {
        doc.push_str(&format!("Given: {clue}\n"));
    }
    doc
}

/// THE full unique solution, expressed cell-by-cell. With the year anchor
/// (Alpha=2001 … Delta=2004), the puzzle's unique solution
///   2001/Lillie/cycling/Kentucky   2002/Neal/hunting/Florida
///   2003/Yvonne/kayaking/Maine     2004/Bill/skydiving/Connecticut
/// becomes these sixteen forced cells.
fn expected_grid() -> Vec<(&'static str, &'static str)> {
    vec![
        // years (the anchor) — "in YEAR" → trip
        ("in 2001", "Alpha"),
        ("in 2002", "Beta"),
        ("in 2003", "Gamma"),
        ("in 2004", "Delta"),
        // states
        ("in Kentucky", "Alpha"),
        ("in Florida", "Beta"),
        ("in Maine", "Gamma"),
        ("in Connecticut", "Delta"),
        // friends
        ("with Lillie", "Alpha"),
        ("with Neal", "Beta"),
        ("with Yvonne", "Gamma"),
        ("with Bill", "Delta"),
        // activities
        ("cycling", "Alpha"),
        ("hunting", "Beta"),
        ("kayaking", "Gamma"),
        ("skydiving", "Delta"),
    ]
}

#[test]
fn every_cell_of_the_simon_grid_is_forced() {
    let p = load_simon();
    assert_eq!(p.clues.len(), 6, "Simon has six clues");
    let doc = build_theorem(&p);
    let ask = |phr: &str| -> Vec<String> {
        answer_question(&format!("{doc}Prove: Who is {phr}?\nProof: Auto.\n"))
            .expect("every Simon cell should be answerable")
    };
    for (phr, trip) in expected_grid() {
        let got = ask(phr);
        assert_eq!(
            got,
            vec![trip.to_string()],
            "cell `Who is {phr}?` must resolve to exactly {trip}; got: {got:?}"
        );
    }
}
