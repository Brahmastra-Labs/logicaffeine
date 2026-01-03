mod axioms;
mod kripke;

pub use axioms::apply_axioms;
pub use kripke::apply_kripke_lowering;

include!(concat!(env!("OUT_DIR"), "/axiom_data.rs"));
