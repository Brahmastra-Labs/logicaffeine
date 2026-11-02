use super::*;

#[test]
fn control_round_trips_are_bit_exact() {
    // apply(decode(hi)) must restore exactly the control bits for arbitrary highs.
    for &hi in &[
        0x000fc400078e00ffu64,
        0x000e620000002100,
        0x001fce0000000000,
        0xffffffffffffffff,
        0,
    ] {
        let c = Control::decode(hi);
        let clear: u64 = ((1u64 << (61 - 41 + 1)) - 1) << 41;
        // the non-control bits are untouched; the control bits equal what we decoded.
        assert_eq!(
            c.apply(hi) & !clear,
            hi & !clear,
            "non-control bits must be preserved"
        );
        assert_eq!(
            Control::decode(c.apply(hi)),
            c,
            "decode∘apply is identity on control fields"
        );
    }
}

#[test]
fn known_ampere_control_fields_decode_sanely() {
    // The first instruction of our GEMM kernel (verified vs nvdisasm): IMAD.MOV, stall 2, no barriers.
    let c = Control::decode(0x000fc400078e00ff);
    assert_eq!(c.stall, 2);
    assert_eq!(c.wr_barrier, 7, "pure ALU op sets no write barrier");
    assert_eq!(c.rd_barrier, 7);
    // S2R SR_TID.X (variable latency): sets write barrier 1.
    let s2r = Control::decode(0x000e620000002100);
    assert_eq!(
        s2r.wr_barrier, 1,
        "S2R must set a write barrier (its result is not immediately ready)"
    );
}
