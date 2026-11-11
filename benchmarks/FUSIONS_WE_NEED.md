# Fusions we need: one IR, shared fusions, chip shims

This book is the work order for beating vLLM and Triton on every kernel, through fusions the compiler finds and
proves on its own.
- **Section 2, the SOTA contract, says what finishing this book proves.** It names every rival, every cell (a model
  on a chip at a bit width in a regime), the rules of a fair win, and why ticking every box implies winning every cell.
- **Section 3, one program on every backend, is the product's claim made concrete.** It traces the one LOGOS
  program `model::author` writes through every backend the compiler has, says what each backend runs today and what it
  will run, and maps every fusion onto every chip.
- **Section 4, the kernel map, is the performance core.** It walks a decode step and a prefill chunk op chain by op chain. Each
  chain gets:
  - what we launch today and what it measured;
  - the bytes and FLOPs it must move at our models' real shapes, with the floor that implies on the 3060;
  - exactly how vLLM and Triton build it;
  - the fused kernel that beats them, with a copy bar and a beyond bar.
- **The catalog (section 10):** every kernel fusion and technique vLLM and Triton implement, mined from their source at
  pinned commits, beside every fusion the ledger already tracks from llama.cpp, SGLang, TensorRT-LLM and FlashInfer.
- **The coverage map (section 11):** every op, kernel, pattern, pass, backend and builtin vLLM and Triton ship at the
  pins, 1,294 rows, each mapped to the ticket that copies it, re-extracted by a lock so nothing is missed.
- **The structure that makes the fusions shared (sections 5 to 9):**
  - the program's meaning lives in one intermediate representation;
  - fusions are decided once on it, by one optimizer;
  - each chip says what it allows and supplies its own primitives.
- **The board (section 12):** every piece of work as a ticket with RED tests, edge cases, success criteria and verify
  commands, written so an agent in a fresh session can take one and finish it.

Every item in sections 2, 3, 4, 6, 7, 8 and 10 to 13 is a checkbox. A box is ticked only with its receipt beside it.

The machine-checked twin of the catalog is `benchmarks/fusion_parity.json`, held by
`fusion_parity_lock`. Task LEDGER-2 makes the lock hold this book and the ledger to each other row for row.

Read at:

| Stack | Commit | Where the pinned tree lives |
|---|---|---|
| vLLM | ceb01b6ddf (v0.32.1rc0+485, 2026-10-25) | external/competitors/vllm (task SETUP-2 places it) |
| Triton | 1ed999d6b2 (main, 2026-10-25) | external/competitors/triton (task SETUP-2 places it) |
| llama.cpp | b11635 | external/competitors/llama.cpp |
| SGLang | v0.5.21 | external/competitors/sglang |
| TensorRT-LLM | v1.4.0rc0 | external/competitors/tensorrt-llm.paths.txt |
| FlashInfer | v0.7.1 | external/competitors/flashinfer |

Counts quoted below are the counts at these commits, as section 11 extracts them: vLLM registers 263 op names and
holds 821 `@triton.jit` kernels in 294 files; Triton's two backends run 134 distinct compiler passes, and its language
has 116 public builtins. LEDGER-3 recomputes every one.

---

## 1. How to use this book

### 1.1 Taking a task

1. Pick a task whose **Depends on** list is done. Section 12.2 gives the order.
2. Post a claim line on the shared lanes board (LANES.md in the private workspace). Name the task id and the files it
   touches.
3. Read everything under **Read first**.
4. Write the **RED tests** exactly as named. Run them, and confirm each fails for the stated reason. A RED test that
   passes on today's tree is not a RED test: stop and find out why.
   - A ticket may arrive with its RED tests already written: a binary in `crates/logicaffeine_tests/tests/` that opens
     with `#![cfg(feature = "handed-reds")]`, so every gate compiles it to nothing. Run it with
     `cargo nextest run -p logicaffeine-tests --features handed-reds --test <suite>`, make it pass by changing the
     implementation, and delete that one line as you land. Its tests are frozen (`frozen_reds_lock`): never edit them.
5. Implement until the RED tests pass.
6. Run **Verify**. Read every line of the output; never pipe a test run through `head` or `tail`.
7. For each **Success** box, post a RECEIPT line on the lanes board with its receipt: the test name, the log path under
   `logs/`, or the ledger row and number. Do the same for the ticket's rows in section 11. The book keeper ticks them
   (sections 1.5 and 1.6); no other agent edits this book.
8. Kill every listed mutant: copy the tree, apply the mutation, and watch a new test fail.
9. Land the work under section 1.3, then mark the task done on the lanes board.

A task is done only when every success box carries a receipt.

### 1.2 Standing rules

- Never edit a failing test to make it pass. A test is the specification. If a RED test looks wrong, stop and ask
  Tristen.
- Never delete a test. When a mechanism changes, port the test to the new mechanism.
- Every new lock is mutation-tested. Run the mutation in a private copy of the tree, never in the shared tree.
- Tests live in `crates/logicaffeine_tests`:
  - integration suites in `tests/`;
  - unit-test bodies in `unit/<crate>/<module>.rs`, reached through a one-line `#[path]` stub.
- When running part of `logicaffeine-tests`, name the suite with `--test <file>`. An `-E` filter selects tests, but cargo
  still builds every test binary it is pointed at.
- Changing what an existing lock asserts needs Tristen's approval first. That covers a census expectation, a pinned
  golden and a ledger schema.
- A model's artifacts all come from its one LOGOS program (`model::author`), and `one_program_lock` holds this. No task
  may add a second writer.
- Some files belong to another session (section 9.2). They change only through a handoff on the lanes board, and are
  never edited by two sessions at once.
- Generated Rust states every binder's type.

### 1.3 Landing

Before every batch of cargo, nvcc or Quartus work:
- re-read the lanes board for an open gate window (a WINDOW START with no FINISH) and hold while one is open;
- never build a test target while a peer's suite is running from it.

To land:
1. Run the task's targeted suites (each with `--test`).
2. When a public enum grew, run `cargo test --no-run --no-fail-fast` over the workspace, so every test target still builds.
3. Run `./scripts/run-all-tests-fast.sh` once, inside a gate window. Its summary goes in the task's receipts.
4. Update the CHANGELOG `[Unreleased]` entries of the crates you touched.

Commits and pushes happen on Tristen's word.

### 1.4 This box

| Item | Value |
|---|---|
| GPU | RTX 3060 12 GB, sm_86 |
| GPU link | PCIe 4.0 x4; mapped host reads measure about 5.7 GB/s |
| nvcc | 12.0 (cannot target sm_100) |
| Driver | 580.178.04 |
| CPU | i9-14900K |
| Memory | DDR4-2133, dual channel |

The checkpoints the GPU locks read are in `models/`:
- llama3.2-1b and its IQ4_XS form;
- gemma3-1b, gemma2-2b;
- qwen3.5-0.8b, qwen3-8b, qwen3.8-27b;
- deepseek-v2-lite, gpt-oss-20b.

Each has an f32 twin, and the goldens are in `benchmarks/models/`.

GPU time is shared:
- the session using the 3060 posts GPU START and GPU FINISH on the lanes board;
- no GPU test runs between them.

Datacenter work has its own status (`compile_verified`, LEDGER-1):
- it is compiled and checked here;
- its values tests run when a rented sm_90 or sm_100 GPU is available.

### 1.5 Streams: running many agents at once

The board is cut into streams whose files do not overlap, so agents in different streams never edit one file. Every
line of the board index (section 12.1) carries its stream as a label, `stream:S0` to `stream:S15`.

**Rules for every stream.**
- An agent takes the next open ticket of its stream whose dependencies are done, and claims it on the lanes board.
- An agent never edits a file another stream owns. A change it needs there is a handoff on the lanes board, made by
  that stream's agent.
- Before every batch of cargo, nvcc or Quartus work, read the lanes board for an open gate window. Hold while one is
  open.
- GPU work runs between GPU START and GPU FINISH lines. One stream holds the 3060 at a time; streams queue for it on
  the lanes board.
- Each landing follows section 1.3. A stream that grows a public enum runs the workspace `--no-run` build first.
- **No agent edits this book but the book keeper (S0).** Every other agent reports on the lanes board in the line
  formats of section 1.6, and S0 applies them. One file, one writer.

**One file per fused class: the lever for parallel fusion work.**
- IR-3 registers every ledger class up front, so a fusion ticket never edits the registry.
- MIGRATE-1 lays the CUDA link out as one file per class. Each spatial link and the CPU link follow the same layout
  when they are written.
- A fusion ticket therefore owns exactly:
  - its rule file, `tensor_graph/fuse/<class>.rs`;
  - its link file on each chip;
  - its kernel template;
  - its tests.

  Two fusion tickets never share a file. Every ready S14 ticket can run at once, limited only by the GPU queue.

| Stream | Owns (files and modules) | First tickets | Agents at once |
|---|---|---|---|
| S0 The book keeper | `benchmarks/FUSIONS_WE_NEED.md`; `frozen_reds_lock` and `crates/logicaffeine_tests/tests/fixtures/frozen_reds.tsv` | applies receipts from the lanes board; runs the book's checks (section 13); freezes each ticket's RED tests when it is posted | 1, always |
| S1 The one IR and its optimizer | `crates/logicaffeine_compile/src/tensor_graph/` except `fuse/<class>.rs`; the front end's move into it; their locks | IR-0, IR-1, IR-2, IR-3, OPT-1, OPT-2 | 1: the critical path |
| S2 The one program everywhere | `crates/logicaffeine_compile/src/model/program.rs`, `crates/logicaffeine_compile/src/bringup.rs`, `apps/logicaffeine_cli/src/commands/emit.rs`, `apps/logicaffeine_cli/src/commands/bringup.rs`, `docs/cli.md`, `one_program_lock`, `bringup_lock`, `cli_emit_model_lock` | ONE-PROGRAM-VM-REFERENCE, ONE-PROGRAM-EVERY-BACKEND | 1 |
| S3 CUDA kernel peaks and techniques | the kernel templates of the lane's GEMM, attention, GDN chunk and GEMV kernels (held by the performance lane today) | PARITY-INT8-GEMM-PEAK, PARITY-PREFILL-ATTENTION-PEAK, PARITY-GDN-CHUNK-TENSOR-CORES | 2, on disjoint kernels |
| S4 CUDA migration and runtime | the lane's `plan.rs`, `emit.rs`, `schedule.rs`, the link's shared core, `qwen_emit.rs`, graph capture | MIGRATE-1, MIGRATE-2 | 1 |
| S5 Baselines and ledgers | `scripts/accel/`, `benchmarks/results/models/`, `benchmarks/fusion_parity.json`, `fusion_parity_lock`, the new baseline locks | SETUP-2, SETUP-3 | 2 |
| S6 Spatial chips | `crates/logicaffeine_compile/src/spatial/` except `decode_engine/` and per-class link files | MIGRATE-3 | 1 per backend directory: 4 |
| S7 The D5005 engine | `crates/logicaffeine_compile/src/spatial/decode_engine/` (peer-owned: handoffs only) | MIGRATE-4 | 1 |
| S8 Proofs and exact methods | new modules for planning, scheduling, allocation, numerics and protocol proofs; callers in `logicaffeine_proof`, `logicaffeine_tv`, `logicaffeine_verify` | BEYOND-BIT-EXACT-EQUIVALENCE, BEYOND-CERTIFIED-ALLOCATION, BEYOND-EXACT-MODULO-SCHEDULER | 3: each ticket owns its own module |
| S9 Below `ptxas` | `crates/logicaffeine_compile/src/codegen_sass.rs`; a GPU stencil library; `crates/logicaffeine_forge/src/patch.rs` callers | BEYOND-OWN-SASS-SCHEDULER | 1 |
| S10 The Triton-class tile tier | the generated-kernel tier under the CUDA link (new); the layout algebra (new) | TRITON-LINEAR-LAYOUT, TRITON-TILE-TIER | 2 |
| S11 New chips | new shim and link modules per chip | PARITY-CPU-SHIM, TRITON-AMD-SHIM, PARITY-XPU-SHIM | 1 per chip: 3 |
| S12 Language and models | `crates/logicaffeine_language/src/parser/`, `crates/logicaffeine_compile/src/model/author.rs`, `model/spec.rs` | PARITY-SOFTCAP | 1: `author.rs` has one writer |
| S13 Serving | the serving runtime (new) and its locks | SERVE-PAGED-KV, SERVE-CONTINUOUS-BATCHING | 2 after SERVE-CONTINUOUS-BATCHING |
| S14 Fusions | each ticket's own class files (rule, links, template, tests) | every PARITY fusion and primitive not in S3, S11 or S12, in section 12.2's order | 1 per ready ticket |
| S15 The demos and their receipts | the Nexys board tops (`crates/logicaffeine_compile/src/codegen_sva/fabric_soc.rs`'s board emitters, `benchmarks/silicon/nexys_*_soc.*`, `benchmarks/silicon/ours_*.v`), their hosts (`scripts/fabric-host.py`, `scripts/validate-on-nexys-fabric.sh`), `fabric_perf_lock`, `measured_fields_lock`, the demo generators and `benchmarks/results/demo/`, the proof receipts' re-runs | FABRIC-REAL-CYCLE, DEMO-FUSION-MAP, DEMO-LAYER-TOUR, PROOF-RECEIPTS-FRESH | 3, one board each |

- [x] Every ticket on the board carries exactly one `stream:` label (receipt: `sota_contract_lock::every_ticket_is_in_one_stream`, `logs/demo-strategy/sota-contract-book2.log`).
- [ ] No file is owned by two streams (receipt: `sota_contract_lock::no_file_belongs_to_two_streams`, over the Owns column and each S14 ticket's Touches row).

Streams S1 to S13 take up to 24 agents at once. S14 holds 53 fusion tickets and adds one agent per ready ticket; most
become ready once OPT-1 and MIGRATE-1 land. The critical path is S1, then S4. Work that needs no IR starts in S3, S5, S8, S9, S10 and S12 on
the first day.

### 1.6 The launch kit

Everything needed to start agents: the prompt each one is given, the lines each one posts, and the first wave.

**The prompt.** Fill in the three names and give it to a fresh session in the repository:

```
You are agent <NAME>, in stream <STREAM> of benchmarks/FUSIONS_WE_NEED.md. Your ticket is <TICKET>.

1. Read section 1 of the book (taking a task, the standing rules, landing, streams, this kit). Then read your ticket's
   body (search for "#### <TICKET> ·", or its line in section 12.14 or 12.16), and everything under its Read first.
2. Before any cargo, nvcc or Quartus work, read the lanes board for an open gate window, and hold while one is open.
   Post: CLAIM <TICKET> <NAME> <STREAM> files: <the ticket's Touches row>
3. Edit only the files your stream owns (section 1.5) and your ticket's Touches row. Never edit the book: report on the
   lanes board in the line formats of section 1.6.
4. Write the RED tests exactly as named. Run each with --test and confirm it fails for the reason the ticket gives.
   Post one RED line per test.
5. Implement until they pass. Run the ticket's Verify, and read every line of the output.
6. Post one RECEIPT line per Success box, per edge case covered, and per section 11 coverage row your ticket owns.
7. Kill every listed mutant in a private copy of the tree. Post the MUTANTS line.
8. Land under section 1.3, inside a gate window. Post the DONE line.

Stop and ask Tristen before you change what an existing lock asserts, edit a failing test, rent or buy hardware, or
touch a file another stream owns.
```

**The lines.** One per event, on the lanes board, in these forms, so the book keeper can apply them by script:

```
CLAIM <TICKET> <agent> <stream> files: <paths>
RED <TICKET> <suite>::<test> fails: <reason> log: <path>
RECEIPT <TICKET> "<box text, as written in the book>" receipt: <test name | log path | ledger row>
MUTANTS <TICKET> killed <k>/<n> log: <path>
HANDOFF <TICKET> to <stream>: <file> <what is needed>
DONE <TICKET> landed: <commit or landing ledger> suite: <fast-suite summary line>
BLOCKED <TICKET> on <ticket or question>
```

- [ ] The book keeper applies each RECEIPT line by finding its box text under its ticket (or in sections 2, 3, 4 and 11
  for boxes owned by the ticket) and ticking it with the receipt beside it. It refuses a line whose box text does not
  match exactly once.
- [ ] After each batch it runs the citation checker (13.3), the extractor (13.4) and the three doc locks of section 13.1,
  then commits the book alone.

**The first wave.** These tickets have no open dependency today, one agent each:

| Agent | Stream | Ticket | What it unblocks |
|---|---|---|---|
| keeper | S0 | (the book) | every receipt |
| ir-0 | S1 | IR-0 | the whole critical path: IR-1, IR-2, IR-3, OPT-1, OPT-2, MIGRATE-1 |
| vm-ref | S2 | ONE-PROGRAM-VM-REFERENCE | ONE-PROGRAM-EVERY-BACKEND, and every backend held to the VM |
| pins | S5 | SETUP-2 | SETUP-4, SETUP-6, LEDGER-1, LEDGER-3 |
| vllm | S5 | SETUP-3 | SETUP-4, SETUP-5, SETUP-7, SETUP-9 |
| exp2 | S3 | TECH-EXP2-SCALE-FOLD | the attention kernels' bars (K-04, K-26) |
| fp-convert | S3 | TECH-PACKED-FP-CONVERSIONS | PARITY-QUANT-SCHEMES, PARITY-TRITON-MX-CONVERT |
| mesh | S6 | SPATIAL-NONUNIFORM-TILING | K-40 |
| bit-exact | S8 | BEYOND-BIT-EXACT-EQUIVALENCE | BEYOND-SUPEROPTIMIZER, every integer kernel's proof |
| sass | S9 | BEYOND-OWN-SASS-SCHEDULER | K-23, BEYOND-RUNTIME-PATCHING |
| layout | S10 | TRITON-LINEAR-LAYOUT | TRITON-GENERIC-SWIZZLE, TRITON-SHUFFLE-CONVERT, TRITON-GLOBAL-LAYOUT-ASSIGNMENT, PARITY-INT8-GEMM-PEAK |
| pdl | S10 | TECH-PDL | GRAPH-CROSS-KERNEL-OVERLAP |

Then, as each DONE line lands, the next ready ticket in each stream starts (section 12.2's order). The second wave opens
when IR-0, SETUP-2 and SETUP-3 land. The wide one opens when OPT-1 and MIGRATE-1 land: the S14 fusion tickets, one
agent each.

- [ ] Before the first wave starts, the lanes board carries one line naming the wave, its agents and their streams.
- [ ] Each wave's start and end are on the lanes board, with the DONE lines that opened the next.

---

## 2. The SOTA contract: what finishing this book proves

This section states what "state of the art" means for this book, and why ticking every box in it gets us there. The
rest of the book is the route; this section is the destination and the test that we arrived.

### 2.1 The claim, and why completing the book implies it

When every box in this book is ticked:
1. **Every cell in 2.4 is won.** Each cell is one model on one chip at one bit width in one regime. For each, LOGOS
   runs faster than the fastest stack in 2.2, measured on one boot under the rules of 2.3.
2. **Every vLLM and Triton entry is copied.** Section 11 lists every kernel, fusion pattern, graph pass, attention
   backend, MoE kernel, linear kernel, Triton file, compiler pass and language builtin that vLLM and Triton ship at the
   pins: 1,294 rows. Each row is ticked only when the ticket that owns it has matched that entry at our shapes.
3. **No ticket is open.**

Five mechanisms make this a consequence of finishing the work, not a hope.

- [ ] **The last box of every cell is a measurement.**
  - A cell's box can be ticked only with a row in the cell ledger, sota_cells.json (new, beside `benchmarks/results/models/rtx3060_scoreboard.json`; LEDGER-3) that shows
    the win under 2.3.
  - `sota_contract_lock::a_ticked_cell_has_a_won_ledger_row` refuses any other tick.
  - So the book cannot reach 100% with a lost cell in it.
- [ ] **The coverage map is total, and it is re-extracted.**
  - Section 11 was extracted from the pinned trees by the rules in 13.4, and `sota_contract_lock` re-runs those rules.
  - An entry at the pins with no row turns it red, and so does a row whose entry no longer exists.
  - Moving the pins to newer vLLM or Triton commits pulls in every new kernel automatically, as a red test that names it.
- [ ] **Every Copy bar is the fastest kernel anyone ships at our shape (2.5).**
  - The bar is never one stack's number. It is the maximum over every stack in 2.2 that implements the op on that chip:
    vLLM's dispatched kernel, Triton's kernel, llama.cpp's, ExLlamaV3's, FlashInfer's, FlashAttention's, and cuBLASLt
    for dense GEMMs.
  - `kernel_ab_lock` refuses a Copy tick below the row maximum.
- [ ] **The launch census is matched as well as each kernel.**
  - For every decode cell, SETUP-7 records the best rival's per-token kernel list from nsys, beside ours (K-01).
  - Every kernel the rival runs that we do not fuse away has a SETUP-4 row.
  - Kernel-by-kernel dominance plus no more launches is the sufficient condition. The cell's measurement is the
    deciding one.
- [ ] **A cell that stays lost grows the book (2.6).**
  - If every ticket a cell names is done and the cell is still lost, the cell's box stays open.
  - The agent records an nsys diff of both sides. Each kernel, gap or host cost where the rival is ahead becomes a new
    ticket, citing the rival's code.
  - "Complete" therefore cannot coexist with a lost cell.

### 2.2 The rivals

A cell is won against the fastest of these, each in its best documented configuration for that cell:
- CUDA graphs on;
- its fastest attention and GEMM backends on the chip;
- its recommended quantized checkpoint at the cell's bits.

Its flags are recorded in the ledger row. An agent may tune a rival's flags, and the best rival number found is the bar.

| Stack | Pin or version | Where it leads | On the 3060 | Baseline ticket |
|---|---|---|---|---|
| vLLM | ceb01b6ddf | serving throughput, paged attention, every quantized kernel family | yes | SETUP-3, SETUP-5 |
| SGLang | v0.5.21 | serving: prefix cache, overlap scheduler, structured output | yes | SETUP-7 |
| TensorRT-LLM | v1.4.0rc0 | NVIDIA's own engine: XQA attention, weight-only GEMV | yes (sm_86 is supported) | SETUP-7 |
| llama.cpp | b11635 | batch-1 decode on consumer GPUs and CPUs, GGUF | yes | scoreboard (today) |
| ExLlamaV3 | 1.6.4 (installed under `/mnt/build/sota`) | batch-1 decode at fractional bits | yes | SETUP-7 |
| MLC-LLM | pinned by SETUP-7 | compiled decode on consumer GPUs | yes | SETUP-7 |
| Triton | 1ed999d6b2 | kernel level: the tutorials and `triton_kernels` | yes | SETUP-6 |
| FlashInfer | v0.7.1 | kernel level: attention, sampling | yes | SETUP-4 |
| FlashAttention-2 | the version vLLM pins | kernel level: attention | yes | SETUP-4 |
| cuBLAS / cuBLASLt | the CUDA toolkit on the box | kernel level: dense GEMM and GEMV | yes | SETUP-4 |
| vLLM CPU backend, llama.cpp CPU | as above | CPU decode and prefill | the i9-14900K | SETUP-7 |
| vLLM on ROCm, llama.cpp HIP | as above | AMD GPUs | no: rented (SETUP-8) | SETUP-8 |
| vLLM on XPU | as above | Intel GPUs | no: bought or rented (SETUP-8) | SETUP-8 |

- [ ] Every row above has a recorded version and install receipt (SETUP-3, SETUP-6, SETUP-7, SETUP-8).
- [ ] A stack not in this table that publishes a faster number for a cell joins it by a new SETUP ticket, and the cell
  is measured again. The claim is always against this table as of the ledger row's date; the row names every version.

### 2.3 The rules of a fair win

- [ ] **One boot, a quiet box.**
  - Both sides run on the same boot, with no other GPU session (GPU START and GPU FINISH on the lanes board).
  - The ledger row records the boot id and the load average.
- [ ] **Matched bits.**
  - The rival never reads more weight bytes per token than we do: its bits per weight, scales included, are at most
    ours.
  - Q8_0 is 8.5 bits a weight. The 8-bit rivals are int8 W8A8, FP8 (weight-only on sm_86), EXL3 at 8.0 bits, and
    llama.cpp Q8_0.
  - Q4_K_M is about 4.85 bits a weight. The 4-bit rivals are AWQ or GPTQ through Marlin, EXL3 at 4.0 bits, and
    llama.cpp Q4_K_M.
  - BF16 is compared with BF16, and MXFP4 with MXFP4.
  - The same holds for the KV cache: the rival's cache is never wider than ours.
  - A rival in a narrower format is allowed, since that only makes our win harder.
- [ ] **Matched quality.**
  - Both sides' top-1 token agreement with the f32 reference is recorded over the golden prompts of `model_golden_lock`.
  - Ours must agree at least as well as the rival's, within 2 standard errors. A faster run that agrees less is not a
    win.
- [ ] **Matched work.**
  - The same prompt and generation lengths on both sides: 6-token prompts and 128 generated tokens for decode, exact
    chunk sizes for prefill, and the trace of SETUP-9 for serving.
  - Greedy decoding in decode and prefill cells. The trace's sampling parameters in serving cells.
- [ ] **Matched timing.**
  - Decode cells time generation only, after the prompt.
  - Prefill cells time each engine's own prefill compute:
    - vLLM's engine timestamps (`scheduled_ts` → `first_token_ts`), which exclude its per-request host cost;
    - llama.cpp's `pp` timing;
    - our chunk's device time.
  - Serving cells time from the client: TTFT, TPOT and goodput.
- [ ] **A real win.**
  - At least five runs a side.
  - Our mean is faster by more than two combined standard errors, and our slowest run beats the rival's fastest.

### 2.4 The cells

A cell id reads `C-<chip>-<model>-<regime>-<bits>`. Regimes:

| Regime | What is measured |
|---|---|
| D1 | decode at batch 1, a 6-token prompt, 128 tokens generated; tok/s |
| D8K | decode at batch 1 at position 8192, f16 KV cache on both sides; tok/s |
| P512 | one 512-token prefill chunk; tok/s |
| P4K | the time to first token of a 4,096-token prompt |
| B16 | aggregate decode tok/s at 16 concurrent sequences of 1,024-token context |
| S | goodput on the SETUP-9 serving trace: the highest request rate each stack sustains with p99 TPOT ≤ 100 ms and p99 TTFT ≤ 2 s |

Each regime is closed by a shared set of tickets, plus the model's own items listed with each cell:

| Regime | Closing set |
|---|---|
| D1 | K-08, K-22, SETUP-5, SETUP-7 |
| D8K | the D1 set, K-04, K-30 (PARITY-ATTN-GQA-ROW-PACKING, PARITY-ATTN-QUANT-IN-FOLD, PARITY-KV-CACHE-QUANT f16 scheme), PARITY-ATTN-LSE-MERGE |
| P512 | K-23, K-26, K-28, K-29, SETUP-5, SETUP-7 |
| P4K | the P512 set, PARITY-PREFILL-ATTENTION-PEAK, SERVE-CHUNKED-PREFILL |
| B16 | SERVE-PAGED-KV, SERVE-CONTINUOUS-BATCHING, SERVE-PER-BATCH-GRAPHS, PARITY-SMALL-M-GEMM, PARITY-BATCH-ROW-QUANT, PARITY-ATTN-GQA-ROW-PACKING, PARITY-SAMPLING, SETUP-7 |
| S | the B16 set, SERVE-CHUNKED-PREFILL, PARITY-MIXED-BATCH-ATTENTION, SERVE-PREFIX-CACHE, SERVE-SPEC-DECODE, SETUP-9 |

The best rival today is quoted where it has been measured: llama.cpp from `benchmarks/results/models/rtx3060_scoreboard.json`
(2026-10-25), and vLLM 0.32 from `logs/demo1102-sota/vllm/` (2026-10-25, a loaded box: re-measured under 2.3 by
SETUP-5). Every other side is "to measure" until SETUP-5 and SETUP-7 record it.

#### RTX 3060 (sm_86, 12 GB)

**Llama-3.2-1B.** Its own items: K-02, K-03, K-05, K-07.
- [ ] C-3060-LLAMA1B-D1-Q8: ours 233.3 tok/s; llama.cpp 213.8; the rest to measure.
- [ ] C-3060-LLAMA1B-D1-Q4: ours 337.2 tok/s in Q4_K_M; llama.cpp 304.7, fastest run 306.1 (2026-11-08,
  logs/fusion-k09/window74/); IQ4_XS, Marlin AWQ and EXL3 4.0 to measure.
- [ ] C-3060-LLAMA1B-D1-BF16: ours 131.4 tok/s, its BF16 matrices read in place (PARITY-WEIGHT-FORMATS); vLLM 125.5,
  llama.cpp 125.5 (2026-11-06, a loaded box: logs/fusion-k09/window/, logs/fusion-k09/window2/).
- [ ] C-3060-LLAMA1B-D8K-Q8: ours 191.7 tok/s, slowest run 191.6 (2026-11-09, the decode attention warp-local in runs of
  hd / 8 lanes a key, the f16 cache on both sides: logs/fusion-k09/window80/, window80b/); llama.cpp 161.7, fastest
  162.5 (window73, load 22–37), 151.8 at window80b's load of 70–83.
- [ ] C-3060-LLAMA1B-D8K-Q4: ours 259.6 tok/s, slowest run 259.5; llama.cpp 211.3, fastest 217.9 (window73), 198.3 at
  window80b's load.
- [ ] C-3060-LLAMA1B-D8K-BF16: ours 116.2 tok/s, slowest run 116.2 (107.5 before the warp-local chunks); llama.cpp
  111.1, fastest 111.6 (window73), 102.7 at window80b's load — 4.1% ahead.
- [ ] C-3060-LLAMA1B-P512-Q8: ours 15,276 tok/s; llama.cpp 12,216.
- [ ] C-3060-LLAMA1B-P512-Q4: ours 13,085 tok/s (beside a test build: logs/fusion-k09/window71/); llama.cpp 9,188,
  fastest 9,505 (logs/fusion-k09/window70/).
- [ ] C-3060-LLAMA1B-P512-BF16: ours 8,372 tok/s on the 16-bit tensor-core GEMM (`lk_gemm_h16`, one bf16 word);
  llama.cpp 7,773, fastest 7,912, in the same window (2026-11-08, logs/fusion-k09/window72/).
- [ ] C-3060-LLAMA1B-P4K-Q8: ours 316.3 ms (12,950 tok/s); llama.cpp 384.4 ms (10,657) (2026-11-08,
  logs/fusion-k09/window73/).
- [ ] C-3060-LLAMA1B-P4K-Q4: ours 377.0 ms (10,865 tok/s); llama.cpp 415.7 ms (9,854).
- [ ] C-3060-LLAMA1B-P4K-BF16: ours 526.4 ms (7,781 tok/s); llama.cpp 536.4 ms (7,636) — 1.6% ahead.
- [ ] C-3060-LLAMA1B-B16-Q8: ours 2,070.7 tok/s, slowest run 2,066.2 (2026-11-10, a batch lane of 16 sequences:
  logs/fusion-k09/window87/); llama.cpp (llama-batched-bench) 1,369.6, fastest 1,384.1, in the same window; vLLM
  (FP8 weights through Marlin) 2,017.9, fastest 2,081.5 (logs/fusion-k09/window89b/). In one window with vLLM
  (logs/fusion-k09/winb16k3/): ours 2,063.5, slowest 2,063.2; vLLM 2,153.2, fastest 2,170.9 — 4.9% behind its fastest
  (2,065.4, slowest 2,064.2 in logs/fusion-k09/winb16k2/; llama.cpp 1,266.2, fastest 1,313.1 there). The step's
  attention takes 2.9 ms of its 7.7 (nsys: 181 µs a layer for 37.7 MB of cache, 208 GB/s); with its chunks on blocks of
  their own it ran 0.8% slower (copy nx3b, winb16k2).
- [ ] C-3060-LLAMA1B-B16-Q4: ours 1,907.1 tok/s, slowest run 1,903.2, its Q4_K and Q6_K products on the int8 tensor
  cores in the decode step's order (`lk_gemv_mma_blk`; logs/fusion-k09/winb16k/); llama.cpp 1,492.7, fastest 1,556.0,
  in the same window — 1.22× its fastest.
- [ ] C-3060-LLAMA1B-B16-BF16: ours 1,461.9 tok/s, slowest run 1,460.6; llama.cpp 1,126.6, fastest 1,137.9 (window87);
  vLLM 1,248.2, fastest 1,403.3 (window89b).
- [ ] C-3060-LLAMA1B-S-Q8, C-3060-LLAMA1B-S-Q4, C-3060-LLAMA1B-S-BF16: not runnable on our side until the S set lands.

**Gemma-3-1B.** Its own items: K-02, K-03, K-04, K-05, K-07, PARITY-NORM-THEN-ADD.
- [ ] C-3060-GEMMA3-1B-D1-Q8: ours 237.1 tok/s; llama.cpp 179.7.
- [ ] C-3060-GEMMA3-1B-D1-Q4: ours 256.3 tok/s; llama.cpp 229.1, fastest 229.7 (2026-11-08, logs/fusion-k09/window74/).
- [ ] C-3060-GEMMA3-1B-D1-BF16: ours 150.2 tok/s; vLLM 142.5, llama.cpp 133.5 (2026-11-06, a loaded box).
- [ ] C-3060-GEMMA3-1B-D8K-Q8: ours 216.8 tok/s, slowest run 216.8 (2026-11-09, logs/fusion-k09/window80/); llama.cpp
  165.4, fastest 173.8 (window73), 155.3 in window80 (load 70–83).
- [ ] C-3060-GEMMA3-1B-D8K-Q4: ours 234.0 tok/s, slowest run 234.0; llama.cpp 214.3, fastest 215.9 (window73).
- [ ] C-3060-GEMMA3-1B-D8K-BF16: ours 143.0 tok/s, slowest run 143.0; llama.cpp 129.0, fastest 129.5 (window73). Its
  global layers' 56 splits fold on one multiprocessor (K-26b).
- [ ] C-3060-GEMMA3-1B-P512-Q8: ours 16,254 tok/s (2026-11-08, a quiet box: logs/fusion-k09/window77/); llama.cpp
  11,050.
- [ ] C-3060-GEMMA3-1B-P512-Q4: ours 13,600 tok/s (logs/fusion-k09/window71/); llama.cpp 9,495, fastest 10,174.
- [ ] C-3060-GEMMA3-1B-P512-BF16: ours 9,523 tok/s, slowest run 9,474; llama.cpp 9,116, fastest 9,439, in the same
  window (2026-11-08, logs/fusion-k09/window72/) — 0.4% past its fastest.
- [ ] C-3060-GEMMA3-1B-P4K-Q8: ours 270.1 ms (15,163 tok/s); llama.cpp 352.7 ms (11,614) (2026-11-08,
  logs/fusion-k09/window73/).
- [ ] C-3060-GEMMA3-1B-P4K-Q4: ours 317.9 ms (12,883 tok/s); llama.cpp 353.1 ms (11,600).
- [ ] C-3060-GEMMA3-1B-P4K-BF16: ours 390.4 ms (10,492 tok/s), slowest run 394.5, over the binary16 cache llama.cpp
  keeps (392.6 over the `f32` cache); llama.cpp 445.2 ms (9,200), fastest 443.1, in the same window (2026-11-09, load
  68–82: logs/fusion-k09/window83/), 409.4 on 2026-11-08's quieter box. K-23's block GEMM and K-29's chunk before the
  prompt's last without its output.
- [ ] C-3060-GEMMA3-1B-B16-Q8: ours 2,478.0 tok/s, slowest run 2,475.5; llama.cpp 1,114.9, fastest 1,139.7 (window87);
  vLLM (FP8) 1,861.6, fastest 1,923.1 (window89b).
- [ ] C-3060-GEMMA3-1B-B16-Q4: ours 2,171.4 tok/s, slowest run 2,163.0, its Q5_0 query, key, value, gate and up
  matrices (1,152 wide) on the tensor cores beside its Q4_K and Q6_K ones (logs/fusion-k09/winb16k2/); llama.cpp
  1,203.9, fastest 1,253.1, in the same window — 1.73× its fastest.
- [ ] C-3060-GEMMA3-1B-B16-BF16: ours 1,853.9 tok/s, slowest run 1,852.6; llama.cpp 1,064.3, fastest 1,077.9
  (window87); vLLM 1,369.1, fastest 1,459.9 (window89b).
- [ ] C-3060-GEMMA3-1B-S-Q8, C-3060-GEMMA3-1B-S-Q4, C-3060-GEMMA3-1B-S-BF16: waits on the S set.

**Gemma-2-2B.** Its own items: K-21 (PARITY-SOFTCAP), PARITY-ATTENTION-VARIANTS-INLINE, then the Gemma items.
- [ ] C-3060-GEMMA2-2B-D1-Q8: ours 109.0 tok/s, slowest run 108.98 (2026-11-09, its soft caps compiled:
  logs/fusion-k09/window82/, load 66–80); llama.cpp 91.9, fastest 93.0, in the same window.
- [ ] C-3060-GEMMA2-2B-D8K-Q8: ours 90.1 tok/s, slowest run 90.10; llama.cpp 75.2, fastest 75.6 (window82).
- [ ] C-3060-GEMMA2-2B-P512-Q8: ours 71.0 ms (7,211 tok/s), slowest run 71.2; llama.cpp 96.7 ms (5,317), fastest
  92.4 (window82).
- [ ] C-3060-GEMMA2-2B-P4K-Q8: ours 640.2 ms (6,398 tok/s), slowest run 642.6, over the binary16 cache (694.7 over the
  `f32` cache); llama.cpp 766.4 ms (5,344), fastest 762.7, in the same window (2026-11-09: logs/fusion-k09/window83/),
  744.6 in window82.
- [ ] C-3060-GEMMA2-2B-D1-Q4: ours 152.9 tok/s, slowest run 152.82 (2026-11-10, bartowski's Q4_K_M:
  logs/fusion-k09/window86/, load 54–88); llama.cpp 125.1, fastest 125.5, in the same window.
- [ ] C-3060-GEMMA2-2B-D8K-Q4: ours 117.8 tok/s, slowest run 117.81; llama.cpp 98.5, fastest 100.1 (window86).
- [ ] C-3060-GEMMA2-2B-P512-Q4: ours 80.1 ms (6,393 tok/s), slowest run 80.3; llama.cpp 101.1 ms (5,063), fastest
  94.1 (window86).
- [ ] C-3060-GEMMA2-2B-P4K-Q4: ours 731.5 ms (5,600 tok/s), slowest run 731.9, over the binary16 cache (797.2 over the
  `f32` cache); llama.cpp 780.1 ms (5,250), fastest 776.5, in the same window (window86).
- [ ] C-3060-GEMMA2-2B-B16-Q8: ours 854.8 tok/s, slowest run 854.1 (2026-11-10, logs/fusion-k09/winb16k/); llama.cpp
  614.7, fastest 621.6, in the same window.
- [ ] C-3060-GEMMA2-2B-B16-Q4: ours 780.0 tok/s, slowest run 779.8 (winb16k); llama.cpp 736.0, fastest 743.6 — 4.9%
  past its fastest.
- [ ] C-3060-GEMMA2-2B-S-Q8, C-3060-GEMMA2-2B-S-Q4: waits on the S set.

**Qwen3.5-0.8B.** Its own items: K-09 to K-13, K-27, PARITY-GDN-HALF-STATE (the rivals keep bf16 state).
- [ ] C-3060-QWEN35-08B-D1-Q8: ours 309.6 tok/s; vLLM FP8 228.2; llama.cpp 237.8.
- [ ] C-3060-QWEN35-08B-D1-BF16: ours 187.8 tok/s; vLLM 183.0, llama.cpp 151.3 (2026-11-06, a loaded box).
- [ ] C-3060-QWEN35-08B-D8K-Q8: ours 284.8 tok/s, slowest run 284.7 (2026-11-09, logs/fusion-k09/window80/); llama.cpp
  233.2, fastest 233.8 (window73), 198.2 in window80.
- [ ] C-3060-QWEN35-08B-D8K-BF16: ours 180.5 tok/s, slowest run 180.5; llama.cpp 144.4, fastest 144.8 (window73).
- [ ] C-3060-QWEN35-08B-P512-Q8: ours 14,559 tok/s (2026-11-08, a quiet box: logs/fusion-k09/window77/); llama.cpp
  11,781.
- [ ] C-3060-QWEN35-08B-P512-BF16: ours 10,815 tok/s; llama.cpp 9,258, fastest 9,747, in the same window (2026-11-08,
  logs/fusion-k09/window72/).
- [ ] C-3060-QWEN35-08B-P4K-Q8: ours 281.2 ms (14,566 tok/s), slowest run 291.4, over the binary16 cache (288.9 over the
  `f32` cache); llama.cpp 379.6 ms (10,790), fastest 376.1, in the same window (2026-11-09: logs/fusion-k09/window85/).
- [ ] C-3060-QWEN35-08B-P4K-BF16: ours 381.3 ms (10,742 tok/s), slowest run 386.0, over the binary16 cache (392.4 over
  the `f32` cache); llama.cpp 440.6 ms (9,296), fastest 438.8, in the same window (2026-11-09:
  logs/fusion-k09/window83/), 397.4 on 2026-11-08.
- [ ] C-3060-QWEN35-08B-B16-Q8: ours 2,292.4 tok/s, slowest run 2,291.9 (2026-11-10, logs/fusion-k09/winb16k2/);
  llama.cpp 913.0, fastest 937.6, in the same window; vLLM (FP8) 1,609.7, fastest 1,771.6 (logs/fusion-k09/window89b/)
  — 1.29× its fastest.
- [ ] C-3060-QWEN35-08B-B16-BF16: ours 1,797.3 tok/s, slowest run 1,794.6, its BF16 matrices read in place
  (2026-11-10, logs/fusion-k09/winb16k3/); vLLM 1,416.8, fastest 1,508.9, and llama.cpp 890.3, fastest 894.5, in the
  same window — won (1.19× vLLM's fastest; 1,502.6, fastest 1,602.6 in window89b).
- [ ] C-3060-QWEN35-08B-S-Q8, C-3060-QWEN35-08B-S-BF16: waits on the S set.

**Qwen3-8B.** BF16 does not fit 12 GB. Its own items: K-03, K-05, K-07, K-25.
- [ ] C-3060-QWEN3-8B-D1-Q8: ours 40.6 tok/s; llama.cpp 39.0; vLLM FP8 to measure (it needs `--max-num-batched-tokens
  1024` here).
- [ ] C-3060-QWEN3-8B-D1-Q4: ours 64.0 tok/s; llama.cpp 63.0, fastest 63.0 (2026-11-08, logs/fusion-k09/window74/).
- [ ] C-3060-QWEN3-8B-D8K-Q8: ours 35.0 tok/s, slowest run 35.0 (2026-11-09, logs/fusion-k09/window80/, window80b; 33.5
  before the warp-local chunks: the attention 164 µs a layer → 114); llama.cpp 34.6, fastest 34.7 (window73), 33.9 in
  window80b — 1.2% ahead.
- [ ] C-3060-QWEN3-8B-D8K-Q4: ours 51.5 tok/s, slowest run 51.5 (48.2 before); llama.cpp 51.2, fastest 51.3 (window73),
  47.2 in window80 — 0.6% ahead; next: the Q4_K rotation GEMV (190 GB/s) and the fold across the chip.
- [ ] C-3060-QWEN3-8B-P512-Q8: ours 2,413 tok/s; llama.cpp 2,178.
- [ ] C-3060-QWEN3-8B-P512-Q4: ours 2,005 tok/s, slowest run 1,990 (logs/fusion-k09/window71/); llama.cpp 1,968.7,
  fastest 2,000.4 — a tie.
- [ ] C-3060-QWEN3-8B-P4K-Q8: ours 1,908 ms (2,146 tok/s); llama.cpp 1,948 ms (2,103) (2026-11-08,
  logs/fusion-k09/window73/).
- [ ] C-3060-QWEN3-8B-P4K-Q4: ours 2,062.6 ms (1,986 tok/s), slowest run 2,071.6, over the binary16 cache (2,174.0 over
  the `f32` cache); llama.cpp 2,175.3 ms (1,883), fastest 2,165.6, in the same window (2026-11-09:
  logs/fusion-k09/window83/), 2,119 on 2026-11-08's quieter box.
- [ ] C-3060-QWEN3-8B-B16-Q8: ours 401.8 tok/s, slowest run 400.9; llama.cpp 335.2, fastest 336.5 (window87); vLLM
  (FP8) 127.4, fastest 127.9 (its engine's own rate 301.6: logs/fusion-k09/window89b/).
- [ ] C-3060-QWEN3-8B-B16-Q4: ours 411.1 tok/s, slowest run 410.1, its Q4_K and Q6_K products on the int8 tensor
  cores (logs/fusion-k09/winb16k/); llama.cpp 422.2, fastest 425.5, in the same window — 3.6% behind its fastest;
  409.8, slowest 409.2, against llama.cpp's 414.5, fastest 421.7 in logs/fusion-k09/winb16k2/ — 3.0%. Behind: its gated
  pair's products at 175 GB/s (11.5 ms of a 39.3 ms step) and its attention at 260 GB/s (10.6 ms); the attention's
  chunks on blocks of their own gained 0.2% (copy nx3b, winb16k2).
- [ ] C-3060-QWEN3-8B-S-Q8, C-3060-QWEN3-8B-S-Q4: waits on the S set.

**DeepSeek-V2-Lite.** Only 4-bit fits. Its own items: K-14 to K-18, K-24.
- [ ] C-3060-DSV2L-D1-Q4: ours 155.3 tok/s; llama.cpp 118.0.
- [ ] C-3060-DSV2L-D8K-Q4: ours 106.2 tok/s (2026-11-09, logs/fusion-k09/window80/); llama.cpp cannot allocate an
  8,192-position cache with every layer on the card (2.3 GB more); K-15 with K-30 on the latent cache.
- [ ] C-3060-DSV2L-P512-Q4: ours 2,928 tok/s on 2026-11-06 (2,592 with the grouped GEMM at one block a multiprocessor;
  the 10-25 3,154 measured code that never landed); llama.cpp 2,554 (logs/fusion-k09/window2/ab.log).
- [ ] C-3060-DSV2L-P4K-Q4: ours 1,855 ms (2,208 tok/s); llama.cpp 1,954 ms (2,097) (2026-11-08,
  logs/fusion-k09/window73/).
- [ ] C-3060-DSV2L-B16-Q4: ours 333.0 tok/s, slowest run 332.2 (2026-11-10, logs/fusion-k09/winmoe/; 334.9 in
  winb16k2); llama.cpp at -ngl 20, the most layers on the card that fit (-ngl 99 runs out of device memory allocating
  its 5.4 GB cache, -ngl 22 its 4.2 GB share of it), 11.3, fastest 20.7, in the same window under other sessions' load
  (35 rising to 185) — won (16× its fastest).
- [ ] C-3060-DSV2L-S-Q4: waits on the S set.

**gpt-oss-20b.** MXFP4 on every side. Its own items: K-19, K-20, K-24.
- [ ] C-3060-OSS20B-D1-MXFP4: ours 111.7 tok/s; llama.cpp 98.4.
- [ ] C-3060-OSS20B-D8K-MXFP4: ours 96.1 tok/s, slowest run 96.0 (2026-11-09, logs/fusion-k09/window80/); llama.cpp
  84.4, fastest 86.3, in the same window at load 70–83 — to re-measure on a quiet box.
- [ ] C-3060-OSS20B-P512-MXFP4: ours 2,731 tok/s on 2026-11-06 (1,991 the day before: the 10-25 2,509 measured code that
  never landed); llama.cpp 2,503. The MXFP4 rows regrouped at upload and the grouped GEMM's two blocks a multiprocessor
  (logs/fusion-k09/window/oss-regroup.log, logs/fusion-k09/window2/ab.log); to re-measure under 2.3.
- [ ] C-3060-OSS20B-P4K-MXFP4: ours 1,332.3 ms (3,074 tok/s), slowest run 1,339.5, over the binary16 cache (1,345.7 over
  the `f32` cache); llama.cpp 1,714.3 ms (2,389), fastest 1,709.1, in the same window (2026-11-09:
  logs/fusion-k09/window85/).
- [ ] C-3060-OSS20B-B16-MXFP4: ours 85.1 tok/s, slowest run 81.8, four matrices past the card's 12 GB read over PCIe
  x4 (2026-11-10, logs/fusion-k09/winmoe/; 86.4 in winb16k2); llama.cpp at -ngl 22, the most layers on the card that
  fit (-ngl 99 runs out of device memory allocating its compute buffers), 47.0, fastest 51.8, in the same window — won
  (1.58× its fastest).
- [ ] C-3060-OSS20B-S-MXFP4: waits on the S set.

**Qwen3.8-27B.** It does not fit 12 GB at 4 bits. The rivals are llama.cpp with partial offload (`-ngl`) and vLLM with
`--cpu-offload-gb`. Its own items: PARITY-HOST-RESIDENT-KV, PARITY-HOST-WEIGHT-STREAMING, PARITY-HOST-LAYERS-ON-CPU,
K-31, SERVE-SPEC-DECODE (MTP).
- [ ] C-3060-QWEN38-27B-D1-Q4: ours 0.93 tok/s, slowest run 0.86, the 195 matrices past an 11.2 GB device budget
  streamed to two device slots a launch at a time over the copy engine (2026-11-10, logs/fusion-k09/window92/, load
  19–34; 0.28 reading them mapped, window91); llama.cpp at -ngl 32 (its last layers and the output head on the card,
  the first on its CPU) 1.30, fastest 1.67, in the same window, one of its five runs beside a two-minute build of mine
  (0.285 at load 62–131, window90) — behind: a token moves the matrices past the card over PCIe x4, where llama.cpp's
  CPU reads its layers from DDR5; the layers past the card belong on the CPU tier (PARITY-HOST-LAYERS-ON-CPU).
- [ ] C-3060-QWEN38-27B-P512-Q4: ours 225.9 tok/s, slowest run 206.6, the same matrices streamed, each read once a
  chunk (window92; 23.8 reading them mapped, once a 128-row tile, window91); llama.cpp at -ngl 32 101.9, fastest
  115.4, in the same window — won.

#### i9-14900K (AVX2 and AVX-VNNI; no AVX-512 on this part)

The rivals are llama.cpp's CPU backend and vLLM's CPU backend. Ours is the CPU tier (PARITY-CPU-SHIM, `codegen_cpu`):
the decode step and prefill chunks linked from the one IR, products by checkpoint rows read in place, the activation
one word a 32-value block, a product's vector lanes its outputs (an exact int32 block dot a lane, eight outputs a
vector, one `f32` chain an output in block order). Measured 2026-11-10 (logs/fusion-k09/cpuwin6/, the box at load 28–70
beside other sessions), ours and llama.cpp b11635 (`-ngl 0 -t 8`) alternating run by run, both pinned to the eight
performance cores one thread a core, ten runs a side; vLLM's CPU backend is not measured yet:
- [ ] C-CPU-LLAMA1B-D1-Q8: ours 5.29 tok/s, slowest run 4.56; llama.cpp 2.91, fastest 3.28 — won.
- [ ] C-CPU-LLAMA1B-P512-Q8: ours 3,008 ms, slowest run 4,441; llama.cpp 8,040, fastest 5,824 — won.
- [ ] C-CPU-LLAMA1B-D1-Q4: ours 5.81 tok/s, slowest run 4.16; llama.cpp 2.83, fastest 3.59 — won.
- [ ] C-CPU-LLAMA1B-P512-Q4: ours 2,495 ms, slowest run 3,856; llama.cpp 4,261, fastest 3,636 — ahead on the mean by
  more than twice the error (604 ms), not every run.
- [ ] C-CPU-QWEN3-8B-D1-Q4: ours 1.36 tok/s, slowest run 0.82; llama.cpp 0.63, fastest 0.87 — ahead on the mean by
  more than twice the error, not every run.
- [ ] C-CPU-QWEN3-8B-P512-Q4: ours 12.2 s, slowest run 15.0; llama.cpp 28.2, fastest 21.3 — won.

#### Rented hardware (SETUP-8; each needs Tristen's approval to rent)

Every cell here is closed by its regime set plus the datacenter tickets:
- PARITY-QUANT-SCHEMES and PARITY-QUANT-GEMMS-DATACENTER;
- TRITON-TILE-TIER, TRITON-MMA-SELECTION, TRITON-TMA, TRITON-WARP-SPECIALIZATION and TRITON-MODULO-SCHEDULER;
- TECH-PDL, plus TECH-2SM-UMMA-TMEM on sm_100.

Until measured, these cells stay open and the book is not complete.

H100 SXM 80 GB (sm_90); regimes D1, P4K, B64 (64 concurrent sequences) and S:
- [ ] C-H100-LLAMA31-8B-{D1,P4K,B64,S}-BF16
- [ ] C-H100-LLAMA31-8B-{D1,P4K,B64,S}-FP8
- [ ] C-H100-QWEN38-27B-{D1,P4K,B64,S}-BF16
- [ ] C-H100-QWEN38-27B-{D1,P4K,B64,S}-FP8
- [ ] C-H100-OSS120B-{D1,P4K,B64,S}-MXFP4
- [ ] C-H100-DSV2L-{D1,P4K,B64,S}-BF16

B200 (sm_100); the same regimes:
- [ ] C-B200-LLAMA31-8B-{D1,P4K,B64,S}-NVFP4
- [ ] C-B200-QWEN38-27B-{D1,P4K,B64,S}-NVFP4
- [ ] C-B200-OSS120B-{D1,P4K,B64,S}-MXFP4

Eight H100s, with tensor and expert parallelism (PARITY-TENSOR-PARALLEL-IR, PARITY-ALLREDUCE-NORM,
PARITY-EXPERT-PARALLEL, SERVE-DUAL-BATCH-OVERLAP):
- [ ] C-8XH100-DSV2-{D1,S}-FP8

MI300X (TRITON-AMD-SHIM):
- [ ] C-MI300X-LLAMA31-8B-{D1,S}-BF16
- [ ] C-MI300X-OSS120B-{D1,S}-MXFP4

Arc B580 (PARITY-XPU-SHIM):
- [ ] C-B580-LLAMA1B-{D1,P512}-Q8

A braced list is one box per regime in the ledger: LEDGER-3 expands `{D1,P4K,B64,S}` into four cell ids.

### 2.5 Where each Copy bar comes from

Wherever a ticket's Copy bar names one stack, that stack is where the design comes from. The bar itself is the row
maximum: the fastest kernel any stack in 2.2 ships for that op on that chip, at our shapes, timed by SETUP-4. The table
names who sets the bar for each op class on the 3060; SETUP-4 measures every name in it.

- [ ] **Decode GEMV, 8-bit:** llama.cpp `mul_mat_vec_q`, vLLM's W8A8 (CUTLASS) and FP8 weight-only (Marlin) paths,
  ExLlamaV3's GEMV, TensorRT-LLM's weight-only batched GEMV.
- [ ] **Decode GEMV, 4-bit:** llama.cpp `mul_mat_vec_q` on Q4_K, Marlin W4A16, ExLlamaV3, TensorRT-LLM's weight-only
  int4, MLC.
- [ ] **Decode GEMV, 16-bit:** cuBLAS, vLLM's bf16 path, TensorRT-LLM.
- [ ] **Prefill GEMM, 8-bit:** llama.cpp `mul_mat_q`, CUTLASS int8 (vLLM W8A8), cuBLASLt int8, Triton's matmul,
  TensorRT-LLM.
- [ ] **Prefill GEMM, 4-bit:** Marlin, llama.cpp `mul_mat_q` on Q4_K, ExLlamaV3, TensorRT-LLM.
- [ ] **Prefill GEMM, 16-bit:** cuBLASLt, CUTLASS, Triton's matmul tutorials.
- [ ] **Decode attention:** FlashInfer's batch decode, FlashAttention-2's split-KV decode, vLLM's Triton unified
  attention, llama.cpp's flash attention (vector and MMA paths), TensorRT-LLM's XQA.
- [ ] **Prefill attention:** FlashAttention-2, FlashInfer's prefill, Triton's fused-attention tutorial, llama.cpp's MMA
  flash attention.
- [ ] **Latent (MLA) decode:** vLLM's Triton MLA, FlashInfer's MLA where it runs on sm_86, llama.cpp's MLA path.
- [ ] **Row kernels (norms, rotations, activations, quantizes, gathers):** vLLM's CUDA kernels, llama.cpp's.
- [ ] **Expert GEMMs:** vLLM's Triton `fused_moe`, Marlin MoE, `triton_kernels` matmul, llama.cpp's `mul_mat_id`,
  TensorRT-LLM's MoE.
- [ ] **Routing and top-k:** vLLM's `topk_softmax` and `grouped_topk`, `triton_kernels` routing and top-k, llama.cpp's
  arg-sort top-k.
- [ ] **Gated-delta chunks and decode:** the FLA kernels vLLM vendors, vLLM's CUDA GDN decode, llama.cpp's GDN.
- [ ] **Sampling:** FlashInfer's sampling, vLLM's Triton top-k and top-p, the rejection samplers of both.

### 2.6 When a cell stays lost

If every ticket a cell names is done and the cell is still lost under 2.3:
1. [ ] Record both sides with nsys on one boot (`--cuda-graph-trace=node` for graph replays). Write the per-token kernel
   tables side by side into the cell's ledger row.
2. [ ] Classify the difference: a kernel slower than the rival's; a launch the rival fuses that we do not; a gap between
   launches; host time; a scheduling or batching policy.
3. [ ] Open a ticket per difference on the board (section 12.1), citing the rival's code as `external/competitors/<stack>/<path>` ·
   `<anchor>`. Add the cell's id to the ticket's goal, and add the ticket to the cell's closers.
4. [ ] Leave the cell's box open until the new tickets close and the cell is won.

### 2.7 What this book does not promise

- **Time.** A rival's next release can be faster. A ticked cell records the date and every version, and a public claim
  quotes the row it rests on. When a rival in 2.2 ships a release, SETUP-7 re-measures the cells it leads.
- **Hardware we do not hold.** The rented cells need rented machines. Renting is Tristen's call, and until those cells
  are measured the book is not complete.
- **Model breadth.** vLLM runs hundreds of architectures. The cells name ours. Every new model is one `model::author`
  program; the shared fusions make it fast, but this book does not list vLLM's model registry.
- **The FPGA and spatial chips.** vLLM and Triton do not target them. Their SOTA is judged against published
  accelerator results in their own campaign ledgers. This book carries only their structure: the shims, the links, and
  the shared fusions they reuse.

---

## 3. One program on every backend

**The claim.** A model is authored once, in LOGOS, and emitted for any backend: the VM, the CPU, RISC-V, CUDA, the
spatial chips, the FPGA. That is the product. Concretely:
- `model::author` writes its computation as one LOGOS program, from the checkpoint's own spec;
- every backend lowers a cut of that program and nothing else;
- every artifact names the exact text it came from.

The VM runs it as its meaning. The GPU, the FPGA, the spatial chips and the CPU run it as fast as each can. A fusion is
decided once on the program's IR, and each chip runs it natively, as parts, or refuses it by name.

This section traces the one program through every backend the compiler has:
- 3.1 the program and its cuts;
- 3.2 the path every backend takes;
- 3.3 each backend, what it runs today and what it must run;
- 3.4 every fusion on every chip;
- 3.5 what the exact-methods tickets of section 12.17 give each chip;
- 3.6 the command: author once, emit anywhere.

Every item is a checkbox, ticked only with its receipt.

### 3.1 The program and its cuts

`model::program` names the cuts. Each is written by `Cut::write` with placeholder inputs (each token and position 1,
weights named by tensor alone) that a backend replaces at run time:
- **Decode:** `Decode { ctx }`, the step over a cache of `ctx` positions; `DecodeEmbed`, `DecodeLayer { layer, ctx }`
  and `DecodeHead`, its parts.
- **Prefill:** `Prefill { rows, ctx }`, a chunk of one sequence; `PrefillLayer` and `PrefillHead`, its parts.
- **Batch:** `Batch { rows, ctx }`, `rows` sequences at once, each with its own state; `BatchEmbed`, `BatchLayer` and
  `BatchHead`, its parts.
- **Forward:** `Forward`, `ForwardLayer` and `ForwardHead`, the single token's forward with no history.
- **Split:** `written_split` writes a decode step, a batch, or one layer of either, split over `tp` chips.

Every artifact records its cut's `Identity`: the cut, the chip count, and the SHA-256 of the text, as the line
`LOGOS program: <cut>, sha256 <hex>`. A program another writer produced cannot pass for this one.

What holds this today, in `one_program_lock`:
- [x] `every_lane_form_names_the_cut_of_the_one_program_it_compiled`: every CUDA lane form carries its cut's identity.
- [x] `the_engine_program_is_read_off_the_one_program` and `the_lane_and_the_engine_lower_one_program`: the D5005
  engine and the CUDA lane lower the same decode step.
- [x] `the_spatial_configuration_writes_the_checkpoints_own_program`, `a_spatial_chips_programs_are_the_decode_step_cut_apart`
  and `an_sn50_bundle_names_the_cut_of_every_program_it_lowered`: a spatial chip's embedding, layer and head programs
  are the decode step cut apart, op for op.
- [x] `only_the_one_program_calls_the_authors_writers`: no other module writes a model's program.
- [x] `the_hand_written_lanes_are_a_closed_set`: four hand-written CUDA files remain, as oracles only (`qwen38.rs`,
  `qwen38_batched.rs`, `qwen_batched.rs`, `qwen_emit.rs`). The set can only shrink.
- [x] `the_hand_lowering_is_only_an_oracle`: the D5005 engine's hand lowering is the oracle its lock compares, never
  the program it runs.

What is missing (tickets in section 12.18):
- [ ] **The VM as the reference for every backend, directly.** Today the VM runs `author::prefill` and
  `author::decode_step` at tiny width for seven architecture shapes, held to `model::reference` within 1e-9
  (`model_conformance_lock::every_architecture_prefills_to_the_references_logits`,
  `every_architecture_decodes_a_step_to_the_references_logits`), and the Qwen3.8 tiny cuts in the `qwen38_*` locks.
  The backends are held to the reference, so their equality to the VM is indirect. Batch cuts, the JIT, and a real
  width are missing. ONE-PROGRAM-VM-REFERENCE.
- [ ] **`largo bringup` lowers one checkpoint to every backend in one command** and prints the matrix of section 3.3.
  Today three backends take author cuts, and `bringup` knows exactly those three: the CUDA lane (decode and prefill),
  the D5005 engine (decode) and the SN50 path (every cut family, Qwen3.5 and Qwen3.8 only). ONE-PROGRAM-EVERY-BACKEND.
- [ ] **No second writer on any product path.** Three writers besides `model::author` still feed backends:
  - `Int8Model::decode_step_lg` (`crates/logicaffeine_compile/src/spatial/int8_model.rs`), the int8 baby transformer
    the CUDA persistent and fabric images run;
  - `scripts/accel/gen_qwen_tiny_lg.py`, which feeds the single-thread CUDA tier's model lock;
  - `scripts/gen_tiny_transformer_lg.py`, which feeds the WPE fabric engine's whole-transformer run.

  The four hand-written CUDA files stay as golden oracles only. ONE-PROGRAM-RETIRE-SECOND-WRITERS.
- [ ] **A program no recognizer lifts still lowers on every backend**, through the generic path, instead of being
  refused. ONE-PROGRAM-GENERAL-LOOPS.
- [ ] **Section 3.4's map is held to every shim's answers by a lock.** ONE-PROGRAM-FUSION-MAP-LOCK.

The front end's vocabulary is the 28 `TensorOpSpec` kinds of `recognize_stages`: Matmul, Elementwise, SwiGlu,
FusedRow, Rope, Silu, Sigmoid, Gelu, Scale, Decay, L2Norm, Transpose, HeadSlice, Join, HeadMatmul, CausalConv,
GatedDelta, Attention, FullAttention, Moe, RowGather, RowsGather, ArgMax, SetRow, IntRequant, IntClamp, Relu and
LutAttention. The spatial pipeline lowers all 28; the other backends refuse some (section 3.3).

### 3.2 The path every backend takes

```
model::author(spec)  →  model::program::Cut::write   one LOGOS text per cut, with its Identity
        │
        ├── the VM and the tree-walker run the text itself              the meaning; every other backend equals it
        │
        └── the front end (recognize_stages → tensor_graph::front)      typed tensor ops, or a loop nest it keeps
              └── the one IR (tensor_graph)                              ops, tensors, facts, the evaluator ≡ the VM
                    └── the shared optimizer                            the fusion table, the e-graph, proofs
                          └── each chip's shim: native | parts | refused by name
                                ├── CUDA link        lane kernels, generated tile kernels, SASS rescheduling
                                ├── CPU link         vectorized C or Rust through the AOT backends
                                ├── AMD, Intel links HIP, SYCL (new)
                                ├── spatial links    CUDA persistent, Tensix, RDU, fabric Verilog
                                └── D5005 link       engine commands, its weight image
                                      └── each chip's own path: tiles, pipelines, placement, packing, graphs
```

- [ ] Every backend in section 3.3 enters at the front end, or runs the text itself (the VM), and at no other point
  (`one_ir_lock::every_backend_lowers_the_tensor_graph`, MIGRATE-3).
- [ ] Every artifact on every backend carries its cut's identity line
  (`one_program_lock::every_backend_artifact_names_its_cut`, ONE-PROGRAM-EVERY-BACKEND).

### 3.3 Each backend: today, and what it must run

Each entry gives the path today, the cuts it lowers, the lock that holds it equal to the VM, and the tickets that
close its gap. "Equal" means bit for bit where the chip's numerics claim exactness, and within the declared bound
elsewhere (OPT-6).

**Today, three backends take author cuts:**
- the CUDA lane: decode and prefill, eight tiny architectures and every scoreboard model;
- the D5005 engine: decode;
- the SN50 spatial path: every cut family, Qwen3.5 and Qwen3.8 only.

Every other backend either takes general LOGOS programs that no lock feeds an author cut, or runs another writer. The
entries below say which, and what closes the gap.

- [ ] **The tree-walker and the bytecode VM: the meaning.**
  - Entry: `vm_outcome` → `vm::run_to_outcome`, and `tw_outcome` (`crates/logicaffeine_compile/src/compile.rs`).
  - Today:
    - `author::prefill` and `author::decode_step` run at d = 16 over seven shapes (qwen3, qwen3moe, llama, tied
      llama, the qwen35 hybrid), held to `model::reference` within 1e-9 (`model_conformance_lock`);
    - the Qwen3.8 tiny decode, prefill and batch cuts run in `qwen38_decode_step_lock`, `qwen38_prefill_lock` and
      `qwen38_batched_decode_lock`;
    - the forward cut agrees between the VM and the tree-walker (`qwen38_authored_lock`).
  - Must run:
    - every cut family (decode, prefill, batch, forward, and the split cuts) of every architecture at tiny width, in
      the fast suite;
    - one real-width decode token for Llama-3.2-1B and Qwen3.5-0.8B, on demand, as a golden that needs no other
      engine.
  - Equal to: itself. The tree-walker equals the VM bit for bit; every other backend is held to the VM directly, not
    only through the reference.
  - Closed by: ONE-PROGRAM-VM-REFERENCE.
- [ ] **The EXODIA JIT.**
  - Entry: `logicaffeine_jit::install()` registers the VM's native tier; the command line installs it.
  - Today: it would run whatever the VM runs, but no lock installs it under an author cut. Its own tests use generic
    loops.
  - Must run: every tiny cut, tiered, bit for bit equal to the VM. That makes it the fast reference for the tiny-model
    corpus.
  - Closed by: ONE-PROGRAM-VM-REFERENCE (its JIT column).
- [ ] **The CPU (x86 and Arm), through the AOT backends.**
  - Entries: Rust (`compile_project`), AST C (`compile_to_c`), bytecode C (`compile_to_c_via_bytecode`), WebAssembly
    (`compile_to_wasm`).
  - Today: all compile general LOGOS programs, and none is fed an author cut.
  - Must run, as two paths:
    - the generic path: every cut compiled as written and equal to the VM, from the evaluator
      (BEYOND-EVALUATOR-SPECIALIZATION);
    - the fast path: the CPU shim's vectorized links (PARITY-CPU-SHIM), which must win the CPU cells of section 2.4.
  - Closed by: ONE-PROGRAM-EVERY-BACKEND, PARITY-CPU-SHIM.
- [ ] **RISC-V (RV64GC), its own AOT tier, and the soft core on the boards.**
  - Entry: `compile_to_riscv` (`largo emit riscv`), RV64GC assembly with its `logos_rt.c` shim, run under qemu. The
    same tier's soft core runs on the Nexys A7 boards with all 115 VM ops.
  - Today: general LOGOS programs, every VM op, equal to the VM. No author cut is fed to it, and it has no vector
    extension path.
  - Must run, as two paths:
    - every tiny cut compiled as written, equal to the VM under qemu and on the soft core;
    - a fast path through the CPU shim with the RISC-V vector extension (RVV) and its int8 dot products, the same links
      as x86 and Arm (PARITY-CPU-SHIM).
  - Closed by: ONE-PROGRAM-EVERY-BACKEND, PARITY-CPU-SHIM.
- [ ] **The CUDA lane.**
  - Entry: `lane::compile` → `compile_holding`. Each form calls `program::written`, then `recognize_stages`, then
    `StepGraph::build`.
  - Today:
    - decode, plus prefill chunks at every power of two up to 512;
    - all eight tiny architectures and every scoreboard model;
    - 16 table rules and the always-on fusions (section 5.9 lists them);
    - held to the reference on the GPU (`cuda_lane_lock::every_tiny_architecture_generates_what_the_reference_computes`),
      and fused equal to apart (`cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`).
  - It refuses, by name:
    - the op kinds Transpose, FullAttention, IntRequant, IntClamp, Relu and LutAttention;
    - softmax and layer-norm rows;
    - RMSNorm with a bias;
    - a matmul by a computed matrix.
  - Must run: every cut, batches included, through the shared optimizer and the CUDA link (MIGRATE-1). Every op kind,
    with the refused ones linked through the generated tier (TRITON-TILE-TIER, ONE-PROGRAM-GENERAL-LOOPS).
  - Closed by: MIGRATE-1, the tier-1 PARITY tickets, ONE-PROGRAM-GENERAL-LOOPS.
- [ ] **The older CUDA forward and the hand-written lanes.**
  - Today:
    - `qwen_emit.rs` (`compile_to_cuda_forward`) walks its own stage list. It refuses 20 op kinds and masked
      attention, so it runs no author cut at all.
    - `qwen38.rs`, `qwen38_batched.rs` and `qwen_batched.rs` are hand-written generators held by golden tokens.
    - All four are in the hand-written closed set.
  - Must run: nothing on the product path. Each stays a golden oracle until the one program's lowering beats it on its
    own numbers, then it retires.
  - Closed by: MIGRATE-2, ONE-PROGRAM-RETIRE-SECOND-WRITERS.
- [ ] **The spatial pipeline: SambaNova RDU (SN40L, SN50).**
  - Entry: `largo emit rdu --model` → `compile_qwen38_to_rdu_streamed`.
  - Today:
    - Forward, Decode, Batch, Prefill and split cuts, each as its embedding, layer and head programs, every bundle
      naming its cut;
    - equal to the VM on the tiny configuration (`qwen38_decode_step_lock`, `qwen38_prefill_lock`,
      `qwen38_batched_decode_lock`, `qwen38_decode_bundle_lock`, `qwen38_rdu_lock`);
    - Qwen3.5 and Qwen3.8 only: `Qwen38Cfg::from_spec` refuses every other architecture;
    - its fusions are `fuse_generic`, `pack_phases` and `share_q8_views`. `forward` is off, and multicast never fires
      on the SN50's crossbar.
  - Must run: any architecture's cuts through the shared optimizer and the RDU shim.
  - Closed by: MIGRATE-3, ONE-PROGRAM-RETIRE-SECOND-WRITERS.
- [ ] **The spatial pipeline: CUDA persistent, Tenstorrent and the fabric.**
  - Today:
    - `CudaPersistentBackend` runs the int8 baby transformer's step (`Int8Model`) and test chains;
    - `TensixBackend` runs single tensor ops;
    - `FabricBackend` runs `Int8Model` board images and chains.

    None takes an author cut.
  - Must run: every cut of every architecture, through the same spatial pipeline the RDU uses. Then:
    - the whole decode step as one persistent kernel per layer kind on the GPU (BEYOND-SPATIAL-ON-GPU);
    - the tiny cuts on the Nexys boards.
  - Closed by: MIGRATE-3, ONE-PROGRAM-EVERY-BACKEND, ONE-PROGRAM-RETIRE-SECOND-WRITERS (`Int8Model`'s writer).
- [ ] **The D5005 decode engine (Stratix 10).**
  - Entry: `program::lower` → `written(spec, Cut::Decode { ctx })` → `StepGraph` → `lower_graph`.
  - Today:
    - decode only; the 27B ran token-exact on the card;
    - held to the reference at every knob point (`d5005_engine_lock`, `d5005_machine_lock`), and the program read off
      the step graph equals the hand lowering (`d5005_lower_graph_lock`);
    - its merges: q/k/v into one matvec; convs and SiLUs into `Conv`; the two gates into `Gates`; q and gate halves
      into one matvec and a split; sigmoid and multiply into `SigmoidGate`; the K and V rows into `CacheAppend`; and
      the arg-max consumed by `Advance`;
    - it refuses, by name: MLA, attention biases, fused qkv, sliding windows, sinks, soft caps, GELU, gpt-oss's SwiGLU,
      experts and post-norms.
  - Must run: the decode step of every architecture its shim covers, reading fused symbols instead of matching blocks,
    and each refusal shrinking as the engine gains commands.
  - Closed by: MIGRATE-4, PARITY-KV-CACHE-QUANT, PARITY-SOFTCAP (its shim answer).
- [ ] **AMD GPUs.** Today: no link. Must run: every cut through an AMD shim and link. Closed by: TRITON-AMD-SHIM,
  SETUP-8.
- [ ] **Intel GPUs.** Today: no link. Must run: every cut through an Intel shim and link. Closed by: PARITY-XPU-SHIM,
  SETUP-8.
- [ ] **The general-program tiers.**
  - Entries: the single-thread CUDA tier (`compile_to_cuda`), the single-core Tenstorrent tier
    (`compile_to_tenstorrent`), the analog lane (`compile_to_analog`), the Plasticine lane (`compile_to_plasticine`),
    and the WPE fabric engine on the boards.
  - Today: they run general LOGOS programs equal to the VM. The only "model" any of them runs comes from a Python
    writer (`gen_qwen_tiny_lg.py` for single-thread CUDA, `gen_tiny_transformer_lg.py` for the WPE engine).
  - Must answer, per author cut: run it as a general program (slow and correct), or refuse it by name with the reason
    (the analog lane's range certificate, a memory limit). Never silently.
  - Closed by: ONE-PROGRAM-EVERY-BACKEND, ONE-PROGRAM-RETIRE-SECOND-WRITERS.

The matrix that ONE-PROGRAM-EVERY-BACKEND prints, and that `one_program_lock` holds, has one cell per (model, cut,
backend). Every cell is lowered with its identity and is equal to the VM, or is refused by name:
- [ ] Every scoreboard model × {decode, prefill, batch} × every backend above: lowered or refused by name (receipt:
  `one_program_lock::every_backend_lowers_every_model_or_refuses_by_name`).
- [ ] Every lowered tiny cell equals the VM (receipt: `one_program_lock::every_backend_equals_the_vm_on_every_tiny_cut`).

### 3.4 Every fusion on every chip

One entry per fused symbol: the ledger's classes, the classes the catalog adds, the kernel map's fusions and the edge layouts. Each entry gives the target answer for every chip family and its state today; a box is ticked when that chip's shim answers the target and its link computes what the parts compute (section 7, INV-FUSED-EQUALS-APART). The VM is the reference and fuses nothing: every fused symbol is defined by its parts, which the VM runs. For the spatial chips, "today" is the SN50 path, the one spatial backend that takes author cuts (CUDA persistent, Tensix and the fabric take none yet); for the D5005 it is `lower_graph`. ONE-PROGRAM-FUSION-MAP-LOCK holds this map to the shims' answers once OPT-2 lands, so it cannot drift from the code.

#### Norms and residuals

- [ ] **`norm_weight`**: RMSNorm with its weight in one row kernel.
  - [x] CUDA: native (receipt: `cuda_fusion_census_lock::a_weighted_norm_is_one_row_kernel`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [x] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4); today: a norm command.
- [ ] **`add_norm`**: the residual add and the next RMSNorm in one row pass.
  - [x] CUDA: native (receipt: `cuda_fusion_census_lock::an_add_then_norm_runs_as_one_kernel_and_matches_the_vm_on_the_gpu`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3); today: runs apart on the SN50 path.
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4); today: runs apart.
- [ ] **`norm_then_add`**: a post-norm, then the residual add (Gemma's sandwich).
  - [x] CUDA: native (receipt: `lane_compile_lock::the_sandwiched_step_norms_each_sublayer_output_inside_the_sums_norm` / `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4); today: refused: post-norms.
- [ ] **`norm_scale`**: a norm followed by a scalar scale.
  - [x] CUDA: native (receipt: `lane_compile_lock::a_scaled_norm_is_one_row_kernel` / `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4).
- [ ] **`norm_weight_offset`**: Gemma's (1 + w) inside the norm.
  - [ ] CUDA: native (PARITY-NORM-WEIGHT-OFFSET); today: depends on whether the checkpoint stores w + 1.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4).
- [ ] **`norm_quant`**: a norm that writes the next projection's int8 words and scales.
  - [x] CUDA: native (receipt: `cuda_fusion_census_lock::a_norm_feeding_an_int8_projection_quantizes_in_its_epilogue`); parts with `fuse_quant` off.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3); today: not applicable: the SN50 path reads stored Q8 weights.
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4); today: not applicable: its numerics live in the engine's units.
- [ ] **`add_norm_quant`**: add, norm and quantize in one pass.
  - [x] CUDA: native (receipt: `cuda_fusion_census_lock::an_add_then_norm_feeding_an_int8_projection_quantizes_in_its_epilogue`); parts with `fuse_quant` off.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3); today: not applicable: the SN50 path reads stored Q8 weights.
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4); today: not applicable: its numerics live in the engine's units.
- [ ] **`dual_norm`**: two independent norms in one kernel.
  - [ ] CUDA: native (PARITY-DUAL-NORM); today: gap.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4).
- [ ] **`gated_norm`**: the gated-delta output's RMSNorm × SiLU(z), quantized.
  - [x] CUDA: native (receipt: `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4); today: parts: the block reader merges the `GatedNorm` command itself, outside the shared table.
- [ ] **`embed_norm`**: the embedding gather and the first norm (K-02).
  - [x] CUDA: native (receipt: `lane_compile_lock::the_embedding_gather_norms_its_row_in_the_same_launch` / `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4).
- [ ] **`embed_scale`**: the embedding gather with its scale folded in (Gemma's √d).
  - [x] CUDA: native (receipt: `lane_compile_lock::the_sandwiched_step_windows_its_local_layers_and_fuses_its_gates`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4).
- [ ] **`gemv_norm_epilogue`**: a projection with the residual and the next norms in its last-arriving block (K-05, K-07, K-11, K-16).
  - [x] CUDA: native for a step's one row in Q8_0 (receipt: `lane_compile_lock::a_projection_feeding_an_add_and_norm_finishes_them_in_its_last_block` / `cuda_lane_lock::the_last_block_norm_computes_what_the_norm_launch_computes`) and in every block format the library decodes (receipt: `lane_compile_lock::a_block_format_projection_finishes_its_add_and_norm_in_its_last_block` / `cuda_lane_lock::the_block_projections_last_block_norms_as_the_norm_launch_does`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: the epilogue rides the product's section (MIGRATE-3).
  - [ ] D5005: native in its matrix command's epilogue, else parts (MIGRATE-4).
- [ ] **`prefill_norm_epilogues`**: wide-chunk norms, residuals and quantizes in GEMM epilogues (K-28).
  - [ ] CUDA: native (PARITY-PREFILL-NORM-EPILOGUES); today: gap: a block per row, its own launch.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: the epilogue rides the product's section (MIGRATE-3).
  - [ ] D5005: native in its matrix command's epilogue, else parts (MIGRATE-4).
- [ ] **`attn_residual_norm`**: attention over block residuals with the norm.
  - [ ] CUDA: native (PARITY-ATTN-RESIDUAL-NORM); today: gap.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4).
- [ ] **`compensated_residual`**: fast2sum-compensated residual sums.
  - [ ] CUDA: native (TECH-COMPENSATED-RESIDUAL); today: gap.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4).
- [ ] **`recompute_fanout`**: a cheap producer recomputed per consumer.
  - [ ] CUDA: native (PARITY-RECOMPUTE-FANOUT); today: gap.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4).

#### Rotations, heads and caches

- [ ] **`qk_norm_rope`**: per-head q/k norm and rotation.
  - [x] CUDA: native (receipt: `lane_compile_lock::the_dense_steps_fuse_where_their_dataflow_allows` / `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3); today: runs apart.
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4); today: runs apart.
- [ ] **`qk_norm_rope_kv_append`**: norm, rotation and the cache row append.
  - [x] CUDA: native (receipt: `lane_compile_lock::the_dense_steps_fuse_where_their_dataflow_allows` / `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4).
- [ ] **`rope_kv_append`**: rotation and the cache append.
  - [x] CUDA: native.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4).
- [ ] **`rope_qk_one_launch`**: query and key rotated in one launch.
  - [x] CUDA: native (`rope_parts`); today: two `lk_rope` launches in Gemma's census (PARITY-ROPE-QK-ONE-LAUNCH).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4).
- [ ] **`qk_norm_rope_gate_split`**: gated attention's split, norms, partial rotation and gate.
  - [ ] CUDA: native (PARITY-QK-NORM-ROPE-GATE-SPLIT); today: partial: `head_norm_rope` plus a separate gate.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4); today: q and gate halves merged into one matvec and a split.
- [ ] **`qkv_head_epilogue`**: one q/k/v projection with the per-head norm, rotation and append in its epilogue (K-03).
  - [x] CUDA: native for a step's one row (receipt: `lane_compile_lock::a_decode_layers_query_key_and_value_are_one_launch_with_their_head_epilogue` / `cuda_lane_lock::the_head_epilogue_computes_what_the_rope_launches_compute`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: the epilogue rides the product's section (MIGRATE-3).
  - [ ] D5005: native in its matrix command's epilogue, else parts (MIGRATE-4).
- [ ] **`horizontal_norm_rope_insert`**: one kernel over a multi-branch row.
  - [ ] CUDA: native (PARITY-HORIZONTAL-NORM-ROPE-INSERT); today: gap.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4).
- [ ] **`kv_cache_quant`**: f16, int8 and Hadamard int8 caches written and read in place (K-30).
  - [x] CUDA: native: the `cache_precision` knob's fp8 e4m3 cache, written rounded by every cache writer and read by every attention kernel (receipt: `lane_compile_lock::a_quantized_cache_is_written_and_read_in_its_format` / `cuda_lane_lock::an_e4m3_cache_decodes_alike_in_every_form_and_near_its_reference`); int8 and Hadamard int8 schemes: PARITY-KV-CACHE-QUANT.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: an attention section (MIGRATE-3).
  - [ ] D5005: refused: a numerics point of the engine, not a fusion (OPT-6); today: f16, int8 and Hadamard int8 caches.
- [ ] **`hadamard_rotation`**: a Hadamard transform before quantizing.
  - [ ] CUDA: native (PARITY-KV-CACHE-QUANT); today: gap.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: refused: a numerics point of the engine, not a fusion (OPT-6); today: added for the Hadamard int8 cache.
- [ ] **`cross_layer_launch`**: every layer's cache insert in one launch.
  - [ ] CUDA: native (PARITY-CROSS-LAYER-LAUNCH); today: gap.
  - [ ] CPU: native: an edge layout (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: an edge layout (MIGRATE-3).
  - [ ] D5005: native: an address pattern (MIGRATE-4).
- [ ] **`host_weight_streaming`**: matrices past the card streamed to it once a chunk.
  - [x] CUDA: native: each matrix a launch reads whole copied into one of two device slots a launch at a time (`residence::Plan::streamed`, PARITY-HOST-WEIGHT-STREAMING).
  - [ ] CPU: not applicable: its matrices are in its memory.
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: a DMA stream (MIGRATE-3).
  - [ ] D5005: native: the weight stream (MIGRATE-4).
- [ ] **`host_layers_on_cpu`**: the layers past the card computed by the CPU tier.
  - [ ] CUDA: native with the CPU tier (PARITY-HOST-LAYERS-ON-CPU); today: gap.
  - [ ] CPU: native: its linked layers (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): not applicable: the host's layers are the CPU tier's.
  - [ ] D5005: not applicable.
- [ ] **`host_resident_kv`**: cache rows read from pinned host memory.
  - [ ] CUDA: native (PARITY-HOST-RESIDENT-KV); today: gap: host-mapped weights only.
  - [ ] CPU: native: an edge layout (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: an edge layout (MIGRATE-3).
  - [ ] D5005: native: an address pattern (MIGRATE-4).
- [ ] **`mla`**: latent attention, absorbed in decode and expanded in prefill.
  - [x] CUDA: native (receipt: `lane_compile_lock::the_latent_step_absorbs_each_heads_query_and_reads_its_values_in_the_cache` / `cuda_lane_lock::the_expanded_latent_attention_computes_what_the_absorbed_one_computes`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: an attention section (MIGRATE-3).
  - [ ] D5005: native where its attention command has it (MIGRATE-4); today: refused.
- [ ] **`mla_prologue`**: q and the latent in one projection with norm, rotation, append and absorption (K-14).
  - [ ] CUDA: native (PARITY-MLA-PROLOGUE); today: gap: the absorption runs as its own launch.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: the epilogue rides the product's section (MIGRATE-3).
  - [ ] D5005: native in its matrix command's epilogue, else parts (MIGRATE-4).

#### Attention

- [ ] **`flash_decode`**: split decode attention with its fold.
  - [x] CUDA: native (receipt: `lane_compile_lock::the_decode_step_attends_a_long_cache_in_chunks_one_launch_a_layer` / `cuda_lane_lock::a_long_prompt_attends_in_chunks_alike_in_every_form`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [x] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: an attention section (MIGRATE-3).
  - [ ] D5005: native where its attention command has it (MIGRATE-4).
- [ ] **`flash_prefill`**: tiled prefill attention on the tensor cores.
  - [x] CUDA: native (receipt: `lane_compile_lock::a_wide_chunk_attends_in_tiles` / `cuda_lane_lock::the_tensor_core_attention_computes_what_the_attention_computes`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [x] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: an attention section (MIGRATE-3).
  - [ ] D5005: native where its attention command has it (MIGRATE-4).
- [ ] **`attention_variants_inline`**: windows, sinks, caps and masks inside attention.
  - [x] CUDA: native: windows, sinks and soft caps inside every attention kernel (receipt: `lane_compile_lock::every_attention_kernel_takes_a_cap_and_a_mask` / `cuda_lane_lock::a_capped_attention_computes_what_its_reference_computes`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: an attention section (MIGRATE-3).
  - [ ] D5005: native where its attention command has it (MIGRATE-4); today: refused: sliding windows, sinks, soft-capped scores.
- [ ] **`softcap`**: soft-capped logits and attention scores.
  - [x] CUDA: native (receipt: `lane_compile_lock::a_capped_attention_caps_inside_its_kernel` / `cuda_lane_lock::a_capped_attention_computes_what_its_reference_computes`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [x] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: in the attention's sections and fold, a row section (ARCH-SOFTCAP).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4); today: refused by name (`LogitSoftcap`).
- [ ] **`attn_output_quant`**: attention writing the output projection's int8 words (K-04).
  - [x] CUDA: native: the lane's decode step and short chunks, heads of whole 32-value groups (receipt: `lane_compile_lock::a_decode_attention_quantizes_its_output_in_its_fold` / `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`); a batch's sequences each quantizing their row in their stores, set with its quantized form into the table's (receipt: `lane_compile_lock::a_batchs_attentions_and_delta_rules_quantize_their_rows_in_their_store`); a chunk in tiles apart; parts with `fuse_quant` off; the older forward (receipt: `cuda_fusion_census_lock::an_attention_feeding_an_int8_projection_quantizes_in_its_epilogue`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: an attention section (MIGRATE-3); today: not applicable: the SN50 path reads stored Q8 weights.
  - [ ] D5005: native where its attention command has it (MIGRATE-4); today: not applicable: its numerics live in the engine's units.
- [ ] **`attn_output_gate`**: a gated attention's output multiplied by its gate's activation as the attention stores it (K-13).
  - [x] CUDA: native: the lane's decode step, short chunks and a batch's sequences (one attention over them, each sequence's row gated by its row of the batch's gate), the gate read where its projection wrote it (receipt: `lane_compile_lock::a_gated_attention_gates_its_output_in_its_store` / `lane_compile_lock::a_batchs_gated_attentions_gate_their_rows_in_their_store` / `cuda_lane_lock::the_gated_attention_store_computes_what_the_gated_product_computes` / `cuda_lane_lock::a_batch_decodes_each_sequence_as_its_own_step_does`); a batch's gated rows quantized in their stores too (PARITY-BATCH-ROW-QUANT); a chunk in tiles apart; the older forward: parts.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: an attention section (MIGRATE-3).
  - [ ] D5005: native where its attention command has it (MIGRATE-4).
- [ ] **`row_write_quant`**: a row set carrying its rows' quantized form into the table's, the quantize of a table assembled a row at a time made by its rows' producers (PARITY-BATCH-ROW-QUANT).
  - [x] CUDA: native: the lane's batch, each sequence's attention, gated attention or delta rule quantizing its row in its store, the set laid out by the batch's sequences or, held apart, a launch copying the rows' words and scales (receipt: `lane_compile_lock::a_batchs_attentions_and_delta_rules_quantize_their_rows_in_their_store` / `cuda_lane_lock::a_batch_decodes_each_sequence_as_its_own_step_does`); the older forward: parts.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): parts (MIGRATE-3).
  - [ ] D5005: parts (MIGRATE-4).
- [ ] **`attn_gqa_row_packing`**: a KV group's heads on the matrix unit's rows in decode (K-04).
  - [ ] CUDA: native (PARITY-ATTN-GQA-ROW-PACKING); today: gap: decode attention is SIMT f32.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: an attention section (MIGRATE-3).
  - [ ] D5005: native where its attention command has it (MIGRATE-4).
- [ ] **`attn_lse_merge`**: a shared merge of partial attention by log-sum-exp.
  - [x] CUDA: native: folded inside the attention launch; PARITY-ATTN-LSE-MERGE holds what remains.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: an attention section (MIGRATE-3).
  - [ ] D5005: native where its attention command has it (MIGRATE-4).
- [ ] **`mla_unabsorb_in_fold`**: latent attention un-absorbing its values in its fold (K-15).
  - [ ] CUDA: native (PARITY-MLA-UNABSORB-IN-FOLD); today: gap: the value product and the quantize run after the fold.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: an attention section (MIGRATE-3).
  - [ ] D5005: native where its attention command has it (MIGRATE-4).
- [ ] **`mixed_batch_attention`**: prefill and decode rows in one launch.
  - [ ] CUDA: native (PARITY-MIXED-BATCH-ATTENTION); today: gap.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: an attention section (MIGRATE-3).
  - [ ] D5005: native where its attention command has it (MIGRATE-4).
- [ ] **`sparse_attention`**: indexers, block top-k and sparse attention.
  - [ ] CUDA: native (PARITY-SPARSE-ATTENTION); today: gap.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: an attention section (MIGRATE-3).
  - [ ] D5005: native where its attention command has it (MIGRATE-4).

#### Activations and matrix products

- [ ] **`glu_act_mul`**: gate · act(up) in one pass.
  - [x] CUDA: native (receipt: `lane_compile_lock::the_hybrid_step_fuses_the_recurrence_and_the_gate`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [x] D5005: native: the sigmoid gate as `SigmoidGate` (MIGRATE-4); the FFN's gate, up and SwiGLU run apart.
- [ ] **`glu_act_mul_quant`**: the gated activation, quantized out.
  - [x] CUDA: native (receipt: `lane_compile_lock::the_dense_steps_fuse_where_their_dataflow_allows`); parts with `fuse_quant` off.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3); today: not applicable: the SN50 path reads stored Q8 weights.
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4); today: not applicable: its numerics live in the engine's units.
- [ ] **`gemv_gate_up_glu`**: gate, up and the activation in one GEMV (K-06).
  - [x] CUDA: native (receipt: `lane_compile_lock::the_dense_steps_fuse_where_their_dataflow_allows`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: the epilogue rides the product's section (MIGRATE-3); today: runs apart.
  - [ ] D5005: native in its matrix command's epilogue, else parts (MIGRATE-4); today: runs apart.
- [ ] **`elementwise_chains`**: pointwise chains in one kernel.
  - [ ] CUDA: native (MIGRATE-1); today: the lane: parts; the older forward: native (receipt: `cuda_elementwise_fusion_lock::a_linear_elementwise_chain_fuses_into_one_kernel`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section; today: parts in the shared table, and `fuse_generic` merges adjacent maps after lowering.
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4); today: no rule.
- [ ] **`elementwise_regions`**: pointwise DAG regions in one kernel.
  - [ ] CUDA: native (PARITY-ELEMENTWISE-REGIONS); today: the lane: parts; the older forward: native (receipt: `cuda_elementwise_fusion_lock::a_diamond_shaped_pointwise_region_fuses_into_one_kernel`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [x] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section, each member's value handed to its reader, never stored (receipt: `spatial_forward_lock::a_pointwise_region_is_one_section_and_one_body`).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4).
- [ ] **`relu_sqr`**: ReLU².
  - [x] CUDA: native (receipt: `lane_compile_lock::a_relu_square_compiles_and_folds_into_its_producer` / `cuda_lane_lock::a_relu_square_computes_what_the_reference_computes`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4).
- [ ] **`masked_expert_activation`**: batched expert activation skipping padded rows.
  - [ ] CUDA: native (PARITY-MASKED-EXPERT-ACTIVATION); today: gap.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4).
- [ ] **`gemm_bias`**: bias in the GEMM epilogue.
  - [x] CUDA: native (receipt: `lane_compile_lock::the_biased_step_folds_its_biases_and_routes_by_an_f32_router`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: the epilogue rides the product's section (MIGRATE-3).
  - [ ] D5005: native in its matrix command's epilogue, else parts (MIGRATE-4); today: refused: attention biases.
- [ ] **`gemm_scale`**: scales in the GEMM epilogue.
  - [x] CUDA: native (receipt: `lane_compile_lock::a_scaled_projection_scales_in_its_epilogue` / `cuda_lane_lock::a_scaled_projection_computes_what_its_launches_compute_apart`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: the epilogue rides the product's section (MIGRATE-3).
  - [ ] D5005: native in its matrix command's epilogue, else parts (MIGRATE-4).
- [ ] **`merged_qkv`**: q, k and v as one matrix product.
  - [x] CUDA: native (receipt: `lane_compile_lock::a_wide_chunk_in_a_block_format_runs_on_the_tensor_cores` / `cuda_lane_lock::a_gemm_in_parts_computes_each_parts_gemm`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: the epilogue rides the product's section (MIGRATE-3); today: one shared view (`share_q8_views`); the products stay separate.
  - [x] D5005: native in its matrix command's epilogue, else parts (MIGRATE-4); today: q/k/v merged into one matvec.
- [ ] **`merged_gate_up`**: gate and up as one matrix product.
  - [x] CUDA: native (receipt: `cuda_lane_lock::a_paired_gemm_computes_what_its_gate_and_up_compute_apart`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: the epilogue rides the product's section (MIGRATE-3).
  - [ ] D5005: native in its matrix command's epilogue, else parts (MIGRATE-4).
- [ ] **`gemm_dequant_inkernel`**: block formats decoded inside the matrix product.
  - [x] CUDA: native (receipt: `cuda_fusion_census_lock::an_int8_weight_projection_dequantizes_inside_its_gemv`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [x] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: the epilogue rides the product's section (MIGRATE-3).
  - [ ] D5005: native in its matrix command's epilogue, else parts (MIGRATE-4).
- [ ] **`affine_input_fold`**: input normalization folded into one affine.
  - [ ] CUDA: native (PARITY-AFFINE-INPUT-FOLD); today: gap.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4).

#### Experts

- [ ] **`router_topk`**: router softmax or sigmoid, top-k and renormalization in one kernel.
  - [x] CUDA: native (receipt: `cuda_fusion_census_lock::a_sparse_moe_routes_with_one_topk_kernel_and_two_fused_expert_kernels`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [x] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: expert sections (MIGRATE-3); the SN50 path takes Qwen3.5 and Qwen3.8 only.
  - [ ] D5005: refused by name: the engine has no expert commands; today: refused: experts.
- [ ] **`grouped_topk`**: group-limited routing in one kernel.
  - [ ] CUDA: native (PARITY-GROUPED-TOPK); today: gap.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: expert sections (MIGRATE-3).
  - [ ] D5005: refused by name: the engine has no expert commands.
- [ ] **`router_shared_gate_merge`**: router and shared-expert gate as one product.
  - [ ] CUDA: native (PARITY-ROUTER-SHARED-GATE-MERGE); today: gap.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: expert sections (MIGRATE-3).
  - [ ] D5005: refused by name: the engine has no expert commands.
- [ ] **`moe_token_sort`**: tokens sorted to their experts on the device.
  - [x] CUDA: native (receipt: `lane_compile_lock::a_wide_chunks_experts_run_on_the_tensor_cores` / `cuda_lane_lock::the_grouped_expert_gemms_compute_what_the_slabs_compute`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: expert sections (MIGRATE-3); today: refused: the SN50 path takes Qwen3.5 and Qwen3.8 only.
  - [ ] D5005: refused by name: the engine has no expert commands; today: refused: experts.
- [ ] **`moe_direct_pairs`**: the sort skipped for tiny batches.
  - [ ] CUDA: native (PARITY-MOE-DIRECT-PAIRS); today: gap.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: expert sections (MIGRATE-3).
  - [ ] D5005: refused by name: the engine has no expert commands.
- [ ] **`moe_gate_up_act`**: every expert's gate, up and activation in one launch (K-17).
  - [x] CUDA: native (receipt: same).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [x] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: expert sections (MIGRATE-3); the SN50 path takes Qwen3.5 and Qwen3.8 only.
  - [ ] D5005: refused by name: the engine has no expert commands; today: refused: experts.
- [ ] **`moe_gather_in_operand`**: the token gather inside the expert product (K-24).
  - [ ] CUDA: native (PARITY-MOE-GATHER-IN-OPERAND); today: gap: `lk_moe_gather` is its own launch.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: expert sections (MIGRATE-3).
  - [ ] D5005: refused by name: the engine has no expert commands.
- [ ] **`moe_down_combine`**: expert down projections combined in their blocks.
  - [x] CUDA: native (receipt: same).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [x] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: expert sections (MIGRATE-3); the SN50 path takes Qwen3.5 and Qwen3.8 only.
  - [ ] D5005: refused by name: the engine has no expert commands; today: refused: experts.
- [ ] **`moe_deferred_finalize`**: the combine folded into the next add and norm (K-18).
  - [x] CUDA: native: the decode step's last down block, a chunk's and a batch's combine a block a row (receipt: `lane_compile_lock::a_decode_mixture_adds_and_norms_in_its_last_down_block` / `lane_compile_lock::a_wide_chunks_expert_combine_folds_into_the_next_add_and_norm` / `cuda_lane_lock::the_mixtures_last_step_norms_as_the_norm_launch_does`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: expert sections (MIGRATE-3).
  - [ ] D5005: refused by name: the engine has no expert commands.
- [ ] **`shared_expert_fusion`**: the shared expert inside the routed experts' launch.
  - [x] CUDA: native (receipt: `lane_compile_lock::the_shared_expert_folds_into_its_mixture_where_nothing_else_reads_its_parts`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: expert sections (MIGRATE-3); today: refused: the SN50 path takes Qwen3.5 and Qwen3.8 only.
  - [ ] D5005: refused by name: the engine has no expert commands; today: refused: experts.

#### Recurrent layers

- [ ] **`gdn_step`**: the gated-delta recurrence step.
  - [x] CUDA: native (receipt: `lane_compile_lock::the_hybrid_step_fuses_the_recurrence_and_the_gate` / `cuda_lane_lock::the_chunked_delta_rule_computes_what_the_delta_rule_a_row_at_a_time_computes`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [x] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: a recurrent section (MIGRATE-3).
  - [ ] D5005: native where a command has it (MIGRATE-4); today: parts: `Delta` and `GatedNorm` commands.
- [ ] **`gdn_input_epilogue`**: every gated-delta input projection with conv, SiLU, L2 and gates in its epilogue (K-09).
  - [x] CUDA: native for a step's one row, its gates in Q8_0 or projected in `f32` (receipt: `lane_compile_lock::a_gated_delta_layers_input_projections_are_one_launch_with_conv_and_gates` / `cuda_lane_lock::the_gdn_input_epilogue_computes_what_its_launches_compute_apart`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: the epilogue rides the product's section (MIGRATE-3).
  - [ ] D5005: native in its matrix command's epilogue, else parts (MIGRATE-4).
- [ ] **`gdn_decode_whole`**: the delta update, gated norm and quantize in one kernel (K-10).
  - [ ] CUDA: native (PARITY-GDN-DECODE-WHOLE); today: partial: `lk_delta_step` then `lk_delta_norm`.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: a recurrent section (MIGRATE-3).
  - [ ] D5005: native where a command has it (MIGRATE-4).
- [ ] **`gdn_post_conv_prep`**: everything after the conv in one prefill kernel.
  - [ ] CUDA: native (PARITY-GDN-POST-CONV-PREP); today: partial: `conv_silu_l2` and `gates`.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: a recurrent section (MIGRATE-3).
  - [ ] D5005: native where a command has it (MIGRATE-4).
- [ ] **`conv_bias_silu`**: the short conv, its bias and SiLU (and the q/k L2 norm).
  - [x] CUDA: native (receipt: `lane_compile_lock::the_hybrid_step_fuses_the_recurrence_and_the_gate`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: a recurrent section (MIGRATE-3); today: runs apart.
  - [x] D5005: native: `Conv` (MIGRATE-4); the L2 norm runs apart.
- [ ] **`conv_parts`**: a step's causal convolutions (q, k and v), each through its SiLU and normed where the model norms it, as one launch over their channels.
  - [x] CUDA: native (receipt: `lane_compile_lock::the_hybrid_step_convolves_its_query_key_and_value_in_one_launch`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: a recurrent section (MIGRATE-3).
  - [ ] D5005: native: one `Conv` command over the merged q/k/v row (MIGRATE-4).
- [ ] **`gdn_gate_projections`**: the two gated-delta gates with the two `f32` projections they read, as one launch.
  - [x] CUDA: native (receipt: `lane_compile_lock::the_hybrid_step_takes_its_gates_from_its_activation_in_one_launch`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: the epilogue rides the product's section (MIGRATE-3).
  - [ ] D5005: native in its matrix command's epilogue, else parts (MIGRATE-4).
- [ ] **`gdn_gating`**: the decay and β gates in one kernel.
  - [x] CUDA: native (receipt: `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3); today: runs apart.
  - [x] D5005: native where a command has it, else parts (MIGRATE-4); today: merged into `Gates`.

#### The head and the token

- [ ] **`fused_sampling`**: the token chosen on the device and fed back (arg-max today).
  - [x] CUDA: native (receipt: `lane_compile_lock::the_dense_steps_fuse_where_their_dataflow_allows`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3); today: a separate arg-max section.
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4); today: parts: the block reader folds the arg-max into `Advance` itself, outside the shared table.
- [ ] **`cap_argmax`**: a soft cap an arg-max alone reads, folded into the arg-max's read: the cap strictly increasing, the token the raw row's arg-max (PARITY-SOFTCAP).
  - [x] CUDA: native: the cap taken in the arg-max's registers, the capped row written as it is read where the host reads it, none where it does not (`CudaKnobs::logits` `Unread`) (receipt: `lane_compile_lock::a_greedy_step_folds_its_final_cap_into_its_arg_max` / `cuda_lane_lock::a_greedy_step_generates_what_its_capped_logits_pick`).
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3); today: parts.
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4); today: parts.
- [ ] **`sampling`**: top-k, top-p, temperature, min-p, penalties and seeded sampling in the head's epilogue (K-21).
  - [ ] CUDA: native (PARITY-SAMPLING); today: arg-max only.
  - [ ] CPU: native (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: one row section (MIGRATE-3).
  - [ ] D5005: native where a command has it, else parts (MIGRATE-4).
- [ ] **`last_layer_rows`**: a prefill chunk's last layer computing only its output rows (K-29).
  - [ ] CUDA: native (PARITY-LAST-LAYER-ROWS); today: partial: the head reads the last row; the last layer computes every row.
  - [ ] CPU: parts: one thread pool per step (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: the whole step as one placed program (BEYOND-SPATIAL-ON-GPU).
  - [ ] D5005: native: the engine runs the step as one program.

#### Placement (edge layouts)

- [ ] **`strided_read`**: a part read in place by stride, no copy.
  - [x] CUDA: native (receipt: `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`).
  - [ ] CPU: native: an edge layout (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: an edge layout (MIGRATE-3); today: runs apart.
  - [ ] D5005: native: an address pattern (MIGRATE-4); today: the `Split` command.
- [ ] **`row_write`**: a result written straight into a row of state.
  - [x] CUDA: native (receipt: `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`).
  - [ ] CPU: native: an edge layout (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: an edge layout (MIGRATE-3); today: runs apart.
  - [ ] D5005: native: an address pattern (MIGRATE-4); today: refused as a placement by the engine's shim; the block reader merges K and V into `CacheAppend` itself.
- [ ] **`join_in_place`**: parts written into their joined buffer.
  - [x] CUDA: native (receipt: `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`).
  - [ ] CPU: native: an edge layout (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: an edge layout (MIGRATE-3).
  - [ ] D5005: native: an address pattern (MIGRATE-4); today: refused with MLA.
- [ ] **`strided_values`**: strided reads of a values tensor.
  - [x] CUDA: native (receipt: `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`).
  - [ ] CPU: native: an edge layout (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: an edge layout (MIGRATE-3).
  - [ ] D5005: native: an address pattern (MIGRATE-4); today: refused with MLA.

#### The whole step

- [ ] **`cuda_graphs`**: every form replayed as one captured graph (a schedule property).
  - [x] CUDA: native: a schedule property of the CUDA path (receipt: `cuda_fusion_census_lock::the_compiled_forward_is_captured_as_one_cuda_graph`).
  - [ ] CPU: parts: one thread pool per step (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: the whole step as one placed program (BEYOND-SPATIAL-ON-GPU).
  - [ ] D5005: native: the engine runs the step as one program.
- [ ] **`persistent_layer_megakernel`**: a decoder layer as one persistent launch (K-22).
  - [ ] CUDA: native (PARITY-PERSISTENT-LAYER-MEGAKERNEL); today: gap: graph replay of separate launches.
  - [ ] CPU: parts: one thread pool per step (PARITY-CPU-SHIM).
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [x] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: the whole step as one placed program (BEYOND-SPATIAL-ON-GPU).
  - [ ] D5005: native: the engine runs the step as one program.

#### Across chips

- [ ] **`allreduce_norm`**: the all-reduce fused with the next norm.
  - [x] CUDA: native: a tensor-parallel step's all-reduce folded into the add and norm its sum feeds, one launch reading every rank's partial where it lives (receipt: `lane_compile_lock::a_tensor_parallel_step_folds_each_all_reduce_into_its_norm` / `cuda_lane_lock::a_tensor_parallel_step_computes_what_its_launches_compute_apart`); the ranks on one device, their transport across devices PARITY-ALLREDUCE-NORM.
  - [ ] CPU: refused by name until tensor parallelism spans CPUs.
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): native: the spatial all-reduce stage (PARITY-ALLREDUCE-NORM).
  - [ ] D5005: refused by name: one card.
- [ ] **`allreduce_sumsq_norm`**: a norm of a row split across chips, each chip's part normed where it lives by the whole row's sum of squares, the one value a row a chip sends (the QK norm across every head of OLMo 2 and MiniMax-M2 under tensor parallelism).
  - [x] CUDA: native: the norm by parts, one launch of each part's sum of squares and one of each part normed by their sum, every part read and written in its own chip's arrays; the norm of a joined row apart sums it by the same parts (receipt: `lane_compile_lock::a_tensor_parallel_whole_qk_norm_sends_one_sum_of_squares_a_chip` / `cuda_lane_lock::a_tensor_parallel_whole_qk_norm_computes_what_its_launches_compute_apart`); the ranks on one device, their transport across devices PARITY-ALLREDUCE-NORM.
  - [ ] CPU: runs as its parts until tensor parallelism spans CPUs.
  - [ ] AMD, Intel: native (TRITON-AMD-SHIM, PARITY-XPU-SHIM).
  - [ ] Spatial chips (CUDA persistent, Tensix, RDU, fabric): runs as its parts; native with the spatial all-reduce stage (PARITY-ALLREDUCE-NORM).
  - [ ] D5005: refused by name: one card.

### 3.5 What the exact methods give each chip

The tickets of section 12.17 build on the IR, not on one chip. Each therefore lands on every chip its shim lets it
reach:
- [ ] **BEYOND-CERTIFIED-OPTIMAL-FUSION:** every chip's fusion plan, optimal under that chip's cost, with a proof.
- [ ] **BEYOND-EXACT-MODULO-SCHEDULER:** GPU copy pipelines, spatial FIFO depths, D5005 command overlap and fabric
  retiming, from one scheduler.
- [ ] **BEYOND-CERTIFIED-ALLOCATION:** GPU shared and tensor memory, the arena, spatial SRAM residency and D5005
  on-chip buffers, from one allocator.
- [ ] **BEYOND-PROVEN-PROTOCOLS:** GPU counters, rings and grid syncs, and spatial handshakes, under one certificate.
- [ ] **BEYOND-BIT-EXACT-EQUIVALENCE:** integer fragments on the GPU and the CPU, and pipelined fabric datapaths.
- [ ] **BEYOND-SUPEROPTIMIZER:** a proven sequence library for each chip's instruction set (CUDA and CPU first).
- [ ] **BEYOND-OWN-SASS-SCHEDULER:** sm_86 cubins.
- [ ] **BEYOND-PROVEN-NUMERICS:** every chip's numerics contract.
- [ ] **BEYOND-LOOP-LEVEL-FUSION** and **BEYOND-EVALUATOR-SPECIALIZATION:** every chip, through the generic path.
- [ ] **BEYOND-EXACT-WEIGHT-FOLDS:** every chip, since a fold rewrites the one program's weights.
- [ ] **BEYOND-SPATIAL-ON-GPU:** the GPU, through the spatial pipeline that already serves Tensix, RDU and the fabric.
- [ ] **BEYOND-RUNTIME-PATCHING:** the GPU while serving.
- [ ] **BEYOND-SYMMETRY-PRUNED-SEARCH:** every chip's tuning search.
- [ ] **BEYOND-TRAINING:** every chip that holds a training step.

### 3.6 The command: author once, emit anywhere

The product claim is one command line: a checkpoint in, its one LOGOS program authored, and the program emitted for the
backend named. Today `largo emit --model` exists for the RDU alone; the command line refuses `--model` for every other
target ("`--model` applies to `emit rdu`"). `largo bringup` lowers to three chips. The command this section asks for:

```
largo emit <target> --model <checkpoint.gguf> [--cut decode|prefill|batch|forward] [--ctx N] [--rows N] [--tp N] -o DIR
```

It writes the authored program (`program.lg`, with its identity line), the target's artifacts, and a report. The report
names every fusion fired or declined (OPT-3), and every refusal by name. One box per target; each is ticked with a
lock that emits a tiny model through it and runs the result equal to the VM, or, for hardware not on the box, compiles
it and records the refusal to run:
- [ ] `largo emit lg --model`: the authored program itself, every cut, with its identity. The VM and the JIT run it:
  `largo run program.lg`.
- [ ] `largo emit c --model` and `largo emit rust --model`: the CPU, generic path first, then the CPU shim's links.
- [ ] `largo emit riscv --model`: RV64GC under qemu and on the soft core; RVV through the CPU shim.
- [ ] `largo emit wasm --model`: the browser and Node, through the WebAssembly backend.
- [ ] `largo emit cuda --model`: the CUDA lane's program (the fused `.cu`, its sidecar, its schedule), replacing today's
  single-thread CUDA tier for model programs.
- [ ] `largo emit rdu --model`: today's command, reading any architecture's program, not only Qwen3.5 and Qwen3.8.
- [ ] `largo emit tenstorrent --model`: the decode step through the spatial pipeline's Tensix lowering.
- [ ] `largo emit plasticine --model`: the Spatial bundle of each cut, checked in Spatial's simulator.
- [ ] `largo emit fabric --model`: the fabric Verilog of the tiny cuts, and the board image (new target).
- [ ] `largo emit d5005 --model`: the engine program and its weight image, from the decode step (new target; today
  `bringup` and the examples carry it).
- [ ] `largo emit analog --model`: the analog lane's answer per cut, a lowering or a refusal with its range
  certificate's reason.
- [ ] `largo emit amd --model` and `largo emit intel --model`: their links when TRITON-AMD-SHIM and PARITY-XPU-SHIM
  land; a named refusal until then.
- [ ] `largo bringup <checkpoint>` with no target: every target above, and the matrix of section 3.3 printed.

Held by `cli_emit_model_lock::every_emit_target_takes_a_model_or_refuses_it_by_name` (ONE-PROGRAM-EVERY-BACKEND), and by
`cli_docs_lock` once `docs/cli.md` documents the flag on every target.

---

## 4. The kernel map: where we beat vLLM and Triton

This section is the core of the book. It walks a decode step and a prefill chunk op chain by op chain. For each chain
it gives:
- what runs today, in launches and in measured time;
- the bytes and FLOPs the chain must move at our models' real shapes;
- how vLLM and Triton build the same chain, kernel by kernel;
- the kernel that beats them.

Every item is a checkbox. A box is ticked only with its receipt beside it: a test name, a log path under `logs/`, or a
ledger row. Each K item names the ticket that owns it (section 12).

Sections 4.1 to 4.14 walk the 3060. Sections 4.15 to 4.17 walk the D5005 engine, the spatial chips and the CPU
the same way (K-33 to K-46), so every chip the one program reaches has its op chains, floors and bars.

Where a K item's Copy names one stack, that stack is where the design comes from. The bar is the row maximum of
section 2.5: the fastest kernel any rival ships for that op at our shapes, timed by SETUP-4.

### 4.1 The box and how to read the numbers

**The machine.**
- RTX 3060 12 GB, sm_86, 28 SMs.
- Measured streaming read bandwidth: about 336 GB/s. The lane's MoE kernels sustained 91% of it on 2026-10-17; SETUP-4
  re-measures it.
- Spec at boost clock: about 102 dense int8 tensor TOPS, and about 25.5 TFLOPS for f16 with f32 accumulation (GeForce
  Ampere runs f32 accumulation at half rate). SETUP-4 measures what a pure MMA loop sustains.

**How floors are computed.**
- A decode floor is the bytes one token must read (every layer's weights, only the active experts, the head) divided by
  336 GB/s.
- The bytes are summed from each checkpoint's GGUF tensor table. Draft (MTP) blocks are excluded, and recurrent state
  traffic is listed separately.
- "Today" numbers come from `benchmarks/results/models/rtx3060_scoreboard.json` (one boot, 2026-10-25, five runs each)
  unless a line names another receipt.

**What vLLM brings to a fair comparison.** vLLM's core no longer loads GGUF, so its rows run Hugging Face checkpoints
at matched bits:
- a 4-bit vLLM row (AWQ or GPTQ through Marlin) is compared with our 4-bit row (Q4_K_M, IQ4_XS);
- an 8-bit vLLM row (W8A8) with our Q8_0 row;
- bf16 with our Dequant lane.

Never compare across bit widths.

### 4.2 The floors: every model, decode and prefill

Decode (batch 1, 6-token prompt, 128 generated):

- [ ] **Llama-3.2-1B Q8_0: 1.313 GB/token, floor 3.91 ms (256 tok/s).** Today 233.3 tok/s = 4.29 ms (91% of floor);
  llama.cpp 213.8. The gap is 0.38 ms, 24 µs a layer. Target ≥ 245 tok/s (≥ 96%).
  Closed by K-02, K-03, K-05, K-07 and K-22.
- [ ] **Gemma-3-1B Q8_0: 1.063 GB/token, floor 3.16 ms (316 tok/s).** Today 237.1 tok/s = 4.22 ms (75%); llama.cpp
  179.7. The gap is 1.06 ms, 41 µs a layer. Target ≥ 290 tok/s (≥ 92%).
  Closed by K-02, K-03, K-04, K-05, K-07 and K-22.
- [ ] **Gemma-2-2B Q8_0: 2.778 GB/token, floor 8.27 ms (121 tok/s).** Today: not compiled (soft caps refused).
  Target ≥ 110 tok/s.
  Closed by K-21 (PARITY-SOFTCAP), then the Gemma items.
- [ ] **Qwen3.5-0.8B Q8_0: 0.801 GB/token of weights plus 37.7 MB of GDN state traffic, floor 2.49 ms (402 tok/s).**
  Today 309.6 tok/s = 3.23 ms (77%); llama.cpp 237.8. The gap is 0.74 ms, 31 µs a layer. Target ≥ 370 tok/s (≥ 92%).
  Closed by K-09 to K-13 and K-22.
- [ ] **Qwen3-8B Q8_0: 8.042 GB/token, floor 23.9 ms (42 tok/s).** Today 40.6 tok/s (97%); llama.cpp 39.0. Decode at
  short context is done. What remains is long context, through the KV cache (K-30), and speculation (SERVE-SPEC-DECODE).
- [ ] **DeepSeek-V2-Lite Q4_K_M: 1.608 GB/token, floor 4.79 ms (209 tok/s).** Today 155.3 tok/s = 6.44 ms (74%);
  llama.cpp 118.0. The gap is 1.65 ms, 61 µs a layer. Target ≥ 190 tok/s (≥ 91%).
  Closed by K-14 to K-18 and K-22.
- [ ] **gpt-oss-20b MXFP4: 2.575 GB/token, floor 7.66 ms (131 tok/s).** Today 111.7 tok/s = 8.95 ms (86%); llama.cpp
  98.4. The gap is 1.29 ms, 54 µs a layer. Target ≥ 122 tok/s (≥ 93%).
  Closed by K-19, K-20 and K-22.
- [ ] **Qwen3.8-27B Q4_K_M: 16.7 GB/token plus 302 MB of GDN state traffic (48 layers × 3 MiB × read and write).**
  It does not fit 12 GB: the 3060 run (W2a) is correctness only, at 0.91 tok/s over PCIe x4. On a ≥ 24 GB card the
  floor at 336 GB/s is 50.6 ms.
  K-31 halves the state traffic; SERVE-SPEC-DECODE (MTP) multiplies tokens per weight read.
- [ ] **vLLM side by side.** Every row above gains its vLLM number at matched bits (SETUP-5) before any claim.

Prefill (one 512-token chunk; FLOPs ≈ 2 × active weights per token, plus attention):

- [ ] **Llama-3.2-1B: today 15,276 tok/s, about 38 effective int8 TOPS; llama.cpp 12,216.** Target ≥ 22,000 tok/s.
  Closed by K-23, K-26, K-28 and K-29.
- [ ] **Gemma-3-1B: today 16,316 tok/s; llama.cpp 11,050.** Target ≥ 22,000.
- [ ] **Qwen3.5-0.8B: today 15,154 tok/s; llama.cpp 11,781.** Target ≥ 21,000. Closed by K-27 in particular.
- [ ] **Qwen3-8B: today 2,413 tok/s, about 40 effective TOPS; llama.cpp 2,178.** Target ≥ 3,400. K-23 is most of it.
- [ ] **DeepSeek-V2-Lite: today 3,154 tok/s; llama.cpp 2,554.** Target ≥ 4,200. Closed by K-24 and K-26.
- [ ] **gpt-oss-20b: today 2,509 tok/s; llama.cpp 2,503, inside llama.cpp's spread.** Target ≥ 3,300. Closed by K-24
  and K-25.

### 4.3 Decode, the dense attention layer (Llama, Qwen3, Gemma)

The census behind these items is the performance lane's Gemma-3-1B decode program of 2026-10-25 18:05: 368 launches a
token, 14 a layer:

| # | Launch | Grid | What it does |
|---|---|---|---|
| 1 | `lk_rmsnorm` | 1 block | add and norm, quantize out |
| 2 | `lk_gemv_q8` | 128 blocks | Wq |
| 3 | `lk_gemv_q8` | 32 blocks | Wk |
| 4 | `lk_gemv_q8` | 32 blocks | Wv, writing its cache row |
| 5 | `lk_rope` | 4 blocks | q head norm and rotation |
| 6 | `lk_rope` | 1 block | k head norm, rotation and cache row |
| 7 | `lk_attention` | 16 split blocks | attention |
| 8 | `lk_quant` | 4 blocks | quantize the attention output |
| 9 | `lk_gemv_q8` | 144 blocks | Wo |
| 10 | `lk_rmsnorm` | 1 block | post-attention norm |
| 11 | `lk_rmsnorm` | 1 block | add and pre-FFN norm, quantize out |
| 12 | `lk_gemv_glu` | 216 blocks | gate, up and GeGLU |
| 13 | `lk_gemv_q8` | 144 blocks | Wd |
| 14 | `lk_rmsnorm` | 1 block | post-FFN norm |

Seven of the fourteen run on 1 to 4 blocks of a 28-SM chip. They are latency, not bandwidth. Llama and Qwen3 follow
the same rules without the sandwich norms (about 11 a layer); K-01 records their exact lists.

- [x] **K-01 · The census is a lock** (owned by MIGRATE-1).
  - [x] `lane_compile_lock::every_decode_layer_launches_what_its_census_says`: the per-layer launch list above for
    Gemma-3-1B, Llama-3.2-1B, Qwen3-8B, Qwen3.5-0.8B, DeepSeek-V2-Lite and gpt-oss-20b, recorded today. Each later K
    item lowers its count on purpose (receipt: `tests/fixtures/lane_census/`, logs/one-ir/live-record-1029.log).

- [ ] **K-02 · The step's first launch: gather, scale, residual and the first norm** (PARITY-EMBED-NORM).
  Today it takes `lk_gather_q8` (5 blocks), then `lk_rmsnorm` (1 block). The row is 1152 to 4096 floats, so the work
  is nothing and the cost is two launch latencies.
  - [x] Copy vLLM's gather plus first RMSNorm (`external/competitors/vllm/vllm/model_executor/layers/fused_embed_norm.py` · `def fused_embed_norm(`): one program per row.
  - [x] Design: a block of the norm's threads a row (`lk_gather_norm`; receipt: `lane_compile_lock::the_embedding_gather_norms_its_row_in_the_same_launch`).
    - It reads the row in place in its block format (`lk_gather_q8` / `lk_gather_blk` decoders), scales it, and writes
      the raw row for the residual.
    - It sums squares in registers, applies the weight, and writes the int8 words and scales the first projection
      reads.
    - Bit-identical to the two launches, with the same reduction tree as `lk_rmsnorm` (`lk_rmsnorm_row`; receipt:
      `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`, logs/one-ir/live-gpu-1030.log).
  - [x] Beyond: in prefill, all chunk rows in one launch, a block per row (receipt: `lane_compile_lock::the_embedding_gather_norms_its_row_in_the_same_launch`).
  - [x] Bar: one launch fewer per token on every model (receipt: `tests/fixtures/lane_census/`).
  - [ ] Bar: profile shows the gather-and-norm ≤ one norm's time.

- [ ] **K-03 · One q/k/v GEMV with the per-head epilogue** (PARITY-QKV-HEAD-EPILOGUE).
  Today Gemma takes five launches (Wq 128 blocks, Wk 32, Wv 32, rope q 4, rope k 1); Llama and Qwen3 take five
  (three GEMVs, two ropes). The GEMVs read 1.88 MB (Gemma) or 6.7 MB (Llama), a 5.6 µs or 20 µs floor. The ropes are
  pure latency.
  - [x] Copy:
    - vLLM merges q, k and v into one GEMM (`QKVParallelLinear`, the ledger's `merged_qkv`).
    - Flint's decode QKV kernel uses one thread-block cluster per head and gathers through distributed shared memory to
      the cluster's first CTA, which applies the q/k norm, the rotation and the paged cache write
      (`external/competitors/vllm/vllm/models/flint/ops/qkv_fused.py`, sm_100 only).
  - [x] Design for sm_86 (`lk_gemv_parts_rope`; receipt: `lane_compile_lock::a_decode_layers_query_key_and_value_are_one_launch_with_their_head_epilogue`):
    - One launch over the concatenated rows [q | k | v] (1536 rows for Gemma, 3072 for Llama), four rows a warp, the
      grid sized to fill 28 SMs at least twice.
    - A per-head completion counter in global memory: each block adds the rows it finished.
    - The block that completes a head (the last arriver, the pattern `lk_attention`'s fold already uses, its counter
      self-resetting) re-reads that head's rows from L2 (64 to 256 floats). It applies:
      - q: RMSNorm with its weight, then the NEOX rotation from the rotary tables, written to the query buffer;
      - k: norm, rotation, and the cache row (quantized when K-30's cache scheme says so);
      - v: the cache row.
    - No separate rope launch, and no partial head ever leaves the block that finishes it.
  - [x] Numerics: the per-head norm sums in the same order as `lk_rope`'s fused norm (`lk_rope_head`), so the result
    is bit-identical (`cuda_lane_lock::the_head_epilogue_computes_what_the_rope_launches_compute`,
    `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`; receipt: logs/one-ir/live-gpu-1030.log).
  - [x] Beyond: Flint needs sm_100 clusters, while this is one launch on sm_86 with no cluster.
  - [x] Bar: 5 → 1 launches a layer (receipt: `tests/fixtures/lane_census/`).
  - [ ] Bar: the merged GEMV at ≥ 85% of bandwidth (receipt: profile).

- [ ] **K-04 · Decode attention on the tensor cores, its output quantized in its fold** (PARITY-ATTN-GQA-ROW-PACKING
  and PARITY-ATTN-QUANT-IN-FOLD).
  Today Gemma's `lk_attention<4,1>` runs 16 split blocks over a 512-window, followed by `lk_quant` (4 blocks). The
  attention is SIMT f32; head dim 256, one KV head shared by 4 query heads.
  - [ ] Copy vLLM's unified attention: the KV group's query heads packed onto MMA M rows, BLOCK_M = max(16, next power
    of two of the group), a 3D split over the span when the grid is small, segments reduced after
    (`external/competitors/vllm/vllm/v1/attention/ops/triton_unified_attention.py` · `def unified_attention(`).
  - [ ] Design:
    - **Group packing.** The group's 4 (Gemma, Llama, Qwen3) or 8 (gpt-oss) query heads fill the m16n8k16 f16 MMA's
      16 rows (zero-padded).
    - **Scores.** Q·Kᵀ on the tensor cores over 16-key tiles staged by `cp.async`, double-buffered; online softmax in
      f32 with exp2 and the scale folded in.
    - **Values.** P·V on the tensor cores.
    - **Split and fold.** The span is split across blocks, and the last-arriving block folds. Its epilogue writes the
      int8 words and scales the output projection reads, so `lk_quant` disappears.
    - **Two-word lane.** Under two-word numerics, Q·Kᵀ runs as int8 MMAs in the exact form `lk_attention_i8` already
      uses.
  - [ ] Bar (copy): ≥ the row maximum (vLLM's unified attention, FlashInfer's decode, FlashAttention-2's split-KV
    decode, llama.cpp's flash attention, TensorRT-LLM's XQA) at Gemma (hd 256, window 512), Llama (hd 64) and Qwen3
    (hd 128) shapes at contexts 512, 1024 and 4096 (SETUP-4 rows).
  - [ ] Bar (beyond): ≥ 1.2× vLLM at context 4096, and one launch per layer for attention plus quantize.
  - The SIMT kernel meanwhile (11-01): each launch compiles only the path it takes past a row of one chunk and its run
    width (`lane_compile_lock::an_attention_launch_compiles_only_the_path_it_takes`), no spill under two blocks a
    multiprocessor and the held path 8 bytes under three, where the four paths together spilled 240 and 888. Decode
    0.6–2.9% faster, the batches of 64 2.7–3.6% (`logs/one-ir/ab-paths-1101.log`).

- [ ] **K-05 · The output projection with the residual and both norms in its epilogue** (PARITY-GEMV-NORM-EPILOGUE).
  Today Gemma runs Wo (144 blocks), the post-attention norm (1 block), then add and pre-FFN norm with quantize
  (1 block). Llama runs Wo, then add and norm.
  - [x] Copy:
    - vLLM fuses the residual add into the norm (`fused_add_rms_norm_kernel`), but the norm stays its own launch: a
      norm needs the whole row, and a GEMM's tiles each own part of it.
    - llama.cpp fuses norm, then add (the ledger's `norm_then_add` citation).
  - [x] Design (`lk_gemv_q8_norm`, Q8_0 one-row projections; receipt: `lane_compile_lock::a_projection_feeding_an_add_and_norm_finishes_them_in_its_last_block`):
    - The GEMV's blocks write their rows of y and add to a per-row-set counter. The last-arriving block reads y (d
      floats, in L2) and the residual.
    - Gemma's sandwich: post-norm y with its weight, then add the residual, write the sum, then pre-FFN norm the sum
      and quantize. Llama: add, write, norm and quantize.
    - Both reductions use `lk_rmsnorm`'s 1024-thread tree, so the result is bit-identical (`lk_rmsnorm_row`; receipt:
      `cuda_lane_lock::the_last_block_norm_computes_what_the_norm_launch_computes`, logs/one-ir/live-gpu-1030.log).
  - [x] Beyond: vLLM cannot put a full-row norm into a GEMM epilogue. The last-arriver fold removes the launch, and the
    fold's work overlaps the GEMV's tail instead of starting a new kernel.
  - [x] Bar: Gemma 3 → 1, Llama 2 → 1 launches (receipt: `tests/fixtures/lane_census/`).

- [ ] **K-06 · Gate, up and the activation in one GEMV, quantized out** (have: `glu_pair`, `lk_gemv_glu`).
  Today Gemma's `lk_gemv_glu` runs 216 blocks of 1024 threads and reads 16.9 MB; floor 50 µs a layer.
  - [ ] Copy:
    - vLLM's `act_and_mul_kernel` follows a merged gate/up GEMM: two launches.
    - Triton's MoE matmul fuses the activation into the GEMM epilogue.
  - [ ] Bar: ≥ 92% of bandwidth on Gemma, Llama and Qwen3 shapes (receipt: profile). The bar is the bandwidth; the
    fusion is already ours.

- [x] **K-07 · The down projection with the residual and the next layer's norm** (PARITY-GEMV-NORM-EPILOGUE).
  The same fold as K-05, for the FFN side. Gemma: post-FFN norm, add, and the next layer's pre-attention norm with
  quantize. Llama: add, then the next norm.
  - [x] Bar: Gemma's layer ends without a norm launch (receipt: `tests/fixtures/lane_census/gemma3-1b.txt`).
  - [x] Bar: layer total 14 → 5 launches for Gemma (K-03, K-04, K-05, K-06, K-07), and 11 → 5 for Llama, the
    attention's quantize in its store (receipt: `tests/fixtures/lane_census/gemma3-1b.txt`,
    `tests/fixtures/lane_census/llama3.2-1b.txt`).

- [ ] **K-08 · Projections at ≥ 90% of bandwidth** (OPT-7, TECH-ALIGNMENT-AWARE-VECTORIZATION, TECH-L2-WEIGHT-PREFETCH).
  - [ ] Every `lk_gemv_q8` / `lk_gemv_blk` launch in the six censuses at ≥ 90% of measured bandwidth. Rows a warp and
    blocks per SM come from the tuning database (OPT-7), not constants.
  - [ ] L2 prefetch of the next projection's first tiles during attention (TECH-L2-WEIGHT-PREFETCH), measured on the
    smallest models first.

### 4.4 Decode, the gated-delta layer (Qwen3.5: 18 of 24 layers; Qwen3.8: 48 of 64)

Qwen3.5-0.8B's GDN layer reads 22.98 MB of weights:
- attn_qkv 1024 → 6144, the conv's input channels (q 2048, k 2048, v 2048);
- attn_gate (z) 1024 → 2048;
- alpha and beta 1024 → 16 each;
- out 2048 → 1024;
- the FFN.

Its state is 16 value heads × 128 × 128 f32 = 1 MiB, read and written every token.

- [ ] **K-09 · One launch for every GDN input projection, with conv, SiLU, L2 and the gates in its epilogue**
  (PARITY-GDN-INPUT-EPILOGUE).
  Today: qkv, z, alpha and beta as separate GEMVs, then `conv_silu_l2` (the conv, its SiLU and the q/k L2 norm), then
  `gates`.
  - [ ] Copy:
    - vLLM runs the conv apart, then its packed decode kernel reads q, k and v from the conv output and computes the
      gates inside (`external/competitors/vllm/vllm/third_party/flash_linear_attention/ops/fused_recurrent.py` · `def fused_recurrent_gated_delta_rule_packed_decode(`).
    - vLLM's CUDA kernel stages the state 32 value rows at a time
      (`external/competitors/vllm/csrc/gdn/fused_gdn_decode_kernel.cu` · `gdn_decode_post_conv_mtp_kernel(`).
  - [x] Design (receipt: `lk_gemv_gdn`; `lane_compile_lock::a_gated_delta_layers_input_projections_are_one_launch_with_conv_and_gates`
    / `cuda_lane_lock::the_gdn_input_epilogue_computes_what_its_launches_compute_apart`, logs/fusion-k09/gpu-k09-tests.log;
    gates projected in `f32`, Qwen3.8's, take the launch's last blocks as `lk_gate_proj` does): one GEMV launch over the
    rows [qkv 6144 | z 2048 | alpha 16 | beta 16]. Epilogues, all per row except one:
    - **qkv rows.** Each row is a conv channel. Apply its 4 taps over the ring (3 kept values plus this step's), add
      the bias, apply SiLU, and advance the ring in place (`lk_conv`'s arithmetic).
    - **q and k heads.** The block finishing a head's 128 channels (last arriver, a per-head counter) applies the L2
      norm.
    - **alpha and beta rows.** g = −exp(A)·softplus(a + dt bias) and β = σ(b), per value head (`lk_gates`'
      arithmetic).
    - **z rows.** Stored for K-10.
  - [ ] Beyond: vLLM needs a conv launch, then its fused decode. Here the conv and the gates disappear into the
    projection's epilogue: 5 launches become 1.
  - [ ] Bar: bit-identical to today's launches; one launch; ≥ 85% of bandwidth.

- [ ] **K-10 · The delta update, the gated norm and the quantize in one kernel** (PARITY-GDN-DECODE-WHOLE).
  Today: `lk_delta_step`, then `lk_delta_norm` (gated RMSNorm × SiLU(z), quantized).
  - [ ] Copy vLLM's MTP decode kernel (the anchor above): state chunked 32 value rows at a time through `cp.async`, 256
    threads, two blocks an SM.
  - [ ] Design:
    - A block per (value head, 32 value columns): 64 blocks for Qwen3.5, 192 for Qwen3.8. A slice's update and output
      need only its own columns, so the slices are independent.
    - The state slice is streamed through shared memory by `cp.async`, double-buffered.
    - Per slice: decay by g, the delta update from k, v and β, the output columns o = qᵀS, and the slice written back.
    - The head's last-arriving block applies RMSNorm × SiLU(z) over the head's 128 outputs and quantizes for the out
      projection.
  - [x] The head's last-arriving block of the step norms, gates and quantizes the head: `lk_delta_step` given the
    norm's operands, one kernel where two ran, bit for bit the step and then the norm (receipt:
    `lane_compile_lock::a_stepped_delta_rule_norms_each_head_in_its_last_block` /
    `cuda_lane_lock::the_stepped_delta_rules_last_block_norms_as_the_norm_kernel_does`, logs/one-ir/live-gpu-k18-1030.log).
  - [ ] Floor: 2 MiB of state traffic a layer = 6.2 µs at 336 GB/s (Qwen3.5); 6 MiB = 18.7 µs (Qwen3.8).
  - [ ] Bar: ≤ 1.25× the state floor. 2 launches become 1. Bit-identical to the apart launches.

- [x] **K-11 · The GDN out projection with the residual and the next norm** (PARITY-GEMV-NORM-EPILOGUE).
  - [x] K-05's fold on the out projection (receipt: `tests/fixtures/lane_census/qwen3.5-0.8b.txt`). GDN layer total: today about 11 launches (four projections, conv, gates,
    delta, norm, out, add-norm, GLU, down); target 4 (K-09, K-10, K-11 with K-06, K-07).

- [ ] **K-12 · Half-precision recurrent state** (PARITY-GDN-HALF-STATE).
  - [ ] Copy: vLLM keeps the GDN state in bf16. Our lane keeps f32.
  - [ ] Design:
    - The state stored in bf16 or f16, updated in f32 registers.
    - Written back with round-to-nearest-even, or stochastic rounding (`cvt.rs`, TECH-STOCHASTIC-ROUNDING-STATE) when
      the shim's numerics allow it.
    - Declared as a numerics contract with its bound (OPT-6).
  - [ ] Bar: state traffic halved (Qwen3.8: 302 → 151 MB a token). Each model's precision bar met at this knob point,
    or the knob is refused for it by name.

### 4.5 Decode, the gated-attention layer (Qwen3.5 and Qwen3.8)

- [x] **K-13 · Gated attention: q with gate, k and v in one GEMV; the gate applied in the attention fold**
  (PARITY-QK-NORM-ROPE-GATE-SPLIT, PARITY-QKV-HEAD-EPILOGUE).
  Qwen3.5's attention layer: q 1024 → 4096 (8 heads × 256, query and gate), k and v 1024 → 512 each, 2 KV heads, the
  interleaved rotary sections [11, 11, 10].
  - [x] Copy vLLM's one-launch split, norm, partial rotation and gate copy
    (`external/competitors/vllm/vllm/model_executor/layers/fused_qk_norm_rope.py` · `def fused_qk_rmsnorm_rope_gate(`):
    the split, norms and rotation in the projection's epilogue, the gate read in place with no copy.
  - [x] Design (receipt: `lane_compile_lock::a_gated_attention_gates_its_output_in_its_store` /
    `lane_compile_lock::a_batchs_gated_attentions_gate_their_rows_in_their_store` /
    `fusion_class_lock::a_gated_attentions_output_gate_folds_into_its_attention` /
    `cuda_lane_lock::the_gated_attention_store_computes_what_the_gated_product_computes`, logs/one-ir/live-gpu-k13-1031.log):
    - K-03's per-head epilogue, extended: query heads normed and rotated with the sections, the gate halves left in
      place (`qk_norm_rope_gate_split`, `qkv_head_epilogue`).
    - The attention's store multiplies its output by σ(gate), read in place, then quantizes for the output projection
      (`attn_output_gate`, `lk_attn_store`); a batch's sequences each gate their row by their row of the batch's gate,
      in one attention over them.
    - No gate copy and no separate gate launch.
  - [x] Bar: the attention layer's q/k/v-to-output-projection path in 2 launches (projection with epilogue; attention
    with gate and quantize in its fold) (receipt: `tests/fixtures/lane_census/qwen3.5-0.8b.txt`, each attention layer
    `gemv_rope attention` before its output projection; Qwen3.5-0.8B 165 → 159 decode launches a token).

### 4.6 Decode, latent attention and experts (DeepSeek-V2-Lite)

DeepSeek-V2-Lite's MoE layer reads 57.35 MB (Q4_K_M):
- q 2048 → 3072 (16 heads × 192: nope 128 + rope 64; no query LoRA);
- kv_a 2048 → 576 (latent 512 + rope 64);
- kv_b 512 → 4096 (the key and value un-projections);
- output 2048 → 2048;
- the router 2048 → 64 (f32, 0.5 MB);
- 6 of 64 routed experts and 2 shared (intermediate 1408 and 2816).

The lane's 10-20 decode profile (`--profile`, 6.75 ms a step then):
- expert up 2.26 ms (91% of bandwidth);
- expert down 1.62 ms;
- dense GEMVs 1.28 ms (83%);
- small kernels about 1.45 ms: 82 RMSNorms at 3.1 µs, staged per-head absorption 9.3 µs a layer, the router 7.9,
  per-head GEMV 5.7, rotations and quantizes.

- [ ] **K-14 · One launch for q and the latent, with the latent norm, rotation, cache append and the query's absorption**
  (PARITY-MLA-PROLOGUE).
  - [ ] Copy:
    - vLLM's rotate-and-append kernel (`external/competitors/vllm/csrc/cache_kernels_fused.cu` · `concat_and_cache_mla_rope_fused_kernel(`).
    - vLLM's MLA decode absorbs W_UK into the query before an MQA kernel
      (`external/competitors/vllm/vllm/model_executor/layers/attention/mla_attention.py`).
  - [ ] Design: one GEMV over [q 3072 | kv_a 576]. Last-arriver epilogues:
    - kv_a: RMSNorm over the 512 latent values, rotation of the 64 rope values, the latent cache row appended
      (quantized under K-30).
    - Each query head: the rope part rotated; the nope part (128) multiplied by that head's W_UK slice (128 × 512) to
      give the absorbed 512-wide query. The last arriver for a head reads 64 K weights from L2/HBM.
  - [ ] Beyond: the absorption leaves its own launch (`lk_head_axpy_staged`, 9.3 µs a layer today).
  - [ ] Bar: the latent layer's prologue in 1 launch; ≥ 80% of the combined bytes' bandwidth.

- [ ] **K-15 · Latent attention with the value un-absorption and quantize in its fold** (PARITY-MLA-UNABSORB-IN-FOLD).
  Today `lk_attention_shared` (all 16 heads of the one latent group in a block, each cached row read once) and
  `lk_attention_fold`, then the per-head value product (× W_UV), then a quantize.
  - [ ] Copy:
    - FlashMLA's tile scheduler (sm_90).
    - CUTLASS MLA (`external/competitors/vllm/csrc/attention/mla/cutlass_sm100_mla/kernel/sm100_fmha_mla_tma_warpspecialized.hpp` · `Sm100FmhaMlaKernelTmaWarpspecialized`).
    - On sm_86, vLLM's Triton MLA decode: split over the span, grouped heads with the rope part apart
      (`external/competitors/vllm/vllm/v1/attention/ops/triton_decode_attention.py` · `def decode_attention_fwd(`).
  - [ ] Design: the fold's last block, per head, multiplies the 512-wide context by W_UV's head slice (512 × 128). It
    writes the 2048-wide output row, then quantizes it for the output projection.
  - [ ] Bar: attention, un-absorption and quantize in 2 launches (attention and its fold kernel). Decode attention
    ≤ 15.8 µs a layer (the 10-20 figure, after a 512-token prompt), and ≤ 0.55× today's at context 4096 with K-30.

- [ ] **K-16 · The output projection with the residual, the FFN norm and the router** (PARITY-GEMV-NORM-EPILOGUE).
  - [ ] K-05's fold, plus the router launched on a second stream as soon as the norm's quantized row is written
    (the branched streams the lane already has).
  - [ ] Copy:
    - vLLM's fused top-k (`external/competitors/vllm/csrc/moe/topk_softmax_kernels.cu` · `topkGating(`).
    - vLLM's small-M fp32 router GEMM with token groups (`external/competitors/vllm/csrc/fp32_router_gemm.cu` · `fp32_router_gemm_kernel(`).
  - [ ] Bar: router ≤ 5 µs (today 7.4 to 7.9), overlapped with the shared expert.

- [ ] **K-17 · The experts' gate and up at the bandwidth floor** (have: `lk_moe_up` with the shared expert folded).
  - [ ] Copy:
    - vLLM's Triton `fused_moe_kernel`: rows through sorted ids, the router weight in the epilogue
      (`external/competitors/vllm/vllm/model_executor/layers/fused_moe/fused_moe.py` · `def fused_moe_kernel(`).
    - Marlin's MoE GEMM.
  - [ ] Design:
    - Every routed expert's rows and the shared expert's rows in one launch.
    - Blocks balanced by bytes, not by expert: the shared expert's rows are twice a routed expert's, the imbalance the
      10-20 profile measured.
    - Interleaved row reads, decoders in registers (`lane_compile_lock::every_row_reading_kernel_keeps_its_decode_in_registers`).
  - [ ] Bar: ≥ 95% of bandwidth (today 91%).

- [ ] **K-18 · The experts' down, combine, shared expert, residual and next norm in one launch**
  (PARITY-MOE-DEFERRED-FINALIZE, PARITY-GEMV-NORM-EPILOGUE).
  Today `lk_moe_down` combines in its blocks, then the add-and-norm runs as its own launch.
  - [x] Copy vLLM's deferred finalize (`external/competitors/vllm/vllm/model_executor/layers/fused_moe/modular_kernel.py` · `return self.prepare_finalize.supports_deferred_moe_finalize()`).
  - [x] Design (receipt: `lane_compile_lock::a_decode_mixture_adds_and_norms_in_its_last_down_block` /
    `cuda_lane_lock::the_mixtures_last_step_norms_as_the_norm_launch_does`, logs/one-ir/live-gpu-k18-1030.log):
    - The decode step's down blocks sum their outputs' kept experts in fixed k order and add the shared expert's row
      (`lk_moe_down`'s block, unchanged); the last-arriving block reads the row back through L2, adds the residual,
      writes the sum, and applies the next layer's norm and quantize as the norm's own block does (`lk_moe_down_norm`).
    - A chunk's or a batch's combine takes a block of the norm's threads a row: the weighed partials in k order, the
      shared expert's product where it is summed first, the residual, the norm (`lk_moe_combine_norm`).
  - [ ] Bar: the MoE layer from router to next norm in 3 launches (router, up, down); ≥ 92% of bandwidth on the down
    rows (today about 84%).
  - The fold as built played the norm's 1024 threads on a down block of 32·(k + shared warps): DeepSeek-V2-Lite decoded
    at 142.2 tokens a second folded and 146.3 apart, gpt-oss-20b at 106.1 and 111.0; with the norm's tail cut, 146.7, with
    the fence cut, 142.6 (11-01, `logs/one-ir/holds2-1101.log`, `fold-exp-1101.log`). Since 11-02 a decode mixture folds
    only where its down block is at least the norm's width (`every_folded_norm_runs_on_a_block_of_its_width`), as does a
    block-format projection (K-05, K-07); DeepSeek-V2-Lite decodes at 152.9 and gpt-oss-20b at 110.9 with them apart.
    Next: a fold whose norm runs at the norm's width, so the Bar's 3 launches hold: the down projection's blocks
    regrouped to a block of the norm's threads, or the add and norm in the next projection's prologue (each of its blocks
    norming the row from L2, no count and no one-block tail).

### 4.7 Decode, gpt-oss (sinks, alternating windows, MXFP4 experts, biases)

gpt-oss-20b's layer reads 81.64 MB:
- attention q 2880 → 4096 (64 heads × 64), k and v 2880 → 512 (8 KV heads), output 4096 → 2880, with biases;
- learned sinks;
- a 128-token window on alternate layers;
- 4 of 32 MXFP4 experts (intermediate 2880, clamped SwiGLU, biases).

On 10-21 the experts ran at 187 µs a layer (285 GB/s, 85% of bandwidth) and the head at 339 GB/s.

- [ ] **K-19 · gpt-oss attention: q/k/v with biases and the per-head epilogue; sinks and windows in the packed decode
  kernel** (PARITY-QKV-HEAD-EPILOGUE, PARITY-ATTN-GQA-ROW-PACKING).
  - [ ] Copy:
    - vLLM's unified attention handles sinks and the window in one kernel.
    - Flint's decode adds the sink to the softmax denominator and merges partials in the last-arriving CTA
      (`external/competitors/vllm/vllm/models/flint/ops/decode_attention.py` · `Split-KV paged decode attention for Flint`).
  - [ ] Design:
    - K-03's epilogue with the biases added before rotation.
    - K-04's group packing with 8 query heads per KV head, padded to the MMA's 16 rows.
    - Window layers skip tiles outside the window at launch time; the sink enters the fold's denominator.
  - [ ] Bar: attention ≤ 0.6× today's time per layer at context 1024.

- [ ] **K-20 · MXFP4 experts at ≥ 92% of bandwidth, the clamped SwiGLU and biases in the epilogue** (PARITY-MOE-GATHER-IN-OPERAND,
  TECH-LOP3-PRMT-DEQUANT).
  - [ ] Copy:
    - Triton's MoE matmul for gpt-oss: MXFP4 dequantized in the K loop, SwiGLU with its clamp and the router weight in
      the epilogue (`external/competitors/triton/python/triton_kernels/triton_kernels/matmul.py` · `def matmul(a, b, bias,`).
    - vLLM's Marlin MXFP4 path.
  - [ ] Design: the MXFP4 decode as byte permutes (`lk_table16`, the 10-21 fix) plus LOP3 for the E8M0 scale apply;
    up/gate paired; down with K-18's fold.
    - Done 11-06: MXFP4 rows regrouped at upload (PARITY-WEIGHT-FORMATS), so every reader takes a sub-block's nibbles
      as one aligned 16-byte word and its exponent as one byte; the GEMV's `lk_sub4` read five words and a byte.
    - MXFP4's nibble order is the int8 `m16n8k32` B fragment: bytes 4·t4 … 4·t4+3 of a sub-block hold its weights
      4·t4 … in their low nibbles (b0) and 16+4·t4 … in their high nibbles (b1), so one 32-bit read through
      `lk_mxfp4_word` is a lane's whole fragment. Q4_0 and IQ4_NL share the order.
    - An E8M0 scale is a power of two, so the fold can be two FMAs, `fma(fma(T, d, −1.5·2²³·d), s, acc)`, rounding as
      `fma(s·d, dot, acc)` does: measured 179.7 → 175.3 ms on gpt-oss's 512-row prefill, but `1.5·2²³·d` overflows
      past `d = 2¹⁰⁴`, so it waits on a form that holds over E8M0's whole range.
  - [ ] Bar: ≥ 92% of bandwidth (today 85%).

### 4.8 Decode, the head and the token

- [ ] **K-21 · The head GEMV with the soft cap, sampling candidates and the arg-max in its epilogue** (PARITY-SOFTCAP,
  PARITY-SAMPLING).
  The heads read 172 MB (DeepSeek-V2-Lite) to 615 MB (gpt-oss) a token, 10% to 34% of each model's bytes. The head
  is already at 98% of bandwidth on DeepSeek (530 µs on 10-17).
  - [ ] Copy:
    - vLLM's soft cap splits the vocabulary row across blocks
      (`external/competitors/vllm/csrc/muse_spark_mul_tanh_soft_cap_kernel.cu` · `muse_spark_mul_tanh_soft_cap_kernel(`).
    - vLLM samples after writing the full logits row
      (`external/competitors/vllm/vllm/v1/sample/ops/topk_topp_triton.py` · `def apply_top_k_top_p_triton(`).
  - [ ] Design: each head block keeps, for its rows, the running maximum and sum of exponentials, plus its top-k
    candidates. The last-arriving block merges them into the arg-max (greedy) or the sampled token (Gumbel on the
    counter RNG of (seed, position)). The full logits row is written only when logprobs are asked for.
  - [ ] Beyond:
    - Greedy decoding drops the final soft cap by a proven monotone rewrite.
    - Sampling never writes and re-reads 1 MB of logits: about 6 µs saved a token at a 248,320 vocabulary.
  - [ ] Bar: the head at ≥ 97% of bandwidth on every model; the token chosen inside the head launch; sampling ≤ 3% of a
    step.
  - Greedy, first cut (11-01, not landed): a shared rule `head_argmax` folding the arg-max into the head, and a lane
    fold in which each block keeps the first maximum of the rows it writes and the last to arrive picks. It was bit for
    bit the launches apart on every tiny architecture and block-format head, and slower: a warp a row makes a 262,144-row
    head about 32K blocks, and each one counting in on one counter cost 1.04 ms against 0.91 apart (Gemma-3-1B, 3060).
    The next cut: a bounded grid of a few blocks a multiprocessor, each striding over the rows and keeping its first
    maximum, the pick tie-broken by index. The ceiling at that vocabulary is the arg-max's own launch, about 10 µs a
    token.

### 4.9 Decode, the whole step

- [ ] **K-22 · Launch latency to zero: graph, overlap, persistence** (PARITY-PERSISTENT-LAYER-MEGAKERNEL,
  GRAPH-CROSS-KERNEL-OVERLAP).
  After K-02 to K-21 a dense layer is about 5 launches. What remains is each launch's head and tail inside the graph.
  - [ ] Copy:
    - vLLM's AMD "mono" decode: a DeepSeek-V4 layer as two persistent launches with an in-kernel all-reduce.
    - Kimi-K3's specialized decode, with an L2 weight prefetch.
    - Both are hand-written for one model.
  - [ ] Step 1: measure each remaining launch's gap in the graph (`nsys` with `--cuda-graph-trace=node`; the recipe
    is in the 10-17 lane notes).
  - [ ] Step 2: PDL is not available on sm_86. Overlap independent launches on the branched streams instead (the
    router beside the shared expert, the next layer's weight prefetch beside attention).
  - [ ] Step 3, a persistent layer kernel:
    - one cooperative launch per layer, with grid-wide syncs between the five stages;
    - the next stage's first weight tiles prefetched before each sync;
    - small intermediates kept in shared memory where the stage's blocks own them.
  - [ ] Beyond: compiled from the IR for every model, not hand-written for one.
  - [ ] Bar: Llama-3.2-1B and Qwen3.5-0.8B at ≥ 96% of their decode floors.

### 4.10 Prefill, the GEMMs

- [ ] **K-23 · The dense int8 GEMM past 55 TOPS** (PARITY-INT8-GEMM-PEAK).
  Today `lk_gemm_blk<8,…>` streams Q8_0 tiles by `cp.async` with ldmatrix fragments. One-word runs at about 40 TOPS;
  two-word at 24 (BM 64). llama.cpp's mmq runs at about 36.
  The 10-21 ablation: no fold 979 µs, no MMA 1014, full 1284. The inner loop is issue-bound on the per-32-element scale
  fold, about 12 instructions per m16n8k32.
  - Done 11-09: the sums read by the float conversion where it issues on the float pipe (I2FP, sm_86 on) and the
    kernel measures faster for it (`lk_dot_f<I>`: the streamed Q8_0 path under both numerics, the block path and the
    batch GEMV under one word), begun at zero, so no `mma` waits on a start value moved into its accumulators; the
    block path's stages lean (offsets once a launch, no branch between a dense tile's sub-blocks where the format has
    no offsets). Bits unchanged (`cuda_lane_lock::a_block_gemm_reads_its_sums_the_same_on_every_architecture`,
    `a_block_gemm_past_k_computes_what_its_zero_padded_k_computes`). Qwen3-8B 512 rows, medians of seven alternations:
    Q8_0 paired gate+up 2.64 → 2.42 ms (39.0 → 42.6 TOPS), Q8_0 down 1.31 → 1.27, two-word Q8_0 down 2.11 → 2.01; Q4_K
    paired 3.28 → 3.01, Q4_K down 1.61 → 1.47, Q6_K down 1.97 → 1.89, output 554 → 520 µs. Against MMQ at the same
    shapes (`test-backend-ops perf`, its quantize included): gate 1.45–1.52 ms each, Q4_K down 1.58, Q6_K down 1.73,
    output 0.55. Per stage the plain Q4_K loop is 28.3 instructions an `mma` (MMQ's 23.1; the float epilogue is 16 of
    both); the multiply alone, no staging, runs 51 TOPS. Still behind MMQ: Q6_K, whose block scale MMQ applies once a
    tile (four operations an output against our five) — a new rounding order, an ask; and the last wave of the 32- and
    128-tile GEMMs at 512 rows (the value projection's cut 216 µs, its whole tiles alone 139; MMQ's stream-K 152) —
    stream-K over the tail, an ask.
  - [ ] Copy vLLM's W8A8 path: CUTLASS int8 GEMM with scales broadcast in the epilogue
    (`external/competitors/vllm/csrc/cutlass_extensions/epilogue/scaled_mm_epilogues_c3x.hpp` · `struct ScaledEpilogueBiasAzpToken`).
    It has no per-block fold, since its weights are per-channel.
  - [ ] Copy Triton's persistent matmul: pid swizzle, epilogue subtiling, warp specialization
    (`external/competitors/triton/python/tutorials/09-persistent-matmul.py` · `EPILOGUE_SUBTILE = nargs.get(`).
  - [ ] Design for the exact Q8_0 lane:
    - **Producer and consumers.** One warp issues `cp.async` for weights, scales and activations three stages ahead.
      Four consumer warps run MMAs, so the fold overlaps the next tile's loads.
    - **Swizzle.** Shared memory is swizzled by the F2 layout algebra (TRITON-GENERIC-SWIZZLE), so ldmatrix reads are
      conflict-free.
    - **Measure the fold.** Microbenchmark the fold alone (no MMA) at the tile's shape. That gives the ceiling the
      exact point can reach.
    - **Shorter fold.** Remove every instruction past one convert, one scale multiply and one FMA per accumulator per
      32-element block, read off the SASS.
  - [ ] Design for a new knob point, W8A8 per channel:
    - The weights requantized to per-channel int8 at load, with the bound declared (OPT-6) and held to its bar.
    - The fold leaves the inner loop entirely; this point runs at CUTLASS-class speed.
    - The exact Q8_0 point keeps its fold.
  - [ ] Bar (copy): ≥ the row maximum (vLLM's W8A8 CUTLASS GEMM, cuBLASLt int8, llama.cpp's `mul_mat_q`, Triton's
    matmul) at our shapes on the per-channel point (SETUP-4).
  - [ ] Bar (beyond): ≥ 55 TOPS one-word on the exact point; ≥ 40 two-word.

- [ ] **K-24 · Grouped expert GEMMs reading rows through the sort, writing back through it** (PARITY-MOE-GATHER-IN-OPERAND).
  Today `lk_moe_gather` copies each expert's rows, the paired gate-up GEMM (G=3) runs with the GLU and quantize epilogue,
  then the down GEMM (G=2) runs weighted. On 10-19: paired 61.6 ms, down 28.3 ms of DeepSeek's 171 ms pp512. On 11-05
  gpt-oss-20b's paired GEMM took 160 ms and its down 53 ms of a 257 ms pp512 (nsys, logs/fusion-k09/window/).
  - Done 11-06: the grouped GEMM takes two blocks a multiprocessor on 64-row one-word tiles (`lk_gemm_blk_blocks`,
    `lane_compile_lock::every_one_word_expert_gemm_takes_two_blocks_a_multiprocessor`): one 256-thread block of 133
    registers left two warps a scheduler for the fold's chain. gpt-oss pp512 241.6 → 187.5 ms, DeepSeek-V2-Lite
    197.6 → 174.8 ms, tokens identical (logs/fusion-k09/window2/ab.log). Q2_K, Q3_K and Q5_K stay at one block: their
    halves' scales and offsets beside the multiply spill at 128 registers.
  - [ ] Next: the weights' raw words staged by `cp.async` into shared memory, a stage decoded into each lane's B
    fragment in registers (the MXFP4 fragment above), so no raw words or decoded tile hold registers across the
    multiply: every format at two blocks, Q2_K, Q3_K and Q5_K included.
  - [ ] Copy:
    - Marlin's MoE gathers in its A loads (`external/competitors/vllm/csrc/moe/marlin_moe_wna16/marlin_template.h` · `reinterpret_cast<const int4*>(sorted_token_ids_ptr)`).
    - Triton's MoE matmul gathers on load and scatters on store.
  - [ ] Design:
    - The A-tile loader reads row ids from the sorted list and issues `cp.async` per row.
    - The down GEMM's epilogue scatters each row back, multiplied by its routing weight, into per-(token, k) slots.
    - K-18's combine finishes it.
  - [ ] Bar: the gather launch and its copy gone; DeepSeek pp512 ≥ 1.15× today.

- [ ] **K-25 · Merged projections everywhere they share an input** (have for wide chunks: `merged_projections`; extend).
  - [ ] Copy: vLLM merges q/k/v and gate/up as single GEMMs.
  - [ ] Design: the GDN layer's four input projections (K-09's rows), and the latent layer's q and kv_a (K-14's rows),
    as one GEMM in parts on wide chunks, with their epilogues.
  - [ ] Bar: one GEMM launch per shared input in every layer kind.

### 4.11 Prefill, attention

- [ ] **K-26 · Prefill attention at ≥ 60% of f16 tensor peak** (PARITY-PREFILL-ATTENTION-PEAK).
  Today `lk_attention_mma` (f16 m16n8k16, Q in registers, ldmatrix and `.trans`, P from the S fragments) runs at
  9.9 TFLOPS on Qwen3-8B (217 µs a layer at 512 rows). The two-word `lk_attention_i8` runs 1.45 to 2.06× faster than
  the f32 flash kernel.
  - [ ] Copy:
    - Triton's tutorial attention: exp2 with the scale folded, causal split into three stages, accumulator halves
      (`external/competitors/triton/python/tutorials/06-fused-attention.py` · `def _attn_fwd_inner`).
    - vLLM's FlashAttention-2 on sm_86.
  - [ ] Design:
    - Static causal masks decide full, partial and empty tiles at compile time (PARITY-ATTENTION-VARIANTS-INLINE).
    - K and V double-buffered by `cp.async`.
    - Two consumer warpgroups ping-ponging the softmax and the MMAs.
    - The output quantized in the epilogue for the output projection.
  - [ ] Bar: ≥ 15 TFLOPS f16 at Qwen3-8B, Llama and Gemma shapes (spec peak about 25.5 with f32 accumulation); ≥ 1.1×
    vLLM's FlashAttention-2 at the same shapes.

### 4.12 Prefill, gated-delta chunks

- [ ] **K-27 · The chunked delta rule on the tensor cores** (PARITY-GDN-CHUNK-TENSOR-CORES).
  Today `lk_delta_prep` (T·diag(β), the blocked 16×16 inverse, decayed transposes) and `lk_delta_state` (16 memory rows
  a block, 2 warps). On 10-20 Qwen3.5 pp512 took prep 3.3 ms and state 4.7 ms. llama.cpp's `gdn_single` takes 5.77 ms
  with bf16x3 hi/lo MMAs.
  - [ ] Copy:
    - vLLM's FLA pipeline: chunk size 64; β·KKᵀ with decay; a 16×16 forward-substitution solve with block merges in
      ieee, tf32 or tf32x3; WY recompute; inter-chunk state in registers
      (`external/competitors/vllm/vllm/third_party/flash_linear_attention/ops/chunk.py` · `def chunk_gated_delta_rule_fwd(`).
    - llama.cpp's bf16x3 MMAs.
  - [ ] Design:
    - Every chunk product (KKᵀ, the WY recompute, K·Sᵀ, Q·Sᵀ, P·Δ, the state update) on the tensor cores in split
      precision: three bf16 products for f32-faithful sums (TECH-SPLIT-PRECISION-MMA), or two int8 words under the
      two-word lane.
    - Prep and state fused per chunk: the triangular solve stays in shared memory and feeds the state update directly.
  - [ ] Bar: ≤ 0.6× llama.cpp's time on Qwen3.5 pp512 (≤ 3.5 ms for prep and state together); within each knob
    point's bound.

### 4.13 Prefill, row work and the last layer

- [ ] **K-28 · Row norms, residuals and quantizes in GEMM epilogues** (PARITY-PREFILL-NORM-EPILOGUES).
  A wide chunk's RMSNorms run as separate launches today, a block per row. llama.cpp spends 5.1 ms of Qwen3.5's
  pp512 in RMSNorm (nsys, 10-20).
  - [ ] Design:
    - The output projection's and down projection's tiles each write a partial sum of squares per row with their
      tile.
    - The last tile of each row (a per-row counter) finishes the norm: residual add, weight, quantize, written in the
      next GEMM's activation format.
    - The same fold handles Gemma's sandwich pairs.
  - [ ] Bar: no standalone norm launch in a wide chunk; pp512 up on every model (receipt: scoreboard).

- [ ] **K-29 · The last layer computes only what the head and the caches need** (PARITY-LAST-LAYER-ROWS).
  The prefill head already reads the last row only. But the last layer still runs attention output, the output
  projection and the FFN for all 512 rows, when only the keys and values are needed for the earlier rows.
  - [ ] Copy llama.cpp, which prunes the last layer to its output rows
    (`external/competitors/llama.cpp/src/llama-graph.cpp` · `ggml_tensor * llm_graph_context::build_inp_out_ids() const {`).
  - [ ] Design:
    - Row-level liveness in the shared optimizer: a row of the last layer that feeds no output and no state is dead.
    - The rows it removes shrink the last layer's attention queries, output projection and FFN to one row.
    - The cache writes stay.
  - [ ] Bar: pp512 time down by the last layer's share (about 1/16 for Llama-3.2-1B, 1/36 for Qwen3-8B, less the
    attention work kept).
  - Done 11-09, its first part: a prompt's chunk before its last computes no output (`emit::output_only`): a reverse
    liveness from the tensors the step carries — the caches, and a state whose first touch is not a whole write
    without a read (`emit::carried`) — drops the launches whose writes nothing carried reads, the head and the
    arg-max among them (`lk_advance` moves the position and the run instead), each form graphed lean beside whole;
    `--whole-chunks` runs every chunk whole. Bits unchanged
    (`cuda_lane_lock::a_chunk_before_the_prompts_last_computes_what_the_whole_chunk_computes`,
    `lane_compile_lock::a_chunks_output_only_launches_are_what_liveness_from_its_carried_tensors_leaves`). With K-23,
    the P4K cells over the `f32` cache (window73 → window83) and the binary16 one (window83): Qwen3-8B Q4_K_M 2,376 →
    2,174 and 2,063 ms, Gemma-3-1B BF16 427.7 → 392.6 and 390.4, Qwen3.5-0.8B BF16 414.9 → 392.4 and 381.3, Gemma-2-2B
    Q8_0 751.4 → 694.7 and 640.2. Open: the final chunk's last layer on its output row (an attention row offset).

### 4.14 Numerics knobs that move bytes

- [ ] **K-30 · Quantized KV caches read straight into the attention MMAs** (PARITY-KV-CACHE-QUANT).
  An f32 cache costs 2 × KV heads × head dim × 4 bytes per position per layer, all of it read every decode step:
  - Llama: 4 KB, so 32 MB a layer at context 8192, half the layer's 64.6 MB of weights;
  - Gemma-3: 2 KB;
  - DeepSeek's latent: 576 × 4 = 2.3 KB.
  - [ ] Copy:
    - vLLM's fp8 caches with per-head scales (`external/competitors/vllm/csrc/cache_kernels.cu` · `reshape_and_cache_flash_kernel(`).
    - llama.cpp's f16 and q8_0 caches.
  - [ ] Design:
    - Schemes f16, int8 per head, and Hadamard int8 (the D5005 engine's form).
    - Written by K-03's epilogue and read by K-04's MMAs. The int8 schemes feed int8 MMAs directly under the
      two-word exact form.
  - [ ] Bar: decode attention at context 8192 ≤ 0.55× its f32-cache time; every model's long-prompt bar met per scheme.

- [ ] **K-31 · Half-precision GDN state** (PARITY-GDN-HALF-STATE). K-12's knob, measured on Qwen3.8's 302 MB of state
  traffic a token.

- [ ] **K-32 · The per-channel W8A8 knob point** (PARITY-INT8-GEMM-PEAK). K-23's second design, with its own bar and
  its own row on the scoreboard.

### 4.15 The D5005 engine (Stratix 10 SX 2800, four channels of DDR4-2400)

The same op chains, on a weight-streaming spatial engine. Its model is `crates/logicaffeine_compile/src/spatial/decode_engine/`;
its ledgers are under `benchmarks/silicon/d5005/`. The 27B decode step has run token-exact on the card.

**The physics.**
- Four channels of DDR4-2400 give 76.8 GB/s peak (`benchmarks/silicon/d5005/predicted.json`, card facts with
  citations).
- On the model, sequential weight reads reach 95.15% of peak (`benchmarks/silicon/d5005/ddr4_efficiency.json`).
- Qwen3.8-27B at Q4_K_M streams 16.7 GB a step, so one stream is memory-bound at 4.3 tok/s, and every extra stream in
  a batch reuses the same weight read.
- The twin's fastest point: 212.3 tok/s aggregate at batch 104 and position 2047, against a roofline of 230.4. Its
  numerics are one int8 word per block-scaled activation, f16 recurrent state, a Hadamard int8 cache, attention on
  the array, and a flush every 16 tokens (`predicted.json`). Memory binds 83.5% of that step.

- [ ] **K-33 · Weight streams at the DDR floor on the card** (MIGRATE-4, BEYOND-EXACT-MODULO-SCHEDULER).
  The card's receipts streamed 47.2 and 48.1 GB/s at 164 and 167 MHz (unrecorded images, EMIF timing short by 0.4 to
  0.65 ns), against 73 GB/s on the model.
  - [ ] Design: the stream, the matrix units and the vector unit are one modulo-scheduled pipeline. The exact
    scheduler sizes every queue between them, so the stream never waits on compute.
  - [ ] Bar: ≥ 90% of the model's 73 GB/s on the card, recorded as a receipt.
- [ ] **K-34 · The compute clock reaches the stream clock** (BEYOND-EXACT-MODULO-SCHEDULER, D5005-FUSED-COMMANDS).
  The reference engine's logic is 706 levels deep in the vector unit, 371 in int8 attention and 293 in the gated-delta
  row. So it closes only in the low MHz, behind a divided clock. A divided clock rode general routing in one failed
  build, and a later hybrid build failed routing on its channel feeds.
  - [ ] Design: retiming bounded by the maximum cycle ratio of each unit's register graph (`max_cycle_ratio`, certified):
    pipeline each unit to the stream clock, and prove the bound the retiming reaches.
  - [ ] Bar: the engine runs at the stream clock (≥ 160 MHz) with no divided clock.
- [ ] **K-35 · The engine fits with room** (D5005-FUSED-COMMANDS).
  Unit synthesis put the 27B engine at about 630k ALMs against a partial-reconfiguration region of about 560k. Shared
  function units brought it to about 410k, and the engine alone fit the part at 49% of ALMs.
  - [ ] Bar: every build below 65% of the region's ALMs. Each fused command (below) is priced in ALMs before it lands.
- [ ] **K-36 · The step makes fewer passes: what the engine runs apart today becomes commands** (D5005-FUSED-COMMANDS,
  MIGRATE-4).
  Today the engine merges q/k/v, the convs, the two gates, q with its gate, the sigmoid gate, the cache append and the
  arg-max. It runs these apart:
  - the residual add and the norm;
  - the FFN's gate, up and SwiGLU;
  - the L2 head norms;
  - the head norms with the rotation.
  - [ ] Design: one command each for add and norm, GLU with its activation, and head norm with rotation. Each reads the
    fused symbol from the shared table, so the engine fuses what every other chip fuses.
  - [ ] Bar: the vector unit's share of the step falls by the passes removed (receipt: the twin's `bound_share`).
- [ ] **K-37 · Aggregate decode at and past the twin** (MIGRATE-4, SERVE-SPEC-DECODE).
  - [ ] Bar (copy of our own model): 212 tok/s aggregate on the card, at the twin's fastest point.
  - [ ] Bar (beyond): MTP drafting on the engine (Qwen3.8 ships an MTP head) multiplies tokens per weight pass. Past
    the twin's 230 tok/s roofline on the card, toward the 650 to 800 tok/s ceiling the card's physics allows.
- [ ] **K-38 · The engine's numerics, shared** (PARITY-KV-CACHE-QUANT, PARITY-GDN-HALF-STATE).
  On Qwen3.5-0.8B, the Hadamard int8 cache's top-logit gap is 0.0276 against the f32 golden (llama.cpp ships 0.0853).
  f16 state costs about 0.001.
  - [ ] The Hadamard int8 cache and f16 state are shared numerics symbols, linked on the GPU and the engine alike, so
    the 3060 inherits the engine's better point.

### 4.16 The spatial chips: Tenstorrent, SambaNova, CUDA persistent, the fabric

These lower through the spatial pipeline (`crates/logicaffeine_compile/src/spatial/`). Their decisions are checked
against measured ledgers (`benchmarks/silicon/tenstorrent_gemm.json`) or modeled ones that say so
(`benchmarks/silicon/qwen38_sn50.json` carries `no_speed_claim`).

- [ ] **K-39 · Tensix GEMM ahead of ttnn, kept ahead** (MIGRATE-3).
  - Wormhole: the compiler-emitted 640³ GEMM ran 6,063 cycles against ttnn's production op at 6,542 in the same
    configuration (HiFi4, fp32 accumulation, L1 accumulate), PCC 1.0, every kernel and the host generated.
  - Blackhole, at equal retired arithmetic: 10.2× fewer NOC transactions, 8.2× fewer NOC bytes and 2.07× fewer Tensix
    instructions than the tt-metal example.
  - [ ] Bar: every Tensix GEMM shape the scoreboard models need at or past ttnn, on Wormhole and Blackhole.
- [ ] **K-40 · Fill the mesh: non-uniform tiling** (SPATIAL-NONUNIFORM-TILING).
  A uniform grid uses only grid sizes that divide the tile count. Blackhole averages 57% of its 130 cores, and none of
  256 shapes fills it. Non-uniform decomposition reaches 86.8%: 1.52× the cores. The IR already carries what it needs.
  - [ ] Bar: ≥ 85% of the cores on Blackhole and Wormhole across the scoreboard models' GEMM shapes.
- [ ] **K-41 · Tensix runs the decode step** (MIGRATE-3, ONE-PROGRAM-EVERY-BACKEND).
  Today Tensix lowers single tensor ops.
  - [ ] Bar: the tiny decode step of every architecture on the simulator, equal to the VM, PCC 1.0. The real-width
    layers fit-checked per chip.
- [ ] **K-42 · SambaNova: every architecture's deployment, certified** (MIGRATE-3, ONE-PROGRAM-RETIRE-SECOND-WRITERS).
  Today the 27B deployment (16-chip tensor parallelism, bf16, weights and state sized per chip) certifies, and the tiny
  step equals the VM in Spatial's simulator. Relative drift against the VM is about 1.1% on the SN50 target and 0.3% on
  Wormhole, from accumulation width.
  - [ ] Bar: every scoreboard architecture's deployment certifies with its drift inside its declared bound. Speed
    claims wait for the vendor SDK and hardware, as the ledger says.
- [ ] **K-43 · CUDA persistent: the whole step as one placed program** (BEYOND-SPATIAL-ON-GPU).
  The persistent GEMM reached 5,744 GOPS once its planner priced the allocator's layout.
  - [ ] Bar: K-22's ≥ 96% of the decode floor on the 3060, from the spatial pipeline.
- [ ] **K-44 · The fabric: the one program's tiny model on the board** (MIGRATE-3, ONE-PROGRAM-RETIRE-SECOND-WRITERS).
  The WPE engine generated a tiny transformer's tokens on the fabric from a Python-written program. The sequential
  engine took 124,559 cycles a token. An emitted 8-wide int8 array maps to 80 DSP48E1 slices.
  - [ ] Design: `model::author`'s tiny cuts through the spatial pipeline. Matrix products go on a P-wide array sized
    to the board's DSPs; everything else stays on the engine.
  - [ ] Bar: cycles a token ≤ 1.2 × ⌈MACs / P⌉ plus the engine's non-matrix cycles, equal to the VM.

### 4.17 The CPU (the i9-14900K: AVX2 and AVX-VNNI)

- [ ] **K-45 · Decode at the memory floor** (PARITY-CPU-SHIM).
  DDR4-2133 dual channel is 34.1 GB/s peak, so Llama-3.2-1B at Q8_0 (1.313 GB a token) has a 38.5 ms floor (26 tok/s).
  - [ ] Copy: llama.cpp's int8 dot products and repacked GEMV (PARITY-CPU-SHIM's anchors). Bar: ≥ llama.cpp's CPU
    backend on one boot.
  - [ ] Beyond: the norm, the quantize and the GEMV in one pass over each row; ≥ 90% of measured bandwidth.
- [ ] **K-46 · Prefill on the VNNI units** (PARITY-CPU-SHIM, BEYOND-SUPEROPTIMIZER).
  - [ ] Design: int8 GEMM tiles sized to the performance cores' caches, on both core types with recorded placement.
    The dequantization and scale folds come from the superoptimizer's proven CPU sequences.
  - [ ] Bar: ≥ llama.cpp's CPU prefill on one boot, and every CPU cell of section 2.4 won.

---

## 5. The architecture

### 5.1 Meaning and operation

A LOGOS program says what is computed, and a chip decides how to compute it. The book keeps the two on opposite sides of
one line:

```
LOGOS program (model::author)                           meaning: WHAT is computed
  front end (recognize_stages::walk)
    THE ONE IR (tensor_graph)                           one meaning, one evaluator equal to the VM
      THE SHARED OPTIMIZER (the rule table)             rewrites that preserve meaning; each fused
        fused symbols carry the ledger's class ids      symbol is defined by its parts
------------------------------- the shim line --------------------------------
      each chip's SHIM: "here's what I allow"           operation: HOW a chip runs it
        symbols it runs whole, numerics, layouts,
        limits, cost
      each chip's LINK: symbol -> primitive             CUDA kernels, Tensix kernels, RDU sections,
        or its parts; or refused by name                fabric units, D5005 commands
      each chip's OWN PATH: schedules                   tiles, split-K, streams, arena, placement
```

Today, four representations sit below the front end, and each derives its own fusions:
- **The CUDA lane:** builds `StepGraph` (`crates/logicaffeine_compile/src/codegen_cuda/lane/graph.rs`) and rewrites CUDA
  launches with a 16-rule table (`RULE_TABLE`, `crates/logicaffeine_compile/src/codegen_cuda/lane/plan.rs`).
- **The older CUDA forward:** keeps a `Stage` list with its own elementwise, add-norm, quantize and MoE fusions
  (`crates/logicaffeine_compile/src/codegen_cuda/qwen_emit.rs`).
- **The spatial pipeline:** converts to its own `TensorOp`, then `ComputeOp::Generic`, and splices them in `fuse_generic`,
  `forward` and `fuse_readers` (`crates/logicaffeine_compile/src/spatial/pipeline.rs`).
- **The D5005 engine:** matches StepGraph blocks into commands
  (`crates/logicaffeine_compile/src/spatial/decode_engine/lower_graph.rs`).

The work in this book moves all four onto one IR. Fusions become shared, and each chip keeps only what is genuinely its
own.

### 5.2 The one IR

**Where it lives:** `crates/logicaffeine_compile/src/tensor_graph/`, which already holds `vjp`. It holds only the meaning
side, and imports no chip descriptor (`INV-DENOTATIONAL-PURITY`). Its modules:

| Module | Contents |
|---|---|
| `tensor_graph::front` | the front end, moved from `spatial/passes/recognize_stages.rs` |
| `tensor_graph::graph` | `TensorGraph`, StepGraph promoted out of the lane |
| `tensor_graph::facts` | dataflow facts; `facts::range` comes from `spatial/value_range.rs` |
| `tensor_graph::eval` | the evaluator |
| `tensor_graph::corpus` | seeded corpora, lifted from `spatial/equiv.rs` |
| `tensor_graph::symbols` | the class registry |
| `tensor_graph::shim` | the `Shim` trait and `ReferenceShim` |
| `tensor_graph::link` | the generic resolver |
| `tensor_graph::fuse` | rules and the rewriter |
| `tensor_graph::numerics` | quantize insertion |
| `tensor_graph::egraph` | equality saturation (task OPT-4) |

The lane's `Part`, `Dst` and `RowSel` move in verbatim.

```rust
pub struct TensorGraph {
    pub identity: Option<Identity>,      // the cut it was lifted from (model::program)
    pub io: Option<StepIo>,              // token, position, logits, next
    pub tensors: BTreeMap<String, Tensor>,
    pub ops: Vec<Op>,                    // program order, the schedule order every backend starts from
}
pub struct Tensor {
    pub name: String, pub rows: usize, pub cols: usize,
    pub role: Role,                      // Param | State | Constant | Activation
    pub elem: Elem,                      // Float | Int
    pub fill: Option<Fill>,
    pub storage: Storage,                // Computed | Values | Rows { tensor, format, transposed, view }
    pub range: Option<Interval>,         // a proven interval
    pub observed: bool,                  // a Show, a scalar read or an opaque statement touches it
}
pub struct Op {
    pub sym: Sym,
    pub ins: Vec<String>, pub access: Vec<Access>,   // Access::{Whole, Part(Part)}
    pub out: String, pub dst: Dst,                   // row and unit destinations
    pub keeps: Vec<String>,                          // further results written whole
    pub covers: Vec<usize>,                          // lifted ops it computes
}
pub enum Sym { Prim(TensorOpSpec), Fused(Fused), View { offset: usize }, Quantize(QuantFormat) }
```

- **State is what an op writes in place.** That covers the `Set row` table, the convolution ring and the delta-rule
  memory, plus any row or unit destination.
- **Facts replace scans.** `facts.untouched(t, range, Reads | Writes)` answers "does anything touch `t` in between",
  replacing the scan each lane rule repeats today.
- **Cuts are separate graphs.** Each cut of the program (decode, a prefill chunk, a batch, a per-layer kind) is its own
  graph and carries its `identity` and `io`.

### 5.3 Three kinds of ledger id, and edge layouts

| Kind | What it is | Examples |
|---|---|---|
| Fused symbol | built by a rule; its parts are its definition | `add_norm`, `norm_quant`, `gemm_bias`, `qk_norm_rope`, `glu_act_mul`, `gemv_gate_up_glu`, `shared_expert_fusion`, `moe_token_sort`, `merged_qkv`, `conv_bias_silu`, `fused_sampling` |
| Primitive symbol | an op the front end lifts whole; a chip links it whole or runs its parts | `norm_weight`, `flash_decode`, `flash_prefill`, `gdn_step`, `router_topk`, `moe_gate_up_act`, `moe_down_combine`, `gemm_dequant_inkernel`, `attention_variants_inline` |
| Schedule property | how a chip runs something; not a fusion | `cuda_graphs`, `rows_a_warp`, split-K |

**Placement fusions are layouts on edges,** not symbols:
- `slice_read` becomes a strided read, `Access::Part`;
- `row_write` becomes a row destination;
- `join_in_place` becomes a unit destination;
- `strided_values` becomes values read in place.

A composite class follows from those: a rotation with a row destination into the cache is `rope_kv_append`.

**Some fusions exist only here, in no other stack.** They get "ours" ids: `strided_read`, `join_in_place`,
`strided_values`, `embed_scale`, `gdn_gating`, `conv_silu_l2norm`, `gdn_output_quant`, `gemv_quant`, `delta_qk_norm`.

`Fused { class, parts }` carries its definition: the parts keep their names, accesses and destinations. When a rule
absorbs a fused node, the class upgrades: `add_norm` plus a quantize becomes `add_norm_quant`. Each class has a typed
view built by the same matcher its rule uses, such as `add_norm(&Fused) -> AddNormView`. Links read the view and never
re-match.

### 5.4 The shared optimizer

`optimize(g, shims, held) -> Optimized { graph, fired }` runs the rule table.

```rust
pub struct Rule { name, kind: Group | Place | Fold | Rewrite | Numerics, builds: &[ClassId], anchor_policy, apply }
```

- **Commit condition.** `Rewriter::propose` commits a candidate only when the target shims answer `Native` and the
  candidate costs no more than the ops it replaces.
- **Names and order.** Rule names stay as they are today, so `Schedule::fired("add_norm")` keeps answering. Each rule
  keeps its anchor slot, so the launch order does not move.
- **Purity.** The optimized graph is a pure function of the lifted graph, the rule table and the shim. Intersecting what
  every chip allows is only for a deployment that spans chips.
- **Numerics lowering.** `tensor_graph::numerics` inserts `Quantize` nodes where `Shim::activation` asks for them. Every
  `*_quant` class (norm, add-norm, activation, attention output, GDN output, GEMV) is then a shared fold, not a lane
  flag.
- **The report.** Each compile writes one: every rule that could fire, and whether it fired or declined. A decline is
  illegal, refused by the chip, unprofitable or held. Artifacts carry the report's digest beside the program identity,
  so the fusions a model received on each chip can be read straight off its artifacts.

### 5.5 The math layers

The optimizer is built on layers we already have.

| Layer | What it does in the optimizer | Side |
|---|---|---|
| `crates/logicaffeine_compile/src/optimize/egraph/` | Its generic core is lifted to `crate::egraph` (`Language`, `Analysis`, `CostModel`, `Rewrite` with a `Certificate`), and `CompilerEGraph` becomes one instance of it. It runs equality saturation over pure tensor terms. Those rewrites cover absorbed vs expanded latent attention (matmul reassociation), merged projections, scale and norm-weight folds, transpose elimination, and contraction recognition. Extraction uses each shim's cost. | meaning (rewrites), operation (extraction) |
| The kernel's `Certificate::Ring` | Certifies each algebraic rewrite at a bounded shape. A floating-point reassociation is admitted only when the shim's numerics allow it. | meaning |
| `crates/logicaffeine_compile/src/spatial/value_range.rs` | Moves to `facts::range`: proven integer intervals. | meaning |
| `crates/logicaffeine_compile/src/optimize/accum_bound.rs` | The model for `facts::accum`: an integer accumulator proven wide enough. | meaning (fact) |
| `optimize/{bta,partial_eval,ctfe,supercompile}.rs` | The concept carries over: binding time on the graph, and `tensor_graph::specialize` folds static subgraphs by running `eval`. | meaning |
| `optimize/{deforest,fold_fusion,splice_fuse}.rs` | The legality law behind vertical fusion: an intermediate nothing else observes need not be built. | meaning |
| `crates/logicaffeine_tv` | A new `tv::tensor` module (verification feature) proves grouping rules over arrays with uninterpreted functions, and algebraic rewrites at bounded shapes. | meaning (proof) |
| `crates/logicaffeine_synth` | The workflow: prove offline, check in the witness, re-check it cheaply in tests. | workflow |
| `spatial/{equiv,eval,certificate}.rs` | Keep validating spatial links and schedules; only their corpus generators move. | operation |
| `spatial/{cost,roofs}.rs`, `spatial/passes/tile.rs` | The spatial shim's cost model. | operation |

The rule engine serves until task OPT-4. OPT-4 then swaps in phase-ordered saturation, one phase per rule in today's
order with extraction after each phase, which reproduces the greedy plan exactly. Merging phases after that is a
measured change, and needs Tristen's approval.

### 5.6 The shim: "here's what I allow"

```rust
pub trait Shim {
    fn name(&self) -> String;                                  // e.g. "cuda-lane:rtx3060:TwoPassInt8"
    fn allows(&self, op: &Op, g: &TensorGraph) -> Allow;       // Native(primitive) | Decompose | Refuse(why)
    fn numerics(&self) -> Numerics;                            // precisions, accumulators, reassociation, tolerances
    fn activation(&self, consumer: &Op, operand: usize, g: &TensorGraph) -> Option<QuantFormat>;
    fn limits(&self) -> Limits;                                // device and local memory, unit widths
    fn cost(&self, op: &Op, g: &TensorGraph) -> Cost;          // launches, bytes, cycles
    fn rules(&self) -> Vec<(Position, Rule)> { Vec::new() }    // chip-only rules: Before / After(name) / End
}
```

Each implementation borrows the descriptor that already exists, so no descriptor is duplicated:

| Shim | Built from | What it answers |
|---|---|---|
| `LaneShim { knobs, held }` | `CudaChip` through `CudaKnobs` | `allows` is the lane's own link succeeding, so the capability answer and the kernel table cannot drift apart. The kernel-only predicates the lane rules carry today move here. |
| `ForwardShim { kind }` | the older forward | `cuda_stage`'s refusals, word for word. |
| `SpatialShim { target, backend }` | `SpatialTarget`, `ComputeCap` and the roofs; the RDU through `RduChip::facts().spatial_target()` | Starts at `Decompose` for fused classes and turns `Native` class by class. |
| `EngineShim { build, numerics }` | `EngineParams` and the D5005 `capability` module | `spec_refusals` and the `lower_graph` refusals, made per node. `covers` becomes `limits()`. |
| `ReferenceShim` | nothing | Allows everything, for graph-level tests. |

### 5.7 The link

`tensor_graph::link::resolve(g, shim, link)` drives every backend:
- `Native` goes to the backend's own link;
- `Decompose` goes to the parts, each of which must resolve;
- `Refuse` produces `Unresolved { op, class, why }`.

Nothing is dropped silently.

Per backend:
- **CUDA lane.** `plan.rs` becomes three steps: optimize under `LaneShim`, then `lane/link.rs` (the op-to-launch arms
  plus one builder per fused class), then `lane/schedule.rs`.
- **Older forward.** It walks the optimized graph. Its MoE trio becomes its link of `Prim(Moe)`.
- **Spatial.** `TensorOp` and `tensor_op_of` go away, and `lower_section` reads tensor facts instead. `dense_tables`,
  `agree_layouts` and the q8 view names become a spatial link prelude.
- **D5005.** `lower_graph` reads symbols directly:

  | Symbol | D5005 command |
  |---|---|
  | `merged_qkv` | one Matvec, under a layout condition the engine shim checks |
  | `conv_bias_silu` | Conv |
  | `gdn_gating` | Gates |
  | `glu_act_mul` (sigmoid) | SigmoidGate |
  | `kv_cache_quant` | the Hadamard int8 cache |

### 5.8 Each chip's own path

What is not a fusion stays with the chip:
- **CUDA:** tiles, split-K, rows a warp, attention splits, shared-row attention, buffers, arena, residence, the
  branched streams, one CUDA graph a form.
- **Spatial:**
  - its passes: `fuse_generic`, `tile`, `place`, `fuse_readers`, `size_buffers`, the handshakes, `persist`, `verify`,
    `compose`, `pack`, `shard`;
  - section joining (`forward`) also stays its own, but its legality reads the shared facts.
- **D5005:** command packing.

A chip may also link chip-only rules into the table through `Shim::rules()`. Those rules are registered, can be held,
and carry definitions like every other rule.

### 5.9 Where today's lane rules go

- [ ] add_norm (group): goes to shared; class `add_norm` (then `add_norm_quant`); predicate moving into `LaneShim`: none.
- [ ] gemv_bias (group): goes to shared; class `gemm_bias`; predicate moving into `LaneShim`: none.
- [ ] head_norm_rope (group): goes to shared; class `qk_norm_rope`; predicate moving into `LaneShim`: none.
- [ ] slice_read (place): goes to shared; class `strided_read`; predicate moving into `LaneShim`: a per-head product's pitch and offset are whole 32-blocks.
- [ ] row_write (place): goes to shared; class `rope_kv_append` / `qk_norm_rope_kv_append`; predicate moving into `LaneShim`: an expert mixture cannot write at a parameter row.
- [ ] join_in_place (place): goes to shared; class `join_in_place`; predicate moving into `LaneShim`: which kernels write unit parts.
- [ ] conv_silu (group): goes to shared; class `conv_bias_silu`; predicate moving into `LaneShim`: none.
- [ ] conv_silu_l2 (group): goes to shared; class `conv_silu_l2norm`; predicate moving into `LaneShim`: cols ≤ 1024, cols % 32, chan % cols.
- [ ] gates (group): goes to shared; class `gdn_gating`; predicate moving into `LaneShim`: none.
- [ ] act_gate (group): goes to shared; class `glu_act_mul`; predicate moving into `LaneShim`: none.
- [ ] glu_pair (group): goes to shared; class `gemv_gate_up_glu` / `merged_gate_up`; predicate moving into `LaneShim`: m ≤ 16, or Dequant, or the same format and n % 64 on tensor cores.
- [ ] shared_expert (group): goes to shared; class `shared_expert_fusion`; predicate moving into `LaneShim`: k < 32, one row.
- [ ] moe_rows (group): goes to shared; class `moe_token_sort`; predicate moving into `LaneShim`: the multi-row expert kernel.
- [ ] gather_scale (group): goes to shared; class `embed_scale`; predicate moving into `LaneShim`: none.
- [ ] strided_values (place): goes to shared; class `strided_values`; predicate moving into `LaneShim`: even pitch and offset.
- [ ] argmax_advance (group (io)): goes to shared, always; class `fused_sampling`; predicate moving into `LaneShim`: none.
- [ ] quantize (numerics): goes to shared `tensor_graph::numerics`; class none; predicate moving into `LaneShim`: `Shim::activation`.
- [ ] fused_quant (fold): goes to shared; class the `*_quant` classes; predicate moving into `LaneShim`: the `fuse_quant` knob; four-rows-a-warp GEMVs; n % 32.
- [ ] merged_projections (group (after numerics)): goes to shared; class `merged_qkv`; predicate moving into `LaneShim`: tensor cores, the same format, n % 128, at most four parts.
- [ ] latent_expanded (rewrite): goes to a `LaneShim` chip rule until OPT-4, then the shared e-graph; class `mla`; predicate moving into `LaneShim`: the tiles regime, not Dequant.
- [x] delta_qk_norm (hand over): a `LaneShim` chip rule after `gdn_input_epilogue`, handing the query's and key's norms from the inputs' fold to the delta rule that reads them by the IR's `Rewriter::hand_over`; class `delta_qk_norm`; predicate in `LaneShim`: a block of the inputs' launch holds less than a head (receipt: `lane_compile_lock::a_steps_delta_rule_takes_its_heads_norms_only_where_its_inputs_launch_holds_no_whole_head`).
- [ ] rows_a_warp (schedule): goes to CUDA; class none; predicate moving into `LaneShim`: all of it, still holdable.
- [ ] shared_rows, split-K, GEMM tails, attention splits, buffers, arena, residence, branches, graphs (schedule): goes to CUDA; class none; predicate moving into `LaneShim`: all of it.

Two checks keep the reordering safe:
- four rows a warp applies only when m = 1, so it never meets the tensor-core path, which needs m > 16;
- shared-row attention and split-K only set fields.

### 5.10 What the other stacks show

**vLLM already has this shape in small.**
- Its compile passes match implementation-agnostic IR ops, such as `vllm_ir.rms_norm` and `vllm_ir.fused_add_rms_norm`.
  `VllmIRLoweringPass` (`external/competitors/vllm/vllm/compilation/passes/ir/lowering_pass.py`) then lowers each
  remaining op to a provider, chosen by a priority list per platform.
- An op is defined by its native function (`external/competitors/vllm/vllm/ir/op.py`, `register_op`). A provider
  registers with a `supports_args` predicate (`register_impl`).
- Its fused-MoE stack and linear-kernel selection work the same way: priority lists of kernels, each answering
  `is_supported(cc)` and `can_implement(cfg)`.

We go further in three ways:
- the shim is consulted before fusing, not after;
- extraction is driven by cost;
- every rule carries a proof.

vLLM's `supports_args` is an opaque predicate; our shim's answer is declared data.

**Triton cannot fuse across kernels.**
- One `@triton.jit` function is one kernel: the NVIDIA backend asserts a single `.entry` in the PTX it emits
  (`external/competitors/triton/third_party/nvidia/backend/compiler.py`).
- Every fused kernel in its tutorials is written by hand.
- The production MoE matmul in `triton_kernels` builds its fused epilogues by templating source strings
  (`external/competitors/triton/python/triton_kernels/triton_kernels/specialize.py`, `define_kernel`).

The shared optimizer here fuses at the graph level, automatically, and proves each fusion. Section 10.3 lists the rest of
Triton's documented limits. Each is a task, and each becomes a claim once its task is done.

### 5.11 Designed for fifty years

The chips of 2075 do not exist yet, and neither do their number formats. What lasts is meaning, proofs, and a way to take
in new knowledge. The design rests on six choices:

1. **Meaning first.**
   - A rewrite is admitted because it provably preserves what the program means, never because it was faster on one
     card.
   - A proof stays true when the hardware changes.
2. **Chips are data.**
   - A new chip is a shim: what it allows, what it costs, and the primitives it links. That holds for chips nobody has
     built yet (photonic, analog, in-memory, wafer-scale).
   - Adding a chip touches no optimizer code.
   - Every chip can run every program from its first day, as parts.
3. **The vocabulary grows by definition.**
   - A new op or fused symbol enters as a definition in terms of what already exists.
   - Every chip runs it as parts at once, and a chip makes it fast by linking a primitive.
4. **Decisions come from search.**
   - The legal rewrite to take is extracted by each chip's cost model.
   - Costs come from measurement wherever a measurement exists.
   - When a chip's economics change, its decisions follow.
5. **Numerics are declared contracts.**
   - The IR's meaning is exact.
   - Each chip states the approximation it makes, as a contract with a bound that is proven or measured.
   - A new number format is a new contract, not a new compiler.
6. **Knowledge accrues.**
   - Every fusion found by vLLM, Triton, llama.cpp or an agent becomes a rule with a definition, a proof and a ledger row.
   - A contributed rule is accepted when it proves its definition and pays on some chip's cost model or in a same-boot
     measurement.

---

## 6. Requirements

- [ ] **REQ-ONE-PROGRAM.** Every artifact of a model comes from `model::author`'s one LOGOS program (`one_program_lock`).
- [ ] **REQ-ONE-IR.** Below the front end there is exactly one intermediate representation, and every tensor backend consumes it: the CUDA lane, the older CUDA forward, the spatial pipeline and its four backends (Tensix, RDU, fabric Verilog, persistent CUDA), the D5005 engine, and the evaluator.
- [ ] **REQ-MEANING.** The IR has a reference evaluator, held equal to the VM on the same program. What a graph computes can be answered without any chip.
- [ ] **REQ-SHARED-OPTIMIZER.** Every fusion that preserves meaning and is not peculiar to one chip is a rule in one table over the IR. A rule is a legality predicate over facts computed once, a rewrite into a fused symbol, and that symbol's definition. Rules run in a fixed order, are deterministic, and can each be held back.
- [ ] **REQ-LEDGER-IDS.** A fused symbol's id is its ledger class id. A fusion mined from another stack lands as a symbol, never as backend code.
- [ ] **REQ-SHIM.** One trait, implemented by every chip from the descriptors that already exist, answers for every symbol and primitive: native, parts, or refused with a reason. It also declares numerics, layouts, limits and cost.
- [ ] **REQ-LINK.** Each chip resolves every symbol to its own primitive or to parts, or refuses by name. Nothing is dropped.
- [ ] **REQ-OWN-PATH.** Schedules stay with the chip. Chip-only rules enter the table through the shim, can be held, and carry definitions.
- [ ] **REQ-NUMERICS.** Shared rewrites are exact under the IR's meaning. A chip's numerics are declared in its shim and held to their bound. Fused equals apart bit for bit where the chip claims exactness.
- [ ] **REQ-REPORT.** Every compile reports each rule as fired or declined with a reason, and artifacts carry the report's digest.
- [ ] **REQ-PURE-REFACTOR-FIRST.** Moving today's representations onto the IR changes no artifact by a byte. New behaviour lands only after that.
- [ ] **REQ-PARITY.** Every fusion and technique in section 10 is catalogued, added as a shared symbol or a chip technique, linked per chip, and measured same-boot against vLLM, Triton and llama.cpp.
- [ ] **REQ-COVERAGE.** Every op, kernel, pattern, pass, backend and builtin vLLM and Triton ship at the pins has a row in section 11 naming the ticket that copies it, and the rows are re-extracted from the pinned trees, never maintained by hand alone.
- [ ] **REQ-SOTA.** Every cell of section 2.4 is won against the fastest rival in section 2.2 under the rules of section 2.3, and every Copy bar is the row maximum of section 2.5.

## 7. Invariants that define success

Each invariant names the lock that holds it. Every lock is mutation-tested.

- [ ] **INV-ONE-IR**, held by `one_ir_lock::{the_tensor_graph_lives_in_tensor_graph, the_front_end_lives_in_tensor_graph, no_backend_keeps_a_private_op_enum, every_backend_lowers_the_tensor_graph}`: Only `tensor_graph` builds from the front end and matches on `TensorOpSpec`. The spatial `TensorOp` and the forward's `Stage` no longer exist.
- [ ] **INV-DENOTATIONAL-PURITY**, held by `one_ir_lock::the_tensor_graph_imports_no_chip`: Nothing under `tensor_graph` imports `codegen_cuda`, `spatial::{target, lower, decode_engine}` or a GGUF reader. Storage arrives through a closure the caller supplies.
- [ ] **INV-DENOTATION**, held by `tensor_graph_eval_lock::{the_evaluator_computes_what_the_vm_computes_for_every_tiny_cut, every_op_kind_has_an_evaluation_or_a_named_refusal, access_and_destination_mean_what_they_say}`: The evaluator equals the VM bit for bit.
- [ ] **INV-SYMBOL-MEANS-PARTS**, held by `fusion_class_lock::{every_rule_preserves_its_exemplars_meaning, every_rule_holds_back_where_its_dataflow_forbids, every_rule_can_be_held}`: A fused node evaluates exactly as its parts on seeded corpora.
- [ ] **INV-NO-BACKEND-FUSES**, held by `one_ir_lock::{no_fusion_rule_is_added_outside_the_tensor_graph, the_lane_derives_no_fusion}`: The table is the only graph rewriter. The allowlist only shrinks.
- [ ] **INV-SHIM-TOTAL**, held by `shim_lock::every_shim_answers_every_class`: Exhaustive matches with no catch-all arm. Every refusal names its reason.
- [ ] **INV-LINK-TOTAL**, held by `shim_lock::{the_lane_shim_is_its_link, every_native_answer_links}`, `d5005_lower_graph_lock::capability_is_the_engine_shim`: A native answer links; parts resolve; refusals equal what the chip's capability module says.
- [ ] **INV-FUSED-EQUALS-APART**, held by `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`, generalized to each chip's executor (the GPU, `spatial::eval`, the D5005 twin, the fabric simulation), and `numerics_contract_lock`: Fused equals apart bit for bit where the shim claims exactness, else within its declared bound.
- [ ] **INV-REFACTOR-IDENTITY**, held by `one_ir_identity_lock`: Lane, engine, spatial and forward artifacts stay byte-identical through every migration step.
- [ ] **INV-PROVEN-REWRITES**, held by `tensor_egraph_lock::every_tensor_rewrite_carries_a_certificate`, `tensor_rule_proof_lock`: Every algebraic rewrite carries a kernel certificate. Every grouping rule is proven over arrays.
- [ ] **INV-LEDGER-AND-BOOK**, held by `fusion_parity_lock`, extended by LEDGER-2: Status is per chip. A compiled claim cites a census through the shared optimizer, a link on that chip and a values test. This book names every class id and nothing else. Citations resolve at the pins.
- [ ] **INV-HONEST-REPORT**, held by `fusion_report_lock`: The same input gives the same graph and report. Holding a rule changes only its own entries.
- [ ] **INV-NO-SLOWER**, held by `model_scoreboard_lock` and a same-boot rerun: Scoreboard rows never fall.
- [ ] **INV-COVERAGE-TOTAL**, held by `sota_contract_lock::{every_entry_at_the_pins_has_a_coverage_row, every_coverage_row_names_an_entry_at_the_pins, every_coverage_row_names_tickets_on_the_board}`: Section 11 equals the pinned trees' inventories, row for row.
- [ ] **INV-SOTA-CLAIMS**, held by `sota_contract_lock::{a_ticked_cell_has_a_won_ledger_row, a_won_row_meets_every_fair_win_rule, no_cell_is_ticked_while_a_closer_is_open}` and `kernel_ab_lock::the_row_maximum_is_derived_and_a_copy_tick_meets_it`: No cell is claimed without a win, and no Copy bar is ticked below the fastest rival kernel.

## 8. What we reuse

There is no parallel machinery. Each piece of the new structure is built from something already in the tree.

- [ ] **Home:** `crates/logicaffeine_compile/src/tensor_graph/`, as the IR's module.
- [ ] **Front end:** `spatial::passes::recognize_stages::walk` and `RecognizedProgram`, as unchanged; moves to `tensor_graph::front`.
- [ ] **The IR's seed:** `StepGraph`, `Role`, `readers`, as promoted out of the lane.
- [ ] **Legality:** the lane's rule predicates (`plan.rs`, the rule functions after `RULE_TABLE`), as split from their CUDA facts.
- [ ] **Holding rules:** `compile_holding` and held sets, as the holdability mechanism.
- [ ] **The table pattern:** `plan::RULES`, as the rule table.
- [ ] **Shims:** `CudaChip` (`crates/logicaffeine_compile/src/codegen_cuda/chip.rs`), `SpatialTarget`, `ComputeCap` and `Roofs` (`crates/logicaffeine_compile/src/spatial/target.rs`), `RduChip`, D5005 `capability::{refusals, covers}` and `EngineParams`, as the shims' sources.
- [ ] **Totality:** `SpatialBackend` and `spatial_backend_totality_lock`, as the totality pattern.
- [ ] **Front door:** `bringup::Backend` (`crates/logicaffeine_compile/src/bringup.rs`), as the bring-up entry.
- [ ] **Evaluators:** `spatial::eval`, `model::reference`, the D5005 twin, as value oracles.
- [ ] **Proof machinery:** `optimize/egraph`, the kernel's certificates, `logicaffeine_tv`, `logicaffeine_synth`, as proofs and equality saturation.
- [ ] **Ledger:** `benchmarks/fusion_parity.json`, `fusion_parity_lock`, `one_program_lock`, as parity and provenance.
- [ ] **Scoreboard:** `scripts/accel/model_scoreboard.py`, `benchmarks/results/models/rtx3060_scoreboard.json`, `model_scoreboard_lock`, as vLLM becomes a third side.
- [ ] **GEMM tiling:** `crates/logicaffeine_compile/src/codegen_cuda/gemm_tile.rs`, as seed of the generated-kernel tier.
- [ ] **Persistent CUDA:** `crates/logicaffeine_compile/src/spatial/lower/cuda.rs`, as seed of the generated-kernel tier.

## 9. How we lift without breaking anything

### 9.1 The rules of the lift

- **New beside old, then one consumer at a time.**
  - The IR is built first, beside today's four representations, and nothing reads it.
  - Each consumer then switches in its own step, starting from an adapter that reproduces exactly what it read before.
    The switch is byte-identical by construction.
- **Digests before code.** `one_ir_identity_lock` is written and green on today's tree before the first migration edit.
- **One switch, behind a differential.** `one_ir_lock::the_shared_table_fires_what_the_lane_fires` proves, rule by rule,
  that the shared table fires exactly the lane's fusions. MIGRATE-1 then deletes `RULE_TABLE` in the same edit that makes
  the lane link. The final tree never holds two copies of a rule.
- **Cheap locks.** Every new lock is CPU-only: compile paths only, no GPU, and nvcc only where an existing lock already
  uses it. All of them run in the fast suite. GPU values run in the existing GPU locks when a step touches a kernel.
- **The old forward is ported, not deleted.** `cuda_fusion_census_lock` and the `cuda_forward_*` locks keep passing on
  the new path.
- **The blast radius is known.**

  | Consumer | Test files touching it |
  |---|---|
  | Spatial pipeline | 123 |
  | D5005 engine | 26 |
  | Older forward | 25 |
  | Front end | 21 |
  | CUDA lane | 7 |

  The adapter-first order means none of them is edited during the move.

### 9.2 Who owns what (2026-10-25)

| Files | Owner session | How a task touches them |
|---|---|---|
| `crates/logicaffeine_compile/src/codegen_cuda/lane/` (all files), `crates/logicaffeine_compile/examples/model_lane.rs`, `scripts/accel/model_scoreboard.py`, `benchmarks/results/models/` | audit-gaps-demo-prep (the performance lane) | ask on the lanes board for a freeze window; record digests at the window's start |
| `crates/logicaffeine_compile/src/spatial/decode_engine/lower_graph.rs`, `capability.rs` | d5005-model-foundry | handoff; their hands or explicitly co-signed |
| `crates/logicaffeine_compile/src/spatial/decode_engine/{program,model,machine,image,twin,ofs}.rs` | qwen-fpga-decode-engine | handoff |

Ownership moves. Read the lanes board before claiming, and treat this table as the state at the book's writing.

---

## 10. The parity catalog

Each row has five parts:
- what the fusion does;
- where the best current implementation lives, as a file and a grep-unique anchor at the pins;
- what it serves;
- our state today, with its receipt;
- the task that closes it.

Column abbreviations:
- **Serves:** D = decode, P = prefill.
- **Model names:** `gemma2` is Gemma-2-2B, `qwen35` is Qwen3.5-0.8B and Qwen3.8-27B (one architecture), `ds2` is
  DeepSeek-V2-Lite, `oss` is gpt-oss-20b.

Quantization schemes (fp8 static, per token, group 128, UE8M0, TMA-aligned, NVFP4, MXFP4, MXFP8, int8 with zero point)
are a `schemes` attribute on a class, not separate classes.

### 10.1 The 36 classes already in the ledger

The stack citations for these rows are in `benchmarks/fusion_parity.json`. Our receipts are the census and values
tests named there.

- [x] **`norm_weight`**: compiled. Receipt: `cuda_fusion_census_lock::a_weighted_norm_is_one_row_kernel`. Ticket: MIGRATE-1, MIGRATE-2.
- [x] **`add_norm`**: compiled. Receipt: `cuda_fusion_census_lock::an_add_then_norm_runs_as_one_kernel_and_matches_the_vm_on_the_gpu`. Ticket: MIGRATE-1.
- [x] **`norm_then_add`**: compiled (the lane's `sandwich_norm`). Receipt: `lane_compile_lock::the_sandwiched_step_norms_each_sublayer_output_inside_the_sums_norm` / `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`. Ticket: PARITY-NORM-THEN-ADD (its shared rule).
- [x] **`norm_scale`**: compiled. Receipt: `lane_compile_lock::a_scaled_norm_is_one_row_kernel` / `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`. Ticket: PARITY-NORM-SCALE.
- [x] **`norm_quant`**: compiled (int8, one and two words). Receipt: `cuda_fusion_census_lock::a_norm_feeding_an_int8_projection_quantizes_in_its_epilogue`. Ticket: MIGRATE-1; schemes in PARITY-QUANT-SCHEMES.
- [x] **`add_norm_quant`**: compiled (int8). Receipt: `cuda_fusion_census_lock::an_add_then_norm_feeding_an_int8_projection_quantizes_in_its_epilogue`. Ticket: MIGRATE-1; PARITY-QUANT-SCHEMES.
- [x] **`qk_norm_rope`**: compiled. Receipt: `lane_compile_lock::the_dense_steps_fuse_where_their_dataflow_allows` / `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`. Ticket: MIGRATE-1.
- [x] **`qk_norm_rope_kv_append`**: compiled. Receipt: same pair. Ticket: MIGRATE-1.
- [x] **`rope_kv_append`**: compiled. Receipt: same pair. Ticket: MIGRATE-1.
- [x] **`allreduce_norm`**: compiled (tensor parallelism emulated on one device, `compile_split`). Receipt: `lane_compile_lock::a_tensor_parallel_step_folds_each_all_reduce_into_its_norm` / `cuda_lane_lock::a_tensor_parallel_step_computes_what_its_launches_compute_apart`. Ticket: PARITY-ALLREDUCE-NORM.
- [x] **`glu_act_mul`**: compiled. Receipt: `lane_compile_lock::the_hybrid_step_fuses_the_recurrence_and_the_gate`. Ticket: MIGRATE-1.
- [x] **`glu_act_mul_quant`**: compiled (int8). Receipt: `lane_compile_lock::the_dense_steps_fuse_where_their_dataflow_allows`. Ticket: MIGRATE-1; PARITY-QUANT-SCHEMES.
- [x] **`elementwise_chains`**: compiled (linear chains only). Receipt: `cuda_elementwise_fusion_lock::a_linear_elementwise_chain_fuses_into_one_kernel`. Ticket: MIGRATE-2; DAG regions in PARITY-ELEMENTWISE-REGIONS.
- [x] **`softcap`**: compiled (scores inside the attention, logits inside the arg-max that reads them). Receipt: `lane_compile_lock::a_capped_attention_caps_inside_its_kernel` / `cuda_lane_lock::a_capped_attention_computes_what_its_reference_computes`. Ticket: PARITY-SOFTCAP.
- [x] **`relu_sqr`**: compiled. Receipt: `lane_compile_lock::a_relu_square_compiles_and_folds_into_its_producer` / `cuda_lane_lock::a_relu_square_computes_what_the_reference_computes`. Ticket: PARITY-RELU-SQR.
- [x] **`gemv_gate_up_glu`**: compiled. Receipt: `lane_compile_lock::the_dense_steps_fuse_where_their_dataflow_allows`. Ticket: MIGRATE-1.
- [x] **`gemm_bias`**: compiled. Receipt: `lane_compile_lock::the_biased_step_folds_its_biases_and_routes_by_an_f32_router`. Ticket: MIGRATE-1.
- [x] **`gemm_scale`**: compiled. Receipt: `lane_compile_lock::a_scaled_projection_scales_in_its_epilogue` / `cuda_lane_lock::a_scaled_projection_computes_what_its_launches_compute_apart`. Ticket: PARITY-GEMM-SCALE.
- [x] **`merged_qkv`**: compiled. Receipt: `lane_compile_lock::a_wide_chunk_in_a_block_format_runs_on_the_tensor_cores` / `cuda_lane_lock::a_gemm_in_parts_computes_each_parts_gemm`. Ticket: MIGRATE-1.
- [x] **`merged_gate_up`**: compiled. Receipt: `cuda_lane_lock::a_paired_gemm_computes_what_its_gate_and_up_compute_apart`. Ticket: MIGRATE-1.
- [x] **`gemm_dequant_inkernel`**: compiled. Receipt: `cuda_fusion_census_lock::an_int8_weight_projection_dequantizes_inside_its_gemv`. Ticket: MIGRATE-1 (primitive).
- [x] **`router_topk`**: compiled. Receipt: `cuda_fusion_census_lock::a_sparse_moe_routes_with_one_topk_kernel_and_two_fused_expert_kernels`. Ticket: MIGRATE-1 (primitive).
- [x] **`moe_gate_up_act`**: compiled. Receipt: same. Ticket: MIGRATE-1 (primitive).
- [x] **`moe_down_combine`**: compiled. Receipt: same. Ticket: MIGRATE-1 (primitive).
- [x] **`shared_expert_fusion`**: compiled. Receipt: `lane_compile_lock::the_shared_expert_folds_into_its_mixture_where_nothing_else_reads_its_parts`. Ticket: MIGRATE-1.
- [x] **`moe_token_sort`**: compiled. Receipt: `lane_compile_lock::a_wide_chunks_experts_run_on_the_tensor_cores` / `cuda_lane_lock::the_grouped_expert_gemms_compute_what_the_slabs_compute`. Ticket: MIGRATE-1.
- [x] **`flash_decode`**: compiled. Receipt: `lane_compile_lock::the_decode_step_attends_a_long_cache_in_chunks_one_launch_a_layer` / `cuda_lane_lock::a_long_prompt_attends_in_chunks_alike_in_every_form`. Ticket: MIGRATE-1 (primitive).
- [x] **`flash_prefill`**: compiled. Receipt: `lane_compile_lock::a_wide_chunk_attends_in_tiles` / `cuda_lane_lock::the_tensor_core_attention_computes_what_the_attention_computes`. Ticket: MIGRATE-1 (primitive).
- [x] **`attention_variants_inline`**: compiled (sliding and chunked-local windows, sinks and caps; no mask read from a tensor). Receipt: `lane_compile_lock::every_attention_kernel_takes_a_cap_and_a_mask` / `cuda_lane_lock::a_chunked_local_attention_computes_what_its_reference_computes`. Ticket: PARITY-ATTENTION-VARIANTS-INLINE.
- [x] **`attn_output_quant`**: compiled (int8). Receipt: `cuda_fusion_census_lock::an_attention_feeding_an_int8_projection_quantizes_in_its_epilogue`. Ticket: MIGRATE-1; PARITY-QUANT-SCHEMES.
- [x] **`mla`**: compiled. Receipt: `lane_compile_lock::the_latent_step_absorbs_each_heads_query_and_reads_its_values_in_the_cache` / `cuda_lane_lock::the_expanded_latent_attention_computes_what_the_absorbed_one_computes`. Ticket: MIGRATE-1; OPT-4.
- [x] **`gdn_step`**: compiled. Receipt: `lane_compile_lock::the_hybrid_step_fuses_the_recurrence_and_the_gate` / `cuda_lane_lock::the_chunked_delta_rule_computes_what_the_delta_rule_a_row_at_a_time_computes`. Ticket: MIGRATE-1; PARITY-GDN-DECODE-WHOLE.
- [x] **`conv_bias_silu`**: compiled. Receipt: `lane_compile_lock::the_hybrid_step_fuses_the_recurrence_and_the_gate`. Ticket: MIGRATE-1.
- [x] **`fused_sampling`**: compiled (arg-max only). Receipt: `lane_compile_lock::the_dense_steps_fuse_where_their_dataflow_allows`. Ticket: MIGRATE-1; PARITY-SAMPLING.
- [x] **`cuda_graphs`**: compiled. Receipt: `cuda_fusion_census_lock::the_compiled_forward_is_captured_as_one_cuda_graph`. Ticket: schedule property.
- [x] **`kv_cache_quant`**: compiled (f16 and e4m3 caches on CUDA); the D5005 engine has int8 and Hadamard int8 caches. Receipt: `lane_compile_lock::a_quantized_cache_is_written_and_read_in_its_format` / `cuda_lane_lock::an_e4m3_cache_decodes_alike_in_every_form_and_near_its_reference`. Ticket: PARITY-KV-CACHE-QUANT.

The ledger's seven serving rows (continuous batching, paged KV, prefix cache, chunked prefill, speculative decoding,
expert offload, per-batch graphs) are all gaps. Their tickets are in section 12.12.

### 10.2 vLLM: new rows, by area

Paths are under `external/competitors/vllm/` at ceb01b6ddf.

#### Norm

- [ ] **`norm_quant` fp8 / group / NVFP4 schemes**: norm and activation quantize in one pass, three in-block phases (rms, absmax, quantize), transposed scales for the GEMM. Copy from `external/competitors/vllm/csrc/quantization/fused_kernels/fused_layernorm_dynamic_per_token_quant.cu` · `rms_norm_per_block_quant_kernel(`. Serves: every fp8 / fp4 checkpoint, D+P. Ours today: int8 only. Ticket: PARITY-QUANT-SCHEMES.
- [ ] **`norm_weight` reference kernel**: one block a row, 128-bit vectors, strided 3D/4D q/k views, block 1024 below 256 tokens. Copy from `external/competitors/vllm/csrc/layernorm_kernels.cu` · `__global__ void rms_norm_kernel(`. Serves: all. Ours today: compiled. Ticket: SETUP-4 A/B row.
- [ ] **`add_norm` reference kernel**: packed half2 add and sum of squares, residual written in place. Copy from `external/competitors/vllm/csrc/layernorm_kernels.cu` · `Generic fused_add_rms_norm_kernel`. Serves: all. Ours today: compiled. Ticket: SETUP-4 A/B row.
- [ ] **`allreduce_sumsq_norm`**: all-reduce only the per-row sum of squares, then normalize locally. Copy from `external/competitors/vllm/csrc/minimax_reduce_rms_kernel.cu` · `minimax_reduce_rms_kernel_lamport(`. Serves: tensor parallel. Ours today: gap. Ticket: PARITY-ALLREDUCE-NORM.
- [ ] **`allreduce_norm_moe_finalize`**: all-reduce, post-norm, residual, next norm and the expert top-k finalize. Copy from `external/competitors/vllm/csrc/beam/allreduce_norm.cu` · `finishTwoshot(`. Serves: tensor parallel. Ours today: gap. Ticket: PARITY-ALLREDUCE-NORM.
- [ ] **`attn_residual_norm`**: online softmax over block residuals, residual, RMSNorm. Copy from `external/competitors/vllm/csrc/kimi_k3/attn_res_kernel.cu` · `attn_res_fwd_online_v2_kernel(`. Serves: Kimi-K3 family. Ours today: gap. Ticket: PARITY-ATTN-RESIDUAL-NORM.
- [ ] **`dual_norm`**: two independent norms in one kernel. Copy from `external/competitors/vllm/vllm/compilation/passes/fusion/rocm_aiter_fusion.py` · `class MLADualRMSNormPattern`. Serves: MLA models with a query LoRA. Ours today: gap. Ticket: PARITY-DUAL-NORM.
- [ ] **`recompute_fanout`**: a cheap fused producer duplicated into each consumer instead of materialized. Copy from `external/competitors/vllm/vllm/compilation/passes/fusion/rocm_aiter_fusion.py` · `class DoubleAiterRMSFp8GroupQuantPattern`. Serves: any fan-out. Ours today: gap. Ticket: PARITY-RECOMPUTE-FANOUT.
- [ ] **`gated_norm`**: the GDN output's gated RMSNorm. Copy from `external/competitors/vllm/vllm/third_party/flash_linear_attention/ops/fused_norm_gate.py` · `def rms_norm_gated(`. Serves: qwen35. Ours today: compiled (`lk_delta_norm` with a quantize epilogue). Ticket: LEDGER-1 row.
- [x] **`embed_norm`**: the embedding gather and the first layer's RMSNorm in one kernel. Copy from `external/competitors/vllm/vllm/model_executor/layers/fused_embed_norm.py` · `def fused_embed_norm(`. Serves: every model, D. Ours today: compiled on the CUDA lane (receipt: `lane_compile_lock::the_embedding_gather_norms_its_row_in_the_same_launch`). Ticket: PARITY-EMBED-NORM.
- [ ] **`norm_weight_offset`**: Gemma's (1 + w) folded into the kernel. Copy from `external/competitors/vllm/vllm/model_executor/layers/layernorm.py` · `class GemmaRMSNorm(`. Serves: Gemma. Ours today: depends on whether the checkpoint stores w + 1. Ticket: PARITY-NORM-WEIGHT-OFFSET.
- [ ] **`affine_input_fold`**: (x·rescale − mean)/std as one per-channel affine. Copy from `external/competitors/vllm/vllm/model_executor/layers/fusion/mm_input_norm.py` · `class FusedMMInputNorm(`. Serves: vision towers. Ours today: gap. Ticket: PARITY-AFFINE-INPUT-FOLD.
- [ ] **`compensated_residual`**: fast2sum-compensated RMSNorm and gated residual. Copy from `external/competitors/vllm/vllm/models/muse_spark/common/ops/fused_rmsnorm.py` · `def _fast2sum`. Serves: precision. Ours today: gap. Ticket: TECH-COMPENSATED-RESIDUAL.

#### Activation

- [ ] **`glu_act_mul` reference kernel**: packed 128-bit (256-bit on sm100) gate·act·up, clamp, alpha, beta. Copy from `external/competitors/vllm/csrc/activation_kernels.cu` · `__global__ void act_and_mul_kernel(`. Serves: all. Ours today: compiled. Ticket: SETUP-4 A/B row.
- [ ] **activation kinds**: swigluoai interleaved, situ, fatrelu, gelu_quick/new/fast, xIELU, sparse GELU. Copy from `external/competitors/vllm/csrc/activation_kernels.cu` · `__global__ void swigluoai_and_mul_kernel(`. Serves: oss, Kimi, Nemotron, Gemma3n. Ours today: silu, gelu, sigmoid, clamped swiglu. Ticket: PARITY-ACTIVATION-KINDS.
- [ ] **`glu_act_mul_quant` fp8 / NVFP4 / MXFP4**: activation and quantize in one pass. Copy from `external/competitors/vllm/csrc/quantization/activation_kernels.cu` · `act_and_mul_quant_kernel(`. Serves: fp8 / fp4 checkpoints. Ours today: int8 only. Ticket: PARITY-QUANT-SCHEMES.
- [ ] **`masked_expert_activation`**: batched-expert activation that skips padded rows, the valid count read on device. Copy from `external/competitors/vllm/csrc/activation_kernels.cu` · `masked_act_and_mul_kernel(`. Serves: batched MoE. Ours today: gap. Ticket: PARITY-MASKED-EXPERT-ACTIVATION.
- [ ] **`softcap` (logits)**: cap·tanh(x/cap), the vocab row split across blocks at batch 1. Copy from `external/competitors/vllm/csrc/muse_spark_mul_tanh_soft_cap_kernel.cu` · `muse_spark_mul_tanh_soft_cap_kernel(`. Serves: gemma2. Ours today: gap. Ticket: PARITY-SOFTCAP.
- [ ] **`relu_sqr` + fp8**: relu², rounded where the standalone kernel rounds, then quantized. Copy from `external/competitors/vllm/vllm/model_executor/layers/fusion/relu2_fp8_quant.py` · `def relu_squared_static_fp8_quant(`. Serves: Nemotron. Ours today: gap. Ticket: PARITY-RELU-SQR.

#### RoPE and KV insert

- [ ] **`rope_qk_one_launch`**: query and key rotated in one launch, strided over qkv views. Copy from `external/competitors/vllm/csrc/pos_encoding_kernels.cu` · `rotary_embedding_kernel(`. Serves: all. Ours today: check. Ticket: PARITY-ROPE-QK-ONE-LAUNCH.
- [ ] **`qk_norm_rope` reference kernel**: a warp per (token, head), shuffle partner exchange, several heads a warp with cos/sin in smem, PDL at ≤ 16 tokens. Copy from `external/competitors/vllm/csrc/fused_qknorm_rope_kernel.cu` · `fusedQKNormRopeKernel(`. Serves: qwen3, qwen35. Ours today: compiled. Ticket: SETUP-4 A/B row.
- [ ] **`qk_norm_rope_kv_append` reference kernel**: norm, partial rotary and paged insert with an fp8 cache. Copy from `external/competitors/vllm/csrc/fused_qknorm_rope_cache_kernel.cu` · `qknorm_rope_cache_kernel(`. Serves: qwen3. Ours today: compiled (f32 cache). Ticket: SETUP-4; PARITY-KV-CACHE-QUANT.
- [ ] **`qk_norm_rope_gate_split`**: split, Gemma RMSNorm of q and k, partial NeoX rotation and the gate chunk copy in one launch. Copy from `external/competitors/vllm/vllm/model_executor/layers/fused_qk_norm_rope.py` · `def fused_qk_rmsnorm_rope_gate(`. Serves: qwen35, D+P. Ours today: check against `head_norm_rope` plus the gate. Ticket: PARITY-QK-NORM-ROPE-GATE-SPLIT.
- [ ] **`horizontal_norm_rope_insert`**: one kernel over a multi-branch row [q, k, v, index q, index k]. Copy from `external/competitors/vllm/csrc/fused_minimax_m3_qknorm_rope_kv_insert_kernel.cu` · `fusedMiniMaxM3QNormRopeKVInsertKernel(`. Serves: MiniMax-M3 shape. Ours today: gap. Ticket: PARITY-HORIZONTAL-NORM-ROPE-INSERT.
- [ ] **`mla` rope + cache reference**: rotate q_pe and k_pe in place and write the latent row. Copy from `external/competitors/vllm/csrc/cache_kernels_fused.cu` · `concat_and_cache_mla_rope_fused_kernel(`. Serves: ds2. Ours today: compiled. Ticket: SETUP-4 A/B row.
- [ ] **rope variants**: llama3 smoothing, longrope, NTK and dynamic NTK, YaRN, mrope / IMRoPE, dual chunk, FoPE. Copy from `external/competitors/vllm/vllm/model_executor/layers/rotary_embedding/llama3_rope.py` · `low_freq_wavelen = self.orig_max_position / self.low_freq_factor`. Serves: Llama 3.x, Phi, Qwen-VL. Ours today: NEOX, partial, sections, YaRN magnitude. Ticket: PARITY-ROPE-VARIANTS.

#### KV cache

- [ ] **`kv_cache_quant`**: quantize on insert, dequantize inside attention; fp8 e4m3/e5m2 with per-head scales, per-token-head int4 with a rotation, NVFP4 "4over6". Copy from `external/competitors/vllm/csrc/cache_kernels.cu` · `reshape_and_cache_flash_kernel(`. Serves: long context, D. Ours today: gap (f32 caches). Ticket: PARITY-KV-CACHE-QUANT.
- [ ] **`hadamard_rotation`**: a Hadamard transform as tensor-core MMAs. Copy from `external/competitors/vllm/csrc/quantization/hadamard/hadacore/hadamard_transform_cuda.cu` · `hadamard_transform_kernel(`. Serves: rotated int4/int8 KV and weights. Ours today: the D5005 engine has it. Ticket: PARITY-KV-CACHE-QUANT.
- [ ] **`cross_layer_launch`**: every layer's cache insert in one launch through a device array of pointers. Copy from `external/competitors/vllm/csrc/cache_kernels.cu` · `concat_and_cache_mla_grouped_kernel(`. Serves: many layers. Ours today: gap. Ticket: PARITY-CROSS-LAYER-LAUNCH.
- [ ] **`host_resident_kv`**: KV rows read zero-copy from pinned host memory. Copy from `external/competitors/vllm/csrc/cache_kernels.cu` · `cp_gather_and_upconvert_fp8_kv_cache_page(`. Serves: qwen35 27B on 12 GB. Ours today: weights only. Ticket: PARITY-HOST-RESIDENT-KV.

#### Attention

- [ ] **`attn_lse_merge`**: LSE-weighted merge of partial attention, with fp8 output. Copy from `external/competitors/vllm/csrc/attention/merge_attn_states.cu` · `merge_attn_states_kernel(`. Serves: split KV, chunked context. Ours today: folded inside our launches. Ticket: PARITY-ATTN-LSE-MERGE.
- [ ] **`attn_gqa_row_packing`**: a KV group's query heads packed onto the MMA's M rows (BLOCK_M = max(16, next power of two of the group)); 2D vs 3D split-KV. Copy from `external/competitors/vllm/vllm/v1/attention/ops/triton_unified_attention.py` · `def unified_attention(`. Serves: llama, qwen3, gemma, oss; D. Ours today: heads grouped per block, no tensor cores in decode. Ticket: PARITY-ATTN-GQA-ROW-PACKING.
- [ ] **`mixed_batch_attention`**: prefill and decode rows in one launch. Copy from same file · `def kernel_unified_attention(`. Serves: serving. Ours today: gap. Ticket: PARITY-MIXED-BATCH-ATTENTION.
- [ ] **`flash_decode` reference**: stage 1 partial softmax per split with grouped heads and the rope part apart; stage 2 merge. Copy from `external/competitors/vllm/vllm/v1/attention/ops/triton_decode_attention.py` · `def decode_attention_fwd(`. Serves: all. Ours today: compiled. Ticket: SETUP-4 A/B row.
- [ ] **`attn_output_quant` fp8 / NVFP4**: attention writes the quantized output directly. Copy from `external/competitors/vllm/vllm/compilation/passes/fusion/attn_quant_fusion.py` · `class AttnFp8StaticQuantPattern`. Serves: fp8 / fp4. Ours today: int8 only. Ticket: PARITY-QUANT-SCHEMES.
- [ ] **`sparse_attention`**: DSA / MSA / QSA / CSA indexers, block top-k, sparse GQA. Copy from `external/competitors/vllm/vllm/model_executor/layers/sparse_attn_indexer.py` · `def sparse_attn_indexer(`. Serves: DeepSeek V3.2+, GLM-5, MiniMax-M3. Ours today: gap. Ticket: PARITY-SPARSE-ATTENTION.
- [ ] **MLA decode on sm100**: 2-SM UMMA, TMEM, warp roles, persistent, split KV with a reduction kernel. Copy from `external/competitors/vllm/csrc/attention/mla/cutlass_sm100_mla/kernel/sm100_fmha_mla_tma_warpspecialized.hpp` · `Sm100FmhaMlaKernelTmaWarpspecialized`. Serves: ds2 family on sm100. Ours today: gap. Ticket: PARITY-MLA-DATACENTER.

#### MoE

- [ ] **`router_topk` reference**: softmax or sigmoid, correction bias, top-k, renormalize, routed scale, expert-parallel range. Copy from `external/competitors/vllm/csrc/moe/topk_softmax_kernels.cu` · `topkGating(`. Serves: MoE. Ours today: compiled. Ticket: SETUP-4 A/B row.
- [ ] **`grouped_topk`**: group scores, top groups, top-k inside them, renormalize, all in one kernel. Copy from `external/competitors/vllm/csrc/moe/grouped_topk_kernels.cu` · `grouped_topk_fused_kernel(`. Serves: DeepSeek V3, Kimi K2, GLM. Ours today: gap. Ticket: PARITY-GROUPED-TOPK.
- [ ] **`router_gemm_f32`**: small-M fp32 router; token groups read the weights once. Copy from `external/competitors/vllm/csrc/fp32_router_gemm.cu` · `fp32_router_gemm_kernel(`. Serves: MoE routers. Ours today: compiled (`lk_gemm_f32` router split). Ticket: LEDGER-1 row.
- [ ] **`moe_direct_pairs`**: when tokens·k·4 ≤ experts, each block is one (token, expert) pair and the sort is skipped. Copy from `external/competitors/vllm/vllm/model_executor/layers/fused_moe/fused_moe.py` · `def _prepare_expert_assignment(`. Serves: ds2, oss; small batches. Ours today: gap. Ticket: PARITY-MOE-DIRECT-PAIRS.
- [ ] **`moe_gate_up_act` reference**: one Triton GEMM for every expert, rows gathered by sorted ids, epilogue dequant, bias and router weight. Copy from `external/competitors/vllm/vllm/model_executor/layers/fused_moe/fused_moe.py` · `def fused_moe_kernel(`. Serves: MoE. Ours today: compiled. Ticket: SETUP-4 A/B row.
- [ ] **`moe_gather_in_operand`**: the token gather folded into the grouped GEMM's A loads. Copy from `external/competitors/vllm/csrc/moe/marlin_moe_wna16/marlin_template.h` · `reinterpret_cast<const int4*>(sorted_token_ids_ptr)`. Serves: ds2, oss; P. Ours today: separate `lk_moe_gather` launch. Ticket: PARITY-MOE-GATHER-IN-OPERAND.
- [ ] **`moe_deferred_finalize`**: the expert reduction left to the next fused boundary. Copy from `external/competitors/vllm/vllm/model_executor/layers/fused_moe/modular_kernel.py` · `return self.prepare_finalize.supports_deferred_moe_finalize()`. Serves: MoE. Ours today: gap. Ticket: PARITY-MOE-DEFERRED-FINALIZE.
- [ ] **`router_shared_gate_merge`**: router and shared-expert gate as one GEMM. Copy from `external/competitors/vllm/vllm/model_executor/layers/fused_moe/runner/moe_runner.py` · `def _maybe_fuse_gate_weights(`. Serves: Qwen2-MoE, Qwen3-Next. Ours today: gap. Ticket: PARITY-ROUTER-SHARED-GATE-MERGE.
- [ ] **MoE tuning tables**: 338 tables keyed by (experts, N, device, dtype, block) mapping batch size to tiles. Copy from `external/competitors/vllm/vllm/model_executor/layers/fused_moe/fused_moe.py` · `def get_moe_configs(`. Serves: MoE. Ours today: none. Ticket: OPT-7.
- [ ] **`split_precision_mma` router**: an fp32-exact router through three bf16 planes on tensor cores. Copy from `external/competitors/vllm/vllm/model_executor/layers/fused_moe/router/gate_linear.py` · `class GateLinear(`. Serves: fp32 routers. Ours today: int8 two-word is the same idea. Ticket: TECH-SPLIT-PRECISION-MMA.

#### Dense and quantized GEMM

- [ ] **`small_m_gemm`**: 1–16-row GEMM: A/B swapped, `mma.sync` fed by an mbarrier `cp.async` pipeline, 4-warp split-K inside the CTA, PDL. Copy from `external/competitors/vllm/csrc/dsv3_fused_a_gemm.cu` · `fused_a_gemm_kernel(`. Serves: decode with MTP or batches. Ours today: GEMV up to 16 rows. Ticket: PARITY-SMALL-M-GEMM.
- [ ] **`gemm_scale`**: scales and bias in the epilogue, broadcast from device pointers (no host sync), int8 zero point. Copy from `external/competitors/vllm/csrc/cutlass_extensions/epilogue/scaled_mm_epilogues_c3x.hpp` · `struct ScaledEpilogueBiasAzpToken`. Serves: fp8 / int8 W8A8. Ours today: gap. Ticket: PARITY-GEMM-SCALE.
- [ ] **Marlin**: weight-only and W4A8: LOP3/PRMT magic-number dequant, striped partition with a slice-locked reduce, offline repack. Copy from `external/competitors/vllm/csrc/quantization/marlin/marlin_template.h` · `if (use_fp32_reduce) {`. Serves: GPTQ / AWQ / fp8 / fp4 on Ampere. Ours today: our block-format GEMMs. Ticket: TECH-LOP3-PRMT-DEQUANT, TECH-STREAM-K.
- [ ] **Machete**: Hopper mixed-input GEMM: weights prepacked to the wgmma register layout, register-sourced A operand, stream-K. Copy from `external/competitors/vllm/csrc/quantization/machete/machete_mainloop.cuh` · `MacheteCollectiveMma`. Serves: sm90. Ours today: gap. Ticket: TECH-WGMMA-RS-PREPACKED.
- [ ] **blockwise fp8**: 1×128 activation and 128×128 weight scales in the mainloop. Copy from `external/competitors/vllm/csrc/quantization/w8a8/cutlass/c3x/scaled_mm_blockwise_sm90_fp8_dispatch.cuh` · `struct cutlass_3x_gemm_fp8_blockwise`. Serves: DeepSeek V3 fp8. Ours today: gap. Ticket: PARITY-QUANT-GEMMS-DATACENTER.
- [ ] **NVFP4 GEMM**: E2M1 with 16-element E4M3 scales, block-scaled tensor op, 2-SM cluster. Copy from `external/competitors/vllm/csrc/quantization/fp4/nvfp4_scaled_mm_kernels.cu` · `struct sm100_fp4_config_default`. Serves: sm100. Ours today: gap. Ticket: PARITY-QUANT-GEMMS-DATACENTER.

#### Recurrent

- [ ] **`gdn_decode_whole`**: gates, q/k L2 norm, delta update, multi-token MTP, gated RMSNorm output: one kernel. Copy from `external/competitors/vllm/csrc/gdn/fused_gdn_decode_kernel.cu` · `gdn_decode_post_conv_mtp_kernel(`. Serves: qwen35, D. Ours today: partial (`lk_gates`, `lk_delta_step`, `lk_delta_norm`). Ticket: PARITY-GDN-DECODE-WHOLE.
- [ ] **`gdn_post_conv_prep`**: after the conv, one kernel writes q, k, v, g, β: split, rearrange, two L2 norms, gating. Copy from `external/competitors/vllm/vllm/third_party/flash_linear_attention/ops/fused_gdn_prefill_post_conv.py` · `def fused_post_conv_prep(`. Serves: qwen35, P. Ours today: partial (`conv_silu_l2`, `gates`). Ticket: PARITY-GDN-POST-CONV-PREP.
- [ ] **GDN chunked prefill reference**: cumsum, β·KKᵀ with decay, a 16×16 block triangular solve (ieee / tf32 / tf32x3), WY recompute, inter-chunk state, output. Copy from `external/competitors/vllm/vllm/third_party/flash_linear_attention/ops/chunk.py` · `def chunk_gated_delta_rule_fwd(`. Serves: qwen35, P. Ours today: compiled (`lk_delta_prep`, `lk_delta_state`). Ticket: SETUP-4 A/B row.
- [ ] **packed GDN decode reference**: q, k, v read from the packed conv output; gates computed in-kernel. Copy from `external/competitors/vllm/vllm/third_party/flash_linear_attention/ops/fused_recurrent.py` · `def fused_recurrent_gated_delta_rule_packed_decode(`. Serves: qwen35, D. Ours today: partial. Ticket: PARITY-GDN-DECODE-WHOLE.
- [ ] **`selective_scan`**: Mamba1 scan with softplus(Δ + bias), D skip and z gate. Copy from `external/competitors/vllm/csrc/mamba/selective_scan_fwd.cu` · `selective_scan_fwd_kernel(`. Serves: Mamba, Jamba. Ours today: the language cannot write it. Ticket: PARITY-SELECTIVE-SCAN.
- [ ] **`ssd_chunk_scan`**: Mamba2 chunked state-space scan. Copy from `external/competitors/vllm/vllm/model_executor/layers/mamba/ops/ssd_combined.py` · `def mamba_chunk_scan_combined_varlen(`. Serves: Mamba2 family. Ours today: the language cannot write it. Ticket: PARITY-SSD-CHUNK-SCAN.
- [ ] **`kda`**: Kimi delta attention: per-dimension decay, fused gate, chunk and recurrent forms. Copy from `external/competitors/vllm/vllm/third_party/flash_linear_attention/ops/kda.py` · `def chunk_kda_with_fused_gate(`. Serves: Kimi Linear, Kimi K3. Ours today: the language cannot write it. Ticket: PARITY-KDA.
- [ ] **`lightning_attention`**: block linear attention with decay slopes. Copy from `external/competitors/vllm/vllm/model_executor/layers/lightning_attn.py` · `def lightning_attention(`. Serves: MiniMax-Text-01. Ours today: the language cannot write it. Ticket: PARITY-LIGHTNING-ATTENTION.
- [ ] **`short_conv`**: B·x, causal conv, C·out. Copy from `external/competitors/vllm/vllm/model_executor/layers/mamba/short_conv.py` · `class ShortConv(`. Serves: LFM2. Ours today: the language cannot write it. Ticket: PARITY-SHORT-CONV.
- [ ] **`stochastic_rounding_state`**: fp16 state stored with `cvt.rs.f16x2.f32` and Philox bits. Copy from `external/competitors/vllm/vllm/model_executor/layers/mamba/ops/mamba_ssm.py` · `def convert_rs_fp16x2`. Serves: half-precision state. Ours today: gap. Ticket: TECH-STOCHASTIC-ROUNDING-STATE.

#### Sampling

- [ ] **`sampling_topk_topp`**: pivot-based top-k / top-p truncation (Gaussian σ seeds, split-row top-p with a ladder search). Copy from `external/competitors/vllm/vllm/v1/sample/ops/topk_topp_triton.py` · `def apply_top_k_top_p_triton(`. Serves: all. Ours today: arg-max only. Ticket: PARITY-SAMPLING.
- [ ] **exponential-race sample**: argmax(p / Exp(1)) with per-request generators. Copy from `external/competitors/vllm/vllm/v1/sample/ops/topk_topp_sampler.py` · `class TopKTopPSampler(`. Serves: all. Ours today: gap. Ticket: PARITY-SAMPLING.
- [ ] **`sampling_penalties`**: repetition, frequency and presence penalties in one kernel. Copy from `external/competitors/vllm/vllm/v1/worker/gpu/sample/penalties.py` · `def _penalties_kernel(`. Serves: all. Ours today: gap. Ticket: PARITY-SAMPLING.
- [ ] **`sampling_gumbel_counter_rng`**: temperature, a counter RNG on (seed, position), Gumbel, block arg-max. Copy from `external/competitors/vllm/vllm/v1/worker/gpu/sample/gumbel.py` · `def _gumbel_sample_kernel(`. Serves: reproducible sampling. Ours today: gap. Ticket: PARITY-SAMPLING.
- [ ] **`sampling_logprobs`**: fused top-k log-softmax; ranks by count-greater. Copy from `external/competitors/vllm/vllm/v1/worker/gpu/sample/logprob.py` · `def _topk_log_softmax_kernel(`. Serves: all. Ours today: gap. Ticket: PARITY-SAMPLING.
- [ ] **`sampling_grammar_bitmask`**: packed bitmask applied to logits rows. Copy from `external/competitors/vllm/vllm/v1/worker/gpu/structured_outputs.py` · `def _apply_grammar_bitmask_kernel(`. Serves: structured output. Ours today: gap. Ticket: PARITY-SAMPLING.

#### Communication, embedding, whole-layer kernels

- [ ] **custom all-reduce**: IPC peer-to-peer one-shot and two-shot, release/acquire flags, graph-registered buffers. Copy from `external/competitors/vllm/csrc/custom_all_reduce.cuh` · `cross_device_reduce_1stage(`. Serves: tensor parallel. Ours today: gap. Ticket: PARITY-ALLREDUCE-NORM.
- [ ] **Lamport two-shot**: flagless all-reduce with a -0.0 sentinel and triple buffers. Copy from `external/competitors/vllm/csrc/mnnvl_lamport.cuh` · `twoshotAllreduceKernel(`. Serves: NVSwitch. Ours today: gap. Ticket: PARITY-ALLREDUCE-NORM.
- [ ] **`dequant_on_gather`**: only the gathered embedding rows are unpacked. Copy from `external/competitors/vllm/vllm/model_executor/layers/quantization/compressed_tensors/compressed_tensors_embedding.py` · `def _dequant_gather_kernel(`. Serves: all. Ours today: compiled (`lk_gather_blk`, `lk_gather_q8`). Ticket: LEDGER-1 row.
- [ ] **vocab-parallel embedding**: masked gather that writes zeros for rows the rank does not own. Copy from `external/competitors/vllm/csrc/vocab_parallel_embedding_kernels.cu` · `vocab_parallel_embedding_kernel(`. Serves: tensor parallel. Ours today: gap. Ticket: PARITY-ALLREDUCE-NORM.
- [ ] **`persistent_layer_megakernel`**: a decoder layer as two persistent launches with in-kernel all-reduce. Copy from `external/competitors/vllm/vllm/models/deepseek_v4/amd/mega_moe.py` · `def _append_shared_expert_kernel`. Serves: decode. Ours today: gap. Ticket: PARITY-PERSISTENT-LAYER-MEGAKERNEL.

#### vLLM's IR, graph and runtime machinery

- [ ] **IR ops with providers**: an op defined by its native function; providers chosen by priority and `supports_args`. Copy from `external/competitors/vllm/vllm/ir/op.py` · `class IrOp:`. Ours today: the shim design. Ticket: OPT-2.
- [ ] **lowering after fusion**: each remaining IR op lowered to its provider. Copy from `external/competitors/vllm/vllm/compilation/passes/ir/lowering_pass.py` · `class VllmIRLoweringPass`. Ours today: the link design. Ticket: OPT-2.
- [ ] **quantized-activation contract**: a producer emits (data, scale, key); the consumer advertises the key it reads. Copy from `external/competitors/vllm/vllm/model_executor/layers/fusion/quant_activation.py` · `class QuantizedActivation:`. Ours today: `fused_quant`, `quant_in_place`. Ticket: OPT-6.
- [ ] **no-op elimination**: collapses reshape chains and no-op slices. Copy from `external/competitors/vllm/vllm/compilation/passes/utility/noop_elimination.py` · `class NoOpEliminationPass`. Ours today: views. Ticket: GRAPH-CANONICALIZE.
- [ ] **in-place restoration**: functionalized in-place ops turned back into in-place writes. Copy from `external/competitors/vllm/vllm/compilation/passes/utility/fix_functionalization.py` · `class FixFunctionalizationPass`. Ours today: row and unit destinations. Ticket: GRAPH-IN-PLACE.
- [ ] **CUDA graph modes**: none / piecewise / full / full-decode-only / full-and-piecewise. Copy from `external/competitors/vllm/vllm/config/compilation.py` · `class CUDAGraphMode(`. Ours today: one graph a form. Ticket: GRAPH-CUDA-GRAPH-MODES.
- [ ] **breakable capture**: capture paused around attention and resumed. Copy from `external/competitors/vllm/vllm/compilation/breakable_cudagraph.py` · `class BreakableCUDAGraphCapture`. Ours today: none. Ticket: GRAPH-CUDA-GRAPH-MODES.
- [ ] **async scheduling**: sampled tokens fed back on the GPU, no host round trip. Copy from `external/competitors/vllm/vllm/v1/worker/gpu_model_runner.py` · `class AsyncGPUModelRunnerOutput(`. Ours today: have (`argmax_advance`). Ticket: LEDGER-1 row.
- [ ] **KV layouts**: logical [L, B, H, N, C] with declared stride permutations. Copy from `external/competitors/vllm/vllm/v1/kv_cache_layout.py` · `class KVCacheLayout(`. Ours today: contiguous only. Ticket: SERVE-PAGED-KV.
- [ ] **batch invariance**: fixed reduction order whatever the batch. Copy from `external/competitors/vllm/vllm/model_executor/determinism/batch_invariant.py` · `def matmul_kernel_persistent(`. Ours today: stronger (chunk equals token-at-a-time bit for bit). Ticket: TECH-BATCH-INVARIANCE.

#### Techniques found in vLLM's kernels

- [ ] **256-bit loads and stores, streaming `.cs`**: copy from `external/competitors/vllm/csrc/cuda_vec_utils.cuh` · `ld.global.nc.v8.u32`. Ticket: TECH-256BIT-LOADS.
- [ ] **alignment-aware vectorization**: copy from `external/competitors/vllm/csrc/quantization/vectorization_utils.cuh` · `__device__ inline void vectorize_with_alignment`. Ticket: TECH-ALIGNMENT-AWARE-VECTORIZATION.
- [ ] **per-arch launch bounds**: copy from `external/competitors/vllm/csrc/launch_bounds_utils.h` · `VLLM_BLOCKS_PER_SM`. Ticket: TECH-PER-ARCH-LAUNCH-BOUNDS.
- [ ] **programmatic dependent launch**: copy from `external/competitors/vllm/csrc/fused_qknorm_rope_kernel.cu` · `constexpr int kPDLEnableTokens`. Ticket: TECH-PDL.
- [ ] **LOP3 dequant**: copy from `external/competitors/vllm/csrc/quantization/marlin/dequant.h` · `__device__ inline int lop3`. Ticket: TECH-LOP3-PRMT-DEQUANT.
- [ ] **UVA host views**: copy from `external/competitors/vllm/csrc/cuda_view.cu` · `get_cuda_view_from_cpu_tensor`. Ticket: TECH-UVA-HOST-VIEW.
- [ ] **radix top-k with order-preserving keys**: copy from `external/competitors/vllm/csrc/persistent_topk.cuh` · `histogram_2048_topk(`. Ticket: TECH-RADIX-TOPK-KEYS.
- [ ] **last-arriving-block merge and cluster DSMEM merge**: copy from `external/competitors/vllm/vllm/models/flint/ops/decode_attention.py` · `Split-KV paged decode attention for Flint`. Ticket: TECH-CLUSTER-DSMEM-REDUCE.

### 10.3 Triton: compiler passes, kernels, limits

Paths are under `external/competitors/triton/` at 1ed999d6b2.

#### Compiler passes and analyses

- [ ] **linear layouts**: every register, shared and tensor-memory layout as a linear map over F2 with compose, invertAndCompose, pseudoinvert, direct sum, divide, reshape. Copy from `external/competitors/triton/include/triton/Tools/LinearLayout.h` · `Adam P. Goucher`. Side: shared library (operational). Ticket: TRITON-LINEAR-LAYOUT.
- [ ] **optimal swizzling**: closed-form shared-memory swizzle from layout matrices: max vectorization, fewest bank conflicts. Copy from `external/competitors/triton/lib/Tools/GenericSwizzling.cpp` · `static std::optional<SmallVector<int32_t>> optimalSwizzling`. Side: CUDA path. Ticket: TRITON-GENERIC-SWIZZLE.
- [ ] **layout conversion by warp shuffles**: a permutation factored into selp / shfl / selp row operations, no shared memory. Copy from `external/competitors/triton/lib/Conversion/TritonGPUToLLVM/ConvertLayoutOpToLLVM.cpp` · `LogicalResult transferWithinWarp`. Side: CUDA path. Ticket: TRITON-SHUFFLE-CONVERT.
- [ ] **axis info**: per-dimension contiguity, divisibility and constancy over index expressions. Copy from `external/competitors/triton/lib/Analysis/AxisInfo.cpp` · `class MakeRangeOpAxisInfoVisitor`. Side: shared facts (proven, not trusted). Ticket: TRITON-AXIS-FACTS.
- [ ] **combine**: dot plus add into an accumulate; pointer adds folded; broadcast-multiply-sum recognized as a dot. Copy from `external/competitors/triton/lib/Dialect/Triton/Transforms/Combine.cpp` · `bool isZero`. Side: shared (e-graph). Ticket: TRITON-COMBINE-REWRITES.
- [ ] **broadcast sinking**: elementwise(broadcast) becomes broadcast(elementwise). Copy from `external/competitors/triton/lib/Dialect/Triton/Transforms/ReorderBroadcast.cpp` · `struct MoveSplatAfterElementwisePattern`. Side: shared (e-graph). Ticket: TRITON-COMBINE-REWRITES.
- [ ] **loop-aware CSE**: optimistic equality of loop-carried values, proven recursively. Copy from `external/competitors/triton/lib/Dialect/Triton/Transforms/LoopAwareCSE.cpp` · `struct LoopAwareCSE`. Side: shared (e-graph). Ticket: TRITON-COMBINE-REWRITES.
- [ ] **coalescing**: vector width = min(divisibility / element bytes, contiguity, 128 bits), shared across dependent accesses. Copy from `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/Coalesce.cpp` · `struct CoalescePass`. Side: CUDA path. Ticket: TRITON-COALESCE.
- [ ] **async-copy coalescing**: clip the per-thread width to the shared vector so copies stay coalesced. Copy from `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/CoalesceAsyncCopy.cpp` · `struct CoalesceAsyncCopy`. Side: CUDA path. Ticket: TRITON-COALESCE.
- [ ] **layout-conversion removal**: anchors, forward propagation, backward rematerialization by cost, at most 20 rounds. Copy from `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/RemoveLayoutConversions.cpp` · `Hacky resolve, prefer block encoding`. Side: CUDA path (we replace it with a global assignment). Ticket: TRITON-GLOBAL-LAYOUT-ASSIGNMENT.
- [ ] **thread locality**: loop-carried reductions kept thread-local and merged after the loop; gather layouts. Copy from `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/OptimizeThreadLocality.cpp` · `class TritonGPUOptimizeThreadLocality`. Side: CUDA path. Ticket: TRITON-THREAD-LOCALITY.
- [ ] **dot operand transposes**: transposes and reshapes folded into shared-memory addressing. Copy from `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/OptimizeDotOperands.cpp` · `class TritonGPUOptimizeDotOperands`. Side: CUDA path. Ticket: TRITON-TRANSPOSE-AS-ADDRESSING.
- [ ] **data-duplication reduction**: conversions to dot operands staged through shared memory so they can be shared. Copy from `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/ReduceDataDuplication.cpp` · `class TritonGPUReduceDataDuplication`. Side: CUDA path. Ticket: TRITON-SMEM-STAGING-NODES.
- [ ] **instruction reordering**: lowers register pressure. Copy from `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/ReorderInstructions.cpp` · `class TritonGPUReorderInstructions`. Side: CUDA path. Ticket: TRITON-REGISTER-PRESSURE-ORDER.
- [ ] **split-precision dots**: fp32 dot as three TF32 dots or bf16x3 / bf16x6 splits. Copy from `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/F32DotTC.cpp` · `struct F32DotTC`. Side: CUDA path. Ticket: TECH-SPLIT-PRECISION-MMA.
- [ ] **accumulator init**: zero-init replaced by the MMA's use-D flag. Copy from `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/OptimizeAccumulatorInit.cpp` · `class OptimizeAccumulatorInit`. Side: CUDA path. Ticket: TRITON-ACCUMULATOR-INIT.
- [ ] **nested-loop flattening**: a loop nest flattened into one persistent loop. Copy from `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/FuseNestedLoops.cpp` · `struct FuseNestedLoops`. Side: CUDA path. Ticket: TRITON-FUSE-NESTED-LOOPS.
- [ ] **operand prefetch**: next iteration's operand slice loaded at the end of the loop. Copy from `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/Prefetch.cpp` · `class Prefetch`. Side: CUDA path. Ticket: TRITON-OPERAND-PREFETCH.
- [ ] **latency assignment**: loadLatency = (stages − 1) / (indirection + 1). Copy from `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/Pipeliner/AssignLatencies.cpp` · `struct AssignLatencies`. Side: CUDA path. Ticket: TRITON-MODULO-SCHEDULER.
- [ ] **loop scheduling**: a coarse (stage, cluster) schedule; innermost loops, distance ≤ 1. Copy from `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/Pipeliner/ScheduleLoops.cpp` · `struct ScheduleLoops`. Side: CUDA path. Ticket: TRITON-MODULO-SCHEDULER.
- [ ] **loop lowering**: async copies and TMA with mbarrier ring buffers, phase bits, multibuffered accumulators. Copy from `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/Pipeliner/LowerLoops.cpp` · `void createAsyncCopy`. Side: CUDA path. Ticket: TRITON-MODULO-SCHEDULER.
- [ ] **async wgmma**: waits minimized; register operands split along K. Copy from `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/Pipeliner/WGMMAPipeline.cpp` · `static std::optional<int> dotCanBeProperlyAsync`. Side: CUDA path (sm90). Ticket: TRITON-ASYNC-MMA-WAITS.
- [ ] **automatic warp specialization**: partitions scheduled by edge-merge heuristics, async channels lowered to smem rings. Copy from `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/WarpSpecialization/PartitionScheduling.cpp` · `void duplicateCheapOps`. Side: CUDA path. Ticket: TRITON-WARP-SPECIALIZATION.
- [ ] **partition warps**: warps per partition from a rough register estimate. Copy from `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/WarpSpecialization/OptimizePartitionWarps.cpp` · `struct OptimizePartitionWarps`. Side: CUDA path. Ticket: TRITON-WARP-SPECIALIZATION.
- [ ] **MMA selection**: v2 / v3 / v5 / scaled by capability; chained dots tiled so row reductions stay in a warp. Copy from `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/AccelerateMatmul.cpp` · `static int getMMAVersionSafe`. Side: CUDA path. Ticket: TRITON-MMA-SELECTION.
- [ ] **shared-memory allocation**: liveness and interference-graph colouring. Copy from `external/competitors/triton/lib/Analysis/Allocation.cpp` · `class Allocation`. Side: CUDA path. Ticket: TRITON-SMEM-COLORING-ALLOCATOR.
- [ ] **barrier insertion**: memory-region analysis of shared-memory hazards. Copy from `external/competitors/triton/lib/Analysis/Membar.cpp` · `Operation *MembarAnalysis::insertBarrier`. Side: CUDA path. Ticket: TRITON-REGION-BARRIERS.
- [ ] **proxy fences**: generic-to-async proxy fences. Copy from `external/competitors/triton/lib/Dialect/TritonNvidiaGPU/Transforms/FenceInsertion.cpp` · `struct FenceInsertion`. Side: CUDA path. Ticket: TRITON-REGION-BARRIERS.
- [ ] **TMA lowering**: descriptor loads and stores to async TMA with mbarriers. Copy from `external/competitors/triton/lib/Dialect/TritonNvidiaGPU/Transforms/TMALowering.cpp` · `class TritonNvidiaGPUTMALowering`. Side: CUDA path. Ticket: TRITON-TMA.
- [ ] **reductions**: in-thread, then warp shuffles, then shared memory; arity-3 trees. Copy from `external/competitors/triton/lib/Conversion/TritonGPUToLLVM/ReduceOpToLLVM.cpp` · `struct ReduceOpConversion`. Side: CUDA path. Ticket: TRITON-REDUCE-SCAN-CODEGEN.
- [ ] **scans**: in-thread chunks, warp scan on the tails, shared partials. Copy from `external/competitors/triton/lib/Conversion/TritonGPUToLLVM/ScanOpToLLVM.cpp` · `struct ScanOpConversion`. Side: CUDA path. Ticket: TRITON-REDUCE-SCAN-CODEGEN.
- [ ] **autotuning**: runtime benchmarking per key; results not cached by default. Copy from `external/competitors/triton/python/triton/runtime/autotuner.py` · `class Autotuner`. Side: measured database. Ticket: OPT-7.
- [ ] **timing**: 256 MB zeroed before each repetition to flush L2. Copy from `external/competitors/triton/python/triton/testing.py` · `def do_bench(`. Side: measurement method. Ticket: SETUP-4.
- [ ] **specialization**: `% 16` becomes divisibility, `== 1` becomes a constant. Copy from `external/competitors/triton/python/src/specialize.cc` · `std::pair<py::object, py::object> handle_long_type(`. Side: runtime dispatch. Ticket: TRITON-AOT-DISPATCH.

#### Hand-written fused kernels to match, then beat

- [ ] **fused softmax**: copy from `external/competitors/triton/python/tutorials/02-fused-softmax.py` · `def softmax(`; it embodies one pass in SRAM; occupancy computed from register count and shared memory after warmup. Ticket: PARITY-TRITON-SOFTMAX.
- [ ] **layer norm, forward and backward**: copy from `external/competitors/triton/python/tutorials/05-layer-norm.py` · `def _layer_norm_bwd_dx_fused`; it embodies backward partial dW/dB under `atomic_cas` spin locks, then a reduction kernel. Ticket: PARITY-TRITON-LAYERNORM.
- [ ] **fused attention**: copy from `external/competitors/triton/python/tutorials/06-fused-attention.py` · `def _attn_fwd_inner`; it embodies exp2 with the scale folded; causal split into three stages; accumulator halves for the rescale. Ticket: PARITY-TRITON-ATTENTION.
- [ ] **Blackwell attention**: copy from `external/competitors/triton/python/examples/gluon/01-attention-forward.py` · `class AttentionConfig:`; it embodies six warp-specialized partitions, ping-pong softmax groups, P in tensor memory. Ticket: PARITY-TRITON-ATTENTION.
- [ ] **grouped GEMM**: copy from `external/competitors/triton/python/tutorials/08-grouped-gemm.py` · `def grouped_matmul_kernel`; it embodies persistent CTAs walking a static tile schedule across problems. Ticket: PARITY-TRITON-GROUPED-GEMM.
- [ ] **persistent matmul**: copy from `external/competitors/triton/python/tutorials/09-persistent-matmul.py` · `EPILOGUE_SUBTILE = nargs.get(`; it embodies pid swizzle, flatten, warp specialization, epilogue subtiling. Ticket: PARITY-TRITON-PERSISTENT-MATMUL.
- [ ] **block-scaled matmul**: copy from `external/competitors/triton/python/tutorials/10-block-scaled-matmul.py` · `def block_scaled_matmul_kernel(`; it embodies nvfp4 / mxfp4 / mxfp8 with pre-swizzled scales. Ticket: PARITY-QUANT-GEMMS-DATACENTER.
- [ ] **low-memory dropout**: copy from `external/competitors/triton/python/tutorials/04-low-memory-dropout.py` · `tl.rand`; it embodies Philox from (seed, offset), no mask stored. Ticket: PARITY-TRITON-DROPOUT.
- [ ] **MoE matmul**: copy from `external/competitors/triton/python/triton_kernels/triton_kernels/matmul.py` · `def matmul(a, b, bias,`; it embodies ragged expert GEMM: gather on load, scatter on store, split-K, fused activation, quantize and communication epilogues. Ticket: PARITY-TRITON-MOE-MATMUL.
- [ ] **SwiGLU**: copy from `external/competitors/triton/python/triton_kernels/triton_kernels/swiglu.py` · `def swiglu(`; it embodies clamp, flexpoint scales, thread-local absmax. Ticket: PARITY-TRITON-SWIGLU.
- [ ] **top-k and routing**: copy from `external/competitors/triton/python/triton_kernels/triton_kernels/topk.py` · `def topk(`; it embodies (value, index) packed into one integer and sorted; bitmatrix of assignments. Ticket: PARITY-TRITON-TOPK.
- [ ] **MX downcast**: copy from `external/competitors/triton/python/triton_kernels/triton_kernels/numerics_details/mxfp_details/_downcast_to_mxfp.py` · `_downcast_to_mxfp`; it embodies e8m0 scales by bit tricks, e2m1 round-to-nearest-even packing. Ticket: PARITY-TRITON-MX-CONVERT.

#### Triton's documented limits (each one a claim once its task is done)

- [ ] **One kernel per function.** The NVIDIA backend asserts a single entry point (`external/competitors/triton/third_party/nvidia/backend/compiler.py` · `assert len(names) == 1`). Production epilogue fusion is source-string templating. Closed by OPT-1 and the PARITY tasks: graph-level fusion, automatic and proven.
- [ ] **No cross-kernel scheduling.** PDL is placed by hand. Closed by GRAPH-CROSS-KERNEL-OVERLAP.
- [ ] **Autotuning at run time, uncached by default.** Closed by OPT-7.
- [ ] **Coarse specialization.** Only `% 16` and `== 1` are seen; nested-loop flattening needs a user's `tl.assume`. Closed by TRITON-AXIS-FACTS and `facts::range`.
- [ ] **Greedy layout assignment** with fixed costs and a 20-round cap. Closed by TRITON-GLOBAL-LAYOUT-ASSIGNMENT.
- [ ] **Little control over register allocation.** One global maxnreg and ptxas workarounds. Closed by TRITON-WARP-SPECIALIZATION's compiler register budgets.
- [ ] **A restricted pipeliner:** innermost loops only, distance ≤ 1, latency from a formula. Closed by TRITON-MODULO-SCHEDULER.
- [ ] **Restricted warp specialization.** It is automatic on sm100 only; Hopper requires four warps and no else-blocks. Closed by TRITON-WARP-SPECIALIZATION.
- [ ] **Empirical constants,** such as the register-operand split fixed at 2. Closed by OPT-7 measurements.
- [ ] **No proof of correctness.** Hints are trusted. Closed by OPT-5 and TRITON-AXIS-FACTS.
- [ ] **Power-of-two tiles, dot dimensions ≥ 16.** Closed by TRITON-TILE-TIER.
- [ ] **An arch-fragmented pipeline.** Peak kernels arrive first as hand-written Gluon. Closed by the shim design.
- [ ] **Launch and JIT overhead.** Ours is ahead of time.

---

## 11. The coverage map: every vLLM and Triton entry, mapped to the ticket that copies it

Section 10 explains the fusions and tricks worth knowing. This section is the exhaustive list that proves nothing was
missed: every kernel, op, fusion pattern, pass, backend and builtin that vLLM and Triton ship at the pins, each mapped
to the ticket that copies it.

### 11.1 How to read and tick it

- **What is listed.** Each subsection names the rule that extracted it. Section 13.4 holds the extractor itself, and
  LEDGER-3's `sota_contract_lock` re-runs it against the pinned trees.
  - An entry the pins ship with no row turns the lock red, and so does a row whose entry is gone. A new pin therefore
    arrives as a list of red names, not as silent drift.
  - Counts at the pins: 263 registered op names, 86 fusion-pass classes, 8 graph passes, 3 IR ops, 63 attention-backend
    modules, 42 attention-op modules, 68 MoE modules, 97 linear kernels, 294 files holding 821 `@triton.jit` kernels,
    38 speculative-decoding and sampling modules, 134 Triton compiler passes, 116 Triton language builtins, and 82
    Triton tutorials, Gluon examples and `triton_kernels` modules. That is 1,294 rows.
- **Where each row maps.**
  - Each row names the ticket, or tickets, whose work copies it.
  - Hardware-specific entries map to that hardware's shim: ROCm to TRITON-AMD-SHIM, CPU to PARITY-CPU-SHIM, Intel to
    PARITY-XPU-SHIM, and sm_90 or sm_100 to the datacenter tickets.
  - An entry in a per-model tree maps to the shared fusion it embodies. A layer used by only one family maps to
    PARITY-MODEL-SPECIFIC-LAYERS.
- **When a row is ticked.** A row is ticked when its ticket lands our equivalent and the receipt says so:
  - for a kernel, a SETUP-4 row at our shapes at or past the row maximum (section 2.5);
  - for a pass, a pattern or a utility, the equivalent mechanism named in the ticket's receipts.

  A row is never ticked by a refusal. An entry for hardware no shim targets yet stays open until that shim's ticket
  closes it.
- **The inverse view.** The rows mapped to one ticket are that ticket's coverage. The extractor prints them with
  `--by-ticket`, and LEDGER-3 holds every done ticket's rows ticked.

### 11.2 vLLM: registered operators

#### CUDA (168)

Every op name a `.def("…")` in `csrc/` registers, listed under its CUDA registration. An op the CPU backend also registers is listed here once; PARITY-CPU-SHIM covers its CPU form too.

Registered in `external/competitors/vllm/csrc/attention/dcp_utils/dcp_direct_a2a_lse_reduce.cu`:

- [ ] `direct_dcp_a2a_lse_reduce` → PARITY-TENSOR-PARALLEL-IR

Registered in `external/competitors/vllm/csrc/attention/dcp_utils/dcp_direct_kv_gather.cu`:

- [ ] `direct_dcp_kv_gather` → PARITY-TENSOR-PARALLEL-IR

Registered in `external/competitors/vllm/csrc/attention/dcp_utils/dcp_direct_q_gather.cu`:

- [ ] `direct_dcp_q_gather` → PARITY-TENSOR-PARALLEL-IR

Registered in `external/competitors/vllm/csrc/custom_all_gather_reduce_scatter_ops.cpp`:

- [ ] `custom_all_gather` → PARITY-ALLREDUCE-NORM
- [ ] `custom_reduce_scatter` → PARITY-ALLREDUCE-NORM
- [ ] `mnnvl_lamport_all_gather` → PARITY-ALLREDUCE-NORM
- [ ] `mnnvl_lamport_reduce_scatter` → PARITY-ALLREDUCE-NORM
- [ ] `mnnvl_multimem_reduce_scatter` → PARITY-ALLREDUCE-NORM

Registered in `external/competitors/vllm/csrc/flashkda_registration.cpp`:

- [ ] `fwd` → PARITY-KDA
- [ ] `get_workspace_size` → PARITY-KDA

Registered in `external/competitors/vllm/csrc/moe/torch_bindings.cpp`:

- [ ] `batched_moe_align_block_size` → PARITY-MOE-DIRECT-PAIRS
- [ ] `fused_globalize_align_block_size` → PARITY-EXPERT-PARALLEL
- [ ] `grouped_topk` → PARITY-GROUPED-TOPK
- [ ] `moe_align_block_size` → PARITY-MOE-DIRECT-PAIRS
- [ ] `moe_lora_align_block_size` → SERVE-LORA
- [ ] `moe_permute` → PARITY-MOE-GATHER-IN-OPERAND
- [ ] `moe_permute_sort_workspace_size` → PARITY-MOE-GATHER-IN-OPERAND
- [ ] `moe_permute_unpermute_supported` → OPT-2
- [ ] `moe_permute_with_scratch` → PARITY-MOE-GATHER-IN-OPERAND
- [ ] `moe_prepare_scatter` → PARITY-MOE-GATHER-IN-OPERAND
- [ ] `moe_sum` → PARITY-MOE-DEFERRED-FINALIZE
- [ ] `moe_unpermute` → PARITY-MOE-GATHER-IN-OPERAND
- [ ] `moe_wna16_gemm` → PARITY-WEIGHT-FORMATS
- [ ] `moe_wna16_marlin_gemm` → PARITY-WEIGHT-FORMATS
- [ ] `shuffle_rows` → PARITY-MOE-GATHER-IN-OPERAND
- [ ] `topk_sigmoid` → PARITY-ROUTING-VARIANTS
- [ ] `topk_softmax` → PARITY-ROUTING-VARIANTS
- [ ] `topk_softplus_sqrt` → PARITY-ROUTING-VARIANTS

Registered in `external/competitors/vllm/csrc/quantization/fp4/mxfp4_experts_quant.cu`:

- [ ] `mxfp4_experts_quant_supported` → OPT-2

Registered in `external/competitors/vllm/csrc/torch_bindings.cpp`:

- [ ] `all_reduce` → PARITY-ALLREDUCE-NORM
- [ ] `allocate_shared_buffer_and_handle` → PARITY-ALLREDUCE-NORM
- [ ] `apply_repetition_penalties_` → PARITY-SAMPLING
- [ ] `awq_dequantize` → PARITY-WEIGHT-FORMATS
- [ ] `awq_gemm` → PARITY-WEIGHT-FORMATS
- [ ] `awq_marlin_repack` → PARITY-WEIGHT-FORMATS
- [ ] `beam_allreduce` → PARITY-ALLREDUCE-NORM
- [ ] `beam_allreduce_norm` → PARITY-ALLREDUCE-NORM
- [ ] `concat_and_cache_mla` → PARITY-MLA-PROLOGUE
- [ ] `concat_and_cache_mla_grouped` → PARITY-CROSS-LAYER-LAUNCH
- [ ] `concat_and_cache_mla_rope_fused` → PARITY-MLA-PROLOGUE
- [ ] `concat_mla_q` → PARITY-MLA-PROLOGUE
- [ ] `convert_fp8` → PARITY-KV-CACHE-QUANT
- [ ] `cooperative_topk` → PARITY-SPARSE-ATTENTION
- [ ] `cp_gather_and_upconvert_fp8_kv_cache` → PARITY-HOST-RESIDENT-KV
- [ ] `cp_gather_and_upconvert_nvfp4_kv_cache` → PARITY-HOST-RESIDENT-KV
- [ ] `cp_gather_cache` → PARITY-HOST-RESIDENT-KV
- [ ] `cp_gather_indexer_k_quant_cache` → PARITY-SPARSE-ATTENTION
- [ ] `cutlass_encode_and_reorder_int4b` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `cutlass_encode_and_reorder_int4b_grouped` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `cutlass_fp4_group_mm` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `cutlass_group_gemm_supported` → OPT-2
- [ ] `cutlass_moe_mm` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `cutlass_mxfp4_group_mm` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `cutlass_pack_scale_fp8` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `cutlass_scaled_fp4_mm` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `cutlass_scaled_mm` → PARITY-INT8-GEMM-PEAK
- [ ] `cutlass_scaled_mm_azp` → PARITY-INT8-GEMM-PEAK
- [ ] `cutlass_scaled_mm_supports_block_fp8` → OPT-2
- [ ] `cutlass_scaled_mm_supports_fp4` → OPT-2
- [ ] `cutlass_scaled_mm_supports_fp8` → OPT-2
- [ ] `cutlass_w4a8_mm` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `cutlass_w4a8_moe_mm` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `dispose` → PARITY-ALLREDUCE-NORM
- [ ] `dsv3_fused_a_gemm` → PARITY-SMALL-M-GEMM
- [ ] `dynamic_per_token_scaled_fp8_quant` → PARITY-QUANT-SCHEMES
- [ ] `dynamic_scaled_fp8_quant` → PARITY-QUANT-SCHEMES
- [ ] `dynamic_scaled_int8_quant` → PARITY-QUANT-SCHEMES
- [ ] `fatrelu_and_mul` → PARITY-ACTIVATION-KINDS
- [ ] `fp32_router_gemm` → PARITY-GEMV-NORM-EPILOGUE
- [ ] `free_shared_buffer` → PARITY-ALLREDUCE-NORM
- [ ] `fused_add_rms_norm` → PARITY-GEMV-NORM-EPILOGUE
- [ ] `fused_add_rms_norm_static_fp8_quant` → PARITY-QUANT-SCHEMES
- [ ] `fused_deepseek_v4_kv_rope_insert` → PARITY-SPARSE-ATTENTION
- [ ] `fused_deepseek_v4_qnorm_rope_kv_rope_full_cache_bf16_insert` → PARITY-SPARSE-ATTENTION
- [ ] `fused_deepseek_v4_qnorm_rope_kv_rope_full_cache_fp8_insert` → PARITY-SPARSE-ATTENTION, PARITY-KV-CACHE-QUANT
- [ ] `fused_deepseek_v4_qnorm_rope_kv_rope_quant_insert` → PARITY-SPARSE-ATTENTION, PARITY-KV-CACHE-QUANT
- [ ] `fused_gdn_decode_post_conv_mtp` → PARITY-GDN-DECODE-WHOLE
- [ ] `fused_kda_chunk` → PARITY-KDA
- [ ] `fused_kda_decode` → PARITY-KDA
- [ ] `fused_kda_prologue` → PARITY-KDA
- [ ] `fused_kimi_k3_mla_decode_q_concat_ds_mla_insert` → PARITY-MLA-PROLOGUE
- [ ] `fused_kimi_k3_mla_decode_q_concat_kv_cache_fp8_insert` → PARITY-MLA-PROLOGUE, PARITY-KV-CACHE-QUANT
- [ ] `fused_kimi_k3_mla_decode_q_concat_kv_cache_insert` → PARITY-MLA-PROLOGUE
- [ ] `fused_kimi_k3_mla_key_concat_ds_mla_insert` → PARITY-MLA-PROLOGUE
- [ ] `fused_kimi_k3_mla_key_concat_kv_cache_insert` → PARITY-MLA-PROLOGUE
- [ ] `fused_kimi_k3_mla_kv_concat` → PARITY-MLA-PROLOGUE
- [ ] `fused_kimi_k3_mla_kv_concat_quant_fp8` → PARITY-MLA-PROLOGUE, PARITY-KV-CACHE-QUANT
- [ ] `fused_kimi_k3_mla_qkv_quant_kv_cache_fp8_insert` → PARITY-MLA-PROLOGUE, PARITY-KV-CACHE-QUANT
- [ ] `fused_minimax_m3_qknorm_rope_kv_insert` → PARITY-HORIZONTAL-NORM-ROPE-INSERT
- [ ] `fused_muse_spark_qknorm_rope_kv_insert` → PARITY-QKV-HEAD-EPILOGUE
- [ ] `fused_qk_norm_rope` → PARITY-QKV-HEAD-EPILOGUE
- [ ] `fused_qknorm_rope_cache` → PARITY-QKV-HEAD-EPILOGUE
- [ ] `gather_and_maybe_dequant_cache` → SERVE-CHUNKED-PREFILL
- [ ] `gelu_and_mul` → PARITY-ACTIVATION-KINDS
- [ ] `gelu_fast` → PARITY-ACTIVATION-KINDS
- [ ] `gelu_new` → PARITY-ACTIVATION-KINDS
- [ ] `gelu_quick` → PARITY-ACTIVATION-KINDS
- [ ] `gelu_tanh_and_mul` → PARITY-ACTIVATION-KINDS
- [ ] `get_cuda_view_from_cpu_tensor` → TECH-UVA-HOST-VIEW
- [ ] `get_cutlass_batched_moe_mm_data` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `get_cutlass_moe_mm_data` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `get_cutlass_moe_mm_problem_sizes_from_expert_offsets` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `get_device_attribute` → OPT-2
- [ ] `get_graph_buffer_ipc_meta` → PARITY-ALLREDUCE-NORM
- [ ] `get_max_shared_memory_per_block_device_attribute` → OPT-2
- [ ] `gptq_gemm` → PARITY-WEIGHT-FORMATS
- [ ] `gptq_marlin_repack` → PARITY-WEIGHT-FORMATS
- [ ] `gptq_shuffle` → PARITY-WEIGHT-FORMATS
- [ ] `hadacore_transform` → PARITY-KV-CACHE-QUANT
- [ ] `hisparse_gather_compact` → PARITY-SPARSE-ATTENTION
- [ ] `hisparse_gather_plan` → PARITY-SPARSE-ATTENTION
- [ ] `hisparse_invalidate_written_slots` → PARITY-SPARSE-ATTENTION
- [ ] `hisparse_resolve_residency` → PARITY-SPARSE-ATTENTION
- [ ] `indexer_k_quant_and_cache` → PARITY-SPARSE-ATTENTION
- [ ] `init_custom_ar` → PARITY-ALLREDUCE-NORM
- [ ] `kimi_k3_attn_res` → PARITY-ATTN-RESIDUAL-NORM
- [ ] `machete_mm` → TECH-WGMMA-RS-PREPACKED
- [ ] `machete_prepack_B` → TECH-WGMMA-RS-PREPACKED
- [ ] `machete_supported_schedules` → TECH-WGMMA-RS-PREPACKED
- [ ] `marlin_gemm` → PARITY-WEIGHT-FORMATS
- [ ] `marlin_int4_fp8_preprocess` → PARITY-WEIGHT-FORMATS
- [ ] `masked_moe_activation` → PARITY-MASKED-EXPERT-ACTIVATION
- [ ] `merge_attn_states` → PARITY-ATTN-LSE-MERGE
- [ ] `meta_size` → PARITY-ALLREDUCE-NORM
- [ ] `minimax_allreduce_rms_qk` → PARITY-ALLREDUCE-NORM
- [ ] `mul_and_silu` → PARITY-ACTIVATION-KINDS
- [ ] `muse_spark_mul_tanh_soft_cap` → PARITY-SOFTCAP
- [ ] `mxfp4_experts_quant` → PARITY-QUANT-SCHEMES
- [ ] `ngram_compute_n_gram_ids` → PARITY-MODEL-SPECIFIC-LAYERS
- [ ] `open_mem_handle` → PARITY-ALLREDUCE-NORM
- [ ] `per_token_group_fp8_quant` → PARITY-QUANT-SCHEMES
- [ ] `per_token_group_fp8_quant_packed` → PARITY-QUANT-SCHEMES
- [ ] `per_token_group_quant_int8` → PARITY-QUANT-SCHEMES
- [ ] `persistent_masked_m_silu_mul_quant` → PARITY-MASKED-EXPERT-ACTIVATION
- [ ] `persistent_topk` → PARITY-SPARSE-ATTENTION
- [ ] `register_buffer` → PARITY-ALLREDUCE-NORM
- [ ] `register_graph_buffers` → PARITY-ALLREDUCE-NORM
- [ ] `relu_squared` → PARITY-RELU-SQR
- [ ] `reshape_and_cache` → PARITY-KV-CACHE-QUANT
- [ ] `reshape_and_cache_flash` → PARITY-KV-CACHE-QUANT
- [ ] `rms_norm` → PARITY-GEMV-NORM-EPILOGUE
- [ ] `rms_norm_dynamic_per_token_quant` → PARITY-QUANT-SCHEMES
- [ ] `rms_norm_per_block_quant` → PARITY-QUANT-SCHEMES
- [ ] `rms_norm_static_fp8_quant` → PARITY-QUANT-SCHEMES
- [ ] `rotary_embedding` → PARITY-ROPE-QK-ONE-LAUNCH
- [ ] `scaled_fp4_experts_quant` → PARITY-QUANT-SCHEMES
- [ ] `scaled_fp4_quant` → PARITY-QUANT-SCHEMES
- [ ] `selective_scan_fwd` → PARITY-SELECTIVE-SCAN
- [ ] `silu_and_mul` → PARITY-ACTIVATION-KINDS
- [ ] `silu_and_mul_mxfp4_experts_quant` → PARITY-QUANT-SCHEMES
- [ ] `silu_and_mul_nvfp4_quant` → PARITY-QUANT-SCHEMES
- [ ] `silu_and_mul_per_block_quant` → PARITY-QUANT-SCHEMES
- [ ] `silu_and_mul_quant` → PARITY-QUANT-SCHEMES
- [ ] `silu_and_mul_scaled_fp4_experts_quant` → PARITY-QUANT-SCHEMES
- [ ] `silu_and_mul_with_clamp` → PARITY-ACTIVATION-KINDS
- [ ] `situ_and_mul` → PARITY-ACTIVATION-KINDS
- [ ] `situ_and_mul_quant` → PARITY-QUANT-SCHEMES
- [ ] `sm100_cutlass_mla_decode` → PARITY-MLA-DATACENTER
- [ ] `sm100_cutlass_mla_get_workspace_size` → PARITY-MLA-DATACENTER
- [ ] `static_scaled_fp8_quant` → PARITY-QUANT-SCHEMES
- [ ] `static_scaled_int8_quant` → PARITY-QUANT-SCHEMES
- [ ] `swap_blocks` → SERVE-KV-OFFLOAD
- [ ] `swap_blocks_batch` → SERVE-KV-OFFLOAD
- [ ] `swigluoai_and_mul` → PARITY-ACTIVATION-KINDS
- [ ] `top_k_per_row_decode` → PARITY-SPARSE-ATTENTION
- [ ] `top_k_per_row_prefill` → PARITY-SPARSE-ATTENTION
- [ ] `vocab_parallel_embedding` → PARITY-TENSOR-PARALLEL-IR
- [ ] `weak_ref_tensor` → GRAPH-CUDA-GRAPH-MODES

#### ROCm only (16)

Op names only the ROCm bindings register.

Registered in `external/competitors/vllm/csrc/rocm/quickreduce/torch_bindings.cpp`:

- [ ] `init_custom_qr` → TRITON-AMD-SHIM
- [ ] `qr_all_reduce` → TRITON-AMD-SHIM
- [ ] `qr_destroy` → TRITON-AMD-SHIM
- [ ] `qr_get_handle` → TRITON-AMD-SHIM
- [ ] `qr_max_size` → TRITON-AMD-SHIM
- [ ] `qr_open_handles` → TRITON-AMD-SHIM

Registered in `external/competitors/vllm/csrc/rocm/torch_bindings.cpp`:

- [ ] `LLMM1` → TRITON-AMD-SHIM
- [ ] `gptq_gemm_rdna3` → TRITON-AMD-SHIM
- [ ] `gptq_gemm_rdna3_wmma` → TRITON-AMD-SHIM
- [ ] `moe_gptq_gemm_rdna3` → TRITON-AMD-SHIM
- [ ] `mxfp4_w4a8_gemv_rdna3` → TRITON-AMD-SHIM
- [ ] `paged_attention` → TRITON-AMD-SHIM
- [ ] `wvSplitK` → TRITON-AMD-SHIM
- [ ] `wvSplitKQ` → TRITON-AMD-SHIM
- [ ] `wvSplitK_int4_g` → TRITON-AMD-SHIM
- [ ] `wvSplitKrc` → TRITON-AMD-SHIM

#### CPU only (79)

Op names only the CPU bindings register.

Registered in `external/competitors/vllm/csrc/cpu/torch_bindings.cpp`:

- [ ] `activation_lut_bf16` → PARITY-CPU-SHIM
- [ ] `biased_topk_cpu` → PARITY-CPU-SHIM
- [ ] `bmm_cpu` → PARITY-CPU-SHIM
- [ ] `causal_conv1d_fwd_cpu` → PARITY-CPU-SHIM
- [ ] `causal_conv1d_update_cpu` → PARITY-CPU-SHIM
- [ ] `causal_conv1d_update_cpu_vec` → PARITY-CPU-SHIM
- [ ] `causal_conv1d_weight_pack` → PARITY-CPU-SHIM
- [ ] `chunk_gated_delta_rule_cpu` → PARITY-CPU-SHIM
- [ ] `compress_norm_rope_store_cpu` → PARITY-CPU-SHIM
- [ ] `compress_norm_rope_store_indexer_cpu` → PARITY-CPU-SHIM
- [ ] `compute_slot_mapping_kernel_impl` → PARITY-CPU-SHIM
- [ ] `concat_and_cache_mla_cpu` → PARITY-CPU-SHIM
- [ ] `convert_scale_packed` → PARITY-CPU-SHIM
- [ ] `convert_weight_packed` → PARITY-CPU-SHIM
- [ ] `convert_weight_packed_scale_zp` → PARITY-CPU-SHIM
- [ ] `copy_and_expand_dflash_inputs_kernel_impl` → PARITY-CPU-SHIM
- [ ] `copy_and_expand_eagle_inputs_kernel_impl` → PARITY-CPU-SHIM
- [ ] `cpu_attention_with_kv_cache` → PARITY-CPU-SHIM
- [ ] `cpu_attn_has_isa` → PARITY-CPU-SHIM
- [ ] `cpu_attn_reshape_and_cache` → PARITY-CPU-SHIM
- [ ] `cpu_fused_moe` → PARITY-CPU-SHIM
- [ ] `cpu_fused_moe_int8` → PARITY-CPU-SHIM
- [ ] `cpu_gemm_wna16` → PARITY-CPU-SHIM
- [ ] `cpu_has_amx_fp8` → PARITY-CPU-SHIM
- [ ] `create_onednn_mm_handler` → PARITY-CPU-SHIM
- [ ] `create_onednn_scaled_mm_handler` → PARITY-CPU-SHIM
- [ ] `decode_attention_cpu` → PARITY-CPU-SHIM
- [ ] `dynamic_4bit_int_moe` → PARITY-CPU-SHIM
- [ ] `eagle_prepare_inputs_padded_kernel_impl` → PARITY-CPU-SHIM
- [ ] `eagle_prepare_next_token_padded_kernel_impl` → PARITY-CPU-SHIM
- [ ] `eagle_step_slot_mapping_metadata_kernel_impl` → PARITY-CPU-SHIM
- [ ] `expand_kernel_impl` → PARITY-CPU-SHIM
- [ ] `extend_attention_cpu` → PARITY-CPU-SHIM
- [ ] `flash_mla_with_kvcache_cpu` → PARITY-CPU-SHIM
- [ ] `float8_linear_prepack_cpu` → PARITY-CPU-SHIM
- [ ] `fp8_paged_mqa_logits_cpu` → PARITY-CPU-SHIM
- [ ] `fp8_scaled_mm_cpu` → PARITY-CPU-SHIM
- [ ] `fp8_scaled_mm_with_quant` → PARITY-CPU-SHIM
- [ ] `fused_experts_cpu` → PARITY-CPU-SHIM
- [ ] `fused_gdn_gating_cpu` → PARITY-CPU-SHIM
- [ ] `fused_gumbel_argmax` → PARITY-CPU-SHIM
- [ ] `fused_indexer_q_rope_quant_cpu` → PARITY-CPU-SHIM
- [ ] `fused_qnorm_rope_kv_insert_cpu` → PARITY-CPU-SHIM
- [ ] `fused_sigmoid_gating_delta_rule_update_cpu` → PARITY-CPU-SHIM
- [ ] `fused_sigmoid_gating_delta_rule_update_spec_cpu` → PARITY-CPU-SHIM
- [ ] `gelu_tanh` → PARITY-CPU-SHIM
- [ ] `get_scheduler_metadata` → PARITY-CPU-SHIM
- [ ] `greedy_argmax` → PARITY-CPU-SHIM
- [ ] `hash_topk_cpu` → PARITY-CPU-SHIM
- [ ] `hc_head_fused_cpu` → PARITY-CPU-SHIM
- [ ] `hc_post_fused_cpu` → PARITY-CPU-SHIM
- [ ] `hc_pre_fused_cpu` → PARITY-CPU-SHIM
- [ ] `init_cpu_memory_env` → PARITY-CPU-SHIM
- [ ] `init_shm_manager` → PARITY-CPU-SHIM
- [ ] `int4_scaled_mm_cpu` → PARITY-CPU-SHIM
- [ ] `int8_scaled_mm_with_quant` → PARITY-CPU-SHIM
- [ ] `inverse_gptj_rope_o_proj_cpu` → PARITY-CPU-SHIM
- [ ] `is_onednn_acl_supported` → PARITY-CPU-SHIM
- [ ] `join_shm_manager` → PARITY-CPU-SHIM
- [ ] `mamba_chunk_scan_fwd_cpu` → PARITY-CPU-SHIM
- [ ] `mla_decode_kvcache` → PARITY-CPU-SHIM
- [ ] `onednn_mm` → PARITY-CPU-SHIM
- [ ] `onednn_scaled_mm` → PARITY-CPU-SHIM
- [ ] `prepack_moe_weight` → PARITY-CPU-SHIM
- [ ] `prepack_moe_weight_int8` → PARITY-CPU-SHIM
- [ ] `quantize_fp8e4m3_vec` → PARITY-CPU-SHIM
- [ ] `rejection_greedy_sample_kernel_impl` → PARITY-CPU-SHIM
- [ ] `rejection_random_sample_kernel_impl` → PARITY-CPU-SHIM
- [ ] `release_dnnl_matmul_handler` → PARITY-CPU-SHIM
- [ ] `sample_recovered_tokens_kernel_impl` → PARITY-CPU-SHIM
- [ ] `save_partial_states_cpu` → PARITY-CPU-SHIM
- [ ] `selective_state_update_cpu` → PARITY-CPU-SHIM
- [ ] `shm_all_gather` → PARITY-CPU-SHIM
- [ ] `shm_allreduce` → PARITY-CPU-SHIM
- [ ] `shm_gather` → PARITY-CPU-SHIM
- [ ] `shm_recv_tensor_list` → PARITY-CPU-SHIM
- [ ] `shm_send_tensor_list` → PARITY-CPU-SHIM
- [ ] `topk_transform_512_cpu` → PARITY-CPU-SHIM
- [ ] `weight_packed_linear` → PARITY-CPU-SHIM

### 11.3 vLLM: compile-time fusion patterns

86 entries.

Every top-level class in `vllm/compilation/passes/fusion/`: the patterns, the passes that run them, and their matchers.

In `external/competitors/vllm/vllm/compilation/passes/fusion/`:

- [ ] `ActivationQuantPattern` (act_quant_fusion.py) → PARITY-QUANT-SCHEMES
- [ ] `SiluMulFp8StaticQuantPattern` (act_quant_fusion.py) → PARITY-QUANT-SCHEMES
- [ ] `SiluMulNvfp4QuantPattern` (act_quant_fusion.py) → PARITY-QUANT-SCHEMES
- [ ] `SiluMulBlockQuantPattern` (act_quant_fusion.py) → PARITY-QUANT-SCHEMES
- [ ] `ActivationQuantFusionPass` (act_quant_fusion.py) → PARITY-QUANT-SCHEMES
- [ ] `AddRMSNormPattern` (add_rms_fusion.py) → PARITY-GEMV-NORM-EPILOGUE
- [ ] `RMSNormReshapePattern` (add_rms_fusion.py) → PARITY-GEMV-NORM-EPILOGUE
- [ ] `FusedAddRMSNormReshapePattern` (add_rms_fusion.py) → PARITY-GEMV-NORM-EPILOGUE
- [ ] `AddRMSNormFusionPass` (add_rms_fusion.py) → PARITY-GEMV-NORM-EPILOGUE
- [ ] `RMSNormReshapeFusionPass` (add_rms_fusion.py) → PARITY-GEMV-NORM-EPILOGUE
- [ ] `FlashInferFusedAllReduceParams` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `BasePattern` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `AllReduceRMSNormPattern` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `AllReduceFusedAddRMSNormPattern` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `AllReduceGemmaRMSNormPattern` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `AllReduceFusedAddGemmaRMSNormPattern` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `AllReduceFusedRMSNormStaticQuantFP8Pattern` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `AllReduceFusedAddRMSNormStaticQuantFP8Pattern` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `AllReduceFusedRMSNormStaticQuantNVFP4Pattern` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `AllReduceFusedAddRMSNormStaticQuantNVFP4Pattern` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `AllReduceFusionPass` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `AiterAllreduceFusedRMSNormPattern` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `AiterAllreduceFusedGemmaRMSNormPattern` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `AiterAllreduceFusedAddRMSNormPattern` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `AiterAllreduceFusedAddGemmaRMSNormPattern` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `AiterAllreduceFusedAddRMSNormOutputOnlyPattern` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `AiterAllreduceFusedAddGemmaRMSNormOutputOnlyPattern` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `AiterAllreduceFusedRMSNormGroupQuantFP8Pattern` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `AiterAllreduceFusedAddRMSNormGroupQuantFP8Pattern` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `AiterAllreduceFusedAddRMSNormGroupQuantWithIndexerPattern` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `RocmAiterAllReduceFusionPass` (allreduce_rms_fusion.py) → PARITY-ALLREDUCE-NORM
- [ ] `AttnFp8StaticQuantPattern` (attn_quant_fusion.py) → PARITY-ATTN-QUANT-IN-FOLD
- [ ] `RocmAttnFp8StaticQuantPattern` (attn_quant_fusion.py) → PARITY-ATTN-QUANT-IN-FOLD
- [ ] `AttnNvfp4QuantPattern` (attn_quant_fusion.py) → PARITY-ATTN-QUANT-IN-FOLD
- [ ] `AttnQuantFusionPass` (attn_quant_fusion.py) → PARITY-ATTN-QUANT-IN-FOLD
- [ ] `MatcherCustomOp` (matcher_utils.py) → OPT-1
- [ ] `MatcherRotaryEmbedding` (matcher_utils.py) → OPT-1
- [ ] `MatcherMRotaryEmbedding` (matcher_utils.py) → OPT-1
- [ ] `MatcherRMSNormGated` (matcher_utils.py) → OPT-1
- [ ] `MatcherDeepseekScalingRotaryEmbedding` (matcher_utils.py) → OPT-1
- [ ] `MatcherQuantFP8` (matcher_utils.py) → OPT-1
- [ ] `MatcherSiluAndMul` (matcher_utils.py) → OPT-1
- [ ] `MLAAttnFp8StaticQuantPattern` (mla_attn_quant_fusion.py) → PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `RocmMLAAttnFp8StaticQuantPattern` (mla_attn_quant_fusion.py) → PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `MLAAttnNvfp4QuantPattern` (mla_attn_quant_fusion.py) → PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `MLAAttnFp8GroupQuantPattern` (mla_attn_quant_fusion.py) → PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `MLAAttnQuantFusionPass` (mla_attn_quant_fusion.py) → PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `MLARoPEKVCacheCatPattern` (mla_rope_kvcache_cat_fusion.py) → PARITY-MLA-PROLOGUE
- [ ] `MLARoPEKVCacheCatFusionPass` (mla_rope_kvcache_cat_fusion.py) → PARITY-MLA-PROLOGUE
- [ ] `QkNormRopePattern` (qk_norm_rope_fusion.py) → PARITY-QKV-HEAD-EPILOGUE
- [ ] `QKNormRoPEFusionPass` (qk_norm_rope_fusion.py) → PARITY-QKV-HEAD-EPILOGUE
- [ ] `QkNormRopeKvCachePattern` (qk_norm_rope_kvcache_fusion.py) → PARITY-QKV-HEAD-EPILOGUE
- [ ] `QkNormMRopeKvCachePattern` (qk_norm_rope_kvcache_fusion.py) → PARITY-QKV-HEAD-EPILOGUE
- [ ] `QkNormRopeKvCacheFusionPass` (qk_norm_rope_kvcache_fusion.py) → PARITY-QKV-HEAD-EPILOGUE
- [ ] `FusedRMSQuantKey` (rms_quant_fusion.py) → PARITY-QUANT-SCHEMES
- [ ] `RMSNormQuantPattern` (rms_quant_fusion.py) → PARITY-QUANT-SCHEMES
- [ ] `RMSNormStaticQuantPattern` (rms_quant_fusion.py) → PARITY-QUANT-SCHEMES
- [ ] `FusedAddRMSNormStaticQuantPattern` (rms_quant_fusion.py) → PARITY-QUANT-SCHEMES
- [ ] `FusedAddRMSNormGroupQuantPattern` (rms_quant_fusion.py) → PARITY-QUANT-SCHEMES
- [ ] `RMSNormGroupQuantPattern` (rms_quant_fusion.py) → PARITY-QUANT-SCHEMES
- [ ] `RMSNormDynamicQuantPattern` (rms_quant_fusion.py) → PARITY-QUANT-SCHEMES
- [ ] `FusedAddRMSNormDynamicQuantPattern` (rms_quant_fusion.py) → PARITY-QUANT-SCHEMES
- [ ] `FusedAddRMSNormNvfp4QuantPattern` (rms_quant_fusion.py) → PARITY-QUANT-SCHEMES
- [ ] `RMSNormQuantFusionPass` (rms_quant_fusion.py) → PARITY-QUANT-SCHEMES
- [ ] `AiterRMSNormQuantPattern` (rocm_aiter_fusion.py) → TRITON-AMD-SHIM
- [ ] `AiterRMSNormDynamicQuantPattern` (rocm_aiter_fusion.py) → TRITON-AMD-SHIM
- [ ] `AiterFusedAddRMSNormDynamicQuantPattern` (rocm_aiter_fusion.py) → TRITON-AMD-SHIM
- [ ] `AiterRMSFp8GroupQuantPattern` (rocm_aiter_fusion.py) → TRITON-AMD-SHIM
- [ ] `AiterFusedAddRMSFp8GroupQuantPattern` (rocm_aiter_fusion.py) → TRITON-AMD-SHIM
- [ ] `DoubleAiterRMSFp8GroupQuantPattern` (rocm_aiter_fusion.py) → PARITY-RECOMPUTE-FANOUT
- [ ] `DoubleAiterRMSFp8GroupQuantViewPattern` (rocm_aiter_fusion.py) → PARITY-RECOMPUTE-FANOUT
- [ ] `AiterRMSNormGatedFp8GroupQuantPattern` (rocm_aiter_fusion.py) → TRITON-AMD-SHIM
- [ ] `AiterRMSNormStaticQuantPattern` (rocm_aiter_fusion.py) → TRITON-AMD-SHIM
- [ ] `AiterFusedAddRMSNormStaticQuantPattern` (rocm_aiter_fusion.py) → TRITON-AMD-SHIM
- [ ] `RocmAiterRMSNormQuantFusionPass` (rocm_aiter_fusion.py) → TRITON-AMD-SHIM
- [ ] `AiterSiluMulFp8GroupQuantPattern` (rocm_aiter_fusion.py) → TRITON-AMD-SHIM
- [ ] `RocmAiterSiluMulFp8GroupQuantFusionPass` (rocm_aiter_fusion.py) → TRITON-AMD-SHIM
- [ ] `AddAiterRMSNormPadPattern` (rocm_aiter_fusion.py) → TRITON-AMD-SHIM
- [ ] `RocmAiterTritonAddRMSNormPadFusionPass` (rocm_aiter_fusion.py) → TRITON-AMD-SHIM
- [ ] `MLADualRMSNormPattern` (rocm_aiter_fusion.py) → PARITY-DUAL-NORM
- [ ] `MLADualRMSPerTokenQuantPattern` (rocm_aiter_fusion.py) → PARITY-DUAL-NORM
- [ ] `MLADualRMSGroupQuantPattern` (rocm_aiter_fusion.py) → PARITY-DUAL-NORM
- [ ] `MLADualRMSNormFusionPass` (rocm_aiter_fusion.py) → PARITY-DUAL-NORM
- [ ] `RopeStaticQQuantKVCachePattern` (rope_kvcache_fusion.py) → PARITY-QKV-HEAD-EPILOGUE
- [ ] `RopeReshapeKVCachePattern` (rope_kvcache_fusion.py) → PARITY-QKV-HEAD-EPILOGUE
- [ ] `RopeKVCacheFusionPass` (rope_kvcache_fusion.py) → PARITY-QKV-HEAD-EPILOGUE

### 11.4 vLLM: graph passes and IR ops

#### Graph passes (8)

The `…Pass` classes in `vllm/compilation/passes/ir/` and `passes/utility/`.

In `external/competitors/vllm/vllm/compilation/passes/ir/`:

- [ ] `UnsafeCloneEliminationPass` (clone_elimination.py) → GRAPH-IN-PLACE
- [ ] `VllmIRInplaceFunctionalizationPass` (inplace_functionalization.py) → GRAPH-IN-PLACE
- [ ] `VllmIRLoweringPass` (lowering_pass.py) → OPT-2

In `external/competitors/vllm/vllm/compilation/passes/utility/`:

- [ ] `FixFunctionalizationPass` (fix_functionalization.py) → GRAPH-IN-PLACE
- [ ] `NoOpEliminationPass` (noop_elimination.py) → GRAPH-CANONICALIZE
- [ ] `PostCleanupPass` (post_cleanup.py) → OPT-1
- [ ] `ScatterSplitReplacementPass` (scatter_split_replace.py) → GRAPH-IN-PLACE
- [ ] `SplitCoalescingPass` (split_coalescing.py) → GRAPH-CANONICALIZE

#### IR ops (3)

The ops `vllm/ir/ops/` registers with `@register_op`.

In `external/competitors/vllm/vllm/ir/ops/`:

- [ ] `gelu_and_mul_sparse` (activation.py) → PARITY-ACTIVATION-KINDS
- [ ] `rms_norm` (layernorm.py) → PARITY-GEMV-NORM-EPILOGUE
- [ ] `fused_add_rms_norm` (layernorm.py) → PARITY-GEMV-NORM-EPILOGUE

### 11.5 vLLM: attention backends and attention ops

#### Attention backends (63)

Every module under `vllm/v1/attention/backends/`.

In `external/competitors/vllm/vllm/v1/attention/backends/`:

- [ ] `b12x.py` → PARITY-PREFILL-ATTENTION-PEAK
- [ ] `composite.py` → PARITY-ATTN-GQA-ROW-PACKING
- [ ] `cpu_attn.py` → PARITY-CPU-SHIM
- [ ] `fa_utils.py` → PARITY-ATTN-GQA-ROW-PACKING
- [ ] `flash_attn.py` → PARITY-PREFILL-ATTENTION-PEAK
- [ ] `flash_attn_diffkv.py` → PARITY-KV-CACHE-QUANT
- [ ] `flashinfer.py` → PARITY-ATTN-GQA-ROW-PACKING
- [ ] `flex_attention.py` → PARITY-PREFILL-ATTENTION-PEAK
- [ ] `gdn_attn.py` → PARITY-GDN-CHUNK-TENSOR-CORES
- [ ] `hpc_attn.py` → PARITY-PREFILL-ATTENTION-PEAK
- [ ] `linear_attn.py` → PARITY-LIGHTNING-ATTENTION
- [ ] `mamba1_attn.py` → PARITY-SSD-CHUNK-SCAN
- [ ] `mamba2_attn.py` → PARITY-SSD-CHUNK-SCAN
- [ ] `mamba_attn.py` → PARITY-SSD-CHUNK-SCAN
- [ ] `recoverssm_metadata.py` → PARITY-ATTN-GQA-ROW-PACKING
- [ ] `registry.py` → SERVE-PAGED-KV
- [ ] `rocm_aiter_fa.py` → TRITON-AMD-SHIM
- [ ] `rocm_aiter_unified_attn.py` → TRITON-AMD-SHIM
- [ ] `rocm_attn.py` → TRITON-AMD-SHIM
- [ ] `rocm_segmented_attn.py` → TRITON-AMD-SHIM
- [ ] `short_conv_attn.py` → PARITY-SHORT-CONV
- [ ] `triton_attn.py` → PARITY-ATTN-GQA-ROW-PACKING
- [ ] `triton_attn_diffkv.py` → PARITY-KV-CACHE-QUANT
- [ ] `triton_flash_attn.py` → PARITY-PREFILL-ATTENTION-PEAK
- [ ] `triton_flashinfer.py` → PARITY-PREFILL-ATTENTION-PEAK
- [ ] `turboquant_attn.py` → PARITY-KV-CACHE-QUANT
- [ ] `ultraquant_attn.py` → PARITY-KV-CACHE-QUANT
- [ ] `utils.py` → SERVE-PAGED-KV
- [ ] `zentorch_sdpa.py` → PARITY-CPU-SHIM

In `external/competitors/vllm/vllm/v1/attention/backends/mla/`:

- [ ] `aiter_triton_mla.py` → TRITON-AMD-SHIM
- [ ] `amx_mla.py` → PARITY-CPU-SHIM
- [ ] `compressor_utils.py` → PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `cpu_mla.py` → PARITY-CPU-SHIM
- [ ] `cutlass_mla.py` → PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `flashattn_mla.py` → PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `flashattn_mla_sparse.py` → PARITY-SPARSE-ATTENTION
- [ ] `flashinfer_mla.py` → PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `flashinfer_mla_sparse.py` → PARITY-SPARSE-ATTENTION
- [ ] `flashinfer_mla_sparse_sm120.py` → PARITY-SPARSE-ATTENTION
- [ ] `flashinfer_mla_sparse_sm90.py` → PARITY-SPARSE-ATTENTION
- [ ] `flashmla.py` → PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `flashmla_sparse.py` → PARITY-SPARSE-ATTENTION
- [ ] `index_group.py` → PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `indexer.py` → PARITY-SPARSE-ATTENTION
- [ ] `rocm_aiter_mla.py` → TRITON-AMD-SHIM
- [ ] `rocm_aiter_mla_sparse.py` → TRITON-AMD-SHIM
- [ ] `rocm_paged_mxfp4_indexer.py` → TRITON-AMD-SHIM
- [ ] `sparse_indexer.py` → PARITY-SPARSE-ATTENTION
- [ ] `sparse_swa.py` → PARITY-SPARSE-ATTENTION
- [ ] `sparse_utils.py` → PARITY-SPARSE-ATTENTION
- [ ] `tokenspeed_mla.py` → PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `triton_mla.py` → PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `xpu_mla_sparse.py` → PARITY-XPU-SHIM

In `external/competitors/vllm/vllm/v1/attention/backends/mla/prefill/`:

- [ ] `aiter_flash_attn.py` → TRITON-AMD-SHIM
- [ ] `base.py` → PARITY-PREFILL-ATTENTION-PEAK, PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `cpu_sdpa.py` → PARITY-CPU-SHIM
- [ ] `flash_attn.py` → PARITY-PREFILL-ATTENTION-PEAK, PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `flashinfer.py` → PARITY-PREFILL-ATTENTION-PEAK, PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `registry.py` → PARITY-PREFILL-ATTENTION-PEAK, PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `selector.py` → PARITY-PREFILL-ATTENTION-PEAK, PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `tokenspeed_mla.py` → PARITY-PREFILL-ATTENTION-PEAK, PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `trtllm_ragged.py` → PARITY-PREFILL-ATTENTION-PEAK, PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `zen_cpu_sdpa.py` → PARITY-CPU-SHIM

#### Attention ops (42)

Every module under `vllm/v1/attention/ops/`.

In `external/competitors/vllm/vllm/v1/attention/ops/`:

- [ ] `chunked_prefill_paged_decode.py` → PARITY-MIXED-BATCH-ATTENTION
- [ ] `common.py` → SERVE-PAGED-KV
- [ ] `cp_common.py` → PARITY-TENSOR-PARALLEL-IR
- [ ] `dcp.py` → PARITY-TENSOR-PARALLEL-IR
- [ ] `flashmla.py` → PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `flydsl_turboquant_decode.py` → TRITON-AMD-SHIM
- [ ] `flydsl_ultraquant_decode.py` → TRITON-AMD-SHIM
- [ ] `int4_per_token_head.py` → PARITY-KV-CACHE-QUANT
- [ ] `merge_attn_states.py` → PARITY-ATTN-LSE-MERGE
- [ ] `metadata.py` → SERVE-PAGED-KV
- [ ] `paged_attn.py` → SERVE-PAGED-KV
- [ ] `pcp.py` → PARITY-TENSOR-PARALLEL-IR
- [ ] `prefix_prefill.py` → PARITY-MIXED-BATCH-ATTENTION
- [ ] `rocm_aiter_mla_merge.py` → TRITON-AMD-SHIM
- [ ] `rocm_aiter_mla_prefill.py` → TRITON-AMD-SHIM
- [ ] `rocm_aiter_mla_sparse.py` → TRITON-AMD-SHIM
- [ ] `rocm_paged_mxfp4_indexer.py` → TRITON-AMD-SHIM
- [ ] `segmented_attention.py` → TRITON-AMD-SHIM
- [ ] `triton_attention_helpers.py` → PARITY-ATTENTION-VARIANTS-INLINE
- [ ] `triton_decode_attention.py` → PARITY-ATTN-GQA-ROW-PACKING
- [ ] `triton_merge_attn_states.py` → PARITY-ATTN-LSE-MERGE
- [ ] `triton_prefill_attention.py` → PARITY-PREFILL-ATTENTION-PEAK
- [ ] `triton_reshape_and_cache_flash.py` → PARITY-KV-CACHE-QUANT
- [ ] `triton_turboquant_decode.py` → PARITY-KV-CACHE-QUANT
- [ ] `triton_turboquant_store.py` → PARITY-KV-CACHE-QUANT
- [ ] `triton_unified_attention.py` → PARITY-ATTN-GQA-ROW-PACKING
- [ ] `triton_unified_attention_diffkv.py` → PARITY-KV-CACHE-QUANT
- [ ] `vit_attn_wrappers.py` → PARITY-MULTIMODAL-TOWERS
- [ ] `xpu_mla_sparse.py` → PARITY-XPU-SHIM

In `external/competitors/vllm/vllm/v1/attention/ops/flydsl_kernels/`:

- [ ] `buffer_utils.py` → TRITON-AMD-SHIM
- [ ] `mma_utils.py` → TRITON-AMD-SHIM
- [ ] `tq_decode.py` → TRITON-AMD-SHIM
- [ ] `ultraquant_decode_hd256.py` → TRITON-AMD-SHIM

In `external/competitors/vllm/vllm/v1/attention/ops/turboquant_soa/`:

- [ ] `triton_turboquant_decode.py` → PARITY-KV-CACHE-QUANT
- [ ] `triton_turboquant_decode_v2.py` → PARITY-KV-CACHE-QUANT
- [ ] `triton_turboquant_store.py` → PARITY-KV-CACHE-QUANT
- [ ] `triton_turboquant_unified_attention.py` → PARITY-KV-CACHE-QUANT

In `external/competitors/vllm/vllm/v1/attention/ops/ultraquant/`:

- [ ] `format.py` → PARITY-KV-CACHE-QUANT
- [ ] `reference.py` → PARITY-KV-CACHE-QUANT
- [ ] `triton_dequant.py` → PARITY-KV-CACHE-QUANT
- [ ] `triton_store.py` → PARITY-KV-CACHE-QUANT
- [ ] `triton_unified_attention.py` → PARITY-KV-CACHE-QUANT

### 11.6 vLLM: MoE experts, routers, prepare and finalize

#### Experts (40)

Every module under `fused_moe/experts/`.

In `external/competitors/vllm/vllm/model_executor/layers/fused_moe/experts/`:

- [ ] `aiter_mxfp4_w4a16_moe.py` → TRITON-AMD-SHIM
- [ ] `aiter_mxfp4_w4a4_moe.py` → TRITON-AMD-SHIM
- [ ] `aiter_mxfp4_w4a8_moe.py` → TRITON-AMD-SHIM
- [ ] `aiter_mxfp8_moe.py` → TRITON-AMD-SHIM
- [ ] `batched_deep_gemm_moe.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `cpu_int4_moe.py` → PARITY-CPU-SHIM
- [ ] `cpu_moe.py` → PARITY-CPU-SHIM
- [ ] `cutlass_moe.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `deep_gemm_moe.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `fallback.py` → PARITY-MOE-GATHER-IN-OPERAND
- [ ] `flashinfer_b12x_moe.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `flashinfer_cutedsl_batched_moe.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `flashinfer_cutedsl_moe.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `flashinfer_cutlass_moe.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `flashinfer_moe_ep.py` → PARITY-EXPERT-PARALLEL
- [ ] `fused_batched_moe.py` → PARITY-MOE-GATHER-IN-OPERAND
- [ ] `fused_humming_moe.py` → PARITY-WEIGHT-FORMATS
- [ ] `gpt_oss_triton_kernels_moe.py` → PARITY-TRITON-MOE-MATMUL
- [ ] `int4_emulation_moe.py` → PARITY-QUANT-SCHEMES
- [ ] `lora_context.py` → SERVE-LORA
- [ ] `lora_experts_mixin.py` → SERVE-LORA
- [ ] `marlin_moe.py` → PARITY-WEIGHT-FORMATS
- [ ] `moonep_experts.py` → PARITY-EXPERT-PARALLEL
- [ ] `mxfp8_emulation_moe.py` → PARITY-QUANT-SCHEMES
- [ ] `mxfp8_native_moe.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `nvfp4_emulation_moe.py` → PARITY-QUANT-SCHEMES
- [ ] `ocp_mx_emulation_moe.py` → PARITY-QUANT-SCHEMES
- [ ] `rdna3_moe.py` → TRITON-AMD-SHIM
- [ ] `rocm_aiter_moe.py` → TRITON-AMD-SHIM
- [ ] `triton_cutlass_moe.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `triton_deep_gemm_moe.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `triton_moe.py` → PARITY-MOE-GATHER-IN-OPERAND
- [ ] `trtllm_bf16_moe.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `trtllm_fp8_moe.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `trtllm_lora_moe.py` → SERVE-LORA
- [ ] `trtllm_mxfp4_moe.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `trtllm_mxint4_moe.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `trtllm_nvfp4_moe.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `xpu_moe.py` → PARITY-XPU-SHIM
- [ ] `zentorch_moe.py` → PARITY-CPU-SHIM

#### Routers, prepare and finalize (28)

Every module under `fused_moe/prepare_finalize/` and `fused_moe/router/`.

In `external/competitors/vllm/vllm/model_executor/layers/fused_moe/prepare_finalize/`:

- [ ] `batched.py` → PARITY-EXPERT-PARALLEL
- [ ] `deepep_ht.py` → PARITY-EXPERT-PARALLEL
- [ ] `deepep_ll.py` → PARITY-EXPERT-PARALLEL
- [ ] `deepep_v2.py` → PARITY-EXPERT-PARALLEL
- [ ] `flashinfer_nvlink_one_sided.py` → PARITY-EXPERT-PARALLEL
- [ ] `flashinfer_nvlink_two_sided.py` → PARITY-EXPERT-PARALLEL
- [ ] `moonep.py` → PARITY-EXPERT-PARALLEL
- [ ] `mori.py` → PARITY-EXPERT-PARALLEL
- [ ] `naive_dp_ep.py` → PARITY-EXPERT-PARALLEL
- [ ] `nixl_ep.py` → PARITY-EXPERT-PARALLEL
- [ ] `no_dp_ep.py` → PARITY-EXPERT-PARALLEL
- [ ] `passthrough.py` → PARITY-EXPERT-PARALLEL

In `external/competitors/vllm/vllm/model_executor/layers/fused_moe/router/`:

- [ ] `aiter_shared_routed_fused_moe_router.py` → TRITON-AMD-SHIM
- [ ] `base_router.py` → PARITY-ROUTING-VARIANTS
- [ ] `bf16x3_router_gemm_cutedsl.py` → TECH-SPLIT-PRECISION-MMA
- [ ] `bf16x3_router_gemm_rocm.py` → TRITON-AMD-SHIM
- [ ] `cpu_router.py` → PARITY-CPU-SHIM
- [ ] `custom_routing_router.py` → PARITY-ROUTING-VARIANTS
- [ ] `dsv4_topk.py` → PARITY-ROUTING-VARIANTS
- [ ] `fused_moe_router.py` → PARITY-ROUTING-VARIANTS
- [ ] `fused_topk_bias_router.py` → PARITY-ROUTING-VARIANTS
- [ ] `fused_topk_router.py` → PARITY-ROUTING-VARIANTS
- [ ] `gate_linear.py` → PARITY-ROUTING-VARIANTS
- [ ] `grouped_topk_router.py` → PARITY-ROUTING-VARIANTS
- [ ] `rocm_fp32_router_gemm.py` → TRITON-AMD-SHIM
- [ ] `router_factory.py` → PARITY-ROUTING-VARIANTS
- [ ] `routing_simulator_router.py` → PARITY-ROUTING-VARIANTS
- [ ] `zero_expert_router.py` → PARITY-ROUTING-VARIANTS

### 11.7 vLLM: linear kernels

97 entries.

Every `…Kernel` class under `vllm/model_executor/kernels/linear/`: the kernels vLLM's linear-layer selector chooses among.

In `external/competitors/vllm/vllm/model_executor/kernels/linear/`:

- [ ] `MMLinearKernel` (base.py) → OPT-2

In `external/competitors/vllm/vllm/model_executor/kernels/linear/mixed_precision/`:

- [ ] `MPLinearKernel` (MPLinearKernel.py) → OPT-2
- [ ] `ConchLinearKernel` (conch.py) → PARITY-WEIGHT-FORMATS
- [ ] `CPUWNA16LinearKernel` (cpu.py) → PARITY-CPU-SHIM
- [ ] `CutlassW4A8LinearKernel` (cutlass.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `Dynamic4bitLinearKernel` (dynamic_4bit.py) → PARITY-CPU-SHIM
- [ ] `ExllamaLinearKernel` (exllama.py) → PARITY-WEIGHT-FORMATS
- [ ] `HummingLinearKernel` (humming.py) → PARITY-WEIGHT-FORMATS
- [ ] `MacheteLinearKernel` (machete.py) → TECH-WGMMA-RS-PREPACKED
- [ ] `MarlinLinearKernel` (marlin.py) → PARITY-WEIGHT-FORMATS
- [ ] `RDNA3W4A16LinearKernel` (rdna3_w4a16.py) → TRITON-AMD-SHIM
- [ ] `RDNAHybridW4A16LinearKernel` (rdna_hybrid_w4a16.py) → TRITON-AMD-SHIM
- [ ] `TritonW4A16LinearKernel` (triton_w4a16.py) → PARITY-WEIGHT-FORMATS
- [ ] `XPUwNa16LinearKernel` (xpu.py) → PARITY-XPU-SHIM
- [ ] `XPUW4A8IntLinearKernel` (xpu.py) → PARITY-XPU-SHIM
- [ ] `ZentorchWNA16LinearKernel` (zentorch.py) → PARITY-CPU-SHIM

In `external/competitors/vllm/vllm/model_executor/kernels/linear/mxfp4/`:

- [ ] `AiterMxfp4LinearKernel` (aiter.py) → TRITON-AMD-SHIM
- [ ] `B12xMxFp4LinearKernel` (b12x.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `MxFp4LinearKernel` (base.py) → OPT-2
- [ ] `EmulationMxfp4LinearKernel` (emulation.py) → PARITY-QUANT-SCHEMES
- [ ] `FlashInferMxFp4LinearKernel` (flashinfer.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `HummingMxFp4LinearKernel` (humming.py) → PARITY-WEIGHT-FORMATS
- [ ] `MarlinMxFp4LinearKernel` (marlin.py) → PARITY-WEIGHT-FORMATS
- [ ] `TorchMxFp4LinearKernel` (pytorch.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `Rdna3W4A8MxFp4LinearKernel` (rdna3_hip.py) → TRITON-AMD-SHIM
- [ ] `XPUMxFp4LinearKernel` (xpu.py) → PARITY-XPU-SHIM

In `external/competitors/vllm/vllm/model_executor/kernels/linear/mxfp6/`:

- [ ] `MxFp6LinearKernel` (base.py) → OPT-2
- [ ] `EmulationMxfp6LinearKernel` (emulation.py) → PARITY-QUANT-SCHEMES
- [ ] `HummingMxFp6LinearKernel` (humming.py) → PARITY-WEIGHT-FORMATS

In `external/competitors/vllm/vllm/model_executor/kernels/linear/mxfp8/`:

- [ ] `Mxfp8LinearKernel` (Mxfp8LinearKernel.py) → OPT-2
- [ ] `B12xMxfp8LinearKernel` (b12x.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `DeepGemmMxfp8BmmLinearKernel` (deep_gemm.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `EmulationMxfp8LinearKernel` (emulation.py) → PARITY-QUANT-SCHEMES
- [ ] `FlashInferCutlassMxfp8LinearKernel` (flashinfer.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `FlashInferCutedslMxfp8LinearKernel` (flashinfer.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `FlashInferTrtllmMxfp8LinearKernel` (flashinfer.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `HummingMxfp8LinearKernel` (humming.py) → PARITY-WEIGHT-FORMATS
- [ ] `MarlinMxfp8LinearKernel` (marlin.py) → PARITY-WEIGHT-FORMATS
- [ ] `TorchMxFp8LinearKernel` (pytorch.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `RocmDotScaledMxfp8LinearKernel` (rocm_native.py) → TRITON-AMD-SHIM
- [ ] `XPUMxFp8LinearKernel` (xpu.py) → PARITY-XPU-SHIM

In `external/competitors/vllm/vllm/model_executor/kernels/linear/nvfp4/`:

- [ ] `B12xNvFp4LinearKernel` (b12x.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `B12xNvFp4W4A16LinearKernel` (b12x.py) → PARITY-WEIGHT-FORMATS
- [ ] `NvFp4LinearKernel` (base.py) → OPT-2
- [ ] `CutlassNvFp4LinearKernel` (cutlass.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `EmulationNvFp4LinearKernel` (emulation.py) → PARITY-QUANT-SCHEMES
- [ ] `FbgemmNvFp4LinearKernel` (fbgemm.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `FlashInferCuteDslNvFp4W4A16LinearKernel` (flashinfer.py) → PARITY-WEIGHT-FORMATS
- [ ] `FlashInferCuteDslNvFp4LinearKernel` (flashinfer.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `FlashInferCutlassNvFp4LinearKernel` (flashinfer.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `FlashInferTrtllmNvFp4LinearKernel` (flashinfer.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `FlashInferCudnnNvFp4LinearKernel` (flashinfer.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `FlashInferB12xNvFp4LinearKernel` (flashinfer.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `HummingNvFp4LinearKernel` (humming.py) → PARITY-WEIGHT-FORMATS
- [ ] `MarlinNvFp4LinearKernel` (marlin.py) → PARITY-WEIGHT-FORMATS
- [ ] `TorchNvFp4LinearKernel` (pytorch.py) → PARITY-QUANT-GEMMS-DATACENTER

In `external/competitors/vllm/vllm/model_executor/kernels/linear/scaled_mm/`:

- [ ] `Fp8BlockScaledMMLinearKernel` (BlockScaledMMLinearKernel.py) → OPT-2
- [ ] `Fp8BlockScaledDynamicMMLinearKernel` (BlockScaledMMLinearKernel.py) → OPT-2
- [ ] `ScaledMMLinearKernel` (ScaledMMLinearKernel.py) → OPT-2
- [ ] `FP8ScaledMMLinearKernel` (ScaledMMLinearKernel.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `Int8ScaledMMLinearKernel` (ScaledMMLinearKernel.py) → PARITY-INT8-GEMM-PEAK
- [ ] `AiterInt8ScaledMMLinearKernel` (aiter.py) → TRITON-AMD-SHIM
- [ ] `AiterPreshuffledPerTokenFp8ScaledMMLinearKernel` (aiter.py) → TRITON-AMD-SHIM
- [ ] `AiterHipbMMPerTokenFp8ScaledMMLinearKernel` (aiter.py) → TRITON-AMD-SHIM
- [ ] `AiterPerTokenFp8ScaledMMLinearKernel` (aiter.py) → TRITON-AMD-SHIM
- [ ] `AiterFp8BlockScaledMMKernel` (aiter.py) → TRITON-AMD-SHIM
- [ ] `B12xFp8BlockScaledMMKernel` (b12x.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `B12xTensorFP8ScaledMMLinearKernel` (b12x.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `CPUInt8ScaledMMLinearKernel` (cpu.py) → PARITY-CPU-SHIM
- [ ] `CPUFp8BlockScaledMMKernel` (cpu.py) → PARITY-CPU-SHIM
- [ ] `CPUFp8PerTensorScaledMMLinearKernel` (cpu.py) → PARITY-CPU-SHIM
- [ ] `CPUFp8ChannelWiseScaledMMLinearKernel` (cpu.py) → PARITY-CPU-SHIM
- [ ] `CPUFp8W8A8BlockScaledMMKernel` (cpu.py) → PARITY-CPU-SHIM
- [ ] `CPUFP8W8A8ScaledMMLinearKernel` (cpu.py) → PARITY-CPU-SHIM
- [ ] `CutlassInt8ScaledMMLinearKernel` (cutlass.py) → PARITY-INT8-GEMM-PEAK
- [ ] `CutlassFP8ScaledMMLinearKernel` (cutlass.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `CutlassFp8BlockScaledMMKernel` (cutlass.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `DeepGemmFp8BlockScaledMMKernel` (deep_gemm.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `FlashInferFP8ScaledMMLinearKernel` (flashinfer.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `FlashInferFp8BlockScaledMMKernel` (flashinfer.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `FlashInferFp8DeepGEMMDynamicBlockScaledKernel` (flashinfer.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `HummingFP8ScaledMMLinearKernel` (humming.py) → PARITY-WEIGHT-FORMATS
- [ ] `HummingInt8ScaledMMLinearKernel` (humming.py) → PARITY-WEIGHT-FORMATS
- [ ] `MarlinFP8ScaledMMLinearKernel` (marlin.py) → PARITY-WEIGHT-FORMATS
- [ ] `TorchFP8ScaledMMLinearKernel` (pytorch.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `PerTensorTorchFP8ScaledMMLinearKernel` (pytorch.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `RowWiseTorchFP8ScaledMMLinearKernel` (pytorch.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `ChannelWiseTorchFP8ScaledMMLinearKernel` (pytorch.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `BlockWiseTorchFP8ScaledMMLinearKernel` (pytorch.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `ROCmFP8ScaledMMLinearKernel` (rocm.py) → TRITON-AMD-SHIM
- [ ] `TritonInt8ScaledMMLinearKernel` (triton.py) → PARITY-INT8-GEMM-PEAK
- [ ] `TritonFP8ScaledMMLinearKernel` (triton.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `TritonFp8BlockScaledMMKernel` (triton.py) → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `XPUW8A8FP8LinearKernel` (xpu.py) → PARITY-XPU-SHIM
- [ ] `XPUW8A16FP8LinearKernel` (xpu.py) → PARITY-XPU-SHIM
- [ ] `XPUFp8BlockScaledMMKernel` (xpu.py) → PARITY-XPU-SHIM
- [ ] `ZentorchInt8ScaledMMLinearKernel` (zentorch.py) → PARITY-CPU-SHIM

### 11.8 vLLM: every file with a Triton kernel

294 entries.

Every file under `vllm/` with at least one `@triton.jit`. The per-model trees under `vllm/models/` are here too: each maps to the shared fusion it embodies, never to a model-specific ticket, unless the layer exists in one family only (PARITY-MODEL-SPECIFIC-LAYERS).

Triton kernels in `external/competitors/vllm/vllm/`:

- [ ] `_xpu_ops.py` → PARITY-XPU-SHIM

Triton kernels in `external/competitors/vllm/vllm/distributed/kv_transfer/kv_connector/v1/hf3fs/utils/`:

- [ ] `gather_scatter_helper.py` → SERVE-KV-OFFLOAD

Triton kernels in `external/competitors/vllm/vllm/kernels/triton/`:

- [ ] `activation.py` → PARITY-ACTIVATION-KINDS
- [ ] `qkv_padded_fp8_quant.py` → PARITY-QUANT-SCHEMES

Triton kernels in `external/competitors/vllm/vllm/lora/ops/triton_ops/`:

- [ ] `fp8_kernel_utils.py` → SERVE-LORA
- [ ] `fused_moe_lora_fp8_op.py` → SERVE-LORA
- [ ] `fused_moe_lora_op.py` → SERVE-LORA
- [ ] `kernel_utils.py` → SERVE-LORA
- [ ] `lora_expand_fp8_op.py` → SERVE-LORA
- [ ] `lora_expand_op.py` → SERVE-LORA
- [ ] `lora_shrink_fp8_op.py` → SERVE-LORA
- [ ] `lora_shrink_op.py` → SERVE-LORA

Triton kernels in `external/competitors/vllm/vllm/model_executor/determinism/`:

- [ ] `batch_invariant.py` → TECH-BATCH-INVARIANCE

Triton kernels in `external/competitors/vllm/vllm/model_executor/kernels/attention/dsa/`:

- [ ] `candidate_blocks.py` → PARITY-SPARSE-ATTENTION
- [ ] `dcp_indexer_cutedsl.py` → PARITY-SPARSE-ATTENTION
- [ ] `sparse_mqa_logits.py` → PARITY-SPARSE-ATTENTION

Triton kernels in `external/competitors/vllm/vllm/model_executor/kernels/linear/mixed_precision/`:

- [ ] `rdna_hybrid_w4a16.py` → TRITON-AMD-SHIM
- [ ] `triton_w4a16.py` → PARITY-WEIGHT-FORMATS

Triton kernels in `external/competitors/vllm/vllm/model_executor/kernels/linear/mxfp8/`:

- [ ] `rocm_block32_gemm.py` → TRITON-AMD-SHIM
- [ ] `rocm_native.py` → TRITON-AMD-SHIM

Triton kernels in `external/competitors/vllm/vllm/model_executor/kernels/mhc/`:

- [ ] `triton.py` → PARITY-HYPER-CONNECTIONS

Triton kernels in `external/competitors/vllm/vllm/model_executor/layers/`:

- [ ] `activation.py` → PARITY-ACTIVATION-KINDS
- [ ] `fused_embed_norm.py` → PARITY-EMBED-NORM
- [ ] `fused_qk_norm_rope.py` → PARITY-QK-NORM-ROPE-GATE-SPLIT
- [ ] `lightning_attn.py` → PARITY-LIGHTNING-ATTENTION
- [ ] `sparse_attn_indexer.py` → PARITY-SPARSE-ATTENTION

Triton kernels in `external/competitors/vllm/vllm/model_executor/layers/attention/`:

- [ ] `sparse_mla_attention.py` → PARITY-SPARSE-ATTENTION

Triton kernels in `external/competitors/vllm/vllm/model_executor/layers/fused_moe/`:

- [ ] `deep_gemm_utils.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `fused_moe.py` → PARITY-MOE-GATHER-IN-OPERAND
- [ ] `moe_fused_mul_sum.py` → PARITY-MOE-DEFERRED-FINALIZE
- [ ] `prepare_megamoe.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `utils.py` → PARITY-MOE-GATHER-IN-OPERAND

Triton kernels in `external/competitors/vllm/vllm/model_executor/layers/fused_moe/experts/`:

- [ ] `batched_deep_gemm_moe.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `fused_batched_moe.py` → PARITY-MOE-GATHER-IN-OPERAND
- [ ] `gpt_oss_triton_kernels_moe.py` → PARITY-TRITON-MOE-MATMUL
- [ ] `mxfp8_native_moe.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `nvfp4_emulation_moe.py` → PARITY-QUANT-SCHEMES
- [ ] `trtllm_lora_moe.py` → SERVE-LORA

Triton kernels in `external/competitors/vllm/vllm/model_executor/layers/fused_moe/prepare_finalize/`:

- [ ] `deepep_v2.py` → PARITY-EXPERT-PARALLEL

Triton kernels in `external/competitors/vllm/vllm/model_executor/layers/fused_moe/router/`:

- [ ] `base_router.py` → PARITY-ROUTING-VARIANTS
- [ ] `bf16x3_router_gemm_cutedsl.py` → TECH-SPLIT-PRECISION-MMA
- [ ] `bf16x3_router_gemm_rocm.py` → TRITON-AMD-SHIM
- [ ] `dsv4_topk.py` → PARITY-ROUTING-VARIANTS
- [ ] `rocm_fp32_router_gemm.py` → TRITON-AMD-SHIM

Triton kernels in `external/competitors/vllm/vllm/model_executor/layers/fusion/`:

- [ ] `mm_input_norm.py` → PARITY-AFFINE-INPUT-FOLD
- [ ] `relu2_fp8_quant.py` → PARITY-RELU-SQR

Triton kernels in `external/competitors/vllm/vllm/model_executor/layers/mamba/`:

- [ ] `checkpoint.py` → PARITY-SSD-CHUNK-SCAN

Triton kernels in `external/competitors/vllm/vllm/model_executor/layers/mamba/gdn/`:

- [ ] `olmo_gdn_linear_attn.py` → PARITY-GDN-CHUNK-TENSOR-CORES

Triton kernels in `external/competitors/vllm/vllm/model_executor/layers/mamba/linear/`:

- [ ] `bailing_linear_attn.py` → PARITY-LIGHTNING-ATTENTION

Triton kernels in `external/competitors/vllm/vllm/model_executor/layers/mamba/ops/`:

- [ ] `causal_conv1d.py` → PARITY-GDN-POST-CONV-PREP
- [ ] `gather_initial_states.py` → PARITY-SSD-CHUNK-SCAN
- [ ] `layernorm_gated.py` → PARITY-SSD-CHUNK-SCAN
- [ ] `mamba_ssm.py` → PARITY-SSD-CHUNK-SCAN, TECH-STOCHASTIC-ROUNDING-STATE
- [ ] `scatter_states.py` → PARITY-SSD-CHUNK-SCAN
- [ ] `selective_state_update_replayssm_output_only.py` → PARITY-SSD-CHUNK-SCAN, SERVE-SPEC-DECODE
- [ ] `ssd_bmm.py` → PARITY-SSD-CHUNK-SCAN
- [ ] `ssd_checkpoint.py` → PARITY-SSD-CHUNK-SCAN
- [ ] `ssd_chunk_scan.py` → PARITY-SSD-CHUNK-SCAN
- [ ] `ssd_chunk_state.py` → PARITY-SSD-CHUNK-SCAN
- [ ] `ssd_state_passing.py` → PARITY-SSD-CHUNK-SCAN
- [ ] `ssu_dispatch.py` → PARITY-SSD-CHUNK-SCAN
- [ ] `triton_helpers.py` → PARITY-SSD-CHUNK-SCAN

Triton kernels in `external/competitors/vllm/vllm/model_executor/layers/minimax_rms_norm/`:

- [ ] `rms_norm_tp.py` → PARITY-ALLREDUCE-NORM

Triton kernels in `external/competitors/vllm/vllm/model_executor/layers/quantization/`:

- [ ] `awq_triton.py` → PARITY-WEIGHT-FORMATS
- [ ] `qutlass_utils.py` → PARITY-QUANT-GEMMS-DATACENTER

Triton kernels in `external/competitors/vllm/vllm/model_executor/layers/quantization/compressed_tensors/`:

- [ ] `compressed_tensors_embedding.py` → PARITY-EMBED-NORM
- [ ] `triton_scaled_mm.py` → PARITY-INT8-GEMM-PEAK

Triton kernels in `external/competitors/vllm/vllm/model_executor/layers/quantization/utils/`:

- [ ] `fp8_utils.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `int8_utils.py` → PARITY-INT8-GEMM-PEAK
- [ ] `mxfp4_utils.py` → PARITY-QUANT-SCHEMES
- [ ] `mxfp8_utils.py` → PARITY-QUANT-SCHEMES
- [ ] `nvfp4_emulation_utils.py` → PARITY-QUANT-SCHEMES

Triton kernels in `external/competitors/vllm/vllm/model_executor/layers/rotary_embedding/`:

- [ ] `bailing_mrope.py` → PARITY-ROPE-VARIANTS
- [ ] `mrope.py` → PARITY-ROPE-VARIANTS
- [ ] `mrope_vit_setup.py` → PARITY-ROPE-VARIANTS, PARITY-MULTIMODAL-TOWERS
- [ ] `packed_qk_rope.py` → PARITY-ROPE-QK-ONE-LAUNCH

Triton kernels in `external/competitors/vllm/vllm/model_executor/models/`:

- [ ] `cohere_compass.py` → PARITY-MULTIMODAL-TOWERS
- [ ] `deepencoder.py` → PARITY-MULTIMODAL-TOWERS
- [ ] `diffusion_gemma_sampler.py` → PARITY-SAMPLING
- [ ] `gemma4.py` → PARITY-ROUTING-VARIANTS
- [ ] `qwen3_dflash2.py` → SERVE-SPEC-DECODE
- [ ] `qwen3_vl.py` → PARITY-MULTIMODAL-TOWERS

Triton kernels in `external/competitors/vllm/vllm/models/beam/nvidia/`:

- [ ] `activation.py` → PARITY-SOFTCAP, PARITY-ACTIVATION-KINDS
- [ ] `attention_ops.py` → PARITY-QK-NORM-ROPE-GATE-SPLIT
- [ ] `dflash2_context_ops.py` → SERVE-SPEC-DECODE
- [ ] `dflash2_conv_ops.py` → SERVE-SPEC-DECODE
- [ ] `dflash2_ops.py` → SERVE-SPEC-DECODE
- [ ] `layernorm.py` → PARITY-NORM-THEN-ADD

Triton kernels in `external/competitors/vllm/vllm/models/common/ops/`:

- [ ] `fused_qk_rmsnorm.py` → PARITY-QKV-HEAD-EPILOGUE

Triton kernels in `external/competitors/vllm/vllm/models/deepseek_v32/common/`:

- [ ] `kernels.py` → PARITY-SPARSE-ATTENTION, PARITY-KV-CACHE-QUANT, PARITY-EMBED-NORM

Triton kernels in `external/competitors/vllm/vllm/models/deepseek_v4/amd/`:

- [ ] `mega_moe.py` → TRITON-AMD-SHIM
- [ ] `rocm.py` → TRITON-AMD-SHIM

Triton kernels in `external/competitors/vllm/vllm/models/deepseek_v4/`:

- [ ] `attention.py` → PARITY-SPARSE-ATTENTION
- [ ] `sparse_mla.py` → PARITY-SPARSE-ATTENTION

Triton kernels in `external/competitors/vllm/vllm/models/deepseek_v4/common/ops/`:

- [ ] `cache_utils.py` → PARITY-KV-CACHE-QUANT
- [ ] `fused_compress_quant_cache.py` → PARITY-SPARSE-ATTENTION
- [ ] `fused_indexer_q.py` → PARITY-SPARSE-ATTENTION
- [ ] `fused_inv_rope_fp8_quant.py` → PARITY-ATTN-QUANT-IN-FOLD
- [ ] `fused_mtp_input_rmsnorm.py` → PARITY-EMBED-NORM
- [ ] `save_partial_states.py` → PARITY-SPARSE-ATTENTION

Triton kernels in `external/competitors/vllm/vllm/models/deepseek_v4/xpu/`:

- [ ] `model.py` → PARITY-XPU-SHIM
- [ ] `xpu_qnorm_rope_kv_fp8_insert.py` → PARITY-XPU-SHIM
- [ ] `xpu_sparse_decode_fp8.py` → PARITY-XPU-SHIM

Triton kernels in `external/competitors/vllm/vllm/models/deepseek_v41/amd/mono/`:

- [ ] `keylist.py` → TRITON-AMD-SHIM

Triton kernels in `external/competitors/vllm/vllm/models/deepseek_v41/amd/ops/`:

- [ ] `fused_markov.py` → TRITON-AMD-SHIM
- [ ] `swa_decode_meta.py` → TRITON-AMD-SHIM
- [ ] `topk_pack.py` → TRITON-AMD-SHIM

Triton kernels in `external/competitors/vllm/vllm/models/deepseek_v41/amd/`:

- [ ] `rocm.py` → TRITON-AMD-SHIM

Triton kernels in `external/competitors/vllm/vllm/models/deepseek_v41/`:

- [ ] `attention.py` → PARITY-SPARSE-ATTENTION
- [ ] `compressor.py` → PARITY-SPARSE-ATTENTION

Triton kernels in `external/competitors/vllm/vllm/models/deepseek_v41/common/`:

- [ ] `engram.py` → PARITY-MODEL-SPECIFIC-LAYERS

Triton kernels in `external/competitors/vllm/vllm/models/deepseek_v41/common/ops/`:

- [ ] `cache_utils.py` → PARITY-KV-CACHE-QUANT
- [ ] `fused_compress_quant_cache.py` → PARITY-SPARSE-ATTENTION
- [ ] `indexer_k_store.py` → PARITY-SPARSE-ATTENTION
- [ ] `query_quant.py` → PARITY-SPARSE-ATTENTION

Triton kernels in `external/competitors/vllm/vllm/models/deepseek_v41/nvidia/`:

- [ ] `model_state.py` → SERVE-SPEC-DECODE

Triton kernels in `external/competitors/vllm/vllm/models/deepseek_v41/nvidia/ops/`:

- [ ] `engram_peer.py` → PARITY-MODEL-SPECIFIC-LAYERS

Triton kernels in `external/competitors/vllm/vllm/models/deepseek_v41/nvidia/specialized_decode/attention/`:

- [ ] `candidate_topk.py` → PARITY-SPARSE-ATTENTION
- [ ] `indexer_k_store.py` → PARITY-SPARSE-ATTENTION

Triton kernels in `external/competitors/vllm/vllm/models/dots3_note/nvidia/`:

- [ ] `attention.py` → PARITY-ATTENTION-VARIANTS-INLINE

Triton kernels in `external/competitors/vllm/vllm/models/flint/ops/`:

- [ ] `moe.py` → PARITY-MOE-DEFERRED-FINALIZE
- [ ] `norm.py` → PARITY-GEMV-NORM-EPILOGUE
- [ ] `nvfp4_gemv.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `qk_norm_rope.py` → PARITY-QKV-HEAD-EPILOGUE

Triton kernels in `external/competitors/vllm/vllm/models/glm5next/amd/ops/`:

- [ ] `kpool_compress.py` → TRITON-AMD-SHIM

Triton kernels in `external/competitors/vllm/vllm/models/glm5next/amd/ops/third_party/kda/`:

- [ ] `fused_recurrent.py` → TRITON-AMD-SHIM
- [ ] `kernels.py` → TRITON-AMD-SHIM

Triton kernels in `external/competitors/vllm/vllm/models/glm5next/nvidia/ops/`:

- [ ] `fused_eh_norm.py` → PARITY-EMBED-NORM
- [ ] `kpool_compress.py` → PARITY-SPARSE-ATTENTION, PARITY-KV-CACHE-QUANT

Triton kernels in `external/competitors/vllm/vllm/models/glm5next/nvidia/ops/third_party/kda/`:

- [ ] `fused_recurrent.py` → PARITY-KDA
- [ ] `kernels.py` → PARITY-KDA

Triton kernels in `external/competitors/vllm/vllm/models/hy_v4/amd/`:

- [ ] `triton_ihc.py` → TRITON-AMD-SHIM

Triton kernels in `external/competitors/vllm/vllm/models/hy_v4/nvidia/`:

- [ ] `triton_ihc.py` → PARITY-HYPER-CONNECTIONS

Triton kernels in `external/competitors/vllm/vllm/models/inkling/amd/`:

- [ ] `moe.py` → TRITON-AMD-SHIM

Triton kernels in `external/competitors/vllm/vllm/models/inkling/amd/ops/`:

- [ ] `fa4_rel_attention.py` → TRITON-AMD-SHIM
- [ ] `norm.py` → TRITON-AMD-SHIM
- [ ] `qkvr_prep.py` → TRITON-AMD-SHIM
- [ ] `rel_attention_decode.py` → TRITON-AMD-SHIM
- [ ] `sconv.py` → TRITON-AMD-SHIM
- [ ] `silu_and_mul.py` → TRITON-AMD-SHIM

Triton kernels in `external/competitors/vllm/vllm/models/inkling/nvidia/`:

- [ ] `moe.py` → PARITY-ROUTING-VARIANTS

Triton kernels in `external/competitors/vllm/vllm/models/inkling/nvidia/ops/`:

- [ ] `lamport.py` → PARITY-ALLREDUCE-NORM
- [ ] `mm_towers.py` → PARITY-MULTIMODAL-TOWERS
- [ ] `norm.py` → PARITY-EMBED-NORM, PARITY-DUAL-NORM
- [ ] `qkvr_prep.py` → PARITY-MODEL-SPECIFIC-LAYERS, PARITY-QKV-HEAD-EPILOGUE
- [ ] `sconv.py` → PARITY-SHORT-CONV
- [ ] `silu_and_mul.py` → PARITY-ACTIVATION-KINDS

Triton kernels in `external/competitors/vllm/vllm/models/kimi_k3/amd/`:

- [ ] `kda_metadata.py` → TRITON-AMD-SHIM

Triton kernels in `external/competitors/vllm/vllm/models/kimi_k3/amd/ops/`:

- [ ] `attn_res.py` → TRITON-AMD-SHIM

Triton kernels in `external/competitors/vllm/vllm/models/kimi_k3/amd/ops/third_party/kda/`:

- [ ] `chunk.py` → TRITON-AMD-SHIM
- [ ] `chunk_intra.py` → TRITON-AMD-SHIM
- [ ] `chunk_intra_token_parallel.py` → TRITON-AMD-SHIM
- [ ] `fused_recurrent.py` → TRITON-AMD-SHIM

Triton kernels in `external/competitors/vllm/vllm/models/kimi_k3/common/`:

- [ ] `mtp.py` → PARITY-EMBED-NORM

Triton kernels in `external/competitors/vllm/vllm/models/kimi_k3/nvidia/`:

- [ ] `kda_metadata.py` → PARITY-KDA

Triton kernels in `external/competitors/vllm/vllm/models/kimi_k3/nvidia/ops/`:

- [ ] `attn_res.py` → PARITY-ATTN-RESIDUAL-NORM
- [ ] `recoverssm.py` → SERVE-SPEC-DECODE, PARITY-KDA

Triton kernels in `external/competitors/vllm/vllm/models/kimi_k3/nvidia/ops/third_party/kda/`:

- [ ] `chunk.py` → PARITY-KDA
- [ ] `chunk_intra.py` → PARITY-KDA
- [ ] `chunk_intra_token_parallel.py` → PARITY-KDA
- [ ] `fused_recurrent.py` → PARITY-KDA

Triton kernels in `external/competitors/vllm/vllm/models/minimax_m3/amd/ops/`:

- [ ] `gemma_rmsnorm.py` → TRITON-AMD-SHIM
- [ ] `index_topk.py` → TRITON-AMD-SHIM
- [ ] `indexer_context_parallel.py` → TRITON-AMD-SHIM
- [ ] `indexer_cp_exchange.py` → TRITON-AMD-SHIM
- [ ] `sparse_attn.py` → TRITON-AMD-SHIM
- [ ] `sparse_pa.py` → TRITON-AMD-SHIM
- [ ] `swiglu_oai.py` → TRITON-AMD-SHIM

Triton kernels in `external/competitors/vllm/vllm/models/minimax_m3/common/ops/`:

- [ ] `index_topk.py` → PARITY-SPARSE-ATTENTION
- [ ] `sparse_attn.py` → PARITY-SPARSE-ATTENTION

Triton kernels in `external/competitors/vllm/vllm/models/minimax_m3/nvidia/`:

- [ ] `msa_cutlass_sparse_decode.py` → PARITY-SPARSE-ATTENTION

Triton kernels in `external/competitors/vllm/vllm/models/muse_spark/amd/ops/`:

- [ ] `fused_norm_expand.py` → TRITON-AMD-SHIM
- [ ] `router_bf16x3.py` → TRITON-AMD-SHIM

Triton kernels in `external/competitors/vllm/vllm/models/muse_spark/common/ops/`:

- [ ] `fused_finalize.py` → PARITY-MOE-DEFERRED-FINALIZE
- [ ] `fused_rmsnorm.py` → TECH-COMPENSATED-RESIDUAL
- [ ] `fused_routing.py` → PARITY-ROUTING-VARIANTS

Triton kernels in `external/competitors/vllm/vllm/models/qwen4_exp/amd/ops/`:

- [ ] `hc.py` → TRITON-AMD-SHIM
- [ ] `qsa.py` → TRITON-AMD-SHIM
- [ ] `qsa_prepare.py` → TRITON-AMD-SHIM

Triton kernels in `external/competitors/vllm/vllm/models/qwen4_exp/common/`:

- [ ] `ngram_embedding.py` → PARITY-MODEL-SPECIFIC-LAYERS
- [ ] `qsa_cache.py` → PARITY-SPARSE-ATTENTION

Triton kernels in `external/competitors/vllm/vllm/models/qwen4_exp/common/ops/`:

- [ ] `ple.py` → PARITY-MODEL-SPECIFIC-LAYERS

Triton kernels in `external/competitors/vllm/vllm/models/qwen4_exp/nvidia/ops/`:

- [ ] `hc.py` → PARITY-HYPER-CONNECTIONS
- [ ] `qsa.py` → PARITY-SPARSE-ATTENTION
- [ ] `qsa_indexer.py` → PARITY-SPARSE-ATTENTION
- [ ] `qsa_prepare.py` → PARITY-SPARSE-ATTENTION

Triton kernels in `external/competitors/vllm/vllm/models/step5/nvidia/`:

- [ ] `csa_tail_ring.py` → PARITY-SPARSE-ATTENTION

Triton kernels in `external/competitors/vllm/vllm/models/step5/nvidia/ops/`:

- [ ] `add.py` → PARITY-ELEMENTWISE-REGIONS
- [ ] `attention_gate.py` → PARITY-QK-NORM-ROPE-GATE-SPLIT
- [ ] `indexer_norm_rope.py` → PARITY-SPARSE-ATTENTION
- [ ] `mtp_norm.py` → PARITY-EMBED-NORM
- [ ] `prefill_indexer_metadata.py` → PARITY-SPARSE-ATTENTION
- [ ] `qkv_indexer_norm_rope.py` → PARITY-SPARSE-ATTENTION
- [ ] `sparse_decode_attention.py` → PARITY-SPARSE-ATTENTION
- [ ] `sparse_indexer.py` → PARITY-SPARSE-ATTENTION
- [ ] `sparse_indexer_scores.py` → PARITY-SPARSE-ATTENTION
- [ ] `sparse_prefill.py` → PARITY-SPARSE-ATTENTION
- [ ] `sparse_prefill_pages.py` → PARITY-SPARSE-ATTENTION
- [ ] `summary_update.py` → PARITY-SPARSE-ATTENTION
- [ ] `swa_decode.py` → PARITY-ATTENTION-VARIANTS-INLINE

Triton kernels in `external/competitors/vllm/vllm/third_party/flash_linear_attention/ops/`:

- [ ] `chunk_delta_h.py` → PARITY-GDN-CHUNK-TENSOR-CORES
- [ ] `chunk_o.py` → PARITY-GDN-CHUNK-TENSOR-CORES
- [ ] `chunk_scaled_dot_kkt.py` → PARITY-GDN-CHUNK-TENSOR-CORES
- [ ] `cumsum.py` → PARITY-GDN-CHUNK-TENSOR-CORES
- [ ] `fused_gdn_decode_post_conv_mtp.py` → PARITY-GDN-DECODE-WHOLE
- [ ] `fused_gdn_prefill_post_conv.py` → PARITY-GDN-POST-CONV-PREP
- [ ] `fused_norm_gate.py` → PARITY-GDN-DECODE-WHOLE
- [ ] `fused_recurrent.py` → PARITY-GDN-DECODE-WHOLE
- [ ] `fused_sigmoid_gating.py` → PARITY-GDN-POST-CONV-PREP
- [ ] `kda.py` → PARITY-KDA
- [ ] `l2norm.py` → PARITY-GDN-POST-CONV-PREP
- [ ] `layernorm_guard.py` → PARITY-GDN-DECODE-WHOLE
- [ ] `op.py` → PARITY-GDN-CHUNK-TENSOR-CORES
- [ ] `solve_tril.py` → PARITY-GDN-CHUNK-TENSOR-CORES
- [ ] `wy_fast.py` → PARITY-GDN-CHUNK-TENSOR-CORES

Triton kernels in `external/competitors/vllm/vllm/v1/attention/backends/`:

- [ ] `flashinfer.py` → PARITY-ATTN-GQA-ROW-PACKING
- [ ] `rocm_aiter_fa.py` → TRITON-AMD-SHIM
- [ ] `utils.py` → SERVE-PAGED-KV

Triton kernels in `external/competitors/vllm/vllm/v1/attention/backends/mla/`:

- [ ] `compressor_utils.py` → PARITY-MLA-UNABSORB-IN-FOLD
- [ ] `flashinfer_mla_sparse_sm90.py` → PARITY-SPARSE-ATTENTION
- [ ] `indexer.py` → PARITY-SPARSE-ATTENTION
- [ ] `rocm_aiter_mla.py` → TRITON-AMD-SHIM
- [ ] `rocm_aiter_mla_sparse.py` → TRITON-AMD-SHIM
- [ ] `sparse_swa.py` → PARITY-SPARSE-ATTENTION
- [ ] `sparse_utils.py` → PARITY-SPARSE-ATTENTION

Triton kernels in `external/competitors/vllm/vllm/v1/attention/ops/`:

- [ ] `chunked_prefill_paged_decode.py` → PARITY-MIXED-BATCH-ATTENTION
- [ ] `common.py` → SERVE-PAGED-KV
- [ ] `dcp.py` → PARITY-TENSOR-PARALLEL-IR
- [ ] `flydsl_turboquant_decode.py` → TRITON-AMD-SHIM
- [ ] `flydsl_ultraquant_decode.py` → TRITON-AMD-SHIM
- [ ] `int4_per_token_head.py` → PARITY-KV-CACHE-QUANT
- [ ] `metadata.py` → SERVE-PAGED-KV
- [ ] `prefix_prefill.py` → PARITY-MIXED-BATCH-ATTENTION
- [ ] `rocm_aiter_mla_merge.py` → TRITON-AMD-SHIM
- [ ] `rocm_aiter_mla_sparse.py` → TRITON-AMD-SHIM
- [ ] `rocm_paged_mxfp4_indexer.py` → TRITON-AMD-SHIM
- [ ] `segmented_attention.py` → TRITON-AMD-SHIM
- [ ] `triton_attention_helpers.py` → PARITY-ATTENTION-VARIANTS-INLINE
- [ ] `triton_decode_attention.py` → PARITY-ATTN-GQA-ROW-PACKING
- [ ] `triton_merge_attn_states.py` → PARITY-ATTN-LSE-MERGE
- [ ] `triton_prefill_attention.py` → PARITY-PREFILL-ATTENTION-PEAK
- [ ] `triton_reshape_and_cache_flash.py` → PARITY-KV-CACHE-QUANT
- [ ] `triton_turboquant_decode.py` → PARITY-KV-CACHE-QUANT
- [ ] `triton_turboquant_store.py` → PARITY-KV-CACHE-QUANT
- [ ] `triton_unified_attention.py` → PARITY-ATTN-GQA-ROW-PACKING
- [ ] `triton_unified_attention_diffkv.py` → PARITY-KV-CACHE-QUANT
- [ ] `xpu_mla_sparse.py` → PARITY-XPU-SHIM

Triton kernels in `external/competitors/vllm/vllm/v1/attention/ops/turboquant_soa/`:

- [ ] `triton_turboquant_decode.py` → PARITY-KV-CACHE-QUANT
- [ ] `triton_turboquant_store.py` → PARITY-KV-CACHE-QUANT
- [ ] `triton_turboquant_unified_attention.py` → PARITY-KV-CACHE-QUANT

Triton kernels in `external/competitors/vllm/vllm/v1/attention/ops/ultraquant/`:

- [ ] `triton_dequant.py` → PARITY-KV-CACHE-QUANT
- [ ] `triton_store.py` → PARITY-KV-CACHE-QUANT
- [ ] `triton_unified_attention.py` → PARITY-KV-CACHE-QUANT

Triton kernels in `external/competitors/vllm/vllm/v1/kv_offload/cpu/`:

- [ ] `swap_blocks_triton.py` → SERVE-KV-OFFLOAD

Triton kernels in `external/competitors/vllm/vllm/v1/pool/flash_maxsim/`:

- [ ] `flash_maxsim_rerank.py` → SERVE-POOLING

Triton kernels in `external/competitors/vllm/vllm/v1/sample/ops/`:

- [ ] `topk_topp_triton.py` → PARITY-SAMPLING

Triton kernels in `external/competitors/vllm/vllm/v1/sample/`:

- [ ] `rejection_sampler.py` → SERVE-SPEC-DECODE

Triton kernels in `external/competitors/vllm/vllm/v1/spec_decode/`:

- [ ] `fly.py` → SERVE-SPEC-DECODE
- [ ] `utils.py` → SERVE-SPEC-DECODE

Triton kernels in `external/competitors/vllm/vllm/v1/worker/`:

- [ ] `block_table.py` → SERVE-CONTINUOUS-BATCHING
- [ ] `mamba_utils.py` → PARITY-SSD-CHUNK-SCAN
- [ ] `utils.py` → SERVE-CONTINUOUS-BATCHING

Triton kernels in `external/competitors/vllm/vllm/v1/worker/gpu/`:

- [ ] `block_table.py` → SERVE-CONTINUOUS-BATCHING
- [ ] `buffer_utils.py` → SERVE-CONTINUOUS-BATCHING
- [ ] `cp_utils.py` → PARITY-TENSOR-PARALLEL-IR
- [ ] `input_batch.py` → SERVE-CONTINUOUS-BATCHING
- [ ] `structured_outputs.py` → SERVE-STRUCTURED-OUTPUT

Triton kernels in `external/competitors/vllm/vllm/v1/worker/gpu/metrics/`:

- [ ] `logits.py` → SERVE-CONTINUOUS-BATCHING

Triton kernels in `external/competitors/vllm/vllm/v1/worker/gpu/mm/`:

- [ ] `rope.py` → PARITY-ROPE-VARIANTS, PARITY-MULTIMODAL-TOWERS

Triton kernels in `external/competitors/vllm/vllm/v1/worker/gpu/model_states/`:

- [ ] `mamba_hybrid.py` → PARITY-SSD-CHUNK-SCAN
- [ ] `prompt_embeds.py` → SERVE-CONTINUOUS-BATCHING
- [ ] `recoverssm.py` → SERVE-SPEC-DECODE

Triton kernels in `external/competitors/vllm/vllm/v1/worker/gpu/sample/`:

- [ ] `bad_words.py` → SERVE-STRUCTURED-OUTPUT
- [ ] `batch_shard.py` → PARITY-SAMPLING
- [ ] `gumbel.py` → PARITY-SAMPLING
- [ ] `logit_bias.py` → PARITY-SAMPLING
- [ ] `logprob.py` → PARITY-SAMPLING
- [ ] `min_p.py` → PARITY-SAMPLING
- [ ] `output.py` → PARITY-SAMPLING
- [ ] `penalties.py` → PARITY-SAMPLING
- [ ] `prompt_logprob.py` → PARITY-SAMPLING
- [ ] `thinking_budget.py` → PARITY-SAMPLING
- [ ] `trace_replay.py` → PARITY-SAMPLING
- [ ] `watermark.py` → PARITY-SAMPLING

Triton kernels in `external/competitors/vllm/vllm/v1/worker/gpu/spec_decode/`:

- [ ] `acceptance_estimator.py` → SERVE-SPEC-DECODE
- [ ] `rejection_sampler.py` → SERVE-SPEC-DECODE
- [ ] `rejection_sampler_utils.py` → SERVE-SPEC-DECODE

Triton kernels in `external/competitors/vllm/vllm/v1/worker/gpu/spec_decode/dflash/`:

- [ ] `speculator.py` → SERVE-SPEC-DECODE

Triton kernels in `external/competitors/vllm/vllm/v1/worker/gpu/spec_decode/dflash2/`:

- [ ] `speculator.py` → SERVE-SPEC-DECODE

Triton kernels in `external/competitors/vllm/vllm/v1/worker/gpu/spec_decode/multi_module_mtp/`:

- [ ] `speculator.py` → SERVE-SPEC-DECODE

Triton kernels in `external/competitors/vllm/vllm/v1/worker/gpu/spec_decode/ngram/`:

- [ ] `speculator.py` → SERVE-SPEC-DECODE

Triton kernels in `external/competitors/vllm/vllm/v1/worker/gpu/spec_decode/standalone_ar/`:

- [ ] `speculator.py` → SERVE-SPEC-DECODE

Triton kernels in `external/competitors/vllm/vllm/v1/worker/gpu/spec_decode/target_dependent_ar/`:

- [ ] `speculator.py` → SERVE-SPEC-DECODE

Triton kernels in `external/competitors/vllm/vllm/v1/worker/gpu/spec_decode/xpress/`:

- [ ] `kernels.py` → SERVE-SPEC-DECODE

### 11.9 vLLM: speculative decoding and sampling

#### Speculative decoding (17)

Every module under `vllm/v1/spec_decode/`.

In `external/competitors/vllm/vllm/v1/spec_decode/`:

- [ ] `custom_class_proposer.py` → SERVE-SPEC-DECODE
- [ ] `dflash.py` → SERVE-SPEC-DECODE
- [ ] `draft_model.py` → SERVE-SPEC-DECODE
- [ ] `eagle.py` → SERVE-SPEC-DECODE
- [ ] `extract_hidden_states.py` → SERVE-SPEC-DECODE
- [ ] `fly.py` → SERVE-SPEC-DECODE
- [ ] `gemma4.py` → SERVE-SPEC-DECODE
- [ ] `llm_base_proposer.py` → SERVE-SPEC-DECODE
- [ ] `metadata.py` → SERVE-SPEC-DECODE
- [ ] `metrics.py` → SERVE-SPEC-DECODE
- [ ] `ngram_proposer.py` → SERVE-SPEC-DECODE
- [ ] `ngram_proposer_gpu.py` → SERVE-SPEC-DECODE
- [ ] `step3p5.py` → SERVE-SPEC-DECODE
- [ ] `suffix_decoding.py` → SERVE-SPEC-DECODE
- [ ] `utils.py` → SERVE-SPEC-DECODE
- [ ] `vocab_mapping.py` → SERVE-SPEC-DECODE

In `external/competitors/vllm/vllm/v1/spec_decode/dynamic/`:

- [ ] `utils.py` → SERVE-SPEC-DECODE

#### Sampling (21)

Every module under `vllm/v1/sample/ops/` and `vllm/v1/worker/gpu/sample/`.

In `external/competitors/vllm/vllm/v1/sample/ops/`:

- [ ] `bad_words.py` → SERVE-STRUCTURED-OUTPUT
- [ ] `logprobs.py` → PARITY-SAMPLING
- [ ] `penalties.py` → PARITY-SAMPLING
- [ ] `topk_topp_sampler.py` → PARITY-SAMPLING
- [ ] `topk_topp_triton.py` → PARITY-SAMPLING

In `external/competitors/vllm/vllm/v1/worker/gpu/sample/`:

- [ ] `bad_words.py` → SERVE-STRUCTURED-OUTPUT
- [ ] `batch_shard.py` → PARITY-SAMPLING
- [ ] `gumbel.py` → PARITY-SAMPLING
- [ ] `logit_bias.py` → PARITY-SAMPLING
- [ ] `logprob.py` → PARITY-SAMPLING
- [ ] `min_p.py` → PARITY-SAMPLING
- [ ] `output.py` → PARITY-SAMPLING
- [ ] `penalties.py` → PARITY-SAMPLING
- [ ] `prompt_logprob.py` → PARITY-SAMPLING
- [ ] `sampler.py` → PARITY-SAMPLING
- [ ] `states.py` → PARITY-SAMPLING
- [ ] `thinking_budget.py` → PARITY-SAMPLING
- [ ] `trace_replay.py` → PARITY-SAMPLING
- [ ] `watermark.py` → PARITY-SAMPLING

In `external/competitors/vllm/vllm/v1/worker/gpu/sample/logits_processor/`:

- [ ] `interface.py` → PARITY-SAMPLING
- [ ] `loader.py` → PARITY-SAMPLING

### 11.10 Triton: the compiler pipelines

#### NVIDIA (73)

Every distinct `passes.<dialect>.add_<pass>` the NVIDIA backend adds, in pipeline order.

In `external/competitors/triton/third_party/nvidia/backend/compiler.py`:

- [ ] `common.inliner` → TRITON-COMBINE-REWRITES
- [ ] `ttir.rewrite_tensor_descriptor_to_pointer` → TRITON-TMA
- [ ] `common.canonicalizer` → TRITON-COMBINE-REWRITES
- [ ] `ttir.combine` → TRITON-COMBINE-REWRITES
- [ ] `ttir.reorder_broadcast` → TRITON-COMBINE-REWRITES
- [ ] `common.cse` → TRITON-COMBINE-REWRITES
- [ ] `common.symbol_dce` → TRITON-COMBINE-REWRITES
- [ ] `ttir.loop_unroll` → TRITON-COMBINE-REWRITES
- [ ] `ttir.convert_to_ttgpuir` → TRITON-GLOBAL-LAYOUT-ASSIGNMENT
- [ ] `ttnvgpuir.to_clc` → TECH-CLUSTER-LAUNCH-CONTROL
- [ ] `ttgpuir.coalesce` → TRITON-COALESCE
- [ ] `ttgpuir.f32_dot_tc` → TECH-SPLIT-PRECISION-MMA
- [ ] `ttnvgpuir.assign_cga_layouts` → TRITON-GLOBAL-LAYOUT-ASSIGNMENT
- [ ] `ttgpuir.remove_layout_conversions` → TRITON-GLOBAL-LAYOUT-ASSIGNMENT
- [ ] `ttnvgpuir.optimize_cta_locality` → TRITON-GLOBAL-LAYOUT-ASSIGNMENT
- [ ] `ttgpuir.optimize_thread_locality` → TRITON-THREAD-LOCALITY
- [ ] `ttgpuir.accelerate_matmul` → TRITON-MMA-SELECTION
- [ ] `ttnvgpuir.check_matmul_two_cta` → TRITON-MMA-SELECTION
- [ ] `ttgpuir.optimize_dot_operands` → TRITON-TRANSPOSE-AS-ADDRESSING
- [ ] `ttnvgpuir.optimize_descriptor_encoding` → TRITON-TMA
- [ ] `ttir.loop_aware_cse` → TRITON-COMBINE-REWRITES
- [ ] `ttgpuir.fuse_nested_loops` → TRITON-FUSE-NESTED-LOOPS
- [ ] `ttir.triton_licm` → TRITON-COMBINE-REWRITES
- [ ] `ttgpuir.combine_tensor_select_and_if` → TRITON-COMBINE-REWRITES
- [ ] `hopper.hopper_warpspec` → TRITON-WARP-SPECIALIZATION
- [ ] `ttgpuir.assign_latencies` → TRITON-MODULO-SCHEDULER
- [ ] `ttgpuir.schedule_loops` → TRITON-MODULO-SCHEDULER
- [ ] `ttgpuir.pipeline` → TRITON-MODULO-SCHEDULER
- [ ] `ttgpuir.optimize_accumulator_init` → TRITON-ACCUMULATOR-INIT
- [ ] `ttgpuir.hoist_tmem_alloc` → TECH-2SM-UMMA-TMEM
- [ ] `ttnvgpuir.promote_lhs_to_tmem` → TECH-2SM-UMMA-TMEM
- [ ] `ttgpuir.warp_specialize` → TRITON-WARP-SPECIALIZATION
- [ ] `ttgpuir.optimize_partition_warps` → TRITON-WARP-SPECIALIZATION
- [ ] `ttnvgpuir.remove_tmem_tokens` → TECH-2SM-UMMA-TMEM
- [ ] `ttgpuir.prefetch` → TRITON-OPERAND-PREFETCH
- [ ] `ttgpuir.coalesce_async_copy` → TRITON-COALESCE
- [ ] `ttnvgpuir.optimize_tmem_layouts` → TECH-2SM-UMMA-TMEM
- [ ] `ttnvgpuir.tmem_load_reduce` → TECH-2SM-UMMA-TMEM
- [ ] `ttnvgpuir.tma_lowering` → TRITON-TMA
- [ ] `ttnvgpuir.lower_clc` → TECH-CLUSTER-LAUNCH-CONTROL
- [ ] `ttnvgpuir.interleave_tmem` → TECH-2SM-UMMA-TMEM
- [ ] `ttgpuir.reduce_data_duplication` → TRITON-SMEM-STAGING-NODES
- [ ] `ttgpuir.reorder_instructions` → TRITON-REGISTER-PRESSURE-ORDER
- [ ] `ttnvgpuir.fence_insertion` → TRITON-REGION-BARRIERS
- [ ] `ttnvgpuir.lower_mma` → TRITON-MMA-SELECTION
- [ ] `ttnvgpuir.hoist_mbarrier_lifecycle` → TRITON-REGION-BARRIERS
- [ ] `common.sccp` → TRITON-COMBINE-REWRITES
- [ ] `ttgpuir.fp_sanitizer` → OPT-5
- [ ] `gluon.inliner` → TRITON-COMBINE-REWRITES
- [ ] `gluon.infer_coalesced_encodings` → TRITON-COALESCE
- [ ] `gluon.resolve_auto_encodings` → TRITON-GLOBAL-LAYOUT-ASSIGNMENT
- [ ] `gluon.canonicalizer` → TRITON-COMBINE-REWRITES
- [ ] `ttgpuir.global_sanitizer` → OPT-5
- [ ] `ttgpuir.allocate_warp_groups` → TRITON-WARP-SPECIALIZATION
- [ ] `convert.scf_to_cf` → TRITON-TILE-TIER
- [ ] `ttgpuir.prepare_consan_captures` → OPT-5
- [ ] `ttgpuir.allocate_shared_memory_nv` → TRITON-SMEM-COLORING-ALLOCATOR
- [ ] `ttnvgpuir.allocate_tensor_memory` → TECH-2SM-UMMA-TMEM
- [ ] `ttnvgpuir.proxy_fence_insertion` → TRITON-REGION-BARRIERS
- [ ] `ttgpuir.membar` → TRITON-REGION-BARRIERS
- [ ] `ttnvgpuir.tmem_barrier_insertion` → TECH-2SM-UMMA-TMEM
- [ ] `ttnvgpuir.optimize_mbarrier_arrivals` → TRITON-REGION-BARRIERS
- [ ] `ttgpuir.concurrency_sanitizer` → OPT-5
- [ ] `ttnvgpuir.cluster_barrier_mbar_allocator` → TRITON-REGION-BARRIERS
- [ ] `ttgpuir.allocate_global_scratch_memory` → TRITON-SMEM-COLORING-ALLOCATOR
- [ ] `ttgpuir.to_llvmir` → TRITON-TILE-TIER
- [ ] `ttnvgpuir.initialize_ws_cluster_barriers` → TRITON-WARP-SPECIALIZATION
- [ ] `ttgpuir.canonicalize_llvm_ir` → TRITON-COMBINE-REWRITES
- [ ] `ttnvgpuir.warp_specialize_to_llvm` → TRITON-WARP-SPECIALIZATION
- [ ] `ttnvgpuir.nvgpu_to_llvm` → TRITON-TILE-TIER
- [ ] `convert.nvvm_to_llvm` → TRITON-TILE-TIER
- [ ] `llvmir.di_scope` → TRITON-TILE-TIER
- [ ] `llvmir.di_local_variable` → TRITON-TILE-TIER

#### AMD (61)

Every distinct `passes.<dialect>.add_<pass>` the AMD backend adds, in pipeline order.

In `external/competitors/triton/third_party/amd/backend/compiler.py`:

- [ ] `common.inliner` → TRITON-COMBINE-REWRITES
- [ ] `ttir.rewrite_tensor_descriptor_to_pointer` → TRITON-TMA
- [ ] `common.canonicalizer` → TRITON-COMBINE-REWRITES
- [ ] `ttir.combine` → TRITON-COMBINE-REWRITES
- [ ] `ttir.reorder_broadcast` → TRITON-COMBINE-REWRITES
- [ ] `common.cse` → TRITON-COMBINE-REWRITES
- [ ] `ttir.triton_licm` → TRITON-COMBINE-REWRITES
- [ ] `common.symbol_dce` → TRITON-COMBINE-REWRITES
- [ ] `ttir.loop_unroll` → TRITON-COMBINE-REWRITES
- [ ] `ttir.convert_to_ttgpuir` → TRITON-GLOBAL-LAYOUT-ASSIGNMENT
- [ ] `ttgpuir.coalesce` → TRITON-COALESCE
- [ ] `ttgpuir.f32_dot_tc` → TECH-SPLIT-PRECISION-MMA
- [ ] `ttgpuir.remove_layout_conversions` → TRITON-GLOBAL-LAYOUT-ASSIGNMENT
- [ ] `ttgpuir.optimize_thread_locality` → TRITON-THREAD-LOCALITY
- [ ] `ttgpuir.accelerate_matmul` → TRITON-MMA-SELECTION
- [ ] `ttgpuir.optimize_epilogue` → TRITON-AMD-SHIM
- [ ] `ttgpuir.optimize_dot_operands` → TRITON-TRANSPOSE-AS-ADDRESSING
- [ ] `ttgpuir.hoist_layout_conversions` → TRITON-GLOBAL-LAYOUT-ASSIGNMENT
- [ ] `ttgpuir.sink_layout_conversions` → TRITON-GLOBAL-LAYOUT-ASSIGNMENT
- [ ] `ttgpuir.fuse_nested_loops` → TRITON-FUSE-NESTED-LOOPS
- [ ] `ttgpuir.optimize_descriptor_encoding` → TRITON-TMA
- [ ] `ttgpuir.schedule_loops` → TRITON-MODULO-SCHEDULER
- [ ] `ttgpuir.pipeline` → TRITON-MODULO-SCHEDULER
- [ ] `ttgpuir.coalesce_async_copy` → TRITON-COALESCE
- [ ] `ttgpuir.convert_to_tensor_ops` → TRITON-AMD-SHIM
- [ ] `ttgpuir.reduce_data_duplication` → TRITON-SMEM-STAGING-NODES
- [ ] `ttgpuir.in_thread_transpose` → TRITON-AMD-SHIM
- [ ] `ttgpuir.move_up_prologue_loads` → TRITON-AMD-SHIM
- [ ] `ttgpuir.block_pingpong` → TRITON-TILE-TIER
- [ ] `ttgpuir.canonicalize_pointers` → TRITON-AMD-SHIM
- [ ] `ttgpuir.convert_to_buffer_ops` → TRITON-AMD-SHIM
- [ ] `ttgpuir.optimize_buffer_op_ptr` → TRITON-AMD-SHIM
- [ ] `ttgpuir.fold_true_cmpi` → TRITON-AMD-SHIM
- [ ] `ttgpuir.prepare_if_combining` → TRITON-AMD-SHIM
- [ ] `ttgpuir.fp_sanitizer` → OPT-5
- [ ] `gluon.inliner` → TRITON-COMBINE-REWRITES
- [ ] `gluon.infer_coalesced_encodings` → TRITON-COALESCE
- [ ] `gluon.resolve_auto_encodings` → TRITON-GLOBAL-LAYOUT-ASSIGNMENT
- [ ] `common.sccp` → TRITON-COMBINE-REWRITES
- [ ] `ttir.loop_aware_cse` → TRITON-COMBINE-REWRITES
- [ ] `gluon.canonicalizer` → TRITON-COMBINE-REWRITES
- [ ] `ttgpuir.combine_tensor_select_and_if` → TRITON-COMBINE-REWRITES
- [ ] `ttgpuir.warp_pipeline` → TRITON-AMD-SHIM
- [ ] `ttgpuir.allocate_warp_groups` → TRITON-WARP-SPECIALIZATION
- [ ] `ttgpuir.update_async_wait_count` → TRITON-AMD-SHIM
- [ ] `ttgpuir.warp_pipeline_conversion` → TRITON-AMD-SHIM
- [ ] `convert.scf_to_cf` → TRITON-TILE-TIER
- [ ] `convert.index_to_llvmir` → TRITON-TILE-TIER
- [ ] `ttgpuir.prepare_consan_captures` → OPT-5
- [ ] `ttgpuir.allocate_shared_memory` → TRITON-SMEM-COLORING-ALLOCATOR
- [ ] `ttgpuir.membar` → TRITON-REGION-BARRIERS
- [ ] `ttgpuir.concurrency_sanitizer` → OPT-5
- [ ] `ttgpuir.allocate_global_scratch_memory` → TRITON-SMEM-COLORING-ALLOCATOR
- [ ] `ttgpuir.to_llvmir` → TRITON-TILE-TIER
- [ ] `ttgpuir.warp_specialize_to_llvm` → TRITON-WARP-SPECIALIZATION
- [ ] `convert.cf_to_llvmir` → TRITON-TILE-TIER
- [ ] `convert.arith_to_llvmir` → TRITON-TILE-TIER
- [ ] `llvmir.di_scope` → TRITON-TILE-TIER
- [ ] `ttgpuir.builtin_func_to_llvmir` → TRITON-AMD-SHIM
- [ ] `convert.reconcile_unrealized_casts` → TRITON-TILE-TIER
- [ ] `llvmir.di_local_variable` → TRITON-TILE-TIER

### 11.11 Triton: the language

116 entries.

Every public builtin of `triton.language`: `@builtin` in `core.py`, `@core.builtin` in `math.py`, and the `@jit` functions of `standard.py` and `random.py`. The generated-kernel tier (TRITON-TILE-TIER) expresses each one, or the ticket named does.

In `external/competitors/triton/python/triton/language/core.py`:

- [ ] `tl.to_tensor` → TRITON-TILE-TIER
- [ ] `tl.program_id` → TRITON-TILE-TIER
- [ ] `tl.num_programs` → TRITON-TILE-TIER
- [ ] `tl.arange` → TRITON-TILE-TIER
- [ ] `tl.full` → TRITON-TILE-TIER
- [ ] `tl.broadcast` → TRITON-LINEAR-LAYOUT
- [ ] `tl.broadcast_to` → TRITON-LINEAR-LAYOUT
- [ ] `tl.trans` → TRITON-TRANSPOSE-AS-ADDRESSING
- [ ] `tl.permute` → TRITON-TRANSPOSE-AS-ADDRESSING
- [ ] `tl.cat` → TRITON-LINEAR-LAYOUT
- [ ] `tl.join` → TRITON-LINEAR-LAYOUT
- [ ] `tl.split` → TRITON-LINEAR-LAYOUT
- [ ] `tl.view` → TRITON-LINEAR-LAYOUT
- [ ] `tl.item` → TRITON-TILE-TIER
- [ ] `tl.reshape` → TRITON-LINEAR-LAYOUT
- [ ] `tl.expand_dims` → TRITON-LINEAR-LAYOUT
- [ ] `tl.cast` → TRITON-TILE-TIER
- [ ] `tl.dot` → TRITON-MMA-SELECTION
- [ ] `tl.dot_scaled` → TRITON-MMA-SELECTION
- [ ] `tl.load` → TRITON-TILE-TIER
- [ ] `tl.load_tensor_descriptor` → TRITON-TMA
- [ ] `tl.store_tensor_descriptor` → TRITON-TMA
- [ ] `tl.store` → TRITON-TILE-TIER
- [ ] `tl.make_block_ptr` → TRITON-TILE-TIER
- [ ] `tl.advance` → TRITON-TILE-TIER
- [ ] `tl.make_tensor_descriptor` → TRITON-TMA
- [ ] `tl.atomic_load` → TRITON-TILE-TIER
- [ ] `tl.atomic_store` → TRITON-TILE-TIER
- [ ] `tl.atomic_cas` → TRITON-TILE-TIER
- [ ] `tl.atomic_poll` → TRITON-TILE-TIER
- [ ] `tl.atomic_xchg` → TRITON-TILE-TIER
- [ ] `tl.atomic_add` → TRITON-TILE-TIER
- [ ] `tl.atomic_max` → TRITON-TILE-TIER
- [ ] `tl.atomic_min` → TRITON-TILE-TIER
- [ ] `tl.atomic_and` → TRITON-TILE-TIER
- [ ] `tl.atomic_or` → TRITON-TILE-TIER
- [ ] `tl.atomic_xor` → TRITON-TILE-TIER
- [ ] `tl.where` → TRITON-TILE-TIER
- [ ] `tl.expect_zero` → TRITON-TILE-TIER
- [ ] `tl.add` → TRITON-TILE-TIER
- [ ] `tl.sub` → TRITON-TILE-TIER
- [ ] `tl.mul` → TRITON-TILE-TIER
- [ ] `tl.minimum` → TRITON-TILE-TIER
- [ ] `tl.maximum` → TRITON-TILE-TIER
- [ ] `tl.clamp` → TRITON-TILE-TIER
- [ ] `tl.reduce` → TRITON-REDUCE-SCAN-CODEGEN
- [ ] `tl.associative_scan` → TRITON-REDUCE-SCAN-CODEGEN
- [ ] `tl.histogram` → TRITON-REDUCE-SCAN-CODEGEN
- [ ] `tl.gather` → TRITON-REDUCE-SCAN-CODEGEN
- [ ] `tl.map_elementwise` → TRITON-TILE-TIER
- [ ] `tl.debug_barrier` → TRITON-REGION-BARRIERS
- [ ] `tl.multiple_of` → TRITON-AXIS-FACTS
- [ ] `tl.max_contiguous` → TRITON-AXIS-FACTS
- [ ] `tl.max_constancy` → TRITON-AXIS-FACTS
- [ ] `tl.assume` → TRITON-AXIS-FACTS
- [ ] `tl.static_print` → OPT-5
- [ ] `tl.static_assert` → OPT-5
- [ ] `tl.device_print` → OPT-5
- [ ] `tl.device_assert` → OPT-5
- [ ] `tl.inline_asm_elementwise` → TECH-INLINE-PTX-PACKING
- [ ] `tl.extern_elementwise` → TECH-INLINE-PTX-PACKING
- [ ] `tl.builtin_max` → TRITON-TILE-TIER
- [ ] `tl.builtin_min` → TRITON-TILE-TIER

In `external/competitors/triton/python/triton/language/math.py`:

- [ ] `tl.umulhi` → TRITON-TILE-TIER
- [ ] `tl.exp` → TRITON-TILE-TIER
- [ ] `tl.exp2` → TECH-EXP2-SCALE-FOLD
- [ ] `tl.log` → TRITON-TILE-TIER
- [ ] `tl.log2` → TRITON-TILE-TIER
- [ ] `tl.cos` → TRITON-TILE-TIER
- [ ] `tl.sin` → TRITON-TILE-TIER
- [ ] `tl.sqrt` → TRITON-TILE-TIER
- [ ] `tl.sqrt_rn` → TRITON-TILE-TIER
- [ ] `tl.rsqrt` → TRITON-TILE-TIER
- [ ] `tl.abs` → TRITON-TILE-TIER
- [ ] `tl.fdiv` → TRITON-TILE-TIER
- [ ] `tl.div_rn` → TRITON-TILE-TIER
- [ ] `tl.erf` → TRITON-TILE-TIER
- [ ] `tl.floor` → TRITON-TILE-TIER
- [ ] `tl.ceil` → TRITON-TILE-TIER
- [ ] `tl.fma` → TRITON-TILE-TIER

In `external/competitors/triton/python/triton/language/standard.py`:

- [ ] `tl.cdiv` → TRITON-TILE-TIER
- [ ] `tl.sigmoid` → PARITY-ACTIVATION-KINDS
- [ ] `tl.softmax` → PARITY-TRITON-SOFTMAX
- [ ] `tl.ravel` → TRITON-LINEAR-LAYOUT
- [ ] `tl.swizzle2d` → PARITY-TRITON-PERSISTENT-MATMUL
- [ ] `tl.zeros` → TRITON-TILE-TIER
- [ ] `tl.zeros_like` → TRITON-TILE-TIER
- [ ] `tl.max` → TRITON-REDUCE-SCAN-CODEGEN
- [ ] `tl.argmax` → TRITON-REDUCE-SCAN-CODEGEN
- [ ] `tl.min` → TRITON-REDUCE-SCAN-CODEGEN
- [ ] `tl.argmin` → TRITON-REDUCE-SCAN-CODEGEN
- [ ] `tl.all` → TRITON-REDUCE-SCAN-CODEGEN
- [ ] `tl.any` → TRITON-REDUCE-SCAN-CODEGEN
- [ ] `tl.sum` → TRITON-REDUCE-SCAN-CODEGEN
- [ ] `tl.xor_sum` → TRITON-REDUCE-SCAN-CODEGEN
- [ ] `tl.reduce_or` → TRITON-REDUCE-SCAN-CODEGEN
- [ ] `tl.cumsum` → TRITON-REDUCE-SCAN-CODEGEN
- [ ] `tl.cumprod` → TRITON-REDUCE-SCAN-CODEGEN
- [ ] `tl.sort_impl` → TRITON-REDUCE-SCAN-CODEGEN
- [ ] `tl.sort` → TRITON-REDUCE-SCAN-CODEGEN
- [ ] `tl.topk` → TRITON-REDUCE-SCAN-CODEGEN
- [ ] `tl.bitonic_merge` → TRITON-REDUCE-SCAN-CODEGEN
- [ ] `tl.flip` → TRITON-REDUCE-SCAN-CODEGEN
- [ ] `tl.interleave` → TRITON-LINEAR-LAYOUT
- [ ] `tl.squeeze` → TRITON-LINEAR-LAYOUT
- [ ] `tl.unsqueeze` → TRITON-LINEAR-LAYOUT

In `external/competitors/triton/python/triton/language/random.py`:

- [ ] `tl.philox_impl` → PARITY-TRITON-DROPOUT
- [ ] `tl.philox` → PARITY-TRITON-DROPOUT
- [ ] `tl.randint` → PARITY-TRITON-DROPOUT
- [ ] `tl.randint4x` → PARITY-TRITON-DROPOUT
- [ ] `tl.uint_to_uniform_float` → PARITY-TRITON-DROPOUT
- [ ] `tl.rand` → PARITY-TRITON-DROPOUT
- [ ] `tl.rand4x` → PARITY-TRITON-DROPOUT
- [ ] `tl.pair_uniform_to_normal` → PARITY-TRITON-DROPOUT
- [ ] `tl.randn` → PARITY-TRITON-DROPOUT
- [ ] `tl.randn4x` → PARITY-TRITON-DROPOUT

### 11.12 Triton: tutorials, Gluon and triton_kernels

#### Tutorials (11)

Every tutorial under `python/tutorials/`.

In `external/competitors/triton/python/tutorials/`:

- [ ] `01-vector-add.py` → TRITON-TILE-TIER
- [ ] `02-fused-softmax.py` → PARITY-TRITON-SOFTMAX
- [ ] `03-matrix-multiplication.py` → PARITY-TRITON-PERSISTENT-MATMUL
- [ ] `04-low-memory-dropout.py` → PARITY-TRITON-DROPOUT
- [ ] `05-layer-norm.py` → PARITY-TRITON-LAYERNORM
- [ ] `06-fused-attention.py` → PARITY-TRITON-ATTENTION
- [ ] `07-extern-functions.py` → TECH-INLINE-PTX-PACKING
- [ ] `08-grouped-gemm.py` → PARITY-TRITON-GROUPED-GEMM
- [ ] `09-persistent-matmul.py` → PARITY-TRITON-PERSISTENT-MATMUL
- [ ] `10-block-scaled-matmul.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `11-programmatic-dependent-launch.py` → TECH-PDL

#### Gluon (24)

Every Gluon example and tutorial.

In `external/competitors/triton/python/examples/gluon/`:

- [ ] `01-attention-forward.py` → PARITY-TRITON-ATTENTION
- [ ] `02-conv-common.py` → TRITON-TMA
- [ ] `02-conv-dgrad.py` → TRITON-TMA
- [ ] `02-conv-fprop.py` → TRITON-TMA
- [ ] `02-conv-wgrad.py` → TRITON-TMA
- [ ] `03-matmul-multicta.py` → TECH-CLUSTER-DSMEM-REDUCE
- [ ] `04-2cta-block-scale-matmul.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `05-moe-bmm1-fused-gather.py` → PARITY-MOE-GATHER-IN-OPERAND
- [ ] `06-overlapping-accumulator.py` → TECH-2SM-UMMA-TMEM

In `external/competitors/triton/python/tutorials/gluon/`:

- [ ] `01-intro.py` → TRITON-LINEAR-LAYOUT
- [ ] `02-layouts.py` → TRITON-LINEAR-LAYOUT
- [ ] `03-async-copy.py` → TRITON-MODULO-SCHEDULER
- [ ] `04-tma.py` → TRITON-TMA
- [ ] `05-wgmma.py` → TRITON-ASYNC-MMA-WAITS
- [ ] `06-tcgen05.py` → TECH-2SM-UMMA-TMEM
- [ ] `07-persistence.py` → TRITON-FUSE-NESTED-LOOPS
- [ ] `08-warp-specialization.py` → TRITON-WARP-SPECIALIZATION
- [ ] `09-tma-gather-scatter.py` → TRITON-TMA
- [ ] `10-tcgen05-copy.py` → TECH-2SM-UMMA-TMEM
- [ ] `11-tcgen05-mma-scaled.py` → PARITY-QUANT-GEMMS-DATACENTER
- [ ] `12-cluster-launch-control.py` → TECH-CLUSTER-LAUNCH-CONTROL
- [ ] `13-conv-im2col.py` → TRITON-TMA
- [ ] `14-multicta.py` → TECH-CLUSTER-DSMEM-REDUCE
- [ ] `conftest.py` → TRITON-TILE-TIER

#### triton_kernels (47)

Every module of `triton_kernels`.

In `external/competitors/triton/python/triton_kernels/triton_kernels/`:

- [ ] `compaction.py` → PARITY-TRITON-MOE-MATMUL
- [ ] `distributed.py` → PARITY-EXPERT-PARALLEL
- [ ] `fpsan.py` → OPT-5
- [ ] `matmul.py` → PARITY-TRITON-MOE-MATMUL
- [ ] `meta.py` → PARITY-TRITON-MOE-MATMUL
- [ ] `numerics.py` → PARITY-TRITON-MX-CONVERT
- [ ] `proton_opts.py` → SETUP-4
- [ ] `reduce.py` → PARITY-TRITON-MOE-MATMUL
- [ ] `roofline.py` → SETUP-4
- [ ] `specialize.py` → OPT-1
- [ ] `swiglu.py` → PARITY-TRITON-SWIGLU
- [ ] `target_info.py` → OPT-2
- [ ] `tensor.py` → PARITY-TRITON-MOE-MATMUL
- [ ] `testing.py` → SETUP-4
- [ ] `topk.py` → PARITY-TRITON-TOPK

In `external/competitors/triton/python/triton_kernels/triton_kernels/compaction_details/`:

- [ ] `_masked_compaction.py` → PARITY-TRITON-MOE-MATMUL

In `external/competitors/triton/python/triton_kernels/triton_kernels/distributed_details/`:

- [ ] `mesh.py` → PARITY-EXPERT-PARALLEL

In `external/competitors/triton/python/triton_kernels/triton_kernels/matmul_details/`:

- [ ] `_common.py` → PARITY-TRITON-MOE-MATMUL
- [ ] `_matmul.py` → PARITY-TRITON-MOE-MATMUL
- [ ] `_p_matmul.py` → PARITY-TRITON-MOE-MATMUL
- [ ] `opt_flags.py` → OPT-7

In `external/competitors/triton/python/triton_kernels/triton_kernels/matmul_details/opt_flags_details/`:

- [ ] `opt_flags_amd.py` → OPT-7
- [ ] `opt_flags_nvidia.py` → OPT-7

In `external/competitors/triton/python/triton_kernels/triton_kernels/numerics_details/`:

- [ ] `flexpoint.py` → PARITY-TRITON-MX-CONVERT
- [ ] `mxfp.py` → PARITY-TRITON-MX-CONVERT

In `external/competitors/triton/python/triton_kernels/triton_kernels/numerics_details/mxfp_details/`:

- [ ] `_downcast_to_mxfp.py` → PARITY-TRITON-MX-CONVERT
- [ ] `_upcast_from_mxfp.py` → PARITY-TRITON-MX-CONVERT

In `external/competitors/triton/python/triton_kernels/triton_kernels/swiglu_details/`:

- [ ] `_swiglu.py` → PARITY-TRITON-SWIGLU

In `external/competitors/triton/python/triton_kernels/triton_kernels/tensor_details/`:

- [ ] `bitmatrix.py` → PARITY-TRITON-MOE-MATMUL
- [ ] `dtype.py` → PARITY-TRITON-MOE-MATMUL
- [ ] `layout.py` → TRITON-LINEAR-LAYOUT
- [ ] `ragged_tensor.py` → PARITY-TRITON-MOE-MATMUL

In `external/competitors/triton/python/triton_kernels/triton_kernels/tensor_details/bitmatrix_details/`:

- [ ] `sum_bitmatrix_rows.py` → PARITY-TRITON-MOE-MATMUL

In `external/competitors/triton/python/triton_kernels/triton_kernels/tensor_details/layout_details/`:

- [ ] `base.py` → TRITON-LINEAR-LAYOUT
- [ ] `blackwell_scale.py` → PARITY-WEIGHT-FORMATS
- [ ] `blackwell_value.py` → PARITY-WEIGHT-FORMATS
- [ ] `blackwell_value_shuffled.py` → PARITY-WEIGHT-FORMATS
- [ ] `cdna4_scale.py` → TRITON-AMD-SHIM
- [ ] `cdna5_scale.py` → TRITON-AMD-SHIM
- [ ] `hopper_scale.py` → PARITY-WEIGHT-FORMATS
- [ ] `hopper_value.py` → PARITY-WEIGHT-FORMATS
- [ ] `scale.py` → PARITY-WEIGHT-FORMATS
- [ ] `strided.py` → TRITON-LINEAR-LAYOUT
- [ ] `tiled.py` → TRITON-LINEAR-LAYOUT
- [ ] `torch_utils.py` → TRITON-LINEAR-LAYOUT

In `external/competitors/triton/python/triton_kernels/triton_kernels/topk_details/`:

- [ ] `_topk_backward.py` → PARITY-TRITON-TOPK
- [ ] `_topk_forward.py` → PARITY-TRITON-TOPK

---

## 12. The board

Every piece of work is a ticket below, and every ticket appears once in the index. A ticket's id never changes; its
status moves open → claimed → done, and the lanes board records who holds it.

How a PARITY ticket proceeds:
1. **Copy.** Port the best existing implementation and match it at our shapes. The number to match is the row
   maximum of section 2.5: the fastest kernel any rival ships for that op, timed by SETUP-4.
2. **Beyond.** Push past it, by the route the ticket names: graph-level fusion the other stack cannot do, a roofline
   fraction, a proof, or all three.

The ticket's success boxes hold both bars.

Labels:
- kind: `fusion`, `primitive`, `technique`, `serving`, `infrastructure`, `beyond`;
- stream: `stream:S0` to `stream:S15` (section 1.5);
- chip: `cuda`, `cpu`, `spatial`, `d5005`, `fabric`, `tensix`, `rdu`, `datacenter`;
- area: `area:norm`, `area:attention`, `area:moe`, `area:gemm`, `area:recurrent`, `area:sampling`, `area:quant`,
  `area:comm`, `area:graph`;
- model: `model:gemma2`, `model:qwen35`, `model:ds2`, `model:oss`, `model:llama`, `model:qwen3`, `model:gemma3`.

### 12.1 Board index

- [x] **SETUP-1** · The working clones live in external/ · setup · `infrastructure` `stream:S5` · depends on: none
- [ ] **SETUP-2** · Pin vLLM and Triton under external/competitors · setup · `infrastructure` `stream:S5` · depends on: none
- [ ] **SETUP-3** · A vLLM install on this box · setup · `infrastructure` `stream:S5` · depends on: none
- [ ] **SETUP-4** · The per-kernel A/B harness and ledger · setup · `infrastructure` `cuda` `stream:S5` · depends on: SETUP-2, SETUP-3, SETUP-6
- [ ] **SETUP-5** · vLLM as the scoreboard's third side · setup · `infrastructure` `cuda` `stream:S5` · depends on: SETUP-3
- [ ] **SETUP-6** · A Triton build at the pin · setup · `infrastructure` `stream:S5` · depends on: SETUP-2
- [ ] **SETUP-7** · Every 3060 rival on one boot · setup · `infrastructure` `cuda` `stream:S5` · depends on: SETUP-3, SETUP-5
- [ ] **SETUP-8** · Baselines on rented hardware · setup · `infrastructure` `datacenter` `stream:S5` · depends on: SETUP-7, SETUP-9
- [ ] **SETUP-9** · The serving benchmark · setup · `infrastructure` `serving` `stream:S5` · depends on: SETUP-3
- [ ] **LEDGER-1** · Grow the ledger to the catalog · ledger · `infrastructure` `stream:S5` · depends on: SETUP-2
- [ ] **LEDGER-2** · Hold the book and the ledger together · ledger · `infrastructure` `stream:S5` · depends on: LEDGER-1
- [ ] **LEDGER-3** · The SOTA contract lock · ledger · `infrastructure` `stream:S5` · depends on: LEDGER-2, SETUP-2, SETUP-7
- [x] **IR-0** · Digests before code · IR · `infrastructure` `stream:S1` · depends on: none
- [ ] **IR-1** · The IR's home · IR · `infrastructure` `stream:S1` · depends on: IR-0
- [x] **IR-2** · The evaluator, equal to the VM · IR · `infrastructure` `stream:S1` · depends on: IR-1
- [x] **IR-3** · The symbol registry · IR · `infrastructure` `stream:S1` · depends on: IR-1, LEDGER-1
- [x] **OPT-1** · Facts, the rule engine and the shared rules · optimizer · `infrastructure` `stream:S1` · depends on: IR-2, IR-3
- [x] **OPT-2** · The shim trait and its five shims · optimizer · `infrastructure` `stream:S1` · depends on: OPT-1
- [ ] **OPT-3** · The fusion report · optimizer · `infrastructure` `stream:S1` · depends on: OPT-1
- [ ] **OPT-4** · Equality saturation over the IR · optimizer · `infrastructure` `stream:S1` · depends on: OPT-1, MIGRATE-1
- [x] **OPT-5** · Every rule proven · optimizer · `infrastructure` `stream:S1` · depends on: OPT-1
- [x] **OPT-6** · Numerics contracts · optimizer · `infrastructure` `area:quant` `stream:S1` · depends on: OPT-2
- [ ] **OPT-7** · A measured cost model and tuning database · optimizer · `infrastructure` `cuda` `stream:S1` · depends on: OPT-2, SETUP-4
- [ ] **OPT-8** · Per-chip ledger and provenance · optimizer · `infrastructure` `stream:S1` · depends on: MIGRATE-1, MIGRATE-2, MIGRATE-3, MIGRATE-4, LEDGER-2
- [ ] **MIGRATE-1** · The CUDA lane links the shared table · migrate · `infrastructure` `cuda` `stream:S4` · depends on: OPT-2
- [x] **MIGRATE-2** · The older CUDA forward on the IR · migrate · `infrastructure` `cuda` `stream:S4` · depends on: OPT-2
- [x] **MIGRATE-3** · The spatial pipeline on the IR · migrate · `infrastructure` `spatial` `stream:S6` · depends on: OPT-2
- [x] **MIGRATE-4** · The D5005 engine on the IR · migrate · `infrastructure` `d5005` `stream:S7` · depends on: OPT-2
- [ ] **PARITY-SOFTCAP** · Soft-capped logits and attention scores · parity, tier 1 · `fusion` `cuda` `spatial` `d5005` `area:attention` `model:gemma2` `stream:S12` · depends on: OPT-1, MIGRATE-1, SETUP-4
- [ ] **PARITY-NORM-THEN-ADD** · A norm then a residual add · parity, tier 1 · `fusion` `cuda` `area:norm` `model:gemma3` `model:gemma2` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-ATTENTION-VARIANTS-INLINE** · Caps and masks inside attention · parity, tier 1 · `primitive` `cuda` `area:attention` `model:gemma2` `stream:S14` · depends on: PARITY-SOFTCAP
- [ ] **PARITY-EMBED-NORM** · The embedding gather and the first norm · parity, tier 1 · `fusion` `cuda` `spatial` `area:norm` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-QK-NORM-ROPE-GATE-SPLIT** · Gated attention's prologue in one launch · parity, tier 1 · `fusion` `cuda` `area:attention` `model:qwen35` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-GDN-DECODE-WHOLE** · The whole gated-delta decode in one kernel · parity, tier 1 · `fusion` `cuda` `area:recurrent` `model:qwen35` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-GDN-POST-CONV-PREP** · Everything after the conv in one prefill kernel · parity, tier 1 · `fusion` `cuda` `area:recurrent` `model:qwen35` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-KV-CACHE-QUANT** · Quantized KV caches · parity, tier 1 · `fusion` `cuda` `d5005` `area:attention` `area:quant` `stream:S14` · depends on: OPT-1, OPT-6, MIGRATE-1
- [ ] **PARITY-ATTN-GQA-ROW-PACKING** · A KV group's heads on the tensor cores in decode · parity, tier 1 · `primitive` `cuda` `area:attention` `stream:S14` · depends on: MIGRATE-1, SETUP-4
- [ ] **PARITY-ATTN-LSE-MERGE** · A shared merge for partial attention · parity, tier 1 · `fusion` `cuda` `area:attention` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-MOE-DIRECT-PAIRS** · Skip the expert sort for tiny batches · parity, tier 1 · `primitive` `cuda` `area:moe` `model:ds2` `model:oss` `stream:S14` · depends on: MIGRATE-1
- [ ] **PARITY-MOE-GATHER-IN-OPERAND** · The token gather inside the expert GEMM · parity, tier 1 · `fusion` `cuda` `area:moe` `model:ds2` `model:oss` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-MOE-DEFERRED-FINALIZE** · The expert combine folded into the next add and norm · parity, tier 1 · `fusion` `cuda` `area:moe` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-SMALL-M-GEMM** · A minimum-latency GEMM for 1–16 rows · parity, tier 1 · `primitive` `cuda` `area:gemm` `stream:S3` · depends on: MIGRATE-1, SETUP-4
- [ ] **PARITY-BATCH-ROW-QUANT** · A batch's sequences quantize their rows where they store them · parity, tier 1 · `fusion` `cuda` `area:quant` `stream:S14` · depends on: none
- [ ] **PARITY-SAMPLING** · Serving-grade sampling on the device · parity, tier 1 · `fusion` `cuda` `area:sampling` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-PERSISTENT-LAYER-MEGAKERNEL** · A decoder layer as one persistent launch · parity, tier 1 · `fusion` `cuda` `stream:S14` · depends on: OPT-2, MIGRATE-1, TRITON-TILE-TIER
- [ ] **PARITY-ROPE-QK-ONE-LAUNCH** · Query and key rotated in one launch · parity, tier 1 · `fusion` `cuda` `area:attention` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-ELEMENTWISE-REGIONS** · Pointwise DAG regions fused · parity, tier 1 · `fusion` `cuda` `spatial` `stream:S14` · depends on: OPT-1, MIGRATE-1, MIGRATE-2
- [ ] **PARITY-QKV-HEAD-EPILOGUE** · One q/k/v GEMV with the per-head norm, rotation and cache append in its epilogue · parity, tier 1 · `fusion` `cuda` `area:attention` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-GEMV-NORM-EPILOGUE** · A projection with the residual add and the next norms in its last-arriving block · parity, tier 1 · `fusion` `cuda` `area:norm` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-ATTN-QUANT-IN-FOLD** · Decode attention quantizes its output in its fold · parity, tier 1 · `fusion` `cuda` `area:attention` `area:quant` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-GDN-INPUT-EPILOGUE** · Every gated-delta input projection in one launch, with conv, SiLU, L2 and gates in its epilogue · parity, tier 1 · `fusion` `cuda` `area:recurrent` `model:qwen35` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-GDN-HALF-STATE** · Half-precision gated-delta state · parity, tier 1 · `fusion` `cuda` `area:recurrent` `area:quant` `model:qwen35` `stream:S14` · depends on: OPT-6, PARITY-GDN-DECODE-WHOLE
- [ ] **PARITY-MLA-PROLOGUE** · The latent layer's q and latent in one launch with norm, rotation, append and absorption · parity, tier 1 · `fusion` `cuda` `area:attention` `model:ds2` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-MLA-UNABSORB-IN-FOLD** · Latent attention un-absorbs its values and quantizes in its fold · parity, tier 1 · `fusion` `cuda` `area:attention` `model:ds2` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-LAST-LAYER-ROWS** · A prefill chunk's last layer computes only its output rows beyond the caches · parity, tier 1 · `fusion` `cuda` `spatial` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-INT8-GEMM-PEAK** · The dense int8 GEMM past 55 TOPS, and the per-channel W8A8 knob point · parity, tier 1 · `primitive` `cuda` `area:gemm` `stream:S3` · depends on: SETUP-4, TRITON-GENERIC-SWIZZLE
- [ ] **PARITY-PREFILL-ATTENTION-PEAK** · Prefill attention at 15 TFLOPS f16 · parity, tier 1 · `primitive` `cuda` `area:attention` `stream:S3` · depends on: SETUP-4, PARITY-ATTENTION-VARIANTS-INLINE
- [ ] **PARITY-GDN-CHUNK-TENSOR-CORES** · The chunked delta rule on the tensor cores · parity, tier 1 · `primitive` `cuda` `area:recurrent` `model:qwen35` `stream:S3` · depends on: SETUP-4, TECH-SPLIT-PRECISION-MMA
- [ ] **PARITY-PREFILL-NORM-EPILOGUES** · Wide-chunk norms, residuals and quantizes in GEMM epilogues · parity, tier 1 · `fusion` `cuda` `area:norm` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-WEIGHT-FORMATS** · Every weight format the rivals run, read in place or repacked at load · parity, tier 1 · `primitive` `cuda` `area:gemm` `area:quant` `stream:S3` · depends on: MIGRATE-1, SETUP-4, TECH-LOP3-PRMT-DEQUANT
- [ ] **PARITY-NORM-SCALE** · A norm then a scalar scale · parity, tier 2 · `fusion` `cuda` `area:norm` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-GEMM-SCALE** · Scales and bias in a GEMM's epilogue · parity, tier 2 · `fusion` `cuda` `area:gemm` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-RELU-SQR** · ReLU² on every chip · parity, tier 2 · `fusion` `cuda` `area:norm` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-DUAL-NORM** · Two independent norms in one kernel · parity, tier 2 · `fusion` `cuda` `area:norm` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-RECOMPUTE-FANOUT** · Recompute a cheap producer per consumer · parity, tier 2 · `fusion` `cuda` `spatial` `stream:S14` · depends on: OPT-1, OPT-7
- [ ] **PARITY-HORIZONTAL-NORM-ROPE-INSERT** · One kernel over a multi-branch row · parity, tier 2 · `fusion` `cuda` `area:attention` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-CROSS-LAYER-LAUNCH** · Every layer's cache insert in one launch · parity, tier 2 · `fusion` `cuda` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-HOST-RESIDENT-KV** · KV rows read from pinned host memory · parity, tier 2 · `primitive` `cuda` `model:qwen35` `stream:S14` · depends on: MIGRATE-1
- [ ] **PARITY-HOST-WEIGHT-STREAMING** · Matrices past the card streamed to it once a chunk · parity, tier 2 · `primitive` `cuda` `model:qwen35` `stream:S14` · depends on: MIGRATE-1
- [ ] **PARITY-HOST-LAYERS-ON-CPU** · The layers past the card on the CPU tier · parity, tier 2 · `primitive` `cuda` `cpu` `model:qwen35` `stream:S14` · depends on: MIGRATE-1, PARITY-CPU-SHIM
- [ ] **PARITY-ROUTER-SHARED-GATE-MERGE** · Router and shared-expert gate as one GEMM · parity, tier 2 · `fusion` `cuda` `area:moe` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-GROUPED-TOPK** · Group-limited routing in one kernel · parity, tier 2 · `primitive` `cuda` `area:moe` `stream:S14` · depends on: MIGRATE-1
- [ ] **PARITY-ROUTING-VARIANTS** · Sigmoid with bias and groups, softplus-sqrt, hash routing · parity, tier 2 · `primitive` `cuda` `area:moe` `stream:S14` · depends on: MIGRATE-1
- [ ] **PARITY-MASKED-EXPERT-ACTIVATION** · Batched experts that skip padded rows · parity, tier 2 · `fusion` `cuda` `area:moe` `stream:S14` · depends on: SERVE-CONTINUOUS-BATCHING
- [ ] **PARITY-ACTIVATION-KINDS** · Every activation vLLM ships · parity, tier 2 · `primitive` `cuda` `spatial` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-ROPE-VARIANTS** · Every rotary scaling vLLM ships · parity, tier 2 · `primitive` `cuda` `spatial` `d5005` `stream:S14` · depends on: IR-2
- [ ] **PARITY-NORM-WEIGHT-OFFSET** · Gemma's (1 + w) inside the norm · parity, tier 2 · `fusion` `cuda` `area:norm` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-AFFINE-INPUT-FOLD** · Input normalization folded into one affine · parity, tier 2 · `fusion` `cuda` `spatial` `stream:S14` · depends on: OPT-4
- [ ] **PARITY-ATTN-RESIDUAL-NORM** · Attention over block residuals with the norm · parity, tier 2 · `fusion` `cuda` `stream:S14` · depends on: OPT-1, MIGRATE-1
- [ ] **PARITY-SELECTIVE-SCAN** · Mamba1 selective scan · parity, tier 2 · `primitive` `cuda` `spatial` `area:recurrent` `stream:S12` · depends on: IR-2
- [ ] **PARITY-SSD-CHUNK-SCAN** · Mamba2 chunked scan · parity, tier 2 · `primitive` `cuda` `spatial` `area:recurrent` `stream:S12` · depends on: IR-2
- [ ] **PARITY-KDA** · Kimi delta attention · parity, tier 2 · `primitive` `cuda` `area:recurrent` `stream:S12` · depends on: PARITY-GDN-DECODE-WHOLE
- [ ] **PARITY-LIGHTNING-ATTENTION** · Block linear attention with decay · parity, tier 2 · `primitive` `cuda` `area:recurrent` `stream:S12` · depends on: IR-2
- [ ] **PARITY-SHORT-CONV** · Gated short convolution · parity, tier 2 · `primitive` `cuda` `area:recurrent` `stream:S12` · depends on: IR-2
- [ ] **PARITY-MIXED-BATCH-ATTENTION** · Prefill and decode rows in one attention launch · parity, tier 2 · `primitive` `cuda` `serving` `stream:S14` · depends on: SERVE-PAGED-KV
- [ ] **PARITY-CPU-SHIM** · A CPU shim, its link and its kernels · parity, tier 2 · `infrastructure` `cpu` `stream:S11` · depends on: OPT-2, MIGRATE-1, SETUP-7
- [ ] **PARITY-MULTIMODAL-TOWERS** · Vision and audio encoder towers · parity, tier 2 · `fusion` `cuda` `spatial` `stream:S12` · depends on: PARITY-AFFINE-INPUT-FOLD, PARITY-ROPE-VARIANTS
- [ ] **PARITY-MODEL-SPECIFIC-LAYERS** · Layers one model family uses · parity, tier 2 · `primitive` `cuda` `stream:S12` · depends on: IR-2, OPT-1
- [ ] **PARITY-QUANT-SCHEMES** · fp8, NVFP4, MXFP4 and MXFP8 for every quantize fold · parity, tier 3 · `fusion` `datacenter` `area:quant` `stream:S14` · depends on: OPT-6, MIGRATE-1
- [ ] **PARITY-QUANT-GEMMS-DATACENTER** · fp8, fp4 and W4A8 GEMMs, dense and grouped · parity, tier 3 · `primitive` `datacenter` `area:gemm` `stream:S14` · depends on: PARITY-QUANT-SCHEMES, TRITON-MMA-SELECTION
- [ ] **PARITY-MLA-DATACENTER** · Latent attention on sm90 and sm100 · parity, tier 3 · `primitive` `datacenter` `area:attention` `model:ds2` `stream:S14` · depends on: TRITON-WARP-SPECIALIZATION
- [ ] **PARITY-SPARSE-ATTENTION** · Indexers, block top-k and sparse attention · parity, tier 3 · `fusion` `datacenter` `area:attention` `stream:S14` · depends on: OPT-2
- [ ] **PARITY-HYPER-CONNECTIONS** · Hyper-connection mixing kernels · parity, tier 3 · `fusion` `datacenter` `stream:S14` · depends on: OPT-2
- [ ] **PARITY-XPU-SHIM** · An Intel GPU shim · parity, tier 3 · `infrastructure` `stream:S11` · depends on: OPT-2, TRITON-TILE-TIER, SETUP-8
- [ ] **PARITY-TENSOR-PARALLEL-IR** · Tensor parallelism in the IR · parity, tier 4 · `infrastructure` `area:comm` `stream:S14` · depends on: OPT-2
- [ ] **PARITY-ALLREDUCE-NORM** · Collectives fused with norms · parity, tier 4 · `fusion` `area:comm` `stream:S14` · depends on: PARITY-TENSOR-PARALLEL-IR
- [ ] **PARITY-EXPERT-PARALLEL** · Expert-parallel dispatch and combine · parity, tier 4 · `fusion` `area:comm` `area:moe` `stream:S14` · depends on: PARITY-TENSOR-PARALLEL-IR
- [ ] **SERVE-PAGED-KV** · Paged KV caches · serving · `serving` `cuda` `stream:S13` · depends on: MIGRATE-1
- [ ] **SERVE-CONTINUOUS-BATCHING** · Requests join and leave every step · serving · `serving` `cuda` `stream:S13` · depends on: SERVE-PAGED-KV
- [ ] **SERVE-CHUNKED-PREFILL** · Long prompts in chunks beside decode · serving · `serving` `cuda` `stream:S13` · depends on: SERVE-CONTINUOUS-BATCHING, PARITY-MIXED-BATCH-ATTENTION
- [ ] **SERVE-PREFIX-CACHE** · Shared prefixes reuse their KV · serving · `serving` `cuda` `stream:S13` · depends on: SERVE-PAGED-KV
- [ ] **SERVE-SPEC-DECODE** · Speculative decoding, MTP first · serving · `serving` `cuda` `model:qwen35` `stream:S13` · depends on: SERVE-CONTINUOUS-BATCHING, PARITY-SMALL-M-GEMM
- [ ] **SERVE-PER-BATCH-GRAPHS** · One captured graph per batch size · serving · `serving` `cuda` `stream:S13` · depends on: SERVE-CONTINUOUS-BATCHING
- [ ] **SERVE-EXPERT-OFFLOAD** · Experts computed on the CPU · serving · `serving` `cuda` `stream:S13` · depends on: MIGRATE-1
- [ ] **SERVE-KV-OFFLOAD** · KV blocks swapped to host memory · serving · `serving` `cuda` `stream:S13` · depends on: SERVE-PAGED-KV
- [ ] **SERVE-STRUCTURED-OUTPUT** · Grammar-constrained decoding · serving · `serving` `cuda` `stream:S13` · depends on: PARITY-SAMPLING
- [ ] **SERVE-LORA** · Adapters, including inside the experts · serving · `serving` `cuda` `stream:S13` · depends on: SERVE-CONTINUOUS-BATCHING
- [ ] **SERVE-DUAL-BATCH-OVERLAP** · Compute and communication overlapped across microbatches · serving · `serving` `area:comm` `stream:S13` · depends on: PARITY-EXPERT-PARALLEL
- [ ] **SERVE-POOLING** · Embedding, reranking and pooling models · serving · `serving` `cuda` `stream:S13` · depends on: SERVE-CONTINUOUS-BATCHING
- [ ] **GRAPH-CANONICALIZE** · Canonicalize views and reshapes · graph · `infrastructure` `area:graph` `stream:S1` · depends on: OPT-1
- [ ] **GRAPH-IN-PLACE** · In-place destinations as a recorded property · graph · `infrastructure` `area:graph` `stream:S1` · depends on: OPT-1
- [ ] **GRAPH-PER-SHAPE-FORMS** · Per-shape forms, recorded · graph · `infrastructure` `area:graph` `stream:S5` · depends on: LEDGER-1
- [ ] **GRAPH-CUDA-GRAPH-MODES** · Full, piecewise and breakable capture · graph · `infrastructure` `cuda` `area:graph` `stream:S4` · depends on: MIGRATE-1
- [ ] **GRAPH-GRAPH-MEMORY-POOL** · One pool for every captured graph · graph · `infrastructure` `cuda` `area:graph` `stream:S4` · depends on: GRAPH-CUDA-GRAPH-MODES
- [ ] **GRAPH-CROSS-KERNEL-OVERLAP** · Kernel overlap decided by the compiler · graph · `infrastructure` `cuda` `area:graph` `stream:S4` · depends on: MIGRATE-1, TECH-PDL
- [ ] **TECH-ALIGNMENT-AWARE-VECTORIZATION** · Scalar prefix, vector body, tail · technique · `technique` `cuda` `stream:S3` · depends on: SETUP-4
- [ ] **TECH-PACKED-HALF2-MATH** · Packed half2 and bf16x2 arithmetic · technique · `technique` `cuda` `stream:S3` · depends on: SETUP-4
- [ ] **TECH-PER-ARCH-LAUNCH-BOUNDS** · Launch bounds from the arch's threads per SM · technique · `technique` `cuda` `stream:S3` · depends on: none
- [ ] **TECH-CP-ASYNC-MULTISTAGE** · Multi-stage async copies · technique · `technique` `cuda` `stream:S3` · depends on: SETUP-4
- [ ] **TECH-MBARRIER-PIPELINE** · mbarrier-driven pipelines · technique · `technique` `cuda` `stream:S3` · depends on: SETUP-4
- [ ] **TECH-LOP3-PRMT-DEQUANT** · Magic-number dequantization · technique · `technique` `cuda` `area:quant` `stream:S3` · depends on: SETUP-4
- [ ] **TECH-STREAM-K** · Striped work with a slice-locked reduce · technique · `technique` `cuda` `area:gemm` `stream:S3` · depends on: SETUP-4
- [ ] **TECH-BROADCAST-SCALE-EPILOGUE** · Scales read from device pointers in the epilogue · technique · `technique` `cuda` `area:gemm` `stream:S3` · depends on: PARITY-GEMM-SCALE
- [ ] **TECH-UVA-HOST-VIEW** · Host memory addressed from the GPU · technique · `technique` `cuda` `stream:S3` · depends on: none
- [ ] **TECH-RADIX-TOPK-KEYS** · Order-preserving keys for radix top-k · technique · `technique` `cuda` `area:sampling` `stream:S3` · depends on: PARITY-SAMPLING
- [ ] **TECH-COMPENSATED-RESIDUAL** · Compensated residual sums · technique · `technique` `cuda` `area:norm` `stream:S3` · depends on: OPT-6
- [ ] **TECH-SPLIT-PRECISION-MMA** · Exact sums from split-precision MMAs · technique · `technique` `cuda` `stream:S3` · depends on: OPT-6
- [ ] **TECH-STOCHASTIC-ROUNDING-STATE** · Stochastic rounding for half-precision state · technique · `technique` `cuda` `area:recurrent` `stream:S3` · depends on: OPT-6
- [ ] **TECH-EXP2-SCALE-FOLD** · exp2 with log2(e) folded into the scale · technique · `technique` `cuda` `area:attention` `stream:S3` · depends on: none
- [ ] **TECH-L2-WEIGHT-PREFETCH** · The next weights pulled into L2 · technique · `technique` `cuda` `stream:S3` · depends on: SETUP-4
- [ ] **TECH-SCHED-FOR-REGISTERS** · Register-pressure scheduling flags · technique · `technique` `cuda` `stream:S3` · depends on: none
- [ ] **TECH-PACKED-FP-CONVERSIONS** · Packed fp8 and fp4 conversions · technique · `technique` `cuda` `area:quant` `stream:S3` · depends on: none
- [ ] **TECH-INLINE-PTX-PACKING** · Packed inline PTX · technique · `technique` `cuda` `stream:S3` · depends on: none
- [ ] **TECH-OCCUPANCY-FROM-REGS** · The grid from registers and shared memory · technique · `technique` `cuda` `stream:S3` · depends on: none
- [ ] **TECH-256BIT-LOADS** · 256-bit loads and stores · technique · `technique` `datacenter` `stream:S10` · depends on: none
- [ ] **TECH-PDL** · Programmatic dependent launch · technique · `technique` `datacenter` `stream:S10` · depends on: none
- [ ] **TECH-WGMMA-RS-PREPACKED** · Register-sourced wgmma with prepacked weights · technique · `technique` `datacenter` `area:gemm` `stream:S10` · depends on: TRITON-MMA-SELECTION
- [ ] **TECH-TMA-WS-PINGPONG** · TMA with warp-specialized ping-pong · technique · `technique` `datacenter` `stream:S10` · depends on: TRITON-TMA, TRITON-WARP-SPECIALIZATION
- [ ] **TECH-2SM-UMMA-TMEM** · Two-SM MMAs and tensor memory · technique · `technique` `datacenter` `stream:S10` · depends on: TRITON-MMA-SELECTION
- [ ] **TECH-CLUSTER-DSMEM-REDUCE** · Reductions across a cluster's shared memory · technique · `technique` `datacenter` `stream:S10` · depends on: none
- [ ] **TECH-CLUSTER-LAUNCH-CONTROL** · Persistent work stealing · technique · `technique` `datacenter` `stream:S10` · depends on: TRITON-FUSE-NESTED-LOOPS
- [ ] **TECH-BATCH-INVARIANCE** · Bit-identical results across batch shapes, recorded · technique · `technique` `cuda` `stream:S3` · depends on: LEDGER-1
- [ ] **TRITON-TILE-TIER** · A generated-kernel tier under the CUDA link · triton · `infrastructure` `cuda` `stream:S10` · depends on: MIGRATE-1
- [ ] **TRITON-LINEAR-LAYOUT** · Layouts as linear maps over F2 · triton · `infrastructure` `stream:S10` · depends on: none
- [ ] **TRITON-GENERIC-SWIZZLE** · Optimal shared-memory swizzles · triton · `technique` `cuda` `stream:S10` · depends on: TRITON-LINEAR-LAYOUT
- [ ] **TRITON-SHUFFLE-CONVERT** · Layout conversion through warp shuffles · triton · `technique` `cuda` `stream:S10` · depends on: TRITON-LINEAR-LAYOUT
- [ ] **TRITON-AXIS-FACTS** · Proven contiguity, divisibility and constancy · triton · `infrastructure` `stream:S10` · depends on: OPT-1
- [ ] **TRITON-COMBINE-REWRITES** · Contraction recognition and broadcast sinking · triton · `infrastructure` `stream:S10` · depends on: OPT-4
- [ ] **TRITON-COALESCE** · Coalesced accesses from proven facts · triton · `technique` `cuda` `stream:S10` · depends on: TRITON-AXIS-FACTS, TRITON-TILE-TIER
- [ ] **TRITON-GLOBAL-LAYOUT-ASSIGNMENT** · Minimum-cost layout assignment · triton · `infrastructure` `cuda` `stream:S10` · depends on: TRITON-LINEAR-LAYOUT, TRITON-TILE-TIER
- [ ] **TRITON-THREAD-LOCALITY** · Thread-local partials across loops · triton · `technique` `cuda` `stream:S10` · depends on: TRITON-TILE-TIER
- [ ] **TRITON-TRANSPOSE-AS-ADDRESSING** · Transposes folded into addressing · triton · `technique` `cuda` `stream:S10` · depends on: TRITON-TILE-TIER
- [ ] **TRITON-SMEM-STAGING-NODES** · Shared-memory staging as graph nodes · triton · `technique` `cuda` `stream:S10` · depends on: TRITON-TILE-TIER
- [ ] **TRITON-REGISTER-PRESSURE-ORDER** · Instruction order for register pressure · triton · `technique` `cuda` `stream:S10` · depends on: TRITON-TILE-TIER
- [ ] **TRITON-ACCUMULATOR-INIT** · The use-D flag instead of a zero fill · triton · `technique` `cuda` `stream:S10` · depends on: TRITON-TILE-TIER
- [ ] **TRITON-FUSE-NESTED-LOOPS** · Persistent loops from nests · triton · `technique` `cuda` `stream:S10` · depends on: TRITON-TILE-TIER
- [ ] **TRITON-OPERAND-PREFETCH** · Operand register double-buffering · triton · `technique` `cuda` `stream:S10` · depends on: TRITON-TILE-TIER
- [ ] **TRITON-MODULO-SCHEDULER** · A real modulo scheduler · triton · `infrastructure` `cuda` `stream:S10` · depends on: TRITON-TILE-TIER
- [ ] **TRITON-ASYNC-MMA-WAITS** · Minimal waits for async MMAs · triton · `technique` `datacenter` `stream:S10` · depends on: TRITON-MODULO-SCHEDULER
- [ ] **TRITON-TMA** · TMA descriptors, ragged, gather and im2col · triton · `technique` `datacenter` `stream:S10` · depends on: TRITON-TILE-TIER
- [ ] **TRITON-WARP-SPECIALIZATION** · Warp specialization as a graph cut · triton · `infrastructure` `cuda` `stream:S10` · depends on: TRITON-MODULO-SCHEDULER
- [ ] **TRITON-MMA-SELECTION** · Tensor-core instruction selection · triton · `infrastructure` `cuda` `stream:S10` · depends on: TRITON-TILE-TIER
- [ ] **TRITON-SMEM-COLORING-ALLOCATOR** · Shared memory by interference colouring · triton · `technique` `cuda` `stream:S10` · depends on: TRITON-TILE-TIER
- [ ] **TRITON-REGION-BARRIERS** · Barriers and fences from region analysis · triton · `technique` `cuda` `stream:S10` · depends on: TRITON-SMEM-COLORING-ALLOCATOR
- [ ] **TRITON-REDUCE-SCAN-CODEGEN** · Reductions, scans, sorts and top-k · triton · `technique` `cuda` `stream:S10` · depends on: TRITON-TILE-TIER
- [ ] **TRITON-AOT-DISPATCH** · Dispatch on run-time lengths · triton · `technique` `cuda` `stream:S10` · depends on: MIGRATE-1
- [ ] **TRITON-CACHE-POLICY-EDGES** · Cache policies as edge annotations · triton · `technique` `cuda` `stream:S10` · depends on: OPT-2
- [ ] **TRITON-AMD-SHIM** · An AMD shim · triton · `infrastructure` `stream:S11` · depends on: OPT-2
- [ ] **PARITY-TRITON-SOFTMAX** · Match Triton's fused softmax, then beat it · parity-triton · `fusion` `cuda` `stream:S10` · depends on: SETUP-6, OPT-1
- [ ] **PARITY-TRITON-LAYERNORM** · Layer norm, forward and backward · parity-triton · `fusion` `cuda` `stream:S10` · depends on: SETUP-6, OPT-1
- [ ] **PARITY-TRITON-ATTENTION** · Triton's fused attention · parity-triton · `primitive` `cuda` `area:attention` `stream:S10` · depends on: SETUP-6, PARITY-ATTN-GQA-ROW-PACKING
- [ ] **PARITY-TRITON-GROUPED-GEMM** · Grouped GEMM · parity-triton · `primitive` `cuda` `area:gemm` `stream:S10` · depends on: SETUP-6, TRITON-TILE-TIER
- [ ] **PARITY-TRITON-PERSISTENT-MATMUL** · Persistent matmul with epilogue subtiling · parity-triton · `primitive` `cuda` `area:gemm` `stream:S10` · depends on: SETUP-6, TRITON-FUSE-NESTED-LOOPS
- [ ] **PARITY-TRITON-DROPOUT** · Seeded dropout with no stored mask · parity-triton · `fusion` `cuda` `stream:S10` · depends on: SETUP-6, OPT-1
- [ ] **PARITY-TRITON-MOE-MATMUL** · The MoE matmul with fused gather, scatter and epilogues · parity-triton · `fusion` `cuda` `area:moe` `stream:S10` · depends on: SETUP-6, PARITY-MOE-GATHER-IN-OPERAND
- [ ] **PARITY-TRITON-SWIGLU** · SwiGLU with clamps and flexpoint scales · parity-triton · `fusion` `cuda` `stream:S10` · depends on: SETUP-6, OPT-1
- [ ] **PARITY-TRITON-TOPK** · Top-k by packed keys · parity-triton · `primitive` `cuda` `area:sampling` `stream:S10` · depends on: SETUP-6, TECH-RADIX-TOPK-KEYS
- [ ] **PARITY-TRITON-MX-CONVERT** · MX up- and down-conversion · parity-triton · `primitive` `cuda` `area:quant` `stream:S10` · depends on: SETUP-6
- [ ] **BEYOND-CERTIFIED-OPTIMAL-FUSION** · Fusion plans that are provably optimal under the shim's cost · beyond · `beyond` `infrastructure` `stream:S8` · depends on: OPT-2, OPT-4, OPT-7
- [ ] **BEYOND-EXACT-MODULO-SCHEDULER** · One exact scheduler for every chip's pipelines · beyond · `beyond` `infrastructure` `cuda` `spatial` `d5005` `fabric` `stream:S8` · depends on: OPT-2, TRITON-TILE-TIER
- [ ] **BEYOND-CERTIFIED-ALLOCATION** · Every on-chip memory allocated by proof · beyond · `beyond` `infrastructure` `cuda` `spatial` `d5005` `stream:S8` · depends on: OPT-2
- [ ] **BEYOND-PROVEN-PROTOCOLS** · Every multi-block protocol proven deadlock-free before it is emitted · beyond · `beyond` `infrastructure` `cuda` `spatial` `stream:S8` · depends on: OPT-2
- [ ] **BEYOND-BIT-EXACT-EQUIVALENCE** · Integer kernels proven for every input · beyond · `beyond` `technique` `cuda` `fabric` `stream:S8` · depends on: none
- [ ] **BEYOND-SUPEROPTIMIZER** · The shortest proven instruction sequence for every hot fragment · beyond · `beyond` `technique` `cuda` `cpu` `stream:S9` · depends on: BEYOND-BIT-EXACT-EQUIVALENCE, SETUP-4
- [ ] **BEYOND-OWN-SASS-SCHEDULER** · Our own instruction schedule and control codes on sm_86 · beyond · `beyond` `technique` `cuda` `stream:S9` · depends on: none
- [ ] **BEYOND-PROVEN-NUMERICS** · Cheaper numerics wherever their error is proven · beyond · `beyond` `infrastructure` `area:quant` `stream:S8` · depends on: OPT-6
- [ ] **BEYOND-LOOP-LEVEL-FUSION** · Any producer and consumer loop nests fuse when reordering is proven invisible · beyond · `beyond` `fusion` `stream:S8` · depends on: OPT-1, ONE-PROGRAM-GENERAL-LOOPS
- [ ] **BEYOND-EVALUATOR-SPECIALIZATION** · Each chip's generic path is the evaluator, specialized on the graph · beyond · `beyond` `infrastructure` `cpu` `stream:S8` · depends on: IR-2
- [ ] **BEYOND-EXACT-WEIGHT-FOLDS** · Algebraic weight rewrites proven over the rationals · beyond · `beyond` `fusion` `stream:S8` · depends on: OPT-4, OPT-6
- [ ] **BEYOND-SPATIAL-ON-GPU** · The GPU lowered as a spatial fabric · beyond · `beyond` `cuda` `spatial` `stream:S6` · depends on: MIGRATE-3, BEYOND-PROVEN-PROTOCOLS, BEYOND-CERTIFIED-ALLOCATION
- [ ] **BEYOND-RUNTIME-PATCHING** · Kernels specialized at serving time in microseconds · beyond · `beyond` `serving` `cuda` `stream:S9` · depends on: SERVE-CONTINUOUS-BATCHING, BEYOND-OWN-SASS-SCHEDULER
- [ ] **BEYOND-SYMMETRY-PRUNED-SEARCH** · Search spaces cut by proven symmetry · beyond · `beyond` `infrastructure` `stream:S8` · depends on: OPT-7
- [ ] **BEYOND-TRAINING** · The backward program through the same optimizer and chips · beyond · `beyond` `fusion` `cuda` `spatial` `stream:S12` · depends on: OPT-1, MIGRATE-1
- [ ] **ONE-PROGRAM-VM-REFERENCE** · The VM is every backend's reference, directly · one program · `infrastructure` `stream:S2` · depends on: none
- [ ] **ONE-PROGRAM-EVERY-BACKEND** · One command lowers a checkpoint to every backend · one program · `infrastructure` `stream:S2` · depends on: ONE-PROGRAM-VM-REFERENCE
- [ ] **ONE-PROGRAM-RETIRE-SECOND-WRITERS** · No product path runs a program `model::author` did not write · one program · `infrastructure` `stream:S2` · depends on: ONE-PROGRAM-EVERY-BACKEND, MIGRATE-2, MIGRATE-3
- [ ] **ONE-PROGRAM-GENERAL-LOOPS** · A program no recognizer lifts still lowers everywhere · one program · `infrastructure` `stream:S2` · depends on: IR-2, TRITON-TILE-TIER
- [x] **ONE-PROGRAM-FUSION-MAP-LOCK** · Section 3.4 is the shims' answers · one program · `infrastructure` `stream:S2` · depends on: OPT-2, LEDGER-2
- [ ] **D5005-FUSED-COMMANDS** · The engine fuses what every other chip fuses · chip path · `fusion` `d5005` `stream:S7` · depends on: MIGRATE-4
- [ ] **SPATIAL-NONUNIFORM-TILING** · Every core of the mesh at work · chip path · `technique` `spatial` `tensix` `stream:S6` · depends on: none
- [ ] **FABRIC-REAL-CYCLE** · The fabric's cycle count is measured on the board · demo · `infrastructure` `fabric` `stream:S15` · depends on: none
- [ ] **FABRIC-SETTLE-PROBE** · One settle probe for every combinational board top · demo · `infrastructure` `fabric` `stream:S15` · depends on: FABRIC-REAL-CYCLE
- [ ] **FABRIC-BUILD-RECIPES** · Every demo image names its build and its resources · demo · `infrastructure` `fabric` `stream:S15` · depends on: FABRIC-REAL-CYCLE
- [ ] **FABRIC-AUTHOR-MODEL** · The fabric generates tokens from a program `model::author` writes · demo · `infrastructure` `fabric` `spatial` `stream:S15` · depends on: AUTHOR-INTEGER-POINT, FABRIC-REAL-CYCLE
- [ ] **AUTHOR-INTEGER-POINT** · `model::author` writes an integer knob point · demo · `infrastructure` `area:quant` `stream:S12` · depends on: none
- [ ] **FABRIC-TERMINAL-LAND** · The Fabric Terminal lives in the tree · demo · `infrastructure` `fabric` `stream:S15` · depends on: none
- [ ] **DEMO-QUALITY-SIDES** · Every scoreboard side records how closely it follows the f32 model · demo · `infrastructure` `cuda` `stream:S5` · depends on: SETUP-5
- [ ] **DEMO-CELLS** · The sheet prints only won cells · demo · `infrastructure` `stream:S5` · depends on: LEDGER-3, DEMO-QUALITY-SIDES
- [ ] **DEMO-FUSION-MAP** · Fused symbols × chips, read from what each chip's lowering fired · demo · `infrastructure` `stream:S15` · depends on: none
- [ ] **DEMO-LAYER-TOUR** · Each English statement, and what it became on each chip · demo · `infrastructure` `stream:S15` · depends on: none
- [ ] **DEMO-BRINGUP-RUN** · `largo bringup --run`: every chip runs the cut and reports tokens, speed and agreement · demo · `infrastructure` `stream:S2` · depends on: ONE-PROGRAM-VM-REFERENCE
- [ ] **D5005-CARD-RECEIPTS** · The card's runs are receipts in the tree · demo · `infrastructure` `d5005` `stream:S7` · depends on: none
- [ ] **PROOF-RECEIPTS-FRESH** · Demo 3's figures are current and read from their ledgers · demo · `infrastructure` `stream:S15` · depends on: none
- [ ] **PROOF-LIVE** · The proof demo as one rehearsed runbook · demo · `infrastructure` `stream:S15` · depends on: PROOF-RECEIPTS-FRESH
- [x] **MEASURED-FIELDS** · No board streams a constant where a measurement belongs · demo · `infrastructure` `stream:S15` · depends on: none
- [x] **FROZEN-REDS** · A handed-over RED test stays the test it was · demo · `infrastructure` `stream:S0` · depends on: none
- [ ] **NUMERICS-EXACT** · A knob point at which every chip computes the same bits · demo · `infrastructure` `area:quant` `stream:S1` · depends on: OPT-6, IR-2

### 12.2 Dispatch order

- **Critical path for the structure:** IR-0 → IR-1 → IR-2 → IR-3 → OPT-1 → OPT-2 → MIGRATE-1. After MIGRATE-1 the tier-1
  PARITY tickets can link on CUDA.
- **The kernel map's order, by decode time recovered on the scoreboard models:**
  1. PARITY-GEMV-NORM-EPILOGUE and PARITY-QKV-HEAD-EPILOGUE: every dense layer, 14 → 5 launches on Gemma.
  2. PARITY-GDN-INPUT-EPILOGUE and PARITY-GDN-DECODE-WHOLE: Qwen3.5 and Qwen3.8.
  3. PARITY-MLA-PROLOGUE, PARITY-MLA-UNABSORB-IN-FOLD and PARITY-MOE-DEFERRED-FINALIZE: DeepSeek.
  4. PARITY-ATTN-GQA-ROW-PACKING with PARITY-ATTN-QUANT-IN-FOLD: every grouped model.
  5. PARITY-SOFTCAP: Gemma-2.

  For prefill: PARITY-INT8-GEMM-PEAK, then PARITY-GDN-CHUNK-TENSOR-CORES, PARITY-PREFILL-ATTENTION-PEAK,
  PARITY-PREFILL-NORM-EPILOGUES and PARITY-LAST-LAYER-ROWS.
- **Kernel quality starts now.** A kernel's speed (its special sauce) needs no IR: PARITY-INT8-GEMM-PEAK,
  PARITY-PREFILL-ATTENTION-PEAK, PARITY-GDN-CHUNK-TENSOR-CORES, PARITY-SMALL-M-GEMM and the TECH tickets can start
  beside the critical path, with the lane's owner. Only new fusion rules wait for OPT-1.
- **No IR dependency.** These start at once, in parallel with the critical path:
  - SETUP-2 to SETUP-7, SETUP-9 and LEDGER-1, then LEDGER-3 after LEDGER-2;
  - TRITON-LINEAR-LAYOUT;
  - the TECH tickets marked `none`.
- **The shared side of a fusion starts after OPT-1.** A tier-1 PARITY ticket's rule, definition and proof can be written
  before MIGRATE-1 lands. Its CUDA link cannot.
- **The one program and the exact methods start at once.** ONE-PROGRAM-VM-REFERENCE needs nothing; the S8 tickets
  that need no IR (BEYOND-BIT-EXACT-EQUIVALENCE, BEYOND-CERTIFIED-ALLOCATION's core, BEYOND-EXACT-MODULO-SCHEDULER's
  core) start beside the critical path, and each lands its chip use when OPT-2 and the links exist.
- **The SOTA cells decide the order inside each tier.** A ticket that closes more of section 2.4's open cells goes
  first. PARITY-WEIGHT-FORMATS opens every BF16 and 4-bit cell on our side, so it starts with the kernel-quality
  tickets.
- **The demo track (section 12.20) runs beside the critical path.** In order: FABRIC-REAL-CYCLE, then
  DEMO-QUALITY-SIDES with SETUP-5 and SETUP-7, then DEMO-FUSION-MAP and DEMO-LAYER-TOUR, then DEMO-BRINGUP-RUN, then
  FABRIC-BUILD-RECIPES and FABRIC-AUTHOR-MODEL; the kernel-peak and batch closers (PARITY-INT8-GEMM-PEAK,
  PARITY-SMALL-M-GEMM) close the cells the demo's sheet cannot yet print. NUMERICS-EXACT is deferred.
- **No new fusion rule enters `codegen_cuda/lane/plan.rs`.** Every new fusion goes in the shared table
  (INV-NO-BACKEND-FUSES). Improving a CUDA kernel, the special sauce, is allowed at any time.

### 12.3 The ticket template

Every ticket below follows this form. Copy it for new work.

```
#### <ID> · <title>
| Field | Value |
|---|---|
| Phase | ... |
| Labels | ... |
| Depends on | ... |
| Touches | files and modules |
| Owner notes | peer-owned files, freeze windows, GPU windows |
| Status | open |

Goal. What is true when this is done.
Copy (PARITY tickets). The best implementation today and where it lives. The bar is the row maximum (section 2.5).
Beyond (PARITY tickets). The bar past it, and why we can reach it.
Read first. Files, tests, receipts.
RED tests. Each fails on today's tree for the reason given.
  - [ ] suite::test_name: asserts ...; fails today because ...
Edge cases. Each covered by a RED test.
  - [ ] ...
Implementation.
  - [ ] ...
Success. Tick only with a receipt.
  - [ ] ... (receipt: ...)
Coverage. Every section 11 row mapped to this ticket is ticked with it.
Mutants to kill.
  - [ ] ...
Verify.
  cargo nextest run -p logicaffeine-tests --test <suite>
Done when. One sentence.
```

### 12.4 Setup

#### SETUP-1 · The working clones live in external/

| Field | Value |
|---|---|
| Phase | setup |
| Labels | `infrastructure` |
| Depends on | none |
| Touches | `external/vllm`, `external/triton` (local-only) |
| Status | done |

**Goal.** Tristen's working clones of vLLM (ceb01b6ddf) and Triton (1ed999d6b2) live in `external/`, beside the other
third-party checkouts. The existing `/external` rule ignores them, so the repository's shape and its public prose are
untouched.

**Success.**
- [x] `git check-ignore -v --no-index external/vllm external/triton` answers with the `/external` rule for both
  (receipt: run on 2026-10-25).
- [x] `git -C external/vllm rev-parse --short HEAD` answers ceb01b6ddf and `git -C external/triton rev-parse --short HEAD`
  answers 1ed999d6b2 (receipt: run on 2026-10-25).

**Constraint for every later ticket.**
- A clone at the repo root would be an ignored root folder, which `public_prose_lock` treats as private.
- `docs/cli.md` documents `largo emit --format vllm`, so a root-level vllm folder turns that lock red.
- Keep third-party checkouts under `external/`.

#### SETUP-2 · Pin vLLM and Triton under external/competitors

| Field | Value |
|---|---|
| Phase | setup |
| Labels | `infrastructure` |
| Depends on | none |
| Touches | `external/competitors/vllm`, `external/competitors/triton`, `external/competitors/PINS.tsv` (all local-only); `benchmarks/fusion_parity.json` pins |
| Status | open |

**Goal.**
- `external/competitors/vllm` holds vLLM at ceb01b6ddf6e19510ba138f810d38dd70e665f52, replacing v0.31.0.
- `external/competitors/triton` holds Triton at 1ed999d6b21d7bd9e89b830f9eeb7e5d8ba8934b.
- `PINS.tsv` and the ledger's `pins` both name these commits.

**Read first.**
- `external/competitors/PINS.tsv`;
- `fusion_parity_lock::every_citation_resolves_in_the_pinned_sources`;
- the `competitor-sources` probe in `crates/logicaffeine_tests/tests/common/mod.rs`.

**RED tests.**
- [ ] `fusion_parity_lock::every_citation_resolves_in_the_pinned_sources`, run with `LOGOS_REQUIRE_TOOLS=competitor-sources`.
  After the new pins are written into the ledger and before the trees are refetched, it fails on the PINS.tsv mismatch.
  Once the tree is refetched, it fails on the 8 vLLM anchors that moved, because `csrc/libtorch_stable/` was folded into
  `csrc/`.

**Edge cases.**
- [ ] A codeload tarball whose top directory is named after the commit. Extract into the stack's directory itself.
- [ ] `external/competitors` is a symlink into shared storage on some boxes. Write through it; never replace the link.
- [ ] The extracted tree carries no `.git`. `git log` inside it reports the enclosing repository, so record the commit
  from PINS.tsv only.

**Implementation.**
- [ ] Fetch `https://codeload.github.com/vllm-project/vllm/tar.gz/<commit>` and `https://codeload.github.com/triton-lang/triton/tar.gz/<commit>`.
  Move the old v0.31.0 tree aside until the lock is green, then delete it.
- [ ] Add triton rows to `PINS.tsv` and to the ledger's `pins`.
- [ ] Re-anchor the 8 moved vLLM citations in `benchmarks/fusion_parity.json`.

**Success.**
- [ ] `fusion_parity_lock` all green with `LOGOS_REQUIRE_TOOLS=competitor-sources` (receipt: log path).
- [ ] Every `external/competitors/<stack>/<path>` · `<anchor>` citation in this book resolves to exactly one match
  (receipt: a checker run over the book; see section 13.3).

**Verify.**
```
LOGOS_REQUIRE_TOOLS=competitor-sources cargo nextest run -p logicaffeine-tests --test fusion_parity_lock
```

**Done when.** Both pinned trees are on disk, PINS.tsv and the ledger agree on the commits, and every citation resolves.

#### SETUP-3 · A vLLM install on this box

| Field | Value |
|---|---|
| Phase | setup |
| Labels | `infrastructure` |
| Depends on | none |
| Touches | a virtual environment under `/mnt/build/sota` (outside the repo); `logs/vllm-install/` |
| Owner notes | the performance lane installs competitor engines under `/mnt/build/sota`; coordinate on the lanes board |
| Status | open |

**Goal.** vLLM at the pinned version runs on the 3060 in its own virtual environment. The install records:
- torch, CUDA runtime, Triton and FlashInfer versions;
- the driver;
- one model and one op smoke test.

**Read first.** The four vLLM bring-up traps:
- an EngineCore child re-opens its script by path, so piped stdin cannot work;
- a module-level `LLM()` recurses, so it needs a `__main__` guard;
- FlashInfer JIT-compiles with flags its nvcc must support;
- `timeout` cannot exec a shell function.

All four print "Engine core initialization failed". The real error is on the `(EngineCore pid=…)` lines.

**RED tests.**
- [ ] `vllm_baseline_lock::the_install_records_its_versions`. A new suite. It reads the recorded install manifest under
  `benchmarks/results/models/` and fails today because no manifest exists.

**Edge cases.**
- [ ] A wheel built for a newer CUDA than the driver supports. Record the failure, then pick the wheel matching driver
  580.
- [ ] FlashInfer absent: vLLM falls back to other kernels. Record which backend each op used.
- [ ] Memory headroom at 12 GB for the A/B shapes.

**Implementation.**
- [ ] Install the version matching the clone (v0.32.1rc0 line), or build from the pinned tree when no wheel matches.
- [ ] Write the manifest: versions, the backend chosen per op, and the commands used.

**Success.**
- [ ] One op (`rms_norm`) and one model (Llama-3.2-1B, bf16) run and their outputs are recorded (receipt: log path).
- [ ] `vllm_baseline_lock` green.

**Done when.** vLLM runs here with its versions on record.

#### SETUP-4 · The per-kernel A/B harness and ledger

| Field | Value |
|---|---|
| Phase | setup |
| Labels | `infrastructure` `cuda` |
| Depends on | SETUP-2, SETUP-3, SETUP-6 |
| Touches | a new script under `scripts/accel/`; a new ledger, rtx3060_kernel_ab.json, beside `benchmarks/results/models/rtx3060_scoreboard.json`; a new lock `kernel_ab_lock` |
| Owner notes | GPU START and FINISH on the lanes board |
| Coverage | 3 rows in section 11 |
| Status | open |

**Goal.** Every kernel a PARITY ticket must match has a measured row: every rival kernel section 2.5 names for that op
class, against ours, at our exact shapes, on one boot. The row maximum is the "Copy" number every PARITY ticket cites.

**Copy.** Measure the way Triton's `do_bench` does (`external/competitors/triton/python/triton/testing.py` · `def do_bench(`):
- zero a buffer larger than L2 before every timed launch of a decode-shaped kernel;
- run r ≥ 5 timed repetitions after a warmup;
- record every run.

**Beyond.** Our rows record kernel time from nsys kernel sums (`nsys stats -r gpukernsum`), not wall clock, beside the
wall-clock figure.

**RED tests.**
- [ ] `kernel_ab_lock::every_row_has_both_sides_and_the_same_number_of_runs`
- [ ] `kernel_ab_lock::every_ratio_is_derived_from_the_runs`
- [ ] `kernel_ab_lock::a_win_is_counted_only_past_two_standard_errors`
- [ ] `kernel_ab_lock::every_shape_is_one_our_scoreboard_models_run`
- [ ] `kernel_ab_lock::every_row_has_every_rival_section_2_5_names_or_a_named_refusal`
- [ ] `kernel_ab_lock::the_row_maximum_is_derived_and_a_copy_tick_meets_it`

Each fails today because the ledger does not exist.

**Edge cases.**
- [ ] A vLLM op that does not run on sm_86 (fp8 hardware, sm90-only kernels). Refuse it by name in the row; never drop
  it.
- [ ] Decode-shaped kernels (working set below L2). The L2 flush is mandatory for these.
- [ ] Different numerics. vLLM bf16 vs our int8: each row records both sides' numerics, and a row comparing unlike
  numerics says so.

**Implementation.**
- [ ] Shapes from the six scoreboard models: decode and a 512-row prefill chunk per op.
- [ ] Ops to cover:
  - norms: `rms_norm`, `fused_add_rms_norm`;
  - `silu_and_mul`;
  - rotation: `rotary_embedding`, `fused_qk_norm_rope`;
  - routing: `topk_softmax`, `grouped_topk`;
  - the Triton `fused_moe_kernel`;
  - attention: FlashAttention-2 decode and prefill, unified attention, `merge_attn_states`;
  - GDN: chunk prefill and packed decode;
  - sampling;
  - the Triton tutorial kernels from SETUP-6.
- [ ] Rivals per op, beyond vLLM's dispatched kernel and Triton's: every name in section 2.5's row for the op class:
  - llama.cpp's ggml-cuda kernels (`mul_mat_vec_q`, `mul_mat_q`, `mul_mat_id`, flash attention, row kernels), from its
    `test-backend-ops` perf mode at our shapes;
  - ExLlamaV3's GEMV and GEMM;
  - FlashInfer's decode, prefill and sampling;
  - FlashAttention-2;
  - cuBLAS and cuBLASLt (f16, bf16 and int8);
  - TensorRT-LLM's weight-only GEMV, XQA and MoE kernels.
- [ ] Each row stores every rival's time and the row maximum, derived; a Copy box is ticked against the maximum.

**Success.**
- [ ] The ledger holds a row for every op above at every shape (receipt: ledger path, boot id).
- [ ] `kernel_ab_lock` green.

**Mutants to kill.**
- [ ] A typed ratio.
- [ ] A side dropped.
- [ ] A win counted inside 2 SE.
- [ ] A rival dropped from a row, or a Copy tick below the row maximum.

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test kernel_ab_lock
```

**Done when.** Every PARITY ticket can quote its Copy number from this ledger.

#### SETUP-5 · vLLM as the scoreboard's third side

| Field | Value |
|---|---|
| Phase | setup |
| Labels | `infrastructure` `cuda` |
| Depends on | SETUP-3 |
| Touches | `scripts/accel/model_scoreboard.py`, `benchmarks/results/models/rtx3060_scoreboard.json`, `model_scoreboard_lock` |
| Owner notes | these files belong to the performance lane today |
| Status | open |

**Goal.** Each scoreboard row gains a vLLM side on the same boot.
- vLLM no longer loads GGUF in core, so it runs the matching Hugging Face checkpoint at matched precision: int8 W8A8
  beside our one-word int8 lane, and bf16 beside our Dequant lane.
- Each row records the precision match it uses.

**RED tests.**
- [ ] `model_scoreboard_lock::every_row_has_a_vllm_side_or_a_named_refusal`
- [ ] `model_scoreboard_lock::the_vllm_side_names_its_checkpoint_and_precision`

Both fail today because the schema has no vLLM side. This changes what an existing lock asserts, so it needs Tristen's
approval before landing.

**Edge cases.**
- [ ] A model vLLM cannot run on sm_86 or in 12 GB. A named refusal, not a missing row.
- [ ] Prompt and generation lengths matched exactly (6-token prompt and 128 generated for decode; 512 prefill).
- [ ] Our slowest run must beat their fastest for a quoted win.

**Success.**
- [ ] Six rows with three sides each, on one boot (receipt: ledger and boot id).
- [ ] `model_scoreboard_lock` green.

**Done when.** The scoreboard quotes vLLM beside llama.cpp, on one boot.

#### SETUP-6 · A Triton build at the pin

| Field | Value |
|---|---|
| Phase | setup |
| Labels | `infrastructure` |
| Depends on | SETUP-2 |
| Touches | a virtual environment under `/mnt/build/sota`; `logs/triton-build/` |
| Status | open |

**Goal.** Triton at 1ed999d6b2 is built (or a wheel at the nearest release is installed, with the difference recorded).
The tutorial kernels and `triton_kernels` run on the 3060 for SETUP-4.

**RED tests.**
- [ ] `vllm_baseline_lock::the_triton_build_records_its_commit`. It fails today because nothing records it.

**Edge cases.**
- [ ] Tutorials that need sm90 or sm100 (TMA, warp specialization, block scaling): refused by name on this box.
- [ ] The LLVM build's disk footprint. Build on `/mnt/build`, never on `/`.

**Success.**
- [ ] The softmax, layer-norm, attention, grouped-GEMM, persistent-matmul, MoE matmul, SwiGLU and top-k kernels run on
  the 3060 or are refused by name (receipt: log path).

**Done when.** SETUP-4 can time Triton's kernels at our shapes.

#### SETUP-7 · Every 3060 rival on one boot

| Field | Value |
|---|---|
| Phase | setup |
| Labels | `infrastructure` `cuda` |
| Depends on | SETUP-3, SETUP-5 |
| Touches | virtual environments and builds under `/mnt/build/sota` (outside the repo); `scripts/accel/model_scoreboard.py`; a new cell ledger, sota_cells.json, beside `benchmarks/results/models/rtx3060_scoreboard.json`; a new lock `rival_baseline_lock` |
| Owner notes | the scoreboard recorder belongs to the performance lane today; GPU START and FINISH on the lanes board for every run |
| Status | open |

**Goal.** Every stack in section 2.2 that runs on the 3060 or the i9-14900K has a side in every cell of section 2.4
for its chip, measured on one boot under section 2.3, or a named refusal. These stacks are SGLang, TensorRT-LLM,
ExLlamaV3, MLC-LLM, llama.cpp at its pin, vLLM, and the two CPU backends. The cell ledger exists and holds them.

**Copy.** The serving stacks' own benchmark drivers, so each rival runs the way its authors measure it:
- vLLM's `vllm bench`;
- SGLang's `bench_one_batch` and `bench_serving`;
- TensorRT-LLM's `trtllm-bench`;
- llama.cpp's `llama-bench`;
- ExLlamaV3's own benchmark;
- MLC's `mlc_llm bench`.

**Beyond.**
- Every rival's per-token kernel census is recorded with nsys beside its tok/s, so a lost cell is diagnosed from the
  ledger alone (section 2.6).
- The census also feeds the "launch census matched" mechanism of section 2.1.

**Read first.**
- `scripts/accel/model_scoreboard.py`;
- `benchmarks/results/models/rtx3060_scoreboard.json`;
- the vLLM bring-up traps in SETUP-3;
- `logs/demo1102-sota/vllm/` (the 2026-10-25 vLLM 0.32 runs, and why FP8 checkpoints needed the CUDA 13 NVRTC
  libraries on `LD_LIBRARY_PATH`).

**RED tests.**
- [ ] `rival_baseline_lock::every_rival_records_its_version_install_and_flags`
- [ ] `rival_baseline_lock::every_cell_on_this_box_has_a_side_per_rival_or_a_named_refusal`
- [ ] `rival_baseline_lock::every_side_names_its_checkpoint_and_bits_per_weight`
- [ ] `rival_baseline_lock::no_rival_side_reads_more_weight_bytes_than_ours`
- [ ] `rival_baseline_lock::every_decode_cell_records_both_kernel_censuses`

Each fails today because sota_cells.json does not exist.

**Edge cases.**
- [ ] A rival with no checkpoint at the cell's bits (no EXL3 for a model, no AWQ for Gemma-2). It gets a named refusal
  with the search that was made, never a silent gap.
- [ ] A rival that crashes or runs out of memory at 12 GB. A named refusal with the log path.
- [ ] TensorRT-LLM's engine build per shape. The build time is recorded, and is not counted in the cell.
- [ ] ExLlamaV3's JIT extension build. It needs the CUDA toolkit headers the box carries. Record the failure if it
  does not build, then retry with the matching toolkit.
- [ ] MLC's compiled model library per model and quantization. Pinned by version in the row.
- [ ] Rivals that sample by default. Force greedy in decode and prefill cells.

**Implementation.**
- [ ] Install each rival in its own environment under `/mnt/build/sota`, recording versions.
- [ ] Fetch each rival's checkpoint at matched bits under `/mnt/build/sota/hf`, recording its bits per weight with
  scales.
- [ ] Extend the recorder to run every rival per cell, five runs a side, same boot, with nsys censuses for decode.
- [ ] Write sota_cells.json: one row per cell, with every side, its runs, its flags, and its quality agreement.

**Success.**
- [ ] Every 3060 and CPU cell of section 2.4 has every rival side or a named refusal (receipt: ledger path, boot id).
- [ ] `rival_baseline_lock` green.

**Mutants to kill.**
- [ ] A rival side with fewer runs than ours.
- [ ] A rival checkpoint wider than ours, accepted.
- [ ] A refusal with no reason.

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test rival_baseline_lock
```

**Done when.** Every cell on this box names the fastest rival, by measurement.

#### SETUP-8 · Baselines on rented hardware

| Field | Value |
|---|---|
| Phase | setup |
| Labels | `infrastructure` `datacenter` |
| Depends on | SETUP-7, SETUP-9 |
| Touches | the cell ledger sota_cells.json; a runbook under `scripts/accel/` |
| Owner notes | renting any machine is Tristen's decision. Nothing is rented, bought or provisioned without his word, and every row records the machine and what it cost |
| Status | open |

**Goal.** The rented cells of section 2.4 (H100, B200, eight H100s, MI300X, Arc B580) each have their rival sides and
our side, measured under section 2.3 on the rented machine.

**Copy.** The rivals' published configurations for each GPU: vLLM's recipes, SGLang's, TensorRT-LLM's
`trtllm-bench` defaults. Each is run as published, then tuned.

**Beyond.** One script provisions a machine, runs every side of every cell for that GPU in one session, and writes the
rows. A rented hour therefore produces a complete column, not a single number.

**RED tests.**
- [ ] `rival_baseline_lock::a_rented_row_records_its_machine_driver_and_cost`
- [ ] `rival_baseline_lock::a_rented_cell_has_our_side_and_every_rival_side_from_one_session`

Both fail today because no rented row exists.

**Edge cases.**
- [ ] A cloud image whose driver differs from the rival's wheel. Record the driver, and pick the matching wheel.
- [ ] Multi-GPU topology (NVLink, or PCIe only). Recorded, since it decides all-reduce cost.
- [ ] A session that ends before every side ran. The partial rows are kept and marked incomplete; the cells stay open.

**Implementation.**
- [ ] A runbook script: install the rivals, fetch the checkpoints, build ours, run every cell, write the rows, and
  copy back the logs.
- [ ] Our side runs the artifacts built and `compile_verified` here (LEDGER-1).

**Success.**
- [ ] Every rented cell has a row with every side (receipt: ledger rows, machine ids).
- [ ] `rival_baseline_lock` green.

**Mutants to kill.**
- [ ] A rented row without its cost.
- [ ] Sides from two sessions merged into one row.

**Done when.** Every rented cell is measured, on Tristen's approval to rent.

#### SETUP-9 · The serving benchmark

| Field | Value |
|---|---|
| Phase | setup |
| Labels | `infrastructure` `serving` |
| Depends on | SETUP-3 |
| Touches | a trace file and a client in a new serving directory under `benchmarks/`; the cell ledger sota_cells.json; a new lock `serving_bench_lock` |
| Status | open |

**Goal.** One serving trace and one client drive every stack's server the same way, and measure the S cells of
section 2.4.
- The trace: 1,000 requests, prompt and output lengths drawn from the ShareGPT distribution with seed 0, Poisson
  arrivals, and the trace's sampling parameters.
- The metric: goodput, the highest request rate each stack sustains with p99 time per output token ≤ 100 ms and p99
  time to first token ≤ 2 s.

**Copy.** vLLM's serving benchmark (`vllm bench serve`) and its request-rate sweep. SGLang's `bench_serving` is the
cross-check.

**Beyond.**
- The trace is a checked-in file with its digest, not a dataset download at run time, so every stack and every rerun
  sees the same requests.
- The client is one program for every stack, using each stack's OpenAI-compatible endpoint.

**RED tests.**
- [ ] `serving_bench_lock::the_trace_is_pinned_by_seed_and_digest`
- [ ] `serving_bench_lock::every_stack_ran_the_same_trace`
- [ ] `serving_bench_lock::goodput_is_derived_from_the_recorded_latencies`

Each fails today because the trace does not exist.

**Edge cases.**
- [ ] A stack that rejects a request (context too long for its configuration). The rejection counts against its
  goodput, and is recorded.
- [ ] Warm-up: the first 50 requests are excluded on every side alike.
- [ ] Prefix sharing in the trace. The trace records which requests share prefixes, so prefix caching is exercised
  the same way on every stack.

**Implementation.**
- [ ] Generate the trace and check it in with its digest.
- [ ] The client: replay the trace at a rate, record TTFT and TPOT per request, and sweep rates to find goodput.
- [ ] Our server: the generated program behind an OpenAI-compatible endpoint (SERVE-CONTINUOUS-BATCHING delivers it).

**Success.**
- [ ] vLLM and SGLang S rows on the 3060 (receipt: ledger rows).
- [ ] `serving_bench_lock` green.

**Mutants to kill.**
- [ ] A trace regenerated with another seed.
- [ ] Goodput typed rather than derived.

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test serving_bench_lock
```

**Done when.** Every S cell can be measured by one command per stack.

### 12.5 Ledger

#### LEDGER-1 · Grow the ledger to the catalog

| Field | Value |
|---|---|
| Phase | ledger |
| Labels | `infrastructure` |
| Depends on | SETUP-2 |
| Touches | `benchmarks/fusion_parity.json`, `crates/logicaffeine_tests/tests/fusion_parity_lock.rs` |
| Owner notes | changes what `fusion_parity_lock` asserts: Tristen approves the schema first |
| Status | open |

**Goal.** Every row of section 10 is a ledger row.
- New fields per class:
  - `kind` (fused, primitive, schedule);
  - `chips` (a status per chip: cuda_lane, cuda_forward, spatial, fabric, tensix, rdu, d5005);
  - `schemes`, `models`, `regime`.
- New sections: `techniques` and `graph`.
- A new status, `compile_verified`: built and compile-checked here, with its GPU values test waiting for a datacenter
  GPU.
- Stale notes fixed:
  - `norm_scale`: the scale sugar exists, the norm fold does not;
  - `relu_sqr`: relu sugar exists, the lane refuses it;
  - `kv_cache_quant`: the D5005 engine has int8 caches.

**RED tests.**
- [ ] `fusion_parity_lock::every_class_names_its_kind_and_a_status_per_chip`
- [ ] `fusion_parity_lock::a_compile_verified_row_names_its_compile_census_and_its_replica_values_test`
- [ ] `fusion_parity_lock::every_technique_names_its_kernel_census_and_its_ab_row`

All three fail today because the fields do not exist.

**Edge cases.**
- [ ] A class compiled on one chip and a gap on another.
- [ ] A primitive class that a chip links whole on one chip and decomposes on another.
- [ ] The inventory counts (`CLASSES`, `SERVING`) move with the new rows, deliberately.

**Implementation.**
- [ ] Add every catalog row with its anchors at the new pins. Status starts at gap unless a receipt exists.
- [ ] Raise `COMPILED_FLOOR` only to what is proven.

**Success.**
- [ ] `fusion_parity_lock` green with `LOGOS_REQUIRE_TOOLS=competitor-sources` (receipt: log path).

**Mutants to kill.**
- [ ] A status marked compiled with no census.
- [ ] A chip status missing.
- [ ] A `compile_verified` row with no replica test.

**Done when.** The ledger is the catalog, row for row.

#### LEDGER-2 · Hold the book and the ledger together

| Field | Value |
|---|---|
| Phase | ledger |
| Labels | `infrastructure` |
| Depends on | LEDGER-1 |
| Touches | `crates/logicaffeine_tests/tests/fusion_parity_lock.rs` |
| Status | open |

**Goal.** This book and `benchmarks/fusion_parity.json` cannot drift apart.

**RED tests.**
- [ ] `fusion_parity_lock::the_book_names_every_ledger_id`
- [ ] `fusion_parity_lock::every_id_the_book_catalogs_is_a_ledger_row`
- [ ] `fusion_parity_lock::every_ticket_the_book_cites_is_on_the_board`
- [ ] `fusion_parity_lock::every_book_citation_resolves_once_at_the_pins` (gated on `competitor-sources`)

These are new tests. The first fails today until LEDGER-1 adds the new rows.

**Edge cases.**
- [ ] An id that appears in prose but not in a catalog checklist row. Only catalog rows count.
- [ ] A ticket id cited that is missing from the board index checklist.
- [ ] An anchor that appears twice in its file.

**Success.**
- [ ] All four green (receipt: log path).

**Mutants to kill.**
- [ ] Delete a catalog row.
- [ ] Add an orphan catalog row.
- [ ] Cite a ticket not on the board.
- [ ] Duplicate an anchor.

**Done when.** The book, the board and the ledger are held to each other by one lock.

#### LEDGER-3 · The SOTA contract lock

| Field | Value |
|---|---|
| Phase | ledger |
| Labels | `infrastructure` |
| Depends on | LEDGER-2, SETUP-2, SETUP-7 |
| Touches | a new lock `sota_contract_lock`; the cell ledger sota_cells.json; this book's sections 2, 3 and 11 |
| Owner notes | the cell ledger's schema is new; Tristen approves it before it lands |
| Status | open |

**Goal.** The contract of section 2 is machine-checked, so "every box ticked" and "state of the art" cannot drift apart.
- Section 11's coverage map is re-extracted from the pinned trees and must match the book row for row.
- Every section 2.4 cell must name closers that are on the board.
- A ticked cell must have a won row in the cell ledger under section 2.3's rules.

**Read first.**
- Section 2;
- section 11;
- the extraction rules in section 13.4;
- `fusion_parity_lock` (the pattern for a ledger held to a book).

**RED tests.**
- [ ] `sota_contract_lock::every_entry_at_the_pins_has_a_coverage_row` (gated on `competitor-sources`): runs section
  12.4's rules over `external/competitors/vllm` and `external/competitors/triton` and finds every entry in section 11.
- [ ] `sota_contract_lock::every_coverage_row_names_an_entry_at_the_pins` (gated on `competitor-sources`)
- [x] `sota_contract_lock::every_coverage_row_names_tickets_on_the_board` (receipt: `logs/demo-strategy/sota-contract-book2.log`)
- [ ] `sota_contract_lock::a_ticked_coverage_row_belongs_to_a_done_ticket`
- [ ] `sota_contract_lock::every_cell_names_closers_on_the_board`
- [ ] `sota_contract_lock::a_ticked_cell_has_a_won_ledger_row`
- [ ] `sota_contract_lock::a_won_row_meets_every_fair_win_rule`: one boot, matched bits, matched quality, matched work,
  at least five runs a side, more than two combined standard errors, and our slowest run beating the rival's fastest.
- [ ] `sota_contract_lock::no_cell_is_ticked_while_a_closer_is_open`
- [ ] `sota_contract_lock::every_cell_id_is_unique_and_braced_lists_expand`

Each fails today because the lock does not exist.

**Edge cases.**
- [ ] A competitor tree absent from the box. The two pin-reading tests skip with the `competitor-sources` probe; the rest
  run everywhere.
- [ ] A rival side refused by name. It does not count as a win, and it is not a rival: the cell is won against the
  fastest rival that ran.
- [ ] A cell where every rival refused. It cannot be ticked. A cell with no rival is not a claim.
- [ ] A row from an earlier boot than a rival's newer release. The claim stands for its date; the lock reports which
  ticked cells predate a rival version recorded later.

**Implementation.**
- [ ] The extractor of section 13.4, ported to Rust in the test's common module, reading the pinned trees.
- [ ] A parser for section 11's rows, section 2.4's cells and their closers, and the board index.
- [ ] The cell ledger's schema: per cell, the boot id, the date, and per side the stack, version, flags, checkpoint,
  bits per weight, runs, quality agreement and census.

**Success.**
- [ ] `sota_contract_lock` green with `LOGOS_REQUIRE_TOOLS=competitor-sources` (receipt: log path).

**Mutants to kill.**
- [ ] A coverage row deleted.
- [ ] An entry added to a copy of a pinned tree with no row.
- [ ] A cell ticked with a row inside two standard errors.
- [ ] A cell ticked where our side read fewer weight bytes than the rival's.
- [ ] A cell ticked while one of its closers is open.

**Verify.**
```
LOGOS_REQUIRE_TOOLS=competitor-sources cargo nextest run -p logicaffeine-tests --test sota_contract_lock
```

**Done when.** The book cannot claim a cell it has not won, and cannot miss a kernel the pins ship.

### 12.6 The one IR

#### IR-0 · Digests before code

| Field | Value |
|---|---|
| Phase | IR |
| Labels | `infrastructure` |
| Depends on | none |
| Touches | new suites `one_ir_identity_lock` and `one_ir_lock`, and their fixtures in `crates/logicaffeine_tests/tests/fixtures/` |
| Owner notes | record the lane digests inside a freeze window granted by the lane's owner, and name the tree state the fixture was taken at |
| Status | done |

**Goal.** Every artifact the migration could change is pinned by digest, on today's tree, before any migration edit:
- lane programs;
- engine programs;
- spatial artifacts;
- the older forward's text.

**Read first.**
- `model_author_migration_lock` (the precedent: fixtures recorded once with `LOGOS_UPDATE_FIXTURES=1`);
- `lane_compile_lock::every_fusion_rule_can_be_held_back`;
- `spatial_backend_totality_lock`;
- `d5005_lower_graph_lock`.

**RED tests.** These pin today's behaviour, so they are green on today's tree by design. Their RED moment is a mutation:
change one emitted byte and watch each fail.
- [x] `one_ir_lane_identity_lock::every_lane_program_is_the_pinned_one` (receipt: logs/one-ir/ir0-land-1028.log, 2700 pins on 51ceaa25e): the `.cu`, the
  sidecar and the schedule's debug text for every tiny architecture × three numerics × prefill 1 and 4 × held {none,
  all, each rule alone} × `fuse_quant` on and off, and the two storage knobs at their non-default points.
- [x] `one_ir_identity_lock::every_engine_program_is_the_pinned_one` (receipt: logs/one-ir/migrate4-verify-1027.log): the encoded words at every knob point, for the tiny
  models and the 27B header ledgers.
- [x] `one_ir_identity_lock::every_spatial_artifact_is_the_pinned_one` (receipt: logs/one-ir/migrate3-ir1-verify-1027.log): the totality grid's programs and the FPGA decode
  programs, covering fabric Verilog, Tensix, RDU neutral bundles and persistent CUDA.
- [x] `one_ir_identity_lock::every_forward_program_is_the_pinned_one` (receipt: logs/one-ir/migrate2-pre-1027.log, 70 pins): the corpus of `cuda_fusion_census_lock` and the
  `cuda_forward_*` locks.
- [x] `one_ir_lane_identity_lock::the_pins_cover_every_rule_held_alone` (receipt: logs/one-ir/ir0-land-1028.log): the held-alone set equals today's rule table.
- [x] `one_ir_lock::no_fusion_rule_is_added_outside_the_tensor_graph` (receipt: logs/one-ir/migrate4-recheck3-1027.log): a shrink-only allowlist of today's 16 lane rules,
  the forward's chain and add-norm logic, and `lower_graph`'s merging. It fails on any new rewrite outside
  `tensor_graph`.

**Edge cases.**
- [x] Emission order that varies between runs. Generate twice and compare before pinning (`hold` refuses a family whose two generations differ).
- [x] A peer's uncommitted lane edit live at recording time. Record inside the freeze window only (the lane's pins wait in `one_ir_lane_identity_lock`).
- [x] A new rule added later: the held-alone set must grow with it, or the lock fails (killed: logs/one-ir/mutants-ir0b-1028.log).

**Success.**
- [x] All six tests green on today's tree (receipt: logs/one-ir/ir0-land-1028.log).
- [x] Mutation kills (receipt: per mutant, the failing test; logs/one-ir/mutants-ir0b-1028.log, logs/one-ir/mutants-s1.log):
  - one byte flipped in each artifact family is caught;
  - a rule added to the lane table is caught;
  - a rule removed from the held-alone set is caught.

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test one_ir_identity_lock --test one_ir_lock
```

**Done when.** No later migration step can change an artifact without a lock failing.

#### IR-1 · The IR's home

| Field | Value |
|---|---|
| Phase | IR |
| Labels | `infrastructure` |
| Depends on | IR-0 |
| Touches | `crates/logicaffeine_compile/src/tensor_graph/` (the front end moves in, from `spatial/passes/recognize_stages.rs`), `crates/logicaffeine_compile/src/codegen_cuda/lane/graph.rs` (moves), about 32 import paths |
| Owner notes | the D5005 files change only their imports and two accessor calls in this window: a handoff with d5005-model-foundry and qwen-fpga-decode-engine |
| Status | claimed |

**Goal.** A pure move with every digest unchanged:
- the front end moves to `tensor_graph::front`;
- `StepGraph` becomes `TensorGraph` in `tensor_graph::graph`, gaining `elem`, `fill`, `storage`, `range` and `observed`;
- `Part`, `Dst` and `RowSel` move in;
- `Op` gains `access`, `dst`, `keeps` and `covers`, plus the accessors `prim()` and `kind()`.

**RED tests.**
- [ ] `one_ir_lock::the_tensor_graph_lives_in_tensor_graph`: no `StepGraph` type outside `tensor_graph`.
- [x] `one_ir_lock::the_front_end_lives_in_tensor_graph` (receipt: logs/one-ir/migrate4-recheck3-1027.log): `recognize_stages` is defined in `tensor_graph::front` only.
- [x] `one_ir_lock::every_tensor_carries_its_facts`: every tensor of every tiny cut has `elem`, `storage`, and `range`
  where integer.
- [x] `one_ir_lock::state_writes_are_explicit`: `writes_in_place()` equals today's `state_inputs()` plus row and unit
  destinations.
- [x] `one_ir_lock::every_lifted_graph_records_its_cut`: `identity` is the cut's program identity.

**Edge cases.**
- [ ] A literal-row view of a table that is rewritten before the row's last reader becomes a copy, not a view
  (`lane_compile_lock::a_row_of_a_table_written_before_its_last_reader_is_copied_where_it_is_taken`).
- [x] An integer tensor with no proven range (unranged: `every_tensor_carries_its_facts`).
- [x] A tensor a `Show` statement observes (`every_tensor_carries_its_facts`).

**Success.**
- [ ] The five tests green.
- [x] `one_ir_identity_lock` unchanged (engine, spatial and forward families).
- [ ] `lane_compile_lock`, `d5005_lower_graph_lock`, `spatial_backend_totality_lock` and `bringup_lock` green (receipt:
  log path).
- [ ] `cargo test --no-run --no-fail-fast` over the workspace builds (receipt: log path).

**Done when.** The IR has one home and nothing produced changed.

#### IR-2 · The evaluator, equal to the VM

| Field | Value |
|---|---|
| Phase | IR |
| Labels | `infrastructure` |
| Depends on | IR-1 |
| Touches | `tensor_graph::eval`, `tensor_graph::corpus` (generators lifted from `crates/logicaffeine_compile/src/spatial/equiv.rs`) |
| Status | done |

**Goal.** What a graph computes can be answered without a chip.
- `eval(g, inputs, Mode::{Exact, AtNumerics(n)})` sums in the authored loop order, so it equals the VM bit for bit.
- `Quantize` is the identity under `Exact` and a quantize-then-dequantize under `AtNumerics`.

**Read first.**
- `spatial_stacked_rows_lock` (the harness that prints `item i of X` and compares with the VM);
- `crates/logicaffeine_compile/src/spatial/equiv.rs` (seeded corpora).

**RED tests.**
- [x] `tensor_graph_eval_lock::the_evaluator_computes_what_the_vm_computes_for_every_tiny_cut`
- [x] `tensor_graph_eval_lock::every_op_kind_has_an_evaluation_or_a_named_refusal`
- [x] `tensor_graph_eval_lock::access_and_destination_mean_what_they_say`

All fail today because `tensor_graph::eval` does not exist.

**Edge cases.**
- [x] Every `TensorOpSpec` variant: Matmul, Elementwise, SwiGlu, FusedRow (RMSNorm, Softmax, LayerNorm), Rope, Silu,
  Sigmoid, Gelu, Scale, Decay, L2Norm, Transpose, HeadSlice, Join, HeadMatmul, CausalConv, GatedDelta, Attention,
  FullAttention, Moe, RowGather, RowsGather, ArgMax, SetRow, IntRequant, IntClamp, Relu, LutAttention.
- [x] Strided parts at the last row.
- [x] Row destinations at row 0 and at the last cache row.
- [x] State carried across two steps.
- [x] Windows of one position and a full window.
- [x] Sinks.
- [x] Values narrower than keys (latent attention).
- [x] −0.0 and NaN propagation.
- [x] Integer overflow at the range bound.

**Success.**
- [x] All three green, for every tiny architecture and every op probe (receipt: logs/one-ir/migrate3-ir1-verify-1027.log).

**Mutants to kill.**
- [x] Swap the summation order of one primitive (killed: logs/one-ir/mutants-s1b-1027.log).
- [x] Drop the sink term (killed).
- [x] Ignore a row destination's offset (killed).

**Done when.** The IR has a meaning every backend is held to.

#### IR-3 · The symbol registry

| Field | Value |
|---|---|
| Phase | IR |
| Labels | `infrastructure` |
| Depends on | IR-1, LEDGER-1 |
| Touches | `tensor_graph::symbols` |
| Status | done |

**Goal.**
- Every class has a registered symbol: `Class { id, kind, ledger, exemplar }`. That covers every ledger id plus the
  "ours" ids `strided_read`, `join_in_place`, `strided_values`, `embed_scale`, `gdn_gating`, `conv_silu_l2norm`,
  `gdn_output_quant` and `gemv_quant`.
- Gap classes are registered with no rule yet, so a mined fusion has a place to land.
- `exemplar` is a minimal LOGOS program containing the pattern.

**RED tests.**
- [x] `fusion_class_lock::every_ledger_class_is_a_registered_symbol`
- [x] `fusion_class_lock::every_registered_symbol_names_its_kind`
- [x] `fusion_class_lock::no_symbol_id_is_both_ledger_and_ours`
- [x] `fusion_class_lock::every_exemplar_lifts_and_contains_its_pattern`

**Edge cases.**
- [x] A class whose exemplar needs a model shape (MoE, latent attention). Use the tiny models' dimensions.
- [x] A schedule class (`cuda_graphs`, `rows_a_warp`) has no exemplar rule.

**Success.**
- [x] All four green (receipt: logs/one-ir/migrate3-ir1-verify-1027.log).

**Done when.** Every fusion we will ever add has a name and a home before its rule exists.

### 12.7 The shared optimizer

#### OPT-1 · Facts, the rule engine and the shared rules

| Field | Value |
|---|---|
| Phase | optimizer |
| Labels | `infrastructure` |
| Depends on | IR-2, IR-3 |
| Touches | `tensor_graph::facts`, `tensor_graph::fuse`, `tensor_graph::numerics`, `tensor_graph::shim` (`ReferenceShim`) |
| Coverage | 9 rows in section 11 |
| Status | done |

**Goal.**
- **Facts:** producer, ordered readers, writers, only reader, untouched between, observed, carried, results, element
  type, range and storage. Computed once and updated as ops are absorbed.
- **The rule engine:** `optimize(g, shims, held)`.
- **The shared rules:** the 15 lane rules that are shared plus `argmax_advance`, `fused_quant`, `quantize` and
  `merged_projections`, all expressed in `tensor_graph::fuse` under the same names and anchor slots.
- **Kernel predicates move to the shim:** every condition that is really about a kernel leaves the rule for
  `LaneShim::allows` (section 5.9).

**Read first.**
- `crates/logicaffeine_compile/src/codegen_cuda/lane/plan.rs` (the rule functions and `RULE_TABLE`);
- `lane_compile_lock::the_in_place_folds_hold_back_where_a_launch_between_would_see_the_difference`.

**RED tests.**
- [x] `fusion_class_lock::every_rule_preserves_its_exemplars_meaning`: fused equals parts under `eval` on seeded
  corpora.
- [x] `fusion_class_lock::every_rule_holds_back_where_its_dataflow_forbids`
- [x] `fusion_class_lock::every_rule_can_be_held`
- [x] `one_ir_lock::the_shared_table_fires_what_the_lane_fires` (415 forms): the shared engine under `LaneShim` against `plan.rs`,
  rule by rule, for every tiny architecture × numerics × form. It fails until every rule fires identically.

**Edge cases.** One witness each:
- [x] a reader between producer and consumer;
- [x] a write to the source between;
- [x] a second reader;
- [x] an observed intermediate;
- [x] a result intermediate, and
- [x] a carried state: a run never carries a step's logits or its next token in its registers, and its twin through a
  name the host does not read does (`fusion_class_lock::the_rules_keep_what_the_host_reads_across_steps`);
- [x] a view that is quantized in place only when its offset is whole 32-blocks
  (`fusion_class_lock::a_view_is_quantized_in_place_only_at_whole_blocks`; receipt: logs/one-ir/opt1-edges-1027.log,
  mutants killed: logs/one-ir/mutants-opt1e-1027.log).

**Success.**
- [x] All four green (receipt: logs/one-ir/migrate3-ir1-verify-1027.log).
- [x] `one_ir_identity_lock` unchanged: nothing is switched yet (receipt: logs/one-ir/migrate3-ir1-verify-1027.log).

**Mutants to kill.**
- [x] Drop the "untouched between" check from one rule (killed: logs/one-ir/mutants-s1b-1027.log).
- [x] Fire a rule on a second reader (killed: logs/one-ir/mutants-s1.log).
- [x] Reorder two rules (killed: logs/one-ir/mutants-s1.log).

**Done when.** The shared table decides exactly what the lane decides today.

#### OPT-2 · The shim trait and its five shims

| Field | Value |
|---|---|
| Phase | optimizer |
| Labels | `infrastructure` |
| Depends on | OPT-1 |
| Touches | `tensor_graph::shim`; `codegen_cuda/lane/shim.rs` (new, lane-owned); a new forward shim in `codegen_cuda`; a new spatial shim; `spatial/decode_engine/shim.rs` (peer-owned) |
| Owner notes | the lane shim needs the lane owner's window; the engine shim belongs to d5005-model-foundry |
| Coverage | 19 rows in section 11 |
| Status | done |

**Goal.** Every chip says what it allows through one trait (section 5.6), built from its existing descriptor:

| Shim | Answers |
|---|---|
| `LaneShim` | its link succeeding |
| `ForwardShim` | `kernel_refusal`'s refusals |
| `SpatialShim` | `ComputeCap`, precisions and roofs |
| `EngineShim` | `spec_refusals`, the `lower_graph` refusals, and `covers` as limits |
| `ReferenceShim` | everything |

**RED tests.**
- [x] `shim_lock::every_shim_answers_every_class`: an exhaustive match with no catch-all arm; adding a class must fail
  the build until every shim answers.
- [x] `shim_lock::the_lane_shim_is_its_link`: `allows` is `Native` exactly when the lane's link builds a launch.
- [x] `shim_lock::every_native_answer_links`
- [x] `shim_lock::every_refusal_names_its_reason`
- [x] `shim_lock::a_decomposed_class_whose_part_is_refused_is_refused_whole_by_name`

**Edge cases.**
- [x] A chip-only rule registered `Before("add_norm")`, `After(name)` and `End`, each holdable (`shim_lock::a_chips_own_rules_run_where_they_are_linked_and_can_be_held`).
- [x] A shim answering for a class registered after it was written: the build fails (every `supports` is an exhaustive match).

**Success.**
- [x] All five green, with all five shims: `LaneShim`, `ForwardShim`, `SpatialShim`, `EngineShim`, `ReferenceShim` (receipt: logs/one-ir/migrate4-verify-1027.log).

**Mutants to kill.**
- [x] A catch-all arm added (killed: logs/one-ir/mutants-s1.log).
- [x] A refusal with an empty reason (killed: logs/one-ir/mutants-s1.log).
- [x] `LaneShim` answering `Native` where the link fails (killed: logs/one-ir/mutants-s1.log, logs/one-ir/mutants-s1b-1027.log).

**Done when.** Every chip's capabilities are declared data behind one interface.

#### OPT-3 · The fusion report

| Field | Value |
|---|---|
| Phase | optimizer |
| Labels | `infrastructure` |
| Depends on | OPT-1 |
| Touches | `tensor_graph::fuse` |
| Status | claimed |

**Goal.** Every compile records:
- each rule that could fire, and where;
- whether it fired or declined (illegal, refused by the chip, unprofitable, held).

Artifacts carry the report's digest beside the program identity.

**RED tests.**
- [x] `fusion_report_lock::every_rule_that_could_fire_is_reported`
- [x] `fusion_report_lock::holding_a_rule_moves_it_to_declined_and_changes_nothing_else`
- [x] `fusion_report_lock::the_report_is_deterministic`
- [ ] `fusion_report_lock::every_artifact_carries_its_reports_digest` (after MIGRATE-1: the lane's artifact is the last to come from the shared optimizer)

**Edge cases.**
- [x] A rule that could fire at two anchors and fires at one.
- [x] A rule declined for cost on one chip and fired on another.

**Success.**
- [ ] All four green (receipt: log path).

**Done when.** The fusions a model received on a chip can be read off its artifact.

#### OPT-4 · Equality saturation over the IR

| Field | Value |
|---|---|
| Phase | optimizer |
| Labels | `infrastructure` |
| Depends on | OPT-1, MIGRATE-1 |
| Touches | `crates/logicaffeine_compile/src/optimize/egraph/` (its generic core lifted to a shared `crate::egraph`), `tensor_graph::egraph` |
| Status | open |

**Goal.** Algebraic rewrites live in one e-graph over pure tensor terms, extracted by each shim's cost:
- latent attention absorbed ⇄ expanded (matmul reassociation; today's `latent_expanded` chip rule moves here);
- merged projections as one matmul by concatenated weights plus split views;
- scale and norm-weight folds;
- transpose elimination;
- contraction recognition.

In-place state becomes versioned terms; multi-output ops use projections.

**Read first.**
- `crates/logicaffeine_compile/src/optimize/egraph/mod.rs`;
- `crates/logicaffeine_compile/src/optimize/egraph/extract.rs`.

**RED tests.**
- [ ] `tensor_egraph_lock::the_egraph_extracts_the_greedy_plan_on_the_corpus`: phase-ordered saturation, one phase per
  rule in today's order with extraction after each, reproduces the rule engine's graph exactly.
- [ ] `tensor_egraph_lock::every_tensor_rewrite_carries_a_certificate`
- [ ] `tensor_egraph_lock::extraction_follows_each_shims_cost`: two shims with opposite costs extract opposite forms of
  latent attention.
- [ ] The imperative optimizer's suites stay byte-identical after the generic core is lifted.

**Edge cases.**
- [ ] Saturation node budgets on the 27B graph (about 1,600 ops). Report time and nodes; fail past the budget.
- [ ] A float reassociation the shim's numerics do not allow: it is not extracted.

**Success.**
- [ ] All green; compile time on the 27B graph recorded (receipt: log path and timing).

**Done when.** Rewrites are found by search and chosen by cost, and each one is certified.

#### OPT-5 · Every rule proven

| Field | Value |
|---|---|
| Phase | optimizer |
| Labels | `infrastructure` |
| Depends on | OPT-1 |
| Touches | a new `tv::tensor` module in `crates/logicaffeine_tv` (the verification feature); `tensor_graph::smt`, the meaning it discharges, printed without Z3 |
| Coverage | 12 rows in section 11 |
| Status | done |

**Goal.**
- Each grouping and placement rule is proven over arrays: tensors as arrays, ops as uninterpreted functions, in-place
  writes as row-range stores.
- Each algebraic rewrite is proven at bounded shapes.
- Witnesses are checked in and re-checked cheaply, following the `logicaffeine_synth` workflow.

**RED tests.**
- [x] `tensor_rule_proof_lock::every_grouping_rule_is_proven_over_arrays`: every step of the table and the lane's own,
  on every witness of `fusion_class_lock`, every hold-back witness and its twin, and every lane cut
- [x] `tensor_rule_proof_lock::every_algebraic_rewrite_is_proven_at_bounded_shapes`: projections by weights side by side,
  a reassociated product, a scale through a product, at every shape up to 3
- [x] `tensor_rule_proof_lock::every_rule_has_a_checked_in_witness`: `fixtures/tensor_rule_proofs.tsv`, re-checked without
  Z3 in the default suite

The proofs need the verification feature and Z3; the pinned digests re-check without them (RED receipt:
logs/one-ir/opt5-1027.log).

**Edge cases.**
- [x] A rule whose legality depends on a range fact. The proof assumes the fact and cites it
  (`a_range_fact_is_assumed_and_cited`; proven with it and refuted without).
- [x] An in-place write overlapping a read (a slice moved past a write to its source is refuted; every hold-back
  witness is proven as the table leaves it).

**Success.**
- [x] All three green under `--features verification` (receipt: logs/one-ir/opt5-run2-1027.log).

**Verify.**
```
Z3_SYS_Z3_HEADER=/usr/include/z3.h cargo nextest run -p logicaffeine-tests --features verification --test tensor_rule_proof_lock
```

**Mutants killed** (receipt: logs/one-ir/mutants-opt5b-1027.log): a join's second half stored at the first's offset, a head slice's offset
ignored, a landing that ignores its units, the rewritten graph's reads not owed, the range facts dropped, every term
of a shape one class, proven pairs not joined, vacuous premises accepted, `slice_read` moving past a write, the chip
rules dropped from the steps.

**Done when.** No rule is trusted without a proof.

#### OPT-6 · Numerics contracts

| Field | Value |
|---|---|
| Phase | optimizer |
| Labels | `infrastructure` `area:quant` |
| Depends on | OPT-2 |
| Touches | `tensor_graph::numerics`, `tensor_graph::facts` (range, accumulator), every shim's `numerics()` |
| Status | done |

**Goal.**
- Each knob point declares its bound: int8 one word, int8 two words, f16 attention, f32, and later fp8 and fp4.
- A quantized activation is a typed IR value carrying its scheme, mirroring vLLM's quantized-activation contract.
- Integer accumulator widths are proven from ranges.

**RED tests.**
- [x] `numerics_contract_lock::every_knob_point_declares_its_bound`: every shim, the D5005 engine at each of its real
  knob points; every scheme's and storage format's bound held to the reference's quantizers (the engine's int8 cache
  rows through its own `cache_row`) and near tight (receipt: logs/one-ir/opt6-commit-check-1027.log; RED: logs/one-ir/opt6-red-1027.log; mutants
  killed: logs/one-ir/mutants-opt6-1027.log)
- [x] `numerics_contract_lock::every_chip_fused_equals_apart_within_its_declared_bound`: the meaning under every shim;
  the D5005 machine with every vector job taken apart into its parts' jobs, and its merged projections row by row, at
  every real point in Q8_0 and f32; the spatial chips' joined sections against the program stored, bit for bit where
  the program claims exactness; the GPU chips held on the device by `cuda_lane_lock`'s two apart tests and
  `cuda_fusion_census_lock::every_forward_fusion_computes_what_its_kernels_compute_apart` (receipt:
  logs/one-ir/opt6d-commit-check3-1027.log; GPU: logs/one-ir/gpu-opt6-forward2-1027.log). It found the forward's
  runs contracted into a fused multiply-add (logs/one-ir/gpu-opt6-forward-1027.log) and `fuse_generic` splicing one of
  a rotation's three reads of the norm before it (logs/one-ir/fuse-generic-red-1027.log); both fixed.
- [x] `numerics_contract_lock::a_quantized_activation_names_its_scheme_and_its_consumer_reads_it`: on every lane cut
  (receipt: logs/one-ir/opt6-commit-check-1027.log)
- [x] `numerics_contract_lock::every_integer_accumulator_is_proven_wide_enough`: `facts::accumulator`, judged by every
  shim's `fits` (RED: logs/one-ir/opt6-acc-red-1027.log; green: logs/one-ir/opt6-acc-green2-1027.log)

**Edge cases.**
- [x] The two-word lane's exact attention form: every word product taken, or the bound cannot be pinned. Declared:
  `Contract::attention` says how each attention a chip narrows takes its operands — the lane's f16 tensor-core attention,
  its two-word one (queries, keys and values in two words a row, weights in three, every product of the words taken),
  the D5005 array's — each scheme held to the chip's own quantizer, and the lane's two cite their device tests
  (`numerics_contract_lock::the_lanes_narrowed_attentions_are_held_on_the_device`; receipt:
  logs/one-ir/opt6e-commit-check-1027.log, mutants: logs/one-ir/mutants-opt6e-1027.log; on the device:
  `cuda_lane_lock::the_two_word_attention_computes_what_the_attention_computes` and the f16 one, green on 7d453641b,
  logs/one-ir/gpu-opt6-spatial-attention-1027.log).
- [x] A view that reads a quantized form in place only at whole blocks (a product's quantized read is traced through
  views at whole blocks to the tensor quantized).

**Success.**
- [x] All four green (receipt: logs/one-ir/opt6d-commit-check3-1027.log).

**Done when.** Every approximation a chip makes is declared and held.

#### OPT-7 · A measured cost model and tuning database

| Field | Value |
|---|---|
| Phase | optimizer |
| Labels | `infrastructure` `cuda` |
| Depends on | OPT-2, SETUP-4 |
| Touches | `LaneShim::cost`, a checked-in database under `benchmarks/results/`, the closed-form choices in `crates/logicaffeine_compile/src/codegen_cuda/lane/kernels.rs` |
| Owner notes | the lane's kernels belong to the performance lane |
| Coverage | 3 rows in section 11 |
| Status | open |

**Goal.** Every closed-form shape choice the lane makes today becomes a measured database entry or a named default.

The choices: `rows_a_warp`, `attn_heads_per_block`, `attn_splits`, `attn_flash_queries`, `gemm_blk_rows`,
`gemm_blk_expert_rows`, `gemm_blk_tail`, `gemm_f32_splits`, `gemv_f32_parts`, `axpy_staged`, `moe_down_rows`,
`route_warps`, `argmax_blocks`, `norm_threads`, `attn_shared_splits`, `delta_chunks_take`.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's 338 MoE tables, 245 block-fp8 tables and 34 Mamba tables, keyed by shape and device (`def get_moe_configs(` in
  section 10.2).
- Triton's deterministic heuristic tables in `triton_kernels` (`opt_flags`).

**Beyond.**
- The database is committed and measured offline with the SETUP-4 method.
- The cost model reads it, so every decision is reproducible and explainable.
- Triton measures at run time and caches nothing by default; vLLM ships tables for datacenter parts only.

**RED tests.**
- [ ] `tuning_db_lock::every_closed_form_choice_is_a_measured_entry_or_a_named_default`
- [ ] `tuning_db_lock::a_db_entry_records_its_measurement_and_boot`
- [ ] `tuning_db_lock::the_lane_reads_its_choices_from_the_db`

**Edge cases.**
- [ ] A shape missing from the database falls back to the named default and says so in the fusion report.
- [ ] A chip with no measurements (sm_100 before a rented GPU): defaults only.

**Success.**
- [ ] All three green.
- [ ] The scoreboard rerun shows no row slower (receipt: ledger).

**Done when.** No shape decision is a constant in code.

#### OPT-8 · Per-chip ledger and provenance

| Field | Value |
|---|---|
| Phase | optimizer |
| Labels | `infrastructure` |
| Depends on | MIGRATE-1, MIGRATE-2, MIGRATE-3, MIGRATE-4, LEDGER-2 |
| Touches | `benchmarks/fusion_parity.json`, `fusion_parity_lock`, every backend's artifact header, `bringup_lock` |
| Owner notes | artifacts gain their shim and rule set: a deliberate byte change, re-pinned in `one_ir_identity_lock` with Tristen's approval |
| Status | open |

**Goal.**
- Every compiled claim is per chip, through that chip's link.
- Every artifact names its shim and rule set.

**RED tests.**
- [ ] `fusion_parity_lock::every_class_id_is_a_registered_symbol`
- [ ] `fusion_parity_lock::every_chip_claim_compiles_through_that_chips_link`
- [ ] `bringup_lock::every_backend_optimizes_the_one_lifted_graph`

**Success.**
- [ ] All green.
- [ ] `one_ir_identity_lock` re-pinned once, with approval (receipt: log path and the approval).

**Done when.** The ledger says, per chip, what is compiled, and every artifact says how it was optimized.

### 12.8 Migration

#### MIGRATE-1 · The CUDA lane links the shared table

| Field | Value |
|---|---|
| Phase | migrate |
| Labels | `infrastructure` `cuda` |
| Depends on | OPT-2 |
| Touches | `crates/logicaffeine_compile/src/codegen_cuda/lane/plan.rs` (the plan and the schedule), the link `codegen_cuda/lane_link.rs`, `lane/graph.rs` (a view of the lifted IR) |
| Owner notes | the lane belongs to the performance lane: a freeze window, digests recorded at its start, a GPU window for the values run |
| Status | claimed |

**Goal.**
- `plan.rs` becomes: optimize under `LaneShim`, then link (the op-to-launch arms plus one builder per fused class), then
  schedule, then buffers.
- `RULE_TABLE` and the rule functions are deleted in the same edit.
- `rows_a_warp` stays in the lane's schedule, holdable through `LaneShim.held`. The schedule (four rows a warp, the
  shared-row attention, the split counts, a batch's sequences) stays in `plan.rs` beside the plan, so the shrink-only
  allowlist of `one_ir_lock` names no new file.

**RED tests.**
- [x] `one_ir_lock::the_lane_derives_no_fusion`: no fusion rewrite in `codegen_cuda/lane`; its rewriters are the
  schedule's (receipt: logs/one-ir/migrate1-check-1028.log).
- [x] The `lane_compile_lock` imports (`RULES`, the plan helper) are ported: both keep their names and signatures, so
  the lock imports what it imported (receipt: logs/one-ir/migrate1-check-1028.log).

**Edge cases.**
- [ ] The paired gate-and-up on the tensor cores (G=5) and the paired expert GEMM (G=3) keep their bit-exact locks.
- [x] The branched streams still wait on every launch they depend on
  (`lane_compile_lock::every_branched_launch_follows_every_launch_it_depends_on`) (receipt: logs/one-ir/migrate1-check-1028.log).

**Success.**
- [x] `one_ir_identity_lock` unchanged, and `one_ir_lane_identity_lock`'s 2700 lane pins byte-identical (receipt:
  logs/one-ir/migrate1-check-1028.log). Real checkpoints' lanes (Q8_0, IQ4_XS, Q4_K, MXFP4; 1B to 27B) and the wide
  and medium block-format models: every program, sidecar, launch, op list, buffer and arena identical before and after;
  the firing records differ only where `rows_a_warp` fires, its records now after the link, where the schedule takes
  them (receipt: logs/one-ir/migrate1-real-lanes-1028.log).
- [x] `lane_compile_lock` green (receipt: logs/one-ir/migrate1-check-1028.log).
- [ ] `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart` green in a GPU window (receipt: log path).
- [ ] `cuda_lane_lock` green in full (receipt: log path).
- [x] `fusion_parity_lock` green; its census check accepts censuses through the shared optimizer (receipt: logs/one-ir/migrate1-check-1028.log).

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test one_ir_identity_lock --test one_ir_lock --test lane_compile_lock --test fusion_parity_lock
cargo nextest run -p logicaffeine-tests --test cuda_lane_lock
```

**Done when.** The lane fuses nothing on its own, and everything it emits is byte-identical.

#### MIGRATE-2 · The older CUDA forward on the IR

| Field | Value |
|---|---|
| Phase | migrate |
| Labels | `infrastructure` `cuda` |
| Depends on | OPT-2 |
| Touches | `crates/logicaffeine_compile/src/codegen_cuda/qwen_emit.rs`, `crates/logicaffeine_compile/src/compile.rs` (`compile_to_cuda_forward*`) |
| Status | done |

**Goal.**
- `emit_forward` walks the optimized graph under `ForwardShim`.
- `Stage`, `cuda_stage` and `fuse_elementwise_chains` are deleted; `cuda_stage`'s refusals become `forward_shim::kernel_refusal`, word for word.
- The MoE trio becomes its link of `Prim(Moe)`.

**RED tests.**
- [x] `one_ir_lock::no_backend_keeps_a_private_op_enum`: no `Stage` enum.
- [x] The `recognizer_migration_lock` lens is ported. The golden `crates/logicaffeine_tests/tests/fixtures/cuda_stages_golden.txt`
  must not change.

**Edge cases.**
- [x] The elementwise chain that must keep an intermediate a later op reads
  (`cuda_elementwise_fusion_lock::an_intermediate_needed_later_is_materialized_not_fused_away`): red from b1ec1e6aa, where
  the shared `add_norm` ran before the forward's chains; green once `ForwardShim` runs its chains first and adds into a
  norm only at the next stage (receipt: logs/one-ir/gpu-opt6-forward-1027.log).

**Success.**
- [x] Forward digests unchanged: the 70 pins recorded before the move (receipt: logs/one-ir/migrate2-pre-1027.log).
- [x] `cuda_fusion_census_lock`, every `cuda_forward_*` lock, `cuda_elementwise_fusion_lock` and
  `recognizer_migration_lock` green (receipt: logs/one-ir/gpu-opt6-forward3-1027.log, logs/one-ir/gpu-opt6-forward2-1027.log,
  logs/one-ir/opt6d-commit-check3-1027.log).

**Done when.** The older forward reads the one IR and its fusions come from the shared table.

#### MIGRATE-3 · The spatial pipeline on the IR

| Field | Value |
|---|---|
| Phase | migrate |
| Labels | `infrastructure` `spatial` |
| Depends on | OPT-2 |
| Touches | `crates/logicaffeine_compile/src/spatial/pipeline.rs`, `crates/logicaffeine_compile/src/spatial/passes/` |
| Status | done |

**Goal.**
- `lower_program_stored` iterates the optimized graph's ops.
- `lower_section` reads tensor facts.
- `TensorOp` and `tensor_op_of` are deleted.
- The spatial-only decisions (`dense_tables`, `agree_layouts`, q8 view names) become a link prelude.
- `forward`'s legality reads the shared facts.

**RED tests.**
- [x] The `one_ir_lock::no_backend_keeps_a_private_op_enum` census is extended to `TensorOp` (RED receipt: logs/one-ir/migrate3-red-1027.log).
- [x] `one_ir_lock::every_backend_lowers_the_tensor_graph`: only `tensor_graph` calls the front end's walk. The SN50
  Qwen3.8 path stays on the allowlist until its writer moves to `model::author`.

**Edge cases.**
- [x] The FPGA decode programs, whose fabric RTL is printed from the program.
- [x] Tensor-parallel manifests, which recognize all-reduce as a chain of additions.

**Success.**
- [x] Spatial digests unchanged (receipt: logs/one-ir/migrate3-ir1-verify-1027.log).
- [x] `spatial_backend_totality_lock` and the spatial suites green (receipt: logs/one-ir/migrate3-ir1-verify-1027.log).
- [x] `cuda_spatial_general_lock::the_decode_step_runs_on_the_gpu_at_every_position` green in a GPU window (receipt:
  logs/one-ir/gpu-migrate23-opt6-1027.log).

**Done when.** Every spatial backend lowers the one IR.

#### MIGRATE-4 · The D5005 engine on the IR

| Field | Value |
|---|---|
| Phase | migrate |
| Labels | `infrastructure` `d5005` |
| Depends on | OPT-2 |
| Touches | `crates/logicaffeine_compile/src/spatial/decode_engine/lower_graph.rs`, `capability.rs` |
| Owner notes | d5005-model-foundry's files: their hands, or explicitly co-signed on the lanes board |
| Status | done |

**Goal.** `lower_graph` reads fused symbols instead of matching blocks:

| Symbol | D5005 command |
|---|---|
| `merged_qkv` | one Matvec, under the engine shim's layout condition |
| `conv_bias_silu` | Conv |
| `gdn_gating` | Gates |
| `glu_act_mul` (sigmoid) | SigmoidGate |
| `qk_norm_rope` | RmsHeads plus Rotate, unless the shim allows it whole |
| `kv_cache_quant` | the Hadamard int8 cache, a numerics point the engine shim refuses as a fusion |

The capability module becomes the engine shim.

**RED tests.**
- [x] `d5005_lower_graph_lock::the_engine_reads_fused_symbols`
- [x] `d5005_lower_graph_lock::capability_is_the_engine_shim`

**Success.**
- [x] Engine digests unchanged (receipt: logs/one-ir/migrate4-verify-1027.log).
- [x] The hand lowering still equal (`d5005_lower_graph_lock::the_program_read_off_the_step_graph_is_the_hand_lowering_at_every_knob_point`).
- [x] `d5005_capability_lock` green (receipt: logs/one-ir/migrate4-verify-1027.log).

**Done when.** The engine's commands are links of shared symbols.

### 12.9 Parity, tier 1: the demo models on the 3060

Every tier-1 ticket has three layers of work:
1. **The shared side:** the rule, its definition, its proof and its `fusion_class_lock` case. It can start after OPT-1.
2. **The CUDA link:** the kernel, the census and the values test. It needs MIGRATE-1.
3. **Other chips:** each chip's link and shim answer, where the shim allows.

A ticket is done when the CUDA link lands and every other chip answers, either `Native` with a link or `Decompose` and
refuses nothing.

#### PARITY-SOFTCAP · Soft-capped logits and attention scores

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `spatial` `d5005` `area:attention` `model:gemma2` |
| Depends on | OPT-1, MIGRATE-1, SETUP-4 |
| Touches | `crates/logicaffeine_language/src/parser/mod.rs` (new sugar); `tensor_graph::front` (a new op); `crates/logicaffeine_compile/src/model/author.rs` (its soft-cap refusal); `crates/logicaffeine_compile/src/model/spec.rs` (Gemma-2's caps); the lane's attention kernels and head |
| Owner notes | the lane belongs to the performance lane. The differential race's run A wrote Gemma-2 support in its own copy (`/mnt/build/race-a`); whether any of it lands is Tristen's call |
| Coverage | 2 rows in section 11 |
| Status | open |

**Goal.** Gemma-2 compiles through the one program on every chip. It caps its final logits at 30 and its attention scores
at 50 (cap · tanh(x / cap)). Today `model::author::check` refuses both by name.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- Logits: vLLM's soft-cap kernel splits the vocabulary row across blocks at batch 1
  (`external/competitors/vllm/csrc/muse_spark_mul_tanh_soft_cap_kernel.cu` · `muse_spark_mul_tanh_soft_cap_kernel(`).
- Attention: llama.cpp and FlashInfer apply the cap inside the attention kernel (the ledger's
  `attention_variants_inline` citations).
- The number to match: llama.cpp's Gemma-2-2B Q8_0 decode and pp512 on the 3060, and vLLM's Gemma-2-2B at matched
  precision. Both come from SETUP-4 and SETUP-5.

**Beyond.**
- A cap is strictly increasing, so arg-max of the capped logits equals arg-max of the raw logits. Under greedy decoding
  the shared optimizer drops the final cap entirely, by a proven rewrite (OPT-5). Under sampling, the cap folds into the
  sampler's read of the logits.
- The attention cap folds into the online softmax, with exp2 and the scale folded in (TECH-EXP2-SCALE-FOLD).
- Target: Gemma-2-2B decode ≥ 1.10× llama.cpp, and ≥ vLLM at matched precision, on the same boot.

**Read first.**
- `crates/logicaffeine_compile/src/model/author.rs` (the refusal);
- `crates/logicaffeine_compile/src/model/reference.rs` (the reference already computes both caps);
- `cuda_lane_lock::gemma3_1b_meets_every_activation_precisions_bar` (the bar pattern);
- `scripts/accel/model-golden.sh`.

**RED tests.**
- [ ] `tensor_softcap_lock::a_soft_cap_desugars_to_the_cap_op`: the parser does not know the sugar today.
- [ ] `tensor_softcap_lock::the_vm_computes_cap_times_tanh_of_x_over_cap`
- [ ] `tensor_softcap_lock::the_author_writes_gemma2s_caps`: fails today on the author's refusal.
- [ ] `fusion_class_lock::a_cap_before_an_argmax_is_dropped_and_the_argmax_is_unchanged`
- [ ] `fusion_class_lock::an_attention_cap_folds_into_the_attention`
- [ ] `lane_compile_lock::a_capped_attention_caps_inside_its_kernel`
- [ ] `cuda_lane_lock::a_capped_attention_computes_what_its_reference_computes`
- [ ] `model_golden_lock::gemma2_2b_matches_its_llama_cpp_goldens`: needs goldens under `benchmarks/models/`, made by
  `scripts/accel/model-golden.sh`.
- [ ] `cuda_lane_lock::gemma2_2b_meets_every_activation_precisions_bar`

**Edge cases.**
- [ ] Logits of ±inf and NaN: tanh saturates; NaN stays NaN.
- [ ] A cap on a windowed layer. Gemma-2 alternates local and global layers.
- [ ] The cap on every attention path: the split decode kernel, the tiled kernel, flash f32, f16 tensor cores, and the
  two-word int8 path.
- [ ] Sampling with temperature keeps the cap; greedy drops it.
- [ ] The D5005 engine refuses soft caps by name today (`Refusal::LogitSoftcap`). Its shim answers `Decompose` for the
  final cap, or keeps the refusal, and says which.

**Implementation.**
- [ ] Sugar: `X soft-capped at C`, and an attention clause `with scores soft-capped at C`, desugaring to loops the VM
  runs.
- [ ] A new `TensorOpSpec` variant and a field on the attention spec.
- [ ] The spec reads Gemma-2's caps from the GGUF metadata.
- [ ] The shared rules: drop a monotone cap before an arg-max; fold a cap into attention.
- [ ] The CUDA links in every attention kernel. The final cap goes into the sampler when sampling.

**Success.**
- [ ] Every RED test above green (receipt: log paths).
- [ ] Gemma-2-2B golden agreement on par with llama.cpp's shipped walk.
- [ ] Copy: decode and pp512 ≥ llama.cpp on one boot (receipt: scoreboard row).
- [ ] Beyond: decode ≥ 1.10× llama.cpp and ≥ vLLM (receipt: scoreboard row).

**Mutants to kill.**
- [ ] The cap dropped under sampling.
- [ ] The cap applied after the softmax.
- [ ] tanh replaced by a clamp.
- [ ] The cap value read from the wrong metadata key.

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test tensor_softcap_lock --test fusion_class_lock --test lane_compile_lock
cargo nextest run -p logicaffeine-tests --test cuda_lane_lock --test model_golden_lock --cargo-profile test-opt
```

**Done when.** Gemma-2 runs through the one program at or past both competitors, with its caps proven where they are
dropped.

#### PARITY-NORM-THEN-ADD · A norm then a residual add

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `area:norm` `model:gemma3` `model:gemma2` |
| Depends on | OPT-1, MIGRATE-1 |
| Touches | `tensor_graph::fuse`, the lane's RMSNorm kernel |
| Owner notes | the performance lane was testing sandwich-norm fusion in its own copy on 2026-10-25. Read the lanes board first: if that work lands as a lane rule, this ticket carries it into the shared table rather than writing a second rule |
| Coverage | 1 row in section 11 |
| Status | open |

**Goal.**
- Gemma's sandwich block writes `x + post_norm(attention(pre_norm(x)))`. The post-norm, its weight and the residual add
  become one row kernel.
- With `add_norm` already fusing the add into the next pre-norm, the whole chain post-norm → add → next norm → quantize
  becomes one launch.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- llama.cpp fuses an RMSNorm with the following add (the ledger's `norm_then_add` citation).
- The number: Gemma-3-1B and Gemma-2-2B decode against llama.cpp (SETUP-4 row for the norm kernels).

**Beyond.** One launch for post-norm, residual add, next pre-norm and the next projection's activation quantize.
llama.cpp stops after the add. Target: the norm launches per Gemma layer drop from 4 to 2.

**RED tests.**
- [x] `fusion_class_lock::a_norm_feeding_an_add_fuses_into_one_row_op` (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log)
- [ ] `lane_compile_lock::a_sandwich_post_norm_adds_its_residual_in_its_row_kernel`
- [ ] `lane_compile_lock::a_sandwich_layer_launches_two_norm_kernels`
- [ ] Values: `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart` covers the new rule (the Gemma tiny
  architecture is in its corpus).

**Edge cases.**
- [x] The post-norm's output read by a second op: keep it, where the op comes after the add; one before it holds the fold back (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log).
- [x] A norm with no weight (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log).
- [x] The add's operands in either order (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log).
- [x] The sum written back only when a later op reads it (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log).
- [x] The next norm carrying the quantize epilogue under each numerics: one word and two (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log).

**Success.**
- [ ] All RED green (receipt).
- [ ] Gemma-3-1B decode not slower and launches per step reduced (receipt: scoreboard and profile).

**Mutants to kill.**
- [ ] The residual added before the norm.
- [ ] The sum not written back when read later.

**Done when.** A sandwich layer's norms cost two launches.

#### PARITY-ATTENTION-VARIANTS-INLINE · Caps and masks inside attention

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `primitive` `cuda` `area:attention` `model:gemma2` |
| Depends on | PARITY-SOFTCAP |
| Touches | the language's attention clauses, the attention spec, every lane attention kernel |
| Coverage | 4 rows in section 11 |
| Status | open |

**Goal.** Every attention kernel takes:
- a soft cap;
- per-sequence causality;
- bidirectional spans (prefix-LM);
- chunked-local lookback;
- ALiBi slopes.

Windows and sinks already exist. The ledger row moves from partial to compiled.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's unified attention carries ALiBi, a query-query bias, soft cap, sinks, sliding window, chunked-local lookback
  and prefix spans in one kernel
  (`external/competitors/vllm/vllm/v1/attention/ops/triton_unified_attention.py` · `def unified_attention(`).
- Triton's tutorial splits causal attention into three stages, so off-diagonal tiles skip masking.

**Beyond.**
- A mask the compiler knows statically decides at compile time which tiles are full, partial or empty. Empty tiles are
  never launched, and full tiles carry no mask arithmetic.
- Target: causal prefill attention at 512 rows ≥ 1.1× vLLM's unified attention at our shapes.

**RED tests.**
- [ ] `tensor_attention_masks_lock::every_mask_clause_desugars_and_the_vm_runs_it`
- [ ] `lane_compile_lock::every_attention_kernel_takes_a_cap_and_a_mask`
- [ ] `lane_compile_lock::a_statically_empty_tile_is_never_launched`
- [ ] `cuda_lane_lock::a_masked_attention_computes_what_its_reference_computes`

**Edge cases.**
- [ ] A row with every position masked: no NaN; the output is zero by definition.
- [ ] Mask with window with sink.
- [ ] A chunk boundary inside a tile.
- [ ] ALiBi with a window.
- [ ] Decode (one query) and prefill (many) in each mask.

**Success.**
- [ ] All RED green.
- [ ] Ledger row `attention_variants_inline` compiled (receipt).
- [ ] A/B against vLLM recorded (receipt: kernel A/B row).

**Mutants to kill.**
- [ ] A tile misclassified as full.
- [ ] The ALiBi slope applied with the wrong sign.

**Done when.** No attention variant any model needs is refused.

#### PARITY-EMBED-NORM · The embedding gather and the first norm

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `spatial` `area:norm` |
| Depends on | OPT-1, MIGRATE-1 |
| Touches | `tensor_graph::fuse`, the lane's gather kernels |
| Coverage | 8 rows in section 11 |
| Status | linked on the CUDA lane (K-02); its profile owed |

**Goal.** One launch per token row does four things:
- gathers the embedding row (any block format);
- applies the embedding scale (Gemma's √d);
- writes the raw row for the residual;
- writes the first layer's RMSNorm of it, with that norm's quantize epilogue.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's Triton kernel fuses the gather and the first RMSNorm
  (`external/competitors/vllm/vllm/model_executor/layers/fused_embed_norm.py` · `def fused_embed_norm(`).
- The number: the SETUP-4 row for gather plus norm at each model's width.

**Beyond.** The quantize epilogue for the first projection is in the same launch, which vLLM's does not fold. Prefill
chunks gather all rows in one launch.

**RED tests.**
- [x] `fusion_class_lock::a_gather_feeding_a_norm_fuses_into_one_op` (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log)
- [x] `lane_compile_lock::the_embedding_gather_norms_its_row_in_the_same_launch` (receipt: logs/one-ir/live-green3-1029.log)
- [x] Values in `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart` (receipt: logs/one-ir/live-gpu-1030.log).

**Edge cases.**
- [x] The same token twice in a chunk (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log).
- [x] An embedding in a block format (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log).
- [x] A tied embedding whose table the head also reads (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log).
- [x] The raw row needed as the residual. Both outputs are written (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log).
- [x] A model with no embedding scale (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log).

**Success.**
- [x] All RED green.
- [x] One launch fewer per step on every model (receipt: `tests/fixtures/lane_census/`).
- [ ] The same in a profile.

**Mutants to kill.**
- [ ] The residual given the normed row.
- [ ] The scale applied after the norm.

**Done when.** A step's first launch is gather, scale, residual, norm and quantize together.

#### PARITY-QK-NORM-ROPE-GATE-SPLIT · Gated attention's prologue in one launch

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `area:attention` `model:qwen35` |
| Depends on | OPT-1, MIGRATE-1 |
| Touches | `tensor_graph::fuse`, the lane's rope kernel |
| Coverage | 3 rows in section 11 |
| Status | open |

**Goal.** Qwen3.5 and Qwen3.8 project the query and its output gate together. One launch does:
- split the projection;
- RMS-normalize the query and key heads;
- rotate their partial NeoX dimensions with the interleaved sections;
- write the key into its cache row.

The gate is never copied: its consumer reads it in place through a strided read.

**First step.** Count today's launches between the query projection and the attention on the real Qwen3.5-0.8B program.
`head_norm_rope`, `slice_read` and `row_write` may already cover part of this. Record what is fused and what is not
before writing code.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM fuses split, Gemma RMSNorm of q and k, partial rotation and the gate copy in one Triton launch
  (`external/competitors/vllm/vllm/model_executor/layers/fused_qk_norm_rope.py` · `def fused_qk_rmsnorm_rope_gate(`).
- The number: the SETUP-4 row at Qwen3.5 shapes.

**Beyond.**
- No gate copy at all: a strided read.
- Query and key in one launch rather than two.
- Target: one launch between the projection and the attention, on every Qwen3.5 attention layer.

**RED tests.**
- [ ] `lane_compile_lock::the_gated_attentions_prologue_is_one_launch`
- [ ] `lane_compile_lock::the_output_gate_is_read_in_place`
- [ ] Values in `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`, plus
  `cuda_lane_lock::qwen3_5_0_8b_meets_every_activation_precisions_bar`.

**Edge cases.**
- [ ] Unequal query and key head counts.
- [ ] The interleaved rotary sections.
- [ ] The rotary dimensions shorter than the head.
- [ ] Prefill chunks writing many cache rows.

**Success.**
- [ ] All RED green.
- [ ] Qwen3.5-0.8B decode not slower (receipt: scoreboard).

**Done when.** Gated attention's prologue is one launch with no copies.

#### PARITY-GDN-DECODE-WHOLE · The whole gated-delta decode in one kernel

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `area:recurrent` `model:qwen35` |
| Depends on | OPT-1, MIGRATE-1 |
| Touches | `tensor_graph::fuse`; the lane's `lk_gates`, `lk_delta_step`, `lk_delta_norm` and `lk_conv` |
| Coverage | 5 rows in section 11 |
| Status | open |

**Goal.** One decode-step launch per GDN layer, between its input and output projections. It does:
- the causal conv with SiLU;
- the gates, g = −exp(A)·softplus(a + dt) and β = σ(b);
- the q/k L2 norms;
- the delta-rule update of the state;
- the gated RMSNorm output with its quantize epilogue.

The multi-token form for MTP verification follows in SERVE-SPEC-DECODE.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's CUDA kernel does gates, L2 norms, delta update, MTP and gated norm in one launch, staging the state 32 value
  rows at a time with `cp.async`
  (`external/competitors/vllm/csrc/gdn/fused_gdn_decode_kernel.cu` · `gdn_decode_post_conv_mtp_kernel(`).
- vLLM's packed Triton decode reads q, k and v straight from the conv output
  (`external/competitors/vllm/vllm/third_party/flash_linear_attention/ops/fused_recurrent.py` · `def fused_recurrent_gated_delta_rule_packed_decode(`).
- The number: SETUP-4 rows at Qwen3.5-0.8B and Qwen3.8 layer shapes.

**Beyond.**
- The conv ring is folded into the same kernel too. The whole mixer between the projections is one launch, with the
  conv state and the delta state each read and written once.
- Target: the GDN layer's decode time within 1.25× of the time to read and write its state at measured bandwidth.

**RED tests.**
- [x] `fusion_class_lock::a_delta_step_with_its_conv_gates_norms_and_output_fuses_into_one_op`: the step between the
  projections as one op (`gdn_decode_whole`), a decode step's and each sequence's of a batch (shared side; receipt:
  logs/one-ir/gdn-commit-check-1028.log)
- [ ] `lane_compile_lock::the_gated_delta_decode_is_one_launch_between_its_projections`
- [ ] `cuda_lane_lock::the_whole_delta_decode_computes_what_its_launches_compute_apart`: bit for bit; keep the
  per-element arithmetic order of today's kernels.
- [ ] `cuda_lane_lock::qwen3_5_0_8b_meets_every_activation_precisions_bar` stays green.

**Edge cases.**
- [x] More value heads than key heads (shared side; receipt: logs/one-ir/gdn-commit-check-1028.log).
- [x] State widths 64, 128 and 256 (shared side; receipt: logs/one-ir/gdn-commit-check-1028.log).
- [x] The first step (zero state) (shared side; receipt: logs/one-ir/gdn-commit-check-1028.log).
- [x] A long run of steps (state magnitude): 64 steps, the memory and rings carried (shared side; receipt: logs/one-ir/gdn-commit-check-1028.log).
- [x] The quantize epilogue under every numerics: one word and two, joined by `fused_quant` (shared side; receipt: logs/one-ir/gdn-commit-check-1028.log).
- [ ] Registers: no spill (`lane_compile_lock::every_row_reading_kernel_keeps_its_decode_in_registers` pattern).

**Success.**
- [ ] All RED green.
- [ ] Copy: ≥ vLLM's packed decode at our shapes (receipt: A/B row).
- [ ] Beyond: within 1.25× of state bandwidth (receipt: profile).

**Mutants to kill.**
- [ ] β applied before the decay.
- [ ] The conv ring advanced twice.
- [ ] The L2 norm skipped on k.

**Done when.** A Qwen3.5 GDN layer decodes in one launch at the state's bandwidth.

#### PARITY-GDN-POST-CONV-PREP · Everything after the conv in one prefill kernel

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `area:recurrent` `model:qwen35` |
| Depends on | OPT-1, MIGRATE-1 |
| Touches | `tensor_graph::fuse`; the lane's `lk_conv_rows`, `lk_gates` and `lk_delta_prep` |
| Coverage | 4 rows in section 11 |
| Status | open |

**Goal.** For a wide chunk, one launch does:
- the conv over the chunk's rows with its carry;
- SiLU;
- the q/k L2 norms;
- the gates.

It writes q, k, v, g and β laid out for the chunked delta rule.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM replaces split, rearrange, three contiguous copies, two L2 norms and the gating with one kernel
  (`external/competitors/vllm/vllm/third_party/flash_linear_attention/ops/fused_gdn_prefill_post_conv.py` · `def fused_post_conv_prep(`).
- The number: SETUP-4 at a 512-row chunk.

**Beyond.**
- The conv itself is in the same launch.
- The outputs land in the transposed layouts `lk_delta_prep` reads, so the prep kernel loses its own loads.
- Target: Qwen3.5-0.8B pp512 ≥ 1.1× today.

**RED tests.**
- [ ] `lane_compile_lock::a_wide_chunks_gated_delta_prologue_is_one_launch`
- [ ] `cuda_lane_lock::the_chunked_delta_rule_computes_what_the_delta_rule_a_row_at_a_time_computes` stays green.

**Edge cases.**
- [ ] A chunk that is not a multiple of the delta rule's chunk size.
- [ ] The conv carry from a previous chunk.
- [ ] The value-head count differing from the key-head count.

**Success.**
- [ ] All RED green.
- [ ] pp512 up (receipt: scoreboard).

**Done when.** A chunk's GDN prologue is one launch.

#### PARITY-KV-CACHE-QUANT · Quantized KV caches

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `d5005` `area:attention` `area:quant` |
| Depends on | OPT-1, OPT-6, MIGRATE-1 |
| Touches | the cache tensors' storage; every lane attention kernel; the `kv_cache` knob (unread today); the D5005 engine shim |
| Owner notes | `cuda_knob_consumers_lock` lists `kv_cache` as not yet read; moving it to honored edits that list, which is the direction its ratchet expects (`cuda_knob_consumers_lock::the_unread_knobs_never_grow`) |
| Coverage | 42 rows in section 11 |
| Status | open |

**Goal.** The KV cache is stored in one of the following schemes:
- f16;
- int8 with per-head scales;
- Hadamard-rotated int8 (the D5005 engine's form);
- fp8 e4m3, encoded in software on sm_86 and in hardware on sm_89 and later.

Each scheme is written by its producer's epilogue (the `row_write` destination) and read by every attention kernel.
`kv_cache_quant` is one shared symbol with schemes:
- CUDA links each scheme;
- the D5005 links its Hadamard int8 cache;
- the bound for each scheme is declared (OPT-6).

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM writes fp8 caches with per-head scales
  (`external/competitors/vllm/csrc/cache_kernels.cu` · `reshape_and_cache_flash_kernel(`) and decodes them inside
  attention.
- llama.cpp's default cache is f16, and q8_0 is an option.
- `model_reference_lock::an_f16_cache_keeps_each_key_and_value_as_llama_cpps_default_cache_does` already holds the
  reference's f16 form.
- The number: decode tok/s at contexts 4096 and 8192 against llama.cpp with f16 and q8_0 caches, and vLLM with fp8 (SETUP-5).

**Beyond.**
- The Hadamard int8 cache feeds the two-word int8 attention MMAs directly, with no dequantize step, under a proven bound.
- Target: decode attention at context 8192 ≤ 0.55× its time on an f16 cache.

**RED tests.**
- [ ] `fusion_class_lock::a_quantized_cache_row_reads_back_what_its_scheme_keeps`
- [ ] `lane_compile_lock::a_quantized_cache_is_written_and_read_in_its_format`
- [ ] `cuda_knob_consumers_lock::every_honored_knob_changes_what_an_emitter_prints`, with `kv_cache` honored.
- [ ] `cuda_lane_lock::every_cache_format_meets_its_bar_on_long_prompts`
- [ ] `numerics_contract_lock::every_cache_scheme_declares_its_bound`
- [ ] `d5005_lower_graph_lock::the_engines_hadamard_cache_is_the_shared_symbols_link`

**Edge cases.**
- [ ] The latent cache of latent attention (rank plus rope columns).
- [ ] Scales per head per token vs per tensor.
- [ ] The first position.
- [ ] A prefill chunk writing many rows.
- [ ] A head dimension that is not a power of two under Hadamard (MLA's 576).
- [ ] The two-word attention lane reading an int8 cache.
- [ ] A cache shared by layers of different widths.

**Success.**
- [ ] All RED green.
- [ ] Copy: each scheme's long-prompt bar met (receipt).
- [ ] Beyond: attention ≤ 0.55× f16 at context 8192 (receipt: profile).

**Mutants to kill.**
- [ ] A scale applied per tensor where per head was declared.
- [ ] The rotation applied on write but not on read.
- [ ] Rounding to zero instead of to nearest even.

**Done when.** Every model can run long contexts on a quantized cache within its declared bound, on CUDA and on the
D5005.

#### PARITY-ATTN-GQA-ROW-PACKING · A KV group's heads on the tensor cores in decode

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `primitive` `cuda` `area:attention` |
| Depends on | MIGRATE-1, SETUP-4 |
| Touches | the lane's decode attention kernels (`lk_attention`, `lk_attention_shared`) |
| Coverage | 10 rows in section 11 |
| Status | open |

**Goal.** In decode, a KV group's query heads (4 on Llama-3.2-1B and Qwen3-8B, 8 on gpt-oss) are packed onto an MMA's
M rows:
- scores and the weighted sum run on the tensor cores;
- split over the span, folded in the last block.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's unified attention sets BLOCK_M = max(16, next power of two of the group) and switches between a 2D grid and a
  3D split over the span (`external/competitors/vllm/vllm/v1/attention/ops/triton_unified_attention.py` · `def unified_attention(`).
- FlashInfer and llama.cpp use tensor cores for decode at large groups.
- The number: decode attention µs per layer at contexts 1024 and 4096 (SETUP-4).

**Beyond.**
- Under the two-word numerics the scores use int8 MMAs in the exact form.
- The split fold stays inside the launch.
- The shim's cost decides per group size and span: a group of 1 keeps the SIMT path.
- Target: ≥ 1.2× vLLM's unified attention at context 4096 on the 3060.

**RED tests.**
- [ ] `lane_compile_lock::a_grouped_decode_attention_runs_on_the_tensor_cores`
- [ ] `lane_compile_lock::every_attention_takes_the_same_kernel_with_the_rules_held_back` stays green.
- [ ] `cuda_lane_lock::the_packed_decode_attention_computes_what_the_attention_computes`

**Edge cases.**
- [ ] A group of 1.
- [ ] Groups of 6 and 12.
- [ ] A window and sinks.
- [ ] A span shorter than one tile.
- [ ] Head dimensions 64, 128 and 256.
- [ ] f16 and two-word numerics.

**Success.**
- [ ] All RED green.
- [ ] Copy and Beyond rows recorded (receipt: A/B ledger).

**Done when.** Decode attention for grouped heads runs on the tensor cores at or past the best published kernel.

#### PARITY-ATTN-LSE-MERGE · A shared merge for partial attention

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `area:attention` |
| Depends on | OPT-1, MIGRATE-1 |
| Touches | `tensor_graph::fuse`, the lane's attention folds |
| Coverage | 4 rows in section 11 |
| Status | open |

**Goal.** `attn_lse_merge` is a shared symbol: attention over spans A ∪ B equals the log-sum-exp-weighted merge of
attention over A and over B. Chunked-context attention, cascade prefixes and context parallelism can then all be
expressed. Our in-launch fold becomes this symbol's CUDA link.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM merges partial outputs by their log-sum-exp, with an fp8 output option
  (`external/competitors/vllm/csrc/attention/merge_attn_states.cu` · `merge_attn_states_kernel(`).

**Beyond.** The merge is folded into the producing launch (the last arriving block), with the output's quantize
epilogue. vLLM runs it as its own launch.

**RED tests.**
- [ ] `fusion_class_lock::attention_split_into_two_spans_merges_to_the_whole`
- [ ] `lane_compile_lock::a_chunk_attending_a_long_prefix_merges_in_its_last_block`

**Edge cases.**
- [ ] An empty partial (its LSE is −inf).
- [ ] Both partials empty.
- [ ] Sinks counted once.
- [ ] A cap applied before the merge.
- [ ] Merges summed in f32, or in f64 under the precise lane.

**Success.**
- [ ] All RED green.

**Done when.** Partial attention is a shared, proven symbol.

#### PARITY-MOE-DIRECT-PAIRS · Skip the expert sort for tiny batches

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `primitive` `cuda` `area:moe` `model:ds2` `model:oss` |
| Depends on | MIGRATE-1 |
| Touches | the lane's expert grouping (`lk_moe_group`) and its choice |
| Coverage | 2 rows in section 11 |
| Status | open |

**Goal.** When tokens × k × 4 ≤ experts, each block takes one (token, expert) pair and no sort runs.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's direct assignment (`external/competitors/vllm/vllm/model_executor/layers/fused_moe/fused_moe.py` · `def _prepare_expert_assignment(`).

**Beyond.** The crossover is a measured entry in the tuning database (OPT-7), per (rows, experts, k, format), not a
constant.

**RED tests.**
- [ ] `lane_compile_lock::a_tiny_chunks_experts_skip_the_sort`
- [ ] `cuda_lane_lock::the_direct_expert_pairs_compute_what_the_grouped_experts_compute`: bit for bit.

**Edge cases.**
- [ ] k = 1.
- [ ] The same expert chosen by two tokens.
- [ ] A shared expert.
- [ ] Expert biases (gpt-oss).

**Success.**
- [ ] All RED green.
- [ ] Prefill of 2–16 rows faster on DeepSeek-V2-Lite and gpt-oss (receipt: profile).

**Done when.** Tiny MoE batches pay no sort.

#### PARITY-MOE-GATHER-IN-OPERAND · The token gather inside the expert GEMM

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `area:moe` `model:ds2` `model:oss` |
| Depends on | OPT-1, MIGRATE-1 |
| Touches | `tensor_graph::fuse`; the lane's grouped `lk_gemm_blk` (G 1, 2, 3) and `lk_moe_gather` |
| Coverage | 13 rows in section 11 |
| Status | open |

**Goal.**
- The grouped expert GEMMs read their A rows through the sorted token index. `lk_moe_gather` and its materialized copy
  disappear.
- The down projection scatters its rows back with the routing weight on store.
- With the paired gate-and-up epilogue, an expert FFN is two launches with no gathered copy.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- Marlin's MoE GEMM gathers through `sorted_token_ids` in its operand loads
  (`external/competitors/vllm/csrc/moe/marlin_moe_wna16/marlin_template.h` · `reinterpret_cast<const int4*>(sorted_token_ids_ptr)`).
- Triton's MoE matmul gathers on load and scatters on store
  (`external/competitors/triton/python/triton_kernels/triton_kernels/matmul.py` · `def matmul(a, b, bias,`).
- The number: SETUP-4 rows for the grouped GEMMs at DeepSeek-V2-Lite and gpt-oss prefill shapes.

**Beyond.**
- Each token's int8 activation is quantized once and read through the index.
- The paired gate-and-up GLU and the scatter-with-weight run in the same two launches.
- Target: DeepSeek-V2-Lite pp512 ≥ 1.1× today.

**RED tests.**
- [ ] `fusion_class_lock::a_gather_feeding_a_grouped_gemm_folds_into_its_operand_read`
- [ ] `lane_compile_lock::a_wide_chunks_experts_read_their_rows_through_the_sort`
- [ ] `cuda_lane_lock::the_expert_gemm_reading_through_the_sort_computes_what_the_gathered_rows_compute`: bit for bit.

**Edge cases.**
- [ ] A token in k experts.
- [ ] Padding entries.
- [ ] Quantized rows read in place at whole blocks.
- [ ] An expert with no tokens.
- [ ] DeepSeek's k = 6 over 64 experts; gpt-oss's k = 4 over 32.

**Success.**
- [ ] All RED green.
- [ ] pp512 target met (receipt: scoreboard).

**Done when.** No MoE prefill materializes a gathered copy.

#### PARITY-MOE-DEFERRED-FINALIZE · The expert combine folded into the next add and norm

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `area:moe` |
| Depends on | OPT-1, MIGRATE-1 |
| Touches | `tensor_graph::fuse`; the lane's `lk_moe_combine` and `lk_rmsnorm` |
| Coverage | 4 rows in section 11 |
| Status | open |

**Goal.**
- On a wide chunk, the down GEMM writes per-(token, k) partials, and `lk_moe_combine` sums them before `add_norm`. The
  combine folds into `add_norm`, which reads the k partials, the shared expert and the residual, then writes the sum,
  the next norm and its quantize.
- In decode the down kernel already combines, so the fold is decided by cost.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM leaves the top-k reduction to the next fused boundary
  (`external/competitors/vllm/vllm/model_executor/layers/fused_moe/modular_kernel.py` · `return self.prepare_finalize.supports_deferred_moe_finalize()`).

**Beyond.** Combine, shared expert, residual add, next norm and quantize in one row kernel, under a fixed summation
order so the fold is bit-exact.

**RED tests.**
- [x] `fusion_class_lock::an_expert_combine_feeding_an_add_and_norm_fuses_into_it`: every mixture on Qwen3-MoE, gpt-oss
  and DeepSeek-V2, decode, chunk and batch (shared side; receipt: logs/one-ir/moe-commit-check-1028.log)
- [x] `lane_compile_lock::a_wide_chunks_expert_combine_folds_into_the_next_add_and_norm` (receipt: logs/one-ir/live-green-k18-2-1030.log)
- [x] `lane_compile_lock::a_decode_mixture_adds_and_norms_in_its_last_down_block` (receipt: logs/one-ir/live-green-k18-2-1030.log)
- [x] Values in `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart` and
  `cuda_lane_lock::the_mixtures_last_step_norms_as_the_norm_launch_does` (receipt: logs/one-ir/live-gpu-k18-1030.log).

**Edge cases.**
- [x] The last layer, whose next norm is the final norm: a step's; a chunk's last layer feeds only its last row
  (PARITY-LAST-LAYER-ROWS) (shared side; receipt: logs/one-ir/moe-commit-check-1028.log).
- [x] A shared expert: folded into the mixture (a step), or its product summed with the mixture first (a chunk) (shared
  side; receipt: logs/one-ir/moe-commit-check-1028.log).
- [x] The k order kept for bit-exactness: the fold equals its parts bit for bit (shared side; receipt: logs/one-ir/moe-commit-check-1028.log).
- [x] Weights renormalized or not (shared side; receipt: logs/one-ir/moe-commit-check-1028.log).

**Success.**
- [x] All RED green (receipt: logs/one-ir/live-gpu-k18-1030.log).

**Done when.** A MoE layer's output is combined inside the next norm.

#### PARITY-SMALL-M-GEMM · A minimum-latency GEMM for 1–16 rows

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `primitive` `cuda` `area:gemm` |
| Depends on | MIGRATE-1, SETUP-4 |
| Touches | the lane's GEMV / GEMM choice (`GEMV_ROWS`), a new tensor-core kernel |
| Coverage | 1 row in section 11 |
| Status | open |

**Goal.** For 2–16 rows (MTP verification, small batches, short chunks), a tensor-core GEMM with A and B swapped (the
weight rows as M):
- split over K inside the CTA;
- fed by an mbarrier `cp.async` pipeline;
- chosen by the shim's cost against the GEMV.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's 1–16-token GEMM swaps A and B, uses `mma.sync` m16n8k16 fed by an mbarrier pipeline, splits K over four warps
  inside the CTA, and uses PDL (`external/competitors/vllm/csrc/dsv3_fused_a_gemm.cu` · `fused_a_gemm_kernel(`).
- The number: µs at M = 2, 4, 8 and 16 for each model's projections (SETUP-4), against our GEMV and llama.cpp's mmvq
  and mmq.

**Beyond.**
- int8 m16n8k32 with the block formats decoded in registers.
- Target: ≥ 85% of measured bandwidth at M = 8.

**RED tests.**
- [ ] `lane_compile_lock::a_chunk_of_eight_rows_multiplies_on_the_tensor_cores_with_its_weights_as_m`
- [ ] `cuda_lane_lock::the_small_m_gemm_computes_what_the_gemv_computes`: within its declared bound (OPT-6).
  The reduction order differs from the GEMV's.

**Edge cases.**
- [ ] M = 1 never takes it.
- [ ] M not a multiple of 8.
- [ ] A K tail.
- [ ] 256-element super-blocks.

**Success.**
- [ ] All RED green.
- [ ] The bandwidth target met (receipt: A/B row).

**Done when.** No row count between 2 and 16 runs slower than the best published kernel.


#### PARITY-BATCH-ROW-QUANT · A batch's sequences quantize their rows where they store them

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `area:quant` |
| Depends on | none |
| Touches | the lane's `sequences` layout (`plan.rs`: a family laid out in the table's quantized form too), `merge_attentions` and `merge_deltas`, the lane shim's fits for `AttnOutputQuant`, `GdnOutputQuant` and `GluActMulQuant` over a batch's `AttnOutputGate` |
| Coverage | 0 rows in section 11 |
| Status | open |

**Goal.** A batch step's per-sequence producers whose rows the output projection reads quantized store them quantized,
as the decode step's do: no quantize launch of a batch's attended or recurrent rows.
- Today each sequence's attention (gated or not, K-13) and delta rule writes its row of the table plain, and the table
  is quantized by a launch of its own: the `sequences` layout lays a family out only in the plain array.
- A family is laid out in the table's quantized form too: sequence `b`'s words and scales are its row's of the table's,
  so the merged launch quantizes every row in its store as the decode step's launch quantizes its one row.

**Copy.** Bar: B16 (section 2.4).
- vLLM's attention backends write the output projection's quantized input for every row of the batch in the attention
  itself, `AttnFusionPass` folding the quantize where the backend supports it
  (`external/competitors/vllm/vllm/v1/attention/backends/triton_attn.py` · `def fused_output_quant_supported(`).

**RED tests.**
- [x] `lane_compile_lock::a_batchs_attentions_and_delta_rules_quantize_their_rows_in_their_store`: every tiny
  architecture at its wide width, both quantized numerics, batches of 2, 4 and 20: no `quant` launch reads an
  attention's or a delta rule's rows (logs/one-ir/live-red-brq-1031.log RED, live-green-brq5-1101.log GREEN).
- [x] `cuda_lane_lock::a_batch_decodes_each_sequence_as_its_own_step_does` and
  `cuda_lane_lock::a_batch_past_the_gemvs_rows_decodes_each_sequence_as_alone` stay bit for bit
  (logs/one-ir/live-gpu-brq-1101.log).

**Design.** The shared numerics pass quantizes a table every row of which is set, once and in whole blocks, from a
result its producer can quantize in its epilogue, a row at a time: each producer takes its row's quantize
(`attn_output_quant`, `gdn_output_quant`, `glu_act_mul_quant` over a batch's gated attentions) and each set the
table's (`row_write_quant`), a set carrying its rows' quantized form. The lane's `sequences` lays a family out in the
table's quantized form too, so the sets leave the step and the merged launch quantizes every row in its store; held
apart, a set copies the rows' words and scales (`lk_set_row_q`). A producer writes `f32`, and a set copies values,
only where something reads the table `f32`.

**Edge cases.**
- [ ] Heads of 16 (half a 32-value group): apart, as in the decode step.
- [x] `sequences` held: each sequence's launch quantizes its own row of the table (the RED test's held half).

**Success.**
- [x] All RED green.
- [ ] B16 on Qwen3.5 and Llama-3.2-1B: one launch fewer a layer (receipt: the census and the A/B row).

#### PARITY-SAMPLING · Serving-grade sampling on the device

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `area:sampling` |
| Depends on | OPT-1, MIGRATE-1 |
| Touches | the language (a sampling clause), `tensor_graph::front`, the lane's head and arg-max |
| Coverage | 33 rows in section 11 |
| Status | open |

**Goal.** The step's next token is chosen on the device by every rule serving needs:
- temperature;
- top-k, top-p, min-p;
- repetition, frequency and presence penalties;
- a Gumbel sample on a counter RNG keyed by (seed, position), so a seeded run is reproducible in every form;
- top-k logprobs;
- a grammar bitmask.

Today the lane has arg-max only.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's pivot-based top-k and top-p (`external/competitors/vllm/vllm/v1/sample/ops/topk_topp_triton.py` · `def apply_top_k_top_p_triton(`).
- vLLM's Gumbel sampler (`external/competitors/vllm/vllm/v1/worker/gpu/sample/gumbel.py` · `def _gumbel_sample_kernel(`).
- vLLM's penalties in one kernel (`external/competitors/vllm/vllm/v1/worker/gpu/sample/penalties.py` · `def _penalties_kernel(`).
- The ledger's FlashInfer citation.

**Beyond.**
- The head GEMV keeps per-block candidates (the block's top-k, max and sum of exponentials) in its epilogue, so the full
  logits row is never written and re-read. vLLM samples after writing the full row.
- Target: sampling costs ≤ 3% of a decode step at a vocabulary of 151,936.

**RED tests.**
- [ ] `device_sampling_lock::greedy_sampling_is_the_argmax`
- [ ] `device_sampling_lock::a_seeded_sample_is_reproducible_across_forms`
- [ ] `device_sampling_lock::top_k_and_top_p_keep_the_reference_sets`
- [ ] `device_sampling_lock::penalties_match_the_reference`
- [ ] `device_sampling_lock::logprobs_match_the_f64_log_softmax`
- [ ] `lane_compile_lock::the_head_samples_in_its_epilogue`

**Edge cases.**
- [ ] Ties (the first maximum wins).
- [ ] Top-p of 1 and top-k of the whole vocabulary, which change nothing.
- [ ] Temperature 0, which equals greedy.
- [ ] A row the grammar masks entirely: refused by name.
- [ ] Very peaked distributions (underflow).
- [ ] A vocabulary that is not a multiple of the block.

**Success.**
- [ ] All RED green.
- [ ] The 3% target met (receipt: profile).

**Done when.** Sampling is on the device, reproducible, and nearly free.

#### PARITY-PERSISTENT-LAYER-MEGAKERNEL · A decoder layer as one persistent launch

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` |
| Depends on | OPT-2, MIGRATE-1, TRITON-TILE-TIER |
| Touches | `tensor_graph::fuse` (a layer-region symbol), the CUDA link, cooperative launch |
| Status | open |

**Goal.** A decoder layer, and later the whole step, runs as one or two persistent cooperative launches:
- grid-wide syncs between stages;
- weights streamed;
- small intermediates kept on chip.

The aim is the launch-bound small models, where a decode step today is many short kernels replayed from a graph.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's AMD "mono" decode runs a whole DeepSeek-V4 decoder layer as two persistent launches with an in-kernel
  all-reduce (`external/competitors/vllm/vllm/models/deepseek_v4/amd/mega_moe.py` · `def _append_shared_expert_kernel`).
- Kimi-K3's specialized decode does the same, with an L2 weight prefetch.
- Both are written by hand for one model.

**Beyond.**
- Compiled from the IR for every model.
- Target: decode ≥ 1.15× our graph replay on Llama-3.2-1B and Qwen3.5-0.8B.

**RED tests.**
- [ ] `lane_compile_lock::a_megakernel_step_is_one_cooperative_launch`
- [ ] `cuda_lane_lock::the_megakernel_step_computes_what_the_launches_compute`: bit for bit.

**Edge cases.**
- [ ] Every block must be resident (occupancy).
- [ ] The grid-sync cost.
- [ ] MoE layers.
- [ ] Long attention spans.
- [ ] Host-resident weights.

**Success.**
- [ ] All RED green.
- [ ] The target met (receipt: scoreboard).

**Done when.** Small models decode from one launch per layer at a speed graph replay cannot reach.

#### PARITY-ROPE-QK-ONE-LAUNCH · Query and key rotated in one launch

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `area:attention` |
| Depends on | OPT-1, MIGRATE-1 |
| Touches | `tensor_graph::fuse` (a horizontal rule), the lane's rope kernel |
| Coverage | 2 rows in section 11 |
| Status | open |

**Goal.** The query's norm-and-rotate and the key's norm-rotate-and-append run in one launch over the projection's
output. Today they are two.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM rotates query and key in one launch over strided views of qkv
  (`external/competitors/vllm/csrc/pos_encoding_kernels.cu` · `rotary_embedding_kernel(`).
- vLLM fuses norm and rotation for both (`external/competitors/vllm/csrc/fused_qknorm_rope_kernel.cu` · `fusedQKNormRopeKernel(`).

**Beyond.** The value row's cache write joins the same launch, making it horizontal over all three outputs.

**RED tests.**
- [ ] `fusion_class_lock::two_independent_row_ops_over_one_source_fuse_horizontally`
- [ ] `lane_compile_lock::query_and_key_rotate_in_one_launch`

**Edge cases.**
- [ ] Unequal head counts.
- [ ] Partial rotation.
- [ ] A key with a row destination.
- [ ] The interleaved sections.

**Success.**
- [ ] All RED green.

**Done when.** An attention layer's prologue is one launch.

#### PARITY-ELEMENTWISE-REGIONS · Pointwise DAG regions fused

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `spatial` |
| Depends on | OPT-1, MIGRATE-1, MIGRATE-2 |
| Touches | `tensor_graph::fuse`; the forward's elementwise emitter; the spatial Generic splice |
| Coverage | 1 row in section 11 |
| Status | the older CUDA forward and the spatial chips link it; the lane open |

**Goal.**
- Any connected pointwise subgraph becomes one region symbol: diamonds, several outputs, broadcast operands. Today only
  linear chains fuse (the ledger's note on `elementwise_chains`).
- CUDA emits it as one kernel; spatial emits it as one Generic.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- Inductor's pointwise fusion, which vLLM's compile path uses (the ledger's `elementwise_chains` citation).

**Beyond.** Proven regions, with registers allocated across the region and outputs kept only where read.

**RED tests.**
- [x] `fusion_class_lock::a_pointwise_dag_fuses_into_one_region` (receipt: logs/one-ir/live-green3-1029.log)
- [x] `cuda_elementwise_fusion_lock::a_diamond_shaped_pointwise_region_fuses_into_one_kernel` (receipt: logs/one-ir/live-green3-1029.log)
- [x] `spatial_forward_lock::a_pointwise_region_is_one_section_and_one_body` (receipt: logs/one-ir/live-green-k18-2-1030.log)

**Edge cases.**
- [x] An intermediate a later op reads (`cuda_elementwise_fusion_lock::an_intermediate_needed_later_is_materialized_not_fused_away`,
  `cuda_elementwise_fusion_lock::a_region_writes_out_an_arm_a_later_op_reads`; receipt: logs/one-ir/live-green3-1029.log).
- [x] Mixed integer and float (`fusion_class_lock::a_pointwise_dag_fuses_into_one_region`; receipt: logs/one-ir/live-green3-1029.log).
- [x] A region that stops at a reduction (same receipt).

**Success.**
- [x] All RED green; the regions equal their kernels apart bit for bit on the card
  (`cuda_fusion_census_lock::every_forward_fusion_computes_what_its_kernels_compute_apart`; receipt: logs/one-ir/live-green3-1029.log).

**Done when.** Every pointwise region is one kernel on every chip.

#### PARITY-QKV-HEAD-EPILOGUE · One q/k/v GEMV with the per-head norm, rotation and cache append in its epilogue

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `area:attention` |
| Depends on | OPT-1, MIGRATE-1 |
| Touches | `tensor_graph::fuse` (a horizontal merge plus a per-head epilogue fold); the lane's GEMV kernels and `lk_rope` |
| Owner notes | the lane belongs to the performance lane |
| Coverage | 14 rows in section 11 |
| Status | linked on the CUDA lane for a step's one row (K-03); bandwidth owed |

**Goal.**
- K-03, K-13 and K-19: a decode layer's query, key and value come from one GEMV launch.
- The block that completes a head applies that head's RMSNorm, its rotation and its cache write.
- Gemma's 5 launches and Llama's 5 become 1.

**RED tests.**
- [x] `fusion_class_lock::a_projection_feeding_per_head_norms_and_rotations_folds_them_into_its_epilogue`: one op for a
  layer's q, k and v with each head's norm, rotation and cache write (shared side; receipt: logs/one-ir/qkv-commit-check-1028.log)
- [x] `lane_compile_lock::a_decode_layers_query_key_and_value_are_one_launch_with_their_head_epilogue` (receipt: logs/one-ir/live-green3-1029.log)
- [x] `cuda_lane_lock::the_head_epilogue_computes_what_the_rope_launches_compute`: bit for bit (receipt: logs/one-ir/live-gpu-1030.log).

**Edge cases.**
- [x] Unequal query and key head counts (4:1, 8:1, 3:1 in Gemma-3, Llama, Qwen3.5) (shared side; receipt: logs/one-ir/qkv-commit-check-1028.log).
- [x] Head dimensions 64, 128 and 256 (shared side; receipt: logs/one-ir/qkv-commit-check-1028.log).
- [x] Partial rotation with interleaved sections: Qwen3.5's (shared side; receipt: logs/one-ir/qkv-commit-check-1028.log).
- [x] Biases (gpt-oss) (shared side; receipt: logs/one-ir/qkv-commit-check-1028.log).
- [x] A query with its gate (Qwen3.5) (shared side; receipt: logs/one-ir/qkv-commit-check-1028.log).
- [ ] A key written into a quantized cache (PARITY-KV-CACHE-QUANT); the key's cache write is in the fold (shared side; receipt: logs/one-ir/qkv-commit-check-1028.log), its
  quantize with that ticket.
- [x] The per-head counter reset between steps, and between graph replays: every form is a captured graph the
  GPU test replays through a prompt and its decode steps (receipt: logs/one-ir/live-gpu-1030.log).

**Success.**
- [x] RED green.
- [x] Each census shows one launch for q, k and v (receipt: `tests/fixtures/lane_census/`; a Q4_K query four rows a
  warp turns its heads in a launch of its own).
- [ ] The merged GEMV at ≥ 85% of bandwidth (receipt: profile).

**Mutants to kill.**
- [ ] The counter never resets.
- [ ] The norm applied before the last rows arrive.
- [ ] Rotation pairs crossing a head.

**Done when.** No decode layer launches a separate rotation.

#### PARITY-GEMV-NORM-EPILOGUE · A projection with the residual add and the next norms in its last-arriving block

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `area:norm` |
| Depends on | OPT-1, MIGRATE-1 |
| Touches | `tensor_graph::fuse`; the lane's GEMV kernels and `lk_rmsnorm` |
| Coverage | 11 rows in section 11 |
| Status | linked on the CUDA lane for a step's one row in Q8_0 (K-05, K-07); the block formats and the scoreboard owed |

**Goal.** K-05, K-07, K-11, K-16 and K-18.
- An output, down or GDN-out projection finishes the row it writes in its last-arriving block: the post-norm (sandwich),
  the residual add, the written sum, the next pre-norm and the quantize.
- vLLM keeps every full-row norm as its own launch.

**RED tests.**
- [x] `fusion_class_lock::a_projection_feeding_a_residual_and_a_norm_folds_them` (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log)
- [x] `lane_compile_lock::a_projection_feeding_an_add_and_norm_finishes_them_in_its_last_block` (receipt: logs/one-ir/live-green3-1029.log)
- [x] `cuda_lane_lock::the_last_block_norm_computes_what_the_norm_launch_computes`: bit for bit, using
  `lk_rmsnorm`'s reduction tree (receipt: logs/one-ir/live-gpu-1030.log).
- [x] `lane_compile_lock::a_block_format_projection_finishes_its_add_and_norm_in_its_last_block` and
  `cuda_lane_lock::the_block_projections_last_block_norms_as_the_norm_launch_does`: a projection in every block format
  the library decodes (`lk_gemv_blk_norm`), bit for bit (receipt: logs/one-ir/live-green-blk2-1031.log).

**Edge cases.**
- [x] Gemma's two norms in one fold (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log).
- [x] The last layer, whose next norm is the final norm (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log).
- [x] A residual also read by a later op (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log).
- [x] Every numerics point's quantize epilogue: one word and two (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log).
- [x] Rows of 1024 to 5120 (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log).
- [x] A block-format projection four rows a warp, the block's four warps playing the norm's 32 (receipt:
  logs/one-ir/live-green-blk2-1031.log).

**Success.**
- [x] RED green.
- [x] Gemma-3-1B decode layer at 5 launches (receipt: `tests/fixtures/lane_census/gemma3-1b.txt`).
- [ ] Decode tok/s up on every model (receipt: scoreboard).

**Mutants to kill.**
- [ ] The residual added after the norm.
- [ ] A block other than the last applying the norm.

**Done when.** No decode layer launches a standalone norm.

#### PARITY-ATTN-QUANT-IN-FOLD · Decode attention quantizes its output in its fold

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `area:attention` `area:quant` |
| Depends on | OPT-1, MIGRATE-1 |
| Touches | `tensor_graph::fuse` (the `attn_output_quant` fold on the lane); the lane's attention fold |
| Coverage | 5 rows in section 11 |
| Status | open |

**Goal.** K-04. The older forward already quantizes in the attention epilogue
(`cuda_fusion_census_lock::an_attention_feeding_an_int8_projection_quantizes_in_its_epilogue`), but the lane launches
`lk_quant` after `lk_attention` (Gemma's census, launch 8). The lane's fold writes the int8 words and scales.

**RED tests.**
- [x] `lane_compile_lock::a_decode_attention_quantizes_its_output_in_its_fold` (receipt: logs/one-ir/live-green-k18-4-1031.log)
- [x] Values in `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart` (receipt: logs/one-ir/live-gpu-k18-1030.log).

**Edge cases.**
- [ ] The shared-row (latent) attention fold.
- [ ] The tiled prefill attention epilogue; today a chunk in tiles quantizes in a launch of its own.
- [x] The two-word numerics point (`lane_compile_lock::a_decode_attention_quantizes_its_output_in_its_fold`).

**Success.**
- [x] RED green (receipt: logs/one-ir/live-gpu-k18-1030.log).
- [x] No `lk_quant` after any attention in any census (receipt: `tests/fixtures/lane_census/`).

**Done when.** Attention outputs leave quantized.

#### PARITY-GDN-INPUT-EPILOGUE · Every gated-delta input projection in one launch, with conv, SiLU, L2 and gates in its epilogue

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `area:recurrent` `model:qwen35` |
| Depends on | OPT-1, MIGRATE-1 |
| Touches | `tensor_graph::fuse`; the lane's GEMVs, `lk_conv`, `lk_gates` and `lk_l2norm` |
| Status | linked on the CUDA lane for a step's one row, its gates in Q8_0 or projected in `f32` (`lk_gemv_gdn`); slower than its launches apart on the card (Qwen3.5-0.8B 294 tok/s fused, 321 held, 11-06): its epilogue being repaired |

**Goal.**
- K-09. One GEMV over the rows [qkv | z | alpha | beta] (8224 rows for Qwen3.5).
- Per-row epilogues: the conv tap, the bias, SiLU and the ring advance for qkv rows; the gates for alpha and beta.
- A per-head last-arriver applies the q/k L2 norms.
- 5 launches become 1.

**Measured 11-06 (logs/fusion-k09/window2/, logs/fusion-k09/k09var/).** On the real Qwen3.5-0.8B the fused decode step
ran 294 tok/s and its launches apart 321: `lk_gemv_gdn` took 931 µs of a token's 18 layers where the GEMV in parts, the
convolutions and the gates took 577, 125 and 90. Variants of the same program: without the convolution tap 660 µs,
without the L2 last-arriver 749, at 48 registers 872. Each warp's tap read its ring and taps after its dot — a global
read's latency at the end of every warp, nine waves of them a launch — and every block fenced for the heads only the
query and key blocks norm. The repair reads a row's operands into shared memory by `cp.async` from the warp's idle
lanes while its dot streams, and lets only a normed head's blocks fence (held by
`cuda_lane_lock::the_gdn_input_epilogue_decodes_faster_than_its_launches_apart`, RED at 2,596 tok/s against 2,796 on a
Qwen3.5 at 2048 wide with the real heads).

**RED tests.**
- [x] `fusion_class_lock::a_projection_feeding_a_causal_conv_and_gates_folds_them_per_row`: every input projection of
  one activation with the convs, SiLU, L2 norms and gates as one op (`gdn_input_epilogue`) (shared side; receipt: logs/one-ir/gdn-commit-check-1028.log)
- [x] `fusion_class_lock::a_gated_delta_layers_input_epilogue_takes_its_gates_products_where_they_stay_apart`: the gates'
  products join the epilogue where the gates take them apart (a chip that runs Q8_0 gates only inside the epilogue),
  those of a second activation stay out (shared side; receipt: RED logs/fusion-k09/red-k09-shared.log, GREEN
  logs/fusion-k09/green-k09-shared.log).
- [x] `lane_compile_lock::a_gated_delta_layers_input_projections_are_one_launch_with_conv_and_gates`: gates in `f32` and
  in Q8_0, 2048 wide, Qwen3.8's three value heads a key head, heads of 512, `fuse_quant` off (receipt: RED
  logs/fusion-k09/red-k09.log, GREEN logs/fusion-k09/green-k09.log).
- [x] `cuda_lane_lock::the_gdn_input_epilogue_computes_what_its_launches_compute_apart`: bit for bit, every case of
  `tiny_models::gated_delta_cases` under one word, two words and the precise numerics (receipt:
  logs/fusion-k09/gpu-k09-tests.log, with `every_fusion_computes_what_its_launches_compute_apart`, the hybrid step at width,
  the reference and Qwen3.5-0.8B's golden bars).

**Edge cases.**
- [x] The conv ring's first steps (a zero history) (shared side; receipt: logs/one-ir/gdn-commit-check-1028.log).
- [x] Value heads outnumbering key heads (Qwen3.8: 48 to 16): three to one (shared side; receipt: logs/one-ir/gdn-commit-check-1028.log).
- [ ] Prefill chunks: the carry across chunks (`lk_conv_carry`); a chunk's convs and gates are one op past its
  projections, the rings and memory carried chunk to chunk (shared side; receipt: logs/one-ir/gdn-commit-check-1028.log), the lane's carry with its kernel.
- [x] f32 projections on Qwen3.8 (its alpha and beta are F32) (shared side; receipt: logs/one-ir/gdn-commit-check-1028.log).

**Success.**
- [x] RED green (receipts above).
- [ ] Qwen3.5 GDN layer census at 4 launches (receipt). Today 5: K-09 took each GDN layer from 7 (`gemv_parts conv
  gates delta gemv_norm gemv_glu gemv_norm`) to 5 (`gemv_gdn delta gemv_norm gemv_glu gemv_norm`), Qwen3.5-0.8B's decode
  step from 159 launches a token to 123 (receipt: `tests/fixtures/lane_census/qwen3.5-0.8b.txt`); the fourth goes with
  the GLU and its down projection.
- [ ] Decode up (receipt: scoreboard).

**Mutants to kill.**
- [ ] The ring advanced before the tap.
- [ ] softplus replaced by exp.

**Done when.** A GDN layer's inputs are one launch.

#### PARITY-GDN-HALF-STATE · Half-precision gated-delta state

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `area:recurrent` `area:quant` `model:qwen35` |
| Depends on | OPT-6, PARITY-GDN-DECODE-WHOLE |
| Touches | the GDN state tensors' storage; `lk_delta_step` / the whole-decode kernel; the numerics contracts |
| Owner notes | a numerics knob: Tristen rules on each model's bar |
| Status | open |

**Goal.** K-12 and K-31.
- The state is stored in bf16 or f16, updated in f32, written back with round-to-nearest-even or stochastic rounding.
- State traffic halves: Qwen3.8 302 → 151 MB a token.
- vLLM keeps this state in bf16.

**RED tests.**
- [ ] `numerics_contract_lock::a_half_precision_state_declares_and_meets_its_bound`
- [ ] `cuda_lane_lock::qwen3_5_0_8b_meets_its_bar_with_a_half_precision_state`

**Edge cases.**
- [ ] Long runs (error accumulation over 4096 steps).
- [ ] Stochastic rounding seeded by (layer, position) so runs reproduce.

**Success.**
- [ ] RED green.
- [ ] The bar met, or the knob refused for the model by name (receipt).

**Done when.** Recurrent state traffic is a knob with a proven bound.

#### PARITY-MLA-PROLOGUE · The latent layer's q and latent in one launch with norm, rotation, append and absorption

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `area:attention` `model:ds2` |
| Depends on | OPT-1, MIGRATE-1 |
| Touches | `tensor_graph::fuse`; the lane's GEMVs, `lk_head_axpy_staged` and `lk_rope` |
| Coverage | 13 rows in section 11 |
| Status | open |

**Goal.**
- K-14. One GEMV over [q 3072 | kv_a 576].
- The latent's last arriver normalizes the 512 latent values, rotates the 64 rope values and appends the cache row.
- Each query head's last arriver rotates its rope part and multiplies its nope part by W_UK's head slice into the
  absorbed query.

**RED tests.**
- [ ] `lane_compile_lock::a_latent_layers_prologue_is_one_launch`
- [ ] `cuda_lane_lock::the_latent_prologue_computes_what_its_launches_compute_apart`

**Edge cases.**
- [ ] A query LoRA (DeepSeek V3), which adds a norm between two projections.
- [ ] The split `attn_k_b` / `attn_v_b` checkpoint form.
- [ ] YaRN scaling on the rope part.

**Success.**
- [ ] RED green.
- [ ] DeepSeek-V2-Lite decode up (receipt: scoreboard).

**Done when.** A latent layer's prologue is one launch.

#### PARITY-MLA-UNABSORB-IN-FOLD · Latent attention un-absorbs its values and quantizes in its fold

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `area:attention` `model:ds2` |
| Depends on | OPT-1, MIGRATE-1 |
| Touches | `lk_attention_fold`; the per-head value product |
| Coverage | 22 rows in section 11 |
| Status | open |

**Goal.** K-15. The fold's last block multiplies each head's 512-wide context by W_UV's head slice and writes the
quantized 2048-wide output row.

**RED tests.**
- [ ] `lane_compile_lock::a_latent_attention_unabsorbs_its_values_in_its_fold`
- [ ] `cuda_lane_lock::the_unabsorbing_fold_computes_what_its_launches_compute_apart`

**Success.**
- [ ] RED green.

**Done when.** Latent attention ends quantized, in two launches.

#### PARITY-LAST-LAYER-ROWS · A prefill chunk's last layer computes only its output rows beyond the caches

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `spatial` |
| Depends on | OPT-1, MIGRATE-1 |
| Touches | `tensor_graph::facts` (row-level liveness), `tensor_graph::fuse` |
| Status | open |

**Goal.**
- K-29. A row of the last layer that feeds no output and no state is dead.
- The last layer's attention queries, output projection and FFN run on the last row only; its key and value cache
  writes stay.
- This is llama.cpp's output-row pruning (`external/competitors/llama.cpp/src/llama-graph.cpp` · `ggml_tensor * llm_graph_context::build_inp_out_ids() const {`).

**RED tests.**
- [x] `fusion_class_lock::a_row_that_feeds_no_output_and_no_state_is_removed` (shared side; receipt: logs/one-ir/live-green4-1030.log)
- [ ] `lane_compile_lock::a_prefill_chunks_last_layer_runs_its_output_rows_only`
- [ ] `cuda_lane_lock::a_pruned_chunk_continues_as_the_unpruned_chunk_does`: bit for bit on the next decode steps.

**Edge cases.**
- [x] A recurrent last layer: its state needs every row in order (shared side; receipt: logs/one-ir/live-green4-1030.log).
- [x] A batched form with several output rows (shared side; same receipt).
- [x] Logprobs requested for every prompt row: nothing is pruned (shared side; same receipt).

**Success.**
- [ ] RED green.
- [ ] pp512 up by the last layer's share (receipt: scoreboard).

**Done when.** No prefill computes a row nobody reads.

#### PARITY-INT8-GEMM-PEAK · The dense int8 GEMM past 55 TOPS, and the per-channel W8A8 knob point

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `primitive` `cuda` `area:gemm` |
| Depends on | SETUP-4, TRITON-GENERIC-SWIZZLE |
| Touches | `lk_gemm_blk`, `lk_gemm_q8`; the numerics contracts (the per-channel point) |
| Owner notes | the lane's GEMMs belong to the performance lane; the per-channel point is a numerics knob Tristen rules on |
| Coverage | 7 rows in section 11 |
| Status | open |

**Goal.** K-23 and K-32.
- The exact Q8_0 GEMM:
  - a warp-specialized producer three stages ahead;
  - four consumer warps;
  - swizzled shared memory;
  - the fold measured and shortened.
- Beside it, a per-channel W8A8 point with no fold in the inner loop.

**RED tests.**
- [ ] `cuda_lane_lock::the_warp_specialized_gemm_computes_what_the_block_gemm_computes`: bit for bit on the exact point.
- [ ] `kernel_ab_lock::the_dense_int8_gemm_meets_its_tops_bar`
- [ ] `numerics_contract_lock::the_per_channel_point_declares_and_meets_its_bound`

**Edge cases.**
- [ ] K not a multiple of 128.
- [ ] N tails.
- [ ] The two-word fold.
- [ ] The paired gate-and-up and parts epilogues on the new mainloop.

**Success.**
- [ ] Exact point ≥ 55 TOPS one-word and ≥ 40 two-word (receipt: A/B row).
- [ ] Per-channel point ≥ vLLM W8A8 at our shapes (receipt).

**Done when.** Prefill GEMMs run at or past CUTLASS-class speed on the 3060.

#### PARITY-PREFILL-ATTENTION-PEAK · Prefill attention at 15 TFLOPS f16

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `primitive` `cuda` `area:attention` |
| Depends on | SETUP-4, PARITY-ATTENTION-VARIANTS-INLINE |
| Touches | `lk_attention_mma`, `lk_attention_i8` |
| Coverage | 15 rows in section 11 |
| Status | open |

**Goal.** K-26.
- Static masks skip tiles.
- K and V double-buffered.
- Two consumer warpgroups ping-pong softmax and MMAs.
- The output quantized in the epilogue.

**RED tests.**
- [ ] `lane_compile_lock::a_causal_chunk_launches_no_empty_tile`
- [ ] `cuda_lane_lock::the_pipelined_tensor_core_attention_computes_what_the_attention_computes`
- [ ] `kernel_ab_lock::prefill_attention_meets_its_tflops_bar`

**Edge cases.**
- [ ] Head dims 64, 128 and 256.
- [ ] A window.
- [ ] Sinks.
- [ ] Soft cap.
- [ ] A chunk shorter than a tile.

**Success.**
- [ ] ≥ 15 TFLOPS on Qwen3-8B shapes.
- [ ] ≥ 1.1× vLLM FlashAttention-2 (receipt: A/B row).

**Done when.** Prefill attention runs near the f16 tensor peak.

#### PARITY-GDN-CHUNK-TENSOR-CORES · The chunked delta rule on the tensor cores

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `primitive` `cuda` `area:recurrent` `model:qwen35` |
| Depends on | SETUP-4, TECH-SPLIT-PRECISION-MMA |
| Touches | `lk_delta_prep`, `lk_delta_state` |
| Coverage | 9 rows in section 11 |
| Status | open |

**Goal.**
- K-27. Every chunk product runs on the tensor cores in split precision: KKᵀ, the WY recompute, K·Sᵀ, Q·Sᵀ, P·Δ and
  the state update.
- Prep and state fused per chunk.

**RED tests.**
- [ ] `cuda_lane_lock::the_tensor_core_delta_rule_computes_what_the_delta_rule_a_row_at_a_time_computes`: within each
  knob point's bound.
- [ ] `kernel_ab_lock::the_chunked_delta_rule_meets_its_bar`

**Edge cases.**
- [ ] A chunk shorter than 64.
- [ ] The decay underflowing in long chunks.
- [ ] Value heads outnumbering key heads.

**Success.**
- [ ] ≤ 0.6× llama.cpp's time on Qwen3.5 pp512 (receipt: A/B row).

**Done when.** GDN prefill is tensor-core bound, not latency bound.

#### PARITY-PREFILL-NORM-EPILOGUES · Wide-chunk norms, residuals and quantizes in GEMM epilogues

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `fusion` `cuda` `area:norm` |
| Depends on | OPT-1, MIGRATE-1 |
| Touches | `tensor_graph::fuse`; `lk_gemm_blk` epilogues |
| Status | open |

**Goal.**
- K-28. Each output tile writes a per-row partial sum of squares.
- The last tile of each row finishes the residual add, the norm and the quantize.

**RED tests.**
- [ ] `lane_compile_lock::a_wide_chunk_launches_no_standalone_norm`
- [ ] Values in `cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart` on wide chunks.

**Edge cases.**
- [ ] Gemma's sandwich pairs.
- [ ] Split-K tails.
- [ ] Rows whose tiles finish out of order.

**Success.**
- [ ] RED green.
- [ ] pp512 up on every model (receipt: scoreboard).

**Done when.** Wide chunks launch no norm of their own.

#### PARITY-WEIGHT-FORMATS · Every weight format the rivals run, read in place or repacked at load

| Field | Value |
|---|---|
| Phase | parity, tier 1 |
| Labels | `primitive` `cuda` `area:gemm` `area:quant` |
| Depends on | MIGRATE-1, SETUP-4, TECH-LOP3-PRMT-DEQUANT |
| Touches | `crates/logicaffeine_compile/src/codegen_cuda/lane/weights.rs`; `crates/logicaffeine_compile/src/model/gguf.rs`; a safetensors reader beside it; the lane's GEMV and GEMM kernels |
| Owner notes | the lane belongs to the performance lane |
| Coverage | 37 rows in section 11 |
| Status | 16-bit float matrices read in place on the CUDA lane's GEMVs, gathers and folds (phase 1); MXFP4 rows regrouped at upload; the 16-bit tensor-core products owed at two knob points (Tristen 11-06: one bf16 word, the cells' point, at the tensor cores' full rate; two words, the precise point), the safetensors formats and the other repacks owed |

**Goal.** Our side of every cell reads at most the weight bytes the rival reads.
- **16-bit weights stream at 16 bits.** Today `lane/weights.rs` reads a matrix stored as F32, F16 or BF16 as values,
  and widens 16-bit weights to f32 on the host. That doubles their bytes, and it is why no BF16 cell is comparable
  today.
- **The rivals' quantized checkpoints read too.** GPTQ, AWQ and compressed-tensors (int8, FP8 and int4 groups) from
  Hugging Face safetensors. Each is read as its reference dequantizes it, so a cell can run both sides from the same
  checkpoint.
- **Repacked layouts.** Weights are repacked at load into the layout the kernel reads (Marlin's interleave) where that
  layout is faster, with the repack proven equal to the stored form.

**Copy.**
- Marlin's repacks:
  - `external/competitors/vllm/csrc/quantization/marlin/gptq_marlin_repack.cu` · `__global__ void gptq_marlin_repack_kernel(`;
  - `external/competitors/vllm/csrc/quantization/marlin/awq_marlin_repack.cu` · `__global__ void awq_marlin_repack_kernel(`.
- The magic-number dequantization in registers: `external/competitors/vllm/csrc/quantization/marlin/dequant.h` · `__device__ inline int lop3`.
- compressed-tensors' scheme dispatch: `external/competitors/vllm/vllm/model_executor/layers/quantization/compressed_tensors/compressed_tensors.py` · `class CompressedTensorsConfig(`.
- Bar: the row maximum (section 2.5) for decode GEMV and prefill GEMM at 16 and 4 bits.

**Beyond.**
- One reader per format feeds the same block decoders, so every format inherits every fusion the lane has. A Marlin
  kernel is a GEMM only; ours carries the norm, quantize and residual epilogues of K-05 and K-28.
- The repack is a proven layout map (TRITON-LINEAR-LAYOUT), not a hand-written shuffle.

**Read first.**
- `crates/logicaffeine_compile/src/codegen_cuda/lane/weights.rs` (`let float = matches!(ty, GgmlType::F32 | GgmlType::F16 | GgmlType::Bf16);`);
- `crates/logicaffeine_compile/src/model/gguf.rs`;
- `lane_compile_lock::every_row_reading_kernel_keeps_its_decode_in_registers`.

**RED tests.**
- [x] `weight_format_lock::a_bf16_matrix_streams_two_bytes_a_weight`: fails today because the lane widens it to f32
  (receipt: RED logs/fusion-k09/red-bf16-final.log, GREEN logs/fusion-k09/green-bf16.log; a BF16 Llama-3.2-1B planned at
  5,785 MiB of `f32` values and 195 launches a token before, 2,425 MiB read in place and 115 launches after).
- [x] `weight_format_lock::a_f16_matrix_streams_two_bytes_a_weight` (receipt: as above).
- [ ] `weight_format_lock::every_safetensors_format_reads_as_its_reference_dequantizes`: GPTQ, AWQ, compressed-tensors
  int8, FP8 and int4. Fails today because there is no safetensors reader.
- [x] `weight_format_lock::a_repacked_matrix_is_the_stored_matrix_under_its_layout_map` (receipt: RED
  logs/fusion-k09/red-mxfp4.log, GREEN logs/fusion-k09/green-mxfp4.log and live-mxfp4.log): an MXFP4 matrix regrouped at
  upload, each row's sub-blocks 16 to a group, the group's exponents before its nibbles (`lk_mx_exp`, `lk_mx_nib`,
  `lk_mx_regroup`, one map the decoders and the upload share), 4.25 bits a weight kept (gpt-oss's 2,880-wide rows: 1,536
  bytes where the checkpoint has 1,530).
- [x] `weight_format_lock::an_mxfp4_gemm_fetches_each_sub_blocks_nibbles_in_one_load` (receipt: as above; RED
  `lk_gemm_blk<39, 1, 0, 128>` 0 16-byte words and 34 byte loads, GREEN one word and one exponent byte a sub-block). The
  byte-wise fetch's warp-wide loads each touched a line of the L1 for each of the warp's 8 rows, about 2.7 times a
  stage's tensor cycles: gpt-oss-20b's 512-row prefill 257.2 ms before, 232.9 after (logs/fusion-k09/window/oss-regroup.log).
- [ ] `lane_compile_lock::a_sixteen_bit_projection_runs_on_the_tensor_cores_without_widening`
- [ ] `cuda_lane_lock::every_weight_format_computes_what_its_dequantized_matrix_computes` (the 16-bit formats held to
  the reference on Llama, Qwen3, Gemma-3 and Qwen3.5 under every numerics: logs/fusion-k09/gpu-bf16.log; the safetensors
  formats owed)

**Edge cases.**
- [ ] BF16 subnormals and NaN payloads, read bit-exact.
- [ ] GPTQ with activation reordering (`g_idx`), and with descending groups.
- [ ] AWQ's packed order differs from GPTQ's.
- [ ] A group size that does not divide the row (the tail group).
- [ ] FP8 e4m3 on sm_86: weight-only, widened in registers, never in memory.
- [ ] A checkpoint that mixes formats per tensor.

**Implementation.**
- [ ] 16-bit rows in place: half and bf16 GEMV with f32 accumulation (done: `lk_h16_dot` under `lk_row_dot`, every GEMV,
  GEMV in parts, gated pair, fold and gather kernel taking it; the activation read `f32`; machine code of every other
  program unchanged, the Q8_0 Llama program's SASS byte-identical), and f16 tensor-core GEMM for prefill.
- [ ] The safetensors reader, and a format per quantization config.
- [ ] Repack at load where the cost model (OPT-7) prefers it, recorded in the program's identity (MXFP4 done: its group
  layout is in BLOCKS, so every program's identity carries it). Q4_0 and IQ4_NL share MXFP4's nibble order and gain
  the same aligned word by the same regroup.

**Success.**
- [ ] Every RED test green (receipt: log paths).
- [ ] Every BF16 and 4-bit cell of section 2.4 runnable on our side (receipt: sota_cells.json rows).
- [ ] Copy: 16-bit and 4-bit decode GEMV and prefill GEMM at or past the row maximum (receipt: SETUP-4 rows).

**Mutants to kill.**
- [ ] A BF16 matrix widened to f32 again.
- [ ] GPTQ read with AWQ's packing.
- [ ] A repack whose layout map is off by one interleave.

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test weight_format_lock --test lane_compile_lock
cargo nextest run -p logicaffeine-tests --test cuda_lane_lock --cargo-profile test-opt
```

**Done when.** No cell has us reading more weight bytes than the rival, and every rival's checkpoint runs on our side.

### 12.10 Parity, tier 2: the other single-GPU rows

#### PARITY-NORM-SCALE · A norm then a scalar scale

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `fusion` `cuda` `area:norm` |
| Depends on | OPT-1, MIGRATE-1 |
| Status | open |

**Goal.** An RMSNorm followed by `X scaled by C` folds the scale into the norm's weight read. The sugar already exists:
`TensorOpSpec::Scale`.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- llama.cpp fuses a norm with a scale (the ledger's `norm_scale` citation).

**Beyond.** OPT-4 folds the scalar into the weight vector at compile time when the weight is a constant, so the kernel
does no extra multiply.

**RED tests.**
- [x] `fusion_class_lock::a_norm_feeding_a_scale_fuses` (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log)
- [ ] `lane_compile_lock::a_scaled_norm_is_one_row_kernel`

**Edge cases.**
- [x] A scale of 1, whose fold computes the norm itself bit for bit (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log).
- [x] A negative scale (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log).
- [x] A norm with no weight (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log).

**Success.**
- [ ] RED green.
- [ ] Ledger row compiled.

**Done when.** A scaled norm is one kernel.

#### PARITY-GEMM-SCALE · Scales and bias in a GEMM's epilogue

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `fusion` `cuda` `area:gemm` |
| Depends on | OPT-1, MIGRATE-1 |
| Status | open |

**Goal.** A per-row, per-column or scalar scale, plus an optional bias, applied to a GEMM's output in its epilogue. The
scales are read from device pointers, so no host sync is needed.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- CUTLASS epilogues in vLLM with broadcast loads and an int8 zero point
  (`external/competitors/vllm/csrc/cutlass_extensions/epilogue/scaled_mm_epilogues_c3x.hpp` · `struct ScaledEpilogueBiasAzpToken`).

**Beyond.** Constant scales are folded into the weight scales of block formats at compile time.

**RED tests.**
- [x] `fusion_class_lock::a_gemm_feeding_a_scale_fuses` (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log)
- [ ] `lane_compile_lock::a_scaled_projection_scales_in_its_epilogue`
- [ ] `cuda_lane_lock::a_scaled_projection_computes_what_its_launches_compute_apart`

**Edge cases.**
- [x] Per-row vs per-column vs scalar scales: a scalar, a constant row, a constant of the product's shape (no sugar writes a column) (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log).
- [x] Bias and scale order: a bias before the scale and one after it (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log).
- [ ] Tensor-core and GEMV paths: a step's one row and a chunk's four fold (shared side; receipt: logs/one-ir/tier1-commit-check-1028.log); the kernels' paths are the lane's.

**Success.**
- [ ] RED green.

**Done when.** No scaled projection takes a second launch.

#### PARITY-RELU-SQR · ReLU² on every chip

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `fusion` `cuda` `area:norm` |
| Depends on | OPT-1, MIGRATE-1 |
| Coverage | 2 rows in section 11 |
| Status | open |

**Goal.**
- The lane stops refusing `relu of X` (the sugar exists, `ReluSpec`).
- relu² folds into its producer's epilogue, with a quantize when the consumer needs one.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's relu² with a static fp8 quantize, rounding where the standalone kernel rounds
  (`external/competitors/vllm/vllm/model_executor/layers/fusion/relu2_fp8_quant.py` · `def relu_squared_static_fp8_quant(`).

**RED tests.**
- [ ] `lane_compile_lock::a_relu_square_compiles_and_folds_into_its_producer`
- [ ] `cuda_lane_lock::a_relu_square_computes_what_the_reference_computes`

**Edge cases.**
- [ ] NaN in.
- [ ] Negative zero.
- [ ] The quantize epilogue.

**Success.**
- [ ] RED green.
- [ ] Ledger row compiled.

**Done when.** relu² runs fused on every chip.

#### PARITY-DUAL-NORM · Two independent norms in one kernel

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `fusion` `cuda` `area:norm` |
| Depends on | OPT-1, MIGRATE-1 |
| Coverage | 5 rows in section 11 |
| Status | open |

**Goal.** Two norms over independent rows of one source (latent attention's query and KV norms) run as one launch: a
horizontal fusion.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's dual RMSNorm pattern (`external/competitors/vllm/vllm/compilation/passes/fusion/rocm_aiter_fusion.py` · `class MLADualRMSNormPattern`).

**Beyond.** The same horizontal rule, `fusion_class_lock::two_independent_row_ops_over_one_source_fuse_horizontally`
from PARITY-ROPE-QK-ONE-LAUNCH, generalized to any two row ops with a quantize on either.

**RED tests.**
- [ ] `lane_compile_lock::a_latent_layers_two_norms_are_one_launch`

**Edge cases.**
- [ ] Different widths.
- [ ] One norm quantized, the other not.

**Success.**
- [ ] RED green.

**Done when.** Independent norms over one source are one launch.

#### PARITY-RECOMPUTE-FANOUT · Recompute a cheap producer per consumer

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `fusion` `cuda` `spatial` |
| Depends on | OPT-1, OPT-7 |
| Coverage | 2 rows in section 11 |
| Status | open |

**Goal.**
- A cheap producer feeding two consumers, each of which could fuse it, is duplicated into both rather than written out.
- The rule fires when the shim's cost of computing it twice is below the cost of writing it out and reading it back.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM duplicates a fused norm-and-quantize into each of two consumers
  (`external/competitors/vllm/vllm/compilation/passes/fusion/rocm_aiter_fusion.py` · `class DoubleAiterRMSFp8GroupQuantPattern`).

**Beyond.** Decided by cost for any producer, not by a pattern written for one.

**RED tests.**
- [ ] `fusion_class_lock::a_cheap_producer_with_two_fusing_consumers_is_recomputed_in_each`
- [ ] `fusion_class_lock::recomputation_is_refused_when_writing_out_is_cheaper`

**Edge cases.**
- [ ] A producer with a side effect (a state write): never duplicated.
- [ ] Three consumers.

**Success.**
- [ ] RED green.

**Done when.** The optimizer can trade recomputation for memory traffic by cost.

#### PARITY-HORIZONTAL-NORM-ROPE-INSERT · One kernel over a multi-branch row

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `fusion` `cuda` `area:attention` |
| Depends on | OPT-1, MIGRATE-1 |
| Coverage | 1 row in section 11 |
| Status | open |

**Goal.** One launch covers every branch of a packed projection row (query, key, value, and index query and key where a
model has them): their norms, shared rotation and cache inserts.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's MiniMax-M3 kernel covers five branches
  (`external/competitors/vllm/csrc/fused_minimax_m3_qknorm_rope_kv_insert_kernel.cu` · `fusedMiniMaxM3QNormRopeKVInsertKernel(`).

**RED tests.**
- [ ] `lane_compile_lock::every_branch_of_a_packed_projection_is_normed_rotated_and_inserted_in_one_launch`

**Edge cases.**
- [ ] A branch with no norm.
- [ ] A branch with no rotation.

**Success.**
- [ ] RED green.

**Done when.** A packed projection's epilogue work is one launch.

#### PARITY-CROSS-LAYER-LAUNCH · Every layer's cache insert in one launch

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `fusion` `cuda` |
| Depends on | OPT-1, MIGRATE-1 |
| Coverage | 1 row in section 11 |
| Status | open |

**Goal.** Ops that repeat identically across layers and touch only their own layer's state (cache inserts after a
chunked prefill, state zeroing) run as one launch over a device array of per-layer pointers.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM inserts every layer's latent cache rows in one launch
  (`external/competitors/vllm/csrc/cache_kernels.cu` · `concat_and_cache_mla_grouped_kernel(`).

**RED tests.**
- [ ] `lane_compile_lock::a_per_layer_op_independent_of_its_layers_neighbours_runs_once_for_all_layers`

**Edge cases.**
- [ ] Layers of different widths.
- [ ] Layers of different kinds (hybrid models).

**Success.**
- [ ] RED green.

**Done when.** Layer-independent ops are batched across layers.

#### PARITY-HOST-RESIDENT-KV · KV rows read from pinned host memory

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `primitive` `cuda` `model:qwen35` |
| Depends on | MIGRATE-1 |
| Coverage | 3 rows in section 11 |
| Status | open |

**Goal.** The residence planner (`crates/logicaffeine_compile/src/codegen_cuda/lane/residence.rs`) may place cache
blocks in mapped host memory, as it already places matrices. This is for long contexts of the 27B on the 3060.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM reads cache rows zero-copy from pinned host memory
  (`external/competitors/vllm/csrc/cache_kernels.cu` · `cp_gather_and_upconvert_fp8_kv_cache_page(`).

**RED tests.**
- [ ] `lane_compile_lock::a_cache_past_the_device_lives_in_host_memory_in_the_order_a_step_misses_it_least`
- [ ] `cuda_lane_lock::a_cache_in_host_memory_computes_what_it_computes_on_the_device`

**Edge cases.**
- [ ] The PCIe x4 link: measure, and report the cost in the fusion report.

**Success.**
- [ ] RED green.

**Done when.** Context length is not capped by device memory.

#### PARITY-HOST-WEIGHT-STREAMING · Matrices past the card streamed to it once a chunk

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `primitive` `cuda` `model:qwen35` |
| Depends on | MIGRATE-1 |
| Status | open |

**Goal.** A prefill chunk reads each matrix the residence planner placed in mapped host memory once, not once a
128-row tile: the copy engine streams it into one of two device slots while the matrix before it multiplies, and the
product reads the slot. Qwen3.8-27B's 512-token chunk on the 3060 read its 5.4 GB past the card four times over PCIe
x4 (23.8 tok/s against llama.cpp's 93.7: logs/fusion-k09/window91/); the link moves 3.7–4.0 GB/s to the copy engine
and 2.2–2.6 to a kernel's mapped reads (q4kb16/pcie).

**Copy.** Bar: C-3060-QWEN38-27B-P512-Q4.
- llama.cpp offloads an op's weights to the card when its batch is wide enough (`ggml_backend_sched` op offload).

**RED tests.**
- [ ] `lane_compile_lock::a_lane_streams_each_matrix_it_reads_whole_from_host_memory_through_two_slots`
- [ ] `cuda_lane_lock::a_lane_streaming_its_moved_matrices_computes_what_it_computes_on_the_device`
- [ ] `cuda_lane_lock::a_lanes_graphs_take_no_more_memory_than_its_spare`

**Edge cases.**
- [ ] A matrix past a slot's size stays on the device or is streamed in parts.
- [ ] The graphs capture the copies and their events; the slots come out of the device budget.

**Success.**
- [ ] RED green; C-3060-QWEN38-27B-P512-Q4 past llama.cpp's fastest.

**Done when.** A chunk's bytes over the link are its moved matrices' bytes, once.

#### PARITY-HOST-LAYERS-ON-CPU · The layers past the card on the CPU tier

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `primitive` `cuda` `cpu` `model:qwen35` |
| Depends on | MIGRATE-1, PARITY-CPU-SHIM |
| Status | open |

**Goal.** A decode step whose matrices do not fit the card computes the layers past it on the CPU tier from host
memory, the residual crossing the link twice a step, as llama.cpp's partial offload does: DDR5 feeds the CPU ~60
GB/s where PCIe x4 feeds the card 2.2–4.0. Qwen3.8-27B's D1 with its matrices past the card streamed over the link a
token decodes 0.93 tok/s against llama.cpp's 1.30 at -ngl 32 (logs/fusion-k09/window92/).

**Copy.** Bar: C-3060-QWEN38-27B-D1-Q4.
- llama.cpp's `-ngl n` puts the last n − 1 layers and the output head on the card and runs the layers before them on
  the CPU backend (`i_gpu_start = n_layer + 1 − n_gpu_layers`, `src/llama-model.cpp`).

**RED tests.**
- [ ] `cpu_shim_lock::a_step_split_between_the_card_and_the_cpu_computes_what_the_evaluator_computes`

**Edge cases.**
- [ ] The split chosen by the residence planner's budget, the CPU tier's caches and states in host memory.

**Success.**
- [ ] RED green; C-3060-QWEN38-27B-D1-Q4 past llama.cpp's fastest.

**Done when.** A decode step reads no matrix over the link.

#### PARITY-ROUTER-SHARED-GATE-MERGE · Router and shared-expert gate as one GEMM

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `fusion` `cuda` `area:moe` |
| Depends on | OPT-1, MIGRATE-1 |
| Status | open |

**Goal.** The router's logits and the shared expert's gate come from one GEMM over concatenated weights. This is a
horizontal merge, like `merged_qkv`.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM (`external/competitors/vllm/vllm/model_executor/layers/fused_moe/runner/moe_runner.py` · `def _maybe_fuse_gate_weights(`).

**RED tests.**
- [ ] `lane_compile_lock::a_router_and_its_shared_gate_are_one_gemm`

**Edge cases.**
- [ ] An f32 router beside a quantized gate: not merged.

**Success.**
- [ ] RED green.

**Done when.** Qwen-MoE-style routers cost one GEMM.

#### PARITY-GROUPED-TOPK · Group-limited routing in one kernel

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `primitive` `cuda` `area:moe` |
| Depends on | MIGRATE-1 |
| Coverage | 1 row in section 11 |
| Status | open |

**Goal.** One kernel computes, in order:
- the expert scores;
- the correction bias;
- the group scores (the sum of the top two biased scores per group);
- the top groups;
- the top-k inside them;
- the renormalized, scaled weights from the unbiased scores.

This is DeepSeek V3's routing.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM (`external/competitors/vllm/csrc/moe/grouped_topk_kernels.cu` · `grouped_topk_fused_kernel(`).

**RED tests.**
- [ ] `lane_compile_lock::a_group_limited_router_is_one_launch`
- [ ] `cuda_lane_lock::the_grouped_router_routes_as_the_reference_does`

**Edge cases.**
- [ ] Ties within a group.
- [ ] One group.
- [ ] Every group selected.

**Success.**
- [ ] RED green.

**Done when.** DeepSeek V3-style routing is one launch.

#### PARITY-ROUTING-VARIANTS · Sigmoid with bias and groups, softplus-sqrt, hash routing

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `primitive` `cuda` `area:moe` |
| Depends on | MIGRATE-1 |
| Coverage | 19 rows in section 11 |
| Status | open |

**Goal.**
- Every scoring function vLLM's routers ship: softmax, sigmoid, sqrt(softplus).
- Correction biases.
- Hash routing from a table.
- Routed scaling.

Each is expressible in the language and linked on CUDA.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's biased router (`external/competitors/vllm/csrc/moe/topk_softmax_kernels.cu` · `topkGating(`) and its
  softplus-sqrt and hash variants.

**RED tests.**
- [ ] `tensor_moe_routing_lock::every_scoring_function_desugars_and_the_vm_runs_it`
- [ ] `cuda_lane_lock::every_scoring_function_routes_as_the_reference_does`

**Edge cases.**
- [ ] NaN scores, clamped as vLLM does so expert ids never repeat.

**Success.**
- [ ] RED green.

**Done when.** No router vLLM runs is refused.

#### PARITY-MASKED-EXPERT-ACTIVATION · Batched experts that skip padded rows

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `fusion` `cuda` `area:moe` |
| Depends on | SERVE-CONTINUOUS-BATCHING |
| Coverage | 2 rows in section 11 |
| Status | open |

**Goal.** In a batched expert layout ([experts, max tokens, width]), the activation skips rows past each expert's valid
count. The count is read on the device, so the graph stays static.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM (`external/competitors/vllm/csrc/activation_kernels.cu` · `masked_act_and_mul_kernel(`).

**RED tests.**
- [ ] `lane_compile_lock::a_batched_expert_activation_reads_its_valid_count_on_the_device`

**Edge cases.**
- [ ] An expert with zero valid rows.

**Success.**
- [ ] RED green.

**Done when.** Padded expert rows cost nothing.

#### PARITY-ACTIVATION-KINDS · Every activation vLLM ships

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `primitive` `cuda` `spatial` |
| Depends on | OPT-1, MIGRATE-1 |
| Coverage | 17 rows in section 11 |
| Status | open |

**Goal.** Each of these desugars, runs on the VM and links on CUDA and spatial:
- swigluoai interleaved;
- situ;
- fatrelu;
- gelu_quick, gelu_new and gelu_fast;
- xIELU;
- relu²;
- the Gaussian-threshold sparse GELU.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM (`external/competitors/vllm/csrc/activation_kernels.cu` · `__global__ void swigluoai_and_mul_kernel(`) and
  `external/competitors/vllm/vllm/model_executor/layers/activation.py`.

**RED tests.**
- [ ] `tensor_activation_kinds_lock::every_activation_desugars_and_the_vm_runs_it`
- [ ] `cuda_lane_lock::every_activation_computes_what_the_reference_computes`

**Edge cases.**
- [ ] The interleaved gate/up layout.
- [ ] Clamps at their limits.

**Success.**
- [ ] RED green.

**Done when.** No activation in a shipped checkpoint is refused.

#### PARITY-ROPE-VARIANTS · Every rotary scaling vLLM ships

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `primitive` `cuda` `spatial` `d5005` |
| Depends on | IR-2 |
| Coverage | 4 rows in section 11 |
| Status | open |

**Goal.** Each of these is computed by the one angle function (`Rope::rotation`) and held to `model::reference`:
- llama3 frequency smoothing;
- long-RoPE short and long factors;
- NTK and dynamic NTK;
- YaRN with DeepSeek's magnitude;
- mrope and IMRoPE;
- dual-chunk;
- FoPE.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's rotary embedding directory (`external/competitors/vllm/vllm/model_executor/layers/rotary_embedding/llama3_rope.py` · `low_freq_wavelen = self.orig_max_position / self.low_freq_factor`).

**RED tests.**
- [ ] `model_reference_lock::every_rotary_scaling_matches_its_published_formula`
- [ ] `tensor_rope_variants_lock::every_rotary_scaling_desugars_and_the_vm_runs_it`

**Edge cases.**
- [ ] Context past the original training length.
- [ ] Partial rotation with scaling.

**Success.**
- [ ] RED green.

**Done when.** Every rotary scheme is one angle function, held to the reference.

#### PARITY-NORM-WEIGHT-OFFSET · Gemma's (1 + w) inside the norm

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `fusion` `cuda` `area:norm` |
| Depends on | OPT-1, MIGRATE-1 |
| Status | open |

**Goal.**
- First find out whether our GGUF checkpoints store w or w + 1, and record it.
- Where they store w, the norm reads (1 + w) inside the kernel.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM (`external/competitors/vllm/vllm/model_executor/layers/layernorm.py` · `class GemmaRMSNorm(`).

**RED tests.**
- [ ] `model_reference_lock::gemmas_norm_weight_is_read_as_the_checkpoint_stores_it`

**Success.**
- [ ] RED green.

**Done when.** No Gemma checkpoint pays a materialized w + 1.

#### PARITY-AFFINE-INPUT-FOLD · Input normalization folded into one affine

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `fusion` `cuda` `spatial` |
| Depends on | OPT-4 |
| Coverage | 1 row in section 11 |
| Status | open |

**Goal.** (x·rescale − mean) / std, per channel, folds into one multiply-add at compile time (an e-graph rewrite), and
then into the first projection's weights where they are constant.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM (`external/competitors/vllm/vllm/model_executor/layers/fusion/mm_input_norm.py` · `class FusedMMInputNorm(`).

**RED tests.**
- [ ] `tensor_egraph_lock::an_input_normalization_folds_into_the_next_projections_weights`

**Success.**
- [ ] RED green.

**Done when.** Vision towers pay no separate input normalization.

#### PARITY-ATTN-RESIDUAL-NORM · Attention over block residuals with the norm

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `fusion` `cuda` |
| Depends on | OPT-1, MIGRATE-1 |
| Coverage | 2 rows in section 11 |
| Status | open |

**Goal.** An online softmax over up to eight block residuals, the residual and an RMSNorm, in one kernel. This is
Kimi-K3's attention-residual layer.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM (`external/competitors/vllm/csrc/kimi_k3/attn_res_kernel.cu` · `attn_res_fwd_online_v2_kernel(`).

**RED tests.**
- [ ] `lane_compile_lock::an_attention_over_block_residuals_is_one_launch`

**Success.**
- [ ] RED green.

**Done when.** The attention-residual block is one launch.

#### PARITY-SELECTIVE-SCAN · Mamba1 selective scan

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `primitive` `cuda` `spatial` `area:recurrent` |
| Depends on | IR-2 |
| Coverage | 1 row in section 11 |
| Status | open |

**Goal.** The language writes a selective scan: softplus(Δ + bias), A, B, C, D skip, z gate. `model::author` writes
Mamba1 models. The CUDA link scans in blocks.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM (`external/competitors/vllm/csrc/mamba/selective_scan_fwd.cu` · `selective_scan_fwd_kernel(`): a block scan on
  (a, b) pairs with the prefix carried in shared memory.

**RED tests.**
- [ ] `tensor_selective_scan_lock::the_scan_desugars_and_the_vm_runs_it`
- [ ] `cuda_lane_lock::the_selective_scan_computes_what_the_reference_computes`

**Edge cases.**
- [ ] Variable-length sequences.
- [ ] State cache indices.

**Success.**
- [ ] RED green.

**Done when.** Mamba1 models bring up through the one program.

#### PARITY-SSD-CHUNK-SCAN · Mamba2 chunked scan

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `primitive` `cuda` `spatial` `area:recurrent` |
| Depends on | IR-2 |
| Coverage | 18 rows in section 11 |
| Status | open |

**Goal.** The Mamba2 state-space duality in chunks: cumulative sums, chunk states, state passing, C·Bᵀ, chunk scan.
Expressible in the language, written by the author for Mamba2-family models, and linked on CUDA.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM (`external/competitors/vllm/vllm/model_executor/layers/mamba/ops/ssd_combined.py` · `def mamba_chunk_scan_combined_varlen(`).

**RED tests.**
- [ ] `tensor_ssd_lock::the_chunked_scan_desugars_and_the_vm_runs_it`
- [ ] `cuda_lane_lock::the_chunked_scan_computes_what_the_scan_a_row_at_a_time_computes`

**Success.**
- [ ] RED green.

**Done when.** Mamba2 models bring up through the one program.

#### PARITY-KDA · Kimi delta attention

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `primitive` `cuda` `area:recurrent` |
| Depends on | PARITY-GDN-DECODE-WHOLE |
| Coverage | 14 rows in section 11 |
| Status | open |

**Goal.** The delta rule with a per-dimension decay, its fused gate, and chunked and recurrent forms. This generalizes
our gated delta rule, whose decay is per head.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's FLA kernels (`external/competitors/vllm/vllm/third_party/flash_linear_attention/ops/kda.py` · `def chunk_kda_with_fused_gate(`).
- vLLM's fused decode (`external/competitors/vllm/csrc/kimi_k3/fused_kda_decode_kernel.cu`).

**RED tests.**
- [ ] `tensor_kda_lock::a_per_dimension_decay_desugars_and_the_vm_runs_it`
- [ ] `cuda_lane_lock::the_kda_chunk_computes_what_the_recurrence_computes`

**Success.**
- [ ] RED green.

**Done when.** Kimi Linear brings up through the one program.

#### PARITY-LIGHTNING-ATTENTION · Block linear attention with decay

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `primitive` `cuda` `area:recurrent` |
| Depends on | IR-2 |
| Coverage | 3 rows in section 11 |
| Status | open |

**Goal.** Linear attention with per-head decay slopes, as diagonal blocks plus a decayed block-KV prefix. Expressible
and linked.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM (`external/competitors/vllm/vllm/model_executor/layers/lightning_attn.py` · `def lightning_attention(`).

**RED tests.**
- [ ] `tensor_linear_attention_lock::lightning_attention_desugars_and_the_vm_runs_it`

**Success.**
- [ ] RED green.

**Done when.** MiniMax-Text-01-style layers bring up.

#### PARITY-SHORT-CONV · Gated short convolution

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `primitive` `cuda` `area:recurrent` |
| Depends on | IR-2 |
| Coverage | 2 rows in section 11 |
| Status | open |

**Goal.** B·x, a causal conv, then C·out. Expressible and linked, reusing the causal-conv machinery.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM (`external/competitors/vllm/vllm/model_executor/layers/mamba/short_conv.py` · `class ShortConv(`).

**RED tests.**
- [ ] `tensor_short_conv_lock::the_short_conv_desugars_and_the_vm_runs_it`

**Success.**
- [ ] RED green.

**Done when.** LFM2-style layers bring up.

#### PARITY-MIXED-BATCH-ATTENTION · Prefill and decode rows in one attention launch

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `primitive` `cuda` `serving` |
| Depends on | SERVE-PAGED-KV |
| Coverage | 4 rows in section 11 |
| Status | open |

**Goal.** One attention launch serves a batch mixing prefill chunks and decode rows over a paged cache.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's unified attention finds each query's sequence by binary search and bounds the grid without a host sync
  (`external/competitors/vllm/vllm/v1/attention/ops/triton_unified_attention.py` · `def kernel_unified_attention(`).

**RED tests.**
- [ ] `cuda_lane_lock::a_mixed_batch_attends_as_its_sequences_attend_alone`

**Success.**
- [ ] RED green.

**Done when.** Mixed batches cost one attention launch.

#### PARITY-CPU-SHIM · A CPU shim, its link and its kernels

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `infrastructure` `cpu` |
| Depends on | OPT-2, MIGRATE-1, SETUP-7 |
| Touches | `tensor_graph::shim` (a new `CpuShim`); a CPU link emitting C or Rust through the existing AOT backends; `crates/logicaffeine_compile/src/bringup.rs` (a CPU backend) |
| Coverage | 99 rows in section 11 |
| Status | open |

**Goal.** The one IR runs on CPUs (x86, Arm and RISC-V) through a shim and a link, as it does on every other chip.
- `CpuShim` answers for x86 (AVX2, AVX-VNNI, AVX-512 with VNNI and BF16, AMX), Arm (NEON with dotprod and i8mm, SVE)
  and RISC-V (the vector extension, RVV, through the RV64GC tier).
- The link emits vectorized C or Rust through the AOT backends the compiler already has.
- `largo bringup` gains a CPU target.
- Every vLLM CPU op in section 11.2 is matched, and the CPU cells of section 2.4 are won against llama.cpp's and vLLM's
  CPU backends.

**Copy.**
- llama.cpp's int8 dot products: `external/competitors/llama.cpp/ggml/src/ggml-cpu/arch/x86/quants.c` · `void ggml_vec_dot_q8_0_q8_0(`
  and `void ggml_vec_dot_q4_K_q8_K(`.
- llama.cpp's repacked GEMV and GEMM: `external/competitors/llama.cpp/ggml/src/ggml-cpu/arch/x86/repack.cpp` · `void ggml_gemv_q4_0_8x8_q8_0(`.
- vLLM's oneDNN GEMM: `external/competitors/vllm/csrc/cpu/dnnl_kernels.cpp` · `void onednn_mm(`.
- Bar: the row maximum on the i9-14900K for every op class of section 2.5.

**Beyond.**
- The shared fusions run on the CPU unchanged. A decode layer's norm, quantize and GEMV share one pass over cache
  lines that llama.cpp's graph makes in separate ops.
- Shapes are known when the program is compiled, so every loop is specialized and unrolled at compile time. llama.cpp
  dispatches at run time.

**Read first.**
- `crates/logicaffeine_compile/src/codegen/simd_reduce.rs` (AVX2 emission today, with its byte-identity argument);
- `crates/logicaffeine_compile/src/compile.rs` (`pub enum Backend`);
- `crates/logicaffeine_compile/src/bringup.rs`.

**RED tests.**
- [ ] `cpu_shim_lock::every_class_has_a_cpu_answer`: fails today because there is no `CpuShim`.
- [ ] `cpu_shim_lock::the_cpu_link_computes_what_the_evaluator_computes_on_every_tiny_cut`
- [ ] `cpu_shim_lock::the_emitted_kernels_use_the_isa_the_shim_declares`: a census of intrinsics per ISA.
- [ ] `bringup_lock::the_cpu_backend_brings_up_every_model_or_refuses_by_name`

**Edge cases.**
- [ ] A host without the declared ISA. The program refuses at load, by name, never with an illegal instruction.
- [ ] Hybrid cores (performance and efficiency) on the 14900K: thread placement is recorded with each run.
- [ ] Bit-identity under reassociation. The shim declares it (OPT-6), and the bound holds.
- [ ] Memory bandwidth bound: DDR4-2133 dual channel on this box. Floors are computed per cell.

**Implementation.**
- [ ] `CpuShim` with its numerics, limits and costs.
- [ ] Links for the row kernels, GEMV, GEMM, attention, MoE and recurrent ops, in the C or Rust AOT output.
- [ ] A CPU target in `largo bringup`.

**Success.**
- [ ] Every RED test green (receipt: log paths).
- [ ] Every section 11.2 CPU row ticked (receipt: SETUP-4 CPU rows).
- [ ] Every CPU cell of section 2.4 won (receipt: sota_cells.json rows).

**Mutants to kill.**
- [ ] An AVX-512 kernel emitted for an AVX2-only host.
- [ ] A fused CPU kernel that drops the residual.

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test cpu_shim_lock --test bringup_lock
```

**Done when.** LOGOS is the fastest way to run the CPU cells' models on this box.

#### PARITY-MULTIMODAL-TOWERS · Vision and audio encoder towers

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `fusion` `cuda` `spatial` |
| Depends on | PARITY-AFFINE-INPUT-FOLD, PARITY-ROPE-VARIANTS |
| Touches | `crates/logicaffeine_compile/src/model/` (a tower in the one program); `tensor_graph::fuse`; the lane's attention kernels |
| Coverage | 7 rows in section 11 |
| Status | open |

**Goal.** A vision or audio tower is written in the model's one LOGOS program and lowered with it. Covered:
- patch embedding with interpolated position embeddings;
- the tower's attention (bidirectional, variable length);
- multimodal rotary setup;
- input normalization;
- the projector into the language model.

**Copy.**
- Interpolated position embedding: `external/competitors/vllm/vllm/model_executor/models/qwen3_vl.py` · `def _bilinear_pos_embed_kernel(`.
- The tower attention dispatch: `external/competitors/vllm/vllm/v1/attention/ops/vit_attn_wrappers.py` · `def vit_flash_attn_wrapper(`.
- Bar: the row maximum for the tower's attention at its shapes.

**Beyond.** The tower and the language model are one graph, so the projector's GEMM carries the language model's first
norm in its epilogue, and the image's patches never round-trip through the host.

**RED tests.**
- [ ] `multimodal_tower_lock::the_author_writes_a_vision_tower`: fails today because the author writes text models only.
- [ ] `multimodal_tower_lock::the_tower_computes_what_its_reference_computes`
- [ ] `lane_compile_lock::a_tower_attends_with_variable_lengths_in_one_launch`

**Edge cases.**
- [ ] Images of different sizes in one batch.
- [ ] A position grid interpolated to a size it was not trained at.
- [ ] Video frames with temporal rotary sections.

**Success.**
- [ ] Every RED test green (receipt: log paths).
- [ ] Every section 11 row mapped here ticked (receipt: SETUP-4 rows).

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test multimodal_tower_lock --test lane_compile_lock
```

**Done when.** A vision-language model runs through the one program, tower included.

#### PARITY-MODEL-SPECIFIC-LAYERS · Layers one model family uses

| Field | Value |
|---|---|
| Phase | parity, tier 2 |
| Labels | `primitive` `cuda` |
| Depends on | IR-2, OPT-1 |
| Touches | `crates/logicaffeine_language/src/parser/` (new sugar where needed); `tensor_graph::front`; the lane's kernels |
| Coverage | 6 rows in section 11 |
| Status | open |

**Goal.** Every layer that exists in one model family only is an op the IR evaluates, and a link on CUDA:
- n-gram and per-layer embedding tables, including tables read from pinned host memory;
- Engram memory: hashed row selection and its fused post-projection;
- relative attention with a query-side bias projection;
- constant and zero experts.

**Copy.**
- Per-layer embedding gates: `external/competitors/vllm/vllm/models/qwen4_exp/common/ops/ple.py` · `def _ple_gate_kernel(`.
- Engram's fused post-projection: `external/competitors/vllm/vllm/models/deepseek_v41/common/engram.py` · `def _fused_engram_post_wkv_kernel(`.
- Relative attention's projection: `external/competitors/vllm/vllm/models/inkling/nvidia/ops/qkvr_prep.py` · `def _rel_proj_low_latency_kernel(`.
- Bar: the row maximum at each family's shapes.

**Beyond.** Each layer enters as a definition in terms of existing ops (choice 3 of section 5.11), so every chip
runs it as parts on day one, and the CUDA link makes it fast.

**RED tests.**
- [ ] `model_specific_layer_lock::every_layer_evaluates_as_its_reference`: fails today because none is an op.
- [ ] `model_specific_layer_lock::every_layer_runs_as_parts_on_every_shim`
- [ ] `cuda_lane_lock::every_model_specific_layer_computes_what_its_parts_compute`

**Edge cases.**
- [ ] A hashed row that collides.
- [ ] A host-resident table read while the GPU computes the previous layer.
- [ ] A zero expert selected by every token.

**Success.**
- [ ] Every RED test green (receipt: log paths).
- [ ] Every section 11 row mapped here ticked (receipt: SETUP-4 rows).

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test model_specific_layer_lock
```

**Done when.** No layer in vLLM's model trees lacks an op in our IR.

### 12.11 Parity, tiers 3 and 4: datacenter and multi-GPU

Datacenter tickets build here and land with the `compile_verified` status:
- a compile census (the PTX or SASS carries the instructions: wgmma, tcgen05, TMA, fp8 and fp4 conversions);
- a CPU replica values test.

Their GPU values tests run when a rented sm_90 or sm_100 GPU exists. nvcc 12.0 cannot target sm_100, so these tickets
first install a newer CUDA toolkit beside it, and the CPU-only locks name that toolkit as a required tool.

#### PARITY-QUANT-SCHEMES · fp8, NVFP4, MXFP4 and MXFP8 for every quantize fold

| Field | Value |
|---|---|
| Phase | parity, tier 3 |
| Labels | `fusion` `datacenter` `area:quant` |
| Depends on | OPT-6, MIGRATE-1 |
| Coverage | 49 rows in section 11 |
| Status | open |

**Goal.** Every quantize fold carries every scheme, each as a shim-declared numerics contract:
- `norm_quant`, `add_norm_quant`, `glu_act_mul_quant`, `attn_output_quant`, `gdn_output_quant`, `kv_cache_quant`;
- schemes: fp8 static, per token, group 128 with UE8M0 and TMA-aligned scales; NVFP4 (16-element E4M3 scales with a
  global scale); MXFP4 and MXFP8 (32-element E8M0).

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's norm, activation and attention quantize patterns and kernels (section 10.2's quantize rows).
- Triton's MX conversions (PARITY-TRITON-MX-CONVERT).

**RED tests.**
- [ ] `numerics_contract_lock::every_scheme_quantizes_as_its_replica_does`: a bit-exact CPU replica per scheme.
- [ ] `lane_compile_lock::every_scheme_emits_its_conversion_instructions`

**Edge cases.**
- [ ] Saturation at the format maximum.
- [ ] A scale of zero (an all-zero block).
- [ ] UE8M0 rounding up.
- [ ] Software encoding on sm_86.

**Success.**
- [ ] RED green.
- [ ] Ledger rows at `compile_verified`.

**Done when.** Every quantize fold has every scheme, replica-exact.

#### PARITY-QUANT-GEMMS-DATACENTER · fp8, fp4 and W4A8 GEMMs, dense and grouped

| Field | Value |
|---|---|
| Phase | parity, tier 3 |
| Labels | `primitive` `datacenter` `area:gemm` |
| Depends on | PARITY-QUANT-SCHEMES, TRITON-MMA-SELECTION |
| Coverage | 72 rows in section 11 |
| Status | open |

**Goal.** Dense and grouped GEMMs for fp8 (per tensor, per token, block 128), NVFP4, MXFP4 and W4A8 on sm_90 and
sm_100, through the generated-kernel tier.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's blockwise fp8 (`external/competitors/vllm/csrc/quantization/w8a8/cutlass/c3x/scaled_mm_blockwise_sm90_fp8_dispatch.cuh` · `struct cutlass_3x_gemm_fp8_blockwise`).
- NVFP4 (`external/competitors/vllm/csrc/quantization/fp4/nvfp4_scaled_mm_kernels.cu` · `struct sm100_fp4_config_default`).
- Triton's block-scaled matmul (`external/competitors/triton/python/tutorials/10-block-scaled-matmul.py` · `def block_scaled_matmul_kernel(`).

**RED tests.**
- [ ] `cuda_tile_tier_lock::every_datacenter_gemm_emits_its_mma_instructions`
- [ ] `cuda_tile_tier_lock::every_datacenter_gemm_matches_its_cpu_replica_on_a_small_shape`

**Success.**
- [ ] RED green; `compile_verified`.
- [ ] GPU values when rented.

**Done when.** Datacenter GEMMs are compiled and replica-checked.

#### PARITY-MLA-DATACENTER · Latent attention on sm90 and sm100

| Field | Value |
|---|---|
| Phase | parity, tier 3 |
| Labels | `primitive` `datacenter` `area:attention` `model:ds2` |
| Depends on | TRITON-WARP-SPECIALIZATION |
| Coverage | 2 rows in section 11 |
| Status | open |

**Goal.** Absorbed latent attention decode on sm_100: two-SM MMAs, tensor memory, warp roles, persistent, split over the
span with its reduction.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's CUTLASS kernel (`external/competitors/vllm/csrc/attention/mla/cutlass_sm100_mla/kernel/sm100_fmha_mla_tma_warpspecialized.hpp` · `Sm100FmhaMlaKernelTmaWarpspecialized`).

**RED tests.**
- [ ] `cuda_tile_tier_lock::the_latent_decode_emits_its_tensor_memory_instructions`

**Success.**
- [ ] `compile_verified`.

**Done when.** Latent attention is compiled for datacenter parts.

#### PARITY-SPARSE-ATTENTION · Indexers, block top-k and sparse attention

| Field | Value |
|---|---|
| Phase | parity, tier 3 |
| Labels | `fusion` `datacenter` `area:attention` |
| Depends on | OPT-2 |
| Coverage | 63 rows in section 11 |
| Status | open |

**Goal.**
- The language expresses a lightning indexer (scores per key block), block top-k with forced initial and local blocks,
  and attention over the selected blocks.
- The author writes the DeepSeek V3.2 / GLM-5 / MiniMax-M3 families.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's indexer (`external/competitors/vllm/vllm/model_executor/layers/sparse_attn_indexer.py` · `def sparse_attn_indexer(`).
- vLLM's persistent and cooperative top-k (section 10.2's techniques).

**RED tests.**
- [ ] `tensor_sparse_attention_lock::a_block_sparse_attention_desugars_and_the_vm_runs_it`

**Success.**
- [ ] RED green.

**Done when.** Sparse-attention models bring up.

#### PARITY-HYPER-CONNECTIONS · Hyper-connection mixing kernels

| Field | Value |
|---|---|
| Phase | parity, tier 3 |
| Labels | `fusion` `datacenter` |
| Depends on | OPT-2 |
| Coverage | 3 rows in section 11 |
| Status | open |

**Goal.** Hyper-connection pre-mix, post and collapse, with their norms, expressible and fused (DeepSeek V4, HunYuan
V4).

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM (`external/competitors/vllm/vllm/model_executor/kernels/mhc/triton.py`).

**RED tests.**
- [ ] `tensor_hyper_connection_lock::a_hyper_connection_desugars_and_the_vm_runs_it`

**Success.**
- [ ] RED green.

**Done when.** Hyper-connection models bring up.

#### PARITY-TENSOR-PARALLEL-IR · Tensor parallelism in the IR

| Field | Value |
|---|---|
| Phase | parity, tier 4 |
| Labels | `infrastructure` `area:comm` |
| Depends on | OPT-2 |
| Coverage | 9 rows in section 11 |
| Status | open |

**Goal.** The IR has collective symbols (all-reduce, all-gather, reduce-scatter, all-to-all) whose meaning is defined
over a mesh of chips. The spatial pipeline's tensor-parallel manifests, which recognize an all-reduce as a chain of
additions, read them.

**RED tests.**
- [ ] `tensor_graph_eval_lock::a_collective_means_its_definition_over_the_mesh`

**Success.**
- [ ] RED green.

**Done when.** Multi-chip programs have a meaning in the IR.

#### PARITY-ALLREDUCE-NORM · Collectives fused with norms

| Field | Value |
|---|---|
| Phase | parity, tier 4 |
| Labels | `fusion` `area:comm` |
| Depends on | PARITY-TENSOR-PARALLEL-IR |
| Coverage | 41 rows in section 11 |
| Status | open |

**Goal.** Each of these is a shared rule with chip links:
- all-reduce + residual + RMSNorm (+ quantize);
- all-reduce of only the sum of squares;
- all-reduce + norm + MoE finalize;
- one-shot vs two-shot by size;
- Lamport flagless buffers;
- multimem reductions;
- GEMM + all-reduce;
- vocab-parallel embedding and arg-max.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's fusion pass (`external/competitors/vllm/vllm/compilation/passes/fusion/allreduce_rms_fusion.py`) and kernels
  (section 10.2's communication rows).

**RED tests.**
- [ ] `fusion_class_lock::an_all_reduce_feeding_an_add_and_norm_fuses`
- [ ] `fusion_class_lock::a_norms_all_reduce_carries_only_its_sum_of_squares`

**Success.**
- [ ] RED green.
- [ ] Values on a multi-GPU box when one exists.

**Done when.** Collectives fuse like any other op.

#### PARITY-EXPERT-PARALLEL · Expert-parallel dispatch and combine

| Field | Value |
|---|---|
| Phase | parity, tier 4 |
| Labels | `fusion` `area:comm` `area:moe` |
| Depends on | PARITY-TENSOR-PARALLEL-IR |
| Coverage | 18 rows in section 11 |
| Status | open |

**Goal.** Expert-parallel dispatch, combine and load balancing as IR collectives, with quantized dispatch.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's modular MoE prepare-and-finalize backends
  (`external/competitors/vllm/vllm/model_executor/layers/fused_moe/modular_kernel.py`).

**RED tests.**
- [ ] `fusion_class_lock::an_expert_dispatch_means_its_definition`

**Success.**
- [ ] RED green.

**Done when.** Large MoE models can be laid across chips by the compiler.

#### PARITY-XPU-SHIM · An Intel GPU shim

| Field | Value |
|---|---|
| Phase | parity, tier 3 |
| Labels | `infrastructure` |
| Depends on | OPT-2, TRITON-TILE-TIER, SETUP-8 |
| Touches | `tensor_graph::shim` (a new `XpuShim`); a SYCL or Level Zero link |
| Owner notes | needs Intel hardware: bought or rented on Tristen's word (SETUP-8) |
| Coverage | 15 rows in section 11 |
| Status | open |

**Goal.** Intel GPUs are one more shim and link.
- `XpuShim` declares the XMX matrix engines, subgroup sizes and shared local memory.
- The link emits SYCL or SPIR-V through the generated-kernel tier.
- Every XPU row of section 11 is matched, and the Arc B580 cells of section 2.4 are won.

**Copy.** vLLM's XPU op surface: `external/competitors/vllm/vllm/_xpu_ops.py` · `def _fused_add_gemma_rms_norm_impl(`
and the XPU kernels section 11 maps here. Bar: the row maximum on the card.

**Beyond.** The shared fusions arrive unchanged; only the link is new.

**RED tests.**
- [ ] `xpu_shim_lock::every_class_has_an_xpu_answer`: fails today because there is no `XpuShim`.
- [ ] `xpu_shim_lock::the_xpu_link_compiles_every_tiny_cut`: a compile census, `compile_verified` until the card exists.

**Success.**
- [ ] Every RED test green (receipt: log paths).
- [ ] The Arc B580 cells won (receipt: sota_cells.json rows).

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test xpu_shim_lock
```

**Done when.** Intel GPUs run the one IR at or past vLLM on XPU.

### 12.12 Serving

Serving tickets are runtime features. Their RED tests run on the generated program, through the `--prompts` runner the
lane already has (`cuda_lane_lock::a_program_run_over_many_prompts_gives_each_the_run_it_gives_alone`).

#### SERVE-PAGED-KV · Paged KV caches

| Field | Value |
|---|---|
| Phase | serving |
| Labels | `serving` `cuda` |
| Depends on | MIGRATE-1 |
| Coverage | 8 rows in section 11 |
| Status | open |

**Goal.**
- The cache is allocated in fixed pages through a block table.
- Each chip's shim declares the page layouts it allows: vLLM's logical [L, B, H, N, C] with stride permutations.
- Attention kernels read through the table.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's cache manager (`external/competitors/vllm/vllm/v1/core/kv_cache_manager.py` · `class KVCacheManager`) and
  layout system (`external/competitors/vllm/vllm/v1/kv_cache_layout.py` · `class KVCacheLayout(`).

**Beyond.** The page layout is chosen by the shim's cost per chip, and the block table lives in the arena.

**RED tests.**
- [ ] `cuda_lane_lock::a_paged_cache_attends_as_the_contiguous_cache_does`: bit for bit.
- [ ] `lane_compile_lock::every_attention_kernel_reads_through_the_block_table`

**Edge cases.**
- [ ] A page boundary inside an attention tile.
- [ ] The last page partly filled.
- [ ] The page size against the tile size.

**Success.**
- [ ] RED green.

**Done when.** Caches are paged with no change in what is computed.

#### SERVE-CONTINUOUS-BATCHING · Requests join and leave every step

| Field | Value |
|---|---|
| Phase | serving |
| Labels | `serving` `cuda` |
| Depends on | SERVE-PAGED-KV |
| Coverage | 7 rows in section 11 |
| Status | open |

**Goal.**
- The generated program serves a changing set of sequences.
- The decode form compiles for a batch, using the author's `decode_batch` and the `Cut::Batch` cut.
- Sequences join and leave between steps.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's scheduler (`external/competitors/vllm/vllm/v1/core/sched/scheduler.py` · `class Scheduler`).

**Beyond.** Every batch form is a cut of the one program, and the batch's bit-identity to each sequence run alone is
locked.

**RED tests.**
- [ ] `cuda_lane_lock::a_batch_of_sequences_generates_what_each_generates_alone`
- [ ] `one_program_lock::every_lane_form_names_the_cut_of_the_one_program_it_compiled`, extended to batch forms.

**Edge cases.**
- [ ] A sequence finishing mid-batch.
- [ ] Batch 1.
- [ ] Sequences of different lengths.

**Success.**
- [ ] RED green.
- [ ] Throughput at batch 8 and 32 measured against vLLM (receipt: SETUP-5 row).

**Done when.** The generated program batches like a server.

#### SERVE-CHUNKED-PREFILL · Long prompts in chunks beside decode

| Field | Value |
|---|---|
| Phase | serving |
| Labels | `serving` `cuda` |
| Depends on | SERVE-CONTINUOUS-BATCHING, PARITY-MIXED-BATCH-ATTENTION |
| Coverage | 1 row in section 11 |
| Status | open |

**Goal.** A long prompt's prefill is chunked and interleaved with other sequences' decode steps, in mixed batches.

**RED tests.**
- [ ] `cuda_lane_lock::a_prompt_prefilled_beside_decoding_sequences_continues_as_alone`

**Success.**
- [ ] RED green.

**Done when.** Prefill does not stall decode.

#### SERVE-PREFIX-CACHE · Shared prefixes reuse their KV

| Field | Value |
|---|---|
| Phase | serving |
| Labels | `serving` `cuda` |
| Depends on | SERVE-PAGED-KV |
| Status | open |

**Goal.** Pages holding a shared prompt prefix are reused across sequences, through a radix structure.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- SGLang's radix cache (the ledger's `prefix_cache` citation).

**RED tests.**
- [ ] `cuda_lane_lock::a_reused_prefix_continues_as_a_freshly_prefilled_one`

**Success.**
- [ ] RED green.

**Done when.** Repeated prefixes cost no prefill.

#### SERVE-SPEC-DECODE · Speculative decoding, MTP first

| Field | Value |
|---|---|
| Phase | serving |
| Labels | `serving` `cuda` `model:qwen35` |
| Depends on | SERVE-CONTINUOUS-BATCHING, PARITY-SMALL-M-GEMM |
| Coverage | 38 rows in section 11 |
| Status | open |

**Goal.**
- Qwen3.8 ships a multi-token-prediction head, so MTP comes first: the draft head is a cut of the one program, and
  verification runs a multi-row step. The GDN state is kept per token and rolled back to the last accepted token.
- Then EAGLE and EAGLE3, n-gram proposals, suffix trees, parallel drafting, and the rejection sampler on the device.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's EAGLE proposer (`external/competitors/vllm/vllm/v1/spec_decode/llm_base_proposer.py` · `def propose(`).
- vLLM's n-gram proposer (`external/competitors/vllm/vllm/v1/spec_decode/ngram_proposer.py` · `def _find_longest_matched_ngram_and_propose_tokens(`).
- vLLM's rejection sampler (`external/competitors/vllm/vllm/v1/sample/rejection_sampler.py` · `def rejection_sample(`).
- vLLM's MTP GDN decode (`external/competitors/vllm/csrc/gdn/fused_gdn_decode_kernel.cu` · `gdn_decode_post_conv_mtp_kernel(`).

**Beyond.** Greedy speculative decoding generates exactly what plain decoding generates. That is locked bit for bit, so
speculation never changes an answer.

**RED tests.**
- [ ] `cuda_lane_lock::greedy_speculation_generates_what_plain_decoding_generates`
- [ ] `cuda_lane_lock::a_rejected_draft_restores_the_recurrent_state_it_started_from`

**Edge cases.**
- [ ] Every draft rejected.
- [ ] Every draft accepted.
- [ ] A draft longer than the remaining context.

**Success.**
- [ ] RED green.
- [ ] Measured speedup at batch 1 recorded (receipt).

**Done when.** Qwen3.8 decodes with its MTP head and never changes its greedy answer.

#### SERVE-PER-BATCH-GRAPHS · One captured graph per batch size

| Field | Value |
|---|---|
| Phase | serving |
| Labels | `serving` `cuda` |
| Depends on | SERVE-CONTINUOUS-BATCHING |
| Status | open |

**Goal.** One CUDA graph per batch-size bucket, captured once, with uniform decode widths for speculation.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's graph wrapper and dispatcher (`external/competitors/vllm/vllm/config/compilation.py` · `class CUDAGraphMode(`).

**RED tests.**
- [ ] `lane_compile_lock::every_batch_bucket_captures_its_own_graph`

**Success.**
- [ ] RED green.

**Done when.** Batched decode replays graphs.

#### SERVE-EXPERT-OFFLOAD · Experts computed on the CPU

| Field | Value |
|---|---|
| Phase | serving |
| Labels | `serving` `cuda` |
| Depends on | MIGRATE-1 |
| Owner notes | CPU-computed experts cannot be bit-identical to the GPU (the device's exp is approximate), so this is a numerics knob; Tristen rules on its bar |
| Status | open |

**Goal.** For models past device memory, host-resident experts are computed on the CPU, beside a GPU-resident model.
This is the regime llama.cpp's `--n-cpu-moe` serves.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- llama.cpp's `n-cpu-moe` (the ledger's `expert_offload` citation).

**RED tests.**
- [ ] `cuda_lane_lock::a_model_with_cpu_experts_meets_its_declared_bar`

**Success.**
- [ ] RED green.

**Done when.** The 27B-class MoE models run on this box at a declared bar.

#### SERVE-KV-OFFLOAD · KV blocks swapped to host memory

| Field | Value |
|---|---|
| Phase | serving |
| Labels | `serving` `cuda` |
| Depends on | SERVE-PAGED-KV |
| Coverage | 4 rows in section 11 |
| Status | open |

**Goal.** Cold KV pages swap to host memory in one batched copy, and swap back on demand.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's batched swap (`external/competitors/vllm/csrc/cache_kernels.cu`, `swap_blocks_batch`).
- vLLM's Triton swap kernel (`external/competitors/vllm/vllm/v1/kv_offload/cpu/swap_blocks_triton.py`).

**RED tests.**
- [ ] `cuda_lane_lock::a_swapped_out_page_reads_back_what_was_written`

**Success.**
- [ ] RED green.

**Done when.** Contexts can exceed device memory by paging.

#### SERVE-STRUCTURED-OUTPUT · Grammar-constrained decoding

| Field | Value |
|---|---|
| Phase | serving |
| Labels | `serving` `cuda` |
| Depends on | PARITY-SAMPLING |
| Coverage | 4 rows in section 11 |
| Status | open |

**Goal.** A grammar's per-step token mask is applied on the device, for JSON schemas and regular expressions.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM (`external/competitors/vllm/vllm/v1/worker/gpu/structured_outputs.py` · `def _apply_grammar_bitmask_kernel(`).

**RED tests.**
- [ ] `device_sampling_lock::a_grammar_constrained_run_only_emits_allowed_tokens`

**Success.**
- [ ] RED green.

**Done when.** Constrained output costs one masked read.

#### SERVE-LORA · Adapters, including inside the experts

| Field | Value |
|---|---|
| Phase | serving |
| Labels | `serving` `cuda` |
| Depends on | SERVE-CONTINUOUS-BATCHING |
| Coverage | 13 rows in section 11 |
| Status | open |

**Goal.** Low-rank adapters per sequence: shrink and expand kernels, including inside the expert GEMMs.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's Punica-style kernels (`external/competitors/vllm/vllm/lora/ops/triton_ops/fused_moe_lora_op.py`).

**RED tests.**
- [ ] `cuda_lane_lock::an_adapted_sequence_generates_what_its_merged_weights_generate`

**Success.**
- [ ] RED green.

**Done when.** Adapters serve per sequence.

#### SERVE-DUAL-BATCH-OVERLAP · Compute and communication overlapped across microbatches

| Field | Value |
|---|---|
| Phase | serving |
| Labels | `serving` `area:comm` |
| Depends on | PARITY-EXPERT-PARALLEL |
| Status | open |

**Goal.** Two microbatches alternate between compute and communication streams, for expert-parallel serving.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's microbatching (`external/competitors/vllm/vllm/v1/worker/ubatching.py` · `class UBatchContext:`).

**RED tests.**
- [ ] `fusion_class_lock::two_microbatches_overlapped_compute_what_one_batch_computes`

**Success.**
- [ ] RED green.

**Done when.** Communication hides behind compute.

#### SERVE-POOLING · Embedding, reranking and pooling models

| Field | Value |
|---|---|
| Phase | serving |
| Labels | `serving` `cuda` |
| Depends on | SERVE-CONTINUOUS-BATCHING |
| Touches | `crates/logicaffeine_compile/src/model/` (pooling heads in the one program); the serving runtime |
| Coverage | 1 row in section 11 |
| Status | open |

**Goal.** Embedding and reranking models serve through the same runtime as generation:
- pooling by mean, first token or last token;
- normalization;
- classification heads;
- late-interaction MaxSim reranking.

**Copy.** vLLM's MaxSim rerank kernel: `external/competitors/vllm/vllm/v1/pool/flash_maxsim/flash_maxsim_rerank.py` · `def _maxsim_rerank_kernel(`.
Bar: the row maximum at our shapes, and vLLM's pooling throughput on the S trace's prompts.

**Beyond.** The pooling and normalization fold into the last layer's epilogue, so no hidden-state matrix is written
for a pooled request.

**RED tests.**
- [ ] `serving_pooling_lock::every_pooling_kind_computes_what_its_reference_computes`: fails today because the author
  writes no pooling head.
- [ ] `serving_pooling_lock::a_pooled_request_writes_no_hidden_states`
- [ ] `serving_pooling_lock::maxsim_reranks_as_its_reference_ranks`

**Edge cases.**
- [ ] A batch mixing pooled and generating requests.
- [ ] Documents longer than one chunk.
- [ ] Ties in MaxSim scores (a stable order).

**Success.**
- [ ] Every RED test green (receipt: log paths).
- [ ] Throughput at or past vLLM's on one boot (receipt: sota_cells.json row).

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test serving_pooling_lock
```

**Done when.** Embedding and reranking requests run at or past vLLM on the same server as generation.

### 12.13 Graph-level

#### GRAPH-CANONICALIZE · Canonicalize views and reshapes

| Field | Value |
|---|---|
| Phase | graph |
| Labels | `infrastructure` `area:graph` |
| Depends on | OPT-1 |
| Coverage | 2 rows in section 11 |
| Status | open |

**Goal.** No-op views, reshape chains and split coalescing are canonicalized by the shared optimizer before the rules
run, so patterns match regardless of how a program spells a view.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's no-op elimination (`external/competitors/vllm/vllm/compilation/passes/utility/noop_elimination.py` · `class NoOpEliminationPass`).

**RED tests.**
- [ ] `fusion_class_lock::a_rule_fires_through_a_no_op_view`

**Success.**
- [ ] RED green.

**Done when.** Spelling never blocks a fusion.

#### GRAPH-IN-PLACE · In-place destinations as a recorded property

| Field | Value |
|---|---|
| Phase | graph |
| Labels | `infrastructure` `area:graph` |
| Depends on | OPT-1 |
| Coverage | 4 rows in section 11 |
| Status | open |

**Goal.**
- vLLM restores in-place writes after functionalization (`external/competitors/vllm/vllm/compilation/passes/utility/fix_functionalization.py` · `class FixFunctionalizationPass`).
- Ours writes in place by construction: row and unit destinations.
- Record it as a ledger row with its receipts.

**RED tests.**
- [ ] `fusion_class_lock::every_in_place_destination_is_a_shared_layout`

**Success.**
- [ ] Ledger row with receipts.

**Done when.** The property is on record.

#### GRAPH-PER-SHAPE-FORMS · Per-shape forms, recorded

| Field | Value |
|---|---|
| Phase | graph |
| Labels | `infrastructure` `area:graph` |
| Depends on | LEDGER-1 |
| Status | open |

**Goal.** vLLM compiles per token range (`external/competitors/vllm/vllm/config/compilation.py`, `get_compile_ranges`).
Ours compiles one form per chunk size. Record the correspondence as a ledger row.

**Success.**
- [ ] Ledger row with receipts.

**Done when.** The correspondence is on record.

#### GRAPH-CUDA-GRAPH-MODES · Full, piecewise and breakable capture

| Field | Value |
|---|---|
| Phase | graph |
| Labels | `infrastructure` `cuda` `area:graph` |
| Depends on | MIGRATE-1 |
| Coverage | 1 row in section 11 |
| Status | open |

**Goal.** The lane captures full graphs today. It gains piecewise capture (eager regions between captured pieces) and
breakable capture, for ops that cannot be captured: host-dependent shapes during serving.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM's graph modes and breakable capture (`external/competitors/vllm/vllm/compilation/breakable_cudagraph.py` · `class BreakableCUDAGraphCapture`).

**RED tests.**
- [ ] `lane_compile_lock::a_form_with_an_uncapturable_op_captures_the_pieces_around_it`

**Success.**
- [ ] RED green.

**Done when.** Every form replays as much as it can.

#### GRAPH-GRAPH-MEMORY-POOL · One pool for every captured graph

| Field | Value |
|---|---|
| Phase | graph |
| Labels | `infrastructure` `cuda` `area:graph` |
| Depends on | GRAPH-CUDA-GRAPH-MODES |
| Status | open |

**Goal.** All captured graphs share one memory pool, so many batch-size graphs do not multiply memory.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- vLLM (`external/competitors/vllm/vllm/compilation/cudagraph_pool.py`).

**RED tests.**
- [ ] `lane_compile_lock::every_captured_graph_draws_from_one_pool`

**Success.**
- [ ] RED green.

**Done when.** Graph memory is shared.

#### GRAPH-CROSS-KERNEL-OVERLAP · Kernel overlap decided by the compiler

| Field | Value |
|---|---|
| Phase | graph |
| Labels | `infrastructure` `cuda` `area:graph` |
| Depends on | MIGRATE-1, TECH-PDL |
| Status | open |

**Goal.**
- From the IR's dependencies, the compiler decides where a kernel's prologue may overlap its predecessor's tail (PDL)
  and where independent kernels share streams.
- The lane's four-stream branches already do the second part.
- Triton leaves PDL to the programmer (its tutorial 11).

**RED tests.**
- [ ] `lane_compile_lock::every_overlapped_launch_waits_exactly_on_what_it_reads`

**Success.**
- [ ] RED green.

**Done when.** Overlap is the compiler's decision, proven safe from dependencies.

### 12.14 Techniques

Every technique ticket has the same shape:
- a kernel-level census proves the technique is in the emitted code: a PTX or SASS instruction, a launch attribute, or
  `ptxas -v` stack and spill counts;
- a SETUP-4 row measures it against the kernel it came from and the row maximum of section 2.5, at our shapes;
- a values test shows nothing changed in what is computed.

- [ ] **TECH-ALIGNMENT-AWARE-VECTORIZATION**: a scalar prefix up to alignment, a vector body, a scalar tail, checked on input and output together. Copy from `external/competitors/vllm/csrc/quantization/vectorization_utils.cuh` · `__device__ inline void vectorize_with_alignment`. Census: `lane_compile_lock::every_row_kernel_loads_in_its_widest_aligned_vector`. Done when misaligned rows run at vector speed.
- [ ] **TECH-PACKED-HALF2-MATH**: half2 and bf16x2 arithmetic through float2. Copy from `external/competitors/vllm/csrc/layernorm_kernels.cu` · `Generic fused_add_rms_norm_kernel`. Census: PTX carries `.f16x2` / `.bf16x2` arithmetic where operands are half. Done when half-precision paths run two lanes per instruction.
- [ ] **TECH-PER-ARCH-LAUNCH-BOUNDS**: blocks per SM from the arch's threads per SM. Copy from `external/competitors/vllm/csrc/launch_bounds_utils.h` · `VLLM_BLOCKS_PER_SM`. Census: every kernel's launch bounds name the chip (`CudaChip`). Done when occupancy is chosen per chip.
- [ ] **TECH-CP-ASYNC-MULTISTAGE**: async copies in N stages. Copy from `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/Pipeliner/LowerLoops.cpp` · `void createAsyncCopy`. Census: SASS carries `LDGSTS` in the pipelined kernels. Done when every streamed kernel hides its loads.
- [ ] **TECH-MBARRIER-PIPELINE**: mbarrier-tracked async copies. Copy from `external/competitors/vllm/csrc/dsv3_fused_a_gemm.cu` · `fused_a_gemm_kernel(`. Census: PTX carries `mbarrier` arrive and wait. Done when stage hand-off costs no full barrier.
- [ ] **TECH-LOP3-PRMT-DEQUANT**: magic-number int4 and int8 to half via LOP3 and PRMT. Copy from `external/competitors/vllm/csrc/quantization/marlin/dequant.h` · `__device__ inline int lop3`. Census: SASS carries `LOP3` / `PRMT` in the block-format decoders. Done when dequantization costs one or two instructions per pair.
- [ ] **TECH-STREAM-K**: striped partitioning with a slice-locked global reduce. Copy from `external/competitors/vllm/csrc/quantization/marlin/marlin_template.h` · `if (use_fp32_reduce) {`. Census: the last-wave GEMMs split by stripes; bit-exactness stays declared per OPT-6. Done when no GEMM loses a partial wave.
- [ ] **TECH-BROADCAST-SCALE-EPILOGUE**: scales read from device pointers in the epilogue. Copy from `external/competitors/vllm/csrc/cutlass_extensions/epilogue/scaled_mm_epilogues_c3x.hpp` · `struct ScaledEpilogueBiasAzpToken`. Census: no host sync between a scale's producer and its GEMM. Done when scaled GEMMs never sync.
- [ ] **TECH-UVA-HOST-VIEW**: pinned host memory addressed from the GPU. Copy from `external/competitors/vllm/csrc/cuda_view.cu` · `get_cuda_view_from_cpu_tensor`. Census: the residence planner's host pool is mapped (`lane_compile_lock::a_matrix_takes_the_bytes_its_upload_carves`). Done when already done: record it in the ledger. Coverage: 1 row in section 11.
- [ ] **TECH-RADIX-TOPK-KEYS**: order-preserving float-to-uint keys for radix selection. Copy from `external/competitors/vllm/csrc/persistent_topk.cuh` · `histogram_2048_topk(`. Census: top-k uses radix passes over flipped keys. Done when top-k over a vocabulary costs a few passes.
- [ ] **TECH-COMPENSATED-RESIDUAL**: fast2sum-compensated residual sums. Copy from `external/competitors/vllm/vllm/models/muse_spark/common/ops/fused_rmsnorm.py` · `def _fast2sum`. Census: a numerics contract declares the compensation (OPT-6). Done when long residual chains keep their low bits. Coverage: 1 row in section 11.
- [ ] **TECH-SPLIT-PRECISION-MMA**: exact sums from split-precision MMAs (bf16x3, tf32x3, int8 two-word). Copy from `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/F32DotTC.cpp` · `struct F32DotTC`. Census: the f32 router and GEMMs offer a split-precision tensor-core path. Done when f32-exact products run on tensor cores. Coverage: 4 rows in section 11.
- [ ] **TECH-STOCHASTIC-ROUNDING-STATE**: fp16 state stored with stochastic rounding. Copy from `external/competitors/vllm/vllm/model_executor/layers/mamba/ops/mamba_ssm.py` · `def convert_rs_fp16x2`. Census: PTX carries `cvt.rs` where the shim's numerics allow it. Done when half-precision recurrent state keeps its accuracy. Coverage: 1 row in section 11.
- [ ] **TECH-EXP2-SCALE-FOLD**: exp2 with log2(e) folded into the scale. Copy from `external/competitors/triton/python/tutorials/06-fused-attention.py` · `def _attn_fwd_inner`. Census: attention kernels call `ex2` with the folded scale. Done when softmax costs one multiply fewer. Coverage: 1 row in section 11.
- [ ] **TECH-L2-WEIGHT-PREFETCH**: the next weights pulled into L2 while attention runs. Copy from Kimi-K3's specialized decode in vLLM (`external/competitors/vllm/vllm/models/kimi_k3`). Census: a prefetch launch overlaps attention in the branched streams. Done when weight reads start before they are needed.
- [ ] **TECH-SCHED-FOR-REGISTERS**: register-pressure scheduling and loop-strength-reduction flags. Copy from `external/competitors/triton/third_party/nvidia/backend/compiler.py` · `sched4reg: bool = False`. Census: the lane's nvcc flags are recorded per kernel. Done when flags are chosen by measurement.
- [ ] **TECH-PACKED-FP-CONVERSIONS**: packed fp8 and fp4 conversions, in software on sm_86. Copy from `external/competitors/vllm/csrc/quantization/fp4/nvfp4_quant_kernels.cu` · `cvt_fp16_to_fp4(`. Census: the conversions match a CPU replica bit for bit. Done when every format converts at packed speed.
- [ ] **TECH-INLINE-PTX-PACKING**: inline PTX over packed registers. Copy from `external/competitors/triton/python/triton/language/core.py` · `def inline_asm_elementwise(`. Census: the kernels' inline PTX is listed and tested. Done when escape hatches are tested like everything else. Coverage: 3 rows in section 11.
- [ ] **TECH-OCCUPANCY-FROM-REGS**: the grid sized from register and shared-memory use after compiling. Copy from `external/competitors/triton/python/tutorials/02-fused-softmax.py` · `def softmax(`. Census: grid sizes follow `ptxas -v`. Done when every persistent grid fills the chip.
- [ ] **TECH-256BIT-LOADS**: 256-bit loads and stores with streaming hints (sm_100). Copy from `external/competitors/vllm/csrc/cuda_vec_utils.cuh` · `ld.global.nc.v8.u32`. Census: PTX carries `v8` loads for sm_100 builds. Done when `compile_verified`.
- [ ] **TECH-PDL**: programmatic dependent launch (sm_90+). Copy from `external/competitors/vllm/csrc/fused_qknorm_rope_kernel.cu` · `constexpr int kPDLEnableTokens`. Census: launches carry the PDL attribute; kernels call `griddepcontrol`. Done when `compile_verified`; feeds GRAPH-CROSS-KERNEL-OVERLAP. Coverage: 1 row in section 11.
- [ ] **TECH-WGMMA-RS-PREPACKED**: register-sourced wgmma with weights prepacked to its layout. Copy from `external/competitors/vllm/csrc/quantization/machete/machete_mainloop.cuh` · `MacheteCollectiveMma`. Census: PTX carries `wgmma` with a register A operand. Done when `compile_verified`. Coverage: 4 rows in section 11.
- [ ] **TECH-TMA-WS-PINGPONG**: TMA loads with warp-specialized ping-pong consumers. Copy from `external/competitors/triton/python/tutorials/09-persistent-matmul.py` · `EPILOGUE_SUBTILE = nargs.get(`. Census: PTX carries `cp.async.bulk.tensor` and named barriers. Done when `compile_verified`.
- [ ] **TECH-2SM-UMMA-TMEM**: two-SM MMAs and tensor memory (sm_100). Copy from `external/competitors/vllm/csrc/attention/mla/cutlass_sm100_mla/kernel/sm100_fmha_mla_tma_warpspecialized.hpp` · `Sm100FmhaMlaKernelTmaWarpspecialized`. Census: PTX carries `tcgen05`. Done when `compile_verified`. Coverage: 11 rows in section 11.
- [ ] **TECH-CLUSTER-DSMEM-REDUCE**: reductions across a cluster's distributed shared memory. Copy from `external/competitors/vllm/vllm/models/flint/ops/decode_attention.py` · `Split-KV paged decode attention for Flint`. Census: PTX carries `mapa` / cluster barriers. Done when `compile_verified`. Coverage: 2 rows in section 11.
- [ ] **TECH-CLUSTER-LAUNCH-CONTROL**: persistent work stealing with cluster launch control (sm_100). Copy from `external/competitors/triton/lib/Dialect/TritonNvidiaGPU/Transforms/ToCLC.cpp`. Census: PTX carries `clusterlaunchcontrol`. Done when `compile_verified`. Coverage: 3 rows in section 11.
- [ ] **TECH-BATCH-INVARIANCE**: results independent of batch shape. Copy from `external/competitors/vllm/vllm/model_executor/determinism/batch_invariant.py` · `def matmul_kernel_persistent(`. Census: `cuda_lane_lock::a_prompt_run_in_chunks_continues_as_one_fed_a_token_at_a_time`. Done when already stronger: record it in the ledger. Coverage: 1 row in section 11.

Each TECH ticket has `Depends on` as listed in the board index. Its RED test is the census named in the table, written as
a `lane_compile_lock` test, plus the A/B row. Its mutant is the technique removed, which the census must catch.

### 12.15 Triton: compiler parity and past it

#### TRITON-TILE-TIER · A generated-kernel tier under the CUDA link

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `infrastructure` `cuda` |
| Depends on | MIGRATE-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/lower/cuda.rs` (persistent CUDA from the spatial IR), `crates/logicaffeine_compile/src/codegen_cuda/gemm_tile.rs` (the measured GEMM lattice), the CUDA link |
| Coverage | 69 rows in section 11 |
| Status | open |

**Goal.**
- The CUDA link today is hand-written `lk_*` templates. Below it goes a tile-level kernel IR, built from the persistent
  CUDA lowering and the GEMM lattice we already have.
- A fused symbol with no template links a generated kernel. Templates become oracles held equal to the generated kernel,
  and retire when it matches them.
- The layout algebra, proven axis facts, global layout assignment, the modulo scheduler, warp specialization and MMA
  selection all live in this tier.

**Copy.** Triton's whole TTGIR pipeline, which section 10.3 catalogs.

**Beyond.** Triton generates one kernel per hand-written function. This tier generates a kernel per fused symbol the
shared optimizer chose, so every fusion gets a fast kernel without anyone writing one.

**RED tests.**
- [ ] `cuda_tile_tier_lock::every_fused_symbol_links_a_template_or_a_generated_kernel`
- [ ] `cuda_tile_tier_lock::a_generated_kernel_computes_what_its_template_computes`
- [ ] `cuda_tile_tier_lock::a_generated_kernel_compiles_with_nvcc_and_spills_nothing`

**Edge cases.**
- [ ] A symbol with neither a template nor a generatable form: refused by name.
- [ ] Non-power-of-two tile shapes. Triton cannot do these; this tier must.

**Success.**
- [ ] RED green.
- [ ] One template retired because its generated kernel matched it (receipt: A/B row).

**Done when.** Every fused symbol has a kernel, written or generated.

#### TRITON-LINEAR-LAYOUT · Layouts as linear maps over F2

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `infrastructure` |
| Depends on | none |
| Touches | a new chip-agnostic layout module (spatial fabrics place data with it too) |
| Coverage | 19 rows in section 11 |
| Status | open |

**Goal.**
- Every register, shared and tensor-memory layout is a linear map over F2, defined by its bases at powers of two.
- The operations are compose, invertAndCompose, pseudoinvert, direct sum, divide left and right, reshape, transpose,
  sublayout and free-variable masks.

**Copy.** `external/competitors/triton/include/triton/Tools/LinearLayout.h` · `Adam P. Goucher`.

**Beyond.** Property-tested against brute-force index maps, and every operation's algebraic laws proven at bounded
sizes (OPT-5's workflow).

**RED tests.**
- [ ] `layout_algebra_lock::every_operation_matches_its_brute_force_index_map`
- [ ] `layout_algebra_lock::invert_and_compose_inverts_a_bijection`
- [ ] `layout_algebra_lock::composition_is_associative_on_every_small_layout`

**Edge cases.**
- [ ] Broadcast layouts (not injective).
- [ ] Zero-dimensional inputs.
- [ ] Layouts larger than 2^20 elements.

**Mutants to kill.**
- [ ] A basis vector dropped.
- [ ] XOR replaced by OR.

**Success.**
- [ ] RED green.

**Done when.** Layout decisions are linear algebra.

#### TRITON-GENERIC-SWIZZLE · Optimal shared-memory swizzles

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `technique` `cuda` |
| Depends on | TRITON-LINEAR-LAYOUT |
| Status | open |

**Goal.** Given the source and destination register layouts, compute the shared-memory layout with the widest
vectorization and the fewest bank conflicts, in closed form: nullspace bases and a bank-segment complement.

**Copy.** `external/competitors/triton/lib/Tools/GenericSwizzling.cpp` · `static std::optional<SmallVector<int32_t>> optimalSwizzling`.

**RED tests.**
- [ ] `layout_algebra_lock::the_swizzle_matches_brute_force_search_on_small_layouts`
- [ ] `layout_algebra_lock::the_swizzle_counts_its_bank_conflicts_exactly`

**Success.**
- [ ] RED green.

**Done when.** Every staged tile is conflict-free by construction.

#### TRITON-SHUFFLE-CONVERT · Layout conversion through warp shuffles

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `technique` `cuda` |
| Depends on | TRITON-LINEAR-LAYOUT |
| Status | open |

**Goal.** A register-to-lane layout conversion factored into selp / shfl / selp row operations over F2, with no shared
memory.

**Copy.** `external/competitors/triton/lib/Conversion/TritonGPUToLLVM/ConvertLayoutOpToLLVM.cpp` · `LogicalResult transferWithinWarp`.

**RED tests.**
- [ ] `layout_algebra_lock::every_within_warp_conversion_lowers_to_shuffles_and_permutes_correctly`

**Success.**
- [ ] RED green.

**Done when.** Within-warp conversions never touch shared memory.

#### TRITON-AXIS-FACTS · Proven contiguity, divisibility and constancy

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `infrastructure` |
| Depends on | OPT-1 |
| Touches | `tensor_graph::facts` |
| Coverage | 4 rows in section 11 |
| Status | open |

**Goal.**
- Per dimension of every index expression: contiguity, divisibility and constancy, proven from the program.
- Triton trusts `multiple_of` and `assume`. These facts are derived, and every vector width a link picks cites one.

**Copy.** `external/competitors/triton/lib/Analysis/AxisInfo.cpp` · `class MakeRangeOpAxisInfoVisitor` (the transfer
functions).

**RED tests.**
- [ ] `fusion_class_lock::every_vector_width_a_link_picks_is_justified_by_a_proven_fact`
- [ ] `fusion_class_lock::an_unproven_alignment_is_never_assumed`

**Mutants to kill.**
- [ ] A trusted hint with no proof.

**Success.**
- [ ] RED green.

**Done when.** No alignment is assumed.

#### TRITON-COMBINE-REWRITES · Contraction recognition and broadcast sinking

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `infrastructure` |
| Depends on | OPT-4 |
| Coverage | 27 rows in section 11 |
| Status | open |

**Goal.** Each of these is an e-graph rewrite with a certificate:
- dot plus add becomes a dot with an accumulator;
- broadcast-multiply-sum is recognized as a matmul;
- elementwise of a broadcast becomes a broadcast of the elementwise;
- loop-carried values are equated by an optimistic fixpoint.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- `external/competitors/triton/lib/Dialect/Triton/Transforms/Combine.cpp` · `bool isZero`
- `external/competitors/triton/lib/Dialect/Triton/Transforms/ReorderBroadcast.cpp` · `struct MoveSplatAfterElementwisePattern`
- `external/competitors/triton/lib/Dialect/Triton/Transforms/LoopAwareCSE.cpp` · `struct LoopAwareCSE`

**RED tests.**
- [ ] `tensor_egraph_lock::a_broadcast_multiply_sum_is_recognized_as_a_matmul`
- [ ] `tensor_egraph_lock::broadcasts_sink_below_elementwise_ops`

**Success.**
- [ ] RED green.

**Done when.** A program written as loops reaches the tensor cores.

#### TRITON-COALESCE · Coalesced accesses from proven facts

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `technique` `cuda` |
| Depends on | TRITON-AXIS-FACTS, TRITON-TILE-TIER |
| Coverage | 6 rows in section 11 |
| Status | open |

**Goal.**
- Each generated access takes vector width = min(divisibility / element bytes, contiguity, 128 bits), shared across
  dependent accesses.
- Async copies clip their per-thread width to the shared vector.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/Coalesce.cpp` · `struct CoalescePass`
- `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/CoalesceAsyncCopy.cpp` · `struct CoalesceAsyncCopy`

**RED tests.**
- [ ] `cuda_tile_tier_lock::every_generated_access_is_as_wide_as_its_facts_allow`

**Success.**
- [ ] RED green.

**Done when.** Generated kernels coalesce by proof.

#### TRITON-GLOBAL-LAYOUT-ASSIGNMENT · Minimum-cost layout assignment

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `infrastructure` `cuda` |
| Depends on | TRITON-LINEAR-LAYOUT, TRITON-TILE-TIER |
| Coverage | 10 rows in section 11 |
| Status | open |

**Goal.** Layouts across a generated kernel's dataflow are chosen by a global minimum-cost assignment (exact on small
graphs, ILP or DP beyond), with Triton's cost primitives. A conversion costs a shared round trip; rematerialization costs
the producer's arithmetic.

**Copy.** `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/RemoveLayoutConversions.cpp` · `Hacky resolve, prefer block encoding`. It is greedy, with fixed costs and a cap of 20 rounds.

**RED tests.**
- [ ] `cuda_tile_tier_lock::the_assignment_never_converts_more_than_the_greedy_baseline`
- [ ] `cuda_tile_tier_lock::the_assignment_is_optimal_on_every_small_graph`

**Success.**
- [ ] RED green.

**Done when.** No generated kernel pays a conversion a better assignment avoids.

#### TRITON-THREAD-LOCALITY · Thread-local partials across loops

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `technique` `cuda` |
| Depends on | TRITON-TILE-TIER |
| Coverage | 2 rows in section 11 |
| Status | open |

**Goal.** Loop-carried reductions keep thread-local partials and merge after the loop. Gathers choose the layout that
minimizes shuffles.

**Copy.** `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/OptimizeThreadLocality.cpp` · `class TritonGPUOptimizeThreadLocality`.

**RED tests.**
- [ ] `cuda_tile_tier_lock::a_loop_carried_reduction_merges_once_after_its_loop`

**Success.**
- [ ] RED green.

**Done when.** Reductions inside loops cost one merge.

#### TRITON-TRANSPOSE-AS-ADDRESSING · Transposes folded into addressing

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `technique` `cuda` |
| Depends on | TRITON-TILE-TIER |
| Coverage | 4 rows in section 11 |
| Status | open |

**Goal.** A transposed or reshaped operand is folded into shared-memory addressing (`ldmatrix.trans`) instead of being
moved.

**Copy.** `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/OptimizeDotOperands.cpp` · `class TritonGPUOptimizeDotOperands`.

**RED tests.**
- [ ] `cuda_tile_tier_lock::a_transposed_operand_is_read_through_ldmatrix_trans`

**Success.**
- [ ] RED green.

**Done when.** Transposes are free.

#### TRITON-SMEM-STAGING-NODES · Shared-memory staging as graph nodes

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `technique` `cuda` |
| Depends on | TRITON-TILE-TIER |
| Coverage | 2 rows in section 11 |
| Status | open |

**Goal.** A conversion to a dot operand is staged through shared memory as an explicit node, so two consumers share
one staged copy.

**Copy.** `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/ReduceDataDuplication.cpp` · `class TritonGPUReduceDataDuplication`.

**RED tests.**
- [ ] `cuda_tile_tier_lock::two_consumers_of_one_operand_share_one_staged_copy`

**Success.**
- [ ] RED green.

**Done when.** Staged operands are shared.

#### TRITON-REGISTER-PRESSURE-ORDER · Instruction order for register pressure

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `technique` `cuda` |
| Depends on | TRITON-TILE-TIER |
| Coverage | 1 row in section 11 |
| Status | open |

**Goal.** Generated kernels order their operations to lower peak register pressure, measured by `ptxas -v`.

**Copy.** `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/ReorderInstructions.cpp` · `class TritonGPUReorderInstructions`.

**RED tests.**
- [ ] `cuda_tile_tier_lock::the_ordering_never_raises_peak_registers`

**Success.**
- [ ] RED green.

**Done when.** No generated kernel spills because of its order.

#### TRITON-ACCUMULATOR-INIT · The use-D flag instead of a zero fill

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `technique` `cuda` |
| Depends on | TRITON-TILE-TIER |
| Coverage | 1 row in section 11 |
| Status | open |

**Goal.** An accumulator's zero initialization is replaced by the MMA's use-D flag on the first iteration.

**Copy.** `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/OptimizeAccumulatorInit.cpp` · `class OptimizeAccumulatorInit`.

**RED tests.**
- [ ] `cuda_tile_tier_lock::no_generated_accumulator_is_zero_filled_before_its_first_mma`

**Success.**
- [ ] RED green.

**Done when.** Accumulators start free.

#### TRITON-FUSE-NESTED-LOOPS · Persistent loops from nests

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `technique` `cuda` |
| Depends on | TRITON-TILE-TIER |
| Coverage | 3 rows in section 11 |
| Status | open |

**Goal.**
- A loop nest is flattened into one persistent loop, so the outer loop's work can be pipelined.
- Child bounds that depend on the parent are handled too. Triton cannot do this, and needs a user's `tl.assume` even for
  bounds that do not.

**Copy.** `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/FuseNestedLoops.cpp` · `struct FuseNestedLoops`.

**RED tests.**
- [ ] `cuda_tile_tier_lock::a_nest_with_parent_dependent_bounds_flattens`

**Success.**
- [ ] RED green.

**Done when.** Persistent kernels come from any nest.

#### TRITON-OPERAND-PREFETCH · Operand register double-buffering

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `technique` `cuda` |
| Depends on | TRITON-TILE-TIER |
| Coverage | 1 row in section 11 |
| Status | open |

**Goal.** The next iteration's operand slice is loaded from shared memory into registers at the end of the current
iteration.

**Copy.** `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/Prefetch.cpp` · `class Prefetch`.

**RED tests.**
- [ ] `cuda_tile_tier_lock::the_next_operand_slice_loads_during_the_current_mma`

**Success.**
- [ ] RED green.

**Done when.** MMAs never wait on shared-memory loads.

#### TRITON-MODULO-SCHEDULER · A real modulo scheduler

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `infrastructure` `cuda` |
| Depends on | TRITON-TILE-TIER |
| Coverage | 6 rows in section 11 |
| Status | open |

**Goal.**
- Loops are software-pipelined by an iterative modulo schedule: the initiation interval comes from the resource and
  recurrence bounds.
- The schedule uses async-copy and TMA ring buffers with mbarrier phases, and multibuffered accumulators.
- It covers outer and nested loops too.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- Triton's latencies: `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/Pipeliner/AssignLatencies.cpp` · `struct AssignLatencies`.
- Triton's coarse schedule: `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/Pipeliner/ScheduleLoops.cpp` · `struct ScheduleLoops`.
- Triton's lowering: `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/Pipeliner/LowerLoops.cpp` · `void createAsyncCopy`.
- Triton pipelines innermost loops only, at distance ≤ 1, with latency from a formula.

**RED tests.**
- [ ] `cuda_tile_tier_lock::the_schedule_reaches_the_lower_bound_on_its_initiation_interval`
- [ ] `cuda_tile_tier_lock::a_nested_loop_is_pipelined`
- [ ] `cuda_tile_tier_lock::a_pipelined_loop_computes_what_the_plain_loop_computes`

**Success.**
- [ ] RED green.

**Done when.** Every loop runs at its initiation-interval bound.

#### TRITON-ASYNC-MMA-WAITS · Minimal waits for async MMAs

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `technique` `datacenter` |
| Depends on | TRITON-MODULO-SCHEDULER |
| Coverage | 1 row in section 11 |
| Status | open |

**Goal.** wgmma runs asynchronously, with waits placed only where an operand or result requires them.

**Copy.** `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/Pipeliner/WGMMAPipeline.cpp` · `static std::optional<int> dotCanBeProperlyAsync`.

**RED tests.**
- [ ] `cuda_tile_tier_lock::every_mma_wait_is_required_by_a_dependency`

**Success.**
- [ ] `compile_verified`.

**Done when.** No MMA waits without a reason.

#### TRITON-TMA · TMA descriptors, ragged, gather and im2col

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `technique` `datacenter` |
| Depends on | TRITON-TILE-TIER |
| Coverage | 15 rows in section 11 |
| Status | open |

**Goal.** Tiles move by TMA through host- or device-built descriptors, including:
- ragged descriptors (MoE);
- row gather and scatter;
- im2col (convolutions).

**Copy.** `external/competitors/triton/lib/Dialect/TritonNvidiaGPU/Transforms/TMALowering.cpp` · `class TritonNvidiaGPUTMALowering`.

**RED tests.**
- [ ] `cuda_tile_tier_lock::every_tma_tile_matches_its_pointer_loaded_replica`

**Success.**
- [ ] `compile_verified`.

**Done when.** Datacenter tiles move by TMA.

#### TRITON-WARP-SPECIALIZATION · Warp specialization as a graph cut

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `infrastructure` `cuda` |
| Depends on | TRITON-MODULO-SCHEDULER |
| Coverage | 9 rows in section 11 |
| Status | open |

**Goal.**
- Producer and consumer partitions are a graph cut with communication cost. Channels are lowered to shared-memory rings
  with mbarriers.
- Register budgets per partition are decided by the compiler and redistributed with `setmaxnreg`.
- It works on every arch with async copies, not only sm_100.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/WarpSpecialization/PartitionScheduling.cpp` · `void duplicateCheapOps`
- `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/WarpSpecialization/OptimizePartitionWarps.cpp` · `struct OptimizePartitionWarps`

Triton's warp specialization is automatic only on sm_100, with rough register estimates. Its Hopper form needs four
warps and no else-blocks.

**RED tests.**
- [ ] `cuda_tile_tier_lock::the_partition_minimizes_communication_on_every_small_graph`
- [ ] `cuda_tile_tier_lock::every_partition_gets_the_registers_it_needs_and_no_more`
- [ ] `cuda_tile_tier_lock::a_specialized_kernel_computes_what_the_plain_kernel_computes`

**Success.**
- [ ] RED green.

**Done when.** Warp specialization is chosen, sized and proven by the compiler.

#### TRITON-MMA-SELECTION · Tensor-core instruction selection

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `infrastructure` `cuda` |
| Depends on | TRITON-TILE-TIER |
| Coverage | 6 rows in section 11 |
| Status | open |

**Goal.**
- The generated tier picks the MMA instruction and shape by chip: m16n8k16 and m16n8k32 on sm_80/86, wgmma on sm_90,
  tcgen05 on sm_100, and scaled MMAs.
- Chained dots (attention) are tiled so row reductions stay inside a warp.
- Mixed-mode operands are promoted where the chip lacks a native form.

**Copy.** `external/competitors/triton/lib/Dialect/TritonGPU/Transforms/AccelerateMatmul.cpp` · `static int getMMAVersionSafe`.

**RED tests.**
- [ ] `cuda_tile_tier_lock::every_chip_gets_its_native_mma`
- [ ] `cuda_tile_tier_lock::a_chained_dot_keeps_its_row_reduction_in_a_warp`

**Success.**
- [ ] RED green.

**Done when.** Every chip's tensor cores are used natively.

#### TRITON-SMEM-COLORING-ALLOCATOR · Shared memory by interference colouring

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `technique` `cuda` |
| Depends on | TRITON-TILE-TIER |
| Coverage | 4 rows in section 11 |
| Status | open |

**Goal.** Shared-memory buffers inside a generated kernel are placed by liveness and interference-graph colouring.
The lane's arena already places step-local arrays by lifetime.

**Copy.** `external/competitors/triton/lib/Analysis/Allocation.cpp` · `class Allocation`.

**RED tests.**
- [ ] `cuda_tile_tier_lock::no_two_live_buffers_share_shared_memory`

**Success.**
- [ ] RED green.

**Done when.** Generated kernels use the least shared memory their liveness allows.

#### TRITON-REGION-BARRIERS · Barriers and fences from region analysis

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `technique` `cuda` |
| Depends on | TRITON-SMEM-COLORING-ALLOCATOR |
| Coverage | 8 rows in section 11 |
| Status | open |

**Goal.** Barriers and async-proxy fences are inserted exactly where shared-memory regions conflict.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- `external/competitors/triton/lib/Analysis/Membar.cpp` · `Operation *MembarAnalysis::insertBarrier`
- `external/competitors/triton/lib/Dialect/TritonNvidiaGPU/Transforms/FenceInsertion.cpp` · `struct FenceInsertion`

**RED tests.**
- [ ] `cuda_tile_tier_lock::every_shared_memory_hazard_has_a_barrier_and_every_barrier_a_hazard`

**Success.**
- [ ] RED green.
- [ ] compute-sanitizer racecheck clean on the generated kernels (receipt).

**Done when.** No race, and no barrier without a reason.

#### TRITON-REDUCE-SCAN-CODEGEN · Reductions, scans, sorts and top-k

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `technique` `cuda` |
| Depends on | TRITON-TILE-TIER |
| Coverage | 20 rows in section 11 |
| Status | open |

**Goal.**
- Hierarchical reductions: in-thread, then warp shuffles, then shared memory, in arity-3 trees.
- Generic associative scans.
- Histograms.
- Gathers that choose between shuffles and shared memory.
- Bitonic sorts.
- Top-k by packed (value, index) keys.

**Copy.** Bar: the row maximum in SETUP-4 (section 2.5).
- `external/competitors/triton/lib/Conversion/TritonGPUToLLVM/ReduceOpToLLVM.cpp` · `struct ReduceOpConversion`
- `external/competitors/triton/lib/Conversion/TritonGPUToLLVM/ScanOpToLLVM.cpp` · `struct ScanOpConversion`

**RED tests.**
- [ ] `cuda_tile_tier_lock::every_reduction_computes_what_its_sequential_order_declares`
- [ ] `cuda_tile_tier_lock::every_scan_computes_the_prefix`

**Success.**
- [ ] RED green.

**Done when.** Reductions and scans are generated for any combine function.

#### TRITON-AOT-DISPATCH · Dispatch on run-time lengths

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `technique` `cuda` |
| Depends on | MIGRATE-1 |
| Status | open |

**Goal.**
- Our forms are compiled ahead of time for known shapes. A run-time length (a prompt, a batch) picks its form through a
  dispatcher on proven properties: divisibility, a length of 1.
- Triton auto-detects only `% 16` and `== 1`.

**Copy.** `external/competitors/triton/python/src/specialize.cc` · `std::pair<py::object, py::object> handle_long_type(`.

**RED tests.**
- [ ] `lane_compile_lock::every_run_time_length_dispatches_to_its_form`

**Success.**
- [ ] RED green.

**Done when.** Run-time lengths never fall to a slow generic path.

#### TRITON-CACHE-POLICY-EDGES · Cache policies as edge annotations

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `technique` `cuda` |
| Depends on | OPT-2 |
| Status | open |

**Goal.** Load and store cache policies (`.ca`, `.cg`, `.cs`, `.wt`, evict first and last) are edge annotations the shim
allows and the cost model picks. Weights read once take a streaming policy; reused activations stay cached.

**RED tests.**
- [ ] `lane_compile_lock::every_read_once_weight_loads_with_a_streaming_policy`

**Success.**
- [ ] RED green.

**Done when.** Cache policy is the compiler's decision.

#### TRITON-AMD-SHIM · An AMD shim

| Field | Value |
|---|---|
| Phase | triton |
| Labels | `infrastructure` |
| Depends on | OPT-2 |
| Coverage | 134 rows in section 11 |
| Status | open |

**Goal.** When a HIP backend exists, its shim declares what it allows:
- buffer loads and stores with hardware bounds checks;
- scalar-base fat pointers;
- block ping-pong;
- in-thread transposes;
- XCD-aware program ordering;
- MFMA and WMMA selection.

**Copy.** `external/competitors/triton/third_party/amd/lib/TritonAMDGPUTransforms/BlockPingpong.cpp`, and the AMD
pipeline in section 10.3.

**RED tests.**
- [ ] `shim_lock::the_amd_shim_answers_every_class`

**Success.**
- [ ] RED green.

**Done when.** AMD is a chip like any other.

### 12.16 Matching Triton's fused kernels, then beating them

Each ticket below has the same shape:
- **Copy:** time Triton's own kernel at our shapes on the 3060 (SETUP-6, SETUP-4).
- **Produce ours from the compiler:** the shared optimizer's fused symbol, linked by a template or by the generated
  tier, never hand-written for the comparison.
- **Beyond:** beat Triton's time and say why.

- [ ] **PARITY-TRITON-SOFTMAX**: Triton's kernel `external/competitors/triton/python/tutorials/02-fused-softmax.py` · `def softmax(`. Our symbol: `FusedRow` softmax, linked. Beyond: the grid sized from registers and shared memory (TECH-OCCUPANCY-FROM-REGS); softmax fused with its producer and consumer, which a standalone Triton kernel cannot do. RED tests: `cuda_lane_lock::a_fused_softmax_computes_what_the_reference_computes`; A/B row ≥ 1.0× then ≥ 1.1×. Coverage: 2 rows in section 11.
- [ ] **PARITY-TRITON-LAYERNORM**: Triton's kernel `external/competitors/triton/python/tutorials/05-layer-norm.py` · `def _layer_norm_bwd_dx_fused`. Our symbol: LayerNorm forward, linked (the lane refuses LayerNorm today); the backward through `vjp`. Beyond: the backward's partial weight gradients reduced deterministically, without spin locks. RED tests: `cuda_lane_lock::a_layer_norm_computes_what_the_reference_computes`; `tensor_graph_eval_lock` for the backward. Coverage: 1 row in section 11.
- [ ] **PARITY-TRITON-ATTENTION**: Triton's kernel `external/competitors/triton/python/tutorials/06-fused-attention.py` · `def _attn_fwd_inner`; `external/competitors/triton/python/examples/gluon/01-attention-forward.py` · `class AttentionConfig:`. Our symbol: `flash_prefill` / `flash_decode`. Beyond: static masks skip tiles at compile time; warp specialization chosen by the compiler (TRITON-WARP-SPECIALIZATION) where Gluon's is written by hand. RED tests: A/B rows at every scoreboard model's shapes ≥ 1.0× then ≥ 1.1×. Coverage: 2 rows in section 11.
- [ ] **PARITY-TRITON-GROUPED-GEMM**: Triton's kernel `external/competitors/triton/python/tutorials/08-grouped-gemm.py` · `def grouped_matmul_kernel`. Our symbol: the grouped `lk_gemm_blk`, or generated. Beyond: tile schedules from the cost model per problem set. RED tests: A/B row. Coverage: 1 row in section 11.
- [ ] **PARITY-TRITON-PERSISTENT-MATMUL**: Triton's kernel `external/competitors/triton/python/tutorials/09-persistent-matmul.py` · `EPILOGUE_SUBTILE = nargs.get(`. Our symbol: dense tensor-core GEMM. Beyond: epilogue subtiling chosen by shared-memory budget; nested loops flattened without `tl.assume`. RED tests: A/B row. Coverage: 3 rows in section 11.
- [ ] **PARITY-TRITON-DROPOUT**: Triton's kernel `external/competitors/triton/python/tutorials/04-low-memory-dropout.py` · `tl.rand`. Our symbol: a seeded mask from a counter RNG, fused into its consumer. Beyond: the mask never exists in memory, and the same seed reproduces across forms (the PARITY-SAMPLING RNG). RED tests: `device_sampling_lock::a_seeded_dropout_is_reproducible_across_forms`. Coverage: 11 rows in section 11.
- [ ] **PARITY-TRITON-MOE-MATMUL**: Triton's kernel `external/competitors/triton/python/triton_kernels/triton_kernels/matmul.py` · `def matmul(a, b, bias,`. Our symbol: the grouped expert GEMMs with gather in operand (PARITY-MOE-GATHER-IN-OPERAND). Beyond: fusion by the optimizer, not by source-string templating; the whole expert FFN in two launches. RED tests: A/B rows at DeepSeek-V2-Lite and gpt-oss prefill shapes. Coverage: 15 rows in section 11.
- [ ] **PARITY-TRITON-SWIGLU**: Triton's kernel `external/competitors/triton/python/triton_kernels/triton_kernels/swiglu.py` · `def swiglu(`. Our symbol: `glu_act_mul` with clamps, folded into the GEMM epilogue. Beyond: never a standalone launch. RED tests: A/B row. Coverage: 2 rows in section 11.
- [ ] **PARITY-TRITON-TOPK**: Triton's kernel `external/competitors/triton/python/triton_kernels/triton_kernels/topk.py` · `def topk(`. Our symbol: the router's top-k and the sampler's top-k. Beyond: radix passes over order-preserving keys (TECH-RADIX-TOPK-KEYS). RED tests: A/B row. Coverage: 3 rows in section 11.
- [ ] **PARITY-TRITON-MX-CONVERT**: Triton's kernel `external/competitors/triton/python/triton_kernels/triton_kernels/numerics_details/mxfp_details/_downcast_to_mxfp.py` · `_downcast_to_mxfp`. Our symbol: the MX schemes of PARITY-QUANT-SCHEMES. Beyond: bit-exact against a CPU replica, then at packed speed. RED tests: `numerics_contract_lock::every_scheme_quantizes_as_its_replica_does`. Coverage: 5 rows in section 11.

Each of these tickets depends on SETUP-6 and on the symbol's own ticket (board index). A ticket is done when its A/B row
shows ≥ 1.0× the row maximum (copied: Triton's kernel and every other rival section 2.5 names) and then the Beyond bar,
past 2 standard errors, on one boot.

### 12.17 Beyond SOTA: what only our mathematics makes possible

Every ticket above copies the best stack, then passes it by a route the ticket names. The tickets below are different
in kind. Each turns an engine this repository already has into a compiler capability no other ML compiler ships.
- The certified SAT, pseudo-Boolean and optimization core;
- exact min-cost flow and cycle ratios;
- SAT-swept equivalence;
- the spatial choreography certificate;
- Presburger and affine reasoning;
- interval and sum-of-squares proofs;
- the partial evaluator and supercompiler;
- our own SASS tooling;
- the copy-and-patch JIT.

Each is held to the same template and success rules as the rest of the board. Section 3.5 says which chip each serves.

#### BEYOND-CERTIFIED-OPTIMAL-FUSION · Fusion plans that are provably optimal under the shim's cost

| Field | Value |
|---|---|
| Phase | beyond |
| Labels | `beyond` `infrastructure` |
| Depends on | OPT-2, OPT-4, OPT-7 |
| Touches | `tensor_graph::fuse` (a new planner beside the rule engine); `crates/logicaffeine_proof/src/optimize.rs`, `crates/logicaffeine_proof/src/pseudo_boolean.rs` (callers only) |
| Status | open |

**Goal.** Which fused symbols to form, where several overlap, is a choice. The rule engine makes it greedily, in rule
order.
- This ticket states the choice as a 0/1 problem over the IR: one variable per candidate fusion (the e-graph's
  extraction choices of OPT-4), linear constraints from each shim's limits (registers, shared memory, launch shape),
  and an objective of bytes moved plus launches priced by the shim's cost (OPT-7).
- `minimize_certified` solves it to a **certified optimum**: a witness plan at cost C, and a refutation that no plan
  costs C − 1.
- The compile carries the certificate in its fusion report (OPT-3). No other compiler can say its fusion plan is
  optimal, only that it is good.

**Copy.** None exists: Triton fuses within one kernel by hand, vLLM by fixed patterns, and research compilers that
pose fusion as an integer program trust their solver. Bar: the plan's measured step time at or below the greedy
plan's on every scoreboard model.

**Beyond.**
- The optimality is proven, and the proof is re-checked by the small RUP checker the SAT core already trusts.
- When the problem exceeds a solve budget, the planner returns the best witness with its proven lower bound, so the
  report states the optimality gap rather than nothing.

**Read first.**
- `crates/logicaffeine_proof/src/optimize.rs` (`minimize_certified`);
- `crates/logicaffeine_proof/src/pseudo_boolean.rs`;
- `crates/logicaffeine_compile/src/optimize/egraph/extract.rs`.

**RED tests.**
- [ ] `fusion_plan_lock::the_plan_is_optimal_and_carries_its_refutation`: fails today because no planner exists.
- [ ] `fusion_plan_lock::the_optimal_plan_is_never_worse_than_the_greedy_plan_under_the_cost`
- [ ] `fusion_plan_lock::every_plan_respects_every_shim_limit`
- [ ] `fusion_plan_lock::a_budget_hit_reports_the_witness_and_its_lower_bound`

**Edge cases.**
- [ ] Two candidates sharing a producer (the fan-out PARITY-RECOMPUTE-FANOUT decides by cost).
- [ ] A candidate legal on one chip and refused by another in a multi-chip deployment.
- [ ] A 27B graph (about 1,600 ops) within the compile-time budget, or the reported gap.

**Success.**
- [ ] Every RED test green (receipt: log paths).
- [ ] On every scoreboard model, the plan's step time ≤ the greedy plan's, on one boot (receipt: SETUP-4 rows).

**Mutants to kill.**
- [ ] A refutation that does not refute C − 1.
- [ ] A limit dropped from the constraints.

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test fusion_plan_lock
```

**Done when.** Every compile's fusion plan is optimal under its shim's cost, with a proof, or states its gap.

#### BEYOND-EXACT-MODULO-SCHEDULER · One exact scheduler for every chip's pipelines

| Field | Value |
|---|---|
| Phase | beyond |
| Labels | `beyond` `infrastructure` `cuda` `spatial` `d5005` `fabric` |
| Depends on | OPT-2, TRITON-TILE-TIER |
| Touches | a shared scheduling module beside `tensor_graph` (operational side); `crates/logicaffeine_proof/src/cycle_ratio.rs`, `crates/logicaffeine_proof/src/min_cost_flow.rs` (callers only) |
| Status | open |

**Goal.** Software pipelining on every chip is one problem:
- GPU `cp.async` and TMA stages;
- spatial FIFO depths and handshakes;
- D5005 command overlap;
- fabric retiming.

This ticket solves it with exact tools:
- the recurrence bound on the initiation interval is the maximum cycle ratio of the loop's dependence graph
  (`max_cycle_ratio`, certified);
- the resource bound comes from the shim's units;
- the start offsets are the dual of a min-cost flow over difference constraints (`min_cost_flow`).

TRITON-MODULO-SCHEDULER is this scheduler's GPU application.

**Copy.** Triton's pipeliner (`external/competitors/triton/lib/Dialect/TritonGPU/Transforms/Pipeliner/ScheduleLoops.cpp` · `struct ScheduleLoops`):
innermost loops, distance ≤ 1, latency from a formula. Bar: no slower than Triton's pipelined kernels at our shapes.

**Beyond.**
- The initiation interval reached equals the proven lower bound max(resource, recurrence), or the schedule reports
  the gap.
- Outer and nested loops, any distance.
- The same scheduler sizes spatial FIFOs and retimes fabric pipelines, so one proof covers four chips.

**RED tests.**
- [ ] `modulo_schedule_lock::the_interval_meets_the_proven_bound_on_the_corpus`: fails today because no shared
  scheduler exists.
- [ ] `modulo_schedule_lock::every_schedule_respects_every_dependence_and_resource`
- [ ] `modulo_schedule_lock::the_recurrence_bound_is_the_certified_cycle_ratio`
- [ ] `modulo_schedule_lock::one_scheduler_serves_the_gpu_the_fabric_and_the_engine`

**Edge cases.**
- [ ] A loop with no recurrence (resource-bound).
- [ ] Two recurrences of equal ratio.
- [ ] A dependence of distance 3 (a ring of three buffers).
- [ ] A resource the shim declares shared between stages (one TMA unit, one DSP column).

**Success.**
- [ ] Every RED test green (receipt: log paths).
- [ ] The prefill GEMM and attention kernels at their proven interval (receipt: SASS census and SETUP-4 rows).

**Mutants to kill.**
- [ ] A dependence dropped from the graph.
- [ ] A resource count off by one.

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test modulo_schedule_lock
```

**Done when.** Every pipelined loop on every chip runs at its proven interval or reports by how much it misses.

#### BEYOND-CERTIFIED-ALLOCATION · Every on-chip memory allocated by proof

| Field | Value |
|---|---|
| Phase | beyond |
| Labels | `beyond` `infrastructure` `cuda` `spatial` `d5005` |
| Depends on | OPT-2 |
| Touches | the CUDA arena and shared-memory layout; `crates/logicaffeine_compile/src/spatial/residency.rs`; `crates/logicaffeine_proof/src/interval_sched.rs`, `crates/logicaffeine_proof/src/register_alloc.rs` (callers only) |
| Status | open |

**Goal.** Buffer lifetimes over a schedule are intervals, and interval graphs are perfect. So the minimum memory equals
the peak of simultaneously live bytes, and a colouring that reaches it is optimal by proof. One allocator serves:
- GPU shared memory and tensor memory;
- the global arena;
- spatial SRAM residency;
- the D5005 engine's on-chip buffers.

It uses `schedule_or_overflow` and `peak_concurrency`. When a schedule does not fit, it returns an overflow witness:
the instant where too much is live.

**Copy.** Triton's shared-memory allocator: liveness plus interference colouring
(`external/competitors/triton/lib/Analysis/Allocation.cpp` · `class Allocation`). Bar: never more bytes than Triton's
allocation of the same kernel.

**Beyond.**
- Optimal where the interference graph is an interval graph, with the certificate.
- A refusal comes with its witness: "these k buffers are live together at step t", which the fusion planner reads.

**RED tests.**
- [ ] `allocation_lock::every_allocation_uses_exactly_the_peak_live_bytes`: fails today because the arena is planned
  by a heuristic.
- [ ] `allocation_lock::an_overflow_names_the_instant_and_the_buffers`
- [ ] `allocation_lock::the_gpu_the_spatial_chips_and_the_engine_share_one_allocator`

**Edge cases.**
- [ ] Alignment and bank constraints (a colour is a byte range, not a slot).
- [ ] Buffers live across a loop back-edge (circular-arc graphs are not interval graphs: allocated with a stated
  bound).
- [ ] Two buffers that may alias by proof (the IR's in-place destinations).

**Success.**
- [ ] Every RED test green (receipt: log paths).
- [ ] Arena and shared-memory footprints never larger than today's on every model (receipt: lane census).

**Mutants to kill.**
- [ ] Two overlapping lifetimes given one range.
- [ ] An overflow witness at an instant where everything fits.

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test allocation_lock
```

**Done when.** Every on-chip buffer on every chip is placed by an allocator whose footprint is proven minimal or whose
refusal names its witness.

#### BEYOND-PROVEN-PROTOCOLS · Every multi-block protocol proven deadlock-free before it is emitted

| Field | Value |
|---|---|
| Phase | beyond |
| Labels | `beyond` `infrastructure` `cuda` `spatial` |
| Depends on | OPT-2 |
| Touches | `crates/logicaffeine_compile/src/spatial/certificate.rs` (reused for GPU protocols); `logicaffeine_verify` (liveness, k-induction, parameterized); the lane's emitter |
| Status | open |

**Goal.** The fastest kernels coordinate blocks and warps:
- last-arriving-block counters (K-03, K-05, K-10);
- grid-wide syncs in persistent kernels (K-22);
- `mbarrier` rings and warp-specialized producer and consumer pipelines;
- split-K semaphores;
- all-reduce flags.

Every one of these protocols is modelled as the marked graph the spatial pipeline already certifies
(`certify_choreography`, `liveness_witness`). It is proven:
- deadlock-free and race-free;
- correct for every buffer depth, parameterized by `logicaffeine_verify`.

The emitter refuses a protocol without its certificate.

**Copy.** Triton checks such protocols with sanitizers at run time (`fp_sanitizer`, `concurrency_sanitizer`, section
11.10). Bar: every protocol Triton's warp-specialized kernels use is expressible and certified here.

**Beyond.** A proof, checked by a small refutation checker, replaces a test that only catches the races it happens to
run into.

**RED tests.**
- [ ] `protocol_proof_lock::every_emitted_counter_protocol_carries_a_liveness_certificate`: fails today because the
  lane emits counters without one.
- [ ] `protocol_proof_lock::a_ring_is_proven_for_every_depth`
- [ ] `protocol_proof_lock::a_protocol_with_a_missing_arrive_is_refused_with_its_deadlock`

**Edge cases.**
- [ ] A self-resetting counter read by the next launch (graph replay).
- [ ] A persistent kernel with fewer resident blocks than its grid (a sync that can never complete: refused).
- [ ] Phase bits wrapping in an `mbarrier` ring.

**Success.**
- [ ] Every RED test green (receipt: log paths).
- [ ] Every lane kernel with a protocol carries its certificate (receipt: lane census).

**Mutants to kill.**
- [ ] An arrive removed from one path.
- [ ] A grid sync with one block too many.

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test protocol_proof_lock
```

**Done when.** No kernel on any chip coordinates its parts by a protocol that is not proven.

#### BEYOND-BIT-EXACT-EQUIVALENCE · Integer kernels proven for every input

| Field | Value |
|---|---|
| Phase | beyond |
| Labels | `beyond` `technique` `cuda` `fabric` |
| Depends on | none |
| Touches | a bit-level lift of kernel fragments to and-inverter graphs; `crates/logicaffeine_proof/src/sweep.rs`, `fraig.rs`, `seq_equiv.rs`, `units.rs` (callers only) |
| Status | open |

**Goal.** Every integer fragment a kernel computes is lifted to an and-inverter graph, mitered against its reference,
and decided by SAT sweeping (`sweep::decide`, `fraig::decide`, LRAT-certified). The fragments:
- block-format decoders;
- LOP3 and PRMT magic-number dequantization;
- the int8 one- and two-word scale folds;
- quantize epilogues;
- MX and NVFP4 conversions;
- the Hadamard int8 cache.

Pipelined fabric datapaths use `seq_equiv::latency_equivalent`. A fragment is proven equal for **every** input, not for
a sample.

**Copy.** vLLM and Triton test such kernels against references on random inputs. Bar: every fragment SETUP-4 measures
has its proof.

**Beyond.** A wrong rounding in a corner case of a 16-bit decoder is caught by the proof before any run.

**RED tests.**
- [ ] `bit_exact_lock::every_block_decoder_is_proven_equal_to_its_reference`: fails today because no lift exists.
- [ ] `bit_exact_lock::the_magic_number_dequantization_is_proven_for_every_byte`
- [ ] `bit_exact_lock::every_quantize_epilogue_rounds_as_its_reference_for_every_input`
- [ ] `bit_exact_lock::a_pipelined_fabric_datapath_is_proven_at_its_latency`

**Edge cases.**
- [ ] Signed zero and NaN encodings in conversions.
- [ ] Saturation at the int8 bounds.
- [ ] A fragment wider than the sweep budget, split at unit boundaries (`units::decide`).

**Success.**
- [ ] Every RED test green (receipt: log paths, LRAT checks).

**Mutants to kill.**
- [ ] A decoder with one wrong table entry.
- [ ] A rounding mode flipped.

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test bit_exact_lock
```

**Done when.** Every integer fragment on every chip is proven equal to its reference for all inputs.

#### BEYOND-SUPEROPTIMIZER · The shortest proven instruction sequence for every hot fragment

| Field | Value |
|---|---|
| Phase | beyond |
| Labels | `beyond` `technique` `cuda` `cpu` |
| Depends on | BEYOND-BIT-EXACT-EQUIVALENCE, SETUP-4 |
| Touches | an offline search tool; a checked-in library of proven sequences used by the CUDA and CPU links; `logicaffeine_synth` (the witness workflow) |
| Status | open |

**Goal.** For each hot fragment:
- enumerate candidate instruction sequences in the chip's instruction set;
- prune by cost;
- prove each survivor equal to the reference with BEYOND-BIT-EXACT-EQUIVALENCE;
- keep the fastest by microbenchmark.

The fragments: the K-23 scale fold (about 12 instructions per m16n8k32 today), block decoders, packed fp8 and fp4
converts, the softmax rescale, top-k key flips. The results are checked in with their proofs and witnesses (the
synth workflow), and the links read them.

**Copy.** The hand-tuned sequences in Marlin's dequantization
(`external/competitors/vllm/csrc/quantization/marlin/dequant.h` · `__device__ inline int lop3`). Bar: no proven
sequence slower than the hand-tuned one.

**Beyond.** The search runs again for every new chip and format, and every result carries its proof.

**RED tests.**
- [ ] `superopt_lock::every_library_sequence_carries_its_proof_and_witnesses`: fails today because the library does
  not exist.
- [ ] `superopt_lock::the_scale_fold_is_no_longer_than_its_proven_minimum`

**Edge cases.**
- [ ] A sequence equal only under a precondition (a range fact from `facts/range.rs`), recorded with it.
- [ ] Chip-specific instructions absent on another chip: a sequence per chip.

**Success.**
- [ ] Every RED test green (receipt: log paths).
- [ ] K-23's fold at or under 8 instructions per m16n8k32 (receipt: SASS census).

**Done when.** Every hot fragment runs its shortest proven sequence on every chip that runs it.

#### BEYOND-OWN-SASS-SCHEDULER · Our own instruction schedule and control codes on sm_86

| Field | Value |
|---|---|
| Phase | beyond |
| Labels | `beyond` `technique` `cuda` |
| Depends on | none |
| Touches | `crates/logicaffeine_compile/src/codegen_sass.rs` |
| Owner notes | the lane's kernels belong to the performance lane; this ticket rewrites their cubins after `ptxas`, never their source |
| Status | open |

**Goal.** `ptxas` decides register allocation and every instruction's control codes: stalls, yields, barriers and the
operand reuse cache. `codegen_sass.rs` already reads and writes those fields on sm_86, verified bit-exact against
`nvdisasm`. It also builds the dependency graph and list-schedules (`precise_dependency_graph`, `list_schedule`,
`assign_minimal_stalls_opt`, `optimize_reuse`).

This ticket puts that to work on the hot loops: the prefill GEMM, prefill attention and the decode GEMVs. It
re-schedules them, assigns minimal stalls and reuse flags, patches the cubin, and proves the rewrite preserves every
dependence.

**Copy.** cuBLAS's hand-scheduled SASS is the bar the module was written for. Bar: our rescheduled kernels at or past
cuBLASLt on the same shapes (SETUP-4).

**Beyond.** Triton cannot reach below `ptxas` (its limit 6 in section 10.3); this does, and every rewrite is checked
against the dependency graph and by a differential run.

**RED tests.**
- [ ] `sass_schedule_lock::a_rescheduled_kernel_preserves_every_dependence`: fails today because no rescheduling pass
  runs on the lane's cubins.
- [ ] `sass_schedule_lock::every_patched_control_field_reencodes_bit_exact`
- [ ] `cuda_lane_lock::a_rescheduled_kernel_computes_what_its_ptxas_kernel_computes`

**Edge cases.**
- [ ] Variable-latency instructions (memory, `MUFU`) waiting on scoreboards, never on stall counts.
- [ ] Reuse flags across a branch target.
- [ ] A driver or `ptxas` version change: the pass refuses an encoding it has not verified.

**Success.**
- [ ] Every RED test green (receipt: log paths).
- [ ] K-23 past 55 TOPS one-word on the exact point (receipt: SETUP-4 row, once SETUP-4 lands).

**Done when.** The hot loops run our schedule, proven to preserve their dependences, at or past cuBLASLt.

#### BEYOND-PROVEN-NUMERICS · Cheaper numerics wherever their error is proven

| Field | Value |
|---|---|
| Phase | beyond |
| Labels | `beyond` `infrastructure` `area:quant` |
| Depends on | OPT-6 |
| Touches | `tensor_graph::facts`; `crates/logicaffeine_compile/src/optimize/accum_bound.rs`; `crates/logicaffeine_proof/src/interval_replay.rs`, `crates/logicaffeine_proof/src/sos.rs` (callers only) |
| Status | open |

**Goal.** Each numerics contract (OPT-6) gets its bound by proof:
- **Value ranges.** Outward-rounded interval replay (`check_range_claim`) bounds every tensor's values from the
  checkpoint's weights and the input domain.
- **Integer widths.** `accum_bound` and Presburger arithmetic prove accumulator widths. For example, an int8 dot
  product of length k fits in int16 for small k, or needs a split every n terms.
- **Approximations.** Sum-of-squares certificates (`sos_certificate`) prove the maximum error of polynomial
  approximations for `exp2`, `tanh`, SiLU and GELU on the ranges the model reaches.

A chip then runs the cheaper form exactly where the proof holds, and the full form elsewhere.

**Copy.** vLLM and Triton pick numerics by accuracy tests. Bar: every model's precision bar met at every knob point
this ticket enables.

**Beyond.** Narrower accumulators and cheaper transcendentals are taken where they are safe for this model's real
ranges, with the proof in the artifact.

**RED tests.**
- [ ] `numerics_proof_lock::every_contract_bound_carries_its_certificate`: fails today because bounds are declared,
  not proven.
- [ ] `numerics_proof_lock::an_approximation_is_used_only_inside_its_proven_range`
- [ ] `numerics_proof_lock::an_accumulator_narrows_only_where_its_width_is_proven`

**Edge cases.**
- [ ] An outlier channel that breaks a range: the proof fails, and the full form stays.
- [ ] Ranges that depend on the prompt: proven over the declared input domain, never a sampled one.

**Success.**
- [ ] Every RED test green (receipt: log paths).
- [ ] Measured decode and prefill gains on the scoreboard from proven narrowing (receipt: scoreboard rows).

**Done when.** Every narrowed numeric on every chip is narrowed by a proof, and nowhere else.

#### BEYOND-LOOP-LEVEL-FUSION · Any producer and consumer loop nests fuse when reordering is proven invisible

| Field | Value |
|---|---|
| Phase | beyond |
| Labels | `beyond` `fusion` |
| Depends on | OPT-1, ONE-PROGRAM-GENERAL-LOOPS |
| Touches | `tensor_graph::fuse`; `crates/logicaffeine_compile/src/optimize/dependence.rs`, `karr.rs`, `deforest.rs`; `crates/logicaffeine_proof/src/omega_solve.rs` (callers only) |
| Status | open |

**Goal.** A rule fuses named symbols. This ticket fuses loop nests the front end does not name. Two adjacent nests
fuse, tiled or interchanged as needed, when the reorder is proven invisible. The proof uses:
- `dependence.rs`'s nest analysis;
- Presburger arithmetic with certificates (`omega_solve`);
- Karr's affine equalities (`karr.rs`).

The result is a fused symbol whose definition is the original nests, linked through the generated-kernel tier on every
chip. An op written in LOGOS that no recognizer knows still fuses with its neighbours.

**Copy.** Polyhedral fusion in research compilers, and `deforest.rs`'s producer-consumer fusion over collections. Bar:
every fusion the rule table makes, a loop-level fusion of the same nests makes too (a differential).

**Beyond.** Fusion is not limited to a vocabulary: the language is the vocabulary.

**RED tests.**
- [ ] `loop_fusion_lock::two_nests_fuse_when_the_reorder_is_proven_invisible`: fails today because nests the front
  end does not lift are lowered apart or refused.
- [ ] `loop_fusion_lock::a_dependence_that_forbids_the_reorder_keeps_the_nests_apart`
- [ ] `loop_fusion_lock::every_rule_fusion_is_also_a_loop_fusion`

**Edge cases.**
- [ ] A consumer reading the producer at an offset (a stencil): fused with a halo.
- [ ] A reduction between the nests: fused only under a declared reassociation contract.
- [ ] Non-affine indices: refused by name.

**Success.**
- [ ] Every RED test green (receipt: log paths).

**Done when.** Any two LOGOS loop nests whose fusion is legal are fused on every chip, by proof.

#### BEYOND-EVALUATOR-SPECIALIZATION · Each chip's generic path is the evaluator, specialized on the graph

| Field | Value |
|---|---|
| Phase | beyond |
| Labels | `beyond` `infrastructure` `cpu` |
| Depends on | IR-2 |
| Touches | `tensor_graph::eval`; `crates/logicaffeine_compile/src/optimize/bta.rs`, `partial_eval.rs`, `supercompile.rs` (callers only) |
| Status | open |

**Goal.** The IR's evaluator (IR-2) defines what a graph means. This ticket partially evaluates it with the graph,
shapes and constants static (binding-time analysis, then the partial evaluator and supercompiler, under the Jones
optimality the Futamura locks hold). The output is straight-line code for that graph: the generic code path of the VM,
of the C and Rust AOT backends, and the seed of the generated-kernel tier.

Equal to the evaluator by construction, it is the fallback every chip has for every op, on its first day.

**Copy.** None: other compilers write their generic paths by hand. Bar: the specialized program runs every tiny cut
equal to the evaluator, and faster than interpreting it.

**Beyond.** Every chip's fallback path is correct by construction, and gets faster as the partial evaluator improves.

**RED tests.**
- [ ] `evaluator_specialization_lock::the_specialized_program_computes_what_the_evaluator_computes`: fails today
  because the evaluator does not exist yet (IR-2).
- [ ] `evaluator_specialization_lock::the_residual_program_has_no_interpretive_overhead` (Jones optimality on the
  corpus).

**Edge cases.**
- [ ] A dynamic shape (a sequence length): left dynamic, specialized on everything else.
- [ ] Constant weights: never folded into code; read through the storage closure.

**Success.**
- [ ] Every RED test green (receipt: log paths).

**Done when.** Every chip's generic path is the evaluator specialized on the graph.

#### BEYOND-EXACT-WEIGHT-FOLDS · Algebraic weight rewrites proven over the rationals

| Field | Value |
|---|---|
| Phase | beyond |
| Labels | `beyond` `fusion` |
| Depends on | OPT-4, OPT-6 |
| Touches | `tensor_graph::fuse` (rewrites over constant tensors); `crates/logicaffeine_proof/src/linalg_certify.rs` (callers only) |
| Status | open |

**Goal.** Rewrites that change weights instead of kernels, each proven exactly over ℚ (`linalg_certify`) and bounded
under the chip's numerics:
- an RMSNorm's weight folded into the next projection (W·diag(γ));
- MLA's key absorption folded into the query projection;
- Hadamard rotations folded into weights for rotated quantization;
- biases folded into following affine ops;
- merged projections laid out once.

A rewrite is a graph rewrite on the one program, so every chip receives it.

**Copy.** vLLM absorbs MLA's keys in its decode path
(`external/competitors/vllm/vllm/model_executor/layers/attention/mla_attention.py`); rotated quantization folds
rotations offline. Bar: every fold equal to its unfolded graph within the declared bound on every model.

**Beyond.** Each fold carries a kernel certificate of its algebra, and the e-graph (OPT-4) chooses folds by each chip's
cost instead of by hand.

**RED tests.**
- [ ] `weight_fold_lock::every_fold_carries_its_exact_certificate`: fails today because no fold is a rewrite.
- [ ] `weight_fold_lock::a_folded_graph_computes_what_its_unfolded_graph_computes_within_its_bound`

**Edge cases.**
- [ ] A norm weight shared by two projections (fold into both, or keep).
- [ ] A quantized weight: folded before quantization, with the requantization error inside the bound.

**Success.**
- [ ] Every RED test green (receipt: log paths).

**Done when.** Every weight fold on every chip is a proven rewrite, chosen by cost.

#### BEYOND-SPATIAL-ON-GPU · The GPU lowered as a spatial fabric

| Field | Value |
|---|---|
| Phase | beyond |
| Labels | `beyond` `cuda` `spatial` |
| Depends on | MIGRATE-3, BEYOND-PROVEN-PROTOCOLS, BEYOND-CERTIFIED-ALLOCATION |
| Touches | `crates/logicaffeine_compile/src/spatial/lower/cuda.rs` (`CudaPersistentBackend`); `spatial/work.rs`; `spatial/residency.rs` |
| Status | open |

**Goal.** A persistent GPU kernel is a spatial fabric:
- SMs are sites;
- L2 and global memory are the network;
- flags and counters are handshakes;
- shared memory is residency.

The spatial pipeline already places, routes, persists and certifies programs on Tensix, RDU and fabric chips, and
`CudaPersistentBackend` already lowers spatial programs to CUDA. This ticket lowers a whole decode step that way:
- the persistent layer megakernel of K-22 comes from the spatial pipeline, not a new mechanism;
- the counted work model (`work_of_plan`) is its cost;
- its handshakes carry BEYOND-PROVEN-PROTOCOLS's certificate.

**Copy.** vLLM's hand-written AMD decode megakernel for one model (section 4.9). Bar: K-22's ≥ 96% of the decode
floor on Llama-3.2-1B and Qwen3.5-0.8B.

**Beyond.** One dataflow scheduler for every chip. A GPU megakernel is generated for any model the spatial pipeline
lowers, and proven deadlock-free.

**RED tests.**
- [ ] `spatial_gpu_lock::a_decode_step_lowers_to_one_persistent_kernel_per_layer_kind`: fails today because the
  persistent backend lowers GEMM-class programs only.
- [ ] `spatial_gpu_lock::the_persistent_kernel_computes_what_the_evaluator_computes`
- [ ] `spatial_gpu_lock::every_handshake_carries_its_certificate`

**Success.**
- [ ] Every RED test green (receipt: log paths).
- [ ] K-22's bar met (receipt: scoreboard rows).

**Done when.** The decode step runs on the GPU as a certified spatial program at or past the graph-replay lane.

#### BEYOND-RUNTIME-PATCHING · Kernels specialized at serving time in microseconds

| Field | Value |
|---|---|
| Phase | beyond |
| Labels | `beyond` `serving` `cuda` |
| Depends on | SERVE-CONTINUOUS-BATCHING, BEYOND-OWN-SASS-SCHEDULER |
| Touches | a GPU stencil library beside the lane; `crates/logicaffeine_forge/src/patch.rs` (the copy-and-patch model); `logicaffeine_synth` (witness verification) |
| Status | open |

**Goal.** The copy-and-patch JIT already specializes VM code by patching precompiled stencils, each stencil verified
by three evaluators (`logicaffeine_synth`'s witness harness). This ticket carries that model to GPU kernels:
- precompiled cubins with patch sites for batch size, sequence lengths, strides and pointers;
- patched in microseconds when the batch changes;
- every stencil verified the same way.

**Copy.** Triton compiles a kernel per specialization at run time (limit 13 in section 10.3), and vLLM captures a CUDA
graph per batch size. Bar: a batch-size change costs less than one kernel launch.

**Beyond.** Continuous batching runs fully specialized kernels for every batch shape, without a compile and without a
graph per shape.

**RED tests.**
- [ ] `gpu_patch_lock::a_patched_kernel_computes_what_its_compiled_kernel_computes`: fails today because no GPU
  stencil exists.
- [ ] `gpu_patch_lock::every_patch_site_is_verified_by_its_witnesses`
- [ ] `gpu_patch_lock::a_batch_change_patches_in_less_than_a_launch`

**Success.**
- [ ] Every RED test green (receipt: log paths).

**Done when.** Serving specializes its kernels to every batch shape at patch speed, verified.

#### BEYOND-SYMMETRY-PRUNED-SEARCH · Search spaces cut by proven symmetry

| Field | Value |
|---|---|
| Phase | beyond |
| Labels | `beyond` `infrastructure` |
| Depends on | OPT-7 |
| Touches | the tuning database's search; `crates/logicaffeine_proof/src/symmetry_detect.rs`, `sym_break.rs`, `permgroup.rs` (callers only) |
| Status | open |

**Goal.** Tuning and layout searches range over spaces full of equivalent points:
- tile shapes that swap under a transpose;
- layouts that differ by a permutation the kernel ignores;
- identical layers.

This ticket:
- detects each space's automorphism group (`automorphism_group`);
- keeps one point per orbit with complete lex-leader breaking;
- compiles isomorphic subgraphs once.

The search then reports that it covered every distinct candidate.

**Copy.** Triton's autotuner prunes by shared-memory fit and a latency model (section 10.3). Bar: no tuned choice
worse than an exhaustive search over the same space.

**Beyond.** "Exhaustive" becomes affordable, and provably so.

**RED tests.**
- [ ] `tuning_search_lock::every_orbit_is_searched_exactly_once`: fails today because the search has no symmetry
  reduction.
- [ ] `tuning_search_lock::isomorphic_subgraphs_compile_once`

**Success.**
- [ ] Every RED test green (receipt: log paths).

**Done when.** Every tuning search is exhaustive over distinct candidates, and says so.

#### BEYOND-TRAINING · The backward program through the same optimizer and chips

| Field | Value |
|---|---|
| Phase | beyond |
| Labels | `beyond` `fusion` `cuda` `spatial` |
| Depends on | OPT-1, MIGRATE-1 |
| Touches | `crates/logicaffeine_compile/src/tensor_graph/vjp.rs`; `tensor_graph::fuse` |
| Status | open |

**Goal.** `vjp.rs` already writes a forward program's backward pass as ordinary LOGOS. This ticket runs that backward
program, and a training step, through the same front end, optimizer, shims and links as inference. Fusions include
attention backward, norm backward with deterministic weight-gradient reduction, fused cross-entropy, and the optimizer
step.

**Copy.**
- Triton's layer-norm backward (`external/competitors/triton/python/tutorials/05-layer-norm.py` · `def _layer_norm_bwd_dx_fused`).
- Triton's fused-attention tutorial, which carries its backward (`external/competitors/triton/python/tutorials/06-fused-attention.py` · `def _attn_fwd_inner`).
- Bar: at or past those kernels at our shapes.

**Beyond.** Gradients come from the same one program, so a model trained here and served here never changes
representation. The backward's reductions are deterministic by construction.

**RED tests.**
- [ ] `training_lock::the_backward_program_lowers_through_the_shared_optimizer`: fails today because only the forward
  lowers.
- [ ] `training_lock::every_gradient_equals_its_finite_difference_within_its_bound`
- [ ] `training_lock::the_weight_gradient_reduction_is_deterministic`

**Success.**
- [ ] Every RED test green (receipt: log paths).
- [ ] Layer-norm backward and attention backward at or past Triton's (receipt: SETUP-4 rows).

**Done when.** A training step of a scoreboard model runs from the one program on every chip that can hold it.

### 12.18 One program on every backend

These tickets close section 3's gaps. Their stream is S2 (section 1.5).

#### ONE-PROGRAM-VM-REFERENCE · The VM is every backend's reference, directly

| Field | Value |
|---|---|
| Phase | one program |
| Labels | `infrastructure` |
| Depends on | none |
| Touches | `crates/logicaffeine_tests/tests/common/tiny_models.rs`; `one_program_lock`; `model_conformance_lock` |
| Status | open |

**Goal.**
- **Every cut family on the VM.** Decode, prefill, batch, forward, and the split cuts of every architecture run on the
  VM at tiny width, from `model::author`'s text. Today the VM covers prefill and decode for seven shapes, and the
  Qwen3.8 tiny cuts.
- **The JIT agrees.** The tree-walker and the JIT agree with the VM, bit for bit.
- **Backends against the VM.** Each backend that runs a tiny cut is compared with the VM's outputs directly, not only
  through `model::reference`.
- **A real-width golden.** One real-width decode token of Llama-3.2-1B and Qwen3.5-0.8B runs on the VM with the JIT,
  on demand, as a golden that needs no other engine.

**Read first.**
- `model_conformance_lock` (the seven shapes, the VM held to the reference within 1e-9);
- `qwen38_decode_step_lock`, `qwen38_prefill_lock`, `qwen38_batched_decode_lock`;
- `crates/logicaffeine_compile/src/compile.rs` (`vm_outcome`, `tw_outcome`).

**RED tests.**
- [ ] `one_program_lock::every_cut_family_of_every_architecture_runs_on_the_vm`: fails today on the batch and split
  cuts outside Qwen3.8.
- [ ] `one_program_lock::the_tree_walker_the_vm_and_the_jit_agree_on_every_tiny_cut`: fails today because no lock
  installs the JIT under an author cut.
- [ ] `one_program_lock::the_lane_equals_the_vm_on_every_tiny_cut`: fails today because the lane is held to the
  reference only.
- [ ] `one_program_lock::the_engine_equals_the_vm_on_every_tiny_decode_step`
- [ ] `one_program_lock::the_sn50_path_equals_the_vm_on_every_tiny_cut`
- [ ] `model_golden_lock::a_real_width_token_on_the_vm_matches_the_reference` (on demand, under `#[ignore]` with its
  runner named)

**Edge cases.**
- [ ] A cut whose placeholders (token and position 1) hide a position-dependent bug: also run at position 0 and at the
  last cache row.
- [ ] Mixture-of-experts routing ties: the VM and every backend break them identically.
- [ ] Numerics that a backend declares inexact: compared within its declared bound, never bit for bit by accident.

**Success.**
- [ ] Every RED test green (receipt: log paths).
- [ ] The real-width golden recorded for both models (receipt: log path).

**Mutants to kill.**
- [ ] A backend compared with itself instead of the VM.
- [ ] A cut family dropped from the matrix.

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test one_program_lock --test model_conformance_lock
```

**Done when.** Every backend's every tiny cut is held to the VM running the one program's text.

#### ONE-PROGRAM-EVERY-BACKEND · One command lowers a checkpoint to every backend

| Field | Value |
|---|---|
| Phase | one program |
| Labels | `infrastructure` |
| Depends on | ONE-PROGRAM-VM-REFERENCE |
| Touches | `crates/logicaffeine_compile/src/bringup.rs`; `apps/logicaffeine_cli/src/commands/bringup.rs`; `apps/logicaffeine_cli/src/commands/emit.rs`; `docs/cli.md`; `bringup_lock`; `one_program_lock`; a new `cli_emit_model_lock` |
| Status | open |

**Goal.** Author once, emit anywhere (section 3.6).
- `largo emit <target> --model <checkpoint>` works for every emit target: the authored program, the target's
  artifacts, and the fusion report. Today `--model` is refused for every target but `rdu`.
- `largo bringup` lowers a checkpoint's cuts to every backend of section 3.3, and prints the matrix: model × cut ×
  backend, each cell lowered with its identity or refused by name.
- **Today's three.** The CUDA lane, the D5005 engine and the SN50 path keep their entries.
- **New entries:**
  - the VM and the JIT;
  - the CPU through the C and Rust backends (the generic path first; PARITY-CPU-SHIM's links when they land);
  - RISC-V through its AOT tier, under qemu and on the soft core;
  - WebAssembly;
  - CUDA persistent, Tenstorrent and the fabric through the spatial pipeline;
  - the general-program tiers, each running the cut or refusing it with its reason;
  - AMD and Intel, as refusals naming their ticket until their shims exist.
- **Identity everywhere.** Every artifact on every backend carries its cut's identity line.

**Read first.**
- `crates/logicaffeine_compile/src/bringup.rs` (`Backend`, `backends`, `bring_up`);
- `bringup_lock` (one checkpoint on three chips is one program; refusals named once);
- section 3.3.

**RED tests.**
- [ ] `bringup_lock::every_backend_in_the_book_is_a_bringup_backend`: fails today, since three of section 3.3's backends
  are bring-up backends.
- [ ] `cli_emit_model_lock::every_emit_target_takes_a_model_or_refuses_it_by_name`: fails today on the usage error
  every target but `rdu` returns.
- [ ] `cli_emit_model_lock::an_emitted_tiny_model_runs_equal_to_the_vm_on_every_target_this_box_runs`
- [ ] `one_program_lock::every_backend_lowers_every_model_or_refuses_by_name`
- [ ] `one_program_lock::every_backend_equals_the_vm_on_every_tiny_cut`
- [ ] `one_program_lock::every_backend_artifact_names_its_cut`

**Edge cases.**
- [ ] A backend that lowers some layers and refuses others: the refusal names the layers it stands in for (the
  existing `a_refusal_names_each_gap_once_with_the_layers_it_stands_in` pattern).
- [ ] A backend needing hardware the box does not have: lowered and compile-checked here, run where the hardware is.
- [ ] A cut too large for a general-program tier: refused with the size, never attempted.

**Success.**
- [ ] Every RED test green (receipt: log paths).
- [ ] The matrix printed for every scoreboard model, every cell lowered or refused by name (receipt: log path).

**Mutants to kill.**
- [ ] A backend that silently skips a cut.
- [ ] An artifact whose identity line names another cut.

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test bringup_lock --test one_program_lock --test cli_emit_model_lock
```

**Done when.** One command takes a checkpoint to every backend, and every result is the one program or a named
refusal.

#### ONE-PROGRAM-RETIRE-SECOND-WRITERS · No product path runs a program `model::author` did not write

| Field | Value |
|---|---|
| Phase | one program |
| Labels | `infrastructure` |
| Depends on | ONE-PROGRAM-EVERY-BACKEND, MIGRATE-2, MIGRATE-3 |
| Touches | `crates/logicaffeine_compile/src/spatial/int8_model.rs` callers; `scripts/accel/gen_qwen_tiny_lg.py` and `scripts/gen_tiny_transformer_lg.py` callers; the hand-written CUDA lanes' callers; `one_program_lock` |
| Owner notes | the hand-written lanes are the performance lane's oracles; they retire only when the one program's lowering beats them on their own numbers, and Tristen agrees |
| Status | open |

**Goal.** Every writer besides `model::author` leaves the product path:
- **`Int8Model::decode_step_lg`.** The CUDA persistent backend and the fabric board images run the baby transformer
  from this writer. They run `model::author`'s tiny cuts instead, and the int8 model becomes a spec the author writes.
- **The two Python writers.** `gen_qwen_tiny_lg.py` and `gen_tiny_transformer_lg.py` feed the single-thread CUDA tier
  and the WPE engine. Their locks move to author cuts.
- **The SN50 path.** It reads cuts of any architecture's program, not only through `Qwen38Cfg`.
- **The four hand-written CUDA files.** They stay golden oracles, and the closed set shrinks as each is beaten.

**Read first.**
- `one_program_lock::{only_the_one_program_calls_the_authors_writers, the_hand_written_lanes_are_a_closed_set}`;
- `model_author_migration_lock` (how the Qwen3.8 generators became wrappers over the author, pinned byte-identical).

**RED tests.**
- [ ] `one_program_lock::no_backend_runs_a_program_from_a_second_writer`: a census over the product sources for
  `decode_step_lg` and the Python writers' outputs. It fails today on the CUDA persistent, fabric, single-thread CUDA
  and WPE locks.
- [ ] `one_program_lock::the_sn50_path_lowers_any_architecture_the_author_writes`: fails today because
  `Qwen38Cfg::from_spec` refuses non-hybrid models.

**Edge cases.**
- [ ] A lock whose golden numbers came from a second writer: its goldens are regenerated from the author's program,
  and the change is shown to Tristen before it lands.
- [ ] A board image size limit the author's tiny cut exceeds: a smaller tiny spec, never a second writer.

**Success.**
- [ ] Every RED test green (receipt: log paths).
- [ ] The second-writer census empty on product paths (receipt: lock output).

**Mutants to kill.**
- [ ] A new call to `decode_step_lg` from a backend.
- [ ] A hand-written lane added outside the closed set.

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test one_program_lock --test model_author_migration_lock
```

**Done when.** Every artifact on every product path is lowered from `model::author`'s program.

#### ONE-PROGRAM-GENERAL-LOOPS · A program no recognizer lifts still lowers everywhere

| Field | Value |
|---|---|
| Phase | one program |
| Labels | `infrastructure` |
| Depends on | IR-2, TRITON-TILE-TIER |
| Touches | `tensor_graph::front` (a loop-nest op); each chip's link for it |
| Status | open |

**Goal.** Today a statement the front end cannot lift into one of the 28 op kinds is refused. A backend may also refuse
an op kind it has no kernel for: the CUDA lane refuses six, and the older forward twenty. This ticket closes both:
- the front end keeps an unrecognized loop nest as a loop-nest op, defined by its own statements and evaluated by the
  IR's evaluator;
- every chip links it: the generated-kernel tier on CUDA, AMD and Intel; generated code on the CPU; a generic section
  on the spatial chips; a refusal by name only where the chip cannot run loops at all (the D5005 engine's fixed
  commands);
- each op kind a backend refuses today gets a link or a decomposition.

Every LOGOS program then runs on every chip that can run it, and the one-program claim holds for programs the
recognizers never saw.

**Read first.**
- `crates/logicaffeine_compile/src/tensor_graph/front.rs` (the 28 kinds);
- the CUDA lane's refusals in `codegen_cuda/lane/plan.rs`;
- TRITON-TILE-TIER.

**RED tests.**
- [ ] `one_program_lock::an_unrecognized_loop_nest_lowers_on_every_backend_that_runs_loops`: fails today on the
  refusal.
- [ ] `lane_compile_lock::every_op_kind_links_or_decomposes`: fails today on the six refused kinds.
- [ ] `tensor_graph_eval_lock::a_loop_nest_op_evaluates_as_its_statements`

**Edge cases.**
- [ ] A loop nest with data-dependent bounds: a dynamic dimension, never a refusal.
- [ ] A loop nest writing state: an in-place destination, under the IR's state rules.
- [ ] A loop nest that is really a known op: the recognizer still wins, and the loop path is the fallback.

**Success.**
- [ ] Every RED test green (receipt: log paths).

**Mutants to kill.**
- [ ] A loop nest dropped instead of lowered.
- [ ] A refused op kind silently replaced by a no-op.

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test one_program_lock --test lane_compile_lock --test tensor_graph_eval_lock
```

**Done when.** No backend that can run loops refuses a LOGOS program because no recognizer knew it.

#### ONE-PROGRAM-FUSION-MAP-LOCK · Section 3.4 is the shims' answers

| Field | Value |
|---|---|
| Phase | one program |
| Labels | `infrastructure` |
| Depends on | OPT-2, LEDGER-2 |
| Touches | `shim_lock`; this book's section 3.4 |
| Status | done |

**Goal.** Section 3.4 maps every fused symbol onto every chip family. This lock reads that map and asks every shim
(OPT-2) for its answer for each class: native, parts or refused. It fails on any disagreement, either way:
- a box ticked that the shim does not answer natively;
- a native answer whose box is still open.

The map therefore always says what the code does.

**RED tests.**
- [x] `shim_lock::the_books_fusion_map_is_every_shims_answers`: it failed on 26 of the map's boxes (RED receipt:
  logs/one-ir/fusion-map-lock-red-1027.log), which now say what the shims answer.
- [x] `shim_lock::every_class_in_the_ledger_has_a_row_in_the_map`

**Edge cases.**
- [x] A chip family of several shims (the spatial chips): the row's box ticks only when every shim in the family
  answers natively, and each shim's answer is listed where they differ.
- [x] A class a shim answers differently by knob point (int8 against f16 numerics): both answers recorded (the lane's
  quantize epilogues: parts with `fuse_quant` off).

**Success.**
- [x] Both RED tests green (receipt: logs/one-ir/opt5-run2-1027.log).

**Mutants to kill.**
- [x] A ticked box with a parts answer (killed: logs/one-ir/mutants-opt5b-1027.log).
- [x] A class missing from the map (killed).

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test shim_lock
```

**Done when.** The book's map of fusions to chips cannot drift from the shims.

### 12.19 Chip-own paths: the D5005 engine and the spatial mesh

#### D5005-FUSED-COMMANDS · The engine fuses what every other chip fuses

| Field | Value |
|---|---|
| Phase | chip path |
| Labels | `fusion` `d5005` |
| Depends on | MIGRATE-4 |
| Touches | `crates/logicaffeine_compile/src/spatial/decode_engine/` (peer-owned: handoffs only) |
| Owner notes | the engine files belong to the engine session; every change is a lanes-board handoff, priced in ALMs before it lands (K-35) |
| Status | open |

**Goal.** Today the engine merges seven groups into commands. It runs these apart, each a pass over its row:
- the residual add and the norm;
- the FFN's gate, up and SwiGLU;
- the L2 head norms;
- the head norms with the rotation.

This ticket gives each a command:
- `AddNorm`;
- `GluAct` (gate and up from one matrix pass, the activation in the vector unit's same pass);
- `HeadNorm`, with an optional rotation.

Each reads the fused symbol from the shared table (MIGRATE-4), so the engine's fusions are the shared table's. The units
are pipelined to the stream clock by the exact scheduler (K-34).

**Copy.** The CUDA lane's `add_norm`, `glu_pair` and `head_norm_rope` rules (section 5.9); the engine's own `Gates` and
`SigmoidGate` commands as the pattern. Bar: each command bit-identical to the commands it replaces at every knob point.

**Beyond.** The vector unit's share of the step falls by the passes removed. Each fusion is a command on a streaming
engine, not a kernel, so it costs ALMs once rather than launches every step.

**RED tests.**
- [ ] `d5005_lower_graph_lock::an_add_then_norm_lowers_to_one_command`: fails today because they lower apart.
- [ ] `d5005_lower_graph_lock::a_gated_ffn_lowers_to_one_glu_command`
- [ ] `d5005_lower_graph_lock::a_head_norm_and_its_rotation_lower_to_one_command`
- [ ] `d5005_machine_lock::every_fused_command_computes_what_its_parts_compute_at_every_knob_point`
- [ ] `d5005_engine_rtl_lock::every_fused_command_unit_equals_the_machine`

**Edge cases.**
- [ ] A norm whose output feeds two consumers (the residual stream and the next projection).
- [ ] Rotation sections (Qwen3.8's interleaved sections) inside the head-norm command.
- [ ] Every format the matrix units read (Q4_K, Q5_K, Q6_K, Q8_0).

**Success.**
- [ ] Every RED test green (receipt: log paths).
- [ ] The twin's vector share down by the removed passes, and the build within K-35's ALM bar (receipt: `predicted.json`,
  `quartus_builds.json`).

**Mutants to kill.**
- [ ] A fused command that skips the residual write.
- [ ] A rotation applied before the norm.

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test d5005_lower_graph_lock --test d5005_machine_lock --test d5005_engine_rtl_lock
```

**Done when.** The engine runs no op chain apart that the shared table fuses and its command set can hold.

#### SPATIAL-NONUNIFORM-TILING · Every core of the mesh at work

| Field | Value |
|---|---|
| Phase | chip path |
| Labels | `technique` `spatial` `tensix` |
| Depends on | none |
| Touches | `crates/logicaffeine_compile/src/spatial/passes/tile.rs`; `spatial/passes/place.rs`; the Tensix and RDU lowerings |
| Status | open |

**Goal.** The tile pass picks grid dimensions that divide the tile count (`ny = mt/psm`, `nx = nt/psn`). The results:
- Blackhole averages 57% of its 130 cores, and none of 256 shapes reaches all of them;
- Wormhole averages 80.7% (simulator grid 71.8%).

This ticket adds non-uniform decomposition: rows of tiles of two sizes, or a remainder band. A shape then uses the
cores that fit it, not only those that divide it. The IR already places sections at arbitrary sites.

**Copy.** None to copy: the measurement is ours. Bar: the tile pass's chosen grid on every scoreboard GEMM shape uses
≥ 85% of the cores on Blackhole and on Wormhole.

**Beyond.** 1.52× the cores on Blackhole at the same per-core program. The certificate (liveness, buffer balance)
covers non-uniform placements as it covers uniform ones.

**RED tests.**
- [ ] `spatial_tile_lock::a_shape_that_does_not_divide_the_mesh_still_fills_it`: fails today because only divisors are
  reachable.
- [ ] `spatial_tile_lock::a_non_uniform_grid_computes_what_the_uniform_grid_computes`
- [ ] `spatial_tile_lock::every_non_uniform_placement_certifies`

**Edge cases.**
- [ ] A remainder band of one tile row.
- [ ] Multicast rectangles over non-uniform bands (Wormhole's interconnect).
- [ ] The RDU's crossbar, where placement is free but bandwidth per site is not.

**Success.**
- [ ] Every RED test green (receipt: log paths).
- [ ] Core use ≥ 85% on both chips across the scoreboard shapes (receipt: the tile report).

**Mutants to kill.**
- [ ] A band whose tiles overlap the uniform region.
- [ ] A remainder dropped.

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test spatial_tile_lock
```

**Done when.** No spatial chip idles cores because a shape does not divide its mesh.

### 12.20 The demos: every slide claim is a measurement

Three demos carry the product to a room (Tristen, 2026-10-26):
- **Demo 1, the fusion across chips:** a model written once in English; one optimizer's fusions on every chip; per
  chip, logits at least as good as the rivals' and speed and throughput that match or beat them.
- **Demo 2, the spatial fabric:** the same LOGOS program on one board as a generated soft-core CPU and as a generated
  spatial fabric, compared in cycles, LUTs and clock. No energy or power figure appears.
- **Demo 3, proof:** the Lean kernel on the Arena's own metric, certified SAT, and the compiler's own certificates.

A slide prints only what a ledger row holds and a lock derives. The tickets below close the gaps between today's
receipts and those slides. Each one's stream is on its board index line; most are S15.

#### FABRIC-REAL-CYCLE · The fabric's cycle count is measured on the board

| Field | Value |
|---|---|
| Phase | demo |
| Labels | `infrastructure` `fabric` |
| Depends on | none |
| Touches | `crates/logicaffeine_compile/src/codegen_sva/fabric_soc.rs` (`emit_nexys_fabric_soc` only); `benchmarks/silicon/{nexys_fabric_soc.v,nexys_fabric_soc.bit,fabric_perf.json}`; `scripts/fabric-host.py`; `scripts/validate-on-nexys-fabric.sh`; `fabric_perf_lock` |
| Owner notes | Board B: post BOARD-B START and FINISH on the lanes board around every flash |
| Status | claimed |

**Goal.** The board top streamed `localparam CYCLES = 16'd1` and the host compared it with 1, so "the fabric decides in
one clock" was never measured: the result bytes were read from the combinational cone thousands of clocks after its
inputs settled. The board top now measures its own settle: for every vector it primes the launch register (the
vector's inputs inverted, then the next vector), launches the vector, and captures the array's output `L` clocks
later for `L` from `LMAX − 1` down to 1, `REPS` times. The settled value is the capture at `2·LMAX`, and it counts only
when the capture at `LMAX` already equals it under both primes. The streamed cycle count is the smallest `L` from
which every capture equals the settled value, the worst over the repetitions; 0 marks a result that did not settle
inside the window.

**RED tests.**
- [x] `fabric_perf_lock::the_fabric_cycle_count_is_read_from_a_counter_the_rtl_drives`: under iverilog, the board top
  around a `config_fabric` whose outputs arrive 0, 15, 35, 95 and 200 ns after its inputs (10 ns clock) streams the
  VM's answers with 1, 2, 4 and 10 cycles, and 0 past the 16-clock window. It failed on the old board top, which
  streamed the constant (receipt: `logs/demo-strategy/red-fabric-perf.log`; with a 95 ns fabric the old top streamed
  every result correct at "1 cycle").

**Edge cases.**
- [x] A vector whose two primes compute the same value (`clamp(20, 0, 10)`: both primes give −1): the double
  reference capture still marks a slow fabric unsettled.
- [ ] `ratio`'s 64-bit combinational divider (about 640 ns): reports its measured clocks, never 1.
- [ ] A setup violation on silicon (varying high bits, as `nexys_fabric_soc_slow.v` records): the settled value is
  checked against the VM by the host, and a capture that varies across repetitions marks the vector unsettled.

**Success.**
- [x] The RED test green (receipt: `logs/demo-strategy/green-fabric-perf.log`).
- [ ] The image rebuilt through `scripts/openxc7-build.sh` with its `.build.txt` recipe, flashed to Board B, and the
  host's receipt recorded (receipt: `RECORD=… scripts/validate-on-nexys-fabric.sh`, the JSON receipt path).
- [ ] `fabric_perf.json`'s `fabric_cycles` derived per datapath from that receipt (the worst vector), with the
  receipt's bitstream digest, and `fabric_perf_lock` deriving every speedup from it.

**Mutants to kill.**
- [ ] The cycle field streamed as a constant again.
- [ ] The capture taken one clock late (the count off by one).
- [ ] The `LMAX` reference check removed.

**Verify.**
```
cargo nextest run -p logicaffeine-tests --test fabric_perf_lock --test measured_fields_lock
```

**Done when.** Every fabric cycle count a slide could quote was counted by the board that produced it.

#### FABRIC-SETTLE-PROBE · One settle probe for every combinational board top

| Field | Value |
|---|---|
| Phase | demo |
| Labels | `infrastructure` `fabric` |
| Depends on | FABRIC-REAL-CYCLE |
| Touches | `crates/logicaffeine_compile/src/codegen_sva/` (a settle-probe emitter beside `fabric_soc.rs`); `benchmarks/silicon/{nexys_conv_soc,nexys_sobel_soc,nexys_stream_soc,ours_adder,ours_affine,ours_ratio}.v`; their hosts in `scripts/`; `measured_fields_lock`'s allowlist |
| Status | open |

**Goal.** Six more board tops stream `localparam CYCLES = 16'd1` (the conv and Sobel stencils, the streaming tile, and
three place-and-route probes). The settle sweep of FABRIC-REAL-CYCLE becomes one emitted probe (launch register, the
two primes, the descending scan, the double reference) that any combinational datapath plugs into, and every one of
those tops adopts it. `measured_fields_lock`'s allowlist empties.

**RED tests.** Staged behind `handed-reds` (section 1.1); RED receipt: `logs/demo-strategy/handed-reds-red.log`.
- [ ] `fabric_settle_probe_lock::no_board_top_streams_a_constant`: the census with no allowlist. Fails today on the six
  tops.
- [ ] `fabric_settle_probe_lock::every_combinational_top_measures_its_settle`: the conv and Sobel tops, around their
  stencils slowed by 0, 15, 35, 95 and 200 ns (`tests/common/board_settle.rs`), stream 1, 2, 4, 10 and 0 cycles and
  unmoved results. Fails today: they take no `LMAX` or `REPS`, and stream a constant.
- [ ] `measured_fields_lock::the_allowlist_only_names_boards_that_still_need_it` stays green while each top leaves the
  list.

**Edge cases.**
- [ ] The streaming tile (`nexys_stream_soc`): a pipelined top measures throughput (clocks between valid outputs on a
  buffered tile), not settle; the probe's interface says which it measures.
- [ ] The place-and-route probes (`ours_*`): their bitstreams belong to the place-and-route regression; rebuilding them
  is coordinated with that repository's owner.

**Success.**
- [ ] `measured_fields_lock`'s `STILL_CONSTANT` is empty (receipt: the lock's source and log).

**Done when.** No board top in `benchmarks/silicon/` streams a constant where a measurement belongs.

#### FABRIC-BUILD-RECIPES · Every demo image names its build and its resources

| Field | Value |
|---|---|
| Phase | demo |
| Labels | `infrastructure` `fabric` |
| Depends on | FABRIC-REAL-CYCLE |
| Touches | `benchmarks/silicon/*.bit.build.txt` for the demo images; `fabric_perf_lock` |
| Status | open |

**Goal.** `fabric_perf.json` quotes 737 LUTs for the fabric image, a figure from an older four-datapath array; the
image on the board adds the divider (38,908 LUTs by the same ledger). Every image a demo flashes carries the recipe
`scripts/openxc7-build.sh` writes (tool, seed, router, the RTL's digest, nextpnr's utilisation and maximum frequency),
and every LUT, flip-flop and clock figure a ledger quotes is read from that recipe.

**RED tests.** Staged behind `handed-reds` (section 1.1); RED receipt: `logs/demo-strategy/handed-reds-red.log`. The recipe gains `rtl sha256` of its sources and nextpnr's
utilisation lines (`scripts/openxc7-build.sh` keeps only the last four matching lines today, the frequency lines).
- [ ] `fabric_build_recipes_lock::every_demo_image_has_a_recipe_that_names_its_sources`: the fabric, state, float and
  soft-core images, each digest equal to its committed sources'.
- [ ] `fabric_build_recipes_lock::every_recipe_records_its_resources`.
- [ ] `fabric_build_recipes_lock::every_lut_figure_in_fabric_perf_is_its_images_recipes`.

**Edge cases.**
- [ ] An image built by our own place and route (`ours_flow`): its recipe names that flow.
- [ ] A recipe whose RTL digest no longer matches the committed sources: the image is stale and the lock says so.

**Done when.** No resource or clock figure about a board image is typed.

#### FABRIC-AUTHOR-MODEL · The fabric generates tokens from a program `model::author` writes

| Field | Value |
|---|---|
| Phase | demo |
| Labels | `infrastructure` `fabric` `spatial` |
| Depends on | AUTHOR-INTEGER-POINT, FABRIC-REAL-CYCLE |
| Touches | the spatial pipeline's fabric lowering of an author cut; the board images and hosts for it; `one_program_lock`'s second-writer census; a new `fabric_model_lock` |
| Owner notes | `one_program_lock` belongs to the one-program stream (S2): its census change is a handoff |
| Status | open |

**Goal.** Today the boards run the int8 baby transformer from a second writer (`Int8Model::decode_step_lg`). A small
trained llama-architecture checkpoint (stories260K in block RAM, or stories42M on Board B's DDR2; the ticket measures
both and picks by fit) is written by `model::author` at its integer knob point (AUTHOR-INTEGER-POINT) and runs twice on
one board: on the generated soft core (the RISC-V tier) and on the generated fabric (matrix products on a DSP array,
row work on the engine; section 4.16, K-44). Both generate the VM's tokens, each timed by an on-chip counter.

**RED tests.**
- [ ] `fabric_author_model_lock::no_board_image_or_gpu_program_comes_from_a_second_writer`. Staged behind `handed-reds` (section 1.1); RED receipt: `logs/demo-strategy/handed-reds-red.log`. Fails
  today on `crates/logicaffeine_compile/examples/decode_board_image.rs` and
  `crates/logicaffeine_compile/examples/decode_gpu_programs.rs`.
- [ ] `fabric_model_lock::the_soft_core_and_the_fabric_generate_the_vms_tokens` (iverilog for the tiny cut; the board
  run under `LOGOS_FPGA_BOARD=1`).
- [ ] `fabric_model_lock::every_cycle_count_is_read_from_its_counter`.
- [ ] `one_program_lock::no_backend_runs_a_program_from_a_second_writer` (ONE-PROGRAM-RETIRE-SECOND-WRITERS's census,
  for the board images).

**Success.**
- [ ] Tokens a second, cycles a token and LUTs for both designs on one board, each from its counter and its recipe
  (receipt: the ledger row and the host's receipt).
- [ ] The integer point's agreement with the f32 reference recorded beside them.

**Done when.** The board demo runs a model the one program wrote, on both the soft core and the fabric, side by side.

#### AUTHOR-INTEGER-POINT · `model::author` writes an integer knob point

| Field | Value |
|---|---|
| Phase | demo |
| Labels | `infrastructure` `area:quant` |
| Depends on | none |
| Touches | `crates/logicaffeine_compile/src/model/author.rs`; the integer sugar the front end already lifts (`IntRequant`, `IntClamp`, `LutAttention`) |
| Owner notes | `author.rs` has one writer at a time (S12); the default program text and every pinned digest stay unchanged |
| Status | open |

**Goal.** A knob point at which `model::author` writes the model as integer arithmetic: int8 weights and activations,
int32 accumulation, requantization by a fixed multiplier and shift, and table nonlinearities. It is exact on every
integer chip (the C and RISC-V per-op probes already match the VM on these op kinds), and it is the program the fabric
runs. Its quality against the f32 reference is measured, never assumed.

**RED tests.**
- [ ] `model_conformance_lock::the_integer_point_runs_on_the_vm_and_matches_its_integer_reference`.
- [ ] `model_conformance_lock::the_default_point_is_unchanged` (the identity of every existing cut, recomputed).
- [ ] `model_golden_lock::the_integer_point_records_its_agreement_with_the_f32_reference` (on demand).

**Done when.** The one program has an integer form every integer chip runs exactly.

#### FABRIC-TERMINAL-LAND · The Fabric Terminal lives in the tree

| Field | Value |
|---|---|
| Phase | demo |
| Labels | `infrastructure` `fabric` |
| Depends on | none |
| Touches | the terminal's host script under `scripts/`, its emitters and their locks, from a copy outside the tree |
| Status | open |

**Goal.** The Fabric Terminal (type a LOGOS program on the board's screen; it compiles, streams to the fabric engine,
and prints SILICON == VM) ran 10 of 10 library programs on Board B on 2026-10-25, from a copy of 2026-10-04 that never
landed. It lands in the tree with its locks, resynced by git's file list, and the demo runbook names its image and
recipe.

**RED tests.**
- [ ] `fabric_terminal_lock::every_library_program_runs_equal_to_the_vm` (iverilog), and the board run under
  `LOGOS_FPGA_BOARD=1`.

**Done when.** The live demo of Demo 2 runs from the tree, not from a copy.

#### DEMO-QUALITY-SIDES · Every scoreboard side records how closely it follows the f32 model

| Field | Value |
|---|---|
| Phase | demo |
| Labels | `infrastructure` `cuda` |
| Depends on | SETUP-5 |
| Touches | `scripts/accel/model_scoreboard.py`, `benchmarks/results/models/rtx3060_scoreboard.json`, `model_scoreboard_lock` |
| Owner notes | the scoreboard belongs to the performance lane; this changes what an existing lock asserts, so Tristen approves the schema first |
| Status | open |

**Goal.** Section 2.3 requires matched quality for every win, and no rival's quality is in any ledger. Every side
(ours and every rival) records its agreement with the f32 reference over `model_golden_lock`'s golden prompts: top-1
agreement over 512 greedy steps, and the top-logit gap. The rivals' logits come from llama.cpp's logit dump and
vLLM's logprobs. The 27B's figure through the generic lane (497 of 512 steps, against 473 for llama.cpp's own build)
becomes a ledger row, not a log line.

**RED tests.** Staged behind `handed-reds` (section 1.1); RED receipt: `logs/demo-strategy/handed-reds-red.log`. Each side's `agreement` is `{golden, steps, top1, top_logit_gap}`.
- [ ] `demo_quality_sides_lock::every_side_records_its_agreement_with_the_f32_reference`.
- [ ] `demo_quality_sides_lock::a_quoted_win_agrees_at_least_as_well_as_the_rival_within_two_standard_errors`.

**Done when.** No speed win is quoted without the quality it was won at.

#### DEMO-CELLS · The sheet prints only won cells

| Field | Value |
|---|---|
| Phase | demo |
| Labels | `infrastructure` |
| Depends on | LEDGER-3, DEMO-QUALITY-SIDES |
| Touches | the private sheet generator; `sota_cells.json` |
| Owner notes | the sheet belongs to the performance lane |
| Status | open |

**Goal.** The one-page sheet and the slides read every number from a won `sota_cells.json` row (section 2.3: one boot,
matched bits, matched quality, five runs a side, past two standard errors, our slowest run past the rival's fastest).
Batch cells included. A lost cell is never printed; the sheet's appendix names its closers instead.

**RED tests.**
- [ ] The sheet's own check refuses a cell with no won row, and a figure typed into a template.

**Done when.** A slide cannot show a cell the ledger has not won.

#### DEMO-FUSION-MAP · Fused symbols × chips, read from what each chip's lowering fired

| Field | Value |
|---|---|
| Phase | demo |
| Labels | `infrastructure` |
| Depends on | none |
| Touches | a new generator (an example in `crates/logicaffeine_compile/examples/`) and `benchmarks/results/demo/fusion_map.json`; a new `fusion_map_lock` |
| Owner notes | reads the lane, the engine and the spatial pipeline through their public APIs only; an accessor any of them needs is a handoff to its owner |
| Status | claimed |

**Goal.** Section 3.4 maps every fused symbol onto every chip family as a target. Demo 1's middle panel needs the same
map as fact, today, before the shims of OPT-2 exist. For each scoreboard model and each chip that takes author cuts,
the generator records which ledger classes that chip's lowering fired and how often:
- the CUDA lane: `Schedule::fusions` of every form, mapped from rule names to class ids by section 5.9;
- the D5005 engine: the merges `lower_graph` makes (q/k/v into one matvec, convs into `Conv`, the gates into `Gates`,
  the sigmoid gate, the cache append, the arg-max into `Advance`);
- the spatial path: `fuse_generic`, `pack_phases` and `share_q8_views`.

Each cell cites the census test that holds it. When OPT-2 lands, ONE-PROGRAM-FUSION-MAP-LOCK reads the shims' answers
into the same file under the same lock.

**RED tests.** The generator is `crates/logicaffeine_tests/tests/common/fusion_map.rs`; each rule's census is found,
not typed: the `lane_compile_lock` tests whose bodies name it.
- [x] `fusion_map_lock::the_map_regenerates_byte_identical`: every tiny architecture, pinned in
  `tests/fixtures/fusion_map_tiny.json` (receipt: `logs/demo-strategy/green-fusion-map.log`).
- [x] `fusion_map_lock::every_lane_rule_has_a_meaning`: every rule in `plan::RULES`, and every rule a schedule records,
  maps to a symbol of section 3.4 or a schedule property.
- [x] `fusion_map_lock::every_symbol_the_map_names_is_one_the_book_maps` (it added `embed_scale`, `conv_parts` and
  `gdn_gate_projections` to section 3.4: the lane fired them and the map did not name them).
- [x] `fusion_map_lock::every_rule_the_lane_fires_has_a_census_lock`.
- [x] `fusion_map_lock::the_d5005_cells_are_the_engine_programs_of_the_one_program`.

All failed before the generator and the fixture existed (receipt: `logs/demo-strategy/red-fusion-map2.log`).

**Success.**
- [x] The real checkpoints' map, `benchmarks/results/demo/fusion_map.json`: seven models, the CUDA lane and the D5005
  engine, every refusal by name (receipt: `fusion_map_lock::the_real_checkpoints_map_regenerates`, on demand,
  `logs/demo-strategy/record-real-fusion-map.log`).
- [ ] The spatial column: the SN50 path's passes (`fuse_generic`, `pack_phases`, `share_q8_views`) record no per-section
  report of what fused (OPT-3, MIGRATE-3).
- [ ] Primitives a chip runs whole (`flash_decode`, `router_topk`, `gdn_step`, the expert kernels), read from the launch
  kinds and the engine's commands.
- [x] The drift the map found, handed to S5: the lane fires `norm_then_add` on Gemma-3 (`sandwich_norm`, 52 times on
  the real checkpoint), and `benchmarks/fusion_parity.json` listed the class as a gap (receipt: the ledger now says
  compiled, `COMPILED_FLOOR` 29, by audit-gaps-demo-prep on 2026-10-26).

**Mutants to kill.**
- [ ] A cell marked fired whose census does not name it.
- [ ] A lane rule removed from `LANE_RULES`.

**Done when.** Demo 1's fusion panel is generated from the lowerings and cannot drift from them.

#### DEMO-LAYER-TOUR · Each English statement, and what it became on each chip

| Field | Value |
|---|---|
| Phase | demo |
| Labels | `infrastructure` |
| Depends on | none |
| Touches | a new generator beside DEMO-FUSION-MAP's; a new `layer_tour_lock` |
| Owner notes | the D5005 engine's commands do not record the graph ops they cover; an exact tour there needs `lower_graph` to record them (a handoff to its owner, part of MIGRATE-4) |
| Status | claimed |

**Goal.** For each statement of one Gated-DeltaNet and one gated-attention layer,
the tour prints the launch it lowered into on the CUDA lane (through `Form::graph` and `Schedule::ops`), the command on
the D5005 engine, and the section on the spatial path, and what it fused with. That is the slide's line from English to
silicon.

**RED tests.** The statements are the layer's own cut of the program (`Cut::DecodeLayer`), so the tour needs no
private file; the generator is `crates/logicaffeine_tests/tests/common/layer_tour.rs`.
- [x] `layer_tour_lock::every_statement_of_the_layer_lands_in_exactly_one_launch` (the CUDA lane; receipt:
  `logs/demo-strategy/green-layer-tour.log`).
- [x] `layer_tour_lock::the_tour_regenerates_byte_identical` (`tests/fixtures/layer_tour_qwen35.md`).
- [x] `layer_tour_lock::the_real_qwen38_tour_regenerates` (on demand): `benchmarks/results/demo/layer_tour_qwen38.md`.
  Qwen3.8-27B's gated-delta layer is 26 statements in 10 launches; its gated-attention layer 22 statements.
- [ ] The D5005 column, once `lower_graph` records the graph ops each command covers.
- [ ] The spatial column, once the spatial pipeline reports its sections' ops (MIGRATE-3).

**Done when.** Every line of the slide's English has its destination on every chip, generated.

#### DEMO-BRINGUP-RUN · `largo bringup --run`: every chip runs the cut and reports tokens, speed and agreement

| Field | Value |
|---|---|
| Phase | demo |
| Labels | `infrastructure` |
| Depends on | ONE-PROGRAM-VM-REFERENCE |
| Touches | `crates/logicaffeine_compile/src/bringup.rs`, `apps/logicaffeine_cli/src/commands/bringup.rs`, `docs/cli.md`, `bringup_lock` |
| Owner notes | bring-up belongs to the one-program stream |
| Status | open |

**Goal.** `bring_up` lowers today and runs nothing. `Backend::run` runs a lowered cut on a fixed prompt and reports its
identity, its tokens, its tokens a second, and its agreement with the f32 golden. New backends: the VM and the JIT
(correctness), the CPU through the AOT tiers (generic first, PARITY-CPU-SHIM's links later), RISC-V under qemu, WASM,
and the fabric through FABRIC-AUTHOR-MODEL. A chip not on the box compiles and refuses to run by name.

**RED tests.** Staged behind `handed-reds` (section 1.1); RED receipt: `logs/demo-strategy/handed-reds-red.log`.
- [ ] `demo_bringup_run_lock::every_backend_in_the_book_is_a_bringup_backend` (ONE-PROGRAM-EVERY-BACKEND): vm, jit, c,
  rust, riscv-qemu, wasm, cuda, d5005, sn50, fabric.
- [ ] `demo_bringup_run_lock::a_run_reports_tokens_speed_and_agreement_per_chip`: `largo bringup <tiny> --target vm
  --run --tokens 2` prints a `ran:` line with its tokens a second and its agreement.
- [ ] `cli_docs_lock` for the `--run` flag.

**Done when.** One command brings a checkpoint up on every chip on the box and says how each did.

#### D5005-CARD-RECEIPTS · The card's runs are receipts in the tree

| Field | Value |
|---|---|
| Phase | demo |
| Labels | `infrastructure` `d5005` |
| Depends on | none |
| Touches | `benchmarks/silicon/d5005/card_runs.json` and a ledger of published comparisons beside it; `d5005_card_runs_lock` |
| Owner notes | the card and its run logs belong to the engine session (qwen-fpga-decode-engine), which handed the ledger to S15 on 2026-10-26 and checks its rows |
| Status | claimed |

**Goal.** Qwen3.8-27B decoded token-exact on the card on 2026-10-26 (hybrid7c at 100 MHz, about 1.01 s a token; four
sequences at once, 3.2 tokens a second), and the record lives only in a session's memory and on the card's host. Each
run becomes a row: image, Fmax, clock, the program's identity line, tokens, core cycles, seconds a token, sequences and
faults, with its log. Beside it, the agreement of the card's tokens with the 27B f32 golden, and a ledger of published
single-card FPGA LLM decode results with citations. "The first 27B-class model on one FPGA card" is printed only if
that ledger holds no counterexample.

**RED tests.** The runs' logs, the image's fit summary and Fmax table, and both programs' identity lines are committed in
`benchmarks/silicon/d5005/card_runs/2026-10-26/`.
- [x] `d5005_card_runs_lock::every_figure_is_derived_from_its_log`: tokens, cycles and faults are the log's; a step's
  seconds are its last cycles × 8 / 100 MHz; tokens a second are its sequences over that.
- [x] `d5005_card_runs_lock::every_run_is_token_exact_to_its_golden`.
- [x] `d5005_card_runs_lock::every_run_ran_without_faults`.
- [x] `d5005_card_runs_lock::every_run_names_its_image_and_its_program` (the fit succeeded; the user clock's Fmax,
  103.14 MHz, covers the runs' 100 MHz).
- [x] `d5005_card_runs_lock::the_card_ran_the_program_the_author_writes` (on demand): each run's identity, recomputed from
  the checkpoint's own spec through `model::program::written`, is the one the card ran.
- [ ] `d5005_card_runs_lock::every_comparison_cites_its_source`, over the published comparisons' ledger.

**Success.**
- [x] Qwen3.8-27B: one sequence at 1.012 s a step (0.988 tokens a second); four at 1.273 s a step (3.14 tokens a
  second), every sequence token-exact; Qwen3.5-0.8B at 0.160 s a step (receipt: `card_runs.json`,
  `logs/demo-strategy/d5005-card-runs.log`).
- [ ] The published comparisons, and the "first on one card" check against them.

**Done when.** The card's demo figures are read from rows, not memories.

#### PROOF-RECEIPTS-FRESH · Demo 3's figures are current and read from their ledgers

| Field | Value |
|---|---|
| Phase | demo |
| Labels | `infrastructure` |
| Depends on | none |
| Touches | `benchmarks/results/solvers.json` (re-run on a quiet box), `benchmarks/results/lean-kernel-arena/scoreboard.json` (checked against the Arena's live revision), the sheet's proof lines |
| Status | open |

**Goal.** The certified-SAT figures date from 2026-10-08. Re-run `benchmarks/run-solver-vs-z3.sh` on a quiet box,
recording the boot, the load average and the commit of the build measured; confirm the Arena scoreboard against the
leaderboard's current revision (it names its revision and its checker's digest already); and have the sheet read both
from their ledgers.

**RED tests.** Staged behind `handed-reds` (section 1.1); RED receipt: `logs/demo-strategy/handed-reds-red.log`.
- [ ] `proof_receipts_lock::the_solver_ledger_records_the_box_and_the_build_it_ran_on`.

**Done when.** Every proof figure on a slide is from a ledger dated after the rival versions it names.

#### PROOF-LIVE · The proof demo as one rehearsed runbook

| Field | Value |
|---|---|
| Phase | demo |
| Labels | `infrastructure` |
| Depends on | PROOF-RECEIPTS-FRESH |
| Touches | the private demo workspace (a runbook beside the live Arena script) |
| Status | open |

**Goal.** One runbook runs the Arena cases that the official kernel, mathgraph and sokonanoda time out on (`arena_live.py`)
and re-checks the PHP(40) certificate with external drat-trim, live, with its expected output and recovery steps.

**Done when.** Demo 3 runs live from a runbook that has been rehearsed.

#### MEASURED-FIELDS · No board streams a constant where a measurement belongs

| Field | Value |
|---|---|
| Phase | demo |
| Labels | `infrastructure` |
| Depends on | none |
| Touches | a new `measured_fields_lock` |
| Status | done |

**Goal.** A census over every board top in `benchmarks/silicon/` refuses a streamed byte that is a constant (an
expression of localparams only, or a literal other than the sync byte and the reset value). Today's six offenders sit on
a shrink-only allowlist, each naming FABRIC-SETTLE-PROBE.

**Success.**
- [x] `measured_fields_lock` green, its synthetic mutants refused (receipt: `the_census_refuses_every_form_of_a_streamed_constant`).

#### FROZEN-REDS · A handed-over RED test stays the test it was

| Field | Value |
|---|---|
| Phase | demo |
| Labels | `infrastructure` |
| Depends on | none |
| Touches | a new `frozen_reds_lock` and `crates/logicaffeine_tests/tests/fixtures/frozen_reds.tsv` |
| Status | done |

**Goal.** Every RED test handed to an agent is pinned by the SHA-256 of its normalized body (layout-insensitive,
token-sensitive) with its ticket. An agent that edits a frozen test turns the lock red; re-pinning changes a ticket's
specification and needs Tristen's approval (CLAUDE.md rule 4, made mechanical). The keeper adds a row for every RED
test when its ticket is posted.

**Success.**
- [x] `frozen_reds_lock` green, its digest refusing every token edit and ignoring reflow (receipt:
  `the_digest_ignores_layout_and_sees_every_token`).

#### NUMERICS-EXACT · A knob point at which every chip computes the same bits

| Field | Value |
|---|---|
| Phase | demo |
| Labels | `infrastructure` `area:quant` |
| Depends on | OPT-6, IR-2 |
| Touches | `tensor_graph::numerics`; `model/author.rs` (a header and explicit quantize statements); the CUDA lane (`Q8MatKind::Exact`); the D5005 engine (`EngineNumerics::exact()`, a handoff) |
| Status | open (deferred: not on the demo path; Tristen, 2026-10-26, "we want speed") |

**Goal.** One declared point at which every chip claims zero error, so a model's logits are bit-identical on every chip,
tiling, batch and fusion. The design, recorded here so it is not re-derived:
- every reduction is computed exactly and rounded once to f32 (Q8_0 block terms are exact integers times exact scales;
  a declared window floors terms below an anchor, which makes int128 a provable width);
- every elementwise function takes one f32 input and is correctly rounded, so an exhaustive 2^32 sweep proves a native
  version equal to its LOGOS definition;
- the activation quantizer is the D5005's exact integer one (`decode_engine/numerics.rs`, `quantize_with`);
- IEEE subnormals are kept;
- the program says so in its text (`The arithmetic is single.`), so the VM, the JIT and the AOT tiers inherit it.

Decode on the 3060 stays memory-bound at this point (about 40 of the roughly 316 integer operations per Q8_0 block the
bandwidth allows); prefill falls below tensor-core peak until TECH-SPLIT-PRECISION-MMA.

**Done when.** `one_program_lock::every_backend_produces_the_vms_bits_on_every_tiny_cut` is green at this point.

---

## 13. Appendices

### 13.1 Locks every ticket keeps green

No ticket may leave these red. Run the ones a change touches, each with `--test`.

- [ ] `lane_compile_lock` stays green: the lane's censuses: which kernels each architecture and form launches; every rule can be held back; refusals by name.
- [ ] `cuda_lane_lock` stays green: the lane's values on the GPU: every fusion computes what its launches compute apart; every model meets its precision bar.
- [ ] `cuda_fusion_census_lock` stays green: the older forward's fusions.
- [ ] `cuda_elementwise_fusion_lock` stays green: elementwise chains.
- [ ] `fusion_parity_lock` stays green: the ledger.
- [ ] `one_program_lock` stays green: every artifact is a cut of the one program.
- [ ] `model_scoreboard_lock` stays green: the scoreboard's runs and ratios.
- [ ] `model_golden_lock`, `model_reference_lock` stays green: golden agreement and the reference.
- [ ] `d5005_lower_graph_lock`, `d5005_capability_lock` stays green: the engine's programs and refusals.
- [ ] `spatial_backend_totality_lock` stays green: every spatial backend lowers or refuses by name.
- [ ] `recognizer_migration_lock`, `model_author_migration_lock` stays green: pinned migrations.
- [ ] `bringup_lock` stays green: `largo bringup` over every chip.
- [ ] `cuda_knob_consumers_lock` stays green: which knobs an emitter reads.
- [ ] `sota_contract_lock`, `kernel_ab_lock`, `rival_baseline_lock`, `serving_bench_lock` stay green once they exist: the SOTA contract, its kernel rows, its cell rows and its serving trace.
- [ ] `repo_shape_lock`, `public_prose_lock`, `workspace_reads_lock` stays green: the checkout's shape and its public prose, which this book is held to.
- [ ] `frozen_reds_lock` stays green: every handed-over RED test is the test that was handed over.
- [ ] `measured_fields_lock` and `fabric_perf_lock` stay green: no board streams a constant as a measurement, and the fabric's figures are the board's.

### 13.2 Glossary

| Term | Meaning |
|---|---|
| Cut | One form of a model's program: the decode step, a prefill chunk, a batch, a per-layer kind (`model::program`). |
| Fused symbol | A node of the IR whose definition is its parts; built by a rule. |
| Primitive symbol | An op the front end lifts whole, which a chip links whole or runs as parts. |
| Edge layout | A strided read or a row or unit destination: a placement fusion carried on an edge. |
| Shim | A chip's declaration of what it allows, its numerics, limits and cost. |
| Link | A chip's resolution of a symbol to its own primitive. |
| Own path | A chip's schedules and chip-only rules. |
| Numerics | The lane's activation precisions: Dequant (f32 activations, f64 sums), one int8 word per 32-block, two int8 words per 32-block. |
| `compile_verified` | Built and checked here by compile censuses and CPU replicas; GPU values pending a datacenter GPU. |
| Copy / Beyond | A PARITY ticket's two bars: match the best implementation at our shapes, then pass it. |
| Row maximum | The fastest kernel any rival in section 2.2 ships for an op at our shapes; every Copy bar (section 2.5). |
| Cell | One model on one chip at one bit width in one regime (section 2.4); won only under section 2.3. |
| Rival | A stack in section 2.2's table. |
| Coverage row | One vLLM or Triton entry at the pins, mapped to the ticket that copies it (section 11). |
| Cut | One form of a model's one program (`model::program::Cut`): a decode step, a prefill chunk, a batch, a forward, or one layer, embedding or head of them (section 3.1). |
| Identity | A cut, its chip count and the SHA-256 of its text; every artifact records it (section 3.1). |
| Stream | A set of tickets whose files no other stream touches, so agents in different streams work at once (section 1.5). |

### 13.3 Checking this book's citations

Every `external/competitors/<stack>/<path>` · `<anchor>` citation must resolve to exactly one match in its file at the
pin. Until LEDGER-2 turns this into a lock, run:

```
python3 - <<'PY'
import re, os
book = open('benchmarks/FUSIONS_WE_NEED.md').read()
bad = 0
for m in re.finditer(r'`external/competitors/([a-z.\-]+)/([^`]+)`\s*·\s*`([^`]+)`', book):
    stack, path, anchor = m.groups()
    p = os.path.join('external/competitors', stack, path)
    text = open(p, errors='ignore').read() if os.path.isfile(p) else ''
    if text.count(anchor) != 1:
        bad += 1
        print(text.count(anchor), stack, path, anchor)
print('unresolved or ambiguous:', bad)
PY
```

When this book was written, the pinned trees in `external/competitors/` still held vLLM v0.31.0 and no Triton. Every
vLLM and Triton citation was checked against the working clones `external/vllm` (ceb01b6ddf) and `external/triton`
(1ed999d6b2). All 162 distinct citations resolved to exactly one match. SETUP-2 places those trees at the pins,
so this check runs against them.

### 13.4 Re-extracting the coverage map

Section 11 is the output of the rules below, run over the pinned trees. Until LEDGER-3 ports them into
`sota_contract_lock`, run this script from the repository root:
- it prints every entry at the pins that has no row, and every row whose entry is gone;
- run as `python3 - --by-ticket`, it also prints each ticket's rows, ticked or not.

Before SETUP-2 places the pins, point it at the working clones with `VLLM_TREE=external/vllm TRITON_TREE=external/triton`.

```
python3 - <<'PY'
import os, re, sys, collections

V = os.environ.get('VLLM_TREE', 'external/competitors/vllm')
T = os.environ.get('TRITON_TREE', 'external/competitors/triton')
CV, CT = 'external/competitors/vllm/', 'external/competitors/triton/'


def files(root, sub, pat=r'\.py$'):
    out = []
    for d, _, fs in os.walk(os.path.join(root, sub)):
        out += [os.path.relpath(os.path.join(d, f), root) for f in fs if re.search(pat, f) and f != '__init__.py']
    return sorted(out)


def read(root, p):
    return open(os.path.join(root, p), errors='ignore').read()


def entries():
    keys = set()
    regs = collections.defaultdict(set)
    for p in files(V, 'csrc', r'\.(cpp|cu|cc|h|cuh)$'):
        for m in re.finditer(r'\.def\(\s*"(\w+)', read(V, p)):
            regs[m.group(1)].add(p)
    for name, ps in regs.items():
        home = sorted(p for p in ps if not p.startswith(('csrc/cpu/', 'csrc/rocm/'))) or \
            sorted(p for p in ps if p.startswith('csrc/rocm/')) or sorted(ps)
        keys.add((CV + home[0], name))
    for p in files(V, 'vllm/compilation/passes/fusion'):
        keys |= {(CV + os.path.dirname(p) + '/', f'{c} ({os.path.basename(p)})') for c in re.findall(r'^class (\w+)', read(V, p), re.M)}
    for sub in ('vllm/compilation/passes/ir', 'vllm/compilation/passes/utility'):
        for p in files(V, sub):
            keys |= {(CV + os.path.dirname(p) + '/', f'{c} ({os.path.basename(p)})') for c in re.findall(r'^class (\w+Pass)\b', read(V, p), re.M)}
    for p in files(V, 'vllm/ir/ops'):
        keys |= {(CV + os.path.dirname(p) + '/', f'{c} ({os.path.basename(p)})') for c in re.findall(r'@register_op[^\n]*\ndef (\w+)', read(V, p))}
    for p in files(V, 'vllm/model_executor/kernels/linear'):
        keys |= {(CV + os.path.dirname(p) + '/', f'{c} ({os.path.basename(p)})') for c in re.findall(r'^class (\w+Kernel)\b', read(V, p), re.M)}
    for sub in ('vllm/v1/attention/backends', 'vllm/v1/attention/ops', 'vllm/model_executor/layers/fused_moe/experts',
                'vllm/model_executor/layers/fused_moe/prepare_finalize', 'vllm/model_executor/layers/fused_moe/router',
                'vllm/v1/spec_decode', 'vllm/v1/sample/ops', 'vllm/v1/worker/gpu/sample'):
        keys |= {(CV + os.path.dirname(p) + '/', os.path.basename(p)) for p in files(V, sub)}
    keys |= {('Triton kernels in', CV + os.path.dirname(p) + '/', os.path.basename(p)) for p in files(V, 'vllm')
             if re.search(r'^\s*@triton\.jit', read(V, p), re.M)}
    for p in ('third_party/nvidia/backend/compiler.py', 'third_party/amd/backend/compiler.py'):
        keys |= {(CT + p, f'{a}.{b}') for a, b in re.findall(r'\bpasses\.(\w+)\.add_(\w+)\(', read(T, p))}
    L = 'python/triton/language/'
    for p, decos in ((L + 'core.py', ['@builtin']), (L + 'math.py', ['@core.builtin']),
                     (L + 'standard.py', ['@jit', '@core._tensor_member_fn']), (L + 'random.py', ['@jit'])):
        lines = read(T, p).split('\n')
        for i, l in enumerate(lines):
            m = re.match(r'^def (\w+)\(', l)
            if not m or m.group(1).startswith('_'):
                continue
            j, ok = i - 1, False
            while j >= 0 and lines[j].strip() and not re.match(r'^(def|class) ', lines[j]):
                ok |= any(lines[j].startswith(d) for d in decos)
                j -= 1
            if ok:
                keys.add((CT + p, 'tl.' + m.group(1)))
    for sub in ('python/tutorials', 'python/examples/gluon', 'python/triton_kernels/triton_kernels'):
        keys |= {(CT + os.path.dirname(p) + '/', os.path.basename(p)) for p in files(T, sub)}
    return keys


def rows(book):
    text = book.split('## 11. ', 1)[1].split('\n## 12. ', 1)[0]
    out, group = {}, None
    for line in text.split('\n'):
        m = re.match(r'^(Registered in|Triton kernels in|In) `([^`]+)`:$', line)
        if m:
            group = ('Triton kernels in' if m.group(1) == 'Triton kernels in' else None, m.group(2))
            continue
        m = re.match(r'^- \[([ x])\] `([^`]+)`( \([^)]*\))? → (.+)$', line)
        if m and group:
            token = m.group(2) + (m.group(3) or '')
            key = (group[0], group[1], token) if group[0] else (group[1], token)
            out[key] = (m.group(1) == 'x', [t.strip() for t in m.group(4).split(',')])
    return out


if __name__ == '__main__':
    book = open('benchmarks/FUSIONS_WE_NEED.md').read()
    have, want = rows(book), entries()
    missing, stale = sorted(want - set(have)), sorted(set(have) - want)
    for k in missing:
        print('no row for', k)
    for k in stale:
        print('row with no entry', k)
    if '--by-ticket' in sys.argv:
        by = collections.defaultdict(list)
        for k, (ticked, tks) in sorted(have.items()):
            for t in tks:
                by[t].append(('x' if ticked else ' ') + ' ' + ' '.join(x for x in k if x))
        for t in sorted(by):
            print(f'{t} ({len(by[t])})')
            print('\n'.join('  ' + r for r in by[t]))
    print(f'{len(have)} rows, {len(want)} entries, {len(missing)} missing, {len(stale)} stale')
PY
```

On 2026-10-25, against the working clones, it reported 1,294 rows, 1,294 entries, 0 missing and 0 stale.
