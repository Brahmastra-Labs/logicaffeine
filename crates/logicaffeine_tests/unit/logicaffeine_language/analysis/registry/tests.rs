use super::*;

#[test]
fn registry_stores_and_retrieves() {
    let mut interner = Interner::new();
    let mut registry = TypeRegistry::new();
    let foo = interner.intern("Foo");
    registry.register(foo, TypeDef::Primitive);
    assert!(registry.is_type(foo));
    assert!(!registry.is_generic(foo));
}

fn record(interner: &mut Interner, field: &str) -> TypeDef {
    TypeDef::Struct {
        fields: vec![FieldDef {
            name: interner.intern(field),
            ty: FieldType::Primitive(interner.intern("Int")),
            is_public: true,
        }],
        generics: Vec::new(),
        is_portable: false,
        is_shared: false,
    }
}

fn enumeration(interner: &mut Interner, variants: &[&str]) -> TypeDef {
    TypeDef::Enum {
        variants: variants
            .iter()
            .map(|v| VariantDef {
                name: interner.intern(v),
                fields: Vec::new(),
            })
            .collect(),
        generics: Vec::new(),
        is_portable: false,
        is_shared: false,
    }
}

/// A bare name is the twin of the ONE namespaced type of its own kind that shares its simple name —
/// the pair the assembler registers for an imported type — and of nothing else.
#[test]
fn a_bare_type_names_its_unique_namespaced_twin_of_its_own_kind() {
    let mut interner = Interner::new();
    let mut registry = TypeRegistry::new();
    let point = interner.intern("Point");
    let geometry_point = interner.intern("Geometry::Point");
    let point_def = record(&mut interner, "x");
    registry.register(point, point_def.clone());
    registry.register(geometry_point, point_def);

    let x = interner.intern("X");
    let a_x = interner.intern("A::X");
    let b_x = interner.intern("B::X");
    for sym in [x, a_x, b_x] {
        let def = record(&mut interner, "v");
        registry.register(sym, def);
    }

    let shape = interner.intern("Shape");
    let c_shape = interner.intern("C::Shape");
    let shape_def = record(&mut interner, "side");
    registry.register(shape, shape_def);
    let c_shape_def = enumeration(&mut interner, &["Round"]);
    registry.register(c_shape, c_shape_def);

    let twins = registry.namespaced_twins(&interner);
    assert_eq!(twins.get(&point), Some(&geometry_point), "a unique twin");
    assert_eq!(
        twins.get(&x),
        None,
        "two namespaced types share `X`: ambiguous, no twin"
    );
    assert_eq!(
        twins.get(&shape),
        None,
        "a record is never the twin of an enum"
    );
    assert_eq!(twins.len(), 1);
    assert_eq!(registry.canonical_type(point, &interner), geometry_point);
    assert_eq!(
        registry.canonical_type(geometry_point, &interner),
        geometry_point
    );
    assert_eq!(registry.canonical_type(x, &interner), x);
}

/// A variant a twin pair declares is not ambiguous — the pair is one enum — while a variant two
/// different enums declare still is.
#[test]
fn a_twin_pair_is_one_variant_candidate() {
    let mut interner = Interner::new();
    let mut registry = TypeRegistry::new();
    let filter = interner.intern("Filter");
    let colors_filter = interner.intern("Colors::Filter");
    let filter_def = enumeration(&mut interner, &["Sepia", "Mono"]);
    registry.register(filter, filter_def.clone());
    registry.register(colors_filter, filter_def);
    let sepia = interner.intern("Sepia");

    let distinct: Vec<Symbol> = registry
        .find_distinct_variant_candidates(sepia, &interner)
        .iter()
        .map(|(e, _)| *e)
        .collect();
    assert_eq!(
        distinct,
        vec![colors_filter],
        "the twin pair counts once, as its namespaced member"
    );
    assert_eq!(
        registry.find_variant_candidates(sepia).len(),
        2,
        "the raw candidates still name both spellings"
    );

    let tint = interner.intern("Tint");
    let tint_def = enumeration(&mut interner, &["Sepia"]);
    registry.register(tint, tint_def);
    let distinct: Vec<Symbol> = registry
        .find_distinct_variant_candidates(sepia, &interner)
        .iter()
        .map(|(e, _)| *e)
        .collect();
    assert_eq!(
        distinct.len(),
        2,
        "a second, different enum declaring `Sepia` is still an ambiguity: {distinct:?}"
    );
    assert!(distinct.contains(&colors_filter) && distinct.contains(&tint));
}
