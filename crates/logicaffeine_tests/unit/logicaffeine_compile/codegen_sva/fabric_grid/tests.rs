use super::*;

/// S2+S3 — the PLACER + ROUTER is certified: the REAL compiled Horner residual, mapped onto
/// the tile array by `route_straight_line`, computes exactly the abstract datapath (and the VM)
/// across the full input range; and a multiply-free routed grid is proven EQUAL to its datapath
/// EXHAUSTIVELY by our own SAT — so place-and-route is semantics-preserving.
#[test]
fn a_routed_grid_is_certified_equal_to_the_fabric_datapath_by_our_sat() {
    use super::super::fabric_rtl::{
        eval_fabric, fabric_to_bounded, poly_program, residual_from_source,
    };
    use crate::codegen_sva::sva_to_verify::BoundedExpr;
    use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
    use logicaffeine_proof::ProofExpr;

    // The Horner cubic residual placed+routed onto the grid == the datapath, over full range.
    let (ops, result, n_in) = residual_from_source(&poly_program("Show poly(4)."), "poly");
    let g = route_straight_line(&ops, n_in, result);
    assert!(
        g.rows > 2 && g.cols > 1,
        "a real 2-D tile array (not a line)"
    );
    for x in [0i64, 1, -1, 2, -3, 7, -9, 50, -64, 127, i64::MIN, i64::MAX] {
        assert_eq!(
            eval_grid(&g, &[x]),
            eval_fabric(&ops, &[x], result),
            "the routed grid must equal the abstract datapath at x={x}"
        );
    }

    // A multiply-free residual (`x + x + x + 5`) placed+routed, certified EXHAUSTIVELY.
    let src = "## To lin3 (x: Int) -> Int:\n    Return x + x + x + 5.\n## Main\n    Show lin3(0).";
    let (ops2, res2, _) = residual_from_source(src, "lin3");
    let g2 = route_straight_line(&ops2, 1, res2);
    const PW: u32 = 8;
    let grid = grid_to_bounded(&g2, PW);
    let datapath = fabric_to_bounded(&ops2, res2, PW);
    let claim = BoundedExpr::Eq(Box::new(grid), Box::new(datapath));
    let p = super::super::bitblast::lower_bool(&claim).expect("claim lowers");
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(p))),
        UnsatOutcome::Refuted,
        "the placed+routed grid must equal the abstract datapath for ALL inputs (certified P&R)"
    );
}

fn silicon_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon")
}

/// Run `poly(x)` through the REAL VM and reduce its exact result to the machine word (mod 2⁶⁴)
/// — the honest oracle for a 64-bit circuit.
fn vm_poly(x: i64) -> u64 {
    use super::super::fabric_rtl::POLY_BODY;
    let src = format!(
        "## To poly (x: Int) -> Int:\n    Return {POLY_BODY}.\n## Main\n    Show poly({x})."
    );
    let out = crate::compile::vm_outcome(&src);
    let s = out.output.trim();
    let (neg, digits) = s.strip_prefix('-').map_or((false, s), |d| (true, d));
    let mut acc: u64 = 0;
    for ch in digits.bytes() {
        acc = acc.wrapping_mul(10).wrapping_add((ch - b'0') as u64);
    }
    if neg {
        acc.wrapping_neg()
    } else {
        acc
    }
}

/// S4 — emit the placed+routed Horner grid (`grid_horner.v`), its testbench, and the VM oracle
/// vectors (golden; regenerate with `FABRIC_RECORD=1`). The `iverilog` gate clocks the tile
/// array against the RUNNING VM's output — the reconfigurable-array culmination of Futamura→si.
#[test]
fn grid_artifacts_are_current() {
    use super::super::fabric_rtl::{poly_program, residual_from_source};
    const N: usize = 48;
    let (ops, result, n_in) = residual_from_source(&poly_program("Show poly(4)."), "poly");
    let g = route_straight_line(&ops, n_in, result);
    let mut hin = String::new();
    let mut hout = String::new();
    for k in 0..N {
        let x = (k as i64) - 24;
        hin.push_str(&format!("{:016x}\n", x as u64));
        hout.push_str(&format!("{:016x}\n", vm_poly(x)));
    }
    // Time-multiplexed array: ONE grid running lin (`3x+7`, ctx=1) and sq (`x²+1`, ctx=0).
    let (la, lr, ln) = residual_from_source(
        "## To lin (x: Int) -> Int:\n    Return 3 * x + 7.\n## Main\n    Show lin(0).",
        "lin",
    );
    let (sa, sr, sn) = residual_from_source(
        "## To sq (x: Int) -> Int:\n    Return x * x + 1.\n## Main\n    Show sq(0).",
        "sq",
    );
    let (gl, gs) = (
        route_straight_line(&la, ln, lr),
        route_straight_line(&sa, sn, sr),
    );
    let mut tin = String::new();
    let mut tlin = String::new();
    let mut tsq = String::new();
    for k in 0..N {
        let x = (k as i64) - 24;
        tin.push_str(&format!("{:016x}\n", x as u64));
        tlin.push_str(&format!(
            "{:016x}\n",
            (3i64.wrapping_mul(x).wrapping_add(7)) as u64
        ));
        tsq.push_str(&format!(
            "{:016x}\n",
            (x.wrapping_mul(x).wrapping_add(1)) as u64
        ));
    }
    let tb_tmux = format!(
        r#"// tb_tmux_linsq — ONE tile array, two fabrics: ctx=1 runs lin (3x+7), ctx=0 runs sq (x²+1).
`timescale 1ns/1ps
module tb_tmux_linsq;
  reg [63:0] in0; reg ctx;
  reg [63:0] oin [0:{last}]; reg [63:0] olin [0:{last}]; reg [63:0] osq [0:{last}];
  wire [63:0] result;
  integer i, errors = 0;
  tmux_linsq dut(.in0(in0), .ctx(ctx), .result(result));
  initial begin
    $readmemh("tmux_in.hex", oin); $readmemh("tmux_lin.hex", olin); $readmemh("tmux_sq.hex", osq);
    for (i = 0; i < {N}; i = i + 1) begin
      in0 = oin[i]; ctx = 1'b1; #1;
      if (result !== olin[i]) begin errors = errors + 1; $display("FAIL lin %0d", i); end
      ctx = 1'b0; #1;
      if (result !== osq[i]) begin errors = errors + 1; $display("FAIL sq %0d", i); end
    end
    if (errors == 0) $display("PASS: {N} time-mux runs match (one array runs two fabrics by ctx)");
    else $display("FAIL: %0d tmux mismatches", errors);
    $finish;
  end
endmodule
"#,
        last = N - 1,
    );
    let files = [
        ("grid_horner.v".to_string(), emit_grid_verilog("horner", &g)),
        (
            "tb_grid_horner.v".to_string(),
            emit_grid_testbench("horner", 1, N),
        ),
        ("grid_horner_in.hex".to_string(), hin),
        ("grid_horner_out.hex".to_string(), hout),
        (
            "tmux_linsq.v".to_string(),
            emit_time_multiplexed("linsq", &gl, &gs),
        ),
        ("tb_tmux_linsq.v".to_string(), tb_tmux),
        ("tmux_in.hex".to_string(), tin),
        ("tmux_lin.hex".to_string(), tlin),
        ("tmux_sq.hex".to_string(), tsq),
    ];
    let dir = silicon_dir();
    if std::env::var("FABRIC_RECORD").is_ok() {
        for (n, b) in &files {
            std::fs::write(dir.join(n), b).expect("write");
        }
        return;
    }
    for (n, b) in &files {
        let got = std::fs::read_to_string(dir.join(n)).unwrap_or_else(|e| {
            panic!("missing {n} ({e}) — record: FABRIC_RECORD=1 cargo test grid_artifacts")
        });
        assert_eq!(&got, b, "{n} is stale — regenerate with FABRIC_RECORD=1");
    }
}

/// Run a counted loop with the tile ARRAY as its body: the controller (counter) latches the
/// grid's next-state outputs each iteration — a loop running on a placed+routed CGRA.
fn run_loop_on_grid(lf: &super::super::fabric_rtl::LoopFabric, grid: &Grid, params: &[i64]) -> i64 {
    let empty: [i64; 0] = [];
    let mut state: Vec<i64> = lf.init.iter().map(|s| s.eval(params, &empty)).collect();
    let lo = lf.lo.eval(params, &empty);
    let hi = lf.hi.eval(params, &empty);
    let invs: Vec<i64> = lf
        .invariants
        .iter()
        .map(|s| s.eval(params, &empty))
        .collect();
    let mut iv = lo;
    while iv <= hi {
        let mut bin: Vec<i64> = state.clone();
        bin.push(iv);
        bin.extend_from_slice(&invs);
        state = eval_grid_all(grid, &bin); // the tile array computes the next-state
        if iv == hi {
            break;
        }
        iv = iv.wrapping_add(1);
    }
    state[lf.result_state]
}

/// P1 — a LOOP BODY runs on the tile array: the real compiled body of `sumto` (one Arith tile)
/// and `altsum` (a data-dependent branch → a mux tile) is placed+routed onto the grid; each
/// next-state output is proven == the fabric body's next-state EXHAUSTIVELY by our SAT (so P&R
/// of the loop body is semantics-preserving), and the whole loop clocked with the grid as its
/// body computes the VM's result. Control flow, placed and routed on a reconfigurable array.
#[test]
fn a_loop_body_mapped_onto_the_grid_is_certified_and_runs_the_vm() {
    use super::super::fabric_rtl::{
        eval_loop_fabric, loop_fabric_from_source, loop_next_to_bounded,
    };
    use crate::codegen_sva::sva_to_verify::BoundedExpr;
    use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
    use logicaffeine_proof::ProofExpr;

    let sumto = "## To sumto (n: Int) -> Int:\n    Let total be 0.\n    Repeat for i from 1 to n:\n        Set total to total + i.\n    Return total.";
    let altsum = "## To altsum (n: Int) -> Int:\n    Let s be 0.\n    Repeat for i from 1 to n:\n        If i is greater than 3:\n            Set s to s + i.\n        Otherwise:\n            Set s to s - i.\n    Return s.";

    for (decl, name) in [(sumto, "sumto"), (altsum, "altsum")] {
        let lf = loop_fabric_from_source(&format!("{decl}\n## Main\n    Show {name}(0)."), name);
        let n_in = lf.n_state + 1 + lf.invariants.len();
        let grid = route_loop_body(&lf.body, n_in, &lf.next);
        assert_eq!(
            grid.all_outs().len(),
            lf.n_state,
            "{name}: one grid output per accumulator"
        );
        // altsum's branchy body must place a mux tile onto the array.
        if name == "altsum" {
            let has_mux = grid
                .tiles
                .iter()
                .flatten()
                .any(|t| matches!(t.alu, Some(CellKind::Select(..))));
            assert!(
                has_mux,
                "altsum's branch must map to a mux tile on the grid"
            );
        }

        // Certify each grid output == the fabric body's next-state (same variable namespace).
        const PW: u32 = 8;
        let ns = lf.n_state;
        let name_in = move |k: usize| -> String {
            if k < ns {
                format!("s{k}")
            } else if k == ns {
                "iv".to_string()
            } else {
                format!("inv{}", k - ns - 1)
            }
        };
        for j in 0..lf.n_state {
            let g_out = grid_output_to_bounded(&grid, j, PW, &name_in);
            let f_next = loop_next_to_bounded(&lf, j, PW);
            let claim = BoundedExpr::Eq(Box::new(g_out), Box::new(f_next));
            let p = super::super::bitblast::lower_bool(&claim).expect("claim lowers");
            assert_eq!(
                prove_unsat(&ProofExpr::Not(Box::new(p))),
                UnsatOutcome::Refuted,
                "{name}: grid body output {j} must equal the fabric next-state (certified P&R)"
            );
        }

        // The whole loop, running on the grid, matches the VM (== the hand-wired fabric loop).
        for n in [0i64, 1, 2, 3, 5, 8, 20, -1] {
            assert_eq!(
                run_loop_on_grid(&lf, &grid, &[n]),
                eval_loop_fabric(&lf, &[n]),
                "{name}: loop-on-grid != VM at n={n}"
            );
        }
    }

    // MULTI-OUTPUT: a while body with TWO accumulators (`accw`: total += a; a += 1) maps to a
    // grid with two next-state outputs; both are certified == the fabric body's next-state.
    use super::super::fabric_rtl::{while_fabric_from_source, while_next_to_bounded};
    let accw = "## To accw (n: Int) -> Int:\n    Let a be 0.\n    Let total be 0.\n    While a is less than n:\n        Set total to total + a.\n        Set a to a + 1.\n    Return total.";
    let wf = while_fabric_from_source(&format!("{accw}\n## Main\n    Show accw(0)."), "accw");
    let n_in = wf.n_state + wf.invariants.len(); // while body: state + invariants, no iv
    let grid = route_loop_body(&wf.body, n_in, &wf.next);
    assert_eq!(
        grid.all_outs().len(),
        2,
        "accw has TWO accumulators → two grid outputs"
    );
    const PW: u32 = 8;
    let ns = wf.n_state;
    let name_in = move |k: usize| -> String {
        if k < ns {
            format!("s{k}")
        } else {
            format!("inv{}", k - ns)
        }
    };
    for j in 0..wf.n_state {
        let g_out = grid_output_to_bounded(&grid, j, PW, &name_in);
        let f_next = while_next_to_bounded(&wf, j, PW);
        let claim = BoundedExpr::Eq(Box::new(g_out), Box::new(f_next));
        let p = super::super::bitblast::lower_bool(&claim).expect("claim lowers");
        assert_eq!(
            prove_unsat(&ProofExpr::Not(Box::new(p))),
            UnsatOutcome::Refuted,
            "accw multi-output grid body {j} must equal the fabric next-state (certified)"
        );
    }
}

/// P2 — the AREA-MINIMIZING placer packs level-0 cells and stays certified. On a fabric with
/// input parallelism (`(a+b)+(c+d)`, two independent level-0 adds) the level scheduler uses
/// strictly fewer rows AND smaller area than the naive diagonal placer; on the Horner chain it
/// is never worse. Both packings compute the datapath, and the compact one is re-certified ==
/// the datapath EXHAUSTIVELY by our SAT — a smaller array, still provably correct.
#[test]
fn the_area_minimizing_placer_packs_and_stays_certified() {
    use super::super::fabric_rtl::{
        eval_fabric, fabric_to_bounded, poly_program, residual_from_source,
    };
    use crate::codegen_sva::sva_to_verify::BoundedExpr;
    use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
    use logicaffeine_proof::ProofExpr;

    let src = "## To par (a: Int, b: Int, c: Int, d: Int) -> Int:\n    Return (a + b) + (c + d).\n## Main\n    Show par(0, 0, 0, 0).";
    let (ops, result, n_in) = residual_from_source(src, "par");
    let naive = route_straight_line(&ops, n_in, result);
    let leveled = route_leveled(&ops, n_in, result);
    let (nr, nc, _nu) = grid_area(&naive);
    let (lr, lc, _lu) = grid_area(&leveled);
    assert!(lr < nr, "leveled must use fewer rows ({lr} < {nr})");
    assert!(
        lr * lc < nr * nc,
        "leveled must have smaller area ({}×{} < {}×{})",
        lr,
        lc,
        nr,
        nc
    );

    for v in [
        [1i64, 2, 3, 4],
        [0, 0, 0, 0],
        [10, -5, 7, -3],
        [i64::MAX, 1, 1, 1],
    ] {
        let want = eval_fabric(&ops, &v, result);
        assert_eq!(eval_grid(&naive, &v), want, "naive grid != datapath");
        assert_eq!(
            eval_grid(&leveled, &v),
            want,
            "leveled grid != datapath at {v:?}"
        );
    }

    const PW: u32 = 8;
    let claim = BoundedExpr::Eq(
        Box::new(grid_to_bounded(&leveled, PW)),
        Box::new(fabric_to_bounded(&ops, result, PW)),
    );
    let p = super::super::bitblast::lower_bool(&claim).expect("claim lowers");
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(p))),
        UnsatOutcome::Refuted,
        "the area-minimized placement must still equal the datapath for ALL inputs (certified)"
    );

    // Horner (a chain) also packs its one level-0 cell — never worse than naive.
    let (hops, hres, hn) = residual_from_source(&poly_program("Show poly(4)."), "poly");
    assert!(
        grid_area(&route_leveled(&hops, hn, hres)).0
            <= grid_area(&route_straight_line(&hops, hn, hres)).0,
        "Horner leveled rows must be <= naive"
    );
    let hl = route_leveled(&hops, hn, hres);
    for x in [0i64, 3, -7, 50, i64::MIN] {
        assert_eq!(
            eval_grid(&hl, &[x]),
            eval_fabric(&hops, &[x], hres),
            "Horner leveled != datapath"
        );
    }
}

/// P3 — ONE tile array runs TWO fabrics by reconfiguration. A single physical grid is muxed on
/// `ctx` between fabric A (`3·x+7`) and fabric B (`x²+1`); it is proven EXHAUSTIVELY by our SAT
/// that `ctx=1 ⇒ the array ≡ fabric A` and `ctx=0 ⇒ ≡ fabric B`. The reconfigurable-array
/// payoff: the same silicon runs any program by loading a configuration bit.
#[test]
fn a_time_multiplexed_array_runs_two_fabrics_certified_by_our_sat() {
    use super::super::fabric_rtl::residual_from_source;
    use crate::codegen_sva::sva_to_verify::BoundedExpr;
    use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
    use logicaffeine_proof::ProofExpr;

    let (aops, ares, an) = residual_from_source(
        "## To lin (x: Int) -> Int:\n    Return 3 * x + 7.\n## Main\n    Show lin(0).",
        "lin",
    );
    let (bops, bres, bn) = residual_from_source(
        "## To sq (x: Int) -> Int:\n    Return x * x + 1.\n## Main\n    Show sq(0).",
        "sq",
    );
    let a = route_straight_line(&aops, an, ares);
    let b = route_straight_line(&bops, bn, bres);
    assert_eq!(
        (a.rows, a.cols),
        (b.rows, b.cols),
        "the two fabrics share array dimensions"
    );

    const PW: u32 = 8;
    let lb = |e: BoundedExpr| super::super::bitblast::lower_bool(&e).expect("lowers");
    let combined = time_mux_bounded(&a, &b, PW);
    let a_out = grid_to_bounded(&a, PW);
    let b_out = grid_to_bounded(&b, PW);
    let ctx_is_a = lb(BoundedExpr::BitVecBinary {
        op: crate::codegen_sva::sva_to_verify::BitVecBoundedOp::Eq,
        left: Box::new(BoundedExpr::BitVecVar("ctx".to_string(), 1)),
        right: Box::new(BoundedExpr::BitVecConst { width: 1, value: 1 }),
    });
    // ctx=1 ⇒ combined == A: ¬(ctx=1) ∨ (combined == A).
    let claim_a = ProofExpr::Or(
        Box::new(ProofExpr::Not(Box::new(ctx_is_a.clone()))),
        Box::new(lb(BoundedExpr::Eq(
            Box::new(combined.clone()),
            Box::new(a_out),
        ))),
    );
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(claim_a))),
        UnsatOutcome::Refuted,
        "ctx=1 must make the array equal fabric A for ALL inputs"
    );
    // ctx=0 ⇒ combined == B: (ctx=1) ∨ (combined == B).
    let claim_b = ProofExpr::Or(
        Box::new(ctx_is_a),
        Box::new(lb(BoundedExpr::Eq(Box::new(combined), Box::new(b_out)))),
    );
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(claim_b))),
        UnsatOutcome::Refuted,
        "ctx=0 must make the array equal fabric B for ALL inputs"
    );

    // The emitted RTL is one array with a `ctx` reconfiguration input muxing every channel.
    let v = emit_time_multiplexed("linsq", &a, &b);
    assert!(
        v.contains("input ctx") && v.contains("ctx ?"),
        "the RTL must mux on ctx"
    );
}

/// The switchbox machinery is sound: a small HAND-PLACED grid — a passthrough that routes an
/// input across two tiles, an Add tile, and a Sub tile — computes exactly what its placement
/// says, proven EXHAUSTIVELY by our own SAT. This validates eval/emit/SAT before the router.
#[test]
fn a_hand_placed_grid_is_certified_equal_to_its_datapath_by_our_sat() {
    use crate::codegen_sva::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
    use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
    use logicaffeine_proof::ProofExpr;

    // 2×3 grid computing (in0 + in1) - in2, with in0 ROUTED east one tile before the add so
    // the two channels genuinely cross a routed value.
    let mut g = Grid::new(2, 3, 3);
    // (0,0): source in0 onto its east channel (route it right).
    g.set(
        0,
        0,
        Tile {
            alu: None,
            vs: Drive::Pass(Src2::Zero),
            he: Drive::Pass(Src2::In(0)),
        },
    );
    // (0,1): Add tile — W = in0 (from (0,0).he), other operand in1 — result south.
    g.set(
        0,
        1,
        Tile {
            alu: Some(CellKind::Arith(AluOp::Add, Src2::W, Src2::In(1))),
            vs: Drive::Alu,
            he: Drive::Pass(Src2::Zero),
        },
    );
    // (1,1): Sub tile — N = (in0+in1) from (0,1).vs, minus in2 — result south (the output).
    g.set(
        1,
        1,
        Tile {
            alu: Some(CellKind::Arith(AluOp::Sub, Src2::N, Src2::In(2))),
            vs: Drive::Alu,
            he: Drive::Pass(Src2::Zero),
        },
    );
    g.out = (1, 1, Chan::Vs);

    // eval sanity.
    assert_eq!(eval_grid(&g, &[7, 5, 3]), (7 + 5) - 3);
    assert_eq!(eval_grid(&g, &[0, 0, 0]), 0);

    // The emitted RTL is nearest-neighbour (only vs_{r-1}/he_{c-1} references), no global bus.
    let v = emit_grid_verilog("t", &g);
    assert!(
        v.contains("vs_0_1") && v.contains("he_0_0"),
        "grid RTL must wire tile channels"
    );

    // Certify: the placed+routed grid == the datapath spec `(in0+in1)-in2`, EXHAUSTIVELY.
    const PW: u32 = 8;
    let grid_out = grid_to_bounded(&g, PW);
    let bv = |k: usize| Box::new(BoundedExpr::BitVecVar(format!("in{k}"), PW));
    let bin = |o, l, r| {
        Box::new(BoundedExpr::BitVecBinary {
            op: o,
            left: l,
            right: r,
        })
    };
    let spec = *bin(
        BitVecBoundedOp::Sub,
        bin(BitVecBoundedOp::Add, bv(0), bv(1)),
        bv(2),
    );
    let claim = BoundedExpr::Eq(Box::new(grid_out), Box::new(spec));
    let p = super::super::bitblast::lower_bool(&claim).expect("claim lowers");
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(p))),
        UnsatOutcome::Refuted,
        "the placed+routed grid must equal its datapath for ALL inputs (certified P&R)"
    );
}
