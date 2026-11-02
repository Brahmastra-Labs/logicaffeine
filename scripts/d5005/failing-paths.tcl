# The setup paths into a fitted image's clocks that a target period fails, one line an endpoint, for the host to group
# by unit and by the net they share: run by `quartus.sh sta <job> <tree> failing-paths.tcl [TARGET_NS] [CORNER]`, which
# hands it the project, its revision and the job's reports directory first.
#
# Each clock whose name matches CLOCKS is analysed at CORNER (default the slow corner the image's sign-off found worst,
# 2_slow_900mv_100c) against its own constraint, and every endpoint whose worst slack would be negative at TARGET_NS
# (default 5.0) is written to <reports>/failing-paths.tsv: the clock, the slack, the data delay, the logic levels, the
# clock skew, the widest fan-out on the path and the node that drives it, the path's first node and its last. Its
# endpoints' count by target goes to <reports>/summary.txt.

lassign $quartus(args) project revision out target corner
if {$target eq ""} { set target 5.0 }
if {$corner eq ""} { set corner 2_slow_900mv_100c }
set clocks {*qph_user_clk_iopll|iopll_0_outclk0 *qph_user_clk_iopll|iopll_0_outclk1}
set most 200000

# A path's property, or "?" where the Timing Analyzer gives none.
proc property {path option} {
  if {[catch {get_path_info $path $option} value]} { return "?" }
  return $value
}

# A node's name, or "?".
proc named {node} {
  if {[catch {get_node_info -name $node} name]} { return "?" }
  return $name
}

project_open $project -revision $revision
create_timing_netlist
read_sdc
set chosen ""
set available {}
foreach_in_collection op [get_available_operating_conditions] {
  set shown [get_operating_conditions_info $op -display_name]
  lappend available "$op ($shown)"
  if {[string tolower $op] eq [string tolower $corner] || $shown eq $corner} { set chosen $op }
}
if {$chosen eq ""} { error "no operating condition $corner among: [join $available {, }]" }
set_operating_conditions $chosen
update_timing_netlist

set tsv [open "$out/failing-paths.tsv" w]
set summary [open "$out/summary.txt" w]
puts $tsv "clock\tslack\tdata_delay\tlevels\tskew\tfanout\tfanout_node\tfrom\tto"
puts $summary "corner $chosen of [join $available {, }]; target $target ns"
foreach pattern $clocks {
  foreach_in_collection clk [get_clocks -nowarn $pattern] {
    set name [get_clock_info -name $clk]
    set period [get_clock_info -period $clk]
    set limit [expr {$period - $target}]
    set paths [get_timing_paths -setup -to_clock $clk -npaths $most -nworst 1 -less_than_slack $limit]
    set n 0
    foreach_in_collection p $paths {
      set slack [get_path_info $p -slack]
      set widest 0
      set driver ""
      foreach_in_collection point [get_path_info $p -arrival_points] {
        if {[catch {get_point_info $point -number_of_fanout} fanout] || ![string is integer -strict $fanout]} { continue }
        if {$fanout > $widest} {
          set widest $fanout
          if {[catch {get_point_info $point -node} node]} { set driver "?" } else { set driver [named $node] }
        }
      }
      puts $tsv [join [list $name $slack [property $p -data_delay] [property $p -num_logic_levels] \
        [property $p -clock_skew] $widest $driver [named [get_path_info $p -from]] [named [get_path_info $p -to]]] "\t"]
      incr n
    }
    puts $summary "$name period $period ns: $n endpoints fail $target ns (at most $most read)"
  }
}
close $tsv
close $summary
delete_timing_netlist
project_close
