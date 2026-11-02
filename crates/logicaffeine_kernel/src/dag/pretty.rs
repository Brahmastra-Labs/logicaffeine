//! A bounded, readable rendering of core expressions for diagnostics.

use super::expr::{ExprId, Lit, Node};
use super::level::{LevelId, LevelNode};
use super::tc::Kernel;

const BUDGET: usize = 400;

pub fn show(k: &Kernel, e: ExprId) -> String {
    let mut out = String::new();
    let mut budget = BUDGET;
    go(k, e, 0, &mut out, &mut budget, false);
    out
}

pub fn show_level(k: &Kernel, l: LevelId) -> String {
    match k.s.levels.node(l) {
        LevelNode::Zero => "0".into(),
        LevelNode::SProp => "SProp".into(),
        LevelNode::Param(n) => k.s.names.display(n),
        LevelNode::Succ(_) => {
            let (mut base, mut n) = (l, 0);
            while let LevelNode::Succ(a) = k.s.levels.node(base) {
                base = a;
                n += 1;
            }
            if base == LevelId::ZERO {
                n.to_string()
            } else {
                format!("{}+{}", show_level(k, base), n)
            }
        }
        LevelNode::Max(a, b) => format!("max ({}) ({})", show_level(k, a), show_level(k, b)),
        LevelNode::IMax(a, b) => format!("imax ({}) ({})", show_level(k, a), show_level(k, b)),
    }
}

fn go(k: &Kernel, e: ExprId, depth: u32, out: &mut String, budget: &mut usize, paren: bool) {
    if *budget == 0 {
        out.push('…');
        return;
    }
    *budget -= 1;
    let open = |out: &mut String| {
        if paren {
            out.push('(')
        }
    };
    let close = |out: &mut String| {
        if paren {
            out.push(')')
        }
    };
    match k.s.node(e) {
        Node::BVar(i) => {
            if i < depth {
                out.push_str(&format!("x{}", depth - 1 - i));
            } else {
                out.push_str(&format!("#{i}"));
            }
        }
        Node::FVar(i) => out.push_str(&format!("h{i}")),
        Node::Sort(l) => {
            if l == LevelId::ZERO {
                out.push_str("Prop");
            } else {
                out.push_str(&format!("Sort ({})", show_level(k, l)));
            }
        }
        Node::Const(n, _) => out.push_str(&k.s.names.display(n)),
        Node::App(..) => {
            open(out);
            let (f, args) = k.s.app_args(e);
            go(k, f, depth, out, budget, true);
            for a in args {
                out.push(' ');
                go(k, a, depth, out, budget, true);
            }
            close(out);
        }
        Node::Lam(t, b) => {
            open(out);
            out.push_str(&format!("fun (x{depth} : "));
            go(k, t, depth, out, budget, false);
            out.push_str(") => ");
            go(k, b, depth + 1, out, budget, false);
            close(out);
        }
        Node::Pi(t, b) => {
            open(out);
            if k.s.loose_bvar_range(b) == 0 {
                go(k, t, depth, out, budget, true);
                out.push_str(" → ");
            } else {
                out.push_str(&format!("(x{depth} : "));
                go(k, t, depth, out, budget, false);
                out.push_str(") → ");
            }
            go(k, b, depth + 1, out, budget, false);
            close(out);
        }
        Node::Let(t, v, b) => {
            open(out);
            out.push_str(&format!("let x{depth} : "));
            go(k, t, depth, out, budget, false);
            out.push_str(" := ");
            go(k, v, depth, out, budget, false);
            out.push_str("; ");
            go(k, b, depth + 1, out, budget, false);
            close(out);
        }
        Node::NatLit(n) => out.push_str(&k.s.nat(n).to_decimal()),
        Node::StrLit(s) => out.push_str(&format!("{:?}", &*k.s.string(s))),
        Node::Proj(_, i, x) => {
            go(k, x, depth, out, budget, true);
            out.push_str(&format!(".{}", i + 1));
        }
        Node::Lit(l) => out.push_str(&match k.s.lit_value(l) {
            Lit::Int(i) => i.to_string(),
            Lit::BigInt(b) => b.to_string(),
            Lit::Float(bits) => format!("{:?}", f64::from_bits(*bits)),
            Lit::Duration(d) => format!("{d}ns"),
            Lit::Date(d) => format!("date({d})"),
            Lit::Moment(m) => format!("moment({m})"),
        }),
        Node::Match(d, m, cs) => {
            open(out);
            out.push_str("match ");
            go(k, d, depth, out, budget, true);
            out.push_str(" return ");
            go(k, m, depth, out, budget, true);
            out.push_str(" with");
            for c in k.s.seq_items(cs) {
                out.push_str(" | ");
                go(k, c, depth, out, budget, false);
            }
            out.push_str(" end");
            close(out);
        }
        Node::Fix(b) => {
            open(out);
            out.push_str(&format!("fix x{depth} => "));
            go(k, b, depth + 1, out, budget, false);
            close(out);
        }
        Node::MutualFix(ds, i) => {
            open(out);
            let ds = k.s.seq_items(ds);
            let n = ds.len() as u32;
            out.push_str("fix");
            for (j, b) in ds.iter().enumerate() {
                out.push_str(&format!(
                    " {}x{} => ",
                    if j == 0 { "" } else { "with " },
                    depth + j as u32
                ));
                go(k, *b, depth + n, out, budget, false);
            }
            out.push_str(&format!(" return x{}", depth + i));
            close(out);
        }
        Node::Seq(..) | Node::Nil => out.push_str("⟨sequence⟩"),
    }
}
