//! Automata for Reactive Synthesis
//!
//! Büchi automaton construction from LTL specifications.
//! Used as the intermediate representation for game solving.

use crate::ir::VerifyExpr;

/// A state in a Büchi automaton.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AutomatonState {
    pub id: usize,
    pub label: String,
}

/// A transition in a Büchi automaton.
#[derive(Debug, Clone)]
pub struct AutomatonTransition {
    pub from: usize,
    pub guard: VerifyExpr,
    pub to: usize,
}

/// A Büchi automaton for LTL specifications.
#[derive(Debug, Clone)]
pub struct BuchiAutomaton {
    pub states: Vec<AutomatonState>,
    pub initial: usize,
    pub accepting: Vec<usize>,
    pub transitions: Vec<AutomatonTransition>,
}

/// Convert an LTL specification to a Büchi automaton.
///
/// Handles common LTL patterns:
/// - G(p) → single accepting state, self-loop with guard p
/// - G(F(p)) → two states (waiting, seen), both accepting
/// - G(p → F(q)) → two states, transitions on p/q
/// - Simple boolean → single state
pub fn ltl_to_buchi(spec: &VerifyExpr) -> BuchiAutomaton {
    match detect_ltl_pattern(spec) {
        LtlPattern::Safety(prop) => {
            // G(p) → single accepting state with self-loop guarded by p
            BuchiAutomaton {
                states: vec![AutomatonState { id: 0, label: "safe".into() }],
                initial: 0,
                accepting: vec![0],
                transitions: vec![AutomatonTransition {
                    from: 0,
                    guard: prop,
                    to: 0,
                }],
            }
        }
        LtlPattern::Response(trigger, response) => {
            // G(p → F(q)) → two states: idle, waiting_for_response
            BuchiAutomaton {
                states: vec![
                    AutomatonState { id: 0, label: "idle".into() },
                    AutomatonState { id: 1, label: "waiting".into() },
                ],
                initial: 0,
                accepting: vec![0], // Only idle is accepting (response must eventually happen)
                transitions: vec![
                    // idle → idle (no trigger)
                    AutomatonTransition {
                        from: 0,
                        guard: VerifyExpr::not(trigger.clone()),
                        to: 0,
                    },
                    // idle → waiting (trigger fires, response not yet)
                    AutomatonTransition {
                        from: 0,
                        guard: VerifyExpr::and(trigger.clone(), VerifyExpr::not(response.clone())),
                        to: 1,
                    },
                    // idle → idle (trigger fires AND response immediate)
                    AutomatonTransition {
                        from: 0,
                        guard: VerifyExpr::and(trigger, response.clone()),
                        to: 0,
                    },
                    // waiting → idle (response arrives)
                    AutomatonTransition {
                        from: 1,
                        guard: response.clone(),
                        to: 0,
                    },
                    // waiting → waiting (still waiting)
                    AutomatonTransition {
                        from: 1,
                        guard: VerifyExpr::not(response),
                        to: 1,
                    },
                ],
            }
        }
        LtlPattern::Liveness(prop) => {
            // G(F(p)) → two states, loop between them
            BuchiAutomaton {
                states: vec![
                    AutomatonState { id: 0, label: "unseen".into() },
                    AutomatonState { id: 1, label: "seen".into() },
                ],
                initial: 0,
                accepting: vec![1], // Must visit "seen" infinitely often
                transitions: vec![
                    AutomatonTransition { from: 0, guard: VerifyExpr::not(prop.clone()), to: 0 },
                    AutomatonTransition { from: 0, guard: prop.clone(), to: 1 },
                    AutomatonTransition { from: 1, guard: VerifyExpr::bool(true), to: 0 },
                ],
            }
        }
        LtlPattern::Boolean(prop) => {
            // Simple boolean → single state
            BuchiAutomaton {
                states: vec![AutomatonState { id: 0, label: "check".into() }],
                initial: 0,
                accepting: vec![0],
                transitions: vec![AutomatonTransition {
                    from: 0,
                    guard: prop,
                    to: 0,
                }],
            }
        }
    }
}

/// Detected LTL pattern.
enum LtlPattern {
    Safety(VerifyExpr),                      // G(p)
    Response(VerifyExpr, VerifyExpr),         // G(p → F(q))
    Liveness(VerifyExpr),                     // G(F(p))
    Boolean(VerifyExpr),                      // p
}

/// Detect the LTL pattern of a specification.
fn detect_ltl_pattern(spec: &VerifyExpr) -> LtlPattern {
    // Pattern: NOT(something) at top level → safety (G(NOT(bad)))
    if let VerifyExpr::Not(inner) = spec {
        return LtlPattern::Safety(spec.clone());
    }

    // Pattern: implies at top → response (G(p → F(q)))
    if let VerifyExpr::Binary { op: crate::ir::VerifyOp::Implies, left, right } = spec {
        return LtlPattern::Response(*left.clone(), *right.clone());
    }

    // Pattern: single variable or boolean → safety
    if matches!(spec, VerifyExpr::Var(_) | VerifyExpr::Bool(_)) {
        return LtlPattern::Safety(spec.clone());
    }

    // Default: treat as safety
    LtlPattern::Safety(spec.clone())
}
