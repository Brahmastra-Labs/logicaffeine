use super::*;

/// A definite kill is a write. `kills` (liveness) and `Op::for_each_written_reg` (every register
/// typing) are both exhaustive, so each classifies a new op — this holds them to the same answer:
/// an op may not fully overwrite more registers than the ISA says it writes.
#[test]
fn every_kill_is_a_write() {
    for op in Op::all_variants() {
        let mut killed = Vec::new();
        kills(&op, &mut killed);
        let mut written = 0usize;
        op.for_each_written_reg(|_| written += 1);
        assert!(
            killed.len() <= written,
            "{}: liveness kills {} register(s) but the ISA write set lists {written}",
            op.name(),
            killed.len()
        );
    }
}
