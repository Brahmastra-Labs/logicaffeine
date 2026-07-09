//! Human-readable rendering of a compile result: tokens, AST tree, FOL, readings.

use crate::count_nodes;
use logicaffeine_compile::ui_bridge::{AstNode, CompileResult};

fn render_ast(node: &AstNode, prefix: &str, last: bool, out: &mut String) {
    let branch = if last { "└─ " } else { "├─ " };
    out.push_str(&format!("{prefix}{branch}{} [{}]\n", node.label, node.node_type));
    let child_prefix = format!("{prefix}{}", if last { "   " } else { "│  " });
    let count = node.children.len();
    for (i, child) in node.children.iter().enumerate() {
        render_ast(child, &child_prefix, i + 1 == count, out);
    }
}

/// Verbose per-sentence dump used by both `wiki-trace` and the triage harness.
pub fn render_trace(n: usize, sentence: &str, result: &CompileResult) -> String {
    let (other_nodes, total_nodes) = result.ast.as_ref().map(count_nodes).unwrap_or((0, 0));

    let mut s = String::new();
    s.push_str(&format!("═══ Sentence {n} ═══\n"));
    s.push_str(&format!("INPUT: {sentence}\n\n"));

    s.push_str("TOKENS:\n");
    for t in &result.tokens {
        s.push_str(&format!(
            "  {:>3}..{:<3} {:<12} {:?}\n",
            t.start,
            t.end,
            format!("\"{}\"", t.text),
            t.category
        ));
    }
    s.push('\n');

    match &result.error {
        Some(err) => {
            s.push_str("ERROR (Socratic):\n");
            for line in err.lines() {
                s.push_str(&format!("  {line}\n"));
            }
            s.push('\n');
        }
        None => {
            s.push_str("FOL:\n");
            s.push_str(&format!("  unicode: {}\n", result.logic.as_deref().unwrap_or("")));
            s.push_str(&format!("  simple : {}\n", result.simple_logic.as_deref().unwrap_or("")));
            s.push_str(&format!("  kripke : {}\n\n", result.kripke_logic.as_deref().unwrap_or("")));

            s.push_str(&format!("AST ({total_nodes} nodes, {other_nodes} unhandled):\n"));
            if let Some(ast) = &result.ast {
                render_ast(ast, "  ", true, &mut s);
            }
            s.push('\n');

            if result.readings.len() > 1 {
                s.push_str(&format!("AMBIGUITY READINGS ({}):\n", result.readings.len()));
                for (j, r) in result.readings.iter().enumerate() {
                    s.push_str(&format!("  [{}] {r}\n", j + 1));
                }
                s.push('\n');
            }
        }
    }
    s
}
