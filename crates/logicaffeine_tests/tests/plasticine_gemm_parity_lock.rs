//! ═══════════════════════════════════════════════════════════════════════
//! RDU GEMM PARITY LOCK — a program compiled from English must not lose to the same program written by hand
//! in the architecture's own language, on the architecture's own toolchain.
//!
//! Spatial's regression suite carries a parameterised GEMM and ~24 instantiations of it. Each one fixes a
//! shape (`dim`), a tiling (`ts`, `its`), an innermost vector width (`ip`), and — this is the part that
//! matters — up to five loop PARALLELISATION factors:
//!
//!     Foreach(dim by its par loop_ii, dim by ts par loop_jj)   outer tile loops
//!       MemReduce(c_col par ip)(dim by ts par loop_kk)         the reduction
//!         Foreach(its by 1 par loop_i)                         inner row loop
//!           Foreach(ts by 1 par loop_j)                        inner column loop
//!             Reduce(Reg[T])(ts by 1 par ip)                   innermost dot
//!
//! We emit the same GEMM structure from an ordinary English program. If our emission fixes any of those
//! factors at 1 while their instantiation raises it, we do strictly less work per cycle and lose by roughly
//! that factor — which is exactly what happened: on GEMM_9 (`loop_j = 2`) we measured 1.94x their cycles,
//! against 1.000x on GEMM_0 where every factor is 1.
//!
//! A. For every Spatial GEMM variant we have emitted an English twin of, our cycles are at or below theirs
//!    on the same chain, same mesh, same tiling — measured by Tungsten, not modelled.
//! B. The twin really is a twin: same dim / ts / its / ip, so a win cannot come from quietly tiling
//!    differently.
//! C. Non-vacuous: the corpus contains at least one variant that raises a parallelisation factor above 1,
//!    since a suite of all-ones variants would prove nothing about the thing that bit us.
//!
//! ⚠️ If this goes RED, emit the missing parallelisation — never widen the tolerance.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(plasticine_gemm_parity_lock)'`
//! ═══════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

/// Their measurement and ours are separate runs of a deterministic simulator; a hair of slack absorbs
/// harness-level differences (an extra store, a differently-ordered gold check) without admitting a real
/// parallelisation gap, which shows up as a whole factor.
const SLACK: f64 = 1.02;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn ledger() -> serde_json::Value {
    let p = repo_root().join("benchmarks/silicon/plasticine_toolchain.json");
    let t = std::fs::read_to_string(&p).unwrap_or_else(|e| {
        panic!(
            "read {}: {e} — run scripts/plasticine/run-in-container.sh",
            p.display()
        )
    });
    serde_json::from_str(&t).expect("the toolchain ledger parses")
}

fn rows(d: &serde_json::Value) -> Vec<serde_json::Value> {
    d.get("rows")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_else(|| d.as_array().cloned().unwrap_or_default())
}

fn param(r: &serde_json::Value, k: &str) -> Option<i64> {
    r["params"][k].as_i64()
}

fn cycles(r: &serde_json::Value) -> Option<f64> {
    r["hybrid_network"]["cycles"].as_f64()
}

/// `(their row, our twin)` for every Spatial GEMM variant that has one: same dim, ts, its and ip.
fn twins(d: &serde_json::Value) -> Vec<(serde_json::Value, serde_json::Value)> {
    let all = rows(d);
    let theirs: Vec<&serde_json::Value> = all
        .iter()
        .filter(|r| r["app"].as_str().map_or(false, |a| a.starts_with("GEMM_")))
        .collect();
    let ours: Vec<&serde_json::Value> = all
        .iter()
        .filter(|r| {
            r["app"]
                .as_str()
                .map_or(false, |a| a.starts_with("Logos_gemm_"))
        })
        .collect();
    // Every matching variant, not the first. Their suite hand-tunes several DIFFERENT unrollings of the same
    // shape and tiling — at 128^3 alone: GEMM_0 serial, GEMM_2 loop_j, GEMM_3 loop_kk, GEMM_4 loop_i, GEMM_6
    // all three. Our compiler emits ONE program for that configuration and chooses its own parallelisation,
    // so the property worth holding is that the single automatic choice matches or beats every hand-tuned
    // variant of it — not merely the first one we happen to pair with.
    let mut out = Vec::new();
    for t in theirs {
        for o in &ours {
            if ["dim", "ts", "its", "ip"]
                .iter()
                .all(|k| param(t, k).is_some() && param(t, k) == param(o, k))
            {
                out.push(((*t).clone(), (*o).clone()));
            }
        }
    }
    out
}

/// The parallelisation factors their instantiation raises above 1.
fn raised(r: &serde_json::Value) -> Vec<(String, i64)> {
    ["loop_ii", "loop_jj", "loop_kk", "loop_i", "loop_j"]
        .iter()
        .filter_map(|k| {
            param(r, k)
                .filter(|v| *v > 1)
                .map(|v| ((*k).to_string(), v))
        })
        .collect()
}

// ───────────────────────────── A + B + C ─────────────────────────────

#[test]
fn compiled_from_english_never_loses_to_hand_written_spatial() {
    let d = ledger();
    let pairs = twins(&d);
    assert!(
        !pairs.is_empty(),
        "no Spatial GEMM variant has an English twin in the toolchain ledger — emit one at their parameters \
         (examples/emit_plasticine_spatial_gemm) and run it through the chain, or this proves nothing"
    );

    // C: the corpus must exercise the thing that actually bit us.
    let with_par: Vec<&(serde_json::Value, serde_json::Value)> = pairs
        .iter()
        .filter(|(t, _)| !raised(t).is_empty())
        .collect();
    assert!(
        !with_par.is_empty(),
        "every paired variant leaves all five parallelisation factors at 1, so this lock would pass without \
         testing the case it exists for. Pair a variant that raises one (GEMM_9 raises loop_j)."
    );

    let mut lost = Vec::new();
    for (t, o) in &pairs {
        let (Some(tc), Some(oc)) = (cycles(t), cycles(o)) else {
            continue;
        };
        // B: a win must not come from a different tiling.
        for k in ["dim", "ts", "its", "ip"] {
            assert_eq!(
                param(t, k),
                param(o, k),
                "{} vs {}: `{k}` differs — not a twin",
                t["app"],
                o["app"]
            );
        }
        if oc > tc * SLACK {
            let par = raised(t);
            lost.push(format!(
                "{} {} cycles vs our {} ({:.2}x){}",
                t["app"].as_str().unwrap_or("?"),
                tc as u64,
                oc as u64,
                oc / tc,
                if par.is_empty() {
                    " — and their variant raises no parallelisation factor, so the gap is elsewhere"
                        .to_string()
                } else {
                    format!(
                        " — their variant raises {}, which we do not emit",
                        par.iter()
                            .map(|(k, v)| format!("{k}={v}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            ));
        }
    }
    assert!(
        lost.is_empty(),
        "compiled-from-English GEMM loses to hand-written Spatial on the architecture's own toolchain:\n  {}\n\
         Emit the missing loop parallelisation — do NOT widen the tolerance.",
        lost.join("\n  ")
    );
}
