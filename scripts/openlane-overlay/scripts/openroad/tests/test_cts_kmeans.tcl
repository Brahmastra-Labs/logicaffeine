# Unit tests for logos_cts_kmeans.tcl. Plain tclsh, 8.5 or later:
#   tclsh scripts/openlane-overlay/scripts/openroad/tests/test_cts_kmeans.tcl
source [file join [file dirname [info script]] .. logos_cts_kmeans.tcl]

set bad 0
proc check {what got want} {
    global bad
    if { $got ne $want } {
        incr bad
        puts "FAIL: $what\n  got:  $got\n  want: $want"
    }
}

# The bound itself, restated: an exponent is safe on a span when 9 times its dearest arc stays under 2^30, an arc being
# at most 5200 units long (Clustering.cpp drops longer ones).
proc safe {span e} {
    set d [expr { min($span, 5200) }]
    return [expr { 9 * $d**$e < 2**30 }]
}

# Every span from one wire unit to past the reach: the exponent is safe, and it is the largest safe one up to 4.
for { set s 1 } { $s <= 6000 } { incr s } {
    set e [logos_cts_kmeans::exponent $s]
    if { ![safe $s $e] } { check "exponent $e on span $s is safe" 0 1 }
    if { $e < 4 && [safe $s [expr { $e + 1 }]] } { check "exponent $e on span $s is the largest safe" $e [expr { $e + 1 }] }
    if { $e < 2 || $e > 4 } { check "exponent on span $s within 2..4" $e "2..4" }
}

# The thresholds, as numbers: 4 holds while 9 d^4 < 2^30, i.e. through d = 104; 3 through d = 492; 2 through
# d = 10922, past the reach.
check "span 104 keeps 4" [logos_cts_kmeans::exponent 104] 4
check "span 105 takes 3" [logos_cts_kmeans::exponent 105] 3
check "span 492 keeps 3" [logos_cts_kmeans::exponent 492] 3
check "span 493 takes 2" [logos_cts_kmeans::exponent 493] 2
check "the reach takes 2" [logos_cts_kmeans::exponent 5200] 2
check "a span past the reach takes 2" [logos_cts_kmeans::exponent 100000] 2
check "a zero span keeps 4" [logos_cts_kmeans::exponent 0] 4

# TritonCTS's unit for sky130 hd (2.72 um rows, 1000 dbu/um): 2720 * 10 / 2 / 1000 = 13 um, whole microns.
check "sky130 hd wire unit" [logos_cts_kmeans::wire_unit_um 2720 1000] 13
check "a unit under a micron is one" [logos_cts_kmeans::wire_unit_um 100 1000] 1

# The span rounds up: a core of exactly k units is k, one dbu more is k + 1.
check "an exact span" [logos_cts_kmeans::span 13000 13000 13 1000] 2
check "a span rounds up" [logos_cts_kmeans::span 13000 13001 13 1000] 3

# The WPE macro's core (2,800 x 3,300 um at 13 um units, the run that spun at exponent 4) takes 3, and a block of
# gpu_simt's size (about 600 x 600 um) keeps TritonCTS's own 4.
check "the WPE takes 3" [logos_cts_kmeans::exponent [logos_cts_kmeans::span 2800000 3300000 13 1000]] 3
check "a 0.6 mm block keeps 4" [logos_cts_kmeans::exponent [logos_cts_kmeans::span 600000 600000 13 1000]] 4

# The design's exponent through odb, on stand-ins for the block and the library: the WPE run's core (5.52..2794.04 by
# 10.88..3348.32 um, RUN_2026.10.18_19.41.51) and sky130 hd's 2.72 um buffers span 472 units of 13 um, as the replay
# printed, and take 3.
namespace eval ord {}
proc ord::get_db_block {} { return stub_block }
proc ord::get_db {} { return stub_db }
proc stub_block {method args} {
    switch $method {
        getDbUnitsPerMicron { return 1000 }
        getCoreArea { return stub_core }
    }
    error "stub_block: no $method"
}
proc stub_core {method} {
    switch $method {
        dx { return $::core_dx }
        dy { return $::core_dy }
    }
    error "stub_core: no $method"
}
proc stub_db {method name} {
    if { $method ne "findMaster" } { error "stub_db: no $method" }
    if { ![dict exists $::heights $name] } { return NULL }
    interp alias {} stub_master_$name {} stub_master $name
    return stub_master_$name
}
proc stub_master {name method} {
    if { $method ne "getHeight" } { error "stub_master: no $method" }
    return [dict get $::heights $name]
}
proc design_exponent {dx dy heights buffers root} {
    set ::core_dx $dx
    set ::core_dy $dy
    set ::heights $heights
    set ::env(CTS_CLK_BUFFER_LIST) $buffers
    set ::env(CTS_ROOT_BUFFER) $root
    return [logos_cts_kmeans_exponent]
}
set hd [dict create sky130_fd_sc_hd__clkbuf_8 2720 sky130_fd_sc_hd__clkbuf_4 2720 sky130_fd_sc_hd__clkbuf_2 2720 sky130_fd_sc_hd__clkbuf_16 2720]
set bufs {sky130_fd_sc_hd__clkbuf_8 sky130_fd_sc_hd__clkbuf_4 sky130_fd_sc_hd__clkbuf_2}
check "the WPE's core takes 3" [design_exponent 2788520 3337440 $hd $bufs sky130_fd_sc_hd__clkbuf_16] 3
check "a 0.6 mm core keeps 4" [design_exponent 600000 600000 $hd $bufs sky130_fd_sc_hd__clkbuf_16] 4
# A buffer the library lacks is skipped; a shorter one sets a finer unit (6 um at 1.36 um), which doubles the span.
check "a missing buffer is skipped" [design_exponent 2788520 3337440 $hd [concat $bufs no_such_buffer] sky130_fd_sc_hd__clkbuf_16] 3
dict set hd short_clkbuf 1360
check "the finest unit of the list sets the span" [design_exponent 2788520 3337440 $hd [concat $bufs short_clkbuf] sky130_fd_sc_hd__clkbuf_16] 2
check "the root buffer counts too" [design_exponent 2788520 3337440 $hd $bufs short_clkbuf] 2
check "no buffer found falls back to one micron" [design_exponent 600000 600000 [dict create] $bufs none] 2

if { $bad } {
    puts "FAILED: $bad check(s)"
    exit 1
}
puts "PASS: H-tree k-means exponent"
