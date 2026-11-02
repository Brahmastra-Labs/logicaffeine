# LOGOS clock buffers within the fanout limit (sourced by cts.tcl after the tree is built).
#
# TritonCTS bounds the sinks of a leaf cluster and nothing above it: an H-tree buffer drives every branch below it
# down to the next buffer, and it buffers a branch only where the wire is long enough to need one. MEASURED on
# gpu_simt's placement (RUN_2026.09.23_09.37.32, CTS replayed at a design limit of 9): the tree went six levels
# deep, the last four too short to buffer, and each of the four level-2 buffers drove the sixteen leaf buffers
# under it. Neither branch-point buffering distance, vertex buffers nor forced leaf buffers changed that; a buffer
# spacing short enough to buffer every level did, at that geometry only.
#
# So the bound is kept by construction, after CTS: a clock buffer driving more than the limit is CLONED — the same
# cell, every input on the same net, placed beside it — and each clone takes a compact share of its sinks
# (logos_fo::bins). The stage count to every sink stays what CTS balanced; a parent the clones take over the limit
# is cloned on the next round.
source $::env(SCRIPTS_DIR)/openroad/logos_diode_fanout_core.tcl

proc logos_ck_pin_xy {it} {
    set r [$it getAvgXY]
    if { [lindex $r 0] } { return [lrange $r 1 2] }
    set bb [[$it getInst] getBBox]
    return [list [expr { ([$bb xMin] + [$bb xMax]) / 2 }] [expr { ([$bb yMin] + [$bb yMax]) / 2 }]]
}

# Clone every clock driver over `cap` loads until none is. Returns {clones residual}: the clones placed, and the
# clock nets still over the limit — a net driven by a port, which has no cell to clone.
proc logos_clone_clock_drivers {cap} {
    set block [ord::get_db_block]
    set seq 0
    set clones 0
    for { set round 1 } { $round <= 16 } { incr round } {
        set changed 0
        set residual [list]
        foreach net [$block getNets] {
            if { [$net getSigType] ne "CLOCK" } { continue }
            set drv ""
            set loads [list]
            foreach it [$net getITerms] {
                if { [$it isOutputSignal] } { set drv $it } elseif { [$it isInputSignal] } { lappend loads $it }
            }
            if { [llength $loads] <= $cap } { continue }
            if { $drv eq "" } {
                lappend residual [$net getName]
                continue
            }
            set inst [$drv getInst]
            set master [$inst getMaster]
            set units [list]
            set load_it [dict create]
            set i 0
            foreach l $loads {
                lassign [logos_ck_pin_xy $l] x y
                lappend units [list $i $x $y 1]
                dict set load_it $i $l
                incr i
            }
            set bins [logos_fo::bins $units $cap]
            lassign [logos_ck_pin_xy $drv] sx sy
            set keep -1
            set keep_d 0
            set k 0
            foreach b $bins {
                lassign [logos_fo::centroid $units $b] cx cy
                set d [expr { abs($cx - $sx) + abs($cy - $sy) }]
                if { $keep < 0 || $d < $keep_d } {
                    set keep $k
                    set keep_d $d
                }
                incr k
            }
            set loc [$inst getLocation]
            set k 0
            foreach b $bins {
                if { $k == $keep } {
                    incr k
                    continue
                }
                while { [$block findInst "logos_ckclone_$seq"] ne "NULL" || [$block findNet "logos_ckclone_net_$seq"] ne "NULL" } {
                    incr seq
                }
                set c [odb::dbInst_create $block $master "logos_ckclone_$seq"]
                set cnet [odb::dbNet_create $block "logos_ckclone_net_$seq"]
                $cnet setSigType CLOCK
                incr seq
                $c setLocation [lindex $loc 0] [lindex $loc 1]
                $c setPlacementStatus PLACED
                foreach it [$inst getITerms] {
                    set mt [$it getMTerm]
                    if { [$mt getSigType] ni {SIGNAL CLOCK} } { continue }
                    set cit [$c findITerm [$mt getConstName]]
                    if { [$it isOutputSignal] } {
                        $cit connect $cnet
                    } elseif { [$it getNet] ne "NULL" } {
                        $cit connect [$it getNet]
                    }
                }
                foreach id $b {
                    set l [dict get $load_it $id]
                    $l disconnect
                    $l connect $cnet
                }
                incr clones
                incr changed
                incr k
            }
        }
        if { $changed == 0 } { break }
    }
    return [list $clones $residual]
}
