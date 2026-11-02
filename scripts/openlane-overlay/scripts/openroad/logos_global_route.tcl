# LOGOS: every incremental global route continues the route it increments, under that route's arguments.
#
# OpenROAD's global_route takes its congestion tolerance, congestion iterations, grid origin and verbosity from each
# call's own arguments, so an incremental call given none routes with congestion refused and fifty iterations, whatever
# the route it continues was made with. MEASURED on systolic_os_pipe8_realbist (RUN_2026.10.15_19.03.07): the
# post-diode upsize measured on a route made with -allow_congestion, and its first incremental re-route, after round
# 1's isolations, failed GRT-0232. A step that sources this file makes every route in full as asked, and each
# incremental call carries the arguments of the last route made in full, less its guide file, which that route wrote.
# Sourcing it again changes nothing.
if { [info commands logos_global_route_call] eq "" } {
    rename global_route logos_global_route_call
}
proc global_route {args} {
    global logos_global_route_args
    if { "-start_incremental" ni $args && "-end_incremental" ni $args } {
        set logos_global_route_args [list]
        for {set i 0} {$i < [llength $args]} {incr i} {
            if { [lindex $args $i] eq "-guide_file" } {
                incr i
            } else {
                lappend logos_global_route_args [lindex $args $i]
            }
        }
    } elseif { ![info exists logos_global_route_args] } {
        error "an incremental global route continues a route made in full, and none was made"
    } else {
        set args [concat $logos_global_route_args $args]
    }
    logos_global_route_call {*}$args
}
