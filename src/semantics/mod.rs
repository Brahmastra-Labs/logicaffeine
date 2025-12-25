mod axioms;

pub use axioms::apply_axioms;

include!(concat!(env!("OUT_DIR"), "/axiom_data.rs"));
