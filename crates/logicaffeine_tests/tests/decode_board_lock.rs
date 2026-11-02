//! decode_board_lock — the int8 decode step's board image, run under iverilog through its serial line exactly as a host
//! drives the board.
//!
//! The image (`spatial::lower::fabric_board`) puts the step's RTL behind its host port and a byte protocol on an 8N1
//! line: `I` identifies the image, `W` and `R` write and read a tensor's word, `S` runs one step at a token and a
//! position, and `G` runs the autoregressive loop on chip. This lock holds the serial pair to every byte value, a word
//! to its round trip, and the whole golden trace to the line: the prompt stepped with `S` — each step's next token the
//! evaluator's and its cycles the twin's — then the rest generated with `G`, its tokens the trainer's and its cycles
//! exactly [`generate_cycles`] of the steps it ran. Three machines run it: the sky130 fabric, whose memories answer in
//! the cycle of the address, and the two Nexys parts, whose block RAM answers the cycle after and whose capped PE arrays
//! run every matmul in blocks.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test decode_board_lock` (iverilog-gated)

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/fabric_rtl.rs"]
mod fabric_rtl;
#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::lower::fabric::{fabric_cycle_model, FabricBackend, VerilogArtifact};
use logicaffeine_compile::spatial::lower::fabric::input_hex_files;
use logicaffeine_compile::spatial::lower::fabric_board::{
    board_image, clks_per_bit, decode_board_verilog, generate_cycles, uart_verilog, BoardProtocol, CoreClock, DecodeWords, NexysBoard,
};
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;
use spatial_support::{inputs_from_fills, prompt_next_tokens};

/// Clocks per serial bit in simulation: the protocol is the board's, the line merely fast.
const CPB: u32 = 4;

fn model() -> (Int8Model, Vec<usize>, Vec<usize>) {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let tokens = |k: &str| -> Vec<usize> {
        j["golden_trace"][k].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as usize).collect()
    };
    (Int8Model::from_json(&j).expect("the model"), tokens("prompt_tokens"), tokens("generated_tokens"))
}

fn fabrics() -> [SpatialTarget; 3] {
    [SpatialTarget::sky_fabric(16, 64), SpatialTarget::xc7a200t_fabric(16, 64), SpatialTarget::xc7a100t_fabric(16, 64)]
}

fn lowered(src: &str, target: &SpatialTarget) -> (SpatialProgram, VerilogArtifact) {
    let l = compile_program_to_spatial(src, target).expect("a tensor program");
    let p = l.program.unwrap_or_else(|| panic!("the step lowers whole: {:?}", l.missing.first()));
    let art = match FabricBackend::new(target.clone()).lower(&p) {
        Ok(SpatialArtifact::Verilog(v)) => v,
        other => panic!("the fabric lowers the step: {other:?}"),
    };
    (p, art)
}

fn u16le(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}

fn le(bytes: &[u8]) -> u64 {
    bytes.iter().rev().fold(0u64, |a, b| (a << 8) | *b as u64)
}

/// The serial pair alone, receiver into transmitter: every byte value goes out as it came in.
#[test]
fn the_serial_pair_carries_every_byte() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("decode_board_uart_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bit = 10 * CPB;
    let mut tb = uart_verilog();
    tb.push_str(&format!(
        "module tb;\n  reg clk = 0, rst = 1, rx = 1; wire tx, v; wire [7:0] d; wire busy; integer i, n = 0; reg [7:0] b;\n\
         uart_rx #({CPB}) r(.clk(clk), .rst(rst), .rx(rx), .valid(v), .data(d));\n\
         uart_tx #({CPB}) t(.clk(clk), .rst(rst), .go(v), .data(d), .tx(tx), .busy(busy));\n\
         always #5 clk = ~clk;\n\
         initial forever begin @(negedge tx); #({}); for (i = 0; i < 8; i = i + 1) begin #({bit}); b[i] = tx; end #({bit}); $display(\"TX %0d\", b); n = n + 1; end\n\
         task send(input [7:0] x); integer k; begin rx = 0; #({bit}); for (k = 0; k < 8; k = k + 1) begin rx = x[k]; #({bit}); end rx = 1; #({bit}); end endtask\n\
         integer c; initial begin #100 rst = 0; for (c = 0; c < 256; c = c + 1) begin send(c); wait (n == c + 1); end $display(\"END\"); $finish; end\n\
         endmodule\n",
        bit / 2
    ));
    let src = dir.join("tb.v");
    std::fs::write(&src, tb).unwrap();
    let bin = dir.join("tb.vvp");
    let build = std::process::Command::new("iverilog").arg("-g2012").arg("-o").arg(&bin).arg(&src).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new("vvp").arg(&bin).output().expect("vvp");
    let log = String::from_utf8_lossy(&run.stdout).to_string();
    let got: Vec<u32> = log.lines().filter_map(|l| l.strip_prefix("TX ")).map(|v| v.parse().unwrap()).collect();
    assert!(log.contains("END"), "every byte came back\n{log}");
    assert_eq!(got, (0..256).collect::<Vec<u32>>(), "every byte value, in order");
    let _ = std::fs::remove_dir_all(&dir);
}

fn repo() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The golden trace as the board runs it: identify, every prompt token stepped at its position, the rest generated —
/// each command with its reply's length.
fn trace_script(prompt: &[usize], golden: &[usize]) -> Vec<(Vec<u8>, usize)> {
    let mut script = vec![(b"I".to_vec(), 6)];
    for (pos, tok) in prompt.iter().enumerate() {
        let mut s = vec![b'S'];
        s.extend(u16le(*tok as u16 + 1));
        s.extend(u16le(pos as u16 + 1));
        script.push((s, 7));
    }
    let n = golden.len() - 1;
    let mut g = vec![b'G'];
    g.extend(u16le(n as u16));
    script.push((g, 2 * n + 5));
    script
}

/// `scripts/decode-host.py`, the host a board run uses, with `args`: its exit status and what it printed.
fn host(args: &[&str]) -> (bool, String, String) {
    let out = std::process::Command::new("python3")
        .arg(repo().join("scripts/decode-host.py"))
        .args(args)
        .output()
        .expect("python3");
    (out.status.success(), String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string())
}

/// The host sends a board exactly the trace this lock runs on the simulated board: its dry run prints each command's
/// bytes and reply length.
#[test]
fn the_host_sends_the_trace_the_board_is_proven_on() {
    if !common::require_tool("python3") {
        return;
    }
    let (_, prompt, golden) = model();
    let (ok, out, err) = host(&["B", "--dry-run"]);
    assert!(ok, "the host's dry run: {err}");
    let sent: Vec<(Vec<u8>, usize)> = out
        .lines()
        .map(|l| {
            let (hex, n) = l.split_once(' ').expect("a command and its reply length");
            let bytes = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
            (bytes, n.parse().unwrap())
        })
        .collect();
    assert_eq!(sent, trace_script(&prompt, &golden), "the host's commands are the proven trace's");
}

/// The host holds a board's replies to the trace and to the image record: fed the simulated XC7A100T board's own replies
/// it records the golden tokens, the evaluator's next token at every prompt position, the twin's cycles and the loop's,
/// and its decode time at the image's clock. Fed the same replies with one generated token, one prompt step's token, one
/// step's cycles or the loop's cycles changed, it refuses the run by name; and it refuses an image record that does not
/// say what the prompt steps reply and what the loop takes a step.
#[test]
fn the_host_holds_the_simulated_board_to_the_trace() {
    if !common::require_tool("iverilog") || !common::require_tool("python3") {
        return;
    }
    let (m, prompt, golden) = model();
    let target = SpatialTarget::xc7a100t_fabric(16, 64);
    let lat = target.site.local_mem_read_latency.unwrap() as u64;
    let (first, art) = lowered(&m.decode_step_lg(prompt[0], 0, &[], &[]), &target);
    let twin = fabric_cycle_model(&first, &target);
    let want = prompt_next_tokens(&m, &prompt, &target);
    let board = decode_board_verilog(DecodeWords::of(&first, "tok", "pos", "next").expect("the step's words"), lat);
    let script = trace_script(&prompt, &golden);
    let n = golden.len() - 1;
    let dir = std::env::temp_dir().join(format!("decode_board_host_{}", std::process::id()));
    let bound = (twin + 1000) * (prompt.len() + n) as u64 * 2;
    let replies = fabric_rtl::simulate_board(&dir, &first, &art, &inputs_from_fills(&first), &board, CPB, &script, bound);
    std::fs::create_dir_all(&dir).unwrap();
    let image = serde_json::json!({
        "part": "xc7a100tcsg324-1", "read_latency": lat, "cycles_per_step": twin, "core_mhz": 25.0,
        "loop_cycles_per_step": generate_cycles(&[twin], lat), "prompt_next_tokens": want,
        "bitstream": "none.bit", "sha256": "none"
    });
    let write_images = |tag: &str, image: &serde_json::Value| -> std::path::PathBuf {
        let path = dir.join(format!("images_{tag}.json"));
        std::fs::write(&path, serde_json::json!({ "images": [image] }).to_string()).unwrap();
        path
    };
    let images = write_images("full", &image);
    let run_on = |replies: &[Vec<u8>], tag: &str, images: &std::path::Path| -> (bool, String, String) {
        let file = dir.join(format!("replies_{tag}.txt"));
        let text: String = replies.iter().map(|r| r.iter().map(|b| format!("{b:02x}")).collect::<String>() + "\n").collect();
        std::fs::write(&file, text).unwrap();
        host(&["B", "--replay", file.to_str().unwrap(), "--images", images.to_str().unwrap()])
    };
    let run = |replies: &[Vec<u8>], tag: &str| run_on(replies, tag, &images);
    let (ok, out, err) = run(&replies, "board");
    assert!(ok, "the host accepts the simulated board's run: {err}");
    let got: serde_json::Value = serde_json::from_str(&out).expect("the host's record");
    let tokens: Vec<usize> = got["tokens"].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as usize).collect();
    assert_eq!(tokens, golden, "the host records the trainer's tokens");
    let stepped: Vec<usize> = got["prompt_next_tokens"].as_array().expect("the prompt steps' tokens").iter().map(|t| t.as_u64().unwrap() as usize).collect();
    assert_eq!(stepped, want, "the host records the evaluator's next token at every prompt position");
    assert!(got["step_cycles"].as_array().unwrap().iter().all(|c| c.as_u64() == Some(twin)), "every step took the twin's cycles");
    let loop_cycles = generate_cycles(&vec![twin; n], lat);
    assert_eq!(got["generate_cycles"].as_u64(), Some(loop_cycles), "the loop's own accounting");
    let us = loop_cycles as f64 / n as f64 / 25.0;
    assert!((got["us_per_token"].as_f64().unwrap() - us).abs() < 1e-2, "a token's time is the loop's cycles at the clock");
    assert_eq!(got.get("date"), Some(&serde_json::Value::Null), "a record says when its board ran, and a replay ran on none");
    let mut wrong_token = replies.clone();
    wrong_token[1 + prompt.len()][0] ^= 1;
    let (ok, _, err) = run(&wrong_token, "token");
    assert!(!ok && err.contains("generated"), "a changed token is refused by name: {err}");
    let mut wrong_prompt = replies.clone();
    wrong_prompt[5][1] ^= 1;
    let (ok, _, err) = run(&wrong_prompt, "prompt");
    assert!(!ok && err.contains("position 4"), "a prompt step's changed token is refused at its position: {err}");
    let mut wrong_cycles = replies.clone();
    wrong_cycles[1][3] ^= 1;
    let (ok, _, err) = run(&wrong_cycles, "cycles");
    assert!(!ok && err.contains("cycles"), "a step off the twin's cycles is refused by name: {err}");
    let mut wrong_loop = replies.clone();
    wrong_loop[1 + prompt.len()][2 * n + 1] ^= 1;
    let (ok, _, err) = run(&wrong_loop, "loop");
    assert!(!ok && err.contains("loop"), "a loop off its own accounting is refused by name: {err}");
    for field in ["prompt_next_tokens", "loop_cycles_per_step"] {
        let mut bare = image.clone();
        bare.as_object_mut().unwrap().remove(field);
        let (ok, _, err) = run_on(&replies, &format!("bare_{field}"), &write_images(field, &bare));
        assert!(!ok && err.contains(field) && !err.contains("Traceback"), "an image record without `{field}` is refused by name, not by a crash: {err}");
    }
    let mut short = image.clone();
    short["prompt_next_tokens"] = serde_json::json!(want[..want.len() - 1]);
    let (ok, _, err) = run_on(&replies, "short", &write_images("short", &short));
    assert!(!ok && err.contains("prompt") && !err.contains("Traceback"), "a record short of a prompt step's token is refused: {err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The Nexys A7 top as the image builds it — the oscillator through its PLL to the core, the reset held until the PLL
/// locks, `decode_board` on the board's UART pins at the bit time its clock gives 115200 baud — identifies itself and
/// runs a step whose next token is the evaluator's and whose cycles are the twin's.
#[test]
fn the_board_top_runs_the_protocol_through_its_pll() {
    if !common::require_tool("iverilog") {
        return;
    }
    let (m, prompt, _) = model();
    let target = SpatialTarget::xc7a100t_fabric(16, 64);
    let lat = target.site.local_mem_read_latency.unwrap() as u64;
    let (p, art) = lowered(&m.decode_step_lg(prompt[0], 0, &[], &[]), &target);
    let twin = fabric_cycle_model(&p, &target);
    let want = match &eval_choreography(&p, &inputs_from_fills(&p)).expect("the step evaluates").tensors[&p.tensors.iter().find(|t| t.name == "next").unwrap().id] {
        Value::Int(v) => v[0] as u64,
        Value::Float(v) => panic!("next through floats: {v:?}"),
    };
    let clock = CoreClock::fastest_within(35.0).expect("a clock");
    let words = DecodeWords::of(&p, "tok", "pos", "next").expect("the step's words");
    let top = "logos_decode_a7";
    let files = board_image(&art, &input_hex_files(&p, &inputs_from_fills(&p)), BoardProtocol::Decode(words), lat, NexysBoard::A7, top, clock);
    let mut sources: Vec<(String, String)> = files.into_iter().filter(|(n, _)| !n.ends_with(".xdc")).collect();
    let mut s = vec![b'S'];
    s.extend(u16le(prompt[0] as u16 + 1));
    s.extend(u16le(1));
    let script = vec![(b"I".to_vec(), 6), (s, 7)];
    let bit_ns = clks_per_bit(clock) as f64 * 1000.0 / clock.mhz();
    let bound_ns = ((twin + 20_000) as f64 * 1000.0 / clock.mhz() + 30.0 * 10.0 * bit_ns) as u64;
    sources.push(("clocking.v".into(), fabric_rtl::clocking_models().into()));
    sources.push(("tb.v".into(), fabric_rtl::top_testbench(top, bit_ns, &script, bound_ns)));
    let dir = std::env::temp_dir().join(format!("decode_board_top_{}", std::process::id()));
    let replies = fabric_rtl::run_board(&dir, &sources, &script, bound_ns);
    assert_eq!(replies[0], [b'i', b'L', b'O', b'G', b'S', lat as u8], "the top identifies its image at {:.3} MHz", clock.mhz());
    assert_eq!(replies[1][0], b's', "a step reply");
    assert_eq!(le(&replies[1][1..3]), want, "the step's next token is the evaluator's");
    assert_eq!(le(&replies[1][3..7]), twin, "the step takes the twin's cycles");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The board identifies itself and its read latency, writes a word it is sent, and reads the word back — every byte of
/// it, a negative value's sign bits included.
#[test]
fn the_board_identifies_itself_and_round_trips_a_word() {
    if !common::require_tool("iverilog") {
        return;
    }
    let (m, prompt, _) = model();
    for target in fabrics() {
        let label = format!("words@{}", target.name);
        let lat = target.site.local_mem_read_latency.unwrap() as u64;
        let (p, art) = lowered(&m.decode_step_lg(prompt[0], 0, &[], &[]), &target);
        let words = DecodeWords::of(&p, "tok", "pos", "next").expect("the step's words");
        let board = decode_board_verilog(words, lat);
        let kc = p.tensors.iter().find(|t| t.name == "Kc").unwrap();
        let addr = logicaffeine_compile::spatial::lower::fabric::host_word(&p, kc, 0, 37) as u32;
        let mut script = vec![(b"I".to_vec(), 6)];
        for v in [0x0123_4567_89ab_cdefu64, (-1234567i64) as u64] {
            let mut w = vec![b'W'];
            w.extend(u16le(kc.id.idx() as u16));
            w.extend(addr.to_le_bytes());
            w.extend(v.to_le_bytes());
            script.push((w, 1));
            let mut r = vec![b'R'];
            r.extend(u16le(kc.id.idx() as u16));
            r.extend(addr.to_le_bytes());
            script.push((r, 9));
        }
        let dir = std::env::temp_dir().join(format!("decode_board_{label}_{}", std::process::id()));
        let replies = fabric_rtl::simulate_board(&dir, &p, &art, &inputs_from_fills(&p), &board, CPB, &script, 2_000_000);
        assert_eq!(replies[0], [b'i', b'L', b'O', b'G', b'S', lat as u8], "{label}: the image and its read latency");
        for (k, v) in [0x0123_4567_89ab_cdefu64, (-1234567i64) as u64].iter().enumerate() {
            assert_eq!(replies[1 + 2 * k], [b'k'], "{label}: the write is acknowledged");
            assert_eq!(replies[2 + 2 * k][0], b'r', "{label}: a read reply");
            assert_eq!(le(&replies[2 + 2 * k][1..]), *v, "{label}: the word reads back as written");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The golden trace over the serial line. The prompt is stepped with `S` — each reply's next token the evaluator's for
/// that position after the steps before it, its cycles the twin's — and the board then generates the rest itself with
/// `G`: the tokens it sends are the trainer's, and the cycles it reports are exactly the loop's own accounting of the
/// steps it ran.
#[test]
fn the_board_generates_the_golden_tokens_over_its_serial_line() {
    if !common::require_tool("iverilog") {
        return;
    }
    let (m, prompt, golden) = model();
    for target in fabrics() {
        let label = format!("golden@{}", target.name);
        let lat = target.site.local_mem_read_latency.unwrap() as u64;
        let (first, art) = lowered(&m.decode_step_lg(prompt[0], 0, &[], &[]), &target);
        let twin = fabric_cycle_model(&first, &target);
        let board = decode_board_verilog(DecodeWords::of(&first, "tok", "pos", "next").expect("the step's words"), lat);
        let want = prompt_next_tokens(&m, &prompt, &target);
        let mut script = Vec::new();
        for (pos, tok) in prompt.iter().enumerate() {
            let mut s = vec![b'S'];
            s.extend(u16le(*tok as u16 + 1));
            s.extend(u16le(pos as u16 + 1));
            script.push((s, 7));
        }
        let n = golden.len() - 1;
        let mut g = vec![b'G'];
        g.extend(u16le(n as u16));
        script.push((g, 2 * n + 5));
        let dir = std::env::temp_dir().join(format!("decode_board_{label}_{}", std::process::id()));
        let bound = (twin + 1000) * (prompt.len() + n) as u64 * 2;
        let replies = fabric_rtl::simulate_board(&dir, &first, &art, &inputs_from_fills(&first), &board, CPB, &script, bound);
        for (pos, (r, w)) in replies[..prompt.len()].iter().zip(&want).enumerate() {
            assert_eq!(r[0], b's', "{label}: a step reply at position {pos}");
            assert_eq!(le(&r[1..3]) as usize - 1, *w, "{label}: the next token at position {pos} is the evaluator's");
            assert_eq!(le(&r[3..7]), twin, "{label}: the step at position {pos} takes the twin's cycles");
        }
        assert_eq!(
            le(&replies[prompt.len() - 1][1..3]) as usize - 1,
            golden[0],
            "{label}: the prompt's last step predicts the trainer's first token"
        );
        let gen = &replies[prompt.len()];
        let tokens: Vec<usize> = (0..n).map(|k| le(&gen[2 * k..2 * k + 2]) as usize - 1).collect();
        assert_eq!(tokens, golden[1..], "{label}: the board generates the trainer's tokens");
        assert_eq!(gen[2 * n], b'e', "{label}: the loop ends with its cycles");
        assert_eq!(
            le(&gen[2 * n + 1..]),
            generate_cycles(&vec![twin; n], lat),
            "{label}: the loop takes the steps' cycles and the protocol's own"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
