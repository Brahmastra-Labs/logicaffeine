# Modified by the LOGOS project from OpenLane ff5509f65b17bfa4068d5336495ab1718987ff69; see scripts/openlane-overlay/NOTICE.
# Copyright 2020-2022 Efabless Corporation
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#      http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
source $::env(SCRIPTS_DIR)/openroad/common/io.tcl
set read_args [list]
if { $::env(RSZ_MULTICORNER_LIB) } {
    lappend read_args -lib_fastest $::env(RSZ_LIB_FASTEST)
    lappend read_args -lib_slowest $::env(RSZ_LIB_SLOWEST)
}
lappend read_args -lib_typical $::env(RSZ_LIB)
read {*}$read_args

set_propagated_clock [all_clocks]

# set don't touch nets
source $::env(SCRIPTS_DIR)/openroad/common/resizer.tcl
set_dont_touch_wrapper

# set don't use cells
if { [info exists ::env(DONT_USE_CELLS)] } {
    set_dont_use $::env(DONT_USE_CELLS)
}

source $::env(SCRIPTS_DIR)/openroad/common/set_routing_layers.tcl

source $::env(SCRIPTS_DIR)/openroad/common/set_layer_adjustments.tcl

set arg_list [list]
lappend arg_list -congestion_iterations $::env(GRT_OVERFLOW_ITERS)
lappend arg_list -verbose
lappend arg_list -congestion_report_file $::env(GRT_CONGESTION_REPORT_FILE)
if { $::env(GRT_ALLOW_CONGESTION) == 1 } {
    lappend arg_list -allow_congestion
}
puts $arg_list
global_route {*}$arg_list

# set rc values
source $::env(SCRIPTS_DIR)/openroad/common/set_rc.tcl

# estimate wire rc parasitics
estimate_parasitics -global_routing

# LOGOS: both repairs can leave out pin swapping (flow_config::ClosureSettings::glb_skip_pin_swap).
set skip_pin_swap [expr {[info exists ::env(LOGOS_GLB_SKIP_PIN_SWAP)] && $::env(LOGOS_GLB_SKIP_PIN_SWAP) == 1}]

# Resize
set setup_args [list -setup]
lappend setup_args -setup_margin $::env(GLB_RESIZER_SETUP_SLACK_MARGIN)
lappend setup_args -max_buffer_percent $::env(GLB_RESIZER_SETUP_MAX_BUFFER_PERCENT)
if { $skip_pin_swap } {
    lappend setup_args -skip_pin_swap
}
repair_timing {*}$setup_args

set arg_list [list]
lappend arg_list -hold
lappend arg_list -setup_margin $::env(GLB_RESIZER_SETUP_SLACK_MARGIN)
lappend arg_list -hold_margin $::env(GLB_RESIZER_HOLD_SLACK_MARGIN)
lappend arg_list -max_buffer_percent $::env(GLB_RESIZER_HOLD_MAX_BUFFER_PERCENT)
if { $::env(GLB_RESIZER_ALLOW_SETUP_VIOS) == 1 } {
    lappend arg_list -allow_setup_violations
}
if { $skip_pin_swap } {
    lappend arg_list -skip_pin_swap
}
repair_timing {*}$arg_list

source $::env(SCRIPTS_DIR)/openroad/common/dpl_cell_pad.tcl

detailed_placement

if { $::env(GLB_OPTIMIZE_MIRRORING) } {
    optimize_mirroring
}

if { [catch {check_placement -verbose} errmsg] } {
    puts stderr $errmsg
    exit 1
}

unset_dont_touch_wrapper

write

# Run post timing optimizations STA
estimate_parasitics -global_routing

puts "area_report"
puts "\n==========================================================================="
puts "report_design_area"
puts "============================================================================"
report_design_area
puts "area_report_end"
