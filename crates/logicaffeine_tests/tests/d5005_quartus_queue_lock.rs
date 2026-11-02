//! `scripts/d5005/quartus.sh`, the box's one Quartus queue, held to its contract through its dry run and its functions
//! (no Docker, no Quartus): the project a job writes, the container it would run in, the refusals, the order in which
//! a real run takes the queue, runs and records, which failures it runs again and from where, and the evidence its
//! row is judged on.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn script() -> PathBuf {
    root().join("scripts/d5005/quartus.sh")
}

fn dry(jobs: &Path, processors: Option<&str>, args: &[&str]) -> Output {
    let mut c = Command::new("bash");
    c.arg(script()).args(args).env("QUARTUS_DRY_RUN", "1").env("QUARTUS_JOBS", jobs).env("Q234", "/opt/q234-test");
    c.env("QUARTUS_LOGS", jobs.with_extension("logs"));
    match processors {
        Some(p) => c.env("QUARTUS_PROCESSORS", p),
        None => c.env_remove("QUARTUS_PROCESSORS"),
    };
    c.output().expect("bash runs the queue script")
}

fn design(dir: &Path) -> PathBuf {
    let v = dir.join("unit.v");
    std::fs::write(&v, "module unit #(parameter W = 1) (input clk, input [W-1:0] a, output reg [W-1:0] y);\n always @(posedge clk) y <= a;\nendmodule\n").unwrap();
    v
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// Whether the printed command carries `part`, as bash's `printf %q` quotes it: as it is inside a multi-line `$'...'`
/// word, backslash-escaped in a one-line word.
fn carries(cmd: &str, part: &str) -> bool {
    let quoted = Command::new("bash").args(["-c", "printf %q \"$1\"", "-", part]).output().expect("bash quotes").stdout;
    cmd.contains(part) || cmd.contains(String::from_utf8_lossy(&quoted).as_ref())
}

#[test]
fn a_synthesis_job_writes_its_project_and_names_its_container() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = tmp.path().join("jobs");
    let v = design(tmp.path());
    let o = dry(&jobs, Some("16"), &["syn", "unit-w8", v.to_str().unwrap(), "unit", "W=8"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let qsf = std::fs::read_to_string(jobs.join("unit-w8/unit-w8.qsf")).unwrap();
    for line in [
        "set_global_assignment -name FAMILY \"Stratix 10\"",
        "set_global_assignment -name DEVICE 1SX280HN2F43E2VG",
        "set_global_assignment -name TOP_LEVEL_ENTITY unit",
        "set_global_assignment -name VERILOG_FILE design.v",
        "set_global_assignment -name NUM_PARALLEL_PROCESSORS 16",
        "set_instance_assignment -name VIRTUAL_PIN ON -to *",
        "set_parameter -name W 8",
    ] {
        assert!(qsf.lines().any(|l| l == line), "{line} in\n{qsf}");
    }
    assert!(!qsf.contains("SDC_FILE") && !jobs.join("unit-w8/design.sdc").exists(), "synthesis alone has no clock");
    assert_eq!(std::fs::read_to_string(jobs.join("unit-w8/design.v")).unwrap(), std::fs::read_to_string(&v).unwrap());
    assert_eq!(std::fs::read_to_string(jobs.join("unit-w8/unit-w8.qpf")).unwrap(), "PROJECT_REVISION = \"unit-w8\"\n");
    let cmd = stdout(&o);
    for part in ["--name logos-q-unit-w8", "logos-quartus-ofs:23.4-s10", "/opt/q234-test:/opt/q234:ro", "quartus_syn unit-w8"] {
        assert!(carries(&cmd, part), "{part} in {cmd}");
    }
    assert!(!cmd.contains("quartus_fit"));
}

#[test]
fn a_fit_job_closes_one_clock_at_its_period_on_every_core_by_default() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = tmp.path().join("jobs");
    let v = design(tmp.path());
    let o = dry(&jobs, None, &["fit", "unit-300", v.to_str().unwrap(), "unit", "3.333"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let qsf = std::fs::read_to_string(jobs.join("unit-300/unit-300.qsf")).unwrap();
    assert!(qsf.contains("set_instance_assignment -name VIRTUAL_PIN OFF -to clk"));
    assert!(qsf.contains("set_global_assignment -name SDC_FILE design.sdc"));
    let cores = String::from_utf8(Command::new("nproc").output().unwrap().stdout).unwrap();
    assert!(qsf.contains(&format!("NUM_PARALLEL_PROCESSORS {}", cores.trim())), "every core: {qsf}");
    assert_eq!(
        std::fs::read_to_string(jobs.join("unit-300/design.sdc")).unwrap(),
        "create_clock -name clk -period 3.333 [get_ports clk]\n"
    );
    let cmd = stdout(&o);
    assert!(carries(&cmd, "run quartus_syn unit-300 && run quartus_fit unit-300 && run quartus_sta unit-300"), "each module in order: {cmd}");
    assert!(carries(&cmd, "grep -qx \"$1\" modules.done"), "a module finished is not run again: {cmd}");
}

#[test]
fn a_job_it_cannot_run_as_asked_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = tmp.path().join("jobs");
    let v = design(tmp.path());
    let v = v.to_str().unwrap();
    for args in [
        vec!["syn", "bad name", v, "unit"],
        vec!["syn", "ok", "/nonexistent.v", "unit"],
        vec!["syn", "ok2", v, "unit", "W"],
        vec!["fit", "ok3", v, "unit", "fast"],
        vec!["fit", "ok4", v, "unit"],
        vec!["frobnicate"],
    ] {
        let o = dry(&jobs, None, &args);
        assert_eq!(o.status.code(), Some(2), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
    }
    assert!(dry(&jobs, None, &["syn", "once", v, "unit"]).status.success());
    let again = dry(&jobs, None, &["syn", "once", v, "unit"]);
    assert_eq!(again.status.code(), Some(2), "a job name is used once");
    assert!(String::from_utf8_lossy(&again.stderr).contains("exists"));
}

#[test]
fn a_persona_build_runs_the_fims_template_and_states_processors_only_when_asked() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("tree");
    std::fs::create_dir_all(tree.join("afu/hw/rtl")).unwrap();
    std::fs::write(tree.join("afu/hw/rtl/sources.txt"), "logos.v\n").unwrap();
    let o = dry(&tmp.path().join("jobs"), None, &["pr", "engine3-pr", tree.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let cmd = stdout(&o);
    for part in ["--name logos-q-engine3-pr", "afu_synth_setup", "/d5005/pr-template", "afu_synth"] {
        assert!(cmd.contains(part), "{part} in {cmd}");
    }
    assert!(!cmd.contains("NUM_PARALLEL_PROCESSORS"), "the template decides unless told");
    let o = dry(&tmp.path().join("jobs"), Some("32"), &["pr", "engine4-pr", tree.to_str().unwrap()]);
    assert!(carries(&stdout(&o), "set_global_assignment -name NUM_PARALLEL_PROCESSORS 32"), "{}", stdout(&o));
    let missing = dry(&tmp.path().join("jobs"), None, &["pr", "x", tmp.path().to_str().unwrap()]);
    assert_eq!(missing.status.code(), Some(2), "no AFU tree");
}

/// A fit job given a floorplan (`QUARTUS_FLOORPLAN`) carries it into its project: the Tcl copied into the job as
/// `floorplan.tcl`, the project's settings sourcing it, and the job's row naming it among its inputs. A fit given none
/// has neither; a synthesis, which places nothing, refuses one; and a floorplan that is no file is refused before the
/// job is written.
#[test]
fn a_fit_job_given_a_floorplan_sources_it_and_a_synthesis_refuses_one() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = tmp.path().join("jobs");
    let v = design(tmp.path());
    let plan = tmp.path().join("tiles.tcl");
    let tcl = "set_instance_assignment -name PLACE_REGION \"X0 Y0 X15 Y23\" -to tile_0_0\nset_instance_assignment -name CORE_ONLY_PLACE_REGION ON -to tile_0_0\n";
    std::fs::write(&plan, tcl).unwrap();
    let run = |args: &[&str], floorplan: Option<&Path>| {
        let mut c = Command::new("bash");
        c.arg(script()).args(args).env("QUARTUS_DRY_RUN", "1").env("QUARTUS_JOBS", &jobs).env("Q234", "/opt/q234-test");
        c.env("QUARTUS_LOGS", jobs.with_extension("logs")).env_remove("QUARTUS_PROCESSORS");
        match floorplan {
            Some(f) => c.env("QUARTUS_FLOORPLAN", f),
            None => c.env_remove("QUARTUS_FLOORPLAN"),
        };
        c.output().expect("bash runs the queue script")
    };
    let source = "set_global_assignment -name SOURCE_TCL_SCRIPT_FILE floorplan.tcl";
    let o = run(&["fit", "unit-tiles", v.to_str().unwrap(), "unit", "5"], Some(&plan));
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let qsf = std::fs::read_to_string(jobs.join("unit-tiles/unit-tiles.qsf")).unwrap();
    assert_eq!(qsf.lines().filter(|l| *l == source).count(), 1, "the floorplan sourced once:\n{qsf}");
    assert_eq!(std::fs::read_to_string(jobs.join("unit-tiles/floorplan.tcl")).unwrap(), tcl);
    let o = run(&["fit", "unit-free", v.to_str().unwrap(), "unit", "5"], None);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(!std::fs::read_to_string(jobs.join("unit-free/unit-free.qsf")).unwrap().contains("SOURCE_TCL_SCRIPT_FILE"));
    assert!(!jobs.join("unit-free/floorplan.tcl").exists());
    let o = run(&["syn", "unit-syn", v.to_str().unwrap(), "unit"], Some(&plan));
    assert!(!o.status.success() && String::from_utf8_lossy(&o.stderr).contains("floorplan"), "a synthesis takes no floorplan: {}", String::from_utf8_lossy(&o.stderr));
    assert!(!jobs.join("unit-syn").exists(), "nothing written for a refused job");
    let o = run(&["fit", "unit-none", v.to_str().unwrap(), "unit", "5"], Some(&tmp.path().join("absent.tcl")));
    assert!(!o.status.success() && String::from_utf8_lossy(&o.stderr).contains("absent.tcl"), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(!jobs.join("unit-none").exists(), "nothing written for a refused job");
    let text = std::fs::read_to_string(script()).unwrap();
    let body = &text[text.find("  syn|fit)").unwrap()..text.find("  pr|resume)").unwrap()];
    assert!(body.contains("[ -f \"$JOBS/$job/floorplan.tcl\" ] && inputs+=(\"$JOBS/$job/floorplan.tcl\")"), "the row names its floorplan");
}

#[test]
fn a_real_run_takes_the_queue_then_attempts_then_records_and_never_drops_the_exit_status() {
    let text = std::fs::read_to_string(script()).unwrap();
    for branch in ["  syn|fit)", "  pr|resume)"] {
        let at = text.find(branch).unwrap_or_else(|| panic!("{branch}"));
        let body = &text[at..];
        let order = ["take_queue \"$job\"", "since=$(date +%s)", "attempts \"$job\"", "record \""];
        let mut from = 0;
        for step in order {
            let found = body[from..].find(step).unwrap_or_else(|| panic!("{branch}: `{step}` after byte {from}"));
            from += found + step.len();
        }
    }
    let queue = &text[text.find("take_queue() {").unwrap()..];
    let queue = &queue[..queue.find("\n}\n").unwrap()];
    assert!(queue.contains("flock 9"), "one lock for the box");
    assert!(queue.contains("docker ps -q --filter \"ancestor=$IMAGE\""), "and no Quartus container from outside the queue");
    assert!(text.contains("quartus_record"), "every run becomes a ledger row");
    assert!(!text.contains("|| true"), "no exit status is thrown away");
}

#[test]
fn a_synthesized_persona_resumes_from_its_fitter_and_an_unsynthesized_one_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("tree");
    std::fs::create_dir_all(tree.join("afu/hw/rtl")).unwrap();
    std::fs::write(tree.join("afu/hw/rtl/sources.txt"), "logos.v\n").unwrap();
    let refused = dry(&tmp.path().join("jobs"), None, &["resume", "hybrid3-fit", tree.to_str().unwrap()]);
    assert_eq!(refused.status.code(), Some(2), "nothing synthesized to resume");
    assert!(String::from_utf8_lossy(&refused.stderr).contains("no synthesized persona"));
    std::fs::create_dir_all(tree.join("build/build/syn/syn_top/qdb")).unwrap();
    let o = dry(&tmp.path().join("jobs"), None, &["resume", "hybrid3-fit", tree.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let cmd = stdout(&o);
    let mut from = 0;
    for module in ["quartus_fit --read_settings_files=on", "quartus_sta d5005 -c iofs_pr_afu --mode=finalize", "quartus_asm", "gen_gbs.tcl dni_compile"] {
        let found = cmd[from..].find(module).unwrap_or_else(|| panic!("`{module}` in order in {cmd}"));
        from += found + module.len();
    }
    assert!(carries(&cmd, "--name logos-q-hybrid3-fit"), "{cmd}");
    assert!(!cmd.contains("afu_synth"), "synthesis is not run again");
}

/// A synthesized flat image resumes as a persona does (`resume <job> <tree>`): its FIM's work tree as its synthesis left
/// it, the design held to the device's budget, then its fit, timing analysis and assembly into the SOF, no setup, IP
/// generation or synthesis run again; a fit that failed on its settings (a floorplan region too small) is fitted again
/// from them. A flat tree with nothing synthesized is refused, naming what it lacks.
#[test]
fn a_synthesized_flat_image_resumes_from_its_budget_and_an_unsynthesized_one_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = flat_tree(tmp.path(), WHOLE_DEVICE);
    let refused = dry(&tmp.path().join("jobs"), None, &["resume", "hybrid7b-fit", tree.to_str().unwrap()]);
    assert_eq!(refused.status.code(), Some(2), "nothing synthesized to resume");
    assert!(String::from_utf8_lossy(&refused.stderr).contains("no synthesized flat image under"), "{}", String::from_utf8_lossy(&refused.stderr));
    std::fs::create_dir_all(tree.join("fim/syn/syn_top/qdb")).unwrap();
    let o = dry(&tmp.path().join("jobs"), None, &["resume", "hybrid7b-fit", tree.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let cmd = stdout(&o);
    assert!(carries(&cmd, "--name logos-q-hybrid7b-fit"), "{cmd}");
    assert!(carries(&cmd, &format!("{}:/work", tree.display())), "the tree is /work: {cmd}");
    assert!(
        in_order(
            &cmd,
            &[
                "cd /work/fim/syn/syn_top",
                "Estimate of Logic utilization (ALMs needed)",
                "run quartus_fit --read_settings_files=on --write_settings_files=off d5005 -c d5005",
                "run quartus_sta d5005 -c d5005 --mode=finalize",
                "run quartus_asm --read_settings_files=on --write_settings_files=off d5005 -c d5005",
                "/work/fim/syn/syn_top/output_files/d5005.sof",
            ]
        ),
        "the budget, then the fit, timing analysis and assembly: {cmd}"
    );
    for absent in ["--stage=setup", "quartus_ipgenerate", "build_top.sh", "rm -rf", "afu_synth", "iofs_pr_afu"] {
        assert!(!cmd.contains(absent), "{absent} in {cmd}");
    }
}

/// A flat fit whose retiming finished since its placement plan, and the plan since its synthesis, goes on from its
/// retimed snapshot when it is resumed: the fitter's finalize stage alone (`quartus_fit --finalize`), the fit then
/// marked run so its timing analysis and assembly follow. A finalize the fitter refuses (a retimed snapshot it cannot
/// load: hybrid9h-fit1's, killed in its finalize stage, "Cannot load retimed database") leaves the fit unmarked, so the
/// fitter runs whole after it, and leaves the job's log no line the recorder judges a failure: hybrid9h-fit3's whole fit,
/// timing analysis and assembly passed and its row read FAILED from the refusal's "Fitter was unsuccessful" before them.
/// The refusal's reason is in the log and the attempt's own output in finalize.log beside the job; an accepted
/// finalize's output is the fit's, in the log. A fit stopped before its retiming, one whose retiming is an earlier placement's, one
/// whose placement is an earlier synthesis's, and one already marked run go on as before: the fitter whole, or not at all.
#[test]
fn a_flat_fit_stopped_after_its_retiming_resumes_at_its_finalize_stage() {
    let text = std::fs::read_to_string(script()).unwrap();
    let at = text.find("FLAT_RESUME_SCRIPT=").expect("the flat resume");
    let resume = &text[at..at + text[at..].find("d5005.sof'").expect("its end")];
    assert!(
        in_order(resume, &["$FLAT_BUDGET", "$FLAT_FIT_REST", "run quartus_fit --read_settings_files=on", "run quartus_sta"]),
        "the budget, the rest of a retimed fit, then the fit, timing analysis and assembly: {resume}"
    );
    let tmp = tempfile::tempdir().unwrap();
    let w = tmp.path();
    let out = w.join("syn_top/output_files");
    std::fs::create_dir_all(&out).unwrap();
    let env = [("W", w)];
    let rest = |files: &[&str], marked: bool, refused: bool| -> String {
        let done = w.join("resume.done");
        let _ = std::fs::remove_file(&done);
        if marked {
            std::fs::write(&done, "quartus_fit\n").unwrap();
        }
        for f in ["d5005.syn.rpt", "d5005.fit.plan.rpt", "d5005.fit.retime.rpt"] {
            let _ = std::fs::remove_file(out.join(f));
        }
        for (k, f) in files.iter().enumerate() {
            let p = out.join(f);
            std::fs::write(&p, "report\n").unwrap();
            let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000 + 60 * k as u64);
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
        }
        let _ = std::fs::remove_file(w.join("finalize.log"));
        let fit = if refused {
            "quartus_fit() { echo \"fit $*\"; echo \"Error (16546): Cannot load retimed database - ensure all earlier stages of the compiler have completed.\"; \
             echo \"Error: Quartus Prime Fitter was unsuccessful. 1 error, 1 warning\"; return 3; }"
        } else {
            "quartus_fit() { echo \"fit $*\"; echo \"Info: Quartus Prime Fitter was successful. 0 errors, 1 warning\"; }"
        };
        with_functions(&env, &format!("{fit}; cd \"$W/syn_top\"; eval \"${{FLAT_FIT_REST//\\/work/$W}}\"; echo \"done: $(cat \"$W/resume.done\" 2>/dev/null | tr '\\n' ' ')\""))
    };
    let finalize = "fit --read_settings_files=on --write_settings_files=off d5005 -c d5005 --finalize";
    let got = rest(&["d5005.syn.rpt", "d5005.fit.plan.rpt", "d5005.fit.retime.rpt"], false, false);
    assert!(got.lines().any(|l| l == finalize) && got.contains("done: quartus_fit"), "retimed since its plan, planned since its synthesis: {got}");
    assert!(got.contains("Quartus Prime Fitter was successful"), "an accepted finalize's output is the fit's, in the log: {got}");
    let got = rest(&["d5005.syn.rpt", "d5005.fit.plan.rpt", "d5005.fit.retime.rpt"], false, true);
    assert!(got.contains("done: \n"), "a finalize the fitter refuses leaves the fit to run whole: {got}");
    assert_eq!(logicaffeine_compile::codegen_sva::quartus::log_failure(&got), None, "a refused finalize leaves the log no failure: {got}");
    assert!(got.contains("Cannot load retimed database"), "the log names the fitter's reason: {got}");
    let attempt = std::fs::read_to_string(w.join("finalize.log")).unwrap_or_default();
    assert!(attempt.lines().any(|l| l == finalize) && attempt.contains("Fitter was unsuccessful"), "the attempt's own output beside the job: {attempt}");
    for (what, files, marked) in [
        ("stopped before its retiming", &["d5005.syn.rpt", "d5005.fit.plan.rpt"][..], false),
        ("its retiming an earlier placement's", &["d5005.syn.rpt", "d5005.fit.retime.rpt", "d5005.fit.plan.rpt"][..], false),
        ("its placement an earlier synthesis's", &["d5005.fit.plan.rpt", "d5005.fit.retime.rpt", "d5005.syn.rpt"][..], false),
        ("nothing but its synthesis", &["d5005.syn.rpt"][..], false),
        ("its fit already marked run", &["d5005.syn.rpt", "d5005.fit.plan.rpt", "d5005.fit.retime.rpt"][..], true),
    ] {
        let got = rest(files, marked, false);
        assert!(!got.lines().any(|l| l.starts_with("fit ")), "{what}: no stage of the fitter run here: {got}");
        let want = if marked { "done: quartus_fit" } else { "done: \n" };
        assert!(got.contains(want), "{what}: the fit marked only as it was: {got}");
    }
}

#[test]
fn a_job_whose_log_exists_is_refused_before_it_queues() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = tmp.path().join("jobs");
    let logs = tmp.path().join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(logs.join("taken.log"), "an earlier run\n").unwrap();
    let v = design(tmp.path());
    let o = Command::new("bash")
        .arg(script())
        .args(["syn", "taken", v.to_str().unwrap(), "unit"])
        .env("QUARTUS_JOBS", &jobs)
        .env("QUARTUS_LOGS", &logs)
        .env("QUARTUS_LOCK", tmp.path().join("lock"))
        .env_remove("QUARTUS_DRY_RUN")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stderr).contains("has a log"));
    assert!(!jobs.join("taken").exists(), "refused before any project is written");
    assert!(!tmp.path().join("lock").exists(), "and before the queue is taken");
}

/// Run `body` in bash with the queue script's functions defined and nothing run; its stdout.
fn with_functions(env: &[(&str, &Path)], body: &str) -> String {
    let mut c = Command::new("bash");
    c.arg("-c").arg(format!("set -euo pipefail; QUARTUS_LIB=1 source {}; {body}", script().display()));
    for (k, v) in env {
        c.env(k, v);
    }
    let o = c.output().unwrap();
    assert!(o.status.success(), "{body}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn the_ddm_start_up_check_is_told_from_any_other_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("project");
    std::fs::create_dir_all(project.join("build/syn")).unwrap();
    let log = tmp.path().join("job.log");
    let fatal = "Fatal Error: Assertion failed tools/cpp/ddm/ddm_assessor.cpp:53: DDM_T::verify_token(token) : Cannot identify the client\n";
    std::fs::write(&log, format!("attempt 1\n{fatal}attempt 2\nError: Quartus Prime Fitter was unsuccessful. 2 errors\n")).unwrap();
    let second = std::fs::read_to_string(&log).unwrap().find("attempt 2").unwrap();
    let env = [("LOG", log.as_path()), ("PROJECT", project.as_path())];
    let ask = |mark: usize, began: u64| with_functions(&env, &format!("if transient \"$LOG\" {mark} \"$PROJECT\" {began}; then echo yes; else echo no; fi"));
    assert_eq!(ask(0, 0).trim(), "yes", "the module's own message");
    assert_eq!(ask(second, 0).trim(), "no", "an earlier attempt's message is not this attempt's");
    let crash = project.join("build/syn/serv_req_info.txt");
    std::fs::write(&crash, "<callstack>0x7f: dni_dms + 0xd07fa (_ZN11DMS_MANAGERC1Ev + 0x456)</callstack>").unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    assert_eq!(ask(second, now - 60).trim(), "yes", "a crash report from the DMS manager's start-up since the attempt began");
    assert_eq!(ask(second, now + 60).trim(), "no", "one written before it began is an earlier run's");
    std::fs::write(&crash, "<callstack>0x7f: quartus_fit + 0x10 (_Z5placev)</callstack>").unwrap();
    assert_eq!(ask(second, now - 60).trim(), "no", "any other crash is a real failure");
}

/// A step the stall watch stopped standing still — a synthesis hung on a helper the DDM start-up check killed — is the
/// check's, and its attempt goes again; a step the watch stopped at its bound is a real failure.
#[test]
fn a_step_the_stall_watch_stopped_standing_still_is_tried_again_and_one_past_its_bound_is_not() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let log = tmp.path().join("job.log");
    let env = [("LOG", log.as_path()), ("PROJECT", project.as_path())];
    let ask = || with_functions(&env, "if transient \"$LOG\" 0 \"$PROJECT\" 0; then echo yes; else echo no; fi");
    let still = "stall_watch.sh: ./ofs-common/scripts/common/syn/build_top.sh took 6.19 s of CPU in 300 s, under 30; its processes stopped\n";
    std::fs::write(&log, format!("Info: Running Quartus Prime Synthesis\n{still}")).unwrap();
    assert_eq!(ask().trim(), "yes", "a synthesis standing still");
    let bound = "stall_watch.sh: ./ofs-common/scripts/common/syn/build_top.sh still ran 14400 s after it started; its processes stopped\n";
    std::fs::write(&log, format!("Info: Running Quartus Prime Synthesis\n{bound}")).unwrap();
    assert_eq!(ask().trim(), "no", "a synthesis past its bound");
}

#[test]
fn a_persona_that_finished_synthesis_goes_on_from_its_fitter() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("job.log");
    std::fs::write(&log, "Info: Quartus Prime Synthesis was successful. 0 errors, 276 warnings\nInfo: Run task: Fitter\n").unwrap();
    std::fs::write(tmp.path().join("resume.done"), "quartus_fit\n").unwrap();
    let env = [("LOG_FILE", log.as_path()), ("TREE", tmp.path())];
    let after = |mark: usize| {
        with_functions(
            &env,
            &format!("script=\"$PR_SCRIPT\"; expect=\"$PR_MODULES\"; log=\"$LOG_FILE\"; tree=\"$TREE\"; mark={mark}; pr_between; [ \"$script\" = \"$RESUME_SCRIPT\" ] && echo resumed || echo again; echo \"$expect\""),
        )
    };
    assert_eq!(after(0), "resumed\nFitter,Timing Analyzer,Assembler\n");
    assert!(!tmp.path().join("resume.done").exists(), "a resume starts at the fitter");
    let past = std::fs::read_to_string(&log).unwrap().len();
    assert_eq!(after(past), "again\nSynthesis,Fitter,Timing Analyzer,Assembler\n", "an attempt that did not finish synthesis runs it again");
}

/// A persona whose fit succeeded after its synthesis goes on from its timing analysis, the fit not run again: after an
/// attempt that died past its fit, as by `quartus.sh resume`, its expected modules the timing analysis and the
/// assembler. A fit that failed, or one older than the synthesis (a tree synthesized again since), is run again.
#[test]
fn a_persona_fitted_since_its_synthesis_goes_on_from_its_timing_analysis() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("build/build/syn/syn_top/output_files");
    std::fs::create_dir_all(&out).unwrap();
    let log = tmp.path().join("job.log");
    std::fs::write(&log, "Info: Quartus Prime Synthesis was successful. 0 errors, 276 warnings\nInfo: Quartus Prime Fitter was successful.\n").unwrap();
    let syn = out.join("iofs_pr_afu.syn.summary");
    let fit = out.join("iofs_pr_afu.fit.summary");
    let set = |p: &Path, text: &str, age: u64| {
        std::fs::write(p, text).unwrap();
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(age);
        std::fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
    };
    let env = [("LOG_FILE", log.as_path()), ("TREE", tmp.path())];
    let between = || {
        with_functions(
            &env,
            "script=\"$PR_SCRIPT\"; expect=\"$PR_MODULES\"; log=\"$LOG_FILE\"; tree=\"$TREE\"; mark=0; pr_between; [ \"$script\" = \"$RESUME_SCRIPT\" ] && echo resumed || echo again; echo \"$expect\"",
        )
    };
    let resumed = || with_functions(&env, "resume_from \"$TREE\"; echo \"$expect\"");
    let done = || std::fs::read_to_string(tmp.path().join("resume.done")).ok();
    set(&syn, "Synthesis Status : Successful\n", 7200);
    set(&fit, "Fitter Status : Successful - Fri Oct 23 13:05:20 2026\nFamily : Stratix 10\n", 60);
    assert_eq!(between(), "resumed\nTiming Analyzer,Assembler\n", "an attempt that died past its fit");
    assert_eq!(done().as_deref(), Some("quartus_fit\n"), "the fit is marked run");
    assert_eq!(resumed(), "Timing Analyzer,Assembler\n", "quartus.sh resume of a fitted persona");
    assert_eq!(done().as_deref(), Some("quartus_fit\n"));
    for (what, fit_text, fit_age) in [("a failed fit", "Fitter Status : Failed - Fri Oct 23 13:05:20 2026\n", 60), ("a fit older than the synthesis", "Fitter Status : Successful - Thu Oct 22 09:00:00 2026\n", 9000)] {
        set(&fit, fit_text, fit_age);
        assert_eq!(between(), "resumed\nFitter,Timing Analyzer,Assembler\n", "{what}");
        assert_eq!(done(), None, "{what}: nothing marked run");
        assert_eq!(resumed(), "Fitter,Timing Analyzer,Assembler\n", "{what}");
        assert_eq!(done(), None, "{what}");
    }
}

/// A stand-in for the container: each run appends to `runs` and prints the next line of `outcomes`, `transient` or
/// an exit status, then exits as it says; a tool's start-up always passes.
fn stub(tmp: &Path, outcomes: &[&str]) -> String {
    std::fs::write(tmp.join("outcomes"), outcomes.join("\n") + "\n").unwrap();
    format!(
        r#"run_command() {{ echo "fake {dir}"; }}
           startup_probe() {{ :; }}
           fake() {{ local n; n=$(($(wc -l < "$1/runs" 2>/dev/null || echo 0) + 1)); echo run >> "$1/runs";
                    local o; o=$(sed -n "${{n}}p" "$1/outcomes");
                    if [ "$o" = transient ]; then echo 'Fatal Error: DDM_T::verify_token(token) : Cannot identify the client'; return 1; fi;
                    echo "exit $o"; return "$o"; }}
"#,
        dir = tmp.display()
    )
}

#[test]
fn attempts_run_a_transient_failure_again_and_stop_at_a_real_one_or_the_limit() {
    let cases: [(&[&str], usize, &str); 4] = [
        (&["0"], 1, "0"),
        (&["transient", "0"], 2, "0"),
        (&["transient", "7"], 2, "7"),
        (&["transient", "transient", "transient", "0"], 3, "1"),
    ];
    for (outcomes, runs, status) in cases {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("job.log");
        std::fs::write(&log, "").unwrap();
        let body = format!(
            "{} PROBE_WAIT=0; ATTEMPTS=3; script=x; log=\"$LOG_FILE\"; attempts job \"$DIR\" \"$LOG_FILE\" :; echo \"$status\"",
            stub(tmp.path(), outcomes)
        );
        let out = with_functions(&[("LOG_FILE", log.as_path()), ("DIR", tmp.path())], &body);
        let ran = std::fs::read_to_string(tmp.path().join("runs")).unwrap().lines().count();
        assert_eq!(ran, runs, "{outcomes:?}");
        assert_eq!(out.lines().last(), Some(status), "{outcomes:?}: {out}");
    }
}

#[test]
fn a_rows_evidence_is_the_last_attempts_exit_its_output_the_modules_and_the_start() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("job.log");
    std::fs::write(&log, "attempt 1: Cannot identify the client\nattempt 2: fine\n").unwrap();
    let mark = "attempt 1: Cannot identify the client\n".len();
    let recorder = tmp.path().join("recorder");
    std::fs::write(&recorder, "#!/usr/bin/env bash\nfor a in \"$@\"; do echo \"$a\"; done > \"$(dirname \"$0\")/args\"\ncp \"$4\" \"$(dirname \"$0\")/last\"\n").unwrap();
    Command::new("chmod").arg("+x").arg(&recorder).status().unwrap();
    let body = format!(
        "QUARTUS_RECORD=\"$RECORDER\"; LEDGER=/ledger.json; status=0; mark={mark}; since=1234; expect='Synthesis,Fitter'; record /reports rev job \"$LOG_FILE\" /in.v"
    );
    with_functions(&[("LOG_FILE", log.as_path()), ("RECORDER", recorder.as_path())], &body);
    let args: Vec<String> = std::fs::read_to_string(tmp.path().join("args")).unwrap().lines().map(str::to_string).collect();
    assert_eq!(args[0..2], ["--exit", "0"]);
    assert_eq!(args[2], "--log");
    assert_eq!(args[4..], ["--expect", "Synthesis,Fitter", "--started", "1234", "/reports", "rev", "job", "/ledger.json", "/in.v"]);
    assert_eq!(std::fs::read_to_string(tmp.path().join("last")).unwrap(), "attempt 2: fine\n", "the last attempt's output alone");
    assert!(!Path::new(&args[3]).exists(), "the excerpt is removed after");
}

/// A stand-in for the start-up probe: each call appends to `probes` and fails while it has been called fewer than
/// `failing` times.
fn probe_stub(tmp: &Path, failing: usize) -> String {
    format!(
        r#"startup_probe() {{ echo probe >> "{dir}/probes"; [ "$(wc -l < "{dir}/probes")" -gt {failing} ]; }}
"#,
        dir = tmp.display()
    )
}

#[test]
fn each_attempt_waits_until_a_quartus_tool_starts_and_a_check_that_never_passes_ends_the_job() {
    // The DDM start-up check passes or fails by the wall clock at a tool's start: probed before every attempt, an
    // attempt starts only once a tool does, however long the window lasts.
    let cases: [(&[&str], usize, usize, usize); 3] = [(&["0"], 0, 1, 1), (&["0"], 3, 4, 1), (&["transient", "0"], 2, 4, 2)];
    for (outcomes, failing, probes, runs) in cases {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("job.log");
        std::fs::write(&log, "").unwrap();
        let body = format!(
            "{}{} PROBE_WAIT=0; STARTUP_LIMIT=60; ATTEMPTS=3; script=x; log=\"$LOG_FILE\"; attempts job \"$DIR\" \"$LOG_FILE\" :; echo \"$status\"",
            stub(tmp.path(), outcomes),
            probe_stub(tmp.path(), failing)
        );
        let out = with_functions(&[("LOG_FILE", log.as_path()), ("DIR", tmp.path())], &body);
        let count = |f: &str| std::fs::read_to_string(tmp.path().join(f)).map_or(0, |t| t.lines().count());
        assert_eq!((count("probes"), count("runs")), (probes, runs), "{outcomes:?} with the check failing {failing} times");
        assert_eq!(out.lines().last(), Some("0"), "{outcomes:?}: {out}");
        let said = std::fs::read_to_string(&log).unwrap();
        assert_eq!(said.contains("waits for the DDM start-up check"), failing > 0, "{said}");
    }
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("job.log");
    std::fs::write(&log, "").unwrap();
    let body = format!(
        "{}{} PROBE_WAIT=1; STARTUP_LIMIT=2; ATTEMPTS=3; script=x; attempts job \"$DIR\" \"$LOG_FILE\" :",
        stub(tmp.path(), &["0"]),
        probe_stub(tmp.path(), 1000)
    );
    let o = Command::new("bash")
        .arg("-c")
        .arg(format!("set -euo pipefail; QUARTUS_LIB=1 source {}; {body}", script().display()))
        .env("LOG_FILE", &log)
        .env("DIR", tmp.path())
        .output()
        .unwrap();
    assert!(!o.status.success(), "a check that never passes ends the job");
    assert!(String::from_utf8_lossy(&o.stderr).contains("no Quartus tool started"), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(!tmp.path().join("runs").exists(), "and nothing ran");
}

#[test]
fn a_resumed_jobs_row_names_the_compile_it_went_on_from() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("job.log");
    std::fs::write(&log, "attempt 1: fine\n").unwrap();
    let recorder = tmp.path().join("recorder");
    std::fs::write(&recorder, "#!/usr/bin/env bash\nfor a in \"$@\"; do echo \"$a\"; done > \"$(dirname \"$0\")/args\"\n").unwrap();
    Command::new("chmod").arg("+x").arg(&recorder).status().unwrap();
    let env = [("LOG_FILE", log.as_path()), ("RECORDER", recorder.as_path())];
    let args = |resumed: &str| -> Vec<String> {
        let body = format!(
            "QUARTUS_RECORD=\"$RECORDER\"; LEDGER=/ledger.json; status=0; mark=0; since=1234; expect='Timing Analyzer,Assembler'; resumed='{resumed}'; record /reports rev job \"$LOG_FILE\" /in.v"
        );
        with_functions(&env, &body);
        std::fs::read_to_string(tmp.path().join("args")).unwrap().lines().map(str::to_string).collect()
    };
    let named = args("hybrid3-pr");
    assert_eq!(named[6..], ["--started", "1234", "--resumed", "hybrid3-pr", "/reports", "rev", "job", "/ledger.json", "/in.v"], "the row names the compile it went on from");
    let fresh = args("");
    assert!(!fresh.contains(&"--resumed".to_string()), "a job that went on from nothing names nothing: {fresh:?}");
}

#[test]
fn the_compile_that_ran_a_trees_fitter_is_the_one_its_next_resume_names() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    let env = [("TREE", tree.as_path())];
    assert_eq!(with_functions(&env, "resumed_from \"$TREE\""), "", "a tree no compile has fitted names none");
    with_functions(&env, "remember \"$TREE\" hybrid3-pr");
    assert_eq!(std::fs::read_to_string(tree.join("compiled.by")).unwrap(), "hybrid3-pr\n");
    assert_eq!(with_functions(&env, "resumed_from \"$TREE\""), "hybrid3-pr\n");
    let text = std::fs::read_to_string(script()).unwrap();
    let branch = &text[text.find("  pr|resume)").expect("the persona branch")..];
    let recorded = branch.find("record \"$tree/build").expect("the persona's record");
    let remembered = branch.find("case \",$expect,\" in *,Fitter,*) remember \"$tree\" \"$job\" ;; esac").expect("a compile that ran the fitter remembered");
    assert!(recorded < remembered, "remembered once recorded");
    let read = branch.find("resumed=$(resumed_from \"$tree\")").expect("a resume reads the compile it goes on from");
    assert!(read < branch.find("attempts \"$job\"").unwrap(), "before it runs");
    assert!(text.trim_end().ends_with("main \"$@\"; exit"), "the dispatch read whole before it runs, the exit on its line");
}

#[test]
fn a_job_whose_row_could_not_be_recorded_is_refused_before_it_queues() {
    // No cargo on the PATH and no built recorder: the job would run its hour of Quartus and end with no row.
    let tmp = tempfile::tempdir().unwrap();
    let (jobs, logs, lock) = (tmp.path().join("jobs"), tmp.path().join("logs"), tmp.path().join("lock"));
    let v = design(tmp.path());
    let bare = Command::new("bash")
        .arg(script())
        .args(["syn", "unrecorded", v.to_str().unwrap(), "unit"])
        .env("PATH", "/usr/bin:/bin")
        .env_remove("QUARTUS_RECORD")
        .env("QUARTUS_JOBS", &jobs)
        .env("QUARTUS_LOGS", &logs)
        .env("QUARTUS_LOCK", &lock)
        .env_remove("QUARTUS_DRY_RUN")
        .output()
        .unwrap();
    assert_eq!(bare.status.code(), Some(2), "{}", String::from_utf8_lossy(&bare.stderr));
    assert!(String::from_utf8_lossy(&bare.stderr).contains("could not be recorded"), "{}", String::from_utf8_lossy(&bare.stderr));
    assert!(!jobs.join("unrecorded").exists() && !lock.exists() && !logs.join("unrecorded.log").exists(), "refused before a project, the queue or a log");
    // A recorder named that is no executable, for a persona: refused the same way.
    let tree = tmp.path().join("tree");
    std::fs::create_dir_all(tree.join("afu/hw/rtl")).unwrap();
    std::fs::write(tree.join("afu/hw/rtl/sources.txt"), "unit.v\n").unwrap();
    let named = Command::new("bash")
        .arg(script())
        .args(["pr", "unrecorded-pr", tree.to_str().unwrap()])
        .env("QUARTUS_RECORD", tmp.path().join("no-such-recorder"))
        .env("QUARTUS_JOBS", &jobs)
        .env("QUARTUS_LOGS", &logs)
        .env("QUARTUS_LOCK", &lock)
        .env_remove("QUARTUS_DRY_RUN")
        .output()
        .unwrap();
    assert_eq!(named.status.code(), Some(2), "{}", String::from_utf8_lossy(&named.stderr));
    assert!(String::from_utf8_lossy(&named.stderr).contains("no executable"), "{}", String::from_utf8_lossy(&named.stderr));
    assert!(!lock.exists() && !logs.join("unrecorded-pr.log").exists());
    // Asked before every real run takes the queue.
    let text = std::fs::read_to_string(script()).unwrap();
    for branch in ["      [ \"${QUARTUS_DRY_RUN:-}\" = 1 ] || { recordable; log=$(new_log \"$job\"); }", "      recordable\n      log=$(new_log \"$job\")"] {
        assert_eq!(text.matches(branch).count(), 1, "{branch}");
    }
}

/// The floorplan `scripts/d5005/flat-fim.sh` leaves a flat image's FIM, and the one the release ships for its PR base.
const WHOLE_DEVICE: &str = "# A flat image (scripts/d5005/flat-fim.sh): afu_main is no partial-reconfiguration partition and has no place or route\n# region, so the accelerator is placed over the whole device beside the FIM's static logic.\n";
const PERSONA_REGION: &str = "set_global_assignment -name REVISION_TYPE PR_BASE\n\
set_instance_assignment -name PARTITION persona1 -to afu_top|port_gasket|pr_slot|afu_main -entity iofs_top\n\
set_instance_assignment -name PARTIAL_RECONFIGURATION_PARTITION ON -to afu_top|port_gasket|pr_slot|afu_main -entity iofs_top\n\
set_instance_assignment -name PLACE_REGION \"X240 Y0 X280 Y430\" -to afu_top|port_gasket|pr_slot|afu_main\n\
set_instance_assignment -name RESERVE_PLACE_REGION ON -to afu_top|port_gasket|pr_slot|afu_main\n";

/// A tree as build-afu.sh --flat leaves it: the accelerator's, and the FIM's source with `floorplan` as its base
/// revision's floorplan, beside the PIM.
fn flat_tree(dir: &Path, floorplan: &str) -> PathBuf {
    let tree = dir.join("tree");
    std::fs::create_dir_all(tree.join("afu/hw/rtl")).unwrap();
    std::fs::write(tree.join("afu/hw/rtl/sources.txt"), "logos.v\n").unwrap();
    let fim = tree.join("ofs/ofs-d5005-ofs-2024.1-1");
    std::fs::create_dir_all(fim.join("syn/setup")).unwrap();
    std::fs::create_dir_all(fim.join("ofs-common/scripts/common/syn")).unwrap();
    std::fs::write(fim.join("ofs-common/scripts/common/syn/build_top.sh"), "").unwrap();
    std::fs::write(fim.join("syn/setup/sr_logic_lock_region.tcl"), floorplan).unwrap();
    std::fs::create_dir_all(tree.join("ofs/ofs-platform-afu-bbb-ofs-2024.1-1/plat_if_develop/ofs_plat_if")).unwrap();
    tree
}

/// Whether `parts` occur in `cmd` in their order.
fn in_order(cmd: &str, parts: &[&str]) -> bool {
    let mut from = 0;
    for part in parts {
        match cmd[from..].find(part) {
            Some(at) => from += at + part.len(),
            None => return false,
        }
    }
    true
}

/// A flat build (`flat <job> <tree>`) compiles the FIM's own source with the accelerator inside it, in the job's
/// container with the tree as /work: the FIM's setup into /work/fim with the PIM beside it and the accelerator's
/// sources.txt as the AFU the PIM wraps, then its compile, with the processors stated between the two only when asked.
/// It never runs the FIM's finish (its flash images, which a JTAG-loaded image has no use for) and never the PR
/// template's afu_synth.
#[test]
fn a_flat_build_compiles_the_fims_own_source_with_the_accelerator_inside_it() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = flat_tree(tmp.path(), WHOLE_DEVICE);
    let o = dry(&tmp.path().join("jobs"), None, &["flat", "hybrid7-flat", tree.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let cmd = stdout(&o);
    assert!(carries(&cmd, "--name logos-q-hybrid7-flat"), "{cmd}");
    assert!(carries(&cmd, &format!("{}:/work", tree.display())), "the tree is /work: {cmd}");
    assert!(
        in_order(
            &cmd,
            &[
                "cd /work/ofs/ofs-d5005-ofs-2024.1-1",
                "OFS_PLATFORM_AFU_BBB=/work/ofs/ofs-platform-afu-bbb-ofs-2024.1-1",
                "AFU_WITH_PIM=/work/afu/hw/rtl/sources.txt",
                "build_top.sh --stage=setup d5005 /work/fim",
                "build_top.sh -e --stage=compile d5005 /work/fim",
                "/work/fim/syn/syn_top/output_files/d5005.sof",
            ]
        ),
        "the FIM's setup, then its compile, then its image: {cmd}"
    );
    for absent in ["--stage=finish", "afu_synth", "pr-template", "NUM_PARALLEL_PROCESSORS"] {
        assert!(!cmd.contains(absent), "{absent} in {cmd}");
    }
    let o = dry(&tmp.path().join("jobs"), Some("24"), &["flat", "hybrid7-flat-24", tree.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let cmd = stdout(&o);
    assert!(
        in_order(
            &cmd,
            &[
                "--stage=setup",
                "set_global_assignment -name NUM_PARALLEL_PROCESSORS 24",
                "/work/fim/syn/syn_top/d5005.qsf",
                "--stage=compile",
            ]
        ),
        "the processors stated in the FIM's project between its setup and its compile: {cmd}"
    );
}

/// A flat build's attempt from its setup starts from no work tree: the FIM's setup asks before it compiles into a work
/// directory an earlier attempt left, and gives up when no one answers, and a module an earlier attempt marked run
/// (resume.done) is not this attempt's — so the attempt removes both before its setup. An attempt resumed from its
/// fitter keeps both.
#[test]
fn a_flat_builds_attempt_from_its_setup_starts_from_no_work_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = flat_tree(tmp.path(), WHOLE_DEVICE);
    let o = dry(&tmp.path().join("jobs"), None, &["flat", "hybrid7-flat", tree.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let cmd = stdout(&o);
    assert!(
        in_order(&cmd, &["rm -rf /work/fim /work/resume.done", "build_top.sh --stage=setup d5005 /work/fim"]),
        "the earlier attempt's work tree and marks removed before the setup: {cmd}"
    );
    let text = std::fs::read_to_string(script()).unwrap();
    let at = text.find("FLAT_RESUME_SCRIPT='").expect("the flat resume");
    let resume = &text[at..];
    let resume = &resume[..resume[20..].find('\'').expect("its end") + 20];
    assert!(!resume.contains("rm -rf") && resume.contains("/work/resume.done"), "a resume keeps the work tree and its marks: {resume}");
}

/// A flat build generates the FIM's IP as its own step, after the setup and before the compile: the IP generation tool
/// run again until it finishes — each run generates only what the last left ungenerated, its regeneration policy
/// skipping the rest, while a run's generation server dies or deadlocks at random — the setup marked done first, so an
/// attempt that dies in it goes on from it rather than from no work tree.
#[test]
fn a_flat_build_generates_its_ip_until_every_ip_is_generated_before_it_compiles() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = flat_tree(tmp.path(), WHOLE_DEVICE);
    let o = dry(&tmp.path().join("jobs"), None, &["flat", "hybrid7-flat", tree.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let cmd = stdout(&o);
    assert!(
        in_order(
            &cmd,
            &[
                "build_top.sh --stage=setup d5005 /work/fim",
                "echo setup >> /work/resume.done",
                "cd /work/fim/syn/syn_top",
                "until quartus_ipgenerate --dni d5005 -c d5005 --run_default_mode_op",
                "build_top.sh -e --stage=compile d5005 /work/fim",
            ]
        ),
        "the setup marked, the IP generated until it is all generated, then the compile: {cmd}"
    );
}

/// `scripts/d5005/ip_cache.sh <cmd> <fim> <state> [<cache>]` with the Quartus version at `version`: its exit status
/// and output.
fn ip_cache(args: &[&Path], cmd: &str, version: &Path) -> (Option<i32>, String) {
    let o = Command::new("bash")
        .arg(root().join("scripts/d5005/ip_cache.sh"))
        .arg(cmd)
        .args(args)
        .env("QUARTUS_VERSION_FILE", version)
        .output()
        .expect("bash runs the IP cache");
    (o.status.code(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

/// A FIM work tree as the setup leaves it: IP sources (an .ip, a .qsys, a component's _hw.tcl) and the project directory
/// syn/ with its settings, `project` its settings file's text.
fn fim_tree(dir: &Path, project: &str) -> PathBuf {
    let fim = dir.to_path_buf();
    for (path, text) in [
        ("ipss/pcie/pcie_ep.ip", "<ip pcie/>"),
        ("src/pd_qsys/fabric/apf.qsys", "<system apf/>"),
        ("src/pd_qsys/fabric/shim_hw.tcl", "package require qsys"),
        ("ofs-common/src/rtl/fme.sv", "module fme; endmodule"),
        ("syn/syn_top/d5005.qsf", project),
    ] {
        std::fs::create_dir_all(fim.join(path).parent().unwrap()).unwrap();
        std::fs::write(fim.join(path), text).unwrap();
    }
    fim
}

/// What a generation writes into a tree: each IP's outputs beside its source, and the project's database and report,
/// each dated long before any restore.
fn generate(fim: &Path) {
    for (path, text) in [
        ("ipss/pcie/pcie_ep/synth/pcie_ep.v", "module pcie_ep; endmodule"),
        ("ipss/pcie/pcie_ep/pcie_ep.qgsynthc", "generated"),
        ("src/pd_qsys/fabric/apf/synth/apf.v", "module apf; endmodule"),
        ("src/pd_qsys/fabric/apf.sopcinfo", "<sopcinfo/>"),
        ("syn/syn_top/qdb/_compiler/d5005/ipgen.ddb", "this build's database"),
        ("syn/syn_top/d5005.ipgen.rpt", "this build's report"),
    ] {
        std::fs::create_dir_all(fim.join(path).parent().unwrap()).unwrap();
        std::fs::write(fim.join(path), text).unwrap();
        let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(978_307_200);
        std::fs::File::options().write(true).open(fim.join(path)).unwrap().set_modified(old).unwrap();
    }
}

/// Every file under a directory and its bytes.
fn files_of(dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.insert(p.strip_prefix(dir).unwrap().display().to_string(), std::fs::read(&p).unwrap());
            }
        }
    }
    out
}

/// The IP cache keeps what a generation wrote into a tree since its setup outside the project directory — each IP's
/// outputs, never the project's database or report — and a tree of the same IP sources and Quartus, set up for another
/// build (its own project settings), restores them byte for byte, each given the time of its restoring, its project
/// directory as it was: the restored tree's IP is the generated tree's. A store finds an entry kept already and leaves it.
#[test]
fn the_ip_cache_keeps_a_generations_outputs_and_restores_them_into_a_tree_of_the_same_sources() {
    let tmp = tempfile::tempdir().unwrap();
    let version = tmp.path().join("version.txt");
    std::fs::write(&version, "Version=23.4.0.79\n").unwrap();
    let cache = tmp.path().join("cache");
    let a = fim_tree(&tmp.path().join("a/fim"), "set_global_assignment -name NUM_PARALLEL_PROCESSORS 24");
    let sa = tmp.path().join("a/ipcache");
    assert_eq!(ip_cache(&[&a, &sa], "begin", &version).0, Some(0));
    assert_eq!(ip_cache(&[&a, &sa, &cache], "restore", &version).0, Some(1), "nothing kept yet");
    generate(&a);
    let (code, out) = ip_cache(&[&a, &sa, &cache], "store", &version);
    assert_eq!(code, Some(0), "{out}");
    let key = std::fs::read_to_string(sa.join("key")).unwrap();
    let kept = std::fs::read_to_string(cache.join(format!("{}.files", key.trim()))).unwrap();
    let mut kept: Vec<&str> = kept.lines().map(|l| &l[66..]).collect();
    kept.sort();
    assert_eq!(
        kept,
        ["./ipss/pcie/pcie_ep/pcie_ep.qgsynthc", "./ipss/pcie/pcie_ep/synth/pcie_ep.v", "./src/pd_qsys/fabric/apf.sopcinfo", "./src/pd_qsys/fabric/apf/synth/apf.v"],
        "the generation's outputs outside syn/"
    );
    let b = fim_tree(&tmp.path().join("b/fim"), "set_global_assignment -name NUM_PARALLEL_PROCESSORS 8");
    let sb = tmp.path().join("b/ipcache");
    assert_eq!(ip_cache(&[&b, &sb], "begin", &version).0, Some(0));
    assert_eq!(std::fs::read_to_string(sb.join("key")).unwrap(), key, "the same sources, another build's project");
    let project = std::fs::read(b.join("syn/syn_top/d5005.qsf")).unwrap();
    let before = std::time::SystemTime::now() - std::time::Duration::from_secs(1);
    let (code, out) = ip_cache(&[&b, &sb, &cache], "restore", &version);
    assert_eq!(code, Some(0), "{out}");
    let (ga, gb) = (files_of(&a), files_of(&b));
    for path in ["ipss/pcie/pcie_ep/synth/pcie_ep.v", "ipss/pcie/pcie_ep/pcie_ep.qgsynthc", "src/pd_qsys/fabric/apf/synth/apf.v", "src/pd_qsys/fabric/apf.sopcinfo"] {
        assert_eq!(gb.get(path), ga.get(path), "{path} restored byte for byte");
        let restored = std::fs::metadata(b.join(path)).unwrap().modified().unwrap();
        assert!(restored >= before, "{path} given the time of its restoring");
    }
    assert_eq!(std::fs::read(b.join("syn/syn_top/d5005.qsf")).unwrap(), project, "the project's settings its own");
    assert!(!b.join("syn/syn_top/qdb").exists() && !b.join("syn/syn_top/d5005.ipgen.rpt").exists(), "no other build's database or report");
    let (code, out) = ip_cache(&[&a, &sa, &cache], "store", &version);
    assert!(code == Some(0) && out.contains("kept already"), "{out}");
}

/// The IP cache's key is the tree's IP sources and the Quartus version: a tree whose .ip, .qsys or _hw.tcl differs by a
/// byte, or whose Quartus is another, restores nothing and is left as it was; one whose project or other sources differ
/// restores. A tree set up before its setup kept a key restores nothing and keeps nothing.
#[test]
fn the_ip_cache_restores_only_into_a_tree_of_the_same_ip_sources_and_quartus() {
    let tmp = tempfile::tempdir().unwrap();
    let version = tmp.path().join("version.txt");
    std::fs::write(&version, "Version=23.4.0.79\n").unwrap();
    let other = tmp.path().join("other.txt");
    std::fs::write(&other, "Version=24.1.0.115\n").unwrap();
    let cache = tmp.path().join("cache");
    let a = fim_tree(&tmp.path().join("a/fim"), "project");
    let sa = tmp.path().join("a/ipcache");
    ip_cache(&[&a, &sa], "begin", &version);
    generate(&a);
    assert_eq!(ip_cache(&[&a, &sa, &cache], "store", &version).0, Some(0));
    let changed: [(&str, Option<(&str, &str)>, &Path); 5] = [
        ("an .ip", Some(("ipss/pcie/pcie_ep.ip", "<ip pcie gen4/>")), &version),
        ("a .qsys", Some(("src/pd_qsys/fabric/apf.qsys", "<system apf2/>")), &version),
        ("a component", Some(("src/pd_qsys/fabric/shim_hw.tcl", "package require qsys 2")), &version),
        ("another Quartus", None, &other),
        ("another RTL source", Some(("ofs-common/src/rtl/fme.sv", "module fme2; endmodule")), &version),
    ];
    for (k, (what, edit, quartus)) in changed.into_iter().enumerate() {
        let t = fim_tree(&tmp.path().join(format!("t{k}/fim")), "project");
        if let Some((path, text)) = edit {
            std::fs::write(t.join(path), text).unwrap();
        }
        let st = tmp.path().join(format!("t{k}/ipcache"));
        ip_cache(&[&t, &st], "begin", quartus);
        let was = files_of(&t);
        let (code, out) = ip_cache(&[&t, &st, &cache], "restore", quartus);
        if what == "another RTL source" {
            assert_eq!(code, Some(0), "{what}: {out}");
        } else {
            assert_eq!(code, Some(1), "{what}: {out}");
            assert_eq!(files_of(&t), was, "{what}: the tree as it was");
        }
    }
    let late = fim_tree(&tmp.path().join("late/fim"), "project");
    let sl = tmp.path().join("late/ipcache");
    assert_eq!(ip_cache(&[&late, &sl, &cache], "restore", &version).0, Some(1), "no key");
    generate(&late);
    assert_eq!(ip_cache(&[&late, &sl, &cache], "store", &version).0, Some(1), "no manifest, nothing kept");
}

/// A kept entry that does not check against its own manifest — a file's bytes changed, a file missing, a file more — is
/// set aside (`<key>.refused.*`) and restores nothing, the tree left as it was; the next generation keeps a good entry.
#[test]
fn the_ip_cache_sets_aside_an_entry_that_does_not_check() {
    let tmp = tempfile::tempdir().unwrap();
    let version = tmp.path().join("version.txt");
    std::fs::write(&version, "Version=23.4.0.79\n").unwrap();
    for (k, spoil) in ["bytes", "missing", "more"].into_iter().enumerate() {
        let cache = tmp.path().join(format!("cache{k}"));
        let a = fim_tree(&tmp.path().join(format!("a{k}/fim")), "project");
        let sa = tmp.path().join(format!("a{k}/ipcache"));
        ip_cache(&[&a, &sa], "begin", &version);
        generate(&a);
        ip_cache(&[&a, &sa, &cache], "store", &version);
        let key = std::fs::read_to_string(sa.join("key")).unwrap().trim().to_string();
        let tar = cache.join(format!("{key}.tar"));
        let x = tmp.path().join(format!("x{k}"));
        std::fs::create_dir_all(&x).unwrap();
        assert!(Command::new("tar").arg("-xf").arg(&tar).arg("-C").arg(&x).status().unwrap().success());
        match spoil {
            "bytes" => std::fs::write(x.join("ipss/pcie/pcie_ep/synth/pcie_ep.v"), "module pcie_ep_tampered; endmodule").unwrap(),
            "missing" => std::fs::remove_file(x.join("src/pd_qsys/fabric/apf.sopcinfo")).unwrap(),
            _ => std::fs::write(x.join("ipss/pcie/pcie_ep/stray.v"), "module stray; endmodule").unwrap(),
        }
        assert!(Command::new("tar").arg("-cf").arg(&tar).arg("-C").arg(&x).arg(".").status().unwrap().success());
        let b = fim_tree(&tmp.path().join(format!("b{k}/fim")), "project");
        let sb = tmp.path().join(format!("b{k}/ipcache"));
        ip_cache(&[&b, &sb], "begin", &version);
        let was = files_of(&b);
        let (code, out) = ip_cache(&[&b, &sb, &cache], "restore", &version);
        assert_eq!(code, Some(1), "{spoil}: {out}");
        assert!(out.contains("does not check"), "{spoil}: {out}");
        assert_eq!(files_of(&b), was, "{spoil}: the tree as it was");
        assert!(cache.join(format!("{key}.refused.tar")).exists() && !tar.exists(), "{spoil}: set aside");
        generate(&b);
        assert_eq!(ip_cache(&[&b, &sb, &cache], "store", &version).0, Some(0), "{spoil}: a good entry kept");
        let c = fim_tree(&tmp.path().join(format!("c{k}/fim")), "project");
        let sc = tmp.path().join(format!("c{k}/ipcache"));
        ip_cache(&[&c, &sc], "begin", &version);
        assert_eq!(ip_cache(&[&c, &sc, &cache], "restore", &version).0, Some(0), "{spoil}: the good entry restores");
    }
}

/// `scripts/d5005/stall_watch.sh <window> <least> <bound> bash -c <body>`, its group read every second and given a
/// second's grace: its exit status, what it said, and how long it took.
fn stall_watch(window: u32, least: u32, bound: u32, body: &str) -> (Option<i32>, String, std::time::Duration) {
    let began = std::time::Instant::now();
    let o = Command::new("bash")
        .arg(root().join("scripts/d5005/stall_watch.sh"))
        .args([window.to_string(), least.to_string(), bound.to_string()])
        .args(["bash", "-c", body])
        .env("STALL_POLL", "1")
        .env("STALL_GRACE", "1")
        .output()
        .expect("bash runs the watch");
    (o.status.code(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)), began.elapsed())
}

/// Whether process `pid` is gone: no such process, or one dead and waiting for its parent to reap it.
fn gone(pid: &str) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).map_or(true, |s| s.rsplit_once(") ").is_some_and(|(_, rest)| rest.starts_with('Z')))
}

/// A command whose processes stand still — a parent asleep waiting on a child asleep, as an IP generation waits on a
/// helper that died at its start — is stopped once they take under the least CPU and write nothing in the window, long
/// before its bound: the watch exits 124 saying how little they took, and no process of the command is left.
#[test]
fn the_stall_watch_stops_a_command_whose_processes_stand_still() {
    let tmp = tempfile::tempdir().unwrap();
    let pids = tmp.path().join("pids");
    let (code, out, took) = stall_watch(5, 1, 40, &format!("sleep 600 & echo $! > {0}; echo $$ >> {0}; wait", pids.display()));
    assert_eq!(code, Some(124), "{out}");
    assert!(out.contains("s of CPU in") && out.contains("wrote nothing, under 1; its processes stopped"), "stopped at its window, not its bound: {out}");
    assert!(took < std::time::Duration::from_secs(30), "{took:?}");
    let pids = std::fs::read_to_string(&pids).unwrap();
    for pid in pids.lines() {
        let mut waited = 0;
        while !gone(pid) && waited < 50 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            waited += 1;
        }
        assert!(gone(pid), "process {pid} of the command left running");
    }
}

/// A command whose processes work is left to finish, its own status passed on, through several windows: a parent asleep
/// on a child that keeps a core busy (the group's CPU counted, its parent's alone near none), and a parent running short
/// busy children one after another (each child's CPU counted once its parent reaps it, as a generator's are), each then
/// exiting 7.
#[test]
fn the_stall_watch_leaves_a_working_command_and_passes_its_status_on() {
    let busy_child = "bash -c 'end=$((SECONDS + 12)); while [ $SECONDS -lt $end ]; do :; done' & wait; exit 7";
    let short_children = "end=$((SECONDS + 12)); while [ $SECONDS -lt $end ]; do bash -c 'i=0; while [ $i -lt 100000 ]; do i=$((i + 1)); done'; done; exit 7";
    for (what, body) in [("a busy child", busy_child), ("short children", short_children)] {
        let (code, out, took) = stall_watch(5, 1, 600, body);
        assert_eq!(code, Some(7), "{what}: {out}");
        assert!(!out.contains("stopped"), "{what}: {out}");
        assert!(took >= std::time::Duration::from_secs(10), "{what}: ran to its end: {took:?}");
    }
}

/// A command whose processes take next to no CPU but keep writing — a synthesis's pass over the IP it finds generated
/// logs each IP at about sixteen CPU-seconds in five minutes — is left to finish, every line of its output passed on and
/// its status its own; one that writes and then falls silent is stopped a window after its last line.
#[test]
fn the_stall_watch_leaves_a_command_that_keeps_writing_and_stops_one_that_falls_silent() {
    let (code, out, took) = stall_watch(5, 1, 600, "for i in $(seq 12); do echo line $i; sleep 1; done; exit 7");
    assert_eq!(code, Some(7), "{out}");
    assert!(!out.contains("stopped"), "{out}");
    assert!((1..=12).all(|i| out.contains(&format!("line {i}\n"))), "every line passed on: {out}");
    assert!(took >= std::time::Duration::from_secs(11), "ran to its end: {took:?}");
    let (code, out, took) = stall_watch(5, 1, 40, "for i in $(seq 3); do echo line $i; sleep 1; done; sleep 600");
    assert_eq!(code, Some(124), "{out}");
    assert!(out.contains("line 3\n") && out.contains("wrote nothing, under 1; its processes stopped"), "{out}");
    assert!(took < std::time::Duration::from_secs(30), "{took:?}");
}

/// A command still running at its bound is stopped then, however busy: the watch exits 124 saying so.
#[test]
fn the_stall_watch_stops_a_command_at_its_bound() {
    let (code, out, took) = stall_watch(600, 1, 3, "end=$((SECONDS + 30)); while [ $SECONDS -lt $end ]; do :; done");
    assert_eq!(code, Some(124), "{out}");
    assert!(out.contains("still ran 3 s after it started; its processes stopped"), "{out}");
    assert!(took < std::time::Duration::from_secs(25), "{took:?}");
}

/// A flat build keeps its setup's manifest and key once its setup and processor count are done, and its IP step restores
/// the IP a build of the same sources kept before it would generate, generating only when it cannot — each generation
/// under the stall watch, stopped once its processes take under thirty seconds of CPU and write nothing in three
/// minutes, or at forty minutes — and keeping what it generated only once the design built from it has synthesized,
/// never what it restored; the container has the cache as /ipcache and the scripts as /logos, read-only.
#[test]
fn a_flat_build_restores_its_ip_from_the_cache_before_generating_it_and_keeps_what_it_generates() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = flat_tree(tmp.path(), WHOLE_DEVICE);
    let o = Command::new("bash")
        .arg(script())
        .args(["flat", "hybrid9c-flat", tree.to_str().unwrap()])
        .env("QUARTUS_DRY_RUN", "1")
        .env("QUARTUS_JOBS", tmp.path().join("jobs"))
        .env("QUARTUS_LOGS", tmp.path().join("logs"))
        .env("Q234", "/opt/q234-test")
        .env("QUARTUS_PROCESSORS", "24")
        .env("QUARTUS_IP_CACHE", tmp.path().join("ip-cache"))
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let cmd = stdout(&o);
    assert!(carries(&cmd, &format!("{}:/ipcache", tmp.path().join("ip-cache").display())), "the cache: {cmd}");
    assert!(carries(&cmd, &format!("{}:/logos:ro", root().join("scripts/d5005").canonicalize().unwrap().display())), "the scripts: {cmd}");
    assert!(
        in_order(
            &cmd,
            &[
                "build_top.sh --stage=setup d5005 /work/fim",
                "NUM_PARALLEL_PROCESSORS 24",
                "/logos/ip_cache.sh begin /work/fim /work/ipcache",
                "echo setup >> /work/resume.done",
                "cd /work/fim/syn/syn_top",
                "quartus_ipgenerate() { /logos/stall_watch.sh 180 30 2400 \"$(type -P quartus_ipgenerate)\" \"$@\"; }",
                "restored=1",
                "if ! /logos/ip_cache.sh restore /work/fim /work/ipcache /ipcache; then",
                "restored=0",
                "until quartus_ipgenerate --dni d5005 -c d5005 --run_default_mode_op",
                "/logos/stall_watch.sh 300 30 14400 ./ofs-common/scripts/common/syn/build_top.sh -e --stage=compile d5005 /work/fim",
                "if [ \"$restored\" = 0 ] && ! /logos/ip_cache.sh store /work/fim /work/ipcache /ipcache; then echo \"quartus.sh: the IP generated and not kept\"; fi",
            ]
        ),
        "the setup's manifest kept last, the IP restored or generated, the synthesis, then the generated IP kept: {cmd}"
    );
    assert_eq!(cmd.matches("/logos/ip_cache.sh store").count(), 1, "the IP kept at one place: {cmd}");
}

/// A flat build's attempt that died after its setup and before its synthesis finished goes on from its IP generation:
/// the work tree and the setup kept, the IP generation run until it finishes, then the compile; one that died before its
/// setup was marked starts again from no work tree.
#[test]
fn a_flat_attempt_that_dies_after_its_setup_goes_on_from_its_ip_generation() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("job.log");
    std::fs::write(&log, "Info: Running Quartus Prime IP Generation Tool\nError: UNAVAILABLE: Network closed for unknown reason\n").unwrap();
    let env = [("LOG_FILE", log.as_path()), ("TREE", tmp.path())];
    let after = || {
        with_functions(
            &env,
            "script=\"$FLAT_SCRIPT\"; expect=\"$FLAT_MODULES\"; log=\"$LOG_FILE\"; tree=\"$TREE\"; mark=0; flat_between; \
             if [ \"$script\" = \"$FLAT_IP_SCRIPT\" ]; then echo ip; elif [ \"$script\" = \"$FLAT_SCRIPT\" ]; then echo again; else echo other; fi",
        )
    };
    assert_eq!(after().trim(), "again", "no setup marked: from no work tree");
    std::fs::write(tmp.path().join("resume.done"), "setup\n").unwrap();
    assert_eq!(after().trim(), "ip", "the setup marked: from the IP generation");
    let text = std::fs::read_to_string(script()).unwrap();
    let at = text.find("FLAT_IP_SCRIPT='").expect("the flat IP resume");
    let ip = &text[at..];
    let ip = &ip[..ip[16..].find('\'').expect("its end") + 16];
    assert!(!ip.contains("rm -rf") && !ip.contains("--stage=setup"), "the work tree and its setup kept: {ip}");
    assert!(
        in_order(ip, &["cd /work/fim/syn/syn_top", "until quartus_ipgenerate --dni d5005 -c d5005 --run_default_mode_op", "build_top.sh -e --stage=compile d5005 /work/fim"]),
        "the IP generated until it is all generated, then the compile: {ip}"
    );
}

/// A flat synthesis (`flat-syn`) is a flat build stopped at its budget: the FIM's setup, its IP, its compile to its
/// synthesis's end and the budget held, and no fitter, timing analysis or assembly — the report a floorplan is drawn
/// from, its fit then `resume`'s. It is recorded as a synthesis, no fitter remembered for the tree; an attempt that dies
/// after its synthesis goes on to the budget alone, one that dies after its setup from its IP generation, never into a
/// fit.
#[test]
fn a_flat_synthesis_stops_at_its_budget_and_never_fits() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = flat_tree(tmp.path(), WHOLE_DEVICE);
    let o = dry(&tmp.path().join("jobs"), None, &["flat-syn", "hybrid8b-syn", tree.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let cmd = stdout(&o);
    assert!(carries(&cmd, "--name logos-q-hybrid8b-syn"), "{cmd}");
    assert!(
        in_order(
            &cmd,
            &[
                "rm -rf /work/fim /work/resume.done",
                "build_top.sh --stage=setup d5005 /work/fim",
                "echo setup >> /work/resume.done",
                "until quartus_ipgenerate --dni d5005 -c d5005 --run_default_mode_op",
                "./ofs-common/scripts/common/syn/build_top.sh -e --stage=compile d5005 /work/fim",
                "the design needs",
            ]
        ),
        "the setup, the IP, the synthesis, the budget: {cmd}"
    );
    for absent in ["quartus_fit", "quartus_sta", "quartus_asm", "d5005.sof", "ANALYSIS_AND_ELAB_ONLY="] {
        assert!(!cmd.contains(absent), "{absent} in {cmd}");
    }
    let log = tmp.path().join("job.log");
    let env = [("LOG_FILE", log.as_path()), ("TREE", tmp.path())];
    let after = || {
        with_functions(
            &env,
            "script=\"$FLAT_SYN_SCRIPT\"; expect=\"$FLAT_SYN_MODULES\"; log=\"$LOG_FILE\"; tree=\"$TREE\"; mark=0; flat_syn_between; \
             if [ \"$script\" = \"$FLAT_SYN_BUDGET_SCRIPT\" ]; then echo budget; elif [ \"$script\" = \"$FLAT_SYN_IP_SCRIPT\" ]; then echo ip; \
             elif [ \"$script\" = \"$FLAT_SYN_SCRIPT\" ]; then echo again; else echo other; fi; echo \"$expect\"",
        )
    };
    std::fs::write(&log, "Info: Running Quartus Prime Synthesis\nError: UNAVAILABLE: Network closed for unknown reason\n").unwrap();
    assert_eq!(after(), "again\nSynthesis\n", "nothing marked: from no work tree");
    std::fs::write(tmp.path().join("resume.done"), "setup\n").unwrap();
    assert_eq!(after(), "ip\nSynthesis\n", "the setup marked: from the IP generation");
    std::fs::write(&log, "Info: Quartus Prime Synthesis was successful. 0 errors\nError: DDM_T::verify_token: Cannot identify the client\n").unwrap();
    assert_eq!(after(), "budget\nSynthesis\n", "the synthesis done: the budget alone");
    for name in ["FLAT_SYN_IP_SCRIPT", "FLAT_SYN_BUDGET_SCRIPT"] {
        let body = with_functions(&env, &format!("printf '%s' \"${name}\""));
        assert!(body.contains("the design needs"), "{name} holds the budget: {body}");
        for absent in ["quartus_fit", "quartus_sta", "quartus_asm", "rm -rf", "--stage=setup"] {
            assert!(!body.contains(absent), "{name} holds {absent}: {body}");
        }
    }
    let ip = with_functions(&env, "printf '%s' \"$FLAT_SYN_IP_SCRIPT\"");
    assert!(
        in_order(&ip, &["until quartus_ipgenerate --dni d5005 -c d5005 --run_default_mode_op", "build_top.sh -e --stage=compile", "the design needs"]),
        "the IP generated, the synthesis, the budget: {ip}"
    );
    let text = std::fs::read_to_string(script()).unwrap();
    let branch = &text[text.find("    flat-syn)").expect("the flat-syn branch")..];
    let branch = &branch[..branch.find(";;").unwrap()];
    assert!(branch.contains("attempts \"$job\" \"$tree\" \"$log\" flat_syn_between") && !branch.contains("remember"), "{branch}");
}

/// A synthesis report of a flat build's two partitions as Quartus writes them: the whole design's and the debug fabric's
/// estimate of the ALMs each needs, its block memory bits and its DSP blocks.
fn synthesis_report(dir: &Path, root: (u64, u64, u64), fabric: (u64, u64, u64)) {
    let part = |name: &str, (alms, bits, dsps): (u64, u64, u64)| {
        format!(
            "; Synthesis Resource Usage Summary for Partition \"{name}\" ;\n+------+\n; Resource ; Usage ;\n+------+\n\
             ; Estimate of Logic utilization (ALMs needed) ; {alms}               ;\n;   ;   ;\n\
             ; Combinational ALUT usage for logic          ; {}                ;\n\
             ; Total block memory bits                     ; {bits}               ;\n\
             ; Total DSP Blocks                            ; {dsps}               ;\n",
            alms * 9 / 10
        )
    };
    std::fs::create_dir_all(dir.join("output_files")).unwrap();
    std::fs::write(dir.join("output_files/d5005.syn.rpt"), format!("{}\n{}", part("root_partition", root), part("auto_fab_0", fabric))).unwrap();
}

/// A flat build synthesizes alone before it fits: the FIM's compile run to its synthesis's end (build_top.sh's `-e`,
/// which sets the ANALYSIS_AND_ELAB_ONLY OFS's build_fim_compile.sh turns into `-end synthesis`; build_top.sh clears
/// any ANALYSIS_AND_ELAB_ONLY it is handed, so the variable alone runs the whole flow), then the design's synthesis
/// estimates — the ALMs it needs, its DSP blocks and its block memory bits, each summed over every partition — held to
/// the device's (933,120 ALMs, 5,760 DSP blocks, 11,721 M20Ks of 20,480 bits), then its fit, timing analysis and
/// assembly, each once across attempts; a flat attempt resumed from its fitter is held to them first too. A design the
/// device cannot hold, or a report that gives no estimate, stops before the fitter with its own exit status (5), saying
/// what it needs and what the device has: hybrid8's synthesis needed 1,056,208 ALMs, 113% of the device, and its fit
/// would have spent hours to fail.
#[test]
fn a_flat_build_fits_only_a_design_its_synthesis_says_the_device_holds() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = flat_tree(tmp.path(), WHOLE_DEVICE);
    let o = dry(&tmp.path().join("jobs"), None, &["flat", "hybrid8-flat", tree.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let cmd = stdout(&o);
    assert!(
        in_order(
            &cmd,
            &[
                "./ofs-common/scripts/common/syn/build_top.sh -e --stage=compile d5005 /work/fim",
                "Estimate of Logic utilization (ALMs needed)",
                "run quartus_fit --read_settings_files=on --write_settings_files=off d5005 -c d5005",
                "run quartus_sta d5005 -c d5005 --mode=finalize",
                "run quartus_asm --read_settings_files=on --write_settings_files=off d5005 -c d5005",
                "/work/fim/syn/syn_top/output_files/d5005.sof",
            ]
        ),
        "synthesis alone, the device's budget, then the fit, timing analysis and assembly: {cmd}"
    );
    let text = std::fs::read_to_string(script()).unwrap();
    let at = text.find("FLAT_RESUME_SCRIPT=").expect("the flat resume");
    let resume = &text[at..at + text[at..].find("d5005.sof'").expect("its end")];
    assert!(in_order(resume, &["$FLAT_BUDGET", "run quartus_fit"]), "a resumed attempt held to the budget first: {resume}");

    let dir = tmp.path().join("syn_top");
    let env = [("DIR", dir.as_path())];
    let budget = |root: (u64, u64, u64), fabric: (u64, u64, u64)| {
        synthesis_report(&dir, root, fabric);
        with_functions(&env, "s=0; ( cd \"$DIR\" && eval \"$FLAT_BUDGET\" ) || s=$?; echo \"status=$s\"")
    };
    let out = budget((1_056_208, 113_931_720, 4_648), (1_511, 132_736, 0));
    assert!(out.contains("status=5"), "hybrid8's synthesis stops before its fitter: {out}");
    assert!(
        out.contains("quartus.sh: the design needs 1057719 ALMs, 4648 DSP blocks and 114064456 block memory bits; the device has 933120, 5760 and 240046080: no fit"),
        "{out}"
    );
    let out = budget((600_000, 100_000_000, 4_000), (1_511, 132_736, 0));
    assert!(out.contains("status=0"), "a design the device holds goes on to its fit: {out}");
    assert!(out.contains("quartus.sh: the design needs 601511 ALMs (64%), 4000 DSP blocks (69%) and 100132736 block memory bits (41%)"), "{out}");
    let out = budget((600_000, 100_000_000, 5_800), (1_511, 132_736, 0));
    assert!(out.contains("status=5"), "too many DSP blocks stop it too: {out}");
    synthesis_report(&dir, (0, 0, 0), (0, 0, 0));
    std::fs::write(dir.join("output_files/d5005.syn.rpt"), "Synthesis Status : Successful\n").unwrap();
    let out = with_functions(&env, "s=0; ( cd \"$DIR\" && eval \"$FLAT_BUDGET\" ) || s=$?; echo \"status=$s\"");
    assert!(out.contains("status=5") && out.contains("gives no estimate"), "a report without its estimate stops it: {out}");
}

/// A flat build is refused before anything runs when its tree is not one: no accelerator, no FIM source beside it, or
/// a FIM whose floorplan still reserves the persona region for afu_main, which would compile the accelerator into the
/// region's 601,050 ALMs and 3,971 DSP blocks instead of the whole device.
#[test]
fn a_flat_build_of_a_tree_that_is_not_flat_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = tmp.path().join("jobs");
    let no_afu = flat_tree(&tmp.path().join("a"), WHOLE_DEVICE);
    std::fs::remove_dir_all(no_afu.join("afu")).unwrap();
    let no_fim = flat_tree(&tmp.path().join("b"), WHOLE_DEVICE);
    std::fs::remove_dir_all(no_fim.join("ofs/ofs-d5005-ofs-2024.1-1")).unwrap();
    let no_floorplan = flat_tree(&tmp.path().join("e"), WHOLE_DEVICE);
    std::fs::remove_file(no_floorplan.join("ofs/ofs-d5005-ofs-2024.1-1/syn/setup/sr_logic_lock_region.tcl")).unwrap();
    let no_pim = flat_tree(&tmp.path().join("c"), WHOLE_DEVICE);
    std::fs::remove_dir_all(no_pim.join("ofs/ofs-platform-afu-bbb-ofs-2024.1-1")).unwrap();
    let persona = flat_tree(&tmp.path().join("d"), PERSONA_REGION);
    for (what, tree, says) in [
        ("no accelerator", &no_afu, "no AFU tree"),
        ("no FIM", &no_fim, "no FIM source"),
        ("no floorplan", &no_floorplan, "no FIM source"),
        ("no PIM", &no_pim, "no FIM source"),
        ("the persona region", &persona, "reserves the persona region"),
    ] {
        let o = dry(&jobs, None, &["flat", "refused", tree.to_str().unwrap()]);
        assert_eq!(o.status.code(), Some(2), "{what}: {}", String::from_utf8_lossy(&o.stderr));
        assert!(String::from_utf8_lossy(&o.stderr).contains(says), "{what}: {}", String::from_utf8_lossy(&o.stderr));
        assert!(stdout(&o).is_empty(), "{what}: no container command");
    }
    let o = Command::new("bash")
        .arg(script())
        .args(["flat", "refused-real", persona.to_str().unwrap()])
        .env("QUARTUS_JOBS", &jobs)
        .env("QUARTUS_LOGS", tmp.path().join("logs"))
        .env("QUARTUS_LOCK", tmp.path().join("lock"))
        .env_remove("QUARTUS_DRY_RUN")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(!tmp.path().join("lock").exists() && !tmp.path().join("logs/refused-real.log").exists(), "refused before the queue or a log");
}

/// A flat build's real run is a persona's: asked first whether its row can be recorded and given a new log, then it
/// takes the queue, attempts, and records its row from the FIM's own reports (project and revision d5005 under
/// /work/fim), the tree remembering the compile that ran its fitter; a compile that finished synthesis goes on from its
/// fitter, as the FIM's flow runs its modules, and one that did not runs again whole.
#[test]
fn a_flat_build_queues_attempts_and_records_as_a_persona_does() {
    let text = std::fs::read_to_string(script()).unwrap();
    let at = text.find("  flat)").expect("the flat branch");
    let branch = &text[at..];
    let branch = &branch[..branch.find("\n      ;;").expect("its end")];
    assert!(
        in_order(
            branch,
            &[
                "recordable",
                "log=$(new_log \"$job\")",
                "take_queue \"$job\"",
                "since=$(date +%s)",
                "attempts \"$job\" \"$tree\" \"$log\" flat_between",
                "record \"$tree/fim/syn/syn_top/output_files\" d5005 \"$job\" \"$log\"",
                "remember \"$tree\" \"$job\"",
            ]
        ),
        "{branch}"
    );
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("job.log");
    std::fs::write(&log, "Info: Quartus Prime Synthesis was successful. 0 errors, 2116 warnings\nInfo: Run task: Fitter\n").unwrap();
    std::fs::write(tmp.path().join("resume.done"), "quartus_fit\n").unwrap();
    let env = [("LOG_FILE", log.as_path()), ("TREE", tmp.path())];
    let after = |mark: usize| {
        with_functions(
            &env,
            &format!("script=\"$FLAT_SCRIPT\"; expect=\"$FLAT_MODULES\"; log=\"$LOG_FILE\"; tree=\"$TREE\"; mark={mark}; flat_between; [ \"$script\" = \"$FLAT_RESUME_SCRIPT\" ] && echo resumed || echo again; echo \"$expect\""),
        )
    };
    assert_eq!(after(0), "resumed\nFitter,Timing Analyzer,Assembler\n");
    assert!(!tmp.path().join("resume.done").exists(), "a resume starts at the fitter");
    let past = std::fs::read_to_string(&log).unwrap().len();
    assert_eq!(after(past), "again\nSynthesis,Fitter,Timing Analyzer,Assembler\n", "an attempt that did not finish synthesis runs it again");
    let out = tmp.path().join("fim/syn/syn_top/output_files");
    std::fs::create_dir_all(&out).unwrap();
    let set = |p: &Path, text: &str, age: u64| {
        std::fs::write(p, text).unwrap();
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(age);
        std::fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
    };
    set(&out.join("d5005.syn.summary"), "Synthesis Status : Successful\n", 7200);
    set(&out.join("d5005.fit.summary"), "Fitter Status : Successful - Sat Oct 24 23:05:20 2026\nFamily : Stratix 10\n", 60);
    assert_eq!(after(0), "resumed\nTiming Analyzer,Assembler\n", "an attempt that died past the FIM's fit goes on from its timing analysis");
    assert_eq!(std::fs::read_to_string(tmp.path().join("resume.done")).unwrap(), "quartus_fit\n", "the fit marked run");
    set(&out.join("d5005.fit.summary"), "Fitter Status : Failed - Sat Oct 24 23:05:20 2026\n", 60);
    assert_eq!(after(0), "resumed\nFitter,Timing Analyzer,Assembler\n", "a failed fit is run again");
    let resume = with_functions(&[], "echo \"$FLAT_RESUME_SCRIPT\"");
    assert!(
        in_order(
            &resume,
            &[
                "cd /work/fim/syn/syn_top",
                "run quartus_fit --read_settings_files=on --write_settings_files=off d5005 -c d5005",
                "run quartus_sta d5005 -c d5005 --mode=finalize",
                "run quartus_asm --read_settings_files=on --write_settings_files=off d5005 -c d5005",
                "output_files/d5005.sof",
            ]
        ),
        "{resume}"
    );
}

/// A fake `docker` on the PATH for an attempt: `top` reports the same CPU time whenever asked, `stop` kills the attempt's
/// process (its pid in `pid`) and notes it in `stopped`.
fn fake_docker(dir: &std::path::Path) {
    let script = format!(
        "#!/usr/bin/env bash\ncase \"$1\" in\n  top) printf 'TIME\\n00:00:05\\n';;\n  stop) echo \"$2\" >> {d}/stopped; kill \"$(cat {d}/pid)\" 2>/dev/null;;\nesac\n",
        d = dir.display()
    );
    std::fs::write(dir.join("docker"), script).unwrap();
    Command::new("chmod").arg("+x").arg(dir.join("docker")).status().unwrap();
}

#[test]
fn an_attempt_whose_container_sits_without_cpu_or_log_is_stopped_as_hung_and_tried_again() {
    let tmp = tempfile::tempdir().unwrap();
    fake_docker(tmp.path());
    let log = tmp.path().join("job.log");
    std::fs::write(&log, "").unwrap();
    let path = format!("{}:{}", tmp.path().display(), std::env::var("PATH").unwrap());
    let env = [("LOG_FILE", log.as_path()), ("DIR", tmp.path()), ("PATH", std::path::Path::new(&path))];
    let started = std::time::Instant::now();
    let out = with_functions(
        &env,
        "QUARTUS_HANG_SECONDS=2; QUARTUS_HANG_POLL=1; HANG_SECONDS=2; HANG_POLL=1; script=x; \
         run_command() { echo \"bash -c 'echo \\$\\$ > $DIR/pid; exec sleep 60'\"; }; \
         attempt job \"$DIR\" \"$LOG_FILE\"; echo \"status=$status\"; \
         if transient \"$LOG_FILE\" \"$mark\" \"$DIR\" \"$began\"; then echo transient; fi",
    );
    assert!(started.elapsed().as_secs() < 30, "stopped long before the tool would end");
    assert!(out.contains("status=124"), "a hung attempt fails as hung: {out}");
    assert!(out.contains("transient"), "and is tried again: {out}");
    assert_eq!(std::fs::read_to_string(tmp.path().join("stopped")).unwrap().trim(), "logos-q-job", "its container stopped by name");
    assert!(std::fs::read_to_string(&log).unwrap().contains("quartus.sh: job hung: no CPU time and no log for"), "the log says so");
}

/// A fake `docker` on the PATH that answers `top` as Docker does: columns without the process ID refused ("Couldn't find
/// PID field in ps output", exit 1), and otherwise the attempt's tool under its header, its CPU time a second more each
/// time it is asked; `stop` as `fake_docker`'s.
fn computing_docker(dir: &std::path::Path) {
    let script = format!(
        "#!/usr/bin/env bash\ncase \"$1\" in\n  top)\n    case \"$*\" in *pid*) ;; *) echo \"Error response from daemon: Couldn't find PID field in ps output\" >&2; exit 1;; esac\n    n=$(( $(cat {d}/asked 2>/dev/null || echo 0) + 1 )); echo $n > {d}/asked\n    printf 'PID TIME\\n4242 00:%02d:%02d\\n' $((n / 60)) $((n % 60));;\n  stop) echo \"$2\" >> {d}/stopped; kill \"$(cat {d}/pid)\" 2>/dev/null;;\nesac\n",
        d = dir.display()
    );
    std::fs::write(dir.join("docker"), script).unwrap();
    Command::new("chmod").arg("+x").arg(dir.join("docker")).status().unwrap();
}

/// A tool that computes with its log silent — a synthesis writes nothing to the job's log until it ends — is never taken
/// for hung while its container's CPU time grows, read as Docker gives it: `docker top` names the process ID beside the
/// time, or it answers nothing.
#[test]
fn an_attempt_whose_tool_keeps_computing_with_a_silent_log_is_never_taken_for_hung() {
    let tmp = tempfile::tempdir().unwrap();
    computing_docker(tmp.path());
    let log = tmp.path().join("job.log");
    std::fs::write(&log, "").unwrap();
    let path = format!("{}:{}", tmp.path().display(), std::env::var("PATH").unwrap());
    let env = [("LOG_FILE", log.as_path()), ("DIR", tmp.path()), ("PATH", std::path::Path::new(&path))];
    let out = with_functions(
        &env,
        "HANG_SECONDS=2; HANG_POLL=1; script=x; \
         run_command() { echo \"bash -c 'echo \\$\\$ > $DIR/pid; sleep 5'\"; }; \
         attempt job \"$DIR\" \"$LOG_FILE\"; echo \"status=$status\"; echo \"cpu=$(container_cpu logos-q-job)\"",
    );
    assert!(out.contains("status=0"), "a tool whose CPU time grows runs to its end: {out}");
    assert!(!tmp.path().join("stopped").exists(), "its container never stopped: {out}");
    assert!(!std::fs::read_to_string(&log).unwrap().contains("hung"), "{}", std::fs::read_to_string(&log).unwrap());
    let asked: u64 = std::fs::read_to_string(tmp.path().join("asked")).unwrap().trim().parse().unwrap();
    assert!(asked >= 3 && out.contains(&format!("cpu={asked}\n")), "the CPU time read as Docker last gave it: {out}");
}

#[test]
fn an_attempt_that_keeps_writing_its_log_is_never_taken_for_hung() {
    let tmp = tempfile::tempdir().unwrap();
    fake_docker(tmp.path());
    let log = tmp.path().join("job.log");
    std::fs::write(&log, "").unwrap();
    let path = format!("{}:{}", tmp.path().display(), std::env::var("PATH").unwrap());
    let env = [("LOG_FILE", log.as_path()), ("DIR", tmp.path()), ("PATH", std::path::Path::new(&path))];
    let out = with_functions(
        &env,
        "HANG_SECONDS=2; HANG_POLL=1; script=x; \
         run_command() { echo \"bash -c 'echo \\$\\$ > $DIR/pid; for i in 1 2 3 4 5 6 7 8; do echo working \\$i; sleep 0.5; done'\"; }; \
         attempt job \"$DIR\" \"$LOG_FILE\"; echo \"status=$status\"",
    );
    assert!(out.contains("status=0"), "a tool that keeps writing runs to its end: {out}");
    assert!(!tmp.path().join("stopped").exists(), "its container never stopped");
    assert!(std::fs::read_to_string(&log).unwrap().contains("working 8"));
}

#[test]
fn a_job_whose_own_earlier_attempt_ran_the_fitter_names_itself_resumed() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("tree");
    let out = tree.join("build/build/syn/syn_top/output_files");
    std::fs::create_dir_all(&out).unwrap();
    let log = tmp.path().join("job.log");
    std::fs::write(&log, "Info: Quartus Prime Synthesis was successful. 0 errors\nFatal Error: Cannot identify the client\n").unwrap();
    std::fs::write(out.join("iofs_pr_afu.syn.summary"), "Synthesis Status : Successful\n").unwrap();
    let env = [("LOG_FILE", log.as_path()), ("TREE", tree.as_path())];
    let between = |pre: &str| with_functions(&env, &format!("job=self; tree=\"$TREE\"; log=\"$LOG_FILE\"; mark=0; {pre} pr_between; echo \"$expect|${{resumed:-}}\""));
    // Synthesized, not fitted: the next attempt runs the fitter itself and names nothing.
    assert_eq!(between("script=\"$PR_SCRIPT\"; expect=\"$PR_MODULES\";").trim(), "Fitter,Timing Analyzer,Assembler|");
    // Fitted since its synthesis, by this job's first attempt: judged without the fitter, resumed from itself.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(out.join("iofs_pr_afu.fit.summary"), "Fitter Status : Successful\n").unwrap();
    assert_eq!(between("script=\"$PR_SCRIPT\"; expect=\"$PR_MODULES\";").trim(), "Timing Analyzer,Assembler|self");
    // Already going on from its fitter when an attempt finished the fit: the next is judged without it, from itself.
    std::fs::write(tree.join("resume.done"), "quartus_fit\n").unwrap();
    assert_eq!(between("script=\"$RESUME_SCRIPT\"; expect=\"$RESUMED_MODULES\";").trim(), "Timing Analyzer,Assembler|self");
    // A resume that began after another compile's fit keeps naming that compile.
    assert_eq!(between("script=\"$RESUME_SCRIPT\"; expect='Timing Analyzer,Assembler'; resumed=other;").trim(), "Timing Analyzer,Assembler|other");
    let text = std::fs::read_to_string(script()).unwrap();
    let branch = &text[text.find("  pr|resume)").expect("the persona branch")..];
    let remembered = branch.find("[ \"${resumed:-}\" = \"$job\" ] && remember \"$tree\" \"$job\"").expect("a job that fitted in an earlier attempt remembered too");
    assert!(branch.find("record \"$tree/build").unwrap() < remembered, "once recorded");
}

/// A flat build's synthesis report of one partition, `root_partition`, as Quartus writes it: its estimate of the ALMs it
/// needs, its DSP blocks and its block memory bits (every memory's), and its RAM summary, each memory `(path, depth,
/// width)` an M20K block of simple dual ports.
fn synthesis_with_rams(dir: &Path, alms: u64, dsp: u64, rams: &[(&str, u64, u64)]) {
    let bits: u64 = rams.iter().map(|&(_, d, w)| d * w).sum();
    let rows: String = rams
        .iter()
        .map(|&(path, d, w)| format!("; {path} ; M20K block ; Simple Dual Port ; {d} ; {w} ; {d} ; {w} ; {} ; None ;\n", d * w))
        .collect();
    let report = format!(
        "+----+\n; Synthesis Resource Usage Summary for Partition \"root_partition\" ;\n+----+\n; Resource ; Usage ;\n+----+\n\
         ; Estimate of Logic utilization (ALMs needed) ; {alms} ;\n; Total block memory bits ; {bits} ;\n; Total DSP Blocks ; {dsp} ;\n+----+\n\n\n\
         +----+\n; Synthesis RAM Summary for Partition \"root_partition\" ;\n+----+\n\
         ; Name ; Type ; Mode ; Port A Depth ; Port A Width ; Port B Depth ; Port B Width ; Size ; MIF ;\n+----+\n{rows}+----+\n\n\n"
    );
    std::fs::create_dir_all(dir.join("output_files")).unwrap();
    std::fs::write(dir.join("output_files/d5005.syn.rpt"), report).unwrap();
}

/// A stand-in for `quartus_record` in `dir`: its arguments, one a line, left in `args`, and the log excerpt it was
/// given (its fourth argument) in `last`.
fn arg_recorder(dir: &Path) -> PathBuf {
    let r = dir.join("recorder");
    std::fs::write(&r, "#!/usr/bin/env bash\nd=$(dirname \"$0\")\nfor a in \"$@\"; do echo \"$a\"; done > \"$d/args\"\ncp \"$4\" \"$d/last\"\n").unwrap();
    Command::new("chmod").arg("+x").arg(&r).status().unwrap();
    r
}

/// A fake `docker` in `dir/bin` for a real run: every call noted in `dir/calls`, no container listed by `ps`, a start-up
/// probe's `run` passing; a job's own run, when `synthesis` (from, to) names a report at `from`, leaves it at `to` and says
/// its synthesis was successful, and exits as `dir/exit` says (0 without it).
fn queue_docker(dir: &Path, synthesis: Option<(&Path, &Path)>) {
    let synthesize = synthesis.map_or(":".to_string(), |(from, to)| {
        format!(
            "if [ -f {f} ]; then cp {f} {t}; echo 'Info: Quartus Prime Synthesis was successful. 0 errors, 0 warnings'; fi",
            f = from.display(),
            t = to.display()
        )
    });
    let script = format!(
        "#!/usr/bin/env bash\necho \"$*\" >> {d}/calls\ncase \"$1\" in\n  run) case \"$*\" in *-probe*) ;; *) {synthesize}; [ -f {d}/exit ] && exit \"$(cat {d}/exit)\";; esac;;\nesac\nexit 0\n",
        d = dir.display()
    );
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::write(dir.join("bin/docker"), script).unwrap();
    Command::new("chmod").arg("+x").arg(dir.join("bin/docker")).status().unwrap();
}

/// A real run of the queue script from `tmp`, with its jobs, logs, lock and ledger under it, `arg_recorder` as its
/// recorder, `queue_docker`'s Docker first on the PATH, and `budget` as its d5005_budget (none: the script's own, run by
/// cargo in the repository whatever directory the script is run from).
fn queue_run(tmp: &Path, args: &[&str], budget: Option<&Path>) -> Output {
    queue_command(tmp, args, budget).output().expect("bash runs the queue script")
}

/// `queue_run`'s command, unrun, given no floorplan.
fn queue_command(tmp: &Path, args: &[&str], budget: Option<&Path>) -> Command {
    let mut c = Command::new("bash");
    c.arg(script())
        .args(args)
        .current_dir(tmp)
        .env_remove("QUARTUS_DRY_RUN")
        .env("QUARTUS_JOBS", tmp.join("jobs"))
        .env("QUARTUS_LOGS", tmp.join("logs"))
        .env("QUARTUS_LOCK", tmp.join("lock"))
        .env("QUARTUS_LEDGER", tmp.join("ledger.json"))
        .env("QUARTUS_RECORD", arg_recorder(tmp))
        .env("Q234", "/opt/q234-test")
        .env("PATH", format!("{}:{}", tmp.join("bin").display(), std::env::var("PATH").unwrap()))
        .env_remove("QUARTUS_FLOORPLAN");
    match budget {
        Some(b) => c.env("QUARTUS_BUDGET", b),
        None => c.env_remove("QUARTUS_BUDGET"),
    };
    c
}

/// A persona's build and a persona's resume that pass exit 0, as a flat image's resume does: the job ran, its row is
/// recorded from its last attempt's exit status (0) and its tree remembered, whether or not the job resumed itself.
#[test]
fn a_persona_build_or_resume_that_passes_exits_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("tree");
    std::fs::create_dir_all(tree.join("afu/hw/rtl")).unwrap();
    std::fs::write(tree.join("afu/hw/rtl/sources.txt"), "logos.v\n").unwrap();
    queue_docker(tmp.path(), None);
    let built = queue_run(tmp.path(), &["pr", "persona-build", tree.to_str().unwrap()], None);
    assert_eq!(built.status.code(), Some(0), "{}{}", stdout(&built), String::from_utf8_lossy(&built.stderr));
    assert!(std::fs::read_to_string(tmp.path().join("args")).unwrap().starts_with("--exit\n0\n"), "its row recorded as passed");
    assert_eq!(std::fs::read_to_string(tree.join("compiled.by")).unwrap(), "persona-build\n", "its fitter remembered");
    std::fs::create_dir_all(tree.join("build/build/syn/syn_top/qdb")).unwrap();
    let resumed = queue_run(tmp.path(), &["resume", "persona-resume", tree.to_str().unwrap()], None);
    assert_eq!(resumed.status.code(), Some(0), "{}{}", stdout(&resumed), String::from_utf8_lossy(&resumed.stderr));
    let args = std::fs::read_to_string(tmp.path().join("args")).unwrap();
    assert!(args.starts_with("--exit\n0\n") && args.contains("--resumed\npersona-build\n"), "{args}");
    assert_eq!(std::fs::read_to_string(tree.join("compiled.by")).unwrap(), "persona-resume\n", "the resume's fitter remembered");
}

/// A flat image's fit is placed by the floorplan its resume is given (`QUARTUS_FLOORPLAN`) and by no other: the Tcl
/// copied beside the FIM's project as `logos_floorplan.tcl` and sourced once by `d5005.qsf`, the project's own settings
/// before it untouched, and named among the row's inputs; a resume given none leaves no floorplan an earlier resume was
/// given. The regions are written once the queue is taken, so a resume waiting on the queue never moves those of a fit
/// still running in the same tree. A floorplan is refused, before a log or the queue and with the project unchanged,
/// where no fit would be placed by it: a floorplan that is no file, a persona's (placed in the FIM's region), an image
/// fitted since its synthesis (its fit done), a tree with no FIM project to source it, a flat build (its work tree made
/// inside its container) and a synthesis.
#[test]
fn a_flat_images_fit_is_placed_by_the_floorplan_its_job_is_given_and_by_no_other() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = flat_tree(tmp.path(), WHOLE_DEVICE);
    let syn_top = tree.join("fim/syn/syn_top");
    std::fs::create_dir_all(syn_top.join("qdb")).unwrap();
    synthesis_with_rams(&syn_top, 1_000, 0, &[("afu_top|afu|logos_engine|store|ram", 57_344, 512)]);
    let settings = "set_global_assignment -name SOURCE_TCL_SCRIPT_FILE ../setup/config_env.tcl\nset_global_assignment -name SEED 5\n";
    let qsf = syn_top.join("d5005.qsf");
    std::fs::write(&qsf, settings).unwrap();
    let plan = tmp.path().join("regions.tcl");
    let tcl = "set_instance_assignment -name PLACE_REGION \"X48 Y24 X79 Y47\" -to afu_top|port_gasket|pr_slot|afu_main|ofs_plat_afu|shell|ch[3].channel\n";
    std::fs::write(&plan, tcl).unwrap();
    let placed_at = syn_top.join("logos_floorplan.tcl");
    let source = "set_global_assignment -name SOURCE_TCL_SCRIPT_FILE logos_floorplan.tcl";
    queue_docker(tmp.path(), None);
    let holds = tmp.path().join("budget");
    std::fs::write(&holds, "#!/usr/bin/env bash\necho \"stand-in holds $1\"\n").unwrap();
    Command::new("chmod").arg("+x").arg(&holds).status().unwrap();
    let run = |args: &[&str], floorplan: Option<&Path>| {
        let mut c = queue_command(tmp.path(), args, Some(&holds));
        if let Some(f) = floorplan {
            c.env("QUARTUS_FLOORPLAN", f);
        }
        c.output().expect("bash runs the queue script")
    };
    let said = |o: &Output| format!("{}{}", stdout(o), String::from_utf8_lossy(&o.stderr));
    let inputs = || std::fs::read_to_string(tmp.path().join("args")).unwrap();
    let flat = tree.to_str().unwrap();

    let lock = tmp.path().join("lock");
    let release = tmp.path().join("release");
    let mut holder = Command::new("bash")
        .args(["-c", "exec 9>\"$1\"; flock 9; : > \"$1.taken\"; until [ -e \"$2\" ]; do sleep 0.05; done", "-"])
        .args([&lock, &release])
        .spawn()
        .expect("bash holds the queue");
    let patience = std::time::Instant::now();
    while !tmp.path().join("lock.taken").exists() {
        assert!(patience.elapsed().as_secs() < 60, "the queue was never held");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let waited = tmp.path().join("waiting.out");
    let mut c = queue_command(tmp.path(), &["resume", "h7c-wait", flat], Some(&holds));
    c.env("QUARTUS_FLOORPLAN", &plan).stdout(std::fs::File::create(&waited).unwrap()).stderr(std::process::Stdio::null());
    let mut waiting = c.spawn().expect("bash runs the queue script");
    while !std::fs::read_to_string(&waited).unwrap().contains("quartus.sh: h7c-wait waits for the queue") {
        assert!(waiting.try_wait().unwrap().is_none(), "it ended before it waited: {}", std::fs::read_to_string(&waited).unwrap());
        assert!(patience.elapsed().as_secs() < 120, "it never waited for the queue");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(std::fs::read_to_string(&qsf).unwrap(), settings, "a resume waiting on the queue leaves the project as the job holding it has it");
    assert!(!placed_at.exists());
    std::fs::write(&release, "").unwrap();
    assert!(holder.wait().unwrap().success());
    assert!(waiting.wait().unwrap().success(), "{}", std::fs::read_to_string(&waited).unwrap());
    assert_eq!(std::fs::read_to_string(&qsf).unwrap(), format!("{settings}{source}\n"), "placed once it took the queue");

    for job in ["h7c-fit", "h7c-fit2"] {
        let placed = run(&["resume", job, flat], Some(&plan));
        assert!(placed.status.success(), "{}", said(&placed));
        assert_eq!(std::fs::read_to_string(&qsf).unwrap(), format!("{settings}{source}\n"), "{job}: the project's settings, then the floorplan sourced once");
        assert_eq!(std::fs::read_to_string(&placed_at).unwrap(), tcl, "{job}");
        assert!(inputs().lines().any(|l| Path::new(l) == placed_at), "{job}: the row names its floorplan: {}", inputs());
    }
    let free = run(&["resume", "h7c-free", flat], None);
    assert!(free.status.success(), "{}", said(&free));
    assert_eq!(std::fs::read_to_string(&qsf).unwrap(), settings, "a resume given no floorplan is placed by none");
    assert!(!placed_at.exists() && !inputs().contains("logos_floorplan.tcl"), "{}", inputs());

    let text = std::fs::read_to_string(script()).unwrap();
    let resume = &text[text.find("resume_flat() {").unwrap()..];
    let resume = &resume[..resume.find("\n}\n").unwrap()];
    assert!(in_order(resume, &["place_flat \"$tree\"", "attempts \"$job\""]), "placed before its first attempt:\n{resume}");
    let branch = &text[text.find("  pr|resume)").unwrap()..text.find("    flat)").unwrap()];
    assert!(in_order(branch, &["take_queue \"$job\"", "resume_flat \"$job\" \"$tree\""]), "and after the queue is taken:\n{branch}");

    let persona = tmp.path().join("persona");
    std::fs::create_dir_all(persona.join("afu/hw/rtl")).unwrap();
    std::fs::write(persona.join("afu/hw/rtl/sources.txt"), "logos.v\n").unwrap();
    std::fs::create_dir_all(persona.join("build/build/syn/syn_top/qdb")).unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    let fitted = tmp.path().join("fitted");
    let fitted_tree = flat_tree(&fitted, WHOLE_DEVICE);
    let fitted_top = fitted_tree.join("fim/syn/syn_top");
    std::fs::create_dir_all(fitted_top.join("qdb")).unwrap();
    std::fs::create_dir_all(fitted_top.join("output_files")).unwrap();
    std::fs::write(fitted_top.join("d5005.qsf"), settings).unwrap();
    std::fs::write(fitted_top.join("output_files/d5005.syn.summary"), "Synthesis Status : Successful\n").unwrap();
    std::fs::File::options().write(true).open(fitted_top.join("output_files/d5005.syn.summary")).unwrap().set_modified(old).unwrap();
    std::fs::write(fitted_top.join("output_files/d5005.fit.summary"), "Fitter Status : Successful - Mon Oct 26 07:00:00 2026\n").unwrap();
    let bare = flat_tree(&tmp.path().join("bare"), WHOLE_DEVICE);
    std::fs::create_dir_all(bare.join("fim/syn/syn_top/qdb")).unwrap();
    let v = design(tmp.path());
    let absent = tmp.path().join("absent.tcl");
    for (job, args, floorplan, why) in [
        ("h7c-absent", vec!["resume", "h7c-absent", flat], &absent, "absent.tcl"),
        ("p-resume", vec!["resume", "p-resume", persona.to_str().unwrap()], &plan, "persona"),
        ("p-build", vec!["pr", "p-build", persona.to_str().unwrap()], &plan, "persona"),
        ("h7c-done", vec!["resume", "h7c-done", fitted_tree.to_str().unwrap()], &plan, "fitted since its synthesis"),
        ("h7c-bare", vec!["resume", "h7c-bare", bare.to_str().unwrap()], &plan, "no FIM project"),
        ("h7c-flat", vec!["flat", "h7c-flat", flat], &plan, "flat-syn"),
        ("h7c-syn", vec!["flat-syn", "h7c-syn", flat], &plan, "places nothing"),
        ("unit-syn", vec!["syn", "unit-syn", v.to_str().unwrap(), "unit"], &plan, "places nothing"),
    ] {
        let o = run(&args, Some(floorplan));
        assert_eq!(o.status.code(), Some(2), "{job} refused: {}", said(&o));
        assert!(String::from_utf8_lossy(&o.stderr).contains(why), "{job} names why ({why}): {}", said(&o));
        assert!(!tmp.path().join(format!("logs/{job}.log")).exists(), "{job}: refused before a log");
        assert!(!tmp.path().join("jobs").join(job).exists(), "{job}: nothing written");
    }
    assert_eq!(std::fs::read_to_string(&qsf).unwrap(), settings, "no refused job touched the project");
    assert_eq!(std::fs::read_to_string(fitted_top.join("d5005.qsf")).unwrap(), settings);
    assert!(!placed_at.exists() && !fitted_top.join("logos_floorplan.tcl").exists());
    assert!(!bare.join("fim/syn/syn_top/d5005.qsf").exists() && !bare.join("fim/syn/syn_top/logos_floorplan.tcl").exists());
}

/// A synthesized flat image is held to the device before its fitter by the M20K blocks the fitter gives its memories
/// (`d5005_budget`, `codegen_sva::preflight::synthesized_holds`), not by its block memory bits: one store of 458,752 words
/// of 512 bits is 234,881,024 bits, within the 240,046,080 of the device's 11,721 M20Ks, so the bits pass it, but its
/// blocks stack 524,288 words deep, thirteen 40-bit blocks wide by 1,024: 13,312 M20K blocks, 113% of the device. Its
/// resume is refused before it queues (5), the budget's verdict printed; no log is begun and no container run, so the
/// job's name stays free for the next attempt. A design the device holds goes on into the queue, the budget's verdict
/// at the head of its log, and a tree whose fit succeeded since its synthesis goes on from its timing analysis, unheld.
#[test]
fn a_flat_resume_the_device_cannot_hold_by_its_m20k_blocks_is_refused_before_it_queues() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = flat_tree(tmp.path(), WHOLE_DEVICE);
    let syn_top = tree.join("fim/syn/syn_top");
    std::fs::create_dir_all(syn_top.join("qdb")).unwrap();
    synthesis_with_rams(&syn_top, 1_000, 0, &[("afu_top|afu|logos_engine|store|ram", 458_752, 512)]);
    let bits = with_functions(&[("DIR", syn_top.as_path())], "s=0; ( cd \"$DIR\" && eval \"$FLAT_BUDGET\" ) || s=$?; echo \"status=$s\"");
    assert!(bits.contains("status=0") && bits.contains("234881024 block memory bits (97%)"), "the bits alone pass it: {bits}");
    queue_docker(tmp.path(), None);
    let calls = tmp.path().join("calls");
    let lock = tmp.path().join("lock");
    let log = tmp.path().join("logs/h8b-fit.log");

    let refused = queue_run(tmp.path(), &["resume", "h8b-fit", tree.to_str().unwrap()], None);
    let said = stdout(&refused);
    assert_eq!(refused.status.code(), Some(5), "{said}{}", String::from_utf8_lossy(&refused.stderr));
    assert!(said.contains("d5005_budget: the design needs 1000 ALMs (0%), 0 DSP blocks (0%) and 13312 M20K blocks (113%)\n"), "{said}");
    assert!(said.contains("d5005_budget: 13312 M20K blocks, where the device holds 11721: no fit\n"), "{said}");
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("quartus.sh: h8b-fit is not fitted: the device cannot hold its synthesis"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(!lock.exists() && !log.exists() && !calls.exists(), "refused before the queue, a log or a container");

    synthesis_with_rams(&syn_top, 1_000, 0, &[("afu_top|afu|logos_engine|store|ram", 57_344, 512)]);
    let held = queue_run(tmp.path(), &["resume", "h8b-fit", tree.to_str().unwrap()], None);
    assert!(held.status.success(), "{}{}", stdout(&held), String::from_utf8_lossy(&held.stderr));
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(text.starts_with("d5005_budget: the design needs 1000 ALMs (0%), 0 DSP blocks (0%) and 1664 M20K blocks (14%)\n"), "{text}");
    assert!(std::fs::read_to_string(tmp.path().join("lock.holder")).unwrap().starts_with("h8b-fit "), "it took the queue");
    let ran = std::fs::read_to_string(&calls).unwrap();
    assert!(ran.lines().any(|c| c.starts_with("run ") && c.contains("--name logos-q-h8b-fit ")), "its fit ran: {ran}");
    let args = std::fs::read_to_string(tmp.path().join("args")).unwrap();
    assert!(args.starts_with("--exit\n0\n") && args.contains("--expect\nFitter,Timing Analyzer,Assembler\n"), "{args}");

    synthesis_with_rams(&syn_top, 1_000, 0, &[("afu_top|afu|logos_engine|store|ram", 458_752, 512)]);
    let out = syn_top.join("output_files");
    let set = |p: &Path, text: &str, age: u64| {
        std::fs::write(p, text).unwrap();
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(age);
        std::fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
    };
    set(&out.join("d5005.syn.summary"), "Synthesis Status : Successful\n", 7200);
    set(&out.join("d5005.fit.summary"), "Fitter Status : Successful - Sun Oct 25 23:05:20 2026\n", 60);
    let timed = queue_run(tmp.path(), &["resume", "h8b-ta", tree.to_str().unwrap()], None);
    assert!(timed.status.success(), "a fitted tree is never held again: {}{}", stdout(&timed), String::from_utf8_lossy(&timed.stderr));
    assert!(!stdout(&timed).contains("d5005_budget") && !std::fs::read_to_string(tmp.path().join("logs/h8b-ta.log")).unwrap().contains("d5005_budget"));
    let args = std::fs::read_to_string(tmp.path().join("args")).unwrap();
    assert!(args.contains("--expect\nTiming Analyzer,Assembler\n"), "{args}");
}

/// A flat synthesis ends held to the device by its M20K blocks too, on the host once its synthesis is done: a synthesis
/// whose bits the in-container budget passes but whose memories the device cannot hold is recorded as refused (exit 5),
/// the budget's verdict the end of its row's log; one the device holds is recorded as passed, its verdict in the log; a
/// synthesis that failed is recorded as it failed, never held.
#[test]
fn a_flat_synthesis_the_device_cannot_hold_by_its_m20k_blocks_is_recorded_as_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = flat_tree(tmp.path(), WHOLE_DEVICE);
    let prepared = tmp.path().join("prepared");
    let report = tree.join("fim/syn/syn_top/output_files/d5005.syn.rpt");
    std::fs::create_dir_all(report.parent().unwrap()).unwrap();
    queue_docker(tmp.path(), Some((&prepared.join("output_files/d5005.syn.rpt"), &report)));
    for (job, depth, exit, verdict) in [
        ("h8b-syn", 458_752u64, "5", "d5005_budget: 13312 M20K blocks, where the device holds 11721: no fit\n"),
        ("h8b-syn2", 57_344, "0", "d5005_budget: the design needs 1000 ALMs (0%), 0 DSP blocks (0%) and 1664 M20K blocks (14%)\n"),
    ] {
        synthesis_with_rams(&prepared, 1_000, 0, &[("afu_top|afu|logos_engine|store|ram", depth, 512)]);
        let o = queue_run(tmp.path(), &["flat-syn", job, tree.to_str().unwrap()], None);
        assert!(o.status.success(), "{job}: {}{}", stdout(&o), String::from_utf8_lossy(&o.stderr));
        let args: Vec<String> = std::fs::read_to_string(tmp.path().join("args")).unwrap().lines().map(str::to_string).collect();
        assert_eq!(args[0..2], ["--exit".to_string(), exit.to_string()], "{job}: {args:?}");
        let last = std::fs::read_to_string(tmp.path().join("last")).unwrap();
        assert!(last.contains("Quartus Prime Synthesis was successful") && last.ends_with(verdict), "{job}: {last}");
        assert!(stdout(&o).contains(verdict), "{job}: the verdict among the log's last lines: {}", stdout(&o));
    }
    std::fs::remove_file(prepared.join("output_files/d5005.syn.rpt")).unwrap();
    std::fs::remove_file(&report).unwrap();
    std::fs::write(tmp.path().join("exit"), "3").unwrap();
    let o = queue_run(tmp.path(), &["flat-syn", "h8b-syn3", tree.to_str().unwrap()], None);
    assert!(o.status.success(), "{}{}", stdout(&o), String::from_utf8_lossy(&o.stderr));
    let args: Vec<String> = std::fs::read_to_string(tmp.path().join("args")).unwrap().lines().map(str::to_string).collect();
    assert_eq!(args[0..2], ["--exit".to_string(), "3".to_string()], "a failed synthesis keeps its status: {args:?}");
    assert!(!std::fs::read_to_string(tmp.path().join("last")).unwrap().contains("d5005_budget"), "and is never held");
}

/// The d5005_budget a flat job is given (QUARTUS_BUDGET) is the one that holds it, given the tree's synthesis report: its
/// refusal (5) refuses the resume before it queues, its verdict printed, and a budget that fails any other way refuses it
/// too (2), the synthesis said to be unheld.
#[test]
fn a_flat_job_is_held_by_the_budget_it_is_given_on_its_own_report() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = flat_tree(tmp.path(), WHOLE_DEVICE);
    let syn_top = tree.join("fim/syn/syn_top");
    std::fs::create_dir_all(syn_top.join("qdb")).unwrap();
    synthesis_with_rams(&syn_top, 1_000, 0, &[("afu_top|afu|logos_engine|store|ram", 57_344, 512)]);
    queue_docker(tmp.path(), None);
    let stand_in = tmp.path().join("budget");
    for (exit, code, says) in [("5", 5, "is not fitted: the device cannot hold its synthesis"), ("3", 2, "could not be held to the device: d5005_budget exited 3")] {
        std::fs::write(&stand_in, format!("#!/usr/bin/env bash\necho \"stand-in holds $1\"\nexit {exit}\n")).unwrap();
        Command::new("chmod").arg("+x").arg(&stand_in).status().unwrap();
        let o = queue_run(tmp.path(), &["resume", "stood-in", tree.to_str().unwrap()], Some(&stand_in));
        assert_eq!(o.status.code(), Some(code), "{exit}: {}{}", stdout(&o), String::from_utf8_lossy(&o.stderr));
        assert!(stdout(&o).contains(&format!("stand-in holds {}\n", syn_top.join("output_files/d5005.syn.rpt").display())), "{exit}: {}", stdout(&o));
        assert!(String::from_utf8_lossy(&o.stderr).contains(says), "{exit}: {}", String::from_utf8_lossy(&o.stderr));
    }
    assert!(!tmp.path().join("lock").exists() && !tmp.path().join("logs/stood-in.log").exists() && !tmp.path().join("calls").exists());
}

/// A flat job that would hold its synthesis to the device is refused before it queues or begins a log when it could not:
/// a d5005_budget named that is no executable, or none named and no cargo on the PATH to run the script's own.
#[test]
fn a_flat_job_whose_synthesis_could_not_be_held_to_the_device_is_refused_before_it_queues() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = flat_tree(tmp.path(), WHOLE_DEVICE);
    let syn_top = tree.join("fim/syn/syn_top");
    std::fs::create_dir_all(syn_top.join("qdb")).unwrap();
    synthesis_with_rams(&syn_top, 1_000, 0, &[("afu_top|afu|logos_engine|store|ram", 57_344, 512)]);
    queue_docker(tmp.path(), None);
    let missing = tmp.path().join("no-such-budget");
    for args in [["resume", "unheld"], ["flat-syn", "unheld"]] {
        let o = queue_run(tmp.path(), &[args[0], args[1], tree.to_str().unwrap()], Some(&missing));
        assert_eq!(o.status.code(), Some(2), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
        assert!(String::from_utf8_lossy(&o.stderr).contains("QUARTUS_BUDGET is no executable"), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
        let bare = Command::new("bash")
            .arg(script())
            .args([args[0], args[1], tree.to_str().unwrap()])
            .env_remove("QUARTUS_DRY_RUN")
            .env_remove("QUARTUS_BUDGET")
            .env("QUARTUS_RECORD", arg_recorder(tmp.path()))
            .env("QUARTUS_LOGS", tmp.path().join("logs"))
            .env("QUARTUS_LOCK", tmp.path().join("lock"))
            .env("PATH", format!("{}:/usr/bin:/bin", tmp.path().join("bin").display()))
            .output()
            .unwrap();
        assert_eq!(bare.status.code(), Some(2), "{args:?}: {}", String::from_utf8_lossy(&bare.stderr));
        assert!(
            String::from_utf8_lossy(&bare.stderr).contains("no cargo on the PATH and no QUARTUS_BUDGET"),
            "{args:?}: {}",
            String::from_utf8_lossy(&bare.stderr)
        );
    }
    assert!(!tmp.path().join("lock").exists() && !tmp.path().join("logs/unheld.log").exists() && !tmp.path().join("calls").exists());
}

/// A fitted image as the flow leaves it under `syn_top`: its synthesis summary, then its fit's, newer.
fn fitted_image(syn_top: &Path, revision: &str) {
    let out = syn_top.join("output_files");
    std::fs::create_dir_all(&out).unwrap();
    let syn = out.join(format!("{revision}.syn.summary"));
    std::fs::write(&syn, "Synthesis Status : Successful\n").unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    std::fs::File::options().write(true).open(&syn).unwrap().set_modified(old).unwrap();
    std::fs::write(out.join(format!("{revision}.fit.summary")), "Fitter Status : Successful - Thu Oct 29 04:00:00 2026\n").unwrap();
}

/// An analysis (`sta <job> <tree> <tcl> [ARG ...]`) runs a Tcl script in the Timing Analyzer (`quartus_sta -t`) on a
/// fitted image's timing netlist, in the job's container with the tree as /work: a flat image's FIM project (revision
/// d5005) or a persona's (iofs_pr_afu), the script given the project, its revision, the job's reports directory under
/// the tree (`sta/<job>`) and the job's own arguments, in that order. It runs nothing else: no synthesis, fit, assembly,
/// IP generation, setup or sign-off timing analysis, and nothing removed. An image with no fit since its synthesis, a
/// script that is no file, a floorplan (an analysis places nothing) and a job whose reports directory exists are
/// refused, and a dry run writes nothing into the tree.
#[test]
fn an_analysis_runs_its_script_on_a_fitted_images_timing_netlist_and_nothing_else() {
    let tmp = tempfile::tempdir().unwrap();
    let tcl = tmp.path().join("unit-paths.tcl");
    std::fs::write(&tcl, "puts $quartus(args)\n").unwrap();
    let jobs = tmp.path().join("jobs");
    let tree = flat_tree(tmp.path(), WHOLE_DEVICE);
    let syn_top = tree.join("fim/syn/syn_top");
    std::fs::create_dir_all(syn_top.join("qdb")).unwrap();
    let refused = dry(&jobs, None, &["sta", "h9e-units", tree.to_str().unwrap(), tcl.to_str().unwrap()]);
    assert_eq!(refused.status.code(), Some(2), "a synthesized image has no timing netlist of its fit");
    assert!(String::from_utf8_lossy(&refused.stderr).contains("no fitted image"), "{}", String::from_utf8_lossy(&refused.stderr));
    fitted_image(&syn_top, "d5005");
    let o = dry(&jobs, None, &["sta", "h9e-units", tree.to_str().unwrap(), tcl.to_str().unwrap(), "5.0", "units"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let cmd = stdout(&o);
    assert!(carries(&cmd, "--name logos-q-h9e-units"), "{cmd}");
    assert!(carries(&cmd, &format!("{}:/work", tree.display())), "the tree is /work: {cmd}");
    assert!(
        in_order(&cmd, &["cd /work/fim/syn/syn_top", "quartus_sta -t /work/sta/h9e-units/analysis.tcl d5005 d5005 /work/sta/h9e-units 5.0 units"]),
        "the script on the flat image's project, its revision, its reports and its arguments: {cmd}"
    );
    let body = &cmd[cmd.find(" -c ").expect("the container's script")..];
    for absent in ["quartus_syn", "quartus_fit", "quartus_asm", "quartus_ipgenerate", "build_top.sh", "rm ", "--mode=finalize", "afu_synth"] {
        assert!(!body.contains(absent), "{absent} in {body}");
    }
    assert!(!tree.join("sta").exists(), "a dry run writes nothing into the tree");

    let persona = tmp.path().join("persona");
    std::fs::create_dir_all(persona.join("afu/hw/rtl")).unwrap();
    std::fs::write(persona.join("afu/hw/rtl/sources.txt"), "logos.v\n").unwrap();
    let persona_top = persona.join("build/build/syn/syn_top");
    std::fs::create_dir_all(persona_top.join("qdb")).unwrap();
    let refused = dry(&jobs, None, &["sta", "h3b-units", persona.to_str().unwrap(), tcl.to_str().unwrap()]);
    assert_eq!(refused.status.code(), Some(2), "a persona not fitted since its synthesis");
    fitted_image(&persona_top, "iofs_pr_afu");
    let o = dry(&jobs, None, &["sta", "h3b-units", persona.to_str().unwrap(), tcl.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let cmd = stdout(&o);
    assert!(
        in_order(&cmd, &["cd /work/build/build/syn/syn_top", "quartus_sta -t /work/sta/h3b-units/analysis.tcl d5005 iofs_pr_afu /work/sta/h3b-units"]),
        "the script on the persona's project: {cmd}"
    );

    let refused = dry(&jobs, None, &["sta", "h9e-units", tree.to_str().unwrap(), tmp.path().join("none.tcl").to_str().unwrap()]);
    assert_eq!(refused.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("no Tcl script at"), "{}", String::from_utf8_lossy(&refused.stderr));
    let refused = dry(&jobs, None, &["sta", "h9e-units", tree.to_str().unwrap()]);
    assert_eq!(refused.status.code(), Some(2), "a script is named");
    let placed = Command::new("bash")
        .arg(script())
        .args(["sta", "h9e-placed", tree.to_str().unwrap(), tcl.to_str().unwrap()])
        .env("QUARTUS_DRY_RUN", "1")
        .env("QUARTUS_JOBS", &jobs)
        .env("QUARTUS_LOGS", jobs.with_extension("logs"))
        .env("QUARTUS_FLOORPLAN", &tcl)
        .output()
        .unwrap();
    assert_eq!(placed.status.code(), Some(2), "an analysis places nothing");
    assert!(String::from_utf8_lossy(&placed.stderr).contains("an analysis places nothing"), "{}", String::from_utf8_lossy(&placed.stderr));
    std::fs::create_dir_all(tree.join("sta/h9e-done")).unwrap();
    let refused = dry(&jobs, None, &["sta", "h9e-done", tree.to_str().unwrap(), tcl.to_str().unwrap()]);
    assert_eq!(refused.status.code(), Some(2), "a job's reports are written once");
    assert!(String::from_utf8_lossy(&refused.stderr).contains("has reports at"), "{}", String::from_utf8_lossy(&refused.stderr));
}

/// A real analysis takes the queue, a log of its own and its attempts as a build does, its script copied into its
/// reports directory before it queues (a script edited while the job waits never reaches it), and exits with its last
/// attempt's status; it records no row of the build ledger, for it builds nothing and leaves the fit's reports as they
/// were.
#[test]
fn an_analysis_queues_keeps_its_script_and_records_no_row() {
    let tmp = tempfile::tempdir().unwrap();
    let tcl = tmp.path().join("unit-paths.tcl");
    std::fs::write(&tcl, "report_timing -setup -npaths 1\n").unwrap();
    let tree = flat_tree(tmp.path(), WHOLE_DEVICE);
    let syn_top = tree.join("fim/syn/syn_top");
    std::fs::create_dir_all(syn_top.join("qdb")).unwrap();
    fitted_image(&syn_top, "d5005");
    let fit_summary = std::fs::read(syn_top.join("output_files/d5005.fit.summary")).unwrap();
    queue_docker(tmp.path(), None);
    let o = queue_run(tmp.path(), &["sta", "h9e-units", tree.to_str().unwrap(), tcl.to_str().unwrap()], None);
    assert_eq!(o.status.code(), Some(0), "{}{}", stdout(&o), String::from_utf8_lossy(&o.stderr));
    assert_eq!(std::fs::read(tree.join("sta/h9e-units/analysis.tcl")).unwrap(), std::fs::read(&tcl).unwrap(), "the script kept with its reports");
    assert!(tmp.path().join("logs/h9e-units.log").exists(), "a log of its own");
    let calls = std::fs::read_to_string(tmp.path().join("calls")).unwrap();
    assert!(calls.contains("logos-q-h9e-units") && calls.contains("quartus_sta -t /work/sta/h9e-units/analysis.tcl"), "{calls}");
    assert!(!tmp.path().join("args").exists() && !tmp.path().join("ledger.json").exists(), "no row recorded");
    assert_eq!(std::fs::read(syn_top.join("output_files/d5005.fit.summary")).unwrap(), fit_summary);
    std::fs::write(tmp.path().join("exit"), "3\n").unwrap();
    let failed = queue_run(tmp.path(), &["sta", "h9e-failed", tree.to_str().unwrap(), tcl.to_str().unwrap()], None);
    assert_eq!(failed.status.code(), Some(3), "the attempt's status: {}", String::from_utf8_lossy(&failed.stderr));
    let again = queue_run(tmp.path(), &["sta", "h9e-units", tree.to_str().unwrap(), tcl.to_str().unwrap()], None);
    assert_eq!(again.status.code(), Some(2), "a job name is used once");
    let text = std::fs::read_to_string(script()).unwrap();
    let branch = &text[text.find("    sta)").expect("the sta branch")..];
    let branch = &branch[..branch.find(";;").unwrap()];
    assert!(in_order(branch, &["cp \"$tcl\"", "take_queue \"$job\"", "attempts \"$job\" \"$tree\" \"$log\" :", "exit \"$status\""]), "{branch}");
    assert!(!branch.contains("record ") && !branch.contains("remember"), "{branch}");
}

/// `failing_classes.py` reads an analysis's `failing-paths.tsv` (`failing-paths.tcl`): every endpoint classed by the
/// node driving the widest fan-out on its path, below the shell, that net's copies across cores, channels and lanes one
/// class (its indices and numbers made `[*]` and `#`), printed widest first with its share, its worst slack and the
/// nodes its paths most often start and end at. `--worst N` classes only the N endpoints of least slack, and
/// `--set-aside` leaves out an endpoint whose class or first node holds a pattern, saying how many it left.
#[test]
fn a_failing_paths_report_groups_its_endpoints_by_the_net_they_share() {
    let tmp = tempfile::tempdir().unwrap();
    let tsv = tmp.path().join("failing-paths.tsv");
    let shell = "afu_top|port_gasket|pr_slot|afu_main|ofs_plat_afu|shell|engine|fast|";
    let reset = "afu_top|port_gasket|pr_slot|afu_main|clocks|port[0].r|uClk_usr_reset|cc|q";
    let mut text = String::from("clock\tslack\tdata_delay\tlevels\tskew\tfanout\tfanout_node\tfrom\tto\n");
    for (slack, fanout, node, from, to) in [
        ("-9.0", "5280", format!("{shell}unit|u|core[0].u|i23~0|combout"), format!("{shell}unit|u|core[0].u|rp[1]"), format!("{shell}unit|u|core[0].u|lbd[3]")),
        ("-8.0", "5280", format!("{shell}unit|u|core[1].u|i23~0|combout"), format!("{shell}unit|u|core[1].u|rp[0]"), format!("{shell}unit|u|core[1].u|lbd[7]")),
        ("-12.5", "23589", reset.to_string(), reset.to_string(), format!("{shell}side|lane[1].lane|fl|x[4]")),
        ("-7.0", "5280", format!("{shell}unit|u|core[2].u|i23~0|combout"), format!("{shell}unit|u|core[2].u|rp[1]"), format!("{shell}unit|u|core[2].u|lbd[0]")),
        ("-6.0", "37", format!("{shell}port|rs_data[3]~12|combout"), format!("{shell}port|o_count[2]"), format!("{shell}vside|fq_data~9")),
    ] {
        text += &format!("uClk\t{slack}\t11.5\t3\t0.1\t{fanout}\t{node}\t{from}\t{to}\n");
    }
    std::fs::write(&tsv, text).unwrap();
    let run = |args: &[&str]| -> String {
        let o = Command::new("python3").arg(root().join("scripts/d5005/failing_classes.py")).arg(&tsv).args(args).output().expect("python3");
        assert!(o.status.success(), "failing_classes.py {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    let classes = |out: &str| -> Vec<String> { out.lines().filter(|l| l.contains(" worst ")).map(|l| l.trim().to_string()).collect() };
    let all = run(&[]);
    assert!(all.starts_with("5 endpoints, slack -12.5 to -6.0 ns\n"), "{all}");
    let lines = classes(&all);
    assert_eq!(lines.len(), 3, "the cores' three copies of one net are one class:\n{all}");
    assert!(
        lines[0].starts_with("3  60.0%  60.0%  worst   -9.000  levels 3  delay 11.5  fan-out 5280  engine|fast|unit|u|core[*].u|i#~#|combout"),
        "the widest class first, its worst slack the least of its copies':\n{all}"
    );
    assert!(all.contains("          from engine|fast|unit|u|core[*].u|rp[*]\n          to   engine|fast|unit|u|core[*].u|lbd[*]\n"), "{all}");
    assert!(lines[1].starts_with("1  20.0%  80.0%  worst  -12.500") && lines[1].ends_with("port[*].r|uClk_usr_reset|cc|q"), "a net outside the shell is named whole:\n{all}");
    let worst = run(&["--worst", "2"]);
    assert!(worst.starts_with("2 endpoints, slack -12.5 to -9.0 ns\n"), "{worst}");
    assert_eq!(classes(&worst).len(), 2, "the two endpoints of least slack, a class each:\n{worst}");
    let aside = run(&["--set-aside", "uClk_usr_reset", "--set-aside", "port|o_count"]);
    assert!(aside.starts_with("2 of 5 endpoints set aside (uClk_usr_reset, port|o_count)\n3 endpoints"), "{aside}");
    assert_eq!(classes(&aside).len(), 1, "the reset by its class and the fast port's by its first node:\n{aside}");
}
