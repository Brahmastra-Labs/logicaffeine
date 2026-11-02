//! Writes the int8 tiny transformer's decode step split over the two Nexys boards — chip 0 on the Nexys Video, chip 1 on
//! the Nexys A7, the step tensor-parallel over its two heads (`Int8Model::decode_step_tp_lg`) — as two board images and
//! the relay the host runs between them. Each OUT gets its chip's board image (`fabric_board::program_board_image`) and
//! `image.json`; OUT_A also gets `split.json`: the tensors that cross, each chip's halts with the words they send and
//! receive, and the relay schedule the compiler derives from both chips' halts. `scripts/xc7/split_ledger.py` builds
//! `benchmarks/silicon/decode_split_images.json` from the routed images and `split.json`. Each core clock is a PLLE2
//! setting (`MULT,DIV,OUT`); `--lanes N` sets chip 0's vector lanes where the Video preset's are not wanted.
//!
//! Run: `cargo run -p logicaffeine-compile --example decode_split_images -- OUT_A OUT_B MULT,DIV,OUT(A) MULT,DIV,OUT(B) [--lanes N]`

use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::lower::fabric_board::{
    chip_record, image_digest, program_board_image, relay_record, CoreClock, ImageRole, NexysBoard,
};
use logicaffeine_compile::spatial::pipeline::compile_program_to_chips_returning;
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::spatial::tensor_parallel::relay_schedule;
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: decode_split_images OUT_A OUT_B MULT,DIV,OUT(A) MULT,DIV,OUT(B) [--lanes N]";

fn usage() -> ! {
    eprintln!("{USAGE}");
    std::process::exit(2)
}

fn clock_of(arg: &str) -> CoreClock {
    let pll: Vec<u32> = arg.split(',').filter_map(|v| v.parse().ok()).collect();
    let [mult, div, out] = pll[..] else { usage() };
    let clock = CoreClock { mult, div, out };
    if !clock.legal() {
        eprintln!("{clock:?} is no PLLE2 setting a -1 part takes");
        std::process::exit(2)
    }
    clock
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        usage()
    }
    let (out_a, out_b) = (PathBuf::from(&args[0]), PathBuf::from(&args[1]));
    let (clock_a, clock_b) = (clock_of(&args[2]), clock_of(&args[3]));
    let mut target_a = SpatialTarget::xc7a200t_fabric(16, 64);
    if let Some(i) = args.iter().position(|a| a == "--lanes") {
        target_a.site.vector_lanes = args.get(i + 1).and_then(|n| n.parse().ok()).unwrap_or_else(|| usage());
    }
    let model = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(model).expect("the model")).expect("JSON");
    let m = Int8Model::from_json(&j).expect("the model");
    let prompt: Vec<usize> = j["golden_trace"]["prompt_tokens"].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as usize).collect();
    let (src, manifest) = m.decode_step_tp_lg(prompt[0], 0, &[], &[], 2).expect("the step splits over two chips");
    let boards = [
        (NexysBoard::Video, target_a.linked(), clock_a, out_a, "logos_chip0_video"),
        (NexysBoard::A7, SpatialTarget::xc7a100t_fabric(16, 64).linked(), clock_b, out_b, "logos_chip1_a7"),
    ];
    let mut chips: Vec<SpatialProgram> = Vec::new();
    let mut transfers = Vec::new();
    for (c, (_, target, _, _, _)) in boards.iter().enumerate() {
        let lowered = compile_program_to_chips_returning(&src, target, &manifest, &Int8Model::step_reads()).expect("the split step lowers");
        chips.push(lowered.chips[c].whole().unwrap_or_else(|| panic!("chip {c} on {}: {}", target.name, lowered.chips[c].refusal())).clone());
        transfers = lowered.transfers;
    }
    let relay = relay_schedule(&chips, &transfers).expect("the chips' halts relay");
    // Chip 0 holds the head: its next token at each prompt position is the whole step's on chip 0's fabric.
    let prompt_next = m.prompt_next_tokens(&prompt, &boards[0].1).expect("the prompt steps");
    let mut records = Vec::new();
    for (c, ((board, target, clock, out, top), p)) in boards.iter().zip(&chips).enumerate() {
        let files = program_board_image(p, target, *board, ImageRole::Chip, top, *clock).unwrap_or_else(|e| panic!("chip {c}: {e:?}"));
        std::fs::create_dir_all(out).expect("OUT");
        for (name, body) in &files {
            std::fs::write(out.join(name), body).expect("an image file");
        }
        let chip = chip_record(p, target).expect("the chip's record");
        let record = serde_json::json!({
            "top": top,
            "part": board.part(),
            "target": target.name,
            "chip": c,
            "read_latency": target.site.local_mem_read_latency.expect("a fabric memory's read latency"),
            "mem_banks": target.site.mem_banks,
            "vector_lanes": target.site.vector_lanes,
            "pll": { "mult": clock.mult, "div": clock.div, "out": clock.out },
            "core_mhz": clock.mhz(),
            "segments": chip["segments"],
            "links": chip["links"],
            "inputs": chip["inputs"],
            "next": chip["next"],
            "prompt_next_tokens": if c == 0 { serde_json::json!(prompt_next) } else { serde_json::Value::Null },
            "sources_sha256": image_digest(&files),
        });
        std::fs::write(out.join("image.json"), serde_json::to_string_pretty(&record).unwrap()).expect("image.json");
        println!("{top}: part {} at {:.3} MHz, segments {}, {}", board.part(), clock.mhz(), chip["segments"], out.display());
        records.push(record);
    }
    let split = serde_json::json!({
        "transfers": transfers.iter().map(|t| serde_json::json!({ "tensor": t.tensor, "from": t.from, "to": t.to })).collect::<Vec<_>>(),
        "relay": relay_record(&relay),
        "chips": records,
    });
    std::fs::write(boards[0].3.join("split.json"), serde_json::to_string_pretty(&split).unwrap()).expect("split.json");
    println!("relay: {} steps", relay.len());
}
