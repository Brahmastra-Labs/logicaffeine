//! ★ LOCK — no field of a monomorphic program names a generic. ★
//!
//! A backend with no generics of its own compiles the program monomorphization makes of the one written: every applied
//! generic (`Wrap of Int`, and for C, which has no `Option` either, `Option of Int`) becomes one concrete type
//! (`Wrap__Int`, `Option__Int`). A field is where one can stay behind. A record declared without type parameters kept
//! its `Option of Int` field as written while every construction and `Inspect` of it named `Option__Int`, and the C
//! emitter, knowing no `Option`, laid the field out as an `int64_t`: no record holding an `Option` built in C, nor the
//! JSON reader and writer every record has. This lock walks the registry each program monomorphizes to and finds no
//! field, at any depth, that names an applied generic.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test monomorphic_fields_lock`

use logicaffeine_compile::analysis::{FieldType, TypeDef, TypeRegistry};
use logicaffeine_compile::intern::{Interner, Symbol};
use logicaffeine_compile::monomorphize::Options;
use logicaffeine_compile::ui_bridge::with_monomorphic_program;

/// Programs whose records and enums hold applied generics in their fields, each shape once.
const PROGRAMS: &[(&str, &str)] = &[
    (
        "an Option field of a record",
        "## A Box has:\n    a n, which is Option of Int.\n\n## Main\nLet b be a new Box with n some(3).\nInspect b's n:\n    When Some (v): Show v.\n    When None: Show \"none\".\n",
    ),
    (
        "an Option field of a variant",
        "## A Shape is either:\n    A Circle with a radius, which is Option of Int.\n    A Dot.\n\n## Main\nLet s be a new Circle with radius some(2).\nInspect s:\n    When Circle (radius): Show \"circle\".\n    When Dot: Show \"dot\".\n",
    ),
    (
        "an Option of a record",
        "## A Point has:\n    An x: Int.\n\n## A Marker has:\n    a at, which is Option of Point.\n\n## Main\nLet m be a new Marker with at some(a new Point with x 3).\nInspect m's at:\n    When Some (p): Show p's x.\n    When None: Show \"nowhere\".\n",
    ),
    (
        "a sequence of options",
        "## A Shelf has:\n    a slots, which is Seq of (Option of Int).\n\n## Main\nLet mutable xs be a new Seq of (Option of Int).\nPush some(4) to xs.\nLet s be a new Shelf with slots xs.\nShow length of s's slots.\n",
    ),
    (
        "a Maybe and a Result",
        "## A Job has:\n    a held, which is Maybe of Text.\n    a outcome, which is Result of Int and Text.\n\n## Main\nLet j be a new Job with outcome (a new Ok with value 10) and held some(\"x\").\nInspect j's outcome:\n    When Ok (v): Show v.\n    When Err (e): Show e.\n",
    ),
    (
        "a generic record's own Option field",
        "## A Slot of [T] has:\n    a value, which is T.\n    a spare, which is Option of Int.\n\n## Main\nLet s be a new Slot of Text with value \"a\" and spare some(7).\nShow s's value.\n",
    ),
    (
        "an applied generic record in a plain record",
        "## A Wrap of [T] has:\n    a inner, which is T.\n\n## A Holder has:\n    a w, which is Wrap of Int.\n\n## Main\nLet h be a new Holder with w (a new Wrap of Int with inner 41).\nShow h's w's inner.\n",
    ),
];

/// A name the monomorphic program must not apply: a generic record or enum it registers, or — when `Option` is
/// lowered — the built-in `Option` (`Maybe` too, unless the program declares its own).
fn names_a_generic(base: Symbol, registry: &TypeRegistry, interner: &Interner, option_lowered: bool) -> bool {
    let generic = matches!(
        registry.get(base),
        Some(TypeDef::Struct { generics, .. } | TypeDef::Enum { generics, .. }) if !generics.is_empty()
    );
    let declared = matches!(registry.get(base), Some(TypeDef::Struct { .. } | TypeDef::Enum { .. }));
    let option = match interner.resolve(base) {
        "Option" => true,
        "Maybe" => !declared,
        _ => false,
    };
    generic || (option_lowered && option)
}

/// Every place in `ty` that applies a name [`names_a_generic`] refuses, written as the base it applies.
fn applied_generics(ty: &FieldType, registry: &TypeRegistry, interner: &Interner, lowered: bool, out: &mut Vec<String>) {
    match ty {
        FieldType::Primitive(_) | FieldType::Named(_) | FieldType::TypeParam(_) => {}
        FieldType::Generic { base, params } => {
            if names_a_generic(*base, registry, interner, lowered) {
                out.push(interner.resolve(*base).to_string());
            }
            for p in params {
                applied_generics(p, registry, interner, lowered, out);
            }
        }
        FieldType::Tuple(items) => {
            for t in items {
                applied_generics(t, registry, interner, lowered, out);
            }
        }
        FieldType::Record(fields) => {
            for (_, t) in fields {
                applied_generics(t, registry, interner, lowered, out);
            }
        }
    }
}

/// `type.field: Base` for every field of a concrete record or enum in `src`'s monomorphic registry that applies a
/// generic; a generic record's or enum's own declaration is a template, and its type parameters are its to name.
fn generic_fields(src: &str, monomorphize: Options, judged_lowered: bool) -> Vec<String> {
    with_monomorphic_program(src, monomorphize, |parsed, interner| {
        let (_, registry, _) = parsed.unwrap_or_else(|e| panic!("the program monomorphizes:\n{src}\n{e}"));
        let mut found = Vec::new();
        for (name, def) in registry.iter_types() {
            let fields: Vec<_> = match def {
                TypeDef::Struct { fields, generics, .. } if generics.is_empty() => fields.iter().collect(),
                TypeDef::Enum { variants, generics, .. } if generics.is_empty() => {
                    variants.iter().flat_map(|v| v.fields.iter()).collect()
                }
                _ => continue,
            };
            for f in fields {
                let mut bases = Vec::new();
                applied_generics(&f.ty, registry, interner, judged_lowered, &mut bases);
                for base in bases {
                    found.push(format!("{}.{}: {base}", interner.resolve(*name), interner.resolve(f.name)));
                }
            }
        }
        found.sort();
        found
    })
}

/// Every shape whose monomorphic program, made with `monomorphize`, keeps a field naming a generic, with those fields.
fn shapes_left_generic(monomorphize: Options, judged_lowered: bool) -> Vec<String> {
    PROGRAMS
        .iter()
        .filter_map(|(shape, src)| {
            let left = generic_fields(src, monomorphize, judged_lowered);
            (!left.is_empty()).then(|| format!("{shape}: {left:?}"))
        })
        .collect()
}

/// With `Option` lowered (the C backend's program), no field of any record or enum names an applied generic.
#[test]
fn a_program_with_option_lowered_has_no_generic_field() {
    let left = shapes_left_generic(Options { lower_option: true }, true);
    assert!(left.is_empty(), "fields still naming a generic:\n{}", left.join("\n"));
}

/// With `Option` kept (the WebAssembly backend's program), no field names an applied generic record or enum.
#[test]
fn a_program_with_option_kept_has_no_generic_record_field() {
    let left = shapes_left_generic(Options { lower_option: false }, false);
    assert!(left.is_empty(), "fields still naming a generic:\n{}", left.join("\n"));
}

/// The lock is a check that can fail: where `Option` is kept, the walk that judges it lowered finds the field that
/// holds one.
#[test]
fn the_walk_finds_a_field_that_names_a_generic() {
    let (_, src) = PROGRAMS[0];
    assert_eq!(generic_fields(src, Options { lower_option: false }, true), ["Box.n: Option"]);
}
