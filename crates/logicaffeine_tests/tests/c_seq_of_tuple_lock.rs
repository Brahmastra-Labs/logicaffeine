//! ★ SEQ-OF-TUPLE + DESTRUCTURE LOCK ★ — a `Seq of (Int, Int)` literal and a `Repeat for (a, b) in …`
//! tuple-destructuring loop (`Op::DestructureTuple`) lower faithfully through the C backend.
//!
//! `Let pairs be [(1, 2), (3, 4)]` builds a Seq whose element is a synthesized `LTuple_…` struct. Like
//! every other dynamic-named Seq element (`SeqValue`/`SeqStruct`/`SeqSeq`), the tuple case can only be
//! named by `c_type_str_resolved` (the interner mangles `Seq_LTuple_i64_i64`), so it MUST ride the general
//! statement-expression Let path — not the static-name special case, which falls to `seq_i64` and emits the
//! `/* Seq of tuple */` placeholder as a type, producing `'pairs' undeclared` under gcc. This lock pins the
//! faithful lowering so the whole C-derived tier family (C / Tenstorrent / CUDA) can build it.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. If it regresses, fix `codegen_c`, not the lock.

use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::compile::{compile_to_c, vm_outcome};

/// A Seq-of-tuple literal iterated with a tuple-destructuring `Repeat for (a, b) in …` (`DestructureTuple`).
/// Pre-fix, the Let for the literal emitted `/* Seq of tuple */ pairs = seq_i64_new();` (wrong family +
/// a comment where the C type belongs) → `'pairs' undeclared` under gcc.
const SEQ_OF_TUPLE_DESTRUCTURE: &str = "\
## Main
Let pairs be [(1, 2), (3, 4), (5, 6)].
Repeat for (a, b) in pairs:
    Show a + b.
";

fn have_gcc() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Compile C with gcc and run it; `Some(stdout)` on success, `None` if it failed to compile or ran nonzero.
fn gcc_compile_and_run(c_src: &str) -> Option<String> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("logos_seqtuple_{}_{n}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let c_path = dir.join("k.c");
    let bin_path = dir.join("k");
    std::fs::write(&c_path, c_src).ok()?;
    let c = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin_path)
        .arg(&c_path)
        .arg("-lm")
        .output()
        .ok()?;
    if !c.status.success() {
        eprintln!("  gcc failed:\n{}", String::from_utf8_lossy(&c.stderr));
        let _ = std::fs::remove_dir_all(&dir);
        return None;
    }
    let r = std::process::Command::new(&bin_path).output().ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    r.status
        .success()
        .then(|| String::from_utf8_lossy(&r.stdout).to_string())
}

/// The invariant: a Seq-of-tuple literal + tuple-destructuring loop compiles and runs ≡VM through C.
#[test]
fn seq_of_tuple_destructure_compiles_and_runs_equiv_vm() {
    let vm = vm_outcome(SEQ_OF_TUPLE_DESTRUCTURE);
    assert!(
        vm.error.is_none(),
        "the reference VM run must succeed: {:?}",
        vm.error
    );

    let c = compile_to_c(SEQ_OF_TUPLE_DESTRUCTURE).expect("compiles to C");
    // The Seq element must be named as a resolved tuple struct, never the `/* Seq of tuple */` placeholder.
    assert!(
        !c.contains("/* Seq of tuple */ "),
        "the Seq-of-tuple Let emitted the placeholder comment as its C type:\n{c}"
    );

    if !have_gcc() {
        eprintln!("SKIP: gcc not present — cannot prove the emitted C compiles");
        return;
    }
    let out = gcc_compile_and_run(&c).expect(
        "the emitted C must gcc-compile (a Seq-of-tuple placeholder type is `'pairs' undeclared`)",
    );
    assert_eq!(
        out.trim(),
        vm.output.trim(),
        "C output diverged from the VM for the Seq-of-tuple destructure"
    );
}
