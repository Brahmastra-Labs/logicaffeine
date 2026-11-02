use super::*;

#[test]
fn region_bounds_guard_only_inspects_its_inputs() {
    let guard = Op::RegionBoundsGuard {
        array: 7,
        bound: 8,
        iv: 9,
        add_max: 2,
        add_min: -1,
    };

    assert_eq!(inspected_regs(&guard), vec![7, 8, 9]);
}
