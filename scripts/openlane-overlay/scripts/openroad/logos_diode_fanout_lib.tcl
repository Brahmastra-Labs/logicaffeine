# LOGOS diode-aware fanout split: what the fanout step (logos_diode_fanout.tcl) does to a net, as procs, so the post-diode
# upsize's antenna closure (logos_post_diode_upsize.tcl) splits the nets its antenna repair takes over the limit by the
# same rules, on the route detailed routing follows.
source $::env(SCRIPTS_DIR)/openroad/logos_diode_fanout_core.tcl
source $::env(SCRIPTS_DIR)/openroad/logos_fanout_limit.tcl

# A cell the flow may place: `DONT_USE_CELLS` carries the process's DRC-excluded cells, and neither `swapMaster`
# nor `dbInst_create` consults it.
proc logos_fo_usable {name} {
    return [expr { ![info exists ::env(DONT_USE_CELLS)] || [lsearch -exact $::env(DONT_USE_CELLS) $name] < 0 }]
}

# OpenSTA answers a lookup that finds no object with the string `NULL`; every quiet lookup below reads it as absent.
proc logos_fo_obj {o} {
    return [expr { $o eq "NULL" ? "" : $o }]
}

# The buffers of a driver's own library the flow may place, weakest first, as {drive name} pairs.
proc logos_fo_buffers {ref} {
    set lib [string range $ref 0 [expr { [string last "__" $ref] + 1 }]]
    set found [list]
    foreach c [get_lib_cells -quiet "*/${lib}buf_*"] {
        set name [get_name $c]
        if { ![logos_fo_usable $name] } { continue }
        if { [regexp "^[string map {. \\.} $lib]buf_(\[0-9\]+)\$" $name -> d] } { lappend found [list $d $name] }
    }
    return [lsort -integer -index 0 [lsort -unique $found]]
}

# A net's driver pin (or ""), its gate input pins and its diode pins, as odb iterms.
proc logos_fo_net_loads {net} {
    set drv ""
    set gates [list]
    set diodes [list]
    foreach it [$net getITerms] {
        if { [$it isOutputSignal] } {
            if { $drv eq "" } { set drv $it }
        } elseif { [$it isInputSignal] } {
            if { [logos_fo_is_diode $it] } { lappend diodes $it } else { lappend gates $it }
        }
    }
    return [list $drv $gates $diodes]
}

# The setup slack through an instance pin, in ns, or "" when STA has none for it.
proc logos_fo_slack {path} {
    set p [logos_fo_obj [sta::find_pin $path]]
    if { $p eq "" } { return "" }
    set s [get_property $p slack_max]
    return [expr { [string is double -strict $s] ? $s : "" }]
}

# ISOLATE a driver already at the strongest drive of its function, as the post-diode upsize does: the library's
# strongest buffer takes over the net and every sink and diode on it, and the driver keeps one short hop to the
# buffer's input — so no load moves and no diode leaves its gate's net. A driver an isolation buffer already is
# gains nothing from a second one. Answers whether it isolated.
proc logos_fo_isolate_driver {block db drv} {
    set inst [$drv getInst]
    set ref [[$inst getMaster] getConstName]
    if { [string match "logos_fo_iso_*" [$inst getConstName]] } { return 0 }
    if { [[$drv getNet] isDoNotTouch] } { return 0 }
    set bufs [logos_fo_buffers $ref]
    if { [llength $bufs] == 0 } { return 0 }
    # A buffer already as strong as the one isolation would add gains a stage and nothing else.
    if { [regexp {__(?:buf|clkbuf|bufbuf)_([0-9]+)$} $ref -> d] && $d >= [lindex $bufs end 0] } { return 0 }
    set master [$db findMaster [lindex $bufs end 1]]
    set net [$drv getNet]
    set n [logos_fo_fresh_name $block logos_fo_iso]
    set b [odb::dbInst_create $block $master "logos_fo_iso_$n"]
    set hop [odb::dbNet_create $block "logos_fo_iso_net_$n"]
    set loc [$inst getLocation]
    $b setLocation [lindex $loc 0] [lindex $loc 1]
    $b setPlacementStatus PLACED
    $drv disconnect
    $drv connect $hop
    foreach mt [$master getMTerms] {
        if { [$mt getSigType] ne "SIGNAL" } { continue }
        set bit [$b findITerm [$mt getConstName]]
        if { [$mt getIoType] eq "INPUT" } { $bit connect $hop } else { $bit connect $net }
    }
    return 1
}

# A buffer's input diode follows the buffer when a level moves the buffer's input to another net.
proc logos_fo_move_guard {block inst net} {
    foreach it [$inst getITerms] {
        if { ![$it isInputSignal] } { continue }
        set d [$block findInst "ANTENNA_[$inst getConstName]_[[$it getMTerm] getConstName]"]
        if { $d eq "NULL" } { continue }
        set dt [$d findITerm $::env(DIODE_CELL_PIN)]
        $dt disconnect
        $dt connect $net
    }
}

proc logos_fo_fresh_name {block stem} {
    global logos_fo_seq
    while { [$block findInst "${stem}_$logos_fo_seq"] ne "NULL" || [$block findNet "${stem}_net_$logos_fo_seq"] ne "NULL" } {
        incr logos_fo_seq
    }
    set n $logos_fo_seq
    incr logos_fo_seq
    return $n
}

# A diode belongs on its gate's net whoever moved it: a pass before this one that splits a net by pins — the
# resizer's single-net repair, which the post-diode upsize calls on a slew violator — can leave a diode on one
# side and its gate on the other. MEASURED on gpu_simt (RUN_2026.10.03_07.11.26, a design limit of 9): four.
# Each returns to its gate's net before anything is counted, so the split below sees the loads as they belong.
proc logos_fo_return_named {block} {
    set repaired 0
    foreach inst [$block getInsts] {
        if { [[$inst getMaster] getConstName] ne $::env(DIODE_CELL) } { continue }
        set o [logos_fo_named_owner $block [$inst getConstName]]
        if { [llength $o] == 0 } { continue }
        set dt [$inst findITerm $::env(DIODE_CELL_PIN)]
        set gnet [[lindex $o 1] getNet]
        if { $dt eq "NULL" || $gnet eq "NULL" || [$dt getNet] eq $gnet } { continue }
        $dt disconnect
        $dt connect $gnet
        incr repaired
    }
    return $repaired
}

# The nets over `cap` with a gate to move, and the setup slack of each gate on a signal one, as {candidates
# pin_slack dont_touch_skipped}; the caller has measured. Every slack is read before any net is edited: an edit invalidates timing, and re-timing after each of thousands
# of nets would cost more than the step itself. It is read on the global-route estimate the flow's timing repair
# judges — the placement estimate ranks differently: on fabric_soc (RUN_2026.10.03_08.06.46) it read the worst
# path at +12.97 ns where the global-route estimate read -2.75.
proc logos_fo_candidates {block cap} {
    set skipped 0
    set candidates [list]
    set pin_slack [dict create]
    foreach net [$block getNets] {
        if { [$net isSpecial] } { continue }
        set st [$net getSigType]
        if { $st eq "POWER" || $st eq "GROUND" } { continue }
        lassign [logos_fo_net_loads $net] drv gates diodes
        set loads [expr { [llength $gates] + [llength $diodes] }]
        if { $loads <= $cap || [llength $gates] == 0 } { continue }
        if { [$net isDoNotTouch] } {
            incr skipped
            puts "\[LOGOS-FANOUT\]   [$net getName]: $loads loads, marked dont_touch, left as it is"
            continue
        }
        lappend candidates $net
        if { [logos_fo_is_clock $net] } { continue }
        foreach g $gates {
            set path "[[$g getInst] getConstName]/[[$g getMTerm] getConstName]"
            set sl [logos_fo_slack $path]
            if { $sl ne "" } { dict set pin_slack $path $sl }
        }
    }
    return [list $candidates $pin_slack $skipped]
}

# Split one net over `cap`: whole units, each a gate with the diodes placed beside it, into buffered bins, the most
# critical bin kept on the driver (logos_diode_fanout_core.tcl), every diode back on the net of the pin it was placed
# beside — the diodes beside the driver stay on the net, loads every bin left there must fit beside. A clock net is
# never split; it sheds the diodes nearest its driver. Counts go to the dict named `statsvar`; the buffers' nets and
# the split net go on its `touched`, the buffers on its `created`, and the diodes it sheds, disconnected, on its
# `doomed`, for the caller to destroy: the global router's incremental session faults on an instance destroyed while
# it is open (GlobalRouter::updateDirtyNets, SIGSEGV; fabric_soc, FO5 replay of the route-mode step).
proc logos_fo_split_net {block db net cap pin_slack heuristic statsvar} {
    upvar $statsvar st
    lassign [logos_fo_net_loads $net] drv gates diodes
    set loads [expr { [llength $gates] + [llength $diodes] }]
    set nname [$net getName]
    if { $drv ne "" } {
        lassign [logos_fo_pin_xy $drv] sx sy
    } else {
        set sx 0
        set sy 0
        foreach bt [$net getBTerms] {
            set r [$bt getFirstPinLocation]
            if { [lindex $r 0] } { lassign [lrange $r 1 2] sx sy; break }
        }
    }

    set gate_rows [list]
    set gate_it [dict create]
    set i 0
    foreach g $gates {
        lassign [logos_fo_pin_xy $g] gx gy
        lappend gate_rows [list $i [[$g getInst] getConstName] [[$g getMTerm] getConstName] $gx $gy]
        dict set gate_it $i $g
        incr i
    }
    set diode_rows [list]
    set diode_it [dict create]
    set j 0
    foreach d $diodes {
        lassign [logos_fo_pin_xy $d] dx dy
        lappend diode_rows [list $j [[$d getInst] getConstName] $dx $dy]
        dict set diode_it $j $d
        incr j
    }
    set owner [logos_fo_owners $block $net]
    set gid_of [dict create]
    dict for {gid g} $gate_it { dict set gid_of $g $gid }
    set owned [dict create]
    foreach gr $gate_rows { dict set owned [lindex $gr 0] [list] }
    set rooted [list]
    foreach dr $diode_rows {
        lassign $dr did name dx dy
        set d [dict get $diode_it $did]
        if { [dict exists $owner $d] && [dict exists $gid_of [dict get $owner $d]] } {
            dict lappend owned [dict get $gid_of [dict get $owner $d]] [list $did $dx $dy]
        } else {
            lappend rooted [list $did $dx $dy]
        }
    }

    if { [logos_fo_is_clock $net] } {
        set units [list]
        foreach gr $gate_rows {
            lassign $gr gid inst pin gx gy
            set ds [list]
            foreach o [dict get $owned $gid] { lappend ds [lindex $o 0] }
            lappend units [list $gid $gx $gy $ds]
        }
        set ds [list]
        foreach o $rooted { lappend ds [lindex $o 0] }
        lappend units [list root $sx $sy $ds]
        set drop [logos_fo::budget_trim $units $sx $sy $loads $cap]
        foreach did $drop {
            [dict get $diode_it $did] disconnect
            dict lappend st doomed [[dict get $diode_it $did] getInst]
            dict incr st trimmed_clock
        }
        if { [llength $drop] > 0 } {
            puts "\[LOGOS-FANOUT\]   clock net $nname: shed [llength $drop] of [llength $diodes] diode(s), [llength $gates] clock pin(s) remain"
        }
        if { [llength $gates] > $cap } { dict incr st clock_over }
        return
    }

    # A net the heuristic pass protected, protected the same way where the split adds gates: each buffer's input
    # takes a diode, so a bin leaving the net costs it two loads.
    set guard [expr { $heuristic && [logos_fo_net_span $block $net] >= $::env(HEURISTIC_ANTENNA_THRESHOLD) }]
    set per_buffer [expr { 1 + $guard }]

    # Every diode leaves the net now and returns to its pin's net once the bins are built.
    foreach d $diodes { $d disconnect }
    set units [list]
    set keep [dict create]
    foreach gr $gate_rows {
        lassign $gr gid inst pin gx gy
        set ds [dict get $owned $gid]
        if { [expr { 1 + [llength $ds] }] > $cap } {
            lassign [logos_fo::trim_unit $ds $gx $gy $cap] kept dropped
            foreach did $dropped {
                dict lappend st doomed [[dict get $diode_it $did] getInst]
                dict incr st trimmed_heavy
            }
            puts "\[LOGOS-FANOUT\]   $nname: gate $inst/$pin carried [llength $ds] diode(s), kept the [llength $kept] nearest"
            set ds_ids $kept
        } else {
            set ds_ids [list]
            foreach o $ds { lappend ds_ids [lindex $o 0] }
        }
        dict set keep $gid $ds_ids
        lappend units [list $gid $gx $gy [expr { 1 + [llength $ds_ids] }]]
    }
    # The diodes beside the driver leave room for one buffer's input; those nearest it stay.
    set room [expr { $cap - $per_buffer }]
    if { [llength $rooted] > $room } {
        lassign [logos_fo::trim_unit $rooted $sx $sy [expr { $room + 1 }]] root_keep dropped
        foreach did $dropped {
            dict lappend st doomed [[dict get $diode_it $did] getInst]
            dict incr st trimmed_heavy
        }
        puts "\[LOGOS-FANOUT\]   $nname: its driver carried [llength $rooted] diode(s), kept the [llength $root_keep] nearest"
    } else {
        set root_keep [list]
        foreach o $rooted { lappend root_keep [lindex $o 0] }
    }
    set fixed [llength $root_keep]

    set slack [dict create]
    foreach gr $gate_rows {
        lassign $gr gid inst pin
        if { [dict exists $pin_slack "$inst/$pin"] } { dict set slack $gid [dict get $pin_slack "$inst/$pin"] }
    }
    set bins [logos_fo::bins $units $cap]
    set kept_bin [logos_fo::kept_bin $bins $units $slack $sx $sy $cap $per_buffer $fixed]
    if { $kept_bin < 0 } {
        lassign [logos_fo::carve $units $slack $sx $sy $cap $per_buffer $fixed] carved rest_bins
        if { [llength $carved] > 0 } {
            set bins [concat [list $carved] $rest_bins]
            set kept_bin 0
        }
    }
    set bufs [logos_fo_buffers [expr { $drv ne "" ? [[[$drv getInst] getMaster] getConstName] : $::env(DIODE_CELL) }]]
    if { [llength $bufs] == 0 } {
        puts stderr "\[LOGOS-FANOUT\] $nname: no buffer the flow may place in its library"
        exit 1
    }
    set start [lindex $bufs 0 1]
    foreach b $bufs { if { [lindex $b 0] >= 4 } { set start [lindex $b 1]; break } }
    set master [$db findMaster $start]

    # One buffer per bin that leaves the net, at its bin's centroid; the net keeps the most critical bin when that
    # still fits. Each buffer's input is guarded as the heuristic pass guards the net's other inputs. Both halves are
    # MEASURED. Unguarded, buffer logos_fo_92 on fabric_horner_seq (RUN_2026.10.03_14.54.29) took a met3 input wire
    # 2.12x over the antenna limit that neither the jog nor a diode could fix. Moved to the driver instead, each bin
    # became its own long wire from one point: on systolic_os_pipe8_realbist (RUN_2026.10.03_15.52.08) the step ended
    # at -1.888 ns worst setup with 1,884 drivers grown, against -0.092 ns and 512 at the centroid.
    set level_units [list]
    set k 0
    foreach b $bins {
        if { $k == $kept_bin } {
            incr k
            continue
        }
        set n [logos_fo_fresh_name $block logos_fo]
        set inst [odb::dbInst_create $block $master "logos_fo_$n"]
        set bnet [odb::dbNet_create $block "logos_fo_net_$n"]
        lassign [logos_fo::centroid $units $b] cx cy
        $inst setLocation $cx $cy
        $inst setPlacementStatus PLACED
        foreach mt [$master getMTerms] {
            if { [$mt getSigType] ne "SIGNAL" } { continue }
            set bit [$inst findITerm [$mt getConstName]]
            if { [$mt getIoType] eq "INPUT" } { $bit connect $net } else { $bit connect $bnet }
        }
        if { $guard } { dict incr st guarded [logos_fo_guard_input $block $db $inst] }
        foreach gid $b {
            set g [dict get $gate_it $gid]
            $g disconnect
            $g connect $bnet
        }
        dict lappend st created [$inst getConstName]
        dict lappend st touched [$bnet getConstName]
        lappend level_units [list $n $cx $cy $per_buffer]
        dict incr st buffers
        incr k
    }

    foreach gr $gate_rows {
        set gid [lindex $gr 0]
        set g [dict get $gate_it $gid]
        foreach did [dict get $keep $gid] {
            set d [dict get $diode_it $did]
            $d connect [$g getNet]
        }
    }
    foreach did $root_keep { [dict get $diode_it $did] connect $net }

    # A net so wide that its bins' buffers alone overflow it takes another level, built the same way.
    set parent_w 0
    foreach it [$net getITerms] { if { [$it isInputSignal] } { incr parent_w } }
    while { $parent_w > $cap && [llength $level_units] > 1 } {
        set lbins [logos_fo::bins $level_units $cap]
        set next [list]
        foreach b $lbins {
            set n [logos_fo_fresh_name $block logos_fo]
            set inst [odb::dbInst_create $block $master "logos_fo_$n"]
            set bnet [odb::dbNet_create $block "logos_fo_net_$n"]
            lassign [logos_fo::centroid $level_units $b] cx cy
            $inst setLocation $cx $cy
            $inst setPlacementStatus PLACED
            foreach mt [$master getMTerms] {
                if { [$mt getSigType] ne "SIGNAL" } { continue }
                set bit [$inst findITerm [$mt getConstName]]
                if { [$mt getIoType] eq "INPUT" } { $bit connect $net } else { $bit connect $bnet }
            }
            if { $guard } { dict incr st guarded [logos_fo_guard_input $block $db $inst] }
            foreach lid $b {
                set child [$block findInst "logos_fo_$lid"]
                foreach mt [[$child getMaster] getMTerms] {
                    if { [$mt getSigType] eq "SIGNAL" && [$mt getIoType] eq "INPUT" } {
                        set cit [$child findITerm [$mt getConstName]]
                        $cit disconnect
                        $cit connect $bnet
                    }
                }
                logos_fo_move_guard $block $child $bnet
            }
            dict lappend st created [$inst getConstName]
            dict lappend st touched [$bnet getConstName]
            lappend next [list $n $cx $cy $per_buffer]
            dict incr st buffers
        }
        set level_units $next
        set parent_w 0
        foreach it [$net getITerms] { if { [$it isInputSignal] } { incr parent_w } }
    }

    dict lappend st touched $nname
    dict incr st split_nets
}

# The buffers start at drive 4. Size them, and the drivers of the nets they now hang on, for the load each
# drives: a round measures with `measure`, grows every touched driver whose net violates max slew
# against the post-diode upsize's detection limit, and legalizes before the next round measures.
proc logos_fo_size_touched {block db touched measure} {
    set _m 52
    if { [info exists ::env(LOGOS_UPSIZE_MARGIN)] } { set _m $::env(LOGOS_UPSIZE_MARGIN) }
    set _detect [expr { $::env(MAX_TRANSITION_CONSTRAINT) * (100 - $_m) / 100.0 }]
    set grown 0
    set isolated 0
    if { [llength $touched] > 0 } {
        detailed_placement
        set want [dict create]
        foreach t $touched { dict set want $t 1 }
        set_max_transition $_detect [current_design]
        for {set round 1} {$round <= 12} {incr round} {
            $measure
            set changed 0
            set seen [dict create]
            foreach p [sta::check_slew_limits "NULL" 1 "NULL" max] {
                set n [logos_fo_obj [get_nets -quiet -of_objects $p]]
                if { $n eq "" } { continue }
                set name [get_full_name $n]
                if { ![dict exists $want $name] || [dict exists $seen $name] } { continue }
                dict set seen $name 1
                set dbnet [$block findNet $name]
                if { $dbnet eq "NULL" } { continue }
                foreach it [$dbnet getITerms] {
                    if { ![$it isOutputSignal] } { continue }
                    set inst [$it getInst]
                    set ref [[$inst getMaster] getConstName]
                    if { [string match "*diode*" $ref] || [string match "*dlygate*" $ref] || [$inst isDoNotTouch] } { continue }
                    if { ![regexp {^(.*_)([0-9]+)$} $ref -> stem drive] } { continue }
                    set up ""
                    set up_d 0
                    foreach c [get_lib_cells -quiet "*/${stem}*"] {
                        set cname [get_name $c]
                        if { ![logos_fo_usable $cname] } { continue }
                        if { [regexp "^[string map {. \\.} $stem](\[0-9\]+)\$" $cname -> d] && $d > $drive
                             && ($up eq "" || $d < $up_d) } {
                            set up $cname
                            set up_d $d
                        }
                    }
                    if { $up eq "" } {
                        if { [logos_fo_isolate_driver $block $db $it] } {
                            incr isolated
                            incr changed
                        }
                        continue
                    }
                    if { [$inst swapMaster [$db findMaster $up]] } {
                        incr grown
                        incr changed
                    }
                }
            }
            puts "\[LOGOS-FANOUT\] sizing round $round: $changed driver(s) grown or isolated"
            if { $changed == 0 } { break }
            detailed_placement
        }
        set_max_transition $::env(MAX_TRANSITION_CONSTRAINT) [current_design]
    }
    return [list $grown $isolated]
}

# Timing, repaired as the flow repairs it, after every edit this step and the passes before it made — the diodes,
# the post-diode upsize's isolation buffers, this step's buffers; the flow's own repair ran before all of them.
# Its setup and then its hold repair run with the routing stage's margins and budgets, every net carrying a diode
# held (dont_touch) so neither splits one, and without pin swapping, which moves a net onto another pin of the
# same gate: no diode leaves its gate's net. MEASURED on gpu_simt (RUN_2026.10.03_07.23.27): the slow-corner
# critical path picked up an isolation buf_12 (0.30 ns) after the flow's repair and signed off at -0.01 ns. A
# repair that may not buffer at all is not enough (-0.27 -> -0.06 ns there by sizing alone), and one that may
# buffer anywhere split four of fabric_soc's dioded nets (RUN_2026.10.03_08.06.46).
# The setup repair works every violating endpoint (`-repair_tns 100`), not only the worst: this step slows many paths
# at once, and a worst-endpoint repair stops where that one endpoint stops improving. MEASURED on soft_core_realbist
# (RUN_2026.10.03_22.32.03, this step replayed): -0.871 -> +0.016 ns worst endpoint only, short of the margin, and
# the block signed off at -0.00 ns at the slowest corner; -0.871 -> +0.061 ns every endpoint, in the same 21 s, and
# 193 s against 176 s on systolic_pipe16_realbist.
# It works every endpoint only while the design is within reach of its clock — its worst slack less than a clock period
# short — and the worst endpoint beyond: a run missing by more needs a longer period (signoff_blocks' honest_period_ns
# gives it), which no repair supplies. MEASURED on qorsyn_k2_logos_p10 (RUN_2026.10.17_00.12.39, this step replayed from
# its state before this repair): at -28.9 ns against a 10 ns clock the every-endpoint repair rebuffered net after net
# until RepairSetup::rebuffer -> makeBufferedNetGroute reached a net whose global route has a loop, and OpenROAD
# 41a51eaf's rsz::makeBufferedNet, which walks a route into a buffer tree by recursion with no visited set, recursed
# 10,450 frames and overflowed the stack; the worst-endpoint repair of the same state completed (-28.9 -> -27.7 ns,
# 7 buffers, 39 resizes). The guard lowers that exposure, not to zero: a design within reach can still meet a looped
# route, and the step then exits 139 and the flow fails loudly, never signing anything off. This OpenROAD's
# repair_timing has no switch that leaves rebuffering out, so the endpoint scope is the narrowest lever.
# It runs where the flow's own global-route timing repair runs, and nowhere else: a design that turns that off
# (the Caravel wrapper, a routing shell whose buffering was measured to cost 517,296 cells and DRC) keeps it off.
proc logos_fo_held_timing_repair {block measure} {
    if { $::env(GLB_RESIZER_TIMING_OPTIMIZATIONS) != 1 } {
        puts "\[LOGOS-FANOUT\] no timing repair: the flow's global-route timing optimizations are off"
        return
    }
    $measure
    set wns_pre [expr { [sta::worst_slack_cmd max] * 1e9 }]
    set held [list]
    foreach net [$block getNets] {
        if { [$net isSpecial] || [$net isDoNotTouch] } { continue }
        foreach it [$net getITerms] {
            if { [$it isInputSignal] && [logos_fo_is_diode $it] } {
                $net setDoNotTouch 1
                lappend held $net
                break
            }
        }
    }
    repair_timing -setup -setup_margin $::env(GLB_RESIZER_SETUP_SLACK_MARGIN) \
        {*}[expr { $wns_pre > -$::env(CLOCK_PERIOD) ? {-repair_tns 100} : {} }] \
        -max_buffer_percent $::env(GLB_RESIZER_SETUP_MAX_BUFFER_PERCENT) -skip_pin_swap
    set hold_args [list -hold -setup_margin $::env(GLB_RESIZER_SETUP_SLACK_MARGIN) \
        -hold_margin $::env(GLB_RESIZER_HOLD_SLACK_MARGIN) -max_buffer_percent $::env(GLB_RESIZER_HOLD_MAX_BUFFER_PERCENT) -skip_pin_swap]
    if { $::env(GLB_RESIZER_ALLOW_SETUP_VIOS) == 1 } { lappend hold_args -allow_setup_violations }
    repair_timing {*}$hold_args
    foreach net $held { $net setDoNotTouch 0 }
    puts "\[LOGOS-FANOUT\] timing repair with [llength $held] dioded net(s) held: worst setup slack [format %.3f $wns_pre] -> [format %.3f [expr { [sta::worst_slack_cmd max] * 1e9 }]] ns"
}

# Every diode must still be on the net of the gate it was placed for.
proc logos_fo_separated {block} {
    set separated 0
    foreach inst [$block getInsts] {
        if { [[$inst getMaster] getConstName] ne $::env(DIODE_CELL) } { continue }
        set name [$inst getConstName]
        set o [logos_fo_named_owner $block $name]
        if { [llength $o] == 0 } { continue }
        set dt [$inst findITerm $::env(DIODE_CELL_PIN)]
        if { $dt eq "NULL" } { continue }
        if { [[lindex $o 1] getNet] ne [$dt getNet] } {
            incr separated
            puts "\[LOGOS-FANOUT\]   diode $name is not on its gate's net"
        }
    }
    return $separated
}
