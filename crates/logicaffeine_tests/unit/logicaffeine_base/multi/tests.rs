use super::*;
use crate::limit_value::ResidueType;

fn scale(name: &str) -> Arc<Scale> {
    Arc::new(Scale::new(name, ResidueType::any()))
}

fn below(name: &str, larger: &Arc<Scale>) -> Arc<Scale> {
    Arc::new(Scale::ordered(name, ResidueType::any(), Some(larger.clone()), None).unwrap())
}

fn above(name: &str, smaller: &Arc<Scale>) -> Arc<Scale> {
    Arc::new(Scale::ordered(name, ResidueType::any(), None, Some(smaller.clone())).unwrap())
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
    Multi::from_i64(k)
}

fn pow(x: &Multi, k: i32) -> Multi {
    x.pow(k).unwrap()
}

fn div(a: &Multi, b: &Multi) -> Multi {
    a.mul(&b.recip().unwrap())
}

/// `dx` is smaller than every power of `dt`, and positive.
#[test]
fn the_smaller_infinitesimal_lies_below_every_power_of_the_larger() {
    let Two { dt, dx } = two();
    assert_eq!(dx.signum(), 1);
    for k in [1, 2, 10, 1000] {
        assert_eq!(pow(&dt, k).sub(&dx).signum(), 1, "dx < dt^{k}");
    }
    assert!(div(&dx, &pow(&dt, 1000)).is_infinitesimal(), "dx/dt^1000 is infinitesimal");
    let inverse = dx.recip().unwrap();
    assert_eq!(inverse.sub(&pow(&dt, -1000)).signum(), 1, "1/dx exceeds 1/dt^1000");
    assert!(!inverse.is_limited());
    assert!(dx.is_infinitesimal() && dx.is_limited());
}

/// A value is held at the smallest infinitesimal it involves, and a value of one infinitesimal as that
/// infinitesimal's own.
#[test]
fn a_value_is_held_at_the_smallest_infinitesimal_it_involves() {
    let Two { dt, dx } = two();
    assert_eq!(dt.add(&dx).sub(&dx), dt);
    assert!(matches!(dt.add(&dx).sub(&dx), Multi::One(..)));
    assert_eq!(dx.mul(&dx.recip().unwrap()), n(1));
    assert!(matches!(dx.mul(&dx.recip().unwrap()), Multi::Standard(_)));
    assert_eq!(div(&dx.mul(&dt), &dt), dx);
    assert_eq!(dx.add(&dx), dx.mul(&n(2)));
    assert_eq!(dt.add(&dx).top().unwrap().name, "dx");
    assert!(matches!(dt.add(&dx), Multi::Over { .. }));
    assert_eq!(dt.add(&dx), dx.add(&dt), "one form whichever side the smaller stands on");
    assert_eq!(dt.add(&dx).scales().len(), 2);
}

#[test]
fn the_standard_part_reads_each_infinitesimal_in_turn() {
    let Two { dt, dx } = two();
    let x = n(3).add(&dt).add(&dx);
    assert_eq!(x.standard_part(), Some(Algebraic::from_i64(3)));
    assert!(x.is_limited() && !x.is_infinitesimal());
    let slope = div(&dx.add(&dt.mul(&dx)), &dx);
    assert_eq!(slope, n(1).add(&dt), "(dx + dt·dx)/dx = 1 + dt");
    assert_eq!(slope.standard_part(), Some(Algebraic::from_i64(1)));
    let quotient = div(&dx, &dt);
    assert_eq!(quotient.standard_part(), Some(Algebraic::from_i64(0)));
    assert!(quotient.is_infinitesimal());
    assert_eq!(div(&dt, &dx).standard_part(), None, "dt/dx is infinite");
    assert_eq!(dx.add(&dt.recip().unwrap()).standard_part(), None, "1/dt + dx is infinite at dt");
}

/// From `dt`'s standpoint every value of `dt` is ordinary and `dx` is infinitesimal.
#[test]
fn the_standard_part_relative_to_the_larger_keeps_it() {
    let t = scale("dt");
    let x = below("dx", &t);
    let (dt, dx) = (Multi::generator(&t), Multi::generator(&x));
    let at = Some(&t);
    assert_eq!(dt.add(&dx).standard_part_at(at), Some(dt.clone()));
    assert_eq!(div(&dt.add(&dx), &dt).standard_part_at(at), Some(n(1)));
    assert_eq!(n(3).add(&dt.mul(&dt)).add(&dt.mul(&dx)).standard_part_at(at), Some(n(3).add(&dt.mul(&dt))));
    assert_eq!(div(&dt, &dx).standard_part_at(at), None);
    assert!(dx.is_infinitesimal_at(at));
    assert!(!dt.mul(&dt).is_infinitesimal_at(at), "dt^2 is an ordinary number from dt's standpoint");
    assert!(dt.recip().unwrap().is_limited_at(at));
    assert!(!div(&dt, &dx).is_limited_at(at));
    let from_dx = Some(&x);
    assert_eq!(dt.add(&dx).standard_part_at(from_dx), Some(dt.add(&dx)), "every value of both is ordinary from dx");
    assert!(!dx.is_infinitesimal_at(from_dx));
}

#[test]
fn a_third_infinitesimal_lies_below_the_second() {
    let t = scale("dt");
    let x = below("dx", &t);
    let y = below("dy", &x);
    let (dt, dx, dy) = (Multi::generator(&t), Multi::generator(&x), Multi::generator(&y));
    assert_eq!(pow(&dx, 50).sub(&dy).signum(), 1, "dy < dx^50");
    assert_eq!(dy.signum(), 1);
    let z = div(&div(&dy, &dx), &dt);
    assert!(z.is_infinitesimal(), "dy/(dx·dt) is infinitesimal");
    assert_eq!(z.signum(), 1);
    assert_eq!(dt.add(&dy).render(), "dt + dy");
    assert_eq!(dt.add(&dx).add(&dy).render(), "dt + dx + dy");
}

#[test]
fn an_infinitesimal_declared_above_later_is_the_larger() {
    let t = scale("dt");
    let h = above("h", &t);
    let (dt, hh) = (Multi::generator(&t), Multi::generator(&h));
    assert_eq!(hh.add(&dt).top().unwrap().name, "dt");
    assert_eq!(hh.add(&dt).render(), "h + dt");
    assert_eq!(pow(&hh, 1000).sub(&dt).signum(), 1, "dt < h^1000");
}

#[test]
fn a_value_prints_as_a_fraction_of_polynomials_in_its_infinitesimals() {
    let Two { dt, dx } = two();
    let cases = [
        (dt.add(&dx), "dt + dx"),
        (dx.sub(&dt), "-dt + dx"),
        (dt.sub(&dx), "dt - dx"),
        (div(&dx, &dt), "dx/dt"),
        (div(&dt, &dx), "dt/dx"),
        (dt.add(&dx).recip().unwrap(), "1/(dt + dx)"),
        (div(&dt.add(&dx), &dt.sub(&dx)), "(dt + dx)/(dt - dx)"),
        (div(&dx, &dt.mul(&n(2))), "dx/(2·dt)"),
        (dx.sub(&dt).recip().unwrap(), "-1/(dt - dx)"),
        (n(1).add(&dt).mul(&dx), "(1 + dt)·dx"),
        (n(1).add(&dt).mul(&dx).neg(), "-(1 + dt)·dx"),
        (n(3).mul(&dt).mul(&dx), "3·dt·dx"),
        (dt.add(&dx).mul(&dt.sub(&dx)), "dt^2 - dx^2"),
        (div(&dx.mul(&dx), &dt), "dx^2/dt"),
        (dt.mul(&dx).recip().unwrap(), "1/(dt·dx)"),
        (div(&n(1).add(&dx), &n(1).add(&dt)), "(1 + dx)/(1 + dt)"),
        (div(&dx, &n(3)).add(&div(&dt, &n(2))), "(1/2)·dt + (1/3)·dx"),
        (div(&dx, &n(3)).add(&div(&n(1), &dt.mul(&n(2)))), "(3 + 2·dt·dx)/(6·dt)"),
    ];
    for (value, text) in cases {
        assert_eq!(value.render(), text);
    }
}

#[test]
fn a_root_of_a_product_of_powers_is_exact() {
    let Two { dt, dx } = two();
    let r = dt.mul(&dx).root(2).unwrap();
    assert_eq!(r.render(), "√dt·√dx");
    assert_eq!(pow(&r, 2), dt.mul(&dx));
    let s = n(4).mul(&dt).mul(&dt).mul(&dx).root(2).unwrap();
    assert_eq!(s.render(), "2·dt·√dx");
    assert_eq!(div(&dt, &dx).root(3).unwrap().render(), "∛dt/∛dx");
    assert!(dt.mul(&dx).neg().root(2).unwrap_err().contains("square root of a negative number"));
    assert_eq!(dt.mul(&dx).neg().root(3).unwrap(), dt.mul(&dx).root(3).unwrap().neg());
}

#[test]
fn a_power_past_the_degree_bound_is_refused_by_its_infinitesimal() {
    let Two { dt, dx } = two();
    let err = dt.add(&dx).pow(5000).unwrap_err();
    assert_eq!(err.describe(), "the power would have degree 5000 in dx; one power reaches at most degree 4,096");
    assert_eq!(
        dt.add(&dx).pow(-2).unwrap(),
        dt.add(&dx).mul(&dt.add(&dx)).recip().unwrap()
    );
}

/// The engine's copy of an infinitesimal for one residue class of its reciprocal is that infinitesimal.
#[test]
fn a_residue_class_of_an_infinitesimal_is_that_infinitesimal() {
    let t = scale("dt");
    let x = below("dx", &t);
    let class = Arc::new(t.with_residues(ResidueType::congruent(0, 2)));
    assert!(class.same_infinitesimal(&t));
    let on_class = Multi::generator(&class).add(&Multi::generator(&x));
    let declared = Multi::generator(&t).add(&Multi::generator(&x));
    assert_eq!(on_class, declared);
    assert_eq!(class.order(&x), Some(Ordering::Greater));
}

#[test]
fn a_value_travels_by_its_canonical_bytes() {
    let Two { dt, dx } = two();
    let values = [
        dt.add(&dx),
        dt.add(&dx).recip().unwrap(),
        dt.mul(&dx).root(2).unwrap(),
        div(&dx, &dt.mul(&n(2))),
    ];
    for v in values {
        let bytes = v.to_bytes();
        let back = Multi::from_bytes(&bytes).unwrap();
        assert_eq!(back, v);
        assert_eq!(back.render(), v.render());
        let mut longer = bytes.clone();
        longer.push(0);
        assert_eq!(Multi::from_bytes(&longer), None, "trailing bytes are refused");
        assert_eq!(Multi::from_bytes(&bytes[..bytes.len() - 1]), None, "a cut encoding is refused");
    }
}

/// An encoding that is no value's one form is refused: a coefficient involving the smaller
/// infinitesimal, an unreduced fraction, a root of the infinitesimal finer than the value needs.
#[test]
fn a_non_canonical_encoding_is_refused() {
    let t = scale("dt");
    let x = below("dx", &t);
    let (dt, dx) = (Multi::generator(&t), Multi::generator(&x));
    let forged = |scale: &Arc<Scale>, q: u32, num: Vec<Multi>, den: Vec<Multi>| {
        Multi::Over {
            scale: scale.clone(),
            q,
            num: PolyOver::from(num),
            den: PolyOver::from(den),
        }
        .to_bytes()
    };
    let inverted = forged(&t, 1, vec![dx.clone(), n(1)], vec![n(1)]);
    assert_eq!(Multi::from_bytes(&inverted), None, "a coefficient of dt involving dx");
    let unreduced = forged(&x, 1, vec![n(0), dt.clone()], vec![n(0), n(1)]);
    assert_eq!(Multi::from_bytes(&unreduced), None, "dt·dx/dx");
    let too_fine = forged(&x, 2, vec![dt.clone(), n(0), n(1)], vec![n(1)]);
    assert_eq!(Multi::from_bytes(&too_fine), None, "dt + dx on the grid of √dx");
    let canonical = forged(&x, 1, vec![dt.clone(), n(1)], vec![n(1)]);
    assert_eq!(Multi::from_bytes(&canonical), Some(dt.add(&dx)));
}

/// A polynomial in the reciprocals is an integer when its fraction vanishes on every class its scales'
/// facts admit: `1/(2·dt)` is one where `⌈1/dt⌉` is even and not where nothing is known.
#[test]
fn a_polynomial_in_the_reciprocals_is_an_integer_where_its_facts_make_it_one() {
    let t = scale("dt");
    let x = below("dx", &t);
    let even = Arc::new(t.with_residues(ResidueType::congruent(0, 2)));
    let recip = |s: &Arc<Scale>| Multi::generator(s).recip().unwrap();
    assert!(recip(&t).add(&recip(&x)).is_integral());
    assert!(recip(&t).mul(&recip(&x)).is_integral());
    let half = recip(&t).mul(&Multi::from_i64(1).mul(&n(2).recip().unwrap()));
    assert!(!half.add(&recip(&x)).is_integral(), "1/(2·dt) is no integer on every class");
    let half_even = recip(&even).mul(&n(2).recip().unwrap());
    assert!(half_even.add(&recip(&x)).is_integral(), "1/(2·dt) is one where ⌈1/dt⌉ is even");
    assert!(!recip(&t).add(&Multi::generator(&x)).is_integral(), "dx is no reciprocal");
    assert!(!Multi::generator(&t).mul(&recip(&x)).is_integral(), "dt/dx is no polynomial in the reciprocals");
}

/// Two copies of one infinitesimal combine with the facts both state.
#[test]
fn a_value_on_a_class_combined_with_one_on_every_class_is_on_the_class() {
    let t = scale("dt");
    let x = below("dx", &t);
    let even = Arc::new(t.with_residues(ResidueType::congruent(0, 2)));
    let recip = |s: &Arc<Scale>| Multi::generator(s).recip().unwrap();
    let on_class = recip(&even).mul(&n(2).recip().unwrap());
    let everywhere = recip(&t).mul(&recip(&x));
    assert!(everywhere.add(&on_class).is_integral(), "1/(dt·dx) + 1/(2·dt) on the even class");
    assert!(on_class.add(&everywhere).is_integral(), "whichever comes first");
}

/// Two copies of one infinitesimal, each on a class of its own, combine on the class both state:
/// `1/(2·dt)` where `⌈1/dt⌉` is even and `1/(3·dt)` where it is divisible by 3 add to `5/(6·dt)` where
/// it is divisible by 6 — an integer there, and on neither class alone.
#[test]
fn values_on_two_classes_combine_on_the_class_both_state() {
    let t = scale("dt");
    let x = below("dx", &t);
    let even = Arc::new(t.with_residues(ResidueType::congruent(0, 2)));
    let thirds = Arc::new(t.with_residues(ResidueType::congruent(0, 3)));
    let recip = |s: &Arc<Scale>| Multi::generator(s).recip().unwrap();
    let five_sixths = |s: &Arc<Scale>| recip(s).mul(&n(5)).mul(&n(6).recip().unwrap()).add(&recip(&x));
    assert!(!five_sixths(&even).is_integral(), "5/(6·dt) is no integer where ⌈1/dt⌉ is only even");
    assert!(!five_sixths(&thirds).is_integral(), "nor where it is only divisible by 3");
    let half = recip(&even).mul(&n(2).recip().unwrap());
    let third = recip(&thirds).mul(&n(3).recip().unwrap());
    assert!(half.add(&recip(&x)).add(&third).is_integral(), "1/(2·dt) + 1/dx + 1/(3·dt) where it is divisible by 6");
    assert!(third.add(&recip(&x)).add(&half).is_integral(), "whichever comes first");
}

/// An infinite integer of several prints as a sum of reciprocals, the terms most infinite in the
/// smallest infinitesimal first; a value with any other denominator has no such form.
#[test]
fn a_polynomial_in_the_reciprocals_prints_as_a_sum_of_them() {
    let t = scale("dt");
    let x = below("dx", &t);
    let recip = |s: &Arc<Scale>| Multi::generator(s).recip().unwrap();
    let (a, b) = (recip(&t), recip(&x));
    assert_eq!(a.add(&b).render_reciprocals().unwrap(), "1/dx + 1/dt");
    let square = a.add(&b).mul(&a.add(&b));
    assert_eq!(square.render_reciprocals().unwrap(), "1/dx^2 + 2/(dt·dx) + 1/dt^2");
    let mixed = a.mul(&b).add(&a.mul(&n(2).recip().unwrap())).sub(&n(3).recip().unwrap());
    assert_eq!(mixed.render_reciprocals().unwrap(), "1/(dt·dx) + 1/(2·dt) - 1/3");
    assert!(a.add(&Multi::generator(&x)).render_reciprocals().is_none());
}

/// A value read on a residue class travels with the facts its class adds, so an infinite integer of
/// several there is one again where it arrives; a forged class the declaration does not admit is refused.
#[test]
fn a_value_on_a_class_travels_with_its_class() {
    let t = scale("dt");
    let x = below("dx", &t);
    let even = Arc::new(t.with_residues(ResidueType::congruent(0, 2)));
    let recip = |s: &Arc<Scale>| Multi::generator(s).recip().unwrap();
    let value = recip(&t).mul(&recip(&x)).add(&recip(&even).mul(&n(2).recip().unwrap()));
    assert!(value.is_integral());
    let back = Multi::from_bytes(&value.to_bytes()).unwrap();
    assert!(back.is_integral(), "the class's facts travel with the value");
    assert_eq!(back, value);
    assert_eq!(back.to_bytes(), value.to_bytes());
    let everywhere = recip(&t).add(&recip(&x));
    assert_eq!(Multi::from_bytes(&everywhere.to_bytes()).unwrap().to_bytes(), everywhere.to_bytes());
}
