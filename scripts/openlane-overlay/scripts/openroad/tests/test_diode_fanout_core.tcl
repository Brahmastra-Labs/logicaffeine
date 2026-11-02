# Unit tests for logos_diode_fanout_core.tcl. Plain tclsh, 8.5 or later:
#   tclsh scripts/openlane-overlay/scripts/openroad/tests/test_diode_fanout_core.tcl
source [file join [file dirname [info script]] .. logos_diode_fanout_core.tcl]

set bad 0
proc check {what got want} {
    global bad
    if { $got ne $want } {
        incr bad
        puts "FAIL: $what\n  got:  $got\n  want: $want"
    }
}

proc ids_of {bins} {
    set all [list]
    foreach b $bins { foreach id $b { lappend all $id } }
    return [lsort $all]
}

proc bin_weights {bins units} {
    set w [dict create]
    foreach u $units { dict set w [lindex $u 0] [lindex $u 3] }
    set out [list]
    foreach b $bins {
        set s 0
        foreach id $b { incr s [dict get $w $id] }
        lappend out $s
    }
    return $out
}

proc shuffled {l} {
    set keyed [list]
    foreach x $l { lappend keyed [list [expr { rand() }] $x] }
    set out [list]
    foreach k [lsort -real -index 0 $keyed] { lappend out [lindex $k 1] }
    return $out
}

proc max_of {l} {
    set m [lindex $l 0]
    foreach x $l { if { $x > $m } { set m $x } }
    return $m
}

# --- pair -----------------------------------------------------------------------------------------------------

set gates {{0 _100_ A 0 0} {1 _101_ B 1000 0} {2 _1_02 A_N 5000 0}}
check "a heuristic diode pairs with the gate it is named for, wherever it sits" \
    [logos_fo::pair $gates {{0 ANTENNA__101__B 0 0}}] {0 1}
check "an ECO diode pairs with the gate it is named for" \
    [logos_fo::pair $gates {{0 ANTENNA_ECO__100__A 4999 0}}] {0 0}
check "a pin name with an underscore still pairs by name" \
    [logos_fo::pair $gates {{0 ANTENNA__1_02_A_N 0 0}}] {0 2}
check "a global-route diode pairs with the nearest gate" \
    [logos_fo::pair $gates {{0 ANTENNA_17 900 10}}] {0 1}
check "a tie in distance goes to the lower gate id" \
    [logos_fo::pair $gates {{0 ANTENNA_17 500 0}}] {0 0}
check "a diode named for a gate on another net pairs by distance" \
    [logos_fo::pair $gates {{0 ANTENNA__999__A 4900 0}}] {0 2}
check "a net with no gate pairs nothing" [logos_fo::pair {} {{0 ANTENNA_1 0 0}}] {}
set many [list]
for { set i 0 } { $i < 12 } { incr i } { lappend many [list $i ANTENNA_$i 1000 0] }
set owner [logos_fo::pair $gates $many]
check "every diode gets an owner" [llength [dict keys $owner]] 12
check "the nearest gate takes every unnamed diode beside it" [lsort -unique [dict values $owner]] {1}
# MEASURED on coproc_core (RUN_2026.10.03_23.19.59): global routing's repair put twelve diodes on net _29509_, one
# beside every pin of the component it judged — four beside the driver logos_iso_43/X, eight beside the one gate
# _34338_/A and that gate's own heuristic diode. Given the driver as a pin, each pairs with the pin it sits beside.
set net_pins {{0 _34338_ A 1179715 885550} {drv logos_iso_43 X 440621 890911}}
set net_diodes [list {h ANTENNA__34338__A 1178980 885360}]
foreach n {125 126 127 128 129 130 131 132} x {1182200 1177140 1184040 1175300 1173460 1171620 1169780 1167940} {
    lappend net_diodes [list $n ANTENNA_$n $x 885360]
}
foreach n {133 134 135 136} x {434700 445280 432860 447120} { lappend net_diodes [list $n ANTENNA_$n $x 890800] }
set owner [logos_fo::pair $net_pins $net_diodes]
set by_pin [dict create]
dict for {d g} $owner { dict lappend by_pin $g $d }
check "a global-route diode beside the driver pairs with the driver" [lsort [dict get $by_pin drv]] {133 134 135 136}
check "...and one beside the gate, or beside the gate's own diode, with the gate" \
    [lsort [dict get $by_pin 0]] {125 126 127 128 129 130 131 132 h}

# --- trim_unit ------------------------------------------------------------------------------------------------

set ds {{a 0 10} {b 0 1} {c 0 5} {d 0 100} {e 0 2}}
check "a gate over the cap keeps the cap-1 nearest diodes" [logos_fo::trim_unit $ds 0 0 4] {{b e c} {a d}}
check "a gate within the cap keeps all of them" [lindex [logos_fo::trim_unit $ds 0 0 9] 1] {}
check "a cap of 1 leaves room for the gate alone" [logos_fo::trim_unit $ds 0 0 1] {{} {b e c a d}}
check "equal distances keep the lower ids" [lindex [logos_fo::trim_unit {{z 0 1} {y 0 1} {x 0 1}} 0 0 3] 0] {x y}

# --- budget_trim ----------------------------------------------------------------------------------------------

set cu {{g0 10 0 {d0}} {g1 500 0 {d1}} {g2 20 0 {d2 d3}} {g3 900 0 {}}}
check "within budget nothing is shed" [logos_fo::budget_trim $cu 0 0 9 9] {}
check "the gate nearest the driver sheds first" [logos_fo::budget_trim $cu 0 0 9 8] {d0}
check "one diode per gate per pass, nearest first" [logos_fo::budget_trim $cu 0 0 9 6] {d0 d3 d1}
check "a second pass returns to the nearest gate still carrying one" [logos_fo::budget_trim $cu 0 0 9 5] {d0 d3 d1 d2}
check "a budget below the gates alone sheds every diode and stops" [logos_fo::budget_trim $cu 0 0 9 0] {d0 d3 d1 d2}
check "the diodes beside a clock driver, a unit at the driver, shed first" \
    [logos_fo::budget_trim [concat $cu [list {drv 0 0 {r0 r1}}]] 0 0 11 9] {r1 d0}

# --- ffd ------------------------------------------------------------------------------------------------------

check "first-fit-decreasing counts 5 5 4 4 into two bins of 9" [logos_fo::ffd_count {5 4 5 4} 9] 2
check "and packs them across each other" [logos_fo::ffd_pack {{a 0 0 5} {b 1 0 5} {c 2 0 4} {d 3 0 4}} 9] {{a c} {b d}}
check "equal weights fill in the given order" [logos_fo::ffd_pack {{a 0 0 2} {b 1 0 2} {c 2 0 2}} 4] {{a b} c}

# --- bins: the shapes the step meets --------------------------------------------------------------------------

check "a set that fits is one bin" [logos_fo::bins {{a 0 0 3} {b 9 9 3}} 9] {{a b}}
check "an empty set is no bins" [logos_fo::bins {} 9] {}
check "a unit over the cap is refused" [catch {logos_fo::bins {{a 0 0 10}} 9}] 1
check "a cap below 1 is refused" [catch {logos_fo::bins {{a 0 0 1}} 0}] 1

expr { srand(20261003) }
set suboptimal 0
for { set n 1 } { $n <= 100 } { incr n } {
    set units [list]
    for { set i 0 } { $i < $n } { incr i } {
        lappend units [list u$i [expr { int(rand() * 900000) }] [expr { int(rand() * 900000) }] 2]
    }
    set b [logos_fo::bins $units 9]
    if { [llength $b] != ($n + 3) / 4 } {
        incr suboptimal
        puts "  $n gates with one diode each: [llength $b] bins, optimum [expr { ($n + 3) / 4 }]"
    }
}
check "gates carrying one heuristic diode pack optimally under a cap of 9, 1 to 100 of them" $suboptimal 0

set left [list]
set right [list]
for { set i 0 } { $i < 4 } { incr i } {
    lappend left [list L$i [expr { 1000 + $i * 10 }] 1000 2]
    lappend right [list R$i [expr { 800000 + $i * 10 }] 1000 2]
}
check "two clusters a die apart never share a bin" [lsort [logos_fo::bins [concat $right $left] 8]] {{L0 L1 L2 L3} {R0 R1 R2 R3}}

set row [list]
for { set i 0 } { $i < 12 } { incr i } { lappend row [list g[format %02d [expr { 11 - $i }]] [expr { $i * 1000 }] [expr { ($i * 7) % 3 }] 2] }
set b [logos_fo::bins $row 8]
check "a row of twelve gates splits into three runs of neighbours along it" [lsort $b] {{g00 g01 g02 g03} {g04 g05 g06 g07} {g08 g09 g10 g11}}
set col [list]
for { set i 0 } { $i < 12 } { incr i } { lappend col [list g[format %02d [expr { (5 * $i) % 12 }]] [expr { ($i * 7) % 3 }] [expr { $i * 1000 }] 2] }
set b [logos_fo::bins $col 8]
check "and a column the same way, cut across its own length" [lsort $b] {{g00 g03 g05 g10} {g01 g06 g08 g11} {g02 g04 g07 g09}}

set heavy [list {h 500 500 9}]
for { set i 0 } { $i < 8 } { incr i } { lappend heavy [list l$i [expr { $i * 100 }] 0 2] }
set b [logos_fo::bins $heavy 9]
check "a gate carrying eight diodes takes a bin of its own" [expr { [lsearch -exact $b {h}] >= 0 }] 1
check "and its eight light neighbours take two more" [llength $b] 3

# --- bins: properties over random nets ------------------------------------------------------------------------

expr { srand(7) }
set invalid 0
set unstable 0
set worse 0
set worst 0
for { set c 0 } { $c < 1500 } { incr c } {
    set cap [expr { 1 + int(rand() * 15) }]
    set n [expr { 1 + int(rand() * 50) }]
    set units [list]
    set ws [list]
    for { set i 0 } { $i < $n } { incr i } {
        set w [expr { rand() < 0.8 ? ($cap < 2 ? $cap : 2) : 1 + int(rand() * $cap) }]
        lappend units [list u$i [expr { int(rand() * 200000) }] [expr { int(rand() * 200000) }] $w]
        lappend ws $w
    }
    set b [logos_fo::bins $units $cap]
    set want [list]
    foreach u $units { lappend want [lindex $u 0] }
    if { [ids_of $b] ne [lsort $want] || [max_of [bin_weights $b $units]] > $cap } {
        incr invalid
        puts "  invalid packing: cap $cap units $units -> $b"
    }
    if { [logos_fo::bins [shuffled $units] $cap] ne $b } { incr unstable }
    set over [expr { [llength $b] - [logos_fo::ffd_count $ws $cap] }]
    if { $over > 0 } { incr worse }
    if { $over > $worst } { set worst $over }
}
check "every unit lands in exactly one bin and no bin outweighs the cap" $invalid 0
check "the packing does not depend on the order the units arrive in" $unstable 0
check "a random mix never needs more than three bins beyond first-fit-decreasing" [expr { $worst <= 3 }] 1
puts "  random mixes beyond first-fit-decreasing: $worse of 1500, by at most $worst"

# --- centroid and kept_bin ------------------------------------------------------------------------------------

set cu {{a 0 0 1} {b 300 0 2} {c 0 900 1}}
check "the centroid is weighted by load" [logos_fo::centroid $cu {a b}] {200 0}
check "an empty bin centres on the origin" [logos_fo::centroid $cu {}] {0 0}

set units {{a 0 0 2} {b 10 0 2} {c 1000 0 2} {d 1010 0 2} {e 2000 0 2} {f 2010 0 2}}
set b {{a b} {c d} {e f}}
set calm [dict create]
check "with no slack known, the bin nearest the driver stays on the net when it fits" [logos_fo::kept_bin $b $units $calm 2005 0 9] 2
check "no bin stays when none fits beside the others' buffers" [logos_fo::kept_bin $b $units $calm 0 0 4] -1
check "a bin that fits only with fewer buffers is not kept" [logos_fo::kept_bin $b $units $calm 0 0 5] -1
check "a bin that fits exactly is kept" [logos_fo::kept_bin $b $units $calm 0 0 6] 0
check "a buffer carrying its own input diode costs the net it leaves two loads" [logos_fo::kept_bin $b $units $calm 0 0 7 2] -1
check "...so a bin is kept only with room for both, per other bin" [logos_fo::kept_bin $b $units $calm 0 0 8 2] 0
check "the diodes beside the driver stay on the net, so a bin is kept only with room beside them" \
    [logos_fo::kept_bin $b $units $calm 0 0 6 1 1] -1
check "...and one is kept once they fit" [logos_fo::kept_bin $b $units $calm 0 0 7 1 1] 0
set tight [dict create a 3.1 b 2.0 c 0.12 d 4.0 e 1.5 f 9.9]
check "the bin holding the gate of least setup slack stays, however far from the driver" [logos_fo::kept_bin $b $units $tight 2005 0 9] 1
check "a critical bin that does not fit is passed over for the next most critical" \
    [logos_fo::kept_bin {{a b} {c d e} {f}} {{a 0 0 2} {b 10 0 2} {c 1000 0 3} {d 1010 0 3} {e 1020 0 2} {f 2010 0 2}} $tight 0 0 6] 0
check "a gate whose slack is known outranks gates whose slack is not" \
    [logos_fo::kept_bin $b $units [dict create f 7.0] 0 0 9] 2
check "equally critical bins fall back to the one nearest the driver" \
    [logos_fo::kept_bin $b $units [dict create a 1.0 c 1.0] 1005 0 9] 1

# --- carving the kept set when no whole bin fits ---------------------------------------------------------------
# MEASURED on systolic_pipe16_realbist (RUN_2026.10.03_18.45.27): each level of a buf_12 broadcast tree drove ten
# gates with their diodes; three guarded buffers (two loads each) left no bin that fit beside the others, so no bin
# stayed and the next tree level went behind a split buffer at every level — six stages, -0.04 ns at signoff.
set line [list]
foreach id {a b c d e f g h i j} x {0 100 200 300 400 500 600 700 800 900} { lappend line [list $id $x 0 2] }
set lb [logos_fo::bins $line 9]
check "a net of ten guarded gates under a cap of 9 keeps no whole bin beside two-load buffers" \
    [logos_fo::kept_bin $lb $line [dict create] 0 0 9 2] -1
set crit [dict create e 0.1 f 0.2 g 0.3 a 5.0]
lassign [logos_fo::carve $line $crit 0 0 9 2] kept rest
check "the most critical gates stay on the driver, carved in slack order while they fit" [lsort $kept] {e f}
set rw [bin_weights $rest $line]
check "...beside two loads for every other bin" [expr { [llength $kept] * 2 + 2 * [llength $rest] <= 9 }] 1
check "...and every other bin fits the cap" [expr { [max_of $rw] <= 9 }] 1
set all [lsort [concat $kept {*}$rest]]
check "...and every gate lands in exactly one of them" $all {a b c d e f g h i j}
lassign [logos_fo::carve $line [dict create] 950 0 9 2] kept rest
check "with no slack known the gates nearest the driver stay" [lsort $kept] {i j}
lassign [logos_fo::carve {{a 0 0 8} {b 10 0 8}} [dict create a 0.1] 0 0 9 2] kept rest
check "a gate too heavy to stay beside the others' buffers is not kept" $kept {}
check "...and the net is binned as before" [llength $rest] 2
lassign [logos_fo::carve $line $crit 0 0 9 2 1] kept rest
check "the carve leaves room for the diodes beside the driver" \
    [list [lsort $kept] [expr { 2 * [llength $kept] + 2 * [llength $rest] + 1 }]] {{e f} 9}
lassign [logos_fo::carve $line $crit 0 0 9 2 2] kept rest
check "...and keeps no gate when they leave no room beside the others' buffers" $kept {}

# --- a whole net, as the step plans it ------------------------------------------------------------------------

set gates [list]
set diodes [list]
for { set i 0 } { $i < 10 } { incr i } {
    lappend gates [list $i _[expr { 500 + $i }]_ S [expr { $i * 40000 }] 0]
    lappend diodes [list $i ANTENNA__[expr { 500 + $i }]__S [expr { $i * 40000 + 460 }] 0]
}
set owner [logos_fo::pair $gates $diodes]
set paired_right 1
dict for {d g} $owner { if { $d != $g } { set paired_right 0 } }
check "ten mux2 select inputs each pair with their own heuristic diode" $paired_right 1
set units [list]
foreach g $gates {
    lassign $g gid inst pin gx gy
    lappend units [list $gid $gx $gy 2]
}
set b [logos_fo::bins $units 9]
check "a fanout-20 net of ten gates and their diodes splits into three bins under a cap of 9" [llength $b] 3
set kept [logos_fo::kept_bin $b $units [dict create] 0 0 9]
set parent [expr { [llength $b] - ($kept >= 0 ? 1 : 0) }]
if { $kept >= 0 } { incr parent [lindex [bin_weights [list [lindex $b $kept]] $units] 0] }
check "the net keeps the bin at its driver and still fits beside the other bins' buffers" [expr { $kept >= 0 && $parent <= 9 }] 1
check "every child net fits the cap" [max_of [bin_weights $b $units]] 8

puts [expr { $bad == 0 ? "PASS: diode-aware fanout planning" : "FAIL: $bad case(s)" }]
exit [expr { $bad ? 1 : 0 }]
