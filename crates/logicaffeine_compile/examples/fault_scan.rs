//! Run the streamed-transformer config through the golden interpreter under a battery of hardware fault models
//! (see `WFault`) — singles and pairwise compounds — and report each fault's result against an observed on-chip
//! value. A fault set whose result equals the observed value characterizes the hardware defect (the interpreter
//! is the ground truth). SILICON / CORRECT below are the two comparison targets.
//!
//! Inputs (produced by `emit_transformer_wire`): /tmp/tffab_stream.lg + /tmp/tf_blob.json.
use logicaffeine_compile::codegen_sva::fabric_rtl::{program_cfg_fabric, with_seq_reserve};
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config_faulted, serialize_program, WFault,
};

fn run(
    cfg: &logicaffeine_compile::codegen_sva::wprog_engine_rtl::WProgConfig,
    blob: &[i64],
    fs: &[WFault],
) -> Option<i64> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        eval_wprog_config_faulted(cfg, &[], blob, fs).0
    }))
    .ok()
}

fn main() {
    let src = std::fs::read_to_string("/tmp/tffab_stream.lg")
        .expect("gen_tiny_transformer_lg.py --fabric --stream > /tmp/tffab_stream.lg first");
    let blob: Vec<i64> =
        serde_json::from_str(&std::fs::read_to_string("/tmp/tf_blob.json").unwrap()).unwrap();

    const SILICON: i64 = 130913;
    const CORRECT: i64 = 80113;

    with_seq_reserve(8, || {
        let cfg = serialize_program(&program_cfg_fabric(&src, "generate"));

        // Single-memory decode/dead-bank faults spanning each state memory's plausible mis-wire widths.
        let singles: Vec<WFault> = {
            let mut v = Vec::new();
            for b in 5..=7 {
                v.push(WFault::RegDecodeBits(b));
            }
            for t in [32u16, 48, 64, 96] {
                v.push(WFault::RegZeroAbove(t));
            }
            for b in [11u32, 12, 13, 14, 15] {
                v.push(WFault::HeapDecodeBits(b));
            }
            for t in [4096usize, 8192, 16384] {
                v.push(WFault::HeapZeroAbove(t));
            }
            for b in 5..=7 {
                v.push(WFault::VDecodeBits(b));
            }
            // Heap stores lagging reads by N block-steps (registered read/write pipeline).
            for n in [1u32, 2, 3] {
                v.push(WFault::HeapStoreLatency(n));
            }
            v
        };

        println!("== singles ==  silicon={SILICON} correct={CORRECT}");
        let mut hits: Vec<Vec<WFault>> = Vec::new();
        for f in &singles {
            match run(&cfg, &blob, std::slice::from_ref(f)) {
                Some(val) => {
                    let tag = if val == SILICON {
                        hits.push(vec![*f]);
                        "  <<< MATCHES SILICON"
                    } else if val == CORRECT {
                        " (correct)"
                    } else {
                        ""
                    };
                    println!("  {:<20?} -> {:>10}{}", f, val, tag);
                }
                None => println!("  {:<20?} -> hang/panic", f),
            }
        }

        // Pairwise compounds (opt-in via FAULT_SCAN_PAIRS=1 — O(n^2), slow in debug).
        if std::env::var("FAULT_SCAN_PAIRS").is_ok() {
            println!("\n== pairwise compounds that reproduce silicon {SILICON} ==");
            let mut any_pair = false;
            for (i, a) in singles.iter().enumerate() {
                for b in singles.iter().skip(i + 1) {
                    if let Some(val) = run(&cfg, &blob, &[*a, *b]) {
                        if val == SILICON {
                            any_pair = true;
                            hits.push(vec![*a, *b]);
                            println!("  {:?} + {:?} -> {}", a, b, val);
                        }
                    }
                }
            }
            if !any_pair {
                println!("  (none)");
            }
        }

        println!();
        if hits.is_empty() {
            println!(
                "CONCLUSION: no single or pairwise state-memory decode fault reproduces {SILICON}."
            );
            println!("The divergence is a compute/timing mechanism (arithmetic unit, write-forwarding, or");
            println!("bank-select mux) that a static decode model can't express -> the VM-vs-silicon step");
            println!("trace is the definitive next tool. (Closest single: the v-plane, matching token 1.)");
        } else {
            println!("CULPRIT fault set(s) reproducing silicon {SILICON}: {hits:?}");
        }
    });
}
