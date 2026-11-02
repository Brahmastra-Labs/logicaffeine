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
if { $::env(CTS_MULTICORNER_LIB) } {
    lappend read_args -lib_fastest $::env(LIB_CTS_FASTEST)
    lappend read_args -lib_slowest $::env(LIB_CTS_SLOWEST)
}
lappend read_args -lib_typical $::env(LIB_CTS)
read {*}$read_args

# set rc values
source $::env(SCRIPTS_DIR)/openroad/common/set_rc.tcl
estimate_parasitics -placement

# Clone clock tree inverters next to register loads
# so cts does not try to buffer the inverted clocks.
repair_clock_inverters

puts "\[INFO\]: Configuring cts characterization..."
set cts_characterization_args [list]
lappend cts_characterization_args -max_cap [expr {$::env(CTS_MAX_CAP) * 1e-12}]; # pF -> F
if { [info exists ::env(MAX_TRANSITION_CONSTRAINT)] } {
    lappend cts_characterization_args -max_slew [expr {$::env(MAX_TRANSITION_CONSTRAINT) * 1e-9}]; # ns -> S
}
configure_cts_characterization {*}$cts_characterization_args

puts "\[INFO]: Performing clock tree synthesis..."
puts "\[INFO]: Looking for the following net(s): $::env(CLOCK_NET)"
puts "\[INFO]: Running Clock Tree Synthesis..."

set arg_list [list]

lappend arg_list -buf_list $::env(CTS_CLK_BUFFER_LIST)
lappend arg_list -root_buf $::env(CTS_ROOT_BUFFER)
# LOGOS CLOCK TREE WITHIN THE FANOUT LIMIT (on unless LOGOS_CTS_FANOUT is 0). Two loads
# per buffer decide a clock net's fanout, and both default past the SDC's set_max_fanout: a leaf cluster may hold
# CTS_SINK_CLUSTERING_SIZE sinks (25), and each H-tree split may hand one branch 60% of its parent's clusters, so
# imbalance compounds level by level — five levels let a leaf drive 1.2^5 = 2.5x the average. MEASURED on gpu_simt
# (RUN_2026.09.23_09.37.32 placement, replayed): 33 clock nets over 10 at the defaults (11-14 sinks); 3 with
# clusters held to 10 (H-tree leaves driving 11-12 cluster buffers); 0 with the splits balanced as well, from the
# same 317 clock buffers and with skew 0.54 -> 0.48 ns. The clusters and the tree's depth (through logos_cts.py,
# which runs this script) follow the design limit, the fanout margin under the SDC's (logos_fanout_limit.tcl).
set sink_clustering_size $::env(CTS_SINK_CLUSTERING_SIZE)
if { ![info exists ::env(LOGOS_CTS_FANOUT)] || $::env(LOGOS_CTS_FANOUT) == 1 } {
    source $::env(SCRIPTS_DIR)/openroad/logos_fanout_limit.tcl
    set fanout_limit [logos_design_max_fanout]
    if { $sink_clustering_size > $fanout_limit } { set sink_clustering_size $fanout_limit }
    lappend arg_list -clustering_unbalance_ratio 0.5
    puts "\[LOGOS-CTS\] leaf clusters of at most $sink_clustering_size sinks, H-tree splits balanced"
}
lappend arg_list -sink_clustering_size $sink_clustering_size
lappend arg_list -sink_clustering_max_diameter $::env(CTS_SINK_CLUSTERING_MAX_DIAMETER)
lappend arg_list -sink_clustering_enable
# LOGOS H-TREE K-MEANS WITHIN INT. Each H-tree level's two-way split solves an integer min-cost flow whose arc costs
# grow as distance^exponent; on a large core the default exponent overflows it and the tree never finishes (the WPE
# macro spun 22 h at level 2). The largest exponent the core keeps in range, 4 on a small one (logos_cts_kmeans.tcl).
source $::env(SCRIPTS_DIR)/openroad/logos_cts_kmeans.tcl
lappend arg_list -clustering_exponent [logos_cts_kmeans_exponent]

if { $::env(CTS_DISTANCE_BETWEEN_BUFFERS) != 0 } {
    lappend arg_list -distance_between_buffers $::env(CTS_DISTANCE_BETWEEN_BUFFERS)
}

if { $::env(CTS_DISABLE_POST_PROCESSING) } {
    lappend arg_list -post_cts_disable
}

clock_tree_synthesis {*}$arg_list

set_propagated_clock [all_clocks]

estimate_parasitics -placement
puts "\[INFO]: Repairing long wires on clock nets..."
# CTS leaves a long wire from the pad to the clock tree root.
repair_clock_nets -max_wire_length $::env(CTS_CLK_MAX_WIRE_LENGTH)

if { ![info exists ::env(LOGOS_CTS_FANOUT)] || $::env(LOGOS_CTS_FANOUT) == 1 } {
    source $::env(SCRIPTS_DIR)/openroad/logos_clock_fanout.tcl
    lassign [logos_clone_clock_drivers [logos_design_max_fanout]] _ck_clones _ck_residual
    puts "\[LOGOS-CTS\] cloned $_ck_clones clock buffer(s) driving more than [logos_design_max_fanout]; [llength $_ck_residual] port-driven clock net(s) over it: $_ck_residual"
}

estimate_parasitics -placement
write

puts "\[INFO\]: Legalizing..."
source $::env(SCRIPTS_DIR)/openroad/common/dpl_cell_pad.tcl

detailed_placement

if { [info exists ::env(PL_OPTIMIZE_MIRRORING)] && $::env(PL_OPTIMIZE_MIRRORING) } {
    optimize_mirroring
}
estimate_parasitics -placement

write
if { [catch {check_placement -verbose} errmsg] } {
    puts stderr $errmsg
    exit 1
}

puts "cts_report"
report_cts
puts "cts_report_end"

