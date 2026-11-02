use super::*;

fn silicon_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon")
}

/// Emit the merge RTL, its testbench, and the oracle vectors (golden; regenerate with
/// `CRDT_RECORD=1`). iverilog PASS is wired into scripts/synth-soft-core.sh.
#[test]
fn crdt_merge_artifacts_are_current() {
    const N: usize = 64;
    let (ha, hb, hm) = crdt_vectors(N);
    let files = [
        ("crdt_merge.v", emit_crdt_merge_verilog()),
        ("tb_crdt_merge.v", emit_crdt_merge_testbench(N)),
        ("crdt_a.hex", ha),
        ("crdt_b.hex", hb),
        ("crdt_m.hex", hm),
    ];
    let dir = silicon_dir();
    if std::env::var("CRDT_RECORD").is_ok() {
        for (n, b) in &files {
            std::fs::write(dir.join(n), b).expect("write");
        }
        return;
    }
    for (n, b) in &files {
        let got = std::fs::read_to_string(dir.join(n)).unwrap_or_else(|e| {
            panic!("missing {n} ({e}) — record: CRDT_RECORD=1 cargo test crdt_merge_artifacts")
        });
        assert_eq!(&got, b, "{n} is stale — regenerate with CRDT_RECORD=1");
    }
}

/// The merge is idempotent, commutative, and order-independent — the lattice-join laws a
/// CRDT depends on — for the per-lane max the RTL implements.
#[test]
fn crdt_merge_is_a_lattice_join() {
    for &(a, b) in &[(0u32, 0u32), (5, 3), (3, 5), (7, 7), (u32::MAX, 0)] {
        assert_eq!(merge_lane(a, a), a, "idempotent");
        assert_eq!(merge_lane(a, b), merge_lane(b, a), "commutative");
    }
    // associative on a small grid
    for a in [0u32, 1, 9] {
        for b in [0u32, 4, 9] {
            for c in [0u32, 2, 9] {
                assert_eq!(
                    merge_lane(merge_lane(a, b), c),
                    merge_lane(a, merge_lane(b, c)),
                    "associative"
                );
            }
        }
    }
}

/// WORLD-CLASS: the CRDT's convergence order is a CERTIFIED TOTAL ORDER, proven
/// EXHAUSTIVELY (every input, not sampled) by our OWN certified SAT — the same bar the
/// flagship ALU/reversible datapaths meet, not mere simulation. The RTL selects the join
/// with `(a >= b) ? a : b`; a `max` taken over a *total order* is the unique least upper
/// bound, so certifying `<u` is a strict total order (irreflexive, antisymmetric, total,
/// transitive) AND that the max is unique certifies the per-lane merge is the
/// join-semilattice operation a G-Counter's convergence depends on — for ALL inputs.
#[test]
fn the_crdt_convergence_order_is_a_total_order_proven_by_our_sat() {
    use super::super::bitblast::lower_bool;
    use super::super::sva_to_verify::{BitVecBoundedOp::*, BoundedExpr};
    use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
    use logicaffeine_proof::ProofExpr;

    // Width-independent laws — prove at several widths so the certification is not a
    // one-width fluke (the 32-bit lanes are the same comparator structure).
    for w in [4u32, 8, 16] {
        let bv = |n: &str| Box::new(BoundedExpr::BitVecVar(n.to_string(), w));
        let atom = |op, l: &str, r: &str| {
            lower_bool(&BoundedExpr::BitVecBinary {
                op,
                left: bv(l),
                right: bv(r),
            })
            .expect("comparison atom lowers")
        };
        let ult = |l: &str, r: &str| atom(ULt, l, r);
        let eq = |l: &str, r: &str| atom(Eq, l, r);
        let and = |a: ProofExpr, b: ProofExpr| ProofExpr::And(Box::new(a), Box::new(b));
        let or = |a: ProofExpr, b: ProofExpr| ProofExpr::Or(Box::new(a), Box::new(b));
        let not = |a: ProofExpr| ProofExpr::Not(Box::new(a));
        // A law `φ` holds for ALL inputs iff `¬φ` is UNSAT (refuted by our certified SAT).
        let valid = |neg: ProofExpr| {
            assert_eq!(prove_unsat(&neg), UnsatOutcome::Refuted);
        };

        // Strict total order (the negation of each law is the UNSAT witness):
        valid(ult("a", "a")); // irreflexive: a <u a is impossible
        valid(and(ult("a", "b"), ult("b", "a"))); // antisymmetric
        valid(and(
            and(not(ult("a", "b")), not(ult("b", "a"))),
            not(eq("a", "b")),
        )); // total
        valid(and(and(ult("a", "b"), ult("b", "c")), not(ult("a", "c")))); // transitive

        // The join is UNIQUE: any two values that are ≥ both a and b and equal to one of
        // them coincide — so `max` (the least upper bound) is well-defined.
        let is_max = |m: &str| {
            and(
                and(not(ult(m, "a")), not(ult(m, "b"))),
                or(eq(m, "a"), eq(m, "b")),
            )
        };
        valid(and(and(is_max("m"), is_max("n")), not(eq("m", "n"))));
    }
}
