//! Property/fuzz over RANDOM module graphs: the multi-file assembler must be
//! total — every generated project either assembles + parses + interprets to a
//! well-defined result, OR fails with a clean, non-empty diagnostic. It must
//! NEVER panic and NEVER emit source that panics the parser/interpreter.
//!
//! Determinism: the generator is a pure splitmix64 keyed by the loop index, so
//! any failure is permanently reproducible from its printed seed + project.

#![cfg(not(target_arch = "wasm32"))]

use std::fs;

use tempfile::tempdir;

use logicaffeine_compile::analysis::assemble_project;

/// splitmix64 — a tiny, dependency-free deterministic PRNG.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(0x1234_5678))
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next() % n
        }
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
}

/// Generate a random legal-ish module graph as `(filename, body)` pairs.
///
/// IMPORT edges may be cyclic (any module → any other, and self) to exercise the
/// assembler's cycle/diamond handling; CALL edges form a strict DAG (a function
/// only calls LOWER-indexed functions) so every generated program terminates —
/// the fuzzer targets assembler totality, not runtime non-termination. Module 0
/// is a clean leaf base; a controlled fraction of the rest are DEGENERATE (empty,
/// prose-only, duplicate-import, self-import).
fn gen_project(seed: u64) -> Vec<(String, String)> {
    let mut r = Rng::new(seed);
    let n = 2 + r.below(4) as usize; // 2..=5 modules
    let mut files: Vec<(String, String)> = Vec::new();

    // Module 0 — the always-terminating leaf base.
    files.push((
        "m0.lg".to_string(),
        "# M0\n\n## To f0 (n: Int) -> Int:\n    Return n plus 1.\n".to_string(),
    ));

    for i in 1..n {
        // Import edges: any other module (cyclic allowed) — tests the DFS guard.
        let mut imports: Vec<usize> = Vec::new();
        for j in 0..n {
            if j != i && r.chance(35) {
                imports.push(j);
            }
        }

        let degen = r.below(24); // ~4% chance of each degenerate flavor
        let body = if degen == 0 {
            format!("# M{i}\n") // empty module (header only)
        } else if degen == 1 {
            format!("# M{i}\n\nJust some prose. No code here.\n") // prose-only
        } else {
            let mut s = format!("# M{i}\n\n");
            for &j in &imports {
                s.push_str(&format!("Import M{j}.\n"));
            }
            if degen == 2 && !imports.is_empty() {
                s.push_str(&format!("Import M{}.\n", imports[0])); // duplicate import
            }
            if degen == 3 {
                s.push_str(&format!("Import M{i}.\n")); // self-import
            }
            s.push('\n');
            // CALL only a lower-indexed imported module → DAG → always terminates.
            let lower: Vec<usize> = imports.iter().copied().filter(|&j| j < i).collect();
            let call = if !lower.is_empty() && r.chance(70) {
                let j = lower[r.below(lower.len() as u64) as usize];
                format!("f{j}(n) plus {}", 1 + r.below(9))
            } else {
                format!("n plus {}", 1 + r.below(9))
            };
            s.push_str(&format!(
                "## To f{i} (n: Int) -> Int:\n    Return {call}.\n"
            ));
            s
        };
        files.push((format!("m{i}.lg"), body));
    }

    // Entry: import EVERY module (so the whole graph is assembled every time),
    // call the guaranteed-terminating base `f0`.
    let mut main = String::from("# App\n\n");
    for i in 0..n {
        main.push_str(&format!("Import M{i}.\n"));
    }
    main.push_str("\n## Main\nShow f0(1).\n");
    files.push(("main.lg".to_string(), main));
    files
}

fn render(files: &[(String, String)]) -> String {
    files
        .iter()
        .map(|(name, body)| format!("=== {name} ===\n{body}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The property: over many random graphs, `assemble_project` is total — Ok source
/// interprets without panic, Err is a clean message, and nothing ever panics.
#[test]
fn assembler_is_total_over_random_module_graphs() {
    // Silence panic backtraces from the intentionally-caught inner panics so a
    // GREEN run stays quiet; a real failure re-panics below with full context.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let mut failures: Vec<(u64, String)> = Vec::new();
    for seed in 0..500u64 {
        let files = gen_project(seed);
        let dir = tempdir().unwrap();
        for (name, body) in &files {
            fs::write(dir.path().join(name), body).unwrap();
        }
        let main = dir.path().join("main.lg");

        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match assemble_project(&main) {
                Ok(asm) => {
                    // The emitted source must not panic the interpreter (result
                    // may be Ok or a clean engine error — both are fine).
                    let _ = futures::executor::block_on(
                        logicaffeine_compile::interpret_for_ui_with_args(
                            &asm.source,
                            &["main".to_string()],
                        ),
                    );
                }
                Err(e) => {
                    assert!(!e.trim().is_empty(), "assembly error must be non-empty");
                    assert!(
                        !e.contains("panicked"),
                        "assembly error must not be a panic: {e}"
                    );
                }
            }
        }));

        if outcome.is_err() {
            failures.push((seed, render(&files)));
        }
    }

    std::panic::set_hook(prev);

    assert!(
        failures.is_empty(),
        "the assembler panicked on {} seed(s). First failure:\nseed {}\n{}",
        failures.len(),
        failures[0].0,
        failures[0].1
    );
}
