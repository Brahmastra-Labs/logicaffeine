//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ONE QWEN3.8 DECODE STEP AS SPATIAL SOURCE — the RDU's own lineage, executable.
//!
//! The SN50 backend ships a certified neutral placement and stops at the vendor's config bytes. Spatial is the
//! language the RDU descends from (Plasticine), and its public chain — Spatial → PIR → placement → the Tungsten
//! cycle simulator — runs in our container. So the decode step printed as Spatial source is the one form of the
//! model that EXECUTES on an RDU-lineage fabric without the SDK: the app checks its own token against the VM's.
//!
//! Pinned: the tiny decode step (token in, KV caches appended, next token out) prints as ONE Spatial app whose
//! host checks what the VM computes — the token exactly, every logit, every KV-cache row the step appends — and
//! that app runs, whole, in Spatial's own simulator (the Scala backend: every controller of all 121 sections
//! executed) and passes: the token exactly, every logit and every appended cache row within fp32 rounding of the
//! VM's f64. PIR, the fabric-mapping half of the chain, writes an accelerator as ONE JVM class, and 121 sections
//! are past what the JVM allows a class — the generated `AccelMain` is 27,493 lines in one method — so mapping the
//! step onto the fabric means cutting it into apps of a few sections each, which this lock does not do. The
//! container test is tool-gated (`docker` and the built toolchain) and serialized in the `plasticine-chain`
//! nextest group.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test qwen38_spatial_source_lock`
//!
//! ★ MUTATION PROBES (each in `spatial_lang.rs`, against the simulator test): drop the zero page (a stage writing
//! part of its page no longer zeroes the rest) → the step's host checks fail (the padding a matmul folds over is
//! undefined); call the sections in reverse → fail; drop a gather's `index · stride` term → fail. Dropping the
//! off-page guard does NOT fail here — every off-page read of the step sits on a discarded `Select` branch; the
//! guard is pinned in `spatial_ir_to_spatial_lang_lock`.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/qwen38_fixture.rs"]
mod qwen38_fixture;

use logicaffeine_compile::codegen_cuda::qwen38::Qwen38Cfg;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::lower::spatial_lang::{
    emit_spatial_program, emit_spatial_program_checked, SpatialCheck, SpatialFlavor,
};
use qwen38_fixture::{decode_step, target, POS};

fn vm_token(src: &str) -> i64 {
    let vm = vm_outcome(&format!("{src}Show next.\n"));
    assert!(vm.error.is_none(), "the VM runs the step: {:?}", vm.error);
    vm.output.trim().parse().expect("a token")
}

/// The app's app-side checks, every gold the VM's: the token exactly, every logit, and the row `pos` this step
/// appended to every attention layer's key and value cache. The app computes in fp32 — Plasticine's datapath —
/// against the VM's f64, so its floats sit within `1e-3` of the largest gold.
fn vm_checks(src: &str) -> Vec<SpatialCheck> {
    let cfg = Qwen38Cfg::tiny();
    let caches: Vec<String> = (0..cfg.layers)
        .filter(|l| cfg.is_attn(*l))
        .flat_map(|l| [format!("Kc{l}"), format!("Vc{l}")])
        .collect();
    let mut shown = format!("{src}Show next.\n");
    for i in 1..=cfg.vocab {
        shown.push_str(&format!("Show item {i} of logits.\n"));
    }
    for c in &caches {
        for e in 1..=cfg.akv() {
            shown.push_str(&format!(
                "Show item {} of {c}.\n",
                (POS - 1) * cfg.akv() + e
            ));
        }
    }
    let vm = vm_outcome(&shown);
    assert!(vm.error.is_none(), "the VM runs the step: {:?}", vm.error);
    let vals: Vec<f64> = vm
        .output
        .split_whitespace()
        .map(|t| t.parse().expect("a number"))
        .collect();
    assert_eq!(
        vals.len(),
        1 + cfg.vocab + caches.len() * cfg.akv(),
        "the VM showed every checked value"
    );
    let mut checks = vec![
        SpatialCheck {
            tensor: "next".into(),
            offset: 0,
            gold: vec![vals[0]],
            rel_tol: 0.0,
        },
        SpatialCheck {
            tensor: "logits".into(),
            offset: 0,
            gold: vals[1..=cfg.vocab].to_vec(),
            rel_tol: 1e-3,
        },
    ];
    for (k, c) in caches.iter().enumerate() {
        let at = 1 + cfg.vocab + k * cfg.akv();
        checks.push(SpatialCheck {
            tensor: c.clone(),
            offset: (POS - 1) * cfg.akv(),
            gold: vals[at..at + cfg.akv()].to_vec(),
            rel_tol: 1e-3,
        });
    }
    checks
}

/// ── THE TINY DECODE STEP PRINTS AS ONE SPATIAL APP, CHECKED AGAINST THE VM'S TOKEN.
#[test]
fn the_tiny_decode_step_prints_as_one_spatial_app() {
    let (src, p) = decode_step();
    assert_eq!(
        p.output_tensor().map(|t| t.name.as_str()),
        Some("next"),
        "the app checks the token the step emits"
    );
    let token = vm_token(&src);
    let app = emit_spatial_program(&p, &target(), Some(&token.to_string()))
        .unwrap_or_else(|e| panic!("the decode step prints as Spatial source: {e:?}"));
    assert!(app.contains("@spatial class Logos_"), "a Spatial app");
    assert!(
        app.contains(&format!("val gold = {}", token))
            || app.contains(&format!("({token}L: Long)")),
        "whose gold is the VM's token {token}"
    );
}

/// ── THE APP CHECKS WHAT THE VM COMPUTES: the token, every logit, every appended cache row.
#[test]
fn the_checked_app_carries_the_vms_token_logits_and_cache_rows() {
    let (src, p) = decode_step();
    let checks = vm_checks(&src);
    let app = emit_spatial_program_checked(&p, &target(), &checks, SpatialFlavor::Simulator)
        .unwrap_or_else(|e| panic!("the checked decode step prints: {e:?}"));
    for (k, c) in checks.iter().enumerate() {
        assert!(
            app.contains(&format!("// check {k}: {} elements", c.gold.len()))
                && app.contains(&format!("of {} against", c.tensor)),
            "check {k} ({}) is printed",
            c.tensor
        );
    }
    assert!(
        app.contains(&format!(
            "val cksum = {}",
            (0..checks.len())
                .map(|k| format!("ok{k}"))
                .collect::<Vec<_>>()
                .join(" && ")
        )),
        "the PASS is every check at once"
    );
}

/// ── AND THE APP RUNS, WHOLE, IN SPATIAL'S OWN SIMULATOR: every controller of the 121 sections executes and the
/// host's checks — the VM's token, logits and cache rows — pass. (PIR writes an accelerator as one JVM class, and
/// a program of 121 sections is past what the JVM allows one; Spatial's simulator splits its code.)
#[test]
fn the_tiny_decode_step_is_the_vms_in_spatials_simulator() {
    if !common::require_tool("docker") || !common::require_tool("plasticine-toolchain") {
        return;
    }
    let (src, p) = decode_step();
    let app =
        emit_spatial_program_checked(&p, &target(), &vm_checks(&src), SpatialFlavor::Simulator)
            .expect("the decode step prints");
    let class = app
        .split("@spatial class ")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .expect("the app's class")
        .to_string();
    let regression = common::plasticine::regression_dir();
    std::fs::write(regression.join(format!("{class}.scala")), &app)
        .expect("write the regression app");
    std::fs::write(regression.join(format!("{class}.lg")), &src)
        .expect("write the LOGOS source beside it");
    let run = common::plasticine::run_sim(&class);
    println!("sim log: {}", run.log_path.display());
    assert!(
        run.passed(),
        "Spatial's simulator prints the harness's PASS for {class} — log at {}:\n{}",
        run.log_path.display(),
        run.tail(80)
    );
    for line in run.log.lines().filter(|l| l.contains("check ")) {
        println!("{line}");
    }
}
