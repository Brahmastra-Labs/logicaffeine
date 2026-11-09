use super::*;

/// The Arena's large exports at lean-kernel-arena 66c354d6: their bytes and the expressions a
/// parse of each interns (`logos_kernel --parse-only`).
const EXPORTS: [(&str, u64, usize); 6] = [
    ("init", 347_555_345, 6_132_277),
    ("std", 596_655_086, 10_145_829),
    ("con-leche", 551_412_545, 9_679_077),
    ("navier-stokes-euler", 1_559_685_706, 27_747_197),
    ("cslib", 2_422_584_798, 40_045_123),
    ("mathlib", 6_184_415_052, 104_860_875),
];

/// The persistent table's slots for `n` entries: the smallest power of two that holds them at
/// most half full.
fn slots(n: usize) -> usize {
    ((n + 1) * 2).next_power_of_two()
}

fn persistent_table_bytes(k: &Kernel) -> usize {
    k.s.memory_report()
        .into_iter()
        .find(|(what, _)| *what == "persistent table bytes")
        .expect("the store reports its persistent table")
        .1
}

#[test]
fn a_size_hint_reserves_the_table_its_estimate_needs_half_full() {
    let mut k = Kernel::new();
    let bytes = 56 * 100_000;
    Reader::new(&mut k).size_hint(bytes);
    assert_eq!(
        persistent_table_bytes(&k),
        slots(Reader::expected_expressions(bytes)) * 4
    );
}

#[test]
fn each_arena_export_reserves_the_persistent_table_its_expressions_need() {
    for (name, bytes, exprs) in EXPORTS {
        assert_eq!(
            slots(Reader::expected_expressions(bytes)),
            slots(exprs),
            "{name}: {bytes} bytes, {exprs} expressions"
        );
    }
}

#[test]
fn the_estimate_covers_each_arena_export() {
    for (name, bytes, exprs) in EXPORTS {
        let estimate = Reader::expected_expressions(bytes);
        assert!(estimate >= exprs, "{name}: {estimate} estimated, {exprs} interned");
        assert!(
            estimate * 100 <= exprs * 110,
            "{name}: {estimate} estimated, {exprs} interned"
        );
    }
}
