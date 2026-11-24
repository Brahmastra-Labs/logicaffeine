use super::types::{is_c_map_value, map_ctype, map_entry_types, Ints};
use super::*;

fn point_and_shape() -> (Interner, Symbol, Symbol) {
    let mut interner = Interner::new();
    let (point, shape) = (interner.intern("Point"), interner.intern("Shape"));
    (interner, point, shape)
}

/// A map of a record or an enum value, under a text or an integer key, is the generic family of that value; a value
/// type under a text key keeps its fixed family's name, under an integer key the generic family's; the integer and
/// text families are unchanged.
#[test]
fn a_map_of_any_value_is_the_family_of_that_value() {
    let (_, point, shape) = point_and_shape();
    assert_eq!(map_ctype(&CType::String, &CType::Struct(point)), CType::MapStrValue(Box::new(CType::Struct(point))));
    assert_eq!(map_ctype(&CType::Int64, &CType::Struct(point)), CType::MapI64Value(Box::new(CType::Struct(point))));
    assert_eq!(map_ctype(&CType::String, &CType::Enum(shape)), CType::MapStrValue(Box::new(CType::Enum(shape))));
    assert_eq!(map_ctype(&CType::Int64, &CType::Money), CType::MapI64Value(Box::new(CType::Money)));
    assert_eq!(map_ctype(&CType::String, &CType::Money), CType::MapStrValue(Box::new(CType::Money)));
    let pair = CType::Tuple(vec![CType::Int64, CType::String]);
    assert_eq!(map_ctype(&CType::Int64, &pair), CType::MapI64Value(Box::new(pair.clone())));
    assert_eq!(map_ctype(&CType::Int64, &CType::Int64), CType::MapI64I64(Ints::Int, Ints::Int));
    assert_eq!(map_ctype(&CType::String, &CType::Int64), CType::MapStrI64(Ints::Int));
    assert_eq!(map_ctype(&CType::String, &CType::String), CType::MapStrStr);
}

/// Every value a record, an enum, a tuple, a list or a value type is may be a map's; a value-or-nothing has a family of
/// its own, and nothing is no value.
#[test]
fn a_map_holds_any_value_but_nothing() {
    let (_, point, shape) = point_and_shape();
    for held in [
        CType::Struct(point),
        CType::Enum(shape),
        CType::Tuple(vec![CType::Int64]),
        CType::SeqStruct(point),
        CType::SeqSeq(Box::new(CType::SeqI64(Ints::Int))),
        CType::SeqI64(Ints::Int),
        CType::Money,
        CType::Tower,
    ] {
        assert!(is_c_map_value(&held), "{held:?}");
    }
    assert!(!is_c_map_value(&CType::Void));
    assert!(!is_c_map_value(&CType::Opt(Box::new(CType::Int64))));
}

/// A loop over any map binds a key and a value of the map's own types, the generic families' as the fixed ones'.
#[test]
fn every_map_has_the_entries_a_loop_binds() {
    let (_, point, shape) = point_and_shape();
    assert_eq!(
        map_entry_types(&CType::MapStrValue(Box::new(CType::Struct(point)))),
        Some((CType::String, CType::Struct(point)))
    );
    assert_eq!(
        map_entry_types(&CType::MapI64Value(Box::new(CType::Enum(shape)))),
        Some((CType::Int64, CType::Enum(shape)))
    );
    assert_eq!(map_entry_types(&CType::MapStrStr), Some((CType::String, CType::String)));
    assert_eq!(map_entry_types(&CType::SeqI64(Ints::Int)), None);
}

/// The family's typedef holds its values behind a pointer, so it needs the value's type declared and no more.
#[test]
fn a_map_family_holds_its_values_behind_a_pointer() {
    let (interner, point, _) = point_and_shape();
    assert_eq!(
        generic_map_typedef(&CType::MapStrValue(Box::new(CType::Struct(point))), &interner).as_deref(),
        Some("typedef struct { char **keys; Point *vals; uint8_t *state; size_t cap; size_t len; size_t *order; size_t ord_len; } Map_str_Point;")
    );
    assert_eq!(
        generic_map_typedef(&CType::MapI64Value(Box::new(CType::Struct(point))), &interner).as_deref(),
        Some("typedef struct { int64_t *keys; Point *vals; uint8_t *state; size_t cap; size_t len; size_t *order; size_t ord_len; } Map_i64_Point;")
    );
    assert_eq!(generic_map_typedef(&CType::MapStrI64(Ints::Int), &interner), None);
}

/// The family is declared before it is defined: its name and members are one layout, the anonymous typedef the
/// value-or-nothing family takes is built from it, and the aggregate unit's forward declaration names the same struct.
#[test]
fn a_map_family_has_one_layout_its_typedef_and_its_declaration_share() {
    let (interner, point, _) = point_and_shape();
    let map = CType::MapStrValue(Box::new(CType::SeqStruct(point)));
    let (name, members) = generic_map_layout(&map, &interner).expect("a generic map");
    assert_eq!(name, "Map_str_Seq_Point");
    assert_eq!(
        members,
        "char **keys; Seq_Point *vals; uint8_t *state; size_t cap; size_t len; size_t *order; size_t ord_len;"
    );
    assert_eq!(generic_map_typedef(&map, &interner), Some(format!("typedef struct {{ {members} }} {name};")));
    assert_eq!(generic_map_layout(&CType::MapStrStr, &interner), None);
}

/// The family's functions: the operations always, the renderers when its values render; every one marked as a
/// function a program may leave unused.
#[test]
fn a_map_family_has_its_operations_and_renders_what_renders() {
    let (interner, point, _) = point_and_shape();
    let map = CType::MapI64Value(Box::new(CType::Struct(point)));
    let names = |fns: &[CFn]| -> Vec<String> {
        fns.iter()
            .map(|f| f.sig.rsplit_once('(').unwrap().0.rsplit(' ').next().unwrap().trim_start_matches('*').to_string())
            .collect()
    };
    let plain = generic_map_fns(&map, &interner, false);
    assert_eq!(
        names(&plain),
        ["map_i64_Point_new", "_map_i64_Point_resize", "map_i64_Point_set", "map_i64_Point_get", "map_i64_Point_contains", "map_i64_Point_remove", "map_i64_Point_len"]
    );
    let shown = generic_map_fns(&map, &interner, true);
    assert_eq!(&names(&shown)[7..], ["map_i64_Point_to_string", "show_map_i64_Point"]);
    assert!(shown[7].body.contains("Point_to_string(m->vals[si])"), "a value renders as its type does");
    for f in &shown {
        assert!(f.sig.starts_with("__attribute__((unused)) static "), "{}", f.sig);
        assert!(f.body.starts_with('{') && f.body.ends_with('}'), "{}", f.body);
    }
    assert!(generic_map_fns(&CType::MapStrStr, &interner, true).is_empty());
}

/// The maps a type holds, at any depth, each once; a text-keyed map of a value type and a map of values-or-nothing,
/// whose families the program already has, are left out.
#[test]
fn the_maps_a_type_holds_are_collected_once_at_any_depth() {
    let (interner, point, shape) = point_and_shape();
    let of_points = CType::MapStrValue(Box::new(CType::Struct(point)));
    let of_shapes = CType::MapI64Value(Box::new(CType::Enum(shape)));
    let nested = CType::MapStrValue(Box::new(CType::SeqValue(Box::new(of_shapes.clone()))));
    let (mut seen, mut out) = (std::collections::HashSet::new(), Vec::new());
    for ty in [
        CType::SeqValue(Box::new(of_points.clone())),
        CType::Tuple(vec![CType::Int64, of_points.clone()]),
        CType::Opt(Box::new(of_shapes.clone())),
        nested.clone(),
        CType::MapStrValue(Box::new(CType::Money)),
        CType::MapI64Value(Box::new(CType::Opt(Box::new(CType::Int64)))),
    ] {
        collect_mapvalue_from_ctype(&ty, &mut seen, &mut out, &interner);
    }
    assert_eq!(out, [of_points, of_shapes, nested]);
}
