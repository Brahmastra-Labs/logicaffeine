use super::*;

fn ledger() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/d5005/fp_discriminators.json");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn a_run_word_reads_back_as_its_run_and_a_reserved_bit_is_no_run() {
    for source in [Source::Vectors, Source::Random, Source::Near] {
        for record in [false, true] {
            for tap in 0..TAPS as u8 {
                for delays in [[0, 0, 0], [tap, 15 - tap, 7], [15, 1, 0]] {
                    let run = Run { source, record, tap, delays };
                    assert_eq!(Run::of(run.word()), Some(run));
                }
            }
        }
    }
    assert_eq!(Run { source: Source::Near, record: true, tap: 15, delays: [1, 2, 3] }.word(), 0x321f6);
    assert_eq!(Run::of(3), None, "source 3 is none");
    assert_eq!(Run::of(8), None, "bit 3 is reserved");
    assert_eq!(Run::of(1 << 20), None, "bits 31:20 are reserved");
    assert_eq!(Run::of(0xf_ff00).map(|r| r.delays), Some([15, 15, 15]));
}

#[test]
fn an_aligned_run_meets_every_block_at_the_least_tap_all_reach() {
    let run = Run::aligned(Source::Random, false, [3, 6, 5]).unwrap();
    assert_eq!((run.tap, run.delays), (5, [3, 0, 1]), "the tap the deepest block needs, the others delayed to it");
    for (l, d) in [3usize, 6, 5].iter().zip(run.delays) {
        assert_eq!(usize::from(run.tap) + 1, l + usize::from(d), "a block of latency {l} meets the soft units");
    }
    assert_eq!(Run::aligned(Source::Vectors, true, [4, 4, 4]).map(|r| (r.tap, r.delays)), Some((3, [0, 0, 0])));
    assert_eq!(Run::aligned(Source::Vectors, true, [16, 1, 8]).map(|r| (r.tap, r.delays)), Some((15, [0, 15, 8])));
    assert_eq!(Run::aligned(Source::Vectors, false, [0, 4, 4]), None, "no block is no registers deep");
    assert_eq!(Run::aligned(Source::Vectors, false, [17, 4, 4]), None, "nor deeper than the delay line");
}

#[test]
fn the_ledger_makes_one_unit_a_class_from_its_first_member_and_holds_every_vector() {
    let oracle = Oracle::from_ledger(&ledger()).unwrap();
    assert_eq!((oracle.products.len(), oracle.sums.len()), (98, 42));
    for c in oracle.products.iter().chain(&oracle.sums) {
        assert_eq!(c.semantics.name(), c.members[0], "a class is built as its first member");
    }
    let members: Vec<&String> = oracle.products.iter().flat_map(|c| &c.members).collect();
    assert_eq!(members.len(), 98, "every product member in one class");
    let members: Vec<&String> = oracle.sums.iter().flat_map(|c| &c.members).collect();
    assert_eq!(members.len(), 98, "every sum member in one class");
    for s in [Semantics::ENGINE, Semantics::IEEE] {
        assert!(oracle.products.iter().any(|c| c.members.contains(&s.name())), "{}", s.name());
        assert!(oracle.sums.iter().any(|c| c.members.contains(&s.name())), "{}", s.name());
    }
    assert_eq!(oracle.vectors.len(), 402 + 401 + 64);
    assert!(oracle.vectors[..803].iter().all(|v| v[2] == 0), "the discriminators' C is zero");
    assert_eq!(oracle.vectors[803..], fma_vectors()[..]);
    assert_eq!(oracle.counter_beats(), 18, "140 counters, eight to a beat");
    assert_eq!(oracle.records_at(), 19);
    assert_eq!(oracle.record_cap(), STORE_BEATS - 19);
}

#[test]
fn the_multiply_adds_vectors_part_one_rounding_from_two() {
    let v = fma_vectors();
    assert_eq!(v.len(), 64);
    for (k, &[a, b, c]) in v.iter().enumerate() {
        let (x, y, z) = (f32::from_bits(a), f32::from_bits(b), f32::from_bits(c));
        assert!((1.0..2.0).contains(&x) && (1.0..2.0).contains(&y));
        let exact = f64::from(x) * f64::from(y);
        assert_ne!(f64::from(x * y), exact, "{k}: the product is inexact");
        if k % 4 == 3 {
            assert_eq!(z.abs(), 2.0, "{k}");
        } else {
            assert_eq!(z, -(x * y), "{k}: C the rounded product negated");
            assert_eq!((x * y + z).to_bits(), 0, "{k}: two roundings cancel to +0");
            assert_ne!(exact + f64::from(z), 0.0, "{k}: a fused multiply-add keeps the product's error");
        }
    }
}

#[test]
fn the_engine_top_takes_every_port_the_crossing_carries_and_hands_it_to_the_core() {
    let ports = engine_ports();
    let bridge = bridge_verilog();
    for (input, width, name) in &ports {
        let w = if width.is_empty() { String::new() } else { format!("{width} ") };
        let pair = if *input { format!("input {w}s_{name}, output {w}f_{name}") } else { format!("output {w}s_{name}, input {w}f_{name}") };
        assert!(bridge.contains(&pair), "the crossing carries {pair}");
    }
    let names: Vec<&str> = ports.iter().map(|(_, _, n)| n.as_str()).collect();
    for expected in ["rst", "start", "job_at", "job_end", "busy", "fault", "pc", "host_wr", "host_raddr", "host_rdata", "mem_wr_ready"] {
        assert!(names.contains(&expected), "{expected} in {names:?}");
    }
    let header = &bridge[bridge.find("module logos_engine_bridge").unwrap()..];
    let header = &header[..header.find(");").unwrap()];
    assert_eq!(header.matches(" s_").count(), ports.len(), "every port of the crossing once");
    let top = top_text();
    for (_, _, name) in &ports {
        assert_eq!(top.matches(&format!(".s_{name}({name}), .f_{name}(f_{name})")).count(), 1, "{name} crosses once");
        assert_eq!(top.matches(&format!(".{name}(f_{name})")).count(), 1, "{name} reaches the core once");
    }
    assert!(top.contains("logos_fp_oracle core(.clk(fclk), .step_en(fedge), .read_en(cr)"), "the core is the instance ENGINE_SDC names");
    assert!(top.contains("module logos_engine\n  (input clk,"));
}

#[test]
fn each_hard_block_has_the_registers_its_mode_uses_on_clock_zero_and_no_other() {
    let v = hard_blocks();
    let block = |name: &str| {
        let at = v.find(&format!(") {name}(")).unwrap_or_else(|| panic!("no block {name}"));
        let start = v[..at].rfind("fourteennm_fp_mac #(").unwrap();
        v[start..at].to_string()
    };
    let used = |text: &str| -> Vec<String> {
        text.split(", .").filter(|p| p.contains("_clock(\"0\")")).map(|p| p.trim_start_matches("fourteennm_fp_mac #(.").split('(').next().unwrap().to_string()).collect()
    };
    assert_eq!(used(&block("hmul")), ["ay_clock", "az_clock", "mult_pipeline_clock", "mult_2nd_pipeline_clock", "output_clock"]);
    assert_eq!(used(&block("hadd")), ["ax_clock", "ay_clock", "adder_input_clock", "output_clock"]);
    assert_eq!(
        used(&block("hfma")),
        ["ax_clock", "ay_clock", "az_clock", "ax_chainin_pl_clock", "mult_pipeline_clock", "ax_chainin_2nd_pl_clock", "mult_2nd_pipeline_clock", "adder_input_clock", "output_clock"]
    );
    assert_eq!(used(&block("hmul1")), ["output_clock"], "the least-registered multiply");
    assert_eq!(used(&block("hadd1")), ["output_clock"], "and add");
    for b in ["hmul", "hadd", "hfma", "hmul1", "hadd1"] {
        assert_eq!(block(b).matches("_clock(\"").count(), 13, "{b} sets every clock the IP sets");
    }
    assert!(block("hmul1").contains(".operation_mode(\"sp_mult\")") && block("hadd1").contains(".operation_mode(\"sp_add\")"));
    assert!(block("hmul").contains(".operation_mode(\"sp_mult\")") && block("hadd").contains(".operation_mode(\"sp_add\")"));
    assert!(v.contains("hmul(\n    .clk({2'b00, clk}), .ena({2'b00, step_en}), .clr(2'b00), .ay(opr[31:0]), .az(opr[63:32])"));
    assert!(!v.contains("()"), "every output a block drives is connected: an empty connection is a lost bit to the SV check");
    assert!(v.contains("hadd(\n    .clk({2'b00, clk}), .ena({2'b00, step_en}), .clr(2'b00), .ax(opr[31:0]), .ay(opr[63:32]), .accumulate"));
    assert!(v.contains("hfma(\n    .clk({2'b00, clk}), .ena({2'b00, step_en}), .clr(2'b00), .ax(opr[95:64]), .ay(opr[31:0]), .az(opr[63:32])"));
}

#[test]
fn the_core_holds_a_unit_for_every_class_and_the_top_no_hard_block_of_its_own() {
    let oracle = Oracle::from_ledger(&ledger()).unwrap();
    let core = oracle.core_verilog();
    let (nm, na) = (oracle.products.len(), oracle.sums.len());
    for k in 0..nm {
        assert_eq!(core.matches(&format!("logos_fpo_mul{k} um{k}(")).count(), 1);
    }
    for k in 0..na {
        assert_eq!(core.matches(&format!("logos_fpo_add{k} ua{k}(")).count(), 1);
    }
    assert!(!core.contains(&format!("logos_fpo_mul{nm}")) && !core.contains(&format!("logos_fpo_add{na}")));
    let placeholder = |l: &str| l.split('@').skip(1).step_by(2).any(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_uppercase() || c == '_'));
    assert!(!core.lines().any(placeholder), "every placeholder filled: {}", core.lines().find(|l| placeholder(l)).unwrap_or(""));
    assert!(!core.contains("module fourteennm_fp_mac"), "the device's block is Quartus's");
    let engine = oracle.engine_verilog();
    assert_eq!(engine.matches("module logos_engine\n").count(), 1);
    assert_eq!(engine.matches("module logos_engine_bridge").count(), 1);
    assert_eq!(engine.matches("module logos_ram_ce_w512").count(), 1);
    assert_eq!(engine.matches("fourteennm_fp_mac #(").count(), 5);
    let model = hard_model_verilog(&Semantics::IEEE, &Semantics::ENGINE, Depths { full: [3, 6, 5], least: [1, 2] });
    assert_eq!(model.matches("module fourteennm_fp_mac").count(), 1);
    assert!(model.contains("localparam integer L = operation_mode == \"sp_mult\" ? (mult_pipeline_clock == \"NONE\" ? 1 : 3)"));
    assert!(model.contains(": operation_mode == \"sp_add\" ? (adder_input_clock == \"NONE\" ? 2 : 6) : 5;"));
    assert!(engine.matches("logos_fpo_late l").count() == 3 && engine.matches("module logos_fpo_late").count() == 1, "each block's results delayed on their own");
}

/// A store's beats as the core writes them: the header, the counters eight to a beat, the records.
fn beats(oracle: &Oracle, products: &[u64], sums: &[u64], records: &[Record], run: u32) -> Vec<[u64; 8]> {
    let mut out = vec![[MAGIC, 1234, 5, 6, records.len() as u64, run as u64, oracle.products.len() as u64, oracle.sums.len() as u64]];
    let all: Vec<u64> = products.iter().chain(sums).copied().collect();
    for chunk in all.chunks(8) {
        let mut b = [0u64; 8];
        b[..chunk.len()].copy_from_slice(chunk);
        out.push(b);
    }
    for r in records {
        let mut b = [0u64; 8];
        b[0] = u64::from(r.a) | u64::from(r.b) << 32;
        b[1] = u64::from(r.c) | u64::from(r.product) << 32;
        b[2] = u64::from(r.sum) | u64::from(r.fma) << 32;
        b[3] = u64::from(r.flags[0]) | u64::from(r.flags[1]) << 8 | u64::from(r.flags[2]) << 16 | u64::from(r.least[0]) << 24;
        b[7] = u64::from(r.least[1]) << 32;
        for (k, &on) in r.products_agree.iter().enumerate() {
            b[4 + k / 64] |= (on as u64) << (k % 64);
        }
        for (k, &on) in r.sums_agree.iter().enumerate() {
            b[6 + k / 64] |= (on as u64) << (k % 64);
        }
        out.push(b);
    }
    out
}

#[test]
fn a_store_reads_back_as_the_header_counters_and_records_the_core_wrote() {
    let oracle = Oracle::from_ledger(&ledger()).unwrap();
    let (nm, na) = (oracle.products.len(), oracle.sums.len());
    let products: Vec<u64> = (0..nm as u64).map(|k| k * 3 + 1).collect();
    let sums: Vec<u64> = (0..na as u64).map(|k| 1000 + k).collect();
    let record = |k: usize| Record {
        a: 0x3f80_0000 + k as u32,
        b: 0x4000_0000,
        c: 0x8000_0000,
        product: 0x4000_0001,
        sum: 0x4040_0000,
        fma: 0x7fc0_0000,
        flags: [1, 0x10, 0x11],
        products_agree: (0..nm).map(|j| (j + k) % 3 == 0).collect(),
        sums_agree: (0..na).map(|j| j == k).collect(),
        least: [0x4000_0001 + k as u32, 0x4040_0000 - k as u32],
    };
    let records: Vec<Record> = (0..5).map(record).collect();
    let run = Run { source: Source::Near, record: false, tap: 3, delays: [2, 0, 1] }.word();
    let store = Store::read(&oracle, &beats(&oracle, &products, &sums, &records, run)).unwrap();
    assert_eq!(store.products, products);
    assert_eq!(store.sums, sums);
    assert_eq!(store.records, records);
    assert_eq!((store.header.steps, store.header.lone_products, store.header.lone_sums, store.header.run), (1234, 5, 6, run));
    let mut foreign = beats(&oracle, &products, &sums, &records, run);
    foreign[0][0] ^= 1;
    assert_eq!(Store::read(&oracle, &foreign), None, "a store without the oracle's magic");
    let mut other = beats(&oracle, &products, &sums, &records, run);
    other[0][6] = nm as u64 - 1;
    assert_eq!(Store::read(&oracle, &other), None, "a store of another oracle's classes");
    let short = beats(&oracle, &products, &sums, &records, run);
    assert_eq!(Store::read(&oracle, &short[..short.len() - 1]), None, "a store short of the records its header counts");
}

/// A record gives the product classes two whole words and the sum classes one and a half: an oracle holds 128 of the
/// one and 96 of the other, every agreement bit read back where the core wrote it, the least-registered multiply's
/// result beside the flags.
#[test]
fn a_record_holds_128_product_classes_and_96_sum_classes() {
    assert_eq!((MAX_PRODUCT_CLASSES, MAX_SUM_CLASSES), (128, 96));
    let real = Oracle::from_ledger(&ledger()).unwrap();
    let class = |k: usize, from: &[Class]| Class { members: vec![format!("class {k}")], semantics: from[k % from.len()].semantics };
    let wide = |nm: usize, na: usize| Oracle {
        products: (0..nm).map(|k| class(k, &real.products)).collect(),
        sums: (0..na).map(|k| class(k, &real.sums)).collect(),
        vectors: real.vectors.clone(),
    };
    let oracle = wide(MAX_PRODUCT_CLASSES, MAX_SUM_CLASSES);
    let record = Record {
        a: 1,
        b: 2,
        c: 3,
        product: 4,
        sum: 5,
        fma: 6,
        flags: [0xff, 0xee, 0xdd],
        products_agree: (0..MAX_PRODUCT_CLASSES).map(|k| k == 0 || k == 95 || k == 96 || k == 127).collect(),
        sums_agree: (0..MAX_SUM_CLASSES).map(|k| k == 0 || k == 63 || k == 64 || k == 95).collect(),
        least: [0xdead_beef, 0xfeed_f00d],
    };
    let store = Store::read(&oracle, &beats(&oracle, &[0; 128], &[0; 96], &[record.clone()], 0)).unwrap();
    assert_eq!(store.records, vec![record]);
    let ledger_of = |o: &Oracle| -> String {
        let table = |op: &str, classes: &[Class], pairs: &[[u32; 3]]| {
            let members: Vec<String> = classes.iter().map(|c| format!("{{\"members\": [\"{}\"]}}", c.semantics.name())).collect();
            let vectors: Vec<String> = pairs.iter().map(|[a, b, _]| format!("[\"{a:08x}\", \"{b:08x}\"]")).collect();
            format!("{{\"op\": \"{op}\", \"seeds\": 0, \"vectors\": [{}], \"cover\": [], \"classes\": [{}]}}", vectors.join(", "), members.join(", "))
        };
        format!("[{}, {}]", table("fmul", &o.products, &o.vectors[..1]), table("fadd", &o.sums, &o.vectors[..1]))
    };
    let many = |nm: usize, na: usize| Oracle::from_ledger(&ledger_of(&wide(nm, na)));
    assert!(many(MAX_PRODUCT_CLASSES, MAX_SUM_CLASSES).is_ok(), "{:?}", many(MAX_PRODUCT_CLASSES, MAX_SUM_CLASSES).err());
    assert_eq!(many(MAX_PRODUCT_CLASSES + 1, 1).err(), Some(OracleError::TooMany("product classes", MAX_PRODUCT_CLASSES + 1)));
    assert_eq!(many(1, MAX_SUM_CLASSES + 1).err(), Some(OracleError::TooMany("sum classes", MAX_SUM_CLASSES + 1)));
}

#[test]
fn a_ledger_naming_no_member_or_no_word_or_no_table_makes_no_oracle() {
    let text = ledger();
    let renamed = text.replacen("din-flush.dout-flush.tiny-after-denormals.sign-kept.nan-7fc00000", "din-sometimes", 1);
    assert_eq!(Oracle::from_ledger(&renamed), Err(OracleError::Unknown("din-sometimes".into())));
    // A class's later member, never built, named nothing the family has.
    let later = "din-flush.dout-flush.tiny-before.sign-kept.nan-7fc00000";
    let at = text.find("\"fadd\"").unwrap();
    let k = at + text[at..].find(later).unwrap();
    assert!(!text[..k].trim_end().ends_with('['), "the member renamed is not its class's first");
    let renamed = format!("{}din-seldom{}", &text[..k], &text[k + later.len()..]);
    assert_eq!(Oracle::from_ledger(&renamed), Err(OracleError::Unknown("din-seldom".into())));
    let at = text.find("\"vectors\"").unwrap();
    let first = at + text[at..].find("\"0").unwrap();
    let bad = format!("{}\"0000000g{}", &text[..first], &text[first + 9..]);
    assert_eq!(Oracle::from_ledger(&bad), Err(OracleError::Word("0000000g".into())));
    let no_add = text.replace("\"op\": \"fadd\"", "\"op\": \"fsub\"");
    assert_eq!(Oracle::from_ledger(&no_add), Err(OracleError::Missing("fadd")));
    assert!(matches!(Oracle::from_ledger("{"), Err(OracleError::Json(_))));
}

#[test]
fn the_host_table_names_every_class_and_the_stores_layout() {
    let oracle = Oracle::from_ledger(&ledger()).unwrap();
    let h = host_header(&oracle);
    assert!(h.contains(&format!("#define LOGOS_FPO_MAGIC 0x{MAGIC:016x}ull")));
    assert!(h.contains("#define LOGOS_FPO_PRODUCTS 98") && h.contains("#define LOGOS_FPO_SUMS 42"));
    assert!(h.contains("#define LOGOS_FPO_RECORDS_AT 19") && h.contains(&format!("#define LOGOS_FPO_VECTORS {}", oracle.vectors.len())));
    for c in oracle.products.iter().chain(&oracle.sums) {
        assert!(h.contains(&format!("    \"{}\",", c.members.join(" "))), "{}", c.members[0]);
    }
    for name in ["LOGOS_FPO_STORE_BEATS", "LOGOS_FPO_TAPS", "LOGOS_FPO_PRODUCTS", "LOGOS_FPO_SUMS", "LOGOS_FPO_VECTORS", "LOGOS_FPO_RECORDS_AT", "LOGOS_FPO_RECORD_CAP", "LOGOS_FPO_MAGIC"] {
        assert!(HOST_PROGRAM.contains(name), "the host program reads {name}");
    }
}

#[test]
fn the_persona_is_an_engine_build_whose_engine_is_the_oracle_and_whose_twin_holds_the_stand_in() {
    let oracle = Oracle::from_ledger(&ledger()).unwrap();
    let depths = Depths { full: [3, 6, 5], least: [1, 2] };
    let files = persona_sources(&oracle, 16, &Semantics::ENGINE, depths);
    let file = |p: &str| files.iter().find(|f| f.path == p).unwrap_or_else(|| panic!("no {p}")).text.clone();
    let paths: Vec<&str> = files.iter().map(|f| f.path).collect();
    for f in afu_sources(&persona_config(&oracle, 16)) {
        assert_eq!(paths.iter().filter(|p| **p == f.path).count(), 1, "{} once", f.path);
    }
    assert_eq!(file("hw/rtl/logos_engine.v"), oracle.engine_verilog());
    assert!(!file("hw/rtl/logos_engine.v").contains("module fourteennm_fp_mac"), "the card's block is Quartus's");
    assert_eq!(file("sim/logos_engine_gated.v"), format!("{}\n{}", oracle.engine_verilog(), hard_model_verilog(&Semantics::ENGINE, &Semantics::ENGINE, depths)));
    assert_eq!(file("sw/logos_fp_oracle.c"), HOST_PROGRAM);
    assert_eq!(file("sw/logos_fp_oracle.h"), host_header(&oracle));
    assert!(file("hw/rtl/logos_d5005.sdc").contains("set_multicycle_path -from $from -to $to -setup -end 7"), "core paths half of a 16-cycle compute clock less one");
    let domains = file("hw/logos_d5005_domains.json");
    assert!(domains.contains("\"single_clock\": [\"logos_fp_oracle\"]") && !domains.contains("logos_engine_core_ce"), "the one-clock core the gates box is the oracle's: {domains}");
    let cfg = persona_config(&oracle, 16);
    assert_eq!(cfg.compute_divide, 16);
    assert_eq!(cfg.build.to_be_bytes(), sha256(oracle.engine_verilog().as_bytes())[..8], "the build identity is the engine's digest");
    assert_eq!(cfg.uuid, crate::spatial::decode_engine::ofs::AFU_UUID, "a host program opens it as it opens every engine build");
}

#[test]
fn a_block_connects_the_operands_its_mode_reads_and_none_of_the_devices_internal_ports() {
    // Quartus refuses a block with a port its mode does not read connected (AX in SP_MULT, AZ in SP_ADD), and the
    // internal-use DFX ports connected at all.
    for (mode, read) in [("sp_mult", ["ay", "az"].as_slice()), ("sp_add", &["ax", "ay"]), ("sp_mult_add", &["ax", "ay", "az"])] {
        let v = fp_mac(mode, "u", MULTIPLY_REGISTERS, "en", ["x", "y", "z"], "h", "f");
        for (port, operand) in [("ax", "x"), ("ay", "y"), ("az", "z")] {
            assert_eq!(v.contains(&format!(".{port}(")), read.contains(&port), "{mode}: {port}\n{v}");
            if read.contains(&port) {
                assert!(v.contains(&format!(".{port}({operand})")), "{mode}: {port} is {operand}\n{v}");
            }
        }
        assert!(!v.to_lowercase().contains("dfx"), "{mode}: no internal-use port connected\n{v}");
    }
    // The oracle's five: three read ax (the adds, the multiply-add), three az (the multiplies, the multiply-add).
    let engine = Oracle::from_ledger(&ledger()).unwrap().engine_verilog();
    assert_eq!((engine.matches(".ax(").count(), engine.matches(".ay(").count(), engine.matches(".az(").count()), (3, 5, 3));
    assert!(!engine.to_lowercase().contains("dfx"));
}

#[test]
fn every_operand_a_blocks_mode_reads_crosses_as_many_registers_to_the_adder() {
    // The block's datapath: AX through its input and two chain-in pipeline registers, AY and AZ each through its input
    // register and the multiplier's two pipeline registers, the adder's input and the output after. An operand with
    // fewer meets the others' earlier values: the card's add with AX's pipeline and not AY's computed A(t−2) + B(t).
    let path = |used: &[&str], port: &str| -> usize {
        let own: &[&str] = match port {
            "ax" => &["ax_clock", "ax_chainin_pl_clock", "ax_chainin_2nd_pl_clock"],
            "ay" => &["ay_clock", "mult_pipeline_clock", "mult_2nd_pipeline_clock"],
            _ => &["az_clock", "mult_pipeline_clock", "mult_2nd_pipeline_clock"],
        };
        own.iter().filter(|r| used.contains(r)).count()
    };
    for (mode, used) in [("sp_mult", MULTIPLY_REGISTERS), ("sp_add", ADD_REGISTERS), ("sp_mult_add", MULTIPLY_ADD_REGISTERS), ("sp_mult", LEAST_REGISTERS), ("sp_add", LEAST_REGISTERS)] {
        let read = MODE_OPERANDS.iter().find(|(m, _)| *m == mode).unwrap().1;
        let depths: Vec<usize> = ["ax", "ay", "az"].iter().zip(read).filter(|(_, r)| *r).map(|(p, _)| path(used, p)).collect();
        assert!(depths.windows(2).all(|w| w[0] == w[1]), "{mode} with {used:?}: the operands it reads cross {depths:?} registers");
        // Quartus refuses an add whose multiplier pipeline registers are set (Error 19300).
        if mode == "sp_add" {
            assert!(!used.iter().any(|r| r.starts_with("mult_")), "{mode} with {used:?}: an add sets no multiplier pipeline register");
        }
    }
}
