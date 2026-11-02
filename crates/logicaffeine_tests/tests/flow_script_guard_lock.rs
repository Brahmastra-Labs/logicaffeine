//! flow_script_guard_lock — the four pre-flight checks `openlane-run.sh` must refuse to start
//! without. Each exists because its absence cost a real run today, and each failure mode was
//! expensive precisely because it did NOT look like a failure.
//!
//!  1. CLOCK PORT ON THE TOP MODULE. `openlane-run.sh dram_executor_wb clk 10` on a module whose
//!     clock is `wb_clk_i` ran the full flow and produced DRC 0 / LVS 0 / XOR 0 / antenna 0/0 /
//!     routing 0 / 0 violated paths / wns 0.00 — every number true and every number meaningless,
//!     because STA reported `worst slack INF`: nothing was timed. It also skipped CTS, halving
//!     the cell count so it read as an improvement. An unconstrained design passes every timing
//!     gate by having no paths rather than by meeting them. The check must be scoped to the TOP
//!     module's port list: that very file declares an inner `dram_executor` whose port IS `clk`,
//!     so a file-wide grep matches and the guard passes while the design stays unconstrained.
//!
//!  2. ROUTING_CORES PINNED. detailed_route is not deterministic across thread counts, so an
//!     unpinned run changes the RESULT and not merely the runtime. `set -u` turned the omission
//!     into a bare "unbound variable" with no hint of what to set or why.
//!
//!  3. MEMORY HEADROOM. Kernel log, 2026-09-22:
//!         Out of memory: Killed process 1293216 (openroad) anon-rss:98749324kB
//!     One openroad reached 98.7 GB on a 125 GB box. THREE runs "failed" with an EMPTY .errors
//!     file and a log stopping mid-step — a SIGKILL leaves no diagnostic, so an OOM is
//!     indistinguishable from a crash. The bound is RESIDENT MEMORY: ROUTING_CORES pins threads
//!     and does nothing about RSS, so a policy of "at most N flows" is calibrated on the wrong
//!     variable. Die area drives it as much as cell count — the same 457k-cell design held
//!     91 GB at placement at util 25 and completed at util 28.
//!
//!  4. OUT_JSON FOLLOWS THE BLOCK. Defaulting to sky130_signoff.json wrote a SECOND record for a
//!     block first recorded elsewhere; systolic_os_pipe8_realbist then read signoff_clean=TRUE
//!     from one ledger and FALSE from another. See `ledger_uniqueness_lock`.

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn runner() -> String {
    std::fs::read_to_string(root().join("scripts/openlane-run.sh")).expect("read openlane-run.sh")
}

#[test]
fn the_clock_port_is_checked_against_the_top_modules_port_list() {
    let s = runner();
    assert!(
        s.contains("is not a port of top module"),
        "the clock-port pre-flight is gone. Without it, a clock that is not a top-level port \
         yields `worst slack INF` and every timing gate passes vacuously."
    );
    // Asserted as two independent facts rather than one brittle escaped literal: the search
    // must be anchored on the word `module` AND parameterised by the top-module name. The
    // first attempt at this assertion over-escaped the backslashes and could never match
    // correct code — a literal that cannot be true is a lock that only ever reports its own bug.
    assert!(
        s.contains("re.escape(top)") && s.contains("bmodule") && s.contains("re.search"),
        "the clock-port check must parse the TOP MODULE's own header. A file-wide search is not \
         enough: dram_executor_wb.v declares an inner module whose port IS `clk`, so a whole-file \
         grep passes while the top module stays unconstrained — measured, not hypothetical."
    );
}

#[test]
fn routing_cores_must_be_pinned_explicitly() {
    let s = runner();
    assert!(
        s.contains("ROUTING_CORES is not set") && s.contains("exit 4"),
        "the ROUTING_CORES guard is gone. detailed_route is not deterministic across thread \
         counts, so an unpinned run changes the result, not just the runtime."
    );
}

#[test]
fn the_flow_refuses_to_start_without_memory_headroom() {
    let s = runner();
    assert!(
        s.contains("OL_MIN_FREE_GB"),
        "the memory pre-flight is gone. One openroad reached 98.7 GB resident and was OOM-killed; \
         three runs were lost to it, each presenting as an empty .errors file rather than an error."
    );
    assert!(
        s.contains("free -g") && s.contains("exit 4"),
        "the memory guard must actually measure free memory and REFUSE, not warn. A warning in a \
         multi-hour flow is read after the OOM, not before it."
    );
}

#[test]
fn the_recorder_writes_to_the_ledger_the_block_already_lives_in() {
    let s = runner();
    assert!(
        s.contains("if [ -z \"${OUT_JSON:-}\" ]; then") && s.contains("sky130_ledgers.json"),
        "OUT_JSON must be resolved by searching the registered ledgers (benchmarks/silicon/sky130_ledgers.json) \
         for the block. Defaulting to sky130_signoff.json duplicates any block first recorded elsewhere, and a \
         block with two records can read clean from one file and not-clean from another."
    );
}

/// The runner's whole body is ONE function, invoked on its last line.
///
/// Bash reads a plain script incrementally from a byte offset, so editing this file while a flow
/// executed it made the running shell resume inside different text. That happened three times:
/// once it lost the recording step of a successful route to a transient syntax error. A function
/// body is parsed in full before its first command runs, so a mid-run edit cannot reach a running
/// shell. Anything placed after the call, or outside the function, reopens the hole.
#[test]
fn the_runner_body_is_parsed_whole_before_it_runs() {
    let s = runner();
    let def = s
        .find("\nlogos_openlane_run_main() {\n")
        .expect("the body must be wrapped in logos_openlane_run_main()");
    let strict = s.find("set -euo pipefail\n").expect("strict mode");
    assert!(
        strict < def,
        "strict mode is set before the function so it governs the body"
    );
    let before: Vec<&str> = s[..def]
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .collect();
    assert_eq!(
        before,
        vec!["set -euo pipefail"],
        "no command may run before the function is fully parsed: found {before:?}"
    );
    let last = s.trim_end().lines().last().unwrap_or("");
    assert_eq!(last, "logos_openlane_run_main \"$@\"; exit $?",
        "the call and the exit must share the LAST line: bash reads each next command from its old \
         byte offset, so an exit on its own line can land inside changed text after the flow returns");
    let close = s
        .trim_end()
        .lines()
        .rev()
        .find(|l| {
            !l.trim_start().starts_with('#')
                && l.trim() != "logos_openlane_run_main \"$@\"; exit $?"
        })
        .unwrap_or("");
    assert_eq!(
        close, "}",
        "the function must close immediately before the call (comments aside)"
    );
}

/// Every lever the flow-config emitter accepts must be reachable from the runner.
///
/// A compiler lever the runner cannot forward is only reachable through `OL_EXTRA_JSON`, which
/// writes config keys behind the compiler's back — the measured value and its justification then
/// never travel with the run. `--pdk` and `--cores` are always passed. `--fixed-die`, `--ports`
/// and `--macro` are Caravel-wrapper geometry, which a per-block harden never builds (the wrapper's
/// own harden, `caravel-harden.sh`, is their caller-to-be). `--vehicle-macro`, `--hold-margin` and
/// `--macro-grid` build the Caravel USER MACRO, so `caravel-macro-harden.sh` must pass them.
#[test]
fn every_emitter_lever_is_forwarded_by_the_runner() {
    let emit = std::fs::read_to_string(
        root().join("crates/logicaffeine_compile/examples/emit_flow_config.rs"),
    )
    .unwrap();
    let s = runner();
    let mut flags = Vec::new();
    for line in emit.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("\"--") {
            if let Some(end) = rest.find("\" =>") {
                flags.push(format!("--{}", &rest[..end]));
            }
        }
    }
    assert!(
        flags.len() >= 12,
        "parsed too few emitter flags ({flags:?}) — the parser is stale"
    );
    let always = s.contains("CFG_ARGS=( \"$BLOCK\" \"$CLKP\" \"$PERIOD\" --pdk \"${PDK:-sky130A}\" --cores \"$ROUTING_CORES\" )");
    assert!(always, "--pdk and --cores must be passed on every run");
    let caravel_macro =
        std::fs::read_to_string(root().join("scripts/caravel-macro-harden.sh")).unwrap();
    for f in &flags {
        match f.as_str() {
            "--pdk" | "--cores" | "--fixed-die" | "--ports" | "--macro" => {}
            "--vehicle-macro" | "--hold-margin" | "--macro-grid" => assert!(caravel_macro.contains(f.as_str()),
                "{f} configures the Caravel user macro, so caravel-macro-harden.sh must pass it"),
            _ => assert!(s.contains(&format!("CFG_ARGS+=( {f}")),
                "emit_flow_config accepts {f} but openlane-run.sh never forwards it — the lever reaches no run"),
        }
    }
}

/// The prebuilt emitter must not be older than the compiler source it came from.
///
/// MEASURED: it was a full day stale while `flow_config.rs` had been edited that afternoon; two new
/// levers emitted nothing and every run that day used the previous day's defaults.
#[test]
fn a_stale_flow_config_emitter_is_refused() {
    let s = runner();
    let guard = s
        .find("is NEWER than the prebuilt flow-config emitter")
        .expect("stale-emitter guard");
    let call = s
        .find("if ! \"$EMIT\" \"${CFG_ARGS[@]}\"")
        .expect("emitter call");
    assert!(
        guard < call,
        "the staleness check must run BEFORE the emitter is used"
    );
    for src in [
        "codegen_sva/flow_config.rs",
        "codegen_sva/pdk_target.rs",
        "codegen_sva/signoff_blocks.rs",
        "examples/emit_flow_config.rs",
    ] {
        assert!(
            s[..call].contains(src),
            "the guard must compare {src} against the binary"
        );
    }
    assert!(
        s[..call].contains("-nt \"$EMIT\""),
        "the guard compares modification times with -nt"
    );
}

/// Re-signing a recorded block re-runs the recipe its evidence was built with, or nothing. A flow fix is
/// re-proven on blocks already signed off (the antenna ECO's netlist write, 2026-09-28, took `coproc_core`,
/// `infinichip`, `fabric_horner_seq` and `systolic_pipe16_realbist` back to not clean), and `openlane-run.sh`
/// would rebuild them from today's `ClosureSettings`. `scripts/resign-block.sh` runs the design's own
/// config.json, refused unless it is exactly what the run the ledger reproduces from started with.
#[test]
fn a_re_sign_runs_the_recorded_recipe_or_refuses() {
    let sh = std::fs::read_to_string(root().join("scripts/resign-block.sh"))
        .expect("read resign-block.sh");
    assert!(
        sh.trim_end().ends_with("logos_resign_main \"$@\"; exit $?"),
        "the body is one function, called and exited on one line"
    );
    let apply = sh
        .find("bash \"$ROOT/scripts/openlane-overlay/apply.sh\"")
        .expect("the overlay is laid first");
    let run = sh
        .find("make quick_run QUICK_RUN_DESIGN=$DES")
        .expect("the recipe is run");
    let check = sh
        .find("[ \"$CHECK\" = 1 ] && return 0")
        .expect("--check stops after the refusals");
    let recipe = sh.find("<<'PYRECIPE'").unwrap();
    assert!(
        recipe < check && check < apply && apply < run,
        "refusals, then --check's exit, then the overlay, then the flow"
    );
    assert!(sh.contains("\"--verify\", led") && sh.contains("b.get(\"status\") != \"reproduces\""),
        "the evidence run is the one the ledger entry reproduces from, found by the recorder's own verifier");
    assert!(
        sh.contains("-newer \"$EVIDENCE/config_in.tcl\""),
        "RTL newer than the evidence run is refused"
    );
    assert!(
        sh.contains("OL_MIN_FREE_GB:-80"),
        "the memory headroom every flow launch requires"
    );
    assert!(sh.contains("record-sky130-signoff.py\" \"$RUN\" \"$BLOCK\" \"$LEDGER\"") && sh.contains("run-golden-precheck-drc.sh\" \"$BLOCK\" \"$GDS\""),
        "the new run is recorded into the ledger that holds the block, and golden-checked on the GDS it ships");

    // The recipe check itself, on a recorded design, against tampered copies of its config.
    let body = &sh[recipe + "<<'PYRECIPE'".len()..];
    let body = &body[body.find('\n').unwrap() + 1..body.find("\nPYRECIPE").unwrap()];
    let tmp = std::env::temp_dir().join(format!("resign-recipe-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("recipe.py"), body).unwrap();
    std::fs::write(tmp.join("config_in.tcl"), concat!(
        "set ::env(DESIGN_DIR) /openlane/designs/logos_x\n",
        "set ::env(PDK) \"sky130A\"\n",
        "set ::env(DESIGN_NAME) \"x\"\n",
        "set ::env(VERILOG_FILES) \"/openlane/designs/logos_x/src/a.v /openlane/designs/logos_x/src/b.v\"\n",
        "set ::env(PL_TARGET_DENSITY) \"0.45\"\n",
        "set ::env(ROUTING_CORES) \"4\"\n",
    )).unwrap();
    let judge = |cfg: &str, pdk: &str| -> bool {
        std::fs::write(tmp.join("config.json"), cfg).unwrap();
        std::process::Command::new("python3")
            .env("PDK", pdk)
            .arg(tmp.join("recipe.py"))
            .arg(tmp.join("config.json"))
            .arg(tmp.join("config_in.tcl"))
            .arg("/x/designs/logos_x")
            .output()
            .expect("python3")
            .status
            .success()
    };
    let same = r#"{"DESIGN_NAME": "x", "VERILOG_FILES": "dir::src/*.v", "PL_TARGET_DENSITY": 0.45, "ROUTING_CORES": 4}"#;
    assert!(judge(same, "sky130A"), "the recorded recipe is accepted");
    assert!(
        !judge(&same.replace("0.45", "0.5"), "sky130A"),
        "a changed value is refused"
    );
    assert!(
        !judge(&same.replace(", \"ROUTING_CORES\": 4", ""), "sky130A"),
        "a removed key is refused"
    );
    assert!(
        !judge(&same.replace("}", ", \"GRT_ADJUSTMENT\": 0.3}"), "sky130A"),
        "an added key is refused"
    );
    assert!(
        !judge(&same.replace("dir::src/*.v", "dir::rtl/*.v"), "sky130A"),
        "sources from elsewhere are refused"
    );
    assert!(!judge(&same.replace("}", ", \"pdk::sky130*\": {\"X\": 1}}"), "sky130A"),
        "a pdk:: block is refused: config_in.tcl holds what OpenLane resolved it to, never the block");
    assert!(!judge(same, "gf180mcuC"), "another PDK is refused");
    std::fs::remove_dir_all(&tmp).ok();

    // On this box, a real recorded block passes the refusals.
    if common::require_tool("eda-tools")
        && root()
            .join("tools/OpenLane/designs/logos_gpu_simt/runs")
            .is_dir()
    {
        let out = std::process::Command::new("bash")
            .arg(root().join("scripts/resign-block.sh"))
            .args(["--check", "gpu_simt"])
            .current_dir(root())
            .output()
            .expect("run resign-block.sh --check");
        let log = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.status.success() && log.contains("gpu_simt: recipe of RUN_"),
            "{log}"
        );
    }
}

/// Every script that launches an OpenLane flow is parsed whole before it runs, for the reason
/// `the_runner_body_is_parsed_whole_before_it_runs` gives for `openlane-run.sh`: a flow runs for hours, bash
/// reads a plain script incrementally, and an edit to the script under a running flow resumes the shell inside
/// different text. `caravel-macro-harden.sh` held the WPE's run 11 (RUN_2026.09.27_18.14.14) for 25 hours while
/// its next edit waited, because nothing but care kept that edit from reaching it.
#[test]
fn every_flow_launcher_is_parsed_whole_before_it_runs() {
    let dir = root().join("scripts");
    let mut launchers = 0;
    for e in std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()) {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("sh") {
            continue;
        }
        let s = std::fs::read_to_string(&p).unwrap();
        if !s.contains("make quick_run") {
            continue;
        }
        launchers += 1;
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let def_at = s.find("\nlogos_").and_then(|i| {
            let line = &s[i + 1..i + 1 + s[i + 1..].find('\n').unwrap_or(0)];
            (line.ends_with("_main() {")).then_some((i, line.trim_end_matches("() {").to_string()))
        });
        let (def, main) = def_at
            .unwrap_or_else(|| panic!("{name}: the body must be one `logos_*_main() {{` function"));
        let before: Vec<&str> = s[..def]
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .collect();
        assert_eq!(
            before,
            vec!["set -euo pipefail"],
            "{name}: nothing but strict mode may run before the body is parsed"
        );
        let call = format!("{main} \"$@\"; exit $?");
        assert_eq!(
            s.trim_end().lines().last().unwrap_or(""),
            call,
            "{name}: the call and the exit share the last line"
        );
        let close = s
            .trim_end()
            .lines()
            .rev()
            .find(|l| !l.trim_start().starts_with('#') && l.trim() != call)
            .unwrap_or("");
        assert_eq!(
            close, "}",
            "{name}: the function closes immediately before the call"
        );
    }
    assert!(launchers >= 4, "found {launchers} flow launchers; openlane-run, resign-block and both Caravel hardens launch flows");
}

/// The scripts a flow launcher runs: every launcher, then every `scripts/…` file named by one already found.
fn flow_script_closure() -> Vec<(String, String)> {
    let dir = root().join("scripts");
    let mut todo: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| {
            n.ends_with(".sh")
                && std::fs::read_to_string(dir.join(n))
                    .unwrap_or_default()
                    .contains("make quick_run")
        })
        .collect();
    let mut seen: Vec<(String, String)> = Vec::new();
    while let Some(name) = todo.pop() {
        if seen.iter().any(|(n, _)| *n == name) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(dir.join(&name)) else {
            continue;
        };
        for (at, _) in text.match_indices("scripts/") {
            let tail = &text[at + "scripts/".len()..];
            let end = tail
                .find(|c: char| !(c.is_ascii_alphanumeric() || "_./-".contains(c)))
                .unwrap_or(tail.len());
            let named = tail[..end].trim_end_matches('.');
            if [".sh", ".py", ".tcl"].iter().any(|x| named.ends_with(x)) {
                todo.push(named.to_string());
            }
        }
        seen.push((name, text));
    }
    seen
}

/// Where a script names the host's `/tmp`: an occurrence that does not continue a longer path (`$RUN/tmp` is the
/// run's own scratch directory and stays with it).
fn host_tmp_uses(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| {
            line.match_indices("/tmp").any(|(i, _)| {
                let before = line[..i].chars().last();
                let after = line[i + 4..].chars().next();
                !before.is_some_and(|c| c.is_ascii_alphanumeric() || "_}).-$".contains(c))
                    && !after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            })
        })
        .map(|l| l.trim().to_string())
        .collect()
}

/// Nothing a flow writes or reads back passes through the host's `/tmp`. The golden DRC reports lived at
/// `/tmp/<key>_golden_drc.xml` between `run-golden-precheck-drc.sh` and `record-caravel-signoff.py`: the
/// wrapper's golden verdict was a file a reboot or a tmp cleaner deletes, under a name every checkout on the box
/// shares. `openlane-run.sh` captured the emitter's errors in one fixed `/tmp/emit_flow_config.err`, which every
/// concurrent launch wrote (coproc_core and fabric_horner_seq ran side by side on 2026-09-28). The golden reports
/// now sit beside the GDS they judge, and the wrapper's recorder reads them from there.
#[test]
fn no_flow_script_keeps_anything_in_the_hosts_tmp() {
    assert_eq!(
        host_tmp_uses("x=/tmp/a.xml"),
        vec!["x=/tmp/a.xml"],
        "a bare /tmp path is found"
    );
    assert_eq!(
        host_tmp_uses("cmd 2>/tmp/e.err"),
        vec!["cmd 2>/tmp/e.err"],
        "a redirect into /tmp is found"
    );
    assert_eq!(
        host_tmp_uses("docker run -v /tmp:/out img"),
        vec!["docker run -v /tmp:/out img"],
        "a /tmp mount is found"
    );
    assert!(
        host_tmp_uses("TMP=\"$RUN/tmp\"; x $CRUN/tmp/a.tcl ${D}/tmp /x/tmpfs").is_empty(),
        "a run's own tmp is not the host's"
    );

    let closure = flow_script_closure();
    let names: Vec<&str> = closure.iter().map(|(n, _)| n.as_str()).collect();
    for needed in [
        "openlane-run.sh",
        "caravel-harden.sh",
        "caravel-macro-harden.sh",
        "resign-block.sh",
        "run-golden-precheck-drc.sh",
        "record-caravel-signoff.py",
        "record-sky130-signoff.py",
        "anchor-ir-drop.sh",
    ] {
        assert!(
            names.contains(&needed),
            "the flow closure must reach {needed}; found {names:?}"
        );
    }
    for (name, text) in &closure {
        let uses = host_tmp_uses(text);
        assert!(
            uses.is_empty(),
            "{name} passes flow files through the host's /tmp: {uses:?}"
        );
    }

    let golden = closure
        .iter()
        .find(|(n, _)| n == "run-golden-precheck-drc.sh")
        .unwrap()
        .1
        .as_str();
    assert!(
        golden.contains("GXML=\"$ROOT/${GDS%.gds}.golden_drc.xml\"")
            && golden.contains("ZXML=\"$ROOT/${GDS%.gds}.zeroarea.xml\""),
        "the golden reports are written beside the GDS they judge"
    );
    let wrapper = closure
        .iter()
        .find(|(n, _)| n == "caravel-harden.sh")
        .unwrap()
        .1
        .as_str();
    assert!(
        wrapper.contains("\"$RUN\" \"${GDS%.gds}.golden_drc.xml\" \"$CP_MACRO\""),
        "the wrapper's recorder reads the golden report beside the wrapper GDS it just checked"
    );
}

fn shell_refs(line: &str) -> Vec<String> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'$' {
            let mut j = i + 1;
            if j < b.len() && b[j] == b'{' {
                j += 1;
            }
            let start = j;
            while j < b.len()
                && (b[j].is_ascii_uppercase()
                    || b[j] == b'_'
                    || (j > start && b[j].is_ascii_digit()))
            {
                j += 1;
            }
            if j > start {
                out.push(line[start..j].to_string());
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

fn shell_target(line: &str) -> Option<String> {
    let mut t = line.trim_start();
    if t.starts_with('[') {
        t = t[t.find("] && ")? + 5..].trim_start();
    }
    let end = t
        .find(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
        .unwrap_or(t.len());
    let rest = &t[end..];
    (end > 0 && (rest.starts_with('=') || rest.starts_with("+=")) && !rest.starts_with("=="))
        .then(|| t[..end].to_string())
}

/// Every environment knob the runner accepts reaches the flow or is refused — never silently dropped.
///
/// A variable is LIVE when a command, a condition or a forward reads it, or when it feeds a variable that is itself
/// live; one only ever assigned into variables nothing reads is dead, and an environment read inside a heredoc
/// does not count, because an unexported shell default never reaches it. MEASURED: eleven were dead at once —
/// GRT_ANT_ITERS, GRT_ANT_MARGIN, GRT_REPAIR, RESIZER_DESIGN, RESIZER_TIMING, QUIT_ON_HOLD, SYNTH_MAX_FANOUT and
/// RT_MIN_SIGNAL_LAYER (into ANTENNA_LEVER_LINES), OL_EXTRA (into EXTRA_LINES) — both leftovers of the removed
/// config heredoc — and FP_CORE_UTIL, whose documented 35 never reached the config merge, so a new block placed
/// at OpenLane's 50. Each built the block with a value its caller had not chosen: the HEUR_DIODE bug this script
/// already records, eleven times over.
#[test]
fn every_runner_knob_reaches_the_flow_or_is_refused() {
    let s = runner();
    let code: Vec<&str> = s
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect();
    let assigned: std::collections::BTreeSet<String> =
        code.iter().filter_map(|l| shell_target(l)).collect();
    let knobs = code
        .iter()
        .filter(|l| {
            let t = l.trim_start();
            shell_target(t).is_some_and(|n| t.starts_with(&format!("{n}=\"${{{n}:-")))
        })
        .count();
    assert!(
        knobs >= 8,
        "parsed too few knobs ({knobs}) — the parser is stale"
    );
    let mut live: std::collections::BTreeSet<String> = code
        .iter()
        .filter(|l| shell_target(l).is_none())
        .flat_map(|l| shell_refs(l))
        .collect();
    loop {
        let before = live.len();
        for l in &code {
            if let Some(t) = shell_target(l) {
                if live.contains(&t) {
                    live.extend(shell_refs(l).into_iter().filter(|r| *r != t));
                }
            }
        }
        if live.len() == before {
            break;
        }
    }
    let dead: Vec<&String> = assigned.iter().filter(|v| !live.contains(*v)).collect();
    assert!(
        dead.is_empty(),
        "openlane-run.sh sets these and nothing reads them, so setting one changes nothing: {dead:?}. \
         Forward each to emit_flow_config (CFG_ARGS+=), or refuse it with a FATAL that says why."
    );
}

/// The knobs do what they say when the runner RUNS: in a sandbox tree (a toy block, a stand-in emitter that
/// records its arguments, no OpenLane and no overlay, so nothing past the config merge can start), each refused
/// knob stops the run before the emitter with its own reason, each forwarded one reaches the emitter, and
/// FP_CORE_UTIL — default or set — reaches the merged config.
#[test]
fn each_knob_is_forwarded_or_refused_when_the_runner_runs() {
    use std::os::unix::fs::PermissionsExt;
    let sb = std::env::temp_dir().join(format!("runner-knobs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&sb);
    for d in [
        "scripts",
        "benchmarks/silicon",
        "tools/OpenLane/designs",
        "target/debug/examples",
    ] {
        std::fs::create_dir_all(sb.join(d)).unwrap();
    }
    std::fs::write(sb.join("scripts/openlane-run.sh"), runner()).unwrap();
    std::fs::write(
        sb.join("benchmarks/silicon/toy.v"),
        "module toy(input clk, output reg q);\n  always @(posedge clk) q <= ~q;\nendmodule\n",
    )
    .unwrap();
    let emit = sb.join("target/debug/examples/emit_flow_config");
    std::fs::write(&emit, format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n[ \"${{FAKE_EMIT_OK:-0}}\" = 1 ] && {{ echo '{{\"DESIGN_NAME\": \"toy\"}}'; exit 0; }}\nexit 1\n",
        sb.join("args.txt").display()
    )).unwrap();
    std::fs::set_permissions(&emit, std::fs::Permissions::from_mode(0o755)).unwrap();
    let run = |env: &[(&str, &str)]| -> (i32, String, Option<String>) {
        let _ = std::fs::remove_file(sb.join("args.txt"));
        let mut c = std::process::Command::new("bash");
        c.arg(sb.join("scripts/openlane-run.sh"))
            .args(["toy", "clk", "10"])
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap())
            .env("HOME", sb.as_os_str())
            .env("ROUTING_CORES", "4");
        for (k, v) in env {
            c.env(k, v);
        }
        let o = c.output().expect("run the sandboxed runner");
        let args = std::fs::read_to_string(sb.join("args.txt")).ok();
        (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stderr).into_owned(),
            args,
        )
    };
    let refused = |env: &[(&str, &str)], why: &str| {
        let (code, err, args) = run(env);
        assert!(
            code == 4 && err.contains(why) && args.is_none(),
            "{env:?}: exit {code}, want 4 naming {why:?} before the emitter:\n{err}"
        );
    };
    let forwarded = |env: &[(&str, &str)], want: &str| {
        let (_, err, args) = run(env);
        let args = args.unwrap_or_else(|| panic!("{env:?}: the emitter never ran:\n{err}"));
        assert!(
            args.contains(want),
            "{env:?}: the emitter got\n{args}\nwithout {want:?}"
        );
        args
    };
    let base = forwarded(&[], "toy\nclk\n10\n--pdk\nsky130A\n--cores\n4\n");
    for absent in [
        "--antenna",
        "--grt-antenna-iters",
        "--grt-antenna-margin",
        "--grt-overflow-iters",
        "--grt-adjustment",
        "--dpl-padding-per-side",
        "--setup-slack-margin",
    ] {
        assert!(
            !base.contains(absent),
            "an unset knob forwards nothing, so the compiler's value governs: {base}"
        );
    }
    assert!(!base.contains("--comparative"), "a block is signed off unless the caller says otherwise: {base}");
    forwarded(&[("COMPARATIVE", "1")], "--comparative\n");
    forwarded(&[("GRT_ANT_ITERS", "30")], "--grt-antenna-iters\n30\n");
    forwarded(&[("GRT_ANT_MARGIN", "40")], "--grt-antenna-margin\n40\n");
    forwarded(&[("GRT_OVERFLOW_ITERS", "150")], "--grt-overflow-iters\n150\n");
    forwarded(&[("GRT_ADJUSTMENT", "0.2")], "--grt-adjustment\n0.2\n");
    forwarded(&[("DPL_PADDING", "2")], "--dpl-padding-per-side\n2\n");
    forwarded(&[("SETUP_SLACK_MARGIN", "0.5")], "--setup-slack-margin\n0.5\n");
    forwarded(&[("GRT_REPAIR", "0")], "--antenna\nheuristic+jog\n");
    forwarded(&[("HEUR_DIODE", "0")], "--antenna\njog-only\n");
    forwarded(
        &[("HEUR_DIODE", "0"), ("GRT_REPAIR", "0")],
        "--antenna\njog-only\n",
    );
    let (_, _, a) = run(&[("GRT_REPAIR", "1")]);
    assert!(
        !a.unwrap().contains("--antenna"),
        "GRT_REPAIR=1 with the heuristic pass on is the default strategy"
    );
    refused(&[("HEUR_DIODE", "0"), ("GRT_REPAIR", "1")], "is no");
    refused(&[("GRT_REPAIR", "2")], "are 0 or 1");
    refused(&[("COMPARATIVE", "yes")], "COMPARATIVE is 1");
    refused(
        &[("ANTENNA", "jog-only"), ("GRT_REPAIR", "0")],
        "each choose the antenna strategy",
    );
    refused(
        &[("SYNTH_MAX_FANOUT", "8")],
        "SYNTH_MAX_FANOUT would move the fanout limit",
    );
    refused(
        &[("RT_MIN_SIGNAL_LAYER", "met2")],
        "RT_MIN_SIGNAL_LAYER is not a recipe",
    );
    refused(
        &[("OL_EXTRA", "    \"X\": 1,")],
        "OL_EXTRA (raw config lines) is gone",
    );
    for k in ["RESIZER_DESIGN", "RESIZER_TIMING", "QUIT_ON_HOLD"] {
        refused(&[(k, "0")], "no recipe turns them off");
    }
    for (env, util) in [(vec![], 35), (vec![("FP_CORE_UTIL", "28")], 28)] {
        let mut e = env.clone();
        e.extend([("FAKE_EMIT_OK", "1"), ("OL_MIN_FREE_GB", "0")]);
        run(&e);
        let cfg = std::fs::read_to_string(sb.join("tools/OpenLane/designs/logos_toy/config.json"))
            .expect("the merged config");
        let d: serde_json::Value = serde_json::from_str(&cfg).unwrap();
        assert_eq!(
            d["FP_CORE_UTIL"], util,
            "{env:?}: FP_CORE_UTIL reaches the config:\n{cfg}"
        );
    }
    std::fs::remove_dir_all(&sb).ok();
}

/// The prebuilt emitter is the one cargo builds: under `CARGO_TARGET_DIR` when it is set (a copy of the tree building
/// into a target of its own), under the checkout's `target/` otherwise. A runner that looked in `target/` alone would
/// call an emitter cargo never wrote, and refuse every block as built from stale knowledge.
#[test]
fn the_prebuilt_emitter_is_where_cargo_builds_it() {
    use std::os::unix::fs::PermissionsExt;
    let sb = std::env::temp_dir().join(format!("runner-target-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&sb);
    for d in ["scripts", "benchmarks/silicon", "tools/OpenLane/designs", "elsewhere/debug/examples"] {
        std::fs::create_dir_all(sb.join(d)).unwrap();
    }
    std::fs::write(sb.join("scripts/openlane-run.sh"), runner()).unwrap();
    std::fs::write(
        sb.join("benchmarks/silicon/toy.v"),
        "module toy(input clk, output reg q);\n  always @(posedge clk) q <= ~q;\nendmodule\n",
    )
    .unwrap();
    let emit = sb.join("elsewhere/debug/examples/emit_flow_config");
    std::fs::write(&emit, format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nexit 1\n", sb.join("args.txt").display())).unwrap();
    std::fs::set_permissions(&emit, std::fs::Permissions::from_mode(0o755)).unwrap();
    let o = std::process::Command::new("bash")
        .arg(sb.join("scripts/openlane-run.sh"))
        .args(["toy", "clk", "10"])
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .env("HOME", sb.as_os_str())
        .env("ROUTING_CORES", "4")
        .env("CARGO_TARGET_DIR", sb.join("elsewhere"))
        .output()
        .expect("run the sandboxed runner");
    let args = std::fs::read_to_string(sb.join("args.txt"));
    std::fs::remove_dir_all(&sb).ok();
    assert!(
        args.is_ok_and(|a| a.starts_with("toy\nclk\n10\n")),
        "the emitter under CARGO_TARGET_DIR ran:\n{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

/// A bench run and a control name the image they flowed in (`openlane_image` beside the run: the name
/// `build-openroad.sh --name` prints, and a newline) before the replay or the recorder reads the run; a run in OpenLane
/// as released names none. A bench run and a control record no verdict whose IR gate a source node OpenROAD chose on
/// its own decided: when the anchored replay fails, the run stops before the recorder (exit 5), and `ANCHOR_IR=0` is
/// refused for either before the emitter. Any other run still records, with its unanchored IR and a line saying so,
/// and `ANCHOR_IR=0` skips the replay. ANCHOR_IR is 1 or 0.
#[test]
fn a_bench_run_or_a_control_names_its_image_and_records_only_an_anchored_ir() {
    use std::os::unix::fs::PermissionsExt;
    let sb = std::env::temp_dir().join(format!("runner-anchor-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&sb);
    for d in [
        "scripts/openlane-overlay",
        "scripts/openroad",
        "benchmarks/silicon",
        "tools/OpenLane/designs",
        "tools/qor/openlane-bench/designs",
        "tools/qor/openlane-stock/designs",
        "target/debug/examples",
        "bin",
    ] {
        std::fs::create_dir_all(sb.join(d)).unwrap();
    }
    let exe = |rel: &str, text: &str| {
        std::fs::write(sb.join(rel), text).unwrap();
        std::fs::set_permissions(sb.join(rel), std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    std::fs::write(sb.join("scripts/openlane-run.sh"), runner()).unwrap();
    std::fs::write(sb.join("scripts/openlane-overlay/BASE"), "0123456789abcdef0123456789abcdef01234567\n").unwrap();
    for tree in ["tools/qor/openlane-bench", "tools/qor/openlane-stock"] {
        std::fs::write(sb.join(tree).join(".logos-base"), "0123456789abcdef0123456789abcdef01234567\n").unwrap();
    }
    exe("scripts/openlane-overlay/apply.sh", "#!/bin/sh\nexit 0\n");
    exe("scripts/openroad/build-openroad.sh", "#!/bin/sh\necho logos-openlane:probe\n");
    exe(
        "scripts/anchor-ir-drop.sh",
        &format!(
            "#!/bin/sh\ncat \"$1/openlane_image\" > '{}' 2>/dev/null || echo none > '{}'\necho '== anchoring IR for toy =='\n\
             [ \"$FAKE_ANCHOR_OK\" = 1 ] && {{ echo '   ANCHORED: 1 instances, worst IR drop 1.00 mV'; exit 0; }}\n\
             echo '   FATAL: the replay wrote no report' >&2\nexit 3\n",
            sb.join("anchored.txt").display(),
            sb.join("anchored.txt").display()
        ),
    );
    exe(
        "scripts/record-sky130-signoff.py",
        &format!(
            "#!/usr/bin/env python3\nimport os, sys\nm = os.path.join(sys.argv[1], 'openlane_image')\n\
             open('{}', 'w').write(open(m).read() if os.path.exists(m) else 'none\\n')\n",
            sb.join("recorded.txt").display()
        ),
    );
    exe(
        "target/debug/examples/emit_flow_config",
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\necho '{{\"DESIGN_NAME\": \"toy\"}}'\n",
            sb.join("args.txt").display()
        ),
    );
    exe(
        "bin/sg",
        r#"#!/usr/bin/env python3
import os, re, sys
cmd = sys.argv[sys.argv.index("-c") + 1]
m = re.match(r"make quick_run QUICK_RUN_DESIGN=(\S+)", cmd)
if m:
    run = os.path.join("designs", m.group(1), "runs", "RUN_1")
    os.makedirs(run)
    open(os.path.join(run, "config.tcl"), "w").write('set ::env(CLOCK_PERIOD) "10"\n')
    sys.exit(0)
sys.exit(0 if "--built" in cmd else 1)
"#,
    );
    std::fs::write(
        sb.join("benchmarks/silicon/toy.v"),
        "module toy(input clk, output reg q);\n  always @(posedge clk) q <= ~q;\nendmodule\n",
    )
    .unwrap();
    let marker = |f: &str| std::fs::read_to_string(sb.join(f)).ok();
    let run = |env: &[(&str, &str)]| -> (i32, String, bool, bool, bool) {
        for f in ["args.txt", "anchored.txt", "recorded.txt"] {
            let _ = std::fs::remove_file(sb.join(f));
        }
        for tree in ["tools/OpenLane", "tools/qor/openlane-bench", "tools/qor/openlane-stock"] {
            let _ = std::fs::remove_dir_all(sb.join(tree).join("designs/logos_toy"));
        }
        let mut c = std::process::Command::new("bash");
        c.arg(sb.join("scripts/openlane-run.sh"))
            .args(["toy", "clk", "10"])
            .env_clear()
            .env("PATH", format!("{}:{}", sb.join("bin").display(), std::env::var("PATH").unwrap()))
            .env("HOME", sb.as_os_str())
            .env("ROUTING_CORES", "4")
            .env("OL_MIN_FREE_GB", "0")
            .env("OUT_JSON", sb.join("ledger.json"));
        for (k, v) in env {
            c.env(k, v);
        }
        let o = c.output().expect("run the sandboxed runner");
        (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stderr).into_owned(),
            sb.join("args.txt").exists(),
            sb.join("anchored.txt").exists(),
            sb.join("recorded.txt").exists(),
        )
    };
    for (tree, dir) in [(("OL_BENCH", "1"), "tools/qor/openlane-bench"), (("OL_STOCK", "1"), "tools/qor/openlane-stock")] {
        let (code, err, emitted, anchored, recorded) = run(&[tree]);
        assert!(
            code == 5 && emitted && anchored && !recorded && err.contains("IR anchoring failed"),
            "{tree:?}: a failed replay stops the run before the recorder (exit {code}, anchored {anchored}, recorded {recorded}):\n{err}"
        );
        let (code, _, _, anchored, recorded) = run(&[tree, ("FAKE_ANCHOR_OK", "1")]);
        assert!(code == 0 && anchored && recorded, "{tree:?}: an anchored run records (exit {code})");
        let beside = std::fs::read_to_string(sb.join(dir).join("designs/logos_toy/runs/RUN_1/openlane_image")).ok();
        assert_eq!(beside.as_deref(), Some("logos-openlane:probe\n"), "{tree:?}: the run names the image it flowed in");
        assert_eq!(
            (marker("anchored.txt").as_deref(), marker("recorded.txt").as_deref()),
            (Some("logos-openlane:probe\n"), Some("logos-openlane:probe\n")),
            "{tree:?}: the replay and the recorder find the run's image named"
        );
        let (code, err, emitted, anchored, recorded) = run(&[tree, ("ANCHOR_IR", "0")]);
        assert!(
            code == 1 && !emitted && !anchored && !recorded && err.contains("ANCHOR_IR=0"),
            "{tree:?}: ANCHOR_IR=0 is refused before the emitter (exit {code}):\n{err}"
        );
    }
    let (code, err, _, anchored, recorded) = run(&[]);
    assert!(
        code == 0 && anchored && recorded && err.contains("the recorded ir_drop_worst_mv is UNANCHORED"),
        "a run in tools/OpenLane records with its unanchored IR, and says so (exit {code}):\n{err}"
    );
    assert!(
        !sb.join("tools/OpenLane/designs/logos_toy/runs/RUN_1/openlane_image").exists()
            && marker("anchored.txt").as_deref() == Some("none\n")
            && marker("recorded.txt").as_deref() == Some("none\n"),
        "a run in OpenLane as released names no image"
    );
    let (code, _, _, anchored, recorded) = run(&[("ANCHOR_IR", "0")]);
    assert!(code == 0 && !anchored && recorded, "ANCHOR_IR=0 skips the replay of a run in tools/OpenLane (exit {code})");
    let (code, err, emitted, _, recorded) = run(&[("ANCHOR_IR", "yes")]);
    assert!(
        code == 1 && !emitted && !recorded && err.contains("ANCHOR_IR is 1"),
        "ANCHOR_IR is 1 or 0 (exit {code}):\n{err}"
    );
    std::fs::remove_dir_all(&sb).ok();
}
