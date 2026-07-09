//! DIMACS CNF parsing and printing — the SAT-competition interchange format, and the front
//! door for running arbitrary instances through the solver.
//!
//! The format is a header `p cnf <num_vars> <num_clauses>`, optional `c …` comment lines,
//! then a stream of signed integers: each clause is terminated by `0`, a clause may span
//! several lines, and several clauses may share a line. We parse **fail-closed** — a missing
//! or garbled header, a literal naming a variable beyond `num_vars`, a non-integer token, or
//! a final clause with no terminating `0` is a typed error, never a quietly mangled formula.
//!
//! The header clause COUNT is deliberately advisory: real instances miscount and competition
//! solvers tolerate it, so we keep the clauses we actually read rather than rejecting on a
//! count mismatch. The variable COUNT, by contrast, bounds the literals and is enforced.

use crate::cdcl::{Lit, Solver};

/// A parsed CNF formula: `num_vars` variables and clauses over packed [`Lit`]s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DimacsCnf {
    pub num_vars: usize,
    pub clauses: Vec<Vec<Lit>>,
}

/// Why a DIMACS input was rejected. Better a typed error than a silently wrong formula.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DimacsError {
    /// Clause data appeared before any `p cnf …` header (or no header at all).
    MissingHeader,
    /// A header line was present but was not `p cnf <nat> <nat>`.
    MalformedHeader(String),
    /// A literal named `var`, whose magnitude exceeds the declared `num_vars`.
    VarOutOfRange { var: i64, num_vars: usize },
    /// A non-integer token appeared in the clause body.
    InvalidToken(String),
    /// The input ended mid-clause: literals with no terminating `0`.
    UnterminatedClause,
}

impl DimacsCnf {
    /// Load the clauses into a fresh [`Solver`] over `num_vars` variables.
    pub fn into_solver(&self) -> Solver {
        let mut s = Solver::new(self.num_vars);
        for c in &self.clauses {
            s.add_clause(c.clone());
        }
        s
    }
}

/// Parse a DIMACS CNF string. See the module docs for the fail-closed contract.
pub fn parse(input: &str) -> Result<DimacsCnf, DimacsError> {
    let mut header: Option<usize> = None;
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    let mut current: Vec<Lit> = Vec::new();

    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('c') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("p ") {
            if header.is_some() {
                return Err(DimacsError::MalformedHeader(line.to_string()));
            }
            let mut it = rest.split_whitespace();
            match (it.next(), it.next(), it.next(), it.next()) {
                (Some("cnf"), Some(nv), Some(nc), None) => {
                    let num_vars =
                        nv.parse::<usize>().map_err(|_| DimacsError::MalformedHeader(line.to_string()))?;
                    nc.parse::<usize>().map_err(|_| DimacsError::MalformedHeader(line.to_string()))?;
                    header = Some(num_vars);
                }
                _ => return Err(DimacsError::MalformedHeader(line.to_string())),
            }
            continue;
        }
        // A clause-body line: a header must already have been seen.
        let num_vars = header.ok_or(DimacsError::MissingHeader)?;
        for tok in line.split_whitespace() {
            let n: i64 = tok.parse().map_err(|_| DimacsError::InvalidToken(tok.to_string()))?;
            if n == 0 {
                clauses.push(std::mem::take(&mut current));
            } else {
                let v = n.unsigned_abs();
                if v as usize > num_vars {
                    return Err(DimacsError::VarOutOfRange { var: n, num_vars });
                }
                current.push(Lit::new((v - 1) as u32, n > 0));
            }
        }
    }

    let num_vars = header.ok_or(DimacsError::MissingHeader)?;
    if !current.is_empty() {
        return Err(DimacsError::UnterminatedClause);
    }
    Ok(DimacsCnf { num_vars, clauses })
}

/// Render a [`DimacsCnf`] back to DIMACS text. `parse(print(x)) == x`.
pub fn print(cnf: &DimacsCnf) -> String {
    let mut out = format!("p cnf {} {}\n", cnf.num_vars, cnf.clauses.len());
    for clause in &cnf.clauses {
        for l in clause {
            let v = (l.var() + 1) as i64;
            out.push_str(&(if l.is_positive() { v } else { -v }).to_string());
            out.push(' ');
        }
        out.push_str("0\n");
    }
    out
}
