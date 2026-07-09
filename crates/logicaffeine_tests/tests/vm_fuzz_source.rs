//! Tier-2 differential fuzzing over REAL LOGOS SOURCE TEXT: the generated
//! program goes through the full shared front-end (lexer → MWE → discovery →
//! parser) before each engine runs it, so the parser sees exactly what both
//! engines execute. Deterministic by seed; failures print the full source.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        SplitMix64 { state: seed.wrapping_add(0x9E37_79B9_7F4A_7C15) }
    }
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Generate a well-formed LOGOS source program (verified surface syntax only).
fn gen_source(seed: u64) -> String {
    let mut rng = SplitMix64::new(seed);
    let mut src = String::new();

    // Optionally a helper function.
    let has_fn = rng.below(2) == 0;
    if has_fn {
        src.push_str("## To triple (n: Int) -> Int:\n    Return n * 3.\n\n");
    }

    src.push_str("## Main\n");
    let var_count = 2 + rng.below(3);
    for k in 0..var_count {
        src.push_str(&format!("Let mutable v{k} be {}.\n", rng.below(9)));
    }
    let vars: Vec<String> = (0..var_count).map(|k| format!("v{k}")).collect();
    let pick = |rng: &mut SplitMix64| vars[rng.below(vars.len() as u64) as usize].clone();

    let stmts = 4 + rng.below(8);
    for _ in 0..stmts {
        match rng.below(8) {
            0 | 1 => {
                let v = pick(&mut rng);
                let a = pick(&mut rng);
                let op = ["+", "-", "*"][rng.below(3) as usize];
                src.push_str(&format!("Set {v} to {a} {op} {}.\n", rng.below(5)));
            }
            2 => {
                let v = pick(&mut rng);
                src.push_str(&format!("Set {v} to {} % {}.\n", pick(&mut rng), 1 + rng.below(5)));
            }
            3 => {
                let v = pick(&mut rng);
                let w = pick(&mut rng);
                src.push_str(&format!(
                    "If {v} is greater than {}:\n    Set {w} to {w} + 1.\nOtherwise:\n    Set {w} to {w} - 1.\n",
                    rng.below(8)
                ));
            }
            4 => {
                let bound = 2 + rng.below(3);
                let v = pick(&mut rng);
                src.push_str(&format!(
                    "Let mutable lw{bound} be 0.\nWhile lw{bound} is less than {bound}:\n    Set {v} to {v} + 2.\n    Set lw{bound} to lw{bound} + 1.\n"
                ));
            }
            5 if has_fn => {
                let v = pick(&mut rng);
                src.push_str(&format!("Set {v} to triple({}).\n", rng.below(6)));
            }
            6 => {
                src.push_str(&format!("Show {}.\n", pick(&mut rng)));
            }
            _ => {
                let a = pick(&mut rng);
                let b = pick(&mut rng);
                src.push_str(&format!("Show {a} + {b}.\n"));
            }
        }
    }
    for v in &vars {
        src.push_str(&format!("Show {v}.\n"));
    }
    src
}

#[test]
fn vm_fuzz_source_differential() {
    for seed in 0..500u64 {
        let src = gen_source(seed);
        let tw = tw_outcome(&src);
        let vm = vm_outcome(&src);
        assert_eq!(
            norm(&vm.output),
            norm(&tw.output),
            "SEED={seed} output diverged for source:\n{src}\nvm: {:?}\ntw: {:?}",
            vm,
            tw
        );
        assert_eq!(vm.error, tw.error, "SEED={seed} error diverged for source:\n{src}");
    }
}

#[test]
fn vm_fuzz_source_is_deterministic() {
    for seed in [0u64, 7, 99] {
        assert_eq!(gen_source(seed), gen_source(seed));
    }
}
