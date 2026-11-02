use super::*;
use crate::limit_value::ResidueType;

fn scale(name: &str) -> Arc<Scale> {
    Arc::new(Scale::new(name, ResidueType::any()))
}

fn below(name: &str, larger: &Arc<Scale>) -> Arc<Scale> {
    Arc::new(Scale::ordered(name, ResidueType::any(), Some(larger.clone()), None).unwrap())
}

struct Two {
    dt: Multi,
    dx: Multi,
}

fn two() -> Two {
    let t = scale("dt");
    let x = below("dx", &t);
    Two {
        dt: Multi::generator(&t),
        dx: Multi::generator(&x),
    }
}

fn n(k: i64) -> Multi {
    <Multi as OrderedField>::from_i64(k)
}

fn div(a: &Multi, b: &Multi) -> Multi {
    a.mul(&b.recip().unwrap())
}

fn sqrt(x: &Multi) -> Multi {
    x.root(2).unwrap()
}

#[test]
fn a_root_of_a_value_of_both_is_held_in_the_closure() {
    let Two { dt, dx } = two();
    let r = sqrt(&dt.add(&dx));
    assert!(matches!(r, Multi::Alg(_)), "{r:?}");
    assert_eq!(r.mul(&r), dt.add(&dx), "its square is the rational function it is a root of");
    assert!(matches!(r.mul(&r), Multi::Over { .. }));
    assert_eq!(r.sub(&r), n(0));
    assert_eq!(r.signum(), 1);
    assert_eq!(r.neg().signum(), -1);
}

#[test]
fn the_root_of_a_square_is_found_to_be_a_rational_function() {
    let Two { dt, dx } = two();
    let square = dt.add(&dx).mul(&dt.add(&dx));
    assert_eq!(sqrt(&square), dt.add(&dx));
    assert!(matches!(sqrt(&square), Multi::Over { .. }));
    let cube = dt.add(&dx).pow(3).unwrap();
    assert_eq!(cube.root(3).unwrap(), dt.add(&dx));
}

#[test]
fn roots_reached_two_ways_are_one_value() {
    let Two { dt, dx } = two();
    let a = sqrt(&dt.add(&dx)).mul(&sqrt(&dt.sub(&dx)));
    let b = sqrt(&dt.mul(&dt).sub(&dx.mul(&dx)));
    assert_eq!(a, b);
    assert_eq!(a.sub(&b), n(0));
    let mut hashes = std::collections::HashSet::new();
    hashes.insert(a.clone());
    assert!(hashes.contains(&b), "equal values hash alike");
}

/// Two roots that differ are two values: the first orders of their expansions tell `√(dt + dx)` from
/// `√(dt + 2·dx)`, and only their difference's polynomial tells it from `√(dt + dx) + dx^10`, which
/// agrees with it through every order read first; built another way, a root is the same value.
#[test]
fn roots_that_differ_are_two_values() {
    let Two { dt, dx } = two();
    let r = sqrt(&dt.add(&dx));
    let doubled = sqrt(&dt.add(&dx.mul(&n(2))));
    assert_ne!(r, doubled);
    assert_ne!(r.add(&n(1)), doubled.add(&n(1)));
    let tenth = dx.pow(10).unwrap();
    let far = r.add(&tenth);
    assert_ne!(r, far);
    assert_eq!(far.sub(&tenth), r);
}

/// The orders of the nonzero roots bound every scan: a concave sum lies below twice the root.
#[test]
fn comparisons_of_roots_are_decided() {
    let Two { dt, dx } = two();
    let left = sqrt(&dt.add(&dx)).add(&sqrt(&dt.sub(&dx)));
    let right = sqrt(&dt).mul(&n(2));
    assert_eq!(left.sub(&right).signum(), -1);
    assert_eq!(sqrt(&dt.add(&dx)).sub(&sqrt(&dt)).signum(), 1);
    assert!(sqrt(&dt.add(&dx)).is_infinitesimal());
}

/// The derivative of `√t` at `dt` is the standard part of the difference quotient relative to `dt`.
#[test]
fn a_difference_quotient_reads_a_derivative_relative_to_the_larger() {
    let t = scale("dt");
    let x = below("dx", &t);
    let (dt, dx) = (Multi::generator(&t), Multi::generator(&x));
    let q = div(&sqrt(&dt.add(&dx)).sub(&sqrt(&dt)), &dx);
    let derivative = div(&n(1), &sqrt(&dt).mul(&n(2)));
    assert_eq!(q.standard_part_at(Some(&t)), Some(derivative));
    assert!(!q.is_limited());
    assert_eq!(q.render(), "1/(2·√dt) - (1/(8·dt^(3/2)))·dx + O(dx^2)");
}

#[test]
fn a_root_prints_as_a_radical_or_by_its_leading_terms() {
    let Two { dt, dx } = two();
    let r = sqrt(&dt.add(&dx));
    assert_eq!(r.render(), "√(dt + dx)");
    assert_eq!(r.mul(&n(2)).render(), "2·√(dt + dx)");
    assert_eq!(r.neg().render(), "-√(dt + dx)");
    assert_eq!(r.recip().unwrap().render(), "1/√(dt + dx)");
    assert_eq!(dt.add(&dx).root(3).unwrap().render(), "∛(dt + dx)");
    let shifted = sqrt(&n(1).add(&dx)).add(&dt);
    assert_eq!(shifted.render(), "1 + dt + (1/2)·dx + O(dx^2)");
}

#[test]
fn a_root_travels_by_its_recipe() {
    let Two { dt, dx } = two();
    let values = [
        sqrt(&dt.add(&dx)),
        sqrt(&dt.add(&dx)).add(&sqrt(&dt.sub(&dx))),
        div(&sqrt(&dt.add(&dx)).sub(&sqrt(&dt)), &dx),
        sqrt(&n(1).add(&dx)).add(&dt),
    ];
    for v in values {
        let bytes = v.to_bytes();
        let back = Multi::from_bytes(&bytes).unwrap_or_else(|| panic!("{} did not read back", v.render()));
        assert_eq!(back, v);
        assert_eq!(back.render(), v.render());
    }
}

/// A recipe that builds a rational function is no root's encoding: the reader holds the function.
#[test]
fn a_root_encoding_of_a_rational_function_is_refused() {
    let Two { dt, dx } = two();
    let r = Arc::new(AlgValue::at(&sqrt(&dt.add(&dx)), &match &sqrt(&dt.add(&dx)) {
        Multi::Alg(a) => a.scale.clone(),
        _ => unreachable!(),
    }));
    let forged = Multi::Alg(Arc::new(AlgValue {
        scale: r.scale.clone(),
        recipe: Recipe::Mul(r.clone(), r.clone()),
        poly: r.poly.clone(),
        expansion: r.expansion.mul(&r.expansion),
    }));
    assert_eq!(Multi::from_bytes(&forged.to_bytes()), None);
}
