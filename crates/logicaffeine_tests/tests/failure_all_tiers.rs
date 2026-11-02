//! Every program in `failure_corpus.in` stops with EXACTLY its reason, after EXACTLY its output, on
//! every tier: the tree-walker and VM (error text), the AOT-Rust tier and the C tier (stderr, exit
//! status 1). WASM runs the same corpus in `wasm_aot_lock.rs`. Each sweep reports every divergence.

mod common;
use common::{compile_logos, run_all_tiers};

const CORPUS: &[(&str, &str, &str, &str)] = include!("failure_corpus.in");

fn report(tier: &str, failures: Vec<String>) {
    assert!(
        failures.is_empty(),
        "{} of {} failure programs diverge on the {tier} tier:\n\n{}",
        failures.len(),
        CORPUS.len(),
        failures.join("\n\n")
    );
}

/// Compare a compiled binary's run against the corpus row.
fn judge(
    name: &str,
    src: &str,
    expected_out: &str,
    reason: &str,
    out: std::process::Output,
) -> Option<String> {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let code = out.status.code();
    let ok = stdout.trim() == expected_out
        && if reason.is_empty() {
            code == Some(0)
        } else {
            code == Some(1) && stderr.trim() == reason
        };
    (!ok).then(|| {
        format!(
            "{name}: stdout {:?}, stderr {:?}, status {code:?}; expected stdout {expected_out:?}, reason {reason:?}\n{src}",
            stdout.trim(),
            stderr.trim()
        )
    })
}

#[test]
fn the_corpus_is_broad() {
    assert!(CORPUS.len() >= 14);
}

#[test]
fn failures_on_the_tree_walker_and_vm() {
    let failures = CORPUS
        .iter()
        .filter_map(|(name, src, expected_out, reason)| {
            let r = run_all_tiers(src);
            let expected_err = (!reason.is_empty()).then(|| reason.to_string());
            (r.output.trim() != *expected_out || r.error != expected_err).then(|| {
                format!(
                    "{name}: output {:?}, error {:?}; expected {expected_out:?}, {reason:?}\n{src}",
                    r.output.trim(),
                    r.error
                )
            })
        })
        .collect();
    report("tree-walker/VM", failures);
}

#[test]
fn failures_on_the_rust_tier() {
    let failures = CORPUS
        .iter()
        .filter_map(|(name, src, expected_out, reason)| {
            let built = compile_logos(src);
            if !built.success {
                let errs: Vec<&str> = built
                    .stderr
                    .lines()
                    .filter(|l| l.contains("error"))
                    .take(6)
                    .collect();
                return Some(format!(
                    "{name}: does not build: {}\n{src}",
                    errs.join("\n")
                ));
            }
            let out = std::process::Command::new(&built.binary_path)
                .output()
                .expect("runs");
            judge(name, src, expected_out, reason, out)
        })
        .collect();
    report("AOT-Rust", failures);
}

#[test]
fn failures_on_the_c_tier() {
    let failures = CORPUS
        .iter()
        .filter_map(|(name, src, expected_out, reason)| {
            let c_code = match logicaffeine_compile::compile::compile_to_c(src) {
                Ok(c) => c,
                Err(e) => return Some(format!("{name}: C lowering refused: {e:?}\n{src}")),
            };
            let dir = tempfile::tempdir().expect("temp dir");
            let (c_path, bin) = (dir.path().join("main.c"), dir.path().join("main"));
            std::fs::write(&c_path, &c_code).expect("write C");
            let gcc = std::process::Command::new("gcc")
                .args(["-O2", "-o"])
                .arg(&bin)
                .arg(&c_path)
                .arg("-lm")
                .output()
                .expect("gcc runs");
            if !gcc.status.success() {
                let err = String::from_utf8_lossy(&gcc.stderr);
                return Some(format!(
                    "{name}: C does not build: {}\n{src}",
                    err.lines()
                        .filter(|l| l.contains("error"))
                        .take(6)
                        .collect::<Vec<_>>()
                        .join("\n")
                ));
            }
            judge(
                name,
                src,
                expected_out,
                reason,
                std::process::Command::new(&bin).output().expect("runs"),
            )
        })
        .collect();
    report("C", failures);
}

fn run_c(
    name: &str,
    src: &str,
    expected_out: &str,
    reason: &str,
    c_code: Result<String, String>,
) -> Option<String> {
    let c_code = match c_code {
        Ok(c) => c,
        Err(e) => return Some(format!("{name}: C lowering refused: {e}\n{src}")),
    };
    let dir = tempfile::tempdir().expect("temp dir");
    let (c_path, bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&c_path, &c_code).expect("write C");
    let gcc = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&c_path)
        .arg("-lm")
        .output()
        .expect("gcc runs");
    if !gcc.status.success() {
        let err = String::from_utf8_lossy(&gcc.stderr);
        return Some(format!(
            "{name}: C does not build: {}\n{src}",
            err.lines()
                .filter(|l| l.contains("error"))
                .take(6)
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    judge(
        name,
        src,
        expected_out,
        reason,
        std::process::Command::new(&bin).output().expect("runs"),
    )
}

#[test]
fn failures_on_the_c_tier_lowered_from_bytecode() {
    let failures = CORPUS
        .iter()
        .filter_map(|(name, src, expected_out, reason)| {
            let c = logicaffeine_compile::compile::compile_to_c_via_bytecode(src)
                .map_err(|e| format!("{e:?}"));
            run_c(name, src, expected_out, reason, c)
        })
        .collect();
    report("C (from bytecode)", failures);
}

// ─── Tiers that must REPLAY the VM's failure exactly ────────────────────────

/// A loop that becomes hot (>100 iterations, so the forge tiers it) and then breaks a contract
/// mid-loop: the native region deopts and the VM raises the identical reason.
const HOT_CONTRACT: &str = "## Main\n\
Let mutable total be 0.\n\
Let mutable i be 0.\n\
While i is less than 5000:\n\
\x20   Require that total is less than 900000, otherwise \"total overflowed at i = \" + format(i).\n\
\x20   Set total to total + i.\n\
\x20   Set i to i + 1.\n\
Show total.\n";

fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("thread")
        .join()
        .expect("no panic")
}

#[test]
fn failure_under_the_forge_is_the_vm_failure() {
    use logicaffeine_compile::compile::vm_outcome_with_args;
    use logicaffeine_compile::vm::NativeTier;
    let failures: Vec<String> = on_big_stack(|| {
        let mut programs: Vec<(String, String, String, String)> = CORPUS
            .iter()
            .map(|(n, s, o, r)| (n.to_string(), s.to_string(), o.to_string(), r.to_string()))
            .collect();
        programs.push((
            "hot_contract".into(),
            HOT_CONTRACT.into(),
            "".into(),
            "total overflowed at i = 1343".into(),
        ));
        programs
            .into_iter()
            .filter_map(|(name, src, expected_out, reason)| {
                let tier = logicaffeine_jit::ForgeTier::new();
                let r = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
                let expected_err = (!reason.is_empty()).then(|| reason.clone());
                (r.output.trim() != expected_out || r.error != expected_err).then(|| {
                    format!("{name}: output {:?}, error {:?}; expected {expected_out:?}, {reason:?}\n{src}", r.output.trim(), r.error)
                })
            })
            .collect()
    });
    report("forge-JIT", failures);
}

#[test]
fn failure_survives_the_binary_wire_format() {
    use logicaffeine_compile::vm::binisa::{decode_program, encode_program};
    let failures = CORPUS
        .iter()
        .filter_map(|(name, src, expected_out, reason)| {
            let decoded =
                logicaffeine_compile::ui_bridge::with_parsed_program(src, |parsed, interner| {
                    let (stmts, types, _) = parsed.expect("parses");
                    let program = logicaffeine_compile::vm::Compiler::compile_with_types(
                        stmts,
                        interner,
                        Some(types),
                    )
                    .expect("compiles");
                    let bytes = encode_program(&program, interner);
                    let mut fresh = logicaffeine_base::Interner::new();
                    decode_program(&bytes, &mut fresh)
                });
            let Some(decoded) = decoded else {
                return Some(format!("{name}: the wire bytes do not decode\n{src}"));
            };
            let r = logicaffeine_compile::compile::program_outcome(&decoded);
            let expected_err = (!reason.is_empty()).then(|| reason.to_string());
            (r.output.trim() != *expected_out || r.error != expected_err).then(|| {
                format!(
                    "{name}: output {:?}, error {:?}; expected {expected_out:?}, {reason:?}\n{src}",
                    r.output.trim(),
                    r.error
                )
            })
        })
        .collect();
    report("binary-ISA round trip", failures);
}
