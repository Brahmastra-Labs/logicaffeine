//! emit_qor_dram — the K7 traffic gate's two halves: stage one controller's run of the word stream, and judge the
//! log the run writes (codegen_sva::ddr2_traffic, its bus held to codegen_sva::ddr2_jedec).
//!
//!     cargo run -p logicaffeine-compile --example emit_qor_dram -- stage <out-dir> <candidate> <sequential|scattered|probe> <words>
//!     cargo run -p logicaffeine-compile --example emit_qor_dram -- judge <run-dir>
//!     cargo run -p logicaffeine-compile --example emit_qor_dram -- --list
//!
//! `stage` writes the testbench `tb_qor_dram.v`, the stream's `words.hex` and `dram.json` (the candidate, the stream,
//! the operating point, the request interface, and `row_bits`, the row-address bits the part's model must store).
//! For `k7_logos_ddr2_ctrl` it also writes the controller, `ddr2_ctrl.v`, and the part, `ddr2_device.v`; for
//! `k7_logos_dfi_ctrl` the controller, `ddr2_dfi_ctrl.v`, which runs against LiteDRAM's SDRAMPHYModel as LiteDRAM's
//! controller does. LiteDRAM's controller and its SDRAMPHYModel come from
//! `benchmarks/silicon/qor/kernels/K7/litedram_controller.py` (the model with `--phy-model --row-bits <row_bits>`),
//! each elaborated by yosys `proc` before simulation. `judge` reads `dram.json` and `dram.log` and prints the judgement as JSON, or the refusal on stderr
//! with exit 1. `--list` prints the controllers the gate drives, one per line.

use logicaffeine_compile::codegen_sva::ddr2_traffic::{
    emit_ddr2_ctrl_traffic_testbench, emit_ddr2_device_verilog, emit_ddr2_dfi_ctrl_traffic_testbench,
    emit_litedram_traffic_testbench, judge, k7_controller, k7_ddr2_ctrl_verilog, k7_dfi_ctrl_verilog, k7_timing, read_log,
    Pattern, Stats, Traffic, K7_CONTROLLERS, K7_GRADE, K7_TCK_PS,
};
use serde_json::{json, Value};
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["--list"] => {
            for c in K7_CONTROLLERS {
                println!("{}", c.name);
            }
        }
        ["stage", out, candidate, pattern, words] => stage(Path::new(out), candidate, pattern, words),
        ["judge", dir] => run_judge(Path::new(dir)),
        _ => fail("usage: emit_qor_dram stage <out-dir> <candidate> <sequential|scattered|probe> <words> | judge <run-dir> | --list"),
    }
}

fn stage(out: &Path, candidate: &str, pattern: &str, words: &str) {
    let c = k7_controller(candidate).unwrap_or_else(|| fail(&format!("{candidate}: not a controller the K7 traffic gate drives")));
    let pattern = Pattern::parse(pattern).unwrap_or_else(|| fail(&format!("{pattern}: the stream is sequential, scattered or probe")));
    let words: u32 = words.parse().ok().filter(|&n| n > 0).unwrap_or_else(|| fail(&format!("{words}: a positive word count")));
    let traffic = Traffic { pattern, words };
    let t = k7_timing(c.bl);
    std::fs::create_dir_all(out).unwrap_or_else(|e| fail(&format!("{}: {e}", out.display())));
    let mut files: Vec<(&str, String)> = vec![("words.hex", traffic.rom())];
    match c.name {
        "k7_logos_ddr2_ctrl" => {
            files.push(("ddr2_ctrl.v", k7_ddr2_ctrl_verilog()));
            files.push(("ddr2_device.v", emit_ddr2_device_verilog(&t)));
            files.push(("tb_qor_dram.v", emit_ddr2_ctrl_traffic_testbench(&t, &traffic)));
        }
        "k7_logos_dfi_ctrl" => {
            files.push(("ddr2_dfi_ctrl.v", k7_dfi_ctrl_verilog()));
            files.push(("tb_qor_dram.v", emit_ddr2_dfi_ctrl_traffic_testbench(&t, &traffic)));
        }
        "k7_litedram_ctrl" => files.push(("tb_qor_dram.v", emit_litedram_traffic_testbench(&t, &traffic))),
        other => fail(&format!("{other}: no testbench")),
    }
    for (name, text) in &files {
        std::fs::write(out.join(name), text).unwrap_or_else(|e| fail(&format!("{name}: {e}")));
    }
    let record = json!({
        "candidate": c.name,
        "pattern": pattern.label(),
        "words": words,
        "part": K7_GRADE.part,
        "tck_ps": K7_TCK_PS,
        "bl": c.bl,
        "cl": t.cl,
        "parts": c.iface.parts,
        "cols_per_part": c.iface.cols_per_part,
        "row_bits": traffic.row_bits(),
        "every": traffic.every(),
        "files": files.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
    });
    write_json(&out.join("dram.json"), &record);
    println!("{}", out.join("dram.json").display());
}

fn stats(s: &Stats) -> Value {
    json!({"n": s.n, "min": s.min, "max": s.max, "p50": s.p50, "p99": s.p99, "mean": s.mean})
}

fn run_judge(dir: &Path) {
    let text = std::fs::read_to_string(dir.join("dram.json")).unwrap_or_else(|e| fail(&format!("dram.json: {e}")));
    let rec: Value = serde_json::from_str(&text).unwrap_or_else(|e| fail(&format!("dram.json: {e}")));
    let name = rec["candidate"].as_str().unwrap_or_else(|| fail("dram.json names no candidate"));
    let c = k7_controller(name).unwrap_or_else(|| fail(&format!("{name}: not a controller the K7 traffic gate drives")));
    let pattern = rec["pattern"].as_str().and_then(Pattern::parse).unwrap_or_else(|| fail("dram.json names no stream"));
    let words = rec["words"].as_u64().and_then(|w| u32::try_from(w).ok()).unwrap_or_else(|| fail("dram.json counts no words"));
    let traffic = Traffic { pattern, words };
    let t = k7_timing(c.bl);
    let log_text = std::fs::read_to_string(dir.join("dram.log")).unwrap_or_else(|e| fail(&format!("dram.log: {e}")));
    let log = read_log(&log_text).unwrap_or_else(|e| fail(&format!("dram.log: {e}")));
    let j = judge(&t, &traffic, c.iface, &log).unwrap_or_else(|e| fail(&e));
    let bus = &j.bus;
    let out = json!({
        "candidate": name,
        "pattern": pattern.label(),
        "words": j.words,
        "requests": j.requests,
        "stream_cycles": j.stream_cycles,
        "payload_cycles": j.payload_cycles,
        "bus_span": j.bus_span,
        "data_cycles": bus.data_cycles,
        "row_hits": j.row_hits,
        "commands": {"activate": bus.activates, "read": bus.reads, "write": bus.writes, "precharge": bus.precharges,
                     "refresh": bus.refreshes},
        "write_latency": stats(&j.write_latency),
        "read_latency": stats(&j.read_latency),
        "read_service": stats(&j.read_service),
        "read_bus": stats(&j.read_bus),
    });
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
}

fn write_json(path: &Path, v: &Value) {
    std::fs::write(path, serde_json::to_string_pretty(v).unwrap() + "\n").unwrap_or_else(|e| fail(&format!("{}: {e}", path.display())));
}

fn fail(msg: &str) -> ! {
    eprintln!("emit_qor_dram: {msg}");
    std::process::exit(1);
}
