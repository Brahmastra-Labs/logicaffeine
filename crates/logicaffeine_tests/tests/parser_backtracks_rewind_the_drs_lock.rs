//! A parse that backtracks abandons what the attempt read — the DRS referents it registered
//! included. Saving `self.current` and assigning it back rewinds only the token position, so
//! an abandoned attempt's referents would outlive it: a pronoun could find one, and a
//! conditional could quantify over it. Every speculative parse returns through a backtrack
//! point (`backtrack_point` / `backtrack_to`) or a full checkpoint instead; this lock keeps a
//! bare position save-and-restore from coming back.

use std::path::Path;

fn fn_name(line: &str) -> Option<&str> {
    let rest = line.trim_start();
    let rest = rest
        .strip_prefix("pub(super) ")
        .or_else(|| rest.strip_prefix("pub(crate) "))
        .or_else(|| rest.strip_prefix("pub "))
        .unwrap_or(rest);
    let rest = rest.strip_prefix("fn ")?;
    let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
    Some(&rest[..end])
}

/// `(file, line, function, variable)` for each `self.current = v;` that restores a `v` saved
/// with `let v = self.current;` earlier in the same function.
fn bare_backtracks() -> (Vec<(String, usize, String, String)>, usize) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_language/src/parser");
    let mut found = Vec::new();
    let mut points = 0;
    let mut files: Vec<_> = std::fs::read_dir(&dir).expect("the parser sources").map(|e| e.unwrap().path()).collect();
    files.sort();
    for path in files.into_iter().filter(|p| p.extension().is_some_and(|e| e == "rs")) {
        let text = std::fs::read_to_string(&path).unwrap();
        points += text.matches("self.backtrack_point()").count();
        let mut current_fn = String::new();
        let mut saved: Vec<String> = Vec::new();
        for (index, line) in text.lines().enumerate() {
            if let Some(name) = fn_name(line) {
                current_fn = name.to_string();
                saved.clear();
            }
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("let ").and_then(|r| r.strip_suffix(" = self.current;")) {
                if !rest.starts_with("mut ") {
                    saved.push(rest.to_string());
                }
            }
            if let Some(var) = trimmed.strip_prefix("self.current = ").and_then(|r| r.strip_suffix(';')) {
                if saved.iter().any(|s| s == var) {
                    found.push((path.display().to_string(), index + 1, current_fn.clone(), var.to_string()));
                }
            }
        }
    }
    (found, points)
}

#[test]
fn every_backtrack_rewinds_the_drs() {
    let (bare, points) = bare_backtracks();
    assert!(points >= 40, "the parser's speculative parses return through backtrack points ({points} found)");
    assert!(
        bare.is_empty(),
        "these restore the token position alone — return through `backtrack_to` so the DRS rewinds too:\n{}",
        bare.iter().map(|(f, l, func, v)| format!("{f}:{l} in {func} ({v})")).collect::<Vec<_>>().join("\n"),
    );
}

/// "Every X has a Y. It is Z." — the parser tries "It is Z" more than one way, and an attempt it
/// abandons registers Z as a referent. Without the rewind that ghost outlives the attempt: "It"
/// finds it and the sentence reads as `Z = Z`, and "It fell" makes Z fall. With it, "It" reaches
/// the Y of the first sentence, as the pronoun rules say.
#[test]
fn an_abandoned_parse_leaves_no_referent_for_a_pronoun() {
    let out = logicaffeine_language::compile::compile("Every X has a Y. It is Z.").expect("parses");
    assert!(!out.contains("Z = Z"), "\"It\" found a referent an abandoned parse left behind: {out}");
    let out = logicaffeine_language::compile::compile("Every X has a Y. It is Z. It fell.").expect("parses");
    assert!(!out.contains("Agent(e, Z)"), "\"It fell\" found a referent an abandoned parse left behind: {out}");
}
