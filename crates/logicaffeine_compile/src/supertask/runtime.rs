//! The run-time half of the supertask engine on the interpreter's values: the builtins the
//! statements it writes call (`tripCount`, `firstExit`, `affinePower`, `polynomialFlow`, `cycleAt`,
//! `stateSize`, `searchCandidates`). Each runs [`logicaffeine_base::supertask`], generic over a value model, so a
//! compiled program computes exactly what the interpreter does.

use logicaffeine_base::supertask;

use crate::interpreter::RuntimeValue;

/// How many times a loop stepped by a clock runs; see [`supertask::trip_count`].
pub fn trip_count(
    t0: &RuntimeValue,
    step: &RuntimeValue,
    bound: &RuntimeValue,
    relation: &str,
) -> Result<RuntimeValue, String> {
    supertask::trip_count(t0, step, bound, relation)
}

/// `Aᴺ·x`; see [`supertask::affine_power`].
pub fn affine_power(
    matrix: &RuntimeValue,
    vector: &RuntimeValue,
    n: &RuntimeValue,
) -> Result<RuntimeValue, String> {
    supertask::affine_power(matrix, vector, n)
}

/// `N` Euler steps of a polynomial differential equation; see [`supertask::polynomial_flow`].
pub fn polynomial_flow(
    field: &RuntimeValue,
    vector: &RuntimeValue,
    n: &RuntimeValue,
) -> Result<RuntimeValue, String> {
    supertask::polynomial_flow(field, vector, n)
}

/// The state at step `n` of a recorded run that has entered a cycle; see [`supertask::cycle_at`].
pub fn cycle_at(
    states: &RuntimeValue,
    repeated: &RuntimeValue,
    n: &RuntimeValue,
) -> Result<RuntimeValue, String> {
    supertask::cycle_at(states, repeated, n)
}

/// The bits a state occupies; see [`supertask::state_size`].
pub fn state_size(v: &RuntimeValue) -> u64 {
    supertask::state_size(v)
}

/// How many passes a loop makes whose guard compares terms linear in a counter it steps; see
/// [`supertask::first_exit`].
pub fn first_exit(t0: &RuntimeValue, step: &RuntimeValue, guard: &RuntimeValue) -> Result<RuntimeValue, String> {
    supertask::first_exit(t0, step, guard)
}

/// The naturals a search over every natural must examine; see [`supertask::search_candidates`].
pub fn search_candidates(atoms: &RuntimeValue, moduli: &RuntimeValue) -> Result<RuntimeValue, String> {
    supertask::search_candidates(atoms, moduli)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/supertask/runtime/tests.rs"]
mod tests;
