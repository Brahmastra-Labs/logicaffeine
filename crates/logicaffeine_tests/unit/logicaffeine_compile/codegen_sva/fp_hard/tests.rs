use super::*;
use crate::codegen_sva::fp_hypotheses::family;

fn named(name: &str) -> Semantics {
    family().into_iter().find(|s| s.name() == name).unwrap_or_else(|| panic!("no member {name}"))
}

#[test]
fn a_block_no_deeper_than_the_unit_is_registered_fully_and_padded_to_its_latency() {
    let b = Block { semantics: Semantics::ENGINE, full: 2, least: 1 };
    let p = plan(Op::Multiply, &b, 3).unwrap();
    assert_eq!((p.registering, p.pads), (Registering::Full, 1));
    assert!(!p.flush_operands && !p.flush_results && !p.canonical_nan, "the engine's own semantics need nothing around them");
    let exact = plan(Op::Add, &Block { full: 4, ..b }, 4).unwrap();
    assert_eq!((exact.registering, exact.pads), (Registering::Full, 0));
}

#[test]
fn a_block_deeper_than_the_unit_keeps_its_output_register_alone_or_is_refused() {
    let b = Block { semantics: Semantics::ENGINE, full: 4, least: 1 };
    let p = plan(Op::Multiply, &b, 3).unwrap();
    assert_eq!((p.registering, p.pads), (Registering::Least, 2));
    let deep = Block { full: 5, least: 4, ..b };
    assert_eq!(plan(Op::Multiply, &deep, 3), Err(HardError::TooDeep { op: "multiply", latency: 3, least: 4 }));
    assert!(hard_verilog(Op::Multiply, "m", &deep, 3).unwrap_err().to_string().contains("4 clocks deep with its output register alone, past the unit's 3"));
}

#[test]
fn what_the_block_keeps_the_logic_around_it_flushes_and_what_it_flushes_otherwise_is_refused() {
    let ieee = plan(Op::Multiply, &Block { semantics: Semantics::IEEE, full: 3, least: 1 }, 3).unwrap();
    assert!(ieee.flush_operands && ieee.flush_results && ieee.canonical_nan, "an x86 core's arithmetic needs all three");
    let nan = plan(Op::Add, &Block { semantics: named("din-flush.dout-flush.tiny-after-denormals.sign-kept.nan-ffffffff"), full: 4, least: 1 }, 4).unwrap();
    assert!(!nan.flush_operands && !nan.flush_results && nan.canonical_nan, "another NaN alone");
    let before = Block { semantics: named("din-flush.dout-flush.tiny-before.sign-kept.nan-7fc00000"), full: 3, least: 1 };
    assert!(matches!(plan(Op::Multiply, &before, 3), Err(HardError::Semantics { why, .. }) if why.contains("before the engine does")));
    let positive = Block { semantics: named("din-flush.dout-flush.tiny-after-denormals.sign-pos.nan-7fc00000"), full: 3, least: 1 };
    assert!(matches!(plan(Op::Add, &positive, 4), Err(HardError::Semantics { why, .. }) if why.contains("+0 whatever its sign")));
    let accepted = family().iter().filter(|s| plan(Op::Multiply, &Block { semantics: **s, full: 1, least: 1 }, 3).is_ok()).count();
    assert_eq!(accepted, 2 * 7 + 2 * 7, "kept results, or results flushed as the engine flushes them, under every operand and NaN choice");
}

#[test]
fn the_wrapper_has_the_pipelined_units_ports_and_its_block_as_the_plan_registers_it() {
    let b = Block { semantics: Semantics::IEEE, full: 4, least: 1 };
    let v = hard_verilog(Op::Multiply, "logos_fmul_hard", &b, 3).unwrap();
    assert!(v.contains("module logos_fmul_hard(input clk, input [31:0] a, input [31:0] b, output [31:0] y);"));
    assert_eq!(v.matches("fourteennm_fp_mac #(").count(), 1);
    assert!(v.contains(".operation_mode(\"sp_mult\")") && v.contains(".output_clock(\"0\")") && v.contains(".mult_pipeline_clock(\"NONE\")"), "the least-registered multiply");
    assert!(v.contains(".ena({2'b00, 1'b1})") && v.contains(".ay(ia), .az(ib)"));
    assert!(v.contains("reg [31:0] pad [0:1];"), "two registers after the block");
    assert!(v.contains("wire [31:0] ia = a, ib = b;"), "a multiply's block takes the operands as they come");
    assert!(v.contains("{da || db, (na || nb || (fa && zb) || (fb && za)) ? 32'h7fc0_0000 : {a[31] ^ b[31], 31'd0}}"), "a denormal operand decided beside it");
    assert!(v.contains("reg [32:0] side [0:0];"), "as deep as the least-registered block");
    assert!(v.contains("wire [31:0] c = side[0][32] ? side[0][31:0] : nan ? 32'h7fc0_0000 : (h[30:23] == 8'd0) ? {h[31], 31'd0} : h;"));
    let add = hard_verilog(Op::Add, "a", &Block { semantics: Semantics::IEEE, full: 4, least: 1 }, 4).unwrap();
    assert!(add.contains("wire [31:0] ia = ((a[30:23] == 8'd0) ? {a[31], 31'd0} : a)") && !add.contains("side"), "an add's flushed before the block");
    assert!(add.contains(".\n// Around it: a denormal operand read as a zero of its sign, a denormal result written as a zero of its sign, every NaN result 0x7fc00000.\n"));
    assert!(!v.contains("()"), "every output connected");
    let ce = hard_verilog_ce(Op::Add, "logos_fadd_hard_ce", &Block { semantics: Semantics::ENGINE, full: 4, least: 1 }, 4).unwrap();
    assert!(ce.contains("module logos_fadd_hard_ce(input clk, input step_en, input [31:0] a, input [31:0] b, output [31:0] y);"));
    assert!(ce.contains(".ena({2'b00, step_en})") && ce.contains(".ax(ia), .ay(ib)") && ce.contains(".adder_input_clock(\"0\")"));
    assert!(ce.contains("assign y = c;") && ce.contains("wire [31:0] ia = a, ib = b;") && ce.contains("wire [31:0] c = h;"), "the engine's block, exactly deep: nothing around it");
}

fn oracle() -> Oracle {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/d5005/fp_discriminators.json");
    Oracle::from_ledger(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// A measurement's JSON: class `p` of the products and `s` of the sums agreeing everywhere (none when `None`), and the
/// multiply-add's records as `fma` computes them.
fn measurement(o: &Oracle, p: Option<usize>, s: Option<usize>, fma: fn(f32, f32, f32) -> f32) -> String {
    let counters = |n: usize, zero: Option<usize>| (0..n).map(|k| if Some(k) == zero { "0".to_string() } else { format!("{}", 3 + k) }).collect::<Vec<_>>().join(", ");
    let run = |name: &str| {
        format!(
            "\"{name}\": {{\"steps\": 100, \"lone_products\": {}, \"lone_sums\": {}, \"records\": 0, \"products\": [{}], \"sums\": [{}]}}",
            u64::from(p.is_none()) * 7,
            u64::from(s.is_none()) * 5,
            counters(o.products.len(), p),
            counters(o.sums.len(), s)
        )
    };
    let records: Vec<String> = fma_vectors()
        .iter()
        .map(|&[a, b, c]| {
            let y = fma(f32::from_bits(a), f32::from_bits(b), f32::from_bits(c)).to_bits();
            format!("[\"{a:08x}\", \"{b:08x}\", \"{c:08x}\", \"00000000\", \"00000000\", \"{y:08x}\", \"000000\", \"00000000\", \"00000000\"]")
        })
        .collect();
    format!(
        "{{\"seed\": 1, \"taps\": [], \"latency\": {{\"product\": 4, \"sum\": 6, \"multiply_add\": 5, \"product_least\": 2, \"sum_least\": 1}}, \"runs\": {{{}, \"records\": [{}], {}, {}}}, \"run_word\": 0, \"complete\": true}}",
        run("vectors"),
        records.join(", "),
        run("random"),
        run("near")
    )
}

#[test]
fn a_measurement_names_each_modes_one_agreeing_class_its_depths_and_how_its_multiply_add_rounds() {
    let o = oracle();
    let (p, s) = (o.products.iter().position(|c| c.members.contains(&Semantics::IEEE.name())).unwrap(), 3);
    let twice = Measurement::read(&o, &measurement(&o, Some(p), Some(s), |x, y, z| x * y + z)).unwrap();
    assert_eq!(twice.product, Block { semantics: Semantics::IEEE, full: 4, least: 2 });
    assert_eq!(twice.sum, Block { semantics: o.sums[s].semantics, full: 6, least: 1 });
    assert_eq!((twice.multiply_add_latency, twice.multiply_add), (5, Rounding::Twice));
    let once = Measurement::read(&o, &measurement(&o, Some(p), Some(s), |x, y, z| (f64::from(x) * f64::from(y) + f64::from(z)) as f32)).unwrap();
    assert_eq!(once.multiply_add, Rounding::Once, "fused: the product's error survives");
    let neither = Measurement::read(&o, &measurement(&o, Some(p), Some(s), |x, _, _| x)).unwrap();
    assert_eq!(neither.multiply_add, Rounding::Other);
    let outside = Measurement::read(&o, &measurement(&o, None, Some(s), |x, y, z| x * y + z)).unwrap_err();
    assert_eq!(outside, MeasureError::Class { op: "multiply", agreeing: 0, unexplained: 21 });
    assert!(outside.to_string().contains("no member of the family"));
    let unfinished = measurement(&o, Some(p), Some(s), |x, y, z| x * y + z).replace("\"complete\": true", "\"complete\": false");
    assert_eq!(Measurement::read(&o, &unfinished), Err(MeasureError::Incomplete("finished run")));
    assert_ne!(p, 0);
    let two = measurement(&o, Some(p), Some(s), |x, y, z| x * y + z).replace("\"products\": [3, ", "\"products\": [0, ");
    assert!(matches!(Measurement::read(&o, &two), Err(MeasureError::Class { op: "multiply", agreeing: 2, .. })), "two classes agreeing in every run name no block");
    let one_run = measurement(&o, Some(p), Some(s), |x, y, z| x * y + z).replacen("\"products\": [3, ", "\"products\": [0, ", 1);
    assert_eq!(Measurement::read(&o, &one_run).unwrap().product.semantics, Semantics::IEEE, "a class agreeing in one run alone is not the block's");
}

#[test]
fn each_blocks_reading_stands_alone_so_a_multiply_no_class_names_leaves_the_add_named() {
    let o = oracle();
    let (p, s) = (o.products.iter().position(|c| c.members.contains(&Semantics::IEEE.name())).unwrap(), 3);
    let outside = measurement(&o, None, Some(s), |x, y, z| x * y + z);
    assert_eq!(Measurement::read_sum(&o, &outside), Ok(Block { semantics: o.sums[s].semantics, full: 6, least: 1 }), "the add read alone");
    assert_eq!(Measurement::read_product(&o, &outside), Err(MeasureError::Class { op: "multiply", agreeing: 0, unexplained: 21 }));
    assert!(Measurement::read(&o, &outside).is_err(), "the whole reading still wants both");
    let sum_outside = measurement(&o, Some(p), None, |x, y, z| x * y + z);
    assert_eq!(Measurement::read_product(&o, &sum_outside), Ok(Block { semantics: Semantics::IEEE, full: 4, least: 2 }));
    assert_eq!(Measurement::read_sum(&o, &sum_outside), Err(MeasureError::Class { op: "add", agreeing: 0, unexplained: 15 }));
    let both = measurement(&o, Some(p), Some(s), |x, y, z| x * y + z);
    let whole = Measurement::read(&o, &both).unwrap();
    assert_eq!((Measurement::read_product(&o, &both), Measurement::read_sum(&o, &both)), (Ok(whole.product), Ok(whole.sum)), "each alone is the whole reading's");
    let unfinished = both.replace("\"complete\": true", "\"complete\": false");
    assert_eq!(Measurement::read_sum(&o, &unfinished), Err(MeasureError::Incomplete("finished run")));
    let no_depth = both.replace("\"sum\": 6", "\"sum\": -1");
    assert_eq!(Measurement::read_sum(&o, &no_depth), Err(MeasureError::Incomplete("sum")), "an add whose depth was not found names no block");
    assert_eq!(Measurement::read_product(&o, &no_depth).map(|b| b.full), Ok(4), "while the multiply's still reads");
}

#[test]
fn a_modes_tally_is_each_classs_differing_steps_over_every_run_read_beside_its_depths_and_the_runs() {
    let o = oracle();
    let p = o.products.iter().position(|c| c.members.contains(&Semantics::IEEE.name())).unwrap();
    let both = measurement(&o, Some(p), None, |x, y, z| x * y + z);
    let product = Measurement::read_tally(&o, &both, Op::Multiply).unwrap();
    assert_eq!(product.differing.len(), o.products.len());
    assert_eq!(product.differing[p], 0);
    assert!((0..o.products.len()).filter(|&k| k != p).all(|k| product.differing[k] == 3 * (3 + k as u64)), "three runs of 3 + k each");
    assert_eq!(product.unexplained, 0);
    let sum = Measurement::read_tally(&o, &both, Op::Add).unwrap();
    assert_eq!((sum.differing.len(), sum.differing[0], sum.unexplained), (o.sums.len(), 9, 15), "no class agreeing, 5 unexplained a run");
    assert_eq!(Measurement::read_depths(&both, Op::Multiply), Ok((4, 2)));
    assert_eq!(Measurement::read_depths(&both, Op::Add), Ok((6, 1)));
    assert_eq!(Measurement::read_runs(&both), Ok((1, [100, 100, 100])));
    let no_depth = both.replace("\"sum_least\": 1", "\"sum_least\": 0");
    assert_eq!(Measurement::read_depths(&no_depth, Op::Add), Err(MeasureError::Incomplete("sum_least")));
    assert_eq!(Measurement::read_depths(&no_depth, Op::Multiply), Ok((4, 2)), "each mode's depths alone");
    let short = both.replacen("\"sums\": [3, ", "\"sums\": [", 1);
    assert_eq!(Measurement::read_tally(&o, &short, Op::Add), Err(MeasureError::Incomplete("counters")), "a run missing a class's counter");
    let unread = both.replacen("\"lone_sums\": 5", "\"lone_sums\": -5", 1);
    assert_eq!(Measurement::read_tally(&o, &unread, Op::Add), Err(MeasureError::Incomplete("unexplained steps")));
    let no_steps = both.replacen("\"steps\": 100", "\"steps\": \"many\"", 1);
    assert_eq!(Measurement::read_runs(&no_steps), Err(MeasureError::Incomplete("steps")));
    let unfinished = both.replace("\"complete\": true", "\"complete\": false");
    assert_eq!(Measurement::read_tally(&o, &unfinished, Op::Multiply), Err(MeasureError::Incomplete("finished run")));
    assert_eq!(Measurement::read_runs(&unfinished), Err(MeasureError::Incomplete("finished run")));
}

/// The plan for the engine is `plan`'s: every member, both ops, both registerings, the same plan or the same refusal.
#[test]
fn the_plan_for_the_engine_is_the_plan() {
    for s in family() {
        for (op, latency) in [(Op::Multiply, 3), (Op::Add, 4)] {
            for (full, least) in [(1, 1), (latency + 1, 1), (latency + 1, latency + 1)] {
                let b = Block { semantics: s, full, least };
                assert_eq!(plan_for(op, &b, latency, &Semantics::ENGINE), plan(op, &b, latency), "{} {full}/{least}", s.name());
            }
        }
    }
    let card = named("din-flush.dout-flush.tiny-after-unbounded.sign-kept.nan-xor-7fc00000");
    let refused = plan(Op::Multiply, &Block { semantics: card, full: 4, least: 1 }, 3).unwrap_err().to_string();
    assert_eq!(
        refused,
        "the block's multiply (din-flush.dout-flush.tiny-after-unbounded.sign-kept.nan-xor-7fc00000) decides a result tiny before the engine does, \
         and flushes what the engine would round into the least normal",
        "the words fp-oracle6's receipt recorded"
    );
}

/// A block planned for its own semantics needs nothing around it: every member stands in for itself.
#[test]
fn a_block_planned_for_its_own_semantics_needs_nothing_around_it() {
    for s in family() {
        let p = plan_for(Op::Multiply, &Block { semantics: s, full: 4, least: 1 }, 3, &s).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(p, Plan { registering: Registering::Least, pads: 2, flush_operands: false, flush_results: false, canonical_nan: false }, "{}", s.name());
    }
    let card = named("din-flush.dout-flush.tiny-after-unbounded.sign-kept.nan-xor-7fc00000");
    let p = plan_for(Op::Multiply, &Block { semantics: card, full: 4, least: 1 }, 3, &card).unwrap();
    assert_eq!((p.registering, p.pads), (Registering::Least, 2), "the card's multiply at the engine's three clocks: its output register alone");
}

/// For another target the plan corrects what logic can and refuses the rest by name, the target named.
#[test]
fn a_plan_for_another_target_corrects_what_logic_can_and_refuses_the_rest_by_name() {
    let card = named("din-flush.dout-flush.tiny-after-unbounded.sign-kept.nan-xor-7fc00000");
    let ieee_like = named("din-keep.dout-keep.nan-7fc00000");
    let why = |b: Semantics, target: &Semantics| match plan_for(Op::Multiply, &Block { semantics: b, full: 1, least: 1 }, 3, target) {
        Err(HardError::Semantics { why, .. }) => why,
        other => panic!("{other:?}"),
    };
    assert_eq!(why(Semantics::ENGINE, &ieee_like), "reads a denormal operand as a zero, which din-keep.dout-keep.nan-7fc00000 reads as itself");
    let keeps_in = named("din-keep.dout-flush.tiny-after-denormals.sign-kept.nan-7fc00000");
    assert_eq!(why(keeps_in, &ieee_like), "flushes a result din-keep.dout-keep.nan-7fc00000 keeps as a denormal");
    assert_eq!(why(Semantics::ENGINE, &card), format!("rounds into the least normal a result {} decides tiny and flushes", card.name()));
    let before = named("din-flush.dout-flush.tiny-before.sign-kept.nan-xor-7fc00000");
    assert!(why(before, &card).starts_with("decides a result tiny before din-flush.dout-flush.tiny-after-unbounded"));
    let positive = named("din-flush.dout-flush.tiny-after-unbounded.sign-pos.nan-xor-7fc00000");
    assert_eq!(why(positive, &card), format!("flushes a tiny result to +0 whatever its sign, which {} keeps", card.name()));
    let canonical = named("din-flush.dout-flush.tiny-after-unbounded.sign-kept.nan-7fc00000");
    assert_eq!(why(canonical, &card), format!("returns NaNs {} does not, and the logic after it writes only 0x7fc00000", card.name()));
    let keeps_out = named("din-keep.dout-keep.nan-7fc00000");
    assert_eq!(why(keeps_out, &card), format!("keeps a denormal result, which flushed after it is no result {} decides tiny", card.name()));
    let p = plan_for(Op::Multiply, &Block { semantics: card, full: 1, least: 1 }, 3, &Semantics::ENGINE);
    assert!(p.is_err(), "the card's multiply is not the engine's");
    let corrected = plan_for(Op::Multiply, &Block { semantics: keeps_out, full: 1, least: 1 }, 3, &Semantics::ENGINE).unwrap();
    assert!(corrected.flush_operands && corrected.flush_results && !corrected.canonical_nan, "a block keeping denormals made the engine's");
}

/// The wrappers for the engine are the wrappers `hard_verilog` and `hard_verilog_ce` write, byte for byte, on every
/// member; for the card's multiply planned for itself, a wrapper with nothing around the block, named for the card's
/// arithmetic.
#[test]
fn the_wrapper_for_the_engine_is_the_wrapper_and_one_for_the_card_has_nothing_around_its_block() {
    for s in family() {
        for (op, latency) in [(Op::Multiply, 3), (Op::Add, 4)] {
            let b = Block { semantics: s, full: latency, least: 1 };
            assert_eq!(hard_verilog_for(op, "w", &b, latency, &Semantics::ENGINE), hard_verilog(op, "w", &b, latency), "{}", s.name());
            assert_eq!(hard_verilog_ce_for(op, "w", &b, latency, &Semantics::ENGINE), hard_verilog_ce(op, "w", &b, latency), "{}", s.name());
        }
    }
    let card = named("din-flush.dout-flush.tiny-after-unbounded.sign-kept.nan-xor-7fc00000");
    let v = hard_verilog_for(Op::Multiply, "card_mul", &Block { semantics: card, full: 4, least: 1 }, 3, &card).unwrap();
    assert!(v.starts_with(&format!("// The f32 multiply of {} on the card's floating-point DSP block, 3 clocks: y is that multiply", card.name())), "{v}");
    assert!(v.contains("with its output register alone, 1 deep, then 2 registers.\nmodule card_mul("), "{v}");
    assert!(!v.contains("Around it") && !v.contains("side_c"), "nothing around the block: {v}");
    let keeps_in = named("din-keep.dout-flush.tiny-after-unbounded.sign-kept.nan-xor-7fc00000");
    let w = hard_verilog_for(Op::Multiply, "m", &Block { semantics: keeps_in, full: 3, least: 1 }, 3, &card).unwrap();
    assert!(w.contains("? {a[31] ^ b[31], 31'h7fc00000} : {a[31] ^ b[31], 31'd0}};"), "a denormal operand decided beside the block with the card's NaN: {w}");
    assert!(hard_verilog_for(Op::Multiply, "m", &Block { semantics: card, full: 4, least: 1 }, 3, &Semantics::ENGINE).is_err(), "the card's multiply is not the engine's");
}
