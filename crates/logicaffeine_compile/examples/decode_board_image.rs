//! Writes the int8 tiny transformer's decode step as a board image for one Nexys board into OUT_DIR: every file synthesis
//! reads (`fabric_board::program_board_image`), and `image.json`, the record `scripts/xc7/decode_ledger.py` builds the
//! image's row of `benchmarks/silicon/decode_images.json` from — the part, the target, the step's cycles and the on-chip
//! loop's, the next token each prompt step predicts, the core clock and the image's digest. The core clock is a PLLE2
//! setting (`MULT,DIV,OUT`), or the fastest a route closes under the clock contract: `--fmax MHZ --residual NS`, its
//! routed Fmax and the residual the router bounds (`CoreClock::for_route`). `--lanes N` sets the vector unit's lanes
//! where the part's preset is not wanted. `scripts/xc7/sweep-image.sh` builds the image into a bitstream.
//!
//! Run: `cargo run -p logicaffeine-compile --example decode_board_image -- a7|video OUT_DIR (MULT,DIV,OUT | --fmax MHZ --residual NS) [--lanes N]`

use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::lower::fabric::fabric_cycle_model;
use logicaffeine_compile::spatial::lower::fabric_board::{
    generate_cycles, image_digest, program_board_image, CoreClock, DecodeWords, ImageRole, NexysBoard,
};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial_returning;
use logicaffeine_compile::spatial::target::SpatialTarget;
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: decode_board_image a7|video OUT_DIR (MULT,DIV,OUT | --fmax MHZ --residual NS) [--lanes N]";

fn usage() -> ! {
    eprintln!("{USAGE}");
    std::process::exit(2)
}

/// The core clock the arguments name: a PLLE2 setting, or the fastest one a route closes.
fn clock_of(args: &[String]) -> CoreClock {
    let value = |flag: &str| -> Option<f64> { args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)?.parse().ok()) };
    let clock = match (value("--fmax"), value("--residual")) {
        (Some(fmax), Some(residual)) => CoreClock::for_route(fmax, residual).unwrap_or_else(|| usage()),
        (None, None) => {
            let pll: Vec<u32> = args.first().map(|a| a.split(',').filter_map(|v| v.parse().ok()).collect()).unwrap_or_default();
            let [mult, div, out] = pll[..] else { usage() };
            CoreClock { mult, div, out }
        }
        _ => usage(),
    };
    if !clock.legal() {
        eprintln!("{clock:?} is no PLLE2 setting a -1 part takes");
        std::process::exit(2)
    }
    clock
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        usage()
    }
    let (board, mut target, top) = match args[0].as_str() {
        "a7" => (NexysBoard::A7, SpatialTarget::xc7a100t_fabric(16, 64), "logos_decode_a7"),
        "video" => (NexysBoard::Video, SpatialTarget::xc7a200t_fabric(16, 64), "logos_decode_video"),
        _ => usage(),
    };
    let out = PathBuf::from(&args[1]);
    let clock = clock_of(&args[2..]);
    if let Some(i) = args.iter().position(|a| a == "--lanes") {
        target.site.vector_lanes = args.get(i + 1).and_then(|n| n.parse().ok()).unwrap_or_else(|| usage());
    }
    let model = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(model).expect("the model")).expect("JSON");
    let m = Int8Model::from_json(&j).expect("the model");
    let prompt: Vec<usize> = j["golden_trace"]["prompt_tokens"].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as usize).collect();
    let p = compile_program_to_spatial_returning(&m.decode_step_lg(prompt[0], 0, &[], &[]), &target, &Int8Model::step_reads())
        .expect("a tensor program")
        .program
        .expect("the step lowers whole");
    let files = program_board_image(&p, &target, board, ImageRole::DecodeStep, top, clock).unwrap_or_else(|e| panic!("{e:?}"));
    std::fs::create_dir_all(&out).expect("OUT_DIR");
    for (name, body) in &files {
        std::fs::write(out.join(name), body).expect("an image file");
    }
    let lat = target.site.local_mem_read_latency.expect("a fabric memory's read latency") as u64;
    let cycles = fabric_cycle_model(&p, &target);
    let words = DecodeWords::of(&p, "tok", "pos", "next").expect("the step's words");
    let record = serde_json::json!({
        "top": top,
        "part": board.part(),
        "target": target.name,
        "read_latency": lat,
        "mem_banks": target.site.mem_banks,
        "vector_lanes": target.site.vector_lanes,
        "pll": { "mult": clock.mult, "div": clock.div, "out": clock.out },
        "core_mhz": clock.mhz(),
        "cycles_per_step": cycles,
        "loop_cycles_per_step": generate_cycles(&[cycles], lat),
        "prompt_next_tokens": m.prompt_next_tokens(&prompt, &target).expect("the prompt steps"),
        "sources_sha256": image_digest(&files),
        "words": { "tok": words.tok, "pos": words.pos, "next": words.next },
    });
    std::fs::write(out.join("image.json"), serde_json::to_string_pretty(&record).unwrap()).expect("image.json");
    println!("{top}: part {} at {:.3} MHz, {cycles} cycles a step, {}", board.part(), clock.mhz(), out.display());
}
