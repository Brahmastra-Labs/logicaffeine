use super::*;

const FLOW: &str = "\
+---------------------------------------------------------------------------------------------+
; Flow Summary                                                                                 ;
+---------------------------------+------------------------------------------------------------+
; Flow Status                     ; Successful - Thu Oct 22 04:02:50 2026                      ;
; Quartus Prime Version           ; 23.4.0 Build 79 11/22/2023 Patches 0.01iofs SC Pro Edition ;
; Revision Name                   ; engine                                                     ;
; Top-level Entity Name           ; logos_engine                                               ;
; Family                          ; Stratix 10                                                 ;
; Device                          ; 1SX280HN2F43E2VG                                           ;
; Logic utilization (in ALMs)     ; 453,119 / 933,120 ( 49 % )                                 ;
+---------------------------------+------------------------------------------------------------+

+-----------------------------------------+
; Flow Settings                           ;
+-------------------+---------------------+
; Option            ; Setting             ;
+-------------------+---------------------+
; Start date & time ; 10/22/2026 00:49:24 ;
; Main task         ; Compilation         ;
; Revision Name     ; engine              ;
+-------------------+---------------------+

+------------------------------------------------------+
; Flow Elapsed Time                                    ;
+-----------------+--------------+---------------------+
; Module Name     ; Elapsed Time ; Peak Virtual Memory ;
+-----------------+--------------+---------------------+
; Synthesis       ; 00:16:49     ; 11628 MB            ;
; Fitter          ; 02:47:28     ; 67136 MB            ;
; Timing Analyzer ; 00:06:45     ; 54533 MB            ;
; Assembler       ; 00:01:52     ; 14344 MB            ;
; Total           ; 03:12:54     ; --                  ;
+-----------------+--------------+---------------------+
";

const FIT_SUMMARY: &str = "\
Fitter Status : Successful - Thu Oct 22 03:53:43 2026
Quartus Prime Version : 23.4.0 Build 79 11/22/2023 Patches 0.01iofs SC Pro Edition
Revision Name : engine
Top-level Entity Name : logos_engine
Family : Stratix 10
Device : 1SX280HN2F43E2VG
Logic utilization (in ALMs) : 453,119 / 933,120 ( 49 % )
  * Logic utilization warning : A large number of ALMs contain virtual pins and count towards logic utilization
  * ALMs containing virtual pins : 1,185 / 933,120 ( < 1 % )
Total dedicated logic registers : 144812
Total pins : 0 / 912 ( 0 % )
Total block memory bits : 41,231,068 / 240,046,080 ( 17 % )
Total RAM Blocks : 4,378 / 11,721 ( 37 % )
Total DSP Blocks : 2,385 / 5,760 ( 41 % )
Total PLLs : 0 / 104 ( 0 % )
";

const STA_SUMMARY: &str = "\
------------------------------------------------------------
Timing Analyzer Summary
------------------------------------------------------------

Type  : Setup 'clk'
Slack : -325.342
TNS   : -8849465.191
Corner: Slow 900mV 100C Model

Type  : Setup 'emif_top_inst|mem_bank[0].emif_ddr4_inst|emif_s10_0_core_usr_clk'
Slack : -0.584
TNS   : -178.482
Corner: Slow 900mV 100C Model

Type  : Hold 'clk'
Slack : 0.000
TNS   : 0.000
Corner: Slow 900mV 0C Model

Type  : Minimum Pulse Width 'clk'
Slack : 49.179
TNS   : 0.000
Corner: Slow 900mV 0C Model

------------------------------------------------------------
";

const RETIME: &str = "\
+----------------------------------------------------------------------------------------------------------------------+
; Retiming Limit Summary                                                                                               ;
+------------------+----------------------------------------------------------------------------------+----------------+
; Clock Transfer   ; Limiting Reason                                                                  ; Recommendation ;
+------------------+----------------------------------------------------------------------------------+----------------+
; Clock Domain clk ; Reached analysis threshold limit because of hold: No further analysis performed. ; None           ;
+------------------+----------------------------------------------------------------------------------+----------------+
";

const ROUTE: &str = "\
Info (20032): Parallel compilation is enabled and will use up to 8 processors
Info (20215): Router estimated peak short interconnect demand : 88% of up directional wire in region X128_Y392 to X135_Y399
    Info (20265): Estimated peak short right directional wire demand : 43% in region X56_Y120 to X63_Y127
Info (20215): Router estimated peak long high speed interconnect demand : 118% of down directional wire in region X160_Y40 to X167_Y47
    Info (20265): Estimated peak long high speed right directional wire demand : 111% in region X176_Y40 to X183_Y47
";

const HIERARCHY: &str = "\
+------------------------------------------------------------------------+
; Partition \"root_partition\" Resource Utilization by Entity              ;
+---------------------------------------+---------------------+---------------------------+-------------------+------------+------+--------+---------------+------------------------------+---------------------------+--------------+
; Compilation Hierarchy Node            ; Combinational ALUTs ; Dedicated Logic Registers ; Block Memory Bits ; DSP Blocks ; Pins ; IOPLLs ; Max Depth     ; Full Hierarchy Name          ; Entity Name               ; Library Name ;
+---------------------------------------+---------------------+---------------------------+-------------------+------------+------+--------+---------------+------------------------------+---------------------------+--------------+
; |                                     ; 661602 (53012)      ; 153507 (96543)            ; 41231068          ; 2627       ; 2369 ; 0 (0)  ; 729.0 (194.0) ; |                            ; logos_engine              ; altera_work  ;
;    |aq|                               ; 36962 (1074)        ; 5191 (4158)               ; 8192              ; 0          ; 0    ; 0 (0)  ; 196.2 (196.2) ; aq                           ; logos_act_stream          ; altera_work  ;
;       |beats|                         ; 514 (514)           ; 1033 (1033)               ; 8192              ; 0          ; 0    ; 0 (0)  ; 196.2 (196.2) ; aq|beats                     ; logos_ram_w512            ; altera_work  ;
;       |lane[0].u|                     ; 2186 (268)          ; 0 (0)                     ; 0                 ; 0          ; 0    ; 0 (0)  ; 196.2 (196.2) ; aq|lane[0].u                 ; logos_act_code            ; altera_work  ;
;          |div_0_rtl_0|                ; 962 (0)             ; 0 (0)                     ; 0                 ; 0          ; 0    ; 0 (0)  ; 196.2 (0.0)   ; aq|lane[0].u|div_0_rtl_0     ; lpm_divide                ; work         ;
;       |lane[1].u|                     ; 2186 (268)          ; 0 (0)                     ; 0                 ; 0          ; 0    ; 0 (0)  ; 196.2 (196.2) ; aq|lane[1].u                 ; logos_act_code            ; altera_work  ;
;          |div_0_rtl_0|                ; 962 (0)             ; 0 (0)                     ; 0                 ; 0          ; 0    ; 0 (0)  ; 196.2 (0.0)   ; aq|lane[1].u|div_0_rtl_0     ; lpm_divide                ; work         ;
;    |vu|                               ; 223000 (900)        ; 40000 (100)               ; 0                 ; 473        ; 0    ; 0 (0)  ; 706.0 (12.0)  ; vu                           ; logos_simd                ; altera_work  ;
;       |inner|                         ; 1000 (1000)         ; 0 (0)                     ; 0                 ; 0          ; 0    ; 0 (0)  ; 20.0 (20.0)   ; vu|inner                     ; logos_simd                ; altera_work  ;
+---------------------------------------+---------------------+---------------------------+-------------------+------------+------+--------+---------------+------------------------------+---------------------------+--------------+
";

#[test]
fn the_flow_report_names_the_build_and_times_each_phase() {
    let f = parse_flow(FLOW);
    assert_eq!(f.status.as_deref(), Some("Successful - Thu Oct 22 04:02:50 2026"));
    assert_eq!(f.version.as_deref(), Some("23.4.0 Build 79 11/22/2023 Patches 0.01iofs SC Pro Edition"));
    assert_eq!(f.revision.as_deref(), Some("engine"));
    assert_eq!(f.top.as_deref(), Some("logos_engine"));
    assert_eq!(f.device.as_deref(), Some("1SX280HN2F43E2VG"));
    assert_eq!(f.started.as_deref(), Some("10/22/2026 00:49:24"));
    assert_eq!(
        f.phases,
        vec![
            Phase { name: "Synthesis".into(), seconds: 16 * 60 + 49, peak_mb: Some(11628) },
            Phase { name: "Fitter".into(), seconds: 2 * 3600 + 47 * 60 + 28, peak_mb: Some(67136) },
            Phase { name: "Timing Analyzer".into(), seconds: 6 * 60 + 45, peak_mb: Some(54533) },
            Phase { name: "Assembler".into(), seconds: 112, peak_mb: Some(14344) },
            Phase { name: "Total".into(), seconds: 3 * 3600 + 12 * 60 + 54, peak_mb: None },
        ]
    );
    assert!(f.succeeded());
}

#[test]
fn a_summary_reads_its_resources_as_used_over_total() {
    let u = parse_summary(FIT_SUMMARY);
    assert_eq!(u.alms, Some(Ratio { used: 453_119, total: 933_120 }));
    assert_eq!(u.registers, Some(144_812));
    assert_eq!(u.block_memory_bits, Some(Ratio { used: 41_231_068, total: 240_046_080 }));
    assert_eq!(u.ram_blocks, Some(Ratio { used: 4_378, total: 11_721 }));
    assert_eq!(u.dsp_blocks, Some(Ratio { used: 2_385, total: 5_760 }));
    assert_eq!(u.status.as_deref(), Some("Successful - Thu Oct 22 03:53:43 2026"));
    assert_eq!(parse_summary("Synthesis Status : Successful - x\nFamily : Stratix 10\n").alms, None);
}

#[test]
fn every_clock_of_every_analysis_keeps_its_slack() {
    let s = parse_sta_summary(STA_SUMMARY);
    assert_eq!(s.len(), 4);
    assert_eq!(
        s[0],
        Slack {
            analysis: "Setup".into(),
            clock: "clk".into(),
            slack_ns: -325.342,
            tns_ns: -8849465.191,
            corner: "Slow 900mV 100C Model".into()
        }
    );
    assert_eq!(s[1].clock, "emif_top_inst|mem_bank[0].emif_ddr4_inst|emif_s10_0_core_usr_clk");
    assert_eq!(s[3].analysis, "Minimum Pulse Width");
    assert_eq!(worst_setup(&s).map(|w| w.slack_ns), Some(-325.342));
}

#[test]
fn the_retimers_limit_and_the_routers_peaks_are_read() {
    assert_eq!(
        parse_retime(RETIME),
        vec![("Clock Domain clk".to_string(), "Reached analysis threshold limit because of hold: No further analysis performed.".to_string())]
    );
    let peaks = parse_wire_demand(ROUTE);
    assert_eq!(
        peaks,
        vec![
            WireDemand { wire: "short interconnect".into(), percent: 88, direction: "up".into(), region: "X128_Y392 to X135_Y399".into() },
            WireDemand {
                wire: "long high speed interconnect".into(),
                percent: 118,
                direction: "down".into(),
                region: "X160_Y40 to X167_Y47".into()
            },
        ]
    );
    assert_eq!(parse_processors(ROUTE), Some(8));
    assert_eq!(parse_processors("no such line"), None);
}

#[test]
fn the_hierarchy_keeps_each_node_its_depth_and_its_totals() {
    let rows = parse_hierarchy(HIERARCHY);
    assert_eq!(rows.len(), 9);
    assert_eq!(rows[0].path, "|");
    assert!(rows.iter().all(|r| r.partition == "root_partition"));
    assert_eq!((rows[0].depth, rows[0].aluts, rows[0].registers, rows[0].dsp), (0, 661_602, 153_507, 2_627));
    assert_eq!(rows[0].max_depth, Some(729.0));
    assert_eq!((rows[1].path.as_str(), rows[1].entity.as_str(), rows[1].depth), ("aq", "logos_act_stream", 1));
    assert_eq!((rows[4].path.as_str(), rows[4].depth, rows[4].aluts), ("aq|lane[0].u|div_0_rtl_0", 3, 962));
    assert_eq!(rows[7].max_depth, Some(706.0));
}

#[test]
fn entities_total_over_their_outermost_instances() {
    let totals = by_entity(&parse_hierarchy(HIERARCHY));
    let get = |name: &str| totals.iter().find(|t| t.entity == name).unwrap();
    assert_eq!(get("logos_act_code").instances, 2);
    assert_eq!(get("logos_act_code").aluts, 2 * 2186);
    assert_eq!(get("lpm_divide").aluts, 2 * 962);
    let simd = get("logos_simd");
    assert_eq!((simd.instances, simd.aluts), (2, 223_000), "a nested instance of the same entity is not counted twice");
    assert_eq!(simd.max_depth, Some(706.0));
    assert_eq!(totals[0].entity, "logos_engine", "largest first");
}

#[test]
fn malformed_numbers_are_not_read_as_zeros() {
    assert_eq!(parse_summary("Total DSP Blocks : many / 5,760 ( 41 % )\n").dsp_blocks, None);
    assert!(parse_sta_summary("Type  : Setup 'clk'\nSlack : fast\nTNS : 0\nCorner: x\n").is_empty());
}

const SYNTH: &str = "\
+-----------------------------------------------------------------+
; Synthesis Resource Usage Summary for Partition \"root_partition\" ;
+---------------------------------------------+-------------------+
; Resource                                    ; Usage             ;
+---------------------------------------------+-------------------+
; Estimate of Logic utilization (ALMs needed) ; 16326             ;
;                                             ;                   ;
; Combinational ALUT usage for logic          ; 23845             ;
; Dedicated logic registers                   ; 0                 ;
; Total DSP Blocks                            ; 24                ;
;     -- [A] Total Fixed Point DSP Blocks     ; 24                ;
;     -- [B] Total Floating Point DSP Blocks  ; 0                 ;
; Maximum fan-out node                        ; on_l1p            ;
+---------------------------------------------+-------------------+
";

#[test]
fn synthesis_estimates_split_fixed_from_floating_point_dsp() {
    let parts = parse_synthesis(SYNTH);
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].partition, "root_partition");
    let e = &parts[0].estimate;
    assert_eq!(e.alms, Some(16_326));
    assert_eq!(e.registers, Some(0));
    assert_eq!((e.dsp_blocks, e.fixed_point_dsp, e.floating_point_dsp), (Some(24), Some(24), Some(0)));
    assert_eq!(e.memory_bits, None);
    assert!(parse_synthesis(FIT_SUMMARY).is_empty(), "a report without the table has no estimate");
}

#[test]
fn a_partial_reconfiguration_build_keeps_each_partitions_estimate_apart() {
    let persona = SYNTH.replace("root_partition", "persona1").replace("16326", "20491").replace("; 24 ", "; 7  ");
    let parts = parse_synthesis(&format!("{SYNTH}\n{persona}"));
    assert_eq!(parts.iter().map(|p| p.partition.as_str()).collect::<Vec<_>>(), ["root_partition", "persona1"]);
    assert_eq!(parts[1].estimate.alms, Some(20_491));
    assert_eq!(parts[1].estimate.dsp_blocks, Some(7));
    let b = Build { synthesis: parts, ..Build::default() };
    assert_eq!(b.synthesis_of("persona1").and_then(|e| e.alms), Some(20_491));
    assert_eq!(b.synthesis_of("auto_fab_0"), None);
}

#[test]
fn the_processors_allowed_come_from_the_message_or_the_parallel_compilation_table() {
    let table = "; Parallel Compilation                ;\n+----------------------------+--------+\n\
                 ; Processors                 ; Number ;\n+----------------------------+--------+\n\
                 ; Number detected on machine ; 32     ;\n; Maximum allowed            ; 24     ;\n";
    assert_eq!(parse_processors(table), Some(24));
}

#[test]
fn a_compile_directory_reads_as_one_build() {
    let dir = tempfile::tempdir().unwrap();
    let put = |suffix: &str, text: &str| std::fs::write(dir.path().join(format!("engine.{suffix}")), text).unwrap();
    put("flow.rpt", FLOW);
    put("fit.summary", FIT_SUMMARY);
    put("sta.summary", STA_SUMMARY);
    put("fit.retime.rpt", RETIME);
    put("fit.route.rpt", ROUTE);
    put("syn.rpt", &format!("{SYNTH}\n{HIERARCHY}"));
    let b = read(dir.path(), "engine").unwrap();
    assert!(b.flow.succeeded());
    assert_eq!(b.flow.phase("Fitter").map(|p| p.peak_mb), Some(Some(67136)));
    assert_eq!(b.fit.as_ref().and_then(|f| f.alms), Some(Ratio { used: 453_119, total: 933_120 }));
    assert_eq!(b.synthesis_of("root_partition").and_then(|s| s.floating_point_dsp), Some(0));
    assert_eq!(b.worst_setup().map(|s| s.slack_ns), Some(-325.342));
    assert_eq!(b.retime.len(), 1);
    assert_eq!(b.wire_demand.iter().map(|w| w.percent).max(), Some(118));
    assert_eq!(b.processors, Some(8));
    assert_eq!(b.top.iter().map(|n| n.path.as_str()).collect::<Vec<_>>(), ["|", "aq", "vu"]);
    assert_eq!(b.entities[0].entity, "logos_engine");
    let back: Build = serde_json::from_str(&serde_json::to_string(&b).unwrap()).unwrap();
    assert_eq!(back, b, "a build round-trips through its ledger form");
}

#[test]
fn a_report_quartus_never_wrote_is_absent_not_zero() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("unit.flow.rpt"), FLOW).unwrap();
    let b = read(dir.path(), "unit").unwrap();
    assert_eq!(b.fit, None);
    assert!(b.synthesis.is_empty());
    assert!(b.clocks.is_empty() && b.top.is_empty() && b.entities.is_empty());
    assert_eq!(read(dir.path(), "other").err(), Some(QuartusError::NoFlowReport(dir.path().join("other.flow.rpt").display().to_string())));
}

#[test]
fn timestamps_are_utc_civil_dates() {
    assert_eq!(utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(utc(1_792_691_483), "2026-10-22T17:51:23Z");
    assert_eq!(utc(951_868_799), "2000-02-29T23:59:59Z", "a leap day");
    assert_eq!(utc(4_107_542_400), "2100-03-01T00:00:00Z", "2100 is not a leap year");
}

#[test]
fn a_ledger_refuses_a_name_twice_and_round_trips_through_its_file() {
    let row = |name: &str| Row {
        name: name.into(),
        recorded: utc(0),
        inputs: vec![Input { path: "engine.v".into(), sha256: "00".repeat(32) }],
        build: Build { flow: parse_flow(FLOW), ..Build::default() },
        verdict: Verdict::default(),
        resumed: None,
    };
    let mut ledger = Ledger::default();
    ledger.record(row("fit27")).unwrap();
    assert_eq!(ledger.record(row("fit27")), Err(QuartusError::Recorded("fit27".into())));
    ledger.record(row("engine2")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("builds.json");
    assert_eq!(Ledger::load(&path).unwrap(), Ledger::default(), "no file is an empty ledger");
    ledger.save(&path).unwrap();
    let back = Ledger::load(&path).unwrap();
    assert_eq!(back, ledger);
    assert_eq!(back.row("engine2").map(|r| r.name.as_str()), Some("engine2"));
    assert!(std::fs::read_to_string(&path).unwrap().ends_with("}\n"));
}

#[test]
fn a_wide_partition_keeps_its_root_and_largest_children_in_report_order() {
    let node = |partition: &str, path: String, depth: usize, aluts: u64| Node {
        partition: partition.into(),
        path,
        entity: "e".into(),
        depth,
        aluts,
        registers: 0,
        memory_bits: 0,
        dsp: 0,
        max_depth: None,
    };
    let mut nodes = vec![node("root_partition", "|".into(), 0, 1_000_000)];
    for k in 0..40u64 {
        nodes.push(node("root_partition", format!("c{k}"), 1, (k * 7919) % 97));
        nodes.push(node("root_partition", format!("c{k}|g"), 2, 5));
    }
    nodes.push(node("persona1", "|".into(), 0, 9));
    nodes.push(node("persona1", "p".into(), 1, 3));
    let top = top_of(nodes.clone());
    let root: Vec<&Node> = top.iter().filter(|n| n.partition == "root_partition").collect();
    assert_eq!(root.len(), 1 + TOP_ROWS);
    assert_eq!(root[0].path, "|");
    let kept_min = root[1..].iter().map(|n| n.aluts).min().unwrap();
    let dropped_max = nodes.iter().filter(|n| n.depth == 1 && n.partition == "root_partition" && !root.iter().any(|k| k.path == n.path)).map(|n| n.aluts).max().unwrap();
    assert!(kept_min >= dropped_max, "the largest children are the ones kept");
    let order: Vec<usize> = root[1..].iter().map(|n| n.path[1..].parse().unwrap()).collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "report order: {order:?}");
    assert_eq!(top.iter().filter(|n| n.partition == "persona1").count(), 2);
    assert!(top.iter().all(|n| n.depth <= 1));
}

#[test]
fn the_router_messages_the_fitters_report_repeats_are_read_once() {
    let dir = tempfile::tempdir().unwrap();
    let put = |suffix: &str, text: &str| std::fs::write(dir.path().join(format!("engine.{suffix}")), text).unwrap();
    put("flow.rpt", FLOW);
    put("fit.rpt", &format!("; Maximum allowed            ; 24     ;\n{ROUTE}"));
    put("fit.route.rpt", ROUTE);
    let b = read(dir.path(), "engine").unwrap();
    assert_eq!(b.wire_demand.len(), 2, "{:?}", b.wire_demand);
    assert_eq!(b.processors, Some(24), "the fitter's own table first");
    std::fs::remove_file(dir.path().join("engine.fit.route.rpt")).unwrap();
    assert_eq!(read(dir.path(), "engine").unwrap().wire_demand.len(), 2, "the fitter's report when the router's is not there");
}

#[test]
fn the_processors_message_reads_in_both_of_its_forms() {
    assert_eq!(parse_processors("Info (20032): Parallel compilation is enabled and will use up to 8 processors"), Some(8));
    assert_eq!(parse_processors("Info (20030): Parallel compilation is enabled and will use 24 of the 24 processors detected"), Some(24));
}

/// A flow report whose status is `status` and whose modules are `phases`.
fn flow_of(status: &str, phases: &[&str]) -> Build {
    Build {
        flow: Flow {
            status: Some(status.to_string()),
            phases: phases.iter().map(|p| Phase { name: p.to_string(), seconds: 1, peak_mb: Some(1) }).collect(),
            ..Flow::default()
        },
        ..Build::default()
    }
}

const PR: [&str; 4] = ["Synthesis", "Fitter", "Timing Analyzer", "Assembler"];

#[test]
fn a_compile_that_ran_everything_it_runs_and_said_so_passed() {
    let b = flow_of("Successful - Thu Oct 22 05:04:06 2026", &["Synthesis", "Fitter", "Timing Analyzer", "Assembler", "Total"]);
    let e = Evidence {
        exit: Some(0),
        expected: PR.iter().map(|s| s.to_string()).collect(),
        log: Some("Info: Quartus Prime Shell was successful. 0 errors, 2058 warnings\n".into()),
        started: Some(100),
        flow_written: Some(200),
    };
    let v = judge(&b, &e);
    assert_eq!(v, Verdict { passed: true, failures: vec![], exit: Some(0), expected: e.expected.clone() });
}

#[test]
fn a_fitter_that_died_at_startup_fails_the_compile_its_synthesis_report_calls_successful() {
    let b = flow_of("Successful - Thu Oct 22 21:08:44 2026", &["Synthesis", "Total"]);
    let flow_alone = judge(&b, &Evidence::default());
    assert!(flow_alone.passed, "the flow report alone cannot tell");
    let e = Evidence { expected: PR.iter().map(|s| s.to_string()).collect(), ..Evidence::default() };
    let v = judge(&b, &e);
    assert!(!v.passed);
    assert_eq!(
        v.failures,
        [
            "the job runs Fitter, which the flow report does not hold",
            "the job runs Timing Analyzer, which the flow report does not hold",
            "the job runs Assembler, which the flow report does not hold",
        ]
    );
}

#[test]
fn an_exit_status_a_failure_line_or_a_stale_report_each_fail_a_compile() {
    let b = flow_of("Successful - Thu Oct 22 21:08:44 2026", &["Synthesis", "Total"]);
    let exit = judge(&b, &Evidence { exit: Some(3), ..Evidence::default() });
    assert_eq!(exit.failures, ["the flow exited 3"]);
    let log = "Info: ...\nError (22912): Unhandled exception: \nFatal Error: Assertion failed tools/cpp/ddm/ddm_assessor.cpp:53: DDM_T::verify_token(token) : Cannot identify the client\n*** Fatal Error: Program termination requested\n";
    let crash = judge(&b, &Evidence { log: Some(log.into()), ..Evidence::default() });
    assert_eq!(crash.failures, ["the log: Fatal Error: Assertion failed tools/cpp/ddm/ddm_assessor.cpp:53: DDM_T::verify_token(token) : Cannot identify the client"]);
    let stale = judge(&b, &Evidence { started: Some(1_000), flow_written: Some(999), ..Evidence::default() });
    assert_eq!(stale.failures, ["the flow report was written before the job started"]);
    let same_second = judge(&b, &Evidence { started: Some(1_000), flow_written: Some(1_000), ..Evidence::default() });
    assert!(same_second.passed);
    let failed = judge(&flow_of("Flow Failed - Thu Oct 22 19:31:14 2026", &["Synthesis", "Fitter", "Total"]), &Evidence::default());
    assert_eq!(failed.failures, ["the flow report says Flow Failed - Thu Oct 22 19:31:14 2026"]);
    let silent = judge(&Build::default(), &Evidence::default());
    assert_eq!(silent.failures, ["the flow report has no status"]);
}

#[test]
fn a_log_fails_on_its_first_failure_line_and_never_on_a_success() {
    assert_eq!(log_failure("Info: Quartus Prime Full Compilation was successful. 0 errors, 2058 warnings\nInfo: Quartus Prime Shell was successful. 0 errors, 2058 warnings\n"), None);
    assert_eq!(log_failure("Info: Running Quartus Prime Fitter\nError: Quartus Prime Shell was unsuccessful. 3 errors, 494 warnings\nQuartus build failed\n").as_deref(), Some("Error: Quartus Prime Shell was unsuccessful. 3 errors, 494 warnings"));
    assert_eq!(log_failure("make: ok\nQuartus build failed\n").as_deref(), Some("Quartus build failed"));
    assert_eq!(log_failure("    *** Fatal Error: Program termination requested\n").as_deref(), Some("*** Fatal Error: Program termination requested"));
    assert_eq!(log_failure("Info: 0 errors\n"), None);
}

#[test]
fn a_recorded_compile_is_replaced_only_under_its_own_name() {
    let row = |name: &str, passed: bool| Row {
        name: name.into(),
        recorded: utc(0),
        inputs: vec![],
        build: Build::default(),
        verdict: Verdict { passed, ..Verdict::default() },
        resumed: None,
    };
    let mut ledger = Ledger::default();
    ledger.record(row("a", true)).unwrap();
    ledger.record(row("b", true)).unwrap();
    ledger.replace(row("a", false)).unwrap();
    assert_eq!(ledger.rows.iter().map(|r| (r.name.as_str(), r.verdict.passed)).collect::<Vec<_>>(), [("a", false), ("b", true)], "in its place");
    assert_eq!(ledger.replace(row("c", true)), Err(QuartusError::Unrecorded("c".into())));
}

#[test]
fn a_router_estimate_printed_twice_is_one_estimate() {
    let line = "Info (20215): Router estimated peak short interconnect demand : 0% of right directional wire in region X0_Y0 to X7_Y7\n";
    let w = parse_wire_demand(&format!("{line}    Info (20265): Estimated peak short up directional wire demand : 0% in region X0_Y0 to X7_Y7\n{line}"));
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(parse_wire_demand(ROUTE).len(), 2, "two kinds of wire stay two");
}

const HOLD: &str = "\
+------------------------------------------------------------+
; Estimated Delay Added for Hold Timing Summary              ;
+-----------------+----------------------+-------------------+
; Source Clock(s) ; Destination Clock(s) ; Delay Added in ns ;
+-----------------+----------------------+-------------------+
; clk             ; clk                  ; 41707.9           ;
; clk,I/O         ; clk                  ; 1795.1            ;
+-----------------+----------------------+-------------------+
Note: For more information on problematic transfers, consider running the Fitter again with the Optimize hold timing option (Settings Menu) turned off.
";

const GLOBALS: &str = "\
+------------------------------------------------------------------------------------------------------------+
; Global & Other Fast Signals Summary                                                                        ;
+----------------------------------------------------+-----------------------+---------+---------------------+
; Name                                               ; Location              ; Fan-Out ; Clock Region        ;
+----------------------------------------------------+-----------------------+---------+---------------------+
; sys_pll|iopll_0|stratix10_altera_iopll_i|outclk[0] ; IOPLL_1A              ; 38113   ; Global Clock        ;
; afu_top|port_gasket|user_clock|outclk[0]           ; IOPLL_3A              ; 207444  ; Sectors (0, 0)      ;
+----------------------------------------------------+-----------------------+---------+---------------------+
";

const CONTROL: &str = "\
+-------------------------------------------------------------------------------------------------------+
; Control Signals                                                                                       ;
+-----------------------------------------------------+-----------------------+---------+---------------+
; Name                                                ; Location              ; Fan-Out ; Usage         ;
+-----------------------------------------------------+-----------------------+---------+---------------+
; PCIE_Rst_n                                          ; PIN_AC26              ; 10      ; Async. clear  ;
; afu_top|port_gasket|user_clock|outclk[0]            ; IOPLL_3A              ; 207444  ; Clock         ;
; afu_main|ofs_plat_afu|compute_clock|cclk            ; Unassigned            ; 11013   ; Clock         ;
; afu_main|ofs_plat_afu|compute_clock|fedge           ; Unassigned            ; 4120    ; Clock enable  ;
+-----------------------------------------------------+-----------------------+---------+---------------+
";

#[test]
fn the_delay_the_fitter_added_for_hold_is_read_by_clock_transfer() {
    let hold = parse_hold(HOLD).expect("the section is there");
    assert_eq!(
        hold,
        [
            HoldDelay { source: "clk".into(), destination: "clk".into(), ns: 41707.9 },
            HoldDelay { source: "clk,I/O".into(), destination: "clk".into(), ns: 1795.1 },
        ]
    );
    assert_eq!(parse_hold(FIT_SUMMARY), None, "a report without the section has no hold figures, not zero");
}

#[test]
fn a_clock_the_fitter_left_off_every_global_is_named_with_its_fan_out() {
    let nets = clock_nets(Some(CONTROL), Some(GLOBALS)).expect("both tables");
    assert_eq!(
        nets,
        [
            ClockNet { name: "afu_main|ofs_plat_afu|compute_clock|cclk".into(), fanout: 11013, global: false },
            ClockNet { name: "afu_top|port_gasket|user_clock|outclk[0]".into(), fanout: 207444, global: true },
        ],
        "only clocks, not their enables or clears, off-global first then by fan-out"
    );
    assert_eq!(clock_nets(Some(CONTROL), None), None, "without the globals table nothing says which are global");
    assert_eq!(clock_nets(None, Some(GLOBALS)), None);
}

#[test]
fn a_fit_still_running_is_read_from_its_plan_and_place_reports() {
    let dir = tempfile::tempdir().unwrap();
    let put = |suffix: &str, text: &str| std::fs::write(dir.path().join(format!("afu.{suffix}")), text).unwrap();
    put("flow.rpt", FLOW);
    put("fit.plan.rpt", CONTROL);
    put("fit.place.rpt", GLOBALS);
    let b = read(dir.path(), "afu").unwrap();
    let off: Vec<&str> = b.clock_nets.iter().flatten().filter(|c| !c.global).map(|c| c.name.as_str()).collect();
    assert_eq!(off, ["afu_main|ofs_plat_afu|compute_clock|cclk"]);
    assert_eq!(b.hold, None, "hold delay is reckoned at routing's end");
    put("fit.rpt", &format!("{CONTROL}\n{GLOBALS}\n{HOLD}"));
    let done = read(dir.path(), "afu").unwrap();
    assert_eq!(done.hold.as_ref().map(|h| h.len()), Some(2));
    assert_eq!(done.clock_nets, b.clock_nets);
    let back: Build = serde_json::from_str(&serde_json::to_string(&done).unwrap()).unwrap();
    assert_eq!(back, done);
    let old: Build = serde_json::from_str(&serde_json::to_string(&Build { flow: parse_flow(FLOW), ..Build::default() }).unwrap()).unwrap();
    assert_eq!((old.hold, old.clock_nets), (None, None), "a row recorded before these were read has none");
}

#[test]
fn a_clock_the_fitter_buffered_onto_a_global_rides_that_global() {
    let globals = "\
+------------------------------------------------------------------------------------+
; Global & Other Fast Signals Summary                                                ;
+-----------------------+---------------------+---------+----------------------------+
; Name                  ; Location            ; Fan-Out ; Clock Region               ;
+-----------------------+---------------------+---------+----------------------------+
; clk~FITTER_INSERTED_0 ; LABCELL_X47_Y36_N42 ; 241711  ; Sectors (0, 0) to (8, 11)  ;
+-----------------------+---------------------+---------+----------------------------+
";
    let control = "\
+------------------------------------------------------------+
; Control Signals                                            ;
+-----------------------+------------+---------+-------------+
; Name                  ; Location   ; Fan-Out ; Usage       ;
+-----------------------+------------+---------+-------------+
; clk                   ; PIN_AU33   ; 197064  ; Clock       ;
; clk~FITTER_INSERTEDX  ; Unassigned ; 12      ; Clock       ;
+-----------------------+------------+---------+-------------+
";
    let nets = clock_nets(Some(control), Some(globals)).unwrap();
    assert_eq!(nets.iter().find(|n| n.name == "clk").map(|n| n.global), Some(true), "{nets:?}");
    assert_eq!(nets.iter().find(|n| n.name == "clk~FITTER_INSERTEDX").map(|n| n.global), Some(false), "only a fitter's numbered buffer stands for its net");
}

#[test]
fn a_routed_fit_that_names_no_hold_transfer_added_none() {
    let routed = "\
+--------------------------------+
; Routing Usage Summary          ;
+-----------------------+--------+
; Routing Resource Type ; Usage  ;
+-----------------------+--------+
; Block interconnects   ; 21 / 1 ;
+-----------------------+--------+
";
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("probe.flow.rpt"), FLOW).unwrap();
    std::fs::write(dir.path().join("probe.fit.rpt"), routed).unwrap();
    assert_eq!(read(dir.path(), "probe").unwrap().hold, Some(vec![]), "routed, and nothing added for hold");
    std::fs::write(dir.path().join("probe.fit.rpt"), CONTROL).unwrap();
    assert_eq!(read(dir.path(), "probe").unwrap().hold, None, "not routed: nothing says");
}

#[test]
fn a_synthesis_that_ran_to_the_end_from_one_verilog_and_one_settings_file_is_a_calibration_source() {
    let input = |path: &str| Input { path: path.into(), sha256: format!("sha of {path}") };
    let row = |inputs: &[&str], passed: bool, expected: &[&str]| Row {
        name: "unit".into(),
        recorded: utc(0),
        inputs: inputs.iter().map(|p| input(p)).collect(),
        build: Build::default(),
        verdict: Verdict { passed, expected: expected.iter().map(|s| s.to_string()).collect(), ..Verdict::default() },
        resumed: None,
    };
    let synth = row(&["/b/unit.qsf", "/b/unit.v"], true, &["Synthesis"]);
    let (v, q) = synth.synthesis_sources().expect("a passed synthesis of one Verilog file");
    assert_eq!((v, q), (&input("/b/unit.v"), &input("/b/unit.qsf")), "the Verilog first, each with its hash");
    assert_eq!(row(&["/b/unit.qsf", "/b/unit.v"], false, &["Synthesis"]).synthesis_sources(), None, "a failed one");
    assert_eq!(row(&["/b/unit.qsf", "/b/unit.v"], true, &["Synthesis", "Fitter"]).synthesis_sources(), None, "a fit");
    assert_eq!(row(&["/b/unit.qsf", "/b/a.v", "/b/b.v"], true, &["Synthesis"]).synthesis_sources(), None, "two Verilog files");
    assert_eq!(row(&["/b/unit.v"], true, &["Synthesis"]).synthesis_sources(), None, "no settings file");
    assert_eq!(row(&["/b/unit.qsf", "/b/unit.sv"], true, &["Synthesis"]).synthesis_sources(), None, "no .v file");
}

/// The fit report's region table of a persona that fills most of the PR region (hybrid3b), padding trimmed.
const REGIONS: &str = "\
+---+
; Logic Lock Region Usage Summary ;
+---+
; Statistic ; afu_top|port_gasket|pr_slot|afu_main ;
+---+
; ALMs needed [=A-B+C] ; 532615.2 / 601050 ( 88 % ) ;
; [A] ALMs used in final placement ; 566038.2 / 601050 ( 94 % ) ;
; [B] Estimate of ALMs recoverable by dense packing ; 35606.4 / 601050 ( 5 % ) ;
; [C] Estimate of ALMs unavailable ; 2183.3 / 601050 ( < 1 % ) ;
; ALMs used for memory ; 1000.0 ;
; Combinational ALUTs ; 788659 ;
; Dedicated Logic Registers ; 278195 / 2404200 ( 11 % ) ;
; I/O Registers ; 0 ;
; Block Memory Bits ; 44103024 ;
; M20Ks ; 4386 / 7583 ( 57 % ) ;
; DSP Blocks needed [=A-B] ; 1507 / 3971 ( 37 % ) ;
; [A] DSP Blocks used in final placement ; 1723 / 3971 ( 43 % ) ;
; [B] Estimate of DSPs recoverable by dense merging ; 216 / 3971 ( 5 % ) ;
; Pins ; 0 ;
; IOPLLs ; 0 ;
;  ;  ;
; Region Placement ; (240, 0) to (280, 430) ; (75, 40) to (76, 109) ; (77, 40) to (204, 320) ; (67, 110) to (76, 320) ; (205, 110) to (239, 320) ; (75, 321) to (203, 393) ;
+---+
";

#[test]
fn each_logic_lock_region_keeps_what_it_holds_and_what_it_needs() {
    let regions = parse_region_usage(REGIONS);
    assert_eq!(regions.len(), 1);
    let r = &regions[0];
    assert_eq!(r.name, "afu_top|port_gasket|pr_slot|afu_main");
    assert_eq!((r.alms_needed, r.alms), (532615.2, 601050));
    assert_eq!(r.registers, Some(Ratio { used: 278195, total: 2404200 }));
    assert_eq!(r.m20k, Some(Ratio { used: 4386, total: 7583 }));
    assert_eq!(r.dsp, Some(Ratio { used: 1507, total: 3971 }), "the blocks needed, after dense merging");
    assert_eq!(r.placement.len(), 6, "the six rectangles of the PR region: {:?}", r.placement);
    assert_eq!(r.placement[0], ((240, 0), (280, 430)));
    assert!(parse_region_usage("; nothing ;").is_empty());
}

#[test]
fn a_table_of_two_regions_keeps_each_region_its_own_column() {
    let two = "\
+---+
; Logic Lock Region Usage Summary ;
+---+
; Statistic ; left ; right ;
+---+
; ALMs needed [=A-B+C] ; 10.5 / 100 ( 10 % ) ; 20.0 / 200 ( 10 % ) ;
; Dedicated Logic Registers ; 1 / 400 ( 0 % ) ; 2 / 800 ( 0 % ) ;
; M20Ks ; 3 / 10 ( 30 % ) ; 4 / 20 ( 20 % ) ;
; DSP Blocks needed [=A-B] ; 5 / 6 ( 83 % ) ; 0 / 12 ( 0 % ) ;
; Region Placement ; (1, 2) to (3, 4) ; (5, 6) to (7, 8) ;
+---+
";
    let r = parse_region_usage(two);
    assert_eq!(r.len(), 2);
    assert_eq!((r[0].name.as_str(), r[0].alms_needed, r[0].alms), ("left", 10.5, 100));
    assert_eq!((r[1].name.as_str(), r[1].alms_needed, r[1].alms), ("right", 20.0, 200));
    assert_eq!((r[0].m20k, r[1].dsp), (Some(Ratio { used: 3, total: 10 }), Some(Ratio { used: 0, total: 12 })));
    assert_eq!((r[0].placement.clone(), r[1].placement.clone()), (vec![((1, 2), (3, 4))], vec![((5, 6), (7, 8))]));
}
