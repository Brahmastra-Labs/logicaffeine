//! Verify `extract_qwen38`'s output against a fresh read of the GGUF.
//!
//! The CPU reference reads the checkpoint directly and reproduces llama.cpp's golden tokens; the CUDA lane
//! reads these extracted files instead. So any divergence between the two paths shows up here first.
//!
//! Usage: verify_extract <gguf> <w38_dir>
use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::codegen_cuda::qwen38::Qwen38Cfg;
use std::path::Path;

fn read_i8(p: &Path) -> Vec<i8> {
    std::fs::read(p)
        .unwrap_or_else(|e| panic!("{p:?}: {e}"))
        .into_iter()
        .map(|b| b as i8)
        .collect()
}
fn read_f32(p: &Path) -> Vec<f32> {
    std::fs::read(p)
        .unwrap_or_else(|e| panic!("{p:?}: {e}"))
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}
fn cmp_i8(tag: &str, got: &[i8], want: &[i8]) {
    if got.len() != want.len() {
        println!("  {tag}: LENGTH {} vs {}", got.len(), want.len());
        return;
    }
    let bad = got.iter().zip(want).filter(|(a, b)| a != b).count();
    println!(
        "  {tag}: {} / {} quants differ{}",
        bad,
        got.len(),
        if bad == 0 { "  OK" } else { "  <-- MISMATCH" }
    );
}
fn cmp_f32(tag: &str, got: &[f32], want: &[f32]) {
    if got.len() != want.len() {
        println!("  {tag}: LENGTH {} vs {}", got.len(), want.len());
        return;
    }
    let bad = got
        .iter()
        .zip(want)
        .filter(|(a, b)| (*a - *b).abs() > 1e-6)
        .count();
    println!(
        "  {tag}: {} / {} values differ{}",
        bad,
        got.len(),
        if bad == 0 { "  OK" } else { "  <-- MISMATCH" }
    );
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (gguf, dir) = (Path::new(&a[1]), Path::new(&a[2]));
    let mut g = Gguf::open(gguf).expect("open gguf");
    let cfg = Qwen38Cfg::real();
    let (qk, vw) = (cfg.gdn_qk(), cfg.gdn_v());

    println!("== global ==");
    let (eq, es) = g.q8_0_matrix("token_embd.weight").unwrap();
    cmp_i8("emb.q", &read_i8(&dir.join("emb.q")), &eq);
    cmp_f32("emb.s", &read_f32(&dir.join("emb.s")), &es);
    cmp_f32(
        "on.f",
        &read_f32(&dir.join("on.f")),
        &g.f32_tensor("output_norm.weight").unwrap(),
    );

    println!("== layer 0 (GDN) ==");
    cmp_f32(
        "an0.f",
        &read_f32(&dir.join("an0.f")),
        &g.f32_tensor("blk.0.attn_norm.weight").unwrap(),
    );
    // qkvz0 = attn_qkv (q|k|v) ++ attn_gate (z)
    let (pq, ps) = g.q8_0_matrix("blk.0.attn_qkv.weight").unwrap();
    let (zq, zs) = g.q8_0_matrix("blk.0.attn_gate.weight").unwrap();
    let mut wq = pq.clone();
    wq.extend_from_slice(&zq);
    let mut ws = ps.clone();
    ws.extend_from_slice(&zs);
    cmp_i8("qkvz0.q", &read_i8(&dir.join("qkvz0.q")), &wq);
    cmp_f32("qkvz0.s", &read_f32(&dir.join("qkvz0.s")), &ws);
    // ba0 = ssm_beta ++ ssm_alpha
    let mut ba = g.f32_tensor("blk.0.ssm_beta.weight").unwrap();
    ba.extend_from_slice(&g.f32_tensor("blk.0.ssm_alpha.weight").unwrap());
    cmp_f32("ba0.f", &read_f32(&dir.join("ba0.f")), &ba);
    // Wc0 = ssm_conv1d TRANSPOSED to tap-major
    let cw = g.f32_tensor("blk.0.ssm_conv1d.weight").unwrap();
    let (t, c) = (cfg.conv_taps, 2 * qk + vw);
    let mut tw = vec![0f32; t * c];
    for ch in 0..c {
        for tap in 0..t {
            tw[tap * c + ch] = cw[ch * t + tap];
        }
    }
    cmp_f32("Wc0.f (transposed)", &read_f32(&dir.join("Wc0.f")), &tw);
    cmp_f32(
        "Al0.f",
        &read_f32(&dir.join("Al0.f")),
        &g.f32_tensor("blk.0.ssm_a").unwrap(),
    );
    cmp_f32(
        "Dt0.f",
        &read_f32(&dir.join("Dt0.f")),
        &g.f32_tensor("blk.0.ssm_dt.bias").unwrap(),
    );
    cmp_f32(
        "Gn0.f",
        &read_f32(&dir.join("Gn0.f")),
        &g.f32_tensor("blk.0.ssm_norm.weight").unwrap(),
    );
    let (oq, os) = g.q8_0_matrix("blk.0.ssm_out.weight").unwrap();
    cmp_i8("Wo0.q", &read_i8(&dir.join("Wo0.q")), &oq);
    cmp_f32("Wo0.s", &read_f32(&dir.join("Wo0.s")), &os);
    // gu0 = ffn_gate ++ ffn_up
    let (gq, gs) = g.q8_0_matrix("blk.0.ffn_gate.weight").unwrap();
    let (uq, us) = g.q8_0_matrix("blk.0.ffn_up.weight").unwrap();
    let mut cq = gq.clone();
    cq.extend_from_slice(&uq);
    let mut cs = gs.clone();
    cs.extend_from_slice(&us);
    cmp_i8("gu0.q", &read_i8(&dir.join("gu0.q")), &cq);
    cmp_f32("gu0.s", &read_f32(&dir.join("gu0.s")), &cs);
    cmp_f32(
        "fn0.f",
        &read_f32(&dir.join("fn0.f")),
        &g.f32_tensor("blk.0.post_attention_norm.weight").unwrap(),
    );
}
