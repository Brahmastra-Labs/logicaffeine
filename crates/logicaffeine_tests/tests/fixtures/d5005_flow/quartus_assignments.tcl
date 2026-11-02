# Quartus Prime's assignment commands, as a project's settings files call them, so a project's settings can be read
# without Quartus: `tclsh quartus_assignments.tcl <qsf>`, run from the project's directory, evaluates the QSF as Quartus
# does — each SOURCE_TCL_SCRIPT_FILE it assigns sourced where it is assigned, at global scope, its path taken from the
# project's directory — and prints every assignment made, in order, one a line, its fields separated by tabs:
#
#   global <name> <value>             set_global_assignment
#   instance <name> <value> <to>      set_instance_assignment
#   location <value> <to>             set_location_assignment
#   missing <path>                    a SOURCE_TCL_SCRIPT_FILE that is not there, passed over as Quartus passes it over
#                                     (Warning 125092, "Tcl Script file … not found")
#
# A file's value (the names below) is printed as an absolute path. An assignment read back before it is made reads as
# an error, as every read of one in the FIM's settings is made under `catch`. Quartus's Tcl gives a command it does not
# know that is a bus index — a number, a range or `*` — back as its bracketed text, so `mem_bank[0]` and `PCIE_Tx[*]`
# name nodes in settings and scripts alike, unescaped. A posted error stops the evaluation, and so does any other
# command none of these stands in for: a settings file that comes to call one is read again rather than read wrong.

set ::file_names {SOURCE_TCL_SCRIPT_FILE SYSTEMVERILOG_FILE VERILOG_FILE VHDL_FILE IP_FILE QSYS_FILE SDC_FILE QIP_FILE
    MISC_FILE TEXT_FILE}
set ::made {}
set ::global [dict create]

# The -name, the value and the -to of an assignment's arguments; every other option and its argument passed over.
proc parsed {args} {
    set name ""
    set to ""
    set values {}
    set removed 0
    for {set i 0} {$i < [llength $args]} {incr i} {
        set a [lindex $args $i]
        switch -exact -- $a {
            -name { incr i; set name [lindex $args $i] }
            -to { incr i; set to [lindex $args $i] }
            -from - -entity - -section_id - -tag - -library - -hdl_version - -comment { incr i }
            -remove { set removed 1 }
            -disable { }
            default { lappend values $a }
        }
    }
    return [list $name [lindex $values 0] $to $removed]
}

proc set_global_assignment {args} {
    lassign [parsed {*}$args] name value to removed
    if {$removed} {
        error "set_global_assignment -remove is not stood in for: $args"
    }
    if {$name in $::file_names} {
        set value [file normalize $value]
    }
    lappend ::made [list global $name $value]
    dict set ::global $name $value
    if {$name eq "SOURCE_TCL_SCRIPT_FILE"} {
        if {[file exists $value]} {
            uplevel #0 [list source $value]
        } else {
            lappend ::made [list missing $value]
        }
    }
}

proc get_global_assignment {args} {
    lassign [parsed {*}$args] name
    if {![dict exists $::global $name]} {
        error "no $name assignment is made"
    }
    return [dict get $::global $name]
}

# A collection of the global assignments of a name made so far, in order, as an assignment object each; walked by
# foreach_in_collection and read by get_assignment_info -value.
proc get_all_assignments {args} {
    set type ""
    for {set i 0} {$i < [llength $args]} {incr i} {
        if {[lindex $args $i] eq "-type"} {
            incr i
            set type [lindex $args $i]
        }
    }
    if {$type ne "global"} {
        error "get_all_assignments -type $type is not stood in for: $args"
    }
    lassign [parsed {*}$args] name
    set found {}
    foreach m $::made {
        if {[lindex $m 0] eq "global" && [lindex $m 1] eq $name} {
            lappend found $m
        }
    }
    return $found
}

proc foreach_in_collection {var collection body} {
    upvar 1 $var each
    foreach each $collection {
        uplevel 1 $body
    }
}

proc get_assignment_info {option object} {
    if {$option ne "-value"} {
        error "get_assignment_info $option is not stood in for"
    }
    return [lindex $object 2]
}

proc set_instance_assignment {args} {
    lassign [parsed {*}$args] name value to removed
    if {$removed} {
        error "set_instance_assignment -remove is not stood in for: $args"
    }
    lappend ::made [list instance $name $value $to]
}

proc set_location_assignment {args} {
    lassign [parsed {*}$args] name value to
    lappend ::made [list location $value $to]
}

proc post_message {args} {
    set type info
    set text [lindex $args end]
    for {set i 0} {$i < [llength $args] - 1} {incr i} {
        if {[lindex $args $i] eq "-type"} {
            incr i
            set type [lindex $args $i]
        }
    }
    if {$type eq "error"} {
        error "the settings posted an error: $text"
    }
}

if {[llength $argv] != 1} {
    puts stderr "usage: tclsh quartus_assignments.tcl <qsf>"
    exit 2
}
rename unknown tcl_unknown
proc unknown {args} {
    if {[llength $args] == 1 && [regexp {^([0-9]+|[0-9]+:[0-9]+|\*)$} [lindex $args 0]]} {
        return "\[[lindex $args 0]\]"
    }
    error "invalid command name \"[lindex $args 0]\": no stand-in for it"
}

source [lindex $argv 0]
foreach m $::made {
    puts [join $m "\t"]
}
