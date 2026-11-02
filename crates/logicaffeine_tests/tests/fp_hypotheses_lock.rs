//! The binary32 hypothesis family (`codegen_sva::fp_hypotheses`) held to two fixed points before it is used to tell a
//! hard floating-point block apart: its ENGINE member is proven equal to the engine's own multiply and add
//! (`decode_engine::rtl::fp32`), and its IEEE member computes what this machine's own `f32` multiply and add compute,
//! bit for bit, on every pair of special values and on thousands of random and boundary operands. The discriminator
//! ledger the oracle measures by (`benchmarks/silicon/d5005/fp_discriminators.json`) is what every member computes.

mod common;

use std::fmt::Write;
use std::path::PathBuf;

use logicaffeine_compile::codegen_sva::fp_hypotheses::{fadd_verilog, family, fmul_verilog, Semantics, SPECIALS};
use logicaffeine_compile::codegen_sva::netlist::{synthesize, synthesize_reading, Undefined};
use logicaffeine_compile::spatial::decode_engine::rtl::fp32::fp32_verilog;
use logicaffeine_proof::aig::Aig;
use logicaffeine_proof::sweep::{equivalent, Options, Verdict};

fn aig(verilog: &str, top: &str) -> Aig {
    synthesize(verilog, top, &[]).unwrap_or_else(|e| panic!("{top}: {e}")).aig
}

/// The operand values every pair of which is tried: signed zeros, the denormals' ends, the normals' ends, one,
/// infinities, quiet and signalling NaNs.
const SPECIAL: [u32; 20] = [
    0x0000_0000, 0x8000_0000, 0x0000_0001, 0x8000_0001, 0x007f_ffff, 0x807f_ffff, 0x0080_0000, 0x8080_0000,
    0x3f80_0000, 0xbf80_0000, 0x7f7f_ffff, 0xff7f_ffff, 0x7f80_0000, 0xff80_0000, 0x7fc0_0000, 0xffc0_0001,
    0x7f80_0001, 0x7fbf_ffff, 0x0040_0000, 0x3400_0001,
];

/// Operand pairs: every pair of specials, then random operands whose exponents crowd the denormals, the overflow edge
/// and the cancellation band, with mantissas that make ties and near-ties.
fn corpus() -> Vec<(u32, u32)> {
    let mut pairs: Vec<(u32, u32)> = SPECIAL.iter().flat_map(|&a| SPECIAL.iter().map(move |&b| (a, b))).collect();
    let mut s: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    for _ in 0..24_000 {
        let r = next();
        let exp = |k: u64| -> u32 {
            match k % 6 {
                0 => (k >> 3) as u32 % 8,
                1 => 58 + (k >> 3) as u32 % 16,
                2 => 120 + (k >> 3) as u32 % 16,
                3 => 240 + (k >> 3) as u32 % 15,
                _ => (k >> 3) as u32 % 256,
            }
        };
        let mant = |k: u64| -> u32 {
            match k % 4 {
                0 => 0x7f_ffff,
                1 => 0x40_0000 | ((k >> 8) as u32 & 1),
                _ => (k >> 9) as u32 & 0x7f_ffff,
            }
        };
        let a = ((r >> 63) as u32) << 31 | exp(r >> 1) << 23 | mant(r >> 17);
        let b = ((r >> 62 & 1) as u32) << 31 | exp(r >> 20) << 23 | mant(r >> 33);
        pairs.push((a, b));
        let near = a ^ ((r >> 40) as u32 & 0x3f) | 0x8000_0000 & !a;
        pairs.push((a, near));
    }
    pairs
}

/// A circuit simulated on operand pairs `a`, `b`, sixty-four pairs a word.
struct Simulated<'a> {
    g: &'a Aig,
    sim: Vec<u64>,
    words: usize,
    pairs: usize,
}

fn simulate<'a>(g: &'a Aig, pairs: &[(u32, u32)]) -> Simulated<'a> {
    let words = pairs.len().div_ceil(64);
    let index = |name: String| g.input_index(&name).unwrap_or_else(|| panic!("input {name}"));
    let (a, b): (Vec<usize>, Vec<usize>) = (0..32).map(|bit| (index(format!("a[{bit}]")), index(format!("b[{bit}]")))).unzip();
    let mut inputs = vec![0u64; g.inputs().len() * words];
    for (k, &(x, y)) in pairs.iter().enumerate() {
        for bit in 0..32 {
            for (v, at) in [(x, a[bit]), (y, b[bit])] {
                if v >> bit & 1 == 1 {
                    inputs[at * words + k / 64] |= 1 << (k % 64);
                }
            }
        }
    }
    Simulated { g, sim: g.simulate(words, &inputs, &[]), words, pairs: pairs.len() }
}

impl Simulated<'_> {
    /// The 32-bit output `port` for every pair.
    fn port(&self, port: &str) -> Vec<u32> {
        let g = self.g;
        let lits: Vec<_> =
            (0..32).map(|bit| g.outputs()[g.output_index(&format!("{port}[{bit}]")).unwrap_or_else(|| panic!("output {port}[{bit}]"))].lit).collect();
        (0..self.pairs)
            .map(|k| lits.iter().enumerate().fold(0u32, |y, (bit, &o)| y | ((Aig::word(&self.sim, self.words, o, k / 64) >> (k % 64) & 1) as u32) << bit))
            .collect()
    }
}

/// The circuit's 32-bit output `port` for every pair.
fn evaluate(g: &Aig, pairs: &[(u32, u32)], port: &str) -> Vec<u32> {
    simulate(g, pairs).port(port)
}

fn is_nan(x: u32) -> bool {
    x & 0x7f80_0000 == 0x7f80_0000 && x & 0x7f_ffff != 0
}

/// What an x86 core's SSE unit returns: its own arithmetic, with the NaN it returns read as the IEEE member reads it
/// (a NaN operand's payload quieted, `0xffc00000` for an invalid operation); when both operands are NaNs the core may
/// return either's payload, as the compiler is free to commute them.
fn agrees(a: u32, b: u32, host: u32, unit: u32) -> bool {
    if is_nan(a) && is_nan(b) {
        return unit == a | 0x40_0000 || unit == b | 0x40_0000;
    }
    unit == host
}

#[test]
fn the_engine_member_is_the_engines_multiply_and_add() {
    if !common::require_tool("yosys") {
        return;
    }
    let engine = fp32_verilog();
    for (ours, theirs, top) in [
        (fmul_verilog("h", &Semantics::ENGINE), "logos_fmul", "fmul"),
        (fadd_verilog("h", &Semantics::ENGINE), "logos_fadd", "fadd"),
    ] {
        let report = equivalent(&aig(&ours, "h"), &aig(engine, theirs), &Options::default()).unwrap();
        assert!(
            report.all_equal(),
            "{top}: {:?}",
            report.verdicts.iter().filter(|(_, v)| *v != Verdict::Equal).collect::<Vec<_>>()
        );
        assert!(report.check(), "{top}");
    }
}

#[test]
#[cfg(target_arch = "x86_64")]
fn the_ieee_member_is_this_machines_multiply_and_add() {
    if !common::require_tool("yosys") {
        return;
    }
    let pairs = corpus();
    let mul = evaluate(&aig(&fmul_verilog("h", &Semantics::IEEE), "h"), &pairs, "y");
    let add = evaluate(&aig(&fadd_verilog("h", &Semantics::IEEE), "h"), &pairs, "y");
    let mut wrong = Vec::new();
    for (k, &(a, b)) in pairs.iter().enumerate() {
        let (x, y) = (f32::from_bits(a), f32::from_bits(b));
        if !agrees(a, b, (x * y).to_bits(), mul[k]) {
            wrong.push(format!("{a:08x} * {b:08x}: host {:08x}, unit {:08x}", (x * y).to_bits(), mul[k]));
        }
        if !agrees(a, b, (x + y).to_bits(), add[k]) {
            wrong.push(format!("{a:08x} + {b:08x}: host {:08x}, unit {:08x}", (x + y).to_bits(), add[k]));
        }
    }
    assert!(wrong.is_empty(), "{} of {} disagree, first: {:#?}", wrong.len(), 2 * pairs.len(), &wrong[..wrong.len().min(8)]);
}

#[derive(serde::Deserialize)]
struct LedgerClass {
    members: Vec<String>,
    results: Vec<String>,
}

#[derive(serde::Deserialize)]
struct LedgerOp {
    op: String,
    seeds: usize,
    vectors: Vec<[String; 2]>,
    cover: Vec<[String; 2]>,
    classes: Vec<LedgerClass>,
}

/// Every member of the family under one top `fam`, ports `a`, `b` and member `k`'s result `y<k>`.
fn whole_family(generate: fn(&str, &Semantics) -> String, members: &[Semantics]) -> String {
    let mut v: String = members.iter().enumerate().map(|(k, s)| generate(&format!("h{k}"), s)).collect();
    v.push_str("module fam(input [31:0] a, input [31:0] b");
    for k in 0..members.len() {
        write!(v, ", output [31:0] y{k}").unwrap();
    }
    v.push_str(");\n");
    for k in 0..members.len() {
        writeln!(v, "  h{k} u{k}(.a(a), .b(b), .y(y{k}));").unwrap();
    }
    v.push_str("endmodule\n");
    v
}

fn hex(s: &str) -> u32 {
    u32::from_str_radix(s, 16).unwrap_or_else(|_| panic!("not a hex word: {s}"))
}

#[test]
fn the_discriminator_ledger_is_what_every_member_computes_and_tells_every_class_apart() {
    if !common::require_tool("yosys") {
        return;
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/d5005/fp_discriminators.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let ledger: Vec<LedgerOp> = serde_json::from_str(&text).unwrap();
    assert_eq!(ledger.iter().map(|o| o.op.as_str()).collect::<Vec<_>>(), ["fmul", "fadd"]);
    let semantics = family();
    let names: Vec<String> = semantics.iter().map(|s| s.name()).collect();
    let specials: Vec<[String; 2]> =
        SPECIALS.iter().flat_map(|&a| SPECIALS.iter().map(move |&b| [format!("{a:08x}"), format!("{b:08x}")])).collect();
    let generators: [fn(&str, &Semantics) -> String; 2] = [fmul_verilog, fadd_verilog];
    for (entry, generate) in ledger.iter().zip(generators) {
        let op = &entry.op;
        assert_eq!(entry.seeds, specials.len(), "{op}");
        assert_eq!(entry.vectors[..entry.seeds], specials[..], "{op}: the special pairs come first, in order");
        assert!(entry.cover.iter().all(|c| entry.vectors.contains(c)), "{op}: the cover is drawn from the vectors");
        let mut listed: Vec<&String> = entry.classes.iter().flat_map(|c| &c.members).collect();
        listed.sort();
        let mut every: Vec<&String> = names.iter().collect();
        every.sort();
        assert_eq!(listed, every, "{op}: every member in one class, once");
        for (i, x) in entry.classes.iter().enumerate() {
            assert_eq!(x.results.len(), entry.cover.len(), "{op}: a result for each cover pair");
            for y in &entry.classes[i + 1..] {
                assert_ne!(x.results, y.results, "{op}: {:?} and {:?} are told apart on the cover", x.members, y.members);
            }
        }
        let cover: Vec<(u32, u32)> = entry.cover.iter().map(|[a, b]| (hex(a), hex(b))).collect();
        let all: Vec<(u32, u32)> = entry.vectors.iter().map(|[a, b]| (hex(a), hex(b))).collect();
        let g = synthesize_reading(&whole_family(generate, &semantics), "fam", &[], Undefined::Zero).unwrap_or_else(|e| panic!("{op}: {e}")).aig;
        let (on_cover, on_vectors) = (simulate(&g, &cover), simulate(&g, &all));
        let mut on_all: Vec<Vec<u32>> = Vec::new();
        for (k, name) in names.iter().enumerate() {
            let class = entry.classes.iter().position(|c| c.members.contains(name)).expect("listed above");
            let want: Vec<u32> = entry.classes[class].results.iter().map(|r| hex(r)).collect();
            assert_eq!(on_cover.port(&format!("y{k}")), want, "{op} {name}: its class's results on the cover");
            on_all.push(on_vectors.port(&format!("y{k}")));
        }
        for class in &entry.classes {
            let first = names.iter().position(|n| *n == class.members[0]).unwrap();
            for m in &class.members[1..] {
                let k = names.iter().position(|n| n == m).unwrap();
                assert_eq!(on_all[k], on_all[first], "{op}: {m} computes what {} does on every vector", class.members[0]);
            }
        }
    }
}

/// No member leaves a bit undefined: the whole family synthesized with every undefined bit a free input has none, so
/// a member computes what the ledger says of it whatever a synthesis tool makes of undefined bits (the oracle persona
/// builds every class on the card).
#[test]
fn no_member_leaves_a_bit_undefined() {
    if !common::require_tool("yosys") {
        return;
    }
    let semantics = family();
    let generators: [(&str, fn(&str, &Semantics) -> String); 2] = [("fmul", fmul_verilog), ("fadd", fadd_verilog)];
    for (op, generate) in generators {
        let g = synthesize_reading(&whole_family(generate, &semantics), "fam", &[], Undefined::Free).unwrap_or_else(|e| panic!("{op}: {e}")).aig;
        let free = g.inputs().iter().filter(|p| p.name.is_none()).count();
        assert_eq!(free, 0, "{op}: {free} undefined bits in the family's netlist");
        assert_eq!(g.inputs().len(), 64, "{op}: the operands alone");
    }
}

/// Every member's software multiply and add (`Semantics::mul`, `Semantics::add`) is its unit, bit for bit, on the
/// corpus and on every vector of the discriminator ledger: the one seam the engine's arithmetic takes a member from.
#[test]
fn every_members_software_multiply_and_add_is_its_unit() {
    if !common::require_tool("yosys") {
        return;
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/d5005/fp_discriminators.json");
    let ledger: Vec<LedgerOp> = serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).unwrap();
    let members = family();
    let ops: [(&str, fn(&str, &Semantics) -> String, fn(&Semantics, u32, u32) -> u32); 2] =
        [("fmul", fmul_verilog, Semantics::mul), ("fadd", fadd_verilog, Semantics::add)];
    for ((op, generate, software), entry) in ops.into_iter().zip(&ledger) {
        assert_eq!(entry.op, op);
        let mut pairs = corpus();
        pairs.extend(entry.vectors.iter().map(|[a, b]| (hex(a), hex(b))));
        pairs.extend(far_apart());
        let g = synthesize_reading(&whole_family(generate, &members), "fam", &[], Undefined::Zero).unwrap_or_else(|e| panic!("{op}: {e}")).aig;
        let on = simulate(&g, &pairs);
        for (k, s) in members.iter().enumerate() {
            let unit = on.port(&format!("y{k}"));
            for (&(a, b), &y) in pairs.iter().zip(&unit) {
                assert_eq!(software(s, a, b), y, "{op} {}: {a:08x}, {b:08x}", s.name());
            }
        }
    }
}

/// A power of two and an operand of the other sign 24 to 30 binades below it, and the same with its sign: where an add
/// cancels into the next binade down and the smaller operand's bits past the guard decide the rounding.
fn far_apart() -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for e1 in [31u32, 127, 200, 254] {
        for d in 24..=30u32 {
            for m in [1u32, 0x40_0001, 0x7f_ffff, 0] {
                for sign in [0u32, 0x8000_0000] {
                    let (a, b) = (sign | e1 << 23, (sign ^ 0x8000_0000) | (e1 - d) << 23 | m);
                    out.extend([(a, b), (b, a), (a, b ^ 0x8000_0000)]);
                }
            }
        }
    }
    out
}
