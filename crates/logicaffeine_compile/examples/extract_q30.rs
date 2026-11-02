//! Standalone extractor: unpack the real Qwen3-30B-A3B Q8_0 GGUF into the per-tensor weight files the emitted
//! forward `.cu` fopens (`<dir>/wv{l}.q` …). A compiled artifact so the token-exactness check can run on a GPU
//! pod without shipping source. Mirrors the extraction in `tests/cuda_qwen30b_forward_lock.rs`.
//!
//! Usage: extract_q30 <gguf> <out_dir> <token>
use logicaffeine_compile::model::gguf::Gguf;
use std::path::Path;

fn write_i8(p: &Path, v: &[i8]) {
    std::fs::write(p, v.iter().map(|&q| q as u8).collect::<Vec<u8>>()).unwrap();
}
fn write_f32(p: &Path, v: &[f32]) {
    let mut b = Vec::with_capacity(v.len() * 4);
    for x in v {
        b.extend_from_slice(&x.to_le_bytes());
    }
    std::fs::write(p, b).unwrap();
}
fn quantize_q8(data: &[f32], out: usize, inn: usize) -> (Vec<i8>, Vec<f32>) {
    let nb = inn / 32;
    let mut q = vec![0i8; out * inn];
    let mut s = vec![0f32; out * nb];
    for o in 0..out {
        for b in 0..nb {
            let blk = &data[o * inn + b * 32..o * inn + b * 32 + 32];
            let amax = blk.iter().map(|x| x.abs()).fold(1e-30f32, f32::max);
            let scale = amax / 127.0;
            s[o * nb + b] = scale;
            for j in 0..32 {
                q[o * inn + b * 32 + j] = (blk[j] / scale).round().clamp(-127.0, 127.0) as i8;
            }
        }
    }
    (q, s)
}
fn extract_q8(g: &mut Gguf, dir: &Path, tag: &str, name: &str) {
    let (q, s) = g
        .q8_0_matrix(name)
        .unwrap_or_else(|e| panic!("q8 {name}: {e}"));
    write_i8(&dir.join(format!("{tag}.q")), &q);
    write_f32(&dir.join(format!("{tag}.s")), &s);
}
fn extract_router(g: &mut Gguf, dir: &Path, tag: &str, name: &str, ne: usize, d: usize) {
    let (q, s) = match g.q8_0_matrix(name) {
        Ok(qs) => qs,
        Err(_) => {
            let raw = g
                .f32_tensor(name)
                .unwrap_or_else(|e| panic!("router {name}: {e}"));
            quantize_q8(&raw, ne, d)
        }
    };
    write_i8(&dir.join(format!("{tag}.q")), &q);
    write_f32(&dir.join(format!("{tag}.s")), &s);
}
fn extract_f32(g: &mut Gguf, dir: &Path, var: &str, name: &str) {
    let v = g
        .f32_tensor(name)
        .unwrap_or_else(|e| panic!("f32 {name}: {e}"));
    write_f32(&dir.join(format!("{var}.f")), &v);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let gguf = &args[1];
    let dir = Path::new(&args[2]);
    let token: usize = args[3].parse().unwrap();
    std::fs::create_dir_all(dir).unwrap();
    let mut g = Gguf::open(Path::new(gguf)).expect("open gguf");
    let (d, h, kv, hd, layers) = (
        g.arch.embedding_length as usize,
        g.arch.head_count as usize,
        g.arch.head_count_kv as usize,
        g.arch.head_dim as usize,
        g.arch.block_count as usize,
    );
    let ed = g
        .dims("blk.0.ffn_gate_exps.weight")
        .expect("ffn_gate_exps dims")
        .to_vec();
    let (dff, ne) = (ed[1] as usize, ed[2] as usize);
    let vocab = g.dims2("output.weight").expect("output dims").1;
    eprintln!("30B arch: d={d} dff={dff} E={ne} layers={layers} h={h} kv={kv} hd={hd} vocab={vocab} token={token}");
    // Fast mode: `qknorm` as a 4th arg extracts ONLY the per-head QK-RMSNorm weights (attn_q_norm/attn_k_norm,
    // f32 [hd]) into an EXISTING weight dir — the pieces autoregressive generation needs (RoPE at pos>0). Avoids
    // re-reading the 32 GB of Q8 weights already extracted.
    if args.get(4).map(|s| s.as_str()) == Some("qknorm") {
        for l in 0..layers {
            extract_f32(
                &mut g,
                dir,
                &format!("aqn{l}"),
                &format!("blk.{l}.attn_q_norm.weight"),
            );
            extract_f32(
                &mut g,
                dir,
                &format!("akn{l}"),
                &format!("blk.{l}.attn_k_norm.weight"),
            );
        }
        // The FULL token-embedding table (Q8) — generation must embed ANY produced token, not just the seed row.
        extract_q8(&mut g, dir, "emb", "token_embd.weight");
        eprintln!(
            "QK-NORM + EMBEDDING EXTRACTION COMPLETE ({layers} layers) → {}",
            dir.display()
        );
        return;
    }
    let emb = g
        .q8_0_dequant_row("token_embd.weight", token)
        .expect("embed row");
    write_f32(&dir.join("x.f"), &emb);
    for l in 0..layers {
        extract_f32(
            &mut g,
            dir,
            &format!("aqn{l}"),
            &format!("blk.{l}.attn_q_norm.weight"),
        );
        extract_f32(
            &mut g,
            dir,
            &format!("akn{l}"),
            &format!("blk.{l}.attn_k_norm.weight"),
        );
        extract_f32(
            &mut g,
            dir,
            &format!("an{l}"),
            &format!("blk.{l}.attn_norm.weight"),
        );
        extract_f32(
            &mut g,
            dir,
            &format!("fn{l}"),
            &format!("blk.{l}.ffn_norm.weight"),
        );
        // FULL GQA attention: real Wq/Wk/Wv (no fold) + Wo — the same weights llama.cpp streams.
        extract_q8(
            &mut g,
            dir,
            &format!("wq{l}"),
            &format!("blk.{l}.attn_q.weight"),
        );
        extract_q8(
            &mut g,
            dir,
            &format!("wk{l}"),
            &format!("blk.{l}.attn_k.weight"),
        );
        extract_q8(
            &mut g,
            dir,
            &format!("wv{l}"),
            &format!("blk.{l}.attn_v.weight"),
        );
        extract_q8(
            &mut g,
            dir,
            &format!("wo{l}"),
            &format!("blk.{l}.attn_output.weight"),
        );
        extract_router(
            &mut g,
            dir,
            &format!("wr{l}"),
            &format!("blk.{l}.ffn_gate_inp.weight"),
            ne,
            d,
        );
        extract_q8(
            &mut g,
            dir,
            &format!("wg{l}"),
            &format!("blk.{l}.ffn_gate_exps.weight"),
        );
        extract_q8(
            &mut g,
            dir,
            &format!("wu{l}"),
            &format!("blk.{l}.ffn_up_exps.weight"),
        );
        extract_q8(
            &mut g,
            dir,
            &format!("wd{l}"),
            &format!("blk.{l}.ffn_down_exps.weight"),
        );
        eprintln!("layer {l} done");
    }
    extract_f32(&mut g, dir, "on", "output_norm.weight");
    extract_q8(&mut g, dir, "wout", "output.weight");
    eprintln!("EXTRACTION COMPLETE → {}", dir.display());
}
