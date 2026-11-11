//! The specializers' memo keys. A specializer memoizes a call by its function and the static arguments it is called
//! with, so two calls share one specialization exactly when their arguments spell one key. Every value and every
//! statement keys by its whole structure, each part length-prefixed by `keyJoin`, so no two values key alike: two
//! closures with different bodies, two records with different fields.
//!
//! `pe_source.logos` holds the keys between [`KEYS_BEGIN`] and [`KEYS_END`], and each of the other two dialects holds
//! the same functions under its own suffix, [`dialect_keys`] (`pe_mini`'s `exprToKeyPartM`, `pe_bti`'s
//! `exprToKeyPartB`), so a dialect keys two values apart exactly when the online specializer does.

/// The line that opens the memo keys in each dialect.
pub const KEYS_BEGIN: &str = "# The memo keys: every value and statement keyed by its whole structure.";
/// The line that closes them.
pub const KEYS_END: &str = "# The memo keys end.";

/// The memo keys in `source`, from [`KEYS_BEGIN`] through [`KEYS_END`].
pub fn keys_in(source: &str) -> Option<&str> {
    let start = source.find(KEYS_BEGIN)?;
    let end = start + source[start..].find(KEYS_END)? + KEYS_END.len();
    Some(&source[start..end])
}

/// The memo keys `pe_source.logos` holds.
pub fn source_keys() -> &'static str {
    keys_in(crate::compile::pe_source_text()).expect("pe_source.logos holds the memo keys")
}

/// The functions the memo keys define, in the order they are written: `keyJoin`, then each `…ToKeyPart`.
pub fn key_functions() -> Vec<&'static str> {
    source_keys()
        .lines()
        .filter_map(|line| line.strip_prefix("## To "))
        .map(|rest| rest.split(' ').next().expect("a definition names its function"))
        .collect()
}

/// The memo keys as the dialect whose functions end in `suffix` (`"M"`, `"B"`) holds them: [`source_keys`] with each
/// key function renamed where it is defined and wherever it is called.
pub fn dialect_keys(suffix: &str) -> String {
    let names = key_functions();
    let text = source_keys();
    let mut out = String::with_capacity(text.len() + 64 * names.len());
    let mut rest = text;
    while let Some(at) = rest.find(|c: char| c.is_ascii_alphabetic()) {
        out.push_str(&rest[..at]);
        let word_len = rest[at..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len() - at);
        let word = &rest[at..at + word_len];
        let after = &rest[at + word_len..];
        let preceded_by_identifier = out.ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_');
        out.push_str(word);
        if !preceded_by_identifier && names.contains(&word) && (after.starts_with('(') || after.starts_with(" (")) {
            out.push_str(suffix);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Each key function's specialization prefix and declared signature under `suffix` —
/// `("exprToKeyPartM_", "(e: CExpr) -> Text:")` — what a projection restores on a specialization the decompiler printed
/// with `Any` types.
pub fn dialect_key_signatures(suffix: &str) -> Vec<(String, String)> {
    source_keys()
        .lines()
        .filter_map(|line| line.strip_prefix("## To "))
        .map(|rest| {
            let (name, signature) = rest.split_once(' ').expect("a definition declares its signature");
            (format!("{name}{suffix}_"), signature.to_string())
        })
        .collect()
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/memo_keys/tests.rs"]
mod tests;
