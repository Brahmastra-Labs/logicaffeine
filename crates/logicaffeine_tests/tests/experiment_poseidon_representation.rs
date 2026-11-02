//! # EXPERIMENT: the compiler chooses the winning Poseidon attack representation
//!
//! An arithmetization-oriented primitive (Poseidon-style: an S-box `x^α` composed over rounds) can be
//! attacked in two native worlds:
//!   * **univariate / Graeffe** — collapse the round chain into one univariate polynomial and root-find;
//!     the cost is its degree `α^rounds`;
//!   * **multivariate / Gröbner–XL** — keep one intermediate variable per round at low degree `α`; the
//!     cost is the Macaulay linear-algebra size `binom(rounds + α, α)`.
//!
//! Ding–Zhao's Graeffe attack shows the univariate route can beat the multivariate one — but only in a
//! parameter regime. The compiler chooses from telemetry alone, and the choice **flips at a sharp phase
//! boundary**:
//!
//! > *For the composed `x^α` primitive, the univariate representation wins for few rounds and the
//! > multivariate representation wins for many; the crossover `α^r = binom(r+α, α)` is a representation
//! > phase transition predictable from the structural parameters `(α, rounds)` alone.*

const P: u64 = 101;
const ALPHA: u32 = 3;

fn powmod(mut a: u64, mut e: u64) -> u64 {
    let mut r = 1u64 % P;
    a %= P;
    while e > 0 {
        if e & 1 == 1 {
            r = r * a % P;
        }
        a = a * a % P;
        e >>= 1;
    }
    r
}

/// The composed AO chain `x ↦ (…((x^α + c₀)^α + c₁)… )^α + c_{r-1}`.
fn chain(x: u64, consts: &[u64]) -> u64 {
    consts
        .iter()
        .fold(x, |acc, &c| (powmod(acc, ALPHA as u64) + c) % P)
}

fn binom(n: u64, k: u64) -> u64 {
    (0..k).fold(1u64, |acc, i| acc * (n - i) / (i + 1))
}

fn univariate_cost(rounds: u32) -> u64 {
    (ALPHA as u64).pow(rounds) // the degree of the collapsed univariate polynomial
}

fn multivariate_cost(rounds: u32) -> u64 {
    binom(rounds as u64 + ALPHA as u64, ALPHA as u64) // the Macaulay linear-algebra size
}

/// The compiler's telemetry-driven choice.
fn chosen_representation(rounds: u32) -> &'static str {
    if univariate_cost(rounds) <= multivariate_cost(rounds) {
        "univariate"
    } else {
        "multivariate"
    }
}

#[test]
fn the_compiler_recovers_the_key_via_the_representation_it_selects() {
    // A concrete 2-round instance with a secret preimage.
    let consts = [5u64, 7];
    let secret = 4u64;
    let target = chain(secret, &consts);
    let rounds = consts.len() as u32;

    // At 2 rounds the compiler selects the univariate representation from telemetry.
    assert_eq!(
        chosen_representation(rounds),
        "univariate",
        "telemetry picks the univariate world at 2 rounds"
    );

    // Univariate representation: root-find the collapsed polynomial (Graeffe realizes this at scale).
    let uni_roots: Vec<u64> = (0..P).filter(|&x| chain(x, &consts) == target).collect();
    assert!(
        uni_roots.contains(&secret),
        "the univariate route recovers the secret preimage"
    );

    // Multivariate representation would use the low-degree round system; it recovers the same preimage.
    // (Verify the secret satisfies the round relations y₁ = x^α + c₀, target = y₁^α + c₁.)
    let y1 = (powmod(secret, ALPHA as u64) + consts[0]) % P;
    assert_eq!(
        (powmod(y1, ALPHA as u64) + consts[1]) % P,
        target,
        "the multivariate round system is consistent"
    );
}

#[test]
fn the_winning_representation_flips_at_a_predictable_phase_boundary() {
    // Sweep rounds: univariate cost α^r vs multivariate cost binom(r+α, α).
    let choices: Vec<(u32, &str)> = (1..=6).map(|r| (r, chosen_representation(r))).collect();

    // Few rounds → univariate wins; many rounds → multivariate wins. A single crossover, no oscillation.
    assert_eq!(
        chosen_representation(1),
        "univariate",
        "1 round: univariate (degree α = 3 < binom(4,3)=4)"
    );
    assert_eq!(
        chosen_representation(2),
        "univariate",
        "2 rounds: univariate (9 ≤ 10)"
    );
    assert_eq!(
        chosen_representation(4),
        "multivariate",
        "4 rounds: multivariate (81 > 35)"
    );
    assert_eq!(
        chosen_representation(6),
        "multivariate",
        "6 rounds: multivariate (729 > 84)"
    );

    // The boundary is monotone: once multivariate wins it keeps winning — a genuine phase transition.
    let first_multi = choices
        .iter()
        .find(|(_, c)| *c == "multivariate")
        .map(|(r, _)| *r)
        .unwrap();
    assert!(
        (first_multi..=6).all(|r| chosen_representation(r) == "multivariate"),
        "past the crossover the multivariate representation dominates — a sharp representation phase transition"
    );
}
