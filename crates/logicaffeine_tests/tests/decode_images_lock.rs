//! decode_images_lock — the decode-step board images on record are the ones the compiler builds, at clocks their
//! routes close.
//!
//! `benchmarks/silicon/decode_images.json` records each Nexys board's decode image:
//! - the part and fabric target, and the memory banks and vector lanes its fabric was printed with;
//! - the step's cycles;
//! - the core clock its divider gives, and the routed design's maximum frequency;
//! - its utilization, and its bitstream with the bitstream's hash;
//! - and, once a board has run it, the host's record of that run (`measured`, empty until one is recorded).
//!
//! This lock holds the record to the compiler and to itself:
//! - each image's vector unit is one its part has: a power of two of lanes, no wider than the part's vector unit or its
//!   banks. The widest that routes depends on the program as well as the part, so an image may print a narrower one;
//! - each image's cycles are the twin's for the decode step — lowered returning what its host reads
//!   (`Int8Model::step_reads`), so every other value only the next statement reads is forwarded — on its target at its
//!   banks and lanes, so an engine change that moves the step reddens the record until the image is rebuilt;
//! - the core clock is the board's 100 MHz oscillator through a PLLE2 setting a -1 part takes, and at most the routed
//!   maximum, so no image runs a clock its route does not close;
//! - the routed maximum says what it covers. The openXC7 nextpnr-xilinx times no path through a DSP slice, a block
//!   RAM, or a LUT-RAM's or shift register's write port, so an image it routes records its timing incomplete and names
//!   how many of each it holds. One routed with hard-cell timing (the SDF's arcs for every such cell) is complete up
//!   to a residual it bounds — a LUT-RAM's write-through, a driven-address shift register's clock-to-out — and its core
//!   clock's period covers the routed path, that residual, and the clock's own uncertainty (the PLL's jitter and the
//!   tree's skew, which the router does not model);
//! - the step's time is its cycles at that clock;
//! - each image says what its host holds a board to: the evaluator's next token after every prompt position, and the
//!   cycles the on-chip loop takes a step, the step's own and the protocol's;
//! - each image is built from the files the compiler prints today — the digest of everything synthesis reads, every
//!   simulation-only check aside, is the record's — so a printer change that moves no cycle but moves the area or the
//!   clock (a FIFO's wrap, a product's width) reddens the record until the image is rebuilt;
//! - each bitstream on record is in the tree at its recorded size and hash, where the checkout holds the images
//!   (`.gitignore` keeps them out of a clone; one image held means every one is);
//! - a board's measured run is the host's record of that bitstream running the golden trace: the trainer's tokens, the
//!   evaluator's token after every prompt position, every step in the twin's cycles, the loop in its own accounting,
//!   and its time per token the loop's cycles at the image's clock — on the board, not replayed;
//! - both boards are recorded.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test decode_images_lock`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::lower::fabric::fabric_cycle_model;
use logicaffeine_compile::spatial::lower::fabric_board::{generate_cycles, image_digest, program_board_image, CoreClock, ImageRole, NexysBoard};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial_returning;
use logicaffeine_compile::spatial::target::SpatialTarget;

mod common;
#[path = "common/spatial.rs"]
mod spatial_support;

fn silicon(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon").join(name)
}

fn ledger() -> serde_json::Value {
    let text = std::fs::read_to_string(silicon("decode_images.json")).expect("benchmarks/silicon/decode_images.json");
    serde_json::from_str(&text).expect("the ledger is JSON")
}

fn images() -> Vec<serde_json::Value> {
    ledger()["images"].as_array().expect("an `images` array").clone()
}

/// The fabric target a decode image names, as the part's preset.
fn preset_of(name: &str) -> SpatialTarget {
    match name {
        "xc7a100t-fabric" => SpatialTarget::xc7a100t_fabric(16, 64),
        "xc7a200t-fabric" => SpatialTarget::xc7a200t_fabric(16, 64),
        other => panic!("a decode image on an unknown fabric target {other}"),
    }
}

/// The target image `i` was printed for: its part's preset at the banks and vector lanes it records.
fn target_of(i: &serde_json::Value) -> SpatialTarget {
    let mut t = preset_of(i["target"].as_str().expect("an image's `target`"));
    t.site.mem_banks = num(i, "mem_banks") as usize;
    t.site.vector_lanes = num(i, "vector_lanes") as usize;
    t
}

/// The decode step a board image carries: the model's step at the golden trace's first prompt token, position 0.
fn decode_step(target: &SpatialTarget) -> SpatialProgram {
    let path = silicon("tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let m = Int8Model::from_json(&j).expect("the model");
    let src = m.decode_step_lg(j["golden_trace"]["prompt_tokens"][0].as_u64().unwrap() as usize, 0, &[], &[]);
    compile_program_to_spatial_returning(&src, target, &Int8Model::step_reads()).unwrap().program.expect("the step lowers whole")
}

fn twin_cycles(target: &SpatialTarget) -> u64 {
    fabric_cycle_model(&decode_step(target), target)
}

/// Every file a build of image `i` reads, as the compiler prints it today: the board image of the decode step at the
/// image's banks, lanes, top and divider, its memory images from the step's fills, and the hand-written sources its board
/// instantiates.
fn image_files(i: &serde_json::Value) -> Vec<(String, String)> {
    let target = target_of(i);
    let board = match i["part"].as_str().unwrap() {
        "xc7a100tcsg324-1" => NexysBoard::A7,
        "xc7a200tsbg484-1" => NexysBoard::Video,
        other => panic!("a decode image on an unknown part {other}"),
    };
    let top = i["top"].as_str().expect("an image's `top`");
    program_board_image(&decode_step(&target), &target, board, ImageRole::DecodeStep, top, clock_of(i))
        .unwrap_or_else(|e| panic!("{top}: the step's board image: {e:?}"))
}

fn num(v: &serde_json::Value, field: &str) -> f64 {
    v[field].as_f64().unwrap_or_else(|| panic!("an image's `{field}`"))
}

#[test]
fn both_boards_are_recorded() {
    let parts: Vec<String> = images().iter().map(|i| i["part"].as_str().unwrap().to_string()).collect();
    for want in ["xc7a100tcsg324-1", "xc7a200tsbg484-1"] {
        assert!(parts.iter().any(|p| p == want), "the ledger records an image for {want}: {parts:?}");
    }
}

/// An image's vector unit is one its part has: a power of two of lanes, no wider than the part's own vector unit and
/// no wider than the banks a run of its lanes reads.
#[test]
fn each_image_s_vector_unit_is_one_its_part_has() {
    for i in images() {
        let preset = preset_of(i["target"].as_str().unwrap());
        let lanes = i["vector_lanes"].as_u64().unwrap_or_else(|| panic!("{}: an image's `vector_lanes`", i["part"])) as usize;
        assert!(lanes.is_power_of_two(), "{}: {lanes} lanes is a power of two", i["part"]);
        assert!(lanes <= preset.site.vector_lanes, "{}: {lanes} lanes within the part's {}", i["part"], preset.site.vector_lanes);
        assert!(lanes <= num(&i, "mem_banks") as usize, "{}: {lanes} lanes within its banks", i["part"]);
    }
}

#[test]
fn each_image_takes_the_twin_s_cycles() {
    for i in images() {
        let target = target_of(&i);
        assert_eq!(
            num(&i, "cycles_per_step") as u64,
            twin_cycles(&target),
            "{}: the image's step is the twin's cycles for the decode step the compiler prints today",
            i["part"]
        );
    }
}

fn clock_of(i: &serde_json::Value) -> CoreClock {
    let pll = &i["pll"];
    let field = |k: &str| pll[k].as_u64().unwrap_or_else(|| panic!("{}: the PLL's `{k}`", i["part"])) as u32;
    CoreClock { mult: field("mult"), div: field("div"), out: field("out") }
}

#[test]
fn each_image_runs_a_clock_its_route_closes() {
    for i in images() {
        let clock = clock_of(&i);
        assert!(clock.legal(), "{}: {clock:?} is a PLLE2 setting a -1 part takes", i["part"]);
        let core = num(&i, "core_mhz");
        assert!((core - clock.mhz()).abs() < 1e-9, "{}: the core clock is the PLL's {} MHz", i["part"], clock.mhz());
        assert!(core <= num(&i, "fmax_mhz"), "{}: {core} MHz runs within the routed {} MHz", i["part"], num(&i, "fmax_mhz"));
        if let Some(residual) = i["timing"]["residual_ns"].as_f64() {
            let path = 1000.0 / num(&i, "fmax_mhz");
            assert_eq!(i["timing"]["clock_uncertainty_ns"].as_f64(), Some(CoreClock::UNCERTAINTY_NS), "{}: the clock's allowance on record", i["part"]);
            assert!(
                1000.0 / core >= path + residual + CoreClock::UNCERTAINTY_NS,
                "{}: a {:.3} ns period covers the routed {path:.3} ns path, the {residual} ns residual and the clock's {} ns",
                i["part"],
                1000.0 / core,
                CoreClock::UNCERTAINTY_NS
            );
        }
        let step_us = num(&i, "cycles_per_step") / core;
        assert!((num(&i, "step_us") - step_us).abs() < 1e-3, "{}: a step is its cycles at its clock ({step_us} µs)", i["part"]);
    }
}

/// The host holds every reply to the image record, so the record says what each reply is: after every prompt position,
/// the evaluator's next token; for each step of the on-chip loop, the step's cycles and the protocol's own.
#[test]
fn each_image_says_what_its_host_holds_the_board_to() {
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(silicon("tiny_transformer.json")).unwrap()).unwrap();
    let m = Int8Model::from_json(&j).expect("the model");
    let tokens = |k: &str| -> Vec<usize> { j["golden_trace"][k].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as usize).collect() };
    let (prompt, golden) = (tokens("prompt_tokens"), tokens("generated_tokens"));
    for i in images() {
        let target = target_of(&i);
        let lat = target.site.local_mem_read_latency.expect("a fabric memory's read latency") as u64;
        assert_eq!(
            num(&i, "loop_cycles_per_step") as u64,
            generate_cycles(&[twin_cycles(&target)], lat),
            "{}: a step of the on-chip loop is the twin's step and the protocol's own cycles",
            i["part"]
        );
        let recorded: Vec<usize> = i["prompt_next_tokens"]
            .as_array()
            .unwrap_or_else(|| panic!("{}: the image's `prompt_next_tokens`", i["part"]))
            .iter()
            .map(|t| t.as_u64().unwrap() as usize)
            .collect();
        let want = spatial_support::prompt_next_tokens(&m, &prompt, &target);
        assert_eq!(recorded, want, "{}: the prompt steps reply the evaluator's next tokens", i["part"]);
        assert_eq!(recorded.last(), golden.first(), "{}: the prompt's last step predicts the trainer's first token", i["part"]);
    }
}

#[test]
fn each_image_is_built_from_the_sources_the_compiler_prints_today() {
    for i in images() {
        assert_eq!(
            image_digest(&image_files(&i)),
            i["sources_sha256"].as_str().expect("an image's `sources_sha256`"),
            "{}: the image on record is built from what the compiler prints today",
            i["part"]
        );
    }
}

/// The PLL settings are the ones a -1 Artix-7 takes, and the fastest clock within a period is the fastest of them: no
/// legal setting is faster and still within it.
#[test]
fn the_core_clock_is_the_fastest_legal_pll_setting_within_its_period() {
    assert!(CoreClock { mult: 10, div: 1, out: 40 }.legal(), "a 1 GHz VCO over 40: 25 MHz");
    assert!(!CoreClock { mult: 7, div: 1, out: 1 }.legal(), "a 700 MHz VCO is below the -1 part's 800");
    assert!(!CoreClock { mult: 17, div: 1, out: 1 }.legal(), "a 1700 MHz VCO is above its 1600");
    assert!(!CoreClock { mult: 64, div: 6, out: 1 }.legal(), "a 16.7 MHz phase detector is below its 19");
    assert!(!CoreClock { mult: 10, div: 1, out: 129 }.legal(), "an output divider past 128");
    for period in [10.0, 20.0, 25.0, 35.9, 36.0, 37.47, 40.0, 79.3] {
        let c = CoreClock::fastest_within(period).expect("a clock within the period");
        assert!(c.legal() && 1000.0 / c.mhz() >= period, "{c:?} within {period} ns");
        for div in 1..=56 {
            for mult in 2..=64 {
                for out in 1..=128 {
                    let o = CoreClock { mult, div, out };
                    assert!(!(o.legal() && 1000.0 / o.mhz() >= period && o.mhz() > c.mhz() + 1e-9), "{o:?} is faster than {c:?} within {period} ns");
                }
            }
        }
    }
    assert_eq!(CoreClock::fastest_within(10.0).map(CoreClock::mhz), Some(100.0));
}

#[test]
fn the_image_digest_moves_with_what_synthesis_reads_and_only_with_it() {
    let files = vec![
        ("a.v".to_string(), "module a; wire x;\n// synthesis translate_off\ninitial $display(\"sim\");\n// synthesis translate_on\nendmodule\n".to_string()),
        ("b.hex".to_string(), "01\n02\n".to_string()),
    ];
    let base = image_digest(&files);
    let mut sim_only = files.clone();
    sim_only[0].1 = sim_only[0].1.replace("$display(\"sim\")", "$display(\"another check\")");
    assert_eq!(image_digest(&sim_only), base, "a simulation-only check moves nothing");
    let reordered = vec![files[1].clone(), files[0].clone()];
    assert_eq!(image_digest(&reordered), base, "the files' order moves nothing");
    let mut logic = files.clone();
    logic[0].1 = logic[0].1.replace("wire x;", "wire y;");
    assert_ne!(image_digest(&logic), base, "a line synthesis reads moves the digest");
    let mut memory = files.clone();
    memory[1].1 = "01\n03\n".to_string();
    assert_ne!(image_digest(&memory), base, "a memory image moves the digest");
    let mut renamed = files.clone();
    renamed[1].0 = "c.hex".to_string();
    assert_ne!(image_digest(&renamed), base, "a file's name moves the digest");
}

#[test]
fn each_image_says_which_of_its_paths_its_clock_was_timed_over() {
    let untimed_classes = ["DSP48E1", "RAMB18E1", "RAMB36E1", "RAM32M", "RAM64M", "RAM32X1D", "RAM64X1D", "RAM128X1D", "RAM256X1S", "SRL16E", "SRLC32E"];
    for i in images() {
        let timing = &i["timing"];
        let untimed = timing["untimed_cells"].as_object().unwrap_or_else(|| panic!("{}: the cells its route did not time", i["part"]));
        for (class, n) in untimed {
            assert!(untimed_classes.contains(&class.as_str()), "{}: {class} is a class nextpnr-xilinx does not time", i["part"]);
            assert!(n.as_u64().is_some_and(|n| n > 0), "{}: {class} is listed with its count", i["part"]);
        }
        let residual = timing["residual_ns"].as_f64();
        assert_eq!(
            timing["complete"].as_bool(),
            Some(residual.is_some()),
            "{}: its timing is complete exactly when hard-cell timing bounded its residual",
            i["part"]
        );
        if let Some(r) = residual {
            assert!(untimed.is_empty(), "{}: hard-cell timing leaves no class untimed", i["part"]);
            assert!((0.0..5.0).contains(&r), "{}: the residual is a cell's own delay, not {r} ns", i["part"]);
        }
        let note = timing["note"].as_str().unwrap_or_default();
        assert!(note.contains("not in it") || note.contains("residual_ns"), "{}: the record says what its maximum frequency leaves out", i["part"]);
    }
}

#[test]
fn each_bitstream_on_record_is_in_the_tree() {
    if !common::require_tool("decode-images") {
        return;
    }
    for i in images() {
        let name = i["bitstream"].as_str().unwrap();
        let bytes = std::fs::read(silicon(name)).unwrap_or_else(|_| panic!("benchmarks/silicon/{name}"));
        assert_eq!(bytes.len() as u64, num(&i, "bytes") as u64, "{name}: its recorded size");
        let hash: String = logicaffeine_compile::crypto_accel::sha256(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hash, i["sha256"].as_str().unwrap(), "{name}: its recorded hash");
    }
}

/// The `decode-images` probe answers whether a checkout holds any bitstream a decode ledger names, so a clone skips the
/// bitstream checks and a checkout that lost one image of several fails them; and it reads the bitstreams these locks
/// check, no more and no fewer.
#[test]
fn the_decode_images_probe_answers_whether_a_checkout_holds_an_image() {
    let root = std::env::temp_dir().join(format!("decode_images_probe_{}", std::process::id()));
    let silicon = root.join("benchmarks/silicon");
    std::fs::create_dir_all(&silicon).unwrap();
    assert!(!common::decode_images_at(&root), "no ledger: no image");
    std::fs::write(silicon.join("decode_images.json"), r#"{"images": [{"bitstream": "a.bit"}, {"bitstream": "a2.bit"}]}"#)
        .unwrap();
    std::fs::write(silicon.join("decode_split_images.json"), r#"{"chips": [{"bitstream": "b.bit"}]}"#).unwrap();
    assert!(!common::decode_images_at(&root), "ledgers whose images the checkout lacks");
    std::fs::write(silicon.join("b.bit"), b"").unwrap();
    assert!(common::decode_images_at(&root), "a split chip's image held");
    std::fs::remove_file(silicon.join("b.bit")).unwrap();
    std::fs::write(silicon.join("a.bit"), b"").unwrap();
    assert!(common::decode_images_at(&root), "one board's image of two held");
    std::fs::write(silicon.join("c.bit"), b"").unwrap();
    std::fs::remove_file(silicon.join("a.bit")).unwrap();
    assert!(!common::decode_images_at(&root), "an image no ledger names is not one");
    let _ = std::fs::remove_dir_all(&root);
    let named: Vec<String> = images().iter().map(|i| i["bitstream"].as_str().unwrap().to_string()).collect();
    assert_eq!(common::decode_ledger_bitstreams(&ledger()), named, "the probe reads the images' bitstreams");
}

/// Every script a board image's record names — the gate its FASM passed, the flow that built it — is one the repository
/// holds, so the record's claim can be run again by whoever reads it.
#[test]
fn every_script_an_image_record_names_is_in_the_repository() {
    fn strings(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::String(s) => out.push(s.clone()),
            serde_json::Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
            serde_json::Value::Object(m) => m.values().for_each(|x| strings(x, out)),
            _ => {}
        }
    }
    let mut named = 0;
    for ledger in common::DECODE_IMAGE_LEDGERS {
        let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(silicon(ledger)).unwrap()).unwrap();
        let mut texts = Vec::new();
        strings(&j, &mut texts);
        for text in texts {
            for word in text.split(|c: char| c.is_whitespace() || c == ':' || c == ',' || c == '(' || c == ')') {
                if word.starts_with("scripts/") && (word.ends_with(".py") || word.ends_with(".sh")) {
                    assert!(silicon("../..").join(word).is_file(), "{ledger} names {word}, which the repository does not hold");
                    named += 1;
                }
            }
        }
    }
    assert!(named >= 4, "each image's record names the gate its FASM passed ({named} named)");
}

/// A board's measured run, once recorded, is the host's record of the image's own bitstream running the golden trace:
/// every token, every step's cycles and the loop's are what the image predicts, so a receipt can say nothing the image
/// does not.
#[test]
fn each_measured_run_is_the_board_running_its_own_image() {
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(silicon("tiny_transformer.json")).unwrap()).unwrap();
    let tokens = |k: &str| -> Vec<u64> { j["golden_trace"][k].as_array().unwrap().iter().map(|t| t.as_u64().unwrap()).collect() };
    let golden = tokens("generated_tokens");
    let n = golden.len() as u64 - 1;
    for i in images() {
        let m = &i["measured"];
        let Some(fields) = m.as_object() else { panic!("{}: `measured` is an object", i["part"]) };
        if fields.is_empty() {
            continue;
        }
        let part = i["part"].as_str().unwrap();
        let list = |k: &str| -> Vec<u64> {
            m[k].as_array().unwrap_or_else(|| panic!("{part}: the measured `{k}`")).iter().map(|t| t.as_u64().unwrap()).collect()
        };
        assert_eq!(m["part"].as_str(), Some(part), "{part}: measured on its own part");
        assert_eq!(m["bitstream_sha256"], i["sha256"], "{part}: measured running the bitstream on record");
        assert_eq!(m["replayed"].as_bool(), Some(false), "{part}: measured on the board, not replayed");
        assert_eq!(list("tokens"), golden, "{part}: the board's tokens are the trainer's");
        assert_eq!(m["prompt_next_tokens"], i["prompt_next_tokens"], "{part}: the board's prompt steps reply the evaluator's tokens");
        assert!(list("step_cycles").iter().all(|c| *c == num(&i, "cycles_per_step") as u64), "{part}: every step in the twin's cycles");
        assert_eq!(m["generate_cycles"].as_u64(), Some(n * num(&i, "loop_cycles_per_step") as u64), "{part}: the loop in its own accounting");
        let us = n as f64 * num(&i, "loop_cycles_per_step") / n as f64 / num(&i, "core_mhz");
        assert!((num(m, "us_per_token") - us).abs() < 1e-2, "{part}: a token's time is the loop's cycles at the image's clock ({us} µs)");
        assert!(m["date"].as_str().is_some_and(|d| d.len() >= 10), "{part}: the run's date");
    }
}

