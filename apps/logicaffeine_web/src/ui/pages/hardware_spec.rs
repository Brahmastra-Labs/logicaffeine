//! The line grammar the Hardware-mode specs share — `pigeons: 6`, `registers: 3`, `tmp: 1-5`,
//! and each line's `# …` comment — read one way by their parsers and by the editor, so what the
//! editor paints as a comment is exactly what the parsers skip.

/// The pigeonhole spec's one field: how many pigeons go into one fewer holes.
pub const PIGEONS: &str = "pigeons";
/// The register-allocation spec's budget: how many physical registers there are.
pub const REGISTERS: &str = "registers";
/// Every field a Hardware spec names before its value.
pub const FIELDS: [&str; 2] = [PIGEONS, REGISTERS];
/// How many pigeons a spec may ask for: one pigeon with no hole is degenerate, and past twenty
/// the picture stops being legible.
pub const PIGEONS_RANGE: std::ops::RangeInclusive<usize> = 2..=20;

/// Whether a run of `#` whose next character is `after` opens a comment: it does when a space or
/// the end of the line follows (`# note`, `## Title`). Verilog's `#5` and `#(10)` delays are code.
pub fn opens_comment(after: Option<char>) -> bool {
    after.map_or(true, char::is_whitespace)
}

/// Where `line`'s comment starts: the first run of `#` that [`opens_comment`].
pub fn comment_start(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' {
            let run_end = i + bytes[i..].iter().take_while(|&&b| b == b'#').count();
            if opens_comment(line[run_end..].chars().next()) {
                return Some(i);
            }
            i = run_end;
        } else {
            i += 1;
        }
    }
    None
}

/// A spec line without its comment, trimmed.
pub fn content(line: &str) -> &str {
    line[..comment_start(line).unwrap_or(line.len())].trim()
}

/// Whether `word` names `field` the way the parsers accept it: lowercase, or capitalised.
pub fn is_field(word: &str, field: &str) -> bool {
    let mut chars = field.chars();
    let capitalised = chars.next().map(|first| first.to_uppercase().chain(chars).collect::<String>());
    word == field || capitalised.is_some_and(|c| word == c)
}

/// The value after `field:` on a spec line, trimmed and without the line's comment, when the line
/// names that field.
pub fn field_value<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let (name, value) = content(line).split_once(':')?;
    is_field(name, field).then(|| value.trim())
}

/// What the editor's hover says about the word at byte `offset` of a spec: the field it names,
/// or the live range it reads. Markdown, as the language server's hovers are.
pub fn hover(source: &str, offset: usize) -> Option<String> {
    let kind = kind(source)?;
    let (line_at, line) = lines(source).find(|(at, line)| (*at..=at + line.len()).contains(&offset))?;
    let entry = entry(line)?;
    let at = offset - line_at;
    let on_name = (entry.name_at..entry.name_at + entry.name.len()).contains(&at);
    let on_value = (entry.value_at..entry.value_at + entry.value.len()).contains(&at);
    if !on_name && !on_value {
        return None;
    }
    if is_field(entry.name, PIGEONS) {
        let (low, high) = (PIGEONS_RANGE.start(), PIGEONS_RANGE.end());
        return Some(format!(
            "**pigeons**: how many pigeons fly into one fewer holes, from {low} to {high}. They can never all fit, and the prover certifies why, with no Z3."
        ));
    }
    if is_field(entry.name, REGISTERS) {
        return Some(
            "**registers**: the physical register budget. Every other line is a variable's live range, `name: first-last`, and the allocator certifies whether they fit.".to_string(),
        );
    }
    match (kind, live_range(entry.value)) {
        (Kind::Registers, Some((first, last))) => Some(format!("**{}** is live from instruction {first} to {last}.", entry.name)),
        _ => None,
    }
}

/// What is wrong in a spec, placed under the value at fault: a field its parser would refuse, a
/// live range that never starts or ends. A spec that names no field has nothing to check here.
pub fn problems(source: &str) -> Vec<crate::ui::components::problems::Problem> {
    use crate::ui::components::problems::{Problem, Severity};
    let Some(kind) = kind(source) else { return Vec::new() };
    let problem = |start: usize, end: usize, message: String| Problem {
        start,
        end,
        severity: Severity::Error,
        unnecessary: false,
        message,
        source: Some("spec".to_string()),
        code: None,
        docs: None,
    };
    let mut out = Vec::new();
    for (line_at, line) in lines(source) {
        let body = content(line);
        if body.is_empty() {
            continue;
        }
        let entry = entry(line);
        // The value's span, or the name's when the value is blank, so a squiggle has text under it.
        let value_span = |e: &Entry| {
            if e.value.is_empty() {
                (line_at + e.name_at, line_at + e.name_at + e.name.len())
            } else {
                (line_at + e.value_at, line_at + e.value_at + e.value.len())
            }
        };
        match (kind, entry) {
            (Kind::Pigeons, Some(e)) if is_field(e.name, PIGEONS) => {
                let (start, end) = value_span(&e);
                let (low, high) = (PIGEONS_RANGE.start(), PIGEONS_RANGE.end());
                match e.value.parse::<usize>() {
                    Err(_) => out.push(problem(start, end, format!("`pigeons` takes a whole number of pigeons, from {low} to {high}."))),
                    Ok(n) if !PIGEONS_RANGE.contains(&n) => {
                        out.push(problem(start, end, format!("`pigeons` takes from {low} to {high} pigeons; {n} is out of range.")))
                    }
                    Ok(_) => {}
                }
            }
            (Kind::Pigeons, _) => {
                let start = line_at + (line.len() - line.trim_start().len());
                out.push(problem(start, start + body.len(), "A pigeonhole spec holds one line, `pigeons: N`; this line stops it being read as one.".to_string()));
            }
            (Kind::Registers, Some(e)) if is_field(e.name, REGISTERS) => {
                if e.value.parse::<usize>().is_err() {
                    let (start, end) = value_span(&e);
                    out.push(problem(start, end, "`registers` takes a whole number of registers.".to_string()));
                }
            }
            (Kind::Registers, Some(e)) => {
                let (start, end) = value_span(&e);
                match live_range(e.value) {
                    None => out.push(problem(start, end, format!("`{}` needs a live range `first-last`, like `0-3`.", e.name))),
                    Some((first, last)) if first >= last => out.push(problem(
                        start,
                        end,
                        format!("`{}`'s live range must end after it starts; `{}` does not, so the allocator skips it.", e.name, e.value),
                    )),
                    Some(_) => {}
                }
            }
            (Kind::Registers, None) => {}
        }
    }
    out
}

/// Which spec `source` is, by the field it names.
#[derive(Clone, Copy)]
enum Kind {
    Pigeons,
    Registers,
}

fn kind(source: &str) -> Option<Kind> {
    let names = || lines(source).filter_map(|(_, line)| entry(line)).map(|e| e.name);
    if names().any(|name| is_field(name, PIGEONS)) {
        Some(Kind::Pigeons)
    } else if names().any(|name| is_field(name, REGISTERS)) {
        Some(Kind::Registers)
    } else {
        None
    }
}

/// Each line of `source` without its line break, with the byte offset it starts at.
fn lines(source: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut at = 0;
    source.split_inclusive('\n').map(move |raw| {
        let here = at;
        at += raw.len();
        (here, raw.trim_end_matches(['\n', '\r']))
    })
}

/// A spec line read as `name: value`, with each part's byte offset in the line; the comment is
/// not part of either.
struct Entry<'a> {
    name: &'a str,
    name_at: usize,
    value: &'a str,
    value_at: usize,
}

fn entry(line: &str) -> Option<Entry<'_>> {
    let body = &line[..comment_start(line).unwrap_or(line.len())];
    let colon = body.find(':')?;
    let before = &body[..colon];
    let name = before.trim();
    if name.is_empty() {
        return None;
    }
    let after = &body[colon + 1..];
    Some(Entry {
        name,
        name_at: before.len() - before.trim_start().len(),
        value: after.trim(),
        value_at: colon + 1 + (after.len() - after.trim_start().len()),
    })
}

/// A live range `first-last`, as the register parser reads it.
fn live_range(value: &str) -> Option<(i64, i64)> {
    let (first, last) = value.split_once('-')?;
    Some((first.trim().parse().ok()?, last.trim().parse().ok()?))
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/hardware_spec/service_tests.rs"]
mod service_tests;

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/hardware_spec/tests.rs"]
mod tests;
