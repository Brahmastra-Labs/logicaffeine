//! Verilog Declaration Parser
//!
//! Extracts module structure from Verilog/SystemVerilog source:
//! module name, ports (direction, width), internal signals, parameters,
//! and clock detection from `always @(posedge/negedge)` blocks.
//!
//! This is NOT a full Verilog parser — it handles declaration-level
//! extraction for hardware verification KG construction.

use std::collections::HashSet;

/// Port direction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortDirection {
    Input,
    Output,
    Inout,
}

/// Signal type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignalType {
    Wire,
    Reg,
    Logic,
}

/// A port in the module declaration.
#[derive(Debug, Clone)]
pub struct RtlPort {
    pub name: String,
    pub direction: PortDirection,
    pub width: u32,
}

/// An internal signal declaration.
#[derive(Debug, Clone)]
pub struct RtlSignal {
    pub name: String,
    pub signal_type: SignalType,
    pub width: u32,
}

/// A parameter declaration.
#[derive(Debug, Clone)]
pub struct RtlParam {
    pub name: String,
    pub value: String,
}

/// Extracted module structure.
#[derive(Debug, Clone)]
pub struct RtlModule {
    pub name: String,
    pub ports: Vec<RtlPort>,
    pub signals: Vec<RtlSignal>,
    pub params: Vec<RtlParam>,
    pub clocks: Vec<String>,
}

/// Parse error.
#[derive(Debug)]
pub struct RtlParseError {
    pub message: String,
}

impl std::fmt::Display for RtlParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RTL parse error: {}", self.message)
    }
}

/// Parse a Verilog module declaration and extract its structure.
pub fn parse_verilog_module(src: &str) -> Result<RtlModule, RtlParseError> {
    // Strip comments
    let cleaned = strip_comments(src);

    // Find module declaration (must be at word boundary — not inside another word)
    let module_start = find_keyword(&cleaned, "module")
        .ok_or_else(|| RtlParseError { message: "no 'module' keyword found".into() })?;

    // Check for endmodule
    if !cleaned.contains("endmodule") {
        return Err(RtlParseError { message: "no 'endmodule' found".into() });
    }

    // Extract module name
    let after_module = &cleaned[module_start + 7..];
    let name_end = after_module.find(|c: char| c == '(' || c == ';' || c.is_whitespace())
        .unwrap_or(after_module.len());
    let module_name = after_module[..name_end].trim().to_string();

    let mut module = RtlModule {
        name: module_name,
        ports: Vec::new(),
        signals: Vec::new(),
        params: Vec::new(),
        clocks: Vec::new(),
    };

    // Parse ANSI-style port list: module name ( ... );
    if let Some(paren_start) = after_module.find('(') {
        if let Some(paren_end) = find_balanced_paren(&after_module[paren_start..]) {
            let port_list = &after_module[paren_start + 1..paren_start + paren_end];
            parse_port_list(port_list, &mut module.ports);
        }
    }

    // Parse body: signals, parameters, always blocks
    let body_start = cleaned.find(';').unwrap_or(0) + 1;
    let body_end = cleaned.rfind("endmodule").unwrap_or(cleaned.len());
    let body = &cleaned[body_start..body_end];

    for line in body.lines() {
        let trimmed = line.trim();
        parse_body_line(trimmed, &mut module);
    }

    Ok(module)
}

/// Find a keyword at a word boundary (not inside another identifier).
fn find_keyword(input: &str, keyword: &str) -> Option<usize> {
    let klen = keyword.len();
    for i in 0..input.len() {
        if i + klen > input.len() { break; }
        if &input[i..i + klen] == keyword {
            let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
            let before_ok = i == 0 || !is_ident(input.as_bytes()[i - 1]);
            let after_ok = i + klen >= input.len() || !is_ident(input.as_bytes()[i + klen]);
            if before_ok && after_ok {
                return Some(i);
            }
        }
    }
    None
}

/// Strip single-line (//) and multi-line (/* */) comments.
fn strip_comments(src: &str) -> String {
    let mut result = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            // Single-line comment: skip to end of line
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            // Multi-line comment: skip to */
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2; // skip */
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}

/// Find the closing paren that balances the opening paren at position 0.
fn find_balanced_paren(input: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in input.chars().enumerate() {
        match c {
            '(' => depth += 1,
            ')' => {
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

/// Parse a comma-separated port list.
fn parse_port_list(port_list: &str, ports: &mut Vec<RtlPort>) {
    for port_decl in port_list.split(',') {
        let trimmed = port_decl.trim();
        if trimmed.is_empty() { continue; }
        if let Some(port) = parse_port_decl(trimmed) {
            ports.push(port);
        }
    }
}

/// Parse a single port declaration like "input [7:0] data".
fn parse_port_decl(decl: &str) -> Option<RtlPort> {
    let tokens: Vec<&str> = decl.split_whitespace().collect();
    if tokens.is_empty() { return None; }

    let mut idx = 0;
    let direction = match tokens.get(idx)?.to_lowercase().as_str() {
        "input" => { idx += 1; PortDirection::Input }
        "output" => { idx += 1; PortDirection::Output }
        "inout" => { idx += 1; PortDirection::Inout }
        _ => return None,
    };

    // Skip optional wire/reg/logic keyword
    if let Some(tok) = tokens.get(idx) {
        if matches!(tok.to_lowercase().as_str(), "wire" | "reg" | "logic") {
            idx += 1;
        }
    }

    // Check for width: [N:M]
    let width = if let Some(tok) = tokens.get(idx) {
        if tok.starts_with('[') {
            idx += 1;
            parse_width_spec(tok)
        } else {
            1
        }
    } else {
        1
    };

    // Port name
    let name = tokens.get(idx)?.trim_end_matches(|c: char| c == ',' || c == ')' || c == ';');
    if name.is_empty() { return None; }

    Some(RtlPort {
        name: name.to_string(),
        direction,
        width,
    })
}

/// Parse [N:M] width specification, returning the width (N - M + 1).
fn parse_width_spec(spec: &str) -> u32 {
    let inner = spec.trim_start_matches('[').trim_end_matches(']');
    if let Some(colon) = inner.find(':') {
        let high: i32 = inner[..colon].trim().parse().unwrap_or(0);
        let low: i32 = inner[colon + 1..].trim().parse().unwrap_or(0);
        ((high - low).abs() + 1) as u32
    } else {
        1
    }
}

/// Parse a line from the module body.
fn parse_body_line(line: &str, module: &mut RtlModule) {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.is_empty() { return; }

    match tokens[0] {
        "wire" | "reg" | "logic" => {
            let sig_type = match tokens[0] {
                "wire" => SignalType::Wire,
                "reg" => SignalType::Reg,
                "logic" => SignalType::Logic,
                _ => return,
            };
            let mut idx = 1;
            let width = if tokens.get(idx).map(|t| t.starts_with('[')).unwrap_or(false) {
                let w = parse_width_spec(tokens[idx]);
                idx += 1;
                w
            } else {
                1
            };
            if let Some(name) = tokens.get(idx) {
                let name = name.trim_end_matches(';');
                if !name.is_empty() {
                    module.signals.push(RtlSignal {
                        name: name.to_string(),
                        signal_type: sig_type,
                        width,
                    });
                }
            }
        }
        "parameter" | "localparam" => {
            if tokens.len() >= 3 {
                let name = tokens[1].to_string();
                // Skip '=' and get value
                let value = if tokens.len() >= 4 && tokens[2] == "=" {
                    tokens[3].trim_end_matches(';').to_string()
                } else if tokens.len() >= 3 {
                    tokens[2].trim_start_matches('=').trim_end_matches(';').to_string()
                } else {
                    String::new()
                };
                module.params.push(RtlParam { name, value });
            }
        }
        "always" => {
            // Detect clock from always @(posedge/negedge clk)
            let joined = line.to_string();
            if let Some(edge_start) = joined.find("posedge").or_else(|| joined.find("negedge")) {
                let after_edge = &joined[edge_start..];
                let clock_start = after_edge.find(char::is_whitespace).unwrap_or(0) + 1;
                let after_space = after_edge[clock_start..].trim();
                let clock_end = after_space.find(|c: char| c == ')' || c == ',' || c.is_whitespace())
                    .unwrap_or(after_space.len());
                let clock_name = &after_space[..clock_end];
                if !clock_name.is_empty() {
                    if !module.clocks.contains(&clock_name.to_string()) {
                        module.clocks.push(clock_name.to_string());
                    }
                }
            }
        }
        _ => {}
    }
}
