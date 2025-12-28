//! Core Type Definitions (Spec 3.2)

pub type Nat = u64;
pub type Int = i64;
pub type Real = f64;
pub type Text = String;
pub type Bool = bool;
pub type Unit = ();

// Phase 30: Collections
pub type Seq<T> = Vec<T>;
