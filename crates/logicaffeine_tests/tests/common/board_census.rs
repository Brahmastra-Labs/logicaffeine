//! The census of board tops: every `.v` under `benchmarks/silicon/` that streams bytes through a `tx_byte`
//! register, and every byte it streams that is a constant (an expression of localparams only, or a literal other than
//! the frame's sync byte and the reset value).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn silicon_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon")
}

/// Remove `//` line comments and `/* */` block comments.
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("//") {
            rest = r.find('\n').map_or("", |i| &r[i..]);
        } else if let Some(r) = rest.strip_prefix("/*") {
            rest = r.find("*/").map_or("", |i| &r[i + 2..]);
        } else {
            let c = rest.chars().next().unwrap();
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// Every name a `localparam` declares, including the comma-separated forms.
fn localparams(text: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for stmt in text.split(';') {
        let Some(at) = stmt.find("localparam") else { continue };
        let decl = &stmt[at + "localparam".len()..];
        let decl = match decl.trim_start().strip_prefix('[') {
            Some(r) => r.find(']').map_or("", |i| &r[i + 1..]),
            None => decl,
        };
        for part in decl.split(',') {
            if let Some((name, _)) = part.split_once('=') {
                let name = name.trim().trim_start_matches("integer").trim();
                if !name.is_empty() {
                    names.insert(name.to_string());
                }
            }
        }
    }
    names
}

/// Replace every Verilog number (sized or not) with a space, so its base letters never read as names.
fn without_numbers(expr: &str) -> String {
    let b = expr.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        let starts_number = b[i].is_ascii_digit() || (b[i] == b'\'' && i + 1 < b.len());
        let prev_is_name = i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
        if starts_number && !prev_is_name {
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'_') {
                i += 1;
            }
            if i < b.len() && b[i] == b'\'' {
                i += 1;
                if i < b.len() && matches!(b[i], b's' | b'S') {
                    i += 1;
                }
                if i < b.len() && matches!(b[i], b'b' | b'B' | b'o' | b'O' | b'd' | b'D' | b'h' | b'H') {
                    i += 1;
                }
                while i < b.len() && (b[i].is_ascii_hexdigit() || matches!(b[i], b'_' | b'x' | b'X' | b'z' | b'Z')) {
                    i += 1;
                }
            }
            out.push(' ');
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

/// The value of a bare Verilog literal (`8'hA5`, `8'd0`, `165`), if the expression is one.
fn literal_value(expr: &str) -> Option<u64> {
    let e: String = expr.chars().filter(|c| !c.is_whitespace() && *c != '_').collect();
    let (radix, digits) = match e.split_once('\'') {
        Some((_, rest)) => {
            let rest = rest.trim_start_matches(['s', 'S']);
            let (base, digits) = rest.split_at(rest.chars().next()?.len_utf8());
            let radix = match base {
                "b" | "B" => 2,
                "o" | "O" => 8,
                "d" | "D" => 10,
                "h" | "H" => 16,
                _ => return None,
            };
            (radix, digits.to_string())
        }
        None => (10, e.clone()),
    };
    u64::from_str_radix(&digits, radix).ok()
}

/// Every right-hand side assigned to `tx_byte` in a board top, with the reason it is a constant, if it is one.
pub fn constant_streams(text: &str) -> Vec<(String, String)> {
    let text = strip_comments(text);
    let params = localparams(&text);
    let mut found = Vec::new();
    let mut rest = text.as_str();
    while let Some(at) = rest.find("tx_byte") {
        rest = &rest[at + "tx_byte".len()..];
        let Some(rhs) = rest.trim_start().strip_prefix("<=") else { continue };
        let Some(end) = rhs.find(';') else { break };
        let expr = rhs[..end].trim().to_string();
        let names: BTreeSet<String> = without_numbers(&expr)
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|w| !w.is_empty() && !w.chars().next().unwrap().is_ascii_digit())
            .map(str::to_string)
            .collect();
        if names.is_empty() {
            match literal_value(&expr) {
                Some(0) | Some(0xA5) => {}
                _ => found.push((expr.clone(), "a literal other than the sync byte or the reset value".to_string())),
            }
        } else if names.iter().all(|n| params.contains(n)) {
            found.push((expr.clone(), format!("only localparams: {}", names.into_iter().collect::<Vec<_>>().join(", "))));
        }
    }
    found
}

/// The board tops: every `.v` under `benchmarks/silicon/` that streams bytes through a `tx_byte` register out of
/// the board's `uart_tx` pin. A module that names a `tx_byte` and drives no UART pin (a link framer, a keyboard
/// port) is part of a design, not a board reporting a number.
pub fn board_tops() -> BTreeMap<String, String> {
    let mut tops = BTreeMap::new();
    for entry in std::fs::read_dir(silicon_dir()).expect("benchmarks/silicon") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("v") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read verilog");
        if is_board_top(&text) {
            tops.insert(path.file_name().unwrap().to_string_lossy().into_owned(), text);
        }
    }
    tops
}

/// Whether `text`, outside its line comments, streams bytes through a `tx_byte` register and drives a `uart_tx`
/// output.
pub fn is_board_top(text: &str) -> bool {
    let code: Vec<&str> = text.lines().map(|line| line.split("//").next().unwrap_or("")).collect();
    let words = |line: &str| line.split(|c: char| !c.is_alphanumeric() && c != '_').map(str::to_string).collect::<Vec<_>>();
    code.iter().any(|line| words(line).iter().any(|w| w == "tx_byte"))
        && code.iter().any(|line| {
            let words = words(line);
            words.iter().any(|w| w == "output") && words.iter().any(|w| w == "uart_tx")
        })
}

