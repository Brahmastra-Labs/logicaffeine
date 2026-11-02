use logicaffeine_kernel::interface::lean as adapter;

#[test]
fn accepts_the_official_unicode_scalar_bound_proof() {
    // The upstream export includes the original proof that both Unicode scalar
    // ranges lie below UInt32.size, with its computed decide/rfl arguments.
    let input = include_str!("fixtures/lean_arena/init-prelude.ndjson")
        .lines()
        .take(19419)
        .collect::<Vec<_>>()
        .join("\n");
    adapter::check_str(&input).expect("the original Lean Unicode scalar bound must check");
}

#[test]
fn accepts_the_official_char_of_nat_aux_body() {
    let input = include_str!("fixtures/lean_arena/init-prelude.ndjson")
        .lines()
        .take(19441)
        .collect::<Vec<_>>()
        .join("\n");
    adapter::check_str(&input).expect("the original Lean Char.ofNatAux body must check");
}

#[test]
fn original_imported_power_computes_the_uint32_bound() {
    use logicaffeine_kernel::{defeq_for_test, BigInt, Literal, Term};
    let input = include_str!("fixtures/lean_arena/init-prelude.ndjson")
        .lines()
        .take(1601)
        .collect::<Vec<_>>()
        .join("\n");
    let (ctx, _) =
        adapter::import_audited(&input).expect("the original arithmetic declarations must check");
    let name = "Nat/s:\"pow\"";
    let lit = |n| Term::Lit(Literal::Nat(BigInt::from_u64(n)));
    let power = Term::Global(name.into());
    let term = Term::App(
        Box::new(Term::App(Box::new(power), Box::new(lit(2)))),
        Box::new(lit(32)),
    );
    assert!(defeq_for_test(&ctx, &term, &lit(4_294_967_296)));
    assert!(!defeq_for_test(&ctx, &term, &lit(4_294_967_295)));
}

#[test]
fn accepts_the_official_zero_below_uint32_bound_proof() {
    let input = include_str!("fixtures/lean_arena/init-prelude.ndjson")
        .lines()
        .take(19453)
        .collect::<Vec<_>>()
        .join("\n");
    adapter::check_str(&input).expect("both kernels must check the original 0 < 2^32 proof");
}
