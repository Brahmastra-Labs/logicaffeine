# Modified by the LOGOS project from OpenLane ff5509f65b17bfa4068d5336495ab1718987ff69; see scripts/openlane-overlay/NOTICE.
# Copyright 2020-2022 Efabless Corporation
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#      http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

proc global_routing_or {args} {
    handle_deprecated_command global_routing
}

proc groute_antenna_extract {args} {
    set options {
        {-from_log required}
    }
    set flags {}
    parse_key_args "groute_antenna_extract" args arg_values $options flags_map $flags

    set value [exec python3 $::env(SCRIPTS_DIR)/extract_antenna_count.py < $arg_values(-from_log)]

    return $value
}

proc global_routing_fastroute {args} {
    increment_index
    TIMER::timer_start
    set log [index_file $::env(routing_logs)/global.log]
    puts_info "Running Global Routing (log: [relpath . $log])..."

    set initial_def [index_file $::env(routing_tmpfiles)/global.def]
    set initial_guide [index_file $::env(routing_tmpfiles)/global.guide]
    set initial_odb [index_file $::env(routing_tmpfiles)/global.odb]

    set ::env(GRT_CONGESTION_REPORT_FILE) $::env(routing_tmpfiles)/groute-congestion.rpt
    run_openroad_script $::env(SCRIPTS_DIR)/openroad/groute.tcl\
        -indexed_log $log\
        -save "def=$initial_def,guide=$initial_guide,odb=$initial_odb"\
        -no_update_current

    if { $::env(GRT_REPAIR_ANTENNAS) } {
        puts_info "Starting OpenROAD Antenna Repair Iterations..."
        set iter 1

        set minimum_def $initial_def
        set minimum_guide $initial_guide
        set minimum_odb $initial_odb
        set minimum_antennae [groute_antenna_extract -from_log $log]

        while {$iter <= $::env(GRT_MAX_DIODE_INS_ITERS) && $minimum_antennae > 0} {
            set log [index_file $::env(routing_logs)/antenna_diodes_$iter.log]
            puts_info "Starting antenna repair iteration $iter with $minimum_antennae violations..."

            manipulate_layout $::env(SCRIPTS_DIR)/odbpy/defutil.py replace_instance_prefixes\
                -indexed_log $log\
                -output_def $::env(CURRENT_DEF)\
                --original-prefix "ANTENNA"\
                --new-prefix "INSDIODE$iter"

            set log [index_file $::env(routing_logs)/antenna_route_$iter.log]
            run_openroad_script $::env(SCRIPTS_DIR)/openroad/groute.tcl\
                -indexed_log $log\
                -save "to=$::env(routing_tmpfiles),name=global_$iter,def,guide,odb"\
                -no_update_current

            set antennae [groute_antenna_extract -from_log $log]

            if { $antennae >= $minimum_antennae } {
                puts_info "\[Iteration $iter\] Failed to reduce antenna violations ($minimum_antennae -> $antennae), stopping iterations..."
                set ::env(SAVE_DEF) $minimum_def
                set ::env(SAVE_GUIDE) $minimum_guide
                set ::env(SAVE_ODB) $minimum_odb
                break
            } else {
                puts_info "\[Iteration $iter\] Reduced antenna violations ($minimum_antennae -> $antennae)"
                set minimum_def $::env(SAVE_DEF)
                set minimum_guide $::env(SAVE_GUIDE)
                set minimum_odb $::env(SAVE_ODB)
                set minimum_antennae [groute_antenna_extract -from_log [groute_antenna_extract -from_log $log]]
            }
            incr iter
        }
    }

    set_def $::env(SAVE_DEF)
    set_guide $::env(SAVE_GUIDE)
    set_odb $::env(SAVE_ODB)
    unset ::env(SAVE_DEF)
    unset ::env(SAVE_GUIDE)
    unset ::env(SAVE_ODB)

    write_verilog\
        $::env(routing_tmpfiles)/global.nl.v\
        -powered_to $::env(routing_tmpfiles)/global.pnl.v\
        -indexed_log [index_file $::env(routing_logs)/global_write_netlist.log]

    TIMER::timer_stop
    run_sta -no_save -log $::env(routing_logs)/grt_sta.log

    exec echo "[TIMER::get_runtime]" | python3 $::env(SCRIPTS_DIR)/write_runtime.py "global routing - openroad"
}

proc global_routing_cugr {args} {
    handle_deprecated_command global_routing_fastroute
}

proc global_routing {args} {
    if { $::env(GLOBAL_ROUTER) == "cugr" } {
        puts_warn "CU-GR is no longer supported. OpenROAD fastroute will be used instead."
    }

    global_routing_fastroute
}

proc detailed_routing_tritonroute {args} {
    if { $::env(DETAILED_ROUTER) == "drcu" } {
        puts_warn "DR-CU is no longer supported. OpenROAD's detailed router will be used instead."
        set ::env(DETAILED_ROUTER) "tritonroute"
    }

    increment_index
    TIMER::timer_start
    set log [index_file $::env(routing_logs)/detailed.log]
    puts_info "Running Detailed Routing (log: [relpath . $log])..."

    set ::env(_tmp_drt_file_prefix) $::env(routing_tmpfiles)/drt
    set ::env(_tmp_drt_rpt_prefix) $::env(routing_reports)/drt
    run_openroad_script $::env(SCRIPTS_DIR)/openroad/droute.tcl\
        -indexed_log $log\
        -save "to=$::env(routing_results),noindex,def,odb,netlist,powered_netlist"

    try_exec python3 $::env(SCRIPTS_DIR)/drc_rosetta.py tr to_klayout \
        -o $::env(routing_reports)/drt.klayout.xml \
        --design-name $::env(DESIGN_NAME) \
        $::env(routing_reports)/drt.drc

    if { $::env(QUIT_ON_TR_DRC) } {
        quit_on_tr_drc
    }

    TIMER::timer_stop
    exec echo "[TIMER::get_runtime]" | python3 $::env(SCRIPTS_DIR)/write_runtime.py "detailed_routing - openroad"
}

proc detailed_routing_drcu {args} {
    handle_deprecated_command detailed_routing_tritonroute
}

proc detailed_routing {args} {
    detailed_routing_tritonroute {*}$args
}

proc ins_fill_cells_or {args} {
    handle_deprecated_command ins_fill_cells
}

proc ins_fill_cells {args} {
    increment_index
    TIMER::timer_start
    set log [index_file $::env(routing_logs)/fill.log]
    puts_info "Running Fill Insertion (log: [relpath . $log])..."

    run_openroad_script $::env(SCRIPTS_DIR)/openroad/fill.tcl\
        -indexed_log [index_file $::env(routing_logs)/fill.log]\
        -save "to=$::env(routing_tmpfiles),name=fill,def,odb,netlist,powered_netlist"

    TIMER::timer_stop
    exec echo "[TIMER::get_runtime]" | python3 $::env(SCRIPTS_DIR)/write_runtime.py "fill insertion - openroad"
}

proc power_routing {args} {
    increment_index
    TIMER::timer_start
    set log [index_file $::env(routing_logs)/power_routing.log]
    puts_info "Routing top-level power nets (log: [relpath . $log])..."

    set options {
        {-lef optional}
        {-odb optional}
        {-power optional}
        {-ground optional}
        {-output_def optional}
        {-output_odb optional}
        {-extra_args optional}
    }
    set flags {}
    parse_key_args "power_routing" args arg_values $options flags_map $flags

    set_if_unset arg_values(-lef) $::env(MERGED_LEF)
    set_if_unset arg_values(-odb) $::env(CURRENT_ODB)
    set_if_unset arg_values(-power) $::env(VDD_PIN)
    set_if_unset arg_values(-ground) $::env(GND_PIN)
    set_if_unset arg_values(-output_def) [index_file $::env(routing_tmpfiles)/$::env(DESIGN_NAME).power_routed.def]
    set_if_unset arg_values(-output_odb) [index_file $::env(routing_tmpfiles)/$::env(DESIGN_NAME).power_routed.odb]
    set_if_unset arg_values(-extra_args) ""


    manipulate_layout $::env(SCRIPTS_DIR)/odbpy/power_utils.py power_route\
        -indexed_log [index_file $::env(routing_logs)/power_routing.log]\
        -output_def $arg_values(-output_def)\
        -output $arg_values(-output_odb)\
        -input $arg_values(-odb)\
        --core-vdd-pin $arg_values(-power)\
        --core-gnd-pin $arg_values(-ground)\
        {*}$arg_values(-extra_args)

    TIMER::timer_stop
    exec echo "[TIMER::get_runtime]" | python3 $::env(SCRIPTS_DIR)/write_runtime.py "top level power routing - openlane"
}


proc ins_diode_cells_1 {args} {
    puts_err "ins_diode_cells_1 is no longer supported"
    throw_error
}

proc io_diode_insertion {args} {
    increment_index
    TIMER::timer_start
    set log [index_file $::env(routing_logs)/io_diodes.log]
    puts_info "Running I/O Diode Insertion (log: [relpath . $log])..."

    # Custom script
    set save_def [index_file $::env(routing_tmpfiles)/io_diodes.def]
    set save_odb [index_file $::env(routing_tmpfiles)/io_diodes.odb]

    manipulate_layout $::env(SCRIPTS_DIR)/odbpy/diodes.py place\
        -indexed_log [index_file $::env(routing_logs)/io_diodes.log]\
        -output $save_odb\
        -output_def $save_def\
        --diode-cell $::env(DIODE_CELL)\
        --diode-pin  $::env(DIODE_CELL_PIN)\
        --threshold Infinity \
        --side-strategy $::env(HEURISTIC_ANTENNA_INSERTION_MODE) \
        --port-protect $::env(DIODE_ON_PORTS)

    set_def $save_def
    set_odb $save_odb

    # Legalize
    detailed_placement_or\
        -outdir $::env(routing_tmpfiles)\
        -log $::env(routing_logs)/io_diode_legalization.log\
        -name [index_file io_diode_legalized]

    TIMER::timer_stop
    exec echo "[TIMER::get_runtime]" | python3 $::env(SCRIPTS_DIR)/write_runtime.py "io diode insertion - openlane"
}

proc heuristic_diode_insertion {args} {
    increment_index
    TIMER::timer_start
    set log [index_file $::env(routing_logs)/diodes.log]
    puts_info "Running Heuristic Diode Insertion (log: [relpath . $log])..."

    # Custom script
    set save_def [index_file $::env(routing_tmpfiles)/diodes.def]
    set save_odb [index_file $::env(routing_tmpfiles)/diodes.odb]

    manipulate_layout $::env(SCRIPTS_DIR)/odbpy/diodes.py place\
        -indexed_log [index_file $::env(routing_logs)/diodes.log]\
        -output $save_odb\
        -output_def $save_def\
        --diode-cell $::env(DIODE_CELL)\
        --diode-pin  $::env(DIODE_CELL_PIN)\
        --threshold $::env(HEURISTIC_ANTENNA_THRESHOLD) \
        --side-strategy $::env(HEURISTIC_ANTENNA_INSERTION_MODE) \
        --port-protect none

    set_def $save_def
    set_odb $save_odb

    # Legalize
    detailed_placement_or\
        -outdir $::env(routing_tmpfiles)\
        -log $::env(routing_logs)/diode_legalization.log\
        -name [index_file diodes_legalized]

    TIMER::timer_stop
    exec echo "[TIMER::get_runtime]" | python3 $::env(SCRIPTS_DIR)/write_runtime.py "heuristic diode insertion - openlane"
}

# LOGOS targeted PRE-route antenna diode insertion. Given a net list (the ARC violators from a prior
# route), drop a diode on every input pin of exactly those nets and legalize with detailed_placement_or
# — the same machinery as heuristic_diode_insertion, so it is LVS-clean by construction: the native
# flow routes the diode pins and streams a consistent netlist+GDS. This replaces the post-route ODB ECO
# (run_antenna_eco), which desynced layout↔netlist. Gated behind TARGETED_DIODE_NETS (path to the list).
proc targeted_diode_insertion {args} {
    if { ![info exists ::env(TARGETED_DIODE_NETS)] || $::env(TARGETED_DIODE_NETS) eq "" } { return }
    if { ![file exists $::env(TARGETED_DIODE_NETS)] } {
        puts_info "\[LOGOS-TDIODE\] net list $::env(TARGETED_DIODE_NETS) not found — skipping"
        return
    }
    # REFUSE resizer-generated net names. The list is produced by reading a PRIOR run's antenna
    # violators, and only SYNTHESIS names (`_012345_`, or an RTL signal name) denote the same net
    # in the next run: yosys is deterministic given identical RTL and config. A `netNNNN` name is
    # invented by the RESIZER, so its number depends on how many buffers that run happened to
    # insert — change the slew margin and `net4021` becomes a different net entirely.
    #
    # Measured: a systolic_os_pipe8_realbist list containing net4021/net2795/net3204 dropped its
    # diodes onto whatever those numbers pointed at in the new netlist and produced 7,240 unmatched
    # nets at LVS (net-count difference 2, 170 unmatched devices) on a block whose previous run was
    # LVS-clean. Refusing costs one flow; accepting cost a full route plus the misdiagnosis.
    set _bad [list]
    set _tf [open $::env(TARGETED_DIODE_NETS) r]
    foreach _l [split [read $_tf] "\n"] {
        set _l [string trim $_l]
        if { $_l eq "" } { continue }
        if { [regexp {^net[0-9]+$} $_l] } { lappend _bad $_l }
    }
    close $_tf
    if { [llength $_bad] > 0 } {
        puts_err "\[LOGOS-TDIODE\] refusing resizer-generated net name(s): $_bad"
        puts_err "\[LOGOS-TDIODE\] `netNNNN` names are invented by the resizer and are NOT stable\
                  between runs — a diode placed by such a name lands on a different net and\
                  corrupts LVS. Target the synthesis name (`_012345_`) of the same net, or use the\
                  jog router / a base-route change for it."
        flow_fail
    }
    increment_index
    TIMER::timer_start
    set save_def [index_file $::env(routing_tmpfiles)/tdiodes.def]
    set save_odb [index_file $::env(routing_tmpfiles)/tdiodes.odb]
    puts_info "\[LOGOS-TDIODE\] targeted pre-route diode insertion from [file tail $::env(TARGETED_DIODE_NETS)]"
    set _tdlog [index_file $::env(routing_logs)/tdiodes.log]
    manipulate_layout $::env(SCRIPTS_DIR)/odbpy/diodes.py place_nets\
        -indexed_log $_tdlog\
        -output $save_odb\
        -output_def $save_def\
        --diode-cell $::env(DIODE_CELL)\
        --diode-pin  $::env(DIODE_CELL_PIN)\
        --net-list $::env(TARGETED_DIODE_NETS)
    # A pass that was asked for specific nets and placed NOTHING has not run; it has no-opped.
    # Downstream it is indistinguishable from success, and the resulting GDS came out
    # byte-identical to the previous run while the flow reported no problem. Fail here instead.
    if { [catch {exec grep -q {ERROR: 0 diodes placed} $_tdlog}] == 0 } {
        puts_err "\[LOGOS-TDIODE\] 0 diodes placed although nets were requested — see\
                  [file tail $_tdlog] for which were already dioded, missing, or pin-less.\
                  A net that violates WITH a diode needs a jog or a base-route change, not\
                  another diode."
        flow_fail
    }
    set_def $save_def
    set_odb $save_odb
    detailed_placement_or\
        -outdir $::env(routing_tmpfiles)\
        -log $::env(routing_logs)/tdiode_legalization.log\
        -name [index_file tdiodes_legalized]
    TIMER::timer_stop
    exec echo "[TIMER::get_runtime]" | python3 $::env(SCRIPTS_DIR)/write_runtime.py "targeted diode insertion - openlane"
}

proc apply_route_obs {args} {
    increment_index
    TIMER::timer_start
    set log [index_file $::env(routing_logs)/obs.log]
    puts_info "Applying Routing Obstructions (log: [relpath . $log])..."

    set save_def [file rootname $::env(CURRENT_DEF)].obs.def
    set save_db [file rootname $::env(CURRENT_DEF)].obs.odb

    manipulate_layout $::env(SCRIPTS_DIR)/odbpy/defutil.py add_obstructions\
        -indexed_log $log\
        -output $save_db\
        -output_def $save_def\
        --obstructions $::env(GRT_OBS)

    puts_verbose "Obstructions added over $::env(GRT_OBS)."

    set_def $save_def
    set_odb $save_db

    TIMER::timer_stop
    exec echo "[TIMER::get_runtime]" | python3 $::env(SCRIPTS_DIR)/write_runtime.py "adding routing obstructions - openlane"
}

proc add_route_obs {args} {
    if {[info exists ::env(GRT_OBS)]} {
        apply_route_obs
    }
}

proc check_wire_lengths {args} {
    increment_index
    TIMER::timer_start
    set log [index_file $::env(routing_logs)/wire_lengths.log]
    puts_info "Checking Wire Lengths (log: [relpath . $log])..."

    set arg_list [list]
    lappend arg_list --report-out [index_file $::env(routing_reports)/wire_lengths.csv]
    if { [info exists ::env(WIRE_LENGTH_THRESHOLD)] } {
        lappend arg_list --threshold $::env(WIRE_LENGTH_THRESHOLD)
    }
    if { $::env(QUIT_ON_LONG_WIRE) } {
        lappend arg_list --fail
    }

    manipulate_layout $::env(SCRIPTS_DIR)/odbpy/wire_lengths.py\
        -indexed_log $log\
        {*}$arg_list

    TIMER::timer_stop

    exec echo "[TIMER::get_runtime]" | python3 $::env(SCRIPTS_DIR)/write_runtime.py "wire lengths - openlane"
}

proc run_spef_extraction {args} {
    set options {
        {-save required}
        {-log required}
        {-rcx_def optional}
        {-rcx_lib optional}
        {-rcx_lef optional}
        {-rcx_rules optional}
        {-process_corner optional}
    }
    parse_key_args "run_spef_extraction" args arg_values $options

    set_if_unset arg_values(-rcx_lib) $::env(LIB_SYNTH_COMPLETE)
    set_if_unset arg_values(-rcx_lef) $::env(MERGED_LEF)
    set_if_unset arg_values(-rcx_rules) $::env(RCX_RULES)
    set_if_unset arg_values(-rcx_def) $::env(CURRENT_DEF)


    increment_index
    set log [index_file $arg_values(-log)]
    TIMER::timer_start

    set ec_postfix ""
    if { [info exists arg_values(-process_corner)]} {
        set ec_postfix " at the $arg_values(-process_corner) process corner"
    }

    puts_info "Running SPEF Extraction$ec_postfix (log: [relpath . $log])..."

    if { $::env(SPEF_EXTRACTOR) == "def2spef" } {
        puts_warn "def2spef/spef_extractor has been removed. OpenROAD OpenRCX will be used instead."
        set ::env(SPEF_EXTRACTOR) "openrcx"
    }


    set ::env(RCX_DEF) $arg_values(-rcx_def)
    set ::env(RCX_LEF) $arg_values(-rcx_lef)
    set ::env(RCX_LIB) $arg_values(-rcx_lib)
    set ::env(RCX_RULESET) $arg_values(-rcx_rules)
    assert_files_exist "$::env(RCX_RULESET) $::env(RCX_LIB)"
    run_openroad_script $::env(SCRIPTS_DIR)/openroad/rcx.tcl\
        -indexed_log $log\
        -save "odb=/dev/null,spef=$arg_values(-save)"
    unset ::env(RCX_LIB)
    unset ::env(RCX_RULESET)
    unset ::env(RCX_LEF)
    unset ::env(RCX_DEF)

    TIMER::timer_stop
    exec echo "[TIMER::get_runtime]" | python3 $::env(SCRIPTS_DIR)/write_runtime.py "parasitics extraction - openroad"
}

# LOGOS Magic/ARC-driven antenna ECO. OpenLane's repair_antennas works on the GLOBAL route and
# can't clear the DETAILED-route violations that check_antennas (the ARC signoff gate) reports.
# This does the standard foundry metal-ECO: check_antennas -> drop a protection diode on every
# offending net -> re-global+detailed-route -> re-check, iterating until the ARC report is empty.
# Gated behind DRT_ANTENNA_ECO=1 and catch-guarded, so the base flow is byte-identical otherwise.
#
# The ECO edits the layout after detailed routing wrote the netlist: a diode pass creates diode and
# filler instances and destroys the fillers it evicts. Signoff STA and the final gate-level view read
# CURRENT_NETLIST, so once any pass has touched the layout the netlist is written again from it.
# MEASURED on wpe_user_project (RUN_2026.09.27_18.14.14): the layout carried 253 ANTENNA_ECO cells
# the STA netlist did not, and the max-corner SPEF named every one of them "not found".
# The diode ECO's fanout limit: unless LOGOS_DIODE_FANOUT is 0 it adds a diode only to a net with room under the
# SDC's set_max_fanout, the room the pre-route fanout step holds for it.
proc logos_eco_fanout_arg {} {
    if { ![info exists ::env(LOGOS_DIODE_FANOUT)] || $::env(LOGOS_DIODE_FANOUT) == 1 } {
        return [list --max-fanout [expr { int(round($::env(MAX_FANOUT_CONSTRAINT))) }]]
    }
    return [list]
}

proc run_antenna_eco {args} {
    if { ![info exists ::env(DRT_ANTENNA_ECO)] || $::env(DRT_ANTENNA_ECO) != 1 } { return }
    set ::logos_eco_touched_layout 0
    logos_antenna_eco_iterations
    if { $::logos_eco_touched_layout } {
        write_verilog\
            $::env(routing_tmpfiles)/eco.nl.v\
            -powered_to $::env(routing_tmpfiles)/eco.pnl.v\
            -indexed_log [index_file $::env(routing_logs)/eco_write_netlist.log]
    }
}

proc logos_antenna_eco_iterations {args} {
    set max_iters 8
    if { [info exists ::env(DRT_ECO_ITERS)] } { set max_iters $::env(DRT_ECO_ITERS) }
    # Violator count at the last diode fallback; -1 = none yet. The fallback may repeat only while
    # each pass REDUCES the count, so it terminates at the first pass that makes no progress.
    set _fallback_last -1
    # Violator count at the previous ECO iteration; -1 = first pass.
    set _prev_nets -1
    for {set _it 1} {$_it <= $max_iters} {incr _it} {
        set _code [catch {
            run_or_antenna_check
            set _nets [list]
            set _pins [list]
            set _pin_layers [dict create]
            set _triples [list]
            set _rpt $::env(ANTENNA_VIOLATOR_LIST)
            if { [file exists $_rpt] } {
                set _fp [open $_rpt r]
                foreach _line [split [read $_fp] "\n"] {
                    if { [regexp {Net:\s*([^,]+)} $_line -> _n] } {
                        regsub -all {\\} $_n {} _n
                        lappend _nets [string trim $_n]
                    }
                    if { [regexp {Pin:\s*([^,]+)} $_line -> _p] } {
                        lappend _pins [string trim $_p]
                        # The layer rides along: the diode ECO ties a met3 violator anywhere in the
                        # component below met3, not only on the gate's own met1 island.
                        if { [regexp {Layer:\s*(\S+)} $_line -> _pl_layer] } {
                            dict lappend _pin_layers [string trim $_p] $_pl_layer
                        }
                    }
                    if { [regexp {Net:\s*([^,]+),\s*Pin:\s*([^,]+),\s*Layer:\s*(\S+)} $_line -> _tn _tp _tl] } {
                        regsub -all {\\} $_tn {} _tn
                        set _tp [string trim $_tp]
                        if { ![string match "*/DIODE" $_tp] } {
                            set _ix [string last "/" $_tp]
                            lappend _triples "[string trim $_tn]\t[string range $_tp 0 [expr {$_ix - 1}]]\t$_tl"
                        }
                    }
                }
                close $_fp
            }
            set _nets [lsort -unique $_nets]
            set _pins [lsort -unique $_pins]
            set _triples [lsort -unique $_triples]
            puts_info "\[LOGOS-ECO\] iteration $_it: [llength $_nets] violating net(s)"
            if { [llength $_nets] == 0 } {
                puts_info "\[LOGOS-ECO\] ANTENNA CLEAN after [expr {$_it - 1}] ECO iteration(s)"
                return
            }
            # LOGOS SAT jog-router (gated ECO_JOG): fix side-area antenna by promoting short free-window met
            # bridges on the violating gates' own components — no diodes, no re-route, so the clean base route
            # is untouched (no strand) and gate-area diodes already placed pre-route stay put.
            if { [info exists ::env(ECO_JOG)] && $::env(ECO_JOG) == 1 } {
                set _trifile $::env(routing_tmpfiles)/eco_tri_$_it.txt
                set _tf [open $_trifile w]; foreach _t $_triples { puts $_tf $_t }; close $_tf
                set _odb [index_file $::env(routing_tmpfiles)/eco_jog_$_it.odb]
                set _def [index_file $::env(routing_tmpfiles)/eco_jog_$_it.def]
                set _joglog [index_file $::env(routing_logs)/eco_jog_$_it.log]
                # Pass the branch-free restriction EXPLICITLY. Setting it as a config variable
                # put it in config.tcl and never reached the python: the run with it produced
                # byte-identical bridge counts to the run without.
                set _jogargs [list --net $_trifile --mode bridge_all]
                if { [info exists ::env(LOGOS_JOG_NO_JUNCTION)] && $::env(LOGOS_JOG_NO_JUNCTION) == 1 } {
                    lappend _jogargs --no-junction
                }
                manipulate_layout $::env(SCRIPTS_DIR)/odbpy/sat_antenna_jog.py\
                    -indexed_log $_joglog\
                    -output $_odb -output_def $_def\
                    {*}$_jogargs
                set_def $_def
                set_odb $_odb
                set ::logos_eco_touched_layout 1
                # Report what the router ACTUALLY did, not what it was asked to do. This used to print
                # `[llength $_triples]` — the number of VIOLATORS — as though it were the number of gates
                # bridged, so an iteration that bridged nothing at all logged "bridged 1 gates" and looked
                # like progress. gemm_mac_realbist printed that line thirty times in a row while the router
                # was in fact refusing the net every time (a via-enclosure RECT patch it cannot re-encode
                # XOR-clean), and the run burned its whole ECO budget on an identical no-op.
                set _bridged 0
                if { [file exists ${_trifile}.done] } {
                    set _df [open ${_trifile}.done r]; set _bridged [string trim [read $_df]]; close $_df
                }
                puts_info "\[LOGOS-JOG\] SAT jog-router bridged $_bridged of [llength $_triples] violating gate(s) in-place, no re-route"
                # A jog ECO is a fixpoint: the router is deterministic and the layout it reads back is the
                # one it just wrote, so an iteration that bridges NOTHING will bridge nothing again forever.
                # Stop and name the remedy instead of spinning to the iteration cap.
                # Progress is measured in VIOLATORS, not in bridges. A pass that bridges something and
                # fixes nothing is not progress — measured on systolic_pipe16_realbist, the jog bridged
                # the same 2 of 972 gates on iterations 3, 4 and 5 while the count sat at 657, so a
                # `bridged != 0` test never reached the fallback and would have ground on to the
                # iteration cap.
                set _jog_progress [expr { $_bridged != 0 && ($_prev_nets < 0 || [llength $_nets] < $_prev_nets) }]
                set _prev_nets [llength $_nets]
                if { $_jog_progress } {
                    continue
                }
                # LOGOS DIODE FALLBACK AT THE JOG FIXPOINT (gated by LOGOS_ECO_DIODE_FALLBACK).
                #
                # The jog and the diode ECO were written as EITHER/OR: with ECO_JOG=1 this branch always
                # `continue`d or `return`ed, so the manual-stub diode ECO below was unreachable. The nets
                # the jog cannot touch are therefore exactly the nets nothing ever repaired. Measured on
                # every remaining violator of coproc_core and systolic_pipe16_realbist: the pin ALREADY
                # carries one heuristic diode, the ratio is still 1.0-1.4x, and TritonRoute has patched
                # the net with a via-enclosure RECT the jog cannot round-trip. One diode is not enough
                # diffusion; nothing in the flow would ever add a second.
                #
                # The manual-stub ECO does exactly that: an ADDITIONAL diode at the violating pin itself
                # (the only placement check_antennas credits), wired by a short li1 stub, with NO router
                # run — so no net can be stranded. Repeat only while each pass reduces the count.
                if { [info exists ::env(LOGOS_ECO_DIODE_FALLBACK)] && $::env(LOGOS_ECO_DIODE_FALLBACK) == 1 \
                        && ($_fallback_last < 0 || [llength $_nets] < $_fallback_last) } {
                    set _fallback_last [llength $_nets]
                    puts_info "\[LOGOS-ECO\] jog fixpoint with [llength $_pins] violating pin(s) — falling\
                               through to the manual-stub diode ECO (additional diode per pin, no re-route)"
                    set _pinfile $::env(routing_tmpfiles)/eco_pins_$_it.txt
                    set _f [open $_pinfile w]; foreach _p $_pins { if { [dict exists $_pin_layers $_p] } { foreach _l [lsort -unique [dict get $_pin_layers $_p]] { puts $_f "$_p\t$_l" } } else { puts $_f $_p } }; close $_f
                    set _odb [index_file $::env(routing_tmpfiles)/eco_diodes_$_it.odb]
                    set _def [index_file $::env(routing_tmpfiles)/eco_diodes_$_it.def]
                    manipulate_layout $::env(SCRIPTS_DIR)/odbpy/magic_antenna_eco.py\
                        -indexed_log [index_file $::env(routing_logs)/eco_diodes_$_it.log]\
                        -output $_odb -output_def $_def\
                        --diode-cell $::env(DIODE_CELL) --diode-pin $::env(DIODE_CELL_PIN)\
                        --violator-pins $_pinfile --manual-stub {*}[logos_eco_fanout_arg]
                    set_def $_def
                    set_odb $_odb
                    set ::logos_eco_touched_layout 1
                    continue
                }
                puts_info "\[LOGOS-JOG\] no gate could be bridged this iteration — the jog ECO is at a\
                           fixpoint and further iterations are identical no-ops. See the per-gate\
                           UNBRIDGED lines in [file tail $_joglog]\
                           for why; a via-enclosure RECT patch needs an additional diode\
                           (LOGOS_ECO_DIODE_FALLBACK), an upper-layer detour needs a base-route change."
                return
            }
            set _netfile $::env(routing_tmpfiles)/eco_nets_$_it.txt
            set _f [open $_netfile w]; foreach _n $_nets { puts $_f $_n }; close $_f
            set _pinfile $::env(routing_tmpfiles)/eco_pins_$_it.txt
            set _f [open $_pinfile w]; foreach _p $_pins { if { [dict exists $_pin_layers $_p] } { foreach _l [lsort -unique [dict get $_pin_layers $_p]] { puts $_f "$_p\t$_l" } } else { puts $_f $_p } }; close $_f
            set _odb [index_file $::env(routing_tmpfiles)/eco_diodes_$_it.odb]
            set _def [index_file $::env(routing_tmpfiles)/eco_diodes_$_it.def]
            # Manual-stub mode targets the EXACT violating pin (--violator-pins) so the diode lands on the
            # gate's own branch (credited); the legacy re-route path targets whole nets (--violator-nets).
            set _tgtarg [list --violator-nets $_netfile]
            set _stubarg [list]
            if { [info exists ::env(ECO_MANUAL_STUB)] && $::env(ECO_MANUAL_STUB) == 1 } {
                set _stubarg [list --manual-stub]
                set _tgtarg [list --violator-pins $_pinfile]
            }
            manipulate_layout $::env(SCRIPTS_DIR)/odbpy/magic_antenna_eco.py\
                -indexed_log [index_file $::env(routing_logs)/eco_diodes_$_it.log]\
                -output $_odb -output_def $_def\
                --diode-cell $::env(DIODE_CELL) --diode-pin $::env(DIODE_CELL_PIN)\
                {*}$_tgtarg {*}$_stubarg {*}[logos_eco_fanout_arg]
            set_def $_def
            set_odb $_odb
            set ::logos_eco_touched_layout 1
            if { [info exists ::env(ECO_MANUAL_STUB)] && $::env(ECO_MANUAL_STUB) == 1 } {
                # MANUAL-STUB mode: the odbpy wires each diode with its own li1 stub straight to the sink pin —
                # NO router runs, so the clean base route is untouched and NO net can be stranded. A router
                # re-route (full OR incremental) is the fundamental LVS-breaker: full strands ~800-1200 nets,
                # incremental crashes check_antennas. We skip it entirely and just re-check on the next pass.
                puts_info "\[LOGOS-ECO\] manual-stub: diodes wired by li1 stub in-place, no re-route"
            } else {
                # Full re-route (global+detailed) regenerates guides CONSISTENT with the routing — incremental
                # detailed-only leaves stale guides that crash iteration-2 check_antennas. Allow congestion so
                # global_route ROUTES THROUGH the diode-dense fabric instead of stranding ~1222 nets (the LVS
                # net/device mismatch). repair_antennas off (our diodes already cover every ARC violator).
                set _gra $::env(GRT_REPAIR_ANTENNAS)
                set ::env(GRT_REPAIR_ANTENNAS) 0
                set ::env(GRT_ALLOW_CONGESTION) 1
                global_routing
                detailed_routing
                set ::env(GRT_REPAIR_ANTENNAS) $_gra
            }
        } _msg]
        if { $_code == 1 } {
            puts_info "\[LOGOS-ECO\] iteration $_it aborted ($_msg) — keeping current route"
            return
        } elseif { $_code == 2 } {
            return
        }
    }
    puts_info "\[LOGOS-ECO\] reached max iterations ($max_iters); residual antennas may remain"
}

proc run_routing {args} {
    puts_verbose "Starting routing process..."

    # |----------------------------------------------------|
    # |----------------   5. ROUTING ----------------------|
    # |----------------------------------------------------|

    # LOGOS DIODES-BEFORE-RESIZE (gated by LOGOS_DIODES_BEFORE_RESIZE; base order otherwise).
    #
    # THE ORDER IS THE DEFECT. The resizer below sizes every driver for the load it can SEE, and
    # the diode passes used to run AFTER it, hanging a protection diode on net after net whose
    # driver was already sized. Nothing downstream compensates, so the slew it just repaired is
    # undone by the very next step. Measured twice, on two different blocks:
    #   coproc_core               25 of 69 max-slew violators were ANTENNA_*/DIODE pins
    #   systolic_pipe16_realbist  every one of its ~5 violating driver nets carried diodes;
    #                             all marginal (0.75-0.85 against a 0.75 limit)
    # Treating the symptom failed in both directions. Re-repairing AFTER the diodes
    # (LOGOS_POST_DIODE_RESIZE) re-runs global route and a whole-design repair: it congested at
    # margins 30/0/5 and, where it ran, inserted 3,587 buffers for 7 violations and made slew
    # 69 -> 181. Raising the resizer margins instead hit a memory cliff in the placement resizer
    # (114 GB true peak, OOM-killed, 457k cells) and a 40-minute CPU-bound stall in the
    # routing-stage one, on a step that normally takes about two minutes.
    #
    # So move the cause instead. Heuristic diode insertion is placement-based (diodes.py reads
    # placement and a fanout/length threshold, not routing), so it can legally run first, and
    # then the resizer sizes each driver for the load that will actually exist on silicon.
    set _logos_dbr [expr { [info exists ::env(LOGOS_DIODES_BEFORE_RESIZE)] && $::env(LOGOS_DIODES_BEFORE_RESIZE) == 1 }]
    set _logos_have_diode [expr { [info exists ::env(DIODE_CELL)] && ($::env(DIODE_CELL) ne "") }]

    if { $_logos_dbr && $_logos_have_diode } {
        puts_info "\[LOGOS-DBR\] inserting diodes BEFORE the resizer so it sizes for their load"
        if { $::env(DIODE_ON_PORTS) ne "none" } {
            io_diode_insertion
        }
        if { $::env(RUN_HEURISTIC_DIODE_INSERTION) } {
            heuristic_diode_insertion
        }
    }

    run_resizer_design_routing
    run_resizer_timing_routing

    if { !$_logos_dbr && $_logos_have_diode } {
        if { $::env(DIODE_ON_PORTS) ne "none" } {
            io_diode_insertion
        }
        if { $::env(RUN_HEURISTIC_DIODE_INSERTION) } {
            heuristic_diode_insertion
        }
    }

    # LOGOS targeted pre-route diode insertion (gated by TARGETED_DIODE_NETS; base flow untouched otherwise)
    targeted_diode_insertion

    # LOGOS POST-DIODE SLEW/CAP REPAIR (gated by LOGOS_POST_DIODE_RESIZE; base flow untouched otherwise)
    #
    # THE ORDERING BUG THIS FIXES. run_resizer_design_routing above is what repairs max-slew and
    # max-capacitance: it sizes each driver for the load it can see. The diode passes then run
    # AFTERWARDS and hang a protection diode on net after net — every one of them additional
    # capacitance on a net whose driver was already sized. The resizer never sees that load, so the
    # slew it just repaired is undone by the step that follows it, and no amount of margin can
    # reach the cause because the margin is applied before the load exists.
    #
    # MEASURED on coproc_core (183,740 cells): the final netlist carries 17,716 ANTENNA diode cells,
    # about 10% of the design. Half its max-slew violators are ANTENNA_*/DIODE pins, and its slew
    # count was IDENTICAL (126) at resizer margin 30 and margin 45 — the classic signature of a
    # lever that cannot reach the cause. Removing the diodes instead is not an option: antenna then
    # explodes to 538 violating nets. The diodes are simultaneously required and responsible, which
    # makes this an ORDER problem rather than a knob problem.
    #
    # So: repair once more, now that the diodes are in the netlist and their capacitance is visible.
    # The second pass does NOT need the first pass's strength. The first repair, before the diodes
    # exist, does the bulk of the sizing; this one only has to catch the degradation the diodes
    # then caused. Running it at full margin inserts more buffers than a congested design can route
    # — MEASURED on coproc_core, which hit GRT-0232 at util 25 AND at util 19 with margin 30 here.
    # LOGOS_PDR_SLEW_MARGIN gives this pass its own, smaller margin; it is restored afterwards so
    # nothing downstream sees a mutated environment.
    if { [info exists ::env(LOGOS_POST_DIODE_RESIZE)] && $::env(LOGOS_POST_DIODE_RESIZE) == 1 } {
        set _pdr_saved_slew $::env(GLB_RESIZER_MAX_SLEW_MARGIN)
        set _pdr_saved_cap  $::env(GLB_RESIZER_MAX_CAP_MARGIN)
        if { [info exists ::env(LOGOS_PDR_SLEW_MARGIN)] } {
            set ::env(GLB_RESIZER_MAX_SLEW_MARGIN) $::env(LOGOS_PDR_SLEW_MARGIN)
            set ::env(GLB_RESIZER_MAX_CAP_MARGIN)  $::env(LOGOS_PDR_SLEW_MARGIN)
        }
        puts_info "\[LOGOS-PDR\] re-running slew/cap repair AFTER diode insertion (margin $::env(GLB_RESIZER_MAX_SLEW_MARGIN))"
        run_resizer_design_routing
        set ::env(GLB_RESIZER_MAX_SLEW_MARGIN) $_pdr_saved_slew
        set ::env(GLB_RESIZER_MAX_CAP_MARGIN)  $_pdr_saved_cap
    }

    # LOGOS POST-DIODE DRIVER UPSIZE (gated by LOGOS_POST_DIODE_UPSIZE; base flow untouched otherwise)
    #
    # The surgical successor to the PDR pass above: rather than a whole-design repair, only the nets
    # that violate max slew with their diodes present are touched - driver upsized in place, or the
    # one net buffered when its driver is already the strongest of its function. Measured on
    # systolic_pipe16_realbist: 22 nets, 28 drivers grown, 15 nets rebuffered, +26 cells of 457k,
    # converged in 7 rounds and about five minutes (see logos_post_diode_upsize.tcl).
    # LOGOS DIODE-AWARE FANOUT (on unless LOGOS_DIODE_FANOUT is 0)
    #
    # Every diode pass above hangs loads on nets the resizer has already held to set_max_fanout, and the liberty
    # counts each diode as one. This step splits the nets they took over the limit by whole gates — each with the
    # diodes placed for it — so no diode leaves its gate's net (logos_diode_fanout.tcl). Last before the routers,
    # so it sees every pre-route diode; the post-diode upsize then sizes for slew the nets it built.
    if { ![info exists ::env(LOGOS_DIODE_FANOUT)] || $::env(LOGOS_DIODE_FANOUT) == 1 } {
        run_logos_diode_fanout
    }

    if { [info exists ::env(LOGOS_POST_DIODE_UPSIZE)] && $::env(LOGOS_POST_DIODE_UPSIZE) == 1 } {
        run_logos_post_diode_upsize
    }

    add_route_obs

    #legalize if not yet legalized

    global_routing

    # LOGOS AFTER GLOBAL ROUTING'S ANTENNA REPAIR (the upsize gated by LOGOS_POST_ANTENNA_UPSIZE, the fanout step on
    # unless LOGOS_DIODE_FANOUT is 0)
    # global_routing's repair_antennas hangs diodes on nets the steps above have already sized and split - up to
    # 20 on a single-sink net of wpe_user_project, whose driver was already the strongest of its function, and 26
    # on one ten-gate net. Split what they took over the fanout limit, size once more with those diodes present,
    # then re-route once without antenna repair so detailed routing follows guides for the netlist these steps
    # produced; the repair's own diodes stay, each on its gate's net.
    # Wherever the recipe runs the post-diode upsize, that re-route is the one it judges, even with nothing to split or
    # size after global routing: global routing re-routes the whole design, so the route it makes after the upsize
    # measured is one no slew check at the upsize's detection limit has seen. MEASURED on systolic_pipe16_realbist
    # (RUN_2026.10.16_04.50.56, no antenna repair in global routing): the upsize left no data net over 0.540 ns on its
    # own route; on the one global routing made next, 13 nets estimated 0.63-0.79 ns at the slow corner, and they
    # signed off at 0.76-0.92 ns at the slow corner on max-RC extraction, 39 pins.
    set _logos_post_upsize [expr { [info exists ::env(LOGOS_POST_ANTENNA_UPSIZE)] && $::env(LOGOS_POST_ANTENNA_UPSIZE) == 1 }]
    set _logos_post_fanout [expr { (![info exists ::env(LOGOS_DIODE_FANOUT)] || $::env(LOGOS_DIODE_FANOUT) == 1) \
                                   && $::env(GRT_REPAIR_ANTENNAS) == 1 }]
    set _logos_post_judged [expr { [info exists ::env(LOGOS_POST_DIODE_UPSIZE)] && $::env(LOGOS_POST_DIODE_UPSIZE) == 1 }]
    if { $_logos_post_upsize || $_logos_post_fanout || $_logos_post_judged } {
        if { $_logos_post_fanout } {
            run_logos_diode_fanout
        }
        if { $_logos_post_upsize } {
            run_logos_post_diode_upsize
        }
        if { [info exists ::env(LOGOS_POST_DIODE_UPSIZE)] && $::env(LOGOS_POST_DIODE_UPSIZE) == 1 } {
            run_logos_route_slew
        } else {
            set _logos_gra $::env(GRT_REPAIR_ANTENNAS)
            set ::env(GRT_REPAIR_ANTENNAS) 0
            global_routing
            set ::env(GRT_REPAIR_ANTENNAS) $_logos_gra
        }
    }

    if { $::env(RUN_FILL_INSERTION) } {
        ins_fill_cells
    }

    # Detailed Routing
    if { $::env(RUN_DRT) } {
        detailed_routing
    }

    # LOGOS iterative antenna ECO (gated by DRT_ANTENNA_ECO; base flow untouched otherwise)
    run_antenna_eco

    # Print Wire Lengths + Check Thresholds
    check_wire_lengths

    # Screenshot (If Applicable)
    scrot_klayout -layout $::env(CURRENT_DEF) -log $::env(routing_logs)/screenshot.log

    # Calculate Runtime
    set ::env(timer_routed) [clock seconds]
}

proc run_logos_post_diode_upsize {args} {
    increment_index
    TIMER::timer_start
    set log [index_file $::env(routing_logs)/post_diode_upsize.log]
    puts_info "\[LOGOS-UPSIZE\] upsizing/rebuffering the nets that violate max slew with their diodes present (log: [relpath . $log])..."
    set ::env(GRT_CONGESTION_REPORT_FILE) $::env(routing_tmpfiles)/post-diode-upsize-congestion.rpt
    run_openroad_script $::env(SCRIPTS_DIR)/openroad/logos_post_diode_upsize.tcl\
        -indexed_log $log\
        -save "dir=$::env(routing_tmpfiles),def,sdc,odb,netlist,powered_netlist"
    TIMER::timer_stop
    exec echo "[TIMER::get_runtime]" | python3 $::env(SCRIPTS_DIR)/write_runtime.py "logos post-diode upsize - openroad"
}

# The re-route after the repairs above, with max slew held on the route it chooses: the post-diode upsize in route
# mode measures that route, sizes or isolates the drivers that violate on it, and re-routes only what it changed,
# so detailed routing follows a route whose slew was judged (logos_post_diode_upsize.tcl, LOGOS_UPSIZE_ROUTE).
proc run_logos_route_slew {args} {
    increment_index
    TIMER::timer_start
    set log [index_file $::env(routing_logs)/global.log]
    puts_info "Running Global Routing, max slew held on the route it chooses (log: [relpath . $log])..."
    set ::env(GRT_CONGESTION_REPORT_FILE) $::env(routing_tmpfiles)/groute-congestion.rpt
    set ::env(LOGOS_UPSIZE_ROUTE) 1
    run_openroad_script $::env(SCRIPTS_DIR)/openroad/logos_post_diode_upsize.tcl\
        -indexed_log $log\
        -save "def=[index_file $::env(routing_tmpfiles)/global.def],guide=[index_file $::env(routing_tmpfiles)/global.guide],odb=[index_file $::env(routing_tmpfiles)/global.odb]"\
        -no_update_current
    unset ::env(LOGOS_UPSIZE_ROUTE)
    set_def $::env(SAVE_DEF)
    set_guide $::env(SAVE_GUIDE)
    set_odb $::env(SAVE_ODB)
    unset ::env(SAVE_DEF)
    unset ::env(SAVE_GUIDE)
    unset ::env(SAVE_ODB)
    write_verilog\
        $::env(routing_tmpfiles)/global.nl.v\
        -powered_to $::env(routing_tmpfiles)/global.pnl.v\
        -indexed_log [index_file $::env(routing_logs)/global_write_netlist.log]
    TIMER::timer_stop
    run_sta -no_save -log $::env(routing_logs)/grt_sta.log
    exec echo "[TIMER::get_runtime]" | python3 $::env(SCRIPTS_DIR)/write_runtime.py "logos global routing, slew held - openroad"
}

proc run_logos_diode_fanout {args} {
    increment_index
    TIMER::timer_start
    set log [index_file $::env(routing_logs)/diode_fanout.log]
    puts_info "\[LOGOS-FANOUT\] splitting the nets their diodes took over the fanout limit, each diode kept on its gate's net (log: [relpath . $log])..."
    set ::env(GRT_CONGESTION_REPORT_FILE) $::env(routing_tmpfiles)/diode-fanout-congestion.rpt
    run_openroad_script $::env(SCRIPTS_DIR)/openroad/logos_diode_fanout.tcl\
        -indexed_log $log\
        -save "dir=$::env(routing_tmpfiles),def,sdc,odb,netlist,powered_netlist"
    TIMER::timer_stop
    exec echo "[TIMER::get_runtime]" | python3 $::env(SCRIPTS_DIR)/write_runtime.py "logos diode-aware fanout - openroad"
}

proc run_resizer_design_routing {args} {
    if { $::env(GLB_RESIZER_DESIGN_OPTIMIZATIONS) == 1} {
        increment_index
        TIMER::timer_start
        set log [index_file $::env(routing_logs)/resizer_design.log]
        puts_info "Running Global Routing Resizer Design Optimizations (log: [relpath . $log])..."

        set ::env(GRT_CONGESTION_REPORT_FILE) $::env(routing_tmpfiles)/resizer-routing-design-congestion.rpt
        run_openroad_script $::env(SCRIPTS_DIR)/openroad/resizer_routing_design.tcl\
            -indexed_log $log\
            -save "dir=$::env(routing_tmpfiles),def,sdc,odb,netlist,powered_netlist"

        TIMER::timer_stop
        exec echo "[TIMER::get_runtime]" | python3 $::env(SCRIPTS_DIR)/write_runtime.py "resizer design optimizations - openroad"
        run_sta -estimate_global -no_save -log $::env(routing_logs)/rsz_design_sta.log
    } else {
        puts_info "Skipping Global Routing Resizer Design Optimizations."
    }
}

proc run_resizer_timing_routing {args} {
    if { $::env(GLB_RESIZER_TIMING_OPTIMIZATIONS) == 1} {
        increment_index
        TIMER::timer_start
        set log [index_file $::env(routing_logs)/resizer_timing.log]
        puts_info "Running Global Routing Resizer Timing Optimizations (log: [relpath . $log])..."

        set ::env(GRT_CONGESTION_REPORT_FILE) $::env(routing_tmpfiles)/resizer-routing-timing-congestion.rpt
        run_openroad_script $::env(SCRIPTS_DIR)/openroad/resizer_routing_timing.tcl\
            -indexed_log $log\
            -save "dir=$::env(routing_tmpfiles),def,sdc,odb,netlist,powered_netlist"

        TIMER::timer_stop
        exec echo "[TIMER::get_runtime]" | python3 $::env(SCRIPTS_DIR)/write_runtime.py "resizer timing optimizations - openroad"
        run_sta -estimate_global -no_save -log $::env(routing_logs)/rsz_timing_sta.log
    } else {
        puts_info "Skipping Global Routing Resizer Timing Optimizations."
    }
}
