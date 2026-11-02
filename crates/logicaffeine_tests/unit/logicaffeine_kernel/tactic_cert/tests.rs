use super::*;

fn name(s: &str) -> Syn {
    Syn::Name(s.to_string())
}

#[test]
fn a_reflected_term_round_trips_through_the_reader() {
    let g = syn_binary("le", Syn::Lit(2), name("x"));
    assert_eq!(syn_of_term(&term_of_syn(&g)), Some(g));
}

#[test]
fn the_reader_refuses_a_numeral_the_first_kernel_cannot_read() {
    let big = Term::App(
        Box::new(Term::Global("SLit".to_string())),
        Box::new(Term::Lit(Literal::BigInt(BigInt::from_i64(7)))),
    );
    assert_eq!(syn_of_term(&big), None);
}
