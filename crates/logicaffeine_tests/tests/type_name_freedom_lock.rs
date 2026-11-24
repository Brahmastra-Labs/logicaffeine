//! ★ LOCK — a keyword can name a TYPE in the type-discovery pass, the way it can name a variable or a field. ★
//!
//! LOGOS keywords are contextual: `check` begins `Check that …`, yet `Let check be 3.` is fine and so
//! is a field called `check`, because in a NAME position the author wins ("identifier freedom",
//! `Parser::token_is_word_identifier`). The type-discovery pass is the first reader of every type
//! name, and the compiled tiers learn a program's types from it alone: a record it fails to register
//! is a record the bytecode compiler and the WebAssembly backend never hear of, so a parameter of
//! that type has no fields and a page drawing one refuses to reach the browser. The tree-walker
//! builds records dynamically and never notices.
//!
//! The rule is the POSITION, not a list of words: after `## A` (or `## An`), where a field's type
//! is written, and where a variant is declared, the word written is the name.
//! `type_name_keyword_lock` holds the same freedom through the parser and the tree-walker, and
//! `type_name_shadowing_lock` holds it on WebAssembly.

use logicaffeine_language::analysis::{DiscoveryPass, FieldType, TypeDef};
use logicaffeine_language::{mwe, Interner, Lexer, Symbol, TypeRegistry};

fn registry(source: &str, interner: &mut Interner) -> TypeRegistry {
    let tokens = {
        let mut lexer = Lexer::new(source, interner);
        let tokens = lexer.tokenize();
        let trie = mwe::build_mwe_trie();
        mwe::apply_mwe_pipeline(tokens, &trie, interner)
    };
    DiscoveryPass::new(&tokens, interner).run()
}

/// Keywords a program could reasonably name a type after. Each leads a statement somewhere in the
/// language, and each is an ordinary noun.
const KEYWORD_NAMES: &[&str] = &[
    "Check", "Note", "Record", "Task", "Pipe", "Given", "Expect", "Trust", "Inspect", "Stop",
];

#[test]
fn a_record_named_by_a_keyword_is_registered_with_its_fields() {
    for name in KEYWORD_NAMES {
        let src = format!("## A {name} has:\n    A prompt: Text.\n    A weight: Int.\n");
        let mut interner = Interner::new();
        let reg = registry(&src, &mut interner);
        let sym = interner.intern(name);
        match reg.get(sym) {
            Some(TypeDef::Struct { fields, .. }) => {
                let names: Vec<&str> = fields.iter().map(|f| interner.resolve(f.name)).collect();
                assert_eq!(names, vec!["prompt", "weight"], "`{name}` keeps both fields");
            }
            other => panic!(
                "`## A {name} has:` must register a record, got {:?}",
                other.map(|_| "a non-record")
            ),
        }
    }
}

#[test]
fn a_field_typed_by_a_keyword_named_record_keeps_its_type() {
    let src = "## A Check has:\n    A prompt: Text.\n\n\
        ## A Lesson has:\n    A predict: Option of Check.\n    A checks: Seq of Check.\n    A first: Check.\n";
    let mut interner = Interner::new();
    let reg = registry(src, &mut interner);
    let lesson = interner.intern("Lesson");
    let check = interner.intern("Check");
    let Some(TypeDef::Struct { fields, .. }) = reg.get(lesson) else {
        panic!("Lesson is a record");
    };
    assert_eq!(fields.len(), 3, "every field survives");
    fn named(ft: &FieldType) -> Option<Symbol> {
        match ft {
            FieldType::Generic { params, .. } => params.first().and_then(named),
            FieldType::Named(s) | FieldType::Primitive(s) => Some(*s),
            _ => None,
        }
    }
    for f in fields {
        assert_eq!(
            named(&f.ty),
            Some(check),
            "field `{}` names `Check` as its type",
            interner.resolve(f.name)
        );
    }
}

#[test]
fn a_variant_named_by_a_keyword_is_a_variant() {
    let src = "## A Step is one of:\n    A Check with prompt Text.\n    A Note.\n";
    let mut interner = Interner::new();
    let reg = registry(src, &mut interner);
    let step = interner.intern("Step");
    let Some(TypeDef::Enum { variants, .. }) = reg.get(step) else {
        panic!("Step is a sum type");
    };
    let names: Vec<&str> = variants.iter().map(|v| interner.resolve(v.name)).collect();
    assert_eq!(names, vec!["Check", "Note"]);
    assert_eq!(variants[0].fields.len(), 1, "`Check` keeps its payload");
}

/// A sum type NAMED by a keyword, the other half of the header position.
#[test]
fn a_sum_type_named_by_a_keyword_is_registered_with_its_variants() {
    for name in KEYWORD_NAMES {
        let src = format!("## A {name} is one of:\n    A Plain with text Text.\n    A Blank.\n");
        let mut interner = Interner::new();
        let reg = registry(&src, &mut interner);
        let sym = interner.intern(name);
        let Some(TypeDef::Enum { variants, .. }) = reg.get(sym) else {
            panic!("`## A {name} is one of:` must register a sum type");
        };
        let names: Vec<&str> = variants.iter().map(|v| interner.resolve(v.name)).collect();
        assert_eq!(names, vec!["Plain", "Blank"], "`{name}` keeps both variants");
    }
}

/// A capitalized plural keeps its capital. The lexer reads `Kinds` before `has` as the common noun
/// `kinds` (`Dogs bark.` is about dogs), so the header registered a type nobody could name.
#[test]
fn a_type_named_by_a_capitalized_plural_keeps_its_capital() {
    let src = "## A Kind has:\n    A label: Text.\n\n## A Kinds has:\n    A kinds: Seq of Kind.\n\n\
## A Widgets is one of:\n    A Many with kinds Kinds.\n    A Blank.\n";
    let mut interner = Interner::new();
    let reg = registry(src, &mut interner);
    for name in ["Kind", "Kinds", "Widgets"] {
        let sym = interner.intern(name);
        assert!(reg.get(sym).is_some(), "`{name}` is registered under the name as written");
    }
    let lowered = interner.intern("kinds");
    assert!(
        reg.get(lowered).is_none(),
        "and never under a lowercased common noun"
    );
}
