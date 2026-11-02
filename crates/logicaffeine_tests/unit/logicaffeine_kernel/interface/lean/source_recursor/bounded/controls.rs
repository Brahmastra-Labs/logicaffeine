use super::*;
use crate::certificate::{
    CaptureRequest, ConversionDiscipline, EnvironmentBase, EnvironmentCertificate,
    EnvironmentLimits,
};
use std::collections::BTreeMap;

fn exercise(export: &'static str, selected: &[&str], corrupt_fields: bool) {
    let selected: Vec<String> = selected.iter().map(|s| s.to_string()).collect();
    let limits = EnvironmentLimits::default();
    let stack = crate::term_store::stack_bytes_for_depth(
        limits.max_depth * 3 + crate::term_store::INLINE_DEPTH,
    );
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(move || {
            let reference = super::super::super::reference::Reference::read(export)
                .expect("authentic source tables");
            let (context, _) =
                super::super::super::import(export).expect("authentic fixture imports");
            let mut budget = RecursorBudget {
                max_depth: crate::type_checker::MAX_TERM_DEPTH,
                max_tree_nodes: 1_000_000,
                max_store_nodes: 1_000_000,
                work: 1_000_000_000_000_000,
                allocation: 1_000_000_000_000,
                materialized_nodes: 100_000_000,
            };
            for selected in selected {
                let target = reference
                    .declaration(&selected)
                    .expect("authentic named recursor");
                let certificate = EnvironmentCertificate::capture(&CaptureRequest {
                    context: &context,
                    base: EnvironmentBase::Kernel,
                    conversion: ConversionDiscipline::Exact,
                    target: &target.name,
                    opaque_proofs: &BTreeMap::new(),
                })
                .expect("capture authentic recursor closure");
                let mut checked = HashSet::new();
                let mut observed = 0;
                let mut cache = BTreeMap::<String, CanonicalRecursor>::new();
                certificate
                    .rebuild_observing_definitions(
                        &limits,
                        &mut |prefix, name, levels, ty, body| {
                            let Ok(source) = reference.declaration(name) else {
                                return;
                            };
                            if source.kind
                                != super::super::super::reference::ReferenceKind::Recursor
                            {
                                return;
                            }
                            if !checked.contains(name) {
                                let mut block = reference
                                    .recursor_block(name)
                                    .expect("independent complete block decode");
                                if corrupt_fields {
                                    let rule = block
                                        .recursors
                                        .iter_mut()
                                        .find_map(|r| r.rules.first_mut())
                                        .expect("fixture has a computation rule");
                                    rule.num_fields += 1;
                                    assert!(
                                        derive_bounded(prefix, &block, &mut budget).is_err(),
                                        "inconsistent source rule metadata must be rejected"
                                    );
                                    for r in &block.recursors {
                                        checked.insert(r.declaration.name.clone());
                                    }
                                    observed += 1;
                                    return;
                                }
                                let before = budget.work;
                                let derived = derive_bounded(prefix, &block, &mut budget).expect(
                                    "bounded canonical source block derives before registration",
                                );
                                assert!(
                                    budget.work < before,
                                    "one caller budget pays for each derived block"
                                );
                                let original = super::super::derive(prefix, &block)
                                    .expect("original canonical comparison path");
                                assert_eq!(derived.len(), original.len());
                                for (bounded, old) in derived.iter().zip(&original) {
                                    assert_eq!(bounded.name, old.name);
                                    assert_eq!(bounded.levels, old.levels);
                                    exact(prefix, &bounded.ty, &old.ty).expect(
                                        "bounded canonical type preserves original semantics",
                                    );
                                    exact(prefix, &bounded.body, &old.body).expect(
                                        "bounded canonical body preserves original semantics",
                                    );
                                }
                                assert_eq!(derived.len(), block.recursors.len());
                                for r in derived {
                                    checked.insert(r.name.clone());
                                    cache.insert(r.name.clone(), r);
                                }
                            }
                            if corrupt_fields {
                                return;
                            }
                            let expected = &cache[name];
                            assert_eq!(levels, expected.levels);
                            exact(prefix, ty, &expected.ty)
                                .expect("carried recursor type equals source canonical type");
                            exact(prefix, body, &expected.body).expect(
                                "carried recursor body equals source canonical implementation",
                            );
                            observed += 1;
                        },
                    )
                    .expect("certificate remains kernel checked");
                assert!(
                    observed > 0,
                    "the requested recursor must actually reach the prefix observer"
                );
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn bounded_canonical_singleton_prop_recursor_matches_authentic_export() {
    exercise(
        include_str!("../../../../../../tests/fixtures/lean_package/W05.clean.ndjson"),
        &["Eq.rec"],
        false,
    );
}

#[test]
fn bounded_canonical_mutual_recursors_preserve_the_shared_motive_universe() {
    exercise(
        include_str!(
            "../../../../../../tests/fixtures/lean_arena/ordinary-mutual-polymorphic.ndjson"
        ),
        &["ArenaTree.rec", "ArenaForest.rec"],
        false,
    );
}

#[test]
fn bounded_canonical_nested_recursors_include_every_auxiliary() {
    exercise(
        include_str!("../../../../../../tests/fixtures/lean_semantics/LF_W03_NestedRose.ndjson"),
        &[
            "LF_W03R.Rose.rec",
            "LF_W03R.Rose.rec_1",
            "LF_W03R.RoseOfTrees.rec_2",
        ],
        false,
    );
}

#[test]
fn bounded_source_rule_field_metadata_is_checked_against_canonical_construction() {
    exercise(
        include_str!(
            "../../../../../../tests/fixtures/lean_arena/ordinary-mutual-polymorphic.ndjson"
        ),
        &["ArenaTree.rec"],
        true,
    );
}

#[test]
fn bounded_canonical_structure_recursors_inline_checked_field_projections() {
    exercise(
        include_str!("../../../../../../tests/fixtures/lean_semantics/LF_StructEtaIota.ndjson"),
        &["LF_SE.P2.rec", "Prod.rec"],
        false,
    );
}

#[test]
fn bounded_canonical_mutual_structure_recursors_preserve_minor_positions() {
    exercise(
        include_str!("../../../../../../tests/fixtures/lean_semantics/LK_MutualStructEta.ndjson"),
        &[
            "LK_MSE.NA.rec",
            "LK_MSE.NB.rec",
            "LK_MSE.MA.rec",
            "LK_MSE.MB.rec",
        ],
        false,
    );
}
