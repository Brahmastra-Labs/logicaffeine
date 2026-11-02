use super::*;

/// Every op the ISA says mutates a register in place is classified by [`mutation`], and nothing else
/// is: a new mutating op cannot reach the RISC-V tier without a decision between value semantics
/// (an `_own` helper, the register's owned flag, and for a record's field its field flag) and shared
/// semantics (the change reaches every alias).
#[test]
fn every_in_place_mutation_is_classified() {
    for op in Op::all_variants() {
        let mut mutated = Vec::new();
        crate::vm::peephole::mutates(&op, &mut mutated);
        match mutation(&op) {
            Some(Mutation::Owned(target) | Mutation::Field(target)) => assert!(
                mutated.contains(&target),
                "{}: classified as mutating r{target}, but the ISA says it mutates {mutated:?}",
                op.name()
            ),
            Some(Mutation::Shared) => assert!(
                !mutated.is_empty(),
                "{}: classified as a shared mutation, but the ISA says it mutates nothing",
                op.name()
            ),
            None => assert!(
                mutated.is_empty(),
                "{}: the ISA says it mutates {mutated:?} in place; classify it in `mutation`",
                op.name()
            ),
        }
    }
}

/// The kind of every mutating op, stated: what a change through one holder does to the others on the VM.
/// A collection, a record and a record's scalar field are values, and so is a list or a counter in a record's
/// field (`ListPushField`, `CrdtBump`), which the record owns apart from any other holder; a `SharedSequence`
/// field is one log every holder of its record appends to (`CrdtAppend`); `EnsureOwned` changes nothing here.
#[test]
fn every_mutation_kind_is_stated() {
    #[derive(Debug, PartialEq)]
    enum Kind {
        Owned,
        Field,
        Shared,
    }
    const STATED: &[(&str, Kind)] = &[
        ("ListPush", Kind::Owned),
        ("ListPop", Kind::Owned),
        ("SetIndex", Kind::Owned),
        ("SetIndexUnchecked", Kind::Owned),
        ("RemoveFrom", Kind::Owned),
        ("SetAdd", Kind::Owned),
        ("StructInsert", Kind::Owned),
        ("CrdtResolve", Kind::Owned),
        ("CrdtMerge", Kind::Owned),
        ("CrdtJoin", Kind::Owned),
        ("ListPushField", Kind::Field),
        ("CrdtBump", Kind::Field),
        ("CrdtAppend", Kind::Shared),
        ("EnsureOwned", Kind::Shared),
    ];
    let mut unstated = Vec::new();
    for op in Op::all_variants() {
        let Some(kind) = mutation(&op) else { continue };
        let kind = match kind {
            Mutation::Owned(_) => Kind::Owned,
            Mutation::Field(_) => Kind::Field,
            Mutation::Shared => Kind::Shared,
        };
        match STATED.iter().find(|(name, _)| *name == op.name()) {
            Some((_, stated)) => assert_eq!(
                &kind,
                stated,
                "{}: classified {kind:?}, stated {stated:?}",
                op.name()
            ),
            None => unstated.push(op.name()),
        }
    }
    assert!(
        unstated.is_empty(),
        "mutating ops with no stated kind: {unstated:?}"
    );
    for (name, _) in STATED {
        assert!(
            Op::all_variants()
                .iter()
                .any(|op| op.name() == *name && mutation(op).is_some()),
            "{name} is stated but not classified as a mutation"
        );
    }
}
