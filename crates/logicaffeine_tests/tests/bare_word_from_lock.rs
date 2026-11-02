//! ★ LOCK — a bare identifier's own expression parse never steals `from` from the STATEMENT that
//! is actually asking for it. ★
//!
//! `crates/logicaffeine_language/src/parser/mod.rs`'s bare-identifier expression arm calls
//! `resolve_construction_target(sym, …)` for EVERY identifier it reads (Phase 33's zero-`new`
//! variant word, `North`, extended by this campaign's `from <Enum>` qualifier). Before the fix
//! this lock protects, that call consumed a `from` immediately after `sym` UNCONDITIONALLY,
//! whether or not `sym` was ever a variant — so `Remove k from m.` parsed `k from m` as a
//! (nonsensical) qualified construction, leaving `Remove`'s own parser with nothing left to see
//! and refusing with `ExpectedKeyword { keyword: "from" }`. `peEnvDrop`'s exact `Remove … from
//! …` shape in `pe_source` (`phase_futamura.rs`/`phase_pe_differential.rs`) is what caught it —
//! 591+ tests across both files, none of them about variants at all.
//!
//! The fix, `Parser::bare_word_from_is_a_qualifier`, treats `from` as THIS qualifier only when
//! BOTH hold: `sym` is a declared variant of SOME enum (so nothing else the sentence could mean
//! survives the check), and the word after `from` names a real enum (or a real module-qualified
//! type) — otherwise `from` is left completely untouched for whoever is asking for it.
//!
//! **Census of every statement verb that takes `from`** (grepped `TokenType::From`/
//! `check_preposition_is("from")` across the whole parser and read each site):
//!
//! | Statement | Site | Operand before `from`? | At risk? |
//! |---|---|---|---|
//! | `Remove x from y.` | `parse_remove_statement` | `parse_imperative_expr()` — general expression, reaches the bare-identifier arm | YES — this lock |
//! | `Pop from y [into z].` | `parse_pop_statement` | none — `from` consumed BEFORE the collection expression | no |
//! | `Read x from …` | `parse_read_statement` | `expect_identifier()` — a raw token, never an expression | no |
//! | `Receive x from pipe.` | `parse_receive_pipe_statement` | `expect_identifier()` | no |
//! | `Try to receive x from pipe.` | `parse_try_statement` | `expect_identifier()` | no |
//! | `On e from s:` | `parse_on_statement` | a raw lexeme, never an expression | no |
//! | `Inside a zone … mapped from "f":` | `parse_zone_statement` | the fixed keyword `mapped` | no |
//! | `Receive x from ch:` (select branch) | `parse_select_branch` | a raw token match | no |
//! | `Await … from … into …` | `parse_await_statement` | fixed keywords (`response`/`view`/`stream`) | no |
//! | `causal attention … from P` | `try_parse_tensor_unary` | the fixed keyword `groups` | no |
//! | `Repeat/for … from N to M` | `parse_repeat_statement`/`parse_for_statement` | `parse_loop_pattern()` — a binding, never evaluated | no |
//! | `Type from Module` (type annotation) | `parse_type_expression` | not this call site at all — a different grammar entirely | no |
//! | `Take n from xs.` | — | **does not exist in this grammar** (grepped; no `Take` statement) | n/a |
//!
//! `Remove` is the only statement whose own operand is a general expression parsed BEFORE `from`
//! is expected — the one this lock exercises directly. Two of the structurally-safe forms
//! (`Pop`, the range `Repeat`) are exercised anyway, as cheap empirical confirmation on top of
//! reading the source.

use logicaffeine_compile::compile::tw_outcome;

fn both_tw(src: &str, want: &[&str]) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "must run:\n{src}");
    assert_eq!(tw.output.lines().collect::<Vec<_>>(), want, "{src}");
}

/// The exact repro shape: a bare variable, never a variant of anything, before `Remove`'s own
/// `from`.
#[test]
fn remove_a_bare_variable_from_a_plain_collection_parses_as_the_remove_statement() {
    let src = "\
## Main
Let m be a new Set of Int.
Add 1 to m.
Add 2 to m.
Let k be 1.
Remove k from m.
Show length of m.
";
    both_tw(src, &["1"]);
}

/// A second instance with different names, so the fix is not merely tuned to the repro's own
/// spelling.
#[test]
fn remove_x_from_items_parses_as_the_remove_statement() {
    let src = "\
## Main
Let items be a new Set of Int.
Add 5 to items.
Add 6 to items.
Let x be 5.
Remove x from items.
Show length of items.
";
    both_tw(src, &["1"]);
}

/// The harder half: the REMOVED value's own name is ALSO a declared variant somewhere in the
/// same program (`bare_word_from_is_a_qualifier`'s first condition is true), but the word after
/// `from` names an ordinary collection, not an enum (its second condition is false) — `from`
/// must still be left for `Remove`, not refused as "not an enum".
#[test]
fn remove_a_variable_sharing_a_variants_name_still_parses_as_the_remove_statement() {
    let src = "\
## A Status is either:
    Ping.
    Down.

## Main
Let items be a new Set of Int.
Add 1 to items.
Add 2 to items.
Let Ping be 1.
Remove Ping from items.
Show length of items.
";
    both_tw(src, &["1"]);
}

/// The other half of the same gate: a REAL qualified variant construction must still resolve —
/// a single enum only, so this cannot also trip the unrelated `variant_fields` cross-tier
/// collision `variant_resolution_lock.rs` already tracks for two enums sharing a variant name.
#[test]
fn a_real_qualified_variant_construction_still_resolves() {
    let src = "\
## An Outcome is either:
    Success (value: Int).
    Failure (message: Text).

## Main
Let a be a new Failure from Outcome with message \"boom\".
Inspect a:
    When Success (v): Show \"wrong\".
    When Failure (m): Show m.
";
    both_tw(src, &["boom"]);
}

/// `Pop from …` consumes `from` BEFORE parsing its collection — structurally unaffected by the
/// bare-identifier gate, confirmed empirically.
#[test]
fn pop_from_a_collection_is_unaffected() {
    let src = "\
## Main
Let xs be a new Seq of Int.
Push 1 to xs.
Push 2 to xs.
Pop from xs into last.
Show last.
Show length of xs.
";
    both_tw(src, &["2", "1"]);
}

/// `Repeat … from N to M:` binds its loop variable through a dedicated pattern parser, never
/// through the bare-identifier expression arm — confirmed empirically.
#[test]
fn a_range_repeat_using_from_is_unaffected() {
    let src = "\
## Main
Repeat for i from 1 to 3:
    Show i.
";
    both_tw(src, &["1", "2", "3"]);
}
