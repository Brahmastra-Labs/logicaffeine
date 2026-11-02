# One cycle beats a gigahertz — i9 vs LOGOS soft-core vs LOGOS fabric

Backing detail for the speed and power comparison. All figures are the **decision kernel** (an HFT
microprice market-making decision + fat-finger clamp + position-risk gate), measured the same way on all three
engines, read from the committed Sky130 signoff artifacts (`decision_fabric_power.json` → `honest_comparison` +
`signoff_clean`, `hft_latency.json` → `x86_baseline`).

## The thesis

A sequential machine — Intel's CPU **or** our own soft-core — only gets fast by cranking its **clock**, and clock
costs power (dynamic energy ∝ frequency × voltage²). The spatial fabric does the whole decision in **one clock
cycle**, so it reaches gigahertz-CPU latency at a **crappy clock** — fast *and* low-power at once, which a clocked
machine can never be. The fabric buys speed with **space** (parallel tiles), not **time** (a fast clock).

## Time per decision = cycles ÷ clock

| Engine | Cycles / decision | At 66.7 MHz (a crappy clock) | At its own fmax | Power |
|---|---:|---:|---:|---:|
| **Intel i9-14900K** | dozens (out-of-order) | — (runs at 5.7 GHz) | **16.6 ns** @ 5.7 GHz | ~12 W / core |
| **LOGOS soft-core** | 82 | 1,229 ns | 824 ns @ 99.5 MHz | 30.8 mW |
| **LOGOS fabric** | **1** | **15.0 ns** | **4.35 ns** @ 230 MHz | 1.19 mW (0.886 measured) |

The fabric at a **crappy 66.7 MHz already answers in 15 ns** — the same latency class as a **5.7 GHz** i9, at an
**~85× slower clock** and **~1/10,000th the power**. The soft-core at that same 66.7 MHz needs 1,229 ns; to reach
the i9's latency it would have to be clocked to ~4.9 GHz (physically impossible on this process, and power-
prohibitive). That is the whole point: **the clock is the sequential machine's only lever; the fabric barely
needs it.**

## What clock each engine needs to reach the i9 latency class (~16.6 ns)

| Engine | Clock required | vs the fabric |
|---|---:|---:|
| **LOGOS fabric** (1 cycle) | **~60 MHz** | — |
| **LOGOS soft-core** (82 cycles) | ~4.9 GHz | **~82× higher clock** |

## Same-kernel reductions, measured (fabric vs the soft-core running the identical kernel)

| Metric | Reduction |
|---|---:|
| Cycles per decision | **82× fewer** (82 → 1) |
| Wall-clock (cycles ÷ clock, incl. 2.3× higher fmax) | **~190× faster** (824 ns → 4.35 ns) |
| Power draw | **16–26× less** (30.8 mW → 1.19–1.87 mW) |
| Energy per decision | **~1,350–2,100× less** (37.9 nJ → 28–18 pJ) |
| Silicon area | **16× smaller** (0.428 → 0.027 mm²) |

## The full pipeline — measured cycles (the whole trade, not just the decision)

The measured numbers above are the **decision kernel** (one stage, a branch-free combinational select-DAG that
settles in 1 cycle). The **whole** tick-to-trade pipeline (ITCH-lite decode → price-ladder book → best-of-book
scan → microprice → risk) also lowers on the fabric and runs **equal to the VM** — end to end, **sequenced** —
and it does **not** collapse to one cycle, because it is loop-dominated.

`lowering_hft_pipeline_lock.rs` runs the whole pipeline on the whole-program fabric via `eval_wprog_config_counted`
(proven ≡VM; the English program also equals the Python reference `hft_model.py` = 7401040). **Measured end-to-end,
not composed:**

| Per market message (full pipeline) | Cycles | Energy | Latency |
|---|---:|---:|---:|
| **Intel i9-14900K** (measured) | dozens (OoO) | ~199 nJ | **16.6 ns** |
| **LOGOS soft-core** (measured) | 866 | 400 nJ | ~9.6 µs @ 90 MHz |
| **LOGOS fabric** (measured ≡VM) | **98** | **~1.75 nJ** | ~0.4 µs @ 230 MHz |

So full-pipeline to full-pipeline: **~8.8× fewer cycles than the soft-core**, and **~114× less energy than an i9
core** (~230× vs the soft-core) — but at these clocks it is **slower** than the i9 in latency (~0.4 µs vs 16.6 ns),
because the pipeline is loop-dominated. **The "one cycle beats a gigahertz" story is the decision kernel; the full
pipeline wins on energy, not latency.** Fabric power is anchored to the *measured* decision-fabric block (1.19 mW,
conservative); a full signoff-power tape of the composed pipeline is the next milestone.

**Correction note.** An earlier draft projected ~700–1,100× (and ~0.2–0.3 nJ) by carrying the decision kernel's
per-op energy reduction across the whole pipeline. That was wrong: the loop-dominated pipeline is 98 cycles/message
(measured), not ~1, so the honest figure is ~114×, not ~1,000×.

## The i9 reality check (honest)

A 5.7 GHz out-of-order core wins **raw median latency** (16.6 ns on the full tick-to-trade pipeline) — but it is a
**general** machine that spends most of its energy fetching/decoding/scheduling instructions, and it **jitters**:
its tail blows out to **4,120 ns (248× the median)** and worsens under memory load. The fabric is **deterministic**
— identical cycle count every run, zero jitter — and spends energy only on the answer. In HFT, the tail is where
the money leaks.

## Honest scope

- **The fabric runs any LOGOS program** as bytecode streamed over the wire — proven ≡VM (all 115 ISA ops as
  tiles, whole-program census 35/35 benchmarks) and **live on a Nexys A7 this session** (loops, recursion,
  collections, arithmetic). What varies is *how* a program runs:
  - **Bounded, branch-free kernels** (the decision, `isSafe`, `siftDown`, …) lower to a **combinational**
    select-DAG that settles in **one clock edge** — this is the source of the 1-cycle / 18-pJ / 1.19-mW figures.
  - **Programs with data-dependent loops/recursion** cannot physically collapse to one clock edge (unbounded
    work ≠ a single settle), so they run on the **same fabric, sequenced** (reused across iterations). Still no
    fetch/decode/register-file per op — just many cycles instead of one.
- The i9's 16.6 ns is the **full** tick-to-trade pipeline. Its fabric form = the decision select-DAG composed
  with a divider (microprice) + loop-fabric (book-keeping) — every one an already-proven fabric mechanism, but
  not yet taped out and signoff-power-measured as one block. It would only widen the advantage.
- Earlier drafts of this file compared the fabric's decision against the soft-core's **full pipeline** (866
  cycles → "866× / 22,000×"). That is apples-to-oranges and is **superseded** by the same-kernel basis above
  (82 cycles → 82× / ~1,350–2,100×), per the artifact's own `honest_comparison` correction.
- Power is **Sky130 ASIC** (OpenSTA signoff, VCD-cross-checked), not FPGA. The Artix-7 is where these designs are
  **functionally proven on real hardware** this session; FPGA per-gate power is far higher, so it is not the
  power story.

## Provenance

- **Fabric** — `decision_hft`, OpenLane Sky130 RTL-to-GDSII, signoff clean (DRC/LVS/XOR/antenna/routing = 0),
  post-route parasitic-extracted OpenSTA `report_power`; 1.19 mW default-activity (conservative), **0.886 mW
  VCD-measured** under a real market-input workload; proven ≡ the LOGOS VM (lock + iverilog, 256 vectors).
- **Soft-core** — Sky130 signoff of the same core family; decision-kernel cycle count (82) from cycle-accurate
  RTL (`tb_hft.v`: 187 inline − 105 marshaling).
- **i9-14900K** — tight `-O2` C, identical pipeline, per-message `rdtscp`, 120k samples on the bench box;
  ~11–12 W single-core active by `turbostat`.
