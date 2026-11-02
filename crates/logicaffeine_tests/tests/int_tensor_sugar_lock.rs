//! int_tensor_sugar_lock — the integer tensor forms an int8 model is written in, each the arithmetic the model's
//! reference does, exactly, on every tier that runs it.
//!
//! An int8 transformer moves between layers through three integer steps the float forms cannot say: a requant
//! `(x · M) >> S` (an arithmetic shift, so a negative accumulator rounds toward −∞), a saturation into int8 or into a
//! table's index range, and a ReLU over integers. Each is a sugar the parser desugars into ordinary LOGOS loops, so the
//! VM's result IS the form's meaning; this lock holds that meaning to an independent Rust statement of the reference
//! arithmetic (`benchmarks/silicon/tiny_transformer.json`'s, as `scripts/transformer-on-fabric.py` computes it), the
//! recognizer to lifting each form to its typed spec, and the spatial evaluator to the VM on each — in `i64`, never
//! through a float.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test int_tensor_sugar_lock`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::passes::recognize_stages::{recognize_stages, TensorOpSpec};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;

#[path = "common/spatial.rs"]
mod spatial_support;
use spatial_support::inputs_from_fills;

/// A deterministic spread of int values over `[lo, hi]`, both ends included.
fn values(n: usize, lo: i64, hi: i64) -> Vec<i64> {
    let span = (hi - lo + 1) as u64;
    let mut out: Vec<i64> = (0..n as u64)
        .map(|i| lo + ((i.wrapping_mul(2_654_435_761) ^ (i >> 3)) % span) as i64)
        .collect();
    if n >= 2 {
        out[0] = lo;
        out[n - 1] = hi;
    }
    out
}

/// `Let X be a R by C tensor of Int.` with its items set, then `body`, then every item of `out` shown.
fn program(x: &[i64], (r, c): (usize, usize), body: &str, out: &str) -> String {
    assert_eq!(x.len(), r * c);
    let mut s = format!("## Main\nLet X be a {r} by {c} tensor of Int.\n");
    for (i, v) in x.iter().enumerate() {
        if *v != 0 {
            s.push_str(&format!("Set item {} of X to {v}.\n", i + 1));
        }
    }
    s.push_str(body);
    s.push_str(&format!(
        "Repeat for k from 1 to {}:\n    Show item k of {out}.\n",
        r * c
    ));
    s
}

fn vm_ints(src: &str) -> Vec<i64> {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "the VM refused:\n{src}\n{:?}", vm.error);
    vm.output
        .lines()
        .map(|l| {
            l.trim()
                .parse()
                .unwrap_or_else(|_| panic!("not an Int: {l:?}"))
        })
        .collect()
}

/// The integer fabric the decode step is built for: its accumulator holds a whole flat row of the tiny model's two
/// heads over its 24-position cache (48 scalar tiles) — the depth is a parameter of the silicon, sized to the model.
fn decode_fabric() -> SpatialTarget {
    SpatialTarget::sky_fabric(16, 64)
}

/// The program without its trailing `Show` loop: what the spatial front end lowers.
fn lowered(src: &str) -> SpatialProgram {
    let body = &src[..src.find("Repeat for k from 1 to").expect("the show loop")];
    let l = compile_program_to_spatial(body, &decode_fabric()).expect("a tensor program");
    l.program.unwrap_or_else(|| {
        panic!(
            "the program lowers whole and certifies: {:?}\n{body}",
            l.missing.first()
        )
    })
}

fn spatial_ints(src: &str, out: &str) -> Vec<i64> {
    let p = lowered(src);
    let got = eval_choreography(&p, &inputs_from_fills(&p)).expect("the program evaluates");
    let t = p
        .tensors
        .iter()
        .find(|t| t.name == out)
        .unwrap_or_else(|| panic!("{out} is a tensor of the program"))
        .id;
    match got.tensors.get(&t).expect("written") {
        Value::Int(v) => v.clone(),
        Value::Float(v) => {
            panic!("{out} evaluated through floats: {v:?} — an Int tensor is evaluated in i64")
        }
    }
}

/// The reference requant: `(x · m) >> s`, an arithmetic shift of the exact product.
fn requant(x: i64, m: i64, s: u32) -> i64 {
    (x * m) >> s
}

const SHAPE: (usize, usize) = (3, 8);

// ── REQUANT ──────────────────────────────────────────────────────────────────────────────────────────────────────

/// The tiny model's own multipliers and its 24-bit shift, over accumulators of both signs up to the int8 matmul's
/// reach: a negative product rounds toward −∞, as the reference's `>>` does.
#[test]
fn requant_is_the_reference_arithmetic_on_the_vm() {
    for (m, s) in [
        (104_593, 24),
        (21_076, 24),
        (16_998_164, 24),
        (7, 3),
        (1, 0),
    ] {
        let x = values(SHAPE.0 * SHAPE.1, -258_064, 258_064);
        let src = program(
            &x,
            SHAPE,
            &format!("Let Y be X times {m} shifted right by {s}.\n"),
            "Y",
        );
        let want: Vec<i64> = x.iter().map(|&v| requant(v, m, s)).collect();
        assert_eq!(vm_ints(&src), want, "requant by {m} >> {s}");
    }
}

#[test]
fn requant_is_recognized_with_its_multiplier_and_shift() {
    let x = values(SHAPE.0 * SHAPE.1, -1000, 1000);
    let src = program(
        &x,
        SHAPE,
        "Let Y be X times 104593 shifted right by 24.\n",
        "Y",
    );
    let rp = recognize_stages(&src).expect("the program parses");
    assert!(
        rp.stages.iter().any(|st| st.out == "Y"
            && matches!(&st.op, TensorOpSpec::IntRequant(r) if r.m == 104_593 && r.s == 24)),
        "`times M shifted right by S` lifts to IntRequant{{m, s}}: {:?}",
        rp.stages
    );
}

#[test]
fn requant_on_the_spatial_evaluator_is_the_vms() {
    for (m, s) in [(104_593, 24), (7, 3)] {
        let x = values(SHAPE.0 * SHAPE.1, -258_064, 258_064);
        let src = program(
            &x,
            SHAPE,
            &format!("Let Y be X times {m} shifted right by {s}.\n"),
            "Y",
        );
        assert_eq!(
            spatial_ints(&src, "Y"),
            vm_ints(&src),
            "requant by {m} >> {s}"
        );
    }
}

// ── SATURATION ───────────────────────────────────────────────────────────────────────────────────────────────────

/// Into int8 and into a 256-entry table's index range, with values below, inside, at and above each bound.
#[test]
fn clamp_saturates_into_its_bounds_on_the_vm() {
    for (lo, hi) in [(-128, 127), (0, 255), (-1, 1)] {
        let x = values(SHAPE.0 * SHAPE.1, lo - 300, hi + 300);
        let src = program(
            &x,
            SHAPE,
            &format!("Let Y be X clamped between {lo} and {hi}.\n"),
            "Y",
        );
        let want: Vec<i64> = x.iter().map(|&v| v.max(lo).min(hi)).collect();
        assert_eq!(vm_ints(&src), want, "clamp to [{lo}, {hi}]");
    }
}

#[test]
fn clamp_is_recognized_with_its_bounds() {
    let x = values(SHAPE.0 * SHAPE.1, -500, 500);
    let src = program(&x, SHAPE, "Let Y be X clamped between -128 and 127.\n", "Y");
    let rp = recognize_stages(&src).expect("the program parses");
    assert!(
        rp.stages.iter().any(|st| st.out == "Y"
            && matches!(&st.op, TensorOpSpec::IntClamp(c) if c.lo == -128 && c.hi == 127)),
        "`clamped between LO and HI` lifts to IntClamp{{lo, hi}}: {:?}",
        rp.stages
    );
}

#[test]
fn clamp_on_the_spatial_evaluator_is_the_vms() {
    let x = values(SHAPE.0 * SHAPE.1, -500, 500);
    let src = program(&x, SHAPE, "Let Y be X clamped between -128 and 127.\n", "Y");
    assert_eq!(spatial_ints(&src, "Y"), vm_ints(&src));
}

/// A bound pair that leaves nothing between is not a clamp: the parser refuses it rather than desugaring a constant.
#[test]
fn an_empty_clamp_range_is_refused() {
    let x = values(SHAPE.0 * SHAPE.1, -5, 5);
    let src = program(&x, SHAPE, "Let Y be X clamped between 5 and -5.\n", "Y");
    let vm = vm_outcome(&src);
    assert!(
        vm.error
            .as_deref()
            .is_some_and(|e| e.contains("clamped between")),
        "an empty range is refused by name: {:?}",
        vm.error
    );
}

// ── RELU ─────────────────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn relu_over_ints_keeps_the_positive_part_on_the_vm() {
    let x = values(SHAPE.0 * SHAPE.1, -40_000, 40_000);
    let src = program(&x, SHAPE, "Let Y be relu of X.\n", "Y");
    let want: Vec<i64> = x.iter().map(|&v| v.max(0)).collect();
    assert_eq!(vm_ints(&src), want);
}

#[test]
fn relu_is_recognized() {
    let x = values(SHAPE.0 * SHAPE.1, -40, 40);
    let src = program(&x, SHAPE, "Let Y be relu of X.\n", "Y");
    let rp = recognize_stages(&src).expect("the program parses");
    assert!(
        rp.stages
            .iter()
            .any(|st| st.out == "Y" && matches!(&st.op, TensorOpSpec::Relu(_))),
        "`relu of X` lifts to Relu: {:?}",
        rp.stages
    );
}

#[test]
fn relu_on_the_spatial_evaluator_is_the_vms() {
    let x = values(SHAPE.0 * SHAPE.1, -40_000, 40_000);
    let src = program(&x, SHAPE, "Let Y be relu of X.\n", "Y");
    assert_eq!(spatial_ints(&src, "Y"), vm_ints(&src));
}

// ── THE THREE COMPOSED: the tiny model's `to8`, and its FFN hidden step ─────────────────────────────────────────────

/// `to8(x, M) = clamp((x · M) >> 24, −128, 127)` — the reference's saturating requant, as two statements.
#[test]
fn the_saturating_requant_composes_on_every_tier() {
    let x = values(SHAPE.0 * SHAPE.1, -258_064, 258_064);
    let body = "Let Y be X times 16777216 shifted right by 24.\nLet Z be Y clamped between -128 and 127.\n";
    let src = program(&x, SHAPE, body, "Z");
    let want: Vec<i64> = x
        .iter()
        .map(|&v| requant(v, 16_777_216, 24).clamp(-128, 127))
        .collect();
    assert_eq!(vm_ints(&src), want, "the VM");
    assert_eq!(spatial_ints(&src, "Z"), want, "the spatial evaluator");
}

/// `hid = max((acc · M_h) >> 24, 0)` — requant then ReLU, the FFN's hidden activation.
#[test]
fn the_ffn_hidden_activation_composes_on_every_tier() {
    let x = values(SHAPE.0 * SHAPE.1, -258_064, 258_064);
    let body = "Let Y be X times 104471 shifted right by 24.\nLet H be relu of Y.\n";
    let src = program(&x, SHAPE, body, "H");
    let want: Vec<i64> = x.iter().map(|&v| requant(v, 104_471, 24).max(0)).collect();
    assert_eq!(vm_ints(&src), want, "the VM");
    assert_eq!(spatial_ints(&src, "H"), want, "the spatial evaluator");
}

/// The forms are integer forms: over a Real tensor the parser refuses them by name rather than truncating.
#[test]
fn the_integer_forms_refuse_a_real_tensor() {
    for body in [
        "Let Y be R times 3 shifted right by 1.\n",
        "Let Y be R clamped between -1 and 1.\n",
    ] {
        let src = format!("## Main\nLet R be a 2 by 2 tensor of Real.\n{body}Show item 1 of Y.\n");
        let vm = vm_outcome(&src);
        assert!(
            vm.error
                .as_deref()
                .is_some_and(|e| e.contains("tensor of Int")),
            "{body:?} over a Real tensor is refused by name: {:?}",
            vm.error
        );
    }
}

// ── LUT ATTENTION: the int8 model's softmax, as the reference computes it ──────────────────────────────────────────

/// The reference decode attention over the first `upto` cached rows, per query head `g` (kv head `g / (h / kv)`):
/// scores `s_j = q_g · k_j`, `mx = max_j s_j`, LUT index `clamp(((mx − s_j) · m) >> s, 0, len − 1)`, weight
/// `w_j = table[idx]`, then `ctx_g = ⌊Σ_j w_j · v_j / Σ_j w_j⌋` — floor division, as the reference's `//`.
#[allow(clippy::too_many_arguments)]
fn lut_attention(
    q: &[i64],
    k: &[i64],
    v: &[i64],
    table: &[i64],
    (h, kv, hd): (usize, usize, usize),
    upto: usize,
    m: i64,
    s: u32,
) -> Vec<i64> {
    let group = h / kv;
    let mut out = vec![0; h * hd];
    for g in 0..h {
        let kh = g / group;
        let score = |j: usize| -> i64 {
            (0..hd)
                .map(|c| q[g * hd + c] * k[j * kv * hd + kh * hd + c])
                .sum()
        };
        let mx = (0..upto).map(score).max().expect("at least one position");
        let mut num = vec![0i64; hd];
        let mut den = 0i64;
        for j in 0..upto {
            let idx = requant(mx - score(j), m, s).clamp(0, table.len() as i64 - 1);
            let w = table[idx as usize];
            for c in 0..hd {
                num[c] += w * v[j * kv * hd + kh * hd + c];
            }
            den += w;
        }
        for c in 0..hd {
            out[g * hd + c] = num[c].div_euclid(den);
        }
    }
    out
}

struct Attn {
    h: usize,
    kv: usize,
    hd: usize,
    l: usize,
    upto: usize,
    m: i64,
    s: u32,
}

impl Attn {
    fn tensors(&self, seed: usize) -> (Vec<i64>, Vec<i64>, Vec<i64>) {
        let q = values(self.h * self.hd + seed, -128, 127)[seed..].to_vec();
        let k =
            values(self.l * self.kv * self.hd + 2 * seed + 1, -128, 127)[2 * seed + 1..].to_vec();
        let v =
            values(self.l * self.kv * self.hd + 3 * seed + 2, -128, 127)[3 * seed + 2..].to_vec();
        (q, k, v)
    }

    /// The program: Q, K, V and the table declared and filled, then `C` by the sugar, then every item of C shown.
    fn program(&self, q: &[i64], k: &[i64], v: &[i64], table: &[i64]) -> String {
        let (qw, kw) = (self.h * self.hd, self.kv * self.hd);
        let mut s = format!(
            "## Main\nLet Q be a 1 by {qw} tensor of Int.\nLet K be a {l} by {kw} tensor of Int.\n\
             Let V be a {l} by {kw} tensor of Int.\nLet E be a 1 by {n} tensor of Int.\n",
            l = self.l,
            n = table.len()
        );
        for (name, vals) in [("Q", q), ("K", k), ("V", v), ("E", table)] {
            for (i, x) in vals.iter().enumerate() {
                if *x != 0 {
                    s.push_str(&format!("Set item {} of {name} to {x}.\n", i + 1));
                }
            }
        }
        s.push_str(&format!("Let pos be {}.\n", self.upto));
        s.push_str(&format!(
            "Let C be lut attention of Q over K and V with {} heads {} groups up to pos with table E and scale {} shifted right by {}.\n",
            self.h, self.kv, self.m, self.s
        ));
        s.push_str(&format!(
            "Repeat for k from 1 to {qw}:\n    Show item k of C.\n"
        ));
        s
    }
}

/// A decaying table shaped like the tiny model's exp LUT (32768 at index 0, each entry 7/8 of the last, floored, never
/// below 1), in integers so it is the same table on every machine: every row's maximum weighs 32768 and a denominator
/// is never 0.
fn exp_table(n: usize) -> Vec<i64> {
    let mut t = vec![32768i64];
    while t.len() < n {
        t.push((t[t.len() - 1] * 7 / 8).max(1));
    }
    t
}

const TINY: Attn = Attn {
    h: 2,
    kv: 2,
    hd: 8,
    l: 24,
    upto: 24,
    m: 21_076,
    s: 24,
};

#[test]
fn lut_attention_is_the_reference_on_the_vm_at_every_position() {
    let table = exp_table(256);
    for upto in [1, 2, 7, 23, 24] {
        let a = Attn { upto, ..TINY };
        let (q, k, v) = a.tensors(upto);
        let want = lut_attention(&q, &k, &v, &table, (a.h, a.kv, a.hd), upto, a.m, a.s);
        assert_eq!(
            vm_ints(&a.program(&q, &k, &v, &table)),
            want,
            "up to {upto}"
        );
    }
}

/// Grouped heads read their shared kv head, and a scale that sends every index past the table clamps to its last entry.
#[test]
fn lut_attention_groups_heads_and_clamps_its_index() {
    let table = exp_table(16);
    for a in [
        Attn {
            h: 4,
            kv: 2,
            hd: 4,
            l: 6,
            upto: 5,
            m: 21_076,
            s: 24,
        },
        Attn {
            h: 4,
            kv: 1,
            hd: 2,
            l: 3,
            upto: 3,
            m: 1 << 20,
            s: 8,
        },
        Attn {
            h: 1,
            kv: 1,
            hd: 3,
            l: 2,
            upto: 1,
            m: 1,
            s: 0,
        },
    ] {
        let (q, k, v) = a.tensors(a.h + a.l);
        let want = lut_attention(&q, &k, &v, &table, (a.h, a.kv, a.hd), a.upto, a.m, a.s);
        assert_eq!(
            vm_ints(&a.program(&q, &k, &v, &table)),
            want,
            "{} heads over {} groups, up to {}",
            a.h,
            a.kv,
            a.upto
        );
    }
}

/// Every numerator negative: floor division rounds toward −∞, which truncation would not.
#[test]
fn lut_attention_floors_a_negative_numerator() {
    let table = exp_table(256);
    let a = Attn {
        h: 1,
        kv: 1,
        hd: 4,
        l: 4,
        upto: 4,
        m: 21_076,
        s: 24,
    };
    let (q, k, _) = a.tensors(3);
    let v: Vec<i64> = (0..a.l * a.hd).map(|i| -1 - (i as i64 % 5)).collect();
    let want = lut_attention(&q, &k, &v, &table, (a.h, a.kv, a.hd), a.upto, a.m, a.s);
    assert!(want.iter().all(|&c| c < 0));
    assert_eq!(
        vm_ints(&a.program(&q, &k, &v, &table)),
        want,
        "the VM floors"
    );
    assert_eq!(
        spatial_ints(&a.program(&q, &k, &v, &table), "C"),
        want,
        "the spatial evaluator floors in i64"
    );
}

#[test]
fn lut_attention_is_recognized_with_its_shape_table_and_scale() {
    let table = exp_table(256);
    let (q, k, v) = TINY.tensors(1);
    let rp = recognize_stages(&TINY.program(&q, &k, &v, &table)).expect("the program parses");
    assert!(
        rp.stages.iter().any(|st| st.out == "C"
            && matches!(&st.op, TensorOpSpec::LutAttention(a)
                if a.h == 2 && a.kv == 2 && a.hd == 8 && a.l == 24 && a.table_len == 256
                    && a.m == 21_076 && a.s == 24 && a.pos.is_some())),
        "`lut attention of …` lifts to LutAttention with its shape, table and scale: {:?}",
        rp.stages
    );
}

#[test]
fn lut_attention_on_the_spatial_evaluator_is_the_vms_at_every_position() {
    let table = exp_table(256);
    for upto in [1, 7, 24] {
        let a = Attn { upto, ..TINY };
        let (q, k, v) = a.tensors(upto);
        let src = a.program(&q, &k, &v, &table);
        assert_eq!(spatial_ints(&src, "C"), vm_ints(&src), "up to {upto}");
    }
}

/// A head row longer than the fabric's accumulator is refused by name — the integer sections do not block a row yet
/// — never lowered into something that computes another answer.
#[test]
fn lut_attention_past_the_accumulator_is_refused_by_name() {
    let table = exp_table(256);
    let a = Attn { upto: 7, ..TINY };
    let (q, k, v) = a.tensors(7);
    let src = a.program(&q, &k, &v, &table);
    let body = &src[..src.find("Repeat for k from 1 to").expect("the show loop")];
    let l = compile_program_to_spatial(body, &SpatialTarget::sky_fabric(16, 16)).expect("a tensor program");
    assert!(l.program.is_none(), "48 positions do not fit a 16-tile accumulator");
    let why = format!("{:?}", l.missing.first());
    assert!(
        why.contains("LutAttention") && why.contains("AccTilesExceeded"),
        "refused by name, naming the accumulator: {why}"
    );
}

/// A position past the cache is refused when it is read, as the float form's `up to` is.
#[test]
fn lut_attention_refuses_a_position_past_the_cache() {
    let table = exp_table(8);
    let a = Attn {
        h: 1,
        kv: 1,
        hd: 2,
        l: 3,
        upto: 4,
        m: 1,
        s: 0,
    };
    let (q, k, v) = a.tensors(0);
    let vm = vm_outcome(&a.program(&q, &k, &v, &table));
    assert!(vm.error.is_some(), "up to 4 over a 3-row cache is refused");
}
