//! fabric_width_lock — the fabric holds every value at the width its proven interval needs: each memory word, each
//! accumulator bank, each register and wire of an engine's datapath.
//!
//! `spatial::value_range::program_ranges` proves what each tensor, buffer and stage of a scheduled program holds, and
//! the fabric printer sizes its storage and its datapath from it. This lock holds:
//!
//! - each tensor memory at its proven width — a tensor the host carries (an `Input`, a `State`) whole, at its own
//!   precision — and each FIFO at the widest of the buffers its module serves;
//! - no memory word wider than 32 bits: a wider word is held in 32-bit lanes, since a block RAM wider than 32 bits maps
//!   to the 72-bit simple dual port whose parity lanes the open 7-series flow wires wrong;
//! - no engine register or wire of the decode step at 64 bits but its ports to its memories and to its PE array and the
//!   weight rows it packs, and its accumulator banks exactly as wide as the widest value any of its stages keeps;
//! - a program whose operands are bounded by nothing still runs whole — words past 32 bits through both lanes — and
//!   equals the choreography under iverilog;
//! - every product of two values stands alone, the whole right side of a node, and is evaluated at exactly that node's
//!   width: no operand wider than the node, the node no wider than its operands' widths together. Verilog evaluates a
//!   product at the widest of what it is assigned to and its operands: an int8 product assigned to a 28-bit accumulator
//!   is a 28-bit multiplier, which on the XC7A100T took eight more DSP slices, 1,600 more carry cells and the clock from
//!   28 MHz to 2 MHz; an operand selected among values and an unsized `0` is 32 bits wide whatever the node keeps. A
//!   product by a literal is a shift and an add, and may stand anywhere.
//!
//! Three machines run the decode step: the sky130 fabric and the two Nexys parts.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test fabric_width_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/fabric_rtl.rs"]
mod fabric_rtl;
#[path = "common/spatial.rs"]
mod spatial_support;

use std::collections::{BTreeMap, BTreeSet};

use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::ir::{NumericsPolicy, SpatialProgram, TensorId, TensorRole};
use logicaffeine_compile::spatial::lower::fabric::{FabricBackend, VerilogArtifact};
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::passes::from_tensor_op::MatmulNames;
use logicaffeine_compile::spatial::pipeline::{compile_program_to_spatial, run_passes, template_from_spec};
use logicaffeine_compile::spatial::target::{precision_int_bits, SpatialTarget};
use logicaffeine_compile::spatial::value_range::{program_ranges, signed_bits};
use logicaffeine_compile::tensor_recognize::{Fill, MatmulSpec};
use logicaffeine_compile::spatial::ir::TensorBody;
use logicaffeine_compile::spatial::passes::{LV_GEN_COL, LV_GEN_ROW};
use spatial_support::{elem_map, inputs_from_fills, one_generic, parallel, sum_fold};

fn decode_step(target: &SpatialTarget) -> (SpatialProgram, VerilogArtifact) {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let m = Int8Model::from_json(&j).expect("the model");
    let tok = j["golden_trace"]["prompt_tokens"][0].as_u64().unwrap() as usize;
    let l = compile_program_to_spatial(&m.decode_step_lg(tok, 0, &[], &[]), target).expect("a tensor program");
    let p = l.program.unwrap_or_else(|| panic!("the step lowers whole: {:?}", l.missing.first()));
    let art = lower(&p, target);
    (p, art)
}

fn lower(p: &SpatialProgram, target: &SpatialTarget) -> VerilogArtifact {
    match FabricBackend::new(target.clone()).lower(p) {
        Ok(SpatialArtifact::Verilog(v)) => v,
        other => panic!("the fabric lowers the program: {other:?}"),
    }
}

fn fabrics() -> [SpatialTarget; 3] {
    [SpatialTarget::sky_fabric(16, 64), SpatialTarget::xc7a200t_fabric(16, 64), SpatialTarget::xc7a100t_fabric(16, 64)]
}

/// Each module of `rtl` by name, its text up to its `endmodule`.
fn modules(rtl: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for chunk in rtl.split("\nmodule ").skip(1) {
        let name: String = chunk.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        let body = chunk.split("\nendmodule").next().unwrap_or(chunk);
        out.insert(name, body.to_string());
    }
    out
}

/// Every memory `reg [W-1:0] name [0:N]` a module declares, by name, as its word width.
fn memories(module: &str) -> BTreeMap<String, u32> {
    let mut out = BTreeMap::new();
    for line in module.lines() {
        let Some(at) = line.find("reg [") else { continue };
        let rest = &line[at + 5..];
        let Some((hi, rest)) = rest.split_once(":0] ") else { continue };
        let Some((name, rest)) = rest.split_once(' ') else { continue };
        if rest.starts_with("[0:") {
            out.insert(name.to_string(), hi.parse::<u32>().unwrap() + 1);
        }
    }
    out
}

/// The lanes a word of `width` bits is held in: one up to 32 bits, 32-bit lanes past it.
fn lanes(width: u32) -> Vec<u32> {
    (0..width.div_ceil(32)).map(|l| (width - l * 32).min(32)).collect()
}

/// The widths memory `mem` is declared in, lane by lane — a banked memory's, every one of its banks alike.
fn declared_lanes(mems: &BTreeMap<String, u32>, mem: &str) -> Vec<u32> {
    let of = |m: &str| -> Vec<u32> {
        if let Some(w) = mems.get(m) {
            return vec![*w];
        }
        (0..).map_while(|l| mems.get(&format!("{m}_l{l}")).copied()).collect()
    };
    let whole = of(mem);
    if !whole.is_empty() {
        return whole;
    }
    let banks: Vec<Vec<u32>> = (0..).map(|b| of(&format!("{mem}_b{b}"))).take_while(|v| !v.is_empty()).collect();
    assert!(banks.windows(2).all(|w| w[0] == w[1]), "{mem}: every bank is declared at one width: {banks:?}");
    banks.into_iter().next().unwrap_or_default()
}

#[test]
fn every_memory_word_is_held_at_its_proven_width() {
    for target in fabrics() {
        let (p, art) = decode_step(&target);
        let proven = program_ranges(&p);
        let mods = modules(&art.rtl);
        let mut checked = 0usize;
        for (name, body) in mods.iter().filter(|(n, _)| n.starts_with("site_")) {
            let mems = memories(body);
            for t in &p.tensors {
                let want = match t.role {
                    TensorRole::Input | TensorRole::State => precision_int_bits(t.dtype).unwrap_or(64),
                    _ => proven.tensors[t.id.idx()].map_or(1, signed_bits),
                };
                assert_eq!(
                    declared_lanes(&mems, &format!("mem_t{}", t.id.idx())),
                    lanes(want),
                    "{}: {name}: tensor {} ({:?}) is held at {want} bits",
                    target.name,
                    t.name,
                    t.role
                );
                checked += 1;
            }
        }
        // A FIFO's pages live in its site's pools: each pool is held at the widest width any buffer it holds needs. The
        // site's merge names a pool's buffers: `wire pool<i>_we = f<a>_wr_en | f<b>_wr_en ...;`.
        let mut pools = 0usize;
        for (name, body) in mods.iter().filter(|(n, _)| n.starts_with("site_")) {
            let xy = name.trim_start_matches("site_");
            for line in body.lines() {
                let t = line.trim();
                let Some(rest) = t.strip_prefix("wire pool") else { continue };
                let Some((i, members)) = rest.split_once("_we = ") else { continue };
                let ids: Vec<usize> = members
                    .trim_end_matches(';')
                    .split(" | ")
                    .map(|m| m.trim().trim_start_matches('f').trim_end_matches("_wr_en").parse().unwrap())
                    .collect();
                let want = ids.iter().filter_map(|b| proven.buffers[*b]).map(signed_bits).max().unwrap_or(1);
                let pool = mods.get(&format!("pool_{xy}_{i}")).unwrap_or_else(|| panic!("{}: {name} instantiates pool_{xy}_{i}", target.name));
                assert_eq!(
                    declared_lanes(&memories(pool), "mem"),
                    lanes(want),
                    "{}: pool {i} of {name} is held at the {want} bits its buffers {ids:?} need",
                    target.name
                );
                pools += 1;
            }
        }
        assert!(pools > 0, "{}: the step's pages live in pools", target.name);
        checked += pools;
        assert_eq!(checked, p.tensors.len() * mods.keys().filter(|n| n.starts_with("site_")).count() + pools, "{}: every tensor and every pool checked", target.name);
    }
}

#[test]
fn no_memory_word_is_wider_than_32_bits() {
    let mut programs: Vec<(String, VerilogArtifact)> = Vec::new();
    for target in fabrics() {
        programs.push((format!("decode@{}", target.name), decode_step(&target).1));
        let p = unbounded_gemm(&target);
        programs.push((format!("unbounded gemm@{}", target.name), lower(&p, &target)));
    }
    for (label, art) in &programs {
        let mut wide = 0usize;
        for (name, body) in modules(&art.rtl) {
            for (mem, w) in memories(&body) {
                assert!(w <= 32, "{label}: {name}.{mem} holds {w}-bit words");
                wide += (w == 32) as usize;
            }
        }
        assert!(wide > 0, "{label}: a word past 32 bits is held in full 32-bit lanes");
    }
}

#[test]
fn the_decode_step_s_engines_hold_their_proven_widths() {
    for target in fabrics() {
        let (p, art) = decode_step(&target);
        let proven = program_ranges(&p);
        let widest = proven.stages.iter().filter_map(|s| s.held).map(signed_bits).max().unwrap();
        let mods = modules(&art.rtl);
        let engines: Vec<(&String, &String)> = mods.iter().filter(|(n, _)| n.starts_with("engine_")).collect();
        assert!(!engines.is_empty(), "{}: the step has engines", target.name);
        let mut banks = Vec::new();
        for (name, body) in engines {
            // The memories' ports carry a word sign-extended, and the PE array's — the shared systolic array's own
            // interface — its operands and partial sums; synthesis keeps the bits the array reads and writes. A row of
            // weights the array loads (`pe_w_data`, `b_r*`, and `wmask` over it) is its columns' operands packed side by
            // side, eight int8 operands making 64 bits on an eight-column array: no value in it is wider than an operand.
            for line in body.lines().filter(|l| l.contains("[63:0]")) {
                let port = ["_wr_data", "_rd_data", " pe_a", " pe_o", " pe_w_data", " b_r", " wmask"].iter().any(|s| line.contains(s));
                assert!(port, "{}: {name} holds a 64-bit value outside its ports: {}", target.name, line.trim());
            }
            for line in body.lines().filter(|l| l.contains(" acc_b0 [")) {
                let hi: u32 = line.split("signed [").nth(1).unwrap().split(':').next().unwrap().parse().unwrap();
                banks.push(hi + 1);
            }
        }
        assert_eq!(
            banks.iter().max().copied(),
            Some(widest),
            "{}: the widest accumulator bank is the widest value a stage keeps ({widest} bits)",
            target.name
        );
        assert!(widest <= 32, "{}: the step keeps nothing past 32 bits ({widest})", target.name);
    }
}

/// The text of `module` as code: no comment, no attribute, every quoted string blank.
fn code(module: &str) -> String {
    let mut out = String::new();
    for line in module.lines() {
        let (mut plain, mut quoted) = (String::new(), false);
        let c: Vec<char> = line.chars().collect();
        for (i, ch) in c.iter().enumerate() {
            match ch {
                '"' => quoted = !quoted,
                '/' if !quoted && c.get(i + 1) == Some(&'/') => break,
                _ if quoted => plain.push(' '),
                _ => plain.push(*ch),
            }
        }
        let mut rest = plain.as_str();
        while let Some(at) = rest.find("(*") {
            out.push_str(&rest[..at]);
            rest = rest[at..].split_once("*)").map_or("", |(_, r)| r);
        }
        out.push_str(rest);
        out.push('\n');
    }
    out
}

/// The tokens of Verilog `text`: a name (`$signed` among them), a number (a based literal whole), or an operator.
fn tokens(text: &str) -> Vec<String> {
    let c: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let start = i;
        if c[i].is_whitespace() {
            i += 1;
            continue;
        }
        if c[i].is_alphabetic() || c[i] == '_' || c[i] == '$' {
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '$') {
                i += 1;
            }
        } else if c[i].is_ascii_digit() || c[i] == '\'' {
            while i < c.len() && c[i].is_ascii_digit() {
                i += 1;
            }
            if i < c.len() && c[i] == '\'' {
                i += 1;
                while i < c.len() && (c[i].is_ascii_alphanumeric() || c[i] == '_') {
                    i += 1;
                }
            }
        } else {
            let three: String = c[i..(i + 3).min(c.len())].iter().collect();
            let two: String = c[i..(i + 2).min(c.len())].iter().collect();
            i += if [">>>", "<<<"].contains(&three.as_str()) {
                3
            } else if ["<=", ">=", "==", "!=", "&&", "||", ">>", "<<", "**"].contains(&two.as_str()) {
                2
            } else {
                1
            };
        }
        out.push(c[start..i].iter().collect());
    }
    out
}

fn number(tok: &str) -> bool {
    tok.starts_with(|c: char| c.is_ascii_digit() || c == '\'')
}

/// A number's width: its size where it is sized, 32 where it is not.
fn number_width(tok: &str) -> u32 {
    match tok.split_once('\'') {
        Some((n, _)) if !n.is_empty() => n.parse().unwrap(),
        _ => 32,
    }
}

/// The index of the token that closes the bracket opening at `open`.
fn close(t: &[String], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, tok) in t.iter().enumerate().skip(open) {
        match tok.as_str() {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// `t` split at every comma outside a bracket.
fn top_level_commas(t: &[String]) -> Vec<&[String]> {
    let (mut out, mut depth, mut from) = (Vec::new(), 0i32, 0);
    for (i, tok) in t.iter().enumerate() {
        match tok.as_str() {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth -= 1,
            "," if depth == 0 => {
                out.push(&t[from..i]);
                from = i + 1;
            }
            _ => {}
        }
    }
    out.push(&t[from..]);
    out
}

/// The `[h:l]` range at the head of `t`, as its width and the tokens it took; none, one bit and no tokens.
fn range(t: &[String]) -> Option<(u32, usize)> {
    if t.first().map(String::as_str) != Some("[") {
        return Some((1, 0));
    }
    let end = close(t, 0)?;
    match &t[1..end] {
        [h, colon, l] if colon == ":" => Some((h.parse::<u32>().ok()?.abs_diff(l.parse().ok()?) + 1, end + 1)),
        _ => None,
    }
}

/// Every name the statements of `module` declare — a port, a `reg`, a `wire`, an `integer` — with its width.
fn declared(module: &str) -> BTreeMap<String, u32> {
    let mut out = BTreeMap::new();
    let declare = |item: &[String], out: &mut BTreeMap<String, u32>| {
        let kinds = ["input", "output", "inout", "reg", "wire", "integer", "signed"];
        let at = item.iter().position(|t| !kinds.contains(&t.as_str())).unwrap_or(item.len());
        if at == 0 {
            return;
        }
        let integer = item[..at].iter().any(|t| t == "integer");
        let Some((width, took)) = range(&item[at..]) else { return };
        for name in top_level_commas(&item[at + took..]) {
            if let Some(n) = name.first() {
                out.insert(n.clone(), if integer { 32 } else { width });
            }
        }
    };
    for stmt in code(module).split(';') {
        let t = tokens(stmt);
        let lead = t.iter().position(|t| !["end", "endcase", "begin", "else", "endfunction"].contains(&t.as_str())).unwrap_or(t.len());
        let t = &t[lead..];
        // A module's head, its name first (`modules` takes `module` off): its ports are the bracket after its name and
        // its parameters.
        let mut open = 1;
        if t.get(1).map(String::as_str) == Some("#") {
            open = close(t, 2).map_or(t.len(), |e| e + 1);
        }
        if t.get(open).map(String::as_str) == Some("(") {
            if let Some(end) = close(t, open) {
                for port in top_level_commas(&t[open + 1..end]) {
                    declare(port, &mut out);
                }
            }
            continue;
        }
        declare(t, &mut out);
    }
    out
}

/// The width of an operand of a product: a name, its slice or its bit, a number, a sign cast of one, or a select
/// `c ? x : y` between them; anything else is an operand the scan does not read.
fn operand_width(t: &[String], names: &BTreeMap<String, u32>) -> Result<u32, String> {
    let shown = || t.join(" ");
    if t.is_empty() {
        return Err("an empty operand".into());
    }
    if t[0] == "(" && close(t, 0) == Some(t.len() - 1) {
        return operand_width(&t[1..t.len() - 1], names);
    }
    // A select: the condition up to its `?` outside every bracket, then the arm up to the `:` that pairs with it.
    let mut depth = 0i32;
    let mut question = None;
    for (i, tok) in t.iter().enumerate() {
        match tok.as_str() {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth -= 1,
            "?" if depth == 0 => {
                question = Some(i);
                break;
            }
            _ => {}
        }
    }
    if let Some(q) = question {
        let (mut depth, mut nested) = (0i32, 0usize);
        for (i, tok) in t.iter().enumerate().skip(q + 1) {
            match tok.as_str() {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => depth -= 1,
                "?" if depth == 0 => nested += 1,
                ":" if depth == 0 && nested > 0 => nested -= 1,
                ":" if depth == 0 => {
                    return Ok(operand_width(&t[q + 1..i], names)?.max(operand_width(&t[i + 1..], names)?));
                }
                _ => {}
            }
        }
        return Err(format!("a select without its `:`: {}", shown()));
    }
    if (t[0] == "$signed" || t[0] == "$unsigned") && t.get(1).map(String::as_str) == Some("(") && close(t, 1) == Some(t.len() - 1) {
        return operand_width(&t[2..t.len() - 1], names);
    }
    if t.len() == 1 && number(&t[0]) {
        return Ok(number_width(&t[0]));
    }
    let name = &t[0];
    if !name.starts_with(|c: char| c.is_alphabetic() || c == '_') {
        return Err(format!("an operand the scan does not read: {}", shown()));
    }
    let Some(&width) = names.get(name) else { return Err(format!("`{name}` is not declared")) };
    match &t[1..] {
        [] => Ok(width),
        [o, i, c] if o == "[" && c == "]" && i.parse::<u32>().is_ok() => Ok(1),
        [o, h, colon, l, c] if o == "[" && colon == ":" && c == "]" => match (h.parse::<u32>(), l.parse::<u32>()) {
            (Ok(h), Ok(l)) => Ok(h.abs_diff(l) + 1),
            _ => Err(format!("a slice the scan does not read: {}", shown())),
        },
        _ => Err(format!("an operand the scan does not read: {}", shown())),
    }
}

/// A product of two values a module computes: the node it is the whole right side of, the node's width, and its
/// operands' widths.
#[derive(Debug)]
struct Product {
    node: String,
    width: u32,
    operands: (u32, u32),
}

/// Every product of two values in `module` — a product by a literal is a shift and an add, and stands anywhere — and
/// every one that does not stand alone as the whole right side of a declared node, or whose operands the scan cannot
/// read, as the statement it is in.
fn products(module: &str) -> (Vec<Product>, Vec<String>) {
    let names = declared(module);
    let (mut found, mut loose) = (Vec::new(), Vec::new());
    for stmt in code(module).split(';') {
        let t = tokens(stmt);
        for k in (0..t.len()).filter(|&k| t[k] == "*" && k > 0 && t[k - 1] != "@" && t[k - 1] != "(") {
            let left_literal = number(&t[k - 1]) || (t[k - 1] == ")" && k >= 3 && t[k - 3] == "(" && number(&t[k - 2]));
            let right_literal = t.get(k + 1).is_some_and(|x| number(x))
                || (t.get(k + 1).map(String::as_str) == Some("(") && t.get(k + 2).is_some_and(|x| number(x)) && t.get(k + 3).map(String::as_str) == Some(")"));
            if left_literal || right_literal {
                continue;
            }
            // The assignment the product is the right side of: the last `=` or `<=` before it, outside every bracket.
            let mut depth = 0i32;
            let mut assign = None;
            for (i, tok) in t[..k].iter().enumerate() {
                match tok.as_str() {
                    "(" | "[" | "{" => depth += 1,
                    ")" | "]" | "}" => depth -= 1,
                    "=" | "<=" if depth == 0 => assign = Some(i),
                    _ => {}
                }
            }
            let shown = t.join(" ");
            let Some(a) = assign.filter(|&a| a > 0) else {
                loose.push(format!("a product outside an assignment: {shown}"));
                continue;
            };
            let node = &t[a - 1];
            let (Some(&width), Ok(x), Ok(y)) = (names.get(node), operand_width(&t[a + 1..k], &names), operand_width(&t[k + 1..], &names)) else {
                loose.push(format!("a product that is not the whole right side of a declared node: {shown}"));
                continue;
            };
            found.push(Product { node: node.clone(), width, operands: (x, y) });
        }
    }
    (found, loose)
}

/// Whether `p` is evaluated at exactly its node's width, or why not.
fn evaluated_at_its_width(p: &Product) -> Result<(), String> {
    let (x, y) = p.operands;
    if x.max(y) > p.width {
        return Err(format!("{} is {} bits and its operands {x} and {y}: the product is a {}-bit multiplier", p.node, p.width, x.max(y)));
    }
    if p.width > x + y {
        return Err(format!("{} is {} bits and its operands {x} and {y}: the product is a {}-bit multiplier", p.node, p.width, p.width));
    }
    Ok(())
}

/// A(1×8)·W(8×8): a matmul of one row, which runs on the matrix-vector unit.
fn matvec(target: &SpatialTarget) -> SpatialProgram {
    let tensor = |name: &str, rows: usize, cols: usize, seed: i64| -> String {
        let mut s = format!("Let {name} be a {rows} by {cols} tensor of Int.\n");
        for i in 0..rows * cols {
            s.push_str(&format!("Set item {} of {name} to {}.\n", i + 1, ((i as i64 * 2_654_435_761 + seed) % 255) - 127));
        }
        s
    };
    let src = format!("## Main\n{}{}Let C be A matmul W.\n", tensor("A", 1, 8, 3), tensor("W", 8, 8, 5));
    let l = compile_program_to_spatial(&src, target).expect("a tensor program");
    l.program.unwrap_or_else(|| panic!("the matvec lowers whole: {:?}", l.missing.first()))
}

/// Small integer programs whose units keep fewer bits than they read: a ReLU that keeps 8 bits of 17, a requant by zero
/// (a Generic product of one bit from a 20-bit operand), a Hadamard product by a tensor of zeros (the unary unit's
/// product, one bit from a 19-bit input), a Hadamard product, a saturation, a requant, a gather then a requant, a LUT
/// attention up to a position.
fn integer_programs() -> Vec<(&'static str, String)> {
    let tensor = |name: &str, rows: usize, cols: usize, lo: i64, hi: i64| -> String {
        let mut s = format!("Let {name} be a {rows} by {cols} tensor of Int.\n");
        for i in 0..rows * cols {
            s.push_str(&format!("Set item {} of {name} to {}.\n", i + 1, lo + (i as i64 * 2_654_435_761).rem_euclid(hi - lo + 1)));
        }
        s
    };
    vec![
        ("relu", format!("## Main\n{}Let Y be relu of X.\n", tensor("X", 1, 8, -40_000, 100))),
        ("requant by zero", format!("## Main\n{}Let Y be X times 0 shifted right by 0.\n", tensor("X", 1, 13, -400_000, 400_000))),
        ("hadamard by zero", format!("## Main\n{}{}Let Z be X hadamard Y.\n", tensor("X", 1, 8, -400_000, 400_000), tensor("Y", 1, 8, 0, 0))),
        ("hadamard", format!("## Main\n{}{}Let Z be X hadamard Y.\n", tensor("X", 1, 8, -400_000, 400_000), tensor("Y", 1, 8, -3, 3))),
        ("clamp", format!("## Main\n{}Let Y be X clamped between -128 and 127.\n", tensor("X", 1, 13, -400_000, 400_000))),
        ("requant", format!("## Main\n{}Let Y be X times 104593 shifted right by 24.\n", tensor("X", 1, 13, -258_064, 258_064))),
        ("gather", format!("## Main\n{}Let at be 3.\nLet r be row at of T.\nLet y be r times 3 shifted right by 0.\n", tensor("T", 5, 7, -900, 900))),
        (
            "lut attention",
            format!(
                "## Main\n{}{}{}{}Let pos be 2.\nLet C be lut attention of Q over K and V with 2 heads 2 groups up to pos with table E and scale 21076 shifted right by 6.\n",
                tensor("Q", 1, 4, -128, 127),
                tensor("K", 4, 4, -128, 127),
                tensor("V", 4, 4, -128, 127),
                tensor("E", 1, 4, 20_000, 32_768)
            ),
        ),
    ]
}

#[test]
fn every_product_is_evaluated_at_its_node_s_width() {
    let mut programs: Vec<(String, VerilogArtifact)> = Vec::new();
    for target in fabrics() {
        programs.push((format!("decode@{}", target.name), decode_step(&target).1));
        programs.push((format!("unbounded gemm@{}", target.name), lower(&unbounded_gemm(&target), &target)));
        programs.push((format!("matvec@{}", target.name), lower(&matvec(&target), &target)));
        for (label, src) in integer_programs() {
            let l = compile_program_to_spatial(&src, &target).expect("a tensor program");
            let p = l.program.unwrap_or_else(|| panic!("{label} lowers whole: {:?}", l.missing.first()));
            programs.push((format!("{label}@{}", target.name), lower(&p, &target)));
        }
    }
    let mut kinds = BTreeSet::new();
    for (label, art) in &programs {
        let mut seen = 0usize;
        for (name, body) in modules(&art.rtl) {
            let (found, loose) = products(&body);
            assert!(loose.is_empty(), "{label}: {name}: {loose:#?}");
            for p in found {
                if let Err(why) = evaluated_at_its_width(&p) {
                    panic!("{label}: {name}: {why}");
                }
                kinds.insert(p.node.trim_end_matches(|c: char| c.is_ascii_digit() || c == '_').to_string());
                seen += 1;
            }
        }
        assert!(seen > 0, "{label}: the scan finds the program's products");
    }
    // Each unit that multiplies: the PE array, the matrix-vector unit, the unary unit, a Generic body's product, a
    // gather's stride.
    for kind in ["prod", "mv_m", "un_mul", "gm", "gu_gm"] {
        assert!(kinds.contains(kind), "the scan reads the {kind} products: {kinds:?}");
    }
}

#[test]
fn the_product_scan_reads_products_as_they_are() {
    let module = "m(input clk, input signed [7:0] a, input [15:0] b, input c);
  reg signed [27:0] wide; always @(posedge clk) begin if (c) wide <= a * $signed(b[7:0]); end
  wire signed [15:0] narrow = a * $signed(b[7:0]);
  (* keep *) wire signed [23:0] sel = $signed(b) * ((c == 1'b1) ? a : 8'sd3);
  wire signed [10:0] inside = 3 + (a * b);
  wire [31:0] scaled = (a * 16 + b) | (b * (4));
  reg [3:0] q; always @* case (c) 1'd0: q = 1; default: q = 2; endcase
";
    let (found, loose) = products(module);
    let got: Vec<(&str, u32, (u32, u32))> = found.iter().map(|p| (p.node.as_str(), p.width, p.operands)).collect();
    assert_eq!(got, vec![("wide", 28, (8, 8)), ("narrow", 16, (8, 8)), ("sel", 24, (16, 8))]);
    assert_eq!(loose.len(), 1, "{loose:#?}");
    assert!(loose[0].contains("inside"), "{loose:#?}");
    let verdicts: Vec<bool> = found.iter().map(|p| evaluated_at_its_width(p).is_ok()).collect();
    assert_eq!(verdicts, vec![false, true, true], "wider than its operands' product, exactly it, and within it");
    let kept = |width, operands| evaluated_at_its_width(&Product { node: "p".into(), width, operands }).is_ok();
    assert!(!kept(11, (11, 32)), "an operand wider than the node widens the multiplier past it");
    assert!(kept(11, (11, 11)) && kept(11, (8, 3)) && !kept(12, (8, 3)));
}

/// A fold's register holds every partial sum its op keeps, which can be wider than any value its body computes: eight
/// values of 100 summed along a row are 800 — 11 bits — where every value the body computes is 100, 8 bits.
#[test]
fn a_fold_s_register_holds_its_widest_partial_sum() {
    if !common::require_tool("iverilog") {
        return;
    }
    for target in fabrics() {
        let p = one_generic(
            &target,
            &[("a", 8, vec![100; 8])],
            1,
            vec![parallel(LV_GEN_ROW, 1), sum_fold(LV_GEN_COL, 8)],
            vec![elem_map(0, &[(LV_GEN_COL, 1)])],
            elem_map(0, &[]),
            TensorBody::Operand(0),
        );
        let proven = program_ranges(&p);
        let widest = proven.stages.iter().filter_map(|s| s.held).map(signed_bits).max().unwrap();
        let node = proven.stages.iter().filter_map(|s| s.nodes.last().copied()).map(signed_bits).max().unwrap();
        assert!(widest > node, "{}: a partial sum ({widest} bits) is wider than every body's value ({node} bits)", target.name);
        let inputs = inputs_from_fills(&p);
        let out = p.output_tensor().unwrap().id;
        let want = match &eval_choreography(&p, &inputs).unwrap().tensors[&out] {
            Value::Int(v) => v.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(want[0], 800, "{}: the row's sum", target.name);
        let art = lower(&p, &target);
        let dir = std::env::temp_dir().join(format!("fabric_width_fold_{}_{}", target.name, std::process::id()));
        let run = fabric_rtl::simulate(&dir, &p, &art, &inputs);
        assert!(run.done, "{}: the design finishes\n{}", target.name, run.log);
        assert_eq!(run.out[&out.idx()], want, "{}: RTL equals the choreography", target.name);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A matmul whose operands declare no range: every value is bounded by nothing, its words are i64.
fn unbounded_gemm(target: &SpatialTarget) -> SpatialProgram {
    let spec = MatmulSpec { m: 2, n: 4, k: 4, a: Default::default(), b: Default::default(), c: Default::default() };
    let names = MatmulNames { a: "a".into(), b: "b".into(), c: "c".into() };
    let p = template_from_spec(&spec, &names, (&Fill::Unknown, &Fill::Unknown), (None, None), NumericsPolicy::Exact, target)
        .expect("the matmul lowers");
    run_passes(p, target).expect("the matmul schedules")
}

/// Words past 32 bits cross the high lane on every path — the host's image, every FIFO, the accumulator — and the
/// result reads back whole.
#[test]
fn an_unbounded_program_runs_whole_words_through_both_lanes() {
    if !common::require_tool("iverilog") {
        return;
    }
    for target in fabrics() {
        let p = unbounded_gemm(&target);
        let draw = |seed: i64, i: i64, span: i64| ((i * 2_654_435_761 + seed * 40_503) % (2 * span + 1)) - span;
        let a: Vec<i64> = (0..8).map(|i| draw(3, i, 1 << 40)).collect();
        let b: Vec<i64> = (0..16).map(|i| draw(7, i, 1 << 18)).collect();
        let mut inputs = Inputs::default();
        inputs.tensors.insert(TensorId(0), Value::Int(a));
        inputs.tensors.insert(TensorId(1), Value::Int(b));
        let want = match &eval_choreography(&p, &inputs).unwrap().tensors[&p.output_tensor().unwrap().id] {
            Value::Int(v) => v.clone(),
            other => panic!("{other:?}"),
        };
        assert!(want.iter().any(|v| v.unsigned_abs() > 1 << 40), "{}: the products pass 32 bits", target.name);
        let art = lower(&p, &target);
        let dir = std::env::temp_dir().join(format!("fabric_width_unbounded_{}_{}", target.name, std::process::id()));
        let run = fabric_rtl::simulate(&dir, &p, &art, &inputs);
        assert!(run.done, "{}: the design finishes\n{}", target.name, run.log);
        assert_eq!(run.out[&p.output_tensor().unwrap().id.idx()], want, "{}: RTL equals the choreography", target.name);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
