# LOGOS design fanout limit: LOGOS_FANOUT_MARGIN loads (1 unless set; 0 designs at the limit) under the SDC's
# set_max_fanout, which every report and signoff judge unchanged.
#
# The margin is room for the one load a pass after routing can add. The post-route antenna ECO adds a diode at a
# pin the checker still flags, and a net built to the limit has nowhere to put it — MEASURED on gpu_simt
# (RUN_2026.10.03_07.00.23): pin _13114_/C, met1 ratio 1.02, on a net of ten gates and no diode, refused, and the
# block missed antenna by that one pin.
#
# It is kept by the steps that know which loads are critical — CTS (cts.tcl) and the diode-aware fanout step
# (logos_diode_fanout.tcl), which keeps a net's least-slack gates on its driver — and NOT by the SDC the resizers
# repair against. MEASURED, the SDC way (RUN_2026.10.03_07.23.27): repair_design split every ten-load net without
# regard to slack, the slow-corner critical path went from 2 to 6 fanout-buffer pins, and setup slack from +0.27
# to -0.01 ns, beyond what a full setup repair afterwards could recover (-0.013 ns at the estimate, no cell added).
proc logos_design_max_fanout {} {
    set margin 1
    if { [info exists ::env(LOGOS_FANOUT_MARGIN)] } { set margin $::env(LOGOS_FANOUT_MARGIN) }
    return [expr { int(round($::env(MAX_FANOUT_CONSTRAINT))) - $margin }]
}

# The diode rules every step that keeps the limit shares (logos_diode_fanout.tcl, logos_post_diode_upsize.tcl): a
# diode protects the pin it was placed beside only while it is on that pin's net, so these name that pin, count a
# net's loads as the liberty counts them, find a diode an edit left behind, and guard a new input the way the
# heuristic diode pass guards a net's.
source $::env(SCRIPTS_DIR)/openroad/logos_diode_fanout_core.tcl

proc logos_fo_pin_xy {it} {
    set r [$it getAvgXY]
    if { [lindex $r 0] } { return [lrange $r 1 2] }
    set bb [[$it getInst] getBBox]
    return [list [expr { ([$bb xMin] + [$bb xMax]) / 2 }] [expr { ([$bb yMin] + [$bb yMax]) / 2 }]]
}

proc logos_fo_is_diode {it} {
    return [expr { [[[$it getInst] getMaster] getConstName] eq $::env(DIODE_CELL)
                   && [[$it getMTerm] getConstName] eq $::env(DIODE_CELL_PIN) }]
}

# A clock net: one CTS built, which it marks CLOCK, or one reaching a clock pin. Its buffers are CTS's to place.
# A driver's cell says nothing: synthesis and the resizer buffer data with clock buffers too — on gpu_simt
# (RUN_2026.10.03_06.46.47) 1,682 SIGNAL nets are driven by clkbuf cells into mux and logic inputs, beside the 318
# CLOCK nets of the tree, and a test on the driver's name stripped twenty of those data nets of their diodes.
proc logos_fo_is_clock {net} {
    if { [$net getSigType] eq "CLOCK" } { return 1 }
    foreach it [$net getITerms] {
        if { [$it isInputSignal] && [[$it getMTerm] getSigType] eq "CLOCK" } { return 1 }
    }
    return 0
}

# The gate a diode was named for, as {inst iterm}, or {} when its name is not ANTENNA_[ECO_]<inst>_<pin> of a pin
# that exists. Instance and pin names may both contain underscores, so every split point is tried from the right.
proc logos_fo_named_owner {block name} {
    if { [string match "ANTENNA_ECO_*" $name] } {
        set rest [string range $name 12 end]
    } elseif { [string match "ANTENNA_*" $name] } {
        set rest [string range $name 8 end]
    } else {
        return [list]
    }
    for { set k [string last "_" $rest] } { $k > 0 } { set k [string last "_" [string range $rest 0 [expr { $k - 1 }]]] } {
        set oi [$block findInst [string range $rest 0 [expr { $k - 1 }]]]
        if { $oi eq "NULL" } { continue }
        set ot [$oi findITerm [string range $rest [expr { $k + 1 }] end]]
        if { $ot ne "NULL" } { return [list $oi $ot] }
    }
    return [list]
}

# The pin each diode on a net was placed beside, as a dict from diode iterm to that pin: the gate its name gives,
# wherever that gate now is, else the nearest of the net's other pins, its driver among them — an iterm, or the
# port's bterm (logos_fo::pair). A diode beside the driver stays on the driver's net whatever an edit does to the
# gates. MEASURED on coproc_core (RUN_2026.10.03_23.19.59, the route-mode step replayed): paired with gates alone, the
# two diodes global routing put beside the driver of _28909_ followed its one gate through five halvings and hung
# 650 um from every pin of logos_arm_net_11, whose route they took to 2,084 um; it stayed over the slew limit.
proc logos_fo_owners {block net} {
    set owner [dict create]
    set rows [list]
    set drows [list]
    set pin_of [dict create]
    set diode_of [dict create]
    set i 0
    foreach it [$net getITerms] {
        if { [logos_fo_is_diode $it] } {
            set name [[$it getInst] getConstName]
            set o [logos_fo_named_owner $block $name]
            if { [llength $o] == 2 } {
                dict set owner $it [lindex $o 1]
                continue
            }
            lassign [logos_fo_pin_xy $it] x y
            lappend drows [list $i $name $x $y]
            dict set diode_of $i $it
        } elseif { [$it isInputSignal] || [$it isOutputSignal] } {
            lassign [logos_fo_pin_xy $it] x y
            lappend rows [list $i [[$it getInst] getConstName] [[$it getMTerm] getConstName] $x $y]
            dict set pin_of $i $it
        }
        incr i
    }
    foreach bt [$net getBTerms] {
        set r [$bt getFirstPinLocation]
        if { ![lindex $r 0] } { continue }
        lappend rows [list $i "" [$bt getName] [lindex $r 1] [lindex $r 2]]
        dict set pin_of $i $bt
        incr i
    }
    dict for {did pid} [logos_fo::pair $rows $drows] {
        dict set owner [dict get $diode_of $did] [dict get $pin_of $pid]
    }
    return $owner
}

# The diodes an edit left behind: each farther than 100 um from every other pin of its net, as {diode net um}. A
# diode goes in beside the pin it protects — MEASURED after global routing's repair on coproc_core and fabric_soc
# (RUN_2026.10.03_23.19.59, _22.44.27), 18,626 of the 18,632 diodes on their nets lay within 37 um of the nearest
# pin — and the other six 219 to 275 um out, on nets the fanout step had split.
proc logos_fo_stranded {block} {
    set lim [expr { 100 * [$block getDbUnitsPerMicron] }]
    set out [list]
    foreach net [$block getNets] {
        if { [$net isSpecial] } { continue }
        set st [$net getSigType]
        if { $st eq "POWER" || $st eq "GROUND" } { continue }
        set pins [list]
        set diodes [list]
        foreach it [$net getITerms] {
            if { [logos_fo_is_diode $it] } { lappend diodes $it } else { lappend pins [logos_fo_pin_xy $it] }
        }
        if { [llength $diodes] == 0 } { continue }
        foreach bt [$net getBTerms] {
            set r [$bt getFirstPinLocation]
            if { [lindex $r 0] } { lappend pins [lrange $r 1 2] }
        }
        foreach d $diodes {
            lassign [logos_fo_pin_xy $d] x y
            set near -1
            foreach pq $pins {
                lassign $pq px py
                set dd [expr { abs($px - $x) + abs($py - $y) }]
                if { $near < 0 || $dd < $near } { set near $dd }
            }
            if { $near < 0 || $near > $lim } {
                lappend out [list [[$d getInst] getConstName] [$net getName] \
                    [expr { $near < 0 ? "-" : round($near / double([$block getDbUnitsPerMicron])) }]]
            }
        }
    }
    return $out
}

# The nets over `cap` loads, counted as the diode-aware steps count them — every input pin, diodes included — as
# {name is_clock} pairs. STA's own count judges the SDC's limit, not the design limit these steps build to.
proc logos_fo_over {block cap} {
    set over [list]
    foreach net [$block getNets] {
        if { [$net isSpecial] } { continue }
        set st [$net getSigType]
        if { $st eq "POWER" || $st eq "GROUND" } { continue }
        set n 0
        foreach it [$net getITerms] { if { [$it isInputSignal] } { incr n } }
        if { $n > $cap } { lappend over [list [$net getName] [logos_fo_is_clock $net]] }
    }
    return $over
}

# The heuristic diode pass protects every input of a net whose pins span HEURISTIC_ANTENNA_THRESHOLD or more (their
# half-perimeter, as diodes.py measures it). A split or a repeater adds gates — its buffers' inputs — after that pass
# has run.
proc logos_fo_net_span {block net} {
    set xs [list]
    set ys [list]
    foreach it [$net getITerms] {
        lassign [logos_fo_pin_xy $it] x y
        lappend xs $x
        lappend ys $y
    }
    foreach bt [$net getBTerms] {
        set r [$bt getFirstPinLocation]
        if { [lindex $r 0] } {
            lappend xs [lindex $r 1]
            lappend ys [lindex $r 2]
        }
    }
    if { [llength $xs] == 0 } { return 0.0 }
    set xs [lsort -integer $xs]
    set ys [lsort -integer $ys]
    return [expr { double([lindex $xs end] - [lindex $xs 0] + [lindex $ys end] - [lindex $ys 0]) / [$block getDbUnitsPerMicron] }]
}

# So a buffer whose input lands on such a net gets the diode that pass would have given it: on its input's net, beside
# it, named as the pass names its own, so every re-pair and separation check pairs it with the buffer.
proc logos_fo_guard_input {block db inst} {
    set dm [$db findMaster $::env(DIODE_CELL)]
    if { $dm eq "NULL" } { return 0 }
    foreach it [$inst getITerms] {
        if { ![$it isInputSignal] || [$it getNet] eq "NULL" } { continue }
        set name "ANTENNA_[$inst getConstName]_[[$it getMTerm] getConstName]"
        if { [$block findInst $name] ne "NULL" } { return 0 }
        set d [odb::dbInst_create $block $dm $name]
        set loc [$inst getLocation]
        $d setLocation [lindex $loc 0] [lindex $loc 1]
        $d setPlacementStatus PLACED
        [$d findITerm $::env(DIODE_CELL_PIN)] connect [$it getNet]
        return 1
    }
    return 0
}
