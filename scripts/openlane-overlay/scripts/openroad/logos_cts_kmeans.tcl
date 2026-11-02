# LOGOS H-tree k-means exponent (sourced by cts.tcl): the largest clustering exponent TritonCTS's branch refinement
# can take on this core without overflowing its min-cost flow, 4 (TritonCTS's own) wherever that is safe.
#
# Each H-tree level splits its sinks between two branch points with a min-cost flow (cts/Clustering.cpp, LEMON
# NetworkSimplex<int, int>). An arc's cost is a sink's Manhattan distance to a point in wire units, raised to the
# clustering exponent, and an arc is kept while that cost is under INT_MAX and its distance at most 5200 units.
# LEMON's integer simplex gives its artificial arc the cost ART = 2^30, so the target's potential starts at ART and
# every other node's at 0, and it keeps each potential the signed sum of the costs on the node's tree path from the
# root. A simple path between the two branch points crosses at most four sink arcs, so with C the dearest arc every
# potential lies within 4C of 0 or of ART, and the dearest reduced cost the pivots evaluate is ART + 9C. That fits a
# signed int only while 9C < 2^30; past it the reduced costs wrap and the pivoting never ends. MEASURED on the WPE
# macro (RUN_2026.10.18_19.41.51, a 2.8 x 3.3 mm core, 38,748 sinks): exponent 4 spun 22 h at the H-tree's second
# level. Replayed, exponents 3 and 2 build the tree in minutes, 6,606 buffers (logs/clock-push/cts-replay).
#
# The dearest arc is bounded by the core's width plus height, in TritonCTS's wire unit. TechChar.cpp sets that unit to
# the characterization buffer's height times 10 / 2, in whole microns, taking the buffer from the CTS buffer list.

namespace eval logos_cts_kmeans {
    # ART_COST of LEMON's NetworkSimplex<int>: numeric_limits<int>::max() / 2 + 1.
    variable art [expr { 2**30 }]

    # The largest exponent up to 4 whose dearest arc on a core `span` wire units across keeps 9 C under ART. It is
    # never below 2: Clustering.cpp's minCostFlow keeps no arc longer than 5200 units, and 9 * 5200^2 < 2^30.
    proc exponent {span} {
        variable art
        foreach e {4 3} {
            if { 9 * $span**$e < $art } { return $e }
        }
        return 2
    }

    # TritonCTS's wire unit in microns for a buffer `height` dbu tall: whole microns, at least one.
    proc wire_unit_um {height dbu} {
        return [expr { max($height * 10 / 2 / $dbu, 1) }]
    }

    # The core's width plus height in wire units, rounded up, for a core `dx` by `dy` dbu and a unit of `unit_um`.
    proc span {dx dy unit_um dbu} {
        return [expr { ($dx + $dy + $unit_um * $dbu - 1) / ($unit_um * $dbu) }]
    }
}

# The exponent for this design: its core in the smallest wire unit any CTS buffer could set, so the span is never
# under the one TritonCTS measures.
proc logos_cts_kmeans_exponent {} {
    set block [ord::get_db_block]
    set db [ord::get_db]
    set dbu [$block getDbUnitsPerMicron]
    set unit 0
    foreach name [concat $::env(CTS_CLK_BUFFER_LIST) $::env(CTS_ROOT_BUFFER)] {
        set m [$db findMaster $name]
        if { $m eq "NULL" } { continue }
        set u [logos_cts_kmeans::wire_unit_um [$m getHeight] $dbu]
        if { $unit == 0 || $u < $unit } { set unit $u }
    }
    if { $unit == 0 } { set unit 1 }
    set core [$block getCoreArea]
    set span [logos_cts_kmeans::span [$core dx] [$core dy] $unit $dbu]
    set e [logos_cts_kmeans::exponent $span]
    puts "\[LOGOS-CTS\] k-means exponent $e: the core spans $span wire units of $unit um"
    return $e
}
