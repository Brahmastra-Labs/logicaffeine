//! The REAL Qwen3.8-27B, all 64 layers, lowered as one program per layer kind plus the head, with a
//! manifest binding every layer. Prints each lowered program and the manifest's shape.
use logicaffeine_compile::spatial::streamed::Storage;
use logicaffeine_compile::codegen_cuda::qwen38::Qwen38Cfg;
use logicaffeine_compile::spatial::streamed::lower_forward_streamed;
use logicaffeine_compile::spatial::rdu_chip::RduChip;

fn peak_gb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find(|l| l.starts_with("VmHWM:"))
        .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
        .unwrap_or(0.0)
        / 1e6
}

fn main() {
    let cfg = Qwen38Cfg::real();
    let t = std::time::Instant::now();
    let target = RduChip::Sn50.facts().spatial_target();
    let m =
        lower_forward_streamed(&cfg.spec(), Storage::AllQ8, &target, |s| s.to_string()).expect("the streamed model lowers");
    for k in &m.kinds {
        let uses = m
            .layers
            .iter()
            .filter(|b| m.kinds[b.kind].name == k.name)
            .count();
        println!("{:<16} from layer {:>2}: {:>4} sections {:>7} stages  cert rechecks {}  runs at {uses} layers",
            k.name, k.representative, k.program.sections.len(), k.program.stages.len(),
            k.program.certificate.as_ref().is_some_and(|c| c.recheck(&k.program)));
    }
    println!(
        "head: {} sections {} stages  cert rechecks {}",
        m.head.sections.len(),
        m.head.stages.len(),
        m.head
            .certificate
            .as_ref()
            .is_some_and(|c| c.recheck(&m.head))
    );
    println!(
        "manifest: {} layers bound, {} tensor bindings; {} programs lowered in total",
        m.layers.len(),
        m.layers.iter().map(|b| b.tensors.len()).sum::<usize>(),
        m.kinds.len() + 1
    );
    println!(
        "ALL {} LAYERS + HEAD in {:.0}s, peak {:.1} GB",
        cfg.layers,
        t.elapsed().as_secs_f64(),
        peak_gb()
    );
}
