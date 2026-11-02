# build_zcu104.tcl — non-project Vivado build of zcu104_top around one Nexys UART SoC.
#
#   vivado -mode batch -source build_zcu104.tcl -tclargs <inner_top> "<rtl files>" ["<extra defines>"]
#
#   nexys_wpe_soc      "nexys_wpe_soc.v wpe_soc_fpga.v"
#   nexys_soc          "nexys_soc.v soft_core.v alu.v shared_divider.v bigint_alu.v bigint_div.v crdt_merge.v fpu.v"
#   nexys_soc_tformer  "nexys_soc_tformer.v soft_core.v alu.v shared_divider.v bigint_alu.v bigint_div.v crdt_merge.v fpu.v"
#   nexys_fabric_soc   "nexys_fabric_soc.v config_fabric.v" "INNER_NO_RX"
#
# The soft-core reads program.hex at elaboration, so run from benchmarks/silicon. If Vivado refuses the
# parseInt trim loops in soft_core.v, pass "SOFT_CORE_NO_PARSEINT" (Yosys needs it; it drops text parseInt only).
# Writes zcu104_<inner>.bit plus utilization and timing reports. Vivado 2025.2 to match the board's PetaLinux image.
set inner [lindex $argv 0]
set srcs  [lindex $argv 1]
set extra [lindex $argv 2]
set part  xczu7ev-ffvc1156-2-e

read_verilog [concat [list zcu104_top.v] $srcs]
read_xdc zcu104.xdc

set defs [concat [list INNER=$inner] $extra]

synth_design -top zcu104_top -part $part -verilog_define $defs
opt_design
place_design
phys_opt_design
route_design
report_utilization    -file util_zcu104_$inner.rpt
report_timing_summary -file timing_zcu104_$inner.rpt -max_paths 10
report_drc            -file drc_zcu104_$inner.rpt
write_bitstream -force zcu104_$inner.bit
puts "== wrote zcu104_$inner.bit — see util_zcu104_$inner.rpt / timing_zcu104_$inner.rpt =="
