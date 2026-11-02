# LOGOS diode-aware fanout repair (run by routing.tcl unless LOGOS_DIODE_FANOUT is 0).
#
# The resizer meets the SDC's set_max_fanout on every logic net — all 23 signed-off blocks carried 0 logic nets
# over it — and every violation signoff then reported was added after it: antenna diodes, which the liberty
# counts as loads (DIODE carries the library's default fanout load, 1), and clock-tree leaves. On
# wpe_user_project (RUN_2026.09.28_20.20.58) 5,052 of 5,755 violating nets were over only because of their
# diodes: the heuristic pass hangs one on every input of a net spanning 300 um or more, so ten mux2 inputs read
# as fanout 20, and global routing's repair adds one per iteration on a pin its estimate still flags, up to 26
# on one net.
#
# This step runs after a pass has added diodes and before the routers see the netlist. A signal net over the
# design limit is split by grouping whole units — a gate with the diodes placed for it — into buffered bins
# (logos_diode_fanout_core.tcl), so every diode stays on the net of the pin it was placed beside, the diodes beside
# the driver on the driver's. A gate whose diodes alone exceed the limit keeps the nearest that fit; a clock net, whose
# tree is CTS's, is never split and sheds diodes from the pins nearest its driver. The post-route antenna check judges
# every pin again on the real route.
#
# Every net is held to the design limit, the fanout margin under the SDC's (logos_fanout_limit.tcl), so the
# post-route diode ECO always has room for the one diode a pin it finds still needs: ECO diodes land one per net
# (51 on 50 wpe_user_project nets, 0-3 on every other block). The resizers build to the SDC's limit itself; this
# step takes a net at it to the design limit as it takes a diode-laden one, its least-slack gates kept on the
# driver, so the margin adds no stage to a critical path.
source $::env(SCRIPTS_DIR)/openroad/common/io.tcl
# The corners the resizers time (resizer_routing_timing.tcl): slack read here, and the setup repair that closes
# this step, judge what the flow's own timing repair judged.
set read_args [list]
if { $::env(RSZ_MULTICORNER_LIB) } {
    lappend read_args -lib_fastest $::env(RSZ_LIB_FASTEST)
    lappend read_args -lib_slowest $::env(RSZ_LIB_SLOWEST)
}
lappend read_args -lib_typical $::env(RSZ_LIB)
read {*}$read_args
set_propagated_clock [all_clocks]
source $::env(SCRIPTS_DIR)/openroad/common/resizer.tcl
set_dont_touch_wrapper
if { [info exists ::env(DONT_USE_CELLS)] } { set_dont_use $::env(DONT_USE_CELLS) }
source $::env(SCRIPTS_DIR)/openroad/common/set_routing_layers.tcl
source $::env(SCRIPTS_DIR)/openroad/common/set_layer_adjustments.tcl
source $::env(SCRIPTS_DIR)/openroad/common/set_rc.tcl
source $::env(SCRIPTS_DIR)/openroad/logos_diode_fanout_lib.tcl

proc logos_fo_measure {} {
    global_route -congestion_iterations $::env(GRT_OVERFLOW_ITERS) -allow_congestion
    estimate_parasitics -global_routing
}

set block [ord::get_db_block]
set db [ord::get_db]
set logos_fo_seq 0
set limit [expr { int(round($::env(MAX_FANOUT_CONSTRAINT))) }]
set cap [logos_design_max_fanout]
if { $cap < 2 } {
    puts stderr "\[LOGOS-FANOUT\] a design limit of $cap (MAX_FANOUT_CONSTRAINT $limit) leaves no room for a gate and its diode"
    exit 1
}
puts "\[LOGOS-FANOUT\] every net to $cap loads, [expr { $limit - $cap }] under the signoff limit of $limit for the post-route diode ECO"

# The single-net repairs below place through the detailed placer's grid, which exists only once a legalization has
# run (DPL-0043 otherwise). The design is already legal here, so this moves nothing.
source $::env(SCRIPTS_DIR)/openroad/common/dpl_cell_pad.tcl
detailed_placement

set fo_before [llength [logos_fo_over $block $cap]]
set heuristic [expr { [info exists ::env(RUN_HEURISTIC_DIODE_INSERTION)] && $::env(RUN_HEURISTIC_DIODE_INSERTION) == 1 }]
set st [dict create split_nets 0 buffers 0 trimmed_heavy 0 trimmed_clock 0 clock_over 0 guarded 0 created [list] touched [list] doomed [list]]

set repaired [logos_fo_return_named $block]
puts "\[LOGOS-FANOUT\] returned $repaired diode(s) an earlier pass had moved off their gate's net"
set stranded_before [llength [logos_fo_stranded $block]]

logos_fo_measure
lassign [logos_fo_candidates $block $cap] candidates pin_slack skipped_dont_touch
foreach net $candidates {
    logos_fo_split_net $block $db $net $cap $pin_slack $heuristic st
}
foreach inst [dict get $st doomed] { odb::dbInst_destroy $inst }
lassign [logos_fo_size_touched $block $db [dict get $st touched] logos_fo_measure] grown isolated

detailed_placement
logos_fo_held_timing_repair $block logos_fo_measure

set separated [logos_fo_separated $block]
set stranded_after [logos_fo_stranded $block]
set stranded [llength $stranded_after]

detailed_placement
if { [catch {check_placement -verbose} errmsg] } {
    puts stderr $errmsg
    exit 1
}
set residual_signal 0
set residual_clock 0
set over_after [logos_fo_over $block $cap]
foreach o $over_after {
    lassign $o name is_clock
    if { $is_clock } { incr residual_clock } else { incr residual_signal }
    puts "\[LOGOS-FANOUT\]   residual $name ([expr { $is_clock ? "clock" : "signal" }])"
}
set fo_after [llength $over_after]
puts "\[LOGOS-FANOUT\] split [dict get $st split_nets] net(s) with [dict get $st buffers] buffer(s) ([dict get $st guarded] input(s) guarded by a diode), grew $grown driver(s) and isolated $isolated for slew; shed [dict get $st trimmed_heavy] diode(s) from gates over the limit and [dict get $st trimmed_clock] from clock nets; $skipped_dont_touch dont_touch net(s) left; $separated diode(s) off their gate's net; diodes left behind: $stranded_before -> $stranded"
puts "\[LOGOS-FANOUT\] nets over the design limit of $cap: $fo_before -> $fo_after ($residual_signal signal, $residual_clock clock; [dict get $st clock_over] clock net(s) with more pins than it, which CTS's sink clustering decides)"
if { $separated > 0 } {
    puts stderr "\[LOGOS-FANOUT\] $separated diode(s) left their gate's net"
    exit 1
}
if { $stranded > $stranded_before } {
    foreach row $stranded_after {
        lassign $row dname nname um
        puts stderr "\[LOGOS-FANOUT\]   $dname on $nname, $um um from its nearest pin"
    }
    puts stderr "\[LOGOS-FANOUT\] the split left $stranded diode(s) behind, $stranded_before before it"
    exit 1
}
unset_dont_touch_wrapper
write
