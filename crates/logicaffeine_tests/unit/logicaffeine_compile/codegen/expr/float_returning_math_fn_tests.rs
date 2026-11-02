use super::FLOAT_RETURNING_MATH_FNS;

/// The list MUST equal the set of `## To <name> … -> Real:` definitions in math.lg. If someone
/// adds a `-> Real` transcendental without listing it here, `complex(newFn(x), …)` would
/// mis-lower to the exact representation and lossily truncate the float part — this fails first.
#[test]
fn float_returning_list_mirrors_math_lg() {
    const MATH_LG: &str = include_str!("../../../../../logicaffeine_compile/assets/std/math.lg");
    let mut from_source: Vec<String> = MATH_LG
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let rest = line.strip_prefix("## To ")?;
            if !line.trim_end().ends_with("-> Real:") {
                return None;
            }
            // Function name is the token up to the first space or '('.
            let name: String = rest
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '(')
                .collect();
            (!name.is_empty()).then_some(name)
        })
        .collect();
    from_source.sort();
    from_source.dedup();

    let mut listed: Vec<String> = FLOAT_RETURNING_MATH_FNS
        .iter()
        .map(|s| s.to_string())
        .collect();
    listed.sort();

    // A duplicate can never equal the deduped `from_source`, and the assert_eq below would then
    // blame "drift" while naming nothing — the exact confusing failure two sessions adding the
    // same `-> Real` name would hit. Catch a repeat here, named.
    let mut seen = std::collections::HashSet::new();
    let dups: Vec<&str> = listed
        .iter()
        .map(|s| s.as_str())
        .filter(|n| !seen.insert(*n))
        .collect();
    assert!(
        dups.is_empty(),
        "FLOAT_RETURNING_MATH_FNS has duplicate entries {dups:?} — the list must be a set; \
             remove the repeat."
    );

    assert_eq!(
        from_source,
        listed,
        "FLOAT_RETURNING_MATH_FNS drifted from math.lg's `-> Real` surface.\n\
             In math.lg but not listed (would mis-lower `complex(fn(x), …)`): {:?}\n\
             Listed but not in math.lg (stale): {:?}",
        from_source
            .iter()
            .filter(|f| !listed.contains(f))
            .collect::<Vec<_>>(),
        listed
            .iter()
            .filter(|l| !from_source.contains(l))
            .collect::<Vec<_>>(),
    );
}

/// The `-> ComplexF` mirror. A `## To <name> … -> ComplexF:` that is not listed would make
/// `re(fn(z))`/`im(fn(z))` cast the complex through `as f64` (a non-primitive-cast compile
/// error) and complex arithmetic on its result mis-lower — this fails first.
#[test]
fn complexf_returning_list_mirrors_math_lg() {
    const MATH_LG: &str = include_str!("../../../../../logicaffeine_compile/assets/std/math.lg");
    let mut from_source: Vec<String> = MATH_LG
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let rest = line.strip_prefix("## To ")?;
            if !line.trim_end().ends_with("-> ComplexF:") {
                return None;
            }
            let name: String = rest
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '(')
                .collect();
            (!name.is_empty()).then_some(name)
        })
        .collect();
    from_source.sort();
    from_source.dedup();

    let mut listed: Vec<String> = super::COMPLEXF_RETURNING_MATH_FNS
        .iter()
        .map(|s| s.to_string())
        .collect();
    listed.sort();

    assert_eq!(
        from_source,
        listed,
        "COMPLEXF_RETURNING_MATH_FNS drifted from math.lg's `-> ComplexF` surface.\n\
             In math.lg but not listed (re/im would `as f64`-cast the complex): {:?}\n\
             Listed but not in math.lg (stale): {:?}",
        from_source
            .iter()
            .filter(|f| !listed.contains(f))
            .collect::<Vec<_>>(),
        listed
            .iter()
            .filter(|l| !from_source.contains(l))
            .collect::<Vec<_>>(),
    );
}
