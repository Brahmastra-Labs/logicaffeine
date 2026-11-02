<!-- SPDX-FileCopyrightText: 2026 DatacenterDotDev, Inc.
     SPDX-License-Identifier: MIT -->
# LOGOS Caravel user project — Infinichip deterministic DRAM command player

This is the Caravel user area for the LOGOS project. The user macro `dram_user_project` places our
certified DRAM command-stream executor (`dram_executor`) behind a standard Wishbone target, with the
issued command mirrored onto the logic-analyzer bus.

## What is inside

- `dram_executor` — an in-order, delta-encoded DRAM command-stream executor. Its countdown
  arithmetic and issue timing are proven exact by our own certified SAT engine, and its RTL is
  proven bit-identical to the reference VM under iverilog.
- `dram_executor_wb` — the Wishbone front end (a standard bus target): the management SoC writes a plan word
  plus a launch strobe, and the FSM issues exactly that command.
- `dram_user_project` — the full Caravel user interface (Wishbone + 128-bit logic analyzer +
  38 GPIO + user_irq + user_clock2) wrapped around the above, so every wrapper pin connects 1:1.

Every user GPIO is configured as an input (`io_oeb` is tied all-ones); the design drives none.

## Signoff

Hardened with OpenLane v1 on the open sky130A PDK. The macro and the wrapper are each clean on every
physical gate: Magic DRC 0, Netgen LVS a unique clean match, KLayout-vs-Magic XOR 0, detailed routing 0,
antenna 0, the golden efabless `sky130A_mr` shuttle DRC 0 with zeroarea 0, and setup and hold met at
every corner (ss/tt/ff × min/nom/max RC).

Provenance is locked: `sky130_provenance_lock` re-derives every recorded signoff number from its
OpenLane run directory, so a claim whose run no longer exists fails the suite.
