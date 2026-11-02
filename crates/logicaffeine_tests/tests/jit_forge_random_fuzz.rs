//! FORGE RANDOM DIFFERENTIAL FUZZER — the copy-and-patch forge tier is hand-written codegen with no
//! correct-by-construction guarantee, and nothing else in the suite fuzzes it (`assert_all_tiers` runs
//! tw/VM/AOT, never the process-wide forge). This generates hundreds of RANDOM list-parameter programs
//! from a deterministic LCG — random list length (incl. empty), random element values, random thresholds
//! and iteration counts, across a family of `length of`-bounded / early-return / mutation / nested-loop /
//! cross-referenced shapes — runs each HOT (≥ tier-up) through the forge tier, and asserts byte-identical
//! output to the tree-walker. A data- or shape-dependent forge miscompile (the class that let hot
//! `length of <list-param>` callees return 0) surfaces here as a non-zero divergence count. Deterministic:
//! same seed → same corpus, so a failure reproduces exactly.

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

/// A tiny deterministic LCG (glibc constants) — reproducible, no external RNG.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(1103515245).wrapping_add(12345) & 0x7fff_ffff;
        self.0
    }
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next() as i64) % (hi - lo + 1)
    }
}

/// The list-param helper families the fuzzer draws from. Each takes a `Seq of Int` param and returns a
/// scalar; `{T}` is a random threshold substituted in. All read `length of xs` in some position.
const FAMILIES: &[&str] = &[
    // sum with length bound, no early return
    "## To helper (xs: Seq of Int, t: Int) -> Int:\n\
     \x20   Let n be length of xs.\n\
     \x20   Let mutable s be 0.\n\
     \x20   Let mutable i be 1.\n\
     \x20   While i is at most n:\n\
     \x20       Set s to s + (item i of xs).\n\
     \x20       Set i to i + 1.\n\
     \x20   Return s + t - t.",
    // first index >= threshold, early return out of a length-bounded loop
    "## To helper (xs: Seq of Int, t: Int) -> Int:\n\
     \x20   Let n be length of xs.\n\
     \x20   Let mutable i be 1.\n\
     \x20   While i is at most n:\n\
     \x20       If (item i of xs) is at least t:\n\
     \x20           Return i.\n\
     \x20       Set i to i + 1.\n\
     \x20   Return 0.",
    // count of elements strictly greater than threshold
    "## To helper (xs: Seq of Int, t: Int) -> Int:\n\
     \x20   Let n be length of xs.\n\
     \x20   Let mutable c be 0.\n\
     \x20   Let mutable i be 1.\n\
     \x20   While i is at most n:\n\
     \x20       If (item i of xs) is greater than t:\n\
     \x20           Set c to c + 1.\n\
     \x20       Set i to i + 1.\n\
     \x20   Return c.",
    // max via a length-bounded scan seeded from threshold
    "## To helper (xs: Seq of Int, t: Int) -> Int:\n\
     \x20   Let n be length of xs.\n\
     \x20   Let mutable m be t.\n\
     \x20   Let mutable i be 1.\n\
     \x20   While i is at most n:\n\
     \x20       If (item i of xs) is greater than m:\n\
     \x20           Set m to item i of xs.\n\
     \x20       Set i to i + 1.\n\
     \x20   Return m.",
    // nested-loop ascending-pair count (length bound in both loops)
    "## To helper (xs: Seq of Int, t: Int) -> Int:\n\
     \x20   Let n be length of xs.\n\
     \x20   Let mutable c be 0.\n\
     \x20   Let mutable i be 1.\n\
     \x20   While i is at most n:\n\
     \x20       Let mutable j be i + 1.\n\
     \x20       While j is at most n:\n\
     \x20           If (item i of xs) is less than (item j of xs):\n\
     \x20               Set c to c + 1.\n\
     \x20           Set j to j + 1.\n\
     \x20       Set i to i + 1.\n\
     \x20   Return c + t - t.",
    // length used directly in arithmetic (no loop over it) + one indexed read guarded on non-empty
    "## To helper (xs: Seq of Int, t: Int) -> Int:\n\
     \x20   Let n be length of xs.\n\
     \x20   If n is equal to 0:\n\
     \x20       Return t.\n\
     \x20   Return n * 1000 + (item 1 of xs) + (item n of xs).",
];

/// Build a hot driver: literal list `xs`, call `helper(xs, t)` 400× accumulating, `Show` the total.
fn program(family: &str, list: &[i64], t: i64) -> String {
    let elems = list
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let xs_decl = if list.is_empty() {
        "Let xs be a new Seq of Int.\n".to_string()
    } else {
        format!("Let xs be [{elems}].\n")
    };
    format!(
        "{family}\n## Main\n{xs_decl}Let mutable acc be 0.\nLet mutable k be 0.\n\
         While k is less than 400:\n\
         \x20   Set acc to acc + helper(xs, {t}).\n\
         \x20   Set k to k + 1.\nShow acc.\n"
    )
}

#[test]
fn forge_matches_tree_walker_on_random_list_param_programs() {
    let mut rng = Lcg(0x1234_5678);
    let mut diverged: Vec<String> = Vec::new();
    let rounds = 300;
    for round in 0..rounds {
        let family = FAMILIES[(rng.next() as usize) % FAMILIES.len()];
        // Random list: length 0..=16 (includes empty), values in a small band so thresholds land
        // both inside and outside the value range (exercises both branch directions).
        let len = rng.range(0, 16) as usize;
        let list: Vec<i64> = (0..len).map(|_| rng.range(-5, 20)).collect();
        let t = rng.range(-8, 25);
        let src = program(family, &list, t);
        let tw = tw_outcome_with_args(&src, &[]);
        let tier = ForgeTier::new();
        let forge = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        if tw.error.is_some() {
            diverged.push(format!(
                "round {round}: tree-walker ERRORED {:?}\n{src}",
                tw.error
            ));
            continue;
        }
        if forge.error.is_some() {
            diverged.push(format!(
                "round {round}: forge ERRORED {:?}\n{src}",
                forge.error
            ));
            continue;
        }
        if tw.output.trim() != forge.output.trim() {
            diverged.push(format!(
                "round {round}: forge={:?} tw={:?} (list={list:?} t={t})\n{src}",
                forge.output.trim(),
                tw.output.trim()
            ));
        }
    }
    assert!(
        diverged.is_empty(),
        "forge JIT diverged from the tree-walker on {}/{rounds} random programs:\n{}",
        diverged.len(),
        diverged.join("\n---\n")
    );
}

/// FLOAT-plane list-param families: `Seq of Float` params whose `length of vs` bound and `item i of vs`
/// reads flow through the float pin slots. Each returns an INT (index / count / bound) so the `Show` is
/// exact (no float-format nondeterminism). The pin refresh (`logos_rt_list_triple`) reads `.len()` /
/// `.as_mut_ptr()` type-agnostically, so this proves it on the float layout, not just Int.
const FLOAT_FAMILIES: &[&str] = &[
    // first index whose float element exceeds the (float) threshold — early return, length bound
    "## To helper (vs: Seq of Float, t: Float) -> Int:\n\
     \x20   Let n be length of vs.\n\
     \x20   Let mutable i be 1.\n\
     \x20   While i is at most n:\n\
     \x20       If (item i of vs) is greater than t:\n\
     \x20           Return i.\n\
     \x20       Set i to i + 1.\n\
     \x20   Return 0.",
    // count of float elements below the threshold — length bound, no early return
    "## To helper (vs: Seq of Float, t: Float) -> Int:\n\
     \x20   Let n be length of vs.\n\
     \x20   Let mutable c be 0.\n\
     \x20   Let mutable i be 1.\n\
     \x20   While i is at most n:\n\
     \x20       If (item i of vs) is less than t:\n\
     \x20           Set c to c + 1.\n\
     \x20       Set i to i + 1.\n\
     \x20   Return c.",
    // index of the running-maximum float — length bound with a carried float compare
    "## To helper (vs: Seq of Float, t: Float) -> Int:\n\
     \x20   Let n be length of vs.\n\
     \x20   If n is equal to 0:\n\
     \x20       Return 0.\n\
     \x20   Let mutable best be 1.\n\
     \x20   Let mutable i be 2.\n\
     \x20   While i is at most n:\n\
     \x20       If (item i of vs) is greater than (item best of vs):\n\
     \x20           Set best to i.\n\
     \x20       Set i to i + 1.\n\
     \x20   Return best + (t - t).",
];

/// Build a hot driver over a `Seq of Float` literal with a float threshold.
fn float_program(family: &str, list: &[i64], t_half: i64) -> String {
    // Deterministic float literals: each `v` renders as `v.5` (e.g. 3 → "3.5", -2 → "-2.5").
    let elems = list
        .iter()
        .map(|v| format!("{v}.5"))
        .collect::<Vec<_>>()
        .join(", ");
    let vs_decl = if list.is_empty() {
        "Let vs be a new Seq of Float.\n".to_string()
    } else {
        format!("Let vs be [{elems}].\n")
    };
    format!(
        "{family}\n## Main\n{vs_decl}Let mutable acc be 0.\nLet mutable k be 0.\n\
         While k is less than 400:\n\
         \x20   Set acc to acc + (helper(vs, {t_half}.5)).\n\
         \x20   Set k to k + 1.\nShow acc.\n"
    )
}

#[test]
fn forge_matches_tree_walker_on_random_float_list_param_programs() {
    let mut rng = Lcg(0x0bad_f00d);
    let mut diverged: Vec<String> = Vec::new();
    let rounds = 240;
    for round in 0..rounds {
        let family = FLOAT_FAMILIES[(rng.next() as usize) % FLOAT_FAMILIES.len()];
        let len = rng.range(0, 14) as usize;
        let list: Vec<i64> = (0..len).map(|_| rng.range(-6, 18)).collect();
        let t = rng.range(-9, 22);
        let src = float_program(family, &list, t);
        let tw = tw_outcome_with_args(&src, &[]);
        let tier = ForgeTier::new();
        let forge = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        if tw.error.is_some() {
            diverged.push(format!(
                "round {round}: tree-walker ERRORED {:?}\n{src}",
                tw.error
            ));
            continue;
        }
        if forge.error.is_some() {
            diverged.push(format!(
                "round {round}: forge ERRORED {:?}\n{src}",
                forge.error
            ));
            continue;
        }
        if tw.output.trim() != forge.output.trim() {
            diverged.push(format!(
                "round {round}: forge={:?} tw={:?} (list={list:?} t={t}.5)\n{src}",
                forge.output.trim(),
                tw.output.trim()
            ));
        }
    }
    assert!(
        diverged.is_empty(),
        "forge JIT diverged from the tree-walker on {}/{rounds} random FLOAT programs:\n{}",
        diverged.len(),
        diverged.join("\n---\n")
    );
}
