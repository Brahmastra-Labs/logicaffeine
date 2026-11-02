<!-- SPDX-FileCopyrightText: 2026 DatacenterDotDev, Inc.
     SPDX-License-Identifier: MIT -->
# LOGOS bring-up — `dram_user_project`

Verification for the macro this submission actually contains. The scaffold's other `dv/` tests
drive `user_proj_example`, which is **not** on this die.

## Run it

    iverilog -g2012 -o /tmp/tb.out \
        verilog/dv/logos_bringup/tb_user_project_wrapper.v \
        verilog/rtl/user_project_wrapper.v \
        verilog/rtl/dram_user_project.v
    vvp /tmp/tb.out        # expect: PASS: user_project_wrapper bring-up

Add `-DGL` and point at `verilog/gl/` to run the same test against the gate-level netlist.

## Register map (Wishbone base `0x3000_0000`)

| offset | dir | meaning |
|---|---|---|
| `0x00` | W | `cmd_word[31:0]`  — row |
| `0x04` | W | `cmd_word[63:32]` — `{delta[23:0], op[2:0], bank[4:0]}` |
| `0x08` | W | bit 0 = `cmd_valid`, launches the command |
| `0x0c` | R | `{issued, 15'b0, issue_count[7:0], bank[4:0], op[2:0]}` |
| `0x10` | R | `row[31:0]` of the last issued command |

`la_data_out` mirrors the same state: `{issued, 15'b0, issue_count, bank, op, row}`.

## Handshake — use the counter, never the `issued` bit

`issued` is sticky: it is set by the first command and cleared only by reset, so it cannot tell
you that *your* command went out. Poll `issue_count` instead, which ticks once per issued
command and is unambiguous even for two identical commands back to back:

    n = read(0x0c) >> 8 & 0xff;
    write(0x00, row);
    write(0x04, (delta << 8) | (op << 5) | bank);
    write(0x08, 1);
    while (((read(0x0c) >> 8) & 0xff) == n) { /* the executor is counting down `delta` */ }
    op_bank = read(0x0c);  row_back = read(0x10);

The counter wraps at 256; only inequality is ever tested, so wrapping is harmless under the
one-command-at-a-time protocol above.

## GPIO

The macro ties `io_oeb` high and drives `io_out` low — every user pad is an input and the user
area drives none of them. `verilog/rtl/user_defines.v` is written to match. The chip is observed
through the management SoC (Wishbone + logic analyzer), not through the pads.