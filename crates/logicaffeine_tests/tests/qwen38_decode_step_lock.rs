//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ★ ONE QWEN3.8 DECODE STEP ON THE SN50 RDU ★ — token in, next token out, the KV cache carried.
//!
//! `codegen_cuda::qwen38::gen_qwen38_decode_step_lg` writes one greedy decode step as a LOGOS program: row `tok`
//! of the embedding table is the hidden state, every attention layer appends this token's key and value to its
//! KV cache as row `pos` and attends over the first `pos` rows (`attention … up to pos`), the Gated-DeltaNet
//! layers carry their memories and conv rings, and the step ends with the arg-max of the logits. Every piece of
//! it — the row gather, the row append, the masked attention, the arg-max — lowers through the spatial compiler,
//! so the whole step is ONE certified program on the SN50.
//!
//! The promises:
//!   1. THE STEP IS THE AUTHORED FORWARD, WRAPPED. Every statement of the authored forward that is not an
//!      attention statement or a rotary one appears in the step verbatim; the step opens with the token and its
//!      embedding row and closes with the arg-max. The rotary angles are the model's position caches: the step declares
//!      `Rcos` / `Rsin` of `ctx` positions, and every attention layer turns its queries and keys by them from `pos`.
//!   2. IT LOWERS WHOLE. Every statement is lifted, the composition certifies, and the RDU lane ships it with a
//!      certificate that rechecks.
//!   3. IT EMITS THE VM'S TOKEN. The step's `next` equals the VM's exactly — the token is what the model emits —
//!      and the logits stay within the whole-forward lock's stated bf16 bound.
//!   4. IT APPENDS THIS TOKEN TO THE CACHE. Row `pos` of every layer's key and value cache holds, after the step,
//!      what the VM holds there.
//!   5. EVERY ATTENTION LAYER ATTENDS OVER THE FIRST `pos` ROWS. A layer's attention output, read where it is the
//!      result, is the VM's. The token and the logits are downstream of three more layers and cannot see a mask
//!      that went missing — the attention output can.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test qwen38_decode_step_lock`
//!
//! ★ MUTATION PROBE (private copy, RED 2026-09-26): the decode step's ropes read the caches from position 1 instead of
//! `pos` → promise 1: `layer 3 turns qm3 by the caches from pos`.
//!
//! ★ MUTATION PROBE (run when this landed): lower `up to pos` as no limit (`from_attention`'s `RowIndex::Named` arm
//! returning `(l, None)`) — the token and the logits STAY within their bound (0.00459 drift, the mask diluted by
//! three more layers), and promise 5 reds at a 1.5452 drift on the attention output. That gap is why promise 5
//! exists.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_cuda::qwen38::{
    gen_qwen38_decode_step_lg, gen_qwen38_lg, Qwen38Cfg,
};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::ir::{Phase, SpatialProgram};
use logicaffeine_compile::spatial::lower::rdu::RduSpatialBackend;
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;

#[path = "common/spatial.rs"]
mod spatial_support;
use spatial_support::inputs_from_fills;

const CTX: usize = 6;
const TOK: usize = 5;
const POS: usize = 3;

/// `qwen38_rdu_lock`'s stated bf16 bound for a whole forward, relative to the largest logit.
const BF16_WHOLE_FORWARD_REL: f64 = 0.02;

/// A deterministic fill after every declared tensor — `qwen38_authored_lock`'s scheme, verbatim. The caches are
/// filled too: rows before `pos` are the earlier tokens' keys and values, rows past it are masked out.
fn with_fills(src: &str) -> String {
    let mut out = String::new();
    for line in src.lines() {
        out.push_str(line);
        out.push('\n');
        let Some(rest) = line.strip_prefix("Let ") else {
            continue;
        };
        let Some((var, tail)) = rest.split_once(" be a ") else {
            continue;
        };
        let Some(dims) = tail.strip_suffix(" tensor of Real.") else {
            continue;
        };
        let (r, c) = dims.split_once(" by ").expect("R by C");
        let n = r.parse::<usize>().unwrap() * c.parse::<usize>().unwrap();
        let salt = var.bytes().map(|b| b as u64).sum::<u64>();
        for i in 0..n {
            let h = (i as u64)
                .wrapping_mul(2654435761)
                .wrapping_add(salt.wrapping_mul(40503));
            let v = ((h % 97) as f64 / 97.0 - 0.5) * 0.4;
            out.push_str(&format!("Set item {} of {var} to {v:?}.\n", i + 1));
        }
    }
    out
}

fn step(cfg: &Qwen38Cfg) -> String {
    gen_qwen38_decode_step_lg(cfg, "/w", CTX, TOK, POS).0
}

fn attention_layers(cfg: &Qwen38Cfg) -> Vec<usize> {
    (0..cfg.layers).filter(|l| cfg.is_attn(*l)).collect()
}

fn composed(src: &str) -> (SpatialProgram, usize) {
    let target = RduChip::Sn50.facts().spatial_target();
    let pl = compile_program_to_spatial(src, &target).expect("the walk lifts the step");
    let p = pl
        .whole()
        .unwrap_or_else(|| panic!("the decode step lowers whole: {}", pl.refusal()))
        .clone();
    (p, pl.sections)
}

fn floats(v: &Value) -> Vec<f64> {
    match v {
        Value::Float(x) => x.clone(),
        Value::Int(x) => x.iter().map(|i| *i as f64).collect(),
    }
}

// ── 1. the step is the authored forward, wrapped ─────────────────────────────────────────────────────

#[test]
fn the_step_is_the_authored_forward_wrapped_token_in_token_out() {
    let cfg = Qwen38Cfg::tiny();
    let (authored, _, _) = gen_qwen38_lg(&cfg, "/w");
    let s = step(&cfg);
    let rotary = |l: &str| {
        l.contains("rope of") || l.starts_with("Let RC be a ") || l.starts_with("Let RS be a ")
    };
    for line in authored
        .lines()
        .filter(|l| !l.contains("attention of") && !l.starts_with("Let x be a ") && !rotary(l))
    {
        assert!(
            s.lines().any(|m| m == line),
            "the step keeps the authored statement `{line}`"
        );
    }
    let half = cfg.rope_dim / 2;
    for cache in ["Rcos", "Rsin"] {
        assert!(
            s.contains(&format!("Let {cache} be a {CTX} by {half} tensor of Real.")),
            "the step declares the {cache} cache of {CTX} positions"
        );
    }
    assert!(
        !s.lines()
            .any(|l| l.starts_with("Let RC be a ") || l.starts_with("Let RS be a ")),
        "the step binds no one-position table of angles"
    );
    let lines: Vec<&str> = s.lines().collect();
    let head = [
        "## Main".to_string(),
        format!("Let tok be {TOK}."),
        format!("Let pos be {POS}."),
    ];
    assert_eq!(
        lines[..3].iter().map(|l| l.to_string()).collect::<Vec<_>>(),
        head,
        "the step opens with its token and position"
    );
    assert!(
        s.contains("Let x be row tok of Emb."),
        "the hidden state is the token's embedding row"
    );
    for l in attention_layers(&cfg) {
        assert!(
            s.contains(&format!("Set row pos of Kc{l} to kp{l}."))
                && s.contains(&format!("Set row pos of Vc{l} to vr{l}.")),
            "layer {l} appends to its cache"
        );
        assert!(s.contains(&format!("Let ct{l} be attention of qp{l} over Kc{l} and Vc{l} with {} heads {} groups up to pos.", cfg.h, cfg.kv)), "layer {l} attends over the cache up to pos");
        for (out, x, heads) in [
            (format!("qp{l}"), format!("qm{l}"), cfg.h),
            (format!("kp{l}"), format!("km{l}"), cfg.kv),
        ] {
            assert!(s.contains(&format!("Let {out} be rope of {x} with cos Rcos and sin Rsin from position pos across {heads} heads.")), "layer {l} turns {x} by the caches from pos");
        }
    }
    assert_eq!(
        lines.last(),
        Some(&"Let next be argmax of logits."),
        "the step closes with the token a greedy decode emits"
    );
}

// ── 2. it lowers whole ───────────────────────────────────────────────────────────────────────────────

#[test]
fn the_tiny_decode_step_lowers_whole_and_the_rdu_lane_ships_it() {
    let cfg = Qwen38Cfg::tiny();
    let src = with_fills(&step(&cfg));
    let (p, sections) = composed(&src);
    assert_eq!(p.phase(), Phase::Verified, "the composed step certifies");
    assert!(
        p.certificate.as_ref().is_some_and(|c| c.recheck(&p)),
        "the step's certificate rechecks"
    );
    assert!(
        sections > cfg.layers,
        "every layer contributes its sections ({sections})"
    );
    match RduSpatialBackend::for_chip(RduChip::Sn50)
        .lower(&p)
        .expect("the SN50 lane lowers the step")
    {
        SpatialArtifact::SdkGated(a) => assert!(
            a.certificate.recheck(&p),
            "the shipped certificate rechecks against the step"
        ),
        other => panic!("the RDU lane is SDK-gated, got {other:?}"),
    }
}

// ── 3 & 4. it emits the VM's token, and appends to the cache ─────────────────────────────────────────

#[test]
fn the_tiny_decode_step_emits_the_vms_token_and_appends_this_token_to_the_cache() {
    let cfg = Qwen38Cfg::tiny();
    let akv = cfg.akv();
    let mut src = with_fills(&step(&cfg));
    src.push_str("Show next.\n");
    for i in 1..=cfg.vocab {
        src.push_str(&format!("Show round(item {i} of logits * 1000000).\n"));
    }
    for l in attention_layers(&cfg) {
        for c in ["Kc", "Vc"] {
            for e in 1..=akv {
                src.push_str(&format!(
                    "Show round(item {} of {c}{l} * 1000000).\n",
                    (POS - 1) * akv + e
                ));
            }
        }
    }
    let vm = vm_outcome(&src);
    assert!(
        vm.error.is_none(),
        "the decode step runs on the VM: {:?}",
        vm.error
    );
    let want: Vec<i64> = vm
        .output
        .split_whitespace()
        .map(|t| t.parse().expect("an integer"))
        .collect();
    let (vm_next, rest) = want.split_first().expect("the token");
    let (vm_logits, vm_rows) = rest.split_at(cfg.vocab);

    let (p, _) = composed(&src);
    let out = eval_choreography(&p, &inputs_from_fills(&p)).expect("the composed step evaluates");
    let tensor = |name: &str| {
        let id = p
            .tensors
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("the step carries `{name}`"))
            .id;
        floats(
            out.tensors
                .get(&id)
                .unwrap_or_else(|| panic!("the choreography writes `{name}`")),
        )
    };
    let next = tensor("next");
    assert_eq!(next, vec![*vm_next as f64], "the step emits the VM's token");

    // `logits` is an ACTIVATION of the step — the arg-max consumes it, and an evaluation hands back results and
    // carried state only — so its values are read from the same step lowered without that last statement, where
    // `logits` is the result.
    let open: String = src
        .lines()
        .filter(|l| *l != "Let next be argmax of logits." && *l != "Show next.")
        .map(|l| format!("{l}\n"))
        .collect();
    let (q, _) = composed(&open);
    let open_out = eval_choreography(&q, &inputs_from_fills(&q))
        .expect("the step without its arg-max evaluates");
    let logits_id = q
        .tensors
        .iter()
        .find(|t| t.name == "logits")
        .expect("the open step's result is `logits`")
        .id;
    let logits: Vec<i64> = floats(
        open_out
            .tensors
            .get(&logits_id)
            .expect("the open step writes `logits`"),
    )
    .iter()
    .map(|x| (x * 1e6).round() as i64)
    .collect();
    let scale = vm_logits.iter().map(|x| x.abs()).max().unwrap_or(1).max(1) as f64;
    let worst = logits
        .iter()
        .zip(vm_logits)
        .map(|(a, b)| ((a - b) as f64).abs() / scale)
        .fold(0.0, f64::max);
    assert!(worst <= BF16_WHOLE_FORWARD_REL, "the step's logits drift {worst:.4} from the VM, past {BF16_WHOLE_FORWARD_REL}\n  ours: {logits:?}\n  vm:   {vm_logits:?}");

    let mut rows = vm_rows.chunks(akv);
    for l in attention_layers(&cfg) {
        for c in ["Kc", "Vc"] {
            let want_row = rows.next().expect("a cache row");
            let cache = tensor(&format!("{c}{l}"));
            let got: Vec<i64> = cache[(POS - 1) * akv..POS * akv]
                .iter()
                .map(|x| (x * 1e6).round() as i64)
                .collect();
            let rscale = want_row.iter().map(|x| x.abs()).max().unwrap_or(1).max(1) as f64;
            let rworst = got
                .iter()
                .zip(want_row)
                .map(|(a, b)| ((a - b) as f64).abs() / rscale)
                .fold(0.0, f64::max);
            assert!(rworst <= BF16_WHOLE_FORWARD_REL, "{c}{l} row {POS}: the appended row drifts {rworst:.4}\n  ours: {got:?}\n  vm:   {want_row:?}");
        }
    }
    println!(
        "Qwen3.8 (tiny) decode step on the SN50: token {vm_next} agrees, logits drift {worst:.5}"
    );
}

// ── 5. every attention layer attends over the first `pos` rows ───────────────────────────────────────

#[test]
fn every_attention_layer_of_the_tiny_step_attends_over_the_first_pos_rows() {
    let cfg = Qwen38Cfg::tiny();
    let full = with_fills(&step(&cfg));
    for l in attention_layers(&cfg) {
        let stmt = format!("Let ct{l} be attention of qp{l} over Kc{l} and Vc{l} with {} heads {} groups up to pos.", cfg.h, cfg.kv);
        let cut = full
            .lines()
            .position(|line| line == stmt)
            .unwrap_or_else(|| panic!("layer {l} attends up to pos"));
        let open: String = full
            .lines()
            .take(cut + 1)
            .map(|line| format!("{line}\n"))
            .collect();
        let (q, _) = composed(&open);
        let out = eval_choreography(&q, &inputs_from_fills(&q))
            .expect("the step up to the attention evaluates");
        let id = q
            .tensors
            .iter()
            .find(|t| t.name == format!("ct{l}"))
            .expect("the attention output is the result")
            .id;
        let ours: Vec<i64> = floats(out.tensors.get(&id).expect("written"))
            .iter()
            .map(|x| (x * 1e6).round() as i64)
            .collect();
        let mut shown = open.clone();
        for i in 1..=ours.len() {
            shown.push_str(&format!("Show round(item {i} of ct{l} * 1000000).\n"));
        }
        let vm = vm_outcome(&shown);
        assert!(vm.error.is_none(), "{:?}", vm.error);
        let want: Vec<i64> = vm
            .output
            .split_whitespace()
            .map(|t| t.parse().expect("an integer"))
            .collect();
        let scale = want.iter().map(|x| x.abs()).max().unwrap_or(1).max(1) as f64;
        let worst = ours
            .iter()
            .zip(&want)
            .map(|(a, b)| ((a - b) as f64).abs() / scale)
            .fold(0.0, f64::max);
        assert!(worst <= BF16_WHOLE_FORWARD_REL, "layer {l}: the attention output drifts {worst:.4} from the VM\n  ours: {ours:?}\n  vm:   {want:?}");
        println!(
            "layer {l}: attention up to pos {POS} of {CTX} agrees with the VM, drift {worst:.5}"
        );
    }
}

// ── 6. at real width ─────────────────────────────────────────────────────────────────────────────────

/// The decode step's new statements at Qwen3.8's real geometry — 24 query heads over 4 KV heads at head dim 256,
/// a 4096-position cache — lower whole and certify: the cache append moves exactly one row of each cache, and the
/// masked attention streams the cache's 128 blocks through ONE section that folds them on chip, an item per KV head,
/// between the regrouping of the query heads by KV head and of the result back.
#[test]
fn a_real_width_cache_append_and_masked_attention_lower_and_certify() {
    let cfg = Qwen38Cfg::real();
    let (ctx, pos) = (4096usize, 1000usize);
    let (aq, akv) = (cfg.aq(), cfg.akv());
    let src = format!(
        "## Main\nLet pos be {pos}.\nLet Kc be a {ctx} by {akv} tensor of Real.\nLet Vc be a {ctx} by {akv} tensor of Real.\n\
         Let qp be a 1 by {aq} tensor of Real.\nLet kp be a 1 by {akv} tensor of Real.\nLet vr be a 1 by {akv} tensor of Real.\n\
         Set row pos of Kc to kp.\nSet row pos of Vc to vr.\n\
         Let ct be attention of qp over Kc and Vc with {} heads {} groups up to pos.\nShow round(item 1 of ct * 1000000).\n",
        cfg.h, cfg.kv
    );
    let (p, sections) = composed(&src);
    assert_eq!(
        p.phase(),
        Phase::Verified,
        "the real-width step statements certify"
    );
    assert!(
        p.certificate.as_ref().is_some_and(|c| c.recheck(&p)),
        "and the certificate rechecks"
    );
    assert_eq!(sections, 5, "two appends, the queries regrouped, the one block section, the result back — the blocks fold on chip, nothing merges after them");
    let blocks: Vec<(Option<usize>, usize)> = p
        .sections
        .iter()
        .filter(|s| s.name.contains("flash_blocks"))
        .map(|s| (s.plan.map(|pl| pl.inner.num_blocks), s.items.1 - s.items.0))
        .collect();
    assert_eq!(
        blocks,
        vec![(Some(ctx / 32), cfg.kv)],
        "the block section folds the cache's {} tile rows, an item per KV head",
        ctx / 32
    );
    let row_tiles = akv / 32;
    for cache in ["Kc", "Vc"] {
        let moved: usize = p
            .stages
            .iter()
            .filter_map(|s| match &s.kind {
                logicaffeine_compile::spatial::ir::StageKind::Store {
                    tensor, pattern, ..
                } if p.tensor(*tensor).name == cache && pattern.origin.indirect.is_some() => {
                    let items = s
                        .site
                        .map(|site| p.items_at_in(p.section_of_stage(s.id), site).len())
                        .unwrap_or(1);
                    Some(pattern.block_h * pattern.block_w * items)
                }
                _ => None,
            })
            .sum();
        assert_eq!(moved, row_tiles, "{cache}: the append moves exactly one row ({row_tiles} tiles) of the {ctx}-position cache");
    }
    println!("real-width cache append + masked attention: {sections} sections, certified");
}
