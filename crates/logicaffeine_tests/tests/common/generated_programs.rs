//! The programs the Futamura locks generate: a seeded source (SplitMix64) and a family of programs over `Int`. Shared
//! (via `#[path = "common/generated_programs.rs"]`) by `futamura_arguments_lock`, whose programs read `n` from their
//! first argument, and `futamura_textbook_lock`, whose offline projections take it as a known value.
#![allow(dead_code)]

/// SplitMix64: the generated programs' deterministic source.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

pub const GENERATED_VARS: [&str; 3] = ["a", "b", "c"];

pub fn generated_leaf(rng: &mut Rng) -> String {
    match rng.below(3) {
        0 => rng.below(9).to_string(),
        1 => "n".to_string(),
        _ => rng.pick(&GENERATED_VARS).to_string(),
    }
}

pub fn generated_expr(rng: &mut Rng) -> String {
    match rng.below(4) {
        0 => generated_leaf(rng),
        1 => format!("helper({}, {})", generated_leaf(rng), generated_leaf(rng)),
        _ => format!("({} {} {})", generated_leaf(rng), rng.pick(&["+", "-", "*"]), generated_leaf(rng)),
    }
}

pub fn generated_cond(rng: &mut Rng) -> String {
    format!(
        "{} is {} {}",
        generated_expr(rng),
        rng.pick(&["less than", "greater than", "at most", "at least"]),
        generated_expr(rng)
    )
}

pub fn generated_statements(rng: &mut Rng, out: &mut Vec<String>, count: usize, indent: usize, loops: usize) {
    let pad = "    ".repeat(indent);
    for _ in 0..count {
        match rng.below(if indent < 2 { 6 } else { 3 }) {
            0 | 1 => out.push(format!("{pad}Set {} to {}.", rng.pick(&GENERATED_VARS), generated_expr(rng))),
            2 => out.push(format!("{pad}Set {} to down({} % 6).", rng.pick(&GENERATED_VARS), rng.pick(&GENERATED_VARS))),
            3 => {
                out.push(format!("{pad}If {}:", generated_cond(rng)));
                let then_count = 1 + rng.below(2) as usize;
                generated_statements(rng, out, then_count, indent + 1, loops);
                out.push(format!("{pad}Otherwise:"));
                let else_count = 1 + rng.below(2) as usize;
                generated_statements(rng, out, else_count, indent + 1, loops);
            }
            _ => {
                let counter = format!("k{loops}");
                out.push(format!("{pad}Let mutable {counter} be n % 30."));
                out.push(format!("{pad}While {counter} is greater than 0:"));
                let body_count = 1 + rng.below(2) as usize;
                generated_statements(rng, out, body_count, indent + 1, loops + 1);
                if rng.below(3) == 0 {
                    out.push(format!("{pad}    If {}:", generated_cond(rng)));
                    out.push(format!("{pad}        Break."));
                }
                out.push(format!("{pad}    Set {counter} to {counter} - 1."));
            }
        }
    }
}

/// A program over `Int` whose `n` is `n` (an expression: its first argument, `parseInt(item 2 of args())`, or a known
/// value): assignments, arithmetic that may overflow (a failure the residual must report where the program does),
/// branches, loops whose counter falls to zero, a `Break` under a condition, a helper and a structurally decreasing
/// recursion, every variable shown.
pub fn generated_program(seed: u64, n: &str) -> String {
    let mut rng = Rng(seed);
    let ops = ["+", "-", "*"];
    let mut out = vec!["## To helper (x: Int) and (y: Int) -> Int:".to_string()];
    out.push(format!("    Return x {} y {} {}.", rng.pick(&ops), rng.pick(&ops), rng.below(5) + 1));
    out.push(String::new());
    out.push("## To down (k: Int) -> Int:\n    If k is at most 0:\n        Return 0.".to_string());
    out.push(format!("    Return k {} down(k - 1).", rng.pick(&["+", "*"])));
    out.push(String::new());
    out.push(format!("## Main\nLet n be {n}."));
    for v in GENERATED_VARS {
        out.push(format!("Let mutable {v} be {}.", rng.below(7)));
    }
    let count = 3 + rng.below(4) as usize;
    generated_statements(&mut rng, &mut out, count, 0, 0);
    for v in GENERATED_VARS {
        out.push(format!("Show {v}."));
    }
    out.join("\n") + "\n"
}
