//! openlane_tcl_lever_lock — a flow setting that reaches no tool is worse than no setting at all.
//!
//! WHY THIS EXISTS. Tcl's `lappend` takes a VARIABLE NAME as its first operand:
//!     lappend arg_list -max_slew $value      # correct: appends two words to arg_list
//!     lappend -max_slew $value               # legal Tcl, and a silent disaster: it creates a
//!                                            # variable literally named "-max_slew" and the flag
//!                                            # never reaches the command
//! The second form raises no error, so the surrounding `{*}$arg_list` call simply runs without
//! the option and the flow reports success.
//!
//! MEASURED — three of these were live in the vendored OpenLane when this lock was written, and
//! every one of them silently discarded a signoff-relevant constraint on every run ever made:
//!
//!   scripts/openroad/cts.tcl:33   lappend -max_cap  ...CTS_MAX_CAP...
//!   scripts/openroad/cts.tcl:35   lappend -max_slew ...MAX_TRANSITION_CONSTRAINT...
//!       `cts_characterization_args` was built empty and never appended to, so
//!       `configure_cts_characterization {*}$cts_characterization_args` ran with NO ARGUMENTS.
//!       Clock tree synthesis never received the max-capacitance or max-transition constraint.
//!
//!   scripts/openroad/resizer_routing_design.tcl:61  lappend -max_wire_length ...
//!       `GLB_RESIZER_MAX_WIRE_LENGTH` never reached `repair_design`. Setting it to 600 on
//!       coproc_core produced a GDS byte-identical to the run without it (114,879,296 bytes,
//!       182,029 cells, slew 69, antenna 14/7) — which is how it was caught: a lever that
//!       changes nothing changes NOTHING, right down to the file size.
//!
//! This class of bug is invisible from the outside: the setting appears in config.tcl, the run
//! completes, and the metric simply does not move. The only cheap defence is to refuse the
//! syntax outright.

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tcl_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            tcl_files(&p, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some("tcl") {
            out.push(p);
        }
    }
}

/// No `lappend` may take a flag where a variable name belongs — in the OpenLane changes this repo tracks
/// (scripts/openlane-overlay, where the repaired `cts.tcl` and `resizer_routing_design.tcl` live), and in the
/// whole fetched OpenLane wherever one is installed.
#[test]
fn no_openlane_tcl_appends_a_flag_instead_of_a_variable() {
    let mut files = Vec::new();
    tcl_files(&root().join("scripts/openlane-overlay/scripts"), &mut files);
    assert!(
        files.len() >= 5,
        "only {} tcl files in the OpenLane overlay — the walk is not working, so this lock would pass vacuously",
        files.len()
    );
    if common::require_tool("eda-tools") {
        let installed = root().join("tools/OpenLane/scripts");
        let before = files.len();
        tcl_files(&installed, &mut files);
        assert!(
            files.len() - before >= 20,
            "only {} tcl files in the installed OpenLane — the walk is not working",
            files.len() - before
        );
    }
    files.sort();

    let mut bad = Vec::new();
    for f in &files {
        let Ok(text) = std::fs::read_to_string(f) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with('#') {
                continue;
            }
            let Some(rest) = t.strip_prefix("lappend") else {
                continue;
            };
            let Some(first) = rest.split_whitespace().next() else {
                continue;
            };
            if first.starts_with('-') {
                bad.push(format!(
                    "  {}:{}  {}",
                    f.strip_prefix(root()).unwrap_or(f).display(),
                    i + 1,
                    t.trim()
                ));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} Tcl `lappend` call(s) pass a FLAG where a variable name belongs. Each one silently \
         drops a tool option: Tcl creates a variable named after the flag, the option never \
         reaches the command, and the flow reports success with the setting having done nothing. \
         A lever that cannot move its metric is indistinguishable from a design that does not \
         respond to it — which is how a whole measurement campaign gets built on sand.\n{}\n\n\
         Fix: `lappend <the_arg_list_variable> <flag> <value>`.",
        bad.len(),
        bad.join("\n")
    );
}

/// The CTS characterization list must actually be populated before it is used.
///
/// Narrower than the syntax check above and aimed at the specific damage: this call decides the
/// clock tree's slew and capacitance targets, and it was being made with an empty argument list.
#[test]
fn cts_characterization_receives_the_constraints_it_is_given() {
    let f = root().join("scripts/openlane-overlay/scripts/openroad/cts.tcl");
    let text = std::fs::read_to_string(&f).expect("read cts.tcl");
    assert!(
        text.contains("configure_cts_characterization"),
        "cts.tcl no longer configures CTS characterization at all"
    );
    for (env, flag) in [
        ("CTS_MAX_CAP", "-max_cap"),
        ("MAX_TRANSITION_CONSTRAINT", "-max_slew"),
    ] {
        assert!(
            text.contains(&format!("lappend cts_characterization_args {flag}")),
            "cts.tcl reads {env} but does not append {flag} to `cts_characterization_args`, so \
             `configure_cts_characterization` is invoked without it and the clock tree is built \
             ignoring that constraint entirely."
        );
    }
}
