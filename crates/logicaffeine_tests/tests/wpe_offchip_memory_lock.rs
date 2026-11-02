//! wpe_offchip_memory_lock — the whole-program engine with its heap, frame stack and config image off the
//! die, in a QSPI PSRAM behind `psram_cache`s, a `psram_arbiter` and one `psram_edge`, runs every program
//! exactly as the on-chip engine does.
//!
//! WHY. A Caravel user area holds a few hundred kilobits of SRAM, and the solved ASIC floorplan
//! (`wpe_floorplan_lock`) fits only with the heap, the frame stack and the config image off the die. Each
//! of those memories already has a registered-read port — its FSM issues an address and takes the word a
//! cycle later — so each can sit behind a cache that keeps that port, and a miss can freeze the whole FSM
//! instead of changing it. The only thing a miss may change is wall-clock time.
//!
//! What this file locks:
//!   1. a cache is a memory: engine-shaped random traffic (reads, writes of either or both halves, a write
//!      and a read in one cycle) through cache, edge and the part's pin model reads back the reference
//!      memory with zero timing violations, at several geometries; the longest stall is the derived bound
//!      exactly, and the edge's initialization takes exactly `init_cycles`;
//!   2. caches sharing the edge through the arbiter stay memories when all of them miss at once;
//!   3. both benches can fail: every mutation of the cache or the arbiter produces wrong reads or a hang,
//!      and the part's pin model reports a controller that leaves a sampled SIO bit undriven or drives
//!      against the part's read data;
//!   4. the on-chip engine is untouched: no off-chip text reaches an on-chip emission and no marker line
//!      reaches any emission;
//!   5. the RTL takes exactly what the floorplan sends off-chip: an array is declared by the off-chip
//!      engine if and only if `plan_wpe` gives it an on-chip home;
//!   6. off-chip ≡ on-chip ≡ model: heap-heavy, text and recursive programs stream the same Show records,
//!      results and run-cycle counts from both engines with zero PSRAM violations and no lost UART byte,
//!      for the heap, the frame stack and the config image off-chip alone and together, at a two-line
//!      cache that thrashes and at the shipped geometry;
//!   7. the UART bound: the held byte carries a byte across a stall that spans one, removing the hold loses
//!      it, a stall that spans two loses one, and `check_uart` accepts exactly the byte times no loading
//!      stall can span;
//!   8. an edge timed for a clock range meets every rule at both ends, and the burst the fast end alone
//!      would allow breaks tCEM at the slow end;
//!   9. refusals: nothing off-chip, the config image off-chip without its registered port, regions past
//!      the part, a one-line or one-word cache, a line longer than one legal burst, and a part still
//!      initializing when the loader first reaches it.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(wpe_offchip_memory_lock)'`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::fabric_rtl::program_cfg_fabric;
use logicaffeine_compile::codegen_sva::pdk_target::PdkTarget;
use logicaffeine_compile::codegen_sva::psram::{
    emit_psram_arbiter_testbench, emit_psram_arbiter_verilog, emit_psram_cache_testbench,
    emit_psram_cache_verilog, emit_psram_edge_verilog, emit_psram_model_verilog, PsramCache,
    PsramEdge, PsramPart, PsramUnplaceable, APS6404L_3SQN,
};
use logicaffeine_compile::codegen_sva::wpe_floorplan::{
    plan_wpe, ArrayRole, Residence, WpeProfile,
};
use logicaffeine_compile::codegen_sva::wpe_offchip::{
    emit_wprog_engine_soc_offchip_testbench, emit_wprog_engine_soc_verilog_offchip,
    uart_byte_cycles, wire_bytes_before_first_part_access, OffchipMemory, OffchipUnplaceable,
};
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    emit_wprog_engine_soc_testbench, emit_wprog_engine_soc_verilog,
    emit_wprog_engine_soc_verilog_backend, emit_wprog_engine_soc_verilog_backend_debug,
    emit_wprog_engine_soc_verilog_backend_rdw, emit_wprog_engine_soc_verilog_hw,
    eval_wprog_config_counted_heap_preloaded, eval_wprog_config_run, program_main_fabric,
    render_show, serialize_program, wprog_config_image, MemBackend, WShow, WPE_STREAM_CAPS,
    WPE_SYNTH_DEPTH, WPE_SYNTH_HEAP, WPE_SYNTH_MAXARG, WPE_SYNTH_MAXBLK, WPE_SYNTH_MAXCELL,
    WPE_SYNTH_MAXK, WPE_SYNTH_MAXLIV, WPE_SYNTH_MAXWR, WPE_SYNTH_NREG,
};
use logicaffeine_compile::compile::vm_outcome;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The engine's clock: 20 ns, the period the edge block is hardened at; SCLK is half of it.
const SYS_PS: u32 = 20_000;

fn edge() -> PsramEdge {
    PsramEdge::new(APS6404L_3SQN, SYS_PS, 1).expect("the part runs at a 20 ns system clock")
}

fn cache(index_bits: u32, offset_bits: u32) -> PsramCache {
    PsramCache::new(&edge(), index_bits, offset_bits).expect("a placeable cache")
}

/// Two lines of eight words: every memory of more than sixteen words thrashes it.
fn thrashing() -> PsramCache {
    cache(1, 3)
}

/// The geometry the engine ships with: 64 lines of 8 words, 4 KiB per memory.
fn shipped() -> PsramCache {
    cache(6, 3)
}

fn memory(heap: bool, frames: bool, cells: bool, c: PsramCache) -> OffchipMemory {
    OffchipMemory {
        edge: edge(),
        heap: heap.then_some(c),
        frames: frames.then_some(c),
        cells: cells.then_some(c),
    }
}

fn workdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join("logos_wpe_offchip").join(tag);
    std::fs::create_dir_all(&d).expect("create the simulation directory");
    d
}

/// Compile `files` (written into `dir`) with iverilog and run them; the simulation's stdout.
fn simulate(dir: &Path, files: &[(&str, String)]) -> String {
    for (name, text) in files {
        std::fs::write(dir.join(name), text).expect("write a simulation source");
    }
    let build = Command::new("iverilog")
        .args(["-g2012", "-o", "sim.vvp"])
        .args(files.iter().map(|(n, _)| *n))
        .current_dir(dir)
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog failed in {}:\n{}",
        dir.display(),
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new("vvp")
        .args(["-n", "sim.vvp"])
        .current_dir(dir)
        .output()
        .expect("run vvp");
    String::from_utf8_lossy(&run.stdout).into_owned()
}

fn field(out: &str, key: &str) -> i64 {
    out.lines()
        .find_map(|l| {
            l.strip_prefix(key)
                .map(|v| v.trim().parse::<i64>().expect("a number after the key"))
        })
        .unwrap_or_else(|| panic!("no `{key}` line in:\n{out}"))
}

// ── 1-3: the cache and the arbiter on their own ────────────────────────────────────────────────────────────

struct CacheRun {
    errors: i64,
    violations: i64,
    max_stall: i64,
    ready_at: i64,
    hit_mismatches: i64,
}

fn run_cache(tag: &str, c: &PsramCache, seed: u32, cache_rtl: &str) -> CacheRun {
    let e = edge();
    let out = simulate(
        &workdir(tag),
        &[
            ("psram_cache.v", cache_rtl.to_string()),
            ("psram_edge.v", emit_psram_edge_verilog(&e)),
            ("psram_model.v", emit_psram_model_verilog(&e.part)),
            (
                "tb.v",
                emit_psram_cache_testbench(&e, c, e.sys_period_ps, seed, 256, 2000),
            ),
        ],
    );
    assert!(
        out.contains("CACHE-DONE"),
        "{tag}: the cache bench did not finish:\n{out}"
    );
    CacheRun {
        errors: field(&out, "CACHE-ERRORS"),
        violations: field(&out, "PSRAM-VIOLATIONS"),
        max_stall: field(&out, "CACHE-MAX-STALL"),
        ready_at: field(&out, "EDGE-READY-AT"),
        hit_mismatches: field(&out, "CACHE-HIT-MISMATCHES"),
    }
}

#[test]
fn a_cache_reads_back_what_was_written_under_engine_shaped_random_traffic() {
    if !common::require_tool("iverilog") {
        return;
    }
    let e = edge();
    for (index_bits, offset_bits, seeds) in [
        (1, 3, [1u32, 99]),
        (2, 3, [7, 31337]),
        (3, 2, [5, 11]),
        (1, 1, [3, 17]),
        (6, 3, [2, 23]),
    ] {
        let c = cache(index_bits, offset_bits);
        for seed in seeds {
            let tag = format!("cache_i{index_bits}_o{offset_bits}_s{seed}");
            let r = run_cache(&tag, &c, seed, &emit_psram_cache_verilog(&c));
            assert_eq!(
                r.errors, 0,
                "{tag}: a read returned something other than the last halves written there"
            );
            assert_eq!(
                r.violations, 0,
                "{tag}: the edge broke a timing rule of the part"
            );
            assert_eq!(
                r.hit_mismatches, 0,
                "{tag}: a hit the bench registered beside an address disagreed with the cache's lines"
            );
            assert_eq!(
                r.ready_at as u64,
                e.init_cycles() + 1,
                "{tag}: req_ready must first be seen at the edge after init_cycles"
            );
            assert!(
                r.max_stall as u64 <= c.worst_stall_cycles(&e),
                "{tag}: a stall of {} cycles outlasts the bound {}",
                r.max_stall,
                c.worst_stall_cycles(&e)
            );
            if index_bits <= 2 {
                assert_eq!(
                    r.max_stall as u64,
                    c.worst_stall_cycles(&e),
                    "{tag}: a thrashing cache meets a write and a read that both evict dirty lines; the longest \
                     stall is exactly the bound"
                );
            }
        }
    }
}

fn run_arbiter(tag: &str, n: u32, seed: u32, arbiter_rtl: &str) -> String {
    let e = edge();
    let c = thrashing();
    simulate(
        &workdir(tag),
        &[
            ("psram_cache.v", emit_psram_cache_verilog(&c)),
            ("psram_arbiter.v", arbiter_rtl.to_string()),
            ("psram_edge.v", emit_psram_edge_verilog(&e)),
            ("psram_model.v", emit_psram_model_verilog(&e.part)),
            (
                "tb.v",
                emit_psram_arbiter_testbench(&e, &c, n, seed, 64, 1500),
            ),
        ],
    )
}

#[test]
fn caches_sharing_the_edge_stay_memories_when_they_all_miss_at_once() {
    if !common::require_tool("iverilog") {
        return;
    }
    let (e, c) = (edge(), thrashing());
    for (n, seed) in [(2u32, 5u32), (3, 0x2468_ACE1), (5, 77)] {
        let tag = format!("arbiter_n{n}_s{seed}");
        let out = run_arbiter(&tag, n, seed, &emit_psram_arbiter_verilog());
        assert!(
            out.contains("MULTI-DONE"),
            "{tag}: the bench did not finish:\n{out}"
        );
        assert_eq!(
            field(&out, "MULTI-ERRORS"),
            0,
            "{tag}: a cache read something other than its own last write"
        );
        assert_eq!(
            field(&out, "PSRAM-VIOLATIONS"),
            0,
            "{tag}: the edge broke a timing rule of the part"
        );
        assert!(
            field(&out, "MULTI-MAX-STALL") as u64 <= n as u64 * c.worst_stall_cycles(&e),
            "{tag}: caches queued behind one edge wait at most for each other's worst miss"
        );
    }
}

#[test]
fn the_benches_catch_every_broken_cache_and_arbiter() {
    if !common::require_tool("iverilog") {
        return;
    }
    let c = thrashing();
    let good = emit_psram_cache_verilog(&c);
    let cache_mutants: [(&str, &str, &str); 7] = [
        (
            "no_write_merge",
            "(fill && !(merge && wm[0])) ? e_rdata[31:0]",
            "fill ? e_rdata[31:0]",
        ),
        (
            "no_write_back",
            "cs <= (cval[idx] && cdirty[idx]) ? C_WBQ : C_FQ;",
            "cs <= C_FQ;",
        ),
        (
            "no_pending_read",
            "if (hit_w) begin cdirty[wi] <= 1'b1; if (req_r) cs <= C_PEND; end",
            "if (hit_w) begin cdirty[wi] <= 1'b1; end",
        ),
        (
            "skipped_word",
            "if (e_wdata_take) wcnt <= wcnt + 1'b1;",
            "if (e_wdata_take) wcnt <= wcnt + 2'd2;",
        ),
        (
            "lost_dirty_bit",
            "cdirty[mi] <= mis_we;",
            "cdirty[mi] <= 1'b0;",
        ),
        (
            "half_enable_ignored",
            "if (fill || (hit_wr && wm[0])) cd_lo[dwa] <= dwd_lo;",
            "if (fill || hit_wr) cd_lo[dwa] <= dwd_lo;",
        ),
        (
            "merge_ignores_halves",
            "(fill && !(merge && wm[1])) ? e_rdata[63:32]",
            "(fill && !merge) ? e_rdata[63:32]",
        ),
    ];
    for (name, from, to) in cache_mutants {
        assert_eq!(
            good.matches(from).count(),
            1,
            "{name}: the mutation site must exist exactly once"
        );
        let r = run_cache(
            &format!("mutant_{name}"),
            &c,
            305_419_896,
            &good.replace(from, to),
        );
        assert!(r.errors > 0, "{name}: a broken cache passed the bench");
    }
    let good = emit_psram_arbiter_verilog();
    let arbiter_mutants: [(&str, &str, &str); 2] = [
        (
            "burst_data_from_the_newest_requester",
            "wire [7:0] sel = pend ? owner : pick;",
            "wire [7:0] sel = pick;",
        ),
        (
            "completion_to_the_wrong_cache",
            "begin pend <= 1'b1; owner <= pick; end",
            "begin pend <= 1'b1; owner <= 8'd0; end",
        ),
    ];
    for (name, from, to) in arbiter_mutants {
        assert_eq!(
            good.matches(from).count(),
            1,
            "{name}: the mutation site must exist exactly once"
        );
        let out = run_arbiter(
            &format!("mutant_{name}"),
            3,
            0x2468_ACE1,
            &good.replace(from, to),
        );
        assert!(
            out.contains("MULTI-TIMEOUT") || field(&out, "MULTI-ERRORS") > 0,
            "{name}: a broken arbiter passed the bench:\n{out}"
        );
    }
}

#[test]
fn the_pin_model_catches_a_controller_that_leaves_the_bus_undriven_or_fights_the_part() {
    if !common::require_tool("iverilog") {
        return;
    }
    let (e, c) = (edge(), thrashing());
    let good = emit_psram_edge_verilog(&e);
    let run = |tag: &str, edge_rtl: String| {
        simulate(
            &workdir(tag),
            &[
                ("psram_cache.v", emit_psram_cache_verilog(&c)),
                ("psram_edge.v", edge_rtl),
                ("psram_model.v", emit_psram_model_verilog(&e.part)),
                (
                    "tb.v",
                    emit_psram_cache_testbench(&e, &c, e.sys_period_ps, 3, 32, 200),
                ),
            ],
        )
    };
    let out = run("pins_good", good.clone());
    assert!(
        !out.contains("PSRAM-VIOLATION "),
        "the edge drives every sampled SIO bit and never fights the part:\n{out}"
    );
    let mutants: [(&str, &str, &str, &str); 2] = [
        (
            "never_drives",
            "sio_oe <= (op == O_SPI) ? 4'b0001 : {4{nx[4]}}; sio_out <= nx[3:0];",
            "sio_oe <= 4'b0000; sio_out <= nx[3:0];",
            "float",
        ),
        (
            "drives_over_read_data",
            "else if (o == O_READ) nibble = 5'b0_0000;",
            "else if (o == O_READ) nibble = 5'b1_0000;",
            "contend",
        ),
    ];
    for (name, from, to, rule) in mutants {
        assert_eq!(
            good.matches(from).count(),
            1,
            "{name}: the mutation site must exist exactly once"
        );
        let out = run(&format!("pins_{name}"), good.replace(from, to));
        assert!(
            out.contains(&format!("PSRAM-VIOLATION {rule}")),
            "{name}: the model must report `{rule}`:\n{out}"
        );
    }
}

// ── 4-5: what the off-chip engine is made of ───────────────────────────────────────────────────────────────

const SYNTH_CAPS: [usize; 10] = [
    WPE_SYNTH_NREG,
    WPE_SYNTH_MAXBLK,
    WPE_SYNTH_MAXCELL,
    WPE_SYNTH_MAXWR,
    WPE_SYNTH_MAXARG,
    WPE_SYNTH_MAXLIV,
    WPE_SYNTH_MAXK,
    WPE_SYNTH_HEAP,
    WPE_SYNTH_DEPTH,
    WPE_SYNTH_MAXCELL,
];

#[test]
fn no_off_chip_text_reaches_an_on_chip_engine() {
    let on_chip = [
        ("plain", emit_wprog_engine_soc_verilog("o")),
        ("hw", emit_wprog_engine_soc_verilog_hw("o")),
        (
            "registers",
            emit_wprog_engine_soc_verilog_backend("o", SYNTH_CAPS, MemBackend::Registers),
        ),
        (
            "bram",
            emit_wprog_engine_soc_verilog_backend("o", SYNTH_CAPS, MemBackend::Bram),
        ),
        (
            "config_bram",
            emit_wprog_engine_soc_verilog_backend("o", SYNTH_CAPS, MemBackend::ConfigBram),
        ),
        (
            "rdw",
            emit_wprog_engine_soc_verilog_backend_rdw("o", SYNTH_CAPS, MemBackend::ConfigBram),
        ),
        (
            "debug",
            emit_wprog_engine_soc_verilog_backend_debug(
                "o",
                SYNTH_CAPS,
                MemBackend::ConfigBram,
                true,
            ),
        ),
        (
            "stream",
            emit_wprog_engine_soc_verilog_backend("o", WPE_STREAM_CAPS, MemBackend::ConfigBram),
        ),
        ("testbench", emit_wprog_engine_soc_testbench("o", 1, 8, 16)),
    ];
    for (name, rtl) in &on_chip {
        for off_chip_text in [
            "mstall",
            "psram",
            "hq_w",
            "sq_w",
            "cq_w",
            "cwrite",
            "__HALF_NS__",
            "OFFCHIP_GLUE",
        ] {
            assert!(
                !rtl.contains(off_chip_text),
                "{name}: the on-chip emission carries `{off_chip_text}`"
            );
        }
        for marker in ["//__ONCHIP_", "//__OFFCHIP_"] {
            assert!(
                !rtl.contains(marker),
                "{name}: a memory marker line `{marker}..` leaked"
            );
        }
    }
    for (name, rtl) in on_chip.iter().filter(|(n, _)| *n != "testbench") {
        for array in ["heap_b0_lo", "sfile_lo", "cell_kind"] {
            assert!(
                rtl.contains(array),
                "{name}: the on-chip engine lost `{array}`"
            );
        }
    }
    let all = memory(true, true, true, shipped());
    let off = emit_wprog_engine_soc_verilog_offchip("o", SYNTH_CAPS, MemBackend::ConfigBram, &all)
        .expect("fits the part");
    for marker in ["_BEGIN__", "_END__", "OFFCHIP_GLUE"] {
        assert!(
            !off.contains(marker),
            "a marker line `{marker}` leaked into the off-chip engine"
        );
    }
    for needed in [
        "module psram_cache",
        "module psram_arbiter",
        "module psram_edge",
        "if (!mstall) case (ph)",
        "rx_valid <= rx_valid & mstall;",
    ] {
        assert!(off.contains(needed), "the off-chip engine lacks `{needed}`");
    }
    let tb = emit_wprog_engine_soc_offchip_testbench("o", 1, 8, 16, &all);
    assert!(
        tb.contains("module psram_model")
            && tb.contains("UART-LOST")
            && !tb.contains("__HALF_NS__")
    );
    assert!(
        tb.contains("always #10 clk = ~clk;"),
        "the board clock runs at the edge's 20 ns period"
    );
}

/// A cache's `busy` is the engine's stall, every register's enable, so it takes the hits the engine registered
/// beside each address and never a tag lookup of the address itself: its combinational inputs are the request
/// strobes and those two hits. MEASURED on the shipped engine at the slow corner, with the lookup inside the stall:
/// at 16 ns the frame cache's index into the stall missed setup by 1.13 ns over 3,611 endpoints (the private tree's
/// RUN_2026.10.20_17.57.25), and at 18.5 ns the config image's, through `c_cellptr + ci`, by 1.71 ns
/// (RUN_2026.10.21_09.00.47).
#[test]
fn a_cache_stalls_from_registered_hits_alone() {
    if !common::require_tool("yosys") {
        return;
    }
    for (name, c) in [("thrashing", thrashing()), ("shipped", shipped()), ("deep", cache(6, 3))] {
        let dir = workdir(&format!("busy_cone_{name}"));
        std::fs::write(dir.join("psram_cache.v"), emit_psram_cache_verilog(&c)).unwrap();
        let ff = "$dff,$sdff,$adff,$dffe,$sdffe,$adffe,$aldff,$aldffe,$sdffce,$dffsr,$dffsre,$mem_v2";
        let script = format!(
            "read_verilog -DSYNTHESIS psram_cache.v; prep -top psram_cache; flatten; \
             select -set cone psram_cache/o:busy %ci*:-{ff}; tee -q -o cone.txt select -list @cone psram_cache/i:* %i"
        );
        let out = Command::new("yosys")
            .args(["-q", "-p", &script])
            .current_dir(&dir)
            .output()
            .expect("run yosys");
        assert!(
            out.status.success(),
            "{name}: yosys failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let mut inputs: Vec<String> = std::fs::read_to_string(dir.join("cone.txt"))
            .expect("the cone listing")
            .lines()
            .filter_map(|l| l.trim().strip_prefix("psram_cache/").map(str::to_string))
            .collect();
        inputs.sort();
        assert_eq!(
            inputs,
            ["req_r", "req_w", "rhit", "whit"],
            "{name}: the stall reaches inputs combinationally that are not the engine's registered strobes and hits"
        );
    }
}

/// The task whose body holds byte `at` of `module`, if any.
fn enclosing_task(module: &str, at: usize) -> Option<&str> {
    let start = module[..at].rfind("\n  task ")? + "\n  task ".len();
    if module[start..at].contains("endtask") {
        return None;
    }
    let name = &module[start..];
    Some(&name[..name.find('(')?])
}

/// The engine-side half of the registered hit: every address a cache serves is set in exactly one task, and that task
/// registers the hit of the address it sets beside it, a lookup of the lines as they will hold after the edge. Nothing
/// else sets one, so no address moves without its hit, and the refresh keeps a held address's hit current across
/// fills and resets.
#[test]
fn every_address_a_cache_serves_is_set_beside_its_hit() {
    use logicaffeine_compile::codegen_sva::wpe_caravel::{
        emit_wpe_user_project_verilog, wpe_caravel_memory, WPE_CARAVEL, WPE_CARAVEL_BACKEND,
        WPE_CARAVEL_CAPS, WPE_CARAVEL_NAME,
    };
    let all = memory(true, true, true, shipped());
    let engines = [
        (
            "synth",
            emit_wprog_engine_soc_verilog_offchip("o", SYNTH_CAPS, MemBackend::ConfigBram, &all)
                .expect("fits the part"),
        ),
        (
            "caravel",
            emit_wpe_user_project_verilog(
                WPE_CARAVEL_NAME,
                WPE_CARAVEL_CAPS,
                WPE_CARAVEL_BACKEND,
                &wpe_caravel_memory(),
                &WPE_CARAVEL,
            )
            .expect("the shipped engine emits"),
        ),
    ];
    let served = [
        ("heap_ra", "hread", "hq_rhit <= hq_hit_at(a);", "hq_rhit <= hq_hit_at(heap_ra);"),
        ("hq_wa", "hwrite", "hq_whit <= hq_hit_at(a);", "hq_whit <= hq_hit_at(hq_wa);"),
        ("sf_ra", "sfread", "sq_rhit <= sq_hit_at(a);", "sq_rhit <= sq_hit_at(sf_ra);"),
        ("sq_wa", "sfwrite", "sq_whit <= sq_hit_at(a);", "sq_whit <= sq_hit_at(sq_wa);"),
        (
            "ci",
            "ci_set",
            "cq_rhit <= cq_hit_at(blk_cell_ptr[pc] + x);",
            "cq_rhit <= cq_hit_at(blk_cell_ptr[pc] + ci);",
        ),
        ("cq_wa", "cwrite", "cq_whit <= cq_hit_at({16'd0, e});", "cq_whit <= cq_hit_at(cq_wa);"),
    ];
    for (engine, rtl) in &engines {
        let module = &rtl[..rtl.find("endmodule").expect("the engine module ends")];
        for (addr, task, beside, refresh) in served {
            let start = module
                .find(&format!("\n  task {task}("))
                .unwrap_or_else(|| panic!("{engine}: no task `{task}`"));
            let body = &module[start..start + module[start..].find("endtask").unwrap()];
            assert_eq!(
                body.matches(beside).count(),
                1,
                "{engine}: `{task}` sets `{addr}` without registering its hit `{beside}`:\n{body}"
            );
            assert_eq!(
                module.matches(refresh).count(),
                1,
                "{engine}: no refresh `{refresh}` keeps a held `{addr}`'s hit current"
            );
            for (at, _) in module.match_indices(&format!("{addr} <=")) {
                if module[..at].ends_with(|ch: char| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.') {
                    continue;
                }
                assert_eq!(
                    enclosing_task(module, at),
                    Some(task),
                    "{engine}: `{addr}` is set outside `{task}`, so its hit lags it: `{}`",
                    module[at..].lines().next().unwrap_or("")
                );
            }
        }
    }
}

/// Every `reg [W:0] name [0:DEPTH]` the engine module (not the appended cache, arbiter, edge or FPU modules)
/// declares.
fn engine_arrays(rtl: &str) -> Vec<String> {
    let engine = &rtl[..rtl.find("endmodule").expect("the engine module ends")];
    engine
        .lines()
        .filter_map(|line| {
            let rest = &line[line.find("reg ")? + 4..];
            let rest = rest
                .strip_prefix("signed ")
                .unwrap_or(rest)
                .strip_prefix('[')?;
            let after = rest[rest.find(":0]")? + 3..].trim();
            let (name, tail) = after.split_at(after.find(|c: char| c.is_whitespace() || c == '[')?);
            tail.trim().starts_with("[0:").then(|| name.to_string())
        })
        .collect()
}

#[test]
fn the_engine_declares_exactly_the_arrays_the_floorplan_keeps_on_the_die() {
    let sky = PdkTarget::by_name("sky130A").expect("sky130A");
    let profile = WpeProfile::from_caps(SYNTH_CAPS);
    for (heap, frames, cells) in [
        (true, false, false),
        (false, true, false),
        (true, true, false),
        (false, false, true),
        (true, true, true),
    ] {
        let m = memory(heap, frames, cells, shipped());
        // The floorplan plans the build that would be taped out: config in registered-read memories.
        let rtl =
            emit_wprog_engine_soc_verilog_offchip("fp", SYNTH_CAPS, MemBackend::ConfigBram, &m)
                .expect("fits the part");
        // `file0`, the block-entry register snapshot, is still declared under the config-in-BRAM backend but
        // never read there, so synthesis removes it and the floorplan rightly charges nothing for it.
        let declared: Vec<String> = engine_arrays(&rtl)
            .into_iter()
            .filter(|a| a != "file0")
            .collect();
        let plan =
            plan_wpe(sky, &profile, &m.roles(), 0).expect("the small profile fits a Caravel die");
        assert_eq!(
            declared.len(),
            plan.placements
                .iter()
                .filter(|p| !matches!(p.residence, Residence::OffChip))
                .count(),
            "{:?}: the engine declares an array the floorplan does not know: {declared:?}",
            m.roles()
        );
        for p in &plan.placements {
            let on_die = !matches!(p.residence, Residence::OffChip);
            assert_eq!(
                declared.iter().any(|d| d == p.array.name),
                on_die,
                "{:?}: `{}` ({:?}) is {} by the floorplan but {} by the RTL",
                m.roles(),
                p.array.name,
                p.array.role,
                if on_die {
                    "kept on the die"
                } else {
                    "sent off-chip"
                },
                if on_die { "missing" } else { "still declared" },
            );
        }
        for off_role in m.roles() {
            assert!(
                plan.placements
                    .iter()
                    .any(|p| p.array.role == off_role && matches!(p.residence, Residence::OffChip)),
                "{off_role:?} is off-chip in the RTL but the floorplan kept it"
            );
        }
    }
    assert_eq!(
        memory(true, true, true, shipped()).roles(),
        vec![
            ArrayRole::Heap,
            ArrayRole::FrameStack,
            ArrayRole::ConfigImage
        ]
    );
}

// ── 6: off-chip ≡ on-chip ≡ model ──────────────────────────────────────────────────────────────────────────

/// One program over the wire: its stream per vector, the model's result per vector, and the model's rendered
/// Show records across the whole stream.
struct Wire {
    bytes: Vec<Vec<u8>>,
    results: Vec<i64>,
    shows: Vec<String>,
}

fn main_program(src: &str) -> Wire {
    let cfg = program_main_fabric(src);
    let run = eval_wprog_config_run(&cfg, &[], &[]);
    let shows = run
        .shows
        .iter()
        .map(|s| render_show(*s, &run.heap))
        .collect::<Vec<_>>();
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "VM error: {:?}", vm.error);
    assert_eq!(
        shows,
        vm.output
            .trim_end()
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>(),
        "the model is not the VM"
    );
    Wire {
        bytes: vec![wprog_config_image(&cfg).wire_bytes(&[])],
        results: vec![run.result],
        shows,
    }
}

/// `sumw(n)` over weights streamed into the heap ahead of the run: every vector rewrites the preload, so
/// from the second on the preload lands on lines the previous run left dirty.
const LOADED_SUM: &str = "## To native loaded () -> Seq of Int\n## To sumw (n: Int) -> Int:\n    Let s be 0.\n    Let i be 0.\n    While i is less than n:\n        Set s to s + item (i + 1) of loaded().\n        Set i to i + 1.\n    Return s.\n";

fn preload_program() -> Wire {
    let cfg = serialize_program(&program_cfg_fabric(LOADED_SUM, "sumw"));
    let blob: Vec<i64> = std::iter::once(40)
        .chain((1..=40).map(|k| 7 * k - 100))
        .collect();
    let mut w = Wire {
        bytes: Vec::new(),
        results: Vec::new(),
        shows: Vec::new(),
    };
    for n in [40i64, 17, 33] {
        let expect: i64 = blob[1..=n as usize].iter().sum();
        assert_eq!(
            eval_wprog_config_counted_heap_preloaded(&cfg, &[n], &blob).0,
            expect,
            "the preload model sums the weights"
        );
        w.bytes
            .push(wprog_config_image(&cfg).wire_bytes_preloaded(&[n], &blob));
        w.results.push(expect);
    }
    w
}

/// Write a program's stream, lengths and oracle beside the testbench.
fn stage(dir: &Path, name: &str, w: &Wire) -> usize {
    let all: Vec<u8> = w.bytes.concat();
    std::fs::write(
        dir.join(format!("wire_{name}.hex")),
        all.iter().map(|b| format!("{b:02x}\n")).collect::<String>(),
    )
    .unwrap();
    std::fs::write(
        dir.join(format!("lens_{name}.hex")),
        w.bytes
            .iter()
            .map(|b| format!("{:08x}\n", b.len()))
            .collect::<String>(),
    )
    .unwrap();
    std::fs::write(
        dir.join(format!("oracle_{name}.hex")),
        w.results
            .iter()
            .map(|r| format!("{:016x}\n", *r as u64))
            .collect::<String>(),
    )
    .unwrap();
    all.len()
}

/// What a run streamed back: every Show record and result frame line, in order.
fn streamed(out: &str) -> Vec<String> {
    out.lines()
        .filter(|l| l.starts_with("SHOW") || l.starts_with("FRAME"))
        .map(str::to_string)
        .collect()
}

/// The records rendered as the host renders them.
fn rendered(out: &str) -> Vec<String> {
    let mut shows = Vec::new();
    let mut text: Option<(usize, Vec<u8>)> = None;
    for l in out.lines() {
        if let Some(rest) = l.strip_prefix("SHOW t=") {
            let (t, v) = rest.split_once(" v=").expect("a SHOW line");
            shows.push(render_show(
                WShow {
                    tag: t.parse().unwrap(),
                    value: u64::from_str_radix(v.trim(), 16).unwrap() as i64,
                },
                &[],
            ));
        } else if let Some(n) = l.strip_prefix("SHOWT len=") {
            match n.trim().parse::<usize>().unwrap() {
                0 => shows.push(String::new()),
                n => text = Some((n, Vec::new())),
            }
        } else if let Some(b) = l.strip_prefix("SHOWB ") {
            let (n, bytes) = text.as_mut().expect("SHOWB outside a text record");
            bytes.push(u8::from_str_radix(b.trim(), 16).unwrap());
            if bytes.len() == *n {
                shows.push(String::from_utf8_lossy(bytes).into_owned());
                text = None;
            }
        }
    }
    shows
}

/// Run `w` on the on-chip engine and on the one with `m`'s memories off-chip; both must pass the oracle and
/// stream the same records and frames (run-cycle counts included), and the off-chip run must break no timing
/// rule of the part and lose no byte. Returns the off-chip run's stdout.
fn on_chip_equals_off_chip(
    tag: &str,
    w: &Wire,
    caps: [usize; 10],
    backend: MemBackend,
    m: &OffchipMemory,
    clks: usize,
) -> String {
    let on_dir = workdir(&format!("{tag}_on"));
    let n = stage(&on_dir, tag, w);
    let on = simulate(
        &on_dir,
        &[
            (
                "engine.v",
                emit_wprog_engine_soc_verilog_backend(tag, caps, backend),
            ),
            (
                "tb.v",
                emit_wprog_engine_soc_testbench(tag, w.bytes.len(), clks, n),
            ),
        ],
    );
    let off_dir = workdir(&format!("{tag}_off"));
    stage(&off_dir, tag, w);
    let off = simulate(
        &off_dir,
        &[
            (
                "engine.v",
                emit_wprog_engine_soc_verilog_offchip(tag, caps, backend, m)
                    .expect("the memories fit the part"),
            ),
            (
                "tb.v",
                emit_wprog_engine_soc_offchip_testbench(tag, w.bytes.len(), clks, n, m),
            ),
        ],
    );
    assert!(
        on.contains("PASS:"),
        "{tag}: the on-chip engine diverged from the model:\n{on}"
    );
    assert!(
        !off.contains("UART-LOST"),
        "{tag}: the off-chip engine lost a received byte:\n{off}"
    );
    assert!(
        !off.contains("PSRAM-CACHE-HIT-MISMATCH"),
        "{tag}: a hit the engine registered beside an address disagreed with its cache's lines:\n{off}"
    );
    assert!(
        off.contains("PASS:"),
        "{tag}: the off-chip engine diverged from the model:\n{off}"
    );
    assert_eq!(
        field(&off, "PSRAM-VIOLATIONS"),
        0,
        "{tag}: the edge broke a timing rule of the part"
    );
    assert_eq!(
        streamed(&off),
        streamed(&on),
        "{tag}: the off-chip engine streamed something other than the on-chip one"
    );
    assert_eq!(
        rendered(&off),
        w.shows,
        "{tag}: the off-chip engine's records are not the model's"
    );
    off
}

/// Programs whose output the VM and the model agree on, covering every heap cell kind the engine has (lists,
/// deep copy, text concat, char-at, equality, Show of text) and recursion through the frame stack.
const MAIN_CORPUS: &[(&str, &str)] = &[
    ("list", "## Main\n    Let xs be [3, 1, 2].\n    Show length of xs.\n    Show item 2 of xs.\n"),
    (
        "text_ops",
        "## Main\n    Let s be \"abc\".\n    Show item 2 of s.\n    Show s equals \"abc\".\n    Show s equals \"abd\".\n    Show \"\".\n    Show length of s.\n",
    ),
    ("text_concat", "## Main\n    Let name be \"LOGOS\".\n    Show name + \" on silicon\".\n    Show 2 + 3.\n"),
    ("text_int", "## Main\n    Show \"n=\" + 42.\n    Show \"neg\" + (0 - 7).\n    Show \"zero\" + 0.\n    Show 1000000 + \"!\".\n"),
    (
        "recursion",
        "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n## Main\n    Show fib(11).\n    Show fib(6).\n",
    ),
    (
        "bubble_sort",
        "## Main\n    Let mutable xs be [9, 4, 7, 1, 8, 2, 6, 3, 5, 0, 11, 10].\n    Let n be length of xs.\n    Let mutable i be 1.\n    While i is less than n:\n        Let mutable j be 1.\n        While j is at most n - i:\n            If item j of xs is greater than item (j + 1) of xs:\n                Let t be item j of xs.\n                Set item j of xs to item (j + 1) of xs.\n                Set item (j + 1) of xs to t.\n            Set j to j + 1.\n        Set i to i + 1.\n    Let mutable k be 1.\n    While k is at most n:\n        Show item k of xs.\n        Set k to k + 1.\n",
    ),
];

/// A profile every corpus program fits, with each memory far past a two-line cache.
const MAIN_CAPS: [usize; 10] = [64, 32, 160, 128, 16, 64, 64, 256, 24, 160];

/// Every corpus program and the streamed preload, on the engine with `m`'s memories off-chip. A program
/// that never touches an off-chip memory — a call-free one when only the frame stack is off — never waits
/// on the part, so the stall is proven where it happens: the corpus must wait, and with the frame stack
/// off-chip the recursive program must.
fn corpus_on(label: &str, m: &OffchipMemory, backend: MemBackend) {
    let clks = m.min_clks_per_bit() as usize;
    m.check_uart(clks as u64)
        .expect("the UART the lock runs at is one no loading stall can span");
    let mut stalls = Vec::new();
    for (name, src) in MAIN_CORPUS {
        let out = on_chip_equals_off_chip(
            &format!("{label}_{name}"),
            &main_program(src),
            MAIN_CAPS,
            backend,
            m,
            clks,
        );
        stalls.push((*name, field(&out, "PSRAM-STALL-CYCLES")));
    }
    let out = on_chip_equals_off_chip(
        &format!("{label}_preload"),
        &preload_program(),
        SYNTH_CAPS,
        backend,
        m,
        clks,
    );
    stalls.push(("preload", field(&out, "PSRAM-STALL-CYCLES")));
    assert!(
        stalls.iter().any(|(_, s)| *s > 0),
        "{label}: no program waited on the part, so none proves the stall: {stalls:?}"
    );
    if m.frames.is_some() {
        let recursion = stalls
            .iter()
            .find(|(n, _)| *n == "recursion")
            .expect("the corpus recurses")
            .1;
        assert!(
            recursion > 0,
            "{label}: recursion never waited on an off-chip frame stack"
        );
    }
}

#[test]
fn the_heap_off_chip_runs_every_program_as_on_chip() {
    if !common::require_tool("iverilog") {
        return;
    }
    corpus_on(
        "heap_thrash",
        &memory(true, false, false, thrashing()),
        MemBackend::Registers,
    );
    corpus_on(
        "heap_shipped",
        &memory(true, false, false, shipped()),
        MemBackend::Registers,
    );
}

#[test]
fn the_frame_stack_off_chip_runs_every_program_as_on_chip() {
    if !common::require_tool("iverilog") {
        return;
    }
    corpus_on(
        "frames_shipped",
        &memory(false, true, false, shipped()),
        MemBackend::Registers,
    );
    corpus_on(
        "heap_frames_thrash",
        &memory(true, true, false, thrashing()),
        MemBackend::Registers,
    );
}

#[test]
fn the_config_image_off_chip_runs_every_program_as_on_chip() {
    if !common::require_tool("iverilog") {
        return;
    }
    corpus_on(
        "cells_thrash",
        &memory(false, false, true, thrashing()),
        MemBackend::ConfigBram,
    );
    corpus_on(
        "all_shipped",
        &memory(true, true, true, shipped()),
        MemBackend::ConfigBram,
    );
}

// ── 7: the UART bound ──────────────────────────────────────────────────────────────────────────────────────

#[test]
fn check_uart_accepts_exactly_the_byte_times_no_loading_stall_can_span() {
    for m in [
        memory(true, false, false, thrashing()),
        memory(true, true, true, shipped()),
        memory(false, true, false, cache(1, 1)),
        memory(true, false, true, cache(4, 2)),
    ] {
        let min = m.min_clks_per_bit();
        assert!(
            m.check_uart(min).is_ok(),
            "{m:?}: the minimum it names must pass"
        );
        assert!(uart_byte_cycles(min) > m.loading_stall_cycles());
        assert!(
            matches!(
                m.check_uart(min - 1),
                Err(OffchipUnplaceable::StallOutlastsByte { .. })
            ) || m.loading_stall_cycles() == 0,
            "{m:?}: one cycle per bit faster lets a loading stall span a byte"
        );
    }
    // The frame stack is never touched while bytes arrive: with only it off-chip, no loading stall exists.
    assert_eq!(
        memory(false, true, false, thrashing()).loading_stall_cycles(),
        0
    );
    let e = edge();
    assert_eq!(
        memory(true, false, true, thrashing()).loading_stall_cycles(),
        thrashing().miss_stall_cycles(&e)
    );
    // At 115200 baud on the 50 MHz engine clock a byte is 4340 cycles, seven loading stalls long.
    assert!(memory(true, true, true, shipped()).check_uart(434).is_ok());
    assert_eq!(
        wire_bytes_before_first_part_access(),
        24 * 2 + 2 + 1,
        "24 empty config memories, an empty parameter list, go"
    );
}

#[test]
fn a_received_byte_waits_out_a_stall_and_only_a_stall_spanning_two_bytes_loses_one() {
    if !common::require_tool("iverilog") {
        return;
    }
    let m = memory(true, false, false, thrashing());
    let w = preload_program();
    let miss = m.loading_stall_cycles();
    // A UART whose byte is shorter than one miss but longer than half of it: during a preload each dirty
    // miss spans exactly one incoming byte, which the receiver holds.
    let held = (miss / 10) as usize - 5;
    assert!(uart_byte_cycles(held as u64) < miss && 2 * uart_byte_cycles(held as u64) > miss);
    assert!(
        m.check_uart(held as u64).is_err(),
        "check_uart is conservative: it refuses a UART the hold still carries"
    );
    on_chip_equals_off_chip("held_byte", &w, SYNTH_CAPS, MemBackend::Registers, &m, held);

    // Without the hold, that byte is gone.
    let dir = workdir("held_byte_removed");
    let n = stage(&dir, "nohold", &w);
    let engine =
        emit_wprog_engine_soc_verilog_offchip("nohold", SYNTH_CAPS, MemBackend::Registers, &m)
            .unwrap();
    let hold = "rx_valid <= rx_valid & mstall;";
    assert_eq!(engine.matches(hold).count(), 1);
    let out = simulate(
        &dir,
        &[
            ("engine.v", engine.replace(hold, "rx_valid <= 1'b0;")),
            (
                "tb.v",
                emit_wprog_engine_soc_offchip_testbench("nohold", w.bytes.len(), held, n, &m),
            ),
        ],
    );
    assert!(
        out.contains("UART-LOST"),
        "with no hold a byte that arrives mid-stall must be lost:\n{out}"
    );

    // A UART so fast that one miss spans two bytes overwrites the held one.
    let too_fast = (miss / 20) as usize - 2;
    let dir = workdir("two_bytes_per_stall");
    let n = stage(&dir, "fast", &w);
    let out = simulate(
        &dir,
        &[
            (
                "engine.v",
                emit_wprog_engine_soc_verilog_offchip(
                    "fast",
                    SYNTH_CAPS,
                    MemBackend::Registers,
                    &m,
                )
                .unwrap(),
            ),
            (
                "tb.v",
                emit_wprog_engine_soc_offchip_testbench("fast", w.bytes.len(), too_fast, n, &m),
            ),
        ],
    );
    assert!(
        out.contains("UART-LOST"),
        "a stall spanning two bytes must lose one:\n{out}"
    );
}

#[test]
fn an_edge_timed_for_a_clock_range_meets_every_rule_across_it() {
    // Timed for 20 ns and run down to 40 ns: the cycle counts for tPU, tRST and tCPH come from the fast end,
    // the burst length from the slow end, where a read of n words holds CE# low for (14 + 16n) x 80 ns.
    let e = edge()
        .clocked_down_to(40_000)
        .expect("a 40 ns clock still takes one-word bursts");
    assert_eq!(e.max_words(false), 5);
    assert!(
        matches!(
            PsramCache::new(&e, 2, 3),
            Err(PsramUnplaceable::LineBurst {
                line_words: 8,
                max_words: 5
            })
        ),
        "an eight-word line holds CE# past tCEM at 40 ns"
    );
    assert!(matches!(
        edge().clocked_down_to(19_000),
        Err(PsramUnplaceable::SlowestFasterThanFastest { .. })
    ));
    assert!(matches!(
        edge().clocked_down_to(400_000),
        Err(PsramUnplaceable::NoBurstAtSlowestClock { .. })
    ));
    if !common::require_tool("iverilog") {
        return;
    }
    let c = PsramCache::new(&e, 2, 2).expect("four-word lines fit tCEM at 40 ns");
    for (tag, period) in [
        ("range_fast_end", e.sys_period_ps),
        ("range_slow_end", e.slowest_period_ps),
    ] {
        let out = simulate(
            &workdir(tag),
            &[
                ("psram_cache.v", emit_psram_cache_verilog(&c)),
                ("psram_edge.v", emit_psram_edge_verilog(&e)),
                ("psram_model.v", emit_psram_model_verilog(&e.part)),
                (
                    "tb.v",
                    emit_psram_cache_testbench(&e, &c, period, 9, 128, 600),
                ),
            ],
        );
        assert!(
            out.contains("CACHE-DONE"),
            "{tag}: the bench did not finish:\n{out}"
        );
        assert_eq!(field(&out, "CACHE-ERRORS"), 0, "{tag}");
        assert_eq!(
            field(&out, "PSRAM-VIOLATIONS"),
            0,
            "{tag}: a rule of the part broke inside the edge's clock range"
        );
    }
    // The line the fast clock alone would allow, run at the slow end, is exactly what tCEM forbids.
    let fast_only = PsramCache::new(&edge(), 2, 3).expect("eight-word lines fit at 20 ns");
    let out = simulate(
        &workdir("range_slow_end_long_lines"),
        &[
            ("psram_cache.v", emit_psram_cache_verilog(&fast_only)),
            ("psram_edge.v", emit_psram_edge_verilog(&edge())),
            ("psram_model.v", emit_psram_model_verilog(&e.part)),
            (
                "tb.v",
                emit_psram_cache_testbench(&edge(), &fast_only, 40_000, 9, 128, 600),
            ),
        ],
    );
    assert!(
        out.contains("PSRAM-VIOLATION tCEM"),
        "eight-word bursts at 40 ns must hold CE# past tCEM:\n{out}"
    );
}

// ── 9: refusals ────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn every_unbuildable_off_chip_memory_is_refused_by_name() {
    let e = edge();
    assert!(matches!(
        PsramCache::new(&e, 0, 3),
        Err(PsramUnplaceable::CacheShape { .. })
    ));
    assert!(matches!(
        PsramCache::new(&e, 4, 0),
        Err(PsramUnplaceable::CacheShape { .. })
    ));
    assert!(matches!(
        PsramCache::new(&e, 12, 8),
        Err(PsramUnplaceable::CacheWiderThanAddress { .. })
    ));
    // At a 40 ns SCLK a read burst of n words holds CE# low for (14 + 16n) edges: eleven words fit tCEM's 8 µs.
    assert_eq!(e.max_words(false), 11);
    assert!(matches!(
        PsramCache::new(&e, 4, 4),
        Err(PsramUnplaceable::LineBurst {
            line_words: 16,
            max_words: 11
        })
    ));
    assert!(
        PsramCache::new(&e, 4, 3).is_ok(),
        "an eight-word line is one legal burst"
    );

    let none = memory(false, false, false, shipped());
    assert_eq!(
        emit_wprog_engine_soc_verilog_offchip("x", SYNTH_CAPS, MemBackend::Registers, &none),
        Err(OffchipUnplaceable::NothingOffChip)
    );
    for backend in [MemBackend::Registers, MemBackend::Bram] {
        assert_eq!(
            emit_wprog_engine_soc_verilog_offchip(
                "x",
                SYNTH_CAPS,
                backend,
                &memory(false, false, true, shipped())
            ),
            Err(OffchipUnplaceable::CellsNeedConfigBram { backend })
        );
    }

    let part_words = 8u64 << 20 >> 3;
    let mut big = SYNTH_CAPS;
    big[7] = part_words as usize;
    assert!(emit_wprog_engine_soc_verilog_offchip(
        "fits",
        big,
        MemBackend::Registers,
        &memory(true, false, false, shipped())
    )
    .is_ok());
    assert!(
        matches!(
            emit_wprog_engine_soc_verilog_offchip(
                "big",
                big,
                MemBackend::Registers,
                &memory(true, true, false, shipped())
            ),
            Err(OffchipUnplaceable::RegionsLargerThanPart { .. })
        ),
        "a whole-part heap leaves no room for the frame stack"
    );

    // A part that takes 4 ms to power up is still initializing when a UART of 100 cycles a bit, whose first
    // 51 bytes take 1.02 ms, reaches it.
    let slow = PsramPart {
        t_pu_min_ps: 4_000_000_000,
        ..APS6404L_3SQN
    };
    let slow_edge = PsramEdge::new(slow, SYS_PS, 1).unwrap();
    let m = OffchipMemory {
        edge: slow_edge,
        heap: Some(PsramCache::new(&slow_edge, 6, 3).unwrap()),
        frames: None,
        cells: None,
    };
    assert!(
        m.loading_stall_cycles() < uart_byte_cycles(100),
        "the stall bound alone would accept 100 cycles a bit"
    );
    assert!(matches!(
        m.check_uart(100),
        Err(OffchipUnplaceable::InitOutlastsLoad { .. })
    ));
    assert!(m.check_uart(m.min_clks_per_bit()).is_ok());
}

#[test]
fn every_memory_gets_its_own_aligned_region() {
    let caps = [208, 48, 1072, 464, 32, 304, 112, 21_056, 32, 528];
    let m = memory(true, true, true, shipped());
    let r = m.regions(caps);
    assert_eq!(r.len(), 5, "the heap, the frame stack and three cell lanes");
    for x in &r {
        assert_eq!(
            x.base % (1 << x.bits),
            0,
            "`{}` is aligned to its own size",
            x.name
        );
    }
    for (i, a) in r.iter().enumerate() {
        for b in &r[i + 1..] {
            assert!(
                a.base + (1 << a.bits) <= b.base || b.base + (1 << b.bits) <= a.base,
                "`{}` and `{}` overlap",
                a.name,
                b.name
            );
        }
    }
    let size = |name: &str| 1usize << r.iter().find(|x| x.name == name).unwrap().bits;
    assert!(size("heap") >= caps[7] && size("heap") < 2 * caps[7]);
    assert!(size("frames") >= caps[8] * caps[0] && size("cell_lane0") >= caps[2]);
}
