# LOGOS post-diode driver upsizing (gated by LOGOS_POST_DIODE_UPSIZE in routing.tcl).
#
# The slew resizer runs BEFORE the heuristic diode pass, so it sizes every driver for a load that
# does not yet include the diodes hung on its net afterwards. On systolic_pipe16_realbist every
# surviving max-slew violator is such a net, and all of them are marginal (0.75-0.85 ns against a
# 0.75 ns limit). The two design-wide remedies both failed: re-running repair_design after the
# diodes (LOGOS_POST_DIODE_RESIZE) inserted 3,587 buffers for 7 violations and made slew 69 -> 181
# on coproc_core, and raising the resizer margin hit a 114 GB OOM in the placement resizer and an
# hour-plus incremental-reroute cascade in the routing one.
#
# So do the smallest thing that addresses the cause, net by net: for each net that violates max
# slew WITH its diodes present, swap its driver for the next-stronger drive of the SAME function
# (sky130_fd_sc_hd__buf_12 -> _16, nor2_2 -> _4 ...); where the driver is already the strongest of
# its function, buffer that one net. Repeat while a round changes something (twenty-four at most),
# legalizing after every round that did, so each round measures the placement that will be routed.
source $::env(SCRIPTS_DIR)/openroad/common/io.tcl
source $::env(SCRIPTS_DIR)/openroad/logos_global_route.tcl
# Every corner, when asked. Signoff judges slew at ss_100C_1v60 as well as tt; a step that loads only
# the typical library repairs a corner signoff barely cares about and never sees the one it fails
# on — every surviving violator on systolic_pipe16_realbist and systolic_os_pipe8_realbist
# (RUN_2026.09.23_14.05.55 / _11.27.15) was slow-corner only, typical slew 0.
if { [info exists ::env(LOGOS_UPSIZE_ALL_CORNERS)] && $::env(LOGOS_UPSIZE_ALL_CORNERS) == 1 } {
    read -lib_fastest $::env(RSZ_LIB_FASTEST) -lib_slowest $::env(RSZ_LIB_SLOWEST) -lib_typical $::env(RSZ_LIB)
} else {
    read -lib_typical $::env(RSZ_LIB)
}
set_propagated_clock [all_clocks]
source $::env(SCRIPTS_DIR)/openroad/common/resizer.tcl
set_dont_touch_wrapper
if { [info exists ::env(DONT_USE_CELLS)] } { set_dont_use $::env(DONT_USE_CELLS) }
source $::env(SCRIPTS_DIR)/openroad/common/set_routing_layers.tcl
source $::env(SCRIPTS_DIR)/openroad/common/set_layer_adjustments.tcl
# Every run measures incrementally. Its first measure routes the whole design, and every later one re-routes only the
# nets an edit touched, so a net the step leaves keeps the route it was judged on. Re-routing everything each round
# found new detours each round: on coproc_core (RUN_2026.10.03_23.19.59, this step replayed before global routing's
# antenna repair) it reached the 24-round bound with 6 nets left, +262 cells in 228 s; incrementally it stopped after
# 6 rounds with 2, +123 cells in 49 s.
# ROUTE MODE (LOGOS_UPSIZE_ROUTE, the re-route after global routing's antenna repair): the step measures on the
# route detailed routing will follow and hands that route on, so its first measure is the flow's own global route
# (groute.tcl's arguments, antenna repair off). MEASURED on soft_core_realbist (RUN_2026.10.03_20.04.57): net _05405_
# (a21oi_4 to one load 165 um away) ran 179 um of met3 in the route the repairs judged and 248 um of met3 plus 331 um
# of met4 in the plain re-route after them; nothing measured it again, and it signed off at 0.93 ns against 0.75.
set logos_route [expr { [info exists ::env(LOGOS_UPSIZE_ROUTE)] && $::env(LOGOS_UPSIZE_ROUTE) == 1 }]
set logos_routed 0
proc logos_measure {} {
    global logos_route logos_routed
    if { $logos_routed } {
        global_route -end_incremental
    } elseif { $logos_route } {
        set_macro_extension $::env(GRT_MACRO_EXTENSION)
        set grt [list -congestion_iterations $::env(GRT_OVERFLOW_ITERS) -verbose -congestion_report_file $::env(GRT_CONGESTION_REPORT_FILE)]
        if { $::env(GRT_ALLOW_CONGESTION) == 1 } { lappend grt -allow_congestion }
        global_route {*}$grt
        set logos_routed 1
    } else {
        global_route -congestion_iterations $::env(GRT_OVERFLOW_ITERS) -allow_congestion
        set logos_routed 1
    }
    estimate_parasitics -global_routing
    global_route -start_incremental
}
source $::env(SCRIPTS_DIR)/openroad/common/set_rc.tcl
source $::env(SCRIPTS_DIR)/openroad/logos_fanout_limit.tcl
source $::env(SCRIPTS_DIR)/openroad/logos_diode_fanout_lib.tcl
set logos_fo_seq 0

# A cell the flow may place: `DONT_USE_CELLS` carries the process's DRC-excluded cells, and `replace_cell` does
# not consult it.
proc logos_usable {name} {
    return [expr { ![info exists ::env(DONT_USE_CELLS)] || [lsearch -exact $::env(DONT_USE_CELLS) $name] < 0 }]
}

# OpenSTA answers a lookup that finds no object with the string `NULL`; every quiet lookup below reads it as absent.
proc logos_obj {o} {
    return [expr { $o eq "NULL" ? "" : $o }]
}

# A violator can be a top-level port: a pin with no net of its own, whose net is its term's.
proc logos_slew_violating_nets {} {
    set nets [dict create]
    foreach p [sta::check_slew_limits "NULL" 1 "NULL" max] {
        set n [logos_obj [get_nets -quiet -of_objects $p]]
        if { $n eq "" } {
            set term [logos_obj [$p term]]
            if { $term ne "" } { set n [logos_obj [$term net]] }
        }
        if { $n ne "" } { dict set nets [get_full_name $n] 1 }
    }
    return [dict keys $nets]
}

proc logos_stronger_cell {ref} {
    if { ![regexp {^(.*_)([0-9]+)$} $ref -> stem drive] } { return "" }
    set best ""
    foreach c [get_lib_cells -quiet "*/${stem}*"] {
        set name [get_name $c]
        if { ![logos_usable $name] } { continue }
        if { [regexp "^[string map {. \\.} $stem](\[0-9\]+)\$" $name -> d] && $d > $drive } {
            if { $best eq "" || $d < [lindex $best 0] } { set best [list $d $name] }
        }
    }
    return [lindex $best 1]
}

# Detect against a TIGHTENED limit. The violations this step exists for only appear at signoff,
# after detailed routing and RC extraction, and the global-route estimate here is about 2x
# optimistic on exactly those nets. Measured on systolic_pipe16_realbist: the 15 signoff-violating
# pins (0.75-0.85 ns) all estimate at 0.37-0.43 ns, yet the estimate RANKS them correctly - every
# one lies among the 103 worst pins at >= 0.36 ns (22 nets). A 52% detection margin therefore
# selects precisely the nets signoff will flag and little else (0.40 -> 31 pins but misses two;
# 0.34 -> 442; 0.30 -> 2,081). Only DETECTION is tightened; signoff still judges the real limit.
set _m 52
if { [info exists ::env(LOGOS_UPSIZE_MARGIN)] } { set _m $::env(LOGOS_UPSIZE_MARGIN) }
set _limit [expr { $::env(MAX_TRANSITION_CONSTRAINT) * (100 - $_m) / 100.0 }]
set_max_transition $_limit [current_design]
# The single-net repair builds its trees to the design's fanout limit, as every net here is held to, not the SDC's.
set_max_fanout [logos_design_max_fanout] [current_design]
puts "\[LOGOS-UPSIZE\] detecting against [format %.3f $_limit] ns ($_m% below $::env(MAX_TRANSITION_CONSTRAINT))"

# The single-net repair places its buffers through the detailed placer, whose site grid only
# exists once a legalization has run (DPL-0043 "No grid layers mapped" otherwise). The design is
# already legal here, so this pass moves nothing; it only builds the grid.
source $::env(SCRIPTS_DIR)/openroad/common/dpl_cell_pad.tcl
detailed_placement

# A DIODE AND THE PIN IT IS NEAREST STAY WHERE THEY ARE THROUGH EVERY LEGALIZATION. Each round adds cells where slew is
# worst and legalizes, and the legalizer clears an overlap by moving cells to the nearest free sites: in a crowded region
# a diode, and the gate beside it, are carried a few tens of microns a legalization until the diode is past every pin of
# its net (logos_fo_stranded) though no edit moved either. MEASURED on wpe_user_project at 18.5 ns, this step replayed
# from RUN_2026.10.22_07.32.54's step 23: unheld, the legalizations left 5 diodes behind by round 6 and 293 by the end
# (RUN_2026.10.21_16.25.31); a diode set back on its pin after each one is carried off again where no site near the pin
# is free (650 a pass by round 16, 86 left behind, RUN_2026.10.22_07.32.54); held alone, the diodes stay but their gates
# drift (72 diodes past 50 um, 3 past 100 by round 13). So each legalization after an edit holds FIRM every diode, and
# the instance whose pin is its net's nearest to it, as the last legalization left them: placed there, unmoved, the same
# cell. The legalizer moves the edit's cells around them. A held pair was legal together, so no two held cells overlap;
# a cell an edit made, moved or resized legalizes as any cell. The pairs are taken afresh after each legalization.
# Held as pairs, the same replay converged in 11 rounds with no diode left behind (the farthest 50 um from its nearest
# pin, as at the start) and no net over the slew limit, adding 610 cells, where re-homing ran all 24 rounds and left 48
# nets over it with 1,462 cells added.
set logos_settled [dict create]
set logos_partners [dict create]
set logos_held [list]
proc logos_diodes_settle {} {
    global logos_settled logos_partners
    set block [ord::get_db_block]
    set logos_settled [dict create]
    set logos_partners [dict create]
    set nets [dict create]
    foreach inst [$block getInsts] {
        if { [[$inst getMaster] getConstName] ne $::env(DIODE_CELL) } { continue }
        dict set logos_settled [$inst getConstName] [$inst getLocation]
        set it [$inst findITerm $::env(DIODE_CELL_PIN)]
        if { $it eq "NULL" } { continue }
        set net [$it getNet]
        if { $net eq "NULL" || [$net isSpecial] } { continue }
        dict set nets [$net getName] $net
    }
    dict for {name net} $nets {
        set pins [list]
        set diodes [list]
        foreach it [$net getITerms] {
            if { [logos_fo_is_diode $it] } { lappend diodes $it } else { lappend pins $it }
        }
        foreach d $diodes {
            lassign [logos_fo_pin_xy $d] x y
            set near -1
            set best ""
            foreach it $pins {
                lassign [logos_fo_pin_xy $it] px py
                set dd [expr { abs($px - $x) + abs($py - $y) }]
                if { $near < 0 || $dd < $near } {
                    set near $dd
                    set best [$it getInst]
                }
            }
            if { $best ne "" } {
                dict set logos_partners [$best getConstName] [list [$best getLocation] [[$best getMaster] getConstName]]
            }
        }
    }
}
proc logos_diodes_hold {} {
    global logos_settled logos_partners logos_held
    set logos_held [list]
    foreach inst [[ord::get_db_block] getInsts] {
        if { [$inst getPlacementStatus] ne "PLACED" } { continue }
        set name [$inst getConstName]
        if { [[$inst getMaster] getConstName] eq $::env(DIODE_CELL) } {
            if { ![dict exists $logos_settled $name] } { continue }
            if { [dict get $logos_settled $name] ne [$inst getLocation] } { continue }
        } elseif { [dict exists $logos_partners $name] } {
            lassign [dict get $logos_partners $name] loc master
            if { $loc ne [$inst getLocation] || $master ne [[$inst getMaster] getConstName] } { continue }
        } else {
            continue
        }
        $inst setPlacementStatus FIRM
        lappend logos_held $inst
    }
}
proc logos_diodes_release {} {
    global logos_held
    foreach inst $logos_held { $inst setPlacementStatus PLACED }
    set logos_held [list]
    logos_diodes_settle
}
logos_diodes_settle

# A driver already at the strongest drive of its function it may use (a21oi_4 and a31oi_4 have no
# bigger sibling; buf_12 none the flow may place) cannot be upsized; on systolic_pipe16_realbist that is the driver of EVERY
# signoff violator. Such a net is handed to the resizer's own single-net repair, which buffers it
# against the same tightened limit - one net at a time, never the design-wide repair_design pass.
proc logos_inst_count {} { return [llength [[ord::get_db_block] getInsts]] }

# The strongest buffer of the driver's OWN library (the name up to its last "__", then buf_N),
# skipping DONT_USE_CELLS, so the step names no PDK cell.
proc logos_strongest_buffer {ref} {
    set lib [string range $ref 0 [expr { [string last "__" $ref] + 1 }]]
    set best ""
    set best_drive -1
    foreach c [get_lib_cells -quiet "*/${lib}buf_*"] {
        set name [get_name $c]
        if { ![logos_usable $name] } { continue }
        if { [regexp "^[string map {. \\.} $lib]buf_(\[0-9\]+)\$" $name -> d] && $d > $best_drive } {
            set best $name
            set best_drive $d
        }
    }
    return $best
}

# ISOLATE a maximal driver the single-net repair could not help. The repair buffers along a wire
# or resizes; neither changes what a driver already at its strongest drive sees at its own pin,
# which is where its output slew is set. Isolation puts the library's strongest buffer on the
# driver: the buffer takes over the original net and every sink, and the driver keeps one short hop
# to the buffer's input. Clock nets belong to CTS; hold delay cells exist to be slow; and a net an
# isolation buffer already drives gains nothing from a second one, since the wire is as long.
# A net this step holds for its diodes is released for the edit and held again after it: isolation leaves
# every sink and diode on the net, which is what the hold protects, and ODB refuses to rewire a held net
# (ODB-0372). A net the design itself marked dont_touch is left as it is.
set logos_iso_seq 0
proc logos_isolate_driver {net drv} {
    global logos_iso_seq logos_held
    set dcell [logos_obj [get_cells -quiet -of_objects $drv]]
    if { $dcell eq "" } { return 0 }
    set ref [get_property $dcell ref_name]
    set dbnet [sta::sta_to_db_net $net]
    if { [$dbnet getSigType] eq "CLOCK" || [string match "*clkbuf*" $ref] || [string match "*dlygate*" $ref] } {
        return 0
    }
    if { [string match "logos_iso_*" [get_full_name $dcell]] } { return 0 }
    set ours [dict exists $logos_held [$dbnet getName]]
    if { [$dbnet isDoNotTouch] && !$ours } { return 0 }
    set buf [logos_strongest_buffer $ref]
    if { $buf eq "" } { return 0 }
    # A buffer already as strong as the one isolation would add gains a stage and nothing else.
    if { [regexp {__(?:buf|clkbuf|bufbuf)_([0-9]+)$} $ref -> d] && [regexp {_([0-9]+)$} $buf -> top] && $d >= $top } {
        return 0
    }
    set block [ord::get_db_block]
    set bb [[$block findInst [get_full_name $dcell]] getBBox]
    while { [$block findInst "logos_iso_$logos_iso_seq"] ne "NULL" || [$block findNet "logos_iso_net_$logos_iso_seq"] ne "NULL" } {
        incr logos_iso_seq
    }
    set bname "logos_iso_$logos_iso_seq"
    set hop "logos_iso_net_$logos_iso_seq"
    make_instance $bname $buf
    make_net $hop
    if { $ours } { $dbnet setDoNotTouch 0 }
    disconnect_pin $net $drv
    connect_pin $hop $drv
    connect_pin $hop $bname/A
    connect_pin $net $bname/X
    if { $ours } { $dbnet setDoNotTouch 1 }
    set bi [$block findInst $bname]
    $bi setLocation [$bb xMin] [$bb yMin]
    $bi setPlacementStatus PLACED
    return 1
}

# REBUFFER a net this step holds for its diodes, once isolation has nothing to add. The resizer's single-net repair
# builds the buffer tree a long or spread net needs, and it splits a net by its pins' tree, so each diode's pin is
# named first (logos_fo_owners: the gate its name gives, else the nearest pin, the driver among them), our hold is
# released for the repair, any diode the tree parted from its pin goes back onto that pin's net, and every net then
# carrying a diode is held again. Answers how many cells the repair added. MEASURED on coproc_core (RUN_2026.10.03_23.19.59, the
# route-mode step replayed): nets _31891_ and net2738, nine pins over 923 and 1,087 um behind buf_12, one buffer each
# and no diode parted.
proc logos_rebuffer_held {net} {
    global logos_held held
    set block [ord::get_db_block]
    set dbnet [sta::sta_to_db_net $net]
    if { ![dict exists $logos_held [$dbnet getName]] } { return 0 }
    set gates [list]
    set diodes [list]
    foreach it [$dbnet getITerms] {
        if { ![$it isInputSignal] } { continue }
        if { [logos_fo_is_diode $it] } { lappend diodes $it } else { lappend gates $it }
    }
    if { [llength $gates] == 0 } { return 0 }
    set owner [logos_fo_owners $block $dbnet]
    set n0 [logos_inst_count]
    $dbnet setDoNotTouch 0
    rsz::repair_net_cmd $net 0 0 0
    set added [expr { [logos_inst_count] - $n0 }]
    set returned 0
    dict for {d g} $owner {
        set gnet [$g getNet]
        if { $gnet ne "NULL" && [$d getNet] ne $gnet } {
            $d disconnect
            $d connect $gnet
            incr returned
        }
    }
    dict unset logos_held [$dbnet getName]
    dict for {d g} $owner {
        set n [$d getNet]
        if { ![$n isDoNotTouch] } {
            $n setDoNotTouch 1
            lappend held $n
        }
        dict set logos_held [$n getName] 1
    }
    if { $returned > 0 } {
        puts "\[LOGOS-UPSIZE\]   net [$dbnet getName]: $returned diode(s) returned to their gate's net after the repair"
    }
    return $added
}

# HALVE THE LONGEST ARM of a wire the strongest buffer already drives, when the single-net repair added nothing: a route
# that detours far past its pins' tree is invisible to that repair's Steiner estimate. The sink farthest from the driver
# sets how long the wire must be, so the library's strongest buffer goes at the midpoint between the two, and every gate
# nearer that midpoint than the driver moves behind it with the diodes placed beside it (logos_fo_owners; a diode
# beside the driver stays with the driver). Sinks gathered on one far side all
# move, and the buffer is a repeater; a far cluster beside a near one moves alone; a driver amid sinks on both sides
# sheds its farther side, and the next round its other. Each application halves the longest arm it acts on, so the
# rounds converge; an arm under 50 um spans a few cell rows, too short for halving to move any wire that matters. The driver's
# net keeps the buffer's input, guarded as the heuristic pass guards a net that long; every net carrying a diode is
# held. MEASURED on coproc_core (RUN_2026.10.03_23.19.59): net _12999_, five pins over 506 um behind buf_12, was routed
# 1,863 um (1,201 um of met2) and signed off at 0.86 ns against 0.75. A repeater toward the sinks' centroid stalls once
# the driver is among them (logos_fo_net_3092 took 23), and a split with each half's buffer at its centroid left a far
# arm of 1,911 um routed that signed off at 0.84 ns (net _30030_, FO4PROBE2_20261004_0106).
set logos_arm_seq 0
proc logos_halve_arm {net drv} {
    global logos_arm_seq logos_held held
    set dcell [logos_obj [get_cells -quiet -of_objects $drv]]
    if { $dcell eq "" } { return 0 }
    set ref [get_property $dcell ref_name]
    set dbnet [sta::sta_to_db_net $net]
    if { [logos_fo_is_clock $dbnet] || [string match "*dlygate*" $ref] } { return 0 }
    set ours [dict exists $logos_held [$dbnet getName]]
    if { [$dbnet isDoNotTouch] && !$ours } { return 0 }
    set buf [logos_strongest_buffer $ref]
    if { $buf eq "" } { return 0 }
    set block [ord::get_db_block]
    lassign [[$block findInst [get_full_name $dcell]] getLocation] dx dy
    set gates [list]
    set diodes [list]
    foreach it [$dbnet getITerms] {
        if { ![$it isInputSignal] } { continue }
        if { [logos_fo_is_diode $it] } { lappend diodes $it } else { lappend gates $it }
    }
    if { [llength $gates] == 0 } { return 0 }
    set far ""
    set fd -1
    foreach g $gates {
        lassign [[$g getInst] getLocation] x y
        set d [expr { abs($x - $dx) + abs($y - $dy) }]
        if { $d > $fd } {
            set far $g
            set fd $d
        }
    }
    if { $fd < 50 * [$block getDbUnitsPerMicron] } { return 0 }
    lassign [[$far getInst] getLocation] fx fy
    set mx [expr { ($dx + $fx) / 2 }]
    set my [expr { ($dy + $fy) / 2 }]
    set moved [list]
    foreach g $gates {
        lassign [[$g getInst] getLocation] x y
        if { abs($x - $mx) + abs($y - $my) < abs($x - $dx) + abs($y - $dy) } { lappend moved $g }
    }
    set owner [logos_fo_owners $block $dbnet]
    while { [$block findInst "logos_arm_$logos_arm_seq"] ne "NULL" || [$block findNet "logos_arm_net_$logos_arm_seq"] ne "NULL" } {
        incr logos_arm_seq
    }
    set bname "logos_arm_$logos_arm_seq"
    set sub "logos_arm_net_$logos_arm_seq"
    make_instance $bname $buf
    make_net $sub
    set subdb [$block findNet $sub]
    if { $ours } { $dbnet setDoNotTouch 0 }
    foreach g $moved {
        $g disconnect
        $g connect $subdb
        dict for {d o} $owner {
            if { $o eq $g } {
                $d disconnect
                $d connect $subdb
            }
        }
    }
    connect_pin $net $bname/A
    connect_pin $sub $bname/X
    set bi [$block findInst $bname]
    $bi setLocation $mx $my
    $bi setPlacementStatus PLACED
    if { $::env(RUN_HEURISTIC_DIODE_INSERTION) == 1 && [logos_fo_net_span $block $dbnet] >= $::env(HEURISTIC_ANTENNA_THRESHOLD) } {
        logos_fo_guard_input $block [ord::get_db] $bi
    }
    foreach n [list $dbnet $subdb] {
        set nd 0
        foreach it [$n getITerms] { if { [logos_fo_is_diode $it] } { incr nd } }
        if { $nd > 0 } {
            $n setDoNotTouch 1
            lappend held $n
            dict set logos_held [$n getName] 1
        } else {
            dict unset logos_held [$n getName]
        }
    }
    return 1
}

# The diodes on a net: a diode a split shed waits, disconnected, for the closure to destroy it.
proc logos_diode_count {block} {
    set n 0
    foreach i [$block getInsts] {
        if { [[$i getMaster] getConstName] ne $::env(DIODE_CELL) } { continue }
        set dt [$i findITerm $::env(DIODE_CELL_PIN)]
        if { $dt ne "NULL" && [$dt getNet] ne "NULL" } { incr n }
    }
    return $n
}

# What this step owes the fanout step that ran before it: every diode named for a gate still on that gate's net
# (logos_fo_separated), no diode left behind by an edit (logos_fo_stranded), and no signal net over the design's
# fanout limit that was not over it before. Each is counted before the rounds and after the last edit; any one
# growing fails the step.
proc logos_signal_over {block} {
    set n 0
    foreach o [logos_fo_over $block [logos_design_max_fanout]] {
        if { ![lindex $o 1] } { incr n }
    }
    return $n
}

set upsized 0
set rebuffered 0
set isolated 0
set halved 0
set cells_before [logos_inst_count]
set separated_before [logos_fo_separated [ord::get_db_block]]
set stranded_before [llength [logos_fo_stranded [ord::get_db_block]]]
set over_before [logos_signal_over [ord::get_db_block]]
# Every net carrying a diode is held (dont_touch) while the rounds run: the single-net repair splits a net by its
# pins, and on a dioded net that parts diodes from the gates they were placed for (logos_diode_fanout.tcl, which
# runs before this step, holds every diode on its gate's net). Held, such a net with a maximal driver is isolated
# instead, which leaves every sink and diode where it is.
proc logos_hold_dioded {} {
    global held logos_held
    set held [list]
    set logos_held [dict create]
    foreach dbnet [[ord::get_db_block] getNets] {
        if { [$dbnet isSpecial] || [$dbnet isDoNotTouch] } { continue }
        foreach it [$dbnet getITerms] {
            if { [$it isInputSignal] && [[[$it getInst] getMaster] getConstName] eq $::env(DIODE_CELL) } {
                $dbnet setDoNotTouch 1
                lappend held $dbnet
                dict set logos_held [$dbnet getName] 1
                break
            }
        }
    }
}
logos_hold_dioded
# Every round re-routes before it measures: a buffer inserted last round has no global route
# yet, and an unrouted net estimates as nearly free wire. Rounds continue while they change
# something, because a grown driver loads ITS driver: on pipe16 round 3's two upsizes surfaced
# four new nets in round 4. The bound is a backstop, not the stopping rule: with each round legalized,
# pipe16 (RUN_2026.09.29_00.40.21, replayed) still found 4, 4 and 3 nets in rounds 10-12.
proc logos_slew_rounds {} {
    global upsized rebuffered isolated halved held logos_held
    for {set round 1} {$round <= 24} {incr round} {
        logos_measure
        set nets [logos_slew_violating_nets]
        puts "\[LOGOS-UPSIZE\] round $round: [llength $nets] net(s) violating max slew"
        if { [llength $nets] == 0 } { break }
        set changed 0
        foreach nname $nets {
            set net [logos_obj [get_nets -quiet $nname]]
            if { $net eq "" } { continue }
            set drvs [logos_obj [get_pins -quiet -of_objects $net -filter "direction==output"]]
            set grown 0
            foreach drv $drvs {
                set inst [logos_obj [get_cells -quiet -of_objects $drv]]
                if { $inst eq "" } { continue }
                set ref [get_property $inst ref_name]
                if { [string match "*diode*" $ref] || [string match "*dlygate*" $ref] } { continue }
                set up [logos_stronger_cell $ref]
                if { $up eq "" } { continue }
                if { [catch { replace_cell $inst $up }] == 0 } {
                    incr grown
                    puts "\[LOGOS-UPSIZE\]   [get_full_name $inst]: $ref -> $up (net $nname)"
                }
            }
            if { $grown > 0 } {
                incr upsized $grown
                incr changed
            } elseif { [llength $drvs] > 0 } {
                # The single-net repair rewires whatever net it is given, and ODB refuses that on a dont_touch net
                # (ODB-0372): a held net goes straight to isolation. MEASURED: fabric_soc net218
                # (RUN_2026.10.03_14.06.24), a dioded net whose driver was at maximum drive.
                set added 0
                if { ![[sta::sta_to_db_net $net] isDoNotTouch] } {
                    set n0 [logos_inst_count]
                    rsz::repair_net_cmd $net 0 0 0
                    set added [expr { [logos_inst_count] - $n0 }]
                }
                if { $added > 0 } {
                    incr rebuffered
                    incr changed
                    puts "\[LOGOS-UPSIZE\]   net $nname: driver at maximum drive, rebuffered with $added cell(s)"
                } elseif { [llength $drvs] == 1 && [logos_isolate_driver $net [lindex $drvs 0]] } {
                    incr isolated
                    incr changed
                    puts "\[LOGOS-UPSIZE\]   net $nname: driver at maximum drive and not rebufferable, isolated behind a buffer"
                } elseif { [set added [logos_rebuffer_held $net]] > 0 } {
                    incr rebuffered
                    incr changed
                    puts "\[LOGOS-UPSIZE\]   net $nname: held for its diodes, rebuffered with $added cell(s), every diode on its gate's net"
                } elseif { [llength $drvs] == 1 && [logos_halve_arm $net [lindex $drvs 0]] } {
                    incr halved
                    incr changed
                    puts "\[LOGOS-UPSIZE\]   net $nname: driven at the strongest buffer's drive, its longest arm halved"
                }
            }
        }
        if { $changed == 0 } { break }
        # Legalize what this round placed before the next round measures: an isolation buffer lands on its
        # driver, and legalization moves it and its neighbours, which moves routes. MEASURED on
        # systolic_pipe16_realbist (RUN_2026.09.29_00.40.21): the last round read net _005642_ at 0.488 ns
        # against a 0.540 ns limit; after the closing legalization the same estimate read 0.618 ns, and it
        # signed off at 0.76 ns at the slow corner, max RC.
        logos_diodes_hold
        detailed_placement
        logos_diodes_release
    }
}
logos_slew_rounds

# THE ROUTE DETAILED ROUTING FOLLOWS IS JUDGED FOR ANTENNAS. The global route's own antenna repair judged a route this
# step replaces with its own, so where the recipe repairs antennas (GRT_REPAIR_ANTENNAS), the repair runs again on this
# route, with the recipe's iterations and margin, while placement can still make room for every diode it adds; the
# nets those diodes take over the design limit are split as the fanout step splits them (logos_diode_fanout_lib.tcl),
# timing is repaired with every dioded net held, and the rounds above run again. A pass that adds no diode ends it, and so
# does one whose splits shed every diode it added.
# MEASURED on coproc_core (FO4PROBE2_20261004_0106, this step replayed): the route it handed on carried 178 violations
# at the recipe's 50% margin; detailed routing found 32 violating nets, and the post-route ECO, with no site free on
# a full die, could not place a diode for wire394/A in 1,507 tries.
set antenna_passes 0
set closure_d0 [logos_diode_count [ord::get_db_block]]
if { $logos_route && $::env(GRT_REPAIR_ANTENNAS) == 1 } {
    set block [ord::get_db_block]
    set db [ord::get_db]
    set cap [logos_design_max_fanout]
    set heuristic [expr { [info exists ::env(RUN_HEURISTIC_DIODE_INSERTION)] && $::env(RUN_HEURISTIC_DIODE_INSERTION) == 1 }]
    set_placement_padding -masters $::env(DIODE_CELL) -left $::env(DIODE_PADDING)
    set doomed [list]
    for {set apass 1} {$apass <= 3} {incr apass} {
        global_route -end_incremental
        foreach inst $doomed { odb::dbInst_destroy $inst }
        set doomed [list]
        # The repair names its diodes ANTENNA_<n> from 1, as every earlier repair did: those are renamed by the pass that
        # placed them, ANTENNA_G0_<n> for the global route's own.
        foreach i [$block getInsts] {
            if { [regexp {^ANTENNA_([0-9]+)$} [$i getConstName] -> k] } { $i rename "ANTENNA_G[expr { $apass - 1 }]_$k" }
        }
        # The repair connects its diodes to the nets it judges, and ODB refuses that on a held net (ODB-0373), so this
        # step's holds are released for it and taken again once its diodes are where they belong.
        foreach dbnet $held { $dbnet setDoNotTouch 0 }
        set held [list]
        set logos_held [dict create]
        set d0 [logos_diode_count $block]
        repair_antennas $::env(DIODE_CELL) -iterations $::env(GRT_ANT_ITERS) -ratio_margin $::env(GRT_ANT_MARGIN)
        set added [expr { [logos_diode_count $block] - $d0 }]
        set antenna_passes $apass
        puts "\[LOGOS-UPSIZE\] antenna pass $apass on the route detailed routing follows: $added diode(s)"
        estimate_parasitics -global_routing
        global_route -start_incremental
        if { $added == 0 } {
            logos_hold_dioded
            break
        }
        set fst [dict create split_nets 0 buffers 0 trimmed_heavy 0 trimmed_clock 0 clock_over 0 guarded 0 created [list] touched [list] doomed [list]]
        lassign [logos_fo_candidates $block $cap] candidates pin_slack skipped
        foreach net $candidates {
            logos_fo_split_net $block $db $net $cap $pin_slack $heuristic fst
        }
        set doomed [dict get $fst doomed]
        # A pass whose diodes its splits shed again, and that built nothing, leaves the route as it found it, and the
        # next would add the same diodes: the closure is at its fixed point. MEASURED on coproc_core
        # (RUN_2026.10.03_23.19.59, this step replayed): the third pass added 10 diodes, every one on a clock net at its
        # budget, and its splits shed the 10.
        if { [logos_diode_count $block] <= $d0 && [dict get $fst buffers] == 0 } {
            puts "\[LOGOS-UPSIZE\] antenna pass $apass: its splits shed every diode it added; the route is at its fixed point"
            logos_hold_dioded
            break
        }
        logos_fo_size_touched $block $db [dict get $fst touched] logos_measure
        set_max_transition $_limit [current_design]
        logos_diodes_hold
        detailed_placement
        logos_diodes_release
        logos_fo_held_timing_repair $block logos_measure
        puts "\[LOGOS-UPSIZE\] antenna pass $apass: split [dict get $fst split_nets] net(s) its diodes took over the design limit, with [dict get $fst buffers] buffer(s)"
        logos_hold_dioded
        logos_slew_rounds
    }
}

logos_measure
foreach dbnet $held { $dbnet setDoNotTouch 0 }
set residual_nets [logos_slew_violating_nets]
set residual [llength $residual_nets]
foreach nname $residual_nets {
    set drv [logos_obj [get_pins -quiet -of_objects [get_nets $nname] -filter "direction==output"]]
    set dcell [expr { $drv eq "" ? "" : [logos_obj [get_cells -quiet -of_objects [lindex $drv 0]]] }]
    set what [expr { $dcell eq "" ? "a port" : "[get_full_name [lindex $drv 0]] ([get_property $dcell ref_name])" }]
    puts "\[LOGOS-UPSIZE\]   residual net $nname driven by $what"
}
puts "\[LOGOS-UPSIZE\] upsized $upsized driver(s), rebuffered $rebuffered net(s), isolated $isolated driver(s), halved $halved arm(s), [expr { [logos_diode_count [ord::get_db_block]] - $closure_d0 }] diode(s) kept from $antenna_passes antenna pass(es) (+[expr {[logos_inst_count] - $cells_before}] cells); $residual net(s) still over the detection limit"
set_max_transition $::env(MAX_TRANSITION_CONSTRAINT) [current_design]
set_max_fanout $::env(MAX_FANOUT_CONSTRAINT) [current_design]

logos_diodes_hold
detailed_placement
logos_diodes_release
if { [catch {check_placement -verbose} errmsg] } {
    puts stderr $errmsg
    exit 1
}
set separated [logos_fo_separated [ord::get_db_block]]
set stranded_after [logos_fo_stranded [ord::get_db_block]]
set stranded [llength $stranded_after]
set over [logos_signal_over [ord::get_db_block]]
puts "\[LOGOS-UPSIZE\] diodes off their gate's net: $separated_before -> $separated; diodes left behind: $stranded_before -> $stranded; signal nets over the design limit of [logos_design_max_fanout]: $over_before -> $over"
if { $separated > $separated_before || $stranded > $stranded_before || $over > $over_before } {
    foreach row $stranded_after {
        lassign $row dname nname um
        puts stderr "\[LOGOS-UPSIZE\]   $dname on $nname, $um um from its nearest pin"
    }
    puts stderr "\[LOGOS-UPSIZE\] an edit parted a diode from its gate, left one behind, or put a net over the design fanout limit"
    exit 1
}
global_route -end_incremental
if { [info exists doomed] } { foreach inst $doomed { odb::dbInst_destroy $inst } }
unset_dont_touch_wrapper
write
