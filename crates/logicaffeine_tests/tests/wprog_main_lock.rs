//! wprog_main_lock — RUN LOGOS on the fabric: a whole `## Main` program, `Show` and all, on the whole-program
//! engine, its output ≡ the VM's.
//!
//! Until now the engine ran one FUNCTION and answered one Int. A LOGOS program is `## Main` with `Show`
//! statements (and functions it calls). So the lowering gains a Main entry (`program_main_fabric`: the code
//! region before the first function, ending in `Halt`) and the engine gains a SHOW cell (kind 10, `code` = the
//! value's type tag) whose model appends to an output tape and whose RTL streams a tagged record over the byte
//! channel the moment it executes — before the final result frame. The host prints those records as the
//! program's output and checks them against the VM's, line for line.
//!
//! Three levels, each ≡ the level before and the first ≡ the language:
//!   model    program_main_fabric + eval_wprog_config_run → rendered tape == vm_outcome(src).output
//!   RTL      the SoC engine under iverilog streams the records; the testbench decodes them == the model tape
//!   silicon  scripts/logos-chip.py run wpe200 prog.lg (board-gated, in logos_chip_lock)

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::chip_protocol::{TAG_BOOL, TAG_FLOAT, TAG_INT, TAG_TEXT};
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    emit_wprog_engine_soc_testbench, emit_wprog_engine_soc_verilog,
    emit_wprog_engine_soc_verilog_hw, eval_wprog_config_run, program_main_fabric, program_output,
    render_show, wprog_config_fits_hw_engine, wprog_config_image, WProgConfig, WShow,
};
use logicaffeine_compile::compile::vm_outcome;

const CLKS_PER_BIT: usize = 4;

/// The VM's output for a whole program, one line per `Show`.
fn vm_lines(src: &str) -> Vec<String> {
    let out = vm_outcome(src);
    assert!(out.error.is_none(), "VM error: {:?}", out.error);
    out.output.trim_end().lines().map(str::to_string).collect()
}

/// (name, LOGOS program) — every one a real `## Main` program a person would write.
const CORPUS: &[(&str, &str)] = &[
    ("int", "## Main\n    Show 6 * 7.\n"),
    (
        "loop",
        "## Main\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is less than or equal to 10:\n        Set acc to acc + i.\n        Set i to i + 1.\n    Show acc.\n",
    ),
    (
        "recursion",
        "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n## Main\n    Show fib(15).\n    Show fib(10).\n",
    ),
    ("list", "## Main\n    Let xs be [3, 1, 2].\n    Show length of xs.\n    Show item 2 of xs.\n"),
    ("text", "## Main\n    Show \"hello fabric\".\n"),
    ("text_concat", "## Main\n    Let name be \"LOGOS\".\n    Show name + \" on silicon\".\n    Show 2 + 3.\n"),
    ("bool", "## Main\n    Let b be 5 is greater than 3.\n    Show b.\n    Show 2 is greater than 3.\n"),
    (
        "text_ops",
        "## Main\n    Let s be \"abc\".\n    Show item 2 of s.\n    Show s equals \"abc\".\n    Show s equals \"abd\".\n    Show \"\".\n    Show length of s.\n",
    ),
    ("text_int", "## Main\n    Show \"n=\" + 42.\n    Show \"neg\" + (0 - 7).\n    Show \"zero\" + 0.\n    Show 1000000 + \"!\".\n"),
    // Small enough for the XC7A100T image (MAXCELL=32): concat, equality and length on that board's netlist too.
    ("text_small", "## Main\n    Let s be \"ab\".\n    Show s + \"c\".\n    Show s equals \"ab\".\n    Show length of s.\n"),
    ("float", "## Main\n    Show 1.5 + 2.25.\n"),
    (
        "mixed",
        "## To square (x: Int) -> Int:\n    Return x * x.\n## Main\n    Let mutable total be 0.\n    Let mutable k be 1.\n    While k is less than or equal to 4:\n        Set total to total + square(k).\n        Set k to k + 1.\n    Show \"sum of squares\".\n    Show total.\n    Show total is greater than 29.\n",
    ),
];

#[test]
fn main_programs_run_on_the_engine_model_with_the_vms_output() {
    for (name, src) in CORPUS {
        let cfg = program_main_fabric(src);
        assert_eq!(cfg.n_params, 0, "{name}: Main takes no parameters");
        let got = program_output(&cfg, &[]);
        assert_eq!(
            got,
            vm_lines(src),
            "{name}: the engine model's output tape is not the VM's output"
        );
    }
}

#[test]
fn show_cells_carry_the_type_the_value_actually_has() {
    // Register REUSE is the trap: a slot that held a Bool and now holds an Int must be shown as an Int. Each Show
    // records the tag the lowering assigned; the tags must match what the VM printed for that line.
    let src = "## Main\n    Let mutable v be 5 is greater than 3.\n    Show v.\n    Let t be \"x\".\n    Show t.\n    Show 1.25.\n    Show 7.\n";
    let cfg = program_main_fabric(src);
    let run = eval_wprog_config_run(&cfg, &[], &[]);
    let tags: Vec<u8> = run.shows.iter().map(|s| s.tag).collect();
    assert_eq!(
        tags,
        vec![TAG_BOOL, TAG_TEXT, TAG_FLOAT, TAG_INT],
        "type tags per Show"
    );
    let rendered: Vec<String> = run
        .shows
        .iter()
        .map(|s| render_show(*s, &run.heap))
        .collect();
    assert_eq!(rendered, vm_lines(src));
    // A shown value is still a value: it flows through the cell array like any other.
    assert_eq!(
        render_show(
            WShow {
                tag: TAG_INT,
                value: -42
            },
            &[]
        ),
        "-42"
    );
    assert_eq!(
        render_show(
            WShow {
                tag: TAG_FLOAT,
                value: 2.5f64.to_bits() as i64
            },
            &[]
        ),
        "2.5"
    );
    assert_eq!(
        render_show(
            WShow {
                tag: TAG_BOOL,
                value: 1
            },
            &[]
        ),
        "true"
    );
    assert_eq!(
        render_show(
            WShow {
                tag: TAG_TEXT,
                value: 0
            },
            &[2, b'o' as i64, b'k' as i64]
        ),
        "ok"
    );
}

/// Stream a Main program's image over the (simulated) wire into the SoC engine and read back every Show record
/// plus the final frame. Returns the rendered records.
fn shows_over_the_wire(name: &str, cfg: &WProgConfig, hw: bool) -> Option<Vec<String>> {
    if !common::require_tool("iverilog") {
        return None;
    }
    let dir = std::env::temp_dir().join("logos_wpe_main_iverilog");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let im = wprog_config_image(cfg);
    let bytes = im.wire_bytes(&[]);
    let run = eval_wprog_config_run(cfg, &[], &[]);
    std::fs::write(
        dir.join(format!("wire_{name}.hex")),
        bytes
            .iter()
            .map(|b| format!("{b:02x}\n"))
            .collect::<String>(),
    )
    .expect("write wire");
    std::fs::write(
        dir.join(format!("lens_{name}.hex")),
        format!("{:08x}\n", bytes.len()),
    )
    .expect("write lens");
    std::fs::write(
        dir.join(format!("oracle_{name}.hex")),
        format!("{:016x}\n", run.result as u64),
    )
    .expect("write oracle");
    let soc = if hw {
        emit_wprog_engine_soc_verilog_hw(name)
    } else {
        emit_wprog_engine_soc_verilog(name)
    };
    let tb = emit_wprog_engine_soc_testbench(name, 1, CLKS_PER_BIT, bytes.len());
    let srcv = dir.join(format!("wpe_soc_{name}.v"));
    let tbv = dir.join(format!("tb_wpe_soc_{name}.v"));
    let sim = dir.join(format!("sim_{name}.vvp"));
    std::fs::write(&srcv, &soc).expect("write soc");
    std::fs::write(&tbv, &tb).expect("write tb");
    let build = std::process::Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&sim)
        .arg(&srcv)
        .arg(&tbv)
        .current_dir(&dir)
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog failed on {name}:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let out = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("run vvp");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("PASS:"),
        "{name}: the SoC's final frame diverged from the model:\n{stdout}"
    );
    // The testbench prints one `SHOW t=<tag> v=<hex64>` per scalar record and, for a text record, `SHOWT len=<n>`
    // followed by `SHOWB <hex>` per byte. Reassemble and render exactly as the host does.
    let mut shows: Vec<String> = Vec::new();
    let mut text: Option<(usize, Vec<u8>)> = None;
    for line in stdout.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("SHOW t=") {
            let (t, v) = rest.split_once(" v=").expect("SHOW line");
            let tag: u8 = t.parse().expect("tag");
            let value = u64::from_str_radix(v.trim(), 16).expect("hex value") as i64;
            shows.push(render_show(WShow { tag, value }, &[]));
        } else if let Some(n) = l.strip_prefix("SHOWT len=") {
            let n: usize = n.trim().parse().expect("len");
            if n == 0 {
                shows.push(String::new()); // `Show ""` — a record with no bytes to follow
            } else {
                text = Some((n, Vec::new()));
            }
        } else if let Some(b) = l.strip_prefix("SHOWB ") {
            let (n, bytes) = text.as_mut().expect("SHOWB outside a text record");
            bytes.push(u8::from_str_radix(b.trim(), 16).expect("byte"));
            if bytes.len() == *n {
                shows.push(String::from_utf8_lossy(bytes).into_owned());
                text = None;
            }
        }
    }
    assert!(text.is_none(), "{name}: a text record was cut short");
    Some(shows)
}

#[test]
fn main_programs_stream_their_shows_over_the_wire_under_iverilog() {
    // The behavioral engine carries the FPU, so every corpus program (floats included) runs here.
    for (name, src) in CORPUS {
        let cfg = program_main_fabric(src);
        if let Some(got) = shows_over_the_wire(&format!("main_{name}"), &cfg, false) {
            assert_eq!(
                got,
                vm_lines(src),
                "{name}: the records the chip streamed are not the VM's output"
            );
        }
    }
}

#[test]
fn main_programs_stream_their_shows_on_the_fpga_netlist_under_iverilog() {
    // The XC7A100T image (WPE_HW caps, no FPU): every corpus program that fits it must stream the same output.
    for (name, src) in CORPUS {
        let cfg = program_main_fabric(src);
        if wprog_config_fits_hw_engine(&cfg).is_err() {
            eprintln!(
                "{name}: exceeds the XC7A100T engine caps — covered by the behavioral engine test"
            );
            continue;
        }
        if let Some(got) = shows_over_the_wire(&format!("hw_{name}"), &cfg, true) {
            assert_eq!(
                got,
                vm_lines(src),
                "{name}: the FPGA netlist's records are not the VM's output"
            );
        }
    }
}

#[test]
fn a_program_with_no_shows_still_answers_with_its_frame() {
    // Show records precede the final frame; a program with none must produce exactly the old reply.
    let src = "## Main\n    Let x be 1.\n";
    let cfg = program_main_fabric(src);
    assert!(eval_wprog_config_run(&cfg, &[], &[]).shows.is_empty());
    if let Some(got) = shows_over_the_wire("main_silent", &cfg, false) {
        assert!(got.is_empty());
    }
}
