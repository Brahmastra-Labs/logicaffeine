use super::*;

#[test]
fn intermediate_growth_retains_existing_cap_and_restores_parent_budget() {
    let before = kernel_stack_bytes();
    let depth = INLINE_DEPTH + 40;
    let bytes = grow_stack_for_depth(depth, || {
        let bytes = kernel_stack_bytes();
        assert!(bytes >= stack_bytes_for_depth(depth));
        assert!(bytes <= stack_bytes_for_depth(crate::type_checker::MAX_TERM_DEPTH));
        let nested = grow_stack_for_depth(depth + 1, kernel_stack_bytes);
        assert!(nested >= bytes);
        bytes
    });
    assert!(bytes >= before);
    assert_eq!(kernel_stack_bytes(), before);
}
