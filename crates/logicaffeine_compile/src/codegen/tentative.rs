//! Declarations a later write may not need.
//!
//! A counter its loop leaves to be set again first inside a nested block is declared without a value where the loop
//! was, and a binding the program declares mutable is `mut` only if the emitted code writes it. Whether that binding is ever written is known only once the nested blocks are written: each may count with a
//! `for` of its own, which shadows it. So the declaration is emitted tentatively ([`tentative`]): one line, tagged
//! with an id. A write to a binding that may be such a declaration marks it needed ([`need`]), and [`settle`], run
//! over the finished program, keeps the needed lines without their tags and drops the rest. A tagged line is already
//! the declaration, so code that never reaches [`settle`] stays correct.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;

thread_local! {
    static NEXT: Cell<usize> = const { Cell::new(0) };
    static NEEDED: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
}

const TAG: &str = " //@tentative ";

/// Number the tentative declarations of the program about to be emitted from the first, with none of them needed yet.
pub(crate) fn reset() {
    NEXT.with(|n| n.set(0));
    NEEDED.with(|n| n.borrow_mut().clear());
}

/// The tentative declaration of `name` at `indent`, mutable when a run of later statements may set it twice, and
/// its id.
pub(crate) fn tentative(indent: &str, name: &str, mutable: bool) -> (usize, String) {
    let id = NEXT.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    let mutable = if mutable { "mut " } else { "" };
    (id, format!("{indent}let {mutable}{name}: i64;{TAG}{id}\n"))
}

/// A write needs the tentative declaration `id`.
pub(crate) fn need(id: usize) {
    NEEDED.with(|n| n.borrow_mut().insert(id));
}

/// The start of a program's `let` of a literal, marked for [`settle`]: dropped where its scope never names the binding.
/// A comment to rustc.
pub(crate) const MAYBE_UNREAD: &str = "/*@unread*/";

/// Whether `name` is named from `from` to the close of the block around it: as an identifier that is not a field or a
/// path segment, or as a format string's `{name}` / `{name:…}`. A `for name in …` body names its own `name`, and so does
/// everything after a `let name` in the block that declares it; the iterator and the initializer still name this one.
fn named_in_scope(bytes: &[u8], from: usize, name: &str) -> bool {
    let name_bytes = name.as_bytes();
    let in_format = |literal: &[u8]| {
        let mut held = Vec::with_capacity(name_bytes.len() + 1);
        held.push(b'{');
        held.extend_from_slice(name_bytes);
        literal.windows(held.len() + 1).any(|w| w.starts_with(&held) && matches!(w[held.len()], b'}' | b':'))
    };
    // The identifier after `at` and the whitespace before it, if one follows.
    let next_ident = |at: usize| -> Option<(usize, usize)> {
        let start = at + bytes[at..].iter().take_while(|b| b.is_ascii_whitespace()).count();
        let end = ident_end(bytes, start);
        (end > start).then_some((start, end))
    };
    // Where the block opening at or after `at` (at this depth) closes.
    let block_end = |at: usize| -> Option<usize> {
        let mut depth = 0i32;
        let mut i = at;
        while i < bytes.len() {
            if let Some(end) = literal_end(bytes, i) {
                i = end;
                continue;
            }
            match bytes[i] {
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                b'{' if depth == 0 => return matching_brace(bytes, i),
                _ => {}
            }
            i += 1;
        }
        None
    };
    let mut depth = 0i32;
    // Depths at which a `let name` shadows this binding until its block closes.
    let mut shadowed_from: Option<i32> = None;
    let mut i = from;
    while i < bytes.len() {
        if let Some(end) = literal_end(bytes, i) {
            if shadowed_from.is_none() && in_format(&bytes[i..end]) {
                return true;
            }
            i = end;
            continue;
        }
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
                if shadowed_from.is_some_and(|d| depth < d) {
                    shadowed_from = None;
                }
            }
            b if is_ident_byte(b) && (i == 0 || !is_ident_byte(bytes[i - 1])) => {
                let end = ident_end(bytes, i);
                let word = &bytes[i..end];
                if word == b"for" {
                    if let Some((s, e)) = next_ident(end) {
                        if &bytes[s..e] == name_bytes && next_ident(e).is_some_and(|(a, b)| &bytes[a..b] == b"in") {
                            // The iterator names this binding; the body names the loop's.
                            let (_, in_end) = next_ident(e).unwrap();
                            let Some(body_end) = block_end(in_end) else { return true };
                            let body_start = in_end + bytes[in_end..body_end].iter().position(|&b| b == b'{').unwrap_or(0);
                            if shadowed_from.is_none() && named_in_scope_until(bytes, in_end, body_start, name) {
                                return true;
                            }
                            i = body_end + 1;
                            continue;
                        }
                    }
                }
                if word == b"let" {
                    let mut at = end;
                    if let Some((s, e)) = next_ident(at) {
                        if &bytes[s..e] == b"mut" {
                            at = e;
                        }
                    }
                    if let Some((s, e)) = next_ident(at) {
                        if &bytes[s..e] == name_bytes {
                            // Its initializer names this binding; what follows in its block names the new one.
                            let Some(stmt_end) = statement_end(bytes, e) else { return true };
                            if shadowed_from.is_none() && named_in_scope_until(bytes, e, stmt_end, name) {
                                return true;
                            }
                            if depth == 0 {
                                return false;
                            }
                            shadowed_from.get_or_insert(depth);
                            i = stmt_end;
                            continue;
                        }
                    }
                }
                if shadowed_from.is_none() && word == name_bytes {
                    let before = bytes[..i].trim_ascii_end();
                    let after = bytes[end..].trim_ascii_start();
                    let field_label = after.first() == Some(&b':') && after.get(1) != Some(&b':');
                    // A member is reached through one `.`; `..` ends a range, which reads the binding.
                    let member = before.ends_with(b".") && !before.ends_with(b"..");
                    if !(member || before.ends_with(b"::") || field_label) {
                        return true;
                    }
                }
                i = end;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// Whether `name` is named in `bytes[from..to]`, a stretch with no scope of its own to leave.
fn named_in_scope_until(bytes: &[u8], from: usize, to: usize, name: &str) -> bool {
    let mut stretch = bytes[..to].to_vec();
    stretch.push(b'}');
    named_in_scope(&stretch, from, name)
}

/// Where the brace opening at `open` closes.
fn matching_brace(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    while i < bytes.len() {
        if let Some(end) = literal_end(bytes, i) {
            i = end;
            continue;
        }
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The lines of `code` holding a [`MAYBE_UNREAD`] `let` whose scope never names its binding, each numbered by the
/// newlines before it.
fn unread_lets(code: &str) -> HashSet<usize> {
    let bytes = code.as_bytes();
    let mut unread = HashSet::new();
    let mut from = 0;
    while let Some(found) = code[from..].find(MAYBE_UNREAD) {
        let at = from + found;
        from = at + MAYBE_UNREAD.len();
        let rest = code[from..].strip_prefix("let ").unwrap_or("");
        let rest = rest.strip_prefix(MAYBE_MUT).or_else(|| rest.strip_prefix("mut ")).unwrap_or(rest);
        let name_start = code.len() - rest.len();
        let name_end = ident_end(bytes, name_start);
        if name_end == name_start {
            continue;
        }
        let named = statement_end(bytes, name_end)
            .is_none_or(|end| named_in_scope(bytes, end, &code[name_start..name_end]));
        if !named {
            unread.insert(code[..at].matches('\n').count());
        }
    }
    unread
}

/// The `mut ` of a binding the program declares mutable, marked for [`settle_mutability`]: still `mut` to rustc.
pub(crate) const MAYBE_MUT: &str = "mut/*@maybe*/ ";

/// `code` with each [`MAYBE_MUT`] a plain `mut ` where the binding it declares is written in its scope, and gone
/// where it is not. Its scope runs from the end of its `let` to the close of the block holding it; a write is an
/// assignment to the name (`x = …`, `x += …`), a `mut` before it (`&mut x`), or a method call, an index, a call
/// through it or the first argument of `write!`/`writeln!`, any of which may take `&mut x`. A later binding of the
/// same name counts its writes too, so the answer only errs toward `mut`.
pub(crate) fn settle_mutability(code: &str) -> String {
    let bytes = code.as_bytes();
    let mut out = String::with_capacity(code.len());
    let mut copied = 0;
    while let Some(found) = code[copied..].find(MAYBE_MUT) {
        let at = copied + found;
        let name_start = at + MAYBE_MUT.len();
        let name_end = ident_end(bytes, name_start);
        let name = &code[name_start..name_end];
        let reference = code[name_end..].trim_start().strip_prefix(':').is_some_and(|ty| ty.trim_start().starts_with('&'));
        let written = statement_end(bytes, name_end)
            .is_some_and(|from| written_in_scope(bytes, from, name, reference));
        out.push_str(&code[copied..at]);
        if written {
            out.push_str("mut ");
        }
        copied = name_start;
    }
    out.push_str(&code[copied..]);
    out
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// Where the identifier at `at` (`x`, `r#type`) ends.
fn ident_end(bytes: &[u8], at: usize) -> usize {
    let mut end = at;
    if bytes[at..].starts_with(b"r#") {
        end += 2;
    }
    while end < bytes.len() && is_ident_byte(bytes[end]) {
        end += 1;
    }
    end
}

/// Where the string, character or comment starting at `at` ends, if one does; a lifetime is not a character.
fn literal_end(bytes: &[u8], at: usize) -> Option<usize> {
    match bytes[at] {
        b'"' => {
            let mut i = at + 1;
            while i < bytes.len() {
                match bytes[i] {
                    b'\\' => i += 2,
                    b'"' => return Some(i + 1),
                    _ => i += 1,
                }
            }
            Some(bytes.len())
        }
        b'r' if bytes.get(at + 1).is_some_and(|&b| b == b'"' || b == b'#')
            && (at == 0 || !is_ident_byte(bytes[at - 1])) =>
        {
            let hashes = bytes[at + 1..].iter().take_while(|&&b| b == b'#').count();
            if bytes.get(at + 1 + hashes) != Some(&b'"') {
                return None;
            }
            let mut close = vec![b'"'];
            close.extend(std::iter::repeat_n(b'#', hashes));
            let body = at + 2 + hashes;
            Some(bytes[body..].windows(close.len()).position(|w| w == close).map_or(bytes.len(), |p| body + p + close.len()))
        }
        b'\'' => {
            // `'a'`, `'\n'`, `'\u{41}'` — and not a lifetime `'a`.
            let close = if bytes.get(at + 1) == Some(&b'\\') {
                bytes[at + 2..].iter().position(|&b| b == b'\'').map(|p| at + 2 + p)
            } else {
                let width = std::str::from_utf8(&bytes[at + 1..(at + 5).min(bytes.len())])
                    .ok()
                    .or_else(|| std::str::from_utf8(&bytes[at + 1..(at + 2).min(bytes.len())]).ok())
                    .and_then(|s| s.chars().next())
                    .map_or(1, char::len_utf8);
                (bytes.get(at + 1 + width) == Some(&b'\'')).then_some(at + 1 + width)
            };
            close.map(|c| c + 1)
        }
        b'/' if bytes.get(at + 1) == Some(&b'/') => {
            Some(bytes[at..].iter().position(|&b| b == b'\n').map_or(bytes.len(), |p| at + p))
        }
        b'/' if bytes.get(at + 1) == Some(&b'*') => {
            Some(bytes[at + 2..].windows(2).position(|w| w == b"*/").map_or(bytes.len(), |p| at + 2 + p + 2))
        }
        _ => None,
    }
}

/// Where the statement running on from `at` ends: after its `;` at depth zero.
fn statement_end(bytes: &[u8], at: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = at;
    while i < bytes.len() {
        if let Some(end) = literal_end(bytes, i) {
            i = end;
            continue;
        }
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            b';' if depth == 0 => return Some(i + 1),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Whether `name` is written from `from` to the close of the block around it ([`settle_mutability`]); a `reference`
/// binding is written only by an assignment or a `&mut` of itself.
fn written_in_scope(bytes: &[u8], from: usize, name: &str, reference: bool) -> bool {
    let name = name.as_bytes();
    let mut depth = 0i32;
    let mut i = from;
    while i < bytes.len() {
        if let Some(end) = literal_end(bytes, i) {
            i = end;
            continue;
        }
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            b if is_ident_byte(b) && (i == 0 || !is_ident_byte(bytes[i - 1])) => {
                let end = ident_end(bytes, i);
                if &bytes[i..end] == name && writes(bytes, i, end, reference) {
                    return true;
                }
                i = end;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// Methods that take their receiver `&self` on every type the emitter writes: a call of one reads the binding.
const SHARED_RECEIVER: &[&str] = &[
    "borrow",
    "len",
    "is_empty",
    "iter",
    "clone",
    "contains",
    "first",
    "last",
    "to_vec",
    "to_string",
    "as_str",
    "chars",
    "bytes",
    "keys",
    "values",
    "logos_get",
    "logos_item",
    "logos_item_front",
    "logos_contains",
];

/// Whether `after`, the code after a `.`, calls a [`SHARED_RECEIVER`] method.
fn calls_shared(after: &[u8]) -> bool {
    let name_end = ident_end(after, 0);
    after[name_end..].trim_ascii_start().starts_with(b"(")
        && SHARED_RECEIVER.iter().any(|m| m.as_bytes() == &after[..name_end])
}

/// Whether the occurrence of a name at `start..end` writes it, or may; through a `reference`, a method call, an index
/// or a call writes what it refers to, not the binding, and a call of a method that borrows its receiver shared reads
/// it.
fn writes(bytes: &[u8], start: usize, end: usize, reference: bool) -> bool {
    let before = bytes[..start].trim_ascii_end();
    if before.ends_with(b".") || before.ends_with(b"::") {
        return false;
    }
    if before.ends_with(b"mut") && before.len() >= 3 && !before[..before.len() - 3].last().is_some_and(|&b| is_ident_byte(b)) {
        return true;
    }
    // `write!(s, …)` and `writeln!(s, …)` borrow their first argument `&mut`.
    if before.ends_with(b"write!(") || before.ends_with(b"writeln!(") {
        return !reference;
    }
    let after = bytes[end..].trim_ascii_start();
    match after {
        [b'=', b'=' | b'>', ..] => false,
        [b'=', ..] => true,
        [b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^', b'=', ..] => true,
        [b'<', b'<', b'=', ..] | [b'>', b'>', b'=', ..] => true,
        [b'.', b'.', ..] => false,
        [b'.', method @ ..] if calls_shared(method.trim_ascii_start()) => false,
        [b'.' | b'[' | b'(', ..] => !reference,
        _ => false,
    }
}

/// `code` with each needed tentative declaration kept, untagged, and every other one dropped; and the lines dropped,
/// each numbered by the newlines before it.
pub(crate) fn settle(code: &str) -> (String, Vec<u32>) {
    let mut settled = String::with_capacity(code.len());
    let mut dropped = Vec::new();
    let mut seen = HashSet::new();
    let unread = unread_lets(code);
    NEEDED.with(|needed| {
        let needed = needed.borrow();
        for (number, line) in code.split_inclusive('\n').enumerate() {
            if line.contains(MAYBE_UNREAD) {
                if unread.contains(&number) {
                    dropped.push(number as u32);
                } else {
                    settled.push_str(&line.replacen(MAYBE_UNREAD, "", 1));
                }
                continue;
            }
            let tagged = line
                .trim_end_matches('\n')
                .rsplit_once(TAG)
                .and_then(|(decl, id)| Some((decl, id.parse::<usize>().ok()?)));
            match tagged {
                Some((decl, id)) => {
                    seen.insert(id);
                    if needed.contains(&id) {
                        settled.push_str(decl);
                        settled.push('\n');
                    } else {
                        dropped.push(number as u32);
                    }
                }
                None => settled.push_str(line),
            }
        }
    });
    // A tagged line copied into both arms of a versioned loop is one declaration: its id is settled once all its
    // copies are.
    NEEDED.with(|needed| needed.borrow_mut().retain(|id| !seen.contains(id)));
    (settled, dropped)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen/tentative/tests.rs"]
mod tests;
