use super::*;

/// The device model and harness emitters are well-formed and self-checking.
#[test]
fn computedram_rtl_emitters_are_well_formed() {
    let m = emit_computedram_model_verilog();
    assert!(m.contains("module computedram_model"), "device model");
    assert!(
        m.contains("RowClone") && m.contains("maj3"),
        "RowClone + triple-row majority"
    );
    let tb = emit_computedram_harness_testbench(4, 10, 20, 0xDEAD_BEEF_CAFE_F00D);
    assert!(tb.contains("module tb_computedram"), "harness");
    assert!(
        tb.contains("dram_executor exec"),
        "drives the proven executor"
    );
    assert!(tb.contains("computedram_model"), "into the device");
    assert!(
        tb.contains("pre_addr=8'd10") && tb.contains("rd_addr=8'd20"),
        "src/dst baked"
    );
    assert!(tb.contains("deadbeefcafef00d"), "pattern baked");
    assert!(tb.contains("PASS:"), "reports PASS");
    // The reduction harness proves in-DRAM AND/OR against the oracle.
    let rt = emit_computedram_reduce_testbench(14, 1, 2, 3, 4, 5, 6, 0xF0F0, 0xFF00);
    assert!(
        rt.contains("module tb_computedram_reduce"),
        "reduce harness"
    );
    assert!(
        rt.contains("in-DRAM AND") && rt.contains("in-DRAM OR"),
        "checks AND and OR"
    );
    assert!(
        rt.contains(&format!("{:016x}", 0xF0F0u64 & 0xFF00u64)),
        "AND oracle baked"
    );
    assert!(
        rt.contains(&format!("{:016x}", 0xF0F0u64 | 0xFF00u64)),
        "OR oracle baked"
    );
}
