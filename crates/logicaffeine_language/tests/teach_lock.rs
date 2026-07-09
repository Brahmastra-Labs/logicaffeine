//! Ratchet locks for the teaching brain (`logicaffeine_language::teach`).
//!
//! Every lesson is complete BY THE TYPE (the struct has no optional teaching
//! fields), and these locks pin the contract the type cannot express:
//!
//! - **Completeness** — `what` is one plain sentence (clear and easy);
//!   `example` is real LOGOS that lexes; `question_or_tip` genuinely guides
//!   (a socratic `?` or an explicit `Tip:`).
//! - **Parity** — everything reachable through `doc_for` / `doc_for_block` /
//!   `doc_for_primitive` is in `ALL_DOCS`, and everything in `ALL_DOCS` is
//!   reachable — no orphaned lessons, no unlisted lookups.
//! - **Anchors** — every `guide_anchor` resolves to a real LOGOS_QUICKGUIDE.md
//!   heading under GitHub's slug rules, both directions checked against the
//!   live guide text.

use logicaffeine_base::Interner;
use logicaffeine_language::lexer::Lexer;
use logicaffeine_language::teach::{
    doc_for, doc_for_block, doc_for_header_at, doc_for_primitive, docs_for_word,
    extract_literate_docs, module_doc, ALL_DOCS,
};
use logicaffeine_language::token::{BlockType, TokenType};

/// The statement keywords with lessons. Append-only: extending the teach
/// table means extending this list in the same change.
fn documented_keywords() -> Vec<TokenType> {
    vec![
        TokenType::Let,
        TokenType::Set,
        TokenType::Return,
        TokenType::If,
        TokenType::While,
        TokenType::Repeat,
        TokenType::Show,
        TokenType::Give,
        TokenType::Push,
        TokenType::Inspect,
        TokenType::Call,
        TokenType::New,
        TokenType::Escape,
        TokenType::Check,
        TokenType::Pop,
        TokenType::Add,
        TokenType::Remove,
        TokenType::Break,
        TokenType::Assert,
        TokenType::Trust,
        TokenType::Increase,
        TokenType::Decrease,
        TokenType::Spawn,
        TokenType::Merge,
    ]
}

/// Every `BlockType`, wildcard-free — a new block type fails to compile here
/// until it appears in this list (and therefore gets a lesson: `doc_for_block`
/// is total).
fn all_block_types(interner: &mut Interner) -> Vec<BlockType> {
    let sym = interner.intern("sample");
    let all = vec![
        BlockType::SuspectedTypo { found: sym, suggestion: sym },
        BlockType::Theorem,
        BlockType::Main,
        BlockType::Definition,
        BlockType::Define,
        BlockType::Axiom,
        BlockType::Theory,
        BlockType::Proof,
        BlockType::Example,
        BlockType::Logic,
        BlockType::Note,
        BlockType::Function,
        BlockType::TypeDef,
        BlockType::Policy,
        BlockType::Requires,
        BlockType::Hardware,
        BlockType::Property,
        BlockType::No,
        BlockType::Tier,
    ];
    for block in &all {
        block_type_guard(block);
    }
    all
}

fn block_type_guard(block: &BlockType) {
    match block {
        BlockType::SuspectedTypo { .. }
        | BlockType::Theorem
        | BlockType::Main
        | BlockType::Definition
        | BlockType::Define
        | BlockType::Axiom
        | BlockType::Theory
        | BlockType::Proof
        | BlockType::Example
        | BlockType::Logic
        | BlockType::Note
        | BlockType::Function
        | BlockType::TypeDef
        | BlockType::Policy
        | BlockType::Requires
        | BlockType::Hardware
        | BlockType::Property
        | BlockType::No
        | BlockType::Tier => {}
    }
}

/// The type names with lessons (primitives + built-in generics).
const DOCUMENTED_PRIMITIVES: &[&str] = &[
    "Int", "Nat", "Text", "Bool", "Float", "Unit", "Char", "Byte",
    "List", "Seq", "Map", "Set", "Option", "Result",
];

#[test]
fn every_teach_doc_is_complete_clear_and_socratic() {
    assert!(!ALL_DOCS.is_empty(), "the teach table must not be empty");
    for doc in ALL_DOCS {
        assert!(!doc.name.is_empty(), "a lesson needs a name");

        // `what` — ONE plain sentence, clear and easy.
        assert!(
            doc.what.ends_with('.'),
            "{}: `what` must be a sentence ending in '.': {:?}",
            doc.name,
            doc.what
        );
        assert!(
            !doc.what.contains('\n'),
            "{}: `what` must be a single line: {:?}",
            doc.name,
            doc.what
        );
        assert!(
            doc.what.chars().count() <= 90,
            "{}: `what` must stay under 90 chars (clear and easy), got {}: {:?}",
            doc.name,
            doc.what.chars().count(),
            doc.what
        );
        let interior = &doc.what[..doc.what.len() - 1];
        assert!(
            !interior.contains(". "),
            "{}: `what` must be ONE sentence — move the rest into the question or tip: {:?}",
            doc.name,
            doc.what
        );

        // `example` — real LOGOS: non-empty, and it must lex without panicking
        // into at least one token (the same bar the quickguide ratchet sets).
        assert!(!doc.example.is_empty(), "{}: a lesson needs an example", doc.name);
        let example = doc.example.to_string();
        let name = doc.name;
        let token_count = std::panic::catch_unwind(move || {
            let mut interner = Interner::new();
            let mut lexer = Lexer::new(&example, &mut interner);
            lexer.tokenize().len()
        })
        .unwrap_or_else(|_| panic!("{name}: the example PANICS the lexer"));
        assert!(
            token_count > 0,
            "{}: the example must lex to at least one token",
            doc.name
        );

        // `question_or_tip` — socratic: a guiding question or an explicit tip.
        assert!(
            doc.question_or_tip.contains('?') || doc.question_or_tip.starts_with("Tip:"),
            "{}: `question_or_tip` must ask a guiding question or start with 'Tip:': {:?}",
            doc.name,
            doc.question_or_tip
        );
    }
}

#[test]
fn lookups_and_all_docs_agree_both_directions() {
    let mut interner = Interner::new();
    let mut reachable: Vec<*const _> = Vec::new();

    for kind in documented_keywords() {
        let doc = doc_for(&kind).unwrap_or_else(|| {
            panic!("{kind:?} is in documented_keywords but doc_for returns None")
        });
        reachable.push(doc as *const _);
    }
    for block in all_block_types(&mut interner) {
        reachable.push(doc_for_block(&block) as *const _);
    }
    for name in DOCUMENTED_PRIMITIVES {
        let doc = doc_for_primitive(name).unwrap_or_else(|| {
            panic!("{name} is in DOCUMENTED_PRIMITIVES but doc_for_primitive returns None")
        });
        reachable.push(doc as *const _);
    }

    for doc in ALL_DOCS {
        assert!(
            reachable.iter().any(|p| std::ptr::eq(*p, *doc as *const _)),
            "{}: in ALL_DOCS but unreachable from doc_for/doc_for_block/doc_for_primitive — \
             extend documented_keywords/all_block_types/DOCUMENTED_PRIMITIVES or wire the lookup",
            doc.name
        );
    }
    for ptr in &reachable {
        assert!(
            ALL_DOCS.iter().any(|d| std::ptr::eq(*d as *const _, *ptr)),
            "a lookup returns a lesson that is missing from ALL_DOCS"
        );
    }
}

#[test]
fn word_lookup_finds_every_lesson_case_insensitively() {
    for doc in ALL_DOCS {
        let hits = docs_for_word(&doc.name.to_lowercase());
        assert!(
            hits.iter().any(|d| std::ptr::eq(*d as *const _, *doc as *const _)),
            "{}: docs_for_word must find the lesson case-insensitively",
            doc.name
        );
    }
    assert!(docs_for_word("no-such-construct").is_empty());
}

// ---------------------------------------------------------------------------
// Literate-doc extraction (the prose → hover pipeline)
// ---------------------------------------------------------------------------

const LITERATE_MODULE: &str = "\
# File

Standard library for file I/O operations.

## Note
Reads a whole file into Text.

## To native read (path: Text) -> Result of Text and Text

## To native write (path: Text) and (content: Text) -> Result of Unit and Text

## Note
A point on the plane.

## A Point has:
    An x: Int.
    A y: Int.
";

#[test]
fn module_doc_is_the_prose_before_the_first_section() {
    assert_eq!(
        module_doc(LITERATE_MODULE).as_deref(),
        Some("Standard library for file I/O operations."),
        "the title line is dropped; the prose paragraph is the module doc"
    );
    assert_eq!(module_doc("## Main\nShow 1.\n"), None, "no prose, no doc");
}

#[test]
fn a_note_block_directly_above_a_header_documents_it() {
    let read_header = LITERATE_MODULE.find("## To native read").unwrap();
    assert_eq!(
        doc_for_header_at(LITERATE_MODULE, read_header).as_deref(),
        Some("Reads a whole file into Text."),
    );

    // `write` has no Note of its own — the `read` Note must NOT leak down.
    let write_header = LITERATE_MODULE.find("## To native write").unwrap();
    assert_eq!(doc_for_header_at(LITERATE_MODULE, write_header), None);

    // Type headers are documented the same way.
    let point_header = LITERATE_MODULE.find("## A Point").unwrap();
    assert_eq!(
        doc_for_header_at(LITERATE_MODULE, point_header).as_deref(),
        Some("A point on the plane."),
    );
}

#[test]
fn definition_block_types_inherit_the_blocks_note() {
    // net/io/crdt declare their types INSIDE a `## Definition` body; the
    // Note above the block documents the type(s) it declares.
    let module = "\
# Net

## Note
A network message: a sender id and a payload.

## Definition

A Message has:
    a sender, which is Int.
    a payload, which is Text.
";
    let docs = extract_literate_docs(module);
    assert_eq!(docs.len(), 1, "the Definition body yields its type");
    assert_eq!(docs[0].name, "Message");
    assert_eq!(docs[0].signature, "A Message has:");
    assert_eq!(docs[0].doc.as_deref(), Some("A network message: a sender id and a payload."));
}

#[test]
fn extract_literate_docs_walks_every_definition() {
    let docs = extract_literate_docs(LITERATE_MODULE);
    let names: Vec<&str> = docs.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["read", "write", "Point"], "every ## To / ## A definition appears");

    let read = &docs[0];
    assert_eq!(read.signature, "## To native read (path: Text) -> Result of Text and Text");
    assert_eq!(read.doc.as_deref(), Some("Reads a whole file into Text."));
    assert_eq!(docs[1].doc, None, "write is undocumented in this fixture");
    assert_eq!(docs[2].doc.as_deref(), Some("A point on the plane."));
}

/// GitHub's heading→anchor slug rules, as applied to the quickguide.
fn github_slug(heading: &str) -> String {
    heading
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-')
        .collect::<String>()
        .replace(' ', "-")
}

#[test]
fn guide_anchors_resolve_to_real_quickguide_headings() {
    let guide = include_str!("../../../LOGOS_QUICKGUIDE.md");
    let slugs: Vec<String> = guide
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start_matches('#');
            (trimmed.len() < line.len()).then(|| github_slug(trimmed.trim()))
        })
        .collect();

    for doc in ALL_DOCS {
        if let Some(anchor) = doc.guide_anchor {
            assert!(
                !anchor.starts_with('#'),
                "{}: anchors are bare slugs, no '#' prefix: {anchor:?}",
                doc.name
            );
            assert!(
                slugs.iter().any(|s| s == anchor),
                "{}: guide_anchor {anchor:?} matches no LOGOS_QUICKGUIDE.md heading; \
                 real slugs include e.g. {:?}",
                doc.name,
                &slugs[..slugs.len().min(8)]
            );
        }
    }
}
