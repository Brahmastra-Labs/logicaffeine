<!-- SPDX-FileCopyrightText: 2026 DatacenterDotDev, Inc.
     SPDX-License-Identifier: MIT -->
# LOGOS Caravel user project — the whole-program engine

The user macro `wpe_user_project` carries the LOGOS whole-program engine. A host streams a compiled LOGOS
program to the chip over a UART; the chip loads the program's configuration — its blocks, cells, write
tables, live-ins and constants — and runs it, streaming each `Show` record and then the result frame
back over the same UART. The program is data, not mask: one die runs any program that fits its profile.

## What is inside

- `wpe_soc_caravel` — the engine: a UART loader, the configuration walk, a 64-bit datapath with integer,
  decimal, text and IEEE-754 double cells, and the frame stack for calls.
- Three write-back caches (eight lines of two 64-bit words each) in front of an external QSPI PSRAM, for
  the heap, the call-frame stack and the configuration image. Register file, cell values and the block,
  write and live-in tables are on die.
- `psram_edge` — the QSPI controller for an AP Memory APS6404L-3SQN (8 MiB, 3.3 V): power-up, reset,
  QPI entry, and bursts cut to what tCEM allows at the slowest clock the edge is timed for.
- `wpe_user_project` — the full Caravel user interface around the above, so every wrapper pin connects
  1:1. The Wishbone target acknowledges every access and reads zero; the logic analyzer and the user
  interrupts are unused.

## What it runs

Of the 35 programs of the LOGOS benchmark suite, the chip runs 24 of the 35 programs, each exactly as the
reference VM does: `loop_sum`, `fib_iterative`, `collatz`, `primes`, `vm`, `fib`, `gcd`, `ackermann`,
`binary_trees`, `nqueens`, `array_fill`, `array_reverse`, `prefix_sum`, `bubble_sort`, `counting_sort`,
`histogram`, `matrix_mult`, `sieve`, `coins`, `graph_bfs`, `pi_leibniz`, `mandelbrot`, `two_sum` and
`collect`. The other eleven need a heap larger than the 32,768 words the part's heap region holds, or more
than 64 values in one block. A program that does not fit is refused, never answered wrong: the host checks
every table against the profile before it sends the program, and the two demands that depend on the data
stop the run on die — a call past 32 frames ends the result frame with `0xE1`, a heap access past the region
with `0xE2`.

## Pins and clock

| GPIO | signal | power-on mode |
|---|---|---|
| 8 | UART receive (host to chip), 115200 baud, 8N1 | input |
| 9 | UART transmit | output |
| 10 | PSRAM CE# | output |
| 11 | PSRAM SCLK | output |
| 12–15 | PSRAM SIO[0..3] | bidirectional |

Every other user GPIO is a no-pull input (`verilog/rtl/user_defines.v`). The engine runs on `user_clock2` at
47.62 MHz (21 ns), the period the macro is hardened at; the UART's bit is 413 of its cycles,
0.09 % from 115200 baud.

Caravel's chip signoff times its core clock at 25 ns, and that clock is `wb_clk_i`, on which the management
SoC runs; only the Wishbone acknowledge runs on it here. `user_clock2` is the PLL's 90° phase through a
divider of its own (`caravel_clocking.v`), so the firmware sets the two apart in the housekeeping registers:
`user_clock2` is f(DCO) / `pll90_sel` and `wb_clk_i` is f(DCO) / `pll_sel` (register 0x11), with f(DCO) the
reference clock times `pll_div` (register 0x12). It sets them so `user_clock2` runs at 47.62 MHz or slower and
`wb_clk_i` at 40 MHz or less.

The engine's reset is `wb_rst_i`, or logic-analyzer probe 0 while the management SoC drives it, brought
into the engine's clock domain through two flops. At power-on the PLL is bypassed and `user_clock2` is the
pad clock; the firmware that switches it to the PLL then resets the engine on probe 0. It configures the
probe as an output from the CPU (`reg_la0_oenb = reg_la0_iena = 1`, as caravel_user_project's `la_test1`
configures its outputs), writes 1 to `reg_la0_data`, then 0.

## Board requirements

- An APS6404L-3SQN on GPIO 10–15, with the user I/O supply at 3.3 V.
- Each trace between a pad and the part at most 1 ns long (about 15 cm of FR-4).

A PSRAM read's round trip — SCLK out through the harness (at most 9.12 ns, efabless's characterization of
Caravel's `io_out` path), the board, the part's tACLK (5.5 ns), the board, and the harness back to
`io_in` (at most 4.55 ns) — is 21.17 ns, inside the 37 ns half SCLK period the edge samples on: two engine
cycles per half, a 74 ns SCLK.

## Signoff

Hardened with OpenLane v1 on the open sky130A PDK. The macro and the wrapper are each clean on every
physical gate: Magic DRC 0, Netgen LVS a unique clean match, KLayout-vs-Magic XOR 0, detailed routing 0,
antenna 0, the golden efabless `sky130A_mr` shuttle DRC 0 with zeroarea 0, and setup and hold met at
every corner (ss/tt/ff × min/nom/max RC).

Provenance is locked: `sky130_provenance_lock` re-derives every recorded signoff number from its
OpenLane run directory, so a claim whose run no longer exists fails the suite.

## Bring-up

`verilog/dv/logos_bringup/` runs programs on the fabricated top cell through the Caravel pins, against a
pin-accurate model of the part, with every PSRAM pin at the harness's slowest delay.
