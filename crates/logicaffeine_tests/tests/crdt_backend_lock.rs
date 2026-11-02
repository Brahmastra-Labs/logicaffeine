//! Every CRDT kind × every backend without the Rust runtime: the backend either lowers the kind and
//! runs it exactly as the tree-walker and the VM do, or refuses it by the type's name.
//!
//! [`CrdtKind::lowered_by`] is the one answer to "does this backend lower this CRDT?", and the gate
//! (`compile::refuse_unlinkable`, and the AST→C backend's field gate) acts on it. An answer that says
//! `true` for a kind the backend mis-runs is a silent wrong answer; an answer that says `false` for a
//! kind it runs is a refusal nobody needed. This lock runs every pair and holds each answer to what
//! the backend actually does, over a single-replica program that walks the kind's whole surface.

#![cfg(all(feature = "wasm-jit", not(target_arch = "wasm32")))]

use logicaffeine_compile::codegen_riscv::RISCV_RUNTIME_C;
use logicaffeine_compile::compile::{
    compile_to_c, compile_to_c_via_bytecode, compile_to_riscv, tw_outcome, vm_outcome, Backend,
};
use logicaffeine_compile::semantics::crdt::CrdtKind;
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "compile_common/mod.rs"]
mod common;

/// The single-replica program walking `kind`'s surface. Each builds its CRDT as a `Shared` record's
/// field — the only way a program declares one — and shows every observable the kind has.
fn program(kind: CrdtKind) -> &'static str {
    match kind {
        CrdtKind::Set => "## Definition\nA Doc is Shared and has:\n    a tags, which is a SharedSet of Text.\n\n\
            ## Main\nLet mutable d be a new Doc.\nAdd \"pear\" to d's tags.\nAdd \"apple\" to d's tags.\n\
            Add \"fig\" to d's tags.\nAdd \"apple\" to d's tags.\nShow d's tags.\nShow length of d's tags.\n\
            Show d's tags contains \"fig\".\nRemove \"apple\" from d's tags.\nShow d's tags.\n",
        CrdtKind::SetRemoveWins => "## Definition\nA Mod is Shared and has:\n    a blocked, which is a SharedSet (RemoveWins) of Text.\n\n\
            ## Main\nLet mutable m be a new Mod.\nAdd \"zed\" to m's blocked.\nAdd \"amy\" to m's blocked.\n\
            Show m's blocked.\nRemove \"zed\" from m's blocked.\nShow m's blocked.\nShow length of m's blocked.\n",
        CrdtKind::Seq => "## Definition\nA Log is Shared and has:\n    a entries, which is a SharedSequence of Int.\n\n\
            ## Main\nLet mutable l be a new Log.\nAppend 20 to l's entries.\nAppend 10 to l's entries.\n\
            Show length of l's entries.\nShow l's entries.\n",
        CrdtKind::Register => "## Definition\nA Page is Shared and has:\n    a rev, which is a Divergent Int.\n\n\
            ## Main\nLet mutable p be a new Page.\nSet p's rev to 3.\nShow p's rev.\n\
            Resolve p's rev to 4.\nShow p's rev.\n",
        CrdtKind::Counter => "## Definition\nA Hits is Shared and has:\n    a total, which is a ConvergentCount.\n\n\
            ## Main\nLet mutable h be a new Hits.\nIncrease h's total by 5.\nIncrease h's total by 3.\nShow h's total.\n",
        CrdtKind::PnCounter => "## Definition\nA Score is Shared and has:\n    a points, which is a Tally.\n\n\
            ## Main\nLet mutable s be a new Score.\nIncrease s's points by 10.\nDecrease s's points by 3.\nShow s's points.\n",
        CrdtKind::Map => "## Definition\nA Warehouse is Shared and has:\n    stock: a SharedMap from Text to Int.\n\n\
            ## To restock (w: Warehouse) -> Int:\n    Set w's stock[\"iron\"] to 4.\n    Return length of w's stock.\n\n\
            ## Main\nLet mutable w be a new Warehouse.\nShow length of w's stock.\nSet w's stock[\"wood\"] to 50.\n\
            Set w's stock[\"stone\"] to 30.\nSet w's stock[\"clay\"] to 7.\nSet w's stock[\"wood\"] to 55.\n\
            Show w's stock[\"wood\"].\nShow item \"clay\" of w's stock.\nShow length of w's stock.\n\
            Show w's stock contains \"stone\".\nShow w's stock contains \"iron\".\nShow w's stock.\n\
            Remove \"stone\" from w's stock.\nShow w's stock contains \"stone\".\nShow length of w's stock.\n\
            Show w's stock.\nLet mutable b be w.\nSet b's stock[\"apple\"] to 2.\nShow w's stock.\n\
            Show restock(w).\nShow w's stock.\nLet mutable c be copy(w).\nSet c's stock[\"zinc\"] to 9.\n\
            Show w's stock.\nShow c's stock.\n",
    }
}

fn tool(name: &str, arg: &str) -> bool {
    std::process::Command::new(name)
        .arg(arg)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Build `code` with `cc` and run it; `Err` names what failed.
fn build_run(code: &str, cc: &str, extra: &[&str], ext: &str) -> Result<String, String> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "logos_crdt_backend_{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let src = dir.join(format!("prog.{ext}"));
    let bin = dir.join("prog");
    std::fs::write(&src, code).map_err(|e| e.to_string())?;
    let build = std::process::Command::new(cc)
        .arg("-O1")
        .args(extra)
        .arg("-o")
        .arg(&bin)
        .arg(&src)
        .arg("-lm")
        .output()
        .map_err(|e| e.to_string())?;
    if !build.status.success() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(format!(
            "{cc} could not build it: {}",
            String::from_utf8_lossy(&build.stderr)
        ));
    }
    let run = std::process::Command::new(&bin)
        .output()
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    if !run.status.success() {
        return Err(format!(
            "it failed at run time: {}",
            String::from_utf8_lossy(&run.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&run.stdout).trim_end().to_string())
}

fn riscv_run(asm: &str) -> Result<String, String> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "logos_crdt_backend_rv_{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let (asm_path, rt_path, bin) = (dir.join("main.s"), dir.join("logos_rt.c"), dir.join("main"));
    std::fs::write(&asm_path, asm).map_err(|e| e.to_string())?;
    std::fs::write(&rt_path, RISCV_RUNTIME_C).map_err(|e| e.to_string())?;
    let build = std::process::Command::new("riscv64-linux-gnu-gcc")
        .args(["-O1", "-static", "-o"])
        .arg(&bin)
        .arg(&asm_path)
        .arg(&rt_path)
        .arg("-lm")
        .output()
        .map_err(|e| e.to_string())?;
    if !build.status.success() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(format!(
            "could not assemble: {}",
            String::from_utf8_lossy(&build.stderr)
        ));
    }
    let run = std::process::Command::new("qemu-riscv64")
        .arg(&bin)
        .output()
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    if !run.status.success() {
        return Err(format!(
            "it failed at run time: {}",
            String::from_utf8_lossy(&run.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&run.stdout).trim_end().to_string())
}

/// What `backend` does with `src`: `Ok(output)` when it lowers and runs it, `Err(why)` when it
/// refuses or the lowered program fails. `None` when this machine lacks the backend's toolchain.
fn on(backend: Backend, src: &str) -> Option<Result<String, String>> {
    Some(match backend {
        Backend::C => {
            if !tool("g++", "--version") {
                return None;
            }
            compile_to_c_via_bytecode(src)
                .map_err(|e| format!("{e:?}"))
                .and_then(|c| build_run(&c, "g++", &["-pthread"], "cpp"))
        }
        Backend::CFromAst => {
            if !tool("gcc", "--version") {
                return None;
            }
            compile_to_c(src)
                .map_err(|e| format!("{e:?}"))
                .and_then(|c| build_run(&c, "gcc", &[], "c"))
        }
        Backend::RiscV => {
            if !(tool("riscv64-linux-gnu-gcc", "--version") && tool("qemu-riscv64", "-version")) {
                return None;
            }
            compile_to_riscv(src)
                .map_err(|e| format!("{e:?}"))
                .and_then(|asm| riscv_run(&asm))
        }
        Backend::WebAssembly => common::wasm_host::run_show(src).map(|o| o.trim_end().to_string()),
        Backend::SoftCore => logicaffeine_compile::codegen_sva::full_asm::compile_to_full_words(
            src,
        )
        .and_then(|words| {
            let run = logicaffeine_compile::codegen_sva::soft_core_model::run_soft_core_outcome(
                &words, 5_000_000,
            );
            match run.failure {
                None => Ok(run
                    .shows
                    .iter()
                    .map(|v| v.fmt())
                    .collect::<Vec<_>>()
                    .join("\n")),
                Some(f) => Err(format!("it failed on the core: {f:?}")),
            }
        }),
    })
}

#[test]
fn every_crdt_kind_is_lowered_identically_or_refused_by_name_on_every_backend() {
    let mut wrong: Vec<String> = Vec::new();
    for kind in CrdtKind::ALL {
        let src = program(kind);
        let tw = tw_outcome(src);
        let vm = vm_outcome(src);
        assert_eq!(
            tw.error,
            None,
            "the tree-walker must run the {} program:\n{src}",
            kind.type_name()
        );
        assert_eq!(
            vm.error,
            None,
            "the VM must run the {} program:\n{src}",
            kind.type_name()
        );
        assert_eq!(
            tw.output.trim_end(),
            vm.output.trim_end(),
            "tree-walker ≠ VM on the {} program",
            kind.type_name()
        );
        let expected = tw.output.trim_end();
        for backend in Backend::ALL {
            let Some(outcome) = on(backend, src) else {
                eprintln!(
                    "SKIP {} on {backend:?}: toolchain not installed",
                    kind.type_name()
                );
                continue;
            };
            let pair = format!("{} × {backend:?}", kind.type_name());
            match (kind.lowered_by(backend), outcome) {
                (true, Ok(got)) if got == expected => {}
                (true, Ok(got)) => wrong.push(format!(
                    "{pair}: lowered_by says it lowers, and it ran DIFFERENTLY\n  expected {expected:?}\n  got      {got:?}"
                )),
                (true, Err(e)) => wrong.push(format!("{pair}: lowered_by says it lowers, and it did not: {e}")),
                (false, Err(e)) if e.contains(kind.type_name()) => {}
                (false, Err(e)) => wrong.push(format!("{pair}: refused, but not by the type's name: {e}")),
                (false, Ok(got)) => wrong.push(format!(
                    "{pair}: lowered_by says it refuses, and it ran (output {got:?}) — the gate is not reached"
                )),
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} (kind, backend) pairs disagree with `CrdtKind::lowered_by`:\n\n{}",
        wrong.len(),
        wrong.join("\n\n")
    );
}

#[test]
fn the_table_is_not_uniform() {
    // A table answering the same thing for every pair would pass the lock above on a backend that
    // refuses everything; each answer has to be a decision.
    let answers: Vec<bool> = CrdtKind::ALL
        .into_iter()
        .flat_map(|k| Backend::ALL.into_iter().map(move |b| k.lowered_by(b)))
        .collect();
    assert!(answers.iter().any(|a| *a) && answers.iter().any(|a| !*a));
}
