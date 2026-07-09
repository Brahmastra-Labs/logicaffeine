//! Regression: the opening of Tarski's geometry as a formal development must DISCHARGE every
//! theorem AND terminate quickly — including under the `verification` feature. The trap is the Z3
//! oracle fallback: it is a COMPLETE solver over the whole goal + KB, so it must run only as a
//! TOP-LEVEL last resort (`depth == 0`), never per recursive subgoal. When it was tried at every
//! node, the recursive `cong_transitivity` / inner-transitivity search fired a Z3 call per node and
//! blew up time + memory (this test hung 8.6 GB under `--features verification`, certified in 0.14s
//! without). Run on a worker thread with a hard wall-clock cap so a non-terminating search is a
//! clean test FAILURE, never a hung suite.

use logicaffeine_proof::development::prove_development;
use std::sync::mpsc;
use std::time::Duration;

const TARSKI: &str = "\
Axiom cong_pseudo_reflexivity: for all a b, Cong(a, b, b, a).
Axiom cong_inner_transitivity: for all a b c d e f, if Cong(a, b, c, d) and Cong(a, b, e, f) then Cong(c, d, e, f).
Axiom cong_identity: for all a b c, if Cong(a, b, c, c) then a = b.
Axiom segment_construction: for all q a b c, there exists x, Bet(q, a, x) and Cong(a, x, b, c).
Axiom bet_identity: for all a b, if Bet(a, b, a) then a = b.
Axiom pasch: for all a p c b q, if Bet(a, p, c) and Bet(b, q, c) then there exists x, Bet(p, x, b) and Bet(q, x, a).
Axiom five_segment: for all a b c d ap bp cp dp, if not (a = b) and Bet(a, b, c) and Bet(ap, bp, cp) and Cong(a, b, ap, bp) and Cong(b, c, bp, cp) and Cong(a, d, ap, dp) and Cong(b, d, bp, dp) then Cong(c, d, cp, dp).
Theorem cong_reflexivity: prove for all a b, Cong(a, b, a, b).
Theorem cong_symmetry cites cong_reflexivity: prove for all a b c d, if Cong(a, b, c, d) then Cong(c, d, a, b).
Theorem cong_transitivity cites cong_symmetry: prove for all a b c d e f, if Cong(a, b, c, d) and Cong(c, d, e, f) then Cong(a, b, e, f).
Theorem null_segment_identity: given Cong(P, Q, R, R); prove P = Q.
Theorem construct_point: prove there exists x, Bet(Q, A, x) and Cong(A, x, B, C).
Theorem cevians_meet: given Bet(A, P, C); given Bet(B, Q, C); prove there exists x, Bet(P, x, B) and Bet(Q, x, A).
Theorem degenerate_betweenness: given Bet(P, Q, P); prove P = Q.
Theorem outer_five_segment: given not (A = B); given Bet(A, B, C); given Bet(Ap, Bp, Cp); given Cong(A, B, Ap, Bp); given Cong(B, C, Bp, Cp); given Cong(A, D, Ap, Dp); given Cong(B, D, Bp, Dp); prove Cong(C, D, Cp, Dp).
";

/// Run `prove_development` on a worker thread; fail if it does not finish within `cap`.
fn prove_within(body: &str, cap: Duration) -> Vec<(String, bool)> {
    let body = body.to_string();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let out = prove_development(&body)
            .expect("development parses")
            .into_iter()
            .map(|(n, r)| (n, r.verified))
            .collect::<Vec<_>>();
        let _ = tx.send(out);
    });
    rx.recv_timeout(cap)
        .unwrap_or_else(|_| panic!("proof search did not terminate within {cap:?} (the five-segment / inner-transitivity explosion)"))
}

#[test]
fn tarski_opening_development_terminates_and_certifies() {
    let results = prove_within(TARSKI, Duration::from_secs(20));
    assert_eq!(results.len(), 8, "eight theorems");
    for (name, verified) in &results {
        assert!(verified, "Tarski theorem '{name}' must be kernel-certified");
    }
}

/// The EXACT body string `compile_theory_for_ui` feeds `parse_development`: the surface lexer
/// re-spaces every token and DROPS the `;` clause separators. If this reproduces the hang, the
/// divergence is the body text (not global state); the parsed AST is identical either way.
const TARSKI_RESPACED: &str = "Axiom cong_pseudo_reflexivity : for all a b , Cong ( a , b , b , a ) . Axiom cong_inner_transitivity : for all a b c d e f , if Cong ( a , b , c , d ) and Cong ( a , b , e , f ) then Cong ( c , d , e , f ) . Axiom cong_identity : for all a b c , if Cong ( a , b , c , c ) then a = b . Axiom segment_construction : for all q a b c , there exists x , Bet ( q , a , x ) and Cong ( a , x , b , c ) . Axiom bet_identity : for all a b , if Bet ( a , b , a ) then a = b . Axiom pasch : for all a p c b q , if Bet ( a , p , c ) and Bet ( b , q , c ) then there exists x , Bet ( p , x , b ) and Bet ( q , x , a ) . Axiom five_segment : for all a b c d ap bp cp dp , if not ( a = b ) and Bet ( a , b , c ) and Bet ( ap , bp , cp ) and Cong ( a , b , ap , bp ) and Cong ( b , c , bp , cp ) and Cong ( a , d , ap , dp ) and Cong ( b , d , bp , dp ) then Cong ( c , d , cp , dp ) . Theorem cong_reflexivity : prove for all a b , Cong ( a , b , a , b ) . Theorem cong_symmetry cites cong_reflexivity : prove for all a b c d , if Cong ( a , b , c , d ) then Cong ( c , d , a , b ) . Theorem cong_transitivity cites cong_symmetry : prove for all a b c d e f , if Cong ( a , b , c , d ) and Cong ( c , d , e , f ) then Cong ( a , b , e , f ) . Theorem null_segment_identity : given Cong ( P , Q , R , R ) prove P = Q . Theorem construct_point : prove there exists x , Bet ( Q , A , x ) and Cong ( A , x , B , C ) . Theorem cevians_meet : given Bet ( A , P , C ) given Bet ( B , Q , C ) prove there exists x , Bet ( P , x , B ) and Bet ( Q , x , A ) . Theorem degenerate_betweenness : given Bet ( P , Q , P ) prove P = Q . Theorem outer_five_segment : given not ( A = B ) given Bet ( A , B , C ) given Bet ( Ap , Bp , Cp ) given Cong ( A , B , Ap , Bp ) given Cong ( B , C , Bp , Cp ) given Cong ( A , D , Ap , Dp ) given Cong ( B , D , Bp , Dp ) prove Cong ( C , D , Cp , Dp ) .";

#[test]
fn tarski_respaced_body_terminates_and_certifies() {
    let results = prove_within(TARSKI_RESPACED, Duration::from_secs(20));
    assert_eq!(results.len(), 8, "eight theorems");
    for (name, verified) in &results {
        assert!(verified, "Tarski theorem '{name}' (respaced body) must be kernel-certified");
    }
}

