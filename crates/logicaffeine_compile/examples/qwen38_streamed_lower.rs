//! Lower the REAL Qwen3.8-27B one layer at a time: every layer lowered to the SN50 spatial IR and certified,
//! then dropped before the next. Prints each layer and the process's peak memory over the whole run — if
//! streaming works, that peak is one layer's, not sixty-four layers'.
//!
//! `cargo run --release -p logicaffeine-compile --example qwen38_streamed_lower -- [layers]`
use logicaffeine_compile::codegen_cuda::qwen38::{
    gen_qwen38_head_lg, gen_qwen38_layer_lg, Qwen38Cfg,
};
use logicaffeine_compile::spatial::certificate::{certify_choreography, schedule_digest};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;

fn peak_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find(|l| l.starts_with("VmHWM:"))
        .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        .unwrap_or(0)
}

fn lower(label: &str, src: &str) -> bool {
    let target = RduChip::Sn50.facts().spatial_target();
    let t = std::time::Instant::now();
    match compile_program_to_spatial(src, &target) {
        Ok(l) => match l.program {
            Some(mut p) => {
                p.certificate = None;
                let cert = certify_choreography(&p);
                println!(
                    "{label:<22} {:>3}/{:<3} stmts  {:>4} sections  {:>7} stages  cert {}  digest {:016x}  {:>6.1}s  peak {:>6.1} GB",
                    l.lowered,
                    l.statements,
                    p.sections.len(),
                    p.stages.len(),
                    if cert.is_ok() { "ok " } else { "ERR" },
                    schedule_digest(&p),
                    t.elapsed().as_secs_f64(),
                    peak_kb() as f64 / 1e6
                );
                cert.is_ok()
            }
            None => {
                println!(
                    "{label:<22} NOT WHOLE: {:?}",
                    l.missing.iter().take(2).map(|m| &m.why).collect::<Vec<_>>()
                );
                false
            }
        },
        Err(e) => {
            println!("{label:<22} REJECTED: {e:?}");
            false
        }
    }
}

fn main() {
    let mut cfg = Qwen38Cfg::real();
    if let Some(n) = std::env::args().nth(1).and_then(|v| v.parse().ok()) {
        cfg.layers = n;
    }
    println!(
        "Qwen3.8 real: {} layers, d={}, dff={}, vocab={} — streamed, one layer resident at a time",
        cfg.layers, cfg.d, cfg.dff, cfg.vocab
    );
    let t = std::time::Instant::now();
    let mut ok = 0usize;
    for l in 0..cfg.layers {
        let (src, _, _) = gen_qwen38_layer_lg(&cfg, l, "/w");
        let kind = if cfg.is_attn(l) {
            "attention"
        } else {
            "gated deltanet"
        };
        if lower(&format!("layer {l:>2} {kind}"), &src) {
            ok += 1;
        }
    }
    let (head, _, _) = gen_qwen38_head_lg(&cfg, "/w");
    let head_ok = lower("head (norm + LM head)", &head);
    println!(
        "LOWERED {ok}/{} layers + head {} in {:.0}s — process peak {:.1} GB",
        cfg.layers,
        if head_ok { "ok" } else { "FAILED" },
        t.elapsed().as_secs_f64(),
        peak_kb() as f64 / 1e6
    );
}
