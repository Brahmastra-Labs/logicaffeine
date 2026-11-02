# Prepare for SambaNova: model bring-up on RDU-class spatial chips

This book is the work order for one capability: take any open model as its maintainers publish it, bring it up on an
RDU-class spatial chip (SambaNova's SN40L and SN50, and the Plasticine architecture they descend from), correct from
its first token, and run it at the highest tokens per second any compiler reaches on that chip.

It is written for a model bring-up evaluation whose tests we have not seen. Section 10 sets every test a team that
builds these chips could set; every answer is a ticket on the board (section 11) that builds the capability and a lock
that holds it before the test arrives.

- **Section 2, the exam and the score**: how SambaNova measures a compiler in its own papers, the numbers it publishes
  (our Copy bars), the cells we are scored in, and the rules of a fair win.
- **Section 3, access**: every level of access we may be given, from the papers alone to the hardware and its lowest
  SDK, and the adapter that makes each level's first day mechanical.
- **Section 4, the chips**: Plasticine's principles as our lowering must speak them, the SN40L and SN50 facts with
  their sources, and what our chip model gets wrong today.
- **Section 5, what they do**: every technique SambaNova and the Stanford lineage publish, mapped to our status and to
  the ticket that copies it and goes past it.
- **Section 6, model coverage**: the families a test can send, and every feature they need, each a ticket.
- **Section 7, Qwen3.8-Flash-Next and Qwen 4**: the published preview of the next Qwen architecture, component by
  component, with its spatial schedule, its budgets and a timed day-zero drill.
- **Section 8, the compiler we must have**: the board's families walked against today's measurements.
- **Section 9, the hill-climb**: the loop that turns the exam, the ledgers and the compiler's own search into gains
  that never regress.
- **Section 10, the exam catalog**: every anticipated test with its edge cases, its lock and the ticket that answers it.
- **Section 11, the board**: every ticket with RED tests, edge cases, success boxes and verify commands, written so an
  agent in a fresh session can take one and finish it.

Every item in sections 2 to 11 is a checkbox. A box is ticked only with its receipt beside it. `samba_book_lock` holds
the book's derived parts (section 12.4).

**How this book relates to the fusion book.** `benchmarks/FUSIONS_WE_NEED.md` is the work order for the one IR, the
shared fusions and the CUDA lane. Its SOTA contract leaves the spatial chips out on purpose: their SOTA "is judged
against published accelerator results in their own campaign ledgers" (its section 2.7). This book is that campaign
ledger for the RDU class. The split:
- a primitive, a fusion rule or a proof that every chip shares is a fusion-book ticket, named here as
  FUSIONS:PARITY-SPARSE-ATTENTION is named, and never duplicated;
- this book owns the RDU lowering, the schedule, the chip model and the time model, the model intake a bring-up test
  needs, the exam and the scorecard.

**Counts.** 187 tickets on the board, 103 exam tests, 1,482 checkboxes, 219 ticked.

Read at:

| Source | Pin | Where it lives |
|---|---|---|
| vLLM (`qwen4_exp`, the other model trees) | ceb01b6ddf, the fusion book's pin | `external/competitors/vllm` |
| llama.cpp (`qwen4exp`, `deepseek32`, `qwen3next`) | b11635, the fusion book's pin | `external/competitors/llama.cpp` |
| Qwen3.8-Flash-Next configuration | `Qwen/Qwen3.8-Flash-Next`, main, read 2026-10-29 | [HFQ4] |
| transformers `modeling_qwen4_exp.py` | main, read 2026-10-29 | [TFQ4] |
| Spatial, PIR, plastiroute, plastisim, Tungsten | the 2023-01-31 dataplane-ai snapshot, placed by `scripts/plasticine/fetch.sh` | built under `target/plasticine-toolchain` |
| Papers and vendor pages | section 12.3 | PDFs pinned by STUDY-1 |

---

## 1. How to use this book

### 1.1 Taking a task

The protocol is the fusion book's section 1.1, with this book's names:
1. Pick a ticket whose **Depends on** list is done. Section 11.2 gives the order. A `FUSIONS:` dependency is done when
   the fusion book's board marks it done.
2. Post a claim on the lanes board: `CLAIM SAMBA <TICKET> <agent> <stream> files: <the Touches row>`.
3. Read everything under **Read first**.
4. Write the **RED tests** exactly as named. Run each with `--test` and confirm it fails for the stated reason. A RED
   test that passes on today's tree is not a RED test: stop and find out why.
   - An EXAM ticket writes its suite as handed REDs: a binary that opens with `#![cfg(feature = "handed-reds")]`, frozen
     by `frozen_reds_lock`. The ticket a test names makes it pass and deletes the gate line as it lands.
5. Implement until the RED tests pass.
6. Run **Verify**. Read every line of the output; never pipe a test run through `head` or `tail`.
7. Post one `RECEIPT SAMBA` line per Success box, per edge case covered and per exam test turned green.
8. Kill every listed mutant in a private copy of the tree and post the `MUTANTS SAMBA` line.
9. Land under the fusion book's section 1.3 (gate windows, targeted suites, `--no-run` when a public enum grew, one
   fast-suite run, CHANGELOG `[Unreleased]`), then post `DONE SAMBA`.

A ticket is done only when every Success box carries a receipt.

### 1.2 Standing rules

The fusion book's standing rules hold here unchanged: never edit a failing test, never delete a test (port it),
mutation-test every lock in a private copy, tests live in `crates/logicaffeine_tests`, name the suite with `--test`,
changing what a lock asserts needs Tristen's approval, one writer of models (`model::author`, held by
`one_program_lock`), peer-owned files change by handoff, generated Rust states every binder's type. Five more:

- **No speed number without its provenance.** Every number this book or a ledger carries is one of:
  - *published*: a citation from section 12.3;
  - *simulated*: the simulator, the app's hash and the log (Tungsten, ttsim, Spatial's simulator);
  - *modeled*: the time model's version and the calibration set it was fitted on;
  - *measured*: the hardware, the boot, the load and the log.
  A modeled number guides the work; it never wins a cell (section 2.4).
- **Two oracles.** The VM is the oracle of a program's meaning; the f64 reference (`crates/logicaffeine_compile/src/model/reference.rs`)
  and the pinned reference implementations are the oracles of a model. Every chip result is held to both.
- **Certify or refuse by name.** Every lowering carries a certificate that rechecks, or refuses with the statement,
  the op and the reason. A statement the front end does not lift is a refusal, never a silent drop.
- **Shared sides live in the fusion book.** A primitive or fusion rule every chip could share is written in the one IR
  through its fusion-book ticket; a ticket here writes only the spatial side.
- **Plasticine first.** A capability is not done on the RDU until it also compiles through the Plasticine chain
  (Spatial, PIR, plastiroute, Tungsten) where the chain can express it, or the ticket names why it cannot.

### 1.3 Streams

The board is cut into streams whose files do not overlap, so agents in different streams never edit one file. Every
line of the board index (section 11.1) carries its stream as a label, `stream:R0` to `stream:R17`. R stands for the
RDU; the fusion book's streams are S0 to S15.

**Rules for every stream.**
- An agent takes the next open ticket of its stream whose dependencies are done and claims it on the lanes board.
- An agent never edits a file another stream owns, in this book or in the fusion book. A change it needs there is a
  `HANDOFF SAMBA` line to that stream.
- No agent edits this book but the book keeper, R0. Every other agent reports on the lanes board in the line formats of
  section 1.4, and R0 applies them.
- A file a ticket creates is owned by that ticket's stream from its first commit.

| Stream | Owns (files and modules) | First tickets | Agents at once |
|---|---|---|---|
| R0 The book keeper | `benchmarks/PREPARE_FOR_SAMBA.md`, `crates/logicaffeine_tests/tests/samba_book_lock.rs`, `benchmarks/samba_techniques.tsv`, `crates/logicaffeine_tests/tests/samba_techniques_lock.rs`; the paper pins (new) | BOOK-1, STUDY-1, STUDY-3 | 1, always |
| R1 Access and vendor bridges | `crates/logicaffeine_compile/src/spatial/lower/rdu_sdk.rs`, `crates/logicaffeine_compile/src/spatial/lower/sambaflow.rs`, `crates/logicaffeine_compile/src/spatial/lower/rdu.rs`, `crates/logicaffeine_compile/src/spatial/lower/rdu_lanes.rs`, `crates/logicaffeine_tests/tests/rdu_sdk_bridge_lock.rs`, `crates/logicaffeine_tests/tests/rdu_sambaflow_emit_lock.rs`, `scripts/accel/sambacloud-smoke.sh`; the artifact drop and the unit-configuration emitter (new) | ACCESS-2, ACCESS-3, ACCESS-4 | 2 |
| R2 Chip truth and time | `crates/logicaffeine_compile/src/spatial/rdu_chip.rs`, `crates/logicaffeine_compile/src/spatial/cost.rs`, `crates/logicaffeine_compile/src/spatial/work.rs`, `crates/logicaffeine_compile/src/spatial/roofs.rs`, `crates/logicaffeine_compile/src/spatial/clock.rs`, `crates/logicaffeine_compile/src/spatial/decode.rs`, `crates/logicaffeine_compile/src/spatial/fit.rs`, `crates/logicaffeine_tests/tests/sn50_target_lock.rs`, `crates/logicaffeine_tests/tests/sn40l_target_lock.rs`, `crates/logicaffeine_tests/tests/rdu_fit_report_lock.rs`, `crates/logicaffeine_tests/tests/rdu_unit_model_lock.rs`; the time model (new) | CHIP-1, CHIP-2, TIME-1 | 1 |
| R3 Intake and the author | `crates/logicaffeine_compile/src/model/gguf.rs`, `crates/logicaffeine_compile/src/model/quant.rs`, `scripts/accel/model-golden.sh`, `benchmarks/models/`, `crates/logicaffeine_tests/tests/common/tiny_models.rs`; the Hugging Face and safetensors readers, the golden harness and the configuration fuzzer (new) | INTAKE-1, INTAKE-4, AUTHOR-1 | 1, with the fusion book's S12 seat |
| R4 Attention and selection | the attention family's lowering file once SCHED-1 cuts it (new); `crates/logicaffeine_tests/tests/spatial_causal_attention_lock.rs`, `crates/logicaffeine_tests/tests/spatial_grouped_decode_attention_lock.rs`, `crates/logicaffeine_tests/tests/spatial_tilized_kv_cache_lock.rs` | KERNEL-1, ARCH-WINDOW, ARCH-SINKS | 1 |
| R5 Experts | the expert family's lowering file once SCHED-1 cuts it (new); `crates/logicaffeine_tests/tests/spatial_moe_real_width_lock.rs`, `crates/logicaffeine_tests/tests/moe_mechanism_lock.rs`, `crates/logicaffeine_tests/tests/tensor_moe_options_lock.rs` | KERNEL-2, ARCH-ROUTING | 1 |
| R6 Recurrent and convolutional layers | the recurrent and convolution families' lowering files once SCHED-1 cuts them (new); `crates/logicaffeine_tests/tests/tensor_gdn_row_blocks_lock.rs` | KERNEL-3, ARCH-GDN-VARIANTS, ARCH-CNN | 1 |
| R7 Schedule and the spatial IR | `crates/logicaffeine_compile/src/spatial/ir.rs`, `crates/logicaffeine_compile/src/spatial/pipeline.rs`, `crates/logicaffeine_compile/src/spatial/eval.rs`, `crates/logicaffeine_compile/src/spatial/certificate.rs`, `crates/logicaffeine_compile/src/spatial/equiv.rs`, `crates/logicaffeine_compile/src/spatial/link.rs`, `crates/logicaffeine_compile/src/spatial/shim.rs`, `crates/logicaffeine_compile/src/spatial/streamed.rs`, `crates/logicaffeine_compile/src/spatial/passes/from_qwen_ops.rs`, `crates/logicaffeine_compile/src/spatial/passes/from_tensor_op.rs`, `crates/logicaffeine_compile/src/spatial/passes/compose.rs`, `crates/logicaffeine_compile/src/spatial/passes/pack.rs`, `crates/logicaffeine_compile/src/spatial/passes/forward.rs`, `crates/logicaffeine_compile/src/spatial/passes/tile.rs`, `crates/logicaffeine_compile/src/spatial/passes/place.rs`, `crates/logicaffeine_compile/src/spatial/passes/fuse_generic.rs`, `crates/logicaffeine_compile/src/spatial/passes/fuse_readers.rs`, `crates/logicaffeine_compile/src/spatial/passes/size_buffers.rs`, `crates/logicaffeine_compile/src/spatial/passes/synthesize_handshakes.rs`, `crates/logicaffeine_compile/src/spatial/passes/persist.rs`, `crates/logicaffeine_compile/src/spatial/passes/verify.rs` | SCHED-1, SCHED-7 | 1 |
| R8 Multi-chip | `crates/logicaffeine_compile/src/spatial/tensor_parallel.rs`, `crates/logicaffeine_compile/src/spatial/passes/shard.rs`, `crates/logicaffeine_tests/tests/qwen38_tensor_parallel_lock.rs`, `crates/logicaffeine_tests/tests/spatial_tensor_parallel_lock.rs`; collectives, expert and pipeline parallelism (new) | MULTI-4, MULTI-6 | 1 |
| R9 Numerics and formats | `crates/logicaffeine_compile/src/spatial/passes/from_q8.rs`, `crates/logicaffeine_compile/src/spatial/value_range.rs`, `crates/logicaffeine_tests/tests/spatial_q8_weights_lock.rs`, `crates/logicaffeine_tests/tests/rdu_index_datatype_lock.rs`; the fp8, fp4 and int4 lowerings (new) | NUM-1, NUM-3, KERNEL-6 | 1 |
| R10 Memory tiers | `crates/logicaffeine_compile/src/spatial/residency.rs`, `crates/logicaffeine_tests/tests/spatial_residency_lock.rs`; the DDR tier and model switching (new) | MEM-1 | 1 |
| R11 Serving on the RDU | `crates/logicaffeine_compile/src/spatial/serving.rs`; speculative decoding, on-chip sampling and the runner (new) | SERVE-RDU-3 | 1 |
| R12 Mining | the harvester, the numerics search and the parallel-plan search, beside the fusion book's S8 modules (new) | MINE-1 | 2 |
| R13 Exam, score and climb | the exam suites, the exam runner and generator, the cell, climb and experiment ledgers (new) | SCORE-1, CLIMB-1, CLIMB-3, EXAM-1 | 2 |
| R14 Qwen 4 | the hyper-connection and n-gram lowering files and the qwen4_exp locks (new) | QWEN4-1 | 1 |
| R15 Plasticine | `scripts/plasticine/`, `crates/logicaffeine_compile/src/spatial/lower/spatial_lang.rs`, `crates/logicaffeine_compile/src/codegen_sva/plasticine.rs`, `crates/logicaffeine_compile/src/codegen_sva/plasticine_place.rs`, `crates/logicaffeine_compile/src/codegen_sva/plasticine_report.rs`, `crates/logicaffeine_compile/src/codegen_sva/metapipeline.rs`, `crates/logicaffeine_compile/src/codegen_sva/pmu.rs`, `crates/logicaffeine_compile/src/codegen_sva/spatial_emit.rs`, `crates/logicaffeine_tests/tests/common/plasticine.rs`, `crates/logicaffeine_tests/tests/qwen38_fabric_segments_lock.rs`, `crates/logicaffeine_tests/tests/qwen38_spatial_source_lock.rs`, `crates/logicaffeine_tests/tests/plasticine_sim_lock.rs`, `crates/logicaffeine_tests/tests/plasticine_report_lock.rs`, `benchmarks/silicon/qwen38_fabric_segments.json`, `benchmarks/silicon/plasticine_toolchain.json`, `benchmarks/silicon/plasticine_calibration.json`, `benchmarks/silicon/plasticine_table4_gate.json`, `benchmarks/silicon/plasticine_report.json` | PLAST-2, PLAST-7, PLAST-5 | 2, on disjoint apps |
| R16 Deployments | one deployment lock per model family (new) | DEPLOY-QWEN38, DEPLOY-LLAMA3 | 2, on disjoint families |
| R17 Training | the training lowering over the IR's reverse mode and the training report (new) | TRAIN-1 | 1 |

- [ ] No file is owned by two streams (receipt: `samba_book_lock::no_file_belongs_to_two_streams`).
- [ ] Every ticket carries exactly one stream, and every stream holds a ticket (receipt:
  `samba_book_lock::every_ticket_is_in_one_stream_and_every_stream_holds_work`).

**Files the fusion book's streams own.** These tickets reach them by handoff, never by edit:

| Files | Fusion-book stream | What these tickets need there |
|---|---|---|
| `crates/logicaffeine_compile/src/model/spec.rs`, `crates/logicaffeine_compile/src/model/author.rs`, `crates/logicaffeine_compile/src/model/bind.rs`, `crates/logicaffeine_compile/src/model/reference.rs` | S12 | new architecture arms, the author's lifted refusals; R3's agent works these in the S12 seat (one writer of `author.rs`) |
| `crates/logicaffeine_compile/src/tensor_graph/` | S1 | new primitives (top-k, hash, scan) and their evaluator arms, through the fusion-book ticket that owns each |
| `apps/logicaffeine_cli/src/commands/emit.rs`, `apps/logicaffeine_cli/src/commands/bringup.rs`, `crates/logicaffeine_compile/src/bringup.rs`, `crates/logicaffeine_compile/src/model/program.rs` | S2 | `--model` for every family on `emit rdu`, the bring-up clock |
| `crates/logicaffeine_compile/src/spatial/` (as a directory) | S6 | the R streams cut S6's directory file by file; S6's open tickets (FUSIONS:SPATIAL-NONUNIFORM-TILING, FUSIONS:BEYOND-SPATIAL-ON-GPU) claim through R7's seat |

**Before SCHED-1 lands**, every op family's lowering lives in one 12,661-line file,
`crates/logicaffeine_compile/src/spatial/passes/from_qwen_ops.rs`, which R7 owns. An R4, R5 or R6 ticket that must edit
it before the cut claims the file through R7's seat for the length of one edit batch. SCHED-1 is first in R7's queue for
that reason.

### 1.4 The launch kit

**The prompt.** Fill in the three names and give it to a fresh session in the repository:

```
You are agent <NAME>, in stream <STREAM> of benchmarks/PREPARE_FOR_SAMBA.md. Your ticket is <TICKET>.

1. Read section 1 of the book, then your ticket's body (search for "#### <TICKET> ·") and everything under its Read
   first. When the ticket depends on a fusion-book ticket, read that ticket's body in benchmarks/FUSIONS_WE_NEED.md.
2. Before any cargo, nvcc, Quartus or Plasticine-chain work, read the lanes board for an open gate window, and hold
   while one is open. Post: CLAIM SAMBA <TICKET> <NAME> <STREAM> files: <the ticket's Touches row>
3. Edit only the files your stream owns (section 1.3) and your ticket's Touches row. Never edit the book: report on the
   lanes board in the line formats below.
4. Write the RED tests exactly as named. Run each with --test and confirm it fails for the reason the ticket gives.
   Post one RED line per test.
5. Implement until they pass. Run the ticket's Verify, and read every line of the output.
6. Post one RECEIPT line per Success box, per edge case covered and per exam test turned green.
7. Kill every listed mutant in a private copy of the tree. Post the MUTANTS line.
8. Land inside a gate window. Post the DONE line.

Stop and ask Tristen before you change what an existing lock asserts, edit a failing test, publish a speed number
without its provenance, apply for hardware access, or touch a file another stream owns.
```

**The lines.** One per event, on the lanes board, so the book keeper applies them by script:

```
CLAIM SAMBA <TICKET> <agent> <stream> files: <paths>
RED SAMBA <TICKET> <suite>::<test> fails: <reason> log: <path>
RECEIPT SAMBA <TICKET> "<box text, as written in the book>" receipt: <test name | log path | ledger row>
MUTANTS SAMBA <TICKET> killed <k>/<n> log: <path>
HANDOFF SAMBA <TICKET> to <stream>: <file> <what is needed>
DONE SAMBA <TICKET> landed: <commit or landing ledger> suite: <fast-suite summary line>
BLOCKED SAMBA <TICKET> on <ticket or question>
EXPERIMENT SAMBA <TICKET> on: <chip or simulator> hypothesis: <text> change: <text> delta: <measured> verdict: <kept|rejected|no-effect> log: <path>
```

- [ ] The book keeper applies each RECEIPT line by finding its box text under its ticket, or in sections 2 to 10 for the
  boxes a ticket owns, and ticks it with the receipt beside it. It refuses a line whose box text does not match exactly
  once.
- [ ] After each batch it runs `samba_book_lock`, `public_prose_lock` and the citation pins (STUDY-1), then commits the
  book alone.
- [x] Every EXPERIMENT line becomes a row of the experiment ledger (CLIMB-3), kept, rejected or of no effect (receipt: `samba_climb_lock::every_experiment_line_becomes_a_row`, run with `SAMBA_LANES`).

**The first wave.** These tickets have no open dependency today, one agent each:

| Agent | Stream | Ticket | What it unblocks |
|---|---|---|---|
| keeper | R0 | BOOK-1, STUDY-1 | every receipt; every citation; CHIP-1 |
| qwen-read | R0 | STUDY-3 | QWEN4-1 and the whole Qwen 4 column |
| drop | R1 | ACCESS-4 | ACCESS-6, SERVE-RDU-5, the artifact-drop access level |
| bridge | R1 | ACCESS-3 | the low-level SDK access level |
| cut | R7 | SCHED-1 | every R4, R5 and R6 ticket |
| head | R7 | SCHED-7 | SRAM residency for state (MEM-4) |
| hf | R3 | INTAKE-1 | INTAKE-2 to INTAKE-9, QWEN4-1 |
| int64 | R9 | KERNEL-6 | KERNEL-5, QWEN4-3 |
| islands | R9 | NUM-1 | NUM-7, NUM-8, the SN50 drift bound |
| routes | R15 | PLAST-2 | PLAST-1 |
| checks | R15 | PLAST-7 | TIME-2 |
| kit | R15 | STUDY-5, PLAST-11 | PLAST-12, PLAST-13, PLAST-14, the kit's access level |
| score | R13 | SCORE-1, CLIMB-3 | every cell row, every experiment row |
| exam | R13 | EXAM-1, EXAM-6, EXAM-8 | the exam's first handed REDs |
| uneven | R8 | MULTI-4 | the 16-socket widths |

Then, as each DONE line lands, the next ready ticket in each stream starts in section 11.2's order.

- [ ] Before the first wave starts, the lanes board carries one line naming the wave, its agents and their streams.
- [ ] Each wave's start and end are on the lanes board, with the DONE lines that opened the next.

---

## 2. The exam and the score

This section states how SambaNova measures a compiler, what it has published, and how this book scores us against it.
Its numbers are the Copy bars: the published result on a chip, at a model, bits and regime. A cell's Beyond bar is the
same cell won by a margin: a higher share of the chip's memory bandwidth at the same chip count, bits, context and
batch, or the same share at fewer chips.

### 2.1 How SambaNova measures a compiler

SambaNova's own compiler papers and talks define the measures their engineers use, so they are the measures an
evaluation they design is most likely to use:

- **Time per output token and decode throughput**, measured in steady state and averaged over 10 prompts that generate
  from under 100 to over 1,000 tokens [DIAYN26]. Decode dominates: 75% to 97% of model-inference time across the six
  frontier-model configurations they analyzed [HC26].
- **Share of the roofline.** `Perf_RL = samples × HBM_bw / (weights_size + KV_cache × samples)`, KV_cache per sample
  [DIAYN26]. Decode is memory-bound on their hardware at every configuration they tabulate [KL24].
- **Model bandwidth utilization (MBU)**: achieved model-data bandwidth (weights and KV cache) over the system's peak
  memory bandwidth; installed bandwidth times MBU is the effective model bandwidth [HC26].
- **Kernel calls per output token.** For Llama-3.1-8B: 323 on a DGX H100 under TensorRT-LLM, 35 on SN40L-8 with one
  kernel a decoder layer, 4 with KernelLooping [KL24]. A kernel call boundary is a synchronization point; on SN40L-8
  synchronization was 30% of the time per token before KernelLooping and 3% after [KL24].
- **Socket scaling.** KernelLooping lets decode scale across SN40L sockets, with speedups up to 2.5× [KL24]; the SN50 talk reports
  DeepSeek-R1 from 64 to 256 RDUs [HC26].
- **Speculative decoding.** A step is k draft runs and one target run; the acceptance rate is AR = (a+1)/(k+1);
  latency = (draft_latency × k + target_latency) / (AR × (k+1)); acceptance is reported on AlpacaEval, HumanEval and
  GSM8K at k = 5 and k = 9 [DIAYN26].
- **The compiler's units.** The compiler translates a model into kernels (fused subgraphs) and schedules (kernel order)
  and assembles them into one PEF file; the runtime concatenates schedules [DIAYN26]. The SambaFlow user surface adds
  compile modes O0 (operator), O1 (module, its operator fusions steered by rules in YAML) and O3 (full graph, with a
  default, a GEMM-dominated and an attention heuristic) [SNSW24], and a tensor-parallel mode across RDUs [SFRN].

- **The Plasticine evaluation kit's measures** [PTUT]. The 2019 Stanford Plasticine kit, the most direct public trace of
  how a Plasticine-principled evaluation is run, scores two kinds of submission:
  - a PyTorch module behind a `get_model_info(batch_size)` function returning the model, dummy inputs, and an optional
    loss and its arguments. Its Model Analyzer maps the network in a spatial mode onto chips of 64 compute units and 64
    memory units, a chip count and a weight sparsity the submitter sets, and reports: nodes (original and after merging),
    function nodes, maximum depth, total FLOPs, chip FLOPS utilization, compute units used, throughput in samples a
    second, stage latency, model latency, the same for a training step, memory bytes, memory units and their
    utilization, and power;
  - a graph-analytics program (NetworkX) or a sparse linear-algebra program (numpy, scipy, `torch.sparse`), run under
    the kit's tracer (`stubbed` 0.1.1 [STUBBED]), whose memory spaces and access trace drive the Tungsten estimator to a
    cycle count, a runtime and a power figure.
  Its throughput is one over the stage latency and its model latency is depth times the stage latency: the network is
  laid out whole and pipelined over samples (squeezenet: a depth of 21 merged nodes, a stage latency of 85.0 µs, a
  model latency of 1.8 ms and 11.8K samples a second [PTUT]).

- [x] Every measure above is a column of the cell ledger, with its definition and source (SCORE-1; receipt: `samba_cells_lock::every_measure_of_section_2_1_is_a_column_with_its_definition_and_source`).
- [ ] Our compiler produces the Model Analyzer's report fields for any `get_model_info` module (PLAST-13).
- [ ] Our time model reports every measure above for any deployment it prices (TIME-1, TIME-6).

### 2.2 The Copy bars

Each row is a published result: the cell it bars, the chip and its count, the model and bits, the regime, the number,
and its source. A row's number is a Copy bar only at the regime it states.

| Cell | Chip | Model, bits | Regime | Published | Source |
|---|---|---|---|---|---|
| C-SN40Lx8-LLAMA31-8B-D1-BF16 | SN40L, 8 sockets | Llama-3.1-8B, BF16 | batch 1, decode | 584 tok/s, 78% of roofline | [KL24] |
| C-SN40Lx16-LLAMA31-8B-D1-BF16 | SN40L, 16 sockets | Llama-3.1-8B, BF16 | batch 1, decode | 1,140 tok/s, 76% of roofline | [KL24] |
| C-SN40Lx16-ZOO-KL-BF16 | SN40L, 16 sockets | the paper's model set | decode, KernelLooping on against off | 1.6× geometric mean, 2.2× at most | [KL24] |
| C-SN40Lx16-ZOO-ROOF | SN40L, 16 sockets | dense, MoE and hybrid models | decode with KernelLooping, BatchStreaming and ScheduleOffloading | over 75% of the roofline | [DIAYN26] |
| C-SN40Lx16-LLAMA31-70B-SPEC-K9-BF16 | SN40L, 16 sockets | Llama-3.1-70B target, 8B draft, BF16 | speculative, 9 draft tokens a step | 9 + 6.7 = 15.7 ms a step (draft, then target), against 33 + 12.6 = 45.6 on a DGX H100 | [DIAYN26] |
| C-SN40Lx16-ZOO-SPEC | SN40L, 16 sockets | the paper's draft and target pairs | speculative decoding | 1.7× a DGX H100; over 6× the target alone | [DIAYN26] |
| C-SN40Lx8-LLAMA31-8B-SYNC | SN40L, 8 sockets | Llama-3.1-8B | kernel calls a token | 4 (35 without KernelLooping) | [KL24] |
| C-SN50-LLAMA33-70B-D-FP8 | SN50 | Llama-3.3-70B, FP8 | 1K input, 1K output | 895 tok/s a user, measured by SemiAnalysis InferenceX | [SN50PR] |
| C-SN50x16-MINIMAXM27-PD | 16 SN50 for decode, 4 H200 for prefill | MiniMax M2.7 | 10,000 input tokens | 763 tok/s | [HC26] |
| C-SN50-MINIMAXM27-D | SN50 | MiniMax M2.7 | short and long context | up to 850 tok/s short; over 450 long | [SN50MM] |
| C-SN50x64-DSR1-D | 64 SN50 | DeepSeek-R1 | 8K input, 1K output | 200 tok/s a user, 51% MBU, modeled | [HC26] |
| C-SN50x128-DSR1-D | 128 SN50 | DeepSeek-R1 | 8K input, 1K output | 300 tok/s a user, 44% MBU, modeled | [HC26] |
| C-SN50x256-DSR1-D | 256 SN50 | DeepSeek-R1 | 8K input, 1K output | 500 tok/s a user, 45% MBU, modeled | [HC26] |
| C-SN50x512-ZOO-MBU | 512 SN50 | not stated | aggregate | MBU in the 40% range; over 350 TB/s of aggregate model bandwidth | [STH26] |
| C-SN50-DSV3-EXPERT-BW | SN50 | DeepSeek-V3 | expert loads in decode | close to 80% of bandwidth | [STH26] |
| C-SN50x32-GEMM-PEAK | 8, 16 and 32 SN50 | GEMM under tensor parallelism | compute | at least 70% of peak FLOPs; full compute and communication overlap at 32 | [HC26] |
| C-SAMBACLOUD-OSS120B-D | SambaCloud (generation not stated) | gpt-oss-120b, low reasoning | output speed | 740 tok/s as captured 2026-10-29 | [AA26] |
| C-PLAST64-SQUEEZENET-INFER | one chip of 64 compute and 64 memory units | squeezenet, batch 1 | spatial mapping, inference | 11.8K samples/s, 65.804% FLOPS utilization, 85.0 µs a stage, 1.8 ms latency, 701.0M FLOPs, 125 memory units | [PTUT] |
| C-PLAST64-SQUEEZENET-TRAIN | one chip of 64 and 64 | squeezenet | training step | 2.1G FLOPs; "configuration too small to map training" on one chip | [PTUT] |
| C-PLAST-RANDOMWALK-TRACE | the kit's Tungsten configuration, 1 GHz, four DRAMSim2 channels | NetworkX random walk on the kit's dataset | trace-driven | 95,592,587 cycles, 0.096 s | [PTUT] |

GPU references from the same papers, for scale: Llama-3.1-8B at batch 1 decodes at 205, 265 and 293 tok/s on 2, 4 and
8 H100s [DIAYN26], 21% of the DGX H100's HBM bandwidth at 8 [KL24].

- [ ] Every Copy bar is a row of the cell ledger with its citation (SCORE-2).
- [ ] Every Copy bar's regime is reproducible by the exam runner's prompt set and context (EXAM-3, EXAM-4).
- [ ] A Copy bar that a newer publication supersedes is replaced in the ledger with both citations kept (STUDY-1).

### 2.3 Cells and regimes

A cell is `C-<chip>-<model>-<regime>-<bits>`. The model is the checkpoint's short name in capitals (`LLAMA31-8B`,
`DSR1`), or `ZOO` for a result over a set of models. The bits are left off where the source does not state them or the
regime does not depend on them.

| Chip | What it is | A win is |
|---|---|---|
| SN40L, SN50 | the RDU, with a socket count where there is more than one (`SN50x16`) | measured |
| PLAST | the Plasticine chain: Tungsten on the SARA 20×20 HBM2 preset, and on an SN-shaped mesh once PLAST-9 lands | simulated |
| PLAST64 | the kit's chip of 64 compute and 64 memory units, with a chip count where there is more than one | simulated |
| SAMBACLOUD | SambaNova's hosted service, whose chip generation it does not state | never: a Copy bar only |

| Bits | The weights |
|---|---|
| BF16 | bfloat16 |
| FP8 | 8-bit floats (e4m3) with their scales |
| Q8 | 8.5 bits a weight: 8-bit integers with a 16-bit scale for each block of 32 |
| MXFP4 | 4-bit floats in blocks of 32 sharing an 8-bit exponent |
| NVFP4 | 4-bit floats in blocks of 16, each with an 8-bit float scale |
| INT4 | 4-bit integers with group scales (AWQ, GPTQ) |

| Regime | What it measures |
|---|---|
| D1, D8, D16, D64 | decode at batch 1, 8, 16, 64; 4,096-token context; steady-state time per token and tok/s |
| L32K, L64K, L262K | decode at batch 1 with the cache at 32,768, 65,536 and 262,144 positions |
| SPEC-K5, SPEC-K9 | speculative decoding at 5 and 9 draft tokens a step: effective tok/s and acceptance |
| TTFT-8K | time to first token of an 8,192-token prompt |
| PD | prefill on a GPU, decode on the RDU, KV handed over |
| SCALE-n | the D1 cell at n sockets, and its ratio to the 1-socket or 8-socket cell |
| COMPILE | wall seconds from a checkpoint to a deployable bundle |
| BRINGUP | wall time from receiving an unseen checkpoint to the first token that equals the reference, every code change counted |
| SYNC | synchronization boundaries a token: barriers, kernel calls and host returns |
| MBU | model bandwidth utilization in the D regimes |
| D | decode at the protocol a published result states: its batch, input and output lengths beside the row |
| KL | decode with KernelLooping on against off: the speedup |
| ROOF | decode's share of its roofline (section 2.1) |
| SPEC | speculative decoding at the source's own draft lengths, against the baseline it states |
| EXPERT-BW | the share of memory bandwidth decode spends loading experts |
| PEAK | a kernel's share of the chip's peak FLOPs |
| INFER, TRAIN | the Plasticine kit's spatial mapping of a network: inference, or one training step |
| TRACE | a program run trace-driven under Tungsten: cycles and seconds |

The ledger is `benchmarks/silicon/samba_cells.json` (SCORE-1). Its columns are the measures of section 2.1 and the
exam's own, each with its unit, the direction that is better, its definition, and its source or regime. Per row: the
cell, the side (ours, or the citation of a published result), the provenance (published, simulated, modeled, measured,
or not yet), the date, the values and the runs behind them, the conditions as the source states them and where in the
source, the program identity (spec digest, cut, source sha256), the bundle digest, the time-model version and
calibration set when modeled, the simulator and its tolerance when simulated, the boot and load when measured, the
quality gates, the closers, and the log. `samba_cells_lock` holds every row to section 2.4.

- [ ] The ledger's schema, approved by Tristen (SCORE-1).
- [ ] Every D, L and SPEC cell of the Tier A models (section 6) has a modeled row (SCORE-3).
- [ ] Every PLAST cell of a layer kind has a simulated row (PLAST-1, SCORE-3).

### 2.4 The rules of a fair win

- **Matched chip.** The same chip at the same socket count, or the published number's own count.
- **Matched bits.** We never read fewer weight bytes a token than the rival because of a narrower format than it used,
  unless the cell's bits say so. The KV cache is never narrower than theirs at a matched cell.
- **Matched work.** Their protocol (steady state over 10 prompts, the stated context and batch) or the exam's.
- **Matched quality.** Top-1 agreement with the f32 reference over the golden prompts at least the reference
  implementation's own at the same bits; KL to the reference and perplexity on a fixed text within the bounds NUM-7
  declares. We hold these stricter than any threshold an evaluator states.
- **Provenance decides what can win.** A Plasticine cell is won only by a simulated row; an SN cell only by a measured
  row. A modeled row ranks plans and predicts; it never wins.
- **A real win.** Measured: at least 5 runs a side, our mean ahead by more than 2 combined standard errors, our slowest
  run ahead of their fastest. Simulated: deterministic, ahead by more than the simulator's own stated tolerance.

- [x] `samba_cells_lock` refuses a won row whose provenance is modeled (SCORE-1; receipt: `samba_cells_lock::a_won_row_is_never_modeled`).
- [x] `samba_cells_lock` refuses a won row without quality gates beside it (SCORE-3; receipt: `samba_cells_lock::a_won_row_carries_its_quality_gates`).

### 2.5 When a cell stays lost

1. Attribute both sides where both can be seen: our time model's breakdown (TIME-6) and theirs where a paper gives one
   (their Figure 2 and Figure 4 breakdowns of DGX H100 and SN40L time per token [KL24]).
2. Classify each difference: bytes moved, synchronization, idle compute or memory units, routing, collectives, host,
   numerics.
3. Open one ticket per difference, citing the source of the difference, and add it to the cell's closers (SCORE-4).
4. Leave the cell open until it is won.

- [ ] Every lost cell names its closers (SCORE-4).

---

## 3. What we will have access to

We do not know which access an evaluation grants. It may be the papers alone, SambaNova hardware with SambaFlow, a
lower-level SDK, a cycle simulator, or a drop of our artifacts that they run themselves. The evaluation's tests rest on
Plasticine's principles whatever the access. So every level below has its adapter built before the first day, and every
adapter is held by a lock that runs today on what we hold.

| Level | What it is | What we can run today | The adapter built now | Tickets |
|---|---|---|---|---|
| A0 Ours | the VM oracle, the spatial evaluator and certificate, Spatial's simulator, the Plasticine chain (Spatial, PIR, plastiroute, Tungsten), ttsim, the RTX 3060, the D5005 card, the reference implementations in `external/competitors` | everything | the chain at real width, our placement simulated, a calibrated time model | PLAST-1 to PLAST-10, TIME-1, TIME-2 |
| A1 Public SambaNova | the papers [KL24] [DIAYN26] [SN40L24], the Hot Chips talks [HC24] [HC26], the SambaCloud API, the SambaFlow user documentation | quality and latency of their served models through the API (`scripts/accel/sambacloud-smoke.sh`) | Copy bars, a reference side for quality | STUDY-1, SCORE-2 |
| A2 A research allocation | a research testbed's SambaNova systems [ALCF]: its SN40L serves inference through SambaStack, and its open call for proposals names other platforms | nothing until granted; no compiler access offered there today | the allocation runbook; the artifact drop | ACCESS-6 |
| A3a SambaFlow on SN40L or SN50 | their compiler under a PyTorch front end | nothing until granted | a PyTorch module per program the compiler writes, with the fusion-rule file and compiler configuration their modes read | ACCESS-2 |
| A3b A low-level SDK | unit configurations, a Spatial-like IR, or PEF-level kernels and schedules | nothing until granted | three emitters kept live (Spatial source, the neutral placement, unit configurations at Plasticine granularity) and a bridge that reads a vendor placement back and certifies it | ACCESS-3, ACCESS-8 |
| A3c A cycle simulator | their model of the chip | nothing until granted | one harness interface our Tungsten runs already use | ACCESS-5 |
| A4 The Plasticine evaluation kit | the 2019 Stanford kit [PTUT]: `get_model_info` modules for the Model Analyzer; the `stubbed` tracer [STUBBED] and the trace-driven Tungsten estimator for graph and sparse programs | the tracer and the Tungsten tree (its trace format and sparsity apps are in the tree we build) | the estimator rebuilt from our Tungsten tree; LOGOS programs traced in the kit's format; the Analyzer's report produced by our compiler | STUDY-5, PLAST-11 to PLAST-14 |
| A3d They run our artifacts | the drop is all they see | the drop runs on our evaluator | a self-checking drop: manifest, weights, goldens, certificates, a runner and its expected output | ACCESS-4 |

**The first hour at any level** is the intake card (ACCESS-1): the questions whose answers change a plan, asked in a
fixed order so the answers drop straight into the chip facts:
1. Which level, which formats in and out, which version.
2. Units: PCU and PMU counts, a PCU's lanes and stages and its systolic shape, a PMU's capacity and banks, the clock.
3. Memory: HBM capacity and bandwidth a socket, DDR capacity and bandwidth, SRAM a socket.
4. Links: the P2P bandwidth and topology inside a node and between nodes.
5. Numerics: accumulator widths, FP8 and FP32 support in each unit, the index type.
6. Limits: kernel and schedule size, compile-time budget, the number of distinct kernels a PEF may hold.
7. The runtime: schedule concatenation, host round trips, the profiler's counters.

- [ ] A0: every adapter's lock runs green on what we hold today (ACCESS-2 to ACCESS-5, ACCESS-8).
- [ ] A1: the Copy bars are ledger rows (SCORE-2); the SambaCloud quality side is recorded for the Tier A models it
  serves (ACCESS-7).
- [ ] A2: the allocation runbook is current (ACCESS-6).
- [ ] A3a: a SambaFlow module for every program the compiler writes, experts included, imports under a stub of their
  package and computes what the VM computes (ACCESS-2).
- [ ] A3b: a vendor placement in their schema is read back and certified against our source (ACCESS-3); unit
  configurations are emitted at Plasticine granularity (ACCESS-8).
- [ ] A3c: the simulator harness reads cycles and checks values through one interface (ACCESS-5).
- [ ] A3d: the drop verifies itself on a clean machine with only its runner (ACCESS-4).
- [ ] A4: a `get_model_info` module, a NetworkX program and a scipy program each run end to end through the kit's flow
  and through ours, and agree where both report (STUDY-5, PLAST-11, PLAST-13, PLAST-14).
- [ ] The intake card's answers flow into the chip facts with provenance `measured` or `vendor` (ACCESS-1, CHIP-4).

---

## 4. The chips, Plasticine first

### 4.1 Plasticine's principles, as our lowering must speak them

Plasticine [ISCA17] and the Spatial language [PLDI18] are the architecture and the programming model the RDU descends
from; SARA [SARA21] is the compiler that scales them. An evaluation built on Plasticine's principles tests whether a
compiler speaks them:

- **Pattern compute units (PCUs).** SIMD lanes across pipelined stages, a cross-lane reduction tree, counters and a
  control box. The ISCA'17 PCU is 16 lanes by 6 stages. Ours: `crates/logicaffeine_compile/src/codegen_sva/metapipeline.rs` and
  `crates/logicaffeine_compile/src/codegen_sva/plasticine.rs` model it; the RDU unit model (`plasticine_inherited` in
  `crates/logicaffeine_compile/src/spatial/rdu_chip.rs`) inherits it.
- **Pattern memory units (PMUs).** A banked scratchpad (ISCA'17: 16 banks of 16 KB) with an address pipeline and banking
  modes (strided, FIFO, line buffer, duplication), N-buffered so a metapipeline's stages overlap across iterations.
  Ours: `crates/logicaffeine_compile/src/codegen_sva/pmu.rs`.
- **Address generators and coalescing units** that turn tile loads into DRAM bursts.
- **The interconnect.** Static word-level scalar, vector and control networks; plastiroute adds a dynamic network with
  virtual channels. Four of our ten Qwen3.8 segments fail the routed network today on virtual-channel coloring
  (`benchmarks/silicon/qwen38_fabric_segments.json`; PLAST-2).
- **Hierarchical control.** Sequential, Pipe (a metapipeline: stages overlap across iterations through N-buffered
  memories), Parallel, and Stream (FIFO-coupled, data-driven). Our composed programs run their sections as a Sequential
  with a barrier at each boundary (SCHED-2, PLAST-4).
- **Parallelization factors and the initiation interval.** Unrolling (`par`) and the II a pipeline sustains.
- **Virtual units, split and merged to fit** [SARA21]: a program larger than the fabric is partitioned onto virtual
  PCUs and PMUs, then mapped (SCHED-9, PLAST-9).
- **Tiling and double buffering** between DRAM and scratchpads.

- [ ] Every principle above has a lowering that produces it and a lock that checks the produced program has it
  (PLAST-4, PLAST-5, SCHED-2, SCHED-9, CHIP-2, CHIP-3).

### 4.2 SN40L and SN50

| Fact | SN40L | SN50 | Source |
|---|---|---|---|
| Dies, cores | 2 dies, 4 cores | 2 dies | [DIAYN26], [STH26] |
| Compute and memory units | 1040 PCUs and 1040 PMUs a socket; a PCU runs as a systolic array; a PMU holds 512 KB | not published | [DIAYN26] |
| BF16 compute | 638 TFLOPS a socket | 5× SN40's FLOPS | [DIAYN26], [STH26] |
| On-chip SRAM | 520 MB | 432 MB | [DIAYN26], [HC26] |
| HBM | 64 GB at 1.6 TB/s | HBM2e | [DIAYN26], [STH26] |
| DDR | 1.5 TB at over 100 GB/s | not published in the talk | [DIAYN26] |
| Off-chip units | AGCUs bridge to HBM, DDR, host, remote sockets; P2P collectives SRAM to SRAM | SRAM-to-SRAM streaming collectives | [DIAYN26], [STH26] |
| Scale-up | 8 or 16 sockets | ten 800G Ethernet ports, 2 TB/s an RDU: seven make the eight-socket full mesh, two reach the inter-node fabric; 256+ chips | [HC26] |
| Scale-out | not stated | 400G RoCEv2 a socket | [HC26] |
| Rack | not restated here | 16 RDUs in an air-cooled rack, two nodes | [STH26] |

`crates/logicaffeine_compile/src/spatial/rdu_chip.rs` holds these with provenance (`Fact { value, provenance, cite }`). Its SN50 entry
also carries the product page's table [SN50WEB]: 1600 TFLOPS at BF16 and 3200 at FP8, 432 MB of SRAM, 64 GB of HBM2E and
up to 512 GB of DDR5, beside an SN40 column of 640 TFLOPS, 520 MB, 64 GB of HBM2E and up to 512 GB of DDR5. Five
figures disagree between sources, and CHIP-1 keeps both of each:
- SN50 DDR5: up to 512 GB on the page [SN50WEB], up to 2 TB in the datasheet [SN50DS].
- SN40L DDR: up to 1.5 TB in both papers [SN40L24], [DIAYN26], up to 512 GB of DDR5 on the page [SN50WEB].
- SN40L BF16: 640 TFLOPS on the page [SN50WEB], 638 at Hot Chips [HC24] and in the MLSys paper [DIAYN26]; the tree
  carries 640 and cites Hot Chips for it.
- SN40L HBM bandwidth: 1.6 TB/s in the MLSys paper [DIAYN26]; 1.8 TB/s in the SN40L paper's table and about 2 TB/s in
  its prose [SN40L24]. The tree carries 1.6 and keeps 1.8 beside it.
- SN40L DDR bandwidth: over 100 GB/s in the MLSys paper [DIAYN26], over 200 GB/s in the SN40L paper [SN40L24]. The
  tree carries the first and keeps the second beside it.

The vendor counts capacity in powers of two. It states 1040 PMUs of 512 KB and 520 MB of SRAM [DIAYN26]. Their product,
532,480 KB, is 520 MB only when a MB is 1,024 KB. The SN40L paper writes the same figures as 520 MiB, 64 GiB and
1.5 TiB in its prose and as 520 MB, 64 GB and 1.5 TB in its table [SN40L24].

### 4.3 What our chip model gets wrong today

Measured from the tree on 2026-10-29:

- [ ] The RDU unit model is Tensix-shaped: 32×32 tiles and 8 accumulator tiles, "the Tensix-class DST budget". At 1040
  PCUs × 16 lanes × 1.0 GHz it reaches about 33 TFLOPS against 638 published for SN40L and 1,600 in our SN50 facts
  (CHIP-2).
- [ ] Capacities are carried in binary units, as the vendor counts them (section 4.2), but no fact says so or shows why;
  the SN40L's 640 TFLOPS cites Hot Chips, which says 638; the SN50's DDR has two published maxima (CHIP-1).
- [ ] HBM bandwidth is modeled as 4 stacks × 460.8 GB/s = 1,843.2 GB/s; SN40L publishes 1.6 TB/s (CHIP-1).
- [ ] There is no DDR roof, no P2P roof and no on-chip rate (CHIP-3, MULTI-6).
- [ ] The cycle model is uncalibrated for the RDU (`crates/logicaffeine_compile/src/spatial/cost.rs`), so the tile pass chooses plans
  on placeholder constants (TIME-1, TIME-2).
- [ ] The decode pricer's chip catalog (`crates/logicaffeine_compile/src/spatial/decode.rs`) has no SN40L or SN50 entry (TIME-3).
- [ ] The fit report sizes Qwen3.8-27B in BF16 at TP16 with state in SRAM; the bundle reads Q8_0 at TP8 and streams the
  state; `benchmarks/silicon/qwen38_vllm_plan.json` asks vLLM for a tensor-parallel width of 16, which 24 heads refuse
  (CHIP-5).
- [ ] The head's buffers take 412 of 432 MiB and push every recurrent state off chip (SCHED-7).

---

## 5. What they do, and our answer

### 5.1 SambaNova's published compiler techniques

| Technique | Source | What it does | Ours today | Tickets |
|---|---|---|---|---|
| One kernel a decoder layer | [KL24] | every op of a decoder in one kernel, so 35 calls a token on SN40L-8 against 323 on a DGX H100 | each layer kind is one composed program, its sections separated by barriers (`crates/logicaffeine_compile/src/spatial/passes/compose.rs`) | SCHED-2 |
| KernelLooping | [KL24] [DIAYN26] | matches a chain of calls to one kernel by how each parameter is used across the calls (constant, read, carried, live, written), then rewrites the chain into one call with a pipelined outer loop; the carried activation stays on chip, double-buffered; weights are iterated | the streamed bundle runs one layer-kind program at a time and rebinds weights a layer (`crates/logicaffeine_compile/src/spatial/streamed.rs`) | SCHED-3 |
| BatchStreaming | [DIAYN26] | a LoopBuffer spread over PMUs lets each sample of a batch enter the next decoder as soon as it leaves this one | none | SCHED-4 |
| ScheduleOffloading | [DIAYN26] | the runtime concatenates schedules so M tokens run without the host | the arg-max and the next token's embedding gather run on chip; no loop over steps | SCHED-6 |
| Speculative decoding on the RDU | [DIAYN26] [SPEC25] | draft and target schedules offloaded together; acceptance modeled per benchmark and window | none on the RDU path | SERVE-RDU-1 |
| All-reduce as a dataflow operator | [KL24] [DIAYN26] [STH26] | P2P streaming between on-chip memories, overlapped with the GEMM and the add, no HBM round trip | a fold centralized on chip 0, unpriced (`crates/logicaffeine_compile/src/spatial/tensor_parallel.rs`) | MULTI-1 |
| Persistent decoder, prefetching PMUs | [HC26] | double-buffered memory units move the next model tile while the current one computes | double-buffered FIFOs inside a section; nothing across layers | SCHED-5 |
| Expert parallelism, two dispatch forms | [HC26] [STH26] | broadcast-and-filter and all-to-all dispatch-combine | none | MULTI-2 |
| Three memory tiers, Composition of Experts | [SN40L24] | DDR holds many models and experts; HBM holds the active ones; SRAM holds the working set | the residency plan places tensors in SRAM, HBM or DDR by density (`crates/logicaffeine_compile/src/spatial/residency.rs`); no switching | MEM-1, MEM-2, MEM-6 |
| One decoder graph for every layer | [DIAYN26] | looping reduces compilation to one decoder graph compiled once, so compile time and memory do not grow with the decoder count | layers of one kind lower to one program with one digest; compile once a kind | SCHED-8 |
| Compile modes and a fusion-rule file | [SNSW24] [SFRN] | O0 operator, O1 module, O3 full graph; fusion rules in YAML and GEMM-dominated heuristics | the shared optimizer and the spatial shim; no user-facing rule file | ACCESS-2, SCHED-10 |
| Heterogeneous disaggregation | [HC26] | prefill on H200s, decode on SN50s, KV over RoCE | one hand-off contract (`spatial::handoff`): a prompt prefilled on the GPU target decodes on the RDU bundle and the reverse, a model splits over two chips; the CUDA lane's card prefill owed | SERVE-RDU-3 |
| MBU as the scaling measure | [HC26] | model bytes moved over installed bandwidth | no time model on the RDU | TIME-1, TIME-6 |

- [ ] Every row's technique has a ticket here that copies it, and a Beyond bar where ours can go past it (STUDY-2).

### 5.2 The Stanford lineage

K. Olukotun's group at Stanford built the architecture and the compilers the RDU descends from. What each work gives us:

| Work | Source | What it contributes | Where an evaluation tests it | Ours today | Tickets |
|---|---|---|---|---|---|
| Plasticine | [ISCA17] | PCUs, PMUs, switches, address generators; parallel patterns on a reconfigurable fabric | the unit model a mapping is judged on | modeled (`crates/logicaffeine_compile/src/codegen_sva/plasticine.rs`) | CHIP-2, CHIP-3, PLAST-5 |
| Spatial | [PLDI18] | controllers, parallelization factors, banking, design-space exploration | whether a program metapipelines and banks | our Spatial emitter (`crates/logicaffeine_compile/src/spatial/lower/spatial_lang.rs`), sections in Sequential order | PLAST-4, PLAST-5 |
| SARA | [SARA21] | compiler-managed memory consistency, which pipelines a nested, data-dependent control-flow graph hierarchically; the program graph decomposed across distributed heterogeneous units | a program larger than a chip | none | SCHED-9, PLAST-9, PLAST-10 |
| Capstan | [CAPSTAN21] | sparse iteration on a vector RDA | block selection in indexed sparse attention; expert gathers | gathers through an indirect tile; no selection | KERNEL-1, ARCH-SPARSE |
| Gorgon | [GORGON] | database parallel patterns added to a fabric built for machine learning, the two sharing its resources | sorting tokens by expert | none | KERNEL-2 |
| Aurochs | [AUROCHS] | lightweight thread contexts on a vector dataflow fabric, trees and hash tables written as dataflow threads, one sparse scratchpad added | a hashed table lookup (Qwen 4's n-gram embedding) | none | KERNEL-5, QWEN4-4 |
| Revet | [REVET] | a language and compiler for dataflow threads | data-dependent control: routing, top-k, trip counts | data-dependent trip counts with a windowed certificate (`crates/logicaffeine_tests/tests/spatial_trip_count_lock.rs`) | PLAST-10 |
| DAM | [DAM24] | a dataflow abstract machine: a simulator framework for timed dataflow programs | a time model of a whole step | none on the RDU | TIME-1 |
| STeP | [STEP] | streaming tensor programs with dynamic shapes | dynamic expert routing as streams | none | MULTI-2, KERNEL-2 |

- [ ] Every row's contribution is confirmed against the pinned paper before this table is used to plan work, and a row
  whose reading changes is re-planned (STUDY-1, STUDY-2).

### 5.3 The other spatial compilers

| Compiler | Chip | What it shows | What we take | Tickets |
|---|---|---|---|---|
| SambaFlow | SN40L, SN50 | the incumbent: section 5.1's techniques in production | every row of 5.1 | section 5.1 |
| tt-mlir, ttnn, tt-metal | Tenstorrent Wormhole, Blackhole | vendor-tuned kernels; our generated 640³ GEMM ran 6,063 cycles against ttnn's 6,542 (`benchmarks/silicon/tenstorrent_gemm.json`) | the fusion book's K-39 to K-41 | FUSIONS:SPATIAL-NONUNIFORM-TILING |
| Loom | Tenstorrent Wormhole, Blackhole | tile programs compiled by solving schedule symbols with CP-SAT: parity with or better than TTNN on GEMM, flash attention and flash decode, with no per-shape tuning [LOOM26] | exact schedules from the hardware description | MINE-4 |
| The dataflow benchmark suite | Cerebras, SambaNova, Graphcore | intra-chip profiling and inter-chip scaling of LLM workloads on dataflow accelerators [DABENCH26] | its measures as exam columns | STUDY-4, EXAM-8 |
| Cerebras, Groq | wafer-scale and LPU | priced in our decode chip catalog (`crates/logicaffeine_compile/src/spatial/decode.rs`) | SRAM-resident spines, compiler-scheduled determinism | MEM-4, NUM-8 |

What none of them publishes, and we carry: a certificate on every placement that an independent checker rechecks
(`crates/logicaffeine_compile/src/spatial/certificate.rs`, `crates/logicaffeine_compile/src/codegen_sva/pnr_certificate.rs`), one model program written once and lowered to every chip
(`one_program_lock`), and the VM as the oracle every chip is held to.

- [ ] The rival table is re-read each quarter; a new spatial compiler that publishes a result on these chips joins it
  with its Copy bar (STUDY-4).

### 5.4 The whole set

Sections 5.1 and 5.2 summarize. The whole set is `benchmarks/samba_techniques.tsv`: every technique each pinned source
describes, read end to end (STUDY-2), one row for each source's account of each technique. A row gives the source's key,
the technique's name, its kind (compiler, mapper, runtime or hardware), what we have of it (`none`, `partial`, `have`,
or `past` where we go beyond the source), the repository path that shows ours, the tickets that copy it, where the
source describes it, a quote of at most 25 words from the pinned text, and the mechanism. Two sources that describe one
technique are two rows under one name, and the name has one answer. A hardware feature is recorded with the chip facts:
one of its tickets is a CHIP ticket. `crates/logicaffeine_tests/tests/samba_techniques_lock.rs` holds the ledger to
the board, the quotes to the captures, and these two tables to the ledger.

The publishers keep four of the papers closed. Their rows come from the abstracts pinned beside their records
[SARA21] [GORGON] [AUROCHS] [SPEC25]; what their full texts describe beyond the abstracts is read the day a copy we may
read is pinned. SARA's and Aurochs' mechanisms also reach the ledger through Revet's account of them [REVET].

Three gaps had no ticket before the ledger, and have one now: directives that pin the compiler's choices (SCHED-12:
SambaFlow's human-decision files and looping decorators, Spatial's scheduling directives), transposes and banks as
layouts (SCHED-13), and storage reused by lifetime and spilled by bandwidth (MEM-7).

**Sources that describe no technique.**
- [HFQ4] A model configuration: Qwen3.8-Flash-Next's shape, read for section 7 (STUDY-3).
- [TFQ4] The PyTorch model and its configuration. Their tensor-, expert- and data-parallel plans are GPU distribution
  plans, recorded in section 7.
- [LCPPQ4] The llama.cpp port and its pull request: a CPU and GPU graph, with the n-gram hash on the host.
- [VLLMQ4] The vLLM model package: its GPU kernels and parallel plans are section 7's references.
- [MLXQ4] The Apple-GPU port: a memory-mapped n-gram table, draft verification and 4-bit experts, recorded in section 7.
- [TINYQ4] A tiny checkpoint's configuration, the golden source of QWEN4-1.

- [ ] Every technique in every pinned source is a row, and every row names a ticket on this board (STUDY-2).

---

## 6. Model coverage

A bring-up test sends a model. The compiler reads a GGUF checkpoint, or a Hugging Face directory as the GGUF the pinned
converter writes from it (`model::hf`), through the arms of `ModelSpec::from_source`
(`crates/logicaffeine_compile/src/model/spec.rs`): llama (Mixtral's checkpoints among them), qwen2, qwen3, qwen3moe,
qwen2moe, qwen35 (Qwen3.5 and Qwen3.8), qwen35moe, qwen3next, gemma3, gpt-oss, deepseek2 and llama4, the names
`model::triage::ARCHITECTURES` lists and `model_triage_lock` holds to the arms. A Qwen3-Next checkpoint, whose converter
groups its value heads by key head and keeps each key head's betas and alphas in one projection, is opened in Qwen3.5's
layout (`model::layout`), so one Gated DeltaNet reads both. Any other architecture string is refused naming the families whose
tensors it carries (`model::triage`). The author's gate (`check` in `crates/logicaffeine_compile/src/model/author.rs`)
refuses several features the f64 reference already computes. `largo emit rdu` writes an SN50 bundle for a spec the author
writes and the spatial passes lower (INTAKE-6); its serving plan sizes only the Qwen3.5 and Qwen3.8 hybrids
(`Qwen38Cfg::from_spec` in `crates/logicaffeine_compile/src/codegen_cuda/qwen38.rs`, CHIP-5).

### 6.1 The tiers

**Tier A: what SambaNova publishes on and serves.** The models of their papers' evaluation [DIAYN26] [KL24] and of
their production rows [HC26] [SN50PR] [SN50MM].

| Model | Architecture | Intake today | Author today | RDU path today | Tickets |
|---|---|---|---|---|---|
| Llama 3.2 1B and 3B; 3.1 8B, 70B, 405B; 3.3 70B | llama | the GGUF `llama` arm and the Hugging Face reader | written | brings up on the SN50 (`rdu_bringup_lock`), its twin from its directory, and every published size's layer kinds lower (`deploy_llama3_lock`) | INTAKE-1, INTAKE-6, DEPLOY-LLAMA3 |
| Llama 4 Scout and Maverick (17B active, 16 and 128 experts) | llama4 | the GGUF `llama4` arm and the Hugging Face reader, the published multimodal checkpoints included | written (`model_conformance_lock`) | its twin brings up on the SN50 from its directory and both sizes' layer kinds lower (`deploy_llama4_lock`) | INTAKE-2, ARCH-WINDOW, ARCH-NOPE, ARCH-DENSE-FIRST, DEPLOY-LLAMA4 |
| Mixtral 8x7B | mixtral (a `llama` GGUF with experts) | the Hugging Face reader and the `llama` arm's experts | written | its twin brings up on the SN50 from its directory and 8x7B's and 8x22B's layer kinds lower (`deploy_mixtral_lock`) | INTAKE-2, ARCH-MIXTRAL, DEPLOY-MIXTRAL |
| Qwen 2.5 0.5B and 72B | qwen2 | the GGUF `qwen2` arm and the Hugging Face reader (`model_hf_intake_lock`) | written: biased query, key and value projections, no output bias (`Attention.output_bias`) | its twin brings up on the SN50 from its directory and every published size's layer kinds lower (`deploy_qwen25_lock`) | INTAKE-2, DEPLOY-QWEN25 |
| gpt-oss 20B and 120B | gpt-oss | the GGUF arm | written | brings up on the SN50 (`rdu_bringup_lock`), its twin from its directory, and both sizes' layer kinds lower (`deploy_oss_lock`); MXFP4 experts owed | ARCH-OSS, ARCH-SINKS, ARCH-WINDOW, DEPLOY-OSS |
| DeepSeek-V3 and R1 (671B) | deepseek_v3 | the `deepseek2` arm reads sigmoid routing, its bias and groups | written: sigmoid routing, its bias, groups and routed scale (`model_conformance_lock`) | its twin brings up on the SN50 from its directory and V3's layer kinds lower at 671B's widths (`deploy_deepseek3_lock`) | AUTHOR-1, ARCH-ROUTING, ARCH-MLA, DEPLOY-DEEPSEEK3 |
| MiniMax M2 series | minimax | the GGUF `minimax-m2` arm and the Hugging Face reader | written: a QK norm across every head (`Attention.qk_norm_whole`) | its twin brings up on the SN50 from its directory and M2's layer kinds lower (`deploy_minimax_lock`) | INTAKE-2, DEPLOY-MINIMAX |

**Tier B: the Qwen line the next release builds on.**

| Model | Architecture | Intake today | Author today | RDU path today | Tickets |
|---|---|---|---|---|---|
| Qwen3.8-Flash-Next | qwen4_exp (llama.cpp `qwen4exp`) | the Hugging Face reader as the converter writes `qwen4exp` (QSA kept on the stack, which the converter loses); the published configuration reads; the f64 reference equals transformers (`qwen4_exp_intake_lock`) | written: Gated DeltaNet's sigmoid gate (`with sigmoid gate`); refused by name: hyper-connections, the n-gram embedding, QSA | the sigmoid-gated delta lowered and equal to the VM (`tensor_gated_delta_gate_lock`); the rest refused by name | QWEN4-1 to QWEN4-13 |
| Qwen3.8-27B and 2.4T-A95B, Qwen3.5 0.8B to 397B-A17B | qwen35, qwen35moe | the GGUF arms and the Hugging Face reader; every release's configuration reads (`model_hf_intake_lock`) | written | certified bundles for decode, batch, prefill and TP; Qwen 3.5's twin brings up on the SN50 from its directory and every release's layer kinds lower (`qwen38_real_dims_lock`); the mixture's twin is past the bf16 bound (NUM-1); the 2.4T's layout (the text model alone, experts fused) brings up | DEPLOY-QWEN38, NUM-1 |
| The Qwen 4 family on release | expected to extend qwen4_exp | none | none | none | QWEN4-13 |

**Tier C: every other open family a test can send.**

| Family | Architecture | What it needs | Tickets |
|---|---|---|---|
| Gemma 1 and 1.1 | gemma | heads wider than the width over the heads at 7B; read from GGUF and Hugging Face, every release's configuration read, the f64 reference equal to transformers (`model_hf_intake_lock`); its twin brings up on the SN50 and 2B's and 7B's layer kinds lower (`deploy_gemma_lock`) | INTAKE-2, DEPLOY-GEMMA |
| Gemma 2 | gemma2 | soft caps, alternating windows, sandwich norms; read from GGUF and Hugging Face, every release's configuration read, the f64 reference equal to transformers with the window and both caps binding (`model_hf_intake_lock`); its twin brings up on the SN50 and 2B's, 9B's and 27B's layer kinds lower, each capped at 50 and 30 (`deploy_gemma_lock`) | INTAKE-2, ARCH-SOFTCAP, ARCH-WINDOW, ARCH-NORMS, DEPLOY-GEMMA |
| Gemma 3 | gemma3 | local and global layers, a non-1/√hd scale at 27B, sandwich norms; every size read from its published configuration, the multimodal 4B, 12B and 27B included, and its layer kinds lowered on the SN50 (`deploy_gemma_lock`), its twin within 2.04% of the bf16 bound's 2% (NUM-1) | AUTHOR-1, ARCH-WINDOW, ARCH-NORMS, DEPLOY-GEMMA |
| Gemma 3n | gemma3n | per-layer embeddings, altered residual streams | INTAKE-2, INTAKE-9 |
| Gemma 4 | gemma4 | a mixture at 26B-A4B, per-layer embeddings at E2B and E4B, local layers with heads of their own width, key-value layers shared down the stack, scores unscaled | INTAKE-2, INTAKE-9 |
| Qwen3.5-MoE | qwen3_5_moe | read from GGUF and Hugging Face and written (`model_hf_intake_lock`, `model_conformance_lock`); its decode step equals the VM on the SN50 at a tiny shape (`spatial_moe_routing_lock`); the gate merged with the router owed | ARCH-SHARED-GATE, DEPLOY tickets |
| Qwen3-Next | qwen3_next | read from GGUF and Hugging Face and opened in Qwen3.5's layout (`model::layout`); its logits from a directory are transformers' within a millionth (`model_hf_intake_lock`); on the SN50 it lowers whole and runs from its directory and its GGUF, its logits past the bf16 bound (E1.13); the gate merged with the router owed | INTAKE-2, ARCH-SHARED-GATE, DEPLOY tickets |
| Kimi K2, Kimi Linear | kimi | latent attention, sigmoid group routing; delta attention; Kimi K2 read (its configurations, and its tiktoken vocabulary to its published GGUF's without running its code) and its twin deployed (`deploy_kimi_glm_lock`); its 384 experts lowered at real width in blocks of the router row (QWEN4-7), Kimi K2 Thinking's int4 experts owed (INTAKE-3), Kimi Linear's delta attention owed | ARCH-MLA, ARCH-ROUTING, ARCH-KDA, DEPLOY-KIMI-GLM |
| GLM-4 | glm4 | half of each head rotated by interleaved pairs, sandwich norms, one gate-and-up projection, biased projections at 9B; GLM-4 0414 read, written and deployed (`deploy_glm_lock`), GLM-4V's multimodal rotation refused by name | AUTHOR-1, ARCH-ROPE, ARCH-NORMS |
| GLM-4.5, GLM-5 | glm4_moe, glm5 | routing variants, partial rotation, indexed sparse attention; GLM-4.5, 4.6, 4.7 and 4.5-Air read, written and deployed (`deploy_kimi_glm_lock`), the draft block read as the converter writes it; GLM-5's indexer owed | ARCH-ROUTING, ARCH-SPARSE, DEPLOY-KIMI-GLM |
| DeepSeek V3.2 | deepseek_v32 | the lightning indexer and block top-k | ARCH-SPARSE, KERNEL-1 |
| Mistral, Ministral | mistral | sliding window (early releases) | INTAKE-2, ARCH-WINDOW |
| Phi-4 | phi3 | fused QKV and gate-up, partial rotation; Phi-4 read, written and deployed (`deploy_phi_lock`), LongRoPE (Phi-3.5, Phi-4-mini) refused by name | AUTHOR-1, ARCH-ROPE |
| Jamba, Falcon-H1, Nemotron-H, Mamba2 | jamba, falcon_h1, nemotron_h, mamba2 | selective and chunked scans beside attention | ARCH-SELECTIVE-SCAN, ARCH-SSD, DEPLOY-HYBRID-SSM |
| RWKV-7 | rwkv7 | its recurrence | ARCH-RWKV |
| MiniMax-Text-01 | minimax_text | lightning attention | ARCH-LIGHTNING |
| LFM2 | lfm2 | gated short convolution | ARCH-SHORT-CONV |
| Qwen3-VL, Llama 4 vision | qwen3_vl, llama4 | a vision tower, multimodal rotation | ARCH-VISION, ARCH-ROPE |
| Whisper | whisper | an encoder, cross-attention | ARCH-CROSS, AUTHOR-2 |
| Embedding and reranking models | various | bidirectional attention, pooling heads | ARCH-POOLING |

**Tier D: the Plasticine kit's workloads** [PTUT]. A Plasticine-principled evaluation sends more than language models.

| Workload | Form it arrives in | What it needs | Tickets |
|---|---|---|---|
| ResNet-50, SqueezeNet and other torchvision networks | a `get_model_info` module, inference and a training step | convolutions, pooling, batch norm, the spatial mapping mode, the Analyzer's report | INTAKE-8, ARCH-CNN, SCHED-11, TRAIN-1, PLAST-13 |
| A network with sparse weights | the same, with a weight-sparsity percentage | sparse weight products at a stated density | KERNEL-8 |
| Random walk, BFS, PageRank | NetworkX under the tracer, or LOGOS | a trace in the kit's format; data-dependent control | PLAST-11, PLAST-12, PLAST-14 |
| Sparse matrix-vector products (CSR, COO) | numpy and scipy under the tracer, `torch.sparse`, or LOGOS | sparse iteration, gathers, a trace | PLAST-14, KERNEL-8 |

- [ ] Every Tier A model reads from its Hugging Face checkpoint and its GGUF (INTAKE-1, INTAKE-2).
- [ ] Every Tier A model deploys on SN50 through the generic path, certified, equal to its reference on the tiny
  configuration of its family (DEPLOY tickets).
- [ ] Every Tier C family either deploys or is refused at intake by name with its missing pieces listed (INTAKE-9).

### 6.2 The feature matrix

| Feature | Needed by | IR today | Author today | RDU today | Tickets |
|---|---|---|---|---|---|
| Grouped and multi-query attention | every family | `Attention` | written | flash fold, grouped decode | DEPLOY tickets |
| Sliding window, alternating patterns | Mistral, Gemma 2 and 3, gpt-oss | `Attention.window` | written | lowered in the sections and flash forms | ARCH-WINDOW |
| Chunked-local attention | Llama 4 | none | none | none | ARCH-WINDOW |
| Attention sinks | gpt-oss | `Attention.sinks` | written | lowered in the sections and flash forms | ARCH-SINKS |
| Soft caps on scores and logits | Gemma 2 | `Attention.softcap`, `Softcap` | written | lowered in the sections and flash forms; a capped tensor one row section | ARCH-SOFTCAP |
| Layers without rotation, tuned temperature | Llama 4 | `rope: None` | written for no rotation | none | ARCH-NOPE |
| QK norm, gated attention output | Qwen3, Gemma 3, Qwen3.5 and 3.8, qwen4_exp | yes | written | lowered | none |
| Latent attention | DeepSeek V2 and V3, Kimi K2 | composed of head slices, joins, head products, rotation and attention | written; refused under TP | decode lowered at whole-tile widths; other widths past an accumulator page owed | ARCH-MLA, AUTHOR-4 |
| Indexed sparse attention | DeepSeek V3.2, qwen4_exp, GLM-5 | a class, refused by every shim | none | none | ARCH-SPARSE, QWEN4-6 |
| Rotation variants | most families | partial NEOX, from a position | partial | NEOX, unscaled | ARCH-ROPE, QWEN4-9 |
| Softmax top-k routing | Qwen3-MoE, Mixtral, gpt-oss | `Moe` | written | lowered at real width; a router row past one accumulator page (Kimi K2's 384, the 512 of Qwen3-Next, Qwen 3.5 and qwen4_exp) scored and routed in blocks of the row, and more picks than a page (top 10) stored and combined in pages | ARCH-MIXTRAL |
| Sigmoid routing, bias correction, group limits, routed scale | DeepSeek V3, Kimi K2, GLM | `Gating::Sigmoid`, `Moe.groups`; the `GroupedTopk` class refused by every shim | written | a group-limited sigmoid step lowered (`spatial_moe_routing_lock`) | ARCH-ROUTING, AUTHOR-1 |
| Gated shared experts | Qwen3.5-MoE, qwen4_exp | `Moe::shared_gate` | written: the gate a product, its sigmoid spread across the row | a whole Qwen3.5-MoE decode step (`spatial_moe_routing_lock`); the gate merged with the router owed | ARCH-SHARED-GATE |
| Dense leading layers, per-layer FFN kind | DeepSeek, Llama 4 | per-layer `Ffn` | written | refused (one layer shape a kind) | ARCH-DENSE-FIRST |
| Clamped SwiGLU, biases, MXFP4 experts | gpt-oss | yes | written | the clamp and the biases lowered in both forms; MXFP4 owed | ARCH-OSS, NUM-3 |
| Gated delta | Qwen3.5 and 3.8, qwen4_exp | `GatedDelta` | written | one-pass step, chunked form | ARCH-GDN-VARIANTS, QWEN4-8 |
| Selective and chunked state-space scans | Jamba, Falcon-H1, Nemotron-H, Mamba2 | none | none | none | ARCH-SELECTIVE-SCAN, ARCH-SSD, KERNEL-3 |
| RWKV-7, lightning attention, delta attention | RWKV, MiniMax-Text-01, Kimi Linear | none | none | none | ARCH-RWKV, ARCH-LIGHTNING, ARCH-KDA |
| Short convolution | LFM2 | depthwise `CausalConv` | none | the causal convolution lowers | ARCH-SHORT-CONV |
| Hyper-connections | qwen4_exp, DeepSeek V4 | none | none | none | QWEN4-2 |
| Hashed n-gram embedding | qwen4_exp | none | none | none | QWEN4-3, QWEN4-4, QWEN4-5 |
| Multi-token prediction heads | DeepSeek V3, Qwen3.8, qwen4_exp | dropped at intake | dropped | none | AUTHOR-3, SERVE-RDU-1, QWEN4-10 |
| LayerNorm, plain MLP, erf GELU | Phi, Whisper, encoders | `FusedRow` LayerNorm | refused | none | AUTHOR-2, AUTHOR-5, ARCH-NORMS |
| Sandwich norms, embedding scale, (1 + w) norms | Gemma | yes | written | refused | ARCH-NORMS |
| Fused QKV, value width unlike key width, a non-1/√hd scale | Phi-3, GLM-4, latent attention, Gemma 3 27B | yes | written, on one chip and split over chips (a fused projection's parts bound through `View::FusedPart`) | none | AUTHOR-1 |
| Vision towers | Qwen3-VL, qwen4_exp, Llama 4 | none | none | none | ARCH-VISION, QWEN4-11 |
| Cross-attention, encoder and decoder | Whisper, T5 | single-query only | none | none | ARCH-CROSS |
| Pooling heads, bidirectional attention | embedding and reranking models | none | none | none | ARCH-POOLING |
| FP8 block-scaled weights | DeepSeek V3 and R1, Kimi K2, MiniMax-M2, FP8 releases | read in place: a directory's FP8 weights dequantized to f32 as the converter writes them (INTAKE-3); no FP8 datapath | none | none | INTAKE-3, NUM-2 |
| NVFP4, AWQ, GPTQ weights | quantized releases | none | none | none | INTAKE-3, NUM-4, NUM-5 |
| Contexts to 262,144 and 1M | qwen4_exp, Qwen3.8 | yes | written | tested to 4,096 | MEM-3, QWEN4-6, EXAM-3 |

- [ ] Every row's RDU column reads "lowered" or names the refusal a deployment lock pins (the DEPLOY tickets, INTAKE-9).

---

## 7. Qwen3.8-Flash-Next and Qwen 4

Qwen3.8-Flash-Next is the published preview of the architecture the next Qwen generation builds on. Its configuration
names the model type `qwen4_exp` and the class `Qwen4ExpForConditionalGeneration` [HFQ4]; llama.cpp converts it as
architecture `qwen4exp`, which is not `qwen3next` [LCPPQ4]. Reference implementations exist in transformers [TFQ4],
vLLM [VLLMQ4] (`external/competitors/vllm/vllm/models/qwen4_exp`), llama.cpp (`external/competitors/llama.cpp/src/models/qwen4exp.cpp`)
and MLX [MLXQ4], and a tiny random checkpoint exists for tests [TINYQ4].

### 7.1 What the checkpoint is

| Configuration key | Value | What it sets |
|---|---|---|
| `num_hidden_layers`, `hidden_size` | 48, 2560 | the main stack |
| `layer_types`, `full_attention_interval` | three `linear_attention` then one `full_attention`, repeated; 4 | 36 gated-delta layers, 12 attention layers |
| `linear_num_key_heads`, `linear_num_value_heads`, `linear_key_head_dim`, `linear_value_head_dim`, `linear_conv_kernel_dim` | 16, 48, 128, 128, 4 | the gated-delta layer |
| `num_attention_heads`, `num_key_value_heads`, `head_dim` | 24, 2, 256 | the attention layer |
| `partial_rotary_factor`, `rope_parameters` | 0.25; interleaved multimodal sections [11, 11, 10], θ 1e7 | 64 rotated dimensions of 256 |
| `indexer_n_heads`, `indexer_head_dim`, `indexer_kv_heads`, `indexer_compress_ratio`, `indexer_budget` | 4, 128, 1, 4, 2048 | the sparse-attention indexer |
| `hc_count`, `hc_lowrank` | 4, 320 | hyper-connections: four residual lanes, rank-320 mixers |
| `ple_layer_ids`, `ngram_size`, `heads_per_ngram`, `ngram_vocab_size_base`, `split_ngram_parts`, `ple_embed_dim`, `ple_conv_kernel_size` | [2], 3, 8, 20,000,000, 128, 2560, 4 | the hashed n-gram embedding at layer 2 |
| `num_experts`, `num_experts_per_tok`, `moe_intermediate_size`, `shared_expert_intermediate_size` | 512, 10, 640, 640 | the experts |
| `output_gate_type` | sigmoid | the gated-delta output gate |
| `mtp_num_hidden_layers`, `mtp` | 1; one hybrid full-attention layer | the draft head |
| `vocab_size`, `tie_word_embeddings` | 248,320, false | the embedding and the head |
| `max_position_embeddings`, `mamba_ssm_dtype` | 262,144; float32 | context and recurrent-state precision |
| `vision_config` | depth 27, hidden 1152, patch 16, spatial merge 2, temporal patch 2 | the Qwen3-VL vision tower |

- [ ] The intake reads every key above from the checkpoint's own `config.json`, and refuses by name any key it does not
  understand (QWEN4-1).

### 7.2 Each component, its meaning and its references

Section 7.7 gives the file and line of each formula below in transformers, vLLM and llama.cpp, and every place they
differ.

**Hyper-connections** [TFQ4] (`external/competitors/vllm/vllm/models/qwen4_exp/common/hyperconnection.py`). The residual
stream is four lanes of 2,560, 10,240 wide, and begins as four copies of the token's embedding. Every layer norm is
replaced by a gated low-rank mixer. Before each sublayer (the mixer, then the FFN):
- `x̂ = RMSNorm(h)`, each 2,560-wide lane normalized on its own and scaled by `1 + w` (one weight of 10,240, stored
  centred on zero);
- `w = σ(U · silu(D · x̂ / 4))`, with D 10,240 → 320 and U 320 → 10,240;
- the sublayer's input is `mean over lanes of (w ⊙ x̂)`, 2,560 wide;
- the injection weights are `inj = 2σ(J · x̂ / 4)`, with J 10,240 → 4, read from the same `x̂`;
- after the sublayer, `h' = h + out ⊗ inj`: the 2,560-wide output added to each lane by its own weight.
The final norm is the last mixer: a mixer without injection, whose 2,560-wide output is what the head reads. The
checkpoint has no other norm on the residual stream.

**The embedding and the head**. The embedding is 248,320 × 2,560, and the head a separate matrix of the same shape
(`tie_word_embeddings` is false). The draft head has neither of its own: it reads the model's.

**The hashed n-gram embedding** [TFQ4] (`external/competitors/vllm/vllm/models/qwen4_exp/common/ple.py`,
`external/competitors/vllm/vllm/models/qwen4_exp/common/ngram_embedding.py`). In the second decoder layer (index 1;
`ple_layer_ids` counts layers from one):
- the token history begins with end-of-sequence padding, and a token before the start of its segment (an
  end-of-sequence token ends a segment) reads as the end-of-sequence token, as does a padding position of the batch;
- for the bigram and the trigram, the token and its predecessors are multiplied by the layer's odd multipliers
  (`m0`, `m1`, `m2`, drawn by splitmix64 from the layer's seed and bounded so that every product fits a signed 64-bit
  integer) and the products exclusive-ored: `(t0 · m0) ⊕ (t1 · m1)` and `(t0 · m0) ⊕ (t1 · m1) ⊕ (t2 · m2)`;
- each of an order's 8 heads reduces that value by its own prime and adds the head's offset in the table;
- the ids gather rows from one table of about 320 million rows by 160 [MLXQ4], 51.2 billion parameters; llama.cpp's
  conversion is 97.7 GiB [LCPPQ4]; the 16 gathered rows of 160 concatenate to 16 × 160 = 2,560;
- a key projection to 10,240 (each lane's key normalized) and a value projection to 2,560; a gate per lane
  `g = k̂ · q̂ / √2560` against the normalized stream, taken to `sign(g) · √max(|g|, 1e-6)` and through a sigmoid to
  weight the value; the output is the gated value plus a depthwise convolution over its normalized copy (10,240
  channels, kernel 4, dilation 3, through SiLU), whose state lives in the recurrent cache;
- the result is added to the residual stream before the layer's mixer.
The table is meant for host or DDR memory: llama.cpp keeps it on the host with `-ot "ple_ngram_embd=CPU"` [LCPPQ4].

`model::ngram_hash` computes the rows, and `qwen38_flash_next_facts_lock` holds them to transformers' module and to the three
integer tensors the checkpoint stores beside the table. The configuration has no `seed`, so the configuration class's
default of 1234 applies; at it the multipliers are 23,703,573,157,769, 20,109,073,645,365 and 8,052,911,324,071, the
values the checkpoint stores. The heads' primes are the 16 consecutive primes after 19,999,999, from 20,000,003 to
20,000,171; the table has 320,001,446 rows, padded to 320,001,536 = 128 × 2,500,012, its 128 shards in the checkpoint.

**Qwen sparse attention** [TFQ4] (`external/competitors/vllm/vllm/models/qwen4_exp/nvidia/ops/qsa_indexer.py`,
`external/competitors/vllm/vllm/models/qwen4_exp/common/qsa_cache.py`). On the 12 attention layers:
- the indexer keeps one raw key a token (128 wide, one KV head); each complete block of 4 visible tokens pools to its
  mean, taken in float32 and rounded to bfloat16, then normalized and rotated at the block's first position;
- a query of 4 heads, normalized and rotated (the attention's 64 rotated dimensions, the first 64 of the 128), scores
  each pooled block `Σ_h relu(q_h · k̄) / √128` in float32;
- the 512 best blocks (2,048 tokens) are kept, plus the tokens of the incomplete tail block;
- dense attention runs over the kept tokens only: 24 query heads, 2 KV heads, head dim 256, each head's query and key
  normalized, the output multiplied by a sigmoid gate that the second half of each head's query projection carries;
- up to 2,048 + 4 − 1 = 2,051 visible tokens the selection keeps every token, so the layer is exactly dense; blocks
  that tie at the 512th place fall to `torch.topk` in transformers, a CUDA top-k kernel in vLLM and `ggml_top_k` in
  llama.cpp, and no reference fixes the order.
The llama.cpp port lifts its top-k mask builder from its DeepSeek sparse-attention support and reports a mean Jaccard
of 0.975 against the reference above the budget, bit-identical below it [LCPPQ4].

**Gated delta** [TFQ4]: the recurrence of the Qwen3.5 gated delta layer, with a sigmoid output gate in its gated
RMSNorm in place of SiLU (`output_gate_type`). Queries and keys are L2-normalized and the queries scaled by one over
the square root of the key head dimension; MLX's notes call it the readout scale `1/sqrt(head_v_dim)` [MLXQ4]. Both head
dimensions are 128, so the two readings agree. Value head j of 48 reads key head ⌊j/3⌋ of 16; the decay is
`exp(−exp(A_log) · softplus(a + dt_bias))` and the write strength `σ(b)`. Recurrent state in float32.

**Experts** [TFQ4] [HFQ4]: a softmax router over 512 experts, top 10, renormalized; each expert a SwiGLU of 2,560 → 640 → 2,560;
one shared expert of the same shape under a sigmoid gate `σ(g · x)`. The router's logits are taken in the activation
type and its softmax in float32; `norm_topk_prob` is absent from the configuration, and its default renormalizes.

**The draft head** [HFQ4]: one full-attention hybrid layer that drafts the next token from the main model's residual
stream and the embedding of the last accepted token [MLXQ4]. Each lane of the main model's residual before its final
mixer is normalized and projected by `fc_hidden` (2,560 → 2,560, shared by the lanes) and added to the embedding,
normalized and projected by `fc_embedding`. One layer follows (hyper-connection mixers, indexed attention, a full
block of 512 experts), and its own final mixer feeds the head. transformers drops these weights at load; vLLM and
llama.cpp run them.

**Rotation** [HFQ4]: multimodal rotation with sections [11, 11, 10] at a partial factor of 0.25 of the head dimension
256, θ 10,000,000. That is 32 frequencies over the first 64 dimensions, rotated by halves; frequency j turns with the
height position when j mod 3 = 1, the width position when j mod 3 = 2, and the time position otherwise. For text the
three positions are equal, and it is plain rotate-half rotation of the first 64 dimensions.

**The vision tower**: the Qwen3-VL ViT, reused unchanged by llama.cpp's CLIP path [LCPPQ4]. It has 27 blocks of
1,152 with LayerNorm and biases, a patch convolution over 2 frames × 16 × 16 pixels, a learned position table of
2,304, two-axis rotation in attention, and a merger that folds 2 × 2 patches into one 2,560-wide token; it has no
deepstack taps (`deepstack_visual_indexes` is empty).

- [ ] Every component above is a recognized op or a refusal by name in the one IR, with an evaluator arm equal to its
  reference implementation on the tiny checkpoint (QWEN4-1 to QWEN4-11).

### 7.3 Budgets

Each parameter figure is a sum over the checkpoint's tensor table (its shards' headers at revision de4b8e4d, 1,658
tensors, 180.0B parameters with the n-gram table, the draft block and the vision tower), held by
`qwen38_flash_next_facts_lock`; the byte figures follow from the configuration.

| Item | Parameters or bytes | How |
|---|---|---|
| A gated-delta layer | 57.96M | q, k 2,048 each, v and gate 6,144 each, 96 gate scalars from 2,560; output 6,144 → 2,560; conv 10,240 × 4 |
| An attention layer | 51.45M | gated q 12,288, k and v 512 each from 2,560; output 6,144 → 2,560; the indexer's projections |
| Experts stored a layer | 2,521.5M | 512 × 3 × 2,560 × 640, plus the shared expert |
| Experts active a token a layer | 54.07M | 10 routed and 1 shared |
| Hyper-connections a layer | 13.21M | two mixers of 3.28M + 3.28M + 0.04M, each with its group norm of 10,240 |
| Head; embedding | 635.7M each | 248,320 × 2,560, untied |
| Stored, main stack | about 125.7B | the above over 48 layers |
| Active a token | about 6.67B | gated delta 2.09B, attention 0.62B, experts 2.60B, routers 0.06B, hyper-connections 0.63B, head 0.64B, the n-gram projections 0.03B, the final mixer 0.01B |
| Weight bytes a token | 13.3 GB BF16; about 6.7 GB FP8; about 7.1 GB at 8.5 bits | active parameters × bits |
| KV cache a token | 24.75 KiB | 12 layers × 2 heads × 256 × K and V × 2 bytes, plus the pooled indexer keys (128 × 2 bytes ÷ 4 a layer) |
| KV cache at 262,144 positions | 6.19 GiB a sequence | |
| Indexer scan a decode token at 262,144 | 192 MiB | 65,536 pooled keys × 128 × 2 bytes × 12 layers |
| Selected KV a decode token | at most 48 MiB | 2,051 tokens × 24 KiB |
| Gated-delta state | 108 MiB a sequence in float32 | 36 layers × 48 heads × 128 × 128 × 4 bytes |
| n-gram gather | 5 KiB a token | 16 rows × 160 × 2 bytes, from a 97.7 GiB table |
| Experts touched a step | 512 · (1 − (502/512)^B): 10 at B = 1, 75 at 8, 240 at 32, 367 at 64, 471 at 128 | uniform routing; adversarial routing in EXAM-6 |

- [ ] Every row is recomputed by the deployment lock from the checkpoint's tensor table, not typed (QWEN4-12).

### 7.4 The spatial schedule

- **Hyper-connections** are one row section and one epilogue. The pre-mix (norm over 10,240, the two skinny products,
  the gate, the lane mean, the injection weights) is a single section whose products read D, U and J once a step for the
  whole batch. The injection `h + out ⊗ inj` rides the epilogue of the sublayer's last product. The 10,240-wide residual
  stays in SRAM across the layer (QWEN4-2).
- **The n-gram embedding** needs no matrix product on its table. The ids depend only on token ids, so they are known
  when the previous step's arg-max lands; the gather is issued then and overlaps the embedding and the first layer,
  ahead of the second layer that adds it. The hash needs 64-bit products on a chip whose integer type is 32 bits. Two
  exact lowerings compete:
  - limb arithmetic on 32-bit lanes (KERNEL-6);
  - per-token product tables `P_k[t] = t · m_k` (248,320 × 3 × 8 bytes, about 6 MB a layer) gathered and combined by
    32-bit exclusive-or, with the modulus taken by limbs.
  Both are proven equal to the 64-bit definition (QWEN4-3). The table lives in DDR, sharded by hash head, so 16 heads
  map onto 16 sockets (QWEN4-4).
- **The indexer** keeps pooled keys as a carried table (one tile row a 32 blocks); the score is a relu-sum fold; the
  selection is a top-k of 512 among up to 65,536 by threshold select (KERNEL-1); the attention gathers the selected
  blocks through an indirect tile and runs the flash fold over at most 2,051 positions (QWEN4-6).
- **Experts**: 512 experts of 640 over 16 sockets is 32 a socket; at batch 1 a step touches 10. Broadcast-and-filter
  sends x to every socket and folds the partials in the next all-reduce; all-to-all sends x only where its experts live.
  The time model chooses per batch (MULTI-2, QWEN4-7).
- **Gated delta** reuses the one-pass step; the sigmoid gate and readout scale are parameters of the variant
  (QWEN4-8).
- **The draft head** runs on chip after the step; verification is a block of k + 1 rows through the stacked-row path
  prefill already uses; recurrent state is checkpointed at each draft position so a rejection restores it (QWEN4-10).

### 7.5 Edge cases

- [ ] A prompt shorter than the n-gram order: history padded with end-of-sequence, as the reference pads (QWEN4-3).
- [ ] A non-contiguous batch history (a sequence that joins mid-batch): the n-gram history is per sequence (QWEN4-5).
- [ ] Exactly 2,051 cached tokens, then 2,052: the dense-exact boundary of the indexer (QWEN4-6).
- [ ] Ties between pooled-block scores at the 512th place: the selection agrees with the reference's tie order, or the
  Jaccard bound is stated and held (QWEN4-6).
- [ ] All tokens routed to one expert, and to experts on one socket (EXAM-6, QWEN4-7).
- [ ] A hash multiplier near 2^45 times the largest token id: the product near 2^63 is exact (QWEN4-3).
- [ ] Context at 262,144 and, with YaRN, at 1,048,576 positions (QWEN4-6, MEM-3).
- [ ] An image in the prompt: multimodal rotation with distinct (t, h, w) positions (QWEN4-9, QWEN4-11).
- [ ] A rejected draft after a gated-delta update: state restored exactly (QWEN4-10).

### 7.6 The day-zero drill

The day a Qwen 4 checkpoint is published, a script runs, and a clock records each step:
1. Fetch the configuration and tensor table; diff them against `qwen4_exp`'s known keys and shapes (INTAKE-9).
2. Build its tiny random twin from the configuration and record goldens from the reference implementations
   (INTAKE-4).
3. Read the real checkpoint (INTAKE-1), author it, and run the tiny twin on the VM against the goldens.
4. Emit the RDU bundle for decode, batch, prefill and the parallel plan the time model picks; certify it; run the tiny
   bundle against the VM.
5. Record the modeled cells and the bring-up clock (INTAKE-7, SCORE-3).

- [ ] The drill runs end to end on Qwen3.8-Flash-Next, timed, before any Qwen 4 release (QWEN4-13).
- [ ] The drill's new-key path runs on a configuration with one invented key, and refuses it by name (QWEN4-13).

### 7.7 Reading notes

Sections 7.1 to 7.3 were read against transformers' modeling and configuration files at 5c65f7c, vLLM's `qwen4_exp`
tree, llama.cpp's graph and converter, and the checkpoint's own tensor table and integer tensors. The files, by letter:

| Letter | File |
|---|---|
| T | `external/papers/tf-modeling_qwen4_exp.py` |
| C | `external/papers/tf-configuration_qwen4_exp.py` |
| V | `external/competitors/vllm/vllm/models/qwen4_exp` |
| L | `external/competitors/llama.cpp/src/models/qwen4exp.cpp` |
| K | `external/competitors/llama.cpp/conversion/qwen4exp.py` |

Where each formula lives:

| Component | transformers | vLLM | llama.cpp |
|---|---|---|---|
| Lane norm, `1 + w` over each 2,560 | T 143–166 | V `common/hyperconnection.py` 55–90 | the converter adds 1 to every zero-centred gamma, K 162–166 |
| Mixer and injection | T 995–1024; used at T 1282–1303 | V `common/hyperconnection.py` 203–242 | L 311–397 |
| Four lanes from the embedding; the final mixer | T 1474, 1487 | V `nvidia/model.py` 507, 580 | L 441–446, 511–519 |
| Hash multipliers, primes, offsets | T 1026–1105 | V `nvidia/ngram_embedding.py` 48–138 | read from the checkpoint's int64 tensors, K 57–69 |
| The segment reset and the hash | T 1107–1168 | V `nvidia/ops/ple.py` 78–106 | L 1192–1291, on the host |
| The n-gram gate and dilated convolution | T 1177–1249 | V `nvidia/ops/ple.py` 186–688 | L 1369–1457 |
| The layer that carries it (one-indexed id) | T 1262; C 239–255 | V `nvidia/model.py` 195–206 | K 94–98 |
| Indexer: pooling, score, selection, tail | T 665–794 | V `nvidia/ops/qsa_pre_indexer.py` 266–330, `nvidia/ops/qsa_indexer.py` 476–505 | L 736–867 |
| Attention: per-head norms, gate | T 811–894 | V `nvidia/qsa.py` 318–351 | L 928–1013 |
| Gated delta: scale, head map, decay, gate | T 391–453, 456–610 | `external/competitors/vllm/vllm/model_executor/layers/mamba/gdn/qwen_gdn_linear_attn.py` | L 634–645, 1015–1140 |
| Router and shared expert | T 913–993 | V `nvidia/model.py` 160–172 over Qwen3-Next's block | L 1142–1190 |
| Rotation | T 74–140, 620–660 | V `nvidia/qsa.py` 345–360 | L 975–986 |
| Draft head | absent: T 1316 drops `mtp.*` | V `nvidia/mtp.py` | K 30–36, 145–151 |
| Vision tower | T 1685–2009 | V `nvidia/model.py` 938, the Qwen3-VL tower | K 224–227, the Qwen3-VL tower unchanged |

Where the implementations differ, and what this book follows:
- **The end-of-sequence token, given as a list.** transformers takes the first (T 1086, 1469), llama.cpp's converter the last
  (K 121–127), and vLLM refuses a list (V `nvidia/ngram_embedding.py` 167). The checkpoint names a single token, so all
  three agree on it; `model::ngram_hash` takes the first, as transformers does.
- **Image positions in the n-gram history.** transformers hashes the image placeholder's id from the input ids;
  llama.cpp, whose image batch carries no ids, substitutes `image_token_id` (L 1222–1232) to the same effect.
- **Tie order at the 512th block.** `torch.topk` (T 749), vLLM's persistent and cooperative top-k kernels, and
  `ggml_top_k`, unordered (L 809–811): no two promise the same order, and none is the reference's contract. Below 2,052
  visible tokens no tie matters (section 7.5).
- **The draft's selection.** vLLM's draft head selects blocks on its first draft step and reuses that selection on
  the later ones (V `nvidia/mtp.py` 260–263, `nvidia/indexer_qsa.py` 127–129); llama.cpp's draft selects at every step
  (L 593, 942), and transformers has no draft head. Only drafts differ: the main model's verifying pass selects afresh
  in both.
- **Pooled keys.** transformers takes the block mean in float32 and rounds it to bfloat16 before the norm (T 735); vLLM's
  fused kernel rounds at the same point to match it (V `nvidia/ops/qsa_pre_indexer.py` 266).
- **Configuration defaults the checkpoint leaves to code.** `seed` is absent and defaults to 1234 (C 155), the value the
  checkpoint's stored multipliers confirm; `norm_topk_prob` is absent and defaults to renormalizing (C 162);
  `output_gate_type` defaults to the activation, SiLU, in transformers (C 163, T 491) and to sigmoid in vLLM
  (V `config.py` 50), and the checkpoint states sigmoid; `split_ngram_parts` defaults to 512 (C 156) and the checkpoint
  states its 128 shards; `layer_types` says `full_attention` where transformers runs `indexed_attention` (C 174–184);
  vLLM turns the attention's output gate on whatever the configuration says (V `nvidia/qsa.py` 324–326).
- **Keys transformers does not read.** `mamba_ssm_dtype` (transformers keeps the recurrent state in float32 regardless,
  T 410–413), `partial_rotary_factor` at the top level (it reads the copy inside `rope_parameters`, T 106–110),
  `full_attention_interval` once `layer_types` is given, and the draft head's `mtp` keys.
- **The installed transformers 5.19.0** differs from the pinned file in its cache interface, in where it keeps the
  indexer's position history, and in two paths that compute the same values (the logits' slicing, an autocast guard
  around the rotation); the arithmetic is the same, and `scripts/accel/qwen4exp-oracle.py` drives both interfaces.

Every one of the checkpoint's 1,658 tensors belongs to a family that one component of section 7.2 owns
(`qwen38_flash_next_facts_lock::every_tensor_belongs_to_a_component_section_7_names`).

---

## 8. The compiler we must have

What the spatial pipeline is today, measured on 2026-10-29, and the board families that take it where section 2's bars
are. The pipeline is the spatial IR (`crates/logicaffeine_compile/src/spatial/ir.rs`) and its fixed pass order (`PASS_ORDER` in
`crates/logicaffeine_compile/src/spatial/pipeline.rs`: generic fusion, tiling, placement, reader fusion, buffer sizing, handshake
synthesis, persistence, verification), then composition, phase packing, sharding and residency.

**What holds today.**
- The whole Qwen3.8-27B decode step lowers as a deployment: an embedding program, one program for each of two layer
  kinds, a head with a blocked arg-max over 248,320 entries, and a manifest; every certificate rechecks; the tiny step
  equals the VM from the files on disk (`crates/logicaffeine_tests/tests/emit_rdu_e2e.rs`, `crates/logicaffeine_tests/tests/qwen38_decode_bundle_lock.rs`).
- Batched decode with every weight read once for the batch, prefill chunks with block-causal flash attention and the
  chunked gated-delta form, tensor parallelism over 8 chips, and a serving plan (`crates/logicaffeine_tests/tests/qwen38_batched_decode_lock.rs`,
  `crates/logicaffeine_tests/tests/qwen38_prefill_lock.rs`, `crates/logicaffeine_tests/tests/qwen38_tensor_parallel_lock.rs`).
- Q8_0 weights folded in place, the flash fold, data-dependent trip counts, residency across SRAM, HBM and DDR
  (`crates/logicaffeine_tests/tests/spatial_q8_weights_lock.rs`, `crates/logicaffeine_tests/tests/spatial_fold_lock.rs`, `crates/logicaffeine_tests/tests/spatial_trip_count_lock.rs`,
  `crates/logicaffeine_tests/tests/spatial_residency_lock.rs`).
- Compile time linear in sections: the TP8 × batch-22 bundle compiles in 1:16 (`crates/logicaffeine_tests/tests/spatial_compile_scaling_timing_lock.rs`).
- Ten Plasticine segments of the tiny step pass Tungsten's ideal network; six pass the routed network
  (`benchmarks/silicon/qwen38_fabric_segments.json`).

**Where it stops, family by family.**
- **Chip truth and time** (CHIP, TIME): no time model, a Tensix-shaped unit model, unit slips. Nothing on the board can
  rank its own plans until TIME-1 and TIME-2 land, so they sit at the head of the critical path.
- **Schedule** (SCHED): a barrier at every phase, one layer-kind program at a time, no prefetch, no multi-token loop.
  SambaNova's own numbers say this costs 30% of a token's time on SN40L-8 before KernelLooping [KL24].
- **Multi-chip** (MULTI): tensor parallelism only, a centralized fold. No expert or pipeline parallelism.
- **Numerics and formats** (NUM): bf16 throughout gives about 1.1% relative logit drift on the SN50 target, against
  0.3% on Wormhole with the same source; no fp8 or 4-bit weights.
- **Memory** (MEM): residency by density; no DDR roof, no model switching, no KV tiering.
- **Intake and the author** (INTAKE, AUTHOR): GGUF only, seven arms, several refusals the reference already computes.
- **Architectures and kernels** (ARCH, KERNEL): every feature in section 6.2's RDU column that is not "lowered".
- **Serving on the RDU** (SERVE-RDU): no speculative decoding, sampling, prefill hand-off or ragged batches.
- **The vendor surface** (ACCESS): a stub SambaFlow bridge and a template compile script.
- **Plasticine** (PLAST): the tiny configuration only, in fp32, on the chain's own mesh, with routed failures.

- [ ] Every bullet above names its family's closers on the board, and none is left without one (BOOK-1).

---

## 9. The hill-climb

The board is finite; the climb is not. Once the first tickets land, the compiler improves by a loop that reads the
exam, the ledgers and its own time model, and writes back measured, proven changes. Every turn of the loop leaves a
receipt, and no turn may make a cell worse.

### 9.1 The loop

1. **Score.** The exam runner computes every cell (section 2.3) and the aggregate objective (CLIMB-1).
2. **Attribute.** The time model splits each cell's step into causes: HBM and DDR stalls, barriers, idle compute and
   memory units, routing and virtual-channel stalls, collectives, host round trips (TIME-6).
3. **Mine the gap.** The largest losses across cells become hypotheses, each naming the cell, the cause and the ticket
   that would close it (CLIMB-2).
4. **Check the ledger.** The experiment ledger is read first, so no change already measured and rejected is tried again
   without a new reason (CLIMB-3).
5. **Experiment** in a private copy of the tree: the change, its RED test, its measurement.
6. **Record** the result as an `EXPERIMENT SAMBA` line, kept, rejected or of no effect, with its delta and log (CLIMB-3).
7. **Land** a kept change through its ticket's verify and the ratchet (CLIMB-8).
8. **Recalibrate.** Every new simulated or measured row refits the time model; a refit that moves a prediction past its
   bound turns the calibration lock red until the model is fixed (CLIMB-5).
9. **Search again.** The compiler's own choices (tiles, section cuts, parallelization factors, residency, parallel
   plans, fusion) are searched against the refit model (CLIMB-4).
10. **Grow the exam.** Fuzzed configurations, shapes and routing distributions add tests; each failure becomes a frozen
    RED with its ticket (CLIMB-7).

Mining (section 11, MINE) is one engine inside the loop: rewrite candidates are harvested from every graph the zoo
produces, proven or refuted, measured, and landed as fusion rules (CLIMB-6).

### 9.2 What the loop keeps

| Ledger | Holds | Written by |
|---|---|---|
| The cell ledger, benchmarks/silicon/samba_cells.json (new) | every cell, ours and published, with provenance | SCORE-1, SCORE-2, SCORE-3 |
| The climb history, benchmarks/silicon/samba_climb.json (new) | the objective and every cell's best, by commit | CLIMB-1, CLIMB-8 |
| The experiment ledger, benchmarks/silicon/samba_experiments.json (new) | every hypothesis, change, delta, verdict and log | CLIMB-3 |
| The mined-rule ledger (new) | every mined rewrite, its proof, its measured gain, its landing commit | MINE-7 |
| The calibration ledger | every simulated or measured row against the time model's prediction | TIME-2, CLIMB-5 |

### 9.3 What may never happen

- [ ] No cell's best score regresses: `samba_climb_lock` names the commit and the cell when one does (CLIMB-8).
- [ ] Every landed change has a measurement or a proof beside it (CLIMB-6).
- [ ] No rejected experiment is retried without a new hypothesis: the gap miner reads the ledger first (CLIMB-2,
  CLIMB-3).
- [ ] A default changes only to a choice that dominates the old one on every measured axis; otherwise both stay as toggles
  (`crates/logicaffeine_tests/tests/dominance_toggle_lock.rs`, CLIMB-4).
- [ ] The time model is never trusted past its calibrated bound (CLIMB-5).

---

## 10. The exam catalog

Every test an RDU compiler team could set, written down before they set it. Each family is one EXAM ticket that writes its
suite as handed REDs (section 1.1); each test names the ticket whose work turns it green. A test is ticked when its
lock passes on the tree, with the log beside it. CLIMB-7 grows this catalog from fuzzing and from every failure.

### 10.1 E1: Cold bring-up of unseen configurations

A compiler team that wants to know whether a compiler brings up *any* model sends one it was not built for: odd widths, odd head counts, a new architecture string, a configuration key nobody has seen.
The rows that hold are in `samba_exam_bringup_lock`; the rows still open are handed REDs in `samba_exam_bringup_reds_lock`,
written by EXAM-1, and a ticket that lands one moves its test, unchanged, into the lock. Both run a row the same way:
the checkpoint on disk, `largo emit rdu --chip sn50 --model <checkpoint>`, the bundle read back off disk, one decode step
in the spatial evaluator with every tensor filled from the checkpoint as a host fills it (`model::bind`), the token held
to the f64 reference's.

- [x] **E1.1** A Llama-family checkpoint read from its Hugging Face directory alone, with no GGUF anywhere: its first token equals the reference's. Lock: `samba_exam_bringup_lock::a_hugging_face_directory_brings_up_with_no_gguf`. Ticket: INTAKE-6 (receipt: `logs/samba/exam1-green-3.log`).
- [x] **E1.2** Hidden widths of 2,880 (gpt-oss) and 7,168 (DeepSeek-V3): widths that are not powers of two tile with no remainder lost. Lock: `samba_exam_bringup_lock::hidden_widths_of_2880_and_7168_bring_up`. Ticket: INTAKE-6, the configuration from INTAKE-5's generator (receipt: `logs/samba/exam1-green-3.log`).
- [x] **E1.3** Head dimensions 64, 80, 96, 112, 128 and 256, and latent attention's 192 = 128 + 64 split. Lock: `samba_exam_bringup_lock::every_published_head_dim_brings_up`. Ticket: INTAKE-6, CHIP-2, ARCH-ROPE, the configuration from INTAKE-5's generator. Today 64, 96 and 128 hold; 80 and 112 need 10 and 14 accumulator tiles to write the KV cache where the unit model has 8 (CHIP-2), 256 streams a rotary table whose rows are not whole tiles (ARCH-ROPE), and latent attention is refused (INTAKE-6).
- [x] **E1.4** Vocabularies of 151,936, 163,840, 201,088 and 248,320: the head and its arg-max over each, padded and unpadded (GPT-2's 50,257, which no tile divides). Lock: `samba_exam_bringup_lock::every_published_vocabulary_brings_up`. Ticket: INTAKE-6, the configuration from INTAKE-5's generator (receipt: `logs/samba/exam1-green-2.log`).
- [x] **E1.5** One KV head (multi-query) and as many KV heads as query heads. Lock: `samba_exam_bringup_lock::one_kv_head_and_all_kv_heads_bring_up`. Ticket: INTAKE-6, the configuration from INTAKE-5's generator (receipt: `logs/samba/exam1-green-2.log`).
- [ ] **E1.6** Query-head counts no tensor-parallel width of 16 divides (24, 40, 56). Lock: `samba_exam_bringup_reds_lock::head_counts_no_width_divides_bring_up_at_16_sockets`. Ticket: MULTI-4.
- [ ] **E1.7** A prime layer count (61, as DeepSeek-V3) and a single layer. Lock: `samba_exam_bringup_reds_lock::prime_and_single_layer_counts_bring_up`. Ticket: INTAKE-6, NUM-1, the configuration from INTAKE-5's generator. Today the single (Llama) layer brings up, and the 61-layer hybrid lowers and runs but emits the wrong token: the bf16 datapath's logits drift from the f64 reference's by up to 1.6% of the largest logit at 3 layers, 3.9% at 13, 6.3% at 25 and 17.8% at 61, past the 0.32 margin (6.6%) between this checkpoint's top two tokens (`logs/samba/diag-2.log`). The drift is NUM-1's: fp32 islands in the bf16 datapath.
- [ ] **E1.8** Tied and untied embeddings, QKV biases and an output bias. Lock: `samba_exam_bringup_reds_lock::tied_untied_and_biased_checkpoints_bring_up`. Ticket: INTAKE-6, the configuration from INTAKE-5's generator. Today the untied head and gpt-oss's biased projections bring up token for token; the tied head's reference leads its second token (44) by 0.014, an eighth of the bf16 bound's 0.106 (`logs/samba/e1-8-lead.log`), and the bundle emits 44 on the bf16 datapath and with the SIMD path's fp32 witness alike (`logs/samba/e1-8-simd-fp32.log`): a near-tie the matrix path's bf16 inputs reorder, held within the bound by `rdu_bringup_lock`. An exact token on a lead under the datapath's bound is Tristen's ruling (the row's criterion, or an fp32 matrix path).
- [x] **E1.9** An architecture string the compiler has never seen: refused within a second, every missing piece named. Lock: `samba_exam_bringup_lock::an_unknown_architecture_is_refused_with_its_missing_pieces`. Ticket: INTAKE-9.
- [x] **E1.10** A configuration key the compiler does not understand, on a known architecture: refused by name, never ignored. Lock: `model_hf_intake_lock::an_unknown_key_is_refused_by_name`. Ticket: INTAKE-1 (receipt: `logs/samba/green-2.log`).
- [ ] **E1.11** The bring-up clock, from an unseen checkpoint to a golden-matching first token from the RDU bundle, recorded for every Tier A model. Lock: `samba_exam_bringup_reds_lock::the_bring_up_clock_is_recorded_for_every_tier_a_model`. Ticket: INTAKE-7.
- [ ] **E1.12** An exported PyTorch graph of a model no spec arm describes. Lock: `samba_exam_bringup_reds_lock::an_exported_graph_brings_up_without_a_spec_arm`. Ticket: INTAKE-8.
- [ ] **E1.13** A Qwen3-Next checkpoint as its converter writes it (value heads grouped by key head, each key head's betas and alphas one projection, a gated shared expert in every block, a draft block after the last), from its Hugging Face directory and from its converted GGUF: each brings up within the bf16 datapath's bound. Lock: `samba_exam_bringup_reds_lock::a_qwen3next_checkpoint_brings_up_from_its_directory_and_its_gguf`. Ticket: INTAKE-2. Today both lower whole and run, the same bundle from either source, and a logit lands 0.106 from the reference's against the datapath's 0.071 (`logs/samba/green-gd-subtile-4.log`). The lowering is exact: on the SN50's fp32 twin the same model's decode is the reference's to fp32's rounding (`spatial_mixed_precision_lock::qwen3next_decodes_on_the_fp32_twin_as_the_reference_computes`, `logs/samba/q3next-fp32twin-2.log`), so the drift is the bf16 datapath's and closes with NUM-1.
- [x] **E1.14** Gated DeltaNet heads that are not whole tiles (16, 8, 48 and 80), their memory past one tile, in a decode step and a block. Lock: `tensor_gdn_row_blocks_lock::a_gated_delta_whose_heads_are_not_whole_tiles_lowers_on_sn50_and_computes_the_vms_values`. Ticket: ARCH-GDN-VARIANTS (receipt: `logs/samba/green-gd-subtile-4.log`).
- [x] **E1.15** A fused query-key-value projection (Phi-3's) and one with its bias (GLM-4's), both under Llama's interleaved rotation, and values half and twice as wide as the keys. Lock: `samba_exam_bringup_lock::a_fused_projection_and_values_unlike_the_keys_bring_up`. Ticket: AUTHOR-1 (receipt: `logs/samba/deploy-1.log`).

### 10.2 E2: Correctness against the references

A bring-up that is fast and wrong loses. The references are the f32 reference forward and the pinned reference implementations; the bounds are the numerics contract's.
The suites are `samba_exam_correctness_lock` (the rows that hold) and `samba_exam_correctness_reds_lock` (the rows
still open), written by EXAM-2; the rows that need the reference implementations' goldens (INTAKE-4) are written into
`samba_exam_quality_lock`, the row that needs a prompt runner over a bundle (ACCESS-4) into `samba_exam_runner_lock`, and
the long-context and cross-chip numerics rows into `samba_exam_numerics_lock`, each when what it needs exists.

- [ ] **E2.1** Top-1 agreement with the f32 reference over the golden prompts, at least the reference implementation's own at the same bits. Lock: `samba_exam_quality_lock::top1_agreement_meets_the_reference_implementations`. Ticket: NUM-7.
- [ ] **E2.2** KL divergence to the f32 reference within the contract's bound on every Tier A model. Lock: `samba_exam_quality_lock::kl_to_the_reference_is_within_the_contract`. Ticket: NUM-7.
- [ ] **E2.3** Perplexity on a fixed text within the contract's bound. Lock: `samba_exam_quality_lock::perplexity_on_a_fixed_text_is_within_the_contract`. Ticket: NUM-7.
- [ ] **E2.4** Batch invariance: a sequence's tokens are bit-identical alone and beside 63 others. Lock: `samba_exam_correctness_reds_lock::a_sequence_decodes_alike_alone_and_in_a_batch_of_64`. Ticket: NUM-8.
- [x] **E2.5** Determinism: two runs of one bundle give bit-identical logits. Lock: `samba_exam_correctness_lock::two_runs_of_one_bundle_are_bit_identical`. Ticket: NUM-8.
- [ ] **E2.6** An empty prompt and a prompt of one beginning-of-sequence token. Lock: `samba_exam_runner_lock::empty_and_bos_only_prompts_decode_as_the_reference`. Ticket: INTAKE-4.
- [ ] **E2.7** A prompt that fills the context to its last position, and one past it refused by name. Lock: `samba_exam_correctness_reds_lock::the_last_position_decodes_and_the_next_is_refused`. Ticket: MEM-3.
- [ ] **E2.8** 4,096 repetitions of one token. Lock: `samba_exam_numerics_lock::a_long_repetition_decodes_as_the_reference`. Ticket: NUM-1.
- [x] **E2.9** A residual channel at 1e4 (the outlier channels large models carry) through bf16 sums. Lock: `samba_exam_correctness_lock::an_outlier_channel_survives_bf16_sums`. Ticket: NUM-1.
- [x] **E2.10** A NaN or an infinity in an input, propagated as the reference propagates it and never masked. Lock: `samba_exam_correctness_lock::nan_and_infinity_propagate_as_the_reference_propagates_them`. Ticket: NUM-7.
- [ ] **E2.11** fp32 softmax and norm islands bring the SN50 target's relative logit drift to the Wormhole target's for the same source. Lock: `samba_exam_numerics_lock::fp32_islands_bring_the_sn50_drift_to_wormholes`. Ticket: NUM-1.

### 10.3 E3: The decode matrix

Their headline number is decode: steady-state time per token, its share of the roofline, its bandwidth utilization, its synchronization count.
The suite is `samba_exam_decode_lock`, written by EXAM-3.

- [ ] **E3.1** D1 on every Tier A model, modeled, with provenance. Lock: `samba_exam_decode_lock::d1_is_modeled_for_every_tier_a_model`. Ticket: TIME-3.
- [ ] **E3.2** D8, D16 and D64 with every weight read once a step for the whole batch. Lock: `samba_exam_decode_lock::batched_decode_reads_every_weight_once_a_step`. Ticket: SCHED-4.
- [ ] **E3.3** L32K, L64K and L262K on Qwen3.8-Flash-Next and Qwen3.8-27B. Lock: `samba_exam_decode_lock::long_context_decode_runs_to_262144`. Ticket: MEM-3.
- [ ] **E3.4** The bandwidth utilization of every D cell beside the published figure. Lock: `samba_exam_decode_lock::every_d_cell_reports_its_bandwidth_utilization`. Ticket: TIME-6.
- [ ] **E3.5** At most 4 synchronization boundaries a token on a looped deployment. Lock: `samba_exam_decode_lock::a_looped_deployment_synchronizes_at_most_four_times_a_token`. Ticket: SCHED-3.
- [ ] **E3.6** Decode at 1,048,576 positions with YaRN on Qwen3.8-Flash-Next. Lock: `samba_exam_decode_lock::decode_at_one_million_positions_runs`. Ticket: QWEN4-6.
- [ ] **E3.7** Steady state over 10 prompts that generate 64 to 2,048 tokens, their protocol. Lock: `samba_exam_decode_lock::the_decode_protocol_matches_theirs`. Ticket: SCORE-3.

### 10.4 E4: Prefill and the first token

Prefill is compute-bound where decode is memory-bound; an evaluation that scores serving scores both.
The suite is `samba_exam_prefill_lock`, written by EXAM-4.

- [ ] **E4.1** TTFT-8K on every Tier A model. Lock: `samba_exam_prefill_lock::ttft_8k_is_modeled_for_every_tier_a_model`. Ticket: TIME-3.
- [ ] **E4.2** A 262,144-token prompt in chunks leaves the cache a run of 262,144 decode steps leaves. Lock: `samba_exam_prefill_lock::a_chunked_long_prompt_leaves_the_decode_cache`. Ticket: MEM-3.
- [ ] **E4.3** Prefill on a GPU and decode on the RDU: the handed cache decodes the tokens a cache built on the RDU decodes. Lock: `samba_exam_prefill_lock::a_gpu_prefilled_cache_decodes_on_the_rdu`. Ticket: SERVE-RDU-3.
- [ ] **E4.4** A chunk whose length is not a multiple of the tile height. Lock: `samba_exam_prefill_lock::a_ragged_chunk_prefills_as_the_reference`. Ticket: SCHED-2.
- [ ] **E4.5** A prompt sharing a prefix with a cached one prefills only its suffix. Lock: `samba_exam_prefill_lock::a_shared_prefix_prefills_only_its_suffix`. Ticket: MEM-5.

### 10.5 E5: Socket scaling

SambaNova reports scaling from 8 to 16 sockets and from 64 to 256; a compiler that does not scale across sockets loses the large models.
The suite is `samba_exam_scaling_lock`, written by EXAM-5.

- [ ] **E5.1** Llama-3.1-8B at 1, 8 and 16 sockets: the modeled ratio within its bound of the published 584 and 1,140 tok/s. Lock: `samba_exam_scaling_lock::llama_8b_scales_as_published_from_8_to_16`. Ticket: MULTI-1.
- [ ] **E5.2** DeepSeek-R1 at 64, 128 and 256 sockets, modeled beside the published bandwidth utilization. Lock: `samba_exam_scaling_lock::deepseek_r1_scales_to_256_sockets`. Ticket: MULTI-5.
- [ ] **E5.3** A tensor-parallel all-reduce hidden behind the next product in the modeled step. Lock: `samba_exam_scaling_lock::an_all_reduce_hides_behind_the_next_product`. Ticket: MULTI-1.
- [ ] **E5.4** Pipeline parallelism across two nodes. Lock: `samba_exam_scaling_lock::two_nodes_pipeline_a_model`. Ticket: MULTI-3.
- [ ] **E5.5** A collective missing one chip's operand is refused by the certificate. Lock: `samba_exam_scaling_lock::a_collective_missing_a_chip_is_refused`. Ticket: MULTI-1.
- [ ] **E5.6** A model whose heads admit 8 sockets but not 16 gets a plan that uses all 16. Lock: `samba_exam_scaling_lock::a_model_admitting_8_uses_16_sockets`. Ticket: MULTI-5.

### 10.6 E6: Adversarial expert routing

Routing is data. A team that knows where a mixture-of-experts mapping breaks sends the routing that breaks it.
The suites are `samba_exam_moe_lock` (the rows that hold) and `samba_exam_moe_reds_lock` (the rows still open),
written by EXAM-6; the rows that need expert parallelism across sockets (MULTI-2) are written into
`samba_exam_scaling_lock` and the row that needs experts streamed from memory (MEM-6) into `samba_exam_overflow_lock`,
each when what it needs exists.

- [ ] **E6.1** Every token of a batch of 64 routed to one expert. Lock: `samba_exam_moe_reds_lock::a_batch_routed_to_one_expert_decodes`. Ticket: NUM-1 (the batch decodes each sequence as it decodes alone, within 0.015, on its one hot expert — the reference routed by its own logits lies 5 to 9 away — and the bf16 datapath's error after a prompt reaches 0.22 against a bound of 0.12 on 2 of 8 sequences, `logs/samba/e61-batch-evidence.log`).
- [ ] **E6.2** Every token routed to experts that live on one socket. Lock: `samba_exam_scaling_lock::a_batch_routed_to_one_socket_decodes`. Ticket: MULTI-2.
- [ ] **E6.3** Uniform routing at batch 128 over 512 experts, 471 touched. Lock: `samba_exam_overflow_lock::uniform_routing_over_512_experts_decodes`. Ticket: MEM-6.
- [ ] **E6.4** Router ties at the k-th place broken in the reference's order. Lock: `samba_exam_moe_reds_lock::router_ties_break_in_the_references_order`. Ticket: NUM-1 (the tie is broken in the reference's order — the bundle's logits lie 0.115 from the reference's and 6.7 from the other order's, `logs/samba/e64-tie-order-evidence.log` — and the bf16 datapath's error on this checkpoint is 2.6% of its largest logit, past the 2% the exam allows).
- [x] **E6.5** Group-limited routing where the best experts sit in a group the limit excludes. Lock: `samba_exam_moe_lock::group_limits_exclude_the_best_experts`. Ticket: ARCH-ROUTING (receipt: `logs/samba/e6-classify-1.log`).
- [ ] **E6.6** A router that produces NaN for one token. Lock: `samba_exam_moe_reds_lock::a_nan_router_row_is_handled_as_the_reference`. Ticket: NUM-1 (the reference ranks a NaN score below every number, as the chip does: a NaN router score leaves the decode finite and the reference's within the datapath, and a sequence whose router row is all NaN decodes to NaN alone; a sequence beside it lies 0.114 from its reference after its prompt against a bound of 0.109, `logs/samba/e66-nan-policy-evidence.log`).
- [ ] **E6.7** Both dispatch forms compute alike, and the time model's choice is the faster in simulation. Lock: `samba_exam_scaling_lock::both_dispatch_forms_agree_and_the_faster_is_chosen`. Ticket: MULTI-2.

### 10.7 E7: Speculative decoding

Their papers spend a section on speculative decoding; their 6× claim rests on it.
The suite is `samba_exam_spec_lock`, written by EXAM-7.

- [ ] **E7.1** MTP drafting on Qwen3.8-Flash-Next at k from 1 to 9 emits the tokens greedy decoding emits. Lock: `samba_exam_spec_lock::mtp_drafting_emits_the_greedy_tokens`. Ticket: QWEN4-10.
- [ ] **E7.2** A draft model (Llama-3.2-1B for Llama-3.1-70B) at k = 5 and k = 9. Lock: `samba_exam_spec_lock::a_draft_model_speculates_at_5_and_9`. Ticket: SERVE-RDU-1.
- [ ] **E7.3** A rejection at the first draft after a gated-delta update restores the state bit for bit. Lock: `samba_exam_spec_lock::a_rejection_restores_the_recurrent_state`. Ticket: SERVE-RDU-1.
- [ ] **E7.4** Acceptance rates on AlpacaEval-, HumanEval- and GSM8K-style prompts, recorded beside theirs. Lock: `samba_exam_spec_lock::acceptance_is_recorded_beside_theirs`. Ticket: SERVE-RDU-1.
- [ ] **E7.5** Draft and target schedules concatenated on chip with no host step between them. Lock: `samba_exam_spec_lock::draft_and_target_run_with_no_host_step`. Ticket: SCHED-6.

### 10.8 E8: Compiler quality

A compiler team measures the compiler itself: how long it takes, what it produces, whether it says what it did.
The suite is `samba_exam_compiler_lock`, written by EXAM-8.

- [ ] **E8.1** Compile time for 70B, 405B and 671B models inside the budget. Lock: `samba_exam_compiler_lock::large_models_compile_inside_the_budget`. Ticket: SCHED-8.
- [ ] **E8.2** One program a layer kind: 64 layers compile as their distinct kinds. Lock: `samba_exam_compiler_lock::layers_of_one_kind_compile_once`. Ticket: SCHED-8.
- [ ] **E8.3** One checkpoint compiled twice gives byte-identical bundles. Lock: `samba_exam_compiler_lock::a_checkpoint_compiles_to_identical_bytes`. Ticket: SCHED-8.
- [ ] **E8.4** A new batch size recompiles only what the batch changes. Lock: `samba_exam_compiler_lock::a_new_batch_size_recompiles_only_what_it_changes`. Ticket: SCHED-8.
- [ ] **E8.5** PCU, PMU and SRAM use reported for every section. Lock: `samba_exam_compiler_lock::every_section_reports_its_unit_use`. Ticket: TIME-6.
- [ ] **E8.6** Every refusal names its statement and reason; no partial program exits 0. Lock: `samba_exam_compiler_lock::no_partial_program_exits_zero`. Ticket: SCHED-10.
- [ ] **E8.7** The fusion report lists every class fired and declined in every section. Lock: `samba_exam_compiler_lock::the_fusion_report_covers_every_section`. Ticket: SCHED-10.
- [ ] **E8.8** Every program's placement certificate passes the independent checker. Lock: `samba_exam_compiler_lock::every_placement_certificate_rechecks`. Ticket: ACCESS-3.

### 10.9 E9: Programs larger than a chip

Plasticine and SARA make a program that does not fit a central question; a test of a Plasticine-principled compiler asks it.
The suite is `samba_exam_overflow_lock`, written by EXAM-9.

- [ ] **E9.1** A single layer whose weights exceed one socket's SRAM splits across virtual units. Lock: `samba_exam_overflow_lock::a_layer_larger_than_sram_splits`. Ticket: SCHED-9.
- [ ] **E9.2** A 248,320-way head whose buffers exceed SRAM keeps the recurrent state on chip. Lock: `samba_exam_overflow_lock::the_head_leaves_room_for_the_state`. Ticket: SCHED-7.
- [ ] **E9.3** An expert table larger than HBM spills its cold experts to DDR. Lock: `samba_exam_overflow_lock::cold_experts_spill_to_ddr`. Ticket: MEM-6.
- [ ] **E9.4** A model larger than one node's HBM deploys across nodes. Lock: `samba_exam_overflow_lock::a_model_larger_than_a_node_deploys`. Ticket: MULTI-3.
- [ ] **E9.5** A 97.7 GiB n-gram table served from DDR. Lock: `samba_exam_overflow_lock::the_ngram_table_is_served_from_ddr`. Ticket: QWEN4-4.

### 10.10 E10: A new op, first run then fused

A team that will send models nobody has built yet tests how fast a new op runs at all, and how fast it runs well.
The suite is `samba_exam_agility_lock`, written by EXAM-10.

- [ ] **E10.1** A novel activation written in LOGOS runs on the RDU as a generic section on its first compile. Lock: `samba_exam_agility_lock::a_new_activation_runs_on_its_first_compile`. Ticket: SCHED-10.
- [ ] **E10.2** A novel attention mask written in LOGOS runs on its first compile. Lock: `samba_exam_agility_lock::a_new_attention_mask_runs_on_its_first_compile`. Ticket: ARCH-WINDOW.
- [ ] **E10.3** A novel recurrence written in LOGOS runs as a sequential section, then chunked. Lock: `samba_exam_agility_lock::a_new_recurrence_runs_then_chunks`. Ticket: KERNEL-3.
- [ ] **E10.4** The time from a written op to its fused, certified form, recorded. Lock: `samba_exam_agility_lock::the_time_to_a_fused_new_op_is_recorded`. Ticket: CLIMB-6.
- [ ] **E10.5** A novel routing function written in LOGOS. Lock: `samba_exam_agility_lock::a_new_routing_function_runs`. Ticket: ARCH-ROUTING.

### 10.11 E11: The kernel micro-exam

The tests a compiler team writes first: one kernel shape at a time, each at the edge of a unit's capacity.
The suite is `samba_exam_kernel_lock`, written by EXAM-11.

- [ ] **E11.1** A 7,168 × 18,432 bf16 GEMM at its modeled share of peak. Lock: `samba_exam_kernel_lock::a_large_gemm_reaches_its_modeled_share_of_peak`. Ticket: CHIP-2.
- [ ] **E11.2** Decode attention at 131,072 positions at its modeled share of HBM bandwidth. Lock: `samba_exam_kernel_lock::long_decode_attention_reaches_its_bandwidth_share`. Ticket: TIME-6.
- [ ] **E11.3** Top-2,048 among 131,072 scores. Lock: `samba_exam_kernel_lock::top_2048_of_131072_selects_as_the_reference`. Ticket: KERNEL-1.
- [ ] **E11.4** Expert dispatch: 256 experts, top 8, batch 64. Lock: `samba_exam_kernel_lock::dispatch_of_256_experts_top_8_at_batch_64`. Ticket: KERNEL-2.
- [ ] **E11.5** LayerNorm over 16,384 with fp32 accumulation. Lock: `samba_exam_kernel_lock::layernorm_over_16384_accumulates_in_fp32`. Ticket: NUM-1.
- [ ] **E11.6** A chain of 200 pointwise ops fused into one section. Lock: `samba_exam_kernel_lock::a_chain_of_200_pointwise_ops_is_one_section`. Ticket: SCHED-10.
- [ ] **E11.7** A graph of 10,000 nodes compiles in time linear in its size. Lock: `samba_exam_kernel_lock::a_graph_of_10000_nodes_compiles_linearly`. Ticket: SCHED-8.
- [ ] **E11.8** One tensor fanned out to 64 consumers. Lock: `samba_exam_kernel_lock::one_tensor_feeds_64_consumers`. Ticket: SCHED-2.
- [ ] **E11.9** Random gather and scatter with repeated indices. Lock: `samba_exam_kernel_lock::gather_and_scatter_with_repeated_indices`. Ticket: KERNEL-4.
- [ ] **E11.10** Ragged sequence lengths in one batch. Lock: `samba_exam_kernel_lock::ragged_lengths_in_one_batch`. Ticket: SERVE-RDU-4.
- [ ] **E11.11** A 248,320-way sampling step: temperature, top-p, top-k, min-p, seeded. Lock: `samba_exam_kernel_lock::sampling_over_248320_is_seeded_and_exact`. Ticket: SERVE-RDU-2.
- [ ] **E11.12** 64-bit hash arithmetic on 32-bit lanes, exact on every input class. Lock: `samba_exam_kernel_lock::hash_arithmetic_is_exact_on_32_bit_lanes`. Ticket: KERNEL-6.

### 10.12 E12: Multimodal input, serving and the Qwen 4 day zero

A contract for model bring-up is a contract for serving the model: its inputs, its API, its switching.
The suite is `samba_exam_serving_lock`, written by EXAM-12.

- [ ] **E12.1** An image prompt through Qwen3.8-Flash-Next's vision tower. Lock: `samba_exam_serving_lock::an_image_prompt_runs_through_the_vision_tower`. Ticket: QWEN4-11.
- [ ] **E12.2** An OpenAI-compatible endpoint over a bundle: streaming, stop strings, log-probabilities. Lock: `samba_exam_serving_lock::the_endpoint_streams_stops_and_reports_logprobs`. Ticket: SERVE-RDU-5.
- [ ] **E12.3** JSON-schema-constrained decoding on chip. Lock: `samba_exam_serving_lock::json_constrained_decoding_runs_on_chip`. Ticket: SERVE-RDU-6.
- [ ] **E12.4** Two models held in DDR and switched between requests. Lock: `samba_exam_serving_lock::two_models_switch_between_requests`. Ticket: MEM-2.
- [ ] **E12.5** An embedding model and a reranker. Lock: `samba_exam_serving_lock::embedding_and_reranking_models_serve`. Ticket: ARCH-POOLING.
- [ ] **E12.6** A speech encoder with cross-attention. Lock: `samba_exam_serving_lock::a_speech_encoder_cross_attends`. Ticket: ARCH-CROSS.
- [ ] **E12.7** The Qwen 4 day-zero drill, timed. Lock: `samba_exam_serving_lock::the_day_zero_drill_runs_timed`. Ticket: QWEN4-13.
### 10.13 E13: The Plasticine kit's workloads

The 2019 Stanford Plasticine kit [PTUT] shows how a Plasticine-principled evaluation is run: PyTorch modules behind
`get_model_info` for its Model Analyzer, and graph and sparse programs traced into its Tungsten estimator. A test set
built on Plasticine's principles is likely to arrive in this shape.
The suite is `samba_exam_kit_lock`, written by EXAM-13.

- [ ] **E13.1** ResNet-50 from a `get_model_info` module at batch 1: every field of the Analyzer's report produced (nodes before and after merging, function nodes, depth, FLOPs, FLOPS utilization, compute units, throughput, stage and model latency, memory bytes and units). Lock: `samba_exam_kit_lock::resnet50_gets_every_field_of_the_report`. Ticket: PLAST-13.
- [ ] **E13.2** SqueezeNet's total FLOPs equal the kit's published 700,981,753 for the same model and input, and its report sits beside the published one. Lock: `samba_exam_kit_lock::squeezenet_flops_equal_the_published_count`. Ticket: PLAST-13.
- [ ] **E13.3** A training step for SqueezeNet (forward, loss, backward): 2,097,762,553 FLOPs as published, mapped onto as many chips as it needs. Lock: `samba_exam_kit_lock::squeezenet_training_maps_on_enough_chips`. Ticket: TRAIN-3.
- [ ] **E13.4** A NetworkX random walk traced by the kit's tracer runs on our rebuilt trace estimator and reports cycles and runtime. Lock: `samba_exam_kit_lock::a_traced_random_walk_runs_on_the_estimator`. Ticket: PLAST-11.
- [ ] **E13.5** The same random walk written in LOGOS emits a trace the estimator accepts, its memory spaces matching the tracer's. Lock: `samba_exam_kit_lock::a_logos_random_walk_emits_the_kits_trace`. Ticket: PLAST-12.
- [ ] **E13.6** CSR and COO sparse matrix-vector products and PageRank written in LOGOS, simulated on Plasticine. Lock: `samba_exam_kit_lock::sparse_and_graph_programs_simulate`. Ticket: PLAST-14.
- [ ] **E13.7** Weight sparsity swept through 0%, 50% and 90%: FLOPs and throughput move as the stated density says. Lock: `samba_exam_kit_lock::weight_sparsity_moves_flops_and_throughput`. Ticket: KERNEL-8.
- [ ] **E13.8** A model too large for one chip mapped onto 2 and 4 chips, as the Analyzer's chip count does. Lock: `samba_exam_kit_lock::a_model_too_large_for_one_chip_maps_onto_four`. Ticket: MULTI-5.
- [ ] **E13.9** A model no spec arm describes, uploaded as a `get_model_info` file with its own loss. Lock: `samba_exam_kit_lock::an_unknown_module_with_its_own_loss_maps`. Ticket: INTAKE-8.
- [ ] **E13.10** The whole network resident and pipelined over samples: throughput one over the stage latency, latency depth times it. Lock: `samba_exam_kit_lock::a_resident_network_pipelines_over_samples`. Ticket: SCHED-11.

---

## 11. The board

### 11.1 Board index

- [x] **BOOK-1** · The book holds together · book · `infrastructure` `stream:R0` · depends on: none
- [x] **STUDY-1** · The bibliography, pinned · study · `study` `stream:R0` · depends on: none
- [ ] **STUDY-2** · Every published technique, mapped · study · `study` `stream:R0` · depends on: STUDY-1
- [x] **STUDY-3** · Qwen3.8-Flash-Next, read end to end · study · `study` `model:qwen4exp` `stream:R0` · depends on: none
- [ ] **STUDY-4** · The rival spatial compilers, read where they publish · study · `study` `stream:R0` · depends on: STUDY-1
- [ ] **STUDY-5** · The Plasticine evaluation kit, read and pinned · study · `study` `plasticine` `stream:R15` · depends on: none
- [ ] **ACCESS-1** · The first-hour intake card · access · `infrastructure` `rdu` `stream:R1` · depends on: STUDY-2
- [ ] **ACCESS-2** · SambaFlow modules for every program, experts included · access · `infrastructure` `rdu` `stream:R1` · depends on: none
- [ ] **ACCESS-3** · Vendor placements read back and certified · access · `infrastructure` `rdu` `stream:R1` · depends on: none
- [ ] **ACCESS-4** · A self-checking artifact drop · access · `infrastructure` `rdu` `stream:R1` · depends on: none
- [ ] **ACCESS-5** · One harness for any cycle simulator · access · `infrastructure` `rdu` `stream:R1` · depends on: TIME-1
- [ ] **ACCESS-6** · A research allocation on SambaNova hardware · access · `infrastructure` `rdu` `stream:R1` · depends on: ACCESS-4
- [ ] **ACCESS-7** · The SDK-day runbook, current · access · `infrastructure` `rdu` `stream:R1` · depends on: ACCESS-1
- [ ] **ACCESS-8** · Unit configurations at Plasticine granularity · access · `infrastructure` `rdu` `plasticine` `stream:R1` · depends on: CHIP-2, CHIP-3
- [ ] **PLAST-1** · Real-width layers through PIR and Tungsten · plasticine · `plasticine` `stream:R15` · depends on: PLAST-2, PLAST-6
- [ ] **PLAST-2** · Every segment passes the routed network · plasticine · `plasticine` `area:comm` `stream:R15` · depends on: none
- [ ] **PLAST-3** · Our placement is the one simulated · plasticine · `plasticine` `stream:R15` · depends on: none
- [ ] **PLAST-4** · Section chains as metapipelines · plasticine · `plasticine` `technique` `stream:R15` · depends on: SCHED-2
- [ ] **PLAST-5** · Multi-PCU GEMMs, and Table 4 reproduced · plasticine · `plasticine` `area:gemm` `stream:R15` · depends on: none
- [ ] **PLAST-6** · bf16 apps · plasticine · `plasticine` `area:quant` `stream:R15` · depends on: none
- [ ] **PLAST-7** · Every segment check can fail · plasticine · `plasticine` `stream:R15` · depends on: none
- [ ] **PLAST-8** · The calibration covers every app · plasticine · `plasticine` `area:time` `stream:R15` · depends on: TIME-1
- [ ] **PLAST-9** · SARA-scale and SN-shaped meshes · plasticine · `plasticine` `stream:R15` · depends on: CHIP-2
- [ ] **PLAST-10** · Data-dependent control on Plasticine · plasticine · `plasticine` `technique` `stream:R15` · depends on: KERNEL-1
- [ ] **PLAST-11** · The kit's trace-driven estimator, rebuilt and checked · plasticine · `plasticine` `stream:R15` · depends on: STUDY-5
- [ ] **PLAST-12** · Traces of LOGOS programs in the kit's format · plasticine · `plasticine` `stream:R15` · depends on: PLAST-11
- [ ] **PLAST-13** · The Model Analyzer's report, from our compiler · plasticine · `plasticine` `area:time` `stream:R15` · depends on: INTAKE-8, TIME-1, SCHED-11
- [ ] **PLAST-14** · Graph and sparse programs on Plasticine · plasticine · `plasticine` `stream:R15` · depends on: PLAST-12
- [ ] **CHIP-1** · Every chip fact in its published unit · chip · `infrastructure` `rdu` `stream:R2` · depends on: STUDY-1
- [ ] **CHIP-2** · A PCU that reaches the published peak · chip · `infrastructure` `rdu` `stream:R2` · depends on: CHIP-1
- [ ] **CHIP-3** · The PMU, the AGCU and the networks · chip · `infrastructure` `rdu` `stream:R2` · depends on: CHIP-1
- [ ] **CHIP-4** · SN40L and SN50 from one fact table · chip · `infrastructure` `rdu` `stream:R2` · depends on: CHIP-1
- [ ] **CHIP-5** · One sizing for the fit, the bundle and the serving plan · chip · `infrastructure` `rdu` `stream:R2` · depends on: CHIP-4
- [ ] **TIME-1** · A time model of a composed program · time · `infrastructure` `rdu` `area:time` `stream:R2` · depends on: CHIP-2, CHIP-3
- [ ] **TIME-2** · The time model calibrated on Tungsten and ttsim · time · `infrastructure` `area:time` `stream:R2` · depends on: TIME-1, PLAST-7
- [ ] **TIME-3** · SN40L and SN50 in the decode pricer · time · `infrastructure` `rdu` `area:time` `stream:R2` · depends on: TIME-1
- [ ] **TIME-4** · A speed claim only from a calibrated row · time · `infrastructure` `area:time` `stream:R2` · depends on: TIME-2
- [ ] **TIME-5** · Predicted against measured, the day hardware answers · time · `infrastructure` `rdu` `area:time` `stream:R2` · depends on: TIME-2, ACCESS-1
- [ ] **TIME-6** · Where every cycle of a step goes · time · `infrastructure` `area:time` `stream:R2` · depends on: TIME-1
- [ ] **INTAKE-1** · Hugging Face checkpoints read in place · intake · `infrastructure` `area:intake` `stream:R3` · depends on: none
- [ ] **INTAKE-2** · Every architecture string a test can send · intake · `infrastructure` `area:intake` `stream:R3` · depends on: INTAKE-1
- [ ] **INTAKE-3** · fp8 blocks, NVFP4, AWQ and GPTQ read · intake · `infrastructure` `area:intake` `area:quant` `stream:R3` · depends on: INTAKE-1
- [ ] **INTAKE-4** · Goldens from the reference implementations · intake · `infrastructure` `area:intake` `stream:R3` · depends on: INTAKE-1
- [ ] **INTAKE-5** · A configuration-space fuzzer · intake · `infrastructure` `area:intake` `stream:R3` · depends on: INTAKE-4
- [ ] **INTAKE-6** · Any architecture through the RDU emitter · intake · `infrastructure` `rdu` `area:intake` `stream:R3` · depends on: INTAKE-2, FUSIONS:ONE-PROGRAM-RETIRE-SECOND-WRITERS
- [ ] **INTAKE-7** · The bring-up clock · intake · `infrastructure` `area:intake` `stream:R3` · depends on: INTAKE-6, SCORE-1
- [ ] **INTAKE-8** · An exported PyTorch graph as a model · intake · `infrastructure` `area:intake` `stream:R3` · depends on: INTAKE-2
- [ ] **INTAKE-9** · An unknown architecture refused with its missing pieces named · intake · `infrastructure` `area:intake` `stream:R3` · depends on: INTAKE-2
- [ ] **AUTHOR-1** · Value width, attention scale, fused projections and routed scale written · author · `primitive` `area:graph` `stream:R3` · depends on: none
- [ ] **AUTHOR-2** · LayerNorm models and plain MLPs written · author · `primitive` `area:norm` `stream:R3` · depends on: none
- [ ] **AUTHOR-3** · The MTP head kept, not dropped · author · `primitive` `stream:R3` · depends on: none
- [ ] **AUTHOR-4** · Latent attention and experts under tensor parallelism · author · `primitive` `area:comm` `stream:R3` · depends on: FUSIONS:PARITY-TENSOR-PARALLEL-IR
- [ ] **AUTHOR-5** · Every activation and norm offset the families use · author · `primitive` `area:norm` `stream:R3` · depends on: FUSIONS:PARITY-ACTIVATION-KINDS
- [ ] **ARCH-WINDOW** · Sliding and chunked-local attention · architecture · `primitive` `rdu` `area:attention` `stream:R4` · depends on: SCHED-1
- [ ] **ARCH-SINKS** · Attention sinks · architecture · `primitive` `rdu` `area:attention` `stream:R4` · depends on: SCHED-1
- [ ] **ARCH-SOFTCAP** · Soft caps on scores and logits · architecture · `primitive` `rdu` `area:attention` `stream:R4` · depends on: SCHED-1, FUSIONS:PARITY-SOFTCAP
- [ ] **ARCH-NOPE** · Layers without rotation and with tuned temperature · architecture · `primitive` `rdu` `area:attention` `stream:R4` · depends on: SCHED-1
- [ ] **ARCH-MLA** · Latent attention, absorbed for decode · architecture · `primitive` `rdu` `area:attention` `stream:R4` · depends on: SCHED-1
- [ ] **ARCH-SPARSE** · Indexed sparse attention: DSA, QSA and NSA · architecture · `primitive` `rdu` `area:attention` `stream:R4` · depends on: KERNEL-1, FUSIONS:PARITY-SPARSE-ATTENTION
- [ ] **ARCH-ROPE** · Every rotation the families use · architecture · `primitive` `rdu` `area:attention` `stream:R4` · depends on: SCHED-1, FUSIONS:PARITY-ROPE-VARIANTS
- [ ] **ARCH-CROSS** · Cross-attention and encoder–decoder models · architecture · `primitive` `rdu` `area:attention` `stream:R4` · depends on: SCHED-1
- [ ] **ARCH-POOLING** · Bidirectional encoders and pooling heads · architecture · `primitive` `rdu` `area:attention` `stream:R4` · depends on: SCHED-1, FUSIONS:SERVE-POOLING
- [ ] **ARCH-VISION** · Vision towers · architecture · `primitive` `rdu` `area:attention` `stream:R4` · depends on: ARCH-CNN, FUSIONS:PARITY-MULTIMODAL-TOWERS
- [ ] **ARCH-ROUTING** · Sigmoid routing with bias correction and group limits · architecture · `primitive` `rdu` `area:moe` `stream:R5` · depends on: SCHED-1, AUTHOR-1, FUSIONS:PARITY-ROUTING-VARIANTS
- [ ] **ARCH-SHARED-GATE** · Gated shared experts · architecture · `primitive` `rdu` `area:moe` `stream:R5` · depends on: SCHED-1, FUSIONS:PARITY-ROUTER-SHARED-GATE-MERGE
- [ ] **ARCH-DENSE-FIRST** · Dense leading layers and per-layer expert widths · architecture · `primitive` `rdu` `area:moe` `stream:R5` · depends on: SCHED-1
- [ ] **ARCH-MIXTRAL** · Mixtral's experts read and run · architecture · `primitive` `rdu` `area:moe` `stream:R5` · depends on: INTAKE-2, SCHED-1
- [ ] **ARCH-OSS** · gpt-oss experts on the RDU · architecture · `primitive` `rdu` `area:moe` `model:oss` `stream:R5` · depends on: SCHED-1, NUM-3
- [ ] **ARCH-GDN-VARIANTS** · Gated-delta variants · architecture · `primitive` `rdu` `area:recurrent` `stream:R6` · depends on: SCHED-1
- [ ] **ARCH-SSD** · Mamba2 chunked state-space scan · architecture · `primitive` `rdu` `area:recurrent` `stream:R6` · depends on: KERNEL-3, FUSIONS:PARITY-SSD-CHUNK-SCAN
- [ ] **ARCH-SELECTIVE-SCAN** · Mamba1's selective scan · architecture · `primitive` `rdu` `area:recurrent` `stream:R6` · depends on: KERNEL-3, FUSIONS:PARITY-SELECTIVE-SCAN
- [ ] **ARCH-RWKV** · RWKV-7 · architecture · `primitive` `rdu` `area:recurrent` `stream:R6` · depends on: KERNEL-3
- [ ] **ARCH-LIGHTNING** · Lightning attention · architecture · `primitive` `rdu` `area:recurrent` `stream:R6` · depends on: KERNEL-3, FUSIONS:PARITY-LIGHTNING-ATTENTION
- [ ] **ARCH-KDA** · Kimi delta attention · architecture · `primitive` `rdu` `area:recurrent` `stream:R6` · depends on: ARCH-GDN-VARIANTS, FUSIONS:PARITY-KDA
- [ ] **ARCH-SHORT-CONV** · Gated short convolution · architecture · `primitive` `rdu` `area:recurrent` `stream:R6` · depends on: SCHED-1, FUSIONS:PARITY-SHORT-CONV
- [ ] **ARCH-CNN** · Convolutional networks · architecture · `primitive` `rdu` `stream:R6` · depends on: SCHED-1
- [ ] **ARCH-NORMS** · Norm and residual forms on the RDU · architecture · `primitive` `rdu` `area:norm` `stream:R7` · depends on: SCHED-1, AUTHOR-2
- [ ] **QWEN4-1** · qwen4_exp read, bound and golden · qwen4 · `infrastructure` `model:qwen4exp` `stream:R14` · depends on: INTAKE-1, INTAKE-4, STUDY-3
- [ ] **QWEN4-2** · Hyper-connections as a row section and an epilogue · qwen4 · `primitive` `rdu` `model:qwen4exp` `stream:R14` · depends on: QWEN4-1, SCHED-1, FUSIONS:PARITY-HYPER-CONNECTIONS
- [ ] **QWEN4-3** · The n-gram hash, exact on 32-bit lanes · qwen4 · `primitive` `rdu` `model:qwen4exp` `stream:R14` · depends on: QWEN4-1, KERNEL-6
- [ ] **QWEN4-4** · The n-gram table in DDR, gathered ahead of the step · qwen4 · `technique` `rdu` `model:qwen4exp` `stream:R14` · depends on: QWEN4-3, KERNEL-5
- [ ] **QWEN4-5** · The per-layer embedding mixer and its convolution state · qwen4 · `primitive` `rdu` `model:qwen4exp` `stream:R14` · depends on: QWEN4-4, QWEN4-2
- [ ] **QWEN4-6** · Qwen sparse attention at 262,144 positions · qwen4 · `primitive` `rdu` `model:qwen4exp` `stream:R14` · depends on: ARCH-SPARSE, QWEN4-1
- [ ] **QWEN4-7** · 512 narrow experts with a gated shared expert · qwen4 · `primitive` `rdu` `model:qwen4exp` `stream:R14` · depends on: ARCH-SHARED-GATE, MULTI-2, QWEN4-1
- [ ] **QWEN4-8** · Gated delta with a sigmoid gate · qwen4 · `primitive` `rdu` `model:qwen4exp` `stream:R14` · depends on: ARCH-GDN-VARIANTS, QWEN4-1
- [ ] **QWEN4-9** · Interleaved multimodal rotation on a quarter of the head · qwen4 · `primitive` `rdu` `model:qwen4exp` `stream:R14` · depends on: ARCH-ROPE, QWEN4-1
- [ ] **QWEN4-10** · The MTP head drafts on chip · qwen4 · `serving` `rdu` `model:qwen4exp` `stream:R14` · depends on: SERVE-RDU-1, QWEN4-1
- [ ] **QWEN4-11** · The vision tower · qwen4 · `primitive` `rdu` `model:qwen4exp` `stream:R14` · depends on: ARCH-VISION, QWEN4-1
- [ ] **QWEN4-12** · The whole model as a deployment · qwen4 · `infrastructure` `rdu` `model:qwen4exp` `stream:R14` · depends on: QWEN4-2, QWEN4-5, QWEN4-6, QWEN4-7, QWEN4-8, QWEN4-9, MULTI-5
- [ ] **QWEN4-13** · The Qwen 4 day-zero drill · qwen4 · `exam` `model:qwen4exp` `stream:R14` · depends on: QWEN4-12, INTAKE-7
- [ ] **KERNEL-1** · Top-k over a long row · kernel · `primitive` `rdu` `area:sampling` `stream:R4` · depends on: SCHED-1
- [ ] **KERNEL-2** · Sort and permutation of tokens by expert · kernel · `primitive` `rdu` `area:moe` `stream:R5` · depends on: SCHED-1
- [ ] **KERNEL-3** · Segmented and chunked scans · kernel · `primitive` `rdu` `area:recurrent` `stream:R6` · depends on: SCHED-1
- [ ] **KERNEL-4** · Scatter with conflicts resolved by a fold · kernel · `primitive` `rdu` `area:moe` `stream:R5` · depends on: SCHED-1
- [ ] **KERNEL-5** · A hashed gather from a table in DDR · kernel · `primitive` `rdu` `area:memory` `stream:R10` · depends on: MEM-1, KERNEL-6
- [x] **KERNEL-6** · 64-bit integer arithmetic on 32-bit lanes · kernel · `primitive` `rdu` `stream:R9` · depends on: none
- [ ] **KERNEL-7** · Reductions past a chip's accumulator budget · kernel · `primitive` `rdu` `stream:R7` · depends on: SCHED-1
- [ ] **KERNEL-8** · Sparse weights at a stated density · kernel · `primitive` `rdu` `area:quant` `stream:R9` · depends on: SCHED-1
- [ ] **SCHED-1** · One file per op family in the spatial lowerings · schedule · `infrastructure` `rdu` `stream:R7` · depends on: none
- [ ] **SCHED-2** · Sections stream into each other · schedule · `technique` `rdu` `stream:R7` · depends on: TIME-1
- [ ] **SCHED-3** · The layer loop · schedule · `technique` `rdu` `stream:R7` · depends on: SCHED-2
- [ ] **SCHED-4** · Samples stream across layer boundaries · schedule · `technique` `rdu` `stream:R7` · depends on: SCHED-3
- [ ] **SCHED-5** · The next layer's weights arrive during this one · schedule · `technique` `rdu` `area:memory` `stream:R7` · depends on: SCHED-3
- [ ] **SCHED-6** · Many tokens a launch · schedule · `technique` `rdu` `stream:R7` · depends on: SCHED-3
- [ ] **SCHED-7** · Buffers sized by footprint, the head first · schedule · `technique` `rdu` `area:memory` `stream:R7` · depends on: none
- [ ] **SCHED-8** · Compile time inside a budget at 70B, 405B and 671B · schedule · `infrastructure` `rdu` `stream:R7` · depends on: INTAKE-6
- [ ] **SCHED-9** · A program larger than a chip splits itself · schedule · `technique` `rdu` `stream:R7` · depends on: TIME-1
- [ ] **SCHED-10** · Fused classes accepted whole by the spatial shim · schedule · `technique` `rdu` `stream:R7` · depends on: SCHED-1
- [ ] **SCHED-11** · A whole model resident and pipelined · schedule · `technique` `rdu` `plasticine` `stream:R7` · depends on: SCHED-2
- [ ] **SCHED-12** · Directives that pin the compiler's choices · schedule · `technique` `rdu` `stream:R7` · depends on: SCHED-1
- [ ] **SCHED-13** · Transposes and banks as layouts · schedule · `technique` `rdu` `plasticine` `stream:R7` · depends on: CHIP-3
- [ ] **MULTI-1** · Collectives as dataflow, overlapped · multi-chip · `technique` `rdu` `area:comm` `stream:R8` · depends on: TIME-1
- [ ] **MULTI-2** · Expert parallelism, both dispatch forms · multi-chip · `technique` `rdu` `area:moe` `area:comm` `stream:R8` · depends on: MULTI-1, KERNEL-2, FUSIONS:PARITY-EXPERT-PARALLEL
- [ ] **MULTI-3** · Pipeline parallelism across nodes · multi-chip · `technique` `rdu` `area:comm` `stream:R8` · depends on: MULTI-1
- [ ] **MULTI-4** · Tensor parallelism at widths the heads do not divide · multi-chip · `technique` `rdu` `area:comm` `stream:R8` · depends on: none
- [ ] **MULTI-5** · The parallel plan chosen by the time model · multi-chip · `technique` `rdu` `stream:R8` · depends on: MULTI-2, MULTI-3, TIME-2
- [ ] **MULTI-6** · Link roofs and topology · multi-chip · `infrastructure` `rdu` `area:comm` `stream:R8` · depends on: CHIP-1
- [ ] **NUM-1** · fp32 islands in a bf16 datapath · numerics · `technique` `rdu` `area:quant` `stream:R9` · depends on: none
- [ ] **NUM-2** · fp8 weights with block scales · numerics · `primitive` `rdu` `area:quant` `stream:R9` · depends on: INTAKE-3
- [ ] **NUM-3** · MXFP4 experts · numerics · `primitive` `rdu` `area:quant` `stream:R9` · depends on: none
- [ ] **NUM-4** · NVFP4 weights · numerics · `primitive` `rdu` `area:quant` `stream:R9` · depends on: INTAKE-3
- [ ] **NUM-5** · 4-bit integer weights · numerics · `primitive` `rdu` `area:quant` `stream:R9` · depends on: INTAKE-3
- [ ] **NUM-6** · An 8-bit KV cache · numerics · `primitive` `rdu` `area:quant` `area:attention` `stream:R9` · depends on: FUSIONS:PARITY-KV-CACHE-QUANT
- [ ] **NUM-7** · The RDU's numerics contract · numerics · `infrastructure` `rdu` `stream:R9` · depends on: NUM-1, FUSIONS:OPT-6
- [ ] **NUM-8** · Batch invariance and determinism · numerics · `technique` `rdu` `stream:R9` · depends on: NUM-1
- [ ] **MEM-1** · DDR as a placed tier · memory · `technique` `rdu` `area:memory` `stream:R10` · depends on: CHIP-1
- [ ] **MEM-2** · Model switching from DDR · memory · `technique` `rdu` `area:memory` `stream:R10` · depends on: MEM-1
- [ ] **MEM-3** · The KV cache across HBM and DDR · memory · `technique` `rdu` `area:memory` `stream:R10` · depends on: MEM-1
- [ ] **MEM-4** · The dense spine resident in SRAM across chips · memory · `technique` `rdu` `area:memory` `stream:R10` · depends on: MEM-1, MULTI-1
- [ ] **MEM-5** · A prefix cache · memory · `serving` `rdu` `area:memory` `stream:R10` · depends on: MEM-3
- [ ] **MEM-6** · Hot experts in HBM, cold experts in DDR · memory · `technique` `rdu` `area:moe` `area:memory` `stream:R10` · depends on: MEM-1, MULTI-2
- [ ] **MEM-7** · Storage reused by lifetime, spilled by bandwidth · memory · `technique` `rdu` `area:memory` `stream:R10` · depends on: MEM-1
- [ ] **SERVE-RDU-1** · Speculative decoding with recurrent rollback · serving · `serving` `rdu` `stream:R11` · depends on: SCHED-6, AUTHOR-3, FUSIONS:SERVE-SPEC-DECODE
- [ ] **SERVE-RDU-2** · Sampling on chip · serving · `serving` `rdu` `area:sampling` `stream:R11` · depends on: KERNEL-1, FUSIONS:PARITY-SAMPLING
- [ ] **SERVE-RDU-3** · Prefill on a GPU, decode on the RDU · serving · `serving` `rdu` `stream:R11` · depends on: MULTI-1
- [ ] **SERVE-RDU-4** · Ragged and continuous batches · serving · `serving` `rdu` `stream:R11` · depends on: SCHED-4, FUSIONS:SERVE-CONTINUOUS-BATCHING
- [ ] **SERVE-RDU-5** · An OpenAI-compatible runner over a bundle · serving · `serving` `rdu` `stream:R11` · depends on: ACCESS-4, SERVE-RDU-2
- [ ] **SERVE-RDU-6** · Grammar-constrained decoding on chip · serving · `serving` `rdu` `area:sampling` `stream:R11` · depends on: SERVE-RDU-2, FUSIONS:SERVE-STRUCTURED-OUTPUT
- [ ] **TRAIN-1** · A training step on the spatial pipeline · training · `technique` `rdu` `stream:R17` · depends on: SCHED-1, FUSIONS:BEYOND-TRAINING
- [ ] **TRAIN-2** · The optimizer step in place · training · `technique` `rdu` `stream:R17` · depends on: TRAIN-1
- [ ] **TRAIN-3** · Training mapped onto as many chips as it needs · training · `technique` `rdu` `stream:R17` · depends on: TRAIN-2, MULTI-5
- [ ] **MINE-1** · Rewrite candidates harvested from every graph · mining · `beyond` `stream:R12` · depends on: INTAKE-5
- [ ] **MINE-2** · Every candidate proved or refuted · mining · `beyond` `stream:R12` · depends on: MINE-1, FUSIONS:OPT-5
- [ ] **MINE-3** · Precision chosen by proven bounds · mining · `beyond` `area:quant` `stream:R12` · depends on: NUM-7, FUSIONS:BEYOND-PROVEN-NUMERICS
- [ ] **MINE-4** · Exact schedules by constraint solving · mining · `beyond` `stream:R12` · depends on: TIME-1, FUSIONS:BEYOND-EXACT-MODULO-SCHEDULER
- [ ] **MINE-5** · Residency by integer program · mining · `beyond` `area:memory` `stream:R12` · depends on: MEM-1, FUSIONS:BEYOND-CERTIFIED-ALLOCATION
- [ ] **MINE-6** · Parallel plans with a certificate of optimality · mining · `beyond` `area:comm` `stream:R12` · depends on: MULTI-5
- [ ] **MINE-7** · The mined-rule ledger · mining · `beyond` `stream:R12` · depends on: MINE-2
- [ ] **MINE-8** · Qwen 4's algebraic folds · mining · `beyond` `model:qwen4exp` `stream:R12` · depends on: MINE-2, QWEN4-1
- [ ] **CLIMB-1** · One objective · climb · `infrastructure` `stream:R13` · depends on: SCORE-1
- [ ] **CLIMB-2** · The gap miner · climb · `infrastructure` `stream:R13` · depends on: CLIMB-1, TIME-6
- [x] **CLIMB-3** · The experiment ledger · climb · `infrastructure` `stream:R13` · depends on: none
- [ ] **CLIMB-4** · The compiler searches its own choices · climb · `beyond` `stream:R13` · depends on: TIME-2
- [ ] **CLIMB-5** · Every measurement refits the time model · climb · `infrastructure` `area:time` `stream:R13` · depends on: TIME-2
- [ ] **CLIMB-6** · Rules land through proof and measurement · climb · `beyond` `stream:R13` · depends on: MINE-7, CLIMB-3
- [ ] **CLIMB-7** · The exam grows itself · climb · `exam` `stream:R13` · depends on: INTAKE-5, EXAM-1
- [ ] **CLIMB-8** · The ratchet · climb · `infrastructure` `stream:R13` · depends on: CLIMB-1
- [ ] **CLIMB-9** · The standing loop · climb · `infrastructure` `stream:R13` · depends on: CLIMB-2, CLIMB-3, CLIMB-8
- [ ] **CLIMB-10** · The climb, reported · climb · `infrastructure` `stream:R13` · depends on: CLIMB-9
- [ ] **EXAM-1** · Cold bring-up of unseen configurations · exam · `exam` `stream:R13` · depends on: none
- [ ] **EXAM-2** · Correctness against the references · exam · `exam` `stream:R13` · depends on: none
- [ ] **EXAM-3** · The decode matrix · exam · `exam` `stream:R13` · depends on: none
- [ ] **EXAM-4** · Prefill and the first token · exam · `exam` `stream:R13` · depends on: none
- [ ] **EXAM-5** · Socket scaling · exam · `exam` `stream:R13` · depends on: none
- [ ] **EXAM-6** · Adversarial expert routing · exam · `exam` `stream:R13` · depends on: none
- [ ] **EXAM-7** · Speculative decoding · exam · `exam` `stream:R13` · depends on: none
- [ ] **EXAM-8** · Compiler quality · exam · `exam` `stream:R13` · depends on: none
- [ ] **EXAM-9** · Programs larger than a chip · exam · `exam` `stream:R13` · depends on: none
- [ ] **EXAM-10** · A new op, first run then fused · exam · `exam` `stream:R13` · depends on: none
- [ ] **EXAM-11** · The kernel micro-exam · exam · `exam` `stream:R13` · depends on: none
- [ ] **EXAM-12** · Multimodal input, serving and the Qwen 4 day zero · exam · `exam` `stream:R13` · depends on: none
- [ ] **EXAM-13** · The Plasticine kit's workloads · exam · `exam` `stream:R13` · depends on: none
- [ ] **SCORE-1** · The cell ledger · score · `infrastructure` `stream:R13` · depends on: none
- [ ] **SCORE-2** · Every Copy bar a ledger row · score · `infrastructure` `stream:R13` · depends on: SCORE-1, STUDY-1
- [ ] **SCORE-3** · Our rows, with their provenance · score · `infrastructure` `stream:R13` · depends on: SCORE-1, TIME-4
- [ ] **SCORE-4** · A lost cell grows tickets · score · `infrastructure` `stream:R13` · depends on: SCORE-3, TIME-6
- [ ] **DEPLOY-QWEN38** · Qwen3.8-27B and Qwen3.5 on the generic path · deployment · `infrastructure` `rdu` `stream:R16` · depends on: INTAKE-6
- [ ] **DEPLOY-LLAMA3** · Llama 3.x, 1B to 405B · deployment · `infrastructure` `rdu` `stream:R16` · depends on: INTAKE-6, ARCH-ROPE
- [ ] **DEPLOY-QWEN25** · Qwen 2.5, 0.5B to 72B · deployment · `infrastructure` `rdu` `stream:R16` · depends on: INTAKE-6
- [ ] **DEPLOY-MIXTRAL** · Mixtral 8x7B · deployment · `infrastructure` `rdu` `stream:R16` · depends on: ARCH-MIXTRAL, INTAKE-6
- [ ] **DEPLOY-OSS** · gpt-oss 20B and 120B · deployment · `infrastructure` `rdu` `stream:R16` · depends on: ARCH-OSS, ARCH-SINKS, ARCH-WINDOW, INTAKE-6
- [ ] **DEPLOY-DEEPSEEK3** · DeepSeek-V3 and R1 · deployment · `infrastructure` `rdu` `stream:R16` · depends on: ARCH-MLA, ARCH-ROUTING, ARCH-DENSE-FIRST, MULTI-2, NUM-2, INTAKE-6
- [ ] **DEPLOY-LLAMA4** · Llama 4 Scout and Maverick · deployment · `infrastructure` `rdu` `stream:R16` · depends on: ARCH-WINDOW, ARCH-NOPE, ARCH-DENSE-FIRST, MULTI-2, INTAKE-6
- [ ] **DEPLOY-MINIMAX** · MiniMax M2 series · deployment · `infrastructure` `rdu` `stream:R16` · depends on: MULTI-2, INTAKE-6
- [ ] **DEPLOY-GEMMA** · Gemma 1, 2 and 3 · deployment · `infrastructure` `rdu` `stream:R16` · depends on: ARCH-SOFTCAP, ARCH-WINDOW, ARCH-NORMS, INTAKE-6
- [ ] **DEPLOY-KIMI-GLM** · Kimi K2 and GLM · deployment · `infrastructure` `rdu` `stream:R16` · depends on: ARCH-MLA, ARCH-ROUTING, ARCH-SPARSE, MULTI-2, INTAKE-6
- [ ] **DEPLOY-HYBRID-SSM** · Jamba, Falcon-H1, Nemotron-H and Mamba2 · deployment · `infrastructure` `rdu` `stream:R16` · depends on: ARCH-SSD, ARCH-SELECTIVE-SCAN, INTAKE-6

### 11.2 Dispatch order

The critical path runs through the chip facts and the time model: STUDY-1, CHIP-1, CHIP-2 and CHIP-3, TIME-1,
TIME-2. Every schedule, multi-chip and scoring ticket that ranks plans waits on it. Work that needs no time model
starts on the first day in every stream (section 1.4's first wave). Within a wave, a stream takes its tickets in the
order section 6 weights them: Tier A first, then Qwen 4, then the kit's workloads, then Tier C.

The waves below are this board's dependency levels: a ticket in wave n depends only on tickets in earlier waves and
on fusion-book tickets.

| Wave | Tickets |
|---|---|
| 1 | BOOK-1, STUDY-1, STUDY-3, STUDY-5, ACCESS-2, ACCESS-3, ACCESS-4, PLAST-2, PLAST-3, PLAST-5, PLAST-6, PLAST-7, INTAKE-1, AUTHOR-1, AUTHOR-2, AUTHOR-3, AUTHOR-4, AUTHOR-5, KERNEL-6, SCHED-1, SCHED-7, MULTI-4, NUM-1, NUM-3, NUM-6, CLIMB-3, EXAM-1, EXAM-2, EXAM-3, EXAM-4, EXAM-5, EXAM-6, EXAM-7, EXAM-8, EXAM-9, EXAM-10, EXAM-11, EXAM-12, EXAM-13, SCORE-1 |
| 2 | STUDY-2, STUDY-4, ACCESS-6, PLAST-1, PLAST-11, CHIP-1, INTAKE-2, INTAKE-3, INTAKE-4, ARCH-WINDOW, ARCH-SINKS, ARCH-SOFTCAP, ARCH-NOPE, ARCH-MLA, ARCH-ROPE, ARCH-CROSS, ARCH-POOLING, ARCH-ROUTING, ARCH-SHARED-GATE, ARCH-DENSE-FIRST, ARCH-OSS, ARCH-GDN-VARIANTS, ARCH-SHORT-CONV, ARCH-CNN, ARCH-NORMS, KERNEL-1, KERNEL-2, KERNEL-3, KERNEL-4, KERNEL-7, KERNEL-8, SCHED-10, SCHED-12, NUM-7, NUM-8, TRAIN-1, CLIMB-1, SCORE-2 |
| 3 | ACCESS-1, PLAST-10, PLAST-12, CHIP-2, CHIP-3, CHIP-4, INTAKE-5, INTAKE-6, INTAKE-8, INTAKE-9, ARCH-SPARSE, ARCH-VISION, ARCH-MIXTRAL, ARCH-SSD, ARCH-SELECTIVE-SCAN, ARCH-RWKV, ARCH-LIGHTNING, ARCH-KDA, QWEN4-1, MULTI-6, NUM-2, NUM-4, NUM-5, MEM-1, SERVE-RDU-2, TRAIN-2, MINE-3, CLIMB-8 |
| 4 | ACCESS-7, ACCESS-8, PLAST-9, PLAST-14, CHIP-5, TIME-1, INTAKE-7, QWEN4-2, QWEN4-3, QWEN4-6, QWEN4-8, QWEN4-9, QWEN4-11, KERNEL-5, SCHED-8, SCHED-13, MEM-2, MEM-3, MEM-7, SERVE-RDU-5, SERVE-RDU-6, MINE-1, MINE-5, CLIMB-7, DEPLOY-QWEN38, DEPLOY-LLAMA3, DEPLOY-QWEN25, DEPLOY-MIXTRAL, DEPLOY-OSS, DEPLOY-GEMMA, DEPLOY-HYBRID-SSM |
| 5 | ACCESS-5, PLAST-8, TIME-2, TIME-3, TIME-6, QWEN4-4, SCHED-2, SCHED-9, MULTI-1, MEM-5, MINE-2, MINE-4 |
| 6 | PLAST-4, TIME-4, TIME-5, QWEN4-5, SCHED-3, SCHED-11, MULTI-2, MULTI-3, MEM-4, SERVE-RDU-3, MINE-7, MINE-8, CLIMB-2, CLIMB-4, CLIMB-5 |
| 7 | PLAST-13, QWEN4-7, SCHED-4, SCHED-5, SCHED-6, MULTI-5, MEM-6, CLIMB-6, CLIMB-9, SCORE-3, DEPLOY-DEEPSEEK3, DEPLOY-LLAMA4, DEPLOY-MINIMAX, DEPLOY-KIMI-GLM |
| 8 | QWEN4-12, SERVE-RDU-1, SERVE-RDU-4, TRAIN-3, MINE-6, CLIMB-10, SCORE-4 |
| 9 | QWEN4-10, QWEN4-13 |

### 11.3 The ticket template

Every body on this board follows this form (the fusion book's section 12.3, with this book's labels):

```
#### <ID> · <title>

| Field | Value |
|---|---|
| Phase | <phase> |
| Labels | <kind> <chip> <area> <model> |
| Depends on | <ids on this board, FUSIONS:<id> for the fusion book's, or none> |
| Touches | <existing files, and new modules named in words> |
| Status | open, claimed or done |

Goal. Copy (a published bar, when one exists). Beyond (where we pass it).
Read first. RED tests (each named suite::test with the reason it fails today). Edge cases.
Implementation. Success (each box with its receipt). Mutants to kill. Verify (the commands). Done when.
```

Labels: kind `study`, `infrastructure`, `primitive`, `technique`, `serving`, `beyond`, `exam`; chip `rdu`, `plasticine`;
area `area:attention`, `area:moe`, `area:gemm`, `area:recurrent`, `area:quant`, `area:comm`, `area:graph`, `area:norm`,
`area:memory`, `area:intake`, `area:sampling`, `area:time`; model `model:qwen4exp`, `model:oss`.

### 11.4 The book and the study

The book's own lock and the reading every later ticket stands on. A study ticket's output is this book's tables, corrected and pinned.

#### BOOK-1 · The book holds together

| Field | Value |
|---|---|
| Phase | book |
| Labels | `infrastructure` |
| Depends on | none |
| Touches | `benchmarks/PREPARE_FOR_SAMBA.md`, `crates/logicaffeine_tests/tests/samba_book_lock.rs` |
| Status | done |

**Goal.** Every derived part of this book is checked, never kept by hand: index lines against bodies, streams against owners, dependencies against both boards, exam tests against locks and tickets, Copy bars against the bibliography, cited paths against the tree, and the preface's counts against the book.

**Read first.**
- `crates/logicaffeine_tests/tests/sota_contract_lock.rs`, the fusion book's lock this one extends.
- `crates/logicaffeine_tests/tests/public_prose_lock.rs`, the citation and voice rules every public document keeps.

**RED tests.**
- [x] `samba_book_lock::every_ticket_body_has_one_index_line_that_agrees`: red until the book exists (RED receipt: `logs/samba-book/red-1.log`)
- [x] `samba_book_lock::the_dependencies_resolve_and_never_cycle`: red until the book exists (RED receipt: `logs/samba-book/red-1.log`)
- [x] `samba_book_lock::every_exam_test_names_its_lock_and_its_ticket`: red until the book exists (RED receipt: `logs/samba-book/red-1.log`)

**Edge cases.**
- [x] A dependency on a fusion-book ticket that the fusion book renames (receipt: mutant `bad_fusions_ref`, `logs/samba-book/mutants.log`).
- [x] A ticket body whose template misses one part (receipt: mutant `no_mutants_part`).
- [x] A Copy bar row added without a citation (receipt: mutant `uncited_bar`).
- [x] A stream that owns a directory another stream owns a file inside (receipt: `samba_book_lock::no_file_belongs_to_two_streams`, prefix check).

**Implementation.** The lock parses section 11.1's index and section 11's bodies as `sota_contract_lock` parses the fusion book's, and adds the checks this book needs: template completeness, file ownership by prefix, a dependency graph with no cycle, exam rows, citations and counts. `SAMBA_BOOK` points a mutation run at a mutated copy.

**Success.**
- [x] Every test of `samba_book_lock` green on the book (receipt: `logs/samba-book/final-locks.log`).
- [x] Each mutant below turns at least one test red (receipt: `logs/samba-book/mutants.log`, 12/12).

**Mutants to kill.**
- [x] Drop one ticket body (killed: `drop_body`).
- [x] Give two streams one file (killed: `two_owners`).
- [x] Cite a path that does not exist (killed: `missing_path`).
- [x] Add a Copy bar without a citation (killed: `uncited_bar`).
- [x] Change one count in the preface (killed: `wrong_count`).
- [x] Add a dependency cycle of two tickets (killed: `cycle`).

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_book_lock
cargo nextest run -p logicaffeine-tests --test public_prose_lock
```

**Done when.** The lock is green on the book and red on each mutant.

#### STUDY-1 · The bibliography, pinned

| Field | Value |
|---|---|
| Phase | study |
| Labels | `study` |
| Depends on | none |
| Touches | section 12.3 of this book; the paper pins (new) |
| Status | done |

**Goal.** Every source section 12.3 lists is read in full and pinned: its title, authors, venue and year as published, a stable link, and the SHA-256 of the PDF or page captured under `external/`. Entries this book marks as read through a search excerpt are re-read at the source; a number that differs from the source is corrected everywhere it is quoted.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [x] `samba_book_lock::every_copy_bar_cites_the_bibliography_and_every_entry_is_cited`: already green on structure; this ticket adds the pinned digest column it will check (receipt: `logs/samba/green-2.log`)
- [x] `samba_papers_lock::every_entry_names_a_venue_a_year_and_a_digest`: fails today: entries marked to pin carry no venue, year or digest (receipt: green in `logs/samba/green-2.log`; the RED was not run before the pins landed, and the pin-mismatch mutant stands for it)
- [x] `samba_papers_lock::every_copy_bar_number_appears_in_its_pinned_source`: fails today: no pinned source text exists to search (receipt: green in `logs/samba/green-2.log`; the RED was not run before the captures landed, and the one-digit mutant stands for it)

**Edge cases.**
- [x] A source that has moved or gone offline: the capture is the record, its date beside it (receipt: the `captured` column of `benchmarks/samba_papers.tsv`; ALCF's pages are captured from Wayback).
- [x] A preprint and its published version: both pinned, the published one cited (receipt: SN40L24, CAPSTAN21, REVET and STEP each pin the arXiv PDF and the venue's record).
- [x] A vendor page whose numbers change after capture: the capture date is part of the citation (receipt: the AA26 Copy bar reads "as captured 2026-10-29").
- [x] A Copy bar whose number differs from its pinned source (receipt: the gpt-oss-120b bar corrected to 740 tok/s as captured, and `samba_papers_lock` in `logs/samba/green-2.log`).

**Implementation.** Capture each source's text under `external/` (a local location the public tree may name), write the pins as a tab-separated file the lock reads, and add a lock that searches each pinned source for every number this book attributes to it.

**Success.**
- [x] Every entry of section 12.3 carries venue, year and digest (receipt: `samba_papers_lock` in `logs/samba/green-2.log`).
- [x] Every Copy bar number is found in its pinned source (receipt: `samba_papers_lock` in `logs/samba/green-2.log`).
- [x] Every entry marked to pin in this book is pinned or removed with its quotes (receipt: no entry of section 12.3 says "to pin", `samba_papers_lock::every_entry_names_a_venue_a_year_and_a_digest`).

**Mutants to kill.**
- [x] A Copy bar number changed by one digit (receipt: `logs/samba/mutants2/results.json`, study1-copy-bar-digit killed).
- [x] A pin whose digest does not match its capture (receipt: `logs/samba/mutants2/results.json`, study1-pin-mismatch killed).

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_papers_lock
cargo nextest run -p logicaffeine-tests --test samba_book_lock
```

**Done when.** Every number this book attributes to a source is checked against that source's pinned text.

#### STUDY-2 · Every published technique, mapped

| Field | Value |
|---|---|
| Phase | study |
| Labels | `study` |
| Depends on | STUDY-1 |
| Touches | sections 5.1 and 5.2 of this book |
| Status | open |

**Goal.** Read every pinned SambaNova and Stanford paper end to end and extract every compiler or runtime technique it describes, not only those section 5 lists today. Each becomes a row in section 5 with its source, mechanism, our status and the ticket that copies it; a technique with no ticket gets one.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `samba_book_lock::every_ticket_the_book_names_is_on_the_board`: holds the new rows' tickets
- [ ] `samba_techniques_lock::every_technique_row_names_a_ticket_on_the_board`: fails today: the table is not machine-read

**Edge cases.**
- [ ] A technique that is a hardware feature, not a compiler technique: recorded with the chip facts instead.
- [ ] A technique two papers name differently: one row, both sources.
- [ ] A technique we already exceed: its row says so and cites our lock.

**Implementation.** Read; extract; write the rows; open tickets for the gaps through the book keeper.

**Success.**
- [ ] Every technique in every pinned paper is a row (receipt: the extraction notes beside each pin).
- [ ] Every row names a ticket on this board (receipt: `samba_techniques_lock`).

**Mutants to kill.**
- [ ] A row with its ticket removed.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_techniques_lock
cargo nextest run -p logicaffeine-tests --test samba_book_lock
```

**Done when.** Section 5 is the whole of the published technique set, each row a ticket.

#### STUDY-3 · Qwen3.8-Flash-Next, read end to end

| Field | Value |
|---|---|
| Phase | study |
| Labels | `study` `model:qwen4exp` |
| Depends on | none |
| Touches | section 7 of this book |
| Status | done |

**Goal.** Read the transformers modeling file, vLLM's `qwen4_exp` tree, llama.cpp's `qwen4exp` graph and its converter line by line, and the checkpoint's tensor table. Every formula, shape and budget in section 7 is confirmed or corrected against them: the hyper-connection mixers, the n-gram hash and its padding, the indexer's selection and tie order, the gated-delta gate and scale, the expert router, the draft head, the rotation.

**Read first.**
- `external/competitors/vllm/vllm/models/qwen4_exp`
- `external/competitors/llama.cpp/src/models/qwen4exp.cpp`
- `external/competitors/llama.cpp/conversion/qwen4exp.py`
- `external/competitors/llama.cpp/src/llama-memory-hybrid-idx.cpp`

**RED tests.**
- [x] `qwen38_flash_next_facts_lock::every_budget_in_section_7_equals_the_tensor_table`: fails today: section 7's budgets are computed from the configuration, not read from the checkpoint (receipt: RED `logs/qwen4exp/red-1.log`, the suite did not build; green `logs/samba/green-3.log`)
- [x] `qwen38_flash_next_facts_lock::the_hash_matches_the_reference_on_sampled_token_triples`: fails today: no hash exists in the tree (receipt: RED `logs/qwen4exp/red-1.log`, no `model::ngram_hash`; green `logs/samba/green-3.log`)

**Edge cases.**
- [x] A tensor present in the checkpoint that no section 7 component names (receipt: `qwen38_flash_next_facts_lock::every_tensor_belongs_to_a_component_section_7_names`; the embedding and the head had no paragraph and now do).
- [x] A configuration default the modeling file overrides (receipt: section 7.7's configuration defaults; the absent `seed` confirmed by `qwen38_flash_next_facts_lock::the_hash_constants_equal_the_checkpoints_own`).
- [x] A difference between the vLLM and llama.cpp implementations: recorded, and the reference that matches transformers chosen (receipt: section 7.7's differences).

**Implementation.** Read the four implementations; record each formula with its file and line; recompute section 7.3 from the tensor table by script; correct the book through the keeper.

**Success.**
- [x] Section 7 matches all four references, every difference between them recorded (receipt: section 7.7, the reading notes).
- [x] Section 7.3's budgets equal the tensor table (receipt: `qwen38_flash_next_facts_lock` in `logs/samba/green-3.log`; the attention layer corrected to 51.45M).

**Mutants to kill.**
- [x] A budget row off by one layer (receipt: `logs/samba/mutants2/results.json`, with an unnamed component and a hash offset off by one, all killed).

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test qwen38_flash_next_facts_lock
```

**Done when.** Section 7 is a reading of the references, not of articles about them.

#### STUDY-4 · The rival spatial compilers, read where they publish

| Field | Value |
|---|---|
| Phase | study |
| Labels | `study` |
| Depends on | STUDY-1 |
| Touches | section 5.3 of this book |
| Status | open |

**Goal.** Read every spatial compiler that publishes results on dataflow or spatial chips (SambaFlow's documentation, tt-mlir and ttnn, Loom, the dataflow benchmark suite, Cerebras and Groq's compiler papers) and record what each does that we must match, and every published number on a chip this book scores as a Copy bar.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `samba_techniques_lock::every_rival_row_cites_a_pinned_source`: fails today: the rival table is not machine-read

**Edge cases.**
- [ ] A rival result on a chip we do not score: recorded for scale, not as a Copy bar.
- [ ] A rival technique already on the board: its row names the ticket.

**Implementation.** Read, record, open tickets for gaps, add Copy bars through SCORE-2.

**Success.**
- [ ] Every rival row cites a pinned source (receipt: `samba_techniques_lock`).
- [ ] Every rival number on a scored chip is a ledger row (receipt: the cell ledger).

**Mutants to kill.**
- [ ] A rival row without a source.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_techniques_lock
```

**Done when.** Section 5.3 says what every published spatial compiler does and where it stands.

#### STUDY-5 · The Plasticine evaluation kit, read and pinned

| Field | Value |
|---|---|
| Phase | study |
| Labels | `study` `plasticine` |
| Depends on | none |
| Touches | `scripts/plasticine/`; the kit's pins under `external/` (new) |
| Status | open |

**Goal.** Pin the 2019 Stanford Plasticine kit [PTUT] and its tracer [STUBBED] (version 0.1.1, sdist SHA-256 4f7ba5dc…596e78), read both, and write down exactly what a submission is and what the kit reports: the `get_model_info` contract, the Model Analyzer's fields, the tracer's two files (memory spaces: id, bytes, element bytes; accesses: space, offset, size, R or W, iterator id; the operation count) and the estimator's report (memory entries, the DRAMSim2 configuration, cycles, power, runtime).

**Read first.**
- `scripts/plasticine/README.md`
- `scripts/plasticine/fetch.sh`

**RED tests.**
- [ ] `plasticine_kit_lock::the_tracer_writes_the_two_files_this_book_describes`: fails today: the tracer is not pinned in the tree
- [ ] `plasticine_kit_lock::a_get_model_info_module_returns_the_four_parts`: fails today: no reader of the contract exists

**Edge cases.**
- [ ] A `get_model_info` that returns no loss: the analyzer adds a default; ours must too.
- [ ] A tracer access with iterator id −1 (an untracked access).
- [ ] A NumPy array not converted to the tracer's tracked type: untraced, as the kit's own behaviour.

**Implementation.** Fetch the kit and the tracer into `external/` with their digests by a script beside `scripts/plasticine/fetch.sh`; record the formats in this book's section 2.1 and in the tickets PLAST-11 to PLAST-14 read; add a lock that runs the tracer on a small NetworkX program and checks the two files.

**Success.**
- [ ] The kit and tracer pinned with digests (receipt: the fetch script and its log).
- [ ] The formats recorded and checked by running the tracer (receipt: `plasticine_kit_lock`).

**Mutants to kill.**
- [ ] A memory-space line with its element size dropped.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test plasticine_kit_lock
```

**Done when.** The kit's interface is written down, pinned and exercised.

### 11.5 Access

The adapters of section 3. Each is built and locked on what we hold today, so the first day of any access level is mechanical.

#### ACCESS-1 · The first-hour intake card

| Field | Value |
|---|---|
| Phase | access |
| Labels | `infrastructure` `rdu` |
| Depends on | STUDY-2 |
| Touches | `crates/logicaffeine_compile/src/spatial/lower/rdu_sdk.rs`; the intake card (new) |
| Status | open |

**Goal.** The questions of section 3, in their fixed order, as a card an agent fills in the first hour of any access level. Each answer has a slot in the chip facts, so a filled card regenerates `RduChipFacts` with provenance `vendor` and re-runs every plan that depends on a changed fact.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `rdu_intake_card_lock::a_filled_card_regenerates_the_chip_facts`: fails today: no card exists
- [ ] `rdu_intake_card_lock::a_changed_fact_names_every_plan_it_moves`: fails today: plans do not record which facts they read

**Edge cases.**
- [ ] An answer that contradicts a published fact: both kept, the vendor answer used, the contradiction reported.
- [ ] A question the vendor declines: the fact stays modeled and says so.
- [ ] A fact the card has no slot for: added as a new slot, never dropped.
- [ ] A vendor-runtime feature the ledger sends here (the runtime as an operating system for the chips, its
  fault-management records, replay on an uncorrectable error): a question on the card, its answer kept with the facts.

**Implementation.** A card format with one slot per fact; a reader into `RduChipFacts`; plans record the facts they read so a change names what it moves.

**Success.**
- [ ] A filled card regenerates the facts (receipt: `rdu_intake_card_lock`).
- [ ] A changed HBM bandwidth names every cell whose prediction it moves (receipt: `rdu_intake_card_lock`).

**Mutants to kill.**
- [ ] A slot read into the wrong fact.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test rdu_intake_card_lock
cargo nextest run -p logicaffeine-tests --test sn50_target_lock
```

**Done when.** An hour with the vendor's answers turns into updated facts and re-ranked plans by one command.

#### ACCESS-2 · SambaFlow modules for every program, experts included

| Field | Value |
|---|---|
| Phase | access |
| Labels | `infrastructure` `rdu` |
| Depends on | none |
| Touches | `crates/logicaffeine_compile/src/spatial/lower/sambaflow.rs`, `crates/logicaffeine_tests/tests/rdu_sambaflow_emit_lock.rs` |
| Status | open |

**Goal.** The PyTorch-level adapter. For every program the compiler writes (every cut of every model, experts included) the emitter writes a PyTorch module per kind program, the weights' layout, a fusion-rule file and a compile configuration naming the compile mode, the tensor-parallel mode and the layers that repeat. Today it refuses `moe` and writes `compile.py` as a template.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `rdu_sambaflow_emit_lock::an_expert_layer_emits_a_module`: fails today: the emitter refuses moe
- [ ] `rdu_sambaflow_emit_lock::every_emitted_module_computes_what_the_vm_computes`: fails today: modules are not executed against the VM
- [ ] `rdu_sambaflow_emit_lock::the_compile_configuration_names_every_repeated_subgraph`: fails today: no hints are written

**Edge cases.**
- [ ] A module that imports under a stub of their package and runs under plain PyTorch on the CPU.
- [ ] A tensor-parallel bundle: one module a chip slice.
- [ ] An op with no PyTorch spelling: refused by name, never approximated.

**Implementation.** Write each kind program as an `nn.Module` whose forward is the program's op sequence; run every module under CPU PyTorch in the lock against the VM; write the configuration from the bundle's manifest.

**Success.**
- [ ] Every Tier A model's kind programs emit and equal the VM under CPU PyTorch (receipt: the lock).
- [ ] The configuration names every repeated subgraph (receipt: the lock).

**Mutants to kill.**
- [ ] An emitted module with one op's operands swapped.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test rdu_sambaflow_emit_lock
```

**Done when.** On the day SambaFlow arrives, every program we write is already a module it can compile.

#### ACCESS-3 · Vendor placements read back and certified

| Field | Value |
|---|---|
| Phase | access |
| Labels | `infrastructure` `rdu` |
| Depends on | none |
| Touches | `crates/logicaffeine_compile/src/spatial/lower/rdu_sdk.rs`, `crates/logicaffeine_tests/tests/rdu_sdk_bridge_lock.rs` |
| Status | open |

**Goal.** The low-level adapter's return path. `RdaBridge::certify_back` today reads only our own neutral schema. It gains a reader per vendor placement form we can anticipate (unit configurations, a Spatial-like IR, a PEF's kernels and schedules once its format is known) behind one trait, and certifies what it reads against our source as `codegen_sva/pnr_certificate.rs` certifies an external placement.

**Read first.**
- `crates/logicaffeine_compile/src/codegen_sva/pnr_certificate.rs`

**RED tests.**
- [ ] `rdu_sdk_bridge_lock::a_placement_in_a_foreign_schema_is_read_and_certified`: fails today: only our schema is read
- [ ] `rdu_sdk_bridge_lock::a_foreign_placement_that_computes_otherwise_is_refused`: fails today: no foreign reader exists

**Edge cases.**
- [ ] A vendor placement that fuses two of our sections into one kernel.
- [ ] A placement that reorders a reduction: certified within the numerics contract, not bit for bit.
- [ ] A placement the certificate cannot decide within its budget: reported as undecided, never as certified.

**Implementation.** A `PlacementReader` trait; a reader for the Plasticine chain's own mapping output as the first foreign schema (it exists today); the certificate path of `certify_external_placement` reused.

**Success.**
- [ ] PIR's mapping of a segment is read back and certified against our source (receipt: the lock).
- [ ] A mapping with one wrong edge is refused (receipt: the lock).

**Mutants to kill.**
- [ ] A reader that drops one edge.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test rdu_sdk_bridge_lock
cargo nextest run -p logicaffeine-tests --test pnr_certificate_lock
```

**Done when.** Whatever placement a vendor tool produces, we read it back and prove it computes what our source computes.

#### ACCESS-4 · A self-checking artifact drop

| Field | Value |
|---|---|
| Phase | access |
| Labels | `infrastructure` `rdu` |
| Depends on | none |
| Touches | `crates/logicaffeine_compile/src/spatial/lower/rdu.rs`; the drop format (new) |
| Status | open |

**Goal.** When the evaluator runs our artifacts and we see nothing else, the drop must carry its own proof: the manifest, the weights or their source, every program's certificate, the goldens of the tiny configuration, a runner that executes the bundle on our evaluator, and the expected output with its digest. A drop verifies itself on a machine with only the runner.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `rdu_drop_lock::a_drop_verifies_itself_with_only_its_runner`: fails today: no drop format exists
- [ ] `rdu_drop_lock::a_drop_with_one_flipped_byte_refuses_itself`: fails today: no digests are carried

**Edge cases.**
- [ ] A drop whose weights are fetched, not carried: their digests are carried.
- [ ] A drop for a chip count the runner's machine cannot emulate: the per-chip programs run in turn.

**Implementation.** A drop writer over the existing `largo emit rdu --model` output, plus a runner and a manifest of digests.

**Success.**
- [ ] A tiny Qwen3.8 drop verifies on a clean checkout with only the runner (receipt: the lock and its log).
- [ ] A drop with one flipped byte refuses (receipt: the lock).

**Mutants to kill.**
- [ ] A runner that skips the certificate recheck.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test rdu_drop_lock
cargo nextest run -p logicaffeine-tests --test emit_rdu_e2e
```

**Done when.** Anything we hand over proves itself.

#### ACCESS-5 · One harness for any cycle simulator

| Field | Value |
|---|---|
| Phase | access |
| Labels | `infrastructure` `rdu` |
| Depends on | TIME-1 |
| Touches | the simulator harness interface (new) |
| Status | open |

**Goal.** A cycle simulator from the vendor drops into the harness our Tungsten runs use: one interface that takes a program, runs it, reads cycles and checks values against the oracle. Tungsten and Spatial's simulator are its first two implementations.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `rdu_simulator_harness_lock::tungsten_and_spatials_simulator_run_behind_one_interface`: fails today: each has its own harness

**Edge cases.**
- [ ] A simulator that reports cycles but no values: values checked by our evaluator from its trace.
- [ ] A simulator that times out: reported, never read as a pass.

**Implementation.** Lift `crates/logicaffeine_tests/tests/common/plasticine.rs`'s run and parse into a trait with two implementations.

**Success.**
- [ ] Both simulators behind one interface (receipt: the lock).

**Mutants to kill.**
- [ ] An implementation that reports a timeout as zero cycles.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test rdu_simulator_harness_lock
cargo nextest run -p logicaffeine-tests --test qwen38_fabric_segments_lock
```

**Done when.** A new simulator is one implementation of one trait.

#### ACCESS-6 · A research allocation on SambaNova hardware

| Field | Value |
|---|---|
| Phase | access |
| Labels | `infrastructure` `rdu` |
| Depends on | ACCESS-4 |
| Touches | the allocation runbook (new) |
| Status | open |

**Goal.** SambaNova DataScale systems are run on a research testbed with allocations [ALCF]. Write the runbook for applying, for moving a drop onto the system, for compiling with SambaFlow and for recording measured rows with provenance; the application itself is Tristen's decision.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `rdu_drop_lock::the_runbook_names_every_step_a_drop_takes_on_the_testbed`: fails today: no runbook exists

**Edge cases.**
- [ ] An allocation with an older SambaFlow: the runbook records the version.
- [ ] A system of an older chip generation: its facts become a new preset, never folded into SN40L's.

**Implementation.** A runbook in the book's style, held by a lock that checks each step names a command and a receipt.

**Success.**
- [ ] The runbook exists and is current (receipt: the lock).

**Mutants to kill.**
- [ ] A runbook step without a receipt.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test rdu_drop_lock
```

**Done when.** If an allocation is granted, the first measured row is a runbook away.

#### ACCESS-7 · The SDK-day runbook, current

| Field | Value |
|---|---|
| Phase | access |
| Labels | `infrastructure` `rdu` |
| Depends on | ACCESS-1 |
| Touches | `scripts/accel/sambacloud-smoke.sh`; the SDK-day runbook (new) |
| Status | open |

**Goal.** The SDK-day runbook names a stale toolchain path and a segment table that no longer matches `benchmarks/silicon/qwen38_fabric_segments.json`. Bring it current, move it into the public tree as a runbook a lock checks, and record SambaCloud's quality side for the Tier A models it serves through `scripts/accel/sambacloud-smoke.sh`.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `rdu_runbook_lock::every_path_the_runbook_names_exists`: fails today: the runbook names a moved toolchain path
- [ ] `rdu_runbook_lock::the_segment_table_is_the_ledgers`: fails today: the table is stale

**Edge cases.**
- [ ] A SambaCloud model whose name changes: the runbook records the date.

**Implementation.** Rewrite the runbook; generate its segment table from the ledger; add the lock.

**Success.**
- [ ] The runbook is current and generated where it can be (receipt: the lock).

**Mutants to kill.**
- [ ] A stale segment row.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test rdu_runbook_lock
```

**Done when.** The runbook an agent follows on SDK day is true.

#### ACCESS-8 · Unit configurations at Plasticine granularity

| Field | Value |
|---|---|
| Phase | access |
| Labels | `infrastructure` `rdu` `plasticine` |
| Depends on | CHIP-2, CHIP-3 |
| Touches | the unit-configuration emitter (new) |
| Status | open |

**Goal.** A low-level SDK built on Plasticine's principles configures units: a PCU's stages and lanes, a PMU's banking and address pipeline, a switch's routes, an address generator's pattern. Emit a placed program as exactly that, one record a unit, from the unit model CHIP-2 and CHIP-3 build, and check it by simulating the records on our evaluator.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `rdu_unit_config_lock::a_placed_program_emits_one_record_a_unit`: fails today: no emitter exists
- [ ] `rdu_unit_config_lock::the_records_simulate_to_the_programs_values`: fails today: no record simulator exists

**Edge cases.**
- [ ] A section that needs more stages than a PCU has: chained PCUs, each a record.
- [ ] A PMU in a banking mode the record cannot state: refused by name.

**Implementation.** Lower each placed stage to a PCU record, each buffer to a PMU record, each edge to a route; simulate the records with a small interpreter the lock runs.

**Success.**
- [ ] Every Tier A layer kind emits records that simulate to the VM's values (receipt: the lock).

**Mutants to kill.**
- [ ] A route with its destination swapped.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test rdu_unit_config_lock
```

**Done when.** The lowest SDK level we can anticipate is already a target.

### 11.6 Plasticine

The open chain the evaluation's principles come from: Spatial, PIR, plastiroute and Tungsten, built by `scripts/plasticine/build.sh`. Every ticket here runs on what we hold today.

#### PLAST-1 · Real-width layers through PIR and Tungsten

| Field | Value |
|---|---|
| Phase | plasticine |
| Labels | `plasticine` |
| Depends on | PLAST-2, PLAST-6 |
| Touches | `crates/logicaffeine_compile/src/spatial/lower/spatial_lang.rs`, `crates/logicaffeine_tests/tests/qwen38_fabric_segments_lock.rs`, `benchmarks/silicon/qwen38_fabric_segments.json` |
| Status | open |

**Goal.** Today's Plasticine evidence is the tiny Qwen3.8 configuration only. Run real-width layer kinds through the chain: a Qwen3.8-27B gated-delta layer and attention layer, a Llama-3.1-8B decoder layer, and the Qwen3.8-Flash-Next layer kinds, each as segments small enough for PIR, each passing the ideal and the routed network, each checked against the oracle on every tensor it stores.

**Read first.**
- `scripts/plasticine/README.md`
- `crates/logicaffeine_tests/tests/common/plasticine.rs`

**RED tests.**
- [ ] `qwen38_fabric_segments_lock::every_real_width_segment_passes_both_networks`: fails today: only the tiny configuration is recorded
- [ ] `qwen38_fabric_segments_lock::a_llama_8b_layer_simulates_in_segments`: fails today: no Llama layer is emitted

**Edge cases.**
- [ ] A segment whose app is too large for one JVM method: split by the section-chunking printer.
- [ ] A real-width weight that does not fit the chain's DRAM model: the chain's DRAM sized to it.
- [ ] A segment whose run exceeds the 30-minute per-test cap: recorded by the ledger's recorder, not the fast test.

**Implementation.** Extend the segment recorder to real-width fills (seeded, deterministic); record the ledger rows; keep the fast test hash-checking today's apps against the ledger.

**Success.**
- [ ] Every real-width layer kind of the four models has its segments recorded, ideal and routed passing (receipt: the ledger rows).
- [ ] Each segment's cycles are a simulated cell row (receipt: the cell ledger).

**Mutants to kill.**
- [ ] A segment app with one constant changed: its hash reds the fast test.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test qwen38_fabric_segments_lock
```

**Done when.** Plasticine evidence exists at the widths the exam scores.

#### PLAST-2 · Every segment passes the routed network

| Field | Value |
|---|---|
| Phase | plasticine |
| Labels | `plasticine` `area:comm` |
| Depends on | none |
| Touches | `scripts/plasticine/`, `benchmarks/silicon/qwen38_fabric_segments.json` |
| Status | open |

**Goal.** Four of ten recorded segments (gated-delta 20 to 24, attention 96 to 100, 100 to 104, 104 to 108) pass Tungsten's ideal network and fail the routed one with a virtual-channel coloring fault ("no route for packet", deadlock). Find why each needs more than one virtual channel and remove the cause in our emitted program, or configure plastiroute with the channels the program needs, so every segment passes both networks.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `qwen38_fabric_segments_lock::every_recorded_segment_passes_the_routed_network`: fails today: four segments fail routed

**Edge cases.**
- [ ] A segment that passes routed only with more channels: the channel count is a recorded configuration, not a silent default.
- [ ] A segment that deadlocks routed: its cycle of dependences named in the ledger.

**Implementation.** Read plastiroute's coloring for each failing segment; trace each extra channel to the edges that force it; change the emitted program (fan-out, buffer placement) where it forces them.

**Success.**
- [ ] All ten segments pass routed (receipt: the ledger).

**Mutants to kill.**
- [ ] Revert the fix for one segment: its routed row reds.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test qwen38_fabric_segments_lock
```

**Done when.** No emitted segment depends on the ideal network to pass.

#### PLAST-3 · Our placement is the one simulated

| Field | Value |
|---|---|
| Phase | plasticine |
| Labels | `plasticine` |
| Depends on | none |
| Touches | `crates/logicaffeine_compile/src/codegen_sva/plasticine_place.rs`, `crates/logicaffeine_compile/src/spatial/lower/spatial_lang.rs` |
| Status | open |

**Goal.** PIR maps every emitted app onto its own mesh with its own placement; our placement is never simulated. Emit our placement as PIR constraints (or a PIR-level program with units fixed) so Tungsten simulates the mapping our compiler chose, and compare its cycles with PIR's own.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `plasticine_placement_lock::tungsten_simulates_our_placement`: fails today: PIR places every app itself
- [ ] `plasticine_placement_lock::our_placement_is_no_slower_than_pirs_on_every_segment`: fails today: no comparison exists

**Edge cases.**
- [ ] A placement PIR cannot honor on its mesh: refused with the unit it cannot place.
- [ ] A segment where ours is slower: the gap is a ticket under the gap rule.

**Implementation.** Read PIR's placement pass for the constraint form it accepts; emit ours in it; run both; record both.

**Success.**
- [ ] Every recorded segment simulates under our placement (receipt: the lock).
- [ ] Our placement's cycles at or under PIR's on every segment, or the gap is a ticket (receipt: the ledger).

**Mutants to kill.**
- [ ] A placement with two stages swapped.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test plasticine_placement_lock
```

**Done when.** The Plasticine numbers are our compiler's, end to end.

#### PLAST-4 · Section chains as metapipelines

| Field | Value |
|---|---|
| Phase | plasticine |
| Labels | `plasticine` `technique` |
| Depends on | SCHED-2 |
| Touches | `crates/logicaffeine_compile/src/spatial/lower/spatial_lang.rs` |
| Status | open |

**Goal.** Our composed programs print as a `Sequential` of sections, a barrier at each boundary. Print a chain of sections that stream into each other (SCHED-2) as a Spatial `Pipe` or `Stream` controller over N-buffered memories, so consecutive sections overlap across iterations as Plasticine's metapipelines do, and measure the cycle drop.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `plasticine_metapipeline_lock::a_streamed_chain_prints_as_a_pipe`: fails today: every chain prints as Sequential
- [ ] `plasticine_metapipeline_lock::a_metapipelined_segment_is_faster_than_its_sequential_twin`: fails today: no Pipe is printed

**Edge cases.**
- [ ] A chain whose middle section is read by two later sections: buffered for both.
- [ ] A chain with a data-dependent trip count: Stream with a FIFO, not Pipe.
- [ ] N-buffering beyond what the PMUs hold: the depth limited and recorded.

**Implementation.** Print `Pipe` with N-buffered SRAMs for chains SCHED-2 marks streamable; `Stream` with FIFOs for data-dependent ones; record both twins in the ledger.

**Success.**
- [ ] Every streamable chain in the recorded segments prints as a metapipeline (receipt: the lock).
- [ ] The cycle drop recorded per segment (receipt: the ledger).

**Mutants to kill.**
- [ ] A Pipe with its buffer depth set to 1.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test plasticine_metapipeline_lock
```

**Done when.** Our programs use Plasticine's own overlap mechanism.

#### PLAST-5 · Multi-PCU GEMMs, and Table 4 reproduced

| Field | Value |
|---|---|
| Phase | plasticine |
| Labels | `plasticine` `area:gemm` |
| Depends on | none |
| Touches | `crates/logicaffeine_compile/src/codegen_sva/plasticine_report.rs`, `benchmarks/silicon/plasticine_table4_gate.json`, `benchmarks/silicon/plasticine_calibration.json` |
| Status | open |

**Goal.** Every emitted GEMM today uses one compute PCU (640³ takes 16.4M cycles, 16 MACs a cycle), and the PLDI'18 Table 4 gate fails on every mapped row. Emit GEMMs with outer and inner parallelization factors and banking chosen per shape, so a GEMM spreads over many PCUs, and reproduce Table 4's Plasticine column inside the gate's stated tolerance.

**Copy.** PLDI'18 Table 4's Plasticine column [PLDI18].

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `plasticine_report_lock::a_640_cubed_gemm_uses_more_than_one_pcu`: fails today: one compute PCU
- [ ] `plasticine_sim_lock::table4_is_reproduced_within_the_gate`: fails today: every mapped row is outside the gate

**Edge cases.**
- [ ] A shape whose parallelization factor does not divide its dimension.
- [ ] A banking that PIR splits differently from our model: recorded and modeled.
- [ ] A Table 4 row PIR cannot map: named in the gate.

**Implementation.** Choose par factors by the time model per shape; bank PMUs for the parallel reads; record the Table 4 rows.

**Success.**
- [ ] Table 4's Plasticine column reproduced within the gate (receipt: the gate ledger).
- [ ] GEMM cycles fall with par as the model predicts (receipt: the calibration ledger).

**Mutants to kill.**
- [ ] A par factor forced to 1.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test plasticine_report_lock
cargo nextest run -p logicaffeine-tests --test plasticine_sim_lock
```

**Done when.** Our GEMMs use the fabric as Plasticine's authors did, and their published table agrees.

#### PLAST-6 · bf16 apps

| Field | Value |
|---|---|
| Phase | plasticine |
| Labels | `plasticine` `area:quant` |
| Depends on | none |
| Touches | `crates/logicaffeine_compile/src/spatial/lower/spatial_lang.rs` |
| Status | open |

**Goal.** The Spatial emitter writes every float as Spatial's `Float` (fp32). Emit bf16 storage and products where the program's numerics say bf16 (Spatial's custom float types), fp32 where they say fp32, so simulated cycles and values reflect the chip's datapath.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `plasticine_precision_lock::a_bf16_program_prints_bf16_types`: fails today: every float prints fp32
- [ ] `plasticine_precision_lock::a_bf16_segment_matches_the_bf16_evaluator`: fails today: no bf16 app is simulated

**Edge cases.**
- [ ] A mixed program: bf16 products with fp32 accumulators and islands.
- [ ] A conversion Tungsten does not model: refused by name.

**Implementation.** Map the program's per-buffer precision onto Spatial's float type parameters.

**Success.**
- [ ] Every recorded segment's precision is the program's (receipt: the lock).

**Mutants to kill.**
- [ ] A bf16 buffer printed as fp32.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test plasticine_precision_lock
```

**Done when.** Simulated values and cycles are the chip's precision, not fp32's.

#### PLAST-7 · Every segment check can fail

| Field | Value |
|---|---|
| Phase | plasticine |
| Labels | `plasticine` |
| Depends on | none |
| Touches | `crates/logicaffeine_tests/tests/qwen38_fabric_segments_lock.rs`, `crates/logicaffeine_compile/src/spatial/lower/spatial_lang.rs` |
| Status | open |

**Goal.** The attention-score segment's check compares against a masked score of −1e30 at relative tolerance, so its bound is about 2e28 and it can never fail. Check masked positions by mask, live positions by value, and mutation-test every segment check.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `qwen38_fabric_segments_lock::a_wrong_score_fails_the_attention_check`: fails today: the check passes any value

**Edge cases.**
- [ ] A masked position that should read the mask value exactly.
- [ ] A live position near the mask threshold.

**Implementation.** Split each tensor check into mask positions (exact) and live positions (tolerance); mutate one live score in a private copy and see the check fail.

**Success.**
- [ ] Every segment's check fails on a corrupted value (receipt: the mutation log).

**Mutants to kill.**
- [ ] Corrupt one live attention score by 1%.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test qwen38_fabric_segments_lock
```

**Done when.** No Plasticine check passes by construction.

#### PLAST-8 · The calibration covers every app

| Field | Value |
|---|---|
| Phase | plasticine |
| Labels | `plasticine` `area:time` |
| Depends on | TIME-1 |
| Touches | `benchmarks/silicon/plasticine_calibration.json`, `crates/logicaffeine_compile/src/codegen_sva/plasticine_report.rs` |
| Status | open |

**Goal.** 34 of the chain's apps fall within the modeled-cycles tolerance and 76 cannot be described by the model at all. Extend the time model (TIME-1) until every app in the chain's regression set is described, and hold each within the tolerance or record why not.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `plasticine_sim_lock::every_regression_app_is_described_by_the_model`: fails today: 76 apps are not described

**Edge cases.**
- [ ] An app with data-dependent control: described by its trip-count window.
- [ ] An app whose cycles the model misses by more than the tolerance: a ticket under the gap rule.

**Implementation.** Describe each missing app's controllers, memories and DRAM traffic in the time model; refit; record.

**Success.**
- [ ] Every app described (receipt: the calibration ledger).
- [ ] The share within tolerance recorded (receipt: the calibration ledger).

**Mutants to kill.**
- [ ] Drop one app from the calibration set.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test plasticine_sim_lock
```

**Done when.** The time model is checked against the whole public Plasticine regression set.

#### PLAST-9 · SARA-scale and SN-shaped meshes

| Field | Value |
|---|---|
| Phase | plasticine |
| Labels | `plasticine` |
| Depends on | CHIP-2 |
| Touches | `crates/logicaffeine_compile/src/codegen_sva/plasticine.rs`, `scripts/plasticine/` |
| Status | open |

**Goal.** Run the chain on the SARA 20×20 HBM2 preset and on the largest mesh PIR and Tungsten accept toward an SN-shaped checkerboard of 1040 PCUs and 1040 PMUs, and record what PIR's mapping does at each size.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `plasticine_mesh_lock::a_segment_maps_on_the_sara_preset`: fails today: the chain runs its default mesh
- [ ] `plasticine_mesh_lock::the_largest_mesh_pir_accepts_is_recorded`: fails today: never measured

**Edge cases.**
- [ ] A mesh PIR refuses: its limit recorded with the error.
- [ ] A mesh whose routing time exceeds the per-test cap: run by the recorder.

**Implementation.** Pass the mesh configuration through the gate script; record mapping, routing and cycles per size.

**Success.**
- [ ] Segments simulated on two mesh sizes beyond the default (receipt: the ledger).

**Mutants to kill.**
- [ ] A mesh size silently clamped.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test plasticine_mesh_lock
```

**Done when.** We know how the open chain behaves at the scale of the chips we are scored on.

#### PLAST-10 · Data-dependent control on Plasticine

| Field | Value |
|---|---|
| Phase | plasticine |
| Labels | `plasticine` `technique` |
| Depends on | KERNEL-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/lower/spatial_lang.rs` |
| Status | open |

**Goal.** Expert routing, top-k selection and data-dependent trip counts are refused by the Spatial emitter today. Print them with Spatial's data-dependent controllers and FIFOs, and simulate a routed expert layer and a top-k selection through the chain.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `plasticine_dynamic_lock::a_routed_expert_layer_simulates`: fails today: the emitter refuses trips and routing
- [ ] `plasticine_dynamic_lock::a_top_k_selection_simulates`: fails today: no selection is emitted

**Edge cases.**
- [ ] A trip count of zero.
- [ ] Routing that sends every token to one expert.

**Implementation.** Map the IR's trip counts to Spatial's counters bounded by a register; routing to FIFO-steered controllers; selection to KERNEL-1's form.

**Success.**
- [ ] Both simulate and equal the oracle (receipt: the lock).

**Mutants to kill.**
- [ ] A trip counter off by one.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test plasticine_dynamic_lock
```

**Done when.** Plasticine runs the data-dependent parts of a modern model.

#### PLAST-11 · The kit's trace-driven estimator, rebuilt and checked

| Field | Value |
|---|---|
| Phase | plasticine |
| Labels | `plasticine` |
| Depends on | STUDY-5 |
| Touches | `scripts/plasticine/`; the trace estimator build (new) |
| Status | open |

**Goal.** The kit estimates graph and sparse programs from two files, memory spaces and an access trace, with a Tungsten configuration of four DRAMSim2 channels of 4096 MB, a DRAMSim2 clock of 800000000 Hz and a CPU clock of 1000000000 Hz [PTUT]. Our Tungsten tree carries that path's pieces (a trace in the same format under its `trace/` directory, the sparsity apps and templates). Rebuild the trace-driven estimator from it, run the kit's tracer on a NetworkX random walk, and get a cycle count, a runtime and the report's memory-entry lines.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `plasticine_kit_lock::a_traced_random_walk_reports_cycles_and_runtime`: fails today: no trace estimator is built
- [ ] `plasticine_kit_lock::the_estimator_reports_one_memory_entry_per_space`: fails today: no trace estimator is built

**Edge cases.**
- [ ] A trace with an untracked access (iterator id −1).
- [ ] A trace too large to simulate in the cap: the kit's own advice, a smaller graph, scaled linearly and recorded as scaled.

**Implementation.** Locate the trace front end in the Tungsten tree; build it with the kit's DRAM configuration; wire it into the simulator harness (ACCESS-5).

**Success.**
- [ ] A random walk's cycles and runtime from the rebuilt estimator (receipt: the lock).
- [ ] The estimator's configuration recorded beside every row (receipt: the ledger).

**Mutants to kill.**
- [ ] A configuration with one DRAM channel dropped.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test plasticine_kit_lock
```

**Done when.** The kit's estimator runs here, on its own inputs.

#### PLAST-12 · Traces of LOGOS programs in the kit's format

| Field | Value |
|---|---|
| Phase | plasticine |
| Labels | `plasticine` |
| Depends on | PLAST-11 |
| Touches | the trace emitter (new) |
| Status | open |

**Goal.** A graph or sparse program written in LOGOS runs on our evaluator and writes the kit's two files: one memory space a tracked collection (bytes, element bytes), one access row a tracked read or write (space, offset, size, R or W, iterator id), and the operation count. The estimator then simulates LOGOS programs exactly as it simulates the tracer's.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `plasticine_kit_lock::a_logos_random_walk_writes_both_files`: fails today: no emitter exists
- [ ] `plasticine_kit_lock::a_logos_trace_and_the_tracers_agree_on_memory_spaces`: fails today: no emitter exists

**Edge cases.**
- [ ] A program that frees and reuses a collection: a new space, as the tracer does.
- [ ] A nested iterator: its own iterator id.

**Implementation.** Instrument the evaluator's collection reads and writes behind a trace sink; write the two files.

**Success.**
- [ ] The same random walk, written in LOGOS and in NetworkX, gives the same spaces and cycles within the estimator's noise (receipt: the lock).

**Mutants to kill.**
- [ ] An emitter that drops write rows.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test plasticine_kit_lock
```

**Done when.** Any LOGOS program can be scored the way the kit scores Python.

#### PLAST-13 · The Model Analyzer's report, from our compiler

| Field | Value |
|---|---|
| Phase | plasticine |
| Labels | `plasticine` `area:time` |
| Depends on | INTAKE-8, TIME-1, SCHED-11 |
| Touches | the analyzer report (new) |
| Status | open |

**Goal.** Take a `get_model_info` module (INTAKE-8), map it in the spatial mode (SCHED-11) onto chips of 64 compute and 64 memory units at a stated chip count and weight sparsity, and report every field the Model Analyzer reports, inference and training: nodes original and after merging, function nodes, maximum depth, total FLOPs, chip FLOPS utilization, compute units used, throughput, stage latency, model latency, memory bytes, memory units and utilization. The power field is filled only from a simulated source and never appears on a slide.

**Copy.** The kit's SqueezeNet report [PTUT].

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `plasticine_analyzer_lock::squeezenet_reports_every_field`: fails today: no report exists
- [ ] `plasticine_analyzer_lock::squeezenet_flops_equal_700981753`: fails today: no FLOP count of an imported module exists

**Edge cases.**
- [ ] A model too large for one chip: the analyzer's warning, then a plan on more chips.
- [ ] A FLOP-counting convention that differs from the kit's: the convention stated, the kit's number matched.

**Implementation.** Count FLOPs on the imported graph; merge nodes as the spatial mapping does; take stage latency from the time model; write the report.

**Success.**
- [ ] SqueezeNet's report beside the kit's published one, every field present (receipt: the lock).
- [ ] ResNet-50's report (receipt: the lock).

**Mutants to kill.**
- [ ] A report with the training section dropped.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test plasticine_analyzer_lock
```

**Done when.** A model handed to us the kit's way gets the kit's report from our compiler.

#### PLAST-14 · Graph and sparse programs on Plasticine

| Field | Value |
|---|---|
| Phase | plasticine |
| Labels | `plasticine` |
| Depends on | PLAST-12 |
| Touches | `crates/logicaffeine_compile/src/spatial/lower/spatial_lang.rs` |
| Status | open |

**Goal.** Random walk, BFS, PageRank and CSR and COO sparse matrix-vector products, written in LOGOS, lower through the whole-program and loop lanes to Spatial and simulate on the chain, and trace through PLAST-12 to the estimator. Tungsten's own sparsity apps (BFS, PageRank, CSR and COO GEMV) are the reference implementations.

**Read first.**
- Section 1 of this book, and the sections this ticket's Goal names.

**RED tests.**
- [ ] `plasticine_kit_lock::csr_and_coo_spmv_simulate`: fails today: no sparse program is emitted
- [ ] `plasticine_kit_lock::pagerank_and_bfs_simulate`: fails today: no graph program is emitted

**Edge cases.**
- [ ] An empty row in CSR.
- [ ] A graph with a vertex of degree zero.
- [ ] Repeated column indices in COO.

**Implementation.** Write the programs in LOGOS; lower; simulate; compare with Tungsten's sparsity apps on the same inputs.

**Success.**
- [ ] All four programs simulate and equal the VM (receipt: the lock).
- [ ] Cycles beside the Tungsten sparsity apps' (receipt: the ledger).

**Mutants to kill.**
- [ ] A CSR row pointer off by one.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test plasticine_kit_lock
```

**Done when.** The kit's non-neural workloads run through our compiler.
### 11.7 Chip truth and time

The chip facts and the time model every plan is ranked by. Until TIME-1 and TIME-2 land, no ticket on this board can
rank its own plans, so they head the critical path.

#### CHIP-1 · Every chip fact in its published unit

| Field | Value |
|---|---|
| Phase | chip |
| Labels | `infrastructure` `rdu` |
| Depends on | STUDY-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/rdu_chip.rs`, `crates/logicaffeine_tests/tests/rdu_chip_facts_lock.rs` (new), `crates/logicaffeine_tests/tests/sn50_target_lock.rs`, `crates/logicaffeine_tests/tests/sn40l_target_lock.rs` |
| Status | open |

**Goal.** `RduChipFacts` carries the capacities in binary units without saying why, models HBM bandwidth as
4 × 460.8 GB/s where SN40L publishes 1.6 TB/s a socket, cites Hot Chips for an SN40L peak of 640 TFLOPS where Hot Chips
says 638, has no DDR bandwidth and no P2P rate, and keeps one of the two SN50 DDR maxima the vendor publishes (section
4.2). Every fact is restated from its pinned source (STUDY-1) in the unit the source uses, with the reading that turns
it into bytes, or marked modeled with the reason; a figure two sources state differently keeps both.

**Read first.**
- `crates/logicaffeine_compile/src/spatial/rdu_chip.rs`: the `Fact` type and both presets.
- Section 4.2 of this book.

**RED tests.**
- [x] `rdu_chip_facts_lock::every_capacity_is_read_as_the_vendor_counts`: fails today: no PMU capacity to hold the
  520 MB against, and no capacity's citation names its reading.
- [x] `rdu_chip_facts_lock::the_sn40l_hbm_bandwidth_is_the_published_1_6_tb_s`: fails today: modeled at 1,843.2 GB/s.
- [x] `rdu_chip_facts_lock::ddr_and_p2p_rates_are_facts`: fails today: no such fields.
- [x] `rdu_chip_facts_lock::a_figure_two_sources_state_differently_keeps_both`: fails today: one figure each.
- [x] `rdu_chip_facts_lock::every_published_fact_cites_a_pinned_source`: fails today: a cited source carries no
  bibliography key, and the SN40L's 640 TFLOPS is not in the source it cites. (Receipts for the five: RED
  `logs/samba/chip1-red.log`, green `logs/samba/intake6-run-7.log`.)

**Edge cases.**
- [x] A fact published in two units by two sources: both kept, the unit named (`RduChipFacts::others`).
- [x] A fact a later source supersedes: the later one used, the earlier kept as history (the SN40L's HBM and DDR rates).
- [x] Every number that moves: the plans and pinned fixtures it moves are re-recorded with Tristen's approval (the
  lanes board's RE-RECORD line of 2026-10-30).

**Implementation.** Add the PMU capacity, the DDR bandwidth, the P2P rate and each figure's history to
`RduChipFacts`; name each cited source's bibliography key; restate each value from its pin; re-record the fixtures that
move.

**Success.**
- [x] Every fact cites a pinned source or says modeled (receipt: `rdu_chip_facts_lock`, `logs/samba/intake6-run-7.log`).
- [x] The re-recorded fixtures listed with their approvals (receipt: the lanes board's RE-RECORD line of 2026-10-30).

**Mutants to kill.**
- [ ] A capacity read in decimal units.
- [ ] A fact with its citation removed.
- [ ] A superseded figure dropped from the history.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test rdu_chip_facts_lock --test sn40l_target_lock --test sn50_target_lock --test rdu_fit_report_lock
```

**Done when.** No chip number in the tree disagrees with its source or its unit.

#### CHIP-2 · A PCU that reaches the published peak

| Field | Value |
|---|---|
| Phase | chip |
| Labels | `infrastructure` `rdu` |
| Depends on | CHIP-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/rdu_chip.rs`, `crates/logicaffeine_tests/tests/rdu_unit_model_lock.rs` |
| Status | open |

**Goal.** The RDU unit model is Tensix-shaped (32×32 tiles, 8 accumulator tiles) and its PCUs, at 16 lanes and 1 GHz,
reach about 33 TFLOPS against 638 published for SN40L. Model the PCU as SN40L describes it: a unit that runs either as
SIMD lanes over pipelined stages or as a systolic array whose inputs stream left to right and top to bottom and whose
results drain right [DIAYN26], with its shape solved from the published peak, unit count and clock, so the model's peak
equals the published peak.

**Read first.**
- `crates/logicaffeine_compile/src/codegen_sva/systolic.rs`: our weight-stationary and output-stationary arrays.
- `crates/logicaffeine_compile/src/codegen_sva/metapipeline.rs`: the ISCA'17 PCU model.

**RED tests.**
- [ ] `rdu_unit_model_lock::the_modeled_peak_equals_the_published_peak`: fails today: about 33 TFLOPS against 638.
- [ ] `rdu_unit_model_lock::a_matmul_section_maps_onto_systolic_pcus`: fails today: tiles are 32×32 Tensix tiles.

**Edge cases.**
- [ ] A clock the vendor has not published: solved, marked modeled, and every plan that reads it says so.
- [ ] A GEMM narrower than the systolic shape: its padding counted.
- [ ] The SIMD mode for elementwise and reduction sections.

**Implementation.** A `PcuMode { Simd { lanes, stages }, Systolic { rows, cols } }` in the unit model; tile shapes
derived from it; the tile pass reads them.

**Success.**
- [ ] The modeled peak equals the published peak for SN40L, and for SN50 once its count is known (receipt: the lock).
- [ ] Every pinned spatial artifact that moves is re-recorded with approval (receipt: the lanes board).

**Mutants to kill.**
- [ ] A systolic shape transposed.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test rdu_unit_model_lock --test spatial_rdu_lock
```

**Done when.** The unit model's arithmetic is the chip's arithmetic.

#### CHIP-3 · The PMU, the AGCU and the networks

| Field | Value |
|---|---|
| Phase | chip |
| Labels | `infrastructure` `rdu` |
| Depends on | CHIP-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/rdu_chip.rs`, `crates/logicaffeine_compile/src/spatial/roofs.rs` |
| Status | open |

**Goal.** Model a PMU (512 KB, its banks and address pipeline, double buffering), the AGCUs that move data between
SRAM and HBM, DDR, host and peer sockets, and the on-chip and P2P networks, with roofs for each, so the time model
(TIME-1) can price every byte that moves.

**Read first.**
- `crates/logicaffeine_compile/src/codegen_sva/pmu.rs`
- `crates/logicaffeine_compile/src/spatial/roofs.rs`

**RED tests.**
- [ ] `rdu_unit_model_lock::every_byte_path_has_a_roof`: fails today: no DDR, P2P or on-chip roof.
- [ ] `rdu_unit_model_lock::a_pmu_holds_512_kb_in_its_banks`: fails today: PMU capacity is SRAM divided by the PCU count.

**Edge cases.**
- [ ] A transfer that crosses dies inside a socket.
- [ ] An AGCU count the vendor has not published: modeled, named.

**Implementation.** PMU and AGCU records in the unit model; roofs for HBM, DDR, P2P, on-chip; each cites its fact.

**Success.**
- [ ] Every path a byte can take has a roof with a cited or modeled rate (receipt: the lock).

**Mutants to kill.**
- [ ] A DDR roof set to the HBM rate.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test rdu_unit_model_lock
```

**Done when.** No byte moves in a plan without a priced path.

#### CHIP-4 · SN40L and SN50 from one fact table

| Field | Value |
|---|---|
| Phase | chip |
| Labels | `infrastructure` `rdu` |
| Depends on | CHIP-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/rdu_chip.rs`, `crates/logicaffeine_compile/src/codegen_sva/plasticine.rs` |
| Status | open |

**Goal.** Three descriptions of the same chips exist: `RduChipFacts`, `PlasticineTarget::rdu`, and the spatial target
derived from the facts. Every one is derived from the fact table, and the intake card (ACCESS-1) writes into that table
alone.

**Read first.**
- `crates/logicaffeine_compile/src/codegen_sva/plasticine.rs`: `rdu(chip)` and its provenance list.

**RED tests.**
- [ ] `sn50_target_lock::every_rdu_description_derives_from_the_fact_table`: fails today: the Plasticine preset copies
  numbers.

**Edge cases.**
- [ ] A fact one description needs that the table lacks: added to the table, never to the description.

**Implementation.** Derive `PlasticineTarget::rdu` and the spatial target from `RduChipFacts`; delete copied numbers.

**Success.**
- [ ] One table, every description derived (receipt: the lock).

**Mutants to kill.**
- [ ] A copied number reintroduced in the Plasticine preset.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test sn50_target_lock --test sn40l_target_lock --test plasticine_report_lock
```

**Done when.** A fact changes in one place.

#### CHIP-5 · One sizing for the fit, the bundle and the serving plan

| Field | Value |
|---|---|
| Phase | chip |
| Labels | `infrastructure` `rdu` |
| Depends on | CHIP-4 |
| Touches | `crates/logicaffeine_compile/src/spatial/fit.rs`, `crates/logicaffeine_tests/tests/rdu_fit_report_lock.rs`, `benchmarks/silicon/qwen38_vllm_plan.json` |
| Status | open |

**Goal.** The fit report sizes Qwen3.8-27B in BF16 at TP16 with state in SRAM; the bundle reads Q8_0 at TP8 and
streams state; the vLLM plan asks for a width of 16, which 24 heads refuse. One sizing function computes all three from
the bundle the compiler actually writes.

**Read first.**
- `crates/logicaffeine_compile/src/spatial/fit.rs`: `qwen38_fit`, `qwen38_serving_plan`, `vllm_plan`.

**RED tests.**
- [ ] `rdu_fit_report_lock::the_fit_reads_the_bundles_formats_and_width`: fails today: BF16 at TP16.
- [ ] `rdu_fit_report_lock::the_vllm_plan_uses_an_admissible_width`: fails today: 16.

**Edge cases.**
- [ ] A model whose heads admit no width above 1: the plan says so.

**Implementation.** The fit, the serving plan and the vLLM plan read the bundle's manifest; the TP width comes from
`qwen38_admits_tp` generalized to any spec.

**Success.**
- [ ] All three agree on formats, width and residency (receipt: the lock).

**Mutants to kill.**
- [ ] The vLLM plan's width hard-coded again.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test rdu_fit_report_lock
```

**Done when.** Every sizing the tree prints is the bundle's.

#### TIME-1 · A time model of a composed program

| Field | Value |
|---|---|
| Phase | time |
| Labels | `infrastructure` `rdu` `area:time` |
| Depends on | CHIP-2, CHIP-3 |
| Touches | `crates/logicaffeine_compile/src/spatial/cost.rs`, `crates/logicaffeine_compile/src/spatial/work.rs`, `crates/logicaffeine_compile/src/spatial/clock.rs`; the time model (new) |
| Status | open |

**Goal.** The spatial compiler has a work model (tiles, bytes, matmuls) and no time. Build a time model of a composed
program: each stage's initiation interval from the unit model, each FIFO's credits, each memory path's roof, each
barrier's cost, each collective's link time, simulated as a timed dataflow program in the manner of a dataflow abstract
machine [DAM24]. It reports every measure of section 2.1 (time per token, share of roofline, MBU, synchronization
boundaries) for a whole deployment.

**Read first.**
- `crates/logicaffeine_compile/src/spatial/clock.rs`: the marked-graph period it already computes.
- `crates/logicaffeine_compile/src/spatial/eval.rs`: the untimed choreography the time model times.
- The DAM paper (section 12.3).

**RED tests.**
- [ ] `spatial_time_model_lock::a_deployment_reports_time_per_token_and_mbu`: fails today: no time model.
- [ ] `spatial_time_model_lock::a_barrier_costs_what_its_participants_wait`: fails today: barriers are free.
- [ ] `spatial_time_model_lock::the_model_equals_the_marked_graph_period_on_a_steady_pipeline`: fails today.

**Edge cases.**
- [ ] A data-dependent trip count: timed at the count the inputs give, bounded at the window the certificate proves.
- [ ] A section whose FIFOs back up: the stall attributed to the producer it waits on.
- [ ] A collective across sockets: link time, overlap with compute.

**Implementation.** A discrete-event simulation over the composed program's choreography: stages as processes with an
II and a latency, buffers with credits, memory and links as servers with roofs.

**Success.**
- [ ] Every Tier A decode deployment priced, every section 2.1 measure reported (receipt: the lock).
- [ ] Equal to the marked-graph period on steady pipelines (receipt: the lock).

**Mutants to kill.**
- [ ] A barrier priced at zero.
- [ ] An HBM roof ignored.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_time_model_lock
```

**Done when.** Every plan the compiler makes has a predicted time.

#### TIME-2 · The time model calibrated on Tungsten and ttsim

| Field | Value |
|---|---|
| Phase | time |
| Labels | `infrastructure` `area:time` |
| Depends on | TIME-1, PLAST-7 |
| Touches | the time model (new); the calibration ledger (new) |
| Status | open |

**Goal.** Fit the time model's free constants on every simulated row we hold (Tungsten segments and regression apps,
ttsim GEMMs in `benchmarks/silicon/tenstorrent_gemm.json`) and hold the fit: each prediction within a stated bound of
its simulation, the bound itself recorded.

**Read first.**
- `benchmarks/silicon/plasticine_calibration.json`
- `crates/logicaffeine_compile/src/spatial/cost.rs`: `wormhole_ttsim`, the one fitted model today.

**RED tests.**
- [ ] `spatial_time_model_lock::every_simulated_row_is_within_the_bound`: fails today: no time model.

**Edge cases.**
- [ ] A row the model cannot describe: listed, never dropped from the set.
- [ ] Two simulators that disagree on one program: both kept, the bound per simulator.

**Implementation.** Least-squares fit of per-unit constants over the ledger; the bound is the maximum relative error,
recorded with the fit.

**Success.**
- [ ] The fit and its bound recorded (receipt: the calibration ledger).

**Mutants to kill.**
- [ ] A row dropped from the fit.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_time_model_lock --test plasticine_sim_lock
```

**Done when.** The time model's error is measured, not assumed.

#### TIME-3 · SN40L and SN50 in the decode pricer

| Field | Value |
|---|---|
| Phase | time |
| Labels | `infrastructure` `rdu` `area:time` |
| Depends on | TIME-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/decode.rs` |
| Status | open |

**Goal.** The decode pricer's chip catalog prices GPUs, Groq, Cerebras and our FPGA cards, and not SN40L or SN50. Add
both, priced by the time model for a deployment, not by bandwidth alone, and print the Copy bar beside every priced
cell.

**Read first.**
- `crates/logicaffeine_compile/src/spatial/decode.rs`: the catalog and `price`.

**RED tests.**
- [ ] `spatial_decode_price_lock::sn50_prices_every_tier_a_model`: fails today: no entry.

**Edge cases.**
- [ ] A model too large for the chip count asked: the pricer names the count it needs.

**Implementation.** Catalog entries from the fact table; the price calls the time model.

**Success.**
- [ ] Every Tier A model priced at D1 and TTFT-8K (receipt: the lock).

**Mutants to kill.**
- [ ] A price computed from HBM bandwidth alone.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_decode_price_lock
```

**Done when.** Any model's decode on the RDU has a predicted rate.

#### TIME-4 · A speed claim only from a calibrated row

| Field | Value |
|---|---|
| Phase | time |
| Labels | `infrastructure` `area:time` |
| Depends on | TIME-2 |
| Touches | `crates/logicaffeine_compile/src/spatial/fit.rs`, `benchmarks/silicon/qwen38_sn50.json` |
| Status | open |

**Goal.** Every RDU artifact carries `no_speed_claim`. Lift it only for rows the calibrated model predicts, with the
model's version, its calibration set and its bound beside the number; nothing else may print a speed for the RDU.

**Read first.**
- `benchmarks/silicon/qwen38_sn50.json`

**RED tests.**
- [ ] `spatial_time_model_lock::a_speed_without_calibration_is_refused`: fails today: no claim is possible, so none is
  checked.

**Edge cases.**
- [ ] A prediction outside the calibrated range of shapes: refused.

**Implementation.** A `SpeedClaim` type that can only be built from a calibrated prediction; artifacts take it.

**Success.**
- [ ] Every printed RDU speed carries its provenance (receipt: the lock).

**Mutants to kill.**
- [ ] A speed printed from a raw float.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_time_model_lock --test rdu_fit_report_lock
```

**Done when.** A speed on the RDU is always a calibrated prediction or a measurement.

#### TIME-5 · Predicted against measured, the day hardware answers

| Field | Value |
|---|---|
| Phase | time |
| Labels | `infrastructure` `rdu` `area:time` |
| Depends on | TIME-2, ACCESS-1 |
| Touches | the calibration ledger (new) |
| Status | open |

**Goal.** When any measurement from the vendor's hardware or simulator arrives, it joins the calibration set with its
provenance, the model refits, and every cell's prediction is re-recorded beside its measurement.

**Read first.**
- Section 3 of this book.

**RED tests.**
- [ ] `spatial_time_model_lock::a_measured_row_refits_and_re_predicts`: fails today: no measured rows exist; the test
  uses a recorded synthetic row marked as such.

**Edge cases.**
- [ ] A measurement far outside the bound: the model is held wrong until explained (CLIMB-5).

**Implementation.** The calibration ledger accepts `measured` rows; the refit is the same as TIME-2's.

**Success.**
- [ ] The path from a measured row to re-predicted cells runs (receipt: the lock).

**Mutants to kill.**
- [ ] A measured row ignored by the refit.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_time_model_lock
```

**Done when.** The first hardware number improves every prediction.

#### TIME-6 · Where every cycle of a step goes

| Field | Value |
|---|---|
| Phase | time |
| Labels | `infrastructure` `area:time` |
| Depends on | TIME-1 |
| Touches | the time model (new) |
| Status | open |

**Goal.** The time model attributes each step's time to causes: HBM and DDR stalls, barriers, idle compute and memory
units, routing and virtual-channel stalls, collectives, host round trips; and each section's PCU, PMU and SRAM use.
This is the attribution the gap miner (CLIMB-2) and the gap rule (section 2.5) read.

**Read first.**
- Section 9 of this book.

**RED tests.**
- [ ] `spatial_time_model_lock::a_steps_attribution_sums_to_its_time`: fails today.
- [ ] `spatial_time_model_lock::every_section_reports_its_unit_use`: fails today.

**Edge cases.**
- [ ] Overlapped causes: attributed to the critical path, the overlap reported.

**Implementation.** Critical-path accounting over the time model's event trace.

**Success.**
- [ ] Every Tier A deployment's step attributed (receipt: the lock).

**Mutants to kill.**
- [ ] An attribution that double-counts an overlap.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_time_model_lock
```

**Done when.** Every lost cycle has a named cause.
### 11.8 Intake and the author

A bring-up test hands over a checkpoint as its maintainers publish it. These tickets read any of them and write its
one program. `crates/logicaffeine_compile/src/model/spec.rs` and `crates/logicaffeine_compile/src/model/author.rs` are the fusion book's S12 files: these tickets
are worked in that seat.

#### INTAKE-1 · Hugging Face checkpoints read in place

| Field | Value |
|---|---|
| Phase | intake |
| Labels | `infrastructure` `area:intake` |
| Depends on | none |
| Touches | `crates/logicaffeine_compile/src/model/gguf.rs`; the Hugging Face and safetensors readers (new); `crates/logicaffeine_compile/src/model/spec.rs` through S12 |
| Status | open |

**Goal.** Today a model enters only as GGUF; there is no `config.json` or safetensors reader in Rust, and a
Hugging-Face-only drop needs llama.cpp's converter first. Read a checkpoint directory as published: `config.json`
(including nested `text_config`), the safetensors index and shards (memory-mapped, sharded, any dtype they carry), and
the tokenizer files, into the same `ModelSpec` and weight source the GGUF path yields.

**Read first.**
- `crates/logicaffeine_compile/src/model/spec.rs`: `SpecSource` and `from_source`.
- `crates/logicaffeine_compile/src/model/gguf.rs`: the source the new reader mirrors.

**RED tests.**
- [x] `model_hf_intake_lock::a_llama_directory_yields_the_spec_its_gguf_yields`: fails today: no reader (RED receipt: `logs/intake/red-1.log`)
- [x] `model_hf_intake_lock::sharded_safetensors_bind_every_tensor`: fails today: no reader (RED receipt: `logs/intake/red-1.log`)
- [x] `model_hf_intake_lock::a_nested_text_config_is_read`: fails today: no reader (RED receipt: `logs/intake/red-1.log`)

**Edge cases.**
- [x] A tensor stored transposed or split differently from GGUF's layout (fused gate-up, fused QKV): bound by name rules
  per family, never by position (receipt: gpt-oss's interleaved, transposed gate-up and DeepSeek-V3's split latent key
  and value in `a_gpt_oss_directory_reads_as_its_converted_gguf` and `a_deepseek_v3_directory_reads_as_its_converted_gguf`,
  Qwen3.5's fused QKV with its value heads reordered in `a_qwen35_directory_reads_as_its_converted_gguf`).
- [x] BF16, F16, F32 and FP8 tensors in one checkpoint (receipt: `a_checkpoint_of_mixed_dtypes_reads_as_its_converted_gguf`,
  both 8-bit floats with every code; their block-scaled formats are INTAKE-3's).
- [x] A shard index that names a missing shard: refused by name (receipt: `a_missing_shard_is_refused_by_name`).
- [x] A 400 GB checkpoint: memory-mapped, never read whole (receipt:
  `a_checkpoint_of_hundreds_of_gigabytes_is_never_read_whole`: an 812 GB Llama-3.1-405B-shaped directory in eight
  sparse shards opens and serves rows of its first and last layers while the process reads under 16 MB; the reader
  reads by position, which needs no mapping).

**Implementation.** A `HfSource: SpecSource` over the configuration and a safetensors reader; per-family tensor-name
maps beside each spec arm; the tokenizer through the existing tokenizer module. Built as `model::hf::HfCheckpoint`
(presenting the directory as the GGUF the pinned converter writes from it), `model::safetensors`, the converter's
tokenizer keys read by `tokenizer::Vocab::from_meta`, and `model::hf::HfTokenizer`, a port of the `tokenizers`
library's encode that names the byte-pair pre-tokenizer by the converter's own probe digest.

**Success.**
- [x] Every family the spec reads reads from its Hugging Face directory into the spec its GGUF yields, in each dtype a
  checkpoint ships unquantized; INTAKE-2 brings the rest of Tier A to the spec and INTAKE-3 the block-quantized
  formats (receipt: `logs/intake/final-1.log`, 42/42 with every tool required: seven families' twins in bf16, f16 and
  mixed dtypes, and the published Qwen3.5-0.8B, gemma-3-1b-it and Llama-3.2-1B-Instruct, keys, values and tokenizer
  keys).
- [x] The tiny goldens agree from either source (receipt: the reference's logits from a directory are its converted
  GGUF's, to the bit for every twin and for gemma-3-1b-it and Llama-3.2-1B-Instruct, and for Qwen3.5-0.8B, whose
  converter's decay exponential is a unit in the last place off in places, the same greedy token at every position:
  `the_reference_reads_a_hugging_face_directory_as_its_gguf`, `the_reference_reads_the_published_checkpoints_as_their_ggufs`).

**Mutants to kill.**
- [x] A name map that swaps gate and up (receipt: `logs/intake/mutants/results.json`, gate-up-swap and
  expert-gate-up-swap killed).
- [x] A shard skipped (receipt: `logs/intake/mutants/results.json`, shard-skipped and index-shard-dropped killed).

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test model_hf_intake_lock --test model_golden_lock
```

**Done when.** A checkpoint is read as its maintainers publish it.

#### INTAKE-2 · Every architecture string a test can send

| Field | Value |
|---|---|
| Phase | intake |
| Labels | `infrastructure` `area:intake` |
| Depends on | INTAKE-1 |
| Touches | `crates/logicaffeine_compile/src/model/spec.rs` and `crates/logicaffeine_compile/src/model/bind.rs` through S12; `crates/logicaffeine_tests/tests/common/tiny_models.rs` |
| Status | open |

**Goal.** Seven arms today; anything else is `UnknownArchitecture`, and Mixtral is silently read as dense. Add an arm
for every architecture section 6 lists (qwen2, qwen2_moe, qwen3_next, qwen3_5_moe, qwen4_exp, mistral, mixtral, llama4,
phi3, phimoe, gemma2, gemma3n, glm4, glm4_moe, glm5, deepseek_v3, deepseek_v32, kimi, minimax, jamba, falcon_h1,
nemotron_h, mamba2, rwkv7, lfm2), each mapping its configuration keys onto the spec. A key an arm does not understand is
refused by name.

**Read first.**
- `crates/logicaffeine_compile/src/model/spec.rs`: the arms of `from_source`.
- Each family's converter under `external/competitors/llama.cpp/conversion/` and its vLLM model file.

**RED tests.**
- [ ] `model_hf_intake_lock::every_section_6_architecture_has_an_arm`: fails today: 7 of them.
- [ ] `model_hf_intake_lock::mixtral_is_read_as_experts`: fails today: read as dense.
- [ ] `model_hf_intake_lock::an_unknown_key_is_refused_by_name`: fails today: keys are read selectively, the rest
  ignored.
- [x] `model_hf_intake_lock::a_qwen2moe_directory_reads_as_its_converted_gguf` and
  `model_tiny_checkpoint_lock::a_qwen2moe_checkpoint_reads_back_as_its_spec`, `…::a_qwen2moe_checkpoint_reads_llama_cpps_widths`:
  the `qwen2moe` arm (llama.cpp `models/qwen2moe.cpp`: a softmax router whose kept weights stay unrenormalized, a shared
  expert scaled by its sigmoid gate; unnamed or zero widths as llama.cpp defaults them, a per-block width array read at
  block 0); failed: unknown architecture (`logs/samba/red-q2moe.log`) (receipt: `logs/samba/green-q2moe.log`).
- [x] `model_hf_intake_lock::a_qwen35moe_directory_reads_as_its_converted_gguf` and
  `model_tiny_checkpoint_lock::a_qwen35moe_checkpoint_reads_back_as_its_spec`: the `qwen35moe` arm (llama.cpp
  `models/qwen35moe.cpp`: Qwen3.5's hybrid with Qwen's mixture in every block, the kept weights renormalized, one
  mixture builder for both Qwen arms) and `Qwen3_5MoeForConditionalGeneration`, whose draft block stores its experts as one
  gate-and-up tensor the converter splits; failed: unknown architecture (`logs/samba/red-q35moe.log`) (receipt: `logs/samba/green-q35moe.log`). The author and
  the chip held the shape before the arm existed (`model_conformance_lock::qwen35_moe_prefills_and_decodes_to_the_references_logits`,
  `spatial_moe_routing_lock::a_qwen35_moe_step_equals_the_vm`, both passing in `logs/samba/red-q35moe.log`).
- [x] `model_hf_intake_lock::every_published_configuration_reads`: the published `config.json` of every release the book
  deploys (`scripts/accel/fetch-hf-configs.sh`, gated repositories through their public mirrors) reads without its
  weights (`HfCheckpoint::read_configuration`) as its family's architecture at its stated width and stack; the twins,
  written by transformers' own configuration classes, had never carried the keys releases publish: `auto_map`,
  training settings, transformers 4's serialized defaults (three of which change the model and are refused unless at
  their defaults), DeepSeek's `moe_layer_freq` (refused unless 1), gpt-oss's own names for two settings (held to the
  converter's), Qwen3-Next's `use_sliding_window`; failed: DeepSeek-V3 and R1, Kimi K2, MiniMax-M2, gpt-oss and
  Qwen3-Next refused (`logs/samba/red-published-configs.log`) (receipt: `logs/samba/green-kimi-vocab.log`).
- [x] `model_hf_intake_lock::a_gemma_directory_reads_as_its_converted_gguf`, `…::a_gemma2_directory_reads_as_its_converted_gguf`,
  `…::a_gemma_directory_computes_what_transformers_computes`, `…::a_gemma2_directory_computes_what_transformers_computes`,
  `…::a_gemma_scale_the_gguf_cannot_carry_is_refused`, `…::every_gemma2_setting_the_gguf_cannot_carry_is_refused_by_name`,
  `…::a_gemma2_rotation_base_reads_as_the_configuration_says`: the `gemma` and `gemma2` arms (llama.cpp `models/gemma.cpp`
  and `models/gemma2.cpp`: Gemma 2's window on every other block from the first, its scores and logits capped, 50 and 30
  where unwritten, the score scale fixed by the stack's depth) and `GemmaForCausalLM` and `Gemma2ForCausalLM`, whose
  converters drop a stored head; a configuration the GGUF cannot carry is refused by name (a `query_pre_attn_scalar`
  other than the scale llama.cpp fixes, for Gemma 3 too; an unset cap; a second window; another alternation), and
  Gemma 2's rotation base, which the converter leaves out, is written as the configuration states it. Against
  transformers at f64 (its attention eager, the one form that takes the cap, every softmax kept at f64) the window and
  both caps bind on the twin; failed: the architectures unread (`logs/samba/red-gemma-head.log`) (receipt:
  `logs/samba/green-gemma-4.log`).
- [x] `model_hf_intake_lock::a_multimodal_checkpoints_language_model_reads_as_its_converted_gguf` and
  `…::llama4_experts_stored_one_linear_each_are_refused_by_name`: Llama 4 and Gemma 3's larger sizes are published only
  as multimodal checkpoints (`…ForConditionalGeneration`), whose language model the converter reads with the vision
  tower and its projector skipped. The reader finds the text model under either prefix transformers has written
  (`model.language_model.`, `language_model.model.`) and its head beside it (`hf-tiny-twin.py gemma3-mm`, `llama4-mm`);
  the handed RED `model_hf_config_reds_lock::the_multimodal_checkpoints_published_configurations_read` moved into
  `every_published_configuration_reads` (Scout, Maverick, Gemma 3 4B, 12B and 27B), Llama 4's
  `for_llm_compressor` read (true, experts stored one linear each, refused by name); failed: the wrappers unread
  (`logs/samba/red-multimodal-head.log`) (receipt: `logs/samba/green-multimodal-3.log`).
- [x] `model_hf_intake_lock::a_tiktoken_vocabulary_encodes_as_tiktoken_encodes`,
  `…::every_class_of_kimi_k2s_pattern_matches_what_tiktoken_matches`, `…::kimi_k2s_probe_names_its_pre_tokenizer` and
  `…::kimi_k2s_vocabulary_is_its_published_ggufs`: Kimi K2's tiktoken vocabulary (`model::hf::Tiktoken`: the ranks, the
  pattern and reserved count read out of its tokenizer code's text, its encode's cuts, `tiktoken`'s byte-pair merge)
  encodes as the `tiktoken` library does over the corpus and a Unicode sweep; each class of its pattern matches what
  tiktoken matches over every codepoint (the Han script from Unicode 16.0's Scripts.txt, `&&` intersections and nested
  classes, read in `tiktoken`'s syntax only); the probe names `kimi-k2`; and the vocabulary the converter's Kimi path
  writes (each token in GPT-2's byte alphabet, each merge by byte-pair merging below its rank) is, key for key, the one
  in the published GGUF converted from the same files (its head only, `scripts/accel/fetch-gguf-heads.sh`): the
  converter runs Kimi's tokenizer code, the intake never does; failed: no reader (`logs/samba/red-kimi-vocab.log`)
  (receipt: `logs/samba/green-kimi-2.log`).
- [x] `model_hf_intake_lock::a_glm4moe_directory_reads_as_its_converted_gguf`, `…::a_glm45_directorys_tokenizer_reads_as_its_converted_gguf`
  and `model_tiny_checkpoint_lock::a_glm4moe_checkpoint_reads_back_as_its_spec`: the `glm4moe` arm (llama.cpp
  `models/glm4-moe.cpp`: NEOX rotation over half of each head, biased projections, per-head QK norms where the
  checkpoint carries them, dense lead blocks, then DeepSeek's mixture, which the `deepseek2` arm now shares) and
  `Glm4MoeForCausalLM`, whose draft block after the stack (`model.layers.<n>.*`) is presented as the converter writes
  it (`blk.<n>.nextn.*`); `model::layout` presents the FFN norm the checkpoint stores as `post_attention_norm` as
  `ffn_norm`, and the spec refuses a checkpoint read raw; the vocabulary takes `_set_vocab_glm`'s special tokens.
  `deploy_kimi_glm_lock` brings GLM-4.5's twin up on the SN50 and lowers GLM-4.5, 4.6, 4.7 and 4.5-Air; failed:
  unknown architecture, unknown reader family (`logs/samba/red-glm45.log`) (receipt: `logs/samba/green-glm45.log`).
- [x] `model_hf_intake_lock::a_<family>_directorys_tokenizer_reads_as_its_converted_gguf` for Qwen 2.5, Mixtral,
  MiniMax-M2, Phi-4 and GLM-4, the families read since the parity lock was written: GLM-4's vocabulary missed the
  special tokens its converter names by text (`Glm4Model.set_vocab`), now read (`logs/samba/red-tokenizer-parity.log`)
  (receipt: `logs/samba/green-glm45.log`).
- [x] `model_hf_intake_lock::a_rotation_share_and_original_context_are_read_from_the_rope_parameters_first`: the
  Phi-3, GLM-4 and GLM-4.5 converters read `partial_rotary_factor` (and Phi-3's `original_max_position_embeddings`)
  from the rope parameters before the configuration's own key, and Phi-3's fails where neither names the original
  context; failed: the configuration's key was read first (`logs/samba/red-glm45.log`) (receipt:
  `logs/samba/green-glm45-rope.log`).
- [x] `model_hf_intake_lock::a_glm4_directory_reads_as_its_converted_gguf` and
  `model_tiny_checkpoint_lock::a_glm4_checkpoint_reads_back_as_its_spec`: the `glm4` arm (llama.cpp `models/glm4.cpp`:
  interleaved rotation over `rope.dimension_count`, half of each head; the projections biased where the checkpoint
  carries biases; norms on the attention's and the FFN's outputs before each residual add) and `Glm4ForCausalLM`;
  `model::layout` presents the stored `ffn_up` as `ffn_gate` and `ffn_up` as it does Phi-3's, and the spec refuses a
  checkpoint read raw and GLM-4V's multimodal rotation by name. `deploy_glm_lock` brings GLM-4's twin up on the SN50
  and lowers both 0414 sizes (the 9B's projections biased, the 32B's not); failed: unknown architecture, unknown reader
  family (`logs/samba/red-glm.log`) (receipt: `logs/samba/green-glm.log`).
- [x] `model_hf_intake_lock::a_phi3_directory_reads_as_its_converted_gguf` and
  `model_tiny_checkpoint_lock::a_phi3_checkpoint_reads_back_as_its_spec`: the `phi3` arm (llama.cpp `models/phi3.cpp`:
  NEOX rotation over `rope.dimension_count`, the query, key and value one projection or three, a window where
  `attention.sliding_window` is short of the context) and `Phi3ForCausalLM`; `model::layout` presents the stored
  `ffn_up` (the gate's rows, then the up's) as `ffn_gate` and `ffn_up`, and the spec refuses a checkpoint read raw and
  LongRoPE's factor tables by name. `deploy_phi_lock` brings Phi-4's twin up on the SN50 and lowers its 14B shape;
  failed: unknown architecture, unknown reader family (`logs/samba/red-phi.log`) (receipt: `logs/samba/green-phi.log`).
- [x] `model_hf_intake_lock::a_minimax_m2_directory_reads_as_its_converted_gguf` and
  `model_tiny_checkpoint_lock::a_minimax_m2_checkpoint_reads_back_as_its_spec`: the `minimax-m2` arm (llama.cpp
  `models/minimax-m2.cpp`: a partial NEOX rotation, the query and key each normed across every head at once, read off
  `attn_q_norm`'s width, and experts scored as `expert_gating_func` says, chosen by score plus `exp_probs_b`, the kept
  weights renormalized) and `MiniMaxM2ForCausalLM`, whose linear-attention layers, scaled residuals and shared expert
  (MiniMax-01's) are refused by name; `model_conformance_lock::a_qk_norm_across_every_head_equals_the_reference` holds
  the author's norm to the reference, and splitting it over chips is refused; failed: no field to say it and no arm
  (`logs/samba/red-minimax-compile.log`) (receipt: `logs/samba/green-minimax.log`).
- [x] `model_hf_intake_lock::a_qwen3next_directory_reads_as_its_converted_gguf`: the `qwen3next` arm (llama.cpp
  `models/qwen3next.cpp`: Qwen3.5-MoE's blocks, its value heads grouped by key head and each key head's betas and alphas
  one projection, `ssm_ba`, as the converter writes them) and `Qwen3NextForCausalLM`; `model::open_checkpoint` presents
  the grouped layout in Qwen3.5's tiled one (`model::layout`: tiled value head `t` is grouped head `(t mod k)·r + ⌊t/k⌋`,
  `ssm_ba` split into `ssm_beta` and `ssm_alpha`), and the spec refuses a checkpoint read raw, naming `ssm_ba`; failed:
  `Qwen3NextForCausalLM` is not one the reader presents (`logs/samba/red-q3next.log`) (receipt: `logs/samba/green-q3next-2.log`).
- [x] `model_tiny_checkpoint_lock::a_routed_scale_llama_cpp_never_reads_is_not_read`: llama.cpp reads
  `expert_weights_scale` only on DeepSeek-family graphs; the llama, qwen3moe, qwen2moe, gpt-oss and llama4 arms leave the
  kept weights unscaled whatever the checkpoint carries; failed: the scale was read (`logs/samba/red-q2moe.log`) (receipt: `logs/samba/green-q2moe.log`).
- [x] `model_hf_intake_lock::a_renormalization_the_gguf_cannot_say_is_refused`: a Qwen2-MoE configuration that
  renormalizes or a Qwen3-MoE one that does not is refused by name (llama.cpp fixes it by architecture and the converter
  writes no key); failed: no Qwen2-MoE directory was read at all (`logs/samba/red-q2moe.log`) (receipt: `logs/samba/green-q2moe.log`).
- [x] `model_hf_intake_lock::a_directorys_vocabulary_is_read_by_its_converters_rule`: a SentencePiece model is read only
  for Qwen2 and Qwen3, as the converter's classes read it; failed: Qwen2's SentencePiece model was ignored and its
  tokenizer.json read in its place (`logs/samba/red-q2moe.log`) (receipt: `logs/samba/green-q2moe.log`).

**Edge cases.**
- [ ] A feature an arm reads that the author cannot write yet: the arm reads it, the author refuses it by name
  (INTAKE-9).
- [ ] Two releases of one family with different key names.

**Implementation.** One arm per family, a tiny configuration per family in the shared fixture, a known-keys list per
arm.

**Success.**
- [ ] Every arm reads its tiny configuration (receipt: the lock).

**Mutants to kill.**
- [ ] A key removed from an arm's known list.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test model_hf_intake_lock --test model_reference_lock
```

**Done when.** No architecture a test can send is unknown.

#### INTAKE-3 · fp8 blocks, NVFP4, AWQ and GPTQ read

| Field | Value |
|---|---|
| Phase | intake |
| Labels | `infrastructure` `area:intake` `area:quant` |
| Depends on | INTAKE-1 |
| Touches | `crates/logicaffeine_compile/src/model/quant.rs` |
| Status | open |

**Goal.** The format reader covers the GGUF block formats and MXFP4. Read the formats published checkpoints use: FP8
e4m3 with 128×128 block scales (DeepSeek-V3, Llama 4 FP8), NVFP4, AWQ and GPTQ 4-bit with group scales and zero points,
and the remaining GGUF IQ formats.

**Read first.**
- `crates/logicaffeine_compile/src/model/quant.rs`
- `crates/logicaffeine_tests/tests/model_quant_lock.rs`

**RED tests.**
- [x] `model_quant_lock::fp8_block_scaled_dequantizes_as_the_reference` and `…::a_scale_the_converter_would_not_broadcast_is_refused`:
  `quant::ScaleGrid` widens FP8 (e4m3fn, e5m2) and bf16 weights times their scales as the converter's own
  `dequant_simple` (run out of the pinned `conversion/base.py`) does, bit for bit, over every code (subnormals and NaNs
  among them), 128×128 blocks, blocks that divide neither axis (the scale cropped), a stack of experts, bf16 scales, and
  per-row and whole-tensor scales; failed: no reader (`logs/samba/red-fp8-quant.log`) (receipt:
  `logs/samba/green-fp8-quant.log`).
- [x] `model_hf_intake_lock::an_fp8_block_scaled_directory_reads_as_its_converted_gguf` and
  `…::a_quantization_the_reader_does_not_read_is_refused_by_name`: a directory whose `quantization_config` is FP8
  (DeepSeek-V3's release, here its twin with every projection in e4m3 beside a float32 scale per 16 × 24 block) reads in
  place, each weight beside a `_scale_inv` read through `ScaleGrid` under every view the reader presents (the latent
  attention's split, the experts' stack), the scales consumed; any other quantized format is refused naming it; failed:
  `quantization_config` an unknown key (`logs/samba/red-fp8-intake.log`) (receipt: `logs/samba/green-fp8.log`).
- [ ] `model_quant_lock::nvfp4_dequantizes_as_the_reference`: fails today: no reader.
- [ ] `model_quant_lock::awq_and_gptq_dequantize_as_the_reference`: fails today: no reader.
- [ ] `model_hf_config_reds_lock::gpt_oss_published_configurations_read` and
  `…::kimi_k2_thinking_published_configuration_reads`: handed REDs: gpt-oss's MXFP4 experts and Kimi K2 Thinking's int4
  compressed-tensors experts are refused by name.

**Edge cases.**
- [ ] FP8 subnormals, NaN encodings and saturation.
- [ ] A group size that does not divide a row.

**Implementation.** One reader per format, each checked bit for bit against its reference implementation's dequantizer.

**Success.**
- [ ] Every format dequantizes bit for bit (receipt: the lock).

**Mutants to kill.**
- [ ] An exponent bias off by one.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test model_quant_lock
```

**Done when.** Every published weight format is read in place.

#### INTAKE-4 · Goldens from the reference implementations

| Field | Value |
|---|---|
| Phase | intake |
| Labels | `infrastructure` `area:intake` |
| Depends on | INTAKE-1 |
| Touches | `scripts/accel/model-golden.sh`, `benchmarks/models/`, `crates/logicaffeine_tests/tests/common/tiny_models.rs` |
| Status | open |

**Goal.** Goldens today come from llama.cpp. For every family, record goldens from its reference implementation
(transformers on the CPU, and vLLM's and llama.cpp's where they exist) for a tiny random configuration built from the
family's real configuration: logits for a fixed prompt set, each intermediate a reference exposes, and the reference's
own top-1 agreement with f32.

**Read first.**
- `scripts/accel/model-golden.sh`
- `crates/logicaffeine_tests/tests/model_golden_lock.rs`

**RED tests.**
- [ ] `model_golden_lock::every_family_has_a_transformers_golden`: fails today: llama.cpp goldens only.
- [ ] `model_hf_intake_lock::the_reference_computes_what_transformers_computes`: transformers' own forward at f64 over the
  same weights (`scripts/accel/hf-oracle.py logits`, experts run eagerly), live rather than recorded, for Qwen3-Next,
  Qwen3.5, Qwen3.5-MoE, Qwen3-MoE and Qwen2-MoE twins: the f64 reference's last-position logits within a millionth of
  the largest. The converter-and-reader comparison cannot see a misreading both share; this can. Failed: Qwen3-Next
  unread (`logs/samba/red-q3next.log`). Holds at six value heads over two (`logs/samba/green-q3next-2.log`); at Qwen3.5's
  four, a logit lands 6.4e-6 from transformers' at a largest of 3.87 (`logs/samba/verify-push-2.log`): the reader
  presents the converter's f32-rounded values (a norm's `1 + w`, `-exp(A_log)`), and a millionth is tighter than that
  rounding. A misread layout moves a logit by its own size. The bound is a ruling owed. The recorded goldens remain owed.

**Edge cases.**
- [ ] Two references that disagree beyond rounding: both recorded, the disagreement reported.
- [ ] A family transformers does not ship: vLLM's or llama.cpp's reference, named.

**Implementation.** A script that builds the tiny configuration, seeds weights, runs each reference and writes goldens
with their provenance.

**Success.**
- [ ] Every section 6 family has goldens with provenance (receipt: `benchmarks/models/`).

**Mutants to kill.**
- [ ] A golden recorded from the wrong seed.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test model_golden_lock --test model_conformance_lock
```

**Done when.** Every family has a reference to be held to.

#### INTAKE-5 · A configuration-space fuzzer

| Field | Value |
|---|---|
| Phase | intake |
| Labels | `infrastructure` `area:intake` |
| Depends on | INTAKE-4 |
| Touches | the configuration fuzzer (new) |
| Status | open |

**Goal.** An evaluator's unseen configuration is a point in a family's configuration space. Draw legal random points
(widths, head counts and dims, KV heads, layer counts and patterns, vocabularies, expert counts and top-k, rotation
variants, biases, tying) for every family, author each, and hold the VM equal to the reference on each.

**Read first.**
- Section 10.1 of this book.

**RED tests.**
- [ ] `model_config_fuzz_lock::a_thousand_random_configurations_equal_their_references`: fails today: no fuzzer.

**Edge cases.**
- [ ] Every edge listed in section 10.1 is in the generator's distribution with its own weight.
- [ ] A failing draw is shrunk to a minimal configuration and frozen as a RED.

**Implementation.** A seeded generator per family; the reference run per draw; shrinking on failure.

**Success.**
- [ ] The fuzzer's nightly count green (receipt: the lock and its seed log).

**Mutants to kill.**
- [ ] A generator that never draws one KV head.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test model_config_fuzz_lock
```

**Done when.** An unseen configuration is one we have already drawn.

#### INTAKE-6 · Any architecture through the RDU emitter

| Field | Value |
|---|---|
| Phase | intake |
| Labels | `infrastructure` `rdu` `area:intake` |
| Depends on | INTAKE-2, FUSIONS:ONE-PROGRAM-RETIRE-SECOND-WRITERS |
| Touches | `crates/logicaffeine_compile/src/spatial/streamed.rs` through R7; `crates/logicaffeine_compile/src/compile.rs`'s RDU bundle; `crates/logicaffeine_compile/src/model/mod.rs` (`Checkpoint`, `open_checkpoint`); `apps/logicaffeine_cli/src/commands/emit.rs` and `crates/logicaffeine_compile/src/bringup.rs` through S2; `crates/logicaffeine_tests/tests/rdu_bringup_lock.rs` (new) |
| Status | open |

**Goal.** `largo emit rdu --model` takes only Qwen3.5 and Qwen3.8, through `Qwen38Cfg::from_spec`. Take any spec the
author writes: the deployment's kind programs are the program's distinct layer kinds, found by digest, not by a
Qwen-specific enumeration; `--model` accepts a Hugging Face directory or a GGUF; and a weight is read as the
checkpoint stores it, its Q8_0 planes folded on the chip only where it is stored Q8_0.

**Read first.**
- `crates/logicaffeine_compile/src/spatial/streamed.rs`
- `crates/logicaffeine_compile/src/codegen_cuda/qwen38.rs`: `Qwen38Cfg::from_spec`.
- The fusion book's ONE-PROGRAM-RETIRE-SECOND-WRITERS.

**RED tests.**
- [x] `rdu_bringup_lock::kind_programs_are_found_by_digest`: failed (no grouping by program; the API did not exist)
  (receipt: `logs/samba/intake6-run-6.log`).
- [x] `rdu_bringup_lock::two_layers_that_differ_in_one_op_are_two_kinds` (receipt: `logs/samba/intake6-run-6.log`).
- [x] `rdu_bringup_lock::every_architecture_the_spatial_lowering_takes_brings_up_on_the_sn50`: Qwen3, Llama, a tied
  Llama, Qwen3-MoE and Qwen3.5 from their checkpoints alone, the logits within the bf16 datapath's bound; failed before
  (every architecture but Qwen3.5 refused by `Qwen38Cfg::from_spec`, `logs/samba/intake6-red.log`) (receipt:
  `logs/samba/intake6-run-6.log`).
- [x] `rdu_bringup_lock::every_tiny_architecture_brings_up_on_the_sn50` (handed RED, frozen, moved green unchanged):
  failed (Gemma 3 and gpt-oss attend a window the sections did not lower, gpt-oss's expert mixture did not lower, and
  DeepSeek-V2's latent attention needed more accumulator tiles than a PCU has; `logs/samba/reds-after-intake6.log`)
  (receipt: `logs/samba/reds-mla-1.log`; ARCH-WINDOW, ARCH-SINKS, ARCH-OSS, ARCH-MLA).
- [ ] `rdu_bringup_reds_lock::every_tiny_architecture_four_times_as_wide_brings_up_on_the_sn50` (handed RED, frozen):
  every architecture but Qwen3-MoE brings up; its mixture drifts 8.1% of the largest logit against a bound of 4%
  (`logs/samba/reds-mla-1.log`). Traced: the bf16 chip's mixture is the f64 mixture of its own input within 0.3%, and
  its fp32 twin is the reference's within 1.2% at the logits; the bf16 hidden state's rounding (0.5% a layer) is what
  each router amplifies about threefold. NUM-1's fp32 islands (norms, residual and router input) close it; the SN50's
  FP32 is chip-supported and witness-pending in `spatial/rdu_chip.rs`, so the islands wait on its witness.

**Edge cases.**
- [x] A model whose every layer differs (no two share a digest): one program a layer
  (`rdu_bringup_lock::a_model_whose_every_layer_differs_is_one_program_a_layer`).
- [x] Interleaved dense and expert layers (Llama 4): two kinds plus the attention variants
  (`rdu_bringup_lock::interleaved_dense_and_expert_layers_are_their_own_kinds`).
- [x] A weight the author declares Q8_0 that the checkpoint stores another way (a Gated DeltaNet's `ssm_beta` in f32):
  read dense; Qwen3.5 four times as wide brings up (`rdu_bringup_lock::every_dense_architecture_the_spatial_lowering_takes_brings_up_four_times_as_wide`).
- [x] One weight stored Q8_0 in one layer and another way in the next: two kinds
  (`rdu_bringup_lock::layers_that_read_a_weight_differently_are_two_kinds`).
- [x] A rotary table named by its rotation (`Rcos1`) is every layer's, never renamed as a layer's tensor
  (`rdu_bringup_lock::a_rotary_table_is_every_layers_and_a_weight_its_own`).

**Implementation.** Group the authored layers by the digest of their program up to its names, each name replaced by the
order of its first appearance and whether it is read as stored Q8_0 (`streamed::layer_kinds`), and bind each layer by
the renaming its program's tokens align to (`streamed::renaming`); emit one kind program per group; the manifest binds each layer's weights. The checkpoint is
a `model::Checkpoint` (its metadata and its tensors); `streamed::Storage` says which declared Q8_0 weights it stores
Q8_0 (`model::bind::stored_as_q8_planes`), from its description of each tensor; the rotary caches are each rotation's
`rope_tables`, over the checkpoint's frequency factors where the rotation reads them.

**Success.**
- [ ] Every Tier A family emits a certified bundle whose tiny form equals the reference (receipt: the lock).

**Mutants to kill.**
- [ ] Two kinds merged that differ in one op.
- [ ] A weight stored in f32 read as Q8_0 planes.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test rdu_bringup_lock --test emit_rdu_e2e --test one_program_lock
```

**Done when.** The RDU takes any model the author writes.

#### INTAKE-7 · The bring-up clock

| Field | Value |
|---|---|
| Phase | intake |
| Labels | `infrastructure` `area:intake` |
| Depends on | INTAKE-6, SCORE-1 |
| Touches | `apps/logicaffeine_cli/src/commands/bringup.rs` through S2 |
| Status | open |

**Goal.** Measure bring-up as a test would: from receiving a checkpoint to the first token from the RDU bundle that
equals the reference. The clock records each step (read, author, golden, emit, certify, run) and its wall time, and
writes a BRINGUP cell row.

**Read first.**
- `apps/logicaffeine_cli/src/commands/bringup.rs`

**RED tests.**
- [ ] `bringup_lock::the_bring_up_clock_records_every_step`: fails today: no clock.

**Edge cases.**
- [ ] A step that needs a code change: the clock records the wall time of the change, measured by the commits.

**Implementation.** Timestamps per step in `largo bringup`; a cell row per run.

**Success.**
- [ ] Every Tier A model's clock recorded (receipt: the cell ledger).

**Mutants to kill.**
- [ ] A step's time not counted.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test bringup_lock
```

**Done when.** How long a bring-up takes is a number.

#### INTAKE-8 · An exported PyTorch graph as a model

| Field | Value |
|---|---|
| Phase | intake |
| Labels | `infrastructure` `area:intake` |
| Depends on | INTAKE-2 |
| Touches | the graph importer (new); `crates/logicaffeine_compile/src/model/author.rs` through S12 |
| Status | open |

**Goal.** A test can hand over a PyTorch module, as the Plasticine kit does (`get_model_info`: the model, dummy inputs,
an optional loss and its arguments [PTUT]). Export it (`torch.export`) to an ATen graph, map each ATen op to the one IR's
primitives, refuse an unmapped op by name, and enter it through `model::author` as a new spec kind so one writer of
models still holds. **Ruling owed (Tristen):** that an imported graph enters as a spec kind written by the author.

**Read first.**
- The kit's `get_model_info` contract (section 2.1).
- `crates/logicaffeine_compile/src/tensor_graph/front.rs`: the primitives an ATen op maps to.

**RED tests.**
- [ ] `model_graph_import_lock::resnet50_imports_and_equals_pytorch`: fails today: no importer.
- [ ] `model_graph_import_lock::a_get_model_info_module_with_its_own_loss_imports`: fails today.
- [ ] `model_graph_import_lock::an_unmapped_aten_op_is_refused_by_name`: fails today.

**Edge cases.**
- [ ] Dynamic shapes in the export: the batch dimension symbolic, the rest fixed.
- [ ] A loss function: imported as the program's last ops; a missing loss replaced as the kit replaces it.
- [ ] In-place ops and views.

**Implementation.** An exporter script writes the ATen graph and weights; a Rust importer maps them; the author writes
the program from the imported spec kind.

**Success.**
- [ ] ResNet-50, SqueezeNet and a decoder layer import and equal PyTorch on the CPU (receipt: the lock).

**Mutants to kill.**
- [ ] An ATen op mapped to the wrong primitive.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test model_graph_import_lock --test one_program_lock
```

**Done when.** A model nobody wrote a spec for still comes in.

#### INTAKE-9 · An unknown architecture refused with its missing pieces named

| Field | Value |
|---|---|
| Phase | intake |
| Labels | `infrastructure` `area:intake` |
| Depends on | INTAKE-2 |
| Touches | `crates/logicaffeine_compile/src/model/spec.rs` through S12 |
| Status | open |

**Goal.** When an architecture or a feature cannot be brought up, say exactly why in under a second: every
configuration key and tensor not understood, every feature the author refuses, every op a backend refuses, with the
ticket that would lift each.

**Read first.**
- `crates/logicaffeine_compile/src/model/author.rs`: `check`.

**RED tests.**
- [ ] `model_hf_intake_lock::a_refusal_lists_every_missing_piece_and_its_ticket`: fails today: the first refusal stops
  the walk.
- [x] `model_triage_lock::*` and exam row E1.9: a checkpoint of an architecture no arm reads is refused naming the
  families whose tensors it carries (`model::triage`), or the nearest reading with what it lacks and leaves unread:
  failed (refused naming the architecture alone, `logs/samba/exam1-red-6.log`) (receipt: `logs/samba/multi4-green-3.log`,
  `logs/samba/reds-classify-2.log`).

**Edge cases.**
- [ ] A refusal at intake, at the author and at the backend for one model: all three listed.
- [x] Every arm triaged: the triage reads a checkpoint under every name an arm of `from_source` reads, the names read
  from its match (`model_triage_lock::every_architecture_an_arm_reads_is_triaged`; it read 7 of 9 before: qwen2 and llama4
  were never tried, `logs/samba/red-q2moe.log`) (receipt: `logs/samba/green-q2moe.log`).

**Implementation.** Collect refusals across stages instead of returning the first; map each refusal kind to its ticket.

**Success.**
- [ ] A complete refusal for every Tier C family not yet supported (receipt: the lock).

**Mutants to kill.**
- [x] A refusal list truncated to its first entry (receipt: `logs/samba/mutants5b/results.json`, triage-refusal-names-the-first-exact-only killed).

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test model_hf_intake_lock
```

**Done when.** A model we cannot run tells us everything it needs.

#### AUTHOR-1 · Value width, attention scale, fused projections and routed scale written

| Field | Value |
|---|---|
| Phase | author |
| Labels | `primitive` `area:graph` |
| Depends on | none |
| Touches | `crates/logicaffeine_compile/src/model/author.rs` through S12 |
| Status | open |

**Goal.** `author::check` refuses features the f64 reference already computes: a value width unlike the key width, an
attention scale other than 1/√hd (Gemma 3 27B), fused QKV (Phi-3), sigmoid routing with bias correction and group-limited
top-k (DeepSeek-V3, Kimi K2), and a routed scale other than 1. Write each, held equal to the reference.

**Read first.**
- `crates/logicaffeine_compile/src/model/reference.rs`: the computations the author must match.

**RED tests.**
- [x] `model_conformance_lock::deepseek_v3s_routing_prefills_and_decodes_to_the_references_logits` and
  `deepseek_v3s_routed_model_prefills_and_decodes_to_the_references_logits`: sigmoid routing, its bias, groups and the
  routed scale written (receipt: `logs/samba/green-scaled-attention.log`).
- [x] `model_conformance_lock::a_scaled_attention_equals_the_reference`: refused by `check` (receipt:
  `logs/samba/red-scaled-attention.log`), written `scaled by S` after the window clause (receipt:
  `logs/samba/green-scaled-attention.log`).
- [x] `model_conformance_lock::fused_qkv_equals_the_reference`: refused by `check` (receipt: `logs/samba/red-author1.log`);
  each part of `attn_qkv` and of its bias bound through `View::FusedPart`, under the NEOX rotation and the interleaved one
  (receipt: `logs/samba/green-author1.log`).
- [x] `model_conformance_lock::a_value_width_unlike_the_key_width_equals_the_reference`: values 2 and 8 wide over keys 4
  wide, refused before (receipts: `logs/samba/red-author1.log`, `logs/samba/green-author1.log`).
- [x] `model_conformance_lock::a_softcapped_attention_equals_the_reference` and
  `a_final_logit_softcap_equals_the_reference`: the sugar's `capped at C` on an attention and `X capped at C` on the
  logits (`tensor_softcap_lock` holds both on the VM, the tree-walker, C and AOT-Rust); refused before (receipts:
  `logs/samba/red-author1.log`, `logs/samba/green-author1.log`).
- [x] `model_conformance_lock::each_attention_the_author_writes_splits_over_chips_as_the_reference_computes`: every
  feature above split over 2 and 3 chips, each chip's value columns, cache columns and output rows its heads'
  `value_dim` apiece (receipts: `logs/samba/red-author1-split.log` on HEAD's author, `logs/samba/green-author1-split.log`).

**Edge cases.**
- [x] Group-limited routing whose group count does not divide the experts: refused by name
  (`model_conformance_lock::a_group_limit_the_experts_cannot_hold_is_refused`; receipt:
  `logs/samba/green-scaled-attention.log`).
- [x] A value width larger than the key width (receipt: `logs/samba/green-author1.log`).

**Implementation.** Lift each refusal with its sugar, held by the conformance lock on a tiny configuration.

**Success.**
- [x] Each refusal lifted and equal to the reference: the author still refuses only what the reference refuses (an output
  gate on a fused projection or over values unlike the keys in width) and non-numbers (receipt:
  `logs/samba/green-author1-split.log`).

**Mutants to kill.**
- [ ] The routing bias applied after the top-k instead of before.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test model_conformance_lock --test tensor_softcap_lock
```

**Done when.** The author refuses nothing the reference computes.

#### AUTHOR-2 · LayerNorm models and plain MLPs written

| Field | Value |
|---|---|
| Phase | author |
| Labels | `primitive` `area:norm` |
| Depends on | none |
| Touches | `crates/logicaffeine_compile/src/model/author.rs` and `crates/logicaffeine_compile/src/model/spec.rs` through S12 |
| Status | open |

**Goal.** The spec has no LayerNorm models and no ungated MLP. Encoders, Whisper and older families need both, with
biases.

**Read first.**
- `crates/logicaffeine_compile/src/tensor_graph/front.rs`: `FusedRow` already carries LayerNorm.

**RED tests.**
- [ ] `model_conformance_lock::a_layernorm_model_equals_the_reference`: fails today: not representable.

**Edge cases.**
- [ ] LayerNorm without a bias.

**Implementation.** Spec fields for the norm kind and the FFN kind; author sugar for both.

**Success.**
- [ ] A LayerNorm, plain-MLP tiny model equals its reference (receipt: the lock).

**Mutants to kill.**
- [ ] The LayerNorm's mean subtraction dropped.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test model_conformance_lock
```

**Done when.** Every norm and FFN shape in the open families is writable.

#### AUTHOR-3 · The MTP head kept, not dropped

| Field | Value |
|---|---|
| Phase | author |
| Labels | `primitive` |
| Depends on | none |
| Touches | `crates/logicaffeine_compile/src/model/author.rs` and `crates/logicaffeine_compile/src/model/spec.rs` through S12 |
| Status | open |

**Goal.** The spec counts multi-token-prediction blocks and drops them. Keep them: author the draft head as its own cut,
taking the main stack's hidden state and the embedding of the last accepted token, so speculative decoding (SERVE-RDU-1)
has a draft.

**Read first.**
- `crates/logicaffeine_compile/src/model/spec.rs`: `mtp_blocks`.

**RED tests.**
- [ ] `model_conformance_lock::the_mtp_head_drafts_as_the_reference`: fails today: dropped.

**Edge cases.**
- [ ] A draft head with its own expert layer (DeepSeek-V3).
- [ ] A draft head sharing the main embedding and head.

**Implementation.** A `Cut::Draft` written by the author; bound by the family's MTP tensor names.

**Success.**
- [ ] Qwen3.8's and DeepSeek-V3's tiny draft heads equal their references (receipt: the lock).

**Mutants to kill.**
- [ ] The draft fed the wrong hidden state.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test model_conformance_lock
```

**Done when.** Every published draft head is a program.

#### AUTHOR-4 · Latent attention and experts under tensor parallelism

| Field | Value |
|---|---|
| Phase | author |
| Labels | `primitive` `area:comm` |
| Depends on | FUSIONS:PARITY-TENSOR-PARALLEL-IR |
| Touches | `crates/logicaffeine_compile/src/model/author.rs` through S12 |
| Status | open |

**Goal.** `check_tp` refuses latent attention and experts under tensor parallelism, so DeepSeek, Kimi and every
mixture model cannot shard. Write both sharded: latent attention by heads, experts by expert parallelism or by sliced
expert widths, each with its collectives.

**Read first.**
- `crates/logicaffeine_compile/src/model/author.rs`: `check_tp`.
- The fusion book's PARITY-TENSOR-PARALLEL-IR.

**RED tests.**
- [ ] `qwen38_tensor_parallel_lock::a_sharded_latent_attention_equals_the_unsharded`: fails today: refused.
- [ ] `qwen38_tensor_parallel_lock::sharded_experts_equal_the_unsharded`: fails today: refused.

**Edge cases.**
- [ ] A shared expert under expert parallelism: replicated or sliced, chosen by cost.

**Implementation.** Lift `check_tp`; author the shard decompositions; hold them equal to the unsharded program on the
VM.

**Success.**
- [ ] Both shard and equal the unsharded program (receipt: the lock).

**Mutants to kill.**
- [ ] A shard missing its all-reduce.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test qwen38_tensor_parallel_lock
```

**Done when.** Every model shards.

#### AUTHOR-5 · Every activation and norm offset the families use

| Field | Value |
|---|---|
| Phase | author |
| Labels | `primitive` `area:norm` |
| Depends on | FUSIONS:PARITY-ACTIVATION-KINDS |
| Touches | `crates/logicaffeine_compile/src/model/author.rs` through S12 |
| Status | open |

**Goal.** The author writes SiLU and tanh-GELU. Write every activation the open families use (erf GELU, quick GELU,
ReLU², gated variants) and Gemma's (1 + w) norm weight.

**Read first.**
- The fusion book's PARITY-ACTIVATION-KINDS and PARITY-NORM-WEIGHT-OFFSET.

**RED tests.**
- [ ] `model_conformance_lock::every_activation_equals_the_reference`: fails today: two are written.

**Edge cases.**
- [ ] erf GELU at large magnitude.

**Implementation.** Spec activation kinds; author sugar; the shared primitive from the fusion-book ticket.

**Success.**
- [ ] Every activation equal to its reference (receipt: the lock).

**Mutants to kill.**
- [ ] erf replaced by tanh.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test model_conformance_lock
```

**Done when.** No activation stands between a model and the author.
### 11.9 Architectures

Each ticket takes one feature of section 6.2 from "refused" to "lowered" on the RDU: the shared primitive from its
fusion-book ticket, the spatial lowering here, and a lock that holds the lowered program equal to the VM, certified, at
the tiny configuration and at real width. Until SCHED-1 cuts the op families into their own files, an edit to
`crates/logicaffeine_compile/src/spatial/passes/from_qwen_ops.rs` is claimed through R7's seat.

#### ARCH-WINDOW · Sliding and chunked-local attention

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:attention` |
| Depends on | SCHED-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/passes/from_qwen_ops.rs` (`from_attention`, the sections' mask, `flash_fold`); `crates/logicaffeine_tests/tests/spatial_attention_window_lock.rs` (new) |
| Status | open |

**Goal.** `Attention.window` is authored for Mistral, Gemma 2 and 3 and gpt-oss, and refused by the spatial shim.
Lower it in the flash fold: the trip count starts at the window's first block, the mask covers the partial block. Add
Llama 4's chunked-local attention (attend within fixed chunks) as the same mechanism with a chunk-aligned start.

**Read first.**
- `crates/logicaffeine_tests/tests/spatial_trip_count_lock.rs`: the trip counts a window's start rides on.

**RED tests.**
- [x] `spatial_attention_window_lock::a_decode_row_keeps_the_last_w_positions_on_the_chip`, `…every_head_of_a_group_keeps_the_same_window_on_the_chip`,
  `…each_row_of_a_causal_block_keeps_its_own_window_on_the_chip`, `…a_flash_attention_keeps_the_window_across_its_blocks`:
  failed (refused: "the sections attend a prefix of the cache, never a sliding window", `logs/samba/reds-after-intake6.log`)
  (receipt: `logs/samba/window-1.log`).
- [ ] `spatial_attention_window_lock::a_chunked_local_layer_equals_the_vm`: fails today: not representable in the
  language.
- [x] Alternating windowed and global layers lower as two kinds (`rdu_bringup_lock::kind_programs_are_found_by_digest`,
  Gemma 3; Gemma 3 brings up on the SN50 at both widths, `rdu_bringup_lock`) (receipt: `logs/samba/window-2.log`).

**Edge cases.**
- [x] A window of 1. A window larger than the context. A window that ends mid-block (`spatial_attention_window_lock`).
- [ ] gpt-oss's 128-token window on alternate layers beside full layers.
- [ ] A window with sinks (ARCH-SINKS).

**Implementation.** A window is a masked attention whose rows keep the last `W` positions below their limit: the
sections' mask adds `column ≥ threshold − W` beside `column < threshold`, and the flash fold's mask adds the block's
floor (its limit less `W`) beside its limit, so a block wholly before the window contributes nothing. Still owed: the
fold's trip starting at the window's first block, so it reads only the window's blocks; the certificate's trip window
then covers the start.

**Success.**
- [ ] Both forms equal the VM at tiny and real width, certified (receipt: the lock). The sliding form does at heads 2
  and 32 wide (`logs/samba/window-1.log`); the chunked-local form is owed.
- [ ] The fold reads only the window's blocks (receipt: the work model's bytes).

**Mutants to kill.**
- [x] A window start off by one block (receipt: `logs/samba/mutants5b/results.json`, flash-window-floor-off-by-one killed).

**Verify.**

```
cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test spatial_attention_window_lock --test tensor_attention_window_lock
```

**Done when.** Every windowed family's attention runs on the RDU.

#### ARCH-SINKS · Attention sinks

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:attention` |
| Depends on | SCHED-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/passes/from_qwen_ops.rs` (`flat_softmax_rows_at`, `FlashGeometry.sinks`, `spread_sinks`); `crates/logicaffeine_tests/tests/spatial_attention_window_lock.rs` |
| Status | open |

**Goal.** gpt-oss adds a learned sink logit per head to the softmax's denominator. Lower it in the flash fold's
running max and sum.

**Read first.**
- `crates/logicaffeine_compile/src/model/reference.rs`: the sink's meaning.

**RED tests.**
- [x] `spatial_attention_window_lock::sinks_equal_the_vm`, `…a_flash_attention_starts_its_softmax_from_the_sinks`,
  `…a_sink_changes_the_attention`: failed (refused: "an attention with sinks (`with sinks S`)", gpt-oss's bring-up in
  `logs/samba/reds-window.log`) (receipt: `logs/samba/sinks-1.log`, 8/8).

**Edge cases.**
- [x] Sinks with a window, on a decode row, grouped heads and a causal block, in both forms
  (`spatial_attention_window_lock`).
- [ ] A sink larger than every score.

**Implementation.** The sections' row softmax folds the sink into the row: its maximum is `max(row max, sink)` and its
sum adds `e^(sink − m)`, the numerator unchanged. The flash fold seeds each row's running maximum with its head's sink
and its running sum with 1 (`e^(sink − sink)`): a spread section lays the sinks out one tile per item
(`{out}__flash_sinks`), so each item's fold starts from its own head's sink.

**Success.**
- [x] Equal to the VM, certified, heads 2 and 32 wide (receipt: `logs/samba/sinks-1.log`).

**Mutants to kill.**
- [ ] The sink added to the numerator.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test spatial_attention_window_lock --test tensor_attention_window_lock
```

**Done when.** gpt-oss's attention runs on the RDU.

#### ARCH-SOFTCAP · Soft caps on scores and logits

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:attention` |
| Depends on | SCHED-1, FUSIONS:PARITY-SOFTCAP |
| Touches | `crates/logicaffeine_compile/src/tensor_recognize/qwen.rs` (`as_capped`, `match_softcap`, `AttentionSpec.softcap`); `crates/logicaffeine_compile/src/tensor_graph/{front,eval,vjp}.rs`; `crates/logicaffeine_compile/src/spatial/passes/from_qwen_ops.rs` (`capped`, `from_softcap`, `HeadWidths.cap`, `FlashGeometry.cap`); `crates/logicaffeine_compile/src/spatial/{shim,link}.rs`; the shims of the chips without a cap; `crates/logicaffeine_tests/tests/{spatial_softcap_lock,tensor_softcap_lock}.rs` |
| Status | lowered; Gemma 2's own deployment is DEPLOY-GEMMA's |

**Goal.** Gemma 2 caps attention scores and final logits with `c·tanh(x/c)`. Lower both: the score cap inside the flash
fold before the running max, the logit cap in the head before the arg-max.

**Read first.**
- The fusion book's PARITY-SOFTCAP.

**RED tests.**
- [x] `spatial_softcap_lock::capped_scores_equal_the_vm`, `…capped_logits_equal_the_vm`,
  `…a_flash_attention_caps_each_block_before_its_running_maximum`, `…scores_far_past_the_cap_equal_the_vm`,
  `…a_cap_changes_the_attention`: failed (refused as unrecognized, `logs/samba/red-spatial-softcap-head.log`, 0/5)
  (receipt: `logs/samba/green-softcap-9.log`).
- [x] `tensor_softcap_lock::the_front_end_lifts_the_cap_into_the_spec_apart_from_the_scale`,
  `…the_sn50_computes_a_capped_decode_step`, `…every_consumer_of_an_attention_spec_is_held_to_its_cap`,
  `…every_shim_without_the_cap_refuses_the_capped_op_by_name_and_every_spatial_shim_resolves_it`,
  `…the_backward_pass_refuses_a_cap_by_name_until_its_rule_is_written`,
  `…the_graph_evaluator_computes_every_capped_op_as_the_vm_does`: failed (no `softcap` field, no `Softcap` op,
  `logs/samba/red-softcap-head-2.log`) (receipt: `logs/samba/green-softcap-9.log`, 12/12).
- [x] `tensor_recognize::qwen::tests::the_cap_is_read_from_its_sugar_and_from_its_loop_written_out`,
  `…no_near_miss_of_the_caps_form_is_read_as_a_cap` (receipt: `logs/samba/softcap-unit-1.log`).

**Edge cases.**
- [x] Scores far past the cap: a scale of 200 puts them past ±100 against a cap of 0.5, where a cap taken after the
  running maximum would underflow every weight (`spatial_softcap_lock::scores_far_past_the_cap_equal_the_vm`).
- [x] A cap with a window, with sinks, on grouped heads and a causal block, in both forms (`spatial_softcap_lock`).
- [x] Logits far past the cap reach `±C` (`spatial_softcap_lock::capped_logits_equal_the_vm`), and a capped element
  of `±1e300` is `±C` with no `∞/∞` on the VM, the tree-walker, C and AOT-Rust (`tensor_softcap_lock`).

**Implementation.** The front end lifts `capped at C` on an attention into `AttentionSpec.softcap`, apart from the
scale, and `Let Y be X capped at C` as `TensorOpSpec::Softcap`. The lowering writes the cap as `2c·σ(2x/c) − c`, one
exponential and one reciprocal, reaching `±c` in both tails: the sections form caps each scaled score before the mask,
the flash fold caps each block's scores before its maximum, and a capped tensor is one row section. Every file that
reads an attention's spec computes the cap or refuses it by name (the census in `tensor_softcap_lock`): the CUDA lane
and forward and the D5005 engine refuse it, and the backward pass differentiates through it.

**Success.**
- [x] Both equal the VM on the SN50 within the bf16 datapath's error, heads 2 and 32 wide; a Gemma 2 shaped decode step
  within a relative 2⁻¹⁶ on an fp32 twin of the chip (receipt: `logs/samba/green-softcap-9.log`).

**Mutants to kill.**
- [ ] The cap applied after the max (round `logs/samba/mutants-softcap/`, 21 mutants: the cap dropped in each form, the
  formula's rate and shift, the evaluator, each refusal, the lift and each of the recognizer's checks).

**Verify.**

```
cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test spatial_softcap_lock --test tensor_softcap_lock
cargo nextest run -p logicaffeine-compile --lib -E 'test(tensor_recognize::qwen::)'
```

**Done when.** Gemma 2 runs on the RDU.

#### ARCH-NOPE · Layers without rotation and with tuned temperature

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:attention` |
| Depends on | SCHED-1 |
| Touches | the attention family's lowering file (new) |
| Status | open |

**Goal.** Llama 4 interleaves rotation-free attention layers with rotated ones and tunes attention temperature by
position on the rotation-free layers. Lower both as layer kinds.

**Read first.**
- The Llama 4 model file under `external/competitors/vllm`.

**RED tests.**
- [ ] `spatial_nope_attention_lock::a_rotation_free_layer_equals_the_vm`: fails today: no RDU kind.
- [ ] `spatial_nope_attention_lock::temperature_tuning_equals_the_vm`: fails today: not representable.

**Edge cases.**
- [ ] Temperature at position 0.

**Implementation.** A kind without the rotation section; a per-position scale table read like the rotation cache.

**Success.**
- [ ] Both equal the VM (receipt: the lock).

**Mutants to kill.**
- [ ] Temperature applied on the rotated layers.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_nope_attention_lock
```

**Done when.** Llama 4's attention layers all run.

#### ARCH-MLA · Latent attention, absorbed for decode

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:attention` |
| Depends on | SCHED-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/passes/from_qwen_ops.rs` (`tile_head_slice`, `tile_join`, `tile_head_matmul`, `FlashGeometry.vd`, `value_pages`, `FlatBuild::columns`); `crates/logicaffeine_compile/src/spatial/ir.rs` and `crates/logicaffeine_compile/src/spatial/passes/place.rs` (a result stored in parts); `crates/logicaffeine_tests/tests/spatial_latent_tiles_lock.rs` (new) |
| Status | open |

**Goal.** DeepSeek-V2 and V3 and Kimi K2 cache a compressed latent and a rotated key part. Lower decode in the absorbed
form (the key up-projection folded into the query, the value up-projection after attention) so the cache read is the
latent, and prefill in the expanded form; choose by the time model.

**Read first.**
- `crates/logicaffeine_compile/src/model/author.rs`: `mla`.
- The fusion book's PARITY-MLA-PROLOGUE and PARITY-MLA-UNABSORB-IN-FOLD.

**RED tests.**
- [x] `spatial_latent_tiles_lock::a_head_slice_of_whole_tiles_moves_tiles`, `…a_join_of_whole_tiles_moves_tiles`,
  `…a_per_head_matmul_of_whole_tiles_is_a_gemv_per_head`, `…an_absorbed_latent_step_of_whole_tiles_is_the_vms`,
  `…an_absorbed_latent_step_wider_than_an_accumulator_page_is_the_vms`,
  `…deepseek_v2_lites_latent_step_lowers_whole_and_certifies`: failed (the per-head product `NoLegalTiling`, the join
  and the head slice `AccTilesExceeded`, the attention `NoLegalTiling` and then `OutputOverlap`,
  `logs/samba/latent-tiles-1.log` … `-4.log`) (receipt: `logs/samba/latent-tiles-5.log`, 6/6).
- [ ] `rdu_bringup_reds_lock`: the tiny DeepSeek-V2 checkpoints (widths of 72, not whole tiles) bring up.

**Edge cases.**
- [x] Keys wider than values (576 against 512), each past an accumulator page; a latent of 320 (ten tiles, two value
  pages of five); a head slice and a join wider than an accumulator page; a per-head product of 2 to 16 heads.
- [x] DeepSeek-V2-Lite's own widths (16 heads, 128 + 64 query dims, a latent of 512, values of 128) lower whole and
  certify on the SN50.
- [ ] Widths that are not whole tiles past an accumulator page (the tiny checkpoints' 72): the column-blocked flat form.
- [ ] q-LoRA present and absent. A split versus fused key-value up-projection. Prefill (`m > 1` per-head products).

**Implementation.** At whole-tile widths the head slice and the join are tile moves a block at a time (the join's source
block from compile-time tables, so every item runs the same stages); the per-head product is a GEMV an item per output
tile, folding over its weight tiles' rows and over the head's tiles, since a page is one plane of its tiles side by side.
The flash fold carries a value width of its own, its running output folded an accumulator page at a time; a decode fold
too large for a site is one fold per value page, joined per head, each reading the keys again. A section that stores its
result in parts is covered by what each store addresses, and placement keeps each part's shape. Owed: the values read
from the key pages (the latent is the keys' first part), so the cache is read once, and the grouped fold (one item per
key/value head) at these widths.

**Success.**
- [x] Equal to the VM within the program-path ledger's rule at whole-tile widths, certified (receipt:
  `logs/samba/latent-tiles-5.log`).
- [ ] The cache bytes are the latent's: one read a step (receipt: the work model's bytes).
- [ ] DeepSeek-V2 brings up on the SN50 (`rdu_bringup_lock`).

**Mutants to kill.**
- [ ] The value up-projection applied before the fold.
- [x] The join keeping one block too many of `A`; the head slice ignoring its offset; a value page read from page 0 (receipt: `logs/samba/mutants5b/results.json`, tile-join-keeps-a-block-too-many, tile-head-slice-from-ignored and value-page-reads-page-zero killed).

**Verify.**

```
cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test spatial_latent_tiles_lock --test tensor_latent_attention_lock
```

**Done when.** DeepSeek's attention runs on the RDU at the bytes it was designed for.

#### ARCH-SPARSE · Indexed sparse attention: DSA, QSA and NSA

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:attention` |
| Depends on | KERNEL-1, FUSIONS:PARITY-SPARSE-ATTENTION |
| Touches | the attention family's lowering file (new) |
| Status | open |

**Goal.** DeepSeek V3.2, GLM-5 and qwen4_exp select which cached tokens attend by an indexer and a top-k. Lower the
indexer (its key cache, its scores, its pooling where the model pools), the selection (KERNEL-1), and attention over the
selected blocks through an indirect tile into the flash fold.

**Read first.**
- `external/competitors/llama.cpp/src/models/deepseek32.cpp` and `external/competitors/llama.cpp/src/models/qwen4exp.cpp`.
- The fusion book's PARITY-SPARSE-ATTENTION.

**RED tests.**
- [ ] `spatial_sparse_attention_lock::dsa_selection_equals_the_vm`: fails today: refused.
- [ ] `spatial_sparse_attention_lock::below_the_budget_the_layer_is_exactly_dense`: fails today.
- [ ] `spatial_sparse_attention_lock::the_fold_reads_only_the_selected_blocks`: fails today.

**Edge cases.**
- [ ] Budget minus one, budget, budget plus one cached tokens.
- [ ] Score ties at the k-th place.
- [ ] An incomplete tail block.

**Implementation.** The indexer as a fold over the key cache; selection by KERNEL-1; the selected block list as an
indirect tile table.

**Success.**
- [ ] DSA and QSA equal the VM; dense below the budget bit for bit (receipt: the lock).

**Mutants to kill.**
- [ ] The tail block dropped.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_sparse_attention_lock
```

**Done when.** Indexed sparse attention runs on the RDU at the bytes it was designed for.

#### ARCH-ROPE · Every rotation the families use

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:attention` |
| Depends on | SCHED-1, FUSIONS:PARITY-ROPE-VARIANTS |
| Touches | the attention family's lowering file (new) |
| Status | open |

**Goal.** The RDU path rotates NEOX-style, unscaled. Lower interleaved rotation, partial rotation, the multimodal
sections (interleaved and not), and every scaling (linear, YaRN with its magnitude, llama3's smoothing, LongRoPE's
factor tables, NTK), all through the rotation cache the decode step already reads.

**Read first.**
- `crates/logicaffeine_compile/src/model/spec.rs`: `Rope`, `RopeScaling`.

**RED tests.**
- [ ] `spatial_rope_variants_lock::every_variant_equals_the_vm`: fails today: NEOX unscaled only.

**Edge cases.**
- [ ] A rotated width that is not a multiple of 4.
- [ ] YaRN past its original context.

**Implementation.** The cache generator computes each variant's angles and magnitudes; the rotation section reads the
layout the variant needs.

**Success.**
- [ ] Every variant equal to the VM (receipt: the lock).

**Mutants to kill.**
- [ ] Interleaved pairs read NEOX-style.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_rope_variants_lock
```

**Done when.** No rotation stops a family.

#### ARCH-CROSS · Cross-attention and encoder–decoder models

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:attention` |
| Depends on | SCHED-1 |
| Touches | the attention family's lowering file (new) |
| Status | open |

**Goal.** Whisper and T5-class models attend from the decoder to an encoder's output. Lower full attention between
tensors of different lengths and an encoder pass whose output the decoder steps read.

**Read first.**
- `crates/logicaffeine_compile/src/tensor_graph/front.rs`: `FullAttention`.

**RED tests.**
- [ ] `spatial_cross_attention_lock::cross_attention_equals_the_vm`: fails today: equal lengths only.

**Edge cases.**
- [ ] An encoder output longer than the decoder's cache.

**Implementation.** Generalize `FullAttention` to distinct query and key lengths; a two-program deployment.

**Success.**
- [ ] A tiny encoder–decoder equals the VM (receipt: the lock).

**Mutants to kill.**
- [ ] A causal mask applied to cross-attention.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_cross_attention_lock
```

**Done when.** Encoder–decoder models run.

#### ARCH-POOLING · Bidirectional encoders and pooling heads

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:attention` |
| Depends on | SCHED-1, FUSIONS:SERVE-POOLING |
| Touches | the attention family's lowering file (new) |
| Status | open |

**Goal.** Embedding and reranking models attend without a causal mask and end in a pooling head (mean, last token,
CLS) and a normalization or a score.

**Read first.**
- The fusion book's SERVE-POOLING.

**RED tests.**
- [ ] `spatial_pooling_lock::an_embedding_model_equals_the_vm`: fails today.

**Edge cases.**
- [ ] Mean pooling over a padded batch.

**Implementation.** An unmasked attention kind; pooling row ops.

**Success.**
- [ ] Embedding and reranking tiny models equal the VM (receipt: the lock).

**Mutants to kill.**
- [ ] Padding counted in the mean.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_pooling_lock
```

**Done when.** Retrieval models run.

#### ARCH-VISION · Vision towers

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:attention` |
| Depends on | ARCH-CNN, FUSIONS:PARITY-MULTIMODAL-TOWERS |
| Touches | the attention family's lowering file (new) |
| Status | open |

**Goal.** Qwen3-VL's ViT (which qwen4_exp reuses), Llama 4's vision encoder: a patch embedding, bidirectional
attention with two-dimensional rotation, a spatial merger, and a projector into the language model's width.

**Read first.**
- The fusion book's PARITY-MULTIMODAL-TOWERS.

**RED tests.**
- [ ] `spatial_vision_tower_lock::a_tiny_vit_equals_the_vm`: fails today.

**Edge cases.**
- [ ] An image whose size is not a multiple of the patch. A video with temporal patches.

**Implementation.** Patch embedding as a strided convolution (ARCH-CNN); the tower as attention kinds; the merger as a
gather.

**Success.**
- [ ] A tiny tower equals the VM (receipt: the lock).

**Mutants to kill.**
- [ ] Two-dimensional positions swapped.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_vision_tower_lock
```

**Done when.** A multimodal prompt runs.
#### ARCH-ROUTING · Sigmoid routing with bias correction and group limits

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:moe` |
| Depends on | SCHED-1, AUTHOR-1, FUSIONS:PARITY-ROUTING-VARIANTS |
| Touches | the expert family's lowering file (new); `crates/logicaffeine_tests/tests/spatial_moe_real_width_lock.rs` |
| Status | open |

**Goal.** DeepSeek-V3, Kimi K2 and GLM route by sigmoid scores plus a per-expert bias for selection only, choose the
top groups first and then the top experts within them, and scale the routed sum. Lower the router as a fold that
yields the selected experts' Int32 ids and their weights, at fp32 (NUM-1).

**Read first.**
- `crates/logicaffeine_compile/src/model/reference.rs`: the routing it computes.

**RED tests.**
- [ ] `spatial_moe_routing_lock::group_limited_sigmoid_routing_equals_the_vm`: fails today: refused.
- [ ] `spatial_moe_routing_lock::the_bias_selects_but_does_not_weight`: fails today.

**Edge cases.**
- [ ] Ties across a group boundary. All scores equal. A NaN score.

**Implementation.** Group scores as a fold of the top two in each group; a group top-k; an expert top-k within the
chosen groups (KERNEL-1's selection).

**Success.**
- [ ] Equal to the VM at DeepSeek-V3 width (receipt: the lock).

**Mutants to kill.**
- [ ] The bias added to the weights.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_moe_routing_lock --test spatial_moe_real_width_lock
```

**Done when.** Every published router runs on the RDU.

#### ARCH-SHARED-GATE · Gated shared experts

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:moe` |
| Depends on | SCHED-1, FUSIONS:PARITY-ROUTER-SHARED-GATE-MERGE |
| Touches | the expert family's lowering file (new) |
| Status | open |

**Goal.** Qwen's mixtures (Qwen2-MoE, Qwen3.5-MoE, qwen4_exp) scale the shared expert by `σ(g·x)`. Lower the gate with
the router as one product (the fusion book's merge) and the shared expert beside the routed ones.

**Read first.**
- The fusion book's PARITY-ROUTER-SHARED-GATE-MERGE.

**RED tests.**
- [ ] `spatial_moe_routing_lock::a_gated_shared_expert_equals_the_vm`: fails today.

**Edge cases.**
- [x] A shared expert wider than a routed one (receipt: `model_harness::qwen35_moe_spec`'s shared expert is twice a
  routed one's width, `spatial_moe_routing_lock::a_qwen35_moe_step_equals_the_vm` in `logs/samba/green-q35moe.log`).

**Implementation.** Built: `model::spec::Moe::shared_gate`, read by the `qwen2moe` and `qwen35moe` arms; the author writes
the gate as a product of the normed input with a one-column weight (`ffn_gate_inp_shexp`), its sigmoid spread across the
row by the host's row of ones (`bind::computed_tables`), multiplied into the shared expert's output before it joins the
routed sum; the reference computes the same from the checkpoint. Owed: the gate's row appended to the router's product
and the shared output scaled in the combine, which waits on SCHED-1 and the fusion book's merge.

**Success.**
- [x] Equal to the VM on Qwen3.5-MoE's shape (receipt: `spatial_moe_routing_lock::a_qwen35_moe_step_equals_the_vm` in
  `logs/samba/green-q35moe.log`: on the fp32 twin within 2⁻¹⁶ of the largest logit, on the SN50 within 2%).
- [ ] The gate's row merged into the router's product (SCHED-1, FUSIONS:PARITY-ROUTER-SHARED-GATE-MERGE).

**Mutants to kill.**
- [ ] The gate applied to the routed sum.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_moe_routing_lock
```

**Done when.** Qwen's mixtures run on the RDU.

#### ARCH-DENSE-FIRST · Dense leading layers and per-layer expert widths

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:moe` |
| Depends on | SCHED-1 |
| Touches | the expert family's lowering file (new) |
| Status | open |

**Goal.** DeepSeek's first layers are dense; Llama 4 interleaves dense and expert layers; some families vary the
expert width by layer. The deployment's kinds follow the program's layers, not a fixed pattern.

**Read first.**
- INTAKE-6 (kinds found by digest).

**RED tests.**
- [ ] `spatial_moe_routing_lock::dense_and_expert_layers_deploy_as_their_kinds`: fails today: one layer shape a kind.

**Edge cases.**
- [ ] A model where every layer's expert width differs.

**Implementation.** Kinds by digest (INTAKE-6); the residency plan per kind.

**Success.**
- [ ] DeepSeek-V3's and Llama 4's tiny deployments equal the VM (receipt: the lock).

**Mutants to kill.**
- [ ] A dense layer bound to an expert kind.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_moe_routing_lock
```

**Done when.** Mixed layer stacks deploy.

#### ARCH-MIXTRAL · Mixtral's experts read and run

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:moe` |
| Depends on | INTAKE-2, SCHED-1 |
| Touches | the expert family's lowering file (new) |
| Status | open |

**Goal.** Mixtral 8x7B (in SambaNova's evaluation set [DIAYN26]) is read as dense today and fails at bind. Read its 8
experts, top-2 softmax routing and sliding window (early releases), and deploy it.

**Read first.**
- INTAKE-2's Mixtral arm.

**RED tests.**
- [ ] `spatial_moe_routing_lock::mixtral_equals_the_vm`: fails today: read as dense.

**Edge cases.**
- [ ] Top-2 where both picks tie.

**Implementation.** The arm reads the experts; the existing softmax top-k MoE lowering runs them.

**Success.**
- [ ] A tiny Mixtral equals the VM; the real-width layer certifies (receipt: the lock).

**Mutants to kill.**
- [ ] Top-2 weights not renormalized.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_moe_routing_lock
```

**Done when.** A model in their evaluation set that we misread is read right.

#### ARCH-OSS · gpt-oss experts on the RDU

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:moe` `model:oss` |
| Depends on | SCHED-1, NUM-3 |
| Touches | `crates/logicaffeine_compile/src/spatial/passes/from_qwen_ops.rs` (`from_moe`, `blocked_moe`, `moe_hidden`, `moe_combine`, `MoeOperands`, `moe_gated`); `crates/logicaffeine_compile/src/tensor_recognize/qwen.rs` (`moe_options`); `crates/logicaffeine_tests/tests/spatial_moe_options_lock.rs` (new) |
| Status | open |

**Goal.** gpt-oss's experts use a clamped SwiGLU, biases on the router and the experts, and MXFP4 weights. Lower all of
it; GPT-OSS-120B is in SambaNova's evaluation set [DIAYN26] and served at 740 tok/s as captured 2026-10-29 [AA26].
DeepSeek-V2's weighting by the softmax over every expert (`without renormalizing`) rides the same combine.

**Read first.**
- `crates/logicaffeine_tests/tests/tensor_moe_options_lock.rs`

**RED tests.**
- [x] `spatial_moe_options_lock::every_option_computes_the_vms_mixture_in_the_flat_form`,
  `…every_option_computes_the_vms_mixture_in_the_blocked_form`: failed (refused: "a mixture-of-experts block with a
  router or expert bias — the sections add none", gpt-oss's bring-up in `logs/samba/reds-sinks.log`), then an up bias
  alone computed as the gate's (`the up bias at d5 dff7 e4 k2: WIRING Show #1: fp32 twin 256 vs VM -1204`,
  `logs/samba/moe-options-1.log`: the front end told the gate's bias from the up's by order) (receipt:
  `logs/samba/moe-options-9.log`, 3/3).
- [x] `tensor_moe_options_lock::each_expert_bias_alone_is_lifted_into_its_own_slot` (receipt:
  `logs/samba/moe-options-4.log`).

**Edge cases.**
- [x] Each bias alone, the clamp alone, the weighting alone, gpt-oss's set, and every option together, in the flat and
  the blocked form, on inputs where each moves the answer and the clamp crosses both the gate's and the up's limits
  (`spatial_moe_options_lock`).
- [x] A router bias that changes which experts are chosen.
- [ ] A mixture whose widths are not whole tiles and whose experts do not fit one site: the flat form holds every
  expert's weights in one site and refuses it (`NoLegalTiling`; a mixture of 6 experts of 24 × 40 needs more than the
  SN50 site's 435,562 bytes). Every published mixture's widths are whole tiles (gpt-oss 2,880, DeepSeek 1,408 and
  1,536, Qwen3 768), so the blocked form takes them; the ragged case takes the blocked form once the lowering pads
  each expert's rows to whole tiles and the host fill writes the padding.

**Implementation.** `MoeOperands` places the optional operands past `[x, wr, wg, wu, wd]`. The flat form adds the
router's bias after its product and each expert's bias row gathered by the round's pick; the blocked form adds the
router's bias in a section of its own, gathers the chosen expert's gate and up bias tiles by its pick in the hidden
section, and mixes the down biases by the router weights in the combine (`Σ_r w_r·B_d[b_r]`). `moe_gated` evaluates
the clamped SwiGLU as `silu(A·min(g, L))/A·(clamp(u, −L, L) + 1)`, one SFPU op. The combine divides by the kept
weights' sum only when the router renormalizes. The flat form's per-round intermediates hold one page each: the section
is one work item. The front end now tells the gate's bias from the up's by the weight its accumulator sums. MXFP4
weights ride NUM-3.

**Success.**
- [x] Equal to the VM, certified, in both forms (receipt: `logs/samba/moe-options-9.log`).
- [ ] The real-width layer certifies (2,880 wide, 32 experts, top 4).
- [ ] gpt-oss brings up on the SN50 (`rdu_bringup_lock`).

**Mutants to kill.**
- [x] The clamp applied to the wrong half of the GLU (receipt: `logs/samba/mutants5b/results.json`, moe-clamp-wrong-half killed).
- [x] The down bias added unweighted (receipt: `logs/samba/mutants5b/results.json`, moe-down-bias-unweighted killed).
- [x] The weights' sum divided out without renormalizing (receipt: `logs/samba/mutants5b/results.json`, moe-flat-divides-without-renormalizing and moe-blocked-divides-without-renormalizing killed).

**Verify.**

```
cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test spatial_moe_options_lock --test tensor_moe_options_lock --test tensor_moe_weighting_lock
```

**Done when.** gpt-oss runs on the RDU.

#### ARCH-GDN-VARIANTS · Gated-delta variants

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:recurrent` |
| Depends on | SCHED-1 |
| Touches | the recurrent family's lowering file (new); `crates/logicaffeine_tests/tests/tensor_gdn_row_blocks_lock.rs` |
| Status | open |

**Goal.** The gated-delta lowering is Qwen3.5's. Make its variants parameters: the output gate's activation (SiLU or
sigmoid), a readout scale, the ratio of value heads to key heads, the convolution width, and the state precision.

**Read first.**
- `crates/logicaffeine_tests/tests/tensor_gdn_row_blocks_lock.rs`

**RED tests.**
- [ ] `tensor_gdn_row_blocks_lock::a_sigmoid_gated_variant_equals_the_vm`: fails today.
- [ ] `tensor_gdn_row_blocks_lock::a_scaled_readout_equals_the_vm`: fails today.
- [x] `tensor_gdn_row_blocks_lock::a_gated_delta_whose_heads_are_not_whole_tiles_lowers_on_sn50_and_computes_the_vms_values`:
  heads of 16 (Qwen3-Next's tiny twin: six value heads over two key heads, a decode step and a three-row block), of 8
  (32 over four), of 48 (eight over two) and of 80 (two over one, a two-row block), every memory past one tile.
  Failed: the blocked form refused a head that is not whole tiles once the memory passed one tile (`logs/samba/red-gd-subtile.log`) (receipt: `logs/samba/green-gd-subtile-4.log`).

**Edge cases.**
- [ ] Value heads that are not a multiple of key heads.
- [ ] A head that is not whole tiles whose memory is more than a site holds: the widening reads the memory whole into
  each item. The released models' heads (128) are whole tiles and never take it.

**Implementation.** A variant record carried by the recognizer into the one-pass step and the chunked form.
Head widths are done (`from_gated_delta` in `crates/logicaffeine_compile/src/spatial/passes/from_qwen_ops.rs`): a head of
whole tiles is blocked as before; one that is not stays flat while the whole memory fits one tile, and past it is
widened to whole tiles (`widened_gated_delta`). Zeros past a head's width are a fixed point of the step, so its
operands and memory are moved into whole-tile heads (copies and zeros, exact), the blocked step runs there reading the
head's own width for `1/√d` and the norm, and the output and memory move back to the program's layout.

**Success.**
- [ ] Each variant equal to the VM in decode and in chunks (receipt: the lock).

**Mutants to kill.**
- [ ] The readout scale applied twice.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test tensor_gdn_row_blocks_lock
```

**Done when.** Any gated-delta model runs.

#### ARCH-SSD · Mamba2 chunked state-space scan

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:recurrent` |
| Depends on | KERNEL-3, FUSIONS:PARITY-SSD-CHUNK-SCAN |
| Touches | the recurrent family's lowering file (new) |
| Status | open |

**Goal.** Mamba2's scan in its chunked form (intra-chunk products, inter-chunk state passing) for prefill and its
recurrent step for decode.

**Read first.**
- The fusion book's PARITY-SSD-CHUNK-SCAN.

**RED tests.**
- [ ] `spatial_ssm_lock::a_mamba2_chunk_equals_the_vm`: fails today.
- [ ] `spatial_ssm_lock::a_mamba2_step_equals_the_vm`: fails today.

**Edge cases.**
- [ ] A chunk of one row. Decay near zero.

**Implementation.** The chunked form from the contraction axis; the step from KERNEL-3.

**Success.**
- [ ] Both equal the VM (receipt: the lock).

**Mutants to kill.**
- [ ] Inter-chunk state passed without its decay.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_ssm_lock
```

**Done when.** Mamba2 runs.

#### ARCH-SELECTIVE-SCAN · Mamba1's selective scan

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:recurrent` |
| Depends on | KERNEL-3, FUSIONS:PARITY-SELECTIVE-SCAN |
| Touches | the recurrent family's lowering file (new) |
| Status | open |

**Goal.** Jamba, Falcon-H1 and Nemotron-H interleave attention with Mamba layers. Lower the selective scan and its
convolution.

**Read first.**
- The fusion book's PARITY-SELECTIVE-SCAN.

**RED tests.**
- [ ] `spatial_ssm_lock::a_selective_scan_equals_the_vm`: fails today.

**Edge cases.**
- [ ] A state dimension of 1.

**Implementation.** The scan as KERNEL-3's segmented scan; the convolution as the causal convolution.

**Success.**
- [ ] Equal to the VM (receipt: the lock).

**Mutants to kill.**
- [ ] The discretization step skipped.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_ssm_lock
```

**Done when.** Hybrid state-space models run.

#### ARCH-RWKV · RWKV-7

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:recurrent` |
| Depends on | KERNEL-3 |
| Touches | the recurrent family's lowering file (new) |
| Status | open |

**Goal.** RWKV-7's recurrence and token shift, in step and chunked forms.

**Read first.**
- The RWKV-7 model file under `external/competitors/vllm`.

**RED tests.**
- [ ] `spatial_ssm_lock::an_rwkv7_step_equals_the_vm`: fails today.

**Edge cases.**
- [ ] The first token, with no shift history.

**Implementation.** The recurrence as a recurrent section; chunked by KERNEL-3.

**Success.**
- [ ] Equal to the VM (receipt: the lock).

**Mutants to kill.**
- [ ] The token shift reading the current token.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_ssm_lock
```

**Done when.** RWKV runs.

#### ARCH-LIGHTNING · Lightning attention

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:recurrent` |
| Depends on | KERNEL-3, FUSIONS:PARITY-LIGHTNING-ATTENTION |
| Touches | the recurrent family's lowering file (new) |
| Status | open |

**Goal.** MiniMax-Text-01's block linear attention with decay, in prefill blocks and a decode step.

**Read first.**
- The fusion book's PARITY-LIGHTNING-ATTENTION.

**RED tests.**
- [ ] `spatial_ssm_lock::lightning_attention_equals_the_vm`: fails today.

**Edge cases.**
- [ ] A block of one row.

**Implementation.** Intra-block products and inter-block state, as the chunked gated-delta form does.

**Success.**
- [ ] Equal to the VM (receipt: the lock).

**Mutants to kill.**
- [ ] Decay applied once too often.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_ssm_lock
```

**Done when.** MiniMax's linear-attention models run.

#### ARCH-KDA · Kimi delta attention

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:recurrent` |
| Depends on | ARCH-GDN-VARIANTS, FUSIONS:PARITY-KDA |
| Touches | the recurrent family's lowering file (new) |
| Status | open |

**Goal.** Kimi Linear's delta attention with channel-wise gates, as a variant of the gated-delta lowering.

**Read first.**
- The fusion book's PARITY-KDA.

**RED tests.**
- [ ] `tensor_gdn_row_blocks_lock::kimi_delta_attention_equals_the_vm`: fails today.

**Edge cases.**
- [ ] A gate of zero in one channel.

**Implementation.** A channel-wise decay in the variant record.

**Success.**
- [ ] Equal to the VM (receipt: the lock).

**Mutants to kill.**
- [ ] A scalar gate where the channel gate belongs.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test tensor_gdn_row_blocks_lock
```

**Done when.** Kimi Linear runs.

#### ARCH-SHORT-CONV · Gated short convolution

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:recurrent` |
| Depends on | SCHED-1, FUSIONS:PARITY-SHORT-CONV |
| Touches | the recurrent family's lowering file (new) |
| Status | open |

**Goal.** LFM2's gated short convolution layers beside its attention layers.

**Read first.**
- The fusion book's PARITY-SHORT-CONV.

**RED tests.**
- [ ] `spatial_ssm_lock::a_gated_short_convolution_equals_the_vm`: fails today.

**Edge cases.**
- [ ] A convolution longer than the prompt.

**Implementation.** The causal convolution ring with gates in its epilogue.

**Success.**
- [ ] Equal to the VM (receipt: the lock).

**Mutants to kill.**
- [ ] The gate applied before the convolution.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_ssm_lock
```

**Done when.** LFM2 runs.

#### ARCH-CNN · Convolutional networks

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` |
| Depends on | SCHED-1 |
| Touches | the convolution family's lowering file (new) |
| Status | open |

**Goal.** The Plasticine kit's model set is torchvision networks (ResNet-50, SqueezeNet) [PTUT], and vision towers begin
with a patch convolution. Lower two-dimensional convolutions (strides, padding, groups, dilation), pooling, batch norm
folded into the convolution for inference and kept for training, and the residual and concatenation structure.

**Read first.**
- The kit's README (section 12.3).

**RED tests.**
- [ ] `spatial_cnn_lock::resnet50_equals_pytorch`: fails today: no convolution lowering.
- [ ] `spatial_cnn_lock::squeezenet_equals_pytorch`: fails today.

**Edge cases.**
- [ ] A 1×1 convolution, a depthwise convolution, a 7×7 stride-2 stem.
- [ ] Batch norm folded and unfolded giving equal results within the contract.

**Implementation.** Convolution as an implicit product over the contraction axis; pooling as folds; batch-norm folding
proved (MINE-2).

**Success.**
- [ ] Both networks equal PyTorch on the CPU, certified (receipt: the lock).

**Mutants to kill.**
- [ ] Padding applied on one side only.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_cnn_lock
```

**Done when.** The kit's model set runs.

#### ARCH-NORMS · Norm and residual forms on the RDU

| Field | Value |
|---|---|
| Phase | architecture |
| Labels | `primitive` `rdu` `area:norm` |
| Depends on | SCHED-1, AUTHOR-2 |
| Touches | `crates/logicaffeine_compile/src/spatial/passes/from_qwen_ops.rs` |
| Status | open |

**Goal.** The RDU path's layers are Qwen's. Lower the norm and residual forms the other families use: sandwich norms,
Gemma's (1 + w) weight and embedding scale, LayerNorm with and without bias, and post-norms.

**Read first.**
- `crates/logicaffeine_compile/src/spatial/passes/from_qwen_ops.rs`: `blocked_rms_norm`.

**RED tests.**
- [ ] `spatial_norm_forms_lock::every_norm_form_equals_the_vm`: fails today.

**Edge cases.**
- [ ] A norm over a width that is not a multiple of a tile.

**Implementation.** Each form as a row section with its weights and epilogues.

**Success.**
- [ ] Every form equal to the VM (receipt: the lock).

**Mutants to kill.**
- [ ] (1 + w) read as w.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_norm_forms_lock
```

**Done when.** No norm stops a family.
### 11.10 Qwen 4

Qwen3.8-Flash-Next, component by component (section 7), then the whole deployment and the day-zero drill. The
components other streams own (sparse attention, the router, gated delta, rotation, speculative decoding) are those
streams' tickets; these tickets bind them to qwen4_exp's shapes and own what only this architecture has.

#### QWEN4-1 · qwen4_exp read, bound and golden

| Field | Value |
|---|---|
| Phase | qwen4 |
| Labels | `infrastructure` `model:qwen4exp` |
| Depends on | INTAKE-1, INTAKE-4, STUDY-3 |
| Touches | `crates/logicaffeine_compile/src/model/spec.rs`, `crates/logicaffeine_compile/src/model/bind.rs` and `crates/logicaffeine_compile/src/model/reference.rs` through S12; the qwen4_exp locks (new) |
| Status | open |

**Goal.** Read Qwen3.8-Flash-Next's checkpoint into a spec that carries every section 7.1 key, bind every tensor of its
table, extend the f64 reference with each new component, and record goldens from transformers, vLLM and llama.cpp on
the tiny random checkpoint [TINYQ4] and on a short prompt of the real checkpoint.

**Read first.**
- Section 7 of this book.
- `external/competitors/llama.cpp/conversion/qwen4exp.py`: the tensor names.

**RED tests.**
- [x] `qwen4_exp_intake_lock::the_spec_carries_every_configuration_key`: the twin (`hf-tiny-twin.py qwen4exp`) reads into
  `ModelSpec::{hyper, ple, indexer}` and `GatedDelta::gate`, as its converted GGUF does but for QSA's layers: the
  converter reads the configuration through transformers, which names a QSA layer `indexed_attention`, matches
  `full_attention` alone and writes the stack's QSA layers a ratio of 0, so llama.cpp attends them densely; the reader
  keeps QSA on them, as transformers computes (`logs/samba/red-qwen4exp-1.log`, `logs/samba/green-qwen4exp-6.log`).
- [x] `qwen4_exp_intake_lock::every_tensor_of_the_table_is_bound`: every key and tensor as the converter writes it,
  nothing unread, and the reference's step reads every tensor but the draft block's. Gated DeltaNet's `-exp(A_log)`
  is held within one f32 step: the converter takes torch's exponential, MKL's on this box (1.15625 gives
  3.1779933 where the correctly rounded value is 3.1779935), which no portable reader reproduces bit for bit.
- [x] `qwen4_exp_intake_lock::the_reference_equals_transformers_on_the_tiny_checkpoint`: within a millionth of the
  largest logit at positions before and after an end of document and with QSA keeping every block, then two of
  three and two of four, the prompt whole and a token at a time.

**Edge cases.**
- [x] The vision tower's tensors present in a text-only run: bound, unused (skipped as the converter skips them;
  `every_tensor_of_the_table_is_bound` leaves nothing unread).
- [ ] The draft head's tensors: bound to the draft cut (AUTHOR-3). They read as the converter writes them (`eh_proj`
  the two projections side by side, its own hyper-connection mix); the draft cut is AUTHOR-3's.
- [x] The n-gram table read lazily, never loaded whole: the reader presents its shards as one table read by row, and
  a step gathers one row a head a token (`every_tensor_of_the_table_is_bound`).
- [x] Every part the author does not write is refused by name, by the author and by `largo emit rdu`
  (`qwen4_exp_intake_lock::every_part_the_author_does_not_write_is_refused_by_name`); the published configuration
  reads (`model_hf_intake_lock::every_published_configuration_reads`).

**Implementation.** The spec arm; the bind map; the reference's new components, each checked against its reference
implementation's tests under `external/competitors/vllm/tests/models/qwen4_exp`.

**Success.**
- [ ] The reference equals transformers on the tiny checkpoint and llama.cpp on the real one's first tokens (receipt:
  the lock).

**Mutants to kill.**
- [ ] Two hash heads' offsets swapped.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test qwen4_exp_intake_lock
```

**Done when.** Qwen3.8-Flash-Next has a reference we hold every chip to.

#### QWEN4-2 · Hyper-connections as a row section and an epilogue

| Field | Value |
|---|---|
| Phase | qwen4 |
| Labels | `primitive` `rdu` `model:qwen4exp` |
| Depends on | QWEN4-1, SCHED-1, FUSIONS:PARITY-HYPER-CONNECTIONS |
| Touches | the hyper-connection lowering (new) |
| Status | open |

**Goal.** Lower a hyper-connection as one section (norm over 10,240, the rank-320 down and up products, the gate, the
lane mean, the injection weights) and its injection as the epilogue of the sublayer's last product, with the 10,240-wide
residual kept in SRAM across the layer.

**Read first.**
- Section 7.2's hyper-connection math and `external/competitors/vllm/vllm/models/qwen4_exp/common/hyperconnection.py`.

**RED tests.**
- [ ] `qwen4_exp_hyper_lock::a_mixer_equals_the_reference`: fails today.
- [ ] `qwen4_exp_hyper_lock::the_injection_rides_the_sublayers_last_product`: fails today.
- [ ] `qwen4_exp_hyper_lock::the_wide_residual_stays_on_chip_across_a_layer`: fails today.

**Edge cases.**
- [ ] The final mixer (the model's last norm).
- [ ] A batch of 64: D, U and J read once a step for all rows.

**Implementation.** The shared rule from the fusion book; a section template with two skinny products and row ops;
the injection as an outer-product epilogue.

**Success.**
- [ ] Equal to the reference at real width; one read of the mixer weights a step (receipt: the lock).

**Mutants to kill.**
- [ ] The lane mean taken before the gate.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test qwen4_exp_hyper_lock
```

**Done when.** qwen4_exp's residual stream runs on chip.

#### QWEN4-3 · The n-gram hash, exact on 32-bit lanes

| Field | Value |
|---|---|
| Phase | qwen4 |
| Labels | `primitive` `rdu` `model:qwen4exp` |
| Depends on | QWEN4-1, KERNEL-6 |
| Touches | the n-gram lowering (new) |
| Status | open |

**Goal.** The ids are `((t0·m0) ⊕ (t1·m1) ⊕ (t2·m2)) mod V_h` over 64-bit integers; the RDU's integer type is 32 bits.
Lower the hash exactly, two ways, and keep the faster: limb arithmetic on 32-bit lanes (KERNEL-6), and per-token product
tables `P_k[t]` gathered and combined by limb-wise exclusive-or with the modulus by limbs. Prove both equal to the 64-bit
definition.

**Read first.**
- `external/competitors/vllm/vllm/models/qwen4_exp/common/ngram_embedding.py`

**RED tests.**
- [ ] `qwen4_exp_ngram_lock::the_hash_equals_the_reference_on_every_input_class`: fails today.
- [ ] `qwen4_exp_ngram_lock::both_lowerings_are_proved_equal_to_the_definition`: fails today.

**Edge cases.**
- [ ] The largest token id times the largest multiplier.
- [ ] End-of-sequence padding for the first two positions.
- [ ] A head vocabulary that is not a power of two.

**Implementation.** KERNEL-6's 64-bit primitives; the product tables generated at load; the proof by SMT over bit
vectors, with the kernel rechecking it.

**Success.**
- [ ] Both lowerings equal the reference and carry a proof (receipt: the lock).

**Mutants to kill.**
- [ ] A carry dropped in the limb product.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test qwen4_exp_ngram_lock
```

**Done when.** The hash runs on chip, exact.

#### QWEN4-4 · The n-gram table in DDR, gathered ahead of the step

| Field | Value |
|---|---|
| Phase | qwen4 |
| Labels | `technique` `rdu` `model:qwen4exp` |
| Depends on | QWEN4-3, KERNEL-5 |
| Touches | the n-gram lowering (new) |
| Status | open |

**Goal.** Place the 97.7 GiB table in DDR, sharded by hash head across sockets, and issue each token's 16 row gathers as
soon as the previous step's arg-max lands, so their latency overlaps the embedding and the first layer, ahead of the
second layer that adds them.

**Read first.**
- `crates/logicaffeine_compile/src/spatial/residency.rs`

**RED tests.**
- [ ] `qwen4_exp_ngram_lock::the_table_is_placed_in_ddr_by_head`: fails today.
- [ ] `qwen4_exp_ngram_lock::the_gather_is_issued_before_layer_zero`: fails today.

**Edge cases.**
- [ ] A batch whose rows hit one shard.
- [ ] A speculative step: gathers for every draft position.

**Implementation.** MEM-1's DDR tier; KERNEL-5's hashed gather; the schedule hoists the gather to the step's start.

**Success.**
- [ ] The gather's latency hidden in the modeled step (receipt: the time model's attribution).

**Mutants to kill.**
- [ ] The gather issued after layer 2 begins.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test qwen4_exp_ngram_lock
```

**Done when.** A 51-billion-parameter table costs no step time.

#### QWEN4-5 · The per-layer embedding mixer and its convolution state

| Field | Value |
|---|---|
| Phase | qwen4 |
| Labels | `primitive` `rdu` `model:qwen4exp` |
| Depends on | QWEN4-4, QWEN4-2 |
| Touches | the n-gram lowering (new) |
| Status | open |

**Goal.** Lower what follows the gather: the key and value projections, the per-lane gate against the normalized
stream, the dilated depthwise convolution (kernel 4, dilation 3) with its state carried per sequence, and the addition
into the 10,240-wide stream.

**Read first.**
- `external/competitors/vllm/vllm/models/qwen4_exp/common/ple.py`

**RED tests.**
- [ ] `qwen4_exp_ngram_lock::the_mixer_equals_the_reference`: fails today.
- [ ] `qwen4_exp_ngram_lock::the_dilated_state_carries_across_steps`: fails today.

**Edge cases.**
- [ ] A sequence shorter than the dilated receptive field.
- [ ] A sequence that joins a batch mid-run.

**Implementation.** Products and row ops as sections; the dilated ring as the causal convolution's ring with a stride.

**Success.**
- [ ] Equal to the reference over a 64-token decode (receipt: the lock).

**Mutants to kill.**
- [ ] Dilation 1 where 3 belongs.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test qwen4_exp_ngram_lock
```

**Done when.** Layer 2 of qwen4_exp runs.

#### QWEN4-6 · Qwen sparse attention at 262,144 positions

| Field | Value |
|---|---|
| Phase | qwen4 |
| Labels | `primitive` `rdu` `model:qwen4exp` |
| Depends on | ARCH-SPARSE, QWEN4-1 |
| Touches | the qwen4_exp locks (new) |
| Status | open |

**Goal.** Bind ARCH-SPARSE to qwen4_exp's indexer (4 heads × 128, 1 KV head, pooling by 4, 512 blocks plus the tail)
and attention (24 query heads, 2 KV heads, 256 wide, gated), and run decode at 262,144 positions and, with YaRN, at
1,048,576.

**Read first.**
- `external/competitors/vllm/vllm/models/qwen4_exp/common/qsa_cache.py`

**RED tests.**
- [ ] `qwen4_exp_qsa_lock::selection_matches_the_reference_above_the_budget`: fails today.
- [ ] `qwen4_exp_qsa_lock::the_layer_is_dense_below_2051_tokens`: fails today.
- [ ] `qwen4_exp_qsa_lock::decode_runs_at_262144_positions`: fails today.

**Edge cases.**
- [ ] 2,050, 2,051 and 2,052 cached tokens.
- [ ] Score ties at the 512th block.

**Implementation.** ARCH-SPARSE's lowering with these shapes; the pooled-key cache as a carried table.

**Success.**
- [ ] Selection set equal to the reference's, or within the stated Jaccard bound with the bound recorded (receipt: the
  lock).

**Mutants to kill.**
- [ ] The tail block excluded.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test qwen4_exp_qsa_lock
```

**Done when.** qwen4_exp's attention layers run at full context.

#### QWEN4-7 · 512 narrow experts with a gated shared expert

| Field | Value |
|---|---|
| Phase | qwen4 |
| Labels | `primitive` `rdu` `model:qwen4exp` |
| Depends on | ARCH-SHARED-GATE, MULTI-2, QWEN4-1 |
| Touches | the qwen4_exp locks (new) |
| Status | open |

**Goal.** 512 experts of 640, top 10, plus a gated shared expert, spread across sockets by expert parallelism with the
dispatch form the time model chooses per batch.

**Read first.**
- Section 7.3's experts-touched table.

**RED tests.**
- [ ] `qwen4_exp_moe_lock::the_expert_layer_equals_the_reference`: fails today.
- [ ] `qwen4_exp_moe_lock::both_dispatch_forms_agree_at_batch_1_and_64`: fails today.
- [x] `spatial_moe_real_width_lock::a_router_row_past_one_accumulator_computes_the_block`,
  `…::at_kimi_k2_geometry_the_choice_lowers_and_only_the_chosen_experts_move`,
  `…::at_qwen4_exp_geometry_the_top_ten_of_512_lowers_and_only_the_chosen_experts_move`, and
  `spatial_moe_options_lock`'s `…_where_the_router_row_is_wider_than_a_page` (every option, routing and ungrouped choice
  at 300 experts, top 2, and 512, top 10): a router row wider than an accumulator page is scored in blocks of the row (a
  softmax from each block's maximum and sum, combined; the sigmoid, the router bias and the selection bias a block an
  item) and routed in two levels, each block's top K as candidates then the top K of the candidates in row order (the
  row's own top K, the lowest expert first among equal scores); more picks than a page holds are packed, stored and
  combined in pages. Each case computes the VM's mixture on the fp32 twin and within the bf16 bound on the chip, and at
  Kimi K2's (384 experts of 2048 × 7168) and qwen4_exp's (top 10 of 512 of 640 × 2560) geometries the block lowers whole
  with only the chosen experts' tiles moving; failed: `NoLegalTiling` on the router row, `MissingPrecision Int64` on the
  flat form (`logs/samba/red-wide-routing-head.log`) (receipt: `logs/samba/green-wide-options.log`,
  `logs/samba/wide-routing-3.log`). Kimi K2 lowers at real width (`deploy_kimi_glm_lock::every_kimi_k2_layer_kind_lowers_and_certifies`,
  landed from `deploy_reds_lock`; `logs/samba/green-wide-routing.log`). A group limit over a row this wide is refused.

**Edge cases.**
- [ ] All 10 picks on one socket. 471 experts touched at batch 128.

**Implementation.** MULTI-2's dispatch; experts of 640 as one tile-aligned product each.

**Success.**
- [ ] Equal to the reference; the chosen form the faster in the model (receipt: the lock).

**Mutants to kill.**
- [ ] The shared gate dropped.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test qwen4_exp_moe_lock
```

**Done when.** qwen4_exp's experts run across sockets.

#### QWEN4-8 · Gated delta with a sigmoid gate

| Field | Value |
|---|---|
| Phase | qwen4 |
| Labels | `primitive` `rdu` `model:qwen4exp` |
| Depends on | ARCH-GDN-VARIANTS, QWEN4-1 |
| Touches | the language parser (`with sigmoid gate`); `crates/logicaffeine_compile/src/tensor_recognize/qwen.rs` (`GateAct`, `GatedDeltaSpec::gate`); `crates/logicaffeine_compile/src/tensor_graph/eval.rs`; `crates/logicaffeine_compile/src/spatial/passes/from_qwen_ops.rs` (`delta_gate`); the CUDA shims and `crates/logicaffeine_compile/src/spatial/decode_engine/capability.rs`; `crates/logicaffeine_compile/src/model/author.rs`; `crates/logicaffeine_tests/tests/tensor_gated_delta_gate_lock.rs` |
| Status | the gate written, lifted and lowered; qwen4_exp's real layers over 64 steps owed |

**Goal.** Bind the gated-delta variant to qwen4_exp's 36 layers: 16 key heads, 48 value heads, sigmoid gate, readout
scale, float32 state of 108 MiB a sequence.

**Read first.**
- Section 7.2.

**RED tests.**
- [ ] `qwen4_exp_gdn_lock::the_layer_equals_the_reference_over_64_steps`: fails today.
- [x] `tensor_gated_delta_gate_lock::a_sigmoid_gate_takes_the_normed_head_times_sigmoid_of_z`,
  `…::the_compiled_tiers_take_the_sigmoid_gate`, `…::the_front_end_lifts_the_gate_into_the_op`,
  `…::the_graph_evaluator_computes_the_gated_step_as_the_vm_does`, `…::the_sn50_computes_a_sigmoid_gated_step`,
  `…::every_chip_without_the_gate_refuses_it_by_name_and_every_spatial_shim_resolves_it`,
  `…::a_model_whose_deltanet_a_chip_cannot_gate_is_refused_by_name`,
  `…::every_consumer_of_a_gated_delta_spec_is_held_to_its_gate`: the sugar `with sigmoid gate Z`, the gate
  (`GateAct`) lifted into the op, computed by the evaluator and the spatial forms (flat, blocked, chunked), refused by
  name by the CUDA lane and forward, the D5005 engine and Qwen3.8's CUDA configuration; failed: no gate in the op
  (`logs/samba/red-qwen4-8-head.log`) (receipt: `logs/samba/green-qwen4-8-3.log`).
- [x] `model_conformance_lock::a_sigmoid_gated_deltanet_prefills_and_decodes_to_the_reference`: the author writes the
  gate, a hybrid's prompt and step equal to the reference; failed: the author refused the gate
  (`logs/samba/red-qwen4-8-author-head.log`) (receipt: `logs/samba/green-qwen4-8-1.log`).

**Edge cases.**
- [ ] State residency at batch 64: 6.9 GB of state against the domain's SRAM.

**Implementation.** The variant record; MEM-4's residency for the state.

**Success.**
- [ ] Equal to the reference over 64 steps (receipt: the lock).

**Mutants to kill.**
- [ ] SiLU where sigmoid belongs (round `logs/samba/mutants-q48/`, 11 mutants: the desugar, the clause, the
  recognizer, the evaluator, the spatial gate and its sign, each refusal, the author's clause).

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test qwen4_exp_gdn_lock
```

**Done when.** qwen4_exp's gated-delta layers run.

#### QWEN4-9 · Interleaved multimodal rotation on a quarter of the head

| Field | Value |
|---|---|
| Phase | qwen4 |
| Labels | `primitive` `rdu` `model:qwen4exp` |
| Depends on | ARCH-ROPE, QWEN4-1 |
| Touches | the qwen4_exp locks (new) |
| Status | open |

**Goal.** Rotate the first 64 of 256 dimensions with interleaved sections [11, 11, 10] at θ 1e7, with three equal
position ids for text and distinct (t, h, w) ids for image tokens.

**Read first.**
- Section 7.2's rotation.

**RED tests.**
- [ ] `qwen4_exp_rope_lock::text_rotation_equals_the_reference`: fails today.
- [ ] `qwen4_exp_rope_lock::image_positions_rotate_as_the_reference`: fails today.

**Edge cases.**
- [ ] A text token after an image span: its position continues from the span's maximum.

**Implementation.** ARCH-ROPE's multimodal cache with three position streams.

**Success.**
- [ ] Equal to the reference for text and image tokens (receipt: the lock).

**Mutants to kill.**
- [ ] Sections in the wrong order.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test qwen4_exp_rope_lock
```

**Done when.** qwen4_exp's positions are right.

#### QWEN4-10 · The MTP head drafts on chip

| Field | Value |
|---|---|
| Phase | qwen4 |
| Labels | `serving` `rdu` `model:qwen4exp` |
| Depends on | SERVE-RDU-1, QWEN4-1 |
| Touches | the qwen4_exp locks (new) |
| Status | open |

**Goal.** qwen4_exp's one-layer draft head drafts k tokens on chip after each verified step; verification is a k + 1
row block; the gated-delta state and the n-gram history are checkpointed at each draft position and restored on a
rejection.

**Read first.**
- `external/competitors/vllm/vllm/models/qwen4_exp/nvidia/mtp.py`

**RED tests.**
- [ ] `qwen4_exp_mtp_lock::drafting_emits_the_greedy_tokens`: fails today.
- [ ] `qwen4_exp_mtp_lock::a_rejection_restores_every_carried_state`: fails today.

**Edge cases.**
- [ ] Rejection at the first draft; acceptance of all k.
- [ ] The T = 1 and T > 1 gated-delta paths: equal state, or the difference bounded and stated [MLXQ4].

**Implementation.** SERVE-RDU-1's machinery bound to this head and these states.

**Success.**
- [ ] Greedy-equal output; acceptance recorded at k = 1 to 9 (receipt: the lock).

**Mutants to kill.**
- [ ] The n-gram history not restored.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test qwen4_exp_mtp_lock
```

**Done when.** qwen4_exp speculates on chip.

#### QWEN4-11 · The vision tower

| Field | Value |
|---|---|
| Phase | qwen4 |
| Labels | `primitive` `rdu` `model:qwen4exp` |
| Depends on | ARCH-VISION, QWEN4-1 |
| Touches | the qwen4_exp locks (new) |
| Status | open |

**Goal.** Bind ARCH-VISION to qwen4_exp's tower (depth 27, width 1152, patch 16, merge 2, temporal patch 2) and run an
image prompt end to end.

**Read first.**
- Section 7.1's `vision_config`.

**RED tests.**
- [ ] `qwen4_exp_vision_lock::an_image_prompt_equals_the_reference`: fails today.

**Edge cases.**
- [ ] Two images in one prompt.

**Implementation.** The tower as a program of its own in the deployment; its output spliced at the image tokens.

**Success.**
- [ ] Equal to the reference (receipt: the lock).

**Mutants to kill.**
- [ ] Image tokens spliced one position late.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test qwen4_exp_vision_lock
```

**Done when.** qwen4_exp takes images.

#### QWEN4-12 · The whole model as a deployment

| Field | Value |
|---|---|
| Phase | qwen4 |
| Labels | `infrastructure` `rdu` `model:qwen4exp` |
| Depends on | QWEN4-2, QWEN4-5, QWEN4-6, QWEN4-7, QWEN4-8, QWEN4-9, MULTI-5 |
| Touches | the qwen4_exp locks (new) |
| Status | open |

**Goal.** Emit Qwen3.8-Flash-Next as a certified deployment for decode, batch, prefill and the parallel plan the time
model picks; run the tiny bundle against the VM and the references; record every modeled cell; recompute section 7.3's
budgets from the bundle.

**Read first.**
- Sections 7.3 and 7.4.

**RED tests.**
- [ ] `qwen4_exp_deploy_lock::the_tiny_bundle_equals_the_vm_and_the_references`: fails today.
- [ ] `qwen4_exp_deploy_lock::the_budgets_are_the_bundles`: fails today.
- [ ] `qwen4_exp_deploy_lock::every_cell_has_a_modeled_row`: fails today.

**Edge cases.**
- [ ] The parallel plan at 8, 16 and 64 sockets.

**Implementation.** The generic deployment path (INTAKE-6) with every component above.

**Success.**
- [ ] Certified, equal, priced (receipt: the lock and the cell ledger).

**Mutants to kill.**
- [ ] One layer bound to another layer's weights.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test qwen4_exp_deploy_lock
```

**Done when.** The Qwen 4 preview runs on the RDU, end to end.

#### QWEN4-13 · The Qwen 4 day-zero drill

| Field | Value |
|---|---|
| Phase | qwen4 |
| Labels | `exam` `model:qwen4exp` |
| Depends on | QWEN4-12, INTAKE-7 |
| Touches | the drill script (new) |
| Status | open |

**Goal.** Section 7.6's drill as a script and a lock: fetch a configuration and tensor table, diff against qwen4_exp,
build the tiny twin, record goldens, read, author, emit, certify, run, price, clock. Rehearsed on Qwen3.8-Flash-Next and
on a configuration with one invented key.

**Read first.**
- Section 7.6.

**RED tests.**
- [ ] `qwen4_exp_drill_lock::the_drill_runs_end_to_end_on_the_preview`: fails today.
- [ ] `qwen4_exp_drill_lock::an_invented_key_stops_the_drill_by_name`: fails today.

**Edge cases.**
- [ ] A released checkpoint that changes a shape only: the drill finishes without a code change.

**Implementation.** A script over the commands the earlier tickets built; the clock from INTAKE-7.

**Success.**
- [ ] The drill's time on the preview recorded (receipt: the cell ledger's BRINGUP row).

**Mutants to kill.**
- [ ] A drill that skips the golden comparison.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test qwen4_exp_drill_lock
```

**Done when.** The day Qwen 4 is published is a scripted, timed run.
### 11.11 Kernels

The primitives the exam's micro-tests poke (section 10.11), each a spatial lowering with its certificate.

#### KERNEL-1 · Top-k over a long row

| Field | Value |
|---|---|
| Phase | kernel |
| Labels | `primitive` `rdu` `area:sampling` |
| Depends on | SCHED-1 |
| Touches | the attention family's lowering file (new) |
| Status | open |

**Goal.** Indexed sparse attention selects 512 of up to 65,536 pooled blocks; routing selects 8 of 256 or 10 of 512;
sampling selects top-k of 248,320. Lower a top-k selection: a threshold search (a radix or histogram select over
order-preserving keys) that finds the k-th value in a fixed number of passes, then a compaction of the values above it,
with ties broken in the reference's order.

**Read first.**
- `crates/logicaffeine_tests/tests/spatial_blocked_argmax_lock.rs`: the blocked arg-max this generalizes.
- The fusion book's TECH-RADIX-TOPK-KEYS.

**RED tests.**
- [ ] `spatial_topk_lock::top_512_of_65536_equals_the_vm`: fails today: no selection.
- [ ] `spatial_topk_lock::ties_break_in_the_references_order`: fails today.

**Edge cases.**
- [ ] k equal to n. k of 1 (the arg-max). All values equal. NaN in the row.

**Implementation.** Order-preserving integer keys; per-PCU histograms merged by a tree; the threshold pass; a gather
compaction.

**Success.**
- [ ] Equal to the VM for every edge case (receipt: the lock).

**Mutants to kill.**
- [ ] A histogram bin off by one.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_topk_lock
```

**Done when.** Selection is a primitive on the RDU.

#### KERNEL-2 · Sort and permutation of tokens by expert

| Field | Value |
|---|---|
| Phase | kernel |
| Labels | `primitive` `rdu` `area:moe` |
| Depends on | SCHED-1 |
| Touches | the expert family's lowering file (new) |
| Status | open |

**Goal.** Batched expert layers group tokens by expert so each expert's weights are read once for its tokens. Lower a
counting sort by expert id with its inverse permutation for the combine.

**Read first.**
- The fusion book's PARITY-MOE-DIRECT-PAIRS (when not to sort).

**RED tests.**
- [ ] `spatial_moe_dispatch_lock::tokens_sorted_by_expert_equal_the_vm`: fails today.

**Edge cases.**
- [ ] An expert with no tokens. Every token on one expert.

**Implementation.** Count, prefix-sum, scatter; the inverse kept for the combine.

**Success.**
- [ ] Equal to the VM; each expert's weights read once a step (receipt: the lock).

**Mutants to kill.**
- [ ] A prefix sum that starts at 1.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_moe_dispatch_lock
```

**Done when.** Batched experts read their weights once.

#### KERNEL-3 · Segmented and chunked scans

| Field | Value |
|---|---|
| Phase | kernel |
| Labels | `primitive` `rdu` `area:recurrent` |
| Depends on | SCHED-1 |
| Touches | the recurrent family's lowering file (new) |
| Status | open |

**Goal.** State-space models, RWKV and lightning attention are scans with a carried state. Lower a scan in step form,
in chunked form (intra-chunk work, inter-chunk carry), and segmented by sequence for ragged batches.

**Read first.**
- `crates/logicaffeine_tests/tests/tensor_gdn_row_blocks_lock.rs`: the chunked gated-delta form.

**RED tests.**
- [ ] `spatial_scan_lock::a_chunked_scan_equals_the_step_scan`: fails today.
- [ ] `spatial_scan_lock::a_segmented_scan_resets_at_each_sequence`: fails today.

**Edge cases.**
- [ ] A chunk of one row. A segment of zero rows.

**Implementation.** The scan operator as a carried fold; chunks over the contraction axis.

**Success.**
- [ ] Both forms equal the VM (receipt: the lock).

**Mutants to kill.**
- [ ] A carry not reset at a segment boundary.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_scan_lock
```

**Done when.** Every recurrence has a fast form.

#### KERNEL-4 · Scatter with conflicts resolved by a fold

| Field | Value |
|---|---|
| Phase | kernel |
| Labels | `primitive` `rdu` `area:moe` |
| Depends on | SCHED-1 |
| Touches | the expert family's lowering file (new) |
| Status | open |

**Goal.** The eval refuses a scatter (an indirect output map) today. Lower scatter with repeated indices combined by a
fold (sum, max), as the expert combine and sparse updates need.

**Read first.**
- `crates/logicaffeine_compile/src/spatial/eval.rs`: the indirect gather this mirrors.

**RED tests.**
- [ ] `spatial_scatter_lock::a_scatter_with_repeats_sums_as_the_vm`: fails today: refused.

**Edge cases.**
- [ ] Every index the same. An index out of range: refused, never wrapped.

**Implementation.** A sorted scatter (KERNEL-2) followed by a segmented fold; or an owner-computes fold where the
target is on one site.

**Success.**
- [ ] Equal to the VM; out of range refused (receipt: the lock).

**Mutants to kill.**
- [ ] A repeated index overwritten instead of folded.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_scatter_lock
```

**Done when.** Scatter is a primitive.

#### KERNEL-5 · A hashed gather from a table in DDR

| Field | Value |
|---|---|
| Phase | kernel |
| Labels | `primitive` `rdu` `area:memory` |
| Depends on | MEM-1, KERNEL-6 |
| Touches | the DDR tier (new) |
| Status | open |

**Goal.** Gather rows of a table placed in DDR by computed ids, many small rows a step, with their latency overlapped:
the primitive under qwen4_exp's n-gram table and under cold experts.

**Read first.**
- `crates/logicaffeine_compile/src/spatial/residency.rs`

**RED tests.**
- [ ] `spatial_ddr_gather_lock::a_hashed_gather_equals_the_vm`: fails today.
- [ ] `spatial_ddr_gather_lock::the_gather_overlaps_the_next_sections`: fails today.

**Edge cases.**
- [ ] Two ids hitting one row. Ids spread across every shard.

**Implementation.** An indirect tile load whose source is the DDR tier; the time model prices its latency.

**Success.**
- [ ] Equal to the VM; latency hidden in the modeled step (receipt: the lock).

**Mutants to kill.**
- [ ] A gather issued after its first reader.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_ddr_gather_lock
```

**Done when.** A table too large for HBM is a gather away.

#### KERNEL-6 · 64-bit integer arithmetic on 32-bit lanes

| Field | Value |
|---|---|
| Phase | kernel |
| Labels | `primitive` `rdu` |
| Depends on | none |
| Touches | the integer lowering (new); `crates/logicaffeine_tests/tests/rdu_index_datatype_lock.rs` |
| Status | done |

**Goal.** The RDU's integer type is 32 bits; hashes (qwen4_exp), large indices and some samplers need 64. Lower 64-bit
add, multiply, exclusive-or, shift and modulus by a 32-bit-or-smaller divisor as limb arithmetic on 32-bit lanes, each
proved equal to its 64-bit definition.

**Read first.**
- `crates/logicaffeine_tests/tests/rdu_index_datatype_lock.rs`: the Int32 ruling.

**RED tests.**
- [x] `spatial_int64_lock::every_operation_equals_its_64_bit_definition`: fails today (receipt: green `logs/samba/green-3.log`; the RED was not run before `spatial::wide_int` landed, and `a_wrong_program_has_no_proof` with the dropped-carry mutant stands for it).
- [x] `spatial_int64_lock::every_lowering_carries_a_proof`: fails today (receipt: green `logs/samba/int64-proofs-2.log`, 144 identities accepted by the first kernel, 3 also decided by the second; the RED was not run before the module landed).

**Edge cases.**
- [x] Products near 2^63. A modulus by 1. A modulus near 2^31 (receipt: `spatial_int64_lock::the_edge_cases_hold_exactly`).

**Implementation.** `spatial::wide_int`: 16-bit limbs so every limb product fits a lane; carries limb by limb; Barrett
reduction for the modulus. The arithmetic lowerings are proved by their algebra: the program evaluated symbolically
with an interval for every lane showing no operation wraps, its result an identity over the integers that the kernel
checks by ring normalization (the second kernel re-checks those small enough for its exact evaluation, and never
disagrees), and for the modulus the bounds of Barrett's difference and the two cases of its correction. The bitwise
lowerings and the shifts are proved by a certified equivalence check of their bit-blasted circuits (SAT sweeping, its
LRAT derivation replayed).

**Success.**
- [x] Every operation equal on every input class and proved (receipt: `spatial_int64_lock` in `logs/samba/green-3.log` and `logs/samba/int64-proofs-2.log`).

**Mutants to kill.**
- [x] A carry dropped (receipt: `logs/samba/mutants2/results.json`, kernel6-carry-dropped killed).

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_int64_lock --test rdu_index_datatype_lock
```

**Done when.** A 64-bit computation runs exact on a 32-bit chip.

#### KERNEL-7 · Reductions past a chip's accumulator budget

| Field | Value |
|---|---|
| Phase | kernel |
| Labels | `primitive` `rdu` |
| Depends on | SCHED-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/passes/from_qwen_ops.rs` |
| Status | open |

**Goal.** The blocked arg-max reduces 248,320 entries through a merge tree. Generalize it to every long reduction
(sum, max, log-sum-exp for sampling, norms over very wide rows), with a streaming merge where a merge's footprint caps
its fan-in.

**Read first.**
- `crates/logicaffeine_tests/tests/spatial_blocked_argmax_lock.rs`

**RED tests.**
- [ ] `spatial_long_reduction_lock::a_logsumexp_over_248320_equals_the_vm`: fails today.

**Edge cases.**
- [ ] A row of one element. A row whose length is prime.

**Implementation.** The arg-max's block-and-merge form parameterized by the fold.

**Success.**
- [ ] Every fold equal to the VM at vocabulary width (receipt: the lock).

**Mutants to kill.**
- [ ] A merge that drops the last block.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_long_reduction_lock
```

**Done when.** No reduction is too long.

#### KERNEL-8 · Sparse weights at a stated density

| Field | Value |
|---|---|
| Phase | kernel |
| Labels | `primitive` `rdu` `area:quant` |
| Depends on | SCHED-1 |
| Touches | the sparse-weight lowering (new) |
| Status | open |

**Goal.** The Plasticine kit lets a submitter set the share of zero weights [PTUT]; Capstan built sparse iteration
into the fabric [CAPSTAN21]. Lower products against weights stored sparse (unstructured CSR or blocked, and 2:4) so
work and bytes fall with density.

**Read first.**
- `crates/logicaffeine_tests/tests/pnr_certificate_lock.rs` and the 2:4 work under `crates/logicaffeine_compile/src/codegen_sva/`.

**RED tests.**
- [ ] `spatial_sparse_weight_lock::a_90_percent_sparse_product_equals_the_vm`: fails today.
- [ ] `spatial_sparse_weight_lock::work_falls_with_density`: fails today.

**Edge cases.**
- [ ] An all-zero row. A density of 0% (dense).

**Implementation.** A sparse weight format read through an indirect tile; the time model prices it.

**Success.**
- [ ] Equal to the VM; modeled work proportional to density (receipt: the lock).

**Mutants to kill.**
- [ ] A zero row skipped in the output.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_sparse_weight_lock
```

**Done when.** Sparsity is speed.

### 11.12 Schedule

How a deployment runs over time: SambaNova's own techniques (section 5.1) and Plasticine's metapipelines, on our IR.

#### SCHED-1 · One file per op family in the spatial lowerings

| Field | Value |
|---|---|
| Phase | schedule |
| Labels | `infrastructure` `rdu` |
| Depends on | none |
| Touches | `crates/logicaffeine_compile/src/spatial/passes/from_qwen_ops.rs`, `crates/logicaffeine_compile/src/spatial/link.rs` |
| Status | open |

**Goal.** Every op family's lowering is in one 12,661-line file, so the attention, expert and recurrent streams cannot
work at once. Cut it into one file per family (attention, experts, recurrent, convolution, norms and rows, gathers and
selection, products) and the link likewise, with every spatial artifact byte-identical.

**Read first.**
- `crates/logicaffeine_tests/tests/one_ir_identity_lock.rs`: the pinned artifacts that must not move.

**RED tests.**
- [ ] `spatial_one_ir_lock::every_op_family_lowers_from_its_own_file`: fails today: one file.

**Edge cases.**
- [ ] A helper two families share: in a shared module, owned by R7.

**Implementation.** Move code without changing it; re-run every pinned digest.

**Success.**
- [ ] Every pinned spatial artifact identical (receipt: `one_ir_identity_lock`).

**Mutants to kill.**
- [ ] One family's helper copied instead of shared.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test one_ir_identity_lock --test spatial_one_ir_lock
```

**Done when.** Three streams can lower three families at once.

#### SCHED-2 · Sections stream into each other

| Field | Value |
|---|---|
| Phase | schedule |
| Labels | `technique` `rdu` |
| Depends on | TIME-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/passes/compose.rs`, `crates/logicaffeine_compile/src/spatial/passes/pack.rs`, `crates/logicaffeine_compile/src/spatial/certificate.rs` |
| Status | open |

**Goal.** Every phase boundary is a barrier today. Where a consumer section reads its producer's output tile by tile
in order, replace the barrier with a FIFO so the consumer starts on the producer's first tile: one kernel a decoder
layer with no internal synchronization, as SambaNova's layer fusion runs [KL24]. The certificate proves the streamed
order; the time model prices it.

**Read first.**
- `crates/logicaffeine_compile/src/spatial/passes/pack.rs`: phase packing, whose next step this is.

**RED tests.**
- [ ] `spatial_streaming_lock::a_streamable_chain_has_no_barrier`: fails today.
- [ ] `spatial_streaming_lock::a_streamed_chain_equals_its_barriered_twin`: fails today.
- [ ] `spatial_streaming_lock::a_consumer_reading_out_of_order_keeps_its_barrier`: fails today.

**Edge cases.**
- [ ] A consumer that reads a tile twice. A producer whose tiles finish out of order. One tensor fanned out to 64 consumers.

**Implementation.** A streamability analysis over each boundary's tile order; FIFOs sized by the producer's rate; the
certificate's ordering obligations by edge instead of by barrier.

**Success.**
- [ ] A decoder layer of each Tier A family has no internal barrier (receipt: the lock).
- [ ] Synchronization boundaries a token reported (receipt: the time model).

**Mutants to kill.**
- [ ] A barrier removed where the consumer reads out of order.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_streaming_lock --test spatial_rendezvous_lock
```

**Done when.** A layer runs as one kernel.

#### SCHED-3 · The layer loop

| Field | Value |
|---|---|
| Phase | schedule |
| Labels | `technique` `rdu` |
| Depends on | SCHED-2 |
| Touches | `crates/logicaffeine_compile/src/spatial/streamed.rs`, `crates/logicaffeine_compile/src/spatial/ir.rs` |
| Status | open |

**Goal.** KernelLooping [KL24]: rewrite the chain of calls to one layer-kind program into one program with a pipelined
outer loop over layers, the activation carried on chip and double-buffered, the weights iterated, so a token takes a
handful of synchronizations instead of one a layer. For hybrid models, loop over the period of the layer pattern (three
gated-delta layers and one attention layer for qwen4_exp) rather than one kind. Match calls by how each parameter is
used across them (constant, read, carried, live, written), as the paper's lattice does.

**Read first.**
- The Kernel Looping paper's sections 3 and 4.
- `crates/logicaffeine_compile/src/spatial/streamed.rs`: today's per-layer rebinding.

**RED tests.**
- [ ] `spatial_layer_loop_lock::a_homogeneous_stack_is_one_looped_program`: fails today.
- [ ] `spatial_layer_loop_lock::a_hybrid_stack_loops_its_period`: fails today.
- [ ] `spatial_layer_loop_lock::a_looped_step_equals_the_streamed_step`: fails today.

**Edge cases.**
- [ ] A layer count that the period does not divide: the remainder as its own program.
- [ ] A parameter carried across two calls instead of one: refused for looping.

**Implementation.** A loop over the layer index in the IR, its weights addressed by the index; the activation as a
double-buffered carry.

**Success.**
- [ ] At most 4 synchronizations a token on looped Tier A deployments (receipt: the time model, the lock).

**Mutants to kill.**
- [ ] Layer i reading layer i+1's weights.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_layer_loop_lock --test qwen38_decode_bundle_lock
```

**Done when.** A token takes a handful of synchronizations.

#### SCHED-4 · Samples stream across layer boundaries

| Field | Value |
|---|---|
| Phase | schedule |
| Labels | `technique` `rdu` |
| Depends on | SCHED-3 |
| Touches | `crates/logicaffeine_compile/src/spatial/ir.rs`, `crates/logicaffeine_compile/src/spatial/passes/compose.rs` |
| Status | open |

**Goal.** BatchStreaming [DIAYN26]: in a batch, each sample enters the next layer as soon as it leaves this one,
through a loop buffer spread across PMUs, so the pipeline's warm-up is paid once a step, not once a layer.

**Read first.**
- The Dataflow paper's section 4.2.

**RED tests.**
- [ ] `spatial_layer_loop_lock::samples_stream_across_layers`: fails today.
- [ ] `spatial_layer_loop_lock::streamed_samples_equal_the_batch_step`: fails today.

**Edge cases.**
- [ ] A batch of 1 (no change). Samples of different lengths.

**Implementation.** The loop buffer as a carried table indexed by sample; read-after-write ordering per sample in the
certificate.

**Success.**
- [ ] The modeled step drops at batch 2 to 16 (receipt: the time model).

**Mutants to kill.**
- [ ] A sample read before its write.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_layer_loop_lock
```

**Done when.** Batches keep every unit busy.

#### SCHED-5 · The next layer's weights arrive during this one

| Field | Value |
|---|---|
| Phase | schedule |
| Labels | `technique` `rdu` `area:memory` |
| Depends on | SCHED-3 |
| Touches | `crates/logicaffeine_compile/src/spatial/passes/size_buffers.rs`, `crates/logicaffeine_compile/src/spatial/passes/synthesize_handshakes.rs` |
| Status | open |

**Goal.** Double-buffered PMUs move the next model tile while the current one computes [HC26]. In the layer loop,
prefetch layer i+1's weights into the second buffer while layer i runs, sized by the time model so HBM is never idle.

**Read first.**
- `crates/logicaffeine_compile/src/spatial/passes/size_buffers.rs`

**RED tests.**
- [ ] `spatial_layer_loop_lock::the_next_layers_weights_load_during_this_one`: fails today.

**Edge cases.**
- [ ] A layer whose weights exceed half the SRAM: tile-level prefetch.

**Implementation.** A prefetch stage in the loop body; buffers sized by bytes a layer and the HBM roof.

**Success.**
- [ ] HBM busy through the modeled step (receipt: the time model's attribution).

**Mutants to kill.**
- [ ] A prefetch that overwrites the buffer in use.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_layer_loop_lock
```

**Done when.** The memory never waits for the compute to ask.

#### SCHED-6 · Many tokens a launch

| Field | Value |
|---|---|
| Phase | schedule |
| Labels | `technique` `rdu` |
| Depends on | SCHED-3 |
| Touches | `crates/logicaffeine_compile/src/spatial/streamed.rs`, `crates/logicaffeine_compile/src/spatial/ir.rs` |
| Status | open |

**Goal.** ScheduleOffloading [DIAYN26]: the arg-max and the next token's embedding gather already run on chip. Close
the loop: M decode steps in one launch, with a stop predicate (end of sequence, a stop token, a length), and draft and
target schedules concatenated for speculative decoding.

**Read first.**
- The Dataflow paper's section 4.3.

**RED tests.**
- [ ] `spatial_decode_loop_lock::m_tokens_run_without_the_host`: fails today.
- [ ] `spatial_decode_loop_lock::a_stop_token_ends_the_loop`: fails today.

**Edge cases.**
- [ ] M = 1. A stop at the first token. The context's end inside the loop.

**Implementation.** A step loop around the deployment with the token as a carried scalar and a data-dependent exit.

**Success.**
- [ ] Equal to M host-driven steps, with zero host returns in between (receipt: the lock).

**Mutants to kill.**
- [ ] A loop that runs one step past the stop.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_decode_loop_lock
```

**Done when.** The host starts a generation and is not needed again.

#### SCHED-7 · Buffers sized by footprint, the head first

| Field | Value |
|---|---|
| Phase | schedule |
| Labels | `technique` `rdu` `area:memory` |
| Depends on | none |
| Touches | `crates/logicaffeine_compile/src/spatial/passes/size_buffers.rs` |
| Status | open |

**Goal.** Buffer sizing spends local memory greedily; the head's buffers take 412 of 432 MiB and push every recurrent
state off chip. Size buffers by the time model's marginal value of each byte, with the residency plan's needs reserved
first.

**Read first.**
- `crates/logicaffeine_compile/src/spatial/residency.rs`

**RED tests.**
- [ ] `spatial_residency_lock::the_head_leaves_room_for_the_state`: fails today.

**Edge cases.**
- [ ] A program whose minimum footprint exceeds SRAM: refused with the footprint named.

**Implementation.** Minimum depths first; the residency reservation; remaining bytes to the buffers whose depth the
time model values most.

**Success.**
- [ ] Qwen3.8's state on chip at batch 1 with the head's buffers within their share (receipt: the lock).

**Mutants to kill.**
- [ ] Greedy sizing restored.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_residency_lock
```

**Done when.** SRAM goes where it saves the most time.

#### SCHED-8 · Compile time inside a budget at 70B, 405B and 671B

| Field | Value |
|---|---|
| Phase | schedule |
| Labels | `infrastructure` `rdu` |
| Depends on | INTAKE-6 |
| Touches | `crates/logicaffeine_compile/src/spatial/pipeline.rs` |
| Status | open |

**Goal.** Compile time is linear in sections today (the TP8 × batch-22 bundle in 1:16). Hold a budget for the largest
Tier A models, compile each distinct layer kind once (one kernel for every repeated layer, as kernel looping runs
them [KL24]), recompile only what a batch or
context change moves, and emit byte-identical bundles on every run.

**Read first.**
- `crates/logicaffeine_tests/tests/spatial_compile_scaling_timing_lock.rs`

**RED tests.**
- [ ] `spatial_compile_budget_lock::llama_405b_compiles_inside_the_budget`: fails today: unmeasured.
- [ ] `spatial_compile_budget_lock::a_checkpoint_compiles_to_identical_bytes`: fails today: unmeasured.
- [ ] `spatial_compile_budget_lock::a_new_batch_size_recompiles_only_what_it_moves`: fails today.

**Edge cases.**
- [ ] 61 layers with 3 dense: two kinds. A 10,000-node graph.

**Implementation.** Kind dedup by digest; a compile cache keyed by kind digest and shape; budgets recorded in the
ledger.

**Success.**
- [ ] Every Tier A model compiles inside the budget, byte-stable (receipt: the lock).

**Mutants to kill.**
- [ ] A cache keyed without the shape.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_compile_budget_lock --test spatial_compile_scaling_timing_lock
```

**Done when.** Compile time is never what a test is waiting on.

#### SCHED-9 · A program larger than a chip splits itself

| Field | Value |
|---|---|
| Phase | schedule |
| Labels | `technique` `rdu` |
| Depends on | TIME-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/passes/tile.rs`, `crates/logicaffeine_compile/src/spatial/passes/place.rs` |
| Status | open |

**Goal.** SARA [SARA21] partitions a program onto virtual units and merges them to fit. Do the same: a section that
needs more units or SRAM than a chip has is split into virtual sections, placed, and merged where they fit, with the
split chosen by the time model.

**Read first.**
- The SARA paper (section 12.3).

**RED tests.**
- [ ] `spatial_partition_lock::a_layer_larger_than_sram_splits_and_equals_the_vm`: fails today: refused.

**Edge cases.**
- [ ] A section that splits across chips (MULTI-3).

**Implementation.** Virtual units in the IR; a partitioner over the time model; merging under the certificate.

**Success.**
- [ ] Every Tier A layer at real width fits by splitting where it must (receipt: the lock).

**Mutants to kill.**
- [ ] A split that drops a partial result.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_partition_lock
```

**Done when.** Size never refuses a program.

#### SCHED-10 · Fused classes accepted whole by the spatial shim

| Field | Value |
|---|---|
| Phase | schedule |
| Labels | `technique` `rdu` |
| Depends on | SCHED-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/shim.rs`, `crates/logicaffeine_compile/src/spatial/link.rs` |
| Status | open |

**Goal.** `SpatialShim::fits` rejects every fused symbol and the link rejects anything not primitive, so the nine
classes the shim calls native reach the chip as parts. Accept each fused class whose spatial lowering is one section
(norm epilogues, gated-delta whole, flash decode, the expert pair), with a fusion report a section lists every class
fired and declined.

**Read first.**
- `crates/logicaffeine_compile/src/spatial/shim.rs`
- The fusion book's OPT-3 (the fusion report).

**RED tests.**
- [ ] `shim_lock::the_spatial_shim_accepts_its_native_classes_whole`: fails today: every fused symbol refused.
- [ ] `spatial_one_ir_lock::every_section_reports_its_fusions`: fails today.

**Edge cases.**
- [ ] A class native on one target and decomposed on another.
- [ ] A novel op written in LOGOS: runs as a generic section, never refused for being new.

**Implementation.** Per-class spatial links (one file a class, as the fusion book lays out); the report from the
optimizer's record.

**Success.**
- [ ] The nine classes accepted whole and equal to their parts (receipt: the locks).

**Mutants to kill.**
- [ ] A fused class accepted whose link computes the parts in the wrong order.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test shim_lock --test spatial_one_ir_lock --test numerics_contract_lock
```

**Done when.** The shared optimizer's fusions reach the RDU.

#### SCHED-11 · A whole model resident and pipelined

| Field | Value |
|---|---|
| Phase | schedule |
| Labels | `technique` `rdu` `plasticine` |
| Depends on | SCHED-2 |
| Touches | `crates/logicaffeine_compile/src/spatial/passes/place.rs`, `crates/logicaffeine_compile/src/spatial/passes/compose.rs` |
| Status | open |

**Goal.** The Plasticine kit's Model Analyzer maps a network spatially: every layer resident at once, stages pipelined
over samples, throughput one over the slowest stage, latency depth times it [PTUT]. Where a model's weights fit the
chips' SRAM (a CNN, a small language model, a dense spine across a large domain), place every layer at once, balance
the stages by the time model, and stream samples through.

**Read first.**
- Section 2.1's kit measures.

**RED tests.**
- [ ] `spatial_resident_model_lock::squeezenet_runs_resident_and_pipelined`: fails today.
- [ ] `spatial_resident_model_lock::throughput_is_one_over_the_slowest_stage`: fails today.

**Edge cases.**
- [ ] A model that fits only across several chips. A stage much slower than the rest: split (SCHED-9).

**Implementation.** One placement for the whole program; stage balancing by unit allocation; streaming boundaries
(SCHED-2) between every stage.

**Success.**
- [ ] SqueezeNet's throughput and latency reported in the kit's form (receipt: the lock).

**Mutants to kill.**
- [ ] A stage given no units.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_resident_model_lock
```

**Done when.** A model that fits runs the way Plasticine was built to run it.

#### SCHED-12 · Directives that pin the compiler's choices

| Field | Value |
|---|---|
| Phase | schedule |
| Labels | `technique` `rdu` |
| Depends on | SCHED-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/pipeline.rs`; the directive reader (new) |
| Status | open |

**Goal.** SambaFlow lets an engineer overrule its compiler: human-decision files fix what the heuristics would choose
[SNSW24] [ALCF], and looping decorators scope the kernel a chain of calls fuses into [DIAYN26]. Spatial's scheduling
directives override the controller schedule its compiler infers [PLDI18]. Ours has no such surface: every tile plan,
section cut, parallelization factor and tier is the compiler's alone. A directive file beside a program pins any of
them; the passes take a pinned choice as a fixed point of their search, the certificate and the VM hold it as they hold
the compiler's own, and the report sets each pinned choice beside the one the cost model would have made.

**Read first.**
- Section 5.4's ledger rows for human-decision files, looping directives and user scheduling directives.
- `crates/logicaffeine_compile/src/spatial/passes/tile.rs` and `crates/logicaffeine_compile/src/spatial/residency.rs`:
  the choices a directive pins.

**RED tests.**
- [ ] `spatial_directives_lock::a_pinned_tile_plan_is_the_one_placed`: fails today: no directive is read.
- [ ] `spatial_directives_lock::a_pinned_choice_still_certifies_and_equals_the_vm`: fails today.
- [ ] `spatial_directives_lock::a_directive_the_target_cannot_honor_is_refused_with_its_limit`: fails today.

**Edge cases.**
- [ ] A tile wider than a site's memory: refused, naming the memory and the tile.
- [ ] A directive naming a tensor or a section the program does not have: refused, never ignored.
- [ ] Two directives that contradict each other: refused, naming both.
- [ ] A loop scope pinned before SCHED-3 lands: refused as not yet lowered.

**Implementation.** A directive table keyed by the IR's stable names (section shape digests, tensor names), read in
`lower_program_stored` and handed to the tiling, placement and residency passes as fixed choices; the compile report
lists each pinned choice with the cost model's price of it and of the choice it displaced.

**Success.**
- [ ] A pinned plan is placed, certified and equal to the VM (receipt: `spatial_directives_lock`).
- [ ] The report names a directive the cost model prices worse than its own choice (receipt: `spatial_directives_lock`).

**Mutants to kill.**
- [ ] A directive read and dropped.
- [ ] A pinned choice that skips the certificate.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_directives_lock
```

**Done when.** An engineer can overrule any choice the compiler makes, and what runs is still proved.

#### SCHED-13 · Transposes and banks as layouts

| Field | Value |
|---|---|
| Phase | schedule |
| Labels | `technique` `rdu` `plasticine` |
| Depends on | CHIP-3 |
| Touches | `crates/logicaffeine_compile/src/spatial/passes/tile.rs`, `crates/logicaffeine_compile/src/spatial/passes/size_buffers.rs` |
| Status | open |

**Goal.** The SN40L compiler moves no data to transpose it: a transpose is the access pattern of the buffer between
producer and consumer, a diagonally striped layout lets the row-major write and the column-major read both run at full
bandwidth, and double buffers are given banks statically so the writer and the reader never collide; the PMU's bank-bit
mapping and its interleaving across memory units are the compiler's to set [SN40L24]. A looped kernel materializes only
the part of a tensor a transpose or a reread after a reduction needs [DIAYN26]. Ours lowers a transpose as a stage of
affine band maps that copies the tensor (`from_transpose`), and records only how many banks a site's memory interleaves
over (`mem_banks`), not which bank a buffer takes.

**Read first.**
- Section 5.4's ledger rows for the SN40L's PMU and layout techniques.
- `crates/logicaffeine_compile/src/spatial/passes/from_qwen_ops.rs` `from_transpose`.

**RED tests.**
- [ ] `spatial_layout_lock::a_transpose_between_sections_moves_no_data`: fails today: the transpose is a copy.
- [ ] `spatial_layout_lock::a_striped_buffer_reads_both_ways_without_a_bank_conflict`: fails today: no layout is chosen.
- [ ] `spatial_layout_lock::the_two_halves_of_a_double_buffer_never_share_a_bank`: fails today.

**Edge cases.**
- [ ] A transpose whose consumer is on another chip: the data moves anyway, in the layout the receiver reads.
- [ ] A shape whose rows are not a multiple of the bank count.
- [ ] A tensor read in two orders by two consumers.

**Implementation.** Every buffer carries a layout (order, stripe, bank map) chosen with the tile plan; a stage's
address map composes with the layout of what it reads; the PMU bank model (CHIP-3) prices a conflict.

**Success.**
- [ ] A decode layer's transposes cost no stage, and the layer equals the VM (receipt: `spatial_layout_lock`).

**Mutants to kill.**
- [ ] A stripe off by one bank.
- [ ] Both halves of a double buffer in one bank.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_layout_lock
cargo nextest run -p logicaffeine-tests --test spatial_transpose_any_shape_lock
```

**Done when.** No transpose moves data a layout could have read in place.
### 11.13 Multi-chip

Sixteen sockets a rack and 256 in a scale-up domain [HC26]: the large models are multi-chip problems first.

#### MULTI-1 · Collectives as dataflow, overlapped

| Field | Value |
|---|---|
| Phase | multi-chip |
| Labels | `technique` `rdu` `area:comm` |
| Depends on | TIME-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/tensor_parallel.rs`; collectives (new) |
| Status | open |

**Goal.** The tensor-parallel all-reduce folds every partial on chip 0 and is unpriced. Lower all-reduce, reduce-scatter
and all-gather as dataflow over the link topology (ring, tree, the eight-socket full mesh), streamed SRAM to SRAM and
overlapped with the next product, as SambaNova's P2P collectives run [DIAYN26] [STH26].

**Read first.**
- `crates/logicaffeine_compile/src/spatial/tensor_parallel.rs`: `allreduce_section`.

**RED tests.**
- [ ] `spatial_collectives_lock::a_ring_all_reduce_equals_the_central_fold`: fails today.
- [ ] `spatial_collectives_lock::the_all_reduce_overlaps_the_next_product`: fails today.
- [ ] `spatial_collectives_lock::a_collective_missing_a_chip_is_refused`: fails today: shape untested beyond the fold.

**Edge cases.**
- [ ] 1, 2, 8, 16 and 64 chips. A non-power-of-two count.
- [ ] Reduction order: fixed per chip count so results are deterministic (NUM-8).

**Implementation.** Collective algorithms chosen by topology and size; chunks pipelined with the producer; the
certificate's unreduced-shard taint rule kept.

**Success.**
- [ ] Equal to the central fold; overlap visible in the time model (receipt: the lock).

**Mutants to kill.**
- [ ] A ring step that skips a chip.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_collectives_lock --test qwen38_tensor_parallel_lock
```

**Done when.** Communication hides behind compute.

#### MULTI-2 · Expert parallelism, both dispatch forms

| Field | Value |
|---|---|
| Phase | multi-chip |
| Labels | `technique` `rdu` `area:moe` `area:comm` |
| Depends on | MULTI-1, KERNEL-2, FUSIONS:PARITY-EXPERT-PARALLEL |
| Touches | expert parallelism (new) |
| Status | open |

**Goal.** Place experts across sockets and dispatch tokens to them in either form SambaNova runs [HC26]:
broadcast-and-filter (every socket sees every token, computes its local picks, the partials fold in the next
all-reduce) and all-to-all dispatch and combine. The time model picks per batch and per model.

**Read first.**
- The fusion book's PARITY-EXPERT-PARALLEL.

**RED tests.**
- [ ] `spatial_expert_parallel_lock::both_forms_equal_the_unsharded_layer`: fails today.
- [ ] `spatial_expert_parallel_lock::the_faster_form_is_chosen`: fails today.

**Edge cases.**
- [ ] All tokens to experts on one socket. A shared expert replicated or sliced.

**Implementation.** Expert placement by load (MEM-6); the two dispatch programs; a choice by the time model.

**Success.**
- [ ] Equal for DeepSeek-V3 and qwen4_exp tiny configurations at 8 and 16 sockets (receipt: the lock).

**Mutants to kill.**
- [ ] A token dispatched to a socket without its expert.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_expert_parallel_lock
```

**Done when.** Mixtures spread across sockets.

#### MULTI-3 · Pipeline parallelism across nodes

| Field | Value |
|---|---|
| Phase | multi-chip |
| Labels | `technique` `rdu` `area:comm` |
| Depends on | MULTI-1 |
| Touches | pipeline parallelism (new) |
| Status | open |

**Goal.** Stages of layers on different nodes, activations handed over the inter-node fabric, micro-batches in flight,
for models larger than one node's memory.

**Read first.**
- Section 4.2's scale-up topology.

**RED tests.**
- [ ] `spatial_pipeline_parallel_lock::two_stages_equal_one`: fails today.

**Edge cases.**
- [ ] Stages of unequal cost: balanced by the time model.

**Implementation.** Stage boundaries as cross-node edges; micro-batch scheduling in the time model.

**Success.**
- [ ] Equal to the single-node program (receipt: the lock).

**Mutants to kill.**
- [ ] A micro-batch's activation handed to the wrong stage.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_pipeline_parallel_lock
```

**Done when.** Model size is not limited by a node.

#### MULTI-4 · Tensor parallelism at widths the heads do not divide

| Field | Value |
|---|---|
| Phase | multi-chip |
| Labels | `technique` `rdu` `area:comm` |
| Depends on | none |
| Touches | `crates/logicaffeine_compile/src/model/author.rs` (its tensor-parallel section, through S12), `crates/logicaffeine_compile/src/spatial/tensor_parallel.rs` (`TensorParallelShim`), `crates/logicaffeine_compile/src/spatial/pipeline.rs` (the walk's optimizer, through S1), `crates/logicaffeine_tests/tests/qwen38_tensor_parallel_lock.rs` |
| Status | open |

**Goal.** Qwen3.8-27B admits widths {1, 2, 4, 8}, so half of a 16-socket rack serves a replica. Shard heads unevenly
(some chips one more head), or shard the products by columns independent of heads, so every socket of the domain works
on one model.

**Read first.**
- `crates/logicaffeine_compile/src/spatial/fit.rs`: `qwen38_admits_tp`.

**RED tests.**
- [x] `qwen38_tensor_parallel_lock::twenty_four_heads_shard_over_sixteen_chips` and `…_in_a_batch`: failed (refused:
  "the language cannot write a tensor-parallel width that does not divide the query heads",
  `logs/samba/multi4-red.log`) (receipt: `logs/samba/multi4-hook-1.log`).

**Edge cases.**
- [x] One KV head shared by every chip (the lock's "one KV head on every chip").
- [x] Fewer Gated-DeltaNet heads than chips: the chips with no share of a layer fold zero partials (the lock's third
  configuration). The shared optimizer's elementwise regions had fused those partials, the fold chain and the residual
  into one op on the spine, dropping the all-reduce (`UnreducedShard`, `logs/samba/multi4-probe.log`); a
  tensor-parallel program now optimizes under `TensorParallelShim`, which refuses any fused op over two chips, a chip
  and the spine, or a collective's fold (`qwen38_tensor_parallel_lock::a_fusion_never_spans_two_chips_nor_holds_a_fold`).

**Implementation.** Each head count is shared over the chips with the heads that read one another kept together
(`author::share`): with no more chips than KV (key) heads a chip holds a balanced run of them and every query (value)
head reading them; with more, each KV (key) head is held by a balanced run of chips, replicated on each, and its query
heads (recurrences) split among them. The FFN's columns are balanced runs. A chip with no share of a layer folds the
empty sum — its replicated input scaled by zero — so every all-reduce still draws a partial from every chip and the
certificate's shard obligation is unchanged; the walk's fusions keep to one chip. At widths every count divides the programs and manifests are the ones
written before (`model_author_migration_lock`'s pinned digests). The serving stacks' rule stays `fit::admits_tp`; the
emitter's `--tp` and the serving plan still take it, and the time model that prices the imbalance is TIME-1's.

**Success.**
- [x] Equal to the unsharded program at 16 chips, the step and the batched step, certified over the 16 chips; every pinned
  spatial artifact unchanged (receipt: `logs/samba/multi4-hook-1.log`).

**Mutants to kill.**
- [x] The extra head assigned to two chips (receipt: `logs/samba/mutants5b/results.json`, multi4-every-holder-takes-the-first-parts killed).
- [x] A fused op over two chips admitted (receipt: `logs/samba/mutants5b/results.json`, tp-shim-fuses-across-chips killed).

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test qwen38_tensor_parallel_lock
```

**Done when.** Every socket works on one model.

#### MULTI-5 · The parallel plan chosen by the time model

| Field | Value |
|---|---|
| Phase | multi-chip |
| Labels | `technique` `rdu` |
| Depends on | MULTI-2, MULTI-3, TIME-2 |
| Touches | `crates/logicaffeine_compile/src/spatial/fit.rs` through R2; the plan search (new) |
| Status | open |

**Goal.** Choose tensor, expert, pipeline and data parallel widths for a model, a chip count and a regime by searching
the calibrated time model, and record the plan's predicted cells beside the alternatives.

**Read first.**
- `crates/logicaffeine_compile/src/spatial/fit.rs`: `qwen38_serving_plan`.

**RED tests.**
- [ ] `spatial_parallel_plan_lock::every_tier_a_model_gets_a_plan_at_8_16_and_64`: fails today.
- [ ] `spatial_parallel_plan_lock::the_chosen_plan_beats_every_alternative_in_the_model`: fails today.

**Edge cases.**
- [ ] A model that fits one socket: a plan of data parallelism only.

**Implementation.** Enumerate legal plans; price each; keep the best and the next three.

**Success.**
- [ ] A plan for every Tier A model at three counts (receipt: the lock).

**Mutants to kill.**
- [ ] A plan whose pipeline stages exceed the node count.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_parallel_plan_lock
```

**Done when.** No parallel plan is chosen by hand.

#### MULTI-6 · Link roofs and topology

| Field | Value |
|---|---|
| Phase | multi-chip |
| Labels | `infrastructure` `rdu` `area:comm` |
| Depends on | CHIP-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/tensor_parallel.rs` |
| Status | open |

**Goal.** Model the links: SN50's ten 800G ports (seven making the eight-socket mesh, two to the inter-node fabric,
2 TB/s a socket) and 400G scale-out [HC26]; SN40L's P2P [DIAYN26]. Every cross-chip edge priced on its link.

**Read first.**
- Section 4.2.

**RED tests.**
- [ ] `spatial_collectives_lock::every_cross_chip_edge_has_a_link_and_a_rate`: fails today.

**Edge cases.**
- [ ] Two sockets in different nodes: the inter-node path.

**Implementation.** A topology record from the facts; each edge mapped to a path.

**Success.**
- [ ] Every cross-chip edge priced (receipt: the lock).

**Mutants to kill.**
- [ ] An inter-node edge priced at the intra-node rate.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_collectives_lock
```

**Done when.** The network is in the model.

### 11.14 Numerics and formats

#### NUM-1 · fp32 islands in a bf16 datapath

| Field | Value |
|---|---|
| Phase | numerics |
| Labels | `technique` `rdu` `area:quant` |
| Depends on | none |
| Touches | `crates/logicaffeine_compile/src/spatial/value_range.rs`, `crates/logicaffeine_tests/tests/rdu_index_datatype_lock.rs` |
| Status | open |

**Goal.** The SN50 target runs everything in bf16 and drifts about 1.1% in relative logits against 0.3% for the same
source on Wormhole. Run softmax, norms, routers, the gated-delta state and long reductions in fp32 where the chip's
units allow (SN40L's PCUs compute in FP32 as well as BF16, INT32 and INT8), and products in bf16, so drift falls to the
accumulation's floor.

**Read first.**
- `crates/logicaffeine_tests/tests/rdu_index_datatype_lock.rs`: how a precision is admitted.

**RED tests.**
- [ ] `spatial_mixed_precision_reds_lock::the_sn50_drift_falls_to_wormholes`: fails today (frozen; RED receipt:
  `logs/samba/red-num1.log`): every harness shape's decode step drifts past an eighth over Wormhole's datapath —
  fp32 in every unit but the products', on the SN50's geometry — Qwen3.5 0.56% against 0.41%, Qwen3.5-MoE 0.87%
  against 0.70%, DeepSeek-V3 1.24% against 0.69%, Llama 4 0.91% against 0.41% (`logs/samba/num1-probe-2.log`; the
  Wormhole target itself lowers no decode step, its token gather needing the Int32 it lacks; the fp32 twin drifts by
  nothing measurable, so the floor is the bf16 products').
- [ ] `spatial_mixed_precision_reds_lock::every_island_is_fp32`: fails today (frozen; `logs/samba/red-num1.log`): an
  RMSNorm's output is bf16 on the SN50.
- [x] `spatial_mixed_precision_lock::qwen3next_decodes_on_the_fp32_twin_as_the_reference_computes`: Qwen3-Next's own
  checkpoint, its prompt prefilled and next token decoded with every program lowered for the SN50 with fp32 in place of
  bf16, is the f64 reference's within 1e-4 of the largest logit, where the SN50's bf16 bundle drifts 0.106 against a
  bound of 0.071 (E1.13): the lowering is exact and the drift is the datapath's (receipt: `logs/samba/q3next-fp32twin-2.log`).
  A target that offers fp32 beside bf16 still multiplies in bf16, since a float matmul takes the narrowest float its unit
  offers. So the fp32 twin replaces bf16 rather than adding fp32 to it. `common/samba_exam.rs`'s frozen `fp32_twin` (EXAM-2) adds
  it, so the E2 rows that use it multiply in bf16. Changing a frozen helper is Tristen's call.
- [x] `tensor_gdn_row_blocks_lock::a_real_width_step_fits_a_site_with_fp32_beside_its_bf16_products`: Qwen3.8's 48 value
  heads over 16 key heads of 128 at fp32 lower whole, certify and move the memory once; failed: `NoLegalTiling`, an
  item's fp32 buffers past a site (`logs/samba/red-gd-pages.log`) (receipt: `logs/samba/green-gd-pages.log`).
- [x] `spatial_grouped_decode_attention_lock::with_fp32_beside_its_bf16_products_each_kv_head_still_folds_once`: Qwen3.8's
  24 query heads over 4 K/V heads of 256 fold one item per K/V head at fp32; failed: the regroup of a K/V head's query
  heads did not fit a site, so the fold took an item per query head, 24 (`logs/samba/red-kv-cache.log`) (receipt:
  `logs/samba/green-regroup.log`).
- [x] `spatial_latent_tiles_lock::deepseek_v2_lites_latent_step_lowers_whole_with_fp32_beside_its_bf16_products`:
  DeepSeek-V2-Lite's latent step lowers whole and certifies in its forms at fp32; failed: its flash fold did not fit a
  site (`logs/samba/red-latent-fp32.log`, against the fold before its pages) (receipt: `logs/samba/green-fold.log`).
- [x] `spatial_mixed_precision_lock::an_input_two_widths_read_is_held_at_the_wider`: compose refused a tensor its readers
  declared at two widths of one float with nothing in the program writing it — on Wormhole a product reads its operands
  bf16 and a sum fp32, so an input both read did not compose (`logs/samba/red-compose-width.log`); it is held at the
  wider width and the narrower reader rounds it on the way in, as below a wider producer (receipt: `logs/samba/green-compose-width.log`).

**Edge cases.**
- [ ] An island whose input is a bf16 product: widened once.
- [ ] A chip that cannot witness fp32 in a unit: refused by name.

**Implementation.** Per-section precision from the numerics contract; the witness for fp32 added to the facts.
A first pass (an island float, fp32 wherever the target holds fp32 beside its products, taken by the norms, the
softmaxes, the arg-max's logits and the gated delta rule's decode sections) made every island fp32 and was reverted:
Qwen3.5's drift rose from 0.56% to 0.68%, three tiny bring-ups crossed their bound by a few thousandths, the 4×-wide
Qwen3.5's fp32 gated-delta state no longer tiled (`NoLegalTiling`, acc tiles 8), and 43 identity pins moved
(`logs/samba/green-num1-2.log`). Owed before the next pass: which island narrows into which bf16 reader, measured island
by island against the floor; the state's tiling at fp32 widths.
The state's tiling is done: the blocked gated delta holds one page of each buffer an item writes once (each a-block's
store, the norm weight, the gated output) where two pages do not fit a site, two otherwise, so a bf16 step is unchanged.
A decode attention's regroup of a K/V head's query heads and a flash fold's key, value and output pages take the same
rule (`paged` in `crates/logicaffeine_compile/src/spatial/passes/from_qwen_ops.rs`).
A second pass ran in a private copy (`logs/samba/num1-trial-1.log`, `logs/samba/num1-trial-2.log`). The chip facts take
the SIMD path's witness for fp32: [SN40L24] has each SIMD stage computing in FP32, BF16 and INT32, and its Llama 3.1
rates use mixed BF16/FP32 activations. The SN50's elementwise and row-reduce units take fp32 and its products stay
bf16. Seven handed REDs pass with it: both above, E1.7's 61 layers, E1.13's Qwen3-Next, E6.4's ties, E6.6's NaN row
and the four-times-wide tiny bring-ups. Two deployments' twins wait on it too, each a handed RED of
`deploy_reds_lock`: Gemma 3's (2.04% of the largest logit on the bf16 datapath, held with the witness:
`logs/samba/deploy-gemma-simd-fp32.log`) and Qwen 3.5's mixture's (2.54%, 1.34% with the witness:
`logs/samba/diag-qwen35moe-simd-fp32.log`). The real-width footprints at fp32 (the gated delta, a decode attention's
regroup, a flash fold) now fit by the paging above. Owed before it lands, each Tristen's word:
- where a KV cache is stored. Written at the pointwise float, a cache at fp32 is read at four bytes an element where
  its products take two (`spatial_grouped_decode_attention_lock`'s and `spatial_trip_count_lock`'s byte counts). It
  is a storage format, NUM-6's;
- gpt-oss's tiny bring-up. It drifts 0.175, 2.14% of its largest logit, against the 2% bound of the frozen
  `brings_up_close`, with no routing near-call: the closest is a margin of 0.124 on a score of 0.247, and flipping it
  moves the logits by 3.2 (`logs/samba/probe-gptoss-1.log`). The bound or the datapath decides it;
- the tests that hold the SN50 as the bf16 chip: `rdu_index_datatype_lock`'s tile datapath,
  `spatial_attention_real_width_lock`'s carries-bf16 rows, `spatial_algebra_semantics_lock`, `spatial_forward_lock`,
  and `spatial_moe_real_width_lock`'s crowded two-site target, whose 64 KB sites are sized for bf16 tiles;
- the identity and fabric-segment re-records, and `spatial_op_gap_ledger_lock`'s partial rotation, which now works
  and leaves the gap ledger.

**Success.**
- [ ] Drift at or under Wormhole's on every Tier A model (receipt: the lock).

**Mutants to kill.**
- [ ] The softmax island narrowed to bf16.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_mixed_precision_lock --test rdu_index_datatype_lock
```

**Done when.** Precision is spent where it buys accuracy.

#### NUM-2 · fp8 weights with block scales

| Field | Value |
|---|---|
| Phase | numerics |
| Labels | `primitive` `rdu` `area:quant` |
| Depends on | INTAKE-3 |
| Touches | the fp8 lowering (new) |
| Status | open |

**Goal.** SambaNova's DeepSeek and Llama 4 rows run FP8 [DIAYN26]. Read FP8 e4m3 weights with 128×128 block scales in
place, as `crates/logicaffeine_compile/src/spatial/passes/from_q8.rs` reads Q8_0, with the scale applied at the block's fold.

**Read first.**
- `crates/logicaffeine_compile/src/spatial/passes/from_q8.rs`

**RED tests.**
- [ ] `spatial_fp8_weights_lock::a_block_scaled_product_equals_the_vm`: fails today.
- [ ] `spatial_fp8_weights_lock::bytes_read_are_one_a_weight_plus_scales`: fails today.

**Edge cases.**
- [ ] A dimension not a multiple of 128. Saturated values.

**Implementation.** The Q8 fold generalized to a format record.

**Success.**
- [ ] Equal to the VM; bytes as stored (receipt: the lock).

**Mutants to kill.**
- [ ] A scale applied to the wrong block.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_fp8_weights_lock
```

**Done when.** FP8 checkpoints run at their size.

#### NUM-3 · MXFP4 experts

| Field | Value |
|---|---|
| Phase | numerics |
| Labels | `primitive` `rdu` `area:quant` |
| Depends on | none |
| Touches | the fp4 lowering (new) |
| Status | open |

**Goal.** gpt-oss ships MXFP4 experts; the format reader reads them already. Fold them in place on the RDU.

**Read first.**
- `crates/logicaffeine_compile/src/model/quant.rs`: the MXFP4 reader.

**RED tests.**
- [ ] `spatial_fp4_weights_lock::an_mxfp4_product_equals_the_vm`: fails today.

**Edge cases.**
- [ ] A shared exponent at its extreme.

**Implementation.** The format record's MXFP4 entry.

**Success.**
- [ ] Equal to the VM; bytes as stored (receipt: the lock).

**Mutants to kill.**
- [ ] The shared exponent ignored.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_fp4_weights_lock
```

**Done when.** gpt-oss runs at its size.

#### NUM-4 · NVFP4 weights

| Field | Value |
|---|---|
| Phase | numerics |
| Labels | `primitive` `rdu` `area:quant` |
| Depends on | INTAKE-3 |
| Touches | the fp4 lowering (new) |
| Status | open |

**Goal.** NVFP4 checkpoints (block scales in fp8, a tensor scale in fp32) folded in place.

**Read first.**
- INTAKE-3's reader.

**RED tests.**
- [ ] `spatial_fp4_weights_lock::an_nvfp4_product_equals_the_vm`: fails today.

**Edge cases.**
- [ ] A block scale of zero.

**Implementation.** The format record's NVFP4 entry.

**Success.**
- [ ] Equal to the VM (receipt: the lock).

**Mutants to kill.**
- [ ] The tensor scale dropped.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_fp4_weights_lock
```

**Done when.** NVFP4 checkpoints run at their size.

#### NUM-5 · 4-bit integer weights

| Field | Value |
|---|---|
| Phase | numerics |
| Labels | `primitive` `rdu` `area:quant` |
| Depends on | INTAKE-3 |
| Touches | the int4 lowering (new) |
| Status | open |

**Goal.** AWQ, GPTQ and Q4_K weights folded in place with their group scales and zero points.

**Read first.**
- INTAKE-3's reader.

**RED tests.**
- [ ] `spatial_int4_weights_lock::awq_gptq_and_q4k_products_equal_the_vm`: fails today.

**Edge cases.**
- [ ] GPTQ's activation-order permutation.

**Implementation.** The format record's int4 entries.

**Success.**
- [ ] Equal to the VM (receipt: the lock).

**Mutants to kill.**
- [ ] A zero point added with the wrong sign.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_int4_weights_lock
```

**Done when.** 4-bit checkpoints run at their size.

#### NUM-6 · An 8-bit KV cache

| Field | Value |
|---|---|
| Phase | numerics |
| Labels | `primitive` `rdu` `area:quant` `area:attention` |
| Depends on | FUSIONS:PARITY-KV-CACHE-QUANT |
| Touches | the attention family's cache format (new) |
| Status | open |

**Goal.** The spatial shim refuses KV-cache quantization. Store K and V in fp8 or int8 with per-head or per-block
scales, written by the append and read by the flash fold, within the numerics contract.

**Read first.**
- The fusion book's PARITY-KV-CACHE-QUANT.

**RED tests.**
- [ ] `spatial_kv_quant_lock::an_8_bit_cache_decodes_within_the_contract`: fails today: refused.

**Edge cases.**
- [ ] An outlier key.

**Implementation.** Quantize in the append's epilogue; dequantize in the fold's load.

**Success.**
- [ ] Within the contract; cache bytes halved (receipt: the lock).

**Mutants to kill.**
- [ ] A scale written for the wrong head.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_kv_quant_lock
```

**Done when.** Long contexts cost half the cache bytes.

#### NUM-7 · The RDU's numerics contract

| Field | Value |
|---|---|
| Phase | numerics |
| Labels | `infrastructure` `rdu` |
| Depends on | NUM-1, FUSIONS:OPT-6 |
| Touches | `crates/logicaffeine_compile/src/spatial/shim.rs` through R7 |
| Status | open |

**Goal.** The fusion book's numerics contracts give each chip a declared bound. Declare the RDU's per knob point
(bf16, bf16 with fp32 islands, fp8, 4-bit) and hold every deployment to it: top-1 agreement, KL and perplexity against
the f32 reference.

**Read first.**
- `crates/logicaffeine_tests/tests/numerics_contract_lock.rs`

**RED tests.**
- [ ] `numerics_contract_lock::the_rdu_declares_a_contract_per_knob_point`: fails today.

**Edge cases.**
- [ ] A model whose top-1 margin is tiny at some step: judged by KL there.

**Implementation.** `SpatialShim::numerics` returns the contract; deployment locks read it.

**Success.**
- [ ] Every Tier A deployment within its contract (receipt: the lock).

**Mutants to kill.**
- [ ] A contract bound loosened by ten.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test numerics_contract_lock
```

**Done when.** Every RDU result has a declared and held accuracy.

#### NUM-8 · Batch invariance and determinism

| Field | Value |
|---|---|
| Phase | numerics |
| Labels | `technique` `rdu` |
| Depends on | NUM-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/value_range.rs` |
| Status | open |

**Goal.** A sequence's tokens are bit-identical whether it decodes alone or in a batch, and two runs are bit-identical.
Fix every reduction order independent of batch and chip count, and prove it per lowering.

**Read first.**
- The fusion book's TECH-BATCH-INVARIANCE.

**RED tests.**
- [ ] `spatial_batch_invariance_lock::a_sequence_decodes_alike_alone_and_in_a_batch`: fails today: unmeasured.

**Edge cases.**
- [ ] A batch-dependent tiling choice: forbidden from changing a reduction order.

**Implementation.** Reduction order recorded per section; the tile pass keeps it fixed across batch sizes.

**Success.**
- [ ] Bit-identical at batch 1 and 64 (receipt: the lock).

**Mutants to kill.**
- [ ] A batch-size-dependent split of a reduction.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_batch_invariance_lock
```

**Done when.** Results do not depend on who else is in the batch.

### 11.15 Memory tiers

#### MEM-1 · DDR as a placed tier

| Field | Value |
|---|---|
| Phase | memory |
| Labels | `technique` `rdu` `area:memory` |
| Depends on | CHIP-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/residency.rs`, `crates/logicaffeine_tests/tests/spatial_residency_lock.rs` |
| Status | open |

**Goal.** The residency plan spills to DDR by density with no DDR roof. Make DDR a priced tier: what lives there,
when it moves to HBM or SRAM, and what its latency and bandwidth cost the step, as SN40L's three tiers are meant to be
used [SN40L24].

**Read first.**
- `crates/logicaffeine_compile/src/spatial/residency.rs`: `plan_off_chip`.

**RED tests.**
- [ ] `spatial_residency_lock::a_ddr_tensor_is_priced_by_its_roof`: fails today.

**Edge cases.**
- [ ] A tensor larger than one socket's DDR: sharded.

**Implementation.** The DDR roof from CHIP-3; residency classes by access frequency, not density alone.

**Success.**
- [ ] Every DDR placement priced (receipt: the lock).

**Mutants to kill.**
- [ ] A DDR read priced at HBM speed.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_residency_lock
```

**Done when.** The third tier is a tool.

#### MEM-2 · Model switching from DDR

| Field | Value |
|---|---|
| Phase | memory |
| Labels | `technique` `rdu` `area:memory` |
| Depends on | MEM-1 |
| Touches | model switching (new) |
| Status | open |

**Goal.** Composition of Experts [SN40L24] holds many models in DDR and swaps the active one into HBM. Deploy several
models' bundles sharing a socket set, with a switch whose cost the time model states.

**Read first.**
- The SN40L paper's Composition of Experts sections.

**RED tests.**
- [ ] `spatial_model_switch_lock::two_models_switch_between_requests`: fails today.

**Edge cases.**
- [ ] Two models sharing an embedding.

**Implementation.** A deployment of deployments; the switch as a DDR-to-HBM transfer.

**Success.**
- [ ] Two models served alternately, each equal to its own bundle (receipt: the lock).

**Mutants to kill.**
- [ ] A switch that leaves the old model's weights bound.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_model_switch_lock
```

**Done when.** Many models share the hardware.

#### MEM-3 · The KV cache across HBM and DDR

| Field | Value |
|---|---|
| Phase | memory |
| Labels | `technique` `rdu` `area:memory` |
| Depends on | MEM-1 |
| Touches | the KV tiering (new) |
| Status | open |

**Goal.** Contexts to 262,144 and 1M positions (qwen4_exp, Qwen3.8) at batch exceed HBM. Page the cache across HBM and
DDR by recency and by the indexer's selection, and price it.

**Read first.**
- The fusion book's SERVE-KV-OFFLOAD.

**RED tests.**
- [ ] `spatial_kv_tier_lock::a_262144_context_decodes_with_its_cache_split`: fails today.

**Edge cases.**
- [ ] A selected block that lives in DDR.

**Implementation.** A paged cache with a tier per page; the fold's loads by page.

**Success.**
- [ ] Equal to an all-HBM cache (receipt: the lock).

**Mutants to kill.**
- [ ] A page read from the wrong tier.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_kv_tier_lock
```

**Done when.** Context length is not capped by HBM.

#### MEM-4 · The dense spine resident in SRAM across chips

| Field | Value |
|---|---|
| Phase | memory |
| Labels | `technique` `rdu` `area:memory` |
| Depends on | MEM-1, MULTI-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/residency.rs` |
| Status | open |

**Goal.** Sixteen SN50s hold 6.9 GB of SRAM. A model's dense spine (attention, norms, routers, hyper-connections, the
recurrent state) fits there for many models; only routed experts then stream from HBM. Plan residency across the
domain's SRAM, not one chip's.

**Read first.**
- Section 7.3's budgets.

**RED tests.**
- [ ] `spatial_residency_lock::qwen4exps_spine_is_resident_across_sixteen_chips`: fails today.

**Edge cases.**
- [ ] A spine that does not fit: the densest part resident, priced.

**Implementation.** Residency over the TP domain; per-chip slices placed by the plan.

**Success.**
- [ ] HBM bytes a step fall to the experts' (receipt: the time model).

**Mutants to kill.**
- [ ] A resident slice also streamed.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_residency_lock
```

**Done when.** SRAM across the rack is one memory.

#### MEM-5 · A prefix cache

| Field | Value |
|---|---|
| Phase | memory |
| Labels | `serving` `rdu` `area:memory` |
| Depends on | MEM-3 |
| Touches | the KV tiering (new) |
| Status | open |

**Goal.** Requests sharing a prefix reuse its cache, and prefill only the suffix.

**Read first.**
- The fusion book's SERVE-PREFIX-CACHE.

**RED tests.**
- [ ] `spatial_kv_tier_lock::a_shared_prefix_prefills_only_its_suffix`: fails today.

**Edge cases.**
- [ ] A prefix shared by a hybrid model: the recurrent state at the prefix's end kept too.

**Implementation.** Content-addressed cache pages; recurrent state snapshots at page ends.

**Success.**
- [ ] Equal to a full prefill (receipt: the lock).

**Mutants to kill.**
- [ ] A recurrent state reused from a different prefix.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_kv_tier_lock
```

**Done when.** Repeated prompts cost their new part.

#### MEM-6 · Hot experts in HBM, cold experts in DDR

| Field | Value |
|---|---|
| Phase | memory |
| Labels | `technique` `rdu` `area:moe` `area:memory` |
| Depends on | MEM-1, MULTI-2 |
| Touches | `crates/logicaffeine_compile/src/spatial/residency.rs` |
| Status | open |

**Goal.** Expert tables larger than HBM keep the frequently routed experts in HBM and the rest in DDR, moved by
measured routing statistics; at batch 128 qwen4_exp touches about 471 of 512 experts a step.

**Read first.**
- Section 7.3's experts-touched row.

**RED tests.**
- [ ] `spatial_residency_lock::cold_experts_spill_to_ddr_and_equal_the_vm`: fails today.

**Edge cases.**
- [ ] A routing shift that makes a cold expert hot.

**Implementation.** Residency by measured frequency; a migration between steps.

**Success.**
- [ ] Equal to all-HBM; HBM bytes priced (receipt: the lock).

**Mutants to kill.**
- [ ] A cold expert read from HBM.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_residency_lock
```

**Done when.** Expert count is not capped by HBM.

#### MEM-7 · Storage reused by lifetime, spilled by bandwidth

| Field | Value |
|---|---|
| Phase | memory |
| Labels | `technique` `rdu` `area:memory` |
| Depends on | MEM-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/residency.rs` |
| Status | open |

**Goal.** The SN40L compiler manages device memory as a heap it collects at compile time: every symbol's lifetime is
known, its space is reused when it dies, a symbol marked read-only is never copied back, and what does not fit in HBM
spills to DDR ranked by the bandwidth it needs [SN40L24]. Loom shares physical buffers among virtual ones by liveness,
coloring the interference graph, and merges the buffers of exclusive handoffs [LOOM26]; Revet fuses and hoists on-chip
allocations [REVET]. Ours allocates the SRAM over each intermediate's live range by linear scan, spilling the one whose
last use is furthest away, and fills HBM with the carried state that moves the most bytes per byte held
(`plan_off_chip`). Not yet: a dead tensor's HBM or DDR space reused, a read-only marking, the weights and caches ranked
with the state, and coloring where lifetimes interleave.

**Read first.**
- Section 5.4's ledger rows for lifetime analysis, spilling and buffer sharing.
- `crates/logicaffeine_compile/src/spatial/residency.rs`.

**RED tests.**
- [ ] `spatial_residency_lock::a_dead_tensors_hbm_is_reused_by_a_later_one`: fails today: off-chip space is never reused.
- [ ] `spatial_residency_lock::a_read_only_tensor_is_never_written_back`: fails today.
- [ ] `spatial_residency_lock::coloring_needs_no_more_buffers_than_linear_scan`: fails today.

**Edge cases.**
- [ ] A tensor whose lifetime crosses the step boundary (a cache): never reused within the deployment.
- [ ] Two tensors whose lifetimes touch at one phase: the writer of the second waits for the last reader of the first.
- [ ] A spill ranking tie: broken by name, so the plan is deterministic.

**Implementation.** Lifetimes over phases for every tensor and tier; one allocator per tier over them; a read-only mark
from the program's writes; the off-chip ranking widened to weights and caches by moved bytes per byte held.

**Success.**
- [ ] A decode layer's peak HBM falls to its largest live set, equal to the VM (receipt: `spatial_residency_lock`).

**Mutants to kill.**
- [ ] A space reused before its last reader.
- [ ] A read-only tensor written back.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_residency_lock
```

**Done when.** Device memory holds only what is alive.
### 11.16 Serving on the RDU

#### SERVE-RDU-1 · Speculative decoding with recurrent rollback

| Field | Value |
|---|---|
| Phase | serving |
| Labels | `serving` `rdu` |
| Depends on | SCHED-6, AUTHOR-3, FUSIONS:SERVE-SPEC-DECODE |
| Touches | `crates/logicaffeine_compile/src/spatial/serving.rs`; speculative decoding (new) |
| Status | open |

**Goal.** SambaNova's decode claims rest on speculative decoding [DIAYN26]. Draft k tokens (an MTP head, a draft model,
or an EAGLE-style head), verify them as one k + 1 row block, accept the agreeing prefix, and restore every carried
state on a rejection: attention caches by length, gated-delta and other recurrent states by checkpoints taken at each
draft position.

**Read first.**
- The Dataflow paper's section 2 (acceptance, the step's latency).
- `crates/logicaffeine_compile/src/spatial/serving.rs`: the FPGA card's speculative model.

**RED tests.**
- [ ] `spatial_speculative_lock::speculation_emits_the_greedy_tokens`: fails today.
- [ ] `spatial_speculative_lock::a_rejection_restores_every_recurrent_state`: fails today.
- [ ] `spatial_speculative_lock::acceptance_is_recorded_per_prompt_set`: fails today.

**Edge cases.**
- [ ] k = 1 and k = 9. Rejection at the first draft. Acceptance of every draft.
- [ ] Sampling instead of greedy: the acceptance rule that keeps the target's distribution.

**Implementation.** Draft and target as concatenated schedules (SCHED-6); state checkpoints in SRAM where they fit;
verification through the stacked-row path.

**Success.**
- [ ] Greedy-equal output; acceptance recorded on AlpacaEval-, HumanEval- and GSM8K-style prompts (receipt: the lock).

**Mutants to kill.**
- [ ] A rejected draft's KV rows left in the cache.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_speculative_lock
```

**Done when.** Speculation multiplies the decode rate on the RDU.

#### SERVE-RDU-2 · Sampling on chip

| Field | Value |
|---|---|
| Phase | serving |
| Labels | `serving` `rdu` `area:sampling` |
| Depends on | KERNEL-1, FUSIONS:PARITY-SAMPLING |
| Touches | on-chip sampling (new) |
| Status | open |

**Goal.** The RDU path samples by arg-max only. Sample on chip over the full vocabulary with temperature, top-k, top-p,
min-p, repetition and presence penalties and a seeded counter-based generator, so a seed reproduces a run.

**Read first.**
- The fusion book's PARITY-SAMPLING.

**RED tests.**
- [ ] `spatial_sampling_lock::seeded_sampling_reproduces`: fails today.
- [ ] `spatial_sampling_lock::top_p_and_top_k_equal_the_reference_distribution`: fails today.

**Edge cases.**
- [ ] Temperature 0 (the arg-max). top-p of 1. A vocabulary row of equal logits.

**Implementation.** Log-sum-exp (KERNEL-7), selection (KERNEL-1), a counter-based generator in INT32 (KERNEL-6).

**Success.**
- [ ] Distribution tests pass at fixed seeds (receipt: the lock).

**Mutants to kill.**
- [ ] The generator's counter not advanced.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_sampling_lock
```

**Done when.** Serving needs no host for a token.

#### SERVE-RDU-3 · Prefill on a GPU, decode on the RDU

| Field | Value |
|---|---|
| Phase | serving |
| Labels | `serving` `rdu` |
| Depends on | MULTI-1 |
| Touches | `crates/logicaffeine_compile/src/spatial/handoff.rs` (new); `crates/logicaffeine_compile/src/compile.rs` (the manifest's `handoff`); `crates/logicaffeine_tests/tests/spatial_disaggregation_lock.rs` and `crates/logicaffeine_tests/tests/common/samba_handoff.rs` (new) |
| Status | open |

**Goal.** SambaNova's MiniMax row prefills on H200s and decodes on SN50s [HC26]. We own both sides, so any split of a
model's work between chips is one contract: a prompt prefilled on a GPU decodes on the RDU, a prompt prefilled on the RDU decodes on a GPU, and a model's
layers split over two chips hand their hidden state on, each side's state agreed at compile time.

**Read first.**
- The fusion book's section 3.3 (the CUDA lane's prefill).

**RED tests.**
- [x] `spatial_disaggregation_lock::the_decode_bundle_states_the_state_it_carries`: the manifest names every carried
  tensor with its layer and shape, the contract `spatial::handoff::contract` reads off the decode step's authored layer
  programs (receipt: `logs/samba/handoff-3.log`).
- [x] `spatial_disaggregation_lock::a_hand_off_that_breaks_the_contract_is_refused_by_name`: a missing, unknown or
  wrong-sized tensor, and a position whose next row the cache does not have (receipt: `logs/samba/handoff-3.log`).
- [ ] `spatial_disaggregation_reds_lock::a_prompt_prefilled_on_a_gpu_decodes_on_the_rdu` (handed RED, frozen, NUM-1):
  Qwen3, Qwen3.5 (its gated-delta
  memory and conv rings handed over) and Gemma 3 hold the reference's bound; gpt-oss lands 0.181 from the reference
  against 0.137 (`logs/samba/handoff-3.log`).
- [ ] `spatial_disaggregation_reds_lock::a_prompt_prefilled_on_the_rdu_decodes_on_a_gpu`,
  `…a_hand_off_decodes_as_a_prompt_kept_on_one_chip`, `…a_model_split_over_two_chips_hands_its_hidden_state_on`
  (handed REDs, frozen, NUM-1): the SN50's own bf16 prefill and decode drift past the bound (Qwen3 0.143 against 0.100, gpt-oss 0.575 against 0.137).
  With fp32 elementwise on the SN50 (its fp32 twin) every one of these holds for all four architectures, so the hand-off
  is exact and what is left is NUM-1.
- [ ] `samba_exam_prefill_lock::a_gpu_prefilled_cache_decodes_on_the_rdu` (E4.3): the CUDA lane's prefill on the card
  handing its cache to the RDU bundle.

**Edge cases.**
- [x] A hybrid model: the gated-delta memory and the three conv rings handed over (Qwen3.5).
- [x] Different precisions on the two sides: the hand-off is the logical values, each side converting at its edge (the
  GPU target's fp32 elementwise cache into the SN50's bf16 bundle).
- [x] A model split over two chips mid-stack: the hidden state at the cut, each chip its own layers' state.
- [ ] A tensor-parallel decode: each chip's shard of the contract.

**Implementation.** The author declares each carried tensor once, by name and shape, and the prefill program declares
the same tensors the decode step reads, so the contract is read off the authored decode layers (`carried_state` of each
layer's recognized program, shapes from its declarations) and stated in every decode bundle's manifest (`handoff`:
`ctx`, `hidden`, `carried`). `Contract::take` lifts a run's carried tensors into a `Handoff`; `Contract::check` refuses
one that does not match, by name. Making the GPU side lower the hybrid's prefill took two fixes: a conv ring is a matrix
whenever the single step's flat form does not fit a site (one decision for the step and a block of rows), and an
elementwise op whose right operand is one row combined with every row (a bias over a prefill block) lowers in the dense
and the flat layouts.

**Success.**
- [ ] Equal tokens to an RDU-only run, within the reference's bound (receipt: the lock).

**Mutants to kill.**
- [ ] A head order swapped in the hand-off.
- [x] A carried tensor left out of the contract (receipt: `logs/samba/mutants5b/results.json`, handoff-contract-drops-a-tensor killed).

**Verify.**

```
cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test spatial_disaggregation_lock
```

**Done when.** Prefill and decode each run where they run best.

#### SERVE-RDU-4 · Ragged and continuous batches

| Field | Value |
|---|---|
| Phase | serving |
| Labels | `serving` `rdu` |
| Depends on | SCHED-4, FUSIONS:SERVE-CONTINUOUS-BATCHING |
| Touches | `crates/logicaffeine_compile/src/spatial/serving.rs` |
| Status | open |

**Goal.** Sequences of different lengths in one batch, joining and leaving every step, without recompiling.

**Read first.**
- The fusion book's SERVE-CONTINUOUS-BATCHING.

**RED tests.**
- [ ] `spatial_ragged_batch_lock::ragged_lengths_equal_each_sequence_alone`: fails today.

**Edge cases.**
- [ ] A batch slot that empties mid-step.

**Implementation.** Per-sequence positions and trip counts as runtime inputs; slots by index.

**Success.**
- [ ] Equal to each sequence alone (receipt: the lock).

**Mutants to kill.**
- [ ] One sequence's position read for another.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_ragged_batch_lock
```

**Done when.** Serving traffic runs as it arrives.

#### SERVE-RDU-5 · An OpenAI-compatible runner over a bundle

| Field | Value |
|---|---|
| Phase | serving |
| Labels | `serving` `rdu` |
| Depends on | ACCESS-4, SERVE-RDU-2 |
| Touches | the runner (new) |
| Status | open |

**Goal.** A runner that serves a bundle through the completions and chat API (streaming, stop strings,
log-probabilities, seeds), on our evaluator today and on the vendor's runtime when it exists.

**Read first.**
- `scripts/accel/sambacloud-smoke.sh`: the API shape SambaCloud serves.

**RED tests.**
- [ ] `spatial_runner_lock::the_endpoint_streams_stops_and_reports_logprobs`: fails today.

**Edge cases.**
- [ ] A stop string split across tokens.

**Implementation.** An HTTP front over the drop's runner.

**Success.**
- [ ] The smoke script's requests answered (receipt: the lock).

**Mutants to kill.**
- [ ] A stop string matched only within one token.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_runner_lock
```

**Done when.** A bundle is a service.

#### SERVE-RDU-6 · Grammar-constrained decoding on chip

| Field | Value |
|---|---|
| Phase | serving |
| Labels | `serving` `rdu` `area:sampling` |
| Depends on | SERVE-RDU-2, FUSIONS:SERVE-STRUCTURED-OUTPUT |
| Touches | on-chip sampling (new) |
| Status | open |

**Goal.** JSON-schema and grammar constraints as a token mask applied before sampling, the mask's automaton stepped on
chip.

**Read first.**
- The fusion book's SERVE-STRUCTURED-OUTPUT.

**RED tests.**
- [ ] `spatial_sampling_lock::json_constrained_output_parses`: fails today.

**Edge cases.**
- [ ] A schema whose only legal next token is end of sequence.

**Implementation.** The automaton's transition table in SRAM; the mask as a select before the sampler.

**Success.**
- [ ] Every constrained output parses (receipt: the lock).

**Mutants to kill.**
- [ ] A mask applied after sampling.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_sampling_lock
```

**Done when.** Structured output needs no host.

### 11.17 Training

The Plasticine kit reports a training step for every model [PTUT], and SN40L trains as well as serves.

#### TRAIN-1 · A training step on the spatial pipeline

| Field | Value |
|---|---|
| Phase | training |
| Labels | `technique` `rdu` |
| Depends on | SCHED-1, FUSIONS:BEYOND-TRAINING |
| Touches | the training lowering (new) |
| Status | open |

**Goal.** Forward, loss and backward of a model through the IR's reverse mode (`crates/logicaffeine_compile/src/tensor_graph/vjp.rs`), lowered
spatially, gradients equal to PyTorch's.

**Read first.**
- `crates/logicaffeine_compile/src/tensor_graph/vjp.rs`
- The fusion book's BEYOND-TRAINING.

**RED tests.**
- [ ] `spatial_training_lock::squeezenet_gradients_equal_pytorch`: fails today.

**Edge cases.**
- [ ] A loss with an ignore index. Batch norm in training mode.

**Implementation.** The reverse program from the IR; its sections lowered as any program's.

**Success.**
- [ ] Gradients equal to PyTorch's within the contract (receipt: the lock).

**Mutants to kill.**
- [ ] A transposed gradient product.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_training_lock
```

**Done when.** A training step runs on the RDU.

#### TRAIN-2 · The optimizer step in place

| Field | Value |
|---|---|
| Phase | training |
| Labels | `technique` `rdu` |
| Depends on | TRAIN-1 |
| Touches | the training lowering (new) |
| Status | open |

**Goal.** SGD and AdamW updates applied to weights in place, their moments resident where they fit.

**Read first.**
- TRAIN-1.

**RED tests.**
- [ ] `spatial_training_lock::an_adamw_step_equals_pytorch`: fails today.

**Edge cases.**
- [ ] Weight decay on norms excluded.

**Implementation.** Elementwise update sections over weights and moments.

**Success.**
- [ ] Equal to PyTorch over 10 steps (receipt: the lock).

**Mutants to kill.**
- [ ] Bias correction dropped.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_training_lock
```

**Done when.** Training runs end to end.

#### TRAIN-3 · Training mapped onto as many chips as it needs

| Field | Value |
|---|---|
| Phase | training |
| Labels | `technique` `rdu` |
| Depends on | TRAIN-2, MULTI-5 |
| Touches | the training lowering (new) |
| Status | open |

**Goal.** The kit refuses SqueezeNet's training on one chip as "configuration too small" [PTUT]. Plan training across
chips (data, tensor and pipeline parallel) by the time model and report it in the kit's form.

**Read first.**
- PLAST-13.

**RED tests.**
- [ ] `spatial_training_lock::squeezenet_training_maps_on_enough_chips`: fails today.

**Edge cases.**
- [ ] A gradient all-reduce overlapped with the backward pass.

**Implementation.** MULTI-5's search over training programs.

**Success.**
- [ ] A plan and its report for SqueezeNet and ResNet-50 (receipt: the lock).

**Mutants to kill.**
- [ ] A gradient reduced on one chip only.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test spatial_training_lock
```

**Done when.** Any training step the kit scores, we map.

### 11.18 Mining

Optimizations nobody has published, found by search and kept only with a proof and a measurement. The engine inside
the climb (section 9).

#### MINE-1 · Rewrite candidates harvested from every graph

| Field | Value |
|---|---|
| Phase | mining |
| Labels | `beyond` |
| Depends on | INTAKE-5 |
| Touches | the harvester (new) |
| Status | open |

**Goal.** Run equality saturation over the optimized graph of every Tier A to D model and every fuzzed configuration,
and collect candidate rewrites: subgraphs that recur across families, algebraic identities between them, layout changes
that remove a relayout, folds of constants into weights.

**Read first.**
- The fusion book's OPT-4 (equality saturation over the IR).

**RED tests.**
- [ ] `samba_mining_lock::the_harvest_covers_every_tier`: fails today.

**Edge cases.**
- [ ] A candidate that holds only for some shapes: kept with its shape condition.

**Implementation.** The IR's e-graph; candidate extraction by recurrence and cost delta.

**Success.**
- [ ] A ranked candidate list per run (receipt: the mining ledger).

**Mutants to kill.**
- [ ] A harvester that skips one tier.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_mining_lock
```

**Done when.** Every model we can read feeds the search.

#### MINE-2 · Every candidate proved or refuted

| Field | Value |
|---|---|
| Phase | mining |
| Labels | `beyond` |
| Depends on | MINE-1, FUSIONS:OPT-5 |
| Touches | the harvester (new) |
| Status | open |

**Goal.** Each candidate is proved equal by the fusion book's SMT path (`crates/logicaffeine_tv/src/tensor.rs`) or, for algebraic
identities over real arithmetic, by the kernel and its Lean interface (`crates/logicaffeine_kernel/src/interface/lean/`), or refuted with a
counterexample; numerically different rewrites are proved within a stated bound instead.

**Read first.**
- `crates/logicaffeine_tests/tests/tensor_rule_proof_lock.rs`

**RED tests.**
- [ ] `samba_mining_lock::every_kept_rewrite_has_a_proof_the_checker_accepts`: fails today.

**Edge cases.**
- [ ] A rewrite exact over the reals and not in floating point: proved with a bound.

**Implementation.** Route each candidate to SMT, the kernel or interval proofs by its form.

**Success.**
- [ ] Every kept rewrite carries a rechecked proof (receipt: the lock).

**Mutants to kill.**
- [ ] A rewrite kept whose proof was not rechecked.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_mining_lock --test tensor_rule_proof_lock
```

**Done when.** Nothing mined lands on trust.

#### MINE-3 · Precision chosen by proven bounds

| Field | Value |
|---|---|
| Phase | mining |
| Labels | `beyond` `area:quant` |
| Depends on | NUM-7, FUSIONS:BEYOND-PROVEN-NUMERICS |
| Touches | the numerics search (new) |
| Status | open |

**Goal.** For each section, the cheapest precision whose proven error bound (interval and sum-of-squares methods in
`crates/logicaffeine_proof/src/`) keeps the whole deployment inside its contract.

**Read first.**
- The fusion book's BEYOND-PROVEN-NUMERICS.

**RED tests.**
- [ ] `samba_mining_lock::a_mined_precision_plan_holds_its_contract`: fails today.

**Edge cases.**
- [ ] A section whose bound depends on the input range: the range proved.

**Implementation.** A search over per-section precisions with bounds composed along the graph.

**Success.**
- [ ] Bytes a token fall with the contract held (receipt: the lock).

**Mutants to kill.**
- [ ] A bound composed without one section.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_mining_lock
```

**Done when.** Precision is chosen by proof.

#### MINE-4 · Exact schedules by constraint solving

| Field | Value |
|---|---|
| Phase | mining |
| Labels | `beyond` |
| Depends on | TIME-1, FUSIONS:BEYOND-EXACT-MODULO-SCHEDULER |
| Touches | the schedule solver (new) |
| Status | open |

**Goal.** Loom solves a tile program's mapping and schedule as constraints from the hardware description and matches
or beats vendor kernels with no tuning [LOOM26]. Solve our sections' tiling, parallelization factors, FIFO depths and
stage overlap the same way, against the time model, with the optimum's gap reported.

**Read first.**
- The Loom paper.
- `crates/logicaffeine_proof/src/cycle_ratio.rs` and `crates/logicaffeine_proof/src/min_cost_flow.rs`.

**RED tests.**
- [ ] `samba_mining_lock::the_solver_matches_or_beats_the_tile_pass`: fails today.

**Edge cases.**
- [ ] A section whose solve exceeds its time budget: the best found and its gap.

**Implementation.** Constraints from the unit model; the time model as the objective.

**Success.**
- [ ] Every Tier A section's schedule solved with its gap (receipt: the lock).

**Mutants to kill.**
- [ ] A constraint dropped that makes an illegal schedule optimal.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_mining_lock
```

**Done when.** Schedules are solved, not guessed.

#### MINE-5 · Residency by integer program

| Field | Value |
|---|---|
| Phase | mining |
| Labels | `beyond` `area:memory` |
| Depends on | MEM-1, FUSIONS:BEYOND-CERTIFIED-ALLOCATION |
| Touches | the residency search (new) |
| Status | open |

**Goal.** Place every tensor in SRAM, HBM or DDR across the domain by an integer program over the time model, with a
certificate of the solution's bound.

**Read first.**
- The fusion book's BEYOND-CERTIFIED-ALLOCATION.

**RED tests.**
- [ ] `samba_mining_lock::the_residency_program_beats_the_greedy_plan`: fails today.

**Edge cases.**
- [ ] A domain whose SRAM holds nothing but the head.

**Implementation.** A pseudo-Boolean or integer formulation in `crates/logicaffeine_proof/src/`.

**Success.**
- [ ] Modeled step time at or under the greedy plan's on every Tier A model (receipt: the lock).

**Mutants to kill.**
- [ ] A capacity constraint dropped.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_mining_lock
```

**Done when.** Every byte is placed by an optimum.

#### MINE-6 · Parallel plans with a certificate of optimality

| Field | Value |
|---|---|
| Phase | mining |
| Labels | `beyond` `area:comm` |
| Depends on | MULTI-5 |
| Touches | the plan search (new) |
| Status | open |

**Goal.** MULTI-5 searches parallel plans by enumeration. Bound the search so its choice is certified optimal under the
time model, or its gap stated.

**Read first.**
- MULTI-5.

**RED tests.**
- [ ] `samba_mining_lock::the_parallel_plan_carries_its_optimality_gap`: fails today.

**Edge cases.**
- [ ] Ties between plans: the deterministic one chosen.

**Implementation.** Branch and bound with the time model's lower bounds.

**Success.**
- [ ] Every Tier A plan's gap is zero or stated (receipt: the lock).

**Mutants to kill.**
- [ ] A lower bound that is not a bound.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_mining_lock
```

**Done when.** The plan is the best the model knows, and says so.

#### MINE-7 · The mined-rule ledger

| Field | Value |
|---|---|
| Phase | mining |
| Labels | `beyond` |
| Depends on | MINE-2 |
| Touches | the mining ledger (new) |
| Status | open |

**Goal.** Every mined rewrite in a ledger: its pattern, its proof, its measured gain per cell, its landing commit, or
its rejection and why.

**Read first.**
- Section 9.2.

**RED tests.**
- [ ] `samba_mining_lock::every_landed_rule_has_its_ledger_row`: fails today.

**Edge cases.**
- [ ] A rule whose gain is negative on one cell: kept as a toggle, not a default.

**Implementation.** A JSON ledger written by the miner and the landing.

**Success.**
- [ ] Every rule accounted for (receipt: the lock).

**Mutants to kill.**
- [ ] A landed rule without a row.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_mining_lock
```

**Done when.** The search remembers what it found.

#### MINE-8 · Qwen 4's algebraic folds

| Field | Value |
|---|---|
| Phase | mining |
| Labels | `beyond` `model:qwen4exp` |
| Depends on | MINE-2, QWEN4-1 |
| Touches | the harvester (new) |
| Status | open |

**Goal.** Targets the architecture invites: fold a hyper-connection's norm weight into its down product; fold the
n-gram key projection into per-table-row precomputation where storage allows; keep the indexer's pooled keys as the
only key read above the budget; absorb latent-attention projections. Each proved, measured and landed or rejected.

**Read first.**
- Section 7.

**RED tests.**
- [ ] `samba_mining_lock::each_qwen4_fold_is_proved_and_measured`: fails today.

**Edge cases.**
- [ ] A fold that trades HBM bytes for DDR bytes: priced on both.

**Implementation.** Targeted harvest over qwen4_exp's graph.

**Success.**
- [ ] Every target has a ledger row (receipt: the mining ledger).

**Mutants to kill.**
- [ ] A fold landed whose proof covers a different shape.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_mining_lock
```

**Done when.** The next architecture's own shortcuts are found before the release.

### 11.19 The climb

#### CLIMB-1 · One objective

| Field | Value |
|---|---|
| Phase | climb |
| Labels | `infrastructure` |
| Depends on | SCORE-1 |
| Touches | the climb history (new) |
| Status | open |

**Goal.** A per-cell score (MBU or share of roofline, the quality gates as hard constraints) and an aggregate (the
geometric mean over the exam's cells), computed by the exam runner into the climb history with the commit.

**Read first.**
- Section 9.

**RED tests.**
- [ ] `samba_climb_lock::the_objective_is_recorded_per_commit`: fails today.

**Edge cases.**
- [ ] A cell that fails a quality gate: scored zero, never skipped.

**Implementation.** The runner writes a history row per run.

**Success.**
- [ ] The history has its first rows (receipt: the ledger).

**Mutants to kill.**
- [ ] A failing cell skipped in the mean.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_climb_lock
```

**Done when.** Progress is one number per commit.

#### CLIMB-2 · The gap miner

| Field | Value |
|---|---|
| Phase | climb |
| Labels | `infrastructure` |
| Depends on | CLIMB-1, TIME-6 |
| Touches | the gap miner (new) |
| Status | open |

**Goal.** Rank the largest attributed losses across cells, read the experiment ledger, and emit hypotheses as
`BLOCKED SAMBA` or new-ticket lines naming the cell, the cause and the ticket that would close it.

**Read first.**
- TIME-6.

**RED tests.**
- [ ] `samba_climb_lock::the_miner_skips_rejected_hypotheses`: fails today.

**Edge cases.**
- [ ] A cause with no ticket: a new ticket drafted for the keeper.

**Implementation.** A ranking over the attribution with the ledger as a filter.

**Success.**
- [ ] Hypotheses emitted per run (receipt: the lock).

**Mutants to kill.**
- [ ] A rejected hypothesis re-emitted.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_climb_lock
```

**Done when.** The next thing to try is computed.

#### CLIMB-3 · The experiment ledger

| Field | Value |
|---|---|
| Phase | climb |
| Labels | `infrastructure` |
| Depends on | none |
| Touches | the experiment ledger (new) |
| Status | done |

**Goal.** Every experiment's hypothesis, change, measured delta, verdict and log, written from `EXPERIMENT SAMBA` lines;
the tried-and-rejected notes from earlier campaigns migrated in.

**Read first.**
- Section 1.4's line formats.

**RED tests.**
- [x] `samba_climb_lock::every_experiment_line_becomes_a_row`: fails today (receipt: RED `logs/samba/red-ledgers-int64.log`, no ledger; green `logs/samba/green-2.log`).

**Edge cases.**
- [x] An experiment whose delta is within noise: verdict "no effect", kept (receipt: `samba_climb_lock::a_delta_within_noise_is_no_effect_and_kept`).

**Implementation.** A JSON ledger and a parser for the lines.

**Success.**
- [x] The ledger exists with the migrated rows (receipt: `benchmarks/silicon/samba_experiments.json`, 21 rows from the GPU campaigns' records, `samba_climb_lock::every_migrated_row_points_at_its_record` in `logs/samba/green-2.log`).

**Mutants to kill.**
- [x] A row without its log (receipt: `logs/samba/mutants2/results.json`, the rule and the ledger row both killed).

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_climb_lock
```

**Done when.** No dead end is walked twice.

#### CLIMB-4 · The compiler searches its own choices

| Field | Value |
|---|---|
| Phase | climb |
| Labels | `beyond` |
| Depends on | TIME-2 |
| Touches | the plan search (new) |
| Status | open |

**Goal.** Tiles, section cuts, parallelization factors, residency, parallel plans and fusion choices searched against
the calibrated time model at compile time, under certificates, with a default changed only when the new choice
dominates on every measured axis.

**Read first.**
- `crates/logicaffeine_tests/tests/dominance_toggle_lock.rs`

**RED tests.**
- [ ] `samba_climb_lock::the_searched_plan_beats_the_default_in_the_model`: fails today.

**Edge cases.**
- [ ] A search that times out: the default kept.

**Implementation.** A search loop over the knobs the passes expose.

**Success.**
- [ ] Modeled gains recorded per cell (receipt: the climb history).

**Mutants to kill.**
- [ ] A dominated choice made the default.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_climb_lock --test dominance_toggle_lock
```

**Done when.** The compiler tunes itself.

#### CLIMB-5 · Every measurement refits the time model

| Field | Value |
|---|---|
| Phase | climb |
| Labels | `infrastructure` `area:time` |
| Depends on | TIME-2 |
| Touches | the calibration ledger (new) |
| Status | open |

**Goal.** A new simulated or measured row refits the time model; a refit that moves any prediction past its bound turns
the calibration lock red until the model is fixed.

**Read first.**
- TIME-2.

**RED tests.**
- [ ] `samba_climb_lock::a_drifting_refit_turns_the_lock_red`: fails today.

**Edge cases.**
- [ ] A measurement from a noisy box: its load recorded, its weight lowered.

**Implementation.** The refit on every ledger append; the bound check.

**Success.**
- [ ] Refits recorded per run (receipt: the ledger).

**Mutants to kill.**
- [ ] A refit that ignores new rows.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_climb_lock
```

**Done when.** The model learns from every number.

#### CLIMB-6 · Rules land through proof and measurement

| Field | Value |
|---|---|
| Phase | climb |
| Labels | `beyond` |
| Depends on | MINE-7, CLIMB-3 |
| Touches | the mining ledger (new) |
| Status | open |

**Goal.** A mined rule lands as a fusion rule (through the fusion book's per-class file layout) only with its proof and
a measured gain on at least one cell and no loss on any.

**Read first.**
- The fusion book's section 1.5 (one file per fused class).

**RED tests.**
- [ ] `samba_climb_lock::a_rule_without_a_measurement_does_not_land`: fails today.

**Edge cases.**
- [ ] A rule that gains on one chip and loses on another: a per-shim answer.

**Implementation.** A landing check over the mining and climb ledgers.

**Success.**
- [ ] Every landed rule has proof and gain (receipt: the lock).

**Mutants to kill.**
- [ ] A rule landed on a measured loss.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_climb_lock
```

**Done when.** Mining feeds the compiler without risk.

#### CLIMB-7 · The exam grows itself

| Field | Value |
|---|---|
| Phase | climb |
| Labels | `exam` |
| Depends on | INTAKE-5, EXAM-1 |
| Touches | the exam generator (new) |
| Status | open |

**Goal.** Fuzzed configurations, shapes and routing distributions run as exam tests every night; a failure is shrunk,
frozen as a handed RED, and given a ticket.

**Read first.**
- INTAKE-5.

**RED tests.**
- [ ] `samba_climb_lock::a_fuzzed_failure_becomes_a_frozen_red`: fails today.

**Edge cases.**
- [ ] A flaky failure: re-run, never frozen without a reproduction.

**Implementation.** The fuzzer feeding the exam suites; the shrinker; the handed-red writer.

**Success.**
- [ ] The catalog's size grows by its fuzzed tests (receipt: the lock).

**Mutants to kill.**
- [ ] A failure frozen without its seed.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_climb_lock
```

**Done when.** The exam finds what we did not think of.

#### CLIMB-8 · The ratchet

| Field | Value |
|---|---|
| Phase | climb |
| Labels | `infrastructure` |
| Depends on | CLIMB-1 |
| Touches | the climb history (new) |
| Status | open |

**Goal.** No cell's best score regresses; a regression names the commit and the cell and holds the landing.

**Read first.**
- Section 9.3.

**RED tests.**
- [ ] `samba_climb_lock::a_regressed_cell_names_its_commit`: fails today.

**Edge cases.**
- [ ] A cell whose provenance changes (modeled to measured): a new series, not a regression.

**Implementation.** Best-so-far per cell in the history; the lock compares.

**Success.**
- [ ] The ratchet runs on every landing (receipt: the lock).

**Mutants to kill.**
- [ ] A ratchet that compares to the previous run instead of the best.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_climb_lock
```

**Done when.** Every gain is kept.

#### CLIMB-9 · The standing loop

| Field | Value |
|---|---|
| Phase | climb |
| Labels | `infrastructure` |
| Depends on | CLIMB-2, CLIMB-3, CLIMB-8 |
| Touches | the loop runbook (new) |
| Status | open |

**Goal.** The protocol a standing agent follows, written so `/loop` can run it: worst cell, attribution, hypothesis,
private-copy experiment, measurement, land or record, next. Each turn leaves a receipt.

**Read first.**
- Section 9.1.

**RED tests.**
- [ ] `samba_climb_lock::the_runbook_names_a_command_and_a_receipt_per_step`: fails today.

**Edge cases.**
- [ ] A turn that finds nothing to try: it records that and stops.

**Implementation.** A runbook held by a lock like ACCESS-6's.

**Success.**
- [ ] One turn run end to end and recorded (receipt: the experiment ledger).

**Mutants to kill.**
- [ ] A step without its receipt.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_climb_lock
```

**Done when.** The climb runs without a human choosing the next step.

#### CLIMB-10 · The climb, reported

| Field | Value |
|---|---|
| Phase | climb |
| Labels | `infrastructure` |
| Depends on | CLIMB-9 |
| Touches | the climb report (new) |
| Status | open |

**Goal.** A generated page from the ledgers: the objective's history, the top gaps, the experiments kept and rejected,
the rules mined, the cells won.

**Read first.**
- Section 9.2.

**RED tests.**
- [ ] `samba_climb_lock::the_report_is_generated_from_the_ledgers`: fails today.

**Edge cases.**
- [ ] An empty ledger: the report says so.

**Implementation.** A generator over the four ledgers.

**Success.**
- [ ] The report regenerates byte-identically from the ledgers (receipt: the lock).

**Mutants to kill.**
- [ ] A hand-edited report.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_climb_lock
```

**Done when.** Anyone can see the climb.
### 11.20 The exam and the score

Each EXAM ticket writes one family of section 10 as a suite of handed REDs, frozen by `frozen_reds_lock`, so the
exam exists before the work that passes it. The ticket each test names turns it green and deletes the gate line.

#### EXAM-1 · Cold bring-up of unseen configurations

| Field | Value |
|---|---|
| Phase | exam |
| Labels | `exam` |
| Depends on | none |
| Touches | the `samba_exam_bringup_lock` and `samba_exam_bringup_reds_lock` suites (new), their harness `crates/logicaffeine_tests/tests/common/samba_bringup.rs`; `crates/logicaffeine_tests/tests/fixtures/frozen_reds.tsv` rows |
| Status | open |

**Goal.** Write every test of section 10.1, each red today for the reason its ticket states and frozen so no later change
can weaken it, or green today for a reason found and written down. The family's tickets turn the reds green one by one.

**Read first.**
- Section 10.1 of this book.
- `crates/logicaffeine_tests/tests/frozen_reds_lock.rs`: how a handed RED is frozen.

**RED tests.**
- [x] `samba_exam_bringup_reds_lock::*`: every open row of section 10.1, each red today for its stated reason; a row over
  several shapes names every shape that fails and why (receipt: `logs/samba/exam1-red-6.log`).

**Edge cases.**
- [x] An unseen width, head count, vocabulary or layer count: E1.2 to E1.7, a shape each, every row written on a
  checkpoint that reads back as its spec (`model_tiny_checkpoint_lock`, receipt: `logs/samba/tiny-ckpt-3.log`).
- [x] An unknown architecture string and an unknown key: E1.9 (red) and E1.10 (green since INTAKE-1).
- [x] A test that passes on today's tree is not a RED: E1.4 and E1.5 pass because the emitter reads a Qwen3.5 checkpoint
  whole and these shapes lower within the unit model; they are in `samba_exam_bringup_lock` and ticked (receipt:
  `logs/samba/exam1-green-2.log`). Two red reasons were the harness's and were fixed before freezing: a weight the
  bundle reads as Q8_0 planes and a rotary table are filled as a host fills them (`model::bind::q8_planes`,
  `model::bind::rotary_values`, locked by `model_host_fill_lock`), and the program counts tokens from 1.

**Implementation.** One test function per row of the section, named as the row names it: in the reds suite, behind the
handed-reds gate, while the row is open; in the lock once it holds. The checkpoints are the tiny models of
`common/tiny_models.rs` with the converter metadata `tiny_models::checkpoint_meta` writes, so the emitter is handed the
file alone.

**Success.**
- [x] Every open row of section 10.1 is a frozen RED (receipt: `frozen_reds_lock`, `logs/samba/frozen-3.log`).
- [x] Every test fails today for its stated reason (receipt: `logs/samba/exam1-red-6.log`).

**Mutants to kill.**
- [ ] A test whose assertion is weakened: `frozen_reds_lock` reds.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test samba_exam_bringup_lock
cargo nextest run -p logicaffeine-tests --features handed-reds --cargo-profile test-opt --test samba_exam_bringup_reds_lock
cargo nextest run -p logicaffeine-tests --test frozen_reds_lock
```

**Done when.** Section 10.1 exists as tests before anyone sets them for us.

#### EXAM-2 · Correctness against the references

| Field | Value |
|---|---|
| Phase | exam |
| Labels | `exam` |
| Depends on | none |
| Touches | the `samba_exam_correctness_lock` suite (new); `crates/logicaffeine_tests/tests/frozen_reds_lock.rs` fixture rows |
| Status | open |

**Goal.** Write every test of section 10.2 as a handed RED in `samba_exam_correctness_lock`, each failing today for the reason its ticket
states, each frozen so no later change can weaken it. The family's tickets turn them green one by one.

**Read first.**
- Section 10.2 of this book.
- `crates/logicaffeine_tests/tests/frozen_reds_lock.rs`: how a handed RED is frozen.

**RED tests.**
- [ ] `samba_exam_correctness_reds_lock::*`: every test of section 10.2, each red today for its stated reason. Written and
  frozen: E2.4 (a sequence's logits beside 63 others differ in their last bits, NUM-8) and E2.7 (a chunk of 7 rows does
  not lower — its stacked rows' QK-norm, rotation and attention reach past the SN50's 8 accumulator tiles — before the
  last position is reached, MEM-3) (receipt: `logs/samba/reds-classify-2.log`). Owed: E2.1–E2.3 (the reference
  implementations' goldens, INTAKE-4), E2.6 (a prompt runner over a bundle, ACCESS-4), E2.8 (4,096 positions), E2.11 (the
  Wormhole target model runs no position-indexed op: it has no Int32 datapath, so the comparison the row names cannot
  run).

**Edge cases.**
- [x] A prompt at the context's last position (E2.7).
- [x] A NaN in an input (E2.10, the dense architectures; a router meeting a NaN is E6.6's).
- [x] A test that passes on today's tree: it is not a RED. E2.5, E2.9 and E2.10 hold, each non-vacuous (two bundles
  compared bit for bit; a channel at 1e4 the reference sees; every logit's class against a reference that carries the
  NaN), and stand in `samba_exam_correctness_lock` (`logs/samba/reds-classify-2.log`).

**Implementation.** One test function per row of the section, named as the row names it, opening with the
handed-reds gate; inputs generated by the fixtures the intake and fuzz tickets provide.

**Success.**
- [ ] Every row of section 10.2 is a frozen RED (receipt: `frozen_reds_lock`).
- [ ] Every test fails today for its stated reason (receipt: the RED log).

**Mutants to kill.**
- [ ] A test whose assertion is weakened: `frozen_reds_lock` reds.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --features handed-reds --test samba_exam_correctness_lock
cargo nextest run -p logicaffeine-tests --test frozen_reds_lock
```

**Done when.** Section 10.2 exists as tests before anyone sets them for us.

#### EXAM-3 · The decode matrix

| Field | Value |
|---|---|
| Phase | exam |
| Labels | `exam` |
| Depends on | none |
| Touches | the `samba_exam_decode_lock` suite (new); `crates/logicaffeine_tests/tests/frozen_reds_lock.rs` fixture rows |
| Status | open |

**Goal.** Write every test of section 10.3 as a handed RED in `samba_exam_decode_lock`, each failing today for the reason its ticket
states, each frozen so no later change can weaken it. The family's tickets turn them green one by one.

**Read first.**
- Section 10.3 of this book.
- `crates/logicaffeine_tests/tests/frozen_reds_lock.rs`: how a handed RED is frozen.

**RED tests.**
- [ ] `samba_exam_decode_lock::*`: every test of section 10.3, each red today for its stated reason (receipt: the RED log).

**Edge cases.**
- [ ] Batch 64 at 262,144 positions.
- [ ] A looped deployment's synchronization count.
- [ ] A test that passes on today's tree: it is not a RED; find out why before freezing it.

**Implementation.** One test function per row of the section, named as the row names it, opening with the
handed-reds gate; inputs generated by the fixtures the intake and fuzz tickets provide.

**Success.**
- [ ] Every row of section 10.3 is a frozen RED (receipt: `frozen_reds_lock`).
- [ ] Every test fails today for its stated reason (receipt: the RED log).

**Mutants to kill.**
- [ ] A test whose assertion is weakened: `frozen_reds_lock` reds.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --features handed-reds --test samba_exam_decode_lock
cargo nextest run -p logicaffeine-tests --test frozen_reds_lock
```

**Done when.** Section 10.3 exists as tests before anyone sets them for us.

#### EXAM-4 · Prefill and the first token

| Field | Value |
|---|---|
| Phase | exam |
| Labels | `exam` |
| Depends on | none |
| Touches | the `samba_exam_prefill_lock` suite (new); `crates/logicaffeine_tests/tests/frozen_reds_lock.rs` fixture rows |
| Status | open |

**Goal.** Write every test of section 10.4 as a handed RED in `samba_exam_prefill_lock`, each failing today for the reason its ticket
states, each frozen so no later change can weaken it. The family's tickets turn them green one by one.

**Read first.**
- Section 10.4 of this book.
- `crates/logicaffeine_tests/tests/frozen_reds_lock.rs`: how a handed RED is frozen.

**RED tests.**
- [ ] `samba_exam_prefill_lock::*`: every test of section 10.4, each red today for its stated reason (receipt: the RED log).

**Edge cases.**
- [ ] A chunk that is not a multiple of the tile.
- [ ] A GPU-prefilled cache.
- [ ] A test that passes on today's tree: it is not a RED; find out why before freezing it.

**Implementation.** One test function per row of the section, named as the row names it, opening with the
handed-reds gate; inputs generated by the fixtures the intake and fuzz tickets provide.

**Success.**
- [ ] Every row of section 10.4 is a frozen RED (receipt: `frozen_reds_lock`).
- [ ] Every test fails today for its stated reason (receipt: the RED log).

**Mutants to kill.**
- [ ] A test whose assertion is weakened: `frozen_reds_lock` reds.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --features handed-reds --test samba_exam_prefill_lock
cargo nextest run -p logicaffeine-tests --test frozen_reds_lock
```

**Done when.** Section 10.4 exists as tests before anyone sets them for us.

#### EXAM-5 · Socket scaling

| Field | Value |
|---|---|
| Phase | exam |
| Labels | `exam` |
| Depends on | none |
| Touches | the `samba_exam_scaling_lock` suite (new); `crates/logicaffeine_tests/tests/frozen_reds_lock.rs` fixture rows |
| Status | open |

**Goal.** Write every test of section 10.5 as a handed RED in `samba_exam_scaling_lock`, each failing today for the reason its ticket
states, each frozen so no later change can weaken it. The family's tickets turn them green one by one.

**Read first.**
- Section 10.5 of this book.
- `crates/logicaffeine_tests/tests/frozen_reds_lock.rs`: how a handed RED is frozen.

**RED tests.**
- [ ] `samba_exam_scaling_lock::*`: every test of section 10.5, each red today for its stated reason (receipt: the RED log).

**Edge cases.**
- [ ] A width the heads do not divide.
- [ ] Two nodes.
- [ ] A test that passes on today's tree: it is not a RED; find out why before freezing it.

**Implementation.** One test function per row of the section, named as the row names it, opening with the
handed-reds gate; inputs generated by the fixtures the intake and fuzz tickets provide.

**Success.**
- [ ] Every row of section 10.5 is a frozen RED (receipt: `frozen_reds_lock`).
- [ ] Every test fails today for its stated reason (receipt: the RED log).

**Mutants to kill.**
- [ ] A test whose assertion is weakened: `frozen_reds_lock` reds.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --features handed-reds --test samba_exam_scaling_lock
cargo nextest run -p logicaffeine-tests --test frozen_reds_lock
```

**Done when.** Section 10.5 exists as tests before anyone sets them for us.

#### EXAM-6 · Adversarial expert routing

| Field | Value |
|---|---|
| Phase | exam |
| Labels | `exam` |
| Depends on | none |
| Touches | the `samba_exam_moe_lock` and `samba_exam_moe_reds_lock` suites (new); `crates/logicaffeine_tests/tests/frozen_reds_lock.rs` fixture rows |
| Status | open |

**Goal.** Write every test of section 10.6 as a handed RED in `samba_exam_moe_reds_lock`, each failing today for the reason its
ticket states, each frozen so no later change can weaken it; a row that holds lives in `samba_exam_moe_lock`. The family's
tickets turn them green one by one.

**Read first.**
- Section 10.6 of this book.
- `crates/logicaffeine_tests/tests/frozen_reds_lock.rs`: how a handed RED is frozen.

**RED tests.**
- [x] `samba_exam_moe_reds_lock::*`: E6.1, E6.4, E6.6 written, each red today for its stated reason and frozen
  (receipt: `logs/samba/e6-classify-1.log`, `fixtures/frozen_reds.tsv`).
- [x] `samba_exam_moe_lock::*`: E6.5 hold on the SN50 bundle `largo emit rdu` writes (receipt: `logs/samba/e6-classify-1.log`);
  each shows its call matters (deciding it the other way moves the reference's logits past twice the datapath's bound).
- [ ] Owed: E6.2 and E6.7 in `samba_exam_scaling_lock` (a batch decoded across sockets, its experts dispatched in either
  form, MULTI-2) and E6.3 in `samba_exam_overflow_lock` (512 experts' weights streamed from memory a step, MEM-6), each
  written when what it needs exists.

**Edge cases.**
- [ ] Every token on one expert.
- [ ] Router ties and a NaN router row.
- [x] A test that passes on today's tree: it is not a RED; find out why before freezing it (E6.5 holds because
  ARCH-ROUTING landed the routing first; it lives in `samba_exam_moe_lock`, never frozen). And its converse: E6.1 and
  E6.4 fail, but not on their routing — probes of the bundle against the reference, the reference routed otherwise and
  each sequence decoded alone show every call made the reference's way and the batch decoding each sequence as it
  decodes alone, so their ticket is NUM-1's datapath.

**Implementation.** One test function per row of the section, named as the row names it, opening with the
handed-reds gate; inputs generated by the fixtures the intake and fuzz tickets provide.

**Success.**
- [ ] Every row of section 10.6 is a frozen RED (receipt: `frozen_reds_lock`).
- [ ] Every test fails today for its stated reason (receipt: the RED log).

**Mutants to kill.**
- [ ] A test whose assertion is weakened: `frozen_reds_lock` reds.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test samba_exam_moe_lock
cargo nextest run -p logicaffeine-tests --features handed-reds --cargo-profile test-opt --test samba_exam_moe_reds_lock
cargo nextest run -p logicaffeine-tests --test frozen_reds_lock
```

**Done when.** Section 10.6 exists as tests before anyone sets them for us.

#### EXAM-7 · Speculative decoding

| Field | Value |
|---|---|
| Phase | exam |
| Labels | `exam` |
| Depends on | none |
| Touches | the `samba_exam_spec_lock` suite (new); `crates/logicaffeine_tests/tests/frozen_reds_lock.rs` fixture rows |
| Status | open |

**Goal.** Write every test of section 10.7 as a handed RED in `samba_exam_spec_lock`, each failing today for the reason its ticket
states, each frozen so no later change can weaken it. The family's tickets turn them green one by one.

**Read first.**
- Section 10.7 of this book.
- `crates/logicaffeine_tests/tests/frozen_reds_lock.rs`: how a handed RED is frozen.

**RED tests.**
- [ ] `samba_exam_spec_lock::*`: every test of section 10.7, each red today for its stated reason (receipt: the RED log).

**Edge cases.**
- [ ] A rejection at the first draft.
- [ ] k = 9.
- [ ] A test that passes on today's tree: it is not a RED; find out why before freezing it.

**Implementation.** One test function per row of the section, named as the row names it, opening with the
handed-reds gate; inputs generated by the fixtures the intake and fuzz tickets provide.

**Success.**
- [ ] Every row of section 10.7 is a frozen RED (receipt: `frozen_reds_lock`).
- [ ] Every test fails today for its stated reason (receipt: the RED log).

**Mutants to kill.**
- [ ] A test whose assertion is weakened: `frozen_reds_lock` reds.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --features handed-reds --test samba_exam_spec_lock
cargo nextest run -p logicaffeine-tests --test frozen_reds_lock
```

**Done when.** Section 10.7 exists as tests before anyone sets them for us.

#### EXAM-8 · Compiler quality

| Field | Value |
|---|---|
| Phase | exam |
| Labels | `exam` |
| Depends on | none |
| Touches | the `samba_exam_compiler_lock` suite (new); `crates/logicaffeine_tests/tests/frozen_reds_lock.rs` fixture rows |
| Status | open |

**Goal.** Write every test of section 10.8 as a handed RED in `samba_exam_compiler_lock`, each failing today for the reason its ticket
states, each frozen so no later change can weaken it. The family's tickets turn them green one by one.

**Read first.**
- Section 10.8 of this book.
- `crates/logicaffeine_tests/tests/frozen_reds_lock.rs`: how a handed RED is frozen.

**RED tests.**
- [ ] `samba_exam_compiler_lock::*`: every test of section 10.8, each red today for its stated reason (receipt: the RED log).

**Edge cases.**
- [ ] A 405B and a 671B compile.
- [ ] A recompile after a batch change.
- [ ] A test that passes on today's tree: it is not a RED; find out why before freezing it.

**Implementation.** One test function per row of the section, named as the row names it, opening with the
handed-reds gate; inputs generated by the fixtures the intake and fuzz tickets provide.

**Success.**
- [ ] Every row of section 10.8 is a frozen RED (receipt: `frozen_reds_lock`).
- [ ] Every test fails today for its stated reason (receipt: the RED log).

**Mutants to kill.**
- [ ] A test whose assertion is weakened: `frozen_reds_lock` reds.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --features handed-reds --test samba_exam_compiler_lock
cargo nextest run -p logicaffeine-tests --test frozen_reds_lock
```

**Done when.** Section 10.8 exists as tests before anyone sets them for us.

#### EXAM-9 · Programs larger than a chip

| Field | Value |
|---|---|
| Phase | exam |
| Labels | `exam` |
| Depends on | none |
| Touches | the `samba_exam_overflow_lock` suite (new); `crates/logicaffeine_tests/tests/frozen_reds_lock.rs` fixture rows |
| Status | open |

**Goal.** Write every test of section 10.9 as a handed RED in `samba_exam_overflow_lock`, each failing today for the reason its ticket
states, each frozen so no later change can weaken it. The family's tickets turn them green one by one.

**Read first.**
- Section 10.9 of this book.
- `crates/logicaffeine_tests/tests/frozen_reds_lock.rs`: how a handed RED is frozen.

**RED tests.**
- [ ] `samba_exam_overflow_lock::*`: every test of section 10.9, each red today for its stated reason (receipt: the RED log).

**Edge cases.**
- [ ] A layer larger than SRAM.
- [ ] A table larger than HBM.
- [ ] A test that passes on today's tree: it is not a RED; find out why before freezing it.

**Implementation.** One test function per row of the section, named as the row names it, opening with the
handed-reds gate; inputs generated by the fixtures the intake and fuzz tickets provide.

**Success.**
- [ ] Every row of section 10.9 is a frozen RED (receipt: `frozen_reds_lock`).
- [ ] Every test fails today for its stated reason (receipt: the RED log).

**Mutants to kill.**
- [ ] A test whose assertion is weakened: `frozen_reds_lock` reds.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --features handed-reds --test samba_exam_overflow_lock
cargo nextest run -p logicaffeine-tests --test frozen_reds_lock
```

**Done when.** Section 10.9 exists as tests before anyone sets them for us.

#### EXAM-10 · A new op, first run then fused

| Field | Value |
|---|---|
| Phase | exam |
| Labels | `exam` |
| Depends on | none |
| Touches | the `samba_exam_agility_lock` suite (new); `crates/logicaffeine_tests/tests/frozen_reds_lock.rs` fixture rows |
| Status | open |

**Goal.** Write every test of section 10.10 as a handed RED in `samba_exam_agility_lock`, each failing today for the reason its ticket
states, each frozen so no later change can weaken it. The family's tickets turn them green one by one.

**Read first.**
- Section 10.10 of this book.
- `crates/logicaffeine_tests/tests/frozen_reds_lock.rs`: how a handed RED is frozen.

**RED tests.**
- [ ] `samba_exam_agility_lock::*`: every test of section 10.10, each red today for its stated reason (receipt: the RED log).

**Edge cases.**
- [ ] A new activation, mask, recurrence and router.
- [ ] The time from writing to fused.
- [ ] A test that passes on today's tree: it is not a RED; find out why before freezing it.

**Implementation.** One test function per row of the section, named as the row names it, opening with the
handed-reds gate; inputs generated by the fixtures the intake and fuzz tickets provide.

**Success.**
- [ ] Every row of section 10.10 is a frozen RED (receipt: `frozen_reds_lock`).
- [ ] Every test fails today for its stated reason (receipt: the RED log).

**Mutants to kill.**
- [ ] A test whose assertion is weakened: `frozen_reds_lock` reds.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --features handed-reds --test samba_exam_agility_lock
cargo nextest run -p logicaffeine-tests --test frozen_reds_lock
```

**Done when.** Section 10.10 exists as tests before anyone sets them for us.

#### EXAM-11 · The kernel micro-exam

| Field | Value |
|---|---|
| Phase | exam |
| Labels | `exam` |
| Depends on | none |
| Touches | the `samba_exam_kernel_lock` suite (new); `crates/logicaffeine_tests/tests/frozen_reds_lock.rs` fixture rows |
| Status | open |

**Goal.** Write every test of section 10.11 as a handed RED in `samba_exam_kernel_lock`, each failing today for the reason its ticket
states, each frozen so no later change can weaken it. The family's tickets turn them green one by one.

**Read first.**
- Section 10.11 of this book.
- `crates/logicaffeine_tests/tests/frozen_reds_lock.rs`: how a handed RED is frozen.

**RED tests.**
- [ ] `samba_exam_kernel_lock::*`: every test of section 10.11, each red today for its stated reason (receipt: the RED log).

**Edge cases.**
- [ ] A prime-length reduction.
- [ ] Repeated scatter indices.
- [ ] A test that passes on today's tree: it is not a RED; find out why before freezing it.

**Implementation.** One test function per row of the section, named as the row names it, opening with the
handed-reds gate; inputs generated by the fixtures the intake and fuzz tickets provide.

**Success.**
- [ ] Every row of section 10.11 is a frozen RED (receipt: `frozen_reds_lock`).
- [ ] Every test fails today for its stated reason (receipt: the RED log).

**Mutants to kill.**
- [ ] A test whose assertion is weakened: `frozen_reds_lock` reds.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --features handed-reds --test samba_exam_kernel_lock
cargo nextest run -p logicaffeine-tests --test frozen_reds_lock
```

**Done when.** Section 10.11 exists as tests before anyone sets them for us.

#### EXAM-12 · Multimodal input, serving and the Qwen 4 day zero

| Field | Value |
|---|---|
| Phase | exam |
| Labels | `exam` |
| Depends on | none |
| Touches | the `samba_exam_serving_lock` suite (new); `crates/logicaffeine_tests/tests/frozen_reds_lock.rs` fixture rows |
| Status | open |

**Goal.** Write every test of section 10.12 as a handed RED in `samba_exam_serving_lock`, each failing today for the reason its ticket
states, each frozen so no later change can weaken it. The family's tickets turn them green one by one.

**Read first.**
- Section 10.12 of this book.
- `crates/logicaffeine_tests/tests/frozen_reds_lock.rs`: how a handed RED is frozen.

**RED tests.**
- [ ] `samba_exam_serving_lock::*`: every test of section 10.12, each red today for its stated reason (receipt: the RED log).

**Edge cases.**
- [ ] Two images in one prompt.
- [ ] A stop string split across tokens.
- [ ] A test that passes on today's tree: it is not a RED; find out why before freezing it.

**Implementation.** One test function per row of the section, named as the row names it, opening with the
handed-reds gate; inputs generated by the fixtures the intake and fuzz tickets provide.

**Success.**
- [ ] Every row of section 10.12 is a frozen RED (receipt: `frozen_reds_lock`).
- [ ] Every test fails today for its stated reason (receipt: the RED log).

**Mutants to kill.**
- [ ] A test whose assertion is weakened: `frozen_reds_lock` reds.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --features handed-reds --test samba_exam_serving_lock
cargo nextest run -p logicaffeine-tests --test frozen_reds_lock
```

**Done when.** Section 10.12 exists as tests before anyone sets them for us.

#### EXAM-13 · The Plasticine kit's workloads

| Field | Value |
|---|---|
| Phase | exam |
| Labels | `exam` |
| Depends on | none |
| Touches | the `samba_exam_kit_lock` suite (new); `crates/logicaffeine_tests/tests/frozen_reds_lock.rs` fixture rows |
| Status | open |

**Goal.** Write every test of section 10.13 as a handed RED in `samba_exam_kit_lock`, each failing today for the reason its ticket
states, each frozen so no later change can weaken it. The family's tickets turn them green one by one.

**Read first.**
- Section 10.13 of this book.
- `crates/logicaffeine_tests/tests/frozen_reds_lock.rs`: how a handed RED is frozen.

**RED tests.**
- [ ] `samba_exam_kit_lock::*`: every test of section 10.13, each red today for its stated reason (receipt: the RED log).

**Edge cases.**
- [ ] A module with its own loss.
- [ ] Weight sparsity of 90%.
- [ ] A test that passes on today's tree: it is not a RED; find out why before freezing it.

**Implementation.** One test function per row of the section, named as the row names it, opening with the
handed-reds gate; inputs generated by the fixtures the intake and fuzz tickets provide.

**Success.**
- [ ] Every row of section 10.13 is a frozen RED (receipt: `frozen_reds_lock`).
- [ ] Every test fails today for its stated reason (receipt: the RED log).

**Mutants to kill.**
- [ ] A test whose assertion is weakened: `frozen_reds_lock` reds.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --features handed-reds --test samba_exam_kit_lock
cargo nextest run -p logicaffeine-tests --test frozen_reds_lock
```

**Done when.** Section 10.13 exists as tests before anyone sets them for us.
#### SCORE-1 · The cell ledger

| Field | Value |
|---|---|
| Phase | score |
| Labels | `infrastructure` |
| Depends on | none |
| Touches | the cell ledger and `samba_cells_lock` (new) |
| Status | open |

**Goal.** The ledger of section 2.3: one row per cell and side, with provenance, program identity, bundle digest, the
time model's version where modeled, quality gates, boot and load where measured, and the log. Its schema is approved by
Tristen before the first row.

**Read first.**
- Sections 2.3 and 2.4.
- The fusion book's LEDGER-3 (`sota_cells.json`), the schema this one mirrors.

**RED tests.**
- [x] `samba_cells_lock::a_won_row_is_never_modeled`: fails today: no ledger (receipt: RED `logs/samba/red-ledgers-int64.log`; green `logs/samba/green-2.log`).
- [x] `samba_cells_lock::a_won_row_carries_its_quality_gates`: fails today (receipt: RED `logs/samba/red-ledgers-int64.log`; green `logs/samba/green-2.log`).

**Edge cases.**
- [x] A cell with only a published side (receipt: `samba_cells_lock::a_cell_with_only_a_published_side_is_a_bar_and_no_claim`).
- [x] Two published sides for one cell: both kept, the later cited (receipt: `samba_cells_lock::two_published_sides_are_both_kept_and_the_later_is_the_bar`).

**Implementation.** A JSON schema; a lock that reads it.

**Success.**
- [ ] The schema approved and the lock green on an empty ledger (receipt: the lock, the approval line).

**Mutants to kill.**
- [x] A won modeled row (receipt: `logs/samba/mutants2/results.json`, score1-won-modeled-row killed).

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_cells_lock
```

**Done when.** Every score has one home.

#### SCORE-2 · Every Copy bar a ledger row

| Field | Value |
|---|---|
| Phase | score |
| Labels | `infrastructure` |
| Depends on | SCORE-1, STUDY-1 |
| Touches | the cell ledger (new) |
| Status | open |

**Goal.** Every row of section 2.2 is a published row of the ledger with its pinned citation.

**Read first.**
- Section 2.2.

**RED tests.**
- [ ] `samba_cells_lock::every_copy_bar_is_a_published_row`: fails today.

**Edge cases.**
- [ ] A Copy bar superseded by a newer publication.

**Implementation.** Rows written from the pinned sources.

**Success.**
- [ ] The ledger's published rows equal section 2.2 (receipt: the lock).

**Mutants to kill.**
- [ ] A published row without a citation.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_cells_lock
```

**Done when.** The bars are data.

#### SCORE-3 · Our rows, with their provenance

| Field | Value |
|---|---|
| Phase | score |
| Labels | `infrastructure` |
| Depends on | SCORE-1, TIME-4 |
| Touches | the cell ledger (new) |
| Status | open |

**Goal.** Our side of every cell: modeled rows from the calibrated time model for every Tier A deployment, simulated
rows from Tungsten for every Plasticine cell, measured rows when hardware answers.

**Read first.**
- TIME-4.

**RED tests.**
- [ ] `samba_cells_lock::every_tier_a_cell_has_our_row`: fails today.

**Edge cases.**
- [ ] A cell we cannot yet run: an explicit "not yet" row naming its closers.

**Implementation.** The exam runner writes rows.

**Success.**
- [ ] Our side present for every Tier A cell (receipt: the lock).

**Mutants to kill.**
- [ ] A row without its program identity.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_cells_lock
```

**Done when.** We know where we stand on every cell.

#### SCORE-4 · A lost cell grows tickets

| Field | Value |
|---|---|
| Phase | score |
| Labels | `infrastructure` |
| Depends on | SCORE-3, TIME-6 |
| Touches | the cell ledger (new) |
| Status | open |

**Goal.** Section 2.5's rule as a check: a cell whose best row is behind its Copy bar names its closers, each a ticket
on this board, each with the attributed cause it closes.

**Read first.**
- Section 2.5.

**RED tests.**
- [ ] `samba_cells_lock::every_lost_cell_names_its_closers`: fails today.

**Edge cases.**
- [ ] A cell lost on a published number whose conditions we cannot match: stated, never silently dropped.

**Implementation.** The ledger's closers field; the lock checks each against the board.

**Success.**
- [ ] Every lost cell has closers (receipt: the lock).

**Mutants to kill.**
- [ ] A closer that is not on the board.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test samba_cells_lock
```

**Done when.** Losing a cell starts the work to win it.

### 11.21 Deployments

One ticket per family: the family's bundle through the generic path, certified, equal to its reference on its tiny
configuration, its real-width layers lowered, its cells modeled. Each is the integration test of the tickets it depends
on.
#### DEPLOY-QWEN38 · Qwen3.8-27B and Qwen3.5 on the generic path

| Field | Value |
|---|---|
| Phase | deployment |
| Labels | `infrastructure` `rdu` |
| Depends on | INTAKE-6 |
| Touches | the family's deployment lock (new) |
| Status | open |

**Goal.** Deploy the family on SN50 and SN40L: the RDU path's first family, now read through the generic intake and kinds by digest. The bundle is certified, its tiny configuration equals the
VM and the family's reference implementations from the files on disk, its real-width layers lower, its parallel plan
is the time model's, and its D, L, SPEC and TTFT cells are modeled with provenance beside their Copy bars.

**Read first.**
- Section 6.1's row for the family.
- `crates/logicaffeine_tests/tests/emit_rdu_e2e.rs`: how a bundle on disk is run against the VM.

**RED tests.**
- [x] `qwen38_real_dims_lock::the_tiny_bundle_equals_the_vm_and_the_references`: Qwen 3.5's twin brings up from its
  directory (`logs/samba/green-qwen38-deploy.log`), its Gated DeltaNet of four heads of 16 blocked by head, its memory
  being past one accumulator page of a row (`logs/samba/red-gdn-page.log`, `logs/samba/green-gdn-page.log`). The
  mixture's twin is a handed RED on NUM-1
  (`deploy_reds_lock::qwen_3_5s_mixtures_tiny_bundle_equals_the_vm_and_the_references`): lowered with fp32 in place
  of bf16 it is the reference's within 1.4e-6 of the largest logit, its routing is the reference's (the closest call
  decided by 0.036 on a score of 0.23), and on the bf16 datapath it lands 2.54% of the largest logit off against the 2%
  bound, 1.34% with the SIMD path's fp32 witness (`logs/samba/diag-qwen35moe-layers.log`,
  `logs/samba/diag-qwen35moe-simd-fp32.log`).
- [x] `qwen38_real_dims_lock::every_real_width_layer_kind_lowers_and_certifies`: every dense release, and the
  mixtures by `qwen38_real_dims_lock::every_real_width_mixture_layer_kind_lowers_and_certifies`
  (`logs/samba/green-qwen38-deploy.log`).
- [ ] `qwen38_real_dims_lock::every_cell_has_a_modeled_row`: fails today.

**Edge cases.**
- [x] Every size the family publishes, each a configuration of the same lock: Qwen3.5-0.8B, 2B, 4B, 9B, 27B,
  35B-A3B, 122B-A10B and 397B-A17B, Qwen3.8-27B and 2.4T-A95B, each held to its published `config.json`
  (`qwen38_real_dims_lock::every_size_is_its_published_configuration`) and read by the intake
  (`model_hf_intake_lock::every_published_configuration_reads`). Qwen3.8 states Gated DeltaNet's gate activation
  (`output_gate_type`), read as swish, the SiLU the spec holds
  (`model_hf_intake_lock::a_gated_deltanets_output_gate_is_read_by_its_activation`). Qwen3.8-2.4T's own layout, the
  text model alone (`Qwen3_5MoeForCausalLM`) under `model.` with its experts stored fused (`experts.gate_up_proj` as
  experts × 2·dff × d, `experts.down_proj`), reads as the converter reads it
  (`model_hf_intake_lock::a_qwen35moe_text_model_reads_as_its_converted_gguf`) and its twin brings up on the SN50
  (`qwen38_real_dims_lock::the_text_model_layouts_tiny_bundle_equals_the_vm_and_the_references`,
  `logs/samba/red-qwen35moe-causal.log`, `logs/samba/green-qwen35moe-causal.log`).
- [ ] The family's quantized releases at their own bits.

**Implementation.** The generic path end to end; no code specific to the family outside its spec arm and its
features' tickets.

**Success.**
- [ ] Certified, equal, lowered and priced (receipt: the lock and the cell ledger).

**Mutants to kill.**
- [ ] One layer bound to another layer's weights.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test qwen38_real_dims_lock
```

**Done when.** The family runs on the RDU and knows how fast.

#### DEPLOY-LLAMA3 · Llama 3.x, 1B to 405B

| Field | Value |
|---|---|
| Phase | deployment |
| Labels | `infrastructure` `rdu` |
| Depends on | INTAKE-6, ARCH-ROPE |
| Touches | the family's deployment lock (new) |
| Status | open |

**Goal.** Deploy the family on SN50 and SN40L: SambaNova's most-published family [KL24] [DIAYN26], with llama3 rotation scaling. The bundle is certified, its tiny configuration equals the
VM and the family's reference implementations from the files on disk, its real-width layers lower, its parallel plan
is the time model's, and its D, L, SPEC and TTFT cells are modeled with provenance beside their Copy bars.

**Read first.**
- Section 6.1's row for the family.
- `crates/logicaffeine_tests/tests/emit_rdu_e2e.rs`: how a bundle on disk is run against the VM.

**RED tests.**
- [x] `deploy_llama3_lock::the_tiny_bundle_equals_the_vm_and_the_references`: the family's twin (`hf-tiny-twin.py llama`: the interleaved rotation under Llama 3's frequency factors, a tied head) from its directory: transformers, the f64 reference (within a millionth), the VM and the SN50 bundle (within the bf16 bound) agree on its first step; green on its first run, the
  generic path having deployed the family since INTAKE-6 (receipt: `logs/samba/deploy-2.log`).
- [x] `deploy_llama3_lock::every_real_width_layer_kind_lowers_and_certifies`: Llama 3.2 1B and 3B, 3.1 8B, 70B (3.3 70B's shape) and 405B, each at its real widths, every
  layer kind lowered for the SN50 and certified (receipt: `logs/samba/deploy-1.log`).
- [ ] `deploy_llama3_lock::every_cell_has_a_modeled_row`: fails today.

**Edge cases.**
- [ ] Every size the family publishes, each a configuration of the same lock.
- [ ] The family's quantized releases at their own bits.

**Implementation.** The generic path end to end; no code specific to the family outside its spec arm and its
features' tickets.

**Success.**
- [ ] Certified, equal, lowered and priced (receipt: the lock and the cell ledger).

**Mutants to kill.**
- [ ] One layer bound to another layer's weights.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test deploy_llama3_lock
```

**Done when.** The family runs on the RDU and knows how fast.

#### DEPLOY-QWEN25 · Qwen 2.5, 0.5B to 72B

| Field | Value |
|---|---|
| Phase | deployment |
| Labels | `infrastructure` `rdu` |
| Depends on | INTAKE-6 |
| Touches | the family's deployment lock (new) |
| Status | open |

**Goal.** Deploy the family on SN50 and SN40L: in SambaNova's evaluation set as draft and target [DIAYN26], with QKV biases. The bundle is certified, its tiny configuration equals the
VM and the family's reference implementations from the files on disk, its real-width layers lower, its parallel plan
is the time model's, and its D, L, SPEC and TTFT cells are modeled with provenance beside their Copy bars.

**Read first.**
- Section 6.1's row for the family.
- `crates/logicaffeine_tests/tests/emit_rdu_e2e.rs`: how a bundle on disk is run against the VM.

**RED tests.**
- [x] `deploy_qwen25_lock::the_tiny_bundle_equals_the_vm_and_the_references`: the family's twin (`hf-tiny-twin.py qwen2`: biased query, key and value projections, a tied head) from its directory: transformers, the f64 reference (within a millionth), the VM and the SN50 bundle (within the bf16 bound) agree on its first step; green on its first run, the
  generic path having deployed the family since INTAKE-6 (receipt: `logs/samba/deploy-2.log`).
- [x] `deploy_qwen25_lock::every_real_width_layer_kind_lowers_and_certifies`: Qwen 2.5 0.5B, 1.5B, 3B, 7B, 14B, 32B and 72B, each at its real widths, every
  layer kind lowered for the SN50 and certified (receipt: `logs/samba/deploy-1.log`).
- [ ] `deploy_qwen25_lock::every_cell_has_a_modeled_row`: fails today.

**Edge cases.**
- [ ] Every size the family publishes, each a configuration of the same lock.
- [ ] The family's quantized releases at their own bits.

**Implementation.** The generic path end to end; no code specific to the family outside its spec arm and its
features' tickets.

**Success.**
- [ ] Certified, equal, lowered and priced (receipt: the lock and the cell ledger).

**Mutants to kill.**
- [ ] One layer bound to another layer's weights.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test deploy_qwen25_lock
```

**Done when.** The family runs on the RDU and knows how fast.

#### DEPLOY-MIXTRAL · Mixtral 8x7B

| Field | Value |
|---|---|
| Phase | deployment |
| Labels | `infrastructure` `rdu` |
| Depends on | ARCH-MIXTRAL, INTAKE-6 |
| Touches | the family's deployment lock (new) |
| Status | open |

**Goal.** Deploy the family on SN50 and SN40L: in SambaNova's evaluation set [DIAYN26], read as experts. The bundle is certified, its tiny configuration equals the
VM and the family's reference implementations from the files on disk, its real-width layers lower, its parallel plan
is the time model's, and its D, L, SPEC and TTFT cells are modeled with provenance beside their Copy bars.

**Read first.**
- Section 6.1's row for the family.
- `crates/logicaffeine_tests/tests/emit_rdu_e2e.rs`: how a bundle on disk is run against the VM.

**RED tests.**
- [x] `deploy_mixtral_lock::the_tiny_bundle_equals_the_vm_and_the_references`: the family's twin (`hf-tiny-twin.py mixtral`: Mistral's interleaved rotation, experts two a token, the kept softmax weights renormalized); transformers, the f64
  reference (within a millionth), the VM and the SN50 bundle from its directory (within the bf16 bound) agree on its
  first step (receipt: `logs/samba/deploy-3.log`).
- [x] `deploy_mixtral_lock::every_real_width_layer_kind_lowers_and_certifies`: Mixtral 8x7B and 8x22B, each at its real widths, every
  layer kind lowered for the SN50 and certified (receipt: `logs/samba/deploy-3.log`).
- [ ] `deploy_mixtral_lock::every_cell_has_a_modeled_row`: fails today.

**Edge cases.**
- [ ] Every size the family publishes, each a configuration of the same lock.
- [ ] The family's quantized releases at their own bits.

**Implementation.** The generic path end to end; no code specific to the family outside its spec arm and its
features' tickets.

**Success.**
- [ ] Certified, equal, lowered and priced (receipt: the lock and the cell ledger).

**Mutants to kill.**
- [ ] One layer bound to another layer's weights.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test deploy_mixtral_lock
```

**Done when.** The family runs on the RDU and knows how fast.

#### DEPLOY-OSS · gpt-oss 20B and 120B

| Field | Value |
|---|---|
| Phase | deployment |
| Labels | `infrastructure` `rdu` |
| Depends on | ARCH-OSS, ARCH-SINKS, ARCH-WINDOW, INTAKE-6 |
| Touches | the family's deployment lock (new) |
| Status | open |

**Goal.** Deploy the family on SN50 and SN40L: in SambaNova's evaluation set at 8K and 64K [DIAYN26], with sinks, alternating windows and MXFP4 experts. The bundle is certified, its tiny configuration equals the
VM and the family's reference implementations from the files on disk, its real-width layers lower, its parallel plan
is the time model's, and its D, L, SPEC and TTFT cells are modeled with provenance beside their Copy bars.

**Read first.**
- Section 6.1's row for the family.
- `crates/logicaffeine_tests/tests/emit_rdu_e2e.rs`: how a bundle on disk is run against the VM.

**RED tests.**
- [x] `deploy_oss_lock::the_tiny_bundle_equals_the_vm_and_the_references`: the family's twin (`hf-tiny-twin.py gpt-oss`: biased attention with sinks, a 128-token window on alternate layers, YaRN, biased experts under the clamped SwiGLU); transformers, the f64
  reference (within a millionth), the VM and the SN50 bundle from its directory (within the bf16 bound) agree on its
  first step (receipt: `logs/samba/deploy-4.log`).
- [x] `deploy_oss_lock::every_real_width_layer_kind_lowers_and_certifies`: gpt-oss-20b and gpt-oss-120b, each at its real widths, every
  layer kind lowered for the SN50 and certified (receipt: `logs/samba/deploy-3.log`).
- [ ] `deploy_oss_lock::every_cell_has_a_modeled_row`: fails today.

**Edge cases.**
- [ ] Every size the family publishes, each a configuration of the same lock.
- [ ] The family's quantized releases at their own bits.

**Implementation.** The generic path end to end; no code specific to the family outside its spec arm and its
features' tickets.

**Success.**
- [ ] Certified, equal, lowered and priced (receipt: the lock and the cell ledger).

**Mutants to kill.**
- [ ] One layer bound to another layer's weights.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test deploy_oss_lock
```

**Done when.** The family runs on the RDU and knows how fast.

#### DEPLOY-DEEPSEEK3 · DeepSeek-V3 and R1

| Field | Value |
|---|---|
| Phase | deployment |
| Labels | `infrastructure` `rdu` |
| Depends on | ARCH-MLA, ARCH-ROUTING, ARCH-DENSE-FIRST, MULTI-2, NUM-2, INTAKE-6 |
| Touches | the family's deployment lock (new) |
| Status | open |

**Goal.** Deploy the family on SN50 and SN40L: SambaNova's scaling showcase from 64 to 256 sockets [HC26], with latent attention, group-limited routing, dense leading layers and FP8 weights. The bundle is certified, its tiny configuration equals the
VM and the family's reference implementations from the files on disk, its real-width layers lower, its parallel plan
is the time model's, and its D, L, SPEC and TTFT cells are modeled with provenance beside their Copy bars.

**Read first.**
- Section 6.1's row for the family.
- `crates/logicaffeine_tests/tests/emit_rdu_e2e.rs`: how a bundle on disk is run against the VM.

**RED tests.**
- [x] `deploy_deepseek3_lock::the_tiny_bundle_equals_the_vm_and_the_references`: the family's twin (`hf-tiny-twin.py deepseek2`, a DeepSeek-V3: latent attention with a query LoRA and YaRN, a dense first block, sigmoid routing within the top groups with a selection bias, a shared expert and a routed scale); transformers, the f64
  reference (within a millionth), the VM and the SN50 bundle from its directory (within the bf16 bound) agree on its
  first step (receipt: `logs/samba/deploy-3.log`).
- [x] `deploy_deepseek3_lock::every_real_width_layer_kind_lowers_and_certifies`: DeepSeek-V3's shape, which R1 shares (7,168 wide, 256 experts in 8 groups, a latent of 512), each at its real widths, every
  layer kind lowered for the SN50 and certified (receipt: `logs/samba/deploy-3.log`).
- [ ] `deploy_deepseek3_lock::every_cell_has_a_modeled_row`: fails today.

**Edge cases.**
- [ ] Every size the family publishes, each a configuration of the same lock.
- [ ] The family's quantized releases at their own bits.

**Implementation.** The generic path end to end; no code specific to the family outside its spec arm and its
features' tickets.

**Success.**
- [ ] Certified, equal, lowered and priced (receipt: the lock and the cell ledger).

**Mutants to kill.**
- [ ] One layer bound to another layer's weights.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test deploy_deepseek3_lock
```

**Done when.** The family runs on the RDU and knows how fast.

#### DEPLOY-LLAMA4 · Llama 4 Scout and Maverick

| Field | Value |
|---|---|
| Phase | deployment |
| Labels | `infrastructure` `rdu` |
| Depends on | ARCH-WINDOW, ARCH-NOPE, ARCH-DENSE-FIRST, MULTI-2, INTAKE-6 |
| Touches | the family's deployment lock (new) |
| Status | open |

**Goal.** Deploy the family on SN50 and SN40L: in SambaNova's evaluation set at 8K and 32K [DIAYN26], with chunked attention, rotation-free layers and interleaved expert layers. The bundle is certified, its tiny configuration equals the
VM and the family's reference implementations from the files on disk, its real-width layers lower, its parallel plan
is the time model's, and its D, L, SPEC and TTFT cells are modeled with provenance beside their Copy bars.

**Read first.**
- Section 6.1's row for the family.
- `crates/logicaffeine_tests/tests/emit_rdu_e2e.rs`: how a bundle on disk is run against the VM.

**RED tests.**
- [x] `deploy_llama4_lock::the_tiny_bundle_equals_the_vm_and_the_references`: the family's twin (`hf-tiny-twin.py llama4`, Scout's text model: chunked rotated layers with a weightless QK norm, a rotation-free layer with the attention temperature, sigmoid-scaled experts beside a shared one, Llama 3's frequency factors); transformers, the f64
  reference (within a millionth), the VM and the SN50 bundle from its directory (within the bf16 bound) agree on its
  first step (receipt: `logs/samba/deploy-3.log`).
- [x] `deploy_llama4_lock::every_real_width_layer_kind_lowers_and_certifies`: Llama 4 Scout (16 experts) and Maverick (128 experts), each at its real widths, every
  layer kind lowered for the SN50 and certified (receipt: `logs/samba/deploy-3.log`).
- [ ] `deploy_llama4_lock::every_cell_has_a_modeled_row`: fails today.

**Edge cases.**
- [ ] Every size the family publishes, each a configuration of the same lock.
- [ ] The family's quantized releases at their own bits.

**Implementation.** The generic path end to end; no code specific to the family outside its spec arm and its
features' tickets.

**Success.**
- [ ] Certified, equal, lowered and priced (receipt: the lock and the cell ledger).

**Mutants to kill.**
- [ ] One layer bound to another layer's weights.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test deploy_llama4_lock
```

**Done when.** The family runs on the RDU and knows how fast.

#### DEPLOY-MINIMAX · MiniMax M2 series

| Field | Value |
|---|---|
| Phase | deployment |
| Labels | `infrastructure` `rdu` |
| Depends on | MULTI-2, INTAKE-6 |
| Touches | the family's deployment lock (new) |
| Status | open |

**Goal.** Deploy the family on SN50 and SN40L: SambaNova's SN50 showcase [HC26] [SN50MM], whose architecture STUDY-4 reads from its checkpoint. The bundle is certified, its tiny configuration equals the
VM and the family's reference implementations from the files on disk, its real-width layers lower, its parallel plan
is the time model's, and its D, L, SPEC and TTFT cells are modeled with provenance beside their Copy bars.

**Read first.**
- Section 6.1's row for the family.
- `crates/logicaffeine_tests/tests/emit_rdu_e2e.rs`: how a bundle on disk is run against the VM.

**RED tests.**
- [x] `deploy_minimax_lock::the_tiny_bundle_equals_the_vm_and_the_references`: the family's twin (`hf-tiny-twin.py
  minimax_m2`: a QK norm across every head, half of each head rotated, sigmoid-scored experts chosen with a routing
  bias); transformers, the f64 reference (within a millionth), the VM and the SN50 bundle from its directory (within the
  bf16 bound) agree on its first step; failed: the reader did not present `MiniMaxM2ForCausalLM`
  (`logs/samba/red-minimax-deploy.log`) (receipt: `logs/samba/green-minimax.log`).
- [x] `deploy_minimax_lock::every_real_width_layer_kind_lowers_and_certifies`: MiniMax-M2's shape (3,072 wide, 256
  experts, eight a token), every layer kind lowered for the SN50 and certified (receipt: `logs/samba/green-minimax.log`).
- [ ] `deploy_minimax_lock::every_cell_has_a_modeled_row`: fails today.

**Edge cases.**
- [ ] Every size the family publishes, each a configuration of the same lock.
- [ ] The family's quantized releases at their own bits.

**Implementation.** The generic path end to end; no code specific to the family outside its spec arm and its
features' tickets.

**Success.**
- [ ] Certified, equal, lowered and priced (receipt: the lock and the cell ledger).

**Mutants to kill.**
- [ ] One layer bound to another layer's weights.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test deploy_minimax_lock
```

**Done when.** The family runs on the RDU and knows how fast.

#### DEPLOY-GEMMA · Gemma 1, 2 and 3

| Field | Value |
|---|---|
| Phase | deployment |
| Labels | `infrastructure` `rdu` |
| Depends on | ARCH-SOFTCAP, ARCH-WINDOW, ARCH-NORMS, INTAKE-6 |
| Touches | the family's deployment lock (new) |
| Status | open |

**Goal.** Deploy the family on SN50 and SN40L: a Tier C family with soft caps, local and global layers and sandwich norms. The bundle is certified, its tiny configuration equals the
VM and the family's reference implementations from the files on disk, its real-width layers lower, its parallel plan
is the time model's, and its D, L, SPEC and TTFT cells are modeled with provenance beside their Copy bars.

**Read first.**
- Section 6.1's row for the family.
- `crates/logicaffeine_tests/tests/emit_rdu_e2e.rs`: how a bundle on disk is run against the VM.

**RED tests.**
- [ ] `deploy_reds_lock::gemma_3s_tiny_bundle_equals_the_vm_and_the_references`: handed RED, frozen on NUM-1. Gemma 3's
  twin from its directory: transformers, the reference and the VM agree, and on the SN50's bf16 datapath a logit lands
  2.04% of the largest from the reference's against the 2% bound (`logs/samba/red-deploy-gemma-handed.log`); with the
  SIMD path's fp32 witness it holds (`logs/samba/deploy-gemma-simd-fp32.log`). Gemma 2's twin brings up (below).
- [ ] `deploy_gemma_lock::every_real_width_layer_kind_lowers_and_certifies`: Gemma 3 1B, 4B, 12B and 27B (27B's
  1/√168 scale) lower and certify (receipt: `logs/samba/red-deploy-gemma-handed.log`).
- [ ] `deploy_gemma_lock::every_cell_has_a_modeled_row`: fails today.
- [x] `deploy_gemma_lock::gemmas_tiny_bundle_equals_the_vm_and_the_references`,
  `…::gemma_2s_tiny_bundle_equals_the_vm_and_the_references`, `…::every_gemma_size_lowers_and_certifies`,
  `…::every_gemma2_size_lowers_and_certifies`: Gemma's and Gemma 2's twins from their directories (transformers, the
  reference and the VM agree, and the bundle brings up on the SN50, a capped head held to the bound of its raw logits,
  the cap being 1-Lipschitz) and every size's layer kinds; failed: the architectures unread
  (`logs/samba/red-gemma-head.log`) (receipt: `logs/samba/green-gemma-3.log`).
- [x] `deploy_gemma_lock::every_size_is_its_published_configuration`: Gemma 2B and 7B (1 and 1.1), Gemma 2 2B, 9B and
  27B, and Gemma 3 1B and 27B as their published configurations state them, caps and score scales included (receipt:
  `logs/samba/green-gemma-3.log`).

**Edge cases.**
- [x] Every size the family publishes, each a configuration of the same lock: Gemma 2B and 7B (1 and 1.1), Gemma 2 2B,
  9B and 27B, Gemma 3 1B, 4B, 12B and 27B (`deploy_gemma_lock::every_size_is_its_published_configuration`, receipt:
  `logs/samba/green-multimodal-3.log`).
- [ ] The family's quantized releases at their own bits.

**Implementation.** The generic path end to end; no code specific to the family outside its spec arm and its
features' tickets.

**Success.**
- [ ] Certified, equal, lowered and priced (receipt: the lock and the cell ledger).

**Mutants to kill.**
- [ ] One layer bound to another layer's weights.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test deploy_gemma_lock
```

**Done when.** The family runs on the RDU and knows how fast.

#### DEPLOY-KIMI-GLM · Kimi K2 and GLM

| Field | Value |
|---|---|
| Phase | deployment |
| Labels | `infrastructure` `rdu` |
| Depends on | ARCH-MLA, ARCH-ROUTING, ARCH-SPARSE, MULTI-2, INTAKE-6 |
| Touches | the family's deployment lock (new) |
| Status | open |

**Goal.** Deploy the family on SN50 and SN40L: Tier C mixtures with latent attention, group-limited routing and indexed sparse attention. The bundle is certified, its tiny configuration equals the
VM and the family's reference implementations from the files on disk, its real-width layers lower, its parallel plan
is the time model's, and its D, L, SPEC and TTFT cells are modeled with provenance beside their Copy bars.

**Read first.**
- Section 6.1's row for the family.
- `crates/logicaffeine_tests/tests/emit_rdu_e2e.rs`: how a bundle on disk is run against the VM.

**RED tests.**
- [ ] `deploy_kimi_glm_lock::the_tiny_bundle_equals_the_vm_and_the_references`: fails today: the family does not deploy.
  GLM-4.5's half passes, with every GLM-4.5 size lowered (`logs/samba/green-glm45.log`); Kimi K2's twin brings up
  (`deploy_kimi_glm_lock::kimi_k2s_tiny_bundle_equals_the_vm_and_the_references`, `logs/samba/green-kimi-3.log`), its
  vocabulary and FP8 weights read (INTAKE-2, INTAKE-3), and Kimi K2 lowers at real width
  (`deploy_kimi_glm_lock::every_kimi_k2_layer_kind_lowers_and_certifies`, `logs/samba/green-wide-routing.log`).
- [ ] `deploy_kimi_glm_lock::every_real_width_layer_kind_lowers_and_certifies`: fails today.
- [ ] `deploy_kimi_glm_lock::every_cell_has_a_modeled_row`: fails today.

**Edge cases.**
- [ ] Every size the family publishes, each a configuration of the same lock.
- [ ] The family's quantized releases at their own bits.

**Implementation.** The generic path end to end; no code specific to the family outside its spec arm and its
features' tickets.

**Success.**
- [ ] Certified, equal, lowered and priced (receipt: the lock and the cell ledger).

**Mutants to kill.**
- [ ] One layer bound to another layer's weights.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test deploy_kimi_glm_lock
```

**Done when.** The family runs on the RDU and knows how fast.

#### DEPLOY-HYBRID-SSM · Jamba, Falcon-H1, Nemotron-H and Mamba2

| Field | Value |
|---|---|
| Phase | deployment |
| Labels | `infrastructure` `rdu` |
| Depends on | ARCH-SSD, ARCH-SELECTIVE-SCAN, INTAKE-6 |
| Touches | the family's deployment lock (new) |
| Status | open |

**Goal.** Deploy the family on SN50 and SN40L: Tier C hybrids of attention and state-space layers. The bundle is certified, its tiny configuration equals the
VM and the family's reference implementations from the files on disk, its real-width layers lower, its parallel plan
is the time model's, and its D, L, SPEC and TTFT cells are modeled with provenance beside their Copy bars.

**Read first.**
- Section 6.1's row for the family.
- `crates/logicaffeine_tests/tests/emit_rdu_e2e.rs`: how a bundle on disk is run against the VM.

**RED tests.**
- [ ] `deploy_hybrid_ssm_lock::the_tiny_bundle_equals_the_vm_and_the_references`: fails today: the family does not deploy.
- [ ] `deploy_hybrid_ssm_lock::every_real_width_layer_kind_lowers_and_certifies`: fails today.
- [ ] `deploy_hybrid_ssm_lock::every_cell_has_a_modeled_row`: fails today.

**Edge cases.**
- [ ] Every size the family publishes, each a configuration of the same lock.
- [ ] The family's quantized releases at their own bits.

**Implementation.** The generic path end to end; no code specific to the family outside its spec arm and its
features' tickets.

**Success.**
- [ ] Certified, equal, lowered and priced (receipt: the lock and the cell ledger).

**Mutants to kill.**
- [ ] One layer bound to another layer's weights.

**Verify.**

```
cargo nextest run -p logicaffeine-tests --test deploy_hybrid_ssm_lock
```

**Done when.** The family runs on the RDU and knows how fast.
---

## 12. Appendices

### 12.1 Locks every ticket keeps green

Beside the fusion book's section 13.1 locks, every ticket here keeps these green:
- `samba_book_lock`, `public_prose_lock`, `frozen_reds_lock`;
- `one_program_lock`, `one_ir_identity_lock`, `spatial_one_ir_lock`, `numerics_contract_lock`, `shim_lock`;
- `qwen38_decode_bundle_lock`, `qwen38_real_dims_lock`, `qwen38_tensor_parallel_lock`, `emit_rdu_e2e`;
- `qwen38_fabric_segments_lock`, `plasticine_sim_lock`, `plasticine_report_lock`;
- `model_conformance_lock`, `model_golden_lock`, `model_reference_lock`, `bringup_lock`;
- once they exist: `samba_cells_lock`, `samba_climb_lock`, the exam suites, `spatial_time_model_lock`.

### 12.2 Glossary

- **RDU**: SambaNova's reconfigurable dataflow unit; SN40L and SN50 are its recent generations.
- **PCU, PMU, AGCU**: a pattern compute unit, a pattern memory unit, an address generation and coalescing unit.
- **Section**: a part of a composed spatial program that runs as one placed unit of work; a phase is a set of
  sections that run together.
- **Kind program**: the one lowered program a deployment runs for every layer of a kind; layers bind their own weights.
- **Deployment**: a model's kind programs, embedding, head and manifest, emitted by `largo emit rdu --model`.
- **Cell**: a model on a chip at a socket count, bits and regime; it holds a Copy bar and our rows.
- **Copy bar, Beyond bar**: the published number to match on a cell; the margin by which we pass it.
- **MBU**: model bandwidth utilization, achieved model bytes over peak bandwidth [HC26].
- **Share of the roofline**: decode throughput over `samples × HBM_bw / (weights + KV × samples)` [DIAYN26].
- **KernelLooping, BatchStreaming, ScheduleOffloading**: SambaNova's techniques of section 5.1 [KL24] [DIAYN26].
- **The kit**: the 2019 Stanford Plasticine evaluation kit [PTUT] and its tracer [STUBBED].
- **Handed RED**: a test written ahead of its implementation, compiled only under the `handed-reds` feature, frozen by
  `frozen_reds_lock`.
- **Provenance**: published, simulated, modeled or measured (section 1.2).

### 12.3 Bibliography

Every entry is pinned in `benchmarks/samba_papers.tsv`: its venue and year, a stable locator, the capture under
`external/papers/` (`scripts/accel/samba-papers.py`) with its SHA-256 and capture date, and the text
`samba_papers_lock` searches for every number this book attributes to it. A page that changes after capture is cited
as captured.

- [KL24] D. Koeplinger, D. Gandhi, P. Nandkar, N. Sheeley, M. Musaddiq, L. Zhang, R. Goodbar, M. Shaffer, H. Wang,
  A. Wang, M. Wang, R. Prabhakar. Kernel Looping: Eliminating Synchronization Boundaries for Peak Inference
  Performance. arXiv:2410.23668, 2024.
- [DIAYN26] D. Gandhi, P. Nandkar, D. Koeplinger, N. Farahini, R. Tsoupidi, S. Rydh, M. Musaddiq, T. Zhao, R. Goodbar,
  N. Sheeley, L. Zhang, M. Shaffer, J. Long, H. Wang, A. Wang, A. Sabnis, J. Brot, Y. Du, H. Zeffer, M. Wang,
  R. Prabhakar. Dataflow Is All You Need. Proceedings of Machine Learning and Systems 8 (MLSys 2026), Industry Track,
  2026.
- [SN40L24] R. Prabhakar et al. SambaNova SN40L: Scaling the AI Memory Wall with Dataflow and Composition of Experts.
  MICRO 2024, pp. 1353–1366, 2024 (preprint arXiv:2405.07518).
- [HC24] R. Prabhakar. SambaNova SN40L RDU: Breaking the Barrier of Trillion+ Parameter Scale Gen AI Computing. Hot
  Chips 36 (HCS 2024), 2024.
- [HC26] R. Prabhakar. Hot Chips 2026: SN50 RDU Dataflow at Scale. SambaNova blog, 2026, captured 2026-10-29.
- [STH26] ServeTheHome. SambaNova's SN50 RDU for AI at Hot Chips 2026. 2026, captured 2026-10-29.
- [SN50PR] SambaNova. SambaNova Unveils Fastest Chip for Agentic AI, Collaborates with Intel, and Raises $350M+. Press
  release, 2026-02-24, captured 2026-10-29.
- [SN50MM] SambaNova. SN50 Runs the Fastest MiniMax Speeds in the World. SambaNova blog, 2026, captured 2026-10-29.
- [SN50WEB] SambaNova. RDU: Next-Gen AI Chip for Inference at Scale, the SN50 and SN40 RDU hardware specifications.
  sambanova.ai product page, 2026, captured 2026-10-30.
- [SN50DS] SambaNova. SambaNova SN50 Reconfigurable Dataflow Unit. Datasheet, 2026, captured 2026-10-30.
- [AA26] Artificial Analysis. SambaNova provider page. 2026, captured 2026-10-29.
- [SPEC25] D. Gandhi, P. Nandkar, N. Farahini, H. Zeffer, J. Long, S. Rydh, M. Musaddiq, T. Zhao, J. Brot, R. Goodbar,
  Y. Du, M. Wang, R. Prabhakar. Speculative Decoding on the SN40L Reconfigurable Dataflow Unit. IEEE Micro 45(5),
  pp. 56–66, 2025.
- [ALCF] Argonne Leadership Computing Facility. AI Testbed user guide: the SambaNova SN30 guide as last published
  (`argonne-lcf/user-guides` at dc72a72, removed 2026-04-09) and the SN40L inference page now in its place (ef0105d),
  2026.
- [SNSW24] SambaNova Systems. DataScale: Software Overview. Training workshop slides, May 2024, hosted by ALCF;
  2024.
- [SFRN] SambaNova Systems. SambaFlow software release notes. SambaNova documentation, archived 2026-01-12; 2026.
- [ISCA17] R. Prabhakar, Y. Zhang, D. Koeplinger, M. Feldman, T. Zhao, S. Hadjis, A. Pedram, C. Kozyrakis,
  K. Olukotun. Plasticine: A Reconfigurable Architecture for Parallel Patterns. ISCA 2017, pp. 389–402, 2017.
- [PLDI18] D. Koeplinger, M. Feldman, R. Prabhakar, Y. Zhang, S. Hadjis, R. Fiszel, T. Zhao, L. Nardi, A. Pedram,
  C. Kozyrakis, K. Olukotun. Spatial: A Language and Compiler for Application Accelerators. PLDI 2018, pp. 296–311,
  2018.
- [SARA21] Y. Zhang, N. Zhang, T. Zhao, M. Vilim, M. Shahbaz, K. Olukotun. SARA: Scaling a Reconfigurable Dataflow
  Accelerator. ISCA 2021, pp. 1041–1054, 2021.
- [CAPSTAN21] A. Rucker, M. Vilim, T. Zhao, Y. Zhang, R. Prabhakar, K. Olukotun. Capstan: A Vector RDA for Sparsity.
  MICRO 2021, pp. 1022–1035, 2021.
- [GORGON] M. Vilim, A. Rucker, Y. Zhang, S. Liu, K. Olukotun. Gorgon: Accelerating Machine Learning from Relational
  Data. ISCA 2020, pp. 309–321, 2020.
- [AUROCHS] M. Vilim, A. Rucker, K. Olukotun. Aurochs: An Architecture for Dataflow Threads. ISCA 2021, pp. 402–415,
  2021.
- [REVET] A. C. Rucker, S. Sundram, C. Smith, M. Vilim, R. Prabhakar, F. Kjølstad, K. Olukotun. Revet: A Language and
  Compiler for Dataflow Threads. HPCA 2024, 2024 (preprint arXiv:2302.06124, 2023).
- [DAM24] N. Zhang, R. Lacouture, G. Sohn, P. Mure, Q. Zhang, F. Kjolstad, K. Olukotun. The Dataflow Abstract Machine
  Simulator Framework. ISCA 2024, pp. 532–547, 2024.
- [STEP] G. Sohn, G. Zhang, K. Hossfeld, J. Kim, N. Sobotka, N. Zhang, O. Hsu, K. Olukotun. Streaming Tensor Programs:
  A Streaming Abstraction for Dynamic Parallelism. ASPLOS 2026, pp. 1912–1932, 2026 (preprint arXiv:2511.07776, 2025).
- [LOOM26] H. Wang, W. Li, Z. Bai, T. Mitra. Schedules Are Solvable Symbols: Tuning-Free Compilation of Tile Programs on
  Dataflow Architectures. arXiv:2609.29219, 2026.
- [DABENCH26] Z. Hu et al. DABench-LLM: Standardized and In-Depth Benchmarking of Post-Moore Dataflow AI Accelerators
  for LLMs. arXiv:2601.19904, 2026.
- [PTUT] Stanford Plasticine Project. PlasticineTutorial (the evaluation kit: Model Analyzer, tracer, Tungsten
  estimator), github.com/stanford-ppl/PlasticineTutorial at b8ee92be49ccc1f1fd2ddfa3358930c7ec884b20, 2019-08-05.
- [STUBBED] Stanford Plasticine Project. stubbed 0.1.1, a library for obtaining runtime traces (PyPI), 2019, sdist
  SHA-256 4f7ba5dcc7daa3148c2782ab4bd6124fe492791d0b2551955663205b28596e78; local copy
  `external/plasticine-kit/stubbed-0.1.1`.
- [HFQ4] Qwen. Qwen/Qwen3.8-Flash-Next, config.json, huggingface.co, revision de4b8e4d, 2026.
- [TFQ4] Hugging Face transformers. src/transformers/models/qwen4_exp/modeling_qwen4_exp.py and configuration_qwen4_exp.py
  at 5c65f7c, 2026.
- [LCPPQ4] ggml-org/llama.cpp pull request 27742, model: add Qwen3.8-Flash-Next (qwen4exp), merged 2026-08-27; local
  copy `external/competitors/llama.cpp/src/models/qwen4exp.cpp`.
- [VLLMQ4] vLLM, vllm/models/qwen4_exp, 2026; local copy `external/competitors/vllm/vllm/models/qwen4_exp`.
- [MLXQ4] Rocktalk-Holdings, mlx-qwen4exp, README (table geometry, readout scale, draft acceptance) at 2a4346c, 2026.
- [TINYQ4] yujiepan, qwen3.8-flash-next-tiny-random, huggingface.co, 2026, captured 2026-10-29.

### 12.4 This book's lock

`samba_book_lock` (`crates/logicaffeine_tests/tests/samba_book_lock.rs`) checks, on every run:
- every ticket body has one board index line that agrees with it, and carries the whole template;
- every ticket is in one stream that section 1.3 defines, and every stream holds a ticket;
- no file is owned by two streams, and no owned directory holds another stream's file;
- every dependency is a ticket on this board or on the fusion book's, and the dependencies close no cycle;
- every fusion-book ticket the book names is on that book's board, and every ticket of this board the book names is on
  this board;
- every exam test names a lock test and a ticket on the board, and every exam family holds at least three tests;
- every Copy bar cites the bibliography, and every bibliography entry is cited;
- every repository path the book cites exists;
- the preface's counts are the book's own.

`SAMBA_BOOK` points the lock at a mutated copy for a mutation run.

```
cargo nextest run -p logicaffeine-tests --test samba_book_lock
```
