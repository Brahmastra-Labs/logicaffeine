//! ★ LOCK — a tier no value of the exact extension can live on refuses a program the extension reaches, by name. ★
//!
//! A value of the exact extension lives in the value runtime a compiled program links: the C backends, linked
//! WebAssembly and RISC-V link it beside the program. A device tier runs its program where no such runtime can be
//! called — a Tensix core, an analog or dataflow fabric, the LOGOS chip, its soft core — so it refuses a program the
//! extension reaches, saying so, rather than failing on some op it does not lower or emitting a kernel that names a
//! runtime it cannot reach. A program the extension never reaches is never refused for it.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::full_asm::compile_to_full_words;
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::program_main_fabric;
use logicaffeine_compile::compile;
use logicaffeine_compile::spatial::rdu_chip::RduChip;

/// A program the extension reaches, in Main.
const EXACT: &str = "## Main\nLet dt be an infinitesimal.\nShow dt * 2.\n";

/// A program the extension reaches, in the region function a spatial tier lowers first.
const EXACT_REGION: &str = "## To nudge (x: Real) -> Real:\n    Let dt be an infinitesimal.\n    Let y: Hyperreal be x + dt.\n    Return the standard part of y.\n\n## Main\nShow nudge(1.5).\n";

/// A program the extension never reaches.
const PLAIN: &str = "## Main\nLet mutable total be 0.\nRepeat for i from 1 to 10:\n    Set total to total + i.\nShow total.\n";

const NAMED: &str = "does not run the exact extension";

fn named(tier: &str, refusal: &str) {
    assert!(
        refusal.contains(NAMED),
        "the {tier} must refuse a program the exact extension reaches by name: {refusal}"
    );
}

fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default()
}

#[test]
fn the_tenstorrent_tier_refuses_an_exact_program_by_name() {
    match compile::compile_to_tenstorrent(EXACT) {
        Ok(_) => panic!("the Tenstorrent tier accepted a program the exact extension reaches"),
        Err(e) => named("Tenstorrent tier", &format!("{e:?}")),
    }
}

#[test]
fn the_analog_tier_refuses_an_exact_program_by_name() {
    match compile::compile_to_analog(EXACT_REGION) {
        Ok(_) => panic!("the analog tier accepted a program the exact extension reaches"),
        Err(e) => named("analog tier", &format!("{e:?}")),
    }
}

#[test]
fn the_plasticine_tier_refuses_an_exact_program_by_name() {
    match compile::compile_to_plasticine(EXACT_REGION) {
        Ok(_) => panic!("the Plasticine tier accepted a program the exact extension reaches"),
        Err(e) => named("Plasticine tier", &format!("{e:?}")),
    }
}

#[test]
fn the_rdu_tier_refuses_an_exact_program_by_name() {
    for chip in RduChip::ALL {
        match compile::compile_to_rdu(EXACT_REGION, chip) {
            Ok(_) => panic!("the RDU tier ({chip:?}) accepted a program the exact extension reaches"),
            Err(e) => named("RDU tier", &format!("{e:?}")),
        }
    }
}

#[test]
fn the_silicon_isa_refuses_an_exact_program_by_name() {
    match compile::compile_to_isa(EXACT) {
        Ok(_) => panic!("the Silicon ISA accepted a program the exact extension reaches"),
        Err(e) => named("Silicon ISA", &format!("{e:?}")),
    }
}

#[test]
fn the_soft_core_refuses_an_exact_program_by_name() {
    match compile_to_full_words(EXACT) {
        Ok(_) => panic!("the soft core accepted a program the exact extension reaches"),
        Err(e) => named("soft core", &e),
    }
}

#[test]
fn the_fabric_refuses_an_exact_program_by_name() {
    match std::panic::catch_unwind(|| program_main_fabric(EXACT)) {
        Ok(_) => panic!("the fabric accepted a program the exact extension reaches"),
        Err(panic) => named("fabric", &panic_text(&*panic)),
    }
}

/// A tier refuses a program for the extension only when the extension reaches it: a plain program compiles, or is
/// refused for a reason of its own.
#[test]
fn a_program_the_extension_never_reaches_is_never_refused_for_it() {
    let reasons: Vec<(&str, String)> = vec![
        ("Tenstorrent tier", compile::compile_to_tenstorrent(PLAIN).err().map(|e| format!("{e:?}")).unwrap_or_default()),
        ("Silicon ISA", compile::compile_to_isa(PLAIN).err().map(|e| format!("{e:?}")).unwrap_or_default()),
        ("soft core", compile_to_full_words(PLAIN).err().unwrap_or_default()),
        (
            "fabric",
            std::panic::catch_unwind(|| program_main_fabric(PLAIN))
                .err()
                .map(|p| panic_text(&*p))
                .unwrap_or_default(),
        ),
    ];
    for (tier, reason) in reasons {
        assert!(!reason.contains(NAMED), "the {tier} refused a program the extension never reaches for it: {reason}");
    }
    assert!(compile::compile_to_tenstorrent(PLAIN).is_ok(), "the Tenstorrent tier refused a plain program");
    assert!(compile::compile_to_isa(PLAIN).is_ok(), "the Silicon ISA refused a plain program");
    assert!(compile_to_full_words(PLAIN).is_ok(), "the soft core refused a plain program");
}
