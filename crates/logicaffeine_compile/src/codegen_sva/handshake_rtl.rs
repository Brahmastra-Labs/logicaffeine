//! Handshake primitives of the spatial IR in silicon: the bounded FIFO (`Handshake::Fifo`), the counting
//! semaphore (`Handshake::Semaphore`'s ready counter / valid flag), the arrive-all barrier
//! (`Handshake::Barrier`) and the one-writer multicast bus (a `Group`). Each emitter is paired with a trace
//! testbench whose expected values come from the Rust model of the same primitive, and the FIFO's bound is
//! proven by k-induction with our own certified SAT — the sequential analogue of the combinational proofs
//! the datapaths carry.
//!
//! `emit_fifo_verilog("chan_fifo", 8, 32)` is byte-identical to `chan_rtl::emit_chan_fifo_verilog()`: the
//! language Pipe IS the parameterized FIFO at its recorded parameters (locked).

use std::collections::VecDeque;

use logicaffeine_proof::bmc::InductionOutcome;

use super::rtl::{NextState, Register, TransitionSystem};
use super::sva_to_verify::{BitVecBoundedOp, BoundedExpr};

/// Bits needed to count `0..=n`.
fn count_bits(n: usize) -> u32 {
    (usize::BITS - n.leading_zeros()).max(1)
}

/// Bits needed to index `0..n`.
fn index_bits(n: usize) -> u32 {
    if n <= 1 {
        1
    } else {
        (usize::BITS - (n - 1).leading_zeros()).max(1)
    }
}

fn pad_to(s: &str, col: usize) -> String {
    let mut out = s.to_string();
    while out.len() < col {
        out.push(' ');
    }
    out
}

/// A bounded FIFO of `depth` entries of `width` bits: FIFO order, `full` refuses a sender, `empty` refuses
/// a receiver, `dout` is the oldest buffered value. Power-of-two depths wrap their pointers naturally; other
/// depths compare explicitly.
pub fn emit_fifo_verilog(name: &str, depth: usize, width: usize) -> String {
    assert!(depth >= 1 && width >= 1);
    let cw = count_bits(depth);
    let pw = index_bits(depth);
    let pow2 = depth.is_power_of_two();
    let w1 = width - 1;
    let mut s = String::new();
    s.push_str(&format!(
        "// {name} — the language Pipe (bounded FIFO channel) in silicon.\n"
    ));
    s.push_str("// FIFO order, bounded (full refuses a sender, empty refuses a receiver) — the discipline of\n");
    s.push_str("// logicaffeine_runtime::channel::Chan. Proven ≡ the software channel by chan_fifo_matches_the_runtime_channel.\n");
    s.push_str(&format!("// DEPTH = {depth}, WIDTH = {width}.\n"));
    s.push_str(&format!("module {name}(\n"));
    let port = |kind: &str, range: &str, ident: &str, tail: &str| -> String {
        let head = if range.is_empty() {
            format!("  {kind}")
        } else {
            format!(
                "  {kind}{}{range}",
                if kind == "input" { "  " } else { " " }
            )
        };
        format!("{}{ident}{tail}\n", pad_to(&head, 20))
    };
    s.push_str(&port("input", "", "clk", ","));
    s.push_str(&port("input", "", "rst", ","));
    s.push_str(&port("input", "", "push", ","));
    s.push_str(&port("input", &format!("[{w1}:0]"), "din", ","));
    s.push_str(&port("input", "", "pop", ","));
    s.push_str(&port(
        "output",
        &format!("[{w1}:0]"),
        "dout",
        ",     // combinational: the oldest buffered value (FIFO front)",
    ));
    s.push_str(&port(
        "output",
        "",
        "pushed",
        ",   // this cycle's push was accepted (buffer had room)",
    ));
    s.push_str(&port(
        "output",
        "",
        "popped",
        ",   // this cycle's pop was accepted (buffer was non-empty)",
    ));
    s.push_str(&port("output", "", "empty", ","));
    s.push_str(&port("output", "", "full", ","));
    s.push_str(&port("output", &format!("[{}:0]", cw - 1), "count", ""));
    s.push_str(");\n");
    let mem_decl = format!("  reg [{w1}:0] ");
    let reg_decl = |bits: u32| pad_to(&format!("  reg [{}:0]", bits - 1), mem_decl.len());
    s.push_str(&format!("{mem_decl}mem [0:{}];\n", depth - 1));
    s.push_str(&format!(
        "{}// 0..{depth}\n",
        pad_to(&format!("{}cnt;", reg_decl(cw)), 30)
    ));
    let wrap = if pow2 {
        format!("wrap mod {depth}")
    } else {
        format!("wrap at {depth} (explicit compare)")
    };
    s.push_str(&format!(
        "{}// {wrap}\n",
        pad_to(&format!("{}head, tail;", reg_decl(pw)), 30)
    ));
    s.push_str(&format!("  assign empty  = (cnt == {cw}'d0);\n"));
    s.push_str(&format!("  assign full   = (cnt == {cw}'d{depth});\n"));
    s.push_str("  assign popped = pop  && !empty;\n");
    s.push_str("  assign pushed = push && !full;\n");
    s.push_str("  assign dout   = mem[head];\n");
    s.push_str("  assign count  = cnt;\n");
    s.push_str("  always @(posedge clk) begin\n");
    s.push_str("    if (rst) begin\n");
    s.push_str(&format!(
        "      cnt <= {cw}'d0; head <= {pw}'d0; tail <= {pw}'d0;\n"
    ));
    s.push_str("    end else begin\n");
    if pow2 {
        s.push_str(&format!("      if (popped) head <= head + {pw}'d1;\n"));
        s.push_str(&format!(
            "      if (pushed) begin mem[tail] <= din; tail <= tail + {pw}'d1; end\n"
        ));
    } else {
        let last = depth - 1;
        s.push_str(&format!(
            "      if (popped) head <= (head == {pw}'d{last}) ? {pw}'d0 : head + {pw}'d1;\n"
        ));
        s.push_str(&format!("      if (pushed) begin mem[tail] <= din; tail <= (tail == {pw}'d{last}) ? {pw}'d0 : tail + {pw}'d1; end\n"));
    }
    s.push_str(&format!(
        "      cnt <= cnt - (popped ? {cw}'d1 : {cw}'d0) + (pushed ? {cw}'d1 : {cw}'d0);\n"
    ));
    s.push_str("    end\n  end\nendmodule\n");
    s
}

/// `0 ≤ cnt ≤ depth` for EVERY push/pop stream, by k-induction over the FIFO's count register exactly as
/// the RTL computes it (`popped = pop ∧ cnt≠0`, `pushed = push ∧ cnt≠depth`).
pub fn fifo_bound_certified(depth: usize, width: usize) -> bool {
    let _ = width;
    let cw = count_bits(depth);
    let cnt = || Box::new(BoundedExpr::BitVecVar("cnt".into(), cw));
    let k = |v: u64| {
        Box::new(BoundedExpr::BitVecConst {
            width: cw,
            value: v,
        })
    };
    let bv = |op, l: Box<BoundedExpr>, r: Box<BoundedExpr>| {
        Box::new(BoundedExpr::BitVecBinary {
            op,
            left: l,
            right: r,
        })
    };
    let eqk = |v: u64| *bv(BitVecBoundedOp::Eq, cnt(), k(v));
    let and = |a: BoundedExpr, b: BoundedExpr| BoundedExpr::And(Box::new(a), Box::new(b));
    let not = |a: BoundedExpr| BoundedExpr::Not(Box::new(a));
    let popped = and(BoundedExpr::Var("pop".into()), not(eqk(0)));
    let pushed = and(BoundedExpr::Var("push".into()), not(eqk(depth as u64)));
    let next = NextState::Ite {
        cond: popped,
        then_: Box::new(NextState::Ite {
            cond: pushed.clone(),
            then_: Box::new(NextState::Simple(*cnt())),
            else_: Box::new(NextState::Simple(*bv(BitVecBoundedOp::Sub, cnt(), k(1)))),
        }),
        else_: Box::new(NextState::Ite {
            cond: pushed,
            then_: Box::new(NextState::Simple(*bv(BitVecBoundedOp::Add, cnt(), k(1)))),
            else_: Box::new(NextState::Simple(*cnt())),
        }),
    };
    let bound_ok = not(*bv(BitVecBoundedOp::ULt, k(depth as u64), cnt()));
    let ts = TransitionSystem {
        registers: vec![Register {
            name: "cnt".into(),
            width: cw,
            init: Some(0),
            next,
        }],
        property: bound_ok,
    };
    ts.prove_invariant(1) == InductionOutcome::Proven
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A `width`-bit hex literal from a seeded stream (works past 64 bits).
fn hex_value(state: &mut u64, width: usize) -> String {
    let digits = width.div_ceil(4);
    let mut s = String::new();
    for i in 0..digits {
        let nib = (splitmix(state) & 0xF) as u32;
        let nib = if i == 0 {
            let top_bits = width - (digits - 1) * 4;
            nib & ((1u32 << top_bits) - 1)
        } else {
            nib
        };
        s.push(std::char::from_digit(nib, 16).unwrap());
    }
    s
}

/// Scripted push/pop trace against a `VecDeque` model: every cycle the DUT's `pushed`, `popped`, `count` and
/// (when popped) `dout` must equal the model's. Prints `PASS:` or the first mismatch.
pub fn emit_fifo_trace_testbench(
    name: &str,
    depth: usize,
    width: usize,
    steps: usize,
    seed: u64,
) -> String {
    let mut st = seed.wrapping_mul(0xA24B_AED4_963E_E407).wrapping_add(1);
    let mut model: VecDeque<String> = VecDeque::new();
    let mut lines = Vec::new();
    let w1 = width - 1;
    let cw = count_bits(depth);
    for _ in 0..steps {
        let r = splitmix(&mut st);
        let push = (r & 3) != 0;
        let pop = ((r >> 2) & 3) == 0 || ((r >> 4) & 1) == 1 && model.len() > depth / 2;
        let din = hex_value(&mut st, width);
        let popped = pop && !model.is_empty();
        let pushed = push && model.len() < depth;
        // `count` and `dout` are read BEFORE the clock edge, so the expectation is the state the step starts
        // from; `pushed`/`popped` are the combinational accept flags of the step itself.
        let count_before = model.len();
        let dout = if popped {
            model.pop_front().unwrap()
        } else {
            String::new()
        };
        if pushed {
            model.push_back(din.clone());
        }
        lines.push((push, pop, din, pushed, popped, dout, count_before));
    }
    let mut s = String::new();
    s.push_str(&format!("`timescale 1ns/1ps\nmodule tb_{name};\n  reg clk = 0, rst = 1, push = 0, pop = 0;\n  reg [{w1}:0] din = 0;\n  wire [{w1}:0] dout;\n  wire pushed, popped, empty, full;\n  wire [{}:0] count;\n", cw - 1));
    s.push_str(&format!("  {name} dut(.clk(clk), .rst(rst), .push(push), .din(din), .pop(pop), .dout(dout), .pushed(pushed), .popped(popped), .empty(empty), .full(full), .count(count));\n"));
    s.push_str("  integer fails = 0;\n  always #5 clk = ~clk;\n");
    s.push_str("  task step(input p, input q, input [{w}:0] d, input ep, input eq, input [{w}:0] ed, input integer ec, input integer cyc);\n".replace("{w}", &w1.to_string()).as_str());
    s.push_str("    begin\n      push = p; pop = q; din = d;\n      #1;\n");
    s.push_str("      if (pushed !== ep || popped !== eq || count !== ec[{c}:0] || (eq && dout !== ed)) begin\n        $display(\"FAIL at cycle %0d: pushed=%b/%b popped=%b/%b count=%0d/%0d dout=%h/%h\", cyc, pushed, ep, popped, eq, count, ec, dout, ed);\n        fails = fails + 1;\n      end\n".replace("{c}", &(cw - 1).to_string()).as_str());
    s.push_str("      @(posedge clk); #1;\n    end\n  endtask\n");
    s.push_str("  initial begin\n    @(posedge clk); #1 rst = 0;\n");
    for (i, (push, pop, din, pushed, popped, dout, count_before)) in lines.iter().enumerate() {
        let ed = if dout.is_empty() {
            "0".to_string()
        } else {
            dout.clone()
        };
        s.push_str(&format!("    step(1'b{}, 1'b{}, {width}'h{din}, 1'b{}, 1'b{}, {width}'h{ed}, {count_before}, {i});\n", *push as u8, *pop as u8, *pushed as u8, *popped as u8));
    }
    s.push_str("    if (fails == 0) $display(\"PASS: fifo trace\"); else $display(\"FAIL: %0d mismatches\", fails);\n    $finish;\n  end\nendmodule\n");
    s
}

/// A counting semaphore of `width` bits: `set` loads `set_val` (the owner's reset), else `inc` adds `inc_n`
/// (a remote increment); `ge` is `count >= thresh` — the set / increment / wait-threshold discipline every
/// dataflow mesh's semaphores implement.
pub fn emit_semaphore_verilog(name: &str, width: usize) -> String {
    let w1 = width - 1;
    let mut s = String::new();
    s.push_str(&format!(
        "// {name} — a counting semaphore ({width} bits): set wins over inc in the same cycle.\n"
    ));
    s.push_str(&format!("module {name}(\n  input clk,\n  input rst,\n  input inc,\n  input [{w1}:0] inc_n,\n  input set,\n  input [{w1}:0] set_val,\n  input [{w1}:0] thresh,\n  output [{w1}:0] count,\n  output ge\n);\n"));
    s.push_str(&format!(
        "  reg [{w1}:0] cnt;\n  assign count = cnt;\n  assign ge = (cnt >= thresh);\n"
    ));
    s.push_str(&format!("  always @(posedge clk) begin\n    if (rst) cnt <= {width}'d0;\n    else if (set) cnt <= set_val;\n    else if (inc) cnt <= cnt + inc_n;\n  end\nendmodule\n"));
    s
}

/// Scripted set/inc/threshold trace against the Rust model of the semaphore.
pub fn emit_semaphore_trace_testbench(name: &str, width: usize) -> String {
    let w1 = width - 1;
    let mask: u64 = if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    };
    let mut st = 0x5EED_u64;
    let mut cnt: u64 = 0;
    let mut script = Vec::new();
    for i in 0..120 {
        let r = splitmix(&mut st);
        let (set, set_val, inc, inc_n, thresh) = match r % 5 {
            0 => (true, (r >> 8) & 7, false, 0, (r >> 16) & 7),
            1 | 2 => (false, 0, true, 1 + ((r >> 8) & 3), (r >> 16) & 7),
            3 => (true, 0, true, 1, (r >> 16) & 7),
            _ => (false, 0, false, 0, (r >> 16) & 7),
        };
        let ge_before = cnt >= thresh;
        if set {
            cnt = set_val & mask;
        } else if inc {
            cnt = (cnt + inc_n) & mask;
        }
        script.push((i, set, set_val, inc, inc_n, thresh, ge_before, cnt));
    }
    let mut s = String::new();
    s.push_str(&format!("`timescale 1ns/1ps\nmodule tb_{name};\n  reg clk = 0, rst = 1, inc = 0, set = 0;\n  reg [{w1}:0] inc_n = 0, set_val = 0, thresh = 0;\n  wire [{w1}:0] count;\n  wire ge;\n"));
    s.push_str(&format!("  {name} dut(.clk(clk), .rst(rst), .inc(inc), .inc_n(inc_n), .set(set), .set_val(set_val), .thresh(thresh), .count(count), .ge(ge));\n"));
    s.push_str("  integer fails = 0;\n  always #5 clk = ~clk;\n  initial begin\n    @(posedge clk); #1 rst = 0;\n");
    for (i, set, set_val, inc, inc_n, thresh, ge_before, after) in script {
        s.push_str(&format!("    set = {}; set_val = {set_val}; inc = {}; inc_n = {inc_n}; thresh = {thresh}; #1;\n", set as u8, inc as u8));
        s.push_str(&format!("    if (ge !== 1'b{}) begin $display(\"FAIL step {i}: ge=%b expected {}\", ge); fails = fails + 1; end\n", ge_before as u8, ge_before as u8));
        s.push_str(&format!("    @(posedge clk); #1;\n    if (count !== {width}'d{after}) begin $display(\"FAIL step {i}: count=%0d expected {after}\", count); fails = fails + 1; end\n"));
    }
    s.push_str("    if (fails == 0) $display(\"PASS: semaphore trace\"); else $display(\"FAIL: %0d mismatches\", fails);\n    $finish;\n  end\nendmodule\n");
    s
}

/// An arrive-all barrier over `n` parties: arrivals are sticky; the cycle every party has arrived, `released`
/// pulses for one cycle and the barrier re-arms.
pub fn emit_barrier_verilog(name: &str, n: usize) -> String {
    assert!(n >= 1);
    let n1 = n - 1;
    let mut s = String::new();
    s.push_str(&format!("// {name} — an arrive-all barrier over {n} parties (sticky arrivals, one-cycle released, re-arms).\n"));
    s.push_str(&format!("module {name}(\n  input clk,\n  input rst,\n  input [{n1}:0] arrive,\n  output released\n);\n"));
    s.push_str(&format!("  reg [{n1}:0] seen;\n  reg rel;\n  wire all_in = &(seen | arrive);\n  assign released = rel;\n"));
    s.push_str(&format!("  always @(posedge clk) begin\n    if (rst) begin seen <= {n}'d0; rel <= 1'b0; end\n    else if (all_in) begin seen <= {n}'d0; rel <= 1'b1; end\n    else begin seen <= seen | arrive; rel <= 1'b0; end\n  end\nendmodule\n"));
    s
}

/// Staggered arrivals over several rounds: `released` pulses exactly once per round, on the cycle after the
/// last party arrives, and never otherwise.
pub fn emit_barrier_trace_testbench(name: &str, n: usize) -> String {
    let n1 = n - 1;
    let mut st = 0xBA77_u64;
    let mut s = String::new();
    s.push_str(&format!("`timescale 1ns/1ps\nmodule tb_{name};\n  reg clk = 0, rst = 1;\n  reg [{n1}:0] arrive = 0;\n  wire released;\n  {name} dut(.clk(clk), .rst(rst), .arrive(arrive), .released(released));\n  integer fails = 0;\n  always #5 clk = ~clk;\n  initial begin\n    @(posedge clk); #1 rst = 0;\n"));
    for round in 0..4 {
        // Model: sticky set; released the cycle after the last arrival.
        let mut seen: u64 = 0;
        let full: u64 = if n >= 64 { u64::MAX } else { (1u64 << n) - 1 };
        let mut order: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            let j = (splitmix(&mut st) as usize) % (i + 1);
            order.swap(i, j);
        }
        let mut idx = 0;
        while seen != full {
            // Arrive one or two parties this cycle, sometimes none.
            let r = splitmix(&mut st);
            let mut arrive: u64 = 0;
            let how_many = if r % 4 == 0 {
                0
            } else if r % 4 == 1 && idx + 1 < n {
                2
            } else {
                1
            };
            for _ in 0..how_many {
                if idx < n {
                    arrive |= 1u64 << order[idx];
                    idx += 1;
                }
            }
            let completes = (seen | arrive) == full;
            s.push_str(&format!("    arrive = {n}'h{arrive:X}; #1;\n    if (released !== 1'b0) begin $display(\"FAIL round {round}: early released\"); fails = fails + 1; end\n    @(posedge clk); #1;\n"));
            if completes {
                s.push_str(&format!("    arrive = {n}'d0; if (released !== 1'b1) begin $display(\"FAIL round {round}: no released after the last arrival\"); fails = fails + 1; end\n    @(posedge clk); #1;\n    if (released !== 1'b0) begin $display(\"FAIL round {round}: released did not drop\"); fails = fails + 1; end\n"));
                seen = full;
            } else {
                seen |= arrive;
                s.push_str(&format!("    if (released !== 1'b0) begin $display(\"FAIL round {round}: released before all arrived\"); fails = fails + 1; end\n"));
            }
        }
    }
    s.push_str("    if (fails == 0) $display(\"PASS: barrier trace\"); else $display(\"FAIL: %0d mismatches\", fails);\n    $finish;\n  end\nendmodule\n");
    s
}

/// A one-writer multicast bus: on `send`, the next cycle every one of the `n` receivers sees `push` with the
/// same `dout` — the multicast landing of a `Group` (each receiver pushes it into its own FIFO).
pub fn emit_mcast_bus_verilog(name: &str, width: usize, n: usize) -> String {
    let (w1, n1) = (width - 1, n - 1);
    let mut s = String::new();
    s.push_str(&format!(
        "// {name} — a one-writer multicast bus to {n} receivers ({width} bits), registered.\n"
    ));
    s.push_str(&format!("module {name}(\n  input clk,\n  input rst,\n  input send,\n  input [{w1}:0] din,\n  output [{n1}:0] push,\n  output [{w1}:0] dout\n);\n"));
    s.push_str(&format!("  reg strobe;\n  reg [{w1}:0] data;\n  assign push = {{{n}{{strobe}}}};\n  assign dout = data;\n"));
    s.push_str(&format!("  always @(posedge clk) begin\n    if (rst) begin strobe <= 1'b0; data <= {width}'d0; end\n    else begin strobe <= send; if (send) data <= din; end\n  end\nendmodule\n"));
    s
}

/// Sends, idles and back-to-back sends: every receiver's `push` and `dout` follow the writer one cycle later.
pub fn emit_mcast_bus_trace_testbench(name: &str, width: usize, n: usize) -> String {
    let (w1, n1) = (width - 1, n - 1);
    let mut st = 0xB05_u64;
    let mut s = String::new();
    s.push_str(&format!("`timescale 1ns/1ps\nmodule tb_{name};\n  reg clk = 0, rst = 1, send = 0;\n  reg [{w1}:0] din = 0;\n  wire [{n1}:0] push;\n  wire [{w1}:0] dout;\n  {name} dut(.clk(clk), .rst(rst), .send(send), .din(din), .push(push), .dout(dout));\n  integer fails = 0;\n  always #5 clk = ~clk;\n  initial begin\n    @(posedge clk); #1 rst = 0;\n"));
    let all_ones = format!(
        "{n}'h{:X}",
        if n >= 64 { u64::MAX } else { (1u64 << n) - 1 }
    );
    let mut last_sent: Option<String> = None;
    for i in 0..60 {
        let r = splitmix(&mut st);
        let send = r % 3 != 0;
        let din = hex_value(&mut st, width);
        s.push_str(&format!(
            "    send = {}; din = {width}'h{din};\n    @(posedge clk); #1;\n",
            send as u8
        ));
        if send {
            s.push_str(&format!("    if (push !== {all_ones} || dout !== {width}'h{din}) begin $display(\"FAIL step {i}: push=%h dout=%h\", push, dout); fails = fails + 1; end\n"));
            last_sent = Some(din);
        } else {
            s.push_str(&format!("    if (push !== {n}'d0) begin $display(\"FAIL step {i}: spurious push\"); fails = fails + 1; end\n"));
            if let Some(prev) = &last_sent {
                s.push_str(&format!("    if (dout !== {width}'h{prev}) begin $display(\"FAIL step {i}: dout drifted\"); fails = fails + 1; end\n"));
            }
        }
    }
    s.push_str("    if (fails == 0) $display(\"PASS: mcast bus trace\"); else $display(\"FAIL: %0d mismatches\", fails);\n    $finish;\n  end\nendmodule\n");
    s
}
