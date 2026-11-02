use super::*;

/// The user macro is a Wishbone slave that instantiates the CERTIFIED command player unchanged
/// and exposes the standard Caravel signals a real `user_project_wrapper` binds. A missing
/// Wishbone port or a mis-instantiated executor (the integration bug) is caught here.
#[test]
fn the_wishbone_user_macro_wraps_the_certified_command_player() {
    let v = emit_dram_executor_wb_verilog();
    assert!(
        v.contains("module dram_executor_wb"),
        "must define the Wishbone user macro"
    );
    assert!(
        v.contains("module dram_executor"),
        "must inline the certified command player"
    );
    assert_eq!(
        v.matches("dram_executor u_exec").count(),
        1,
        "instantiate the executor once"
    );
    for port in [
        "wb_clk_i",
        "wb_rst_i",
        "wbs_stb_i",
        "wbs_cyc_i",
        "wbs_we_i",
        "wbs_sel_i",
        "wbs_adr_i",
        "wbs_dat_i",
        "wbs_ack_o",
        "wbs_dat_o",
        "la_data_out",
    ] {
        assert!(
            v.contains(port),
            "the Caravel user interface must expose {port}"
        );
    }
    // The plan word is written over Wishbone and launched — the executor is genuinely driven.
    assert!(
        v.contains(".cmd_word(cmd_word_r)") && v.contains(".cmd_valid(cmd_valid_r)"),
        "the executor must be fed the Wishbone-written plan word + launch"
    );
}

/// The Caravel `user_project_wrapper` exposes the golden interface VERBATIM and instantiates our
/// macro as `mprj`, threading Wishbone + logic-analyzer into the certified command player, with
/// the macro carrying the `USE_POWER_PINS` hooks (vccd1/vssd1) the fixed-size wrapper PDN needs.
#[test]
fn the_user_project_wrapper_instantiates_our_macro_on_the_golden_interface() {
    let w = emit_user_project_wrapper_verilog();
    assert!(
        w.contains("module user_project_wrapper"),
        "must be the Caravel wrapper"
    );
    assert_eq!(
        w.matches("dram_user_project mprj").count(),
        1,
        "instantiate the full-interface macro as mprj once"
    );
    // The golden Caravel user interface must be present verbatim.
    for port in [
        "wb_clk_i",
        "wbs_stb_i",
        "wbs_dat_o",
        "la_data_in",
        "la_data_out",
        "io_in",
        "io_out",
        "io_oeb",
        "user_irq",
        "analog_io",
    ] {
        assert!(w.contains(port), "the golden wrapper must expose {port}");
    }
    // EVERY user pin is wired 1:1 into the macro (the passthrough that makes the wrapper
    // LVS-clean) — not just the Wishbone subset.
    for conn in [
        ".la_data_in(la_data_in)",
        ".io_in(io_in)",
        ".io_out(io_out)",
        ".io_oeb(io_oeb)",
        ".user_irq(user_irq)",
        ".user_clock2(user_clock2)",
    ] {
        assert!(
            w.contains(conn),
            "the wrapper must connect {conn} 1:1 for a clean LVS"
        );
    }
    // Power pins are threaded under USE_POWER_PINS so the fixed-size wrapper PDN can hook them.
    assert!(
        w.contains("`ifdef USE_POWER_PINS")
            && w.contains(".vccd1(vccd1)")
            && w.contains(".vssd1(vssd1)"),
        "the macro must expose Caravel power hooks under USE_POWER_PINS"
    );
    // The full-interface macro carries the guarded power pins AND wraps the certified core.
    let m = emit_dram_user_project_verilog();
    assert!(
        m.contains("module dram_user_project") && m.contains("dram_executor_wb core"),
        "the full-interface macro must wrap the certified Wishbone core"
    );
    assert!(
        m.contains("`ifdef USE_POWER_PINS"),
        "the macro must carry USE_POWER_PINS hooks for Caravel PDN"
    );
}

/// Emit the user macro + its self-checking Wishbone testbench into benchmarks/silicon/
/// (golden; regenerate with CARAVEL_RECORD=1). iverilog PASS is wired into synth-soft-core.sh;
/// the block also hardens through OpenLane + the golden DRC (golden_precheck / sky130 locks).
#[test]
fn caravel_user_macro_artifacts_are_current() {
    let rtl = emit_dram_executor_wb_verilog();
    let tb = emit_dram_executor_wb_testbench();
    let user_macro = emit_dram_user_project_verilog();
    let wrapper = emit_user_project_wrapper_verilog();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon");
    let files = [
        ("dram_executor_wb.v", rtl),
        ("tb_dram_executor_wb.v", tb),
        ("dram_user_project.v", user_macro),
        ("user_project_wrapper.v", wrapper),
    ];
    if std::env::var("CARAVEL_RECORD").is_ok() {
        for (name, body) in &files {
            std::fs::write(dir.join(name), body).expect("write artifact");
        }
        return;
    }
    for (name, body) in &files {
        let path = dir.join(name);
        let got = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!("missing {} ({e}) — record: CARAVEL_RECORD=1 cargo test caravel_user_macro_artifacts", path.display())
        });
        assert_eq!(
            &got, body,
            "{name} is stale — regenerate with CARAVEL_RECORD=1"
        );
    }
}
