use crate::ui::components::proof_panel::PROOF_PANEL_STYLE;
use crate::ui::pages::studio_layout::PROOF_MIN;

/// The proof drawer's CSS floor is the grip's rule's (`proof_dragged`), so a drag and a window
/// resize agree on where the drawer stops shrinking.
#[test]
fn the_proof_drawer_css_matches_its_layout_rule() {
    let css = PROOF_PANEL_STYLE;
    let at = css.find(".proof-drawer {").expect("no rule for .proof-drawer");
    let rule = &css[at..at + css[at..].find('}').unwrap()];
    assert!(rule.contains(&format!("min-height: {}px", PROOF_MIN)), "the drawer's floor is not {PROOF_MIN}px: {rule}");
}
