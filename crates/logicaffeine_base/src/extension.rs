//! The builtins of the exact extension, read and run once for every value model. A program calls
//! them by name (`infinitesimal`, `standardPart`, `tripCount`, …), the declarations and the
//! supertask engine's resolved loops call them too, and the interpreter and a compiled program
//! both answer through [`call`], so a builtin computes the same value and refuses in the same
//! words on every tier.

use crate::numeric::BigInt;
use crate::supertask;
use crate::tower::{self, ValueAlgebra};

/// A builtin of the exact extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Builtin {
    /// `infinitesimal(name, modulus, allowed…)`: a fresh infinitesimal whose reciprocal's residue
    /// modulo `modulus` is one of `allowed`.
    Infinitesimal,
    /// `standardPart(x[, at])`, `isInfinitesimal(x[, at])`, `isLimited(x[, at])`,
    /// `infinitelyClose(x, y[, at])`: the standard part and the predicates, relative to the infinitesimal
    /// `at` when it is given.
    StandardPart,
    IsInfinitesimal,
    IsLimited,
    InfinitelyClose,
    /// The infinite integer `1/dt` an infinitesimal generates.
    GeneratorOf,
    /// A value read under a named limit rule.
    LimitOf,
    /// A search's answer with its evidence: the steps it took and the revisions it made.
    Provisional,
    /// The statement `At the limit, take …`: names a rule, answers nothing.
    AtTheLimit,
    TripCount,
    AffinePower,
    PolynomialFlow,
    CycleAt,
    SearchCandidates,
    StateSize,
    /// A search over every natural that reached a run unresolved.
    EveryNatural,
    /// The exact square root, exponential and natural logarithm, which `sqrt`, `exp` and `ln` are in
    /// a context that declares an exact type.
    ExactSqrt,
    ExactExp,
    ExactLn,
    /// π and e exactly, which `pi` and `e` are in a context that declares an exact type.
    ExactPi,
    ExactE,
    /// ω, the first infinite ordinal, which `omega` is in an `Ordinal` context.
    ExactOmega,
    /// The end state of a loop whose state flows between jumps.
    HybridFlow,
    /// How many passes a loop makes whose guard compares terms linear in a counter it steps.
    FirstExit,
    /// `base ** exponent` to an exponent that is an exact number, which `**` and `pow` are when their
    /// exponent is one: a fraction `p/q` takes the real `q`-th root to the `p`-th power.
    ExactPower,
    /// `orderedInfinitesimal(name, larger, smaller, modulus, allowed…)`: a fresh infinitesimal much
    /// smaller than the infinitesimal `larger` and much larger than `smaller`, either bound `nothing`
    /// when the declaration does not name it.
    OrderedInfinitesimal,
}

impl Builtin {
    pub const ALL: [Builtin; 26] = [
        Builtin::Infinitesimal,
        Builtin::StandardPart,
        Builtin::IsInfinitesimal,
        Builtin::IsLimited,
        Builtin::InfinitelyClose,
        Builtin::GeneratorOf,
        Builtin::LimitOf,
        Builtin::Provisional,
        Builtin::AtTheLimit,
        Builtin::TripCount,
        Builtin::AffinePower,
        Builtin::PolynomialFlow,
        Builtin::CycleAt,
        Builtin::SearchCandidates,
        Builtin::StateSize,
        Builtin::EveryNatural,
        Builtin::ExactSqrt,
        Builtin::ExactExp,
        Builtin::ExactLn,
        Builtin::ExactPi,
        Builtin::ExactE,
        Builtin::ExactOmega,
        Builtin::HybridFlow,
        Builtin::FirstExit,
        Builtin::ExactPower,
        Builtin::OrderedInfinitesimal,
    ];

    /// The builtin's name as a program calls it.
    pub fn name(self) -> &'static str {
        match self {
            Builtin::Infinitesimal => "infinitesimal",
            Builtin::StandardPart => "standardPart",
            Builtin::IsInfinitesimal => "isInfinitesimal",
            Builtin::IsLimited => "isLimited",
            Builtin::InfinitelyClose => "infinitelyClose",
            Builtin::GeneratorOf => "generatorOf",
            Builtin::LimitOf => "limitOf",
            Builtin::Provisional => "provisional",
            Builtin::AtTheLimit => "atTheLimit",
            Builtin::TripCount => "tripCount",
            Builtin::AffinePower => "affinePower",
            Builtin::PolynomialFlow => "polynomialFlow",
            Builtin::HybridFlow => "hybridFlow",
            Builtin::FirstExit => "firstExit",
            Builtin::CycleAt => "cycleAt",
            Builtin::SearchCandidates => "searchCandidates",
            Builtin::StateSize => "stateSize",
            Builtin::EveryNatural => "everyNatural",
            Builtin::ExactSqrt => "exactSqrt",
            Builtin::ExactExp => "exactExp",
            Builtin::ExactLn => "exactLn",
            Builtin::ExactPi => "exactPi",
            Builtin::ExactE => "exactE",
            Builtin::ExactOmega => "exactOmega",
            Builtin::ExactPower => "exactPower",
            Builtin::OrderedInfinitesimal => "orderedInfinitesimal",
        }
    }

    /// The builtin a program calls by `name`.
    pub fn named(name: &str) -> Option<Builtin> {
        Builtin::ALL.into_iter().find(|b| b.name() == name)
    }
}

/// `builtin` applied to `args`, which number as many as the builtin takes (the checker refuses a
/// call with any other count before a program runs).
pub fn call<V: ValueAlgebra>(builtin: Builtin, args: Vec<V>) -> Result<V, String> {
    let mut args = args.into_iter();
    let mut next = || args.next().expect("the checker counted this builtin's arguments");
    let name_of = |v: V| match v.text() {
        Some(t) => Ok(t.to_string()),
        None => Err(format!("an infinitesimal's name is Text, got {}", v.type_name())),
    };
    let natural = |v: V| match v.integer().and_then(|b| b.to_i64()) {
        Some(n) if n >= 0 => Ok(n as u64),
        _ => Err(format!(
            "an infinitesimal's residue facts are natural numbers, got {}",
            v.display_string()
        )),
    };
    match builtin {
        Builtin::Infinitesimal => {
            let name = name_of(next())?;
            let modulus = natural(next())?;
            let allowed = args.map(natural).collect::<Result<Vec<u64>, String>>()?;
            tower::new_infinitesimal(&name, modulus, &allowed)
        }
        Builtin::OrderedInfinitesimal => {
            let name = name_of(next())?;
            let (larger, smaller) = (next(), next());
            let modulus = natural(next())?;
            let allowed = args.map(natural).collect::<Result<Vec<u64>, String>>()?;
            tower::new_ordered_infinitesimal(&name, &larger, &smaller, modulus, &allowed)
        }
        Builtin::StandardPart => {
            let x = next();
            match args.next() {
                Some(at) => tower::standard_part_relative(&x, &at),
                None => tower::standard_part(&x),
            }
        }
        Builtin::IsInfinitesimal => {
            let x = next();
            match args.next() {
                Some(at) => tower::is_infinitesimal_relative(&x, &at),
                None => tower::is_infinitesimal(&x),
            }
        }
        Builtin::IsLimited => {
            let x = next();
            match args.next() {
                Some(at) => tower::is_limited_relative(&x, &at),
                None => tower::is_limited(&x),
            }
        }
        Builtin::InfinitelyClose => {
            let left = next();
            let right = next();
            let difference = V::subtract(left, right)?;
            match args.next() {
                Some(at) => tower::is_infinitesimal_relative(&difference, &at),
                None => tower::is_infinitesimal(&difference),
            }
        }
        Builtin::GeneratorOf => tower::generator_of(&next()),
        Builtin::LimitOf => {
            let value = next();
            let rule = rule_named(&next())?;
            tower::apply_limit(&value, rule)
        }
        Builtin::Provisional => {
            let count = |v: V| match v.integer().and_then(|b| b.to_i64()) {
                Some(n) if n >= 0 => Ok(n as u64),
                _ => Err(format!(
                    "a provisional answer counts with natural numbers, got {}",
                    v.display_string()
                )),
            };
            let value = next();
            let steps = count(next())?;
            let revisions = count(next())?;
            Ok(tower::provisional(value, steps, revisions))
        }
        Builtin::AtTheLimit => {
            rule_named(&next())?;
            Ok(V::nothing())
        }
        Builtin::TripCount => {
            let (t0, step, bound, relation) = (next(), next(), next(), next());
            match relation.text() {
                Some(rel) => supertask::trip_count(&t0, &step, &bound, rel),
                None => Err(format!("a clock's relation is named by Text, got {}", relation.type_name())),
            }
        }
        Builtin::AffinePower => {
            let (matrix, vector, n) = (next(), next(), next());
            supertask::affine_power(&matrix, &vector, &n)
        }
        Builtin::PolynomialFlow => {
            let (field, vector, n) = (next(), next(), next());
            supertask::polynomial_flow(&field, &vector, &n)
        }
        Builtin::HybridFlow => {
            let (model, vector, n) = (next(), next(), next());
            supertask::hybrid_flow(&model, &vector, &n)
        }
        Builtin::FirstExit => {
            let (t0, step, guard) = (next(), next(), next());
            supertask::first_exit(&t0, &step, &guard)
        }
        Builtin::CycleAt => {
            let (states, repeated, n) = (next(), next(), next());
            supertask::cycle_at(&states, &repeated, &n)
        }
        Builtin::SearchCandidates => {
            let (atoms, moduli) = (next(), next());
            supertask::search_candidates(&atoms, &moduli)
        }
        Builtin::StateSize => {
            let bits = supertask::state_size(&next());
            Ok(V::from_bigint(BigInt::from_i64(i64::try_from(bits).unwrap_or(i64::MAX))))
        }
        Builtin::EveryNatural => Err(
            "a search over every natural number reached the runtime unresolved; the supertask engine resolves every such loop before the program runs"
                .to_string(),
        ),
        Builtin::ExactSqrt => tower::real_fn(tower::RealFn::Sqrt, &next()),
        Builtin::ExactExp => tower::real_fn(tower::RealFn::Exp, &next()),
        Builtin::ExactLn => tower::real_fn(tower::RealFn::Ln, &next()),
        Builtin::ExactPi => Ok(tower::pi()),
        Builtin::ExactE => Ok(tower::euler()),
        Builtin::ExactOmega => Ok(tower::omega()),
        Builtin::ExactPower => {
            let (base, exponent) = (next(), next());
            tower::exact_power(&base, &exponent)
        }
    }
}

fn rule_named<V: ValueAlgebra>(rule: &V) -> Result<crate::limit_value::LimitRule, String> {
    match rule.text() {
        Some(name) => tower::limit_rule(name),
        None => Err(format!("a limit rule is named by Text, got {}", rule.type_name())),
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/extension/tests.rs"]
mod tests;
