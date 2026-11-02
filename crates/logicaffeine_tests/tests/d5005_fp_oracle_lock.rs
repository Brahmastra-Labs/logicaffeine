//! The hard floating-point oracle (`codegen_sva::fp_oracle`) under iverilog, its core beside a stand-in for the D5005's
//! floating-point DSP block whose semantics and latency are known (`fp_oracle::hard_model_verilog`): the oracle counts
//! no difference for the block's class and some for every other, only at the tap that is the block's latency; a
//! recording run stores every vector with the block's results; a step no class explains is counted and stored; and the
//! streams take every triple once, the near one reaching where the semantics part.

mod common;

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Command;

use logicaffeine_compile::codegen_sva::fp_hard::{Block, Measurement, Rounding};
use logicaffeine_compile::codegen_sva::fp_hypotheses::Semantics;
use logicaffeine_compile::codegen_sva::fp_oracle::{hard_model_verilog, persona_sources, Depths, Oracle, Run, Source, Store};

fn oracle() -> Oracle {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/d5005/fp_discriminators.json");
    Oracle::from_ledger(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

/// A run for the simulation: its word, the triples it takes, and how many records of its store to read back.
struct Step {
    run: Run,
    steps: u32,
    records: usize,
}

const TB: &str = r#"`timescale 1ns/1ps
module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  reg rst = 1, start = 0, host_wr = 0;
  reg [31:0] job_at = 0, job_end = 0, host_raddr = 0;
  reg [511:0] host_wdata = 0;
  wire busy, fault, mem_rd, mem_wr;
  wire [31:0] pc, mem_rd_line, mem_wr_line;
  wire [511:0] host_rdata, mem_wr_data;
  wire [63:0] mem_wr_mask;
  logos_fp_oracle u(.clk(clk), .step_en(1'b1), .read_en(1'b1), .rst(rst), .start(start), .job_at(job_at), .job_end(job_end),
    .rows(16'd0), .state_f16(1'b0), .cache_int8(1'b0), .two_words(1'b0), .per_block(1'b0), .busy(busy), .fault(fault), .pc(pc),
    .host_wr(host_wr), .host_table(3'd0), .host_addr(32'd0), .host_wdata(host_wdata), .host_raddr(host_raddr),
    .host_rdata(host_rdata), .mem_rd(mem_rd), .mem_rd_line(mem_rd_line), .mem_rd_valid(1'b0), .mem_rd_data(512'd0),
    .mem_wr(mem_wr), .mem_wr_line(mem_wr_line), .mem_wr_data(mem_wr_data), .mem_wr_mask(mem_wr_mask), .mem_wr_ready(1'b1));
  reg [95:0] runs [0:@LAST@];
  integer r, b, fd;
  initial begin
    $readmemh("runs.hex", runs);
    fd = $fopen("out.hex", "w");
    @(negedge clk); @(negedge clk); rst = 0;
    host_wdata = 512'h@SEED@; host_wr = 1; @(negedge clk); host_wr = 0;
    for (r = 0; r <= @LAST@; r = r + 1) begin
      job_at = runs[r][31:0]; job_end = runs[r][63:32]; start = 1; @(negedge clk); start = 0;
      @(negedge clk);
      while (busy) @(negedge clk);
      if (fault) $display("FAULT in run %0d", r);
      for (b = 0; b < runs[r][95:64]; b = b + 1) begin
        host_raddr = b; @(negedge clk); @(negedge clk); $fwrite(fd, "%h\n", host_rdata);
      end
    end
    $fclose(fd);
    $finish;
  end
endmodule
"#;

/// The core beside a block computing `product` and `sum`, `full` steps deep fully registered (multiply, add,
/// multiply-add) and one step with its output register alone, the stream seeded `seed`, through `steps` one after
/// another: each run's store as read back.
fn simulate(oracle: &Oracle, product: &Semantics, sum: &Semantics, full: [usize; 3], seed: u64, steps: &[Step]) -> Vec<Store> {
    simulate_deep(oracle, product, sum, Depths { full, least: [1; 2] }, seed, steps)
}

/// [`simulate`] with the least-registered blocks as deep as `depths` says.
fn simulate_deep(oracle: &Oracle, product: &Semantics, sum: &Semantics, depths: Depths, seed: u64, steps: &[Step]) -> Vec<Store> {
    let dir = tempfile::tempdir().unwrap();
    let verilog = format!("{}\n{}", oracle.core_verilog(), hard_model_verilog(product, sum, depths));
    std::fs::write(dir.path().join("oracle.v"), verilog).unwrap();
    let beats = |s: &Step| oracle.records_at() + s.records;
    let runs: String = steps.iter().map(|s| format!("{:08x}{:08x}{:08x}\n", beats(s), s.steps, s.run.word())).collect();
    std::fs::write(dir.path().join("runs.hex"), runs).unwrap();
    let tb = TB.replace("@LAST@", &(steps.len() - 1).to_string()).replace("@SEED@", &format!("{seed:x}"));
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = Command::new("iverilog").current_dir(dir.path()).args(["-g2012", "-s", "tb", "-o", "sim", "tb.v", "oracle.v"]).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir.path()).arg("sim").output().expect("vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(run.status.success() && !stdout.contains("FAULT"), "vvp: {stdout}{}", String::from_utf8_lossy(&run.stderr));
    let out = std::fs::read_to_string(dir.path().join("out.hex")).unwrap();
    let beat = |l: &str| -> [u64; 8] {
        assert_eq!(l.len(), 128, "a beat of 512 bits: {l}");
        let mut w = [0u64; 8];
        for (k, word) in w.iter_mut().enumerate() {
            *word = u64::from_str_radix(&l[128 - 16 * (k + 1)..128 - 16 * k], 16).unwrap_or_else(|_| panic!("a written beat with unknown bits: {l}"));
        }
        w
    };
    let mut lines = out.lines();
    steps
        .iter()
        .enumerate()
        .map(|(k, s)| {
            let run: Vec<&str> = lines.by_ref().take(beats(s)).collect();
            assert_eq!(run.len(), beats(s), "run {k}: every beat read back");
            // The beats past the records the header counts were never written.
            let written = oracle.records_at() + beat(run[0])[4] as usize;
            let run: Vec<[u64; 8]> = run[..written.min(run.len())].iter().map(|l| beat(l)).collect();
            Store::read(oracle, &run).unwrap_or_else(|| panic!("run {k}: not the oracle's store: {:x?}", run.first()))
        })
        .collect()
}

fn counting(source: Source, tap: u8, steps: u32) -> Step {
    Step { run: Run { source, record: false, tap, delays: [0; 3] }, steps, records: 0 }
}

/// The class of `s` among `classes`, by its members.
fn class_of(classes: &[logicaffeine_compile::codegen_sva::fp_oracle::Class], s: &Semantics) -> usize {
    classes.iter().position(|c| c.members.contains(&s.name())).unwrap_or_else(|| panic!("no class holds {}", s.name()))
}

/// Every class but `own` differs somewhere, `own` nowhere.
fn names(counters: &[u64], own: usize, what: &str) {
    assert_eq!(counters[own], 0, "{what}: the block's own class never differs");
    let zero: Vec<usize> = (0..counters.len()).filter(|&k| k != own && counters[k] == 0).collect();
    assert!(zero.is_empty(), "{what}: classes {zero:?} agree with the block on every vector, yet are not its");
}

#[test]
fn the_oracle_counts_no_difference_for_the_class_of_the_block_beside_it_and_some_for_every_other() {
    if !common::require_tool("iverilog") {
        return;
    }
    let o = oracle();
    let n = o.vectors.len() as u32;
    let [store] = &simulate(&o, &Semantics::IEEE, &Semantics::IEEE, [4; 3], 1, &[counting(Source::Vectors, 3, n)])[..] else { unreachable!() };
    assert_eq!(store.header.steps, u64::from(n), "every vector compared once");
    assert_eq!((store.header.lone_products, store.header.lone_sums, store.header.records), (0, 0, 0), "the block's own class explains every step");
    names(&store.products, class_of(&o.products, &Semantics::IEEE), "product");
    names(&store.sums, class_of(&o.sums, &Semantics::IEEE), "sum");
}

#[test]
fn the_engines_own_arithmetic_is_named_at_its_own_latency() {
    if !common::require_tool("iverilog") {
        return;
    }
    let o = oracle();
    let n = o.vectors.len() as u32;
    let [store] = &simulate(&o, &Semantics::ENGINE, &Semantics::ENGINE, [7; 3], 1, &[counting(Source::Vectors, 6, n)])[..] else { unreachable!() };
    assert_eq!(store.header.steps, u64::from(n));
    names(&store.products, class_of(&o.products, &Semantics::ENGINE), "product");
    names(&store.sums, class_of(&o.sums, &Semantics::ENGINE), "sum");
}

#[test]
fn only_the_blocks_own_latency_lets_a_class_agree_with_every_vector_and_a_step_nothing_explains_is_stored() {
    if !common::require_tool("iverilog") {
        return;
    }
    let o = oracle();
    let n = o.vectors.len() as u32;
    let whole = o.record_cap();
    let steps: Vec<Step> = (0..16).map(|tap| Step { run: Run { source: Source::Vectors, record: false, tap, delays: [0; 3] }, steps: n, records: whole }).collect();
    let stores = simulate(&o, &Semantics::IEEE, &Semantics::IEEE, [4; 3], 1, &steps);
    for (tap, store) in stores.iter().enumerate() {
        let agreeing = store.products.iter().min() == Some(&0) && store.sums.iter().min() == Some(&0);
        assert_eq!(agreeing, tap == 3, "tap {tap}: a class agrees with every vector exactly at the latency, 4 steps");
        if tap == 3 {
            continue;
        }
        assert!(store.header.lone_products > 0 && store.header.lone_sums > 0, "tap {tap}: results of other operands explain nothing");
        let (one, either) = (store.header.lone_products.max(store.header.lone_sums), store.header.lone_products + store.header.lone_sums);
        let cap = o.record_cap() as u64;
        assert!((one.min(cap)..=either.min(cap)).contains(&store.header.records), "tap {tap}: a record for each step nothing explains, up to the cap");
        assert!(store.header.records >= 32, "tap {tap}: the steps nothing explains are stored");
        for r in &store.records {
            assert!(r.products_agree.iter().all(|&a| !a) || r.sums_agree.iter().all(|&a| !a), "tap {tap}: a stored step one of whose results nothing explains");
        }
    }
}

#[test]
fn a_recording_run_stores_every_vector_with_the_blocks_three_results() {
    if !common::require_tool("iverilog") {
        return;
    }
    let o = oracle();
    let n = o.vectors.len();
    let step = Step { run: Run { source: Source::Vectors, record: true, tap: 3, delays: [0; 3] }, steps: n as u32, records: n };
    let [store] = &simulate(&o, &Semantics::IEEE, &Semantics::IEEE, [4; 3], 1, &[step])[..] else { unreachable!() };
    assert_eq!(store.header.records, n as u64, "a record a step");
    let (p, s) = (class_of(&o.products, &Semantics::IEEE), class_of(&o.sums, &Semantics::IEEE));
    for (k, (r, &[a, b, c])) in store.records.iter().zip(&o.vectors).enumerate() {
        assert_eq!((r.a, r.b, r.c), (a, b, c), "record {k}: the vector's operands");
        assert!(r.products_agree[p] && r.sums_agree[s], "record {k}: the block's class agrees with it");
        assert_eq!(r.flags, [0; 3], "record {k}: the stand-in raises no flag");
    }
    let fma_at = n - 64;
    for (k, r) in store.records[fma_at..].iter().enumerate() {
        let (x, y, z) = (f32::from_bits(r.a), f32::from_bits(r.b), f32::from_bits(r.c));
        assert_eq!(r.product, (x * y).to_bits(), "multiply-add vector {k}: the product");
        assert_eq!(r.fma, (x * y + z).to_bits(), "multiply-add vector {k}: the stand-in rounds twice");
        if k % 4 != 3 {
            assert_eq!(r.fma, 0, "multiply-add vector {k}: twice-rounded, the product's error is lost");
        }
    }
}

#[test]
fn the_streams_take_every_triple_once_from_the_seed_and_the_near_one_reaches_where_the_semantics_part() {
    if !common::require_tool("iverilog") {
        return;
    }
    let o = oracle();
    let steps = [counting(Source::Random, 3, 3000), counting(Source::Near, 3, 3000), counting(Source::Random, 3, 3000)];
    let a = simulate(&o, &Semantics::IEEE, &Semantics::IEEE, [4; 3], 0x1234_5678_9abc_def1, &steps);
    let (p, s, e) = (class_of(&o.products, &Semantics::IEEE), class_of(&o.sums, &Semantics::IEEE), class_of(&o.products, &Semantics::ENGINE));
    for store in &a {
        assert_eq!(store.header.steps, 3000);
        assert_eq!((store.products[p], store.sums[s], store.header.lone_products, store.header.lone_sums), (0, 0, 0, 0));
    }
    assert_eq!(a[0].products, a[2].products, "a stream is its seed's");
    assert!(
        a[1].products[e] > 4 * a[0].products[e].max(1) && a[1].products[e] > 3000 / 4,
        "the near stream lands where flushing tells products apart, a quarter of its steps or more: {} near, {} random",
        a[1].products[e],
        a[0].products[e]
    );
    let es = class_of(&o.sums, &Semantics::ENGINE);
    assert!(a[1].sums[es] > a[0].sums[es], "and sums: {} near, {} random", a[1].sums[es], a[0].sums[es]);
    let b = simulate(&o, &Semantics::IEEE, &Semantics::IEEE, [4; 3], 0x0fed_cba9_8765_4321, &steps[..1]);
    assert_ne!(a[0].products, b[0].products, "another seed, another stream");
}

#[test]
fn the_persona_is_the_oracle_behind_the_engines_crossing() {
    let o = oracle();
    let engine = o.engine_verilog();
    let mut want = String::new();
    writeln!(want, "  logos_fp_oracle core(.clk(fclk), .step_en(fedge), .read_en(cr),").unwrap();
    assert!(engine.contains(want.trim_end()), "the core is the instance the engine build's timing constraints name");
    assert!(engine.contains("logos_engine_bridge bridge(.sclk(clk), .fclk(fclk), .frst(frst), .fedge(fedge), .fmid(fmid),"));
}

/// The persona's tree (`fp_oracle::persona_sources`) under `scripts/d5005/run-cosim.sh`: the shell, the engine's crossing
/// at the compute clock the build divides, the oracle beside a stand-in block as deep in each mode as no other (three,
/// six and five registers), and the measurement the card runs (`sw/logos_fp_oracle.c`) through the simulation's host.
/// The measurement finds each mode's latency, the multiply's and the add's as the one tap at which their classes agree
/// with every vector and the multiply-add's from its results with C zero, the aligned vectors name exactly the
/// stand-in's classes, and both streams run whole.
#[test]
fn the_persona_and_its_measurement_name_the_block_through_the_shell() {
    if !common::require_tool("iverilog") || !common::require_tool("iverilog-vpi") || !common::require_tool("vvp") {
        return;
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tree = tempfile::tempdir().unwrap();
    let model = Semantics::ENGINE;
    let o = oracle();
    for f in persona_sources(&o, 16, &model, Depths { full: [3, 6, 5], least: [2, 1] }) {
        let path = tree.path().join(f.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, f.text).unwrap();
    }
    let run = Command::new("bash")
        .arg(root.join("scripts/d5005/run-cosim.sh"))
        .arg(tree.path())
        .args(["fp_oracle", "300", "--seed", "77"])
        .env("TMPDIR", std::env::temp_dir())
        .output()
        .expect("bash");
    let out = String::from_utf8_lossy(&run.stdout);
    assert_eq!(run.status.code(), Some(0), "the measurement against the RTL:\n{out}{}", String::from_utf8_lossy(&run.stderr));
    let text = &out[out.find('{').unwrap()..=out.rfind('}').unwrap()];
    let json: serde_json::Value = serde_json::from_str(text).unwrap();
    let measured = Measurement::read(&o, text).unwrap_or_else(|e| panic!("the measurement names a block: {e}"));
    assert_eq!(measured.product, Block { semantics: model, full: 3, least: 2 }, "the stand-in's multiply");
    assert_eq!(measured.sum, Block { semantics: model, full: 6, least: 1 }, "and its add");
    assert_eq!((measured.multiply_add_latency, measured.multiply_add), (5, Rounding::Twice), "its multiply-add rounds twice, five deep");
    assert_eq!(json["complete"], true);
    assert_eq!(json["seed"], 77);
    let latency = &json["latency"];
    assert_eq!((latency["product"].as_u64(), latency["sum"].as_u64(), latency["multiply_add"].as_u64()), (Some(3), Some(6), Some(5)), "each block's own depth: {latency}");
    assert_eq!((latency["product_least"].as_u64(), latency["sum_least"].as_u64()), (Some(2), Some(1)), "and each least-registered block's: {latency}");
    for t in json["taps"].as_array().unwrap() {
        assert_eq!(t["least_product"] == 0, t["tap"] == 2, "the product's classes meet the multiply at its depth alone: {t}");
        assert_eq!(t["least_sum"] == 0, t["tap"] == 5, "the sum's the add at its: {t}");
    }
    assert_eq!(json["run_word"], Run::aligned(Source::Vectors, false, [3, 6, 5]).unwrap().word(), "the aligned runs at the deepest block's tap");
    let zero = |run: &serde_json::Value, key: &str| -> Vec<usize> {
        run[key].as_array().unwrap().iter().enumerate().filter(|(_, c)| c.as_u64() == Some(0)).map(|(k, _)| k).collect()
    };
    let vectors = &json["runs"]["vectors"];
    assert_eq!(vectors["steps"], o.vectors.len());
    assert_eq!(zero(vectors, "products"), [class_of(&o.products, &model)], "the vectors name the block's product class alone");
    assert_eq!(zero(vectors, "sums"), [class_of(&o.sums, &model)], "and its sum class alone");
    let records = json["runs"]["records"].as_array().unwrap();
    assert_eq!(records.len(), o.vectors.len(), "a record a vector");
    for (k, (r, v)) in records.iter().zip(&o.vectors).enumerate() {
        let word = |i: usize| u32::from_str_radix(r[i].as_str().unwrap(), 16).unwrap();
        assert_eq!([word(0), word(1), word(2)], *v, "record {k}: its vector's operands");
    }
    for stream in ["random", "near"] {
        let r = &json["runs"][stream];
        assert_eq!((r["steps"].as_u64(), r["lone_products"].as_u64(), r["lone_sums"].as_u64()), (Some(300), Some(0), Some(0)), "{stream}: {r}");
        assert!(zero(r, "products").contains(&class_of(&o.products, &model)) && zero(r, "sums").contains(&class_of(&o.sums, &model)), "{stream}");
    }
}

/// A recording run stores every step, so its header's counts are its records': the steps no product class explains
/// are the records no product class agrees with, the sums' likewise, and each class's counter its records'
/// disagreements.
#[test]
fn a_recording_runs_counts_are_its_records() {
    if !common::require_tool("iverilog") {
        return;
    }
    let o = oracle();
    let n = o.vectors.len();
    let step = Step { run: Run { source: Source::Vectors, record: true, tap: 2, delays: [0; 3] }, steps: n as u32, records: n };
    let [store] = &simulate(&o, &Semantics::IEEE, &Semantics::IEEE, [4; 3], 1, &[step])[..] else { unreachable!() };
    assert_eq!(store.records.len(), n);
    let lone = |agree: fn(&logicaffeine_compile::codegen_sva::fp_oracle::Record) -> &Vec<bool>| store.records.iter().filter(|r| agree(r).iter().all(|&a| !a)).count() as u64;
    assert_eq!(store.header.lone_products, lone(|r| &r.products_agree), "the steps no product class explains");
    assert_eq!(store.header.lone_sums, lone(|r| &r.sums_agree), "the steps no sum class explains");
    assert_ne!(store.header.lone_products, store.header.lone_sums, "a tap a step short leaves the two apart");
    for k in 0..o.products.len() {
        assert_eq!(store.products[k], store.records.iter().filter(|r| !r.products_agree[k]).count() as u64, "product class {k}");
    }
    for k in 0..o.sums.len() {
        assert_eq!(store.sums[k], store.records.iter().filter(|r| !r.sums_agree[k]).count() as u64, "sum class {k}");
    }
}

/// A run that keeps more records than the store holds keeps the first ones the store holds, compares every step, and
/// counts what it could not keep.
#[test]
fn a_recording_run_past_the_store_keeps_the_first_records_it_holds_and_compares_every_step() {
    if !common::require_tool("iverilog") {
        return;
    }
    let o = oracle();
    let cap = o.record_cap();
    let steps = cap as u32 + 300;
    let step = Step { run: Run { source: Source::Random, record: true, tap: 3, delays: [0; 3] }, steps, records: cap };
    let [store] = &simulate(&o, &Semantics::IEEE, &Semantics::IEEE, [4; 3], 9, &[step])[..] else { unreachable!() };
    assert_eq!(store.header.steps, u64::from(steps), "every step compared");
    assert_eq!(store.header.records, cap as u64, "the store full and no further");
    assert_eq!(store.records.len(), cap);
    let (p, s) = (class_of(&o.products, &Semantics::IEEE), class_of(&o.sums, &Semantics::IEEE));
    assert!(store.records.iter().all(|r| r.products_agree[p] && r.sums_agree[s]));
    assert_eq!((store.products[p], store.sums[s]), (0, 0));
}

/// Blocks of different depths are each met at their own: at a tap with no delay only the class of the block that deep
/// agrees with every vector, and a run aligned to all three (`Run::aligned`) has the block's classes agree with
/// every vector for the multiply and the add both, the multiply-add's results its own operands'.
#[test]
fn blocks_of_different_depths_are_each_met_at_their_own_and_aligned_together() {
    if !common::require_tool("iverilog") {
        return;
    }
    let o = oracle();
    let n = o.vectors.len() as u32;
    let latencies = [3, 6, 5];
    let whole = o.record_cap();
    let mut steps: Vec<Step> =
        [2u8, 5].iter().map(|&tap| Step { run: Run { source: Source::Vectors, record: false, tap, delays: [0; 3] }, steps: n, records: whole }).collect();
    steps.push(Step { run: Run::aligned(Source::Vectors, true, latencies).unwrap(), steps: n, records: n as usize });
    let stores = simulate(&o, &Semantics::IEEE, &Semantics::IEEE, latencies, 1, &steps);
    let (p, s) = (class_of(&o.products, &Semantics::IEEE), class_of(&o.sums, &Semantics::IEEE));
    // The multiply deepest instead: the add's results wait for it.
    let deeper = [6, 3, 5];
    let swapped = Run::aligned(Source::Vectors, false, deeper).unwrap();
    assert_eq!(swapped.delays, [0, 3, 1]);
    let [other] = &simulate(&o, &Semantics::IEEE, &Semantics::IEEE, deeper, 1, &[Step { run: swapped, steps: n, records: 0 }])[..] else { unreachable!() };
    names(&other.products, p, "aligned product, the multiply deepest");
    names(&other.sums, s, "aligned sum, delayed to the multiply");
    assert!(stores[0].products[p] == 0 && stores[0].sums.iter().all(|&c| c > 0), "at tap 2 the multiply alone is met");
    assert!(stores[1].sums[s] == 0 && stores[1].products.iter().all(|&c| c > 0), "at tap 5 the add alone");
    assert_eq!((stores[0].header.lone_products, stores[1].header.lone_sums), (0, 0), "each block explained where it is met");
    assert!(stores[0].records.iter().all(|r| r.sums_agree.iter().all(|&a| !a)), "the steps stored at tap 2 are the sums nothing explains");
    let aligned = &stores[2];
    names(&aligned.products, p, "aligned product");
    names(&aligned.sums, s, "aligned sum");
    for (k, r) in aligned.records[n as usize - 64..].iter().enumerate() {
        let (x, y, z) = (f32::from_bits(r.a), f32::from_bits(r.b), f32::from_bits(r.c));
        assert_eq!(r.fma, (x * y + z).to_bits(), "multiply-add vector {k}: its own operands' result");
    }
}

/// The least-registered multiply and add take the same triples, their results recorded as they leave them: in a run
/// aligned to the fully registered blocks they are those of the triples their latency before the tap's.
#[test]
fn the_least_registered_blocks_are_recorded_their_latency_ahead() {
    if !common::require_tool("iverilog") {
        return;
    }
    let o = oracle();
    let n = o.vectors.len();
    let depths = Depths { full: [4, 6, 5], least: [1, 3] };
    let run = Run::aligned(Source::Vectors, true, depths.full).unwrap();
    let [store] = &simulate_deep(&o, &Semantics::IEEE, &Semantics::IEEE, depths, 1, &[Step { run, steps: n as u32, records: n }])[..] else { unreachable!() };
    let tap = usize::from(run.tap);
    for (which, least) in depths.least.iter().enumerate() {
        let shift = tap + 1 - least;
        for k in 0..n - shift {
            let (seen, want) = (store.records[k].least[which], if which == 0 { store.records[k + shift].product } else { store.records[k + shift].sum });
            assert_eq!(seen, want, "record {k}: the {} {least} deep is the aligned one's {shift} records on", ["multiply", "add"][which]);
        }
    }
}

/// What the near stream is for, in its own records: products that land where the least normal is (denormal or a step
/// from it), and sums that cancel down past their operands, each far more often than the random stream gives them.
#[test]
fn the_near_stream_lands_products_at_the_least_normal_and_sums_that_cancel() {
    if !common::require_tool("iverilog") {
        return;
    }
    let o = oracle();
    let steps = 1500u32;
    let record = |source| Step { run: Run { source, record: true, tap: 3, delays: [0; 3] }, steps, records: steps as usize };
    let stores = simulate(&o, &Semantics::IEEE, &Semantics::IEEE, [4; 3], 0x1234_5678_9abc_def1, &[record(Source::Near), record(Source::Random)]);
    let exponent = |x: u32| (x >> 23) & 0xff;
    let tiny = |st: &Store| st.records.iter().filter(|r| r.product & 0x7fff_ffff != 0 && exponent(r.product) <= 1).count();
    let cancelled = |st: &Store| {
        st.records
            .iter()
            .filter(|r| r.sum & 0x7fff_ffff != 0 && exponent(r.sum) < 255 && exponent(r.a).max(exponent(r.b)) >= exponent(r.sum) + 8 && exponent(r.a).abs_diff(exponent(r.b)) <= 2)
            .count()
    };
    let (near, random) = (&stores[0], &stores[1]);
    assert!(tiny(near) * 5 > steps as usize, "a fifth or more of the near products at the least normal: {} of {steps}", tiny(near));
    assert!(tiny(near) > 4 * tiny(random).max(1), "four times the random stream's or more: {} near, {} random", tiny(near), tiny(random));
    assert!(cancelled(near) * 20 > steps as usize, "and a twentieth or more of the sums cancelling eight binades below their operands: {} of {steps}", cancelled(near));
    assert!(cancelled(near) > 10 * cancelled(random).max(1), "{} near, {} random", cancelled(near), cancelled(random));
}

/// A block whose multiply no class of the oracle names (the card's: its NaNs signed as products are, which no member
/// does): the measurement still finds each block's depth, the multiply's as the tap its classes come nearest at, and
/// names the add, which the add read alone takes; the multiply read alone names nothing.
#[test]
fn a_multiply_no_class_names_still_has_its_depth_found_and_leaves_the_add_named() {
    if !common::require_tool("iverilog") || !common::require_tool("iverilog-vpi") || !common::require_tool("vvp") {
        return;
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let model = Semantics::ENGINE;
    // The oracle without the stand-in's product class: its multiply is now outside every class it counts.
    let path = root.join("benchmarks/silicon/d5005/fp_discriminators.json");
    let mut ledger: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let fmul = ledger.as_array_mut().unwrap().iter_mut().find(|op| op["op"] == "fmul").unwrap();
    let classes = fmul["classes"].as_array_mut().unwrap();
    let before = classes.len();
    classes.retain(|c| !c["members"].as_array().unwrap().iter().any(|m| m == &serde_json::json!(model.name())));
    assert_eq!(classes.len(), before - 1);
    let o = Oracle::from_ledger(&ledger.to_string()).unwrap();
    let tree = tempfile::tempdir().unwrap();
    for f in persona_sources(&o, 16, &model, Depths { full: [3, 6, 5], least: [2, 1] }) {
        let path = tree.path().join(f.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, f.text).unwrap();
    }
    let run = Command::new("bash")
        .arg(root.join("scripts/d5005/run-cosim.sh"))
        .arg(tree.path())
        .args(["fp_oracle", "300", "--seed", "77"])
        .env("TMPDIR", std::env::temp_dir())
        .output()
        .expect("bash");
    let out = String::from_utf8_lossy(&run.stdout);
    assert_eq!(run.status.code(), Some(0), "the measurement against the RTL:\n{out}{}", String::from_utf8_lossy(&run.stderr));
    let text = &out[out.find('{').unwrap()..=out.rfind('}').unwrap()];
    let json: serde_json::Value = serde_json::from_str(text).unwrap();
    let latency = &json["latency"];
    assert_eq!((latency["product"].as_u64(), latency["sum"].as_u64(), latency["multiply_add"].as_u64()), (Some(3), Some(6), Some(5)), "each block's own depth: {latency}");
    assert!(json["taps"].as_array().unwrap().iter().all(|t| t["least_product"] != 0), "no class meets the multiply anywhere");
    assert_eq!(Measurement::read_sum(&o, text), Ok(Block { semantics: model, full: 6, least: 1 }), "the add named");
    assert!(matches!(Measurement::read_product(&o, text), Err(logicaffeine_compile::codegen_sva::fp_hard::MeasureError::Class { op: "multiply", agreeing: 0, .. })));
}
