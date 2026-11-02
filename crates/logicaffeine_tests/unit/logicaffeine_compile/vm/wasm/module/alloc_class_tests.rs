use super::*;

/// Every `Kind` the enum declares is matched by name in `kind_is_handle` — the `match` has no
/// wildcard arm, so this is really just proof the function still compiles; written anyway so a
/// future wildcard added to silence a "non-exhaustive" error is caught by a mutation of THIS
/// test rather than by the compiler going quiet.
#[test]
fn kind_is_handle_disagrees_with_wasm_valtype_exactly_where_the_doc_says() {
    // `Date` and `Word32` are i32-valtype SCALARS — the two kinds `k.wasm_valtype() == I32`
    // would misclassify as handles. Nothing else in the enum is an exception.
    assert!(!kind_is_handle(Kind::Date));
    assert!(!kind_is_handle(Kind::Word32));
    assert_eq!(Kind::Date.wasm_valtype(), I32);
    assert_eq!(Kind::Word32.wasm_valtype(), I32);
    // Every OTHER i32-valtype kind agrees with `kind_is_handle` (a positive control — if this
    // ever fails, either the exception list above grew or `kind_is_handle` drifted from it).
    let i32_valtype_kinds = [
        Kind::SeqInt,
        Kind::SeqBool,
        Kind::SeqFloat,
        Kind::SeqText,
        Kind::SeqStruct,
        Kind::SeqEnum,
        Kind::SeqSeqInt,
        Kind::SeqHandle,
        Kind::SeqAny,
        Kind::Text,
        Kind::Struct,
        Kind::Map,
        Kind::Set,
        Kind::SetText,
        Kind::CrdtSetText,
        Kind::CrdtCounter,
        Kind::Enum,
        Kind::Closure,
        Kind::Tuple,
        Kind::Rational,
        Kind::Optional,
        Kind::SeqWord32,
        Kind::SeqWord64,
        Kind::BigInt,
        Kind::Complex,
        Kind::Modular,
        Kind::Decimal,
        Kind::Money,
        Kind::Quantity,
        Kind::Uuid,
        Kind::Lanes,
        Kind::LanesV,
        Kind::Dynamic,
    ];
    for k in i32_valtype_kinds {
        assert_eq!(k.wasm_valtype(), I32);
        assert!(kind_is_handle(k), "{k:?} is i32-valtype and not one of the two documented exceptions, so it must be a handle");
    }
    // And every scalar (non-i32-valtype, plus the two named exceptions) is NOT a handle.
    for k in [
        Kind::Int,
        Kind::Bool,
        Kind::Char,
        Kind::Float,
        Kind::Moment,
        Kind::Duration,
        Kind::Time,
        Kind::Span,
        Kind::Word64,
    ] {
        assert!(!kind_is_handle(k));
    }
}

/// `AllocClass::code()` round-trips through the exact byte values `heap.md`'s S7 design record
/// promises (0-6) — a census consumer (a later step) reads these off raw memory, so a code
/// silently renumbering would relabel every block a previously-built module already stamped.
#[test]
fn alloc_class_codes_are_pinned() {
    assert_eq!(AllocClass::Raw.code(), 0);
    assert_eq!(AllocClass::ScalarHeader.code(), 1);
    assert_eq!(AllocClass::HandleHeader.code(), 2);
    assert_eq!(AllocClass::MapScalarScalar.code(), 3);
    assert_eq!(AllocClass::MapScalarHandle.code(), 4);
    assert_eq!(AllocClass::MapHandleScalar.code(), 5);
    assert_eq!(AllocClass::MapHandleHandle.code(), 6);
}

#[test]
fn of_map_matches_the_four_combinations() {
    assert_eq!(
        AllocClass::of_map(false, false),
        AllocClass::MapScalarScalar
    );
    assert_eq!(AllocClass::of_map(false, true), AllocClass::MapScalarHandle);
    assert_eq!(AllocClass::of_map(true, false), AllocClass::MapHandleScalar);
    assert_eq!(AllocClass::of_map(true, true), AllocClass::MapHandleHandle);
}

/// An empty map already owns a counted header; its first insertion refines the entry kinds.
/// Unmodeled kinds remain Raw. Scalar and handle collections retain their declared classes.
#[test]
fn empty_collection_class_counts_maps_and_defers_unresolved_kinds() {
    assert_eq!(
        empty_collection_class(Some(Kind::Map)),
        AllocClass::MapScalarScalar
    );
    assert_eq!(empty_collection_class(None), AllocClass::Raw);
    assert_eq!(empty_collection_class(Some(Kind::Struct)), AllocClass::Raw);
    assert_eq!(
        empty_collection_class(Some(Kind::SeqInt)),
        AllocClass::ScalarHeader
    );
    assert_eq!(
        empty_collection_class(Some(Kind::Set)),
        AllocClass::ScalarHeader
    );
    assert_eq!(
        empty_collection_class(Some(Kind::SeqText)),
        AllocClass::HandleHeader
    );
    assert_eq!(
        empty_collection_class(Some(Kind::SetText)),
        AllocClass::HandleHeader
    );
}

#[test]
fn exact_integer_cells_count_host_values_without_heap_children() {
    assert_eq!(AllocClass::ExactIntegerHeader.code(), 12);
    assert_eq!(
        AllocClass::ExactIntegerHeader.held_handles(),
        (false, false)
    );
    assert!(AllocClass::ALL.contains(&AllocClass::ExactIntegerHeader));
}
