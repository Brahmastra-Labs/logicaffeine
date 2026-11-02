# Vivado non-project batch flow: the full 113-op LOGOS soft-core -> Nexys A7-100T bitstream.
#
#   vivado -mode batch -source build_nexys.tcl
#
# Produces soft_core_nexys.bit plus utilization/timing reports. Run from benchmarks/silicon/
# (or `cd` there first) so the relative RTL/XDC paths resolve. The open-source fit check
# (scripts/synth-soft-core-nexys.sh, yosys) already proved this maps to ~38.5k / 63,400 LUT6
# (60.8%); Vivado does the place-and-route + bitstream that no open 7-series tool provides.
#
# Program the board afterwards with `openFPGALoader -b nexys_a7_100 soft_core_nexys.bit`
# (or Vivado's Hardware Manager). SW0 is reset (active-high); the low 16 result bits show on
# the LEDs, `halted` on LD16_R, `out_valid` on LD17_R.

set part xc7a100t-1csg324c
set top  soft_core

read_verilog [list soft_core.v alu.v shared_divider.v]
read_xdc nexys_a7.xdc

synth_design -top $top -part $part
opt_design
place_design
phys_opt_design
route_design

report_utilization      -file util_nexys.rpt
report_timing_summary   -file timing_nexys.rpt -max_paths 10
report_drc              -file drc_nexys.rpt

write_bitstream -force soft_core_nexys.bit
puts "== wrote soft_core_nexys.bit — see util_nexys.rpt / timing_nexys.rpt =="
