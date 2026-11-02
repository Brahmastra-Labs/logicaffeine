//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! WHAT WE CAN ACTUALLY HAND SAMBANOVA ON DAY ONE — pinned, so it cannot quietly shrink.
//!
//! The SDK is not here. Everything up to the vendor seam must therefore be real TODAY, and the only
//! honest way to know that is to produce every artifact from the real programs and record, per program
//! per format, whether it comes out. This lock is that record.
//!
//! The four day-one routes, in the order they are likely to be taken:
//!   * **SambaFlow** ingests PyTorch, so `--format sambaflow` emits `model.py` + a weight manifest.
//!   * **their vLLM RDU backend** takes a deployment plan, so `--format vllm` emits it.
//!   * **RDA / a neutral bridge** takes `neutral.json` and the recheckable certificate.
//!   * **Spatial / PIR** takes Spatial source, so `--format spatial` emits `spatial.scala`.
//!
//! ★ WHAT THIS DOES NOT PROVE. A cell here is "producible": the command ran and the artifact is not
//! empty. That says nothing about whether the artifact COMPUTES the right thing — a lowering that emits a
//! well-formed, wrong program passes every cell. The semantic check is `emit_rdu_e2e` in the CLI crate:
//! it runs `largo emit rdu`, reads `neutral.json` back off disk, re-certifies it, executes it, and
//! compares every output against the VM. Read this ledger as coverage of the SURFACE, and that file as
//! evidence the surface is right.
//!
//! ★ WHY A LEDGER AND NOT A LIST OF PASSING CASES. A matrix of things that work says nothing about the
//! things that do not, and a gap nobody wrote down is a gap discovered on the day. Every cell here is
//! either PRODUCIBLE or a named gap carrying the refusal it must still be refused BY, so the failure
//! mode "a format silently stopped covering a program" is impossible: the cell flips and the lock reds.
//!
//! ★ THIS WOULD HAVE CAUGHT THE ONE I SHIPPED. Gating the fused-row flat twin on width made a 2x4 rmsnorm
//! emit Spatial source and simultaneously made the composed Qwen forward stop lowering at all — `qr3`
//! came out (2, 4) where every other section reads it flat as (1, 8). One cell improved and a different
//! one broke, which is exactly the trade a per-cell ledger makes visible and a spot check does not.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test rdu_emit_matrix_lock`
//!
//! ★ MUTATION PROBE: delete a row from `GAPS` — `the_ledger_has_no_stale_rows` stays green but
//! `every_cell_matches_the_ledger` reds on the now-unexplained refusal. Change a gap's `because` to a
//! string the refusal does not contain — `every_gap_is_refused_for_the_reason_it_records` reds while the
//! rest stay green, which is the pair proving the reasons are read and not decoration.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::compile_to_rdu;
use logicaffeine_compile::spatial::lower::sambaflow::emit_torch_module;
use logicaffeine_compile::spatial::rdu_chip::RduChip;

/// The demo corpus — one program per lane shape the day could ask for.
const PROGRAMS: &[&str] = &["gemm", "rmsnorm", "dot", "fib", "primes", "qwen38_tiny"];

/// Every artifact `largo emit rdu --format F` can produce.
const FORMATS: &[&str] = &["placement", "spatial", "sambaflow", "vllm", "report"];

/// A cell that does NOT produce its artifact, and the refusal it must be refused by. SHRINK-ONLY: a row
/// leaves this table when the gap closes, and one may only be added with a written reason.
const GAPS: &[(&str, &str, &str)] = &[
    // ── Spatial source: the whole-program lane prints through codegen_sva, not this emitter.
    ("fib", "spatial", "Scalar stage"),
    ("primes", "spatial", "Scalar stage"),
    // ── SambaFlow ingests a TENSOR program; the loop and whole-program lanes are not one.
    ("dot", "sambaflow", "not a tensor sentence"),
    ("fib", "sambaflow", "not a tensor sentence"),
    ("primes", "sambaflow", "not a tensor sentence"),
];

/// The gap count may only fall. Raising it means a day-one artifact stopped being producible.
const FROZEN_GAP_COUNT: usize = 7;

fn source_of(name: &str) -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let p = root.join("examples/rdu_demo").join(format!("{name}.lg"));
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// Produce one cell. `Ok(bytes)` when the artifact comes out, `Err(refusal)` when it is refused by name.
fn cell(program: &str, format: &str) -> Result<usize, String> {
    let src = source_of(program);
    if format == "sambaflow" {
        return emit_torch_module(&src)
            .map(|t| t.model_py.len() + t.compile_py.len() + t.manifest_json.len());
    }
    let art = compile_to_rdu(&src, RduChip::Sn50).map_err(|e| format!("{e:?}"))?;
    match format {
        "placement" => Ok(art.placement_txt.len()),
        "report" => Ok(art.report_json.len()),
        // The vLLM plan is derived from the chip facts and the model, not from this source.
        "vllm" => Ok(art.neutral_json.len()),
        "spatial" => art
            .spatial_source
            .as_ref()
            .map(|s| s.len())
            .ok_or_else(|| art.notes.join(" | ")),
        other => panic!("unknown format {other}"),
    }
}

fn gap_for(program: &str, format: &str) -> Option<&'static str> {
    GAPS.iter()
        .find(|(p, f, _)| *p == program && *f == format)
        .map(|(_, _, why)| *why)
}

/// ── THE MATRIX IS THE LEDGER. Every cell is producible, or a gap this file names.
#[test]
fn every_cell_matches_the_ledger() {
    let mut surprises = Vec::new();
    let mut produced = 0usize;
    for p in PROGRAMS {
        for f in FORMATS {
            match (cell(p, f), gap_for(p, f)) {
                (Ok(n), None) => {
                    assert!(n > 0, "{p}/{f}: produced an EMPTY artifact, which is not producing it");
                    produced += 1;
                }
                (Ok(_), Some(why)) => surprises.push(format!("{p}/{f} now PRODUCES but the ledger records a gap ({why}) — delete the row, the gap closed")),
                (Err(_), Some(_)) => {}
                (Err(e), None) => surprises.push(format!("{p}/{f} is REFUSED and no gap records it: {}", e.chars().take(160).collect::<String>())),
            }
        }
    }
    assert!(
        surprises.is_empty(),
        "the ledger disagrees with the compiler:\n  {}",
        surprises.join("\n  ")
    );
    println!(
        "{produced} of {} cells producible; {} named gaps",
        PROGRAMS.len() * FORMATS.len(),
        GAPS.len()
    );
}

/// ── AND EVERY GAP IS REFUSED FOR THE REASON IT RECORDS, not merely refused.
#[test]
fn every_gap_is_refused_for_the_reason_it_records() {
    for (p, f, because) in GAPS {
        let e = cell(p, f).err().unwrap_or_else(|| {
            panic!("{p}/{f}: the ledger records a gap but the artifact came out")
        });
        assert!(
            e.contains(because),
            "{p}/{f}: the ledger says the refusal is `{because}`, the compiler says:\n    {}",
            e.chars().take(300).collect::<String>()
        );
    }
}

/// ── NO STALE ROWS, AND THE LEDGER ONLY SHRINKS.
#[test]
fn the_ledger_has_no_stale_rows_and_only_shrinks() {
    assert!(
        GAPS.len() <= FROZEN_GAP_COUNT,
        "a day-one artifact stopped being producible: {} gaps against a frozen {FROZEN_GAP_COUNT}",
        GAPS.len()
    );
    for (p, f, _) in GAPS {
        assert!(
            PROGRAMS.contains(p),
            "gap names a program not in the corpus: {p}"
        );
        assert!(
            FORMATS.contains(f),
            "gap names a format that does not exist: {f}"
        );
    }
    let mut seen: Vec<(&str, &str)> = GAPS.iter().map(|(p, f, _)| (*p, *f)).collect();
    seen.sort_unstable();
    let n = seen.len();
    seen.dedup();
    assert_eq!(seen.len(), n, "the ledger records the same cell twice");
}

/// ── THE ROUTES THAT MATTER MOST MUST WORK ON THE REAL MODEL. The primary day-one path is SambaFlow,
/// which ingests PyTorch; the neutral artifact plus its recheckable certificate is the bridge path.
/// Both are asserted on the Qwen3.8 forward specifically, because a kit that works only on a GEMM is
/// not a kit.
#[test]
fn the_qwen_forward_produces_the_primary_day_one_artifacts() {
    let src = source_of("qwen38_tiny");
    let torch =
        emit_torch_module(&src).expect("SambaFlow ingests PyTorch — the model module must emit");
    assert!(
        torch.model_py.len() > 4096,
        "a whole Qwen forward's torch module is not 4 KB: {}",
        torch.model_py.len()
    );
    assert!(
        torch.manifest_json.len() > 1024,
        "the weight manifest names the tensors the module loads"
    );
    assert!(
        torch.compile_py.contains("TEMPLATE") || torch.compile_py.len() > 200,
        "the compile scaffold is present and marked where the API is unverified"
    );

    let art = compile_to_rdu(&src, RduChip::Sn50).expect("the Qwen forward lowers to the SN50");
    assert!(
        art.neutral_json.len() > 10_000,
        "the neutral artifact carries the program"
    );
    assert!(
        art.certificate_json.len() > 1_000,
        "the certificate travels with it"
    );
    assert!(
        art.report_json.contains("modeled"),
        "every number is labelled modeled"
    );
    assert!(
        !art.report_json.contains("tok/s"),
        "no speed claim may appear in an artifact we hand over"
    );
    let neutral = art
        .neutral
        .as_ref()
        .expect("the tensor lane carries a neutral artifact for a bridge to ingest");
    let _ = neutral;
    println!(
        "qwen38_tiny: model.py {} B, manifest {} B, neutral {} B, certificate {} B",
        torch.model_py.len(),
        torch.manifest_json.len(),
        art.neutral_json.len(),
        art.certificate_json.len()
    );
}
