//! The generator of fabric-lowerable LOGOS programs: bounded, terminating, VM-error-free function-form programs
//! in the shape the whole-program engine lowers (`## To f (n: Int) -> Int:` with while-loops, int arithmetic,
//! if/else, nested loops, Break, and self-recursion). Deterministic by seed (splitmix64), so a failure reproduces
//! exactly. Every suite that needs a corpus of such programs draws from this one generator.
#![allow(dead_code)]

/// splitmix64 — the deterministic mixer the soft-core fuzz uses; no `rand`/time, so a failure reproduces exactly.
pub struct Rng {
    state: u64,
}
impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng {
            state: seed.wrapping_add(0x9E37_79B9_7F4A_7C15),
        }
    }
    pub fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    /// pick one of `xs`.
    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len() as u64) as usize]
    }
}

/// The modulus every accumulation is reduced by — keeps all values well inside i64, so the engine's i64 registers
/// and the arbitrary-precision VM MUST agree (a mismatch is a genuine miscompile, never an overflow artifact).
pub const MOD: &str = "1000000007";

/// A random arithmetic RHS over the loop variable `i` and the accumulator `acc` (the current value). Every leaf is
/// bounded and every `/`/`%` divisor is made nonzero by `(… % k) + 1`, so the whole expression is VM-error-free
/// and stays within the `% MOD` reduction the caller wraps it in.
pub fn gen_expr(rng: &mut Rng, depth: u32) -> String {
    if depth == 0 || rng.below(3) == 0 {
        // A leaf: the loop variable, the accumulator, or a small constant.
        return match rng.below(4) {
            0 => "i".into(),
            1 => "acc".into(),
            2 => (rng.below(97) as i64 + 1).to_string(),
            _ => format!("(i + {})", rng.below(50) + 1),
        };
    }
    let l = gen_expr(rng, depth - 1);
    // `+ - *` compose freely under the outer `% MOD`; `/ %` use a guaranteed-nonzero divisor.
    match rng.below(5) {
        0 => format!("({l} + {})", gen_expr(rng, depth - 1)),
        1 => format!("({l} - {})", gen_expr(rng, depth - 1)),
        2 => format!("({l} * {})", rng.below(31) + 1),
        3 => format!(
            "({l} / (({}) % {} + 1))",
            gen_expr(rng, depth - 1),
            rng.below(20) + 2
        ),
        _ => format!(
            "({l} % (({}) % {} + 1))",
            gen_expr(rng, depth - 1),
            rng.below(20) + 2
        ),
    }
}

pub const CMPS: &[&str] = &[
    "is greater than",
    "is less than",
    "is at most",
    "is at least",
    "is equal to",
    "is not",
];

/// A random comparison of the loop variable / accumulator against a small constant — the branch predicate.
pub fn gen_cond(rng: &mut Rng) -> String {
    let lhs = match rng.below(3) {
        0 => "i".to_string(),
        1 => "acc".to_string(),
        _ => "(i + acc)".to_string(),
    };
    let cmp = rng.pick(CMPS);
    let rhs = match rng.below(2) {
        0 => (rng.below(50) as i64).to_string(),
        _ => format!("i {} 2", if rng.below(2) == 0 { "*" } else { "%" }),
    };
    format!("{lhs} {cmp} {rhs}")
}

/// Randomize the body of the main loop: a mandatory accumulate, plus optionally an if/else that steers the
/// accumulate, a nested inner loop (bounded by the same counter), or a `Break`. Emits 8-space-indented lines
/// (inside `While`, which is inside the function). Every path keeps `acc` reduced by `% MOD`.
pub fn gen_loop_body(rng: &mut Rng) -> String {
    let mut s = String::new();
    match rng.below(4) {
        // 0 — plain accumulate
        0 => {
            s.push_str(&format!(
                "        Set acc to ({} + acc) % {MOD}.\n",
                gen_expr(rng, 2)
            ));
        }
        // 1 — if/else steering the accumulate
        1 => {
            s.push_str(&format!("        If {}:\n", gen_cond(rng)));
            s.push_str(&format!(
                "            Set acc to ({} + acc) % {MOD}.\n",
                gen_expr(rng, 2)
            ));
            s.push_str("        Otherwise:\n");
            s.push_str(&format!(
                "            Set acc to ({} + acc) % {MOD}.\n",
                gen_expr(rng, 2)
            ));
        }
        // 2 — a nested inner loop bounded by the outer counter (j from 1 to i)
        2 => {
            s.push_str("        Let mutable j be 1.\n");
            s.push_str("        While j is at most i:\n");
            s.push_str(&format!(
                "            Set acc to (acc + {} * j) % {MOD}.\n",
                rng.below(7) + 1
            ));
            s.push_str("            Set j to j + 1.\n");
        }
        // 3 — a conditional Break, then accumulate
        _ => {
            s.push_str(&format!("        If {}:\n", gen_cond(rng)));
            s.push_str("            Break.\n");
            s.push_str(&format!(
                "        Set acc to ({} + acc) % {MOD}.\n",
                gen_expr(rng, 2)
            ));
        }
    }
    s
}

/// Generate one random ITERATIVE function-form program: `## To f (n: Int) -> Int:` with a `while i is at most n`
/// loop whose body is randomized (`gen_loop_body`), returning the reduced accumulator. Terminating (single
/// bounded counter) and VM-error-free (nonzero divisors, `% MOD` reductions) by construction — the benchmark
/// shape `program_cfg_fabric` lowers.
pub fn gen_iterative(rng: &mut Rng) -> String {
    let mut s = String::new();
    s.push_str("## To f (n: Int) -> Int:\n");
    s.push_str("    Let mutable acc be 0.\n");
    s.push_str(&format!("    Let mutable i be {}.\n", rng.below(2)));
    s.push_str("    While i is at most n:\n");
    s.push_str(&gen_loop_body(rng));
    s.push_str("        Set i to i + 1.\n");
    s.push_str(&format!("    Return acc % {MOD}.\n"));
    s
}

/// Generate one random SELF-RECURSIVE function-form program: `f(n) = base if n<=0 else <expr in n> + f(n-1)` —
/// a genuine `Call` back into `f`'s entry (the frame stack). Depth is exactly the argument (bounded by the small
/// args we feed), and every value is `% MOD`-reduced. The benchmark shape `program_calls_fabric` lowers.
pub fn gen_recursive(rng: &mut Rng) -> String {
    let mut s = String::new();
    s.push_str("## To f (n: Int) -> Int:\n");
    s.push_str("    If n is at most 0:\n");
    s.push_str(&format!("        Return {}.\n", rng.below(10)));
    // A step term over `n` (no `acc`/`i` here — recursion has only the param `n` in scope). Keep it bounded.
    let step = match rng.below(4) {
        0 => "n".to_string(),
        1 => format!("(n * {})", rng.below(7) + 1),
        2 => format!("(n + {})", rng.below(50) + 1),
        _ => format!("(n % {} + 1)", rng.below(20) + 2),
    };
    s.push_str(&format!("    Return ({step} + f(n - 1)) % {MOD}.\n"));
    s
}

/// The kinds of programs the generator produces — a breadth axis so a green run isn't just one shape.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Shape {
    Iterative,
    Recursive,
}

pub fn gen(rng: &mut Rng) -> (String, Shape) {
    // Bias toward iterative (the richer control-flow surface) but keep a real recursion population.
    if rng.below(3) == 0 {
        (gen_recursive(rng), Shape::Recursive)
    } else {
        (gen_iterative(rng), Shape::Iterative)
    }
}

/// A random 2-arg recursive function whose recursive-call step computes a MULTIPLY over the args — the exact
/// "value live across a Call" shape that produced bug #5 on single-arg calls, now stressed on MULTI-ARG calls
/// (the arg-passing path where 2 of the 5 bugs already lived). `a` counts down (bounded depth); every value is
/// `% 1000000007`-reduced (bounded, VM-error-free).
pub fn gen_recursive_2arg(rng: &mut Rng) -> String {
    let k = (rng.below(50) + 1) as i64;
    let base = if rng.below(2) == 0 {
        "b".to_string()
    } else {
        format!("(b + {}) % 1000000007", rng.below(1000))
    };
    let step = match rng.below(3) {
        0 => format!("(b + a * {k}) % 1000000007"),
        1 => format!("(b * {k} + a) % 1000000007"),
        _ => format!("(a * a + b) % 1000000007"),
    };
    format!("## To f (a: Int, b: Int) -> Int:\n    If a is at most 0:\n        Return {base}.\n    Return f(a - 1, {step}).\n")
}
