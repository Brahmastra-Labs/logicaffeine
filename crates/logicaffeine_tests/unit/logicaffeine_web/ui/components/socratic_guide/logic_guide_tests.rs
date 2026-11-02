use super::{logic_guide, GuideMode};
use crate::ui::components::proof_panel::ProofStatus;

fn text(mode: &GuideMode) -> &str {
    match mode {
        GuideMode::Idle => "",
        GuideMode::Success(m) | GuideMode::Error(m) | GuideMode::Hint(m) | GuideMode::Info(m) => m,
    }
}

#[test]
fn a_proved_theorem_says_its_goal_follows() {
    let mode = logic_guide(None, Some("Mortal(Socrates)"), 0, "## Theorem: T\nProve: Socrates is mortal.", &ProofStatus::Success);
    assert!(matches!(mode, GuideMode::Success(_)), "a proved theorem is not a success: {:?}", text(&mode));
    assert!(text(&mode).to_lowercase().contains("proved"), "{:?}", text(&mode));
}

#[test]
fn an_unproved_theorem_says_so() {
    let mode = logic_guide(None, Some("Mortal(Socrates)"), 0, "## Theorem: T", &ProofStatus::Failed("no proof".into()));
    assert!(text(&mode).to_lowercase().contains("not"), "{:?}", text(&mode));
    assert!(!matches!(mode, GuideMode::Success(_)));
}

#[test]
fn an_uncertified_derivation_is_neither_proved_nor_refuted() {
    let mode = logic_guide(None, Some("\u{00AC}\u{2203}x Barber(x)"), 0, "## Theorem: Barber_Paradox", &ProofStatus::Uncertified("kernel".into()));
    assert!(matches!(mode, GuideMode::Hint(_)), "an uncertified derivation is a caution: {:?}", text(&mode));
    assert!(text(&mode).contains("not certified"), "{:?}", text(&mode));
    assert!(!text(&mode).contains("Proved"), "{:?}", text(&mode));
    assert!(!text(&mode).contains("does not follow"), "{:?}", text(&mode));
}

#[test]
fn a_formula_on_screen_is_never_called_unresolved() {
    let mode = logic_guide(None, Some("Run(John)"), 0, "John runs.", &ProofStatus::Idle);
    assert!(!text(&mode).contains("could not be resolved"), "{:?}", text(&mode));
    assert!(text(&mode).contains("single"), "{:?}", text(&mode));
}

#[test]
fn ambiguous_sentences_keep_their_reading_count() {
    let mode = logic_guide(None, Some("x"), 2, "Every man loves a woman.", &ProofStatus::Idle);
    assert!(text(&mode).contains("ambiguous"), "{:?}", text(&mode));
}

#[test]
fn an_error_is_the_error() {
    let mode = logic_guide(Some("bad"), None, 0, "Bad.", &ProofStatus::Idle);
    assert!(matches!(mode, GuideMode::Error(ref m) if m == "bad"));
}

#[test]
fn nothing_to_say_without_a_formula() {
    assert!(logic_guide(None, None, 0, "", &ProofStatus::Idle) == GuideMode::Idle);
}
