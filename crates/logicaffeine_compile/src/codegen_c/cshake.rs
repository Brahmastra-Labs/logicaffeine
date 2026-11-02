//! Structural reachability over an emitted C translation unit — split the runtime into top-level
//! definition UNITS, then keep only those reachable (transitively) from a set of root symbols.
//!
//! This is the shared tree-shaking pass for the C-derived tiers. The CUDA device-lift uses it so
//! `nvcc` only compiles the slice a program actually references (a float one-liner pulls in ~3
//! functions, not the whole ~1900-line runtime); the same pass can later shrink the C / Tenstorrent
//! output. It is an OVER-APPROXIMATION (a body's every identifier is treated as a dependency), so it
//! never drops a needed unit; and it is verified downstream — a genuinely missed edge surfaces as a
//! compiler "undefined symbol", never a silent miscompile.
//!
//! The parser is a small byte scanner that understands C string/char/line/block-comment lexemes (so a
//! brace inside `"{"` does not throw off nesting) and the three top-level shapes the runtime uses:
//! `typedef … ;`, `static … (…) { … }` / `… (…) ;` functions, and file-scope globals.

use std::collections::{BTreeSet, VecDeque};

/// One top-level C construct: a typedef, a `static` function definition or forward declaration, or a
/// file-scope global. `defined` is the symbol(s) it introduces; `references` is every identifier it
/// mentions (an over-approximation of its dependencies); `text` is the exact source span (with any
/// leading comment/blank lines attached, so units travel with their doc comment).
#[derive(Debug, Clone)]
pub struct CUnit {
    pub defined: Vec<String>,
    pub references: BTreeSet<String>,
    pub text: String,
    pub kind: UnitKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitKind {
    Typedef,
    /// A `static … (…) { … }` definition or a `… (…);` forward declaration.
    Function,
    /// A file-scope `static`/`const` global (data, not a function).
    Global,
    /// A `#…` preprocessor line — carried through the splitter but never a dependency target.
    Preproc,
}

/// Collect every C identifier in `s`, skipping string/char literals and comments.
pub fn c_identifiers(s: &str) -> BTreeSet<String> {
    let b = s.as_bytes();
    let mut out = BTreeSet::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
            }
            b'\'' => {
                i += 1;
                while i < b.len() && b[i] != b'\'' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'/' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'*' => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
            }
            _ if c == b'_' || c.is_ascii_alphabetic() => {
                let start = i;
                while i < b.len() && (b[i] == b'_' || b[i].is_ascii_alphanumeric()) {
                    i += 1;
                }
                out.insert(s[start..i].to_string());
            }
            _ => i += 1,
        }
    }
    out
}

/// A byte cursor that steps one lexeme at a time, reporting the significant structural bytes (`{`,
/// `}`, `;`, `(`, `)`, `=`, `#`) at brace/paren depth while skipping over string/char/comment content.
struct Scan<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Scan<'a> {
    /// Advance past one lexeme starting at `self.i`. Returns the significant byte if this lexeme is a
    /// bare structural char (else `None` for identifiers/whitespace/strings/comments). Always makes
    /// progress.
    fn step(&mut self) -> Option<u8> {
        let b = self.b;
        let c = b[self.i];
        match c {
            b'"' => {
                self.i += 1;
                while self.i < b.len() && b[self.i] != b'"' {
                    if b[self.i] == b'\\' {
                        self.i += 1;
                    }
                    self.i += 1;
                }
                self.i += 1;
                None
            }
            b'\'' => {
                self.i += 1;
                while self.i < b.len() && b[self.i] != b'\'' {
                    if b[self.i] == b'\\' {
                        self.i += 1;
                    }
                    self.i += 1;
                }
                self.i += 1;
                None
            }
            b'/' if self.i + 1 < b.len() && b[self.i + 1] == b'/' => {
                while self.i < b.len() && b[self.i] != b'\n' {
                    self.i += 1;
                }
                None
            }
            b'/' if self.i + 1 < b.len() && b[self.i + 1] == b'*' => {
                self.i += 2;
                while self.i + 1 < b.len() && !(b[self.i] == b'*' && b[self.i + 1] == b'/') {
                    self.i += 1;
                }
                self.i += 2;
                None
            }
            _ if c == b'_' || c.is_ascii_alphabetic() => {
                while self.i < b.len() && (b[self.i] == b'_' || b[self.i].is_ascii_alphanumeric()) {
                    self.i += 1;
                }
                None
            }
            _ => {
                self.i += 1;
                Some(c)
            }
        }
    }
}

/// Split an emitted C translation unit into top-level [`CUnit`]s. Leading comments/blank lines attach
/// to the unit that follows. `#…` preprocessor directives, backslash-continued lines included, are their own
/// [`UnitKind::Preproc`] units.
pub fn parse_c_units(src: &str) -> Vec<CUnit> {
    let b = src.as_bytes();
    let mut units = Vec::new();
    let mut unit_start = 0usize; // includes leading ws/comments
    let mut i = 0usize;
    // Find where the current unit's *code* begins (first significant, non-ws/comment/`#`).
    while i < b.len() {
        // Skip whitespace.
        while i < b.len() && (b[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        // A comment before a definition — skip it (stays attached via `unit_start`).
        if b[i] == b'/' && i + 1 < b.len() && (b[i + 1] == b'/' || b[i + 1] == b'*') {
            let mut sc = Scan { b, i };
            sc.step();
            i = sc.i;
            continue;
        }
        // A preprocessor directive is its own unit (dropped downstream: no defined symbol). It ends at the
        // first newline no backslash escapes, so a macro continued over lines stays whole.
        if b[i] == b'#' {
            let ls = unit_start;
            let continued = |nl: usize| {
                nl > 0
                    && (b[nl - 1] == b'\\' || (nl > 1 && b[nl - 1] == b'\r' && b[nl - 2] == b'\\'))
            };
            while i < b.len() && (b[i] != b'\n' || continued(i)) {
                i += 1;
            }
            if i < b.len() {
                i += 1;
            }
            let text = src[ls..i].to_string();
            units.push(CUnit {
                defined: vec![],
                references: BTreeSet::new(),
                text,
                kind: UnitKind::Preproc,
            });
            unit_start = i;
            continue;
        }
        // The code of a real definition begins here. Consume to its terminator.
        let code_start = i;
        let starts_typedef = src[code_start..].trim_start().starts_with("typedef");
        let mut sc = Scan { b, i };
        let mut depth = 0i32;
        // For a function DEF the terminating `}` returns depth to 0 and its matching `{` was preceded
        // by `)`. Track the byte just before each depth 0→1 `{` to tell a body from an initializer/struct.
        let mut last_sig_before_brace: u8 = 0;
        let mut last_sig: u8 = 0;
        let mut end = b.len();
        while sc.i < b.len() {
            let at = sc.i;
            if let Some(ch) = sc.step() {
                match ch {
                    b'{' => {
                        if depth == 0 {
                            last_sig_before_brace = last_sig;
                        }
                        depth += 1;
                    }
                    b'}' => {
                        depth -= 1;
                        if depth == 0 && !starts_typedef && last_sig_before_brace == b')' {
                            // Function body just closed — unit ends here (no trailing `;`).
                            end = sc.i;
                            break;
                        }
                    }
                    b';' if depth == 0 => {
                        end = sc.i;
                        break;
                    }
                    _ => {}
                }
                if !(ch as char).is_whitespace() {
                    last_sig = ch;
                }
                let _ = at;
            }
        }
        let text = src[unit_start..end].to_string();
        let code = &src[code_start..end];
        let (defined, kind) = classify_unit(code, starts_typedef);
        units.push(CUnit {
            defined,
            references: c_identifiers(code),
            text,
            kind,
        });
        i = end;
        unit_start = end;
    }
    units
}

/// Extract the defined symbol name(s) and the kind of a unit from its code span (no leading comments).
fn classify_unit(code: &str, starts_typedef: bool) -> (Vec<String>, UnitKind) {
    if starts_typedef {
        // `typedef … Name;` — the LAST identifier in the span. Handles `typedef struct {…} Name;`
        // (whose inner `;`s sit inside the braces) and plain `typedef T Name;`. The runtime uses no
        // function-pointer typedefs (no `typedef R (*Name)(…);`), so last-identifier is exact.
        let name = last_ident(code);
        return (name.into_iter().collect(), UnitKind::Typedef);
    }
    // `struct Name { … };` — a struct defined apart from its typedef defines its tag, the name a
    // `typedef struct Name Name;` elsewhere gives it.
    if let Some(tag) = struct_definition_tag(code) {
        return (vec![tag], UnitKind::Typedef);
    }
    // A function: the identifier immediately before the first top-level `(`.
    if let Some(name) = ident_before_first_paren(code) {
        return (vec![name], UnitKind::Function);
    }
    // Otherwise a global: the last identifier before the first `;`/`=`/`[`.
    let name = global_name(code);
    (name.into_iter().collect(), UnitKind::Global)
}

/// The tag of a `struct Tag { … };` definition: the code begins `struct Tag` and its body opens before any `(`.
fn struct_definition_tag(code: &str) -> Option<String> {
    let rest = code.trim_start().strip_prefix("struct")?;
    if !rest.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    let rest = rest.trim_start();
    let end = rest
        .find(|c: char| !(c == '_' || c.is_ascii_alphanumeric()))
        .unwrap_or(rest.len());
    let (tag, after) = rest.split_at(end);
    let brace = after.find('{')?;
    if tag.is_empty() || after[..brace].trim() != "" || after[..brace].contains('(') {
        return None;
    }
    Some(tag.to_string())
}

fn ident_before_first_paren(code: &str) -> Option<String> {
    let b = code.as_bytes();
    let mut sc = Scan { b, i: 0 };
    let mut last_ident: Option<(usize, usize)> = None;
    while sc.i < b.len() {
        let start = sc.i;
        let c = b[sc.i];
        if c == b'_' || c.is_ascii_alphabetic() {
            sc.step();
            last_ident = Some((start, sc.i));
        } else if c == b'(' {
            // A GCC attribute specifier `__attribute__((…))` opens a `(` that is NOT the parameter
            // list — skip its balanced parens so the real function name (which follows) is found.
            let is_attr = last_ident.map(|(s, e)| &code[s..e]) == Some("__attribute__");
            sc.step(); // consume the '('
            if is_attr {
                let mut depth = 1i32;
                while sc.i < b.len() && depth > 0 {
                    match sc.step() {
                        Some(b'(') => depth += 1,
                        Some(b')') => depth -= 1,
                        _ => {}
                    }
                }
                last_ident = None;
                continue;
            }
            return last_ident.map(|(s, e)| code[s..e].to_string());
        } else {
            sc.step();
        }
    }
    None
}

fn last_ident(code: &str) -> Option<String> {
    let b = code.as_bytes();
    let mut sc = Scan { b, i: 0 };
    let mut last: Option<(usize, usize)> = None;
    while sc.i < b.len() {
        let start = sc.i;
        let c = b[sc.i];
        if c == b'_' || c.is_ascii_alphabetic() {
            sc.step();
            last = Some((start, sc.i));
        } else {
            sc.step();
        }
    }
    last.map(|(s, e)| code[s..e].to_string())
}

fn global_name(code: &str) -> Option<String> {
    let b = code.as_bytes();
    let mut sc = Scan { b, i: 0 };
    let mut last_ident: Option<(usize, usize)> = None;
    while sc.i < b.len() {
        let start = sc.i;
        let c = b[sc.i];
        if c == b'_' || c.is_ascii_alphabetic() {
            sc.step();
            last_ident = Some((start, sc.i));
        } else if c == b';' || c == b'=' || c == b'[' {
            break;
        } else {
            sc.step();
        }
    }
    last_ident.map(|(s, e)| code[s..e].to_string())
}

/// A `#define`'s macro name and the names its replacement mentions, its own parameters left out: a macro is
/// expanded where it is named, so what it names is reached through it. `None` for any other directive. A
/// preprocessor unit is its one directive behind whatever comments [`parse_c_units`] attached to it.
pub fn macro_references(unit: &CUnit) -> Option<(String, BTreeSet<String>)> {
    if unit.kind != UnitKind::Preproc {
        return None;
    }
    let mut text = unit.text.as_str();
    loop {
        text = text.trim_start();
        if let Some(line) = text.strip_prefix("//") {
            text = line.split_once('\n').map_or("", |(_, rest)| rest);
        } else if let Some(block) = text.strip_prefix("/*") {
            text = block.split_once("*/").map_or("", |(_, rest)| rest);
        } else {
            break;
        }
    }
    let directive = text.strip_prefix('#')?.trim_start();
    let rest = directive.strip_prefix("define")?;
    if !rest.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    let rest = rest.trim_start();
    let end = rest
        .find(|c: char| c != '_' && !c.is_ascii_alphanumeric())
        .unwrap_or(rest.len());
    let name = &rest[..end];
    if name.is_empty() {
        return None;
    }
    let after = &rest[end..];
    let (params, replacement): (BTreeSet<&str>, &str) = match after.strip_prefix('(') {
        Some(list) => {
            let close = list.find(')').unwrap_or(list.len());
            (
                list[..close].split(',').map(str::trim).collect(),
                &list[(close + 1).min(list.len())..],
            )
        }
        None => (BTreeSet::new(), after),
    };
    let references = c_identifiers(replacement)
        .into_iter()
        .filter(|r| r != name && !params.contains(r.as_str()))
        .collect();
    Some((name.to_string(), references))
}

/// Indices (in original order) of the units transitively reachable from `roots`. A unit is reached
/// when any symbol it defines is in the frontier; reaching it adds every identifier it references.
pub fn reachable_units(units: &[CUnit], roots: &BTreeSet<String>) -> Vec<usize> {
    // symbol → indices of units that define it (a symbol may have a forward decl + a definition).
    let mut by_symbol: std::collections::HashMap<&str, Vec<usize>> =
        std::collections::HashMap::new();
    for (idx, u) in units.iter().enumerate() {
        for d in &u.defined {
            by_symbol.entry(d.as_str()).or_default().push(idx);
        }
    }
    let mut wanted: BTreeSet<usize> = BTreeSet::new();
    let mut queue: VecDeque<String> = roots.iter().cloned().collect();
    let mut seen_sym: BTreeSet<String> = roots.iter().cloned().collect();
    while let Some(sym) = queue.pop_front() {
        if let Some(idxs) = by_symbol.get(sym.as_str()) {
            for &idx in idxs {
                if wanted.insert(idx) {
                    for r in &units[idx].references {
                        if seen_sym.insert(r.clone()) {
                            queue.push_back(r.clone());
                        }
                    }
                }
            }
        }
    }
    wanted.into_iter().collect()
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_c/cshake/tests.rs"]
mod tests;
