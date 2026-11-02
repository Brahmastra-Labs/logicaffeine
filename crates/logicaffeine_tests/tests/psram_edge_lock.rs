//! psram_edge_lock — the whole-program engine's off-chip memory edge: a QSPI PSRAM behind the same
//! descriptor contract the DRAM path uses, so the engine's heap, frame
//! stack and program image can leave the die (`wpe_floorplan_lock`: they must, to fit a Caravel user
//! area with the engine).
//!
//! The part is the AP Memory APS6404L-3SQN (8 MiB, 3.3 V — Caravel's user GPIO pads run at 3.3 V),
//! held as data with its datasheet provenance. What this file locks:
//!   1. the part's numbers ARE its datasheet's;
//!   2. a transfer is split into bursts that never cross a 1 KiB page (the part wraps inside a page)
//!      and never hold chip-select low past tCEM (the part cannot refresh while selected), and an
//!      independent legality check refuses a plan that breaks either;
//!   3. the edge refuses a clock the part cannot take;
//!   4. under iverilog, the emitted controller driving a pin-level model of the part reads back
//!      everything it wrote, across page boundaries and the top of the address space, with ZERO
//!      protocol violations, and every burst takes exactly the cycles the edge documents;
//!   5. each protocol check the model makes is proven to fire, by driving the controller with the
//!      one parameter that breaks it — a model that cannot report a violation proves nothing by
//!      reporting none;
//!   6. the emitted artifacts in `benchmarks/silicon/` are exactly what the emitter produces.

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::dram::StreamDesc;
use logicaffeine_compile::codegen_sva::psram::{
    check_legal, emit_psram_edge_testbench, emit_psram_edge_verilog, emit_psram_model_verilog,
    plan_stream, EdgeOverrides, PsramBurst, PsramEdge, PsramUnplaceable, APS6404L_3SQN,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// A 50 MHz system clock and a 25 MHz SCLK: well inside the part's 109 MHz (3.3 V) limit and
/// within what Caravel's GPIO pads drive.
fn edge() -> PsramEdge {
    PsramEdge::new(APS6404L_3SQN, 20_000, 1).expect("a 25 MHz SCLK is legal for the part")
}

#[test]
fn the_part_is_its_datasheet() {
    let p = APS6404L_3SQN;
    assert!(
        p.provenance.contains("APS6404L-3SQN") && p.provenance.contains("Rev. 2.7"),
        "{}",
        p.provenance
    );
    assert_eq!(p.capacity_bytes, 8 << 20, "64 Mb, 8M x 8, A[22:0]");
    assert_eq!(p.page_bytes, 1024, "page size 1K, CA[9:0]");
    assert_eq!(
        p.t_clk_min_ps, 9_170,
        "tCLK, all other operations, 3.3 V package (109 MHz)"
    );
    assert_eq!(
        p.t_cph_min_ps, 18_000,
        "tCPH, CE# high between subsequent burst operations"
    );
    assert_eq!(
        p.t_cem_max_ps, 8_000_000,
        "tCEM, CE# low pulse width, standard grade"
    );
    assert_eq!(p.t_pu_min_ps, 150_000_000, "tPU, power-up");
    assert_eq!(
        p.t_rst_min_ps, 50_000,
        "tRST, end of RST command to next valid command"
    );
    assert_eq!(p.t_aclk_max_ps, 5_500, "tACLK, CLK to output delay");
    assert_eq!(
        (p.read_cmd, p.read_wait, p.write_cmd),
        (0xEB, 6, 0x38),
        "QPI Fast Quad Read / Quad Write"
    );
    assert_eq!((p.enter_qpi, p.reset_enable, p.reset), (0x35, 0x66, 0x99));
}

#[test]
fn a_stream_is_split_at_pages_and_at_the_chip_select_limit() {
    let e = edge();
    // CE# is low for every SCLK edge of the burst: command (2) + address (6) + wait (reads: 6) +
    // 16 data edges per 64-bit word, at 40 ns an edge; tCEM is 8 us.
    let max_read = (0..)
        .take_while(|n| (14 + 16 * n) * 40 <= 8_000)
        .last()
        .unwrap();
    let max_write = (0..)
        .take_while(|n| (8 + 16 * n) * 40 <= 8_000)
        .last()
        .unwrap();
    assert_eq!(
        (e.max_words(false), e.max_words(true)),
        (max_read, max_write)
    );

    for write in [false, true] {
        let s = StreamDesc {
            base: 100 * 8,
            elem_bytes: 8,
            stride_bytes: 8,
            count: 1000,
            write,
        };
        let plan = plan_stream(&e, &s).expect("a contiguous stream plans");
        let mut next = 100u32;
        for b in &plan {
            assert_eq!(b.write, write);
            assert_eq!(
                b.word_addr, next,
                "bursts cover the stream in order, without gaps"
            );
            assert!(b.words >= 1 && b.words <= e.max_words(write), "{b:?}");
            assert_eq!(
                b.word_addr / 128,
                (b.word_addr + b.words - 1) / 128,
                "{b:?} crosses a 1 KiB page"
            );
            next += b.words;
        }
        assert_eq!(next, 1100, "every word planned exactly once");
        check_legal(&e, &plan).expect("the planner's own plan is legal");
    }

    let across = [PsramBurst {
        write: true,
        word_addr: 127,
        words: 2,
    }];
    assert!(
        check_legal(&e, &across).unwrap_err().contains("page"),
        "a burst across a page must be refused"
    );
    let long = [PsramBurst {
        write: false,
        word_addr: 0,
        words: e.max_words(false) + 1,
    }];
    assert!(
        check_legal(&e, &long).unwrap_err().contains("tCEM"),
        "a burst past tCEM must be refused"
    );
    let top = [PsramBurst {
        write: false,
        word_addr: (8 << 20) / 8,
        words: 1,
    }];
    assert!(
        check_legal(&e, &top).is_err(),
        "a burst past the part's capacity must be refused"
    );
}

#[test]
fn the_edge_refuses_a_clock_the_part_cannot_take() {
    assert!(
        matches!(
            PsramEdge::new(APS6404L_3SQN, 4_000, 1),
            Err(PsramUnplaceable::SclkTooFast { .. })
        ),
        "an 8 ns SCLK is faster than the part's 9.17 ns"
    );
    assert!(
        PsramEdge::new(APS6404L_3SQN, 4_000, 2).is_ok(),
        "the same clock divided to 16 ns is legal"
    );
    let e = edge();
    assert_eq!(e.sclk_period_ps(), 40_000);
    assert_eq!(e.powerup_cycles(), 7_500, "150 us at 20 ns");
    assert_eq!(e.cph_cycles(), 1, "18 ns rounds up to one 20 ns cycle");
    assert_eq!(e.rst_cycles(), 3, "50 ns rounds up to three 20 ns cycles");
}

/// One co-simulation: `bursts` through the emitted controller into the emitted model. Returns the
/// words read (in order; `None` for a word with undefined bits), the per-burst cycle counts, and the
/// model's violation report.
struct Run {
    reads: Vec<Option<u64>>,
    cycles: Vec<u64>,
    violations: HashMap<String, u64>,
    total: u64,
}

fn run(
    e: &PsramEdge,
    over: &EdgeOverrides,
    bursts: &[PsramBurst],
    wdata: &[u64],
    tag: &str,
) -> Option<Run> {
    if !common::require_tool("iverilog") {
        return None;
    }
    let dir = std::env::temp_dir().join(format!("logos_psram_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("edge.v"), emit_psram_edge_verilog(e)).unwrap();
    std::fs::write(dir.join("model.v"), emit_psram_model_verilog(&e.part)).unwrap();
    std::fs::write(
        dir.join("tb.v"),
        emit_psram_edge_testbench(e, over, bursts, wdata),
    )
    .unwrap();
    let build = Command::new("iverilog")
        .current_dir(&dir)
        .args(["-g2012", "-o", "sim.vvp", "tb.v", "edge.v", "model.v"])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "iverilog: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let out = Command::new("vvp")
        .current_dir(&dir)
        .arg("sim.vvp")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        text.contains("PSRAM-DONE"),
        "the simulation did not finish:\n{}",
        &text[text.len().saturating_sub(2000)..]
    );
    let mut r = Run {
        reads: vec![],
        cycles: vec![],
        violations: HashMap::new(),
        total: 0,
    };
    for l in text.lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        match f.first() {
            Some(&"RD") => r.reads.push(u64::from_str_radix(f[1], 16).ok()),
            Some(&"BURST") => r.cycles.push(f[1].parse().unwrap()),
            Some(&"PSRAM-VIOLATION") => *r.violations.entry(f[1].to_string()).or_default() += 1,
            Some(&"PSRAM-VIOLATIONS") => r.total = f[1].parse().unwrap(),
            _ => {}
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    Some(r)
}

fn lcg(s: &mut u64) -> u64 {
    *s = s
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *s
}

#[test]
fn the_controller_and_the_part_agree_cycle_for_cycle() {
    let e = edge();
    let mut bursts = Vec::new();
    let mut wdata = Vec::new();
    let mut mem: HashMap<u32, u64> = HashMap::new();
    let mut seed = 0x5eed;
    let mut write = |bursts: &mut Vec<PsramBurst>, wdata: &mut Vec<u64>, base: u32, count: u64| {
        let s = StreamDesc {
            base: base as u64 * 8,
            elem_bytes: 8,
            stride_bytes: 8,
            count,
            write: true,
        };
        for b in plan_stream(&e, &s).unwrap() {
            for k in 0..b.words {
                let v = lcg(&mut seed);
                wdata.push(v);
                mem.insert(b.word_addr + k, v);
            }
            bursts.push(b);
        }
    };
    write(&mut bursts, &mut wdata, 120, 300); // crosses the pages at words 128, 256 and 384
    write(&mut bursts, &mut wdata, (8 << 20) / 8 - 5, 5); // the top of the address space
    write(&mut bursts, &mut wdata, 0x80000, 1); // A[22] alone
    let mut expect = Vec::new();
    for (base, count) in [
        (120u32, 300u64),
        ((8 << 20) / 8 - 5, 5),
        (0x80000, 1),
        (200, 40),
    ] {
        let s = StreamDesc {
            base: base as u64 * 8,
            elem_bytes: 8,
            stride_bytes: 8,
            count,
            write: false,
        };
        for b in plan_stream(&e, &s).unwrap() {
            expect.extend((0..b.words).map(|k| mem[&(b.word_addr + k)]));
            bursts.push(b);
        }
    }
    check_legal(&e, &bursts).expect("the traffic is a legal plan");

    let Some(r) = run(&e, &EdgeOverrides::default(), &bursts, &wdata, "agree") else {
        return;
    };
    assert_eq!(
        r.total, 0,
        "the part saw protocol violations: {:?}",
        r.violations
    );
    assert!(
        r.reads.iter().all(Option::is_some),
        "a word read back had undefined bits"
    );
    assert_eq!(
        r.reads.into_iter().flatten().collect::<Vec<_>>(),
        expect,
        "the words read back are not the words written"
    );
    let want: Vec<u64> = bursts
        .iter()
        .map(|b| e.burst_sys_cycles(b.write, b.words))
        .collect();
    assert_eq!(
        r.cycles, want,
        "every burst must take exactly the cycles the edge documents"
    );
}

#[test]
fn every_protocol_check_the_model_makes_is_proven_to_fire() {
    let e = edge();
    // At 20 ns a single cycle of CE# high already exceeds tCPH (18 ns); the tCPH case needs a 5 ns
    // clock, its SCLK divided by two to stay inside the part's limit.
    let fast = PsramEdge::new(APS6404L_3SQN, 5_000, 2).expect("a 20 ns SCLK is legal");
    let one = [PsramBurst {
        write: true,
        word_addr: 0,
        words: 1,
    }];
    let cases: [(&str, PsramEdge, EdgeOverrides, Vec<PsramBurst>); 5] = [
        (
            "tPU",
            e,
            EdgeOverrides {
                powerup_cycles: Some(100),
                ..Default::default()
            },
            one.to_vec(),
        ),
        (
            "tCPH",
            fast,
            EdgeOverrides {
                cph_cycles: Some(0),
                ..Default::default()
            },
            vec![one[0], one[0]],
        ),
        (
            "tRST",
            e,
            EdgeOverrides {
                rst_cycles: Some(0),
                ..Default::default()
            },
            one.to_vec(),
        ),
        (
            "tCEM",
            e,
            EdgeOverrides::default(),
            vec![PsramBurst {
                write: false,
                word_addr: 0,
                words: e.max_words(false) + 1,
            }],
        ),
        (
            "wrap",
            e,
            EdgeOverrides::default(),
            vec![PsramBurst {
                write: true,
                word_addr: 127,
                words: 2,
            }],
        ),
    ];
    for (kind, e, over, bursts) in cases {
        let wdata = vec![0x0123_4567_89ab_cdef; bursts.iter().map(|b| b.words as usize).sum()];
        let Some(r) = run(&e, &over, &bursts, &wdata, kind) else {
            return;
        };
        assert!(r.violations.get(kind).copied().unwrap_or(0) > 0,
            "driving the part with a broken {kind} must be reported as a {kind} violation; the model reported {:?}",
            r.violations);
    }
}

#[test]
fn the_edge_artifacts_are_the_emitted_ones() {
    let e = edge();
    assert_eq!(
        emit_psram_edge_verilog(&e),
        emit_psram_edge_verilog(&e),
        "emission is deterministic"
    );
    let sil = root().join("benchmarks/silicon");
    let rtl = std::fs::read_to_string(sil.join("psram_edge.v"))
        .expect("benchmarks/silicon/psram_edge.v — run the emit_psram_edge example");
    assert_eq!(
        rtl,
        emit_psram_edge_verilog(&e),
        "psram_edge.v is not what the emitter writes"
    );
    let model = std::fs::read_to_string(sil.join("psram_model.v"))
        .expect("benchmarks/silicon/psram_model.v — run the emit_psram_edge example");
    assert_eq!(
        model,
        emit_psram_model_verilog(&e.part),
        "psram_model.v is not what the emitter writes"
    );
}
