//! ★ LOCK — every trigger-parameter annotation `parse_trigger` (`parser/component.rs`) reads is
//! either KEPT in the AST (the `Trigger` variant's own field holds `(Symbol, &'a TypeExpr<'a>)`,
//! the pair itself, same shape [`Trigger::Message`] already used) or VALIDATED against the one
//! shape that trigger actually delivers (`require_trigger_param_type`, refusing anything else) —
//! never silently read and thrown away. G25's regression was exactly that: `When a file is
//! dropped on … (name: Text):` had its `Text` annotation parsed and discarded, so the reference
//! tier bound a `File` record regardless of what the author wrote, breaking the pre-existing
//! `(name: Text)` signature (`gauntlet_findings_lock`, `lgx_vm_parity_lock`).
//!
//! This file reads BOTH `ast/component.rs` (which `Trigger` fields are a bare `Symbol` — type
//! discarded — versus a `(Symbol, &TypeExpr)` pair — type kept) and `parser/component.rs`
//! (every `parse_param_groups()` call site inside `parse_trigger`, and which `Trigger` variant
//! its own `return Ok(Trigger::…)` constructs) and cross-checks them: a call site whose variant
//! keeps every field structurally passes with no further question; a call site whose variant has
//! N bare-`Symbol` fields must show at least N `require_trigger_param_type(` calls of its own. A
//! FIFTH call site (a new trigger parameter) changes `KNOWN_CALL_SITES` and fails this census
//! until it is taught the new variant's own field shape.

const AST_SRC: &str = include_str!("../../logicaffeine_language/src/ast/component.rs");
const PARSER_SRC: &str = include_str!("../../logicaffeine_language/src/parser/component.rs");

/// `parse_trigger`'s own body — from its signature to the next `fn` at the same indent
/// (`trigger_error`) — so the census reads exactly the function G25's regression was in, not any
/// other `parse_param_groups()` call in this file (an action's or a component's own parameter
/// list travels as a REAL function parameter; its type is never in question there). Takes the
/// source text as a parameter (rather than reading `PARSER_SRC` directly) so the extraction logic
/// itself is exercised against synthetic, deliberately-broken text below — proving each of this
/// census's own checks can actually fail, without ever mutating the real source files it locks.
fn parse_trigger_body_of(src: &str) -> &str {
    let start = src
        .find("fn parse_trigger(&mut self)")
        .expect("parse_trigger must exist in parser/component.rs");
    let after = &src[start..];
    let end = after
        .find("\n    fn trigger_error(")
        .expect("trigger_error must immediately follow parse_trigger");
    &after[..end]
}

fn parse_trigger_body() -> &'static str {
    parse_trigger_body_of(PARSER_SRC)
}

/// `enum Trigger<'a> { … }`'s own body. Parameterized over its source text for the same reason
/// [`parse_trigger_body_of`] is.
fn trigger_enum_body_of(src: &str) -> &str {
    let start = src
        .find("pub enum Trigger<'a> {")
        .expect("Trigger enum must exist");
    let after = &src[start..];
    let end = after
        .find("\n}\n")
        .expect("Trigger enum must close with its own `}`");
    &after[..end]
}

fn trigger_enum_body() -> &'static str {
    trigger_enum_body_of(AST_SRC)
}

/// Splits a field list (`a: T, b: (U, V), c: Option<W>`) on its TOP-LEVEL commas — the ones not
/// nested inside `(...)`, `<...>` or `{...}` — so a tuple- or generic-typed field's own internal
/// comma is never mistaken for a field separator.
fn split_top_level(fields: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in fields.char_indices() {
        match c {
            '(' | '<' | '{' => depth += 1,
            ')' | '>' | '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(fields[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = fields[start..].trim();
    if !last.is_empty() {
        out.push(last);
    }
    out
}

/// How many of `variant`'s own fields declare a bare `Symbol` — a bound name with its type
/// annotation discarded from the AST — as opposed to a `(Symbol, &'a TypeExpr<'a>)` pair, which
/// keeps it. Panics (failing the whole census loudly) if `variant` is not declared in the enum at
/// all, so a typo'd or renamed variant can never silently read as "0 fields, trivially sound".
fn symbol_only_field_count_in(body: &str, variant: &str) -> usize {
    let needle = format!("{variant} {{");
    let at = body
        .find(&needle)
        .unwrap_or_else(|| panic!("Trigger::{variant} not declared in ast/component.rs's enum"));
    let after = &body[at + needle.len()..];
    let close = after
        .find('}')
        .expect("a variant's field list must close with `}`");
    let fields = &after[..close];
    split_top_level(fields)
        .iter()
        .filter(|field| {
            // `name: Type` — the type is everything after the field's own first `:`.
            field
                .splitn(2, ':')
                .nth(1)
                .map(|ty| ty.trim() == "Symbol")
                .unwrap_or(false)
        })
        .count()
}

fn symbol_only_field_count(variant: &str) -> usize {
    symbol_only_field_count_in(trigger_enum_body(), variant)
}

/// One `parse_param_groups()` call site inside `parse_trigger`: its own source text, from the
/// call itself up to (and including) the arm's `return Ok(Trigger::…);` statement — everything a
/// reader needs to answer "kept, validated, or silently dropped".
#[derive(Debug)]
struct CallSite {
    text: String,
    variant: String,
}

fn call_sites_in(body: &str) -> Vec<CallSite> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(i) = rest.find(".parse_param_groups()") {
        let after = &rest[i..];
        let ret_at = after.find("return Ok(Trigger::").unwrap_or_else(|| {
            panic!("a parse_param_groups() call site in parse_trigger never reaches its own `return Ok(Trigger::…)` — census cannot tell which variant it feeds:\n{}", &after[..after.len().min(200)])
        });
        let stmt_end = after[ret_at..]
            .find(';')
            .map(|k| ret_at + k + 1)
            .unwrap_or(after.len());
        let text = after[..stmt_end].to_string();
        let variant_start = ret_at + "return Ok(Trigger::".len();
        let variant_len = after[variant_start..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .expect("a Trigger variant name must end before `{` or `(`");
        let variant = after[variant_start..variant_start + variant_len].to_string();
        out.push(CallSite { text, variant });
        rest = &after[stmt_end..];
    }
    out
}

fn call_sites() -> Vec<CallSite> {
    call_sites_in(parse_trigger_body())
}

/// The census's own core predicate: is this call site's variant sound, given how many of its
/// fields are bare `Symbol` (type discarded, `needed` of them require a validation call) against
/// how many `require_trigger_param_type(` calls the site's own text shows? Exercised directly by
/// [`the_soundness_check_actually_rejects_a_discarded_annotation`] with hand-built sites, so this
/// file cannot pass merely because every call site it happens to find today is already sound.
fn call_site_is_sound(site: &CallSite, needed_for: impl Fn(&str) -> usize) -> bool {
    let needed = needed_for(&site.variant);
    let validated = site
        .text
        .matches("self.require_trigger_param_type(")
        .count();
    validated >= needed
}

const KNOWN_CALL_SITES: usize = 4;

#[test]
fn every_trigger_parameter_annotation_is_kept_or_validated() {
    let sites = call_sites();
    assert_eq!(
        sites.len(),
        KNOWN_CALL_SITES,
        "found {} parse_param_groups() call sites inside parse_trigger, expected exactly \
         {KNOWN_CALL_SITES} — a trigger parameter was added or removed; give any new one the same \
         choice every existing one makes (kept in the AST as `(Symbol, &TypeExpr)`, or validated \
         with `require_trigger_param_type` against the one shape the trigger delivers) before \
         updating this count",
        sites.len()
    );
    for site in &sites {
        assert!(
            call_site_is_sound(site, symbol_only_field_count),
            "Trigger::{}'s call site discards a parameter's type annotation without validating \
             it: {} of its fields are a bare `Symbol` (type not kept in the AST) but only {} \
             `require_trigger_param_type` call(s) guard it — exactly G25's regression class. \
             Site:\n{}",
            site.variant,
            symbol_only_field_count(&site.variant),
            site.text
                .matches("self.require_trigger_param_type(")
                .count(),
            site.text
        );
    }
}

/// Proves [`call_site_is_sound`] can actually FAIL — a hand-built call site for a Symbol-only
/// field variant with no validation call at all must be rejected, and the same variant WITH one
/// must be accepted. Without this, `every_trigger_parameter_annotation_is_kept_or_validated`
/// could pass for the wrong reason: every call site it happens to find today already being sound,
/// rather than the check itself being capable of catching an unsound one.
#[test]
fn the_soundness_check_actually_rejects_a_discarded_annotation() {
    let needs_one = |_: &str| 1usize;
    let discarded = CallSite {
        text:
            "let (param, ty) = params.first().copied()?; return Ok(Trigger::ActionFails { param });"
                .to_string(),
        variant: "ActionFails".to_string(),
    };
    assert!(
        !call_site_is_sound(&discarded, needs_one),
        "a call site with NO `require_trigger_param_type` call must be rejected when its variant \
         needs one — the check is trivially true otherwise"
    );
    let validated = CallSite {
        text: "let (param, ty) = params.first().copied()?; self.require_trigger_param_type(ty, \"Text\", \"…\", span)?; return Ok(Trigger::ActionFails { param });".to_string(),
        variant: "ActionFails".to_string(),
    };
    assert!(
        call_site_is_sound(&validated, needs_one),
        "a call site with the required validation call must be accepted"
    );
    let needs_two = |_: &str| 2usize;
    assert!(
        !call_site_is_sound(&validated, needs_two),
        "one validation call must not satisfy a variant that needs two (FetchFails's own url \
         AND error) — a partially-validated site is exactly the bug class where the SECOND field \
         slips through unchecked"
    );
    let kept = CallSite {
        text:
            "let param = params.into_iter().next()?; return Ok(Trigger::Message { topic, param });"
                .to_string(),
        variant: "Message".to_string(),
    };
    let needs_none = |_: &str| 0usize;
    assert!(
        call_site_is_sound(&kept, needs_none),
        "a variant that keeps its whole `(Symbol, &TypeExpr)` pair needs no validation call at all"
    );
}

/// Proves the EXTRACTION itself — [`call_sites_in`] — finds exactly as many call sites as a
/// synthetic body actually has, and reads back the right variant name for each, rather than the
/// real `parse_trigger` simply happening to contain four in the shape the parser expects.
#[test]
fn call_site_extraction_counts_and_names_correctly() {
    let synthetic = "\
        fn parse_trigger(&mut self) -> ParseResult<Trigger<'a>> {\n\
        if self.eat_words(&[\"an\", \"action\", \"fails\"]) {\n\
        let params = self.parse_param_groups()?;\n\
        return Ok(Trigger::ActionFails { param });\n\
        }\n\
        if self.eat_words(&[\"a\", \"message\", \"arrives\", \"on\"]) {\n\
        let params = self.parse_param_groups()?;\n\
        return Ok(Trigger::Message { topic, param });\n\
        }\n\
        }\n";
    let sites = call_sites_in(synthetic);
    assert_eq!(
        sites.len(),
        2,
        "expected exactly 2 synthetic call sites, found {}",
        sites.len()
    );
    assert_eq!(sites[0].variant, "ActionFails");
    assert_eq!(sites[1].variant, "Message");
}

/// Proves [`split_top_level`] does not mistake a tuple-typed field's OWN internal comma
/// (`(Symbol, &'a TypeExpr<'a>)`) for a field separator — the exact shape `Trigger::Message` and
/// `Trigger::FileDropped`'s own `param` field has. A naive `.split(',')` would see 3 fields here,
/// not 2, and every downstream count in this file would be wrong.
#[test]
fn split_top_level_keeps_a_tuple_fields_own_comma_intact() {
    let fields = "topic: &'a Expr<'a>, param: (Symbol, &'a TypeExpr<'a>)";
    let parts = split_top_level(fields);
    assert_eq!(
        parts,
        vec!["topic: &'a Expr<'a>", "param: (Symbol, &'a TypeExpr<'a>)"]
    );
}

/// Proves [`symbol_only_field_count_in`] tells a bare `Symbol` field apart from a kept pair on
/// synthetic enum text, so `tuple_kept_variants_need_no_validation_call`'s use of the real
/// `Trigger` enum is checking something the function can actually get right or wrong.
#[test]
fn symbol_only_field_count_reads_synthetic_enum_text() {
    let synthetic = "pub enum Trigger<'a> {\n\
        Message { topic: &'a Expr<'a>, param: (Symbol, &'a TypeExpr<'a>) },\n\
        ActionFails { param: Symbol },\n\
        FetchFails { url: Symbol, error: Symbol },\n\
        }\n";
    assert_eq!(symbol_only_field_count_in(synthetic, "Message"), 0);
    assert_eq!(symbol_only_field_count_in(synthetic, "ActionFails"), 1);
    assert_eq!(symbol_only_field_count_in(synthetic, "FetchFails"), 2);
}

/// `is_sound`'s converse, spelled out for the two variants that keep the whole pair: a
/// tuple-typed field needs ZERO validation calls to be sound, so `Trigger::Message` and
/// `Trigger::FileDropped`'s own call sites must still pass even though neither one's `return Ok`
/// statement mentions `require_trigger_param_type` at all — this is the belt to the main test's
/// suspenders, pinned to the two variant names directly so a future refactor that accidentally
/// starts requiring (or silently drops) validation on either one is caught by name.
#[test]
fn tuple_kept_variants_need_no_validation_call() {
    for variant in ["Message", "FileDropped"] {
        assert_eq!(
            symbol_only_field_count(variant),
            0,
            "Trigger::{variant} was expected to keep its parameter as a `(Symbol, &TypeExpr)` \
             pair (0 bare-`Symbol` fields) — if this now shows a bare `Symbol` field, its own \
             parser call site needs a `require_trigger_param_type` guard, not a silent pass here",
        );
    }
    for variant in ["ActionFails", "FetchFails"] {
        assert!(
            symbol_only_field_count(variant) >= 1,
            "Trigger::{variant} was expected to have at least one bare-`Symbol` field needing \
             validation at its own call site — if it now keeps a `(Symbol, &TypeExpr)` pair \
             instead, that call site's validation call becomes optional, not wrong",
        );
    }
}
