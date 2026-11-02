//! ★ ANALOG ON-CHIP LOCK (hardware-gated) ★ — the ONLY place `measured:true` can ever
//! originate. There is no analog silicon of ours yet: this test is `#[ignore]`d and refuses to
//! run without the lab environment, so nothing here can fabricate an on-silicon result.
//!
//! When first silicon exists: point `ANALOG_CHIP_PORT` at the characterization rig, and this
//! lock drives the loopback paths (DAC → tile → ADC), compares against the behavioral tiers
//! within the artifact's budget, and writes the signoff that flips `measured` flags — exactly
//! the `record-sky130-signoff.py` discipline.

#[test]
#[ignore = "requires first analog silicon + a characterization rig (ANALOG_CHIP_PORT)"]
fn first_silicon_measures_inside_the_budget() {
    // Hardware-gated tests SKIP CLEAN when the hardware is absent (the same discipline as
    // the GPU-gated locks): the full-suite runner executes `#[ignore]`d tests, and a missing
    // rig is the expected state until first silicon — not a failure to report.
    let Ok(port) = std::env::var("ANALOG_CHIP_PORT") else {
        eprintln!(
            "SKIP: no characterization rig (set ANALOG_CHIP_PORT to its serial port) — \
             first analog silicon has not met this lock yet"
        );
        return;
    };
    unimplemented!("drive the loopback battery on {port} and record the signoff");
}
