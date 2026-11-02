use super::*;

#[test]
fn reflection_attack_preservation_is_kernel_proved() {
    assert!(
        reflection_preserves_attacks(),
        "kernel LIA failed to prove the reflection preserves the attack relation"
    );
}

#[test]
fn full_closure_is_kernel_proven() {
    // bit-level commutation (B1) AND geometric attack-preservation (LIA).
    assert!(reflection_closure_proven());
}

#[test]
fn value_symmetry_group_is_transitive_symmetric() {
    // permgroup-checked: the equality-only value system's symmetry group is
    // the full, transitive Sₖ (order k!, one orbit, equality-preserving).
    assert!(
        value_bijection_preserves_equality(),
        "permgroup failed to certify the value symmetry as the transitive Sₖ"
    );
    assert!(value_symmetry_closure_proven());
}
