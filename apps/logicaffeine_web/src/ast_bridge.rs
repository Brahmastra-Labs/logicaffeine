//! AST Bridge - Converts language AST types to UI display structures
//!
//! This module provides types and functions for displaying AST trees in the UI.

use logicaffeine_language::ast::{self, LogicExpr, Term};
use logicaffeine_language::token::TokenType;
use logicaffeine_base::Interner;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AstNode {
    pub label: String,
    pub node_type: String,
    pub children: Vec<AstNode>,
}

impl AstNode {
    pub fn leaf(label: &str, node_type: &str) -> Self {
        AstNode {
            label: label.to_string(),
            node_type: node_type.to_string(),
            children: Vec::new(),
        }
    }

    pub fn with_children(label: &str, node_type: &str, children: Vec<AstNode>) -> Self {
        AstNode {
            label: label.to_string(),
            node_type: node_type.to_string(),
            children,
        }
    }
}

pub fn expr_to_ast_node(expr: &LogicExpr, interner: &Interner) -> AstNode {
    match expr {
        LogicExpr::Predicate { name, args, .. } => {
            let name_str = interner.resolve(*name);
            let arg_nodes: Vec<AstNode> = args.iter()
                .map(|t| term_to_ast_node(t, interner))
                .collect();
            AstNode::with_children(
                &format!("{}({})", name_str, args.len()),
                "predicate",
                arg_nodes,
            )
        }
        LogicExpr::Quantifier { kind, variable, body, .. } => {
            let var_str = interner.resolve(*variable);
            let symbol = match kind {
                ast::QuantifierKind::Universal => "∀",
                ast::QuantifierKind::Existential => "∃",
                ast::QuantifierKind::Most => "MOST",
                ast::QuantifierKind::Few => "FEW",
                ast::QuantifierKind::Many => "MANY",
                ast::QuantifierKind::Cardinal(n) => return AstNode::with_children(
                    &format!("∃={}{}", n, var_str),
                    "quantifier",
                    vec![expr_to_ast_node(body, interner)],
                ),
                ast::QuantifierKind::AtLeast(n) => return AstNode::with_children(
                    &format!("∃≥{}{}", n, var_str),
                    "quantifier",
                    vec![expr_to_ast_node(body, interner)],
                ),
                ast::QuantifierKind::AtMost(n) => return AstNode::with_children(
                    &format!("∃≤{}{}", n, var_str),
                    "quantifier",
                    vec![expr_to_ast_node(body, interner)],
                ),
                ast::QuantifierKind::Generic => "GEN",
            };
            AstNode::with_children(
                &format!("{}{}", symbol, var_str),
                "quantifier",
                vec![expr_to_ast_node(body, interner)],
            )
        }
        LogicExpr::BinaryOp { left, op, right } => {
            let op_str = match op {
                TokenType::And => "∧",
                TokenType::Or => "∨",
                TokenType::If | TokenType::Then => "→",
                TokenType::Iff => "↔",
                _ => "?",
            };
            AstNode::with_children(
                op_str,
                "binary_op",
                vec![
                    expr_to_ast_node(left, interner),
                    expr_to_ast_node(right, interner),
                ],
            )
        }
        LogicExpr::UnaryOp { op, operand } => {
            let op_str = match op {
                TokenType::Not => "¬",
                _ => "?",
            };
            AstNode::with_children(
                op_str,
                "unary_op",
                vec![expr_to_ast_node(operand, interner)],
            )
        }
        LogicExpr::Identity { left, right } => {
            AstNode::with_children(
                "=",
                "identity",
                vec![
                    term_to_ast_node(left, interner),
                    term_to_ast_node(right, interner),
                ],
            )
        }
        LogicExpr::Modal { vector, operand } => {
            AstNode::with_children(
                &format!("□{:?}", vector.domain),
                "modal",
                vec![expr_to_ast_node(operand, interner)],
            )
        }
        LogicExpr::Lambda { variable, body } => {
            let var_str = interner.resolve(*variable);
            AstNode::with_children(
                &format!("λ{}", var_str),
                "lambda",
                vec![expr_to_ast_node(body, interner)],
            )
        }
        _ => AstNode::leaf(&format!("{:?}", expr), "other"),
    }
}

fn term_to_ast_node(term: &Term, interner: &Interner) -> AstNode {
    match term {
        Term::Constant(sym) => AstNode::leaf(interner.resolve(*sym), "constant"),
        Term::Variable(sym) => AstNode::leaf(interner.resolve(*sym), "variable"),
        Term::Function(name, args) => {
            let name_str = interner.resolve(*name);
            let arg_nodes: Vec<AstNode> = args.iter()
                .map(|t| term_to_ast_node(t, interner))
                .collect();
            AstNode::with_children(&format!("{}()", name_str), "function", arg_nodes)
        }
        Term::Group(terms) => {
            let term_nodes: Vec<AstNode> = terms.iter()
                .map(|t| term_to_ast_node(t, interner))
                .collect();
            AstNode::with_children("⊕", "group", term_nodes)
        }
        _ => AstNode::leaf(&format!("{:?}", term), "term"),
    }
}
