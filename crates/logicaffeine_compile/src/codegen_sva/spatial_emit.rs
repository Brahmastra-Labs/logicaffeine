//! **Spatial source emission** for a counted loop — the bridge from the certified fabric IR to the open
//! Plasticine toolchain (Spatial → PIR → plastiroute → Tungsten; Koeplinger et al., PLDI 2018; the chain is
//! mirrored at gitlab.com/dataplane-ai). P4 of the Plasticine lane.
//!
//! A [`LoopFabric`] is one `Foreach` controller: its carried state registers become `Reg`s, the induction
//! variable the counter, the body cells `val`s in SSA order, a predicated select a `mux`, and the next-state
//! sources the register writes at the end of the iteration (computed first, assigned after — the parallel
//! register-update semantics the fabric has). Every LOGOS `Int` is a 64-bit `Long`. What this emitter does
//! not spell (sequence tiles, heap islands, the wider `FuncUnit` family) is refused TYPED so a program never
//! reaches the toolchain with silently different meaning.

use super::fabric_rtl::{AluOp, CmpKind, FuncUnit, LoopCell, LoopFabric, Src};
use crate::tensor_recognize::{Axis, DagNode, Fill, Stop, TermDag, Ty, Scalar, ZipOp, ZipTerm};
use std::collections::HashMap;

/// A LOGOS `Int` constant as a Spatial `Long`: an `Int`-sized literal is cast, a wider one is a Scala `Long`
/// literal lifted by Spatial's `FixFromLong` (a bare `123456789012.to[Long]` is "integer number too large").
fn lit(c: i64) -> String {
    if i32::try_from(c).is_ok() {
        if c < 0 {
            format!("({c}).to[Long]")
        } else {
            format!("{c}.to[Long]")
        }
    } else {
        format!("({c}L: Long)")
    }
}

fn alu(op: AluOp, a: &str, b: &str) -> String {
    match op {
        AluOp::Add => format!("{a} + {b}"),
        AluOp::Sub => format!("{a} - {b}"),
        AluOp::Mul => format!("{a} * {b}"),
        AluOp::And => format!("{a} & {b}"),
        AluOp::Or => format!("{a} | {b}"),
        AluOp::Xor => format!("{a} ^ {b}"),
        AluOp::Shl => format!("{a} << ({b} & 63.to[Long]).to[Int]"),
        AluOp::Shr => format!("{a} >> ({b} & 63.to[Long]).to[Int]"),
        AluOp::UShr => format!("{a} >>> ({b} & 63.to[Long]).to[Int]"),
    }
}

fn cmp(c: CmpKind) -> &'static str {
    match c {
        CmpKind::Lt => "<",
        CmpKind::Gt => ">",
        CmpKind::Le => "<=",
        CmpKind::Ge => ">=",
        CmpKind::Eq => "===",
        CmpKind::Ne => "=!=",
    }
}

/// The `Accel { … }` block for the counted loop `lf` (shared by the app and the test flavours), indented by
/// `pad`, or a typed reason it cannot be spelled. `arg_in` names circuit input `k` as the host reads it.
fn accel_block(
    lf: &LoopFabric,
    pad: &str,
    arg_in: &dyn Fn(usize) -> String,
    const_params: Option<&[i64]>,
) -> Result<String, String> {
    if lf.seq_input.is_some() || lf.seq_heap_ptr.is_some() {
        return Err("sequence loops (`Repeat for x in xs`) need the DRAM → PMU tile-load stage, which this emitter does not spell yet".into());
    }
    if !lf.epilogue.is_empty() || !lf.prologue_heap.is_empty() {
        return Err(
            "a post-loop epilogue / prologue heap island is not spelled by this emitter yet".into(),
        );
    }
    let body_in = |k: usize| -> String {
        if k < lf.n_state {
            format!("c{k}")
        } else if k == lf.n_state {
            "iv".to_string()
        } else {
            format!("inv{}", k - lf.n_state - 1)
        }
    };
    let over_circuit = |s: &Src| match s {
        Src::In(k) => arg_in(*k),
        Src::Val(v) => format!("v{v}"),
        Src::Const(c) => lit(*c),
    };
    let over_body = |s: &Src| match s {
        Src::In(k) => body_in(*k),
        Src::Val(v) => format!("v{v}"),
        Src::Const(c) => lit(*c),
    };
    let mut body = Vec::new();
    for (i, cell) in lf.body.iter().enumerate() {
        let rhs = match cell {
            LoopCell::Arith { op, a, b } => alu(*op, &over_body(a), &over_body(b)),
            LoopCell::Select {
                cmp: c,
                ca,
                cb,
                t,
                e,
            } => {
                format!(
                    "mux({} {} {}, {}, {})",
                    over_body(ca),
                    cmp(*c),
                    over_body(cb),
                    over_body(t),
                    over_body(e)
                )
            }
            LoopCell::Func {
                unit: FuncUnit::DivS,
                a,
                b,
            } => format!("{} / {}", over_body(a), over_body(b)),
            LoopCell::Func {
                unit: FuncUnit::ModS,
                a,
                b,
            } => format!("{} % {}", over_body(a), over_body(b)),
            LoopCell::Func { unit, .. } => {
                return Err(format!(
                    "functional unit `{unit:?}` has no Spatial spelling in this emitter yet"
                ));
            }
            LoopCell::MemRead { .. }
            | LoopCell::HeapRead { .. }
            | LoopCell::HeapWrite { .. }
            | LoopCell::HeapAlloc { .. } => {
                return Err("memory tiles (sequence / heap) need a PMU-resident buffer this emitter does not spell yet".into());
            }
            LoopCell::VectorArith { .. } => {
                return Err("packed-SIMD tiles are not spelled by this emitter yet".into());
            }
        };
        body.push(format!("{pad}    val v{i} = {rhs}"));
    }
    // Loop bounds: a constant is a bare `Int` literal; an input bound is the baked parameter when the block is
    // specialized (the regression flavour — Spatial then proves every iteration whole and emits no
    // counter-valid enable on the carried write, which PIR's Tungsten codegen cannot type), else narrowed.
    let bound = |s: &Src| match s {
        Src::Const(c) => c.to_string(),
        Src::In(k) if const_params.is_some() => const_params.unwrap()[*k].to_string(),
        other => format!("({}).to[Int]", over_circuit(other)),
    };
    // Carried registers take the DEFAULT init (zero); the LOGOS init is PEELED into the first iteration —
    // each state read is `mux(iv == lo, init, s.value)` — so the loop is one self-contained controller with
    // uniform read counts. PIR refuses the alternatives: a register constructed with a non-zero init
    // ("inits already has value Some(0), but reupdate to 1" in its RewriteTransformer), a bare init write
    // before the loop ("Don't know how to infer vec of MemWrite…_en" in its Tungsten codegen), and an init
    // written from a one-trip loop (its RuntimeAnalyzer asserts every read in a context has one trip count).
    let lo = over_circuit(&lf.lo);
    let mut out = String::new();
    out.push_str(&format!("{pad}Accel {{\n"));
    for (j, inv) in lf.invariants.iter().enumerate() {
        out.push_str(&format!("{pad}  val inv{j} = {}\n", over_circuit(inv)));
    }
    for k in 0..lf.n_state {
        out.push_str(&format!("{pad}  val s{k} = Reg[Long]\n"));
    }
    out.push_str(&format!(
        "{pad}  Foreach({} until {} + 1) {{ i =>\n{pad}    val iv = i.to[Long]\n",
        bound(&lf.lo),
        bound(&lf.hi)
    ));
    for (k, init) in lf.init.iter().enumerate() {
        out.push_str(&format!(
            "{pad}    val c{k} = mux(iv === {lo}, {}, s{k}.value)\n",
            over_circuit(init)
        ));
    }
    for line in &body {
        out.push_str(line);
        out.push('\n');
    }
    for (k, next) in lf.next.iter().enumerate() {
        out.push_str(&format!("{pad}    val n{k} = {}\n", over_body(next)));
    }
    for k in 0..lf.n_state {
        out.push_str(&format!("{pad}    s{k} := n{k}\n"));
    }
    out.push_str(&format!("{pad}  }}\n"));
    out.push_str(&format!("{pad}  out := s{}.value\n", lf.result_state));
    out.push_str(&format!("{pad}}}\n"));
    Ok(out)
}

/// Emit a Spatial REGRESSION-HARNESS app (`class <class_name> extends SpatialTest`) for the counted loop: the
/// sample `params` are its `runtimeArgs`, the VM's answer `gold` is checked on the host, and the harness's
/// `PASS: true (<class_name>)` line is what the toolchain ledger parses. Values are 64-bit `Long`: PIR's
/// Tungsten codegen lowers `Fix(true,64,0)` to C `long`, so the simulation computes what the VM computes
/// (the ISCA'17 FU is 32-bit; a silicon run would need the narrower type).
pub fn emit_spatial_counted_loop_test(
    lf: &LoopFabric,
    class_name: &str,
    params: &[i64],
    gold: i64,
) -> Result<String, String> {
    if lf.n_in != params.len() {
        return Err(format!(
            "{} parameters but {} runtime args",
            lf.n_in,
            params.len()
        ));
    }
    let accel = accel_block(lf, "    ", &|k| format!("in{k}.value"), Some(params))?;
    let mut out = String::new();
    out.push_str(&format!(
        "// Generated by LOGOS (logicaffeine_compile::codegen_sva::spatial_emit) — Spatial regression app for the\n// counted loop `{class_name}`: runtime args {params:?} (baked into the loop bounds), expected {gold} (the LOGOS VM's answer).\n"
    ));
    out.push_str("import spatial.dsl._\n\n");
    out.push_str(&format!(
        "@spatial class {class_name} extends SpatialTest {{\n"
    ));
    out.push_str(&format!(
        "  override def runtimeArgs: Args = \"{}\"\n\n",
        params
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    ));
    out.push_str("  def main(args: Array[String]): Unit = {\n");
    for k in 0..lf.n_in {
        out.push_str(&format!(
            "    val in{k} = ArgIn[Long]\n    setArg(in{k}, args({k}).to[Long])\n"
        ));
    }
    out.push_str("    val out = ArgOut[Long]\n");
    out.push_str(&accel);
    out.push_str(&format!("    val gold = {}\n", lit(gold)));
    out.push_str("    val result = getArg(out)\n");
    out.push_str("    val cksum = result == gold\n");
    out.push_str(&format!("    println(r\"result $result gold $gold\")\n    println(\"PASS: \" + cksum + \" ({class_name})\")\n    assert(cksum)\n"));
    out.push_str("  }\n}\n");
    Ok(out)
}

/// Emit Spatial source for the counted loop `lf` of LOGOS function `fn_name` as a standalone `SpatialApp`
/// (arguments from the command line, the result printed), or a typed reason it cannot.
pub fn emit_spatial_counted_loop(lf: &LoopFabric, fn_name: &str) -> Result<String, String> {
    let accel = accel_block(lf, "    ", &|k| format!("in{k}.value"), None)?;
    let mut out = String::new();
    out.push_str(&format!(
        "// Generated by LOGOS (logicaffeine_compile::codegen_sva::spatial_emit) from `{fn_name}` — Spatial source\n// (Koeplinger et al., PLDI 2018) for the open Plasticine toolchain. LOGOS `Int` is a 64-bit `Long`.\n"
    ));
    out.push_str("import spatial.dsl._\n\n");
    out.push_str(&format!(
        "@spatial object Logos_{fn_name} extends SpatialApp {{\n"
    ));
    out.push_str("  def main(args: Array[String]): Unit = {\n");
    for k in 0..lf.n_in {
        out.push_str(&format!(
            "    val in{k} = ArgIn[Long]\n    setArg(in{k}, args({k}).to[Long])\n"
        ));
    }
    out.push_str("    val out = ArgOut[Long]\n");
    out.push_str(&accel);
    out.push_str(&format!("    println(r\"{fn_name} = ${{getArg(out)}}\")\n"));
    out.push_str("  }\n}\n");
    Ok(out)
}

/// Emit a Spatial REGRESSION-HARNESS GEMM app for `C[m×n] = A[m×k] · B[k×n]` — the structure of Spatial's own
/// `regression/GEMM.scala` (an output tile `its × ts` accumulated over `k` in `ts`-chunks by a `MemReduce`, an
/// `ip`-lane `Reduce` per dot product) with OUR shape and tiles, so a LOGOS matmul runs through the public
/// chain as the toolchain's authors would write it. Host data are small residues so the 32-bit sums stay
/// exact and the harness's `PASS` line means equality. Typed refusal when the tiles do not divide the shape
/// (the template streams whole tiles).
/// How wide each loop of the GEMM nest runs. Spatial's own regression variants set exactly these, and a
/// factor left at 1 is a loop we run serially where the hand-written app runs it wide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GemmUnroll {
    /// Outer tile loops over the output — rows and columns of tiles.
    pub ii: usize,
    pub jj: usize,
    /// The reduction over K-chunks.
    pub kk: usize,
    /// Inner element loops within a tile — rows and columns.
    pub i: usize,
    pub j: usize,
}

impl Default for GemmUnroll {
    /// Serial everywhere. The innermost dot is always `par ip`, which is a separate argument.
    fn default() -> Self {
        GemmUnroll {
            ii: 1,
            jj: 1,
            kk: 1,
            i: 1,
            j: 1,
        }
    }
}

impl GemmUnroll {
    /// The widest any SINGLE loop may be unrolled, and the largest TOTAL unroll across all of them.
    ///
    /// Both are measured frontiers, not resource derivations, and the distinction between them matters. A
    /// 16-wide column loop at ts=32 does not merely stop paying: PIR emitted over eighteen thousand Tungsten
    /// contexts and made no progress in forty minutes on a program that otherwise compiles in two. But that
    /// is a PER-LOOP effect — widening one loop 16 ways instantiates sixteen reduces fed by one operand
    /// buffer — and it does not say a product of 16 spread across four loops is unplaceable. Spatial's own
    /// variants confirm the split: they never widen a single loop past 3, and they freely combine three
    /// loops at 2 for a product of 8.
    ///
    /// So the search is bounded per loop by what stalled, and in total by what has been demonstrated to
    /// COMPLETE on the chain — which is 4, measured from both sides:
    ///
    ///   * their `GEMM_10` (loop_i=2 loop_j=2, product 4) simulates in minutes at 9,065 cycles;
    ///   * a product of 8 does not. Our own emission at kk=2 i=2 j=2 made no progress in sixteen minutes,
    ///     the same signature as the 16-wide single loop, and their `GEMM_13` (loop_ii=2 loop_jj=2, also
    ///     product 4 but on the OUTER tile loops) stalled at twenty-five minutes — which is why the outer
    ///     loops are ranked last: they replicate the whole tile pipeline rather than one reduce.
    ///
    /// Raise it when the parity corpus shows a wider configuration completing and winning. Both numbers are
    /// evidence, not intuition, and neither is tuned to a shape.
    pub const MAX_PER_LOOP: usize = 3;
    pub const MAX_TOTAL_UNROLL: usize = 4;

    /// Kept as the single knob the older call sites pass; the total is what bounds a whole configuration.
    pub const PLACEABLE_UNROLL: usize = Self::MAX_TOTAL_UNROLL;

    /// Every divisor of `extent` that is at most `cap`, largest first.
    fn divisors(extent: usize, cap: usize) -> Vec<usize> {
        (1..=extent.min(cap).max(1))
            .rev()
            .filter(|f| extent % f == 0)
            .collect()
    }

    /// Search the unroll space for the configuration that does the most parallel work per cycle within what
    /// the toolchain can place.
    ///
    /// The space is small — each factor divides its own loop extent — so this enumerates rather than
    /// guesses. Two facts order the search, both measured on the vendor's own simulator at 128^3, ts 64,
    /// its 64, ip 16, where the serial program costs 132,068 cycles:
    ///
    ///   * widening ANY single loop by 2 roughly halves the cycles — `loop_j` 66,587, `loop_i` 66,713,
    ///     `loop_kk` 67,700 — so total parallel work is what the objective should maximise;
    ///   * among equal products the column loop wins, because `j` iterations are independent dot products
    ///     over the SAME `a_sram`, while `i` reloads that buffer per iteration and `kk` adds a partial-sum
    ///     buffer per parallel chunk. The measured ordering j < i < kk is exactly that reasoning.
    ///
    /// So: maximise the product subject to the placement frontier, break ties by that ordering. No shape is
    /// named anywhere — the same rule runs for every tiling, and a shape it has never seen gets the same
    /// treatment as one in the corpus.
    pub fn search(ts: usize, its: usize, k_chunks: usize, budget: usize) -> GemmUnroll {
        Self::search_nest(ts, its, k_chunks, 1, 1, budget)
    }

    /// The full five-loop search. `m_tiles` and `n_tiles` are how many output tiles the shape has in each
    /// direction, which is what the outer `ii` / `jj` loops range over.
    pub fn search_nest(
        ts: usize,
        its: usize,
        k_chunks: usize,
        m_tiles: usize,
        n_tiles: usize,
        budget: usize,
    ) -> GemmUnroll {
        let budget = budget.max(1);
        let per = Self::MAX_PER_LOOP;
        let mut best = GemmUnroll::default();
        let mut best_key = (1usize, 0i32);
        for &j in &Self::divisors(ts, per) {
            for &i in &Self::divisors(its, per) {
                for &kk in &Self::divisors(k_chunks, per) {
                    for &jj in &Self::divisors(n_tiles, per) {
                        for &ii in &Self::divisors(m_tiles, per) {
                            let product = j * i * kk * jj * ii;
                            if product > budget {
                                continue;
                            }
                            // Tie-break among equal parallel work, by what each loop costs beyond the
                            // arithmetic. `j` iterations are independent dots over the SAME operand buffer.
                            // `i` reloads that buffer per iteration. `kk` adds a partial-sum buffer per
                            // parallel chunk. The outer tile loops replicate the whole tile pipeline and are
                            // the most expensive, so they are taken last. Measured at 128^3: j 66,587,
                            // i 66,713, kk 67,700 against 132,068 serial.
                            let pref = 4 * (j > 1) as i32
                                + 3 * (i > 1) as i32
                                + 2 * (kk > 1) as i32
                                + (jj > 1) as i32;
                            if (product, pref) > best_key {
                                best_key = (product, pref);
                                best = GemmUnroll { ii, jj, kk, i, j };
                            }
                        }
                    }
                }
            }
        }
        best
    }

    /// The compiler's choice for a tiling when the tile counts are not known to the caller.
    pub fn choose(ts: usize, its: usize, ip: usize) -> GemmUnroll {
        let _ = ip;
        Self::search(ts, its, 1, Self::MAX_TOTAL_UNROLL)
    }
}

pub fn emit_spatial_gemm_test(
    m: usize,
    n: usize,
    k: usize,
    ts: usize,
    its: usize,
    ip: usize,
    class_name: &str,
) -> Result<String, String> {
    // The tile counts the outer loops range over, and the K-chunk count the reduction ranges over, are all
    // known here — so the search sees the whole nest rather than the inner two loops.
    let u = GemmUnroll::search_nest(
        ts,
        its,
        k / ts.max(1),
        m / its.max(1),
        n / ts.max(1),
        GemmUnroll::MAX_TOTAL_UNROLL,
    );
    emit_spatial_gemm_test_with(m, n, k, ts, its, ip, u, class_name)
}

/// The same emission with the loop widths given explicitly — the form the parity corpus uses to reproduce a
/// Spatial variant's exact configuration.
pub fn emit_spatial_gemm_test_with(
    m: usize,
    n: usize,
    k: usize,
    ts: usize,
    its: usize,
    ip: usize,
    u: GemmUnroll,
    class_name: &str,
) -> Result<String, String> {
    if ts == 0 || its == 0 || ip == 0 {
        return Err("tiles and lanes are at least 1".into());
    }
    if m % its != 0 || n % ts != 0 || k % ts != 0 {
        return Err(format!("the Spatial GEMM template streams whole tiles: its={its} must divide m={m}, ts={ts} must divide n={n} and k={k}"));
    }
    if ts % ip != 0 {
        return Err(format!(
            "the {ip}-lane reduce must divide the k-chunk ts={ts}"
        ));
    }
    let mut o = String::new();
    o.push_str(&format!(
        "// Generated by LOGOS (logicaffeine_compile::codegen_sva::spatial_emit) from a LOGOS matmul recognized as\n// C[{m}x{n}] = A[{m}x{k}] · B[{k}x{n}]: Spatial's GEMM structure (regression/GEMM.scala) with our shape, tiles\n// its={its} ts={ts} and {ip}-lane reduce. Gold on the host; the harness's PASS line is what the ledger parses.\n"
    ));
    o.push_str("import spatial.dsl._\n\n");
    o.push_str(&format!(
        "@spatial class {class_name} extends SpatialTest {{\n"
    ));
    o.push_str("  type T = Int\n");
    o.push_str(&format!("  val M = {m}\n  val N = {n}\n  val K = {k}\n  val ts = {ts}\n  val its = {its}\n  val ip = {ip}\n\n"));
    o.push_str("  def gemm(a_data: Matrix[T], b_data: Matrix[T], c_init: Matrix[T]) = {\n");
    o.push_str("    val a_dram = DRAM[T](M, K)\n    val b_dram = DRAM[T](K, N)\n    val c_dram = DRAM[T](M, N)\n");
    o.push_str(
        "    setMem(a_dram, a_data)\n    setMem(b_dram, b_data)\n    setMem(c_dram, c_init)\n",
    );
    o.push_str("    Accel {\n");
    // Every loop of the nest carries its own parallelisation factor. Emitting `par 1` everywhere but the
    // innermost dot is what made a compiled GEMM lose to the same GEMM written by hand: Spatial's own
    // variants raise `loop_ii`/`loop_jj`/`loop_kk`/`loop_i`/`loop_j`, and a factor left at 1 is strictly less
    // work per cycle. `par 1` is emitted as no annotation at all, so a fully-serial nest is byte-identical to
    // what this emitted before.
    let par = |f: usize| {
        if f > 1 {
            format!(" par {f}")
        } else {
            String::new()
        }
    };
    o.push_str(&format!(
        "      Foreach(M by its{}, N by ts{}) {{ (ii, jj) =>\n",
        par(u.ii),
        par(u.jj)
    ));
    o.push_str("        val c_col = SRAM[T](its, ts)\n");
    o.push_str(&format!(
        "        MemReduce(c_col par ip)(K by ts{}) {{ kk =>\n",
        par(u.kk)
    ));
    o.push_str("          val c_col_partial = SRAM[T](its, ts)\n");
    o.push_str("          val b_sram = SRAM[T](ts, ts)\n");
    o.push_str("          b_sram load b_dram(kk::kk+ts, jj::jj+ts par ip)\n");
    o.push_str(&format!(
        "          Foreach(its by 1{}) {{ i =>\n",
        par(u.i)
    ));
    o.push_str("            val a_sram = SRAM[T](ts)\n");
    o.push_str("            a_sram load a_dram(ii+i, kk::kk+ts par ip)\n");
    o.push_str(&format!(
        "            Foreach(ts by 1{}) {{ j =>\n",
        par(u.j)
    ));
    o.push_str("              val dot = Reduce(Reg[T])(ts by 1 par ip) { k =>\n");
    o.push_str("                b_sram(k, j) * a_sram(k)\n");
    o.push_str("              } { _ + _ }\n");
    o.push_str("              Pipe { c_col_partial(i, j) = dot }\n");
    o.push_str("            }\n");
    o.push_str("          }\n");
    o.push_str("          c_col_partial\n");
    o.push_str("        } { _ + _ }\n");
    o.push_str("        c_dram(ii::ii+its, jj::jj+ts par ip) store c_col\n");
    o.push_str("      }\n");
    o.push_str("    }\n");
    o.push_str("    getMatrix(c_dram)\n");
    o.push_str("  }\n\n");
    o.push_str("  def main(args: Array[String]): Unit = {\n");
    o.push_str("    val a_data = (0::M, 0::K) { (i, j) => ((i * K + j) % 7).to[T] }\n");
    o.push_str("    val b_data = (0::K, 0::N) { (i, j) => ((i * N + j) % 5).to[T] }\n");
    o.push_str("    val c_init = (0::M, 0::N) { (i, j) => 0.to[T] }\n");
    o.push_str("    val c_gold = (0::M, 0::N) { (i, j) => Array.tabulate(K) { k => a_data(i, k) * b_data(k, j) }.reduce { _ + _ } }\n");
    o.push_str("    val c_result = gemm(a_data, b_data, c_init)\n");
    o.push_str("    val cksum = approxEql(c_gold, c_result, margin = 0.05)\n");
    o.push_str(&format!(
        "    println(\"PASS: \" + cksum + \" ({class_name})\")\n    assert(cksum)\n"
    ));
    o.push_str("  }\n}\n");
    Ok(o)
}

/// The knobs of Spatial's reduce-over-tiles regression apps (`pir/regression/DotProduct.scala`,
/// `TPCHQ6.scala`): the tile `ts` each outer iteration loads of every column, how many tiles run at once
/// (`op`), and the lanes of the loads and of the inner reduce (`ip`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZipPar {
    pub ts: usize,
    pub ip: usize,
    pub op: usize,
}

/// The knobs of Spatial's `OuterProduct` regression app (`pir/regression/OuterProduct.scala`): the tiles of
/// the two vectors (`ts1`, `ts2`), the lanes of the loads and the store (`ip`), the lanes of the tile's
/// column and row loops (`ip1`, `ip2`), and how many tiles of each vector run at once (`op1`, `op2`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OuterPar {
    pub ts1: usize,
    pub ts2: usize,
    pub ip: usize,
    pub ip1: usize,
    pub ip2: usize,
    pub op1: usize,
    pub op2: usize,
}

fn at_least_one(knobs: &[(&str, usize)]) -> Result<(), String> {
    match knobs.iter().find(|(_, v)| *v == 0) {
        Some((k, _)) => Err(format!("`{k}` is at least 1")),
        None => Ok(()),
    }
}

/// One column of a zip sum as the app declares it: its name (a Scala identifier the app does not otherwise use)
/// and the host data the program fills it with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZipColumn {
    pub name: String,
    pub fill: Fill,
}

/// The identifiers the zip-sum app declares itself, which a column may not take.
const ZIP_RESERVED: &[&str] = &[
    "N", "ts", "ip", "op", "T", "out", "accO", "part", "gold", "cksum", "args", "i", "ii", "zipsum", "main",
];

/// A column's host data as Scala, `t` the element index.
fn host_fill(f: &Fill) -> Result<String, String> {
    match f {
        Fill::Const(c) if i32::try_from(*c).is_ok() => Ok(format!("{c}.to[T]")),
        Fill::Pattern { modulus, shift: 0 } => Ok(format!("(i % {modulus}).to[T]")),
        Fill::Pattern { modulus, shift } => Ok(format!("((i % {modulus}) + {shift}).to[T]")),
        other => Err(format!("a column filled as {other:?} has no host data the app can print")),
    }
}

/// A column's element `t` under its fill.
fn fill_at(f: &Fill, t: i64) -> i64 {
    match f {
        Fill::Const(c) => *c,
        Fill::Pattern { modulus, shift } => t % modulus + shift,
        _ => unreachable!("refused by host_fill"),
    }
}

/// An operator as Spatial spells it, inside the accelerator and on the host alike.
fn op_symbol(op: ZipOp) -> &'static str {
    match op {
        ZipOp::Add => "+",
        ZipOp::Sub => "-",
        ZipOp::Mul => "*",
        ZipOp::Div => "/",
        ZipOp::Pow => "**",
        ZipOp::Lt => "<",
        ZipOp::Gt => ">",
        ZipOp::Le => "<=",
        ZipOp::Ge => ">=",
        ZipOp::Eq => "===",
        ZipOp::Ne => "=!=",
        ZipOp::And => "&&",
        ZipOp::Or => "||",
    }
}

/// The exponent of a power printed as Spatial's own apps write a whole one — a Scala `Int`, `x ** 2`, which Spatial
/// stages as multiplication — when it is a non-negative whole-number constant; `None` for any other exponent, printed
/// at the operand's type, a power the accelerator computes as such.
fn whole_exponent(x: f64) -> Option<i64> {
    (x >= 0.0 && x.fract() == 0.0 && x <= f64::from(i32::MAX)).then_some(x as i64)
}

/// Why a term has no value in the app's 32-bit arithmetic at one element, as a refusal names it.
fn stop_reason(stop: Stop) -> &'static str {
    match stop {
        Stop::IntRange => "leaves the 32-bit Int the Spatial app computes in (the VM's Int is 64-bit)",
        Stop::RealRange => "leaves the range of the 32-bit Float the Spatial app computes in (the VM's Real is a double)",
        Stop::ZeroDivisor => "divides by zero",
        Stop::NegativeExponent => "raises to a negative power",
        Stop::Mistyped => "is not a value the app computes",
    }
}

/// The term printed with column `k`'s element as `read(k)`: `===`/`=!=` and `mux` (Spatial's staged equality and
/// select) inside the accelerator, the host's `if` for a select when `host`.
fn zip_term(t: &ZipTerm, read: &dyn Fn(usize) -> String, host: bool, top: bool) -> String {
    let wrap = |s: String| if top { s } else { format!("({s})") };
    match t {
        ZipTerm::Read(k) => read(*k),
        ZipTerm::Const(c) => format!("{c}.to[T]"),
        ZipTerm::Real(b) => format!("{:?}.to[T]", f64::from_bits(*b)),
        ZipTerm::Abs(x) => format!("abs({})", zip_term(x, read, host, true)),
        ZipTerm::Op(op, l, r) => {
            let whole = match (op, &**r) {
                (ZipOp::Pow, ZipTerm::Const(c)) => whole_exponent(*c as f64),
                (ZipOp::Pow, ZipTerm::Real(b)) => whole_exponent(f64::from_bits(*b)),
                _ => None,
            };
            let rhs = whole.map_or_else(|| zip_term(r, read, host, false), |n| n.to_string());
            wrap(format!("{} {} {rhs}", zip_term(l, read, host, false), op_symbol(*op)))
        }
        ZipTerm::Select(c, a, b) => {
            let (c, a, b) = (zip_term(c, read, host, true), zip_term(a, read, host, true), zip_term(b, read, host, true));
            if host {
                wrap(format!("if ({c}) {a} else {b}"))
            } else {
                format!("mux({c}, {a}, {b})")
            }
        }
    }
}

/// Emit a Spatial REGRESSION-HARNESS zip sum `Σ_{t<n} f(c₀[t], c₁[t], …)` with the structure of Spatial's own
/// reduce-over-tiles apps (`DotProduct`, `TPCHQ6`): a `ts`-element tile of every column loaded in parallel,
/// `op` tiles reduced at once, each tile's sum an `ip`-lane `Reduce` of the term (the body of
/// `spatial.lib.ML.sum_flat`) — at our length, over the program's own column data, so the app's gold is the
/// LOGOS program's answer. The app computes in Spatial's 32-bit `Int`, as the regression apps do; a sum that
/// would wrap there (where the VM's 64-bit `Int` does not) is refused, as is a tile that does not divide the
/// length or a reduce wider than its tile.
pub fn emit_spatial_zip_sum_test(
    n: usize,
    columns: &[ZipColumn],
    term: &ZipTerm,
    p: ZipPar,
    class_name: &str,
) -> Result<String, String> {
    at_least_one(&[("ts", p.ts), ("ip", p.ip), ("op", p.op)])?;
    if n % p.ts != 0 {
        return Err(format!("the Spatial reduce-over-tiles template loads whole tiles: ts={} must divide N={n}", p.ts));
    }
    if p.ip > p.ts {
        return Err(format!("the reduce is {} lanes wide over a tile of {}", p.ip, p.ts));
    }
    for (k, c) in columns.iter().enumerate() {
        let ident = c.name.chars().next().map_or(false, |h| h.is_ascii_alphabetic())
            && c.name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_');
        if !ident || ZIP_RESERVED.contains(&c.name.as_str()) || columns[..k].iter().any(|o| o.name == c.name) {
            return Err(format!("the column `{}` is not a name the app can declare", c.name));
        }
    }
    if term.ty(&vec![Ty::Int; columns.len()]) != Some(Ty::Int) {
        return Err("the reduce-over-tiles app sums an Int term of Int columns".to_string());
    }
    let fills = columns.iter().map(|c| host_fill(&c.fill)).collect::<Result<Vec<_>, _>>()?;
    let (lo, hi) = (i32::MIN as i128, i32::MAX as i128);
    let mut magnitude: i128 = 0;
    for t in 0..n as i64 {
        let row: Vec<Scalar> = columns.iter().map(|c| Scalar::Int(fill_at(&c.fill, t))).collect();
        let v = match term.value(&row, (lo, hi), f64::MAX) {
            Ok(Scalar::Int(v)) => v,
            Ok(_) => return Err(format!("element {t} {}", stop_reason(Stop::Mistyped))),
            Err(stop) => return Err(format!("element {t} {}", stop_reason(stop))),
        };
        magnitude += (v as i128).abs();
        if magnitude > hi {
            return Err(format!("the sum leaves the 32-bit Int the Spatial app computes in by element {t} (the VM's Int is 64-bit)"));
        }
    }
    let ZipPar { ts, ip, op } = p;
    let accel = zip_term(term, &|k| format!("{}Blk(ii)", columns[k].name), false, true);
    let host = zip_term(term, &|k| format!("{}(i)", columns[k].name), true, true);
    let mut o = String::new();
    o.push_str(&format!(
        "// Generated by LOGOS (logicaffeine_compile::codegen_sva::spatial_emit) from a LOGOS loop recognized as the zip\n// sum Σ_t {host} over N={n}: Spatial's reduce-over-tiles structure (regression/DotProduct.scala,\n// regression/TPCHQ6.scala) at ts={ts} ip={ip} op={op}, over the program's own data. The PASS line is what the ledger parses.\n"
    ));
    o.push_str("import spatial.dsl._\n\n");
    o.push_str(&format!("@spatial class {class_name} extends SpatialTest {{\n"));
    o.push_str("  type T = Int\n");
    o.push_str(&format!("  val N = {n}\n  val ts = {ts}\n  val ip = {ip}\n  val op = {op}\n\n"));
    let params: Vec<String> = columns.iter().map(|c| format!("{}In: Array[T]", c.name)).collect();
    o.push_str(&format!("  def zipsum({}) = {{\n", params.join(", ")));
    for c in columns {
        o.push_str(&format!("    val {} = DRAM[T](N)\n", c.name));
    }
    o.push_str("    val out = ArgOut[T]\n");
    for c in columns {
        o.push_str(&format!("    setMem({0}, {0}In)\n", c.name));
    }
    o.push_str("    Accel {\n");
    o.push_str("      val accO = Reg[T](0.to[T])\n");
    o.push_str("      out := Reduce(accO)(N by ts par op) { i =>\n");
    for c in columns {
        o.push_str(&format!("        val {}Blk = SRAM[T](ts)\n", c.name));
    }
    o.push_str("        Parallel {\n");
    for c in columns {
        o.push_str(&format!("          {0}Blk load {0}(i::i+ts par ip)\n", c.name));
    }
    o.push_str("        }\n");
    o.push_str("        val part = Reg[T]\n");
    o.push_str(&format!("        Reduce(part)(ts par ip) {{ ii => {accel} }} {{ _ + _ }}\n"));
    o.push_str("        part.value\n");
    o.push_str("      } { _ + _ }\n");
    o.push_str("    }\n");
    o.push_str("    out\n");
    o.push_str("  }\n\n");
    o.push_str("  def main(args: Array[String]): Unit = {\n");
    for (c, f) in columns.iter().zip(&fills) {
        o.push_str(&format!("    val {} = Array.tabulate(N) {{ i => {f} }}\n", c.name));
    }
    let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    o.push_str(&format!("    val out = zipsum({})\n", names.join(", ")));
    o.push_str(&format!("    val gold = Array.tabulate(N) {{ i => {host} }}.reduce {{ _ + _ }}\n"));
    o.push_str("    val cksum = checkGold(out, gold)\n");
    o.push_str(&format!("    println(\"PASS: \" + cksum + \" ({class_name})\")\n    assert(cksum)\n"));
    o.push_str("  }\n}\n");
    Ok(o)
}

/// Emit a Spatial REGRESSION-HARNESS outer product `out[i][j] = a[i]·b[j]` (`m × n`) with the structure of
/// Spatial's own `OuterProduct` app — a `ts1`-tile of `a` per outer iteration, a `ts2`-tile of `b` per inner
/// one, the `ts1 × ts2` product tile computed in SRAM and stored whole — at our shape. Host data are small
/// residues; the harness's `PASS` line means equality. Typed refusal when a tile does not divide its vector
/// or a tile loop is wider than its tile.
pub fn emit_spatial_outer_test(m: usize, n: usize, p: OuterPar, class_name: &str) -> Result<String, String> {
    let OuterPar { ts1, ts2, ip, ip1, ip2, op1, op2 } = p;
    at_least_one(&[("ts1", ts1), ("ts2", ts2), ("ip", ip), ("ip1", ip1), ("ip2", ip2), ("op1", op1), ("op2", op2)])?;
    if m % ts1 != 0 || n % ts2 != 0 {
        return Err(format!(
            "the Spatial OuterProduct template streams whole tiles: ts1={ts1} must divide M={m} and ts2={ts2} must divide N={n}"
        ));
    }
    if ip2 > ts1 || ip1 > ts2 {
        return Err(format!("the tile loops run {ip2} x {ip1} lanes over a {ts1} x {ts2} tile"));
    }
    let mut o = String::new();
    o.push_str(&format!(
        "// Generated by LOGOS (logicaffeine_compile::codegen_sva::spatial_emit) from a LOGOS nest recognized as the outer\n// product out[{m}x{n}] = a[i]·b[j]: Spatial's OuterProduct structure (regression/OuterProduct.scala) with tiles\n// ts1={ts1} ts2={ts2}, ip={ip} ip1={ip1} ip2={ip2} op1={op1} op2={op2}. Gold on the host; the PASS line is what the ledger parses.\n"
    ));
    o.push_str("import spatial.dsl._\n\n");
    o.push_str(&format!("@spatial class {class_name} extends SpatialTest {{\n"));
    o.push_str("  type T = Int\n");
    o.push_str(&format!(
        "  val M = {m}\n  val N = {n}\n  val ts1 = {ts1}\n  val ts2 = {ts2}\n  val ip = {ip}\n  val ip1 = {ip1}\n  val ip2 = {ip2}\n  val op1 = {op1}\n  val op2 = {op2}\n\n"
    ));
    o.push_str("  def outerproduct(a: Array[T], b: Array[T]) = {\n");
    o.push_str("    val vec1 = DRAM[T](M)\n    val vec2 = DRAM[T](N)\n    val out = DRAM[T](M, N)\n");
    o.push_str("    setMem(vec1, a)\n    setMem(vec2, b)\n");
    o.push_str("    Accel {\n");
    o.push_str("      Foreach(M by ts1 par op1) { i =>\n");
    o.push_str("        val b1 = SRAM[T](ts1)\n");
    o.push_str("        b1 load vec1(i::i+ts1 par ip)\n");
    o.push_str("        Foreach(N by ts2 par op2) { j =>\n");
    o.push_str("          val b2 = SRAM[T](ts2)\n");
    o.push_str("          b2 load vec2(j::j+ts2 par ip)\n");
    o.push_str("          val outTile = SRAM[T](ts1, ts2)\n");
    o.push_str("          Foreach(ts1 par ip2, ts2 par ip1) { (ii, jj) => outTile(ii, jj) = b1(ii) * b2(jj) }\n");
    o.push_str("          out(i::i+ts1, j::j+ts2 par ip) store outTile\n");
    o.push_str("        }\n");
    o.push_str("      }\n");
    o.push_str("    }\n");
    o.push_str("    out\n");
    o.push_str("  }\n\n");
    o.push_str("  def main(args: Array[String]): Unit = {\n");
    o.push_str("    val a = Array.tabulate(M) { i => (i % 7).to[T] }\n");
    o.push_str("    val b = Array.tabulate(N) { i => (i % 5).to[T] }\n");
    o.push_str("    val out = outerproduct(a, b)\n");
    o.push_str("    val gold = (0::M, 0::N) { (i, j) => a(i) * b(j) }\n");
    o.push_str("    val cksum = checkGold(out, gold, 0)\n");
    o.push_str(&format!("    println(\"PASS: \" + cksum + \" ({class_name})\")\n    assert(cksum)\n"));
    o.push_str("  }\n}\n");
    Ok(o)
}

/// The knobs of Spatial's `GDA` regression app (`pir/regression/GDA.scala`): the rows a tile of the data loads
/// (`ts`), how many such tiles run at once (`op`), the rows of a tile reduced at once (`mp1`), the covariance rows
/// one rank-1 update writes at once (`mp2`), and the lanes of the loads, the store and the column loops (`ip`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GdaPar {
    pub ts: usize,
    pub op: usize,
    pub mp1: usize,
    pub mp2: usize,
    pub ip: usize,
}

/// One array a scatter row term reads, as the GDA app declares it: its name, its axis and its host data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GdaColumn {
    pub name: String,
    pub axis: Axis,
    pub fill: Fill,
}

/// Emit a Spatial REGRESSION-HARNESS scatter accumulation `S = Σ_{r<R} d_r d_rᵀ` (`C × C`) with the structure of
/// Spatial's own `GDA` app — the column-axis arrays loaded once, a `MemReduce` over `ts`-row tiles of the row-axis
/// arrays and of the row-major data, inside it a `MemReduce` over the tile's rows that computes each row's `d` into
/// `subTile` and its outer product into `sigmaTile` — at our shape, over the program's own data. The app computes in
/// Spatial's 32-bit `Int`; a sum that would wrap there (where the VM's 64-bit `Int` does not) is refused, as is a
/// tile that does not divide the rows.
pub fn emit_spatial_gda_test(
    rows: usize,
    cols: usize,
    columns: &[GdaColumn],
    term: &ZipTerm,
    p: GdaPar,
    class_name: &str,
) -> Result<String, String> {
    let GdaPar { ts, op, mp1, mp2, ip } = p;
    at_least_one(&[("ts", ts), ("op", op), ("mp1", mp1), ("mp2", mp2), ("ip", ip)])?;
    if rows % ts != 0 {
        return Err(format!("the Spatial GDA template loads whole tiles: ts={ts} must divide R={rows}"));
    }
    for (k, c) in columns.iter().enumerate() {
        let ident = c.name.chars().next().map_or(false, |h| h.is_ascii_alphabetic())
            && c.name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_');
        let reserved = ["R", "C", "ts", "op", "mp1", "mp2", "ip", "T", "sigma", "sigmaOut", "sigmaBlk", "sigmaTile",
                        "subTile", "gold", "result", "cksum", "args", "r", "rr", "cc", "ii", "jj", "i", "j", "gda", "main"];
        if !ident || reserved.contains(&c.name.as_str()) || columns[..k].iter().any(|o| o.name == c.name) {
            return Err(format!("the array `{}` is not a name the app can declare", c.name));
        }
    }
    if term.ty(&vec![Ty::Int; columns.len()]) != Some(Ty::Int) {
        return Err("the GDA app centres Int rows on Int means".to_string());
    }
    let fills = columns.iter().map(|c| host_fill(&c.fill)).collect::<Result<Vec<_>, _>>()?;
    let (lo, hi) = (i32::MIN as i128, i32::MAX as i128);
    let d = |r: usize, c: usize| -> Result<i64, String> {
        let row: Vec<Scalar> = columns
            .iter()
            .map(|col| {
                let t = match col.axis {
                    Axis::Row => r,
                    Axis::Col => c,
                    Axis::RowCol => r * cols + c,
                };
                Scalar::Int(fill_at(&col.fill, t as i64))
            })
            .collect::<Vec<_>>();
        match term.value(&row, (lo, hi), f64::MAX) {
            Ok(Scalar::Int(v)) => Ok(v),
            Ok(_) => Err(format!("row {r} column {c} {}", stop_reason(Stop::Mistyped))),
            Err(stop) => Err(format!("row {r} column {c} {}", stop_reason(stop))),
        }
    };
    let dr: Vec<Vec<i64>> = (0..rows).map(|r| (0..cols).map(|c| d(r, c)).collect::<Result<_, _>>()).collect::<Result<_, _>>()?;
    for i in 0..cols {
        for j in 0..cols {
            let magnitude: i128 = dr.iter().map(|v| (v[i] as i128 * v[j] as i128).abs()).sum();
            if magnitude > hi {
                return Err(format!("covariance [{i}][{j}] leaves the 32-bit Int the Spatial app computes in (the VM's Int is 64-bit)"));
            }
        }
    }
    let tile = |c: &GdaColumn| format!("{}Tile", c.name);
    let accel = zip_term(term, &|k| match columns[k].axis {
        Axis::Row => format!("{}(rr)", tile(&columns[k])),
        Axis::Col => format!("{}(cc)", tile(&columns[k])),
        Axis::RowCol => format!("{}(rr, cc)", tile(&columns[k])),
    }, false, true);
    let host = |at: &str| zip_term(term, &|k| match columns[k].axis {
        Axis::Row => format!("{}(r)", columns[k].name),
        Axis::Col => format!("{}({at})", columns[k].name),
        Axis::RowCol => format!("{}(r * C + {at})", columns[k].name),
    }, true, false);
    let loads = |lines: Vec<String>, pad: &str, o: &mut String| {
        if lines.len() > 1 {
            o.push_str(&format!("{pad}Parallel {{\n"));
            for l in lines {
                o.push_str(&format!("{pad}  {l}\n"));
            }
            o.push_str(&format!("{pad}}}\n"));
        } else {
            for l in lines {
                o.push_str(&format!("{pad}{l}\n"));
            }
        }
    };
    let mut o = String::new();
    o.push_str(&format!(
        "// Generated by LOGOS (logicaffeine_compile::codegen_sva::spatial_emit) from a LOGOS nest recognized as the scatter\n// accumulation S = Σ_r d_r d_rᵀ over R={rows} rows of C={cols}, d_r[c] = {}: Spatial's GDA structure\n// (regression/GDA.scala) at ts={ts} op={op} mp1={mp1} mp2={mp2} ip={ip}, over the program's own data. The PASS line is what the ledger parses.\n",
        host("c")
    ));
    o.push_str("import spatial.dsl._\n\n");
    o.push_str(&format!("@spatial class {class_name} extends SpatialTest {{\n"));
    o.push_str("  type T = Int\n");
    o.push_str(&format!("  val R = {rows}\n  val C = {cols}\n  val ts = {ts}\n  val op = {op}\n  val mp1 = {mp1}\n  val mp2 = {mp2}\n  val ip = {ip}\n\n"));
    let params: Vec<String> = columns.iter().map(|c| format!("{}In: Array[T]", c.name)).collect();
    o.push_str(&format!("  def gda({}) = {{\n", params.join(", ")));
    for c in columns {
        let dims = match c.axis {
            Axis::Row => "R",
            Axis::Col => "C",
            Axis::RowCol => "R, C",
        };
        o.push_str(&format!("    val {} = DRAM[T]({dims})\n", c.name));
    }
    o.push_str("    val sigma = DRAM[T](C, C)\n");
    for c in columns {
        o.push_str(&format!("    setMem({0}, {0}In)\n", c.name));
    }
    o.push_str("    Accel {\n");
    let by_axis = |a: Axis| columns.iter().filter(move |c| c.axis == a);
    for c in by_axis(Axis::Col) {
        o.push_str(&format!("      val {} = SRAM[T](C)\n", tile(c)));
    }
    loads(by_axis(Axis::Col).map(|c| format!("{} load {}(0 :: C par ip)", tile(c), c.name)).collect(), "      ", &mut o);
    o.push_str("      val sigmaOut = SRAM[T](C, C)\n");
    o.push_str("      MemReduce(sigmaOut par ip)(R by ts par op) { r =>\n");
    for c in columns.iter().filter(|c| c.axis != Axis::Col) {
        let dims = if c.axis == Axis::Row { "ts" } else { "ts, C" };
        o.push_str(&format!("        val {} = SRAM[T]({dims})\n", tile(c)));
    }
    loads(
        columns
            .iter()
            .filter(|c| c.axis != Axis::Col)
            .map(|c| match c.axis {
                Axis::Row => format!("{} load {}(r :: r + ts par ip)", tile(c), c.name),
                _ => format!("{} load {}(r :: r + ts, 0 :: C par ip)", tile(c), c.name),
            })
            .collect(),
        "        ",
        &mut o,
    );
    o.push_str("        val sigmaBlk = SRAM[T](C, C)\n");
    o.push_str("        MemReduce(sigmaBlk par ip)(ts par mp1) { rr =>\n");
    o.push_str("          val subTile = SRAM[T](C)\n          val sigmaTile = SRAM[T](C, C)\n");
    o.push_str(&format!("          Foreach(C par ip) {{ cc => subTile(cc) = {accel} }}\n"));
    o.push_str("          Foreach(C by 1 par mp2) { ii =>\n");
    o.push_str("            Foreach(C par ip) { jj => sigmaTile(ii, jj) = subTile(ii) * subTile(jj) }\n");
    o.push_str("          }\n          sigmaTile\n        } { _ + _ }\n      } { _ + _ }\n");
    o.push_str("      sigma(0 :: C, 0 :: C par ip) store sigmaOut\n    }\n    getMem(sigma)\n  }\n\n");
    o.push_str("  def main(args: Array[String]): Unit = {\n");
    for (c, f) in columns.iter().zip(&fills) {
        let n = match c.axis {
            Axis::Row => "R",
            Axis::Col => "C",
            Axis::RowCol => "R * C",
        };
        o.push_str(&format!("    val {} = Array.tabulate({n}) {{ i => {f} }}\n", c.name));
    }
    let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    o.push_str(&format!("    val result = gda({})\n", names.join(", ")));
    o.push_str(&format!(
        "    val gold = Array.tabulate(C) {{ i => Array.tabulate(C) {{ j => Array.tabulate(R) {{ r => {} * {} }}.reduce {{ _ + _ }} }} }}.flatten\n",
        host("i"),
        host("j")
    ));
    o.push_str("    val cksum = approxEql(gold, result)\n");
    o.push_str(&format!("    println(\"PASS: \" + cksum + \" ({class_name})\")\n    assert(cksum)\n  }}\n}}\n"));
    Ok(o)
}

/// The knobs of Spatial's `BlackScholes` regression app (`pir/regression/BlackScholes.scala`): the tile `ts` of every
/// column each outer iteration streams through FIFOs, how many tiles run at once (`op`), and the lanes of the loads,
/// the map and the store (`ip`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapPar {
    pub ts: usize,
    pub op: usize,
    pub ip: usize,
}

/// One column of a zip map as the app declares it: its name, its element type and the host data the program fills
/// it with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapColumn {
    pub name: String,
    pub ty: Ty,
    pub fill: Fill,
}

/// Spatial's type for a LOGOS element type, as the regression apps declare it: their 32-bit `Int` and `Float`.
fn spatial_ty(t: Ty) -> &'static str {
    match t {
        Ty::Int => "Int",
        Ty::Real => "Float",
        Ty::Bool => "Bit",
    }
}

/// Names an app's column may not take: Scala's keywords and the Spatial names a zip-map app uses.
const MAP_RESERVED: &[&str] = &[
    "N", "ts", "op", "ip", "i", "j", "r", "g", "gold", "result", "cksum", "zipmap", "main", "args", "abs", "mux", "min",
    "max", "pow", "sqrt", "exp", "ln", "Int", "Float", "Bit", "Array", "DRAM", "FIFO", "SRAM", "Reg", "Foreach",
    "Reduce", "Accel", "val", "var", "def", "type", "if", "else", "for", "while", "do", "new", "class", "object",
    "trait", "match", "case", "return", "true", "false", "null", "this", "super", "import", "package", "with",
    "extends", "override", "lazy", "yield", "try", "catch", "finally", "throw",
];

/// A constant as the app spells it at type `at`: `3.to[Int]`, `0.25.to[Float]` (a Scala `Double` literal, so the
/// `Float` is the VM's double rounded once), a negative one parenthesized so it is a literal, not a negation.
fn literal(v: Scalar, at: Ty) -> String {
    let real = |x: f64| if x.is_sign_negative() { format!("({x:?}).to[Float]") } else { format!("{x:?}.to[Float]") };
    match (v, at) {
        (Scalar::Int(c), Ty::Real) => real(c as f64),
        (Scalar::Int(c), _) if c < 0 => format!("({c}).to[Int]"),
        (Scalar::Int(c), _) => format!("{c}.to[Int]"),
        (Scalar::Real(x), _) => real(x),
        (Scalar::Bool(b), _) => format!("{b}.to[Bit]"),
    }
}

/// A column's host data as Scala, at its element type, the element's index the Scala expression `at`.
fn host_data(f: &Fill, ty: Ty, at: &str) -> Result<String, String> {
    let int = |c: i64| i32::try_from(c).is_ok();
    let at = if at.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { at.to_string() } else { format!("({at})") };
    match (f, ty) {
        (Fill::Const(c), Ty::Int) if int(*c) => Ok(literal(Scalar::Int(*c), Ty::Int)),
        (Fill::Pattern { modulus, shift: 0 }, Ty::Int) if int(*modulus) => Ok(format!("({at} % {modulus}).to[Int]")),
        (Fill::Pattern { modulus, shift }, Ty::Int) if int(*modulus) && int(*shift) && int(modulus + shift) => {
            Ok(format!("(({at} % {modulus}) + {shift}).to[Int]"))
        }
        (Fill::ConstF(b), Ty::Real) => Ok(literal(Scalar::Real(f64::from_bits(*b)), Ty::Real)),
        (Fill::RealPattern { modulus, shift, divisor }, Ty::Real) if int(*modulus) && int(*shift) && int(modulus + shift) => {
            Ok(format!(
                "((({at} % {modulus}) + {shift}).to[Float] / {})",
                literal(Scalar::Real(f64::from_bits(*divisor)), Ty::Real)
            ))
        }
        (other, ty) => Err(format!("a column of {ty:?} filled as {other:?} has no host data the app can print")),
    }
}

/// A term printed from its DAG for a Spatial app. Each operand of an operator is printed at the operator's operand
/// type (an `Int` beside a `Real` converted with `.to[Float]`), a constant at the type it is used at; a subterm used
/// more than once is bound once, as a `val`, in the order first needed; `mux` and Spatial's `===`/`=!=` inside the
/// accelerator, the host's `if` for a select when `host`.
struct DagPrinter<'d> {
    dag: &'d TermDag,
    tys: &'d [Ty],
    read: &'d dyn Fn(usize) -> String,
    host: bool,
    names: HashMap<usize, String>,
    lines: Vec<String>,
}

impl<'d> DagPrinter<'d> {
    fn print(dag: &'d TermDag, tys: &'d [Ty], read: &'d dyn Fn(usize) -> String, host: bool) -> (Vec<String>, String) {
        let mut p = DagPrinter { dag, tys, read, host, names: HashMap::new(), lines: Vec::new() };
        let root = p.at(dag.root, tys[dag.root]);
        (p.lines, root)
    }

    /// Node `i` as an expression of type `want`.
    fn at(&mut self, i: usize, want: Ty) -> String {
        let own = self.tys[i];
        let text = match self.dag.nodes[i] {
            DagNode::Const(c) => return literal(Scalar::Int(c), want),
            DagNode::Real(b) => return literal(Scalar::Real(f64::from_bits(b)), want),
            DagNode::Read(k) => (self.read)(k),
            _ if self.names.contains_key(&i) => self.names[&i].clone(),
            node => {
                let expr = self.node(node, own);
                if self.dag.uses[i] > 1 {
                    let name = format!("t{}", self.names.len());
                    self.lines.push(format!("val {name} = {expr}"));
                    self.names.insert(i, name.clone());
                    name
                } else {
                    expr
                }
            }
        };
        if own == Ty::Int && want == Ty::Real {
            format!("{text}.to[Float]")
        } else {
            text
        }
    }

    fn node(&mut self, node: DagNode, own: Ty) -> String {
        match node {
            DagNode::Abs(x) => format!("abs({})", self.at(x, own)),
            DagNode::Op(op, l, r) => {
                let operand = if op.connects() {
                    Ty::Bool
                } else if !op.compares() {
                    own
                } else if self.tys[l] == Ty::Real || self.tys[r] == Ty::Real {
                    Ty::Real
                } else {
                    Ty::Int
                };
                let whole = match (op, self.dag.nodes[r]) {
                    (ZipOp::Pow, DagNode::Const(c)) => whole_exponent(c as f64),
                    (ZipOp::Pow, DagNode::Real(b)) => whole_exponent(f64::from_bits(b)),
                    _ => None,
                };
                let a = self.at(l, operand);
                let b = whole.map_or_else(|| self.at(r, operand), |n| n.to_string());
                format!("({a} {} {b})", op_symbol(op))
            }
            DagNode::Select(c, a, b) => {
                let c = self.at(c, Ty::Bool);
                let (a, b) = (self.at(a, own), self.at(b, own));
                if self.host {
                    format!("(if ({c}) {a} else {b})")
                } else {
                    format!("mux({c}, {a}, {b})")
                }
            }
            DagNode::Read(_) | DagNode::Const(_) | DagNode::Real(_) => unreachable!("leaves are printed in place"),
        }
    }
}

/// Emit a Spatial REGRESSION-HARNESS zip map `out[t] = f(c₀[t], c₁[t], …)` with the structure of Spatial's own
/// `BlackScholes` app: per `ts`-element tile, a FIFO of every column loaded in the order the program declares its
/// columns (`declared`, a permutation of the columns, which are in the order the term reads them), an `ip`-lane
/// `Foreach` dequeuing one element of every column and enqueuing the term's value, the output tile stored, `op`
/// tiles at once — at our length, over the program's own data, with the variant's PIR arguments (`pir_args`, the
/// suffix its `pirArgs` override appends). The term is printed as its DAG: a function the program calls twice is two
/// copies, as Spatial stages it, and a subterm used twice is computed once. The app computes in Spatial's 32-bit
/// `Int` and `Float`; a term whose `Int` would wrap there or whose `Real` would leave the `Float`'s range is refused,
/// as is a division by zero on any path (the host's gold would trap), a tile that does not divide the length, or a
/// map wider than its tile. The gold is the host's own evaluation of the term: a `Float` result passes within 0.1%
/// (and 0.001) of it, an `Int` one exactly.
#[allow(clippy::too_many_arguments)]
pub fn emit_spatial_zip_map_test(
    n: usize,
    columns: &[MapColumn],
    declared: &[usize],
    term: &ZipTerm,
    out: &str,
    p: MapPar,
    pir_args: Option<&str>,
    class_name: &str,
) -> Result<String, String> {
    let MapPar { ts, op, ip } = p;
    at_least_one(&[("ts", ts), ("op", op), ("ip", ip)])?;
    if n % ts != 0 {
        return Err(format!("the Spatial BlackScholes template streams whole tiles: ts={ts} must divide N={n}"));
    }
    if ip > ts {
        return Err(format!("the map is {ip} lanes wide over a tile of {ts}"));
    }
    let mut order = declared.to_vec();
    order.sort_unstable();
    if order != (0..columns.len()).collect::<Vec<_>>() {
        return Err("the declaration order is not an order of the columns".to_string());
    }
    let reads: Vec<Ty> = columns.iter().map(|c| c.ty).collect();
    let dag = TermDag::of(term);
    let tys = dag.types(&reads).ok_or("the term is ill-typed over its columns")?;
    let out_ty = tys[dag.root];
    if out_ty == Ty::Bool || reads.contains(&Ty::Bool) {
        return Err("the app maps Int and Float columns to an Int or Float column".to_string());
    }
    let mut taken: Vec<String> = Vec::new();
    for name in columns.iter().map(|c| c.name.as_str()).chain([out]) {
        let ident = name.chars().next().map_or(false, |h| h.is_ascii_lowercase())
            && name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_');
        if !ident || MAP_RESERVED.contains(&name) {
            return Err(format!("`{name}` is not a name the app can declare"));
        }
        taken.extend([name.to_string(), format!("{name}In"), format!("{name}Blk"), format!("{name}V")]);
    }
    let shared = (0..dag.nodes.len()).filter(|i| dag.uses[*i] > 1).count();
    taken.extend((0..shared).map(|k| format!("t{k}")));
    let mut seen = std::collections::HashSet::new();
    if let Some(clash) = taken.iter().find(|t| !seen.insert(t.as_str()) || MAP_RESERVED.contains(&t.as_str())) {
        return Err(format!("the app would declare `{clash}` twice"));
    }
    for node in &dag.nodes {
        if let DagNode::Const(c) = node {
            if i32::try_from(*c).is_err() {
                return Err(format!("the constant {c} {}", stop_reason(Stop::IntRange)));
            }
        }
    }
    let fills = columns.iter().map(|c| host_data(&c.fill, c.ty, "i")).collect::<Result<Vec<_>, _>>()?;
    let int_divisors: Vec<usize> = dag
        .nodes
        .iter()
        .filter_map(|node| match *node {
            DagNode::Op(ZipOp::Div, l, r) if tys[l] == Ty::Int && tys[r] == Ty::Int => Some(r),
            _ => None,
        })
        .collect();
    let (ints, reals) = ((i32::MIN as i128, i32::MAX as i128), f32::MAX as f64);
    for t in 0..n as i64 {
        let row: Vec<Scalar> = columns.iter().map(|c| c.fill.element(t).expect("host data has elements")).collect();
        dag.value(&row, ints, reals).map_err(|stop| format!("element {t} {}", stop_reason(stop)))?;
        for d in &int_divisors {
            if dag.value_of(*d, &row, ints, reals) == Ok(Scalar::Int(0)) {
                return Err(format!("element {t} {} on a path the program does not take, which the app's host would still evaluate", stop_reason(Stop::ZeroDivisor)));
            }
        }
    }
    let accel_read = |k: usize| format!("{}V", columns[k].name);
    let (accel_lines, accel_root) = DagPrinter::print(&dag, &tys, &accel_read, false);
    let host_read = |k: usize| format!("{}(i)", columns[k].name);
    let (host_lines, host_root) = DagPrinter::print(&dag, &tys, &host_read, true);
    let (oty, decl) = (spatial_ty(out_ty), declared.iter().map(|k| &columns[*k]).collect::<Vec<_>>());
    let mut o = String::new();
    o.push_str(&format!(
        "// Generated by LOGOS (logicaffeine_compile::codegen_sva::spatial_emit) from a LOGOS loop recognized as the zip\n// map {out}[t] = f({}) over N={n}: Spatial's BlackScholes structure (regression/BlackScholes.scala) at ts={ts} op={op}\n// ip={ip}, over the program's own data. The PASS line is what the ledger parses.\n",
        columns.iter().map(|c| format!("{}[t]", c.name)).collect::<Vec<_>>().join(", ")
    ));
    o.push_str("import spatial.dsl._\n\n");
    o.push_str(&format!("@spatial class {class_name} extends SpatialTest {{\n"));
    if let Some(args) = pir_args {
        o.push_str(&format!("  override def pirArgs = super.pirArgs + {args:?}\n"));
    }
    o.push_str(&format!("  val N = {n}\n  val ts = {ts}\n  val op = {op}\n  val ip = {ip}\n\n"));
    let params: Vec<String> = decl.iter().map(|c| format!("{}In: Array[{}]", c.name, spatial_ty(c.ty))).collect();
    o.push_str(&format!("  def zipmap({}) = {{\n", params.join(", ")));
    for c in &decl {
        o.push_str(&format!("    val {} = DRAM[{}](N)\n", c.name, spatial_ty(c.ty)));
    }
    o.push_str(&format!("    val {out} = DRAM[{oty}](N)\n"));
    for c in &decl {
        o.push_str(&format!("    setMem({0}, {0}In)\n", c.name));
    }
    o.push_str("    Accel {\n");
    o.push_str("      Foreach(N by ts par op) { i =>\n");
    for c in &decl {
        o.push_str(&format!("        val {}Blk = FIFO[{}](ts)\n", c.name, spatial_ty(c.ty)));
    }
    o.push_str(&format!("        val {out}Blk = FIFO[{oty}](ts)\n"));
    for c in &decl {
        o.push_str(&format!("        {0}Blk load {0}(i::i+ts par ip)\n", c.name));
    }
    o.push_str("        Foreach(ts par ip) { j =>\n");
    for c in columns {
        o.push_str(&format!("          val {0}V = {0}Blk.deq\n", c.name));
    }
    for l in &accel_lines {
        o.push_str(&format!("          {l}\n"));
    }
    o.push_str(&format!("          {out}Blk.enq({accel_root})\n"));
    o.push_str("        }\n");
    o.push_str(&format!("        {out}(i::i+ts par ip) store {out}Blk\n"));
    o.push_str("      }\n    }\n");
    o.push_str(&format!("    getMem({out})\n  }}\n\n"));
    o.push_str("  def main(args: Array[String]): Unit = {\n");
    for k in declared {
        o.push_str(&format!("    val {} = Array.tabulate(N) {{ i => {} }}\n", columns[*k].name, fills[*k]));
    }
    o.push_str(&format!(
        "    val result = zipmap({})\n",
        decl.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")
    ));
    o.push_str("    val gold = Array.tabulate(N) { i =>\n");
    for l in &host_lines {
        o.push_str(&format!("      {l}\n"));
    }
    o.push_str(&format!("      {host_root}\n    }}\n"));
    if out_ty == Ty::Real {
        o.push_str("    val cksum = result.zip(gold) { (r, g) => abs(r - g) <= 0.001.to[Float] * abs(g) + 0.001.to[Float] }.reduce { _ & _ }\n");
    } else {
        o.push_str("    val cksum = result.zip(gold) { (r, g) => r === g }.reduce { _ & _ }\n");
    }
    o.push_str(&format!("    println(\"PASS: \" + cksum + \" ({class_name})\")\n    assert(cksum)\n"));
    o.push_str("  }\n}\n");
    Ok(o)
}

/// The knobs of Spatial's `Kmeans` regression app (`pir/regression/Kmeans.scala`): the points a tile loads (`ts`), the
/// lanes of the loads, the store and the feature loops (`ip`), how many tiles run at once (`op`), and how many points,
/// centroids and centroid updates run at once (`mp1`, `mp2`, `mp3`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KmeansPar {
    pub ts: usize,
    pub ip: usize,
    pub op: usize,
    pub mp1: usize,
    pub mp2: usize,
    pub mp3: usize,
}

/// Emit a Spatial REGRESSION-HARNESS k-means clustering with the structure of Spatial's own `Kmeans` app — the
/// centroids loaded from the first `K` points, `iters` epochs each a `MemReduce` over `ts`-point tiles of a
/// `MemReduce` over the tile's points (each point's distance to every centroid a `Reduce` over the features, the
/// nearest found by a min and an index reduce, the point added to its centroid's row as a one-hot tile), then every
/// centroid set to its row over its count — at our shape and the app's runtime arguments, over the program's own
/// data, written at the app's `X = Int` (the app's `kmeans[T:Num]` instantiated). The app keeps each centroid's count
/// as a last feature of every point fixed at 1 (`D = f + 1` columns), where the program keeps counts; equally near
/// centroids go to the last (the app's `max` of the matching indices) or, for a program keeping the first, to the
/// first (a `min`). The gold is the clustering the program computes, evaluated on the host the way the accelerator
/// computes it, the nearest centroid chosen by distance and then by index whichever way the host folds. The app
/// computes in Spatial's 32-bit `Int`; a distance or a sum that would wrap there is refused, as is a tile that does
/// not divide the points.
pub fn emit_spatial_kmeans_test(
    spec: &crate::tensor_recognize::KmeansSpec,
    points: &Fill,
    p: KmeansPar,
    class_name: &str,
) -> Result<String, String> {
    use crate::tensor_recognize::Ties;
    let KmeansPar { ts, ip, op, mp1, mp2, mp3 } = p;
    at_least_one(&[("ts", ts), ("ip", ip), ("op", op), ("mp1", mp1), ("mp2", mp2), ("mp3", mp3)])?;
    let (n, k, f, iters) = (spec.n, spec.k, spec.f, spec.iters);
    if n % ts != 0 {
        return Err(format!("the Spatial Kmeans template loads whole tiles: ts={ts} must divide N={n}"));
    }
    if n < k || k == 0 || f == 0 {
        return Err(format!("{n} points cannot seed {k} centroids of {f} features"));
    }
    let data = host_data(points, Ty::Int, "i * DM1 + d")?;
    let values: Vec<i64> = (0..(n * f) as i64)
        .map(|t| match points.element(t) {
            Some(Scalar::Int(v)) => Ok(v),
            _ => Err("the points have no Int data the app can print".to_string()),
        })
        .collect::<Result<_, _>>()?;
    spec.run(&values, (i32::MIN as i128, i32::MAX as i128))
        .map_err(|stop| format!("the clustering {}", stop_reason(stop)))?;
    let dag = TermDag::of(&spec.distance);
    let tys = dag.types(&[Ty::Int, Ty::Int]).ok_or("the distance is ill-typed")?;
    let (accel_lines, accel_term) = DagPrinter::print(&dag, &tys, &|r| ["pts(pt,d)", "cts(ct,d)"][r].to_string(), false);
    let (host_lines, host_term) = DagPrinter::print(&dag, &tys, &|r| ["p1(i)", "p2(i)"][r].to_string(), true);
    let block = |lines: &[String], pad: &str, root: &str| -> String {
        if lines.is_empty() {
            root.to_string()
        } else {
            let body: String = lines.iter().map(|l| format!("{pad}  {l}\n")).collect();
            format!("{{\n{body}{pad}  {root}\n{pad}}}")
        }
    };
    // The host's gold keeps the nearest by distance and then by index, so it does not depend on which way the host
    // folds: Spatial's C++ host reduces `rfunc(next, accumulated)`, its Scala host `rfunc(accumulated, next)`.
    let (sentinel, reduce, prefer) = match spec.ties {
        Ties::Last => ("-1", "max(a,b)", "if (a._1 < b._1 || (a._1 == b._1 && a._2 > b._2)) a else b"),
        Ties::First => ("K", "min(a,b)", "if (a._1 < b._1 || (a._1 == b._1 && a._2 < b._2)) a else b"),
    };
    let ties = if spec.ties == Ties::Last { "last" } else { "first" };
    let mut o = format!(
        "// Generated by LOGOS (logicaffeine_compile::codegen_sva::spatial_emit) from a LOGOS loop nest recognized as k-means\n// clustering ({n} points of {f} features, {k} centroids, {iters} epochs, the {ties} of equally near centroids):\n// Spatial's Kmeans structure (regression/Kmeans.scala) at ts={ts} ip={ip} op={op} mp1={mp1} mp2={mp2} mp3={mp3}, over the\n// program's own data. The PASS line is what the ledger parses.\nimport spatial.dsl._\n\n"
    );
    o.push_str(&format!("@spatial class {class_name} extends SpatialTest {{ self =>\n  type X = Int\n\n"));
    o.push_str(&format!(
        "  val K = {k}\n  val D = {}\n  val ts = {ts}\n  val ip = {ip}\n  val op = {op}\n  val mp1 = {mp1}\n  val mp2 = {mp2}\n  val mp3 = {mp3}\n\n",
        f + 1
    ));
    o.push_str(&format!("  override def runtimeArgs = \"{iters} {n}\"\n\n  def DM1 = D - 1\n\n"));
    o.push_str(&format!(
        r#"  def kmeans(points_in: Array[X], I:Int, N:Int) = {{

    val iters = ArgIn[Int]
    val numPoints     = ArgIn[Int]

    setArg(iters, I)
    setArg(numPoints, N)

    val points = DRAM[X](numPoints, D)
    val centroids = DRAM[X](K, D)
    setMem(points, points_in)

    Accel {{
      val cts = SRAM[X](K, D)

      cts load points(0::K, 0::D par ip)

      Sequential.Foreach(iters by 1){{ epoch =>
        val newCents = SRAM[X](K,D)
        if (ip == 1) {{
          newCents.onlyblockcyclic.blockcyclic_Bs(Seq(63))
          ()
        }}
        MemReduce(newCents par ip)(numPoints by ts par op){{i =>
          val pts = SRAM[X](ts, D)
          pts load points(i::i+ts, 0::D par ip)

          MemReduce(SRAM[X](K,D) par ip)(ts par mp1){{pt =>
            val dists = SRAM[X](K)
            Foreach(K by 1 par mp2) {{ ct =>
              val dist = Reduce(Reg[X])(D par ip){{d =>
                mux(d==DM1, 0, {accel})
              }}{{_+_}}
              dists(ct) = dist.value
            }}
            val minDist = Reduce(Reg[X])(K by 1 par ip) {{ ct => dists(ct) }} {{ (a,b) => min(a, b) }}
            val minCent = Reduce(Reg[I32])(K by 1 par ip) {{ ct =>
              mux(dists(ct) == minDist.value, ct, {sentinel})
            }} {{ (a,b) => {reduce} }}

            val localCent = SRAM[X](K,D)
            Foreach(K by 1, D par ip){{(ct,d) =>
              localCent(ct, d) = mux(ct == minCent.value, pts(pt,d), 0.to[X])
            }}
            localCent
          }}{{_+_}}
        }}{{_+_}}

        Foreach(K by 1 par mp3){{ ct =>
          Foreach(D-1 par ip){{ d =>
            val centCount = newCents(ct,DM1)
            val cond = centCount == 0.to[X]
            cts(ct, d) = mux(cond, 0.to[X], newCents(ct,d) / mux(cond,1.to[X], centCount))
          }}
        }}
      }}

      centroids(0::K, D par ip) store cts
    }}

    getMem(centroids)
  }}

  def main(args: Array[String]): Unit = {{
    val I = args(0).to[Int]
    val N = args(1).to[Int]

    val pts = Array.tabulate(N){{i => Array.tabulate(D){{d => if (d == DM1) 1.to[X] else {data} }}}}

    val result = kmeans(pts.flatten, I, N)

    val cts = Array.tabulate(K) {{ i => Array.empty[X](D) }}
    for (k <- 0 until K) {{
      for (d <- 0 until D) {{
        val cent = cts(k)
        val point = pts(k)
        cent(d) = point(d)
      }}
    }}

    def dist(p1: Array[X], p2: Array[X]) = {{
      Array.tabulate(D) {{ i =>
        if (i==DM1) 0.to[X] else {host}
      }}.reduce(_+_)
    }}

    val ii = Array.tabulate(K){{i => i}}
    for(epoch <- 0 until I) {{
      val accum = Array.tabulate(K) {{ i => Array.empty[X](D) }}
      for (k <- 0 until K) {{
        for (d <- 0 until D) {{
          val acc = accum(k)
          acc(d) = 0.to[X]
        }}
      }}
      pts.foreach {{ pt =>
        val dists = cts.map{{ct => dist(pt, ct) }}
        val idx = dists.zip(ii){{(a,b) => pack(a,b) }}.reduce{{(a,b) => {prefer}}}._2
        for (d <- 0 until D) {{
          val acc = accum(idx)
          acc(d) = acc(d) + pt(d)
        }}
      }}
      for (k <- 0 until K) {{
        val acc = accum(k)
        val n = acc(DM1)
        for (d <- 0 until DM1) {{
          val cent = cts(k)
          cent(d) = if (n == 0) 0.to[X] else acc(d)/n
        }}
      }}
    }}

    val gold = cts.flatten
    val cksum = gold.zip(result) {{ (g, r) => g === r }}.reduce {{ _ & _ }}
    println("PASS: " + cksum + " ({class_name})")
    assert(cksum)
  }}
}}
"#,
        accel = block(&accel_lines, "                ", &accel_term),
        host = block(&host_lines, "        ", &host_term),
    ));
    Ok(o)
}
