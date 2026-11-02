//! Dump the EXPECTED on-chip readback slice for the whole streamed transformer.
//!
//! The DEBUG_READBACK bitstream echoes, at the load→run boundary, the first 8 entries of each of
//! {cell_kind, cell_op0, cell_op1, blk_term_kind, heap} back over UART (the `0xD8 … 0x8D` frame). To tell which
//! config BRAM the open synthesis flow corrupts on silicon, the host compares those echoed values against the
//! KNOWN config image — this test writes that known slice to `/tmp/expected_readback.json` (config build only,
//! no sim). It rebuilds the config EXACTLY as the `wxfmr`/full-UART reproducers do (same `gen`, same
//! `with_seq_reserve(8)`, same `WPE_STREAM` pipeline) so the expected slice matches the flashed engine's memories.

use logicaffeine_compile::codegen_sva::fabric_rtl::{program_cfg_fabric, with_seq_reserve};
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config_counted_heap_preloaded, serialize_program, wprog_config_image_unchecked,
};

#[path = "common/mod.rs"]
mod common;

fn gen(flags: &[&str]) -> Option<String> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let out = std::process::Command::new("python3")
        .current_dir(&root)
        .arg("scripts/gen_tiny_transformer_lg.py")
        .args(flags)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn dump_full_transformer_expected_readback_slice() {
    if !common::require_tool("python3") {
        return;
    }
    let src =
        gen(&["--fabric", "--stream"]).expect("generate the streamed fabric transformer program");
    let blob: Vec<i64> = {
        let s = gen(&["--blob"]).expect("weight blob");
        s.trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .filter(|t| !t.trim().is_empty())
            .map(|t| t.trim().parse::<i64>().expect("blob int"))
            .collect()
    };

    let (cfg, model) = with_seq_reserve(8, || {
        let cfg = serialize_program(&program_cfg_fabric(&src, "generate"));
        let (m, _steps, _heap) = eval_wprog_config_counted_heap_preloaded(&cfg, &[], &blob);
        (cfg, m)
    });
    assert_eq!(
        model, 80113,
        "the config must generate \"mat\" (80113) — else the dumped slice is stale"
    );

    let im = wprog_config_image_unchecked(&cfg);
    let n = 8usize; // DBG_SLICE — the readback FSM echoes entries [0..8) of each memory
    let slice_u32 =
        |v: &[u32]| -> Vec<i64> { (0..n).map(|i| *v.get(i).unwrap_or(&0) as i64).collect() };
    let slice_i64 = |v: &[i64]| -> Vec<i64> { (0..n).map(|i| *v.get(i).unwrap_or(&0)).collect() };

    // heap[0..8] == the pre-loaded weight blob's head (heap is the DATA-segment the blob streams into).
    let mems: [(&str, Vec<i64>); 5] = [
        ("cell_kind", slice_u32(&im.cell_kind)),
        ("cell_op0", slice_u32(&im.cell_op0)),
        ("cell_op1", slice_u32(&im.cell_op1)),
        ("blk_term_kind", slice_u32(&im.blk_term_kind)),
        ("heap", slice_i64(&blob)),
    ];

    let body = mems
        .iter()
        .map(|(name, vals)| {
            let nums = vals
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            format!("  \"{name}\": [{nums}]")
        })
        .collect::<Vec<_>>()
        .join(",\n");
    let json = format!("{{\n{body}\n}}\n");
    std::fs::write("/tmp/expected_readback.json", &json).expect("write expected_readback.json");
    eprintln!("wrote /tmp/expected_readback.json (model={model}):\n{json}");
}
