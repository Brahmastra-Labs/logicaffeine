# LOGOS tick-to-trade — deterministic, English-verified HFT on silicon

A complete, runnable demo: a market-making strategy written in **plain English**, compiled to a chip,
proven to do exactly what the English says, and executed with **zero run-to-run jitter**. Hand this to a
quant; they can read the strategy and run it.

## The 30-second version

```bash
scripts/demo-hft.sh            # runs the whole thing: strategy -> silicon (sim) -> x86 jitter contrast
```

You'll see the ITCH-lite market-data feed decoded on-chip, the order book built, the microprice
market-making decisions, the pre-trade risk gate, and the order stream — plus the **per-message on-chip
latency (deterministic)** next to a tight C loop on this x86 whose latency has a **fat jitter tail**.

## What it is

The pipeline (`programs/hft_pipeline.lg`, written in English):

```
NASDAQ ITCH-lite feed  ->  decode each message on-chip
                       ->  price-ladder order book (add / delete / execute)
                       ->  microprice market-making signal (size-weighted, in cents)
                       ->  pre-trade risk (fat-finger cap + position limit, SEC 15c3-5 in spirit)
                       ->  order (side / price / size)
```

It runs on the LOGOS full soft-core. Every order is **bit-exact** to the language reference, so the
silicon provably equals the spec — there is no separate RTL to get wrong.

## Run it on real hardware (Nexys A7-100T, over the USB port)

```bash
sudo scripts/run-on-nexys-hft.sh --build          # builds the bitstream once, programs the board,
                                                  # streams the pipeline over the port, reads back orders
```

`nexys-host.py` frames the program bytes and sends them over the serial port at 115200 baud (2-byte
length; `--words16`); the FPGA loads, runs, and returns the order stream + the on-chip cycle count. This
is the same wire protocol whether you drive it from this script or your own Python.

## The numbers (see `hft_latency.json`)

| | LOGOS silicon | Tight −O2 C on x86 |
|---|---|---|
| Per-message latency | **866 cycles**, every message | p50 ~85 ns |
| Run-to-run jitter | **0** | tail to **~1000×+ the median** |
| Under load | unchanged | tail fattens |

Absolute latency, honestly tiered from the measured 866 cycles: 138 µs @6.25 MHz (shipped FPGA) →
9.6 µs @90 MHz (Sky130 projection) → **866 ns @1 GHz ASIC**. A GHz CPU wins the median; it **leaks in the
tail**, which is where HFT money leaks and which our deterministic silicon does not have.

## Files

| File | What |
|------|------|
| `programs/hft_pipeline.lg` | the strategy, in English (the thing you edit) |
| `scripts/hft_model.py` | the reference semantics + the x86 baseline generator |
| `scripts/hft_baseline.c` | the tight-C x86 baseline (per-message rdtscp timing) |
| `scripts/hft_gen.py` | emits the `.lg` for any feed (one source of truth) |
| `scripts/demo-hft.py` / `demo-hft.sh` | the harness + narrated runner (sim + contrast) |
| `scripts/run-on-nexys-hft.sh` | build bitstream + run on the real board over the port |
| `nexys_soc_hft.v` / `nexys_top_hft.v` | the board SoC (512-word imem + 2-byte UART loader) |
| `tb_hft.v` / `tb_hft_uart.v` | cycle-accurate RTL testbenches (direct-imem / over-UART) |
| `hft_latency.json` | the measured artifact |

## How it's proven

- **Silicon ≡ reference, bit-exact** — `tb_hft.v` / `tb_hft_uart.v` under iverilog, on the demo feed and
  random feeds.
- **Correctness to absurdity** — 150-feed (Python) + 120-feed (Rust) differential fuzz vs an independent
  reference.
- **CI lock** — `crates/logicaffeine_tests/tests/hft_determinism_lock.rs` (5/5 green): the committed
  strategy produces the exact golden order stream, and compiles to deterministic full-core silicon words.

## Change the strategy

Edit `programs/hft_pipeline.lg` (it's English), or edit `scripts/hft_model.py` and run
`python3 scripts/hft_gen.py` to regenerate. Re-run `scripts/demo-hft.sh` to see it on silicon.
