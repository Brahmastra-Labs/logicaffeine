#!/usr/bin/env bash
# Synthesize the LOGOS reversible-ALU soft-core RTL to a gate-level netlist with yosys.
#
# The RTL is the datapath the campaign's `emit_reversible_alu_verilog` emits (a reversible
# counter — the RevAdd charge-recovery core); the netlist is a ripple-carry incrementer in
# XOR/AND gates + flip-flops. The EQUIVALENCE (netlist ≡ VM datapath q+1) is proven separately by
# OUR OWN certified SAT — see `codegen_sva::rtl` tests `our_certified_sat_proves_the_*` — not by
# yosys's internal SAT. Physical tape-out (place-and-route on a PDK, then a foundry) is downstream.
#
# Requires: yosys (apt install yosys).  Outputs land in benchmarks/silicon/.
set -euo pipefail
cd "$(dirname "$0")/../benchmarks/silicon"

if ! command -v yosys >/dev/null 2>&1; then
  echo "yosys not found — install it (apt install yosys) to (re)generate the netlist."
  echo "The checked-in benchmarks/silicon/rev_alu_netlist.v is the golden artifact."
  exit 0
fi

cat > rev_alu.v <<'EOF'
module rev_alu(input clk, output reg [3:0] q);
  initial q = 0;
  always @(posedge clk) q <= q + 4'd1;
endmodule
EOF

cat > synth.ys <<'EOF'
read_verilog rev_alu.v
hierarchy -check -top rev_alu
proc; opt; techmap; opt; clean
write_verilog -noattr rev_alu_netlist.v
stat
EOF

echo "== yosys synthesis: reversible-ALU soft-core RTL -> gate netlist =="
yosys -q synth.ys
echo "== netlist: benchmarks/silicon/rev_alu_netlist.v =="
cat rev_alu_netlist.v
echo "== equivalence (netlist == VM datapath q+1) is proven by OUR certified SAT in the rtl tests =="

# ── FPGA flow (Lattice ECP5 / ULX3S) — the same design, all the way to a loadable bitstream ──
if command -v nextpnr-ecp5 >/dev/null 2>&1; then
  echo "== FPGA: 32-bit reversible RevAdd datapath -> ECP5 bitstream =="
  cat > rev_alu32.v <<'RTL'
module rev_alu32(input clk, input [31:0] delta, output reg [31:0] q);
  initial q = 0;
  always @(posedge clk) q <= q + delta;
endmodule
RTL
  yosys -q -p "read_verilog rev_alu32.v; synth_ecp5 -top rev_alu32 -json rev_alu32.json"
  nextpnr-ecp5 --85k --json rev_alu32.json --lpf-allow-unconstrained --textcfg rev_alu32.config
  command -v ecppack >/dev/null 2>&1 && ecppack rev_alu32.config rev_alu32.bit && \
    echo "== bitstream benchmarks/silicon/rev_alu32.bit — loadable on a ULX3S with openFPGALoader =="
fi

# ── The full multi-op reversible ALU core (RevAdd/RevSub/RevXor/RevNeg/RevSwap) on the ECP5 ──
# (cwd is already benchmarks/silicon — the top-of-script cd)
if command -v nextpnr-ecp5 >/dev/null 2>&1 && [ -f rev_alu_core.v ]; then
  echo "== FPGA: multi-op reversible ALU core -> ECP5 (utilization + Fmax) =="
  yosys -q -p "read_verilog rev_alu_core.v; synth_ecp5 -top rev_alu_core -json rev_alu_core.json"
  nextpnr-ecp5 --85k --json rev_alu_core.json --lpf-allow-unconstrained --textcfg rev_alu_core.config 2>&1 \
    | grep -iE "TRELLIS_(COMB|FF|IO):|Max frequency"
fi

# ── The wire-driven reversible SOFT-CORE: a WIRE OPCODE BYTE runs the chip (decoder + ALU) on ECP5 ──
# rev_core = isa_core_decoder(opcode byte = binisa::opcode) -> sel -> rev_alu_core. This is the
# language's bytecode executed straight off the wire, in silicon.
if command -v nextpnr-ecp5 >/dev/null 2>&1 && [ -f rev_core.v ]; then
  echo "== FPGA: wire-opcode-driven reversible soft-core (decoder + ALU) -> ECP5 =="
  yosys -q -p "read_verilog isa_core_decoder.v rev_alu_core.v rev_core.v; synth_ecp5 -top rev_core -json rev_core.json"
  nextpnr-ecp5 --85k --json rev_core.json --lpf-allow-unconstrained --textcfg rev_core.config 2>&1 \
    | grep -iE "TRELLIS_(COMB|FF|IO):|Max frequency"
fi

# ── Functional simulation (iverilog): the reversible ALU computes AND reverses ──
if command -v iverilog >/dev/null 2>&1 && [ -f tb_rev_alu.v ]; then
  echo "== SIM: reversible ALU functional round-trip (iverilog) =="
  iverilog -o tb_rev_alu.vvp rev_alu_core.v tb_rev_alu.v && vvp tb_rev_alu.vvp | grep -E "PASS|FAIL"
fi

# ── Functional simulation: WIRE OPCODE BYTES drive the reversible soft-core (decoder + ALU) ──
if command -v iverilog >/dev/null 2>&1 && [ -f tb_rev_core.v ]; then
  echo "== SIM: wire opcode bytes execute on the reversible soft-core (iverilog) =="
  iverilog -o tb_rev_core.vvp isa_core_decoder.v rev_alu_core.v rev_core.v tb_rev_core.v \
    && vvp tb_rev_core.vvp | grep -E "PASS|FAIL"
fi

# ── The GENERAL INTEGER ALU (arith/bitwise/signed-compare), opcode-byte driven, proven ≡ VM by our SAT ──
if [ -f alu.v ]; then
  if command -v iverilog >/dev/null 2>&1 && [ -f tb_alu.v ]; then
    echo "== SIM: wire opcode bytes drive the general integer ALU (iverilog) =="
    iverilog -o tb_alu.vvp alu.v tb_alu.v && vvp tb_alu.vvp | grep -E "PASS|FAIL"
  fi
  if command -v nextpnr-ecp5 >/dev/null 2>&1; then
    echo "== FPGA: general integer ALU (32-bit) -> ECP5 (utilization) =="
    yosys -q -p "read_verilog alu.v; synth_ecp5 -top alu -json alu.json"
    nextpnr-ecp5 --85k --json alu.json --lpf-allow-unconstrained --textcfg alu.config 2>&1 \
      | grep -iE "TRELLIS_(COMB|FF|IO):|Max frequency"
  fi
fi

# ── The SCRATCHPAD SECDED(72,64) ECC: single-error-correct + double-error-detect, proven by our SAT ──
if [ -f secded.v ] && [ -f tb_secded.v ] && [ -f secded_vec.hex ]; then
  if command -v iverilog >/dev/null 2>&1; then
    echo "== SIM: scratchpad SECDED(72,64) encode/corrupt/decode vs the oracle (iverilog) =="
    iverilog -g2012 -o tb_secded.vvp secded.v tb_secded.v && vvp tb_secded.vvp | grep -E "PASS|FAIL"
  fi
  if command -v nextpnr-ecp5 >/dev/null 2>&1; then
    echo "== FPGA: scratchpad SECDED decoder/corrector -> ECP5 (utilization) =="
    yosys -q -p "read_verilog secded.v; synth_ecp5 -top secded_dec -json secded_dec.json"
    nextpnr-ecp5 --85k --json secded_dec.json --lpf-allow-unconstrained --textcfg secded_dec.config 2>&1 \
      | grep -iE "TRELLIS_(COMB|FF|IO):|Max frequency"
  fi
fi

# ── The seeded NoC ARBITER entropy core: HW SplitMix64 draws == the software scheduler bit-exact ──
if [ -f seeded_arbiter_prng.v ] && [ -f tb_seeded_arbiter.v ] && [ -f arbiter_seq.hex ]; then
  if command -v iverilog >/dev/null 2>&1; then
    echo "== SIM: seeded arbiter draws vs the software scheduler (iverilog) =="
    iverilog -g2012 -o tb_seeded_arbiter.vvp seeded_arbiter_prng.v tb_seeded_arbiter.v \
      && vvp tb_seeded_arbiter.vvp | grep -E "PASS|FAIL"
  fi
  if command -v nextpnr-ecp5 >/dev/null 2>&1; then
    echo "== FPGA: seeded arbiter entropy core -> ECP5 (utilization + Fmax) =="
    yosys -q -p "read_verilog seeded_arbiter_prng.v; synth_ecp5 -top seeded_arbiter_prng -json seeded_arbiter_prng.json"
    nextpnr-ecp5 --85k --json seeded_arbiter_prng.json --lpf-allow-unconstrained --textcfg seeded_arbiter_prng.config 2>&1 \
      | grep -iE "TRELLIS_(COMB|FF|IO):|Max frequency"
  fi
fi

# ── The KECCAK-f[1600] core: 24-cycle iterative permutation, proven bit-exact vs the native oracle ──
if [ -f keccak_core.v ] && [ -f tb_keccak_core.v ] && [ -f keccak_vec.hex ]; then
  if command -v iverilog >/dev/null 2>&1; then
    echo "== SIM: Keccak-f[1600] RTL core vs the native oracle (iverilog) =="
    iverilog -g2012 -o tb_keccak_core.vvp keccak_core.v tb_keccak_core.v \
      && vvp tb_keccak_core.vvp | grep -E "PASS|FAIL"
  fi
  if command -v nextpnr-ecp5 >/dev/null 2>&1 && [ -f keccak_top.v ]; then
    # The bare keccak_core has a 1600-bit parallel state port (too many IOs to PnR
    # standalone); keccak_top wraps it with a 64-bit streaming lane bus so it routes.
    echo "== FPGA: Keccak-f[1600] (streaming-I/O keccak_top) -> ECP5 (utilization + Fmax) =="
    yosys -q -p "read_verilog keccak_top.v; synth_ecp5 -top keccak_top -json keccak_top.json"
    nextpnr-ecp5 --85k --json keccak_top.json --lpf-allow-unconstrained --textcfg keccak_top.config 2>&1 \
      | grep -iE "TRELLIS_(COMB|FF|IO):|Max frequency"
  fi
fi

# ── The ML-KEM NTT butterfly (Montgomery fqmul): proven bit-exact vs the native fqmul oracle ──
if [ -f ntt_butterfly.v ] && [ -f ntt_vec.hex ]; then
  if command -v nextpnr-ecp5 >/dev/null 2>&1; then
    echo "== FPGA: ML-KEM NTT butterfly -> ECP5 (utilization) =="
    yosys -q -p "read_verilog ntt_butterfly.v; synth_ecp5 -top ntt_butterfly -json ntt_butterfly.json"
    nextpnr-ecp5 --85k --json ntt_butterfly.json --lpf-allow-unconstrained --textcfg ntt_butterfly.config 2>&1 \
      | grep -iE "TRELLIS_(COMB|FF|IO):|Max frequency"
  fi
fi

# ── The rung-3 DRAM COMMAND-STREAM EXECUTOR: replays a real compile-time plan in order ──
if [ -f dram_executor.v ] && [ -f tb_dram_executor.v ] && [ -f dram_plan.hex ]; then
  if command -v iverilog >/dev/null 2>&1; then
    echo "== SIM: the in-order DRAM command-stream executor replays a real plan (iverilog) =="
    iverilog -o tb_dram_executor.vvp dram_executor.v tb_dram_executor.v \
      && vvp tb_dram_executor.vvp | grep -E "PASS|FAIL"
  fi
  if command -v nextpnr-ecp5 >/dev/null 2>&1; then
    echo "== FPGA: DRAM command-stream executor -> ECP5 (utilization + Fmax) =="
    yosys -q -p "read_verilog dram_executor.v; synth_ecp5 -top dram_executor -json dram_executor.json"
    nextpnr-ecp5 --85k --json dram_executor.json --lpf-allow-unconstrained --textcfg dram_executor.config 2>&1 \
      | grep -iE "TRELLIS_(COMB|FF|IO):|Max frequency"
  fi
fi

# ── The INTEGRATED INFINICHIP subsystem: command player + SECDED ECC + double-buffer wired into
# one top. The self-checking testbench drives all three datapaths through the composed module. ──
if [ -f infinichip.v ] && [ -f tb_infinichip.v ]; then
  if command -v iverilog >/dev/null 2>&1; then
    echo "== SIM: the integrated infinichip subsystem (command player + ECC + double buffer) (iverilog) =="
    iverilog -g2012 -o tb_infinichip.vvp infinichip.v tb_infinichip.v \
      && vvp tb_infinichip.vvp | grep -E "PASS|FAIL"
  fi
  if command -v nextpnr-ecp5 >/dev/null 2>&1; then
    echo "== FPGA: infinichip subsystem -> ECP5 (utilization + Fmax) =="
    yosys -q -p "read_verilog infinichip.v; synth_ecp5 -top infinichip -json infinichip.json"
    nextpnr-ecp5 --85k --json infinichip.json --lpf-allow-unconstrained --textcfg infinichip.config 2>&1 \
      | grep -iE "TRELLIS_(COMB|FF|IO):|Max frequency"
  fi
fi

# ── The CARAVEL USER MACRO: the command player behind a Wishbone slave, the shuttle-integratable
# form. The self-checking testbench drives the plan word over Wishbone and checks the FSM issues. ──
if [ -f dram_executor_wb.v ] && [ -f tb_dram_executor_wb.v ]; then
  if command -v iverilog >/dev/null 2>&1; then
    echo "== SIM: the Caravel user macro (Wishbone-driven command player) issues the command (iverilog) =="
    iverilog -g2012 -o tb_dram_executor_wb.vvp dram_executor_wb.v tb_dram_executor_wb.v \
      && vvp tb_dram_executor_wb.vvp | grep -E "PASS|FAIL"
  fi
fi

# ── The NAND STORAGE COMMAND-STREAM EXECUTOR: the storage analog of dram_executor. Replays a
# host-managed-flash command plan in strict order (the deterministic storage controller). ──
if [ -f storage_controller.v ] && [ -f tb_storage_controller.v ] && [ -f storage_plan.hex ]; then
  if command -v iverilog >/dev/null 2>&1; then
    echo "== SIM: the in-order NAND storage command-stream executor replays a real plan (iverilog) =="
    iverilog -g2012 -o tb_storage_controller.vvp storage_controller.v tb_storage_controller.v \
      && vvp tb_storage_controller.vvp | grep -E "PASS|FAIL"
  fi
  if command -v nextpnr-ecp5 >/dev/null 2>&1; then
    echo "== FPGA: NAND storage command-stream executor -> ECP5 (utilization + Fmax) =="
    yosys -q -p "read_verilog storage_controller.v; synth_ecp5 -top storage_controller -json storage_controller.json"
    nextpnr-ecp5 --85k --json storage_controller.json --lpf-allow-unconstrained --textcfg storage_controller.config 2>&1 \
      | grep -iE "TRELLIS_(COMB|FF|IO):|Max frequency"
  fi
fi

# ── Standalone coprocessor blocks: CRDT merge, channel FIFO, BigInt adder, Futamura fabric ──
# Each emitter lives in codegen_sva/{crdt,chan,bignum,fabric}_rtl.rs; each self-checking TB proves
# the RTL == its software oracle bit-exact under iverilog. Section descriptions match the SECTION /
# SIM_SECTION maps in scripts/record-silicon-reports.sh so the numbers land in reports.json.
for pair in \
  "crdt_merge|CRDT (G-Counter) join-semilattice merge" \
  "chan_fifo|the language bounded FIFO channel" \
  "bigint_alu|the 256-bit BigInt add/subtract/compare datapath" \
  "bigint_mul|the 64x64 BigInt limb multiplier" \
  "shared_divider|the shared iterative divider" \
  "bigint_div|the 256-bit BigInt divide block" \
  "fabric_horner|the Futamura dispatch-free residual dataflow circuit" \
  "cgra_sumto|the Futamura counted-loop CGRA (sequential body datapath + loop counter)" \
  "cgra_altsum|the Futamura predicated CGRA (data-dependent branch if-converted to a mux tile)" \
  "while_accw|the Futamura while-loop CGRA (fabric-predicate controller, data-dependent trip count)" \
  "while_clampw|the Futamura while-loop CGRA with a branch body (predicate + mux combined)" \
  "nested_nsum|the Futamura nested-loop CGRA (hierarchical outer+inner controller, fixed inner bound)" \
  "nested_tri|the Futamura nested-loop CGRA (inner range depends on the outer induction variable)" \
  "grid_horner|the placed-and-routed CGRA tile array (2-D switchbox interconnect, P&R certified)" \
  "tmux_linsq|the time-multiplexed CGRA array (one array runs two fabrics by a ctx bit)" \
  "gpu_simt|the data-parallel SIMT engine (N predicated lanes -> N outputs + warp-reduce; sequential shift-add multiply, bounded fan-out)"; do
  blk="${pair%%|*}"; desc="${pair#*|}"
  if [ -f "$blk.v" ] && [ -f "tb_$blk.v" ]; then
    if command -v iverilog >/dev/null 2>&1; then
      echo "== SIM: $desc vs its software oracle (iverilog) =="
      iverilog -g2012 -o "tb_$blk.vvp" "$blk.v" "tb_$blk.v" && vvp "tb_$blk.vvp" | grep -E "PASS|FAIL"
    fi
    if command -v nextpnr-ecp5 >/dev/null 2>&1; then
      echo "== FPGA: $desc -> ECP5 (utilization + Fmax) =="
      yosys -q -p "read_verilog $blk.v; synth_ecp5 -top $blk -json $blk.json"
      nextpnr-ecp5 --85k --json "$blk.json" --lpf-allow-unconstrained --textcfg "$blk.config" 2>&1 \
        | grep -iE "TRELLIS_(COMB|FF|IO):|Max frequency"
    fi
  fi
done

# ── The COPROCESSOR-ISSUING CORE: fetch/decode/issue/retire over the real coprocessor blocks ──
# coproc_core instantiates bigint_alu + bigint_mul + crdt_merge as functional units, so it is a
# MULTI-FILE build (unlike the single-file datapath blocks above).
if [ -f coproc_core.v ] && [ -f tb_coproc_core.v ]; then
  cc_deps="coproc_core.v bigint_alu.v bigint_mul.v crdt_merge.v"
  if command -v iverilog >/dev/null 2>&1; then
    echo "== SIM: the coprocessor-issuing core (issue/retire over the real blocks) vs its software oracle (iverilog) =="
    iverilog -g2012 -o tb_coproc_core.vvp $cc_deps tb_coproc_core.v && vvp tb_coproc_core.vvp | grep -E "PASS|FAIL"
  fi
  if command -v nextpnr-ecp5 >/dev/null 2>&1; then
    echo "== FPGA: the coprocessor-issuing core -> ECP5 (utilization + Fmax) =="
    yosys -q -p "read_verilog $cc_deps; synth_ecp5 -top coproc_core -json coproc_core.json"
    nextpnr-ecp5 --85k --json coproc_core.json --lpf-allow-unconstrained --textcfg coproc_core.config 2>&1 \
      | grep -iE "TRELLIS_(COMB|FF|IO):|Max frequency"
  fi
fi

# ── The FLASHABLE coproc SoC: coproc_core + a UART-TX result readout, routed with the REAL ULX3S pins ──
# coproc_core's rout[255:0] is 262 pins (unroutable on a board). coproc_soc wraps it in a 7-state UART-TX
# readout FSM so it fits 11 pins (clk/rst/uart_tx/led) and BOOTS: it holds the core in reset, runs it, then
# streams the retired R2..R5 over the FTDI UART; scripts/coproc-host.py reconstructs them and matches run_core.
if [ -f coproc_soc.v ] && [ -f tb_coproc_soc.v ]; then
  cs_deps="coproc_soc.v coproc_core.v bigint_alu.v bigint_mul.v crdt_merge.v"
  if command -v iverilog >/dev/null 2>&1; then
    echo "== SIM: the flashable coproc SoC (UART readout) vs its software oracle (iverilog) =="
    iverilog -g2012 -o tb_coproc_soc.vvp $cs_deps tb_coproc_soc.v && vvp tb_coproc_soc.vvp | grep -E "PASS|FAIL"
  fi
  if command -v nextpnr-ecp5 >/dev/null 2>&1; then
    echo "== FPGA: the flashable coproc SoC -> ECP5 (utilization + Fmax, real ULX3S pins) =="
    yosys -q -p "read_verilog $cs_deps; synth_ecp5 -top coproc_soc -json coproc_soc.json"
    nextpnr-ecp5 --85k --json coproc_soc.json --lpf ulx3s_coproc.lpf --package CABGA381 --textcfg coproc_soc.config 2>&1 \
      | grep -iE "TRELLIS_(COMB|FF|IO):|Max frequency"
  fi
fi

# nexys_soc wraps the FULL 113-op soft_core in a USB-UART loader: a host streams a bytecode program in over
# serial (length byte + MSB-first words), the core runs it, and every Show streams back (0xA5/count/values/0x5A).
# tb_nexys_soc loads 3 programs and checks the read-back values; scripts/nexys-host.py does the same round-trip
# on a real Nexys A7. Artix-7 board top (fit in nexys_soc_fit.json; the .bit is a Vivado step via nexys_soc.xdc).
if [ -f nexys_soc.v ] && [ -f tb_nexys_soc.v ]; then
  if command -v iverilog >/dev/null 2>&1; then
    echo "== SIM: the board-runnable Nexys SoC (UART load + run + readback) vs the VM (iverilog) =="
    iverilog -g2012 -s tb -o tb_nexys_soc.vvp nexys_soc.v soft_core.v alu.v shared_divider.v crdt_merge.v bigint_alu.v bigint_div.v tb_nexys_soc.v \
      && vvp tb_nexys_soc.vvp | grep -E "PASS|FAIL"
  fi
fi

# ── The LOGOS SCALAR SOFT-CORE: the ROUTABLE fetch-decode-execute CPU (PC+regfile+ALU+branch+recursion) ──
# The full 113-op soft_core.v is behaviourally == the VM but not yet routable (its collection heap is read
# asynchronously from ~111 sites — no BRAM mapping; sync-read refactor in progress). The routable soft-core
# today is the scalar-ISA subset soft_core_scalar.v, so THAT is what we synth+route+record here.
if [ -f soft_core_scalar.v ]; then
  if command -v iverilog >/dev/null 2>&1; then
    # Derive the scalar sim TB drift-free from the model-emitted tb_gen_sum_loop.v if it isn't present.
    if [ ! -f tb_scalar_sum_loop.v ] && [ -f tb_gen_sum_loop.v ]; then
      sed 's/^\(\s*\)soft_core dut(/\1soft_core_scalar dut(/' tb_gen_sum_loop.v > tb_scalar_sum_loop.v
    fi
    if [ -f tb_scalar_sum_loop.v ]; then
      echo "== SIM: the LOGOS scalar soft-core runs a real bytecode program (iverilog) =="
      iverilog -o tb_scalar.vvp alu.v soft_core_scalar.v tb_scalar_sum_loop.v && vvp tb_scalar.vvp | grep -E "PASS|FAIL"
    fi
  fi
  if command -v nextpnr-ecp5 >/dev/null 2>&1; then
    echo "== FPGA: the LOGOS scalar soft-core -> ECP5 (utilization + Fmax) =="
    yosys -q -p "read_verilog alu.v soft_core_scalar.v; synth_ecp5 -top soft_core_scalar -json soft_core_scalar.json"
    nextpnr-ecp5 --85k --json soft_core_scalar.json --lpf-allow-unconstrained --textcfg soft_core_scalar.config 2>&1 \
      | grep -iE "TRELLIS_(COMB|FF|IO):|Max frequency"
  fi
fi
# ── The FULL 113-op soft-core: iverilog-verified == VM (sim only; not yet routable — E2f sync-BRAM WIP) ──
if [ -f soft_core.v ] && command -v iverilog >/dev/null 2>&1 && [ -f tb_soft_core.v ]; then
  echo "== SIM: the full 113-op LOGOS soft-core runs a real bytecode program (iverilog) =="
  iverilog -o tb_soft_core.vvp alu.v shared_divider.v crdt_merge.v bigint_alu.v bigint_div.v soft_core.v tb_soft_core.v && vvp tb_soft_core.vvp | grep -E "PASS|FAIL"
  if [ -f tb_soft_core_divide.v ]; then
    echo "== SIM: the soft-core issues Div/Mod/FloorDiv to the shared_divider (incl. negatives), vs the VM (iverilog) =="
    iverilog -g2012 -o tb_scdiv.vvp alu.v shared_divider.v crdt_merge.v bigint_alu.v bigint_div.v soft_core.v tb_soft_core_divide.v && vvp tb_scdiv.vvp | grep -E "PASS|FAIL"
  fi
  if [ -f tb_soft_core_cjoin.v ]; then
    echo "== SIM: the soft-core issues CrdtJoin (op 113) to the crdt_merge coprocessor (per-replica MAX), vs the VM (iverilog) =="
    iverilog -g2012 -s tb -o tb_sccj.vvp alu.v shared_divider.v crdt_merge.v bigint_alu.v bigint_div.v soft_core.v tb_soft_core_cjoin.v && vvp tb_sccj.vvp | grep -E "PASS|FAIL"
  fi
  if [ -f tb_soft_core_bigint.v ]; then
    echo "== SIM: the soft-core runs the 256-bit BigInt lane (BigIntWide op 114 -> bigint_alu/div/mul), vs the VM (iverilog) =="
    iverilog -g2012 -s tb -o tb_scbi.vvp alu.v shared_divider.v crdt_merge.v bigint_alu.v bigint_div.v soft_core.v tb_soft_core_bigint.v && vvp tb_scbi.vvp | grep -E "PASS|FAIL"
  fi
fi
