# RDU demo — LOGOS programs lowered for a SambaNova RDU

A SambaNova RDU (Reconfigurable Dataflow Unit) is a dataflow accelerator. Instead of running a
stream of instructions, it is configured: a program is laid out across a mesh of compute units
(PCUs) and memory units (PMUs), and data flows through that layout. `largo emit rdu` takes an
ordinary LOGOS program, picks one of three ways to lower it (a *lane*) from the program's shape,
places the result on a model of the chip, and writes the placement with certificates you can
re-check. The six programs here cover all three lanes.

Everything runs on your machine, and nothing in this directory has run on an RDU. The chip model is
built from SambaNova's published SN50 and SN40L figures, with the gaps filled by models, and every
number in the artifacts is labeled `modeled` or `published` (the facts live in
`crates/logicaffeine_compile/src/spatial/rdu_chip.rs`). Turning a placement into SambaNova's own
configuration needs their SDK: `--bridge sambaflow` refuses by name until that SDK is wired in.

## Run it

From the repository root (or with `largo emit rdu …` if `largo` is installed):

```bash
cargo run -p logicaffeine-cli -- emit rdu --chip sn50 examples/rdu_demo/dot.lg
cargo run -p logicaffeine-cli -- emit rdu --chip sn50 examples/rdu_demo/fib.lg
cargo run -p logicaffeine-cli -- emit rdu --chip sn50 examples/rdu_demo/primes.lg
cargo run -p logicaffeine-cli -- emit rdu --chip sn50 examples/rdu_demo/gemm.lg -o out/gemm
cargo run -p logicaffeine-cli -- emit rdu --chip sn50 examples/rdu_demo/qwen38_tiny.lg -o out/qwen
```

Bare, the command prints the placement and names the lane and chip on stderr
(`rdu lane: loop chip: sn50 …`). With `-o DIR` it writes `neutral.json`, `placement.txt`,
`certificate.json` and `report.json`. `--chip sn40l` targets the SN40L instead.

## The three lanes

| lane | chosen when | what comes out |
|---|---|---|
| `tensor` | the program declares a tensor (`Let A be a R by C tensor of …`), or the tensor recognizer reads its loops as a matrix multiply, an elementwise op, a SwiGLU activation or a fused row operation such as RMSNorm | the program in the spatial IR (loads, tile compute, stores), placed and scheduled on the mesh, written as the neutral placement with its certificate |
| `loop` | the first function is a counted loop (`Repeat for i from lo to hi`) with scalar state carried between iterations | the Plasticine lowering: the loop's initiation interval under the calibrated PCU model, a placement with static routing, a SAT certificate that the routed datapath computes the loop body (ALU-only bodies), the VM's answer as the reference, and Spatial source for the public Plasticine toolchain |
| `whole-program` | everything else: the first function as a decision program (if/else, collections, structs, enums, helper calls inlined), or a `## Main` of `Show`s | one `Scalar` stage applied element by element over an input vector (`y[i] = f(x[i])`) on the PCU's SIMD lanes, the choreography certificate, a per-PE SAT certificate where the body is ALU operations and selects, and the neutral placement |

The terms in that table:

- **Plasticine** is Stanford's research dataflow architecture (Prabhakar et al., ISCA 2017). Where
  SambaNova's public record says nothing about a PCU, a PMU or the mesh, the chip model uses
  Plasticine's published unit model. Its public toolchain runs Spatial source through PIR and
  plastiroute to Tungsten, a cycle-level simulator (`scripts/plasticine/`).
- **Initiation interval (II)** is the number of cycles between the starts of consecutive loop
  iterations. The PCU model is calibrated against Tungsten's cycle counts: a value carried from one
  iteration to the next costs the pipeline depth plus one, so 7 cycles on the six-stage PCU.
- **The choreography certificate** is re-checkable evidence that the placed schedule cannot
  deadlock, that no buffer overflows its depth, and, stage by stage, that each integer datapath
  computes its operation. Re-checking it trusts a small refutation checker, a topological-order
  verifier and integer arithmetic, never the passes that produced the schedule.
- **A PE** is a processing element; **the neutral placement** (`logos.spatial.neutral` v2) is the
  vendor-neutral JSON description of the placed program, the input an SDK bridge ingests.

## The corpus

| program | lane | what it shows |
|---|---|---|
| `dot.lg` | `loop` | a dot product of two affine streams over a counted range: II 7 (one carried register on the six-stage PCU), a SAT-certified body, and Spatial source that `rdu_loop_lane_lock` runs through the Plasticine toolchain in a container when Docker and the toolchain are present |
| `fib.lg` | `whole-program` | Fibonacci by fast doubling, unrolled over the six bits of n: a branch-free dataflow program, SAT-certified per PE |
| `primes.lg` | `whole-program` | primality below 121 by unrolled trial division: a `Seq` held in heap cells, an inlined helper, and a short-circuit `and` turned into a select. The remainder unit and the heap have no bit-blast lowering, so the per-PE certificate is `Uncertified` by name and equivalence is checked by evaluation against the VM |
| `gemm.lg` | `tensor` | `C = A matmul B` over the tensor sugar |
| `rmsnorm.lg` | `tensor` | `rmsnorm of X with weight W` over the fused-row sugar |
| `qwen38_tiny.lg` | `tensor` | one forward pass of the Qwen3.8-27B language model's architecture at a tiny size (the same operations and layer mix, tiny dimensions) with deterministic weights; generated by `gen_qwen38_filled_lg` and held byte-equal to it by `rdu_whole_program_lane_lock` (regenerate with `LOGOS_UPDATE_FIXTURES=1`) |

Every program here lowers: `emit_rdu_lanes` runs `largo emit rdu` over each one and checks the lane
it names.

## What the dataflow lanes refuse, by name

- **Recursion** (`Return fib(n - 1) + fib(n - 2).`): a call stack is control the dataflow stages do
  not have. The spatial fabric's whole-program call-stack engine
  (`codegen_sva::wprog_engine_rtl::program_calls_fabric`) runs such a program with the VM's results,
  but the RDU lowering refuses it.
- **Any loop but the first function's counted range**: a `While`, a nested loop, a loop over a
  sequence. The loop lane takes exactly one counted loop.
- **An early return**, and **a `Show` of `Text`** on the output tape.
- **Operations the fabric does not realize**, such as host I/O (`Tcp`, `Serve`), each with the reason
  `fabric_tile::frontier_reason` gives.

The tests: `rdu_loop_lane_lock`, `rdu_whole_program_lane_lock` and `rdu_op_coverage_lock` (which
of the fabric's operations the SN50 model hosts) in `crates/logicaffeine_tests/tests/`, and
`emit_rdu_lanes` for the CLI.
