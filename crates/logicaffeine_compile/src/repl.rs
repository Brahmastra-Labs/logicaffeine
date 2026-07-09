//! `ReplSession` — the replay-based interactive session behind `largo repl`
//! (and any future notebook surface).
//!
//! # Architecture: replay, not a persistent interpreter
//!
//! The session accumulates source text — `## ` definition blocks and Main
//! statements — and on every [`eval`](ReplSession::eval) composes them into
//! a complete program and re-runs it through
//! [`interpret_for_ui_with_args`](crate::interpret_for_ui_with_args): the
//! exact engine behind `largo run --interpret` (VM+JIT, prelude
//! auto-import, the optimizer, and the debug shadow oracle). Only output
//! lines past a high-water mark are surfaced, so each eval appears
//! incremental. This gives zero semantic divergence between the REPL and
//! real programs by construction, and makes [`source`](ReplSession::source)
//! a valid, runnable `.lg` program at every moment (`:save` = a program).
//!
//! A failing input **rolls back**: the offending chunk is popped and the
//! high-water mark stays put, so the session never wedges and never
//! duplicates output.
//!
//! # The replay caveat
//!
//! Re-running the whole program re-executes side effects. Deterministic
//! programs are unaffected (their prior output is suppressed by the
//! high-water mark), but non-deterministic operations — current time,
//! random numbers, file or network I/O — are re-evaluated on every eval,
//! so a binding like `Let t be the current time.` drifts across lines.
//! [`reset`](ReplSession::reset) is the escape hatch; a seeded replay mode
//! is the growth path.

/// The outcome of one REPL evaluation.
#[derive(Debug, Default)]
pub struct ReplOutcome {
    /// Output lines newly produced by this eval (past the high-water mark).
    pub new_lines: Vec<String>,
    /// The error message, if the input failed (the input was rolled back).
    pub error: Option<String>,
}

/// A persistent imperative REPL session. See the module docs for the
/// replay architecture.
#[derive(Debug, Default)]
pub struct ReplSession {
    /// Accumulated `## ` definition blocks (functions, types, policies…).
    defs: Vec<String>,
    /// Accumulated Main-body statement chunks.
    stmts: Vec<String>,
    /// Output lines already surfaced (the high-water mark).
    emitted: usize,
    /// The argv the program's `args()` sees; index 0 is the program name.
    argv: Vec<String>,
}

impl ReplSession {
    /// A fresh session.
    pub fn new() -> Self {
        Self { defs: Vec::new(), stmts: Vec::new(), emitted: 0, argv: vec!["repl".to_string()] }
    }

    /// Evaluate one input chunk (a statement, a multi-statement chunk, or a
    /// `## ` definition block), returning the new output lines.
    pub async fn eval(&mut self, input: &str) -> ReplOutcome {
        let trimmed = input.trim();
        if trimmed.is_empty() || trimmed == "## Main" {
            return ReplOutcome::default();
        }
        let is_def = trimmed.starts_with("## ");
        if is_def {
            self.defs.push(trimmed.to_string());
        } else {
            self.stmts.push(trimmed.to_string());
        }

        let program = self.source();
        let result = crate::interpret_for_ui_with_args(&program, &self.argv).await;
        let new_lines: Vec<String> =
            result.lines.get(self.emitted..).map(|s| s.to_vec()).unwrap_or_default();
        match result.error {
            Some(err) => {
                // Roll back: the reverted program deterministically reproduces
                // the old output prefix, so the mark must not advance.
                if is_def {
                    self.defs.pop();
                } else {
                    self.stmts.pop();
                }
                ReplOutcome { new_lines, error: Some(err) }
            }
            None => {
                self.emitted = result.lines.len();
                ReplOutcome { new_lines, error: None }
            }
        }
    }

    /// Synchronous [`eval`](Self::eval) for native callers.
    pub fn eval_sync(&mut self, input: &str) -> ReplOutcome {
        futures::executor::block_on(self.eval(input))
    }

    /// The session as a complete LOGOS program: definitions first, then the
    /// Main body — always runnable as-is.
    pub fn source(&self) -> String {
        let mut out = String::new();
        for def in &self.defs {
            out.push_str(def);
            out.push_str("\n\n");
        }
        out.push_str("## Main\n\n");
        for stmt in &self.stmts {
            out.push_str(stmt);
            out.push('\n');
        }
        out
    }

    /// The session's global bindings as `(name, type, value)` rows, sorted
    /// by name. Unavailable (empty) for concurrent programs — the
    /// inspection replay runs on the synchronous tree-walker.
    ///
    /// NOTE: this **re-executes** the accumulated program (the replay
    /// architecture) — callers that only need names for completion must use
    /// [`binding_names`](Self::binding_names), which never executes.
    pub fn vars(&self) -> Vec<(String, String, String)> {
        crate::ui_bridge::repl_global_bindings(&self.source(), &self.argv).unwrap_or_default()
    }

    /// The names this session binds — a pure textual scan (`Let <name> be`,
    /// `Set <name> to`, `## To <name>`) with **zero execution**, safe to call
    /// after every keystroke for completion feeds.
    pub fn binding_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        let mut push = |name: &str| {
            let name = name.trim();
            if !name.is_empty() && !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        };
        for chunk in self.defs.iter().chain(self.stmts.iter()) {
            for line in chunk.lines() {
                let t = line.trim_start();
                if let Some(rest) = t.strip_prefix("Let ") {
                    if let Some((name, _)) = rest.split_once(" be") {
                        push(name);
                    }
                } else if let Some(rest) = t.strip_prefix("## To ") {
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    push(&name);
                }
            }
        }
        names
    }

    /// Replace the session with the contents of a saved program (the
    /// inverse of [`source`](Self::source)). Returns the program's full
    /// output as the outcome.
    pub fn load_source(&mut self, src: &str) -> ReplOutcome {
        self.reset();

        // A file with no `## ` headers at all is a bare statement list —
        // treat the whole thing as the Main body (mirroring the engine's
        // implicit-main convention) instead of silently loading nothing.
        let headerless = !src.lines().any(|l| l.starts_with("## "));

        let mut current_def: Option<String> = None;
        let mut main_body = String::new();
        let mut in_main = headerless;
        for line in src.lines() {
            if line.trim_end() == "## Main" {
                if let Some(def) = current_def.take() {
                    self.defs.push(def.trim().to_string());
                }
                in_main = true;
            } else if line.starts_with("## ") {
                if let Some(def) = current_def.take() {
                    self.defs.push(def.trim().to_string());
                }
                in_main = false;
                current_def = Some(format!("{line}\n"));
            } else if in_main {
                main_body.push_str(line);
                main_body.push('\n');
            } else if let Some(def) = current_def.as_mut() {
                def.push_str(line);
                def.push('\n');
            }
        }
        if let Some(def) = current_def.take() {
            self.defs.push(def.trim().to_string());
        }
        let main_body = main_body.trim().to_string();
        if !main_body.is_empty() {
            self.stmts.push(main_body);
        }

        let program = self.source();
        let result =
            futures::executor::block_on(crate::interpret_for_ui_with_args(&program, &self.argv));
        match result.error {
            Some(err) => {
                self.reset();
                ReplOutcome { new_lines: Vec::new(), error: Some(err) }
            }
            None => {
                self.emitted = result.lines.len();
                ReplOutcome { new_lines: result.lines, error: None }
            }
        }
    }

    /// Clear the whole session: definitions, statements, and the output
    /// ledger.
    pub fn reset(&mut self) {
        self.defs.clear();
        self.stmts.clear();
        self.emitted = 0;
    }
}
