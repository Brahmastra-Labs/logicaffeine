//! ★ LOCK — every `UiStmt` verb has exactly ONE execution site: `Interpreter::execute_ui`. Both
//! statement executors — the async tree-walker's `execute_stmt` and its sync twin
//! `execute_stmt_sync` — dispatch every `Stmt::Ui(..)` to it with one delegating arm rather than
//! each carrying its own ~320-line copy of the verb match. Two independent copies of the same
//! ~30 verbs is exactly the shape that let the `Return.`/`Break` control-flow defect
//! (`return_control_flow_lock.rs`) hide: a fix landing in one copy has no guarantee of reaching
//! the other. This census reads the SOURCE, so a verb match reintroduced inline in either
//! executor fails it even though the program still compiles and runs. ★

#[test]
fn every_ui_verb_has_one_execution_site() {
    let src = include_str!("../../logicaffeine_compile/src/interpreter.rs");

    assert_eq!(
        src.matches("fn execute_ui(").count(),
        1,
        "exactly one `execute_ui` — the single execution site for every `UiStmt` verb"
    );

    let delegate = "Stmt::Ui(ui) => self.execute_ui(ui),";
    assert_eq!(
        src.matches(delegate).count(),
        2,
        "both statement executors — the async one and its sync twin — must dispatch every \
         `UiStmt` verb to `execute_ui` with this exact one-line arm, never a verb match of \
         their own"
    );

    // Every `UiStmt` variant is named by path EXACTLY ONCE in this file — its one arm inside
    // `execute_ui` — and nothing else is: a verb match reintroduced inline in either executor
    // names a variant twice, a verb quietly dropped from `execute_ui` names it zero times. The
    // expectation is read from the enum itself, so a new verb is held the moment it is declared.
    let catalogue = include_str!("../../logicaffeine_language/src/ast/stmt.rs");
    let variants = ui_stmt_variants(catalogue);
    assert!(
        variants.len() >= 25,
        "the UiStmt enum was read: {variants:?}"
    );
    let prefix = "crate::ast::stmt::UiStmt::";
    let mut named: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for (at, _) in src.match_indices(prefix) {
        let rest = &src[at + prefix.len()..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        *named.entry(name).or_default() += 1;
    }
    let wrong: Vec<String> = variants
        .iter()
        .filter(|v| named.get(*v).copied().unwrap_or(0) != 1)
        .map(|v| format!("`{v}` named {} time(s)", named.get(v).copied().unwrap_or(0)))
        .chain(
            named
                .keys()
                .filter(|n| !variants.contains(n))
                .map(|n| format!("`{n}` is no UiStmt variant")),
        )
        .collect();
    assert!(
        wrong.is_empty(),
        "every `UiStmt` variant must be matched exactly once, inside `execute_ui` — a second \
         execution site (or a dropped one) breaks that:\n{}",
        wrong.join("\n")
    );
}

/// The variant names of `pub enum UiStmt` in the AST source, in declaration order.
fn ui_stmt_variants(source: &str) -> Vec<String> {
    let start = source
        .find("pub enum UiStmt")
        .expect("the AST declares UiStmt");
    let body = &source[start..];
    let end = body.find("\n}").expect("the UiStmt enum closes");
    body[..end]
        .lines()
        .skip(1)
        .filter(|line| line.starts_with("    ") && !line.starts_with("     "))
        .filter_map(|line| {
            let name: String = line
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            name.chars()
                .next()
                .filter(char::is_ascii_uppercase)
                .map(|_| name)
        })
        .collect()
}
