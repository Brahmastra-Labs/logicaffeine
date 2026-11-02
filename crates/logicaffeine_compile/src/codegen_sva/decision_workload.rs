//! The HFT decision as a workload: its one LOGOS source, lowered to a combinational select-DAG by
//! `emit_decision_fabric` (the decision fabric), and run as ordinary bytecode on the routable scalar
//! core by the program here — the CPU twin of `decision_hft_realbist`. Both blocks decide over the same
//! realistic input ranges, so their gate-level switching power per decision is one comparison.

use super::scalar_asm::compile_to_scalar_words;

/// The function name `DECIDE_FN` defines.
pub const DECIDE_NAME: &str = "decide";

/// The HFT decision: microprice market-making, a fat-finger clamp and a position risk gate, returning
/// the signed order (+buy / -sell / 0). No division — the microprice arrives normalized — so the body is
/// compare / arithmetic / branch only.
pub const DECIDE_FN: &str = "\
## To decide (micro100: Int, fair100: Int, edge100: Int, base_size: Int, max_order: Int, inventory: Int, pos_cap: Int, book_ok: Int) -> Int:
    Let mutable order_side be 0.
    Let mutable order_size be 0.
    If book_ok is equal to 1:
        If micro100 is greater than fair100 + edge100:
            Set order_side to 2.
            Set order_size to base_size.
        Otherwise:
            If micro100 is less than fair100 - edge100:
                Set order_side to 1.
                Set order_size to base_size.
    If order_size is greater than max_order:
        Set order_size to max_order.
    Let mutable signed_order be 0.
    If order_side is equal to 1:
        Set signed_order to order_size.
    If order_side is equal to 2:
        Set signed_order to 0 - order_size.
    Let projected be inventory + signed_order.
    Let mutable risk_ok be 1.
    If projected is greater than pos_cap:
        Set risk_ok to 0.
    If projected is less than 0 - pos_cap:
        Set risk_ok to 0.
    If risk_ok is equal to 0:
        Set signed_order to 0.
    Return signed_order.
";

/// The CPU twin's stimulus: a 15-bit linear congruential generator, s' = (s * A + C) mod 2^15. A ≡ 1
/// (mod 4) with C odd gives the full period of 32768 (Hull–Dobell), and every product stays below 2^31,
/// so the 32-bit core and the 64-bit VM compute it identically.
pub const LCG_A: i64 = 20077;
pub const LCG_C: i64 = 12345;
pub const LCG_MASK: i64 = 32767;

/// Decisions the boot ROM makes before it halts — far more than any measured window spans.
pub const SOFT_CORE_REALBIST_ITERATIONS: u32 = 30000;

/// The body of the single-`Return` function `func` spliced in at `indent` spaces, and the expression its
/// final `Return e.` yields. Each parameter is bound to its argument: an integer literal is substituted
/// at every whole-word use (a constant needs no register of its own), an argument that is the
/// parameter's own name needs no binding, and anything else is bound by `Let`. Refused: an argument
/// count that is not the arity, and any `Return` other than the body's last line — its early exit has no
/// inline equivalent.
pub fn inline_body(func: &str, args: &[&str], indent: usize) -> Result<(String, String), String> {
    let mut lines = func.lines().filter(|l| !l.trim().is_empty());
    let header = lines.next().ok_or("an empty function")?;
    let params: Vec<&str> = header
        .split_once('(')
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(ps, _)| {
            ps.split(',')
                .map(|p| p.split(':').next().unwrap_or("").trim())
                .filter(|p| !p.is_empty())
                .collect()
        })
        .ok_or_else(|| format!("no parameter list in `{header}`"))?;
    if params.len() != args.len() {
        return Err(format!(
            "{} arguments for {} parameters",
            args.len(),
            params.len()
        ));
    }
    let body: Vec<&str> = lines.collect();
    let (last, rest) = body.split_last().ok_or("an empty body")?;
    if rest.iter().any(|l| l.trim_start().starts_with("Return ")) {
        return Err("a Return before the body's last line has no inline equivalent".into());
    }
    let ret = last
        .strip_prefix("    Return ")
        .and_then(|e| e.strip_suffix('.'))
        .ok_or_else(|| format!("the body must end in a top-level `Return e.`, not `{last}`"))?;
    let literal = |a: &str| {
        a.strip_prefix('-')
            .unwrap_or(a)
            .chars()
            .all(|c| c.is_ascii_digit())
            && !a.is_empty()
    };
    let substitute = |text: &str| -> String {
        let mut t = text.to_string();
        for (p, a) in params.iter().zip(args).filter(|(_, a)| literal(a)) {
            t = replace_word(&t, p, a);
        }
        t
    };
    let pad = " ".repeat(indent);
    let mut out = String::new();
    for (p, a) in params.iter().zip(args) {
        if p != a && !literal(a) {
            out.push_str(&format!("{pad}Let {p} be {a}.\n"));
        }
    }
    for l in rest {
        let l = l
            .strip_prefix("    ")
            .ok_or_else(|| format!("body line `{l}` is not indented under the header"))?;
        out.push_str(&format!("{pad}{}\n", substitute(l)));
    }
    Ok((out, substitute(ret)))
}

/// [`inline_body`], with the returned value assigned to the already-declared `result`.
pub fn inline_call(
    func: &str,
    args: &[&str],
    result: &str,
    indent: usize,
) -> Result<String, String> {
    let (body, ret) = inline_body(func, args, indent)?;
    Ok(format!(
        "{body}{}Set {result} to {ret}.\n",
        " ".repeat(indent)
    ))
}

/// `text` with every whole-word `word` (not part of a longer identifier) replaced by `with`.
fn replace_word(text: &str, word: &str, with: &str) -> String {
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find(word) {
        let before = rest[..at].chars().next_back();
        let after = rest[at + word.len()..].chars().next();
        out.push_str(&rest[..at]);
        if before.map_or(true, |c| !ident(c)) && after.map_or(true, |c| !ident(c)) {
            out.push_str(with);
        } else {
            out.push_str(word);
        }
        rest = &rest[at + word.len()..];
    }
    out.push_str(rest);
    out
}

/// The CPU twin's program: each iteration advances the LCG three times, mapping each output onto the
/// realbist's input ranges as it is drawn, runs the decision inline, and shows
/// `signed_order * 4 + order_side` — the whole decision, side included, so the output stream classifies
/// every decision as buy, sell, risk-blocked or no-trade.
pub fn soft_core_realbist_source(iterations: u32) -> String {
    let step = format!("Set s to (s * {LCG_A} + {LCG_C}) & {LCG_MASK}.");
    let args = [
        "micro100",
        "10050",
        "edge100",
        "base_size",
        "max_order",
        "inventory",
        "100",
        "book_ok",
    ];
    let (decide, order) = inline_body(DECIDE_FN, &args, 8).expect("the HFT decision inlines");
    format!(
        "## Main\n\
         \x20   Let mutable s be 1.\n\
         \x20   Let mutable i be 0.\n\
         \x20   While i is less than {iterations}:\n\
         \x20       {step}\n\
         \x20       Let micro100 be 10000 + (s shifted right by 8).\n\
         \x20       Let edge100 be (s shifted right by 4) & 15.\n\
         \x20       {step}\n\
         \x20       Let base_size be 1 + (s shifted right by 9).\n\
         \x20       Let max_order be 10 + ((s shifted right by 4) & 31).\n\
         \x20       {step}\n\
         \x20       Let inventory be (s shifted right by 7) - 128.\n\
         \x20       Let book_ok be (s shifted right by 6) & 1.\n\
         {decide}\
         \x20       Show {order} * 4 + order_side.\n\
         \x20       Set i to i + 1.\n"
    )
}

/// The boot ROM the block ships: the program at [`SOFT_CORE_REALBIST_ITERATIONS`], encoded for the
/// scalar core.
pub fn soft_core_realbist_program() -> Vec<u32> {
    compile_to_scalar_words(&soft_core_realbist_source(SOFT_CORE_REALBIST_ITERATIONS))
        .unwrap_or_else(|e| panic!("the CPU twin's program leaves the scalar subset: {e}"))
}

/// The block's top: the scalar core itself, whose `$readmemh("program.hex")` boot ROM holds
/// [`soft_core_realbist_program`]. Only `clk` and `rst` enter; the decision stream leaves on `out`.
pub fn soft_core_realbist_verilog() -> String {
    "// soft_core_realbist — the routable scalar LOGOS core running the HFT decision over the\n\
     // realbist's input ranges (codegen_sva::decision_workload): the CPU twin of decision_hft_realbist.\n\
     // Its boot ROM (program.hex, the emitted soft_core_realbist.hex) draws the inputs from a 15-bit LCG,\n\
     // decides inline and shows signed_order * 4 + order_side once per decision. Emitted by\n\
     // `cargo run -p logicaffeine-compile --example emit_soft_core_realbist`.\n\
     module soft_core_realbist(input clk, input rst, output [31:0] out, output out_valid, output halted);\n\
     \x20 soft_core_scalar core(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));\n\
     endmodule\n"
        .to_string()
}
