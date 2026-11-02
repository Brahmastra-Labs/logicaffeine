//! ★ LOCK — every ledger row belongs to exactly ONE comparison, named by its tag. ★
//!
//! One file records the known divergences of every tier pair: a plain row is the lowered VM's,
//! `browser:` a real Chrome's. The tag decides which lock reads a row and which lock may call it stale, so a row
//! whose tag is misspelled must be refused — read as plain, it would silently become another tier's
//! claim and go stale there, or excuse a divergence it was never written for.
use super::{ledger_map, parse_ledger, LedgerTier};

fn row(kind_field: &str) -> String {
    format!("p.lg::t | {kind_field} @ 2 | because\n")
}

#[test]
fn every_tier_is_read_back_from_its_own_tag() {
    for tier in LedgerTier::ALL {
        let kind_field = match tier.tag() {
            Some(tag) => format!("{tag}:document"),
            None => "document".to_string(),
        };
        let rows = parse_ledger(&row(&kind_field));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tier, tier, "`{kind_field}` is {tier:?}'s row");
        assert_eq!(
            rows[0].kind, "document",
            "the tag is stripped from the axis"
        );
    }
}

#[test]
fn the_tier_table_is_complete() {
    // Exhaustive by construction: a new tier that is not in `ALL` fails to compile here first.
    for tier in LedgerTier::ALL {
        match tier {
            LedgerTier::LoweredVm => {
                assert_eq!(tier.tag(), None, "the lowered VM's rows are plain")
            }
            LedgerTier::Browser => assert_eq!(tier.tag(), Some("browser")),
        }
    }
    let tags: Vec<_> = LedgerTier::ALL.iter().filter_map(|t| t.tag()).collect();
    let mut unique = tags.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        tags.len(),
        "two tiers may not share a tag: {tags:?}"
    );
    assert_eq!(
        LedgerTier::ALL.iter().filter(|t| t.tag().is_none()).count(),
        1,
        "exactly one tier is untagged"
    );
}

#[test]
#[should_panic(expected = "`ats:` is not a ledger tag")]
fn a_misspelled_tag_is_refused_not_read_as_plain() {
    parse_ledger(&row("ats:document"));
}

#[test]
#[should_panic(expected = "`Browser:` is not a ledger tag")]
fn a_tag_is_matched_exactly() {
    parse_ledger(&row("Browser:document"));
}

#[test]
#[should_panic(expected = "carries more than one tag")]
fn a_row_carries_at_most_one_tag() {
    parse_ledger(&row("browser:browser:document"));
}

#[test]
#[should_panic(expected = "names neither `refused` nor a real axis")]
fn a_tagged_row_still_names_a_real_axis() {
    parse_ledger(&row("browser:documents"));
}

#[test]
fn each_tier_reads_only_its_own_rows_and_one_key_may_appear_once_per_tier() {
    let text = [
        row("document"),
        row("browser:document"),
        row("browser:refused"),
    ]
    .concat();
    let rows = parse_ledger(&text);
    for tier in LedgerTier::ALL {
        let map = ledger_map(&rows, tier);
        let expected = if tier == LedgerTier::Browser { 2 } else { 1 };
        assert_eq!(
            map.len(),
            expected,
            "{tier:?} reads exactly its own rows: {map:?}"
        );
        assert!(map.contains_key(&("p.lg::t".to_string(), "document".to_string(), 2)));
    }
}

#[test]
#[should_panic(expected = "duplicate ledger entry")]
fn a_key_listed_twice_for_one_tier_is_refused() {
    let text = [row("browser:document"), row("browser:document")].concat();
    ledger_map(&parse_ledger(&text), LedgerTier::Browser);
}

/// ★ THE AST COMPONENT RUNTIME IS GONE, AND SO IS ITS TAG. Every `ast:` row recorded how that
/// runtime differed from the lowered page; with one implementation there is nothing left for such
/// a row to compare, so writing one is refused rather than read as some other tier's claim.
#[test]
#[should_panic(expected = "`ast:` is not a ledger tag")]
fn a_row_for_the_deleted_ast_runtime_is_refused() {
    parse_ledger(&row("ast:document"));
}
