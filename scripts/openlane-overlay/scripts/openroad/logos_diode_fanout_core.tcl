# LOGOS diode-aware fanout planning: the geometry of the step in logos_diode_fanout.tcl, in plain Tcl so it runs
# (and is tested) without OpenROAD. Tcl 8.5: the image's tclsh is 8.5.13.
#
# A protection diode is a load on its net — sky130's liberty gives the DIODE pin the library's default fanout load
# of 1 — but it protects one gate: the one it was placed beside. So the unit of fanout is a GATE WITH ITS DIODES,
# weight 1 + its diode count, and a net is split by grouping whole units. The resizer's own single-net repair
# groups pins, not units: replayed on gpu_simt's post-diode placement (RUN_2026.09.23_09.37.32, step 21) it
# cleared all 148 signal violators and moved 295 of 1,907 diodes onto a different net than the gate each was
# placed for.
#
# Positions are integers in database units. Every result is deterministic: ties break by id.

namespace eval logos_fo {}

# The pin a diode was placed beside. The heuristic pass and the ECO name a diode after its gate's pin
# (ANTENNA_<inst>_<pin>, ANTENNA_ECO_<inst>_<pin>); global routing's repair names it ANTENNA_<n> and places one beside
# every pin of the wire component it judges, the driver and the diodes already there among them
# (ant::AntennaChecker::findWireRootIterms), so an unnamed diode belongs to the nearest pin it is given, and a caller
# gives the driver beside the gates. MEASURED on coproc_core (RUN_2026.10.03_23.19.59): of the 12 diodes the repair
# put on net _29509_, 4 sat beside its driver and 8 beside its one gate and that gate's own diode.
#   gates:  {id inst pin x y} ...      diodes: {id name x y} ...
# Returns a dict diode id -> gate id. A net with diodes and no pin has nothing to pair them with: empty dict.
proc logos_fo::pair {gates diodes} {
    set by_name [dict create]
    foreach g $gates {
        lassign $g gid inst pin
        dict set by_name "ANTENNA_${inst}_${pin}" $gid
        dict set by_name "ANTENNA_ECO_${inst}_${pin}" $gid
    }
    set owner [dict create]
    if { [llength $gates] == 0 } { return $owner }
    foreach d [lsort -index 0 $diodes] {
        lassign $d did name dx dy
        if { [dict exists $by_name $name] } {
            dict set owner $did [dict get $by_name $name]
            continue
        }
        set best ""
        set best_d 0
        foreach g [lsort -index 0 $gates] {
            lassign $g gid inst pin gx gy
            set dist [expr { abs($gx - $dx) + abs($gy - $dy) }]
            if { $best eq "" || $dist < $best_d } {
                set best $gid
                set best_d $dist
            }
        }
        dict set owner $did $best
    }
    return $owner
}

# The diodes a gate keeps when it alone would weigh more than `cap`: the `cap - 1` nearest to it. Global
# routing's repair adds one diode per iteration until the estimate passes or its iteration count runs out —
# measured at 26 on one wpe_user_project gate and 22 on dram_user_project's single-sink nets — so a pin can carry
# more diodes than any net may hold. What it drops, the post-route antenna check judges again on the real route.
#   diodes: {id x y} ...   gate at (gx, gy)
# Returns {kept dropped}, each a list of diode ids.
proc logos_fo::trim_unit {diodes gx gy cap} {
    set ranked [list]
    foreach d [lsort -index 0 $diodes] {
        lassign $d did dx dy
        lappend ranked [list $did [expr { abs($gx - $dx) + abs($gy - $dy) }]]
    }
    set ranked [lsort -integer -index 1 $ranked]
    set keep [expr { $cap - 1 }]
    if { $keep < 0 } { set keep 0 }
    set kept [list]
    set dropped [list]
    foreach r $ranked {
        if { [llength $kept] < $keep } { lappend kept [lindex $r 0] } else { lappend dropped [lindex $r 0] }
    }
    return [list $kept $dropped]
}

# The diodes a net that cannot be split sheds to come within `budget` loads: a clock net, whose tree is CTS's.
# Diodes go first from the gates nearest the driver, whose wire to the driver is shortest; each such gate sheds
# one diode per pass, so no gate is stripped while a nearer one still has two.
#   units: {gate_id x y {diode_id ...}} ...   driver at (sx, sy)   loads: the net's load count now
# Returns the list of diode ids to remove.
proc logos_fo::budget_trim {units sx sy loads budget} {
    set excess [expr { $loads - $budget }]
    set drop [list]
    if { $excess <= 0 } { return $drop }
    set ranked [list]
    foreach u [lsort -index 0 $units] {
        lassign $u gid x y ds
        if { [llength $ds] == 0 } { continue }
        lappend ranked [list $gid [expr { abs($x - $sx) + abs($y - $sy) }] [lsort $ds]]
    }
    set ranked [lsort -integer -index 1 $ranked]
    while { $excess > 0 } {
        set progressed 0
        set next [list]
        foreach r $ranked {
            lassign $r gid dist ds
            if { $excess > 0 && [llength $ds] > 0 } {
                lappend drop [lindex $ds end]
                set ds [lrange $ds 0 end-1]
                incr excess -1
                set progressed 1
            }
            lappend next [list $gid $dist $ds]
        }
        set ranked $next
        if { !$progressed } { break }
    }
    return $drop
}

proc logos_fo::weight {units} {
    set w 0
    foreach u $units { incr w [lindex $u 3] }
    return $w
}

proc logos_fo::ceil_div {a b} {
    return [expr { ($a + $b - 1) / $b }]
}

# First-fit-decreasing: the units heaviest first, each into the earliest bin with room for it. Units of equal
# weight go in the given order, so a spatially sorted input fills each bin with neighbours.
#   units: {id x y w} ...
# Returns a list of bins, each a sorted list of unit ids.
proc logos_fo::ffd_pack {units cap} {
    set order [list]
    set i 0
    foreach u $units {
        lappend order [list $i [lindex $u 3] [lindex $u 0]]
        incr i
    }
    set order [lsort -integer -decreasing -index 1 [lsort -integer -index 0 $order]]
    set room [list]
    set members [list]
    foreach o $order {
        lassign $o pos w id
        set placed 0
        set b 0
        foreach r $room {
            if { $r >= $w } {
                lset room $b [expr { $r - $w }]
                lset members $b [concat [lindex $members $b] [list $id]]
                set placed 1
                break
            }
            incr b
        }
        if { !$placed } {
            lappend room [expr { $cap - $w }]
            lappend members [list $id]
        }
    }
    set out [list]
    foreach m $members { lappend out [lsort $m] }
    return $out
}

proc logos_fo::ffd_count {weights cap} {
    set units [list]
    set i 0
    foreach w $weights {
        lappend units [list $i 0 0 $w]
        incr i
    }
    return [llength [logos_fo::ffd_pack $units $cap]]
}

# Group units into bins of total weight at most `cap`, each bin spatially compact, in as few bins as packing the
# weights allows. A set that first-fit-decreasing packs into n bins is cut across its longer extent where each
# side still packs into its share — ceil(n/2) on the left, the rest on the right — at the cut nearest the
# proportional split. Where no straight cut keeps that count (weights that only pack across each other: 5 5 4 4
# into two bins of 9), a set of four bins or fewer is packed first-fit-decreasing in its spatial order, and a
# larger one is cut where its two sides need the fewest bins between them.
#   units: {id x y w} ...   every w <= cap
# Returns a list of bins, each a list of unit ids.
proc logos_fo::bins {units cap} {
    if { $cap < 1 } { error "logos_fo::bins: cap $cap" }
    foreach u $units {
        if { [lindex $u 3] > $cap } { error "logos_fo::bins: unit [lindex $u 0] weighs [lindex $u 3], over the cap $cap" }
    }
    if { [llength $units] == 0 } { return [list] }
    set total [logos_fo::weight $units]
    if { $total <= $cap } {
        set ids [list]
        foreach u [lsort -index 0 $units] { lappend ids [lindex $u 0] }
        return [list $ids]
    }
    set xs [list]
    set ys [list]
    set ws [list]
    foreach u $units {
        lappend xs [lindex $u 1]
        lappend ys [lindex $u 2]
        lappend ws [lindex $u 3]
    }
    set xs [lsort -integer $xs]
    set ys [lsort -integer $ys]
    set axis [expr { ([lindex $xs end] - [lindex $xs 0]) >= ([lindex $ys end] - [lindex $ys 0]) ? 1 : 2 }]
    set sorted [lsort -integer -index $axis [lsort -index 0 $units]]
    set n [logos_fo::ffd_count $ws $cap]
    set n1 [logos_fo::ceil_div $n 2]
    set n2 [expr { $n - $n1 }]
    set target [expr { double($total) * $n1 / $n }]
    set best -1
    set best_gap 0
    set fewest -1
    set fewest_sum 0
    set fewest_gap 0
    set acc 0
    set left_ws [list]
    for { set k 1 } { $k < [llength $sorted] } { incr k } {
        set w [lindex $sorted [expr { $k - 1 }] 3]
        incr acc $w
        lappend left_ws $w
        set right_ws [list]
        foreach u [lrange $sorted $k end] { lappend right_ws [lindex $u 3] }
        set fl [logos_fo::ffd_count $left_ws $cap]
        set fr [logos_fo::ffd_count $right_ws $cap]
        set gap [expr { abs($acc - $target) }]
        if { $fl <= $n1 && $fr <= $n2 && ($best < 0 || $gap < $best_gap) } {
            set best $k
            set best_gap $gap
        }
        set sum [expr { $fl + $fr }]
        if { $fewest < 0 || $sum < $fewest_sum || ($sum == $fewest_sum && $gap < $fewest_gap) } {
            set fewest $k
            set fewest_sum $sum
            set fewest_gap $gap
        }
    }
    if { $best < 0 } {
        if { $n <= 4 } { return [logos_fo::ffd_pack $sorted $cap] }
        set best $fewest
    }
    set left [lrange $sorted 0 [expr { $best - 1 }]]
    set right [lrange $sorted $best end]
    return [concat [logos_fo::bins $left $cap] [logos_fo::bins $right $cap]]
}

# The weight-centred position of a bin. units: {id x y w} ...; ids: the bin.
proc logos_fo::centroid {units ids} {
    set want [dict create]
    foreach id $ids { dict set want $id 1 }
    set sx 0
    set sy 0
    set n 0
    foreach u $units {
        lassign $u id x y w
        if { ![dict exists $want $id] } { continue }
        set sx [expr { $sx + $x * $w }]
        set sy [expr { $sy + $y * $w }]
        incr n $w
    }
    if { $n == 0 } { return [list 0 0] }
    return [list [expr { $sx / $n }] [expr { $sy / $n }]]
}

# Which bin, if any, stays on the original net instead of taking a buffer, provided the net then still fits — its
# own weight plus `per_buffer` loads for every other bin (the buffer's input, and the diode it carries when it has
# one) plus the `fixed` loads that stay on the net whatever is split, the diodes beside its driver. The bin holding the gate of least setup slack stays, so
# the most critical path through the net gains no buffer; among bins equally critical, the one nearest the driver.
# MEASURED on gpu_simt (RUN_2026.10.03_06.46.47): keeping the nearest bin put a buf_4 on the critical path, 0.39 ns
# at the slow corner, and worst setup slack fell 0.27 -> 0.12 ns.
#   bins: list of id lists   units: {id x y w} ...   slack: dict id -> setup slack, a missing id uncritical
#   driver at (sx, sy)
# Returns the index of that bin, or -1.
proc logos_fo::kept_bin {bins units slack sx sy cap {per_buffer 1} {fixed 0}} {
    set best -1
    set best_rank ""
    set i 0
    set nb [llength $bins]
    foreach b $bins {
        set w 0
        foreach u $units {
            if { [lsearch -exact $b [lindex $u 0]] >= 0 } { incr w [lindex $u 3] }
        }
        if { $w + ($nb - 1) * $per_buffer + $fixed <= $cap } {
            set s ""
            foreach id $b {
                if { [dict exists $slack $id] && ($s eq "" || [dict get $slack $id] < $s) } { set s [dict get $slack $id] }
            }
            lassign [logos_fo::centroid $units $b] cx cy
            set rank [list [expr { $s eq "" }] [expr { $s eq "" ? 0 : $s }] [expr { abs($cx - $sx) + abs($cy - $sy) }]]
            if { $best < 0 || [logos_fo::rank_before $rank $best_rank] } {
                set best $i
                set best_rank $rank
            }
        }
        incr i
    }
    return $best
}

# When no whole bin can stay (kept_bin answers -1), the kept set is carved from the units themselves: the most
# critical first — a known slack before none, the least slack, then the nearest the driver — each taken while it,
# `per_buffer` loads for every bin the rest then packs into, and the net's `fixed` loads still fit, so the critical
# sink gains no stage.
# Returns {kept_ids rest_bins}; kept_ids is empty, and rest_bins the net's own binning, when not even the most
# critical unit fits beside the others' buffers.
proc logos_fo::carve {units slack sx sy cap {per_buffer 1} {fixed 0}} {
    set ranked [list]
    foreach u $units {
        lassign $u id x y
        set s [expr { [dict exists $slack $id] ? [dict get $slack $id] : "" }]
        lappend ranked [list [list [expr { $s eq "" }] [expr { $s eq "" ? 0 : $s }] [expr { abs($x - $sx) + abs($y - $sy) }]] $u]
    }
    set ranked [lsort -command logos_fo::rank_cmp $ranked]
    set kept [list]
    set w 0
    set rest $units
    set rest_bins [logos_fo::bins $rest $cap]
    foreach r $ranked {
        set u [lindex $r 1]
        set trial [lsearch -all -inline -not -exact $rest $u]
        set tb [expr { [llength $trial] ? [logos_fo::bins $trial $cap] : [list] }]
        if { $w + [lindex $u 3] + $per_buffer * [llength $tb] + $fixed > $cap } { break }
        lappend kept [lindex $u 0]
        incr w [lindex $u 3]
        set rest $trial
        set rest_bins $tb
    }
    return [list $kept $rest_bins]
}

proc logos_fo::rank_cmp {a b} {
    if { [logos_fo::rank_before [lindex $a 0] [lindex $b 0]] } { return -1 }
    if { [logos_fo::rank_before [lindex $b 0] [lindex $a 0]] } { return 1 }
    return 0
}

# Whether rank `a` orders before rank `b`, element by element.
proc logos_fo::rank_before {a b} {
    foreach x $a y $b {
        if { $x < $y } { return 1 }
        if { $x > $y } { return 0 }
    }
    return 0
}
