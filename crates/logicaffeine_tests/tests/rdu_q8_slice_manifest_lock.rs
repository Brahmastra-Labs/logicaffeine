//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! A CHIP'S Q8_0 PLANES ARE SLICES OF THE FULL WEIGHT'S PLANES — the manifest says which.
//!
//! A tensor-parallel bundle reads every chip's slice of a weight the checkpoint stores Q8_0 as that slice's int8 values
//! and fp32 scales (`W_c__q8`, `W_c__q8_scale`). The host builds them from the checkpoint: it repacks the FULL weight's
//! Q8_0 blocks into its two planes (`q8_0.values`, `q8_0.scales`) and loads the chip's rows or columns of each. So the
//! manifest binds each plane as a slice of the full weight's plane, in the PLANE's own coordinates:
//!   • the values plane (`K × N`) at the weight's own ranges;
//!   • the scales plane (`K/32 × N`) at the same columns for a column slice, and at one row per 32 of the weight's rows
//!     for a row slice (a row slice folds only in whole Q8_0 blocks, so the division is exact);
//! each on the chip its weight slice lives on, naming the file and the repack.
//!
//! Pinned on a model small enough to emit and wide enough that its projections fold under 4 chips (`d = 64`, attention
//! heads of 64, Gated-DeltaNet heads of 32, a 256-wide FFN): every slice read as stored is bound as BOTH its planes, each
//! plane's slice exactly the coordinates above, and both kinds of slice are among them. (Which slices fold is each
//! product's own choice: a slice so small that its view outweighs its int8 saving reads dense.)
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test rdu_q8_slice_manifest_lock`
//!
//! ★ MUTATION PROBES (run 2026-09-27 in a private copy; logs `logs/sn50/probe_tpq8_*.txt`):
//!   • a row slice's scale rows left in the weight's coordinates → `Wo0_c0__q8_scale`'s ranges `[(0, 32), (128, 32)]`;
//!   • row slices never stored (`&&` for `||` in the whole-blocks rule) → no row slice among the stored;
//!   • a stored weight's slices not recorded as stored (the TP emitter's `decl_q8_slice` forgetting the checkpoint) →
//!     the layer has no stored slices.
//! ════════════════════════════════════════════════════════════════════════════════════════════════
#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::RduModel;
use logicaffeine_compile::codegen_cuda::qwen38::{gen_qwen38_tp_decode_layer_lg, Qwen38Cfg};
use logicaffeine_compile::compile::{compile_model_to_rdu_streamed, StepShape};
use logicaffeine_compile::spatial::passes::from_q8::{stored_name, stored_part, Q8Part};
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::tensor_parallel::SliceAxis;

const TP: usize = 4;
const CTX: usize = 64;

/// The mini model's head pattern at widths every projection folds at: whole Q8_0 blocks and tiles everywhere, and the
/// real model's 64 rotary dimensions (32 pairs, a tile row of angles).
fn foldable() -> Qwen38Cfg {
    Qwen38Cfg {
        d: 64,
        dff: 256,
        hd: 64,
        rope_dim: 64,
        gdn_head_dim: 32,
        vocab: 64,
        ..Qwen38Cfg::mini()
    }
}

#[test]
fn a_chips_q8_planes_are_slices_of_the_full_weights_planes() {
    let cfg = foldable();
    let bundle = compile_model_to_rdu_streamed(&RduModel::qwen38(&cfg),
        RduChip::Sn50,
        "/w",
        Some(StepShape {
            ctx: CTX,
            batch: 1,
            tp: TP,
            prefill: None,
        }),
        |_| Ok(()),
    )
    .unwrap_or_else(|e| panic!("the bundle lowers: {e}"));
    let manifest: serde_json::Value =
        serde_json::from_str(&bundle.manifest).expect("the manifest is JSON");
    let (mut planes, mut axes) = (0usize, std::collections::BTreeSet::new());
    for b in manifest["layers"].as_array().expect("layers") {
        let l = b["layer"].as_u64().expect("a layer index") as usize;
        let (_, _, q8w, m) = gen_qwen38_tp_decode_layer_lg(&cfg, l, "/w", CTX, 1, TP);
        let stored: std::collections::BTreeSet<&String> = m
            .slices
            .iter()
            .filter(|(_, s)| q8w.contains_key(&s.of))
            .map(|(name, _)| name)
            .collect();
        assert!(!stored.is_empty(), "layer {l}: the layer has stored slices");
        let mut bound = std::collections::BTreeSet::new();
        for e in b["tensors"].as_array().expect("tensors") {
            let name = e["layer"].as_str().expect("a layer name");
            let Some((w, part)) = stored_part(name) else {
                continue;
            };
            let s = m
                .slices
                .get(w)
                .unwrap_or_else(|| panic!("layer {l}: `{name}` is a plane of a chip's slice"));
            assert!(
                stored.contains(&w.to_string()),
                "layer {l}: `{w}` is a slice of a stored weight"
            );
            let slice = &e["slice"];
            assert_eq!(
                slice["of"],
                stored_name(&s.of, part),
                "layer {l}: `{name}` slices the full weight's plane"
            );
            assert_eq!(
                slice["repack"],
                part.repack(),
                "layer {l}: `{name}` names the repack that builds the full plane"
            );
            assert!(
                slice["file"].as_str().is_some_and(|f| f.ends_with(&s.of)),
                "layer {l}: `{name}` reads the full weight's file: {slice}"
            );
            assert_eq!(
                e["chip"], m.chip_of[w],
                "layer {l}: `{name}` lives on its weight slice's chip"
            );
            let (ranges, of_shape): (Vec<(usize, usize)>, (usize, usize)) = match (part, s.axis) {
                (Q8Part::Values, _) => (s.ranges.clone(), s.of_shape),
                (Q8Part::Scales, SliceAxis::Rows) => (
                    s.ranges.iter().map(|(a, n)| (a / 32, n / 32)).collect(),
                    (s.of_shape.0 / 32, s.of_shape.1),
                ),
                (Q8Part::Scales, SliceAxis::Cols) => {
                    (s.ranges.clone(), (s.of_shape.0 / 32, s.of_shape.1))
                }
            };
            assert_eq!(
                slice["axis"],
                serde_json::json!(s.axis),
                "layer {l}: `{name}` slices along its weight's axis"
            );
            assert_eq!(
                slice["ranges"],
                serde_json::json!(ranges),
                "layer {l}: `{name}`'s ranges are in its plane's coordinates"
            );
            assert_eq!(
                slice["of_shape"],
                serde_json::json!(of_shape),
                "layer {l}: `{name}`'s full plane"
            );
            bound.insert((w.to_string(), part == Q8Part::Values));
            axes.insert(s.axis == SliceAxis::Rows);
            planes += 1;
        }
        for (w, _) in &bound {
            for values in [true, false] {
                assert!(
                    bound.contains(&(w.clone(), values)),
                    "layer {l}: `{w}`'s {} are bound beside its other plane",
                    if values { "values" } else { "scales" }
                );
            }
        }
    }
    assert!(planes > 0, "the bundle binds Q8_0 planes");
    assert_eq!(
        axes.len(),
        2,
        "row slices and column slices are both read as stored"
    );
}
