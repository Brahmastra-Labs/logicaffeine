<!-- SPDX-FileCopyrightText: 2026 DatacenterDotDev, Inc.
     SPDX-License-Identifier: MIT -->
# LOGOS bring-up — `wpe_user_project`

Verification for the macro this submission contains. The scaffold's other `dv/` tests drive
`user_proj_example`, which is **not** on this die.

## Run it

From this directory:

    iverilog -g2012 -o /tmp/tb.out tb_user_project_wrapper.v \
        ../../rtl/user_project_wrapper.v ../../rtl/wpe_user_project.v
    vvp /tmp/tb.out        # expect: SHOW lines, three FRAME lines, PSRAM-VIOLATIONS 0, PASS

The bench drives the fabricated top cell, `user_project_wrapper`, through its pins only: the host's UART
on GPIO 8 and 9, the engine's clock on `user_clock2` (47.62 MHz), the management SoC's clock on `wb_clk_i`
(40 MHz) and its reset, an idle Wishbone, and a pin-accurate model of the APS6404L-3SQN on GPIO 10–15. Every PSRAM pin reaches the part 10.12 ns after the macro drives it
(the harness's slowest `io_out` path, 9.12 ns, and 1 ns of board), `io_oeb` 10.32 ns after, and the part's
data reaches `io_in` 5.55 ns after the part drives it. It fails the run on a pin driven the wrong way
(`PIN-DIRECTION`), a received byte overwritten before the engine takes it (`UART-LOST`), a PSRAM rule
broken at the part's pins (`PSRAM-VIOLATION`), or a result that differs from the model's (`FAIL`).

## The programs

`wire_caravel.hex` holds three program images, back to back, one byte per line; `lens_caravel.hex` their
lengths and `oracle_caravel.hex` the results the reference model computes for them:

1. text built, compared and shown: `Show item 2 of s`, `Show s equals "abc"`, `Show "n=" + 42`;
2. a weighted sum over 40 weights streamed into the heap ahead of the run — 41 heap words, more than the
   heap cache holds, so the weights come back from the part;
3. the same sum over the first 17 weights.

## The wire protocol

The host sends a program image — for each of the 24 configuration memories a 2-byte big-endian entry count
and its big-endian words, a 2-byte parameter count and its 8-byte parameters, an optional `0x48` heap
preload (a 2-byte word count and its 8-byte words), and the `0x5A` go byte — at 115200 baud, 8N1. The
chip answers with a `0xA7` record per `Show` executed, then the frame: `0xA5`, the 64-bit result
most-significant byte first, the 32-bit run-cycle count, and `0x5A`.
