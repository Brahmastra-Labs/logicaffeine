use super::*;

#[test]
fn every_eda_summary_is_what_its_rows_say() {
    // The page shows summaries; the rows are the measurements. A regenerated JSON whose summary drifted from its rows
    // (a hand edit, a collector bug) would put a number on the page no run produced.
    let data = &*EDA_DATA;
    let mc = &data.mc;
    assert!(!mc.designs.is_empty() && !mc.run.abc_version.is_empty(), "the model-checking run names its designs and ABC");
    fn cell<'a>(engine: &str, d: &'a McDesign) -> &'a McCell {
        if engine == "ours" {
            &d.ours
        } else {
            &d.abc
        }
    }
    for (engine, summary) in &mc.summary {
        let solved: Vec<&McDesign> = mc.designs.iter().filter(|d| cell(engine, d).verdict.is_some()).collect();
        let safe = solved.iter().filter(|d| cell(engine, d).verdict.as_deref() == Some("safe")).count();
        let wrong = solved
            .iter()
            .filter(|d| d.consensus != "unknown" && cell(engine, d).verdict.as_deref() != Some(d.consensus.as_str()))
            .count();
        assert_eq!(
            (summary.solved as usize, summary.safe as usize, summary.unsafe_ as usize, summary.wrong as usize),
            (solved.len(), safe, solved.len() - safe, wrong),
            "{engine}'s model-checking summary"
        );
    }
    assert_eq!(mc.summary["ours"].wrong, 0, "no verdict of ours contradicts the competition's consensus");
    assert!(mc.field.windows(2).all(|w| w[0].solved >= w[1].solved), "the field is listed best first");

    let rw = &data.rewrite;
    assert!(!rw.designs.is_empty());
    for d in &rw.designs {
        assert!(d.ours.ands.is_some(), "{}: every rewrite of ours is proven equal to its input", d.name);
        assert_ne!(d.ours.cec, "unequal", "{}: ABC's cec never finds a rewrite of ours unequal", d.name);
        assert!(d.ours.ands <= d.input, "{}: a rewrite never grows the circuit", d.name);
    }

    let count = &data.count;
    assert!(!count.formulas.is_empty());
    assert!(count.formulas.iter().all(|f| f.ours.models.as_deref().is_none_or(|m| m.bytes().all(|b| b.is_ascii_digit()))));

    let crypto = &data.crypto;
    for engine in &crypto.solvers {
        let by_category: u32 = crypto.categories.values().map(|c| c.get(engine).copied().unwrap_or(0)).sum();
        assert_eq!(crypto.summary[engine].solved, by_category, "{engine}'s cryptanalysis total is its categories'");
    }
    assert_eq!(crypto.summary["ours"].verified, crypto.summary["ours"].solved, "every model of ours was checked");
}

#[test]
fn the_bounded_model_checking_summary_is_what_its_rows_say() {
    // The BMC arena in the model-checking shape: each engine's summary its rows' count, every answer a run to a bad
    // state, and no design among them the competition found safe; none of ours contradicts its consensus.
    let bmc = &EDA_DATA.bmc;
    assert!(!bmc.designs.is_empty() && !bmc.run.abc_version.is_empty(), "the BMC run names its designs and ABC");
    assert!(bmc.designs.iter().all(|d| d.consensus != "safe"), "only designs the competition did not find safe");
    for (engine, summary) in &bmc.summary {
        let cell = |d: &McDesign| if engine == "ours" { d.ours.verdict.clone() } else { d.abc.verdict.clone() };
        let runs: Vec<&McDesign> = bmc.designs.iter().filter(|d| cell(d).is_some()).collect();
        assert!(runs.iter().all(|d| cell(d).as_deref() == Some("unsafe")), "{engine} answers with runs alone");
        let wrong = runs.iter().filter(|d| d.consensus == "safe").count();
        assert_eq!(
            (summary.solved as usize, summary.safe, summary.unsafe_ as usize, summary.wrong as usize),
            (runs.len(), 0, runs.len(), wrong),
            "{engine}'s bounded-model-checking summary"
        );
    }
    assert_eq!(bmc.summary["ours"].wrong, 0, "no run of ours contradicts the competition's consensus");
    assert!(bmc.field.windows(2).all(|w| w[0].solved >= w[1].solved), "the field is listed best first");
}

#[test]
fn abc_reads_each_design_as_aiger_describes_it() {
    // A latch free to start anywhere is one ABC's engines read as starting at zero, and `fold` reads it so too: ABC
    // asked of a circuit read that way would prove its runs from zero alone. Both model-checking arenas record how ABC
    // read each design, and that read makes every such latch's first value an input (`undc`) before anything else.
    for (arena, data) in [("mc", &EDA_DATA.mc), ("bmc", &EDA_DATA.bmc)] {
        let steps: Vec<&str> = data.abc_read.split(';').map(str::trim).collect();
        let at = |step: &str| steps.iter().position(|s| *s == step);
        let (undc, zero, fold) = (at("undc"), at("zero"), at("fold"));
        assert!(undc.is_some() && zero.is_some(), "{arena}: ABC's read {:?} frees and then zeroes", data.abc_read);
        assert!(undc < zero && fold.is_none_or(|f| undc < Some(f)), "{arena}: `undc` comes first in {:?}", data.abc_read);
    }
}

#[test]
fn the_competitions_solvers_are_counted_on_the_clock_our_engines_run_under() {
    // Our engines run one thread each under a CPU limit; the competition's portfolios spread a run over threads, so its
    // real seconds are not ours. Each solver is counted on both clocks, and the page names both counts of each arena's
    // best five on each, best first.
    let mut vdom = VirtualDom::new(EdaSection);
    vdom.rebuild_in_place();
    let page = dioxus_ssr::render(&vdom);
    for (arena, data) in [("mc", &EDA_DATA.mc), ("bmc", &EDA_DATA.bmc)] {
        assert!(!data.field.is_empty(), "{arena}: the competition's solvers are counted");
        assert!(data.field.iter().any(|f| f.solved_cpu != f.solved), "{arena}: the CPU counts are counts of their own");
        for cpu in [true, false] {
            let count = |f: &FieldEntry| if cpu { f.solved_cpu } else { f.solved };
            let mut best: Vec<&FieldEntry> = data.field.iter().filter(|f| count(f) > 0).collect();
            best.sort_by_key(|f| (std::cmp::Reverse(count(f)), f.solver.clone()));
            let line = best.iter().take(5).map(|f| format!("{} {}", eda_label(&f.solver), count(f))).collect::<Vec<_>>();
            let clock = if cpu { "of CPU time over all its threads, the clock our engines run under: " } else { "of real time: " };
            let shown = format!("{clock}{}", line.join(", "));
            assert!(page.contains(&shown), "{arena}: the page shows {shown:?}");
        }
        assert!(page.contains(&format!("({})", data.abc_read)), "{arena}: the page says how ABC read each design");
    }
    assert!(!page.contains("as starting at zero"), "the page claims no zeroed reading of ABC's");
}
