//! Plan a Caravel shuttle: pack the signoff-clean macros into the sky130 user area and print each placement
//! (position and bus base address), or the refusal when they do not fit.
//!
//!   cargo run -q -p logicaffeine-compile --example plan_shuttle

fn main() {
    use logicaffeine_compile::codegen_sva::pdk_target::SKY130A;
    use logicaffeine_compile::codegen_sva::shuttle_wrapper::*;
    let area = SKY130A.user_area.expect("caravel");
    let tot: f64 = CLEAN_MACROS.iter().map(|m| m.area_mm2()).sum();
    println!(
        "  {} macros, {:.3} mm2 of {:.3} mm2 ({:.0}%)",
        CLEAN_MACROS.len(),
        tot,
        area.mm2(),
        tot / area.mm2() * 100.0
    );
    match plan_shuttle(CLEAN_MACROS, area) {
        Ok(p) => {
            println!("  PACKS: {} placements", p.len());
            for q in p.iter().take(6) {
                println!(
                    "    {:26} @ ({:6.0},{:6.0})  base 0x{:08X}",
                    q.name, q.x_um, q.y_um, q.base
                );
            }
            println!("    ...");
            let maxy = p.iter().map(|q| q.y_um).fold(0.0f64, f64::max);
            println!(
                "  tallest shelf top ~{:.0} um of {:.0}",
                maxy, area.height_um
            );
        }
        Err(e) => println!("  REFUSED: {e:?}"),
    }
}
