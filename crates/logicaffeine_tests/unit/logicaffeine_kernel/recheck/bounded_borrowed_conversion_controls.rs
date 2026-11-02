use super::*;

#[test]
fn syntactic_tree_comparison_cannot_bypass_the_probe_allowance() {
    let mut term = Db::Sort(Universe::Type(0));
    for _ in 0..64 {
        term = Db::Pi(DbBox::new(Db::Sort(Universe::Type(0))), DbBox::new(term));
    }
    let mut fuel = 8;
    assert!(
        !borrowed_congruence(&term, &term, &mut fuel),
        "this positive-only probe must decline a tree exceeding its work allowance"
    );
    assert_eq!(fuel, 0);
    let mut sufficient = 256;
    assert!(borrowed_congruence(&term, &term, &mut sufficient));
    assert_eq!(sufficient, 256 - 129);
}
